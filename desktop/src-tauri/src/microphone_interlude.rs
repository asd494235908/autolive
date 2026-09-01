//! 麦克风插话的会话契约和生命周期控制。
//!
//! 本模块拥有真实的本地输入 DSP 会话；它不会启动 ASR、LLM、TTS 或任何实时话术变换。

use crate::audio_cycle_output::MicrophoneAudioBridge;
use crate::cancellation::CancellationToken;
use autolive_speech_dsp::{SpeechDsp, SpeechDspConfig, SpeechDspFrameResult};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const DEFAULT_MICROPHONE_SAMPLE_RATE_HZ: u32 = 48_000;
pub const MAX_MICROPHONE_DEVICE_ID_BYTES: usize = 256;
pub const MAX_MICROPHONE_DEVICE_NAME_BYTES: usize = 256;
pub const MAX_MICROPHONE_JOIN_BUDGET: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MicrophoneInterludeState {
    Disabled,
    Opening,
    Armed,
    Speaking,
    Hangover,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MicrophoneSensitivity {
    Low,
    #[default]
    Standard,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicrophoneInterludeConfigDto {
    /// 仅保存用户选择，不代表当前正在监听。
    #[serde(default)]
    pub enabled: bool,
    /// 只允许来自 `list_portaudio_input_devices` 的稳定 ID；None 表示系统默认。
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default = "default_sample_rate")]
    pub sample_rate_hz: u32,
    #[serde(default)]
    pub sensitivity: MicrophoneSensitivity,
    #[serde(default = "default_true")]
    pub aec_enabled: bool,
    #[serde(default = "default_true")]
    pub noise_suppression_enabled: bool,
    #[serde(default = "default_true")]
    pub agc_enabled: bool,
}

fn default_sample_rate() -> u32 {
    DEFAULT_MICROPHONE_SAMPLE_RATE_HZ
}

fn default_true() -> bool {
    true
}

impl Default for MicrophoneInterludeConfigDto {
    fn default() -> Self {
        Self {
            enabled: false,
            device_id: None,
            sample_rate_hz: DEFAULT_MICROPHONE_SAMPLE_RATE_HZ,
            sensitivity: MicrophoneSensitivity::Standard,
            aec_enabled: true,
            noise_suppression_enabled: true,
            agc_enabled: true,
        }
    }
}

