use super::audio_sink::{pcm_f32le_bytes, RtmpAudioSink};
use super::config::{validate_filter, RtmpConfigError, RtmpOutputConfig};
use crate::background_process::background_command;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(windows)]
mod rtmp_job {
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use win32job::{ExtendedLimitInfo, Job};

    #[derive(Debug)]
    pub(super) struct ManagedRtmpJob {
        job: Job,
    }

    impl ManagedRtmpJob {
        pub(super) fn create() -> io::Result<Self> {
            let mut limits = ExtendedLimitInfo::new();
            limits.limit_kill_on_job_close();
            Job::create_with_limit_info(&limits)
                .map(|job| Self { job })
                .map_err(io::Error::from)
        }

        pub(super) fn assign_process(&self, child: &Child) -> io::Result<()> {
            self.job
                .assign_process(child.as_raw_handle() as isize)
                .map_err(io::Error::from)
        }
    }
}

#[cfg(not(windows))]
mod rtmp_job {
    use std::io;
    use std::process::Child;

    #[derive(Debug)]
    pub(super) struct ManagedRtmpJob;

    impl ManagedRtmpJob {
        pub(super) fn create() -> io::Result<Self> {
            Ok(Self)
        }

        pub(super) fn assign_process(&self, _child: &Child) -> io::Result<()> {
            Ok(())
        }
    }
}

use rtmp_job::ManagedRtmpJob;

const MAX_RETRIES: u32 = 5;
const RETRY_BACKOFF_SECONDS: [u64; 5] = [1, 2, 4, 8, 15];
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const AUDIO_RECEIVE_TIMEOUT: Duration = Duration::from_millis(50);
const DEFAULT_RTMP_AUDIO_SAMPLE_RATE_HZ: u32 = 48_000;
const RTMP_AUDIO_OUTPUT_SAMPLE_RATE_HZ: u32 = 48_000;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const NO_PROGRESS_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_STDERR_BYTES: usize = 64 * 1024;
const MAX_STDERR_LINE_BYTES: usize = 8 * 1024;

/// FFmpeg stderr 只保留可行动的固定分类，不把原始目标地址或 token 传播到状态层。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessFailure {
    Deterministic(&'static str),
    Retryable,
}

#[derive(Debug, Default)]
struct ProcessDiagnostics {
    deterministic: Option<&'static str>,
    retryable: bool,
}

impl ProcessDiagnostics {
    fn record(&mut self, failure: ProcessFailure) {
        match failure {
            ProcessFailure::Deterministic(message) => {
                self.deterministic.get_or_insert(message);
            }
            ProcessFailure::Retryable => self.retryable = true,
        }
    }

    fn failure(&self) -> Option<ProcessFailure> {
        self.deterministic
            .map(ProcessFailure::Deterministic)
            .or(self.retryable.then_some(ProcessFailure::Retryable))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RtmpOutputState {
    #[default]
    Idle,
    Validating,
    Starting,
    Publishing,
    Reconnecting,
    Stopping,
    Failed,
}

/// 对外暴露的 RTMP 发布状态。target_url 永远是脱敏后的地址。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtmpOutputStatus {
    pub state: RtmpOutputState,
    pub session_generation: u64,
    pub target_url: Option<String>,
    pub video_enabled: bool,
    pub audio_enabled: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<u32>,
    pub video_bitrate_kbps: Option<u32>,
    pub audio_bitrate_kbps: Option<u32>,
    /// FFmpeg 最近一次报告的实际输出总码率（kbps）。
    pub current_bitrate_kbps: Option<u32>,
    pub source_identity: Option<RtmpSourceIdentity>,
    /// 当前发布进程实际使用的视觉过滤链：gpu83、cpu4 或 original。
    pub video_filter_backend: Option<String>,
    pub encoder: Option<String>,
    pub process_id: Option<u32>,
    pub retry_count: u32,
    pub published_ms: u64,
    pub output_bytes: u64,
    pub last_progress_ms: Option<u64>,
    pub dropped_audio_chunks: u64,
    pub error_code: Option<String>,
    pub error: Option<String>,
}

impl Default for RtmpOutputStatus {
    fn default() -> Self {
        Self {
            state: RtmpOutputState::Idle,
            session_generation: 0,
            target_url: None,
            video_enabled: false,
            audio_enabled: false,
            width: None,
            height: None,
            fps: None,
            video_bitrate_kbps: None,
            audio_bitrate_kbps: None,
            current_bitrate_kbps: None,
            source_identity: None,
            video_filter_backend: None,
            encoder: None,
            process_id: None,
            retry_count: 0,
            published_ms: 0,
            output_bytes: 0,
            last_progress_ms: None,
            dropped_audio_chunks: 0,
            error_code: None,
            error: None,
        }
    }
}

/// 当前播放源的安全身份快照；不包含本地路径或媒体正文。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtmpSourceIdentity {
    pub playback_generation: u64,
    pub source_media_index: usize,
    pub loop_index: u64,
    pub source_duration_ms: Option<u64>,
    pub source_position_ms: u64,
}

#[derive(Debug)]
struct Session {
    stop: Arc<AtomicBool>,
    join: JoinHandle<()>,
}

#[derive(Debug)]
struct ManagerInner {
    next_session_generation: u64,
    status: RtmpOutputStatus,
    session: Option<Session>,
    audio_sink: Option<RtmpAudioSink>,
}

/// 管理一个唯一的 RTMP 发布进程。
#[derive(Debug)]
pub struct RtmpOutputManager {
    inner: Arc<Mutex<ManagerInner>>,
    /// 只统计对外管理器句柄；工作线程持有的 `inner` Arc 不应阻止 Drop 兜底。
    manager_refs: Arc<AtomicUsize>,
}

impl Clone for RtmpOutputManager {
    fn clone(&self) -> Self {
        self.manager_refs.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: Arc::clone(&self.inner),
            manager_refs: Arc::clone(&self.manager_refs),
        }
    }
}

