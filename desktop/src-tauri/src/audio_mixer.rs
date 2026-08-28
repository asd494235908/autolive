//! PortAudio 的本地 PCM 数据路径。
//!
//! FFmpeg 解码线程只负责把当前音轨转换为交错 f32；有界 channel 将解码和
//! PCM 队列解耦，混音线程执行有限值清理和 true-peak 保护。真正的交叉淡化、
//! PortAudio SPSC 写入和 callback 时间轴由 `audio_cycle_output` 单线程负责。

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use autolive_signalsmith_stretch::{QualityPitchConfig, QualityPitchProcessor, StretchError};

use crate::audio_pcm_effects::{AudioPcmEffectConfig, AudioPcmEffectProcessor};
use crate::background_process::background_command;
use crate::cancellation::CancellationToken;

type DecoderProcessSlot = Arc<Mutex<Option<Child>>>;

const OUTPUT_CHANNELS: usize = 2;
const DECODE_BUFFER_BYTES: usize = 16 * 1024;
const MIX_QUEUE_CAPACITY: usize = 8;
const TRUE_PEAK_DBTP: f32 = -1.5;
const PCM_FRAME_BYTES: usize = std::mem::size_of::<f32>() * OUTPUT_CHANNELS;
const MAX_DECODER_STDERR_BYTES: usize = 16 * 1024;
const FIRST_EMPTY_DECODER_RETRY: Duration = Duration::from_millis(50);
const PCM_STALL_RECOVERY_INTERVAL: Duration = Duration::from_millis(1_500);
const CANCELLATION_CHECK_INTERVAL: Duration = Duration::from_millis(50);
// atempo=s 会把 s 秒输入压成 1 秒输出；按 s×1.1 读取可在任意合法倍速保留 10% PCM 余量。
const REALTIME_READ_RATE_HEADROOM_RATIO: f64 = 1.1;
// 候选需要在有限预算内领先正在播放的画面时钟，但不能无界突发读取。
// 最低 2x；显式变速超过 1x 时再增加 1x 源时间余量，保证经过 atempo 后仍能追赶。
// 500ms 初始突发、有界 channel 和预缓冲上限继续限制内存与 CPU。
const CANDIDATE_MIN_READ_RATE: f64 = 2.0;
// 3 秒周期必须覆盖多支路/特征滤镜冷启动和源文件 EOF 重启；用户素材 30 轮压力测试的
// 最小稳定组合是 3x + 1 秒初始突发，只用于周期 N+1，不扩大普通恢复的 CPU 突发。
const SHORT_CYCLE_MIN_READ_RATE: f64 = 3.0;
const SHORT_CYCLE_INITIAL_BURST_SECONDS: &str = "1.0";
// 候选提交需要 30ms 交叉淡化加 100ms 淡化后连续 PCM。
pub const AUDIO_CROSSFADE_MS: usize = 30;
pub const AUDIO_POST_CROSSFADE_TAIL_MS: usize = 100;
pub const AUDIO_CANDIDATE_COMMIT_TAIL_MS: usize = AUDIO_CROSSFADE_MS + AUDIO_POST_CROSSFADE_TAIL_MS;
const SWITCH_CATCH_UP_MAX_MS: usize = 5_000;

