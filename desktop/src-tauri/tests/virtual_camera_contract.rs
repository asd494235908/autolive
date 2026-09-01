use std::fs;
use std::path::Path;

fn read(relative: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative))
        .unwrap_or_else(|error| panic!("无法读取 {relative}: {error}"))
}

#[test]
fn virtual_camera_commands_are_registered_and_main_window_scoped() {
    let main = read("src/main.rs");
    let commands = read("src/commands.rs");
    let build = read("build.rs");
    let permissions = read("permissions/command-sets.toml");

    for command in [
        "get_virtual_camera_status",
        "install_or_repair_virtual_camera",
        "start_virtual_camera_output",
        "stop_virtual_camera_output",
    ] {
        assert!(main.contains(command), "main 未注册 {command}");
        assert!(
            commands.contains(&format!("pub fn {command}")),
            "commands 缺少 {command}"
        );
        assert!(build.contains(command), "build.rs 未生成 {command}");
        let permission = command.replace('_', "-");
        assert!(
            permissions.contains(&format!("allow-{permission}")),
            "permissions 缺少 {command}"
        );
    }

    for command in [
        "get_virtual_camera_status",
        "install_or_repair_virtual_camera",
        "start_virtual_camera_output",
        "stop_virtual_camera_output",
    ] {
        let marker = format!("pub fn {command}");
        let start = commands.find(&marker).expect("command marker");
        let body = commands[start..]
            .lines()
            .take(12)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            body.contains("ensure_main_window"),
            "{command} 必须限制在 main 窗口"
        );
    }
}

#[test]
fn virtual_camera_start_is_fail_closed_without_capture_or_release_artifacts() {
    let commands = read("src/commands.rs");
    let native = read("crates/autolive-virtual-camera-native/src/lib.rs");
    let capture = read("crates/autolive-virtual-camera-native/src/capture.rs");
    let protocol = read("crates/autolive-virtual-camera-native/src/sidecar_protocol.rs");
    let runtime = read("src/virtual_camera_output.rs");

    assert!(commands.contains("virtual_camera_prerequisites_failed"));
    assert!(commands.contains("virtual_camera_gpu_capture_unavailable"));
    assert!(commands.contains("sidecar_path: Some(sidecar_path.clone())"));
    assert!(commands.contains("let sidecar_path = virtual_camera_sidecar_path(app)?"));
    assert!(commands.contains("start_virtual_camera_task_for_window"));
    let start = commands
        .find("pub fn start_virtual_camera_output")
        .expect("start command marker");
    let start_body = &commands[start
        ..commands
            .find("const VIRTUAL_CAMERA_RESOURCE_DIRECTORY")
            .expect("virtual camera constants marker")];
    assert!(
        start_body.contains("if !refresh_virtual_camera_installation_state(&app, &state)?"),
        "启动命令必须在每次启动前重新核验发布资源门禁"
    );
    assert!(native.contains("release_ready: false"));
    assert!(native.contains("GPU 转换/三槽 staging 仅完成本机短测"));
    assert!(capture.contains("有界 latest-wins 槽"));
    assert!(capture.contains("autolive-wgc-capture"));
    assert!(protocol.contains("FRAME_MAGIC"));
    assert!(protocol.contains("validate_pipe_name"));
    assert!(runtime.contains("ManagedSidecarJob"));
    assert!(runtime.contains("CREATE_NO_WINDOW"));
    assert!(runtime.contains("Named Pipe"));
    assert!(runtime.contains("StatusReaderGuard"));
    assert!(runtime.contains("GPAKVC_CLIENTS"));
    assert!(runtime.contains("arg(\"--session-token-stdin\")"));
    assert!(runtime.contains("stdin(Stdio::piped())"));
    assert!(runtime.contains("写入 sidecar 会话令牌失败"));
    assert!(!runtime.contains("arg(token_from_pipe(pipe)"));
    assert!(runtime.contains("stdout(Stdio::piped())"));
    assert!(runtime.contains("MAX_STATUS_LINE_BYTES"));
    assert!(runtime.contains("VirtualCameraState::Starting | VirtualCameraState::Recovering"));
    assert!(runtime.contains("capture_window_id"));
    assert!(commands.contains("should_rebind_virtual_camera"));
    assert!(commands.contains("rebind_virtual_camera_output_if_window_changed"));
    assert!(commands.contains("failed to rebind virtual camera after host recreation"));
    assert!(commands.contains("virtual_camera_lifecycle_lock"));
}

#[test]
fn virtual_camera_stop_command_joins_the_runtime_before_state_reset() {
    let commands = read("src/commands.rs");
    let marker = "pub fn stop_virtual_camera_output";
    let start = commands.find(marker).expect("stop command marker");
    let body = &commands[start..];
    assert!(
        body.contains("state\n        .stop_virtual_camera_output()"),
        "停止命令必须先回收 sidecar/捕获运行时，再重置状态"
    );
}

#[test]
fn virtual_camera_cleanup_converges_after_runtime_stop_failure() {
    let commands = read("src/commands.rs");
    let stop_start = commands
        .find("    fn stop_virtual_camera_output(&self)")
        .expect("private stop helper marker");
    let stop_end = commands[stop_start..]
        .find("    /// 最终效果宿主")
        .map(|offset| stop_start + offset)
        .expect("rebind helper marker");
    let stop_body = &commands[stop_start..stop_end];
    let task_stop = stop_body.find("task.stop()").expect("task stop call");
    let manager_stop = stop_body.find("manager.stop()").expect("manager stop call");
    assert!(
        task_stop < manager_stop,
        "停止时应先回收运行时，再收敛 manager 状态"
    );
    assert!(
        stop_body.contains("first_error.get_or_insert_with"),
        "运行时或 manager 停止失败必须保留首个错误并继续清理"
    );

    let start_marker = "    // 启动前先回收旧 task";
    let start = commands.find(start_marker).expect("startup cleanup marker");
    let start_end = commands[start..]
        .find("    let task = match start_virtual_camera_task_for_window")
        .map(|offset| start + offset)
        .expect("task start marker");
    let start_body = &commands[start..start_end];
    assert!(
        start_body.contains("manager.fail(reason.clone())"),
        "旧运行时回收失败后必须将 Starting 状态收敛到 Failed"
    );

    assert!(
        commands.contains("store_virtual_camera_runtime_task"),
        "新运行时提交必须经过统一的所有权转移入口"
    );
    let store_start = commands
        .find("    fn store_virtual_camera_runtime_task(")
        .expect("runtime store helper marker");
    let store_end = commands[store_start..]
        .find("    /// 最终效果宿主")
        .map(|offset| store_start + offset)
        .expect("rebind helper marker after runtime store");
    let store_body = &commands[store_start..store_end];
    assert!(
        store_body.contains("task.stop()"),
        "运行时锁损坏时必须回收刚启动的 sidecar/捕获 task"
    );
    assert!(
        store_body.contains("回收新会话失败"),
        "运行时锁损坏且回收失败时必须保留可诊断错误"
    );
}

#[test]
fn virtual_camera_lifecycle_operations_are_serialized() {
    let commands = read("src/commands.rs");
    assert!(
        commands.contains("let _lifecycle_guard = state.virtual_camera_lifecycle_lock.lock()"),
        "启动命令必须持有生命周期锁"
    );
    assert!(
        commands
            .contains("let _lifecycle_guard = self\n            .virtual_camera_lifecycle_lock"),
        "停止和重绑必须持有生命周期锁"
    );
}