impl Default for RtmpOutputManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RtmpOutputManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ManagerInner {
                next_session_generation: 0,
                status: RtmpOutputStatus::default(),
                session: None,
                audio_sink: None,
            })),
            manager_refs: Arc::new(AtomicUsize::new(1)),
        }
    }

    pub fn validate_config(&self, config: &RtmpOutputConfig) -> Result<(), RtmpOutputError> {
        config.validate().map_err(RtmpOutputError::InvalidConfig)
    }

    /// 启动一个 FFmpeg 推流会话。音频开启时，返回的 audio_sink() 接收最终 PCM。
    pub fn start(
        &self,
        config: RtmpOutputConfig,
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        optional_filter: Option<String>,
        source_identity: Option<RtmpSourceIdentity>,
    ) -> Result<(), RtmpOutputError> {
        self.start_with_filters(
            config,
            ffmpeg_path,
            source_path,
            optional_filter,
            None,
            source_identity,
        )
    }

    /// 启动一个带 GPU 过滤链和 CPU4 备用过滤链的会话。
    pub fn start_with_filters(
        &self,
        config: RtmpOutputConfig,
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        optional_filter: Option<String>,
        cpu_fallback_filter: Option<String>,
        source_identity: Option<RtmpSourceIdentity>,
    ) -> Result<(), RtmpOutputError> {
        self.start_with_filters_and_audio_sample_rate(
            config,
            ffmpeg_path,
            source_path,
            optional_filter,
            cpu_fallback_filter,
            source_identity,
            DEFAULT_RTMP_AUDIO_SAMPLE_RATE_HZ,
        )
    }

    /// 启动一个会话，并使用最终 PortAudio 总线的实际采样率解释输入 PCM。
    ///
    /// FFmpeg 会把该输入重采样为固定的 48kHz AAC 输出；不能把 44.1kHz
    /// 的 PortAudio 样本错误地标成 48kHz，否则推流声音会变速变调。
    #[allow(clippy::too_many_arguments)]
    pub fn start_with_filters_and_audio_sample_rate(
        &self,
        config: RtmpOutputConfig,
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        optional_filter: Option<String>,
        cpu_fallback_filter: Option<String>,
        source_identity: Option<RtmpSourceIdentity>,
        audio_sample_rate_hz: u32,
    ) -> Result<(), RtmpOutputError> {
        self.validate_config(&config)?;
        validate_filter(optional_filter.as_deref()).map_err(RtmpOutputError::InvalidConfig)?;
        validate_filter(cpu_fallback_filter.as_deref()).map_err(RtmpOutputError::InvalidConfig)?;
        validate_ffmpeg_path(&ffmpeg_path)?;
        if config.video_enabled {
            validate_source_path(&source_path)?;
        }
        let audio_sample_rate_hz = if config.audio_enabled {
            validate_audio_sample_rate(audio_sample_rate_hz)?
        } else {
            DEFAULT_RTMP_AUDIO_SAMPLE_RATE_HZ
        };

        let (stale_session, stop, audio_sink, audio_receiver, session_generation) = {
            let mut inner = self.inner.lock().map_err(|_| RtmpOutputError::Internal)?;
            let stale_session = if inner
                .session
                .as_ref()
                .is_some_and(|session| session.join.is_finished())
            {
                inner.session.take()
            } else {
                None
            };
            if inner.session.is_some() {
                return Err(RtmpOutputError::AlreadyRunning);
            }
            inner.status = RtmpOutputStatus {
                state: RtmpOutputState::Starting,
                session_generation: inner.next_session_generation.saturating_add(1),
                target_url: Some(redact_target_url(&config.target_url)),
                video_enabled: config.video_enabled,
                audio_enabled: config.audio_enabled,
                width: config.video_enabled.then_some(config.width),
                height: config.video_enabled.then_some(config.height),
                fps: config.video_enabled.then_some(config.fps),
                video_bitrate_kbps: config.video_enabled.then_some(config.video_bitrate_kbps),
                audio_bitrate_kbps: config.audio_enabled.then_some(config.audio_bitrate_kbps),
                current_bitrate_kbps: None,
                source_identity,
                video_filter_backend: None,
                encoder: None,
                process_id: None,
                retry_count: 0,
                published_ms: 0,
                output_bytes: 0,
                last_progress_ms: None,
                dropped_audio_chunks: 0,
                error_code: None,
                error: None,
            };
            inner.next_session_generation = inner.status.session_generation;
            let stop = Arc::new(AtomicBool::new(false));
            let (audio_sink, audio_receiver) = if config.audio_enabled {
                let (sink, receiver) = RtmpAudioSink::channel();
                (Some(sink), Some(receiver))
            } else {
                (None, None)
            };
            inner.audio_sink = audio_sink.clone();
            (
                stale_session,
                stop,
                audio_sink,
                audio_receiver,
                inner.status.session_generation,
            )
        };

        if let Some(session) = stale_session {
            let _ = session.join.join();
        }

        // 持有管理器锁完成“检查仍在 Starting → 创建线程 → 安装 session”整个临界区，
        // 避免 stop() 在线程创建后、session 写入前抢先返回而留下无人管理的发布线程。
        let mut inner = self.inner.lock().map_err(|_| RtmpOutputError::Internal)?;
        if inner.status.session_generation != session_generation
            || inner.status.state != RtmpOutputState::Starting
        {
            // stop() 可能在清理旧会话期间已经把这次启动标记为 Idle；此时不再创建新线程。
            return Ok(());
        }
        let manager_inner = Arc::clone(&self.inner);
        let worker_stop = Arc::clone(&stop);
        let worker_sink = audio_sink.clone();
        let worker = match thread::Builder::new()
            .name("autolive-rtmp-output".to_owned())
            .spawn(move || {
                run_session(SessionContext {
                    inner: manager_inner,
                    stop: worker_stop,
                    config,
                    ffmpeg_path,
                    source_path,
                    optional_filter,
                    cpu_fallback_filter,
                    source_identity,
                    audio_receiver,
                    audio_sink: worker_sink,
                    audio_sample_rate_hz,
                });
            }) {
            Ok(worker) => worker,
            Err(_) => {
                inner.audio_sink = None;
                inner.status.state = RtmpOutputState::Failed;
                inner.status.error_code = Some("rtmp_worker_start_failed".to_owned());
                inner.status.error = Some("RTMP 推流线程启动失败".to_owned());
                return Err(RtmpOutputError::ThreadStartFailed);
            }
        };
        inner.session = Some(Session { stop, join: worker });
        Ok(())
    }

    /// 停止会话。重复停止是幂等的。
    pub fn stop(&self) -> Result<(), RtmpOutputError> {
        let session = {
            let mut inner = self.inner.lock().map_err(|_| RtmpOutputError::Internal)?;
            let Some(session) = inner.session.take() else {
                inner.status.state = RtmpOutputState::Idle;
                inner.status.process_id = None;
                inner.audio_sink = None;
                return Ok(());
            };
            inner.status.state = RtmpOutputState::Stopping;
            session.stop.store(true, Ordering::Release);
            if let Some(sink) = inner.audio_sink.as_ref() {
                sink.close();
            }
            session
        };

        if session.join.join().is_err() {
            let mut inner = self.inner.lock().map_err(|_| RtmpOutputError::Internal)?;
            inner.status = RtmpOutputStatus {
                state: RtmpOutputState::Failed,
                error_code: Some("rtmp_worker_failed".to_owned()),
                error: Some("RTMP 推流线程异常退出".to_owned()),
                ..inner.status.clone()
            };
            inner.audio_sink = None;
            return Err(RtmpOutputError::WorkerPanicked);
        }

        let mut inner = self.inner.lock().map_err(|_| RtmpOutputError::Internal)?;
        inner.status.state = RtmpOutputState::Idle;
        inner.status.process_id = None;
        inner.status.encoder = None;
        inner.audio_sink = None;
        Ok(())
    }

    pub fn status(&self) -> RtmpOutputStatus {
        self.inner
            .lock()
            .map(|inner| inner.status.clone())
            .unwrap_or_else(|_| RtmpOutputStatus {
                state: RtmpOutputState::Failed,
                error_code: Some("rtmp_status_unavailable".to_owned()),
                error: Some("RTMP 状态不可用".to_owned()),
                ..RtmpOutputStatus::default()
            })
    }

    pub fn audio_sink(&self) -> Option<RtmpAudioSink> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| inner.audio_sink.clone())
    }
}

