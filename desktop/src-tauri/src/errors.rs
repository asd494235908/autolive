use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::PlaybackState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackError {
    EmptyWindowId,
    SourceMediaRequired,
    InvalidMediaProcessingOutput,
    StaleMediaProcessing,
    InvalidTransition {
        from: PlaybackState,
        to: PlaybackState,
    },
    WindowAlreadyBound {
        existing: String,
        attempted: String,
    },
}

impl Display for PlaybackError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyWindowId => f.write_str("播放窗口标识不能为空"),
            Self::SourceMediaRequired => f.write_str("请先导入一个源视频"),
            Self::InvalidMediaProcessingOutput => f.write_str("媒体处理输出无效"),
            Self::StaleMediaProcessing => f.write_str("媒体处理结果已过期"),
            Self::WindowAlreadyBound {
                existing,
                attempted,
            } => {
                write!(
                    f,
                    "播放核心已绑定单一窗口，existing={existing}, attempted={attempted}"
                )
            }
            Self::InvalidTransition { from, to } => {
                write!(f, "非法播放状态切换: {from:?} -> {to:?}")
            }
        }
    }
}

impl Error for PlaybackError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaLibraryError {
    Cancelled,
    EmptyPath,
    CanonicalizeFailed {
        path: String,
    },
    NotAFile {
        path: String,
    },
    UnsupportedExtension {
        path: String,
        extension: Option<String>,
    },
    FileNameUnavailable {
        path: String,
    },
    FileMetadataReadFailed {
        path: String,
    },
    FileOpenFailed {
        path: String,
    },
    UnreadableMp4Container {
        path: String,
        message: String,
    },
    NumericOverflow {
        field: &'static str,
    },
}

impl Display for MediaLibraryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("操作已取消"),
            Self::EmptyPath => f.write_str("文件路径不能为空"),
            Self::CanonicalizeFailed { path } => {
                write!(f, "文件路径规范化失败: {path}")
            }
            Self::NotAFile { path } => write!(f, "路径不是普通文件: {path}"),
            Self::UnsupportedExtension { path, extension } => {
                write!(f, "当前仅支持 MP4 文件: {path}")?;
                if let Some(extension) = extension {
                    write!(f, " (extension={extension})")?;
                }
                Ok(())
            }
            Self::FileNameUnavailable { path } => {
                write!(f, "无法从路径读取文件名: {path}")
            }
            Self::FileMetadataReadFailed { path } => {
                write!(f, "读取文件元数据失败: {path}")
            }
            Self::FileOpenFailed { path } => write!(f, "打开文件失败: {path}"),
            Self::UnreadableMp4Container { path, message } => {
                write!(f, "MP4 容器不可读: {path}; {message}")
            }
            Self::NumericOverflow { field } => {
                write!(f, "数值超出当前实现可表达范围: {field}")
            }
        }
    }
}

impl Error for MediaLibraryError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileHashError {
    Cancelled,
    EmptyPath,
    CanonicalizeFailed { path: String },
    NotAFile { path: String },
    UnsupportedExtension { path: String },
    FileOpenFailed { path: String },
    FileReadFailed { path: String },
}

impl Display for FileHashError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("操作已取消"),
            Self::EmptyPath => f.write_str("文件路径不能为空"),
            Self::CanonicalizeFailed { path } => {
                write!(f, "文件路径规范化失败: {path}")
            }
            Self::NotAFile { path } => write!(f, "路径不是普通文件: {path}"),
            Self::UnsupportedExtension { path } => {
                write!(f, "MP4 哈希命令仅支持 .mp4 文件: {path}")
            }
            Self::FileOpenFailed { path } => write!(f, "打开文件失败: {path}"),
            Self::FileReadFailed { path } => write!(f, "读取文件失败: {path}"),
        }
    }
}

impl Error for FileHashError {}
