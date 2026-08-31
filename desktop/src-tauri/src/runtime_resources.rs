use crate::bounded_io::{read_to_end_bounded, BoundedReadError};
use cap_std::{ambient_authority, fs::Dir};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use sysinfo::Disks;

pub const PRODUCTION_BASE_URL: &str = "http://101.96.208.132:7088/autolive-resources/v0.1.0/";
const RELEASE: &str = "v0.1.0";
const RUNTIME_RESOURCES_DIRECTORY: &str = "runtime-resources";
const EMBEDDED_RESOURCE_DIRECTORY: &str = "embedded-runtime-resources";
const MAX_HTTP_REQUESTS: usize = 3;
const MAX_RUNTIME_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_RUNTIME_MANIFEST_FILES: usize = 16;
const MAX_RUNTIME_MANIFEST_PATH_BYTES: usize = 512;
const MAX_RUNTIME_MANIFEST_DECLARED_BYTES: u64 = 1024 * 1024 * 1024;
const RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(25), Duration::from_millis(50)];
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
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeResourceComponent {
    Media,
}

impl RuntimeResourceComponent {
    pub fn required_components(self) -> &'static [ManifestComponent] {
        match self {
            Self::Media => &[ManifestComponent::Media],
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
        if bytes.len() > MAX_RUNTIME_MANIFEST_BYTES {
            return Err(ManifestValidationError(format!(
                "manifest exceeds {MAX_RUNTIME_MANIFEST_BYTES} byte limit"
            )));
        }
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
        if self.files.len() > MAX_RUNTIME_MANIFEST_FILES {
            return Err(ManifestValidationError(format!(
                "manifest contains more than {MAX_RUNTIME_MANIFEST_FILES} files"
            )));
        }
        let declared_bytes = self.files.iter().try_fold(0_u64, |total, file| {
            total.checked_add(file.size_bytes).ok_or_else(|| {
                ManifestValidationError("manifest declared size overflows u64".to_owned())
            })
        })?;
        if declared_bytes > MAX_RUNTIME_MANIFEST_DECLARED_BYTES {
            return Err(ManifestValidationError(format!(
                "manifest declares more than {MAX_RUNTIME_MANIFEST_DECLARED_BYTES} bytes"
            )));
        }

        let mut paths = HashSet::with_capacity(self.files.len());
        for file in &self.files {
            if file.relative_path.len() > MAX_RUNTIME_MANIFEST_PATH_BYTES {
                return Err(ManifestValidationError(format!(
                    "manifest path exceeds {MAX_RUNTIME_MANIFEST_PATH_BYTES} bytes"
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
        for component in [ManifestComponent::Media] {
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
        ValidationMode::Production => PRODUCTION_BASE_URL.to_owned(),
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
            let binaries_prefix = format!("{target}/binaries/");
            let licenses_prefix = format!("{target}/binaries/licenses/");
            file.relative_path.starts_with(&binaries_prefix)
                && file.executable == !file.relative_path.starts_with(&licenses_prefix)
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
            partial_root: version_root.join("partial"),
            installed_record: version_root.join("installed.json"),
            version_root,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeResourceRoots {
    pub version_root: PathBuf,
    pub target_root: PathBuf,
}

impl RuntimeResourceRoots {
    fn for_version_root(
        version_root: PathBuf,
        target: &str,
    ) -> Result<Self, ManifestValidationError> {
        if !SUPPORTED_TARGETS.contains(&target) {
            return Err(ManifestValidationError(
                "unsupported runtime resource target".to_owned(),
            ));
        }
        Ok(Self {
            target_root: version_root.join(target),
            version_root,
        })
    }

    fn from_writable_layout(layout: &RuntimeResourceLayout) -> Self {
        Self {
            version_root: layout.version_root.clone(),
            target_root: layout.target_root.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeResourceCatalog {
    manifest: Arc<RuntimeResourceManifest>,
    roots: RuntimeResourceRoots,
    bundled: bool,
}

impl RuntimeResourceCatalog {
    pub fn from_resource_directory(
        resource_dir: &Path,
        app_data_dir: &Path,
        expected_target: &str,
    ) -> Result<Self, ResourceInstallError> {
        let manifest = read_production_manifest(resource_dir, expected_target)?;
        let embedded_root = resource_dir.join(EMBEDDED_RESOURCE_DIRECTORY);
        let bundled = embedded_resources_complete(&manifest, &embedded_root);
        let roots = if bundled {
            RuntimeResourceRoots::for_version_root(embedded_root, expected_target)?
        } else {
            let layout = RuntimeResourceLayout::for_target(app_data_dir, expected_target)?;
            RuntimeResourceRoots::from_writable_layout(&layout)
        };
        Ok(Self {
            manifest: Arc::new(manifest),
            roots,
            bundled,
        })
    }

    pub fn roots(&self) -> &RuntimeResourceRoots {
        &self.roots
    }

    pub fn is_bundled(&self) -> bool {
        self.bundled
    }

    pub fn bundled_status(
        &self,
        component: RuntimeResourceComponent,
    ) -> Option<RuntimeResourceStatus> {
        if !self.bundled {
            return None;
        }
        let total_bytes = self
            .manifest
            .files
            .iter()
            .filter(|file| component.required_components().contains(&file.component))
            .fold(0_u64, |total, file| total.saturating_add(file.size_bytes));
        Some(RuntimeResourceStatus {
            state: RuntimeResourceState::Ready,
            component: Some(component),
            current_file: None,
            downloaded_bytes: 0,
            total_bytes,
            bytes_per_second: 0,
            installed_bytes: total_bytes,
            resource_root: self.roots.version_root.display().to_string(),
            error: None,
        })
    }
}

fn read_production_manifest(
    resource_dir: &Path,
    expected_target: &str,
) -> Result<RuntimeResourceManifest, ResourceInstallError> {
    let manifest_path = resource_dir.join("runtime-resources.json");
    let manifest_file = File::open(&manifest_path)
        .map_err(|error| io_error("open manifest", &manifest_path, error))?;
    let manifest_bytes = read_to_end_bounded(manifest_file, MAX_RUNTIME_MANIFEST_BYTES).map_err(
        |error| match error {
            BoundedReadError::Io(error) => io_error("read manifest", &manifest_path, error),
            BoundedReadError::LimitExceeded { limit } => ResourceInstallError::Manifest(format!(
                "runtime resource manifest {} exceeds {limit} byte limit",
                manifest_path.display()
            )),
        },
    )?;
    let manifest =
        RuntimeResourceManifest::parse_and_validate(&manifest_bytes, ValidationMode::Production)
            .map_err(|error| {
                ResourceInstallError::Manifest(format!(
                    "failed to parse runtime resource manifest {}: {error}",
                    manifest_path.display()
                ))
            })?;
    if manifest.target != expected_target {
        return Err(ResourceInstallError::Manifest(format!(
            "runtime resource manifest target mismatch: expected {expected_target}, got {} ({})",
            manifest.target,
            manifest_path.display()
        )));
    }
    Ok(manifest)
}

fn embedded_resources_complete(manifest: &RuntimeResourceManifest, embedded_root: &Path) -> bool {
    embedded_root.is_dir()
        && manifest.files.iter().all(|file| {
            fs::metadata(embedded_root.join(&file.relative_path))
                .map(|metadata| metadata.is_file() && metadata.len() == file.size_bytes)
                .unwrap_or(false)
        })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceInstallError {
    Manifest(String),
    Busy,
    Cancelled,
    Io {
        operation: &'static str,
        path: String,
        message: String,
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
    DiskRequirementOverflow,
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
            Self::Io {
                operation,
                path,
                message,
            } => {
                write!(
                    formatter,
                    "failed to {operation} runtime resource {path}: {message}"
                )
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
            Self::DiskRequirementOverflow => {
                formatter.write_str("runtime resource disk requirement exceeds u64")
            }
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
    app_data_capability: Arc<Dir>,
    client: reqwest::blocking::Client,
    operation_key: PathBuf,
}

impl RuntimeResourceInstaller {
    pub fn from_resource_directory(
        resource_dir: &Path,
        app_data_dir: &Path,
        expected_target: &str,
    ) -> Result<Self, ResourceInstallError> {
        let manifest = read_production_manifest(resource_dir, expected_target)?;
        Self::from_manifest(manifest, app_data_dir)
    }

    pub fn from_embedded(
        manifest_bytes: &[u8],
        app_data_dir: &Path,
    ) -> Result<Self, ResourceInstallError> {
        let manifest = RuntimeResourceManifest::parse_and_validate(
            manifest_bytes,
            ValidationMode::Production,
        )?;
        Self::from_manifest(manifest, app_data_dir)
    }

    pub fn resource_root(&self) -> &Path {
        &self.layout.version_root
    }

    pub fn target(&self) -> &str {
        &self.manifest.target
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

    pub(crate) fn from_manifest(
        manifest: RuntimeResourceManifest,
        app_data_dir: &Path,
    ) -> Result<Self, ResourceInstallError> {
        fs::create_dir_all(app_data_dir)
            .map_err(|error| io_error("create app data directory", app_data_dir, error))?;
        let app_data_capability = Arc::new(
            Dir::open_ambient_dir(app_data_dir, ambient_authority())
                .map_err(|error| io_error("open app data directory", app_data_dir, error))?,
        );
        let layout = RuntimeResourceLayout::for_target(app_data_dir, &manifest.target)?;
        let operation_key = operation_key(&layout.version_root)?;
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
            app_data_capability,
            client,
            operation_key,
        })
    }

    pub fn inspect(
        &self,
        component: RuntimeResourceComponent,
    ) -> Result<RuntimeResourceStatus, ResourceInstallError> {
        let files = self.required_files(component);
        let total_bytes = total_size(&files);
        let (installed_bytes, downloaded_bytes, files_complete) = self.progress_bytes(&files)?;
        Ok(self.status(
            if files_complete {
                RuntimeResourceState::Ready
            } else {
                RuntimeResourceState::NotInstalled
            },
            Some(component),
            None,
            downloaded_bytes,
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
        let mut last_status = None;
        let result = self.install_inner(component, cancel, &mut |status| {
            last_status = Some(status.clone());
            on_status(status);
        });
        self.finish_operation(component, result, &mut on_status, last_status)
    }

    pub fn import_directory(
        &self,
        component: RuntimeResourceComponent,
        source_root: &Path,
        cancel: &AtomicBool,
        mut on_status: impl FnMut(RuntimeResourceStatus),
    ) -> Result<RuntimeResourceStatus, ResourceInstallError> {
        let _operation = self.acquire_operation()?;
        let mut last_status = None;
        let result = self.import_inner(component, source_root, cancel, &mut |status| {
            last_status = Some(status.clone());
            on_status(status);
        });
        self.finish_operation(component, result, &mut on_status, last_status)
    }

    pub fn clear_current_release(
        &self,
        cancel: &AtomicBool,
        mut on_status: impl FnMut(RuntimeResourceStatus),
    ) -> Result<RuntimeResourceStatus, ResourceInstallError> {
        let _operation = self.acquire_operation()?;
        let result = self.clear_current_release_inner(cancel, &mut on_status);
        let (state, error) = match &result {
            Ok(()) => (RuntimeResourceState::NotInstalled, None),
            Err(ResourceInstallError::Cancelled) => (
                RuntimeResourceState::Cancelled,
                Some(ResourceInstallError::Cancelled.to_string()),
            ),
            Err(error) => (RuntimeResourceState::Failed, Some(error.to_string())),
        };
        let status = self.status(state, None, None, 0, 0, 0, 0, error);
        on_status(status.clone());
        result.map(|()| status)
    }

    fn clear_current_release_inner(
        &self,
        cancel: &AtomicBool,
        on_status: &mut impl FnMut(RuntimeResourceStatus),
    ) -> Result<(), ResourceInstallError> {
        self.check_cancel(cancel)?;
        let parent_path =
            self.layout
                .version_root
                .parent()
                .ok_or_else(|| ResourceInstallError::Io {
                    operation: "resolve clear root parent",
                    path: self.layout.version_root.display().to_string(),
                    message: "resource release root has no parent directory".to_owned(),
                })?;
        let parent = match self
            .app_data_capability
            .open_dir(RUNTIME_RESOURCES_DIRECTORY)
        {
            Ok(parent) => parent,
            Err(_) => {
                return remove_capability_entry(
                    &self.app_data_capability,
                    std::ffi::OsStr::new(RUNTIME_RESOURCES_DIRECTORY),
                    parent_path,
                    "clear runtime resources entry",
                );
            }
        };
        let root = match parent.open_dir(RELEASE) {
            Ok(root) => root,
            Err(_) => {
                return remove_capability_entry(
                    &parent,
                    std::ffi::OsStr::new(RELEASE),
                    &self.layout.version_root,
                    "clear release root entry",
                );
            }
        };

        remove_directory_contents(
            &root,
            &self.layout.version_root,
            Path::new(""),
            cancel,
            &mut |relative_path| {
                on_status(self.status(
                    RuntimeResourceState::Checking,
                    None,
                    Some(relative_path.display().to_string()),
                    0,
                    0,
                    0,
                    0,
                    None,
                ));
            },
        )?;
        drop(root);
        self.check_cancel(cancel)?;
        remove_capability_entry(
            &parent,
            std::ffi::OsStr::new(RELEASE),
            &self.layout.version_root,
            "clear release root",
        )
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
        let (mut installed_bytes, downloaded_bytes, files_complete) =
            self.progress_bytes(&files)?;
        if !files_complete {
            self.invalidate_installed_record()?;
        }
        on_status(self.status(
            RuntimeResourceState::Checking,
            Some(component),
            None,
            downloaded_bytes,
            total_bytes,
            0,
            installed_bytes,
            None,
        ));
        if files_complete {
            self.write_installed_record()?;
            return Ok(self.ready_status(component, total_bytes));
        }

        fs::create_dir_all(&self.layout.version_root)
            .map_err(|error| io_error("create directory", &self.layout.version_root, error))?;
        let required = required_disk_space(self.additional_disk_bytes(&files)?)?;
        self.ensure_disk_space(required)?;

        for file in files {
            self.check_cancel(cancel)?;
            let final_path = self.final_path(file);
            if file_matches(&final_path, file)? {
                continue;
            }
            if final_path.exists() {
                fs::remove_file(&final_path)
                    .map_err(|error| io_error("remove invalid file", &final_path, error))?;
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
        self.write_installed_record()?;
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
            .map_err(|error| io_error("open import directory", source_root, error))?;
        let source_dir = Dir::open_ambient_dir(&canonical_root, ambient_authority())
            .map_err(|error| io_error("open import directory", source_root, error))?;
        let files = self.required_files(component);
        let total_bytes = total_size(&files);
        let (mut installed_bytes, downloaded_bytes, files_complete) =
            self.progress_bytes(&files)?;
        if !files_complete {
            self.invalidate_installed_record()?;
        }
        on_status(self.status(
            RuntimeResourceState::Checking,
            Some(component),
            None,
            downloaded_bytes,
            total_bytes,
            0,
            installed_bytes,
            None,
        ));
        if files_complete {
            self.write_installed_record()?;
            return Ok(self.ready_status(component, total_bytes));
        }

        fs::create_dir_all(&self.layout.version_root)
            .map_err(|error| io_error("create directory", &self.layout.version_root, error))?;
        self.ensure_disk_space(required_disk_space(self.additional_disk_bytes(&files)?)?)?;
        for file in files {
            self.check_cancel(cancel)?;
            let final_path = self.final_path(file);
            if file_matches(&final_path, file)? {
                continue;
            }
            if final_path.exists() {
                fs::remove_file(&final_path)
                    .map_err(|error| io_error("remove invalid file", &final_path, error))?;
            }
            let relative_path = Path::new(&file.relative_path);
            let source_file = source_dir
                .open(relative_path)
                .map_err(|error| io_error("open import file", relative_path, error))?
                .into_std();
            self.copy_import_file(
                component,
                file,
                relative_path,
                source_file,
                installed_bytes,
                total_bytes,
                cancel,
                on_status,
            )?;
            installed_bytes = installed_bytes.saturating_add(file.size_bytes);
        }
        self.write_installed_record()?;
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
            fs::create_dir_all(parent)
                .map_err(|error| io_error("create partial directory", parent, error))?;
        }
        let url = reqwest::Url::parse(&self.manifest.base_url)
            .and_then(|base| base.join(&file.relative_path))
            .map_err(|error| ResourceInstallError::Http {
                path: file.relative_path.clone(),
                message: error.to_string(),
            })?;
        let mut reset_from_remote_change = false;

        'requests: for request_index in 0..MAX_HTTP_REQUESTS {
            let retry_delay = RETRY_DELAYS.get(request_index).copied();
            self.check_cancel(cancel)?;
            let mut offset = file_size_if_present(&partial_path)?;
            if offset > file.size_bytes {
                fs::remove_file(&partial_path)
                    .map_err(|error| io_error("remove oversized partial", &partial_path, error))?;
                offset = 0;
            }
            if offset == file.size_bytes && partial_path.is_file() {
                return self.verify_and_commit(
                    file,
                    &partial_path,
                    on_status,
                    component,
                    total_bytes,
                    cancel,
                );
            }

            let mut request = self.client.get(url.clone());
            if offset > 0 {
                request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
            }
            let mut response = match request.send() {
                Ok(response) => response,
                Err(error) => {
                    let error = ResourceInstallError::Http {
                        path: file.relative_path.clone(),
                        message: error.to_string(),
                    };
                    let Some(retry_delay) = retry_delay else {
                        return Err(error);
                    };
                    self.wait_for_retry(cancel, retry_delay)?;
                    continue;
                }
            };
            let status = response.status();
            if is_transient_status(status) {
                let error = ResourceInstallError::UnexpectedHttpStatus {
                    path: file.relative_path.clone(),
                    status: status.as_u16(),
                };
                let Some(retry_delay) = retry_delay else {
                    return Err(error);
                };
                self.wait_for_retry(cancel, retry_delay)?;
                continue;
            }

            if offset > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT {
                let expected_range =
                    format!("bytes {offset}-{}/{}", file.size_bytes - 1, file.size_bytes);
                let actual_range = response
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                let expected_length = file.size_bytes - offset;
                let actual_length = content_length(&response);
                let remote_error = if actual_range.as_deref() != Some(expected_range.as_str()) {
                    Some(ResourceInstallError::InvalidContentRange {
                        expected: expected_range,
                        actual: actual_range,
                    })
                } else if actual_length != Some(expected_length) {
                    Some(ResourceInstallError::SizeMismatch {
                        path: file.relative_path.clone(),
                        expected: expected_length,
                        actual: actual_length.unwrap_or(0),
                    })
                } else {
                    None
                };
                if let Some(error) = remote_error {
                    remove_file_if_present(&partial_path, "discard changed remote partial")?;
                    if !reset_from_remote_change && request_index + 1 < MAX_HTTP_REQUESTS {
                        reset_from_remote_change = true;
                        continue;
                    }
                    return Err(error);
                }
            } else if status == reqwest::StatusCode::OK {
                offset = 0;
                let actual_length = content_length(&response);
                if actual_length != Some(file.size_bytes) {
                    remove_file_if_present(&partial_path, "discard changed remote partial")?;
                    return Err(ResourceInstallError::SizeMismatch {
                        path: file.relative_path.clone(),
                        expected: file.size_bytes,
                        actual: actual_length.unwrap_or(0),
                    });
                }
            } else {
                return Err(ResourceInstallError::UnexpectedHttpStatus {
                    path: file.relative_path.clone(),
                    status: status.as_u16(),
                });
            }

            let mut output = OpenOptions::new()
                .create(true)
                .write(true)
                .append(offset > 0)
                .truncate(offset == 0)
                .open(&partial_path)
                .map_err(|error| io_error("open partial file", &partial_path, error))?;
            let started = Instant::now();
            let mut written = offset;
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                self.check_cancel(cancel)?;
                let count = match response.read(&mut buffer) {
                    Ok(count) => count,
                    Err(error) => {
                        output.sync_all().map_err(|sync_error| {
                            io_error("sync partial file", &partial_path, sync_error)
                        })?;
                        drop(output);
                        let Some(retry_delay) = retry_delay else {
                            return Err(ResourceInstallError::Http {
                                path: file.relative_path.clone(),
                                message: error.to_string(),
                            });
                        };
                        self.wait_for_retry(cancel, retry_delay)?;
                        continue 'requests;
                    }
                };
                if count == 0 {
                    break;
                }
                output
                    .write_all(&buffer[..count])
                    .map_err(|error| io_error("write partial file", &partial_path, error))?;
                written = written.saturating_add(count as u64);
                if written > file.size_bytes {
                    drop(output);
                    remove_file_if_present(&partial_path, "remove oversized partial")?;
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
                .map_err(|error| io_error("sync partial file", &partial_path, error))?;
            drop(output);
            return self.verify_and_commit(
                file,
                &partial_path,
                on_status,
                component,
                total_bytes,
                cancel,
            );
        }
        Err(ResourceInstallError::Http {
            path: file.relative_path.clone(),
            message: "HTTP retry budget exhausted".to_owned(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn copy_import_file(
        &self,
        component: RuntimeResourceComponent,
        file: &ManifestFile,
        source: &Path,
        mut source_file: File,
        installed_bytes: u64,
        total_bytes: u64,
        cancel: &AtomicBool,
        on_status: &mut impl FnMut(RuntimeResourceStatus),
    ) -> Result<(), ResourceInstallError> {
        let source_size = source_file
            .metadata()
            .map_err(|error| io_error("read import metadata", source, error))?;
        if !source_size.is_file() {
            return Err(ResourceInstallError::UnsafeImportSource {
                path: source.display().to_string(),
            });
        }
        let source_size = source_size.len();
        if source_size != file.size_bytes {
            return Err(ResourceInstallError::SizeMismatch {
                path: file.relative_path.clone(),
                expected: file.size_bytes,
                actual: source_size,
            });
        }
        let partial_path = self.partial_path(file);
        if let Some(parent) = partial_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| io_error("create partial directory", parent, error))?;
        }
        let mut output = File::create(&partial_path)
            .map_err(|error| io_error("create partial file", &partial_path, error))?;
        let started = Instant::now();
        let mut written = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            self.check_cancel(cancel)?;
            let count = source_file
                .read(&mut buffer)
                .map_err(|error| io_error("read import file", source, error))?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .map_err(|error| io_error("write partial file", &partial_path, error))?;
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
            .map_err(|error| io_error("sync partial file", &partial_path, error))?;
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
            .map_err(|error| io_error("read partial metadata", partial_path, error))?
            .len();
        if actual_size != file.size_bytes {
            remove_untrusted_partial(partial_path)?;
            return Err(ResourceInstallError::SizeMismatch {
                path: file.relative_path.clone(),
                expected: file.size_bytes,
                actual: actual_size,
            });
        }
        if hash_file_cancellable(partial_path, Some(cancel))? != file.sha256 {
            remove_untrusted_partial(partial_path)?;
            return Err(ResourceInstallError::HashMismatch {
                path: file.relative_path.clone(),
            });
        }
        set_executable_if_needed(partial_path, file.executable)?;
        let final_path = self.final_path(file);
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| io_error("create final directory", parent, error))?;
        }
        fs::rename(partial_path, &final_path)
            .map_err(|error| io_error("commit verified file", &final_path, error))?;
        Ok(())
    }

    fn finish_operation(
        &self,
        component: RuntimeResourceComponent,
        result: Result<RuntimeResourceStatus, ResourceInstallError>,
        on_status: &mut impl FnMut(RuntimeResourceStatus),
        last_status: Option<RuntimeResourceStatus>,
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
                let mut status = last_status.unwrap_or_else(|| {
                    self.status(
                        RuntimeResourceState::Checking,
                        Some(component),
                        None,
                        0,
                        total_size(&self.required_files(component)),
                        0,
                        0,
                        None,
                    )
                });
                status.state = state;
                status.bytes_per_second = 0;
                status.error = Some(error.to_string());
                on_status(status);
                Err(error)
            }
        }
    }

    fn write_installed_record(&self) -> Result<(), ResourceInstallError> {
        #[derive(Serialize)]
        struct InstalledRecord<'a> {
            schema_version: u32,
            release: &'a str,
            target: &'a str,
            installed_components: &'a [ManifestComponent],
        }
        let installed_components = self.verified_components()?;
        let record = InstalledRecord {
            schema_version: 1,
            release: RELEASE,
            target: &self.manifest.target,
            installed_components: &installed_components,
        };
        let bytes = serde_json::to_vec_pretty(&record).map_err(|error| {
            ResourceInstallError::Manifest(format!("failed to serialize installed record: {error}"))
        })?;
        let temporary = self
            .layout
            .installed_record
            .with_file_name("installed.json.tmp");
        let mut output = File::create(&temporary)
            .map_err(|error| io_error("create installed record", &temporary, error))?;
        output
            .write_all(&bytes)
            .map_err(|error| io_error("write installed record", &temporary, error))?;
        output
            .sync_all()
            .map_err(|error| io_error("sync installed record", &temporary, error))?;
        drop(output);
        fs::rename(&temporary, &self.layout.installed_record).map_err(|error| {
            io_error(
                "commit installed record",
                &self.layout.installed_record,
                error,
            )
        })?;
        Ok(())
    }

    fn invalidate_installed_record(&self) -> Result<(), ResourceInstallError> {
        remove_file_if_present(&self.layout.installed_record, "invalidate installed record")
    }

    fn verified_components(&self) -> Result<Vec<ManifestComponent>, ResourceInstallError> {
        let mut verified = Vec::with_capacity(1);
        for component in [ManifestComponent::Media] {
            let mut component_verified = true;
            for file in self
                .manifest
                .files
                .iter()
                .filter(|file| file.component == component)
            {
                if !file_matches(&self.final_path(file), file)? {
                    component_verified = false;
                    break;
                }
            }
            if component_verified {
                verified.push(component);
            }
        }
        Ok(verified)
    }

    fn required_files(&self, component: RuntimeResourceComponent) -> Vec<&ManifestFile> {
        self.manifest
            .files
            .iter()
            .filter(|file| component.required_components().contains(&file.component))
            .collect()
    }

    fn progress_bytes(
        &self,
        files: &[&ManifestFile],
    ) -> Result<(u64, u64, bool), ResourceInstallError> {
        files.iter().try_fold(
            (0_u64, 0_u64, true),
            |(installed, downloaded, complete), file| {
                if file_matches(&self.final_path(file), file)? {
                    return Ok((
                        installed.saturating_add(file.size_bytes),
                        downloaded.saturating_add(file.size_bytes),
                        complete,
                    ));
                }
                let partial_size =
                    file_size_if_present(&self.partial_path(file))?.min(file.size_bytes);
                Ok((installed, downloaded.saturating_add(partial_size), false))
            },
        )
    }

    fn additional_disk_bytes(&self, files: &[&ManifestFile]) -> Result<u64, ResourceInstallError> {
        files.iter().try_fold(0_u64, |required, file| {
            if file_matches(&self.final_path(file), file)? {
                return Ok(required);
            }
            let partial_size = file_size_if_present(&self.partial_path(file))?.min(file.size_bytes);
            required
                .checked_add(file.size_bytes - partial_size)
                .ok_or(ResourceInstallError::DiskRequirementOverflow)
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

    fn acquire_operation(&self) -> Result<OperationGuard, ResourceInstallError> {
        let mut roots = operation_registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !roots.insert(self.operation_key.clone()) {
            return Err(ResourceInstallError::Busy);
        }
        Ok(OperationGuard {
            key: self.operation_key.clone(),
        })
    }

    fn check_cancel(&self, cancel: &AtomicBool) -> Result<(), ResourceInstallError> {
        if cancel.load(Ordering::Acquire) {
            Err(ResourceInstallError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn wait_for_retry(
        &self,
        cancel: &AtomicBool,
        delay: Duration,
    ) -> Result<(), ResourceInstallError> {
        let started = Instant::now();
        while started.elapsed() < delay {
            self.check_cancel(cancel)?;
            let remaining = delay.saturating_sub(started.elapsed());
            std::thread::sleep(remaining.min(Duration::from_millis(10)));
        }
        self.check_cancel(cancel)
    }

    fn final_path(&self, file: &ManifestFile) -> PathBuf {
        self.layout.version_root.join(&file.relative_path)
    }

    fn partial_path(&self, file: &ManifestFile) -> PathBuf {
        self.layout
            .partial_root
            .join(format!("{}.partial", file.sha256))
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

struct OperationGuard {
    key: PathBuf,
}

fn operation_registry() -> &'static Mutex<HashSet<PathBuf>> {
    static RUNNING_ROOTS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    RUNNING_ROOTS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn operation_key(version_root: &Path) -> Result<PathBuf, ResourceInstallError> {
    fs::create_dir_all(version_root)
        .map_err(|error| io_error("create resource directory", version_root, error))?;
    fs::canonicalize(version_root)
        .map_err(|error| io_error("resolve resource directory", version_root, error))
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        operation_registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.key);
    }
}

fn remove_directory_contents(
    directory: &Dir,
    display_root: &Path,
    relative_directory: &Path,
    cancel: &AtomicBool,
    on_entry: &mut impl FnMut(&Path),
) -> Result<(), ResourceInstallError> {
    let display_directory = display_root.join(relative_directory);
    let entries = directory
        .entries()
        .map_err(|error| io_error("read clear directory", &display_directory, error))?;
    for entry in entries {
        if cancel.load(Ordering::Acquire) {
            return Err(ResourceInstallError::Cancelled);
        }
        let entry =
            entry.map_err(|error| io_error("read clear entry", &display_directory, error))?;
        let name = entry.file_name();
        let relative = relative_directory.join(&name);
        on_entry(&relative);
        if cancel.load(Ordering::Acquire) {
            return Err(ResourceInstallError::Cancelled);
        }
        let display_path = display_root.join(&relative);
        match directory.open_dir(&name) {
            Ok(child) => {
                remove_directory_contents(&child, display_root, &relative, cancel, on_entry)?;
                drop(child);
                if cancel.load(Ordering::Acquire) {
                    return Err(ResourceInstallError::Cancelled);
                }
                remove_capability_entry(directory, &name, &display_path, "clear directory")?;
            }
            Err(_) => remove_capability_entry(directory, &name, &display_path, "clear file entry")?,
        }
    }
    Ok(())
}

fn remove_capability_entry(
    parent: &Dir,
    name: &std::ffi::OsStr,
    display_path: &Path,
    operation: &'static str,
) -> Result<(), ResourceInstallError> {
    let file_error = match parent.remove_file(name) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    let directory_error = match parent.remove_dir(name) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    if file_error.kind() == std::io::ErrorKind::NotFound
        && directory_error.kind() == std::io::ErrorKind::NotFound
    {
        return Ok(());
    }
    Err(ResourceInstallError::Io {
        operation,
        path: display_path.display().to_string(),
        message: format!("remove_file failed: {file_error}; remove_dir failed: {directory_error}"),
    })
}

fn total_size(files: &[&ManifestFile]) -> u64 {
    files
        .iter()
        .fold(0_u64, |total, file| total.saturating_add(file.size_bytes))
}

pub fn required_disk_space(missing_bytes: u64) -> Result<u64, ResourceInstallError> {
    const FIXED_RESERVE: u64 = 512 * 1024 * 1024;
    let ten_percent_rounded_up = missing_bytes / 10 + u64::from(!missing_bytes.is_multiple_of(10));
    missing_bytes
        .checked_add(FIXED_RESERVE.max(ten_percent_rounded_up))
        .ok_or(ResourceInstallError::DiskRequirementOverflow)
}

fn is_transient_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

fn content_length(response: &reqwest::blocking::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

fn file_size_if_present(path: &Path) -> Result<u64, ResourceInstallError> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(io_error("read partial metadata", path, error)),
    }
}

fn remove_file_if_present(
    path: &Path,
    operation: &'static str,
) -> Result<(), ResourceInstallError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(operation, path, error)),
    }
}

fn remove_untrusted_partial(path: &Path) -> Result<(), ResourceInstallError> {
    fs::remove_file(path).map_err(|error| io_error("remove untrusted partial", path, error))
}

fn file_matches(path: &Path, file: &ManifestFile) -> Result<bool, ResourceInstallError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io_error("read file metadata", path, error)),
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
    let mut input =
        File::open(path).map_err(|error| io_error("open file for hashing", path, error))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        if cancel.is_some_and(|value| value.load(Ordering::Acquire)) {
            return Err(ResourceInstallError::Cancelled);
        }
        let count = input
            .read(&mut buffer)
            .map_err(|error| io_error("read file for hashing", path, error))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let digest = hasher.finalize();
    let mut output = String::with_capacity(64);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(output)
}

#[cfg(unix)]
fn set_executable_if_needed(path: &Path, executable: bool) -> Result<(), ResourceInstallError> {
    if executable {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|error| io_error("set executable permissions", path, error))?;
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

fn io_error(operation: &'static str, path: &Path, error: std::io::Error) -> ResourceInstallError {
    ResourceInstallError::Io {
        operation,
        path: path.display().to_string(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        remove_capability_entry, ManifestComponent, ManifestFile, ResourceInstallError,
        RuntimeResourceComponent, RuntimeResourceInstaller, RuntimeResourceManifest,
        RuntimeResourceState, RuntimeResourceStatus, ValidationMode, MAX_RUNTIME_MANIFEST_BYTES,
        PRODUCTION_BASE_URL, RELEASE,
    };
    use cap_std::{ambient_authority, fs::Dir};
    use serde_json::json;
    use std::ffi::OsStr;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn manifest_bytes(files: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "release": RELEASE,
            "target": "aarch64-apple-darwin",
            "base_url": PRODUCTION_BASE_URL,
            "files": files
        }))
        .expect("manifest fixture should serialize")
    }

    fn manifest_file(relative_path: String, size_bytes: u64) -> serde_json::Value {
        json!({
            "component": "media",
            "relative_path": relative_path,
            "size_bytes": size_bytes,
            "sha256": "00".repeat(32),
            "executable": true
        })
    }

    #[test]
    fn manifest_limits_input_to_256_kib() {
        let mut bytes = manifest_bytes(json!([manifest_file(
            "aarch64-apple-darwin/binaries/ffmpeg".to_owned(),
            1,
        )]));
        bytes.resize(MAX_RUNTIME_MANIFEST_BYTES + 1, b' ');

        let error = RuntimeResourceManifest::parse_and_validate(&bytes, ValidationMode::Production)
            .expect_err("oversized manifest must be rejected before parsing");
        assert!(error.to_string().contains("262144 byte limit"));
    }

    #[test]
    fn manifest_limits_file_count_path_length_and_declared_size() {
        let files = (0..17)
            .map(|index| manifest_file(format!("aarch64-apple-darwin/binaries/ffmpeg-{index}"), 1))
            .collect::<Vec<_>>();
        let error = RuntimeResourceManifest::parse_and_validate(
            &manifest_bytes(json!(files)),
            ValidationMode::Production,
        )
        .expect_err("17 files must be rejected");
        assert!(error.to_string().contains("more than 16 files"));

        let prefix = "aarch64-apple-darwin/binaries/";
        let path = format!("{prefix}{}", "a".repeat(513 - prefix.len()));
        let error = RuntimeResourceManifest::parse_and_validate(
            &manifest_bytes(json!([manifest_file(path, 1)])),
            ValidationMode::Production,
        )
        .expect_err("513-byte path must be rejected");
        assert!(error.to_string().contains("512 bytes"));

        let error = RuntimeResourceManifest::parse_and_validate(
            &manifest_bytes(json!([manifest_file(
                "aarch64-apple-darwin/binaries/ffmpeg".to_owned(),
                1024_u64 * 1024 * 1024 + 1,
            )])),
            ValidationMode::Production,
        )
        .expect_err("declared size above one GiB must be rejected");
        assert!(error.to_string().contains("1073741824 bytes"));
    }

    #[test]
    fn capability_entry_removal_handles_files_directories_and_keeps_both_errors() {
        let root = std::env::temp_dir().join(format!(
            "autolive-capability-entry-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("empty-directory")).expect("directory fixture");
        fs::create_dir_all(root.join("non-empty-directory")).expect("directory fixture");
        fs::write(root.join("non-empty-directory/child"), b"fixture")
            .expect("directory child fixture");
        fs::write(root.join("file"), b"fixture").expect("file fixture");
        let directory =
            Dir::open_ambient_dir(&root, ambient_authority()).expect("directory capability");

        remove_capability_entry(
            &directory,
            OsStr::new("file"),
            &root.join("file"),
            "clear test entry",
        )
        .expect("file should be removed");
        remove_capability_entry(
            &directory,
            OsStr::new("empty-directory"),
            &root.join("empty-directory"),
            "clear test entry",
        )
        .expect("directory should be removed");
        let error = remove_capability_entry(
            &directory,
            OsStr::new("non-empty-directory"),
            &root.join("non-empty-directory"),
            "clear test entry",
        )
        .expect_err("both removal forms should fail for a non-empty directory");

        let ResourceInstallError::Io { message, .. } = error else {
            panic!("non-empty directory should return an IO error");
        };
        assert!(message.contains("remove_file"));
        assert!(message.contains("remove_dir"));
        drop(directory);
        fs::remove_dir_all(root).expect("test directory cleanup");
    }

    #[test]
    fn installer_remains_clone_send_and_sync() {
        fn assert_contract<T: Clone + Send + Sync>() {}
        assert_contract::<RuntimeResourceInstaller>();

        let root = std::env::temp_dir().join(format!(
            "autolive-installer-clone-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let installer = RuntimeResourceInstaller::from_manifest(
            RuntimeResourceManifest {
                schema_version: 1,
                release: RELEASE.to_owned(),
                target: "aarch64-apple-darwin".to_owned(),
                base_url: PRODUCTION_BASE_URL.to_owned(),
                files: Vec::new(),
            },
            &root,
        )
        .expect("installer fixture");
        let cloned = installer.clone();

        assert!(std::sync::Arc::ptr_eq(
            &installer.app_data_capability,
            &cloned.app_data_capability
        ));
        drop((installer, cloned));
        fs::remove_dir_all(root).expect("test directory cleanup");
    }

    #[test]
    fn inspect_includes_partial_file_bytes_in_download_progress() {
        let root = std::env::temp_dir().join(format!(
            "autolive-runtime-partial-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let file = ManifestFile {
            component: ManifestComponent::Media,
            relative_path: "x86_64-pc-windows-msvc/binaries/ffmpeg.exe".to_owned(),
            size_bytes: 5,
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_owned(),
            executable: true,
        };
        let installer = RuntimeResourceInstaller::from_manifest(
            RuntimeResourceManifest {
                schema_version: 1,
                release: RELEASE.to_owned(),
                target: "x86_64-pc-windows-msvc".to_owned(),
                base_url: PRODUCTION_BASE_URL.to_owned(),
                files: vec![file.clone()],
            },
            &root,
        )
        .expect("installer fixture");
        let partial_path = installer.partial_path(&file);
        fs::create_dir_all(partial_path.parent().expect("partial parent"))
            .expect("partial directory");
        fs::write(&partial_path, b"hel").expect("partial file");

        let status = installer
            .inspect(RuntimeResourceComponent::Media)
            .expect("resource status");
        assert_eq!(status.state, RuntimeResourceState::NotInstalled);
        assert_eq!(status.downloaded_bytes, 3);
        assert_eq!(status.installed_bytes, 0);
        assert_eq!(status.total_bytes, 5);

        drop(installer);
        fs::remove_dir_all(root).expect("test directory cleanup");
    }

    #[test]
    fn failed_operation_preserves_last_download_progress_for_resume() {
        let root = std::env::temp_dir().join(format!(
            "autolive-runtime-failure-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let installer = RuntimeResourceInstaller::from_manifest(
            RuntimeResourceManifest {
                schema_version: 1,
                release: RELEASE.to_owned(),
                target: "x86_64-pc-windows-msvc".to_owned(),
                base_url: PRODUCTION_BASE_URL.to_owned(),
                files: Vec::new(),
            },
            &root,
        )
        .expect("installer fixture");
        let last = RuntimeResourceStatus {
            state: RuntimeResourceState::Downloading,
            component: Some(RuntimeResourceComponent::Media),
            current_file: Some("x86_64-pc-windows-msvc/binaries/ffmpeg.exe".to_owned()),
            downloaded_bytes: 7,
            total_bytes: 10,
            bytes_per_second: 4,
            installed_bytes: 3,
            resource_root: installer.resource_root().display().to_string(),
            error: None,
        };
        let mut emitted = Vec::new();
        let result = installer.finish_operation(
            RuntimeResourceComponent::Media,
            Err(ResourceInstallError::Cancelled),
            &mut |status| emitted.push(status),
            Some(last),
        );
        assert_eq!(result, Err(ResourceInstallError::Cancelled));
        let status = emitted.pop().expect("terminal status");
        assert_eq!(status.state, RuntimeResourceState::Cancelled);
        assert_eq!(status.downloaded_bytes, 7);
        assert_eq!(status.installed_bytes, 3);
        assert_eq!(status.total_bytes, 10);
        assert!(status.error.is_some());

        drop(installer);
        fs::remove_dir_all(root).expect("test directory cleanup");
    }
}
