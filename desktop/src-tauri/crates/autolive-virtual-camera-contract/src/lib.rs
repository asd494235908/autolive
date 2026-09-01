//! AkVirtualCamera 输出链的无平台核心契约。
//!
//! 该 crate 不创建窗口、不访问 Direct3D，也不启动 GPL 组件。它只约束跨线程/跨进程
//! 边界上的配置、状态、代际、最新帧策略和可观测事实。Windows WGC/D3D11 与 GPL
//! sidecar 必须在上层通过真实能力事实调用 `mark_ready`，否则状态不会进入 Ready。

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

pub const VIRTUAL_CAMERA_DEVICE_NAME: &str = "GpAutoLive Camera";
pub const VIRTUAL_CAMERA_WIDTH: u32 = 1280;
pub const VIRTUAL_CAMERA_HEIGHT: u32 = 720;
pub const VIRTUAL_CAMERA_FPS: u32 = 30;
pub const VIRTUAL_CAMERA_CAPTURE_API: &str = "windows_graphics_capture";
pub const VIRTUAL_CAMERA_TRANSPORT: &str = "akvcam_mmap_cpu";
pub const VIRTUAL_CAMERA_ZERO_COPY: bool = false;
const READBACK_SAMPLE_CAPACITY: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PixelFormat {
    #[serde(rename = "YUY2")]
    Yuy2,
}

impl PixelFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Yuy2 => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualCameraConfig {
    pub device_name: String,
    pub pixel_format: PixelFormat,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub zero_copy: bool,
}

impl Default for VirtualCameraConfig {
    fn default() -> Self {
        Self {
            device_name: VIRTUAL_CAMERA_DEVICE_NAME.to_owned(),
            pixel_format: PixelFormat::Yuy2,
            width: VIRTUAL_CAMERA_WIDTH,
            height: VIRTUAL_CAMERA_HEIGHT,
            fps: VIRTUAL_CAMERA_FPS,
            zero_copy: VIRTUAL_CAMERA_ZERO_COPY,
        }
    }
}

impl VirtualCameraConfig {
    pub fn frame_bytes(&self) -> Result<usize, VirtualCameraError> {
        let pixels = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .ok_or(VirtualCameraError::InvalidConfiguration("画面尺寸溢出"))?;
        let bytes = pixels
            .checked_mul(self.pixel_format.bytes_per_pixel() as u64)
            .ok_or(VirtualCameraError::InvalidConfiguration("帧大小溢出"))?;
        usize::try_from(bytes)
            .map_err(|_| VirtualCameraError::InvalidConfiguration("帧大小超出平台上限"))
    }

