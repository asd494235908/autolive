use autolive_desktop_core::interlude_player::{
    prepare_interlude_snapshot, InterludeConfig, InterludeError,
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
    let nested_dir = temp.path().join("nested");
    std::fs::write(&keep, b"mp3").expect("supported file");
    std::fs::write(&skip, b"txt").expect("unsupported file");
    std::fs::create_dir_all(&nested_dir).expect("nested directory");
    std::fs::write(nested_dir.join("nested.wav"), b"wav").expect("nested file");

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

    assert_eq!(config.interval_min_ms, 8_000);
    assert_eq!(config.interval_max_ms, 13_000);
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
}
