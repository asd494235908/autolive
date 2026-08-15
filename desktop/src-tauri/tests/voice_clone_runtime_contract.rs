use autolive_desktop_core::media_library::SourceMediaDto;
use autolive_desktop_core::runtime_resources::RuntimeResourceLayout;
use autolive_desktop_core::voice_clone::{
    hash_voice_clone_text, VoiceCloneSegment, VoiceCloneSourceIndex,
};
use autolive_desktop_core::{
    PlaybackCore, VoiceCloneCommittedPlayback, VoiceCloneCommittedReplacement,
    VoiceClonePreGenerationItemState, VoiceClonePreparedSource, VoiceCloneRuntimeError,
    VOICE_CLONE_SYNTHESIS_MODEL_ID,
};
use std::path::Path;

#[test]
fn release_voice_paths_use_the_versioned_app_data_layout() {
    let target = if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else {
        return;
    };
    let layout = RuntimeResourceLayout::for_target(Path::new("/app-data"), target)
        .expect("supported target should have a layout");

    assert_eq!(
        layout
            .target_root
            .join("voice-worker")
            .join(if cfg!(windows) {
                "autolive-voice-clone-worker.exe"
            } else {
                "autolive-voice-clone-worker"
            }),
        Path::new("/app-data")
            .join("runtime-resources/v0.1.0")
            .join(target)
            .join("voice-worker")
            .join(if cfg!(windows) {
                "autolive-voice-clone-worker.exe"
            } else {
                "autolive-voice-clone-worker"
            })
    );
    assert_eq!(
        layout.model_root,
        Path::new("/app-data/runtime-resources/v0.1.0/common/voice-models")
    );
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

fn prepared_source(generation: u64, source_path: &str) -> VoiceClonePreparedSource {
    VoiceClonePreparedSource {
        operation_id: "prepare-operation".to_owned(),
        source_index: VoiceCloneSourceIndex {
            source_generation: generation,
            source_path: source_path.to_owned(),
            segments: vec![VoiceCloneSegment {
                start_ms: 1_000,
                end_ms: 3_000,
                text: "当前话术".to_owned(),
            }],
        },
        source_sha256: "a".repeat(64),
        reference_audio_path: "/tmp/voice-clone/reference.wav".to_owned(),
        reference_audio_sha256: "b".repeat(64),
        sample_rate_hz: 48_000,
        channel_count: 2,
        total_duration_ms: 10_000,
        model: Some("demucs+small+xtts_v2".to_owned()),
    }
}

fn committed_replacement(
    generation: u64,
    source_path: &str,
    operation_id: &str,
) -> VoiceCloneCommittedReplacement {
    VoiceCloneCommittedReplacement {
        source_generation: generation,
        source_path: source_path.to_owned(),
        operation_id: operation_id.to_owned(),
        input_text: "替换后的固定话术".to_owned(),
        replacement_audio_reference: "/tmp/voice-clone/replacement.wav".to_owned(),
        replacement_audio_sha256: "c".repeat(64),
        replacement_duration_ms: 10_000,
        replace_at_ms: 1_500,
        resume_at_ms: 3_000,
        model: Some(VOICE_CLONE_SYNTHESIS_MODEL_ID.to_owned()),
    }
}

fn committed_playback(
    generation: u64,
    source_path: &str,
    operation_id: &str,
) -> VoiceCloneCommittedPlayback {
    VoiceCloneCommittedPlayback {
        source_generation: generation,
        source_path: source_path.to_owned(),
        operation_id: operation_id.to_owned(),
        input_text: "当前文案".to_owned(),
        text_sha256: hash_voice_clone_text("当前文案"),
        audio_reference: "/tmp/voice-clone/current-text.wav".to_owned(),
        audio_sha256: "e".repeat(64),
        duration_ms: 1_200,
        start_at_ms: 1_500,
        model: Some(VOICE_CLONE_SYNTHESIS_MODEL_ID.to_owned()),
    }
}

#[test]
fn pre_generation_counts_duplicate_text_once_and_survives_only_normal_loops() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    let generation = core.snapshot().playback_generation;
    core.set_voice_clone_prepared_source(prepared_source(generation, "/tmp/source.mp4"))
        .expect("prepared source should be accepted");
    core.start().expect("playback should start");
    let text_sha256 = hash_voice_clone_text("相同文案");
    let item = |preset_id: &str| VoiceClonePreGenerationItemState {
        preset_id: preset_id.to_owned(),
        text_sha256: text_sha256.clone(),
        status: "pending".to_owned(),
        audio_sha256: None,
        duration_ms: None,
        error: None,
    };
    core.start_voice_clone_pre_generation(
        "batch-1",
        generation,
        vec![item("preset-a"), item("preset-b")],
    )
    .expect("batch should start");
    assert!(core.finish_voice_clone_pre_generation_text(
        "batch-1",
        generation,
        &text_sha256,
        "cached",
        Some("c".repeat(64)),
        Some(1_200),
        None,
    ));
    let ready = core.snapshot().voice_clone_pre_generation;
    assert_eq!(
        (ready.completed, ready.cache_hits, ready.generated),
        (2, 1, 0)
    );
    assert_eq!(ready.status, "ready");

    core.complete_loop().expect("loop should complete");
    assert_eq!(core.snapshot().voice_clone_pre_generation.status, "ready");

    core.set_source(source("/tmp/next.mp4", "next.mp4"));
    assert_eq!(core.snapshot().voice_clone_pre_generation.status, "idle");
}

