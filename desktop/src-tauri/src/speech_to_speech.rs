use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioTrackInput {
    pub track_id: String,
    pub source_kind: String,
    pub audio_path_or_stream_ref: String,
    pub audio_sha256: Option<String>,
    pub start_at_ms: u64,
    pub duration_ms: u64,
    pub sample_rate_hz: u32,
    pub channel_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeechToSpeechContext {
    pub track_id: String,
    pub source_kind: String,
    pub playback_generation: u64,
    pub loop_index: u64,
    pub segment_id: String,
    pub start_at_ms: u64,
    pub sample_rate_hz: u32,
    pub channel_count: u16,
    pub audio_path_or_stream_ref: String,
    pub transcript_text: String,
    pub previous_variant_text: Option<String>,
    pub locked_fields: Vec<String>,
    pub locked_field_values: Vec<String>,
    pub target_duration_ms: u64,
    pub max_chars: usize,
    pub language: String,
    pub rewrite_policy: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechToSpeechDecision {
    KeepOriginal,
    Rewrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeechToSpeechResult {
    pub decision: SpeechToSpeechDecision,
    pub text: String,
    pub audio_path_or_stream_ref: Option<String>,
    pub audio_sha256: Option<String>,
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub sync_offset_ms: Option<i64>,
    #[serde(default)]
    pub sample_rate_hz: Option<u32>,
    #[serde(default)]
    pub channel_count: Option<u16>,
    pub latency_ms: u64,
    pub model: String,
    pub fallback_reason: Option<String>,
}

impl SpeechToSpeechResult {
    pub fn validate(&self) -> Result<(), SpeechToSpeechValidationError> {
        if self.text.trim().is_empty() || self.model.trim().is_empty() {
            return Err(SpeechToSpeechValidationError::MissingTextOrModel);
        }
        if self.latency_ms == 0 && self.decision == SpeechToSpeechDecision::Rewrite {
            return Err(SpeechToSpeechValidationError::InvalidLatency);
        }
        match self.decision {
            SpeechToSpeechDecision::KeepOriginal => Ok(()),
            SpeechToSpeechDecision::Rewrite => {
                let valid_audio = self
                    .audio_path_or_stream_ref
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                    && self.audio_sha256.as_deref().is_some_and(is_sha256)
                    && self.duration_ms.is_some_and(|value| value > 0)
                    && self.sync_offset_ms.is_some()
                    && self.sample_rate_hz.is_some_and(|value| value > 0)
                    && self.channel_count.is_some_and(|value| value > 0);
                if valid_audio {
                    Ok(())
                } else {
                    Err(SpeechToSpeechValidationError::RewriteAudioRequired)
                }
            }
        }
    }

    pub fn validate_against(
        &self,
        context: &SpeechToSpeechContext,
        duration_tolerance_percent: u64,
    ) -> Result<(), SpeechToSpeechValidationError> {
        self.validate()?;
        if self.text.chars().count() > context.max_chars {
            return Err(SpeechToSpeechValidationError::MaxCharsExceeded);
        }
        if context.locked_fields.len() != context.locked_field_values.len()
            || context
                .locked_field_values
                .iter()
                .any(|value| value.trim().is_empty() || !self.text.contains(value))
        {
            return Err(SpeechToSpeechValidationError::LockedFieldMissing);
        }
        if let Some(duration_ms) = self.duration_ms {
            let target = context.target_duration_ms;
            let tolerance = target.saturating_mul(duration_tolerance_percent) / 100;
            let lower = target.saturating_sub(tolerance);
            let upper = target.saturating_add(tolerance);
            if duration_ms < lower || duration_ms > upper {
                return Err(SpeechToSpeechValidationError::DurationOutsideTarget);
            }
        }
        Ok(())
    }
}

impl SpeechToSpeechContext {
    pub fn validate_for_worker(&self) -> Result<(), SpeechToSpeechValidationError> {
        if self.track_id.trim().is_empty()
            || self.segment_id.trim().is_empty()
            || self.source_kind.trim().is_empty()
            || self.audio_path_or_stream_ref.trim().is_empty()
            || self.language.trim().is_empty()
            || self.rewrite_policy.trim().is_empty()
            || self.target_duration_ms == 0
            || self.max_chars == 0
            || self.sample_rate_hz == 0
            || self.channel_count == 0
            || self.timeout_ms == 0
        {
            return Err(SpeechToSpeechValidationError::InvalidContext);
        }
        if self.locked_fields.len() != self.locked_field_values.len()
            || self
                .locked_field_values
                .iter()
                .any(|value| value.trim().is_empty())
        {
            return Err(SpeechToSpeechValidationError::LockedFieldMissing);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioVariantCandidate {
    pub playback_generation: u64,
    pub loop_index: u64,
    pub segment_id: String,
    pub start_at_ms: u64,
    pub variant_mode: String,
    pub audio_path_or_stream_ref: String,
    pub audio_sha256: String,
    pub duration_ms: u64,
    pub sync_offset_ms: i64,
    pub sample_rate_hz: u32,
    pub channel_count: u16,
    pub ready: bool,
}

impl AudioVariantCandidate {
    pub fn validate_against(
        &self,
        input: &AudioTrackInput,
        context: &SpeechToSpeechContext,
        max_sync_offset_ms: i64,
    ) -> Result<(), CandidateValidationError> {
        if self.playback_generation != context.playback_generation
            || self.loop_index != context.loop_index
            || self.segment_id != context.segment_id
            || self.start_at_ms != input.start_at_ms
            || self.start_at_ms != context.start_at_ms
            || context.track_id != input.track_id
        {
            return Err(CandidateValidationError::StalePlaybackGenerationOrLoop);
        }
        if !self.ready || self.duration_ms == 0 {
            return Err(CandidateValidationError::NotReadyOrEmpty);
        }
        if self.audio_path_or_stream_ref.trim().is_empty() {
            return Err(CandidateValidationError::AudioReferenceMissing);
        }
        if self.sync_offset_ms.abs() > max_sync_offset_ms {
            return Err(CandidateValidationError::SyncOffsetExceeded);
        }
        let target_duration_ms = context.target_duration_ms;
        let duration_tolerance_ms = target_duration_ms.saturating_mul(3) / 100;
        let min_duration_ms = target_duration_ms.saturating_sub(duration_tolerance_ms);
        let max_duration_ms = target_duration_ms.saturating_add(duration_tolerance_ms);
        if self.duration_ms < min_duration_ms || self.duration_ms > max_duration_ms {
            return Err(CandidateValidationError::DurationOutsideTarget);
        }
        if self.sample_rate_hz != input.sample_rate_hz || self.channel_count != input.channel_count
        {
            return Err(CandidateValidationError::AudioFormatMismatch);
        }
        if !is_sha256(&self.audio_sha256) {
            return Err(CandidateValidationError::InvalidSHA256);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateValidationError {
    StalePlaybackGenerationOrLoop,
    NotReadyOrEmpty,
    AudioReferenceMissing,
    SyncOffsetExceeded,
    AudioFormatMismatch,
    InvalidSHA256,
    DurationOutsideTarget,
    RealtimeAudioVariantDisabled,
    WorkerAlreadyRunning,
}

impl fmt::Display for CandidateValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::StalePlaybackGenerationOrLoop => "候选音频不属于当前播放代际、轮次或片段",
            Self::NotReadyOrEmpty => "候选音频尚未就绪或时长为空",
            Self::AudioReferenceMissing => "候选音频引用不能为空",
            Self::SyncOffsetExceeded => "候选音频同步偏移超出门禁",
            Self::AudioFormatMismatch => "候选音频采样率或声道与当前音轨不一致",
            Self::InvalidSHA256 => "候选音频 SHA-256 无效",
            Self::DurationOutsideTarget => "候选音频时长超出目标时长 ±3% 门禁",
            Self::RealtimeAudioVariantDisabled => "实时话术幻化开关未开启",
            Self::WorkerAlreadyRunning => "当前播放片段已有实时话术 Worker 在执行",
        })
    }
}

impl std::error::Error for CandidateValidationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechToSpeechValidationError {
    InvalidContext,
    MissingTextOrModel,
    InvalidLatency,
    RewriteAudioRequired,
    MaxCharsExceeded,
    LockedFieldMissing,
    DurationOutsideTarget,
}

impl fmt::Display for SpeechToSpeechValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidContext => "speech-to-speech 上下文无效",
            Self::MissingTextOrModel => "speech-to-speech 结果缺少文本或模型标识",
            Self::InvalidLatency => "改写结果延迟无效",
            Self::RewriteAudioRequired => "改写结果缺少有效候选音频",
            Self::MaxCharsExceeded => "改写文本超过最大字符数",
            Self::LockedFieldMissing => "改写结果缺少锁定字段值",
            Self::DurationOutsideTarget => "改写音频时长超出目标时长门禁",
        })
    }
}

impl std::error::Error for SpeechToSpeechValidationError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeechToSpeechWorkerCapabilities {
    pub available: bool,
    pub status: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reason: Option<String>,
}

impl SpeechToSpeechWorkerCapabilities {
    #[must_use]
    pub fn unavailable() -> Self {
        Self::unavailable_with_reason("未配置本地 speech-to-speech Worker")
    }

    #[must_use]
    pub fn unavailable_with_reason(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            status: "unavailable".to_owned(),
            provider: None,
            model: None,
            reason: Some(reason.into()),
        }
    }

    pub fn available(
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Result<Self, SpeechToSpeechCapabilityError> {
        let provider = provider.into();
        let model = model.into();
        if provider.trim().is_empty() || model.trim().is_empty() {
            return Err(SpeechToSpeechCapabilityError::MissingProviderOrModel);
        }
        Ok(Self {
            available: true,
            status: "available".to_owned(),
            provider: Some(provider),
            model: Some(model),
            reason: None,
        })
    }

    pub fn validate(&self) -> Result<(), SpeechToSpeechCapabilityError> {
        if self.available {
            if self.status != "available"
                || self.provider.as_deref().is_none_or(is_blank)
                || self.model.as_deref().is_none_or(is_blank)
                || self.reason.is_some()
            {
                return Err(SpeechToSpeechCapabilityError::InvalidAvailableCapability);
            }
        } else if self.status != "unavailable"
            || self.provider.is_some()
            || self.model.is_some()
            || self.reason.as_deref().is_none_or(is_blank)
        {
            return Err(SpeechToSpeechCapabilityError::InvalidUnavailableCapability);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechToSpeechCapabilityError {
    MissingProviderOrModel,
    InvalidAvailableCapability,
    InvalidUnavailableCapability,
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_blank(value: &str) -> bool {
    value.trim().is_empty()
}
