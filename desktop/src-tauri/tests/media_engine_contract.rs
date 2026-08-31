use autolive_desktop_core::cancellation::CancellationToken;
use autolive_desktop_core::media_effect_params::{
    AudioEffectParams, NaturalVoiceMode, VideoEffectParams,
};
use autolive_desktop_core::media_engine::{
    build_audio_stream_filter_graph_with_ambient, build_media_render_args,
    configured_media_engine_paths_with_resource_dir, render_media, MediaEngineError,
    MediaRenderRequest, MediaRenderTarget,
};
#[cfg(unix)]
use autolive_desktop_core::media_engine::{
    probe_media_engine_with_paths, render_media_with_progress,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
type AudioParameterCase = (&'static str, fn(&mut AudioEffectParams));

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
        source_has_video: true,
        ambient_input_path: None,
        source_duration_ms: Some(1_000),
        source_start_ms: 250,
        output_duration_ms: 500,
        loop_source: false,
        staging_output_path: directory.path().join("staging.partial.mp4"),
        output_mp4_path: directory.path().join("processed.mp4"),
        video_processing_enabled: true,
        audio_processing_enabled: true,
        source_audio_sample_rate_hz: Some(48_000),
        video: VideoEffectParams::default(),
        audio: AudioEffectParams::default(),
        audio_variants: Vec::new(),
        advanced: Default::default(),
        timeout_seconds: 2,
        target: MediaRenderTarget::StandardMp4,
    }
}

#[test]
fn bounded_render_seeks_the_source_and_limits_the_output_duration() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.source_start_ms = 12_345;
    input.output_duration_ms = 8_750;
    input.source_duration_ms = Some(30_000);

    let args = build_media_render_args(&input).expect("bounded request should build");
    let values = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let input_index = values
        .iter()
        .position(|value| value == "-i")
        .expect("source input flag");
    assert_eq!(
        &values[input_index - 4..input_index],
        ["-ss", "12.345", "-t", "8.750"]
    );
    assert!(values.windows(2).any(|pair| pair == ["-t", "8.750"]));
}

#[test]
fn looped_render_bounds_the_primary_input_before_reverse_audio_filters() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.loop_source = true;
    input.source_start_ms = 250;
    input.output_duration_ms = 500;
    input.audio.fade_out_ms = 200;

    let args = build_media_render_args(&input).expect("looped request should build");
    let values = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let input_index = values
        .iter()
        .position(|value| value == "-i")
        .expect("source input flag");

    assert_eq!(
        &values[input_index - 6..input_index],
        ["-stream_loop", "-1", "-ss", "0.250", "-t", "0.500"]
    );
    assert!(audio_graph(&values).contains("areverse"));
}

#[test]
fn bounded_render_rejects_empty_or_out_of_range_windows() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");

    input.output_duration_ms = 0;
    assert!(matches!(
        build_media_render_args(&input),
        Err(MediaEngineError::InvalidParameters { .. })
    ));

    input.output_duration_ms = 500;
    input.source_start_ms = 1_000;
    assert!(matches!(
        build_media_render_args(&input),
        Err(MediaEngineError::InvalidParameters { .. })
    ));

    input.source_start_ms = 750;
    assert!(matches!(
        build_media_render_args(&input),
        Err(MediaEngineError::InvalidParameters { .. })
    ));

    input.loop_source = true;
    let args = build_media_render_args(&input).expect("single-item loop may cross EOF");
    let values = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(values.windows(2).any(|pair| pair == ["-stream_loop", "-1"]));
}

fn fixture_file(directory: &TestDir, name: &str) -> PathBuf {
    let path = directory.path().join(name);
    fs::write(&path, b"engine fixture").expect("fixture file should be written");
    path
}

fn audio_graph(values: &[String]) -> &str {
    values
        .windows(2)
        .find(|pair| pair[0] == "-filter_complex")
        .map(|pair| pair[1].as_str())
        .expect("audio processing should use filter_complex")
}

