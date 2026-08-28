use crate::background_process::background_command;
use crate::cancellation::CancellationToken;
use crate::media_engine::{h264_encoder_attempt_order, video_encoder_codec_args};
use crate::media_library::{
    probe_user_selected_video_with_ffprobe, MediaCompatibilityMode, MediaKind,
    MediaProbeRequestDto, SourceMediaDto,
};
use std::fmt::{Display, Formatter};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const MAX_FFMPEG_STDERR_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone)]
pub struct MediaCompatibilityRequest {
    pub ffmpeg_path: PathBuf,
    pub ffprobe_path: PathBuf,
    pub cache_dir: PathBuf,
    pub source: SourceMediaDto,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedMediaCompatibility {
    pub playback_reference: String,
    pub mode: MediaCompatibilityMode,
    pub created_file: Option<PathBuf>,
}

#[derive(Debug)]
pub enum MediaCompatibilityError {
    Cancelled,
    InvalidRequest(String),
    Io(String),
    FfmpegFailed(String),
    Timeout(u64),
    OutputInvalid(String),
}

impl Display for MediaCompatibilityError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("媒体兼容准备已取消"),
            Self::InvalidRequest(message) => write!(formatter, "媒体兼容准备请求无效：{message}"),
            Self::Io(message) => write!(formatter, "媒体兼容缓存读写失败：{message}"),
            Self::FfmpegFailed(message) => {
                write!(formatter, "视频播放兼容转换失败：{message}")
            }
            Self::Timeout(seconds) => {
                write!(formatter, "视频播放兼容转换超时（{seconds} 秒）")
            }
            Self::OutputInvalid(message) => {
                write!(formatter, "视频播放兼容产物无效：{message}")
            }
        }
    }
}

impl std::error::Error for MediaCompatibilityError {}

pub fn prepare_media_compatibility(
    request: &MediaCompatibilityRequest,
    cancellation: &CancellationToken,
) -> Result<PreparedMediaCompatibility, MediaCompatibilityError> {
    if cancellation.is_cancelled() {
        return Err(MediaCompatibilityError::Cancelled);
    }
    if !requires_video_playback_compatibility(&request.source) {
        return Ok(PreparedMediaCompatibility {
            playback_reference: request.source.source_path.clone(),
            mode: MediaCompatibilityMode::Direct,
            created_file: None,
        });
    }
    if request.timeout_seconds == 0 {
        return Err(MediaCompatibilityError::InvalidRequest(
            "超时预算必须大于 0 秒".to_owned(),
        ));
    }
    for executable in [&request.ffmpeg_path, &request.ffprobe_path] {
        if !executable.is_file() {
            return Err(MediaCompatibilityError::InvalidRequest(format!(
                "媒体引擎不可用：{}",
                executable.display()
            )));
        }
    }
    if !Path::new(&request.source.source_path).is_file() {
        return Err(MediaCompatibilityError::InvalidRequest(format!(
            "源媒体不可读：{}",
            request.source.source_path
        )));
    }
    fs::create_dir_all(&request.cache_dir).map_err(|error| {
        MediaCompatibilityError::Io(format!(
            "无法创建目录 {}：{error}",
            request.cache_dir.display()
        ))
    })?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| MediaCompatibilityError::Io(error.to_string()))?
        .as_nanos();
    let base_name = format!("ts-compat-{}-{nonce}", std::process::id());
    let output_path = request.cache_dir.join(format!("{base_name}.mp4"));
    let staging_path = request.cache_dir.join(format!("{base_name}.partial.mp4"));
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(request.timeout_seconds))
        .ok_or_else(|| MediaCompatibilityError::InvalidRequest("超时预算溢出".to_owned()))?;

    let result = prepare_video_inner(request, &staging_path, &output_path, deadline, cancellation);
    if result.is_err() {
        let _ = fs::remove_file(&staging_path);
        let _ = fs::remove_file(&output_path);
    }
    result
}

fn requires_video_playback_compatibility(source: &SourceMediaDto) -> bool {
    source.media_kind == MediaKind::Video
}

