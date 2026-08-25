use autolive_desktop_core::interlude_player::{
    prepare_interlude_snapshot, InterludeAudioSelectionMode, InterludeAudioVariationMode,
    InterludeConfig, InterludeError, MAX_INTERLUDE_AUDIO_FILES,
};
use autolive_desktop_core::media_library::SourceMediaDto;
use autolive_desktop_core::speech_to_speech::{
    AudioTrackInput, AudioVariantCandidate, SpeechToSpeechContext,
};
use autolive_desktop_core::PlaybackCore;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "autolive-{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).expect("test directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn source(path: &str, file_name: &str) -> SourceMediaDto {
    SourceMediaDto {
        source_path: path.to_owned(),
        file_name: file_name.to_owned(),
        file_size_bytes: 1,
        duration_ms: Some(10_000),
        width: Some(1280),
        height: Some(720),
        frame_rate_fps: Some(30.0),
        audio_sample_rate_hz: Some(48_000),
        audio_channel_count: Some(2),
        mp4_sha256: Some("a".repeat(64)),
        mp4_hash_status: "ready".to_owned(),
    }
}

fn track_input() -> AudioTrackInput {
    AudioTrackInput {
        track_id: "track-1".to_owned(),
        source_kind: "original".to_owned(),
        audio_path_or_stream_ref: "/tmp/original.wav".to_owned(),
        audio_sha256: Some("d".repeat(64)),
        start_at_ms: 1_500,
        duration_ms: 1_500,
        sample_rate_hz: 48_000,
        channel_count: 2,
    }
}

fn speech_context(playback_generation: u64, loop_index: u64) -> SpeechToSpeechContext {
    SpeechToSpeechContext {
        track_id: "track-1".to_owned(),
        source_kind: "original".to_owned(),
        playback_generation,
        loop_index,
        segment_id: "segment-1".to_owned(),
        start_at_ms: 1_500,
        sample_rate_hz: 48_000,
        channel_count: 2,
        audio_path_or_stream_ref: "/tmp/original.wav".to_owned(),
        transcript_text: "当前话术".to_owned(),
        previous_variant_text: None,
        locked_fields: Vec::new(),
        locked_field_values: Vec::new(),
        target_duration_ms: 1_500,
        max_chars: 128,
        language: "zh-CN".to_owned(),
        rewrite_policy: "rewrite".to_owned(),
        timeout_ms: 3_000,
    }
}

fn audio_candidate(playback_generation: u64, loop_index: u64) -> AudioVariantCandidate {
    AudioVariantCandidate {
        playback_generation,
        loop_index,
        segment_id: "segment-1".to_owned(),
        start_at_ms: 1_500,
        variant_mode: "rewrite".to_owned(),
        audio_path_or_stream_ref: "/tmp/realtime-variant.wav".to_owned(),
        audio_sha256: "e".repeat(64),
        duration_ms: 1_500,
        sync_offset_ms: 0,
        sample_rate_hz: 48_000,
        channel_count: 2,
        ready: true,
    }
}

#[test]
fn enabled_interlude_rejects_empty_directory() {
    let temp = TempDirGuard::new("interlude-empty");
    let error = prepare_interlude_snapshot(InterludeConfig {
        enabled: true,
        directory: Some(temp.path().display().to_string()),
        ..InterludeConfig::default()
    })
    .expect_err("empty directory should be rejected");

    assert_eq!(error, InterludeError::NoUsableAudioFiles);
}

#[test]
fn interlude_scan_ignores_unsupported_extensions_and_returns_canonical_paths() {
    let temp = TempDirGuard::new("interlude-scan");
    let keep = temp.path().join("keep.mp3");
    let skip = temp.path().join("skip.txt");
    std::fs::write(&keep, b"mp3").expect("supported file");
    std::fs::write(&skip, b"txt").expect("unsupported file");

    let (config, snapshot) = prepare_interlude_snapshot(InterludeConfig {
        enabled: true,
        directory: Some(temp.path().join(".").display().to_string()),
        ..InterludeConfig::default()
    })
    .expect("catalog should scan");

    assert_eq!(
        config.directory.as_deref(),
        Some(
            std::fs::canonicalize(temp.path())
                .expect("canonical directory")
                .display()
                .to_string()
                .as_str()
        )
    );
    assert_eq!(
        snapshot.audio_files,
        vec![std::fs::canonicalize(&keep)
            .expect("canonical file")
            .display()
            .to_string()]
    );
    assert_eq!(snapshot.audio_count, 1);
    assert_eq!(snapshot.status, "ready");
}

