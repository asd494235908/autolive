use crate::cancellation::CancellationToken;
use crate::errors::MediaLibraryError;
use mp4::{ChannelConfig, Mp4Reader, TrackType};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

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
            mp4_hash_status: "pending".to_owned(),
        },
    })
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
        probe_user_selected_mp4, MediaProbeRequestDto, MediaProbeResultDto, SourceMediaDto,
    };
    use crate::cancellation::CancellationToken;
    use crate::errors::MediaLibraryError;
    use mp4::{AvcConfig, Bytes, Mp4Config, Mp4Sample, Mp4Writer, TrackConfig};
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;

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
        assert_eq!(result.source.mp4_hash_status, "pending");
    }

    #[test]
    fn probe_leaves_full_file_sha256_for_the_async_hash_command() {
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
        assert_eq!(result.source.mp4_hash_status, "pending");
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
