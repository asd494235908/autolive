use autolive_desktop_core::runtime_resource_task::RuntimeResourceTask;
use autolive_desktop_core::runtime_resources::{
    RuntimeResourceComponent, RuntimeResourceInstaller, RuntimeResourceLayout,
    RuntimeResourceState, PRODUCTION_BASE_URL,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "autolive-runtime-resource-commands-{}-{}",
            std::process::id(),
            TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
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

fn current_target() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else {
        "unsupported"
    }
}

fn production_manifest() -> String {
    let target = current_target();
    format!(
        r#"{{
            "schema_version": 1,
            "release": "v0.1.0",
            "target": "{target}",
            "base_url": "{PRODUCTION_BASE_URL}",
            "files": [
                {{"component":"media","relative_path":"{target}/binaries/ffmpeg","size_bytes":1,"sha256":"{}","executable":true}},
                {{"component":"voice-runtime","relative_path":"{target}/voice-worker/autolive-voice-clone-worker","size_bytes":1,"sha256":"{}","executable":true}},
                {{"component":"voice-models","relative_path":"common/voice-models/model.bin","size_bytes":1,"sha256":"{}","executable":false}}
            ]
        }}"#,
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
    )
}

fn behavior_manifest(base_url: &str) -> Vec<u8> {
    let target = current_target();
    let hash = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "release": "v0.1.0",
        "target": target,
        "base_url": base_url,
        "files": [
            {"component":"media","relative_path":format!("{target}/binaries/ffmpeg"),"size_bytes":1,"sha256":hash(b"a"),"executable":true},
            {"component":"voice-runtime","relative_path":format!("{target}/voice-worker/autolive-voice-clone-worker"),"size_bytes":1,"sha256":hash(b"b"),"executable":true},
            {"component":"voice-models","relative_path":"common/voice-models/model.bin","size_bytes":1,"sha256":hash(b"c"),"executable":false}
        ]
    }))
    .expect("behavior manifest should serialize")
}

#[test]
fn runtime_resource_commands_are_registered() {
    let source = fs::read_to_string("src/main.rs").expect("main source should exist");
    for command in [
        "get_runtime_resource_status",
        "install_runtime_resources",
        "cancel_runtime_resource_install",
        "import_runtime_resource_directory",
        "clear_runtime_resources",
    ] {
        assert!(source.contains(command), "missing {command}");
    }
}

#[test]
fn packaged_paths_are_derived_from_the_versioned_app_data_layout() {
    let target = current_target();
    if target == "unsupported" {
        return;
    }
    let layout = RuntimeResourceLayout::for_target(Path::new("/app-data"), target)
        .expect("supported target should have a layout");

    assert_eq!(
        layout.target_root.join("binaries/ffprobe"),
        Path::new("/app-data")
            .join("runtime-resources/v0.1.0")
            .join(target)
            .join("binaries/ffprobe")
    );
}

#[test]
fn production_installer_accepts_manifest_bytes_read_at_runtime() {
    let directory = TestDir::new();
    let manifest = production_manifest();

    RuntimeResourceInstaller::from_embedded(manifest.as_bytes(), directory.path())
        .expect("runtime-owned manifest bytes should be accepted");
}

#[test]
fn idle_status_rechecks_the_requested_component_and_detects_damaged_files() {
    let target = current_target();
    if target == "unsupported" {
        return;
    }
    let directory = TestDir::new();
    let base_url = "http://127.0.0.1:9/";
    let installer = RuntimeResourceInstaller::for_test(
        &behavior_manifest(base_url),
        directory.path(),
        base_url,
    )
    .expect("installer should be created");
    let layout = RuntimeResourceLayout::for_target(directory.path(), target).expect("valid layout");
    let media_path = layout.target_root.join("binaries/ffmpeg");
    fs::create_dir_all(media_path.parent().expect("media parent")).expect("media parent");
    fs::write(&media_path, b"a").expect("media fixture");
    let task = RuntimeResourceTask::default();

    task.start_install(RuntimeResourceComponent::Media, installer.clone())
        .expect("media inspection task should start");
    while task
        .running_status()
        .expect("running status should be readable")
        .is_some()
    {
        std::thread::yield_now();
    }

    let voice = task
        .inspect_when_idle(RuntimeResourceComponent::Voice, &installer)
        .expect("voice status should inspect its own files");
    assert_eq!(voice.component, Some(RuntimeResourceComponent::Voice));
    assert_eq!(voice.state, RuntimeResourceState::NotInstalled);

    fs::write(&media_path, b"damaged").expect("media fixture should be damaged");
    let media = task
        .inspect_when_idle(RuntimeResourceComponent::Media, &installer)
        .expect("media status should be re-inspected");
    assert_eq!(media.state, RuntimeResourceState::NotInstalled);
}

