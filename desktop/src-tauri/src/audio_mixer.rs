//! PortAudio 的本地 PCM 数据路径。
//!
//! FFmpeg 解码线程只负责把当前音轨转换为交错 f32；有界 channel 将解码和
//! 混音/输出解耦，混音线程执行有限值清理和 true-peak 保护，最后写入
//! PortAudio crate 提供的 SPSC 环形缓冲。WebView 不再按音频回调频率发送 IPC。

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::cancellation::CancellationToken;

type AudioOutputSlot = Arc<Mutex<Option<autolive_portaudio_output::PortAudioOutput>>>;
type DecoderProcessSlot = Arc<Mutex<Option<Child>>>;

const OUTPUT_CHANNELS: usize = 2;
const DECODE_BUFFER_BYTES: usize = 16 * 1024;
const MIX_QUEUE_CAPACITY: usize = 8;
const OUTPUT_RETRY_INTERVAL: Duration = Duration::from_millis(1);
const OUTPUT_WRITE_TIMEOUT: Duration = Duration::from_millis(500);
const OUTPUT_PAUSE_TIMEOUT: Duration = Duration::from_millis(500);
const TRUE_PEAK_DBTP: f32 = -1.5;
const PCM_FRAME_BYTES: usize = std::mem::size_of::<f32>() * OUTPUT_CHANNELS;
const MAX_DECODER_STDERR_BYTES: usize = 16 * 1024;
const REALTIME_FFMPEG_RATE_ARGS: [&str; 1] = ["-re"];
// 候选需要在有限预算内领先正在播放的画面时钟，但不能无界突发读取。
// 最低 2x；显式变速超过 1x 时再增加 1x 源时间余量，保证经过 atempo 后仍能追赶。
// 500ms 初始突发、有界 channel 和预缓冲上限继续限制内存与 CPU。
const CANDIDATE_MIN_READ_RATE: f64 = 2.0;
// 候选至少保留 50ms 可播放 PCM，并在有界预缓冲内追上滤镜启动期间推进的视频时钟。
const SWITCH_PREBUFFER_MS: usize = 50;
const SWITCH_CATCH_UP_MAX_MS: usize = 5_000;

