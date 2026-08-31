#[test]
fn final_effect_window_owns_a_dedicated_native_video_host() {
    let cargo = std::fs::read_to_string("Cargo.toml").expect("Cargo.toml should be readable");
    let commands =
        std::fs::read_to_string("src/commands.rs").expect("commands.rs should be readable");

    assert!(cargo.contains("autolive-native-video-host"));
    assert!(commands.contains("native_video_host: Arc<Mutex<Option<NativeVideoHost>>>"));
    assert!(commands.contains("pub video_host_ready: bool"));
    assert!(commands.contains("create_final_effect_video_host"));
    assert!(commands.contains("ready_final_effect_video_host_window_id"));
    assert!(commands.contains("if created {\n                let _ignored = window.close();"));

    let host_lookup = commands
        .split_once("fn ready_final_effect_video_host_window_id(")
        .map(|(_, tail)| tail)
        .expect("dedicated host lookup should exist")
        .split("#[tauri::command]")
        .next()
        .expect("host lookup should have a bounded body");
    assert!(!host_lookup.contains("window.hwnd()"));
    assert!(host_lookup.contains(".mpv_wid()"));
    assert!(host_lookup.contains("inspect_mpv_video_window(process_id)"));
    assert!(host_lookup.contains("suspend_realtime_video_runtime"));
}

#[test]
fn mpv_wid_is_uint32_and_video_readiness_is_verified_by_pid_and_window_tree() {
    let host = std::fs::read_to_string("crates/autolive-native-video-host/src/lib.rs")
        .expect("native video host should be readable");

    assert!(host.contains("pub type MpvWindowId = u32;"));
    assert!(host.contains("pub fn mpv_wid(&self) -> Result<MpvWindowId"));
    assert!(host.contains("pub fn inspect_mpv_video_window("));
    assert!(host.contains("WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN"));
    assert!(host.contains("EnumChildWindows("));
    assert!(host.contains("GetWindowThreadProcessId(window"));
    assert!(host.contains("GetAncestor(window, GA_PARENT)"));
    assert!(host.contains("GetParent(window)"));
    assert!(host.contains("IsWindowVisible(window)"));
    assert!(host.contains("GetClientRect(window"));
    assert!(!host.contains("GetWindowText"));
}

#[test]
fn native_video_host_lifecycle_is_ordered_and_resized_with_the_parent() {
    let commands =
        std::fs::read_to_string("src/commands.rs").expect("commands.rs should be readable");
    let cleanup = commands
        .split_once("fn release_final_effect_video_host(")
        .map(|(_, tail)| tail)
        .expect("video host release helper should exist")
        .split("fn attach_final_effect_close_cleanup(")
        .next()
        .expect("video host release helper should have a bounded body");

    let suspend = cleanup
        .find("suspend_realtime_video_runtime")
        .expect("mpv runtime should be suspended before host destruction");
    let destroy = cleanup
        .find("destroy")
        .expect("native host should be destroyed during cleanup");
    assert!(suspend < destroy);
    assert!(commands.contains("tauri::WindowEvent::Resized"));
    assert!(commands.contains("resize_final_effect_video_host"));
}

#[test]
fn native_video_host_promotes_without_moving_resizing_or_activating() {
    let host = std::fs::read_to_string("crates/autolive-native-video-host/src/lib.rs")
        .expect("native video host should be readable");
    let promotion = host
        .split_once("pub(super) fn promote_to_top(")
        .map(|(_, tail)| tail)
        .expect("native video host promotion should exist")
        .split("pub(super) fn destroy(")
        .next()
        .expect("native video host promotion should have a bounded body");

    let owner_check = promotion
        .find("ensure_owner_thread(host)?")
        .expect("promotion must stay on the owner thread");
    let readiness_check = promotion
        .find("ready_window_id(host)?")
        .expect("promotion must validate the host window");
    let set_window_pos = promotion
        .find("SetWindowPos(")
        .expect("promotion must use the native z-order API");

    assert!(owner_check < readiness_check);
    assert!(readiness_check < set_window_pos);
    assert!(promotion.contains("Some(HWND_TOP)"));
    assert!(promotion.contains("SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW"));
}

#[test]
fn timed_out_main_thread_host_task_cannot_execute_late() {
    let commands =
        std::fs::read_to_string("src/commands.rs").expect("commands.rs should be readable");
    let helper = commands
        .split_once("async fn run_on_tauri_main_thread")
        .map(|(_, tail)| tail)
        .expect("main-thread helper should exist")
        .split("async fn create_final_effect_video_host")
        .next()
        .expect("main-thread helper should have a bounded body");

    assert!(helper.contains("task_cancelled.load(Ordering::Acquire)"));
    assert!(helper.contains("cancelled.store(true, Ordering::Release)"));
    assert!(helper.contains("receiver.recv_timeout(NATIVE_VIDEO_HOST_MAIN_THREAD_TIMEOUT)"));
}

#[test]
fn mpv_starts_paused_then_restores_the_requested_session_state() {
    let backend = std::fs::read_to_string("src/realtime_video_backend.rs")
        .expect("realtime_video_backend.rs should be readable");
    let runtime = std::fs::read_to_string("src/realtime_video_runtime.rs")
        .expect("realtime_video_runtime.rs should be readable");

    let launch_constructor = backend
        .split_once("impl MpvLaunchSpec {")
        .map(|(_, tail)| tail)
        .expect("mpv launch constructor should exist")
        .split("pub fn new_with_shader(")
        .next()
        .expect("mpv launch constructor should have a bounded body");
    assert!(launch_constructor.contains("arguments.push(OsString::from(\"--pause=yes\"));"));
    assert!(!launch_constructor.contains("\"--pause=no\""));
    assert!(runtime.contains("restore_requested_playback_state"));
    assert!(runtime.contains("MpvCommand::SetPause { paused: false }"));
}
