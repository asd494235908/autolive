use crate::audio_pcm_effects::{AudioPcmEffectConfig, AudioPcmEffectProcessor};
use crate::background_process::background_command;
use crate::bounded_io::read_to_end_bounded;
use autolive_signalsmith_stretch::{QualityPitchConfig, QualityPitchProcessor};
use std::fmt::{Display, Formatter};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CHANNELS: usize = 2;
const PCM_CHUNK_BYTES: usize = 64 * 1024;
const MAX_CACHE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub struct WebViewInterludeCacheRequest {
    pub ffmpeg_path: PathBuf,
    pub source_path: PathBuf,
    pub ambient_source_path: Option<PathBuf>,
    pub filter_graph: String,
    pub quality_pitch: Option<QualityPitchConfig>,
    pub pcm_effects: Option<AudioPcmEffectConfig>,
    pub sample_rate_hz: u32,
    pub output_bitrate_kbps: u16,
    pub cache_dir: PathBuf,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebViewInterludeCacheResult {
    pub output_path: PathBuf,
    pub output_size_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebViewInterludeCacheCleanup {
    pub removed_files: u32,
    pub removed_bytes: u64,
    pub remaining_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebViewInterludeCacheError {
    InvalidInput(&'static str),
    Cache(String),
    Ffmpeg {
        stage: &'static str,
        code: Option<i32>,
    },
    Timeout {
        stage: &'static str,
    },
    OutputTooLarge,
    Transform(String),
}

impl Display for WebViewInterludeCacheError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) => formatter.write_str(message),
            Self::Cache(message) => write!(formatter, "插话缓存文件失败：{message}"),
            Self::Ffmpeg { stage, code } => {
                write!(formatter, "插话{stage}失败：FFmpeg exit_code={code:?}")
            }
            Self::Timeout { stage } => write!(formatter, "插话{stage}超过本地处理时限"),
            Self::OutputTooLarge => formatter.write_str("插话处理缓存超过 256 MiB 上限"),
            Self::Transform(message) => write!(formatter, "插话 PCM 效果处理失败：{message}"),
        }
    }
}

impl std::error::Error for WebViewInterludeCacheError {}

