//! PortAudio 的本地 PCM 数据路径。
//!
//! FFmpeg 解码线程只负责把当前音轨转换为交错 f32；有界 channel 将解码和
//! PCM 队列解耦，混音线程执行有限值清理和 true-peak 保护。真正的交叉淡化、
//! PortAudio SPSC 写入和 callback 时间轴由 `audio_cycle_output` 单线程负责。

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::cancellation::CancellationToken;

type DecoderProcessSlot = Arc<Mutex<Option<Child>>>;

const OUTPUT_CHANNELS: usize = 2;
const DECODE_BUFFER_BYTES: usize = 16 * 1024;
const MIX_QUEUE_CAPACITY: usize = 8;
const OUTPUT_RETRY_INTERVAL: Duration = Duration::from_millis(1);
const TRUE_PEAK_DBTP: f32 = -1.5;
const PCM_FRAME_BYTES: usize = std::mem::size_of::<f32>() * OUTPUT_CHANNELS;
const MAX_DECODER_STDERR_BYTES: usize = 16 * 1024;
const REALTIME_FFMPEG_RATE_ARGS: [&str; 1] = ["-re"];
// 候选需要在有限预算内领先正在播放的画面时钟，但不能无界突发读取。
// 最低 2x；显式变速超过 1x 时再增加 1x 源时间余量，保证经过 atempo 后仍能追赶。
// 500ms 初始突发、有界 channel 和预缓冲上限继续限制内存与 CPU。
const CANDIDATE_MIN_READ_RATE: f64 = 2.0;
// 候选提交需要 30ms 交叉淡化加 100ms 淡化后连续 PCM。
pub const AUDIO_CROSSFADE_MS: usize = 30;
pub const AUDIO_POST_CROSSFADE_TAIL_MS: usize = 100;
pub const AUDIO_CANDIDATE_COMMIT_TAIL_MS: usize = AUDIO_CROSSFADE_MS + AUDIO_POST_CROSSFADE_TAIL_MS;
const SWITCH_CATCH_UP_MAX_MS: usize = 5_000;

#[derive(Debug, Clone, Copy)]
enum CandidatePrebufferPolicy {
    CatchUp,
    FixedWindow { buffer_ms: usize },
}

struct AudioMixerLoopContext {
    cancellation: CancellationToken,
    failure: Arc<Mutex<Option<AudioMixerFailure>>>,
    receiver: mpsc::Receiver<Vec<f32>>,
    ready: Arc<AtomicBool>,
    prebuffer: Arc<Mutex<VecDeque<f32>>>,
    prebuffer_limit_samples: usize,
    sample_rate_hz: u32,
    prebuffer_started_at: Instant,
    candidate_prebuffer_policy: CandidatePrebufferPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecoderPacing {
    RealTime,
    CatchUp,
}

#[derive(Debug, Clone)]
pub struct AudioMixerTrack {
    samples: Arc<Mutex<VecDeque<f32>>>,
}

impl AudioMixerTrack {
    pub(crate) fn available_samples(&self) -> usize {
        self.samples
            .lock()
            .map(|samples| samples.len())
            .unwrap_or(0)
    }

    pub(crate) fn take_exact(&self, sample_count: usize) -> Result<Option<Vec<f32>>, String> {
        let mut samples = self
            .samples
            .lock()
            .map_err(|_| "音轨 PCM 缓冲锁已损坏".to_owned())?;
        if samples.len() < sample_count {
            return Ok(None);
        }
        Ok(Some(samples.drain(..sample_count).collect()))
    }

