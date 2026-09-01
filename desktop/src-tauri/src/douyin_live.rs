use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

const MAX_REPLY_ITEMS: usize = 100;
const MAX_REPLY_BYTES: usize = 320;
const MAX_REPLY_CHARS: usize = 80;
const MIN_TIMEOUT_SEC: u64 = 30;
const MAX_TIMEOUT_SEC: u64 = 900;
const EVENT_HISTORY_LIMIT: usize = 32;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DouyinLiveProbeRequest {
    pub upstream_root: String,
    pub room_id: String,
    pub replies: Vec<String>,
    pub timeout_sec: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DouyinLiveProbeStatus {
    pub running: bool,
    pub state: String,
    pub last_event: Option<String>,
    pub event_history: Vec<String>,
    pub qr_path: Option<String>,
    pub room_resolved: bool,
    pub chat_received: bool,
    pub reply_attempted: bool,
    pub self_echo_filtered: bool,
    pub error: Option<String>,
}

impl Default for DouyinLiveProbeStatus {
    fn default() -> Self {
        Self {
            running: false,
            state: "idle".to_owned(),
            last_event: None,
            event_history: Vec::new(),
            qr_path: None,
            room_resolved: false,
            chat_received: false,
            reply_attempted: false,
            self_echo_filtered: false,
            error: None,
        }
    }
}

#[derive(Debug)]
struct Runtime {
    child: Option<Child>,
    status: DouyinLiveProbeStatus,
    stopping: bool,
}

#[derive(Debug, Clone)]
pub struct DouyinLiveState {
    runtime: Arc<Mutex<Runtime>>,
}

impl Default for DouyinLiveState {
    fn default() -> Self {
        Self {
            runtime: Arc::new(Mutex::new(Runtime {
                child: None,
                status: DouyinLiveProbeStatus::default(),
                stopping: false,
            })),
        }
    }
}

impl DouyinLiveState {
    pub fn shutdown(&self) {
        let _ = stop_runtime(&self.runtime);
    }
}

impl Drop for DouyinLiveState {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn invalid(message: impl Into<String>) -> String {
    message.into()
}

fn validate_upstream_root(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value.trim());
    if path.as_os_str().is_empty() {
        return Err(invalid("Douyin_Spider 路径不能为空"));
    }
    let root = path
        .canonicalize()
        .map_err(|_| invalid("Douyin_Spider 路径不存在或不可访问"))?;
    if !root.is_dir() {
        return Err(invalid("Douyin_Spider 路径必须是目录"));
    }
    for relative in [
        Path::new("builder/auth.py"),
        Path::new("dy_live/server.py"),
        Path::new("static/Live_pb2.py"),
    ] {
        if !root.join(relative).is_file() {
            return Err(invalid(format!(
                "Douyin_Spider 缺少必要文件：{}",
                relative.display()
            )));
        }
    }
    Ok(root)
}

fn validate_room_id(value: &str) -> Result<String, String> {
    let input = value.trim();
    let room = input
        .strip_prefix("https://live.douyin.com/")
        .unwrap_or(input);
    if room.is_empty() || room.len() > 20 || !room.as_bytes().iter().all(u8::is_ascii_digit) {
        return Err(invalid(
            "直播间号只接受 1～20 位数字或标准 live.douyin.com URL",
        ));
    }
    Ok(room.to_owned())
}

fn validate_replies(replies: &[String]) -> Result<Vec<String>, String> {
    let values = if replies.is_empty() {
        vec!["GpAutoLive探针✅".to_owned()]
    } else {
        replies.to_vec()
    };
    if values.len() > MAX_REPLY_ITEMS {
        return Err(invalid(format!("回复候选最多 {} 条", MAX_REPLY_ITEMS)));
    }
    let mut unique = Vec::with_capacity(values.len());
    for value in values {
        let text = value.trim();
        if text.is_empty()
            || text.chars().count() > MAX_REPLY_CHARS
            || text.len() > MAX_REPLY_BYTES
            || text.chars().any(|character| character.is_control())
        {
            return Err(invalid(
                "每条回复必须是 1～80 个可打印 Unicode 字符且不超过 320 UTF-8 字节",
            ));
        }
        if !unique.iter().any(|item: &String| item == text) {
            unique.push(text.to_owned());
        }
    }
    if unique.is_empty() {
        return Err(invalid("回复候选去重后至少保留一条"));
    }
    Ok(unique)
}

fn validate_request(
    request: &DouyinLiveProbeRequest,
) -> Result<(PathBuf, String, Vec<String>), String> {
    if !(MIN_TIMEOUT_SEC..=MAX_TIMEOUT_SEC).contains(&request.timeout_sec) {
        return Err(invalid(format!(
            "监听总超时必须在 {}～{} 秒",
            MIN_TIMEOUT_SEC, MAX_TIMEOUT_SEC
        )));
    }
    Ok((
        validate_upstream_root(&request.upstream_root)?,
        validate_room_id(&request.room_id)?,
        validate_replies(&request.replies)?,
    ))
}

fn probe_script_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tools")
        .join("douyin_live_compat_probe.py")
}

