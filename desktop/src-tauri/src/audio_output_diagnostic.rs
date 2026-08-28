//! 从最终混音 PCM 提取有界的低频诊断快照。

use crate::audio_feature_analysis::{AudioFeatureAnalyzer, MAX_MFCC_DIMENSIONS};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const LOW_FREQUENCY_DIAGNOSTIC_POINTS: usize = 96;
pub const LOW_FREQUENCY_DIAGNOSTIC_CUTOFF_HZ: f32 = 180.0;
const DIAGNOSTIC_WINDOW_MS: u32 = 120;
const FEATURE_ANALYSIS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq)]
pub struct AudioLowFrequencyDiagnosticSnapshot {
    pub sequence: u64,
    pub captured_at_ms: u64,
    pub sample_rate_hz: u32,
    pub captured_frame_count: u64,
    pub line: [f32; LOW_FREQUENCY_DIAGNOSTIC_POINTS],
    pub rms_dbfs: f32,
    pub peak_dbfs: f32,
    pub low_band_rms_dbfs: f32,
    pub cutoff_hz: f32,
    pub mfcc: Vec<f32>,
    pub mfcc_available: bool,
    pub noise_floor_dbfs: f32,
    pub snr_db: Option<f32>,
    pub formants_hz: [Option<f32>; 3],
    pub current_formant_hz: Option<f32>,
    pub has_pcm: bool,
}

impl AudioLowFrequencyDiagnosticSnapshot {
    pub fn empty(sample_rate_hz: u32) -> Self {
        Self {
            sequence: 0,
            captured_at_ms: 0,
            sample_rate_hz: sample_rate_hz.max(1),
            captured_frame_count: 0,
            line: [0.0; LOW_FREQUENCY_DIAGNOSTIC_POINTS],
            rms_dbfs: -140.0,
            peak_dbfs: -140.0,
            low_band_rms_dbfs: -140.0,
            cutoff_hz: LOW_FREQUENCY_DIAGNOSTIC_CUTOFF_HZ,
            mfcc: vec![0.0; usize::from(MAX_MFCC_DIMENSIONS)],
            mfcc_available: false,
            noise_floor_dbfs: -140.0,
            snr_db: None,
            formants_hz: [None; 3],
            current_formant_hz: None,
            has_pcm: false,
        }
    }
}

#[derive(Debug)]
pub struct LowFrequencyDiagnosticAnalyzer {
    sample_rate_hz: u32,
    frames_per_point: u32,
    lowpass_alpha: f32,
    line: [f32; LOW_FREQUENCY_DIAGNOSTIC_POINTS],
    line_write_index: usize,
    line_points: usize,
    point_frame_count: u32,
    point_sum: f32,
    total_frames: u64,
    window_frame_count: u64,
    sum_square: f64,
    low_sum_square: f64,
    peak: f32,
    lowpass: f32,
    sequence: u64,
    published_frame_count: u64,
    last_pcm_at_ms: u64,
    feature_analyzer: AudioFeatureAnalyzer,
    cached_features: crate::audio_feature_analysis::AudioFeatureSnapshot,
    feature_dirty: bool,
    last_feature_analysis_at: Option<Instant>,
    #[cfg(test)]
    feature_analysis_count: u64,
}

impl LowFrequencyDiagnosticAnalyzer {
    pub fn new(sample_rate_hz: u32) -> Self {
        let sample_rate_hz = sample_rate_hz.max(1);
        let frames_per_point = (u64::from(sample_rate_hz)
            .saturating_mul(u64::from(DIAGNOSTIC_WINDOW_MS))
            .saturating_div(1_000)
            .saturating_div(LOW_FREQUENCY_DIAGNOSTIC_POINTS as u64)
            .max(1)) as u32;
        let normalized_cutoff =
            (LOW_FREQUENCY_DIAGNOSTIC_CUTOFF_HZ / sample_rate_hz as f32).clamp(0.000_001, 0.49);
        let lowpass_alpha = 1.0 - (-std::f32::consts::TAU * normalized_cutoff).exp();
        let mut feature_analyzer =
            AudioFeatureAnalyzer::new_stereo_output(sample_rate_hz, MAX_MFCC_DIMENSIONS);
        let cached_features = feature_analyzer.snapshot();
        Self {
            sample_rate_hz,
            frames_per_point,
            lowpass_alpha,
            line: [0.0; LOW_FREQUENCY_DIAGNOSTIC_POINTS],
            line_write_index: 0,
            line_points: 0,
            point_frame_count: 0,
            point_sum: 0.0,
            total_frames: 0,
            window_frame_count: 0,
            sum_square: 0.0,
            low_sum_square: 0.0,
            peak: 0.0,
            lowpass: 0.0,
            sequence: 0,
            published_frame_count: 0,
            last_pcm_at_ms: 0,
            feature_analyzer,
            cached_features,
            feature_dirty: false,
            last_feature_analysis_at: None,
            #[cfg(test)]
            feature_analysis_count: 0,
        }
    }

