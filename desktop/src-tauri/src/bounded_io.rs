use std::fmt;
use std::io::{self, Read};

#[derive(Debug)]
pub enum BoundedReadError {
    Io(io::Error),
    LimitExceeded { limit: usize },
}

impl fmt::Display for BoundedReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "读取失败：{error}"),
            Self::LimitExceeded { limit } => write!(formatter, "读取内容超过 {limit} 字节上限"),
        }
    }
}

impl std::error::Error for BoundedReadError {}

pub fn read_to_end_bounded(
    mut reader: impl Read,
    limit: usize,
) -> Result<Vec<u8>, BoundedReadError> {
    let mut output = Vec::with_capacity(limit.min(8 * 1024));
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => return Ok(output),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(BoundedReadError::Io(error)),
        };
        let remaining = limit.saturating_sub(output.len());
        if read > remaining {
            return Err(BoundedReadError::LimitExceeded { limit });
        }
        output.extend_from_slice(&buffer[..read]);
    }
}

#[cfg(test)]
mod tests {
    use super::{read_to_end_bounded, BoundedReadError};
    use std::io::Cursor;

    #[test]
    fn accepts_input_exactly_at_the_limit() {
        let bytes = read_to_end_bounded(Cursor::new(vec![7_u8; 8]), 8)
            .expect("input at limit should be accepted");
        assert_eq!(bytes, vec![7_u8; 8]);
    }

    #[test]
    fn rejects_limit_plus_one_byte() {
        let result = read_to_end_bounded(Cursor::new(vec![7_u8; 9]), 8);
        assert!(matches!(
            result,
            Err(BoundedReadError::LimitExceeded { limit: 8 })
        ));
    }
}