#[test]
fn loop_change_does_not_restart_or_invalidate_current_clone_playback() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    let generation = core.snapshot().playback_generation;
    core.set_voice_clone_prepared_source(prepared_source(generation, "/tmp/source.mp4"))
        .expect("prepared source should be accepted");
    core.start().expect("playback should start");

    let plan = core
        .start_voice_clone_playback("当前文案", 1_500, "playback-operation", false)
        .expect("current text playback should start");
    core.apply_voice_clone_playback(committed_playback(
        plan.source_generation,
        &plan.source_path,
        &plan.operation_id,
    ))
    .expect("current text playback should apply");

    core.complete_loop().expect("loop should complete");

    let snapshot = core.snapshot();
    assert_eq!(snapshot.voice_clone_playback.status, "playing");
    assert_eq!(
        snapshot.voice_clone_playback.operation_id.as_deref(),
        Some("playback-operation")
    );
    assert_eq!(snapshot.loop_index, 1);
}

#[test]
fn current_voice_clone_playback_uses_the_synthesis_model_identity() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    let generation = core.snapshot().playback_generation;
    core.set_voice_clone_prepared_source(prepared_source(generation, "/tmp/source.mp4"))
        .expect("prepared source should be accepted");
    core.start().expect("playback should start");

    let plan = core
        .start_voice_clone_playback("当前文案", 1_500, "playback-operation", false)
        .expect("current text playback should start");

    assert_eq!(plan.model.as_deref(), Some(VOICE_CLONE_SYNTHESIS_MODEL_ID));
    assert_eq!(
        core.snapshot().voice_clone_playback.model.as_deref(),
        Some(VOICE_CLONE_SYNTHESIS_MODEL_ID)
    );
}

#[test]
fn finishing_current_clone_playback_restores_ready_source_without_clearing_track() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    let generation = core.snapshot().playback_generation;
    core.set_voice_clone_prepared_source(prepared_source(generation, "/tmp/source.mp4"))
        .expect("prepared source should be accepted");
    core.start().expect("playback should start");

    let plan = core
        .start_voice_clone_playback("当前文案", 1_500, "playback-operation", false)
        .expect("current text playback should start");
    core.apply_voice_clone_playback(committed_playback(
        plan.source_generation,
        &plan.source_path,
        &plan.operation_id,
    ))
    .expect("current text playback should apply");
    core.finish_voice_clone_playback("playback-operation")
        .expect("current text playback should finish");

    let snapshot = core.snapshot();
    assert_eq!(snapshot.voice_clone_playback.status, "ready");
    assert_eq!(
        snapshot.voice_clone_playback.audio_reference.as_deref(),
        Some("/tmp/voice-clone/current-text.wav")
    );
    assert_eq!(snapshot.voice_clone_replacement.status, "ready");
    assert_eq!(
        snapshot.voice_clone_playback.model.as_deref(),
        Some(VOICE_CLONE_SYNTHESIS_MODEL_ID)
    );

    core.clear_voice_clone_playback();
    let cleared = core.snapshot();
    assert_eq!(cleared.voice_clone_playback.status, "ready");
    assert_eq!(
        cleared.voice_clone_playback.model.as_deref(),
        Some(VOICE_CLONE_SYNTHESIS_MODEL_ID)
    );
}

