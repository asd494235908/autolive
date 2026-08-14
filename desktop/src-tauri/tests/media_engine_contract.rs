use autolive_desktop_core::cancellation::CancellationToken;
use autolive_desktop_core::media_engine::{
    build_media_render_args, probe_media_engine_with_paths, render_media, MediaEngineError,
    MediaRenderRequest,
};
use autolive_desktop_core::research_params::{AudioResearchParams, VideoResearchParams};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "autolive-media-engine-contract-{}-{}-{}",
            std::process::id(),
            TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("test directory should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.0);
    }
}

fn request(directory: &TestDir) -> MediaRenderRequest {
    let ffmpeg_path = fixture_file(directory, "ffmpeg");
    let ffprobe_path = fixture_file(directory, "ffprobe");
    MediaRenderRequest {
        ffmpeg_path,
        ffprobe_path,
        input_mp4_path: directory.path().join("source.mp4"),
        staging_output_path: directory.path().join("staging.partial.mp4"),
        output_mp4_path: directory.path().join("processed.mp4"),
        video_processing_enabled: true,
        audio_processing_enabled: true,
        source_audio_sample_rate_hz: Some(48_000),
        video: VideoResearchParams::default(),
        audio: AudioResearchParams::default(),
        research: Default::default(),
        timeout_seconds: 2,
    }
}

fn fixture_file(directory: &TestDir, name: &str) -> PathBuf {
    let path = directory.path().join(name);
    fs::write(&path, b"engine fixture").expect("fixture file should be written");
    path
}

#[cfg(unix)]
fn executable_script(directory: &TestDir, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = directory.path().join(name);
    fs::write(&path, body).expect("script should be written");
    let mut permissions = fs::metadata(&path)
        .expect("script metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("script should be executable");
    path
}

#[test]
fn render_plan_reencodes_video_when_video_processing_is_enabled() {
    let directory = TestDir::new();
    let input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    let args = build_media_render_args(&input).expect("render args should be valid");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();

    assert!(values.windows(2).any(|pair| pair == ["-c:v", "libx264"]));
    assert!(!values.windows(2).any(|pair| pair == ["-c:v", "copy"]));
    assert!(values.iter().any(|value| value == "-vf"));
    assert!(values.iter().any(|value| value == "-af"));
    assert!(!values.iter().any(|value| value.contains(";")));
}

#[test]
fn render_rejects_pitch_shift_without_source_sample_rate() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.audio.pitch_shift_semitones = 0.5;
    input.source_audio_sample_rate_hz = None;

    let result = build_media_render_args(&input);

    assert!(matches!(
        result,
        Err(MediaEngineError::InvalidParameters { .. })
    ));
}

#[test]
fn render_maps_native_video_noise_and_detail_filters() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.video.noise_percent = 2.0;
    input.video.detail_enhancement_percent = 5.0;

    let args = build_media_render_args(&input).expect("native video filters should be mapped");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let filter = values
        .windows(2)
        .find(|pair| pair[0] == "-vf")
        .map(|pair| pair[1].clone())
        .expect("video filter should be present");

    assert!(filter.contains("unsharp=5:5:0.050000"));
    assert!(filter.contains("noise=alls=2.000000:allf=t+u"));
}

#[test]
fn render_maps_explicit_audio_sample_rate() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.audio.sample_rate_hz = Some(44_100);

    let args = build_media_render_args(&input).expect("native audio sample rate should be mapped");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let filter = values
        .windows(2)
        .find(|pair| pair[0] == "-af")
        .map(|pair| pair[1].clone())
        .expect("audio filter should be present");

    assert!(filter.contains("aresample=44100"));
}

#[test]
fn render_maps_static_and_dynamic_video_motion_filters() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.video.space_x_offset_px = 2.0;
    input.video.space_y_offset_px = -1.0;
    input.video.dynamic_crop_percent = 1.0;
    input.video.pixel_jitter_px = 1.0;

    let args = build_media_render_args(&input).expect("motion filters should be mapped");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let filter = values
        .windows(2)
        .find(|pair| pair[0] == "-vf")
        .map(|pair| pair[1].clone())
        .expect("video filter should be present");

    assert!(filter.contains("crop=iw*"));
    assert!(filter.contains("sin(n*0.07)"));
    assert!(filter.contains("pad=iw+4:ih+4"));
    assert!(filter.contains("crop=iw-4:ih-4"));
    assert!(filter.contains("round(cos(n*0.91)"));
}

