use autolive_desktop_core::runtime_resources::{
    required_disk_space, ManifestComponent, ResourceInstallError, RuntimeResourceComponent,
    RuntimeResourceInstaller, RuntimeResourceLayout, RuntimeResourceManifest, RuntimeResourceState,
    ValidationMode, PRODUCTION_BASE_URL,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TARGET: &str = "aarch64-apple-darwin";
const FILE_PATH: &str = "aarch64-apple-darwin/binaries/ffmpeg";
const FILE_BYTES: &[u8] = b"verified runtime resource";
const VOICE_RUNTIME_PATH: &str = "aarch64-apple-darwin/voice-worker/worker";
const VOICE_RUNTIME_BYTES: &[u8] = b"r";
const VOICE_MODEL_PATH: &str = "common/voice-models/model.bin";
const VOICE_MODEL_BYTES: &[u8] = b"m";
static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn manifest_json(base_url: &str, relative_path: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version": 1,
        "release": "v0.1.0",
        "target": "aarch64-apple-darwin",
        "base_url": base_url,
        "files": [{
            "component": "media",
            "relative_path": relative_path,
            "size_bytes": 1,
            "sha256": "00".repeat(32),
            "executable": true
        }]
    }))
    .expect("fixture manifest should serialize")
}

#[test]
fn rejects_manifest_with_untrusted_origin_or_parent_segment() {
    let bad_origin = manifest_json(
        "http://127.0.0.1:7088/autolive-resources/v0.1.0",
        "aarch64-apple-darwin/binaries/ffmpeg",
    );
    assert!(
        RuntimeResourceManifest::parse_and_validate(&bad_origin, ValidationMode::Production)
            .is_err()
    );

    let bad_path = manifest_json(PRODUCTION_BASE_URL, "aarch64-apple-darwin/../ffmpeg");
    let error = RuntimeResourceManifest::parse_and_validate(&bad_path, ValidationMode::Production)
        .expect_err("parent segment must be rejected");
    assert!(error.to_string().contains("unsafe manifest path"));

    let encoded_parent = manifest_json(
        PRODUCTION_BASE_URL,
        "aarch64-apple-darwin/binaries/%2e%2e/ffmpeg",
    );
    assert!(RuntimeResourceManifest::parse_and_validate(
        &encoded_parent,
        ValidationMode::Production
    )
    .is_err());
}

#[test]
fn production_base_url_contract_includes_the_trailing_slash() {
    assert_eq!(
        PRODUCTION_BASE_URL,
        "http://101.96.208.132:7088/autolive-resources/v0.1.0/"
    );
}

#[test]
fn rejects_manifest_missing_a_required_component() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let media_only = manifest_json(&fixture.base_url(), FILE_PATH);

    assert!(RuntimeResourceManifest::parse_and_validate(
        &media_only,
        ValidationMode::Test {
            base_url: fixture.base_url(),
        },
    )
    .is_err());
}

#[test]
fn voice_component_expands_to_all_required_components() {
    assert_eq!(
        RuntimeResourceComponent::Voice.required_components(),
        [
            ManifestComponent::Media,
            ManifestComponent::VoiceRuntime,
            ManifestComponent::VoiceModels,
        ],
    );
}

#[derive(Clone)]
enum FixtureMode {
    Range,
    IgnoreRange,
    WrongContentRangeThenRange,
    MissingContentRangeThenRange,
    BadRangeContentLengthThenRange,
    StatusesThenRange(Vec<u16>),
    CancelAfterStatus(u16, Arc<AtomicBool>),
    BadContentLength,
    DisconnectThenRange(usize),
}