    pub(crate) fn take_up_to(&self, sample_count: usize) -> Result<Vec<f32>, String> {
        let mut samples = self
            .samples
            .lock()
            .map_err(|_| "音轨 PCM 缓冲锁已损坏".to_owned())?;
        let take = samples.len().min(sample_count);
        Ok(samples.drain(..take).collect())
    }
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
}

impl AudioMixerFailure {
    fn message(&self) -> &str {
        match self {
            Self::Ffmpeg(message) | Self::Runtime(message) => message,
        }
    }
}

#[derive(Debug)]
pub struct AudioMixerTask {
    cancellation: CancellationToken,
    decoder_handle: Option<JoinHandle<()>>,
    mixer_handle: Option<JoinHandle<()>>,
    decoder_process: DecoderProcessSlot,
    failure: Arc<Mutex<Option<AudioMixerFailure>>>,
    config: AudioMixerConfigContext,
    ready: Arc<AtomicBool>,
    prebuffer: Arc<Mutex<VecDeque<f32>>>,
    start_position_ms: u64,
}

impl AudioMixerTask {
    /// 启动待切换音轨，但先把首段 PCM 以受控追赶速率预热到内存，不接管当前硬件出口。
    /// 候选最低使用 2x 读取速率；显式加速时增加有界余量，并保留 500ms 初始突发，
    /// 避免经过 `atempo` 后退化为等速读取而无法追上画面时钟。
    #[allow(clippy::too_many_arguments)]
    pub fn start_candidate_with_filter_and_variant_count(
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        sample_rate_hz: u32,
        start_position_ms: u64,
        filter_graph: Option<String>,
        audio_stream_variant_count: usize,
        playback_rate: f64,
    ) -> Result<Self, String> {
        Self::start_internal(
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            start_position_ms,
            start_position_ms,
            filter_graph,
            audio_stream_variant_count,
            DecoderPacing::CatchUp,
            playback_rate,
            CandidatePrebufferPolicy::CatchUp,
            AUDIO_CANDIDATE_COMMIT_TAIL_MS,
        )
    }

    /// 为未来媒体时间窗准备候选。`seek_position_ms` 是源文件内位置，
    /// `timeline_start_position_ms` 是跨循环的绝对媒体位置。
    #[allow(clippy::too_many_arguments)]
    pub fn start_scheduled_candidate_with_filter_and_variant_count(
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
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            seek_position_ms,
            timeline_start_position_ms,
            filter_graph,
            audio_stream_variant_count,
            DecoderPacing::RealTime,
            playback_rate,
            CandidatePrebufferPolicy::FixedWindow { buffer_ms },
            minimum_commit_tail_ms,
        )
    }

