use autolive_desktop_core::realtime_video_backend::{
    MpvCommand, MpvPlaybackSpeed, MpvVideoObservation,
};
use serde_json::{json, Value};

const AUDIO_CLOCK_SOURCE: &str = include_str!("../src/audible_audio_clock.rs");
const RUNTIME_SOURCE: &str = include_str!("../src/realtime_video_runtime.rs");

fn command_json(command: &MpvCommand) -> Value {
    serde_json::from_str(&command.ipc_json_line().expect("command should serialize"))
        .expect("command should be valid JSON")
}

fn source_section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    source
        .split(start)
        .nth(1)
        .unwrap_or_else(|| panic!("missing source section start: {start}"))
        .split(end)
        .next()
        .unwrap_or_else(|| panic!("missing source section end: {end}"))
}

#[test]
fn playback_speed_accepts_only_finite_values_in_the_closed_safe_range() {
    for value in [0.25, 1.0, 3.06, 4.0] {
        assert_eq!(
            MpvPlaybackSpeed::new(value)
                .expect("boundary speed should be accepted")
                .as_f64(),
            value
        );
    }

    for value in [
        0.249_999,
        4.000_001,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        assert!(
            MpvPlaybackSpeed::new(value).is_err(),
            "unsafe speed must be rejected: {value}"
        );
    }
}

#[test]
fn observation_commands_serialize_only_fixed_mpv_properties() {
    assert_eq!(
        command_json(&MpvCommand::SetPlaybackSpeed {
            speed: MpvPlaybackSpeed::new(1.5).expect("valid speed"),
        }),
        json!({"command": ["set_property", "speed", 1.5]})
    );
    assert_eq!(
        command_json(&MpvCommand::GetPlaybackTime),
        json!({"command": ["get_property", "time-pos"]})
    );
    assert_eq!(
        command_json(&MpvCommand::GetEstimatedVideoFps),
        json!({"command": ["get_property", "estimated-vf-fps"]})
    );
    assert_eq!(
        command_json(&MpvCommand::GetPaused),
        json!({"command": ["get_property", "pause"]})
    );
    assert_eq!(
        command_json(&MpvCommand::GetSeeking),
        json!({"command": ["get_property", "seeking"]})
    );
    assert_eq!(
        command_json(&MpvCommand::GetPausedForCache),
        json!({"command": ["get_property", "paused-for-cache"]})
    );
}

#[test]
fn av_sync_consumes_observed_mpv_pause_seek_and_buffering_facts() {
    let av_sync = source_section(
        RUNTIME_SOURCE,
        "    fn observe_av_sync(",
        "\n    fn tick(&mut self)",
    );
    assert!(RUNTIME_SOURCE.contains("read_playback_state"));
    assert!(av_sync.contains("buffering: playback_state.paused_for_cache"));
    assert!(av_sync.contains("seeking: playback_state.seeking"));
    assert!(!av_sync.contains("buffering: false"));
    assert!(!av_sync.contains("seeking: false"));
}

#[test]
fn original_source_clock_does_not_wait_for_estimated_fps() {
    let source_tick = RUNTIME_SOURCE
        .split("let budget = ObservationTickBudget::start();")
        .nth(1)
        .expect("observation tick budget")
        .split(
            "if self.status.backend == VideoBackend::Source\n            && self.status.lifecycle",
        )
        .nth(1)
        .expect("active Source tick branch")
        .split("if self.cycle_controller.is_some()")
        .next()
        .expect("Source tick branch end");
    assert!(source_tick.contains("self.poll_playback_observation()"));
    assert!(!source_tick.contains("self.poll_source_fps_response()"));

    let available_tick = RUNTIME_SOURCE
        .split("if self.status.activation == BackendActivation::Available")
        .nth(1)
        .expect("Available tick branch")
        .split("if self.status.backend == VideoBackend::Cpu4")
        .next()
        .expect("Available tick branch end");
    assert!(available_tick.contains("self.poll_playback_observation()"));
    assert!(available_tick.contains("initialize_cycle_after_source_observation("));

    let original_startup = RUNTIME_SOURCE
        .split("if !require_render_sample")
        .nth(1)
        .expect("Original startup branch")
        .split("if !render_sample_observed")
        .next()
        .expect("Original startup branch end");
    assert!(original_startup.contains("process.read_video_pts("));
    assert!(!original_startup.contains("GetEstimatedVideoFps"));
}

