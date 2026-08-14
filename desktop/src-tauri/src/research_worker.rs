//! 本地研究分析 Worker 适配器。
//!
//! 研究算法由受控的第三方/实验 Worker 执行，Rust 只负责输入哈希、参数文件、
//! 报告契约、取消/超时、staging 和输出提交。该模块不实现隐形标记、鲁棒性
//! 分析或随机扰动算法，也不把分析结果当作平台规避能力。

use crate::cancellation::CancellationToken;
use crate::errors::FileHashError;
use crate::hashing::hash_file_at_path;
use crate::media_library::{probe_user_selected_mp4, MediaProbeRequestDto};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MIN_TIMEOUT_SECONDS: u64 = 1;
const MAX_TIMEOUT_SECONDS: u64 = 6 * 60 * 60;
pub const RESEARCH_WORKER_ENV: &str = "AUTOLIVE_RESEARCH_WORKER";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchWorkerCapabilities {
    pub available: bool,
    pub executable: Option<String>,
    pub reason: Option<String>,
}

impl ResearchWorkerCapabilities {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            executable: None,
            reason: Some(reason.into()),
        }
    }
}

pub fn configured_research_worker_executable() -> Result<PathBuf, ResearchError> {
    let Some(path) = std::env::var_os(RESEARCH_WORKER_ENV).map(PathBuf::from) else {
        return Err(ResearchError::WorkerNotConfigured);
    };
    validate_research_executable_path(&path)?;
    Ok(path)
}

pub fn configured_research_worker_capabilities() -> ResearchWorkerCapabilities {
    match configured_research_worker_executable() {
        Ok(path) => ResearchWorkerCapabilities {
            available: true,
            executable: Some(path.display().to_string()),
            reason: None,
        },
        Err(error) => ResearchWorkerCapabilities::unavailable(error.to_string()),
    }
}