#[test]
fn failing_current_clone_playback_clears_audio_only_for_the_active_operation() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    let generation = core.snapshot().playback_generation;
    core.set_voice_clone_prepared_source(prepared_source(generation, "/tmp/source.mp4"))
        .expect("prepared source should be accepted");
    core.start().expect("playback should start");

    let plan = core
        .start_voice_clone_playback("当前文案", 1_500, "playback-operation", false)
        .expect("current text playback should start");
    core.apply_voice_clone_playback(committed_playback(
        plan.source_generation,
        &plan.source_path,
        &plan.operation_id,
    ))
    .expect("current text playback should apply");

    core.fail_voice_clone_playback("playback-operation", "前端音频加载失败")
        .expect("active playback should be markable as failed");

    let snapshot = core.snapshot();
    assert_eq!(snapshot.voice_clone_playback.status, "failed");
    assert_eq!(
        snapshot.voice_clone_playback.error.as_deref(),
        Some("前端音频加载失败")
    );
    assert!(snapshot.voice_clone_playback.audio_reference.is_none());
    assert!(snapshot.voice_clone_playback.audio_sha256.is_none());
    assert!(snapshot.voice_clone_playback.duration_ms.is_none());
}

#[test]
fn failing_current_clone_playback_rejects_wrong_operation_or_non_playing_state() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    let generation = core.snapshot().playback_generation;
    core.set_voice_clone_prepared_source(prepared_source(generation, "/tmp/source.mp4"))
        .expect("prepared source should be accepted");
    core.start().expect("playback should start");

    let plan = core
        .start_voice_clone_playback("当前文案", 1_500, "playback-operation", false)
        .expect("current text playback should start");
    core.apply_voice_clone_playback(committed_playback(
        plan.source_generation,
        &plan.source_path,
        &plan.operation_id,
    ))
    .expect("current text playback should apply");

    assert_eq!(
        core.fail_voice_clone_playback("other-operation", "错误操作"),
        Err(VoiceCloneRuntimeError::VoiceClonePlaybackStale)
    );

    core.fail_voice_clone_playback("playback-operation", "前端音频加载失败")
        .expect("active playback should be markable as failed");
    assert_eq!(
        core.fail_voice_clone_playback("playback-operation", "重复失败"),
        Err(VoiceCloneRuntimeError::VoiceClonePlaybackStale)
    );
}

#[test]
fn source_change_clears_voice_clone_state_without_changing_processing_switches() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    core.set_processing_switches(true, true, true);
    core.set_voice_clone_prepared_source(prepared_source(
        core.snapshot().playback_generation,
        "/tmp/source.mp4",
    ))
    .expect("prepared source should be accepted");
    core.start().expect("playback should start");
    let operation = core
        .start_voice_clone_replacement("替换后的固定话术", 1_500, "replace-operation", false)
        .expect("replacement should start");
    core.apply_voice_clone_replacement(committed_replacement(
        operation.source_generation,
        &operation.source_path,
        &operation.operation_id,
    ))
    .expect("replacement should apply");

    core.set_source(source("/tmp/other.mp4", "other.mp4"));

    let snapshot = core.snapshot();
    assert!(snapshot.video_processing_enabled);
    assert!(snapshot.audio_processing_enabled);
    assert!(snapshot.realtime_audio_variant_enabled);
    assert_eq!(snapshot.voice_clone_replacement.status, "idle");
    assert!(snapshot
        .voice_clone_replacement
        .replacement_audio_reference
        .is_none());
    assert!(snapshot
        .voice_clone_replacement
        .replacement_audio_sha256
        .is_none());
}