#[test]
fn gpu_neutral_prepare_claims_realtime_status_before_installing_the_cycle_controller() {
    let configure = source_section(
        RUNTIME_SOURCE,
        "    fn configure_cycle(",
        "\n    fn finish_cycle_configuration(",
    );
    let prepare = source_section(
        RUNTIME_SOURCE,
        "    fn prepare(\n        &mut self,\n        request: PrepareRealtimeRenderer,",
        "\n    fn configure_cycle(",
    );

    assert!(
        configure
            .find("self.prepare(")
            .expect("neutral GPU prepare")
            < configure
                .find("self.finish_cycle_configuration(config)")
                .expect("cycle controller installation")
    );
    let gpu_prepare = prepare
        .split_once("MpvLaunchMode::Gpu(_) => {}")
        .expect("GPU prepare branch")
        .1;
    assert!(gpu_prepare.contains("self.claim_realtime_status()"));
    assert!(gpu_prepare.contains("prepared_renderer_state(self.current.is_some())"));
}

#[test]
fn source_lightweight_tick_never_mutates_or_commits_a_cycle_controller() {
    let source_tick = RUNTIME_SOURCE
        .split("let budget = ObservationTickBudget::start();")
        .nth(1)
        .expect("observation tick budget")
        .split("if self.status.activation == BackendActivation::Available")
        .next()
        .expect("Source tick branch end");
    for forbidden in [
        "cycle_controller.as_mut",
        "cycle_controller =",
        "initialize_cycle_after_source_observation",
        "prepare_next_controller_plan",
        "commit_pending_if_due",
        "read_video_observation",
    ] {
        assert!(
            !source_tick.contains(forbidden),
            "Source lightweight tick must not own controller work: {forbidden}"
        );
    }
    assert!(source_tick.contains("&& self.cycle_controller.is_none()"));

    let processing_switch = source_section(
        RUNTIME_SOURCE,
        "    fn set_processing_enabled(",
        "\n    fn neutralize_to_source_with_fallback(",
    );
    assert!(
        processing_switch.find("self.cycle_controller = None")
            < processing_switch.find("self.neutralize_to_source_with_fallback()?")
    );
}

#[test]
fn eof_or_loop_boundary_fps_is_completed_by_the_first_full_mpv_observation() {
    let boundary_reset = source_section(
        RUNTIME_SOURCE,
        "    fn reset_cycle_controller_for_boundary(",
        "\n    fn advance_after_eof(",
    );
    assert!(boundary_reset.contains("VideoCycleEvent::SourceBoundary"));
    assert!(boundary_reset.contains("source_fps: None"));
    assert!(boundary_reset.contains("self.begin_source_transition()"));

    let source_transition = source_section(
        RUNTIME_SOURCE,
        "    fn begin_source_transition(",
        "\n    fn handle(",
    );
    assert!(source_transition.contains("VideoApplyState::SourceTransitioning"));

    let available_tick = RUNTIME_SOURCE
        .split("if self.status.activation == BackendActivation::Available")
        .nth(1)
        .expect("Available observation tick")
        .split("if self.status.backend == VideoBackend::Cpu4")
        .next()
        .expect("Available observation tick end");
    assert!(available_tick.contains("self.poll_playback_observation()"));
    assert!(available_tick.contains("initialize_cycle_after_source_observation("));

    let initialize = source_section(
        RUNTIME_SOURCE,
        "    fn initialize_cycle_after_source_observation(",
        "\n    fn observe_presented_pts_liveness(",
    );
    assert!(
        initialize.contains("VideoCycleEvent::MpvObservation"),
        "an existing SourceBoundary identity with source_fps=None must consume the first complete observation instead of returning early"
    );
}

#[test]
fn original_fallback_returns_before_realtime_claim_and_never_installs_a_controller() {
    let prepare = source_section(
        RUNTIME_SOURCE,
        "    fn prepare(\n        &mut self,\n        request: PrepareRealtimeRenderer,",
        "\n    fn configure_cycle(",
    );
    let original = prepare
        .split("MpvLaunchMode::Original => {")
        .nth(1)
        .expect("Original prepare branch")
        .split("MpvLaunchMode::Gpu(_) => {}")
        .next()
        .expect("Original prepare branch end");
    assert!(original.contains("self.record_neutral_source_status()?"));
    assert!(original.contains("return Ok(self.status.clone())"));
    assert!(!original.contains("self.claim_realtime_status()"));

    let availability = source_section(
        RUNTIME_SOURCE,
        "fn cycle_session_is_available(",
        "\nfn neutral_source_launch_mode(",
    );
    assert!(availability.contains("processing_enabled"));
    assert!(availability.contains("process_present"));
    assert!(availability.contains("session_present"));
    assert!(availability.contains("Some(MpvLaunchMode::Gpu(_) | MpvLaunchMode::Cpu4)"));
    assert!(!availability.contains("MpvLaunchMode::Original"));

    let finish = source_section(
        RUNTIME_SOURCE,
        "    fn finish_cycle_configuration(",
        "\n    fn healthy_effect_session_for_generation(",
    );
    assert!(finish.contains("if !cycle_session_is_available("));
    let controller_clear = finish
        .find("self.cycle_controller = None")
        .expect("clear controller before configuring fallback");
    let controller_install = finish
        .find("self.cycle_controller = Some")
        .expect("install effect controller");
    assert!(controller_clear < controller_install);
}