fn qr_path() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("gpautolive-douyin-probe-{suffix}.png"))
}

fn conda_env_name() -> String {
    let candidate =
        std::env::var("AUTOLIVE_CONDA_ENV").unwrap_or_else(|_| "gpautolive-douyin".to_owned());
    if (1..=64).contains(&candidate.len())
        && candidate.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        candidate
    } else {
        "gpautolive-douyin".to_owned()
    }
}

fn spawn_probe(
    script: &Path,
    upstream_root: &Path,
    room_id: &str,
    replies: &[String],
    timeout_sec: u64,
    qr_output: &Path,
) -> Result<Child, String> {
    if !script.is_file() {
        return Err(invalid("桌面开发探针脚本未找到；当前安装包尚未打包此能力"));
    }
    let mut args = vec![
        script.to_string_lossy().into_owned(),
        "--upstream-root".to_owned(),
        upstream_root.to_string_lossy().into_owned(),
        "--room-id".to_owned(),
        room_id.to_owned(),
        "--timeout".to_owned(),
        timeout_sec.to_string(),
        "--qr-output".to_owned(),
        qr_output.to_string_lossy().into_owned(),
    ];
    for reply in replies {
        args.push("--reply".to_owned());
        args.push(reply.clone());
    }

    let env_name = conda_env_name();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(conda_exe) = std::env::var_os("CONDA_EXE") {
        candidates.push(PathBuf::from(conda_exe));
    }
    candidates.push(PathBuf::from("conda"));

    for program in candidates {
        let result = Command::new(program)
            .arg("run")
            .arg("--no-capture-output")
            .arg("-n")
            .arg(&env_name)
            .arg("python")
            .args(&args)
            .current_dir(upstream_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(child) = result {
            return Ok(child);
        }
    }
    Err(invalid("未找到可用的 Python 3 运行时"))
}

fn set_event(runtime: &Arc<Mutex<Runtime>>, value: &Value, qr_output: &Path) {
    let Some(event) = value.get("event").and_then(Value::as_str) else {
        return;
    };
    let mut guard = match runtime.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    let status = &mut guard.status;
    status.last_event = Some(event.to_owned());
    status.event_history.push(event.to_owned());
    if status.event_history.len() > EVENT_HISTORY_LIMIT {
        let overflow = status.event_history.len() - EVENT_HISTORY_LIMIT;
        status.event_history.drain(..overflow);
    }
    match event {
        "probe_started" | "qr_waiting" | "qr_issued" => status.state = "waiting_qr".to_owned(),
        "login_confirmed" | "self_identity_ready" => status.state = "logged_in".to_owned(),
        "room_resolved" => {
            status.state = "room_resolved".to_owned();
            status.room_resolved = true;
        }
        "reply_selected" | "websocket_connected" => status.state = "listening".to_owned(),
        "chat_received" => {
            status.state = "chat_received".to_owned();
            status.chat_received = true;
        }
        "reply_attempted" => {
            status.state = "reply_attempted".to_owned();
            status.reply_attempted = true;
        }
        "self_echo_filtered" => {
            status.state = "self_echo_filtered".to_owned();
            status.self_echo_filtered = true;
        }
        "probe_passed" => {
            status.running = false;
            status.state = "passed".to_owned();
            status.reply_attempted = true;
            status.self_echo_filtered = true;
            status.qr_path = None;
        }
        "probe_inconclusive" => {
            status.running = false;
            status.state = "inconclusive".to_owned();
            status.error = Some("真实弹幕或自回显未在本轮超时前观察到".to_owned());
            status.qr_path = None;
        }
        "probe_failed" | "reply_failed" => {
            status.running = false;
            status.state = "failed".to_owned();
            status.error = Some("上游探针报告失败；详细正文未写入桌面日志".to_owned());
            status.qr_path = None;
        }
        _ => {}
    }
    if event == "qr_issued" && qr_output.is_file() {
        status.qr_path = Some(qr_output.to_string_lossy().into_owned());
    }
}

fn watch_probe(
    runtime: Arc<Mutex<Runtime>>,
    stdout: impl std::io::Read + Send + 'static,
    qr_output: PathBuf,
) {
    let reader = BufReader::new(stdout);
    for line in reader.lines().map_while(Result::ok) {
        if let Ok(value) = serde_json::from_str::<Value>(&line) {
            set_event(&runtime, &value, &qr_output);
        }
    }
    let mut guard = match runtime.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    if let Some(mut child) = guard.child.take() {
        let _ = child.wait();
    }
    if !guard.stopping && guard.status.running {
        guard.status.running = false;
        guard.status.state = "failed".to_owned();
        guard.status.error = Some("探针进程意外退出".to_owned());
    }
    guard.status.qr_path = None;
    guard.stopping = false;
}

fn stop_runtime(runtime: &Arc<Mutex<Runtime>>) -> Result<DouyinLiveProbeStatus, String> {
    let mut guard = runtime
        .lock()
        .map_err(|_| invalid("抖音探针状态锁已损坏"))?;
    guard.stopping = true;
    if let Some(mut child) = guard.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    guard.status.running = false;
    guard.status.state = "stopped".to_owned();
    guard.status.qr_path = None;
    guard.status.error = None;
    guard.stopping = false;
    Ok(guard.status.clone())
}

#[tauri::command]
pub fn start_douyin_live_probe(
    request: DouyinLiveProbeRequest,
    state: State<'_, DouyinLiveState>,
) -> Result<DouyinLiveProbeStatus, String> {
    let (upstream_root, room_id, replies) = validate_request(&request)?;
    let script = probe_script_path();
    let qr_output = qr_path();
    if qr_output.exists() {
        fs::remove_file(&qr_output).map_err(|_| invalid("无法清理上一次临时二维码"))?;
    }

    let mut guard = state
        .runtime
        .lock()
        .map_err(|_| invalid("抖音探针状态锁已损坏"))?;
    if guard.child.is_some() || guard.status.running {
        return Err(invalid("已有抖音探针在运行，请先停止当前会话"));
    }
    let mut child = spawn_probe(
        &script,
        &upstream_root,
        &room_id,
        &replies,
        request.timeout_sec,
        &qr_output,
    )?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| invalid("探针标准输出初始化失败"))?;
    guard.child = Some(child);
    guard.stopping = false;
    guard.status = DouyinLiveProbeStatus {
        running: true,
        state: "starting".to_owned(),
        last_event: None,
        event_history: Vec::new(),
        qr_path: None,
        room_resolved: false,
        chat_received: false,
        reply_attempted: false,
        self_echo_filtered: false,
        error: None,
    };
    let runtime = Arc::clone(&state.runtime);
    if thread::Builder::new()
        .name("douyin-live-probe".to_owned())
        .spawn(move || watch_probe(runtime, stdout, qr_output))
        .is_err()
    {
        if let Some(mut child) = guard.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        guard.status.running = false;
        guard.status.state = "failed".to_owned();
        guard.status.error = Some("无法启动抖音探针监控线程".to_owned());
        return Err(invalid("无法启动抖音探针监控线程"));
    }
    Ok(guard.status.clone())
}