pub fn render_webview_interlude_cache(
    request: WebViewInterludeCacheRequest,
) -> Result<WebViewInterludeCacheResult, WebViewInterludeCacheError> {
    validate_request(&request)?;
    fs::create_dir_all(&request.cache_dir)
        .map_err(|error| WebViewInterludeCacheError::Cache(error.to_string()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| WebViewInterludeCacheError::Cache(error.to_string()))?
        .as_nanos();
    let stem = format!("interlude-{}-{nonce}", std::process::id());
    let decoded_path = request.cache_dir.join(format!("{stem}.decoded.f32le"));
    let processed_path = request.cache_dir.join(format!("{stem}.processed.f32le"));
    let partial_path = request.cache_dir.join(format!("{stem}.partial.m4a"));
    let output_path = request.cache_dir.join(format!("{stem}.m4a"));
    let started_at = Instant::now();

    let result = (|| {
        decode_to_pcm(&request, &decoded_path, started_at)?;
        process_pcm(
            &decoded_path,
            &processed_path,
            request.quality_pitch,
            request.pcm_effects,
            request.sample_rate_hz,
            request.timeout,
            started_at,
        )?;
        encode_pcm_to_m4a(
            &request.ffmpeg_path,
            &processed_path,
            &partial_path,
            request.sample_rate_hz,
            request.output_bitrate_kbps,
            request.timeout,
            started_at,
        )?;
        let size = fs::metadata(&partial_path)
            .map_err(|error| WebViewInterludeCacheError::Cache(error.to_string()))?
            .len();
        if size == 0 || size > MAX_CACHE_BYTES {
            return Err(WebViewInterludeCacheError::OutputTooLarge);
        }
        fs::rename(&partial_path, &output_path)
            .map_err(|error| WebViewInterludeCacheError::Cache(error.to_string()))?;
        Ok(WebViewInterludeCacheResult {
            output_path,
            output_size_bytes: size,
        })
    })();

    for path in [&decoded_path, &processed_path, &partial_path] {
        let _ = fs::remove_file(path);
    }
    result
}

pub fn cleanup_webview_interlude_cache(
    cache_dir: &Path,
    protected_paths: &[PathBuf],
) -> std::io::Result<WebViewInterludeCacheCleanup> {
    if !cache_dir.is_dir() {
        return Ok(WebViewInterludeCacheCleanup {
            removed_files: 0,
            removed_bytes: 0,
            remaining_bytes: 0,
        });
    }
    let stale_before = SystemTime::now()
        .checked_sub(Duration::from_secs(10 * 60))
        .unwrap_or(UNIX_EPOCH);
    let mut result = WebViewInterludeCacheCleanup {
        removed_files: 0,
        removed_bytes: 0,
        remaining_bytes: 0,
    };
    let protected = protected_paths
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    for entry in fs::read_dir(cache_dir)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            continue;
        }
        let path = entry.path();
        if protected.contains(&path) {
            result.remaining_bytes = result.remaining_bytes.saturating_add(metadata.len());
            continue;
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let complete = name.starts_with("interlude-")
            && name.ends_with(".m4a")
            && !name.ends_with(".partial.m4a");
        let stale_temporary = name.starts_with("interlude-")
            && (name.ends_with(".decoded.f32le")
                || name.ends_with(".processed.f32le")
                || name.ends_with(".partial.m4a"))
            && metadata.modified().unwrap_or(UNIX_EPOCH) <= stale_before;
        if !complete && !stale_temporary {
            result.remaining_bytes = result.remaining_bytes.saturating_add(metadata.len());
            continue;
        }
        match fs::remove_file(path) {
            Ok(()) => {
                result.removed_files = result.removed_files.saturating_add(1);
                result.removed_bytes = result.removed_bytes.saturating_add(metadata.len());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(result)
}

fn validate_request(
    request: &WebViewInterludeCacheRequest,
) -> Result<(), WebViewInterludeCacheError> {
    if !request.ffmpeg_path.is_file() {
        return Err(WebViewInterludeCacheError::InvalidInput(
            "本地 FFmpeg 不可用",
        ));
    }
    if !request.source_path.is_file() {
        return Err(WebViewInterludeCacheError::InvalidInput(
            "插话音频不存在或不可读取",
        ));
    }
    if request
        .ambient_source_path
        .as_ref()
        .is_some_and(|path| !path.is_file())
    {
        return Err(WebViewInterludeCacheError::InvalidInput(
            "环境声素材不存在或不可读取",
        ));
    }
    if request.filter_graph.trim().is_empty() {
        return Err(WebViewInterludeCacheError::InvalidInput(
            "插话音频滤镜计划不能为空",
        ));
    }
    if !matches!(request.sample_rate_hz, 44_100 | 48_000) {
        return Err(WebViewInterludeCacheError::InvalidInput(
            "WebView 插话采样率只支持 44100/48000Hz",
        ));
    }
    if !(64..=320).contains(&request.output_bitrate_kbps) {
        return Err(WebViewInterludeCacheError::InvalidInput(
            "WebView 插话输出码率必须在 64–320kbps 之间",
        ));
    }
    if request.timeout.is_zero() {
        return Err(WebViewInterludeCacheError::InvalidInput(
            "插话处理超时必须大于零",
        ));
    }
    Ok(())
}

fn decode_to_pcm(
    request: &WebViewInterludeCacheRequest,
    output_path: &Path,
    started_at: Instant,
) -> Result<(), WebViewInterludeCacheError> {
    let mut args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostdin".into(),
        "-y".into(),
        "-i".into(),
        request.source_path.as_os_str().to_owned(),
    ];
    if let Some(ambient) = request.ambient_source_path.as_ref() {
        args.extend([
            "-stream_loop".into(),
            "-1".into(),
            "-i".into(),
            ambient.as_os_str().to_owned(),
        ]);
    }
    args.extend([
        "-filter_complex".into(),
        request.filter_graph.clone().into(),
        "-map".into(),
        "[aout]".into(),
        "-vn".into(),
        "-sn".into(),
        "-dn".into(),
        "-ac".into(),
        CHANNELS.to_string().into(),
        "-ar".into(),
        request.sample_rate_hz.to_string().into(),
        "-f".into(),
        "f32le".into(),
        "-acodec".into(),
        "pcm_f32le".into(),
        output_path.as_os_str().to_owned(),
    ]);
    run_ffmpeg(
        &request.ffmpeg_path,
        args,
        output_path,
        "解码",
        request.timeout,
        started_at,
    )
}

fn process_pcm(
    input_path: &Path,
    output_path: &Path,
    quality_pitch: Option<QualityPitchConfig>,
    pcm_effects: Option<AudioPcmEffectConfig>,
    sample_rate_hz: u32,
    timeout: Duration,
    started_at: Instant,
) -> Result<(), WebViewInterludeCacheError> {
    let input = File::open(input_path)
        .map(BufReader::new)
        .map_err(|error| WebViewInterludeCacheError::Cache(error.to_string()))?;
    let output = File::create(output_path)
        .map(BufWriter::new)
        .map_err(|error| WebViewInterludeCacheError::Cache(error.to_string()))?;
    process_pcm_stream(
        input,
        output,
        quality_pitch,
        pcm_effects,
        sample_rate_hz,
        Some((timeout, started_at)),
    )
}

fn process_pcm_stream(
    mut input: impl Read,
    mut output: impl Write,
    quality_pitch: Option<QualityPitchConfig>,
    pcm_effects: Option<AudioPcmEffectConfig>,
    sample_rate_hz: u32,
    deadline: Option<(Duration, Instant)>,
) -> Result<(), WebViewInterludeCacheError> {
    let mut quality_pitch = quality_pitch
        .map(|config| QualityPitchProcessor::new(config, sample_rate_hz, CHANNELS))
        .transpose()
        .map_err(|error| WebViewInterludeCacheError::Transform(error.to_string()))?;
    let mut pcm_effects = pcm_effects
        .map(|config| AudioPcmEffectProcessor::new(config, sample_rate_hz, CHANNELS))
        .transpose()
        .map_err(|error| WebViewInterludeCacheError::Transform(error.to_string()))?;
    let mut buffer = [0_u8; PCM_CHUNK_BYTES];
    let mut total_output_bytes = 0_u64;
    loop {
        if deadline.is_some_and(|(timeout, started_at)| started_at.elapsed() >= timeout) {
            return Err(WebViewInterludeCacheError::Timeout {
                stage: "PCM 处理"
            });
        }
        let read = read_aligned_pcm_chunk(&mut input, &mut buffer)?;
        if read == 0 {
            break;
        }
        let samples = buffer[..read]
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            .collect::<Vec<_>>();
        let processed = apply_pcm_processors(&mut quality_pitch, &mut pcm_effects, &samples)?;
        total_output_bytes = write_pcm_samples(&mut output, &processed, total_output_bytes)?;
    }
    if let Some(processor) = quality_pitch.as_mut() {
        let tail = processor
            .flush()
            .map_err(|error| WebViewInterludeCacheError::Transform(error.to_string()))?;
        let tail = apply_pcm_feature_processor(&mut pcm_effects, tail)?;
        total_output_bytes = write_pcm_samples(&mut output, &tail, total_output_bytes)?;
    }
    output
        .flush()
        .map_err(|error| WebViewInterludeCacheError::Cache(error.to_string()))?;
    if total_output_bytes == 0 {
        return Err(WebViewInterludeCacheError::InvalidInput(
            "插话音频没有可处理的 PCM 样本",
        ));
    }
    Ok(())
}

fn read_aligned_pcm_chunk(
    input: &mut impl Read,
    buffer: &mut [u8],
) -> Result<usize, WebViewInterludeCacheError> {
    let mut read = 0;
    while read < buffer.len() {
        match input.read(&mut buffer[read..]) {
            Ok(0) => break,
            Ok(count) => read += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(WebViewInterludeCacheError::Cache(error.to_string())),
        }
    }
    if read % (CHANNELS * std::mem::size_of::<f32>()) != 0 {
        return Err(WebViewInterludeCacheError::InvalidInput(
            "插话 PCM 数据未按双声道帧对齐",
        ));
    }
    Ok(read)
}

fn apply_pcm_processors(
    quality_pitch: &mut Option<QualityPitchProcessor>,
    pcm_effects: &mut Option<AudioPcmEffectProcessor>,
    input: &[f32],
) -> Result<Vec<f32>, WebViewInterludeCacheError> {
    let pitched = match quality_pitch.as_mut() {
        Some(processor) => processor
            .process_interleaved(input)
            .map_err(|error| WebViewInterludeCacheError::Transform(error.to_string()))?,
        None => input.to_vec(),
    };
    apply_pcm_feature_processor(pcm_effects, pitched)
}

fn apply_pcm_feature_processor(
    pcm_effects: &mut Option<AudioPcmEffectProcessor>,
    input: Vec<f32>,
) -> Result<Vec<f32>, WebViewInterludeCacheError> {
    if input.is_empty() {
        return Ok(input);
    }
    match pcm_effects.as_mut() {
        Some(processor) => processor
            .process_interleaved(&input)
            .map_err(|error| WebViewInterludeCacheError::Transform(error.to_string())),
        None => Ok(input),
    }
}

fn write_pcm_samples(
    output: &mut impl Write,
    samples: &[f32],
    previous_bytes: u64,
) -> Result<u64, WebViewInterludeCacheError> {
    let added_bytes = u64::try_from(samples.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(std::mem::size_of::<f32>() as u64);
    let total = previous_bytes.saturating_add(added_bytes);
    if total > MAX_CACHE_BYTES {
        return Err(WebViewInterludeCacheError::OutputTooLarge);
    }
    for sample in samples {
        output
            .write_all(&sample.to_le_bytes())
            .map_err(|error| WebViewInterludeCacheError::Cache(error.to_string()))?;
    }
    Ok(total)
}

fn encode_pcm_to_m4a(
    ffmpeg_path: &Path,
    input_path: &Path,
    output_path: &Path,
    sample_rate_hz: u32,
    output_bitrate_kbps: u16,
    timeout: Duration,
    started_at: Instant,
) -> Result<(), WebViewInterludeCacheError> {
    let args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostdin".into(),
        "-y".into(),
        "-f".into(),
        "f32le".into(),
        "-ar".into(),
        sample_rate_hz.to_string().into(),
        "-ac".into(),
        CHANNELS.to_string().into(),
        "-i".into(),
        input_path.as_os_str().to_owned(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        format!("{output_bitrate_kbps}k").into(),
        output_path.as_os_str().to_owned(),
    ];
    run_ffmpeg(ffmpeg_path, args, output_path, "编码", timeout, started_at)
}

fn run_ffmpeg(
    ffmpeg_path: &Path,
    args: Vec<std::ffi::OsString>,
    monitored_output: &Path,
    stage: &'static str,
    timeout: Duration,
    started_at: Instant,
) -> Result<(), WebViewInterludeCacheError> {
    let mut child = background_command(ffmpeg_path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| WebViewInterludeCacheError::Cache(error.to_string()))?;
    let stderr = child.stderr.take().ok_or_else(|| {
        terminate_child(&mut child);
        WebViewInterludeCacheError::Cache("FFmpeg 未提供 stderr 管道".to_owned())
    })?;
    let stderr_reader = thread::Builder::new()
        .name("webview-interlude-stderr".to_owned())
        .spawn(move || read_to_end_bounded(stderr, MAX_STDERR_BYTES))
        .map_err(|error| {
            terminate_child(&mut child);
            WebViewInterludeCacheError::Cache(error.to_string())
        })?;
    let status = loop {
        if started_at.elapsed() >= timeout {
            terminate_child(&mut child);
            let _ = stderr_reader.join();
            return Err(WebViewInterludeCacheError::Timeout { stage });
        }
        if fs::metadata(monitored_output)
            .map(|metadata| metadata.len() > MAX_CACHE_BYTES)
            .unwrap_or(false)
        {
            terminate_child(&mut child);
            let _ = stderr_reader.join();
            return Err(WebViewInterludeCacheError::OutputTooLarge);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                terminate_child(&mut child);
                let _ = stderr_reader.join();
                return Err(WebViewInterludeCacheError::Cache(error.to_string()));
            }
        }
    };
    let _ = stderr_reader.join();
    if status.success() {
        Ok(())
    } else {
        Err(WebViewInterludeCacheError::Ffmpeg {
            stage,
            code: status.code(),
        })
    }
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use autolive_signalsmith_stretch::QualityPitchConfig;

    use super::{
        cleanup_webview_interlude_cache, process_pcm_stream, render_webview_interlude_cache,
        WebViewInterludeCacheError, WebViewInterludeCacheRequest,
    };
    use crate::audio_pcm_effects::AudioPcmEffectConfig;
    use crate::media_audio_effects::{MfccOperation, MfccRuntimePlan};
    use crate::media_effect_params::AudioEffectParams;
    use crate::media_engine::build_audio_stream_filter_graph_with_ambient;
    use std::io::Cursor;

    fn quality_pitch_and_mfcc() -> (QualityPitchConfig, AudioPcmEffectConfig) {
        (
            QualityPitchConfig {
                pitch_shift_semitones: -0.018,
                formant_shift_percent: 0.105,
            },
            AudioPcmEffectConfig {
                mfcc: Some(MfccRuntimePlan {
                    dimensions: 12,
                    shift_percent: -0.16,
                    operation: MfccOperation::ShiftAndReconstruct,
                }),
                snr: None,
            },
        )
    }

    #[test]
    fn pcm_cache_transform_rejects_an_unaligned_stereo_frame() {
        let result = process_pcm_stream(
            Cursor::new(vec![0_u8; 4]),
            Vec::new(),
            None,
            None,
            48_000,
            None,
        );
        assert_eq!(
            result,
            Err(WebViewInterludeCacheError::InvalidInput(
                "插话 PCM 数据未按双声道帧对齐"
            ))
        );
    }

    #[test]
    fn pcm_cache_transform_keeps_aligned_finite_samples() {
        let input = [0.25_f32, -0.25_f32, 0.5_f32, -0.5_f32]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
        let mut output = Vec::new();
        process_pcm_stream(
            Cursor::new(input.clone()),
            &mut output,
            None,
            None,
            48_000,
            None,
        )
        .expect("aligned PCM should pass through");
        assert_eq!(output, input);
    }

    #[test]
    fn pcm_cache_warmup_skips_empty_quality_pitch_output_before_features() {
        let input = [0.25_f32, -0.25_f32]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
        let (quality_pitch, pcm_effects) = quality_pitch_and_mfcc();

        let result = process_pcm_stream(
            Cursor::new(input),
            Vec::new(),
            Some(quality_pitch),
            Some(pcm_effects),
            48_000,
            None,
        );

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn pcm_cache_empty_input_skips_empty_quality_pitch_flush_tail() {
        let (quality_pitch, pcm_effects) = quality_pitch_and_mfcc();

        let result = process_pcm_stream(
            Cursor::new(Vec::<u8>::new()),
            Vec::new(),
            Some(quality_pitch),
            Some(pcm_effects),
            48_000,
            None,
        );

        assert_eq!(
            result,
            Err(WebViewInterludeCacheError::InvalidInput(
                "插话音频没有可处理的 PCM 样本"
            ))
        );
    }

    #[test]
    fn pcm_cache_transform_observes_the_shared_render_deadline() {
        let result = process_pcm_stream(
            Cursor::new(vec![0_u8; 8]),
            Vec::new(),
            None,
            None,
            48_000,
            Some((std::time::Duration::ZERO, std::time::Instant::now())),
        );

        assert_eq!(
            result,
            Err(WebViewInterludeCacheError::Timeout {
                stage: "PCM 处理"
            })
        );
    }

    #[test]
    fn cache_cleanup_removes_completed_outputs_but_keeps_unrelated_files() {
        let cache_dir = std::env::temp_dir().join(format!(
            "autolive-webview-interlude-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("test clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&cache_dir).expect("create cache fixture");
        std::fs::write(cache_dir.join("interlude-1.m4a"), b"done").expect("write cache");
        std::fs::write(cache_dir.join("other.wav"), b"keep").expect("write unrelated");

        let protected = cache_dir.join("interlude-protected.m4a");
        std::fs::write(&protected, b"busy").expect("write protected cache");
        let result = cleanup_webview_interlude_cache(&cache_dir, std::slice::from_ref(&protected))
            .expect("cleanup cache");

        assert_eq!(result.removed_files, 1);
        assert_eq!(result.removed_bytes, 4);
        assert_eq!(result.remaining_bytes, 8);
        assert!(!cache_dir.join("interlude-1.m4a").exists());
        assert!(cache_dir.join("other.wav").exists());
        assert!(protected.exists());

        let released = cleanup_webview_interlude_cache(&cache_dir, &[])
            .expect("released cache should be removable");
        assert_eq!(released.removed_files, 1);
        assert!(!protected.exists());
        let _ = std::fs::remove_dir_all(cache_dir);
    }

    #[test]
    fn packaged_ffmpeg_renders_a_webview_cache_with_user_ambient() {
        let Some(ffmpeg) = std::env::var_os("AUTOLIVE_TEST_FFMPEG") else {
            return;
        };
        let cache_dir = std::env::temp_dir().join(format!(
            "autolive-webview-interlude-render-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("test clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&cache_dir).expect("create render fixture");
        let source = cache_dir.join("source.wav");
        let status = crate::background_process::background_command(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=0.2",
                "-ac",
                "2",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&source)
            .status()
            .expect("create source fixture");
        assert!(status.success());
        let audio = AudioEffectParams {
            ambient_sound_mix_percent: 1.0,
            ..Default::default()
        };
        let plan =
            build_audio_stream_filter_graph_with_ambient(&audio, &[], Some(48_000), 48_000, true)
                .expect("build cache filter plan");
        let result = render_webview_interlude_cache(WebViewInterludeCacheRequest {
            ffmpeg_path: ffmpeg.into(),
            source_path: source.clone(),
            ambient_source_path: Some(source),
            filter_graph: plan.filter_graph,
            quality_pitch: plan.quality_pitch,
            pcm_effects: plan.pcm_effects,
            sample_rate_hz: 48_000,
            output_bitrate_kbps: 192,
            cache_dir: cache_dir.clone(),
            timeout: std::time::Duration::from_secs(20),
        })
        .expect("render WebView interlude cache");

        assert!(result.output_path.is_file());
        assert!(result.output_size_bytes > 44);
        let _ = std::fs::remove_dir_all(cache_dir);
    }
}
