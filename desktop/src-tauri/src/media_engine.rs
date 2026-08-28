use crate::audio_pcm_effects::AudioPcmEffectConfig;
use crate::background_process::background_command;
use crate::bounded_io::{read_to_end_bounded, BoundedReadError};
use crate::cancellation::CancellationToken;
use crate::errors::FileHashError;
use crate::hashing::hash_file_at_path;
use crate::media_audio_effects::build_offline_audio_effect_plan;
use crate::media_effect_params::{AdvancedEffectParams, AudioEffectParams, VideoEffectParams};
use crate::media_gpu_capabilities::{
    evaluate_ffmpeg_capabilities, ffmpeg_probe_commands, FfmpegGpuCapabilityReport,
    FfmpegProbeResult,
};
use crate::media_library::SUPPORTED_SOURCE_MEDIA_EXTENSIONS;
use crate::media_video_effects::{
    build_media_video_complex_effect_plan, build_media_video_effect_plan,
};
use crate::media_video_gpu_effects::build_gpu83_video_filter;
use autolive_signalsmith_stretch::QualityPitchConfig;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt::{Display, Formatter};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{mpsc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const MIN_TIMEOUT_SECONDS: u64 = 1;
const MAX_TIMEOUT_SECONDS: u64 = 6 * 60 * 60;
const MAX_AUDIO_VARIANTS: usize = 4;
pub const FFMPEG_PATH_ENV: &str = "AUTOLIVE_FFMPEG_PATH";
pub const FFPROBE_PATH_ENV: &str = "AUTOLIVE_FFPROBE_PATH";
const MEDIA_ENGINE_RESOURCE_DIR: &str = "binaries";
const MEDIA_ENGINE_CAPABILITY_PROBE_TIMEOUT_MS: u64 = 10_000;
const FALLBACK_H264_ENCODER: &str = "libopenh264";
const MAX_MEDIA_STDERR_BYTES: usize = 64 * 1024;
const MAX_PROBE_STDOUT_BYTES: usize = 512 * 1024;
const MAX_FFMPEG_PROGRESS_LINE_BYTES: usize = 256;
const MEDIA_RENDER_PROGRESS_POLL_MS: u64 = 1_000;
const MEDIA_RENDER_STALL_SECONDS: u64 = 120;
const DEFAULT_AUDIO_OUTPUT_SAMPLE_RATE_HZ: u32 = 48_000;
const MIN_AUDIO_CONTENT_PEAK_DB: f64 = -80.0;
const MAX_AUDIO_CONTENT_PROBE_ATTEMPTS: usize = 3;
const AUDIO_FINITE_GUARD_FILTER: &str =
    "aeval=exprs=if(isnan(val(0))\\,0\\,if(isinf(val(0))\\,0\\,val(0)))\\|if(isnan(val(1))\\,0\\,if(isinf(val(1))\\,0\\,val(1)))";
// 各机型候选：按常见硬件加速顺序；运行时探测/编码失败自动降级，不绑死某台机器。
// Windows 才试 MediaFoundation；其它平台跳过 h264_mf。
const H264_ENCODER_CANDIDATES_COMMON: &[&str] = &["h264_nvenc", "h264_amf", "h264_qsv"];
const H264_ENCODER_WINDOWS_EXTRA: &[&str] = &["h264_mf"];
// ponytail: 按 ffmpeg 路径缓存首选；失败后清缓存再探。
static SELECTED_H264_ENCODER: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);
static FFMPEG_GPU_CAPABILITIES: Mutex<Option<(PathBuf, FfmpegGpuCapabilityReport)>> =
    Mutex::new(None);

#[derive(Debug)]
struct MediaOutputProgressWatchdog {
    last_output_size: Option<u64>,
    last_progress_at: Instant,
}

#[derive(Debug)]
struct FfmpegProgressParser {
    line: [u8; MAX_FFMPEG_PROGRESS_LINE_BYTES],
    line_len: usize,
    overflowed: bool,
}

impl Default for FfmpegProgressParser {
    fn default() -> Self {
        Self {
            line: [0; MAX_FFMPEG_PROGRESS_LINE_BYTES],
            line_len: 0,
            overflowed: false,
        }
    }
}

impl FfmpegProgressParser {
    fn push(&mut self, bytes: &[u8], mut on_progress: impl FnMut(u64)) {
        for byte in bytes {
            if *byte == b'\n' {
                self.finish_line(&mut on_progress);
                self.line_len = 0;
                self.overflowed = false;
            } else if *byte != b'\r' {
                if self.line_len < self.line.len() {
                    self.line[self.line_len] = *byte;
                    self.line_len += 1;
                } else {
                    self.overflowed = true;
                }
            }
        }
    }

    fn finish(&mut self, mut on_progress: impl FnMut(u64)) {
        self.finish_line(&mut on_progress);
        self.line_len = 0;
        self.overflowed = false;
    }

    fn finish_line(&self, on_progress: &mut impl FnMut(u64)) {
        if !self.overflowed {
            if let Some(value) = parse_ffmpeg_progress_line(&self.line[..self.line_len]) {
                on_progress(value);
            }
        }
    }
}

fn parse_ffmpeg_progress_line(line: &[u8]) -> Option<u64> {
    std::str::from_utf8(line)
        .ok()?
        .trim()
        .strip_prefix("out_time_us=")?
        .parse()
        .ok()
}

fn video_render_progress_percent(out_time_us: u64, duration_ms: Option<u64>) -> Option<u8> {
    let duration_us = u128::from(duration_ms?.checked_mul(1_000)?);
    if duration_us == 0 {
        return None;
    }
    Some(((u128::from(out_time_us) * 100 / duration_us).min(99)) as u8)
}

impl MediaOutputProgressWatchdog {
    fn new(started_at: Instant) -> Self {
        Self {
            last_output_size: None,
            last_progress_at: started_at,
        }
    }