#[test]
fn missing_and_invalid_manifests_are_recorded_as_failed_terminal_statuses() {
    let target = current_target();
    if target == "unsupported" {
        return;
    }
    let resource_dir = TestDir::new();
    let app_data_dir = TestDir::new();
    let layout =
        RuntimeResourceLayout::for_target(app_data_dir.path(), target).expect("valid layout");
    let task = RuntimeResourceTask::default();

    for fixture in [None, Some(b"not-json".as_slice())] {
        let manifest_path = resource_dir.path().join("runtime-resources.json");
        let _ignored = fs::remove_file(&manifest_path);
        if let Some(bytes) = fixture {
            fs::write(&manifest_path, bytes).expect("invalid manifest fixture");
        }
        let error = match RuntimeResourceInstaller::from_resource_directory(
            resource_dir.path(),
            app_data_dir.path(),
            target,
        ) {
            Ok(_) => panic!("manifest should fail"),
            Err(error) => error.to_string(),
        };
        let status = task
            .record_failure(RuntimeResourceComponent::Media, error, &layout.version_root)
            .expect("failure status should be recorded");
        assert_eq!(status.state, RuntimeResourceState::Failed);
        assert_eq!(status.component, Some(RuntimeResourceComponent::Media));
        assert!(status
            .error
            .as_deref()
            .is_some_and(|error| error.contains("runtime-resources.json")));
    }

    fs::write(
        resource_dir.path().join("runtime-resources.json"),
        production_manifest(),
    )
    .expect("valid manifest fixture");
    let wrong_target = if target == "aarch64-apple-darwin" {
        "x86_64-apple-darwin"
    } else {
        "aarch64-apple-darwin"
    };
    let target_error = match RuntimeResourceInstaller::from_resource_directory(
        resource_dir.path(),
        app_data_dir.path(),
        wrong_target,
    ) {
        Ok(_) => panic!("target mismatch should fail"),
        Err(error) => error.to_string(),
    };
    let target_status = task
        .record_failure(
            RuntimeResourceComponent::Media,
            target_error,
            &layout.version_root,
        )
        .expect("target failure should be recorded");
    assert_eq!(target_status.state, RuntimeResourceState::Failed);
    assert!(target_status
        .error
        .as_deref()
        .is_some_and(|error| error.contains("target mismatch")));

    let invalid_app_data = resource_dir.path().join("app-data-is-a-file");
    fs::write(&invalid_app_data, b"not-a-directory").expect("invalid app data fixture");
    let app_data_error = match RuntimeResourceInstaller::from_resource_directory(
        resource_dir.path(),
        &invalid_app_data,
        target,
    ) {
        Ok(_) => panic!("invalid app data path should fail"),
        Err(error) => error.to_string(),
    };
    let app_data_status = task
        .record_failure(
            RuntimeResourceComponent::Voice,
            app_data_error,
            Path::new(""),
        )
        .expect("app data failure should be recorded");
    assert_eq!(app_data_status.state, RuntimeResourceState::Failed);
    assert_eq!(
        app_data_status.component,
        Some(RuntimeResourceComponent::Voice)
    );
    assert!(app_data_status
        .error
        .as_deref()
        .is_some_and(|error| error.contains("app-data-is-a-file")));
}

#[test]
fn clear_task_uses_its_shared_cancel_and_reaps_the_worker() {
    let target = current_target();
    if target == "unsupported" {
        return;
    }
    let directory = TestDir::new();
    let base_url = "http://127.0.0.1:9/";
    let installer = RuntimeResourceInstaller::for_test(
        &behavior_manifest(base_url),
        directory.path(),
        base_url,
    )
    .expect("installer should be created");
    let layout = RuntimeResourceLayout::for_target(directory.path(), target).expect("valid layout");
    for index in 0..2_000 {
        let path = layout.version_root.join(format!("entries/{index}.bin"));
        fs::create_dir_all(path.parent().expect("entry parent")).expect("entry parent");
        fs::write(path, [0]).expect("entry fixture");
    }
    let task = RuntimeResourceTask::default();

    task.start_clear(installer).expect("clear should start");
    task.cancel().expect("clear should accept cancellation");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while task
        .running_status()
        .expect("running status should be readable")
        .is_some()
        && std::time::Instant::now() < deadline
    {
        std::thread::yield_now();
    }

    let status = task
        .status(RuntimeResourceComponent::Media)
        .expect("cancelled status should be readable");
    assert_eq!(status.state, RuntimeResourceState::Cancelled);
    assert!(task
        .running_status()
        .expect("finished worker should reap")
        .is_none());
}
