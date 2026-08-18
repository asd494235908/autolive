use crate::background_process::background_command;
use crate::cancellation::CancellationToken;
use crate::hashing::hash_file_at_path;
use crate::media_library::SUPPORTED_SOURCE_VIDEO_EXTENSIONS;
use crate::research_params::{AudioResearchParams, ResearchExperimentParams, VideoResearchParams};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

const MIN_TIMEOUT_SECONDS: u64 = 1;
const MAX_TIMEOUT_SECONDS: u64 = 6 * 60 * 60;
pub const FFMPEG_PATH_ENV: &str = "AUTOLIVE_FFMPEG_PATH";
pub const FFPROBE_PATH_ENV: &str = "AUTOLIVE_FFPROBE_PATH";
const MEDIA_ENGINE_RESOURCE_DIR: &str = "binaries";
const MEDIA_ENGINE_CAPABILITY_PROBE_TIMEOUT_MS: u64 = 10_000;
const FALLBACK_H264_ENCODER: &str = "libopenh264";
// 各机型候选：按常见硬件加速顺序；运行时探测/编码失败自动降级，不绑死某台机器。
// Windows 才试 MediaFoundation；其它平台跳过 h264_mf。
const H264_ENCODER_CANDIDATES_COMMON: &[&str] = &["h264_nvenc", "h264_amf", "h264_qsv"];
const H264_ENCODER_WINDOWS_EXTRA: &[&str] = &["h264_mf"];
// ponytail: 按 ffmpeg 路径缓存首选；失败后清缓存再探。
static SELECTED_H264_ENCODER: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaEngineStatus {
    pub available: bool,
    pub ffmpeg_version: Option<String>,
    pub ffprobe_version: Option<String>,
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
    pub staging_output_path: PathBuf,
    pub output_mp4_path: PathBuf,
    pub video_processing_enabled: bool,
    pub audio_processing_enabled: bool,
    pub source_audio_sample_rate_hz: Option<u32>,
    pub video: VideoResearchParams,
    pub audio: AudioResearchParams,
    pub research: ResearchExperimentParams,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRenderResult {
    pub output_mp4_path: PathBuf,
    pub output_mp4_sha256: String,
    pub output_size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaEngineError {
    EngineUnavailable { reason: String },
    InvalidExecutable { path: String },
    InvalidInput { path: String },
    InvalidOutputPath { path: String },
    InvalidParameters { message: String },
    InvalidTimeout { seconds: u64 },
    OutputConflict { path: String },
    SpawnFailed { path: String, message: String },
    Failed { code: Option<i32> },
    Timeout { seconds: u64 },
    Cancelled,
    OutputMissing { path: String },
    OutputEmpty { path: String },
    OutputUnreadable { path: String, message: String },
    OutputCommitFailed { from: String, to: String },
    HashFailed { path: String, message: String },
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
            Self::Failed { code } => write!(formatter, "媒体引擎异常退出：exit_code={code:?}"),
            Self::Timeout { seconds } => write!(formatter, "媒体处理超过超时限制：{seconds} 秒"),
            Self::Cancelled => formatter.write_str("媒体处理已取消"),
            Self::OutputMissing { path } => write!(formatter, "媒体处理未生成输出：{path}"),
            Self::OutputEmpty { path } => write!(formatter, "媒体处理输出为空：{path}"),
            Self::OutputUnreadable { path, message } => {
                write!(formatter, "媒体处理输出不可读：{path}；{message}")
            }
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
        reason: None,
    })
}

pub fn build_media_render_args(
    request: &MediaRenderRequest,
) -> Result<Vec<std::ffi::OsString>, MediaEngineError> {
    let preferred = if request.video_processing_enabled {
        Some(select_h264_encoder(&request.ffmpeg_path))
    } else {
        None
    };
    build_media_render_args_with_video_encoder(request, preferred.as_deref())
}