    fn observe(&mut self, observed_at: Instant, output_size: Option<u64>) -> bool {
        let progressed = match (self.last_output_size, output_size) {
            (None, Some(_)) => true,
            (Some(previous), Some(current)) => current != previous,
            _ => false,
        };
        if progressed {
            self.last_output_size = output_size;
            self.last_progress_at = observed_at;
            return false;
        }
        observed_at.saturating_duration_since(self.last_progress_at)
            >= Duration::from_secs(MEDIA_RENDER_STALL_SECONDS)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioStreamFilterPlan {
    pub filter_graph: String,
    pub quality_pitch: Option<QualityPitchConfig>,
    pub pcm_effects: Option<AudioPcmEffectConfig>,
    pub requires_ambient_input: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaEngineStatus {
    pub available: bool,
    pub ffmpeg_version: Option<String>,
    pub ffprobe_version: Option<String>,
    pub gpu_capabilities: Option<FfmpegGpuCapabilityReport>,
    pub reason: Option<String>,
}

pub fn configured_media_engine_paths() -> Result<(PathBuf, PathBuf), MediaEngineError> {
    Ok((
        configured_executable(FFMPEG_PATH_ENV, "ffmpeg")?,
        configured_executable(FFPROBE_PATH_ENV, "ffprobe")?,
    ))
}

pub fn packaged_media_engine_paths(
    verified_target_root: &Path,
) -> Result<(PathBuf, PathBuf), MediaEngineError> {
    let target = target_triple();
    if target == "unsupported" {
        return Err(MediaEngineError::EngineUnavailable {
            reason: "当前平台未提供内置 FFmpeg/FFprobe 构建".to_owned(),
        });
    }
    let extension = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
    Ok((
        verified_target_root
            .join(MEDIA_ENGINE_RESOURCE_DIR)
            .join(format!("ffmpeg{extension}")),
        verified_target_root
            .join(MEDIA_ENGINE_RESOURCE_DIR)
            .join(format!("ffprobe{extension}")),
    ))
}

pub fn configured_media_engine_paths_with_resource_dir(
    verified_target_root: &Path,
) -> Result<(PathBuf, PathBuf), MediaEngineError> {
    if cfg!(debug_assertions)
        && (std::env::var_os(FFMPEG_PATH_ENV).is_some()
            || std::env::var_os(FFPROBE_PATH_ENV).is_some())
    {
        return configured_media_engine_paths();
    }
    packaged_media_engine_paths(verified_target_root)
}

pub fn configured_media_engine_status() -> MediaEngineStatus {
    media_engine_status_for_paths(configured_media_engine_paths())
}

pub fn configured_media_engine_status_with_resource_dir(
    verified_target_root: &Path,
) -> MediaEngineStatus {
    media_engine_status_for_paths(configured_media_engine_paths_with_resource_dir(
        verified_target_root,
    ))
}

fn media_engine_status_for_paths(
    paths: Result<(PathBuf, PathBuf), MediaEngineError>,
) -> MediaEngineStatus {
    let (ffmpeg_path, ffprobe_path) = match paths {
        Ok(paths) => paths,
        Err(error) => {
            return MediaEngineStatus {
                available: false,
                ffmpeg_version: None,
                ffprobe_version: None,
                gpu_capabilities: None,
                reason: Some(error.to_string()),
            }
        }
    };
    match probe_media_engine_with_paths(
        &ffmpeg_path,
        &ffprobe_path,
        MEDIA_ENGINE_CAPABILITY_PROBE_TIMEOUT_MS,
    ) {
        Ok(status) => status,
        Err(error) => MediaEngineStatus {
            available: false,
            ffmpeg_version: None,
            ffprobe_version: None,
            gpu_capabilities: None,
            reason: Some(error.to_string()),
        },
    }
}

pub fn target_triple() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else {
        "unsupported"
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaRenderRequest {
    pub ffmpeg_path: PathBuf,
    pub ffprobe_path: PathBuf,
    pub input_mp4_path: PathBuf,
    /// 源是否包含可展示视频流；纯音频候选不得触发视频编码器探测。
    pub source_has_video: bool,
    /// 经调用方校验的可选环境声音频；存在时作为第二路输入循环到主媒体结束。
    pub ambient_input_path: Option<PathBuf>,
    pub source_duration_ms: Option<u64>,
    /// 候选在源媒体上的起点；自动周期不得从整文件起点重渲染。
    pub source_start_ms: u64,
    /// 候选输出的有界时长，包含计划窗口与有限安全尾部。
    pub output_duration_ms: u64,
    /// 单项池允许主输入在 EOF 后从头继续；多项池必须为 false。
    pub loop_source: bool,
    pub staging_output_path: PathBuf,
    pub output_mp4_path: PathBuf,
    pub video_processing_enabled: bool,
    pub audio_processing_enabled: bool,
    pub source_audio_sample_rate_hz: Option<u32>,
    pub video: VideoEffectParams,
    pub audio: AudioEffectParams,
    /// 多虚拟轨音频参数；空表示使用 `audio`，最多允许四条支路。
    pub audio_variants: Vec<AudioEffectParams>,
    pub advanced: AdvancedEffectParams,
    pub timeout_seconds: u64,
    pub target: MediaRenderTarget,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MediaRenderTarget {
    StandardMp4,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRenderResult {
    pub output_mp4_path: PathBuf,
    pub output_mp4_sha256: String,
    pub output_size_bytes: u64,
    /// 本次闭环实际成功的编码器；视频流 copy/纯音频输出时为 None。
    pub video_encoder: Option<String>,
    /// 本次闭环实际成功的解码路径；不得把 GPU 滤镜或硬件编码反推为硬件解码。
    pub video_decoder: Option<String>,
    /// 本次视频滤镜实际执行位置；不能根据硬件编码器反推滤镜也在 GPU。
    pub video_filter_backend: Option<MediaVideoFilterBackend>,
    /// 本次成功渲染实际进入滤镜图的 UI 字段；请求参数不能充当生效证据。
    pub applied_video_fields: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaVideoFilterBackend {
    VulkanLibplacebo,
    Software,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaEngineError {
    EngineUnavailable {
        reason: String,
    },
    InvalidExecutable {
        path: String,
    },
    InvalidInput {
        path: String,
    },
    InvalidOutputPath {
        path: String,
    },
    InvalidParameters {
        message: String,
    },
    InvalidTimeout {
        seconds: u64,
    },
    OutputConflict {
        path: String,
    },
    SpawnFailed {
        path: String,
        message: String,
    },
    ProbeOutputTooLarge {
        path: String,
        limit_bytes: usize,
    },
    Failed {
        code: Option<i32>,
        stderr: Option<String>,
    },
    Timeout {
        seconds: u64,
    },
    Stalled {
        seconds: u64,
    },
    Cancelled,
    OutputMissing {
        path: String,
    },
    OutputEmpty {
        path: String,
    },
    OutputSilent {
        path: String,
        max_volume_db: String,
    },
    OutputUnreadable {
        path: String,
        message: String,
    },
    AudioContentProbeIncomplete {
        path: String,
    },
    OutputCommitFailed {
        from: String,
        to: String,
    },
    HashFailed {
        path: String,
        message: String,
    },
}

impl Display for MediaEngineError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EngineUnavailable { reason } => write!(formatter, "本地媒体引擎不可用：{reason}"),
            Self::InvalidExecutable { path } => write!(formatter, "媒体引擎可执行文件无效：{path}"),
            Self::InvalidInput { path } => write!(formatter, "媒体输入无效：{path}"),
            Self::InvalidOutputPath { path } => write!(formatter, "媒体输出路径无效：{path}"),
            Self::InvalidParameters { message } => write!(formatter, "媒体参数无效：{message}"),
            Self::InvalidTimeout { seconds } => {
                write!(formatter, "媒体处理超时范围无效：{seconds} 秒")
            }
            Self::OutputConflict { path } => write!(formatter, "媒体输出已存在，拒绝覆盖：{path}"),
            Self::SpawnFailed { path, message } => {
                write!(formatter, "媒体引擎启动失败：{path}；{message}")
            }
            Self::ProbeOutputTooLarge { path, limit_bytes } => {
                write!(
                    formatter,
                    "媒体引擎探测输出超过上限：{path}；limit={limit_bytes} bytes"
                )
            }
            Self::Failed { code, stderr } => {
                write!(
                    formatter,
                    "媒体引擎异常退出：exit_code={code:?}（常见 -22/EINVAL=滤镜参数非法）"
                )?;
                if let Some(detail) = stderr.as_ref().filter(|text| !text.trim().is_empty()) {
                    write!(formatter, "；FFmpeg: {detail}")?;
                }
                Ok(())
            }
            Self::Timeout { seconds } => write!(formatter, "媒体处理超过超时限制：{seconds} 秒"),
            Self::Stalled { seconds } => {
                write!(formatter, "媒体处理连续 {seconds} 秒没有输出进展，已终止")
            }
            Self::Cancelled => formatter.write_str("媒体处理已取消"),
            Self::OutputMissing { path } => write!(formatter, "媒体处理未生成输出：{path}"),
            Self::OutputEmpty { path } => write!(formatter, "媒体处理输出为空：{path}"),
            Self::OutputSilent {
                path,
                max_volume_db,
            } => write!(
                formatter,
                "媒体处理输出音频疑似静音：{path}；max_volume={max_volume_db} dB"
            ),
            Self::OutputUnreadable { path, message } => {
                write!(formatter, "媒体处理输出不可读：{path}；{message}")
            }
            Self::AudioContentProbeIncomplete { path } => write!(
                formatter,
                "媒体处理输出不可读：{path}；volumedetect 未返回有效 max_volume"
            ),
            Self::OutputCommitFailed { from, to } => {
                write!(formatter, "媒体输出提交失败：{from} -> {to}")
            }
            Self::HashFailed { path, message } => {
                write!(formatter, "媒体输出 SHA-256 计算失败：{path}；{message}")
            }
        }
    }
}

impl std::error::Error for MediaEngineError {}

pub fn probe_media_engine_with_paths(
    ffmpeg_path: &Path,
    ffprobe_path: &Path,
    timeout_ms: u64,
) -> Result<MediaEngineStatus, MediaEngineError> {
    let ffmpeg_version = probe_version(ffmpeg_path, timeout_ms)?;
    let ffprobe_version = probe_version(ffprobe_path, timeout_ms)?;
    Ok(MediaEngineStatus {
        available: true,
        ffmpeg_version: Some(ffmpeg_version),
        ffprobe_version: Some(ffprobe_version),
        gpu_capabilities: Some(probe_ffmpeg_gpu_capabilities(ffmpeg_path)),
        reason: None,
    })
}

/// 用真实三帧闭环探测选择 GPU/CPU 回退；结果按已校验的 FFmpeg 路径缓存。
pub fn probe_ffmpeg_gpu_capabilities(ffmpeg_path: &Path) -> FfmpegGpuCapabilityReport {
    if let Ok(cache) = FFMPEG_GPU_CAPABILITIES.lock() {
        if let Some((path, report)) = cache.as_ref() {
            if path == ffmpeg_path {
                return report.clone();
            }
        }
    }
    let results = ffmpeg_probe_commands()
        .into_iter()
        .map(
            |probe| match run_command_with_timeout(ffmpeg_path, probe.args, 8_000) {
                Ok((status, _)) if status.success() => FfmpegProbeResult::available(probe.kind),
                Ok((status, _)) => FfmpegProbeResult::unavailable(
                    probe.kind,
                    format!("短样本进程退出：exit_code={:?}", status.code()),
                ),
                Err(error) => FfmpegProbeResult::unavailable(probe.kind, error.to_string()),
            },
        )
        .collect::<Vec<_>>();
    let report = evaluate_ffmpeg_capabilities(&results);
    if let Ok(mut cache) = FFMPEG_GPU_CAPABILITIES.lock() {
        *cache = Some((ffmpeg_path.to_path_buf(), report.clone()));
    }
    report
}

/// 在把本地环境声交给长生命周期 PCM 任务前，先解码一个短片段。
/// 这里只验证真实音频流可解码，不把文件内容读入当前进程。
pub fn validate_audio_input_decodable(
    ffmpeg_path: &Path,
    input_path: &Path,
    timeout_ms: u64,
) -> Result<(), MediaEngineError> {
    if !input_path.is_file() {
        return Err(MediaEngineError::InvalidInput {
            path: input_path.display().to_string(),
        });
    }
    let args: Vec<std::ffi::OsString> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostdin".into(),
        "-i".into(),
        input_path.as_os_str().to_owned(),
        "-map".into(),
        "0:a:0".into(),
        "-t".into(),
        "0.250".into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ];
    let (status, _) = run_command_with_timeout(ffmpeg_path, args, timeout_ms.max(1))?;
    if status.success() {
        Ok(())
    } else {
        Err(MediaEngineError::InvalidInput {
            path: input_path.display().to_string(),
        })
    }
}

pub fn build_media_render_args(
    request: &MediaRenderRequest,
) -> Result<Vec<std::ffi::OsString>, MediaEngineError> {
    let preferred = if should_reencode_video(request) {
        Some(select_h264_encoder(&request.ffmpeg_path))
    } else {
        None
    };
    let use_vulkan_filter = gpu_atomic_video_fields(request).is_some()
        && probe_ffmpeg_gpu_capabilities(&request.ffmpeg_path)
            .vulkan_libplacebo
            .available;
    let cpu_fallback;
    let effective_request = if request.video_processing_enabled && !use_vulkan_filter {
        cpu_fallback = cpu_basic_video_request(request);
        &cpu_fallback
    } else {
        request
    };
    build_media_render_args_for_backend(
        effective_request,
        preferred.as_deref(),
        use_vulkan_filter,
        use_vulkan_filter,
    )
}

#[cfg(test)]
fn build_media_render_args_with_video_encoder(
    request: &MediaRenderRequest,
    video_encoder: Option<&str>,
) -> Result<Vec<std::ffi::OsString>, MediaEngineError> {
    build_media_render_args_for_backend(request, video_encoder, false, false)
}

fn build_media_render_args_for_backend(
    request: &MediaRenderRequest,
    video_encoder: Option<&str>,
    use_vulkan_filter: bool,
    use_vulkan_decode: bool,
) -> Result<Vec<std::ffi::OsString>, MediaEngineError> {
    validate_request_shape(request)?;
    let mut args: Vec<std::ffi::OsString> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostats".into(),
        "-nostdin".into(),
        "-y".into(),
    ];
    if use_vulkan_filter {
        args.extend([
            "-init_hw_device".into(),
            "vulkan=autolive_gpu:0".into(),
            "-filter_hw_device".into(),
            "autolive_gpu".into(),
        ]);
    }
    if use_vulkan_decode {
        args.extend([
            "-hwaccel".into(),
            "vulkan".into(),
            "-hwaccel_device".into(),
            "autolive_gpu".into(),
            "-hwaccel_output_format".into(),
            "vulkan".into(),
        ]);
    }
    if !request.source_has_video && request.audio_processing_enabled {
        args.extend(["-filter_complex_threads".into(), "1".into()]);
    }
    if request.loop_source {
        args.extend(["-stream_loop".into(), "-1".into()]);
    }
    args.extend([
        "-ss".into(),
        format_media_millis(request.source_start_ms).into(),
        // The source may be looped forever, while offline audio effects can contain
        // reverse filters that buffer until EOF. Bound the primary input itself so
        // those filters receive EOF instead of growing memory until the job timeout.
        "-t".into(),
        format_media_millis(bounded_primary_input_duration_ms(request)).into(),
        "-i".into(),
        request.input_mp4_path.clone().into_os_string(),
    ]);
    if let Some(ambient_input_path) = &request.ambient_input_path {
        args.extend([
            "-stream_loop".into(),
            "-1".into(),
            "-i".into(),
            ambient_input_path.clone().into_os_string(),
        ]);
    }
    let filter_option_index = args.len();
    let mut complex_graph_parts = Vec::with_capacity(2);

    if !request.source_has_video {
        // 纯音频候选不声明不存在的视频流，也不触发视频编码器。
    } else if request.video_processing_enabled {
        let filter_plan = if use_vulkan_filter {
            gpu83_video_filter(request, use_vulkan_decode).ok_or_else(|| {
                MediaEngineError::InvalidParameters {
                    message: "当前活动视频参数不能由 Vulkan GPU83 滤镜完整执行".to_owned(),
                }
            })?
        } else {
            video_filter(&request.video, &request.advanced)?
        };
        if let Some(graph) = filter_plan.complex_graph {
            complex_graph_parts.push(graph);
            args.extend(["-map".into(), "[vout]".into()]);
        } else {
            args.extend([
                "-map".into(),
                "0:v:0?".into(),
                "-vf".into(),
                filter_plan.serial_filter.into(),
            ]);
        }
        if filter_plan.requires_variable_frame_rate {
            args.extend(["-fps_mode".into(), "vfr".into()]);
        }
        let encoder = video_encoder.unwrap_or(FALLBACK_H264_ENCODER);
        args.extend(video_encoder_codec_args(encoder));
    } else if should_reencode_video(request) {
        args.extend(["-map".into(), "0:v:0?".into()]);
        let encoder = video_encoder.unwrap_or(FALLBACK_H264_ENCODER);
        args.extend(video_encoder_codec_args(encoder));
    } else {
        args.extend(["-map".into(), "0:v:0?".into(), "-c:v".into(), "copy".into()]);
    }

    if request.audio_processing_enabled {
        let variants = effective_audio_variants(request);
        let output_sample_rate_hz = effective_audio_output_sample_rate_hz(
            &variants,
            request.source_audio_sample_rate_hz,
            request.audio.sample_rate_hz,
        );
        let mut audio_graph = audio_mix_filter_complex(
            &request.audio,
            &variants,
            request.source_audio_sample_rate_hz,
            Some(output_sample_rate_hz),
            false,
            request.ambient_input_path.is_some(),
        )?;
        let output_label = if request.source_has_video {
            "[aout]"
        } else {
            let duration = format_media_millis(request.output_duration_ms);
            audio_graph.push_str(&format!(
                ";[aout]apad=whole_dur={duration},atrim=duration={duration},asetpts=PTS-STARTPTS[aout_bounded]"
            ));
            "[aout_bounded]"
        };
        complex_graph_parts.push(audio_graph);
        args.extend([
            "-map".into(),
            output_label.into(),
            "-c:a".into(),
            "aac".into(),
            "-threads:a".into(),
            "1".into(),
            "-ar".into(),
            output_sample_rate_hz.to_string().into(),
            "-ac".into(),
            "2".into(),
            "-sample_fmt".into(),
            "fltp".into(),
        ]);
    } else {
        args.extend(["-map".into(), "0:a:0?".into(), "-c:a".into(), "copy".into()]);
    }

    if !complex_graph_parts.is_empty() {
        args.splice(
            filter_option_index..filter_option_index,
            [
                "-filter_complex".into(),
                complex_graph_parts.join(";").into(),
            ],
        );
    }

    if request.audio_processing_enabled {
        args.extend([
            "-b:a".into(),
            format!("{}k", request.audio.output_bitrate_kbps).into(),
        ]);
    }
    args.extend([
        "-t".into(),
        format_media_millis(request.output_duration_ms).into(),
        "-movflags".into(),
        "+faststart".into(),
    ]);
    args.extend([
        "-f".into(),
        "mp4".into(),
        request.staging_output_path.clone().into_os_string(),
    ]);
    Ok(args)
}

fn should_reencode_video(request: &MediaRenderRequest) -> bool {
    request.source_has_video
        && (request.video_processing_enabled || request.audio_processing_enabled)
}

fn format_media_millis(value_ms: u64) -> String {
    format!("{}.{:03}", value_ms / 1_000, value_ms % 1_000)
}

fn bounded_primary_input_duration_ms(request: &MediaRenderRequest) -> u64 {
    if request.source_has_video || !request.audio_processing_enabled {
        return request.output_duration_ms;
    }
    let max_speed = effective_audio_variants(request)
        .iter()
        .map(|audio| audio.playback_speed)
        .fold(1.0_f64, f64::max);
    ((request.output_duration_ms as f64 * max_speed).ceil() as u64).max(1)
}

pub fn render_media(
    request: &MediaRenderRequest,
    cancellation: &CancellationToken,
) -> Result<MediaRenderResult, MediaEngineError> {
    render_media_with_progress(request, cancellation, |_| {})
}

pub fn render_media_with_progress(
    request: &MediaRenderRequest,
    cancellation: &CancellationToken,
    mut on_progress: impl FnMut(u8),
) -> Result<MediaRenderResult, MediaEngineError> {
    validate_request_shape(request)?;
    let render_started_at = Instant::now();
    let render_deadline = media_render_deadline(render_started_at, request.timeout_seconds).ok_or(
        MediaEngineError::InvalidTimeout {
            seconds: request.timeout_seconds,
        },
    )?;
    if request.output_mp4_path.exists() {
        return Err(MediaEngineError::OutputConflict {
            path: request.output_mp4_path.display().to_string(),
        });
    }
    if cancellation.is_cancelled() {
        return Err(MediaEngineError::Cancelled);
    }

    let _ignored = fs::remove_file(&request.staging_output_path);
    let reencode_video = should_reencode_video(request);
    let encoder_attempts = if reencode_video {
        h264_encoder_attempt_order(&request.ffmpeg_path)
    } else {
        vec![FALLBACK_H264_ENCODER.to_owned()]
    };
    let gpu_atomic_fields = gpu_atomic_video_fields(request);
    let use_vulkan_filter = gpu_atomic_fields.is_some()
        && probe_ffmpeg_gpu_capabilities(&request.ffmpeg_path)
            .vulkan_libplacebo
            .available;
    let cpu_fallback_request = request
        .video_processing_enabled
        .then(|| cpu_basic_video_request(request));
    // GPU83 运行时失败也必须给 CPU4 留下真实执行窗口；不能让一次 GPU 初始化/渲染
    // 占满整个 period 预算后才宣布“回退”。能力探测直接判定不可用时 CPU4 仍独占全预算。
    let gpu_phase_deadline = (use_vulkan_filter && cpu_fallback_request.is_some()).then(|| {
        let total_ms = request.timeout_seconds.saturating_mul(1_000);
        render_started_at + Duration::from_millis(total_ms.saturating_mul(2) / 5)
    });
    let mut render_attempts = Vec::new();
    if use_vulkan_filter {
        for encoder in &encoder_attempts {
            render_attempts.push((encoder.clone(), true, true));
            render_attempts.push((encoder.clone(), true, false));
        }
    }
    render_attempts.extend(
        encoder_attempts
            .into_iter()
            .map(|encoder| (encoder, false, false)),
    );
    let mut last_failure: Option<MediaEngineError> = None;
    let mut encoded_ok = false;
    let mut used_video_encoder = None;
    let mut used_video_decoder = None;
    let mut used_video_filter_backend = None;
    let mut used_applied_video_fields = Vec::new();
    let render_attempt_count = render_attempts.len();
    for (attempt_index, (encoder, attempt_vulkan_filter, attempt_vulkan_decode)) in
        render_attempts.into_iter().enumerate()
    {
        if cancellation.is_cancelled() {
            cleanup(&request.staging_output_path);
            return Err(MediaEngineError::Cancelled);
        }
        let attempt_deadline = if attempt_vulkan_filter {
            gpu_phase_deadline.unwrap_or(render_deadline)
        } else {
            render_deadline
        };
        if Instant::now() >= attempt_deadline {
            if attempt_vulkan_filter {
                last_failure = Some(MediaEngineError::Timeout {
                    seconds: request.timeout_seconds,
                });
                continue;
            }
            cleanup(&request.staging_output_path);
            return Err(MediaEngineError::Timeout {
                seconds: request.timeout_seconds,
            });
        }
        let attempt_request = if attempt_vulkan_filter {
            request
        } else {
            cpu_fallback_request.as_ref().unwrap_or(request)
        };
        let mut args = if reencode_video {
            build_media_render_args_for_backend(
                attempt_request,
                Some(&encoder),
                attempt_vulkan_filter,
                attempt_vulkan_decode,
            )?
        } else {
            build_media_render_args_for_backend(request, None, false, false)?
        };
        args.splice(
            0..0,
            ["-progress".into(), "pipe:1".into(), "-nostats".into()],
        );
        let decoder = if attempt_vulkan_decode {
            "vulkan"
        } else {
            "software"
        };
        let attempt_started_at = Instant::now();
        eprintln!(
            "[video-render] stage=attempt_start processed={} encoder={} decoder={} vulkan_filter={} duration_ms={} timeout_s={}",
            request.video_processing_enabled,
            encoder,
            decoder,
            attempt_vulkan_filter,
            request.output_duration_ms,
            request.timeout_seconds,
        );
        let _ignored = fs::remove_file(&request.staging_output_path);
        let mut child = match background_command(&request.ffmpeg_path)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                cleanup(&request.staging_output_path);
                return Err(MediaEngineError::SpawnFailed {
                    path: request.ffmpeg_path.display().to_string(),
                    message: error.to_string(),
                });
            }
        };
        let mut stderr_reader = child
            .stderr
            .take()
            .map(|stderr| thread::spawn(move || read_stderr_tail(stderr)));
        let (progress_sender, progress_receiver) = mpsc::sync_channel(8);
        let mut progress_reader = child
            .stdout
            .take()
            .map(|stdout| thread::spawn(move || read_ffmpeg_progress(stdout, progress_sender)));
        let mut progress_watchdog = MediaOutputProgressWatchdog::new(Instant::now());
        let mut next_progress_check_at = Instant::now();
        let run_result = loop {
            publish_ffmpeg_progress(
                &progress_receiver,
                Some(request.output_duration_ms),
                &mut on_progress,
            );
            if cancellation.is_cancelled() {
                terminate_child(&mut child);
                join_progress_reader(&mut progress_reader);
                let _ = join_stderr_reader(&mut stderr_reader);
                cleanup(&request.staging_output_path);
                return Err(MediaEngineError::Cancelled);
            }
            if Instant::now() >= attempt_deadline {
                eprintln!(
                    "[video-render] stage=attempt_timeout processed={} encoder={} decoder={} vulkan_filter={} duration_ms={} timeout_s={} elapsed_ms={}",
                    request.video_processing_enabled,
                    encoder,
                    decoder,
                    attempt_vulkan_filter,
                    request.output_duration_ms,
                    request.timeout_seconds,
                    attempt_started_at.elapsed().as_millis(),
                );
                terminate_child(&mut child);
                join_progress_reader(&mut progress_reader);
                let _ = join_stderr_reader(&mut stderr_reader);
                cleanup(&request.staging_output_path);
                break Err(MediaEngineError::Timeout {
                    seconds: request.timeout_seconds,
                });
            }
            match child.try_wait() {
                Ok(Some(status)) if status.success() => {
                    join_progress_reader(&mut progress_reader);
                    publish_ffmpeg_progress(
                        &progress_receiver,
                        Some(request.output_duration_ms),
                        &mut on_progress,
                    );
                    let _ = join_stderr_reader(&mut stderr_reader);
                    break Ok(());
                }
                Ok(Some(status)) => {
                    join_progress_reader(&mut progress_reader);
                    let stderr = join_stderr_reader(&mut stderr_reader);
                    cleanup(&request.staging_output_path);
                    break Err(MediaEngineError::Failed {
                        code: status.code(),
                        stderr,
                    });
                }
                Ok(None) => {
                    let now = Instant::now();
                    if now >= next_progress_check_at {
                        let output_size = fs::metadata(&request.staging_output_path)
                            .ok()
                            .map(|metadata| metadata.len());
                        if progress_watchdog.observe(now, output_size) {
                            terminate_child(&mut child);
                            join_progress_reader(&mut progress_reader);
                            let _ = join_stderr_reader(&mut stderr_reader);
                            cleanup(&request.staging_output_path);
                            return Err(MediaEngineError::Stalled {
                                seconds: MEDIA_RENDER_STALL_SECONDS,
                            });
                        }
                        next_progress_check_at =
                            now + Duration::from_millis(MEDIA_RENDER_PROGRESS_POLL_MS);
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                Err(error) => {
                    terminate_child(&mut child);
                    join_progress_reader(&mut progress_reader);
                    let _ = join_stderr_reader(&mut stderr_reader);
                    cleanup(&request.staging_output_path);
                    break Err(MediaEngineError::SpawnFailed {
                        path: request.ffmpeg_path.display().to_string(),
                        message: error.to_string(),
                    });
                }
            }
        };
        match run_result {
            Ok(()) => {
                // 编码成功：缓存此编码器，供本机后续任务复用。
                if reencode_video {
                    if let Ok(mut cache) = SELECTED_H264_ENCODER.lock() {
                        *cache = Some((request.ffmpeg_path.clone(), encoder.clone()));
                    }
                }
                if reencode_video {
                    used_video_encoder = Some(encoder.clone());
                    used_video_decoder = Some(if attempt_vulkan_decode {
                        "vulkan".to_owned()
                    } else {
                        "software".to_owned()
                    });
                }
                if request.video_processing_enabled {
                    used_video_filter_backend = Some(if attempt_vulkan_filter {
                        MediaVideoFilterBackend::VulkanLibplacebo
                    } else {
                        MediaVideoFilterBackend::Software
                    });
                    used_applied_video_fields = if attempt_vulkan_filter {
                        gpu_atomic_fields.clone().unwrap_or_default()
                    } else {
                        applied_video_fields(attempt_request)
                    };
                }
                eprintln!(
                    "[video-render] stage=attempt_succeeded processed={} encoder={} decoder={} backend={:?} applied_fields={} elapsed_ms={}",
                    request.video_processing_enabled,
                    encoder,
                    used_video_decoder.as_deref().unwrap_or("copy"),
                    used_video_filter_backend,
                    used_applied_video_fields.len(),
                    attempt_started_at.elapsed().as_millis(),
                );
                encoded_ok = true;
                break;
            }
            Err(error) => {
                let can_try_next = reencode_video
                    && attempt_index + 1 < render_attempt_count
                    && if attempt_vulkan_filter {
                        matches!(
                            error,
                            MediaEngineError::Failed { .. }
                                | MediaEngineError::Timeout { .. }
                                | MediaEngineError::Stalled { .. }
                        )
                    } else {
                        encoder_failure_allows_retry(&encoder, &error)
                    };
                if !can_try_next {
                    return Err(error);
                }
                // 仅明确的编码器不可用或初始化失败才清缓存并降级到下一项。
                if let Ok(mut cache) = SELECTED_H264_ENCODER.lock() {
                    *cache = None;
                }
                last_failure = Some(error);
            }
        }
    }
    if !encoded_ok {
        return Err(last_failure.unwrap_or(MediaEngineError::Failed {
            code: None,
            stderr: None,
        }));
    }

    let metadata = fs::metadata(&request.staging_output_path).map_err(|_| {
        MediaEngineError::OutputMissing {
            path: request.staging_output_path.display().to_string(),
        }
    })?;
    if metadata.len() == 0 {
        cleanup(&request.staging_output_path);
        return Err(MediaEngineError::OutputEmpty {
            path: request.staging_output_path.display().to_string(),
        });
    }
    let expected_audio_sample_rate_hz = if request.audio_processing_enabled {
        let variants = effective_audio_variants(request);
        Some(effective_audio_output_sample_rate_hz(
            &variants,
            request.source_audio_sample_rate_hz,
            request.audio.sample_rate_hz,
        ))
    } else {
        None
    };
    let probe_timeout_ms = match remaining_deadline_millis(render_deadline, request.timeout_seconds)
    {
        Ok(timeout_ms) => timeout_ms,
        Err(error) => {
            cleanup(&request.staging_output_path);
            return Err(error);
        }
    };
    if let Err(error) = probe_output(
        &request.ffprobe_path,
        &request.staging_output_path,
        probe_timeout_ms,
        expected_audio_sample_rate_hz,
    ) {
        cleanup(&request.staging_output_path);
        return Err(error);
    }
    if request.audio_processing_enabled {
        if let Err(error) = validate_audio_content(
            &request.ffmpeg_path,
            &request.staging_output_path,
            render_deadline,
            request.timeout_seconds,
            cancellation,
        ) {
            cleanup(&request.staging_output_path);
            return Err(error);
        }
    }
    if let Err(error) = remaining_deadline_millis(render_deadline, request.timeout_seconds) {
        cleanup(&request.staging_output_path);
        return Err(error);
    }
    let output_hash = match hash_file_at_path(&request.staging_output_path, cancellation) {
        Ok(hash) => hash,
        Err(FileHashError::Cancelled) => {
            cleanup(&request.staging_output_path);
            return Err(MediaEngineError::Cancelled);
        }
        Err(error) => {
            cleanup(&request.staging_output_path);
            return Err(MediaEngineError::HashFailed {
                path: request.staging_output_path.display().to_string(),
                message: error.to_string(),
            });
        }
    };
    if let Err(error) = remaining_deadline_millis(render_deadline, request.timeout_seconds) {
        cleanup(&request.staging_output_path);
        return Err(error);
    }
    if request.output_mp4_path.exists() {
        cleanup(&request.staging_output_path);
        return Err(MediaEngineError::OutputConflict {
            path: request.output_mp4_path.display().to_string(),
        });
    }
    fs::rename(&request.staging_output_path, &request.output_mp4_path).map_err(|_| {
        cleanup(&request.staging_output_path);
        MediaEngineError::OutputCommitFailed {
            from: request.staging_output_path.display().to_string(),
            to: request.output_mp4_path.display().to_string(),
        }
    })?;
    on_progress(100);
    Ok(MediaRenderResult {
        output_mp4_path: request.output_mp4_path.clone(),
        output_mp4_sha256: output_hash,
        output_size_bytes: metadata.len(),
        video_encoder: used_video_encoder,
        video_decoder: used_video_decoder,
        video_filter_backend: used_video_filter_backend,
        applied_video_fields: used_applied_video_fields,
    })
}

fn gpu_atomic_video_fields(request: &MediaRenderRequest) -> Option<Vec<String>> {
    if !request.video_processing_enabled {
        return None;
    }
    build_gpu83_video_filter(&request.video, &request.advanced, false, None)
        .map(|plan| plan.applied_fields)
}

fn cpu_basic_video_request(request: &MediaRenderRequest) -> MediaRenderRequest {
    let mut fallback = request.clone();
    fallback.video = VideoEffectParams {
        brightness_percent: request.video.brightness_percent,
        contrast_percent: request.video.contrast_percent,
        saturation_percent: request.video.saturation_percent,
        hue_rotation_degrees: request.video.hue_rotation_degrees,
        ..VideoEffectParams::default()
    };
    fallback.advanced = AdvancedEffectParams::default();
    fallback
}

fn applied_video_fields(request: &MediaRenderRequest) -> Vec<String> {
    match build_media_video_effect_plan(&request.video, &request.advanced) {
        Ok(plan) => plan
            .applied_fields
            .into_iter()
            .filter(|field| *field != "advanced.band_weights")
            .map(str::to_owned)
            .collect(),
        Err(never) => match never {},
    }
}

fn media_render_deadline(started_at: Instant, timeout_seconds: u64) -> Option<Instant> {
    started_at.checked_add(Duration::from_secs(timeout_seconds))
}

fn remaining_deadline_millis(
    deadline: Instant,
    timeout_seconds: u64,
) -> Result<u64, MediaEngineError> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(MediaEngineError::Timeout {
            seconds: timeout_seconds,
        })?;
    Ok(u64::try_from(remaining.as_millis())
        .unwrap_or(u64::MAX)
        .max(1))
}

fn encoder_failure_allows_retry(encoder: &str, error: &MediaEngineError) -> bool {
    encoder != FALLBACK_H264_ENCODER && matches!(error, MediaEngineError::Failed { .. })
}

fn validate_request_shape(request: &MediaRenderRequest) -> Result<(), MediaEngineError> {
    for path in [&request.ffmpeg_path, &request.ffprobe_path] {
        if !path.is_file() {
            return Err(MediaEngineError::InvalidExecutable {
                path: path.display().to_string(),
            });
        }
    }
    let input_extension = request
        .input_mp4_path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    if !request.input_mp4_path.is_file()
        || !input_extension
            .as_deref()
            .is_some_and(|value| SUPPORTED_SOURCE_MEDIA_EXTENSIONS.contains(&value))
    {
        return Err(MediaEngineError::InvalidInput {
            path: request.input_mp4_path.display().to_string(),
        });
    }
    if let Some(path) = request
        .ambient_input_path
        .as_ref()
        .filter(|path| !path.is_file())
    {
        return Err(MediaEngineError::InvalidInput {
            path: path.display().to_string(),
        });
    }
    if request.output_duration_ms == 0 || request.output_duration_ms > 120_000 {
        return Err(MediaEngineError::InvalidParameters {
            message: format!(
                "output_duration_ms 必须在 1..=120000 范围内，实际收到 {}",
                request.output_duration_ms
            ),
        });
    }
    let Some(source_duration_ms) = request.source_duration_ms.filter(|duration| *duration > 0)
    else {
        return Err(MediaEngineError::InvalidParameters {
            message: "有界候选需要有效的源媒体时长".to_owned(),
        });
    };
    if request.source_start_ms >= source_duration_ms {
        return Err(MediaEngineError::InvalidParameters {
            message: format!(
                "source_start_ms 必须小于源媒体时长 {source_duration_ms}，实际收到 {}",
                request.source_start_ms
            ),
        });
    }
    let Some(source_end_ms) = request
        .source_start_ms
        .checked_add(request.output_duration_ms)
    else {
        return Err(MediaEngineError::InvalidParameters {
            message: "有界候选的源起点与输出时长相加溢出".to_owned(),
        });
    };
    if !request.loop_source && source_end_ms > source_duration_ms {
        return Err(MediaEngineError::InvalidParameters {
            message: "多项播放池候选不能跨越当前源媒体 EOF".to_owned(),
        });
    }
    let expected_output_extension = if request.source_has_video {
        "mp4"
    } else {
        "m4a"
    };
    for path in [&request.staging_output_path, &request.output_mp4_path] {
        if !path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case(expected_output_extension))
        {
            return Err(MediaEngineError::InvalidOutputPath {
                path: path.display().to_string(),
            });
        }
    }
    if !(MIN_TIMEOUT_SECONDS..=MAX_TIMEOUT_SECONDS).contains(&request.timeout_seconds) {
        return Err(MediaEngineError::InvalidTimeout {
            seconds: request.timeout_seconds,
        });
    }
    if request.video_processing_enabled {
        if !request.source_has_video {
            return Err(MediaEngineError::InvalidParameters {
                message: "纯音频源不能启用视频处理".to_owned(),
            });
        }
        request
            .video
            .validate()
            .map_err(|errors| MediaEngineError::InvalidParameters {
                message: errors
                    .iter()
                    .map(|error| error.message.clone())
                    .collect::<Vec<_>>()
                    .join("；"),
            })?;
        request
            .advanced
            .validate()
            .map_err(|errors| MediaEngineError::InvalidParameters {
                message: errors
                    .iter()
                    .map(|error| error.message.clone())
                    .collect::<Vec<_>>()
                    .join("；"),
            })?;
    }
    if request.audio_variants.len() > MAX_AUDIO_VARIANTS {
        return Err(MediaEngineError::InvalidParameters {
            message: format!(
                "audio_variants 最多允许 {MAX_AUDIO_VARIANTS} 条，实际收到 {} 条",
                request.audio_variants.len()
            ),
        });
    }
    if request.audio_processing_enabled {
        validate_audio_params(&request.audio, "audio")?;
    }
    for (index, audio) in request.audio_variants.iter().enumerate() {
        validate_audio_params(audio, &format!("audio_variants[{index}]"))?;
    }
    if request.staging_output_path == request.output_mp4_path {
        return Err(MediaEngineError::InvalidOutputPath {
            path: request.output_mp4_path.display().to_string(),
        });
    }
    validate_filter_support(request)?;
    Ok(())
}

fn validate_filter_support(request: &MediaRenderRequest) -> Result<(), MediaEngineError> {
    let ambient_input_available = request.ambient_input_path.is_some();
    if request.video_processing_enabled {
        build_media_video_effect_plan(&request.video, &request.advanced).map_err(|error| {
            MediaEngineError::InvalidParameters {
                message: error.to_string(),
            }
        })?;
    }
    if request.audio_processing_enabled {
        validate_audio_filter_support(&request.audio, "audio", false, ambient_input_available)?;
    }
    // variants 是 IPC 输入的一部分；即使当前关闭声音处理，也不能借此绕过
    // 未映射参数校验，避免后续切换开关后把非法预设带入媒体链。
    for (index, audio) in request.audio_variants.iter().enumerate() {
        validate_audio_filter_support(
            audio,
            &format!("audio_variants[{index}]"),
            !request.audio_processing_enabled,
            ambient_input_available,
        )?;
    }
    Ok(())
}

fn validate_audio_params(
    audio: &AudioEffectParams,
    field_prefix: &str,
) -> Result<(), MediaEngineError> {
    audio
        .validate()
        .map_err(|errors| MediaEngineError::InvalidParameters {
            message: errors
                .into_iter()
                .map(|error| {
                    let field = error.field.strip_prefix("audio.").unwrap_or(&error.field);
                    format!("{field_prefix}.{field}: {}", error.message)
                })
                .collect::<Vec<_>>()
                .join("；"),
        })
}

fn validate_audio_filter_support(
    audio: &AudioEffectParams,
    field_prefix: &str,
    _allow_pcm_runtime: bool,
    ambient_input_available: bool,
) -> Result<(), MediaEngineError> {
    let defaults = AudioEffectParams::default();
    let plan = build_offline_audio_effect_plan(audio).map_err(|errors| {
        MediaEngineError::InvalidParameters {
            message: errors
                .into_iter()
                .map(|error| error.message)
                .collect::<Vec<_>>()
                .join("；"),
        }
    })?;
    if plan.ambient_sound_mix.is_some() && !ambient_input_available {
        return Err(MediaEngineError::InvalidParameters {
            message: format!(
                "{field_prefix}.ambient_sound_mix_percent 需要选择并验证真实环境声素材"
            ),
        });
    }
    // MFCC/SNR/共振峰仍是正式待接入字段，不阻断同一候选中已映射的声音效果。
    if audio.current_formant_hz != defaults.current_formant_hz {
        return Err(MediaEngineError::InvalidParameters {
            message: format!(
                "{field_prefix}.current_formant_hz 是最终混音 PCM 的只读测量值，不能作为效果输入"
            ),
        });
    }
    Ok(())
}

fn configured_executable(env_name: &str, command_name: &str) -> Result<PathBuf, MediaEngineError> {
    if let Some(path) = std::env::var_os(env_name).map(PathBuf::from) {
        if path.is_file() {
            return Ok(path);
        }
        return Err(MediaEngineError::InvalidExecutable {
            path: path.display().to_string(),
        });
    }
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&path_var) {
        let candidate = directory.join(command_name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(MediaEngineError::EngineUnavailable {
        reason: format!(
            "未找到 {command_name}，请重新安装当前平台资源或设置开发覆盖变量 {env_name}"
        ),
    })
}

fn probe_version(path: &Path, timeout_ms: u64) -> Result<String, MediaEngineError> {
    if !path.is_file() {
        return Err(MediaEngineError::InvalidExecutable {
            path: path.display().to_string(),
        });
    }
    let (status, stdout) =
        run_command_with_timeout(path, ["-hide_banner", "-version"], timeout_ms)?;
    if !status.success() {
        return Err(MediaEngineError::EngineUnavailable {
            reason: format!("{} 退出码 {:?}", path.display(), status.code()),
        });
    }
    let text = String::from_utf8_lossy(&stdout);
    let version = text.lines().map(str::trim).find(|line| !line.is_empty());
    version
        .map(str::to_owned)
        .ok_or_else(|| MediaEngineError::EngineUnavailable {
            reason: format!(
                "{} 未返回版本信息（超时预算 {}ms）",
                path.display(),
                timeout_ms
            ),
        })
}

fn probe_output(
    path: &Path,
    output: &Path,
    timeout_ms: u64,
    expected_audio_sample_rate_hz: Option<u32>,
) -> Result<(), MediaEngineError> {
    let mut args: Vec<std::ffi::OsString> =
        vec!["-hide_banner".into(), "-v".into(), "error".into()];
    if expected_audio_sample_rate_hz.is_some() {
        args.extend([
            "-select_streams".into(),
            "a:0".into(),
            "-show_entries".into(),
            "stream=codec_type,codec_name,sample_rate,channels,sample_fmt".into(),
            "-of".into(),
            "json".into(),
        ]);
    } else {
        args.extend([
            "-show_entries".into(),
            "format=format_name".into(),
            "-of".into(),
            "default=noprint_wrappers=1:nokey=1".into(),
        ]);
    }
    args.push(output.as_os_str().to_owned());
    let (status, stdout) = run_command_with_timeout(path, args, timeout_ms)?;
    if !status.success() {
        return Err(MediaEngineError::OutputUnreadable {
            path: output.display().to_string(),
            message: format!(
                "ffprobe 退出码 {:?}（剩余超时预算 {}ms）",
                status.code(),
                timeout_ms
            ),
        });
    }
    if let Some(expected_sample_rate_hz) = expected_audio_sample_rate_hz {
        validate_audio_probe_output(&stdout, expected_sample_rate_hz).map_err(|message| {
            MediaEngineError::OutputUnreadable {
                path: output.display().to_string(),
                message,
            }
        })?;
    }
    Ok(())
}

fn validate_audio_content(
    path: &Path,
    output: &Path,
    deadline: Instant,
    timeout_seconds: u64,
    cancellation: &CancellationToken,
) -> Result<(), MediaEngineError> {
    let max_volume_db = probe_audio_content_with_retry(|| {
        probe_audio_content_max_volume(path, output, deadline, timeout_seconds, cancellation)
    })?;
    if max_volume_db <= MIN_AUDIO_CONTENT_PEAK_DB {
        return Err(MediaEngineError::OutputSilent {
            path: output.display().to_string(),
            max_volume_db: format!("{max_volume_db:.2}"),
        });
    }
    Ok(())
}

fn probe_audio_content_max_volume(
    path: &Path,
    output: &Path,
    deadline: Instant,
    timeout_seconds: u64,
    cancellation: &CancellationToken,
) -> Result<f64, MediaEngineError> {
    let args: Vec<std::ffi::OsString> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "info".into(),
        "-nostats".into(),
        "-i".into(),
        output.as_os_str().to_owned(),
        "-vn".into(),
        "-af".into(),
        "volumedetect".into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ];
    let mut child = background_command(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| MediaEngineError::SpawnFailed {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
    let mut stderr_reader = child
        .stderr
        .take()
        .map(|stderr| thread::spawn(move || read_stderr_capture(stderr)));
    let status = loop {
        if cancellation.is_cancelled() {
            terminate_child(&mut child);
            let _ = join_stderr_capture_reader(&mut stderr_reader);
            return Err(MediaEngineError::Cancelled);
        }
        if Instant::now() >= deadline {
            terminate_child(&mut child);
            let _ = join_stderr_capture_reader(&mut stderr_reader);
            return Err(MediaEngineError::Timeout {
                seconds: timeout_seconds,
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                terminate_child(&mut child);
                let _ = join_stderr_capture_reader(&mut stderr_reader);
                return Err(MediaEngineError::SpawnFailed {
                    path: path.display().to_string(),
                    message: error.to_string(),
                });
            }
        }
    };
    let stderr = join_stderr_capture_reader(&mut stderr_reader).unwrap_or_default();
    if !status.success() {
        return Err(MediaEngineError::OutputUnreadable {
            path: output.display().to_string(),
            message: format!(
                "音频内容探测退出码 {:?}：{}",
                status.code(),
                stderr
                    .diagnostic_tail
                    .unwrap_or_else(|| "未返回诊断信息".to_owned())
            ),
        });
    }
    let max_volume_db =
        stderr
            .max_volume_db
            .ok_or_else(|| MediaEngineError::AudioContentProbeIncomplete {
                path: output.display().to_string(),
            })?;
    Ok(max_volume_db)
}

fn probe_audio_content_with_retry(
    mut probe: impl FnMut() -> Result<f64, MediaEngineError>,
) -> Result<f64, MediaEngineError> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        match probe() {
            Err(MediaEngineError::AudioContentProbeIncomplete { .. })
                if attempts < MAX_AUDIO_CONTENT_PROBE_ATTEMPTS => {}
            result => return result,
        }
    }
}

fn parse_max_volume_db(stderr: &str) -> Option<f64> {
    stderr.split("max_volume:").skip(1).find_map(|tail| {
        let value = tail.split_whitespace().next()?;
        value.parse::<f64>().ok()
    })
}

fn validate_audio_probe_output(stdout: &[u8], expected_sample_rate_hz: u32) -> Result<(), String> {
    let document: serde_json::Value = serde_json::from_slice(stdout)
        .map_err(|error| format!("ffprobe 音频流 JSON 无法解析：{error}"))?;
    let stream = document
        .get("streams")
        .and_then(serde_json::Value::as_array)
        .and_then(|streams| streams.first())
        .ok_or_else(|| "输出没有可用音频流".to_owned())?;
    let codec_type = stream
        .get("codec_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if codec_type != "audio" {
        return Err(format!("音频流类型异常：{codec_type}"));
    }
    let codec_name = stream
        .get("codec_name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if codec_name != "aac" {
        return Err(format!("音频编码不是 AAC：{codec_name}"));
    }
    let sample_rate = stream
        .get("sample_rate")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| "音频流缺少有效采样率".to_owned())?;
    if sample_rate != expected_sample_rate_hz {
        return Err(format!(
            "音频采样率不匹配：实际 {sample_rate}Hz，期望 {expected_sample_rate_hz}Hz"
        ));
    }
    let channels = stream
        .get("channels")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    if channels != 2 {
        return Err(format!("音频声道数不是双声道：{channels}"));
    }
    let sample_fmt = stream
        .get("sample_fmt")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if sample_fmt != "fltp" {
        return Err(format!("音频采样格式不是 fltp：{sample_fmt}"));
    }
    Ok(())
}

fn run_command_with_timeout<I, S>(
    path: &Path,
    args: I,
    timeout_ms: u64,
) -> Result<(ExitStatus, Vec<u8>), MediaEngineError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut child = background_command(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| MediaEngineError::SpawnFailed {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
    let Some(stdout) = child.stdout.take() else {
        terminate_child(&mut child);
        return Err(MediaEngineError::SpawnFailed {
            path: path.display().to_string(),
            message: "媒体引擎未提供 stdout 管道".to_owned(),
        });
    };
    let mut stdout_reader = match thread::Builder::new()
        .name("media-probe-stdout".to_owned())
        .spawn(move || read_to_end_bounded(stdout, MAX_PROBE_STDOUT_BYTES))
    {
        Ok(reader) => Some(reader),
        Err(error) => {
            terminate_child(&mut child);
            return Err(MediaEngineError::SpawnFailed {
                path: path.display().to_string(),
                message: format!("启动探测 stdout 读取线程失败：{error}"),
            });
        }
    };
    let mut captured_stdout = None;
    let started = Instant::now();
    let status = loop {
        if stdout_reader
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            match join_probe_stdout(path, &mut stdout_reader) {
                Ok(stdout) => captured_stdout = Some(stdout),
                Err(error) => {
                    terminate_child(&mut child);
                    return Err(error);
                }
            }
        }
        if started.elapsed() >= Duration::from_millis(timeout_ms) {
            terminate_child(&mut child);
            if let Err(error @ MediaEngineError::ProbeOutputTooLarge { .. }) =
                join_probe_stdout(path, &mut stdout_reader)
            {
                return Err(error);
            }
            return Err(MediaEngineError::EngineUnavailable {
                reason: format!("{} 探测超时：{}ms", path.display(), timeout_ms),
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                terminate_child(&mut child);
                let _ = join_probe_stdout(path, &mut stdout_reader);
                return Err(MediaEngineError::SpawnFailed {
                    path: path.display().to_string(),
                    message: error.to_string(),
                });
            }
        }
    };
    let stdout = match captured_stdout {
        Some(stdout) => stdout,
        None => join_probe_stdout(path, &mut stdout_reader)?,
    };
    Ok((status, stdout))
}

fn join_probe_stdout(
    path: &Path,
    reader: &mut Option<thread::JoinHandle<Result<Vec<u8>, BoundedReadError>>>,
) -> Result<Vec<u8>, MediaEngineError> {
    let Some(reader) = reader.take() else {
        return Ok(Vec::new());
    };
    match reader.join() {
        Ok(Ok(stdout)) => Ok(stdout),
        Ok(Err(BoundedReadError::LimitExceeded { limit })) => {
            Err(MediaEngineError::ProbeOutputTooLarge {
                path: path.display().to_string(),
                limit_bytes: limit,
            })
        }
        Ok(Err(BoundedReadError::Io(error))) => Err(MediaEngineError::SpawnFailed {
            path: path.display().to_string(),
            message: format!("读取探测 stdout 失败：{error}"),
        }),
        Err(_) => Err(MediaEngineError::SpawnFailed {
            path: path.display().to_string(),
            message: "读取探测 stdout 的线程异常退出".to_owned(),
        }),
    }
}

/// 选择可用 H.264 编码器（本机探测，不写死硬件）：
/// nvenc → amf → qsv → (Windows) mf → openh264。
pub fn select_h264_encoder(ffmpeg_path: &Path) -> String {
    if let Ok(cache) = SELECTED_H264_ENCODER.lock() {
        if let Some((path, encoder)) = cache.as_ref() {
            if path == ffmpeg_path {
                return encoder.clone();
            }
        }
    }
    let encoder = detect_h264_encoder(ffmpeg_path);
    if let Ok(mut cache) = SELECTED_H264_ENCODER.lock() {
        *cache = Some((ffmpeg_path.to_path_buf(), encoder.clone()));
    }
    encoder
}

fn h264_encoder_candidates() -> Vec<&'static str> {
    let mut candidates = H264_ENCODER_CANDIDATES_COMMON.to_vec();
    if cfg!(windows) {
        candidates.extend_from_slice(H264_ENCODER_WINDOWS_EXTRA);
    }
    candidates.push(FALLBACK_H264_ENCODER);
    candidates
}

/// 渲染时从实测首选开始，只向更低优先级单调降级。
pub fn h264_encoder_attempt_order(ffmpeg_path: &Path) -> Vec<String> {
    let preferred = select_h264_encoder(ffmpeg_path);
    encoder_attempt_order_from_preferred(&preferred)
}

fn encoder_attempt_order_from_preferred(preferred: &str) -> Vec<String> {
    let candidates = h264_encoder_candidates();
    let start = candidates
        .iter()
        .position(|candidate| *candidate == preferred)
        .unwrap_or(0);
    candidates[start..]
        .iter()
        .map(|candidate| (*candidate).to_owned())
        .collect()
}

fn detect_h264_encoder(ffmpeg_path: &Path) -> String {
    let available = list_video_encoders(ffmpeg_path);
    for candidate in h264_encoder_candidates() {
        if !available.iter().any(|name| name == candidate) {
            continue;
        }
        if candidate == FALLBACK_H264_ENCODER {
            return candidate.to_owned();
        }
        if probe_h264_encoder(ffmpeg_path, candidate) {
            return candidate.to_owned();
        }
    }
    FALLBACK_H264_ENCODER.to_owned()
}

fn list_video_encoders(ffmpeg_path: &Path) -> Vec<String> {
    let Ok((status, stdout)) =
        run_command_with_timeout(ffmpeg_path, ["-hide_banner", "-encoders"], 5_000)
    else {
        return Vec::new();
    };
    if !status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&stdout)
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            // 形如 " V....D h264_nvenc  NVIDIA ..."
            if !trimmed.starts_with('V') {
                return None;
            }
            let mut parts = trimmed.split_whitespace();
            let _flags = parts.next()?;
            let name = parts.next()?;
            Some(name.to_owned())
        })
        .collect()
}

