//! AkVirtualCamera 输出运行时。
//!
//! 该模块只负责把已经由 WGC/D3D11 完成 GPU 缩放和 YUY2 转换的帧交给独立 GPL
//! sidecar。它不安装系统设备、不接受前端路径，也不把 AkVirtualCamera C API
//! 链接进 Rust 主程序。Windows 以外保持 fail-closed。

use autolive_virtual_camera_contract::{GpuCaptureFacts, VirtualCameraOutputManager};
use autolive_virtual_camera_native::{CaptureConfig, CaptureGpuFacts};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VirtualCameraOutputError {
    #[cfg(not(windows))]
    Unsupported(&'static str),
    InvalidConfiguration(String),
    Startup(String),
    Runtime(String),
}

impl std::fmt::Display for VirtualCameraOutputError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(not(windows))]
            Self::Unsupported(message) => write!(formatter, "虚拟摄像头输出不可用：{message}"),
            Self::InvalidConfiguration(message) => {
                write!(formatter, "虚拟摄像头输出配置无效：{message}")
            }
            Self::Startup(message) => write!(formatter, "虚拟摄像头输出启动失败：{message}"),
            Self::Runtime(message) => write!(formatter, "虚拟摄像头输出运行失败：{message}"),
        }
    }
}

impl std::error::Error for VirtualCameraOutputError {}

/// 启动前已由原生探测确认的 GPU 事实。单独传入而不是让运行时猜测，避免把
/// WARP、CPU 转换或未经验证的 adapter 冒充成 GPU 输出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualCameraGpuFacts {
    pub capture_api: String,
    pub adapter_luid: String,
    pub adapter_name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    pub feature_level: String,
}

impl VirtualCameraGpuFacts {
    pub fn into_contract(self) -> GpuCaptureFacts {
        GpuCaptureFacts {
            capture_api: self.capture_api,
            adapter_luid: self.adapter_luid,
            adapter_name: self.adapter_name,
            vendor_id: self.vendor_id,
            device_id: self.device_id,
            feature_level: self.feature_level,
            is_warp: false,
            gpu_scale: true,
            gpu_color_convert: true,
            transport: "akvcam_mmap_cpu".to_owned(),
            zero_copy: false,
            width: 1280,
            height: 720,
            fps: 30,
        }
    }
}

impl From<CaptureGpuFacts> for VirtualCameraGpuFacts {
    fn from(facts: CaptureGpuFacts) -> Self {
        Self {
            capture_api: autolive_virtual_camera_native::WINDOWS_GRAPHICS_CAPTURE_API.to_owned(),
            adapter_luid: facts.adapter_luid,
            adapter_name: facts.adapter_name,
            vendor_id: facts.vendor_id,
            device_id: facts.device_id,
            feature_level: facts.feature_level,
        }
    }
}

/// 一个输出 session 的唯一所有者。`stop` 会请求捕获线程和 sidecar 退出并 Join，
/// 不把子进程或写管道任务遗留在后台。
pub struct VirtualCameraOutputTask {
    inner: platform::VirtualCameraOutputTask,
    capture_window_id: u64,
    gpu_facts: Option<VirtualCameraGpuFacts>,
}

impl std::fmt::Debug for VirtualCameraOutputTask {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VirtualCameraOutputTask")
            .finish_non_exhaustive()
    }
}

impl VirtualCameraOutputTask {
    pub fn start(
        manager: Arc<Mutex<VirtualCameraOutputManager>>,
        sidecar_path: PathBuf,
        capture_config: CaptureConfig,
        generation: u64,
    ) -> Result<Self, VirtualCameraOutputError> {
        let capture_window_id = capture_config.final_effect_window_id;
        Ok(Self {
            inner: platform::VirtualCameraOutputTask::start(
                manager,
                sidecar_path,
                capture_config,
                generation,
            )?,
            capture_window_id,
            gpu_facts: None,
        })
    }

    /// 返回当前捕获会话绑定的最终效果窗口 HWND。窗口宿主重建后，调用方
    /// 必须用新的句柄重建 WGC session；不能继续向旧 HWND 取帧。
    pub fn capture_window_id(&self) -> u64 {
        self.capture_window_id
    }

    pub fn wait_until_ready(&mut self, timeout: Duration) -> Result<(), VirtualCameraOutputError> {
        let facts = self.inner.wait_until_ready(timeout)?;
        self.gpu_facts = Some(facts);
        Ok(())
    }

    pub fn gpu_facts(&self) -> Option<&VirtualCameraGpuFacts> {
        self.gpu_facts.as_ref()
    }

    pub fn activate(&mut self) -> Result<(), VirtualCameraOutputError> {
        self.inner.activate()
    }

