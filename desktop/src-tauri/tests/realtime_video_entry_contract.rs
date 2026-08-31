use std::fs;
use std::path::PathBuf;

fn workspace_file(relative: &str) -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(manifest.join(relative)).expect("contract source should be readable")
}

#[test]
fn command_configures_the_managed_mpv_cycle_instead_of_webview_css() {
    let commands = workspace_file("src/commands.rs");
    let main = workspace_file("src/main.rs");
    let configure = commands
        .split("configure_realtime_video_cycle(")
        .nth(1)
        .expect("configure realtime command")
        .split("fn command_error_from_realtime_video_prepare")
        .next()
        .expect("configure realtime command body");

    assert!(configure.contains("VideoCycleConfig::try_new"));
    assert!(configure.contains("ConfigureRealtimeVideoCycle {"));
    assert!(configure.contains(".configure_cycle("));
    assert!(
        configure
            .find(".configure_cycle(")
            .expect("runtime configure")
            < configure
                .find("drop(transition_guard)")
                .expect("transition release"),
        "playback intent must not overtake runtime cycle configuration"
    );
    assert!(!configure.contains("source.frame_rate_fps"));
    assert!(!configure.contains("VideoCycleController::new"));
    assert!(!configure.contains("compile_gpu83_realtime_parameters"));
    assert!(!configure.contains("prepare_webview"));
    for legacy in [
        "prepare_media_video_stream_period",
        "read_media_video_stream_chunk",
        "ack_media_video_stream_chunk",
        "commit_media_video_stream_period",
        "seek_media_video_stream",
        "stop_media_video_stream",
    ] {
        assert!(
            !commands.contains(legacy),
            "legacy command remains: {legacy}"
        );
        assert!(
            !main.contains(legacy),
            "legacy command remains registered: {legacy}"
        );
    }
}

#[test]
fn desktop_routes_formal_video_cycles_only_to_mpv() {
    let app = workspace_file("../ui/src/App.tsx");
    let main = workspace_file("src/main.rs");
    assert!(app.contains("'configure_realtime_video_cycle'"));
    assert!(!app.contains("'prepare_realtime_video_plan'"));
    assert!(!app.contains("'commit_realtime_video_plan'"));
    assert!(main.contains("stop_realtime_video_renderer"));
    for legacy in [
        "prepare_media_video_stream_period",
        "read_media_video_stream_chunk",
        "ack_media_video_stream_chunk",
        "commit_media_video_stream_period",
        "seek_media_video_stream",
        "stop_media_video_stream",
        "VideoMse",
        "video_stream",
    ] {
        assert!(
            !app.contains(legacy),
            "legacy video contract remains: {legacy}"
        );
    }
    assert!(app.contains("const MPV_REALTIME_VIDEO_ENABLED = true"));
}

#[test]
fn webview_video_prepare_and_commit_are_not_production_commands() {
    let main = workspace_file("src/main.rs");
    let build = workspace_file("build.rs");
    let permissions = workspace_file("permissions/command-sets.toml");

    for retired in [
        "prepare_realtime_video_plan",
        "commit_realtime_video_plan",
        "sync_realtime_video_renderer",
    ] {
        assert!(
            !main.contains(retired),
            "retired WebView scheduler command remains registered: {retired}"
        );
        assert!(
            !build.contains(&format!("\"{retired}\"")),
            "retired command remains in the Tauri manifest: {retired}"
        );
        assert!(
            !permissions.contains(&format!("allow-{}", retired.replace('_', "-"))),
            "retired command remains window-accessible: {retired}"
        );
    }

    for retained in [
        "configure_realtime_video_cycle",
        "seek_playback",
        "stop_realtime_video_renderer",
    ] {
        assert!(
            main.contains(retained),
            "required runtime command missing: {retained}"
        );
        assert!(build.contains(&format!("\"{retained}\"")));
    }
    let final_effect_permissions = permissions
        .split("identifier = \"final-effect-commands\"")
        .nth(1)
        .expect("final effect permissions");
    assert!(permissions.contains("allow-seek-playback"));
    assert!(!final_effect_permissions.contains("allow-seek-playback"));
}

