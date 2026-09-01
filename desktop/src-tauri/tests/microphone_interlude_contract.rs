use autolive_desktop_core::microphone_interlude::{
    list_input_devices, MicrophoneInterludeConfigDto, MicrophoneInterludeController,
    MicrophoneInterludeError, MicrophoneInterludeState, MicrophoneSensitivity,
};

#[test]
fn microphone_wire_values_and_defaults_are_stable() {
    assert_eq!(
        serde_json::to_string(&MicrophoneInterludeState::Disabled).unwrap(),
        "\"disabled\""
    );
    assert_eq!(
        serde_json::to_string(&MicrophoneInterludeState::Speaking).unwrap(),
        "\"speaking\""
    );
    assert_eq!(
        serde_json::to_string(&MicrophoneSensitivity::Standard).unwrap(),
        "\"standard\""
    );

    let config = MicrophoneInterludeConfigDto::default();
    assert!(!config.enabled);
    assert_eq!(config.sample_rate_hz, 48_000);
    assert_eq!(config.sensitivity, MicrophoneSensitivity::Standard);
    assert!(config.aec_enabled);
    assert!(config.noise_suppression_enabled);
    assert!(config.agc_enabled);
}

#[test]
fn microphone_config_rejects_unbounded_or_invalid_values() {
    let config = MicrophoneInterludeConfigDto {
        device_id: Some("x".repeat(257)),
        ..Default::default()
    };
    assert_eq!(
        config.validate().unwrap_err(),
        MicrophoneInterludeError::DeviceIdTooLong
    );

    let config = MicrophoneInterludeConfigDto {
        sample_rate_hz: 1,
        ..Default::default()
    };
    assert_eq!(
        config.validate().unwrap_err(),
        MicrophoneInterludeError::SampleRateUnsupported
    );

    let config = MicrophoneInterludeConfigDto {
        device_id: Some("default-device".to_owned()),
        ..Default::default()
    };
    assert_eq!(
        config.validate().unwrap_err(),
        MicrophoneInterludeError::DeviceIdInvalid
    );

    let valid_fingerprint = MicrophoneInterludeConfigDto {
        device_id: Some("pa-input-mme-0123456789abcdef".to_owned()),
        ..Default::default()
    };
    assert!(valid_fingerprint.validate().is_ok());
}

#[test]
fn microphone_controller_starts_disabled_until_audio_bridge_is_committed() {
    let controller = MicrophoneInterludeController::default();
    assert_eq!(
        controller.status().state,
        MicrophoneInterludeState::Disabled
    );

    let status = controller
        .configure(MicrophoneInterludeConfigDto::default())
        .expect("default microphone config is valid");
    assert_eq!(status.generation, 0);
    assert_eq!(status.state, MicrophoneInterludeState::Disabled);
    assert!(!status.media_muted);
}

#[test]
fn microphone_stop_is_idempotent_and_invalidates_old_generation() {
    let controller = MicrophoneInterludeController::default();
    let first = controller
        .stop()
        .expect("stop without a session is idempotent");
    assert_eq!(first.state, MicrophoneInterludeState::Disabled);
    assert_eq!(first.generation, 1);

    let second = controller.stop().expect("repeated stop remains idempotent");
    assert_eq!(second.state, MicrophoneInterludeState::Disabled);
    assert_eq!(second.generation, 2);
    assert!(!second.media_muted);
}

#[test]
fn input_device_listing_never_falls_back_to_webview_or_network() {
    match list_input_devices() {
        Ok(devices) => {
            assert!(devices.iter().all(|device| !device.id.is_empty()));
            assert!(devices
                .iter()
                .all(|device| device.id.starts_with("pa-input-")));
            assert!(devices.iter().all(|device| !device.name.contains('\n')));
        }
        Err(error) => assert_eq!(error, MicrophoneInterludeError::InputBackendUnavailable),
    }
}