#[test]
fn configured_media_paths_treat_the_argument_as_the_verified_target_root() {
    let directory = TestDir::new();
    let target_root = directory
        .path()
        .join("runtime-resources/v0.1.0/current-target");
    let expected_ffmpeg = target_root.join(if cfg!(windows) {
        "binaries/ffmpeg.exe"
    } else {
        "binaries/ffmpeg"
    });
    let expected_ffprobe = target_root.join(if cfg!(windows) {
        "binaries/ffprobe.exe"
    } else {
        "binaries/ffprobe"
    });

    let paths = configured_media_engine_paths_with_resource_dir(&target_root)
        .expect("verified target root should resolve deterministic media paths");

    assert_eq!(paths, (expected_ffmpeg, expected_ffprobe));
}

#[test]
fn render_loop_shares_one_deadline_and_gates_encoder_fallback() {
    let source = include_str!("../src/media_engine.rs");
    let render_start = source
        .find("pub fn render_media_with_progress(")
        .expect("render_media_with_progress should exist");
    let render_end = source[render_start..]
        .find("fn media_render_deadline(")
        .map(|offset| render_start + offset)
        .expect("deadline helper should follow render_media");
    let render_body = &source[render_start..render_end];

    assert_eq!(render_body.matches("media_render_deadline(").count(), 1);
    assert!(!render_body.contains("probe_media_engine_with_paths("));
    assert!(!render_body.contains("let started = Instant::now()"));
    assert!(render_body.contains("encoder_failure_allows_retry(&encoder, &error)"));
    assert!(render_body.contains("if !can_try_next"));
    assert!(render_body.contains(".stdout(Stdio::piped())"));
    assert!(render_body.contains("mpsc::sync_channel(8)"));
    assert!(render_body.matches("join_progress_reader(").count() >= 5);
    assert!(render_body.contains("remaining_deadline_millis(render_deadline"));
    assert!(render_body.contains("validate_audio_content(\n            &request.ffmpeg_path,\n            &request.staging_output_path,\n            render_deadline,"));
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

    // 假 ffmpeg 路径探测失败 → 回退 libopenh264；真机可能选硬编，但规划绝不能 video copy。
    assert!(values.windows(2).any(|pair| {
        pair[0] == "-c:v"
            && matches!(
                pair[1].as_str(),
                "libopenh264" | "h264_nvenc" | "h264_amf" | "h264_qsv" | "h264_mf"
            )
    }));
    assert!(!values.windows(2).any(|pair| pair == ["-c:v", "copy"]));
    assert!(values.iter().any(|value| value == "-vf"));
    assert!(values.iter().any(|value| value == "-filter_complex"));
    assert!(!audio_graph(&values).contains("loudnorm="));
    let vf = values
        .windows(2)
        .find(|pair| pair[0] == "-vf")
        .map(|pair| pair[1].clone())
        .expect("video filter");
    assert!(vf.contains("lutyuv="));
    assert!(!vf.contains("eq="));
}

#[test]
fn render_plan_keeps_audio_copy_when_only_video_processing_is_enabled() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.audio_processing_enabled = false;

    let args = build_media_render_args(&input).expect("video-only render args should be valid");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();

    assert!(values.windows(2).any(|pair| {
        pair[0] == "-c:v"
            && matches!(
                pair[1].as_str(),
                "libopenh264" | "h264_nvenc" | "h264_amf" | "h264_qsv" | "h264_mf"
            )
    }));
    assert!(values.windows(2).any(|pair| pair == ["-c:a", "copy"]));
    assert!(values.iter().any(|value| value == "-vf"));
    assert!(!values.iter().any(|value| value == "-af"));
}

#[test]
fn select_h264_encoder_falls_back_when_ffmpeg_is_unusable() {
    use autolive_desktop_core::media_engine::select_h264_encoder;
    let directory = TestDir::new();
    let fake = fixture_file(&directory, "not-a-real-ffmpeg");
    assert_eq!(select_h264_encoder(&fake), "libopenh264");
}

