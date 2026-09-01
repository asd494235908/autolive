//! Windows Graphics Capture 到 YUY2 的原生 GPU 捕获边界。
//!
//! 该模块只负责：从受管最终效果 HWND 创建 WGC frame pool，在同一硬件
//! D3D11 adapter 上使用 Video Processor 做缩放/色彩转换，并通过三槽
//! staging texture 做一次有界回读。它不负责 AkVirtualCamera 安装、注册或
//! GPL sidecar 的生命周期；上层必须在 release gate 通过后再把帧提交给 sidecar。

use std::fmt;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{OUTPUT_HEIGHT, OUTPUT_WIDTH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureGpuFacts {
    pub adapter_luid: String,
    pub adapter_name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    pub feature_level: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureConfig {
    pub final_effect_window_id: u64,
    pub output_width: u32,
    pub output_height: u32,
    pub output_fps: u32,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            final_effect_window_id: 0,
            output_width: OUTPUT_WIDTH,
            output_height: OUTPUT_HEIGHT,
            output_fps: 30,
        }
    }
}

impl CaptureConfig {
    fn validate(&self) -> Result<(), CaptureError> {
        if self.final_effect_window_id == 0 {
            return Err(CaptureError::InvalidConfiguration("最终效果 HWND 不能为空"));
        }
        if self.output_width == 0
            || self.output_height == 0
            || self.output_width > 4096
            || self.output_height > 4096
        {
            return Err(CaptureError::InvalidConfiguration(
                "输出尺寸必须处于 1..=4096 的受控范围",
            ));
        }
        if self.output_width != OUTPUT_WIDTH || self.output_height != OUTPUT_HEIGHT {
            return Err(CaptureError::InvalidConfiguration(
                "AkVirtualCamera 首版输出尺寸固定为 1280×720",
            ));
        }
        if self.output_fps == 0 || self.output_fps > 120 {
            return Err(CaptureError::InvalidConfiguration(
                "输出帧率必须处于 1..=120 的受控范围",
            ));
        }
        if self.output_fps != 30 {
            return Err(CaptureError::InvalidConfiguration(
                "AkVirtualCamera 首版输出帧率固定为 30fps",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFrame {
    /// YUY2 packed payload，行间 padding 已去除，长度固定为 width*height*2。
    pub payload: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp_100ns: i64,
    pub sequence: u64,
    /// 从 GPU 转换提交到 staging 成功映射的耗时，用于输出链 P50/P95/P99 观测。
    pub readback_elapsed: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureError {
    InvalidConfiguration(&'static str),
    Unsupported(&'static str),
    Windows(String),
    DeviceLost(String),
    FrameSizeChanged {
        expected: (u32, u32),
        received: (u32, u32),
    },
}

impl fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message) => {
                write!(formatter, "无效的 GPU 捕获配置：{message}")
            }
            Self::Unsupported(message) => write!(formatter, "GPU 捕获不可用：{message}"),
            Self::Windows(message) => write!(formatter, "Windows GPU 捕获失败：{message}"),
            Self::DeviceLost(message) => write!(formatter, "D3D11 设备已丢失：{message}"),
            Self::FrameSizeChanged { expected, received } => write!(
                formatter,
                "WGC 窗口尺寸发生变化：期望 {}×{}，实际 {}×{}",
                expected.0, expected.1, received.0, received.1
            ),
        }
    }
}

impl std::error::Error for CaptureError {}

pub struct NativeCaptureSession {
    inner: platform::Session,
}

impl NativeCaptureSession {
    pub fn start(config: CaptureConfig) -> Result<Self, CaptureError> {
        config.validate()?;
        Ok(Self {
            inner: platform::Session::start(config)?,
        })
    }

    /// 返回一帧已完成 staging 回读的 YUY2 数据；没有新帧或三槽均未完成时返回 None。
    pub fn try_next_frame(&mut self) -> Result<Option<CapturedFrame>, CaptureError> {
        self.inner.try_next_frame()
    }

    pub fn stop(&mut self) {
        self.inner.stop();
    }

    pub fn gpu_facts(&self) -> Option<&CaptureGpuFacts> {
        self.inner.gpu_facts()
    }
}

impl Drop for NativeCaptureSession {
    fn drop(&mut self) {
        self.inner.stop();
    }
}

/// 在受管线程上持有 WGC/D3D11 会话，并以一个有界 latest-wins 槽向上层交付帧。
///
/// 该泵不把帧排成无界队列：如果 sidecar 或虚拟摄像头暂时变慢，旧帧会被
/// 新帧覆盖，避免捕获线程反向阻塞最终效果窗口。`stop` 会请求取消并等待
/// 工作线程退出，确保 WinRT apartment 和 D3D11 资源在创建它们的线程释放。
pub struct NativeCapturePump {
    stop: Arc<AtomicBool>,
    slot: Arc<Mutex<Option<Result<CapturedFrame, CaptureError>>>>,
    join: Option<JoinHandle<()>>,
    gpu_facts: CaptureGpuFacts,
    terminal_error: Option<CaptureError>,
}

impl NativeCapturePump {
    pub fn start(config: CaptureConfig) -> Result<Self, CaptureError> {
        config.validate()?;

        let (startup_sender, startup_receiver) =
            std::sync::mpsc::sync_channel::<Result<CaptureGpuFacts, CaptureError>>(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let slot = Arc::new(Mutex::new(None));
        let worker_slot = Arc::clone(&slot);
        let join = thread::Builder::new()
            .name("autolive-wgc-capture".to_owned())
            .spawn(move || {
                let mut session = match NativeCaptureSession::start(config) {
                    Ok(mut session) => {
                        let Some(gpu_facts) = session.gpu_facts().cloned() else {
                            let _ = startup_sender.send(Err(CaptureError::Unsupported(
                                "GPU 捕获会话未返回实际 adapter 事实",
                            )));
                            session.stop();
                            return;
                        };
                        let _ = startup_sender.send(Ok(gpu_facts));
                        session
                    }
                    Err(error) => {
                        let _ = startup_sender.send(Err(error));
                        return;
                    }
                };

                while !worker_stop.load(Ordering::Acquire) {
                    match session.try_next_frame() {
                        Ok(Some(frame)) => replace_slot(&worker_slot, Ok(frame)),
                        Ok(None) => thread::sleep(Duration::from_millis(2)),
                        Err(error) => {
                            replace_slot(&worker_slot, Err(error));
                            break;
                        }
                    }
                }
                session.stop();
            })
            .map_err(|error| CaptureError::Windows(format!("启动 GPU 捕获线程失败：{error}")))?;

        match startup_receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(gpu_facts)) => Ok(Self {
                stop,
                slot,
                join: Some(join),
                gpu_facts,
                terminal_error: None,
            }),
            Ok(Err(error)) => {
                stop.store(true, Ordering::Release);
                let _ = join.join();
                Err(error)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                stop.store(true, Ordering::Release);
                let _ = join.join();
                Err(CaptureError::Windows("GPU 捕获线程启动超时".to_owned()))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                stop.store(true, Ordering::Release);
                let _ = join.join();
                Err(CaptureError::Windows(
                    "GPU 捕获线程未返回启动结果".to_owned(),
                ))
            }
        }
    }

    /// 非阻塞地读取最新的一帧；旧帧在新帧到达时会被覆盖。
    pub fn try_next_frame(&mut self) -> Result<Option<CapturedFrame>, CaptureError> {
        if let Some(error) = &self.terminal_error {
            return Err(error.clone());
        }
        let value = take_slot(&self.slot);
        match value {
            Some(Ok(frame)) => Ok(Some(frame)),
            Some(Err(error)) => {
                self.terminal_error = Some(error.clone());
                Err(error)
            }
            None => Ok(None),
        }
    }

    pub fn stop(&mut self) -> Result<(), CaptureError> {
        self.stop.store(true, Ordering::Release);
        let Some(join) = self.join.take() else {
            return Ok(());
        };
        join.join()
            .map_err(|_| CaptureError::Windows("GPU 捕获线程异常退出".to_owned()))
    }

    pub fn gpu_facts(&self) -> &CaptureGpuFacts {
        &self.gpu_facts
    }
}

impl Drop for NativeCapturePump {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn replace_slot(
    slot: &Mutex<Option<Result<CapturedFrame, CaptureError>>>,
    value: Result<CapturedFrame, CaptureError>,
) {
    let mut guard = match slot.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = Some(value);
}

fn take_slot(
    slot: &Mutex<Option<Result<CapturedFrame, CaptureError>>>,
) -> Option<Result<CapturedFrame, CaptureError>> {
    let mut guard = match slot.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.take()
}

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::gpu_yuy2_pack::GpuYuy2Pack;
    use std::ffi::c_void;
    use std::mem::ManuallyDrop;
    use std::ptr::null_mut;

    use windows::core::{factory, Error as WindowsError, Interface};
    use windows::Foundation::TimeSpan;
    use windows::Graphics::Capture::{
        Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
    };
    use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use windows::Graphics::DirectX::DirectXPixelFormat;
    use windows::Win32::Foundation::{E_BOUNDS, HWND};
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
        D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_FLAG_DO_NOT_WAIT,
        D3D11_MAP_READ, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC as TextureDesc,
        D3D11_USAGE_DEFAULT, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
        D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT,
        D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
        D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
        D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
        D3D11_VIDEO_USAGE_PLAYBACK_NORMAL, D3D11_VPIV_DIMENSION_TEXTURE2D,
        D3D11_VPOV_DIMENSION_TEXTURE2D,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
    };
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
    };
    use windows::Win32::System::WinRT::Direct3D11::{
        CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
    };
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
    use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

    const DXGI_ERROR_NOT_FOUND: u32 = 0x887a0002;
    const DXGI_ERROR_WAS_STILL_DRAWING: u32 = 0x887a000a;
    const DXGI_ERROR_DEVICE_REMOVED: u32 = 0x887a0005;
    const DXGI_ERROR_DEVICE_HUNG: u32 = 0x887a0006;
    const DXGI_ERROR_DEVICE_RESET: u32 = 0x887a0007;
    const DXGI_ERROR_DRIVER_INTERNAL_ERROR: u32 = 0x887a0020;
    const STAGING_SLOTS: usize = 3;

    struct RoGuard;

    impl Drop for RoGuard {
        fn drop(&mut self) {
            // SAFETY: RoInitialize 成功后在同一线程成对调用 RoUninitialize。
            unsafe { RoUninitialize() };
        }
    }

    pub(super) struct Session {
        frame_pool: Direct3D11CaptureFramePool,
        capture_session: GraphicsCaptureSession,
        device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
        context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
        video_context: windows::Win32::Graphics::Direct3D11::ID3D11VideoContext,
        video_processor: windows::Win32::Graphics::Direct3D11::ID3D11VideoProcessor,
        video_enumerator: windows::Win32::Graphics::Direct3D11::ID3D11VideoProcessorEnumerator,
        output_texture: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
        yuy2_pack: GpuYuy2Pack,
        pending: [bool; STAGING_SLOTS],
        pending_timestamps: [i64; STAGING_SLOTS],
        pending_sequences: [u64; STAGING_SLOTS],
        pending_started_at: [Option<Instant>; STAGING_SLOTS],
        input_size: (u32, u32),
        output_size: (u32, u32),
        next_slot: usize,
        sequence: u64,
        last_delivered_sequence: u64,
        stopped: bool,
        // 必须最后析构：所有 WGC/WinRT 对象释放后才能调用 RoUninitialize。
        _ro: RoGuard,
        gpu_facts: CaptureGpuFacts,
    }

    impl Session {
        pub(super) fn start(config: CaptureConfig) -> Result<Self, CaptureError> {
            // SAFETY: 当前线程独占初始化 WinRT apartment；失败不会创建半初始化会话。
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
                .map_err(|error| windows_error("初始化 WinRT", error))?;
            let ro = RoGuard;

            let interop: IGraphicsCaptureItemInterop =
                factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
                    .map_err(|error| windows_error("获取 GraphicsCaptureItem 工厂", error))?;
            let item: GraphicsCaptureItem = unsafe {
                interop.CreateForWindow(HWND(config.final_effect_window_id as *mut c_void))
            }
            .map_err(|error| windows_error("从最终效果 HWND 创建 WGC item", error))?;
            let size = item
                .Size()
                .map_err(|error| windows_error("读取 WGC item 尺寸", error))?;
            let input_size = (
                u32::try_from(size.Width).unwrap_or(0),
                u32::try_from(size.Height).unwrap_or(0),
            );
            if input_size.0 == 0 || input_size.1 == 0 {
                return Err(CaptureError::Unsupported("最终效果 HWND 的 WGC 尺寸为空"));
            }

            // 先读取窗口尺寸，再逐个选择能够创建 GPU 输出面的硬件 adapter。
            // 远程/虚拟显示 adapter 可能能创建设备，却拒绝 Video Processor
            // 或 BGRA render-target；不能让它抢占真实 GPU 后在会话初始化中途失败。
            let (device, context, gpu_facts) = create_hardware_device(
                input_size,
                (config.output_width, config.output_height),
                config.output_fps,
            )?;
            let dxgi_device = device
                .cast::<windows::Win32::Graphics::Dxgi::IDXGIDevice>()
                .map_err(|error| windows_error("获取 IDXGIDevice", error))?;
            let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }
                .map_err(|error| windows_error("包装 WinRT Direct3D11 设备", error))?;
            let direct_device: IDirect3DDevice = inspectable
                .cast()
                .map_err(|error| windows_error("转换 IDirect3DDevice", error))?;

            let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &direct_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                STAGING_SLOTS as i32,
                size,
            )
            .map_err(|error| windows_error("创建 WGC frame pool", error))?;
            let capture_session = frame_pool
                .CreateCaptureSession(&item)
                .map_err(|error| windows_error("创建 WGC capture session", error))?;
            // 产品只捕获应用自己的最终效果，不把系统光标或边框混入视频帧。
            let _ = capture_session.SetIsCursorCaptureEnabled(false);
            let _ = capture_session.SetIsBorderRequired(false);

            let video_device = device
                .cast::<windows::Win32::Graphics::Direct3D11::ID3D11VideoDevice>()
                .map_err(|error| windows_error("获取 ID3D11VideoDevice", error))?;
            let video_context = context
                .cast::<windows::Win32::Graphics::Direct3D11::ID3D11VideoContext>()
                .map_err(|error| windows_error("获取 ID3D11VideoContext", error))?;
            let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: DXGI_RATIONAL {
                    Numerator: config.output_fps,
                    Denominator: 1,
                },
                InputWidth: input_size.0,
                InputHeight: input_size.1,
                OutputFrameRate: DXGI_RATIONAL {
                    Numerator: config.output_fps,
                    Denominator: 1,
                },
                OutputWidth: config.output_width,
                OutputHeight: config.output_height,
                Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            };
            let video_enumerator = unsafe { video_device.CreateVideoProcessorEnumerator(&content) }
                .map_err(|error| windows_error("创建 D3D11 Video Processor 枚举器", error))?;
            check_video_processor_formats(&video_enumerator)?;
            let video_processor =
                unsafe { video_device.CreateVideoProcessor(&video_enumerator, 0) }
                    .map_err(|error| windows_error("创建 D3D11 Video Processor", error))?;
            unsafe {
                video_context.VideoProcessorSetStreamFrameFormat(
                    &video_processor,
                    0,
                    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                );
            }

