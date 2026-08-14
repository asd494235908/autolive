use autolive_desktop_core::speech_to_speech::{
    AudioTrackInput, AudioVariantCandidate, CandidateValidationError, SpeechToSpeechContext,
    SpeechToSpeechDecision, SpeechToSpeechResult, SpeechToSpeechWorkerCapabilities,
};

fn input() -> AudioTrackInput {
    AudioTrackInput {
        track_id: "track_0001".to_owned(),
        source_kind: "local_file".to_owned(),
        audio_path_or_stream_ref: "file:///tmp/source.wav".to_owned(),
        audio_sha256: Some("a".repeat(64)),
        start_at_ms: 0,
        duration_ms: 2_000,
        sample_rate_hz: 48_000,
        channel_count: 2,
    }
}

fn context() -> SpeechToSpeechContext {
    SpeechToSpeechContext {
        track_id: "track_0001".to_owned(),
        source_kind: "local_file".to_owned(),
        playback_generation: 4,
        loop_index: 2,
        segment_id: "segment_0001".to_owned(),
        start_at_ms: 0,
        sample_rate_hz: 48_000,
        channel_count: 2,
        audio_path_or_stream_ref: "file:///tmp/source.wav".to_owned(),
        transcript_text: "今天介绍这款产品".to_owned(),
        previous_variant_text: None,
        locked_fields: vec!["产品名".to_owned()],
        locked_field_values: vec!["这款产品".to_owned()],
        target_duration_ms: 1_000,
        max_chars: 40,
        language: "zh-CN".to_owned(),
        rewrite_policy: "keep_locked_fields".to_owned(),
        timeout_ms: 800,
    }
}

fn candidate() -> AudioVariantCandidate {
    AudioVariantCandidate {
        playback_generation: 4,
        loop_index: 2,
        segment_id: "segment_0001".to_owned(),
        start_at_ms: 0,
        variant_mode: "rewrite".to_owned(),
        audio_path_or_stream_ref: "file:///tmp/variant.wav".to_owned(),
        audio_sha256: "b".repeat(64),
        duration_ms: 980,
        sync_offset_ms: 40,
        sample_rate_hz: 48_000,
        channel_count: 2,
        ready: true,
    }
}

#[test]
fn candidate_accepts_current_generation_and_audio_format() {
    let result = candidate().validate_against(&input(), &context(), 120);
    assert!(result.is_ok(), "candidate should be accepted: {result:?}");
}

#[test]
fn stale_candidate_is_rejected_without_touching_current_audio_slot() {
    let mut stale = candidate();
    stale.loop_index = 1;
    assert_eq!(
        stale.validate_against(&input(), &context(), 120),
        Err(CandidateValidationError::StalePlaybackGenerationOrLoop)
    );
}

#[test]
fn invalid_sync_format_and_hash_are_rejected() {
    let mut invalid = candidate();
    invalid.sync_offset_ms = 121;
    assert_eq!(
        invalid.validate_against(&input(), &context(), 120),
        Err(CandidateValidationError::SyncOffsetExceeded)
    );

    invalid = candidate();
    invalid.audio_sha256 = "not-a-sha256".to_owned();
    assert_eq!(
        invalid.validate_against(&input(), &context(), 120),
        Err(CandidateValidationError::InvalidSHA256)
    );

    invalid = candidate();
    invalid.duration_ms = 1_100;
    assert_eq!(
        invalid.validate_against(&input(), &context(), 120),
        Err(CandidateValidationError::DurationOutsideTarget)
    );
}

#[test]
fn result_requires_audio_only_for_rewrite_and_unavailable_worker_is_explicit() {
    let keep_original = SpeechToSpeechResult {
        decision: SpeechToSpeechDecision::KeepOriginal,
        text: "今天介绍这款产品".to_owned(),
        audio_path_or_stream_ref: None,
        audio_sha256: None,
        duration_ms: None,
        sync_offset_ms: None,
        sample_rate_hz: None,
        channel_count: None,
        latency_ms: 20,
        model: "test".to_owned(),
        fallback_reason: None,
    };
    assert!(keep_original.validate().is_ok());

    let rewrite_without_audio = SpeechToSpeechResult {
        decision: SpeechToSpeechDecision::Rewrite,
        ..keep_original
    };
    assert!(rewrite_without_audio.validate().is_err());

    let capabilities = SpeechToSpeechWorkerCapabilities::unavailable();
    assert!(!capabilities.available);
    assert_eq!(capabilities.status, "unavailable");
    assert!(capabilities.validate().is_ok());

    let available = SpeechToSpeechWorkerCapabilities::available("hf-speech-to-speech", "local");
    assert!(available.is_ok());
    assert!(available
        .expect("capability should be available")
        .validate()
        .is_ok());
    assert!(SpeechToSpeechWorkerCapabilities::available("", "local").is_err());

    let mut blank_provider = SpeechToSpeechWorkerCapabilities::available("local", "model")
        .expect("capability should be available");
    blank_provider.provider = Some("   ".to_owned());
    assert!(blank_provider.validate().is_err());
}

#[test]
fn rewrite_result_must_respect_locked_values_length_and_target_duration() {
    let context = context();
    let result = SpeechToSpeechResult {
        decision: SpeechToSpeechDecision::Rewrite,
        text: "今天介绍这款产品".to_owned(),
        audio_path_or_stream_ref: Some("file:///tmp/variant.wav".to_owned()),
        audio_sha256: Some("c".repeat(64)),
        duration_ms: Some(1_020),
        sync_offset_ms: Some(20),
        sample_rate_hz: Some(48_000),
        channel_count: Some(2),
        latency_ms: 100,
        model: "test".to_owned(),
        fallback_reason: None,
    };
    assert!(result.validate_against(&context, 3).is_ok());

    let mut too_long = result.clone();
    too_long.text = "这是一段超过限制的改写话术".to_owned();
    assert!(too_long
        .validate_against(
            &SpeechToSpeechContext {
                max_chars: 4,
                ..context.clone()
            },
            3
        )
        .is_err());

    let mut missing_locked_value = result.clone();
    missing_locked_value.text = "今天介绍新的内容".to_owned();
    assert!(matches!(
        missing_locked_value.validate_against(&context, 3),
        Err(autolive_desktop_core::speech_to_speech::SpeechToSpeechValidationError::LockedFieldMissing)
    ));

    let mut duration_outside_target = result;
    duration_outside_target.duration_ms = Some(1_100);
    assert!(matches!(
        duration_outside_target.validate_against(&context, 3),
        Err(autolive_desktop_core::speech_to_speech::SpeechToSpeechValidationError::DurationOutsideTarget)
    ));
}