#[test]
fn audio_processing_reencodes_video_for_an_exact_candidate_start() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.video_processing_enabled = false;

    let args = build_media_render_args(&input).expect("audio-only render args should be valid");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();

    assert!(values.windows(2).any(|pair| {
        pair[0] == "-c:v"
            && matches!(
                pair[1].as_str(),
                "libopenh264" | "h264_nvenc" | "h264_amf" | "h264_qsv" | "h264_mf"
            )
    }));
    assert!(!values.windows(2).any(|pair| pair == ["-c:v", "copy"]));
    assert!(values.windows(2).any(|pair| pair == ["-c:a", "aac"]));
    assert!(!values.iter().any(|value| value == "-vf"));
    assert!(values.iter().any(|value| value == "-filter_complex"));
}

#[test]
fn render_plan_keeps_stream_copy_when_no_processing_is_requested() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.video_processing_enabled = false;
    input.audio_processing_enabled = false;

    let args = build_media_render_args(&input).expect("passthrough args should be valid");
    let values = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    assert!(values.windows(2).any(|pair| pair == ["-c:v", "copy"]));
    assert!(values.windows(2).any(|pair| pair == ["-c:a", "copy"]));
}

#[test]
fn pure_audio_candidate_accepts_audio_extensions_without_video_mapping() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    input.input_mp4_path = directory.path().join("source.mp3");
    fs::write(&input.input_mp4_path, b"source").expect("audio source should be written");
    input.source_has_video = false;
    input.video_processing_enabled = false;
    input.audio_processing_enabled = true;
    input.staging_output_path = directory.path().join("staging.partial.m4a");
    input.output_mp4_path = directory.path().join("processed.m4a");

    let args = build_media_render_args(&input).expect("pure audio candidate should be valid");
    let values = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    assert!(!values.iter().any(|value| value == "0:v:0?"));
    assert!(!values.iter().any(|value| value == "-c:v"));
    assert!(values.windows(2).any(|pair| pair == ["-c:a", "aac"]));
}

#[test]
fn independent_audio_candidate_covers_the_requested_duration_after_speed_change() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.source_has_video = false;
    input.video_processing_enabled = false;
    input.staging_output_path = directory.path().join("staging.partial.m4a");
    input.output_mp4_path = directory.path().join("processed.m4a");
    input.output_duration_ms = 5_500;
    input.source_duration_ms = Some(60_000);
    input.audio.playback_speed = 1.25;
    input.audio_variants = vec![AudioEffectParams {
        playback_speed: 1.5,
        ..input.audio.clone()
    }];

    let args = build_media_render_args(&input).expect("audio candidate should be valid");
    let values = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let input_index = values
        .iter()
        .position(|value| value == "-i")
        .expect("source input flag");
    assert_eq!(&values[input_index - 2..input_index], ["-t", "8.250"]);
    let graph = audio_graph(&values);
    assert!(graph.contains(
        "[aout]apad=whole_dur=5.500,atrim=duration=5.500,asetpts=PTS-STARTPTS[aout_bounded]"
    ));
    assert!(values
        .windows(2)
        .any(|pair| pair == ["-map", "[aout_bounded]"]));
}

#[test]
fn render_maps_independent_audio_eq_speed_and_reverb_filters() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.video_processing_enabled = false;
    input.audio.playback_speed = 1.25;
    input.audio.low_eq_db = 3.0;
    input.audio.mid_eq_db = -2.0;
    input.audio.high_eq_db = 1.5;
    input.audio.fade_in_ms = 250;
    input.audio.fade_out_ms = 500;
    input.audio.reverb_wet_percent = 10.0;

    let args = build_media_render_args(&input).expect("audio filters should be mapped");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let filter = audio_graph(&values);

    assert!(filter.contains("equalizer=f=200"));
    assert!(filter.contains("equalizer=f=1000"));
    assert!(filter.contains("equalizer=f=8000"));
    assert!(filter.contains("atempo=1.250000"));
    assert!(filter.contains("afade=t=in:st=0:d=0.250000"));
    assert!(filter.contains("areverse,afade=t=in:st=0:d=0.500000,areverse"));
    assert!(!filter.contains("afade=t=out:d=0.500000"));
    assert!(filter.contains("aecho=1.0:1.0:"));
}

