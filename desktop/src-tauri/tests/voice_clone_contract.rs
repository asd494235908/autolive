#[path = "../src/voice_clone.rs"]
mod voice_clone;

use voice_clone::{
    locate_current_voice_segment, validate_replacement_result, validate_voice_clone_text,
    VoiceCloneError, VoiceClonePrepareRequest, VoiceClonePrepareResult,
    VoiceCloneReplacementRequest, VoiceCloneReplacementResult, VoiceCloneSegment,
    VoiceCloneSourceIndex,
};

fn sample_request() -> VoiceCloneReplacementRequest {
    VoiceCloneReplacementRequest {
        source_generation: 7,
        source_path: "/Users/mac/work/gepin/autoLive/media/source.mp4".to_owned(),
        operation_id: "operation-001".to_owned(),
        text: "替换当前话术".to_owned(),
        position_ms: 1_500,
    }
}

fn sample_result() -> VoiceCloneReplacementResult {
    VoiceCloneReplacementResult {
        source_generation: 7,
        source_path: "/Users/mac/work/gepin/autoLive/media/source.mp4".to_owned(),
        operation_id: "operation-001".to_owned(),
        audio_reference: "/Users/mac/work/gepin/autoLive/cache/voice-clone/output.wav".to_owned(),
        audio_sha256: "a".repeat(64),
        replacement_duration_ms: 900,
    }
}

#[test]
fn text_validation_trims_and_rejects_empty_or_over_500_unicode_chars() {
    assert_eq!(
        validate_voice_clone_text("  你好，话术  "),
        Ok("你好，话术".to_owned())
    );

    assert_eq!(
        validate_voice_clone_text("   "),
        Err(VoiceCloneError::EmptyText)
    );

    let long_text = "字".repeat(501);
    assert_eq!(
        validate_voice_clone_text(&long_text),
        Err(VoiceCloneError::TextTooLong {
            max_chars: 500,
            actual_chars: 501,
        })
    );
}

#[test]
fn segment_lookup_uses_half_open_current_time_boundary() {
    let first = VoiceCloneSegment {
        start_ms: 0,
        end_ms: 1_000,
        text: "第一段".to_owned(),
    };
    let second = VoiceCloneSegment {
        start_ms: 1_000,
        end_ms: 2_000,
        text: "第二段".to_owned(),
    };

    assert_eq!(
        locate_current_voice_segment(&[first.clone(), second.clone()], 999),
        Some(first.clone())
    );
    assert_eq!(
        locate_current_voice_segment(&[first.clone(), second.clone()], 1_000),
        Some(second.clone())
    );
    assert_eq!(locate_current_voice_segment(&[second], 2_000), None);
}

#[test]
fn replacement_result_rejects_stale_operation_source_or_invalid_hash() {
    let request = sample_request();

    let mut stale_generation = sample_result();
    stale_generation.source_generation = 8;
    assert_eq!(
        validate_replacement_result(&stale_generation, &request),
        Err(VoiceCloneError::SourceGenerationMismatch {
            expected: 7,
            actual: 8,
        })
    );

    let mut stale_path = sample_result();
    stale_path.source_path = "/Users/mac/work/gepin/autoLive/media/other.mp4".to_owned();
    assert_eq!(
        validate_replacement_result(&stale_path, &request),
        Err(VoiceCloneError::SourcePathMismatch {
            expected: request.source_path.clone(),
            actual: stale_path.source_path,
        })
    );

    let mut stale_operation = sample_result();
    stale_operation.operation_id = "operation-002".to_owned();
    assert_eq!(
        validate_replacement_result(&stale_operation, &request),
        Err(VoiceCloneError::OperationIdMismatch {
            expected: request.operation_id.clone(),
            actual: "operation-002".to_owned(),
        })
    );

    let mut invalid_hash = sample_result();
    invalid_hash.audio_sha256 = "not-a-sha256".to_owned();
    assert_eq!(
        validate_replacement_result(&invalid_hash, &request),
        Err(VoiceCloneError::InvalidSha256 {
            value: "not-a-sha256".to_owned(),
        })
    );
}

#[test]
fn replacement_result_accepts_current_source_and_absolute_audio_output() {
    let request = sample_request();
    let source_index = VoiceCloneSourceIndex {
        source_generation: request.source_generation,
        source_path: request.source_path.clone(),
        segments: vec![VoiceCloneSegment {
            start_ms: 1_000,
            end_ms: 2_000,
            text: "当前话术".to_owned(),
        }],
    };
    let prepare_request = VoiceClonePrepareRequest {
        source_generation: request.source_generation,
        source_path: request.source_path.clone(),
        operation_id: request.operation_id.clone(),
    };
    let prepare_result = VoiceClonePrepareResult {
        source_generation: request.source_generation,
        source_path: request.source_path.clone(),
        operation_id: request.operation_id.clone(),
        source_index: source_index.clone(),
    };
    assert_eq!(prepare_result.source_index, source_index);
    assert_eq!(prepare_request.operation_id, prepare_result.operation_id);
    assert_eq!(
        validate_replacement_result(&sample_result(), &request),
        Ok(())
    );
}
