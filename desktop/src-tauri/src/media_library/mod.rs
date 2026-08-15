use crate::cancellation::CancellationToken;
use crate::errors::MediaLibraryError;
use mp4::{ChannelConfig, Mp4Reader, TrackType};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const MAX_FFPROBE_OUTPUT_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaProbeRequestDto {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceMediaProbeDto {
    pub source_path: String,
    pub file_name: String,
    pub file_size_bytes: u64,
    pub duration_ms: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate_fps: Option<f64>,
    pub audio_sample_rate_hz: Option<u32>,
    pub audio_channel_count: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceMediaDto {
    pub source_path: String,
    pub file_name: String,
    pub file_size_bytes: u64,
    pub duration_ms: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate_fps: Option<f64>,
    pub audio_sample_rate_hz: Option<u32>,
    pub audio_channel_count: Option<u16>,
    pub mp4_sha256: Option<String>,
    pub mp4_hash_status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaProbeResultDto {
    pub canonical_path: String,
    pub source: SourceMediaDto,
}

pub fn probe_user_selected_mp4(
    request: &MediaProbeRequestDto,
    cancellation: &CancellationToken,
) -> Result<MediaProbeResultDto, MediaLibraryError> {
    let resolved = resolve_user_selected_file(&request.path, cancellation)?;
    let metadata = probe_mp4_metadata(&resolved, cancellation)?;

    Ok(MediaProbeResultDto {
        canonical_path: resolved.display().to_string(),
        source: SourceMediaDto {
            source_path: metadata.source_path,
            file_name: metadata.file_name,
            file_size_bytes: metadata.file_size_bytes,
            duration_ms: metadata.duration_ms,
            width: metadata.width,
            height: metadata.height,
            frame_rate_fps: metadata.frame_rate_fps,
            audio_sample_rate_hz: metadata.audio_sample_rate_hz,
            audio_channel_count: metadata.audio_channel_count,
            mp4_sha256: None,
            mp4_hash_status: "disabled".to_owned(),
        },
    })
}

#[derive(Debug, Deserialize)]
struct FfprobeMediaOutput {
    #[serde(default)]
    streams: Vec<FfprobeMediaStream>,
    format: Option<FfprobeMediaFormat>,
}

#[derive(Debug, Deserialize)]
struct FfprobeMediaStream {
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct FfprobeMediaFormat {
    duration: Option<String>,
}

pub fn probe_user_selected_mp4_with_ffprobe(
    request: &MediaProbeRequestDto,
    ffprobe_path: &Path,
    timeout_ms: u64,
    cancellation: &CancellationToken,
) -> Result<MediaProbeResultDto, MediaLibraryError> {
    let resolved = resolve_user_selected_file(&request.path, cancellation)?;
    let metadata =
        std::fs::metadata(&resolved).map_err(|_| MediaLibraryError::FileMetadataReadFailed {
            path: resolved.display().to_string(),
        })?;
    let file_name = resolved
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_owned)
        .ok_or_else(|| MediaLibraryError::FileNameUnavailable {
            path: resolved.display().to_string(),
        })?;
    if !ffprobe_path.is_file() {
        return Err(unreadable_ffprobe_error(
            &resolved,
            format!("本地 FFprobe 不可用：{}", ffprobe_path.display()),
        ));
    }
    if timeout_ms == 0 {
        return Err(unreadable_ffprobe_error(
            &resolved,
            "FFprobe 超时预算必须大于 0ms".to_owned(),
        ));
    }

    let mut command = Command::new(ffprobe_path);
    command
        .args([
            "-hide_banner",
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_type,width,height,avg_frame_rate,r_frame_rate,sample_rate,channels",
            "-of",
            "json",
        ])
        .arg(&resolved)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(|error| {
        unreadable_ffprobe_error(&resolved, format!("启动本地 FFprobe 失败：{error}"))
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        terminate_probe_process(&mut child);
        unreadable_ffprobe_error(&resolved, "无法读取 FFprobe 输出".to_owned())
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        terminate_probe_process(&mut child);
        unreadable_ffprobe_error(&resolved, "无法读取 FFprobe 错误输出".to_owned())
    })?;
    let stdout_reader = thread::spawn(move || read_probe_output(stdout));
    let stderr_reader = thread::spawn(move || read_probe_output(stderr));
    let started_at = Instant::now();
    let status = loop {
        if cancellation.is_cancelled() {
            terminate_probe_process(&mut child);
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(MediaLibraryError::Cancelled);
        }
        if started_at.elapsed() >= Duration::from_millis(timeout_ms) {
            terminate_probe_process(&mut child);
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(unreadable_ffprobe_error(
                &resolved,
                format!("读取视频信息超时（{timeout_ms}ms），请检查 MP4 文件是否完整"),
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                terminate_probe_process(&mut child);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(unreadable_ffprobe_error(
                    &resolved,
                    format!("等待 FFprobe 失败：{error}"),
                ));
            }
        }
    };
    let stdout = join_probe_output(stdout_reader, &resolved, "FFprobe 输出")?;
    let stderr = join_probe_output(stderr_reader, &resolved, "FFprobe 错误输出")?;
    validate_ffprobe_status(status, &stderr, &resolved)?;
    let output = serde_json::from_slice::<FfprobeMediaOutput>(&stdout).map_err(|error| {
        unreadable_ffprobe_error(&resolved, format!("FFprobe 返回的 JSON 无法解析：{error}"))
    })?;
    let video = output
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"))
        .ok_or_else(|| unreadable_ffprobe_error(&resolved, "MP4 中没有视频轨道".to_owned()))?;
    let audio = output
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"));
    let duration_ms = output
        .format
        .and_then(|format| format.duration)
        .and_then(|duration| duration.parse::<f64>().ok())
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .map(|duration| (duration * 1_000.0).round() as u64);
    let frame_rate_fps = video
        .avg_frame_rate
        .as_deref()
        .and_then(parse_ffprobe_ratio)
        .or_else(|| video.r_frame_rate.as_deref().and_then(parse_ffprobe_ratio));
    let audio_sample_rate_hz = audio
        .and_then(|stream| stream.sample_rate.as_deref())
        .and_then(|sample_rate| sample_rate.parse::<u32>().ok())
        .filter(|sample_rate| *sample_rate > 0);

    Ok(MediaProbeResultDto {
        canonical_path: resolved.display().to_string(),
        source: SourceMediaDto {
            source_path: resolved.display().to_string(),
            file_name,
            file_size_bytes: metadata.len(),
            duration_ms,
            width: video.width.filter(|width| *width > 0),
            height: video.height.filter(|height| *height > 0),
            frame_rate_fps,
            audio_sample_rate_hz,
            audio_channel_count: audio
                .and_then(|stream| stream.channels)
                .filter(|count| *count > 0),
            mp4_sha256: None,
            mp4_hash_status: "disabled".to_owned(),
        },
    })
}

fn read_probe_output(mut stream: impl Read) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    stream
        .by_ref()
        .take(MAX_FFPROBE_OUTPUT_BYTES)
        .read_to_end(&mut output)?;
    Ok(output)
}

fn join_probe_output(
    handle: thread::JoinHandle<std::io::Result<Vec<u8>>>,
    path: &Path,
    label: &str,
) -> Result<Vec<u8>, MediaLibraryError> {
    handle
        .join()
        .map_err(|_| unreadable_ffprobe_error(path, format!("{label}读取线程异常退出")))?
        .map_err(|error| unreadable_ffprobe_error(path, format!("{label}读取失败：{error}")))
}

fn validate_ffprobe_status(
    status: ExitStatus,
    stderr: &[u8],
    path: &Path,
) -> Result<(), MediaLibraryError> {
    if status.success() {
        return Ok(());
    }
    let reason = String::from_utf8_lossy(stderr).trim().to_owned();
    Err(unreadable_ffprobe_error(
        path,
        if reason.is_empty() {
            format!("FFprobe 退出码 {:?}", status.code())
        } else {
            reason
        },
    ))
}

fn parse_ffprobe_ratio(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once('/')?;
    let numerator = numerator.parse::<f64>().ok()?;
    let denominator = denominator.parse::<f64>().ok()?;
    let ratio = numerator / denominator;
    (denominator != 0.0 && ratio.is_finite() && ratio > 0.0).then_some(ratio)
}

fn unreadable_ffprobe_error(path: &Path, message: String) -> MediaLibraryError {
    MediaLibraryError::UnreadableMp4Container {
        path: path.display().to_string(),
        message,
    }
}

fn terminate_probe_process(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let process_group = format!("-{}", child.id());
        let _ = Command::new("/bin/kill")
            .args(["-KILL", process_group.as_str()])
            .status();
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn probe_mp4_metadata(
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<SourceMediaProbeDto, MediaLibraryError> {
    if cancellation.is_cancelled() {
        return Err(MediaLibraryError::Cancelled);
    }

    let file = File::open(path).map_err(|_| MediaLibraryError::FileOpenFailed {
        path: path.display().to_string(),
    })?;
    let file_size_bytes = file
        .metadata()
        .map_err(|_| MediaLibraryError::FileMetadataReadFailed {
            path: path.display().to_string(),
        })?
        .len();
    let file_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_owned)
        .ok_or_else(|| MediaLibraryError::FileNameUnavailable {
            path: path.display().to_string(),
        })?;
    let reader = BufReader::new(file);
    let mp4 = Mp4Reader::read_header(reader, file_size_bytes).map_err(|error| {
        MediaLibraryError::UnreadableMp4Container {
            path: path.display().to_string(),
            message: error.to_string(),
        }
    })?;

    let duration_ms = u64::try_from(mp4.duration().as_millis())
        .map(Some)
        .map_err(|_| MediaLibraryError::NumericOverflow {
            field: "duration_ms",
        })?;

    let mut width = None;
    let mut height = None;
    let mut frame_rate_fps = None;
    let mut audio_sample_rate_hz = None;
    let mut audio_channel_count = None;

    for track in mp4.tracks().values() {
        match track.track_type() {
            Ok(TrackType::Video) if width.is_none() => {
                width = Some(u32::from(track.width()));
                height = Some(u32::from(track.height()));
                let fps = track.frame_rate();
                if fps.is_finite() && fps > 0.0 {
                    frame_rate_fps = Some(fps);
                }
            }
            Ok(TrackType::Audio) if audio_sample_rate_hz.is_none() => {
                audio_sample_rate_hz = track.sample_freq_index().ok().map(|index| index.freq());
                audio_channel_count = track.channel_config().ok().map(channel_count);
            }
            _ => {}
        }
    }

    Ok(SourceMediaProbeDto {
        source_path: path.display().to_string(),
        file_name,
        file_size_bytes,
        duration_ms,
        width,
        height,
        frame_rate_fps,
        audio_sample_rate_hz,
        audio_channel_count,
    })
}

fn channel_count(config: ChannelConfig) -> u16 {
    match config {
        ChannelConfig::Mono => 1,
        ChannelConfig::Stereo => 2,
        ChannelConfig::Three => 3,
        ChannelConfig::Four => 4,
        ChannelConfig::Five => 5,
        ChannelConfig::FiveOne => 6,
        ChannelConfig::SevenOne => 8,
    }
}

fn resolve_user_selected_file(
    raw_path: &str,
    cancellation: &CancellationToken,
) -> Result<PathBuf, MediaLibraryError> {
    if cancellation.is_cancelled() {
        return Err(MediaLibraryError::Cancelled);
    }

    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(MediaLibraryError::EmptyPath);
    }

    let canonical_path =
        std::fs::canonicalize(trimmed).map_err(|_| MediaLibraryError::CanonicalizeFailed {
            path: trimmed.to_owned(),
        })?;
    let metadata = std::fs::metadata(&canonical_path).map_err(|_| {
        MediaLibraryError::FileMetadataReadFailed {
            path: canonical_path.display().to_string(),
        }
    })?;

    if !metadata.is_file() {
        return Err(MediaLibraryError::NotAFile {
            path: canonical_path.display().to_string(),
        });
    }

    let extension = canonical_path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_ascii_lowercase);
    if extension.as_deref() != Some("mp4") {
        return Err(MediaLibraryError::UnsupportedExtension {
            path: canonical_path.display().to_string(),
            extension,
        });
    }

    Ok(canonical_path)
}

#[cfg(test)]
mod tests {
    use super::{
        probe_user_selected_mp4, probe_user_selected_mp4_with_ffprobe, MediaProbeRequestDto,
        MediaProbeResultDto, SourceMediaDto,
    };
    use crate::cancellation::CancellationToken;
    use crate::errors::MediaLibraryError;
    use mp4::{AvcConfig, Bytes, Mp4Config, Mp4Sample, Mp4Writer, TrackConfig};
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(suffix: &str) -> Self {
            let unique = format!(
                "autolive-media-library-{}-{}-{}",
                suffix,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time should be after unix epoch")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("test directory should be created");
            Self { path }
        }

        fn path(&self) -> &PathBuf {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ignored = fs::remove_dir_all(&self.path);
        }
    }

    fn create_minimal_mp4(path: &std::path::Path) {
        let config = Mp4Config {
            major_brand: "isom".parse().expect("brand should parse"),
            minor_version: 512,
            compatible_brands: vec![
                "isom".parse().expect("brand should parse"),
                "iso2".parse().expect("brand should parse"),
                "avc1".parse().expect("brand should parse"),
                "mp41".parse().expect("brand should parse"),
            ],
            timescale: 1000,
        };
        let avc = AvcConfig {
            width: 1280,
            height: 720,
            seq_param_set: vec![
                0x67, 0x64, 0x00, 0x1f, 0xac, 0xd9, 0x40, 0x50, 0x1e, 0xd0, 0x08, 0x9f, 0x97, 0x01,
                0x01, 0x01, 0x02,
            ],
            pic_param_set: vec![0x68, 0xeb, 0xe3, 0xcb, 0x22, 0xc0],
        };
        let track = TrackConfig::from(avc);

        let cursor = Cursor::new(Vec::<u8>::new());
        let mut writer =
            Mp4Writer::write_start(cursor, &config).expect("mp4 writer should start successfully");
        writer
            .add_track(&track)
            .expect("video track should be added successfully");
        writer
            .write_sample(
                1,
                &Mp4Sample {
                    start_time: 0,
                    duration: 1000,
                    rendering_offset: 0,
                    is_sync: true,
                    bytes: Bytes::from(vec![
                        0x00, 0x00, 0x00, 0x01, 0x09, 0x10, 0x00, 0x00, 0x00, 0x01, 0x65, 0x88,
                        0x84,
                    ]),
                },
            )
            .expect("sample should be written successfully");
        writer
            .write_end()
            .expect("mp4 file should finish successfully");

        fs::write(path, writer.into_writer().into_inner()).expect("mp4 bytes should be written");
    }

    #[cfg(unix)]
    fn create_executable_script(path: &std::path::Path, source: &str) {
        fs::write(path, source).expect("script should be written");
        let mut permissions = fs::metadata(path)
            .expect("script metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("script should be executable");
    }

    #[cfg(unix)]
    #[test]
    fn ffprobe_import_reads_metadata_without_hashing_the_mp4() {
        let directory = TestDir::new("ffprobe-import");
        let input = directory.path().join("source.mp4");
        let ffprobe = directory.path().join("ffprobe");
        fs::write(&input, b"not-read-by-the-fake-probe").expect("input should be written");
        create_executable_script(
            &ffprobe,
            r#"#!/bin/sh
printf '%s' '{"streams":[{"codec_type":"video","width":720,"height":1280,"avg_frame_rate":"30/1"},{"codec_type":"audio","sample_rate":"44100","channels":2}],"format":{"duration":"72.3"}}'
"#,
        );

        let result = probe_user_selected_mp4_with_ffprobe(
            &MediaProbeRequestDto {
                path: input.display().to_string(),
            },
            &ffprobe,
            1_000,
            &CancellationToken::new(),
        )
        .expect("ffprobe import should succeed");

        assert_eq!(result.source.duration_ms, Some(72_300));
        assert_eq!(result.source.width, Some(720));
        assert_eq!(result.source.height, Some(1280));
        assert_eq!(result.source.frame_rate_fps, Some(30.0));
        assert_eq!(result.source.audio_sample_rate_hz, Some(44_100));
        assert_eq!(result.source.audio_channel_count, Some(2));
        assert_eq!(result.source.mp4_sha256, None);
        assert_eq!(result.source.mp4_hash_status, "disabled");
    }

    #[cfg(unix)]
    #[test]
    fn ffprobe_import_times_out_instead_of_leaving_the_ui_waiting_forever() {
        let directory = TestDir::new("ffprobe-timeout");
        let input = directory.path().join("source.mp4");
        let ffprobe = directory.path().join("ffprobe");
        fs::write(&input, b"input").expect("input should be written");
        create_executable_script(&ffprobe, "#!/bin/sh\nwhile :; do :; done\n");

        let result = probe_user_selected_mp4_with_ffprobe(
            &MediaProbeRequestDto {
                path: input.display().to_string(),
            },
            &ffprobe,
            50,
            &CancellationToken::new(),
        );

        assert!(matches!(
            result,
            Err(MediaLibraryError::UnreadableMp4Container { message, .. })
                if message.contains("超时")
        ));
    }

    #[test]
    fn probe_rejects_empty_path() {
        let request = MediaProbeRequestDto {
            path: String::new(),
        };
        let cancellation = CancellationToken::new();
        let result = probe_user_selected_mp4(&request, &cancellation);

        assert!(result.is_err());
    }

    #[test]
    fn probe_result_is_serializable_shape() {
        let _result_shape = MediaProbeResultDto {
            canonical_path: String::from("/tmp/example.mp4"),
            source: SourceMediaDto {
                source_path: String::from("/tmp/example.mp4"),
                file_name: String::from("example.mp4"),
                file_size_bytes: 1024,
                duration_ms: Some(1000),
                width: Some(1280),
                height: Some(720),
                frame_rate_fps: Some(30.0),
                audio_sample_rate_hz: Some(48_000),
                audio_channel_count: Some(2),
                mp4_sha256: None,
                mp4_hash_status: "pending".to_owned(),
            },
        };
    }

    #[test]
    fn probe_rejects_directory_path() {
        let directory = TestDir::new("probe-dir");
        let request = MediaProbeRequestDto {
            path: directory.path().display().to_string(),
        };
        let cancellation = CancellationToken::new();
        let result = probe_user_selected_mp4(&request, &cancellation);

        assert!(matches!(result, Err(MediaLibraryError::NotAFile { .. })));
    }

    #[test]
    fn probe_rejects_non_mp4_extension() {
        let directory = TestDir::new("probe-text");
        let file_path = directory.path().join("notes.txt");
        fs::write(&file_path, b"plain text").expect("text file should be written");

        let request = MediaProbeRequestDto {
            path: file_path.display().to_string(),
        };
        let cancellation = CancellationToken::new();
        let result = probe_user_selected_mp4(&request, &cancellation);

        assert!(matches!(
            result,
            Err(MediaLibraryError::UnsupportedExtension { .. })
        ));
    }

    #[test]
    fn probe_reads_mp4_metadata_and_file_size() {
        let directory = TestDir::new("probe-mp4");
        let file_path = directory.path().join("sample.mp4");
        create_minimal_mp4(&file_path);
        let expected_size = fs::metadata(&file_path)
            .expect("metadata should be readable")
            .len();

        let request = MediaProbeRequestDto {
            path: file_path.display().to_string(),
        };
        let cancellation = CancellationToken::new();
        let result =
            probe_user_selected_mp4(&request, &cancellation).expect("probe should succeed");

        assert_eq!(result.source.file_name, "sample.mp4");
        assert_eq!(result.source.file_size_bytes, expected_size);
        assert_eq!(result.source.duration_ms, Some(1000));
        assert_eq!(result.source.width, Some(1280));
        assert_eq!(result.source.height, Some(720));
        assert_eq!(result.source.frame_rate_fps, Some(1.0));
        assert_eq!(result.source.mp4_sha256, None);
        assert_eq!(result.source.mp4_hash_status, "disabled");
    }

    #[test]
    fn probe_does_not_start_full_file_sha256() {
        let directory = TestDir::new("build-source");
        let file_path = directory.path().join("sample.mp4");
        create_minimal_mp4(&file_path);

        let cancellation = CancellationToken::new();
        let request = MediaProbeRequestDto {
            path: file_path.display().to_string(),
        };
        let result = probe_user_selected_mp4(&request, &cancellation)
            .expect("source media probe should succeed");

        assert_eq!(result.source.file_name, "sample.mp4");
        assert_eq!(result.source.mp4_sha256, None);
        assert_eq!(result.source.mp4_hash_status, "disabled");
    }

    #[test]
    fn probe_stops_when_cancelled() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let request = MediaProbeRequestDto {
            path: String::from("/tmp/example.mp4"),
        };

        let result = probe_user_selected_mp4(&request, &cancellation);

        assert_eq!(result, Err(MediaLibraryError::Cancelled));
    }
}