#[derive(Debug, Clone, Copy)]
enum CandidatePrebufferPolicy {
    CatchUp,
    FixedWindow { buffer_ms: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecoderPacing {
    RealTime,
    CatchUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioMixerConfigContext {
    pub sample_rate_hz: u32,
    pub prebuffer_ms: usize,
    pub audio_stream_variant_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioMixerReadinessError {
    PreheatTimeout {
        waited_ms: u64,
        timeout_ms: u64,
        config: AudioMixerConfigContext,
    },
    Ffmpeg {
        waited_ms: u64,
        config: AudioMixerConfigContext,
        reason: String,
    },
    Runtime {
        waited_ms: u64,
        config: AudioMixerConfigContext,
        reason: String,
    },
}

impl std::fmt::Display for AudioMixerReadinessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, waited_ms, config, detail) = match self {
            Self::PreheatTimeout {
                waited_ms,
                timeout_ms,
                config,
            } => (
                format!("音轨预热超时，无有效 PCM（预算 {timeout_ms}ms）"),
                *waited_ms,
                *config,
                None,
            ),
            Self::Ffmpeg {
                waited_ms,
                config,
                reason,
            } => (
                "FFmpeg 真实错误".to_owned(),
                *waited_ms,
                *config,
                Some(reason),
            ),
            Self::Runtime {
                waited_ms,
                config,
                reason,
            } => (
                "音频运行时错误".to_owned(),
                *waited_ms,
                *config,
                Some(reason),
            ),
        };
        write!(
            formatter,
            "{kind}；已等待 {waited_ms}ms；配置：采样率 {}Hz、预缓冲 {}ms、音频支路 {} 条",
            config.sample_rate_hz, config.prebuffer_ms, config.audio_stream_variant_count,
        )?;
        if let Some(detail) = detail {
            write!(formatter, "：{detail}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
enum AudioMixerFailure {
    Ffmpeg(String),
    Runtime(String),
    OutputBackpressure(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OutputWriteError {
    ConsumerNoProgress {
        waited_ms: u64,
        remaining_samples: usize,
    },
    OutputState(String),
}

impl OutputWriteError {
    fn preserves_output(&self) -> bool {
        matches!(self, Self::ConsumerNoProgress { .. })
    }
}

impl std::fmt::Display for OutputWriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConsumerNoProgress {
                waited_ms,
                remaining_samples,
            } => write!(
                formatter,
                "输出消费者无进度：PortAudio 环缓背压，已等待 {waited_ms}ms，剩余 {remaining_samples} 个 PCM 样本未写入；候选 PCM 未判定为失效"
            ),
            Self::OutputState(reason) => formatter.write_str(reason),
        }
    }
}

impl AudioMixerFailure {
    fn message(&self) -> &str {
        match self {
            Self::Ffmpeg(message) | Self::Runtime(message) | Self::OutputBackpressure(message) => {
                message
            }
        }
    }

    fn is_output_backpressure(&self) -> bool {
        matches!(self, Self::OutputBackpressure(_))
    }
}

#[derive(Debug)]
pub struct AudioMixerTask {
    cancellation: CancellationToken,
    decoder_handle: Option<JoinHandle<()>>,
    mixer_handle: Option<JoinHandle<()>>,
    decoder_process: DecoderProcessSlot,
    output_slot: AudioOutputSlot,
    failure: Arc<Mutex<Option<AudioMixerFailure>>>,
    config: AudioMixerConfigContext,
    write_enabled: Arc<AtomicBool>,
    resume_requested: Arc<AtomicBool>,
    paused_acknowledged: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    prebuffer: Arc<Mutex<VecDeque<f32>>>,
    start_position_ms: u64,
    clear_output_on_stop: bool,
}

impl AudioMixerTask {
    pub fn start(
        output_slot: AudioOutputSlot,
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        sample_rate_hz: u32,
        start_position_ms: u64,
    ) -> Result<Self, String> {
        Self::start_with_filter(
            output_slot,
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            start_position_ms,
            None,
        )
    }

    pub fn start_with_filter(
        output_slot: AudioOutputSlot,
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        sample_rate_hz: u32,
        start_position_ms: u64,
        filter_graph: Option<String>,
    ) -> Result<Self, String> {
        Self::start_with_filter_and_variant_count(
            output_slot,
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            start_position_ms,
            filter_graph,
            1,
        )
    }

    pub fn start_with_filter_and_variant_count(
        output_slot: AudioOutputSlot,
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        sample_rate_hz: u32,
        start_position_ms: u64,
        filter_graph: Option<String>,
        audio_stream_variant_count: usize,
    ) -> Result<Self, String> {
        Self::start_internal(
            output_slot,
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            start_position_ms,
            start_position_ms,
            filter_graph,
            audio_stream_variant_count,
            false,
            DecoderPacing::RealTime,
            1.0,
            CandidatePrebufferPolicy::CatchUp,
            SWITCH_PREBUFFER_MS,
        )
    }

    /// 启动待切换音轨，但先把首段 PCM 以受控追赶速率预热到内存，不接管当前硬件出口。
    /// 候选最低使用 2x 读取速率；显式加速时增加有界余量，并保留 500ms 初始突发，
    /// 避免经过 `atempo` 后退化为等速读取而无法追上画面时钟。
    pub fn start_candidate(
        output_slot: AudioOutputSlot,
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        sample_rate_hz: u32,
        start_position_ms: u64,
    ) -> Result<Self, String> {
        Self::start_candidate_with_filter(
            output_slot,
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            start_position_ms,
            None,
        )
    }

    pub fn start_candidate_with_filter(
        output_slot: AudioOutputSlot,
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        sample_rate_hz: u32,
        start_position_ms: u64,
        filter_graph: Option<String>,
    ) -> Result<Self, String> {
        Self::start_candidate_with_filter_and_variant_count(
            output_slot,
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            start_position_ms,
            filter_graph,
            1,
            1.0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_candidate_with_filter_and_variant_count(
        output_slot: AudioOutputSlot,
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        sample_rate_hz: u32,
        start_position_ms: u64,
        filter_graph: Option<String>,
        audio_stream_variant_count: usize,
        playback_rate: f64,
    ) -> Result<Self, String> {
        Self::start_internal(
            output_slot,
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            start_position_ms,
            start_position_ms,
            filter_graph,
            audio_stream_variant_count,
            false,
            DecoderPacing::CatchUp,
            playback_rate,
            CandidatePrebufferPolicy::CatchUp,
            SWITCH_PREBUFFER_MS,
        )
    }

    /// 为未来媒体时间窗准备候选。`seek_position_ms` 是源文件内位置，
    /// `timeline_start_position_ms` 是跨循环的绝对媒体位置。
    #[allow(clippy::too_many_arguments)]
    pub fn start_scheduled_candidate_with_filter_and_variant_count(
        output_slot: AudioOutputSlot,
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        sample_rate_hz: u32,
        seek_position_ms: u64,
        timeline_start_position_ms: u64,
        filter_graph: Option<String>,
        audio_stream_variant_count: usize,
        playback_rate: f64,
        buffer_ms: usize,
        minimum_commit_tail_ms: usize,
    ) -> Result<Self, String> {
        Self::start_internal(
            output_slot,
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            seek_position_ms,
            timeline_start_position_ms,
            filter_graph,
            audio_stream_variant_count,
            false,
            DecoderPacing::RealTime,
            playback_rate,
            CandidatePrebufferPolicy::FixedWindow { buffer_ms },
            minimum_commit_tail_ms,
        )
    }

    // 线程启动边界显式传递所有权；为减少参数数目包装一次性配置对象反而会隐藏生命周期。
    #[allow(clippy::too_many_arguments)]
    fn start_internal(
        output_slot: AudioOutputSlot,
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        sample_rate_hz: u32,
        seek_position_ms: u64,
        timeline_start_position_ms: u64,
        filter_graph: Option<String>,
        audio_stream_variant_count: usize,
        write_enabled_initially: bool,
        decoder_pacing: DecoderPacing,
        playback_rate: f64,
        candidate_prebuffer_policy: CandidatePrebufferPolicy,
        minimum_commit_tail_ms: usize,
    ) -> Result<Self, String> {
        validate_audio_source(&ffmpeg_path, &source_path)?;
        let sample_rate_hz = match sample_rate_hz {
            44_100 | 48_000 => sample_rate_hz,
            _ => autolive_portaudio_output::DEFAULT_SAMPLE_RATE_HZ,
        };
        let cancellation = CancellationToken::new();
        let failure: Arc<Mutex<Option<AudioMixerFailure>>> = Arc::new(Mutex::new(None));
        let decoder_process = Arc::new(Mutex::new(None));
        let write_enabled = Arc::new(AtomicBool::new(write_enabled_initially));
        let resume_requested = Arc::new(AtomicBool::new(false));
        let paused_acknowledged = Arc::new(AtomicBool::new(!write_enabled_initially));
        let ready = Arc::new(AtomicBool::new(write_enabled_initially));
        let prebuffer = Arc::new(Mutex::new(VecDeque::new()));
        let prebuffer_limit_ms = match candidate_prebuffer_policy {
            CandidatePrebufferPolicy::CatchUp => SWITCH_CATCH_UP_MAX_MS + SWITCH_PREBUFFER_MS,
            CandidatePrebufferPolicy::FixedWindow { buffer_ms } => buffer_ms,
        };
        let prebuffer_limit_samples = sample_rate_hz
            .saturating_mul(prebuffer_limit_ms as u32)
            .saturating_div(1_000)
            .saturating_mul(OUTPUT_CHANNELS as u32)
            .max(PCM_FRAME_BYTES as u32) as usize;
        let prebuffer_started_at = Instant::now();
        let config = AudioMixerConfigContext {
            sample_rate_hz,
            prebuffer_ms: minimum_commit_tail_ms,
            audio_stream_variant_count,
        };
        let (sender, receiver) = mpsc::sync_channel(MIX_QUEUE_CAPACITY);

        let decoder_cancellation = cancellation.clone();
        let decoder_failure = Arc::clone(&failure);
        let decoder_process_slot = Arc::clone(&decoder_process);
        let decoder_handle = thread::Builder::new()
            .name("autolive-ffmpeg-decoder".to_owned())
            .spawn(move || {
                decode_audio_loop(
                    &decoder_cancellation,
                    &decoder_failure,
                    sender,
                    &ffmpeg_path,
                    &source_path,
                    sample_rate_hz,
                    seek_position_ms,
                    filter_graph.as_deref(),
                    decoder_process_slot,
                    decoder_pacing,
                    playback_rate,
                );
            })
            .map_err(|error| format!("启动 FFmpeg 解码线程失败：{error}"))?;

        let mixer_cancellation = cancellation.clone();
        let mixer_failure = Arc::clone(&failure);
        let mixer_output_slot = Arc::clone(&output_slot);
        let mixer_write_enabled = Arc::clone(&write_enabled);
        let mixer_resume_requested = Arc::clone(&resume_requested);
        let mixer_paused_acknowledged = Arc::clone(&paused_acknowledged);
        let mixer_ready = Arc::clone(&ready);
        let mixer_prebuffer = Arc::clone(&prebuffer);
        let mixer_handle = match thread::Builder::new()
            .name("autolive-audio-mixer".to_owned())
            .spawn(move || {
                mix_audio_loop(
                    &mixer_cancellation,
                    &mixer_failure,
                    receiver,
                    mixer_output_slot,
                    mixer_write_enabled,
                    mixer_resume_requested,
                    mixer_paused_acknowledged,
                    mixer_ready,
                    mixer_prebuffer,
                    prebuffer_limit_samples,
                    sample_rate_hz,
                    prebuffer_started_at,
                    candidate_prebuffer_policy,
                );
            }) {
            Ok(handle) => handle,
            Err(error) => {
                cancellation.cancel();
                terminate_decoder_process(&decoder_process);
                let _ = decoder_handle.join();
                return Err(format!("启动音频混音线程失败：{error}"));
            }
        };

        Ok(Self {
            cancellation,
            decoder_handle: Some(decoder_handle),
            mixer_handle: Some(mixer_handle),
            decoder_process,
            output_slot,
            failure,
            config,
            write_enabled,
            resume_requested,
            paused_acknowledged,
            ready,
            prebuffer,
            start_position_ms: timeline_start_position_ms,
            clear_output_on_stop: write_enabled_initially,
        })
    }

    pub fn stop(&mut self) {
        self.stop_inner(self.clear_output_on_stop);
    }

    /// 停止任务但保留环缓中尚未播放的旧轨，供候选切换失败时继续播放。
    pub fn stop_preserving_output(&mut self) {
        self.clear_output_on_stop = false;
        self.stop_inner(false);
    }

    /// 暂停旧轨向共享环缓写入，等待混音线程确认后才能提交候选。
    /// 失败时旧任务仍保留在调用方手中，可继续播放或恢复。
    pub fn pause_output_for_switch(&self) -> Result<(), String> {
        if self.cancellation.is_cancelled() {
            return Err("旧音轨已取消，无法暂停切换".to_owned());
        }
        self.resume_requested.store(false, Ordering::Release);
        self.write_enabled.store(false, Ordering::Release);
        let started = Instant::now();
        while !self.paused_acknowledged.load(Ordering::Acquire) {
            if let Some(failure) = self.failure_snapshot() {
                return Err(failure.message().to_owned());
            }
            if started.elapsed() >= OUTPUT_PAUSE_TIMEOUT {
                return Err("旧音轨混音线程未在 500ms 内确认暂停".to_owned());
            }
            thread::sleep(OUTPUT_RETRY_INTERVAL);
        }
        Ok(())
    }

    /// 候选提交失败时恢复旧轨；候选成功时不调用此方法。
    pub fn resume_output_after_switch_failure(&self) {
        if self.cancellation.is_cancelled() {
            return;
        }
        self.resume_requested.store(true, Ordering::Release);
        self.write_enabled.store(true, Ordering::Release);
    }

    fn stop_inner(&mut self, clear_output: bool) {
        self.cancellation.cancel();
        terminate_decoder_process(&self.decoder_process);
        if let Some(handle) = self.mixer_handle.take() {
            join_thread(handle, "音频混音");
        }
        if let Some(handle) = self.decoder_handle.take() {
            join_thread(handle, "FFmpeg 解码");
        }
        if clear_output {
            if let Ok(mut output) = self.output_slot.lock() {
                if let Some(stream) = output.as_mut() {
                    stream.clear_ring();
                }
            }
        }
    }

    pub fn wait_until_ready_with_reason(
        &self,
        timeout: Duration,
    ) -> Result<(), AudioMixerReadinessError> {
        let started = Instant::now();
        while !self.ready.load(Ordering::Acquire) && started.elapsed() < timeout {
            if self.cancellation.is_cancelled() {
                return Err(AudioMixerReadinessError::Runtime {
                    waited_ms: elapsed_ms(started),
                    config: self.config,
                    reason: "音频混音任务已取消".to_owned(),
                });
            }
            if let Some(failure) = self.failure_snapshot() {
                return Err(self.readiness_error(elapsed_ms(started), failure));
            }
            thread::sleep(Duration::from_millis(5));
        }
        if self.ready.load(Ordering::Acquire) && !self.cancellation.is_cancelled() {
            return Ok(());
        }
        if let Some(failure) = self.failure_snapshot() {
            return Err(self.readiness_error(elapsed_ms(started), failure));
        }
        Err(AudioMixerReadinessError::PreheatTimeout {
            waited_ms: elapsed_ms(started),
            timeout_ms: timeout.as_millis().min(u128::from(u64::MAX)) as u64,
            config: self.config,
        })
    }

    /// 在当前视频位置提交候选音轨，丢弃预热期间已经落后的 PCM。
    ///
    /// 提交只把固定 50ms 的候选 PCM 原子放入输出环缓；多余连续 PCM 仍留在候选
    /// 预缓冲中，待调用方确认 PortAudio 消费者 Active 后由 [`Self::enable_output`]
    /// 触发混音线程按正常 50ms 水位继续写入。这样候选预缓冲失败时可以在停止旧轨前回滚。
    pub fn commit_at_position(&self, position_ms: u64) -> Result<(), String> {
        let mut prebuffer = self
            .prebuffer
            .lock()
            .map_err(|_| "音频候选预缓冲锁已损坏".to_owned())?;
        trim_prebuffer_to_position(
            &mut prebuffer,
            self.start_position_ms,
            position_ms,
            self.config.sample_rate_hz,
            SWITCH_PREBUFFER_MS,
        )?;
        let prime_samples = candidate_prime_samples(&prebuffer, self.config.sample_rate_hz)?;
        let ring_capacity_samples = output_slot_capacity_samples(&self.output_slot)?;
        validate_candidate_prebuffer_capacity(prime_samples.len(), ring_capacity_samples)?;
        prime_samples_to_output(&self.cancellation, &self.output_slot, &prime_samples)?;
        prebuffer.drain(..prime_samples.len());
        Ok(())
    }

    /// 在输出消费者确认已经 Active 后启用候选混音线程。
    pub fn enable_output(&self) {
        if !self.cancellation.is_cancelled() {
            self.write_enabled.store(true, Ordering::Release);
        }
    }

    /// 只验证候选是否已经追上目标位置；验证失败时旧混音任务仍继续供给硬件出口。
    pub fn validate_commit_at_position(&self, position_ms: u64) -> Result<(), String> {
        let buffered = self
            .prebuffer
            .lock()
            .map_err(|_| "音频候选预缓冲锁已损坏".to_owned())?;
        prebuffer_skip_samples(
            buffered.len(),
            self.start_position_ms,
            position_ms,
            self.config.sample_rate_hz,
            self.config.prebuffer_ms,
        )
        .map(|_| ())
    }

    pub fn failure(&self) -> Option<String> {
        self.failure_snapshot()
            .map(|failure| failure.message().to_owned())
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
            && !self.cancellation.is_cancelled()
            && self.failure_snapshot().is_none()
    }

    pub fn prebuffered_ms(&self) -> u64 {
        let samples = self
            .prebuffer
            .lock()
            .ok()
            .map(|buffered| buffered.len())
            .unwrap_or(0);
        (samples as u64)
            .saturating_mul(1_000)
            .saturating_div(u64::from(self.config.sample_rate_hz.max(1)))
            .saturating_div(OUTPUT_CHANNELS as u64)
    }

    /// 返回当前实时 FFmpeg 子进程 PID；任务尚未生成或已经退出时返回 None。
    pub fn ffmpeg_pid(&self) -> Option<u32> {
        self.decoder_process
            .lock()
            .ok()
            .and_then(|process| process.as_ref().map(Child::id))
    }

    pub fn has_output_backpressure(&self) -> bool {
        self.failure_snapshot()
            .is_some_and(|failure| failure.is_output_backpressure())
    }

    fn failure_snapshot(&self) -> Option<AudioMixerFailure> {
        self.failure.lock().ok().and_then(|failure| failure.clone())
    }

    fn readiness_error(
        &self,
        waited_ms: u64,
        failure: AudioMixerFailure,
    ) -> AudioMixerReadinessError {
        match failure {
            AudioMixerFailure::Ffmpeg(reason) => AudioMixerReadinessError::Ffmpeg {
                waited_ms,
                config: self.config,
                reason,
            },
            AudioMixerFailure::Runtime(reason) | AudioMixerFailure::OutputBackpressure(reason) => {
                AudioMixerReadinessError::Runtime {
                    waited_ms,
                    config: self.config,
                    reason,
                }
            }
        }
    }
}

impl Drop for AudioMixerTask {
    fn drop(&mut self) {
        self.stop();
    }
}

fn validate_audio_source(ffmpeg_path: &Path, source_path: &Path) -> Result<(), String> {
    if !ffmpeg_path.is_file() {
        return Err("FFmpeg 可执行文件不存在".to_owned());
    }
    if !source_path.is_file() {
        return Err("PortAudio 音频源不存在".to_owned());
    }
    Ok(())
}

// 解码线程入口显式接收其拥有/借用的资源，避免再造只使用一次的上下文容器。
#[allow(clippy::too_many_arguments)]
fn decode_audio_loop(
    cancellation: &CancellationToken,
    failure: &Arc<Mutex<Option<AudioMixerFailure>>>,
    sender: mpsc::SyncSender<Vec<f32>>,
    ffmpeg_path: &Path,
    source_path: &Path,
    sample_rate_hz: u32,
    start_position_ms: u64,
    filter_graph: Option<&str>,
    decoder_process: DecoderProcessSlot,
    decoder_pacing: DecoderPacing,
    playback_rate: f64,
) {
    let mut first_loop = true;
    while !cancellation.is_cancelled() {
        let start_seconds = if first_loop {
            format!("{:.3}", start_position_ms as f64 / 1000.0)
        } else {
            "0".to_owned()
        };
        let mut command = Command::new(ffmpeg_path);
        command.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
        match decoder_pacing {
            DecoderPacing::RealTime => {
                command.args(REALTIME_FFMPEG_RATE_ARGS);
            }
            DecoderPacing::CatchUp => {
                let read_rate = format!("{:.3}", candidate_ffmpeg_read_rate(playback_rate));
                command.args(["-readrate", &read_rate, "-readrate_initial_burst", "0.5"]);
            }
        }
        command
            .args(["-ss", &start_seconds, "-stream_loop", "-1", "-i"])
            .arg(source_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(filter_graph) = filter_graph {
            command.args(["-filter_complex", filter_graph, "-map", "[aout]"]);
        } else {
            command.args(["-map", "0:a:0?"]);
        }
        command.args([
            "-vn",
            "-sn",
            "-dn",
            "-ac",
            "2",
            "-ar",
            &sample_rate_hz.to_string(),
            "-f",
            "f32le",
            "-acodec",
            "pcm_f32le",
            "pipe:1",
        ]);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                set_failure(
                    failure,
                    AudioMixerFailure::Ffmpeg(sanitize_error_detail(
                        &format!("FFmpeg 音频解码启动失败：{error}"),
                        &[ffmpeg_path, source_path],
                    )),
                );
                return;
            }
        };
        let stderr_reader = child
            .stderr
            .take()
            .map(|stderr| thread::spawn(move || read_stderr_tail(stderr)));
        let Some(mut stdout) = child.stdout.take() else {
            set_failure(
                failure,
                AudioMixerFailure::Ffmpeg("FFmpeg 未提供音频解码输出".to_owned()),
            );
            terminate_child_process(&mut child);
            let _ = join_stderr_reader(stderr_reader);
            return;
        };
        if let Ok(mut process) = decoder_process.lock() {
            *process = Some(child);
        } else {
            set_failure(
                failure,
                AudioMixerFailure::Runtime("FFmpeg 子进程状态锁已损坏".to_owned()),
            );
            let mut child = child;
            terminate_child_process(&mut child);
            let _ = join_stderr_reader(stderr_reader);
            return;
        }

        let mut bytes = vec![0_u8; DECODE_BUFFER_BYTES];
        let mut pending = Vec::with_capacity(PCM_FRAME_BYTES);
        let mut emitted_samples = 0_usize;
        loop {
            if cancellation.is_cancelled() {
                terminate_decoder_process(&decoder_process);
                let _ = join_stderr_reader(stderr_reader);
                return;
            }
            let read = match stdout.read(&mut bytes) {
                Ok(read) => read,
                Err(error) => {
                    if !cancellation.is_cancelled() {
                        set_failure(
                            failure,
                            AudioMixerFailure::Ffmpeg(format!("FFmpeg 音频解码读取失败：{error}")),
                        );
                    }
                    terminate_decoder_process(&decoder_process);
                    let _ = join_stderr_reader(stderr_reader);
                    return;
                }
            };
            if read == 0 {
                break;
            }
            pending.extend_from_slice(&bytes[..read]);
            let samples = take_complete_stereo_samples(&mut pending);
            if samples.is_empty() {
                continue;
            }
            emitted_samples += samples.len();
            if !send_samples_with_cancellation(cancellation, &sender, samples) {
                terminate_decoder_process(&decoder_process);
                let _ = join_stderr_reader(stderr_reader);
                return;
            }
        }
        // 先从共享槽位取出 Child，再等待；不能持有槽位锁调用 wait，
        // 否则 stop_inner 无法取得同一把锁来终止子进程。
        let mut child = decoder_process
            .lock()
            .ok()
            .and_then(|mut process| process.take());
        let status = child.as_mut().and_then(|child| {
            if cancellation.is_cancelled() {
                terminate_child_process(child);
                return None;
            }
            wait_for_child_exit(child)
        });
        let stderr = join_stderr_reader(stderr_reader);
        if cancellation.is_cancelled() {
            return;
        }
        if !pending.is_empty() {
            set_failure(
                failure,
                AudioMixerFailure::Ffmpeg("FFmpeg 音频解码输出未按完整双声道帧对齐".to_owned()),
            );
            return;
        }
        if !status.is_some_and(|status| status.success()) {
            set_failure(
                failure,
                AudioMixerFailure::Ffmpeg(format_decoder_failure(
                    "FFmpeg 音频解码异常退出",
                    stderr.as_deref(),
                    &[ffmpeg_path, source_path],
                )),
            );
            return;
        }
        if emitted_samples == 0 {
            set_failure(
                failure,
                AudioMixerFailure::Ffmpeg("FFmpeg 音频解码未输出 PCM 数据".to_owned()),
            );
            return;
        }
        first_loop = false;
    }
}

fn candidate_ffmpeg_read_rate(playback_rate: f64) -> f64 {
    let playback_rate = if playback_rate.is_finite() && playback_rate > 0.0 {
        playback_rate
    } else {
        1.0
    };
    CANDIDATE_MIN_READ_RATE.max(playback_rate + 1.0)
}

fn terminate_decoder_process(decoder_process: &DecoderProcessSlot) {
    let mut child = decoder_process
        .lock()
        .ok()
        .and_then(|mut process| process.take());
    if let Some(child) = child.as_mut() {
        terminate_child_process(child);
    }
}

fn terminate_child_process(child: &mut Child) {
    if let Ok(Some(_)) = child.try_wait() {
        return;
    }
    if let Err(error) = child.kill() {
        eprintln!("autolive audio mixer: FFmpeg 子进程终止请求失败：{error}");
    }
    if wait_for_child_exit(child).is_none() {
        eprintln!("autolive audio mixer: FFmpeg 子进程退出状态确认失败");
    }
}

fn wait_for_child_exit(child: &mut Child) -> Option<ExitStatus> {
    child.wait().ok()
}

fn send_samples_with_cancellation(
    cancellation: &CancellationToken,
    sender: &mpsc::SyncSender<Vec<f32>>,
    samples: Vec<f32>,
) -> bool {
    let mut samples = Some(samples);
    loop {
        if cancellation.is_cancelled() {
            return false;
        }
        let samples_to_send = match samples.take() {
            Some(samples) => samples,
            None => return false,
        };
        match sender.try_send(samples_to_send) {
            Ok(()) => return true,
            Err(mpsc::TrySendError::Full(samples_to_retry)) => {
                samples = Some(samples_to_retry);
                thread::sleep(OUTPUT_RETRY_INTERVAL);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => return false,
        }
    }
}

// 混音线程入口显式接收有界队列、状态与时钟，便于审查取消和所有权边界。
#[allow(clippy::too_many_arguments)]
fn mix_audio_loop(
    cancellation: &CancellationToken,
    failure: &Arc<Mutex<Option<AudioMixerFailure>>>,
    receiver: mpsc::Receiver<Vec<f32>>,
    output_slot: AudioOutputSlot,
    write_enabled: Arc<AtomicBool>,
    resume_requested: Arc<AtomicBool>,
    paused_acknowledged: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    prebuffer: Arc<Mutex<VecDeque<f32>>>,
    prebuffer_limit_samples: usize,
    sample_rate_hz: u32,
    prebuffer_started_at: Instant,
    candidate_prebuffer_policy: CandidatePrebufferPolicy,
) {
    let mut prebuffer_pending = true;
    while !cancellation.is_cancelled() {
        if write_enabled.load(Ordering::Acquire) {
            if resume_requested.swap(false, Ordering::AcqRel) {
                prebuffer_pending = true;
            }
            paused_acknowledged.store(false, Ordering::Release);
        } else {
            paused_acknowledged.store(true, Ordering::Release);
        }
        // 候选追时钟缓存到达上限后暂停消费，让有界 channel 反压 FFmpeg；
        // 不能继续读取后丢弃，否则提交后的下一块 PCM 会产生时间跳跃。
        if !write_enabled.load(Ordering::Acquire)
            && prebuffer
                .lock()
                .ok()
                .is_some_and(|buffered| buffered.len() >= prebuffer_limit_samples)
        {
            thread::sleep(OUTPUT_RETRY_INTERVAL);
            continue;
        }
        let mut samples = match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(samples) => samples,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if !cancellation.is_cancelled() {
                    set_failure(
                        failure,
                        AudioMixerFailure::Runtime("音频解码输入已断开".to_owned()),
                    );
                }
                if write_enabled.load(Ordering::Acquire) && !cancellation.is_cancelled() {
                    stop_output(&output_slot);
                }
                return;
            }
        };
        process_audio_bus(&mut samples);
        if !write_enabled.load(Ordering::Acquire) {
            if let Ok(mut buffered) = prebuffer.lock() {
                let buffered_samples =
                    append_prebuffer(&mut buffered, &samples, prebuffer_limit_samples);
                let elapsed_ms = elapsed_ms(prebuffer_started_at) as usize;
                let required_samples = candidate_readiness_samples(
                    sample_rate_hz,
                    elapsed_ms,
                    prebuffer_limit_samples,
                    candidate_prebuffer_policy,
                );
                if buffered_samples >= required_samples {
                    ready.store(true, Ordering::Release);
                }
            } else {
                set_failure(
                    failure,
                    AudioMixerFailure::Runtime("音频候选预缓冲锁已损坏".to_owned()),
                );
                return;
            }
            continue;
        }
        if prebuffer_pending {
            // commit_at_position 只 prime 固定 50ms；这里先按顺序消费剩余候选 PCM，
            // 交给正常 50ms 水位写入，再处理新解码块，避免丢弃或重排连续音频。
            let buffered = match prebuffer.lock() {
                Ok(mut buffered) => buffered.drain(..).collect::<Vec<_>>(),
                Err(_) => {
                    set_failure(
                        failure,
                        AudioMixerFailure::Runtime("音频候选预缓冲锁已损坏".to_owned()),
                    );
                    return;
                }
            };
            if let Err(error) = write_samples_to_output(cancellation, &output_slot, &buffered) {
                handle_output_write_error(cancellation, failure, &output_slot, "预缓冲", error);
                return;
            }
            prebuffer_pending = false;
        }
        let write_result = write_samples_to_output(cancellation, &output_slot, &samples);
        if let Err(error) = write_result {
            handle_output_write_error(cancellation, failure, &output_slot, "输出", error);
            return;
        }
    }
}

fn handle_output_write_error(
    cancellation: &CancellationToken,
    failure: &Arc<Mutex<Option<AudioMixerFailure>>>,
    output_slot: &AudioOutputSlot,
    stage: &str,
    error: OutputWriteError,
) {
    if !cancellation.is_cancelled() {
        set_failure(
            failure,
            if error.preserves_output() {
                AudioMixerFailure::OutputBackpressure(format!(
                    "音频混音线程{stage}写入异常：{error}"
                ))
            } else {
                AudioMixerFailure::Runtime(format!("音频混音线程{stage}写入异常：{error}"))
            },
        );
    }
    // 环缓背压只说明消费者暂时没有进度；保留现有环缓，交给上层的
    // PortAudio 健康检查决定是否回退，避免候选失败时主动清掉旧轨。
    if !cancellation.is_cancelled() && !error.preserves_output() {
        stop_output(output_slot);
    }
}

fn append_prebuffer(buffered: &mut VecDeque<f32>, samples: &[f32], limit_samples: usize) -> usize {
    if buffered.len() < limit_samples {
        // 整块保留，最多只会比上限多一个解码块；拆块丢尾会破坏 PCM 连续性。
        buffered.extend(samples.iter().copied());
    }
    buffered.len()
}

fn candidate_required_prebuffer_samples(
    sample_rate_hz: u32,
    elapsed_ms: usize,
    max_samples: usize,
) -> usize {
    let required_ms = elapsed_ms.saturating_add(SWITCH_PREBUFFER_MS);
    let required_samples = u64::from(sample_rate_hz)
        .saturating_mul(required_ms as u64)
        .saturating_div(1_000)
        .saturating_mul(OUTPUT_CHANNELS as u64);
    required_samples.min(max_samples as u64) as usize
}

fn candidate_readiness_samples(
    sample_rate_hz: u32,
    elapsed_ms: usize,
    max_samples: usize,
    policy: CandidatePrebufferPolicy,
) -> usize {
    match policy {
        CandidatePrebufferPolicy::CatchUp => {
            candidate_required_prebuffer_samples(sample_rate_hz, elapsed_ms, max_samples)
        }
        CandidatePrebufferPolicy::FixedWindow { buffer_ms } => {
            stereo_samples_for_ms(sample_rate_hz, buffer_ms).min(max_samples)
        }
    }
}

fn write_samples_to_output(
    cancellation: &CancellationToken,
    output_slot: &AudioOutputSlot,
    samples: &[f32],
) -> Result<(), OutputWriteError> {
    write_samples_to_output_with_timeout(
        cancellation,
        output_slot,
        samples,
        OUTPUT_WRITE_TIMEOUT,
        |output, samples| output.write_stereo_interleaved_available(samples),
    )
}

fn write_samples_to_output_with_timeout<F>(
    cancellation: &CancellationToken,
    output_slot: &AudioOutputSlot,
    samples: &[f32],
    timeout: Duration,
    write: F,
) -> Result<(), OutputWriteError>
where
    F: FnMut(&mut autolive_portaudio_output::PortAudioOutput, &[f32]) -> Result<usize, String>,
{
    write_samples_to_output_with_timeout_observed(
        cancellation,
        output_slot,
        samples,
        timeout,
        |output| output.progress_snapshot(),
        write,
    )
}

fn write_samples_to_output_with_timeout_observed<F, O>(
    cancellation: &CancellationToken,
    output_slot: &AudioOutputSlot,
    samples: &[f32],
    timeout: Duration,
    mut observe: O,
    mut write: F,
) -> Result<(), OutputWriteError>
where
    F: FnMut(&mut autolive_portaudio_output::PortAudioOutput, &[f32]) -> Result<usize, String>,
    O: FnMut(
        &autolive_portaudio_output::PortAudioOutput,
    ) -> autolive_portaudio_output::PortAudioOutputProgress,
{
    let mut offset = 0_usize;
    let mut last_progress = None;
    let mut last_progress_at = Instant::now();
    while offset < samples.len() {
        if cancellation.is_cancelled() {
            return Ok(());
        }
        let (written, progress_before, progress_after) = output_slot
            .lock()
            .map_err(|_| OutputWriteError::OutputState("PortAudio 状态锁已损坏".to_owned()))
            .and_then(|mut output| {
                let output = output.as_mut().ok_or_else(|| {
                    OutputWriteError::OutputState("PortAudio 输出已停止".to_owned())
                })?;
                let progress_before = observe(output);
                let written =
                    write(output, &samples[offset..]).map_err(OutputWriteError::OutputState)?;
                let progress_after = observe(output);
                Ok((written, progress_before, progress_after))
            })?;

        if last_progress.is_none() {
            last_progress = Some(progress_before);
            last_progress_at = Instant::now();
        }
        if last_progress.is_some_and(|previous| consumer_progressed(previous, progress_before)) {
            last_progress = Some(progress_before);
            last_progress_at = Instant::now();
        }
        if last_progress.is_some_and(|previous| consumer_progressed(previous, progress_after)) {
            last_progress = Some(progress_after);
            last_progress_at = Instant::now();
        }

        if written == 0 {
            if last_progress_at.elapsed() >= timeout {
                return Err(OutputWriteError::ConsumerNoProgress {
                    waited_ms: elapsed_ms(last_progress_at),
                    remaining_samples: samples.len().saturating_sub(offset),
                });
            }
            thread::sleep(OUTPUT_RETRY_INTERVAL);
        } else {
            offset = offset.saturating_add(written).min(samples.len());
        }
    }
    Ok(())
}

fn consumer_progressed(
    previous: autolive_portaudio_output::PortAudioOutputProgress,
    current: autolive_portaudio_output::PortAudioOutputProgress,
) -> bool {
    current.callback_count > previous.callback_count
        || current.ring_len_samples < previous.ring_len_samples
}

fn output_slot_capacity_samples(output_slot: &AudioOutputSlot) -> Result<usize, String> {
    let output = output_slot
        .lock()
        .map_err(|_| "PortAudio 状态锁已损坏".to_owned())?;
    output
        .as_ref()
        .map(autolive_portaudio_output::PortAudioOutput::ring_capacity_samples)
        .ok_or_else(|| "PortAudio 输出已停止".to_owned())
}

fn validate_candidate_prebuffer_capacity(
    candidate_samples: usize,
    ring_capacity_samples: usize,
) -> Result<(), String> {
    if candidate_samples <= ring_capacity_samples {
        return Ok(());
    }
    Err(format!(
        "候选预缓冲无法原子提交：{} 个 PCM 样本超过 PortAudio 环缓容量 {} 个样本",
        candidate_samples, ring_capacity_samples
    ))
}

fn stereo_samples_for_ms(sample_rate_hz: u32, duration_ms: usize) -> usize {
    usize::try_from(
        u64::from(sample_rate_hz)
            .saturating_mul(duration_ms as u64)
            .saturating_div(1_000)
            .saturating_mul(OUTPUT_CHANNELS as u64),
    )
    .unwrap_or(usize::MAX)
}

fn candidate_prime_samples(
    buffered: &VecDeque<f32>,
    sample_rate_hz: u32,
) -> Result<Vec<f32>, String> {
    let prime_len = stereo_samples_for_ms(sample_rate_hz, SWITCH_PREBUFFER_MS);
    if buffered.len() < prime_len {
        return Err(format!(
            "候选音轨剩余 PCM 不足 {}ms，无法原子提交",
            SWITCH_PREBUFFER_MS
        ));
    }
    Ok(buffered.iter().take(prime_len).copied().collect())
}

fn prime_samples_to_output(
    cancellation: &CancellationToken,
    output_slot: &AudioOutputSlot,
    samples: &[f32],
) -> Result<(), String> {
    write_samples_to_output_with_timeout(
        cancellation,
        output_slot,
        samples,
        OUTPUT_WRITE_TIMEOUT,
        |output, samples| output.prime_stereo_interleaved_available(samples),
    )
    .map_err(|error| error.to_string())
}

fn trim_prebuffer_to_position(
    buffered: &mut VecDeque<f32>,
    start_position_ms: u64,
    position_ms: u64,
    sample_rate_hz: u32,
    prebuffer_ms: usize,
) -> Result<(), String> {
    let skip_samples = prebuffer_skip_samples(
        buffered.len(),
        start_position_ms,
        position_ms,
        sample_rate_hz,
        prebuffer_ms,
    )?;
    buffered.drain(..skip_samples);
    Ok(())
}

fn prebuffer_skip_samples(
    buffered_samples: usize,
    start_position_ms: u64,
    position_ms: u64,
    sample_rate_hz: u32,
    minimum_remaining_ms: usize,
) -> Result<usize, String> {
    let elapsed_ms = position_ms.saturating_sub(start_position_ms);
    let skip_samples = (u64::from(sample_rate_hz)
        .saturating_mul(elapsed_ms)
        .saturating_div(1_000)
        .saturating_mul(u64::from(OUTPUT_CHANNELS as u32))) as usize;
    let minimum_remaining_samples = (u64::from(sample_rate_hz)
        .saturating_mul(minimum_remaining_ms as u64)
        .saturating_div(1_000)
        .saturating_mul(OUTPUT_CHANNELS as u64)) as usize;
    if skip_samples.saturating_add(minimum_remaining_samples) > buffered_samples {
        let buffered_ms = (buffered_samples as u64)
            .saturating_mul(1_000)
            .saturating_div(u64::from(sample_rate_hz.max(1)))
            .saturating_div(OUTPUT_CHANNELS as u64);
        return Err(format!(
            "候选音轨尚未追上画面：起始位置 {start_position_ms}ms，目标位置 {position_ms}ms，当前仅缓冲 {buffered_ms}ms"
        ));
    }
    Ok(skip_samples)
}

fn stop_output(output_slot: &AudioOutputSlot) {
    if let Ok(mut output) = output_slot.lock() {
        if let Some(mut stream) = output.take() {
            stream.stop();
        }
    }
}

fn process_audio_bus(samples: &mut [f32]) {
    debug_assert_eq!(samples.len() % OUTPUT_CHANNELS, 0);
    let peak = samples
        .iter_mut()
        .map(|sample| {
            if !sample.is_finite() {
                *sample = 0.0;
            }
            sample.abs()
        })
        .fold(0.0_f32, f32::max);
    let true_peak = 10_f32.powf(TRUE_PEAK_DBTP / 20.0);
    if peak > true_peak {
        let scale = true_peak / peak;
        for sample in samples {
            *sample *= scale;
        }
    }
}

fn take_complete_stereo_samples(pending: &mut Vec<u8>) -> Vec<f32> {
    let usable_bytes = pending.len() - (pending.len() % PCM_FRAME_BYTES);
    if usable_bytes == 0 {
        return Vec::new();
    }
    let samples = pending[..usable_bytes]
        .chunks_exact(std::mem::size_of::<f32>())
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect::<Vec<_>>();
    pending.drain(..usable_bytes);
    samples
}

fn read_stderr_tail(mut stderr: impl Read) -> Option<String> {
    let mut retained = VecDeque::with_capacity(MAX_DECODER_STDERR_BYTES);
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stderr.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            if retained.len() == MAX_DECODER_STDERR_BYTES {
                retained.pop_front();
            }
            retained.push_back(*byte);
        }
    }
    let bytes = retained.into_iter().collect::<Vec<_>>();
    let text = String::from_utf8_lossy(&bytes);
    let compact = text
        .lines()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ");
    let compact = compact.trim();
    (!compact.is_empty()).then(|| compact.chars().take(500).collect())
}

fn join_stderr_reader(reader: Option<JoinHandle<Option<String>>>) -> Option<String> {
    let handle = reader?;
    match handle.join() {
        Ok(stderr) => stderr,
        Err(_) => {
            eprintln!("autolive audio mixer: FFmpeg stderr 线程异常退出");
            None
        }
    }
}

fn join_thread(handle: JoinHandle<()>, name: &str) {
    if handle.join().is_err() {
        eprintln!("autolive audio mixer: {name} 线程异常退出");
    }
}

fn format_decoder_failure(prefix: &str, stderr: Option<&str>, sensitive_paths: &[&Path]) -> String {
    match stderr {
        Some(stderr) => {
            let detail = sanitize_error_detail(stderr, sensitive_paths);
            if detail.is_empty() {
                prefix.to_owned()
            } else {
                format!("{prefix}：{detail}")
            }
        }
        None => prefix.to_owned(),
    }
}

fn sanitize_error_detail(detail: &str, sensitive_paths: &[&Path]) -> String {
    let mut sanitized = detail.to_owned();
    for path in sensitive_paths {
        let path = path.to_string_lossy();
        sanitized = sanitized.replace(path.as_ref(), "<path>");
    }
    sanitized
        .split_whitespace()
        .map(|token| {
            let trimmed = token.trim_matches(|character: char| {
                matches!(character, '"' | '\'' | '[' | ']' | '(' | ')' | ',' | ';')
            });
            if looks_like_path(trimmed) {
                "<path>"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect()
}

fn looks_like_path(token: &str) -> bool {
    token.starts_with('/')
        || token.starts_with("\\\\")
        || token
            .as_bytes()
            .get(1..3)
            .is_some_and(|drive| drive[0].is_ascii_alphabetic() && drive[1] == b':')
        || token.starts_with("file://")
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn set_failure(failure: &Arc<Mutex<Option<AudioMixerFailure>>>, reason: AudioMixerFailure) {
    if let Ok(mut current) = failure.lock() {
        if current.is_none() {
            eprintln!("autolive audio mixer failure: {}", reason.message());
            *current = Some(reason);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use super::{
        append_prebuffer, candidate_ffmpeg_read_rate, candidate_prime_samples,
        candidate_readiness_samples, candidate_required_prebuffer_samples,
        handle_output_write_error, prebuffer_skip_samples, process_audio_bus,
        sanitize_error_detail, send_samples_with_cancellation, take_complete_stereo_samples,
        trim_prebuffer_to_position, validate_candidate_prebuffer_capacity, write_samples_to_output,
        write_samples_to_output_with_timeout, write_samples_to_output_with_timeout_observed,
        AudioMixerConfigContext, AudioMixerReadinessError, AudioMixerTask,
        CandidatePrebufferPolicy, OutputWriteError, SWITCH_PREBUFFER_MS,
    };
    use crate::cancellation::CancellationToken;

    #[test]
    fn realtime_ffmpeg_input_uses_real_time_rate_limit() {
        assert_eq!(super::REALTIME_FFMPEG_RATE_ARGS, ["-re"]);
    }

    #[test]
    fn candidate_decoder_read_rate_stays_ahead_after_atempo() {
        assert_eq!(candidate_ffmpeg_read_rate(0.5), 2.0);
        assert_eq!(candidate_ffmpeg_read_rate(1.0), 2.0);
        assert_eq!(candidate_ffmpeg_read_rate(1.5), 2.5);
        assert_eq!(candidate_ffmpeg_read_rate(2.0), 3.0);
        assert_eq!(candidate_ffmpeg_read_rate(f64::NAN), 2.0);
    }

    #[test]
    fn audio_bus_cleans_non_finite_samples_and_limits_true_peak() {
        let mut samples = [f32::NAN, f32::INFINITY, -2.0, 0.2];
        process_audio_bus(&mut samples);
        assert_eq!(samples[0], 0.0);
        assert_eq!(samples[1], 0.0);
        let scale = 10_f32.powf(-1.5 / 20.0) / 2.0;
        assert!((samples[2].abs() - 10_f32.powf(-1.5 / 20.0)).abs() < 1e-6);
        assert!((samples[3] - 0.2 * scale).abs() < 1e-6);
    }

    #[test]
    fn decoder_preserves_partial_stereo_frame_until_next_read() {
        let bytes = [
            1.0_f32.to_le_bytes(),
            2.0_f32.to_le_bytes(),
            3.0_f32.to_le_bytes(),
            4.0_f32.to_le_bytes(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        let mut pending = bytes[..10].to_vec();
        assert_eq!(take_complete_stereo_samples(&mut pending), vec![1.0, 2.0]);
        pending.extend_from_slice(&bytes[10..]);
        assert_eq!(take_complete_stereo_samples(&mut pending), vec![3.0, 4.0]);
        assert!(pending.is_empty());
    }

    #[test]
    fn mixer_write_exits_when_cancelled_before_output_access() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let output = Arc::new(Mutex::new(None));

        write_samples_to_output(&cancellation, &output, &[0.0, 0.0]).unwrap();
    }

    #[test]
    fn decoder_send_exits_when_cancelled_while_channel_is_full() {
        let cancellation = CancellationToken::new();
        let (sender, receiver) = std::sync::mpsc::sync_channel(0);
        let cancellation_for_thread = cancellation.clone();
        let thread = std::thread::spawn(move || {
            send_samples_with_cancellation(&cancellation_for_thread, &sender, vec![0.0, 0.0])
        });

        std::thread::sleep(std::time::Duration::from_millis(5));
        cancellation.cancel();
        assert!(!thread.join().unwrap());
        drop(receiver);
    }

    #[test]
    fn finite_ring_without_consumer_reports_backpressure_timeout() {
        let mut stream = autolive_portaudio_output::PortAudioOutput::new(1_000, 128, 2);
        stream.set_prestart_writes_enabled(true);
        let capacity = autolive_portaudio_output::ring_capacity_samples(128, 2);
        let fill = vec![0.0; capacity];
        assert_eq!(
            stream.prime_stereo_interleaved_available(&fill).unwrap(),
            capacity
        );
        let output = Arc::new(Mutex::new(Some(stream)));
        let error = write_samples_to_output_with_timeout(
            &CancellationToken::new(),
            &output,
            &[0.0, 0.0],
            std::time::Duration::from_millis(5),
            |stream, samples| stream.write_stereo_interleaved_available(samples),
        )
        .unwrap_err();

        assert!(matches!(error, OutputWriteError::ConsumerNoProgress { .. }));
        assert!(error.to_string().contains("输出消费者无进度"));
        assert!(error.to_string().contains("环缓背压"));
        assert_eq!(
            output.lock().unwrap().as_ref().unwrap().ring_len_samples(),
            capacity
        );
    }

    #[test]
    fn partial_batch_with_callback_progress_does_not_timeout() {
        let stream = autolive_portaudio_output::PortAudioOutput::new(1_000, 128, 2);
        let output = Arc::new(Mutex::new(Some(stream)));
        let callback_count = Arc::new(AtomicU64::new(0));
        let observed_callback_count = Arc::clone(&callback_count);
        let mut attempts = 0_u8;

        let result = write_samples_to_output_with_timeout_observed(
            &CancellationToken::new(),
            &output,
            &[0.0, 0.0],
            std::time::Duration::from_millis(5),
            |stream| autolive_portaudio_output::PortAudioOutputProgress {
                callback_count: observed_callback_count.load(Ordering::Relaxed),
                ring_len_samples: stream.ring_len_samples(),
            },
            |_stream, samples| {
                attempts = attempts.saturating_add(1);
                if attempts < 8 {
                    callback_count.fetch_add(1, Ordering::Relaxed);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    Ok(0)
                } else {
                    Ok(samples.len())
                }
            },
        );

        assert!(result.is_ok());
        assert!(callback_count.load(Ordering::Relaxed) >= 7);
    }

    #[test]
    fn backpressure_failure_keeps_existing_output_ring() {
        let mut stream = autolive_portaudio_output::PortAudioOutput::new(1_000, 128, 2);
        stream.set_prestart_writes_enabled(true);
        let capacity = autolive_portaudio_output::ring_capacity_samples(128, 2);
        stream
            .prime_stereo_interleaved_available(&vec![0.0; capacity])
            .unwrap();
        let output = Arc::new(Mutex::new(Some(stream)));
        let failure = Arc::new(Mutex::new(None));

        handle_output_write_error(
            &CancellationToken::new(),
            &failure,
            &output,
            "预缓冲",
            OutputWriteError::ConsumerNoProgress {
                waited_ms: 500,
                remaining_samples: 2,
            },
        );

        let output = output.lock().unwrap();
        assert_eq!(output.as_ref().unwrap().ring_len_samples(), capacity);
        assert!(failure
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|failure| failure.message().contains("环缓背压")));
    }

    #[test]
    fn candidate_commit_rejects_prebuffer_larger_than_ring_capacity() {
        assert!(validate_candidate_prebuffer_capacity(100, 100).is_ok());
        let error = validate_candidate_prebuffer_capacity(101, 100).unwrap_err();
        assert!(error.contains("候选预缓冲无法原子提交"));
        assert!(error.contains("101"));
        assert!(error.contains("100"));
    }

    #[test]
    fn candidate_commit_primes_only_fixed_50ms_and_keeps_the_contiguous_tail() {
        let buffered = (0..10_000)
            .map(|value| value as f32)
            .collect::<VecDeque<_>>();
        let prime = candidate_prime_samples(&buffered, 1_000).unwrap();

        assert_eq!(prime.len(), 100);
        assert_eq!(prime.first(), Some(&0.0));
        assert_eq!(prime.last(), Some(&99.0));
        assert_eq!(buffered.len(), 10_000);

        let insufficient = (0..99).map(|value| value as f32).collect::<VecDeque<_>>();
        assert!(candidate_prime_samples(&insufficient, 1_000).is_err());
    }

    #[test]
    fn candidate_commit_capacity_uses_fixed_prime_not_full_prebuffer() {
        let full_prebuffer = candidate_required_prebuffer_samples(48_000, 5_000, usize::MAX);
        let buffered = vec![0.0; full_prebuffer]
            .into_iter()
            .collect::<VecDeque<_>>();
        let prime = candidate_prime_samples(&buffered, 48_000).unwrap();
        let ring_capacity = autolive_portaudio_output::ring_capacity_samples(1_024, 2);

        assert!(prime.len() < ring_capacity);
        assert!(validate_candidate_prebuffer_capacity(prime.len(), ring_capacity).is_ok());
        assert!(validate_candidate_prebuffer_capacity(buffered.len(), ring_capacity).is_err());
    }

    #[test]
    fn candidate_commit_primes_fixed_depth_and_leaves_tail_for_mixer() {
        let mut output = autolive_portaudio_output::PortAudioOutput::new(48_000, 1_024, 2);
        output.set_prestart_writes_enabled(true);
        let output_slot = Arc::new(Mutex::new(Some(output)));
        let prebuffer = Arc::new(Mutex::new(
            (0..10_000)
                .map(|value| value as f32)
                .collect::<VecDeque<_>>(),
        ));
        let task = AudioMixerTask {
            cancellation: CancellationToken::new(),
            decoder_handle: None,
            mixer_handle: None,
            decoder_process: Arc::new(Mutex::new(None)),
            output_slot: Arc::clone(&output_slot),
            failure: Arc::new(Mutex::new(None)),
            config: AudioMixerConfigContext {
                sample_rate_hz: 48_000,
                prebuffer_ms: SWITCH_PREBUFFER_MS,
                audio_stream_variant_count: 1,
            },
            write_enabled: Arc::new(AtomicBool::new(false)),
            resume_requested: Arc::new(AtomicBool::new(false)),
            paused_acknowledged: Arc::new(AtomicBool::new(true)),
            ready: Arc::new(AtomicBool::new(true)),
            prebuffer: Arc::clone(&prebuffer),
            start_position_ms: 0,
            clear_output_on_stop: true,
        };

        task.commit_at_position(0).unwrap();

        assert_eq!(
            output_slot
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .ring_len_samples(),
            4_800
        );
        let prebuffer = prebuffer.lock().unwrap();
        assert_eq!(prebuffer.len(), 5_200);
        assert_eq!(prebuffer.front(), Some(&4_800.0));
    }

    #[test]
    fn candidate_prebuffer_catches_up_slow_filter_start_and_stays_bounded() {
        let mut buffered = VecDeque::new();
        assert_eq!(append_prebuffer(&mut buffered, &[1.0, 2.0], 4), 2);
        assert_eq!(append_prebuffer(&mut buffered, &[3.0, 4.0, 5.0, 6.0], 4), 6);
        assert_eq!(buffered.len(), 6);
        assert_eq!(append_prebuffer(&mut buffered, &[7.0, 8.0, 9.0], 4), 6);
        assert_eq!(buffered.len(), 6);

        // 1kHz 双声道、滤镜启动耗时 2 秒时，候选必须保留 2.05 秒 PCM，
        // 而不是只保留最早 50ms 后继续丢弃后续数据。
        assert_eq!(
            candidate_required_prebuffer_samples(1_000, 2_000, 10_100),
            4_100
        );
    }

    #[test]
    fn scheduled_candidate_readiness_uses_a_fixed_window() {
        let max_samples = 20_000;
        let policy = CandidatePrebufferPolicy::FixedWindow { buffer_ms: 750 };

        assert_eq!(
            candidate_readiness_samples(1_000, 100, max_samples, policy),
            1_500
        );
        assert_eq!(
            candidate_readiness_samples(1_000, 10_000, max_samples, policy),
            1_500
        );
    }

    #[test]
    fn scheduled_candidate_requires_the_full_commit_tail() {
        let buffered_samples = 700 * 2;

        assert_eq!(
            prebuffer_skip_samples(buffered_samples, 0, 600, 1_000, 100),
            Ok(1_200)
        );
        assert!(prebuffer_skip_samples(buffered_samples, 0, 601, 1_000, 100).is_err());
    }

    #[test]
    fn candidate_commit_trims_pcm_that_video_already_passed() {
        let mut buffered = (0..600).map(|value| value as f32).collect::<VecDeque<_>>();
        trim_prebuffer_to_position(&mut buffered, 1_000, 1_200, 1_000, 50).unwrap();
        assert_eq!(buffered.len(), 200);
        assert_eq!(buffered.front(), Some(&400.0));

        // 还剩不到最低 50ms 时才拒绝提交；耗时超过 50ms 本身不再视为过期。
        let mut insufficient = (0..600).map(|value| value as f32).collect::<VecDeque<_>>();
        assert!(trim_prebuffer_to_position(&mut insufficient, 1_000, 1_251, 1_000, 50).is_err());
    }

    #[test]
    fn readiness_errors_distinguish_timeout_from_ffmpeg_and_redact_paths() {
        let config = AudioMixerConfigContext {
            sample_rate_hz: 44_100,
            prebuffer_ms: 50,
            audio_stream_variant_count: 3,
        };
        let timeout = AudioMixerReadinessError::PreheatTimeout {
            waited_ms: 5_000,
            timeout_ms: 5_000,
            config,
        }
        .to_string();
        assert!(timeout.contains("预热超时"));
        assert!(timeout.contains("已等待 5000ms"));
        assert!(timeout.contains("音频支路 3 条"));

        let path = Path::new(r"C:\Users\private\source.mp4");
        let detail = sanitize_error_detail(
            "Error opening input C:\\Users\\private\\source.mp4: invalid data",
            &[path],
        );
        assert!(!detail.contains("private"));
        assert!(detail.contains("<path>"));

        let ffmpeg = AudioMixerReadinessError::Ffmpeg {
            waited_ms: 42,
            config,
            reason: detail,
        }
        .to_string();
        assert!(ffmpeg.contains("FFmpeg 真实错误"));
        assert!(ffmpeg.contains("已等待 42ms"));
        assert!(!ffmpeg.contains("private"));
    }
}
