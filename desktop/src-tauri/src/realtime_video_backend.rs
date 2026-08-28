//! mpv/libplacebo 实时画面后端的安全核心边界。
//!
//! 本模块不接触 Tauri 窗口或前端输入。调用方只能传入已经由 Rust 播放池选中的
//! 本地媒体、受信运行资源根目录和宿主窗口句柄；mpv 参数与可写属性均由这里的
//! 固定枚举生成。实际 IPC 连接、窗口句柄获取和播放状态接线由桌面命令层负责。

use std::collections::VecDeque;
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
pub use realtime_video_ipc::MpvIpcOptions;

#[cfg(not(test))]
use crate::background_process::background_command;

const MPV_PATH_ENV: &str = "AUTOLIVE_MPV_PATH";
const MPV_IPC_PREFIX: &str = r"\\.\pipe\autolive-mpv-";
const MPV_EXIT_GRACE: Duration = Duration::from_millis(750);
const MPV_QUIT_RESPONSE_TIMEOUT: Duration = Duration::from_millis(300);
const MPV_EXIT_POLL: Duration = Duration::from_millis(10);
const STDERR_TAIL_MAX_LINES: usize = 64;
const STDERR_TAIL_MAX_BYTES: usize = 32 * 1024;
const STDERR_LINE_MAX_BYTES: usize = 2 * 1024;
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
    SerializeCommand(String),
    IpcQueueFull,
    IpcTimeout {
        request_id: u64,
    },
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
            Self::SerializeCommand(message) => {
                write!(formatter, "mpv IPC 命令序列化失败：{message}")
            }
            Self::IpcQueueFull => formatter.write_str("mpv IPC 命令队列已满"),
            Self::IpcTimeout { request_id } => {
                write!(formatter, "mpv IPC 请求 {request_id} 等待响应超时")
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MpvGraphicsApi {
    D3d11,
    Vulkan,
}

impl MpvGraphicsApi {
    pub const fn attempt_order() -> [Self; 2] {
        [Self::D3d11, Self::Vulkan]
    }

    fn fixed_arguments(self) -> [&'static str; 2] {
        match self {
            Self::D3d11 => ["--gpu-api=d3d11", "--gpu-context=d3d11"],
            Self::Vulkan => ["--gpu-api=vulkan", "--gpu-context=winvk"],
        }
    }
}

/// 已完成路径和 IPC 边界校验的固定 mpv 启动说明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MpvLaunchSpec {
    executable: VerifiedMpvExecutable,
    arguments: Vec<OsString>,
    graphics_api: MpvGraphicsApi,
    ipc_pipe: String,
    media_path: PathBuf,
}

impl MpvLaunchSpec {
    pub fn new(
        executable: VerifiedMpvExecutable,
        selected_media_path: &Path,
        host_window_id: u64,
        ipc_pipe: &str,
        graphics_api: MpvGraphicsApi,
        source_start_ms: u64,
        paused: bool,
    ) -> Result<Self, RealtimeVideoBackendError> {
        if host_window_id == 0 {
            return Err(RealtimeVideoBackendError::InvalidHostWindow);
        }
        if !is_managed_ipc_pipe(ipc_pipe) {
            return Err(RealtimeVideoBackendError::InvalidIpcPipe);
        }
        let media = selected_media_path.canonicalize().map_err(|error| {
            RealtimeVideoBackendError::InvalidMediaPath {
                path: selected_media_path.to_path_buf(),
                message: error.to_string(),
            }
        })?;
        if !media.is_file() {
            return Err(RealtimeVideoBackendError::InvalidMediaPath {
                path: media,
                message: "路径不是普通文件".to_owned(),
            });
        }

        let mut arguments = [
            "--no-config",
            "--load-scripts=no",
            "--input-default-bindings=no",
            "--input-vo-keyboard=no",
            "--osc=no",
            "--terminal=no",
            "--idle=no",
            "--vo=gpu-next",
            "--audio=no",
        ]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
        arguments.extend(graphics_api.fixed_arguments().map(OsString::from));
        arguments.push(OsString::from(match graphics_api {
            MpvGraphicsApi::D3d11 => "--hwdec=d3d11va",
            MpvGraphicsApi::Vulkan => "--hwdec=d3d11va-copy",
        }));
        arguments.push(OsString::from(format!(
            "--start={}.{:03}",
            source_start_ms / 1_000,
            source_start_ms % 1_000
        )));
        arguments.push(OsString::from(if paused {
            "--pause=yes"
        } else {
            "--pause=no"
        }));
        arguments.push(OsString::from(format!("--wid={host_window_id}")));
        arguments.push(OsString::from(format!("--input-ipc-server={ipc_pipe}")));
        // `--` 必须紧邻媒体路径，避免以 `-` 开头的合法文件名被解释为 mpv 参数。
        arguments.push(OsString::from("--"));
        arguments.push(media.clone().into_os_string());

        Ok(Self {
            executable,
            arguments,
            graphics_api,
            ipc_pipe: ipc_pipe.to_owned(),
            media_path: media,
        })
    }

