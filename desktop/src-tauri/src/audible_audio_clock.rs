//! PortAudio 实际可听 PTS 的进程内只读事实源。
//!
//! 音频 callback 只维护底层无锁计数；普通输出线程把计数、实际采样率和 DAC 延迟
//! 换算为快照。视频运行时只能读取匹配当前播放身份且足够新鲜的快照。

use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use crate::media_timeline::MediaSegmentIdentity;

pub const AUDIBLE_AUDIO_CLOCK_MAX_AGE: Duration = Duration::from_millis(160);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioClockBoundary {
    Startup,
    SourceChanged,
    UserSeek,
    LoopBoundary,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioClockIdentity {
    pub segment: MediaSegmentIdentity,
    pub audio_epoch: u64,
}

impl Deref for AudioClockIdentity {
    type Target = MediaSegmentIdentity;

    fn deref(&self) -> &Self::Target {
        &self.segment
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoClockIdentity {
    pub playback_generation: u64,
    pub source_path: PathBuf,
    pub loop_index: u64,
}

#[derive(Debug, Clone)]
pub struct AudibleAudioPtsSnapshot {
    pub identity: AudioClockIdentity,
    pub boundary: AudioClockBoundary,
    pub presentation_anchor_pts_ms: u64,
    pub source_anchor_pts_ms: u64,
    pub source_anchor_callback_frame: u64,
    pub audible_callback_frame: u64,
    pub actual_sample_rate_hz: u32,
    pub output_latency_us: u64,
    pub playback_rate: f64,
    pub audible_presentation_pts_ms: u64,
    pub audible_source_pts_ms: u64,
    pub playing: bool,
    pub observed_at: Instant,
}

#[derive(Debug, Clone)]
pub struct AudibleAudioClockObservation {
    pub identity: AudioClockIdentity,
    pub boundary: AudioClockBoundary,
    pub presentation_anchor_pts_ms: u64,
    pub source_anchor_pts_ms: u64,
    pub source_anchor_callback_frame: u64,
    pub callback_pcm_frames_total: u64,
    pub actual_sample_rate_hz: u32,
    pub output_latency_us: u64,
    pub playback_rate: f64,
    pub playing: bool,
    pub observed_at: Instant,
}

#[derive(Debug, Default)]
pub struct AudibleAudioClock {
    next_epoch: AtomicU64,
    snapshot: RwLock<Option<AudibleAudioPtsSnapshot>>,
}

impl AudibleAudioClock {
    pub fn next_audio_epoch(&self) -> u64 {
        self.next_epoch
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1)
    }

    pub fn publish(&self, observation: AudibleAudioClockObservation) {
        let snapshot = audible_snapshot(observation);
        match self.snapshot.write() {
            Ok(mut current) => *current = snapshot,
            Err(poisoned) => *poisoned.into_inner() = None,
        }
    }

    pub fn clear(&self) {
        match self.snapshot.write() {
            Ok(mut current) => *current = None,
            Err(poisoned) => *poisoned.into_inner() = None,
        }
    }

    pub fn snapshot(&self) -> Option<AudibleAudioPtsSnapshot> {
        match self.snapshot.read() {
            Ok(current) => current.clone(),
            Err(_) => None,
        }
    }

    pub fn snapshot_for_video(
        &self,
        expected: &VideoClockIdentity,
        now: Instant,
    ) -> Option<AudibleAudioPtsSnapshot> {
        let mut snapshot = self.snapshot()?;
        if !snapshot.playing
            || snapshot.identity.segment.playback_generation != expected.playback_generation
            || snapshot.identity.segment.loop_index != expected.loop_index
            || snapshot.identity.segment.source_path != expected.source_path
            || snapshot.observed_at > now
            || now.duration_since(snapshot.observed_at) > AUDIBLE_AUDIO_CLOCK_MAX_AGE
        {
            return None;
        }
        let age = now.duration_since(snapshot.observed_at);
        let projected_media_ms = duration_at_rate_ms(age, snapshot.playback_rate)?;
        snapshot.audible_presentation_pts_ms = snapshot
            .audible_presentation_pts_ms
            .checked_add(projected_media_ms)?;
        snapshot.audible_source_pts_ms = snapshot
            .audible_source_pts_ms
            .checked_add(projected_media_ms)?;
        if snapshot
            .identity
            .segment
            .presentation_pts_ms(snapshot.audible_source_pts_ms)
            != Some(snapshot.audible_presentation_pts_ms)
        {
            return None;
        }
        Some(snapshot)
    }

    pub fn snapshot_for_segment(
        &self,
        expected: &MediaSegmentIdentity,
        now: Instant,
    ) -> Option<AudibleAudioPtsSnapshot> {
        let snapshot = self.snapshot_for_video(
            &VideoClockIdentity {
                playback_generation: expected.playback_generation,
                source_path: expected.source_path.clone(),
                loop_index: expected.loop_index,
            },
            now,
        )?;
        (snapshot.identity.segment == *expected).then_some(snapshot)
    }
}

fn audible_snapshot(observation: AudibleAudioClockObservation) -> Option<AudibleAudioPtsSnapshot> {
    if observation.identity.audio_epoch == 0
        || observation.actual_sample_rate_hz == 0
        || !observation.playback_rate.is_finite()
        || !(0.5..=2.0).contains(&observation.playback_rate)
        || observation
            .identity
            .segment
            .presentation_pts_ms(observation.source_anchor_pts_ms)
            != Some(observation.presentation_anchor_pts_ms)
    {
        return None;
    }
    let latency_frames = observation
        .output_latency_us
        .checked_mul(u64::from(observation.actual_sample_rate_hz))?
        .checked_add(999_999)?
        / 1_000_000;
    let audible_callback_frame = observation
        .callback_pcm_frames_total
        .checked_sub(latency_frames)?;
    let elapsed_frames =
        audible_callback_frame.checked_sub(observation.source_anchor_callback_frame)?;
    let elapsed_media_ms = frames_at_rate_ms(
        elapsed_frames,
        observation.actual_sample_rate_hz,
        observation.playback_rate,
    )?;
    let audible_presentation_pts_ms = observation
        .presentation_anchor_pts_ms
        .checked_add(elapsed_media_ms)?;
    let audible_source_pts_ms = observation
        .source_anchor_pts_ms
        .checked_add(elapsed_media_ms)?;
    if observation
        .identity
        .segment
        .presentation_pts_ms(audible_source_pts_ms)
        != Some(audible_presentation_pts_ms)
    {
        return None;
    }
    Some(AudibleAudioPtsSnapshot {
        identity: observation.identity,
        boundary: observation.boundary,
        presentation_anchor_pts_ms: observation.presentation_anchor_pts_ms,
        source_anchor_pts_ms: observation.source_anchor_pts_ms,
        source_anchor_callback_frame: observation.source_anchor_callback_frame,
        audible_callback_frame,
        actual_sample_rate_hz: observation.actual_sample_rate_hz,
        output_latency_us: observation.output_latency_us,
        playback_rate: observation.playback_rate,
        audible_presentation_pts_ms,
        audible_source_pts_ms,
        playing: observation.playing,
        observed_at: observation.observed_at,
    })
}

fn frames_at_rate_ms(frames: u64, sample_rate_hz: u32, playback_rate: f64) -> Option<u64> {
    finite_non_negative_millis(frames as f64 * 1_000.0 * playback_rate / f64::from(sample_rate_hz))
}

fn duration_at_rate_ms(duration: Duration, playback_rate: f64) -> Option<u64> {
    finite_non_negative_millis(duration.as_secs_f64() * 1_000.0 * playback_rate)
}

fn finite_non_negative_millis(value: f64) -> Option<u64> {
    let rounded = value.round();
    (rounded.is_finite() && rounded >= 0.0 && rounded < u64::MAX as f64).then_some(rounded as u64)
}

#[cfg(test)]
mod tests {
    use super::{
        AudibleAudioClock, AudibleAudioClockObservation, AudioClockBoundary, AudioClockIdentity,
        VideoClockIdentity,
    };
    use crate::media_timeline::MediaSegmentIdentity;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn identity(loop_index: u64) -> AudioClockIdentity {
        AudioClockIdentity {
            segment: MediaSegmentIdentity::try_new(
                7,
                PathBuf::from(r"E:\media\source.mp4"),
                loop_index,
                72_300,
            )
            .expect("valid test segment"),
            audio_epoch: 11,
        }
    }

    fn presentation_pts_ms(loop_index: u64, source_pts_ms: u64) -> u64 {
        identity(loop_index)
            .segment
            .presentation_pts_ms(source_pts_ms)
            .expect("source PTS belongs to the test segment")
    }

    #[test]
    fn actual_callback_frames_and_dac_latency_publish_the_audible_pts() {
        let clock = AudibleAudioClock::default();
        let observed_at = Instant::now();

        clock.publish(AudibleAudioClockObservation {
            identity: identity(3),
            boundary: AudioClockBoundary::LoopBoundary,
            presentation_anchor_pts_ms: presentation_pts_ms(3, 20_000),
            source_anchor_pts_ms: 20_000,
            source_anchor_callback_frame: 1_000,
            callback_pcm_frames_total: 5_800,
            actual_sample_rate_hz: 48_000,
            output_latency_us: 20_000,
            playback_rate: 1.0,
            playing: true,
            observed_at,
        });

        let snapshot = clock
            .snapshot()
            .expect("healthy output must publish a clock");
        assert_eq!(snapshot.identity, identity(3));
        assert_eq!(snapshot.boundary, AudioClockBoundary::LoopBoundary);
        assert_eq!(snapshot.audible_callback_frame, 4_840);
        assert_eq!(snapshot.audible_source_pts_ms, 20_080);
        assert_eq!(snapshot.audible_presentation_pts_ms, 236_980);
        assert_eq!(snapshot.actual_sample_rate_hz, 48_000);
        assert_eq!(snapshot.output_latency_us, 20_000);
        assert_eq!(snapshot.observed_at, observed_at);
    }

    #[test]
    fn matching_video_snapshot_is_fail_closed_for_identity_age_and_play_state() {
        let clock = AudibleAudioClock::default();
        let observed_at = Instant::now();
        let observation = AudibleAudioClockObservation {
            identity: identity(3),
            boundary: AudioClockBoundary::Startup,
            presentation_anchor_pts_ms: presentation_pts_ms(3, 1_000),
            source_anchor_pts_ms: 1_000,
            source_anchor_callback_frame: 0,
            callback_pcm_frames_total: 4_800,
            actual_sample_rate_hz: 48_000,
            output_latency_us: 0,
            playback_rate: 1.0,
            playing: true,
            observed_at,
        };
        clock.publish(observation.clone());
        let matching = VideoClockIdentity {
            playback_generation: 7,
            source_path: PathBuf::from(r"E:\media\source.mp4"),
            loop_index: 3,
        };

        assert!(clock
            .snapshot_for_video(&matching, observed_at + Duration::from_millis(160))
            .is_some());
        assert!(clock
            .snapshot_for_video(&matching, observed_at + Duration::from_millis(161))
            .is_none());
        assert!(clock
            .snapshot_for_video(
                &VideoClockIdentity {
                    playback_generation: 8,
                    ..matching.clone()
                },
                observed_at,
            )
            .is_none());
        assert!(clock
            .snapshot_for_video(
                &VideoClockIdentity {
                    loop_index: 4,
                    ..matching.clone()
                },
                observed_at,
            )
            .is_none());
        assert!(clock
            .snapshot_for_video(
                &VideoClockIdentity {
                    source_path: PathBuf::from(r"E:\media\other.mp4"),
                    ..matching.clone()
                },
                observed_at,
            )
            .is_none());
        assert!(clock
            .snapshot_for_video(&matching, observed_at - Duration::from_millis(1))
            .is_none());

        clock.publish(AudibleAudioClockObservation {
            playing: false,
            ..observation
        });
        assert!(clock.snapshot_for_video(&matching, observed_at).is_none());
    }

    #[test]
    fn matching_video_snapshot_projects_audible_pts_to_the_read_instant() {
        let clock = AudibleAudioClock::default();
        let observed_at = Instant::now();
        clock.publish(AudibleAudioClockObservation {
            identity: identity(3),
            boundary: AudioClockBoundary::None,
            presentation_anchor_pts_ms: presentation_pts_ms(3, 10_000),
            source_anchor_pts_ms: 10_000,
            source_anchor_callback_frame: 0,
            callback_pcm_frames_total: 4_800,
            actual_sample_rate_hz: 48_000,
            output_latency_us: 0,
            playback_rate: 1.25,
            playing: true,
            observed_at,
        });
        let matching = VideoClockIdentity {
            playback_generation: 7,
            source_path: PathBuf::from(r"E:\media\source.mp4"),
            loop_index: 3,
        };

        let snapshot = clock
            .snapshot_for_video(&matching, observed_at + Duration::from_millis(100))
            .expect("fresh matching clock should be projected");

        assert_eq!(snapshot.audible_source_pts_ms, 10_250);
        assert_eq!(snapshot.audible_presentation_pts_ms, 227_150);
    }

    #[test]
    fn audio_epochs_are_process_wide_monotonic_and_clear_removes_authority() {
        let clock = AudibleAudioClock::default();
        assert_eq!(clock.next_audio_epoch(), 1);
        assert_eq!(clock.next_audio_epoch(), 2);

        clock.publish(AudibleAudioClockObservation {
            identity: identity(0),
            boundary: AudioClockBoundary::Startup,
            presentation_anchor_pts_ms: 0,
            source_anchor_pts_ms: 0,
            source_anchor_callback_frame: 0,
            callback_pcm_frames_total: 0,
            actual_sample_rate_hz: 48_000,
            output_latency_us: 0,
            playback_rate: 1.0,
            playing: true,
            observed_at: Instant::now(),
        });
        assert!(clock.snapshot().is_some());
        clock.clear();
        assert!(clock.snapshot().is_none());
    }

    #[test]
    fn invalid_sample_rate_or_playback_rate_cannot_publish_authority() {
        let clock = AudibleAudioClock::default();
        let base = AudibleAudioClockObservation {
            identity: identity(0),
            boundary: AudioClockBoundary::Startup,
            presentation_anchor_pts_ms: 0,
            source_anchor_pts_ms: 0,
            source_anchor_callback_frame: 0,
            callback_pcm_frames_total: 0,
            actual_sample_rate_hz: 48_000,
            output_latency_us: 0,
            playback_rate: 1.0,
            playing: true,
            observed_at: Instant::now(),
        };

        clock.publish(AudibleAudioClockObservation {
            actual_sample_rate_hz: 0,
            ..base.clone()
        });
        assert!(clock.snapshot().is_none());
        clock.publish(AudibleAudioClockObservation {
            playback_rate: f64::NAN,
            ..base.clone()
        });
        assert!(clock.snapshot().is_none());
        for playback_rate in [0.49, 2.01] {
            clock.publish(AudibleAudioClockObservation {
                playback_rate,
                ..base.clone()
            });
            assert!(clock.snapshot().is_none());
        }
    }

    #[test]
    fn mismatched_anchor_or_segment_duration_cannot_authorize_video_sync() {
        let clock = AudibleAudioClock::default();
        let observed_at = Instant::now();
        clock.publish(AudibleAudioClockObservation {
            identity: identity(20),
            boundary: AudioClockBoundary::LoopBoundary,
            presentation_anchor_pts_ms: 27_000,
            source_anchor_pts_ms: 27_000,
            source_anchor_callback_frame: 0,
            callback_pcm_frames_total: 0,
            actual_sample_rate_hz: 48_000,
            output_latency_us: 0,
            playback_rate: 1.0,
            playing: true,
            observed_at,
        });
        assert!(clock.snapshot().is_none());

        clock.publish(AudibleAudioClockObservation {
            identity: identity(20),
            boundary: AudioClockBoundary::LoopBoundary,
            presentation_anchor_pts_ms: presentation_pts_ms(20, 27_000),
            source_anchor_pts_ms: 27_000,
            source_anchor_callback_frame: 0,
            callback_pcm_frames_total: 0,
            actual_sample_rate_hz: 48_000,
            output_latency_us: 0,
            playback_rate: 1.0,
            playing: true,
            observed_at,
        });
        let other_duration =
            MediaSegmentIdentity::try_new(7, PathBuf::from(r"E:\media\source.mp4"), 20, 6_600)
                .expect("valid alternate segment");
        assert!(clock
            .snapshot_for_segment(&other_duration, observed_at)
            .is_none());
    }

    #[test]
    fn audible_clock_stops_at_the_segment_end_instead_of_wrapping() {
        let clock = AudibleAudioClock::default();
        clock.publish(AudibleAudioClockObservation {
            identity: identity(0),
            boundary: AudioClockBoundary::None,
            presentation_anchor_pts_ms: 72_250,
            source_anchor_pts_ms: 72_250,
            source_anchor_callback_frame: 0,
            callback_pcm_frames_total: 4_800,
            actual_sample_rate_hz: 48_000,
            output_latency_us: 0,
            playback_rate: 1.0,
            playing: true,
            observed_at: Instant::now(),
        });

        assert!(clock.snapshot().is_none());
    }
}
