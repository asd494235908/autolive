use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::net::Ipv6Addr;

const MAX_TARGET_URL_LEN: usize = 2_048;
// GPU83 的静态 custom_shader_bin 会展开为十几 KB 十六进制参数，仍保持有界。
const MAX_FILTER_LEN: usize = 128 * 1024;
const ALLOWED_FPS: [u32; 4] = [25, 30, 50, 60];

/// RTMP/RTMPS 发布配置。
///
/// 该配置只描述发布参数；输入媒体和 FFmpeg 路径由上层在启动时提供。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RtmpOutputConfig {
    pub target_url: String,
    pub video_enabled: bool,
    pub audio_enabled: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub video_bitrate_kbps: u32,
    pub audio_bitrate_kbps: u32,
}

impl Default for RtmpOutputConfig {
    fn default() -> Self {
        Self {
            target_url: String::new(),
            video_enabled: true,
            audio_enabled: true,
            width: 1_280,
            height: 720,
            fps: 30,
            video_bitrate_kbps: 2_500,
            audio_bitrate_kbps: 128,
        }
    }
}

impl RtmpOutputConfig {
    pub fn validate(&self) -> Result<(), RtmpConfigError> {
        let target_url = self.target_url.trim();
        if target_url.is_empty() {
            return Err(RtmpConfigError::TargetUrlRequired);
        }
        if target_url.len() > MAX_TARGET_URL_LEN {
            return Err(RtmpConfigError::TargetUrlTooLong);
        }
        if target_url != self.target_url {
            return Err(RtmpConfigError::TargetUrlWhitespace);
        }
        if target_url.chars().any(|character| {
            character.is_control() || character.is_whitespace() || character == '\\'
        }) {
            return Err(RtmpConfigError::TargetUrlInvalid);
        }

        let (scheme, remainder) = target_url
            .split_once("://")
            .ok_or(RtmpConfigError::TargetUrlInvalid)?;
        if scheme != "rtmp" && scheme != "rtmps" {
            return Err(RtmpConfigError::UnsupportedScheme);
        }

        let authority_end = remainder
            .find('/')
            .ok_or(RtmpConfigError::TargetUrlPathRequired)?;
        let authority = &remainder[..authority_end];
        let path = &remainder[authority_end + 1..];
        let path_without_query = path.split_once('?').map_or(path, |(path, _)| path);
        if authority.is_empty()
            || path_without_query.is_empty()
            || path_without_query.ends_with('/')
        {
            return Err(RtmpConfigError::TargetUrlInvalid);
        }
        if authority.contains('@') || authority.contains('?') || authority.contains('#') {
            return Err(RtmpConfigError::TargetUrlInvalid);
        }
        if path.contains('#') || path.contains('\n') || path.contains('\r') {
            return Err(RtmpConfigError::TargetUrlInvalid);
        }

        validate_authority(authority)?;

        if !self.video_enabled && !self.audio_enabled {
            return Err(RtmpConfigError::TrackRequired);
        }
        if self.video_enabled
            && (self.width < 16
                || self.width > 7_680
                || self.height < 16
                || self.height > 4_320
                || !self.width.is_multiple_of(2)
                || !self.height.is_multiple_of(2))
        {
            return Err(RtmpConfigError::VideoSizeOutOfRange);
        }
        if self.video_enabled && !ALLOWED_FPS.contains(&self.fps) {
            return Err(RtmpConfigError::FpsOutOfRange);
        }
        if self.video_enabled && !(64..=100_000).contains(&self.video_bitrate_kbps) {
            return Err(RtmpConfigError::VideoBitrateOutOfRange);
        }
        if self.audio_enabled && !(16..=512).contains(&self.audio_bitrate_kbps) {
            return Err(RtmpConfigError::AudioBitrateOutOfRange);
        }
        Ok(())
    }
}

fn validate_authority(authority: &str) -> Result<(), RtmpConfigError> {
    if authority.starts_with('[') {
        let closing = authority
            .find(']')
            .ok_or(RtmpConfigError::TargetUrlHostInvalid)?;
        if closing == 1 {
            return Err(RtmpConfigError::TargetUrlHostInvalid);
        }
        authority[1..closing]
            .parse::<Ipv6Addr>()
            .map_err(|_| RtmpConfigError::TargetUrlHostInvalid)?;
        let suffix = &authority[closing + 1..];
        if !suffix.is_empty() && !suffix.starts_with(':') {
            return Err(RtmpConfigError::TargetUrlHostInvalid);
        }
        if let Some(port) = suffix.strip_prefix(':') {
            validate_port(port)?;
        }
        return Ok(());
    }

    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port))
            if !port.is_empty() && port.chars().all(|character| character.is_ascii_digit()) =>
        {
            (host, Some(port))
        }
        Some((_, "")) => return Err(RtmpConfigError::TargetUrlHostInvalid),
        _ => (authority, None),
    };
    if host.is_empty()
        || host.starts_with('.')
        || host.ends_with('.')
        || host.contains(':')
        || host.contains('@')
        || !host.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
    {
        return Err(RtmpConfigError::TargetUrlHostInvalid);
    }
    if let Some(port) = port {
        validate_port(port)?;
    }
    Ok(())
}

