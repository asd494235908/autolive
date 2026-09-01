//! SpeexDSP 的最小安全封装。
//!
//! 所有 FFI 只在本 crate 内出现；调用方以固定大小的 16-bit 单声道帧处理，
//! 顺序固定为 AEC → NS/AGC → VAD。该 crate 不拥有音频设备，也不负责线程调度。

use std::fmt;
use std::ptr::NonNull;

mod ffi {
    use std::os::raw::{c_int, c_void};

    #[repr(C)]
    pub struct SpeexEchoState {
        _private: [u8; 0],
    }
    #[repr(C)]
    pub struct SpeexPreprocessState {
        _private: [u8; 0],
    }

    pub const SPEEX_PREPROCESS_SET_DENOISE: c_int = 0;
    pub const SPEEX_PREPROCESS_SET_AGC: c_int = 2;
    pub const SPEEX_PREPROCESS_SET_VAD: c_int = 4;
    pub const SPEEX_PREPROCESS_SET_AGC_LEVEL: c_int = 6;
    pub const SPEEX_PREPROCESS_SET_PROB_START: c_int = 14;
    pub const SPEEX_PREPROCESS_SET_PROB_CONTINUE: c_int = 16;
    pub const SPEEX_PREPROCESS_SET_ECHO_STATE: c_int = 24;
    pub const SPEEX_ECHO_SET_SAMPLING_RATE: c_int = 24;

    unsafe extern "C" {
        pub fn speex_echo_state_init(
            frame_size: c_int,
            filter_length: c_int,
        ) -> *mut SpeexEchoState;
        pub fn speex_echo_state_destroy(st: *mut SpeexEchoState);
        pub fn speex_echo_cancellation(
            st: *mut SpeexEchoState,
            rec: *const i16,
            play: *const i16,
            out: *mut i16,
        );
        pub fn speex_echo_ctl(st: *mut SpeexEchoState, request: c_int, ptr: *mut c_void) -> c_int;

        pub fn speex_preprocess_state_init(
            frame_size: c_int,
            sampling_rate: c_int,
        ) -> *mut SpeexPreprocessState;
        pub fn speex_preprocess_state_destroy(st: *mut SpeexPreprocessState);
        pub fn speex_preprocess_ctl(
            st: *mut SpeexPreprocessState,
            request: c_int,
            ptr: *mut c_void,
        ) -> c_int;
        pub fn speex_preprocess_run(st: *mut SpeexPreprocessState, x: *mut i16) -> c_int;
    }
}

pub const DEFAULT_FRAME_MS: u32 = 10;
pub const DEFAULT_FILTER_MS: u32 = 200;
pub const MIN_FRAME_SAMPLES: usize = 80;
pub const MAX_FRAME_SAMPLES: usize = 960;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechDspConfig {
    pub sample_rate_hz: u32,
    pub frame_samples: usize,
    pub filter_length_samples: usize,
    pub aec_enabled: bool,
    pub noise_suppression_enabled: bool,
    pub agc_enabled: bool,
    pub vad_enabled: bool,
    pub agc_level: i32,
    pub vad_start_percent: i32,
    pub vad_continue_percent: i32,
}

impl Default for SpeechDspConfig {
    fn default() -> Self {
        Self {
            sample_rate_hz: 48_000,
            frame_samples: 480,
            filter_length_samples: 9_600,
            aec_enabled: true,
            noise_suppression_enabled: true,
            agc_enabled: true,
            vad_enabled: true,
            agc_level: 8_000,
            vad_start_percent: 60,
            vad_continue_percent: 45,
        }
    }
}

