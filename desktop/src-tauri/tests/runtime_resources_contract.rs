use autolive_desktop_core::runtime_resources::{
    ManifestComponent, ResourceInstallError, RuntimeResourceComponent, RuntimeResourceInstaller,
    RuntimeResourceLayout, RuntimeResourceManifest, RuntimeResourceState, ValidationMode,
    PRODUCTION_BASE_URL,
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
    assert!(
        RuntimeResourceManifest::parse_and_validate(&bad_path, ValidationMode::Production).is_err()
    );

    let encoded_parent = manifest_json(
        &format!("{PRODUCTION_BASE_URL}/"),
        "aarch64-apple-darwin/binaries/%2e%2e/ffmpeg",
    );
    assert!(RuntimeResourceManifest::parse_and_validate(
        &encoded_parent,
        ValidationMode::Production
    )
    .is_err());
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

#[derive(Clone, Copy)]
enum FixtureMode {
    Range,
    IgnoreRange,
    WrongContentRange,
}

struct RangeFixture {
    address: SocketAddr,
    requested_ranges: Arc<Mutex<Vec<String>>>,
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
                        serve_request(stream, &body, mode, &thread_ranges, &thread_requests)
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

    fn requested_ranges(&self) -> Vec<String> {
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
    mode: FixtureMode,
    ranges: &Mutex<Vec<String>>,
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
    *requests.lock().expect("request fixture lock") += 1;
    let request = String::from_utf8(request).expect("HTTP request should be UTF-8");
    let range = request.lines().find_map(|line| {
        line.strip_prefix("Range: ")
            .or_else(|| line.strip_prefix("range: "))
            .map(str::trim)
    });
    if let Some(range) = range {
        ranges
            .lock()
            .expect("range fixture lock")
            .push(range.to_owned());
    }

    let start = range
        .and_then(|value| value.strip_prefix("bytes="))
        .and_then(|value| value.strip_suffix('-'))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let (status, response_body, content_range) = match (mode, range) {
        (FixtureMode::Range, Some(_)) => (
            "206 Partial Content",
            &body[start..],
            Some(format!(
                "Content-Range: bytes {start}-{}/{}\r\n",
                body.len() - 1,
                body.len()
            )),
        ),
        (FixtureMode::WrongContentRange, Some(_)) => (
            "206 Partial Content",
            &body[start..],
            Some(format!(
                "Content-Range: bytes {}-{}/{}\r\n",
                start + 1,
                body.len() - 1,
                body.len()
            )),
        ),
        _ => ("200 OK", body, None),
    };
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
        response_body.len(),
        content_range.unwrap_or_default()
    );
    stream
        .write_all(header.as_bytes())
        .expect("fixture should write response header");
    stream
        .write_all(response_body)
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
    _root: TestDir,
    installer: RuntimeResourceInstaller,
    layout: RuntimeResourceLayout,
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
            _root: root,
            installer,
            layout,
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
            .join(format!("{FILE_PATH}.partial"))
    }
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
                "relative_path": "aarch64-apple-darwin/voice-worker/worker",
                "size_bytes": 1,
                "sha256": "11".repeat(32),
                "executable": true
            },
            {
                "component": "voice-models",
                "relative_path": "common/voice-models/model.bin",
                "size_bytes": 1,
                "sha256": "22".repeat(32),
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

    assert_eq!(fixture.requested_ranges(), vec!["bytes=4-"]);
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

    assert_eq!(fixture.requested_ranges(), vec!["bytes=4-"]);
    assert_eq!(fs::read(harness.final_path()).unwrap(), FILE_BYTES);
}

#[test]
fn rejects_wrong_content_range_and_deletes_untrusted_partial() {
    let fixture = RangeFixture::with_mode(FILE_BYTES, FixtureMode::WrongContentRange);
    let harness = InstallerHarness::with_partial(&fixture, &FILE_BYTES[..4]);

    assert!(matches!(
        harness.install(),
        Err(ResourceInstallError::InvalidContentRange { .. })
    ));
    assert!(!harness.final_path().exists());
    assert!(!harness.partial_path().exists());
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
fn rejects_install_when_declared_files_exceed_available_disk_space() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let harness = InstallerHarness::with_manifest_values(&fixture, u64::MAX, &sha256(FILE_BYTES));

    assert!(matches!(
        harness.install(),
        Err(ResourceInstallError::InsufficientDiskSpace { .. })
    ));
    assert_eq!(fixture.request_count(), 0);
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
fn clear_is_rejected_while_an_installation_owns_the_installer() {
    let fixture = RangeFixture::new(FILE_BYTES);
    let harness = InstallerHarness::new(&fixture, FILE_BYTES);
    let installer = harness.installer.clone();
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

    assert_eq!(
        harness.installer.clear_current_release(),
        Err(ResourceInstallError::Busy)
    );

    release_sender.send(()).expect("release installation");
    assert_eq!(
        worker
            .join()
            .expect("installation thread should join")
            .unwrap()
            .state,
        RuntimeResourceState::Ready
    );
}