#[test]
fn interlude_scan_discovers_supported_audio_in_nested_directory() {
    let temp = TempDirGuard::new("interlude-recursive-scan");
    let nested_audio = temp
        .path()
        .join("nested")
        .join("level-two")
        .join("nested.wav");
    std::fs::create_dir_all(nested_audio.parent().expect("nested parent"))
        .expect("nested directory");
    std::fs::write(&nested_audio, b"wav").expect("nested audio file");

    let (_, snapshot) = prepare_interlude_snapshot(InterludeConfig {
        enabled: true,
        directory: Some(temp.path().display().to_string()),
        ..InterludeConfig::default()
    })
    .expect("catalog should recursively scan nested directories");

    assert_eq!(
        snapshot.audio_files,
        vec![std::fs::canonicalize(&nested_audio)
            .expect("canonical nested audio file")
            .display()
            .to_string()]
    );
    assert_eq!(snapshot.audio_count, 1);
}

#[test]
fn interlude_scan_accepts_256_audio_files_and_rejects_the_257th() {
    let temp = TempDirGuard::new("interlude-file-limit");
    for index in 0..MAX_INTERLUDE_AUDIO_FILES {
        std::fs::write(temp.path().join(format!("audio-{index:03}.wav")), b"wav")
            .expect("bounded audio file");
    }

    let config = InterludeConfig {
        enabled: true,
        directory: Some(temp.path().display().to_string()),
        ..InterludeConfig::default()
    };
    let (_, snapshot) =
        prepare_interlude_snapshot(config.clone()).expect("256 audio files should be accepted");
    assert_eq!(
        snapshot.audio_count,
        u32::try_from(MAX_INTERLUDE_AUDIO_FILES).expect("file limit fits u32")
    );

    std::fs::write(temp.path().join("audio-256.wav"), b"wav").expect("257th audio file");
    assert_eq!(
        prepare_interlude_snapshot(config).expect_err("257 audio files should be rejected"),
        InterludeError::TooManyAudioFiles {
            count: MAX_INTERLUDE_AUDIO_FILES + 1,
            max_files: MAX_INTERLUDE_AUDIO_FILES,
        }
    );
}

#[test]
fn interlude_validation_rejects_out_of_range_values() {
    assert_eq!(
        InterludeConfig {
            interval_min_ms: 499,
            ..InterludeConfig::default()
        }
        .validate()
        .expect_err("min interval should be bounded"),
        InterludeError::IntervalMinOutOfRange
    );
    assert_eq!(
        InterludeConfig {
            interval_min_ms: 4_000,
            interval_max_ms: 3_000,
            ..InterludeConfig::default()
        }
        .validate()
        .expect_err("min interval must not exceed max"),
        InterludeError::IntervalOrderInvalid
    );
    assert_eq!(
        InterludeConfig {
            volume_db: -61.0,
            ..InterludeConfig::default()
        }
        .validate()
        .expect_err("volume should be bounded"),
        InterludeError::VolumeOutOfRange
    );
    assert_eq!(
        InterludeConfig {
            ducking_depth_db: 1.0,
            ..InterludeConfig::default()
        }
        .validate()
        .expect_err("ducking depth should be bounded"),
        InterludeError::DuckingDepthOutOfRange
    );
    assert_eq!(
        InterludeConfig {
            ducking_attack_ms: 1_001,
            ..InterludeConfig::default()
        }
        .validate()
        .expect_err("ducking attack should be bounded"),
        InterludeError::DuckingAttackOutOfRange
    );
    assert_eq!(
        InterludeConfig {
            ducking_release_ms: 3_001,
            ..InterludeConfig::default()
        }
        .validate()
        .expect_err("ducking release should be bounded"),
        InterludeError::DuckingReleaseOutOfRange
    );
}

#[test]
fn interlude_validation_accepts_zero_duration_ducking_transitions() {
    let config = InterludeConfig {
        ducking_attack_ms: 0,
        ducking_release_ms: 0,
        ..InterludeConfig::default()
    };

    config
        .validate()
        .expect("zero-duration ducking transitions should be accepted");
    let (_, snapshot) = prepare_interlude_snapshot(config)
        .expect("zero-duration ducking transitions should reach the runtime snapshot");
    assert_eq!(snapshot.ducking_attack_ms, 0);
    assert_eq!(snapshot.ducking_release_ms, 0);
}