    pub fn stop(&mut self) -> Result<(), VirtualCameraOutputError> {
        self.inner.stop()
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use autolive_virtual_camera_contract::{
        OutputPolicy, VirtualCameraConfig, VirtualCameraFrame, VirtualCameraState,
    };
    use autolive_virtual_camera_native::{
        virtual_camera_pipe_name as pipe_name, CaptureError, CapturedFrame, NativeCapturePump,
        SidecarFrame, MAX_PAYLOAD_BYTES, OUTPUT_HEIGHT, OUTPUT_WIDTH,
    };
    use getrandom::fill as get_random_bytes;
    use std::fs::{File, OpenOptions};
    use std::io::{self, BufReader, Read, Write};
    use std::os::windows::io::AsRawHandle;
    use std::process::{Child, ChildStdout, Command, Stdio};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    const SIDE_CAR_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
    const SIDE_CAR_STOP_DEADLINE: Duration = Duration::from_millis(500);
    const PIPE_RETRY_INTERVAL: Duration = Duration::from_millis(20);
    const FRAME_IDLE_POLL: Duration = Duration::from_millis(2);
    const PIPE_CONNECT_ERROR_FILE_NOT_FOUND: i32 = 2;
    const PIPE_CONNECT_ERROR_PIPE_BUSY: i32 = 231;
    const PIPE_CONNECT_ERROR_BAD_PATH: i32 = 161;
    const MAX_CAPTURE_RESIZE_RECOVERIES: u8 = 2;
    const CAPTURE_RESIZE_RECOVERY_BACKOFF: Duration = Duration::from_millis(50);
    const MAX_CAPTURE_DEVICE_LOST_RECOVERIES: u8 = 2;
    const CAPTURE_DEVICE_LOST_RECOVERY_BACKOFF: Duration = Duration::from_millis(100);
    const MAX_STATUS_LINE_BYTES: usize = 64;

    #[derive(Debug)]
    struct ManagedSidecarJob {
        job: win32job::Job,
    }

    impl ManagedSidecarJob {
        fn create() -> io::Result<Self> {
            let mut limits = win32job::ExtendedLimitInfo::new();
            limits.limit_kill_on_job_close();
            win32job::Job::create_with_limit_info(&limits)
                .map(|job| Self { job })
                .map_err(io::Error::from)
        }

        fn assign_process(&self, child: &Child) -> io::Result<()> {
            self.job
                .assign_process(child.as_raw_handle() as isize)
                .map_err(io::Error::from)
        }
    }

    /// sidecar stdout 只承载受限的客户端数量状态。Drop 时先终止 child，
    /// 再 Join 读取线程，避免在正常停止路径上永久等待 EOF。
    struct StatusReaderGuard {
        child_slot: Arc<Mutex<Option<Child>>>,
        join: Option<JoinHandle<()>>,
        stop: Arc<AtomicBool>,
    }

    impl Drop for StatusReaderGuard {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            stop_child_with_deadline(&self.child_slot, Duration::from_millis(250));
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
    }

    pub struct VirtualCameraOutputTask {
        stop: Arc<AtomicBool>,
        child: Arc<Mutex<Option<Child>>>,
        startup_receiver: Option<mpsc::Receiver<Result<VirtualCameraGpuFacts, String>>>,
        activate_sender: Option<mpsc::SyncSender<()>>,
        join: Option<JoinHandle<()>>,
    }

    /// 已经通过 GPU→CPU 回读并提交到契约的最新画面。它与线上的 packet
    /// 分离：暂停期间发送黑帧不能覆盖这份有效画面，恢复后也必须为每次
    /// 重复发送分配新的 sidecar sequence。
    struct LatestOutputFrame {
        timestamp_100ns: i64,
        payload: Vec<u8>,
    }

    impl std::fmt::Debug for VirtualCameraOutputTask {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("VirtualCameraOutputTask")
                .field("running", &self.join.is_some())
                .finish()
        }
    }

    impl VirtualCameraOutputTask {
        pub fn start(
            manager: Arc<Mutex<VirtualCameraOutputManager>>,
            sidecar_path: PathBuf,
            capture_config: CaptureConfig,
            generation: u64,
        ) -> Result<Self, VirtualCameraOutputError> {
            validate_sidecar_path(&sidecar_path)?;
            if generation == 0 {
                return Err(VirtualCameraOutputError::InvalidConfiguration(
                    "generation 不能为 0".to_owned(),
                ));
            }

            let mut token = [0_u8; 16];
            get_random_bytes(&mut token).map_err(|error| {
                VirtualCameraOutputError::Startup(format!("生成 sidecar 会话令牌失败：{error}"))
            })?;
            if token.iter().all(|byte| *byte == 0) {
                return Err(VirtualCameraOutputError::Startup(
                    "系统随机数返回全零会话令牌".to_owned(),
                ));
            }
            let pipe = pipe_name(token);
            let stop = Arc::new(AtomicBool::new(false));
            let child = Arc::new(Mutex::new(None));
            let worker_stop = Arc::clone(&stop);
            let worker_child = Arc::clone(&child);
            let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
            let (activate_sender, activate_receiver) = mpsc::sync_channel(1);
            let join = thread::Builder::new()
                .name("autolive-akvcam-sidecar".to_owned())
                .spawn(move || {
                    run_output_worker(
                        manager,
                        sidecar_path,
                        pipe,
                        capture_config,
                        generation,
                        worker_stop,
                        worker_child,
                        startup_sender,
                        activate_receiver,
                    );
                })
                .map_err(|error| {
                    VirtualCameraOutputError::Startup(format!(
                        "启动虚拟摄像头输出线程失败：{error}"
                    ))
                })?;

            Ok(Self {
                stop,
                child,
                startup_receiver: Some(startup_receiver),
                activate_sender: Some(activate_sender),
                join: Some(join),
            })
        }