fn build_media_render_args_with_video_encoder(
    request: &MediaRenderRequest,
    video_encoder: Option<&str>,
) -> Result<Vec<std::ffi::OsString>, MediaEngineError> {
    validate_request_shape(request)?;
    let mut args: Vec<std::ffi::OsString> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-y".into(),
        "-i".into(),
        request.input_mp4_path.clone().into_os_string(),
        "-map".into(),
        "0:v:0?".into(),
        "-map".into(),
        "0:a:0?".into(),
    ];

    if request.video_processing_enabled {
        args.extend(["-vf".into(), video_filter(&request.video).into()]);
        let encoder = video_encoder.unwrap_or(FALLBACK_H264_ENCODER);
        args.extend(video_encoder_codec_args(encoder));
    } else {
        args.extend(["-c:v".into(), "copy".into()]);
    }

    if request.audio_processing_enabled {
        args.extend([
            "-af".into(),
            audio_filter(&request.audio, request.source_audio_sample_rate_hz).into(),
        ]);
        args.extend(["-c:a".into(), "aac".into()]);
    } else {
        args.extend(["-c:a".into(), "copy".into()]);
    }

    if request.audio_processing_enabled {
        args.extend([
            "-b:a".into(),
            format!("{}k", request.audio.output_bitrate_kbps).into(),
        ]);
    }
    args.extend([
        "-movflags".into(),
        "+faststart".into(),
        "-f".into(),
        "mp4".into(),
        request.staging_output_path.clone().into_os_string(),
    ]);
    Ok(args)
}