struct RangeFixture {
    address: SocketAddr,
    requested_ranges: Arc<Mutex<Vec<Option<String>>>>,
    requests: Arc<Mutex<usize>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RangeFixture {
    fn new(bytes: &[u8]) -> Self {
        Self::with_mode(bytes, FixtureMode::Range)
    }

    fn with_mode(bytes: &[u8], mode: FixtureMode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fixture should bind");
        listener
            .set_nonblocking(true)
            .expect("fixture should become nonblocking");
        let address = listener.local_addr().expect("fixture address");
        let body = bytes.to_vec();
        let requested_ranges = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::new(Mutex::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_ranges = Arc::clone(&requested_ranges);
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if thread_stop.load(Ordering::SeqCst) {
                            break;
                        }
                        stream
                            .set_nonblocking(false)
                            .expect("fixture connection should become blocking");
                        serve_request(stream, &body, &mode, &thread_ranges, &thread_requests)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            address,
            requested_ranges,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}/", self.address)
    }

    fn requested_ranges(&self) -> Vec<Option<String>> {
        self.requested_ranges
            .lock()
            .expect("range fixture lock")
            .clone()
    }

    fn request_count(&self) -> usize {
        *self.requests.lock().expect("request fixture lock")
    }
}

impl Drop for RangeFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ignored = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("fixture thread should stop");
        }
    }
}

fn serve_request(
    mut stream: TcpStream,
    body: &[u8],
    mode: &FixtureMode,
    ranges: &Mutex<Vec<Option<String>>>,
    requests: &Mutex<usize>,
) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("fixture read timeout");
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream
            .read(&mut buffer)
            .expect("fixture should read request");
        if read == 0 {
            return;
        }
        request.extend_from_slice(&buffer[..read]);
    }
    let request_number = {
        let mut requests = requests.lock().expect("request fixture lock");
        *requests += 1;
        *requests
    };
    let request = String::from_utf8(request).expect("HTTP request should be UTF-8");
    let range = request.lines().find_map(|line| {
        line.strip_prefix("Range: ")
            .or_else(|| line.strip_prefix("range: "))
            .map(str::trim)
    });
    ranges
        .lock()
        .expect("range fixture lock")
        .push(range.map(str::to_owned));

    if let FixtureMode::StatusesThenRange(statuses) = mode {
        if let Some(status) = statuses.get(request_number - 1) {
            let header = format!(
                "HTTP/1.1 {status} fixture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream
                .write_all(header.as_bytes())
                .expect("fixture should write status response");
            return;
        }
    }
    if let FixtureMode::CancelAfterStatus(status, cancel) = mode {
        let header =
            format!("HTTP/1.1 {status} fixture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        stream
            .write_all(header.as_bytes())
            .expect("fixture should write cancellation response");
        cancel.store(true, Ordering::SeqCst);
        return;
    }

    let start = range
        .and_then(|value| value.strip_prefix("bytes="))
        .and_then(|value| value.strip_suffix('-'))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let (status, response_body, content_range) = match (mode, range) {
        (FixtureMode::Range | FixtureMode::StatusesThenRange(_), Some(_))
        | (FixtureMode::DisconnectThenRange(_), Some(_)) => (
            "206 Partial Content",
            &body[start..],
            Some(format!(
                "Content-Range: bytes {start}-{}/{}\r\n",
                body.len() - 1,
                body.len()
            )),
        ),
        (FixtureMode::WrongContentRangeThenRange, Some(_)) if request_number == 1 => (
            "206 Partial Content",
            &body[start..],
            Some(format!(
                "Content-Range: bytes {}-{}/{}\r\n",
                start + 1,
                body.len() - 1,
                body.len()
            )),
        ),
        (FixtureMode::MissingContentRangeThenRange, Some(_)) if request_number == 1 => {
            ("206 Partial Content", &body[start..], None)
        }
        (FixtureMode::BadRangeContentLengthThenRange, Some(_)) if request_number == 1 => (
            "206 Partial Content",
            &body[start..],
            Some(format!(
                "Content-Range: bytes {start}-{}/{}\r\n",
                body.len() - 1,
                body.len()
            )),
        ),
        _ => ("200 OK", body, None),
    };
    let content_length = if matches!(mode, FixtureMode::BadContentLength)
        || matches!(mode, FixtureMode::BadRangeContentLengthThenRange) && request_number == 1
    {
        response_body.len() as u64 + 1
    } else {
        response_body.len() as u64
    };
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
        content_length,
        content_range.unwrap_or_default()
    );
    stream
        .write_all(header.as_bytes())
        .expect("fixture should write response header");
    let bytes_to_write = match mode {
        FixtureMode::DisconnectThenRange(limit) if request_number == 1 => {
            (*limit).min(response_body.len())
        }
        _ => response_body.len(),
    };
    stream
        .write_all(&response_body[..bytes_to_write])
        .expect("fixture should write response body");
}

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "autolive-runtime-resources-{label}-{}-{nonce}",
            std::process::id(),
            nonce = nonce
                .saturating_add(TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed) as u128)
        ));
        fs::create_dir_all(&path).expect("test directory should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.0);
    }
}