impl Drop for RtmpOutputManager {
    fn drop(&mut self) {
        // `inner` 还会被发布线程持有，不能用它的 Arc 引用数判断是否
        // 已经没有外部所有者；最后一个管理器句柄释放时必须主动回收会话。
        if self.manager_refs.fetch_sub(1, Ordering::AcqRel) == 1 {
            let _ = self.stop();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtmpOutputError {
    InvalidConfig(RtmpConfigError),
    AlreadyRunning,
    FfmpegNotFound,
    SourceNotFound,
    InvalidAudioSampleRate,
    ThreadStartFailed,
    WorkerPanicked,
    Internal,
}

impl std::fmt::Display for RtmpOutputError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(error) => error.fmt(formatter),
            Self::AlreadyRunning => formatter.write_str("RTMP 推流已经在运行"),
            Self::FfmpegNotFound => formatter.write_str("FFmpeg 不可用"),
            Self::SourceNotFound => formatter.write_str("视频源不可用"),
            Self::InvalidAudioSampleRate => formatter.write_str("RTMP 音频采样率无效"),
            Self::ThreadStartFailed => formatter.write_str("RTMP 推流线程启动失败"),
            Self::WorkerPanicked => formatter.write_str("RTMP 推流线程异常退出"),
            Self::Internal => formatter.write_str("RTMP 推流状态不可用"),
        }
    }
}

impl std::error::Error for RtmpOutputError {}

fn validate_ffmpeg_path(path: &Path) -> Result<(), RtmpOutputError> {
    if !path.is_file() {
        return Err(RtmpOutputError::FfmpegNotFound);
    }
    Ok(())
}

fn validate_source_path(path: &Path) -> Result<(), RtmpOutputError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        _ => Err(RtmpOutputError::SourceNotFound),
    }
}

fn validate_audio_sample_rate(sample_rate_hz: u32) -> Result<u32, RtmpOutputError> {
    if (8_000..=384_000).contains(&sample_rate_hz) {
        Ok(sample_rate_hz)
    } else {
        Err(RtmpOutputError::InvalidAudioSampleRate)
    }
}

struct SessionContext {
    inner: Arc<Mutex<ManagerInner>>,
    stop: Arc<AtomicBool>,
    config: RtmpOutputConfig,
    ffmpeg_path: PathBuf,
    source_path: PathBuf,
    optional_filter: Option<String>,
    cpu_fallback_filter: Option<String>,
    source_identity: Option<RtmpSourceIdentity>,
    audio_receiver: Option<Receiver<Vec<f32>>>,
    audio_sink: Option<RtmpAudioSink>,
    audio_sample_rate_hz: u32,
}

fn run_session(context: SessionContext) {
    let SessionContext {
        inner,
        stop,
        config,
        ffmpeg_path,
        source_path,
        optional_filter,
        cpu_fallback_filter,
        source_identity,
        audio_receiver,
        audio_sink,
        audio_sample_rate_hz,
    } = context;
    let audio_receiver = audio_receiver.map(|receiver| Arc::new(Mutex::new(receiver)));
    let encoder_order = if config.video_enabled {
        let mut order = match crate::media_engine::h264_encoder_attempt_order_with_cancel(
            &ffmpeg_path,
            &stop,
        ) {
            Ok(order) => order,
            Err(crate::media_engine::MediaEngineError::Cancelled) => {
                set_idle(&inner);
                return;
            }
            Err(_) => Vec::new(),
        };
        if order.is_empty() {
            order.push("libopenh264".to_owned());
        }
        order
    } else {
        vec![String::new()]
    };

    let mut retry_count = 0;
    // GPU83 是静态启动快照：先尝试带 Vulkan/libplacebo 的过滤链，
    // 若该链因驱动/设备状态失败，再在同一会话内退回 CPU4 或中性视频链，
    // 避免把本地播放和 RTMP 会话绑定到 GPU 故障上。
    let filter_attempts = build_filter_attempts(
        config.video_enabled,
        optional_filter.as_deref(),
        cpu_fallback_filter.as_deref(),
    );
    loop {
        if stop.load(Ordering::Acquire) {
            set_idle(&inner);
            break;
        }

        let mut process_started = false;
        let mut deterministic_failure = None;
        let mut spawn_failure = false;
        'filter_attempts: for (filter, uses_vulkan_filter, filter_backend) in &filter_attempts {
            for encoder in &encoder_order {
                if stop.load(Ordering::Acquire) {
                    set_idle(&inner);
                    return;
                }
                set_starting_encoder(
                    &inner,
                    if encoder.is_empty() {
                        None
                    } else {
                        Some(encoder)
                    },
                    config.video_enabled,
                    filter_backend,
                );
                let mut command = build_ffmpeg_command(
                    &config,
                    &ffmpeg_path,
                    &source_path,
                    *filter,
                    *uses_vulkan_filter,
                    source_identity,
                    encoder,
                    audio_sample_rate_hz,
                );
                let mut child = match command.spawn() {
                    Ok(child) => child,
                    Err(_) => {
                        spawn_failure = true;
                        continue;
                    }
                };
                let job = match ManagedRtmpJob::create()
                    .and_then(|job| job.assign_process(&child).map(|()| job))
                {
                    Ok(job) => job,
                    Err(_) => {
                        spawn_failure = true;
                        let _ = child.kill();
                        let _ = child.wait();
                        continue;
                    }
                };
                process_started = true;
                match run_process(
                    &inner,
                    &stop,
                    &config,
                    encoder,
                    &mut child,
                    audio_receiver.as_ref(),
                    audio_sample_rate_hz,
                    job,
                ) {
                    AttemptResult::Stopped => {
                        set_idle(&inner);
                        return;
                    }
                    AttemptResult::Exited { status, failure: _ } if status.success() => {
                        // 后续过滤链/编码器已成功运行时，前一候选路径的确定性
                        // 失败不应污染本次会话结果。
                        deterministic_failure = None;
                        break 'filter_attempts;
                    }
                    AttemptResult::Exited {
                        failure: Some(ProcessFailure::Deterministic(message)),
                        ..
                    } => {
                        deterministic_failure.get_or_insert(message);
                        continue;
                    }
                    AttemptResult::Exited { .. } | AttemptResult::SpawnFailed => continue,
                }
            }
        }

        if stop.load(Ordering::Acquire) {
            set_idle(&inner);
            break;
        }
        if let Some(message) = deterministic_failure {
            set_failed(&inner, deterministic_error_code(message), message);
            if let Some(sink) = audio_sink.as_ref() {
                sink.close();
            }
            break;
        }
        if !process_started && spawn_failure {
            set_failed(&inner, "rtmp_process_start_failed", "RTMP 推流进程启动失败");
            if let Some(sink) = audio_sink.as_ref() {
                sink.close();
            }
            break;
        }
        if retry_count >= MAX_RETRIES {
            let (error_code, message) = if process_started {
                ("rtmp_retries_exhausted", "RTMP 推流进程反复退出")
            } else {
                ("rtmp_process_start_failed", "RTMP 推流进程启动失败")
            };
            set_failed(&inner, error_code, message);
            if let Some(sink) = audio_sink.as_ref() {
                sink.close();
            }
            break;
        }

        retry_count += 1;
        set_reconnecting(&inner, retry_count);
        let delay = Duration::from_secs(RETRY_BACKOFF_SECONDS[(retry_count - 1) as usize]);
        let deadline = Instant::now() + delay;
        while Instant::now() < deadline {
            if stop.load(Ordering::Acquire) {
                set_idle(&inner);
                return;
            }
            thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
        }

        // “正常结束”表示源文件到达 EOF；重试时仍从首选编码器开始，
        // 编码器异常退出则在本轮继续向 CPU 回退。
    }
}

