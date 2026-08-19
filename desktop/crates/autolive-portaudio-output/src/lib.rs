//! PortAudio 输出后端：主总线 f32 → 环缓 → 硬件回调。
//! 与 WebView 互斥；探测/启动失败时调用方回退 WebView。

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
// 普通播放只维持换轨预缓冲所需的目标水位；环缓容量仍由用户的 KiB 配置决定。
const TARGET_PLAYBACK_WATERMARK_MS: u32 = 50;

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

fn playback_watermark_samples(sample_rate_hz: u32, channels: u16) -> usize {
    let channels = usize::from(channels.max(1));
    let samples = usize::try_from(
        u64::from(sample_rate_hz)
            .saturating_mul(u64::from(TARGET_PLAYBACK_WATERMARK_MS))
            .saturating_mul(channels as u64)
            / 1_000,
    )
    .unwrap_or(usize::MAX);
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
        pub fn Pa_GetDefaultOutputDevice() -> PaDeviceIndex;
        pub fn Pa_GetDeviceInfo(device: PaDeviceIndex) -> *const PaDeviceInfo;
        pub fn Pa_GetHostApiInfo(host_api: PaHostApiIndex) -> *const PaHostApiInfo;
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
        let count = unsafe { ffi::Pa_GetDeviceCount() };
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
            .finish_non_exhaustive()
    }
}

#[cfg(autolive_has_portaudio)]
struct CallbackUserData {
    shared: Arc<StreamShared>,
    consumer: HeapCons<f32>,
}

#[cfg(any(test, autolive_has_portaudio))]
fn pop_audio_samples(consumer: &mut HeapCons<f32>, output: &mut [f32]) -> usize {
    let filled = consumer.pop_slice(output);
    if filled < output.len() {
        output[filled..].fill(0.0);
    }
    filled
}

fn prepare_mixer_samples(samples: &[f32], output_channels: usize) -> Result<Vec<f32>, String> {
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err("PortAudio 输入包含 NaN 或 Infinity".to_owned());
    }
    match output_channels {
        1 => {
            if !samples.len().is_multiple_of(2) {
                return Err("单声道 PortAudio 需要完整的立体声帧".to_owned());
            }
            Ok(samples
                .chunks_exact(2)
                .map(|frame| (frame[0] + frame[1]) * 0.5)
                .collect())
        }
        2 => {
            if !samples.len().is_multiple_of(2) {
                return Err("立体声 PortAudio 输入未按完整帧对齐".to_owned());
            }
            Ok(samples.to_vec())
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
    _input: *const std::os::raw::c_void,
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
    if data.shared.callback_paused.load(Ordering::Acquire) {
        out.fill(0.0);
        record_callback_timing_and_frames(&data.shared, dac_time_delta_us, 0, channels);
        return ffi::PA_CONTINUE;
    }
    let filled = pop_audio_samples(&mut data.consumer, out);
    record_callback_timing_and_frames(&data.shared, dac_time_delta_us, filled, channels);
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

// SAFETY: stream、producer 和 consumer 的所有权只在控制线程管理；PortAudio
// callback 独占 CallbackUserData 中的 consumer，跨线程共享的状态只有原子值。
unsafe impl Send for PortAudioOutput {}

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
            }),
            producer,
            consumer: Some(consumer),
            #[cfg(autolive_has_portaudio)]
            stream: std::ptr::null_mut(),
            #[cfg(autolive_has_portaudio)]
            user_data: None,
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

    fn target_playback_watermark_samples(&self) -> usize {
        playback_watermark_samples(self.sample_rate_hz, self.channels())
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

    /// 返回应用运行标志、真实 PortAudio 流状态、callback 观测值和环缓水位。
    /// 查询本身不获取 callback 使用的任何锁。
    pub fn stream_health(&self) -> PortAudioStreamHealth {
        #[cfg(autolive_has_portaudio)]
        let hardware_state = query_portaudio_stream_state(self.stream);
        #[cfg(not(autolive_has_portaudio))]
        let hardware_state = PortAudioHardwareState::Unsupported;
        #[cfg(autolive_has_portaudio)]
        let (output_latency_us, actual_sample_rate_hz) = query_portaudio_stream_info(self.stream);
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
                let _ = unsafe { ffi::Pa_CloseStream(stream) };
                let callback_data = unsafe { Box::from_raw(user_ptr) };
                self.consumer = Some(callback_data.consumer);
                self.shutdown_pa_unlocked();
                return Err(format!("Pa_StartStream 失败：{}", pa_error_text(start_err)));
            }
            self.stream = stream;
            // SAFETY: 所有权转回 Box，随 self 生命周期。
            self.user_data = Some(unsafe { Box::from_raw(user_ptr) });
            self.running.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        self.prestart_writes_enabled = false;
        self.shared.callback_paused.store(false, Ordering::Release);
        self.shared.live_pcm_paused.store(false, Ordering::SeqCst);
        #[cfg(autolive_has_portaudio)]
        {
            let _guard = portaudio_lock().lock().unwrap_or_else(|e| e.into_inner());
            if !self.stream.is_null() {
                let _ = unsafe { ffi::Pa_StopStream(self.stream) };
                let _ = unsafe { ffi::Pa_CloseStream(self.stream) };
                self.stream = std::ptr::null_mut();
            }
            if let Some(callback_data) = self.user_data.take() {
                self.consumer = Some(callback_data.consumer);
            }
            self.shutdown_pa_unlocked();
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
        let written = self.push_interleaved_available(&normalized);
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
        let written = self.push_interleaved_available(&normalized);
        Ok(if output_channels == 1 {
            written.saturating_mul(2)
        } else {
            written
        })
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
        let written = self.push_interleaved_available_to_capacity(&normalized);
        Ok(if output_channels == 1 {
            written.saturating_mul(2)
        } else {
            written
        })
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
        let remaining = max_occupied_samples.saturating_sub(self.producer.occupied_len());
        if remaining == 0 {
            return 0;
        }
        self.producer
            .push_iter(samples.iter().copied().take(remaining))
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
        assert_eq!(
            prepare_mixer_samples(&[1.0, 3.0, 2.0, 4.0], 1).unwrap(),
            [2.0, 3.0]
        );
    }

    #[test]
    fn mixer_rejects_incomplete_frame_and_non_finite_input() {
        assert!(prepare_mixer_samples(&[1.0], 1).is_err());
        assert!(prepare_mixer_samples(&[f32::NAN, 0.0], 2).is_err());
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
    fn streaming_writes_stop_at_50ms_without_changing_configured_capacity() {
        let mut output = PortAudioOutput::new(44_100, DEFAULT_RING_CAPACITY_KIB, 2);
        output.set_prestart_writes_enabled(true);
        let target = playback_watermark_samples(44_100, 2);
        assert_eq!(target, 4_410);
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