struct InstallerHarness {
    root: TestDir,
    installer: RuntimeResourceInstaller,
    layout: RuntimeResourceLayout,
    partial_hash: String,
}

impl InstallerHarness {
    fn new(fixture: &RangeFixture, bytes: &[u8]) -> Self {
        Self::with_manifest_values(fixture, bytes.len() as u64, &sha256(bytes))
    }

    fn with_declared_hash(fixture: &RangeFixture, hash: String) -> Self {
        Self::with_manifest_values(fixture, FILE_BYTES.len() as u64, &hash)
    }

    fn with_manifest_values(fixture: &RangeFixture, size_bytes: u64, hash: &str) -> Self {
        let root = TestDir::new("installer");
        let layout = RuntimeResourceLayout::for_target(root.path(), TARGET).expect("valid layout");
        let manifest = test_manifest(&fixture.base_url(), size_bytes, hash);
        let installer =
            RuntimeResourceInstaller::for_test(&manifest, root.path(), &fixture.base_url())
                .expect("valid installer fixture");
        Self {
            root,
            installer,
            layout,
            partial_hash: hash.to_owned(),
        }
    }

    fn with_partial(fixture: &RangeFixture, partial: &[u8]) -> Self {
        let harness = Self::new(fixture, FILE_BYTES);
        let path = harness.partial_path();
        fs::create_dir_all(path.parent().expect("partial parent")).expect("partial directory");
        fs::write(path, partial).expect("partial fixture should be written");
        harness
    }

    fn install(&self) -> Result<(), ResourceInstallError> {
        self.installer
            .install(
                RuntimeResourceComponent::Media,
                &AtomicBool::new(false),
                |_| {},
            )
            .map(|_| ())
    }

    fn final_path(&self) -> PathBuf {
        self.layout.version_root.join(FILE_PATH)
    }

    fn partial_path(&self) -> PathBuf {
        self.layout
            .partial_root
            .join(format!("{}.partial", self.partial_hash))
    }

    fn independent_installer(&self, fixture: &RangeFixture) -> RuntimeResourceInstaller {
        installer_at(fixture, self.root.path())
    }
}

fn installer_at(fixture: &RangeFixture, app_data_dir: &Path) -> RuntimeResourceInstaller {
    let manifest = test_manifest(
        &fixture.base_url(),
        FILE_BYTES.len() as u64,
        &sha256(FILE_BYTES),
    );
    RuntimeResourceInstaller::for_test(&manifest, app_data_dir, &fixture.base_url())
        .expect("installer should be valid")
}

fn test_manifest(base_url: &str, size_bytes: u64, hash: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version": 1,
        "release": "v0.1.0",
        "target": TARGET,
        "base_url": base_url,
        "files": [
            {
                "component": "media",
                "relative_path": FILE_PATH,
                "size_bytes": size_bytes,
                "sha256": hash,
                "executable": true
            },
            {
                "component": "voice-runtime",
                "relative_path": VOICE_RUNTIME_PATH,
                "size_bytes": 1,
                "sha256": sha256(VOICE_RUNTIME_BYTES),
                "executable": true
            },
            {
                "component": "voice-models",
                "relative_path": VOICE_MODEL_PATH,
                "size_bytes": 1,
                "sha256": sha256(VOICE_MODEL_BYTES),
                "executable": false
            }
        ]
    }))
    .expect("test manifest should serialize")
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn skips_an_existing_verified_file() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let harness = InstallerHarness::new(&fixture, FILE_BYTES);
    fs::create_dir_all(harness.final_path().parent().expect("final parent"))
        .expect("final directory");
    fs::write(harness.final_path(), FILE_BYTES).expect("existing final file");

    harness.install().expect("verified file should be accepted");

    assert_eq!(fixture.request_count(), 0);
    assert!(harness.layout.installed_record.is_file());
    assert_eq!(
        harness
            .installer
            .inspect(RuntimeResourceComponent::Media)
            .expect("inspect succeeds")
            .state,
        RuntimeResourceState::Ready
    );
}