    pub fn validate_fixed_output(&self) -> Result<(), VirtualCameraError> {
        let expected = Self::default();
        if self != &expected {
            return Err(VirtualCameraError::InvalidConfiguration(
                "首版虚拟摄像头只允许 YUY2 1280×720@30fps 且 zero_copy=false",
            ));
        }
        let frame_bytes = self.frame_bytes()?;
        if frame_bytes == 0 || frame_bytes > 64 * 1024 * 1024 {
            return Err(VirtualCameraError::InvalidConfiguration(
                "输出帧大小不在受控范围内",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VirtualCameraState {
    Unavailable,
    Installed,
    Starting,
    Ready,
    Streaming,
    Recovering,
    Failed,
    Stopping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuCaptureFacts {
    pub capture_api: String,
    pub adapter_luid: String,
    pub adapter_name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    pub feature_level: String,
    pub is_warp: bool,
    pub gpu_scale: bool,
    pub gpu_color_convert: bool,
    pub transport: String,
    pub zero_copy: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl GpuCaptureFacts {
    pub fn validate_for(&self, config: &VirtualCameraConfig) -> Result<(), VirtualCameraError> {
        if self.capture_api != VIRTUAL_CAMERA_CAPTURE_API {
            return Err(VirtualCameraError::GpuGateFailed(
                "捕获 API 不是 Windows Graphics Capture",
            ));
        }
        if self.adapter_luid.trim().is_empty() {
            return Err(VirtualCameraError::GpuGateFailed("D3D11 adapter LUID 缺失"));
        }
        if self.vendor_id == 0 {
            return Err(VirtualCameraError::GpuGateFailed(
                "D3D11 adapter 厂商 ID 缺失",
            ));
        }
        if self.adapter_name.trim().is_empty() || self.feature_level.trim().is_empty() {
            return Err(VirtualCameraError::GpuGateFailed(
                "GPU 设备名称或 feature level 缺失",
            ));
        }
        if self.is_warp {
            return Err(VirtualCameraError::GpuGateFailed(
                "WARP 软件适配器不能作为 GPU 输出",
            ));
        }
        if !self.gpu_scale || !self.gpu_color_convert {
            return Err(VirtualCameraError::GpuGateFailed(
                "缩放和色彩转换必须由 GPU 完成",
            ));
        }
        if self.transport != VIRTUAL_CAMERA_TRANSPORT || self.zero_copy {
            return Err(VirtualCameraError::GpuGateFailed(
                "首版只允许 AkVirtualCamera CPU raw 传输且明确 zero_copy=false",
            ));
        }
        if self.width != config.width || self.height != config.height || self.fps != config.fps {
            return Err(VirtualCameraError::GpuGateFailed(
                "实际输出规格与固定 720p30 契约不一致",
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    fn test_fixture() -> Self {
        Self {
            capture_api: VIRTUAL_CAMERA_CAPTURE_API.to_owned(),
            adapter_luid: "test-adapter-luid".to_owned(),
            adapter_name: "Test GPU".to_owned(),
            vendor_id: 0x1002,
            device_id: 0x744c,
            feature_level: "11_0".to_owned(),
            is_warp: false,
            gpu_scale: true,
            gpu_color_convert: true,
            transport: VIRTUAL_CAMERA_TRANSPORT.to_owned(),
            zero_copy: false,
            width: VIRTUAL_CAMERA_WIDTH,
            height: VIRTUAL_CAMERA_HEIGHT,
            fps: VIRTUAL_CAMERA_FPS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VirtualCameraError {
    InvalidConfiguration(&'static str),
    InvalidTransition {
        state: VirtualCameraState,
        operation: &'static str,
    },
    GpuGateFailed(&'static str),
    StaleGeneration {
        expected: u64,
        received: u64,
    },
    InvalidFrameSize {
        expected: usize,
        received: usize,
    },
    FrameTooLarge {
        received: usize,
    },
}

impl fmt::Display for VirtualCameraError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message) => {
                write!(formatter, "无效的虚拟摄像头配置：{message}")
            }
            Self::InvalidTransition { state, operation } => {
                write!(
                    formatter,
                    "虚拟摄像头状态 {:?} 不允许执行 {operation}",
                    state
                )
            }
            Self::GpuGateFailed(message) => write!(formatter, "GPU 虚拟摄像头准入失败：{message}"),
            Self::StaleGeneration { expected, received } => write!(
                formatter,
                "虚拟摄像头帧代际过期：当前 {expected}，收到 {received}"
            ),
            Self::InvalidFrameSize { expected, received } => {
                write!(
                    formatter,
                    "虚拟摄像头帧大小无效：需要 {expected} 字节，收到 {received} 字节"
                )
            }
            Self::FrameTooLarge { received } => {
                write!(
                    formatter,
                    "虚拟摄像头帧超过 64 MiB 上限：收到 {received} 字节"
                )
            }
        }
    }
}

impl std::error::Error for VirtualCameraError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualCameraFrame {
    pub generation: u64,
    pub sequence: u64,
    pub timestamp_90khz: u64,
    pub payload: Vec<u8>,
}

impl VirtualCameraFrame {
    pub fn black(
        config: &VirtualCameraConfig,
        generation: u64,
        sequence: u64,
        timestamp_90khz: u64,
    ) -> Result<Self, VirtualCameraError> {
        config.validate_fixed_output()?;
        let mut payload = vec![0_u8; config.frame_bytes()?];
        // YUY2 limited-range black: Y=16, U=128, V=128. All-zero bytes are
        // not black in YUV and would be rendered with a green cast downstream.
        for pixel_pair in payload.chunks_exact_mut(4) {
            pixel_pair.copy_from_slice(&[16, 128, 16, 128]);
        }
        Self::new(generation, sequence, timestamp_90khz, payload)
    }

    pub fn new(
        generation: u64,
        sequence: u64,
        timestamp_90khz: u64,
        payload: Vec<u8>,
    ) -> Result<Self, VirtualCameraError> {
        if payload.len() > 64 * 1024 * 1024 {
            return Err(VirtualCameraError::FrameTooLarge {
                received: payload.len(),
            });
        }
        Ok(Self {
            generation,
            sequence,
            timestamp_90khz,
            payload,
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputContext {
    pub playback_active: bool,
    pub video_source_active: bool,
    pub paused: bool,
    pub stopped: bool,
    pub locked: bool,
    pub has_valid_frame: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputPolicy {
    Black,
    LatestFrame,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualCameraMetrics {
    pub frames_submitted: u64,
    pub frames_delivered: u64,
    pub frames_dropped: u64,
    pub stale_frames_rejected: u64,
    pub frame_sequence_advances: u64,
    pub readback_count: u64,
    pub readback_avg_us: Option<u64>,
    pub readback_p50_us: Option<u64>,
    pub readback_p95_us: Option<u64>,
    pub readback_p99_us: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualCameraStatus {
    pub state: VirtualCameraState,
    pub config: VirtualCameraConfig,
    pub generation: u64,
    pub gpu: Option<GpuCaptureFacts>,
    /// 最近一次由 GPL sidecar 查询到的 DirectShow 客户端数量。
    /// `None` 表示当前 session 尚未返回客户端状态，而不是 0。
    pub downstream_client_count: Option<u32>,
    pub metrics: VirtualCameraMetrics,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualCameraOutputManager {
    config: VirtualCameraConfig,
    state: VirtualCameraState,
    generation: u64,
    gpu: Option<GpuCaptureFacts>,
    downstream_client_count: Option<u32>,
    latest_frame: Option<VirtualCameraFrame>,
    output_context: OutputContext,
    last_delivered_sequence: Option<u64>,
    metrics: VirtualCameraMetrics,
    readback_samples_us: VecDeque<u64>,
    last_error: Option<String>,
}

impl Default for VirtualCameraOutputManager {
    fn default() -> Self {
        Self {
            config: VirtualCameraConfig::default(),
            state: VirtualCameraState::Unavailable,
            generation: 1,
            gpu: None,
            downstream_client_count: None,
            latest_frame: None,
            output_context: OutputContext::default(),
            last_delivered_sequence: None,
            metrics: VirtualCameraMetrics::default(),
            readback_samples_us: VecDeque::with_capacity(READBACK_SAMPLE_CAPACITY),
            last_error: None,
        }
    }
}

impl VirtualCameraOutputManager {
    pub fn new(config: VirtualCameraConfig) -> Result<Self, VirtualCameraError> {
        config.validate_fixed_output()?;
        Ok(Self {
            config,
            ..Self::default()
        })
    }

    pub fn state(&self) -> VirtualCameraState {
        self.state
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn config(&self) -> &VirtualCameraConfig {
        &self.config
    }

    pub fn metrics(&self) -> VirtualCameraMetrics {
        self.metrics.clone()
    }

    pub fn status(&self) -> VirtualCameraStatus {
        VirtualCameraStatus {
            state: self.state,
            config: self.config.clone(),
            generation: self.generation,
            gpu: self.gpu.clone(),
            downstream_client_count: self.downstream_client_count,
            metrics: self.metrics.clone(),
            last_error: self.last_error.clone(),
        }
    }

    /// 更新播放侧事实。默认上下文是全黑策略；调用方必须在播放状态、源类型或
    /// 暂停/锁屏状态变化后同步更新，输出线程不会自行猜测桌面内容。
    pub fn set_output_context(&mut self, context: OutputContext) {
        self.output_context = context;
    }

    pub fn output_context(&self) -> OutputContext {
        self.output_context
    }

    pub fn current_output_policy(&self) -> OutputPolicy {
        self.output_policy(self.output_context)
    }

    pub fn mark_installed(&mut self) -> Result<(), VirtualCameraError> {
        self.transition(VirtualCameraState::Installed, "mark_installed", |manager| {
            manager.gpu = None;
            manager.last_error = None;
            Ok(())
        })
    }

    pub fn mark_unavailable(&mut self, reason: impl Into<String>) {
        self.invalidate_generation();
        self.state = VirtualCameraState::Unavailable;
        self.gpu = None;
        self.last_error = Some(reason.into());
    }

    pub fn begin_start(&mut self) -> Result<(), VirtualCameraError> {
        self.transition(VirtualCameraState::Starting, "begin_start", |_| Ok(()))
    }

    pub fn mark_ready(&mut self, facts: GpuCaptureFacts) -> Result<(), VirtualCameraError> {
        if !matches!(
            self.state,
            VirtualCameraState::Starting | VirtualCameraState::Recovering
        ) {
            return Err(VirtualCameraError::InvalidTransition {
                state: self.state,
                operation: "mark_ready",
            });
        }
        if let Err(error) = facts.validate_for(&self.config) {
            self.state = VirtualCameraState::Failed;
            self.gpu = None;
            self.last_error = Some(error.to_string());
            self.invalidate_generation();
            return Err(error);
        }
        self.gpu = Some(facts);
        self.last_error = None;
        self.state = VirtualCameraState::Ready;
        Ok(())
    }

    pub fn mark_streaming(&mut self) -> Result<(), VirtualCameraError> {
        self.transition(VirtualCameraState::Streaming, "mark_streaming", |_| Ok(()))
    }

    /// 设置 sidecar 观察到的下游客户端数量，并让 Ready/Streaming 严格对应
    /// “无客户端/至少一个客户端”。没有收到 sidecar 状态前保持 Ready，避免
    /// 把“帧已写入管道”误报成下游正在消费。
    pub fn set_downstream_client_count(&mut self, count: u32) -> Result<(), VirtualCameraError> {
        if !matches!(
            self.state,
            VirtualCameraState::Ready | VirtualCameraState::Streaming
        ) {
            return Err(VirtualCameraError::InvalidTransition {
                state: self.state,
                operation: "set_downstream_client_count",
            });
        }
        self.downstream_client_count = Some(count);
        self.state = if count == 0 {
            VirtualCameraState::Ready
        } else {
            VirtualCameraState::Streaming
        };
        Ok(())
    }

    pub fn begin_recovery(&mut self, reason: impl Into<String>) -> Result<(), VirtualCameraError> {
        if !matches!(
            self.state,
            VirtualCameraState::Ready | VirtualCameraState::Streaming
        ) {
            return Err(VirtualCameraError::InvalidTransition {
                state: self.state,
                operation: "begin_recovery",
            });
        }
        self.invalidate_generation();
        self.state = VirtualCameraState::Recovering;
        self.gpu = None;
        self.last_error = Some(reason.into());
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), VirtualCameraError> {
        if self.state == VirtualCameraState::Unavailable {
            return Ok(());
        }
        if !matches!(
            self.state,
            VirtualCameraState::Starting
                | VirtualCameraState::Ready
                | VirtualCameraState::Streaming
                | VirtualCameraState::Recovering
                | VirtualCameraState::Failed
        ) {
            return Err(VirtualCameraError::InvalidTransition {
                state: self.state,
                operation: "stop",
            });
        }
        self.state = VirtualCameraState::Stopping;
        self.invalidate_generation();
        self.gpu = None;
        self.last_error = None;
        self.state = VirtualCameraState::Installed;
        Ok(())
    }

    pub fn fail(&mut self, reason: impl Into<String>) {
        self.invalidate_generation();
        self.state = VirtualCameraState::Failed;
        self.gpu = None;
        self.last_error = Some(reason.into());
    }

    pub fn advance_generation(&mut self) -> u64 {
        self.invalidate_generation();
        self.generation
    }

    pub fn submit_frame(&mut self, frame: VirtualCameraFrame) -> Result<(), VirtualCameraError> {
        if !matches!(
            self.state,
            VirtualCameraState::Ready | VirtualCameraState::Streaming
        ) {
            return Err(VirtualCameraError::InvalidTransition {
                state: self.state,
                operation: "submit_frame",
            });
        }
        if frame.generation != self.generation {
            self.metrics.stale_frames_rejected =
                self.metrics.stale_frames_rejected.saturating_add(1);
            return Err(VirtualCameraError::StaleGeneration {
                expected: self.generation,
                received: frame.generation,
            });
        }
        let expected = self.config.frame_bytes()?;
        if frame.payload.len() != expected {
            return Err(VirtualCameraError::InvalidFrameSize {
                expected,
                received: frame.payload.len(),
            });
        }
        self.metrics.frames_submitted = self.metrics.frames_submitted.saturating_add(1);
        if self.latest_frame.replace(frame).is_some() {
            self.metrics.frames_dropped = self.metrics.frames_dropped.saturating_add(1);
        }
        Ok(())
    }

    pub fn take_latest_frame(&mut self) -> Option<VirtualCameraFrame> {
        let frame = self.latest_frame.take();
        if let Some(frame) = frame.as_ref() {
            self.metrics.frames_delivered = self.metrics.frames_delivered.saturating_add(1);
            if self
                .last_delivered_sequence
                .is_none_or(|last| frame.sequence > last)
            {
                self.metrics.frame_sequence_advances =
                    self.metrics.frame_sequence_advances.saturating_add(1);
                self.last_delivered_sequence = Some(frame.sequence);
            }
        }
        frame
    }

    pub fn output_policy(&self, context: OutputContext) -> OutputPolicy {
        if matches!(
            self.state,
            VirtualCameraState::Ready | VirtualCameraState::Streaming
        ) && context.playback_active
            && context.video_source_active
            && !context.paused
            && !context.stopped
            && !context.locked
            && context.has_valid_frame
        {
            OutputPolicy::LatestFrame
        } else {
            OutputPolicy::Black
        }
    }

    pub fn record_readback(&mut self, elapsed: Duration) {
        let micros = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
        if self.readback_samples_us.len() == READBACK_SAMPLE_CAPACITY {
            self.readback_samples_us.pop_front();
        }
        self.readback_samples_us.push_back(micros);
        self.metrics.readback_count = self.metrics.readback_count.saturating_add(1);
        self.refresh_percentiles();
    }

    fn transition(
        &mut self,
        target: VirtualCameraState,
        operation: &'static str,
        action: impl FnOnce(&mut Self) -> Result<(), VirtualCameraError>,
    ) -> Result<(), VirtualCameraError> {
        let valid = matches!(
            (self.state, target),
            (
                VirtualCameraState::Unavailable,
                VirtualCameraState::Installed
            ) | (VirtualCameraState::Failed, VirtualCameraState::Installed)
                | (VirtualCameraState::Installed, VirtualCameraState::Starting)
                | (VirtualCameraState::Ready, VirtualCameraState::Streaming)
                | (VirtualCameraState::Recovering, VirtualCameraState::Ready)
        );
        if !valid {
            return Err(VirtualCameraError::InvalidTransition {
                state: self.state,
                operation,
            });
        }
        action(self)?;
        self.state = target;
        Ok(())
    }

    fn invalidate_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.generation = 1;
        }
        if self.latest_frame.take().is_some() {
            self.metrics.frames_dropped = self.metrics.frames_dropped.saturating_add(1);
        }
        self.downstream_client_count = None;
        self.last_delivered_sequence = None;
    }

    fn refresh_percentiles(&mut self) {
        if self.readback_samples_us.is_empty() {
            self.metrics.readback_avg_us = None;
            self.metrics.readback_p50_us = None;
            self.metrics.readback_p95_us = None;
            self.metrics.readback_p99_us = None;
            return;
        }
        let mut samples: Vec<u64> = self.readback_samples_us.iter().copied().collect();
        samples.sort_unstable();
        let sum = samples
            .iter()
            .map(|sample| u128::from(*sample))
            .sum::<u128>();
        self.metrics.readback_avg_us =
            Some((sum / samples.len() as u128).min(u128::from(u64::MAX)) as u64);
        self.metrics.readback_p50_us = Some(percentile(&samples, 0.50));
        self.metrics.readback_p95_us = Some(percentile(&samples, 0.95));
        self.metrics.readback_p99_us = Some(percentile(&samples, 0.99));
    }
}

fn percentile(samples: &[u64], quantile: f64) -> u64 {
    let index = ((samples.len() - 1) as f64 * quantile).ceil() as usize;
    samples[index.min(samples.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_machine_accepts_lifecycle_and_rejects_invalid_edges() {
        let mut manager = VirtualCameraOutputManager::default();

        assert!(manager.begin_start().is_err());
        manager.mark_installed().unwrap();
        assert!(manager.mark_installed().is_err());
        manager.begin_start().unwrap();
        assert!(manager.mark_streaming().is_err());
        manager.mark_ready(GpuCaptureFacts::test_fixture()).unwrap();
        manager.mark_streaming().unwrap();
        manager.begin_recovery("窗口重建").unwrap();
        manager.mark_ready(GpuCaptureFacts::test_fixture()).unwrap();
        manager.stop().unwrap();
        assert_eq!(manager.state(), VirtualCameraState::Installed);
        assert!(manager.stop().is_err());
    }

    #[test]
    fn downstream_client_count_controls_ready_and_streaming_states() {
        let mut manager = ready_manager();
        assert_eq!(manager.status().downstream_client_count, None);
        assert_eq!(manager.state(), VirtualCameraState::Ready);

        manager.set_downstream_client_count(0).unwrap();
        assert_eq!(manager.state(), VirtualCameraState::Ready);
        assert_eq!(manager.status().downstream_client_count, Some(0));

        manager.set_downstream_client_count(2).unwrap();
        assert_eq!(manager.state(), VirtualCameraState::Streaming);
        assert_eq!(manager.status().downstream_client_count, Some(2));

        manager.set_downstream_client_count(0).unwrap();
        assert_eq!(manager.state(), VirtualCameraState::Ready);
        assert_eq!(manager.status().downstream_client_count, Some(0));

        manager.stop().unwrap();
        assert_eq!(manager.status().downstream_client_count, None);
    }

    #[test]
    fn default_contract_is_fixed_to_yuy2_720p30_and_not_zero_copy() {
        let config = VirtualCameraConfig::default();

        assert_eq!(config.device_name, "GpAutoLive Camera");
        assert_eq!(config.pixel_format, PixelFormat::Yuy2);
        assert_eq!(config.width, 1280);
        assert_eq!(config.height, 720);
        assert_eq!(config.fps, 30);
        assert!(!config.zero_copy);
        assert_eq!(config.frame_bytes().unwrap(), 1_843_200);
        assert_eq!(
            serde_json::to_string(&config.pixel_format).unwrap(),
            "\"YUY2\""
        );
    }

    #[test]
    fn custom_output_configuration_is_rejected() {
        let config = VirtualCameraConfig {
            width: 1920,
            ..VirtualCameraConfig::default()
        };

        assert!(matches!(
            VirtualCameraOutputManager::new(config),
            Err(VirtualCameraError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn latest_wins_queue_drops_old_frame_and_rejects_stale_generation() {
        let mut manager = ready_manager();
        manager
            .submit_frame(frame(1, 10))
            .expect("first frame accepted");
        manager
            .submit_frame(frame(1, 11))
            .expect("newest frame accepted");
        assert_eq!(manager.metrics().frames_dropped, 1);
        assert_eq!(manager.take_latest_frame().unwrap().sequence, 11);
        assert_eq!(manager.metrics().frames_delivered, 1);
        assert_eq!(manager.metrics().frame_sequence_advances, 1);

        let generation = manager.advance_generation();
        assert_eq!(generation, 2);
        assert!(matches!(
            manager.submit_frame(frame(1, 12)),
            Err(VirtualCameraError::StaleGeneration {
                expected: 2,
                received: 1
            })
        ));
        assert_eq!(manager.metrics().stale_frames_rejected, 1);
        assert_eq!(manager.take_latest_frame(), None);
    }

    #[test]
    fn frame_size_is_checked_at_the_output_boundary() {
        let mut manager = ready_manager();
        let expected = VirtualCameraConfig::default().frame_bytes().unwrap();
        let frame = VirtualCameraFrame::new(1, 1, 0, vec![0; expected - 1]).unwrap();

        assert!(matches!(
            manager.submit_frame(frame),
            Err(VirtualCameraError::InvalidFrameSize {
                expected: actual_expected,
                received: actual_received
            }) if actual_expected == expected && actual_received == expected - 1
        ));
        assert_eq!(manager.metrics().frames_submitted, 0);
    }

    #[test]
    fn output_policy_emits_black_when_playback_is_not_valid() {
        let manager = ready_manager();

        assert_eq!(
            manager.output_policy(OutputContext::default()),
            OutputPolicy::Black
        );
        assert_eq!(
            manager.output_policy(OutputContext {
                playback_active: true,
                video_source_active: true,
                has_valid_frame: true,
                ..OutputContext::default()
            }),
            OutputPolicy::LatestFrame,
        );
        assert_eq!(manager.current_output_policy(), OutputPolicy::Black);
        let mut manager = manager;
        manager.set_output_context(OutputContext {
            playback_active: true,
            video_source_active: true,
            has_valid_frame: true,
            ..OutputContext::default()
        });
        assert_eq!(manager.current_output_policy(), OutputPolicy::LatestFrame);
        assert_eq!(
            manager.output_policy(OutputContext {
                playback_active: true,
                video_source_active: true,
                paused: true,
                has_valid_frame: true,
                ..OutputContext::default()
            }),
            OutputPolicy::Black,
        );
        let black = VirtualCameraFrame::black(manager.config(), manager.generation(), 1, 90_000)
            .expect("fixed black frame should be constructible");
        assert_eq!(black.payload.len(), 1_843_200);
        assert_eq!(&black.payload[..4], &[16, 128, 16, 128]);
    }

    #[test]
    fn gpu_gate_rejects_warp_or_missing_adapter_identity() {
        let mut manager = VirtualCameraOutputManager::default();
        manager.mark_installed().unwrap();
        manager.begin_start().unwrap();

        let mut facts = GpuCaptureFacts::test_fixture();
        facts.is_warp = true;
        assert!(manager.mark_ready(facts).is_err());
        assert_eq!(manager.state(), VirtualCameraState::Failed);

        let mut manager = VirtualCameraOutputManager::default();
        manager.mark_installed().unwrap();
        manager.begin_start().unwrap();
        let mut facts = GpuCaptureFacts::test_fixture();
        facts.adapter_luid.clear();
        assert!(matches!(
            manager.mark_ready(facts),
            Err(VirtualCameraError::GpuGateFailed(_))
        ));

        let mut manager = VirtualCameraOutputManager::default();
        manager.mark_installed().unwrap();
        manager.begin_start().unwrap();
        let mut facts = GpuCaptureFacts::test_fixture();
        facts.vendor_id = 0;
        assert!(matches!(
            manager.mark_ready(facts),
            Err(VirtualCameraError::GpuGateFailed(_))
        ));

        let mut manager = VirtualCameraOutputManager::default();
        manager.mark_installed().unwrap();
        manager.begin_start().unwrap();
        let mut facts = GpuCaptureFacts::test_fixture();
        facts.gpu_color_convert = false;
        assert!(manager.mark_ready(facts).is_err());
    }

    #[test]
    fn stale_generation_is_rejected_after_recovery_and_percentiles_are_bounded() {
        let mut manager = ready_manager();
        manager.begin_recovery("设备重建").unwrap();
        assert_eq!(manager.state(), VirtualCameraState::Recovering);
        assert!(manager.status().gpu.is_none());
        assert!(manager.submit_frame(frame(1, 20)).is_err());

        manager.mark_ready(GpuCaptureFacts::test_fixture()).unwrap();
        assert_eq!(manager.generation(), 2);
        for value in 0..600 {
            manager.record_readback(Duration::from_micros(value));
        }
        let metrics = manager.metrics();
        assert_eq!(metrics.readback_count, 600);
        assert_eq!(metrics.readback_avg_us, Some(343));
        assert_eq!(metrics.readback_p50_us, Some(344));
        assert_eq!(metrics.readback_p95_us, Some(574));
        assert_eq!(metrics.readback_p99_us, Some(594));
    }

    #[test]
    fn sequence_advances_are_monotonic_and_reset_for_new_generation() {
        let mut manager = ready_manager();
        manager.submit_frame(frame(1, 10)).unwrap();
        assert_eq!(manager.take_latest_frame().unwrap().sequence, 10);
        manager.submit_frame(frame(1, 10)).unwrap();
        assert_eq!(manager.take_latest_frame().unwrap().sequence, 10);
        manager.submit_frame(frame(1, 9)).unwrap();
        assert_eq!(manager.take_latest_frame().unwrap().sequence, 9);
        assert_eq!(manager.metrics().frames_delivered, 3);
        assert_eq!(manager.metrics().frame_sequence_advances, 1);

        manager.begin_recovery("窗口重建").unwrap();
        manager.mark_ready(GpuCaptureFacts::test_fixture()).unwrap();
        manager.submit_frame(frame(2, 1)).unwrap();
        assert_eq!(manager.take_latest_frame().unwrap().sequence, 1);
        assert_eq!(manager.metrics().frame_sequence_advances, 2);
    }

    fn ready_manager() -> VirtualCameraOutputManager {
        let mut manager = VirtualCameraOutputManager::default();
        manager.mark_installed().unwrap();
        manager.begin_start().unwrap();
        manager.mark_ready(GpuCaptureFacts::test_fixture()).unwrap();
        manager
    }

    fn frame(generation: u64, sequence: u64) -> VirtualCameraFrame {
        VirtualCameraFrame::new(
            generation,
            sequence,
            sequence.saturating_mul(3_000),
            vec![0; VirtualCameraConfig::default().frame_bytes().unwrap()],
        )
        .unwrap()
    }
}