impl MicrophoneInterludeConfigDto {
    pub fn validate(&self) -> Result<(), MicrophoneInterludeError> {
        if let Some(device_id) = self.device_id.as_deref() {
            if device_id.trim().is_empty() {
                return Err(MicrophoneInterludeError::DeviceIdEmpty);
            }
            if device_id.len() > MAX_MICROPHONE_DEVICE_ID_BYTES {
                return Err(MicrophoneInterludeError::DeviceIdTooLong);
            }
            if device_id
                .chars()
                .any(|character| character.is_control() || matches!(character, '/' | '\\'))
            {
                return Err(MicrophoneInterludeError::DeviceIdInvalid);
            }
            let Some((host_api, fingerprint)) = device_id
                .strip_prefix("pa-input-")
                .and_then(|value| value.split_once('-'))
            else {
                return Err(MicrophoneInterludeError::DeviceIdInvalid);
            };
            if !matches!(
                host_api,
                "default" | "wasapi" | "asio" | "mme" | "dsound" | "wdmks" | "other"
            ) || fingerprint.len() != 16
                || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(MicrophoneInterludeError::DeviceIdInvalid);
            }
        }
        if !matches!(self.sample_rate_hz, 16_000 | 32_000 | 44_100 | 48_000) {
            return Err(MicrophoneInterludeError::SampleRateUnsupported);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MicrophoneInputDeviceDto {
    pub id: String,
    pub name: String,
    pub host_api: String,
    pub max_input_channels: u16,
    pub default_sample_rate_hz: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MicrophoneInterludeStatusDto {
    pub state: MicrophoneInterludeState,
    pub generation: u64,
    pub selected_device_id: Option<String>,
    pub selected_device_name: Option<String>,
    pub host_api: Option<String>,
    pub actual_sample_rate_hz: Option<u32>,
    pub input_level: f32,
    pub speech_probability: f32,
    /// 只有真实 SpeexDSP/等价实现启动成功后才允许为 true。
    pub aec_active: bool,
    pub noise_suppression_active: bool,
    pub agc_active: bool,
    pub input_overflow_count: u64,
    pub output_underflow_count: u64,
    pub dropped_frame_count: u64,
    /// Speaking 时由末端 callback 设为 true；失败/停止必须为 false（fail-open）。
    pub media_muted: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MicrophoneInterludeStartResultDto {
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MicrophoneInterludeError {
    DeviceIdEmpty,
    DeviceIdTooLong,
    DeviceIdInvalid,
    DeviceNameTooLong,
    SampleRateUnsupported,
    InputBackendUnavailable,
    Busy,
    SessionNotActive,
    ControllerPoisoned,
    WorkerStartFailed,
    DspInitFailed,
    DspProcessFailed,
    AudioBridgeFailed,
    WorkerJoinTimeout,
    WorkerJoinFailed,
}

impl MicrophoneInterludeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::DeviceIdEmpty => "microphone_device_id_empty",
            Self::DeviceIdTooLong => "microphone_device_id_too_long",
            Self::DeviceIdInvalid => "microphone_device_id_invalid",
            Self::DeviceNameTooLong => "microphone_device_name_too_long",
            Self::SampleRateUnsupported => "microphone_sample_rate_unsupported",
            Self::InputBackendUnavailable => "microphone_input_unavailable",
            Self::Busy => "microphone_session_busy",
            Self::SessionNotActive => "microphone_session_not_active",
            Self::ControllerPoisoned => "microphone_state_lock_failed",
            Self::WorkerStartFailed => "microphone_worker_start_failed",
            Self::DspInitFailed => "microphone_dsp_init_failed",
            Self::DspProcessFailed => "microphone_dsp_process_failed",
            Self::AudioBridgeFailed => "microphone_audio_bridge_failed",
            Self::WorkerJoinTimeout => "microphone_worker_stop_timeout",
            Self::WorkerJoinFailed => "microphone_worker_join_failed",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::DeviceIdEmpty => "麦克风设备 ID 不能为空",
            Self::DeviceIdTooLong => "麦克风设备 ID 超过 256 字节限制",
            Self::DeviceIdInvalid => "麦克风设备 ID 包含非法字符",
            Self::DeviceNameTooLong => "麦克风设备名称超过 256 字节限制",
            Self::SampleRateUnsupported => "麦克风采样率不在允许范围内",
            Self::InputBackendUnavailable => "麦克风输入设备或全双工后端不可用，当前未开始监听",
            Self::Busy => "麦克风监听会话正在处理另一项请求",
            Self::SessionNotActive => "麦克风监听会话当前未处于可更新状态",
            Self::ControllerPoisoned => "麦克风会话状态锁已损坏",
            Self::WorkerStartFailed => "麦克风 Worker 启动失败",
            Self::DspInitFailed => "麦克风 SpeexDSP 初始化失败",
            Self::DspProcessFailed => "麦克风 SpeexDSP 处理失败",
            Self::AudioBridgeFailed => "麦克风音频桥接失败",
            Self::WorkerJoinTimeout => "麦克风 Worker 未在停止预算内退出",
            Self::WorkerJoinFailed => "麦克风 Worker 异常终止",
        }
    }
}

impl fmt::Display for MicrophoneInterludeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for MicrophoneInterludeError {}

#[derive(Debug)]
struct MicrophoneInterludeSession {
    generation: u64,
    cancellation: CancellationToken,
    bridge: MicrophoneAudioBridge,
    handle: JoinHandle<()>,
}

#[derive(Debug)]
struct Inner {
    generation: u64,
    state: MicrophoneInterludeState,
    config: MicrophoneInterludeConfigDto,
    selected_device_name: Option<String>,
    host_api: Option<String>,
    actual_sample_rate_hz: Option<u32>,
    input_level: f32,
    speech_probability: f32,
    aec_active: bool,
    noise_suppression_active: bool,
    agc_active: bool,
    input_overflow_count: u64,
    output_underflow_count: u64,
    dropped_frame_count: u64,
    media_muted: bool,
    paused: bool,
    error_code: Option<String>,
    error_message: Option<String>,
    session: Option<MicrophoneInterludeSession>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            generation: 0,
            state: MicrophoneInterludeState::Disabled,
            config: MicrophoneInterludeConfigDto::default(),
            selected_device_name: None,
            host_api: None,
            actual_sample_rate_hz: None,
            input_level: 0.0,
            speech_probability: 0.0,
            aec_active: false,
            noise_suppression_active: false,
            agc_active: false,
            input_overflow_count: 0,
            output_underflow_count: 0,
            dropped_frame_count: 0,
            media_muted: false,
            paused: false,
            error_code: None,
            error_message: None,
            session: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MicrophoneInterludeController {
    inner: Arc<Mutex<Inner>>,
}

impl MicrophoneInterludeController {
    pub fn has_active_session(&self) -> bool {
        self.inner
            .lock()
            .map(|inner| inner.session.is_some())
            .unwrap_or(true)
    }

    /// 判断 start 请求是否可安全复用当前会话。
    ///
    /// 只有配置完全相同、Worker 仍在运行且 full-duplex 桥接仍存活时才
    /// 幂等返回；设备切换或桥接失效必须走冲突/停止路径，不能静默复用旧会话。
    pub fn is_active_for_config(
        &self,
        config: &MicrophoneInterludeConfigDto,
    ) -> Result<bool, MicrophoneInterludeError> {
        let inner = self.lock_inner()?;
        Ok(inner.state != MicrophoneInterludeState::Failed
            && inner.config == *config
            && inner.session.as_ref().is_some_and(|session| {
                !session.handle.is_finished() && session.bridge.is_available()
            }))
    }

    pub fn status(&self) -> MicrophoneInterludeStatusDto {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        status_from_inner(&inner)
    }

    pub fn configure(
        &self,
        config: MicrophoneInterludeConfigDto,
    ) -> Result<MicrophoneInterludeStatusDto, MicrophoneInterludeError> {
        config.validate()?;
        let mut inner = self.lock_inner()?;
        if inner.session.is_some() && inner.config != config {
            return Err(MicrophoneInterludeError::Busy);
        }
        // 故障会话仍可能持有已结束但尚未 Join 的 Worker/PortAudio 所有权。
        // 保留 Failed 状态直到显式 Stop，避免配置保存先清掉故障态后让 UI
        // 隐藏停止入口，最终无法回收并重新打开设备。
        if inner.state == MicrophoneInterludeState::Failed && inner.session.is_some() {
            return Err(MicrophoneInterludeError::Busy);
        }
        if inner.config != config {
            inner.generation = next_generation(inner.generation);
            inner.actual_sample_rate_hz = None;
            inner.selected_device_name = None;
            inner.host_api = None;
        }
        inner.config = config.clone();
        if inner.state == MicrophoneInterludeState::Failed {
            clear_failure(&mut inner);
            inner.state = MicrophoneInterludeState::Disabled;
        }
        Ok(status_from_inner(&inner))
    }

    pub fn set_device_metadata(
        &self,
        name: Option<String>,
        host_api: Option<String>,
    ) -> Result<MicrophoneInterludeStatusDto, MicrophoneInterludeError> {
        let name = name.as_deref().map(validate_device_name).transpose()?;
        let mut inner = self.lock_inner()?;
        inner.selected_device_name = name;
        inner.host_api = host_api;
        Ok(status_from_inner(&inner))
    }

    pub fn start_with_bridge(
        &self,
        config: MicrophoneInterludeConfigDto,
        bridge: MicrophoneAudioBridge,
        on_speaking_start: Arc<dyn Fn() + Send + Sync + 'static>,
    ) -> Result<MicrophoneInterludeStartResultDto, MicrophoneInterludeError> {
        config.validate()?;
        let mut inner = self.lock_inner()?;
        if let Some(session) = inner.session.as_ref() {
            if session.generation > 0
                && inner.config == config
                && inner.state != MicrophoneInterludeState::Failed
            {
                return Ok(MicrophoneInterludeStartResultDto {
                    generation: session.generation,
                });
            }
            return Err(MicrophoneInterludeError::Busy);
        }

        inner.generation = next_generation(inner.generation);
        inner.config = config.clone();
        inner.state = MicrophoneInterludeState::Opening;
        inner.media_muted = false;
        clear_failure(&mut inner);
        let generation = inner.generation;
        let input_channels = bridge
            .input_channels()
            .map_err(|_| MicrophoneInterludeError::InputBackendUnavailable)?;
        let output_channels = bridge
            .output_channels()
            .map_err(|_| MicrophoneInterludeError::InputBackendUnavailable)?;
        if input_channels != 1 || !matches!(output_channels, 1 | 2) {
            inner.state = MicrophoneInterludeState::Failed;
            inner.actual_sample_rate_hz = None;
            inner.error_code = Some(
                MicrophoneInterludeError::InputBackendUnavailable
                    .code()
                    .into(),
            );
            inner.error_message = Some("麦克风当前只支持单声道输入和单/双声道输出".to_owned());
            return Err(MicrophoneInterludeError::InputBackendUnavailable);
        }
        if bridge.set_gate_enabled(true).is_err() {
            inner.state = MicrophoneInterludeState::Failed;
            inner.actual_sample_rate_hz = None;
            inner.error_code = Some(
                MicrophoneInterludeError::InputBackendUnavailable
                    .code()
                    .into(),
            );
            inner.error_message = Some("麦克风门控初始化失败".to_owned());
            return Err(MicrophoneInterludeError::InputBackendUnavailable);
        }
        let dsp_config = speech_dsp_config(&config);
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let worker_inner = Arc::clone(&self.inner);
        let worker_bridge = bridge.clone();
        let handle = thread::Builder::new()
            .name(format!("microphone-interlude-{generation}"))
            .spawn(move || {
                run_microphone_worker(
                    worker_inner,
                    worker_bridge,
                    worker_cancellation,
                    dsp_config,
                    input_channels,
                    output_channels,
                    on_speaking_start,
                )
            })
            .map_err(|_| MicrophoneInterludeError::WorkerStartFailed);
        let handle = match handle {
            Ok(handle) => handle,
            Err(error) => {
                let _ = bridge.set_gate_enabled(false);
                inner.state = MicrophoneInterludeState::Failed;
                inner.actual_sample_rate_hz = None;
                inner.error_code = Some(error.code().into());
                inner.error_message = Some(error.message().into());
                inner.media_muted = false;
                return Err(error);
            }
        };
        inner.session = Some(MicrophoneInterludeSession {
            generation,
            cancellation,
            bridge,
            handle,
        });
        // Worker 可能在这里之前就完成 DSP 初始化或报告失败；只覆盖仍停留
        // 在 Opening 的正常路径，不能把 Failed/暂停态竞态改写成 Armed。
        if inner.state == MicrophoneInterludeState::Opening {
            inner.state = MicrophoneInterludeState::Armed;
        }
        Ok(MicrophoneInterludeStartResultDto { generation })
    }

    pub fn stop(&self) -> Result<MicrophoneInterludeStatusDto, MicrophoneInterludeError> {
        let session = {
            let mut inner = self.lock_inner()?;
            inner.generation = next_generation(inner.generation);
            inner.state = MicrophoneInterludeState::Stopping;
            inner.media_muted = false;
            inner.paused = false;
            inner.aec_active = false;
            inner.noise_suppression_active = false;
            inner.agc_active = false;
            inner.actual_sample_rate_hz = None;
            inner.session.take()
        };
        if let Some(session) = session {
            let _ = session.bridge.set_gate_enabled(false);
            session.cancellation.cancel();
            match join_session(session) {
                Ok(()) => {}
                Err(SessionJoinError::Timeout(session)) => {
                    let mut inner = self.lock_inner()?;
                    // 超时不能丢弃 JoinHandle；保留会话供后续 stop/recovery 再次回收。
                    inner.session = Some(session);
                    inner.state = MicrophoneInterludeState::Failed;
                    inner.media_muted = false;
                    inner.error_code =
                        Some(MicrophoneInterludeError::WorkerJoinTimeout.code().into());
                    inner.error_message =
                        Some(MicrophoneInterludeError::WorkerJoinTimeout.message().into());
                    return Err(MicrophoneInterludeError::WorkerJoinTimeout);
                }
                Err(SessionJoinError::Failed) => {
                    let mut inner = self.lock_inner()?;
                    inner.state = MicrophoneInterludeState::Failed;
                    inner.media_muted = false;
                    inner.error_code =
                        Some(MicrophoneInterludeError::WorkerJoinFailed.code().into());
                    inner.error_message =
                        Some(MicrophoneInterludeError::WorkerJoinFailed.message().into());
                    return Err(MicrophoneInterludeError::WorkerJoinFailed);
                }
            }
        }
        let mut inner = self.lock_inner()?;
        inner.state = MicrophoneInterludeState::Disabled;
        clear_failure(&mut inner);
        Ok(status_from_inner(&inner))
    }

    pub fn pause(&self) -> Result<MicrophoneInterludeStatusDto, MicrophoneInterludeError> {
        let mut inner = self.lock_inner()?;
        let mut bridge = None;
        if matches!(
            inner.state,
            MicrophoneInterludeState::Opening
                | MicrophoneInterludeState::Armed
                | MicrophoneInterludeState::Speaking
                | MicrophoneInterludeState::Hangover
        ) {
            inner.state = MicrophoneInterludeState::Disabled;
            inner.media_muted = false;
            inner.paused = true;
            bridge = inner.session.as_ref().map(|session| session.bridge.clone());
        }
        let status = status_from_inner(&inner);
        drop(inner);
        if let Some(bridge) = bridge {
            let _ = bridge.set_gate_speaking(false);
        }
        Ok(status)
    }

    pub fn resume(&self) -> Result<MicrophoneInterludeStatusDto, MicrophoneInterludeError> {
        let mut inner = self.lock_inner()?;
        if inner.state == MicrophoneInterludeState::Disabled && inner.session.is_some() {
            inner.paused = false;
            inner.state = MicrophoneInterludeState::Armed;
        }
        Ok(status_from_inner(&inner))
    }

    /// 用于验证 VAD 状态机边界；生产 Worker 直接提交真实 SpeexDSP 结果。
    #[cfg(test)]
    pub fn update_vad(
        &self,
        speaking: bool,
        hangover: bool,
    ) -> Result<MicrophoneInterludeStatusDto, MicrophoneInterludeError> {
        let mut inner = self.lock_inner()?;
        let next_state = match inner.state {
            MicrophoneInterludeState::Armed => {
                if speaking {
                    MicrophoneInterludeState::Speaking
                } else {
                    MicrophoneInterludeState::Armed
                }
            }
            MicrophoneInterludeState::Speaking => {
                if speaking {
                    MicrophoneInterludeState::Speaking
                } else if hangover {
                    MicrophoneInterludeState::Hangover
                } else {
                    MicrophoneInterludeState::Armed
                }
            }
            MicrophoneInterludeState::Hangover => {
                if speaking {
                    MicrophoneInterludeState::Speaking
                } else if hangover {
                    MicrophoneInterludeState::Hangover
                } else {
                    MicrophoneInterludeState::Armed
                }
            }
            _ => return Err(MicrophoneInterludeError::SessionNotActive),
        };
        inner.state = next_state;
        inner.media_muted = matches!(
            next_state,
            MicrophoneInterludeState::Speaking | MicrophoneInterludeState::Hangover
        );
        Ok(status_from_inner(&inner))
    }

    fn lock_inner(&self) -> Result<std::sync::MutexGuard<'_, Inner>, MicrophoneInterludeError> {
        self.inner
            .lock()
            .map_err(|_| MicrophoneInterludeError::ControllerPoisoned)
    }
}

fn status_from_inner(inner: &Inner) -> MicrophoneInterludeStatusDto {
    MicrophoneInterludeStatusDto {
        state: inner.state,
        generation: inner.generation,
        selected_device_id: inner.config.device_id.clone(),
        selected_device_name: inner.selected_device_name.clone(),
        host_api: inner.host_api.clone(),
        actual_sample_rate_hz: inner.actual_sample_rate_hz,
        input_level: inner.input_level,
        speech_probability: inner.speech_probability,
        aec_active: inner.aec_active,
        noise_suppression_active: inner.noise_suppression_active,
        agc_active: inner.agc_active,
        input_overflow_count: inner.input_overflow_count,
        output_underflow_count: inner.output_underflow_count,
        dropped_frame_count: inner.dropped_frame_count,
        media_muted: inner.media_muted,
        error_code: inner.error_code.clone(),
        error_message: inner.error_message.clone(),
    }
}

fn clear_failure(inner: &mut Inner) {
    inner.error_code = None;
    inner.error_message = None;
    inner.media_muted = false;
}

fn next_generation(previous: u64) -> u64 {
    previous.wrapping_add(1).max(1)
}

const MICROPHONE_DSP_POLL_INTERVAL: Duration = Duration::from_millis(2);
const MICROPHONE_HANGOVER_FRAMES: usize = 40;
const MICROPHONE_VAD_START_FRAMES: usize = 2;
const MICROPHONE_MAX_QUEUED_FRAMES: usize = 4;
/// SpeexDSP VAD 位于 AGC 之后；低电平 USB 麦克风底噪可能被 AGC 放大并误判
/// 为人声。这里再用原始输入 RMS/SNR 门控做非语义能量判断，只决定“有人声/无人声”。
const MICROPHONE_VAD_INITIAL_NOISE_FLOOR_RMS: f32 = 0.01;
const MICROPHONE_VAD_NOISE_FLOOR_MAX_RMS: f32 = 0.25;
const MICROPHONE_VAD_NOISE_FLOOR_ATTACK: f32 = 0.05;
const MICROPHONE_VAD_NOISE_FLOOR_RELEASE: f32 = 0.01;
/// full-duplex 进入监测后，允许设备/驱动在没有任何心跳的最长时间；
/// 首个 callback 也受同一预算约束。
/// 暂停期间不计时，避免把用户主动暂停误判成设备故障。
const MICROPHONE_CALLBACK_STALL_BUDGET: Duration = Duration::from_secs(1);
/// 连续输入溢出通常意味着输入设备已拔出或驱动不再提供有效帧；短暂的
/// Windows 调度抖动只计数，不立即停止会话。
const MICROPHONE_INPUT_OVERFLOW_BUDGET: Duration = Duration::from_millis(250);

fn callback_stall_exceeded(
    callback_seen: bool,
    callback_monitor_started: Instant,
    callback_progress_at: Instant,
    now: Instant,
) -> bool {
    let budget = MICROPHONE_CALLBACK_STALL_BUDGET;
    (callback_seen || now.saturating_duration_since(callback_monitor_started) >= budget)
        && now.saturating_duration_since(callback_progress_at) >= budget
}

#[derive(Debug, Clone, Copy)]
struct VadEnergyGate {
    noise_floor_rms: f32,
}

impl Default for VadEnergyGate {
    fn default() -> Self {
        Self {
            noise_floor_rms: MICROPHONE_VAD_INITIAL_NOISE_FLOOR_RMS,
        }
    }
}

impl VadEnergyGate {
    fn reset(&mut self) {
        self.noise_floor_rms = MICROPHONE_VAD_INITIAL_NOISE_FLOOR_RMS;
    }

    /// 将 SpeexDSP 的候选 VAD 与原始输入能量相交，避免 AGC 把环境底噪
    /// 放大后直接触发麦克风优先级。该门控不读取或理解语音内容。
    fn accept(
        &mut self,
        raw_rms: f32,
        speex_candidate: bool,
        sensitivity: MicrophoneSensitivity,
    ) -> bool {
        let raw_rms = if raw_rms.is_finite() {
            raw_rms.abs().min(MICROPHONE_VAD_NOISE_FLOOR_MAX_RMS)
        } else {
            0.0
        };
        let (absolute_floor, noise_ratio) = match sensitivity {
            MicrophoneSensitivity::Low => (0.03_f32, 2.0_f32),
            MicrophoneSensitivity::Standard => (0.02_f32, 1.8_f32),
            MicrophoneSensitivity::High => (0.015_f32, 1.5_f32),
        };
        let threshold = absolute_floor.max(self.noise_floor_rms * noise_ratio);
        let accepted = speex_candidate && raw_rms >= threshold;
        if !speex_candidate || raw_rms <= self.noise_floor_rms * 1.25 {
            let rate = if raw_rms < self.noise_floor_rms {
                MICROPHONE_VAD_NOISE_FLOOR_RELEASE
            } else {
                MICROPHONE_VAD_NOISE_FLOOR_ATTACK
            };
            self.noise_floor_rms += (raw_rms - self.noise_floor_rms) * rate;
            self.noise_floor_rms = self
                .noise_floor_rms
                .clamp(0.0, MICROPHONE_VAD_NOISE_FLOOR_MAX_RMS);
        }
        accepted
    }
}

fn frame_rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let (sum, count) = frame.iter().fold((0.0_f64, 0_u64), |(sum, count), sample| {
        if sample.is_finite() {
            (sum + f64::from(*sample) * f64::from(*sample), count + 1)
        } else {
            (sum, count)
        }
    });
    if count == 0 {
        0.0
    } else {
        (sum / count as f64).sqrt() as f32
    }
}

fn speech_dsp_config(config: &MicrophoneInterludeConfigDto) -> SpeechDspConfig {
    let (vad_start_percent, vad_continue_percent) = match config.sensitivity {
        MicrophoneSensitivity::Low => (75, 60),
        MicrophoneSensitivity::Standard => (60, 45),
        MicrophoneSensitivity::High => (50, 35),
    };
    SpeechDspConfig {
        sample_rate_hz: config.sample_rate_hz,
        frame_samples: usize::try_from(
            u64::from(config.sample_rate_hz)
                .saturating_mul(10)
                .saturating_div(1_000),
        )
        .unwrap_or(480),
        filter_length_samples: usize::try_from(
            u64::from(config.sample_rate_hz)
                .saturating_mul(200)
                .saturating_div(1_000),
        )
        .unwrap_or(9_600),
        aec_enabled: config.aec_enabled,
        noise_suppression_enabled: config.noise_suppression_enabled,
        agc_enabled: config.agc_enabled,
        vad_enabled: true,
        agc_level: 8_000,
        vad_start_percent,
        vad_continue_percent,
    }
}

fn run_microphone_worker(
    inner: Arc<Mutex<Inner>>,
    bridge: MicrophoneAudioBridge,
    cancellation: CancellationToken,
    dsp_config: SpeechDspConfig,
    input_channels: u16,
    output_channels: u16,
    on_speaking_start: Arc<dyn Fn() + Send + Sync + 'static>,
) {
    let mut dsp = match SpeechDsp::new(dsp_config) {
        Ok(dsp) => dsp,
        Err(error) => {
            fail_worker(
                &inner,
                &bridge,
                MicrophoneInterludeError::DspInitFailed,
                error.to_string(),
            );
            let _ = bridge.set_gate_enabled(false);
            return;
        }
    };
    if let Ok(mut state) = inner.lock() {
        // pause/stop 可能与 DSP 初始化并发到达；不能让初始化完成的
        // Worker 把 Disabled/Stopping 重新伪装成 Armed。
        if state.state != MicrophoneInterludeState::Stopping {
            state.aec_active = dsp_config.aec_enabled;
            state.noise_suppression_active = dsp_config.noise_suppression_enabled;
            state.agc_active = dsp_config.agc_enabled;
            state.actual_sample_rate_hz = Some(dsp_config.sample_rate_hz);
            state.state = if state.paused {
                MicrophoneInterludeState::Disabled
            } else {
                MicrophoneInterludeState::Armed
            };
        }
    }

    let frame_samples = dsp_config.frame_samples;
    let input_channels = usize::from(input_channels.max(1));
    let output_channels = usize::from(output_channels.max(1));
    let mut input_read = vec![0.0_f32; frame_samples.saturating_mul(input_channels)];
    let mut reference_read = vec![0.0_f32; frame_samples.saturating_mul(output_channels)];
    let mut input_queue = VecDeque::with_capacity(
        frame_samples.saturating_mul(input_channels) * MICROPHONE_MAX_QUEUED_FRAMES,
    );
    let mut reference_queue = VecDeque::with_capacity(
        frame_samples.saturating_mul(output_channels) * MICROPHONE_MAX_QUEUED_FRAMES,
    );
    let input_queue_limit = frame_samples
        .saturating_mul(input_channels)
        .saturating_mul(MICROPHONE_MAX_QUEUED_FRAMES);
    let reference_queue_limit = frame_samples
        .saturating_mul(output_channels)
        .saturating_mul(MICROPHONE_MAX_QUEUED_FRAMES);
    let mut input_frame = vec![0.0_f32; frame_samples];
    let mut reference_frame = vec![0.0_f32; frame_samples];
    let mut clean_frame = vec![0.0_f32; frame_samples];
    let mut clean_stereo = vec![0.0_f32; frame_samples * 2];
    let mut speech_streak = 0_usize;
    let mut silence_streak = 0_usize;
    let mut hangover_left = 0_usize;
    let mut speaking = false;
    let mut vad_energy_gate = VadEnergyGate::default();
    let mut callback_seen = false;
    let mut last_callback_count = 0_u64;
    let mut callback_monitor_started = Instant::now();
    let mut callback_progress_at = Instant::now();
    let mut last_input_overflow_count = 0_u64;
    let mut input_overflow_at = None;
    let mut last_reference_drop_count = 0_u64;
    let mut reference_drop_at = None;

    // full-duplex callback 在 Worker 启动前就可能已经运行若干次；从当前
    // 会话快照建立基线，避免把启动前的历史计数当成新故障。后续新增计数
    // 仍由下方的连续预算检查捕获。
    if let Ok(health) = bridge.input_health() {
        last_callback_count = health.callback_count;
        last_input_overflow_count = health.input_overflow_count;
        last_reference_drop_count = health.render_reference_drop_count;
    }

    while !cancellation.is_cancelled() {
        // 输出线程停止、硬件流失效或 full-duplex 拆流时会撤销桥接。
        // 不能继续轮询一个已经失效的环缓，否则会残留 Worker 且无法
        // 对上层反映真实的 fail-open 故障状态。
        if !bridge.is_available() {
            fail_worker(
                &inner,
                &bridge,
                MicrophoneInterludeError::AudioBridgeFailed,
                "PortAudio 全双工音频桥接已失效".to_owned(),
            );
            break;
        }
        if is_paused(&inner) {
            speaking = false;
            speech_streak = 0;
            silence_streak = 0;
            hangover_left = 0;
            callback_seen = false;
            callback_monitor_started = Instant::now();
            callback_progress_at = Instant::now();
            input_queue.clear();
            reference_queue.clear();
            vad_energy_gate.reset();
            let _ = bridge.set_gate_speaking(false);
            thread::sleep(MICROPHONE_DSP_POLL_INTERVAL);
            continue;
        }
        match bridge.input_health() {
            Ok(health) => {
                if health.input_overflow_count > last_input_overflow_count {
                    last_input_overflow_count = health.input_overflow_count;
                    input_overflow_at.get_or_insert_with(Instant::now);
                } else {
                    input_overflow_at = None;
                }
                if input_overflow_at.is_some_and(|started_at| {
                    started_at.elapsed() >= MICROPHONE_INPUT_OVERFLOW_BUDGET
                }) {
                    fail_worker(
                        &inner,
                        &bridge,
                        MicrophoneInterludeError::AudioBridgeFailed,
                        "PortAudio 输入连续溢出，设备可能已断开".to_owned(),
                    );
                    break;
                }
                if health.render_reference_drop_count > last_reference_drop_count {
                    last_reference_drop_count = health.render_reference_drop_count;
                    reference_drop_at.get_or_insert_with(Instant::now);
                } else {
                    reference_drop_at = None;
                }
                if reference_drop_at.is_some_and(|started_at| {
                    started_at.elapsed() >= MICROPHONE_INPUT_OVERFLOW_BUDGET
                }) {
                    fail_worker(
                        &inner,
                        &bridge,
                        MicrophoneInterludeError::AudioBridgeFailed,
                        "PortAudio 回声参考连续丢帧，设备或输出可能已失效".to_owned(),
                    );
                    break;
                }
                if health.callback_count > last_callback_count {
                    callback_seen = true;
                    last_callback_count = health.callback_count;
                    callback_progress_at = Instant::now();
                } else if callback_stall_exceeded(
                    callback_seen,
                    callback_monitor_started,
                    callback_progress_at,
                    Instant::now(),
                ) {
                    fail_worker(
                        &inner,
                        &bridge,
                        MicrophoneInterludeError::AudioBridgeFailed,
                        "PortAudio callback 心跳在停止预算内未推进".to_owned(),
                    );
                    break;
                }
            }
            Err(error) => {
                fail_worker(
                    &inner,
                    &bridge,
                    MicrophoneInterludeError::AudioBridgeFailed,
                    error,
                );
                break;
            }
        }
        match bridge.read_input_interleaved(&mut input_read) {
            Ok(read) if read > 0 => {
                push_bounded(&mut input_queue, &input_read[..read], input_queue_limit)
            }
            Ok(_) => {}
            Err(error) => {
                fail_worker(
                    &inner,
                    &bridge,
                    MicrophoneInterludeError::AudioBridgeFailed,
                    error,
                );
                break;
            }
        }
        match bridge.read_render_reference_interleaved(&mut reference_read) {
            Ok(read) if read > 0 => push_bounded(
                &mut reference_queue,
                &reference_read[..read],
                reference_queue_limit,
            ),
            Ok(_) => {}
            Err(error) => {
                fail_worker(
                    &inner,
                    &bridge,
                    MicrophoneInterludeError::AudioBridgeFailed,
                    error,
                );
                break;
            }
        }

        let reference_frames_available = reference_queue.len() / output_channels;
        if input_queue.len() < frame_samples.saturating_mul(input_channels)
            || reference_frames_available < frame_samples
        {
            thread::sleep(MICROPHONE_DSP_POLL_INTERVAL);
            continue;
        }
        for frame in 0..frame_samples {
            let input_sample = if input_channels == 1 {
                input_queue.pop_front().unwrap_or(0.0)
            } else {
                let mut sum = 0.0_f32;
                for _ in 0..input_channels {
                    sum += input_queue.pop_front().unwrap_or(0.0);
                }
                sum / input_channels as f32
            };
            input_frame[frame] = input_sample;
            let mut sum = 0.0_f32;
            for _ in 0..output_channels {
                sum += reference_queue.pop_front().unwrap_or(0.0);
            }
            reference_frame[frame] = sum / output_channels as f32;
        }
        let result = match dsp.process_interleaved_mono_f32(
            &input_frame,
            &reference_frame,
            &mut clean_frame,
        ) {
            Ok(result) => result,
            Err(error) => {
                fail_worker(
                    &inner,
                    &bridge,
                    MicrophoneInterludeError::DspProcessFailed,
                    error.to_string(),
                );
                break;
            }
        };
        let sensitivity = current_sensitivity(&inner);
        let effective_speech =
            vad_energy_gate.accept(frame_rms(&input_frame), result.speech, sensitivity);
        let result = SpeechDspFrameResult {
            speech: effective_speech,
            speech_probability: if effective_speech {
                result.speech_probability
            } else {
                0.0
            },
            input_level: result.input_level,
        };
        let start_frames = match sensitivity {
            MicrophoneSensitivity::Low => 4,
            MicrophoneSensitivity::Standard => MICROPHONE_VAD_START_FRAMES,
            MicrophoneSensitivity::High => 1,
        };
        let mut started_speaking = false;
        if speaking {
            if result.speech {
                silence_streak = 0;
                hangover_left = MICROPHONE_HANGOVER_FRAMES;
            } else if hangover_left > 0 {
                silence_streak = silence_streak.saturating_add(1);
                hangover_left = hangover_left.saturating_sub(1);
            } else {
                silence_streak = silence_streak.saturating_add(1);
                if silence_streak >= 1 {
                    speaking = false;
                    silence_streak = 0;
                }
            }
        } else if result.speech {
            speech_streak = speech_streak.saturating_add(1);
            if speech_streak >= start_frames {
                speaking = true;
                speech_streak = 0;
                hangover_left = MICROPHONE_HANGOVER_FRAMES;
                started_speaking = true;
            }
        } else {
            speech_streak = 0;
        }
        let in_hangover = speaking && !result.speech && hangover_left > 0;
        if bridge.set_gate_speaking(speaking).is_err() {
            fail_worker(
                &inner,
                &bridge,
                MicrophoneInterludeError::AudioBridgeFailed,
                "麦克风门控更新失败".to_owned(),
            );
            break;
        }
        // 先提交 callback 原子门控，再通知控制面停止文件插话/固定话术。
        // 通知可能需要等待另一个 Worker；若顺序相反，主媒体会在该等待期
        // 继续可听，违反 Speaking 后的 50ms 静音预算。
        if started_speaking && catch_unwind(AssertUnwindSafe(|| on_speaking_start())).is_err() {
            fail_worker(
                &inner,
                &bridge,
                MicrophoneInterludeError::AudioBridgeFailed,
                "麦克风优先级切换回调异常终止".to_owned(),
            );
            break;
        }
        if speaking {
            for (index, sample) in clean_frame.iter().copied().enumerate() {
                clean_stereo[index * 2] = sample;
                clean_stereo[index * 2 + 1] = sample;
            }
            if let Err(error) = bridge.write_clean_mic_stereo_interleaved(&clean_stereo) {
                fail_worker(
                    &inner,
                    &bridge,
                    MicrophoneInterludeError::AudioBridgeFailed,
                    error,
                );
                break;
            }
        }
        refresh_worker_status(&inner, &bridge, result, speaking, in_hangover);
    }
    let _ = bridge.set_gate_speaking(false);
    let _ = bridge.set_gate_enabled(false);
    if let Ok(mut state) = inner.lock() {
        if state.state != MicrophoneInterludeState::Failed {
            state.state = MicrophoneInterludeState::Disabled;
            state.media_muted = false;
        }
    }
}

fn push_bounded(queue: &mut VecDeque<f32>, samples: &[f32], limit: usize) {
    for sample in samples.iter().copied().filter(|sample| sample.is_finite()) {
        if queue.len() == limit {
            queue.pop_front();
        }
        queue.push_back(sample);
    }
}

fn current_sensitivity(inner: &Arc<Mutex<Inner>>) -> MicrophoneSensitivity {
    inner
        .lock()
        .map(|state| state.config.sensitivity)
        .unwrap_or(MicrophoneSensitivity::Standard)
}

fn is_paused(inner: &Arc<Mutex<Inner>>) -> bool {
    inner.lock().map(|state| state.paused).unwrap_or(true)
}

fn refresh_worker_status(
    inner: &Arc<Mutex<Inner>>,
    bridge: &MicrophoneAudioBridge,
    result: SpeechDspFrameResult,
    speaking: bool,
    hangover: bool,
) {
    let health = bridge.input_health().ok();
    if let Ok(mut state) = inner.lock() {
        if state.paused {
            // pause() 与当前 DSP 帧处理可能竞态；暂停请求一旦写入，不能
            // 被这次尚未提交的帧重新报告成 Speaking/Armed。
            state.state = MicrophoneInterludeState::Disabled;
            state.media_muted = false;
        } else {
            state.state = if speaking {
                if hangover {
                    MicrophoneInterludeState::Hangover
                } else {
                    MicrophoneInterludeState::Speaking
                }
            } else {
                MicrophoneInterludeState::Armed
            };
            state.media_muted = speaking;
        }
        state.input_level = result.input_level.clamp(0.0, 1.0);
        state.speech_probability = result.speech_probability.clamp(0.0, 1.0);
        if let Some(health) = health {
            state.input_overflow_count = health.input_overflow_count;
            state.output_underflow_count = health.clean_mic_underrun_count;
            state.dropped_frame_count = health
                .clean_mic_drop_count
                .saturating_add(health.render_reference_drop_count);
        }
    }
}

fn mark_worker_failure(inner: &Arc<Mutex<Inner>>, error: MicrophoneInterludeError, detail: String) {
    if let Ok(mut state) = inner.lock() {
        state.state = MicrophoneInterludeState::Failed;
        state.actual_sample_rate_hz = None;
        state.media_muted = false;
        state.aec_active = false;
        state.noise_suppression_active = false;
        state.agc_active = false;
        state.error_code = Some(error.code().to_owned());
        state.error_message = Some(format!("{}：{detail}", error.message()));
    }
}

fn fail_worker(
    inner: &Arc<Mutex<Inner>>,
    bridge: &MicrophoneAudioBridge,
    error: MicrophoneInterludeError,
    detail: String,
) {
    mark_worker_failure(inner, error, detail);
    // 让唯一 PortAudio 输出线程接管拆流并恢复 output-only；Worker 自身不
    // 直接调用控制队列，避免在 DSP 线程里等待硬件关闭。
    bridge.mark_unavailable();
}

#[derive(Debug)]
enum SessionJoinError {
    Timeout(MicrophoneInterludeSession),
    Failed,
}

fn join_session(session: MicrophoneInterludeSession) -> Result<(), SessionJoinError> {
    let started_at = Instant::now();
    while !session.handle.is_finished() && started_at.elapsed() < MAX_MICROPHONE_JOIN_BUDGET {
        thread::yield_now();
    }
    if !session.handle.is_finished() {
        return Err(SessionJoinError::Timeout(session));
    }
    session.handle.join().map_err(|_| SessionJoinError::Failed)
}

pub fn list_input_devices() -> Result<Vec<MicrophoneInputDeviceDto>, MicrophoneInterludeError> {
    // 设备枚举可以复用现有 PortAudio API；启动仍受真实 full-duplex + DSP 门禁约束，
    // 不回退 WebView，也不从网络/控制面推断麦克风设备。
    autolive_portaudio_output::list_input_devices()
        .map_err(|_| MicrophoneInterludeError::InputBackendUnavailable)?
        .into_iter()
        .map(|device| {
            Ok(MicrophoneInputDeviceDto {
                id: device.id,
                name: validate_device_name(&device.name)?,
                host_api: host_api_label(device.host_api).to_owned(),
                max_input_channels: device.max_input_channels,
                default_sample_rate_hz: device.default_sample_rate_hz,
            })
        })
        .collect()
}

fn host_api_label(host_api: autolive_portaudio_output::HostApiKind) -> &'static str {
    match host_api {
        autolive_portaudio_output::HostApiKind::Default => "default",
        autolive_portaudio_output::HostApiKind::Wasapi => "wasapi",
        autolive_portaudio_output::HostApiKind::Asio => "asio",
        autolive_portaudio_output::HostApiKind::Mme => "mme",
        autolive_portaudio_output::HostApiKind::DirectSound => "dsound",
        autolive_portaudio_output::HostApiKind::Wdmks => "wdmks",
        autolive_portaudio_output::HostApiKind::Other => "other",
    }
}