#[test]
fn resumes_partial_file_with_matching_range_and_installs_atomically() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let harness = InstallerHarness::with_partial(&fixture, &FILE_BYTES[..4]);

    harness.install().expect("install succeeds");

    assert_eq!(
        fixture.requested_ranges(),
        vec![Some("bytes=4-".to_owned())]
    );
    assert_eq!(fs::read(harness.final_path()).unwrap(), FILE_BYTES);
    assert!(!harness.partial_path().exists());
    assert!(harness.layout.installed_record.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(harness.final_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }
}

#[test]
fn restarts_partial_when_server_responds_with_200() {
    let fixture = RangeFixture::with_mode(FILE_BYTES, FixtureMode::IgnoreRange);
    let harness = InstallerHarness::with_partial(&fixture, b"bad!");

    harness.install().expect("200 fallback should restart file");

    assert_eq!(
        fixture.requested_ranges(),
        vec![Some("bytes=4-".to_owned())]
    );
    assert_eq!(fs::read(harness.final_path()).unwrap(), FILE_BYTES);
}

#[test]
fn wrong_content_range_discards_partial_and_restarts_without_range() {
    let fixture = RangeFixture::with_mode(FILE_BYTES, FixtureMode::WrongContentRangeThenRange);
    let harness = InstallerHarness::with_partial(&fixture, &FILE_BYTES[..4]);

    harness
        .install()
        .expect("invalid range metadata should restart once");

    assert_eq!(
        fixture.requested_ranges(),
        vec![Some("bytes=4-".to_owned()), None]
    );
    assert_eq!(fs::read(harness.final_path()).unwrap(), FILE_BYTES);
    assert!(!harness.partial_path().exists());
}

#[test]
fn missing_content_range_discards_partial_and_restarts_without_range() {
    let fixture = RangeFixture::with_mode(FILE_BYTES, FixtureMode::MissingContentRangeThenRange);
    let harness = InstallerHarness::with_partial(&fixture, &FILE_BYTES[..4]);

    harness
        .install()
        .expect("missing range metadata should restart once");

    assert_eq!(
        fixture.requested_ranges(),
        vec![Some("bytes=4-".to_owned()), None]
    );
}

#[test]
fn wrong_range_content_length_discards_partial_and_restarts_without_range() {
    let fixture = RangeFixture::with_mode(FILE_BYTES, FixtureMode::BadRangeContentLengthThenRange);
    let harness = InstallerHarness::with_partial(&fixture, &FILE_BYTES[..4]);

    harness
        .install()
        .expect("wrong remaining length should restart once");

    assert_eq!(
        fixture.requested_ranges(),
        vec![Some("bytes=4-".to_owned()), None]
    );
}

#[test]
fn retries_two_transient_statuses_then_succeeds_on_the_third_request() {
    let fixture =
        RangeFixture::with_mode(FILE_BYTES, FixtureMode::StatusesThenRange(vec![408, 429]));
    let harness = InstallerHarness::new(&fixture, FILE_BYTES);

    harness
        .install()
        .expect("the third request should complete the installation");

    assert_eq!(fixture.request_count(), 3);
    assert_eq!(fixture.requested_ranges(), vec![None, None, None]);
    assert_eq!(fs::read(harness.final_path()).unwrap(), FILE_BYTES);
}