impl SpeechDspConfig {
    pub fn validate(self) -> Result<(), SpeechDspError> {
        if !matches!(self.sample_rate_hz, 16_000 | 32_000 | 44_100 | 48_000) {
            return Err(SpeechDspError::UnsupportedSampleRate);
        }
        if !(MIN_FRAME_SAMPLES..=MAX_FRAME_SAMPLES).contains(&self.frame_samples) {
            return Err(SpeechDspError::InvalidFrameSize);
        }
        let sample_rate_hz = usize::try_from(self.sample_rate_hz).unwrap_or(1);
        if !(self.frame_samples * 1000).is_multiple_of(sample_rate_hz) {
            return Err(SpeechDspError::InvalidFrameSize);
        }
        if self.filter_length_samples < self.frame_samples {
            return Err(SpeechDspError::InvalidFilterLength);
        }
        if !(0..=32_767).contains(&self.agc_level) {
            return Err(SpeechDspError::InvalidAgcLevel);
        }
        if !(0..=100).contains(&self.vad_start_percent)
            || !(0..=100).contains(&self.vad_continue_percent)
            || self.vad_continue_percent > self.vad_start_percent
        {
            return Err(SpeechDspError::InvalidVadProbability);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechDspFrameResult {
    pub speech: bool,
    pub speech_probability: f32,
    pub input_level: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechDspError {
    UnsupportedSampleRate,
    InvalidFrameSize,
    InvalidFilterLength,
    InvalidAgcLevel,
    InvalidVadProbability,
    EchoInitFailed,
    PreprocessInitFailed,
    ControlFailed(&'static str),
}

impl fmt::Display for SpeechDspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedSampleRate => "SpeexDSP 采样率不支持",
            Self::InvalidFrameSize => "SpeexDSP 帧长度无效",
            Self::InvalidFilterLength => "SpeexDSP 回声滤波长度无效",
            Self::InvalidAgcLevel => "SpeexDSP AGC 目标电平无效",
            Self::InvalidVadProbability => "SpeexDSP VAD 概率无效",
            Self::EchoInitFailed => "SpeexDSP AEC 初始化失败",
            Self::PreprocessInitFailed => "SpeexDSP 预处理器初始化失败",
            Self::ControlFailed(name) => name,
        })
    }
}

impl std::error::Error for SpeechDspError {}

pub struct SpeechDsp {
    echo: Option<NonNull<ffi::SpeexEchoState>>,
    preprocess: NonNull<ffi::SpeexPreprocessState>,
    config: SpeechDspConfig,
    input_i16: Vec<i16>,
    far_end_i16: Vec<i16>,
    output_i16: Vec<i16>,
}

impl fmt::Debug for SpeechDsp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpeechDsp")
            .field("config", &self.config)
            .field("aec_enabled", &self.echo.is_some())
            .finish_non_exhaustive()
    }
}

impl SpeechDsp {
    pub fn new(config: SpeechDspConfig) -> Result<Self, SpeechDspError> {
        config.validate()?;
        let frame =
            i32::try_from(config.frame_samples).map_err(|_| SpeechDspError::InvalidFrameSize)?;
        let filter = i32::try_from(config.filter_length_samples)
            .map_err(|_| SpeechDspError::InvalidFilterLength)?;
        let echo = if config.aec_enabled {
            let ptr = unsafe { ffi::speex_echo_state_init(frame, filter) };
            Some(NonNull::new(ptr).ok_or(SpeechDspError::EchoInitFailed)?)
        } else {
            None
        };
        let preprocess_ptr =
            unsafe { ffi::speex_preprocess_state_init(frame, config.sample_rate_hz as i32) };
        let preprocess = match NonNull::new(preprocess_ptr) {
            Some(ptr) => ptr,
            None => {
                if let Some(echo) = echo {
                    unsafe { ffi::speex_echo_state_destroy(echo.as_ptr()) };
                }
                return Err(SpeechDspError::PreprocessInitFailed);
            }
        };
        let mut dsp = Self {
            echo,
            preprocess,
            config,
            input_i16: vec![0; config.frame_samples],
            far_end_i16: vec![0; config.frame_samples],
            output_i16: vec![0; config.frame_samples],
        };
        if let Some(echo) = dsp.echo {
            let mut sample_rate_hz = i32::try_from(config.sample_rate_hz)
                .map_err(|_| SpeechDspError::UnsupportedSampleRate)?;
            let result = unsafe {
                ffi::speex_echo_ctl(
                    echo.as_ptr(),
                    ffi::SPEEX_ECHO_SET_SAMPLING_RATE,
                    (&mut sample_rate_hz as *mut i32).cast(),
                )
            };
            if result != 0 {
                return Err(SpeechDspError::ControlFailed("SpeexDSP AEC 采样率配置失败"));
            }
        }
        dsp.set_int(
            ffi::SPEEX_PREPROCESS_SET_DENOISE,
            config.noise_suppression_enabled,
            "SpeexDSP 降噪配置失败",
        )?;
        dsp.set_int(
            ffi::SPEEX_PREPROCESS_SET_AGC,
            config.agc_enabled,
            "SpeexDSP AGC 配置失败",
        )?;
        dsp.set_int(
            ffi::SPEEX_PREPROCESS_SET_VAD,
            config.vad_enabled,
            "SpeexDSP VAD 配置失败",
        )?;
        let mut agc_level = config.agc_level as f32;
        let result = unsafe {
            ffi::speex_preprocess_ctl(
                dsp.preprocess.as_ptr(),
                ffi::SPEEX_PREPROCESS_SET_AGC_LEVEL,
                (&mut agc_level as *mut f32).cast(),
            )
        };
        if result != 0 {
            return Err(SpeechDspError::ControlFailed(
                "SpeexDSP AGC 目标电平配置失败",
            ));
        }
        dsp.set_i32(
            ffi::SPEEX_PREPROCESS_SET_PROB_START,
            config.vad_start_percent,
            "SpeexDSP VAD 启动概率配置失败",
        )?;
        dsp.set_i32(
            ffi::SPEEX_PREPROCESS_SET_PROB_CONTINUE,
            config.vad_continue_percent,
            "SpeexDSP VAD 保持概率配置失败",
        )?;
        if let Some(echo) = dsp.echo {
            let result = unsafe {
                ffi::speex_preprocess_ctl(
                    dsp.preprocess.as_ptr(),
                    ffi::SPEEX_PREPROCESS_SET_ECHO_STATE,
                    echo.as_ptr().cast(),
                )
            };
            if result != 0 {
                return Err(SpeechDspError::ControlFailed("SpeexDSP AEC 状态绑定失败"));
            }
        }
        Ok(dsp)
    }