#[test]
fn original_video_is_registered_as_the_same_managed_mpv_session() {
    let commands = workspace_file("src/commands.rs");
    let runtime = workspace_file("src/realtime_video_runtime.rs");
    let main = workspace_file("src/main.rs");
    let build = workspace_file("build.rs");
    let permissions = workspace_file("permissions/command-sets.toml");

    let command = commands
        .split("ensure_original_video_renderer(")
        .nth(1)
        .expect("original renderer command")
        .split("configure_realtime_video_cycle(")
        .next()
        .expect("original renderer command body");
    let runtime_state = runtime
        .split("impl RuntimeState")
        .nth(1)
        .expect("runtime state implementation");
    let ensure = runtime_state
        .split("fn ensure_original(")
        .nth(1)
        .expect("original runtime entry")
        .split("fn commit(")
        .next()
        .expect("original runtime entry body");
    let sync = runtime_state
        .split("fn synchronize(")
        .nth(1)
        .expect("runtime sync entry")
        .split("fn tick(")
        .next()
        .expect("runtime sync entry body");

    assert!(command.contains("PrepareOriginalRenderer"));
    assert!(command.contains("resolve_mpv_executable"));
    assert!(!command.contains("original_video_renderer_not_allowed"));
    assert!(command.contains("resolve_mpv_shader"));
    assert!(command.contains("app.path().resource_dir()"));
    assert!(ensure.contains("start_neutral_process("));
    assert!(ensure.contains("neutral_source_launch_mode("));
    assert!(ensure.contains("request.playback_generation < cursor.playback_generation"));
    assert!(ensure.contains("resolve_sync_transition("));
    assert!(sync.contains("self.backend.launch_mode() == MpvLaunchMode::Original"));
    assert!(sync.contains("self.record_original_failure"));
    assert!(main.contains("ensure_original_video_renderer"));
    assert!(build.contains(r#""ensure_original_video_renderer""#));
    assert!(permissions.contains("allow-ensure-original-video-renderer"));
}

#[test]
fn failed_source_terminal_status_bypasses_managed_hwnd_verification() {
    let commands = workspace_file("src/commands.rs");
    let verification = commands
        .split("async fn verify_mpv_video_surface(")
        .nth(1)
        .expect("mpv surface verification")
        .split("#[tauri::command]")
        .next()
        .expect("mpv surface verification end");

    let failed_source = verification
        .find("status.backend == VideoBackend::Source")
        .expect("failed source terminal backend gate");
    let failed_activation = verification
        .find("status.activation == BackendActivation::Failed")
        .expect("failed source terminal activation gate");
    let missing_pid_error = verification
        .find("mpv 已返回视频后端状态，但没有可验证的受管进程 PID")
        .expect("managed surface missing PID error");

    assert!(failed_source < missing_pid_error);
    assert!(failed_activation < missing_pid_error);
    assert!(verification[..missing_pid_error].contains("return Ok(())"));
}

#[test]
fn paused_effect_startup_does_not_wait_for_an_impossible_render_sample() {
    let runtime = workspace_file("src/realtime_video_runtime.rs");
    let cpu4_start = runtime
        .split("fn start_cpu4_process(")
        .nth(1)
        .expect("CPU4 startup")
        .split("fn start_original_fallback(")
        .next()
        .expect("CPU4 startup body");
    let gpu_launch = runtime
        .split("fn launch_gpu_renderer(")
        .nth(1)
        .expect("GPU renderer launch")
        .split("fn launch_mode_renderer(")
        .next()
        .expect("GPU renderer launch body");
    let mode_launch = runtime
        .split("fn launch_mode_renderer(")
        .nth(1)
        .expect("mode renderer launch")
        .split("fn restore_requested_playback_state(")
        .next()
        .expect("mode renderer launch body");
    let readiness = runtime
        .split("fn wait_for_video_output(")
        .nth(1)
        .expect("video output readiness gate")
        .split("fn vo_passes_has_rendered_frame(")
        .next()
        .expect("video output readiness gate body");

    for (name, launch) in [("GPU", gpu_launch), ("CPU4/Original", mode_launch)] {
        let restore = launch
            .find("restore_requested_playback_state(&process, session.paused)")
            .unwrap_or_else(|| panic!("{name} must restore the requested playback state"));
        let wait = launch
            .find("wait_for_video_output(")
            .unwrap_or_else(|| panic!("{name} must confirm video output readiness"));
        assert!(
            restore < wait,
            "{name} must resume real playback before waiting for a render sample"
        );
    }

    assert!(gpu_launch.contains("!session.paused"));
    assert!(mode_launch.contains("mode != MpvLaunchMode::Original && !session.paused"));
    let cpu4_uses_requested_state = !cpu4_start.contains("launch_session.paused = true");
    let cpu4_confirms_after_resume = cpu4_start
        .find("MpvCommand::SetPause { paused: false }")
        .zip(cpu4_start.rfind("wait_for_video_output("))
        .is_some_and(|(resume, wait)| resume < wait);
    assert!(
        cpu4_uses_requested_state || cpu4_confirms_after_resume,
        "CPU4 playing startup must confirm a render sample only after resuming playback"
    );
    assert!(readiness.contains("MpvCommand::GetVideoOutputConfigured"));
    assert!(readiness.contains("process.read_video_pts("));
    assert!(readiness.contains("process.read_active_decoder("));
    assert!(readiness.contains("MpvCommand::GetVideoOutputPasses"));
}

#[test]
fn production_bundle_contains_the_only_allowed_gpu83_shader() {
    let config = workspace_file("tauri.conf.json");
    let shader = workspace_file("resources/shaders/gpu83.hook");

    assert!(config.contains(r#""resources/shaders/gpu83.hook""#));
    assert!(shader.contains("//!DESC AutoLive verified realtime pixel effects"));
    assert!(shader.contains("//!HOOK MAIN"));
}