#[test]
fn retries_an_interrupted_read_from_the_existing_partial() {
    let fixture = RangeFixture::with_mode(FILE_BYTES, FixtureMode::DisconnectThenRange(4));
    let harness = InstallerHarness::new(&fixture, FILE_BYTES);

    harness
        .install()
        .expect("read interruption should resume from bytes already written");

    assert_eq!(
        fixture.requested_ranges(),
        vec![None, Some("bytes=4-".to_owned())]
    );
    assert_eq!(fs::read(harness.final_path()).unwrap(), FILE_BYTES);
}

#[test]
fn stops_after_three_transient_requests() {
    let fixture = RangeFixture::with_mode(
        FILE_BYTES,
        FixtureMode::StatusesThenRange(vec![500, 500, 500]),
    );
    let harness = InstallerHarness::new(&fixture, FILE_BYTES);

    assert!(matches!(
        harness.install(),
        Err(ResourceInstallError::UnexpectedHttpStatus { status: 500, .. })
    ));
    assert_eq!(fixture.request_count(), 3);
}

#[test]
fn cancellation_interrupts_retry_backoff_before_another_request() {
    let cancel = Arc::new(AtomicBool::new(false));
    let fixture = RangeFixture::with_mode(
        FILE_BYTES,
        FixtureMode::CancelAfterStatus(500, Arc::clone(&cancel)),
    );
    let harness = InstallerHarness::new(&fixture, FILE_BYTES);

    let result =
        harness
            .installer
            .install(RuntimeResourceComponent::Media, cancel.as_ref(), |_| {});

    assert_eq!(result, Err(ResourceInstallError::Cancelled));
    assert_eq!(fixture.request_count(), 1);
}

#[test]
fn does_not_retry_a_non_transient_client_error() {
    let fixture = RangeFixture::with_mode(FILE_BYTES, FixtureMode::StatusesThenRange(vec![404]));
    let harness = InstallerHarness::new(&fixture, FILE_BYTES);

    assert!(matches!(
        harness.install(),
        Err(ResourceInstallError::UnexpectedHttpStatus { status: 404, .. })
    ));
    assert_eq!(fixture.request_count(), 1);
}

#[test]
fn rejects_a_full_response_with_the_wrong_content_length_without_looping() {
    let fixture = RangeFixture::with_mode(FILE_BYTES, FixtureMode::BadContentLength);
    let harness = InstallerHarness::new(&fixture, FILE_BYTES);

    assert!(matches!(
        harness.install(),
        Err(ResourceInstallError::SizeMismatch { .. })
    ));
    assert_eq!(fixture.request_count(), 1);
    assert!(!harness.partial_path().exists());
}

#[test]
fn partial_filename_is_the_manifest_hash_without_relative_path_nesting() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let harness = InstallerHarness::with_partial(&fixture, &FILE_BYTES[..4]);

    assert_eq!(
        harness.partial_path(),
        harness
            .layout
            .partial_root
            .join(format!("{}.partial", sha256(FILE_BYTES)))
    );
    assert!(!harness.layout.partial_root.join(FILE_PATH).exists());
}

#[test]
fn cancellation_keeps_partial_for_a_later_range_request() {
    let bytes = vec![0x5a; 256 * 1024];
    let fixture = RangeFixture::new(&bytes);
    let harness = InstallerHarness::new(&fixture, &bytes);
    let cancel = AtomicBool::new(false);

    let result = harness
        .installer
        .install(RuntimeResourceComponent::Media, &cancel, |status| {
            if status.state == RuntimeResourceState::Downloading && status.downloaded_bytes > 0 {
                cancel.store(true, Ordering::SeqCst);
            }
        });

    assert_eq!(result, Err(ResourceInstallError::Cancelled));
    assert!(!harness.final_path().exists());
    let partial_size = fs::metadata(harness.partial_path())
        .expect("cancelled partial should remain")
        .len();
    assert!(partial_size > 0 && partial_size < bytes.len() as u64);
}

#[test]
fn cancellation_during_hash_verification_keeps_complete_partial() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let harness = InstallerHarness::with_partial(&fixture, FILE_BYTES);
    let cancel = AtomicBool::new(false);

    let result = harness
        .installer
        .install(RuntimeResourceComponent::Media, &cancel, |status| {
            if status.state == RuntimeResourceState::Verifying {
                cancel.store(true, Ordering::SeqCst);
            }
        });

    assert_eq!(result, Err(ResourceInstallError::Cancelled));
    assert!(!harness.final_path().exists());
    assert_eq!(fs::read(harness.partial_path()).unwrap(), FILE_BYTES);
}

