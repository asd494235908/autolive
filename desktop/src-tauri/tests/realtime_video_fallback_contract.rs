use std::fs;
use std::path::PathBuf;

fn workspace_file(relative: &str) -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative))
        .expect("contract source should be readable")
}

fn source_section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    source
        .split_once(start)
        .unwrap_or_else(|| panic!("missing source contract start: {start}"))
        .1
        .split_once(end)
        .unwrap_or_else(|| panic!("missing source contract end: {end}"))
        .0
}

fn assert_in_order(source: &str, first: &str, second: &str) {
    let first = source
        .find(first)
        .unwrap_or_else(|| panic!("missing first source contract: {first}"));
    let second = source
        .find(second)
        .unwrap_or_else(|| panic!("missing second source contract: {second}"));
    assert!(first < second);
}

#[test]
fn phase4_uses_one_explicit_mpv_mode_chain() {
    let backend = workspace_file("src/realtime_video_backend.rs");
    assert!(backend.contains("pub enum MpvLaunchMode"));
    assert!(backend.contains("Gpu(MpvGpuProfile)"));
    assert!(backend.contains("Cpu4"));
    assert!(backend.contains("Original"));
    assert!(backend.contains("Self::Gpu(MpvGpuProfile::D3d11ZeroCopy)"));
    assert!(backend.contains("Self::Gpu(MpvGpuProfile::D3d11Copy)"));
    assert!(backend.contains("Self::Gpu(MpvGpuProfile::VulkanCopy)"));
    assert!(backend.contains("Self::Gpu(MpvGpuProfile::SoftwareDecode)"));
    assert!(backend.contains("--hwdec=no"));
    assert!(backend.contains("--vf={CPU4_FILTER_CHAIN}"));
}

#[test]
fn every_runtime_failure_path_uses_the_single_fallback_owner() {
    let runtime = workspace_file("src/realtime_video_runtime.rs");
    let transition = runtime
        .split("fn transition_to_fallback(")
        .nth(1)
        .expect("single fallback owner")
        .split("fn start_renderer(")
        .next()
        .expect("fallback owner end");
    assert!(transition.contains("self.backend.current()"));
    assert!(transition.contains("self.stop_process()"));
    assert!(transition.contains("self.start_cpu4_process"));
    assert!(transition.contains("self.start_original_fallback"));
    assert_eq!(runtime.matches("fn transition_to_fallback(").count(), 1);
    assert!(runtime.matches("self.transition_to_fallback(").count() >= 5);
}

