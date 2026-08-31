use super::{MpvCommand, RealtimeVideoBackendError};
use serde_json::Value;
use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const FIRST_REQUEST_ID: u64 = 1;
const SHUTDOWN_JOIN_TIMEOUT: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy)]
pub struct MpvIpcOptions {
    pub queue_capacity: usize,
    pub connect_timeout: Duration,
    pub connect_poll_interval: Duration,
    pub max_response_bytes: usize,
}

impl Default for MpvIpcOptions {
    fn default() -> Self {
        Self {
            queue_capacity: 32,
            connect_timeout: Duration::from_secs(5),
            connect_poll_interval: Duration::from_millis(20),
            max_response_bytes: 64 * 1024,
        }
    }
}

impl MpvIpcOptions {
    fn validate(self) -> Result<Self, RealtimeVideoBackendError> {
        if self.queue_capacity == 0
            || self.max_response_bytes == 0
            || self.connect_poll_interval.is_zero()
        {
            return Err(RealtimeVideoBackendError::IpcProtocol(
                "IPC 队列容量、连接轮询间隔和响应上限必须大于零".to_owned(),
            ));
        }
        Ok(self)
    }
}

type IpcResponse = Result<Value, RealtimeVideoBackendError>;

/// 已进入唯一持久 IPC worker 的请求。软等待超时不会关闭会话，调用方可继续轮询迟到响应；
/// 只有显式硬超时才会使会话失效，避免在结果未知时把同一副作用机械重发。
#[derive(Debug)]
pub struct PendingMpvResponse {
    request_id: u64,
    operation: &'static str,
    started_at: Instant,
    response: Option<Receiver<IpcResponse>>,
    session: Arc<SessionState>,
}

impl PendingMpvResponse {
    pub const fn request_id(&self) -> u64 {
        self.request_id
    }

    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    /// 非阻塞取得响应。`Ok(None)` 表示请求仍在唯一 IPC worker 中排队或等待 mpv 响应。
    pub fn poll(&mut self) -> Result<Option<Value>, RealtimeVideoBackendError> {
        let response = self.response.as_ref().ok_or_else(|| {
            RealtimeVideoBackendError::IpcProtocol(format!(
                "request_id {} 的响应已被消费",
                self.request_id
            ))
        })?;
        match response.try_recv() {
            Ok(result) => {
                self.response = None;
                result.map(Some)
            }
            Err(TryRecvError::Empty) => {
                if let Some(error) = self.session.failure() {
                    self.response = None;
                    Err(error)
                } else {
                    Ok(None)
                }
            }
            Err(TryRecvError::Disconnected) => {
                self.response = None;
                Err(self.session.failure().unwrap_or_else(|| {
                    RealtimeVideoBackendError::IpcDisconnected("IPC 响应通道已断开".to_owned())
                }))
            }
        }
    }

    /// 有界软等待。超时返回 `Ok(None)`，请求及会话仍保持有效，可随后再次调用。
    pub fn wait(&mut self, deadline: Duration) -> Result<Option<Value>, RealtimeVideoBackendError> {
        let response = self.response.as_ref().ok_or_else(|| {
            RealtimeVideoBackendError::IpcProtocol(format!(
                "request_id {} 的响应已被消费",
                self.request_id
            ))
        })?;
        match response.recv_timeout(deadline) {
            Ok(result) => {
                self.response = None;
                result.map(Some)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(error) = self.session.failure() {
                    self.response = None;
                    Err(error)
                } else {
                    Ok(None)
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.response = None;
                Err(self.session.failure().unwrap_or_else(|| {
                    RealtimeVideoBackendError::IpcDisconnected("IPC 响应通道已断开".to_owned())
                }))
            }
        }
    }

    /// 在 actor tick 中轮询，并在请求总年龄越过硬上限时关闭会话。
    pub fn poll_with_hard_timeout(
        &mut self,
        hard_timeout: Duration,
    ) -> Result<Option<Value>, RealtimeVideoBackendError> {
        match self.poll()? {
            Some(response) => Ok(Some(response)),
            None if self.started_at.elapsed() < hard_timeout => Ok(None),
            None => Err(self.expire()),
        }
    }

    fn expire(&mut self) -> RealtimeVideoBackendError {
        self.response = None;
        self.session.close(format!(
            "request_id {}（{}）硬超时，会话已失效以隔离迟到响应",
            self.request_id, self.operation
        ));
        RealtimeVideoBackendError::IpcTimeout {
            request_id: self.request_id,
            operation: self.operation,
        }
    }
}

#[derive(Debug)]
struct IpcRequest {
    request_id: u64,
    operation: &'static str,
    line: String,
    response: SyncSender<IpcResponse>,
}

#[derive(Debug, Default)]
struct SessionState {
    closed: Mutex<Option<String>>,
}

impl SessionState {
    fn close(&self, message: String) {
        if let Ok(mut closed) = self.closed.lock() {
            if closed.is_none() {
                *closed = Some(message);
            }
        }
    }