fn prepare_video_inner(
    request: &MediaCompatibilityRequest,
    staging_path: &Path,
    output_path: &Path,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<PreparedMediaCompatibility, MediaCompatibilityError> {
    let mut last_error = None;
    for encoder in h264_encoder_attempt_order(&request.ffmpeg_path) {
        let _ = fs::remove_file(staging_path);
        let args = build_transcode_args(
            Path::new(&request.source.source_path),
            staging_path,
            &encoder,
            &request.source,
        );
        match run_ffmpeg(
            &request.ffmpeg_path,
            args,
            deadline,
            request.timeout_seconds,
            cancellation,
        ) {
            Ok(()) => {
                last_error = None;
                break;
            }
            Err(MediaCompatibilityError::FfmpegFailed(message)) => {
                last_error = Some(MediaCompatibilityError::FfmpegFailed(message));
            }
            Err(error) => return Err(error),
        }
    }
    if let Some(error) = last_error {
        return Err(error);
    }

    let metadata = fs::metadata(staging_path).map_err(|error| {
        MediaCompatibilityError::OutputInvalid(format!(
            "产物不存在 {}：{error}",
            staging_path.display()
        ))
    })?;
    if metadata.len() == 0 {
        return Err(MediaCompatibilityError::OutputInvalid(
            "产物为空文件".to_owned(),
        ));
    }
    let remaining_ms = remaining_millis(deadline, request.timeout_seconds)?;
    let probed = probe_user_selected_video_with_ffprobe(
        &MediaProbeRequestDto {
            path: staging_path.display().to_string(),
        },
        &request.ffprobe_path,
        remaining_ms,
        cancellation,
    )
    .map_err(|error| MediaCompatibilityError::OutputInvalid(error.to_string()))?;
    if probed.source.media_kind != MediaKind::Video {
        return Err(MediaCompatibilityError::OutputInvalid(
            "产物没有真实视频轨道".to_owned(),
        ));
    }
    if probed
        .source
        .video_codec_name
        .as_deref()
        .is_none_or(|codec| !codec.eq_ignore_ascii_case("h264"))
    {
        return Err(MediaCompatibilityError::OutputInvalid(
            "产物视频编码不是 H.264".to_owned(),
        ));
    }
    if request.source.audio_codec_name.is_some() && probed.source.audio_codec_name.is_none() {
        return Err(MediaCompatibilityError::OutputInvalid(
            "源媒体包含音频，但兼容产物缺少音频轨道".to_owned(),
        ));
    }
    if probed
        .source
        .audio_codec_name
        .as_deref()
        .is_some_and(|codec| !codec.eq_ignore_ascii_case("aac"))
    {
        return Err(MediaCompatibilityError::OutputInvalid(
            "产物音频编码不是 AAC".to_owned(),
        ));
    }
    if output_path.exists() {
        return Err(MediaCompatibilityError::Io(format!(
            "拒绝覆盖已有缓存：{}",
            output_path.display()
        )));
    }
    fs::rename(staging_path, output_path).map_err(|error| {
        MediaCompatibilityError::Io(format!(
            "无法原子提交 {} 到 {}：{error}",
            staging_path.display(),
            output_path.display()
        ))
    })?;
    let canonical_output = fs::canonicalize(output_path).map_err(|error| {
        MediaCompatibilityError::Io(format!(
            "无法规范化兼容缓存 {}：{error}",
            output_path.display()
        ))
    })?;
    Ok(PreparedMediaCompatibility {
        playback_reference: canonical_output.display().to_string(),
        mode: MediaCompatibilityMode::Transcoded,
        created_file: Some(canonical_output),
    })
}

fn build_transcode_args(
    input_path: &Path,
    staging_path: &Path,
    encoder: &str,
    source: &SourceMediaDto,
) -> Vec<std::ffi::OsString> {
    let has_audio = source.audio_codec_name.is_some();
    let frame_rate_fps = source
        .frame_rate_fps
        .filter(|value| value.is_finite() && (1.0..=240.0).contains(value))
        .unwrap_or(30.0);
    let gop = frame_rate_fps.ceil().clamp(1.0, 240.0) as u32;
    let high_resolution = source
        .width
        .zip(source.height)
        .is_some_and(|(width, height)| u64::from(width) * u64::from(height) > 1920 * 1080);
    let level = if high_resolution || frame_rate_fps > 60.0 {
        "5.2"
    } else {
        "4.2"
    };
    let mut args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostats".into(),
        "-nostdin".into(),
        "-y".into(),
        "-fflags".into(),
        "+genpts".into(),
        "-i".into(),
        input_path.as_os_str().to_owned(),
        "-map".into(),
        "0:v:0".into(),
        "-vf".into(),
        format!(
            "fps=fps={frame_rate_fps:.6}:start_time=0:round=near,format=yuv420p,setsar=1,setpts=PTS-STARTPTS"
        )
        .into(),
    ];
    if has_audio {
        args.extend(["-map".into(), "0:a:0".into()]);
    } else {
        args.push("-an".into());
    }
    args.extend(video_encoder_codec_args(encoder));
    args.extend([
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-profile:v".into(),
        "main".into(),
        "-level:v".into(),
        level.into(),
        "-fps_mode:v".into(),
        "cfr".into(),
        "-g".into(),
        gop.to_string().into(),
        "-keyint_min".into(),
        gop.to_string().into(),
        "-sc_threshold".into(),
        "0".into(),
        "-bf".into(),
        "0".into(),
        "-flags".into(),
        "+cgop".into(),
        "-force_key_frames".into(),
        "expr:gte(t,n_forced*1)".into(),
    ]);
    if has_audio {
        args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()]);
    }
    args.extend([
        "-sn".into(),
        "-dn".into(),
        "-avoid_negative_ts".into(),
        "disabled".into(),
        "-use_editlist".into(),
        "1".into(),
        "-video_track_timescale".into(),
        "90000".into(),
        "-movflags".into(),
        "+faststart".into(),
        "-f".into(),
        "mp4".into(),
        staging_path.as_os_str().to_owned(),
    ]);
    args
}