    pub fn reset(&mut self) {
        self.line = [0.0; LOW_FREQUENCY_DIAGNOSTIC_POINTS];
        self.line_write_index = 0;
        self.line_points = 0;
        self.point_frame_count = 0;
        self.point_sum = 0.0;
        self.total_frames = 0;
        self.window_frame_count = 0;
        self.sum_square = 0.0;
        self.low_sum_square = 0.0;
        self.peak = 0.0;
        self.lowpass = 0.0;
        self.sequence = self.sequence.wrapping_add(1);
        self.published_frame_count = 0;
        self.last_pcm_at_ms = 0;
        self.feature_analyzer.reset();
        self.cached_features = self.feature_analyzer.snapshot();
        self.feature_dirty = false;
        self.last_feature_analysis_at = None;
    }

    /// 观察已经完成混音并准备写入输出环缓的交错 PCM。
    pub fn observe_stereo_pcm(&mut self, samples: &[f32]) {
        let mut captured_frames = 0_u64;
        for frame in samples.chunks_exact(2) {
            let left = finite_sample(frame[0]);
            let right = finite_sample(frame[1]);
            let mono = ((left + right) * 0.5).clamp(-1.0, 1.0);
            self.total_frames = self.total_frames.saturating_add(1);
            self.window_frame_count = self.window_frame_count.saturating_add(1);
            captured_frames = captured_frames.saturating_add(1);
            self.sum_square += f64::from(mono) * f64::from(mono);
            self.peak = self.peak.max(mono.abs());
            self.lowpass += self.lowpass_alpha * (mono - self.lowpass);
            self.low_sum_square += f64::from(self.lowpass) * f64::from(self.lowpass);
            self.point_sum += self.lowpass;
            self.point_frame_count = self.point_frame_count.saturating_add(1);
            if self.point_frame_count >= self.frames_per_point {
                let point = self.point_sum / self.point_frame_count.max(1) as f32;
                self.line[self.line_write_index] = point.clamp(-1.0, 1.0);
                self.line_write_index =
                    (self.line_write_index + 1) % LOW_FREQUENCY_DIAGNOSTIC_POINTS;
                self.line_points = self
                    .line_points
                    .saturating_add(1)
                    .min(LOW_FREQUENCY_DIAGNOSTIC_POINTS);
                self.point_frame_count = 0;
                self.point_sum = 0.0;
            }
        }
        if captured_frames > 0 {
            self.feature_analyzer.observe_interleaved_pcm(samples);
            self.feature_dirty = true;
            self.last_pcm_at_ms = unix_now_ms();
        }
    }

    pub fn snapshot(&mut self) -> AudioLowFrequencyDiagnosticSnapshot {
        self.snapshot_at(Instant::now())
    }

    fn snapshot_at(&mut self, now: Instant) -> AudioLowFrequencyDiagnosticSnapshot {
        let feature_analysis_due = self.feature_dirty
            && self
                .last_feature_analysis_at
                .is_none_or(|last_analysis_at| {
                    now.saturating_duration_since(last_analysis_at) >= FEATURE_ANALYSIS_INTERVAL
                });
        if feature_analysis_due {
            self.cached_features = self.feature_analyzer.snapshot();
            self.feature_dirty = false;
            self.last_feature_analysis_at = Some(now);
            #[cfg(test)]
            {
                self.feature_analysis_count = self.feature_analysis_count.saturating_add(1);
            }
        }
        let features = &self.cached_features;
        let mut line = [0.0; LOW_FREQUENCY_DIAGNOSTIC_POINTS];
        let scale = self
            .line
            .iter()
            .take(self.line_points)
            .map(|value| value.abs())
            .fold(0.0_f32, f32::max)
            .max(0.000_001);
        let first = if self.line_points == LOW_FREQUENCY_DIAGNOSTIC_POINTS {
            self.line_write_index
        } else {
            0
        };
        let leading_empty = LOW_FREQUENCY_DIAGNOSTIC_POINTS.saturating_sub(self.line_points);
        for (index, value) in line.iter_mut().enumerate() {
            *value = if index < leading_empty {
                0.0
            } else {
                let source_offset = index - leading_empty;
                let source_index = (first + source_offset) % LOW_FREQUENCY_DIAGNOSTIC_POINTS;
                (self.line[source_index] / scale).clamp(-1.0, 1.0)
            };
        }
        let captured_frame_count = self.window_frame_count;
        let sum_square = self.sum_square;
        let low_sum_square = self.low_sum_square;
        let peak = self.peak;
        self.window_frame_count = 0;
        self.sum_square = 0.0;
        self.low_sum_square = 0.0;
        self.peak = 0.0;
        if self.total_frames != self.published_frame_count {
            self.sequence = self.sequence.wrapping_add(1);
            self.published_frame_count = self.total_frames;
        }
        AudioLowFrequencyDiagnosticSnapshot {
            sequence: self.sequence,
            captured_at_ms: self.last_pcm_at_ms,
            sample_rate_hz: self.sample_rate_hz,
            captured_frame_count,
            line,
            rms_dbfs: dbfs(sum_square, captured_frame_count),
            peak_dbfs: amplitude_dbfs(peak),
            low_band_rms_dbfs: dbfs(low_sum_square, captured_frame_count),
            cutoff_hz: LOW_FREQUENCY_DIAGNOSTIC_CUTOFF_HZ,
            mfcc: features.mfcc.clone(),
            mfcc_available: features.mfcc_available,
            noise_floor_dbfs: features.noise_floor_dbfs,
            snr_db: features.snr_db,
            formants_hz: features.formants_hz,
            current_formant_hz: features.current_formant_hz,
            has_pcm: self.total_frames > 0,
        }
    }
}