#[test]
fn render_maps_pitch_shift_when_source_sample_rate_is_known() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.audio.pitch_shift_semitones = 1.0;

    let args = build_media_render_args(&input).expect("pitch shift should be mapped");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let filter = values
        .windows(2)
        .find(|pair| pair[0] == "-af")
        .map(|pair| pair[1].clone())
        .expect("audio filter should be present");

    assert!(filter.contains("asetrate="));
    assert!(filter.contains("aresample=48000"));
    assert!(filter.contains("atempo="));
}

#[cfg(unix)]
#[test]
fn render_commits_verified_output_after_probe_and_hash() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.ffmpeg_path = executable_script(
        &directory,
        "ffmpeg-copy",
        "#!/bin/sh\nif [ \"$2\" = \"-version\" ]; then printf '%s\\n' 'ffmpeg version test'; exit 0; fi\nlast=\"\"\nfor arg in \"$@\"; do last=\"$arg\"; done\n/bin/cp \"$5\" \"$last\"\n",
    );
    input.ffprobe_path = executable_script(
        &directory,
        "ffprobe-ok",
        "#!/bin/sh\nif [ \"$2\" = \"-version\" ]; then printf '%s\\n' 'ffprobe version test'; fi\nexit 0\n",
    );

    let result = render_media(&input, &CancellationToken::new()).expect("output should commit");

    assert_eq!(result.output_mp4_path, input.output_mp4_path);
    assert_eq!(result.output_size_bytes, 6);
    assert_eq!(result.output_mp4_sha256.len(), 64);
    assert_eq!(
        fs::read(&input.output_mp4_path).expect("output should exist"),
        b"source"
    );
    assert!(!input.staging_output_path.exists());
}

#[test]
fn render_rejects_existing_output_without_touching_source() {
    let directory = TestDir::new();
    let input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    fs::write(&input.output_mp4_path, b"existing").expect("output should be written");
    let result = render_media(&input, &CancellationToken::new());

    assert!(
        matches!(result, Err(MediaEngineError::OutputConflict { .. })),
        "{result:?}"
    );
    assert_eq!(
        fs::read(&input.input_mp4_path).expect("source should remain"),
        b"source"
    );
    assert_eq!(
        fs::read(&input.output_mp4_path).expect("output should remain"),
        b"existing"
    );
}

#[cfg(unix)]
#[test]
fn render_unavailable_engine_does_not_write_output() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.ffmpeg_path = executable_script(&directory, "ffmpeg", "#!/bin/sh\nexit 1\n");
    input.ffprobe_path = executable_script(&directory, "ffprobe", "#!/bin/sh\nexit 1\n");

    let result = render_media(&input, &CancellationToken::new());

    assert!(matches!(
        result,
        Err(MediaEngineError::EngineUnavailable { .. })
    ));
    assert!(!input.staging_output_path.exists());
    assert!(!input.output_mp4_path.exists());
}

#[cfg(unix)]
#[test]
fn capability_probe_uses_argument_array_without_shell() {
    let directory = TestDir::new();
    let ffmpeg = executable_script(
        &directory,
        "ffmpeg fake",
        "#!/bin/sh\nprintf '%s\\n' 'ffmpeg version test'\n",
    );
    let ffprobe = executable_script(
        &directory,
        "ffprobe fake",
        "#!/bin/sh\nprintf '%s\\n' 'ffprobe version test'\n",
    );

    let status = probe_media_engine_with_paths(&ffmpeg, &ffprobe, 5_000)
        .expect("fake engine should be detected");

    assert!(status.available);
    assert_eq!(
        status.ffmpeg_version.as_deref(),
        Some("ffmpeg version test")
    );
    assert_eq!(
        status.ffprobe_version.as_deref(),
        Some("ffprobe version test")
    );
}

#[cfg(unix)]
#[test]
fn cancelled_render_terminates_worker_and_cleans_partial_output() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.ffmpeg_path = executable_script(
        &directory,
        "ffmpeg-sleep",
        "#!/bin/sh\nif [ \"$2\" = \"-version\" ]; then printf '%s\\n' 'ffmpeg version test'; exit 0; fi\nsleep 10\n",
    );
    input.ffprobe_path = executable_script(
        &directory,
        "ffprobe-ok",
        "#!/bin/sh\nprintf '%s\\n' 'ffprobe version test'\n",
    );
    let token = CancellationToken::new();
    let worker_token = token.clone();
    let worker_input = input.clone();
    let handle = std::thread::spawn(move || render_media(&worker_input, &worker_token));
    std::thread::sleep(std::time::Duration::from_millis(100));
    token.cancel();
    let result = handle.join().expect("worker thread should join");

    assert!(matches!(result, Err(MediaEngineError::Cancelled)));
    assert!(!input.staging_output_path.exists());
    assert!(!input.output_mp4_path.exists());
}
