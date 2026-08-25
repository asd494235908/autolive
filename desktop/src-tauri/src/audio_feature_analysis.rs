//! 从最终混音后的有限 PCM 窗口提取普通声音只读诊断。
//!
//! 本模块不改变声音、不生成话术，也不持有播放或硬件资源。调用方应让单一
//! 输出线程拥有分析器，并在写入 PortAudio 环缓前喂入同一份 PCM。

use std::{collections::VecDeque, error::Error, fmt, sync::Arc};

use realfft::{num_complex::Complex32, RealFftPlanner, RealToComplex};
use rustdct::{DctPlanner, TransformType2And3};

pub const MIN_MFCC_DIMENSIONS: u8 = 1;
pub const MAX_MFCC_DIMENSIONS: u8 = 40;

const MIN_SAMPLE_RATE_HZ: u32 = 8_000;
const MAX_SAMPLE_RATE_HZ: u32 = 192_000;
pub(crate) const FFT_SIZE: usize = 1_024;
pub(crate) const MEL_FILTER_COUNT: usize = 40;
const ANALYSIS_WINDOW_MS: u32 = 40;
const LEVEL_HOP_MS: u32 = 10;
const LEVEL_HISTORY_LEN: usize = 64;
const MIN_FORMANT_WINDOW_MS: u32 = 20;
const MAX_LPC_ORDER: usize = 52;
const FORMANT_GRID_POINTS: usize = 512;
const MIN_FORMANT_HZ: f32 = 80.0;
const MAX_FORMANT_HZ: f32 = 5_000.0;
const MIN_FORMANT_SEPARATION_HZ: f32 = 150.0;
const SILENCE_DBFS: f32 = -140.0;
const MIN_ANALYSIS_AMPLITUDE: f64 = 1.0e-10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioFeatureAnalysisError {
    SampleRateOutOfRange(u32),
    ChannelCountOutOfRange(u16),
    MfccDimensionsOutOfRange(u8),
}

impl fmt::Display for AudioFeatureAnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SampleRateOutOfRange(value) => write!(
                formatter,
                "音频分析采样率必须在 {MIN_SAMPLE_RATE_HZ}–{MAX_SAMPLE_RATE_HZ}Hz，当前为 {value}Hz"
            ),
            Self::ChannelCountOutOfRange(value) => {
                write!(
                    formatter,
                    "音频分析只接受单声道或双声道 PCM，当前为 {value} 声道"
                )
            }
            Self::MfccDimensionsOutOfRange(value) => write!(
                formatter,
                "MFCC 维度必须在 {MIN_MFCC_DIMENSIONS}–{MAX_MFCC_DIMENSIONS}，当前为 {value}"
            ),
        }
    }
}

impl Error for AudioFeatureAnalysisError {}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioFeatureSnapshot {
    pub sequence: u64,
    pub sample_rate_hz: u32,
    pub channel_count: u16,
    pub observed_frame_count: u64,
    pub window_frame_count: u32,
    pub mfcc: Vec<f32>,
    pub mfcc_available: bool,
    pub rms_dbfs: f32,
    pub noise_floor_dbfs: f32,
    pub snr_db: Option<f32>,
    /// 按频率递增的 LPC 谱包络峰，依次兼容 F1/F2/F3 展示槽。
    pub formants_hz: [Option<f32>; 3],
    /// 旧单值字段的兼容投影：取第一个有效 LPC 候选（F1），没有候选时为空。
    pub current_formant_hz: Option<f32>,
    pub has_pcm: bool,
}

/// 有界、同步的普通声音特征分析器。
///
/// 热路径不分配内存；`snapshot` 只为返回指定维数的 MFCC 分配一个小向量。
pub struct AudioFeatureAnalyzer {
    sample_rate_hz: u32,
    channel_count: u16,
    mfcc_dimensions: u8,
    window_capacity_frames: usize,
    mono_window: VecDeque<f32>,
    observed_frame_count: u64,
    published_frame_count: u64,
    sequence: u64,
    level_hop_frames: u32,
    level_hop_frame_count: u32,
    level_hop_sum_square: f64,
    level_history: [f32; LEVEL_HISTORY_LEN],
    level_history_len: usize,
    level_history_write_index: usize,
    fft: Arc<dyn RealToComplex<f32>>,
    fft_input: Vec<f32>,
    fft_output: Vec<Complex32>,
    fft_scratch: Vec<Complex32>,
    mel_filterbank: Vec<f32>,
    mel_log_energies: Vec<f32>,
    dct: Arc<dyn TransformType2And3<f32>>,
}

