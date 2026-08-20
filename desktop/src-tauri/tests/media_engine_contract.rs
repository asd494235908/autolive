use autolive_desktop_core::cancellation::CancellationToken;
#[cfg(unix)]
use autolive_desktop_core::media_engine::probe_media_engine_with_paths;
use autolive_desktop_core::media_engine::{
    build_media_render_args, configured_media_engine_paths_with_resource_dir, render_media,
    MediaEngineError, MediaRenderRequest,
};
use autolive_desktop_core::research_params::{
    AudioResearchParams, NaturalVoiceMode, VideoResearchParams,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
type AudioParameterCase = (&'static str, fn(&mut AudioResearchParams));

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
        audio_variants: Vec::new(),
        research: Default::default(),
        timeout_seconds: 2,
    }
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
    assert!(audio_graph(&values).contains("loudnorm=I=-16:TP=-1.5:LRA=11"));
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
fn render_plan_keeps_video_copy_when_only_audio_processing_is_enabled() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.video_processing_enabled = false;

    let args = build_media_render_args(&input).expect("audio-only render args should be valid");
    let values: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();

    assert!(values.windows(2).any(|pair| pair == ["-c:v", "copy"]));
    assert!(values.windows(2).any(|pair| pair == ["-c:a", "aac"]));
    assert!(!values.iter().any(|value| value == "-vf"));
    assert!(values.iter().any(|value| value == "-filter_complex"));
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
fn render_rejects_unmapped_audio_parameters() {
    let directory = TestDir::new();
    let cases: [AudioParameterCase; 6] = [
        ("audio.natural_voice_mode", |audio| {
            audio.natural_voice_mode = NaturalVoiceMode::NaturalDynamic
        }),
        ("audio.ambient_sound_mix_percent", |audio| {
            audio.ambient_sound_mix_percent = 1.0
        }),
        ("audio.dry_wet_percent", |audio| audio.dry_wet_percent = 1.0),
        ("audio.mfcc_shift_percent", |audio| {
            audio.mfcc_shift_percent = 1.0
        }),
        ("audio.formant_shift_percent", |audio| {
            audio.formant_shift_percent = 1.0
        }),
        ("audio.spectral_perturbation_percent", |audio| {
            audio.spectral_perturbation_percent = 1.0
        }),
    ];

    for (field, configure) in cases {
        let mut input = request(&directory);
        fs::write(&input.input_mp4_path, b"source").expect("source should be written");
        configure(&mut input.audio);

        let error = build_media_render_args(&input).expect_err("unmapped parameter should fail");
        assert!(
            matches!(&error, MediaEngineError::InvalidParameters { message } if message.contains(field)),
            "{field}: {error:?}"
        );
    }
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
    assert!(graph.contains("amix=inputs=2:weights=0.920000 0.080000:duration=first"));
    assert!(graph.contains("loudnorm=I=-16:TP=-1.5:LRA=11"));
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
fn render_guards_non_finite_audio_before_loudnorm_and_aac() {
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
    let loudnorm = filter
        .find("loudnorm=I=-16:TP=-1.5:LRA=11")
        .expect("loudnorm should be present");

    assert!(finite_guard < loudnorm);
    assert!(filter.contains("isinf(val(0))"));
    assert!(filter.contains("isnan(val(1))"));
}

#[test]
fn render_generates_environment_noise_inside_each_audio_variant_branch() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    let branch_a = AudioResearchParams {
        environment_noise_percent: 8.0,
        ..Default::default()
    };
    let branch_b = AudioResearchParams {
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
    assert!(graph.contains("[dry0][noise0]amix=inputs=2"));
    assert!(graph.contains("[dry1][noise1]amix=inputs=2"));
    assert!(!graph.contains("environment_noise_percent"));
}

#[test]
fn render_rejects_more_than_four_audio_variants_at_the_media_boundary() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.audio_variants = vec![AudioResearchParams::default(); 5];

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
    let invalid = AudioResearchParams {
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
fn render_rejects_unmapped_parameter_inside_audio_variant() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    let invalid = AudioResearchParams {
        dry_wet_percent: 1.0,
        ..Default::default()
    };
    input.audio_variants = vec![invalid];

    let error = build_media_render_args(&input).expect_err("unmapped variant field must fail");
    assert!(
        matches!(&error, MediaEngineError::InvalidParameters { message }
            if message.contains("audio_variants[0].dry_wet_percent")),
        "{error:?}"
    );
}

#[test]
fn render_rejects_unmapped_audio_variant_when_processing_is_disabled() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    input.audio_processing_enabled = false;
    let invalid = AudioResearchParams {
        dry_wet_percent: 1.0,
        ..Default::default()
    };
    input.audio_variants = vec![invalid];

    let error = build_media_render_args(&input)
        .expect_err("unmapped variant must fail even when audio processing is disabled");
    assert!(
        matches!(&error, MediaEngineError::InvalidParameters { message }
            if message.contains("audio_variants[0].dry_wet_percent")),
        "{error:?}"
    );
}

#[test]
fn render_mixes_audio_variants_to_single_bus_with_equal_weights() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    let branch_a = AudioResearchParams {
        low_eq_db: 0.4,
        ..Default::default()
    };
    let branch_b = AudioResearchParams {
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
    assert!(graph.contains("amix=inputs=2:weights=0.500000 0.500000:duration=first"));
    assert!(graph.contains("highpass=f=50"));
    assert!(graph.contains("loudnorm=I=-16:TP=-1.5:LRA=11"));
    assert!(graph.contains("aresample="));
    assert!(values
        .windows(2)
        .any(|pair| pair[0] == "-map" && pair[1] == "[aout]"));
    assert!(!values.iter().any(|value| value == "-af"));
}

#[test]
fn render_uses_final_loudness_chain_for_single_audio_variant() {
    let directory = TestDir::new();
    let mut input = request(&directory);
    fs::write(&input.input_mp4_path, b"source").expect("source should be written");
    let only = AudioResearchParams {
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
    assert!(filter.contains("amix=inputs=1:weights=1.000000:duration=first"));
    assert!(filter.contains("highpass=f=50"));
    assert!(filter.contains("adenorm=level=-351:type=ac"));
    assert!(filter.contains("loudnorm=I=-16:TP=-1.5:LRA=11"));
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
    let loudnorm = filter
        .find("loudnorm=I=-16:TP=-1.5:LRA=11")
        .expect("loudnorm should exist");
    let aresample = filter.find("aresample=").expect("aresample should exist");
    assert!(amix < highpass && highpass < loudnorm && loudnorm < aresample);
    assert!(filter.contains("equalizer=f=1000"));
    assert!(!values.iter().any(|value| value == "-af"));
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
    let filter = audio_graph(&values);

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
