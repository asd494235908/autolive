use crate::audio_pcm_effects::AudioPcmEffectConfig;
use crate::background_process::background_command;
use crate::bounded_io::{read_to_end_bounded, BoundedReadError};
use crate::cancellation::CancellationToken;
use crate::errors::FileHashError;
use crate::hashing::hash_file_at_path;
use crate::media_audio_effects::{build_offline_audio_effect_plan, MfccOperation};
use crate::media_effect_params::{AdvancedEffectParams, AudioEffectParams, VideoEffectParams};
use crate::media_library::SUPPORTED_SOURCE_VIDEO_EXTENSIONS;
use crate::media_video_effects::{
    build_media_video_complex_effect_plan, build_media_video_effect_plan,
};
use autolive_signalsmith_stretch::QualityPitchConfig;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
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
const MAX_AUDIO_VARIANTS: usize = 4;
pub const FFMPEG_PATH_ENV: &str = "AUTOLIVE_FFMPEG_PATH";
pub const FFPROBE_PATH_ENV: &str = "AUTOLIVE_FFPROBE_PATH";
const MEDIA_ENGINE_RESOURCE_DIR: &str = "binaries";
const MEDIA_ENGINE_CAPABILITY_PROBE_TIMEOUT_MS: u64 = 10_000;
const FALLBACK_H264_ENCODER: &str = "libopenh264";
const MAX_MEDIA_STDERR_BYTES: usize = 64 * 1024;
const MAX_PROBE_STDOUT_BYTES: usize = 512 * 1024;
const DEFAULT_AUDIO_OUTPUT_SAMPLE_RATE_HZ: u32 = 48_000;
const MIN_AUDIO_CONTENT_PEAK_DB: f64 = -80.0;
const AUDIO_FINITE_GUARD_FILTER: &str =
    "aeval=exprs=if(isnan(val(0))\\,0\\,if(isinf(val(0))\\,0\\,val(0)))\\|if(isnan(val(1))\\,0\\,if(isinf(val(1))\\,0\\,val(1)))";