impl fmt::Debug for AudioFeatureAnalyzer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AudioFeatureAnalyzer")
            .field("sample_rate_hz", &self.sample_rate_hz)
            .field("channel_count", &self.channel_count)
            .field("mfcc_dimensions", &self.mfcc_dimensions)
            .field("window_capacity_frames", &self.window_capacity_frames)
            .field("observed_frame_count", &self.observed_frame_count)
            .finish_non_exhaustive()
    }
}

impl AudioFeatureAnalyzer {
    pub fn new(
        sample_rate_hz: u32,
        channel_count: u16,
        mfcc_dimensions: u8,
    ) -> Result<Self, AudioFeatureAnalysisError> {
        if !(MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&sample_rate_hz) {
            return Err(AudioFeatureAnalysisError::SampleRateOutOfRange(
                sample_rate_hz,
            ));
        }
        if !(1..=2).contains(&channel_count) {
            return Err(AudioFeatureAnalysisError::ChannelCountOutOfRange(
                channel_count,
            ));
        }
        if !(MIN_MFCC_DIMENSIONS..=MAX_MFCC_DIMENSIONS).contains(&mfcc_dimensions) {
            return Err(AudioFeatureAnalysisError::MfccDimensionsOutOfRange(
                mfcc_dimensions,
            ));
        }

        Ok(Self::build(sample_rate_hz, channel_count, mfcc_dimensions))
    }

    /// 为内部双声道输出诊断创建不会 panic 的分析器。
    ///
    /// 输出设备的采样率仍由 PortAudio 决定；异常值只在诊断边界收敛到支持范围，
    /// 不会改变播放流本身。
    pub fn new_stereo_output(sample_rate_hz: u32, mfcc_dimensions: u8) -> Self {
        Self::build(
            sample_rate_hz.clamp(MIN_SAMPLE_RATE_HZ, MAX_SAMPLE_RATE_HZ),
            2,
            mfcc_dimensions.clamp(MIN_MFCC_DIMENSIONS, MAX_MFCC_DIMENSIONS),
        )
    }

    fn build(sample_rate_hz: u32, channel_count: u16, mfcc_dimensions: u8) -> Self {
        let window_capacity_frames = frames_for_ms(sample_rate_hz, ANALYSIS_WINDOW_MS)
            .max(FFT_SIZE)
            .min(frames_for_ms(MAX_SAMPLE_RATE_HZ, ANALYSIS_WINDOW_MS).max(FFT_SIZE));
        let mut fft_planner = RealFftPlanner::<f32>::new();
        let fft = fft_planner.plan_fft_forward(FFT_SIZE);
        let fft_input = fft.make_input_vec();
        let fft_output = fft.make_output_vec();
        let fft_scratch = fft.make_scratch_vec();
        let mel_filterbank = build_mel_filterbank(sample_rate_hz, fft_output.len());
        let mut dct_planner = DctPlanner::<f32>::new();
        let dct = dct_planner.plan_dct2(MEL_FILTER_COUNT);

        Self {
            sample_rate_hz,
            channel_count,
            mfcc_dimensions,
            window_capacity_frames,
            mono_window: VecDeque::with_capacity(window_capacity_frames),
            observed_frame_count: 0,
            published_frame_count: 0,
            sequence: 0,
            level_hop_frames: frames_for_ms(sample_rate_hz, LEVEL_HOP_MS).max(1) as u32,
            level_hop_frame_count: 0,
            level_hop_sum_square: 0.0,
            level_history: [SILENCE_DBFS; LEVEL_HISTORY_LEN],
            level_history_len: 0,
            level_history_write_index: 0,
            fft,
            fft_input,
            fft_output,
            fft_scratch,
            mel_filterbank,
            mel_log_energies: vec![0.0; MEL_FILTER_COUNT],
            dct,
        }
    }