#[test]
fn deletes_partial_when_hash_does_not_match() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let harness = InstallerHarness::with_declared_hash(&fixture, "00".repeat(32));

    assert!(matches!(
        harness.install(),
        Err(ResourceInstallError::HashMismatch { .. })
    ));
    assert!(!harness.final_path().exists());
    assert!(!harness.partial_path().exists());
}

#[test]
fn reports_io_error_when_an_untrusted_partial_cannot_be_deleted() {
    let bytes = vec![0x5a; 12_345];
    let fixture = RangeFixture::new(&bytes);
    let harness = InstallerHarness::new(&fixture, &bytes);
    let partial_path = harness.partial_path();
    fs::create_dir_all(partial_path.parent().expect("partial parent")).expect("partial directory");
    fs::write(&partial_path, &bytes).expect("complete partial fixture");

    let result = harness.installer.install(
        RuntimeResourceComponent::Media,
        &AtomicBool::new(false),
        |status| {
            if status.state == RuntimeResourceState::Verifying {
                fs::remove_file(&partial_path).expect("replace partial file");
                fs::create_dir(&partial_path).expect("replacement directory");
                fs::write(partial_path.join("not-empty"), b"keep")
                    .expect("non-empty directory fixture");
            }
        },
    );

    assert!(matches!(
        result,
        Err(ResourceInstallError::Io {
            operation: "remove untrusted partial",
            ref message,
            ..
        }) if !message.is_empty()
    ));
    assert!(partial_path.join("not-empty").is_file());
    assert!(!harness.final_path().exists());
}

#[test]
fn rejects_install_when_declared_files_exceed_available_disk_space() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let harness = InstallerHarness::with_manifest_values(&fixture, u64::MAX, &sha256(FILE_BYTES));

    assert!(matches!(
        harness.install(),
        Err(ResourceInstallError::DiskRequirementOverflow)
    ));
    assert_eq!(fixture.request_count(), 0);
}

#[test]
fn disk_requirement_uses_the_exact_reserve_and_checks_overflow() {
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;
    let fixed_reserve = 512 * MIB;
    let threshold = 5 * GIB;

    assert_eq!(required_disk_space(0).unwrap(), fixed_reserve);
    assert_eq!(required_disk_space(1).unwrap(), fixed_reserve + 1);
    assert_eq!(
        required_disk_space(threshold).unwrap(),
        threshold + fixed_reserve
    );
    assert_eq!(
        required_disk_space(threshold + 1).unwrap(),
        threshold + 1 + fixed_reserve + 1
    );
    assert_eq!(
        required_disk_space(u64::MAX),
        Err(ResourceInstallError::DiskRequirementOverflow)
    );
}

#[test]
fn invalidates_a_stale_installed_record_before_a_repair_can_fail() {
    let fixture = RangeFixture::with_mode(FILE_BYTES, FixtureMode::StatusesThenRange(vec![404]));
    let harness = InstallerHarness::new(&fixture, FILE_BYTES);
    fs::create_dir_all(&harness.layout.version_root).expect("version directory");
    fs::write(&harness.layout.installed_record, b"stale").expect("stale installed record");
    fs::create_dir_all(harness.final_path().parent().expect("final parent"))
        .expect("final directory");
    fs::write(harness.final_path(), b"invalid").expect("invalid installed file");

    let result = harness.installer.install(
        RuntimeResourceComponent::Media,
        &AtomicBool::new(false),
        |status| {
            if status.state == RuntimeResourceState::Checking {
                assert!(
                    !harness.layout.installed_record.exists(),
                    "the stale marker must be gone before repair work is observable"
                );
            }
        },
    );
    assert!(result.is_err());

    assert!(!harness.layout.installed_record.exists());
    assert_eq!(fixture.request_count(), 1);
}

