use autolive_desktop_core::research_worker::{
    build_research_args, run_research, validate_research_report, ResearchAnalysisRequest,
    ResearchError, ResearchReport,
};
use sha2::Digest;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "autolive-research-contract-{}-{}-{}",
            std::process::id(),
            TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos()
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

fn request() -> ResearchAnalysisRequest {
    ResearchAnalysisRequest {
        analysis_id: String::from("analysis_abcd1234"),
        run_id: String::from("run_0001"),
        input_mp4_path: PathBuf::from("/tmp/source.mp4"),
        expected_input_mp4_sha256: String::from(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        source_mp4_sha256: String::from(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        params_path: PathBuf::from("/tmp/params.json"),
        research_executable: PathBuf::from("/tmp/research-worker"),
        output_report_path: PathBuf::from("/tmp/report.json"),
        output_mp4_path: Some(PathBuf::from("/tmp/research.mp4")),
        timeout_seconds: 60,
    }
}

fn report() -> ResearchReport {
    ResearchReport {
        report_version: String::from("research_report_v1"),
        source_mp4_sha256: String::from(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        input_mp4_sha256: String::from(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        current_mp4_sha256: String::from(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ),
        algorithm_version: String::from("worker-test-v1"),
        random_seed: 42,
        content_similarity_percent: 96.0,
        media_robustness_score: 82.5,
        invisible_mark_status: String::from("analyzed"),
        random_perturbation_applied: true,
        content_fingerprint: String::from("sha256:test-fingerprint"),
    }
}

#[test]
fn research_command_uses_argument_array_and_optional_output_mp4() {
    let directory = TestDir::new();
    let mut input = request();
    input.input_mp4_path = directory.0.join("source.mp4");
    input.params_path = directory.0.join("params.json");
    input.research_executable = PathBuf::from("/bin/sh");
    input.output_report_path = directory.0.join("report.json");
    input.output_mp4_path = Some(directory.0.join("research.mp4"));
    std::fs::write(&input.input_mp4_path, b"mp4 fixture").expect("input should be written");
    std::fs::write(&input.params_path, b"{}").expect("params should be written");

    let args = build_research_args(&input).expect("valid research request should build args");
    let rendered: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    let input_path = input.input_mp4_path.display().to_string();
    let output_path = input
        .output_mp4_path
        .as_ref()
        .expect("output should be present")
        .display()
        .to_string();

    assert!(rendered
        .windows(2)
        .any(|pair| pair[0] == "--input-mp4" && pair[1] == input_path));
    assert!(rendered
        .windows(2)
        .any(|pair| pair[0] == "--output-mp4" && pair[1] == output_path));
    assert!(!rendered.iter().any(|value| value.contains(";")));
}

#[test]
fn research_report_rejects_source_hash_mismatch() {
    let mut invalid = report();
    invalid.source_mp4_sha256 =
        String::from("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");

    assert!(matches!(
        validate_research_report(&invalid, &request(), Some(&invalid.current_mp4_sha256)),
        Err(ResearchError::ReportSourceHashMismatch { .. })
    ));
}

#[test]
fn research_report_requires_current_hash_to_match_committed_output() {
    let invalid_output_hash =
        String::from("dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd");

    assert!(matches!(
        validate_research_report(&report(), &request(), Some(&invalid_output_hash)),
        Err(ResearchError::ReportCurrentHashMismatch { .. })
    ));
}

#[test]
fn research_report_separates_original_and_stage_input_hashes() {
    let mut request = request();
    request.source_mp4_sha256 =
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_owned();
    let report = report();

    assert!(matches!(
        validate_research_report(&report, &request, None),
        Err(ResearchError::ReportSourceHashMismatch { .. })
    ));
}

#[test]
fn research_request_identifiers_reject_path_separators() {
    let directory = TestDir::new();
    let mut input = request();
    input.input_mp4_path = directory.0.join("source.mp4");
    input.params_path = directory.0.join("params.json");
    input.output_report_path = directory.0.join("report.json");
    input.output_mp4_path = None;
    input.research_executable = directory.0.join("worker");
    std::fs::write(&input.input_mp4_path, b"mp4 fixture").expect("input should be written");
    std::fs::write(&input.params_path, b"{}").expect("params should be written");
    std::fs::write(&input.research_executable, b"#!/bin/sh\n").expect("worker should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&input.research_executable)
            .expect("worker metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&input.research_executable, permissions)
            .expect("worker should be executable");
    }
    input.analysis_id = "../escape".to_owned();
    input.expected_input_mp4_sha256 = autolive_desktop_core::hashing::hash_file_at_path(
        &input.input_mp4_path,
        &autolive_desktop_core::cancellation::CancellationToken::new(),
    )
    .expect("fixture hash should succeed");
    input.source_mp4_sha256 = input.expected_input_mp4_sha256.clone();

    assert!(matches!(
        autolive_desktop_core::research_worker::validate_research_identifier(
            "analysis_id",
            &input.analysis_id,
        ),
        Err(ResearchError::InvalidIdentifier { .. })
    ));
}

#[cfg(unix)]
#[test]
fn cancelled_research_worker_removes_all_partial_outputs() {
    let directory = TestDir::new();
    let mut input = request();
    input.input_mp4_path = directory.0.join("source.mp4");
    input.params_path = directory.0.join("params.json");
    input.research_executable = directory.0.join("research-worker.sh");
    input.output_report_path = directory.0.join("report.json");
    input.output_mp4_path = Some(directory.0.join("research.mp4"));
    std::fs::write(&input.input_mp4_path, b"mp4 fixture").expect("input should be written");
    std::fs::write(&input.params_path, b"{}").expect("params should be written");
    std::fs::write(
        &input.research_executable,
        "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --output-report|--output-mp4) target=\"$2\"; touch \"$target\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\nsleep 10\n",
    )
    .expect("worker script should be written");
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(&input.research_executable)
        .expect("worker metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&input.research_executable, permissions)
        .expect("worker should be executable");
    input.expected_input_mp4_sha256 = autolive_desktop_core::hashing::hash_file_at_path(
        &input.input_mp4_path,
        &autolive_desktop_core::cancellation::CancellationToken::new(),
    )
    .expect("fixture hash should succeed");
    input.source_mp4_sha256 = input.expected_input_mp4_sha256.clone();
    let token = autolive_desktop_core::cancellation::CancellationToken::new();
    let worker_token = token.clone();
    let worker_input = input.clone();
    let handle = std::thread::spawn(move || run_research(&worker_input, &worker_token));
    std::thread::sleep(std::time::Duration::from_millis(100));
    token.cancel();
    let result = handle.join().expect("worker thread should join");

    assert!(matches!(result, Err(ResearchError::Cancelled)));
    assert!(!directory.0.join("report.json").exists());
    assert!(!directory.0.join("research.mp4").exists());
    assert_eq!(
        std::fs::read_dir(&directory.0)
            .expect("test directory should be readable")
            .count(),
        3,
        "only source, params, and worker script should remain"
    );
}

#[cfg(unix)]
#[test]
fn successful_research_worker_returns_report_metrics() {
    let directory = TestDir::new();
    let mut input = request();
    input.input_mp4_path = directory.0.join("source.mp4");
    input.params_path = directory.0.join("params.json");
    input.research_executable = directory.0.join("research-worker.sh");
    input.output_report_path = directory.0.join("report.json");
    input.output_mp4_path = None;
    std::fs::write(&input.input_mp4_path, b"mp4 fixture").expect("input should be written");
    std::fs::write(&input.params_path, b"{}").expect("params should be written");
    input.expected_input_mp4_sha256 = autolive_desktop_core::hashing::hash_file_at_path(
        &input.input_mp4_path,
        &autolive_desktop_core::cancellation::CancellationToken::new(),
    )
    .expect("fixture hash should succeed");
    input.source_mp4_sha256 = input.expected_input_mp4_sha256.clone();
    std::fs::write(
        &input.research_executable,
        format!(
            "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --output-report) report=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\ncat > \"$report\" <<'JSON'\n{{\"report_version\":\"research_report_v1\",\"source_mp4_sha256\":\"{hash}\",\"input_mp4_sha256\":\"{hash}\",\"current_mp4_sha256\":\"{hash}\",\"algorithm_version\":\"worker-test-v1\",\"random_seed\":42,\"content_similarity_percent\":96.0,\"media_robustness_score\":82.5,\"invisible_mark_status\":\"analyzed\",\"random_perturbation_applied\":true,\"content_fingerprint\":\"sha256:test-fingerprint\"}}\nJSON\n",
            hash = input.expected_input_mp4_sha256
        ),
    )
    .expect("worker script should be written");
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(&input.research_executable)
        .expect("worker metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&input.research_executable, permissions)
        .expect("worker should be executable");

    let result = run_research(
        &input,
        &autolive_desktop_core::cancellation::CancellationToken::new(),
    )
    .expect("successful research worker should return a result");

    assert_eq!(result.report_version, "research_report_v1");
    assert_eq!(result.algorithm_version, "worker-test-v1");
    assert_eq!(result.random_seed, 42);
    assert_eq!(result.content_similarity_percent, 96.0);
    assert_eq!(result.media_robustness_score, 82.5);
    assert_eq!(result.invisible_mark_status, "analyzed");
    assert!(result.random_perturbation_applied);
    assert_eq!(result.content_fingerprint, "sha256:test-fingerprint");
}

#[cfg(unix)]
#[test]
fn research_worker_rejects_unreadable_optional_mp4() {
    let directory = TestDir::new();
    let mut input = request();
    input.input_mp4_path = directory.0.join("source.mp4");
    input.params_path = directory.0.join("params.json");
    input.research_executable = directory.0.join("research-worker.sh");
    input.output_report_path = directory.0.join("report.json");
    input.output_mp4_path = Some(directory.0.join("research.mp4"));
    std::fs::write(&input.input_mp4_path, b"mp4 fixture").expect("input should be written");
    std::fs::write(&input.params_path, b"{}").expect("params should be written");
    input.expected_input_mp4_sha256 = autolive_desktop_core::hashing::hash_file_at_path(
        &input.input_mp4_path,
        &autolive_desktop_core::cancellation::CancellationToken::new(),
    )
    .expect("fixture hash should succeed");
    input.source_mp4_sha256 = input.expected_input_mp4_sha256.clone();
    let mut output_hasher = sha2::Sha256::new();
    output_hasher.update(b"not an mp4");
    let output_hash = output_hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    std::fs::write(
        &input.research_executable,
        format!(
            "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --output-report) report=\"$2\"; shift 2 ;;\n    --output-mp4) output=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\nprintf '%s' 'not an mp4' > \"$output\"\nprintf '%s' '{{\"report_version\":\"research_report_v1\",\"source_mp4_sha256\":\"{source_hash}\",\"input_mp4_sha256\":\"{source_hash}\",\"current_mp4_sha256\":\"{output_hash}\",\"algorithm_version\":\"worker-test-v1\",\"random_seed\":42,\"content_similarity_percent\":96.0,\"media_robustness_score\":82.5,\"invisible_mark_status\":\"analyzed\",\"random_perturbation_applied\":true,\"content_fingerprint\":\"sha256:test-fingerprint\"}}' > \"$report\"\n",
            source_hash = input.expected_input_mp4_sha256,
            output_hash = output_hash,
        ),
    )
    .expect("worker script should be written");
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(&input.research_executable)
        .expect("worker metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&input.research_executable, permissions)
        .expect("worker should be executable");

    let result = run_research(
        &input,
        &autolive_desktop_core::cancellation::CancellationToken::new(),
    );

    assert!(matches!(
        result,
        Err(ResearchError::OutputUnreadable { .. })
    ));
    assert!(!input.output_report_path.exists());
    assert!(!input
        .output_mp4_path
        .as_ref()
        .expect("optional output path")
        .exists());
}

#[cfg(unix)]
#[test]
fn cancellation_after_report_commit_does_not_turn_committed_result_into_failure() {
    let directory = TestDir::new();
    let mut input = request();
    input.input_mp4_path = directory.0.join("source.mp4");
    input.params_path = directory.0.join("params.json");
    input.research_executable = directory.0.join("research-worker.sh");
    input.output_report_path = directory.0.join("report.json");
    input.output_mp4_path = None;
    std::fs::write(&input.input_mp4_path, b"mp4 fixture").expect("input should be written");
    std::fs::write(&input.params_path, b"{}").expect("params should be written");
    input.expected_input_mp4_sha256 = autolive_desktop_core::hashing::hash_file_at_path(
        &input.input_mp4_path,
        &autolive_desktop_core::cancellation::CancellationToken::new(),
    )
    .expect("fixture hash should succeed");
    input.source_mp4_sha256 = input.expected_input_mp4_sha256.clone();
    std::fs::write(
        &input.research_executable,
        format!(
            "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --output-report) report=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\nprintf '%s' '{{\"report_version\":\"research_report_v1\",\"source_mp4_sha256\":\"{hash}\",\"input_mp4_sha256\":\"{hash}\",\"current_mp4_sha256\":\"{hash}\",\"algorithm_version\":\"worker-test-v1\",\"random_seed\":42,\"content_similarity_percent\":96.0,\"media_robustness_score\":82.5,\"invisible_mark_status\":\"analyzed\",\"random_perturbation_applied\":true,\"content_fingerprint\":\"' > \"$report\"\nhead -c 67108864 /dev/zero | tr '\\000' 'a' >> \"$report\"\nprintf '%s' '\"}}' >> \"$report\"\n",
            hash = input.expected_input_mp4_sha256,
        ),
    )
    .expect("worker script should be written");
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(&input.research_executable)
        .expect("worker metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&input.research_executable, permissions)
        .expect("worker should be executable");

    let token = autolive_desktop_core::cancellation::CancellationToken::new();
    let worker_token = token.clone();
    let worker_input = input.clone();
    let handle = std::thread::spawn(move || run_research(&worker_input, &worker_token));
    for _ in 0..5_000 {
        if input.output_report_path.exists() {
            token.cancel();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let result = handle.join().expect("worker thread should join");

    assert!(token.is_cancelled(), "test must cancel after report commit");
    assert!(result.is_ok(), "committed report should finish: {result:?}");
    assert!(input.output_report_path.exists());
}