#[test]
fn render_plan_accepts_an_allowlisted_non_mp4_source() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    input.input_mp4_path = directory.path().join("source.mkv");
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");

    let args = build_media_render_args(&input).expect("allowlisted source should be accepted");

    assert!(args.iter().any(|value| value == &input.input_mp4_path));
}

#[test]
fn render_accepts_pitch_shift_with_fallback_sample_rate() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.audio.pitch_shift_semitones = 0.5;
    input.source_audio_sample_rate_hz = None;

    let args = build_media_render_args(&input).expect("pitch should fall back to 48000");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let filter = audio_graph(&values);
    assert!(filter.contains("asetrate="));
    assert!(filter.contains("aresample=48000"));
}

#[test]
fn offline_render_only_rejects_effects_that_require_a_missing_input() {
    let directory = TestDir::new();
    let cases: [AudioParameterCase; 1] = [("audio.ambient_sound_mix_percent", |audio| {
        audio.ambient_sound_mix_percent = 1.0
    })];

    for (field, configure) in cases {
        let mut input = request(&directory);
        fs::write(&input.input_mp4_path, b"source").expect("source should be written");
        configure(&mut input.audio);

        let error = build_media_render_args(&input)
            .expect_err("offline-only render boundary should reject missing runtime support");
        assert!(
            matches!(&error, MediaEngineError::InvalidParameters { message } if message.contains(field)),
            "{field}: {error:?}"
        );
    }
}

#[test]
fn offline_render_keeps_pending_pcm_fields_without_blocking_mapped_audio_effects() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.audio.mfcc_shift_percent = 1.0;
    input.audio.formant_shift_percent = 1.0;
    input.audio.input_gain_db = 1.0;

    let args = build_media_render_args(&input).expect("mapped effects should still render");
    let values = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(audio_graph(&values).contains("volume="));
}

#[test]
fn render_maps_natural_dynamic_and_dry_wet_audio_effects() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.audio.natural_voice_mode = NaturalVoiceMode::NaturalDynamic;
    input.audio.dry_wet_percent = 25.0;

    let args = build_media_render_args(&input).expect("mapped effects should build");
    let values = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let graph = audio_graph(&values);
    assert!(graph.contains("volume='1+0.012000*sin"), "{graph}");
    assert!(
        graph.contains("[dry0][wet0]amix=inputs=2:weights=0.750000 0.250000"),
        "{graph}"
    );
}

#[test]
fn render_maps_spectral_and_high_frequency_audio_perturbation() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.audio.spectral_perturbation_percent = 1.0;
    input.audio.high_frequency_perturbation_enabled = true;
    input.audio.high_frequency_perturbation_strength_percent = 4.0;

    let args = build_media_render_args(&input).expect("frequency effects should be mapped");
    let joined = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(joined.matches("afftfilt=").count() >= 2, "{joined}");
    assert!(joined.contains("gte(b/nb\\,0.25)"), "{joined}");
}

#[test]
fn render_maps_ffmpeg_native_noise_phase_and_vibrato_filters() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.audio.noise_reduction_percent = 20.0;
    input.audio.phase_perturbation_percent = 5.0;
    input.audio.vibrato_frequency_hz = 5.5;
    input.audio.vibrato_depth_percent = 1.0;

    let args = build_media_render_args(&input).expect("native audio filters should be mapped");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let filter = audio_graph(&values);

    assert!(filter.contains("afftdn=nr=19.400000"));
    assert!(filter.contains("aphaser="));
    assert!(filter.contains("vibrato=f=5.500000:d=0.010000"));
}

#[test]
fn render_caps_aphaser_decay_at_ffmpeg_maximum() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.audio.phase_perturbation_percent = 20.0;

    let args = build_media_render_args(&input).expect("maximum phase perturbation should map");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let filter = audio_graph(&values);

    assert!(filter.contains("decay=0.990000"));
    assert!(!filter.contains("decay=1.000000"));
}