    pub fn with_shader(mut self, shader: VerifiedMpvShader) -> Self {
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
        graphics_api: MpvGraphicsApi,
        source_start_ms: u64,
        paused: bool,
        shader: VerifiedMpvShader,
    ) -> Result<Self, RealtimeVideoBackendError> {
        Self::new(
            executable,
            selected_media_path,
            host_window_id,
            ipc_pipe,
            graphics_api,
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

    pub fn graphics_api(&self) -> MpvGraphicsApi {
        self.graphics_api
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

fn spawn_stderr_reader(
    stderr: ChildStderr,
    media_path: &Path,
) -> Result<(Arc<Mutex<StderrTailBuffer>>, JoinHandle<()>), RealtimeVideoBackendError> {
    let tail = Arc::new(Mutex::new(StderrTailBuffer {
        lines: VecDeque::new(),
        bytes: 0,
    }));
    let writer = Arc::clone(&tail);
    let media = media_path.to_string_lossy().into_owned();
    let handle = thread::Builder::new()
        .name("mpv-stderr-tail".to_owned())
        .spawn(move || {
            let mut reader = BufReader::new(stderr);
            loop {
                match read_bounded_stderr_line(&mut reader, STDERR_LINE_MAX_BYTES) {
                    Ok(None) => break,
                    Ok(Some(line)) => {
                        let redacted = if media.is_empty() {
                            line
                        } else {
                            line.replace(&media, "<media>")
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

/// mpv 子进程、持久 IPC 与日志线程的唯一所有者。
#[derive(Debug)]
pub struct ManagedMpvProcess {
    child: Option<Child>,
    ipc: Option<MpvIpcClient>,
    stderr_tail: Arc<Mutex<StderrTailBuffer>>,
    stderr_join: Option<JoinHandle<()>>,
}

impl ManagedMpvProcess {
    pub fn spawn(spec: &MpvLaunchSpec) -> Result<Self, RealtimeVideoBackendError> {
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
        let stderr =
            child
                .stderr
                .take()
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "stderr 捕获",
                    message: "mpv stderr 管道未创建".to_owned(),
                })?;
        let (stderr_tail, stderr_join) = match spawn_stderr_reader(stderr, &spec.media_path) {
            Ok(reader) => reader,
            Err(error) => {
                let _ignored = child.kill();
                let _ignored = child.wait();
                return Err(error);
            }
        };
        Ok(Self {
            child: Some(child),
            ipc: None,
            stderr_tail,
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

    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail
            .lock()
            .map(|tail| tail.lines.iter().cloned().collect())
            .unwrap_or_default()
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
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
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

impl VideoBackend {
    const fn next_fallback(self) -> Self {
        match self {
            Self::RealtimeGpu => Self::Cpu4,
            Self::Cpu4 | Self::Source => Self::Source,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendDemotion {
    pub from: VideoBackend,
    pub to: VideoBackend,
    pub reason: String,
    pub at_unix_ms: u64,
}

/// 每次播放会话新建；没有升级方法，确保会话内只能单向降级。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoBackendStateMachine {
    current: VideoBackend,
    last_demotion: Option<BackendDemotion>,
}

impl VideoBackendStateMachine {
    pub fn new() -> Self {
        Self {
            current: VideoBackend::RealtimeGpu,
            last_demotion: None,
        }
    }

    pub fn current(&self) -> VideoBackend {
        self.current
    }

    pub fn last_demotion(&self) -> Option<&BackendDemotion> {
        self.last_demotion.as_ref()
    }

    pub fn demote(&mut self, reason: impl Into<String>, at_unix_ms: u64) -> VideoBackend {
        let next = self.current.next_fallback();
        if next != self.current {
            self.last_demotion = Some(BackendDemotion {
                from: self.current,
                to: next,
                reason: reason.into(),
                at_unix_ms,
            });
            self.current = next;
        }
        self.current
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
    SetShaderOptions { options: MpvShaderOptions },
    InstallCpu4FilterChain,
    UpdateCpu4Filter { update: Cpu4FilterUpdate },
    SetPause { paused: bool },
    SeekAbsoluteMs { position_ms: u64 },
    GetVideoOutputConfigured,
    Quit,
}

impl MpvCommand {
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
            Self::SeekAbsoluteMs { position_ms } => {
                serde_json::json!(["seek", *position_ms as f64 / 1_000.0, "absolute+exact"])
            }
            Self::GetVideoOutputConfigured => {
                serde_json::json!(["get_property", "vo-configured"])
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MpvShaderOptions(String);

impl MpvShaderOptions {
    pub fn parse(value: String) -> Result<Self, RealtimeVideoBackendError> {
        if value.is_empty() || value.len() > 32 * 1024 {
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
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
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

    fn temporary_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "autolive-realtime-video-{name}-{}",
            std::process::id()
        ))
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
            MpvGraphicsApi::D3d11,
            1_234,
            false,
        )
        .expect("valid launch spec");

        assert!(spec.arguments().contains(&"--no-config".to_owned()));
        assert!(spec.arguments().contains(&"--vo=gpu-next".to_owned()));
        assert!(spec.arguments().contains(&"--hwdec=d3d11va".to_owned()));
        assert!(spec.arguments().contains(&"--audio=no".to_owned()));
        assert!(!spec.arguments().contains(&"--loop-file=inf".to_owned()));
        assert!(spec.arguments().contains(&"--start=1.234".to_owned()));
        assert!(spec.arguments().contains(&"--pause=no".to_owned()));
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

        let paused_spec = MpvLaunchSpec::new(
            executable,
            &media,
            42,
            r"\\.\pipe\autolive-mpv-test_2",
            MpvGraphicsApi::Vulkan,
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

        let spec = MpvLaunchSpec::new(
            executable,
            &media,
            42,
            r"\\.\pipe\autolive-mpv-shader_1",
            MpvGraphicsApi::D3d11,
            0,
            false,
        )
        .expect("valid shader launch spec")
        .with_shader(shader);

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
            state.demote("renderer initialization failed", 10),
            VideoBackend::Cpu4
        );
        assert_eq!(state.demote("CPU4 failed", 20), VideoBackend::Source);
        assert_eq!(state.demote("already source", 30), VideoBackend::Source);
        assert_eq!(
            state.last_demotion().expect("demotion reason").at_unix_ms,
            20
        );
    }

    #[test]
    fn managed_process_child_fixture() {
        if std::env::var_os("AUTOLIVE_MPV_PROCESS_FIXTURE").is_some() {
            loop {
                std::thread::park();
            }
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
        let mut owner = ManagedMpvProcess {
            child: Some(child),
            ipc: None,
            stderr_tail: Arc::new(Mutex::new(StderrTailBuffer {
                lines: VecDeque::new(),
                bytes: 0,
            })),
            stderr_join: None,
        };

        assert!(owner.pid().is_some());
        owner.cancel().expect("cancel owned process");
        assert_eq!(owner.pid(), None);
        assert!(owner.has_exited().expect("owner should be empty"));
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
    fn d3d11_remains_primary_for_the_current_1080p_scope() {
        assert_eq!(
            MpvGraphicsApi::attempt_order(),
            [MpvGraphicsApi::D3d11, MpvGraphicsApi::Vulkan]
        );
    }
}