#[test]
fn ducking_transition_range_errors_describe_the_zero_based_contract() {
    assert_eq!(
        InterludeError::DuckingAttackOutOfRange.to_string(),
        "闪避 Attack 必须在 0..=1000ms"
    );
    assert_eq!(
        InterludeError::DuckingReleaseOutOfRange.to_string(),
        "闪避 Release 必须在 0..=3000ms"
    );
}

#[test]
fn disabled_interlude_does_not_require_an_existing_directory_and_reports_disabled() {
    let temp = TempDirGuard::new("interlude-disabled");
    let missing_directory = temp.path().join("removed");
    let (_, snapshot) = prepare_interlude_snapshot(InterludeConfig {
        enabled: false,
        directory: Some(missing_directory.display().to_string()),
        ..InterludeConfig::default()
    })
    .expect("disabled interlude should not scan a missing directory");

    assert_eq!(snapshot.status, "disabled");
    assert!(snapshot.audio_files.is_empty());
}

#[test]
fn default_interlude_interval_matches_the_random_insertion_panel() {
    let config = InterludeConfig::default();
    let expected_default_pool = (1..=20)
        .map(|number| format!("p{number:02}"))
        .collect::<Vec<_>>();

    assert_eq!(config.interval_min_ms, 8_000);
    assert_eq!(config.interval_max_ms, 13_000);
    assert_eq!(
        config.audio_selection_mode,
        InterludeAudioSelectionMode::Random
    );
    assert_eq!(config.audio_fixed_preset_id, "p01");
    assert_eq!(config.audio_preset_ids, expected_default_pool);
    assert!(!config
        .audio_preset_ids
        .iter()
        .any(|id| id == "p21" || id == "p22"));
    assert!(!config.audio_mix_enabled);
    assert_eq!(config.audio_mix_pick_min, 1);
    assert_eq!(config.audio_mix_pick_max, 2);
    assert_eq!(
        config.audio_variation_mode,
        InterludeAudioVariationMode::EachPlayback
    );
    assert_eq!(config.audio_variation_period_min_ms, 8_000);
    assert_eq!(config.audio_variation_period_max_ms, 15_000);
}

#[test]
fn interlude_snapshot_preserves_fixed_selection_and_the_random_pool() {
    let (_, snapshot) = prepare_interlude_snapshot(InterludeConfig {
        audio_selection_mode: InterludeAudioSelectionMode::Fixed,
        audio_fixed_preset_id: "p22".to_owned(),
        audio_preset_ids: vec!["p01".to_owned(), "p22".to_owned()],
        audio_mix_enabled: true,
        audio_mix_pick_min: 2,
        audio_mix_pick_max: 4,
        audio_variation_mode: InterludeAudioVariationMode::Periodic,
        audio_variation_period_min_ms: 2_000,
        audio_variation_period_max_ms: 5_000,
        ..InterludeConfig::default()
    })
    .expect("fixed selection and its independent random pool should be accepted");

    assert_eq!(
        snapshot.audio_selection_mode,
        InterludeAudioSelectionMode::Fixed
    );
    assert_eq!(snapshot.audio_fixed_preset_id, "p22");
    assert_eq!(snapshot.audio_preset_ids, vec!["p01", "p22"]);
    assert!(snapshot.audio_mix_enabled);
    assert_eq!(snapshot.audio_mix_pick_min, 2);
    assert_eq!(snapshot.audio_mix_pick_max, 4);
    assert_eq!(
        snapshot.audio_variation_mode,
        InterludeAudioVariationMode::Periodic
    );
    assert_eq!(snapshot.audio_variation_period_min_ms, 2_000);
    assert_eq!(snapshot.audio_variation_period_max_ms, 5_000);
}

#[test]
fn interlude_selection_mode_uses_strict_wire_values() {
    assert_eq!(
        serde_json::to_string(&InterludeAudioSelectionMode::Fixed)
            .expect("fixed mode should serialize"),
        "\"fixed\""
    );
    assert_eq!(
        serde_json::from_str::<InterludeAudioSelectionMode>("\"random\"")
            .expect("random mode should deserialize"),
        InterludeAudioSelectionMode::Random
    );
    assert!(
        serde_json::from_str::<InterludeAudioSelectionMode>("\"each_playback\"").is_err(),
        "variation mode must not be accepted as a selection mode"
    );
}