        pub fn wait_until_ready(
            &mut self,
            timeout: Duration,
        ) -> Result<VirtualCameraGpuFacts, VirtualCameraOutputError> {
            let Some(receiver) = self.startup_receiver.take() else {
                return Err(VirtualCameraOutputError::Startup(
                    "虚拟摄像头输出启动结果已被读取".to_owned(),
                ));
            };
            match receiver.recv_timeout(timeout) {
                Ok(Ok(facts)) => Ok(facts),
                Ok(Err(error)) => {
                    self.stop()?;
                    Err(VirtualCameraOutputError::Startup(error))
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.stop()?;
                    Err(VirtualCameraOutputError::Startup(
                        "等待 sidecar 管道连接超时".to_owned(),
                    ))
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.stop()?;
                    Err(VirtualCameraOutputError::Startup(
                        "输出线程未返回启动结果".to_owned(),
                    ))
                }
            }
        }

        pub fn activate(&mut self) -> Result<(), VirtualCameraOutputError> {
            self.activate_sender
                .take()
                .ok_or_else(|| {
                    VirtualCameraOutputError::Startup("虚拟摄像头输出已激活".to_owned())
                })?
                .send(())
                .map_err(|_| VirtualCameraOutputError::Startup("输出线程已退出".to_owned()))
        }

        pub fn stop(&mut self) -> Result<(), VirtualCameraOutputError> {
            self.stop.store(true, Ordering::Release);
            // 先让输出线程观察取消并关闭管道，给 sidecar 一个短暂的优雅退出窗口；
            // 到期后再强制终止，避免写管道或异常 sidecar 让 Join 无界等待。
            stop_child_with_deadline(&self.child, SIDE_CAR_STOP_DEADLINE);
            if let Some(join) = self.join.take() {
                join.join().map_err(|_| {
                    VirtualCameraOutputError::Runtime("输出线程异常退出".to_owned())
                })?;
            }
            self.activate_sender.take();
            Ok(())
        }
    }

    impl Drop for VirtualCameraOutputTask {
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }

    fn validate_sidecar_path(path: &std::path::Path) -> Result<(), VirtualCameraOutputError> {
        if !path.is_absolute() {
            return Err(VirtualCameraOutputError::InvalidConfiguration(
                "sidecar 路径必须是绝对路径".to_owned(),
            ));
        }
        if path.file_name().and_then(|value| value.to_str())
            != Some("akvirtualcamera-sidecar-x64.exe")
        {
            return Err(VirtualCameraOutputError::InvalidConfiguration(
                "sidecar 文件名不是固定 x64 组件".to_owned(),
            ));
        }
        let metadata = std::fs::metadata(path).map_err(|error| {
            VirtualCameraOutputError::Startup(format!("读取 sidecar 文件失败：{error}"))
        })?;
        if !metadata.is_file() {
            return Err(VirtualCameraOutputError::InvalidConfiguration(
                "sidecar 路径不是普通文件".to_owned(),
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn run_output_worker(
        manager: Arc<Mutex<VirtualCameraOutputManager>>,
        sidecar_path: PathBuf,
        pipe: String,
        capture_config: CaptureConfig,
        generation: u64,
        stop: Arc<AtomicBool>,
        child_slot: Arc<Mutex<Option<Child>>>,
        startup_sender: mpsc::SyncSender<Result<VirtualCameraGpuFacts, String>>,
        activate_receiver: mpsc::Receiver<()>,
    ) {
        let result = run_output_worker_inner(
            &manager,
            &sidecar_path,
            &pipe,
            capture_config,
            generation,
            &stop,
            &child_slot,
            &startup_sender,
            &activate_receiver,
        );
        if let Err(error) = result {
            if let Ok(mut state) = manager.lock() {
                state.fail(error.clone());
            }
        }
        stop_child(&child_slot);
    }

    #[allow(clippy::too_many_arguments)]
    fn run_output_worker_inner(
        manager: &Arc<Mutex<VirtualCameraOutputManager>>,
        sidecar_path: &std::path::Path,
        pipe: &str,
        capture_config: CaptureConfig,
        generation: u64,
        stop: &Arc<AtomicBool>,
        child_slot: &Arc<Mutex<Option<Child>>>,
        startup_sender: &mpsc::SyncSender<Result<VirtualCameraGpuFacts, String>>,
        activate_receiver: &mpsc::Receiver<()>,
    ) -> Result<(), String> {
        let job = ManagedSidecarJob::create()
            .map_err(|error| format!("创建 sidecar Job Object 失败：{error}"))?;
        let session_token = token_from_pipe(pipe).map_err(|error| error.to_string())?;
        let mut command = Command::new(sidecar_path);
        command
            .arg("--session-token-stdin")
            // The session token is delivered once over the inherited stdin pipe;
            // never place it in argv or the environment where process inspection
            // tools could expose it.
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // CREATE_NO_WINDOW：sidecar 是受管输出组件，不应在用户桌面弹出控制台。
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
        let spawned = command
            .spawn()
            .map_err(|error| format!("启动 AkVirtualCamera sidecar 失败：{error}"))?;
        let mut spawned = spawned;
        let Some(mut token_stdin) = spawned.stdin.take() else {
            let _ = spawned.kill();
            let _ = spawned.wait();
            return Err("sidecar 未提供令牌 stdin 管道".to_owned());
        };
        if let Err(error) = token_stdin
            .write_all(session_token.as_bytes())
            .and_then(|_| token_stdin.write_all(b"\n"))
        {
            let _ = spawned.kill();
            let _ = spawned.wait();
            return Err(format!("写入 sidecar 会话令牌失败：{error}"));
        }
        drop(token_stdin);
        if let Err(error) = job.assign_process(&spawned) {
            let _ = spawned.kill();
            let _ = spawned.wait();
            return Err(format!("绑定 sidecar Job Object 失败：{error}"));
        }
        child_slot
            .lock()
            .map_err(|_| "sidecar 子进程状态锁已损坏".to_owned())?
            .replace(spawned);

        let stdout = child_slot
            .lock()
            .map_err(|_| "sidecar 子进程状态锁已损坏".to_owned())?
            .as_mut()
            .and_then(|child| child.stdout.take())
            .ok_or_else(|| "sidecar 未提供客户端状态 stdout".to_owned())?;
        let session_generation = Arc::new(AtomicU64::new(generation));
        let _status_reader = StatusReaderGuard {
            child_slot: Arc::clone(child_slot),
            join: Some(start_status_reader(
                stdout,
                manager,
                stop,
                Arc::clone(&session_generation),
            )?),
            stop: Arc::clone(stop),
        };

        let mut capture =
            NativeCapturePump::start(capture_config.clone()).map_err(format_capture_error)?;
        let capture_gpu_facts = VirtualCameraGpuFacts::from(capture.gpu_facts().clone());
        let mut output = connect_pipe(pipe, stop, child_slot)?;
        startup_sender
            .send(Ok(capture_gpu_facts))
            .map_err(|_| "调用方在输出线程准备完成前已退出".to_owned())?;
        wait_for_activation(activate_receiver, stop, SIDE_CAR_STARTUP_TIMEOUT)?;

        let config = VirtualCameraConfig::default();
        let mut capture_sequence = 0_u64;
        let mut current_generation = generation;
        let mut resize_recovery_attempts = 0_u8;
        let mut device_lost_recovery_attempts = 0_u8;
        let mut output_sequence = 0_u64;
        let black_payload = black_yuy2_payload();
        let mut latest_frame: Option<LatestOutputFrame> = None;
        let frame_period = Duration::from_nanos(1_000_000_000 / u64::from(config.fps));
        let mut next_send = Instant::now();
        loop {
            if stop.load(Ordering::Acquire) {
                break;
            }
            match capture.try_next_frame() {
                Ok(Some(frame)) => {
                    latest_frame = Some(capture_frame_into_manager(
                        manager,
                        &config,
                        current_generation,
                        frame,
                        &mut capture_sequence,
                    )?);
                }
                Ok(None) => {}
                Err(CaptureError::FrameSizeChanged { expected, received })
                    if resize_recovery_attempts < MAX_CAPTURE_RESIZE_RECOVERIES =>
                {
                    resize_recovery_attempts = resize_recovery_attempts.saturating_add(1);
                    {
                        let mut state = manager
                            .lock()
                            .map_err(|_| "虚拟摄像头状态锁已损坏".to_owned())?;
                        if state.status().gpu.is_none() {
                            return Err("WGC 尺寸变化时缺少已验证的 GPU 事实".to_owned());
                        }
                        state
                            .begin_recovery(format!(
                                "WGC 画面尺寸变化：期望 {expected:?}，收到 {received:?}，正在重建捕获会话"
                            ))
                            .map_err(|error| error.to_string())?;
                        current_generation = state.generation();
                    }
                    session_generation.store(current_generation, Ordering::Release);
                    latest_frame = None;
                    output_sequence = output_sequence.saturating_add(1);
                    write_output_packet(
                        &mut output,
                        current_generation,
                        output_sequence,
                        OutputPolicy::Black,
                        None,
                        &black_payload,
                    )?;
                    capture.stop().map_err(format_capture_error)?;
                    thread::sleep(CAPTURE_RESIZE_RECOVERY_BACKOFF);
                    capture = NativeCapturePump::start(capture_config.clone())
                        .map_err(format_capture_error)?;
                    let gpu_facts =
                        VirtualCameraGpuFacts::from(capture.gpu_facts().clone()).into_contract();
                    manager
                        .lock()
                        .map_err(|_| "虚拟摄像头状态锁已损坏".to_owned())?
                        .mark_ready(gpu_facts)
                        .map_err(|error| error.to_string())?;
                    next_send = Instant::now() + frame_period;
                }
                Err(CaptureError::DeviceLost(reason))
                    if device_lost_recovery_attempts < MAX_CAPTURE_DEVICE_LOST_RECOVERIES =>
                {
                    device_lost_recovery_attempts = device_lost_recovery_attempts.saturating_add(1);
                    {
                        let mut state = manager
                            .lock()
                            .map_err(|_| "虚拟摄像头状态锁已损坏".to_owned())?;
                        if state.status().gpu.is_none() {
                            return Err("D3D11 设备丢失时缺少已验证的 GPU 事实".to_owned());
                        }
                        state
                            .begin_recovery(format!("D3D11 设备已丢失：{reason}，正在重建捕获会话"))
                            .map_err(|error| error.to_string())?;
                        current_generation = state.generation();
                    }
                    session_generation.store(current_generation, Ordering::Release);
                    latest_frame = None;
                    // 在重建 GPU 会话前先发出一帧黑画面，让下游不会继续消费
                    // 已失效的旧代际；恢复成功后再回到最新有效画面。
                    output_sequence = output_sequence.saturating_add(1);
                    write_output_packet(
                        &mut output,
                        current_generation,
                        output_sequence,
                        OutputPolicy::Black,
                        None,
                        &black_payload,
                    )?;
                    capture.stop().map_err(format_capture_error)?;
                    thread::sleep(CAPTURE_DEVICE_LOST_RECOVERY_BACKOFF);
                    capture = NativeCapturePump::start(capture_config.clone())
                        .map_err(format_capture_error)?;
                    let gpu_facts =
                        VirtualCameraGpuFacts::from(capture.gpu_facts().clone()).into_contract();
                    manager
                        .lock()
                        .map_err(|_| "虚拟摄像头状态锁已损坏".to_owned())?
                        .mark_ready(gpu_facts)
                        .map_err(|error| error.to_string())?;
                    next_send = Instant::now() + frame_period;
                }
                Err(error) => return Err(format_capture_error(error)),
            }
            let now = Instant::now();
            if now >= next_send {
                let policy = manager
                    .lock()
                    .map_err(|_| "虚拟摄像头状态锁已损坏".to_owned())?
                    .current_output_policy();
                output_sequence = output_sequence.saturating_add(1);
                write_output_packet(
                    &mut output,
                    current_generation,
                    output_sequence,
                    policy,
                    latest_frame.as_ref(),
                    &black_payload,
                )?;
                next_send = now.checked_add(frame_period).unwrap_or(now + frame_period);
            } else {
                thread::sleep(FRAME_IDLE_POLL);
            }
        }
        capture.stop().map_err(format_capture_error)?;
        let _ = output.flush();
        Ok(())
    }

    fn start_status_reader(
        stdout: ChildStdout,
        manager: &Arc<Mutex<VirtualCameraOutputManager>>,
        stop: &Arc<AtomicBool>,
        session_generation: Arc<AtomicU64>,
    ) -> Result<JoinHandle<()>, String> {
        let manager = Arc::clone(manager);
        let stop = Arc::clone(stop);
        thread::Builder::new()
            .name("autolive-akvcam-status".to_owned())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut line = Vec::with_capacity(MAX_STATUS_LINE_BYTES);
                loop {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    match read_bounded_status_line(&mut reader, &mut line) {
                        Ok(0) => break,
                        Ok(bytes_read) if bytes_read > MAX_STATUS_LINE_BYTES => continue,
                        Ok(_) => {}
                        Err(_) => break,
                    }
                    let Ok(line) = std::str::from_utf8(&line) else {
                        continue;
                    };
                    let line = line.trim_end_matches(&['\r', '\n'][..]);
                    let Some(count) = parse_client_count(line) else {
                        continue;
                    };
                    let Ok(mut state) = manager.lock() else {
                        break;
                    };
                    if state.generation() != session_generation.load(Ordering::Acquire) {
                        break;
                    }
                    // sidecar 可能在命令调用方把 Starting 提升为 Ready 之前
                    // 就发出首个状态；丢弃这一次，继续等待下一次有界状态回报。
                    if matches!(
                        state.state(),
                        VirtualCameraState::Starting | VirtualCameraState::Recovering
                    ) {
                        continue;
                    }
                    if state.set_downstream_client_count(count).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| format!("启动 sidecar 状态读取线程失败：{error}"))
    }

    /// 从 sidecar stdout 读取一行，但最多在内存中保留
    /// `MAX_STATUS_LINE_BYTES` 字节。超长行会继续被丢弃到换行符，避免
    /// 恶意或损坏的 sidecar 输出造成无界 String/Vec 扩容并破坏下一行边界。
    fn read_bounded_status_line<R: Read>(
        reader: &mut BufReader<R>,
        line: &mut Vec<u8>,
    ) -> io::Result<usize> {
        line.clear();
        let mut byte = [0_u8; 1];
        let mut total = 0_usize;
        let mut too_long = false;
        loop {
            let bytes_read = reader.read(&mut byte)?;
            if bytes_read == 0 {
                return Ok(if total == 0 {
                    0
                } else if too_long {
                    MAX_STATUS_LINE_BYTES + 1
                } else {
                    total
                });
            }
            total = total.saturating_add(bytes_read);
            if byte[0] == b'\n' {
                return Ok(if too_long {
                    MAX_STATUS_LINE_BYTES + 1
                } else {
                    total
                });
            }
            if too_long {
                continue;
            }
            if line.len() < MAX_STATUS_LINE_BYTES {
                line.push(byte[0]);
            } else {
                too_long = true;
                line.clear();
            }
        }
    }

    fn parse_client_count(line: &str) -> Option<u32> {
        line.strip_prefix("GPAKVC_CLIENTS ")
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|count| *count <= 1024)
    }

    fn black_yuy2_payload() -> Vec<u8> {
        let mut payload = vec![0_u8; MAX_PAYLOAD_BYTES];
        for pixel_pair in payload.chunks_exact_mut(4) {
            pixel_pair.copy_from_slice(&[16, 128, 16, 128]);
        }
        payload
    }

    fn encode_output_packet(
        generation: u64,
        sequence: u64,
        policy: OutputPolicy,
        latest_frame: Option<&LatestOutputFrame>,
        black_payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        let (timestamp_100ns, payload): (i64, &[u8]) = match (policy, latest_frame) {
            (OutputPolicy::LatestFrame, Some(frame)) => {
                (frame.timestamp_100ns.max(0), frame.payload.as_slice())
            }
            _ => (0, black_payload),
        };
        let frame = SidecarFrame::new(generation, sequence, timestamp_100ns, payload.to_vec())
            .map_err(format_protocol_error)?;
        let mut encoded = Vec::with_capacity(
            autolive_virtual_camera_native::FRAME_HEADER_BYTES + MAX_PAYLOAD_BYTES,
        );
        frame.encode(&mut encoded).map_err(format_protocol_error)?;
        Ok(encoded)
    }

    fn write_output_packet(
        output: &mut File,
        generation: u64,
        sequence: u64,
        policy: OutputPolicy,
        latest_frame: Option<&LatestOutputFrame>,
        black_payload: &[u8],
    ) -> Result<(), String> {
        let packet =
            encode_output_packet(generation, sequence, policy, latest_frame, black_payload)?;
        output
            .write_all(&packet)
            .map_err(|error| format!("写入 sidecar Named Pipe 失败：{error}"))
    }

    fn capture_frame_into_manager(
        manager: &Arc<Mutex<VirtualCameraOutputManager>>,
        config: &autolive_virtual_camera_contract::VirtualCameraConfig,
        generation: u64,
        captured: CapturedFrame,
        capture_sequence: &mut u64,
    ) -> Result<LatestOutputFrame, String> {
        if captured.width != OUTPUT_WIDTH
            || captured.height != OUTPUT_HEIGHT
            || captured.payload.len() != MAX_PAYLOAD_BYTES
        {
            return Err(format!(
                "GPU 捕获输出规格无效：{}×{}、{} 字节",
                captured.width,
                captured.height,
                captured.payload.len()
            ));
        }
        let readback_elapsed = captured.readback_elapsed;
        *capture_sequence = capture_sequence.saturating_add(1);
        let timestamp_90khz = u64::try_from(captured.timestamp_100ns.max(0))
            .unwrap_or_default()
            .saturating_mul(9)
            / 1_000;
        let contract_frame = VirtualCameraFrame::new(
            generation,
            *capture_sequence,
            timestamp_90khz,
            captured.payload,
        )
        .map_err(|error| error.to_string())?;
        let mut state = manager
            .lock()
            .map_err(|_| "虚拟摄像头状态锁已损坏".to_owned())?;
        state.record_readback(readback_elapsed);
        state
            .submit_frame(contract_frame)
            .map_err(|error| error.to_string())?;
        let delivered = state
            .take_latest_frame()
            .ok_or_else(|| "最新 GPU 帧提交后未能交付".to_owned())?;
        let timestamp_100ns =
            i64::try_from(delivered.timestamp_90khz.saturating_mul(1_000) / 9).unwrap_or(i64::MAX);
        // `config` 已由固定输出门禁校验；这里仍通过其 frame_bytes 触发
        // 配置错误传播，避免未来修改规格时静默接受错误大小。
        let expected_bytes = config.frame_bytes().map_err(|error| error.to_string())?;
        if delivered.payload.len() != expected_bytes {
            return Err(format!(
                "契约帧大小与输出配置不一致：需要 {expected_bytes}，收到 {}",
                delivered.payload.len()
            ));
        }
        Ok(LatestOutputFrame {
            timestamp_100ns,
            payload: delivered.payload,
        })
    }

    fn connect_pipe(
        name: &str,
        stop: &Arc<AtomicBool>,
        child_slot: &Arc<Mutex<Option<Child>>>,
    ) -> Result<File, String> {
        let deadline = Instant::now() + SIDE_CAR_STARTUP_TIMEOUT;
        loop {
            if stop.load(Ordering::Acquire) {
                return Err("虚拟摄像头输出已取消".to_owned());
            }
            match OpenOptions::new().write(true).open(name) {
                Ok(file) => return Ok(file),
                Err(error) => {
                    if let Ok(mut child) = child_slot.lock() {
                        if let Some(process) = child.as_mut() {
                            if let Ok(Some(status)) = process.try_wait() {
                                return Err(format!("AkVirtualCamera sidecar 提前退出：{status}"));
                            }
                        }
                    }
                    let retryable = matches!(
                        error.raw_os_error(),
                        Some(PIPE_CONNECT_ERROR_FILE_NOT_FOUND)
                            | Some(PIPE_CONNECT_ERROR_PIPE_BUSY)
                            | Some(PIPE_CONNECT_ERROR_BAD_PATH)
                    );
                    if !retryable || Instant::now() >= deadline {
                        return Err(format!("连接 sidecar Named Pipe 失败：{error}"));
                    }
                    thread::sleep(PIPE_RETRY_INTERVAL);
                }
            }
        }
    }

    fn wait_for_activation(
        receiver: &mpsc::Receiver<()>,
        stop: &Arc<AtomicBool>,
        timeout: Duration,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            if stop.load(Ordering::Acquire) {
                return Err("虚拟摄像头输出已取消".to_owned());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("等待输出激活信号超时".to_owned());
            }
            match receiver.recv_timeout(remaining.min(Duration::from_millis(50))) {
                Ok(()) => return Ok(()),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("输出激活通道已关闭".to_owned())
                }
            }
        }
    }

    fn token_from_pipe(pipe: &str) -> Result<String, &'static str> {
        let prefix = r"\\.\pipe\GpAutoLive-AkVirtualCamera-";
        let token = pipe.strip_prefix(prefix).ok_or("sidecar 管道前缀无效")?;
        if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("sidecar 管道令牌格式无效");
        }
        Ok(token.to_owned())
    }

    fn stop_child(child_slot: &Arc<Mutex<Option<Child>>>) {
        stop_child_with_deadline(child_slot, Duration::ZERO);
    }

    fn stop_child_with_deadline(child_slot: &Arc<Mutex<Option<Child>>>, deadline: Duration) {
        let deadline_at = Instant::now() + deadline;
        loop {
            let exited = match child_slot.lock() {
                Ok(mut child) => match child.as_mut() {
                    None => true,
                    Some(process) => process.try_wait().ok().flatten().is_some(),
                },
                Err(_) => false,
            };
            if exited || Instant::now() >= deadline_at {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        if let Ok(mut child) = child_slot.lock() {
            if let Some(mut process) = child.take() {
                let _ = process.kill();
                let _ = process.wait();
            }
        }
    }

    fn format_capture_error(error: CaptureError) -> String {
        error.to_string()
    }

    fn format_protocol_error(
        error: autolive_virtual_camera_native::VirtualCameraProtocolError,
    ) -> String {
        error.to_string()
    }

    #[cfg(test)]
    mod tests {
        use super::{
            parse_client_count, read_bounded_status_line, wait_for_activation,
            MAX_STATUS_LINE_BYTES,
        };
        use std::io::{BufReader, Cursor};
        use std::sync::{atomic::AtomicBool, mpsc, Arc};
        use std::time::Duration;

        #[test]
        fn client_count_status_is_bounded_and_strictly_parsed() {
            assert_eq!(parse_client_count("GPAKVC_CLIENTS 0"), Some(0));
            assert_eq!(parse_client_count("GPAKVC_CLIENTS 12"), Some(12));
            assert_eq!(parse_client_count("GPAKVC_CLIENTS 1025"), None);
            assert_eq!(parse_client_count("GPAKVC_CLIENTS -1"), None);
            assert_eq!(parse_client_count("GPAKVC_CLIENTS 1 extra"), None);
            assert_eq!(parse_client_count("clients 1"), None);
        }

        #[test]
        fn overlong_status_line_is_discarded_without_unbounded_buffer_growth() {
            let input = format!(
                "{}\nGPAKVC_CLIENTS 2\n",
                "x".repeat(MAX_STATUS_LINE_BYTES + 32)
            );
            let mut reader = BufReader::new(Cursor::new(input.into_bytes()));
            let mut line = Vec::with_capacity(MAX_STATUS_LINE_BYTES);
            assert_eq!(
                read_bounded_status_line(&mut reader, &mut line).unwrap(),
                MAX_STATUS_LINE_BYTES + 1
            );
            assert!(line.is_empty());
            assert_eq!(
                read_bounded_status_line(&mut reader, &mut line).unwrap(),
                "GPAKVC_CLIENTS 2\n".len()
            );
            assert_eq!(std::str::from_utf8(&line).unwrap(), "GPAKVC_CLIENTS 2");
        }

        #[test]
        fn activation_wait_observes_cancellation_without_five_second_join() {
            let (_sender, receiver) = mpsc::sync_channel(1);
            let stop = Arc::new(AtomicBool::new(true));
            let result = wait_for_activation(&receiver, &stop, Duration::from_secs(5));
            assert_eq!(result, Err("虚拟摄像头输出已取消".to_owned()));
        }

        #[test]
        fn activation_wait_accepts_signal_before_deadline() {
            let (sender, receiver) = mpsc::sync_channel(1);
            sender.send(()).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            assert_eq!(
                wait_for_activation(&receiver, &stop, Duration::from_secs(1)),
                Ok(())
            );
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    #[derive(Debug)]
    pub struct VirtualCameraOutputTask;

    impl VirtualCameraOutputTask {
        pub fn start(
            _manager: Arc<Mutex<VirtualCameraOutputManager>>,
            _sidecar_path: PathBuf,
            _capture_config: CaptureConfig,
            _generation: u64,
        ) -> Result<Self, VirtualCameraOutputError> {
            Err(VirtualCameraOutputError::Unsupported(
                "AkVirtualCamera 输出仅支持 Windows 10/11",
            ))
        }

        pub fn wait_until_ready(
            &mut self,
            _timeout: Duration,
        ) -> Result<VirtualCameraGpuFacts, VirtualCameraOutputError> {
            Err(VirtualCameraOutputError::Unsupported(
                "AkVirtualCamera 输出仅支持 Windows 10/11",
            ))
        }

        pub fn activate(&mut self) -> Result<(), VirtualCameraOutputError> {
            Err(VirtualCameraOutputError::Unsupported(
                "AkVirtualCamera 输出仅支持 Windows 10/11",
            ))
        }

        pub fn stop(&mut self) -> Result<(), VirtualCameraOutputError> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_facts_are_explicitly_converted_to_fixed_contract() {
        let facts = VirtualCameraGpuFacts {
            capture_api: "windows_graphics_capture".to_owned(),
            adapter_luid: "luid".to_owned(),
            adapter_name: "gpu".to_owned(),
            vendor_id: 0x1002,
            device_id: 0x744c,
            feature_level: "0xb000".to_owned(),
        }
        .into_contract();
        assert!(facts.gpu_scale);
        assert!(facts.gpu_color_convert);
        assert!(!facts.zero_copy);
        assert_eq!(facts.width, 1280);
        assert_eq!(facts.height, 720);
        assert_eq!(facts.fps, 30);
    }

    #[test]
    fn runtime_uses_the_capture_session_adapter_identity() {
        let facts = VirtualCameraGpuFacts::from(CaptureGpuFacts {
            adapter_luid: "actual-luid".to_owned(),
            adapter_name: "Actual GPU".to_owned(),
            vendor_id: 0x10de,
            device_id: 0x2684,
            feature_level: "0xb000".to_owned(),
        })
        .into_contract();
        assert_eq!(facts.adapter_luid, "actual-luid");
        assert_eq!(facts.adapter_name, "Actual GPU");
        assert_eq!(facts.vendor_id, 0x10de);
        assert_eq!(facts.device_id, 0x2684);
        assert_eq!(facts.feature_level, "0xb000");
    }
}
