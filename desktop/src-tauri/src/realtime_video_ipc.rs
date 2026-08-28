use super::{MpvCommand, RealtimeVideoBackendError};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const FIRST_REQUEST_ID: u64 = 1;

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

#[derive(Debug)]
struct WriteRequest {
    request_id: u64,
    line: String,
}

type IpcResponse = Result<Value, RealtimeVideoBackendError>;

#[derive(Debug)]
struct PendingState {
    waiters: HashMap<u64, SyncSender<IpcResponse>>,
    expired: VecDeque<u64>,
    closed: Option<String>,
}

#[derive(Debug)]
struct PendingResponses {
    state: Mutex<PendingState>,
    capacity: usize,
}

impl PendingResponses {
    fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(PendingState {
                waiters: HashMap::with_capacity(capacity),
                expired: VecDeque::with_capacity(capacity),
                closed: None,
            }),
            capacity,
        }
    }

    fn register(
        &self,
        request_id: u64,
        waiter: SyncSender<IpcResponse>,
    ) -> Result<(), RealtimeVideoBackendError> {
        let mut state = self.lock()?;
        if let Some(message) = &state.closed {
            return Err(RealtimeVideoBackendError::IpcDisconnected(message.clone()));
        }
        if state.waiters.len() >= self.capacity {
            return Err(RealtimeVideoBackendError::IpcQueueFull);
        }
        if state.waiters.contains_key(&request_id) {
            return Err(RealtimeVideoBackendError::IpcProtocol(format!(
                "request_id {request_id} 重复注册"
            )));
        }
        state.waiters.insert(request_id, waiter);
        Ok(())
    }

    fn remove(&self, request_id: u64) -> Result<(), RealtimeVideoBackendError> {
        self.lock()?.waiters.remove(&request_id);
        Ok(())
    }

    fn expire(&self, request_id: u64) -> Result<(), RealtimeVideoBackendError> {
        let mut state = self.lock()?;
        if state.waiters.remove(&request_id).is_some() {
            state.expired.push_back(request_id);
            while state.expired.len() > self.capacity {
                state.expired.pop_front();
            }
        }
        Ok(())
    }

    fn resolve(&self, value: Value) -> Result<(), RealtimeVideoBackendError> {
        let request_id = value
            .get("request_id")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                RealtimeVideoBackendError::IpcProtocol("mpv 响应缺少有效 request_id".to_owned())
            })?;
        let waiter = {
            let mut state = self.lock()?;
            if let Some(waiter) = state.waiters.remove(&request_id) {
                Some(waiter)
            } else if let Some(index) = state.expired.iter().position(|id| *id == request_id) {
                state.expired.remove(index);
                None
            } else {
                return Err(RealtimeVideoBackendError::IpcProtocol(format!(
                    "收到未知或重复 request_id {request_id}"
                )));
            }
        };
        let Some(waiter) = waiter else {
            return Ok(());
        };
        let response = match value.get("error").and_then(Value::as_str) {
            Some("success") => Ok(value),
            Some(error) => Err(RealtimeVideoBackendError::IpcProtocol(format!(
                "mpv 请求 {request_id} 失败：{error}"
            ))),
            None => Err(RealtimeVideoBackendError::IpcProtocol(format!(
                "mpv 请求 {request_id} 响应缺少 error 字段"
            ))),
        };
        let _ignored = waiter.send(response);
        Ok(())
    }

    fn fail_one(&self, request_id: u64, message: String) {
        let waiter = self
            .state
            .lock()
            .ok()
            .and_then(|mut state| state.waiters.remove(&request_id));
        if let Some(waiter) = waiter {
            let _ignored = waiter.send(Err(RealtimeVideoBackendError::IpcDisconnected(message)));
        }
    }

    fn close(&self, message: String) {
        let waiters = match self.state.lock() {
            Ok(mut state) => {
                if state.closed.is_none() {
                    state.closed = Some(message.clone());
                }
                std::mem::take(&mut state.waiters)
            }
            Err(_) => return,
        };
        for (_, waiter) in waiters {
            let _ignored = waiter.send(Err(RealtimeVideoBackendError::IpcDisconnected(
                message.clone(),
            )));
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, PendingState>, RealtimeVideoBackendError> {
        self.state.lock().map_err(|_| {
            RealtimeVideoBackendError::IpcDisconnected("IPC pending 状态锁已中毒".to_owned())
        })
    }
}

pub struct MpvIpcClient {
    sender: Option<SyncSender<WriteRequest>>,
    pending: Arc<PendingResponses>,
    next_request_id: AtomicU64,
    writer_join: Option<JoinHandle<()>>,
    reader_join: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for MpvIpcClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MpvIpcClient")
            .field("connected", &self.sender.is_some())
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
        let reader = connection.try_clone().map_err(|error| {
            RealtimeVideoBackendError::IpcDisconnected(format!("复制命名管道句柄失败：{error}"))
        })?;
        Self::from_io(reader, connection, options)
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

    fn from_io<R, W>(
        reader: R,
        writer: W,
        options: MpvIpcOptions,
    ) -> Result<Self, RealtimeVideoBackendError>
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
    {
        let options = options.validate()?;
        let pending = Arc::new(PendingResponses::new(options.queue_capacity));
        let (sender, receiver) = mpsc::sync_channel(options.queue_capacity);
        let writer_join = spawn_writer(writer, receiver, Arc::clone(&pending))?;
        let reader_join =
            match spawn_reader(reader, Arc::clone(&pending), options.max_response_bytes) {
                Ok(join) => join,
                Err(error) => {
                    drop(sender);
                    let _ignored = writer_join.join();
                    return Err(error);
                }
            };
        Ok(Self {
            sender: Some(sender),
            pending,
            next_request_id: AtomicU64::new(FIRST_REQUEST_ID),
            writer_join: Some(writer_join),
            reader_join: Some(reader_join),
        })
    }

    pub fn send(
        &self,
        command: &MpvCommand,
        deadline: Duration,
    ) -> Result<Value, RealtimeVideoBackendError> {
        let request_id = self
            .next_request_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| RealtimeVideoBackendError::IpcProtocol("request_id 已耗尽".to_owned()))?;
        let line = command.ipc_json_line_with_request_id(request_id)?;
        let (waiter, response) = mpsc::sync_channel(1);
        self.pending.register(request_id, waiter)?;
        let Some(sender) = self.sender.as_ref() else {
            self.pending.remove(request_id)?;
            return Err(RealtimeVideoBackendError::IpcDisconnected(
                "IPC writer 已关闭".to_owned(),
            ));
        };
        match sender.try_send(WriteRequest { request_id, line }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.pending.remove(request_id)?;
                return Err(RealtimeVideoBackendError::IpcQueueFull);
            }
            Err(TrySendError::Disconnected(_)) => {
                self.pending.remove(request_id)?;
                return Err(RealtimeVideoBackendError::IpcDisconnected(
                    "IPC writer 已断开".to_owned(),
                ));
            }
        }
        match response.recv_timeout(deadline) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending.expire(request_id)?;
                Err(RealtimeVideoBackendError::IpcTimeout { request_id })
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.pending.remove(request_id)?;
                Err(RealtimeVideoBackendError::IpcDisconnected(
                    "IPC 响应通道已断开".to_owned(),
                ))
            }
        }
    }

    pub fn shutdown(&mut self) -> Result<(), RealtimeVideoBackendError> {
        self.sender.take();
        self.pending.close("IPC 会话关闭".to_owned());
        let mut first_error = None;
        join_thread(&mut self.writer_join, "writer", &mut first_error);
        join_thread(&mut self.reader_join, "reader", &mut first_error);
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for MpvIpcClient {
    fn drop(&mut self) {
        let _ignored = self.shutdown();
    }
}

fn spawn_writer<W>(
    mut writer: W,
    receiver: Receiver<WriteRequest>,
    pending: Arc<PendingResponses>,
) -> Result<JoinHandle<()>, RealtimeVideoBackendError>
where
    W: Write + Send + 'static,
{
    thread::Builder::new()
        .name("mpv-ipc-writer".to_owned())
        .spawn(move || {
            while let Ok(request) = receiver.recv() {
                if let Err(error) = writer
                    .write_all(request.line.as_bytes())
                    .and_then(|_| writer.flush())
                {
                    let message = format!("写入 request_id {} 失败：{error}", request.request_id);
                    pending.fail_one(request.request_id, message.clone());
                    pending.close(message);
                    break;
                }
            }
        })
        .map_err(|error| RealtimeVideoBackendError::IpcDisconnected(error.to_string()))
}

fn spawn_reader<R>(
    reader: R,
    pending: Arc<PendingResponses>,
    max_response_bytes: usize,
) -> Result<JoinHandle<()>, RealtimeVideoBackendError>
where
    R: Read + Send + 'static,
{
    thread::Builder::new()
        .name("mpv-ipc-reader".to_owned())
        .spawn(move || {
            let mut reader = BufReader::new(reader);
            loop {
                let line = match read_bounded_line(&mut reader, max_response_bytes) {
                    Ok(Some(line)) => line,
                    Ok(None) => {
                        pending.close("mpv 已关闭 IPC 管道".to_owned());
                        break;
                    }
                    Err(error) => {
                        pending.close(format!("读取 mpv IPC 响应失败：{error}"));
                        break;
                    }
                };
                let value: Value = match serde_json::from_slice(&line) {
                    Ok(value) => value,
                    Err(error) => {
                        pending.close(format!("解析 mpv IPC 响应失败：{error}"));
                        break;
                    }
                };
                if value.get("request_id").is_none() {
                    continue;
                }
                if let Err(error) = pending.resolve(value) {
                    pending.close(error.to_string());
                    break;
                }
            }
        })
        .map_err(|error| RealtimeVideoBackendError::IpcDisconnected(error.to_string()))
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

fn join_thread(
    join: &mut Option<JoinHandle<()>>,
    name: &'static str,
    first_error: &mut Option<RealtimeVideoBackendError>,
) {
    if let Some(join) = join.take() {
        if join.join().is_err() && first_error.is_none() {
            *first_error = Some(RealtimeVideoBackendError::IpcDisconnected(format!(
                "IPC {name} 线程发生 panic"
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;

    fn connected_streams() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let address = listener.local_addr().expect("read test address");
        let client = TcpStream::connect(address).expect("connect test client");
        let (server, _) = listener.accept().expect("accept test connection");
        (client, server)
    }

    fn test_client(stream: &TcpStream) -> MpvIpcClient {
        MpvIpcClient::from_io(
            stream.try_clone().expect("clone test reader"),
            stream.try_clone().expect("clone test writer"),
            MpvIpcOptions {
                queue_capacity: 2,
                max_response_bytes: 1024,
                ..MpvIpcOptions::default()
            },
        )
        .expect("create test IPC client")
    }

    #[test]
    fn persistent_connection_matches_response_request_id() {
        let (client_stream, mut server_stream) = connected_streams();
        let server_reader = server_stream.try_clone().expect("clone server reader");
        let server = thread::spawn(move || {
            let mut request = String::new();
            BufReader::new(server_reader)
                .read_line(&mut request)
                .expect("read request");
            let request: Value = serde_json::from_str(&request).expect("parse request");
            let request_id = request["request_id"].as_u64().expect("request id");
            server_stream
                .write_all(b"{\"event\":\"tick\"}\n")
                .and_then(|_| {
                    server_stream.write_all(
                        format!(
                            "{{\"error\":\"success\",\"data\":true,\"request_id\":{request_id}}}\n"
                        )
                        .as_bytes(),
                    )
                })
                .expect("write response");
        });
        let mut client = test_client(&client_stream);
        drop(client_stream);
        let response = client
            .send(
                &MpvCommand::GetVideoOutputConfigured,
                Duration::from_secs(1),
            )
            .expect("matched response");
        assert_eq!(response["data"], Value::Bool(true));
        server.join().expect("join server");
        client.shutdown().expect("join IPC threads");
    }

    #[test]
    fn missing_response_hits_the_request_deadline() {
        let (client_stream, server_stream) = connected_streams();
        let mut client = test_client(&client_stream);
        drop(client_stream);
        let error = client
            .send(
                &MpvCommand::GetVideoOutputConfigured,
                Duration::from_millis(1),
            )
            .expect_err("response must time out");
        assert!(matches!(
            error,
            RealtimeVideoBackendError::IpcTimeout { .. }
        ));
        drop(server_stream);
        client.shutdown().expect("join IPC threads");
    }

    #[test]
    fn out_of_order_responses_are_routed_to_their_request_ids() {
        let (client_stream, mut server_stream) = connected_streams();
        let server_reader = server_stream.try_clone().expect("clone server reader");
        let server = thread::spawn(move || {
            let mut reader = BufReader::new(server_reader);
            let mut requests = Vec::new();
            for _ in 0..2 {
                let mut request = String::new();
                reader.read_line(&mut request).expect("read request");
                requests.push(serde_json::from_str::<Value>(&request).expect("parse request"));
            }
            for request in requests.into_iter().rev() {
                let request_id = request["request_id"].as_u64().expect("request id");
                server_stream
                    .write_all(
                        format!(
                            "{{\"error\":\"success\",\"data\":{request_id},\"request_id\":{request_id}}}\n"
                        )
                        .as_bytes(),
                    )
                    .expect("write response");
            }
        });
        let client = Arc::new(test_client(&client_stream));
        drop(client_stream);
        let first = {
            let client = Arc::clone(&client);
            thread::spawn(move || {
                client.send(
                    &MpvCommand::SetPause { paused: true },
                    Duration::from_secs(1),
                )
            })
        };
        let second = {
            let client = Arc::clone(&client);
            thread::spawn(move || {
                client.send(
                    &MpvCommand::GetVideoOutputConfigured,
                    Duration::from_secs(1),
                )
            })
        };
        let first = first
            .join()
            .expect("join first request")
            .expect("first response");
        let second = second
            .join()
            .expect("join second request")
            .expect("second response");
        assert_ne!(first["request_id"], second["request_id"]);
        server.join().expect("join server");
        Arc::try_unwrap(client)
            .expect("release request owners")
            .shutdown()
            .expect("join IPC threads");
    }

    #[test]
    fn pending_table_rejects_more_than_the_bounded_capacity() {
        let pending = PendingResponses::new(1);
        let (first, _first_response) = mpsc::sync_channel(1);
        let (second, _second_response) = mpsc::sync_channel(1);
        pending.register(1, first).expect("register first request");
        assert!(matches!(
            pending.register(2, second),
            Err(RealtimeVideoBackendError::IpcQueueFull)
        ));
    }

    #[test]
    fn oversized_response_is_rejected_without_unbounded_allocation() {
        let mut input = io::Cursor::new(vec![b'x'; 9]);
        let error = read_bounded_line(&mut input, 8).expect_err("line must be bounded");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn pending_table_matches_out_of_order_responses_and_stays_bounded() {
        let pending = PendingResponses::new(2);
        let (first_sender, first_response) = mpsc::sync_channel(1);
        let (second_sender, second_response) = mpsc::sync_channel(1);
        pending.register(1, first_sender).expect("register first");
        pending.register(2, second_sender).expect("register second");
        let (overflow_sender, _overflow_response) = mpsc::sync_channel(1);
        assert!(matches!(
            pending.register(3, overflow_sender),
            Err(RealtimeVideoBackendError::IpcQueueFull)
        ));

        pending
            .resolve(serde_json::json!({
                "request_id": 2,
                "error": "success",
                "data": "second"
            }))
            .expect("resolve second first");
        pending
            .resolve(serde_json::json!({
                "request_id": 1,
                "error": "success",
                "data": "first"
            }))
            .expect("resolve first second");

        assert_eq!(
            first_response
                .recv()
                .expect("first response")
                .expect("first ok")["data"],
            "first"
        );
        assert_eq!(
            second_response
                .recv()
                .expect("second response")
                .expect("second ok")["data"],
            "second"
        );
        assert!(matches!(
            pending.resolve(serde_json::json!({
                "request_id": 2,
                "error": "success"
            })),
            Err(RealtimeVideoBackendError::IpcProtocol(_))
        ));
    }

    #[test]
    fn stopped_client_rejects_new_commands_without_leaking_pending_waiters() {
        let (client_stream, server_stream) = connected_streams();
        let mut client = test_client(&client_stream);
        drop(client_stream);
        drop(server_stream);
        client.shutdown().expect("join IPC threads");

        let error = client
            .send(
                &MpvCommand::GetVideoOutputConfigured,
                Duration::from_millis(1),
            )
            .expect_err("stopped client must reject commands");
        assert!(matches!(
            error,
            RealtimeVideoBackendError::IpcDisconnected(_)
        ));
        assert!(client
            .pending
            .state
            .lock()
            .expect("pending lock")
            .waiters
            .is_empty());
    }
}