#[test]
fn phase4_fault_matrix_routes_all_failures_to_the_single_owner() {
    let runtime = workspace_file("src/realtime_video_runtime.rs");
    let prepare = source_section(
        &runtime,
        "    fn prepare(\n        &mut self,",
        "\n    fn ensure_original(",
    );
    assert!(prepare.contains("if let Err(error) = self.start_renderer"));
    assert!(prepare.contains("self.transition_to_fallback("));

    let launch = source_section(
        &runtime,
        "fn launch_gpu_renderer(",
        "\nfn launch_mode_renderer(",
    );
    assert!(launch.contains("wait_for_video_output("));
    assert!(launch.contains("process.cancel()"));
    let output_gate = source_section(
        &runtime,
        "fn wait_for_video_output(",
        "\nfn vo_passes_has_rendered_frame(",
    );
    assert!(output_gate.contains("MpvCommand::GetVideoOutputPasses"));
    assert!(output_gate.contains("mpv 首帧尚未呈现"));
    assert!(output_gate.contains("if !require_render_sample"));
    assert!(output_gate.contains("process.read_video_pts"));
    assert!(output_gate.contains("Original 视频输出已配置但未观察到首帧时间线"));
    assert!(output_gate.contains("unreported-original"));
    assert!(output_gate.contains("process.read_active_decoder"));
    assert!(output_gate.contains("started.elapsed() < MPV_PIPE_CONNECT_TIMEOUT"));

    let tick = source_section(
        &runtime,
        "    fn tick(&mut self)",
        "\n    fn record_eof_observation(",
    );
    assert!(tick.matches("poll_playback_observation").count() >= 3);
    assert!(tick.matches("record_tick_failure(error)").count() >= 4);
    assert!(!tick.contains("send_command(&MpvCommand::SetShaderOptions"));
    assert!(tick.contains("self.commit_pending_if_due"));
    assert!(tick.contains("self.poll_pending_shader_apply"));
    let cpu4_tick = source_section(
        tick,
        "if self.status.backend == VideoBackend::Cpu4",
        "if self.status.backend != VideoBackend::RealtimeGpu",
    );
    assert!(cpu4_tick.contains("poll_playback_observation"));
    assert!(cpu4_tick.contains("record_tick_failure(error)"));
    assert!(cpu4_tick.contains("self.observe_frame_budget("));

    let tick_failure = source_section(
        &runtime,
        "    fn observation_failure_reached_threshold(",
        "\n    fn observe_frame_budget(",
    );
    assert!(tick_failure.contains("TICK_FAILURE_DEMOTION_THRESHOLD"));
    assert!(tick_failure.contains("TRANSIENT_OBSERVATION_FAILURE_THRESHOLD"));
    assert!(tick_failure.contains("self.transition_to_fallback("));

    let frame_budget = source_section(
        &runtime,
        "    fn record_frame_budget_observation_failure(",
        "\n    fn stop(&mut self)",
    );
    assert!(frame_budget.contains("FRAME_BUDGET_VIOLATION_THRESHOLD"));
    assert!(frame_budget.contains("FRAME_BUDGET_OBSERVATION_FAILURE_THRESHOLD"));
    assert!(frame_budget.contains("record_frame_budget_observation_failure"));
    assert!(frame_budget.contains("连续渲染健康窗口超限"));
    assert!(frame_budget.contains("self.transition_to_fallback("));

    let fallback = source_section(
        &runtime,
        "    fn transition_to_fallback(",
        "\n    fn start_renderer(",
    );
    assert!(fallback.contains("MpvLaunchMode::Original => self.start_original_fallback"));
    assert!(fallback.contains("self.transition_to_fallback("));
    assert!(fallback.contains("self.record_original_failure"));

    let original = source_section(
        &runtime,
        "    fn start_original_fallback(",
        "\n    fn transition_to_fallback(",
    );
    assert_in_order(
        original,
        "self.process = Some(process)",
        "self.record_original_started(",
    );
}

#[test]
fn cpu4_reports_four_supported_and_seventy_nine_discarded_fields() {
    let runtime = workspace_file("src/realtime_video_runtime.rs");
    let status = workspace_file("../ui/src/media-video-backend-status.ts");
    for field in [
        "video.brightness_percent",
        "video.contrast_percent",
        "video.saturation_percent",
        "video.hue_rotation_degrees",
    ] {
        assert!(runtime.contains(field));
        assert!(status.contains(field));
    }
    assert!(runtime.contains("Cpu4Snapshot::from_ui"));
    assert!(status.contains("fields.length !== 83"));
    assert!(status.contains("value.unsupported_parameter_count !== 79"));
    assert!(status.contains("CPU4 固定未执行"));
}

#[test]
fn rust_owns_video_identity_and_ui_fails_closed_without_cpu_activation_ipc() {
    let commands = workspace_file("src/commands.rs");
    let app = workspace_file("../ui/src/App.tsx");
    let status = workspace_file("../ui/src/media-video-backend-status.ts");
    let configure_request = commands
        .split("pub struct ConfigureRealtimeVideoCycleRequestDto")
        .nth(1)
        .and_then(|section| {
            section
                .split("pub struct EnsureOriginalVideoRendererRequestDto")
                .next()
        })
        .expect("configure realtime video request DTO");
    assert!(!configure_request.contains("backend_epoch"));
    assert!(!configure_request.contains("clock_epoch"));
    assert!(!configure_request.contains("loop_index"));
    assert!(!configure_request.contains("playback_generation"));
    assert!(commands.contains("let runtime_status = state.realtime_video_runtime.status()"));
    assert!(commands.contains("let playback_generation = snapshot.playback_generation"));
    assert!(commands.contains("let loop_index = snapshot.loop_index"));
    assert!(status.contains("incoming.status_revision < current.status_revision"));
    assert!(status.contains("incoming.status_revision === current.status_revision"));
    assert!(status.contains("effective: null"));
    assert!(status.contains("status.backend === 'realtime_gpu' || status.backend === 'cpu4'"));
    assert!(!app.contains("activate_cpu4_video_backend"));
    assert!(!commands.contains("pub fn activate_cpu4_video_backend"));
}