fn run_ffmpeg(
    ffmpeg_path: &Path,
    args: Vec<std::ffi::OsString>,
    deadline: Instant,
    timeout_seconds: u64,
    cancellation: &CancellationToken,
) -> Result<(), MediaCompatibilityError> {
    if cancellation.is_cancelled() {
        return Err(MediaCompatibilityError::Cancelled);
    }
    let mut command = background_command(ffmpeg_path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(|error| {
        MediaCompatibilityError::Io(format!("无法启动 {}：{error}", ffmpeg_path.display()))
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        terminate_child(&mut child);
        MediaCompatibilityError::Io("无法读取 FFmpeg 错误输出".to_owned())
    })?;
    let stderr_reader = thread::spawn(move || {
        let mut output = Vec::new();
        stderr
            .take(MAX_FFMPEG_STDERR_BYTES)
            .read_to_end(&mut output)
            .map(|_| output)
    });
    let status = loop {
        if cancellation.is_cancelled() {
            terminate_child(&mut child);
            let _ = stderr_reader.join();
            return Err(MediaCompatibilityError::Cancelled);
        }
        if Instant::now() >= deadline {
            terminate_child(&mut child);
            let _ = stderr_reader.join();
            return Err(MediaCompatibilityError::Timeout(timeout_seconds));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                terminate_child(&mut child);
                let _ = stderr_reader.join();
                return Err(MediaCompatibilityError::Io(format!(
                    "等待 FFmpeg 失败：{error}"
                )));
            }
        }
    };
    let stderr = stderr_reader
        .join()
        .map_err(|_| MediaCompatibilityError::Io("FFmpeg 错误输出线程异常退出".to_owned()))?
        .map_err(|error| MediaCompatibilityError::Io(format!("读取 FFmpeg 输出失败：{error}")))?;
    if status.success() {
        Ok(())
    } else {
        let message = String::from_utf8_lossy(&stderr).trim().to_owned();
        Err(MediaCompatibilityError::FfmpegFailed(
            if message.is_empty() {
                format!("退出码 {:?}", status.code())
            } else {
                message
            },
        ))
    }
}

fn remaining_millis(
    deadline: Instant,
    timeout_seconds: u64,
) -> Result<u64, MediaCompatibilityError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .map(|remaining| {
            u64::try_from(remaining.as_millis())
                .unwrap_or(u64::MAX)
                .max(1)
        })
        .ok_or(MediaCompatibilityError::Timeout(timeout_seconds))
}