            let output_texture = create_texture(
                &device,
                "GPU BGRA 输出纹理",
                config.output_width,
                config.output_height,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                D3D11_USAGE_DEFAULT,
                (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                0,
            )?;
            let yuy2_pack = GpuYuy2Pack::new(
                &device,
                &output_texture,
                config.output_width,
                config.output_height,
            )
            .map_err(|error| CaptureError::Windows(error.to_string()))?;
            capture_session
                .StartCapture()
                .map_err(|error| windows_error("启动 WGC capture session", error))?;
            Ok(Self {
                _ro: ro,
                frame_pool,
                capture_session,
                device,
                context,
                video_context,
                video_processor,
                video_enumerator,
                output_texture,
                yuy2_pack,
                pending: [false; STAGING_SLOTS],
                pending_timestamps: [0; STAGING_SLOTS],
                pending_sequences: [0; STAGING_SLOTS],
                pending_started_at: [None; STAGING_SLOTS],
                input_size,
                output_size: (config.output_width, config.output_height),
                next_slot: 0,
                sequence: 0,
                last_delivered_sequence: 0,
                stopped: false,
                gpu_facts,
            })
        }

        pub(super) fn gpu_facts(&self) -> Option<&CaptureGpuFacts> {
            Some(&self.gpu_facts)
        }

