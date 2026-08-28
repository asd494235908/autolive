use autolive_desktop_core::audio_mixer::AudioMixerTask;
use autolive_desktop_core::{PlaybackSnapshot, PlaybackState, ValidatedAudioStreamConfiguration};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AudioMixerSourceIdentity {
    pub(crate) playback_generation: u64,
    pub(crate) source_path: Option<String>,
    pub(crate) current_audio_source: Option<String>,
    pub(crate) current_audio_reference: Option<String>,
    pub(crate) current_audio_start_at_ms: u64,
    pub(crate) audio_processing_enabled: bool,
    pub(crate) audio_stream_revision: u64,
}

impl AudioMixerSourceIdentity {
    pub(crate) fn matches_playing(&self, snapshot: &PlaybackSnapshot) -> bool {
        snapshot.playback_state == PlaybackState::Playing
            && snapshot.playback_generation == self.playback_generation
            && snapshot
                .source_media
                .as_ref()
                .map(|source| source.source_path.clone())
                == self.source_path
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
            source_path: snapshot
                .source_media
                .as_ref()
                .map(|source| source.source_path.clone()),
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
    pub(crate) observed_absolute_position_ms: u64,
    pub(crate) candidate_start_absolute_position_ms: u64,
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

    pub(crate) fn stop_preserving_output(&mut self) {
        self.task.stop_preserving_output();
    }
}

#[cfg(test)]
mod tests {
    use super::AudioMixerSourceIdentity;
    use autolive_desktop_core::media_library::{MediaCompatibilityMode, MediaKind, SourceMediaDto};
    use autolive_desktop_core::PlaybackCore;

    fn video_source() -> SourceMediaDto {
        SourceMediaDto {
            source_path: "/tmp/source.mp4".to_owned(),
            playback_reference: "/tmp/source.mp4".to_owned(),
            media_kind: MediaKind::Video,
            compatibility_mode: MediaCompatibilityMode::Direct,
            file_name: "source.mp4".to_owned(),
            duration_ms: Some(60_000),
            audio_start_ms: Some(0),
            audio_end_ms: Some(60_000),
            width: Some(1920),
            height: Some(1080),
            frame_rate_fps: Some(30.0),
            video_codec_name: Some("h264".to_owned()),
            audio_codec_name: Some("aac".to_owned()),
            audio_sample_rate_hz: Some(48_000),
            audio_channel_count: Some(2),
            file_size_bytes: 1_024,
            mp4_sha256: None,
            mp4_hash_status: "disabled".to_owned(),
        }
    }

    #[test]
    fn video_cache_switch_does_not_invalidate_an_audio_cycle_candidate() {
        let mut playback = PlaybackCore::default();
        playback.set_source(video_source());
        playback.set_processing_switches(true, true, false);
        playback.start().expect("source should start");
        let generation = playback.snapshot().playback_generation;
        playback
            .mark_media_processing_running()
            .expect("first render should start");
        playback
            .mark_media_processing_ready(
                generation,
                "/tmp/processed-a.mp4".to_owned(),
                "a".repeat(64),
            )
            .expect("first render should commit");
        let identity = AudioMixerSourceIdentity::from(&playback.snapshot());

        playback
            .mark_media_processing_running()
            .expect("replacement render should start");
        playback
            .mark_media_processing_ready(
                generation,
                "/tmp/processed-b.mp4".to_owned(),
                "b".repeat(64),
            )
            .expect("replacement render should commit");

        assert!(identity.matches_playing(&playback.snapshot()));
    }
}