    /// 观察与构造时声道数一致的交错 f32 PCM；不完整的尾帧留给调用方下一批重送。
    pub fn observe_interleaved_pcm(&mut self, samples: &[f32]) {
        let channel_count = usize::from(self.channel_count);
        for frame in samples.chunks_exact(channel_count) {
            let mono =
                frame.iter().copied().map(finite_sample).sum::<f32>() / self.channel_count as f32;
            if self.mono_window.len() == self.window_capacity_frames {
                self.mono_window.pop_front();
            }
            self.mono_window.push_back(mono);
            self.observed_frame_count = self.observed_frame_count.saturating_add(1);
            self.level_hop_frame_count = self.level_hop_frame_count.saturating_add(1);
            self.level_hop_sum_square += f64::from(mono) * f64::from(mono);
            if self.level_hop_frame_count >= self.level_hop_frames {
                let level = rms_dbfs(
                    self.level_hop_sum_square,
                    u64::from(self.level_hop_frame_count),
                );
                self.push_level(level);
                self.level_hop_frame_count = 0;
                self.level_hop_sum_square = 0.0;
            }
        }
    }

    pub fn reset(&mut self) {
        self.mono_window.clear();
        self.observed_frame_count = 0;
        self.published_frame_count = 0;
        self.sequence = self.sequence.wrapping_add(1);
        self.level_hop_frame_count = 0;
        self.level_hop_sum_square = 0.0;
        self.level_history = [SILENCE_DBFS; LEVEL_HISTORY_LEN];
        self.level_history_len = 0;
        self.level_history_write_index = 0;
        self.fft_input.fill(0.0);
        self.fft_output.fill(Complex32::new(0.0, 0.0));
        self.fft_scratch.fill(Complex32::new(0.0, 0.0));
        self.mel_log_energies.fill(0.0);
    }

    pub fn snapshot(&mut self) -> AudioFeatureSnapshot {
        if self.observed_frame_count != self.published_frame_count {
            self.sequence = self.sequence.wrapping_add(1);
            self.published_frame_count = self.observed_frame_count;
        }

        let window_frame_count = self.mono_window.len();
        let sum_square = self
            .mono_window
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum();
        let rms_dbfs = rms_dbfs(sum_square, window_frame_count as u64);
        let noise_floor_dbfs = self.noise_floor_dbfs(rms_dbfs);
        let snr_db = if rms_dbfs <= SILENCE_DBFS + 0.01 {
            None
        } else {
            Some((rms_dbfs - noise_floor_dbfs).clamp(0.0, 60.0))
        };

        let mfcc = self.compute_mfcc();
        let mfcc_available = mfcc.is_some();
        let mfcc = mfcc.unwrap_or_else(|| vec![0.0; usize::from(self.mfcc_dimensions)]);
        let formants_hz = lpc_formant_candidates(&self.mono_window, self.sample_rate_hz);

        AudioFeatureSnapshot {
            sequence: self.sequence,
            sample_rate_hz: self.sample_rate_hz,
            channel_count: self.channel_count,
            observed_frame_count: self.observed_frame_count,
            window_frame_count: u32::try_from(window_frame_count).unwrap_or(u32::MAX),
            mfcc,
            mfcc_available,
            rms_dbfs,
            noise_floor_dbfs,
            snr_db,
            formants_hz,
            current_formant_hz: formants_hz[0],
            has_pcm: self.observed_frame_count > 0,
        }
    }

    fn push_level(&mut self, level_dbfs: f32) {
        self.level_history[self.level_history_write_index] = finite_dbfs(level_dbfs);
        self.level_history_write_index = (self.level_history_write_index + 1) % LEVEL_HISTORY_LEN;
        self.level_history_len = self
            .level_history_len
            .saturating_add(1)
            .min(LEVEL_HISTORY_LEN);
    }

    fn noise_floor_dbfs(&self, fallback: f32) -> f32 {
        if self.level_history_len == 0 {
            return fallback;
        }
        let mut levels = self.level_history;
        levels[..self.level_history_len].sort_by(f32::total_cmp);
        let percentile_index = (self.level_history_len - 1) / 5;
        finite_dbfs(levels[percentile_index])
    }