    fn failure(&self) -> Option<RealtimeVideoBackendError> {
        match self.closed.lock() {
            Ok(closed) => closed
                .as_ref()
                .map(|message| RealtimeVideoBackendError::IpcDisconnected(message.clone())),
            Err(_) => Some(RealtimeVideoBackendError::IpcDisconnected(
                "IPC 会话状态锁已中毒".to_owned(),
            )),
        }
    }
}

pub struct MpvIpcClient {
    sender: Option<SyncSender<IpcRequest>>,
    session: Arc<SessionState>,
    next_request_id: AtomicU64,
    worker_join: Option<JoinHandle<()>>,
    worker_done: Receiver<()>,
}

impl std::fmt::Debug for MpvIpcClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MpvIpcClient")
            .field(
                "connected",
                &(self.sender.is_some() && self.session.failure().is_none()),
            )
            .finish_non_exhaustive()
    }
}

impl MpvIpcClient {
    #[cfg(windows)]
    pub fn connect(
        ipc_pipe: &str,
        options: MpvIpcOptions,
    ) -> Result<Self, RealtimeVideoBackendError> {
        let options = options.validate()?;
        let started = Instant::now();
        let connection = loop {
            match OpenOptions::new().read(true).write(true).open(ipc_pipe) {
                Ok(connection) => break connection,
                Err(error) if started.elapsed() < options.connect_timeout => {
                    thread::sleep(options.connect_poll_interval);
                    let _ = error;
                }
                Err(error) => {
                    return Err(RealtimeVideoBackendError::IpcDisconnected(format!(
                        "连接命名管道超时：{error}"
                    )))
                }
            }
        };
        Self::from_io(connection, options)
    }

    #[cfg(not(windows))]
    pub fn connect(
        _ipc_pipe: &str,
        _options: MpvIpcOptions,
    ) -> Result<Self, RealtimeVideoBackendError> {
        Err(RealtimeVideoBackendError::IpcDisconnected(
            "mpv 命名管道 IPC 当前只支持 Windows".to_owned(),
        ))
    }

    fn from_io<T>(connection: T, options: MpvIpcOptions) -> Result<Self, RealtimeVideoBackendError>
    where
        T: Read + Write + Send + 'static,
    {
        let options = options.validate()?;
        let session = Arc::new(SessionState::default());
        let (sender, receiver) = mpsc::sync_channel(options.queue_capacity);
        let (worker_done_sender, worker_done) = mpsc::sync_channel(1);
        let worker_join = spawn_worker(
            connection,
            receiver,
            Arc::clone(&session),
            options.max_response_bytes,
            worker_done_sender,
        )?;
        Ok(Self {
            sender: Some(sender),
            session,
            next_request_id: AtomicU64::new(FIRST_REQUEST_ID),
            worker_join: Some(worker_join),
            worker_done,
        })
    }

    pub fn send(
        &self,
        command: &MpvCommand,
        deadline: Duration,
    ) -> Result<Value, RealtimeVideoBackendError> {
        let mut pending = self.submit(command)?;
        match pending.wait(deadline)? {
            Some(response) => Ok(response),
            None => Err(pending.expire()),
        }
    }

