use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;

pub const MAX_VOICE_CLONE_TEXT_CHARS: usize = 500;
pub const MIN_VOICE_SEGMENT_DURATION_MS: u64 = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceCloneSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

impl VoiceCloneSegment {
    pub fn validate(&self) -> Result<(), VoiceCloneError> {
        if self.end_ms <= self.start_ms {
            return Err(VoiceCloneError::InvalidSegmentBounds {
                start_ms: self.start_ms,
                end_ms: self.end_ms,
            });
        }

        let duration_ms = self.end_ms - self.start_ms;
        if duration_ms < MIN_VOICE_SEGMENT_DURATION_MS {
            return Err(VoiceCloneError::SegmentTooShort {
                min_duration_ms: MIN_VOICE_SEGMENT_DURATION_MS,
                actual_duration_ms: duration_ms,
            });
        }

        if self.text.trim().is_empty() {
            return Err(VoiceCloneError::BlankSegmentText);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceCloneSourceIndex {
    pub source_generation: u64,
    pub source_path: String,
    pub segments: Vec<VoiceCloneSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceClonePrepareRequest {
    pub source_generation: u64,
    pub source_path: String,
    pub operation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceClonePrepareResult {
    pub source_generation: u64,
    pub source_path: String,
    pub operation_id: String,
    pub source_index: VoiceCloneSourceIndex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceCloneReplacementRequest {
    pub source_generation: u64,
    pub source_path: String,
    pub operation_id: String,
    pub text: String,
    pub position_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceCloneReplacementResult {
    pub source_generation: u64,
    pub source_path: String,
    pub operation_id: String,
    pub audio_reference: String,
    pub audio_sha256: String,
    pub replacement_duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoiceCloneError {
    EmptyText,
    TextTooLong {
        max_chars: usize,
        actual_chars: usize,
    },
    InvalidSegmentBounds {
        start_ms: u64,
        end_ms: u64,
    },
    SegmentTooShort {
        min_duration_ms: u64,
        actual_duration_ms: u64,
    },
    BlankSegmentText,
    EmptySourcePath,
    EmptyOperationId,
    NonAbsoluteAudioReference {
        reference: String,
    },
    InvalidSha256 {
        value: String,
    },
    SourceGenerationMismatch {
        expected: u64,
        actual: u64,
    },
    SourcePathMismatch {
        expected: String,
        actual: String,
    },
    OperationIdMismatch {
        expected: String,
        actual: String,
    },
    ReplacementDurationZero,
}

impl Display for VoiceCloneError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::EmptyText => "人声克隆文本不能为空",
            Self::TextTooLong { .. } => "人声克隆文本超过最大长度",
            Self::InvalidSegmentBounds { .. } => "话术片段边界无效",
            Self::SegmentTooShort { .. } => "话术片段时长过短",
            Self::BlankSegmentText => "话术片段文本不能为空",
            Self::EmptySourcePath => "源路径不能为空",
            Self::EmptyOperationId => "操作 ID 不能为空",
            Self::NonAbsoluteAudioReference { .. } => "替换音频引用必须是绝对路径",
            Self::InvalidSha256 { .. } => "SHA-256 无效",
            Self::SourceGenerationMismatch { .. } => "源代际不匹配",
            Self::SourcePathMismatch { .. } => "源路径不匹配",
            Self::OperationIdMismatch { .. } => "操作 ID 不匹配",
            Self::ReplacementDurationZero => "替换音频时长不能为空",
        })
    }
}

impl Error for VoiceCloneError {}

pub fn validate_voice_clone_text(text: &str) -> Result<String, VoiceCloneError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(VoiceCloneError::EmptyText);
    }

    let actual_chars = trimmed.chars().count();
    if actual_chars > MAX_VOICE_CLONE_TEXT_CHARS {
        return Err(VoiceCloneError::TextTooLong {
            max_chars: MAX_VOICE_CLONE_TEXT_CHARS,
            actual_chars,
        });
    }

    Ok(trimmed.to_owned())
}

pub fn locate_current_voice_segment(
    segments: &[VoiceCloneSegment],
    position_ms: u64,
) -> Option<VoiceCloneSegment> {
    segments
        .iter()
        .find(|segment| {
            segment.validate().is_ok()
                && segment.start_ms <= position_ms
                && position_ms < segment.end_ms
        })
        .cloned()
}

pub fn validate_replacement_result(
    result: &VoiceCloneReplacementResult,
    request: &VoiceCloneReplacementRequest,
) -> Result<(), VoiceCloneError> {
    if request.source_path.trim().is_empty() {
        return Err(VoiceCloneError::EmptySourcePath);
    }
    if request.operation_id.trim().is_empty() {
        return Err(VoiceCloneError::EmptyOperationId);
    }
    if result.source_generation != request.source_generation {
        return Err(VoiceCloneError::SourceGenerationMismatch {
            expected: request.source_generation,
            actual: result.source_generation,
        });
    }
    if result.source_path != request.source_path {
        if result.source_path.trim().is_empty() {
            return Err(VoiceCloneError::EmptySourcePath);
        }
        return Err(VoiceCloneError::SourcePathMismatch {
            expected: request.source_path.clone(),
            actual: result.source_path.clone(),
        });
    }
    if result.operation_id != request.operation_id {
        if result.operation_id.trim().is_empty() {
            return Err(VoiceCloneError::EmptyOperationId);
        }
        return Err(VoiceCloneError::OperationIdMismatch {
            expected: request.operation_id.clone(),
            actual: result.operation_id.clone(),
        });
    }
    if !Path::new(&result.audio_reference).is_absolute() {
        return Err(VoiceCloneError::NonAbsoluteAudioReference {
            reference: result.audio_reference.clone(),
        });
    }
    if !is_sha256(&result.audio_sha256) {
        return Err(VoiceCloneError::InvalidSha256 {
            value: result.audio_sha256.clone(),
        });
    }
    if result.replacement_duration_ms == 0 {
        return Err(VoiceCloneError::ReplacementDurationZero);
    }

    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
