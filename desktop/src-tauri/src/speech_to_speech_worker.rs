use crate::background_process::background_command;
use crate::bounded_io::{read_to_end_bounded, BoundedReadError};
use crate::cancellation::CancellationToken;
use crate::media_engine::{packaged_media_engine_paths, FFMPEG_PATH_ENV, FFPROBE_PATH_ENV};
use crate::speech_to_speech::{
    SpeechToSpeechContext, SpeechToSpeechResult, SpeechToSpeechWorkerCapabilities,
};
use std::fmt;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

pub const SPEECH_TO_SPEECH_WORKER_ENV: &str = "AUTOLIVE_SPEECH_TO_SPEECH_WORKER";
pub const SPEECH_TO_SPEECH_WORKER_TIMEOUT_MS: u64 = 2_000;
const MAX_SPEECH_TO_SPEECH_WORKER_TIMEOUT_MS: u64 = 120_000;
const MAX_WORKER_JSON_BYTES: usize = 1024 * 1024;

static CAPABILITY_PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerEnvironmentPolicy {
    DevelopmentOverrides,
    PackagedOnly,
}

pub fn worker_environment_policy(debug_assertions: bool) -> WorkerEnvironmentPolicy {
    if debug_assertions {
        WorkerEnvironmentPolicy::DevelopmentOverrides
    } else {
        WorkerEnvironmentPolicy::PackagedOnly
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechToSpeechWorkerRequest {
    pub executable: PathBuf,
    pub input_json_path: PathBuf,
    pub output_json_path: PathBuf,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechToSpeechContextWorkerRequest {
    pub executable: PathBuf,
    pub context_json_path: PathBuf,
    pub output_json_path: PathBuf,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeechToSpeechWorkerError {
    Cancelled,
    Timeout { timeout_ms: u64 },
    InvalidExecutable,
    InvalidInput,
    InvalidOutputPath,
    WorkerNotConfigured,
    InvalidContext,
    SpawnFailed,
    Failed,
    OutputMissing,
    OutputInvalid,
    OutputConflict,
    ContractInvalid,
    CapabilityOutputMissing,
    CapabilityOutputInvalid,
}

impl fmt::Display for SpeechToSpeechWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("speech-to-speech Worker 已取消"),
            Self::Timeout { timeout_ms } => {
                write!(formatter, "speech-to-speech Worker 超时：{timeout_ms}ms")
            }
            Self::InvalidExecutable => {
                formatter.write_str("speech-to-speech Worker 可执行文件无效")
            }
            Self::InvalidInput => formatter.write_str("speech-to-speech Worker 输入 JSON 不存在"),
            Self::InvalidOutputPath => {
                formatter.write_str("speech-to-speech Worker 输出路径必须是 JSON 文件")
            }
            Self::WorkerNotConfigured => {
                formatter.write_str("未配置 speech-to-speech Worker 可执行文件")
            }
            Self::InvalidContext => formatter.write_str("speech-to-speech Worker 上下文无效"),
            Self::SpawnFailed => formatter.write_str("speech-to-speech Worker 启动失败"),
            Self::Failed => formatter.write_str("speech-to-speech Worker 异常退出"),
            Self::OutputMissing => formatter.write_str("speech-to-speech Worker 未生成结果文件"),
            Self::OutputInvalid => formatter.write_str("speech-to-speech Worker 结果文件不可读"),
            Self::OutputConflict => {
                formatter.write_str("speech-to-speech Worker 结果文件已存在，拒绝覆盖")
            }
            Self::ContractInvalid => formatter.write_str("speech-to-speech Worker 输出不符合契约"),
            Self::CapabilityOutputMissing => {
                formatter.write_str("speech-to-speech Worker 未生成能力文件")
            }
            Self::CapabilityOutputInvalid => {
                formatter.write_str("speech-to-speech Worker 能力文件不可读")
            }
        }
    }
}

impl std::error::Error for SpeechToSpeechWorkerError {}

pub fn configured_speech_to_speech_worker_capabilities() -> SpeechToSpeechWorkerCapabilities {
    configured_speech_to_speech_worker_capabilities_for_resource_dir(None)
}

pub fn configured_speech_to_speech_worker_capabilities_with_resource_dir(
    resource_dir: &Path,
) -> SpeechToSpeechWorkerCapabilities {
    configured_speech_to_speech_worker_capabilities_for_resource_dir(Some(resource_dir))
}

fn configured_speech_to_speech_worker_capabilities_for_resource_dir(
    resource_dir: Option<&Path>,
) -> SpeechToSpeechWorkerCapabilities {
    let executable = match configured_worker_executable() {
        Ok(executable) => executable,
        Err(error) => {
            return SpeechToSpeechWorkerCapabilities::unavailable_with_reason(error.to_string())
        }
    };
    match probe_speech_to_speech_worker_for_resource_dir(
        &executable,
        SPEECH_TO_SPEECH_WORKER_TIMEOUT_MS,
        resource_dir,
    ) {
        Ok(capabilities) => capabilities,
        Err(error) => SpeechToSpeechWorkerCapabilities::unavailable_with_reason(format!(
            "本地 Worker 能力探测失败：{error}"
        )),
    }
}

pub fn configured_worker_executable() -> Result<PathBuf, SpeechToSpeechWorkerError> {
    if worker_environment_policy(cfg!(debug_assertions)) == WorkerEnvironmentPolicy::PackagedOnly {
        return Err(SpeechToSpeechWorkerError::WorkerNotConfigured);
    }
    let Some(executable) = std::env::var_os(SPEECH_TO_SPEECH_WORKER_ENV).map(PathBuf::from) else {
        return Err(SpeechToSpeechWorkerError::WorkerNotConfigured);
    };
    if !executable.is_file() {
        return Err(SpeechToSpeechWorkerError::InvalidExecutable);
    }
    Ok(executable)
}

pub fn probe_speech_to_speech_worker(
    executable: &Path,
    timeout_ms: u64,
) -> Result<SpeechToSpeechWorkerCapabilities, SpeechToSpeechWorkerError> {
    probe_speech_to_speech_worker_for_resource_dir(executable, timeout_ms, None)
}

pub fn probe_speech_to_speech_worker_with_resource_dir(
    executable: &Path,
    timeout_ms: u64,
    resource_dir: &Path,
) -> Result<SpeechToSpeechWorkerCapabilities, SpeechToSpeechWorkerError> {
    probe_speech_to_speech_worker_for_resource_dir(executable, timeout_ms, Some(resource_dir))
}

fn probe_speech_to_speech_worker_for_resource_dir(
    executable: &Path,
    timeout_ms: u64,
    resource_dir: Option<&Path>,
) -> Result<SpeechToSpeechWorkerCapabilities, SpeechToSpeechWorkerError> {
    if !executable.is_file() {
        return Err(SpeechToSpeechWorkerError::InvalidExecutable);
    }
    if timeout_ms == 0 {
        return Err(SpeechToSpeechWorkerError::Timeout { timeout_ms: 0 });
    }
    let capability_output = capability_output_path();
    cleanup(&capability_output);
    let mut command = background_command(executable);
    command
        .arg("--capabilities-json")
        .arg(&capability_output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    inject_packaged_media_engine_paths(&mut command, resource_dir);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|_| SpeechToSpeechWorkerError::SpawnFailed)?;

    let started = Instant::now();
    loop {
        if started.elapsed() >= Duration::from_millis(timeout_ms) {
            terminate_child(&mut child);
            cleanup(&capability_output);
            return Err(SpeechToSpeechWorkerError::Timeout { timeout_ms });
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => {
                cleanup(&capability_output);
                return Err(SpeechToSpeechWorkerError::Failed);
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                terminate_child(&mut child);
                cleanup(&capability_output);
                return Err(SpeechToSpeechWorkerError::Failed);
            }
        }
    }

    let content = read_worker_json(&capability_output);
    cleanup(&capability_output);
    let content = match content {
        Ok(content) => content,
        Err(BoundedReadError::Io(_)) => {
            return Err(SpeechToSpeechWorkerError::CapabilityOutputMissing)
        }
        Err(BoundedReadError::LimitExceeded { .. }) => {
            return Err(SpeechToSpeechWorkerError::CapabilityOutputInvalid)
        }
    };
    let capabilities = serde_json::from_slice::<SpeechToSpeechWorkerCapabilities>(&content)
        .map_err(|_| SpeechToSpeechWorkerError::CapabilityOutputInvalid)?;
    capabilities
        .validate()
        .map_err(|_| SpeechToSpeechWorkerError::ContractInvalid)?;
    Ok(capabilities)
}

pub fn run_speech_to_speech_worker(
    request: &SpeechToSpeechWorkerRequest,
    cancellation: &CancellationToken,
) -> Result<SpeechToSpeechResult, SpeechToSpeechWorkerError> {
    run_speech_to_speech_worker_for_resource_dir(request, cancellation, None)
}

pub fn run_speech_to_speech_worker_with_resource_dir(
    request: &SpeechToSpeechWorkerRequest,
    cancellation: &CancellationToken,
    resource_dir: &Path,
) -> Result<SpeechToSpeechResult, SpeechToSpeechWorkerError> {
    run_speech_to_speech_worker_for_resource_dir(request, cancellation, Some(resource_dir))
}

fn run_speech_to_speech_worker_for_resource_dir(
    request: &SpeechToSpeechWorkerRequest,
    cancellation: &CancellationToken,
    resource_dir: Option<&Path>,
) -> Result<SpeechToSpeechResult, SpeechToSpeechWorkerError> {
    validate_request(request)?;
    if cancellation.is_cancelled() {
        return Err(SpeechToSpeechWorkerError::Cancelled);
    }

    let output_partial = partial_path(&request.output_json_path);
    let _ = fs::remove_file(&output_partial);
    let mut command = background_command(&request.executable);
    command
        .arg("--input-json")
        .arg(&request.input_json_path)
        .arg("--output-json")
        .arg(&output_partial)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    inject_packaged_media_engine_paths(&mut command, resource_dir);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|_| SpeechToSpeechWorkerError::SpawnFailed)?;

    let started = Instant::now();
    loop {
        if cancellation.is_cancelled() {
            terminate_child(&mut child);
            cleanup(&output_partial);
            return Err(SpeechToSpeechWorkerError::Cancelled);
        }
        if started.elapsed() >= Duration::from_millis(request.timeout_ms) {
            terminate_child(&mut child);
            cleanup(&output_partial);
            return Err(SpeechToSpeechWorkerError::Timeout {
                timeout_ms: request.timeout_ms,
            });
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => {
                cleanup(&output_partial);
                return Err(SpeechToSpeechWorkerError::Failed);
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                terminate_child(&mut child);
                cleanup(&output_partial);
                return Err(SpeechToSpeechWorkerError::Failed);
            }
        }
    }

    let result = read_result(&output_partial)?;
    result.validate().map_err(|_| {
        cleanup(&output_partial);
        SpeechToSpeechWorkerError::ContractInvalid
    })?;
    if request.output_json_path.exists() {
        cleanup(&output_partial);
        return Err(SpeechToSpeechWorkerError::OutputConflict);
    }
    fs::rename(&output_partial, &request.output_json_path).map_err(|_| {
        cleanup(&output_partial);
        SpeechToSpeechWorkerError::OutputInvalid
    })?;
    Ok(result)
}

