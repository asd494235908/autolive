use autolive_desktop_core::audio_mixer::AudioMixerTask;
use autolive_desktop_core::{PlaybackSnapshot, PlaybackState, ValidatedAudioStreamConfiguration};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AudioMixerSourceIdentity {
    pub(crate) playback_generation: u64,
    pub(crate) loop_index: u64,
    pub(crate) source_path: Option<String>,
    pub(crate) current_video_reference: Option<String>,
    pub(crate) current_audio_source: Option<String>,
    pub(crate) current_audio_reference: Option<String>,
    pub(crate) current_audio_start_at_ms: u64,
    pub(crate) audio_processing_enabled: bool,
    pub(crate) audio_stream_revision: u64,
}

impl AudioMixerSourceIdentity {
    pub(crate) fn matches_playing(
        &self,
        snapshot: &PlaybackSnapshot,
        expected_loop_index: u64,
    ) -> bool {
        snapshot.playback_state == PlaybackState::Playing
            && snapshot.playback_generation == self.playback_generation
            && snapshot.loop_index == expected_loop_index
            && snapshot
                .source_media
                .as_ref()
                .map(|source| source.source_path.clone())
                == self.source_path
            && snapshot.current_video_reference == self.current_video_reference
            && snapshot.current_audio_source == self.current_audio_source
            && snapshot.current_audio_reference == self.current_audio_reference
            && snapshot.current_audio_start_at_ms == self.current_audio_start_at_ms
            && snapshot.audio_processing_enabled == self.audio_processing_enabled
            && snapshot.audio_stream_revision == self.audio_stream_revision
    }
}

impl From<&PlaybackSnapshot> for AudioMixerSourceIdentity {
    fn from(snapshot: &PlaybackSnapshot) -> Self {
        Self {
            playback_generation: snapshot.playback_generation,
            loop_index: snapshot.loop_index,
            source_path: snapshot
                .source_media
                .as_ref()
                .map(|source| source.source_path.clone()),
            current_video_reference: snapshot.current_video_reference.clone(),
            current_audio_source: snapshot.current_audio_source.clone(),
            current_audio_reference: snapshot.current_audio_reference.clone(),
            current_audio_start_at_ms: snapshot.current_audio_start_at_ms,
            audio_processing_enabled: snapshot.audio_processing_enabled,
            audio_stream_revision: snapshot.audio_stream_revision,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AudioCycleCandidate {
    pub(crate) candidate_id: u64,
    pub(crate) configuration: ValidatedAudioStreamConfiguration,
    pub(crate) target_loop_index: u64,
    pub(crate) target_absolute_position_ms: u64,
}

#[derive(Debug)]
pub(crate) enum PendingAudioMixerKind {
    SourceSync,
    AudioCycle(Box<AudioCycleCandidate>),
}

#[derive(Debug)]
pub(crate) struct PendingAudioMixerTask {
    pub(crate) token: u64,
    pub(crate) task: AudioMixerTask,
    pub(crate) source_identity: AudioMixerSourceIdentity,
    pub(crate) sample_rate_hz: u32,
    pub(crate) observed_position_ms: u64,
    pub(crate) candidate_start_position_ms: u64,
    pub(crate) preparation_started_at: Instant,
    pub(crate) playback_rate: f64,
    pub(crate) kind: PendingAudioMixerKind,
}

impl PendingAudioMixerTask {
    pub(crate) fn candidate_id(&self) -> Option<u64> {
        match &self.kind {
            PendingAudioMixerKind::SourceSync => None,
            PendingAudioMixerKind::AudioCycle(candidate) => Some(candidate.candidate_id),
        }
    }

    pub(crate) fn expected_loop_index(&self) -> u64 {
        match &self.kind {
            PendingAudioMixerKind::SourceSync => self.source_identity.loop_index,
            PendingAudioMixerKind::AudioCycle(candidate) => candidate.target_loop_index,
        }
    }

    pub(crate) fn stop_preserving_output(&mut self) {
        self.task.stop_preserving_output();
    }
}