    pub(crate) fn submit(
        &self,
        command: &MpvCommand,
    ) -> Result<PendingMpvResponse, RealtimeVideoBackendError> {
        if let Some(error) = self.session.failure() {
            return Err(error);
        }
        let request_id = self
            .next_request_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| RealtimeVideoBackendError::IpcProtocol("request_id 已耗尽".to_owned()))?;
        let line = command.ipc_json_line_with_request_id(request_id)?;
        let (response_sender, response) = mpsc::sync_channel(1);
        let Some(sender) = self.sender.as_ref() else {
            return Err(RealtimeVideoBackendError::IpcDisconnected(
                "IPC worker 已关闭".to_owned(),
            ));
        };
        match sender.try_send(IpcRequest {
            request_id,
            operation: command.operation_name(),
            line,
            response: response_sender,
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(RealtimeVideoBackendError::IpcQueueFull),
            Err(TrySendError::Disconnected(_)) => {
                self.session.close("IPC worker 已断开".to_owned());
                return Err(RealtimeVideoBackendError::IpcDisconnected(
                    "IPC worker 已断开".to_owned(),
                ));
            }
        }
        Ok(PendingMpvResponse {
            request_id,
            operation: command.operation_name(),
            started_at: Instant::now(),
            response: Some(response),
            session: Arc::clone(&self.session),
        })
    }

    pub fn shutdown(&mut self) -> Result<(), RealtimeVideoBackendError> {
        if self.sender.is_none() && self.worker_join.is_none() {
            return Ok(());
        }
        self.session.close("IPC 会话关闭".to_owned());
        self.sender.take();
        let worker_finished = match self.worker_done.recv_timeout(SHUTDOWN_JOIN_TIMEOUT) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => true,
            Err(mpsc::RecvTimeoutError::Timeout) => false,
        };
        if !worker_finished {
            // 同步命名管道读取只能由对端关闭来解除；ManagedMpvProcess 会先终止 mpv。
            // 若仍未解除，必须暴露所有权异常，不能把被迫放弃 JoinHandle 伪装成成功。
            self.worker_join.take();
            return Err(RealtimeVideoBackendError::IpcDisconnected(
                "IPC 对端未关闭，worker 未能在回收期限内退出".to_owned(),
            ));
        }
        if let Some(join) = self.worker_join.take() {
            if join.join().is_err() {
                return Err(RealtimeVideoBackendError::IpcDisconnected(
                    "IPC worker 线程发生 panic".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

impl Drop for MpvIpcClient {
    fn drop(&mut self) {
        let _ignored = self.shutdown();
    }
}

fn spawn_worker<T>(
    connection: T,
    receiver: Receiver<IpcRequest>,
    session: Arc<SessionState>,
    max_response_bytes: usize,
    done: SyncSender<()>,
) -> Result<JoinHandle<()>, RealtimeVideoBackendError>
where
    T: Read + Write + Send + 'static,
{
    thread::Builder::new()
        .name("mpv-ipc-worker".to_owned())
        .spawn(move || {
            run_worker(connection, receiver, &session, max_response_bytes);
            let _ignored = done.send(());
        })
        .map_err(|error| RealtimeVideoBackendError::IpcDisconnected(error.to_string()))
}

fn run_worker<T>(
    connection: T,
    receiver: Receiver<IpcRequest>,
    session: &SessionState,
    max_response_bytes: usize,
) where
    T: Read + Write,
{
    let mut connection = BufReader::new(connection);
    while let Ok(request) = receiver.recv() {
        if let Some(error) = session.failure() {
            let _ignored = request.response.send(Err(error));
            break;
        }
        if let Err(error) = connection
            .get_mut()
            .write_all(request.line.as_bytes())
            .and_then(|_| connection.get_mut().flush())
        {
            let message = format!("写入 request_id {} 失败：{error}", request.request_id);
            session.close(message.clone());
            let _ignored = request
                .response
                .send(Err(RealtimeVideoBackendError::IpcDisconnected(message)));
            break;
        }
        match read_response(
            &mut connection,
            request.request_id,
            request.operation,
            max_response_bytes,
        ) {
            Ok(response) => {
                if session.failure().is_some() {
                    break;
                }
                let _ignored = request.response.send(response);
            }
            Err(error) => {
                session.close(error.to_string());
                let _ignored = request.response.send(Err(error));
                break;
            }
        }
    }
}

fn read_response<R: BufRead>(
    reader: &mut R,
    expected_request_id: u64,
    operation: &'static str,
    max_response_bytes: usize,
) -> Result<IpcResponse, RealtimeVideoBackendError> {
    loop {
        let line = read_bounded_line(reader, max_response_bytes)
            .map_err(|error| {
                RealtimeVideoBackendError::IpcDisconnected(format!(
                    "读取 mpv IPC 响应失败：{error}"
                ))
            })?
            .ok_or_else(|| {
                RealtimeVideoBackendError::IpcDisconnected("mpv 已关闭 IPC 管道".to_owned())
            })?;
        let value: Value = serde_json::from_slice(&line).map_err(|error| {
            RealtimeVideoBackendError::IpcProtocol(format!("解析 mpv IPC 响应失败：{error}"))
        })?;
        let Some(request_id) = value.get("request_id") else {
            if value.get("event").and_then(Value::as_str).is_some() {
                continue;
            }
            return Err(RealtimeVideoBackendError::IpcProtocol(
                "mpv 非事件响应缺少 request_id".to_owned(),
            ));
        };
        let request_id = request_id.as_u64().ok_or_else(|| {
            RealtimeVideoBackendError::IpcProtocol("mpv 响应的 request_id 无效".to_owned())
        })?;
        let response = value.as_object().ok_or_else(|| {
            RealtimeVideoBackendError::IpcProtocol("mpv IPC 响应必须是对象".to_owned())
        })?;
        if response
            .keys()
            .any(|key| !matches!(key.as_str(), "error" | "data" | "request_id"))
        {
            return Err(RealtimeVideoBackendError::IpcProtocol(
                "mpv IPC 响应包含未知字段".to_owned(),
            ));
        }
        if request_id != expected_request_id {
            return Err(RealtimeVideoBackendError::IpcProtocol(format!(
                "等待 request_id {expected_request_id} 时收到 request_id {request_id}"
            )));
        }
        return Ok(match value.get("error").and_then(Value::as_str) {
            Some("success") => Ok(value),
            Some(error) if is_property_unavailable_error(error) => {
                Err(RealtimeVideoBackendError::PropertyUnavailable {
                    request_id,
                    operation,
                    property_error: error.to_owned(),
                })
            }
            Some(error) => Err(RealtimeVideoBackendError::IpcProtocol(format!(
                "mpv 请求 {request_id} 失败：{error}"
            ))),
            None => {
                return Err(RealtimeVideoBackendError::IpcProtocol(format!(
                    "mpv 请求 {request_id} 响应缺少 error 字段"
                )))
            }
        });
    }
}

fn is_property_unavailable_error(error: &str) -> bool {
    let error = error.trim().to_ascii_lowercase();
    matches!(
        error.as_str(),
        "property unavailable" | "property not found"
    )
}

fn read_bounded_line<R: BufRead>(reader: &mut R, max_bytes: usize) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::with_capacity(max_bytes.min(1024));
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(line))
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(available.len(), |index| index + 1);
        if line.len().saturating_add(take) > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "mpv IPC 单行响应超过大小上限",
            ));
        }
        line.extend_from_slice(&available[..take]);
        reader.consume(take);
        if newline.is_some() {
            return Ok(Some(line));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Shutdown, TcpListener, TcpStream};