        pub(super) fn try_next_frame(&mut self) -> Result<Option<CapturedFrame>, CaptureError> {
            if self.stopped {
                return Ok(None);
            }

            // 先轮询所有已经提交的 staging 槽。三槽允许在 GPU 完成上一帧
            // 回读前继续提交新的 WGC 帧；同一轮若有多个槽完成，只把序列号
            // 最大的一帧交给上层，避免把过期画面重新写回 latest-wins 槽。
            let mut newest_completed = None;
            for offset in 0..STAGING_SLOTS {
                let index = (self.next_slot + offset) % STAGING_SLOTS;
                if !self.pending[index] {
                    continue;
                }
                if let Some(frame) = self.map_staging(
                    index,
                    self.pending_timestamps[index],
                    self.pending_sequences[index],
                )? {
                    self.pending[index] = false;
                    self.next_slot = (index + 1) % STAGING_SLOTS;
                    if frame.sequence <= self.last_delivered_sequence {
                        continue;
                    }
                    let should_replace = newest_completed
                        .as_ref()
                        .map_or(true, |current: &CapturedFrame| {
                            frame.sequence > current.sequence
                        });
                    if should_replace {
                        newest_completed = Some(frame);
                    }
                }
            }

            // 只要存在空槽就继续消费一帧 WGC 输入；没有空槽时保留已完成
            // 的结果并让调用方稍后再次轮询，绝不阻塞最终效果窗口。
            let slot = (0..STAGING_SLOTS)
                .map(|offset| (self.next_slot + offset) % STAGING_SLOTS)
                .find(|index| !self.pending[*index]);
            if let Some(slot) = slot {
                let frame = match self.frame_pool.TryGetNextFrame() {
                    Ok(frame) => Some(frame),
                    Err(error) if error.code() == E_BOUNDS => None,
                    // 某些 Windows 11 构建在首帧尚未到达时返回 S_OK + null
                    // frame；windows crate 将其包装成 code=0 的 Error。把它视为
                    // 暂无新帧，避免把可恢复的启动竞态升级为终止错误。
                    Err(error) if error.code().0 == 0 => None,
                    Err(error) => return Err(windows_error("读取 WGC 下一帧", error)),
                };
                let Some(frame) = frame else {
                    if let Some(frame) = newest_completed.as_ref() {
                        self.last_delivered_sequence = frame.sequence;
                    }
                    return Ok(newest_completed);
                };
                let content_size = frame
                    .ContentSize()
                    .map_err(|error| windows_error("读取 WGC 帧尺寸", error))?;
                let received = (
                    u32::try_from(content_size.Width).unwrap_or(0),
                    u32::try_from(content_size.Height).unwrap_or(0),
                );
                if received != self.input_size {
                    return Err(CaptureError::FrameSizeChanged {
                        expected: self.input_size,
                        received,
                    });
                }
                let surface = frame
                    .Surface()
                    .map_err(|error| windows_error("获取 WGC D3D11 surface", error))?;
                let access: IDirect3DDxgiInterfaceAccess = surface
                    .cast()
                    .map_err(|error| windows_error("获取 WGC DXGI interface access", error))?;
                let input_texture: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D =
                    unsafe { access.GetInterface() }
                        .map_err(|error| windows_error("读取 WGC ID3D11Texture2D", error))?;
                let submitted_at = Instant::now();
                self.convert_to_yuy2(&input_texture, slot)?;
                self.sequence = self.sequence.saturating_add(1);
                let timestamp = frame
                    .SystemRelativeTime()
                    .map(|value: TimeSpan| value.Duration)
                    .unwrap_or_default();
                self.pending_timestamps[slot] = timestamp;
                self.pending_sequences[slot] = self.sequence;
                self.pending_started_at[slot] = Some(submitted_at);
                self.pending[slot] = true;
                if let Some(mapped) = self.map_staging(slot, timestamp, self.sequence)? {
                    // 当前 GPU 已经完成回读，立即交付这一槽，避免人为增加一帧延迟。
                    self.pending[slot] = false;
                    if mapped.sequence > self.last_delivered_sequence {
                        let should_replace = newest_completed
                            .as_ref()
                            .map_or(true, |current: &CapturedFrame| {
                                mapped.sequence > current.sequence
                            });
                        if should_replace {
                            newest_completed = Some(mapped);
                        }
                    }
                }
            }
            if let Some(frame) = newest_completed.as_ref() {
                self.last_delivered_sequence = frame.sequence;
            }
            Ok(newest_completed)
        }