pub fn run_speech_to_speech_context_worker(
    request: &SpeechToSpeechContextWorkerRequest,
    context: &SpeechToSpeechContext,
    cancellation: &CancellationToken,
) -> Result<SpeechToSpeechResult, SpeechToSpeechWorkerError> {
    run_speech_to_speech_context_worker_for_resource_dir(request, context, cancellation, None)
}

fn run_speech_to_speech_context_worker_for_resource_dir(
    request: &SpeechToSpeechContextWorkerRequest,
    context: &SpeechToSpeechContext,
    cancellation: &CancellationToken,
    resource_dir: Option<&Path>,
) -> Result<SpeechToSpeechResult, SpeechToSpeechWorkerError> {
    context
        .validate_for_worker()
        .map_err(|_| SpeechToSpeechWorkerError::InvalidContext)?;
    if request
        .context_json_path
        .extension()
        .and_then(|value| value.to_str())
        != Some("json")
    {
        return Err(SpeechToSpeechWorkerError::InvalidInput);
    }
    let content =
        serde_json::to_vec(context).map_err(|_| SpeechToSpeechWorkerError::InvalidInput)?;
    if request.context_json_path.exists() {
        return Err(SpeechToSpeechWorkerError::OutputConflict);
    }
    fs::write(&request.context_json_path, content)
        .map_err(|_| SpeechToSpeechWorkerError::InvalidInput)?;
    let result = run_speech_to_speech_worker_for_resource_dir(
        &SpeechToSpeechWorkerRequest {
            executable: request.executable.clone(),
            input_json_path: request.context_json_path.clone(),
            output_json_path: request.output_json_path.clone(),
            timeout_ms: request.timeout_ms,
        },
        cancellation,
        resource_dir,
    );
    cleanup(&request.context_json_path);
    result
}