    pub fn process_interleaved_mono_f32(
        &mut self,
        input: &[f32],
        far_end: &[f32],
        output: &mut [f32],
    ) -> Result<SpeechDspFrameResult, SpeechDspError> {
        if input.len() != self.config.frame_samples
            || far_end.len() != self.config.frame_samples
            || output.len() != self.config.frame_samples
        {
            return Err(SpeechDspError::InvalidFrameSize);
        }
        for (index, ((dst, source), far)) in self
            .input_i16
            .iter_mut()
            .zip(input.iter().copied())
            .zip(far_end.iter().copied())
            .enumerate()
        {
            *dst = f32_to_i16(source);
            self.far_end_i16[index] = f32_to_i16(far);
        }
        if let Some(echo) = self.echo {
            unsafe {
                ffi::speex_echo_cancellation(
                    echo.as_ptr(),
                    self.input_i16.as_ptr(),
                    self.far_end_i16.as_ptr(),
                    self.output_i16.as_mut_ptr(),
                );
            }
        } else {
            self.output_i16.copy_from_slice(&self.input_i16);
        }
        let vad = unsafe {
            ffi::speex_preprocess_run(self.preprocess.as_ptr(), self.output_i16.as_mut_ptr())
        };
        let mut peak = 0.0_f32;
        for (dst, sample) in output.iter_mut().zip(self.output_i16.iter().copied()) {
            let converted = f32::from(sample) / 32_768.0;
            *dst = converted;
            peak = peak.max(converted.abs());
        }
        Ok(SpeechDspFrameResult {
            speech: vad > 0,
            speech_probability: if vad > 0 { 1.0 } else { 0.0 },
            input_level: peak.min(1.0),
        })
    }

