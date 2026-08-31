//! mpv/libplacebo 实时画面后端的安全核心边界。
//!
//! 本模块不接触 Tauri 窗口或前端输入。调用方只能传入已经由 Rust 播放池选中的
//! 本地媒体、受信运行资源根目录和宿主窗口句柄；mpv 参数与可写属性均由这里的
//! 固定枚举生成。实际 IPC 连接、窗口句柄获取和播放状态接线由桌面命令层负责。

use std::collections::{BTreeMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[path = "realtime_video_ipc.rs"]
mod realtime_video_ipc;
use realtime_video_ipc::MpvIpcClient;
pub use realtime_video_ipc::{MpvIpcOptions, PendingMpvResponse};

#[path = "realtime_video_job.rs"]
mod realtime_video_job;
use realtime_video_job::ManagedMpvJob;

#[cfg(not(test))]
use crate::background_process::background_command;

const MPV_PATH_ENV: &str = "AUTOLIVE_MPV_PATH";
const MPV_IPC_PREFIX: &str = r"\\.\pipe\autolive-mpv-";
const MPV_EXIT_GRACE: Duration = Duration::from_millis(750);
const MPV_QUIT_RESPONSE_TIMEOUT: Duration = Duration::from_millis(300);
const MPV_EXIT_POLL: Duration = Duration::from_millis(10);
const MPV_MEDIA_SWITCH_POLL: Duration = Duration::from_millis(10);
const STDERR_TAIL_MAX_LINES: usize = 64;
const STDERR_TAIL_MAX_BYTES: usize = 32 * 1024;
const STDERR_LINE_MAX_BYTES: usize = 2 * 1024;
const SHADER_OPTIONS_MAX_BYTES: usize = 32 * 1024;
const SHADER_OPTIONS_MAX_ENTRIES: usize = 256;
const SHADER_OPTION_KEY_MAX_BYTES: usize = 128;
const SHADER_OPTION_VALUE_MAX_BYTES: usize = 256;
const CPU4_MPV_FILTER_LABEL: &str = "autolive_cpu4";
const CPU4_EQ_TARGET: &str = "eq@autolive_cpu4_eq";
const CPU4_HUE_TARGET: &str = "hue@autolive_cpu4_hue";
const CPU4_FILTER_CHAIN: &str = "@autolive_cpu4:lavfi=[eq@autolive_cpu4_eq=brightness=0:contrast=1:saturation=1,hue@autolive_cpu4_hue=h=0:s=1]";

#[cfg(test)]
fn background_command(program: impl AsRef<OsStr>) -> std::process::Command {
    std::process::Command::new(program)
}

#[derive(Debug)]
pub enum RealtimeVideoBackendError {
    InvalidResourceRoot {
        path: PathBuf,
        message: String,
    },
    InvalidExecutable {
        path: PathBuf,
        message: String,
    },
    InvalidShader {
        path: PathBuf,
        message: String,
    },
    ResourceEscapesRoot {
        path: PathBuf,
        root: PathBuf,
    },
    InvalidMediaPath {
        path: PathBuf,
        message: String,
    },
    InvalidHostWindow,
    InvalidIpcPipe,
    SpawnFailed {
        path: PathBuf,
        message: String,
    },
    ProcessFailed {
        operation: &'static str,
        message: String,
    },
    StaleSync {
        field: &'static str,
    },
    SyncSuperseded {
        operation: &'static str,
    },
    InvalidSync {
        message: String,
    },
    StalePrepareBackendEpoch {
        requested: u64,
        current: u64,
    },
    SerializeCommand(String),
    IpcQueueFull,
    IpcTimeout {
        request_id: u64,
        operation: &'static str,
    },
    PropertyUnavailable {
        request_id: u64,
        operation: &'static str,
        property_error: String,
    },
    RuntimeTimeout {
        request_id: u64,
        operation: &'static str,
    },
    MediaSwitchTimeout,
    MediaSwitchCancelled,
    IpcDisconnected(String),
    IpcProtocol(String),
    InvalidCpu4Parameter {
        parameter: &'static str,
        message: &'static str,
    },
}

impl fmt::Display for RealtimeVideoBackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidResourceRoot { path, message } => {
                write!(
                    formatter,
                    "无效的实时视频资源根目录 {}：{message}",
                    path.display()
                )
            }
            Self::InvalidExecutable { path, message } => {
                write!(
                    formatter,
                    "无效的 mpv 可执行文件 {}：{message}",
                    path.display()
                )
            }
            Self::InvalidShader { path, message } => {
                write!(formatter, "无效的 mpv shader {}：{message}", path.display())
            }
            Self::ResourceEscapesRoot { path, root } => write!(
                formatter,
                "mpv 资源路径 {} 不在受信根目录 {} 内",
                path.display(),
                root.display()
            ),
            Self::InvalidMediaPath { path, message } => {
                write!(formatter, "无效的媒体路径 {}：{message}", path.display())
            }
            Self::InvalidHostWindow => formatter.write_str("宿主窗口句柄必须为非零值"),
            Self::InvalidIpcPipe => formatter.write_str("mpv IPC 必须使用受管的 AutoLive 命名管道"),
            Self::SpawnFailed { path, message } => {
                write!(formatter, "无法启动 mpv {}：{message}", path.display())
            }
            Self::ProcessFailed { operation, message } => {
                write!(formatter, "mpv 进程{operation}失败：{message}")
            }
            Self::StaleSync { field } => {
                write!(formatter, "实时画面同步的{field}已过期")
            }
            Self::SyncSuperseded { operation } => {
                write!(formatter, "实时画面{operation}已被更新的同步请求取代")
            }
            Self::InvalidSync { message } => {
                write!(formatter, "实时画面同步请求无效：{message}")
            }
            Self::StalePrepareBackendEpoch { requested, current } => write!(
                formatter,
                "实时画面准备请求的后端 epoch 已过期（请求 {requested}，当前 {current}）"
            ),
            Self::SerializeCommand(message) => {
                write!(formatter, "mpv IPC 命令序列化失败：{message}")
            }
            Self::IpcQueueFull => formatter.write_str("mpv IPC 命令队列已满"),
            Self::IpcTimeout {
                request_id,
                operation,
            } => {
                write!(
                    formatter,
                    "mpv IPC 请求 {request_id}（{operation}）等待响应超时"
                )
            }
            Self::PropertyUnavailable {
                request_id,
                operation,
                property_error,
            } => write!(
                formatter,
                "mpv IPC 请求 {request_id}（{operation}）属性暂不可用：{property_error}"
            ),
            Self::RuntimeTimeout {
                request_id,
                operation,
            } => write!(
                formatter,
                "实时画面运行时请求 {request_id}（{operation}）等待响应超时"
            ),
            Self::MediaSwitchTimeout => formatter.write_str("mpv 同进程换源等待新文件首帧超时"),
            Self::MediaSwitchCancelled => {
                formatter.write_str("mpv 同进程换源已被更新的播放操作取消")
            }
            Self::IpcDisconnected(message) => write!(formatter, "mpv IPC 已断开：{message}"),
            Self::IpcProtocol(message) => write!(formatter, "mpv IPC 协议错误：{message}"),
            Self::InvalidCpu4Parameter { parameter, message } => {
                write!(formatter, "CPU4 参数 {parameter} 无效：{message}")
            }
        }
    }
}

impl std::error::Error for RealtimeVideoBackendError {}

/// 只能通过受信资源解析器取得的 mpv 可执行文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedMpvExecutable(PathBuf);