#[tauri::command]
pub fn get_douyin_live_probe_status(
    state: State<'_, DouyinLiveState>,
) -> Result<DouyinLiveProbeStatus, String> {
    let guard = state
        .runtime
        .lock()
        .map_err(|_| invalid("抖音探针状态锁已损坏"))?;
    Ok(guard.status.clone())
}

#[tauri::command]
pub fn stop_douyin_live_probe(
    state: State<'_, DouyinLiveState>,
) -> Result<DouyinLiveProbeStatus, String> {
    stop_runtime(&state.runtime)
}

#[cfg(test)]
mod tests {
    use super::{validate_replies, validate_room_id, MAX_REPLY_BYTES, MAX_REPLY_CHARS};

    #[test]
    fn room_id_accepts_numeric_id_and_standard_url_only() {
        assert_eq!(validate_room_id(" 12345 ").unwrap(), "12345");
        assert_eq!(
            validate_room_id("https://live.douyin.com/12345").unwrap(),
            "12345"
        );
        assert!(validate_room_id("http://live.douyin.com/12345").is_err());
        assert!(validate_room_id("https://live.douyin.com/12345?x=1").is_err());
    }

    #[test]
    fn replies_are_trimmed_deduplicated_and_bounded() {
        assert_eq!(
            validate_replies(&[" A ".to_owned(), "A".to_owned(), "B".to_owned()]).unwrap(),
            vec!["A".to_owned(), "B".to_owned()]
        );
        assert!(validate_replies(&["x".repeat(MAX_REPLY_CHARS + 1)]).is_err());
        assert!(validate_replies(&["x".repeat(MAX_REPLY_BYTES + 1)]).is_err());
        assert!(validate_replies(&["\u{0000}".to_owned()]).is_err());
    }
}