#[test]
fn render_maps_environment_noise_with_ffmpeg_native_mix() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.audio.environment_noise_percent = 8.0;
    input.audio.environment_noise_dbfs = -42.0;

    let args = build_media_render_args(&input).expect("environment noise should be mapped");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let graph = values
        .windows(2)
        .find(|pair| pair[0] == "-filter_complex")
        .map(|pair| pair[1].clone())
        .expect("environment noise must use filter_complex");

    assert!(graph.contains("anoisesrc=color=white:amplitude="));
    assert!(graph.contains("amix=inputs=2:weights=0.920000 0.080000:duration=shortest"));
    assert!(!graph.contains("loudnorm="));
    assert!(graph.contains("aresample="));
    assert!(values
        .windows(2)
        .any(|pair| pair[0] == "-map" && pair[1] == "[aout]"));
    assert!(!values.iter().any(|value| value == "-af"));
}

#[test]
fn render_normalizes_unknown_source_sample_rate_to_48000() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.source_audio_sample_rate_hz = Some(32_000);

    let args = build_media_render_args(&input).expect("unknown source rate should be normalized");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let filter = audio_graph(&values);

    assert!(filter.contains("aresample=48000"));
    assert!(values
        .windows(2)
        .any(|pair| pair[0] == "-ar" && pair[1] == "48000"));
    assert!(!values
        .windows(2)
        .any(|pair| pair[0] == "-ar" && pair[1] == "32000"));
}

#[test]
fn render_guards_non_finite_audio_before_the_source_relative_output_bus() {
    let directory = TestDir::new();
    let input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");

    let args = build_media_render_args(&input).expect("audio guard should be valid");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let filter = audio_graph(&values);
    let finite_guard = filter
        .find("aeval=exprs=if(isnan(val(0))")
        .expect("non-finite audio guard should be present");
    let highpass = filter
        .find("highpass=f=50")
        .expect("output highpass should be present");

    assert!(finite_guard < highpass);
    assert!(!filter.contains("loudnorm="));
    assert!(filter.contains("isinf(val(0))"));
    assert!(filter.contains("isnan(val(1))"));
}

#[test]
fn render_generates_environment_noise_inside_each_audio_variant_branch() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    let branch_a = AudioEffectParams {
        environment_noise_percent: 8.0,
        ..Default::default()
    };
    let branch_b = AudioEffectParams {
        environment_noise_percent: 12.0,
        ..Default::default()
    };
    input.audio_variants = vec![branch_a, branch_b];

    let args = build_media_render_args(&input).expect("branch noise should be mapped");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let graph = audio_graph(&values);

    assert_eq!(graph.match_indices("anoisesrc=color=white").count(), 2);
    assert!(graph.contains("[processed0][noise0]amix=inputs=2"));
    assert!(graph.contains("[processed1][noise1]amix=inputs=2"));
    assert!(!graph.contains("environment_noise_percent"));
}

#[test]
fn render_rejects_more_than_four_audio_variants_at_the_media_boundary() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.audio_variants = vec![AudioEffectParams::default(); 5];

    let error = build_media_render_args(&input).expect_err("five variants must be rejected");
    assert!(
        matches!(&error, MediaEngineError::InvalidParameters { message }
            if message.contains("audio_variants 最多允许 4 条")),
        "{error:?}"
    );
}

#[test]
fn render_validates_each_audio_variant_range_and_finite_value() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    let invalid = AudioEffectParams {
        pitch_shift_semitones: f64::NAN,
        ..Default::default()
    };
    input.audio_variants = vec![invalid];

    let error = build_media_render_args(&input).expect_err("non-finite variant must be rejected");
    assert!(
        matches!(&error, MediaEngineError::InvalidParameters { message }
            if message.contains("audio_variants[0].pitch_shift_semitones")),
        "{error:?}"
    );
}

#[test]
fn render_maps_dry_wet_parameter_inside_audio_variant() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    let variant = AudioEffectParams {
        dry_wet_percent: 1.0,
        ..Default::default()
    };
    input.audio_variants = vec![variant];

    let args = build_media_render_args(&input).expect("mapped variant field should build");
    let values = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(audio_graph(&values).contains("[dry0][wet0]amix=inputs=2"));
}

