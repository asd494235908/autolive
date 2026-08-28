pub mod ambient_sound;
pub mod audio_cycle_output;
pub mod audio_feature_analysis;
pub mod audio_mixer;
pub mod audio_output_diagnostic;
pub mod audio_output_health;
pub mod audio_pcm_effects;
pub mod audio_processing;
pub mod background_process;
pub mod bounded_io;
pub mod cancellation;
pub mod direct_model;
pub mod errors;
pub mod hashing;
pub mod interlude_player;
pub mod media_audio_effects;
pub mod media_av_sync;
pub mod media_compatibility;
pub mod media_effect_params;
pub mod media_engine;
pub mod media_gpu_capabilities;
pub mod media_library;
pub mod media_video_effects;
pub mod media_video_gpu_effects;
pub mod realtime_video_backend;
pub mod realtime_video_runtime;
pub mod runtime_resource_task;
pub mod runtime_resources;
pub mod speech_to_speech;
pub mod speech_to_speech_worker;
pub mod webview_interlude_cache;
pub mod window_sizing;

use crate::audio_processing::AudioProcessingProfile;
use crate::errors::PlaybackError;
use crate::interlude_player::{resolve_effective_audio_source, InterludeSnapshot};
use crate::media_effect_params::{AudioEffectParams, ParameterValidationError};
use crate::media_library::{MediaKind, SourceMediaDto};
use crate::speech_to_speech::{
    AudioTrackInput, AudioVariantCandidate, CandidateValidationError, SpeechToSpeechContext,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_SOURCE_MEDIA_POOL_ITEMS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackState {
    Ready,
    Playing,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedAudioStreamConfiguration {
    params: AudioEffectParams,
    variants: Vec<AudioEffectParams>,
}

impl ValidatedAudioStreamConfiguration {
    pub fn new(
        params: AudioEffectParams,
        variants: Vec<AudioEffectParams>,
    ) -> Result<Self, Vec<ParameterValidationError>> {
        let mut errors = Vec::new();
        if let Err(mut params_errors) = params.validate() {
            errors.append(&mut params_errors);
        }
        if variants.len() > 4 {
            errors.push(ParameterValidationError {
                field: "audio_variants".to_owned(),
                code: "too_many_items".to_owned(),
                unit: "条".to_owned(),
                value: Some(variants.len() as f64),
                min: Some(0.0),
                max: Some(4.0),
                message: format!(
                    "audio_variants 最多允许 4 条，实际收到 {} 条",
                    variants.len()
                ),
            });
        }
        for (index, variant) in variants.iter().enumerate() {
            if let Err(variant_errors) = variant.validate() {
                errors.extend(variant_errors.into_iter().map(|mut error| {
                    error.field = format!("audio_variants[{index}].{}", error.field);
                    error
                }));
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(Self { params, variants })
    }

    fn into_parts(self) -> (AudioEffectParams, Vec<AudioEffectParams>) {
        (self.params, self.variants)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackSnapshot {
    pub window_id: Option<String>,
    pub playback_generation: u64,
    pub playback_state: PlaybackState,
    pub source_media: Option<SourceMediaDto>,
    pub source_media_pool: Vec<SourceMediaDto>,
    pub source_media_index: usize,
    pub playback_pool_cycle: u64,
    pub loop_index: u64,
    pub current_position_ms: u64,
    pub current_video_source: Option<String>,
    pub current_video_reference: Option<String>,
    pub current_video_sha256: Option<String>,
    pub current_media_plan_id: Option<String>,
    pub current_media_sequence: Option<u64>,
    pub current_media_playback_generation: Option<u64>,
    pub current_media_source_revision: Option<u64>,
    pub current_media_target_absolute_position_ms: Option<u64>,
    pub current_media_source_start_ms: Option<u64>,
    pub current_media_output_duration_ms: Option<u64>,
    pub current_media_valid_until_absolute_position_ms: Option<u64>,
    pub pending_video_reference: Option<String>,
    pub pending_video_sha256: Option<String>,
    pub pending_media_plan_id: Option<String>,
    pub pending_media_sequence: Option<u64>,
    pub pending_media_playback_generation: Option<u64>,
    pub pending_media_source_revision: Option<u64>,
    pub pending_media_target_absolute_position_ms: Option<u64>,
    pub pending_media_source_start_ms: Option<u64>,
    pub pending_media_output_duration_ms: Option<u64>,
    pub pending_media_valid_until_absolute_position_ms: Option<u64>,
    pub video_processing_enabled: bool,
    pub video_processing_status: String,
    pub video_processing_progress_percent: u8,
    pub audio_processing_enabled: bool,
    pub audio_processing_progress_percent: u8,
    pub current_audio_artifact_reference: Option<String>,
    pub current_audio_artifact_sha256: Option<String>,
    pub current_audio_media_plan_id: Option<String>,
    pub current_audio_media_sequence: Option<u64>,
    pub current_audio_media_playback_generation: Option<u64>,
    pub current_audio_media_source_revision: Option<u64>,
    pub current_audio_media_target_absolute_position_ms: Option<u64>,
    pub current_audio_media_source_start_ms: Option<u64>,
    pub current_audio_media_output_duration_ms: Option<u64>,
    pub current_audio_media_valid_until_absolute_position_ms: Option<u64>,
    pub pending_audio_artifact_reference: Option<String>,
    pub pending_audio_artifact_sha256: Option<String>,
    pub pending_audio_media_plan_id: Option<String>,
    pub pending_audio_media_sequence: Option<u64>,
    pub pending_audio_media_playback_generation: Option<u64>,
    pub pending_audio_media_source_revision: Option<u64>,
    pub pending_audio_media_target_absolute_position_ms: Option<u64>,
    pub pending_audio_media_source_start_ms: Option<u64>,
    pub pending_audio_media_output_duration_ms: Option<u64>,
    pub pending_audio_media_valid_until_absolute_position_ms: Option<u64>,
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
    pub audio_stream_params: AudioEffectParams,
    pub audio_stream_variants: Vec<AudioEffectParams>,
    pub audio_stream_variant_count: usize,
    pub audio_stream_revision: u64,
    pub audio_processing_status: String,
    pub audio_processing_runtime: bool,
    pub audio_processing_gain_db: f64,
    pub effective_audio_source: String,
    pub interlude: InterludeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMediaCandidateIdentity {
    pub plan_id: String,
    pub sequence: u64,
    pub playback_generation: u64,
    pub source_revision: u64,
    pub target_absolute_position_ms: u64,
    pub source_start_ms: u64,
    pub output_duration_ms: u64,
    pub valid_until_absolute_position_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAudioMediaCandidateIdentity {
    pub plan_id: String,
    pub sequence: u64,
    pub playback_generation: u64,
    pub source_revision: u64,
    pub target_absolute_position_ms: u64,
    pub source_start_ms: u64,
    pub output_duration_ms: u64,
    pub valid_until_absolute_position_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaProcessingCommitOutcome {
    NotCommitted,
    Committed,
    Expired { artifact_path: Option<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackCore {
    window_id: Option<String>,
    playback_generation: u64,
    playback_state: PlaybackState,
    source_media: Option<SourceMediaDto>,
    source_media_pool: Vec<SourceMediaDto>,
    source_media_index: usize,
    playback_pool_cycle: u64,
    loop_index: u64,
    current_video_source: Option<String>,
    current_video_reference: Option<String>,
    current_video_sha256: Option<String>,
    current_media_candidate: Option<PendingMediaCandidateIdentity>,
    pending_video_reference: Option<String>,
    pending_video_sha256: Option<String>,
    pending_media_candidate: Option<PendingMediaCandidateIdentity>,
    video_processing_enabled: bool,
    video_processing_status: String,
    video_processing_progress_percent: u8,
    audio_processing_enabled: bool,
    audio_processing_progress_percent: u8,
    current_audio_artifact_reference: Option<String>,
    current_audio_artifact_sha256: Option<String>,
    current_audio_media_candidate: Option<PendingAudioMediaCandidateIdentity>,
    pending_audio_artifact_reference: Option<String>,
    pending_audio_artifact_sha256: Option<String>,
    pending_audio_media_candidate: Option<PendingAudioMediaCandidateIdentity>,
    pending_audio_media_configuration: Option<ValidatedAudioStreamConfiguration>,
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
    audio_stream_variants: Vec<AudioEffectParams>,
    audio_stream_revision: u64,
    audio_processing_status: String,
    interlude_snapshot: InterludeSnapshot,
    current_position_ms: u64,
}

impl Default for PlaybackCore {
    fn default() -> Self {
        Self {
            window_id: None,
            playback_generation: 0,
            playback_state: PlaybackState::Stopped,
            source_media: None,
            source_media_pool: Vec::new(),
            source_media_index: 0,
            playback_pool_cycle: 0,
            loop_index: 0,
            current_video_source: None,
            current_video_reference: None,
            current_video_sha256: None,
            current_media_candidate: None,
            pending_video_reference: None,
            pending_video_sha256: None,
            pending_media_candidate: None,
            video_processing_enabled: false,
            video_processing_status: "disabled".to_owned(),
            video_processing_progress_percent: 0,
            audio_processing_enabled: false,
            audio_processing_progress_percent: 0,
            current_audio_artifact_reference: None,
            current_audio_artifact_sha256: None,
            current_audio_media_candidate: None,
            pending_audio_artifact_reference: None,
            pending_audio_artifact_sha256: None,
            pending_audio_media_candidate: None,
            pending_audio_media_configuration: None,
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
            audio_stream_variants: Vec::new(),
            audio_stream_revision: 0,
            audio_processing_status: "disabled".to_owned(),
            interlude_snapshot: InterludeSnapshot::default(),
            current_position_ms: 0,
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

    pub fn set_source(&mut self, source_media: SourceMediaDto) {
        self.replace_source_pool(vec![source_media]);
    }

    pub fn set_source_pool(
        &mut self,
        source_media_pool: Vec<SourceMediaDto>,
    ) -> Result<(), PlaybackError> {
        Self::validate_source_pool(&source_media_pool)?;
        self.replace_source_pool(source_media_pool);
        Ok(())
    }

    pub fn append_source_pool(
        &mut self,
        source_media: Vec<SourceMediaDto>,
    ) -> Result<(), PlaybackError> {
        if source_media.is_empty() {
            return Err(PlaybackError::SourceMediaPoolEmpty);
        }
        let mut edited = self.source_media_pool.clone();
        edited.extend(source_media);
        Self::validate_source_pool(&edited)?;
        self.replace_source_pool(edited);
        Ok(())
    }

    pub fn replace_source_at(
        &mut self,
        index: usize,
        source_media: SourceMediaDto,
    ) -> Result<(), PlaybackError> {
        self.validate_source_index(index)?;
        let mut edited = self.source_media_pool.clone();
        edited[index] = source_media;
        Self::validate_source_pool(&edited)?;
        self.replace_source_pool(edited);
        Ok(())
    }

    pub fn remove_source(&mut self, index: usize) -> Result<(), PlaybackError> {
        self.validate_source_index(index)?;
        let mut edited = self.source_media_pool.clone();
        edited.remove(index);
        self.replace_source_pool(edited);
        Ok(())
    }

    pub fn clear_source_pool(&mut self) {
        if !self.source_media_pool.is_empty() {
            self.replace_source_pool(Vec::new());
        }
    }

    fn validate_source_pool(source_media_pool: &[SourceMediaDto]) -> Result<(), PlaybackError> {
        if source_media_pool.is_empty() {
            return Err(PlaybackError::SourceMediaPoolEmpty);
        }
        if source_media_pool.len() > MAX_SOURCE_MEDIA_POOL_ITEMS {
            return Err(PlaybackError::SourceMediaPoolTooLarge {
                max: MAX_SOURCE_MEDIA_POOL_ITEMS,
                actual: source_media_pool.len(),
            });
        }
        let mut paths = HashSet::with_capacity(source_media_pool.len());
        for (index, source) in source_media_pool.iter().enumerate() {
            let path = source.source_path.trim();
            if path.is_empty() {
                return Err(PlaybackError::SourceMediaPathEmpty { index });
            }
            let duplicate_key = if cfg!(windows) {
                path.to_ascii_lowercase()
            } else {
                path.to_owned()
            };
            if !paths.insert(duplicate_key) {
                return Err(PlaybackError::DuplicateSourceMediaPath {
                    path: path.to_owned(),
                });
            }
        }
        Ok(())
    }

    fn validate_source_index(&self, index: usize) -> Result<(), PlaybackError> {
        if index < self.source_media_pool.len() {
            Ok(())
        } else {
            Err(PlaybackError::SourceMediaPoolIndexOutOfBounds {
                index,
                len: self.source_media_pool.len(),
            })
        }
    }

    fn replace_source_pool(&mut self, mut source_media_pool: Vec<SourceMediaDto>) {
        for source_media in &mut source_media_pool {
            source_media.mp4_sha256 = None;
            source_media.mp4_hash_status = "disabled".to_owned();
        }
        self.source_media = source_media_pool.first().cloned();
        self.source_media_pool = source_media_pool;
        self.source_media_index = 0;
        self.playback_pool_cycle = 0;
        self.loop_index = 0;
        self.playback_generation = self.playback_generation.wrapping_add(1);
        self.playback_state = if self.source_media_pool.is_empty() {
            PlaybackState::Stopped
        } else {
            PlaybackState::Ready
        };
        self.current_position_ms = 0;
        self.reset_audio_to_original();
        self.reset_video_to_original();
        self.clear_pending_media_processing();
        self.worker_status = "unavailable".to_owned();
        self.fallback_reason = None;
        self.pending_audio_candidate = None;
        self.audio_processing_status = self.audio_processing_status_for("unavailable");
        self.video_processing_status = self.video_processing_status_for("unavailable");
    }

    pub fn set_playback_position(&mut self, position_ms: u64) {
        self.current_position_ms = self
            .source_media
            .as_ref()
            .and_then(|source| source.duration_ms)
            .map_or(position_ms, |duration_ms| position_ms.min(duration_ms));
    }

    pub fn mark_media_processing_running(&mut self) -> Result<(), PlaybackError> {
        self.require_source()?;
        if !self.video_processing_is_applicable() && !self.audio_processing_enabled {
            return Ok(());
        }
        self.video_processing_status = self.video_processing_status_for("processing");
        self.video_processing_progress_percent = 0;
        self.clear_pending_media_processing();
        self.fallback_reason = None;
        Ok(())
    }

    pub fn mark_media_processing_running_with_candidate(
        &mut self,
        candidate: PendingMediaCandidateIdentity,
    ) -> Result<(), PlaybackError> {
        if candidate.playback_generation != self.playback_generation {
            return Err(PlaybackError::StaleMediaProcessing);
        }
        self.mark_media_processing_running()?;
        self.pending_media_candidate = Some(candidate);
        Ok(())
    }

    pub fn mark_video_processing_progress(&mut self, percent: u8) {
        if self.video_processing_status == "processing" {
            self.video_processing_progress_percent =
                self.video_processing_progress_percent.max(percent.min(99));
        }
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
        self.video_processing_status = self.video_processing_status_for("ready");
        self.video_processing_progress_percent = 100;
        self.fallback_reason = None;
        Ok(())
    }

    pub fn mark_media_processing_ready_for_candidate(
        &mut self,
        candidate: &PendingMediaCandidateIdentity,
        output_path: String,
        output_sha256: String,
    ) -> Result<(), PlaybackError> {
        if self.pending_media_candidate.as_ref() != Some(candidate)
            || candidate.playback_generation != self.playback_generation
        {
            return Err(PlaybackError::StaleMediaProcessing);
        }
        self.mark_media_processing_ready(candidate.playback_generation, output_path, output_sha256)
    }

    pub fn mark_media_processing_failed(&mut self, reason: impl Into<String>) {
        self.clear_pending_media_processing();
        self.video_processing_status = self.video_processing_status_for("failed");
        self.video_processing_progress_percent = 0;
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

    pub fn mark_audio_processing_runtime(&mut self) {
        if self.audio_processing_enabled {
            self.audio_processing_status = "runtime".to_owned();
            self.fallback_reason = None;
        }
    }

    pub fn mark_audio_media_processing_running(
        &mut self,
        mut candidate: PendingAudioMediaCandidateIdentity,
        configuration: ValidatedAudioStreamConfiguration,
    ) -> Result<PendingAudioMediaCandidateIdentity, PlaybackError> {
        self.require_source()?;
        if !self.audio_processing_enabled
            || candidate.playback_generation != self.playback_generation
            || candidate.source_revision != self.audio_stream_revision
        {
            return Err(PlaybackError::StaleMediaProcessing);
        }
        let configuration_changed = self.audio_processing_profile.params != configuration.params
            || self.audio_stream_variants != configuration.variants;
        candidate.source_revision = self
            .audio_stream_revision
            .wrapping_add(u64::from(configuration_changed));
        self.pending_audio_artifact_reference = None;
        self.pending_audio_artifact_sha256 = None;
        self.pending_audio_media_candidate = Some(candidate.clone());
        self.pending_audio_media_configuration = Some(configuration);
        self.audio_processing_progress_percent = 0;
        self.audio_processing_status = "processing".to_owned();
        self.fallback_reason = None;
        Ok(candidate)
    }

    pub fn mark_audio_media_processing_progress(&mut self, percent: u8) {
        if self.audio_processing_status == "processing" {
            self.audio_processing_progress_percent =
                self.audio_processing_progress_percent.max(percent.min(99));
        }
    }

    pub fn mark_audio_media_processing_ready(
        &mut self,
        candidate: &PendingAudioMediaCandidateIdentity,
        output_path: String,
        output_sha256: String,
    ) -> Result<(), PlaybackError> {
        if self.pending_audio_media_candidate.as_ref() != Some(candidate)
            || candidate.playback_generation != self.playback_generation
        {
            return Err(PlaybackError::StaleMediaProcessing);
        }
        if output_path.trim().is_empty()
            || output_sha256.len() != 64
            || !output_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(PlaybackError::InvalidMediaProcessingOutput);
        }
        self.pending_audio_artifact_reference = Some(output_path);
        self.pending_audio_artifact_sha256 = Some(output_sha256);
        self.audio_processing_progress_percent = 100;
        self.audio_processing_status = "ready".to_owned();
        self.fallback_reason = None;
        Ok(())
    }

    pub fn mark_audio_media_processing_failed(&mut self, reason: impl Into<String>) {
        self.pending_audio_artifact_reference = None;
        self.pending_audio_artifact_sha256 = None;
        self.pending_audio_media_candidate = None;
        self.pending_audio_media_configuration = None;
        self.audio_processing_progress_percent = 0;
        self.audio_processing_status = self.audio_processing_status_for("failed");
        self.fallback_reason = Some(reason.into());
    }

    pub fn commit_audio_media_candidate(
        &mut self,
        plan_id: &str,
        sequence: u64,
        playback_generation: u64,
        source_revision: u64,
    ) -> bool {
        let Some(candidate) = self.pending_audio_media_candidate.as_ref() else {
            return false;
        };
        if self.pending_audio_artifact_reference.is_none()
            || self.pending_audio_artifact_sha256.is_none()
            || candidate.plan_id != plan_id
            || candidate.sequence != sequence
            || candidate.playback_generation != playback_generation
            || candidate.source_revision != source_revision
            || playback_generation != self.playback_generation
        {
            return false;
        }
        let target_revision = candidate.source_revision;
        let Some(configuration) = self.pending_audio_media_configuration.take() else {
            return false;
        };
        self.commit_validated_audio_stream_configuration(configuration);
        if self.audio_stream_revision != target_revision {
            self.pending_audio_media_configuration = None;
            return false;
        }
        self.current_audio_artifact_reference = self.pending_audio_artifact_reference.take();
        self.current_audio_artifact_sha256 = self.pending_audio_artifact_sha256.take();
        self.current_audio_media_candidate = self.pending_audio_media_candidate.take();
        self.audio_processing_progress_percent = 100;
        self.audio_processing_status = "runtime".to_owned();
        self.fallback_reason = None;
        true
    }

    pub fn discard_audio_media_candidate(
        &mut self,
        plan_id: &str,
        sequence: u64,
        playback_generation: u64,
        source_revision: u64,
        reason: impl Into<String>,
    ) -> Option<String> {
        if self.audio_processing_status == "processing" {
            return None;
        }
        let candidate = self.pending_audio_media_candidate.as_ref()?;
        if candidate.plan_id != plan_id
            || candidate.sequence != sequence
            || candidate.playback_generation != playback_generation
            || candidate.source_revision != source_revision
            || playback_generation != self.playback_generation
        {
            return None;
        }
        let artifact_path = self.pending_audio_artifact_reference.take();
        self.pending_audio_artifact_sha256 = None;
        self.pending_audio_media_candidate = None;
        self.pending_audio_media_configuration = None;
        self.audio_processing_progress_percent = 0;
        self.audio_processing_status = self.audio_processing_status_for("failed");
        self.fallback_reason = Some(reason.into());
        artifact_path
    }

    pub fn commit_media_processing_if_ready(
        &mut self,
        plan_id: &str,
        sequence: u64,
        playback_generation: u64,
        source_revision: u64,
        observed_absolute_position_ms: u64,
    ) -> MediaProcessingCommitOutcome {
        if self.pending_video_reference.is_none() || self.pending_video_sha256.is_none() {
            return MediaProcessingCommitOutcome::NotCommitted;
        }
        let Some(candidate) = self.pending_media_candidate.as_ref() else {
            return MediaProcessingCommitOutcome::NotCommitted;
        };
        if candidate.plan_id != plan_id
            || candidate.sequence != sequence
            || candidate.playback_generation != playback_generation
            || candidate.source_revision != source_revision
            || playback_generation != self.playback_generation
        {
            return MediaProcessingCommitOutcome::NotCommitted;
        }
        if observed_absolute_position_ms < candidate.target_absolute_position_ms {
            return MediaProcessingCommitOutcome::NotCommitted;
        }
        if observed_absolute_position_ms >= candidate.valid_until_absolute_position_ms {
            let artifact_path = self.discard_media_processing_candidate(
                plan_id,
                sequence,
                playback_generation,
                source_revision,
                "候选视频已超过有效媒体时间窗口，跳过本轮并准备下一轮",
            );
            return MediaProcessingCommitOutcome::Expired { artifact_path };
        }
        self.commit_pending_video();
        MediaProcessingCommitOutcome::Committed
    }

    pub fn discard_media_processing_candidate(
        &mut self,
        plan_id: &str,
        sequence: u64,
        playback_generation: u64,
        source_revision: u64,
        reason: impl Into<String>,
    ) -> Option<String> {
        if self.video_processing_status == "processing" {
            return None;
        }
        let candidate = self.pending_media_candidate.as_ref()?;
        if candidate.plan_id != plan_id
            || candidate.sequence != sequence
            || candidate.playback_generation != playback_generation
            || candidate.source_revision != source_revision
            || playback_generation != self.playback_generation
        {
            return None;
        }

        let artifact_path = self.pending_video_reference.take();
        self.pending_video_sha256 = None;
        self.pending_media_candidate = None;
        self.video_processing_status = self.video_processing_status_for("failed");
        self.video_processing_progress_percent = 0;
        self.fallback_reason = Some(reason.into());
        artifact_path
    }

    pub fn set_processing_switches(
        &mut self,
        video_processing_enabled: bool,
        audio_processing_enabled: bool,
        realtime_audio_variant_enabled: bool,
    ) {
        let video_was_enabled = self.video_processing_enabled;
        let audio_was_enabled = self.audio_processing_enabled;
        let realtime_was_enabled = self.realtime_audio_variant_enabled;
        let video_changed = video_was_enabled != video_processing_enabled;
        let audio_changed = audio_was_enabled != audio_processing_enabled;
        let realtime_changed = realtime_was_enabled != realtime_audio_variant_enabled;
        self.video_processing_enabled = video_processing_enabled;
        self.audio_processing_enabled = audio_processing_enabled;
        self.realtime_audio_variant_enabled = realtime_audio_variant_enabled;
        if video_changed {
            self.video_processing_status = self.video_processing_status_for("unavailable");
            self.video_processing_progress_percent = 0;
            self.reset_video_to_original();
            self.clear_pending_media_processing();
        }
        if audio_changed {
            self.audio_processing_status = self.audio_processing_status_for("unavailable");
            self.audio_processing_progress_percent = 0;
            if audio_processing_enabled {
                self.clear_audio_media_processing();
            } else {
                self.reset_audio_to_original();
            }
        }
        if realtime_changed && realtime_audio_variant_enabled {
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
        } else if realtime_changed {
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
    ) -> Result<(), Vec<crate::media_effect_params::ParameterValidationError>> {
        profile.validate()?;
        let changed = self.audio_processing_profile != profile;
        self.audio_processing_profile = profile;
        if changed {
            self.audio_stream_revision = self.audio_stream_revision.wrapping_add(1);
        }
        // 处理中只更新配置，不改状态；避免把 processing 冲掉或伪装成 unavailable。
        if self.audio_processing_status != "processing" {
            self.audio_processing_status = self.audio_processing_status_for("configured");
        }
        Ok(())
    }

    pub fn set_audio_stream_configuration(
        &mut self,
        params: AudioEffectParams,
        variants: Vec<AudioEffectParams>,
    ) -> Result<(), Vec<crate::media_effect_params::ParameterValidationError>> {
        let configuration = ValidatedAudioStreamConfiguration::new(params, variants)?;
        self.commit_validated_audio_stream_configuration(configuration);
        Ok(())
    }

    pub fn commit_validated_audio_stream_configuration(
        &mut self,
        configuration: ValidatedAudioStreamConfiguration,
    ) {
        let (params, variants) = configuration.into_parts();
        if self.audio_processing_profile.params == params && self.audio_stream_variants == variants
        {
            return;
        }
        self.audio_processing_profile.params = params;
        self.audio_stream_variants = variants;
        self.audio_stream_revision = self.audio_stream_revision.wrapping_add(1);
    }

    pub fn audio_stream_configuration(&self) -> (AudioEffectParams, Vec<AudioEffectParams>) {
        (
            self.audio_processing_profile.params.clone(),
            self.audio_stream_variants.clone(),
        )
    }

    fn effective_audio_stream_variant_count(&self) -> usize {
        if !self.audio_processing_enabled
            || self.current_audio_source.as_deref() == Some("realtime_variant")
            || self
                .source_media
                .as_ref()
                .and_then(|source| source.audio_sample_rate_hz)
                .is_none()
        {
            0
        } else {
            self.audio_stream_variants.len().max(1)
        }
    }

    pub fn set_interlude_snapshot(&mut self, snapshot: InterludeSnapshot) {
        self.interlude_snapshot = snapshot;
    }

    pub fn set_mp4_sha256_for_generation(&mut self, generation: u64, mp4_sha256: String) {
        if self.playback_generation != generation {
            return;
        }
        if let Some(source_media) = self.source_media.as_mut() {
            source_media.mp4_sha256 = Some(mp4_sha256);
            source_media.mp4_hash_status = "ready".to_owned();
        }
        if let Some(source_media) = self.source_media_pool.get_mut(self.source_media_index) {
            source_media.mp4_sha256 = self
                .source_media
                .as_ref()
                .and_then(|source| source.mp4_sha256.clone());
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
        if let Some(source_media) = self.source_media_pool.get_mut(self.source_media_index) {
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
        let reason = reason.into();
        if self.current_audio_source.as_deref() == Some("realtime_variant")
            && self.current_audio_reference.is_some()
        {
            self.worker_status = "failed".to_owned();
            self.fallback_reason = Some(reason);
            return;
        }
        self.current_audio_source = self.source_media.as_ref().map(|_| "original".to_owned());
        self.current_audio_reference = self
            .source_media
            .as_ref()
            .map(|source| source.file_name.clone());
        self.current_audio_start_at_ms = 0;
        self.current_audio_sha256 = None;
        self.audio_decision = "fallback_original".to_owned();
        self.worker_status = "failed".to_owned();
        self.fallback_reason = Some(reason);
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
        self.current_position_ms = 0;
        self.reset_audio_to_original();
        self.reset_video_to_original();
        self.clear_pending_media_processing();
        self.video_processing_status = self.video_processing_status_for("unavailable");
        self.audio_processing_status = self.audio_processing_status_for("unavailable");
        self.fallback_reason = None;
    }

    pub fn complete_item(&mut self) -> Result<bool, PlaybackError> {
        self.require_source()?;
        if self.source_media_pool.len() > 1 {
            self.source_media_index = (self.source_media_index + 1) % self.source_media_pool.len();
            if self.source_media_index == 0 {
                self.playback_pool_cycle = self.playback_pool_cycle.saturating_add(1);
            }
            self.source_media = self.source_media_pool.get(self.source_media_index).cloned();
            self.playback_generation = self.playback_generation.wrapping_add(1);
            self.loop_index = 0;
            self.playback_state = PlaybackState::Playing;
            self.current_position_ms = 0;
            self.reset_audio_to_original();
            self.reset_video_to_original();
            self.clear_pending_media_processing();
            self.pending_audio_candidate = None;
            self.worker_status = "unavailable".to_owned();
            self.fallback_reason = None;
            self.audio_processing_status = self.audio_processing_status_for("unavailable");
            self.video_processing_status = self.video_processing_status_for("unavailable");
            return Ok(true);
        }
        self.loop_index = self.loop_index.saturating_add(1);
        self.playback_pool_cycle = self.playback_pool_cycle.saturating_add(1);
        self.playback_state = PlaybackState::Playing;
        self.pending_audio_candidate = None;
        self.current_position_ms = 0;
        self.reset_audio_to_original();
        Ok(false)
    }

    pub fn complete_loop(&mut self) -> Result<(), PlaybackError> {
        self.complete_item().map(|_| ())
    }

    #[must_use]
    pub fn snapshot(&self) -> PlaybackSnapshot {
        let effective_audio_source = resolve_effective_audio_source(
            self.current_audio_source.as_deref(),
            self.current_video_source.as_deref(),
        )
        .to_owned();
        PlaybackSnapshot {
            window_id: self.window_id.clone(),
            playback_generation: self.playback_generation,
            playback_state: self.playback_state,
            source_media: self.source_media.clone(),
            source_media_pool: self.source_media_pool.clone(),
            source_media_index: self.source_media_index,
            playback_pool_cycle: self.playback_pool_cycle,
            loop_index: self.loop_index,
            current_position_ms: self.current_position_ms,
            current_video_source: self.current_video_source.clone(),
            current_video_reference: self.current_video_reference.clone(),
            current_video_sha256: self.current_video_sha256.clone(),
            current_media_plan_id: self
                .current_media_candidate
                .as_ref()
                .map(|candidate| candidate.plan_id.clone()),
            current_media_sequence: self
                .current_media_candidate
                .as_ref()
                .map(|candidate| candidate.sequence),
            current_media_playback_generation: self
                .current_media_candidate
                .as_ref()
                .map(|candidate| candidate.playback_generation),
            current_media_source_revision: self
                .current_media_candidate
                .as_ref()
                .map(|candidate| candidate.source_revision),
            current_media_target_absolute_position_ms: self
                .current_media_candidate
                .as_ref()
                .map(|candidate| candidate.target_absolute_position_ms),
            current_media_source_start_ms: self
                .current_media_candidate
                .as_ref()
                .map(|candidate| candidate.source_start_ms),
            current_media_output_duration_ms: self
                .current_media_candidate
                .as_ref()
                .map(|candidate| candidate.output_duration_ms),
            current_media_valid_until_absolute_position_ms: self
                .current_media_candidate
                .as_ref()
                .map(|candidate| candidate.valid_until_absolute_position_ms),
            pending_video_reference: self.pending_video_reference.clone(),
            pending_video_sha256: self.pending_video_sha256.clone(),
            pending_media_plan_id: self
                .pending_media_candidate
                .as_ref()
                .map(|candidate| candidate.plan_id.clone()),
            pending_media_sequence: self
                .pending_media_candidate
                .as_ref()
                .map(|candidate| candidate.sequence),
            pending_media_playback_generation: self
                .pending_media_candidate
                .as_ref()
                .map(|candidate| candidate.playback_generation),
            pending_media_source_revision: self
                .pending_media_candidate
                .as_ref()
                .map(|candidate| candidate.source_revision),
            pending_media_target_absolute_position_ms: self
                .pending_media_candidate
                .as_ref()
                .map(|candidate| candidate.target_absolute_position_ms),
            pending_media_source_start_ms: self
                .pending_media_candidate
                .as_ref()
                .map(|candidate| candidate.source_start_ms),
            pending_media_output_duration_ms: self
                .pending_media_candidate
                .as_ref()
                .map(|candidate| candidate.output_duration_ms),
            pending_media_valid_until_absolute_position_ms: self
                .pending_media_candidate
                .as_ref()
                .map(|candidate| candidate.valid_until_absolute_position_ms),
            video_processing_enabled: self.video_processing_enabled,
            video_processing_status: self.video_processing_status.clone(),
            video_processing_progress_percent: self.video_processing_progress_percent,
            audio_processing_enabled: self.audio_processing_enabled,
            audio_processing_progress_percent: self.audio_processing_progress_percent,
            current_audio_artifact_reference: self.current_audio_artifact_reference.clone(),
            current_audio_artifact_sha256: self.current_audio_artifact_sha256.clone(),
            current_audio_media_plan_id: self
                .current_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.plan_id.clone()),
            current_audio_media_sequence: self
                .current_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.sequence),
            current_audio_media_playback_generation: self
                .current_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.playback_generation),
            current_audio_media_source_revision: self
                .current_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.source_revision),
            current_audio_media_target_absolute_position_ms: self
                .current_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.target_absolute_position_ms),
            current_audio_media_source_start_ms: self
                .current_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.source_start_ms),
            current_audio_media_output_duration_ms: self
                .current_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.output_duration_ms),
            current_audio_media_valid_until_absolute_position_ms: self
                .current_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.valid_until_absolute_position_ms),
            pending_audio_artifact_reference: self.pending_audio_artifact_reference.clone(),
            pending_audio_artifact_sha256: self.pending_audio_artifact_sha256.clone(),
            pending_audio_media_plan_id: self
                .pending_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.plan_id.clone()),
            pending_audio_media_sequence: self
                .pending_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.sequence),
            pending_audio_media_playback_generation: self
                .pending_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.playback_generation),
            pending_audio_media_source_revision: self
                .pending_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.source_revision),
            pending_audio_media_target_absolute_position_ms: self
                .pending_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.target_absolute_position_ms),
            pending_audio_media_source_start_ms: self
                .pending_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.source_start_ms),
            pending_audio_media_output_duration_ms: self
                .pending_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.output_duration_ms),
            pending_audio_media_valid_until_absolute_position_ms: self
                .pending_audio_media_candidate
                .as_ref()
                .map(|candidate| candidate.valid_until_absolute_position_ms),
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
            audio_stream_params: self.audio_processing_profile.params.clone(),
            audio_stream_variants: self.audio_stream_variants.clone(),
            audio_stream_variant_count: self.effective_audio_stream_variant_count(),
            audio_stream_revision: self.audio_stream_revision,
            audio_processing_status: self.audio_processing_status.clone(),
            audio_processing_runtime: self.audio_processing_status == "runtime",
            audio_processing_gain_db: self.audio_processing_profile.params.input_gain_db
                + self.audio_processing_profile.params.output_gain_db
                + self.audio_processing_profile.params.loudness_adjustment_db,
            effective_audio_source,
            interlude: self.interlude_snapshot.clone(),
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
        ordinary_status.to_owned()
    }

    fn video_processing_is_applicable(&self) -> bool {
        self.video_processing_enabled
            && self
                .source_media
                .as_ref()
                .is_some_and(|source| source.media_kind == MediaKind::Video)
    }

    fn video_processing_status_for(&self, ordinary_status: &str) -> String {
        if !self.video_processing_enabled {
            "disabled".to_owned()
        } else if !self.video_processing_is_applicable() {
            "not_applicable".to_owned()
        } else {
            ordinary_status.to_owned()
        }
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

    pub fn restore_original_video(&mut self) {
        self.reset_video_to_original();
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
        self.audio_processing_progress_percent = 0;
        self.current_audio_artifact_reference = None;
        self.current_audio_artifact_sha256 = None;
        self.current_audio_media_candidate = None;
        self.clear_audio_media_processing();
    }

    fn reset_video_to_original(&mut self) {
        self.video_processing_progress_percent = 0;
        self.current_video_source = self.source_media.as_ref().map(|_| "original".to_owned());
        self.current_video_reference = self
            .source_media
            .as_ref()
            .map(|source| source.playback_reference.clone());
        self.current_video_sha256 = None;
        self.current_media_candidate = None;
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
        self.current_media_candidate = self.pending_media_candidate.take();
    }

    fn clear_pending_media_processing(&mut self) {
        self.pending_video_reference = None;
        self.pending_video_sha256 = None;
        self.pending_media_candidate = None;
    }

    fn clear_audio_media_processing(&mut self) {
        self.pending_audio_artifact_reference = None;
        self.pending_audio_artifact_sha256 = None;
        self.pending_audio_media_candidate = None;
        self.pending_audio_media_configuration = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MediaProcessingCommitOutcome, PendingAudioMediaCandidateIdentity,
        PendingMediaCandidateIdentity, PlaybackCore, PlaybackState,
        ValidatedAudioStreamConfiguration, MAX_SOURCE_MEDIA_POOL_ITEMS,
    };
    use crate::audio_processing::AudioProcessingProfile;
    use crate::media_effect_params::AudioEffectParams;
    use crate::media_library::{MediaCompatibilityMode, MediaKind, SourceMediaDto};
    use crate::speech_to_speech::{AudioTrackInput, AudioVariantCandidate, SpeechToSpeechContext};

    fn source() -> SourceMediaDto {
        SourceMediaDto {
            source_path: "/tmp/source.mp4".to_owned(),
            playback_reference: "/tmp/source.mp4".to_owned(),
            media_kind: MediaKind::Video,
            compatibility_mode: MediaCompatibilityMode::Direct,
            file_name: "source.mp4".to_owned(),
            file_size_bytes: 1,
            duration_ms: Some(1_000),
            audio_start_ms: Some(0),
            audio_end_ms: Some(1_000),
            width: Some(1280),
            height: Some(720),
            frame_rate_fps: Some(30.0),
            audio_sample_rate_hz: Some(48_000),
            audio_channel_count: Some(2),
            video_codec_name: Some("h264".to_owned()),
            audio_codec_name: Some("aac".to_owned()),
            mp4_sha256: Some("a".repeat(64)),
            mp4_hash_status: "ready".to_owned(),
        }
    }

    fn pending_media_candidate(core: &PlaybackCore) -> PendingMediaCandidateIdentity {
        let snapshot = core.snapshot();
        PendingMediaCandidateIdentity {
            plan_id: "plan-1".to_owned(),
            sequence: 1,
            playback_generation: snapshot.playback_generation,
            source_revision: snapshot.audio_stream_revision,
            target_absolute_position_ms: 8_000,
            source_start_ms: 8_000,
            output_duration_ms: 10_000,
            valid_until_absolute_position_ms: 18_000,
        }
    }

    fn source_named(file_name: &str) -> SourceMediaDto {
        SourceMediaDto {
            source_path: format!("/tmp/{file_name}"),
            playback_reference: format!("/tmp/{file_name}"),
            file_name: file_name.to_owned(),
            ..source()
        }
    }

    fn audio_source(file_name: &str) -> SourceMediaDto {
        SourceMediaDto {
            source_path: format!("/tmp/{file_name}"),
            playback_reference: format!("/tmp/{file_name}"),
            media_kind: MediaKind::Audio,
            compatibility_mode: MediaCompatibilityMode::Direct,
            file_name: file_name.to_owned(),
            width: None,
            height: None,
            frame_rate_fps: None,
            video_codec_name: None,
            audio_codec_name: Some("mp3".to_owned()),
            ..source()
        }
    }

    #[test]
    fn playback_pool_advances_in_order_and_wraps_to_the_first_source() {
        let mut core = PlaybackCore::default();
        core.set_source_pool(vec![source_named("first.mp4"), source_named("second.mp4")])
            .expect("two sources should be accepted");
        core.set_processing_switches(true, true, false);
        core.start().expect("pool should start");
        let initial_generation = core.snapshot().playback_generation;
        core.current_video_source = Some("processed".to_owned());
        core.current_video_reference = Some("/tmp/processed.mp4".to_owned());
        core.current_video_sha256 = Some("b".repeat(64));
        core.pending_video_reference = Some("/tmp/pending.mp4".to_owned());
        core.pending_video_sha256 = Some("c".repeat(64));

        assert!(core.complete_item().expect("first item should complete"));
        let second = core.snapshot();
        assert_eq!(second.playback_generation, initial_generation + 1);
        assert_eq!(second.source_media_index, 1);
        assert_eq!(
            second.source_media.expect("second source").file_name,
            "second.mp4"
        );
        assert_eq!(second.loop_index, 0);
        assert_eq!(second.playback_pool_cycle, 0);
        assert!(second.video_processing_enabled);
        assert!(second.audio_processing_enabled);
        assert_eq!(second.current_video_source.as_deref(), Some("original"));
        assert_eq!(
            second.current_video_reference.as_deref(),
            Some("/tmp/second.mp4")
        );
        assert!(second.pending_video_reference.is_none());
        assert_eq!(second.current_audio_source.as_deref(), Some("original"));

        assert!(core.complete_item().expect("second item should complete"));
        let wrapped = core.snapshot();
        assert_eq!(wrapped.playback_generation, initial_generation + 2);
        assert_eq!(wrapped.source_media_index, 0);
        assert_eq!(
            wrapped.source_media.expect("first source").file_name,
            "first.mp4"
        );
        assert_eq!(wrapped.loop_index, 0);
        assert_eq!(wrapped.playback_pool_cycle, 1);
    }

    #[test]
    fn mixed_pool_uses_each_items_playback_reference_and_marks_audio_video_processing_not_applicable(
    ) {
        let mut core = PlaybackCore::default();
        let mut ts = source_named("first.ts");
        ts.playback_reference = "/tmp/cache/first.mp4".to_owned();
        ts.compatibility_mode = MediaCompatibilityMode::Remuxed;
        core.set_source_pool(vec![ts, audio_source("second.mp3")])
            .expect("mixed pool should be accepted");
        core.set_processing_switches(true, true, false);

        assert_eq!(
            core.snapshot().current_video_reference.as_deref(),
            Some("/tmp/cache/first.mp4")
        );
        core.start().expect("pool should start");
        core.complete_item().expect("audio item should be selected");

        let snapshot = core.snapshot();
        assert_eq!(
            snapshot.source_media.expect("audio source").media_kind,
            MediaKind::Audio
        );
        assert_eq!(
            snapshot.current_video_reference.as_deref(),
            Some("/tmp/second.mp3")
        );
        assert_eq!(snapshot.video_processing_status, "not_applicable");
        assert!(snapshot.audio_processing_enabled);
    }

    #[test]
    fn single_source_completion_keeps_generation_and_increments_loop_and_pool_cycle() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.start().expect("source should start");
        let generation = core.snapshot().playback_generation;

        assert!(!core.complete_item().expect("single item should loop"));
        let snapshot = core.snapshot();
        assert_eq!(snapshot.playback_generation, generation);
        assert_eq!(snapshot.loop_index, 1);
        assert_eq!(snapshot.playback_pool_cycle, 1);
        assert_eq!(snapshot.source_media_index, 0);
        assert_eq!(snapshot.source_media_pool.len(), 1);
    }

    #[test]
    fn single_source_completion_does_not_commit_a_pending_cycle_candidate() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(true, true, false);
        core.start().expect("source should start");
        let generation = core.snapshot().playback_generation;
        core.mark_media_processing_running_with_candidate(pending_media_candidate(&core))
            .expect("media processing should start");
        core.mark_media_processing_ready(
            generation,
            "/tmp/processed-current.mp4".to_owned(),
            "b".repeat(64),
        )
        .expect("processed media should be accepted");

        assert!(!core.complete_item().expect("single item should loop"));

        let snapshot = core.snapshot();
        assert_eq!(snapshot.current_video_source.as_deref(), Some("original"));
        assert_eq!(
            snapshot.current_video_reference.as_deref(),
            Some("/tmp/source.mp4")
        );
        assert!(snapshot.current_video_sha256.is_none());
        assert_eq!(
            snapshot.pending_video_reference.as_deref(),
            Some("/tmp/processed-current.mp4")
        );
    }

    #[test]
    fn source_pool_rejects_empty_or_more_than_one_hundred_items_without_replacing_current_pool() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        let before = core.snapshot();

        assert!(core.set_source_pool(Vec::new()).is_err());
        assert!(core
            .set_source_pool(vec![source(); MAX_SOURCE_MEDIA_POOL_ITEMS + 1])
            .is_err());

        let after = core.snapshot();
        assert_eq!(after.source_media_pool, before.source_media_pool);
        assert_eq!(after.playback_generation, before.playback_generation);
    }

    #[test]
    fn playback_pool_crud_is_atomic_and_resets_the_edited_pool_to_ready() {
        let mut core = PlaybackCore::default();
        core.set_source_pool(vec![source_named("a.mp4"), source_named("b.mp4")])
            .expect("initial pool should be valid");
        core.start().expect("initial pool should start");

        core.append_source_pool(vec![source_named("c.mp4")])
            .expect("append should succeed");
        assert_eq!(
            core.snapshot()
                .source_media_pool
                .iter()
                .map(|source| source.file_name.as_str())
                .collect::<Vec<_>>(),
            vec!["a.mp4", "b.mp4", "c.mp4"]
        );

        core.set_source_pool(vec![
            source_named("c.mp4"),
            source_named("a.mp4"),
            source_named("b.mp4"),
        ])
        .expect("reorder should succeed");
        core.replace_source_at(1, source_named("replacement.mp4"))
            .expect("replace should succeed");
        core.remove_source(2).expect("remove should succeed");
        let snapshot = core.snapshot();
        assert_eq!(snapshot.playback_state, PlaybackState::Ready);
        assert_eq!(snapshot.source_media_index, 0);
        assert_eq!(snapshot.current_position_ms, 0);
        assert_eq!(
            snapshot
                .source_media_pool
                .iter()
                .map(|source| source.file_name.as_str())
                .collect::<Vec<_>>(),
            vec!["c.mp4", "replacement.mp4"]
        );
        assert_eq!(
            snapshot.source_media.expect("first source").file_name,
            "c.mp4"
        );

        core.clear_source_pool();
        let cleared = core.snapshot();
        assert!(cleared.source_media_pool.is_empty());
        assert!(cleared.source_media.is_none());
        assert_eq!(cleared.playback_state, PlaybackState::Stopped);
    }

    #[test]
    fn playback_pool_crud_rejects_invalid_counts_paths_and_indexes_without_mutation() {
        let mut core = PlaybackCore::default();
        core.set_source_pool(vec![source_named("a.mp4"), source_named("b.mp4")])
            .expect("initial pool should be valid");
        let before = core.snapshot();

        assert!(core
            .append_source_pool(vec![source(); MAX_SOURCE_MEDIA_POOL_ITEMS])
            .is_err());
        assert!(core
            .append_source_pool(vec![source_named("a.mp4")])
            .is_err());
        assert!(core
            .replace_source_at(2, source_named("replacement.mp4"))
            .is_err());
        assert!(core.replace_source_at(1, source_named("a.mp4")).is_err());
        assert!(core.remove_source(2).is_err());

        let mut empty_path = source_named("empty.mp4");
        empty_path.source_path = "  ".to_owned();
        assert!(core.replace_source_at(0, empty_path).is_err());

        assert_eq!(core.snapshot(), before);
    }

    #[cfg(windows)]
    #[test]
    fn playback_pool_paths_are_case_insensitive_on_windows() {
        let mut core = PlaybackCore::default();
        core.set_source(source_named("a.mp4"));
        let before = core.snapshot();
        let mut duplicate = source_named("a.mp4");
        duplicate.source_path = duplicate.source_path.to_ascii_uppercase();

        assert!(core.append_source_pool(vec![duplicate]).is_err());
        assert_eq!(core.snapshot(), before);
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
    fn snapshot_exposes_current_playback_position_for_control_fallback() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_playback_position(1_234);

        assert_eq!(core.snapshot().current_position_ms, 1_000);
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
        assert_eq!(snapshot.audio_stream_variant_count, 1);
        assert_eq!(snapshot.audio_processing_status, "unavailable");
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
    fn enabling_audio_preserves_active_and_pending_video_processing() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(true, false, false);
        core.video_processing_status = "ready".to_owned();
        core.video_processing_progress_percent = 100;
        core.current_video_source = Some("processed".to_owned());
        core.current_video_reference = Some("/tmp/current-video.mp4".to_owned());
        core.current_video_sha256 = Some("b".repeat(64));
        core.pending_video_reference = Some("/tmp/pending-video.mp4".to_owned());
        core.pending_video_sha256 = Some("c".repeat(64));
        let pending_video = pending_media_candidate(&core);
        core.pending_media_candidate = Some(pending_video);

        core.set_processing_switches(true, true, false);

        let snapshot = core.snapshot();
        assert_eq!(snapshot.video_processing_status, "ready");
        assert_eq!(snapshot.video_processing_progress_percent, 100);
        assert_eq!(snapshot.current_video_source.as_deref(), Some("processed"));
        assert_eq!(
            snapshot.current_video_reference.as_deref(),
            Some("/tmp/current-video.mp4")
        );
        assert_eq!(
            snapshot.pending_video_reference.as_deref(),
            Some("/tmp/pending-video.mp4")
        );
        assert_eq!(snapshot.audio_processing_status, "unavailable");
    }

    #[test]
    fn enabling_video_preserves_active_and_pending_audio_processing() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(false, true, false);
        core.audio_processing_status = "ready".to_owned();
        core.audio_processing_progress_percent = 100;
        core.current_audio_artifact_reference = Some("/tmp/current-audio.m4a".to_owned());
        core.current_audio_artifact_sha256 = Some("d".repeat(64));
        core.pending_audio_artifact_reference = Some("/tmp/pending-audio.m4a".to_owned());
        core.pending_audio_artifact_sha256 = Some("e".repeat(64));

        core.set_processing_switches(true, true, false);

        let snapshot = core.snapshot();
        assert_eq!(snapshot.audio_processing_status, "ready");
        assert_eq!(snapshot.audio_processing_progress_percent, 100);
        assert_eq!(
            snapshot.current_audio_artifact_reference.as_deref(),
            Some("/tmp/current-audio.m4a")
        );
        assert_eq!(
            snapshot.pending_audio_artifact_reference.as_deref(),
            Some("/tmp/pending-audio.m4a")
        );
        assert_eq!(snapshot.video_processing_status, "unavailable");
    }

    #[test]
    fn snapshot_reports_effective_audio_stream_variant_count() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(false, true, false);
        assert_eq!(core.snapshot().audio_stream_variant_count, 1);

        core.set_audio_stream_configuration(
            AudioEffectParams::default(),
            vec![AudioEffectParams::default(); 3],
        )
        .expect("audio stream configuration should be valid");
        assert_eq!(core.snapshot().audio_stream_variant_count, 3);

        core.set_processing_switches(false, false, false);
        assert_eq!(core.snapshot().audio_stream_variant_count, 0);
    }

    #[test]
    fn validated_audio_configuration_commits_revision_exactly_once() {
        let mut core = PlaybackCore::default();
        let revision = core.snapshot().audio_stream_revision;
        let configuration = ValidatedAudioStreamConfiguration::new(
            AudioEffectParams::default(),
            vec![AudioEffectParams::default(); 2],
        )
        .expect("configuration should be valid");

        assert_eq!(core.snapshot().audio_stream_revision, revision);
        core.commit_validated_audio_stream_configuration(configuration);
        assert_eq!(core.snapshot().audio_stream_revision, revision + 1);

        core.commit_validated_audio_stream_configuration(
            ValidatedAudioStreamConfiguration::new(
                AudioEffectParams::default(),
                vec![AudioEffectParams::default(); 2],
            )
            .expect("same configuration should remain valid"),
        );
        assert_eq!(core.snapshot().audio_stream_revision, revision + 1);
    }

    #[test]
    fn media_candidate_binds_the_revision_after_audio_configuration_commit() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(false, true, false);
        let baseline = core.snapshot();

        core.set_audio_stream_configuration(
            AudioEffectParams {
                input_gain_db: 1.0,
                ..Default::default()
            },
            Vec::new(),
        )
        .expect("changed audio configuration should commit");
        let committed_revision = core.snapshot().audio_stream_revision;
        assert_eq!(committed_revision, baseline.audio_stream_revision + 1);

        core.mark_media_processing_running_with_candidate(PendingMediaCandidateIdentity {
            plan_id: "audio-plan".to_owned(),
            sequence: 1,
            playback_generation: baseline.playback_generation,
            source_revision: committed_revision,
            target_absolute_position_ms: 8_000,
            source_start_ms: 8_000,
            output_duration_ms: 10_000,
            valid_until_absolute_position_ms: 18_000,
        })
        .expect("candidate should bind the committed audio revision");

        assert_eq!(
            core.snapshot().pending_media_source_revision,
            Some(committed_revision)
        );
    }

    #[test]
    fn snapshot_exposes_exact_committed_audio_stream_configuration() {
        let mut core = PlaybackCore::default();
        let params = AudioEffectParams {
            input_gain_db: 1.25,
            playback_speed: 1.1,
            ..Default::default()
        };
        let variants = vec![
            AudioEffectParams {
                low_eq_db: 2.0,
                ..params.clone()
            },
            AudioEffectParams {
                high_eq_db: -3.0,
                ..params.clone()
            },
        ];

        core.commit_validated_audio_stream_configuration(
            ValidatedAudioStreamConfiguration::new(params.clone(), variants.clone())
                .expect("configuration should be valid"),
        );

        let snapshot = core.snapshot();
        assert_eq!(snapshot.audio_stream_params, params);
        assert_eq!(snapshot.audio_stream_variants, variants);
    }

    #[test]
    fn uncommitted_or_invalid_audio_configuration_does_not_change_snapshot() {
        let mut core = PlaybackCore::default();
        core.set_audio_stream_configuration(
            AudioEffectParams {
                output_gain_db: -1.0,
                ..Default::default()
            },
            Vec::new(),
        )
        .expect("initial configuration should be valid");
        let before = core.snapshot();
        let _pending = ValidatedAudioStreamConfiguration::new(
            AudioEffectParams {
                input_gain_db: 2.0,
                ..Default::default()
            },
            Vec::new(),
        )
        .expect("configuration should be valid");
        let invalid = ValidatedAudioStreamConfiguration::new(
            AudioEffectParams {
                playback_speed: 0.0,
                ..Default::default()
            },
            Vec::new(),
        );

        assert!(invalid.is_err());
        let after = core.snapshot();
        assert_eq!(after.audio_stream_params, before.audio_stream_params);
        assert_eq!(after.audio_stream_variants, before.audio_stream_variants);
        assert_eq!(after.audio_stream_revision, before.audio_stream_revision);
    }

    #[test]
    fn stop_preserves_committed_audio_configuration_and_leaves_runtime_status() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(false, true, false);
        let params = AudioEffectParams {
            output_gain_db: 1.5,
            ..Default::default()
        };
        let variants = vec![params.clone()];
        core.set_audio_stream_configuration(params.clone(), variants.clone())
            .expect("configuration should be valid");
        core.mark_audio_processing_runtime();
        assert_eq!(core.snapshot().audio_processing_status, "runtime");

        core.stop();
        let snapshot = core.snapshot();
        assert_eq!(snapshot.audio_processing_status, "unavailable");
        assert_eq!(snapshot.audio_stream_params, params);
        assert_eq!(snapshot.audio_stream_variants, variants);
    }

    #[test]
    fn changed_audio_processing_profile_invalidates_stream_revision() {
        let mut core = PlaybackCore::default();
        let revision = core.snapshot().audio_stream_revision;
        let profile = AudioProcessingProfile {
            params: AudioEffectParams {
                input_gain_db: 1.0,
                ..Default::default()
            },
            ..Default::default()
        };

        core.set_audio_processing_profile(profile.clone())
            .expect("changed profile should be valid");
        assert_eq!(core.snapshot().audio_stream_revision, revision + 1);

        core.set_audio_processing_profile(profile)
            .expect("unchanged profile should remain valid");
        assert_eq!(core.snapshot().audio_stream_revision, revision + 1);
    }

    #[test]
    fn audio_configuration_rejects_more_than_four_variants() {
        let result = ValidatedAudioStreamConfiguration::new(
            AudioEffectParams::default(),
            vec![AudioEffectParams::default(); 5],
        );

        assert!(result.is_err());
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
        assert_eq!(snapshot.audio_processing_status, "configured");

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
    fn realtime_worker_failure_keeps_the_current_variant_audio_available() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(false, false, true);
        core.current_audio_source = Some("realtime_variant".to_owned());
        core.current_audio_reference = Some("/tmp/variant.wav".to_owned());
        core.current_audio_start_at_ms = 1_000;
        core.current_audio_sha256 = Some("b".repeat(64));
        core.audio_decision = "rewrite".to_owned();

        core.fallback_audio_runtime("worker timeout");

        let snapshot = core.snapshot();
        assert_eq!(
            snapshot.current_audio_source.as_deref(),
            Some("realtime_variant")
        );
        assert_eq!(
            snapshot.current_audio_reference.as_deref(),
            Some("/tmp/variant.wav")
        );
        assert_eq!(snapshot.current_audio_start_at_ms, 1_000);
        assert_eq!(
            snapshot.current_audio_sha256.as_deref(),
            Some("b".repeat(64).as_str())
        );
        assert_eq!(snapshot.audio_decision, "rewrite");
        assert_eq!(snapshot.worker_status, "failed");
        assert_eq!(snapshot.fallback_reason.as_deref(), Some("worker timeout"));
    }

    #[test]
    fn media_processing_ready_switches_current_video_without_losing_source() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(true, true, false);
        let generation = core.snapshot().playback_generation;

        core.mark_media_processing_running_with_candidate(pending_media_candidate(&core))
            .expect("media processing should start");
        assert_eq!(core.snapshot().video_processing_status, "processing");
        assert_eq!(core.snapshot().video_processing_progress_percent, 0);
        core.mark_video_processing_progress(45);
        core.mark_video_processing_progress(20);
        assert_eq!(core.snapshot().video_processing_progress_percent, 45);
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
        assert_eq!(snapshot.video_processing_progress_percent, 100);
        assert_eq!(snapshot.audio_processing_status, "unavailable");
        assert_eq!(
            snapshot.pending_video_reference.as_deref(),
            Some("/tmp/processed.mp4")
        );
        assert_eq!(snapshot.current_video_source.as_deref(), Some("original"));
        assert_eq!(
            core.commit_media_processing_if_ready("stale-plan", 1, generation, 0, 8_000),
            MediaProcessingCommitOutcome::NotCommitted
        );
        assert_eq!(
            core.commit_media_processing_if_ready("plan-1", 2, generation, 0, 8_000),
            MediaProcessingCommitOutcome::NotCommitted
        );
        assert_eq!(
            core.commit_media_processing_if_ready("plan-1", 1, generation + 1, 0, 8_000),
            MediaProcessingCommitOutcome::NotCommitted
        );
        assert_eq!(
            core.commit_media_processing_if_ready("plan-1", 1, generation, 0, 17_999),
            MediaProcessingCommitOutcome::Committed
        );
        let snapshot = core.snapshot();
        assert!(snapshot.pending_video_reference.is_none());
        assert!(snapshot.pending_media_plan_id.is_none());
        assert_eq!(snapshot.current_media_plan_id.as_deref(), Some("plan-1"));
        assert_eq!(snapshot.current_media_sequence, Some(1));
        assert_eq!(snapshot.current_media_playback_generation, Some(generation));
        assert_eq!(snapshot.current_media_source_revision, Some(0));
        assert_eq!(
            snapshot.current_media_target_absolute_position_ms,
            Some(8_000)
        );
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
    fn media_candidate_commit_obeys_its_absolute_time_window() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(true, false, false);
        let candidate = pending_media_candidate(&core);
        core.mark_media_processing_running_with_candidate(candidate.clone())
            .expect("media processing should start");
        core.mark_media_processing_ready_for_candidate(
            &candidate,
            "/tmp/processed.mp4".to_owned(),
            "b".repeat(64),
        )
        .expect("candidate should become ready");

        assert_eq!(
            core.commit_media_processing_if_ready(
                &candidate.plan_id,
                candidate.sequence,
                candidate.playback_generation,
                candidate.source_revision,
                candidate.target_absolute_position_ms - 1,
            ),
            MediaProcessingCommitOutcome::NotCommitted
        );
        assert_eq!(core.snapshot().video_processing_status, "ready");

        assert_eq!(
            core.commit_media_processing_if_ready(
                &candidate.plan_id,
                candidate.sequence,
                candidate.playback_generation,
                candidate.source_revision,
                candidate.valid_until_absolute_position_ms,
            ),
            MediaProcessingCommitOutcome::Expired {
                artifact_path: Some("/tmp/processed.mp4".to_owned()),
            }
        );
        let snapshot = core.snapshot();
        assert_eq!(snapshot.video_processing_status, "failed");
        assert_eq!(snapshot.video_processing_progress_percent, 0);
        assert!(snapshot.pending_video_reference.is_none());
        assert!(snapshot.pending_media_plan_id.is_none());
        assert_eq!(
            snapshot.fallback_reason.as_deref(),
            Some("候选视频已超过有效媒体时间窗口，跳过本轮并准备下一轮")
        );
    }

    #[test]
    fn media_candidate_discard_requires_exact_identity_and_returns_ready_artifact() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(true, true, false);
        let generation = core.snapshot().playback_generation;
        let candidate = pending_media_candidate(&core);
        core.mark_media_processing_running_with_candidate(candidate.clone())
            .expect("media processing should start");
        core.mark_media_processing_ready_for_candidate(
            &candidate,
            "/tmp/processed.mp4".to_owned(),
            "b".repeat(64),
        )
        .expect("candidate should become ready");

        assert!(core
            .discard_media_processing_candidate(
                "stale-plan",
                candidate.sequence,
                generation,
                candidate.source_revision,
                "standby failed",
            )
            .is_none());
        assert_eq!(
            core.snapshot().pending_video_reference.as_deref(),
            Some("/tmp/processed.mp4")
        );

        assert_eq!(
            core.discard_media_processing_candidate(
                &candidate.plan_id,
                candidate.sequence,
                generation,
                candidate.source_revision,
                "standby failed",
            )
            .as_deref(),
            Some("/tmp/processed.mp4")
        );
        let snapshot = core.snapshot();
        assert!(snapshot.pending_video_reference.is_none());
        assert!(snapshot.pending_media_plan_id.is_none());
        assert_eq!(snapshot.video_processing_status, "failed");
        assert_eq!(snapshot.fallback_reason.as_deref(), Some("standby failed"));
    }

    #[test]
    fn processing_candidates_ignore_discard_until_the_worker_finishes() {
        let mut video = PlaybackCore::default();
        video.set_source(source());
        video.set_processing_switches(true, false, false);
        let video_candidate = pending_media_candidate(&video);
        video
            .mark_media_processing_running_with_candidate(video_candidate.clone())
            .expect("video candidate should start");

        assert!(video
            .discard_media_processing_candidate(
                &video_candidate.plan_id,
                video_candidate.sequence,
                video_candidate.playback_generation,
                video_candidate.source_revision,
                "deadline missed",
            )
            .is_none());
        let snapshot = video.snapshot();
        assert_eq!(snapshot.video_processing_status, "processing");
        assert_eq!(
            snapshot.pending_media_plan_id.as_deref(),
            Some(video_candidate.plan_id.as_str())
        );

        let mut audio = PlaybackCore::default();
        audio.set_source(source());
        audio.set_processing_switches(false, true, false);
        let generation = audio.snapshot().playback_generation;
        let audio_candidate = audio
            .mark_audio_media_processing_running(
                PendingAudioMediaCandidateIdentity {
                    plan_id: "audio-plan-1".to_owned(),
                    sequence: 1,
                    playback_generation: generation,
                    source_revision: audio.snapshot().audio_stream_revision,
                    target_absolute_position_ms: 3_000,
                    source_start_ms: 3_000,
                    output_duration_ms: 5_500,
                    valid_until_absolute_position_ms: 8_000,
                },
                ValidatedAudioStreamConfiguration::new(AudioEffectParams::default(), Vec::new())
                    .expect("audio configuration should be valid"),
            )
            .expect("audio candidate should start");

        assert!(audio
            .discard_audio_media_candidate(
                &audio_candidate.plan_id,
                audio_candidate.sequence,
                audio_candidate.playback_generation,
                audio_candidate.source_revision,
                "deadline missed",
            )
            .is_none());
        let snapshot = audio.snapshot();
        assert_eq!(snapshot.audio_processing_status, "processing");
        assert_eq!(
            snapshot.pending_audio_media_plan_id.as_deref(),
            Some(audio_candidate.plan_id.as_str())
        );
    }

    #[test]
    fn audio_media_candidate_commits_and_discards_by_exact_identity() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(false, true, false);
        let generation = core.snapshot().playback_generation;
        let next_audio = AudioEffectParams {
            input_gain_db: 1.0,
            ..Default::default()
        };
        let configuration = ValidatedAudioStreamConfiguration::new(next_audio, Vec::new())
            .expect("audio configuration should be valid");
        let candidate = PendingAudioMediaCandidateIdentity {
            plan_id: "audio-plan-1".to_owned(),
            sequence: 1,
            playback_generation: generation,
            source_revision: core.snapshot().audio_stream_revision,
            target_absolute_position_ms: 3_000,
            source_start_ms: 3_000,
            output_duration_ms: 5_500,
            valid_until_absolute_position_ms: 8_000,
        };

        let candidate = core
            .mark_audio_media_processing_running(candidate, configuration.clone())
            .expect("audio candidate should start");
        assert_eq!(candidate.source_revision, 1);
        assert_eq!(core.snapshot().audio_stream_revision, 0);
        assert_eq!(core.snapshot().audio_stream_params.input_gain_db, 0.0);
        core.mark_audio_media_processing_progress(42);
        core.mark_audio_media_processing_ready(
            &candidate,
            "/tmp/processed-audio.m4a".to_owned(),
            "c".repeat(64),
        )
        .expect("audio candidate should become ready");
        assert!(!core.commit_audio_media_candidate("stale", 1, generation, 0));
        assert!(core.commit_audio_media_candidate("audio-plan-1", 1, generation, 1));

        let snapshot = core.snapshot();
        assert_eq!(snapshot.audio_processing_status, "runtime");
        assert_eq!(snapshot.audio_stream_revision, 1);
        assert_eq!(snapshot.audio_stream_params.input_gain_db, 1.0);
        assert_eq!(snapshot.audio_processing_progress_percent, 100);
        assert_eq!(
            snapshot.current_audio_artifact_reference.as_deref(),
            Some("/tmp/processed-audio.m4a")
        );
        assert_eq!(
            snapshot.current_audio_media_plan_id.as_deref(),
            Some("audio-plan-1")
        );
        assert!(snapshot.pending_audio_artifact_reference.is_none());

        let next = PendingAudioMediaCandidateIdentity {
            plan_id: "audio-plan-2".to_owned(),
            sequence: 2,
            ..candidate
        };
        let next = core
            .mark_audio_media_processing_running(next, configuration)
            .expect("next audio candidate should start");
        core.mark_audio_media_processing_ready(
            &next,
            "/tmp/processed-audio-2.m4a".to_owned(),
            "d".repeat(64),
        )
        .expect("next audio candidate should become ready");
        assert!(core
            .discard_audio_media_candidate("wrong", 2, generation, 1, "preload failed")
            .is_none());
        assert_eq!(
            core.discard_audio_media_candidate("audio-plan-2", 2, generation, 1, "preload failed",)
                .as_deref(),
            Some("/tmp/processed-audio-2.m4a")
        );
        assert_eq!(core.snapshot().audio_processing_status, "failed");
    }

    #[test]
    fn committed_audio_revision_does_not_invalidate_independent_video_candidate() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(true, true, false);
        let video_candidate = pending_media_candidate(&core);
        core.mark_media_processing_running_with_candidate(video_candidate.clone())
            .expect("video candidate should start");

        let audio_configuration = ValidatedAudioStreamConfiguration::new(
            AudioEffectParams {
                input_gain_db: 1.0,
                ..Default::default()
            },
            Vec::new(),
        )
        .expect("audio configuration should be valid");
        let audio_candidate = core
            .mark_audio_media_processing_running(
                PendingAudioMediaCandidateIdentity {
                    plan_id: "audio-independent".to_owned(),
                    sequence: 1,
                    playback_generation: video_candidate.playback_generation,
                    source_revision: video_candidate.source_revision,
                    target_absolute_position_ms: 3_000,
                    source_start_ms: 3_000,
                    output_duration_ms: 5_500,
                    valid_until_absolute_position_ms: 8_000,
                },
                audio_configuration,
            )
            .expect("audio candidate should start");
        core.mark_audio_media_processing_ready(
            &audio_candidate,
            "/tmp/processed-audio-independent.m4a".to_owned(),
            "d".repeat(64),
        )
        .expect("audio candidate should become ready");
        assert!(core.commit_audio_media_candidate(
            &audio_candidate.plan_id,
            audio_candidate.sequence,
            audio_candidate.playback_generation,
            audio_candidate.source_revision,
        ));

        core.mark_media_processing_ready_for_candidate(
            &video_candidate,
            "/tmp/processed-video-independent.mp4".to_owned(),
            "e".repeat(64),
        )
        .expect("audio commit must not invalidate video candidate");
        assert_eq!(
            core.commit_media_processing_if_ready(
                &video_candidate.plan_id,
                video_candidate.sequence,
                video_candidate.playback_generation,
                video_candidate.source_revision,
                video_candidate.target_absolute_position_ms,
            ),
            MediaProcessingCommitOutcome::Committed
        );
    }

    #[test]
    fn stop_clears_cancelled_media_failure() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(true, true, false);
        core.mark_media_processing_running()
            .expect("media processing should start");
        core.mark_media_processing_failed("媒体处理已取消");

        core.stop();

        let snapshot = core.snapshot();
        assert_eq!(snapshot.playback_state, PlaybackState::Stopped);
        assert_eq!(snapshot.video_processing_status, "unavailable");
        assert_eq!(snapshot.audio_processing_status, "unavailable");
        assert_eq!(snapshot.fallback_reason, None);
    }

    #[test]
    fn ordinary_audio_processing_waits_for_ffmpeg_cache_even_with_realtime_variant_enabled() {
        let mut core = PlaybackCore::default();
        core.set_source(source());
        core.set_processing_switches(false, true, true);
        let snapshot = core.snapshot();
        assert_eq!(snapshot.audio_processing_status, "unavailable");
        assert!(!snapshot.audio_processing_runtime);

        let mut profile = AudioProcessingProfile::default();
        profile.params.pitch_shift_semitones = 0.5;
        core.set_audio_processing_profile(profile)
            .expect("validated profile should be accepted");
        let snapshot = core.snapshot();
        assert_eq!(snapshot.audio_processing_status, "configured");
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
        assert_eq!(snapshot.video_processing_progress_percent, 0);
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
        let snapshot = core.snapshot();
        assert!(!snapshot.pending_audio_candidate);
        assert_eq!(snapshot.video_processing_status, "disabled");
        assert_eq!(snapshot.audio_processing_status, "disabled");
        assert_eq!(snapshot.fallback_reason, None);
    }
}