fn probe_h264_encoder(ffmpeg_path: &Path, encoder: &str) -> bool {
    let null_output = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let mut args: Vec<std::ffi::OsString> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "color=c=black:s=320x240:r=30:d=0.1,format=yuv420p".into(),
        "-frames:v".into(),
        "3".into(),
        "-an".into(),
    ];
    args.extend(video_encoder_codec_args(encoder));
    args.extend(["-f".into(), "null".into(), null_output.into()]);
    match run_command_with_timeout(ffmpeg_path, args, 8_000) {
        Ok((status, _)) => status.success(),
        Err(_) => false,
    }
}

pub(crate) fn video_encoder_codec_args(encoder: &str) -> Vec<std::ffi::OsString> {
    match encoder {
        // NVIDIA：各代 NVENC 通用 p 系列 preset。
        "h264_nvenc" => vec![
            "-c:v".into(),
            "h264_nvenc".into(),
            "-preset".into(),
            "p4".into(),
            "-tune".into(),
            "ll".into(),
        ],
        // AMD AMF。
        "h264_amf" => vec![
            "-c:v".into(),
            "h264_amf".into(),
            "-quality".into(),
            "speed".into(),
        ],
        // Intel QSV。
        "h264_qsv" => vec![
            "-c:v".into(),
            "h264_qsv".into(),
            "-preset".into(),
            "veryfast".into(),
        ],
        // Windows MediaFoundation：有硬解就加速，没有也能软回退。
        "h264_mf" => vec![
            "-c:v".into(),
            "h264_mf".into(),
            "-hw_encoding".into(),
            "true".into(),
        ],
        // 任意机器最终兜底：OpenH264 软编，限制编码线程以给 WebView/音频输出留出 CPU。
        _ => vec![
            "-c:v".into(),
            FALLBACK_H264_ENCODER.into(),
            "-threads:v".into(),
            media_worker_thread_limit().to_string().into(),
        ],
    }
}