    fn compute_mfcc(&mut self) -> Option<Vec<f32>> {
        if self.mono_window.len() < FFT_SIZE {
            return None;
        }
        let offset = self.mono_window.len() - FFT_SIZE;
        let mut previous = 0.0_f32;
        for (index, sample) in self.mono_window.iter().skip(offset).copied().enumerate() {
            let emphasized = sample - 0.97 * previous;
            previous = sample;
            let phase = std::f32::consts::TAU * index as f32 / (FFT_SIZE - 1) as f32;
            let hamming = 0.54 - 0.46 * phase.cos();
            self.fft_input[index] = emphasized * hamming;
        }
        if self
            .fft
            .process_with_scratch(
                &mut self.fft_input,
                &mut self.fft_output,
                &mut self.fft_scratch,
            )
            .is_err()
        {
            return None;
        }

        let spectrum_len = self.fft_output.len();
        for filter_index in 0..MEL_FILTER_COUNT {
            let filter_offset = filter_index * spectrum_len;
            let energy = self
                .fft_output
                .iter()
                .enumerate()
                .map(|(bin, value)| {
                    let power = value.norm_sqr() / FFT_SIZE as f32;
                    power * self.mel_filterbank[filter_offset + bin]
                })
                .sum::<f32>();
            self.mel_log_energies[filter_index] = energy.max(1.0e-12).ln();
        }
        self.dct.process_dct2(&mut self.mel_log_energies);

        let mut coefficients = Vec::with_capacity(usize::from(self.mfcc_dimensions));
        for (index, coefficient) in self
            .mel_log_energies
            .iter()
            .copied()
            .take(usize::from(self.mfcc_dimensions))
            .enumerate()
        {
            let normalized = if index == 0 {
                coefficient / (2.0 * (MEL_FILTER_COUNT as f32).sqrt())
            } else {
                coefficient / (2.0 * MEL_FILTER_COUNT as f32).sqrt()
            };
            coefficients.push(if normalized.is_finite() {
                normalized
            } else {
                0.0
            });
        }
        Some(coefficients)
    }
}

pub(crate) fn build_mel_filterbank(sample_rate_hz: u32, spectrum_len: usize) -> Vec<f32> {
    let nyquist_hz = sample_rate_hz as f32 * 0.5;
    let maximum_hz = nyquist_hz.min(20_000.0);
    let minimum_mel = hz_to_mel(20.0_f32.min(maximum_hz));
    let maximum_mel = hz_to_mel(maximum_hz);
    let mut boundaries = [0.0_f32; MEL_FILTER_COUNT + 2];
    for (index, boundary) in boundaries.iter_mut().enumerate() {
        let ratio = index as f32 / (MEL_FILTER_COUNT + 1) as f32;
        *boundary = mel_to_hz(minimum_mel + (maximum_mel - minimum_mel) * ratio);
    }

    let mut filterbank = vec![0.0; MEL_FILTER_COUNT * spectrum_len];
    for filter_index in 0..MEL_FILTER_COUNT {
        let left = boundaries[filter_index];
        let center = boundaries[filter_index + 1];
        let right = boundaries[filter_index + 2];
        for bin in 0..spectrum_len {
            let frequency_hz = bin as f32 * sample_rate_hz as f32 / FFT_SIZE as f32;
            let weight = if frequency_hz >= left && frequency_hz <= center && center > left {
                (frequency_hz - left) / (center - left)
            } else if frequency_hz > center && frequency_hz <= right && right > center {
                (right - frequency_hz) / (right - center)
            } else {
                0.0
            };
            filterbank[filter_index * spectrum_len + bin] = weight.clamp(0.0, 1.0);
        }
    }
    filterbank
}

