use autolive_desktop_core::cancellation::CancellationToken;
use autolive_desktop_core::speech_to_speech::SpeechToSpeechContext;
use autolive_desktop_core::speech_to_speech_worker::{
    probe_speech_to_speech_worker, run_speech_to_speech_context_worker,
    run_speech_to_speech_worker, SpeechToSpeechContextWorkerRequest, SpeechToSpeechWorkerError,
    SpeechToSpeechWorkerRequest,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