fn validate_port(port: &str) -> Result<(), RtmpConfigError> {
    let value = port
        .parse::<u32>()
        .map_err(|_| RtmpConfigError::TargetUrlPortInvalid)?;
    if !(1..=65_535).contains(&value) {
        return Err(RtmpConfigError::TargetUrlPortInvalid);
    }
    Ok(())
}

pub(crate) fn validate_filter(filter: Option<&str>) -> Result<(), RtmpConfigError> {
    let Some(filter) = filter else {
        return Ok(());
    };
    if filter.is_empty() || filter.len() > MAX_FILTER_LEN || filter.chars().any(char::is_control) {
        return Err(RtmpConfigError::FilterInvalid);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtmpConfigError {
    TargetUrlRequired,
    TargetUrlTooLong,
    TargetUrlWhitespace,
    TargetUrlInvalid,
    UnsupportedScheme,
    TargetUrlPathRequired,
    TargetUrlHostInvalid,
    TargetUrlPortInvalid,
    TrackRequired,
    VideoSizeOutOfRange,
    FpsOutOfRange,
    VideoBitrateOutOfRange,
    AudioBitrateOutOfRange,
    FilterInvalid,
}

impl Display for RtmpConfigError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::TargetUrlRequired => "RTMP 地址不能为空",
            Self::TargetUrlTooLong => "RTMP 地址过长",
            Self::TargetUrlWhitespace => "RTMP 地址首尾不能包含空格",
            Self::TargetUrlInvalid => "RTMP 地址格式无效",
            Self::UnsupportedScheme => "仅支持 rtmp:// 或 rtmps://",
            Self::TargetUrlPathRequired => "RTMP 地址必须包含发布路径",
            Self::TargetUrlHostInvalid => "RTMP 地址主机无效",
            Self::TargetUrlPortInvalid => "RTMP 地址端口无效",
            Self::TrackRequired => "至少选择画面或声音一项",
            Self::VideoSizeOutOfRange => "视频尺寸超出允许范围",
            Self::FpsOutOfRange => "帧率仅支持 25、30、50 或 60 FPS",
            Self::VideoBitrateOutOfRange => "视频码率超出允许范围",
            Self::AudioBitrateOutOfRange => "声音码率超出允许范围",
            Self::FilterInvalid => "视频滤镜参数无效",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for RtmpConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> RtmpOutputConfig {
        RtmpOutputConfig {
            target_url: "rtmp://127.0.0.1:1935/live/test".to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn accepts_rtmp_and_rtmps() {
        assert!(valid_config().validate().is_ok());
        let mut config = valid_config();
        config.target_url = "rtmps://media.example.com/live/stream".to_owned();
        assert!(config.validate().is_ok());
        config.target_url = "rtmp://media.example.com/live/stream?token=opaque".to_owned();
        assert!(config.validate().is_ok());
        config.target_url = "rtmp://[::1]:1935/live/stream".to_owned();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn rejects_credentials_and_invalid_port() {
        let mut config = valid_config();
        config.target_url = "rtmp://user:password@127.0.0.1/live/test".to_owned();
        assert_eq!(config.validate(), Err(RtmpConfigError::TargetUrlInvalid));
        config.target_url = "rtmp://127.0.0.1:70000/live/test".to_owned();
        assert_eq!(
            config.validate(),
            Err(RtmpConfigError::TargetUrlPortInvalid)
        );
        config.target_url = "rtmp://[not-an-ip]:1935/live/test".to_owned();
        assert_eq!(
            config.validate(),
            Err(RtmpConfigError::TargetUrlHostInvalid)
        );
        config.target_url = "rtmp://127.0.0.1/?token=opaque".to_owned();
        assert_eq!(config.validate(), Err(RtmpConfigError::TargetUrlInvalid));
    }

    #[test]
    fn requires_one_track_and_valid_video_values() {
        let mut config = valid_config();
        config.video_enabled = false;
        config.audio_enabled = false;
        assert_eq!(config.validate(), Err(RtmpConfigError::TrackRequired));
        config.audio_enabled = true;
        config.video_enabled = true;
        config.width = 8_000;
        assert_eq!(config.validate(), Err(RtmpConfigError::VideoSizeOutOfRange));
        config.width = 1_281;
        assert_eq!(config.validate(), Err(RtmpConfigError::VideoSizeOutOfRange));
    }

    #[test]
    fn allows_audio_only_without_video_values() {
        let mut config = valid_config();
        config.video_enabled = false;
        config.fps = 0;
        config.width = 0;
        config.height = 0;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn accepts_only_the_fixed_video_frame_rate_whitelist() {
        let mut config = valid_config();
        config.fps = 29;
        assert_eq!(config.validate(), Err(RtmpConfigError::FpsOutOfRange));
        config.fps = 60;
        assert!(config.validate().is_ok());
    }
}