fn media_worker_thread_limit() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .saturating_sub(1)
        .clamp(1, 8)
}

struct VideoFilterPlan {
    serial_filter: String,
    complex_graph: Option<String>,
    requires_variable_frame_rate: bool,
}

fn gpu83_video_filter(
    request: &MediaRenderRequest,
    input_on_vulkan: bool,
) -> Option<VideoFilterPlan> {
    let plan = build_gpu83_video_filter(&request.video, &request.advanced, input_on_vulkan, None)?;
    Some(VideoFilterPlan {
        serial_filter: plan.serial_filter,
        complex_graph: None,
        requires_variable_frame_rate: plan.requires_variable_frame_rate,
    })
}

/// 旧五字段 Vulkan 子集只保留给历史参数构图测试；生产路径统一走 GPU83。
#[cfg(test)]
fn legacy_vulkan_subset_filter(
    video: &VideoEffectParams,
    advanced: &AdvancedEffectParams,
    input_on_vulkan: bool,
) -> Option<VideoFilterPlan> {
    if advanced != &AdvancedEffectParams::default() {
        return None;
    }

    let defaults = VideoEffectParams::default();
    let mut unsupported = video.clone();
    unsupported.brightness_percent = defaults.brightness_percent;
    unsupported.saturation_percent = defaults.saturation_percent;
    unsupported.blur_radius_px = defaults.blur_radius_px;
    unsupported.contrast_percent = defaults.contrast_percent;
    unsupported.hue_rotation_degrees = defaults.hue_rotation_degrees;
    unsupported.horizontal_flip_enabled = defaults.horizontal_flip_enabled;
    unsupported.vertical_flip_enabled = defaults.vertical_flip_enabled;
    if unsupported != defaults {
        return None;
    }

    let mut filters = Vec::new();
    if !input_on_vulkan {
        filters.extend(["format=nv12".to_owned(), "hwupload".to_owned()]);
    }
    filters.push(format!(
            "libplacebo=brightness={:.6}:contrast={:.6}:saturation={:.6}:hue={:.10}:upscaler=bilinear:downscaler=bilinear",
            (video.brightness_percent / 100.0).clamp(-1.0, 1.0),
            (video.contrast_percent / 100.0).clamp(0.0, 16.0),
            (video.saturation_percent / 100.0).clamp(0.0, 16.0),
            video
                .hue_rotation_degrees
                .to_radians()
                .clamp(-std::f64::consts::PI, std::f64::consts::PI),
        ));
    if video.blur_radius_px > 0.0 {
        filters.push(format!(
            "gblur_vulkan=sigma={:.6}",
            video.blur_radius_px.max(0.01)
        ));
    }
    if video.horizontal_flip_enabled {
        filters.push("hflip_vulkan".to_owned());
    }
    if video.vertical_flip_enabled {
        filters.push("vflip_vulkan".to_owned());
    }
    filters.extend(["hwdownload".to_owned(), "format=nv12".to_owned()]);
    Some(VideoFilterPlan {
        serial_filter: filters.join(","),
        complex_graph: None,
        requires_variable_frame_rate: false,
    })
}