pub fn run_configured_speech_to_speech_context_worker(
    context: &SpeechToSpeechContext,
    cancellation: &CancellationToken,
) -> Result<SpeechToSpeechResult, SpeechToSpeechWorkerError> {
    run_configured_speech_to_speech_context_worker_for_resource_dir(context, cancellation, None)
}

pub fn run_configured_speech_to_speech_context_worker_with_resource_dir(
    context: &SpeechToSpeechContext,
    cancellation: &CancellationToken,
    resource_dir: &Path,
) -> Result<SpeechToSpeechResult, SpeechToSpeechWorkerError> {
    run_configured_speech_to_speech_context_worker_for_resource_dir(
        context,
        cancellation,
        Some(resource_dir),
    )
}

fn run_configured_speech_to_speech_context_worker_for_resource_dir(
    context: &SpeechToSpeechContext,
    cancellation: &CancellationToken,
    resource_dir: Option<&Path>,
) -> Result<SpeechToSpeechResult, SpeechToSpeechWorkerError> {
    if context.timeout_ms > MAX_SPEECH_TO_SPEECH_WORKER_TIMEOUT_MS {
        return Err(SpeechToSpeechWorkerError::Timeout {
            timeout_ms: context.timeout_ms,
        });
    }
    let executable = configured_worker_executable()?;
    let job_directory = worker_job_directory();
    fs::create_dir(&job_directory).map_err(|_| SpeechToSpeechWorkerError::InvalidInput)?;
    let request = SpeechToSpeechContextWorkerRequest {
        executable,
        context_json_path: job_directory.join("context.json"),
        output_json_path: job_directory.join("result.json"),
        timeout_ms: context.timeout_ms,
    };
    let result = run_speech_to_speech_context_worker_for_resource_dir(
        &request,
        context,
        cancellation,
        resource_dir,
    );
    // 候选音频必须由 Worker 写入 job 目录之外的受控持久化位置。
    // 这样任务结束、取消或超时时，临时上下文和结果不会泄漏；候选文件的生命周期
    // 由后续候选提交/媒体播放层管理。
    let _ignored = fs::remove_dir_all(job_directory);
    result
}

