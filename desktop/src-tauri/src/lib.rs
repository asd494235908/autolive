pub mod audio_processing;
pub mod cancellation;
pub mod direct_model;
pub mod errors;
pub mod hashing;
pub mod media_engine;
pub mod media_library;
pub mod research_params;
pub mod research_worker;
pub mod speech_to_speech;
pub mod speech_to_speech_worker;
pub mod voice_clone;
pub mod window_sizing;

use crate::audio_processing::AudioProcessingProfile;
use crate::errors::PlaybackError;
use crate::media_library::SourceMediaDto;
use crate::speech_to_speech::{
    AudioTrackInput, AudioVariantCandidate, CandidateValidationError, SpeechToSpeechContext,
};
use crate::voice_clone::{
    locate_current_voice_segment, validate_voice_clone_text, VoiceCloneError, VoiceCloneSourceIndex,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackState {
    Ready,
    Playing,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceCloneReplacementState {
    pub status: String,
    pub source_generation: Option<u64>,
    pub source_path: Option<String>,
    pub operation_id: Option<String>,
    pub replacement_audio_reference: Option<String>,
    pub replacement_audio_sha256: Option<String>,
    pub replacement_duration_ms: Option<u64>,
    pub replace_at_ms: Option<u64>,
    pub resume_at_ms: Option<u64>,
    pub input_text: Option<String>,
    pub model: Option<String>,
    pub error: Option<String>,
}

impl Default for VoiceCloneReplacementState {
    fn default() -> Self {
        Self {
            status: "idle".to_owned(),
            source_generation: None,
            source_path: None,
            operation_id: None,
            replacement_audio_reference: None,
            replacement_audio_sha256: None,
            replacement_duration_ms: None,
            replace_at_ms: None,
            resume_at_ms: None,
            input_text: None,
            model: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceClonePreparedSource {
    pub operation_id: String,
    pub source_index: VoiceCloneSourceIndex,
    pub source_sha256: String,
    pub reference_audio_path: String,
    pub reference_audio_sha256: String,
    pub sample_rate_hz: u32,
    pub channel_count: u16,
    pub total_duration_ms: u64,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceCloneReplacementPlan {
    pub source_generation: u64,
    pub source_path: String,
    pub source_sha256: String,
    pub operation_id: String,
    pub input_text: String,
    pub replace_at_ms: u64,
    pub resume_at_ms: u64,
    pub reference_audio_path: String,
    pub reference_audio_sha256: String,
    pub sample_rate_hz: u32,
    pub channel_count: u16,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceCloneCommittedReplacement {
    pub source_generation: u64,
    pub source_path: String,
    pub operation_id: String,
    pub input_text: String,
    pub replacement_audio_reference: String,
    pub replacement_audio_sha256: String,
    pub replacement_duration_ms: u64,
    pub replace_at_ms: u64,
    pub resume_at_ms: u64,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceCloneRuntimeError {
    SourceMediaRequired,
    VoiceCloneTextInvalid(VoiceCloneError),
    PlaybackNotPlaying,
    VoiceCloneSourceNotPrepared,
    VoiceClonePreparedSourceStale,
    CurrentVoiceSegmentNotFound,
    RealtimeAudioWorkerBusy,
    VoiceCloneReplacementStale,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackSnapshot {
    pub window_id: Option<String>,
    pub playback_generation: u64,
    pub playback_state: PlaybackState,
    pub source_media: Option<SourceMediaDto>,
    pub loop_index: u64,
    pub current_video_source: Option<String>,
    pub current_video_reference: Option<String>,
    pub current_video_sha256: Option<String>,
    pub pending_video_reference: Option<String>,
    pub pending_video_sha256: Option<String>,
    pub video_processing_enabled: bool,
    pub video_processing_status: String,
    pub audio_processing_enabled: bool,
    pub realtime_audio_variant_enabled: bool,
    pub current_audio_source: Option<String>,
    pub current_audio_reference: Option<String>,
    pub current_audio_start_at_ms: u64,
    pub current_mp4_sha256: Option<String>,
    pub current_audio_sha256: Option<String>,
    pub audio_decision: String,
    pub worker_status: String,
    pub fallback_reason: Option<String>,
    pub pending_audio_candidate: bool,
    pub pending_audio_reference: Option<String>,
    pub pending_audio_start_at_ms: Option<u64>,
    pub pending_audio_duration_ms: Option<u64>,
    pub audio_processing_parameters_version: String,
    pub audio_processing_status: String,
    pub audio_processing_runtime: bool,
    pub audio_processing_gain_db: f64,
    pub voice_clone_replacement: VoiceCloneReplacementState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackCore {
    window_id: Option<String>,
    playback_generation: u64,
    playback_state: PlaybackState,
    source_media: Option<SourceMediaDto>,
    loop_index: u64,
    current_video_source: Option<String>,
    current_video_reference: Option<String>,
    current_video_sha256: Option<String>,
    pending_video_reference: Option<String>,
    pending_video_sha256: Option<String>,
    video_processing_enabled: bool,
    video_processing_status: String,
    audio_processing_enabled: bool,
    realtime_audio_variant_enabled: bool,
    current_audio_source: Option<String>,
    current_audio_reference: Option<String>,
    current_audio_start_at_ms: u64,
    current_audio_sha256: Option<String>,
    audio_decision: String,
    worker_status: String,
    fallback_reason: Option<String>,
    pending_audio_candidate: Option<AudioVariantCandidate>,
    audio_processing_profile: AudioProcessingProfile,
    audio_processing_status: String,
    voice_clone_prepared_source: Option<VoiceClonePreparedSource>,
    voice_clone_replacement: VoiceCloneReplacementState,
}

impl Default for PlaybackCore {
    fn default() -> Self {
        Self {
            window_id: None,
            playback_generation: 0,
            playback_state: PlaybackState::Ready,
            source_media: None,
            loop_index: 0,
            current_video_source: None,
            current_video_reference: None,
            current_video_sha256: None,
            pending_video_reference: None,
            pending_video_sha256: None,
            video_processing_enabled: false,
            video_processing_status: "disabled".to_owned(),
            audio_processing_enabled: false,
            realtime_audio_variant_enabled: false,
            current_audio_source: None,
            current_audio_reference: None,
            current_audio_start_at_ms: 0,
            current_audio_sha256: None,
            audio_decision: "keep_original".to_owned(),
            worker_status: "unavailable".to_owned(),
            fallback_reason: None,
            pending_audio_candidate: None,
            audio_processing_profile: AudioProcessingProfile::default(),
            audio_processing_status: "disabled".to_owned(),
            voice_clone_prepared_source: None,
            voice_clone_replacement: VoiceCloneReplacementState::default(),
        }
    }
}

impl PlaybackCore {
    pub fn bind_window(&mut self, window_id: &str) -> Result<(), PlaybackError> {
        let window_id = window_id.trim();
        if window_id.is_empty() {
            return Err(PlaybackError::EmptyWindowId);
        }
        match &self.window_id {
            Some(existing) if existing != window_id => Err(PlaybackError::WindowAlreadyBound {
                existing: existing.clone(),
                attempted: window_id.to_owned(),
            }),
            Some(_) => Ok(()),
            None => {
                self.window_id = Some(window_id.to_owned());
                Ok(())
            }
        }
    }

    pub fn set_source(&mut self, mut source_media: SourceMediaDto) {
        source_media.mp4_sha256 = None;
        source_media.mp4_hash_status = "pending".to_owned();
        self.source_media = Some(source_media);
        self.loop_index = 0;
        self.playback_generation = self.playback_generation.wrapping_add(1);
        self.playback_state = PlaybackState::Ready;
        self.reset_audio_to_original();
        self.reset_video_to_original();
        self.pending_video_reference = None;
        self.pending_video_sha256 = None;
        self.worker_status = "unavailable".to_owned();
        self.fallback_reason = None;
        self.pending_audio_candidate = None;
        self.audio_processing_status = self.audio_processing_status_for("unavailable");
        self.video_processing_status = if self.video_processing_enabled {
            "unavailable".to_owned()
        } else {
            "disabled".to_owned()
        };
        self.clear_voice_clone_for_new_generation();
    }

    pub fn mark_voice_clone_preparing(
        &mut self,
        operation_id: &str,
    ) -> Result<(), VoiceCloneRuntimeError> {
        let source = self
            .source_media
            .as_ref()
            .ok_or(VoiceCloneRuntimeError::SourceMediaRequired)?;
        let operation_id = operation_id.trim();
        if operation_id.is_empty() {
            return Err(VoiceCloneRuntimeError::VoiceCloneTextInvalid(
                VoiceCloneError::EmptyOperationId,
            ));
        }
        self.voice_clone_prepared_source = None;
        self.voice_clone_replacement = VoiceCloneReplacementState {
            status: "preparing".to_owned(),
            source_generation: Some(self.playback_generation),
            source_path: Some(source.source_path.clone()),
            operation_id: Some(operation_id.to_owned()),
            replacement_audio_reference: None,
            replacement_audio_sha256: None,
            replacement_duration_ms: None,
            replace_at_ms: None,
            resume_at_ms: None,
            input_text: None,
            model: None,
            error: None,
        };
        Ok(())
    }

    pub fn set_voice_clone_prepared_source(
        &mut self,
        prepared: VoiceClonePreparedSource,
    ) -> Result<(), VoiceCloneRuntimeError> {
        let source = self
            .source_media
            .as_ref()
            .ok_or(VoiceCloneRuntimeError::SourceMediaRequired)?;
        if prepared.source_index.source_generation != self.playback_generation
            || prepared.source_index.source_path != source.source_path
        {
            return Err(VoiceCloneRuntimeError::VoiceClonePreparedSourceStale);
        }
        if prepared.operation_id.trim().is_empty()
            || prepared.source_sha256.len() != 64
            || !prepared
                .source_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || prepared.reference_audio_sha256.len() != 64
            || !prepared
                .reference_audio_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(VoiceCloneRuntimeError::VoiceClonePreparedSourceStale);
        }
        for segment in &prepared.source_index.segments {
            segment
                .validate()
                .map_err(VoiceCloneRuntimeError::VoiceCloneTextInvalid)?;
        }

        self.voice_clone_prepared_source = Some(prepared.clone());
        self.voice_clone_replacement = VoiceCloneReplacementState {
            status: "ready".to_owned(),
            source_generation: Some(prepared.source_index.source_generation),
            source_path: Some(prepared.source_index.source_path.clone()),
            operation_id: Some(prepared.operation_id),
            replacement_audio_reference: None,
            replacement_audio_sha256: None,
            replacement_duration_ms: None,
            replace_at_ms: None,
            resume_at_ms: None,
            input_text: None,
            model: prepared.model,
            error: None,
        };
        Ok(())
    }

    pub fn start_voice_clone_replacement(
        &mut self,
        text: &str,
        position_ms: u64,
        operation_id: &str,
        realtime_worker_occupied: bool,
    ) -> Result<VoiceCloneReplacementPlan, VoiceCloneRuntimeError> {
        if realtime_worker_occupied {
            return Err(VoiceCloneRuntimeError::RealtimeAudioWorkerBusy);
        }
        if self.playback_state != PlaybackState::Playing {
            return Err(VoiceCloneRuntimeError::PlaybackNotPlaying);
        }
        let source = self
            .source_media
            .as_ref()
            .ok_or(VoiceCloneRuntimeError::SourceMediaRequired)?;
        let prepared = self
            .voice_clone_prepared_source
            .clone()
            .ok_or(VoiceCloneRuntimeError::VoiceCloneSourceNotPrepared)?;
        if prepared.source_index.source_generation != self.playback_generation
            || prepared.source_index.source_path != source.source_path
        {
            return Err(VoiceCloneRuntimeError::VoiceClonePreparedSourceStale);
        }
        let input_text = validate_voice_clone_text(text)
            .map_err(VoiceCloneRuntimeError::VoiceCloneTextInvalid)?;
        let current_segment =
            locate_current_voice_segment(&prepared.source_index.segments, position_ms)
                .ok_or(VoiceCloneRuntimeError::CurrentVoiceSegmentNotFound)?;
        let operation_id = operation_id.trim();
        if operation_id.is_empty() {
            return Err(VoiceCloneRuntimeError::VoiceCloneTextInvalid(
                VoiceCloneError::EmptyOperationId,
            ));
        }
        self.voice_clone_replacement = VoiceCloneReplacementState {
            status: "generating".to_owned(),
            source_generation: Some(self.playback_generation),
            source_path: Some(source.source_path.clone()),
            operation_id: Some(operation_id.to_owned()),
            replacement_audio_reference: None,
            replacement_audio_sha256: None,
            replacement_duration_ms: None,
            replace_at_ms: Some(position_ms),
            resume_at_ms: Some(current_segment.end_ms),
            input_text: Some(input_text.clone()),
            model: prepared.model.clone(),
            error: None,
        };
        Ok(VoiceCloneReplacementPlan {
            source_generation: self.playback_generation,
            source_path: source.source_path.clone(),
            source_sha256: prepared.source_sha256,
            operation_id: operation_id.to_owned(),
            input_text,
            replace_at_ms: position_ms,
            resume_at_ms: current_segment.end_ms,
            reference_audio_path: prepared.reference_audio_path,
            reference_audio_sha256: prepared.reference_audio_sha256,
            sample_rate_hz: prepared.sample_rate_hz,
            channel_count: prepared.channel_count,
            model: prepared.model,
        })
    }

    pub fn apply_voice_clone_replacement(
        &mut self,
        replacement: VoiceCloneCommittedReplacement,
    ) -> Result<(), VoiceCloneRuntimeError> {
        let source = self
            .source_media
            .as_ref()
            .ok_or(VoiceCloneRuntimeError::SourceMediaRequired)?;
        if replacement.source_generation != self.playback_generation
            || replacement.source_path != source.source_path
            || self.voice_clone_replacement.operation_id.as_deref()
                != Some(replacement.operation_id.as_str())
        {
            return Err(VoiceCloneRuntimeError::VoiceCloneReplacementStale);
        }
        self.voice_clone_replacement = VoiceCloneReplacementState {
            status: "playing".to_owned(),
            source_generation: Some(replacement.source_generation),
            source_path: Some(replacement.source_path),
            operation_id: Some(replacement.operation_id),
            replacement_audio_reference: Some(replacement.replacement_audio_reference),
            replacement_audio_sha256: Some(replacement.replacement_audio_sha256),
            replacement_duration_ms: Some(replacement.replacement_duration_ms),
            replace_at_ms: Some(replacement.replace_at_ms),
            resume_at_ms: Some(replacement.resume_at_ms),
            input_text: Some(replacement.input_text),
            model: replacement.model,
            error: None,
        };
        Ok(())
    }

    pub fn mark_voice_clone_failed(&mut self, reason: impl Into<String>) {
        self.voice_clone_replacement.status = "failed".to_owned();
        self.voice_clone_replacement.error = Some(reason.into());
        self.voice_clone_replacement.replacement_audio_reference = None;
        self.voice_clone_replacement.replacement_audio_sha256 = None;
        self.voice_clone_replacement.replacement_duration_ms = None;
    }

    pub fn mark_voice_clone_cancelled(&mut self, reason: impl Into<String>) {
        let next_status = if self.voice_clone_prepared_source.is_some() {
            "ready"
        } else {
            "cancelled"
        };
        self.voice_clone_replacement.status = next_status.to_owned();
        self.voice_clone_replacement.error = Some(reason.into());
        self.voice_clone_replacement.replacement_audio_reference = None;
        self.voice_clone_replacement.replacement_audio_sha256 = None;
        self.voice_clone_replacement.replacement_duration_ms = None;
    }

    pub fn clear_voice_clone_replacement(&mut self) {
        self.reset_voice_clone_for_current_source();
    }

    pub fn voice_clone_prepared_source(&self) -> Option<VoiceClonePreparedSource> {
        self.voice_clone_prepared_source.clone()
    }

    pub fn mark_media_processing_running(&mut self) -> Result<(), PlaybackError> {
        self.require_source()?;
        if !self.video_processing_enabled && !self.audio_processing_enabled {
            return Ok(());
        }
        self.video_processing_status = if self.video_processing_enabled {
            "processing".to_owned()
        } else {
            "disabled".to_owned()
        };
        self.audio_processing_status = self.audio_processing_status_for("processing");
        self.pending_video_reference = None;
        self.pending_video_sha256 = None;
        self.fallback_reason = None;
        Ok(())
    }

    pub fn mark_media_processing_ready(
        &mut self,
        generation: u64,
        output_path: String,
        output_sha256: String,
    ) -> Result<(), PlaybackError> {
        if self.playback_generation != generation {
            return Err(PlaybackError::StaleMediaProcessing);
        }
        if output_path.trim().is_empty()
            || output_sha256.len() != 64
            || !output_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(PlaybackError::InvalidMediaProcessingOutput);
        }
        self.pending_video_reference = Some(output_path);
        self.pending_video_sha256 = Some(output_sha256);
        self.video_processing_status = if self.video_processing_enabled {
            "ready".to_owned()
        } else {
            "disabled".to_owned()
        };
        self.audio_processing_status = self.audio_processing_status_for("ready");
        self.fallback_reason = None;
        Ok(())
    }

    pub fn mark_media_processing_failed(&mut self, reason: impl Into<String>) {
        self.pending_video_reference = None;
        self.pending_video_sha256 = None;
        self.video_processing_status = if self.video_processing_enabled {
            "failed".to_owned()
        } else {
            "disabled".to_owned()
        };
        self.audio_processing_status = self.audio_processing_status_for("failed");
        self.fallback_reason = Some(reason.into());
    }

    pub fn mark_audio_processing_unavailable(&mut self, reason: impl Into<String>) {
        self.audio_processing_status = if self.audio_processing_enabled {
            "unavailable".to_owned()
        } else {
            "disabled".to_owned()
        };
        self.fallback_reason = Some(reason.into());
    }

    pub fn commit_media_processing_if_ready(&mut self) -> bool {
        if self.pending_video_reference.is_none() || self.pending_video_sha256.is_none() {
            return false;
        }
        self.commit_pending_video();
        true
    }

    pub fn set_processing_switches(
        &mut self,
        video_processing_enabled: bool,
        audio_processing_enabled: bool,
        realtime_audio_variant_enabled: bool,
    ) {
        let video_was_enabled = self.video_processing_enabled;
        let realtime_was_enabled = self.realtime_audio_variant_enabled;
        self.video_processing_enabled = video_processing_enabled;
        self.audio_processing_enabled = audio_processing_enabled;
        self.realtime_audio_variant_enabled = realtime_audio_variant_enabled;
        self.audio_processing_status = self.audio_processing_status_for("unavailable");
        self.video_processing_status = if video_processing_enabled {
            "unavailable".to_owned()
        } else {
            "disabled".to_owned()
        };
        if !video_processing_enabled || !video_was_enabled {
            self.reset_video_to_original();
            self.pending_video_reference = None;
            self.pending_video_sha256 = None;
        }
        // 实时候选出现后，普通声音处理必须运行在当前最终音轨上。
        // 已经把普通声音写入媒体缓存的旧视频不能继续作为基础源，否则会在
        // 候选音轨上产生双重处理；切换实时开关时回到源视频，等待新的安全缓存。
        if audio_processing_enabled && realtime_was_enabled != realtime_audio_variant_enabled {
            self.reset_video_to_original();
            self.pending_video_reference = None;
            self.pending_video_sha256 = None;
            self.video_processing_status = if video_processing_enabled {
                "unavailable".to_owned()
            } else {
                "disabled".to_owned()
            };
        }
        if realtime_audio_variant_enabled {
            if self.current_audio_source.is_none() {
                self.current_audio_source = Some("original".to_owned());
            }
            if self.current_audio_reference.is_none() {
                self.current_audio_reference = self
                    .source_media
                    .as_ref()
                    .map(|source| source.file_name.clone());
            }
            self.worker_status = "unavailable".to_owned();
        } else {
            self.current_audio_source = self.source_media.as_ref().map(|_| "original".to_owned());
            self.current_audio_reference = self
                .source_media
                .as_ref()
                .map(|source| source.file_name.clone());
            self.current_audio_sha256 = None;
            self.audio_decision = "keep_original".to_owned();
            self.worker_status = "unavailable".to_owned();
            self.fallback_reason = None;
            self.pending_audio_candidate = None;
        }
    }

    pub fn set_audio_processing_profile(
        &mut self,
        profile: AudioProcessingProfile,
    ) -> Result<(), Vec<crate::research_params::ParameterValidationError>> {
        profile.validate()?;
        self.audio_processing_profile = profile;
        self.audio_processing_status = self.audio_processing_status_for("unavailable");
        Ok(())
    }

    pub fn set_mp4_sha256_for_generation(&mut self, generation: u64, mp4_sha256: String) {
        if self.playback_generation != generation {
            return;
        }
        if let Some(source_media) = self.source_media.as_mut() {
            source_media.mp4_sha256 = Some(mp4_sha256);
            source_media.mp4_hash_status = "ready".to_owned();
        }
    }

    pub fn set_mp4_hash_failed_for_generation(&mut self, generation: u64) {
        if self.playback_generation != generation {
            return;
        }
        if let Some(source_media) = self.source_media.as_mut() {
            source_media.mp4_sha256 = None;
            source_media.mp4_hash_status = "failed".to_owned();
        }
    }

    pub fn stage_audio_variant_candidate(
        &mut self,
        input: &AudioTrackInput,
        context: &SpeechToSpeechContext,
        candidate: AudioVariantCandidate,
        max_sync_offset_ms: i64,
    ) -> Result<(), CandidateValidationError> {
        if self.playback_generation != context.playback_generation
            || self.loop_index != context.loop_index
        {
            return Err(CandidateValidationError::StalePlaybackGenerationOrLoop);
        }
        candidate.validate_against(input, context, max_sync_offset_ms)?;
        self.pending_audio_candidate = Some(candidate);
        self.worker_status = "ready".to_owned();
        self.fallback_reason = None;
        Ok(())
    }

    pub fn mark_speech_to_speech_worker_running(&mut self) -> Result<(), CandidateValidationError> {
        if !self.realtime_audio_variant_enabled {
            return Err(CandidateValidationError::RealtimeAudioVariantDisabled);
        }
        if self.worker_status == "running" {
            return Err(CandidateValidationError::WorkerAlreadyRunning);
        }
        self.worker_status = "running".to_owned();
        self.fallback_reason = None;
        Ok(())
    }

    pub fn mark_speech_to_speech_worker_cancelled(&mut self, reason: impl Into<String>) {
        self.worker_status = "cancelled".to_owned();
        self.fallback_reason = Some(reason.into());
    }

    pub fn finish_speech_to_speech_keep_original(
        &mut self,
        playback_generation: u64,
        loop_index: u64,
    ) -> Result<(), CandidateValidationError> {
        if self.playback_generation != playback_generation || self.loop_index != loop_index {
            return Err(CandidateValidationError::StalePlaybackGenerationOrLoop);
        }
        self.reset_audio_to_original();
        self.pending_audio_candidate = None;
        self.worker_status = "ready".to_owned();
        self.audio_decision = "keep_original".to_owned();
        self.fallback_reason = None;
        Ok(())
    }

    pub fn commit_audio_variant_candidate(
        &mut self,
        playback_generation: u64,
        loop_index: u64,
        segment_id: &str,
    ) -> Result<(), CandidateValidationError> {
        let candidate = self
            .pending_audio_candidate
            .as_ref()
            .ok_or(CandidateValidationError::NotReadyOrEmpty)?;
        if candidate.playback_generation != playback_generation
            || candidate.loop_index != loop_index
            || candidate.segment_id != segment_id
            || self.playback_generation != playback_generation
            || self.loop_index != loop_index
        {
            return Err(CandidateValidationError::StalePlaybackGenerationOrLoop);
        }
        let candidate = self
            .pending_audio_candidate
            .take()
            .ok_or(CandidateValidationError::NotReadyOrEmpty)?;
        self.current_audio_source = Some("realtime_variant".to_owned());
        self.current_audio_reference = Some(candidate.audio_path_or_stream_ref);
        self.current_audio_start_at_ms = candidate.start_at_ms;
        self.current_audio_sha256 = Some(candidate.audio_sha256);
        self.audio_decision = "rewrite".to_owned();
        self.worker_status = "ready".to_owned();
        self.fallback_reason = None;
        Ok(())
    }

    pub fn commit_audio_variant_candidate_if_due(
        &mut self,
        position_ms: u64,
    ) -> Result<bool, CandidateValidationError> {
        let Some(candidate) = self.pending_audio_candidate.as_ref() else {
            return Ok(false);
        };
        if position_ms < candidate.start_at_ms {
            return Ok(false);
        }
        let generation = candidate.playback_generation;
        let loop_index = candidate.loop_index;
        let segment_id = candidate.segment_id.clone();
        self.commit_audio_variant_candidate(generation, loop_index, &segment_id)?;
        Ok(true)
    }

    pub fn discard_audio_variant_candidate(&mut self, reason: impl Into<String>) {
        self.pending_audio_candidate = None;
        self.worker_status = "failed".to_owned();
        self.audio_decision = "fallback_original".to_owned();
        self.fallback_reason = Some(reason.into());
    }

    pub fn fallback_audio_runtime(&mut self, reason: impl Into<String>) {
        self.current_audio_source = self.source_media.as_ref().map(|_| "original".to_owned());
        self.current_audio_reference = self
            .source_media
            .as_ref()
            .map(|source| source.file_name.clone());
        self.current_audio_start_at_ms = 0;
        self.current_audio_sha256 = None;
        self.audio_decision = "fallback_original".to_owned();
        self.worker_status = "failed".to_owned();
        self.fallback_reason = Some(reason.into());
    }

    pub fn start(&mut self) -> Result<(), PlaybackError> {
        self.require_source()?;
        self.playback_state = PlaybackState::Playing;
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), PlaybackError> {
        if self.playback_state != PlaybackState::Playing {
            return Err(PlaybackError::InvalidTransition {
                from: self.playback_state,
                to: PlaybackState::Paused,
            });
        }
        self.playback_state = PlaybackState::Paused;
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), PlaybackError> {
        if !matches!(
            self.playback_state,
            PlaybackState::Paused | PlaybackState::Ready
        ) {
            return Err(PlaybackError::InvalidTransition {
                from: self.playback_state,
                to: PlaybackState::Playing,
            });
        }
        self.require_source()?;
        self.playback_state = PlaybackState::Playing;
        Ok(())
    }

    pub fn stop(&mut self) {
        self.playback_state = PlaybackState::Stopped;
        self.playback_generation = self.playback_generation.wrapping_add(1);
        self.pending_audio_candidate = None;
        self.reset_audio_to_original();
        self.reset_video_to_original();
        self.pending_video_reference = None;
        self.pending_video_sha256 = None;
        self.clear_voice_clone_for_new_generation();
    }

    pub fn complete_loop(&mut self) -> Result<(), PlaybackError> {
        self.require_source()?;
        self.loop_index = self.loop_index.saturating_add(1);
        self.playback_state = PlaybackState::Playing;
        self.pending_audio_candidate = None;
        self.reset_audio_to_original();
        self.commit_pending_video();
        self.reset_voice_clone_for_current_source();
        Ok(())
    }

    #[must_use]
    pub fn snapshot(&self) -> PlaybackSnapshot {
        PlaybackSnapshot {
            window_id: self.window_id.clone(),
            playback_generation: self.playback_generation,
            playback_state: self.playback_state,
            source_media: self.source_media.clone(),
            loop_index: self.loop_index,
            current_video_source: self.current_video_source.clone(),
            current_video_reference: self.current_video_reference.clone(),
            current_video_sha256: self.current_video_sha256.clone(),
            pending_video_reference: self.pending_video_reference.clone(),
            pending_video_sha256: self.pending_video_sha256.clone(),
            video_processing_enabled: self.video_processing_enabled,
            video_processing_status: self.video_processing_status.clone(),
            audio_processing_enabled: self.audio_processing_enabled,
            realtime_audio_variant_enabled: self.realtime_audio_variant_enabled,
            current_audio_source: self.current_audio_source.clone(),
            current_audio_reference: self.current_audio_reference.clone(),
            current_audio_start_at_ms: self.current_audio_start_at_ms,
            current_mp4_sha256: self.current_video_sha256.clone().or_else(|| {
                self.source_media
                    .as_ref()
                    .and_then(|source| source.mp4_sha256.clone())
            }),
            current_audio_sha256: self.current_audio_sha256.clone(),
            audio_decision: self.audio_decision.clone(),
            worker_status: self.worker_status.clone(),
            fallback_reason: self.fallback_reason.clone(),
            pending_audio_candidate: self.pending_audio_candidate.is_some(),
            pending_audio_reference: self
                .pending_audio_candidate
                .as_ref()
                .map(|candidate| candidate.audio_path_or_stream_ref.clone()),
            pending_audio_start_at_ms: self
                .pending_audio_candidate
                .as_ref()
                .map(|candidate| candidate.start_at_ms),
            pending_audio_duration_ms: self
                .pending_audio_candidate
                .as_ref()
                .map(|candidate| candidate.duration_ms),
            audio_processing_parameters_version: self
                .audio_processing_profile
                .parameters_version
                .clone(),
            audio_processing_status: self.audio_processing_status.clone(),
            audio_processing_runtime: self.audio_processing_status == "runtime",
            audio_processing_gain_db: self.audio_processing_profile.params.input_gain_db
                + self.audio_processing_profile.params.output_gain_db
                + self.audio_processing_profile.params.loudness_adjustment_db,
            voice_clone_replacement: self.voice_clone_replacement.clone(),
        }
    }

    fn require_source(&self) -> Result<(), PlaybackError> {
        self.source_media
            .as_ref()
            .map(|_| ())
            .ok_or(PlaybackError::SourceMediaRequired)
    }

    fn audio_processing_status_for(&self, ordinary_status: &str) -> String {
        if !self.audio_processing_enabled {
            return "disabled".to_owned();
        }
        if self.realtime_audio_variant_enabled {
            if self.realtime_audio_processing_supported() {
                return "runtime".to_owned();
            }
            return "unavailable".to_owned();
        }
        ordinary_status.to_owned()
    }

    fn realtime_audio_processing_supported(&self) -> bool {
        let params = &self.audio_processing_profile.params;
        let defaults = crate::research_params::AudioResearchParams::default();
        params.natural_voice_mode == defaults.natural_voice_mode
            && params.random_change_period_ms == defaults.random_change_period_ms
            && params.pitch_shift_semitones.abs() <= f64::EPSILON
            && params.spectral_perturbation_percent.abs() <= f64::EPSILON
            && params.environment_noise_percent.abs() <= f64::EPSILON
            && params.environment_noise_dbfs == defaults.environment_noise_dbfs
            && params.mfcc_shift_percent.abs() <= f64::EPSILON
            && params.phase_perturbation_percent.abs() <= f64::EPSILON
            && params.dry_wet_percent.abs() <= f64::EPSILON
            && params.reverb_wet_percent.abs() <= f64::EPSILON
            && params.mfcc_dimensions == defaults.mfcc_dimensions
            && params.snr_variation_db.abs() <= f64::EPSILON
            && params.formant_shift_percent.abs() <= f64::EPSILON
            && params.vibrato_frequency_hz == defaults.vibrato_frequency_hz
            && params.vibrato_depth_percent.abs() <= f64::EPSILON
            && params.spectrum_blind_spot_percent.abs() <= f64::EPSILON
            && params.snr_target_db == defaults.snr_target_db
            && params.current_formant_hz == defaults.current_formant_hz
            && params.filter_q == defaults.filter_q
            && params.sample_rate_hz == defaults.sample_rate_hz
            && params.output_bitrate_kbps == defaults.output_bitrate_kbps
            && params.voice_library_id == defaults.voice_library_id
    }

    pub fn restore_original_audio(&mut self) {
        self.reset_audio_to_original();
        self.pending_audio_candidate = None;
        self.worker_status = if self.realtime_audio_variant_enabled {
            "ready".to_owned()
        } else {
            "unavailable".to_owned()
        };
        self.fallback_reason = None;
    }

    fn reset_audio_to_original(&mut self) {
        self.current_audio_source = self.source_media.as_ref().map(|_| "original".to_owned());
        self.current_audio_reference = self
            .source_media
            .as_ref()
            .map(|source| source.file_name.clone());
        self.current_audio_start_at_ms = 0;
        self.current_audio_sha256 = None;
        self.audio_decision = "keep_original".to_owned();
    }

    fn reset_video_to_original(&mut self) {
        self.current_video_source = self.source_media.as_ref().map(|_| "original".to_owned());
        self.current_video_reference = self
            .source_media
            .as_ref()
            .map(|source| source.source_path.clone());
        self.current_video_sha256 = None;
    }

    fn commit_pending_video(&mut self) {
        let (Some(reference), Some(sha256)) = (
            self.pending_video_reference.take(),
            self.pending_video_sha256.take(),
        ) else {
            return;
        };
        self.current_video_source = Some("processed".to_owned());
        self.current_video_reference = Some(reference);
        self.current_video_sha256 = Some(sha256);
    }

    fn reset_voice_clone_for_current_source(&mut self) {
        let mut state = VoiceCloneReplacementState::default();
        if let Some(source) = self.source_media.as_ref() {
            state.source_generation = Some(self.playback_generation);
            state.source_path = Some(source.source_path.clone());
        }
        if let Some(prepared) = self.voice_clone_prepared_source.as_ref() {
            state.status = "ready".to_owned();
            state.operation_id = Some(prepared.operation_id.clone());
            state.model = prepared.model.clone();
        }
        self.voice_clone_replacement = state;
    }

    fn clear_voice_clone_for_new_generation(&mut self) {
        self.voice_clone_prepared_source = None;
        self.voice_clone_replacement = VoiceCloneReplacementState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::{PlaybackCore, PlaybackState};
    use crate::audio_processing::AudioProcessingProfile;
    use crate::media_library::SourceMediaDto;
    use crate::speech_to_speech::{AudioTrackInput, AudioVariantCandidate, SpeechToSpeechContext};

    fn source() -> SourceMediaDto {
        SourceMediaDto {
            source_path: "/tmp/source.mp4".to_owned(),
            file_name: "source.mp4".to_owned(),
            file_size_bytes: 1,
            duration_ms: Some(1_000),
            width: Some(1280),
            height: Some(720),
            frame_rate_fps: Some(30.0),
            audio_sample_rate_hz: Some(48_000),
            audio_channel_count: Some(2),
            mp4_sha256: Some("a".repeat(64)),
            mp4_hash_status: "ready".to_owned(),
        }
    }

    #[test]
    fn single_source_loops_without_queue_versions() {
        let mut core = PlaybackCore::default();
        core.bind_window("main").expect("window should bind");
        core.set_source(source());
        core.start().expect("source should start");
        core.complete_loop().expect("loop should complete");

        let snapshot = core.snapshot();
        assert_eq!(snapshot.playback_state, PlaybackState::Playing);
        assert_eq!(snapshot.loop_index, 1);
        assert_eq!(
            snapshot.source_media.expect("source").file_name,
            "source.mp4"
        );
    }

    #[test]
    fn processing_switches_are_part_of_single_playback_snapshot() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        let generation = core.snapshot().playback_generation;
        core.set_mp4_sha256_for_generation(generation, "a".repeat(64));
        core.set_processing_switches(true, true, true);
        let snapshot = core.snapshot();
        assert!(snapshot.video_processing_enabled);
        assert_eq!(snapshot.video_processing_status, "unavailable");
        assert!(snapshot.audio_processing_enabled);
        assert_eq!(snapshot.audio_processing_status, "runtime");
        assert!(snapshot.realtime_audio_variant_enabled);
        assert_eq!(snapshot.current_audio_source.as_deref(), Some("original"));
        assert_eq!(snapshot.audio_decision, "keep_original");
        assert_eq!(snapshot.worker_status, "unavailable");
        assert_eq!(snapshot.current_audio_sha256, None);
        assert_eq!(
            snapshot.current_mp4_sha256.as_deref(),
            Some("a".repeat(64).as_str())
        );
    }

    #[test]
    fn audio_processing_profile_is_versioned_without_faking_dsp_execution() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(false, true, false);

        let profile = AudioProcessingProfile {
            parameters_version: "audio_processing_v2".to_owned(),
            params: Default::default(),
        };
        core.set_audio_processing_profile(profile)
            .expect("valid audio profile should be accepted");

        let snapshot = core.snapshot();
        assert_eq!(
            snapshot.audio_processing_parameters_version,
            "audio_processing_v2"
        );
        assert_eq!(snapshot.audio_processing_status, "unavailable");

        let invalid_profile = AudioProcessingProfile {
            parameters_version: String::new(),
            params: Default::default(),
        };
        assert!(core.set_audio_processing_profile(invalid_profile).is_err());
        assert_eq!(
            core.snapshot().audio_processing_parameters_version,
            "audio_processing_v2"
        );
    }

    #[test]
    fn disabling_realtime_audio_keeps_the_same_source_and_falls_back_to_original() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(false, false, true);
        core.fallback_audio_runtime("candidate unavailable");
        core.set_processing_switches(false, false, false);

        let snapshot = core.snapshot();
        assert!(!snapshot.realtime_audio_variant_enabled);
        assert_eq!(snapshot.current_audio_source.as_deref(), Some("original"));
        assert_eq!(snapshot.audio_decision, "keep_original");
        assert_eq!(snapshot.worker_status, "unavailable");
        assert_eq!(snapshot.current_audio_sha256, None);
    }

    #[test]
    fn media_processing_ready_switches_current_video_without_losing_source() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(true, true, false);
        let generation = core.snapshot().playback_generation;

        core.mark_media_processing_running()
            .expect("media processing should start");
        assert_eq!(core.snapshot().video_processing_status, "processing");
        assert_eq!(
            core.snapshot().current_video_source.as_deref(),
            Some("original")
        );

        core.mark_media_processing_ready(
            generation,
            "/tmp/processed.mp4".to_owned(),
            "b".repeat(64),
        )
        .expect("processed media should be accepted");
        let snapshot = core.snapshot();
        assert_eq!(snapshot.video_processing_status, "ready");
        assert_eq!(snapshot.audio_processing_status, "ready");
        assert!(snapshot.pending_video_reference.is_some());
        assert!(core.commit_media_processing_if_ready());
        let snapshot = core.snapshot();
        assert_eq!(snapshot.current_video_source.as_deref(), Some("processed"));
        assert_eq!(
            snapshot.current_video_reference.as_deref(),
            Some("/tmp/processed.mp4")
        );
        assert_eq!(
            snapshot.current_mp4_sha256.as_deref(),
            Some("b".repeat(64).as_str())
        );
        assert_eq!(
            snapshot.source_media.expect("source").source_path,
            "/tmp/source.mp4"
        );
    }

    #[test]
    fn realtime_audio_processing_only_reports_runtime_for_supported_gain_path() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(false, true, true);
        let snapshot = core.snapshot();
        assert_eq!(snapshot.audio_processing_status, "runtime");
        assert!(snapshot.audio_processing_runtime);

        let mut profile = AudioProcessingProfile::default();
        profile.params.pitch_shift_semitones = 0.5;
        core.set_audio_processing_profile(profile)
            .expect("validated profile should be accepted");
        let snapshot = core.snapshot();
        assert_eq!(snapshot.audio_processing_status, "unavailable");
        assert!(!snapshot.audio_processing_runtime);
    }

    #[test]
    fn media_processing_failure_keeps_original_video_reference() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(true, false, false);
        core.mark_media_processing_running()
            .expect("media processing should start");
        core.mark_media_processing_failed("ffmpeg unavailable");

        let snapshot = core.snapshot();
        assert_eq!(snapshot.video_processing_status, "failed");
        assert_eq!(snapshot.current_video_source.as_deref(), Some("original"));
        assert_eq!(
            snapshot.current_video_reference.as_deref(),
            Some("/tmp/source.mp4")
        );
        assert_eq!(
            snapshot.fallback_reason.as_deref(),
            Some("ffmpeg unavailable")
        );
    }

    #[test]
    fn stale_background_hash_cannot_update_a_new_source_generation() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        let old_generation = core.snapshot().playback_generation;
        core.set_source(SourceMediaDto {
            file_name: "new-source.mp4".to_owned(),
            ..source()
        });
        core.set_mp4_sha256_for_generation(old_generation, "b".repeat(64));
        assert_eq!(
            core.snapshot().source_media.expect("source").mp4_sha256,
            None
        );
    }

    #[test]
    fn playback_core_stages_then_commits_audio_candidate_without_replacing_active_audio_early() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(false, false, true);
        let input = AudioTrackInput {
            track_id: "track_0001".to_owned(),
            source_kind: "local_file".to_owned(),
            audio_path_or_stream_ref: "file:///tmp/source.wav".to_owned(),
            audio_sha256: Some("a".repeat(64)),
            start_at_ms: 0,
            duration_ms: 2_000,
            sample_rate_hz: 48_000,
            channel_count: 2,
        };
        let context = SpeechToSpeechContext {
            track_id: "track_0001".to_owned(),
            source_kind: "local_file".to_owned(),
            playback_generation: core.snapshot().playback_generation,
            loop_index: 0,
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
        };
        let candidate = AudioVariantCandidate {
            playback_generation: context.playback_generation,
            loop_index: 0,
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
        };
        core.stage_audio_variant_candidate(&input, &context, candidate, 120)
            .expect("candidate should stage");
        assert_eq!(
            core.snapshot().current_audio_source.as_deref(),
            Some("original")
        );
        assert!(core.snapshot().pending_audio_candidate);
        core.commit_audio_variant_candidate(context.playback_generation, 0, "segment_0001")
            .expect("candidate should commit");
        assert_eq!(
            core.snapshot().current_audio_source.as_deref(),
            Some("realtime_variant")
        );
        assert!(!core.snapshot().pending_audio_candidate);
    }

    #[test]
    fn playback_core_commits_staged_audio_only_at_its_start_boundary() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(false, false, true);
        let input = AudioTrackInput {
            track_id: "track_0001".to_owned(),
            source_kind: "local_file".to_owned(),
            audio_path_or_stream_ref: "file:///tmp/source.wav".to_owned(),
            audio_sha256: Some("a".repeat(64)),
            start_at_ms: 1_000,
            duration_ms: 2_000,
            sample_rate_hz: 48_000,
            channel_count: 2,
        };
        let context = SpeechToSpeechContext {
            track_id: input.track_id.clone(),
            source_kind: input.source_kind.clone(),
            playback_generation: core.snapshot().playback_generation,
            loop_index: 0,
            segment_id: "segment_0001".to_owned(),
            start_at_ms: 1_000,
            sample_rate_hz: 48_000,
            channel_count: 2,
            audio_path_or_stream_ref: input.audio_path_or_stream_ref.clone(),
            transcript_text: "今天介绍这款产品".to_owned(),
            previous_variant_text: None,
            locked_fields: vec!["产品名".to_owned()],
            locked_field_values: vec!["这款产品".to_owned()],
            target_duration_ms: 1_000,
            max_chars: 40,
            language: "zh-CN".to_owned(),
            rewrite_policy: "keep_locked_fields".to_owned(),
            timeout_ms: 800,
        };
        let candidate = AudioVariantCandidate {
            playback_generation: context.playback_generation,
            loop_index: 0,
            segment_id: context.segment_id.clone(),
            start_at_ms: 1_000,
            variant_mode: "rewrite".to_owned(),
            audio_path_or_stream_ref: "file:///tmp/variant.wav".to_owned(),
            audio_sha256: "b".repeat(64),
            duration_ms: 980,
            sync_offset_ms: 40,
            sample_rate_hz: 48_000,
            channel_count: 2,
            ready: true,
        };
        core.stage_audio_variant_candidate(&input, &context, candidate, 120)
            .expect("candidate should stage");
        assert!(!core
            .commit_audio_variant_candidate_if_due(999)
            .expect("early commit check should succeed"));
        assert_eq!(
            core.snapshot().current_audio_source.as_deref(),
            Some("original")
        );
        assert!(core
            .commit_audio_variant_candidate_if_due(1_000)
            .expect("boundary commit should succeed"));
        assert_eq!(
            core.snapshot().current_audio_source.as_deref(),
            Some("realtime_variant")
        );
    }

    #[test]
    fn loop_and_stop_discard_pending_audio_candidate() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(false, false, true);
        let input = AudioTrackInput {
            track_id: "track_0001".to_owned(),
            source_kind: "local_file".to_owned(),
            audio_path_or_stream_ref: "file:///tmp/source.wav".to_owned(),
            audio_sha256: Some("a".repeat(64)),
            start_at_ms: 0,
            duration_ms: 2_000,
            sample_rate_hz: 48_000,
            channel_count: 2,
        };
        let context = SpeechToSpeechContext {
            track_id: "track_0001".to_owned(),
            source_kind: "local_file".to_owned(),
            playback_generation: core.snapshot().playback_generation,
            loop_index: 0,
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
        };
        let candidate = AudioVariantCandidate {
            playback_generation: context.playback_generation,
            loop_index: 0,
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
        };
        core.stage_audio_variant_candidate(&input, &context, candidate, 120)
            .expect("candidate should stage");
        core.complete_loop().expect("loop should complete");
        assert!(!core.snapshot().pending_audio_candidate);
        assert_eq!(
            core.snapshot().current_audio_source.as_deref(),
            Some("original")
        );

        core.stage_audio_variant_candidate(
            &input,
            &context,
            AudioVariantCandidate {
                loop_index: 1,
                ..AudioVariantCandidate {
                    playback_generation: context.playback_generation,
                    loop_index: 0,
                    segment_id: "segment_0001".to_owned(),
                    start_at_ms: 0,
                    variant_mode: "rewrite".to_owned(),
                    audio_path_or_stream_ref: "file:///tmp/variant.wav".to_owned(),
                    audio_sha256: "c".repeat(64),
                    duration_ms: 980,
                    sync_offset_ms: 40,
                    sample_rate_hz: 48_000,
                    channel_count: 2,
                    ready: true,
                }
            },
            120,
        )
        .expect_err("stale loop candidate should be rejected");
        core.stop();
        assert!(!core.snapshot().pending_audio_candidate);
    }
}