#[test]
fn installed_record_keeps_every_component_that_is_actually_verified() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let harness = InstallerHarness::new(&fixture, FILE_BYTES);
    for (relative_path, bytes) in [
        (FILE_PATH, FILE_BYTES),
        (VOICE_RUNTIME_PATH, VOICE_RUNTIME_BYTES),
        (VOICE_MODEL_PATH, VOICE_MODEL_BYTES),
    ] {
        let destination = harness.layout.version_root.join(relative_path);
        fs::create_dir_all(destination.parent().expect("component parent"))
            .expect("component directory");
        fs::write(destination, bytes).expect("verified component fixture");
    }

    harness.install().expect("all existing files are valid");

    let record: serde_json::Value = serde_json::from_slice(
        &fs::read(&harness.layout.installed_record).expect("installed record"),
    )
    .expect("installed record JSON");
    assert_eq!(
        record["installed_components"],
        json!(["media", "voice-runtime", "voice-models"])
    );
}

#[test]
fn imports_matching_directory_with_the_same_validation_and_layout() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let harness = InstallerHarness::new(&fixture, FILE_BYTES);
    let source = TestDir::new("import-source");
    let source_file = source.path().join(FILE_PATH);
    fs::create_dir_all(source_file.parent().expect("source parent")).expect("source directory");
    fs::write(&source_file, FILE_BYTES).expect("source fixture");

    let status = harness
        .installer
        .import_directory(
            RuntimeResourceComponent::Media,
            source.path(),
            &AtomicBool::new(false),
            |_| {},
        )
        .expect("local import succeeds");

    assert_eq!(status.state, RuntimeResourceState::Ready);
    assert_eq!(fs::read(harness.final_path()).unwrap(), FILE_BYTES);
    assert_eq!(fixture.request_count(), 0);
}

#[test]
#[cfg(unix)]
fn import_rejects_a_symlink_that_points_outside_the_source_root() {
    use std::os::unix::fs::symlink;

    let fixture = RangeFixture::new(FILE_BYTES);
    let harness = InstallerHarness::new(&fixture, FILE_BYTES);
    let source = TestDir::new("import-symlink-source");
    let outside = TestDir::new("import-symlink-outside");
    let outside_target_root = outside.path().join(TARGET);
    let outside_file = outside_target_root.join("binaries/ffmpeg");
    fs::create_dir_all(outside_file.parent().expect("outside parent")).expect("outside directory");
    fs::write(&outside_file, FILE_BYTES).expect("matching outside fixture");
    symlink(&outside_target_root, source.path().join(TARGET)).expect("outside symlink fixture");

    let result = harness.installer.import_directory(
        RuntimeResourceComponent::Media,
        source.path(),
        &AtomicBool::new(false),
        |_| {},
    );

    assert!(result.is_err());
    assert!(!harness.final_path().exists());
}

#[test]
fn io_error_includes_the_original_operating_system_message() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let harness = InstallerHarness::new(&fixture, FILE_BYTES);
    let missing = harness.root.path().join("missing-import-directory");
    let original_message = fs::canonicalize(&missing)
        .expect_err("fixture path must not exist")
        .to_string();

    let error = harness
        .installer
        .import_directory(
            RuntimeResourceComponent::Media,
            &missing,
            &AtomicBool::new(false),
            |_| {},
        )
        .expect_err("missing import must fail");

    assert!(error.to_string().contains(&original_message));
}

#[test]
fn clear_removes_only_the_fixed_current_release_directory() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let harness = InstallerHarness::new(&fixture, FILE_BYTES);
    let sibling = harness
        .layout
        .version_root
        .parent()
        .expect("runtime root")
        .join("v0.2.0")
        .join("keep.txt");
    fs::create_dir_all(sibling.parent().expect("sibling parent")).expect("sibling directory");
    fs::write(&sibling, b"keep").expect("sibling fixture");
    fs::create_dir_all(&harness.layout.version_root).expect("current release directory");

    let status = harness
        .installer
        .clear_current_release()
        .expect("clear should succeed");

    assert_eq!(status.state, RuntimeResourceState::NotInstalled);
    assert!(!harness.layout.version_root.exists());
    assert_eq!(fs::read(sibling).unwrap(), b"keep");
}