pub fn validate_research_identifier(field: &'static str, value: &str) -> Result<(), ResearchError> {
    validate_identifier(field, value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchAnalysisRequest {
    pub analysis_id: String,
    pub run_id: String,
    pub input_mp4_path: PathBuf,
    /// 当前阶段实际输入 MP4 的完整 SHA-256。
    pub expected_input_mp4_sha256: String,
    /// 原始源 MP4 第一次进入流水线时记录的完整 SHA-256。
    pub source_mp4_sha256: String,
    pub params_path: PathBuf,
    pub research_executable: PathBuf,
    pub output_report_path: PathBuf,
    pub output_mp4_path: Option<PathBuf>,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResearchReport {
    pub report_version: String,
    pub source_mp4_sha256: String,
    pub input_mp4_sha256: String,
    pub current_mp4_sha256: String,
    pub algorithm_version: String,
    pub random_seed: u64,
    pub content_similarity_percent: f64,
    pub media_robustness_score: f64,
    pub invisible_mark_status: String,
    pub random_perturbation_applied: bool,
    pub content_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResearchResult {
    pub report_path: PathBuf,
    pub output_mp4_path: Option<PathBuf>,
    pub input_mp4_sha256: String,
    pub source_mp4_sha256: String,
    pub current_mp4_sha256: String,
    pub report_version: String,
    pub algorithm_version: String,
    pub random_seed: u64,
    pub content_similarity_percent: f64,
    pub media_robustness_score: f64,
    pub invisible_mark_status: String,
    pub random_perturbation_applied: bool,
    pub content_fingerprint: String,
    pub report_sha256: String,
    pub output_mp4_sha256: Option<String>,
    pub output_mp4_size_bytes: Option<u64>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResearchError {
    Cancelled,
    Timeout { seconds: u64 },
    WorkerNotConfigured,
    InvalidIdentifier { field: &'static str, value: String },
    InvalidExpectedHash { value: String },
    InvalidTimeout { seconds: u64 },
    InvalidExecutable { path: String },
    MissingInput { field: &'static str, path: String },
    InvalidPath { field: &'static str, path: String },
    SpawnFailed { path: String, message: String },
    Failed { code: Option<i32> },
    ReportMissing { path: String },
    ReportInvalid { path: String, message: String },
    InputHashMismatch { expected: String, actual: String },
    ReportSourceHashMismatch { expected: String, actual: String },
    ReportInputHashMismatch { expected: String, actual: String },
    ReportCurrentHashMismatch { expected: String, actual: String },
    OutputMissing { path: String },
    OutputEmpty { path: String },
    OutputConflict { path: String },
    OutputCommitFailed { from: String, to: String },
    OutputUnreadable { path: String, message: String },
    HashFailed { path: String, message: String },
}

impl Display for ResearchError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("研究分析已取消"),
            Self::Timeout { seconds } => write!(f, "研究分析超过超时限制: {seconds} 秒"),
            Self::WorkerNotConfigured => {
                write!(f, "未配置研究分析 Worker（环境变量 {RESEARCH_WORKER_ENV}）")
            }
            Self::InvalidIdentifier { field, value } => write!(f, "{field} 无效: {value}"),
            Self::InvalidExpectedHash { value } => write!(f, "期望的 MP4 SHA-256 无效: {value}"),
            Self::InvalidTimeout { seconds } => write!(f, "研究分析超时范围无效: {seconds} 秒"),
            Self::InvalidExecutable { path } => write!(f, "研究分析 Worker 无效: {path}"),
            Self::MissingInput { field, path } => write!(f, "研究分析输入不存在: {field}={path}"),
            Self::InvalidPath { field, path } => write!(f, "研究分析路径无效: {field}={path}"),
            Self::SpawnFailed { path, message } => write!(f, "启动研究分析失败: {path}; {message}"),
            Self::Failed { code } => write!(f, "研究分析进程异常退出: exit_code={code:?}"),
            Self::ReportMissing { path } => write!(f, "研究分析报告不存在: {path}"),
            Self::ReportInvalid { path, message } => {
                write!(f, "研究分析报告无效: {path}; {message}")
            }
            Self::InputHashMismatch { expected, actual } => {
                write!(
                    f,
                    "研究分析阶段输入 MP4 SHA-256 不一致: expected={expected}, actual={actual}"
                )
            }
            Self::ReportSourceHashMismatch { expected, actual } => {
                write!(
                    f,
                    "研究报告原始源 MP4 SHA-256 不一致: expected={expected}, actual={actual}"
                )
            }
            Self::ReportInputHashMismatch { expected, actual } => {
                write!(
                    f,
                    "研究报告阶段输入 MP4 SHA-256 不一致: expected={expected}, actual={actual}"
                )
            }
            Self::ReportCurrentHashMismatch { expected, actual } => {
                write!(
                    f,
                    "研究报告当前 MP4 SHA-256 不一致: expected={expected}, actual={actual}"
                )
            }
            Self::OutputMissing { path } => write!(f, "研究分析未生成输出: {path}"),
            Self::OutputEmpty { path } => write!(f, "研究分析输出为空: {path}"),
            Self::OutputConflict { path } => write!(f, "研究分析输出已存在，拒绝覆盖: {path}"),
            Self::OutputCommitFailed { from, to } => {
                write!(f, "研究分析输出提交失败: {from} -> {to}")
            }
            Self::OutputUnreadable { path, message } => {
                write!(f, "研究分析输出 MP4 不可读: {path}; {message}")
            }
            Self::HashFailed { path, message } => {
                write!(f, "计算研究产物 SHA-256 失败: {path}; {message}")
            }
        }
    }
}

impl Error for ResearchError {}

pub fn validate_research_executable_path(path: &Path) -> Result<(), ResearchError> {
    let metadata = fs::metadata(path).map_err(|_| ResearchError::InvalidExecutable {
        path: path.display().to_string(),
    })?;
    if metadata.is_file() {
        Ok(())
    } else {
        Err(ResearchError::InvalidExecutable {
            path: path.display().to_string(),
        })
    }
}

pub fn build_research_args(
    request: &ResearchAnalysisRequest,
) -> Result<Vec<std::ffi::OsString>, ResearchError> {
    validate_request(request)?;
    let mut args = vec![
        "--input-mp4".into(),
        request.input_mp4_path.clone().into_os_string(),
        "--params".into(),
        request.params_path.clone().into_os_string(),
        "--output-report".into(),
        request.output_report_path.clone().into_os_string(),
    ];
    if let Some(output_mp4_path) = &request.output_mp4_path {
        args.push("--output-mp4".into());
        args.push(output_mp4_path.clone().into_os_string());
    }
    Ok(args)
}

pub fn run_research(
    request: &ResearchAnalysisRequest,
    cancellation: &CancellationToken,
) -> Result<ResearchResult, ResearchError> {
    check_cancelled(cancellation)?;
    validate_request(request)?;
    let input_hash = hash_file_at_path(&request.input_mp4_path, cancellation)
        .map_err(|error| map_hash_error(&request.input_mp4_path, error))?;
    if input_hash != request.expected_input_mp4_sha256 {
        return Err(ResearchError::InputHashMismatch {
            expected: request.expected_input_mp4_sha256.clone(),
            actual: input_hash,
        });
    }

    let report_partial = partial_path(&request.output_report_path, "json");
    let output_partial = request
        .output_mp4_path
        .as_ref()
        .map(|path| partial_path(path, "mp4"));
    let mut child = Command::new(&request.research_executable);
    child
        .arg("--input-mp4")
        .arg(&request.input_mp4_path)
        .arg("--params")
        .arg(&request.params_path)
        .arg("--output-report")
        .arg(&report_partial);
    if let Some(output_partial) = &output_partial {
        child.arg("--output-mp4").arg(output_partial);
    }
    let mut child = child
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| ResearchError::SpawnFailed {
            path: request.research_executable.display().to_string(),
            message: error.to_string(),
        })?;

    let started_at = Instant::now();
    loop {
        if cancellation.is_cancelled() {
            terminate_child(&mut child);
            cleanup(&report_partial, output_partial.as_deref());
            return Err(ResearchError::Cancelled);
        }
        if started_at.elapsed() >= Duration::from_secs(request.timeout_seconds) {
            terminate_child(&mut child);
            cleanup(&report_partial, output_partial.as_deref());
            return Err(ResearchError::Timeout {
                seconds: request.timeout_seconds,
            });
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(status)) => {
                cleanup(&report_partial, output_partial.as_deref());
                return Err(ResearchError::Failed {
                    code: status.code(),
                });
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                terminate_child(&mut child);
                cleanup(&report_partial, output_partial.as_deref());
                return Err(ResearchError::SpawnFailed {
                    path: request.research_executable.display().to_string(),
                    message: error.to_string(),
                });
            }
        }
    }

    if let Err(error) = check_cancelled(cancellation) {
        cleanup(&report_partial, output_partial.as_deref());
        return Err(error);
    }
    let report = match read_report(&report_partial) {
        Ok(report) => report,
        Err(error) => {
            cleanup(&report_partial, output_partial.as_deref());
            return Err(error);
        }
    };
    let (output_hash, output_size_bytes) = if let Some(output_partial_path) = &output_partial {
        let metadata = match fs::metadata(output_partial_path) {
            Ok(metadata) => metadata,
            Err(_) => {
                cleanup(&report_partial, output_partial.as_deref());
                return Err(ResearchError::OutputMissing {
                    path: output_partial_path.display().to_string(),
                });
            }
        };
        if metadata.len() == 0 {
            cleanup(&report_partial, output_partial.as_deref());
            return Err(ResearchError::OutputEmpty {
                path: output_partial_path.display().to_string(),
            });
        }
        if let Err(error) = probe_user_selected_mp4(
            &MediaProbeRequestDto {
                path: output_partial_path.display().to_string(),
            },
            &CancellationToken::new(),
        ) {
            cleanup(&report_partial, output_partial.as_deref());
            return Err(ResearchError::OutputUnreadable {
                path: output_partial_path.display().to_string(),
                message: error.to_string(),
            });
        }
        match hash_file_at_path(output_partial_path, cancellation) {
            Ok(hash) => (Some(hash), Some(metadata.len())),
            Err(error) => {
                cleanup(&report_partial, output_partial.as_deref());
                return Err(map_hash_error(output_partial_path, error));
            }
        }
    } else {
        (None, None)
    };
    if let Err(error) = validate_research_report(&report, request, output_hash.as_deref()) {
        cleanup(&report_partial, output_partial.as_deref());
        return Err(error);
    }
    if let Err(error) = commit_file(&report_partial, &request.output_report_path) {
        cleanup(&report_partial, output_partial.as_deref());
        return Err(error);
    }
    if let (Some(output_partial), Some(output_path)) = (&output_partial, &request.output_mp4_path) {
        if let Err(error) = commit_file(output_partial, output_path) {
            let _ignored = fs::remove_file(&request.output_report_path);
            cleanup(output_partial, None);
            return Err(error);
        }
    }
    // 报告和可选输出已经提交；提交点之后不能再用用户取消令牌打断收尾，
    // 否则调用方会收到 Cancelled，但磁盘上已经存在有效产物。
    let commit_cancellation = CancellationToken::new();
    let report_hash = hash_file_at_path(&request.output_report_path, &commit_cancellation)
        .map_err(|error| map_hash_error(&request.output_report_path, error))?;
    let size_bytes = fs::metadata(&request.output_report_path)
        .map_err(|_| ResearchError::OutputMissing {
            path: request.output_report_path.display().to_string(),
        })?
        .len();
    Ok(ResearchResult {
        report_path: request.output_report_path.clone(),
        output_mp4_path: request.output_mp4_path.clone(),
        input_mp4_sha256: request.expected_input_mp4_sha256.clone(),
        source_mp4_sha256: request.source_mp4_sha256.clone(),
        current_mp4_sha256: report.current_mp4_sha256,
        report_version: report.report_version,
        algorithm_version: report.algorithm_version,
        random_seed: report.random_seed,
        content_similarity_percent: report.content_similarity_percent,
        media_robustness_score: report.media_robustness_score,
        invisible_mark_status: report.invisible_mark_status,
        random_perturbation_applied: report.random_perturbation_applied,
        content_fingerprint: report.content_fingerprint,
        report_sha256: report_hash,
        output_mp4_sha256: output_hash,
        output_mp4_size_bytes: output_size_bytes,
        size_bytes,
    })
}

pub fn validate_research_report(
    report: &ResearchReport,
    request: &ResearchAnalysisRequest,
    output_mp4_sha256: Option<&str>,
) -> Result<(), ResearchError> {
    if report.source_mp4_sha256 != request.source_mp4_sha256 {
        return Err(ResearchError::ReportSourceHashMismatch {
            expected: request.source_mp4_sha256.clone(),
            actual: report.source_mp4_sha256.clone(),
        });
    }
    if report.input_mp4_sha256 != request.expected_input_mp4_sha256 {
        return Err(ResearchError::ReportInputHashMismatch {
            expected: request.expected_input_mp4_sha256.clone(),
            actual: report.input_mp4_sha256.clone(),
        });
    }
    let expected_current_hash = output_mp4_sha256.unwrap_or(&request.expected_input_mp4_sha256);
    if report.current_mp4_sha256 != expected_current_hash {
        return Err(ResearchError::ReportCurrentHashMismatch {
            expected: expected_current_hash.to_owned(),
            actual: report.current_mp4_sha256.clone(),
        });
    }
    if report.report_version.trim().is_empty()
        || report.algorithm_version.trim().is_empty()
        || report.invisible_mark_status.trim().is_empty()
        || report.content_fingerprint.trim().is_empty()
        || !report.content_similarity_percent.is_finite()
        || !(0.0..=100.0).contains(&report.content_similarity_percent)
        || !report.media_robustness_score.is_finite()
        || !(0.0..=100.0).contains(&report.media_robustness_score)
    {
        return Err(ResearchError::ReportInvalid {
            path: request.output_report_path.display().to_string(),
            message: String::from("报告字段为空或数值超出 0–100 范围"),
        });
    }
    Ok(())
}

fn validate_request(request: &ResearchAnalysisRequest) -> Result<(), ResearchError> {
    validate_identifier("analysis_id", &request.analysis_id)?;
    validate_identifier("run_id", &request.run_id)?;
    if !is_sha256(&request.expected_input_mp4_sha256) || !is_sha256(&request.source_mp4_sha256) {
        return Err(ResearchError::InvalidExpectedHash {
            value: format!(
                "input={}, source={}",
                request.expected_input_mp4_sha256, request.source_mp4_sha256
            ),
        });
    }
    if !(MIN_TIMEOUT_SECONDS..=MAX_TIMEOUT_SECONDS).contains(&request.timeout_seconds) {
        return Err(ResearchError::InvalidTimeout {
            seconds: request.timeout_seconds,
        });
    }
    validate_research_executable_path(&request.research_executable)?;
    validate_input_file("input_mp4_path", &request.input_mp4_path, Some("mp4"))?;
    validate_input_file("params_path", &request.params_path, None)?;
    validate_output_path("output_report_path", &request.output_report_path, "json")?;
    if let Some(path) = &request.output_mp4_path {
        validate_output_path("output_mp4_path", path, "mp4")?;
    }
    Ok(())
}

fn validate_input_file(
    field: &'static str,
    path: &Path,
    extension: Option<&str>,
) -> Result<(), ResearchError> {
    let metadata = fs::metadata(path).map_err(|_| ResearchError::MissingInput {
        field,
        path: path.display().to_string(),
    })?;
    if !metadata.is_file()
        || extension.is_some_and(|value| {
            path.extension()
                .and_then(|item| item.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
                != Some(value)
        })
    {
        return Err(ResearchError::InvalidPath {
            field,
            path: path.display().to_string(),
        });
    }
    Ok(())
}

fn validate_output_path(
    field: &'static str,
    path: &Path,
    extension: &str,
) -> Result<(), ResearchError> {
    if path
        .extension()
        .and_then(|item| item.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
        != Some(extension)
    {
        return Err(ResearchError::InvalidPath {
            field,
            path: path.display().to_string(),
        });
    }
    if path.exists() {
        return Err(ResearchError::OutputConflict {
            path: path.display().to_string(),
        });
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| ResearchError::InvalidPath {
            field,
            path: parent.display().to_string(),
        })?;
    }
    Ok(())
}

fn read_report(path: &Path) -> Result<ResearchReport, ResearchError> {
    let bytes = fs::read(path).map_err(|_| ResearchError::ReportMissing {
        path: path.display().to_string(),
    })?;
    serde_json::from_slice(&bytes).map_err(|error| ResearchError::ReportInvalid {
        path: path.display().to_string(),
        message: error.to_string(),
    })
}

fn commit_file(from: &Path, to: &Path) -> Result<(), ResearchError> {
    if to.exists() {
        cleanup(from, None);
        return Err(ResearchError::OutputConflict {
            path: to.display().to_string(),
        });
    }
    fs::rename(from, to).map_err(|_| ResearchError::OutputCommitFailed {
        from: from.display().to_string(),
        to: to.display().to_string(),
    })
}

fn partial_path(path: &Path, extension: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("research");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.as_nanos());
    parent.join(format!(
        ".{stem}.{}.{}.partial.{extension}",
        std::process::id(),
        timestamp
    ))
}

fn cleanup(report_path: &Path, output_path: Option<&Path>) {
    let _ignored = fs::remove_file(report_path);
    if let Some(output_path) = output_path {
        let _ignored = fs::remove_file(output_path);
    }
}

fn terminate_child(child: &mut std::process::Child) {
    let _ignored = child.kill();
    let _ignored = child.wait();
}

fn map_hash_error(path: &Path, error: FileHashError) -> ResearchError {
    match error {
        FileHashError::Cancelled => ResearchError::Cancelled,
        other => ResearchError::HashFailed {
            path: path.display().to_string(),
            message: other.to_string(),
        },
    }
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), ResearchError> {
    if cancellation.is_cancelled() {
        Err(ResearchError::Cancelled)
    } else {
        Ok(())
    }
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), ResearchError> {
    if (8..=64).contains(&value.len())
        && value
            .chars()
            .next()
            .is_some_and(|item| item.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|item| item.is_ascii_alphanumeric() || matches!(item, '_' | '-'))
    {
        return Ok(());
    }
    Err(ResearchError::InvalidIdentifier {
        field,
        value: value.to_owned(),
    })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|item| item.is_ascii_hexdigit())
}
