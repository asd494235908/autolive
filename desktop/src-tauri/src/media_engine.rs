use crate::cancellation::CancellationToken;
use crate::hashing::hash_file_at_path;
use crate::research_params::{AudioResearchParams, ResearchExperimentParams, VideoResearchParams};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const MIN_TIMEOUT_SECONDS: u64 = 1;
const MAX_TIMEOUT_SECONDS: u64 = 6 * 60 * 60;
pub const FFMPEG_PATH_ENV: &str = "AUTOLIVE_FFMPEG_PATH";
pub const FFPROBE_PATH_ENV: &str = "AUTOLIVE_FFPROBE_PATH";
const MEDIA_ENGINE_RESOURCE_DIR: &str = "binaries";

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
    resource_dir: &Path,
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
        resource_dir
            .join(MEDIA_ENGINE_RESOURCE_DIR)
            .join(format!("ffmpeg{extension}")),
        resource_dir
            .join(MEDIA_ENGINE_RESOURCE_DIR)
            .join(format!("ffprobe{extension}")),
    ))
}

pub fn configured_media_engine_paths_with_resource_dir(
    resource_dir: &Path,
) -> Result<(PathBuf, PathBuf), MediaEngineError> {
    if std::env::var_os(FFMPEG_PATH_ENV).is_some() || std::env::var_os(FFPROBE_PATH_ENV).is_some() {
        return configured_media_engine_paths();
    }
    packaged_media_engine_paths(resource_dir)
}

pub fn configured_media_engine_status() -> MediaEngineStatus {
    media_engine_status_for_paths(configured_media_engine_paths())
}

pub fn configured_media_engine_status_with_resource_dir(resource_dir: &Path) -> MediaEngineStatus {
    media_engine_status_for_paths(configured_media_engine_paths_with_resource_dir(
        resource_dir,
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
    match probe_media_engine_with_paths(&ffmpeg_path, &ffprobe_path, 1_000) {
        Ok(status) => status,
        Err(error) => MediaEngineStatus {
            available: false,
            ffmpeg_version: None,
            ffprobe_version: None,
            reason: Some(error.to_string()),
        },
    }
}

fn target_triple() -> &'static str {
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
        args.extend([
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            "veryfast".into(),
        ]);
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
    let args = build_media_render_args(request)?;
    let mut child = Command::new(&request.ffmpeg_path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| MediaEngineError::SpawnFailed {
            path: request.ffmpeg_path.display().to_string(),
            message: error.to_string(),
        })?;
    let started = Instant::now();
    loop {
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
            Ok(Some(status)) if status.success() => break,
            Ok(Some(status)) => {
                cleanup(&request.staging_output_path);
                return Err(MediaEngineError::Failed {
                    code: status.code(),
                });
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                terminate_child(&mut child);
                cleanup(&request.staging_output_path);
                return Err(MediaEngineError::SpawnFailed {
                    path: request.ffmpeg_path.display().to_string(),
                    message: error.to_string(),
                });
            }
        }
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
    if !request.input_mp4_path.is_file()
        || request
            .input_mp4_path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
            != Some("mp4")
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
        let unsupported = [
            (
                "audio.random_change_period_ms",
                request.audio.random_change_period_ms as f64,
                defaults.random_change_period_ms as f64,
            ),
            (
                "audio.spectral_perturbation_percent",
                request.audio.spectral_perturbation_percent,
                defaults.spectral_perturbation_percent,
            ),
            (
                "audio.environment_noise_percent",
                request.audio.environment_noise_percent,
                defaults.environment_noise_percent,
            ),
            (
                "audio.environment_noise_dbfs",
                request.audio.environment_noise_dbfs,
                defaults.environment_noise_dbfs,
            ),
            (
                "audio.mfcc_shift_percent",
                request.audio.mfcc_shift_percent,
                defaults.mfcc_shift_percent,
            ),
            (
                "audio.phase_perturbation_percent",
                request.audio.phase_perturbation_percent,
                defaults.phase_perturbation_percent,
            ),
            (
                "audio.dry_wet_percent",
                request.audio.dry_wet_percent,
                defaults.dry_wet_percent,
            ),
            (
                "audio.reverb_wet_percent",
                request.audio.reverb_wet_percent,
                defaults.reverb_wet_percent,
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
                "audio.vibrato_frequency_hz",
                request.audio.vibrato_frequency_hz,
                defaults.vibrato_frequency_hz,
            ),
            (
                "audio.vibrato_depth_percent",
                request.audio.vibrato_depth_percent,
                defaults.vibrato_depth_percent,
            ),
            (
                "audio.spectrum_blind_spot_percent",
                request.audio.spectrum_blind_spot_percent,
                defaults.spectrum_blind_spot_percent,
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
            ("audio.filter_q", request.audio.filter_q, defaults.filter_q),
        ];
        if request.audio.voice_library_id != defaults.voice_library_id {
            return Err(MediaEngineError::InvalidParameters {
                message: "当前媒体 Worker 尚未映射参数 audio.voice_library_id".to_owned(),
            });
        }
        if request.audio.pitch_shift_semitones.abs() > f64::EPSILON
            && request.source_audio_sample_rate_hz.is_none()
        {
            return Err(MediaEngineError::InvalidParameters {
                message: "音高微移需要源音轨采样率".to_owned(),
            });
        }
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
    let mut child = Command::new(path)
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

fn video_filter(video: &VideoResearchParams) -> String {
    let mut filters = vec![format!(
        "eq=brightness={:.6}:contrast={:.6}:saturation={:.6}",
        video.brightness_percent / 100.0,
        video.contrast_percent / 100.0,
        video.saturation_percent / 100.0
    )];
    if video.hue_rotation_degrees != 0.0 {
        filters.push(format!("hue=h={:.6}", video.hue_rotation_degrees));
    }
    if video.blur_radius_px > 0.0 {
        filters.push(format!("boxblur={:.6}", video.blur_radius_px));
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
    if audio.pitch_shift_semitones.abs() > f64::EPSILON {
        if let Some(source_sample_rate_hz) = source_sample_rate_hz {
            let factor = 2_f64.powf(audio.pitch_shift_semitones / 12.0);
            let shifted_sample_rate_hz = (f64::from(source_sample_rate_hz) * factor).round();
            filters.push(format!("asetrate={shifted_sample_rate_hz:.0}"));
            filters.push(format!("aresample={source_sample_rate_hz}"));
            filters.push(format!("atempo={:.6}", 1.0 / factor));
        }
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