fn build_filter_attempts<'a>(
    video_enabled: bool,
    optional_filter: Option<&'a str>,
    cpu_fallback_filter: Option<&'a str>,
) -> Vec<(Option<&'a str>, bool, &'static str)> {
    if !video_enabled {
        // 纯音频会话不能因为底层调用方误传视频快照而初始化 Vulkan、
        // 运行滤镜或暴露虚假的视觉链状态。
        return vec![(None, false, "original")];
    }
    match (optional_filter, cpu_fallback_filter) {
        (Some(gpu), Some(cpu)) => vec![(Some(gpu), true, "gpu83"), (Some(cpu), false, "cpu4")],
        (Some(gpu), None) => vec![(Some(gpu), true, "gpu83"), (None, false, "original")],
        (None, Some(cpu)) => vec![(Some(cpu), false, "cpu4")],
        (None, None) => vec![(None, false, "original")],
    }
}

enum AttemptResult {
    Stopped,
    Exited {
        status: ExitStatus,
        failure: Option<ProcessFailure>,
    },
    SpawnFailed,
}

#[allow(clippy::too_many_arguments)]
fn run_process(
    inner: &Arc<Mutex<ManagerInner>>,
    stop: &Arc<AtomicBool>,
    config: &RtmpOutputConfig,
    encoder: &str,
    child: &mut Child,
    audio_receiver: Option<&Arc<Mutex<Receiver<Vec<f32>>>>>,
    audio_sample_rate_hz: u32,
    _job: ManagedRtmpJob,
) -> AttemptResult {
    let process_id = child.id();
    set_process_started(inner, encoder, process_id);
    let started_at = Instant::now();
    let had_audio_overrun = inner
        .lock()
        .ok()
        .and_then(|inner| inner.audio_sink.as_ref().map(RtmpAudioSink::take_overrun))
        .unwrap_or(false);
    if had_audio_overrun {
        drain_audio_receiver(audio_receiver);
    }
    let diagnostics = Arc::new(Mutex::new(ProcessDiagnostics::default()));

    let writer = if config.audio_enabled {
        child.stdin.take().and_then(|mut stdin| {
            let receiver = audio_receiver.cloned();
            let stop = Arc::clone(stop);
            thread::Builder::new()
                .name("autolive-rtmp-audio-pipe".to_owned())
                .spawn(move || {
                    let Some(receiver) = receiver else {
                        return;
                    };
                    let silence =
                        pcm_f32le_bytes(&vec![
                            0.0;
                            silence_audio_samples_per_chunk(audio_sample_rate_hz)
                        ]);
                    loop {
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        let chunk = match receiver.lock() {
                            Ok(receiver) => receiver.recv_timeout(AUDIO_RECEIVE_TIMEOUT),
                            Err(_) => return,
                        };
                        match chunk {
                            Ok(samples) => {
                                if stdin.write_all(&pcm_f32le_bytes(&samples)).is_err() {
                                    break;
                                }
                            }
                            Err(RecvTimeoutError::Timeout) => {
                                // 当前媒体无音轨、PortAudio 尚未产出首段 PCM 或短暂
                                // 断流时维持 AAC 时间轴；真实最终 PCM 到达后立即接管。
                                if stdin.write_all(&silence).is_err() {
                                    break;
                                }
                            }
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                    }
                })
                .ok()
        })
    } else {
        None
    };
    let stderr_reader = child.stderr.take().and_then(|stderr| {
        let inner = Arc::clone(inner);
        let diagnostics = Arc::clone(&diagnostics);
        thread::Builder::new()
            .name("autolive-rtmp-stderr".to_owned())
            .spawn(move || {
                let mut reader = BufReader::new(stderr);
                let mut line = Vec::with_capacity(MAX_STDERR_LINE_BYTES);
                let mut diagnostic_bytes = 0usize;
                while let Ok(Some((line_bytes, truncated))) =
                    read_bounded_stderr_line(&mut reader, &mut line)
                {
                    // 进度协议是长期运行所需的心跳，不能因为错误尾缓冲达到上限
                    // 而停止解析；它只读取固定的数值字段，不保留原文。错误分类
                    // 只对非进度行累计固定预算，避免长稳会话的心跳耗尽诊断空间，
                    // 且所有超长行只排空不解析。
                    if !truncated {
                        let line = String::from_utf8_lossy(&line);
                        observe_progress_line(&inner, &line);
                        if !is_progress_line(&line) && diagnostic_bytes < MAX_STDERR_BYTES {
                            diagnostic_bytes = diagnostic_bytes.saturating_add(line_bytes);
                            if let Some(failure) = classify_stderr_line(&line) {
                                if let Ok(mut diagnostics) = diagnostics.lock() {
                                    diagnostics.record(failure);
                                }
                            }
                        }
                    }
                }
            })
            .ok()
    });

    loop {
        if stop.load(Ordering::Acquire) {
            terminate_child(child);
            join_thread(writer);
            join_thread(stderr_reader);
            clear_process(inner);
            return AttemptResult::Stopped;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                join_thread(writer);
                join_thread(stderr_reader);
                clear_process(inner);
                return AttemptResult::Exited {
                    status,
                    failure: diagnostics
                        .lock()
                        .ok()
                        .and_then(|diagnostics| diagnostics.failure()),
                };
            }
            Ok(None) => {
                if process_timed_out(inner, started_at) {
                    let status = terminate_child(child);
                    join_thread(writer);
                    join_thread(stderr_reader);
                    clear_process(inner);
                    return status.map_or(AttemptResult::SpawnFailed, |status| {
                        AttemptResult::Exited {
                            status,
                            failure: diagnostics
                                .lock()
                                .ok()
                                .and_then(|diagnostics| diagnostics.failure()),
                        }
                    });
                }
                thread::sleep(POLL_INTERVAL);
            }
            Err(_) => {
                terminate_child(child);
                join_thread(writer);
                join_thread(stderr_reader);
                clear_process(inner);
                return AttemptResult::SpawnFailed;
            }
        }
    }
}

