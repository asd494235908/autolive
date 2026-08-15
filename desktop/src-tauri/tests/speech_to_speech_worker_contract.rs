use autolive_desktop_core::cancellation::CancellationToken;
use autolive_desktop_core::speech_to_speech::SpeechToSpeechContext;
#[cfg(not(debug_assertions))]
use autolive_desktop_core::speech_to_speech_worker::configured_worker_executable;
use autolive_desktop_core::speech_to_speech_worker::{
    probe_speech_to_speech_worker, probe_speech_to_speech_worker_with_resource_dir,
    run_speech_to_speech_context_worker, run_speech_to_speech_worker,
    run_speech_to_speech_worker_with_resource_dir, worker_environment_policy,
    SpeechToSpeechContextWorkerRequest, SpeechToSpeechWorkerError, SpeechToSpeechWorkerRequest,
    WorkerEnvironmentPolicy,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static ENVIRONMENT_LOCK: Mutex<()> = Mutex::new(());

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "autolive-speech-worker-{}-{}",
            std::process::id(),
            TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("test directory should be created");
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ignored = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn worker_environment_policy_allows_overrides_only_for_debug_builds() {
    assert_eq!(
        worker_environment_policy(true),
        WorkerEnvironmentPolicy::DevelopmentOverrides
    );
    assert_eq!(
        worker_environment_policy(false),
        WorkerEnvironmentPolicy::PackagedOnly
    );
}

#[cfg(not(debug_assertions))]
#[test]
fn configured_worker_executable_ignores_environment_in_release() {
    let _environment_guard = ENVIRONMENT_LOCK.lock().expect("environment lock");
    let directory = TestDir::new();
    let executable = directory.0.join("untrusted-worker");
    std::fs::write(&executable, b"fixture").expect("worker fixture");
    let previous = std::env::var_os("AUTOLIVE_SPEECH_TO_SPEECH_WORKER");
    std::env::set_var("AUTOLIVE_SPEECH_TO_SPEECH_WORKER", &executable);

    assert_eq!(
        configured_worker_executable(),
        Err(SpeechToSpeechWorkerError::WorkerNotConfigured)
    );

    match previous {
        Some(value) => std::env::set_var("AUTOLIVE_SPEECH_TO_SPEECH_WORKER", value),
        None => std::env::remove_var("AUTOLIVE_SPEECH_TO_SPEECH_WORKER"),
    }
}

#[cfg(unix)]
fn make_executable(path: &std::path::Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).expect("worker script should be written");
    let mut permissions = std::fs::metadata(path)
        .expect("worker metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("worker should be executable");
}

#[cfg(unix)]
#[test]
fn worker_uses_argument_array_and_commits_valid_json_result() {
    let directory = TestDir::new();
    let executable = directory.0.join("worker.sh");
    let input = directory.0.join("input.json");
    let output = directory.0.join("result.json");
    std::fs::write(&input, b"{}").expect("input should be written");
    make_executable(
        &executable,
        "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --output-json) output=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\ncat > \"$output\" <<'JSON'\n{\"decision\":\"keep_original\",\"text\":\"原始话术\",\"audio_path_or_stream_ref\":null,\"audio_sha256\":null,\"duration_ms\":null,\"sync_offset_ms\":null,\"sample_rate_hz\":null,\"channel_count\":null,\"latency_ms\":20,\"model\":\"worker-test\",\"fallback_reason\":null}\nJSON\n",
    );

    let result = run_speech_to_speech_worker(
        &SpeechToSpeechWorkerRequest {
            executable,
            input_json_path: input,
            output_json_path: output.clone(),
            timeout_ms: 5_000,
        },
        &CancellationToken::new(),
    )
    .expect("worker result should be valid");

    assert_eq!(result.model, "worker-test");
    assert_eq!(result.text, "原始话术");
    assert!(output.exists());
}

#[cfg(unix)]
#[test]
fn capability_probe_accepts_structured_worker_capability_output() {
    let directory = TestDir::new();
    let executable = directory.0.join("worker.sh");
    make_executable(
        &executable,
        "#!/bin/sh\nif [ \"$1\" = \"--capabilities-json\" ]; then\n  printf '%s' '{\"available\":true,\"status\":\"available\",\"provider\":\"hf-speech-to-speech\",\"model\":\"local\",\"reason\":null}' > \"$2\"\n  exit 0\nfi\nexit 1\n",
    );

    let capabilities = probe_speech_to_speech_worker(&executable, 2_000)
        .expect("capability output should be valid");
    assert!(capabilities.available);
    assert_eq!(
        capabilities.provider.as_deref(),
        Some("hf-speech-to-speech")
    );
}

#[cfg(unix)]
#[test]
fn context_worker_serializes_context_and_cleans_temporary_context_file() {
    let directory = TestDir::new();
    let executable = directory.0.join("worker.sh");
    let context_path = directory.0.join("context.json");
    let output = directory.0.join("result.json");
    make_executable(
        &executable,
        "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --input-json) input=\"$2\"; shift 2 ;;\n    --output-json) output=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\ngrep -q 'segment-test' \"$input\" || exit 3\ncat > \"$output\" <<'JSON'\n{\"decision\":\"keep_original\",\"text\":\"原始话术\",\"audio_path_or_stream_ref\":null,\"audio_sha256\":null,\"duration_ms\":null,\"sync_offset_ms\":null,\"sample_rate_hz\":null,\"channel_count\":null,\"latency_ms\":20,\"model\":\"worker-test\",\"fallback_reason\":null}\nJSON\n",
    );
    let context = SpeechToSpeechContext {
        track_id: "track-test".to_owned(),
        source_kind: "local_file".to_owned(),
        playback_generation: 1,
        loop_index: 2,
        segment_id: "segment-test".to_owned(),
        start_at_ms: 0,
        sample_rate_hz: 16_000,
        channel_count: 1,
        audio_path_or_stream_ref: "/tmp/source.wav".to_owned(),
        transcript_text: "原始话术".to_owned(),
        previous_variant_text: None,
        locked_fields: vec![],
        locked_field_values: vec![],
        target_duration_ms: 2_000,
        max_chars: 40,
        language: "zh-CN".to_owned(),
        rewrite_policy: "keep_meaning".to_owned(),
        timeout_ms: 2_000,
    };

    let result = run_speech_to_speech_context_worker(
        &SpeechToSpeechContextWorkerRequest {
            executable,
            context_json_path: context_path.clone(),
            output_json_path: output.clone(),
            timeout_ms: 2_000,
        },
        &context,
        &CancellationToken::new(),
    )
    .expect("context worker result should be valid");

    assert_eq!(
        result.decision,
        autolive_desktop_core::speech_to_speech::SpeechToSpeechDecision::KeepOriginal
    );
    assert!(!context_path.exists());
    assert!(output.exists());
}

#[cfg(unix)]
#[test]
fn cancelled_worker_is_terminated_and_staging_output_is_removed() {
    let directory = TestDir::new();
    let executable = directory.0.join("worker.sh");
    let input = directory.0.join("input.json");
    let output = directory.0.join("result.json");
    std::fs::write(&input, b"{}").expect("input should be written");
    make_executable(
        &executable,
        "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --output-json) output=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\ntouch \"$output\"\nsleep 10\n",
    );

    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let request = SpeechToSpeechWorkerRequest {
        executable,
        input_json_path: input,
        output_json_path: output.clone(),
        timeout_ms: 5_000,
    };
    let handle =
        std::thread::spawn(move || run_speech_to_speech_worker(&request, &worker_cancellation));
    std::thread::sleep(std::time::Duration::from_millis(100));
    cancellation.cancel();

    assert_eq!(
        handle.join().expect("worker thread should join"),
        Err(SpeechToSpeechWorkerError::Cancelled)
    );
    assert!(!output.exists());
    assert!(!directory.0.join("result.json.partial").exists());
}

#[cfg(unix)]
#[test]
fn timed_out_worker_is_terminated_and_staging_output_is_removed() {
    let directory = TestDir::new();
    let executable = directory.0.join("worker.sh");
    let input = directory.0.join("input.json");
    let output = directory.0.join("result.json");
    std::fs::write(&input, b"{}").expect("input should be written");
    make_executable(
        &executable,
        "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --output-json) output=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\ntouch \"$output\"\nsleep 10\n",
    );

    let result = run_speech_to_speech_worker(
        &SpeechToSpeechWorkerRequest {
            executable,
            input_json_path: input,
            output_json_path: output.clone(),
            timeout_ms: 100,
        },
        &CancellationToken::new(),
    );

    assert_eq!(
        result,
        Err(SpeechToSpeechWorkerError::Timeout { timeout_ms: 100 })
    );
    assert!(!output.exists());
    assert!(!directory.0.join("result.json.partial").exists());
}

#[cfg(unix)]
#[test]
fn packaged_media_paths_are_injected_for_capability_probe_and_actual_task() {
    let _environment_guard = ENVIRONMENT_LOCK
        .lock()
        .expect("speech worker environment lock should work");
    let directory = TestDir::new();
    let resource_dir = directory.0.join("resources");
    let executable = directory.0.join("worker.sh");
    let input = directory.0.join("input.json");
    let output = directory.0.join("result.json");
    let captured_environment = directory.0.join("media-environment.txt");
    std::fs::create_dir_all(resource_dir.join("binaries")).expect("resource directory");
    std::fs::write(resource_dir.join("binaries/ffmpeg"), b"ffmpeg").expect("ffmpeg fixture");
    std::fs::write(resource_dir.join("binaries/ffprobe"), b"ffprobe").expect("ffprobe fixture");
    std::fs::write(&input, b"{}").expect("input should be written");
    make_executable(
        &executable,
        &format!(
            "#!/bin/sh\n\nif [ \"$1\" = \"--capabilities-json\" ]; then\n  printf '%s\\n%s' \"$AUTOLIVE_FFMPEG_PATH\" \"$AUTOLIVE_FFPROBE_PATH\" > '{}'\n  printf '%s' '{{\"available\":true,\"status\":\"available\",\"provider\":\"hf-speech-to-speech\",\"model\":\"local\",\"reason\":null}}' > \"$2\"\n  exit 0\nfi\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --output-json) output=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\nprintf '%s\\n%s' \"$AUTOLIVE_FFMPEG_PATH\" \"$AUTOLIVE_FFPROBE_PATH\" > '{}'\nprintf '%s' '{{\"decision\":\"keep_original\",\"text\":\"原始话术\",\"audio_path_or_stream_ref\":null,\"audio_sha256\":null,\"duration_ms\":null,\"sync_offset_ms\":null,\"sample_rate_hz\":null,\"channel_count\":null,\"latency_ms\":20,\"model\":\"worker-test\",\"fallback_reason\":null}}' > \"$output\"\n",
            captured_environment.display(),
            captured_environment.display(),
        ),
    );

    let previous_ffmpeg = std::env::var_os("AUTOLIVE_FFMPEG_PATH");
    let previous_ffprobe = std::env::var_os("AUTOLIVE_FFPROBE_PATH");
    std::env::remove_var("AUTOLIVE_FFMPEG_PATH");
    std::env::remove_var("AUTOLIVE_FFPROBE_PATH");

    let capabilities =
        probe_speech_to_speech_worker_with_resource_dir(&executable, 5_000, &resource_dir)
            .expect("capability probe should succeed");
    assert!(capabilities.available);
    let result = run_speech_to_speech_worker_with_resource_dir(
        &SpeechToSpeechWorkerRequest {
            executable,
            input_json_path: input,
            output_json_path: output,
            timeout_ms: 2_000,
        },
        &CancellationToken::new(),
        &resource_dir,
    )
    .expect("worker task should succeed");
    assert_eq!(result.model, "worker-test");

    let expected = format!(
        "{}\n{}",
        resource_dir.join("binaries/ffmpeg").display(),
        resource_dir.join("binaries/ffprobe").display(),
    );
    assert_eq!(
        std::fs::read_to_string(&captured_environment)
            .expect("worker environment should be captured"),
        expected
    );

    let explicit_ffmpeg = directory.0.join("dev-ffmpeg");
    let explicit_ffprobe = directory.0.join("dev-ffprobe");
    let explicit_output = directory.0.join("explicit-result.json");
    std::env::set_var("AUTOLIVE_FFMPEG_PATH", &explicit_ffmpeg);
    std::env::set_var("AUTOLIVE_FFPROBE_PATH", &explicit_ffprobe);
    run_speech_to_speech_worker_with_resource_dir(
        &SpeechToSpeechWorkerRequest {
            executable: directory.0.join("worker.sh"),
            input_json_path: directory.0.join("input.json"),
            output_json_path: explicit_output,
            timeout_ms: 2_000,
        },
        &CancellationToken::new(),
        &resource_dir,
    )
    .expect("explicit media paths should not block the worker");
    let expected_after_override = match worker_environment_policy(cfg!(debug_assertions)) {
        WorkerEnvironmentPolicy::DevelopmentOverrides => format!(
            "{}\n{}",
            explicit_ffmpeg.display(),
            explicit_ffprobe.display()
        ),
        WorkerEnvironmentPolicy::PackagedOnly => expected,
    };
    assert_eq!(
        std::fs::read_to_string(&captured_environment)
            .expect("explicit worker environment should be captured"),
        expected_after_override
    );

    match previous_ffmpeg {
        Some(value) => std::env::set_var("AUTOLIVE_FFMPEG_PATH", value),
        None => std::env::remove_var("AUTOLIVE_FFMPEG_PATH"),
    }
    match previous_ffprobe {
        Some(value) => std::env::set_var("AUTOLIVE_FFPROBE_PATH", value),
        None => std::env::remove_var("AUTOLIVE_FFPROBE_PATH"),
    }
}