#[derive(Debug, Clone, Copy)]
enum CandidatePrebufferPolicy {
    CatchUp { minimum_ready_ms: usize },
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
    eof_behavior: DecoderEofBehavior,
    finished: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecoderPacing {
    RealTime,
    CatchUp,
    ShortCycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecoderEofBehavior {
    Restart,
    Finish,
}

#[derive(Debug, Clone)]
pub struct AudioMixerTrack {
    samples: Arc<Mutex<VecDeque<f32>>>,
    finished: Arc<AtomicBool>,
}

impl AudioMixerTrack {
    #[cfg(test)]
    pub(crate) fn from_samples(samples: VecDeque<f32>) -> Self {
        Self {
            samples: Arc::new(Mutex::new(samples)),
            finished: Arc::new(AtomicBool::new(false)),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_finite_samples(samples: VecDeque<f32>) -> Self {
        Self {
            samples: Arc::new(Mutex::new(samples)),
            finished: Arc::new(AtomicBool::new(true)),
        }
    }

    pub(crate) fn available_samples(&self) -> usize {
        self.samples
            .lock()
            .map(|samples| samples.len())
            .unwrap_or(0)
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub(crate) fn is_exhausted(&self) -> bool {
        self.is_finished() && self.available_samples() == 0
    }

    pub(crate) fn peak_in_first(&self, sample_count: usize) -> Result<Option<f32>, String> {
        let samples = self
            .samples
            .lock()
            .map_err(|_| "音轨 PCM 缓冲锁已损坏".to_owned())?;
        if sample_count == 0 || samples.is_empty() {
            return Ok(None);
        }
        Ok(Some(
            samples
                .iter()
                .take(sample_count)
                .filter(|sample| sample.is_finite())
                .map(|sample| sample.abs())
                .fold(0.0_f32, f32::max),
        ))
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
struct AudioMixerConfig {
    sample_rate_hz: u32,
    prebuffer_ms: usize,
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
    config: AudioMixerConfig,
    ready: Arc<AtomicBool>,
    prebuffer: Arc<Mutex<VecDeque<f32>>>,
    finished: Arc<AtomicBool>,
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
        seek_position_ms: u64,
        timeline_start_position_ms: u64,
        filter_graph: Option<String>,
        quality_pitch: Option<QualityPitchConfig>,
        pcm_effects: Option<AudioPcmEffectConfig>,
        ambient_source_path: Option<PathBuf>,
        playback_rate: f64,
        minimum_ready_ms: usize,
        prebuffer_started_at: Instant,
    ) -> Result<Self, String> {
        Self::start_catch_up_candidate(
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            seek_position_ms,
            timeline_start_position_ms,
            filter_graph,
            quality_pitch,
            pcm_effects,
            ambient_source_path,
            DecoderPacing::CatchUp,
            playback_rate,
            minimum_ready_ms,
            prebuffer_started_at,
        )
    }

    /// 为 3–5 秒普通声音周期预热 N+1；使用经素材压力验证的最小额外突发，
    /// 但保持与普通 CatchUp 相同的动态水位、容量和取消边界。
    #[allow(clippy::too_many_arguments)]
    pub fn start_short_cycle_candidate_with_filter_and_variant_count(
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        sample_rate_hz: u32,
        seek_position_ms: u64,
        timeline_start_position_ms: u64,
        filter_graph: Option<String>,
        quality_pitch: Option<QualityPitchConfig>,
        pcm_effects: Option<AudioPcmEffectConfig>,
        ambient_source_path: Option<PathBuf>,
        playback_rate: f64,
        minimum_ready_ms: usize,
        prebuffer_started_at: Instant,
    ) -> Result<Self, String> {
        Self::start_catch_up_candidate(
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            seek_position_ms,
            timeline_start_position_ms,
            filter_graph,
            quality_pitch,
            pcm_effects,
            ambient_source_path,
            DecoderPacing::ShortCycle,
            playback_rate,
            minimum_ready_ms,
            prebuffer_started_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn start_catch_up_candidate(
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        sample_rate_hz: u32,
        seek_position_ms: u64,
        timeline_start_position_ms: u64,
        filter_graph: Option<String>,
        quality_pitch: Option<QualityPitchConfig>,
        pcm_effects: Option<AudioPcmEffectConfig>,
        ambient_source_path: Option<PathBuf>,
        decoder_pacing: DecoderPacing,
        playback_rate: f64,
        minimum_ready_ms: usize,
        prebuffer_started_at: Instant,
    ) -> Result<Self, String> {
        let minimum_ready_ms = minimum_ready_ms.max(AUDIO_CANDIDATE_COMMIT_TAIL_MS);
        Self::start_internal(
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            seek_position_ms,
            timeline_start_position_ms,
            filter_graph,
            quality_pitch,
            pcm_effects,
            ambient_source_path,
            decoder_pacing,
            DecoderEofBehavior::Restart,
            playback_rate,
            CandidatePrebufferPolicy::CatchUp { minimum_ready_ms },
            minimum_ready_ms,
            prebuffer_started_at,
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
        quality_pitch: Option<QualityPitchConfig>,
        pcm_effects: Option<AudioPcmEffectConfig>,
        ambient_source_path: Option<PathBuf>,
        playback_rate: f64,
        buffer_ms: usize,
        minimum_commit_tail_ms: usize,
        prebuffer_started_at: Instant,
    ) -> Result<Self, String> {
        Self::start_internal(
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            seek_position_ms,
            timeline_start_position_ms,
            filter_graph,
            quality_pitch,
            pcm_effects,
            ambient_source_path,
            DecoderPacing::RealTime,
            DecoderEofBehavior::Restart,
            playback_rate,
            CandidatePrebufferPolicy::FixedWindow { buffer_ms },
            minimum_commit_tail_ms,
            prebuffer_started_at,
        )
    }

    /// 为只播放一次的音轨准备候选；成功到达 EOF 后不从文件开头重新解码。
    #[allow(clippy::too_many_arguments)]
    pub fn start_finite_scheduled_candidate_with_filter_and_variant_count(
        ffmpeg_path: PathBuf,
        source_path: PathBuf,
        sample_rate_hz: u32,
        seek_position_ms: u64,
        timeline_start_position_ms: u64,
        filter_graph: Option<String>,
        quality_pitch: Option<QualityPitchConfig>,
        pcm_effects: Option<AudioPcmEffectConfig>,
        ambient_source_path: Option<PathBuf>,
        playback_rate: f64,
        buffer_ms: usize,
        minimum_commit_tail_ms: usize,
        prebuffer_started_at: Instant,
    ) -> Result<Self, String> {
        Self::start_internal(
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            seek_position_ms,
            timeline_start_position_ms,
            filter_graph,
            quality_pitch,
            pcm_effects,
            ambient_source_path,
            DecoderPacing::RealTime,
            DecoderEofBehavior::Finish,
            playback_rate,
            CandidatePrebufferPolicy::FixedWindow { buffer_ms },
            minimum_commit_tail_ms,
            prebuffer_started_at,
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
        quality_pitch: Option<QualityPitchConfig>,
        pcm_effects: Option<AudioPcmEffectConfig>,
        ambient_source_path: Option<PathBuf>,
        decoder_pacing: DecoderPacing,
        eof_behavior: DecoderEofBehavior,
        playback_rate: f64,
        candidate_prebuffer_policy: CandidatePrebufferPolicy,
        minimum_commit_tail_ms: usize,
        prebuffer_started_at: Instant,
    ) -> Result<Self, String> {
        validate_audio_source(&ffmpeg_path, &source_path)?;
        if let Some(path) = ambient_source_path.as_deref() {
            validate_auxiliary_audio_source(path)?;
        }
        let sample_rate_hz = match sample_rate_hz {
            44_100 | 48_000 => sample_rate_hz,
            _ => autolive_portaudio_output::DEFAULT_SAMPLE_RATE_HZ,
        };
        let cancellation = CancellationToken::new();
        let failure: Arc<Mutex<Option<AudioMixerFailure>>> = Arc::new(Mutex::new(None));
        let decoder_process = Arc::new(Mutex::new(None));
        let ready = Arc::new(AtomicBool::new(false));
        let prebuffer = Arc::new(Mutex::new(VecDeque::new()));
        let finished = Arc::new(AtomicBool::new(false));
        let prebuffer_limit_ms = candidate_prebuffer_limit_ms(candidate_prebuffer_policy);
        let prebuffer_limit_samples =
            stereo_samples_for_ms(sample_rate_hz, prebuffer_limit_ms).max(OUTPUT_CHANNELS);
        let config = AudioMixerConfig {
            sample_rate_hz,
            prebuffer_ms: minimum_commit_tail_ms,
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
                    quality_pitch,
                    pcm_effects,
                    ambient_source_path.as_deref(),
                    decoder_process_slot,
                    decoder_pacing,
                    eof_behavior,
                    playback_rate,
                );
            })
            .map_err(|error| format!("启动 FFmpeg 解码线程失败：{error}"))?;

        let mixer_cancellation = cancellation.clone();
        let mixer_failure = Arc::clone(&failure);
        let mixer_ready = Arc::clone(&ready);
        let mixer_prebuffer = Arc::clone(&prebuffer);
        let mixer_finished = Arc::clone(&finished);
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
                    eof_behavior,
                    finished: mixer_finished,
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
            finished,
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
            finished: Arc::clone(&self.finished),
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

fn validate_auxiliary_audio_source(source_path: &Path) -> Result<(), String> {
    const ALLOWED_EXTENSIONS: [&str; 6] = ["mp3", "wav", "m4a", "aac", "ogg", "flac"];
    let extension_allowed = source_path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|extension| ALLOWED_EXTENSIONS.contains(&extension.as_str()));
    if !source_path.is_file() || !extension_allowed {
        return Err("环境声素材必须是可读的 mp3、wav、m4a、aac、ogg 或 flac 文件".to_owned());
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
    quality_pitch: Option<QualityPitchConfig>,
    pcm_effects: Option<AudioPcmEffectConfig>,
    ambient_source_path: Option<&Path>,
    decoder_process: DecoderProcessSlot,
    decoder_pacing: DecoderPacing,
    eof_behavior: DecoderEofBehavior,
    playback_rate: f64,
) {
    let mut quality_pitch = quality_pitch.and_then(|config| {
        QualityPitchProcessor::new(config, sample_rate_hz, OUTPUT_CHANNELS).ok()
    });
    let mut pcm_effects = match pcm_effects
        .map(|config| AudioPcmEffectProcessor::new(config, sample_rate_hz, OUTPUT_CHANNELS))
        .transpose()
    {
        Ok(processor) => processor,
        Err(error) => {
            set_failure(
                failure,
                AudioMixerFailure::Runtime(format!("初始化 PCM 特征效果失败：{error}")),
            );
            return;
        }
    };
    let mut first_loop = true;
    let mut consecutive_empty_loops = 0_usize;
    while !cancellation.is_cancelled() {
        let start_seconds = if first_loop {
            format!("{:.3}", start_position_ms as f64 / 1000.0)
        } else {
            "0".to_owned()
        };
        let mut command = background_command(ffmpeg_path);
        command.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
        match decoder_pacing {
            DecoderPacing::RealTime => {
                let read_rate = format!("{:.3}", realtime_ffmpeg_read_rate(playback_rate));
                command.args(["-readrate", &read_rate, "-readrate_initial_burst", "0.5"]);
            }
            DecoderPacing::CatchUp => {
                let read_rate = format!("{:.3}", candidate_ffmpeg_read_rate(playback_rate));
                command.args(["-readrate", &read_rate, "-readrate_initial_burst", "0.5"]);
            }
            DecoderPacing::ShortCycle => {
                let read_rate = format!("{:.3}", short_cycle_ffmpeg_read_rate(playback_rate));
                command.args([
                    "-readrate",
                    &read_rate,
                    "-readrate_initial_burst",
                    SHORT_CYCLE_INITIAL_BURST_SECONDS,
                ]);
            }
        }
        command
            .args(decoder_input_seek_args(&start_seconds))
            .arg(source_path);
        if let Some(ambient_source_path) = ambient_source_path {
            command
                .args(["-stream_loop", "-1", "-i"])
                .arg(ambient_source_path);
        }
        command
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
            let (samples, disable_quality_pitch) = match quality_pitch.as_mut() {
                Some(processor) => {
                    let processed = processor.process_interleaved(&samples);
                    quality_pitch_output_or_bypass(samples, processed)
                }
                None => (samples, false),
            };
            if disable_quality_pitch {
                quality_pitch = None;
            }
            // Signalsmith 会在前导延迟尚未填满时合法返回空块；空块不是 PCM，
            // 不能送入要求非空输入的 MFCC/SNR 特征处理器。
            if samples.is_empty() {
                continue;
            }
            let samples = match pcm_effects.as_mut() {
                Some(processor) => match processor.process_interleaved(&samples) {
                    Ok(processed) => processed,
                    Err(error) => {
                        set_failure(
                            failure,
                            AudioMixerFailure::Runtime(format!("PCM 特征效果处理失败：{error}")),
                        );
                        terminate_decoder_process(&decoder_process);
                        let _ = join_stderr_reader(stderr_reader);
                        return;
                    }
                },
                None => samples,
            };
            if samples.is_empty() {
                continue;
            }
            let sample_count = samples.len();
            if !send_samples_with_backpressure(&sender, samples) {
                terminate_decoder_process(&decoder_process);
                let _ = join_stderr_reader(stderr_reader);
                return;
            }
            emitted_samples = emitted_samples.saturating_add(sample_count);
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
        if eof_behavior == DecoderEofBehavior::Finish {
            let tail = match quality_pitch.as_mut() {
                Some(processor) => match processor.flush() {
                    Ok(samples) => samples,
                    Err(error) => {
                        set_failure(
                            failure,
                            AudioMixerFailure::Runtime(format!("排空质量变调尾部失败：{error}")),
                        );
                        return;
                    }
                },
                None => Vec::new(),
            };
            if !tail.is_empty() {
                let tail = match pcm_effects.as_mut() {
                    Some(processor) => match processor.process_interleaved(&tail) {
                        Ok(processed) => processed,
                        Err(error) => {
                            set_failure(
                                failure,
                                AudioMixerFailure::Runtime(format!(
                                    "处理质量变调尾部的 PCM 特征效果失败：{error}"
                                )),
                            );
                            return;
                        }
                    },
                    None => tail,
                };
                let sample_count = tail.len();
                if sample_count > 0 {
                    if !send_samples_with_backpressure(&sender, tail) {
                        return;
                    }
                    emitted_samples = emitted_samples.saturating_add(sample_count);
                }
            }
        }
        if emitted_samples == 0 {
            if eof_behavior == DecoderEofBehavior::Finish {
                set_failure(
                    failure,
                    AudioMixerFailure::Ffmpeg(
                        "音频源从指定位置开始没有可播放的音频数据".to_owned(),
                    ),
                );
                return;
            }
            // 从接近 EOF 启动或 DSP 尚在合法预热时，FFmpeg 可以成功退出但没有
            // 可播放 PCM。首次快速回源；连续空轮按真实停产门槛退避，避免重启风暴。
            consecutive_empty_loops = consecutive_empty_loops.saturating_add(1);
            first_loop = false;
            if !wait_with_cancellation(
                cancellation,
                empty_decoder_restart_delay(consecutive_empty_loops),
            ) {
                return;
            }
            continue;
        }
        if eof_behavior == DecoderEofBehavior::Finish {
            return;
        }
        consecutive_empty_loops = 0;
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

fn decoder_input_seek_args(start_seconds: &str) -> [&str; 3] {
    ["-ss", start_seconds, "-i"]
}

fn realtime_ffmpeg_read_rate(playback_rate: f64) -> f64 {
    let playback_rate = if playback_rate.is_finite() && playback_rate > 0.0 {
        playback_rate
    } else {
        1.0
    };
    playback_rate * REALTIME_READ_RATE_HEADROOM_RATIO
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

fn send_samples_with_backpressure(sender: &mpsc::SyncSender<Vec<f32>>, samples: Vec<f32>) -> bool {
    sender.send(samples).is_ok()
}

fn quality_pitch_output_or_bypass(
    original: Vec<f32>,
    processed: Result<Vec<f32>, StretchError>,
) -> (Vec<f32>, bool) {
    match processed {
        Ok(samples) if samples.iter().all(|sample| sample.is_finite()) => (samples, false),
        Ok(_) | Err(_) => (original, true),
    }
}

fn empty_decoder_restart_delay(consecutive_empty_loops: usize) -> Duration {
    if consecutive_empty_loops <= 1 {
        FIRST_EMPTY_DECODER_RETRY
    } else {
        PCM_STALL_RECOVERY_INTERVAL
    }
}

fn wait_with_cancellation(cancellation: &CancellationToken, duration: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < duration {
        if cancellation.is_cancelled() {
            return false;
        }
        thread::sleep(
            duration
                .saturating_sub(started.elapsed())
                .min(CANCELLATION_CHECK_INTERVAL),
        );
    }
    !cancellation.is_cancelled()
}

fn pcm_decode_block_interval(sample_rate_hz: u32) -> Duration {
    let frames_per_block = DECODE_BUFFER_BYTES / PCM_FRAME_BYTES;
    let sample_rate_hz = u64::from(sample_rate_hz.max(1));
    let micros = u64::try_from(frames_per_block)
        .unwrap_or(u64::MAX)
        .saturating_mul(1_000_000)
        .saturating_add(sample_rate_hz.saturating_sub(1))
        .saturating_div(sample_rate_hz)
        .max(1);
    Duration::from_micros(micros)
}

// 音轨处理线程只填充自己的有界 PCM 队列；PortAudio 由 audio_cycle_output 单线程写入。
fn mix_audio_loop(context: AudioMixerLoopContext) {
    let retry_interval = pcm_decode_block_interval(context.sample_rate_hz);
    let mut pending_samples = VecDeque::new();
    let mut received_pcm = false;
    while !context.cancellation.is_cancelled() {
        // 缓冲达到上限后暂停消费，让有界 channel 反压 FFmpeg；不能读取后丢弃，
        // 否则当前轨或候选轨恢复消费时会出现时间跳跃。
        if context
            .prebuffer
            .lock()
            .ok()
            .is_some_and(|buffered| buffered.len() >= context.prebuffer_limit_samples)
        {
            thread::sleep(retry_interval);
            continue;
        }
        if pending_samples.is_empty() {
            let mut samples = match context.receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(samples) => {
                    received_pcm |= !samples.is_empty();
                    samples
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if context.eof_behavior == DecoderEofBehavior::Finish {
                        if received_pcm {
                            context.ready.store(true, Ordering::Release);
                            context.finished.store(true, Ordering::Release);
                        } else if !context.cancellation.is_cancelled() {
                            set_failure(
                                &context.failure,
                                AudioMixerFailure::Runtime(
                                    "有限音轨结束时没有可播放的 PCM".to_owned(),
                                ),
                            );
                        }
                        return;
                    }
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
            pending_samples = VecDeque::from(samples);
        }
        if let Ok(mut buffered) = context.prebuffer.lock() {
            let buffered_samples = append_prebuffer(
                &mut buffered,
                &mut pending_samples,
                context.prebuffer_limit_samples,
            );
            let elapsed_ms =
                usize::try_from(elapsed_ms(context.prebuffer_started_at)).unwrap_or(usize::MAX);
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

fn append_prebuffer(
    buffered: &mut VecDeque<f32>,
    samples: &mut VecDeque<f32>,
    limit_samples: usize,
) -> usize {
    let writable = limit_samples
        .saturating_sub(buffered.len())
        .min(samples.len());
    buffered.extend(samples.drain(..writable));
    buffered.len()
}

fn candidate_required_prebuffer_samples(
    sample_rate_hz: u32,
    elapsed_ms: usize,
    minimum_ready_ms: usize,
    max_samples: usize,
) -> usize {
    let required_ms =
        u64::try_from(elapsed_ms.saturating_add(minimum_ready_ms)).unwrap_or(u64::MAX);
    let required_samples = u64::from(sample_rate_hz)
        .saturating_mul(required_ms)
        .saturating_div(1_000)
        .saturating_mul(OUTPUT_CHANNELS as u64);
    let max_samples = u64::try_from(max_samples).unwrap_or(u64::MAX);
    usize::try_from(required_samples.min(max_samples)).unwrap_or(usize::MAX)
}

fn candidate_prebuffer_limit_ms(policy: CandidatePrebufferPolicy) -> usize {
    match policy {
        CandidatePrebufferPolicy::CatchUp { minimum_ready_ms } => {
            SWITCH_CATCH_UP_MAX_MS.saturating_add(minimum_ready_ms)
        }
        CandidatePrebufferPolicy::FixedWindow { buffer_ms } => buffer_ms,
    }
}

fn short_cycle_ffmpeg_read_rate(playback_rate: f64) -> f64 {
    SHORT_CYCLE_MIN_READ_RATE.max(candidate_ffmpeg_read_rate(playback_rate))
}

fn candidate_readiness_samples(
    sample_rate_hz: u32,
    elapsed_ms: usize,
    max_samples: usize,
    policy: CandidatePrebufferPolicy,
) -> usize {
    match policy {
        CandidatePrebufferPolicy::CatchUp { minimum_ready_ms } => {
            candidate_required_prebuffer_samples(
                sample_rate_hz,
                elapsed_ms,
                minimum_ready_ms,
                max_samples,
            )
        }
        CandidatePrebufferPolicy::FixedWindow { buffer_ms } => {
            stereo_samples_for_ms(sample_rate_hz, buffer_ms).min(max_samples)
        }
    }
}

fn stereo_samples_for_ms(sample_rate_hz: u32, duration_ms: usize) -> usize {
    let duration_ms = u64::try_from(duration_ms).unwrap_or(u64::MAX);
    usize::try_from(
        u64::from(sample_rate_hz)
            .saturating_mul(duration_ms)
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
    let skip_samples = usize::try_from(
        u64::from(sample_rate_hz)
            .saturating_mul(elapsed_ms)
            .saturating_div(1_000)
            .saturating_mul(u64::from(OUTPUT_CHANNELS as u32)),
    )
    .unwrap_or(usize::MAX);
    let minimum_remaining_ms = u64::try_from(minimum_remaining_ms).unwrap_or(u64::MAX);
    let minimum_remaining_samples = usize::try_from(
        u64::from(sample_rate_hz)
            .saturating_mul(minimum_remaining_ms)
            .saturating_div(1_000)
            .saturating_mul(OUTPUT_CHANNELS as u64),
    )
    .unwrap_or(usize::MAX);
    if skip_samples.saturating_add(minimum_remaining_samples) > buffered_samples {
        let buffered_ms = u64::try_from(buffered_samples)
            .unwrap_or(u64::MAX)
            .saturating_mul(1_000)
            .saturating_div(u64::from(sample_rate_hz.max(1)))
            .saturating_div(OUTPUT_CHANNELS as u64);
        let required_ms = elapsed_ms.saturating_add(minimum_remaining_ms);
        let missing_ms = required_ms.saturating_sub(buffered_ms);
        return Err(format!(
            "候选音轨尚未追上画面：起始位置 {start_position_ms}ms，目标位置 {position_ms}ms；需要缓冲 {required_ms}ms（含安全水位 {minimum_remaining_ms}ms），实际缓冲 {buffered_ms}ms，缺少 {missing_ms}ms"
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
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use autolive_signalsmith_stretch::{QualityPitchConfig, StretchError};

    use super::{
        append_prebuffer, candidate_ffmpeg_read_rate, candidate_prebuffer_limit_ms,
        candidate_readiness_samples, candidate_required_prebuffer_samples, decoder_input_seek_args,
        empty_decoder_restart_delay, pcm_decode_block_interval, prebuffer_skip_samples,
        process_audio_bus, quality_pitch_output_or_bypass, realtime_ffmpeg_read_rate,
        sanitize_error_detail, send_samples_with_backpressure, short_cycle_ffmpeg_read_rate,
        stereo_samples_for_ms, take_complete_stereo_samples, trim_prebuffer_to_position,
        AudioMixerTask, AudioMixerTrack, CandidatePrebufferPolicy, AUDIO_CANDIDATE_COMMIT_TAIL_MS,
        SHORT_CYCLE_INITIAL_BURST_SECONDS, SWITCH_CATCH_UP_MAX_MS,
    };
    use crate::audio_pcm_effects::AudioPcmEffectConfig;
    use crate::media_audio_effects::{MfccOperation, MfccRuntimePlan};

    fn generated_user_audio_fixture(ffmpeg: &Path, name: &str) -> (PathBuf, PathBuf) {
        generated_user_audio_fixture_from_lavfi(
            ffmpeg,
            name,
            "sine=frequency=440:sample_rate=44100:duration=1",
        )
    }

    fn generated_user_audio_fixture_from_lavfi(
        ffmpeg: &Path,
        name: &str,
        source: &str,
    ) -> (PathBuf, PathBuf) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "autolive-user-audio-{name}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create user audio fixture directory");
        let path = root.join("selected.wav");
        let status = crate::background_process::background_command(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                source,
                "-ac",
                "2",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&path)
            .status()
            .expect("generate user audio fixture");
        assert!(status.success());
        (root, path)
    }

    fn wait_until_ready(task: &AudioMixerTask, timeout: Duration) -> Result<(), String> {
        let started_at = Instant::now();
        while started_at.elapsed() < timeout {
            if let Some(reason) = task.failure() {
                return Err(reason);
            }
            if task.is_ready() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err("candidate did not produce continuous PCM before timeout".to_owned())
    }

    #[test]
    fn realtime_producer_keeps_bounded_headroom_across_playback_rates() {
        assert_eq!(realtime_ffmpeg_read_rate(1.0), 1.1);
        assert_eq!(realtime_ffmpeg_read_rate(2.0), 2.2);
        assert_eq!(realtime_ffmpeg_read_rate(0.5), 0.55);
        assert_eq!(realtime_ffmpeg_read_rate(f64::NAN), 1.1);
    }

    #[test]
    fn decoder_restarts_at_eof_instead_of_hiding_the_boundary_in_stream_loop() {
        assert_eq!(decoder_input_seek_args("12.345"), ["-ss", "12.345", "-i"]);
    }

    #[test]
    fn quality_pitch_warmup_does_not_send_empty_pcm_to_feature_processing() {
        let Some(ffmpeg) = std::env::var_os("AUTOLIVE_TEST_FFMPEG") else {
            return;
        };
        let ffmpeg = PathBuf::from(ffmpeg);
        let (fixture_root, audio_fixture) = generated_user_audio_fixture(&ffmpeg, "quality-pitch");
        let mut task = AudioMixerTask::start_scheduled_candidate_with_filter_and_variant_count(
            ffmpeg,
            audio_fixture,
            44_100,
            0,
            0,
            None,
            Some(QualityPitchConfig {
                pitch_shift_semitones: -0.018,
                formant_shift_percent: 0.105,
            }),
            Some(AudioPcmEffectConfig {
                mfcc: Some(MfccRuntimePlan {
                    dimensions: 12,
                    shift_percent: -0.16,
                    operation: MfccOperation::ShiftAndReconstruct,
                }),
                snr: None,
            }),
            None,
            1.0,
            130,
            130,
            Instant::now(),
        )
        .expect("candidate should start");

        let result = wait_until_ready(&task, Duration::from_secs(3));
        task.stop_preserving_output();
        let _ = std::fs::remove_dir_all(fixture_root);
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn quality_pitch_error_disables_processor_and_preserves_original_finite_pcm() {
        let original = vec![0.25, -0.25, 0.5, -0.5];
        let (samples, disable_processor) = quality_pitch_output_or_bypass(
            original.clone(),
            Err(StretchError::NativeFailure("test failure")),
        );

        assert_eq!(samples, original);
        assert!(samples.iter().all(|sample| sample.is_finite()));
        assert!(disable_processor);
    }

    #[test]
    fn non_finite_quality_pitch_output_falls_back_without_matching_error_text() {
        let original = vec![0.1, -0.1];
        let (samples, disable_processor) =
            quality_pitch_output_or_bypass(original.clone(), Ok(vec![f32::NAN, 0.0]));

        assert_eq!(samples, original);
        assert!(disable_processor);
    }

    #[test]
    fn finite_quality_pitch_output_keeps_normal_path_enabled() {
        let processed = vec![0.2, -0.2];
        let (samples, disable_processor) =
            quality_pitch_output_or_bypass(vec![0.1, -0.1], Ok(processed.clone()));

        assert_eq!(samples, processed);
        assert!(!disable_processor);
    }

    #[test]
    fn clean_zero_pcm_seek_restarts_from_source_beginning_without_hard_failure() {
        let Some(ffmpeg) = std::env::var_os("AUTOLIVE_TEST_FFMPEG") else {
            return;
        };
        let ffmpeg = PathBuf::from(ffmpeg);
        let (fixture_root, audio_fixture) = generated_user_audio_fixture(&ffmpeg, "seek-restart");
        let mut task = AudioMixerTask::start_scheduled_candidate_with_filter_and_variant_count(
            ffmpeg,
            audio_fixture,
            44_100,
            3_600_000,
            0,
            None,
            None,
            None,
            None,
            1.0,
            130,
            130,
            Instant::now(),
        )
        .expect("candidate should start");

        let result = wait_until_ready(&task, Duration::from_secs(3));
        task.stop_preserving_output();
        let _ = std::fs::remove_dir_all(fixture_root);
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn finite_candidate_accepts_a_short_eof_tail_without_restarting_the_source() {
        let Some(ffmpeg) = std::env::var_os("AUTOLIVE_TEST_FFMPEG") else {
            return;
        };
        let ffmpeg = PathBuf::from(ffmpeg);
        let (fixture_root, audio_fixture) = generated_user_audio_fixture(&ffmpeg, "finite-eof");
        let mut task =
            AudioMixerTask::start_finite_scheduled_candidate_with_filter_and_variant_count(
                ffmpeg,
                audio_fixture,
                44_100,
                0,
                0,
                None,
                None,
                None,
                None,
                1.0,
                1_500,
                1_500,
                Instant::now(),
            )
            .expect("finite candidate should start");

        let result = wait_until_ready(&task, Duration::from_secs(3));
        let track = task.output_track();
        let available_samples = track.available_samples();
        task.stop_preserving_output();
        let _ = std::fs::remove_dir_all(fixture_root);

        assert!(result.is_ok(), "{result:?}");
        assert!(track.is_finished());
        assert!(available_samples > 0);
        assert!(available_samples < stereo_samples_for_ms(44_100, 1_500));
    }

    #[test]
    fn finite_candidate_flushes_quality_pitch_latency_for_a_ten_millisecond_file() {
        let Some(ffmpeg) = std::env::var_os("AUTOLIVE_TEST_FFMPEG") else {
            return;
        };
        let ffmpeg = PathBuf::from(ffmpeg);
        let (fixture_root, audio_fixture) = generated_user_audio_fixture_from_lavfi(
            &ffmpeg,
            "finite-pitch-tail",
            "sine=frequency=440:sample_rate=44100:duration=0.01",
        );
        let mut task =
            AudioMixerTask::start_finite_scheduled_candidate_with_filter_and_variant_count(
                ffmpeg,
                audio_fixture,
                44_100,
                0,
                0,
                None,
                Some(QualityPitchConfig {
                    pitch_shift_semitones: -0.018,
                    formant_shift_percent: 0.105,
                }),
                None,
                None,
                1.0,
                130,
                130,
                Instant::now(),
            )
            .expect("finite quality-pitch candidate should start");

        let result = wait_until_ready(&task, Duration::from_secs(3));
        let track = task.output_track();
        let available_samples = track.available_samples();
        task.stop_preserving_output();
        let _ = std::fs::remove_dir_all(fixture_root);

        assert!(result.is_ok(), "{result:?}");
        assert!(track.is_finished());
        assert!(available_samples > 0);
        assert!(available_samples < stereo_samples_for_ms(44_100, 130));
    }

    #[test]
    fn finite_candidate_marks_finished_after_live_consumer_drains_the_last_pcm() {
        let Some(ffmpeg) = std::env::var_os("AUTOLIVE_TEST_FFMPEG") else {
            return;
        };
        let ffmpeg = PathBuf::from(ffmpeg);
        let (fixture_root, audio_fixture) = generated_user_audio_fixture(&ffmpeg, "finite-drain");
        let mut task =
            AudioMixerTask::start_finite_scheduled_candidate_with_filter_and_variant_count(
                ffmpeg,
                audio_fixture,
                44_100,
                0,
                0,
                None,
                None,
                None,
                None,
                1.0,
                130,
                130,
                Instant::now(),
            )
            .expect("finite candidate should start");
        wait_until_ready(&task, Duration::from_secs(3)).expect("finite candidate should be ready");
        let track = task.output_track();
        let started_at = Instant::now();
        while !track.is_finished() && started_at.elapsed() < Duration::from_secs(3) {
            track
                .take_up_to(stereo_samples_for_ms(44_100, 50))
                .expect("consume finite candidate");
            std::thread::sleep(Duration::from_millis(5));
        }
        let failure = task.failure();
        task.stop_preserving_output();
        let _ = std::fs::remove_dir_all(fixture_root);

        assert!(track.is_finished(), "finite decoder did not publish EOF");
        assert!(
            failure.is_none(),
            "finite decoder failed at EOF: {failure:?}"
        );
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
    fn short_cycle_decoder_uses_the_smallest_measured_stable_burst() {
        assert_eq!(short_cycle_ffmpeg_read_rate(0.5), 3.0);
        assert_eq!(short_cycle_ffmpeg_read_rate(1.0), 3.0);
        assert_eq!(short_cycle_ffmpeg_read_rate(1.5), 3.0);
        assert_eq!(short_cycle_ffmpeg_read_rate(2.0), 3.0);
        assert_eq!(short_cycle_ffmpeg_read_rate(3.0), 4.0);
        assert_eq!(short_cycle_ffmpeg_read_rate(f64::NAN), 3.0);
        assert_eq!(SHORT_CYCLE_INITIAL_BURST_SECONDS, "1.0");
    }

    #[test]
    fn three_second_cycle_needs_catch_up_headroom_before_becoming_ready() {
        let policy = CandidatePrebufferPolicy::CatchUp {
            minimum_ready_ms: AUDIO_CANDIDATE_COMMIT_TAIL_MS,
        };
        let max_buffer_ms = SWITCH_CATCH_UP_MAX_MS + AUDIO_CANDIDATE_COMMIT_TAIL_MS;
        let max_samples = stereo_samples_for_ms(48_000, max_buffer_ms);

        assert_eq!(
            candidate_readiness_samples(48_000, 0, max_samples, policy),
            stereo_samples_for_ms(48_000, AUDIO_CANDIDATE_COMMIT_TAIL_MS)
        );
        assert_eq!(
            candidate_readiness_samples(48_000, 3_000, max_samples, policy),
            stereo_samples_for_ms(48_000, 3_000 + AUDIO_CANDIDATE_COMMIT_TAIL_MS)
        );
        assert_eq!(
            candidate_readiness_samples(48_000, 5_000, max_samples, policy),
            max_samples
        );
        assert_eq!(candidate_prebuffer_limit_ms(policy), max_buffer_ms);
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
    fn decoder_blocked_send_exits_when_receiver_is_dropped() {
        let (sender, receiver) = std::sync::mpsc::sync_channel(0);
        let thread =
            std::thread::spawn(move || send_samples_with_backpressure(&sender, vec![0.0, 0.0]));

        drop(receiver);
        assert!(!thread.join().unwrap());
    }

    #[test]
    fn decoder_and_empty_output_retries_use_audio_scale_intervals() {
        assert_eq!(pcm_decode_block_interval(48_000).as_micros(), 42_667);
        assert_eq!(empty_decoder_restart_delay(1), Duration::from_millis(50));
        assert_eq!(empty_decoder_restart_delay(2), Duration::from_millis(1_500));
        assert_eq!(
            empty_decoder_restart_delay(usize::MAX),
            Duration::from_millis(1_500)
        );
    }

    #[test]
    fn output_track_is_consumed_only_through_explicit_take_operations() {
        let track = AudioMixerTrack::from_samples(VecDeque::from([1.0, 2.0, 3.0, 4.0]));

        assert_eq!(track.available_samples(), 4);
        assert_eq!(track.take_exact(6).unwrap(), None);
        assert_eq!(track.take_exact(2).unwrap(), Some(vec![1.0, 2.0]));
        assert_eq!(track.take_up_to(8).unwrap(), vec![3.0, 4.0]);
        assert_eq!(track.available_samples(), 0);
    }

    #[test]
    fn finite_output_track_reports_exhaustion_only_after_its_tail_is_consumed() {
        let track = AudioMixerTrack::from_finite_samples(VecDeque::from([1.0, 1.0]));

        assert!(track.is_finished());
        assert!(!track.is_exhausted());
        assert_eq!(track.take_up_to(2).unwrap(), vec![1.0, 1.0]);
        assert!(track.is_exhausted());
    }

    #[test]
    fn candidate_prebuffer_catches_up_slow_filter_start_and_stays_bounded() {
        let mut buffered = VecDeque::new();
        let mut pending = VecDeque::from([1.0, 2.0]);
        assert_eq!(append_prebuffer(&mut buffered, &mut pending, 4), 2);
        assert!(pending.is_empty());

        pending.extend([3.0, 4.0, 5.0, 6.0]);
        assert_eq!(append_prebuffer(&mut buffered, &mut pending, 4), 4);
        assert_eq!(buffered, VecDeque::from([1.0, 2.0, 3.0, 4.0]));
        assert_eq!(pending, VecDeque::from([5.0, 6.0]));

        buffered.drain(..2);
        assert_eq!(append_prebuffer(&mut buffered, &mut pending, 4), 4);
        assert_eq!(buffered, VecDeque::from([3.0, 4.0, 5.0, 6.0]));
        assert!(pending.is_empty());

        // 1kHz 双声道、滤镜启动耗时 2 秒时，候选必须保留 2.13 秒 PCM，
        // 覆盖 30ms 交叉淡化和其后的 100ms 连续尾部。
        assert_eq!(
            candidate_required_prebuffer_samples(1_000, 2_000, 130, 10_100),
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
    fn catch_up_candidate_readiness_uses_the_dynamic_playback_watermark() {
        let policy = CandidatePrebufferPolicy::CatchUp {
            minimum_ready_ms: 200,
        };

        // 1kHz 双声道，已等待 2599ms：应覆盖等待时间加 200ms 动态水位。
        assert_eq!(
            candidate_readiness_samples(1_000, 2_599, 20_000, policy),
            5_598
        );
    }

    #[test]
    fn catch_up_capacity_covers_the_full_timeout_and_dynamic_watermark() {
        assert_eq!(
            candidate_prebuffer_limit_ms(CandidatePrebufferPolicy::CatchUp {
                minimum_ready_ms: 500,
            }),
            5_500
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
    fn candidate_coverage_error_reports_required_actual_and_missing_duration() {
        let error = prebuffer_skip_samples(2_799 * 2, 33_054, 35_721, 1_000, 200)
            .expect_err("候选只剩 132ms，低于 200ms 水位时必须继续预热");

        assert!(error.contains("需要缓冲 2867ms"));
        assert!(error.contains("实际缓冲 2799ms"));
        assert!(error.contains("缺少 68ms"));
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
    fn decoder_errors_redact_source_paths() {
        let path = Path::new(r"C:\Users\private\source.mp4");
        let detail = sanitize_error_detail(
            "Error opening input C:\\Users\\private\\source.mp4: invalid data",
            &[path],
        );
        assert!(!detail.contains("private"));
        assert!(detail.contains("<path>"));
    }
}
