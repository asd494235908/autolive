use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use sysinfo::Disks;

pub const PRODUCTION_BASE_URL: &str = "http://101.96.208.132:7088/autolive-resources/v0.1.0";
const RELEASE: &str = "v0.1.0";
const SUPPORTED_TARGETS: [&str; 3] = [
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationMode {
    Production,
    Test { base_url: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ManifestComponent {
    Media,
    VoiceRuntime,
    VoiceModels,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeResourceComponent {
    Media,
    Voice,
}

impl RuntimeResourceComponent {
    pub fn required_components(self) -> &'static [ManifestComponent] {
        match self {
            Self::Media => &[ManifestComponent::Media],
            Self::Voice => &[
                ManifestComponent::Media,
                ManifestComponent::VoiceRuntime,
                ManifestComponent::VoiceModels,
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeResourceState {
    NotInstalled,
    Checking,
    Downloading,
    Verifying,
    Ready,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct RuntimeResourceStatus {
    pub state: RuntimeResourceState,
    pub component: Option<RuntimeResourceComponent>,
    pub current_file: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub bytes_per_second: u64,
    pub installed_bytes: u64,
    pub resource_root: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeResourceManifest {
    pub schema_version: u32,
    pub release: String,
    pub target: String,
    pub base_url: String,
    pub files: Vec<ManifestFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    pub component: ManifestComponent,
    pub relative_path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub executable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestValidationError(String);

impl fmt::Display for ManifestValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ManifestValidationError {}

impl RuntimeResourceManifest {
    pub fn parse_and_validate(
        bytes: &[u8],
        mode: ValidationMode,
    ) -> Result<Self, ManifestValidationError> {
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|error| ManifestValidationError(format!("invalid manifest JSON: {error}")))?;
        manifest.validate(&mode)?;
        Ok(manifest)
    }

    fn validate(&self, mode: &ValidationMode) -> Result<(), ManifestValidationError> {
        if self.schema_version != 1 {
            return Err(ManifestValidationError(
                "unsupported manifest schema_version".to_owned(),
            ));
        }
        if self.release != RELEASE {
            return Err(ManifestValidationError(
                "unsupported manifest release".to_owned(),
            ));
        }
        if !SUPPORTED_TARGETS.contains(&self.target.as_str()) {
            return Err(ManifestValidationError(
                "unsupported manifest target".to_owned(),
            ));
        }
        validate_base_url(&self.base_url, mode)?;
        if self.files.is_empty() {
            return Err(ManifestValidationError(
                "manifest must contain files".to_owned(),
            ));
        }

        let mut paths = HashSet::with_capacity(self.files.len());
        for file in &self.files {
            if file.size_bytes == 0 {
                return Err(ManifestValidationError(format!(
                    "manifest file has zero size: {}",
                    file.relative_path
                )));
            }
            if file.sha256.len() != 64
                || !file
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err(ManifestValidationError(format!(
                    "manifest file has invalid sha256: {}",
                    file.relative_path
                )));
            }
            validate_relative_path(&file.relative_path)?;
            validate_component_layout(file, &self.target)?;
            if !paths.insert(file.relative_path.as_str()) {
                return Err(ManifestValidationError(format!(
                    "manifest contains duplicate path: {}",
                    file.relative_path
                )));
            }
        }
        for component in [
            ManifestComponent::Media,
            ManifestComponent::VoiceRuntime,
            ManifestComponent::VoiceModels,
        ] {
            if !self.files.iter().any(|file| file.component == component) {
                return Err(ManifestValidationError(format!(
                    "manifest is missing component: {component:?}"
                )));
            }
        }
        Ok(())
    }
}

fn validate_base_url(base_url: &str, mode: &ValidationMode) -> Result<(), ManifestValidationError> {
    let expected = match mode {
        ValidationMode::Production => format!("{PRODUCTION_BASE_URL}/"),
        ValidationMode::Test { base_url } => {
            let parsed = reqwest::Url::parse(base_url)
                .map_err(|_| ManifestValidationError("invalid test base_url".to_owned()))?;
            if parsed.scheme() != "http"
                || parsed.host_str() != Some("127.0.0.1")
                || parsed.port().is_none()
                || !parsed.path().ends_with('/')
                || parsed.query().is_some()
                || parsed.fragment().is_some()
            {
                return Err(ManifestValidationError(
                    "test base_url must be an explicit loopback HTTP origin".to_owned(),
                ));
            }
            base_url.clone()
        }
    };
    if base_url != expected {
        return Err(ManifestValidationError(
            "manifest base_url is not trusted".to_owned(),
        ));
    }
    Ok(())
}

fn validate_relative_path(relative_path: &str) -> Result<(), ManifestValidationError> {
    if relative_path.is_empty()
        || relative_path.starts_with('/')
        || relative_path.contains('\\')
        || relative_path.contains('%')
        || relative_path.contains('\0')
        || relative_path.contains('?')
        || relative_path.contains('#')
        || relative_path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(ManifestValidationError(format!(
            "unsafe manifest path: {relative_path}"
        )));
    }
    Ok(())
}

fn validate_component_layout(
    file: &ManifestFile,
    target: &str,
) -> Result<(), ManifestValidationError> {
    let valid = match file.component {
        ManifestComponent::Media => {
            file.executable
                && file
                    .relative_path
                    .starts_with(&format!("{target}/binaries/"))
        }
        ManifestComponent::VoiceRuntime => {
            file.executable
                && file
                    .relative_path
                    .starts_with(&format!("{target}/voice-worker/"))
        }
        ManifestComponent::VoiceModels => {
            !file.executable && file.relative_path.starts_with("common/voice-models/")
        }
    };
    if !valid {
        return Err(ManifestValidationError(format!(
            "manifest component path is invalid: {}",
            file.relative_path
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeResourceLayout {
    pub version_root: PathBuf,
    pub target_root: PathBuf,
    pub model_root: PathBuf,
    pub partial_root: PathBuf,
    pub installed_record: PathBuf,
}

impl RuntimeResourceLayout {
    pub fn for_target(app_data_dir: &Path, target: &str) -> Result<Self, ManifestValidationError> {
        if !SUPPORTED_TARGETS.contains(&target) {
            return Err(ManifestValidationError(
                "unsupported runtime resource target".to_owned(),
            ));
        }
        let version_root = app_data_dir.join("runtime-resources").join(RELEASE);
        Ok(Self {
            target_root: version_root.join(target),
            model_root: version_root.join("common").join("voice-models"),
            partial_root: version_root.join("partial"),
            installed_record: version_root.join("installed.json"),
            version_root,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceInstallError {
    Manifest(String),
    Busy,
    Cancelled,
    Io {
        operation: &'static str,
        path: String,
    },
    Http {
        path: String,
        message: String,
    },
    UnexpectedHttpStatus {
        path: String,
        status: u16,
    },
    InvalidContentRange {
        expected: String,
        actual: Option<String>,
    },
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    HashMismatch {
        path: String,
    },
    InsufficientDiskSpace {
        required: u64,
        available: u64,
    },
    DiskUnavailable,
    UnsafeImportSource {
        path: String,
    },
}

impl fmt::Display for ResourceInstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(message) => {
                write!(formatter, "invalid runtime resource manifest: {message}")
            }
            Self::Busy => formatter.write_str("a runtime resource operation is already running"),
            Self::Cancelled => formatter.write_str("runtime resource operation was cancelled"),
            Self::Io { operation, path } => {
                write!(formatter, "failed to {operation} runtime resource: {path}")
            }
            Self::Http { path, message } => {
                write!(
                    formatter,
                    "failed to download runtime resource {path}: {message}"
                )
            }
            Self::UnexpectedHttpStatus { path, status } => {
                write!(formatter, "runtime resource {path} returned HTTP {status}")
            }
            Self::InvalidContentRange { expected, actual } => write!(
                formatter,
                "invalid Content-Range: expected {expected}, got {}",
                actual.as_deref().unwrap_or("missing")
            ),
            Self::SizeMismatch {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "runtime resource size mismatch for {path}: expected {expected}, got {actual}"
            ),
            Self::HashMismatch { path } => {
                write!(formatter, "runtime resource SHA-256 mismatch: {path}")
            }
            Self::InsufficientDiskSpace {
                required,
                available,
            } => write!(
                formatter,
                "insufficient disk space: need {required} bytes, have {available} bytes"
            ),
            Self::DiskUnavailable => formatter.write_str("target disk is unavailable"),
            Self::UnsafeImportSource { path } => {
                write!(
                    formatter,
                    "local import path escapes the selected directory: {path}"
                )
            }
        }
    }
}

impl std::error::Error for ResourceInstallError {}

impl From<ManifestValidationError> for ResourceInstallError {
    fn from(error: ManifestValidationError) -> Self {
        Self::Manifest(error.to_string())
    }
}

#[derive(Clone)]
pub struct RuntimeResourceInstaller {
    manifest: Arc<RuntimeResourceManifest>,
    layout: RuntimeResourceLayout,
    client: reqwest::blocking::Client,
    operation_running: Arc<AtomicBool>,
}

impl RuntimeResourceInstaller {
    pub fn from_embedded(
        manifest_bytes: &'static [u8],
        app_data_dir: &Path,
    ) -> Result<Self, ResourceInstallError> {
        let manifest = RuntimeResourceManifest::parse_and_validate(
            manifest_bytes,
            ValidationMode::Production,
        )?;
        Self::from_manifest(manifest, app_data_dir)
    }

    pub fn for_test(
        manifest_bytes: &[u8],
        app_data_dir: &Path,
        fixture_base_url: &str,
    ) -> Result<Self, ResourceInstallError> {
        let manifest = RuntimeResourceManifest::parse_and_validate(
            manifest_bytes,
            ValidationMode::Test {
                base_url: fixture_base_url.to_owned(),
            },
        )?;
        Self::from_manifest(manifest, app_data_dir)
    }

    fn from_manifest(
        manifest: RuntimeResourceManifest,
        app_data_dir: &Path,
    ) -> Result<Self, ResourceInstallError> {
        let layout = RuntimeResourceLayout::for_target(app_data_dir, &manifest.target)?;
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| ResourceInstallError::Http {
                path: manifest.base_url.clone(),
                message: error.to_string(),
            })?;
        Ok(Self {
            manifest: Arc::new(manifest),
            layout,
            client,
            operation_running: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn inspect(
        &self,
        component: RuntimeResourceComponent,
    ) -> Result<RuntimeResourceStatus, ResourceInstallError> {
        let files = self.required_files(component);
        let total_bytes = total_size(&files);
        let installed_bytes = self.installed_bytes(&files)?;
        Ok(self.status(
            if installed_bytes == total_bytes {
                RuntimeResourceState::Ready
            } else {
                RuntimeResourceState::NotInstalled
            },
            Some(component),
            None,
            installed_bytes,
            total_bytes,
            0,
            installed_bytes,
            None,
        ))
    }

    pub fn install(
        &self,
        component: RuntimeResourceComponent,
        cancel: &AtomicBool,
        mut on_status: impl FnMut(RuntimeResourceStatus),
    ) -> Result<RuntimeResourceStatus, ResourceInstallError> {
        let _operation = self.acquire_operation()?;
        let result = self.install_inner(component, cancel, &mut on_status);
        self.finish_operation(component, result, &mut on_status)
    }

    pub fn import_directory(
        &self,
        component: RuntimeResourceComponent,
        source_root: &Path,
        cancel: &AtomicBool,
        mut on_status: impl FnMut(RuntimeResourceStatus),
    ) -> Result<RuntimeResourceStatus, ResourceInstallError> {
        let _operation = self.acquire_operation()?;
        let result = self.import_inner(component, source_root, cancel, &mut on_status);
        self.finish_operation(component, result, &mut on_status)
    }

    pub fn clear_current_release(&self) -> Result<RuntimeResourceStatus, ResourceInstallError> {
        let _operation = self.acquire_operation()?;
        if self.layout.version_root.exists() {
            fs::remove_dir_all(&self.layout.version_root)
                .map_err(|_| io_error("clear", &self.layout.version_root))?;
        }
        Ok(self.status(
            RuntimeResourceState::NotInstalled,
            None,
            None,
            0,
            0,
            0,
            0,
            None,
        ))
    }

    fn install_inner(
        &self,
        component: RuntimeResourceComponent,
        cancel: &AtomicBool,
        on_status: &mut impl FnMut(RuntimeResourceStatus),
    ) -> Result<RuntimeResourceStatus, ResourceInstallError> {
        self.check_cancel(cancel)?;
        let files = self.required_files(component);
        let total_bytes = total_size(&files);
        let mut installed_bytes = self.installed_bytes(&files)?;
        on_status(self.status(
            RuntimeResourceState::Checking,
            Some(component),
            None,
            installed_bytes,
            total_bytes,
            0,
            installed_bytes,
            None,
        ));
        if installed_bytes == total_bytes {
            self.write_installed_record(component)?;
            return Ok(self.ready_status(component, total_bytes));
        }

        fs::create_dir_all(&self.layout.version_root)
            .map_err(|_| io_error("create directory", &self.layout.version_root))?;
        let required = self.additional_disk_bytes(&files)?;
        self.ensure_disk_space(required)?;

        for file in files {
            self.check_cancel(cancel)?;
            let final_path = self.final_path(file);
            if file_matches(&final_path, file)? {
                continue;
            }
            if final_path.exists() {
                fs::remove_file(&final_path)
                    .map_err(|_| io_error("remove invalid file", &final_path))?;
            }
            self.download_file(
                component,
                file,
                installed_bytes,
                total_bytes,
                cancel,
                on_status,
            )?;
            installed_bytes = installed_bytes.saturating_add(file.size_bytes);
        }
        self.write_installed_record(component)?;
        Ok(self.ready_status(component, total_bytes))
    }

    fn import_inner(
        &self,
        component: RuntimeResourceComponent,
        source_root: &Path,
        cancel: &AtomicBool,
        on_status: &mut impl FnMut(RuntimeResourceStatus),
    ) -> Result<RuntimeResourceStatus, ResourceInstallError> {
        self.check_cancel(cancel)?;
        let canonical_root = fs::canonicalize(source_root)
            .map_err(|_| io_error("open import directory", source_root))?;
        if !canonical_root.is_dir() {
            return Err(ResourceInstallError::UnsafeImportSource {
                path: source_root.display().to_string(),
            });
        }
        let files = self.required_files(component);
        let total_bytes = total_size(&files);
        let mut installed_bytes = self.installed_bytes(&files)?;
        on_status(self.status(
            RuntimeResourceState::Checking,
            Some(component),
            None,
            installed_bytes,
            total_bytes,
            0,
            installed_bytes,
            None,
        ));
        if installed_bytes == total_bytes {
            self.write_installed_record(component)?;
            return Ok(self.ready_status(component, total_bytes));
        }

        fs::create_dir_all(&self.layout.version_root)
            .map_err(|_| io_error("create directory", &self.layout.version_root))?;
        self.ensure_disk_space(self.additional_disk_bytes(&files)?)?;
        for file in files {
            self.check_cancel(cancel)?;
            let final_path = self.final_path(file);
            if file_matches(&final_path, file)? {
                continue;
            }
            if final_path.exists() {
                fs::remove_file(&final_path)
                    .map_err(|_| io_error("remove invalid file", &final_path))?;
            }
            let source = canonical_root.join(&file.relative_path);
            let canonical_source =
                fs::canonicalize(&source).map_err(|_| io_error("open import file", &source))?;
            if !canonical_source.starts_with(&canonical_root) || !canonical_source.is_file() {
                return Err(ResourceInstallError::UnsafeImportSource {
                    path: source.display().to_string(),
                });
            }
            self.copy_import_file(
                component,
                file,
                &canonical_source,
                installed_bytes,
                total_bytes,
                cancel,
                on_status,
            )?;
            installed_bytes = installed_bytes.saturating_add(file.size_bytes);
        }
        self.write_installed_record(component)?;
        Ok(self.ready_status(component, total_bytes))
    }

    #[allow(clippy::too_many_arguments)]
    fn download_file(
        &self,
        component: RuntimeResourceComponent,
        file: &ManifestFile,
        installed_bytes: u64,
        total_bytes: u64,
        cancel: &AtomicBool,
        on_status: &mut impl FnMut(RuntimeResourceStatus),
    ) -> Result<(), ResourceInstallError> {
        let partial_path = self.partial_path(file);
        if let Some(parent) = partial_path.parent() {
            fs::create_dir_all(parent).map_err(|_| io_error("create partial directory", parent))?;
        }
        let mut offset = fs::metadata(&partial_path)
            .map(|value| value.len())
            .unwrap_or(0);
        if offset > file.size_bytes {
            fs::remove_file(&partial_path)
                .map_err(|_| io_error("remove oversized partial", &partial_path))?;
            offset = 0;
        }
        if offset == file.size_bytes {
            return self.verify_and_commit(
                file,
                &partial_path,
                on_status,
                component,
                total_bytes,
                cancel,
            );
        }

        let url = reqwest::Url::parse(&self.manifest.base_url)
            .and_then(|base| base.join(&file.relative_path))
            .map_err(|error| ResourceInstallError::Http {
                path: file.relative_path.clone(),
                message: error.to_string(),
            })?;
        let mut request = self.client.get(url);
        if offset > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
        }
        let mut response = request.send().map_err(|error| ResourceInstallError::Http {
            path: file.relative_path.clone(),
            message: error.to_string(),
        })?;

        if offset > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            let expected = format!("bytes {offset}-{}/{}", file.size_bytes - 1, file.size_bytes);
            let actual = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            if actual.as_deref() != Some(expected.as_str()) {
                let _ignored = fs::remove_file(&partial_path);
                return Err(ResourceInstallError::InvalidContentRange { expected, actual });
            }
        } else if response.status() == reqwest::StatusCode::OK {
            offset = 0;
        } else {
            let status = response.status().as_u16();
            return Err(ResourceInstallError::UnexpectedHttpStatus {
                path: file.relative_path.clone(),
                status,
            });
        }

        let mut output = OpenOptions::new()
            .create(true)
            .write(true)
            .append(offset > 0)
            .truncate(offset == 0)
            .open(&partial_path)
            .map_err(|_| io_error("open partial file", &partial_path))?;
        let started = Instant::now();
        let mut written = offset;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            self.check_cancel(cancel)?;
            let count = response
                .read(&mut buffer)
                .map_err(|error| ResourceInstallError::Http {
                    path: file.relative_path.clone(),
                    message: error.to_string(),
                })?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .map_err(|_| io_error("write partial file", &partial_path))?;
            written = written.saturating_add(count as u64);
            if written > file.size_bytes {
                drop(output);
                let _ignored = fs::remove_file(&partial_path);
                return Err(ResourceInstallError::SizeMismatch {
                    path: file.relative_path.clone(),
                    expected: file.size_bytes,
                    actual: written,
                });
            }
            on_status(self.status(
                RuntimeResourceState::Downloading,
                Some(component),
                Some(file.relative_path.clone()),
                installed_bytes.saturating_add(written),
                total_bytes,
                bytes_per_second(written.saturating_sub(offset), started.elapsed()),
                installed_bytes,
                None,
            ));
        }
        output
            .sync_all()
            .map_err(|_| io_error("sync partial file", &partial_path))?;
        drop(output);
        self.verify_and_commit(
            file,
            &partial_path,
            on_status,
            component,
            total_bytes,
            cancel,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn copy_import_file(
        &self,
        component: RuntimeResourceComponent,
        file: &ManifestFile,
        source: &Path,
        installed_bytes: u64,
        total_bytes: u64,
        cancel: &AtomicBool,
        on_status: &mut impl FnMut(RuntimeResourceStatus),
    ) -> Result<(), ResourceInstallError> {
        let source_size = fs::metadata(source)
            .map_err(|_| io_error("read import metadata", source))?
            .len();
        if source_size != file.size_bytes {
            return Err(ResourceInstallError::SizeMismatch {
                path: file.relative_path.clone(),
                expected: file.size_bytes,
                actual: source_size,
            });
        }
        let partial_path = self.partial_path(file);
        if let Some(parent) = partial_path.parent() {
            fs::create_dir_all(parent).map_err(|_| io_error("create partial directory", parent))?;
        }
        let mut input = File::open(source).map_err(|_| io_error("open import file", source))?;
        let mut output = File::create(&partial_path)
            .map_err(|_| io_error("create partial file", &partial_path))?;
        let started = Instant::now();
        let mut written = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            self.check_cancel(cancel)?;
            let count = input
                .read(&mut buffer)
                .map_err(|_| io_error("read import file", source))?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .map_err(|_| io_error("write partial file", &partial_path))?;
            written = written.saturating_add(count as u64);
            on_status(self.status(
                RuntimeResourceState::Downloading,
                Some(component),
                Some(file.relative_path.clone()),
                installed_bytes.saturating_add(written),
                total_bytes,
                bytes_per_second(written, started.elapsed()),
                installed_bytes,
                None,
            ));
        }
        output
            .sync_all()
            .map_err(|_| io_error("sync partial file", &partial_path))?;
        drop(output);
        self.verify_and_commit(
            file,
            &partial_path,
            on_status,
            component,
            total_bytes,
            cancel,
        )
    }

    fn verify_and_commit(
        &self,
        file: &ManifestFile,
        partial_path: &Path,
        on_status: &mut impl FnMut(RuntimeResourceStatus),
        component: RuntimeResourceComponent,
        total_bytes: u64,
        cancel: &AtomicBool,
    ) -> Result<(), ResourceInstallError> {
        on_status(self.status(
            RuntimeResourceState::Verifying,
            Some(component),
            Some(file.relative_path.clone()),
            file.size_bytes,
            total_bytes,
            0,
            0,
            None,
        ));
        let actual_size = fs::metadata(partial_path)
            .map_err(|_| io_error("read partial metadata", partial_path))?
            .len();
        if actual_size != file.size_bytes {
            let _ignored = fs::remove_file(partial_path);
            return Err(ResourceInstallError::SizeMismatch {
                path: file.relative_path.clone(),
                expected: file.size_bytes,
                actual: actual_size,
            });
        }
        if hash_file_cancellable(partial_path, Some(cancel))? != file.sha256 {
            let _ignored = fs::remove_file(partial_path);
            return Err(ResourceInstallError::HashMismatch {
                path: file.relative_path.clone(),
            });
        }
        set_executable_if_needed(partial_path, file.executable)?;
        let final_path = self.final_path(file);
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent).map_err(|_| io_error("create final directory", parent))?;
        }
        fs::rename(partial_path, &final_path)
            .map_err(|_| io_error("commit verified file", &final_path))?;
        Ok(())
    }

    fn finish_operation(
        &self,
        component: RuntimeResourceComponent,
        result: Result<RuntimeResourceStatus, ResourceInstallError>,
        on_status: &mut impl FnMut(RuntimeResourceStatus),
    ) -> Result<RuntimeResourceStatus, ResourceInstallError> {
        match result {
            Ok(status) => {
                on_status(status.clone());
                Ok(status)
            }
            Err(error) => {
                let state = if error == ResourceInstallError::Cancelled {
                    RuntimeResourceState::Cancelled
                } else {
                    RuntimeResourceState::Failed
                };
                on_status(self.status(
                    state,
                    Some(component),
                    None,
                    0,
                    total_size(&self.required_files(component)),
                    0,
                    0,
                    Some(error.to_string()),
                ));
                Err(error)
            }
        }
    }

    fn write_installed_record(
        &self,
        component: RuntimeResourceComponent,
    ) -> Result<(), ResourceInstallError> {
        #[derive(Serialize)]
        struct InstalledRecord<'a> {
            schema_version: u32,
            release: &'a str,
            target: &'a str,
            installed_components: &'a [ManifestComponent],
        }
        let record = InstalledRecord {
            schema_version: 1,
            release: RELEASE,
            target: &self.manifest.target,
            installed_components: component.required_components(),
        };
        let bytes = serde_json::to_vec_pretty(&record).map_err(|error| {
            ResourceInstallError::Manifest(format!("failed to serialize installed record: {error}"))
        })?;
        let temporary = self
            .layout
            .installed_record
            .with_file_name("installed.json.tmp");
        let mut output = File::create(&temporary)
            .map_err(|_| io_error("create installed record", &temporary))?;
        output
            .write_all(&bytes)
            .map_err(|_| io_error("write installed record", &temporary))?;
        output
            .sync_all()
            .map_err(|_| io_error("sync installed record", &temporary))?;
        drop(output);
        fs::rename(&temporary, &self.layout.installed_record)
            .map_err(|_| io_error("commit installed record", &self.layout.installed_record))?;
        Ok(())
    }

    fn required_files(&self, component: RuntimeResourceComponent) -> Vec<&ManifestFile> {
        self.manifest
            .files
            .iter()
            .filter(|file| component.required_components().contains(&file.component))
            .collect()
    }

    fn installed_bytes(&self, files: &[&ManifestFile]) -> Result<u64, ResourceInstallError> {
        files.iter().try_fold(0_u64, |installed, file| {
            if file_matches(&self.final_path(file), file)? {
                Ok(installed.saturating_add(file.size_bytes))
            } else {
                Ok(installed)
            }
        })
    }

    fn additional_disk_bytes(&self, files: &[&ManifestFile]) -> Result<u64, ResourceInstallError> {
        files.iter().try_fold(0_u64, |required, file| {
            if file_matches(&self.final_path(file), file)? {
                return Ok(required);
            }
            let partial_size = fs::metadata(self.partial_path(file))
                .map(|metadata| metadata.len().min(file.size_bytes))
                .unwrap_or(0);
            Ok(required.saturating_add(file.size_bytes.saturating_sub(partial_size)))
        })
    }

    fn ensure_disk_space(&self, required: u64) -> Result<(), ResourceInstallError> {
        let disks = Disks::new_with_refreshed_list();
        let available = disks
            .list()
            .iter()
            .filter(|disk| self.layout.version_root.starts_with(disk.mount_point()))
            .max_by_key(|disk| disk.mount_point().as_os_str().len())
            .map(|disk| disk.available_space())
            .ok_or(ResourceInstallError::DiskUnavailable)?;
        if available < required {
            return Err(ResourceInstallError::InsufficientDiskSpace {
                required,
                available,
            });
        }
        Ok(())
    }

    fn acquire_operation(&self) -> Result<OperationGuard<'_>, ResourceInstallError> {
        self.operation_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ResourceInstallError::Busy)?;
        Ok(OperationGuard(&self.operation_running))
    }

    fn check_cancel(&self, cancel: &AtomicBool) -> Result<(), ResourceInstallError> {
        if cancel.load(Ordering::Acquire) {
            Err(ResourceInstallError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn final_path(&self, file: &ManifestFile) -> PathBuf {
        self.layout.version_root.join(&file.relative_path)
    }

    fn partial_path(&self, file: &ManifestFile) -> PathBuf {
        self.layout
            .partial_root
            .join(format!("{}.partial", file.relative_path))
    }

    fn ready_status(
        &self,
        component: RuntimeResourceComponent,
        total_bytes: u64,
    ) -> RuntimeResourceStatus {
        self.status(
            RuntimeResourceState::Ready,
            Some(component),
            None,
            total_bytes,
            total_bytes,
            0,
            total_bytes,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn status(
        &self,
        state: RuntimeResourceState,
        component: Option<RuntimeResourceComponent>,
        current_file: Option<String>,
        downloaded_bytes: u64,
        total_bytes: u64,
        bytes_per_second: u64,
        installed_bytes: u64,
        error: Option<String>,
    ) -> RuntimeResourceStatus {
        RuntimeResourceStatus {
            state,
            component,
            current_file,
            downloaded_bytes,
            total_bytes,
            bytes_per_second,
            installed_bytes,
            resource_root: self.layout.version_root.display().to_string(),
            error,
        }
    }
}

struct OperationGuard<'a>(&'a AtomicBool);

impl Drop for OperationGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn total_size(files: &[&ManifestFile]) -> u64 {
    files
        .iter()
        .fold(0_u64, |total, file| total.saturating_add(file.size_bytes))
}

fn file_matches(path: &Path, file: &ManifestFile) -> Result<bool, ResourceInstallError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(io_error("read file metadata", path)),
    };
    if !metadata.is_file() || metadata.len() != file.size_bytes {
        return Ok(false);
    }
    Ok(hash_file(path)? == file.sha256)
}

fn hash_file(path: &Path) -> Result<String, ResourceInstallError> {
    hash_file_cancellable(path, None)
}

fn hash_file_cancellable(
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<String, ResourceInstallError> {
    let mut input = File::open(path).map_err(|_| io_error("open file for hashing", path))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        if cancel.is_some_and(|value| value.load(Ordering::Acquire)) {
            return Err(ResourceInstallError::Cancelled);
        }
        let count = input
            .read(&mut buffer)
            .map_err(|_| io_error("read file for hashing", path))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let digest = hasher.finalize();
    let mut output = String::with_capacity(64);
    for byte in digest {
        use fmt::Write as _;
        write!(&mut output, "{byte:02x}").map_err(|_| io_error("format SHA-256", path))?;
    }
    Ok(output)
}

#[cfg(unix)]
fn set_executable_if_needed(path: &Path, executable: bool) -> Result<(), ResourceInstallError> {
    if executable {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|_| io_error("set executable permissions", path))?;
    }
    Ok(())
}

#[cfg(windows)]
fn set_executable_if_needed(_path: &Path, _executable: bool) -> Result<(), ResourceInstallError> {
    Ok(())
}

fn bytes_per_second(bytes: u64, elapsed: Duration) -> u64 {
    let nanos = elapsed.as_nanos().max(1);
    ((bytes as u128).saturating_mul(1_000_000_000) / nanos).min(u64::MAX as u128) as u64
}

fn io_error(operation: &'static str, path: &Path) -> ResourceInstallError {
    ResourceInstallError::Io {
        operation,
        path: path.display().to_string(),
    }
}