impl VerifiedMpvExecutable {
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// 只能从受信资源根目录解析的 mpv hook shader。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedMpvShader(PathBuf);

impl VerifiedMpvShader {
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

pub fn resolve_mpv_shader(
    verified_resource_root: &Path,
    shader_path: &Path,
) -> Result<VerifiedMpvShader, RealtimeVideoBackendError> {
    let root = verified_resource_root.canonicalize().map_err(|error| {
        RealtimeVideoBackendError::InvalidResourceRoot {
            path: verified_resource_root.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    let shader =
        shader_path
            .canonicalize()
            .map_err(|error| RealtimeVideoBackendError::InvalidShader {
                path: shader_path.to_path_buf(),
                message: error.to_string(),
            })?;
    if !shader.starts_with(&root) {
        return Err(RealtimeVideoBackendError::ResourceEscapesRoot { path: shader, root });
    }
    if !shader.is_file() {
        return Err(RealtimeVideoBackendError::InvalidShader {
            path: shader,
            message: "路径不是普通文件".to_owned(),
        });
    }
    if !matches!(
        shader.extension().and_then(OsStr::to_str),
        Some("hook" | "glsl")
    ) {
        return Err(RealtimeVideoBackendError::InvalidShader {
            path: shader,
            message: "扩展名必须是 .hook 或 .glsl".to_owned(),
        });
    }
    Ok(VerifiedMpvShader(shader))
}

/// 开发构建允许显式覆盖；发布构建只读取 `verified_resource_root/binaries/mpv.exe`。
pub fn resolve_mpv_executable(
    verified_resource_root: &Path,
) -> Result<VerifiedMpvExecutable, RealtimeVideoBackendError> {
    let development_override = if cfg!(debug_assertions) {
        std::env::var_os(MPV_PATH_ENV)
    } else {
        None
    };
    resolve_mpv_executable_from(
        verified_resource_root,
        development_override.as_deref(),
        cfg!(debug_assertions),
    )
}

fn resolve_mpv_executable_from(
    verified_resource_root: &Path,
    development_override: Option<&OsStr>,
    allow_development_override: bool,
) -> Result<VerifiedMpvExecutable, RealtimeVideoBackendError> {
    if allow_development_override {
        if let Some(path) = development_override.filter(|value| !value.is_empty()) {
            return canonical_executable(Path::new(path)).map(VerifiedMpvExecutable);
        }
    }

    let root = verified_resource_root.canonicalize().map_err(|error| {
        RealtimeVideoBackendError::InvalidResourceRoot {
            path: verified_resource_root.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    let executable = canonical_executable(&root.join("binaries").join("mpv.exe"))?;
    if !executable.starts_with(&root) {
        return Err(RealtimeVideoBackendError::ResourceEscapesRoot {
            path: executable,
            root,
        });
    }
    Ok(VerifiedMpvExecutable(executable))
}

fn canonical_executable(path: &Path) -> Result<PathBuf, RealtimeVideoBackendError> {
    let canonical =
        path.canonicalize()
            .map_err(|error| RealtimeVideoBackendError::InvalidExecutable {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
    let metadata =
        canonical
            .metadata()
            .map_err(|error| RealtimeVideoBackendError::InvalidExecutable {
                path: canonical.clone(),
                message: error.to_string(),
            })?;
    if !metadata.is_file() {
        return Err(RealtimeVideoBackendError::InvalidExecutable {
            path: canonical,
            message: "路径不是普通文件".to_owned(),
        });
    }
    Ok(canonical)
}

fn canonical_media_file(path: &Path) -> Result<PathBuf, RealtimeVideoBackendError> {
    let canonical =
        path.canonicalize()
            .map_err(|error| RealtimeVideoBackendError::InvalidMediaPath {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
    if !canonical.is_file() {
        return Err(RealtimeVideoBackendError::InvalidMediaPath {
            path: canonical,
            message: "路径不是普通文件".to_owned(),
        });
    }
    Ok(canonical)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MpvGraphicsApi {
    D3d11,
    Vulkan,
}

impl MpvGraphicsApi {
    fn fixed_arguments(self) -> [&'static str; 2] {
        match self {
            Self::D3d11 => ["--gpu-api=d3d11", "--gpu-context=d3d11"],
            Self::Vulkan => ["--gpu-api=vulkan", "--gpu-context=winvk"],
        }
    }
}

/// 跨厂商 GPU 候选只描述 mpv 的输出 API 与解码搬运方式，不按显卡厂商分支。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MpvGpuProfile {
    D3d11ZeroCopy,
    D3d11Copy,
    VulkanCopy,
    SoftwareDecode,
}

impl MpvGpuProfile {
    pub const fn attempt_order() -> [Self; 4] {
        [
            Self::D3d11ZeroCopy,
            Self::D3d11Copy,
            Self::VulkanCopy,
            Self::SoftwareDecode,
        ]
    }

    pub const fn graphics_api(self) -> MpvGraphicsApi {
        match self {
            Self::D3d11ZeroCopy | Self::D3d11Copy | Self::SoftwareDecode => MpvGraphicsApi::D3d11,
            Self::VulkanCopy => MpvGraphicsApi::Vulkan,
        }
    }

    const fn hwdec_argument(self) -> &'static str {
        match self {
            Self::D3d11ZeroCopy => "--hwdec=d3d11va",
            Self::D3d11Copy | Self::VulkanCopy => "--hwdec=d3d11va-copy",
            Self::SoftwareDecode => "--hwdec=no",
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::D3d11ZeroCopy => "gpu_d3d11_zero_copy",
            Self::D3d11Copy => "gpu_d3d11_copy",
            Self::VulkanCopy => "gpu_vulkan_copy",
            Self::SoftwareDecode => "gpu_software_decode",
        }
    }
}

/// Phase 4 的唯一 mpv 启动模式；模式决定解码、视频输出、shader 和 CPU4 滤镜边界。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MpvLaunchMode {
    Gpu(MpvGpuProfile),
    Cpu4,
    Original,
}

impl MpvLaunchMode {
    pub const fn fallback_order() -> [Self; 6] {
        [
            Self::Gpu(MpvGpuProfile::D3d11ZeroCopy),
            Self::Gpu(MpvGpuProfile::D3d11Copy),
            Self::Gpu(MpvGpuProfile::VulkanCopy),
            Self::Gpu(MpvGpuProfile::SoftwareDecode),
            Self::Cpu4,
            Self::Original,
        ]
    }

    const fn output_graphics_api(self) -> MpvGraphicsApi {
        match self {
            Self::Gpu(profile) => profile.graphics_api(),
            Self::Cpu4 | Self::Original => MpvGraphicsApi::D3d11,
        }
    }

    pub const fn backend(self) -> VideoBackend {
        match self {
            Self::Gpu(_) => VideoBackend::RealtimeGpu,
            Self::Cpu4 => VideoBackend::Cpu4,
            Self::Original => VideoBackend::Source,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Gpu(profile) => profile.name(),
            Self::Cpu4 => "cpu4",
            Self::Original => "original",
        }
    }

    const fn next_fallback(self) -> Self {
        match self {
            Self::Gpu(MpvGpuProfile::D3d11ZeroCopy) => Self::Gpu(MpvGpuProfile::D3d11Copy),
            Self::Gpu(MpvGpuProfile::D3d11Copy) => Self::Gpu(MpvGpuProfile::VulkanCopy),
            Self::Gpu(MpvGpuProfile::VulkanCopy) => Self::Gpu(MpvGpuProfile::SoftwareDecode),
            Self::Gpu(MpvGpuProfile::SoftwareDecode) => Self::Cpu4,
            Self::Cpu4 | Self::Original => Self::Original,
        }
    }
}

/// 已完成路径和 IPC 边界校验的固定 mpv 启动说明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MpvLaunchSpec {
    executable: VerifiedMpvExecutable,
    arguments: Vec<OsString>,
    mode: MpvLaunchMode,
    ipc_pipe: String,
    media_path: PathBuf,
}

impl MpvLaunchSpec {
    pub fn new(
        executable: VerifiedMpvExecutable,
        selected_media_path: &Path,
        host_window_id: u64,
        ipc_pipe: &str,
        mode: MpvLaunchMode,
        source_start_ms: u64,
        _paused: bool,
    ) -> Result<Self, RealtimeVideoBackendError> {
        // mpv 在 Windows 上把 --wid 按 uint32_t 解释；先在 Rust 边界收窄，
        // 禁止 64 位 HWND 的高位被 mpv 静默截断后绑定到错误窗口。
        let host_window_id = u32::try_from(host_window_id)
            .ok()
            .filter(|window_id| *window_id != 0)
            .ok_or(RealtimeVideoBackendError::InvalidHostWindow)?;
        if !is_managed_ipc_pipe(ipc_pipe) {
            return Err(RealtimeVideoBackendError::InvalidIpcPipe);
        }
        let media = canonical_media_file(selected_media_path)?;
        let mut arguments = [
            "--no-config",
            "--input-default-bindings=no",
            "--input-vo-keyboard=no",
            "--terminal=no",
            "--keep-open=yes",
            "--vo=gpu-next",
            "--audio=no",
        ]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
        let output_graphics_api = mode.output_graphics_api();
        arguments.extend(output_graphics_api.fixed_arguments().map(OsString::from));
        match mode {
            MpvLaunchMode::Gpu(profile) => {
                arguments.push(OsString::from(profile.hwdec_argument()));
            }
            MpvLaunchMode::Cpu4 => {
                // 复用 Phase 1 已实机验证的 CPU 滤镜路径：软件解码，gpu-next/D3D11 只负责最终呈现。
                arguments.push(OsString::from("--hwdec=no"));
                arguments.push(OsString::from(format!("--vf={CPU4_FILTER_CHAIN}")));
            }
            MpvLaunchMode::Original => {
                arguments.push(OsString::from("--hwdec=d3d11va"));
            }
        }
        arguments.push(OsString::from(format!(
            "--start={}.{:03}",
            source_start_ms / 1_000,
            source_start_ms % 1_000
        )));
        // 进程先暂停启动，避免 IPC 连接前失控播放；连接后 runtime 立即恢复权威状态。
        // 播放态随后等待真实 render sample，用户暂停态只验证 VO 与首帧时间线。
        arguments.push(OsString::from("--pause=yes"));
        arguments.push(OsString::from(format!("--wid={host_window_id}")));
        arguments.push(OsString::from(format!("--input-ipc-server={ipc_pipe}")));
        // `--` 必须紧邻媒体路径，避免以 `-` 开头的合法文件名被解释为 mpv 参数。
        arguments.push(OsString::from("--"));
        arguments.push(media.clone().into_os_string());

        Ok(Self {
            executable,
            arguments,
            mode,
            ipc_pipe: ipc_pipe.to_owned(),
            media_path: media,
        })
    }

    fn with_shader(mut self, shader: VerifiedMpvShader) -> Self {
        let shader_argument =
            OsString::from(format!("--glsl-shaders={}", shader.as_path().display()));
        let media_separator = self.arguments.len().saturating_sub(2);
        self.arguments.insert(media_separator, shader_argument);
        self
    }

    // 调用方已有七项受校验启动信息；shader 是唯一可选资源，保留具名构造器避免拆散原子校验。
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_shader(
        executable: VerifiedMpvExecutable,
        selected_media_path: &Path,
        host_window_id: u64,
        ipc_pipe: &str,
        gpu_profile: MpvGpuProfile,
        source_start_ms: u64,
        paused: bool,
        shader: VerifiedMpvShader,
    ) -> Result<Self, RealtimeVideoBackendError> {
        Self::new(
            executable,
            selected_media_path,
            host_window_id,
            ipc_pipe,
            MpvLaunchMode::Gpu(gpu_profile),
            source_start_ms,
            paused,
        )
        .map(|spec| spec.with_shader(shader))
    }

    pub fn executable(&self) -> &Path {
        self.executable.as_path()
    }

    pub fn arguments(&self) -> Vec<String> {
        self.arguments
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect()
    }

    pub fn mode(&self) -> MpvLaunchMode {
        self.mode
    }

    pub fn graphics_api(&self) -> MpvGraphicsApi {
        self.mode.output_graphics_api()
    }

    pub fn ipc_pipe(&self) -> &str {
        &self.ipc_pipe
    }
}

fn is_managed_ipc_pipe(value: &str) -> bool {
    let Some(suffix) = value.strip_prefix(MPV_IPC_PREFIX) else {
        return false;
    };
    !suffix.is_empty()
        && suffix.len() <= 64
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[derive(Debug)]
struct StderrTailBuffer {
    lines: VecDeque<String>,
    bytes: usize,
}

impl StderrTailBuffer {
    fn push(&mut self, mut line: String) {
        if line.len() > STDERR_TAIL_MAX_BYTES {
            let mut boundary = STDERR_TAIL_MAX_BYTES;
            while !line.is_char_boundary(boundary) {
                boundary -= 1;
            }
            line.truncate(boundary);
        }
        self.bytes = self.bytes.saturating_add(line.len());
        self.lines.push_back(line);
        while self.lines.len() > STDERR_TAIL_MAX_LINES || self.bytes > STDERR_TAIL_MAX_BYTES {
            let Some(removed) = self.lines.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(removed.len());
        }
    }
}

#[derive(Debug, Default)]
struct RedactedMediaPaths {
    values: Vec<String>,
}

impl RedactedMediaPaths {
    fn register(&mut self, path: &Path) {
        let canonical = path.to_string_lossy().into_owned();
        let without_verbatim_prefix = canonical
            .strip_prefix(r"\\?\")
            .unwrap_or(&canonical)
            .to_owned();
        for value in [
            canonical,
            without_verbatim_prefix.clone(),
            without_verbatim_prefix.replace('\\', "/"),
        ] {
            if !value.is_empty() && !self.values.contains(&value) {
                self.values.push(value);
            }
        }
        self.values
            .sort_unstable_by_key(|value| std::cmp::Reverse(value.len()));
    }

    fn redact(&self, line: &str) -> String {
        self.values
            .iter()
            .fold(line.to_owned(), |line, path| line.replace(path, "<media>"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MpvRenderFailureKind {
    Shader,
    VideoOutput,
    DeviceLost,
}

impl MpvRenderFailureKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Shader => "shader/hook",
            Self::VideoOutput => "视频输出",
            Self::DeviceLost => "图形设备丢失",
        }
    }
}

fn classify_mpv_render_failure(line: &str) -> Option<MpvRenderFailureKind> {
    let line = line.to_ascii_lowercase();
    if line.contains("error diffusion") {
        return None;
    }
    let failed = [
        "failed",
        "invalid",
        "disabled",
        "fatal",
        "could not",
        "cannot",
        "error:",
        "error initializing",
        "error opening",
        "error while compiling",
        "error compiling",
        "compilation error",
    ]
    .iter()
    .any(|marker| line.contains(marker));
    if [
        "device lost",
        "device removed",
        "device was lost",
        "vk_error_device_lost",
        "dxgi_error_device_removed",
        "dxgi_error_device_hung",
        "dxgi_error_device_reset",
    ]
    .iter()
    .any(|marker| line.contains(marker))
    {
        return Some(MpvRenderFailureKind::DeviceLost);
    }
    if failed
        && ((line.contains("shader") || line.contains("hook"))
            || ((line.contains("compile") || line.contains("compilation"))
                && (line.contains("gpu") || line.contains("glsl"))))
    {
        return Some(MpvRenderFailureKind::Shader);
    }
    if failed
        && [
            "[vo/",
            "video output",
            "video_out",
            "video chain",
            "--vo",
            "gpu-next",
            "libplacebo",
            "vulkan",
            "d3d11",
        ]
        .iter()
        .any(|marker| line.contains(marker))
    {
        return Some(MpvRenderFailureKind::VideoOutput);
    }
    None
}

fn bounded_failure_line(line: &str) -> String {
    line.trim().chars().take(512).collect()
}

fn spawn_stderr_reader(
    stderr: ChildStderr,
    redacted_media_paths: Arc<Mutex<RedactedMediaPaths>>,
) -> Result<(Arc<Mutex<StderrTailBuffer>>, JoinHandle<()>), RealtimeVideoBackendError> {
    let tail = Arc::new(Mutex::new(StderrTailBuffer {
        lines: VecDeque::new(),
        bytes: 0,
    }));
    let writer = Arc::clone(&tail);
    let handle = thread::Builder::new()
        .name("mpv-stderr-tail".to_owned())
        .spawn(move || {
            let mut reader = BufReader::new(stderr);
            loop {
                match read_bounded_stderr_line(&mut reader, STDERR_LINE_MAX_BYTES) {
                    Ok(None) => break,
                    Ok(Some(line)) => {
                        let redacted = match redacted_media_paths.lock() {
                            Ok(paths) => paths.redact(&line),
                            Err(_) => "<stderr redacted>".to_owned(),
                        };
                        let Ok(mut tail) = writer.lock() else {
                            break;
                        };
                        tail.push(redacted);
                    }
                    Err(_) => break,
                }
            }
        })
        .map_err(|error| RealtimeVideoBackendError::ProcessFailed {
            operation: "stderr 读取线程启动",
            message: error.to_string(),
        })?;
    Ok((tail, handle))
}

fn read_bounded_stderr_line<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
) -> io::Result<Option<String>> {
    let mut line = Vec::with_capacity(max_bytes.min(512));
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(String::from_utf8_lossy(&line).into_owned()))
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        let remaining = max_bytes.saturating_sub(line.len());
        let copied = consumed.min(remaining);
        line.extend_from_slice(&available[..copied]);
        reader.consume(consumed);
        if newline.is_some() {
            while matches!(line.last(), Some(b'\n' | b'\r')) {
                line.pop();
            }
            return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
        }
    }
}

/// 音频主时钟、视频调度器和同步控制器只能通过该值类型写入受限播放速度。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct MpvPlaybackSpeed(f64);

impl MpvPlaybackSpeed {
    pub fn new(value: f64) -> Result<Self, RealtimeVideoBackendError> {
        if value.is_finite() && (0.25..=4.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(RealtimeVideoBackendError::IpcProtocol(
                "mpv 播放速度必须是 0.25..=4.0 的有限数".to_owned(),
            ))
        }
    }

    pub const fn as_f64(self) -> f64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for MpvPlaybackSpeed {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(f64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MpvVideoObservation {
    pub media_pts_ms: u64,
    pub source_fps: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MpvPlaybackState {
    pub paused: bool,
    pub seeking: bool,
    pub paused_for_cache: bool,
}

impl MpvPlaybackState {
    pub(crate) fn from_responses(
        paused: &serde_json::Value,
        seeking: &serde_json::Value,
        paused_for_cache: &serde_json::Value,
    ) -> Result<Self, RealtimeVideoBackendError> {
        Ok(Self {
            paused: parse_boolean_response(paused, "pause")?,
            seeking: parse_boolean_response(seeking, "seeking")?,
            paused_for_cache: parse_boolean_response(paused_for_cache, "paused-for-cache")?,
        })
    }
}

impl MpvVideoObservation {
    pub fn from_responses(
        time: &serde_json::Value,
        fps: &serde_json::Value,
    ) -> Result<Option<Self>, RealtimeVideoBackendError> {
        let media_pts_ms = playback_time_ms_from_response(time)?;
        let source_fps = estimated_video_fps_from_response(fps)?;
        let (Some(media_pts_ms), Some(source_fps)) = (media_pts_ms, source_fps) else {
            return Ok(None);
        };
        Ok(Some(Self {
            media_pts_ms,
            source_fps,
        }))
    }
}

pub fn estimated_video_fps_from_response(
    response: &serde_json::Value,
) -> Result<Option<f64>, RealtimeVideoBackendError> {
    let source_fps = response_number(response, MpvObservationProperty::EstimatedVideoFps)?;
    if source_fps.is_some_and(|fps| !(1.0..=240.0).contains(&fps)) {
        return Err(RealtimeVideoBackendError::IpcProtocol(
            "mpv estimated-vf-fps 必须在 [1, 240] 范围内".to_owned(),
        ));
    }
    Ok(source_fps)
}

pub fn playback_time_ms_from_response(
    response: &serde_json::Value,
) -> Result<Option<u64>, RealtimeVideoBackendError> {
    let Some(time_seconds) = response_number(response, MpvObservationProperty::PlaybackTime)?
    else {
        return Ok(None);
    };
    if time_seconds < 0.0 {
        return Err(RealtimeVideoBackendError::IpcProtocol(
            "mpv time-pos 不能为负数".to_owned(),
        ));
    }
    let media_pts_ms = (time_seconds * 1_000.0).round();
    if !media_pts_ms.is_finite() || media_pts_ms >= 18_446_744_073_709_551_616.0 {
        return Err(RealtimeVideoBackendError::IpcProtocol(
            "mpv time-pos 超出毫秒时间戳范围".to_owned(),
        ));
    }
    Ok(Some(media_pts_ms as u64))
}

#[derive(Debug, Clone, Copy)]
enum MpvObservationProperty {
    PlaybackTime,
    EstimatedVideoFps,
}

impl MpvObservationProperty {
    const fn name(self) -> &'static str {
        match self {
            Self::PlaybackTime => "time-pos",
            Self::EstimatedVideoFps => "estimated-vf-fps",
        }
    }
}

fn response_number(
    response: &serde_json::Value,
    property: MpvObservationProperty,
) -> Result<Option<f64>, RealtimeVideoBackendError> {
    let property = property.name();
    if response.get("error").and_then(serde_json::Value::as_str) != Some("success") {
        return Err(RealtimeVideoBackendError::IpcProtocol(format!(
            "mpv {property} 响应未明确成功"
        )));
    }
    let data = response.get("data").ok_or_else(|| {
        RealtimeVideoBackendError::IpcProtocol(format!("mpv {property} 响应缺少 data 字段"))
    })?;
    if data.is_null() {
        return Ok(None);
    }
    let value = data.as_f64().ok_or_else(|| {
        RealtimeVideoBackendError::IpcProtocol(format!("mpv {property} 响应 data 必须是有限数"))
    })?;
    if !value.is_finite() {
        return Err(RealtimeVideoBackendError::IpcProtocol(format!(
            "mpv {property} 响应 data 必须是有限数"
        )));
    }
    Ok(Some(value))
}

pub fn parse_eof_reached_response(
    response: &serde_json::Value,
) -> Result<bool, RealtimeVideoBackendError> {
    parse_boolean_response(response, "eof-reached")
}

fn parse_boolean_response(
    response: &serde_json::Value,
    property: &'static str,
) -> Result<bool, RealtimeVideoBackendError> {
    if response.get("error").and_then(serde_json::Value::as_str) != Some("success") {
        return Err(RealtimeVideoBackendError::IpcProtocol(format!(
            "mpv {property} 响应未明确成功"
        )));
    }
    response
        .get("data")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| {
            RealtimeVideoBackendError::IpcProtocol(format!("mpv {property} 响应 data 必须是布尔值"))
        })
}

fn parse_hwdec_current_response(
    response: &serde_json::Value,
) -> Result<String, RealtimeVideoBackendError> {
    if response.get("error").and_then(serde_json::Value::as_str) != Some("success") {
        return Err(RealtimeVideoBackendError::IpcProtocol(
            "mpv hwdec-current 响应未明确成功".to_owned(),
        ));
    }
    match response.get("data").and_then(serde_json::Value::as_str) {
        Some("d3d11va") => Ok("d3d11va".to_owned()),
        Some("d3d11va-copy") => Ok("d3d11va-copy".to_owned()),
        Some("no") => Ok("software".to_owned()),
        _ => Err(RealtimeVideoBackendError::IpcProtocol(
            "mpv hwdec-current 响应 data 不是受支持的实际解码器".to_owned(),
        )),
    }
}

fn parse_media_path_response(
    response: &serde_json::Value,
) -> Result<Option<PathBuf>, RealtimeVideoBackendError> {
    if response.get("error").and_then(serde_json::Value::as_str) != Some("success") {
        return Err(RealtimeVideoBackendError::IpcProtocol(
            "mpv path 响应未明确成功".to_owned(),
        ));
    }
    let data = response.get("data").ok_or_else(|| {
        RealtimeVideoBackendError::IpcProtocol("mpv path 响应缺少 data 字段".to_owned())
    })?;
    if data.is_null() {
        return Ok(None);
    }
    let path = data
        .as_str()
        .filter(|path| !path.is_empty())
        .ok_or_else(|| {
            RealtimeVideoBackendError::IpcProtocol("mpv path 响应 data 必须是非空字符串".to_owned())
        })?;
    canonical_media_file(Path::new(path)).map(Some)
}

fn strict_remaining_budget(
    started: Instant,
    total_budget: Duration,
) -> Result<Duration, RealtimeVideoBackendError> {
    total_budget
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(RealtimeVideoBackendError::MediaSwitchTimeout)
}

fn video_output_has_rendered_frame(
    response: &serde_json::Value,
) -> Result<bool, RealtimeVideoBackendError> {
    if response.get("error").and_then(serde_json::Value::as_str) != Some("success") {
        return Err(RealtimeVideoBackendError::IpcProtocol(
            "mpv vo-passes 响应未明确成功".to_owned(),
        ));
    }
    Ok(response
        .get("data")
        .and_then(|data| data.get("fresh"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|passes| {
            passes.iter().any(|pass| {
                pass.get("count")
                    .and_then(serde_json::Value::as_u64)
                    .is_some_and(|count| count > 0)
                    && pass
                        .get("samples")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|samples| !samples.is_empty())
            })
        }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlaybackResumeFact {
    paused: bool,
    eof_reached: bool,
    seeking: bool,
    source_pts_ms: u64,
}

fn physical_playback_resumed(
    first: PlaybackResumeFact,
    current: PlaybackResumeFact,
    source_duration_ms: u64,
) -> bool {
    !first.paused
        && !first.eof_reached
        && !first.seeking
        && first.source_pts_ms < source_duration_ms
        && !current.paused
        && !current.eof_reached
        && !current.seeking
        && current.source_pts_ms < source_duration_ms
        && current.source_pts_ms > first.source_pts_ms
}

fn read_playback_resume_fact<F>(
    started: Instant,
    total_budget: Duration,
    send: &mut F,
) -> Result<Option<PlaybackResumeFact>, RealtimeVideoBackendError>
where
    F: FnMut(&MpvCommand, Duration) -> Result<serde_json::Value, RealtimeVideoBackendError>,
{
    let paused = match send(
        &MpvCommand::GetPaused,
        strict_remaining_budget(started, total_budget)?,
    ) {
        Ok(response) => parse_boolean_response(&response, "pause")?,
        Err(RealtimeVideoBackendError::PropertyUnavailable { .. }) => return Ok(None),
        Err(error) => return Err(error),
    };
    let eof_reached = match send(
        &MpvCommand::GetEofReached,
        strict_remaining_budget(started, total_budget)?,
    ) {
        Ok(response) => parse_eof_reached_response(&response)?,
        Err(RealtimeVideoBackendError::PropertyUnavailable { .. }) => return Ok(None),
        Err(error) => return Err(error),
    };
    let seeking = match send(
        &MpvCommand::GetSeeking,
        strict_remaining_budget(started, total_budget)?,
    ) {
        Ok(response) => parse_boolean_response(&response, "seeking")?,
        Err(RealtimeVideoBackendError::PropertyUnavailable { .. }) => return Ok(None),
        Err(error) => return Err(error),
    };
    let source_pts_ms = match send(
        &MpvCommand::GetPlaybackTime,
        strict_remaining_budget(started, total_budget)?,
    ) {
        Ok(response) => playback_time_ms_from_response(&response)?,
        Err(RealtimeVideoBackendError::PropertyUnavailable { .. }) => None,
        Err(error) => return Err(error),
    };
    Ok(source_pts_ms.map(|source_pts_ms| PlaybackResumeFact {
        paused,
        eof_reached,
        seeking,
        source_pts_ms,
    }))
}

fn wait_for_physical_playback_resume<F, C>(
    source_duration_ms: u64,
    started: Instant,
    total_budget: Duration,
    mut cancelled: C,
    mut send: F,
) -> Result<u64, RealtimeVideoBackendError>
where
    F: FnMut(&MpvCommand, Duration) -> Result<serde_json::Value, RealtimeVideoBackendError>,
    C: FnMut() -> bool,
{
    let mut first = None;
    loop {
        if cancelled() {
            return Err(RealtimeVideoBackendError::MediaSwitchCancelled);
        }
        match read_playback_resume_fact(started, total_budget, &mut send)? {
            Some(current)
                if !current.paused
                    && !current.eof_reached
                    && !current.seeking
                    && current.source_pts_ms < source_duration_ms =>
            {
                if first.is_some_and(|initial| {
                    physical_playback_resumed(initial, current, source_duration_ms)
                }) {
                    return Ok(current.source_pts_ms);
                }
                first = Some(current);
            }
            _ => first = None,
        }
        thread::sleep(strict_remaining_budget(started, total_budget)?.min(MPV_MEDIA_SWITCH_POLL));
    }
}

fn wait_for_loaded_media<F, C>(
    expected_media_path: &Path,
    source_duration_ms: u64,
    started: Instant,
    total_budget: Duration,
    expected_paused: bool,
    mut cancelled: C,
    mut send: F,
) -> Result<u64, RealtimeVideoBackendError>
where
    F: FnMut(&MpvCommand, Duration) -> Result<serde_json::Value, RealtimeVideoBackendError>,
    C: FnMut() -> bool,
{
    loop {
        if cancelled() {
            return Err(RealtimeVideoBackendError::MediaSwitchCancelled);
        }
        let path = match send(
            &MpvCommand::GetMediaPath,
            strict_remaining_budget(started, total_budget)?,
        ) {
            Ok(response) => parse_media_path_response(&response)?,
            Err(RealtimeVideoBackendError::PropertyUnavailable { .. }) => None,
            Err(error) => return Err(error),
        };
        if path.as_deref() == Some(expected_media_path) {
            let paused = match send(
                &MpvCommand::GetPaused,
                strict_remaining_budget(started, total_budget)?,
            ) {
                Ok(response) => parse_boolean_response(&response, "pause")?,
                Err(RealtimeVideoBackendError::PropertyUnavailable { .. }) => !expected_paused,
                Err(error) => return Err(error),
            };
            let eof_reached = match send(
                &MpvCommand::GetEofReached,
                strict_remaining_budget(started, total_budget)?,
            ) {
                Ok(response) => parse_eof_reached_response(&response)?,
                Err(RealtimeVideoBackendError::PropertyUnavailable { .. }) => true,
                Err(error) => return Err(error),
            };
            let seeking = match send(
                &MpvCommand::GetSeeking,
                strict_remaining_budget(started, total_budget)?,
            ) {
                Ok(response) => parse_boolean_response(&response, "seeking")?,
                Err(RealtimeVideoBackendError::PropertyUnavailable { .. }) => true,
                Err(error) => return Err(error),
            };
            let vo_configured = match send(
                &MpvCommand::GetVideoOutputConfigured,
                strict_remaining_budget(started, total_budget)?,
            ) {
                Ok(response) => parse_boolean_response(&response, "vo-configured")?,
                Err(RealtimeVideoBackendError::PropertyUnavailable { .. }) => false,
                Err(error) => return Err(error),
            };
            if paused == expected_paused && !eof_reached && !seeking && vo_configured {
                let time_pos = match send(
                    &MpvCommand::GetPlaybackTime,
                    strict_remaining_budget(started, total_budget)?,
                ) {
                    Ok(response) => playback_time_ms_from_response(&response)?,
                    Err(RealtimeVideoBackendError::PropertyUnavailable { .. }) => None,
                    Err(error) => return Err(error),
                };
                if let Some(time_pos) = time_pos {
                    if time_pos >= source_duration_ms {
                        continue;
                    }
                    if expected_paused {
                        return Ok(time_pos);
                    }
                    let passes = match send(
                        &MpvCommand::GetVideoOutputPasses,
                        strict_remaining_budget(started, total_budget)?,
                    ) {
                        Ok(response) => video_output_has_rendered_frame(&response)?,
                        Err(RealtimeVideoBackendError::PropertyUnavailable { .. }) => false,
                        Err(error) => return Err(error),
                    };
                    if passes {
                        return wait_for_physical_playback_resume(
                            source_duration_ms,
                            started,
                            total_budget,
                            &mut cancelled,
                            &mut send,
                        );
                    }
                }
            }
        }

        // 该等待只用于限制属性轮询频率；成功完全由 path/VO/PTS 事实决定。
        thread::sleep(strict_remaining_budget(started, total_budget)?.min(MPV_MEDIA_SWITCH_POLL));
    }
}

/// mpv 子进程、持久 IPC 与日志线程的唯一所有者。
#[derive(Debug)]
pub struct ManagedMpvProcess {
    child: Option<Child>,
    job: Option<ManagedMpvJob>,
    ipc: Option<MpvIpcClient>,
    stderr_tail: Arc<Mutex<StderrTailBuffer>>,
    redacted_media_paths: Arc<Mutex<RedactedMediaPaths>>,
    stderr_join: Option<JoinHandle<()>>,
}

/// mpv 非预期退出时保留的最小诊断证据。
///
/// stderr 已在读取线程中完成源媒体路径脱敏和总量限制；这里仅复制当前尾部，
/// 避免 runtime 在回收进程后只剩下“会话丢失”这一类不可行动的泛化错误。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MpvProcessExitEvidence {
    pub exit_code: Option<i32>,
    pub success: bool,
    pub stderr_tail: Vec<String>,
}

impl MpvProcessExitEvidence {
    pub fn summary(&self) -> String {
        let exit_code = self
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "无可用退出码".to_owned());
        let diagnostic = self
            .stderr_tail
            .iter()
            .rev()
            .find(|line| stderr_line_is_meaningful(line))
            .map(|line| bounded_failure_line(line));
        match diagnostic {
            Some(diagnostic) => format!("退出码 {exit_code}；stderr：{diagnostic}"),
            None => format!("退出码 {exit_code}；stderr 无有效诊断"),
        }
    }
}

impl ManagedMpvProcess {
    pub fn spawn(spec: &MpvLaunchSpec) -> Result<Self, RealtimeVideoBackendError> {
        // 先完成 KILL_ON_JOB_CLOSE 配置；创建失败时绝不启动未受管 mpv。
        let job =
            ManagedMpvJob::create().map_err(|error| RealtimeVideoBackendError::ProcessFailed {
                operation: "Job Object 创建",
                message: error.to_string(),
            })?;
        let mut child = background_command(spec.executable())
            .args(&spec.arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| RealtimeVideoBackendError::SpawnFailed {
                path: spec.executable().to_path_buf(),
                message: error.to_string(),
            })?;
        if let Err(error) = job.assign_process(&child) {
            let cleanup_error = terminate_unmanaged_child(&mut child).err();
            let message = match cleanup_error {
                Some(cleanup_error) => {
                    format!("{error}；未受管 mpv 回收失败：{cleanup_error}")
                }
                None => error.to_string(),
            };
            return Err(RealtimeVideoBackendError::ProcessFailed {
                operation: "Job Object 绑定",
                message,
            });
        }
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                drop(job);
                let cleanup_error = terminate_unmanaged_child(&mut child).err();
                let message = match cleanup_error {
                    Some(error) => format!("mpv stderr 管道未创建；进程回收失败：{error}"),
                    None => "mpv stderr 管道未创建".to_owned(),
                };
                return Err(RealtimeVideoBackendError::ProcessFailed {
                    operation: "stderr 捕获",
                    message,
                });
            }
        };
        let mut redacted_media_paths = RedactedMediaPaths::default();
        redacted_media_paths.register(&spec.media_path);
        let redacted_media_paths = Arc::new(Mutex::new(redacted_media_paths));
        let (stderr_tail, stderr_join) =
            match spawn_stderr_reader(stderr, Arc::clone(&redacted_media_paths)) {
                Ok(reader) => reader,
                Err(error) => {
                    let _ignored = child.kill();
                    let _ignored = child.wait();
                    return Err(error);
                }
            };
        Ok(Self {
            child: Some(child),
            job: Some(job),
            ipc: None,
            stderr_tail,
            redacted_media_paths,
            stderr_join: Some(stderr_join),
        })
    }

    pub fn spawn_connected(
        spec: &MpvLaunchSpec,
        options: MpvIpcOptions,
    ) -> Result<Self, RealtimeVideoBackendError> {
        let mut process = Self::spawn(spec)?;
        if let Err(error) = process.connect_ipc(spec.ipc_pipe(), options) {
            let _ignored = process.cancel();
            return Err(error);
        }
        Ok(process)
    }

    pub fn connect_ipc(
        &mut self,
        ipc_pipe: &str,
        options: MpvIpcOptions,
    ) -> Result<(), RealtimeVideoBackendError> {
        if self.ipc.is_some() {
            return Err(RealtimeVideoBackendError::IpcProtocol(
                "同一 mpv 进程只能建立一个持久 IPC 连接".to_owned(),
            ));
        }
        if !is_managed_ipc_pipe(ipc_pipe) {
            return Err(RealtimeVideoBackendError::InvalidIpcPipe);
        }
        self.ipc = Some(MpvIpcClient::connect(ipc_pipe, options)?);
        Ok(())
    }

    pub fn send_command(
        &self,
        command: &MpvCommand,
        deadline: Duration,
    ) -> Result<serde_json::Value, RealtimeVideoBackendError> {
        self.ipc
            .as_ref()
            .ok_or_else(|| {
                RealtimeVideoBackendError::IpcDisconnected("持久 IPC 尚未连接".to_owned())
            })?
            .send(command, deadline)
    }

    /// 只为 shader 完整快照开放非阻塞提交；其他 IPC 命令继续走同步受限入口。
    pub fn submit_shader_options(
        &self,
        options: MpvShaderOptions,
    ) -> Result<PendingMpvResponse, RealtimeVideoBackendError> {
        self.ipc
            .as_ref()
            .ok_or_else(|| {
                RealtimeVideoBackendError::IpcDisconnected("持久 IPC 尚未连接".to_owned())
            })?
            .submit(&MpvCommand::SetShaderOptions { options })
    }

    /// 换源后的实际 FPS 由 runtime actor 以异步事务读取。属性暂不可用或软等待
    /// 超时时不得关闭会话，也不得让后续控制命令越过仍在途的响应。
    pub fn submit_estimated_video_fps(
        &self,
    ) -> Result<PendingMpvResponse, RealtimeVideoBackendError> {
        self.ipc
            .as_ref()
            .ok_or_else(|| {
                RealtimeVideoBackendError::IpcDisconnected("持久 IPC 尚未连接".to_owned())
            })?
            .submit(&MpvCommand::GetEstimatedVideoFps)
    }

    pub fn submit_eof_reached(&self) -> Result<PendingMpvResponse, RealtimeVideoBackendError> {
        self.ipc
            .as_ref()
            .ok_or_else(|| {
                RealtimeVideoBackendError::IpcDisconnected("持久 IPC 尚未连接".to_owned())
            })?
            .submit(&MpvCommand::GetEofReached)
    }

    pub fn submit_video_pts(&self) -> Result<PendingMpvResponse, RealtimeVideoBackendError> {
        self.ipc
            .as_ref()
            .ok_or_else(|| {
                RealtimeVideoBackendError::IpcDisconnected("持久 IPC 尚未连接".to_owned())
            })?
            .submit(&MpvCommand::GetPlaybackTime)
    }

    pub fn submit_paused(&self) -> Result<PendingMpvResponse, RealtimeVideoBackendError> {
        self.ipc
            .as_ref()
            .ok_or_else(|| {
                RealtimeVideoBackendError::IpcDisconnected("持久 IPC 尚未连接".to_owned())
            })?
            .submit(&MpvCommand::GetPaused)
    }

    pub fn submit_seeking(&self) -> Result<PendingMpvResponse, RealtimeVideoBackendError> {
        self.ipc
            .as_ref()
            .ok_or_else(|| {
                RealtimeVideoBackendError::IpcDisconnected("持久 IPC 尚未连接".to_owned())
            })?
            .submit(&MpvCommand::GetSeeking)
    }

    pub fn submit_paused_for_cache(&self) -> Result<PendingMpvResponse, RealtimeVideoBackendError> {
        self.ipc
            .as_ref()
            .ok_or_else(|| {
                RealtimeVideoBackendError::IpcDisconnected("持久 IPC 尚未连接".to_owned())
            })?
            .submit(&MpvCommand::GetPausedForCache)
    }

    /// 从固定 `glsl-shader-opts` 属性读回并严格解析有界快照。
    pub fn read_shader_options(
        &self,
        deadline: Duration,
    ) -> Result<MpvShaderOptionMap, RealtimeVideoBackendError> {
        let response = self.send_command(&MpvCommand::GetShaderOptions, deadline)?;
        MpvShaderOptionMap::from_response(&response)
    }

    pub fn read_video_observation(
        &self,
        total_deadline: Duration,
    ) -> Result<Option<MpvVideoObservation>, RealtimeVideoBackendError> {
        let started = Instant::now();
        let time = self.send_command(&MpvCommand::GetPlaybackTime, total_deadline)?;
        let fps = self.send_command(
            &MpvCommand::GetEstimatedVideoFps,
            remaining_ipc_deadline(started, total_deadline),
        )?;
        MpvVideoObservation::from_responses(&time, &fps)
    }

    pub fn read_video_pts(
        &self,
        deadline: Duration,
    ) -> Result<Option<u64>, RealtimeVideoBackendError> {
        let response = self.send_command(&MpvCommand::GetPlaybackTime, deadline)?;
        playback_time_ms_from_response(&response)
    }

    pub fn read_eof_reached(&self, deadline: Duration) -> Result<bool, RealtimeVideoBackendError> {
        let response = self.send_command(&MpvCommand::GetEofReached, deadline)?;
        parse_eof_reached_response(&response)
    }

    pub fn read_paused(&self, deadline: Duration) -> Result<bool, RealtimeVideoBackendError> {
        let response = self.send_command(&MpvCommand::GetPaused, deadline)?;
        parse_boolean_response(&response, "pause")
    }

    pub fn read_playback_state(
        &self,
        total_deadline: Duration,
    ) -> Result<MpvPlaybackState, RealtimeVideoBackendError> {
        let started = Instant::now();
        let paused = self.send_command(&MpvCommand::GetPaused, total_deadline)?;
        let seeking = self.send_command(
            &MpvCommand::GetSeeking,
            remaining_ipc_deadline(started, total_deadline),
        )?;
        let paused_for_cache = self.send_command(
            &MpvCommand::GetPausedForCache,
            remaining_ipc_deadline(started, total_deadline),
        )?;
        MpvPlaybackState::from_responses(&paused, &seeking, &paused_for_cache)
    }

    pub fn read_active_decoder(
        &self,
        deadline: Duration,
    ) -> Result<String, RealtimeVideoBackendError> {
        let response = self.send_command(&MpvCommand::GetHardwareDecoderCurrent, deadline)?;
        parse_hwdec_current_response(&response)
    }

    pub fn read_media_path(
        &self,
        deadline: Duration,
    ) -> Result<Option<PathBuf>, RealtimeVideoBackendError> {
        let response = self.send_command(&MpvCommand::GetMediaPath, deadline)?;
        parse_media_path_response(&response)
    }

    /// 在当前受管 mpv 与唯一 IPC worker 上异步换源，并等待新源首帧事实可用。
    /// 返回首个可用的新源 PTS；所有命令和轮询共享 `total_budget`。
    pub fn switch_media_source<C>(
        &mut self,
        selected_media_path: &Path,
        source_start_ms: u64,
        source_duration_ms: u64,
        total_budget: Duration,
        paused: bool,
        mut cancelled: C,
    ) -> Result<u64, RealtimeVideoBackendError>
    where
        C: FnMut() -> bool,
    {
        let started = Instant::now();
        let command = MpvCommand::load_file_replace(selected_media_path, source_start_ms)?;
        let media_path = command.load_file_path().ok_or_else(|| {
            RealtimeVideoBackendError::IpcProtocol("loadfile 路径缺失".to_owned())
        })?;
        self.redacted_media_paths
            .lock()
            .map_err(|_| RealtimeVideoBackendError::ProcessFailed {
                operation: "stderr 路径脱敏注册",
                message: "路径集合锁已中毒".to_owned(),
            })?
            .register(&media_path);
        if cancelled() {
            return Err(RealtimeVideoBackendError::MediaSwitchCancelled);
        }
        self.send_command(&command, strict_remaining_budget(started, total_budget)?)?;
        // keep-open 到达 EOF 后会把 pause 留在 true；loadfile 又会继承运行时属性。
        // 换源命令返回后显式恢复调用方状态，避免新文件首帧加载后永久停住。
        let attempt_budget = strict_remaining_budget(started, total_budget)? / 2;
        let mut last_error = None;
        for attempt in 0..2 {
            self.send_command(
                &MpvCommand::SetPause { paused },
                strict_remaining_budget(started, total_budget)?,
            )?;
            let attempt_started = Instant::now();
            let budget = if attempt == 0 {
                attempt_budget
            } else {
                strict_remaining_budget(started, total_budget)?
            };
            match wait_for_loaded_media(
                &media_path,
                source_duration_ms,
                attempt_started,
                budget,
                paused,
                &mut cancelled,
                |command, deadline| self.send_command(command, deadline),
            ) {
                Ok(position_ms) => return Ok(position_ms),
                Err(RealtimeVideoBackendError::MediaSwitchTimeout) => {
                    last_error = Some(RealtimeVideoBackendError::ProcessFailed {
                        operation: "换源后播放恢复确认",
                        message: "mpv 未在预算内同时满足 pause/eof/源内 PTS 前进条件".to_owned(),
                    });
                }
                Err(error) => return Err(error),
            }
        }
        Err(
            last_error.unwrap_or(RealtimeVideoBackendError::ProcessFailed {
                operation: "换源后播放恢复确认",
                message: "mpv 物理播放状态无法确认".to_owned(),
            }),
        )
    }

    pub fn resume_after_eof(
        &self,
        source_start_ms: u64,
        source_duration_ms: u64,
        total_budget: Duration,
    ) -> Result<u64, RealtimeVideoBackendError> {
        if source_start_ms >= source_duration_ms {
            return Err(RealtimeVideoBackendError::InvalidSync {
                message: "EOF 恢复位置必须位于当前源媒体内".to_owned(),
            });
        }
        let started = Instant::now();
        let mut last_error = None;
        for attempt in 0..2 {
            let remaining = strict_remaining_budget(started, total_budget)?;
            let attempt_budget = if attempt == 0 {
                remaining / 2
            } else {
                remaining
            };
            let attempt_started = Instant::now();
            self.send_command(
                &MpvCommand::SeekAbsoluteMs {
                    position_ms: source_start_ms,
                },
                strict_remaining_budget(attempt_started, attempt_budget)?,
            )?;
            self.send_command(
                &MpvCommand::SetPause { paused: false },
                strict_remaining_budget(attempt_started, attempt_budget)?,
            )?;
            match wait_for_physical_playback_resume(
                source_duration_ms,
                attempt_started,
                attempt_budget,
                || false,
                |command, deadline| self.send_command(command, deadline),
            ) {
                Ok(position_ms) => return Ok(position_ms),
                Err(RealtimeVideoBackendError::MediaSwitchTimeout) => {
                    last_error = Some(RealtimeVideoBackendError::ProcessFailed {
                        operation: "EOF 播放恢复确认",
                        message: "mpv 仍处于暂停/EOF，或源内 PTS 未继续前进".to_owned(),
                    });
                }
                Err(error) => return Err(error),
            }
        }
        Err(
            last_error.unwrap_or(RealtimeVideoBackendError::ProcessFailed {
                operation: "EOF 播放恢复确认",
                message: "mpv 物理播放状态无法确认".to_owned(),
            }),
        )
    }

    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail
            .lock()
            .map(|tail| tail.lines.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn startup_stderr_diagnostic(&self) -> Option<String> {
        let tail = self.stderr_tail.lock().ok()?;
        stderr_diagnostic_line(&tail)
    }

    pub fn fatal_render_failure(&self) -> Option<String> {
        let tail = self.stderr_tail.lock().ok()?;
        tail.lines.iter().find_map(|line| {
            classify_mpv_render_failure(line)
                .map(|kind| format!("mpv {}错误：{}", kind.label(), bounded_failure_line(line)))
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    pub fn has_exited(&mut self) -> Result<bool, RealtimeVideoBackendError> {
        let Some(child) = self.child.as_mut() else {
            return Ok(true);
        };
        child
            .try_wait()
            .map(|status| status.is_some())
            .map_err(|error| RealtimeVideoBackendError::ProcessFailed {
                operation: "状态检查",
                message: error.to_string(),
            })
    }

    pub fn poll_exit_evidence(
        &mut self,
    ) -> Result<Option<MpvProcessExitEvidence>, RealtimeVideoBackendError> {
        let Some(child) = self.child.as_mut() else {
            return Ok(None);
        };
        let Some(status) =
            child
                .try_wait()
                .map_err(|error| RealtimeVideoBackendError::ProcessFailed {
                    operation: "状态检查",
                    message: error.to_string(),
                })?
        else {
            return Ok(None);
        };
        Ok(Some(MpvProcessExitEvidence {
            exit_code: status.code(),
            success: status.success(),
            stderr_tail: self.stderr_tail(),
        }))
    }

    pub fn cancel(&mut self) -> Result<(), RealtimeVideoBackendError> {
        let mut first_error = None;
        if let Some(ipc) = self.ipc.as_ref() {
            // mpv 可以在返回 quit 响应前关闭管道；最终以子进程是否退出为准。
            let _ignored = ipc.send(&MpvCommand::Quit, MPV_QUIT_RESPONSE_TIMEOUT);
        }
        if let Some(child) = self.child.as_mut() {
            let running = child
                .try_wait()
                .map_err(|error| RealtimeVideoBackendError::ProcessFailed {
                    operation: "状态检查",
                    message: error.to_string(),
                })?
                .is_none();
            let exited_gracefully =
                self.ipc.is_some() && wait_for_child_exit(child, MPV_EXIT_GRACE)?;
            if running && !exited_gracefully {
                #[cfg(windows)]
                let exited_after_job_close = {
                    // 关闭唯一 Job handle 会由 KILL_ON_JOB_CLOSE 终止整个 mpv 进程树。
                    let _job_kill_guard = self.job.take();
                    drop(_job_kill_guard);
                    match wait_for_child_exit(child, MPV_EXIT_GRACE) {
                        Ok(exited) => exited,
                        Err(error) => {
                            first_error = Some(error);
                            false
                        }
                    }
                };
                #[cfg(not(windows))]
                let exited_after_job_close = false;

                if !exited_after_job_close {
                    #[cfg(windows)]
                    {
                        let _ignored = background_command("taskkill")
                            .args(["/PID", &child.id().to_string(), "/T", "/F"])
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status();
                    }
                    child
                        .kill()
                        .or_else(|error| {
                            child.try_wait().and_then(|status| {
                                if status.is_some() {
                                    Ok(())
                                } else {
                                    Err(error)
                                }
                            })
                        })
                        .map_err(|error| RealtimeVideoBackendError::ProcessFailed {
                            operation: "取消",
                            message: error.to_string(),
                        })?;
                    child
                        .wait()
                        .map_err(|error| RealtimeVideoBackendError::ProcessFailed {
                            operation: "回收",
                            message: error.to_string(),
                        })?;
                }
            }
        }
        self.child = None;
        if let Some(mut ipc) = self.ipc.take() {
            if let Err(error) = ipc.shutdown() {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        if let Some(join) = self.stderr_join.take() {
            if join.join().is_err() && first_error.is_none() {
                first_error = Some(RealtimeVideoBackendError::ProcessFailed {
                    operation: "stderr 读取线程 Join",
                    message: "线程发生 panic".to_owned(),
                });
            }
        }
        self.job = None;
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

fn remaining_ipc_deadline(started: Instant, total_deadline: Duration) -> Duration {
    total_deadline
        .saturating_sub(started.elapsed())
        .max(Duration::from_millis(1))
}

fn terminate_unmanaged_child(child: &mut Child) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ignored = background_command("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    child
        .kill()
        .or_else(|error| {
            child
                .try_wait()
                .and_then(|status| if status.is_some() { Ok(()) } else { Err(error) })
        })
        .map_err(|error| error.to_string())?;
    child.wait().map(|_| ()).map_err(|error| error.to_string())
}

fn wait_for_child_exit(
    child: &mut Child,
    timeout: Duration,
) -> Result<bool, RealtimeVideoBackendError> {
    let started = Instant::now();
    loop {
        if child
            .try_wait()
            .map_err(|error| RealtimeVideoBackendError::ProcessFailed {
                operation: "退出等待",
                message: error.to_string(),
            })?
            .is_some()
        {
            return Ok(true);
        }
        if started.elapsed() >= timeout {
            return Ok(false);
        }
        thread::sleep(MPV_EXIT_POLL);
    }
}

fn stderr_diagnostic_line(tail: &StderrTailBuffer) -> Option<String> {
    tail.lines
        .iter()
        .rev()
        .find(|line| classify_mpv_render_failure(line).is_some())
        .or_else(|| {
            tail.lines
                .iter()
                .rev()
                .find(|line| stderr_line_is_actionable(line))
        })
        .or_else(|| {
            tail.lines
                .iter()
                .rev()
                .find(|line| stderr_line_is_meaningful(line))
        })
        .map(|line| bounded_failure_line(line))
}

fn stderr_line_is_actionable(line: &str) -> bool {
    if !stderr_line_is_meaningful(line) {
        return false;
    }
    let line = line.to_ascii_lowercase();
    ["error", "failed", "fatal", "cannot", "could not"]
        .iter()
        .any(|marker| line.contains(marker))
}

fn stderr_line_is_meaningful(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty() && !line.to_ascii_lowercase().starts_with("exiting...")
}

impl Drop for ManagedMpvProcess {
    fn drop(&mut self) {
        let _ignored = self.cancel();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoBackend {
    RealtimeGpu,
    Cpu4,
    Source,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendDemotion {
    pub from: VideoBackend,
    pub to: VideoBackend,
    pub from_mode: String,
    pub to_mode: String,
    pub reason: String,
    pub at_unix_ms: u64,
}

/// 每次播放会话新建；没有升级方法，确保会话内只能单向降级。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoBackendStateMachine {
    current: MpvLaunchMode,
    last_demotion: Option<BackendDemotion>,
    demotion_history: Vec<BackendDemotion>,
}

impl VideoBackendStateMachine {
    pub fn new() -> Self {
        Self {
            current: MpvLaunchMode::Gpu(MpvGpuProfile::D3d11ZeroCopy),
            last_demotion: None,
            demotion_history: Vec::new(),
        }
    }

    pub fn current(&self) -> VideoBackend {
        self.current.backend()
    }

    pub fn launch_mode(&self) -> MpvLaunchMode {
        self.current
    }

    pub fn last_demotion(&self) -> Option<&BackendDemotion> {
        self.last_demotion.as_ref()
    }

    pub fn demotion_history(&self) -> &[BackendDemotion] {
        &self.demotion_history
    }

    pub fn demote(&mut self, reason: impl Into<String>, at_unix_ms: u64) -> VideoBackend {
        let next = self.current.next_fallback();
        if next != self.current {
            let demotion = BackendDemotion {
                from: self.current.backend(),
                to: next.backend(),
                from_mode: self.current.name().to_owned(),
                to_mode: next.name().to_owned(),
                reason: reason.into(),
                at_unix_ms,
            };
            self.demotion_history.push(demotion.clone());
            self.last_demotion = Some(demotion);
            self.current = next;
        }
        self.current.backend()
    }
}

impl Default for VideoBackendStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RendererLifecycleState {
    Unavailable,
    Probing,
    Spawned,
    Active,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeVideoParameter {
    pub field: String,
    pub value: f64,
    pub active: bool,
}

impl RealtimeVideoParameter {
    pub fn active(field: impl Into<String>, value: f64) -> Self {
        Self {
            field: field.into(),
            value,
            active: true,
        }
    }

    pub fn inactive(field: impl Into<String>, value: f64) -> Self {
        Self {
            field: field.into(),
            value,
            active: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterSupportResult {
    pub field: String,
    pub active: bool,
    pub supported: bool,
    pub mapping: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterSupportReport {
    pub backend: VideoBackend,
    pub fully_supported: bool,
    pub parameters: Vec<ParameterSupportResult>,
}

impl ParameterSupportReport {
    pub fn empty(backend: VideoBackend) -> Self {
        Self {
            backend,
            fully_supported: true,
            parameters: Vec::new(),
        }
    }

    pub fn ignored_active_parameter_count(&self) -> usize {
        self.parameters
            .iter()
            .filter(|parameter| parameter.active && !parameter.supported)
            .count()
    }

    pub fn ignored_active_parameter_examples(&self, limit: usize) -> Vec<String> {
        self.parameters
            .iter()
            .filter(|parameter| parameter.active && !parameter.supported)
            .take(limit)
            .map(|parameter| parameter.field.clone())
            .collect()
    }
}

/// CPU4 只暴露四个产品参数；滤镜、命令和目标均不能由调用方提供字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cpu4Parameter {
    Brightness,
    Contrast,
    Saturation,
    Hue,
}

impl Cpu4Parameter {
    const fn ui_name(self) -> &'static str {
        match self {
            Self::Brightness => "brightness_percent",
            Self::Contrast => "contrast_percent",
            Self::Saturation => "saturation_percent",
            Self::Hue => "hue_rotation_degrees",
        }
    }

    const fn command(self) -> &'static str {
        match self {
            Self::Brightness => "brightness",
            Self::Contrast => "contrast",
            Self::Saturation => "saturation",
            Self::Hue => "h",
        }
    }

    const fn target(self) -> &'static str {
        match self {
            Self::Brightness | Self::Contrast | Self::Saturation => CPU4_EQ_TARGET,
            Self::Hue => CPU4_HUE_TARGET,
        }
    }

    const fn mapped_range(self) -> (f64, f64) {
        match self {
            Self::Brightness => (-1.0, 1.0),
            Self::Contrast | Self::Saturation => (0.0, 2.0),
            Self::Hue => (-180.0, 180.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Cpu4FilterUpdate {
    parameter: Cpu4Parameter,
    value: f64,
}

impl Cpu4FilterUpdate {
    fn from_ui(parameter: Cpu4Parameter, ui_value: f64) -> Result<Self, RealtimeVideoBackendError> {
        if !ui_value.is_finite() {
            return Err(RealtimeVideoBackendError::InvalidCpu4Parameter {
                parameter: parameter.ui_name(),
                message: "必须是有限数",
            });
        }
        let (ui_min, ui_max, value) = match parameter {
            Cpu4Parameter::Brightness => (-100.0, 100.0, ui_value / 100.0),
            Cpu4Parameter::Contrast | Cpu4Parameter::Saturation => (0.0, 200.0, ui_value / 100.0),
            Cpu4Parameter::Hue => (-180.0, 180.0, ui_value),
        };
        if !(ui_min..=ui_max).contains(&ui_value) {
            return Err(RealtimeVideoBackendError::InvalidCpu4Parameter {
                parameter: parameter.ui_name(),
                message: "超出产品范围",
            });
        }
        Ok(Self { parameter, value })
    }

    fn validate(self) -> Result<Self, RealtimeVideoBackendError> {
        let (min, max) = self.parameter.mapped_range();
        if self.value.is_finite() && (min..=max).contains(&self.value) {
            Ok(self)
        } else {
            Err(RealtimeVideoBackendError::InvalidCpu4Parameter {
                parameter: self.parameter.ui_name(),
                message: "映射值非法",
            })
        }
    }
}

/// 已完成 UI 范围校验和 FFmpeg 数值映射的一份 CPU4 快照。
#[derive(Debug, Clone, PartialEq)]
pub struct Cpu4Snapshot {
    updates: [Cpu4FilterUpdate; 4],
}

impl Cpu4Snapshot {
    pub fn from_ui(
        brightness_percent: f64,
        contrast_percent: f64,
        saturation_percent: f64,
        hue_rotation_degrees: f64,
    ) -> Result<Self, RealtimeVideoBackendError> {
        Ok(Self {
            updates: [
                Cpu4FilterUpdate::from_ui(Cpu4Parameter::Brightness, brightness_percent)?,
                Cpu4FilterUpdate::from_ui(Cpu4Parameter::Contrast, contrast_percent)?,
                Cpu4FilterUpdate::from_ui(Cpu4Parameter::Saturation, saturation_percent)?,
                Cpu4FilterUpdate::from_ui(Cpu4Parameter::Hue, hue_rotation_degrees)?,
            ],
        })
    }

    /// FFmpeg 的 eq/hue 每条命令只能更新一个选项，所以四条是完整快照的最小集合。
    pub fn commands(&self) -> [MpvCommand; 4] {
        self.updates
            .map(|update| MpvCommand::UpdateCpu4Filter { update })
    }
}

/// 安全 IPC 命令只允许写固定的 mpv 属性，不接受任意命令名或脚本。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MpvCommand {
    LoadFileReplace {
        media_path: PathBuf,
        source_start_ms: u64,
    },
    SetShaderOptions {
        options: MpvShaderOptions,
    },
    InstallCpu4FilterChain,
    UpdateCpu4Filter {
        update: Cpu4FilterUpdate,
    },
    SetPause {
        paused: bool,
    },
    SetPlaybackSpeed {
        speed: MpvPlaybackSpeed,
    },
    SeekAbsoluteMs {
        position_ms: u64,
    },
    GetVideoOutputConfigured,
    GetHardwareDecoderCurrent,
    GetVideoOutputPasses,
    GetFrameDropCount,
    GetDecoderFrameDropCount,
    GetMistimedFrameCount,
    GetVideoOutputDelayedFrameCount,
    GetPlaybackTime,
    GetEstimatedVideoFps,
    GetEofReached,
    GetPaused,
    GetSeeking,
    GetPausedForCache,
    GetMediaPath,
    GetShaderOptions,
    Quit,
}

impl MpvCommand {
    pub fn load_file_replace(
        selected_media_path: &Path,
        source_start_ms: u64,
    ) -> Result<Self, RealtimeVideoBackendError> {
        Ok(Self::LoadFileReplace {
            media_path: canonical_media_file(selected_media_path)?,
            source_start_ms,
        })
    }

    fn load_file_path(&self) -> Option<PathBuf> {
        match self {
            Self::LoadFileReplace { media_path, .. } => Some(media_path.clone()),
            _ => None,
        }
    }

    pub(crate) fn operation_name(&self) -> &'static str {
        match self {
            Self::LoadFileReplace { .. } => "loadfile replace",
            Self::SetShaderOptions { .. } => "set glsl-shader-opts",
            Self::InstallCpu4FilterChain => "install cpu4 filter",
            Self::UpdateCpu4Filter { .. } => "update cpu4 filter",
            Self::SetPause { .. } => "set pause",
            Self::SetPlaybackSpeed { .. } => "set speed",
            Self::SeekAbsoluteMs { .. } => "seek absolute",
            Self::GetVideoOutputConfigured => "get vo-configured",
            Self::GetHardwareDecoderCurrent => "get hwdec-current",
            Self::GetVideoOutputPasses => "get vo-passes",
            Self::GetFrameDropCount => "get frame-drop-count",
            Self::GetDecoderFrameDropCount => "get decoder-frame-drop-count",
            Self::GetMistimedFrameCount => "get mistimed-frame-count",
            Self::GetVideoOutputDelayedFrameCount => "get vo-delayed-frame-count",
            Self::GetPlaybackTime => "get time-pos",
            Self::GetEstimatedVideoFps => "get estimated-vf-fps",
            Self::GetEofReached => "get eof-reached",
            Self::GetPaused => "get pause",
            Self::GetSeeking => "get seeking",
            Self::GetPausedForCache => "get paused-for-cache",
            Self::GetMediaPath => "get path",
            Self::GetShaderOptions => "get glsl-shader-opts",
            Self::Quit => "quit",
        }
    }

    pub fn ipc_json_line(&self) -> Result<String, RealtimeVideoBackendError> {
        self.ipc_json_line_inner(None)
    }

    pub fn ipc_json_line_with_request_id(
        &self,
        request_id: u64,
    ) -> Result<String, RealtimeVideoBackendError> {
        self.ipc_json_line_inner(Some(request_id))
    }

    fn ipc_json_line_inner(
        &self,
        request_id: Option<u64>,
    ) -> Result<String, RealtimeVideoBackendError> {
        let command = match self {
            Self::LoadFileReplace {
                media_path,
                source_start_ms,
            } => {
                let media_path = canonical_media_file(media_path)?;
                let start = format!("{}.{:03}", source_start_ms / 1_000, source_start_ms % 1_000);
                serde_json::json!(["loadfile", media_path, "replace", -1, { "start": start }])
            }
            Self::SetShaderOptions { options } => {
                serde_json::json!(["set_property", "glsl-shader-opts", options.as_str()])
            }
            Self::InstallCpu4FilterChain => {
                serde_json::json!(["vf", "add", CPU4_FILTER_CHAIN])
            }
            Self::UpdateCpu4Filter { update } => {
                let update = update.validate()?;
                serde_json::json!([
                    "vf-command",
                    CPU4_MPV_FILTER_LABEL,
                    update.parameter.command(),
                    update.value.to_string(),
                    update.parameter.target()
                ])
            }
            Self::SetPause { paused } => {
                serde_json::json!(["set_property", "pause", paused])
            }
            Self::SetPlaybackSpeed { speed } => {
                serde_json::json!(["set_property", "speed", speed.as_f64()])
            }
            Self::SeekAbsoluteMs { position_ms } => {
                serde_json::json!(["seek", *position_ms as f64 / 1_000.0, "absolute+exact"])
            }
            Self::GetVideoOutputConfigured => {
                serde_json::json!(["get_property", "vo-configured"])
            }
            Self::GetHardwareDecoderCurrent => {
                serde_json::json!(["get_property", "hwdec-current"])
            }
            Self::GetVideoOutputPasses => serde_json::json!(["get_property", "vo-passes"]),
            Self::GetFrameDropCount => serde_json::json!(["get_property", "frame-drop-count"]),
            Self::GetDecoderFrameDropCount => {
                serde_json::json!(["get_property", "decoder-frame-drop-count"])
            }
            Self::GetMistimedFrameCount => {
                serde_json::json!(["get_property", "mistimed-frame-count"])
            }
            Self::GetVideoOutputDelayedFrameCount => {
                serde_json::json!(["get_property", "vo-delayed-frame-count"])
            }
            Self::GetPlaybackTime => {
                serde_json::json!(["get_property", "time-pos"])
            }
            Self::GetEstimatedVideoFps => {
                serde_json::json!(["get_property", "estimated-vf-fps"])
            }
            Self::GetEofReached => serde_json::json!(["get_property", "eof-reached"]),
            Self::GetPaused => serde_json::json!(["get_property", "pause"]),
            Self::GetSeeking => serde_json::json!(["get_property", "seeking"]),
            Self::GetPausedForCache => {
                serde_json::json!(["get_property", "paused-for-cache"])
            }
            Self::GetMediaPath => serde_json::json!(["get_property", "path"]),
            Self::GetShaderOptions => {
                serde_json::json!(["get_property", "glsl-shader-opts"])
            }
            Self::Quit => serde_json::json!(["quit"]),
        };
        let mut value = serde_json::json!({"command": command});
        if let Some(request_id) = request_id {
            value["request_id"] = serde_json::json!(request_id);
        }
        serde_json::to_string(&value)
            .map(|mut line| {
                line.push('\n');
                line
            })
            .map_err(|error| RealtimeVideoBackendError::SerializeCommand(error.to_string()))
    }
}

/// `glsl-shader-opts` 的规范化读回。键值、数量和总大小均受限，未知或重复项 fail-closed。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MpvShaderOptionMap(BTreeMap<String, String>);

impl MpvShaderOptionMap {
    pub fn parse(value: &str) -> Result<Self, RealtimeVideoBackendError> {
        if value.len() > SHADER_OPTIONS_MAX_BYTES {
            return Err(RealtimeVideoBackendError::IpcProtocol(
                "mpv shader 参数读回超过大小上限".to_owned(),
            ));
        }
        let mut values = BTreeMap::new();
        if value.is_empty() {
            return Ok(Self(values));
        }
        for entry in value.split(',') {
            if values.len() >= SHADER_OPTIONS_MAX_ENTRIES {
                return Err(RealtimeVideoBackendError::IpcProtocol(
                    "mpv shader 参数读回超过条目上限".to_owned(),
                ));
            }
            let (key, option_value) = entry.split_once('=').ok_or_else(|| {
                RealtimeVideoBackendError::IpcProtocol("mpv shader 参数读回条目缺少等号".to_owned())
            })?;
            validate_shader_option_key(key)?;
            let option_value = normalize_shader_option_value(option_value)?;
            if values.insert(key.to_owned(), option_value).is_some() {
                return Err(RealtimeVideoBackendError::IpcProtocol(
                    "mpv shader 参数读回包含重复键".to_owned(),
                ));
            }
        }
        Ok(Self(values))
    }

    fn from_response(response: &serde_json::Value) -> Result<Self, RealtimeVideoBackendError> {
        if response.get("error").and_then(serde_json::Value::as_str) != Some("success") {
            return Err(RealtimeVideoBackendError::IpcProtocol(
                "mpv glsl-shader-opts 响应未明确成功".to_owned(),
            ));
        }
        let data = response.get("data").ok_or_else(|| {
            RealtimeVideoBackendError::IpcProtocol("mpv glsl-shader-opts 响应缺少 data".to_owned())
        })?;
        if let Some(value) = data.as_str() {
            return Self::parse(value);
        }
        let object = data.as_object().ok_or_else(|| {
            RealtimeVideoBackendError::IpcProtocol(
                "mpv glsl-shader-opts 响应 data 必须是字符串或对象".to_owned(),
            )
        })?;
        if object.len() > SHADER_OPTIONS_MAX_ENTRIES
            || serde_json::to_vec(object)
                .map_err(|error| RealtimeVideoBackendError::IpcProtocol(error.to_string()))?
                .len()
                > SHADER_OPTIONS_MAX_BYTES
        {
            return Err(RealtimeVideoBackendError::IpcProtocol(
                "mpv shader 参数读回超过上限".to_owned(),
            ));
        }
        let mut values = BTreeMap::new();
        for (key, value) in object {
            validate_shader_option_key(key)?;
            let normalized = match value {
                serde_json::Value::String(value) => normalize_shader_option_value(value)?,
                serde_json::Value::Number(value) => {
                    normalize_shader_option_value(&value.to_string())?
                }
                _ => {
                    return Err(RealtimeVideoBackendError::IpcProtocol(
                        "mpv shader 参数对象包含非标量值".to_owned(),
                    ))
                }
            };
            values.insert(key.clone(), normalized);
        }
        Ok(Self(values))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    pub fn as_map(&self) -> &BTreeMap<String, String> {
        &self.0
    }
}

fn validate_shader_option_key(key: &str) -> Result<(), RealtimeVideoBackendError> {
    if key.is_empty()
        || key.len() > SHADER_OPTION_KEY_MAX_BYTES
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(RealtimeVideoBackendError::IpcProtocol(
            "mpv shader 参数读回包含非法键".to_owned(),
        ));
    }
    Ok(())
}

fn normalize_shader_option_value(value: &str) -> Result<String, RealtimeVideoBackendError> {
    if value.is_empty()
        || value.len() > SHADER_OPTION_VALUE_MAX_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+' | b'.'))
    {
        return Err(RealtimeVideoBackendError::IpcProtocol(
            "mpv shader 参数读回包含非法值".to_owned(),
        ));
    }
    match value.parse::<f64>() {
        Ok(number) if number.is_finite() => Ok(number.to_string()),
        Ok(_) => Err(RealtimeVideoBackendError::IpcProtocol(
            "mpv shader 参数读回包含非有限数值".to_owned(),
        )),
        Err(_) => Ok(value.to_owned()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MpvShaderOptions(String);

impl MpvShaderOptions {
    pub fn parse(value: String) -> Result<Self, RealtimeVideoBackendError> {
        if value.is_empty() || value.len() > SHADER_OPTIONS_MAX_BYTES {
            return Err(RealtimeVideoBackendError::IpcProtocol(
                "shader 参数快照长度无效".to_owned(),
            ));
        }
        if !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+' | b'.' | b',' | b'=')
        }) {
            return Err(RealtimeVideoBackendError::IpcProtocol(
                "shader 参数快照包含非法字符".to_owned(),
            ));
        }
        MpvShaderOptionMap::parse(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn option_map(&self) -> Result<MpvShaderOptionMap, RealtimeVideoBackendError> {
        MpvShaderOptionMap::parse(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledRealtimeParameters {
    pub commands: Vec<MpvCommand>,
    pub support: ParameterSupportReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoPlanSlot {
    N,
    NPlus1,
    NPlus2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoPlanIdentity {
    pub session_id: u64,
    pub playback_generation: u64,
    pub source_revision: u64,
    pub parameter_revision: u64,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeVideoPlan {
    pub slot: VideoPlanSlot,
    pub identity: VideoPlanIdentity,
    pub target_pts_ms: u64,
    pub period_ms: u64,
    pub seed: u64,
    pub prepared: bool,
    pub commands: Vec<MpvCommand>,
    pub parameter_support: ParameterSupportReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeVideoPlanQueue {
    pub n: Option<RealtimeVideoPlan>,
    pub n_plus_1: Option<RealtimeVideoPlan>,
    pub n_plus_2: Option<RealtimeVideoPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoCommitGate {
    pub identity: VideoPlanIdentity,
    pub media_pts_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCommitDecision {
    Commit,
    WrongSlot,
    NotPrepared,
    StaleIdentity,
    TooEarly,
}

impl RealtimeVideoPlan {
    pub fn commit_decision(&self, gate: &VideoCommitGate) -> VideoCommitDecision {
        if self.slot != VideoPlanSlot::NPlus1 {
            return VideoCommitDecision::WrongSlot;
        }
        if !self.prepared {
            return VideoCommitDecision::NotPrepared;
        }
        if self.identity != gate.identity {
            return VideoCommitDecision::StaleIdentity;
        }
        if gate.media_pts_ms < self.target_pts_ms {
            return VideoCommitDecision::TooEarly;
        }
        VideoCommitDecision::Commit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[cfg(windows)]
    use std::path::Path;
    #[cfg(windows)]
    use std::sync::atomic::{AtomicU64, Ordering};
    #[cfg(windows)]
    use std::time::{SystemTime, UNIX_EPOCH};
    #[cfg(windows)]
    use sysinfo::{Pid, ProcessesToUpdate, System};

    #[cfg(windows)]
    const JOB_TREE_FIXTURE_ROOT_ENV: &str = "AUTOLIVE_MPV_JOB_TREE_FIXTURE_ROOT";
    #[cfg(windows)]
    const JOB_TREE_FIXTURE_TIMEOUT: Duration = Duration::from_secs(5);
    #[cfg(windows)]
    static NEXT_JOB_TREE_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn physical_resume_requires_unpaused_non_eof_source_local_pts_progress() {
        let first = PlaybackResumeFact {
            paused: false,
            eof_reached: false,
            seeking: false,
            source_pts_ms: 120,
        };
        let progressed = PlaybackResumeFact {
            source_pts_ms: 153,
            ..first
        };

        assert!(physical_playback_resumed(first, progressed, 1_000));
        assert!(!physical_playback_resumed(
            first,
            PlaybackResumeFact {
                paused: true,
                ..progressed
            },
            1_000,
        ));
        assert!(!physical_playback_resumed(
            first,
            PlaybackResumeFact {
                eof_reached: true,
                ..progressed
            },
            1_000,
        ));
        assert!(!physical_playback_resumed(first, first, 1_000));
        assert!(!physical_playback_resumed(
            first,
            PlaybackResumeFact {
                source_pts_ms: 1_000,
                ..progressed
            },
            1_000,
        ));
    }

    #[test]
    fn composite_ipc_reads_share_one_total_deadline() {
        let total = Duration::from_millis(250);
        let started = Instant::now() - Duration::from_millis(100);
        let remaining = remaining_ipc_deadline(started, total);
        assert!(remaining <= Duration::from_millis(150));
        assert!(remaining > Duration::ZERO);

        let expired = remaining_ipc_deadline(Instant::now() - total, total);
        assert_eq!(expired, Duration::from_millis(1));
    }

    #[test]
    fn loadfile_replace_is_structured_and_canonicalizes_a_regular_file() {
        let root = temporary_root("loadfile-command");
        fs::create_dir_all(&root).expect("create media root");
        let media = root.join("--next source.mp4");
        fs::write(&media, b"media").expect("create media");

        let line = MpvCommand::load_file_replace(&media, 438)
            .expect("validate local media")
            .ipc_json_line()
            .expect("serialize loadfile");
        let command = serde_json::from_str::<serde_json::Value>(&line)
            .expect("parse loadfile command")["command"]
            .clone();
        assert_eq!(
            command,
            serde_json::json!([
                "loadfile",
                media
                    .canonicalize()
                    .expect("canonical media")
                    .to_string_lossy(),
                "replace",
                -1,
                { "start": "0.438" }
            ])
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &MpvCommand::GetMediaPath
                    .ipc_json_line()
                    .expect("serialize path read")
            )
            .expect("parse path read")["command"],
            serde_json::json!(["get_property", "path"])
        );

        assert!(MpvCommand::load_file_replace(&root, 0).is_err());
        assert!(MpvCommand::load_file_replace(&root.join("missing.mp4"), 0).is_err());
        let _ignored = fs::remove_dir_all(root);
    }

    #[test]
    fn media_switch_restores_pause_state_after_loadfile_submission() {
        let source = include_str!("realtime_video_backend.rs");
        let switch = source
            .split("pub fn switch_media_source<C>")
            .nth(1)
            .expect("media switch function")
            .split("pub fn stderr_tail")
            .next()
            .expect("media switch function end");
        let loadfile = switch
            .find("self.send_command(&command")
            .expect("loadfile submission");
        let pause = switch
            .find("&MpvCommand::SetPause { paused }")
            .expect("pause restoration");
        let readiness = switch
            .find("wait_for_loaded_media(")
            .expect("media readiness barrier");
        assert!(loadfile < pause);
        assert!(pause < readiness);
    }

    #[test]
    fn media_switch_waits_for_matching_path_non_eof_non_seeking_vo_pts_and_rendered_frame() {
        let root = temporary_root("loadfile-facts");
        fs::create_dir_all(&root).expect("create media root");
        let old_media = root.join("old.mp4");
        let next_media = root.join("next.mp4");
        fs::write(&old_media, b"old").expect("create old media");
        fs::write(&next_media, b"next").expect("create next media");
        let expected = canonical_media_file(&next_media).expect("canonical next media");
        let old = old_media
            .canonicalize()
            .expect("canonical old media")
            .to_string_lossy()
            .into_owned();
        let next = expected.to_string_lossy().into_owned();
        let mut responses = VecDeque::from([
            serde_json::json!({"error":"success","data":old}),
            serde_json::json!({"error":"success","data":next}),
            serde_json::json!({"error":"success","data":false}),
            serde_json::json!({"error":"success","data":false}),
            serde_json::json!({"error":"success","data":false}),
            serde_json::json!({"error":"success","data":true}),
            serde_json::json!({"error":"success","data":0.0}),
            serde_json::json!({"error":"success","data":{"fresh":[{"count":1,"samples":[1000]}]}}),
            serde_json::json!({"error":"success","data":false}),
            serde_json::json!({"error":"success","data":false}),
            serde_json::json!({"error":"success","data":false}),
            serde_json::json!({"error":"success","data":0.0}),
            serde_json::json!({"error":"success","data":false}),
            serde_json::json!({"error":"success","data":false}),
            serde_json::json!({"error":"success","data":false}),
            serde_json::json!({"error":"success","data":0.033}),
        ]);
        let mut operations = Vec::new();
        let budget = Duration::from_secs(1);
        let started = Instant::now();

        wait_for_loaded_media(
            &expected,
            1_000,
            started,
            budget,
            false,
            || false,
            |command, deadline| {
                assert!(deadline <= budget);
                assert!(!deadline.is_zero());
                operations.push(command.operation_name());
                Ok(responses.pop_front().expect("scripted mpv response"))
            },
        )
        .expect("new media must become ready");

        assert_eq!(
            operations,
            [
                "get path",
                "get path",
                "get pause",
                "get eof-reached",
                "get seeking",
                "get vo-configured",
                "get time-pos",
                "get vo-passes",
                "get pause",
                "get eof-reached",
                "get seeking",
                "get time-pos",
                "get pause",
                "get eof-reached",
                "get seeking",
                "get time-pos"
            ]
        );
        assert!(responses.is_empty());
        let _ignored = fs::remove_dir_all(root);
    }

    #[test]
    fn media_switch_timeout_does_not_issue_an_unbudgeted_ipc_request() {
        let root = temporary_root("loadfile-timeout");
        fs::create_dir_all(&root).expect("create media root");
        let media = root.join("next.mp4");
        fs::write(&media, b"next").expect("create next media");
        let expected = canonical_media_file(&media).expect("canonical media");

        let error = wait_for_loaded_media(
            &expected,
            1_000,
            Instant::now(),
            Duration::ZERO,
            false,
            || false,
            |_, _| panic!("expired switch must not send IPC"),
        )
        .expect_err("zero budget must time out");
        assert!(matches!(
            error,
            RealtimeVideoBackendError::MediaSwitchTimeout
        ));
        let _ignored = fs::remove_dir_all(root);
    }

    #[test]
    fn media_switch_cancellation_prevents_all_further_ipc() {
        let root = temporary_root("loadfile-cancel");
        fs::create_dir_all(&root).expect("create media root");
        let media = root.join("next.mp4");
        fs::write(&media, b"next").expect("create next media");
        let expected = canonical_media_file(&media).expect("canonical media");

        let error = wait_for_loaded_media(
            &expected,
            1_000,
            Instant::now(),
            Duration::from_secs(1),
            false,
            || true,
            |_, _| panic!("cancelled switch must not send IPC"),
        )
        .expect_err("cancelled switch must stop");
        assert!(matches!(
            error,
            RealtimeVideoBackendError::MediaSwitchCancelled
        ));
        let _ignored = fs::remove_dir_all(root);
    }

    #[test]
    fn stderr_redaction_covers_initial_and_later_media_sources() {
        let root = temporary_root("stderr-redaction");
        fs::create_dir_all(&root).expect("create media root");
        let initial = root.join("initial.mp4");
        let later = root.join("later.ts");
        fs::write(&initial, b"initial").expect("create initial media");
        fs::write(&later, b"later").expect("create later media");
        let mut paths = RedactedMediaPaths::default();
        paths.register(&initial.canonicalize().expect("canonical initial"));
        paths.register(&later.canonicalize().expect("canonical later"));

        let line = format!(
            "failed {} then {}",
            initial.canonicalize().expect("canonical initial").display(),
            later.canonicalize().expect("canonical later").display()
        );
        let redacted = paths.redact(&line);
        assert_eq!(redacted, "failed <media> then <media>");
        assert!(!redacted.contains(root.to_string_lossy().as_ref()));
        let _ignored = fs::remove_dir_all(root);
    }

    fn temporary_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "autolive-realtime-video-{name}-{}",
            std::process::id()
        ))
    }

    #[cfg(windows)]
    fn create_job_tree_fixture_root() -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos();
        let id = NEXT_JOB_TREE_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autolive-mpv-job-tree-{}-{timestamp}-{id}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("claim unique Job Object fixture directory");
        root
    }

    #[cfg(windows)]
    fn wait_for_fixture_path(path: &Path, timeout: Duration) -> bool {
        let started = Instant::now();
        loop {
            if path.exists() {
                return true;
            }
            if started.elapsed() >= timeout {
                return false;
            }
            thread::sleep(MPV_EXIT_POLL);
        }
    }

    #[cfg(windows)]
    fn process_instance_start_time(pid: u32) -> Option<u64> {
        let pid = Pid::from_u32(pid);
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        system.process(pid).map(|process| process.start_time())
    }

    #[cfg(windows)]
    fn wait_for_process_instance_exit(pid: u32, start_time: u64, timeout: Duration) -> bool {
        let pid = Pid::from_u32(pid);
        let mut system = System::new();
        let started = Instant::now();
        loop {
            system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
            if system
                .process(pid)
                .map(|process| process.start_time() != start_time)
                .unwrap_or(true)
            {
                return true;
            }
            if started.elapsed() >= timeout {
                return false;
            }
            thread::sleep(MPV_EXIT_POLL);
        }
    }

    #[cfg(windows)]
    struct ManagedJobTreeFixture {
        root: PathBuf,
        parent: Option<Child>,
        job: Option<ManagedMpvJob>,
        descendant: Option<(u32, u64)>,
    }

    #[cfg(windows)]
    impl Drop for ManagedJobTreeFixture {
        fn drop(&mut self) {
            drop(self.job.take());
            if let Some(parent) = self.parent.as_mut() {
                if !wait_for_child_exit(parent, MPV_EXIT_GRACE).unwrap_or(false) {
                    let _ignored = terminate_unmanaged_child(parent);
                }
            }
            let descendant = self.descendant.or_else(|| {
                fs::read_to_string(self.root.join("descendant.pid"))
                    .ok()?
                    .parse::<u32>()
                    .ok()
                    .and_then(|pid| process_instance_start_time(pid).map(|started| (pid, started)))
            });
            if let Some((pid, start_time)) = descendant {
                if !wait_for_process_instance_exit(pid, start_time, MPV_EXIT_GRACE) {
                    let pid = Pid::from_u32(pid);
                    let mut system = System::new();
                    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
                    if let Some(process) = system
                        .process(pid)
                        .filter(|process| process.start_time() == start_time)
                    {
                        let _ignored = process.kill();
                    }
                    let _ignored =
                        wait_for_process_instance_exit(pid.as_u32(), start_time, MPV_EXIT_GRACE);
                }
            }
            let _ignored = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn packaged_mpv_must_resolve_inside_the_verified_root() {
        let root = temporary_root("packaged-path");
        let binaries = root.join("binaries");
        fs::create_dir_all(&binaries).expect("create test binaries directory");
        fs::write(binaries.join("mpv.exe"), b"mpv").expect("create test mpv");

        let resolved = resolve_mpv_executable_from(&root, None, false).expect("resolve mpv");
        assert!(resolved
            .as_path()
            .starts_with(root.canonicalize().expect("canonical root")));
        let _ignored = fs::remove_dir_all(root);
    }

    #[test]
    fn release_resolution_ignores_development_override() {
        let root = temporary_root("release-path");
        let external = temporary_root("external-path").join("mpv.exe");
        fs::create_dir_all(root.join("binaries")).expect("create packaged binaries");
        fs::create_dir_all(external.parent().expect("external parent"))
            .expect("create external directory");
        fs::write(root.join("binaries/mpv.exe"), b"packaged").expect("create packaged mpv");
        fs::write(&external, b"external").expect("create external mpv");

        let resolved = resolve_mpv_executable_from(&root, Some(external.as_os_str()), false)
            .expect("release resolution");
        assert_eq!(
            resolved.as_path(),
            root.join("binaries/mpv.exe")
                .canonicalize()
                .expect("canonical packaged mpv")
        );
        let _ignored = fs::remove_dir_all(root);
        let _ignored = fs::remove_dir_all(external.parent().expect("external parent"));
    }

    #[test]
    fn launch_arguments_are_fixed_and_media_path_cannot_be_parsed_as_an_option() {
        let root = temporary_root("launch-args");
        fs::create_dir_all(root.join("binaries")).expect("create binaries");
        fs::write(root.join("binaries/mpv.exe"), b"mpv").expect("create mpv");
        let media = root.join("--script=untrusted.lua.mp4");
        fs::write(&media, b"media").expect("create media");
        let executable =
            resolve_mpv_executable_from(&root, None, false).expect("resolve packaged mpv");

        let spec = MpvLaunchSpec::new(
            executable.clone(),
            &media,
            42,
            r"\\.\pipe\autolive-mpv-test_1",
            MpvLaunchMode::Original,
            1_234,
            false,
        )
        .expect("valid launch spec");

        assert!(spec.arguments().contains(&"--no-config".to_owned()));
        assert!(!spec.arguments().contains(&"--load-scripts=no".to_owned()));
        assert!(!spec.arguments().contains(&"--osc=no".to_owned()));
        assert!(spec.arguments().contains(&"--vo=gpu-next".to_owned()));
        assert!(spec.arguments().contains(&"--hwdec=d3d11va".to_owned()));
        assert!(spec.arguments().contains(&"--audio=no".to_owned()));
        assert!(spec.arguments().contains(&"--keep-open=yes".to_owned()));
        assert!(!spec.arguments().contains(&"--loop-file=inf".to_owned()));
        assert!(spec.arguments().contains(&"--start=1.234".to_owned()));
        assert!(spec.arguments().contains(&"--pause=yes".to_owned()));
        assert!(!spec.arguments().contains(&"--pause=no".to_owned()));
        assert!(spec.arguments().contains(&"--wid=42".to_owned()));
        assert_eq!(spec.arguments()[spec.arguments().len() - 2], "--");
        assert_eq!(
            spec.arguments().last(),
            Some(
                &media
                    .canonicalize()
                    .expect("canonical media")
                    .to_string_lossy()
                    .into_owned()
            )
        );

        assert!(matches!(
            MpvLaunchSpec::new(
                executable.clone(),
                &media,
                u64::from(u32::MAX) + 1,
                r"\\.\pipe\autolive-mpv-overflow",
                MpvLaunchMode::Original,
                0,
                true,
            ),
            Err(RealtimeVideoBackendError::InvalidHostWindow)
        ));

        let paused_spec = MpvLaunchSpec::new(
            executable,
            &media,
            42,
            r"\\.\pipe\autolive-mpv-test_2",
            MpvLaunchMode::Gpu(MpvGpuProfile::VulkanCopy),
            0,
            true,
        )
        .expect("valid paused launch spec");
        assert!(paused_spec.arguments().contains(&"--audio=no".to_owned()));
        assert!(paused_spec
            .arguments()
            .contains(&"--start=0.000".to_owned()));
        assert!(paused_spec.arguments().contains(&"--pause=yes".to_owned()));
        assert!(paused_spec
            .arguments()
            .contains(&"--hwdec=d3d11va-copy".to_owned()));
    }

    #[test]
    fn cross_vendor_launch_modes_fix_gpu_cpu4_and_original_arguments() {
        let root = temporary_root("phase4-launch-modes");
        fs::create_dir_all(root.join("binaries")).expect("create binaries");
        fs::write(root.join("binaries/mpv.exe"), b"mpv").expect("create mpv");
        let media = root.join("source.mp4");
        fs::write(&media, b"media").expect("create media");
        let executable =
            resolve_mpv_executable_from(&root, None, false).expect("resolve packaged mpv");
        let modes = MpvLaunchMode::fallback_order();
        let specs = modes.map(|mode| {
            MpvLaunchSpec::new(
                executable.clone(),
                &media,
                42,
                &format!(r"\\.\pipe\autolive-mpv-mode-{}", mode.name()),
                mode,
                0,
                false,
            )
            .expect("valid cross-vendor launch mode")
        });

        assert_eq!(
            modes,
            [
                MpvLaunchMode::Gpu(MpvGpuProfile::D3d11ZeroCopy),
                MpvLaunchMode::Gpu(MpvGpuProfile::D3d11Copy),
                MpvLaunchMode::Gpu(MpvGpuProfile::VulkanCopy),
                MpvLaunchMode::Gpu(MpvGpuProfile::SoftwareDecode),
                MpvLaunchMode::Cpu4,
                MpvLaunchMode::Original,
            ]
        );
        assert_eq!(specs.each_ref().map(|spec| spec.mode()), modes);
        assert!(specs[0].arguments().contains(&"--hwdec=d3d11va".to_owned()));
        assert!(specs[1]
            .arguments()
            .contains(&"--hwdec=d3d11va-copy".to_owned()));
        assert!(specs[1].arguments().contains(&"--gpu-api=d3d11".to_owned()));
        assert!(specs[2]
            .arguments()
            .contains(&"--hwdec=d3d11va-copy".to_owned()));
        assert!(specs[2]
            .arguments()
            .contains(&"--gpu-api=vulkan".to_owned()));
        assert!(specs[3].arguments().contains(&"--hwdec=no".to_owned()));
        assert!(specs[3].arguments().contains(&"--gpu-api=d3d11".to_owned()));
        assert!(!specs[3]
            .arguments()
            .iter()
            .any(|argument| argument.starts_with("--vf=")));
        let cpu4 = specs[4].arguments();
        assert!(cpu4.contains(&"--vo=gpu-next".to_owned()));
        assert!(cpu4.contains(&"--gpu-api=d3d11".to_owned()));
        assert!(cpu4.contains(&"--hwdec=no".to_owned()));
        assert!(cpu4.contains(&format!("--vf={CPU4_FILTER_CHAIN}")));
        assert!(!cpu4
            .iter()
            .any(|argument| argument.starts_with("--glsl-shaders=")));

        let original = specs[5].arguments();
        assert!(original.contains(&"--vo=gpu-next".to_owned()));
        assert!(original.contains(&"--gpu-api=d3d11".to_owned()));
        assert!(original.contains(&"--hwdec=d3d11va".to_owned()));
        assert!(!original
            .iter()
            .any(|argument| argument.starts_with("--vf=")));
        assert!(!original
            .iter()
            .any(|argument| argument.starts_with("--glsl-shaders=")));

        let _ignored = fs::remove_dir_all(root);
    }

    #[test]
    fn launch_arguments_explicitly_load_only_a_verified_shader() {
        let root = temporary_root("launch-shader");
        fs::create_dir_all(root.join("binaries")).expect("create binaries");
        fs::create_dir_all(root.join("shaders")).expect("create shaders");
        fs::write(root.join("binaries/mpv.exe"), b"mpv").expect("create mpv");
        fs::write(root.join("shaders/gpu83.hook"), b"//!HOOK MAIN").expect("create shader");
        let media = root.join("source.mp4");
        fs::write(&media, b"media").expect("create media");
        let executable =
            resolve_mpv_executable_from(&root, None, false).expect("resolve packaged mpv");
        let shader = resolve_mpv_shader(&root, &root.join("shaders/gpu83.hook"))
            .expect("resolve packaged shader");

        let spec = MpvLaunchSpec::new_with_shader(
            executable,
            &media,
            42,
            r"\\.\pipe\autolive-mpv-shader_1",
            MpvGpuProfile::D3d11ZeroCopy,
            0,
            false,
            shader,
        )
        .expect("valid shader launch spec");

        let expected = format!(
            "--glsl-shaders={}",
            root.join("shaders/gpu83.hook")
                .canonicalize()
                .expect("canonical shader")
                .display()
        );
        assert!(spec.arguments().contains(&expected));

        let external = temporary_root("external-shader").join("outside.hook");
        fs::create_dir_all(external.parent().expect("external parent"))
            .expect("create external directory");
        fs::write(&external, b"//!HOOK MAIN").expect("create external shader");
        assert!(matches!(
            resolve_mpv_shader(&root, &external),
            Err(RealtimeVideoBackendError::ResourceEscapesRoot { .. })
        ));

        let _ignored = fs::remove_dir_all(root);
        let _ignored = fs::remove_dir_all(external.parent().expect("external parent"));
    }

    #[test]
    fn ipc_commands_only_serialize_the_fixed_allowlist() {
        let pause = MpvCommand::SetPause { paused: true }
            .ipc_json_line()
            .expect("serialize pause");
        let seek = MpvCommand::SeekAbsoluteMs { position_ms: 1_234 }
            .ipc_json_line()
            .expect("serialize seek");
        let quit = MpvCommand::Quit.ipc_json_line().expect("serialize quit");
        let shader_readback = MpvCommand::GetShaderOptions
            .ipc_json_line()
            .expect("serialize shader readback");
        let shader = MpvCommand::SetShaderOptions {
            options: MpvShaderOptions::parse(
                "al_brightness_percent=1.25,al_noise_percent=0.5".to_owned(),
            )
            .expect("validated shader options"),
        }
        .ipc_json_line()
        .expect("serialize shader options");

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&pause).expect("parse pause")["command"],
            serde_json::json!(["set_property", "pause", true])
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&seek).expect("parse seek")["command"],
            serde_json::json!(["seek", 1.234, "absolute+exact"])
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&quit).expect("parse quit")["command"],
            serde_json::json!(["quit"])
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&shader).expect("parse shader options")
                ["command"],
            serde_json::json!([
                "set_property",
                "glsl-shader-opts",
                "al_brightness_percent=1.25,al_noise_percent=0.5"
            ])
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&shader_readback)
                .expect("parse shader readback")["command"],
            serde_json::json!(["get_property", "glsl-shader-opts"])
        );
        assert!(MpvShaderOptions::parse("ok=1\nquit".to_owned()).is_err());

        let identified = MpvCommand::GetVideoOutputConfigured
            .ipc_json_line_with_request_id(73)
            .expect("serialize identified request");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&identified)
                .expect("parse identified request")["request_id"],
            serde_json::json!(73)
        );
    }

    #[test]
    fn shader_option_readback_is_bounded_and_compares_as_a_map() {
        let expected = MpvShaderOptions::parse(
            "al_runtime_plan_hi=12,al_runtime_source_fps=30.0,al_runtime_plan_lo=34".to_owned(),
        )
        .expect("valid expected options");
        let reordered = serde_json::json!({
            "error": "success",
            "data": "al_runtime_plan_lo=34,al_runtime_plan_hi=12,al_runtime_source_fps=30.0"
        });
        let matching = MpvShaderOptionMap::from_response(&reordered).expect("valid readback");
        assert_eq!(matching, expected.option_map().expect("expected map"));

        let object_readback = MpvShaderOptionMap::from_response(&serde_json::json!({
            "error": "success",
            "data": {
                "al_runtime_plan_lo": 34,
                "al_runtime_plan_hi": "12",
                "al_runtime_source_fps": 30
            }
        }))
        .expect("mpv object readback");
        assert_eq!(object_readback, matching);

        let mismatched = MpvShaderOptionMap::parse(
            "al_runtime_plan_hi=12,al_runtime_plan_lo=35,al_runtime_source_fps=30.0",
        )
        .expect("valid mismatched readback");
        assert_ne!(matching, mismatched);
        assert_eq!(matching.get("al_runtime_source_fps"), Some("30"));

        assert!(MpvShaderOptionMap::parse("al_runtime_plan_hi=12,al_runtime_plan_hi=13").is_err());
        assert!(MpvShaderOptionMap::from_response(&serde_json::json!({
            "error": "success",
            "data": {"al_runtime_plan_hi": [12]}
        }))
        .is_err());
        assert!(MpvShaderOptionMap::parse(&"x".repeat(SHADER_OPTIONS_MAX_BYTES + 1)).is_err());
    }

    #[test]
    fn phase3b_commands_serialize_to_the_fixed_mpv_properties() {
        let speed = MpvCommand::SetPlaybackSpeed {
            speed: MpvPlaybackSpeed::new(1.25).expect("valid playback speed"),
        }
        .ipc_json_line()
        .expect("serialize playback speed");
        let time = MpvCommand::GetPlaybackTime
            .ipc_json_line()
            .expect("serialize full playback time read");
        let fps = MpvCommand::GetEstimatedVideoFps
            .ipc_json_line()
            .expect("serialize estimated FPS read");
        let eof = MpvCommand::GetEofReached
            .ipc_json_line()
            .expect("serialize EOF read");
        let decoder = MpvCommand::GetHardwareDecoderCurrent
            .ipc_json_line()
            .expect("serialize actual decoder read");

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&speed).expect("parse speed command")
                ["command"],
            serde_json::json!(["set_property", "speed", 1.25])
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&time).expect("parse time command")
                ["command"],
            serde_json::json!(["get_property", "time-pos"])
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&eof).expect("parse EOF command")["command"],
            serde_json::json!(["get_property", "eof-reached"])
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&fps).expect("parse FPS command")["command"],
            serde_json::json!(["get_property", "estimated-vf-fps"])
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&decoder)
                .expect("parse actual decoder command")["command"],
            serde_json::json!(["get_property", "hwdec-current"])
        );
    }

    #[test]
    fn actual_decoder_response_is_strict_and_normalizes_software() {
        for (data, expected) in [
            ("d3d11va", "d3d11va"),
            ("d3d11va-copy", "d3d11va-copy"),
            ("no", "software"),
        ] {
            assert_eq!(
                parse_hwdec_current_response(
                    &serde_json::json!({"error": "success", "data": data})
                )
                .expect("supported decoder response"),
                expected
            );
        }
        for invalid in [
            serde_json::json!({"error": "success", "data": null}),
            serde_json::json!({"error": "success", "data": "auto"}),
            serde_json::json!({"error": "success", "data": "software"}),
            serde_json::json!({"error": "success", "data": 1}),
            serde_json::json!({"error": "property-unavailable"}),
        ] {
            assert!(parse_hwdec_current_response(&invalid).is_err());
        }
    }

    #[test]
    fn eof_response_requires_an_explicit_boolean() {
        assert!(
            parse_eof_reached_response(&serde_json::json!({"error":"success","data":true}))
                .expect("true EOF")
        );
        assert!(
            !parse_eof_reached_response(&serde_json::json!({"error":"success","data":false}))
                .expect("false EOF")
        );
        for invalid in [
            serde_json::json!({"error":"success","data":null}),
            serde_json::json!({"error":"success","data":1}),
            serde_json::json!({"error":"success","data":"true"}),
            serde_json::json!({"error":"success"}),
            serde_json::json!({"error":"property-unavailable","data":false}),
        ] {
            assert!(parse_eof_reached_response(&invalid).is_err());
        }
    }

    #[test]
    fn playback_state_responses_require_explicit_booleans() {
        let state = MpvPlaybackState::from_responses(
            &serde_json::json!({"error":"success","data":true}),
            &serde_json::json!({"error":"success","data":false}),
            &serde_json::json!({"error":"success","data":true}),
        )
        .expect("strict playback state");
        assert!(state.paused);
        assert!(!state.seeking);
        assert!(state.paused_for_cache);

        for invalid in [
            serde_json::json!({"error":"success","data":null}),
            serde_json::json!({"error":"success","data":1}),
            serde_json::json!({"error":"success","data":"false"}),
            serde_json::json!({"error":"property-unavailable","data":false}),
        ] {
            assert!(MpvPlaybackState::from_responses(
                &invalid,
                &serde_json::json!({"error":"success","data":false}),
                &serde_json::json!({"error":"success","data":false}),
            )
            .is_err());
        }
    }

    #[test]
    fn playback_speed_accepts_only_finite_composed_bounds() {
        for value in [0.25, 1.0, 3.06, 4.0] {
            assert_eq!(
                MpvPlaybackSpeed::new(value)
                    .expect("speed boundary must be valid")
                    .as_f64(),
                value
            );
        }
        for value in [
            f64::from_bits(0.25_f64.to_bits() - 1),
            f64::from_bits(4.0_f64.to_bits() + 1),
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            assert!(MpvPlaybackSpeed::new(value).is_err());
        }
        assert!(serde_json::from_str::<MpvPlaybackSpeed>("0.249").is_err());
    }

    #[test]
    fn video_observation_handles_not_ready_and_rounds_pts() {
        let valid_time = serde_json::json!({"error": "success", "data": 1.2346});
        let valid_fps = serde_json::json!({"error": "success", "data": 59.94});

        assert_eq!(
            MpvVideoObservation::from_responses(&valid_time, &valid_fps)
                .expect("valid observation"),
            Some(MpvVideoObservation {
                media_pts_ms: 1_235,
                source_fps: 59.94,
            })
        );
        assert_eq!(
            MpvVideoObservation::from_responses(
                &serde_json::json!({"error": "success", "data": null}),
                &valid_fps,
            )
            .expect("time is not ready"),
            None
        );
        assert_eq!(
            MpvVideoObservation::from_responses(
                &valid_time,
                &serde_json::json!({"error": "success", "data": null}),
            )
            .expect("FPS is not ready"),
            None
        );
    }

    #[test]
    fn playback_time_parser_is_independent_from_estimated_fps() {
        assert_eq!(
            playback_time_ms_from_response(
                &serde_json::json!({"error": "success", "data": 1.2346}),
            )
            .expect("valid playback time"),
            Some(1_235)
        );
        assert_eq!(
            playback_time_ms_from_response(&serde_json::json!({"error": "success", "data": null}),)
                .expect("playback time may be temporarily unavailable"),
            None
        );
    }

    #[test]
    fn video_observation_rejects_malformed_and_out_of_range_responses() {
        fn assert_protocol_error(
            result: Result<Option<MpvVideoObservation>, RealtimeVideoBackendError>,
            expected: &str,
        ) {
            match result {
                Err(RealtimeVideoBackendError::IpcProtocol(message)) => {
                    assert_eq!(message, expected)
                }
                other => panic!("expected protocol error, got {other:?}"),
            }
        }

        let valid_time = serde_json::json!({"error": "success", "data": 1.0});
        let valid_fps = serde_json::json!({"error": "success", "data": 60.0});
        let cases = [
            (
                serde_json::json!({"error": "success"}),
                valid_fps.clone(),
                "mpv time-pos 响应缺少 data 字段",
            ),
            (
                valid_time.clone(),
                serde_json::json!({"error": "success", "data": "60"}),
                "mpv estimated-vf-fps 响应 data 必须是有限数",
            ),
            (
                serde_json::json!({"error": "success", "data": -0.001}),
                valid_fps.clone(),
                "mpv time-pos 不能为负数",
            ),
            (
                serde_json::json!({"error": "success", "data": 18_446_744_073_709_552.0}),
                valid_fps.clone(),
                "mpv time-pos 超出毫秒时间戳范围",
            ),
            (
                valid_time.clone(),
                serde_json::json!({"error": "success", "data": 0.9999}),
                "mpv estimated-vf-fps 必须在 [1, 240] 范围内",
            ),
            (
                valid_time.clone(),
                serde_json::json!({"error": "success", "data": 240.0001}),
                "mpv estimated-vf-fps 必须在 [1, 240] 范围内",
            ),
            (
                serde_json::json!({"data": 1.0}),
                valid_fps.clone(),
                "mpv time-pos 响应未明确成功",
            ),
            (
                serde_json::json!({"error": "failure", "data": 1.0}),
                valid_fps.clone(),
                "mpv time-pos 响应未明确成功",
            ),
        ];

        for (time, fps, expected) in cases {
            assert_protocol_error(MpvVideoObservation::from_responses(&time, &fps), expected);
        }
    }

    #[test]
    fn cpu4_snapshot_compiles_to_the_minimum_fixed_vf_commands() {
        let install = MpvCommand::InstallCpu4FilterChain
            .ipc_json_line()
            .expect("serialize fixed CPU4 chain");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&install).expect("parse CPU4 chain")
                ["command"],
            serde_json::json!(["vf", "add", CPU4_FILTER_CHAIN])
        );

        let snapshot =
            Cpu4Snapshot::from_ui(-25.0, 150.0, 40.0, 90.0).expect("map valid CPU4 UI values");
        let commands = snapshot
            .commands()
            .into_iter()
            .map(|command| {
                let line = command.ipc_json_line().expect("serialize CPU4 update");
                serde_json::from_str::<serde_json::Value>(&line).expect("parse CPU4 update")
                    ["command"]
                    .clone()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            commands,
            [
                serde_json::json!([
                    "vf-command",
                    "autolive_cpu4",
                    "brightness",
                    "-0.25",
                    "eq@autolive_cpu4_eq"
                ]),
                serde_json::json!([
                    "vf-command",
                    "autolive_cpu4",
                    "contrast",
                    "1.5",
                    "eq@autolive_cpu4_eq"
                ]),
                serde_json::json!([
                    "vf-command",
                    "autolive_cpu4",
                    "saturation",
                    "0.4",
                    "eq@autolive_cpu4_eq"
                ]),
                serde_json::json!([
                    "vf-command",
                    "autolive_cpu4",
                    "h",
                    "90",
                    "hue@autolive_cpu4_hue"
                ]),
            ]
        );
    }

    #[test]
    fn cpu4_gate_rejects_non_finite_out_of_range_and_unknown_parameters() {
        assert!(Cpu4Snapshot::from_ui(f64::NAN, 100.0, 100.0, 0.0).is_err());
        assert!(Cpu4Snapshot::from_ui(0.0, f64::INFINITY, 100.0, 0.0).is_err());
        assert!(Cpu4Snapshot::from_ui(-100.01, 100.0, 100.0, 0.0).is_err());
        assert!(Cpu4Snapshot::from_ui(0.0, 200.01, 100.0, 0.0).is_err());
        assert!(Cpu4Snapshot::from_ui(0.0, 100.0, -0.01, 0.0).is_err());
        assert!(Cpu4Snapshot::from_ui(0.0, 100.0, 100.0, 180.01).is_err());

        assert!(serde_json::from_str::<Cpu4Parameter>("\"drawtext\"").is_err());
        let forged = MpvCommand::UpdateCpu4Filter {
            update: Cpu4FilterUpdate {
                parameter: Cpu4Parameter::Brightness,
                value: f64::INFINITY,
            },
        };
        assert!(matches!(
            forged.ipc_json_line(),
            Err(RealtimeVideoBackendError::InvalidCpu4Parameter { .. })
        ));
    }

    #[test]
    fn backend_only_demotes_one_way_for_the_session() {
        let mut state = VideoBackendStateMachine::new();
        assert_eq!(state.current(), VideoBackend::RealtimeGpu);
        assert_eq!(
            state.launch_mode(),
            MpvLaunchMode::Gpu(MpvGpuProfile::D3d11ZeroCopy)
        );
        assert_eq!(
            state.demote("renderer initialization failed", 10),
            VideoBackend::RealtimeGpu
        );
        assert_eq!(
            state.launch_mode(),
            MpvLaunchMode::Gpu(MpvGpuProfile::D3d11Copy)
        );
        assert_eq!(
            state.demote("D3D11 copy failed", 20),
            VideoBackend::RealtimeGpu
        );
        assert_eq!(
            state.launch_mode(),
            MpvLaunchMode::Gpu(MpvGpuProfile::VulkanCopy)
        );
        assert_eq!(
            state.demote("Vulkan copy failed", 30),
            VideoBackend::RealtimeGpu
        );
        assert_eq!(
            state.launch_mode(),
            MpvLaunchMode::Gpu(MpvGpuProfile::SoftwareDecode)
        );
        assert_eq!(
            state.demote("GPU software decode failed", 40),
            VideoBackend::Cpu4
        );
        assert_eq!(state.demote("CPU4 failed", 50), VideoBackend::Source);
        assert_eq!(state.demote("already source", 60), VideoBackend::Source);
        assert_eq!(
            state.last_demotion().expect("demotion reason").at_unix_ms,
            50
        );
        assert_eq!(state.demotion_history().len(), 5);
        assert_eq!(state.demotion_history()[0].from_mode, "gpu_d3d11_zero_copy");
        assert_eq!(state.demotion_history()[0].to_mode, "gpu_d3d11_copy");
        assert_eq!(state.demotion_history()[1].to_mode, "gpu_vulkan_copy");
        assert_eq!(state.demotion_history()[2].to_mode, "gpu_software_decode");
        assert_eq!(state.demotion_history()[3].to_mode, "cpu4");
        assert_eq!(state.demotion_history()[4].to_mode, "original");
    }

    #[test]
    fn frame_health_commands_use_official_mpv_properties() {
        let mistimed = MpvCommand::GetMistimedFrameCount
            .ipc_json_line()
            .expect("serialize mistimed counter");
        let delayed = MpvCommand::GetVideoOutputDelayedFrameCount
            .ipc_json_line()
            .expect("serialize delayed counter");
        assert!(mistimed.contains("mistimed-frame-count"));
        assert!(delayed.contains("vo-delayed-frame-count"));
    }

    #[test]
    fn managed_process_child_fixture() {
        if std::env::var_os("AUTOLIVE_MPV_PROCESS_FIXTURE").is_some() {
            loop {
                std::thread::park();
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn managed_job_tree_descendant_fixture() {
        let Some(root) = std::env::var_os(JOB_TREE_FIXTURE_ROOT_ENV).map(PathBuf::from) else {
            return;
        };
        fs::write(
            root.join("descendant.ready"),
            std::process::id().to_string(),
        )
        .expect("publish descendant readiness");
        loop {
            thread::park();
        }
    }

    #[cfg(windows)]
    #[test]
    fn managed_job_tree_parent_fixture() {
        let Some(root) = std::env::var_os(JOB_TREE_FIXTURE_ROOT_ENV).map(PathBuf::from) else {
            return;
        };
        assert!(
            wait_for_fixture_path(&root.join("parent.bound"), JOB_TREE_FIXTURE_TIMEOUT),
            "parent must not create its descendant before Job assignment is acknowledged"
        );

        let executable = std::env::current_exe().expect("resolve test executable");
        let mut descendant = std::process::Command::new(executable)
            .args([
                "--exact",
                "realtime_video_backend::tests::managed_job_tree_descendant_fixture",
                "--nocapture",
            ])
            .env(JOB_TREE_FIXTURE_ROOT_ENV, &root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn managed descendant fixture");
        fs::write(root.join("descendant.pid"), descendant.id().to_string())
            .expect("publish descendant pid");
        assert!(
            wait_for_fixture_path(&root.join("descendant.ready"), JOB_TREE_FIXTURE_TIMEOUT),
            "descendant must reach its blocking fixture"
        );
        assert!(
            descendant
                .try_wait()
                .expect("inspect descendant fixture")
                .is_none(),
            "descendant must still be running before the Job is closed"
        );
        fs::write(root.join("tree.ready"), b"ready").expect("publish process-tree readiness");
        loop {
            thread::park();
        }
    }

    #[test]
    fn managed_process_cancel_reaps_the_owned_child() {
        let executable = std::env::current_exe().expect("resolve test executable");
        let child = std::process::Command::new(executable)
            .args([
                "--exact",
                "realtime_video_backend::tests::managed_process_child_fixture",
                "--nocapture",
            ])
            .env("AUTOLIVE_MPV_PROCESS_FIXTURE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn process fixture");
        let job = ManagedMpvJob::create().expect("create managed test job");
        job.assign_process(&child)
            .expect("assign test child to job");
        let mut owner = ManagedMpvProcess {
            child: Some(child),
            job: Some(job),
            ipc: None,
            stderr_tail: Arc::new(Mutex::new(StderrTailBuffer {
                lines: VecDeque::new(),
                bytes: 0,
            })),
            redacted_media_paths: Arc::new(Mutex::new(RedactedMediaPaths::default())),
            stderr_join: None,
        };

        assert!(owner.pid().is_some());
        owner.cancel().expect("cancel owned process");
        assert_eq!(owner.pid(), None);
        assert!(owner.has_exited().expect("owner should be empty"));
    }

    #[cfg(windows)]
    #[test]
    fn managed_job_close_reaps_the_assigned_child() {
        let executable = std::env::current_exe().expect("resolve test executable");
        let mut child = std::process::Command::new(executable)
            .args([
                "--exact",
                "realtime_video_backend::tests::managed_process_child_fixture",
                "--nocapture",
            ])
            .env("AUTOLIVE_MPV_PROCESS_FIXTURE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn process fixture");
        let job = ManagedMpvJob::create().expect("create managed test job");
        job.assign_process(&child)
            .expect("assign test child to job");

        drop(job);
        assert!(
            wait_for_child_exit(&mut child, MPV_EXIT_GRACE).expect("wait for Job Object cleanup"),
            "closing the Job Object must terminate its assigned process"
        );
    }

    #[cfg(windows)]
    #[test]
    fn managed_job_close_reaps_assigned_process_tree() {
        let root = create_job_tree_fixture_root();
        let executable = std::env::current_exe().expect("resolve test executable");
        let parent = std::process::Command::new(executable)
            .args([
                "--exact",
                "realtime_video_backend::tests::managed_job_tree_parent_fixture",
                "--nocapture",
            ])
            .env(JOB_TREE_FIXTURE_ROOT_ENV, &root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn managed parent fixture");
        let mut fixture = ManagedJobTreeFixture {
            root,
            parent: Some(parent),
            job: None,
            descendant: None,
        };

        let job = ManagedMpvJob::create().expect("create managed process-tree Job");
        job.assign_process(fixture.parent.as_ref().expect("parent fixture"))
            .expect("assign parent fixture before it creates descendants");
        fixture.job = Some(job);
        fs::write(fixture.root.join("parent.bound"), b"bound")
            .expect("acknowledge parent Job assignment");

        assert!(
            wait_for_fixture_path(&fixture.root.join("tree.ready"), JOB_TREE_FIXTURE_TIMEOUT),
            "managed process tree must become ready before the deadline"
        );
        let descendant_pid = fs::read_to_string(fixture.root.join("descendant.pid"))
            .expect("read descendant pid")
            .parse::<u32>()
            .expect("parse descendant pid");
        let descendant_start_time = process_instance_start_time(descendant_pid)
            .expect("observe running descendant process instance");
        fixture.descendant = Some((descendant_pid, descendant_start_time));
        assert!(
            fixture
                .parent
                .as_mut()
                .expect("parent fixture")
                .try_wait()
                .expect("inspect parent fixture")
                .is_none(),
            "parent must still be running before the Job is closed"
        );

        drop(fixture.job.take());
        let parent_exited = wait_for_child_exit(
            fixture.parent.as_mut().expect("parent fixture"),
            JOB_TREE_FIXTURE_TIMEOUT,
        )
        .expect("wait for parent Job cleanup");
        let descendant_exited = wait_for_process_instance_exit(
            descendant_pid,
            descendant_start_time,
            JOB_TREE_FIXTURE_TIMEOUT,
        );
        assert!(
            parent_exited && descendant_exited,
            "closing the Job must terminate both parent ({parent_exited}) and descendant ({descendant_exited})"
        );
    }

    #[test]
    fn stderr_tail_is_bounded_by_lines_and_bytes() {
        let mut tail = StderrTailBuffer {
            lines: VecDeque::new(),
            bytes: 0,
        };
        for index in 0..100 {
            tail.push(format!("{index}:{}", "x".repeat(1_024)));
        }

        assert!(tail.lines.len() <= STDERR_TAIL_MAX_LINES);
        assert!(tail.bytes <= STDERR_TAIL_MAX_BYTES);
        assert!(tail
            .lines
            .back()
            .is_some_and(|line| line.starts_with("99:")));
    }

    #[test]
    fn stderr_diagnostic_prefers_the_latest_actionable_render_failure() {
        let mut tail = StderrTailBuffer {
            lines: VecDeque::new(),
            bytes: 0,
        };
        tail.push("[vo/gpu-next] shader compile failed".to_owned());
        tail.push("Exiting...".to_owned());

        assert_eq!(
            stderr_diagnostic_line(&tail).as_deref(),
            Some("[vo/gpu-next] shader compile failed")
        );
    }

    #[test]
    fn stderr_diagnostic_ignores_shutdown_noise_after_a_general_failure() {
        let mut tail = StderrTailBuffer {
            lines: VecDeque::new(),
            bytes: 0,
        };
        tail.push("Could not initialize decoder".to_owned());
        tail.push("Exiting... (Errors when loading file)".to_owned());

        assert_eq!(
            stderr_diagnostic_line(&tail).as_deref(),
            Some("Could not initialize decoder")
        );
    }

    #[test]
    fn stderr_reader_truncates_one_line_and_keeps_the_next_line_aligned() {
        let input = format!("{}\nnext\n", "x".repeat(32));
        let mut reader = BufReader::new(input.as_bytes());

        assert_eq!(
            read_bounded_stderr_line(&mut reader, 8).expect("read long line"),
            Some("xxxxxxxx".to_owned())
        );
        assert_eq!(
            read_bounded_stderr_line(&mut reader, 8).expect("read next line"),
            Some("next".to_owned())
        );
    }

    #[test]
    fn stderr_render_failure_classifier_is_strict_and_bounded() {
        assert_eq!(
            classify_mpv_render_failure("[vo/gpu-next/libplacebo] Failed to parse user shader:"),
            Some(MpvRenderFailureKind::Shader)
        );
        assert_eq!(
            classify_mpv_render_failure("[vo/gpu-next] VK_ERROR_DEVICE_LOST"),
            Some(MpvRenderFailureKind::DeviceLost)
        );
        assert_eq!(
            classify_mpv_render_failure("[vo/gpu-next] Failed initializing video output"),
            Some(MpvRenderFailureKind::VideoOutput)
        );
        assert_eq!(
            classify_mpv_render_failure(
                "Error opening/initializing the selected video_out (--vo) device."
            ),
            Some(MpvRenderFailureKind::VideoOutput)
        );
        assert_eq!(
            classify_mpv_render_failure("[vo/gpu-next] the graphics device was lost"),
            Some(MpvRenderFailureKind::DeviceLost)
        );
        assert_eq!(
            classify_mpv_render_failure("[vo/gpu-next] shader cache hit"),
            None
        );
        assert_eq!(
            classify_mpv_render_failure("[vo/gpu-next] using Vulkan error diffusion"),
            None
        );
        assert_eq!(
            classify_mpv_render_failure("[statusline] Failed to open audio device"),
            None
        );
        assert_eq!(
            bounded_failure_line(&"x".repeat(1_024)).chars().count(),
            512
        );
    }

    #[test]
    fn n_plus_one_commit_requires_matching_identity_sequence_and_pts() {
        let identity = VideoPlanIdentity {
            session_id: 7,
            playback_generation: 11,
            source_revision: 13,
            parameter_revision: 17,
            sequence: 19,
        };
        let plan = RealtimeVideoPlan {
            slot: VideoPlanSlot::NPlus1,
            identity: identity.clone(),
            target_pts_ms: 3_000,
            period_ms: 8_000,
            seed: 23,
            prepared: true,
            commands: vec![],
            parameter_support: ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
        };
        let mut gate = VideoCommitGate {
            identity,
            media_pts_ms: 2_999,
        };

        assert_eq!(plan.commit_decision(&gate), VideoCommitDecision::TooEarly);
        gate.media_pts_ms = 3_000;
        assert_eq!(plan.commit_decision(&gate), VideoCommitDecision::Commit);
        gate.identity.sequence += 1;
        assert_eq!(
            plan.commit_decision(&gate),
            VideoCommitDecision::StaleIdentity
        );
    }

    #[test]
    fn gpu_profiles_follow_the_cross_vendor_fallback_order() {
        assert_eq!(
            MpvGpuProfile::attempt_order(),
            [
                MpvGpuProfile::D3d11ZeroCopy,
                MpvGpuProfile::D3d11Copy,
                MpvGpuProfile::VulkanCopy,
                MpvGpuProfile::SoftwareDecode,
            ]
        );
    }
}