fn lpc_formant_candidates(samples: &VecDeque<f32>, sample_rate_hz: u32) -> [Option<f32>; 3] {
    let minimum_frames = frames_for_ms(sample_rate_hz, MIN_FORMANT_WINDOW_MS);
    if samples.len() < minimum_frames {
        return [None; 3];
    }

    let mut signal = Vec::with_capacity(samples.len());
    let mut previous = 0.0_f64;
    for (index, sample) in samples.iter().copied().enumerate() {
        let sample = f64::from(finite_sample(sample));
        let emphasized = sample - 0.97 * previous;
        previous = sample;
        let phase = std::f64::consts::TAU * index as f64 / (samples.len() - 1).max(1) as f64;
        signal.push(emphasized * (0.54 - 0.46 * phase.cos()));
    }
    let signal_energy = signal.iter().map(|sample| sample * sample).sum::<f64>();
    if !signal_energy.is_finite() || signal_energy <= MIN_ANALYSIS_AMPLITUDE {
        return [None; 3];
    }

    let lpc_order = (usize::try_from(sample_rate_hz / 1_000).unwrap_or(MAX_LPC_ORDER) + 2)
        .clamp(10, MAX_LPC_ORDER)
        .min(signal.len().saturating_sub(1));
    if lpc_order < 2 {
        return [None; 3];
    }
    let mut autocorrelation = vec![0.0_f64; lpc_order + 1];
    for lag in 0..=lpc_order {
        autocorrelation[lag] = signal[lag..]
            .iter()
            .zip(&signal[..signal.len() - lag])
            .map(|(right, left)| right * left)
            .sum::<f64>()
            / signal.len() as f64;
    }
    let Some(coefficients) = levinson_durbin(&autocorrelation, lpc_order) else {
        return [None; 3];
    };

    let maximum_hz = MAX_FORMANT_HZ.min(sample_rate_hz as f32 * 0.475);
    if maximum_hz <= MIN_FORMANT_HZ {
        return [None; 3];
    }
    let frequency_step = (maximum_hz - MIN_FORMANT_HZ) / (FORMANT_GRID_POINTS - 1) as f32;
    let mut envelope = [0.0_f64; FORMANT_GRID_POINTS];
    for (index, value) in envelope.iter_mut().enumerate() {
        let frequency_hz = MIN_FORMANT_HZ + frequency_step * index as f32;
        let angular = std::f64::consts::TAU * f64::from(frequency_hz) / f64::from(sample_rate_hz);
        let mut real = 1.0_f64;
        let mut imaginary = 0.0_f64;
        for (order, coefficient) in coefficients.iter().copied().enumerate().skip(1) {
            let phase = angular * order as f64;
            real += coefficient * phase.cos();
            imaginary -= coefficient * phase.sin();
        }
        *value = 1.0 / (real * real + imaginary * imaginary).max(1.0e-18);
    }

    let mut peaks = Vec::with_capacity(FORMANT_GRID_POINTS / 2);
    for index in 1..FORMANT_GRID_POINTS - 1 {
        if envelope[index].is_finite()
            && envelope[index] > envelope[index - 1]
            && envelope[index] >= envelope[index + 1]
        {
            peaks.push((
                envelope[index],
                MIN_FORMANT_HZ + frequency_step * index as f32,
            ));
        }
    }
    peaks.sort_by(|left, right| right.0.total_cmp(&left.0));

    let mut selected = Vec::with_capacity(3);
    for (_, frequency_hz) in peaks {
        if selected.iter().all(|selected_hz: &f32| {
            (*selected_hz - frequency_hz).abs() >= MIN_FORMANT_SEPARATION_HZ
        }) {
            selected.push(frequency_hz);
            if selected.len() == 3 {
                break;
            }
        }
    }
    selected.sort_by(f32::total_cmp);
    let mut result = [None; 3];
    for (slot, frequency_hz) in result.iter_mut().zip(selected) {
        *slot = Some(frequency_hz);
    }
    result
}

fn levinson_durbin(autocorrelation: &[f64], order: usize) -> Option<Vec<f64>> {
    let mut error = *autocorrelation.first()?;
    if !error.is_finite() || error <= MIN_ANALYSIS_AMPLITUDE {
        return None;
    }
    let mut coefficients = vec![0.0_f64; order + 1];
    let mut previous = vec![0.0_f64; order + 1];
    coefficients[0] = 1.0;
    for current_order in 1..=order {
        let mut residual = autocorrelation[current_order];
        for coefficient_index in 1..current_order {
            residual += coefficients[coefficient_index]
                * autocorrelation[current_order - coefficient_index];
        }
        if !residual.is_finite() {
            return None;
        }
        let reflection = (-residual / error).clamp(-0.999, 0.999);
        previous.copy_from_slice(&coefficients);
        for coefficient_index in 1..current_order {
            coefficients[coefficient_index] = previous[coefficient_index]
                + reflection * previous[current_order - coefficient_index];
        }
        coefficients[current_order] = reflection;
        error *= 1.0 - reflection * reflection;
        if !error.is_finite() || error <= MIN_ANALYSIS_AMPLITUDE {
            break;
        }
    }
    coefficients
        .iter()
        .all(|value| value.is_finite())
        .then_some(coefficients)
}

fn frames_for_ms(sample_rate_hz: u32, duration_ms: u32) -> usize {
    usize::try_from(
        u64::from(sample_rate_hz)
            .saturating_mul(u64::from(duration_ms))
            .saturating_add(999)
            / 1_000,
    )
    .unwrap_or(usize::MAX)
}

