//! 混音后 PCM 的有界特征域效果。
//!
//! MFCC 偏移在 40 个 Mel 频带上执行 DCT-II、系数位移和 DCT-III 重建，
//! 再以原相位重建 STFT；SNR 目标使用输入 PCM 的滚动 RMS 计算确定性噪声幅度。
//! 本模块不改变话术内容，也不创建模型、网络或后台任务。

use crate::audio_feature_analysis::{build_mel_filterbank, FFT_SIZE, MEL_FILTER_COUNT};
use crate::media_audio_effects::{MfccOperation, MfccRuntimePlan, SnrRuntimePlan};
use realfft::{num_complex::Complex32, ComplexToReal, RealFftPlanner, RealToComplex};
use rustdct::{DctPlanner, TransformType2And3};
use std::collections::VecDeque;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

const HOP_SIZE: usize = FFT_SIZE / 2;
const MAX_CHANNELS: usize = 2;
const MIN_SAMPLE_RATE_HZ: u32 = 8_000;
const MAX_SAMPLE_RATE_HZ: u32 = 192_000;
const MIN_GAIN: f32 = 0.25;
const MAX_GAIN: f32 = 4.0;
const MAX_INJECTED_NOISE_RMS: f32 = 0.25;
const LEVEL_HISTORY_LENGTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct AudioPcmEffectConfig {
    pub mfcc: Option<MfccRuntimePlan>,
    pub snr: Option<SnrRuntimePlan>,
}

