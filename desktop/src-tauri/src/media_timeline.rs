//! 当前媒体段内的呈现时间与源内时间转换。

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaTimelineError {
    InvalidIdentity,
    InvalidDuration,
    Overflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSegmentIdentity {
    pub playback_generation: u64,
    pub source_path: PathBuf,
    pub loop_index: u64,
    pub source_duration_ms: u64,
    pub presentation_start_ms: u64,
}

impl MediaSegmentIdentity {
    pub fn try_new(
        playback_generation: u64,
        source_path: PathBuf,
        loop_index: u64,
        source_duration_ms: u64,
    ) -> Result<Self, MediaTimelineError> {
        if playback_generation == 0 || source_path.as_os_str().is_empty() {
            return Err(MediaTimelineError::InvalidIdentity);
        }
        if source_duration_ms == 0 {
            return Err(MediaTimelineError::InvalidDuration);
        }
        let presentation_start_ms = loop_index
            .checked_mul(source_duration_ms)
            .ok_or(MediaTimelineError::Overflow)?;
        presentation_start_ms
            .checked_add(source_duration_ms - 1)
            .ok_or(MediaTimelineError::Overflow)?;
        Ok(Self {
            playback_generation,
            source_path,
            loop_index,
            source_duration_ms,
            presentation_start_ms,
        })
    }

    pub fn presentation_pts_ms(&self, source_pts_ms: u64) -> Option<u64> {
        (source_pts_ms < self.source_duration_ms)
            .then(|| self.presentation_start_ms.checked_add(source_pts_ms))?
    }

    pub fn source_pts_ms(&self, presentation_pts_ms: u64) -> Option<u64> {
        let source_pts_ms = presentation_pts_ms.checked_sub(self.presentation_start_ms)?;
        (source_pts_ms < self.source_duration_ms).then_some(source_pts_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::{MediaSegmentIdentity, MediaTimelineError};
    use std::path::PathBuf;

    fn segment(loop_index: u64, duration_ms: u64) -> MediaSegmentIdentity {
        MediaSegmentIdentity::try_new(
            9,
            PathBuf::from(r"E:\media\source.mp4"),
            loop_index,
            duration_ms,
        )
        .expect("valid segment")
    }

    #[test]
    fn loop_twenty_round_trips_presentation_and_source_pts() {
        let segment = segment(20, 72_300);

        assert_eq!(segment.presentation_start_ms, 1_446_000);
        assert_eq!(segment.presentation_pts_ms(27_000), Some(1_473_000));
        assert_eq!(segment.source_pts_ms(1_473_000), Some(27_000));
    }

    #[test]
    fn segment_rejects_outside_pts_instead_of_wrapping_or_clamping() {
        let segment = segment(20, 72_300);

        assert_eq!(segment.presentation_pts_ms(72_300), None);
        assert_eq!(segment.source_pts_ms(1_445_999), None);
        assert_eq!(segment.source_pts_ms(1_518_300), None);
    }

    #[test]
    fn invalid_identity_duration_and_arithmetic_fail_closed() {
        assert_eq!(
            MediaSegmentIdentity::try_new(0, PathBuf::from("source.mp4"), 0, 1),
            Err(MediaTimelineError::InvalidIdentity)
        );
        assert_eq!(
            MediaSegmentIdentity::try_new(1, PathBuf::new(), 0, 1),
            Err(MediaTimelineError::InvalidIdentity)
        );
        assert_eq!(
            MediaSegmentIdentity::try_new(1, PathBuf::from("source.mp4"), 0, 0),
            Err(MediaTimelineError::InvalidDuration)
        );
        assert_eq!(
            MediaSegmentIdentity::try_new(1, PathBuf::from("source.mp4"), u64::MAX, 2,),
            Err(MediaTimelineError::Overflow)
        );
    }

    #[test]
    fn different_duration_or_loop_is_a_different_segment_identity() {
        assert_ne!(segment(20, 72_300), segment(20, 6_600));
        assert_ne!(segment(20, 72_300), segment(21, 72_300));
    }
}