        fn convert_to_yuy2(
            &self,
            input_texture: &windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
            slot: usize,
        ) -> Result<(), CaptureError> {
            let video_device = self
                .device
                .cast::<windows::Win32::Graphics::Direct3D11::ID3D11VideoDevice>()
                .map_err(|error| windows_error("获取 Video Processor 设备", error))?;
            let input_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPIV {
                        MipSlice: 0,
                        ArraySlice: 0,
                    },
                },
            };
            let mut input_view = None;
            unsafe {
                video_device.CreateVideoProcessorInputView(
                    input_texture,
                    &self.video_enumerator,
                    &input_desc,
                    Some(&mut input_view),
                )
            }
            .map_err(|error| windows_error("创建 Video Processor 输入视图", error))?;
            let input_view =
                input_view.ok_or_else(|| CaptureError::Windows("输入视图为空".to_owned()))?;

            let output_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            };
            let mut output_view = None;
            unsafe {
                video_device.CreateVideoProcessorOutputView(
                    &self.output_texture,
                    &self.video_enumerator,
                    &output_desc,
                    Some(&mut output_view),
                )
            }
            .map_err(|error| windows_error("创建 Video Processor 输出视图", error))?;
            let output_view =
                output_view.ok_or_else(|| CaptureError::Windows("输出视图为空".to_owned()))?;

            let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                OutputIndex: 0,
                InputFrameOrField: 0,
                PastFrames: 0,
                FutureFrames: 0,
                ppPastSurfaces: null_mut(),
                pInputSurface: ManuallyDrop::new(Some(input_view)),
                ppFutureSurfaces: null_mut(),
                ppPastSurfacesRight: null_mut(),
                pInputSurfaceRight: ManuallyDrop::new(None),
                ppFutureSurfacesRight: null_mut(),
            };
            let blt_result = unsafe {
                self.video_context.VideoProcessorBlt(
                    &self.video_processor,
                    &output_view,
                    0,
                    std::slice::from_ref(&stream),
                )
            };
            // 无论 VideoProcessorBlt 成功还是失败，都必须释放输入视图；否则
            // 驱动拒绝一次提交时会把 COM 资源遗留到整个捕获会话结束。
            unsafe { ManuallyDrop::drop(&mut stream.pInputSurface) };
            blt_result
                .map_err(|error| windows_error("执行 GPU Video Processor BGRA 输出", error))?;
            self.yuy2_pack
                .render(&self.context, slot)
                .map_err(|error| CaptureError::Windows(error.to_string()))?;
            Ok(())
        }

        fn map_staging(
            &mut self,
            index: usize,
            timestamp_100ns: i64,
            sequence: u64,
        ) -> Result<Option<CapturedFrame>, CaptureError> {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            let staging = self
                .yuy2_pack
                .staging_texture(index)
                .ok_or(CaptureError::InvalidConfiguration("YUY2 staging 槽位无效"))?;
            let result = unsafe {
                self.context.Map(
                    staging,
                    0,
                    D3D11_MAP_READ,
                    D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32,
                    Some(&mut mapped),
                )
            };
            match result {
                Ok(()) => {
                    let row_bytes = usize::try_from(self.output_size.0)
                        .ok()
                        .and_then(|width| width.checked_mul(2))
                        .ok_or(CaptureError::InvalidConfiguration("YUY2 行大小溢出"))?;
                    let total = row_bytes
                        .checked_mul(self.output_size.1 as usize)
                        .ok_or(CaptureError::InvalidConfiguration("YUY2 帧大小溢出"))?;
                    let mut payload = vec![0_u8; total];
                    let row_pitch = mapped.RowPitch as usize;
                    if mapped.pData.is_null() || row_pitch < row_bytes {
                        unsafe { self.context.Unmap(staging, 0) };
                        return Err(CaptureError::Windows("staging 行步长无效".to_owned()));
                    }
                    for row in 0..self.output_size.1 as usize {
                        let source = unsafe {
                            std::slice::from_raw_parts(
                                (mapped.pData as *const u8).add(row * row_pitch),
                                row_bytes,
                            )
                        };
                        payload[row * row_bytes..(row + 1) * row_bytes].copy_from_slice(source);
                    }
                    unsafe { self.context.Unmap(staging, 0) };
                    let readback_elapsed = self.pending_started_at[index]
                        .take()
                        .map(|started_at| started_at.elapsed())
                        .unwrap_or_default();
                    Ok(Some(CapturedFrame {
                        payload,
                        width: self.output_size.0,
                        height: self.output_size.1,
                        timestamp_100ns,
                        sequence,
                        readback_elapsed,
                    }))
                }
                Err(error) => {
                    let code = error.code().0 as u32;
                    if code == DXGI_ERROR_WAS_STILL_DRAWING {
                        Ok(None)
                    } else {
                        Err(windows_error("映射 staging 回读纹理", error))
                    }
                }
            }
        }

        pub(super) fn stop(&mut self) {
            if self.stopped {
                return;
            }
            self.stopped = true;
            let _ = self.capture_session.Close();
            let _ = self.frame_pool.Close();
        }
    }

    fn create_hardware_device(
        input_size: (u32, u32),
        output_size: (u32, u32),
        fps: u32,
    ) -> Result<
        (
            windows::Win32::Graphics::Direct3D11::ID3D11Device,
            windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
            CaptureGpuFacts,
        ),
        CaptureError,
    > {
        let factory = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }
            .map_err(|error| windows_error("创建 DXGI 工厂", error))?;
        let mut first_error = None;
        for index in 0..32 {
            let adapter = match unsafe { factory.EnumAdapters1(index) } {
                Ok(adapter) => adapter,
                Err(error) if error.code().0 as u32 == DXGI_ERROR_NOT_FOUND => break,
                Err(error) => {
                    first_error.get_or_insert_with(|| windows_error("枚举 DXGI adapter", error));
                    break;
                }
            };
            let description = match unsafe { adapter.GetDesc1() } {
                Ok(description) => description,
                Err(error) => {
                    first_error
                        .get_or_insert_with(|| windows_error("读取 DXGI adapter 描述", error));
                    continue;
                }
            };
            if (description.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                continue;
            }
            let adapter_base: IDXGIAdapter = match adapter.cast() {
                Ok(adapter_base) => adapter_base,
                Err(error) => {
                    first_error.get_or_insert_with(|| windows_error("转换 IDXGIAdapter", error));
                    continue;
                }
            };
            let mut device = None;
            let mut context = None;
            let mut feature_level = D3D_FEATURE_LEVEL(0);
            let levels = [D3D_FEATURE_LEVEL_11_0];
            let created = unsafe {
                D3D11CreateDevice(
                    Some(&adapter_base),
                    D3D_DRIVER_TYPE_UNKNOWN,
                    Default::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT
                        | windows::Win32::Graphics::Direct3D11::D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                    Some(&levels),
                    windows::Win32::Graphics::Direct3D11::D3D11_SDK_VERSION,
                    Some(&mut device),
                    Some(&mut feature_level),
                    Some(&mut context),
                )
            };
            if let Err(error) = created {
                first_error.get_or_insert_with(|| windows_error("创建硬件 D3D11 设备", error));
                continue;
            }
            if feature_level.0 < D3D_FEATURE_LEVEL_11_0.0 {
                first_error.get_or_insert(CaptureError::Unsupported(
                    "硬件 D3D11 feature level 低于 11_0",
                ));
                continue;
            }
            let Some(device) = device else {
                first_error.get_or_insert(CaptureError::Windows("D3D11 设备为空".to_owned()));
                continue;
            };
            let Some(context) = context else {
                first_error.get_or_insert(CaptureError::Windows("D3D11 context 为空".to_owned()));
                continue;
            };
            if let Err(error) =
                validate_gpu_output_surface(device.clone(), input_size, output_size, fps)
            {
                first_error.get_or_insert(error);
                continue;
            }
            return Ok((
                device,
                context,
                CaptureGpuFacts {
                    adapter_luid: format_luid(description.AdapterLuid),
                    adapter_name: utf16_string(&description.Description),
                    vendor_id: description.VendorId,
                    device_id: description.DeviceId,
                    feature_level: format!("0x{:04x}", feature_level.0),
                },
            ));
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Err(CaptureError::Unsupported("没有可用的非 WARP D3D11 adapter"))
    }

    fn validate_gpu_output_surface(
        device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
        input_size: (u32, u32),
        output_size: (u32, u32),
        fps: u32,
    ) -> Result<(), CaptureError> {
        let video_device = device
            .cast::<windows::Win32::Graphics::Direct3D11::ID3D11VideoDevice>()
            .map_err(|error| windows_error("获取 Video Processor 设备", error))?;
        let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: DXGI_RATIONAL {
                Numerator: fps,
                Denominator: 1,
            },
            InputWidth: input_size.0,
            InputHeight: input_size.1,
            OutputFrameRate: DXGI_RATIONAL {
                Numerator: fps,
                Denominator: 1,
            },
            OutputWidth: output_size.0,
            OutputHeight: output_size.1,
            Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
        };
        let enumerator = unsafe { video_device.CreateVideoProcessorEnumerator(&content) }
            .map_err(|error| windows_error("创建 D3D11 Video Processor 枚举器", error))?;
        check_video_processor_formats(&enumerator)?;
        create_texture(
            &device,
            "探测 GPU BGRA 输出纹理",
            output_size.0,
            output_size.1,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            D3D11_USAGE_DEFAULT,
            (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            0,
        )
        .and_then(|output_texture| {
            GpuYuy2Pack::new(&device, &output_texture, output_size.0, output_size.1)
                .map(|_| ())
                .map_err(|error| CaptureError::Windows(error.to_string()))
        })?;
        Ok(())
    }

    fn check_video_processor_formats(
        enumerator: &windows::Win32::Graphics::Direct3D11::ID3D11VideoProcessorEnumerator,
    ) -> Result<(), CaptureError> {
        let formats = [(
            DXGI_FORMAT_B8G8R8A8_UNORM,
            D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT.0
                | D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT.0,
        )];
        for (format, required) in formats {
            let flags = unsafe { enumerator.CheckVideoProcessorFormat(format) }
                .map_err(|error| windows_error("查询 Video Processor 格式", error))?;
            if (flags & required as u32) != required as u32 {
                return Err(CaptureError::Unsupported(
                    "当前硬件 Video Processor 不同时支持 BGRA 输入和输出",
                ));
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn create_texture(
        device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
        role: &str,
        width: u32,
        height: u32,
        format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
        usage: windows::Win32::Graphics::Direct3D11::D3D11_USAGE,
        bind_flags: u32,
        cpu_access_flags: u32,
    ) -> Result<windows::Win32::Graphics::Direct3D11::ID3D11Texture2D, CaptureError> {
        let desc = TextureDesc {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: usage,
            BindFlags: bind_flags,
            CPUAccessFlags: cpu_access_flags,
            MiscFlags: 0,
        };
        let mut texture = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }
            .map_err(|error| windows_error(role, error))?;
        texture.ok_or_else(|| CaptureError::Windows("D3D11 texture 为空".to_owned()))
    }

    fn windows_error(context: &str, error: WindowsError) -> CaptureError {
        let code = error.code().0 as u32;
        if matches!(
            code,
            DXGI_ERROR_DEVICE_REMOVED
                | DXGI_ERROR_DEVICE_HUNG
                | DXGI_ERROR_DEVICE_RESET
                | DXGI_ERROR_DRIVER_INTERNAL_ERROR
        ) {
            CaptureError::DeviceLost(format!("{context}：{error}"))
        } else {
            CaptureError::Windows(format!("{context}：{error}"))
        }
    }

    fn utf16_string(value: &[u16]) -> String {
        let length = value
            .iter()
            .position(|item| *item == 0)
            .unwrap_or(value.len());
        String::from_utf16_lossy(&value[..length])
    }

    fn format_luid(value: windows::Win32::Foundation::LUID) -> String {
        format!("{:08x}:{:08x}", value.HighPart as u32, value.LowPart)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_configuration_fails_before_platform_initialization() {
        let error = match NativeCaptureSession::start(CaptureConfig::default()) {
            Ok(_) => panic!("无效配置不应创建捕获会话"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            CaptureError::InvalidConfiguration("最终效果 HWND 不能为空")
        );
    }

    #[test]
    fn invalid_dimensions_and_frame_rate_are_rejected() {
        let error = match NativeCaptureSession::start(CaptureConfig {
            final_effect_window_id: 1,
            output_width: 4097,
            ..CaptureConfig::default()
        }) {
            Ok(_) => panic!("超出范围的输出尺寸不应创建捕获会话"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            CaptureError::InvalidConfiguration("输出尺寸必须处于 1..=4096 的受控范围")
        );

        let error = match NativeCaptureSession::start(CaptureConfig {
            final_effect_window_id: 1,
            output_fps: 121,
            ..CaptureConfig::default()
        }) {
            Ok(_) => panic!("超出范围的输出帧率不应创建捕获会话"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            CaptureError::InvalidConfiguration("输出帧率必须处于 1..=120 的受控范围")
        );

        let error = match NativeCaptureSession::start(CaptureConfig {
            final_effect_window_id: 1,
            output_width: 1920,
            output_height: 1080,
            ..CaptureConfig::default()
        }) {
            Ok(_) => panic!("首版固定输出规格不应创建捕获会话"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            CaptureError::InvalidConfiguration("AkVirtualCamera 首版输出尺寸固定为 1280×720")
        );

        let error = match NativeCaptureSession::start(CaptureConfig {
            final_effect_window_id: 1,
            output_fps: 60,
            ..CaptureConfig::default()
        }) {
            Ok(_) => panic!("首版固定输出帧率不应创建捕获会话"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            CaptureError::InvalidConfiguration("AkVirtualCamera 首版输出帧率固定为 30fps")
        );
    }

    #[test]
    fn pump_rejects_invalid_configuration_before_spawning_worker() {
        let error = match NativeCapturePump::start(CaptureConfig::default()) {
            Ok(_) => panic!("无效配置不应启动捕获泵"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            CaptureError::InvalidConfiguration("最终效果 HWND 不能为空")
        );
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub(super) struct Session;

    impl Session {
        pub(super) fn start(_config: CaptureConfig) -> Result<Self, CaptureError> {
            Err(CaptureError::Unsupported("WGC 仅支持 Windows 10/11"))
        }

        pub(super) fn try_next_frame(&mut self) -> Result<Option<CapturedFrame>, CaptureError> {
            Err(CaptureError::Unsupported("WGC 仅支持 Windows 10/11"))
        }

        pub(super) fn stop(&mut self) {}

        pub(super) fn gpu_facts(&self) -> Option<&CaptureGpuFacts> {
            None
        }
    }
}
