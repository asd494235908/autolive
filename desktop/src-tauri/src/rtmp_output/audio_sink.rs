use std::fmt::{Display, Formatter};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};

pub(crate) const AUDIO_QUEUE_CAPACITY: usize = 32;
const MAX_AUDIO_SAMPLES_PER_CHUNK: usize = 192_000;

#[derive(Debug)]
struct SinkState {
    sender: Mutex<Option<SyncSender<Vec<f32>>>>,
    closed: AtomicBool,
    overrun: AtomicBool,
    dropped_chunks: std::sync::atomic::AtomicU64,
}

/// 从最终 PCM 总线向 RTMP FFmpeg 进程提供音频的有界、非阻塞写入端。
#[derive(Clone, Debug)]
pub struct RtmpAudioSink {
    state: Arc<SinkState>,
}

impl RtmpAudioSink {
    pub(crate) fn channel() -> (Self, Receiver<Vec<f32>>) {
        let (sender, receiver) = mpsc::sync_channel(AUDIO_QUEUE_CAPACITY);
        (
            Self {
                state: Arc::new(SinkState {
                    sender: Mutex::new(Some(sender)),
                    closed: AtomicBool::new(false),
                    overrun: AtomicBool::new(false),
                    dropped_chunks: std::sync::atomic::AtomicU64::new(0),
                }),
            },
            receiver,
        )
    }

    /// 尝试写入一段交错双声道 f32 PCM。该方法绝不等待消费者。
    pub fn try_push(&self, samples: &[f32]) -> Result<(), RtmpAudioSinkError> {
        if samples.is_empty() {
            return Ok(());
        }
        if samples.len() > MAX_AUDIO_SAMPLES_PER_CHUNK || !samples.len().is_multiple_of(2) {
            return Err(RtmpAudioSinkError::InvalidChunk);
        }
        if self.state.closed.load(Ordering::Acquire) {
            return Err(RtmpAudioSinkError::Closed);
        }
        let samples = samples.to_vec();
        let sender = self
            .state
            .sender
            .lock()
            .map_err(|_| RtmpAudioSinkError::Closed)?;
        // 发送端锁把关闭和 try_send 线性化：close() 先设置 closed 后，
        // 不会再从 Option 中取出旧 Sender 并向已结束的会话写入 PCM。
        if self.state.closed.load(Ordering::Acquire) {
            return Err(RtmpAudioSinkError::Closed);
        }
        let sender = sender.as_ref().ok_or(RtmpAudioSinkError::Closed)?;
        match sender.try_send(samples) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.state.dropped_chunks.fetch_add(1, Ordering::Relaxed);
                self.state.overrun.store(true, Ordering::Release);
                Err(RtmpAudioSinkError::QueueFull)
            }
            Err(TrySendError::Disconnected(_)) => Err(RtmpAudioSinkError::Closed),
        }
    }

    /// 关闭输入端；关闭后不会再接受新的 PCM 数据。
    pub fn close(&self) {
        self.state.closed.store(true, Ordering::Release);
        if let Ok(mut sender) = self.state.sender.lock() {
            sender.take();
        }
    }

    pub fn is_closed(&self) -> bool {
        self.state.closed.load(Ordering::Acquire)
    }

    pub fn dropped_chunks(&self) -> u64 {
        self.state.dropped_chunks.load(Ordering::Relaxed)
    }

    /// 读取并清除本次 FFmpeg 尝试期间的过载标记。
    pub(crate) fn take_overrun(&self) -> bool {
        self.state.overrun.swap(false, Ordering::AcqRel)
    }

    pub(crate) fn has_overrun(&self) -> bool {
        self.state.overrun.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtmpAudioSinkError {
    Closed,
    QueueFull,
    InvalidChunk,
}

impl Display for RtmpAudioSinkError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Closed => "RTMP 音频输入已关闭",
            Self::QueueFull => "RTMP 音频输入队列已满",
            Self::InvalidChunk => "RTMP 音频 PCM 分片无效",
        })
    }
}

impl std::error::Error for RtmpAudioSinkError {}

pub(crate) fn pcm_f32le_bytes(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(samples));
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::TryRecvError;

    #[test]
    fn push_is_bounded_and_non_blocking() {
        let (sink, receiver) = RtmpAudioSink::channel();
        for _ in 0..AUDIO_QUEUE_CAPACITY {
            sink.try_push(&[0.0, 0.0])
                .expect("queue should accept capacity");
        }
        assert_eq!(
            sink.try_push(&[0.0, 0.0]),
            Err(RtmpAudioSinkError::QueueFull)
        );
        assert_eq!(sink.dropped_chunks(), 1);
        assert_eq!(receiver.try_recv().expect("queued PCM"), vec![0.0, 0.0]);
    }

    #[test]
    fn close_disconnects_input() {
        let (sink, receiver) = RtmpAudioSink::channel();
        sink.close();
        assert!(sink.is_closed());
        assert_eq!(sink.try_push(&[0.0, 0.0]), Err(RtmpAudioSinkError::Closed));
        assert_eq!(receiver.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn pcm_conversion_is_little_endian() {
        let bytes = pcm_f32le_bytes(&[1.0, -2.0]);
        assert_eq!(
            bytes,
            [1.0f32.to_le_bytes(), (-2.0f32).to_le_bytes()].concat()
        );
    }

    #[test]
    fn queue_full_sets_a_consumable_overrun_signal() {
        let (sink, _receiver) = RtmpAudioSink::channel();
        for _ in 0..AUDIO_QUEUE_CAPACITY {
            sink.try_push(&[0.0, 0.0])
                .expect("queue should accept capacity");
        }
        assert_eq!(
            sink.try_push(&[0.0, 0.0]),
            Err(RtmpAudioSinkError::QueueFull)
        );
        assert!(sink.take_overrun());
        assert!(!sink.take_overrun());
        assert!(!sink.has_overrun());
    }
}