#[test]
fn interlude_fixed_preset_accepts_only_exact_p01_through_p22() {
    for number in 1..=22 {
        InterludeConfig {
            audio_fixed_preset_id: format!("p{number:02}"),
            ..InterludeConfig::default()
        }
        .validate()
        .expect("p01-p22 should be accepted as fixed presets");
    }
    for audio_fixed_preset_id in ["", "p00", "p1", "p+1", "p23", "P01"] {
        assert_eq!(
            InterludeConfig {
                audio_fixed_preset_id: audio_fixed_preset_id.to_owned(),
                ..InterludeConfig::default()
            }
            .validate()
            .expect_err("only the exact p01-p22 format should be accepted"),
            InterludeError::AudioPresetInvalid
        );
    }
}

#[test]
fn interlude_validation_accepts_all_22_unique_presets() {
    let audio_preset_ids = (1..=22).map(|number| format!("p{number:02}")).collect();
    InterludeConfig {
        audio_preset_ids,
        ..InterludeConfig::default()
    }
    .validate()
    .expect("p01-p22 should be accepted exactly once each");
}

#[test]
fn interlude_validation_rejects_invalid_preset_pools() {
    assert_eq!(
        InterludeConfig {
            audio_preset_ids: Vec::new(),
            ..InterludeConfig::default()
        }
        .validate()
        .expect_err("preset pool must not be empty"),
        InterludeError::AudioPresetCountOutOfRange
    );
    assert_eq!(
        InterludeConfig {
            audio_preset_ids: vec!["p01".to_owned(); 23],
            ..InterludeConfig::default()
        }
        .validate()
        .expect_err("preset pool must contain at most 22 entries"),
        InterludeError::AudioPresetCountOutOfRange
    );
    assert_eq!(
        InterludeConfig {
            audio_preset_ids: vec!["p01".to_owned(), "p01".to_owned()],
            ..InterludeConfig::default()
        }
        .validate()
        .expect_err("preset pool must be unique"),
        InterludeError::AudioPresetDuplicate
    );
    for number in 1..=22 {
        InterludeConfig {
            audio_preset_ids: vec![format!("p{number:02}")],
            ..InterludeConfig::default()
        }
        .validate()
        .expect("p01-p22 should be accepted");
    }
    for audio_preset_id in ["", "p00", "p1", "p+1", "p23", "P01"] {
        assert_eq!(
            InterludeConfig {
                audio_preset_ids: vec![audio_preset_id.to_owned()],
                ..InterludeConfig::default()
            }
            .validate()
            .expect_err("only the exact p01-p22 format should be accepted"),
            InterludeError::AudioPresetInvalid
        );
    }
}

#[test]
fn interlude_variation_mode_uses_strict_wire_values() {
    assert_eq!(
        serde_json::to_string(&InterludeAudioVariationMode::EachPlayback)
            .expect("each playback mode should serialize"),
        "\"each_playback\""
    );
    assert_eq!(
        serde_json::from_str::<InterludeAudioVariationMode>("\"periodic\"")
            .expect("periodic mode should deserialize"),
        InterludeAudioVariationMode::Periodic
    );
    assert!(
        serde_json::from_str::<InterludeAudioVariationMode>("\"eachPlayback\"").is_err(),
        "camelCase mode must be rejected"
    );
}

#[test]
fn interlude_validation_rejects_invalid_mix_and_variation_ranges() {
    for (config, expected) in [
        (
            InterludeConfig {
                audio_mix_pick_min: 0,
                ..InterludeConfig::default()
            },
            InterludeError::AudioMixPickMinOutOfRange,
        ),
        (
            InterludeConfig {
                audio_mix_pick_max: 5,
                ..InterludeConfig::default()
            },
            InterludeError::AudioMixPickMaxOutOfRange,
        ),
        (
            InterludeConfig {
                audio_mix_pick_min: 3,
                audio_mix_pick_max: 2,
                ..InterludeConfig::default()
            },
            InterludeError::AudioMixPickOrderInvalid,
        ),
        (
            InterludeConfig {
                audio_variation_period_min_ms: 999,
                ..InterludeConfig::default()
            },
            InterludeError::AudioVariationPeriodMinOutOfRange,
        ),
        (
            InterludeConfig {
                audio_variation_period_max_ms: 60_001,
                ..InterludeConfig::default()
            },
            InterludeError::AudioVariationPeriodMaxOutOfRange,
        ),
        (
            InterludeConfig {
                audio_variation_period_min_ms: 5_000,
                audio_variation_period_max_ms: 4_000,
                ..InterludeConfig::default()
            },
            InterludeError::AudioVariationPeriodOrderInvalid,
        ),
    ] {
        assert_eq!(
            config.validate().expect_err("range should be rejected"),
            expected
        );
    }
}

