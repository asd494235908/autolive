//! AkVirtualCamera sidecar 的受限本机传输协议。
//!
//! 这里仅定义固定帧格式和受控 Named Pipe 名称；不打开管道、不启动进程，也不
//! 接受任意路径。独立 GPL sidecar 负责按此名称创建当前用户 ACL 管道，桌面端只
//! 通过受控入口传入绝对 sidecar 路径并交付已完成 GPU 回读的帧。

use std::fmt;

pub const PROTOCOL_VERSION: u16 = 1;
pub const FRAME_MAGIC: [u8; 8] = *b"GPAKVC01";
pub const FRAME_HEADER_BYTES: usize = 52;
pub const OUTPUT_WIDTH: u32 = 1280;
pub const OUTPUT_HEIGHT: u32 = 720;
pub const MAX_PAYLOAD_BYTES: usize = (OUTPUT_WIDTH as usize) * (OUTPUT_HEIGHT as usize) * 2;
const PIPE_PREFIX: &str = r"\\.\pipe\GpAutoLive-AkVirtualCamera-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    InvalidFrame(&'static str),
    InvalidPipeToken,
    Truncated,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFrame(message) => write!(formatter, "sidecar 帧无效：{message}"),
            Self::InvalidPipeToken => write!(formatter, "sidecar Named Pipe 会话令牌无效"),
            Self::Truncated => write!(formatter, "sidecar 帧数据不完整"),
        }
    }
}

impl std::error::Error for ProtocolError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarFrame {
    pub generation: u64,
    pub sequence: u64,
    pub timestamp_100ns: i64,
    pub payload: Vec<u8>,
}

impl SidecarFrame {
    pub fn new(
        generation: u64,
        sequence: u64,
        timestamp_100ns: i64,
        payload: Vec<u8>,
    ) -> Result<Self, ProtocolError> {
        let frame = Self {
            generation,
            sequence,
            timestamp_100ns,
            payload,
        };
        frame.validate()?;
        Ok(frame)
    }

    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.generation == 0 {
            return Err(ProtocolError::InvalidFrame("generation 不能为 0"));
        }
        if self.sequence == 0 {
            return Err(ProtocolError::InvalidFrame("sequence 不能为 0"));
        }
        if self.timestamp_100ns < 0 {
            return Err(ProtocolError::InvalidFrame("时间戳不能为负数"));
        }
        if self.payload.len() != MAX_PAYLOAD_BYTES {
            return Err(ProtocolError::InvalidFrame(
                "YUY2 payload 必须固定为 1280×720×2",
            ));
        }
        Ok(())
    }

    pub fn encode(&self, output: &mut Vec<u8>) -> Result<(), ProtocolError> {
        self.validate()?;
        let payload_len = u32::try_from(self.payload.len())
            .map_err(|_| ProtocolError::InvalidFrame("payload 长度溢出"))?;
        output.reserve(FRAME_HEADER_BYTES + self.payload.len());
        output.extend_from_slice(&FRAME_MAGIC);
        output.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        output.extend_from_slice(&(FRAME_HEADER_BYTES as u16).to_le_bytes());
        output.extend_from_slice(&self.generation.to_le_bytes());
        output.extend_from_slice(&self.sequence.to_le_bytes());
        output.extend_from_slice(&self.timestamp_100ns.to_le_bytes());
        output.extend_from_slice(&OUTPUT_WIDTH.to_le_bytes());
        output.extend_from_slice(&OUTPUT_HEIGHT.to_le_bytes());
        output.extend_from_slice(&payload_len.to_le_bytes());
        output.extend_from_slice(&0_u32.to_le_bytes());
        output.extend_from_slice(&self.payload);
        Ok(())
    }

    pub fn decode(input: &[u8]) -> Result<(Self, usize), ProtocolError> {
        if input.len() < FRAME_HEADER_BYTES {
            return Err(ProtocolError::Truncated);
        }
        if input[..8] != FRAME_MAGIC {
            return Err(ProtocolError::InvalidFrame("magic 不匹配"));
        }
        let version = u16::from_le_bytes([input[8], input[9]]);
        if version != PROTOCOL_VERSION {
            return Err(ProtocolError::InvalidFrame("协议版本不支持"));
        }
        let header_len = u16::from_le_bytes([input[10], input[11]]) as usize;
        if header_len != FRAME_HEADER_BYTES {
            return Err(ProtocolError::InvalidFrame("header 长度不匹配"));
        }
        let generation = read_u64(input, 12);
        let sequence = read_u64(input, 20);
        let timestamp_100ns = read_i64(input, 28);
        let width = read_u32(input, 36);
        let height = read_u32(input, 40);
        let payload_len = read_u32(input, 44) as usize;
        if read_u32(input, 48) != 0 {
            return Err(ProtocolError::InvalidFrame("保留字段必须为 0"));
        }
        if width != OUTPUT_WIDTH || height != OUTPUT_HEIGHT {
            return Err(ProtocolError::InvalidFrame("输出规格不是固定 1280×720"));
        }
        if payload_len != MAX_PAYLOAD_BYTES {
            return Err(ProtocolError::InvalidFrame(
                "payload 长度不是固定 YUY2 大小",
            ));
        }
        let total = FRAME_HEADER_BYTES
            .checked_add(payload_len)
            .ok_or(ProtocolError::InvalidFrame("帧长度溢出"))?;
        if input.len() < total {
            return Err(ProtocolError::Truncated);
        }
        let frame = Self {
            generation,
            sequence,
            timestamp_100ns,
            payload: input[FRAME_HEADER_BYTES..total].to_vec(),
        };
        frame.validate()?;
        Ok((frame, total))
    }
}