#[test]
fn replacement_result_is_rejected_after_playback_generation_changes() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    core.set_voice_clone_prepared_source(prepared_source(
        core.snapshot().playback_generation,
        "/tmp/source.mp4",
    ))
    .expect("prepared source should be accepted");
    core.start().expect("playback should start");
    let operation = core
        .start_voice_clone_replacement("替换后的固定话术", 1_500, "replace-operation", false)
        .expect("replacement should start");
    let replacement = committed_replacement(
        operation.source_generation,
        &operation.source_path,
        &operation.operation_id,
    );

    core.stop();

    assert_eq!(
        core.apply_voice_clone_replacement(replacement),
        Err(VoiceCloneRuntimeError::VoiceCloneReplacementStale)
    );
}

#[test]
fn completing_loop_restores_original_audio_and_keeps_video_processing_flags() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    core.set_processing_switches(true, false, false);
    core.set_voice_clone_prepared_source(prepared_source(
        core.snapshot().playback_generation,
        "/tmp/source.mp4",
    ))
    .expect("prepared source should be accepted");
    core.start().expect("playback should start");
    let operation = core
        .start_voice_clone_replacement("替换后的固定话术", 1_500, "replace-operation", false)
        .expect("replacement should start");
    core.apply_voice_clone_replacement(committed_replacement(
        operation.source_generation,
        &operation.source_path,
        &operation.operation_id,
    ))
    .expect("replacement should apply");

    core.complete_loop().expect("loop should complete");

    let snapshot = core.snapshot();
    assert!(snapshot.video_processing_enabled);
    assert_eq!(snapshot.voice_clone_replacement.status, "ready");
    assert!(snapshot
        .voice_clone_replacement
        .replacement_audio_reference
        .is_none());
    assert!(snapshot
        .voice_clone_replacement
        .replacement_audio_sha256
        .is_none());
}

#[test]
fn completed_loop_keeps_prepared_source_available_for_next_replacement() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    let generation = core.snapshot().playback_generation;
    core.set_voice_clone_prepared_source(prepared_source(generation, "/tmp/source.mp4"))
        .expect("prepared source should be accepted");
    core.start().expect("playback should start");

    let first_operation = core
        .start_voice_clone_replacement("第一句替换话术", 1_500, "replace-operation-1", false)
        .expect("first replacement should start");
    core.apply_voice_clone_replacement(committed_replacement(
        first_operation.source_generation,
        &first_operation.source_path,
        &first_operation.operation_id,
    ))
    .expect("first replacement should apply");
    core.complete_loop().expect("loop should complete");

    assert_eq!(core.snapshot().voice_clone_replacement.status, "ready");
    assert!(core.voice_clone_prepared_source().is_some());

    core.set_playback_position(1_500);
    let second_operation = core
        .start_voice_clone_replacement("第二句替换话术", 1_500, "replace-operation-2", false)
        .expect("replacement should remain available after loop");
    assert_eq!(second_operation.source_generation, generation);
    assert_eq!(second_operation.resume_at_ms, 3_000);
}

#[test]
fn replacement_can_start_when_current_position_has_no_indexed_speech() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    let generation = core.snapshot().playback_generation;
    core.set_voice_clone_prepared_source(prepared_source(generation, "/tmp/source.mp4"))
        .expect("prepared source should be accepted");
    core.start().expect("playback should start");
    core.set_playback_position(5_000);

    let operation = core
        .start_voice_clone_replacement("无人声位置也替换", 5_000, "replace-no-speech", false)
        .expect("replacement should start outside an indexed speech segment");

    assert_eq!(operation.replace_at_ms, 5_000);
    assert_eq!(operation.resume_at_ms, 8_000);
}

#[test]
fn replacement_without_indexed_speech_is_capped_by_source_duration() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    let generation = core.snapshot().playback_generation;
    core.set_voice_clone_prepared_source(prepared_source(generation, "/tmp/source.mp4"))
        .expect("prepared source should be accepted");
    core.start().expect("playback should start");
    core.set_playback_position(9_000);

    let operation = core
        .start_voice_clone_replacement("接近结尾也可以替换", 9_000, "replace-near-end", false)
        .expect("replacement should start outside an indexed speech segment");

    assert_eq!(operation.replace_at_ms, 9_000);
    assert_eq!(operation.resume_at_ms, 10_000);
}