    fn connected_streams() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let address = listener.local_addr().expect("read test address");
        let client = TcpStream::connect(address).expect("connect test client");
        let (server, _) = listener.accept().expect("accept test connection");
        (client, server)
    }

    fn test_client(stream: TcpStream) -> MpvIpcClient {
        MpvIpcClient::from_io(
            stream,
            MpvIpcOptions {
                queue_capacity: 2,
                max_response_bytes: 1024,
                ..MpvIpcOptions::default()
            },
        )
        .expect("create test IPC client")
    }

    fn response_for(request: &str, data: &str) -> String {
        let request: Value = serde_json::from_str(request).expect("parse request");
        let request_id = request["request_id"].as_u64().expect("request id");
        format!("{{\"error\":\"success\",\"data\":{data},\"request_id\":{request_id}}}\n")
    }

    #[test]
    fn one_owner_processes_requests_in_order_and_ignores_events() {
        let (client_stream, mut server_stream) = connected_streams();
        let server_reader = server_stream.try_clone().expect("clone server reader");
        let server = thread::spawn(move || {
            let mut reader = BufReader::new(server_reader);
            let mut first = String::new();
            reader.read_line(&mut first).expect("read first request");
            server_stream
                .write_all(b"{\"event\":\"tick\"}\n")
                .and_then(|_| server_stream.write_all(response_for(&first, "true").as_bytes()))
                .expect("write first response");

            let mut second = String::new();
            reader.read_line(&mut second).expect("read second request");
            server_stream
                .write_all(response_for(&second, "false").as_bytes())
                .expect("write second response");
        });
        let mut client = test_client(client_stream);
        let first = client
            .send(
                &MpvCommand::GetVideoOutputConfigured,
                Duration::from_secs(1),
            )
            .expect("first response");
        let second = client
            .send(
                &MpvCommand::GetVideoOutputConfigured,
                Duration::from_secs(1),
            )
            .expect("second response");
        assert_eq!(first["data"], Value::Bool(true));
        assert_eq!(second["data"], Value::Bool(false));
        server.join().expect("join server");
        client.shutdown().expect("shutdown client");
    }

    #[test]
    fn property_unavailable_has_a_typed_nonfatal_response() {
        let input = b"{\"error\":\"property unavailable\",\"request_id\":7}\n";
        let response = read_response(&mut io::Cursor::new(input), 7, "get time-pos", 1024)
            .expect("valid response frame")
            .expect_err("property must be unavailable");
        assert!(matches!(
            response,
            RealtimeVideoBackendError::PropertyUnavailable {
                request_id: 7,
                operation: "get time-pos",
                ..
            }
        ));
    }

    #[test]
    fn timeout_poisoning_rejects_reuse_before_late_response_arrives() {
        let (client_stream, mut server_stream) = connected_streams();
        let server_reader = server_stream.try_clone().expect("clone server reader");
        let server = thread::spawn(move || {
            let mut request = String::new();
            BufReader::new(server_reader)
                .read_line(&mut request)
                .expect("read request");
            thread::sleep(Duration::from_millis(50));
            let _ignored = server_stream.write_all(response_for(&request, "true").as_bytes());
        });
        let mut client = test_client(client_stream);
        let error = client
            .send(
                &MpvCommand::GetVideoOutputConfigured,
                Duration::from_millis(5),
            )
            .expect_err("first request must time out");
        assert!(matches!(
            error,
            RealtimeVideoBackendError::IpcTimeout { .. }
        ));
        let error = client
            .send(
                &MpvCommand::GetVideoOutputConfigured,
                Duration::from_secs(1),
            )
            .expect_err("poisoned session must reject reuse");
        assert!(matches!(
            error,
            RealtimeVideoBackendError::IpcDisconnected(_)
        ));
        server.join().expect("join server");
        client.shutdown().expect("shutdown client");
    }

    #[test]
    fn pending_request_keeps_late_response_and_allows_follow_up_readback() {
        let (client_stream, mut server_stream) = connected_streams();
        let server_reader = server_stream.try_clone().expect("clone server reader");
        let server = thread::spawn(move || {
            let mut reader = BufReader::new(server_reader);
            let mut set_request = String::new();
            reader
                .read_line(&mut set_request)
                .expect("read delayed set request");
            thread::sleep(Duration::from_millis(40));
            server_stream
                .write_all(response_for(&set_request, "null").as_bytes())
                .expect("write delayed set response");

            let mut get_request = String::new();
            reader
                .read_line(&mut get_request)
                .expect("read readback request");
            server_stream
                .write_all(
                    response_for(
                        &get_request,
                        r#""al_runtime_plan_hi=12,al_runtime_plan_lo=34""#,
                    )
                    .as_bytes(),
                )
                .expect("write readback response");
        });
        let mut client = test_client(client_stream);
        let options = super::super::MpvShaderOptions::parse(
            "al_runtime_plan_hi=12,al_runtime_plan_lo=34".to_owned(),
        )
        .expect("valid options");
        let mut pending = client
            .submit(&MpvCommand::SetShaderOptions { options })
            .expect("submit set request");
        assert_eq!(
            pending.wait(Duration::from_millis(5)).expect("soft wait"),
            None
        );
        assert!(pending
            .wait(Duration::from_secs(1))
            .expect("late response")
            .is_some());

        let readback = client
            .send(&MpvCommand::GetShaderOptions, Duration::from_secs(1))
            .expect("read shader options");
        assert_eq!(
            readback["data"],
            Value::String("al_runtime_plan_hi=12,al_runtime_plan_lo=34".to_owned())
        );
        server.join().expect("join server");
        client.shutdown().expect("shutdown client");
    }

    #[test]
    fn pending_request_hard_timeout_closes_session_and_releases_after_peer_closes() {
        let (client_stream, server_stream) = connected_streams();
        let mut client = test_client(client_stream);
        let mut pending = client
            .submit(&MpvCommand::GetVideoOutputConfigured)
            .expect("submit request");
        thread::sleep(Duration::from_millis(5));
        let error = pending
            .poll_with_hard_timeout(Duration::from_millis(1))
            .expect_err("hard timeout must fail");
        assert!(matches!(
            error,
            RealtimeVideoBackendError::IpcTimeout { .. }
        ));
        drop(pending);
        drop(server_stream);
        client.shutdown().expect("peer close releases worker");
    }

    #[test]
    fn peer_disconnect_fails_the_active_request_and_closes_session() {
        let (client_stream, server_stream) = connected_streams();
        server_stream
            .shutdown(Shutdown::Both)
            .expect("disconnect server");
        let mut client = test_client(client_stream);
        let error = client
            .send(
                &MpvCommand::GetVideoOutputConfigured,
                Duration::from_secs(1),
            )
            .expect_err("disconnected peer must fail");
        assert!(matches!(
            error,
            RealtimeVideoBackendError::IpcDisconnected(_)
        ));
        assert!(matches!(
            client.send(
                &MpvCommand::GetVideoOutputConfigured,
                Duration::from_secs(1)
            ),
            Err(RealtimeVideoBackendError::IpcDisconnected(_))
        ));
        client.shutdown().expect("shutdown client");
    }

    #[test]
    fn shutdown_does_not_wait_forever_for_a_blocked_sync_read() {
        let (client_stream, server_stream) = connected_streams();
        let mut client = test_client(client_stream);
        let request = thread::spawn(move || {
            client
                .send(
                    &MpvCommand::GetVideoOutputConfigured,
                    Duration::from_millis(5),
                )
                .expect_err("request must time out");
            let started = Instant::now();
            assert!(matches!(
                client.shutdown(),
                Err(RealtimeVideoBackendError::IpcDisconnected(_))
            ));
            drop(client);
            started.elapsed()
        });
        let elapsed = request.join().expect("join request thread");
        assert!(elapsed < Duration::from_secs(1));
        drop(server_stream);
    }

    #[test]
    fn mismatched_response_id_is_a_protocol_error_and_poisoning_boundary() {
        let (client_stream, mut server_stream) = connected_streams();
        let server_reader = server_stream.try_clone().expect("clone server reader");
        let server = thread::spawn(move || {
            let mut request = String::new();
            BufReader::new(server_reader)
                .read_line(&mut request)
                .expect("read request");
            server_stream
                .write_all(b"{\"error\":\"success\",\"request_id\":999}\n")
                .expect("write mismatched response");
        });
        let mut client = test_client(client_stream);
        let error = client
            .send(
                &MpvCommand::GetVideoOutputConfigured,
                Duration::from_secs(1),
            )
            .expect_err("mismatched id must fail");
        assert!(matches!(error, RealtimeVideoBackendError::IpcProtocol(_)));
        assert!(matches!(
            client.send(
                &MpvCommand::GetVideoOutputConfigured,
                Duration::from_secs(1)
            ),
            Err(RealtimeVideoBackendError::IpcDisconnected(_))
        ));
        server.join().expect("join server");
        client.shutdown().expect("shutdown client");
    }

    #[test]
    fn oversized_response_is_rejected_without_unbounded_allocation() {
        let mut input = io::Cursor::new(vec![b'x'; 9]);
        let error = read_bounded_line(&mut input, 8).expect_err("line must be bounded");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn non_event_without_request_id_is_protocol_error() {
        let input = b"{\"error\":\"success\",\"data\":true}\n";
        assert!(matches!(
            read_response(&mut io::Cursor::new(input), 7, "get time-pos", 1024),
            Err(RealtimeVideoBackendError::IpcProtocol(_))
        ));
    }

    #[test]
    fn response_with_unknown_field_is_protocol_error() {
        let input = b"{\"error\":\"success\",\"data\":true,\"request_id\":7,\"unknown\":1}\n";
        assert!(matches!(
            read_response(&mut io::Cursor::new(input), 7, "get vo-configured", 1024),
            Err(RealtimeVideoBackendError::IpcProtocol(_))
        ));
    }

    #[test]
    fn only_exact_property_errors_are_retryable() {
        assert!(is_property_unavailable_error("property unavailable"));
        assert!(is_property_unavailable_error("PROPERTY NOT FOUND"));
        assert!(!is_property_unavailable_error("command unavailable"));
    }
}