fn video_filter(
    video: &VideoEffectParams,
    advanced: &AdvancedEffectParams,
) -> Result<VideoFilterPlan, MediaEngineError> {
    video_filter_with_runtime_controls(video, advanced, false)
}

fn video_filter_with_runtime_controls(
    video: &VideoEffectParams,
    advanced: &AdvancedEffectParams,
    runtime_controls: bool,
) -> Result<VideoFilterPlan, MediaEngineError> {
    // 当前打包 FFmpeg 无 eq/boxblur；用 lutyuv + hue + gblur 等价映射。
    let brightness = (video.brightness_percent / 100.0).clamp(-1.0, 1.0);
    let contrast = (video.contrast_percent / 100.0).clamp(0.0, 2.0);
    let saturation = (video.saturation_percent / 100.0).clamp(0.0, 3.0);
    let luma_filter = if runtime_controls {
        "lutyuv@autolive_luma"
    } else {
        "lutyuv"
    };
    let mut filters = vec![format!(
        "{luma_filter}=y='clip((val-128)*{contrast:.6}+128+{brightness:.6}*128,0,255)'"
    )];
    if runtime_controls
        || (saturation - 1.0).abs() > f64::EPSILON
        || video.hue_rotation_degrees != 0.0
    {
        let color_filter = if runtime_controls {
            "hue@autolive_color"
        } else {
            "hue"
        };
        filters.push(format!(
            "{color_filter}=h={:.6}:s={saturation:.6}",
            video.hue_rotation_degrees
        ));
    }
    if video.highlights_percent.abs() > f64::EPSILON || video.shadows_percent.abs() > f64::EPSILON {
        let highlights = video.highlights_percent / 100.0 * 32.0;
        let shadows = video.shadows_percent / 100.0 * 32.0;
        let curve =
            format!("clip(val+({shadows:.6}*(1-val/255)+{highlights:.6}*(val/255))\\,0\\,255)");
        let red = if video.red_channel_lock_enabled {
            "val".to_owned()
        } else {
            curve.clone()
        };
        filters.push(format!("lutrgb=r='{red}':g='{curve}':b='{curve}'"));
    }
    if video.vignette_percent > f64::EPSILON {
        let angle = video.vignette_percent / 100.0 * std::f64::consts::FRAC_PI_2;
        filters.push(format!("vignette=angle={angle:.8}:eval=init"));
    }
    if video.blur_radius_px > 0.0 {
        filters.push(format!("gblur=sigma={:.6}", video.blur_radius_px.max(0.01)));
    }
    let mut combined_unsharp = 0.0;
    if video.image_repair_enabled && video.image_repair_strength_percent > f64::EPSILON {
        combined_unsharp += video.image_repair_strength_percent / 250.0;
        if video.image_repair_strength_percent > 1.0 {
            let strength = 1.0 + video.image_repair_strength_percent / 25.0;
            filters.push(format!("nlmeans=s={strength:.6}:p=7:r=9"));
        }
    }
    combined_unsharp += (video.sharpen_percent + video.detail_enhancement_percent) / 100.0;
    if combined_unsharp > 0.0 {
        filters.push(format!("unsharp=5:5:{combined_unsharp:.6}"));
    }
    if video.noise_percent > 0.0 {
        // `noise` 的 alls 是整数语义。把产品百分比映射为最小有效强度 1
        // 与确定性稀疏帧概率，避免低小数被量化为 0，也避免连续整帧噪点放大感知。
        let density_threshold = (video.noise_percent * 1_000.0).round() as u64;
        let phase = density_threshold * 7_919 % 100_000;
        filters.push(format!(
            "noise=alls=1:allf=t+u:enable='lt(mod(n*7919+{phase}\\,100000)\\,{density_threshold})'"
        ));
    }
    let offset_x = video.space_x_offset_px.round();
    let offset_y = video.space_y_offset_px.round();
    if offset_x != 0.0 || offset_y != 0.0 {
        let padding_x = offset_x.abs().ceil() as i64 + 1;
        let padding_y = offset_y.abs().ceil() as i64 + 1;
        filters.push(format!(
            "pad=iw+{width}:ih+{height}:{padding_x}+({offset_x:.0}):{padding_y}+({offset_y:.0}):color=black,crop=iw-{width}:ih-{height}:{padding_x}:{padding_y}",
            width = padding_x * 2,
            height = padding_y * 2,
        ));
    }
    if video.pixel_jitter_px > 0.0 {
        let padding = video.pixel_jitter_px.ceil() as i64 + 1;
        filters.push(format!(
            "pad=iw+{size}:ih+{size}:{padding}:{padding}:color=black,crop=iw-{size}:ih-{size}:{padding}+round(sin(n*0.73)*{jitter:.6}):{padding}+round(cos(n*0.91)*{jitter:.6})",
            size = padding * 2,
            jitter = video.pixel_jitter_px,
        ));
    }
    if video.pixel_scale_percent != 100.0 {
        let scale = video.pixel_scale_percent / 100.0;
        filters.push(format!("scale=iw*{scale:.6}:ih*{scale:.6}"));
    }
    if video.horizontal_flip_enabled {
        filters.push("hflip".to_owned());
    }
    if video.vertical_flip_enabled {
        filters.push("vflip".to_owned());
    }
    if video.rotation_degrees.abs() > f64::EPSILON {
        let radians = video.rotation_degrees.to_radians();
        filters.push(format!(
            "rotate=angle={radians:.10}:ow=iw:oh=ih:fillcolor=black"
        ));
    }
    let supplementary = build_media_video_effect_plan(video, advanced).map_err(|error| {
        MediaEngineError::InvalidParameters {
            message: error.to_string(),
        }
    })?;
    filters.extend(supplementary.filters);
    let serial_filter = filters.join(",");
    let complex_graph = build_media_video_complex_effect_plan(&serial_filter, video, advanced)
        .map(|plan| plan.graph);
    Ok(VideoFilterPlan {
        serial_filter,
        complex_graph,
        requires_variable_frame_rate: supplementary.requires_variable_frame_rate,
    })
}

fn effective_audio_variants(request: &MediaRenderRequest) -> Vec<AudioEffectParams> {
    if request.audio_variants.is_empty() {
        vec![request.audio.clone()]
    } else {
        request.audio_variants.clone()
    }
}

fn effective_audio_output_sample_rate_hz(
    variants: &[AudioEffectParams],
    source_sample_rate_hz: Option<u32>,
    preferred_output_sample_rate_hz: Option<u32>,
) -> u32 {
    let requested = preferred_output_sample_rate_hz
        .or_else(|| variants.iter().find_map(|audio| audio.sample_rate_hz))
        .or(source_sample_rate_hz);
    match requested {
        Some(44_100) => 44_100,
        Some(48_000) => 48_000,
        _ => DEFAULT_AUDIO_OUTPUT_SAMPLE_RATE_HZ,
    }
}

/// 单个音频支路的效果链；支路之间的环境噪声混合由
/// `audio_mix_filter_complex` 通过独立 `anoisesrc` 完成。
fn audio_branch_filter(
    audio: &AudioEffectParams,
    source_sample_rate_hz: Option<u32>,
    realtime: bool,
) -> Result<String, MediaEngineError> {
    // 某些容器只声明声道数而没有标准 channel layout（FFmpeg 会显示为
    // `1 channels` 等）。native AAC 无法为这种布局初始化编码器，会返回
    // EINVAL 并导致输出出现 audio:0KiB。每条支路先归一到 AAC 可接受的
    // float/stereo 总线；这也与 PortAudio 的双声道出口保持一致。
    // aeval 将解码器或支路滤镜产生的 NaN/Infinity 转为 0，避免把非法
    // 浮点样本继续送入输出总线/AAC。最终渲染仍由 FFmpeg 成功状态决定。
    Ok(format!(
        "aformat=sample_fmts=fltp:channel_layouts=stereo,{AUDIO_FINITE_GUARD_FILTER},{}",
        audio_filter(audio, source_sample_rate_hz, realtime)?
    ))
}

fn audio_mix_filter_complex(
    main_audio: &AudioEffectParams,
    variants: &[AudioEffectParams],
    source_sample_rate_hz: Option<u32>,
    preferred_output_sample_rate_hz: Option<u32>,
    realtime: bool,
    ambient_input_available: bool,
) -> Result<String, MediaEngineError> {
    if variants.is_empty() {
        return Err(MediaEngineError::InvalidParameters {
            message: "audio_variants 不能为空".to_owned(),
        });
    }
    let k = variants.len();
    let mut parts = Vec::with_capacity(k * 6 + 5);
    let output_sample_rate_hz = effective_audio_output_sample_rate_hz(
        variants,
        source_sample_rate_hz,
        preferred_output_sample_rate_hz,
    );
    let split_labels = (0..k).map(|index| format!("a{index}")).collect::<Vec<_>>();
    parts.push(format!(
        "[0:a:0]asetpts=PTS-STARTPTS,aresample=async=1:first_pts=0,asetpts=N/SR/TB,asplit={k}{}",
        split_labels
            .iter()
            .map(|label| format!("[{label}]"))
            .collect::<String>()
    ));
    let mut mixed_inputs = String::new();
    let mut weights = Vec::with_capacity(k);
    for (index, audio) in variants.iter().enumerate() {
        let branch = audio_branch_filter(audio, source_sample_rate_hz, realtime)?;
        parts.push(format!(
            "[a{index}]{branch},asetpts=PTS-STARTPTS,aresample={output_sample_rate_hz}:async=1:first_pts=0,asetpts=N/SR/TB[processed{index}]"
        ));
        let branch_input = if let Some(mix) = build_offline_audio_effect_plan(audio)
            .map_err(|errors| MediaEngineError::InvalidParameters {
                message: errors
                    .into_iter()
                    .map(|error| error.message)
                    .collect::<Vec<_>>()
                    .join("；"),
            })?
            .dry_wet_mix
        {
            // 额外湿声支路使用两级短反射；0% 时不创建支路，保持既有声音效果不变。
            parts.push(format!(
                "[processed{index}]asplit=2[dry{index}][wet_in{index}];[wet_in{index}]aecho=0.8:0.9:60|120:0.35|0.20[wet{index}];{}",
                mix.ffmpeg_complex_graph_fragment(
                    &format!("dry{index}"),
                    &format!("wet{index}"),
                    &format!("dry_wet{index}"),
                )
            ));
            format!("dry_wet{index}")
        } else {
            format!("processed{index}")
        };
        if audio.environment_noise_percent > f64::EPSILON {
            let ratio = (audio.environment_noise_percent / 100.0).clamp(0.0, 1.0);
            let amplitude = 10_f64
                .powf(audio.environment_noise_dbfs / 20.0)
                .clamp(0.000_001, 1.0);
            let dry_weight = (1.0 - ratio).max(0.0);
            // 噪声源独立于每个支路，并由该支路自己的 amix 真正混入。
            // 主声和生成噪声都从零时间轴开始；shortest 以有限主声裁切无限噪声。
            parts.push(format!(
                "anoisesrc=color=white:amplitude={amplitude:.6}:d=86400,asetpts=N/SR/TB[noise{index}];[{branch_input}][noise{index}]amix=inputs=2:weights={dry_weight:.6} {ratio:.6}:duration=shortest:dropout_transition=0[b{index}]"
            ));
        } else {
            parts.push(format!("[{branch_input}]anull[b{index}]"));
        }
        parts.push(format!(
            "[b{index}]asetpts=PTS-STARTPTS,aresample={output_sample_rate_hz}:async=1:first_pts=0,asetpts=N/SR/TB[mix{index}]"
        ));
        mixed_inputs.push_str(&format!("[mix{index}]"));
        weights.push(format!("{:.6}", 1.0 / k as f64));
    }
    parts.push(format!(
        "{mixed_inputs}amix=inputs={k}:weights={}:duration=shortest:dropout_transition=0[variant_bus]",
        weights.join(" "),
    ));
    let final_input = if let Some(mix) = build_offline_audio_effect_plan(main_audio)
        .map_err(|errors| MediaEngineError::InvalidParameters {
            message: errors
                .into_iter()
                .map(|error| error.message)
                .collect::<Vec<_>>()
                .join("；"),
        })?
        .ambient_sound_mix
    {
        if !ambient_input_available {
            return Err(MediaEngineError::InvalidParameters {
                message: "环境声混合已启用，但没有经校验的真实环境声输入".to_owned(),
            });
        }
        parts.push(format!(
            "[1:a:0]asetpts=PTS-STARTPTS,aformat=sample_fmts=fltp:channel_layouts=stereo,aresample={output_sample_rate_hz}:async=1:first_pts=0,asetpts=N/SR/TB[ambient];{}",
            mix.ffmpeg_complex_graph_fragment("variant_bus", "ambient", "ambient_bus")
        ));
        "ambient_bus"
    } else {
        "variant_bus"
    };
    // 保留源素材相对响度；低感知预设只应用自身的微小增益，不能固定归一到 -16 LUFS。
    // PortAudio 出口继续在最终 PCM 总线上执行 true-peak 保护。
    parts.push(format!(
        "[{final_input}]{AUDIO_FINITE_GUARD_FILTER},highpass=f=50,adenorm=level=-351:type=ac,aresample={output_sample_rate_hz}:async=1:first_pts=0,aformat=sample_fmts=fltp:sample_rates={output_sample_rate_hz}:channel_layouts=stereo[aout]",
    ));
    Ok(parts.join(";"))
}