#[test]
fn multiple_replacements_can_start_in_the_same_loop() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    let generation = core.snapshot().playback_generation;
    core.set_voice_clone_prepared_source(prepared_source(generation, "/tmp/source.mp4"))
        .expect("prepared source should be accepted");
    core.start().expect("playback should start");

    let first_operation = core
        .start_voice_clone_replacement("第一句", 1_500, "replace-operation-1", false)
        .expect("first replacement should start");
    core.apply_voice_clone_replacement(committed_replacement(
        first_operation.source_generation,
        &first_operation.source_path,
        &first_operation.operation_id,
    ))
    .expect("first replacement should apply");

    core.set_playback_position(5_000);
    let second_operation = core
        .start_voice_clone_replacement("第二句", 5_000, "replace-operation-2", false)
        .expect("second replacement should start while the first is active");

    assert_eq!(core.snapshot().voice_clone_replacement.status, "generating");
    assert_eq!(second_operation.replace_at_ms, 5_000);
    assert_eq!(second_operation.resume_at_ms, 8_000);
}

#[test]
fn realtime_audio_worker_occupancy_rejects_fixed_text_replacement() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    core.set_voice_clone_prepared_source(prepared_source(
        core.snapshot().playback_generation,
        "/tmp/source.mp4",
    ))
    .expect("prepared source should be accepted");
    core.start().expect("playback should start");

    assert_eq!(
        core.start_voice_clone_replacement("替换后的固定话术", 1_500, "replace-operation", true),
        Err(VoiceCloneRuntimeError::RealtimeAudioWorkerBusy)
    );

    let snapshot = core.snapshot();
    assert_eq!(snapshot.voice_clone_replacement.status, "ready");
    assert!(snapshot
        .voice_clone_replacement
        .replacement_audio_reference
        .is_none());
}

#[test]
fn ordinary_audio_processing_must_be_committed_before_fixed_replacement() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    core.set_processing_switches(false, true, false);
    core.set_voice_clone_prepared_source(prepared_source(
        core.snapshot().playback_generation,
        "/tmp/source.mp4",
    ))
    .expect("prepared source should be accepted");
    core.start().expect("playback should start");

    assert_eq!(
        core.start_voice_clone_replacement("替换后的固定话术", 1_500, "replace-operation", false),
        Err(VoiceCloneRuntimeError::VoiceCloneAudioProcessingNotReady)
    );
}

#[test]
fn fixed_replacement_uses_the_committed_processed_video_as_audio_base() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    core.set_processing_switches(false, true, false);
    let generation = core.snapshot().playback_generation;
    core.mark_media_processing_ready(generation, "/tmp/processed.mp4".to_owned(), "c".repeat(64))
        .expect("processed media should be accepted");
    assert!(core.commit_media_processing_if_ready());
    core.set_voice_clone_prepared_source(prepared_source(generation, "/tmp/source.mp4"))
        .expect("prepared source should be accepted");
    core.start().expect("playback should start");

    let plan = core
        .start_voice_clone_replacement("替换后的固定话术", 1_500, "replace-operation", false)
        .expect("replacement should start");

    assert_eq!(plan.audio_base_path, "/tmp/processed.mp4");
}

#[test]
fn late_replacement_result_is_rejected_after_current_phrase_ends() {
    let mut core = PlaybackCore::default();
    core.set_source(source("/tmp/source.mp4", "source.mp4"));
    core.set_voice_clone_prepared_source(prepared_source(
        core.snapshot().playback_generation,
        "/tmp/source.mp4",
    ))
    .expect("prepared source should be accepted");
    core.start().expect("playback should start");
    let operation = core
        .start_voice_clone_replacement("替换后的固定话术", 1_500, "replace-operation", false)
        .expect("replacement should start");
    core.set_playback_position(3_000);

    assert_eq!(
        core.apply_voice_clone_replacement(committed_replacement(
            operation.source_generation,
            &operation.source_path,
            &operation.operation_id,
        )),
        Err(VoiceCloneRuntimeError::VoiceCloneReplacementStale)
    );
}