#[test]
fn render_accepts_valid_audio_variant_when_processing_is_disabled() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.audio_processing_enabled = false;
    let variant = AudioEffectParams {
        dry_wet_percent: 1.0,
        ..Default::default()
    };
    input.audio_variants = vec![variant];

    let args = build_media_render_args(&input)
        .expect("valid inactive audio configuration should remain serializable");
    assert!(!args.iter().any(|value| value == "-filter_complex"));
}

#[test]
fn standard_mp4_compatibility_is_limited_to_cpu4_video_fields() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.video.brightness_percent = 0.2;
    input.video.contrast_percent = 100.2;
    input.video.saturation_percent = 99.8;
    input.video.hue_rotation_degrees = 0.1;
    input.video.color_space_conversion_enabled = true;
    input.video.color_space_conversion_strength_percent = 25.0;
    input.advanced.local_blur_enabled = true;
    input.advanced.picture_in_picture_enabled = true;

    let args = build_media_render_args(&input).expect("CPU4 compatibility args should build");
    let values = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let filter = values
        .windows(2)
        .find(|pair| pair[0] == "-vf")
        .map(|pair| pair[1].clone())
        .expect("CPU4 video filter should be present");

    assert!(filter.contains("lutyuv="));
    assert!(filter.contains("hue=h=0.100000:s=0.998000"));
    assert!(!filter.contains("colorspace="));
    assert!(!filter.contains("gblur="));
    assert!(!filter.contains("overlay="));
    assert!(!filter.contains("geq="));
    assert!(values.windows(2).any(|pair| pair == ["-map", "[aout]"]));
    assert!(!values.windows(2).any(|pair| pair == ["-map", "[vout]"]));
}

#[test]
fn render_mixes_audio_variants_to_single_bus_with_equal_weights() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    let branch_a = AudioEffectParams {
        low_eq_db: 0.4,
        ..Default::default()
    };
    let branch_b = AudioEffectParams {
        high_eq_db: 0.5,
        ..Default::default()
    };
    input.audio_variants = vec![branch_a, branch_b];

    let args = build_media_render_args(&input).expect("multi-variant mix should be valid");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let graph = values
        .windows(2)
        .find(|pair| pair[0] == "-filter_complex")
        .map(|pair| pair[1].clone())
        .expect("filter_complex should be present for k>1");

    assert!(graph.contains("asplit=2"));
    assert!(graph.contains("amix=inputs=2:weights=0.500000 0.500000:duration=shortest"));
    assert!(graph.contains("highpass=f=50"));
    assert!(!graph.contains("loudnorm="));
    assert!(graph.contains("aresample="));
    assert!(values
        .windows(2)
        .any(|pair| pair[0] == "-map" && pair[1] == "[aout]"));
    assert!(!values.iter().any(|value| value == "-af"));
}

#[test]
fn packaged_ffmpeg_keeps_nonzero_timestamp_multi_branch_mix_audible() {
    let Some(ffmpeg) = std::env::var_os("AUTOLIVE_TEST_FFMPEG") else {
        return;
    };
    let audio = AudioEffectParams {
        ambient_sound_mix_percent: 10.0,
        ..Default::default()
    };
    let variants = [
        AudioEffectParams {
            dry_wet_percent: 25.0,
            environment_noise_percent: 0.01,
            environment_noise_dbfs: -60.0,
            ambient_sound_mix_percent: 10.0,
            ..Default::default()
        },
        AudioEffectParams {
            dry_wet_percent: 35.0,
            environment_noise_percent: 0.01,
            environment_noise_dbfs: -60.0,
            ambient_sound_mix_percent: 10.0,
            ..Default::default()
        },
    ];
    let graph =
        build_audio_stream_filter_graph_with_ambient(&audio, &variants, Some(48_000), 48_000, true)
            .expect("multi-branch graph should build")
            .filter_graph;

    for fragment in [
        "asplit=2[a0][a1]",
        "[processed0]asplit=2",
        "[processed1]asplit=2",
        "anoisesrc=color=white",
        "[1:a:0]asetpts=PTS-STARTPTS,aformat=",
        "[variant_bus][ambient]amix=inputs=2",
    ] {
        assert!(graph.contains(fragment), "{fragment}: {graph}");
    }

    let output = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=0.4,asetpts=PTS+5/TB",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo:d=0.4,asetpts=PTS+5/TB",
            "-filter_complex",
            &graph,
            "-map",
            "[aout]",
            "-t",
            "0.400",
            "-c:a",
            "pcm_f32le",
            "-f",
            "f32le",
            "-",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("packaged FFmpeg should run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.len() % std::mem::size_of::<f32>(), 0);
    let samples = output
        .stdout
        .chunks_exact(std::mem::size_of::<f32>())
        .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("one f32 sample")))
        .collect::<Vec<_>>();
    assert!(
        !samples.is_empty(),
        "nonzero source PTS must still produce PCM"
    );
    assert!(samples.iter().all(|sample| sample.is_finite()));
    let peak = samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0_f32, f32::max);
    let rms =
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt();
    assert!(peak > 0.01, "mixed PCM peak is silent: {peak}");
    assert!(rms > 0.001, "mixed PCM RMS is silent: {rms}");
}