/// 构建实时音频解码可复用的 FFmpeg 滤镜图。
///
/// 输出标签固定为 `[aout]`，调用方应将其映射到 PCM 输出；滤镜图保证
/// `fltp`、双声道、有限值保护，并将常用输出采样率归一到 44.1kHz/48kHz。
pub fn build_audio_stream_filter_graph(
    audio: &AudioEffectParams,
    audio_variants: &[AudioEffectParams],
    source_audio_sample_rate_hz: Option<u32>,
    output_sample_rate_hz: u32,
) -> Result<AudioStreamFilterPlan, MediaEngineError> {
    build_audio_stream_filter_graph_with_ambient(
        audio,
        audio_variants,
        source_audio_sample_rate_hz,
        output_sample_rate_hz,
        false,
    )
}

pub fn build_audio_stream_filter_graph_with_ambient(
    audio: &AudioEffectParams,
    audio_variants: &[AudioEffectParams],
    source_audio_sample_rate_hz: Option<u32>,
    output_sample_rate_hz: u32,
    ambient_input_available: bool,
) -> Result<AudioStreamFilterPlan, MediaEngineError> {
    validate_audio_params(audio, "audio")?;
    validate_audio_filter_support(audio, "audio", true, ambient_input_available)?;
    if !matches!(output_sample_rate_hz, 44_100 | 48_000) {
        return Err(MediaEngineError::InvalidParameters {
            message: format!(
                "实时音频输出采样率仅支持 44100/48000Hz，收到 {output_sample_rate_hz}Hz"
            ),
        });
    }
    let variants = if audio_variants.is_empty() {
        std::slice::from_ref(audio)
    } else {
        audio_variants
    };
    if variants.len() > MAX_AUDIO_VARIANTS {
        return Err(MediaEngineError::InvalidParameters {
            message: format!(
                "audio_variants 最多允许 {MAX_AUDIO_VARIANTS} 条，实际收到 {} 条",
                variants.len()
            ),
        });
    }
    if !audio_variants.is_empty() {
        for (index, variant) in variants.iter().enumerate() {
            let field_prefix = format!("audio_variants[{index}]");
            validate_audio_params(variant, &field_prefix)?;
            validate_audio_filter_support(variant, &field_prefix, true, ambient_input_available)?;
            if (variant.pitch_shift_semitones - audio.pitch_shift_semitones).abs() > f64::EPSILON
                || (variant.formant_shift_percent - audio.formant_shift_percent).abs()
                    > f64::EPSILON
                || variant.mfcc_shift_percent != audio.mfcc_shift_percent
                || variant.mfcc_dimensions != audio.mfcc_dimensions
                || variant.snr_target_db != audio.snr_target_db
                || variant.snr_variation_db != audio.snr_variation_db
                || variant.ambient_sound_mix_percent != audio.ambient_sound_mix_percent
            {
                return Err(MediaEngineError::InvalidParameters {
                    message: format!(
                        "{field_prefix} 的音高、共振峰、MFCC、SNR 和环境声参数必须与主 audio 一致；这些效果位于多支路混音后的总线"
                    ),
                });
            }
        }
    }
    let filter_graph = audio_mix_filter_complex(
        audio,
        variants,
        source_audio_sample_rate_hz,
        Some(output_sample_rate_hz),
        true,
        ambient_input_available,
    )?;
    let quality_pitch = (audio.pitch_shift_semitones.abs() > f64::EPSILON
        || audio.formant_shift_percent.abs() > f64::EPSILON)
        .then_some(QualityPitchConfig {
            pitch_shift_semitones: audio.pitch_shift_semitones,
            formant_shift_percent: audio.formant_shift_percent,
        });
    let offline_plan = build_offline_audio_effect_plan(audio).map_err(|errors| {
        MediaEngineError::InvalidParameters {
            message: errors
                .into_iter()
                .map(|error| error.message)
                .collect::<Vec<_>>()
                .join("；"),
        }
    })?;
    let pcm_effects = AudioPcmEffectConfig {
        mfcc: offline_plan.analysis.mfcc,
        snr: offline_plan.snr,
    };
    Ok(AudioStreamFilterPlan {
        filter_graph,
        quality_pitch,
        pcm_effects: pcm_effects.is_active().then_some(pcm_effects),
        requires_ambient_input: offline_plan.ambient_sound_mix.is_some(),
    })
}

fn audio_filter(
    audio: &AudioEffectParams,
    source_sample_rate_hz: Option<u32>,
    realtime: bool,
) -> Result<String, MediaEngineError> {
    let gain_db = audio.input_gain_db + audio.output_gain_db + audio.loudness_adjustment_db;
    let mut filters = vec![format!("volume={gain_db:.6}dB")];
    for (frequency_hz, gain_db) in [
        (200_u32, audio.low_eq_db),
        (1_000_u32, audio.mid_eq_db),
        (8_000_u32, audio.high_eq_db),
    ] {
        if gain_db.abs() > f64::EPSILON {
            filters.push(format!(
                "equalizer=f={frequency_hz}:t=q:w={:.6}:g={gain_db:.6}",
                audio.filter_q
            ));
        }
    }
    if audio.pitch_shift_semitones.abs() > f64::EPSILON && !realtime {
        // ponytail: 无探测采样率时回退 48k，避免微扰音高整次失败
        let source_sample_rate_hz = source_sample_rate_hz.unwrap_or(48_000);
        let factor = 2_f64.powf(audio.pitch_shift_semitones / 12.0);
        let shifted_sample_rate_hz = (f64::from(source_sample_rate_hz) * factor).round();
        filters.push(format!("asetrate={shifted_sample_rate_hz:.0}"));
        filters.push(format!("aresample={source_sample_rate_hz}"));
        filters.push(format!("atempo={:.6}", 1.0 / factor));
    }
    if (audio.playback_speed - 1.0).abs() > f64::EPSILON {
        filters.push(format!("atempo={:.6}", audio.playback_speed));
    }
    if audio.fade_in_ms > 0 {
        filters.push(format!(
            "afade=t=in:st=0:d={:.6}",
            audio.fade_in_ms as f64 / 1_000.0
        ));
    }
    // 实时解码会在 EOF 后顺序重启 FFmpeg；反向滤镜仍会先缓存整轮输入，造成启动静音。
    // 因此实时链跳过反向淡出以持续输出 PCM；离线有限输入仍保留原链。
    if audio.fade_out_ms > 0 && !realtime {
        filters.push(format!(
            "areverse,afade=t=in:st=0:d={:.6},areverse",
            audio.fade_out_ms as f64 / 1_000.0
        ));
    }
    if audio.reverb_wet_percent > 0.0 {
        let decay = audio.reverb_wet_percent / 100.0;
        filters.push(format!("aecho=1.0:1.0:80:{decay:.6}"));
    }
    if audio.noise_reduction_percent > 0.0 {
        let reduction_db = (audio.noise_reduction_percent * 0.97).clamp(0.01, 97.0);
        filters.push(format!("afftdn=nr={reduction_db:.6}"));
    }
    if audio.phase_perturbation_percent.abs() > f64::EPSILON {
        // FFmpeg aphaser 的 decay 上限是 0.99；参数契约的 20% 仍表示最大效果强度。
        let depth = (audio.phase_perturbation_percent.abs() / 20.0).clamp(0.0, 0.99);
        // aphaser 默认 0.4/0.74 增益会把低感知微扰整体衰减约 10dB；仅强效果
        // 使用默认防削波增益，1% 以内保持源素材相对响度。
        let (input_gain, output_gain) = if audio.phase_perturbation_percent.abs() <= 1.0 {
            ("1.0", "1.0")
        } else {
            ("0.4", "0.74")
        };
        filters.push(format!(
            "aphaser=in_gain={input_gain}:out_gain={output_gain}:delay=3:decay={depth:.6}:speed=0.5"
        ));
    }
    if audio.vibrato_depth_percent > 0.0 {
        let depth = (audio.vibrato_depth_percent / 100.0).clamp(0.0, 1.0);
        filters.push(format!(
            "vibrato=f={:.6}:d={depth:.6}",
            audio.vibrato_frequency_hz
        ));
    }
    let supplementary = build_offline_audio_effect_plan(audio).map_err(|errors| {
        MediaEngineError::InvalidParameters {
            message: errors
                .into_iter()
                .map(|error| error.message)
                .collect::<Vec<_>>()
                .join("；"),
        }
    })?;
    filters.extend(supplementary.serial_filters);
    if let Some(sample_rate_hz) = audio.sample_rate_hz {
        if Some(sample_rate_hz) != source_sample_rate_hz {
            filters.push(format!("aresample={sample_rate_hz}"));
        }
    }
    Ok(filters.join(","))
}

fn cleanup(path: &Path) {
    let _ignored = fs::remove_file(path);
}

fn terminate_child(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let _ = background_command("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn read_ffmpeg_progress(mut stdout: impl Read, sender: mpsc::SyncSender<u64>) {
    let mut parser = FfmpegProgressParser::default();
    let mut buffer = [0_u8; 4096];
    loop {
        let Ok(read) = stdout.read(&mut buffer) else {
            return;
        };
        if read == 0 {
            parser.finish(|value| {
                let _ = sender.try_send(value);
            });
            return;
        }
        parser.push(&buffer[..read], |value| {
            let _ = sender.try_send(value);
        });
    }
}

fn publish_ffmpeg_progress(
    receiver: &mpsc::Receiver<u64>,
    source_duration_ms: Option<u64>,
    on_progress: &mut impl FnMut(u8),
) {
    for out_time_us in receiver.try_iter() {
        if let Some(percent) = video_render_progress_percent(out_time_us, source_duration_ms) {
            on_progress(percent);
        }
    }
}

fn join_progress_reader(reader: &mut Option<thread::JoinHandle<()>>) {
    if let Some(handle) = reader.take() {
        let _ = handle.join();
    }
}

#[derive(Debug, Default)]
struct MediaStderrCapture {
    diagnostic_tail: Option<String>,
    max_volume_db: Option<f64>,
}

fn read_stderr_capture(mut stderr: impl Read) -> MediaStderrCapture {
    let mut retained = VecDeque::with_capacity(MAX_MEDIA_STDERR_BYTES);
    let mut buffer = [0_u8; 4096];
    loop {
        let Ok(read) = stderr.read(&mut buffer) else {
            return MediaStderrCapture::default();
        };
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            if retained.len() == MAX_MEDIA_STDERR_BYTES {
                retained.pop_front();
            }
            retained.push_back(*byte);
        }
    }
    let bytes = retained.into_iter().collect::<Vec<_>>();
    let text = String::from_utf8_lossy(&bytes);
    let max_volume_db = parse_max_volume_db(&text);
    let trimmed = text
        .lines()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ");
    let compact = trimmed.trim();
    let diagnostic_tail = if compact.is_empty() {
        None
    } else {
        Some(
            compact
                .chars()
                .rev()
                .take(500)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect(),
        )
    };
    MediaStderrCapture {
        diagnostic_tail,
        max_volume_db,
    }
}

fn read_stderr_tail(stderr: impl Read) -> Option<String> {
    read_stderr_capture(stderr).diagnostic_tail
}

fn join_stderr_reader(reader: &mut Option<thread::JoinHandle<Option<String>>>) -> Option<String> {
    reader
        .take()
        .and_then(|handle| handle.join().ok().flatten())
}

fn join_stderr_capture_reader(
    reader: &mut Option<thread::JoinHandle<MediaStderrCapture>>,
) -> Option<MediaStderrCapture> {
    reader.take().and_then(|handle| handle.join().ok())
}

#[cfg(test)]
mod tests {
    use super::{
        audio_mix_filter_complex, build_audio_stream_filter_graph,
        build_audio_stream_filter_graph_with_ambient, build_media_render_args_for_backend,
        build_media_render_args_with_video_encoder,
        configured_media_engine_paths_with_resource_dir, encoder_attempt_order_from_preferred,
        encoder_failure_allows_retry, legacy_vulkan_subset_filter, media_render_deadline,
        packaged_media_engine_paths, parse_max_volume_db, probe_audio_content_with_retry,
        read_stderr_capture, read_stderr_tail, remaining_deadline_millis, run_command_with_timeout,
        target_triple, validate_audio_content, validate_audio_input_decodable,
        validate_audio_probe_output, validate_filter_support, validate_request_shape,
        video_encoder_codec_args, video_filter, video_render_progress_percent,
        FfmpegProgressParser, MediaEngineError, MediaOutputProgressWatchdog, MediaRenderRequest,
        MediaRenderTarget, AUDIO_FINITE_GUARD_FILTER, FALLBACK_H264_ENCODER,
        MAX_PROBE_STDOUT_BYTES,
    };
    use crate::media_effect_params::{
        AdvancedEffectParams, AudioEffectParams, NaturalVoiceMode, VideoEffectParams,
    };
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::Stdio;
    use std::sync::Mutex;

    static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

    const OVERSIZED_PROBE_FIXTURE_ENV: &str = "AUTOLIVE_OVERSIZED_PROBE_FIXTURE";