// 各机型候选：按常见硬件加速顺序；运行时探测/编码失败自动降级，不绑死某台机器。
// Windows 才试 MediaFoundation；其它平台跳过 h264_mf。
const H264_ENCODER_CANDIDATES_COMMON: &[&str] = &["h264_nvenc", "h264_amf", "h264_qsv"];
const H264_ENCODER_WINDOWS_EXTRA: &[&str] = &["h264_mf"];
// ponytail: 按 ffmpeg 路径缓存首选；失败后清缓存再探。
static SELECTED_H264_ENCODER: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);

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
    pub video: VideoEffectParams,
    pub audio: AudioEffectParams,
    /// 多虚拟轨音频参数；空表示使用 `audio`，最多允许四条支路。
    pub audio_variants: Vec<AudioEffectParams>,
    pub advanced: AdvancedEffectParams,
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
        "-loglevel".into(),
        "error".into(),
        "-nostats".into(),
        "-nostdin".into(),
        "-y".into(),
        "-i".into(),
        request.input_mp4_path.clone().into_os_string(),
    ];
    let filter_option_index = args.len();
    let mut complex_graph_parts = Vec::with_capacity(2);

    if request.video_processing_enabled {
        let filter_plan = video_filter(&request.video, &request.advanced)?;
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
        complex_graph_parts.push(audio_mix_filter_complex(
            &request.audio,
            &variants,
            request.source_audio_sample_rate_hz,
            Some(output_sample_rate_hz),
            false,
            false,
        )?);
        args.extend([
            "-map".into(),
            "[aout]".into(),
            "-c:a".into(),
            "aac".into(),
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
            .stderr(Stdio::piped())
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
        let mut stderr_reader = child
            .stderr
            .take()
            .map(|stderr| thread::spawn(move || read_stderr_tail(stderr)));
        let started = Instant::now();
        let run_result = loop {
            if cancellation.is_cancelled() {
                terminate_child(&mut child);
                let _ = join_stderr_reader(&mut stderr_reader);
                cleanup(&request.staging_output_path);
                return Err(MediaEngineError::Cancelled);
            }
            if started.elapsed() >= Duration::from_secs(request.timeout_seconds) {
                terminate_child(&mut child);
                let _ = join_stderr_reader(&mut stderr_reader);
                cleanup(&request.staging_output_path);
                return Err(MediaEngineError::Timeout {
                    seconds: request.timeout_seconds,
                });
            }
            match child.try_wait() {
                Ok(Some(status)) if status.success() => {
                    let _ = join_stderr_reader(&mut stderr_reader);
                    break Ok(());
                }
                Ok(Some(status)) => {
                    let stderr = join_stderr_reader(&mut stderr_reader);
                    cleanup(&request.staging_output_path);
                    break Err(MediaEngineError::Failed {
                        code: status.code(),
                        stderr,
                    });
                }
                Ok(None) => thread::sleep(Duration::from_millis(25)),
                Err(error) => {
                    terminate_child(&mut child);
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
    if let Err(error) = probe_output(
        &request.ffprobe_path,
        &request.staging_output_path,
        request.timeout_seconds,
        expected_audio_sample_rate_hz,
    ) {
        cleanup(&request.staging_output_path);
        return Err(error);
    }
    if request.audio_processing_enabled {
        if let Err(error) = validate_audio_content(
            &request.ffmpeg_path,
            &request.staging_output_path,
            request.timeout_seconds,
            cancellation,
        ) {
            cleanup(&request.staging_output_path);
            return Err(error);
        }
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
    if request.video_processing_enabled {
        build_media_video_effect_plan(&request.video, &request.advanced).map_err(|error| {
            MediaEngineError::InvalidParameters {
                message: error.to_string(),
            }
        })?;
    }
    if request.audio_processing_enabled {
        validate_audio_filter_support(&request.audio, "audio", false, false)?;
    }
    // variants 是 IPC 输入的一部分；即使当前关闭声音处理，也不能借此绕过
    // 未映射参数校验，避免后续切换开关后把非法预设带入媒体链。
    for (index, audio) in request.audio_variants.iter().enumerate() {
        validate_audio_filter_support(
            audio,
            &format!("audio_variants[{index}]"),
            !request.audio_processing_enabled,
            false,
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
    allow_pcm_runtime: bool,
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
    if !allow_pcm_runtime
        && (plan.snr.is_some()
            || plan
                .analysis
                .mfcc
                .as_ref()
                .is_some_and(|mfcc| mfcc.operation == MfccOperation::ShiftAndReconstruct))
    {
        return Err(MediaEngineError::InvalidParameters {
            message: format!(
                "{field_prefix} 的 MFCC/SNR 参数只允许进入 PortAudio 混音后 PCM 处理链"
            ),
        });
    }
    if !allow_pcm_runtime
        && (audio.formant_shift_percent - defaults.formant_shift_percent).abs() > f64::EPSILON
    {
        return Err(MediaEngineError::InvalidParameters {
            message: format!(
                "当前媒体 Worker 尚未映射参数 {field_prefix}.formant_shift_percent={}",
                audio.formant_shift_percent
            ),
        });
    }
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
    timeout_seconds: u64,
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
    let (status, stdout) =
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
    timeout_seconds: u64,
    cancellation: &CancellationToken,
) -> Result<(), MediaEngineError> {
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
        .map(|stderr| thread::spawn(move || read_stderr_tail(stderr)));
    let started = Instant::now();
    let status = loop {
        if cancellation.is_cancelled() {
            terminate_child(&mut child);
            let _ = join_stderr_reader(&mut stderr_reader);
            return Err(MediaEngineError::Cancelled);
        }
        if started.elapsed() >= Duration::from_secs(timeout_seconds) {
            terminate_child(&mut child);
            let _ = join_stderr_reader(&mut stderr_reader);
            return Err(MediaEngineError::Timeout {
                seconds: timeout_seconds,
            });
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                terminate_child(&mut child);
                let _ = join_stderr_reader(&mut stderr_reader);
                return Err(MediaEngineError::SpawnFailed {
                    path: path.display().to_string(),
                    message: error.to_string(),
                });
            }
        }
    };
    let stderr = join_stderr_reader(&mut stderr_reader);
    if !status.success() {
        return Err(MediaEngineError::OutputUnreadable {
            path: output.display().to_string(),
            message: format!(
                "音频内容探测退出码 {:?}：{}",
                status.code(),
                stderr.unwrap_or_else(|| "未返回诊断信息".to_owned())
            ),
        });
    }
    let max_volume_db = stderr
        .as_deref()
        .and_then(parse_max_volume_db)
        .ok_or_else(|| MediaEngineError::OutputUnreadable {
            path: output.display().to_string(),
            message: "volumedetect 未返回有效 max_volume".to_owned(),
        })?;
    if max_volume_db <= MIN_AUDIO_CONTENT_PEAK_DB {
        return Err(MediaEngineError::OutputSilent {
            path: output.display().to_string(),
            max_volume_db: format!("{max_volume_db:.2}"),
        });
    }
    Ok(())
}

fn parse_max_volume_db(stderr: &str) -> Option<f64> {
    stderr.lines().find_map(|line| {
        let value = line.split_once("max_volume:")?.1.trim();
        let value = value.strip_suffix("dB")?.trim();
        value.parse::<f64>().ok().filter(|value| value.is_finite())
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

struct VideoFilterPlan {
    serial_filter: String,
    complex_graph: Option<String>,
    requires_variable_frame_rate: bool,
}

fn video_filter(
    video: &VideoEffectParams,
    advanced: &AdvancedEffectParams,
) -> Result<VideoFilterPlan, MediaEngineError> {
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
        filters.push(format!("noise=alls={:.6}:allf=t+u", video.noise_percent));
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
    let split_labels = (0..k).map(|index| format!("a{index}")).collect::<Vec<_>>();
    parts.push(format!(
        "[0:a:0]asplit={k}{}",
        split_labels
            .iter()
            .map(|label| format!("[{label}]"))
            .collect::<String>()
    ));
    let mut mixed_inputs = String::new();
    let mut weights = Vec::with_capacity(k);
    for (index, audio) in variants.iter().enumerate() {
        let branch = audio_branch_filter(audio, source_sample_rate_hz, realtime)?;
        parts.push(format!("[a{index}]{branch}[processed{index}]"));
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
            // 源时长由 duration=first 裁切，避免短片尾部被噪声源延长。
            parts.push(format!(
                "anoisesrc=color=white:amplitude={amplitude:.6}:d=86400[noise{index}];[{branch_input}][noise{index}]amix=inputs=2:weights={dry_weight:.6} {ratio:.6}:duration=first:dropout_transition=0[b{index}]"
            ));
        } else {
            parts.push(format!("[{branch_input}]anull[b{index}]"));
        }
        mixed_inputs.push_str(&format!("[b{index}]"));
        weights.push(format!("{:.6}", 1.0 / k as f64));
    }
    let output_sample_rate_hz = effective_audio_output_sample_rate_hz(
        variants,
        source_sample_rate_hz,
        preferred_output_sample_rate_hz,
    );
    parts.push(format!(
        "{mixed_inputs}amix=inputs={k}:weights={}:duration=first:dropout_transition=0[variant_bus]",
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
            "[1:a:0]aformat=sample_fmts=fltp:channel_layouts=stereo,aresample={output_sample_rate_hz}:async=1:first_pts=0[ambient];{}",
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
    let _ignored = child.kill();
    let _ignored = child.wait();
}

fn read_stderr_tail(mut stderr: impl Read) -> Option<String> {
    let mut retained = VecDeque::with_capacity(MAX_MEDIA_STDERR_BYTES);
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stderr.read(&mut buffer).ok()?;
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
    if compact.is_empty() {
        None
    } else {
        Some(compact.chars().take(500).collect())
    }
}

fn join_stderr_reader(reader: &mut Option<thread::JoinHandle<Option<String>>>) -> Option<String> {
    reader
        .take()
        .and_then(|handle| handle.join().ok().flatten())
}

#[cfg(test)]
mod tests {
    use super::{
        audio_mix_filter_complex, build_audio_stream_filter_graph,
        build_audio_stream_filter_graph_with_ambient,
        configured_media_engine_paths_with_resource_dir, packaged_media_engine_paths,
        parse_max_volume_db, run_command_with_timeout, target_triple,
        validate_audio_input_decodable, validate_audio_probe_output, video_filter,
        MediaEngineError, AUDIO_FINITE_GUARD_FILTER, MAX_PROBE_STDOUT_BYTES,
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
            "[1:a:0]aformat=",
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
        assert_eq!(parse_max_volume_db("max_volume: -91.0 dB"), Some(-91.0));
        assert_eq!(parse_max_volume_db("volumedetect failed"), None);
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