pub fn render_media(
    request: &MediaRenderRequest,
    cancellation: &CancellationToken,
) -> Result<MediaRenderResult, MediaEngineError> {
    validate_request_shape(request)?;
    if request.output_mp4_path.exists() {
        return Err(MediaEngineError::OutputConflict {
            path: request.output_mp4_path.display().to_string(),
        });
    }
    let engine =
        match probe_media_engine_with_paths(&request.ffmpeg_path, &request.ffprobe_path, 5_000) {
            Ok(engine) => engine,
            Err(_error) if cancellation.is_cancelled() => return Err(MediaEngineError::Cancelled),
            Err(error) => {
                return Err(MediaEngineError::EngineUnavailable {
                    reason: error.to_string(),
                })
            }
        };
    if !engine.available {
        return Err(MediaEngineError::EngineUnavailable {
            reason: String::from("FFmpeg/FFprobe 未通过能力探测"),
        });
    }
    if cancellation.is_cancelled() {
        return Err(MediaEngineError::Cancelled);
    }

    let _ignored = fs::remove_file(&request.staging_output_path);
    let encoder_attempts = if request.video_processing_enabled {
        h264_encoder_attempt_order(&request.ffmpeg_path)
    } else {
        vec![FALLBACK_H264_ENCODER.to_owned()]
    };
    let mut last_failure: Option<MediaEngineError> = None;
    let mut encoded_ok = false;
    for encoder in encoder_attempts {
        if cancellation.is_cancelled() {
            cleanup(&request.staging_output_path);
            return Err(MediaEngineError::Cancelled);
        }
        let args = if request.video_processing_enabled {
            build_media_render_args_with_video_encoder(request, Some(&encoder))?
        } else {
            build_media_render_args_with_video_encoder(request, None)?
        };
        let _ignored = fs::remove_file(&request.staging_output_path);
        let mut child = match background_command(&request.ffmpeg_path)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                last_failure = Some(MediaEngineError::SpawnFailed {
                    path: request.ffmpeg_path.display().to_string(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        let started = Instant::now();
        let run_result = loop {
            if cancellation.is_cancelled() {
                terminate_child(&mut child);
                cleanup(&request.staging_output_path);
                return Err(MediaEngineError::Cancelled);
            }
            if started.elapsed() >= Duration::from_secs(request.timeout_seconds) {
                terminate_child(&mut child);
                cleanup(&request.staging_output_path);
                return Err(MediaEngineError::Timeout {
                    seconds: request.timeout_seconds,
                });
            }
            match child.try_wait() {
                Ok(Some(status)) if status.success() => break Ok(()),
                Ok(Some(status)) => {
                    cleanup(&request.staging_output_path);
                    break Err(MediaEngineError::Failed {
                        code: status.code(),
                    });
                }
                Ok(None) => thread::sleep(Duration::from_millis(25)),
                Err(error) => {
                    terminate_child(&mut child);
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
                if request.video_processing_enabled {
                    if let Ok(mut cache) = SELECTED_H264_ENCODER.lock() {
                        *cache = Some((request.ffmpeg_path.clone(), encoder));
                    }
                }
                encoded_ok = true;
                break;
            }
            Err(error) => {
                // 当前编码器不可用（无 GPU/驱动差）：清缓存并试下一个，适配各机型。
                if let Ok(mut cache) = SELECTED_H264_ENCODER.lock() {
                    *cache = None;
                }
                last_failure = Some(error);
            }
        }
    }
    if !encoded_ok {
        return Err(last_failure.unwrap_or(MediaEngineError::Failed { code: None }));
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
    probe_output(
        &request.ffprobe_path,
        &request.staging_output_path,
        request.timeout_seconds,
    )?;
    let output_hash =
        hash_file_at_path(&request.staging_output_path, cancellation).map_err(|error| {
            cleanup(&request.staging_output_path);
            MediaEngineError::HashFailed {
                path: request.staging_output_path.display().to_string(),
                message: error.to_string(),
            }
        })?;
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
    Ok(MediaRenderResult {
        output_mp4_path: request.output_mp4_path.clone(),
        output_mp4_sha256: output_hash,
        output_size_bytes: metadata.len(),
    })
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
            .is_some_and(|value| SUPPORTED_SOURCE_VIDEO_EXTENSIONS.contains(&value))
    {
        return Err(MediaEngineError::InvalidInput {
            path: request.input_mp4_path.display().to_string(),
        });
    }
    for path in [&request.staging_output_path, &request.output_mp4_path] {
        if path.extension().and_then(|value| value.to_str()) != Some("mp4") {
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
    }
    if request.audio_processing_enabled {
        request
            .audio
            .validate()
            .map_err(|errors| MediaEngineError::InvalidParameters {
                message: errors
                    .iter()
                    .map(|error| error.message.clone())
                    .collect::<Vec<_>>()
                    .join("；"),
            })?;
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
    if request.video_processing_enabled {
        let defaults = VideoResearchParams::default();
        let unsupported = [
            (
                "video.crop_edge_smoothing",
                request.video.crop_edge_smoothing,
                defaults.crop_edge_smoothing,
            ),
            (
                "video.frame_rate_jitter_percent",
                request.video.frame_rate_jitter_percent,
                defaults.frame_rate_jitter_percent,
            ),
            (
                "video.frame_rate_perturbation_frequency_hz",
                request.video.frame_rate_perturbation_frequency_hz,
                defaults.frame_rate_perturbation_frequency_hz,
            ),
            (
                "video.frame_rate_perturbation_amplitude_fps",
                request.video.frame_rate_perturbation_amplitude_fps,
                defaults.frame_rate_perturbation_amplitude_fps,
            ),
            (
                "video.frame_inner_perturbation_percent",
                request.video.frame_inner_perturbation_percent,
                defaults.frame_inner_perturbation_percent,
            ),
            (
                "video.frame_inter_perturbation_percent",
                request.video.frame_inter_perturbation_percent,
                defaults.frame_inter_perturbation_percent,
            ),
            (
                "video.color_space_conversion_strength_percent",
                request.video.color_space_conversion_strength_percent,
                defaults.color_space_conversion_strength_percent,
            ),
        ];
        if let Some((field, value, _default)) = unsupported
            .into_iter()
            .find(|(_, value, default)| (value - default).abs() > f64::EPSILON)
        {
            return Err(MediaEngineError::InvalidParameters {
                message: format!("当前媒体 Worker 尚未映射参数 {field}={value}"),
            });
        }
    }
    if request.audio_processing_enabled {
        let defaults = AudioResearchParams::default();
        if request.audio.natural_voice_mode != defaults.natural_voice_mode {
            return Err(MediaEngineError::InvalidParameters {
                message: "当前媒体 Worker 尚未映射参数 audio.natural_voice_mode".to_owned(),
            });
        }
        // random_change_period_ms 仅驱动前端预览调度，不进 FFmpeg，不得当 unsupported 拦截。
        let unsupported = [
            (
                "audio.spectral_perturbation_percent",
                request.audio.spectral_perturbation_percent,
                defaults.spectral_perturbation_percent,
            ),
            (
                "audio.mfcc_shift_percent",
                request.audio.mfcc_shift_percent,
                defaults.mfcc_shift_percent,
            ),
            (
                "audio.ambient_sound_mix_percent",
                request.audio.ambient_sound_mix_percent,
                defaults.ambient_sound_mix_percent,
            ),
            (
                "audio.dry_wet_percent",
                request.audio.dry_wet_percent,
                defaults.dry_wet_percent,
            ),
            (
                "audio.mfcc_dimensions",
                request.audio.mfcc_dimensions as f64,
                defaults.mfcc_dimensions as f64,
            ),
            (
                "audio.snr_variation_db",
                request.audio.snr_variation_db,
                defaults.snr_variation_db,
            ),
            (
                "audio.formant_shift_percent",
                request.audio.formant_shift_percent,
                defaults.formant_shift_percent,
            ),
            (
                "audio.snr_target_db",
                request.audio.snr_target_db.unwrap_or_default(),
                defaults.snr_target_db.unwrap_or_default(),
            ),
            (
                "audio.current_formant_hz",
                request.audio.current_formant_hz.unwrap_or_default(),
                defaults.current_formant_hz.unwrap_or_default(),
            ),
        ];
        if request.audio.voice_library_id != defaults.voice_library_id {
            return Err(MediaEngineError::InvalidParameters {
                message: "当前媒体 Worker 尚未映射参数 audio.voice_library_id".to_owned(),
            });
        }
        // 源采样率缺失时 audio_filter 回退 48000，不因微扰音高拒整次处理。
        if let Some((field, value, _default)) = unsupported
            .into_iter()
            .find(|(_, value, default)| (value - default).abs() > f64::EPSILON)
        {
            return Err(MediaEngineError::InvalidParameters {
                message: format!("当前媒体 Worker 尚未映射参数 {field}={value}"),
            });
        }
    }
    if request.video_processing_enabled && request.research != ResearchExperimentParams::default() {
        return Err(MediaEngineError::InvalidParameters {
            message: "当前媒体 Worker 尚未映射视觉频段、挂件和切片研究参数".to_owned(),
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

fn probe_output(path: &Path, output: &Path, timeout_seconds: u64) -> Result<(), MediaEngineError> {
    let mut args: Vec<std::ffi::OsString> = vec![
        "-hide_banner".into(),
        "-v".into(),
        "error".into(),
        "-show_entries".into(),
        "format=format_name".into(),
        "-of".into(),
        "default=noprint_wrappers=1:nokey=1".into(),
    ];
    args.push(output.as_os_str().to_owned());
    let (status, _stdout) =
        run_command_with_timeout(path, args, timeout_seconds.saturating_mul(1_000))?;
    if !status.success() {
        return Err(MediaEngineError::OutputUnreadable {
            path: output.display().to_string(),
            message: format!(
                "ffprobe 退出码 {:?}（超时预算 {} 秒）",
                status.code(),
                timeout_seconds
            ),
        });
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
    let started = Instant::now();
    let status = loop {
        if started.elapsed() >= Duration::from_millis(timeout_ms) {
            terminate_child(&mut child);
            return Err(MediaEngineError::EngineUnavailable {
                reason: format!("{} 探测超时：{}ms", path.display(), timeout_ms),
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                terminate_child(&mut child);
                return Err(MediaEngineError::SpawnFailed {
                    path: path.display().to_string(),
                    message: error.to_string(),
                });
            }
        }
    };
    let mut stdout = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        pipe.read_to_end(&mut stdout)
            .map_err(|error| MediaEngineError::SpawnFailed {
                path: path.display().to_string(),
                message: error.to_string(),
            })?;
    }
    Ok((status, stdout))
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

/// 渲染时尝试顺序：缓存首选优先，再按候选列表，最后强制软编。
fn h264_encoder_attempt_order(ffmpeg_path: &Path) -> Vec<String> {
    let preferred = select_h264_encoder(ffmpeg_path);
    let mut order = vec![preferred];
    for candidate in h264_encoder_candidates() {
        if !order.iter().any(|name| name == candidate) {
            order.push((*candidate).to_owned());
        }
    }
    if !order.iter().any(|name| name == FALLBACK_H264_ENCODER) {
        order.push(FALLBACK_H264_ENCODER.to_owned());
    }
    order
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
        "color=c=black:s=64x64:d=0.04".into(),
        "-frames:v".into(),
        "1".into(),
        "-an".into(),
    ];
    args.extend(video_encoder_codec_args(encoder));
    args.extend(["-f".into(), "null".into(), null_output.into()]);
    match run_command_with_timeout(ffmpeg_path, args, 8_000) {
        Ok((status, _)) => status.success(),
        Err(_) => false,
    }
}

fn video_encoder_codec_args(encoder: &str) -> Vec<std::ffi::OsString> {
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
        // 任意机器最终兜底：OpenH264 软编。
        _ => vec!["-c:v".into(), FALLBACK_H264_ENCODER.into()],
    }
}

fn video_filter(video: &VideoResearchParams) -> String {
    // 当前打包 FFmpeg 无 eq/boxblur；用 lutyuv + hue + gblur 等价映射。
    let brightness = (video.brightness_percent / 100.0).clamp(-1.0, 1.0);
    let contrast = (video.contrast_percent / 100.0).clamp(0.0, 2.0);
    let saturation = (video.saturation_percent / 100.0).clamp(0.0, 3.0);
    let mut filters = vec![format!(
        "lutyuv=y='clip((val-128)*{contrast:.6}+128+{brightness:.6}*128,0,255)'"
    )];
    if (saturation - 1.0).abs() > f64::EPSILON || video.hue_rotation_degrees != 0.0 {
        filters.push(format!(
            "hue=h={:.6}:s={saturation:.6}",
            video.hue_rotation_degrees
        ));
    }
    if video.blur_radius_px > 0.0 {
        filters.push(format!("gblur=sigma={:.6}", video.blur_radius_px.max(0.01)));
    }
    let detail_strength = (video.sharpen_percent + video.detail_enhancement_percent) / 100.0;
    if detail_strength > 0.0 {
        filters.push(format!("unsharp=5:5:{detail_strength:.6}"));
    }
    if video.noise_percent > 0.0 {
        filters.push(format!("noise=alls={:.6}:allf=t+u", video.noise_percent));
    }
    if video.dynamic_crop_percent > 0.0 {
        let crop = video.dynamic_crop_percent / 100.0;
        filters.push(format!(
            "crop=iw*(1-{double_crop:.6}):ih*(1-{double_crop:.6}):iw*{crop:.6}+iw*{crop:.6}*sin(n*0.07):ih*{crop:.6}+ih*{crop:.6}*cos(n*0.09),scale=iw/(1-{double_crop:.6}):ih/(1-{double_crop:.6})",
            double_crop = crop * 2.0,
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
    filters.join(",")
}

fn audio_filter(audio: &AudioResearchParams, source_sample_rate_hz: Option<u32>) -> String {
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
    if audio.pitch_shift_semitones.abs() > f64::EPSILON {
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
    if audio.fade_out_ms > 0 {
        filters.push(format!(
            "afade=t=out:d={:.6}",
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
        let depth = (audio.phase_perturbation_percent.abs() / 20.0).clamp(0.0, 1.0);
        filters.push(format!(
            "aphaser=in_gain=0.4:out_gain=0.74:delay=3:decay={depth:.6}:speed=0.5"
        ));
    }
    if audio.vibrato_depth_percent > 0.0 {
        let depth = (audio.vibrato_depth_percent / 100.0).clamp(0.0, 1.0);
        filters.push(format!(
            "vibrato=f={:.6}:d={depth:.6}",
            audio.vibrato_frequency_hz
        ));
    }
    if audio.environment_noise_percent > 0.0 {
        let ratio = (audio.environment_noise_percent / 100.0).clamp(0.0, 1.0);
        let amplitude =
            (10_f64.powf(audio.environment_noise_dbfs / 20.0) * ratio).clamp(0.000_001, 1.0);
        let dry_weight = (1.0 - ratio).max(0.0);
        filters.push(format!(
            "asplit=1[a];anoisesrc=color=white:amplitude={amplitude:.6}[noise];[a][noise]amix=inputs=2:weights={dry_weight:.6} {ratio:.6}:duration=first:dropout_transition=0"
        ));
    }
    if let Some(sample_rate_hz) = audio.sample_rate_hz {
        if Some(sample_rate_hz) != source_sample_rate_hz {
            filters.push(format!("aresample={sample_rate_hz}"));
        }
    }
    filters.join(",")
}

fn cleanup(path: &Path) {
    let _ignored = fs::remove_file(path);
}

fn terminate_child(child: &mut std::process::Child) {
    let _ignored = child.kill();
    let _ignored = child.wait();
}

#[cfg(test)]
mod tests {
    use super::{
        configured_media_engine_paths_with_resource_dir, packaged_media_engine_paths, target_triple,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;

    static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

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
