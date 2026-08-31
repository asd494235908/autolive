const BACKEND_SOURCE: &str = include_str!("../src/realtime_video_backend.rs");
const COMMANDS_SOURCE: &str = include_str!("../src/commands.rs");
const RUNTIME_SOURCE: &str = include_str!("../src/realtime_video_runtime.rs");

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
    let first_index = source
        .find(first)
        .unwrap_or_else(|| panic!("missing first source contract: {first}"));
    let second_index = source
        .find(second)
        .unwrap_or_else(|| panic!("missing second source contract: {second}"));
    assert!(
        first_index < second_index,
        "source contract must keep `{first}` before `{second}`"
    );
}

#[test]
fn intentional_suspend_restarts_the_same_backend_instead_of_reporting_session_loss() {
    let suspend = source_section(
        RUNTIME_SOURCE,
        "    fn suspend(&mut self)",
        "\n    fn record_source_backend(",
    );
    assert!(suspend.contains("RendererLifecycleState::Stopped"));
    assert!(suspend.contains("self.backend_epoch"));

    let disposition = source_section(
        RUNTIME_SOURCE,
        "fn gpu_prepare_disposition(",
        "\nimpl Default for RuntimeState",
    );
    assert!(
        disposition.contains("RendererLifecycleState::Stopped"),
        "prepare disposition must distinguish an intentional stopped renderer from a lost process"
    );
    assert_in_order(
        disposition,
        "RendererLifecycleState::Stopped",
        "GpuPrepareDisposition::RecoverLostSession",
    );
    assert!(
        disposition.contains("GpuPrepareDisposition::RestartRenderer"),
        "an intentionally suspended renderer must take the restart path"
    );

    let prepare = source_section(
        RUNTIME_SOURCE,
        "    fn prepare(\n        &mut self,",
        "\n    fn ensure_original(",
    );
    assert!(
        prepare.contains("self.status.lifecycle"),
        "prepare must pass the renderer lifecycle into the disposition decision"
    );
}

#[test]
fn reused_mpv_pid_skips_redundant_hwnd_surface_verification() {
    let gate = source_section(
        COMMANDS_SOURCE,
        "fn should_verify_mpv_video_surface(",
        "\n}",
    );
    assert!(gate.contains("previous_process_id"));
    assert!(gate.contains("current_process_id"));
    assert!(
        gate.contains("previous_process_id != current_process_id"),
        "surface verification should run only when the managed mpv PID changes"
    );

    let configure = source_section(
        COMMANDS_SOURCE,
        "pub async fn configure_realtime_video_cycle(",
        "\nfn command_error_from_realtime_video_prepare(",
    );
    assert_in_order(configure, "previous_process_id", "configure_cycle");
    assert_in_order(
        configure,
        "should_verify_mpv_video_surface",
        "verify_mpv_video_surface",
    );
}

#[test]
fn unexpected_mpv_exit_keeps_exit_code_and_bounded_stderr_tail() {
    let evidence = source_section(BACKEND_SOURCE, "pub struct MpvProcessExitEvidence {", "\n}");
    assert!(evidence.contains("pub exit_code: Option<i32>"));
    assert!(evidence.contains("pub stderr_tail: Vec<String>"));

    let poll = source_section(
        BACKEND_SOURCE,
        "    pub fn poll_exit_evidence(",
        "\n    pub fn cancel(",
    );
    assert!(poll.contains("try_wait()"));
    assert!(poll.contains("status.code()"));
    assert!(poll.contains("self.stderr_tail()"));

    let prepare = source_section(
        RUNTIME_SOURCE,
        "    fn prepare(\n        &mut self,",
        "\n    fn ensure_original(",
    );
    assert!(prepare.contains("poll_exit_evidence"));
    assert!(
        prepare.contains("exit_evidence"),
        "the fallback reason must retain the structured exit evidence instead of a boolean"
    );
}

#[test]
fn original_failure_is_terminal_but_intentional_stop_and_suspend_are_not() {
    let default_status = source_section(
        RUNTIME_SOURCE,
        "impl Default for MediaVideoBackendRuntimeStatus {",
        "\n#[derive(Debug)]\npub struct PrepareRealtimeRenderer",
    );
    assert!(default_status.contains("backend: VideoBackend::Source"));
    assert!(default_status.contains("activation: BackendActivation::Available"));
    assert!(default_status.contains("lifecycle: RendererLifecycleState::Stopped"));
    assert!(default_status.contains("process_id: None"));

    let original_failure = source_section(
        RUNTIME_SOURCE,
        "    fn record_original_failure(&mut self, reason: String)",
        "\n    fn record_original_started(",
    );
    assert!(original_failure.contains("self.bounded_failure_reason(&reason)"));
    assert!(original_failure.contains("MediaVideoBackendRuntimeStatus::default()"));
    assert!(original_failure.contains("BackendActivation::Failed"));
    assert!(original_failure.contains("RendererLifecycleState::Failed"));
    assert!(original_failure.contains("self.status.demotion_reason = Some(reason)"));
    assert!(!original_failure.contains("RendererLifecycleState::Stopped"));
    assert!(!original_failure.contains("process_id = Some"));

    let ensure_original = source_section(
        RUNTIME_SOURCE,
        "    fn ensure_original(\n        &mut self,",
        "\n    fn commit(",
    );
    assert!(ensure_original.contains("requested_launch_mode == MpvLaunchMode::Original"));
    assert!(ensure_original.contains("self.record_original_failure(error.to_string())"));

    let process_failure = source_section(
        RUNTIME_SOURCE,
        "    fn handle_process_failure(",
        "\n    fn transition_to_fallback(",
    );
    assert!(process_failure.contains("Some(MpvLaunchMode::Original)"));
    assert_in_order(
        process_failure,
        "self.record_original_failure(reason)",
        "self.stop_process()",
    );

    let stop = source_section(
        RUNTIME_SOURCE,
        "    fn stop(&mut self)",
        "\n    fn suspend(&mut self)",
    );
    assert!(stop.contains("self.status = MediaVideoBackendRuntimeStatus::default()"));
    assert!(!stop.contains("record_original_failure"));

    let suspend = source_section(
        RUNTIME_SOURCE,
        "    fn suspend(&mut self)",
        "\n    fn set_processing_enabled(",
    );
    assert!(suspend.contains("BackendActivation::Available"));
    assert!(suspend.contains("RendererLifecycleState::Stopped"));
    assert!(!suspend.contains("record_original_failure"));
}