pub fn validate_device_name(name: &str) -> Result<String, MicrophoneInterludeError> {
    if name.len() > MAX_MICROPHONE_DEVICE_NAME_BYTES {
        return Err(MicrophoneInterludeError::DeviceNameTooLong);
    }
    Ok(name
        .chars()
        .filter(|character| !character.is_control())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_name_is_sanitized_at_the_boundary() {
        assert_eq!(validate_device_name("Mic\nA").unwrap(), "MicA");
        assert!(validate_device_name(&"x".repeat(257)).is_err());
    }

    #[test]
    fn sensitivity_maps_to_bounded_speech_dsp_configuration() {
        let low = speech_dsp_config(&MicrophoneInterludeConfigDto {
            sensitivity: MicrophoneSensitivity::Low,
            ..Default::default()
        });
        let high = speech_dsp_config(&MicrophoneInterludeConfigDto {
            sensitivity: MicrophoneSensitivity::High,
            ..Default::default()
        });
        assert!(low.vad_start_percent > high.vad_start_percent);
        assert!(low.validate().is_ok());
        assert!(high.validate().is_ok());
    }

    #[test]
    fn speech_dsp_configuration_uses_the_effective_device_rate() {
        let dsp = speech_dsp_config(&MicrophoneInterludeConfigDto {
            sample_rate_hz: 44_100,
            ..Default::default()
        });

        // start_microphone_interlude 在创建 full-duplex 流后会把配置采样率
        // 规范化为当前输出设备的实际采样率；DSP 的 10ms 帧和 200ms 回声
        // 滤波长度必须跟随该值，不能继续使用 UI 默认的 48kHz。
        assert_eq!(dsp.sample_rate_hz, 44_100);
        assert_eq!(dsp.frame_samples, 441);
        assert_eq!(dsp.filter_length_samples, 8_820);
        assert!(dsp.validate().is_ok());
    }

    #[test]
    fn vad_energy_gate_rejects_low_level_speex_candidates_as_noise() {
        let mut gate = VadEnergyGate::default();
        for _ in 0..120 {
            assert!(!gate.accept(0.011, true, MicrophoneSensitivity::Standard,));
        }
    }

    #[test]
    fn vad_energy_gate_accepts_near_end_signal_above_adaptive_noise_floor() {
        let mut gate = VadEnergyGate::default();
        for _ in 0..20 {
            assert!(!gate.accept(0.008, false, MicrophoneSensitivity::Standard,));
        }
        assert!(gate.accept(0.08, true, MicrophoneSensitivity::Standard));
    }

    #[test]
    fn frame_rms_ignores_non_finite_input() {
        assert_eq!(frame_rms(&[]), 0.0);
        assert_eq!(frame_rms(&[f32::NAN, f32::INFINITY]), 0.0);
        assert!((frame_rms(&[0.03, -0.03]) - 0.03).abs() < 0.000_001);
    }

    #[test]
    fn callback_stall_budget_covers_the_first_heartbeat() {
        let now = Instant::now();
        assert!(callback_stall_exceeded(
            false,
            now - MICROPHONE_CALLBACK_STALL_BUDGET,
            now - MICROPHONE_CALLBACK_STALL_BUDGET,
            now,
        ));
        assert!(!callback_stall_exceeded(
            false,
            now - Duration::from_millis(250),
            now - Duration::from_millis(250),
            now,
        ));
        assert!(callback_stall_exceeded(
            true,
            now,
            now - MICROPHONE_CALLBACK_STALL_BUDGET,
            now,
        ));
    }

    #[test]
    fn vad_state_machine_only_mutes_speaking_and_hangover() {
        let controller = MicrophoneInterludeController::default();
        {
            let mut inner = controller.inner.lock().expect("test state lock");
            inner.state = MicrophoneInterludeState::Armed;
        }

        let speaking = controller.update_vad(true, false).expect("speaking update");
        assert_eq!(speaking.state, MicrophoneInterludeState::Speaking);
        assert!(speaking.media_muted);

        let hangover = controller.update_vad(false, true).expect("hangover update");
        assert_eq!(hangover.state, MicrophoneInterludeState::Hangover);
        assert!(hangover.media_muted);

        let armed = controller.update_vad(false, false).expect("silence update");
        assert_eq!(armed.state, MicrophoneInterludeState::Armed);
        assert!(!armed.media_muted);
    }

    #[test]
    fn dsp_initialization_preserves_pause_state() {
        let controller = MicrophoneInterludeController::default();
        let mut inner = controller.inner.lock().expect("test state lock");
        inner.state = MicrophoneInterludeState::Opening;
        inner.paused = true;

        // 与 Worker 初始化完成后的状态转换保持一致：暂停中的会话仍为
        // Disabled，不能在 race 下短暂报告 Armed。
        inner.state = if inner.paused {
            MicrophoneInterludeState::Disabled
        } else {
            MicrophoneInterludeState::Armed
        };
        assert_eq!(inner.state, MicrophoneInterludeState::Disabled);
    }
}