fn terminate_child(child: &mut Child) -> Option<ExitStatus> {
    let _ = child.kill();
    child.wait().ok()
}

fn drain_audio_receiver(audio_receiver: Option<&Arc<Mutex<Receiver<Vec<f32>>>>>) {
    let Some(receiver) = audio_receiver else {
        return;
    };
    let Ok(receiver) = receiver.lock() else {
        return;
    };
    for _ in 0..crate::rtmp_output::audio_sink::AUDIO_QUEUE_CAPACITY {
        if receiver.try_recv().is_err() {
            break;
        }
    }
}

fn silence_audio_samples_per_chunk(sample_rate_hz: u32) -> usize {
    usize::try_from(
        u64::from(sample_rate_hz)
            .saturating_mul(50)
            .saturating_div(1_000)
            .saturating_mul(2),
    )
    .unwrap_or(4_800)
    .max(2)
}

fn process_timed_out(inner: &Arc<Mutex<ManagerInner>>, started_at: Instant) -> bool {
    let Ok(mut inner) = inner.lock() else {
        return true;
    };
    let dropped_audio_chunks = inner.audio_sink.as_ref().map(RtmpAudioSink::dropped_chunks);
    if let Some(dropped_audio_chunks) = dropped_audio_chunks {
        inner.status.dropped_audio_chunks = dropped_audio_chunks;
    }
    if inner
        .audio_sink
        .as_ref()
        .is_some_and(RtmpAudioSink::has_overrun)
    {
        return true;
    }
    match inner.status.state {
        RtmpOutputState::Starting => started_at.elapsed() >= STARTUP_TIMEOUT,
        RtmpOutputState::Publishing => inner
            .status
            .last_progress_ms
            .and_then(|last| unix_now_ms().checked_sub(last))
            .is_some_and(|elapsed_ms| elapsed_ms >= NO_PROGRESS_TIMEOUT.as_millis() as u64),
        _ => false,
    }
}

/// 将 FFmpeg 的有限 stderr 线索映射到稳定、脱敏的错误合同。
///
/// 原始 stderr 不能进入状态或日志（其中可能包含发布地址和 token）。没有明确
/// 线索时保持未知，让会话走有界重试，而不是凭模糊文案误判为永久失败。
fn classify_stderr_line(line: &str) -> Option<ProcessFailure> {
    let line = line.to_ascii_lowercase();
    if [
        "connection refused",
        "connection timed out",
        "timed out",
        "network is unreachable",
        "temporary failure in name resolution",
        "name or service not known",
        "broken pipe",
        "input/output error",
        "i/o error",
        "end of file",
    ]
    .iter()
    .any(|needle| line.contains(needle))
    {
        return Some(ProcessFailure::Retryable);
    }

    if [
        "unauthorized",
        "forbidden",
        "authentication failed",
        "auth failed",
        "publish denied",
        "not authorized",
        "access denied",
    ]
    .iter()
    .any(|needle| line.contains(needle))
    {
        return Some(ProcessFailure::Deterministic(
            "RTMP 服务器拒绝发布，请检查地址或鉴权",
        ));
    }

    if line.contains("unknown encoder")
        || (line.contains("encoder") && line.contains("not found"))
        || line.contains("error initializing output stream")
    {
        return Some(ProcessFailure::Deterministic(
            "当前 FFmpeg 视频编码器不可用",
        ));
    }

    if line.contains("no such filter")
        || line.contains("error reinitializing filters")
        || line.contains("failed to configure filter")
    {
        return Some(ProcessFailure::Deterministic("视频滤镜初始化失败"));
    }

    if line.contains("invalid argument")
        || line.contains("option not found")
        || line.contains("no such file or directory")
        || line.contains("error opening input")
    {
        return Some(ProcessFailure::Deterministic("源媒体或 FFmpeg 参数不可用"));
    }

    None
}

fn join_thread(thread: Option<JoinHandle<()>>) {
    if let Some(thread) = thread {
        let _ = thread.join();
    }
}