fn validate_request(
    request: &SpeechToSpeechWorkerRequest,
) -> Result<(), SpeechToSpeechWorkerError> {
    if !request.executable.is_file() {
        return Err(SpeechToSpeechWorkerError::InvalidExecutable);
    }
    if !request.input_json_path.is_file() {
        return Err(SpeechToSpeechWorkerError::InvalidInput);
    }
    if request.output_json_path.as_os_str().is_empty()
        || request
            .output_json_path
            .extension()
            .and_then(|value| value.to_str())
            != Some("json")
    {
        return Err(SpeechToSpeechWorkerError::InvalidOutputPath);
    }
    if request.timeout_ms == 0 {
        return Err(SpeechToSpeechWorkerError::Timeout { timeout_ms: 0 });
    }
    Ok(())
}

fn partial_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".partial");
    PathBuf::from(value)
}

fn capability_output_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "autolive-speech-worker-capabilities-{}-{}.json.partial",
        std::process::id(),
        CAPABILITY_PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn worker_job_directory() -> PathBuf {
    std::env::temp_dir().join(format!(
        "autolive-speech-worker-job-{}-{}",
        std::process::id(),
        CAPABILITY_PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn read_result(path: &Path) -> Result<SpeechToSpeechResult, SpeechToSpeechWorkerError> {
    if !path.is_file() {
        return Err(SpeechToSpeechWorkerError::OutputMissing);
    }
    let content = read_worker_json(path).map_err(|_| SpeechToSpeechWorkerError::OutputInvalid)?;
    serde_json::from_slice(&content).map_err(|_| SpeechToSpeechWorkerError::OutputInvalid)
}

fn read_worker_json(path: &Path) -> Result<Vec<u8>, BoundedReadError> {
    let file = File::open(path).map_err(BoundedReadError::Io)?;
    read_to_end_bounded(file, MAX_WORKER_JSON_BYTES)
}

fn cleanup(path: &Path) {
    let _ = fs::remove_file(path);
}

fn inject_packaged_media_engine_paths(command: &mut Command, resource_dir: Option<&Path>) {
    let policy = worker_environment_policy(cfg!(debug_assertions));
    if policy == WorkerEnvironmentPolicy::PackagedOnly {
        command
            .env_remove(SPEECH_TO_SPEECH_WORKER_ENV)
            .env_remove(FFMPEG_PATH_ENV)
            .env_remove(FFPROBE_PATH_ENV);
    }
    let Some(resource_dir) = resource_dir else {
        return;
    };
    let Ok((ffmpeg_path, ffprobe_path)) = packaged_media_engine_paths(resource_dir) else {
        return;
    };
    if (policy == WorkerEnvironmentPolicy::PackagedOnly
        || std::env::var_os(FFMPEG_PATH_ENV).is_none())
        && ffmpeg_path.is_file()
    {
        command.env(FFMPEG_PATH_ENV, ffmpeg_path);
    }
    if (policy == WorkerEnvironmentPolicy::PackagedOnly
        || std::env::var_os(FFPROBE_PATH_ENV).is_none())
        && ffprobe_path.is_file()
    {
        command.env(FFPROBE_PATH_ENV, ffprobe_path);
    }
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

#[cfg(test)]
mod tests {
    use super::{read_result, SpeechToSpeechWorkerError, MAX_WORKER_JSON_BYTES};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn worker_result_rejects_json_above_one_mibibyte() {
        let path = std::env::temp_dir().join(format!(
            "autolive-speech-result-limit-{}-{}.json",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, vec![b' '; MAX_WORKER_JSON_BYTES + 1])
            .expect("oversized worker fixture should be written");

        let result = read_result(&path);
        let _ = fs::remove_file(path);

        assert_eq!(result, Err(SpeechToSpeechWorkerError::OutputInvalid));
    }
}