    #[test]
    fn offline_filter_validation_accepts_a_resolved_ambient_input() {
        let root =
            std::env::temp_dir().join(format!("autolive-offline-ambient-{}", std::process::id()));
        fs::create_dir_all(&root).expect("create fixture root");
        let executable = std::env::current_exe().expect("test executable");
        let input = root.join("input.mp4");
        let ambient = root.join("ambient.wav");
        fs::write(&input, b"input").expect("write input fixture");
        fs::write(&ambient, b"ambient").expect("write ambient fixture");
        let audio = AudioEffectParams {
            ambient_sound_mix_percent: 40.0,
            ..Default::default()
        };
        let mut request = MediaRenderRequest {
            ffmpeg_path: executable.clone(),
            ffprobe_path: executable,
            input_mp4_path: input,
            source_has_video: true,
            ambient_input_path: None,
            source_duration_ms: Some(1_000),
            source_start_ms: 0,
            output_duration_ms: 1_000,
            loop_source: false,
            staging_output_path: root.join("output.partial.mp4"),
            output_mp4_path: root.join("output.mp4"),
            video_processing_enabled: false,
            audio_processing_enabled: true,
            source_audio_sample_rate_hz: Some(48_000),
            video: VideoEffectParams::default(),
            audio,
            audio_variants: Vec::new(),
            advanced: AdvancedEffectParams::default(),
            timeout_seconds: 1,
            target: MediaRenderTarget::StandardMp4,
        };

        assert!(validate_filter_support(&request).is_err());
        request.ambient_input_path = Some(ambient);
        assert!(validate_filter_support(&request).is_ok());
        assert!(build_media_render_args_with_video_encoder(&request, None).is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn audio_only_candidate_accepts_m4a_output_paths() {
        let root = std::env::temp_dir().join(format!(
            "autolive-audio-candidate-output-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create fixture root");
        let executable = std::env::current_exe().expect("test executable");
        let input = root.join("input.mp4");
        fs::write(&input, b"input").expect("write input fixture");
        let request = MediaRenderRequest {
            ffmpeg_path: executable.clone(),
            ffprobe_path: executable,
            input_mp4_path: input,
            source_has_video: false,
            ambient_input_path: None,
            source_duration_ms: Some(1_000),
            source_start_ms: 0,
            output_duration_ms: 1_000,
            loop_source: false,
            staging_output_path: root.join("output.partial.m4a"),
            output_mp4_path: root.join("output.m4a"),
            video_processing_enabled: false,
            audio_processing_enabled: true,
            source_audio_sample_rate_hz: Some(44_100),
            video: VideoEffectParams::default(),
            audio: AudioEffectParams::default(),
            audio_variants: Vec::new(),
            advanced: AdvancedEffectParams::default(),
            timeout_seconds: 1,
            target: MediaRenderTarget::StandardMp4,
        };

        assert!(validate_request_shape(&request).is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn software_h264_fallback_limits_video_encoder_threads() {
        let software_args = video_encoder_codec_args("libopenh264");
        let software_args = software_args
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(software_args.windows(2).any(|pair| {
            pair[0] == "-threads:v"
                && pair[1]
                    .parse::<usize>()
                    .is_ok_and(|threads| (1..=8).contains(&threads))
        }));
        for hardware_encoder in ["h264_nvenc", "h264_amf", "h264_qsv", "h264_mf"] {
            assert!(!video_encoder_codec_args(hardware_encoder)
                .iter()
                .any(|value| value == "-threads:v"));
        }
    }

    #[test]
    fn encoder_fallback_only_moves_down_the_strict_capability_order() {
        let expected_from_amf = if cfg!(windows) {
            vec!["h264_amf", "h264_qsv", "h264_mf", FALLBACK_H264_ENCODER]
        } else {
            vec!["h264_amf", "h264_qsv", FALLBACK_H264_ENCODER]
        };
        assert_eq!(
            encoder_attempt_order_from_preferred("h264_amf"),
            expected_from_amf
        );
        assert_eq!(
            encoder_attempt_order_from_preferred(FALLBACK_H264_ENCODER),
            [FALLBACK_H264_ENCODER]
        );
    }

    #[test]
    fn vulkan_filter_accepts_only_the_verified_gpu_parameter_subset() {
        let video = VideoEffectParams {
            brightness_percent: 0.2,
            contrast_percent: 100.2,
            saturation_percent: 99.8,
            hue_rotation_degrees: 0.1,
            blur_radius_px: 0.03,
            ..VideoEffectParams::default()
        };
        let plan = legacy_vulkan_subset_filter(&video, &AdvancedEffectParams::default(), false)
            .expect("verified Vulkan fields");
        assert!(plan
            .serial_filter
            .contains("format=nv12,hwupload,libplacebo="));
        assert!(plan.serial_filter.contains("gblur_vulkan=sigma=0.030000"));
        assert!(plan.serial_filter.ends_with("hwdownload,format=nv12"));

        let cpu_only = VideoEffectParams {
            noise_percent: 0.05,
            ..video
        };
        assert!(
            legacy_vulkan_subset_filter(&cpu_only, &AdvancedEffectParams::default(), false)
                .is_none()
        );
    }

    #[test]
    fn vulkan_render_args_use_vendor_neutral_filter_with_selected_encoder() {
        let root = std::env::temp_dir().join(format!(
            "autolive-vulkan-filter-args-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create fixture root");
        let input = root.join("input.mp4");
        fs::write(&input, b"input").expect("write input fixture");
        let request = MediaRenderRequest {
            ffmpeg_path: std::env::current_exe().expect("test executable"),
            ffprobe_path: std::env::current_exe().expect("test executable"),
            input_mp4_path: input,
            source_has_video: true,
            ambient_input_path: None,
            source_duration_ms: Some(1_000),
            source_start_ms: 0,
            output_duration_ms: 1_000,
            loop_source: false,
            staging_output_path: root.join("output.partial.mp4"),
            output_mp4_path: root.join("output.mp4"),
            video_processing_enabled: true,
            audio_processing_enabled: false,
            source_audio_sample_rate_hz: Some(48_000),
            video: VideoEffectParams {
                brightness_percent: 0.2,
                ..VideoEffectParams::default()
            },
            audio: AudioEffectParams::default(),
            audio_variants: Vec::new(),
            advanced: AdvancedEffectParams::default(),
            timeout_seconds: 1,
            target: MediaRenderTarget::StandardMp4,
        };
        let args = build_media_render_args_for_backend(&request, Some("h264_nvenc"), true, true)
            .expect("Vulkan render args")
            .into_iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(args
            .windows(2)
            .any(|pair| pair == ["-init_hw_device", "vulkan=autolive_gpu:0"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-filter_hw_device", "autolive_gpu"]));
        assert!(args.windows(2).any(|pair| pair == ["-hwaccel", "vulkan"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-hwaccel_output_format", "vulkan"]));
        assert!(args.iter().any(|value| value.contains("libplacebo=")));
        assert!(!args.iter().any(|value| value.contains("hwupload")));
        assert!(args.windows(2).any(|pair| pair == ["-c:v", "h264_nvenc"]));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn media_render_deadline_is_shared_across_encoder_attempts() {
        let started_at = std::time::Instant::now();
        let deadline = media_render_deadline(started_at, 2).expect("valid render deadline");

        assert!(started_at + std::time::Duration::from_secs(1) < deadline);
        assert_eq!(started_at + std::time::Duration::from_secs(2), deadline);
        assert!(remaining_deadline_millis(deadline, 2).is_ok());
        assert!(matches!(
            remaining_deadline_millis(std::time::Instant::now(), 2),
            Err(MediaEngineError::Timeout { seconds: 2 })
        ));
    }

    #[test]
    fn gpu_attempt_deadline_flows_through_outer_retry_logic() {
        let source = include_str!("media_engine.rs");
        let timeout_branch = source
            .split("[video-render] stage=attempt_timeout")
            .nth(1)
            .and_then(|tail| tail.split("match child.try_wait()").next())
            .expect("GPU attempt timeout branch");

        assert!(timeout_branch.contains("break Err(MediaEngineError::Timeout"));
        assert!(!timeout_branch.contains("return Err(MediaEngineError::Timeout"));
    }

    #[test]
    fn media_output_progress_watchdog_tracks_real_file_size_changes() {
        let started_at = std::time::Instant::now();
        let mut watchdog = MediaOutputProgressWatchdog::new(started_at);

        assert!(!watchdog.observe(started_at + std::time::Duration::from_secs(10), Some(0)));
        assert!(!watchdog.observe(started_at + std::time::Duration::from_secs(100), Some(64)));
        assert!(!watchdog.observe(started_at + std::time::Duration::from_secs(219), Some(64)));
        assert!(watchdog.observe(started_at + std::time::Duration::from_secs(220), Some(64)));
        assert!(!watchdog.observe(started_at + std::time::Duration::from_secs(221), Some(32)));
    }

    #[test]
    fn ffmpeg_progress_parser_accepts_only_bounded_out_time_us_lines() {
        let mut parser = FfmpegProgressParser::default();
        let mut values = Vec::new();
        parser.push(
            b"frame=12\nout_time_us=22325000\nprogress=continue\n",
            |value| values.push(value),
        );
        parser.push(&vec![b'x'; 300], |value| values.push(value));
        parser.push(b"\nout_time_us=44650000\n", |value| values.push(value));

        assert_eq!(values, [22_325_000, 44_650_000]);
    }

    #[test]
    fn video_render_progress_uses_media_time_and_reserves_completion_for_commit() {
        assert_eq!(video_render_progress_percent(0, Some(44_650)), Some(0));
        assert_eq!(
            video_render_progress_percent(22_325_000, Some(44_650)),
            Some(50)
        );
        assert_eq!(
            video_render_progress_percent(44_650_000, Some(44_650)),
            Some(99)
        );
        assert_eq!(video_render_progress_percent(1, None), None);
        assert_eq!(video_render_progress_percent(1, Some(0)), None);
    }

    #[test]
    fn hardware_encoder_nonzero_exit_falls_back_without_driver_message_matching() {
        let failed = MediaEngineError::Failed {
            code: Some(1),
            stderr: Some("localized or previously unknown driver failure".to_owned()),
        };
        assert!(encoder_failure_allows_retry("h264_qsv", &failed));
        assert!(encoder_failure_allows_retry("h264_nvenc", &failed));
        assert!(!encoder_failure_allows_retry(
            FALLBACK_H264_ENCODER,
            &failed
        ));

        for error in [
            MediaEngineError::SpawnFailed {
                path: "ffmpeg".to_owned(),
                message: "access denied".to_owned(),
            },
            MediaEngineError::Timeout { seconds: 1 },
            MediaEngineError::Stalled { seconds: 120 },
            MediaEngineError::Cancelled,
        ] {
            assert!(
                !encoder_failure_allows_retry("h264_qsv", &error),
                "managed process failure must stop fallback: {error:?}"
            );
        }
    }

    #[test]
    fn low_video_noise_uses_integer_strength_with_sparse_probability() {
        let video = VideoEffectParams {
            noise_percent: 1.0,
            ..VideoEffectParams::default()
        };
        let plan = video_filter(&video, &AdvancedEffectParams::default()).expect("video filter");

        assert!(plan.serial_filter.contains("noise=alls=1:allf=t+u"));
        assert!(plan.serial_filter.contains("enable='lt(mod(n*7919+"));
        assert!(!plan.serial_filter.contains("noise=alls=1.000000"));
    }

    #[test]
    fn automatic_low_strength_image_repair_uses_lightweight_filter() {
        let video = VideoEffectParams {
            image_repair_enabled: true,
            image_repair_strength_percent: 0.2,
            ..VideoEffectParams::default()
        };

        let plan = video_filter(&video, &AdvancedEffectParams::default()).expect("video filter");
        assert!(!plan.serial_filter.contains("atadenoise="));
        assert!(!plan.serial_filter.contains("hqdn3d="));
        assert!(!plan.serial_filter.contains("nlmeans="));
        assert!(plan.serial_filter.contains("unsharp=5:5:0.000800"));

        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            let resource_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("embedded-runtime-resources")
                .join(target_triple());
            let (ffmpeg, _) = packaged_media_engine_paths(&resource_root)
                .expect("embedded FFmpeg path should resolve");
            let output = super::background_command(ffmpeg)
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=size=16x16:rate=5:duration=1",
                    "-vf",
                ])
                .arg(&plan.serial_filter)
                .args(["-frames:v", "1", "-f", "null", "-"])
                .output()
                .expect("embedded FFmpeg should start");
            assert!(
                output.status.success(),
                "embedded FFmpeg rejected low-strength image repair: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn explicit_high_strength_image_repair_keeps_nlmeans() {
        let video = VideoEffectParams {
            image_repair_enabled: true,
            image_repair_strength_percent: 10.0,
            ..VideoEffectParams::default()
        };

        let plan = video_filter(&video, &AdvancedEffectParams::default()).expect("video filter");
        assert!(plan.serial_filter.contains("nlmeans="));
        assert!(!plan.serial_filter.contains("hqdn3d="));
    }

    #[test]
    fn low_strength_image_repair_scales_detail_amount() {
        let filter_at = |strength| {
            video_filter(
                &VideoEffectParams {
                    image_repair_enabled: true,
                    image_repair_strength_percent: strength,
                    ..VideoEffectParams::default()
                },
                &AdvancedEffectParams::default(),
            )
            .expect("video filter")
            .serial_filter
        };

        assert!(filter_at(0.2).contains("unsharp=5:5:0.000800"));
        assert!(filter_at(0.8).contains("unsharp=5:5:0.003200"));
    }

    #[test]
    fn image_repair_and_detail_share_one_unsharp_pass() {
        let video = VideoEffectParams {
            image_repair_enabled: true,
            image_repair_strength_percent: 0.2,
            sharpen_percent: 0.2,
            detail_enhancement_percent: 0.2,
            ..VideoEffectParams::default()
        };

        let plan = video_filter(&video, &AdvancedEffectParams::default()).expect("video filter");
        assert_eq!(plan.serial_filter.matches("unsharp=").count(), 1);
    }

    #[test]
    fn oversized_probe_child_fixture() {
        if std::env::var_os(OVERSIZED_PROBE_FIXTURE_ENV).is_none() {
            return;
        }
        std::io::stdout()
            .write_all(&vec![b'x'; MAX_PROBE_STDOUT_BYTES + 1])
            .expect("fixture stdout should be writable");
    }

    #[test]
    fn probe_stdout_is_read_concurrently_and_rejected_above_limit() {
        let _guard = TEST_ENV_LOCK.lock().expect("test environment lock");
        let executable = std::env::current_exe().expect("test executable should resolve");
        std::env::set_var(OVERSIZED_PROBE_FIXTURE_ENV, "1");
        let result = run_command_with_timeout(
            &executable,
            [
                "--exact",
                "media_engine::tests::oversized_probe_child_fixture",
                "--nocapture",
            ],
            5_000,
        );
        std::env::remove_var(OVERSIZED_PROBE_FIXTURE_ENV);

        assert!(matches!(
            result,
            Err(MediaEngineError::ProbeOutputTooLarge {
                limit_bytes: MAX_PROBE_STDOUT_BYTES,
                ..
            })
        ));
    }

    #[test]
    fn multi_variant_audio_graph_normalizes_continuous_timestamps_before_mixing() {
        let primary = AudioEffectParams::default();
        let delayed = AudioEffectParams {
            reverb_wet_percent: 20.0,
            ..primary.clone()
        };
        let variants = [primary.clone(), delayed];
        let graph = audio_mix_filter_complex(
            &primary,
            &variants,
            Some(48_000),
            Some(48_000),
            false,
            false,
        )
        .expect("multi-variant graph should build");

        assert!(graph.starts_with(
            "[0:a:0]asetpts=PTS-STARTPTS,aresample=async=1:first_pts=0,asetpts=N/SR/TB,asplit=2[a0][a1]"
        ));
        for index in 0..2 {
            assert!(graph.contains(&format!(
                "[b{index}]asetpts=PTS-STARTPTS,aresample=48000:async=1:first_pts=0,asetpts=N/SR/TB[mix{index}]"
            )));
        }
        assert!(graph.contains(
            "[mix0][mix1]amix=inputs=2:weights=0.500000 0.500000:duration=shortest:dropout_transition=0[variant_bus]"
        ));
    }

    #[test]
    fn realtime_audio_filter_graph_supports_common_portaudio_rates() {
        let audio = AudioEffectParams::default();
        let explicit_variants = [audio.clone()];
        let fallback_graph = build_audio_stream_filter_graph(&audio, &[], Some(48_000), 48_000)
            .expect("empty variants should fall back to the primary audio profile");
        let explicit_graph =
            build_audio_stream_filter_graph(&audio, &explicit_variants, Some(48_000), 48_000)
                .expect("explicit single variant should build");
        assert_eq!(fallback_graph, explicit_graph);

        let mut invalid_audio = audio.clone();
        invalid_audio.sample_rate_hz = Some(22_050);
        assert!(build_audio_stream_filter_graph(
            &invalid_audio,
            &explicit_variants,
            Some(48_000),
            48_000,
        )
        .is_err());
        assert!(build_audio_stream_filter_graph(&audio, &[], Some(48_000), 22_050).is_err());

        for output_sample_rate_hz in [44_100, 48_000] {
            let graph =
                build_audio_stream_filter_graph(&audio, &[], Some(48_000), output_sample_rate_hz)
                    .expect("default audio profile should build a realtime filter graph");

            assert!(graph
                .filter_graph
                .contains("aformat=sample_fmts=fltp:channel_layouts=stereo"));
            assert!(graph.filter_graph.contains(&format!(
                "sample_rates={output_sample_rate_hz}:channel_layouts=stereo[aout]"
            )));
            assert!(
                graph
                    .filter_graph
                    .matches(AUDIO_FINITE_GUARD_FILTER)
                    .count()
                    >= 2
            );
        }
        assert!(build_audio_stream_filter_graph(&audio, &[], Some(48_000), 96_000).is_err());

        let mut invalid_primary = audio.clone();
        invalid_primary.input_gain_db = f64::NAN;
        assert!(build_audio_stream_filter_graph(
            &invalid_primary,
            std::slice::from_ref(&audio),
            Some(48_000),
            48_000,
        )
        .is_err());
    }

    #[test]
    fn realtime_audio_bus_does_not_force_low_perception_profiles_to_fixed_lufs() {
        let audio = AudioEffectParams {
            voice_library_id: Some("gpal-subtle-p01".to_owned()),
            loudness_adjustment_db: -0.037,
            input_gain_db: -0.06,
            output_gain_db: 0.045,
            ..Default::default()
        };

        let graph = build_audio_stream_filter_graph(&audio, &[], Some(44_100), 44_100)
            .expect("low-perception profile should build")
            .filter_graph;

        assert!(graph.contains("volume=-0.052000dB"));
        assert!(
            !graph.contains("loudnorm="),
            "fixed loudness normalization destroys source-relative level: {graph}"
        );
    }

    #[test]
    fn subtle_phase_perturbation_does_not_apply_the_phaser_default_attenuation() {
        let subtle = AudioEffectParams {
            phase_perturbation_percent: 0.42,
            ..Default::default()
        };
        let obvious = AudioEffectParams {
            phase_perturbation_percent: 8.0,
            ..Default::default()
        };

        let subtle_graph = build_audio_stream_filter_graph(&subtle, &[], Some(48_000), 48_000)
            .expect("subtle phase graph")
            .filter_graph;
        let obvious_graph = build_audio_stream_filter_graph(&obvious, &[], Some(48_000), 48_000)
            .expect("obvious phase graph")
            .filter_graph;

        assert!(subtle_graph.contains("aphaser=in_gain=1.0:out_gain=1.0"));
        assert!(obvious_graph.contains("aphaser=in_gain=0.4:out_gain=0.74"));
    }

    #[test]
    fn realtime_fade_out_does_not_buffer_a_full_source_pass() {
        let audio = AudioEffectParams {
            fade_out_ms: 1_000,
            ..Default::default()
        };

        let realtime_graph = build_audio_stream_filter_graph(&audio, &[], Some(48_000), 48_000)
            .expect("realtime fade-out configuration should still build");
        assert!(!realtime_graph.filter_graph.contains("areverse"));

        let offline_graph = audio_mix_filter_complex(
            &audio,
            std::slice::from_ref(&audio),
            Some(48_000),
            Some(48_000),
            false,
            false,
        )
        .expect("offline finite-input graph should build");
        assert!(offline_graph.contains("areverse"));
    }

    #[test]
    fn realtime_pitch_uses_quality_bus_and_rejects_divergent_variants() {
        let audio = AudioEffectParams {
            pitch_shift_semitones: 1.0,
            formant_shift_percent: 2.0,
            playback_speed: 1.25,
            ..Default::default()
        };
        let plan = build_audio_stream_filter_graph(
            &audio,
            std::slice::from_ref(&audio),
            Some(48_000),
            48_000,
        )
        .expect("matching quality pitch profile should build");
        assert_eq!(
            plan.quality_pitch,
            Some(super::QualityPitchConfig {
                pitch_shift_semitones: 1.0,
                formant_shift_percent: 2.0,
            })
        );
        assert!(!plan.filter_graph.contains("asetrate="));
        assert!(plan.filter_graph.contains("atempo=1.250000"));

        let divergent = AudioEffectParams {
            pitch_shift_semitones: 0.5,
            ..audio.clone()
        };
        assert!(build_audio_stream_filter_graph(
            &audio,
            std::slice::from_ref(&divergent),
            Some(48_000),
            48_000,
        )
        .is_err());
    }

    #[test]
    fn realtime_plan_connects_all_supplementary_audio_runtime_boundaries() {
        let audio = AudioEffectParams {
            natural_voice_mode: NaturalVoiceMode::NaturalDynamic,
            voice_library_id: Some("local-voice-a".to_owned()),
            mfcc_shift_percent: 8.0,
            mfcc_dimensions: 20,
            snr_target_db: Some(24.0),
            snr_variation_db: -2.0,
            spectrum_blind_spot_percent: 2.0,
            dry_wet_percent: 25.0,
            ambient_sound_mix_percent: 40.0,
            ..Default::default()
        };

        assert!(build_audio_stream_filter_graph(&audio, &[], Some(48_000), 48_000).is_err());
        let plan =
            build_audio_stream_filter_graph_with_ambient(&audio, &[], Some(48_000), 48_000, true)
                .expect("validated ambient input should complete the realtime plan");

        assert!(plan.requires_ambient_input);
        assert!(plan.pcm_effects.is_some());
        for fragment in [
            "volume='1+0.012000*sin",
            "equalizer=f=",
            "bandreject=f=8000.000000",
            "[dry0][wet0]amix=inputs=2:weights=0.750000 0.250000",
            "[1:a:0]asetpts=PTS-STARTPTS,aformat=",
            "[variant_bus][ambient]amix=inputs=2:weights=0.600000 0.400000",
        ] {
            assert!(
                plan.filter_graph.contains(fragment),
                "{fragment}: {}",
                plan.filter_graph
            );
        }
    }

    #[test]
    fn packaged_ffmpeg_accepts_realtime_dry_wet_and_real_ambient_graph() {
        let Some(ffmpeg) = std::env::var_os("AUTOLIVE_TEST_FFMPEG") else {
            return;
        };
        let audio = AudioEffectParams {
            dry_wet_percent: 25.0,
            ambient_sound_mix_percent: 40.0,
            ..Default::default()
        };
        let graph =
            build_audio_stream_filter_graph_with_ambient(&audio, &[], Some(48_000), 48_000, true)
                .expect("graph")
                .filter_graph;
        let output = super::background_command(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=0.2",
                "-f",
                "lavfi",
                "-i",
                "anoisesrc=sample_rate=48000:duration=0.2",
                "-filter_complex",
                &graph,
                "-map",
                "[aout]",
                "-f",
                "null",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .expect("run packaged FFmpeg");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn packaged_ffmpeg_decodes_the_bundled_low_level_ambient_resource() {
        let Some(ffmpeg) = std::env::var_os("AUTOLIVE_TEST_FFMPEG") else {
            return;
        };
        validate_audio_input_decodable(
            std::path::Path::new(&ffmpeg),
            std::path::Path::new("ambient/low-level-room-tone.wav"),
            5_000,
        )
        .expect("bundled ambient WAV should contain a decodable audio stream");
    }

    #[test]
    fn audio_probe_requires_expected_aac_contract() {
        let output = br#"{"streams":[{"codec_type":"audio","codec_name":"aac","sample_rate":"48000","channels":2,"sample_fmt":"fltp"}]}"#;
        assert!(validate_audio_probe_output(output, 48_000).is_ok());

        let invalid = br#"{"streams":[{"codec_type":"audio","codec_name":"mp3","sample_rate":"48000","channels":2,"sample_fmt":"fltp"}]}"#;
        assert!(validate_audio_probe_output(invalid, 48_000).is_err());
    }

    #[test]
    fn volume_probe_parses_audible_and_silent_outputs() {
        assert_eq!(
            parse_max_volume_db("mean_volume: -18.7 dB\nmax_volume: -3.2 dB"),
            Some(-3.2)
        );
        assert_eq!(
            parse_max_volume_db("mean_volume: -18.7 dB | max_volume: -3.2 dB | histogram_3db: 4"),
            Some(-3.2)
        );
        assert_eq!(parse_max_volume_db("max_volume: -91.0 dB"), Some(-91.0));
        assert_eq!(
            parse_max_volume_db("max_volume: -inf dB"),
            Some(f64::NEG_INFINITY)
        );
        assert_eq!(parse_max_volume_db("volumedetect failed"), None);
    }

    #[test]
    fn stderr_tail_keeps_the_final_volume_summary() {
        let diagnostic = format!(
            "{}\n[Parsed_volumedetect_0] mean_volume: -18.7 dB\n[Parsed_volumedetect_0] max_volume: -3.2 dB",
            "input and stream diagnostic ".repeat(40)
        );

        let compact = read_stderr_tail(diagnostic.as_bytes()).expect("diagnostic tail");

        assert_eq!(parse_max_volume_db(&compact), Some(-3.2));
    }

    #[test]
    fn stderr_capture_keeps_max_volume_before_histogram_lines() {
        let histogram = (0..12)
            .map(|index| format!("[Parsed_volumedetect_0] histogram_{index}db: 100"))
            .collect::<Vec<_>>()
            .join("\n");
        let diagnostic = format!(
            "[Parsed_volumedetect_0] mean_volume: -18.7 dB\n[Parsed_volumedetect_0] max_volume: -3.2 dB\n{histogram}"
        );

        let capture = read_stderr_capture(diagnostic.as_bytes());

        assert_eq!(capture.max_volume_db, Some(-3.2));
        assert!(capture
            .diagnostic_tail
            .is_some_and(|tail| tail.chars().count() <= 500));
    }

    #[test]
    fn stderr_capture_keeps_max_volume_before_long_suffix() {
        let suffix = (0..7)
            .map(|index| {
                format!(
                    "[Parsed_volumedetect_0] histogram_{index}db: {}",
                    "1".repeat(100)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let diagnostic = format!("[Parsed_volumedetect_0] max_volume: -1.3 dB\n{suffix}");

        let capture = read_stderr_capture(diagnostic.as_bytes());

        assert_eq!(capture.max_volume_db, Some(-1.3));
        assert!(capture
            .diagnostic_tail
            .is_some_and(|tail| tail.chars().count() <= 500));
    }

    #[test]
    fn incomplete_volume_probe_retries_twice_and_stops() {
        let mut recovered_attempts = 0;
        let recovered = probe_audio_content_with_retry(|| {
            recovered_attempts += 1;
            if recovered_attempts < 3 {
                Err(MediaEngineError::AudioContentProbeIncomplete {
                    path: "candidate.partial.m4a".to_owned(),
                })
            } else {
                Ok(-3.2)
            }
        });
        assert_eq!(recovered, Ok(-3.2));
        assert_eq!(recovered_attempts, 3);

        let mut exhausted_attempts = 0;
        let exhausted = probe_audio_content_with_retry(|| {
            exhausted_attempts += 1;
            Err(MediaEngineError::AudioContentProbeIncomplete {
                path: "candidate.partial.m4a".to_owned(),
            })
        });
        assert!(matches!(
            exhausted,
            Err(MediaEngineError::AudioContentProbeIncomplete { .. })
        ));
        assert_eq!(exhausted_attempts, 3);

        let mut silent_attempts = 0;
        let silent = probe_audio_content_with_retry(|| {
            silent_attempts += 1;
            Err(MediaEngineError::OutputSilent {
                path: "candidate.partial.m4a".to_owned(),
                max_volume_db: "-91.00".to_owned(),
            })
        });
        assert!(matches!(silent, Err(MediaEngineError::OutputSilent { .. })));
        assert_eq!(silent_attempts, 1);
    }

    #[test]
    fn packaged_ffmpeg_validates_an_audible_partial_m4a() {
        let Some(ffmpeg) = std::env::var_os("AUTOLIVE_TEST_FFMPEG") else {
            return;
        };
        let root =
            std::env::temp_dir().join(format!("autolive-volume-probe-{}", std::process::id()));
        fs::create_dir_all(&root).expect("create volume probe fixture root");
        let output = root.join("candidate.partial.m4a");
        let generated = super::background_command(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=0.5",
                "-c:a",
                "aac",
                "-ar",
                "48000",
                "-ac",
                "2",
                "-y",
            ])
            .arg(&output)
            .status()
            .expect("generate partial M4A fixture");
        assert!(generated.success());

        validate_audio_content(
            std::path::Path::new(&ffmpeg),
            &output,
            std::time::Instant::now() + std::time::Duration::from_secs(10),
            10,
            &crate::cancellation::CancellationToken::new(),
        )
        .expect("audible partial M4A must pass volumedetect");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn packaged_paths_use_the_current_target_triple() {
        let expected = if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
            "x86_64-apple-darwin"
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            "aarch64-apple-darwin"
        } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            "x86_64-pc-windows-msvc"
        } else {
            "unsupported"
        };
        assert_eq!(target_triple(), expected);

        if expected != "unsupported" {
            let root = PathBuf::from("/tmp/autolive-resources");
            let (ffmpeg, ffprobe) = packaged_media_engine_paths(&root)
                .expect("supported target should resolve packaged media paths");
            let extension = if cfg!(target_os = "windows") {
                ".exe"
            } else {
                ""
            };
            assert_eq!(ffmpeg, root.join(format!("binaries/ffmpeg{extension}")));
            assert_eq!(ffprobe, root.join(format!("binaries/ffprobe{extension}")));
        }
    }

    #[cfg(debug_assertions)]
    #[test]
    fn explicit_development_overrides_win_over_packaged_paths() {
        let _guard = TEST_ENV_LOCK
            .lock()
            .expect("test environment lock should work");
        let root =
            std::env::temp_dir().join(format!("autolive-media-engine-test-{}", std::process::id()));
        fs::create_dir_all(&root).expect("test resource directory should be created");
        let ffmpeg = root.join("dev-ffmpeg");
        let ffprobe = root.join("dev-ffprobe");
        fs::write(&ffmpeg, b"ffmpeg").expect("development ffmpeg should be created");
        fs::write(&ffprobe, b"ffprobe").expect("development ffprobe should be created");
        let previous_ffmpeg = std::env::var_os(super::FFMPEG_PATH_ENV);
        let previous_ffprobe = std::env::var_os(super::FFPROBE_PATH_ENV);
        std::env::set_var(super::FFMPEG_PATH_ENV, &ffmpeg);
        std::env::set_var(super::FFPROBE_PATH_ENV, &ffprobe);

        let resolved = configured_media_engine_paths_with_resource_dir(&root)
            .expect("development overrides should be resolved");
        assert_eq!(resolved, (ffmpeg, ffprobe));

        match previous_ffmpeg {
            Some(value) => std::env::set_var(super::FFMPEG_PATH_ENV, value),
            None => std::env::remove_var(super::FFMPEG_PATH_ENV),
        }
        match previous_ffprobe {
            Some(value) => std::env::set_var(super::FFPROBE_PATH_ENV, value),
            None => std::env::remove_var(super::FFPROBE_PATH_ENV),
        }
        let _ignored = fs::remove_dir_all(root);
    }
}
