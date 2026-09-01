//! PortAudio 输出后端：主总线 f32 → 环缓 → 硬件回调。
//! 可选的 full-duplex 模式在同一 callback 中捕获输入，并通过有界 input/clean-mic
//! SPSC 环缓交给 DSP worker；output-only API 保持兼容。
//! 与 WebView 互斥；探测/启动失败时调用方回退 WebView。

use std::borrow::Cow;
#[cfg(autolive_has_portaudio)]
use std::collections::HashMap;
#[cfg(autolive_has_portaudio)]
use std::ffi::CStr;
#[cfg(all(windows, autolive_has_portaudio))]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU16, AtomicU64, Ordering};
use std::sync::Arc;
#[cfg(autolive_has_portaudio)]
use std::sync::{Mutex, OnceLock};

use ringbuf::{
    traits::{Consumer, Observer, Producer, Split},
    HeapCons, HeapProd, HeapRb,
};

/// PortAudio 全局会话：串行化 + 引用计数，避免探测 Terminate 掉正在播的流。
#[cfg(autolive_has_portaudio)]
fn portaudio_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(autolive_has_portaudio)]
fn pa_session_count() -> &'static AtomicU64 {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    &COUNT
}

#[cfg(autolive_has_portaudio)]
fn pa_acquire() -> Result<(), String> {
    let count = pa_session_count().fetch_add(1, Ordering::SeqCst);
    if count == 0 {
        let err = unsafe { ffi::Pa_Initialize() };
        if err != ffi::PA_NO_ERROR {
            pa_session_count().fetch_sub(1, Ordering::SeqCst);
            return Err(format!("Pa_Initialize 失败：{}", pa_error_text(err)));
        }
    }
    Ok(())
}

#[cfg(autolive_has_portaudio)]
fn pa_release() {
    let prev = pa_session_count().fetch_sub(1, Ordering::SeqCst);
    if prev == 1 {
        let _ = unsafe { ffi::Pa_Terminate() };
    } else if prev == 0 {
        pa_session_count().store(0, Ordering::SeqCst);
    }
}

pub const DEFAULT_SAMPLE_RATE_HZ: u32 = 44_100;
pub const MIN_RING_CAPACITY_KIB: u32 = 128;
pub const MAX_RING_CAPACITY_KIB: u32 = 2_048;
pub const DEFAULT_RING_CAPACITY_KIB: u32 = 1_024;
pub const MIN_FRAMES_PER_BUFFER: u32 = 128;
pub const MAX_FRAMES_PER_BUFFER: u32 = 2_048;
// 硬件回调帧数与应用内存环缓是两个独立参数，不由 UI 的内存缓冲输入控制。
pub const DEFAULT_FRAMES_PER_BUFFER: u32 = 256;
pub const DEFAULT_INPUT_CHANNELS: u16 = 1;
pub const MICROPHONE_MUTE_ATTACK_MS: u32 = 50;
pub const MICROPHONE_MUTE_RELEASE_MS: u32 = 250;
// 有效水位覆盖常见 Windows 调度抖动和至少 8 次硬件回调；环缓 KiB 仍只决定容量上限。
const MIN_PLAYBACK_WATERMARK_MS: u32 = 200;
const MAX_PLAYBACK_WATERMARK_MS: u32 = 500;
const PLAYBACK_WATERMARK_CALLBACK_COUNT: u32 = 8;

pub fn validate_ring_capacity_kib(capacity_kib: u32) -> Result<(), String> {
    if (MIN_RING_CAPACITY_KIB..=MAX_RING_CAPACITY_KIB).contains(&capacity_kib) {
        return Ok(());
    }
    Err(format!(
        "PortAudio 内存缓冲必须是 {} 到 {} KiB 的整数，收到 {}",
        MIN_RING_CAPACITY_KIB, MAX_RING_CAPACITY_KIB, capacity_kib
    ))
}