fn finite_sample(sample: f32) -> f32 {
    if sample.is_finite() {
        sample.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

fn dbfs(sum_square: f64, frame_count: u64) -> f32 {
    if frame_count == 0 {
        return f32::NEG_INFINITY;
    }
    amplitude_dbfs((sum_square / frame_count as f64).sqrt() as f32)
}

fn amplitude_dbfs(amplitude: f32) -> f32 {
    if amplitude <= 0.000_000_1 || !amplitude.is_finite() {
        -140.0
    } else {
        (20.0 * amplitude.log10()).clamp(-140.0, 0.0)
    }
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{LowFrequencyDiagnosticAnalyzer, LOW_FREQUENCY_DIAGNOSTIC_POINTS};
    use std::time::{Duration, Instant};

    #[test]
    fn silent_pcm_snapshot_is_finite_and_bounded() {
        let mut analyzer = LowFrequencyDiagnosticAnalyzer::new(44_100);
        analyzer.observe_stereo_pcm(&[0.0; 44_100 / 10 * 2]);
        let snapshot = analyzer.snapshot();

        assert!(snapshot.has_pcm);
        assert_eq!(snapshot.line.len(), LOW_FREQUENCY_DIAGNOSTIC_POINTS);
        assert!(snapshot.line.iter().all(|value| value.is_finite()));
        assert!(snapshot
            .line
            .iter()
            .all(|value| (-1.0..=1.0).contains(value)));
        assert_eq!(snapshot.rms_dbfs, -140.0);
        assert_eq!(snapshot.low_band_rms_dbfs, -140.0);
    }

    #[test]
    fn mixed_pcm_snapshot_exposes_low_frequency_energy() {
        let mut analyzer = LowFrequencyDiagnosticAnalyzer::new(1_000);
        let mut samples = Vec::new();
        for frame in 0..1_000 {
            let sample = (frame as f32 * std::f32::consts::TAU * 40.0 / 1_000.0).sin() * 0.5;
            samples.extend([sample, sample]);
        }
        analyzer.observe_stereo_pcm(&samples);
        let snapshot = analyzer.snapshot();

        assert!(snapshot.rms_dbfs < -5.0);
        assert!(snapshot.low_band_rms_dbfs < -5.0);
        assert!(snapshot.peak_dbfs > -7.0);
        assert!(snapshot.line.iter().any(|value| value.abs() > 0.5));
    }

    #[test]
    fn non_finite_pcm_does_not_escape_the_snapshot() {
        let mut analyzer = LowFrequencyDiagnosticAnalyzer::new(44_100);
        analyzer.observe_stereo_pcm(&[f32::NAN, f32::INFINITY, -2.0, 2.0]);
        let snapshot = analyzer.snapshot();

        assert!(snapshot.line.iter().all(|value| value.is_finite()));
        assert!(snapshot.rms_dbfs.is_finite());
        assert!(snapshot.peak_dbfs.is_finite());
        assert!(snapshot.low_band_rms_dbfs.is_finite());
    }

    #[test]
    fn heavy_features_require_new_pcm_and_a_250ms_interval() {
        let mut analyzer = LowFrequencyDiagnosticAnalyzer::new(48_000);
        let started = Instant::now();
        analyzer.observe_stereo_pcm(&vec![0.1; 48_000 / 10 * 2]);

        analyzer.snapshot_at(started);
        assert_eq!(analyzer.feature_analysis_count, 1);

        analyzer.observe_stereo_pcm(&vec![0.2; 48_000 / 10 * 2]);
        analyzer.snapshot_at(started + Duration::from_millis(249));
        assert_eq!(analyzer.feature_analysis_count, 1);

        analyzer.snapshot_at(started + Duration::from_millis(250));
        assert_eq!(analyzer.feature_analysis_count, 2);

        analyzer.snapshot_at(started + Duration::from_secs(1));
        assert_eq!(analyzer.feature_analysis_count, 2);
    }
}