fn finite_sample(sample: f32) -> f32 {
    if sample.is_finite() {
        sample.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

fn rms_dbfs(sum_square: f64, frame_count: u64) -> f32 {
    if frame_count == 0 || !sum_square.is_finite() {
        return SILENCE_DBFS;
    }
    amplitude_dbfs((sum_square / frame_count as f64).sqrt() as f32)
}

fn amplitude_dbfs(amplitude: f32) -> f32 {
    if !amplitude.is_finite() || amplitude <= 1.0e-7 {
        SILENCE_DBFS
    } else {
        (20.0 * amplitude.log10()).clamp(SILENCE_DBFS, 0.0)
    }
}

fn finite_dbfs(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(SILENCE_DBFS, 0.0)
    } else {
        SILENCE_DBFS
    }
}

fn hz_to_mel(frequency_hz: f32) -> f32 {
    2_595.0 * (1.0 + frequency_hz / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10.0_f32.powf(mel / 2_595.0) - 1.0)
}

#[cfg(test)]
mod tests {
    use super::{
        AudioFeatureAnalysisError, AudioFeatureAnalyzer, MAX_MFCC_DIMENSIONS, MIN_MFCC_DIMENSIONS,
    };

    #[test]
    fn silence_produces_finite_bounded_diagnostics_without_formants() {
        let mut analyzer =
            AudioFeatureAnalyzer::new(48_000, 2, 13).expect("valid analyzer configuration");
        analyzer.observe_interleaved_pcm(&vec![0.0; 48_000 / 10 * 2]);

        let snapshot = analyzer.snapshot();

        assert!(snapshot.has_pcm);
        assert!(snapshot.mfcc_available);
        assert_eq!(snapshot.mfcc.len(), 13);
        assert!(snapshot.mfcc.iter().all(|value| value.is_finite()));
        assert_eq!(snapshot.rms_dbfs, -140.0);
        assert_eq!(snapshot.noise_floor_dbfs, -140.0);
        assert_eq!(snapshot.snr_db, None);
        assert_eq!(snapshot.formants_hz, [None; 3]);
        assert_eq!(snapshot.current_formant_hz, None);
    }

    #[test]
    fn sine_analysis_is_deterministic_and_has_expected_rms() {
        let samples = stereo_sine(48_000, 440.0, 0.5, 100);
        let mut first =
            AudioFeatureAnalyzer::new(48_000, 2, 13).expect("valid analyzer configuration");
        let mut second =
            AudioFeatureAnalyzer::new(48_000, 2, 13).expect("valid analyzer configuration");
        first.observe_interleaved_pcm(&samples);
        second.observe_interleaved_pcm(&samples);

        let first = first.snapshot();
        let second = second.snapshot();

        assert!((first.rms_dbfs + 9.03).abs() < 0.2);
        assert_eq!(first.mfcc, second.mfcc);
        assert_eq!(first.formants_hz, second.formants_hz);
        assert!(first
            .formants_hz
            .iter()
            .flatten()
            .all(|value| value.is_finite() && (80.0..=5_000.0).contains(value)));
    }

    #[test]
    fn non_finite_and_out_of_range_pcm_never_escape_snapshot() {
        let mut analyzer =
            AudioFeatureAnalyzer::new(8_000, 1, 4).expect("valid analyzer configuration");
        let pattern = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -2.0, 2.0, 0.25];
        let samples: Vec<f32> = pattern.into_iter().cycle().take(2_000).collect();
        analyzer.observe_interleaved_pcm(&samples);

        let snapshot = analyzer.snapshot();

        assert!(snapshot.rms_dbfs.is_finite());
        assert!(snapshot.noise_floor_dbfs.is_finite());
        assert!(snapshot.snr_db.into_iter().all(f32::is_finite));
        assert!(snapshot.mfcc.iter().all(|value| value.is_finite()));
        assert!(snapshot
            .formants_hz
            .iter()
            .flatten()
            .all(|value| value.is_finite()));
    }

    #[test]
    fn rolling_noise_floor_yields_positive_snr_after_quiet_history() {
        let mut analyzer =
            AudioFeatureAnalyzer::new(48_000, 1, 13).expect("valid analyzer configuration");
        analyzer.observe_interleaved_pcm(&mono_sine(48_000, 170.0, 0.01, 400));
        analyzer.observe_interleaved_pcm(&mono_sine(48_000, 440.0, 0.5, 40));

        let snapshot = analyzer.snapshot();

        assert!(snapshot.noise_floor_dbfs < -35.0);
        assert!(snapshot.snr_db.is_some_and(|value| value > 25.0));
    }

    #[test]
    fn lpc_envelope_reports_three_vowel_like_formant_candidates() {
        let mut analyzer =
            AudioFeatureAnalyzer::new(48_000, 1, 13).expect("valid analyzer configuration");
        analyzer.observe_interleaved_pcm(&vowel_like_signal(48_000, 100));

        let snapshot = analyzer.snapshot();
        let formants = snapshot
            .formants_hz
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .expect("three LPC candidates");

        assert!((300.0..=800.0).contains(&formants[0]));
        assert!((1_100.0..=1_900.0).contains(&formants[1]));
        assert!((2_100.0..=3_000.0).contains(&formants[2]));
        assert_eq!(snapshot.current_formant_hz, Some(formants[0]));
    }

    #[test]
    fn constructor_enforces_mfcc_dimensions_sample_rate_and_channels() {
        assert!(AudioFeatureAnalyzer::new(48_000, 1, MIN_MFCC_DIMENSIONS).is_ok());
        assert!(AudioFeatureAnalyzer::new(48_000, 2, MAX_MFCC_DIMENSIONS).is_ok());
        assert!(matches!(
            AudioFeatureAnalyzer::new(48_000, 1, 0),
            Err(AudioFeatureAnalysisError::MfccDimensionsOutOfRange(0))
        ));
        assert!(matches!(
            AudioFeatureAnalyzer::new(48_000, 1, 41),
            Err(AudioFeatureAnalysisError::MfccDimensionsOutOfRange(41))
        ));
        assert!(matches!(
            AudioFeatureAnalyzer::new(0, 1, 13),
            Err(AudioFeatureAnalysisError::SampleRateOutOfRange(0))
        ));
        assert!(matches!(
            AudioFeatureAnalyzer::new(48_000, 3, 13),
            Err(AudioFeatureAnalysisError::ChannelCountOutOfRange(3))
        ));

        let mut normalized = AudioFeatureAnalyzer::new_stereo_output(0, 0);
        let snapshot = normalized.snapshot();
        assert_eq!(snapshot.sample_rate_hz, 8_000);
        assert_eq!(snapshot.channel_count, 2);
        assert_eq!(snapshot.mfcc.len(), 1);
    }

    fn mono_sine(
        sample_rate_hz: u32,
        frequency_hz: f32,
        amplitude: f32,
        duration_ms: u32,
    ) -> Vec<f32> {
        let frames = sample_rate_hz as usize * duration_ms as usize / 1_000;
        (0..frames)
            .map(|frame| {
                (std::f32::consts::TAU * frequency_hz * frame as f32 / sample_rate_hz as f32).sin()
                    * amplitude
            })
            .collect()
    }

    fn stereo_sine(
        sample_rate_hz: u32,
        frequency_hz: f32,
        amplitude: f32,
        duration_ms: u32,
    ) -> Vec<f32> {
        mono_sine(sample_rate_hz, frequency_hz, amplitude, duration_ms)
            .into_iter()
            .flat_map(|sample| [sample, sample])
            .collect()
    }

    fn vowel_like_signal(sample_rate_hz: u32, duration_ms: u32) -> Vec<f32> {
        let frames = sample_rate_hz as usize * duration_ms as usize / 1_000;
        let mut random_state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut signal = (0..frames)
            .map(|_| {
                random_state ^= random_state << 13;
                random_state ^= random_state >> 7;
                random_state ^= random_state << 17;
                (random_state as f64 / u64::MAX as f64 * 2.0 - 1.0) as f32
            })
            .collect::<Vec<_>>();
        for (frequency_hz, bandwidth_hz) in
            [(500.0_f32, 80.0_f32), (1_500.0, 100.0), (2_500.0, 120.0)]
        {
            let radius = (-std::f32::consts::PI * bandwidth_hz / sample_rate_hz as f32).exp();
            let feedback =
                2.0 * radius * (std::f32::consts::TAU * frequency_hz / sample_rate_hz as f32).cos();
            let damping = radius * radius;
            let mut previous = 0.0_f32;
            let mut previous_two = 0.0_f32;
            for sample in &mut signal {
                let output = *sample + feedback * previous - damping * previous_two;
                previous_two = previous;
                previous = output;
                *sample = output;
            }
            let peak = signal.iter().map(|sample| sample.abs()).fold(0.0, f32::max);
            if peak > 0.0 {
                signal.iter_mut().for_each(|sample| *sample *= 0.5 / peak);
            }
        }
        signal
    }
}