fn terminate_child(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let process_group = format!("-{}", child.id());
        let _ = background_command("/bin/kill")
            .args(["-KILL", process_group.as_str()])
            .status();
    }
    #[cfg(windows)]
    {
        let _ = background_command("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub fn cleanup_compatibility_files(paths: impl IntoIterator<Item = PathBuf>) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_transcode_args, prepare_media_compatibility, requires_video_playback_compatibility,
        MediaCompatibilityError, MediaCompatibilityRequest,
    };
    use crate::cancellation::CancellationToken;
    use crate::media_library::{MediaCompatibilityMode, MediaKind, SourceMediaDto};
    use std::path::{Path, PathBuf};

    fn strings(values: Vec<std::ffi::OsString>) -> Vec<String> {
        values
            .into_iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect()
    }

    fn ts_source() -> SourceMediaDto {
        SourceMediaDto {
            source_path: "/tmp/source.ts".to_owned(),
            playback_reference: "/tmp/source.ts".to_owned(),
            media_kind: MediaKind::Video,
            compatibility_mode: MediaCompatibilityMode::Direct,
            file_name: "source.ts".to_owned(),
            file_size_bytes: 1,
            duration_ms: Some(1_000),
            audio_start_ms: Some(0),
            audio_end_ms: Some(1_000),
            width: Some(1280),
            height: Some(720),
            frame_rate_fps: Some(30.0),
            audio_sample_rate_hz: Some(48_000),
            audio_channel_count: Some(2),
            video_codec_name: Some("h264".to_owned()),
            audio_codec_name: Some("aac".to_owned()),
            mp4_sha256: None,
            mp4_hash_status: "disabled".to_owned(),
        }
    }

    fn mp4_source(codec: &str) -> SourceMediaDto {
        let mut source = ts_source();
        source.source_path = "/tmp/source.mp4".to_owned();
        source.playback_reference = source.source_path.clone();
        source.file_name = "source.mp4".to_owned();
        source.video_codec_name = Some(codec.to_owned());
        source
    }

    #[test]
    fn every_video_requires_period_ready_compatibility() {
        assert!(requires_video_playback_compatibility(&mp4_source("HEVC")));
        assert!(requires_video_playback_compatibility(&mp4_source("H264")));
        assert!(requires_video_playback_compatibility(&ts_source()));
    }

    #[test]
    fn pure_audio_does_not_create_video_compatibility_cache() {
        let mut source = mp4_source("h264");
        source.media_kind = MediaKind::Audio;
        source.video_codec_name = None;
        assert!(!requires_video_playback_compatibility(&source));
    }

    #[test]
    fn transcode_uses_selected_h264_encoder_and_period_ready_contract() {
        let source = ts_source();
        let args = strings(build_transcode_args(
            Path::new("source.ts"),
            Path::new("output.partial.mp4"),
            "libopenh264",
            &source,
        ));
        assert!(args.windows(2).any(|pair| pair == ["-c:v", "libopenh264"]));
        assert!(args.windows(2).any(|pair| pair == ["-c:a", "aac"]));
        assert!(args.windows(2).any(|pair| pair == ["-profile:v", "main"]));
        assert!(args.windows(2).any(|pair| pair == ["-level:v", "4.2"]));
        assert!(args.windows(2).any(|pair| pair == ["-pix_fmt", "yuv420p"]));
        assert!(args.windows(2).any(|pair| pair == ["-fps_mode:v", "cfr"]));
        assert!(args.windows(2).any(|pair| pair == ["-g", "30"]));
        assert!(args.windows(2).any(|pair| pair == ["-keyint_min", "30"]));
        assert!(args.windows(2).any(|pair| pair == ["-bf", "0"]));
        assert!(args.windows(2).any(|pair| pair == ["-flags", "+cgop"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-force_key_frames", "expr:gte(t,n_forced*1)"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-video_track_timescale", "90000"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-avoid_negative_ts", "disabled"]));
        assert!(args.windows(2).any(|pair| pair == ["-use_editlist", "1"]));
        assert!(args.iter().any(|value| {
            value
                == "fps=fps=30.000000:start_time=0:round=near,format=yuv420p,setsar=1,setpts=PTS-STARTPTS"
        }));
    }

    #[test]
    fn transcode_uses_high_level_for_high_spec_video() {
        let mut source = mp4_source("hevc");
        source.width = Some(3840);
        source.height = Some(2160);
        source.frame_rate_fps = Some(60.0);
        let args = strings(build_transcode_args(
            Path::new("source.mp4"),
            Path::new("output.partial.mp4"),
            "h264_amf",
            &source,
        ));

        assert!(args.windows(2).any(|pair| pair == ["-level:v", "5.2"]));
    }

    #[test]
    fn portrait_hd_video_stays_within_the_period_level_42_contract() {
        let mut source = mp4_source("hevc");
        source.width = Some(720);
        source.height = Some(1280);
        source.frame_rate_fps = Some(30.0);
        let args = strings(build_transcode_args(
            Path::new("source.mp4"),
            Path::new("output.partial.mp4"),
            "h264_amf",
            &source,
        ));

        assert!(args.windows(2).any(|pair| pair == ["-level:v", "4.2"]));
    }

    #[test]
    fn cancellation_before_start_does_not_touch_the_cache() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let request = MediaCompatibilityRequest {
            ffmpeg_path: PathBuf::from("missing-ffmpeg"),
            ffprobe_path: PathBuf::from("missing-ffprobe"),
            cache_dir: PathBuf::from("unused-cache"),
            source: ts_source(),
            timeout_seconds: 1,
        };

        assert!(matches!(
            prepare_media_compatibility(&request, &cancellation),
            Err(MediaCompatibilityError::Cancelled)
        ));
        assert!(!request.cache_dir.exists());
    }
}