impl AudioPcmEffectConfig {
    pub fn is_active(&self) -> bool {
        self.mfcc
            .as_ref()
            .is_some_and(|plan| plan.operation == MfccOperation::ShiftAndReconstruct)
            || self.snr.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioPcmEffectError {
    InvalidConfiguration(String),
    InvalidBuffer(&'static str),
    TransformFailed(&'static str),
}

impl Display for AudioPcmEffectError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfiguration(message) => formatter.write_str(message),
            Self::InvalidBuffer(message) | Self::TransformFailed(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for AudioPcmEffectError {}

pub struct AudioPcmEffectProcessor {
    channels: usize,
    cepstral: Option<CepstralShiftProcessor>,
    snr: Option<SnrNoiseProcessor>,
}

impl AudioPcmEffectProcessor {
    pub fn new(
        config: AudioPcmEffectConfig,
        sample_rate_hz: u32,
        channels: usize,
    ) -> Result<Self, AudioPcmEffectError> {
        validate_stream_shape(sample_rate_hz, channels)?;
        let cepstral = config
            .mfcc
            .filter(|plan| plan.operation == MfccOperation::ShiftAndReconstruct)
            .map(|plan| CepstralShiftProcessor::new(plan, sample_rate_hz, channels))
            .transpose()?;
        let snr = config
            .snr
            .map(|plan| SnrNoiseProcessor::new(plan, sample_rate_hz, channels))
            .transpose()?;
        Ok(Self {
            channels,
            cepstral,
            snr,
        })
    }

    pub fn process_interleaved(&mut self, input: &[f32]) -> Result<Vec<f32>, AudioPcmEffectError> {
        validate_interleaved(input)?;
        if !input.len().is_multiple_of(self.channels) {
            return Err(AudioPcmEffectError::InvalidBuffer(
                "PCM 特征效果输入未按声道对齐",
            ));
        }
        let mut output = match self.cepstral.as_mut() {
            Some(processor) => processor.process_interleaved(input)?,
            None => input.iter().copied().map(finite_sample).collect(),
        };
        if let Some(processor) = self.snr.as_mut() {
            processor.process_interleaved(&mut output);
        }
        if output.iter().any(|sample| !sample.is_finite()) {
            return Err(AudioPcmEffectError::TransformFailed(
                "PCM 特征效果产生了非有限样本",
            ));
        }
        Ok(output)
    }
}

struct CepstralShiftProcessor {
    channels: usize,
    shift_percent: f32,
    dimensions: usize,
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
    dct: Arc<dyn TransformType2And3<f32>>,
    analysis_window: Vec<f32>,
    mel_filterbank: Vec<f32>,
    pending: VecDeque<f32>,
    output: VecDeque<f32>,
    overlap: Vec<Vec<f32>>,
    fft_input: Vec<f32>,
    spectrum: Vec<Complex32>,
    inverse_output: Vec<f32>,
    forward_scratch: Vec<Complex32>,
    inverse_scratch: Vec<Complex32>,
    mel_log_energies: Vec<f32>,
    cepstral_delta: Vec<f32>,
}

impl CepstralShiftProcessor {
    fn new(
        plan: MfccRuntimePlan,
        sample_rate_hz: u32,
        channels: usize,
    ) -> Result<Self, AudioPcmEffectError> {
        if !plan.shift_percent.is_finite() || !(-20.0..=20.0).contains(&plan.shift_percent) {
            return Err(AudioPcmEffectError::InvalidConfiguration(
                "MFCC 偏移必须在 -20..=20%".to_owned(),
            ));
        }
        if !(1..=40).contains(&plan.dimensions) {
            return Err(AudioPcmEffectError::InvalidConfiguration(
                "MFCC 维度必须在 1..=40".to_owned(),
            ));
        }
        let mut fft_planner = RealFftPlanner::<f32>::new();
        let forward = fft_planner.plan_fft_forward(FFT_SIZE);
        let inverse = fft_planner.plan_fft_inverse(FFT_SIZE);
        let fft_input = forward.make_input_vec();
        let spectrum = forward.make_output_vec();
        let forward_scratch = forward.make_scratch_vec();
        let inverse_output = inverse.make_output_vec();
        let inverse_scratch = inverse.make_scratch_vec();
        let mut dct_planner = DctPlanner::<f32>::new();
        let dct = dct_planner.plan_dct2(MEL_FILTER_COUNT);
        let analysis_window = (0..FFT_SIZE)
            .map(|index| {
                let phase = std::f32::consts::TAU * index as f32 / FFT_SIZE as f32;
                (0.5 - 0.5 * phase.cos()).max(0.0).sqrt()
            })
            .collect();
        let mel_filterbank = build_mel_filterbank(sample_rate_hz, spectrum.len());
        let mut pending = VecDeque::with_capacity((FFT_SIZE + HOP_SIZE) * channels);
        pending.extend(std::iter::repeat_n(0.0, HOP_SIZE * channels));
        Ok(Self {
            channels,
            shift_percent: plan.shift_percent as f32,
            dimensions: usize::from(plan.dimensions),
            forward,
            inverse,
            dct,
            analysis_window,
            mel_filterbank,
            pending,
            output: std::iter::repeat_n(0.0, HOP_SIZE * channels).collect(),
            overlap: vec![vec![0.0; FFT_SIZE]; channels],
            fft_input,
            spectrum,
            inverse_output,
            forward_scratch,
            inverse_scratch,
            mel_log_energies: vec![0.0; MEL_FILTER_COUNT],
            cepstral_delta: vec![0.0; MEL_FILTER_COUNT],
        })
    }

    fn process_interleaved(&mut self, input: &[f32]) -> Result<Vec<f32>, AudioPcmEffectError> {
        if !input.len().is_multiple_of(self.channels) {
            return Err(AudioPcmEffectError::InvalidBuffer(
                "MFCC 输入 PCM 未按声道对齐",
            ));
        }
        self.pending
            .extend(input.iter().copied().map(finite_sample));
        while self.pending.len() / self.channels >= FFT_SIZE {
            for channel in 0..self.channels {
                self.process_channel(channel)?;
            }
            for frame in 0..HOP_SIZE {
                for channel in 0..self.channels {
                    self.output.push_back(self.overlap[channel][frame]);
                }
            }
            for channel in 0..self.channels {
                self.overlap[channel].copy_within(HOP_SIZE.., 0);
                self.overlap[channel][FFT_SIZE - HOP_SIZE..].fill(0.0);
            }
            self.pending.drain(..HOP_SIZE * self.channels);
        }
        // STFT 用预置的 512 frame 输入/输出静音承接固定算法延迟。每消费
        // 512 frame 就产生 512 frame，因此 FIFO 在任意分块下都有等长输出。
        if self.output.len() < input.len() {
            return Err(AudioPcmEffectError::TransformFailed(
                "MFCC 固定延迟 FIFO 出现输出缺口",
            ));
        }
        Ok(self.output.drain(..input.len()).collect())
    }

    fn process_channel(&mut self, channel: usize) -> Result<(), AudioPcmEffectError> {
        for frame in 0..FFT_SIZE {
            self.fft_input[frame] =
                self.pending[frame * self.channels + channel] * self.analysis_window[frame];
        }
        self.forward
            .process_with_scratch(
                &mut self.fft_input,
                &mut self.spectrum,
                &mut self.forward_scratch,
            )
            .map_err(|_| AudioPcmEffectError::TransformFailed("MFCC 正向 FFT 失败"))?;

        self.compute_cepstral_delta();
        let spectrum_len = self.spectrum.len();
        for (bin, value) in self.spectrum.iter_mut().enumerate() {
            let mut weighted_delta = 0.0_f32;
            let mut weight_sum = 0.0_f32;
            for band in 0..MEL_FILTER_COUNT {
                let weight = self.mel_filterbank[band * spectrum_len + bin];
                weighted_delta += self.cepstral_delta[band] * weight;
                weight_sum += weight;
            }
            if weight_sum > f32::EPSILON {
                let gain = (0.5 * weighted_delta / weight_sum)
                    .exp()
                    .clamp(MIN_GAIN, MAX_GAIN);
                *value *= gain;
            }
        }

        self.inverse
            .process_with_scratch(
                &mut self.spectrum,
                &mut self.inverse_output,
                &mut self.inverse_scratch,
            )
            .map_err(|_| AudioPcmEffectError::TransformFailed("MFCC 逆向 FFT 失败"))?;
        for frame in 0..FFT_SIZE {
            let sample = self.inverse_output[frame] / FFT_SIZE as f32 * self.analysis_window[frame];
            self.overlap[channel][frame] += finite_sample(sample);
        }
        Ok(())
    }

    fn compute_cepstral_delta(&mut self) {
        let spectrum_len = self.spectrum.len();
        for band in 0..MEL_FILTER_COUNT {
            let offset = band * spectrum_len;
            let energy = self
                .spectrum
                .iter()
                .enumerate()
                .map(|(bin, value)| value.norm_sqr() * self.mel_filterbank[offset + bin])
                .sum::<f32>();
            self.mel_log_energies[band] = energy.max(1.0e-12).ln();
        }
        self.dct.process_dct2(&mut self.mel_log_energies);
        self.cepstral_delta.fill(0.0);
        if self.dimensions <= 1 || self.shift_percent.abs() <= f32::EPSILON {
            return;
        }
        let shift = self.shift_percent / 100.0 * (self.dimensions - 1) as f32;
        for index in 1..self.dimensions {
            let source = (index as f32 - shift).clamp(1.0, (self.dimensions - 1) as f32);
            let left = source.floor() as usize;
            let right = source.ceil() as usize;
            let ratio = source - left as f32;
            let shifted =
                self.mel_log_energies[left] * (1.0 - ratio) + self.mel_log_energies[right] * ratio;
            self.cepstral_delta[index] = shifted - self.mel_log_energies[index];
        }
        self.dct.process_dct3(&mut self.cepstral_delta);
        let scale = 1.0 / (2 * MEL_FILTER_COUNT) as f32;
        self.cepstral_delta
            .iter_mut()
            .for_each(|value| *value *= scale);
    }
}

struct SnrNoiseProcessor {
    plan: SnrRuntimePlan,
    channels: usize,
    window_frames: usize,
    update_frames: usize,
    frames_since_update: usize,
    window: VecDeque<f32>,
    window_sum_square: f64,
    level_history: VecDeque<f32>,
    noise_rms: f32,
    random_state: u64,
}

impl SnrNoiseProcessor {
    fn new(
        plan: SnrRuntimePlan,
        sample_rate_hz: u32,
        channels: usize,
    ) -> Result<Self, AudioPcmEffectError> {
        let window_frames = frames_for_ms(sample_rate_hz, plan.analysis_window_ms).max(1);
        let update_frames = frames_for_ms(sample_rate_hz, plan.update_interval_ms).max(1);
        Ok(Self {
            plan,
            channels,
            window_frames,
            update_frames,
            frames_since_update: 0,
            window: VecDeque::with_capacity(window_frames),
            window_sum_square: 0.0,
            level_history: VecDeque::with_capacity(LEVEL_HISTORY_LENGTH),
            noise_rms: 0.0,
            random_state: 0x9e37_79b9_7f4a_7c15,
        })
    }

    fn process_interleaved(&mut self, samples: &mut [f32]) {
        for frame in samples.chunks_exact_mut(self.channels) {
            let mono = frame.iter().copied().sum::<f32>() / self.channels as f32;
            let square = f64::from(mono) * f64::from(mono);
            self.window.push_back(mono);
            self.window_sum_square += square;
            if self.window.len() > self.window_frames {
                if let Some(removed) = self.window.pop_front() {
                    self.window_sum_square -= f64::from(removed) * f64::from(removed);
                }
            }
            self.frames_since_update += 1;
            if self.frames_since_update >= self.update_frames {
                self.update_noise_rms();
                self.frames_since_update = 0;
            }
            let noise = self.next_white_noise() * self.noise_rms * 3.0_f32.sqrt();
            for sample in frame {
                *sample = finite_sample(*sample + noise);
            }
        }
    }

    fn update_noise_rms(&mut self) {
        if self.window.is_empty() {
            self.noise_rms = 0.0;
            return;
        }
        let signal_rms = (self.window_sum_square.max(0.0) / self.window.len() as f64).sqrt() as f32;
        let signal_dbfs = amplitude_dbfs(signal_rms);
        if self.level_history.len() == LEVEL_HISTORY_LENGTH {
            self.level_history.pop_front();
        }
        self.level_history.push_back(signal_dbfs);
        let measured_snr = measured_snr_db(&self.level_history);
        let target_db = self
            .plan
            .target_db_for_measured_source(f64::from(measured_snr)) as f32;
        let measured_noise_rms = signal_rms / 10_f32.powf(measured_snr / 20.0);
        let target_noise_rms = signal_rms / 10_f32.powf(target_db / 20.0);
        // 只注入达到更低目标 SNR 所需的“增量”噪声；目标高于当前测量值时，
        // 加噪无法改善源信噪比，因此保持旁路，避免把参数方向做反。
        self.noise_rms = (target_noise_rms * target_noise_rms
            - measured_noise_rms * measured_noise_rms)
            .max(0.0)
            .sqrt()
            .clamp(0.0, MAX_INJECTED_NOISE_RMS);
    }

    fn next_white_noise(&mut self) -> f32 {
        let mut value = self.random_state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.random_state = value;
        (value as f64 / u64::MAX as f64 * 2.0 - 1.0) as f32
    }
}

fn validate_stream_shape(sample_rate_hz: u32, channels: usize) -> Result<(), AudioPcmEffectError> {
    if !(MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&sample_rate_hz) {
        return Err(AudioPcmEffectError::InvalidConfiguration(format!(
            "PCM 特征效果采样率必须在 {MIN_SAMPLE_RATE_HZ}..={MAX_SAMPLE_RATE_HZ}Hz"
        )));
    }
    if !(1..=MAX_CHANNELS).contains(&channels) {
        return Err(AudioPcmEffectError::InvalidConfiguration(
            "PCM 特征效果只支持 1–2 声道".to_owned(),
        ));
    }
    Ok(())
}

fn validate_interleaved(samples: &[f32]) -> Result<(), AudioPcmEffectError> {
    if samples.is_empty() {
        return Err(AudioPcmEffectError::InvalidBuffer("PCM 输入不能为空"));
    }
    Ok(())
}

fn measured_snr_db(levels: &VecDeque<f32>) -> f32 {
    if levels.len() < 2 {
        return 30.0;
    }
    let mut sorted = levels.iter().copied().collect::<Vec<_>>();
    sorted.sort_by(f32::total_cmp);
    if sorted.last().copied().unwrap_or_default() - sorted[0] < 1.0 {
        // 稳态窗口没有可识别的低电平噪声片段；不能把整段信号本身当成噪声底。
        return 60.0;
    }
    let noise_floor = sorted[(sorted.len() - 1) / 5];
    let signal = sorted.iter().sum::<f32>() / sorted.len() as f32;
    (signal - noise_floor).clamp(0.0, 60.0)
}

fn frames_for_ms(sample_rate_hz: u32, duration_ms: u64) -> usize {
    u64::from(sample_rate_hz)
        .saturating_mul(duration_ms)
        .saturating_div(1_000)
        .clamp(1, usize::MAX as u64) as usize
}

fn amplitude_dbfs(amplitude: f32) -> f32 {
    if !amplitude.is_finite() || amplitude <= 1.0e-7 {
        -140.0
    } else {
        (20.0 * amplitude.log10()).clamp(-140.0, 0.0)
    }
}

fn finite_sample(sample: f32) -> f32 {
    if sample.is_finite() {
        sample.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioPcmEffectConfig, AudioPcmEffectProcessor, HOP_SIZE};
    use crate::media_audio_effects::{
        MfccOperation, MfccRuntimePlan, SnrProcessingBackend, SnrRuntimePlan, SnrTargetPlan,
    };

    fn stereo_sine(frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|frame| {
                let sample = (std::f32::consts::TAU * 440.0 * frame as f32 / 48_000.0).sin() * 0.2;
                [sample, sample]
            })
            .collect()
    }

    fn rms_db(samples: &[f32]) -> f32 {
        let mean_square =
            samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len().max(1) as f32;
        20.0 * mean_square.sqrt().max(1.0e-12).log10()
    }

    #[test]
    fn mfcc_shift_reconstructs_finite_pcm_after_bounded_latency() {
        let config = AudioPcmEffectConfig {
            mfcc: Some(MfccRuntimePlan {
                dimensions: 13,
                shift_percent: 10.0,
                operation: MfccOperation::ShiftAndReconstruct,
            }),
            snr: None,
        };
        let mut processor =
            AudioPcmEffectProcessor::new(config, 48_000, 2).expect("valid MFCC processor");
        let input = stereo_sine(4_096);
        let output = processor
            .process_interleaved(&input)
            .expect("MFCC processing");
        assert!(!output.is_empty());
        assert_eq!(output.len(), input.len());
        assert!(output.iter().all(|sample| sample.is_finite()));
        assert_ne!(&output[..128], &input[..128]);
    }

    #[test]
    fn mfcc_shift_keeps_irregular_small_chunks_equal_length() {
        let config = AudioPcmEffectConfig {
            mfcc: Some(MfccRuntimePlan {
                dimensions: 20,
                shift_percent: -8.0,
                operation: MfccOperation::ShiftAndReconstruct,
            }),
            snr: None,
        };
        let mut processor =
            AudioPcmEffectProcessor::new(config, 48_000, 2).expect("valid MFCC processor");
        let input = stereo_sine(4_096);
        let mut output = Vec::with_capacity(input.len());
        for chunk in input.chunks(256 * 2) {
            let processed = processor
                .process_interleaved(chunk)
                .expect("small MFCC chunk");
            assert_eq!(processed.len(), chunk.len());
            output.extend(processed);
        }
        assert_eq!(output.len(), input.len());
        assert!(output.iter().all(|sample| sample.is_finite()));
        assert!(output[HOP_SIZE * 2..]
            .iter()
            .any(|sample| sample.abs() > 1.0e-5));
    }

    #[test]
    fn subtle_mfcc_shift_preserves_source_rms() {
        for (dimensions, shift_percent) in [(12, -0.16), (16, 0.32)] {
            let config = AudioPcmEffectConfig {
                mfcc: Some(MfccRuntimePlan {
                    dimensions,
                    shift_percent,
                    operation: MfccOperation::ShiftAndReconstruct,
                }),
                snr: None,
            };
            let mut processor =
                AudioPcmEffectProcessor::new(config, 48_000, 2).expect("subtle MFCC processor");
            let input = stereo_sine(48_000);
            let output = processor
                .process_interleaved(&input)
                .expect("subtle MFCC processing");
            let delta_db = rms_db(&output) - rms_db(&input);

            assert!(
                delta_db.abs() <= 0.25,
                "dimensions={dimensions} shift={shift_percent}% changed RMS by {delta_db}dB"
            );
        }
    }

    #[test]
    fn fixed_snr_target_injects_bounded_finite_noise() {
        let config = AudioPcmEffectConfig {
            mfcc: None,
            snr: Some(SnrRuntimePlan {
                target: SnrTargetPlan::FixedDb(20.0),
                variation_db: 0.0,
                analysis_window_ms: 400,
                update_interval_ms: 100,
                backend: SnrProcessingBackend::PcmRollingRmsNoiseInjection,
            }),
        };
        let mut processor =
            AudioPcmEffectProcessor::new(config, 48_000, 2).expect("valid SNR processor");
        let input = stereo_sine(24_000);
        let output = processor
            .process_interleaved(&input)
            .expect("SNR processing");
        assert_eq!(output.len(), input.len());
        assert!(output
            .iter()
            .all(|sample| sample.is_finite() && sample.abs() <= 1.0));
        assert_ne!(&output[20_000..20_128], &input[20_000..20_128]);
    }

    #[test]
    fn snr_only_rejects_interleaved_pcm_with_partial_frame() {
        let config = AudioPcmEffectConfig {
            mfcc: None,
            snr: Some(SnrRuntimePlan {
                target: SnrTargetPlan::FixedDb(20.0),
                variation_db: 0.0,
                analysis_window_ms: 400,
                update_interval_ms: 100,
                backend: SnrProcessingBackend::PcmRollingRmsNoiseInjection,
            }),
        };
        let mut processor =
            AudioPcmEffectProcessor::new(config, 48_000, 2).expect("valid SNR processor");

        assert!(processor.process_interleaved(&[0.0, 0.0, 0.0]).is_err());
    }
}