    // 线程启动边界显式传递所有权；为减少参数数目包装一次性配置对象反而会隐藏生命周期。
    #[allow(clippy::too_many_arguments)]
    fn start_internal(
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        sample_rate_hz: u32,
        seek_position_ms: u64,
        timeline_start_position_ms: u64,
        filter_graph: Option<String>,
        audio_stream_variant_count: usize,
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
        let ready = Arc::new(AtomicBool::new(false));
        let prebuffer = Arc::new(Mutex::new(VecDeque::new()));
        let prebuffer_limit_ms = match candidate_prebuffer_policy {
            CandidatePrebufferPolicy::CatchUp => {
                SWITCH_CATCH_UP_MAX_MS + AUDIO_CANDIDATE_COMMIT_TAIL_MS
            }
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
        let mixer_ready = Arc::clone(&ready);
        let mixer_prebuffer = Arc::clone(&prebuffer);
        let mixer_handle = match thread::Builder::new()
            .name("autolive-audio-mixer".to_owned())
            .spawn(move || {
                mix_audio_loop(AudioMixerLoopContext {
                    cancellation: mixer_cancellation,
                    failure: mixer_failure,
                    receiver,
                    ready: mixer_ready,
                    prebuffer: mixer_prebuffer,
                    prebuffer_limit_samples,
                    sample_rate_hz,
                    prebuffer_started_at,
                    candidate_prebuffer_policy,
                });
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
            failure,
            config,
            ready,
            prebuffer,
            start_position_ms: timeline_start_position_ms,
        })
    }

    pub fn stop(&mut self) {
        self.stop_inner();
    }

    /// 解码任务不拥有硬件环缓；兼容调用统一走同一幂等停止路径。
    pub fn stop_preserving_output(&mut self) {
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        self.cancellation.cancel();
        terminate_decoder_process(&self.decoder_process);
        if let Some(handle) = self.mixer_handle.take() {
            join_thread(handle, "音频混音");
        }
        if let Some(handle) = self.decoder_handle.take() {
            join_thread(handle, "FFmpeg 解码");
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

    /// 丢弃目标位置之前的 PCM，并保留 30ms 淡化与其后 100ms 连续尾部。
    /// 本任务不直接写 PortAudio；真正的首次预填或交叉淡化由输出线程完成。
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
            self.config.prebuffer_ms,
        )?;
        Ok(())
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

    pub fn output_track(&self) -> AudioMixerTrack {
        AudioMixerTrack {
            samples: Arc::clone(&self.prebuffer),
        }
    }

    /// 返回当前实时 FFmpeg 子进程 PID；任务尚未生成或已经退出时返回 None。
    pub fn ffmpeg_pid(&self) -> Option<u32> {
        self.decoder_process
            .lock()
            .ok()
            .and_then(|process| process.as_ref().map(Child::id))
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
            AudioMixerFailure::Runtime(reason) => AudioMixerReadinessError::Runtime {
                waited_ms,
                config: self.config,
                reason,
            },
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

// 音轨处理线程只填充自己的有界 PCM 队列；PortAudio 由 audio_cycle_output 单线程写入。
fn mix_audio_loop(context: AudioMixerLoopContext) {
    while !context.cancellation.is_cancelled() {
        // 缓冲达到上限后暂停消费，让有界 channel 反压 FFmpeg；不能读取后丢弃，
        // 否则当前轨或候选轨恢复消费时会出现时间跳跃。
        if context
            .prebuffer
            .lock()
            .ok()
            .is_some_and(|buffered| buffered.len() >= context.prebuffer_limit_samples)
        {
            thread::sleep(OUTPUT_RETRY_INTERVAL);
            continue;
        }
        let mut samples = match context.receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(samples) => samples,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if !context.cancellation.is_cancelled() {
                    set_failure(
                        &context.failure,
                        AudioMixerFailure::Runtime("音频解码输入已断开".to_owned()),
                    );
                }
                return;
            }
        };
        process_audio_bus(&mut samples);
        if let Ok(mut buffered) = context.prebuffer.lock() {
            let buffered_samples =
                append_prebuffer(&mut buffered, &samples, context.prebuffer_limit_samples);
            let elapsed_ms = elapsed_ms(context.prebuffer_started_at) as usize;
            let required_samples = candidate_readiness_samples(
                context.sample_rate_hz,
                elapsed_ms,
                context.prebuffer_limit_samples,
                context.candidate_prebuffer_policy,
            );
            if buffered_samples >= required_samples {
                context.ready.store(true, Ordering::Release);
            }
        } else {
            set_failure(
                &context.failure,
                AudioMixerFailure::Runtime("音频候选预缓冲锁已损坏".to_owned()),
            );
            return;
        }
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
    let required_ms = elapsed_ms.saturating_add(AUDIO_CANDIDATE_COMMIT_TAIL_MS);
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

fn stereo_samples_for_ms(sample_rate_hz: u32, duration_ms: usize) -> usize {
    usize::try_from(
        u64::from(sample_rate_hz)
            .saturating_mul(duration_ms as u64)
            .saturating_div(1_000)
            .saturating_mul(OUTPUT_CHANNELS as u64),
    )
    .unwrap_or(usize::MAX)
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
    use std::sync::{Arc, Mutex};

    use super::{
        append_prebuffer, candidate_ffmpeg_read_rate, candidate_readiness_samples,
        candidate_required_prebuffer_samples, prebuffer_skip_samples, process_audio_bus,
        sanitize_error_detail, send_samples_with_cancellation, take_complete_stereo_samples,
        trim_prebuffer_to_position, AudioMixerConfigContext, AudioMixerReadinessError,
        AudioMixerTrack, CandidatePrebufferPolicy,
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
    fn output_track_is_consumed_only_through_explicit_take_operations() {
        let samples = Arc::new(Mutex::new(VecDeque::from([1.0, 2.0, 3.0, 4.0])));
        let track = AudioMixerTrack {
            samples: Arc::clone(&samples),
        };

        assert_eq!(track.available_samples(), 4);
        assert_eq!(track.take_exact(6).unwrap(), None);
        assert_eq!(track.take_exact(2).unwrap(), Some(vec![1.0, 2.0]));
        assert_eq!(track.take_up_to(8).unwrap(), vec![3.0, 4.0]);
        assert!(samples.lock().unwrap().is_empty());
    }

    #[test]
    fn candidate_prebuffer_catches_up_slow_filter_start_and_stays_bounded() {
        let mut buffered = VecDeque::new();
        assert_eq!(append_prebuffer(&mut buffered, &[1.0, 2.0], 4), 2);
        assert_eq!(append_prebuffer(&mut buffered, &[3.0, 4.0, 5.0, 6.0], 4), 6);
        assert_eq!(buffered.len(), 6);
        assert_eq!(append_prebuffer(&mut buffered, &[7.0, 8.0, 9.0], 4), 6);
        assert_eq!(buffered.len(), 6);

        // 1kHz 双声道、滤镜启动耗时 2 秒时，候选必须保留 2.13 秒 PCM，
        // 覆盖 30ms 交叉淡化和其后的 100ms 连续尾部。
        assert_eq!(
            candidate_required_prebuffer_samples(1_000, 2_000, 10_100),
            4_260
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