#[test]
fn lifecycle_contract_cancels_worker_after_disabling_audio_bridge() {
    let commands = std::fs::read_to_string("src/commands.rs").expect("commands source");
    let stop = commands
        .split("fn stop_microphone_interlude(&self)")
        .nth(1)
        .and_then(|tail| tail.split("fn pause_microphone_interlude").next())
        .expect("AppState microphone stop helper");
    assert!(
        stop.find("disable_microphone").unwrap()
            < stop
                .find("microphone_interlude\n            .stop")
                .unwrap(),
        "PortAudio bridge must be disabled before DSP cancellation and Join"
    );
    assert!(stop.contains("继续执行上面的取消/Join"));

    let output = std::fs::read_to_string("src/audio_cycle_output.rs").expect("output source");
    assert!(output.contains("available: Arc<AtomicBool>"));
    assert!(output.contains("bridge.mark_unavailable()"));
    assert!(output.contains("fail_microphone_bridge(&mut microphone_bridge)"));
    assert!(output.contains("麦克风故障后恢复普通音频出口失败"));
    assert!(output.contains("OutputCommand::Clear"));

    let microphone =
        std::fs::read_to_string("src/microphone_interlude.rs").expect("microphone source");
    assert!(microphone.contains("MICROPHONE_CALLBACK_STALL_BUDGET"));
    assert!(microphone.contains("MICROPHONE_INPUT_OVERFLOW_BUDGET"));
    assert!(microphone.contains("render_reference_drop_count"));
    assert!(microphone.contains("回声参考连续丢帧"));
    assert!(microphone.contains("callback_count"));
    assert!(microphone.contains("callback_monitor_started"));
    assert!(microphone.contains("PortAudio callback 心跳在停止预算内未推进"));
    assert!(microphone.contains("state.state != MicrophoneInterludeState::Stopping"));
    assert!(microphone.contains("inner.state == MicrophoneInterludeState::Opening"));
    assert!(microphone.contains("catch_unwind(AssertUnwindSafe"));
    assert!(microphone.contains("if state.paused"));
    assert!(microphone.contains("VadEnergyGate"));
    assert!(microphone.contains("原始输入 RMS/SNR 门控"));
    let gate_update = microphone
        .find("bridge.set_gate_speaking(speaking)")
        .expect("speaking gate update");
    let priority_callback = microphone
        .find("if started_speaking")
        .expect("speaking priority callback");
    assert!(
        gate_update < priority_callback,
        "speaking must mute the main track before stopping lower-priority sources"
    );
}

#[test]
fn microphone_stop_keeps_joining_when_output_control_lookup_fails() {
    let commands = std::fs::read_to_string("src/commands.rs").expect("commands source");
    let stop = commands
        .split("fn stop_microphone_interlude(&self)")
        .nth(1)
        .and_then(|tail| tail.split("fn pause_microphone_interlude").next())
        .expect("AppState microphone stop helper");
    assert!(
        stop.contains("Ok(Some(control))") && stop.contains("Err(error) => Some(error)"),
        "output control lookup failures must be retained while Stop/Join still runs"
    );
    assert!(
        !stop.contains("audio_output_control_if_started()?"),
        "microphone stop must not short-circuit before Worker Join"
    );
}

#[test]
fn audio_cycle_shutdown_reclaims_microphone_before_destroying_output_task() {
    let commands = std::fs::read_to_string("src/commands.rs").expect("commands source");
    let shutdown = commands
        .split("fn stop_audio_cycle_output(&self)")
        .nth(1)
        .and_then(|tail| tail.split("fn stop_audio_for_shutdown").next())
        .expect("audio cycle shutdown helper");
    assert!(
        shutdown.find("stop_microphone_interlude").unwrap()
            < shutdown.find("take_audio_cycle_output_task").unwrap(),
        "microphone DSP/PortAudio ownership must be reclaimed before output task destruction"
    );
    assert!(
        shutdown.contains("first_error.get_or_insert(error)"),
        "shutdown must continue cleaning later resources after an earlier microphone error"
    );
}

#[test]
fn playback_pause_resume_forwards_to_microphone_before_output_transition() {
    let commands = std::fs::read_to_string("src/commands.rs").expect("commands source");
    let pause = commands
        .split("fn pause_audio_output(&self)")
        .nth(1)
        .and_then(|tail| tail.split("fn stop_audio_for_playback").next())
        .expect("pause helper");
    assert!(pause.find("pause_microphone_interlude").unwrap() < pause.find(".pause()").unwrap());

    let resume = commands
        .split("fn resume_audio_output(&self, app: &AppHandle)")
        .nth(1)
        .and_then(|tail| tail.split("fn stop_audio_for_playback").next())
        .unwrap_or_default();
    assert!(
        resume.contains("resume_microphone_interlude"),
        "audio resume must restore microphone listening state"
    );
}

#[test]
fn commands_expose_only_bounded_microphone_ipc_and_no_speech_pipeline() {
    let commands = std::fs::read_to_string("src/commands.rs").expect("commands source");
    for command in [
        "list_portaudio_input_devices",
        "start_microphone_interlude",
        "stop_microphone_interlude",
        "get_microphone_interlude_status",
        "set_microphone_interlude_config",
    ] {
        assert!(commands.contains(command), "missing command {command}");
    }
    let command_area = commands
        .split("pub fn list_portaudio_input_devices")
        .nth(1)
        .and_then(|tail| tail.split("pub async fn set_audio_output_backend").next())
        .unwrap_or_default();
    for forbidden in ["ASR", "LLM", "TTS", "speech_to_speech", "direct_model"] {
        assert!(
            !command_area.contains(forbidden),
            "forbidden pipeline: {forbidden}"
        );
    }
}