fn read_u32(input: &[u8], offset: usize) -> u32 {
    let mut bytes = [0_u8; 4];
    bytes.copy_from_slice(&input[offset..offset + 4]);
    u32::from_le_bytes(bytes)
}

fn read_u64(input: &[u8], offset: usize) -> u64 {
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&input[offset..offset + 8]);
    u64::from_le_bytes(bytes)
}

fn read_i64(input: &[u8], offset: usize) -> i64 {
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&input[offset..offset + 8]);
    i64::from_le_bytes(bytes)
}

/// 仅根据 16 字节随机会话令牌生成当前用户专用管道名。
///
/// ACL、创建、连接和关闭由独立 sidecar 负责；此函数不接受路径或任意命名管道
/// 名称，避免把任意 IPC 端点暴露给前端。
pub fn pipe_name(session_token: [u8; 16]) -> String {
    let mut suffix = String::with_capacity(32);
    for byte in session_token {
        use std::fmt::Write;
        let _ = write!(suffix, "{byte:02x}");
    }
    format!("{PIPE_PREFIX}{suffix}")
}

pub fn validate_pipe_name(name: &str) -> Result<(), ProtocolError> {
    let Some(suffix) = name.strip_prefix(PIPE_PREFIX) else {
        return Err(ProtocolError::InvalidPipeToken);
    };
    if suffix.len() != 32
        || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        || suffix.bytes().all(|byte| byte == b'0')
    {
        return Err(ProtocolError::InvalidPipeToken);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> SidecarFrame {
        SidecarFrame::new(3, 7, 1234, vec![16; MAX_PAYLOAD_BYTES]).expect("fixture")
    }

    #[test]
    fn frame_round_trip_is_fixed_and_stream_framed() {
        let frame = fixture();
        let mut encoded = Vec::new();
        frame.encode(&mut encoded).expect("encode");
        encoded.extend_from_slice(b"next");
        let (decoded, consumed) = SidecarFrame::decode(&encoded).expect("decode");
        assert_eq!(decoded, frame);
        assert_eq!(consumed, FRAME_HEADER_BYTES + MAX_PAYLOAD_BYTES);
        assert_eq!(&encoded[consumed..], b"next");
    }

    #[test]
    fn malformed_frame_is_rejected_without_unbounded_allocation() {
        let mut encoded = Vec::new();
        fixture().encode(&mut encoded).expect("encode");
        encoded[36..40].copy_from_slice(&1920_u32.to_le_bytes());
        assert!(matches!(
            SidecarFrame::decode(&encoded),
            Err(ProtocolError::InvalidFrame("输出规格不是固定 1280×720"))
        ));
    }

    #[test]
    fn pipe_name_is_derived_from_fixed_token_only() {
        let name = pipe_name([0xab; 16]);
        assert_eq!(
            name,
            r"\\.\pipe\GpAutoLive-AkVirtualCamera-abababababababababababababababab"
        );
        assert!(validate_pipe_name(&name).is_ok());
        assert!(validate_pipe_name(r"\\.\pipe\other").is_err());
        assert!(validate_pipe_name(
            r"\\.\pipe\GpAutoLive-AkVirtualCamera-00000000000000000000000000000000"
        )
        .is_err());
    }
}