#[test]
fn render_uses_source_relative_output_chain_for_single_audio_variant() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    let only = AudioEffectParams {
        mid_eq_db: 0.3,
        ..Default::default()
    };
    input.audio_variants = vec![only];

    let args = build_media_render_args(&input).expect("single variant should stay on -af");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let filter = audio_graph(&values);
    assert!(filter.contains("asplit=1"));
    assert!(filter.contains("aformat=sample_fmts=fltp:channel_layouts=stereo"));
    assert!(filter.contains("amix=inputs=1:weights=1.000000:duration=shortest"));
    assert!(filter.contains("highpass=f=50"));
    assert!(filter.contains("adenorm=level=-351:type=ac"));
    assert!(!filter.contains("loudnorm="));
    assert!(filter.contains("aresample="));
    assert!(
        filter.contains("aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo[aout]")
    );
    assert!(values
        .windows(2)
        .any(|pair| pair[0] == "-ar" && pair[1] == "48000"));
    assert!(values
        .windows(2)
        .any(|pair| pair[0] == "-ac" && pair[1] == "2"));
    assert!(values
        .windows(2)
        .any(|pair| pair[0] == "-sample_fmt" && pair[1] == "fltp"));
    assert!(!filter.contains("alimiter="));
    let amix = filter
        .find("amix=inputs=1")
        .expect("final amix should exist");
    let highpass = filter.find("highpass=f=50").expect("highpass should exist");
    let aresample = filter
        .rfind("aresample=")
        .expect("final aresample should exist");
    assert!(amix < highpass && highpass < aresample);
    assert!(filter.contains("equalizer=f=1000"));
    assert!(!values.iter().any(|value| value == "-af"));
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
    let filter = audio_graph(&values);

    assert!(filter.contains("aresample=44100"));
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
    let filter = audio_graph(&values);

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
        "#!/bin/sh\nif [ \"$2\" = \"-version\" ]; then printf '%s\\n' 'ffmpeg version test'; exit 0; fi\ninput=\"\"\nlast=\"\"\nprevious=\"\"\nfor arg in \"$@\"; do if [ \"$previous\" = \"-i\" ]; then input=\"$arg\"; fi; previous=\"$arg\"; last=\"$arg\"; done\nprintf '%s\\n' 'out_time_us=500000' 'progress=continue'\n/bin/cp \"$input\" \"$last\"\n",
    );
    input.ffprobe_path = executable_script(
        &directory,
        "ffprobe-ok",
        "#!/bin/sh\nif [ \"$2\" = \"-version\" ]; then printf '%s\\n' 'ffprobe version test'; fi\nexit 0\n",
    );

    let mut progress = Vec::new();
    let result = render_media_with_progress(&input, &CancellationToken::new(), |percent| {
        progress.push(percent);
    })
    .expect("output should commit");

    assert_eq!(result.output_mp4_path, input.output_mp4_path);
    assert_eq!(result.output_size_bytes, 6);
    assert_eq!(result.output_mp4_sha256.len(), 64);
    assert_eq!(progress, [50, 100]);
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