#[test]
fn playback_snapshot_reports_effective_audio_source_priority_without_mutating_base_audio_source() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));

    assert_eq!(core.snapshot().effective_audio_source, "original");

    let generation = core.snapshot().playback_generation;
    core.set_processing_switches(true, false, true);
    core.mark_media_processing_ready(generation, "/tmp/processed.mp4".to_owned(), "f".repeat(64))
        .expect("processed media should be accepted");
    // ready 已立即 commit，无需再 commit。
    assert!(!core.commit_media_processing_if_ready());
    assert_eq!(core.snapshot().effective_audio_source, "processed_original");

    core.stage_audio_variant_candidate(
        &track_input(),
        &speech_context(generation, 0),
        audio_candidate(generation, 0),
        250,
    )
    .expect("realtime candidate should stage");
    core.commit_audio_variant_candidate(generation, 0, "segment-1")
        .expect("realtime candidate should commit");
    let snapshot = core.snapshot();
    assert_eq!(
        snapshot.current_audio_source.as_deref(),
        Some("realtime_variant")
    );
    assert_eq!(snapshot.effective_audio_source, "realtime_variant");
}

#[test]
fn portaudio_interlude_commands_are_registered_on_the_single_output_path() {
    let main = std::fs::read_to_string("src/main.rs").expect("main.rs should be readable");
    let commands =
        std::fs::read_to_string("src/commands.rs").expect("commands.rs should be readable");

    for command in [
        "start_portaudio_interlude",
        "pause_portaudio_interlude",
        "resume_portaudio_interlude",
        "stop_portaudio_interlude",
    ] {
        assert!(main.contains(command), "{command} must be registered");
        assert!(commands.contains(command), "{command} must be implemented");
    }
    assert!(commands.contains("output_control.start_interlude("));
    assert!(!commands.contains("PortAudioOutput::new(\n        output_status.sample_rate_hz"));

    let start = commands
        .find("fn start_portaudio_interlude_blocking(")
        .expect("PortAudio interlude implementation should exist");
    let end = commands[start..]
        .find("pub fn pause_portaudio_interlude(")
        .map(|offset| start + offset)
        .expect("PortAudio interlude implementation should have a bounded source segment");
    let implementation = &commands[start..end];
    assert!(commands.contains(
        "ValidatedAudioStreamConfiguration::new(request.audio.clone(), request.audio_variants.clone())"
    ));
    assert!(implementation.contains("validate_interlude_processing_request(&request)?;"));
    assert!(implementation.contains("request.audio_variants"));
    assert!(implementation.contains("interlude_audio_variant_speed_mismatch"));
    assert!(implementation.contains("build_audio_stream_filter_graph_with_ambient("));
    assert!(implementation.contains("&request.audio"));
    assert!(implementation.contains("&request.audio_variants"));
    assert!(implementation.contains("Some(filter_plan.filter_graph)"));
    assert!(implementation.contains("filter_plan.quality_pitch"));
    assert!(implementation.contains("filter_plan.pcm_effects"));
    assert!(implementation.contains("requires_ambient_input"));
    assert!(implementation.contains("request.audio.playback_speed"));
    assert_eq!(
        implementation
            .matches("AudioMixerTask::start_scheduled_candidate")
            .count(),
        1
    );
}

#[test]
fn saving_interlude_config_does_not_interrupt_the_current_interlude() {
    let commands =
        std::fs::read_to_string("src/commands.rs").expect("commands.rs should be readable");
    let start = commands
        .find("pub fn set_interlude_config(")
        .expect("set interlude config command should exist");
    let end = commands[start..]
        .find("pub async fn start_portaudio_interlude(")
        .map(|offset| start + offset)
        .expect("set interlude config should have a bounded source segment");
    let implementation = &commands[start..end];

    assert!(commands.contains("pub audio_selection_mode: InterludeAudioSelectionMode"));
    assert!(commands.contains("pub audio_fixed_preset_id: String"));
    assert!(implementation.contains("audio_selection_mode: request.audio_selection_mode"));
    assert!(implementation.contains("audio_fixed_preset_id: request.audio_fixed_preset_id"));
    assert!(implementation.contains("playback.set_interlude_snapshot("));
    assert!(!implementation.contains("stop_interlude_mixer"));
    assert!(!implementation.contains("旧 PortAudio 插话停止确认失败"));
}