#[test]
fn cpu4_process_without_an_active_plan_is_published_as_available() {
    let start_cpu4 = source_section(
        RUNTIME_SOURCE,
        "    fn start_cpu4_process(",
        "\n    fn start_original_fallback(",
    );
    let state_assignment = start_cpu4
        .find("prepared_renderer_state(active_plan.is_some())")
        .expect("CPU4 started state must depend on whether an active plan exists");
    let active_plan_promotion = start_cpu4
        .find("self.current = Some(RealtimeVideoPlan")
        .expect("CPU4 active plan promotion");
    assert!(state_assignment < active_plan_promotion);
}

#[test]
fn recovery_boundary_is_owned_by_the_video_sync_controller() {
    assert!(!AUDIO_CLOCK_SOURCE.contains("        Recovery,"));
    assert!(RUNTIME_SOURCE.contains("boundary: SyncBoundary::Recovery"));
}

#[test]
fn video_observation_converts_seconds_to_milliseconds_and_accepts_fps_boundaries() {
    for (seconds, expected_ms, fps) in [(0.0, 0, 1.0), (1.234, 1_234, 60.0), (1.2346, 1_235, 240.0)]
    {
        let observation = MpvVideoObservation::from_responses(
            &json!({"error": "success", "data": seconds}),
            &json!({"error": "success", "data": fps}),
        )
        .expect("valid mpv responses")
        .expect("observation should be ready");

        assert_eq!(observation.media_pts_ms, expected_ms);
        assert_eq!(observation.source_fps, fps);
    }
}

#[test]
fn null_data_means_the_video_observation_is_not_ready() {
    for (time, fps) in [
        (
            json!({"error": "success", "data": null}),
            json!({"error": "success", "data": 60.0}),
        ),
        (
            json!({"error": "success", "data": 1.0}),
            json!({"error": "success", "data": null}),
        ),
    ] {
        assert_eq!(
            MpvVideoObservation::from_responses(&time, &fps)
                .expect("null data is a valid not-ready response"),
            None
        );
    }
}

#[test]
fn malformed_or_out_of_range_observations_fail_closed() {
    let valid_time = json!({"error": "success", "data": 1.0});
    let valid_fps = json!({"error": "success", "data": 60.0});
    let cases = [
        (
            json!({"error": "success"}),
            valid_fps.clone(),
            "missing time data",
        ),
        (
            valid_time.clone(),
            json!({"error": "success"}),
            "missing fps data",
        ),
        (
            json!({"error": "success", "data": "1.0"}),
            valid_fps.clone(),
            "string time",
        ),
        (
            valid_time.clone(),
            json!({"error": "success", "data": "60"}),
            "string fps",
        ),
        (
            json!({"error": "success", "data": -0.001}),
            valid_fps.clone(),
            "negative time",
        ),
        (
            json!({"error": "success", "data": 1.844_674_407_370_955e16}),
            valid_fps.clone(),
            "millisecond overflow",
        ),
        (
            valid_time.clone(),
            json!({"error": "success", "data": 0.0}),
            "zero fps",
        ),
        (
            valid_time.clone(),
            json!({"error": "success", "data": 240.000_001}),
            "fps above maximum",
        ),
        (
            json!({"data": 1.0}),
            valid_fps.clone(),
            "missing success status",
        ),
        (
            json!({"error": "failure", "data": 1.0}),
            valid_fps,
            "failure status",
        ),
    ];

    for (time, fps, label) in cases {
        assert!(
            MpvVideoObservation::from_responses(&time, &fps).is_err(),
            "invalid response must fail closed: {label}"
        );
    }
}

#[test]
fn observation_source_has_no_alternate_clock_or_arbitrary_property_api() {
    let source = include_str!("../src/realtime_video_backend.rs");
    for forbidden in [
        "container-fps",
        "get_time_us",
        "GetProperty {",
        "property: String",
        "property: &str",
        "fn get_property(",
    ] {
        assert!(
            !source.contains(forbidden),
            "restricted mpv observation source must not contain {forbidden}"
        );
    }
}
