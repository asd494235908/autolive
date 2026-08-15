use autolive_desktop_core::runtime_resources::{
    RuntimeResourceInstaller, RuntimeResourceLayout, PRODUCTION_BASE_URL,
};
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
fn runtime_manifest_is_loaded_from_the_tauri_resource_directory_at_runtime() {
    let source = fs::read_to_string("src/commands.rs").expect("commands source should exist");

    assert!(source.contains("runtime-resources.json"));
    assert!(source.contains("resource_dir()"));
    assert!(source.contains("std::fs::read"));
    assert!(!source.contains("include_bytes!"));
}

#[test]
fn app_state_owns_the_single_runtime_resource_task_and_exit_has_a_three_second_budget() {
    let commands = fs::read_to_string("src/commands.rs").expect("commands source should exist");
    let main = fs::read_to_string("src/main.rs").expect("main source should exist");

    assert!(commands.contains("RuntimeResourceTask"));
    assert!(commands.contains("runtime_resource_task"));
    assert!(main.contains("RunEvent::ExitRequested"));
    assert!(main.contains("Duration::from_secs(3)"));
}

#[test]
fn packaged_paths_are_derived_from_the_versioned_app_data_layout() {
    let target = current_target();
    if target == "unsupported" {
        return;
    }
    let layout = RuntimeResourceLayout::for_target(Path::new("/app-data"), target)
        .expect("supported target should have a layout");
    let commands = fs::read_to_string("src/commands.rs").expect("commands source should exist");

    assert_eq!(
        layout.target_root.join("binaries/ffprobe"),
        Path::new("/app-data")
            .join("runtime-resources/v0.1.0")
            .join(target)
            .join("binaries/ffprobe")
    );
    assert!(commands.contains("app_data_dir()"));
    assert!(commands.contains("layout.target_root"));
    assert!(commands.contains("layout.model_root"));
}

#[test]
fn production_installer_accepts_manifest_bytes_read_at_runtime() {
    let directory = TestDir::new();
    let manifest = production_manifest();

    RuntimeResourceInstaller::from_embedded(manifest.as_bytes(), directory.path())
        .expect("runtime-owned manifest bytes should be accepted");
}

#[test]
fn missing_release_resources_prompt_for_download_instead_of_reinstallation() {
    let source = fs::read_to_string("src/commands.rs").expect("commands source should exist");

    assert!(source.contains("需要下载运行资源"));
    assert!(!source.contains("请重新安装完整版本"));
}

#[test]
fn duplicate_start_reuses_the_running_task_before_manifest_io() {
    let source = fs::read_to_string("src/commands.rs").expect("commands source should exist");
    for command in [
        "install_runtime_resources",
        "import_runtime_resource_directory",
        "clear_runtime_resources",
    ] {
        let start = source
            .find(&format!("pub async fn {command}"))
            .expect("command should exist");
        let body = &source[start..];
        let running = body
            .find("running_status()")
            .unwrap_or_else(|| panic!("{command} must check the current task first"));
        let manifest = body
            .find("runtime_resource_installer(&app)")
            .expect("command should construct an installer when idle");
        assert!(
            running < manifest,
            "{command} must reuse before reading the manifest"
        );
    }
}
