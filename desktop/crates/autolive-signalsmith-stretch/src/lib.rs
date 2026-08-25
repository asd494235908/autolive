//! Signalsmith Stretch 的交错 `f32` PCM 安全适配层。
//!
//! 本 crate 只负责同步 DSP，不创建线程或异步任务；调用方应在已有音频工作线程中调用。

use std::cell::Cell;
use std::ffi::c_void;
use std::fmt;
use std::marker::PhantomData;
use std::ptr::NonNull;

pub const MAX_CHANNELS: usize = 8;
pub const MIN_SAMPLE_RATE_HZ: u32 = 8_000;
pub const MAX_SAMPLE_RATE_HZ: u32 = 192_000;
pub const MIN_PITCH_SEMITONES: f64 = -2.0;
pub const MAX_PITCH_SEMITONES: f64 = 2.0;
pub const MIN_FORMANT_SHIFT_PERCENT: f64 = -5.0;
pub const MAX_FORMANT_SHIFT_PERCENT: f64 = 5.0;
pub const MAX_FRAMES_PER_CALL: usize = 65_536;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualityPitchConfig {
    pub pitch_shift_semitones: f64,
    pub formant_shift_percent: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessingLatency {
    pub input_frames: usize,
    pub output_frames: usize,
}

impl ProcessingLatency {
    pub fn total_frames(self) -> usize {
        self.input_frames.saturating_add(self.output_frames)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StretchError {
    InvalidConfig(&'static str),
    InvalidBuffer(&'static str),
    InvalidState(&'static str),
    NativeFailure(&'static str),
}

impl fmt::Display for StretchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message)
            | Self::InvalidBuffer(message)
            | Self::InvalidState(message)
            | Self::NativeFailure(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for StretchError {}

#[allow(non_camel_case_types)]
type autolive_stretch_t = c_void;

extern "C" {
    fn autolive_stretch_create(
        channels: i32,
        sample_rate_hz: f32,
        pitch_semitones: f32,
        formant_factor: f32,
        formant_base: f32,
    ) -> *mut autolive_stretch_t;
    fn autolive_stretch_destroy(handle: *mut autolive_stretch_t);
    fn autolive_stretch_input_latency(handle: *const autolive_stretch_t) -> usize;
    fn autolive_stretch_output_latency(handle: *const autolive_stretch_t) -> usize;
    fn autolive_stretch_process(
        handle: *mut autolive_stretch_t,
        input: *const f32,
        frames: usize,
        output: *mut f32,
    ) -> bool;
    fn autolive_stretch_flush(
        handle: *mut autolive_stretch_t,
        output: *mut f32,
        frames: usize,
    ) -> bool;
    fn autolive_stretch_reset(handle: *mut autolive_stretch_t) -> bool;
}

/// 单线程使用的同速音高/共振峰处理器。
///
/// 同速处理时，Signalsmith 的输入和输出延迟之和就是输入时间零点到首个有效输出的
/// 固定偏移。`process_interleaved` 裁掉这段前导，`flush` 用等量尾部补齐，因此一轮
/// 累计输出帧数严格等于累计输入帧数，事件时间不变，前导静音不会进入播放时间轴。
pub struct QualityPitchProcessor {
    inner: NonNull<autolive_stretch_t>,
    channels: usize,
    latency: ProcessingLatency,
    leading_frames_to_drop: usize,
    input_frames: usize,
    output_frames: usize,
    finished: bool,
    _not_sync: PhantomData<Cell<()>>,
}

// SAFETY: 句柄由该类型唯一所有，移动所有权不会并发访问；`Cell` 保证类型不实现 Sync。
unsafe impl Send for QualityPitchProcessor {}

impl QualityPitchProcessor {
    pub fn new(
        config: QualityPitchConfig,
        sample_rate_hz: u32,
        channels: usize,
    ) -> Result<Self, StretchError> {
        validate_config(config, sample_rate_hz, channels)?;

        let inner = NonNull::new(unsafe {
            autolive_stretch_create(
                channels as i32,
                sample_rate_hz as f32,
                config.pitch_shift_semitones as f32,
                1.0 + config.formant_shift_percent as f32 / 100.0,
                0.0,
            )
        })
        .ok_or(StretchError::NativeFailure(
            "Signalsmith Stretch 初始化失败",
        ))?;
        let latency = ProcessingLatency {
            input_frames: unsafe { autolive_stretch_input_latency(inner.as_ptr()) },
            output_frames: unsafe { autolive_stretch_output_latency(inner.as_ptr()) },
        };
        if latency.total_frames() > MAX_FRAMES_PER_CALL {
            unsafe { autolive_stretch_destroy(inner.as_ptr()) };
            return Err(StretchError::InvalidConfig(
                "Signalsmith Stretch 总延迟超过单次处理上限",
            ));
        }

        Ok(Self {
            inner,
            channels,
            latency,
            leading_frames_to_drop: latency.total_frames(),
            input_frames: 0,
            output_frames: 0,
            finished: false,
            _not_sync: PhantomData,
        })
    }

    pub fn latency_frames(&self) -> ProcessingLatency {
        self.latency
    }

    /// 同速处理一块交错 PCM。首批调用可能因前导延迟裁剪而返回空或较短数据。
    pub fn process_interleaved(&mut self, input: &[f32]) -> Result<Vec<f32>, StretchError> {
        if self.finished {
            return Err(StretchError::InvalidState(
                "处理器已 flush；继续处理前必须 reset",
            ));
        }
        let input_frame_count = validate_input_buffer(input, self.channels)?;
        self.input_frames = self
            .input_frames
            .checked_add(input_frame_count)
            .ok_or(StretchError::InvalidBuffer("累计输入帧数溢出"))?;

        let mut output = vec![0.0; input.len()];
        let processed = unsafe {
            autolive_stretch_process(
                self.inner.as_ptr(),
                input.as_ptr(),
                input_frame_count,
                output.as_mut_ptr(),
            )
        };
        if !processed {
            return Err(StretchError::NativeFailure("Signalsmith Stretch 处理失败"));
        }
        ensure_finite_output(&output)?;
        self.drop_leading_frames(&mut output);
        self.output_frames = self
            .output_frames
            .checked_add(output.len() / self.channels)
            .ok_or(StretchError::InvalidBuffer("累计输出帧数溢出"))?;
        Ok(output)
    }

    /// 推进输入延迟并排空尾部，只返回维持本轮输入时长所需的剩余 PCM。
    pub fn flush(&mut self) -> Result<Vec<f32>, StretchError> {
        if self.finished {
            return Err(StretchError::InvalidState("处理器已经 flush"));
        }
        self.finished = true;
        if self.input_frames == 0 {
            self.native_reset()?;
            return Ok(Vec::new());
        }

        let input_samples = checked_sample_count(self.latency.input_frames, self.channels)?;
        let output_samples = checked_sample_count(self.latency.output_frames, self.channels)?;
        let mut tail = vec![0.0; input_samples.saturating_add(output_samples)];
        if input_samples > 0 {
            let silence = vec![0.0; input_samples];
            let processed = unsafe {
                autolive_stretch_process(
                    self.inner.as_ptr(),
                    silence.as_ptr(),
                    self.latency.input_frames,
                    tail.as_mut_ptr(),
                )
            };
            if !processed {
                return Err(StretchError::NativeFailure(
                    "Signalsmith Stretch 尾部推进失败",
                ));
            }
        }
        if output_samples > 0 {
            let flushed = unsafe {
                autolive_stretch_flush(
                    self.inner.as_ptr(),
                    tail[input_samples..].as_mut_ptr(),
                    self.latency.output_frames,
                )
            };
            if !flushed {
                return Err(StretchError::NativeFailure(
                    "Signalsmith Stretch 尾部排空失败",
                ));
            }
        }
        ensure_finite_output(&tail)?;
        self.drop_leading_frames(&mut tail);

        let remaining_frames = self.input_frames.saturating_sub(self.output_frames);
        if tail.len() / self.channels < remaining_frames {
            return Err(StretchError::InvalidState(
                "Signalsmith Stretch 尾部帧数不足",
            ));
        }
        tail.truncate(checked_sample_count(remaining_frames, self.channels)?);
        self.output_frames = self
            .output_frames
            .checked_add(remaining_frames)
            .ok_or(StretchError::InvalidBuffer("累计输出帧数溢出"))?;
        Ok(tail)
    }

    pub fn reset(&mut self) -> Result<(), StretchError> {
        self.native_reset()?;
        self.leading_frames_to_drop = self.latency.total_frames();
        self.input_frames = 0;
        self.output_frames = 0;
        self.finished = false;
        Ok(())
    }

    fn native_reset(&mut self) -> Result<(), StretchError> {
        if unsafe { autolive_stretch_reset(self.inner.as_ptr()) } {
            Ok(())
        } else {
            Err(StretchError::NativeFailure("Signalsmith Stretch 重置失败"))
        }
    }

    fn drop_leading_frames(&mut self, output: &mut Vec<f32>) {
        let frames = output.len() / self.channels;
        let drop_frames = frames.min(self.leading_frames_to_drop);
        output.drain(..drop_frames * self.channels);
        self.leading_frames_to_drop -= drop_frames;
    }
}

impl Drop for QualityPitchProcessor {
    fn drop(&mut self) {
        unsafe { autolive_stretch_destroy(self.inner.as_ptr()) };
    }
}

fn validate_config(
    config: QualityPitchConfig,
    sample_rate_hz: u32,
    channels: usize,
) -> Result<(), StretchError> {
    if !(1..=MAX_CHANNELS).contains(&channels) {
        return Err(StretchError::InvalidConfig("声道数必须在 1..=8 范围内"));
    }
    if !(MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&sample_rate_hz) {
        return Err(StretchError::InvalidConfig(
            "采样率必须在 8000..=192000 Hz 范围内",
        ));
    }
    if !config.pitch_shift_semitones.is_finite()
        || !(MIN_PITCH_SEMITONES..=MAX_PITCH_SEMITONES).contains(&config.pitch_shift_semitones)
    {
        return Err(StretchError::InvalidConfig(
            "音高必须是 -2..=2 的有限半音值",
        ));
    }
    if !config.formant_shift_percent.is_finite()
        || !(MIN_FORMANT_SHIFT_PERCENT..=MAX_FORMANT_SHIFT_PERCENT)
            .contains(&config.formant_shift_percent)
    {
        return Err(StretchError::InvalidConfig(
            "共振峰偏移必须是 -5..=5 的有限百分比",
        ));
    }
    Ok(())
}

fn validate_input_buffer(samples: &[f32], channels: usize) -> Result<usize, StretchError> {
    if samples.is_empty() {
        return Err(StretchError::InvalidBuffer("输入 PCM 不能为空"));
    }
    if !samples.len().is_multiple_of(channels) {
        return Err(StretchError::InvalidBuffer(
            "输入 PCM 长度必须是声道数的整数倍",
        ));
    }
    let frames = samples.len() / channels;
    if frames > MAX_FRAMES_PER_CALL {
        return Err(StretchError::InvalidBuffer("输入 PCM 超过单次帧数上限"));
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(StretchError::InvalidBuffer("输入 PCM 包含非有限值"));
    }
    Ok(frames)
}

fn ensure_finite_output(samples: &[f32]) -> Result<(), StretchError> {
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(StretchError::InvalidState(
            "Signalsmith Stretch 产生非有限 PCM",
        ));
    }
    Ok(())
}

fn checked_sample_count(frames: usize, channels: usize) -> Result<usize, StretchError> {
    frames
        .checked_mul(channels)
        .ok_or(StretchError::InvalidBuffer("PCM 样本数量溢出"))
}

#[cfg(test)]
mod tests {
    use super::{
        QualityPitchConfig, QualityPitchProcessor, StretchError, MAX_FRAMES_PER_CALL,
        MAX_PITCH_SEMITONES,
    };

    fn config() -> QualityPitchConfig {
        QualityPitchConfig {
            pitch_shift_semitones: 1.0,
            formant_shift_percent: 0.0,
        }
    }

    fn stereo_sine(frames: usize) -> Vec<f32> {
        let mut input = Vec::with_capacity(frames * 2);
        for frame in 0..frames {
            let sample = (std::f32::consts::TAU * 220.0 * frame as f32 / 48_000.0).sin() * 0.2;
            input.extend_from_slice(&[sample, sample]);
        }
        input
    }

    fn render(config: QualityPitchConfig, input: &[f32]) -> Vec<f32> {
        let mut processor = QualityPitchProcessor::new(config, 48_000, 2).expect("valid processor");
        let mut output = Vec::with_capacity(input.len());
        for chunk in input.chunks(4_096 * 2) {
            output.extend(processor.process_interleaved(chunk).expect("process chunk"));
        }
        output.extend(processor.flush().expect("flush tail"));
        output
    }

    #[test]
    fn rejects_out_of_contract_config() {
        assert!(matches!(
            QualityPitchProcessor::new(config(), 48_000, 0),
            Err(StretchError::InvalidConfig(_))
        ));
        assert!(matches!(
            QualityPitchProcessor::new(
                QualityPitchConfig {
                    pitch_shift_semitones: MAX_PITCH_SEMITONES + 0.1,
                    ..config()
                },
                48_000,
                2,
            ),
            Err(StretchError::InvalidConfig(_))
        ));
    }

    #[test]
    fn rejects_misaligned_nonfinite_and_unbounded_pcm() {
        let mut processor = QualityPitchProcessor::new(config(), 48_000, 2).expect("valid config");
        assert!(processor.process_interleaved(&[0.0; 3]).is_err());
        assert!(processor.process_interleaved(&[f32::NAN, 0.0]).is_err());
        assert!(processor
            .process_interleaved(&vec![0.0; (MAX_FRAMES_PER_CALL + 1) * 2])
            .is_err());
    }

    #[test]
    fn chunked_processing_and_flush_preserve_total_frame_count() {
        let mut processor = QualityPitchProcessor::new(config(), 48_000, 2).expect("valid config");
        let chunk_frames = [17, 503, 4_096, 61, 8_000, 3];
        let mut output = Vec::new();
        for frames in chunk_frames {
            output.extend(
                processor
                    .process_interleaved(&stereo_sine(frames))
                    .expect("process chunk"),
            );
        }
        output.extend(processor.flush().expect("flush tail"));

        assert_eq!(output.len(), chunk_frames.into_iter().sum::<usize>() * 2);
        assert!(output.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn two_semitones_raise_the_measured_frequency() {
        let input = stereo_sine(96_000);
        let output = render(
            QualityPitchConfig {
                pitch_shift_semitones: 2.0,
                formant_shift_percent: 0.0,
            },
            &input,
        );
        let start_frame = 24_000;
        let end_frame = 72_000;
        let left: Vec<f32> = output[start_frame * 2..end_frame * 2]
            .chunks_exact(2)
            .map(|frame| frame[0])
            .collect();
        let rising_crossings = left
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        let measured_hz = rising_crossings as f64;

        assert!((240.0..=254.0).contains(&measured_hz));
    }

    #[test]
    fn formant_shift_processes_a_nonzero_harmonic_signal() {
        let frames = 48_000;
        let mut input = Vec::with_capacity(frames * 2);
        for frame in 0..frames {
            let time = frame as f32 / 48_000.0;
            let sample = ((std::f32::consts::TAU * 120.0 * time).sin() * 0.18)
                + ((std::f32::consts::TAU * 720.0 * time).sin() * 0.08)
                + ((std::f32::consts::TAU * 1_440.0 * time).sin() * 0.04);
            input.extend_from_slice(&[sample, sample]);
        }
        let output = render(
            QualityPitchConfig {
                pitch_shift_semitones: 0.0,
                formant_shift_percent: 5.0,
            },
            &input,
        );
        let rms = (output
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>()
            / output.len() as f64)
            .sqrt();

        assert_eq!(output.len(), input.len());
        assert!(output.iter().all(|sample| sample.is_finite()));
        assert!(rms > 0.01);
    }

    #[test]
    fn latency_compensation_keeps_an_impulse_on_its_input_timeline() {
        let impulse_frame = 16_000;
        let mut input = vec![0.0; 48_000 * 2];
        input[impulse_frame * 2] = 1.0;
        input[impulse_frame * 2 + 1] = 1.0;
        let output = render(config(), &input);
        let output_peak_frame = output
            .chunks_exact(2)
            .enumerate()
            .max_by(|(_, left), (_, right)| {
                left[0]
                    .abs()
                    .partial_cmp(&right[0].abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(frame, _)| frame)
            .expect("non-empty output");

        assert!(output_peak_frame.abs_diff(impulse_frame) <= 1_024);
    }

    #[test]
    fn shorter_than_latency_still_preserves_tail_frames() {
        let mut processor = QualityPitchProcessor::new(config(), 44_100, 2).expect("valid config");
        let input_frames = processor
            .latency_frames()
            .total_frames()
            .saturating_sub(1)
            .max(1);

        let first = processor
            .process_interleaved(&stereo_sine(input_frames))
            .expect("process short input");
        let tail = processor.flush().expect("flush short tail");

        assert!(first.len() / 2 < input_frames);
        assert_eq!(first.len() + tail.len(), input_frames * 2);
    }

    #[test]
    fn reset_allows_another_non_block_aligned_round() {
        let mut processor = QualityPitchProcessor::new(config(), 48_000, 2).expect("valid config");
        let input = stereo_sine(2_003);

        let first_len = processor
            .process_interleaved(&input)
            .expect("first process")
            .len()
            + processor.flush().expect("first flush").len();
        assert!(processor.process_interleaved(&input).is_err());

        processor.reset().expect("reset");
        let second_len = processor
            .process_interleaved(&input)
            .expect("second process")
            .len()
            + processor.flush().expect("second flush").len();
        assert_eq!(first_len, input.len());
        assert_eq!(second_len, input.len());
    }
}