/// 读取一行 stderr，但把内存占用限制在 `MAX_STDERR_LINE_BYTES` 以内。
///
/// `BufRead::read_line` 会先为超长行增长整个 String；FFmpeg 的错误输出
/// 不应因此成为无界内存入口。这里消费完整行以解除管道回压，只保留前缀
/// 并返回是否截断。
fn read_bounded_stderr_line<R: BufRead>(
    reader: &mut R,
    line: &mut Vec<u8>,
) -> io::Result<Option<(usize, bool)>> {
    line.clear();
    let mut consumed = 0usize;
    let mut truncated = false;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return if consumed == 0 {
                Ok(None)
            } else {
                Ok(Some((consumed, truncated)))
            };
        }

        let take_len = chunk
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(chunk.len(), |position| position + 1);
        let remaining = MAX_STDERR_LINE_BYTES.saturating_sub(line.len());
        let copy_len = take_len.min(remaining);
        if copy_len > 0 {
            line.extend_from_slice(&chunk[..copy_len]);
        }
        if copy_len < take_len {
            truncated = true;
        }
        let has_newline = chunk[..take_len].contains(&b'\n');
        reader.consume(take_len);
        consumed = consumed.saturating_add(take_len);

        if has_newline {
            return Ok(Some((consumed, truncated)));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_ffmpeg_command(
    config: &RtmpOutputConfig,
    ffmpeg_path: &Path,
    source_path: &Path,
    optional_filter: Option<&str>,
    uses_vulkan_filter: bool,
    source_identity: Option<RtmpSourceIdentity>,
    encoder: &str,
    audio_sample_rate_hz: u32,
) -> Command {
    let mut command = background_command(ffmpeg_path);
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-progress")
        .arg("pipe:2");
    // 源文件通常已经带有可用 PTS；不要在 `-stream_loop` 场景重新生成时间戳，
    // 否则循环边界可能把 ZLMediaKit 看到的帧率变成 0/60。缺失 PTS 时由
    // FFmpeg 的输入解复用器和下方固定 CFR 输出策略处理。
    if uses_vulkan_filter {
        // build_gpu83_video_filter 使用 hwupload/libplacebo/hwdownload；
        // 显式绑定 Vulkan 设备，避免依赖 FFmpeg 的隐式设备选择。
        command
            .arg("-init_hw_device")
            .arg("vulkan=autolive_gpu:0")
            .arg("-filter_hw_device")
            .arg("autolive_gpu");
    }
    let mut input_index = 0;
    if config.video_enabled {
        // 源媒体在当前播放会话内循环；换源/停止由上层显式回收并重建进程。
        command.arg("-stream_loop").arg("-1");
        if let Some(source_position_ms) = source_identity
            .map(|identity| identity.source_position_ms)
            .filter(|source_position_ms| *source_position_ms > 0)
        {
            command
                .arg("-ss")
                .arg(format!("{:.3}", source_position_ms as f64 / 1_000.0));
        }
        command.arg("-re").arg("-i").arg(source_path);
        input_index = 1;
    }
    if config.audio_enabled {
        command
            .arg("-f")
            .arg("f32le")
            .arg("-ar")
            .arg(audio_sample_rate_hz.to_string())
            .arg("-ac")
            .arg("2")
            .arg("-i")
            .arg("pipe:0");
    }

    if config.video_enabled {
        command
            .arg("-map")
            .arg("0:v:0")
            .args(crate::media_engine::video_encoder_codec_args(encoder))
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-r")
            .arg(config.fps.to_string())
            // ZLMediaKit/FLV 需要稳定的 CFR 时间戳；源文件循环或 GPU
            // 滤镜可能产生 VFR/重复时间戳，显式 CFR 让编码器在固定输出档位
            // 内补帧/丢帧，而不是把 0/60 FPS 的轨道合同交给服务器。
            .arg("-fps_mode")
            .arg("cfr")
            .arg("-s")
            .arg(format!("{}x{}", config.width, config.height))
            .arg("-b:v")
            .arg(format!("{}k", config.video_bitrate_kbps))
            .arg("-maxrate")
            .arg(format!("{}k", config.video_bitrate_kbps))
            .arg("-bufsize")
            .arg(format!("{}k", config.video_bitrate_kbps.saturating_mul(2)))
            .arg("-g")
            .arg(config.fps.saturating_mul(2).to_string())
            .arg("-bf")
            .arg("0");
        let scale = format!(
            "scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2",
            config.width, config.height, config.width, config.height
        );
        let filter = optional_filter
            .map(|filter| format!("{filter},{scale}"))
            .unwrap_or(scale);
        command.arg("-vf").arg(filter);
    }
    if config.audio_enabled {
        command
            .arg("-map")
            .arg(format!("{}:a:0", input_index))
            .arg("-c:a")
            .arg("aac")
            .arg("-b:a")
            .arg(format!("{}k", config.audio_bitrate_kbps))
            .arg("-ar")
            .arg(RTMP_AUDIO_OUTPUT_SAMPLE_RATE_HZ.to_string())
            .arg("-ac")
            .arg("2");
    }
    command
        .arg("-f")
        .arg("flv")
        .arg("-flvflags")
        .arg("no_duration_filesize")
        .arg(&config.target_url)
        .stdin(if config.audio_enabled {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

fn set_starting_encoder(
    inner: &Arc<Mutex<ManagerInner>>,
    encoder: Option<&String>,
    video_enabled: bool,
    filter_backend: &str,
) {
    if let Ok(mut inner) = inner.lock() {
        inner.status.state = RtmpOutputState::Starting;
        inner.status.encoder = encoder.cloned();
        inner.status.video_filter_backend = video_enabled.then(|| filter_backend.to_owned());
        inner.status.process_id = None;
        inner.status.last_progress_ms = None;
    }
}

fn set_process_started(inner: &Arc<Mutex<ManagerInner>>, encoder: &str, process_id: u32) {
    if let Ok(mut inner) = inner.lock() {
        inner.status.state = RtmpOutputState::Starting;
        inner.status.encoder = (!encoder.is_empty()).then(|| encoder.to_owned());
        inner.status.process_id = Some(process_id);
    }
}

fn is_progress_line(line: &str) -> bool {
    let Some((key, _)) = line.trim().split_once('=') else {
        return false;
    };
    matches!(
        key,
        "frame"
            | "fps"
            | "bitrate"
            | "total_size"
            | "out_time_us"
            | "out_time_ms"
            | "dup_frames"
            | "drop_frames"
            | "speed"
            | "progress"
    ) || (key.starts_with("stream_") && key.ends_with("_q"))
}

fn observe_progress_line(inner: &Arc<Mutex<ManagerInner>>, line: &str) {
    let mut out_time_ms = None;
    let mut total_size = None;
    let mut bitrate_kbps = None;
    for (key, value) in line.trim().split_once('=').into_iter() {
        match key {
            "out_time_ms" => out_time_ms = value.parse::<u64>().ok(),
            "total_size" => total_size = value.parse::<u64>().ok(),
            "bitrate" => bitrate_kbps = parse_progress_bitrate_kbps(value),
            "progress" if value == "continue" => {}
            _ => {}
        }
    }
    let output_advanced =
        out_time_ms.is_some_and(|value| value > 0) || total_size.is_some_and(|value| value > 0);
    if !output_advanced && bitrate_kbps.is_none() {
        return;
    }
    if let Ok(mut inner) = inner.lock() {
        if bitrate_kbps.is_some() {
            inner.status.current_bitrate_kbps = bitrate_kbps;
        }
        if output_advanced {
            inner.status.state = RtmpOutputState::Publishing;
            if let Some(out_time_ms) = out_time_ms {
                inner.status.published_ms = inner.status.published_ms.max(out_time_ms / 1_000);
            }
            if let Some(total_size) = total_size {
                inner.status.output_bytes = inner.status.output_bytes.max(total_size);
            }
            inner.status.last_progress_ms = Some(unix_now_ms());
        }
    }
}

fn parse_progress_bitrate_kbps(value: &str) -> Option<u32> {
    let numeric = value
        .trim()
        .strip_suffix("kbits/s")
        .or_else(|| value.trim().strip_suffix("kbit/s"))?
        .trim()
        .parse::<f64>()
        .ok()?;
    if !numeric.is_finite() || numeric < 0.0 {
        return None;
    }
    Some(numeric.round().min(f64::from(u32::MAX)) as u32)
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

fn clear_process(inner: &Arc<Mutex<ManagerInner>>) {
    if let Ok(mut inner) = inner.lock() {
        inner.status.process_id = None;
        inner.status.last_progress_ms = None;
    }
}

fn set_reconnecting(inner: &Arc<Mutex<ManagerInner>>, retry_count: u32) {
    if let Ok(mut inner) = inner.lock() {
        inner.status.state = RtmpOutputState::Reconnecting;
        inner.status.retry_count = retry_count;
        inner.status.process_id = None;
        inner.status.last_progress_ms = None;
    }
}

fn set_idle(inner: &Arc<Mutex<ManagerInner>>) {
    if let Ok(mut inner) = inner.lock() {
        inner.status.state = RtmpOutputState::Idle;
        inner.status.process_id = None;
        inner.status.video_filter_backend = None;
        inner.status.encoder = None;
    }
}

fn deterministic_error_code(message: &str) -> &'static str {
    if message.contains("鉴权") {
        "rtmp_publish_rejected"
    } else if message.contains("编码器") {
        "rtmp_encoder_unavailable"
    } else if message.contains("滤镜") {
        "rtmp_filter_unavailable"
    } else {
        "rtmp_invalid_media_or_arguments"
    }
}

fn set_failed(inner: &Arc<Mutex<ManagerInner>>, error_code: &str, message: &str) {
    if let Ok(mut inner) = inner.lock() {
        inner.status.state = RtmpOutputState::Failed;
        inner.status.process_id = None;
        inner.status.error_code = Some(error_code.to_owned());
        inner.status.error = Some(message.to_owned());
    }
}

fn redact_target_url(url: &str) -> String {
    let Some((scheme, remainder)) = url.split_once("://") else {
        return "<redacted>".to_owned();
    };
    let authority_end = remainder.find('/').unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    format!("{}://{}/<redacted>", scheme, host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtmp_output::{RtmpAudioSinkError, RtmpOutputConfig};

    #[test]
    fn redacts_stream_key_and_keeps_host() {
        assert_eq!(
            redact_target_url("rtmp://example.com:1935/live/private-key"),
            "rtmp://example.com:1935/<redacted>"
        );
        assert_eq!(
            redact_target_url("rtmps://example.com/live/private-key?token=secret"),
            "rtmps://example.com/<redacted>"
        );
    }

    #[test]
    fn builds_audio_video_pipe_command_without_shell_interpolation() {
        let config = RtmpOutputConfig {
            target_url: "rtmp://127.0.0.1/live/test".to_owned(),
            ..Default::default()
        };
        let command = build_ffmpeg_command(
            &config,
            Path::new("ffmpeg.exe"),
            Path::new("input.mp4"),
            Some("eq=brightness=0"),
            true,
            Some(RtmpSourceIdentity {
                playback_generation: 4,
                source_media_index: 2,
                loop_index: 1,
                source_duration_ms: Some(60_000),
                source_position_ms: 12_345,
            }),
            "libopenh264",
            DEFAULT_RTMP_AUDIO_SAMPLE_RATE_HZ,
        );
        let args = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.windows(2).any(|pair| pair == ["-i", "pipe:0"]));
        assert!(args.iter().any(|argument| argument == "-c:v"));
        assert!(args.iter().any(|argument| argument == "flv"));
        assert!(args.iter().any(|argument| argument == "input.mp4"));
        assert!(args.windows(2).any(|pair| pair == ["-ss", "12.345"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-init_hw_device", "vulkan=autolive_gpu:0"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-filter_hw_device", "autolive_gpu"]));
        assert!(args.windows(2).any(|pair| pair == ["-fps_mode", "cfr"]));
    }

    #[test]
    fn zero_source_position_does_not_add_redundant_seek() {
        let command = build_ffmpeg_command(
            &RtmpOutputConfig {
                target_url: "rtmp://127.0.0.1/live/video".to_owned(),
                audio_enabled: false,
                ..Default::default()
            },
            Path::new("ffmpeg.exe"),
            Path::new("input.mp4"),
            None,
            false,
            Some(RtmpSourceIdentity {
                playback_generation: 1,
                source_media_index: 0,
                loop_index: 0,
                source_duration_ms: Some(8_000),
                source_position_ms: 0,
            }),
            "libopenh264",
            DEFAULT_RTMP_AUDIO_SAMPLE_RATE_HZ,
        );
        let args = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(!args.iter().any(|argument| argument == "-ss"));
    }

    #[test]
    fn rtmp_command_uses_encoder_specific_options() {
        let command = build_ffmpeg_command(
            &RtmpOutputConfig {
                target_url: "rtmp://127.0.0.1/live/video".to_owned(),
                audio_enabled: false,
                ..Default::default()
            },
            Path::new("ffmpeg.exe"),
            Path::new("input.mp4"),
            None,
            false,
            None,
            "h264_amf",
            DEFAULT_RTMP_AUDIO_SAMPLE_RATE_HZ,
        );
        let args = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.windows(2).any(|pair| pair == ["-c:v", "h264_amf"]));
        assert!(args.windows(2).any(|pair| pair == ["-quality", "speed"]));
    }

    #[test]
    fn video_only_command_does_not_create_an_audio_track() {
        let config = RtmpOutputConfig {
            target_url: "rtmp://127.0.0.1/live/video".to_owned(),
            audio_enabled: false,
            ..Default::default()
        };
        let command = build_ffmpeg_command(
            &config,
            Path::new("ffmpeg.exe"),
            Path::new("input.mp4"),
            None,
            false,
            None,
            "libopenh264",
            DEFAULT_RTMP_AUDIO_SAMPLE_RATE_HZ,
        );
        let args = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.windows(2).any(|pair| pair == ["-map", "0:v:0"]));
        assert!(!args.iter().any(|argument| argument == "pipe:0"));
        assert!(!args.iter().any(|argument| argument == "-c:a"));
    }

    #[test]
    fn audio_only_command_has_no_video_input_or_encoder() {
        let config = RtmpOutputConfig {
            target_url: "rtmp://127.0.0.1/live/audio".to_owned(),
            video_enabled: false,
            ..Default::default()
        };
        let command = build_ffmpeg_command(
            &config,
            Path::new("ffmpeg.exe"),
            Path::new("unused.mp4"),
            None,
            false,
            None,
            "",
            DEFAULT_RTMP_AUDIO_SAMPLE_RATE_HZ,
        );
        let args = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.windows(2).any(|pair| pair == ["-i", "pipe:0"]));
        assert!(args.windows(2).any(|pair| pair == ["-map", "0:a:0"]));
        assert!(!args.iter().any(|argument| argument == "-c:v"));
        assert!(!args.iter().any(|argument| argument == "unused.mp4"));
    }

    #[test]
    fn audio_input_uses_actual_rate_and_output_stays_at_48khz() {
        let config = RtmpOutputConfig {
            target_url: "rtmp://127.0.0.1/live/audio".to_owned(),
            video_enabled: false,
            ..Default::default()
        };
        let command = build_ffmpeg_command(
            &config,
            Path::new("ffmpeg.exe"),
            Path::new("unused.mp4"),
            None,
            false,
            None,
            "",
            44_100,
        );
        let args = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.windows(2).any(|pair| pair == ["-ar", "44100"]));
        assert!(args.windows(2).any(|pair| pair == ["-ar", "48000"]));
    }

    #[test]
    fn silence_chunk_matches_actual_input_sample_rate() {
        assert_eq!(silence_audio_samples_per_chunk(44_100), 4_410);
        assert_eq!(silence_audio_samples_per_chunk(48_000), 4_800);
    }

    #[test]
    fn invalid_audio_sample_rate_is_rejected() {
        assert_eq!(
            validate_audio_sample_rate(7_999),
            Err(RtmpOutputError::InvalidAudioSampleRate)
        );
        assert_eq!(validate_audio_sample_rate(8_000), Ok(8_000));
        assert_eq!(validate_audio_sample_rate(384_000), Ok(384_000));
        assert_eq!(
            validate_audio_sample_rate(384_001),
            Err(RtmpOutputError::InvalidAudioSampleRate)
        );
    }

    #[test]
    fn audio_only_filter_attempt_ignores_video_filters() {
        assert_eq!(
            build_filter_attempts(false, Some("gpu83-filter"), Some("cpu4-filter")),
            vec![(None, false, "original")]
        );
    }

    #[test]
    fn progress_lines_publish_only_after_output_advances() {
        let inner = Arc::new(Mutex::new(ManagerInner {
            next_session_generation: 0,
            status: RtmpOutputStatus::default(),
            session: None,
            audio_sink: None,
        }));
        observe_progress_line(&inner, "progress=continue\n");
        assert_eq!(
            inner.lock().expect("status lock").status.state,
            RtmpOutputState::Idle
        );

        observe_progress_line(&inner, "out_time_ms=123000\n");
        observe_progress_line(&inner, "total_size=4096\n");
        observe_progress_line(&inner, "bitrate=256.5kbits/s\n");
        let status = inner.lock().expect("status lock").status.clone();
        assert_eq!(status.state, RtmpOutputState::Publishing);
        assert_eq!(status.published_ms, 123);
        assert_eq!(status.output_bytes, 4096);
        assert_eq!(status.current_bitrate_kbps, Some(257));
        assert!(status.last_progress_ms.is_some());
    }

    #[test]
    fn parses_ffmpeg_progress_bitrate_without_accepting_invalid_values() {
        assert_eq!(parse_progress_bitrate_kbps("256.5kbits/s"), Some(257));
        assert_eq!(parse_progress_bitrate_kbps("128kbit/s"), Some(128));
        assert_eq!(parse_progress_bitrate_kbps("N/A"), None);
        assert_eq!(parse_progress_bitrate_kbps("-1kbits/s"), None);
    }

    #[test]
    fn progress_lines_do_not_consume_diagnostic_budget() {
        assert!(is_progress_line("out_time_ms=123000\n"));
        assert!(is_progress_line("stream_0_0_q=-1.0\n"));
        assert!(is_progress_line("progress=continue\n"));
        assert!(!is_progress_line("[rtmp] Server returned 403 Forbidden\n"));
    }

    #[test]
    fn filter_backend_status_is_observable_and_cleared_on_idle() {
        let inner = Arc::new(Mutex::new(ManagerInner {
            next_session_generation: 1,
            status: RtmpOutputStatus::default(),
            session: None,
            audio_sink: None,
        }));
        let encoder = "libopenh264".to_owned();
        set_starting_encoder(&inner, Some(&encoder), true, "cpu4");
        assert_eq!(
            inner
                .lock()
                .expect("status lock")
                .status
                .video_filter_backend
                .as_deref(),
            Some("cpu4")
        );
        set_starting_encoder(&inner, Some(&encoder), true, "original");
        assert_eq!(
            inner
                .lock()
                .expect("status lock")
                .status
                .video_filter_backend
                .as_deref(),
            Some("original")
        );
        set_starting_encoder(&inner, None, false, "original");
        assert!(inner
            .lock()
            .expect("status lock")
            .status
            .video_filter_backend
            .is_none());
        set_idle(&inner);
        assert!(inner
            .lock()
            .expect("status lock")
            .status
            .video_filter_backend
            .is_none());
    }

    #[test]
    fn last_manager_handle_stops_a_worker_even_when_worker_holds_inner_arc() {
        let manager = RtmpOutputManager::new();
        let stop = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker_exited = Arc::clone(&exited);
        let join = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                thread::yield_now();
            }
            worker_exited.store(true, Ordering::Release);
        });
        manager.inner.lock().expect("manager lock").session = Some(Session { stop, join });

        drop(manager);

        assert!(exited.load(Ordering::Acquire));
    }

    #[test]
    fn classifies_deterministic_ffmpeg_errors_without_exposing_stderr() {
        assert_eq!(
            classify_stderr_line("[rtmp] Server returned 403 Forbidden for publish key"),
            Some(ProcessFailure::Deterministic(
                "RTMP 服务器拒绝发布，请检查地址或鉴权"
            ))
        );
        assert_eq!(
            classify_stderr_line("Unknown encoder 'h264_nvenc'"),
            Some(ProcessFailure::Deterministic(
                "当前 FFmpeg 视频编码器不可用"
            ))
        );
        assert_eq!(
            classify_stderr_line("No such filter: 'gpu83'"),
            Some(ProcessFailure::Deterministic("视频滤镜初始化失败"))
        );
        assert_eq!(
            deterministic_error_code("RTMP 服务器拒绝发布，请检查地址或鉴权"),
            "rtmp_publish_rejected"
        );
        assert_eq!(
            deterministic_error_code("当前 FFmpeg 视频编码器不可用"),
            "rtmp_encoder_unavailable"
        );
    }

    #[test]
    fn classifies_network_failures_as_retryable() {
        assert_eq!(
            classify_stderr_line("Connection refused by remote host"),
            Some(ProcessFailure::Retryable)
        );
        assert_eq!(
            classify_stderr_line("TLS handshake timed out"),
            Some(ProcessFailure::Retryable)
        );
        assert_eq!(classify_stderr_line("Conversion failed!"), None);
    }

    #[test]
    fn stderr_reader_consumes_oversized_lines_without_unbounded_buffer() {
        let oversized = format!("{}\nnext=ok\n", "x".repeat(MAX_STDERR_LINE_BYTES * 4));
        let mut reader = BufReader::new(std::io::Cursor::new(oversized));
        let mut line = Vec::new();
        let first = read_bounded_stderr_line(&mut reader, &mut line)
            .expect("bounded stderr read")
            .expect("first line");
        assert!(first.1);
        assert_eq!(line.len(), MAX_STDERR_LINE_BYTES);
        let second = read_bounded_stderr_line(&mut reader, &mut line)
            .expect("bounded stderr read")
            .expect("second line");
        assert!(!second.1);
        assert_eq!(String::from_utf8_lossy(&line), "next=ok\n");
    }

    #[test]
    fn audio_queue_overrun_requests_an_immediate_process_restart() {
        let (sink, _receiver) = RtmpAudioSink::channel();
        for _ in 0..crate::rtmp_output::audio_sink::AUDIO_QUEUE_CAPACITY {
            sink.try_push(&[0.0, 0.0])
                .expect("queue should accept capacity");
        }
        assert_eq!(
            sink.try_push(&[0.0, 0.0]),
            Err(RtmpAudioSinkError::QueueFull)
        );
        let inner = Arc::new(Mutex::new(ManagerInner {
            next_session_generation: 1,
            status: RtmpOutputStatus {
                state: RtmpOutputState::Publishing,
                ..RtmpOutputStatus::default()
            },
            session: None,
            audio_sink: Some(sink),
        }));
        assert!(process_timed_out(&inner, Instant::now()));
        assert_eq!(
            inner
                .lock()
                .expect("status lock")
                .status
                .dropped_audio_chunks,
            1
        );
        assert!(inner
            .lock()
            .expect("status lock")
            .audio_sink
            .as_ref()
            .is_some_and(RtmpAudioSink::has_overrun));
    }
}