    fn set_int(
        &mut self,
        request: i32,
        value: bool,
        message: &'static str,
    ) -> Result<(), SpeechDspError> {
        let mut value = i32::from(value);
        let result = unsafe {
            ffi::speex_preprocess_ctl(
                self.preprocess.as_ptr(),
                request,
                (&mut value as *mut i32).cast(),
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(SpeechDspError::ControlFailed(message))
        }
    }

    fn set_i32(
        &mut self,
        request: i32,
        value: i32,
        message: &'static str,
    ) -> Result<(), SpeechDspError> {
        let mut value = value;
        let result = unsafe {
            ffi::speex_preprocess_ctl(
                self.preprocess.as_ptr(),
                request,
                (&mut value as *mut i32).cast(),
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(SpeechDspError::ControlFailed(message))
        }
    }
}

impl Drop for SpeechDsp {
    fn drop(&mut self) {
        unsafe {
            ffi::speex_preprocess_state_destroy(self.preprocess.as_ptr());
            if let Some(echo) = self.echo {
                ffi::speex_echo_state_destroy(echo.as_ptr());
            }
        }
    }
}

unsafe impl Send for SpeechDsp {}

fn f32_to_i16(value: f32) -> i16 {
    if !value.is_finite() {
        return 0;
    }
    (value.clamp(-1.0, 1.0) * 32_767.0).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_unsafe_frame_sizes() {
        assert!(SpeechDspConfig {
            frame_samples: 1,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(SpeechDspConfig {
            filter_length_samples: 10,
            ..Default::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn silence_is_finite_and_not_speech() {
        let mut dsp = SpeechDsp::new(SpeechDspConfig::default()).unwrap();
        let input = vec![0.0; 480];
        let mut output = vec![0.0; 480];
        let result = dsp
            .process_interleaved_mono_f32(&input, &input, &mut output)
            .unwrap();
        assert!(output.iter().all(|sample| sample.is_finite()));
        assert!(!result.speech);
    }

    #[test]
    fn aec_accepts_the_actual_44100_hz_device_rate() {
        let config = SpeechDspConfig {
            sample_rate_hz: 44_100,
            frame_samples: 441,
            filter_length_samples: 8_820,
            ..Default::default()
        };
        let mut dsp = SpeechDsp::new(config).expect("AEC must accept the device rate");
        let input = vec![0.0; config.frame_samples];
        let mut output = vec![0.0; config.frame_samples];
        dsp.process_interleaved_mono_f32(&input, &input, &mut output)
            .expect("AEC must process a 44.1 kHz frame");
    }

    #[test]
    fn stable_far_end_echo_is_reduced_after_adaptation() {
        let mut dsp = SpeechDsp::new(SpeechDspConfig {
            noise_suppression_enabled: false,
            agc_enabled: false,
            vad_enabled: false,
            ..Default::default()
        })
        .unwrap();
        let mut input = vec![0.0_f32; 480];
        let mut far_end = vec![0.0_f32; 480];
        let mut output = vec![0.0_f32; 480];
        let mut tail_energy = 0.0_f64;
        let mut tail_samples = 0_usize;
        for frame_index in 0..300_usize {
            for sample_index in 0..480_usize {
                let absolute_sample = frame_index * 480 + sample_index;
                let phase = 2.0 * std::f32::consts::PI * 440.0 * absolute_sample as f32 / 48_000.0;
                let echo = 0.35 * phase.sin();
                far_end[sample_index] = echo;
                input[sample_index] = echo;
            }
            dsp.process_interleaved_mono_f32(&input, &far_end, &mut output)
                .unwrap();
            if frame_index >= 250 {
                tail_energy += output
                    .iter()
                    .map(|sample| f64::from(*sample) * f64::from(*sample))
                    .sum::<f64>();
                tail_samples += output.len();
            }
        }
        let tail_rms = (tail_energy / tail_samples as f64).sqrt();
        assert!(
            tail_rms < 0.12,
            "stable far-end echo was not sufficiently reduced: rms={tail_rms:.4}"
        );
    }

    #[test]
    fn sustained_near_end_signal_triggers_vad() {
        let mut dsp = SpeechDsp::new(SpeechDspConfig {
            aec_enabled: false,
            noise_suppression_enabled: false,
            agc_enabled: false,
            ..Default::default()
        })
        .unwrap();
        let mut input = vec![0.0_f32; 480];
        let far_end = vec![0.0_f32; 480];
        let mut output = vec![0.0_f32; 480];
        let mut speech_frames = 0_usize;
        for frame_index in 0..120_usize {
            for (sample_index, sample) in input.iter_mut().enumerate() {
                let absolute_sample = frame_index * 480 + sample_index;
                let phase = 2.0 * std::f32::consts::PI * 180.0 * absolute_sample as f32 / 48_000.0;
                *sample = 0.35 * phase.sin();
            }
            let result = dsp
                .process_interleaved_mono_f32(&input, &far_end, &mut output)
                .unwrap();
            speech_frames += usize::from(result.speech);
        }
        assert!(
            speech_frames > 0,
            "sustained near-end signal did not trigger VAD"
        );
    }
}
