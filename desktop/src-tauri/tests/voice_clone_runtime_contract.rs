use autolive_desktop_core::media_library::SourceMediaDto;
use autolive_desktop_core::voice_clone::{VoiceCloneSegment, VoiceCloneSourceIndex};
use autolive_desktop_core::{
    PlaybackCore, VoiceCloneCommittedReplacement, VoiceClonePreparedSource, VoiceCloneRuntimeError,
};

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
        model: Some("tts_models/multilingual/multi-dataset/xtts_v2".to_owned()),
    }
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