pub fn validate_frames_per_buffer(frames: u32) -> Result<(), String> {
    if (MIN_FRAMES_PER_BUFFER..=MAX_FRAMES_PER_BUFFER).contains(&frames) {
        return Ok(());
    }
    Err(format!(
        "PortAudio 缓冲区大小必须是 {} 到 {} 的整数，收到 {}",
        MIN_FRAMES_PER_BUFFER, MAX_FRAMES_PER_BUFFER, frames
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostApiKind {
    Default,
    Wasapi,
    Asio,
    Mme,
    DirectSound,
    Wdmks,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDeviceInfo {
    pub id: String,
    pub name: String,
    pub host_api: HostApiKind,
    pub max_output_channels: u16,
    pub default_sample_rate_hz: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceInfo {
    /// 由 Host API、设备名和能力字段生成的稳定绑定 ID；硬件枚举索引不暴露给 UI。
    pub id: String,
    pub name: String,
    pub host_api: HostApiKind,
    pub max_input_channels: u16,
    pub default_sample_rate_hz: u32,
}

/// 麦克风输入的最小状态。算法层只能更新原子门控，不把 VAD/ASR 实现塞进
/// PortAudio callback；实际 AEC/NS/AGC/VAD 由上层 DSP worker 负责。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrophoneGateState {
    Disabled,
    Armed,
    Speaking,
}

#[derive(Debug, Default)]
pub struct MicrophoneGate {
    enabled: AtomicBool,
    speaking: AtomicBool,
}

impl MicrophoneGate {
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
        if !enabled {
            self.speaking.store(false, Ordering::Release);
        }
    }

    pub fn set_speaking(&self, speaking: bool) {
        if self.enabled.load(Ordering::Acquire) {
            self.speaking.store(speaking, Ordering::Release);
        } else {
            self.speaking.store(false, Ordering::Release);
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    pub fn is_speaking(&self) -> bool {
        self.is_enabled() && self.speaking.load(Ordering::Acquire)
    }

    pub fn state(&self) -> MicrophoneGateState {
        if !self.is_enabled() {
            MicrophoneGateState::Disabled
        } else if self.is_speaking() {
            MicrophoneGateState::Speaking
        } else {
            MicrophoneGateState::Armed
        }
    }

    pub fn main_gain_target(&self) -> f32 {
        if self.is_speaking() {
            0.0
        } else {
            1.0
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortAudioDuplexConfig {
    pub input_device_index: Option<i32>,
    pub input_channels: u16,
}

impl Default for PortAudioDuplexConfig {
    fn default() -> Self {
        Self {
            input_device_index: None,
            input_channels: DEFAULT_INPUT_CHANNELS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortAudioInputHealth {
    pub input_channels: u16,
    /// callback 心跳；worker 可用它判断输入/输出 callback 是否仍在推进。
    pub callback_count: u64,
    /// 最近一次 PortAudio callback 状态标志（例如输入/输出 overflow/underflow）。
    pub callback_last_status_flags: u64,
    pub input_frames_captured: u64,
    pub input_overflow_count: u64,
    pub clean_mic_frames_rendered: u64,
    pub clean_mic_drop_count: u64,
    pub clean_mic_underrun_count: u64,
    pub render_reference_drop_count: u64,
    pub input_ring_len_samples: usize,
    pub input_ring_capacity_samples: usize,
    pub clean_mic_ring_len_samples: usize,
    pub clean_mic_ring_capacity_samples: usize,
    /// PortAudio callback 最近一次提供的 ADC 时间，单位为微秒。
    pub last_input_adc_time_us: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputBackendStatus {
    pub available: bool,
    pub selected_backend: &'static str,
    pub reason: Option<String>,
    pub xrun_count: u64,
}

/// PortAudio 实际硬件流状态；与应用自己的 `running` 标志分开记录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortAudioHardwareState {
    /// 当前目标未编译 PortAudio。
    Unsupported,
    /// 尚未创建 PortAudio 流。
    NotCreated,
    /// `Pa_IsStreamActive` 报告流正在运行。
    Active,
    /// 流已创建但不再活动，且 `Pa_IsStreamStopped` 报告已停止。
    Stopped,
    /// 流已创建但当前不活动，PortAudio 没有报告为 stopped。
    Inactive,
    /// 两个 PortAudio 查询结果不构成已知状态。
    Unknown,
    /// 查询函数返回 PortAudio 错误码。
    QueryError(i32),
}

/// 供上层诊断使用的最小无锁快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortAudioStreamHealth {
    pub application_running: bool,
    pub hardware_state: PortAudioHardwareState,
    /// PortAudio `PaStreamInfo.outputLatency`，单位为微秒。
    pub output_latency_us: Option<u64>,
    /// PortAudio `PaStreamInfo.sampleRate`，单位为 Hz。
    pub actual_sample_rate_hz: Option<u32>,
    pub callback_count: u64,
    pub callback_last_status_flags: u64,
    pub callback_status_flags_count: u64,
    /// 最近一次 callback 的 `outputBufferDacTime - currentTime`，单位为微秒。
    pub callback_output_buffer_dac_time_delta_us: i64,
    /// callback 从 SPSC 环缓实际取出的 PCM frame 累计数。
    pub callback_pcm_frames_total: u64,
    pub xrun_count: u64,
    pub callback_underrun_count: u64,
    pub producer_drop_count: u64,
    pub ring_len_samples: usize,
    pub ring_capacity_samples: usize,
}

/// 混音线程写入环缓时使用的轻量消费进度快照。
///
/// 这里只读取原子 callback 计数和 SPSC 环缓水位，不查询 PortAudio 硬件状态，
/// 避免高频重试路径触碰控制面查询。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortAudioOutputProgress {
    pub callback_count: u64,
    pub ring_len_samples: usize,
}

pub fn ring_capacity_samples(capacity_kib: u32, channels: u16) -> usize {
    let channels = usize::from(channels.max(1));
    let samples = usize::try_from(u64::from(capacity_kib).saturating_mul(1_024))
        .unwrap_or(usize::MAX)
        / std::mem::size_of::<f32>();
    samples.saturating_sub(samples % channels).max(channels)
}

pub fn playback_watermark_samples(
    sample_rate_hz: u32,
    channels: u16,
    frames_per_buffer: u32,
) -> usize {
    let channels = usize::from(channels.max(1));
    let sample_rate_hz = sample_rate_hz.max(1);
    let minimum_frames =
        u64::from(sample_rate_hz).saturating_mul(u64::from(MIN_PLAYBACK_WATERMARK_MS)) / 1_000;
    let callback_frames =
        u64::from(frames_per_buffer).saturating_mul(u64::from(PLAYBACK_WATERMARK_CALLBACK_COUNT));
    let maximum_frames =
        u64::from(sample_rate_hz).saturating_mul(u64::from(MAX_PLAYBACK_WATERMARK_MS)) / 1_000;
    let target_frames = minimum_frames.max(callback_frames).min(maximum_frames);
    let samples =
        usize::try_from(target_frames.saturating_mul(channels as u64)).unwrap_or(usize::MAX);
    samples.saturating_sub(samples % channels).max(channels)
}

#[cfg(autolive_has_portaudio)]
mod ffi {
    use std::os::raw::{c_char, c_double, c_int, c_ulong, c_void};

    pub type PaError = c_int;
    pub type PaDeviceIndex = c_int;
    pub type PaHostApiIndex = c_int;
    pub type PaSampleFormat = c_ulong;
    pub type PaStream = c_void;
    pub type PaStreamFlags = c_ulong;
    pub type PaStreamCallbackFlags = c_ulong;

    pub const PA_NO_ERROR: PaError = 0;
    pub const PA_NO_DEVICE: PaDeviceIndex = -1;
    pub const PA_FLOAT32: PaSampleFormat = 0x0000_0001;
    pub const PA_CLIP_OFF: PaStreamFlags = 0x0000_0001;

    pub const PA_WASAPI: c_int = 13;
    pub const PA_ASIO: c_int = 3;
    pub const PA_WMME: c_int = 2;
    pub const PA_DIRECTSOUND: c_int = 1;
    pub const PA_WDMKS: c_int = 11;

    #[repr(C)]
    pub struct PaDeviceInfo {
        pub struct_version: c_int,
        pub name: *const c_char,
        pub host_api: PaHostApiIndex,
        pub max_input_channels: c_int,
        pub max_output_channels: c_int,
        pub default_low_input_latency: c_double,
        pub default_low_output_latency: c_double,
        pub default_high_input_latency: c_double,
        pub default_high_output_latency: c_double,
        pub default_sample_rate: c_double,
    }

    #[repr(C)]
    pub struct PaHostApiInfo {
        pub struct_version: c_int,
        pub type_: c_int,
        pub name: *const c_char,
        pub device_count: c_int,
        pub default_input_device: PaDeviceIndex,
        pub default_output_device: PaDeviceIndex,
    }

    #[repr(C)]
    pub struct PaStreamParameters {
        pub device: PaDeviceIndex,
        pub channel_count: c_int,
        pub sample_format: PaSampleFormat,
        pub suggested_latency: c_double,
        pub host_api_specific_stream_info: *mut c_void,
    }

    #[repr(C)]
    pub struct PaStreamCallbackTimeInfo {
        pub input_buffer_adc_time: c_double,
        pub current_time: c_double,
        pub output_buffer_dac_time: c_double,
    }

    #[repr(C)]
    pub struct PaStreamInfo {
        pub struct_version: c_int,
        pub input_latency: c_double,
        pub output_latency: c_double,
        pub sample_rate: c_double,
    }

    pub type PaStreamCallback = unsafe extern "C" fn(
        input: *const c_void,
        output: *mut c_void,
        frame_count: c_ulong,
        time_info: *const PaStreamCallbackTimeInfo,
        status_flags: PaStreamCallbackFlags,
        user_data: *mut c_void,
    ) -> c_int;

    #[link(name = "portaudio_x64")]
    unsafe extern "C" {
        pub fn Pa_Initialize() -> PaError;
        pub fn Pa_Terminate() -> PaError;
        pub fn Pa_GetErrorText(error_code: PaError) -> *const c_char;
        pub fn Pa_GetDeviceCount() -> PaDeviceIndex;
        pub fn Pa_GetDefaultInputDevice() -> PaDeviceIndex;
        pub fn Pa_GetDefaultOutputDevice() -> PaDeviceIndex;
        pub fn Pa_GetDeviceInfo(device: PaDeviceIndex) -> *const PaDeviceInfo;
        pub fn Pa_GetHostApiInfo(host_api: PaHostApiIndex) -> *const PaHostApiInfo;
        pub fn Pa_IsFormatSupported(
            input_parameters: *const PaStreamParameters,
            output_parameters: *const PaStreamParameters,
            sample_rate: c_double,
        ) -> PaError;
        pub fn Pa_OpenStream(
            stream: *mut *mut PaStream,
            input_parameters: *const PaStreamParameters,
            output_parameters: *const PaStreamParameters,
            sample_rate: c_double,
            frames_per_buffer: c_ulong,
            stream_flags: PaStreamFlags,
            stream_callback: Option<PaStreamCallback>,
            user_data: *mut c_void,
        ) -> PaError;
        pub fn Pa_CloseStream(stream: *mut PaStream) -> PaError;
        pub fn Pa_StartStream(stream: *mut PaStream) -> PaError;
        pub fn Pa_StopStream(stream: *mut PaStream) -> PaError;
        pub fn Pa_IsStreamActive(stream: *mut PaStream) -> PaError;
        pub fn Pa_IsStreamStopped(stream: *mut PaStream) -> PaError;
        pub fn Pa_GetStreamInfo(stream: *mut PaStream) -> *const PaStreamInfo;
    }

    pub const PA_CONTINUE: c_int = 0;
}

#[cfg(autolive_has_portaudio)]
fn ensure_portaudio_dll_search_path() {
    #[cfg(all(windows, autolive_has_portaudio))]
    {
        static ONCE: AtomicBool = AtomicBool::new(false);
        if ONCE.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(dir) = std::env::var("AUTOLIVE_PORTAUDIO_VENDOR") {
            candidates.push(PathBuf::from(dir).join("bin"));
        }
        candidates.push(PathBuf::from(env!("AUTOLIVE_PORTAUDIO_DLL_DIR")));
        // 发布包：EXE 同目录 / portaudio 子目录 / 资源目录
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                candidates.push(parent.to_path_buf());
                candidates.push(parent.join("portaudio"));
                candidates.push(parent.join("resources").join("portaudio"));
            }
        }
        for dir in candidates {
            let dll = dir.join("portaudio_x64.dll");
            if !dll.is_file() {
                continue;
            }
            // SAFETY: 仅追加 DLL 搜索目录；失败忽略，后续 Pa_Initialize 会报错。
            unsafe {
                use std::os::windows::ffi::OsStrExt;
                let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
                windows_add_dll_directory(wide.as_ptr());
            }
            std::env::set_var(
                "PATH",
                format!(
                    "{};{}",
                    dir.display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            );
            return;
        }
    }
}

#[cfg(all(windows, autolive_has_portaudio))]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetDllDirectoryW(path: *const u16) -> i32;
}

#[cfg(all(windows, autolive_has_portaudio))]
unsafe fn windows_add_dll_directory(path: *const u16) {
    // ponytail: SetDllDirectoryW 足够；AddDllDirectory 要改进程默认搜索策略
    let _ = SetDllDirectoryW(path);
}

#[cfg(autolive_has_portaudio)]
fn pa_error_text(code: i32) -> String {
    // SAFETY: Pa_GetErrorText 返回静态字符串。
    unsafe {
        let ptr = ffi::Pa_GetErrorText(code);
        if ptr.is_null() {
            return format!("PaError({code})");
        }
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

#[cfg(autolive_has_portaudio)]
fn checked_device_count(count: ffi::PaDeviceIndex) -> Result<ffi::PaDeviceIndex, String> {
    if count < 0 {
        return Err(format!("Pa_GetDeviceCount 失败：{}", pa_error_text(count)));
    }
    Ok(count)
}

#[cfg(autolive_has_portaudio)]
fn host_api_kind_from_type(type_id: i32) -> HostApiKind {
    match type_id {
        ffi::PA_WASAPI => HostApiKind::Wasapi,
        ffi::PA_ASIO => HostApiKind::Asio,
        ffi::PA_WMME => HostApiKind::Mme,
        ffi::PA_DIRECTSOUND => HostApiKind::DirectSound,
        ffi::PA_WDMKS => HostApiKind::Wdmks,
        _ => HostApiKind::Other,
    }
}

#[cfg(autolive_has_portaudio)]
fn query_portaudio_stream_state(stream: *mut ffi::PaStream) -> PortAudioHardwareState {
    if stream.is_null() {
        return PortAudioHardwareState::NotCreated;
    }
    // SAFETY: `stream` 由 PortAudioOutput 持有；调用方在同一控制锁内查询，
    // stop() 关闭流前不会取走该指针。两个查询函数只读取 PortAudio 流状态。
    let active = unsafe { ffi::Pa_IsStreamActive(stream) };
    let stopped = unsafe { ffi::Pa_IsStreamStopped(stream) };
    if active < 0 {
        return PortAudioHardwareState::QueryError(active);
    }
    if stopped < 0 {
        return PortAudioHardwareState::QueryError(stopped);
    }
    match (active, stopped) {
        (1, 0) => PortAudioHardwareState::Active,
        (0, 1) => PortAudioHardwareState::Stopped,
        (0, 0) => PortAudioHardwareState::Inactive,
        _ => PortAudioHardwareState::Unknown,
    }
}

#[cfg(any(test, autolive_has_portaudio))]
fn latency_seconds_to_micros(seconds: f64) -> Option<u64> {
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    let micros = seconds * 1_000_000.0;
    Some(if micros >= u64::MAX as f64 {
        u64::MAX
    } else {
        micros.round() as u64
    })
}

#[cfg(any(test, autolive_has_portaudio))]
fn actual_sample_rate_hz(sample_rate_hz: f64) -> Option<u32> {
    if !sample_rate_hz.is_finite() || sample_rate_hz <= 0.0 {
        return None;
    }
    Some(if sample_rate_hz >= u32::MAX as f64 {
        u32::MAX
    } else {
        sample_rate_hz.round() as u32
    })
}

#[cfg(any(test, autolive_has_portaudio))]
fn callback_output_buffer_dac_time_delta_us(output_buffer_dac_time: f64, current_time: f64) -> i64 {
    let delta_us = (output_buffer_dac_time - current_time) * 1_000_000.0;
    if !delta_us.is_finite() {
        return 0;
    }
    delta_us.round().clamp(i64::MIN as f64, i64::MAX as f64) as i64
}

#[cfg(autolive_has_portaudio)]
fn query_portaudio_stream_info(stream: *mut ffi::PaStream) -> (Option<u64>, Option<u32>) {
    if stream.is_null() {
        return (None, None);
    }
    // SAFETY: `stream` 由 PortAudioOutput 持有；调用方在同一控制锁内查询，
    // stop() 关闭流前不会取走该指针。该 API 只读取流信息，不在 callback 中调用。
    let info = unsafe { ffi::Pa_GetStreamInfo(stream) };
    if info.is_null() {
        return (None, None);
    }
    let info = unsafe { &*info };
    (
        latency_seconds_to_micros(info.output_latency),
        actual_sample_rate_hz(info.sample_rate),
    )
}

/// 探测 PortAudio；无 vendor/初始化失败 → WebView 回退。
pub fn probe_portaudio() -> OutputBackendStatus {
    #[cfg(not(autolive_has_portaudio))]
    {
        OutputBackendStatus {
            available: false,
            selected_backend: "webview",
            reason: Some("PortAudio vendor 未编译进当前目标；当前回退 WebView 播放。".to_owned()),
            xrun_count: 0,
        }
    }
    #[cfg(autolive_has_portaudio)]
    {
        ensure_portaudio_dll_search_path();
        let _guard = portaudio_lock().lock().unwrap_or_else(|e| e.into_inner());
        if let Err(reason) = pa_acquire() {
            return OutputBackendStatus {
                available: false,
                selected_backend: "webview",
                reason: Some(reason),
                xrun_count: 0,
            };
        }
        let count = unsafe { ffi::Pa_GetDeviceCount() };
        let default_out = unsafe { ffi::Pa_GetDefaultOutputDevice() };
        pa_release();
        if count <= 0 || default_out == ffi::PA_NO_DEVICE {
            return OutputBackendStatus {
                available: false,
                selected_backend: "webview",
                reason: Some("PortAudio 无可用输出设备".to_owned()),
                xrun_count: 0,
            };
        }
        OutputBackendStatus {
            available: true,
            selected_backend: "portaudio",
            reason: None,
            xrun_count: 0,
        }
    }
}

pub fn list_output_devices() -> Result<Vec<OutputDeviceInfo>, String> {
    #[cfg(not(autolive_has_portaudio))]
    {
        Err("PortAudio 未启用：无设备列表".to_owned())
    }
    #[cfg(autolive_has_portaudio)]
    {
        ensure_portaudio_dll_search_path();
        let _guard = portaudio_lock().lock().unwrap_or_else(|e| e.into_inner());
        pa_acquire()?;
        let count = match checked_device_count(unsafe { ffi::Pa_GetDeviceCount() }) {
            Ok(count) => count,
            Err(error) => {
                pa_release();
                return Err(error);
            }
        };
        let mut devices = Vec::new();
        if count > 0 {
            for index in 0..count {
                let info_ptr = unsafe { ffi::Pa_GetDeviceInfo(index) };
                if info_ptr.is_null() {
                    continue;
                }
                let info = unsafe { &*info_ptr };
                if info.max_output_channels <= 0 {
                    continue;
                }
                let name = if info.name.is_null() {
                    format!("device-{index}")
                } else {
                    unsafe { CStr::from_ptr(info.name).to_string_lossy().into_owned() }
                };
                let host_api = unsafe { ffi::Pa_GetHostApiInfo(info.host_api) };
                let host_kind = if host_api.is_null() {
                    HostApiKind::Other
                } else {
                    host_api_kind_from_type(unsafe { (*host_api).type_ })
                };
                devices.push(OutputDeviceInfo {
                    id: index.to_string(),
                    name,
                    host_api: host_kind,
                    max_output_channels: info.max_output_channels.max(0) as u16,
                    default_sample_rate_hz: info.default_sample_rate.round().max(1.0) as u32,
                });
            }
        }
        pa_release();
        Ok(devices)
    }
}

pub fn list_input_devices() -> Result<Vec<InputDeviceInfo>, String> {
    #[cfg(not(autolive_has_portaudio))]
    {
        Err("PortAudio 未启用：无输入设备列表".to_owned())
    }
    #[cfg(autolive_has_portaudio)]
    {
        enumerate_input_devices_with_indices()
            .map(|devices| devices.into_iter().map(|(_, device)| device).collect())
    }
}

/// 返回当前 PortAudio 默认输入设备及其本次枚举索引。
///
/// 索引只用于同一次启动请求的 native 打开，不向 UI 或持久化配置暴露；
/// 调用方仍以稳定 ID 保存用户选择。默认设备不存在时返回 `None`，由上层
/// 给出可恢复的“无输入设备”错误。
pub fn default_input_device() -> Result<Option<(i32, InputDeviceInfo)>, String> {
    #[cfg(not(autolive_has_portaudio))]
    {
        Err("PortAudio 未启用：无默认输入设备".to_owned())
    }
    #[cfg(autolive_has_portaudio)]
    {
        let default_index = {
            ensure_portaudio_dll_search_path();
            let _guard = portaudio_lock().lock().unwrap_or_else(|e| e.into_inner());
            pa_acquire()?;
            let index = unsafe { ffi::Pa_GetDefaultInputDevice() };
            pa_release();
            index
        };
        if default_index == ffi::PA_NO_DEVICE {
            return Ok(None);
        }
        Ok(enumerate_input_devices_with_indices()?
            .into_iter()
            .find(|(index, _)| *index == default_index))
    }
}

/// 将 UI 持久化的稳定设备 ID 解析为当前 PortAudio 设备索引。
///
/// PortAudio 的 native API 只接受会随枚举变化的整数索引，因此索引只在
/// 本次打开前短暂存在；找不到稳定指纹时返回 None，调用方必须报告设备失效。
pub fn input_device_index_for_id(id: &str) -> Result<Option<i32>, String> {
    #[cfg(not(autolive_has_portaudio))]
    {
        let _ = id;
        Err("PortAudio 未启用：无输入设备列表".to_owned())
    }
    #[cfg(autolive_has_portaudio)]
    {
        enumerate_input_devices_with_indices().map(|devices| {
            devices
                .into_iter()
                .find(|(_, device)| device.id == id)
                .map(|(index, _)| index)
        })
    }
}

#[cfg(autolive_has_portaudio)]
fn enumerate_input_devices_with_indices() -> Result<Vec<(i32, InputDeviceInfo)>, String> {
    ensure_portaudio_dll_search_path();
    let _guard = portaudio_lock().lock().unwrap_or_else(|e| e.into_inner());
    pa_acquire()?;
    let count = match checked_device_count(unsafe { ffi::Pa_GetDeviceCount() }) {
        Ok(count) => count,
        Err(error) => {
            pa_release();
            return Err(error);
        }
    };
    let mut devices = Vec::new();
    let mut identity_occurrences = HashMap::<String, u32>::new();
    if count > 0 {
        for index in 0..count {
            let info_ptr = unsafe { ffi::Pa_GetDeviceInfo(index) };
            if info_ptr.is_null() {
                continue;
            }
            let info = unsafe { &*info_ptr };
            if info.max_input_channels <= 0 {
                continue;
            }
            let name = if info.name.is_null() {
                format!("device-{index}")
            } else {
                unsafe { CStr::from_ptr(info.name).to_string_lossy().into_owned() }
            };
            let host_api = unsafe { ffi::Pa_GetHostApiInfo(info.host_api) };
            let host_kind = if host_api.is_null() {
                HostApiKind::Other
            } else {
                host_api_kind_from_type(unsafe { (*host_api).type_ })
            };
            let max_input_channels = info.max_input_channels.max(0) as u16;
            let default_sample_rate_hz = info.default_sample_rate.round().max(1.0) as u32;
            let identity = format!(
                "{}\0{}\0{}\0{}",
                host_api_key(host_kind),
                name,
                max_input_channels,
                default_sample_rate_hz
            );
            let occurrence = identity_occurrences.entry(identity).or_insert(0);
            let device_id = stable_input_device_id_with_occurrence(
                host_kind,
                &name,
                max_input_channels,
                default_sample_rate_hz,
                *occurrence,
            );
            *occurrence = occurrence.saturating_add(1);
            devices.push((
                index,
                InputDeviceInfo {
                    id: device_id,
                    name,
                    host_api: host_kind,
                    max_input_channels,
                    default_sample_rate_hz,
                },
            ));
        }
    }
    pa_release();
    Ok(devices)
}

#[cfg(test)]
fn stable_input_device_id(
    host_api: HostApiKind,
    name: &str,
    max_input_channels: u16,
    default_sample_rate_hz: u32,
) -> String {
    stable_input_device_id_with_occurrence(
        host_api,
        name,
        max_input_channels,
        default_sample_rate_hz,
        0,
    )
}

#[cfg(any(test, autolive_has_portaudio))]
fn stable_input_device_id_with_occurrence(
    host_api: HostApiKind,
    name: &str,
    max_input_channels: u16,
    default_sample_rate_hz: u32,
    occurrence: u32,
) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut update = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for byte in host_api_key(host_api)
        .bytes()
        .chain(std::iter::once(0))
        .chain(name.bytes())
        .chain(std::iter::once(0))
        .chain(max_input_channels.to_le_bytes())
        .chain(default_sample_rate_hz.to_le_bytes())
    {
        update(byte);
    }
    // 保留 occurrence=0 的历史 ID，只有确实重复的身份才追加消歧输入，
    // 避免已有用户配置在升级后无故失效。
    if occurrence > 0 {
        for byte in occurrence.to_le_bytes() {
            update(byte);
        }
    }
    format!("pa-input-{}-{hash:016x}", host_api_key(host_api))
}

#[cfg(any(test, autolive_has_portaudio))]
fn host_api_key(host_api: HostApiKind) -> &'static str {
    match host_api {
        HostApiKind::Default => "default",
        HostApiKind::Wasapi => "wasapi",
        HostApiKind::Asio => "asio",
        HostApiKind::Mme => "mme",
        HostApiKind::DirectSound => "dsound",
        HostApiKind::Wdmks => "wdmks",
        HostApiKind::Other => "other",
    }
}

struct StreamShared {
    xrun_count: AtomicU64,
    callback_underrun_count: AtomicU64,
    producer_drop_count: AtomicU64,
    callback_count: AtomicU64,
    callback_last_status_flags: AtomicU64,
    callback_status_flags_count: AtomicU64,
    callback_output_buffer_dac_time_delta_us: AtomicI64,
    callback_pcm_frames_total: AtomicU64,
    channels: AtomicU16,
    clear_requested: AtomicBool,
    /// 暂停硬件取样但保留环缓内容；暂停期间 callback 只填零，不计为欠载。
    callback_paused: AtomicBool,
    /// 测试音期间暂停跟播 PCM，避免叠音。
    live_pcm_paused: AtomicBool,
    /// DSP worker 更新、PortAudio callback 读取的无锁麦克风门控。
    microphone_gate: Arc<MicrophoneGate>,
    input_channels: AtomicU16,
    input_frames_captured: AtomicU64,
    input_overflow_count: AtomicU64,
    last_input_adc_time_us: AtomicI64,
    clean_mic_frames_rendered: AtomicU64,
    clean_mic_drop_count: AtomicU64,
    clean_mic_underrun_count: AtomicU64,
    render_reference_drop_count: AtomicU64,
}

impl std::fmt::Debug for StreamShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamShared")
            .field("xrun_count", &self.xrun_count.load(Ordering::Relaxed))
            .field(
                "callback_underrun_count",
                &self.callback_underrun_count.load(Ordering::Relaxed),
            )
            .field(
                "producer_drop_count",
                &self.producer_drop_count.load(Ordering::Relaxed),
            )
            .field(
                "callback_count",
                &self.callback_count.load(Ordering::Relaxed),
            )
            .field(
                "callback_last_status_flags",
                &self.callback_last_status_flags.load(Ordering::Relaxed),
            )
            .field(
                "callback_status_flags_count",
                &self.callback_status_flags_count.load(Ordering::Relaxed),
            )
            .field(
                "callback_output_buffer_dac_time_delta_us",
                &self
                    .callback_output_buffer_dac_time_delta_us
                    .load(Ordering::Relaxed),
            )
            .field(
                "callback_pcm_frames_total",
                &self.callback_pcm_frames_total.load(Ordering::Relaxed),
            )
            .field("channels", &self.channels.load(Ordering::Relaxed))
            .field(
                "input_channels",
                &self.input_channels.load(Ordering::Relaxed),
            )
            .field(
                "input_frames_captured",
                &self.input_frames_captured.load(Ordering::Relaxed),
            )
            .field(
                "input_overflow_count",
                &self.input_overflow_count.load(Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

impl StreamShared {
    /// 清除一次 full-duplex 会话独有的输入、参考和 clean-mic 统计。
    ///
    /// `StreamShared` 与 output-only 流共享生命周期，不能让上一轮麦克风
    /// 会话的 overflow/drop 计数污染下一轮的故障门禁或 UI 状态。
    fn reset_microphone_metrics(&self) {
        self.input_frames_captured.store(0, Ordering::Relaxed);
        self.input_overflow_count.store(0, Ordering::Relaxed);
        self.last_input_adc_time_us
            .store(i64::MIN, Ordering::Relaxed);
        self.clean_mic_frames_rendered.store(0, Ordering::Relaxed);
        self.clean_mic_drop_count.store(0, Ordering::Relaxed);
        self.clean_mic_underrun_count.store(0, Ordering::Relaxed);
        self.render_reference_drop_count.store(0, Ordering::Relaxed);
        self.microphone_gate.set_enabled(false);
    }

    #[cfg(test)]
    fn for_test(channels: u16) -> Self {
        Self {
            xrun_count: AtomicU64::new(0),
            callback_underrun_count: AtomicU64::new(0),
            producer_drop_count: AtomicU64::new(0),
            callback_count: AtomicU64::new(0),
            callback_last_status_flags: AtomicU64::new(0),
            callback_status_flags_count: AtomicU64::new(0),
            callback_output_buffer_dac_time_delta_us: AtomicI64::new(0),
            callback_pcm_frames_total: AtomicU64::new(0),
            channels: AtomicU16::new(channels.max(1)),
            clear_requested: AtomicBool::new(false),
            callback_paused: AtomicBool::new(false),
            live_pcm_paused: AtomicBool::new(false),
            microphone_gate: Arc::new(MicrophoneGate::default()),
            input_channels: AtomicU16::new(DEFAULT_INPUT_CHANNELS),
            input_frames_captured: AtomicU64::new(0),
            input_overflow_count: AtomicU64::new(0),
            last_input_adc_time_us: AtomicI64::new(i64::MIN),
            clean_mic_frames_rendered: AtomicU64::new(0),
            clean_mic_drop_count: AtomicU64::new(0),
            clean_mic_underrun_count: AtomicU64::new(0),
            render_reference_drop_count: AtomicU64::new(0),
        }
    }
}

#[cfg(autolive_has_portaudio)]
struct CallbackUserData {
    shared: Arc<StreamShared>,
    consumer: HeapCons<f32>,
    input_producer: Option<HeapProd<f32>>,
    clean_mic_consumer: Option<HeapCons<f32>>,
    render_reference_producer: Option<HeapProd<f32>>,
    /// callback 线程私有，避免跨线程锁或原子浮点数。
    microphone_main_gain: f32,
    sample_rate_hz: u32,
}

#[cfg(any(test, autolive_has_portaudio))]
fn pop_audio_samples(consumer: &mut HeapCons<f32>, output: &mut [f32]) -> usize {
    let filled = consumer.pop_slice(output);
    if filled < output.len() {
        output[filled..].fill(0.0);
    }
    filled
}

#[cfg(any(test, autolive_has_portaudio))]
fn apply_microphone_main_gain(
    gain: &mut f32,
    speaking: bool,
    output: &mut [f32],
    sample_rate_hz: u32,
    channels: usize,
) {
    let channels = channels.max(1);
    let sample_rate_hz = sample_rate_hz.max(1) as f32;
    let attack_step = 1.0 / (sample_rate_hz * MICROPHONE_MUTE_ATTACK_MS as f32 / 1_000.0);
    let release_step = 1.0 / (sample_rate_hz * MICROPHONE_MUTE_RELEASE_MS as f32 / 1_000.0);
    for frame in output.chunks_exact_mut(channels) {
        if speaking {
            *gain = (*gain - attack_step).max(0.0);
        } else {
            *gain = (*gain + release_step).min(1.0);
        }
        for sample in frame {
            *sample *= *gain;
        }
    }
}

#[cfg(any(test, autolive_has_portaudio))]
fn reset_microphone_gain_if_disabled(gain: &mut f32, gate: &MicrophoneGate) {
    if !gate.is_enabled() {
        *gain = 1.0;
    }
}

#[cfg(any(test, autolive_has_portaudio))]
fn mix_clean_mic_samples(
    consumer: &mut HeapCons<f32>,
    output: &mut [f32],
    speaking: bool,
    shared: &StreamShared,
    channels: usize,
) {
    let mut missing = false;
    for sample in output.iter_mut() {
        let mic = consumer.try_pop().unwrap_or_else(|| {
            missing = true;
            0.0
        });
        if speaking {
            *sample = (*sample + mic).clamp(-1.0, 1.0);
        }
    }
    if speaking {
        shared
            .clean_mic_frames_rendered
            .fetch_add((output.len() / channels.max(1)) as u64, Ordering::Relaxed);
        if missing {
            shared
                .clean_mic_underrun_count
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
fn mix_clean_mic_into_output(
    consumer: &mut HeapCons<f32>,
    output: &mut [f32],
    gain: &mut f32,
    shared: &StreamShared,
    channels: usize,
    sample_rate_hz: u32,
) {
    let speaking = shared.microphone_gate.is_speaking();
    apply_microphone_main_gain(gain, speaking, output, sample_rate_hz, channels);
    mix_clean_mic_samples(consumer, output, speaking, shared, channels);
}

#[cfg(any(test, autolive_has_portaudio))]
fn input_adc_time_to_micros(seconds: f64) -> Option<i64> {
    if !seconds.is_finite() {
        return None;
    }
    let micros = seconds * 1_000_000.0;
    if !micros.is_finite() {
        return None;
    }
    Some(micros.round().clamp(i64::MIN as f64, i64::MAX as f64) as i64)
}

fn take_callback_after_confirmed_close<T>(
    callback_data: &mut Option<T>,
    close_succeeded: bool,
) -> Option<T> {
    close_succeeded.then(|| callback_data.take()).flatten()
}

fn should_attempt_stream_close(has_stream: bool, close_quarantined: bool) -> bool {
    has_stream && !close_quarantined
}

fn stereo_samples_for_output_samples(
    output_samples: usize,
    output_channels: usize,
) -> Option<usize> {
    if output_channels == 0 || !output_samples.is_multiple_of(output_channels) {
        return None;
    }
    output_samples.checked_div(output_channels)?.checked_mul(2)
}

fn prepare_mixer_samples(
    samples: &[f32],
    output_channels: usize,
) -> Result<Cow<'_, [f32]>, String> {
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err("PortAudio 输入包含 NaN 或 Infinity".to_owned());
    }
    match output_channels {
        1 => {
            if !samples.len().is_multiple_of(2) {
                return Err("单声道 PortAudio 需要完整的立体声帧".to_owned());
            }
            Ok(Cow::Owned(
                samples
                    .chunks_exact(2)
                    .map(|frame| (frame[0] + frame[1]) * 0.5)
                    .collect(),
            ))
        }
        2 => {
            if !samples.len().is_multiple_of(2) {
                return Err("立体声 PortAudio 输入未按完整帧对齐".to_owned());
            }
            Ok(Cow::Borrowed(samples))
        }
        _ => Err(format!("不支持的 PortAudio 输出声道数：{output_channels}")),
    }
}

fn validate_interleaved_samples(samples: &[f32], channels: usize) -> Result<(), String> {
    if channels == 0 || !samples.len().is_multiple_of(channels) {
        return Err(format!("PortAudio 输入未按 {channels} 声道帧对齐"));
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err("PortAudio 输入包含 NaN 或 Infinity".to_owned());
    }
    Ok(())
}

#[cfg(autolive_has_portaudio)]
impl std::fmt::Debug for CallbackUserData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackUserData").finish_non_exhaustive()
    }
}

#[cfg(any(test, autolive_has_portaudio))]
fn record_callback_status(shared: &StreamShared, status_flags: u64) {
    shared.callback_count.fetch_add(1, Ordering::Relaxed);
    shared
        .callback_last_status_flags
        .store(status_flags, Ordering::Relaxed);
    if status_flags != 0 {
        shared
            .callback_status_flags_count
            .fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(any(test, autolive_has_portaudio))]
fn record_callback_timing_and_frames(
    shared: &StreamShared,
    dac_time_delta_us: Option<i64>,
    pcm_samples_taken: usize,
    channels: usize,
) {
    if let Some(delta_us) = dac_time_delta_us {
        shared
            .callback_output_buffer_dac_time_delta_us
            .store(delta_us, Ordering::Relaxed);
    }
    let frames_taken = pcm_samples_taken / channels.max(1);
    shared
        .callback_pcm_frames_total
        .fetch_add(frames_taken as u64, Ordering::Relaxed);
}

#[cfg(autolive_has_portaudio)]
unsafe extern "C" fn output_callback(
    input: *const std::os::raw::c_void,
    output: *mut std::os::raw::c_void,
    frame_count: std::os::raw::c_ulong,
    time_info: *const ffi::PaStreamCallbackTimeInfo,
    status_flags: ffi::PaStreamCallbackFlags,
    user_data: *mut std::os::raw::c_void,
) -> std::os::raw::c_int {
    if output.is_null() || user_data.is_null() {
        return ffi::PA_CONTINUE;
    }
    // SAFETY: PortAudio owns the callback thread and invokes it serially while the
    // stream is running. `user_data` points to the Box held by PortAudioOutput;
    // stop() stops/closes the stream before taking that Box back.
    let data = &mut *(user_data as *mut CallbackUserData);
    record_callback_status(&data.shared, status_flags as u64);
    if data.shared.clear_requested.swap(false, Ordering::AcqRel) {
        data.consumer.clear();
    }
    let channels = usize::from(data.shared.channels.load(Ordering::Relaxed).max(1));
    let samples = (frame_count as usize).saturating_mul(channels);
    let out = std::slice::from_raw_parts_mut(output as *mut f32, samples);
    let dac_time_delta_us = if time_info.is_null() {
        None
    } else {
        // SAFETY: PortAudio provides a valid callback time-info struct for the
        // duration of this callback; only plain f64 fields are read.
        let time_info = &*time_info;
        Some(callback_output_buffer_dac_time_delta_us(
            time_info.output_buffer_dac_time,
            time_info.current_time,
        ))
    };
    // 暂停时不继续填充输入环缓；这样恢复前不会积压旧麦克风帧，也不会把暂停
    // 期间的采样误判为新的说话事件。
    if !data.shared.callback_paused.load(Ordering::Acquire) {
        if let Some(input_producer) = data.input_producer.as_mut() {
            let input_channels =
                usize::from(data.shared.input_channels.load(Ordering::Relaxed).max(1));
            let input_samples = (frame_count as usize).saturating_mul(input_channels);
            if input.is_null() {
                data.shared
                    .input_overflow_count
                    .fetch_add(1, Ordering::Relaxed);
            } else {
                // SAFETY: PortAudio provides `frame_count * input_channels` contiguous f32
                // samples for a full-duplex callback. The input pointer is valid for this call.
                let input_slice = std::slice::from_raw_parts(input as *const f32, input_samples);
                let written = input_producer.push_slice(input_slice);
                data.shared
                    .input_frames_captured
                    .fetch_add((written / input_channels) as u64, Ordering::Relaxed);
                if written < input_samples {
                    data.shared
                        .input_overflow_count
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
            if !time_info.is_null() {
                // SAFETY: The time-info pointer was checked above and is valid for this callback.
                let time_info = &*time_info;
                if let Some(adc_time_us) = input_adc_time_to_micros(time_info.input_buffer_adc_time)
                {
                    data.shared
                        .last_input_adc_time_us
                        .store(adc_time_us, Ordering::Relaxed);
                }
            }
        }
    }
    if data.shared.callback_paused.load(Ordering::Acquire) {
        out.fill(0.0);
        record_callback_timing_and_frames(&data.shared, dac_time_delta_us, 0, channels);
        return ffi::PA_CONTINUE;
    }
    let filled = pop_audio_samples(&mut data.consumer, out);
    record_callback_timing_and_frames(&data.shared, dac_time_delta_us, filled, channels);
    let speaking = data.shared.microphone_gate.is_speaking();
    // fail-open 的门控撤销必须绕过正常 release 包络；否则输出线程故障后
    // 主媒体仍可能在最多 250ms 内保持静音。
    reset_microphone_gain_if_disabled(&mut data.microphone_main_gain, &data.shared.microphone_gate);
    apply_microphone_main_gain(
        &mut data.microphone_main_gain,
        speaking,
        out,
        data.sample_rate_hz,
        channels,
    );
    if let Some(render_reference_producer) = data.render_reference_producer.as_mut() {
        // AEC 参考使用本 callback 实际送往 DAC 的主轨（不含麦克风回送），
        // 因而在静音包络完成后、clean mic 叠加前写入。
        let written = render_reference_producer.push_slice(out);
        if written < out.len() {
            data.shared
                .render_reference_drop_count
                .fetch_add(1, Ordering::Relaxed);
        }
    }
    if let Some(clean_mic_consumer) = data.clean_mic_consumer.as_mut() {
        mix_clean_mic_samples(clean_mic_consumer, out, speaking, &data.shared, channels);
    }
    if filled < samples {
        data.shared
            .callback_underrun_count
            .fetch_add(1, Ordering::Relaxed);
        data.shared.xrun_count.fetch_add(1, Ordering::Relaxed);
    }
    ffi::PA_CONTINUE
}

pub struct PortAudioOutput {
    shared: Arc<StreamShared>,
    producer: HeapProd<f32>,
    consumer: Option<HeapCons<f32>>,
    #[cfg(autolive_has_portaudio)]
    stream: *mut ffi::PaStream,
    #[cfg(autolive_has_portaudio)]
    user_data: Option<Box<CallbackUserData>>,
    #[cfg(autolive_has_portaudio)]
    close_quarantined: bool,
    #[cfg(autolive_has_portaudio)]
    initialized: bool,
    sample_rate_hz: u32,
    ring_capacity_kib: u32,
    frames_per_buffer: u32,
    /// None = 默认输出设备。
    device_index: Option<i32>,
    /// 启动回调前允许混音线程先把首段 PCM 写入环缓，避免回调补零。
    prestart_writes_enabled: bool,
    running: AtomicBool,
}

pub struct PortAudioDuplexInput {
    shared: Arc<StreamShared>,
    input_consumer: HeapCons<f32>,
    clean_mic_producer: HeapProd<f32>,
    render_reference_consumer: HeapCons<f32>,
    input_channels: u16,
    output_channels: u16,
    #[cfg(test)]
    input_test_producer: Option<HeapProd<f32>>,
}

impl std::fmt::Debug for PortAudioDuplexInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PortAudioDuplexInput")
            .field("input_channels", &self.input_channels)
            .field("output_channels", &self.output_channels)
            .field("input_ring_len_samples", &self.input_ring_len_samples())
            .field(
                "clean_mic_ring_len_samples",
                &self.clean_mic_ring_len_samples(),
            )
            .finish()
    }
}

impl PortAudioDuplexInput {
    pub fn input_channels(&self) -> u16 {
        self.input_channels
    }

    pub fn output_channels(&self) -> u16 {
        self.output_channels
    }

    /// DSP worker 读取 callback 捕获的原始输入帧；永不等待，空环缓返回 0。
    pub fn read_input_interleaved(&mut self, output: &mut [f32]) -> usize {
        self.input_consumer.pop_slice(output)
    }

    /// DSP worker 读取与输入时间顺序对应的主轨参考；空环缓返回 0。
    pub fn read_render_reference_interleaved(&mut self, output: &mut [f32]) -> usize {
        self.render_reference_consumer.pop_slice(output)
    }

    /// DSP worker 写入已经完成 AEC/降噪/VAD 的双声道 PCM。
    /// callback 会在说话门控打开时把它叠加到最终输出，并始终执行限幅。
    pub fn write_clean_mic_stereo_interleaved(&mut self, samples: &[f32]) -> Result<usize, String> {
        if !samples.len().is_multiple_of(2) {
            return Err("clean mic 输入未按完整的立体声帧对齐".to_owned());
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err("clean mic 输入包含 NaN 或 Infinity".to_owned());
        }
        let normalized = prepare_mixer_samples(samples, usize::from(self.output_channels))?;
        let written = self
            .clean_mic_producer
            .push_iter(normalized.iter().copied());
        if written < normalized.len() {
            self.shared
                .clean_mic_drop_count
                .fetch_add(1, Ordering::Relaxed);
        }
        let stereo_written =
            stereo_samples_for_output_samples(written, usize::from(self.output_channels))
                .ok_or_else(|| "clean mic 写入结果未按完整硬件帧对齐".to_owned())?;
        Ok(stereo_written)
    }

    pub fn input_ring_len_samples(&self) -> usize {
        self.input_consumer.occupied_len()
    }

    pub fn input_capacity_samples(&self) -> usize {
        self.input_consumer.capacity().get()
    }

    pub fn clean_mic_ring_len_samples(&self) -> usize {
        self.clean_mic_producer.occupied_len()
    }

    pub fn clean_mic_capacity_samples(&self) -> usize {
        self.clean_mic_producer.capacity().get()
    }

    pub fn input_health(&self) -> PortAudioInputHealth {
        let last_input_adc_time_us =
            match self.shared.last_input_adc_time_us.load(Ordering::Relaxed) {
                i64::MIN => None,
                value => Some(value),
            };
        PortAudioInputHealth {
            input_channels: self.input_channels,
            callback_count: self.shared.callback_count.load(Ordering::Relaxed),
            callback_last_status_flags: self
                .shared
                .callback_last_status_flags
                .load(Ordering::Relaxed),
            input_frames_captured: self.shared.input_frames_captured.load(Ordering::Relaxed),
            input_overflow_count: self.shared.input_overflow_count.load(Ordering::Relaxed),
            clean_mic_frames_rendered: self
                .shared
                .clean_mic_frames_rendered
                .load(Ordering::Relaxed),
            clean_mic_drop_count: self.shared.clean_mic_drop_count.load(Ordering::Relaxed),
            clean_mic_underrun_count: self.shared.clean_mic_underrun_count.load(Ordering::Relaxed),
            render_reference_drop_count: self
                .shared
                .render_reference_drop_count
                .load(Ordering::Relaxed),
            input_ring_len_samples: self.input_ring_len_samples(),
            input_ring_capacity_samples: self.input_capacity_samples(),
            clean_mic_ring_len_samples: self.clean_mic_ring_len_samples(),
            clean_mic_ring_capacity_samples: self.clean_mic_capacity_samples(),
            last_input_adc_time_us,
        }
    }

    pub fn microphone_gate(&self) -> Arc<MicrophoneGate> {
        Arc::clone(&self.shared.microphone_gate)
    }

    pub fn set_gate_enabled(&self, enabled: bool) {
        self.shared.microphone_gate.set_enabled(enabled);
    }

    pub fn set_gate_speaking(&self, speaking: bool) {
        self.shared.microphone_gate.set_speaking(speaking);
    }

    pub fn gate_state(&self) -> MicrophoneGateState {
        self.shared.microphone_gate.state()
    }

    #[cfg(test)]
    fn push_input_for_test(&mut self, samples: &[f32]) -> usize {
        self.input_test_producer
            .as_mut()
            .map(|producer| producer.push_slice(samples))
            .unwrap_or(0)
    }
}

impl std::fmt::Debug for PortAudioOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PortAudioOutput")
            .field("sample_rate_hz", &self.sample_rate_hz)
            .field("ring_capacity_kib", &self.ring_capacity_kib)
            .field("frames_per_buffer", &self.frames_per_buffer)
            .field("device_index", &self.device_index)
            .field("running", &self.running.load(Ordering::Relaxed))
            .field("xrun_count", &self.xrun_count())
            .finish()
    }
}

impl PortAudioOutput {
    pub fn new(sample_rate_hz: u32, ring_capacity_kib: u32, channels: u16) -> Self {
        let sample_rate_hz = if sample_rate_hz == 0 {
            DEFAULT_SAMPLE_RATE_HZ
        } else {
            sample_rate_hz
        };
        let capacity = ring_capacity_samples(ring_capacity_kib, channels);
        let (producer, consumer) = HeapRb::<f32>::new(capacity.max(1)).split();
        Self {
            shared: Arc::new(StreamShared {
                xrun_count: AtomicU64::new(0),
                callback_underrun_count: AtomicU64::new(0),
                producer_drop_count: AtomicU64::new(0),
                callback_count: AtomicU64::new(0),
                callback_last_status_flags: AtomicU64::new(0),
                callback_status_flags_count: AtomicU64::new(0),
                callback_output_buffer_dac_time_delta_us: AtomicI64::new(0),
                callback_pcm_frames_total: AtomicU64::new(0),
                channels: AtomicU16::new(channels.max(1)),
                clear_requested: AtomicBool::new(false),
                callback_paused: AtomicBool::new(false),
                live_pcm_paused: AtomicBool::new(false),
                microphone_gate: Arc::new(MicrophoneGate::default()),
                input_channels: AtomicU16::new(DEFAULT_INPUT_CHANNELS),
                input_frames_captured: AtomicU64::new(0),
                input_overflow_count: AtomicU64::new(0),
                last_input_adc_time_us: AtomicI64::new(i64::MIN),
                clean_mic_frames_rendered: AtomicU64::new(0),
                clean_mic_drop_count: AtomicU64::new(0),
                clean_mic_underrun_count: AtomicU64::new(0),
                render_reference_drop_count: AtomicU64::new(0),
            }),
            producer,
            consumer: Some(consumer),
            #[cfg(autolive_has_portaudio)]
            stream: std::ptr::null_mut(),
            #[cfg(autolive_has_portaudio)]
            user_data: None,
            #[cfg(autolive_has_portaudio)]
            close_quarantined: false,
            #[cfg(autolive_has_portaudio)]
            initialized: false,
            sample_rate_hz,
            ring_capacity_kib,
            frames_per_buffer: DEFAULT_FRAMES_PER_BUFFER,
            device_index: None,
            prestart_writes_enabled: false,
            running: AtomicBool::new(false),
        }
    }

    pub fn set_device_index(&mut self, device_index: Option<i32>) {
        self.device_index = device_index;
    }

    pub fn set_frames_per_buffer(&mut self, frames: u32) -> Result<(), String> {
        validate_frames_per_buffer(frames)?;
        self.frames_per_buffer = frames;
        Ok(())
    }

    pub fn frames_per_buffer(&self) -> u32 {
        self.frames_per_buffer
    }

    pub fn ring_capacity_kib(&self) -> u32 {
        self.ring_capacity_kib
    }

    pub fn ring_capacity_samples(&self) -> usize {
        self.producer.capacity().get()
    }

    pub fn target_playback_watermark_samples(&self) -> usize {
        playback_watermark_samples(self.sample_rate_hz, self.channels(), self.frames_per_buffer)
            .min(self.ring_capacity_samples())
    }

    pub fn device_index(&self) -> Option<i32> {
        self.device_index
    }

    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    pub fn set_sample_rate_hz(&mut self, sample_rate_hz: u32) {
        self.sample_rate_hz = if sample_rate_hz == 0 {
            DEFAULT_SAMPLE_RATE_HZ
        } else {
            sample_rate_hz
        };
    }

    pub fn channels(&self) -> u16 {
        self.shared.channels.load(Ordering::Relaxed).max(1)
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// 返回与硬件 callback 共享的无锁麦克风门控。output-only 流也提供该对象，
    /// 但只有 full-duplex callback 会消费 clean mic ring。
    pub fn microphone_gate(&self) -> Arc<MicrophoneGate> {
        Arc::clone(&self.shared.microphone_gate)
    }

    /// 返回应用运行标志、真实 PortAudio 流状态、callback 观测值和环缓水位。
    /// 查询本身不获取 callback 使用的任何锁。
    pub fn stream_health(&self) -> PortAudioStreamHealth {
        #[cfg(autolive_has_portaudio)]
        let hardware_state = if self.close_quarantined {
            PortAudioHardwareState::Unknown
        } else {
            query_portaudio_stream_state(self.stream)
        };
        #[cfg(not(autolive_has_portaudio))]
        let hardware_state = PortAudioHardwareState::Unsupported;
        #[cfg(autolive_has_portaudio)]
        let (output_latency_us, actual_sample_rate_hz) = if self.close_quarantined {
            (None, None)
        } else {
            query_portaudio_stream_info(self.stream)
        };
        #[cfg(not(autolive_has_portaudio))]
        let (output_latency_us, actual_sample_rate_hz) = (None, None);

        PortAudioStreamHealth {
            application_running: self.running.load(Ordering::SeqCst),
            hardware_state,
            output_latency_us,
            actual_sample_rate_hz,
            callback_count: self.shared.callback_count.load(Ordering::Relaxed),
            callback_last_status_flags: self
                .shared
                .callback_last_status_flags
                .load(Ordering::Relaxed),
            callback_status_flags_count: self
                .shared
                .callback_status_flags_count
                .load(Ordering::Relaxed),
            callback_output_buffer_dac_time_delta_us: self
                .shared
                .callback_output_buffer_dac_time_delta_us
                .load(Ordering::Relaxed),
            callback_pcm_frames_total: self
                .shared
                .callback_pcm_frames_total
                .load(Ordering::Relaxed),
            xrun_count: self.shared.xrun_count.load(Ordering::Relaxed),
            callback_underrun_count: self.shared.callback_underrun_count.load(Ordering::Relaxed),
            producer_drop_count: self.shared.producer_drop_count.load(Ordering::Relaxed),
            ring_len_samples: self.ring_len_samples(),
            ring_capacity_samples: self.ring_capacity_samples(),
        }
    }

    /// 返回最近可观察到的消费者进度；不获取 callback 使用的任何锁。
    pub fn progress_snapshot(&self) -> PortAudioOutputProgress {
        PortAudioOutputProgress {
            callback_count: self.shared.callback_count.load(Ordering::Relaxed),
            ring_len_samples: self.ring_len_samples(),
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        #[cfg(not(autolive_has_portaudio))]
        {
            Err("PortAudio 未启用：请使用 WebView 输出".to_owned())
        }
        #[cfg(autolive_has_portaudio)]
        {
            if self.running.load(Ordering::SeqCst) {
                return Ok(());
            }
            if !self.stream.is_null() || self.user_data.is_some() || self.close_quarantined {
                return Err("此前 PortAudio 流关闭失败，不能重复创建硬件流".to_owned());
            }
            ensure_portaudio_dll_search_path();
            let _guard = portaudio_lock().lock().unwrap_or_else(|e| e.into_inner());
            pa_acquire()?;
            self.initialized = true;
            let device = self
                .device_index
                .unwrap_or_else(|| unsafe { ffi::Pa_GetDefaultOutputDevice() });
            if device == ffi::PA_NO_DEVICE {
                self.shutdown_pa_unlocked();
                return Err("无默认输出设备".to_owned());
            }
            let info_ptr = unsafe { ffi::Pa_GetDeviceInfo(device) };
            if info_ptr.is_null() {
                self.shutdown_pa_unlocked();
                return Err("无法读取默认输出设备".to_owned());
            }
            let info = unsafe { &*info_ptr };
            let channels = i32::from(self.shared.channels.load(Ordering::Relaxed))
                .min(info.max_output_channels);
            if channels <= 0 {
                self.shutdown_pa_unlocked();
                return Err("默认输出设备无输出声道".to_owned());
            }
            // 回调必须按实际开流声道填环缓，否则 1ch 设备会乱音。
            self.shared
                .channels
                .store(channels as u16, Ordering::Relaxed);
            let output = ffi::PaStreamParameters {
                device,
                channel_count: channels,
                sample_format: ffi::PA_FLOAT32,
                suggested_latency: info.default_low_output_latency,
                host_api_specific_stream_info: std::ptr::null_mut(),
            };
            let Some(consumer) = self.consumer.take() else {
                self.shutdown_pa_unlocked();
                return Err("PortAudio 消费端未就绪".to_owned());
            };
            let user_data = Box::new(CallbackUserData {
                shared: Arc::clone(&self.shared),
                consumer,
                input_producer: None,
                clean_mic_consumer: None,
                render_reference_producer: None,
                microphone_main_gain: 1.0,
                sample_rate_hz: self.sample_rate_hz,
            });
            let user_ptr = Box::into_raw(user_data);
            let mut stream: *mut ffi::PaStream = std::ptr::null_mut();
            let open_err = unsafe {
                ffi::Pa_OpenStream(
                    &mut stream,
                    std::ptr::null(),
                    &output,
                    f64::from(self.sample_rate_hz),
                    std::os::raw::c_ulong::from(self.frames_per_buffer),
                    ffi::PA_CLIP_OFF,
                    Some(output_callback),
                    user_ptr as *mut _,
                )
            };
            if open_err != ffi::PA_NO_ERROR {
                // SAFETY: Open 失败时回收 user_data。
                let callback_data = unsafe { Box::from_raw(user_ptr) };
                self.consumer = Some(callback_data.consumer);
                self.shutdown_pa_unlocked();
                return Err(format!("Pa_OpenStream 失败：{}", pa_error_text(open_err)));
            }
            let start_err = unsafe { ffi::Pa_StartStream(stream) };
            if start_err != ffi::PA_NO_ERROR {
                let close_err = unsafe { ffi::Pa_CloseStream(stream) };
                // SAFETY: user_ptr 仍只由本函数持有；关闭失败时转交给 self，
                // 保证可能仍被 native stream 引用的 callback 数据继续存活。
                let callback_data = unsafe { Box::from_raw(user_ptr) };
                if close_err == ffi::PA_NO_ERROR {
                    self.consumer = Some(callback_data.consumer);
                    self.shutdown_pa_unlocked();
                    return Err(format!("Pa_StartStream 失败：{}", pa_error_text(start_err)));
                }
                self.stream = stream;
                self.user_data = Some(callback_data);
                self.close_quarantined = true;
                return Err(format!(
                    "Pa_StartStream 失败：{}；Pa_CloseStream 同时失败：{}",
                    pa_error_text(start_err),
                    pa_error_text(close_err)
                ));
            }
            self.stream = stream;
            // SAFETY: 所有权转回 Box，随 self 生命周期。
            self.user_data = Some(unsafe { Box::from_raw(user_ptr) });
            self.running.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    /// 在同一 PortAudio stream 中打开输入和输出。普通 `start()` 仍保持 output-only
    /// 行为；只有麦克风会话显式调用本方法时才创建输入环缓和 full-duplex callback。
    pub fn start_duplex(
        &mut self,
        config: PortAudioDuplexConfig,
    ) -> Result<PortAudioDuplexInput, String> {
        if config.input_channels == 0 {
            return Err("PortAudio 输入声道数必须大于 0".to_owned());
        }
        #[cfg(not(autolive_has_portaudio))]
        {
            let _ = config;
            Err("PortAudio 未启用：无法打开全双工输入".to_owned())
        }
        #[cfg(autolive_has_portaudio)]
        {
            if self.running.load(Ordering::SeqCst) {
                return Err("PortAudio 输出已经启动，不能重复打开全双工流".to_owned());
            }
            if !self.stream.is_null() || self.user_data.is_some() || self.close_quarantined {
                return Err("此前 PortAudio 流关闭失败，不能重复创建硬件流".to_owned());
            }
            ensure_portaudio_dll_search_path();
            let _guard = portaudio_lock().lock().unwrap_or_else(|e| e.into_inner());
            pa_acquire()?;
            self.initialized = true;

            let output_device = self
                .device_index
                .unwrap_or_else(|| unsafe { ffi::Pa_GetDefaultOutputDevice() });
            let input_device = config
                .input_device_index
                .unwrap_or_else(|| unsafe { ffi::Pa_GetDefaultInputDevice() });
            if output_device == ffi::PA_NO_DEVICE {
                self.shutdown_pa_unlocked();
                return Err("无默认输出设备".to_owned());
            }
            if input_device == ffi::PA_NO_DEVICE {
                self.shutdown_pa_unlocked();
                return Err("无默认输入设备".to_owned());
            }
            let output_info_ptr = unsafe { ffi::Pa_GetDeviceInfo(output_device) };
            let input_info_ptr = unsafe { ffi::Pa_GetDeviceInfo(input_device) };
            if output_info_ptr.is_null() {
                self.shutdown_pa_unlocked();
                return Err("无法读取输出设备".to_owned());
            }
            if input_info_ptr.is_null() {
                self.shutdown_pa_unlocked();
                return Err("无法读取输入设备".to_owned());
            }
            let output_info = unsafe { &*output_info_ptr };
            let input_info = unsafe { &*input_info_ptr };
            let output_channels = i32::from(self.shared.channels.load(Ordering::Relaxed))
                .min(output_info.max_output_channels);
            if output_channels <= 0 {
                self.shutdown_pa_unlocked();
                return Err("输出设备无可用输出声道".to_owned());
            }
            if i32::from(config.input_channels) > input_info.max_input_channels {
                self.shutdown_pa_unlocked();
                return Err(format!(
                    "输入设备最多支持 {} 个声道，不能打开 {} 个声道",
                    input_info.max_input_channels, config.input_channels
                ));
            }
            self.shared
                .channels
                .store(output_channels as u16, Ordering::Relaxed);
            self.shared
                .input_channels
                .store(config.input_channels, Ordering::Relaxed);
            let input_params = ffi::PaStreamParameters {
                device: input_device,
                channel_count: i32::from(config.input_channels),
                sample_format: ffi::PA_FLOAT32,
                suggested_latency: input_info.default_low_input_latency,
                host_api_specific_stream_info: std::ptr::null_mut(),
            };
            let output_params = ffi::PaStreamParameters {
                device: output_device,
                channel_count: output_channels,
                sample_format: ffi::PA_FLOAT32,
                suggested_latency: output_info.default_low_output_latency,
                host_api_specific_stream_info: std::ptr::null_mut(),
            };
            let format_err = unsafe {
                ffi::Pa_IsFormatSupported(
                    &input_params,
                    &output_params,
                    f64::from(self.sample_rate_hz),
                )
            };
            if format_err != ffi::PA_NO_ERROR {
                self.shutdown_pa_unlocked();
                return Err(format!(
                    "PortAudio 全双工格式不支持：{}",
                    pa_error_text(format_err)
                ));
            }

            self.shared.reset_microphone_metrics();

            let (input_producer, input_consumer) = HeapRb::<f32>::new(
                ring_capacity_samples(self.ring_capacity_kib, config.input_channels).max(1),
            )
            .split();
            let (clean_mic_producer, clean_mic_consumer) = HeapRb::<f32>::new(
                ring_capacity_samples(self.ring_capacity_kib, output_channels as u16).max(1),
            )
            .split();
            let (render_reference_producer, render_reference_consumer) = HeapRb::<f32>::new(
                ring_capacity_samples(self.ring_capacity_kib, output_channels as u16).max(1),
            )
            .split();
            let Some(consumer) = self.consumer.take() else {
                self.shutdown_pa_unlocked();
                return Err("PortAudio 消费端未就绪".to_owned());
            };
            let user_data = Box::new(CallbackUserData {
                shared: Arc::clone(&self.shared),
                consumer,
                input_producer: Some(input_producer),
                clean_mic_consumer: Some(clean_mic_consumer),
                render_reference_producer: Some(render_reference_producer),
                microphone_main_gain: 1.0,
                sample_rate_hz: self.sample_rate_hz,
            });
            let user_ptr = Box::into_raw(user_data);
            let mut stream: *mut ffi::PaStream = std::ptr::null_mut();
            let open_err = unsafe {
                ffi::Pa_OpenStream(
                    &mut stream,
                    &input_params,
                    &output_params,
                    f64::from(self.sample_rate_hz),
                    std::os::raw::c_ulong::from(self.frames_per_buffer),
                    ffi::PA_CLIP_OFF,
                    Some(output_callback),
                    user_ptr as *mut _,
                )
            };
            if open_err != ffi::PA_NO_ERROR {
                // SAFETY: Open 失败时仅本函数持有 callback box。
                let callback_data = unsafe { Box::from_raw(user_ptr) };
                self.consumer = Some(callback_data.consumer);
                self.shutdown_pa_unlocked();
                return Err(format!(
                    "Pa_OpenStream 全双工失败：{}",
                    pa_error_text(open_err)
                ));
            }
            let start_err = unsafe { ffi::Pa_StartStream(stream) };
            if start_err != ffi::PA_NO_ERROR {
                let close_err = unsafe { ffi::Pa_CloseStream(stream) };
                // SAFETY: user_ptr 仍只由本函数持有；关闭失败时必须隔离 callback box。
                let callback_data = unsafe { Box::from_raw(user_ptr) };
                if close_err == ffi::PA_NO_ERROR {
                    self.consumer = Some(callback_data.consumer);
                    self.shutdown_pa_unlocked();
                    return Err(format!(
                        "Pa_StartStream 全双工失败：{}",
                        pa_error_text(start_err)
                    ));
                }
                self.stream = stream;
                self.user_data = Some(callback_data);
                self.close_quarantined = true;
                return Err(format!(
                    "Pa_StartStream 全双工失败：{}；Pa_CloseStream 同时失败：{}",
                    pa_error_text(start_err),
                    pa_error_text(close_err)
                ));
            }
            self.stream = stream;
            // SAFETY: 所有权转回 Box，随 self 生命周期。
            self.user_data = Some(unsafe { Box::from_raw(user_ptr) });
            self.running.store(true, Ordering::SeqCst);
            Ok(PortAudioDuplexInput {
                shared: Arc::clone(&self.shared),
                input_consumer,
                clean_mic_producer,
                render_reference_consumer,
                input_channels: config.input_channels,
                output_channels: output_channels as u16,
                #[cfg(test)]
                input_test_producer: None,
            })
        }
    }

    #[cfg(test)]
    fn start_duplex_for_test(
        &mut self,
        input_channels: u16,
        output_channels: u16,
    ) -> Result<PortAudioDuplexInput, String> {
        if input_channels == 0 || output_channels == 0 {
            return Err("测试全双工环缓声道数必须大于 0".to_owned());
        }
        self.shared.reset_microphone_metrics();
        let (input_test_producer, input_consumer) = HeapRb::<f32>::new(256).split();
        let (clean_mic_producer, _clean_callback_consumer) = HeapRb::<f32>::new(256).split();
        let (_render_reference_producer, render_reference_consumer) =
            HeapRb::<f32>::new(256).split();
        self.shared
            .input_channels
            .store(input_channels, Ordering::Relaxed);
        self.shared
            .channels
            .store(output_channels, Ordering::Relaxed);
        Ok(PortAudioDuplexInput {
            shared: Arc::clone(&self.shared),
            input_consumer,
            clean_mic_producer,
            render_reference_consumer,
            input_channels,
            output_channels,
            input_test_producer: Some(input_test_producer),
        })
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self.prestart_writes_enabled = false;
        self.shared.microphone_gate.set_enabled(false);
        self.shared.callback_paused.store(false, Ordering::Release);
        self.shared.live_pcm_paused.store(false, Ordering::SeqCst);
        #[cfg(autolive_has_portaudio)]
        {
            let _guard = portaudio_lock().lock().unwrap_or_else(|e| e.into_inner());
            let close_succeeded = if self.stream.is_null() {
                true
            } else if should_attempt_stream_close(!self.stream.is_null(), self.close_quarantined) {
                let stop_err = unsafe { ffi::Pa_StopStream(self.stream) };
                if stop_err != ffi::PA_NO_ERROR {
                    eprintln!(
                        "autolive PortAudio: Pa_StopStream 失败，继续尝试关闭：{}",
                        pa_error_text(stop_err)
                    );
                }
                let close_err = unsafe { ffi::Pa_CloseStream(self.stream) };
                if close_err == ffi::PA_NO_ERROR {
                    self.stream = std::ptr::null_mut();
                    self.close_quarantined = false;
                    true
                } else {
                    self.close_quarantined = true;
                    eprintln!(
                        "autolive PortAudio: Pa_CloseStream 失败，隔离流与 callback 所有权：{}",
                        pa_error_text(close_err)
                    );
                    false
                }
            } else {
                false
            };
            if let Some(callback_data) =
                take_callback_after_confirmed_close(&mut self.user_data, close_succeeded)
            {
                self.consumer = Some(callback_data.consumer);
            }
            if close_succeeded {
                self.shutdown_pa_unlocked();
            }
        }
        self.shared.clear_requested.store(false, Ordering::Release);
        self.clear_ring();
    }

    #[cfg(autolive_has_portaudio)]
    fn shutdown_pa_unlocked(&mut self) {
        if self.initialized {
            pa_release();
            self.initialized = false;
        }
    }

    pub fn write_interleaved(&mut self, samples: &[f32]) -> Result<(), String> {
        if !self.running.load(Ordering::SeqCst) && !self.prestart_writes_enabled {
            return Err("PortAudio 输出未启动".to_owned());
        }
        if self.shared.live_pcm_paused.load(Ordering::SeqCst) {
            return Ok(());
        }
        validate_interleaved_samples(samples, usize::from(self.channels()))?;
        let written = self.push_interleaved_available(samples);
        if written < samples.len() {
            self.shared
                .producer_drop_count
                .fetch_add(1, Ordering::Relaxed);
            self.shared.xrun_count.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    /// 后台混音器固定提供立体声；单声道硬件在写入环缓前做等比下混。
    pub fn write_stereo_interleaved(&mut self, samples: &[f32]) -> Result<(), String> {
        if !self.running.load(Ordering::SeqCst) && !self.prestart_writes_enabled {
            return Err("PortAudio 输出未启动".to_owned());
        }
        if self.shared.live_pcm_paused.load(Ordering::SeqCst) {
            return Ok(());
        }
        let normalized = prepare_mixer_samples(samples, usize::from(self.channels()))?;
        let written = self.push_interleaved_available(normalized.as_ref());
        if written < normalized.len() {
            self.shared
                .producer_drop_count
                .fetch_add(1, Ordering::Relaxed);
            self.shared.xrun_count.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    /// 尝试写入目标播放水位以内的样本，返回对应的输入立体声采样数。
    /// 混音线程可在目标水位暂满时重试；callback 不需要获取任何锁。
    pub fn write_stereo_interleaved_available(&mut self, samples: &[f32]) -> Result<usize, String> {
        if !self.running.load(Ordering::SeqCst) && !self.prestart_writes_enabled {
            return Err("PortAudio 输出未启动".to_owned());
        }
        if self.shared.live_pcm_paused.load(Ordering::SeqCst) {
            return Ok(samples.len());
        }
        let output_channels = usize::from(self.channels());
        let normalized = prepare_mixer_samples(samples, output_channels)?;
        let written = self.push_interleaved_available(normalized.as_ref());
        stereo_samples_for_output_samples(written, output_channels)
            .ok_or_else(|| "PortAudio 写入结果未按完整硬件帧对齐".to_owned())
    }

    /// 候选提交前原子写入环缓；只允许启动事务使用，正常写入仍要求流已运行。
    /// 空间不足时返回 0，不做部分写入，避免候选提交失败后留下半条候选音轨。
    pub fn prime_stereo_interleaved_available(&mut self, samples: &[f32]) -> Result<usize, String> {
        if !self.running.load(Ordering::SeqCst) && !self.prestart_writes_enabled {
            return Err("PortAudio 输出未启动".to_owned());
        }
        if self.shared.live_pcm_paused.load(Ordering::SeqCst) {
            return Ok(samples.len());
        }
        let output_channels = usize::from(self.channels());
        let normalized = prepare_mixer_samples(samples, output_channels)?;
        if self.producer.vacant_len() < normalized.len() {
            return Ok(0);
        }
        // `push_slice` 可能受环缓分段影响而部分写入；`push_iter` 会跨越两段
        // vacant slice，并在一次 advance_write_index 中提交，满足候选原子提交。
        let written = self.push_interleaved_available_to_capacity(normalized.as_ref());
        stereo_samples_for_output_samples(written, output_channels)
            .ok_or_else(|| "PortAudio 预填结果未按完整硬件帧对齐".to_owned())
    }

    /// 当前目标播放水位还能接收的内部双声道交错采样数。
    pub fn writable_stereo_samples_within_watermark(&self) -> usize {
        let output_channels = usize::from(self.channels());
        let output_samples =
            self.writable_output_samples_up_to(self.target_playback_watermark_samples());
        let frames = output_samples / output_channels;
        frames.saturating_mul(2)
    }

    pub fn set_prestart_writes_enabled(&mut self, enabled: bool) {
        self.prestart_writes_enabled = enabled;
    }

    fn push_interleaved_available(&mut self, samples: &[f32]) -> usize {
        self.push_interleaved_up_to(samples, self.target_playback_watermark_samples())
    }

    fn push_interleaved_available_to_capacity(&mut self, samples: &[f32]) -> usize {
        self.push_interleaved_up_to(samples, self.ring_capacity_samples())
    }

    fn push_interleaved_up_to(&mut self, samples: &[f32], max_occupied_samples: usize) -> usize {
        // `push_slice` 只覆盖当前连续 vacant slice；写指针跨过环尾时，即使总空闲
        // 空间足够也可能返回 0，导致混音线程把“有回调进度”误判为背压失败。
        // `push_iter` 会跨越环缓两段，仍保持单生产者无锁写入。
        let remaining = self.writable_output_samples_up_to(max_occupied_samples);
        if remaining == 0 {
            return 0;
        }
        self.producer
            .push_iter(samples.iter().copied().take(remaining))
    }

    fn writable_output_samples_up_to(&self, max_occupied_samples: usize) -> usize {
        let channels = usize::from(self.channels());
        let remaining = max_occupied_samples
            .saturating_sub(self.producer.occupied_len())
            .min(self.producer.vacant_len());
        remaining.saturating_sub(remaining % channels)
    }

    pub fn clear_ring(&mut self) {
        if let Some(consumer) = self.consumer.as_mut() {
            consumer.clear();
        } else {
            self.shared.clear_requested.store(true, Ordering::Release);
        }
    }

    /// 环缓中剩余采样数（交错后）；用于测试音 drain。
    pub fn ring_len_samples(&self) -> usize {
        self.producer.occupied_len()
    }

    pub fn set_live_pcm_paused(&self, paused: bool) {
        self.shared.live_pcm_paused.store(paused, Ordering::SeqCst);
    }

    /// 暂停硬件消费并保留环缓中的 PCM，供播放暂停/恢复使用。
    pub fn set_callback_paused(&self, paused: bool) {
        self.shared.callback_paused.store(paused, Ordering::Release);
    }

    pub fn is_callback_paused(&self) -> bool {
        self.shared.callback_paused.load(Ordering::Acquire)
    }

    pub fn is_live_pcm_paused(&self) -> bool {
        self.shared.live_pcm_paused.load(Ordering::SeqCst)
    }

    /// 测试音写入：忽略 live_pcm_paused，直接进环缓。
    pub fn write_interleaved_forced(&mut self, samples: &[f32]) -> Result<(), String> {
        if !self.running.load(Ordering::SeqCst) && !self.prestart_writes_enabled {
            return Err("PortAudio 输出未启动".to_owned());
        }
        validate_interleaved_samples(samples, usize::from(self.channels()))?;
        let written = self.push_interleaved_available(samples);
        if written < samples.len() {
            self.shared
                .producer_drop_count
                .fetch_add(1, Ordering::Relaxed);
            self.shared.xrun_count.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    pub fn status(&self) -> OutputBackendStatus {
        let xrun_count = self.shared.xrun_count.load(Ordering::Relaxed);
        if self.running.load(Ordering::SeqCst) {
            return OutputBackendStatus {
                available: true,
                selected_backend: "portaudio",
                reason: None,
                xrun_count,
            };
        }
        let mut status = probe_portaudio();
        status.xrun_count = xrun_count;
        status
    }

    pub fn xrun_count(&self) -> u64 {
        self.shared.xrun_count.load(Ordering::Relaxed)
    }
}

impl Drop for PortAudioOutput {
    fn drop(&mut self) {
        self.stop();
        #[cfg(autolive_has_portaudio)]
        if !self.stream.is_null() {
            // Native close 已失败，不能释放仍可能被 callback 引用的数据。这里选择
            // 在进程余生保留极小资源，避免驱动线程发生 UAF；PA 会话同样不 Terminate。
            if let Some(callback_data) = self.user_data.take() {
                let _ = Box::into_raw(callback_data);
            }
            eprintln!("autolive PortAudio: 流关闭失败，保留 native stream 直到进程退出");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spsc_ring_drops_newest_when_full_without_blocking() {
        let (mut producer, mut consumer) = HeapRb::<f32>::new(3).split();
        assert_eq!(producer.push_slice(&[1.0, 2.0, 3.0, 4.0]), 3);
        let mut out = [0.0; 3];
        assert_eq!(consumer.pop_slice(&mut out), 3);
        assert_eq!(out, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn spsc_ring_consumer_reports_underrun_for_callback_zero_fill() {
        let (mut producer, mut consumer) = HeapRb::<f32>::new(8).split();
        assert_eq!(producer.push_slice(&[0.5]), 1);
        let mut out = [1.0; 3];
        assert_eq!(pop_audio_samples(&mut consumer, &mut out), 1);
        assert_eq!(out, [0.5, 0.0, 0.0]);
    }

    #[test]
    fn mixer_downmixes_stereo_for_mono_device() {
        let normalized = prepare_mixer_samples(&[1.0, 3.0, 2.0, 4.0], 1).unwrap();
        assert_eq!(normalized.as_ref(), &[2.0, 3.0]);
        assert!(matches!(&normalized, Cow::Owned(_)));
    }

    #[test]
    fn mixer_borrows_already_stereo_pcm_without_copying() {
        let samples = [1.0, 2.0, 3.0, 4.0];
        let normalized = prepare_mixer_samples(&samples, 2).unwrap();

        assert!(matches!(&normalized, Cow::Borrowed(_)));
        assert_eq!(normalized.as_ptr(), samples.as_ptr());
    }

    #[test]
    fn mixer_rejects_incomplete_frame_and_non_finite_input() {
        assert!(prepare_mixer_samples(&[1.0], 1).is_err());
        assert!(prepare_mixer_samples(&[f32::NAN, 0.0], 2).is_err());
    }

    #[test]
    fn callback_ownership_is_released_only_after_confirmed_stream_close() {
        let mut callback = Some("callback");

        assert_eq!(
            take_callback_after_confirmed_close(&mut callback, false),
            None
        );
        assert_eq!(callback, Some("callback"));
        assert_eq!(
            take_callback_after_confirmed_close(&mut callback, false),
            None
        );
        assert_eq!(
            take_callback_after_confirmed_close(&mut callback, true),
            Some("callback")
        );
        assert_eq!(
            take_callback_after_confirmed_close(&mut callback, true),
            None
        );
    }

    #[test]
    fn quarantined_stream_close_is_not_retried_or_double_closed() {
        assert!(should_attempt_stream_close(true, false));
        assert!(!should_attempt_stream_close(true, true));
        assert!(!should_attempt_stream_close(false, false));
    }

    #[cfg(autolive_has_portaudio)]
    #[test]
    fn quarantined_stream_health_never_queries_the_native_handle() {
        let mut output = PortAudioOutput::new(DEFAULT_SAMPLE_RATE_HZ, 128, 2);
        output.close_quarantined = true;

        let health = output.stream_health();

        assert_eq!(health.hardware_state, PortAudioHardwareState::Unknown);
        assert_eq!(health.output_latency_us, None);
        assert_eq!(health.actual_sample_rate_hz, None);
        output.close_quarantined = false;
    }

    #[test]
    fn hardware_samples_convert_to_stereo_samples_by_complete_frames() {
        assert_eq!(stereo_samples_for_output_samples(256, 1), Some(512));
        assert_eq!(stereo_samples_for_output_samples(512, 2), Some(512));
        assert_eq!(stereo_samples_for_output_samples(3, 2), None);
        assert_eq!(stereo_samples_for_output_samples(1, 0), None);
        assert_eq!(stereo_samples_for_output_samples(usize::MAX, 1), None);
    }

    #[test]
    fn callback_pause_is_explicit_and_does_not_change_ring_contents() {
        let mut output = PortAudioOutput::new(44_100, 128, 2);
        output.set_prestart_writes_enabled(true);
        output.write_stereo_interleaved(&[0.25, -0.25]).unwrap();
        let before = output.ring_len_samples();
        output.set_callback_paused(true);
        assert!(output.is_callback_paused());
        assert_eq!(output.ring_len_samples(), before);
        output.set_callback_paused(false);
        assert!(!output.is_callback_paused());
    }

    #[test]
    fn capacity_uses_kib_bytes_instead_of_frames_or_milliseconds() {
        assert_eq!(ring_capacity_samples(1_024, 2), 262_144);
        assert!(validate_ring_capacity_kib(128).is_ok());
        assert!(validate_ring_capacity_kib(2_048).is_ok());
        assert!(validate_ring_capacity_kib(2_049).is_err());
    }

    #[test]
    fn zero_sample_rate_still_allocates_default_1024_kib_ring() {
        let output = PortAudioOutput::new(0, DEFAULT_RING_CAPACITY_KIB, 2);
        assert_eq!(output.sample_rate_hz(), DEFAULT_SAMPLE_RATE_HZ);
        assert_eq!(output.ring_capacity_kib(), 1_024);
        assert_eq!(output.producer.capacity().get(), 262_144);
    }

    #[test]
    fn streaming_watermark_covers_scheduler_jitter_without_changing_configured_capacity() {
        let mut output = PortAudioOutput::new(44_100, DEFAULT_RING_CAPACITY_KIB, 2);
        output.set_prestart_writes_enabled(true);
        let target = playback_watermark_samples(44_100, 2, DEFAULT_FRAMES_PER_BUFFER);
        assert_eq!(target, 17_640);
        assert_eq!(output.target_playback_watermark_samples(), target);
        assert!(target < output.ring_capacity_samples());

        let samples = vec![0.0; target + 1_024];
        assert_eq!(
            output.write_stereo_interleaved_available(&samples).unwrap(),
            target
        );
        assert_eq!(output.ring_len_samples(), target);
        assert_eq!(
            output
                .write_stereo_interleaved_available(&[0.0, 0.0])
                .unwrap(),
            0
        );
        assert_eq!(output.ring_capacity_kib(), DEFAULT_RING_CAPACITY_KIB);
        assert_eq!(output.ring_capacity_samples(), 262_144);
    }

    #[test]
    fn streaming_watermark_scales_with_hardware_callback_jitter() {
        assert_eq!(playback_watermark_samples(44_100, 2, 256), 17_640);
        assert_eq!(playback_watermark_samples(44_100, 2, 2_048), 32_768);
    }

    #[test]
    fn frames_per_buffer_is_an_independent_internal_hardware_setting() {
        let mut output = PortAudioOutput::new(DEFAULT_SAMPLE_RATE_HZ, DEFAULT_RING_CAPACITY_KIB, 2);
        assert_eq!(output.frames_per_buffer(), DEFAULT_FRAMES_PER_BUFFER);
        assert!(output.set_frames_per_buffer(MAX_FRAMES_PER_BUFFER).is_ok());
        assert_eq!(output.frames_per_buffer(), MAX_FRAMES_PER_BUFFER);
        assert!(output
            .set_frames_per_buffer(MAX_FRAMES_PER_BUFFER + 1)
            .is_err());
        assert!(output
            .set_frames_per_buffer(MIN_FRAMES_PER_BUFFER - 1)
            .is_err());
        assert_eq!(output.frames_per_buffer(), MAX_FRAMES_PER_BUFFER);
    }

    #[test]
    fn callback_observation_records_last_flags_without_a_lock() {
        let shared = StreamShared {
            xrun_count: AtomicU64::new(0),
            callback_underrun_count: AtomicU64::new(0),
            producer_drop_count: AtomicU64::new(0),
            callback_count: AtomicU64::new(0),
            callback_last_status_flags: AtomicU64::new(0),
            callback_status_flags_count: AtomicU64::new(0),
            callback_output_buffer_dac_time_delta_us: AtomicI64::new(0),
            callback_pcm_frames_total: AtomicU64::new(0),
            channels: AtomicU16::new(2),
            clear_requested: AtomicBool::new(false),
            callback_paused: AtomicBool::new(false),
            live_pcm_paused: AtomicBool::new(false),
            microphone_gate: Arc::new(MicrophoneGate::default()),
            input_channels: AtomicU16::new(DEFAULT_INPUT_CHANNELS),
            input_frames_captured: AtomicU64::new(0),
            input_overflow_count: AtomicU64::new(0),
            last_input_adc_time_us: AtomicI64::new(i64::MIN),
            clean_mic_frames_rendered: AtomicU64::new(0),
            clean_mic_drop_count: AtomicU64::new(0),
            clean_mic_underrun_count: AtomicU64::new(0),
            render_reference_drop_count: AtomicU64::new(0),
        };

        record_callback_status(&shared, 0);
        record_callback_status(&shared, 0x05);
        record_callback_status(&shared, 0x02);

        assert_eq!(shared.callback_count.load(Ordering::Relaxed), 3);
        assert_eq!(
            shared.callback_last_status_flags.load(Ordering::Relaxed),
            0x02
        );
        assert_eq!(
            shared.callback_status_flags_count.load(Ordering::Relaxed),
            2
        );
    }

    #[test]
    fn callback_timing_conversion_handles_seconds_and_invalid_values() {
        assert_eq!(latency_seconds_to_micros(0.001_234_5), Some(1_235));
        assert_eq!(latency_seconds_to_micros(-0.001), None);
        assert_eq!(latency_seconds_to_micros(f64::NAN), None);
        assert_eq!(actual_sample_rate_hz(44_100.4), Some(44_100));
        assert_eq!(actual_sample_rate_hz(0.0), None);
        assert_eq!(
            callback_output_buffer_dac_time_delta_us(2.001_234, 2.0),
            1_234
        );
        assert_eq!(callback_output_buffer_dac_time_delta_us(f64::NAN, 2.0), 0);
    }

    #[test]
    fn callback_observation_records_dac_delta_and_pcm_frames_without_a_lock() {
        let output = PortAudioOutput::new(DEFAULT_SAMPLE_RATE_HZ, 128, 2);
        record_callback_timing_and_frames(&output.shared, Some(1_250), 960, 2);
        record_callback_timing_and_frames(&output.shared, Some(-500), 480, 2);

        let health = output.stream_health();
        assert_eq!(health.callback_output_buffer_dac_time_delta_us, -500);
        assert_eq!(health.callback_pcm_frames_total, 720);
    }

    #[test]
    fn stream_health_exposes_application_state_and_ring_occupancy() {
        let output = PortAudioOutput::new(DEFAULT_SAMPLE_RATE_HZ, 128, 2);
        output.running.store(true, Ordering::SeqCst);
        let health = output.stream_health();

        assert!(health.application_running);
        assert_eq!(health.output_latency_us, None);
        assert_eq!(health.actual_sample_rate_hz, None);
        assert_eq!(health.callback_count, 0);
        assert_eq!(health.callback_last_status_flags, 0);
        assert_eq!(health.callback_status_flags_count, 0);
        assert_eq!(health.callback_output_buffer_dac_time_delta_us, 0);
        assert_eq!(health.callback_pcm_frames_total, 0);
        assert_eq!(health.xrun_count, 0);
        assert_eq!(health.callback_underrun_count, 0);
        assert_eq!(health.producer_drop_count, 0);
        assert_eq!(health.ring_len_samples, 0);
        assert_eq!(health.ring_capacity_samples, ring_capacity_samples(128, 2));
        #[cfg(autolive_has_portaudio)]
        assert_eq!(health.hardware_state, PortAudioHardwareState::NotCreated);
        #[cfg(not(autolive_has_portaudio))]
        assert_eq!(health.hardware_state, PortAudioHardwareState::Unsupported);
    }

    #[test]
    fn candidate_prime_never_leaves_a_partial_candidate_in_the_ring() {
        let mut output = PortAudioOutput::new(1_000, 1, 2);
        output.set_prestart_writes_enabled(true);
        let capacity = output.producer.capacity().get();
        let existing = vec![0.0; capacity - 56];
        output
            .prime_stereo_interleaved_available(&existing)
            .unwrap();

        // 当前仅剩 56 个位置，候选需要 100 个样本；失败不能写入其中一部分。
        let candidate = vec![1.0; 100];
        assert_eq!(
            output
                .prime_stereo_interleaved_available(&candidate)
                .unwrap(),
            0
        );
        assert_eq!(output.ring_len_samples(), capacity - 56);
    }

    #[test]
    fn available_write_reports_partial_progress_without_dropping_silently() {
        let mut output = PortAudioOutput::new(10_000, 1, 2);
        output.running.store(true, Ordering::SeqCst);
        let samples = (0..400).map(|value| value as f32).collect::<Vec<_>>();

        assert_eq!(
            output.write_stereo_interleaved_available(&samples).unwrap(),
            256
        );
        assert_eq!(output.ring_len_samples(), 256);
        assert_eq!(output.xrun_count(), 0);
        output.stop();
    }

    #[test]
    fn available_write_crosses_ring_wrap_when_consumer_has_freed_space() {
        let mut output = PortAudioOutput::new(10_000, 1, 2);
        output.running.store(true, Ordering::SeqCst);
        output
            .write_stereo_interleaved_available(&vec![0.0; 200])
            .unwrap();

        let mut drained = vec![0.0; 100];
        assert_eq!(
            output
                .consumer
                .as_mut()
                .expect("consumer is available before start")
                .pop_slice(&mut drained),
            100
        );

        // 写指针位于环尾，环首有 100 个样本空闲；push_slice 只能看到环尾的
        // 连续空间，而 push_iter 必须把这 156 个可用样本完整交给混音线程。
        assert_eq!(
            output
                .write_stereo_interleaved_available(&vec![1.0; 200])
                .unwrap(),
            156
        );
        assert_eq!(output.ring_len_samples(), 256);
        output.stop();
    }

    #[test]
    fn prestart_write_can_prime_ring_before_hardware_callback() {
        let mut output = PortAudioOutput::new(1_000, 10, 2);
        output.set_prestart_writes_enabled(true);
        assert_eq!(
            output
                .write_stereo_interleaved_available(&[1.0, 2.0, 3.0, 4.0])
                .unwrap(),
            4
        );
        assert_eq!(output.ring_len_samples(), 4);
        output.stop();
        assert!(output
            .write_stereo_interleaved_available(&[1.0, 2.0])
            .is_err());
    }

    #[test]
    fn stop_is_idempotent_before_start() {
        let mut output = PortAudioOutput::new(DEFAULT_SAMPLE_RATE_HZ, DEFAULT_RING_CAPACITY_KIB, 2);
        output.stop();
        output.stop();
        assert!(!output.is_running());
        assert_eq!(output.ring_len_samples(), 0);
    }

    #[test]
    fn probe_does_not_panic() {
        let status = probe_portaudio();
        assert!(status.selected_backend == "portaudio" || status.selected_backend == "webview");
    }

    #[test]
    fn microphone_gate_is_atomic_and_fail_open_when_disabled() {
        let gate = MicrophoneGate::default();

        assert_eq!(gate.state(), MicrophoneGateState::Disabled);
        assert_eq!(gate.main_gain_target(), 1.0);

        gate.set_enabled(true);
        assert_eq!(gate.state(), MicrophoneGateState::Armed);
        gate.set_speaking(true);
        assert_eq!(gate.state(), MicrophoneGateState::Speaking);
        assert_eq!(gate.main_gain_target(), 0.0);

        gate.set_enabled(false);
        assert_eq!(gate.state(), MicrophoneGateState::Disabled);
        assert_eq!(gate.main_gain_target(), 1.0);
    }

    #[test]
    fn disabling_microphone_resets_callback_gain_without_release_delay() {
        let gate = MicrophoneGate::default();
        gate.set_enabled(true);
        gate.set_speaking(true);
        let mut gain = 0.0;

        gate.set_enabled(false);
        reset_microphone_gain_if_disabled(&mut gain, &gate);

        assert_eq!(gain, 1.0);
    }

    #[test]
    fn input_adc_timestamp_rejects_invalid_values_and_preserves_microseconds() {
        assert_eq!(input_adc_time_to_micros(1.234_567), Some(1_234_567));
        assert_eq!(input_adc_time_to_micros(f64::NAN), None);
        assert_eq!(input_adc_time_to_micros(f64::INFINITY), None);
    }

    #[test]
    fn input_health_exposes_callback_heartbeat_and_status_flags() {
        let mut output = PortAudioOutput::new(48_000, 128, 2);
        let input = output
            .start_duplex_for_test(1, 2)
            .expect("test duplex rings should be available");
        input.shared.callback_count.store(12, Ordering::Relaxed);
        input
            .shared
            .callback_last_status_flags
            .store(0x05, Ordering::Relaxed);

        let health = input.input_health();
        assert_eq!(health.callback_count, 12);
        assert_eq!(health.callback_last_status_flags, 0x05);
        output.stop();
    }

    #[test]
    fn starting_a_new_duplex_session_resets_previous_microphone_metrics() {
        let mut output = PortAudioOutput::new(48_000, 128, 2);
        let input = output
            .start_duplex_for_test(1, 2)
            .expect("test duplex rings should be available");
        input
            .shared
            .input_overflow_count
            .store(7, Ordering::Relaxed);
        input
            .shared
            .render_reference_drop_count
            .store(3, Ordering::Relaxed);
        input
            .shared
            .clean_mic_drop_count
            .store(2, Ordering::Relaxed);
        drop(input);

        let next = output
            .start_duplex_for_test(1, 2)
            .expect("a new test duplex session should be available");
        let health = next.input_health();
        assert_eq!(health.input_overflow_count, 0);
        assert_eq!(health.render_reference_drop_count, 0);
        assert_eq!(health.clean_mic_drop_count, 0);
        assert_eq!(health.last_input_adc_time_us, None);
    }

    #[test]
    fn microphone_mute_envelope_reaches_exact_silence_and_recovers() {
        let mut gain = 1.0;
        let mut output = vec![1.0; 480];
        apply_microphone_main_gain(&mut gain, true, &mut output, 48_000, 2);
        assert!(gain < 1.0);
        assert!(gain > 0.0);

        for _ in 0..10 {
            apply_microphone_main_gain(&mut gain, true, &mut output, 48_000, 2);
        }
        assert_eq!(gain, 0.0);
        assert!(output.iter().all(|sample| *sample == 0.0));

        apply_microphone_main_gain(&mut gain, false, &mut output, 48_000, 2);
        assert!(gain > 0.0);
        assert!(gain < 1.0);
    }

    #[test]
    fn duplex_input_and_clean_mic_rings_are_bounded_and_round_trip() {
        let mut output = PortAudioOutput::new(48_000, 128, 2);
        let mut input = output
            .start_duplex_for_test(1, 2)
            .expect("test duplex rings should be available");

        let capacity = input.input_capacity_samples();
        assert!(capacity > 0);
        let oversized = vec![0.25; capacity + 2];
        let written = input.push_input_for_test(&oversized);
        assert_eq!(written, capacity);
        assert_eq!(input.input_ring_len_samples(), capacity);

        let mut captured = vec![0.0; capacity];
        assert_eq!(input.read_input_interleaved(&mut captured), capacity);
        assert!(captured.iter().all(|sample| *sample == 0.25));

        input.set_gate_enabled(true);
        input.set_gate_speaking(true);
        assert_eq!(
            input
                .write_clean_mic_stereo_interleaved(&[0.1, 0.2, 0.3, 0.4])
                .unwrap(),
            4
        );
        assert_eq!(input.clean_mic_ring_len_samples(), 4);
        output.stop();
    }

    #[test]
    fn clean_mic_is_mixed_only_while_speaking_and_main_track_is_gated() {
        let shared = Arc::new(StreamShared::for_test(2));
        let gate = Arc::clone(&shared.microphone_gate);
        let (mut producer, mut consumer) = HeapRb::<f32>::new(8).split();
        producer.push_slice(&[0.25, -0.25, 0.5, -0.5]);
        gate.set_enabled(true);
        gate.set_speaking(true);
        let mut output = vec![0.75, 0.75, 0.75, 0.75];
        let mut gain = 0.0;
        mix_clean_mic_into_output(&mut consumer, &mut output, &mut gain, &shared, 2, 48_000);
        assert_eq!(output, vec![0.25, -0.25, 0.5, -0.5]);

        gate.set_speaking(false);
        producer.push_slice(&[0.1, 0.1, 0.1, 0.1]);
        output.fill(0.75);
        gain = 1.0;
        mix_clean_mic_into_output(&mut consumer, &mut output, &mut gain, &shared, 2, 48_000);
        assert!(output.iter().all(|sample| *sample == 0.75));
    }

    #[cfg(autolive_has_portaudio)]
    #[test]
    fn negative_device_count_is_reported_as_portaudio_error() {
        let error = checked_device_count(-10_000).expect_err("negative count must fail");

        assert_eq!(error, "Pa_GetDeviceCount 失败：PortAudio not initialized");
        assert_eq!(checked_device_count(0).unwrap(), 0);
    }

    #[cfg(autolive_has_portaudio)]
    #[test]
    fn list_devices_when_vendor_present() {
        let devices = list_output_devices().expect("should list when vendor linked");
        // CI/无声卡机器可能空；有设备则通道 > 0
        for device in &devices {
            assert!(device.max_output_channels > 0);
            assert!(!device.id.is_empty());
        }
    }

    #[cfg(autolive_has_portaudio)]
    #[test]
    fn default_input_device_metadata_is_safe_when_available() {
        let device = default_input_device().expect("default input probe should not fail");
        if let Some((index, device)) = device {
            assert!(index >= 0);
            assert!(device.id.starts_with("pa-input-"));
            assert!(!device.name.contains('\n'));
            assert!(device.max_input_channels > 0);
        }
    }

    #[test]
    fn microphone_device_id_is_stable_for_same_identity() {
        let first = stable_input_device_id(HostApiKind::Mme, "Mic", 1, 44_100);
        let second = stable_input_device_id(HostApiKind::Mme, "Mic", 1, 44_100);
        let changed_name = stable_input_device_id(HostApiKind::Mme, "Other Mic", 1, 44_100);
        let duplicate =
            stable_input_device_id_with_occurrence(HostApiKind::Mme, "Mic", 1, 44_100, 1);
        assert_eq!(first, second);
        assert_eq!(
            first,
            stable_input_device_id_with_occurrence(HostApiKind::Mme, "Mic", 1, 44_100, 0)
        );
        assert_ne!(first, changed_name);
        assert_ne!(first, duplicate);
        assert!(first.starts_with("pa-input-mme-"));
    }

    #[cfg(autolive_has_portaudio)]
    #[test]
    fn enumerated_microphone_ids_resolve_without_exposing_indices() {
        let devices = list_input_devices().expect("should list input devices");
        for device in devices {
            assert!(
                input_device_index_for_id(&device.id)
                    .expect("stable input device lookup should succeed")
                    .is_some(),
                "enumerated input device must resolve: {}",
                device.id
            );
        }
        assert_eq!(
            input_device_index_for_id("pa-input-mme-deadbeefdeadbeef")
                .expect("unknown ID lookup should succeed"),
            None
        );
    }

    #[cfg(autolive_has_portaudio)]
    #[test]
    fn start_write_short_tone_and_stop() {
        let probe = probe_portaudio();
        if !probe.available {
            return;
        }
        let mut output = PortAudioOutput::new(DEFAULT_SAMPLE_RATE_HZ, DEFAULT_RING_CAPACITY_KIB, 2);
        output.start().expect("start default output");
        let mut phase = 0.0_f32;
        let delta = std::f32::consts::TAU * 440.0 / DEFAULT_SAMPLE_RATE_HZ as f32;
        let mut buffer = vec![0.0_f32; 512 * 2];
        for _ in 0..20 {
            for frame in 0..512 {
                let sample = phase.sin() * 0.05;
                phase = (phase + delta) % std::f32::consts::TAU;
                buffer[frame * 2] = sample;
                buffer[frame * 2 + 1] = sample;
            }
            output.write_stereo_interleaved(&buffer).expect("write");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        output.stop();
    }
}