#[test]
fn independent_installers_for_the_same_root_are_mutually_exclusive() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let harness = InstallerHarness::new(&fixture, FILE_BYTES);
    let installer = harness.independent_installer(&fixture);
    let competing_installer = harness.independent_installer(&fixture);
    assert_installers_conflict(installer, competing_installer);
}

#[cfg(unix)]
#[test]
fn operation_gate_canonicalizes_a_missing_leaf_through_a_symlink_parent() {
    use std::os::unix::fs::symlink;

    let fixture = RangeFixture::new(FILE_BYTES);
    let root = TestDir::new("operation-symlink-alias");
    let real_parent = root.path().join("real-parent");
    let symlink_parent = root.path().join("symlink-parent");
    fs::create_dir(&real_parent).expect("real parent");
    symlink(&real_parent, &symlink_parent).expect("parent symlink");
    let real_app_data = real_parent.join("new");
    let alias_app_data = symlink_parent.join("new");
    assert!(!real_app_data.exists());
    assert!(!alias_app_data.exists());

    let installer = installer_at(&fixture, &real_app_data);
    let competing_installer = installer_at(&fixture, &alias_app_data);

    assert_installers_conflict(installer, competing_installer);
}

#[test]
fn operation_gate_canonicalizes_a_missing_leaf_with_parent_segments() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let root = TestDir::new("operation-parent-segment-alias");
    let real_parent = root.path().join("real-parent");
    let existing_segment = real_parent.join("existing");
    fs::create_dir_all(&existing_segment).expect("existing parent segment");
    let direct_app_data = real_parent.join("new");
    let alias_app_data = existing_segment.join("..").join("new");
    assert!(!direct_app_data.exists());
    assert!(!alias_app_data.exists());

    let installer = installer_at(&fixture, &direct_app_data);
    let competing_installer = installer_at(&fixture, &alias_app_data);

    assert_installers_conflict(installer, competing_installer);
}

fn assert_installers_conflict(
    installer: RuntimeResourceInstaller,
    competing_installer: RuntimeResourceInstaller,
) {
    let (checking_sender, checking_receiver) = sync_channel(0);
    let (release_sender, release_receiver) = sync_channel(0);
    let worker = thread::spawn(move || {
        installer.install(
            RuntimeResourceComponent::Media,
            &AtomicBool::new(false),
            |status| {
                if status.state == RuntimeResourceState::Checking {
                    checking_sender.send(()).expect("checking signal");
                    release_receiver.recv().expect("release signal");
                }
            },
        )
    });
    checking_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("installation should reach checking");

    let competing_result = competing_installer.install(
        RuntimeResourceComponent::Media,
        &AtomicBool::new(false),
        |_| {},
    );
    release_sender.send(()).expect("release installation");
    let first_result = worker.join().expect("installation thread should join");

    assert_eq!(competing_result, Err(ResourceInstallError::Busy));
    assert_eq!(
        first_result
            .expect("first installation should finish")
            .state,
        RuntimeResourceState::Ready
    );
}

#[test]
fn installers_for_different_roots_do_not_block_each_other() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let blocked = InstallerHarness::new(&fixture, FILE_BYTES);
    let other = InstallerHarness::new(&fixture, FILE_BYTES);
    let installer = blocked.independent_installer(&fixture);
    let (checking_sender, checking_receiver) = sync_channel(0);
    let (release_sender, release_receiver) = sync_channel(0);
    let worker = thread::spawn(move || {
        installer.install(
            RuntimeResourceComponent::Media,
            &AtomicBool::new(false),
            |status| {
                if status.state == RuntimeResourceState::Checking {
                    checking_sender.send(()).expect("checking signal");
                    release_receiver.recv().expect("release signal");
                }
            },
        )
    });
    checking_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("first installation should hold its root gate");

    other
        .install()
        .expect("a distinct root must not share the operation gate");

    release_sender.send(()).expect("release first installation");
    worker
        .join()
        .expect("installation thread should join")
        .expect("first installation should finish");
}
