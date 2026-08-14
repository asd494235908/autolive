use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

const MIN_INTERVAL_MS: u64 = 500;
const MAX_INTERVAL_MS: u64 = 60_000;
const MIN_VOLUME_DB: f64 = -60.0;
const MAX_VOLUME_DB: f64 = 12.0;
const MIN_DUCKING_DEPTH_DB: f64 = -60.0;
const MAX_DUCKING_DEPTH_DB: f64 = 0.0;
const MIN_DUCKING_ATTACK_MS: u64 = 5;
const MAX_DUCKING_ATTACK_MS: u64 = 1_000;
const MIN_DUCKING_RELEASE_MS: u64 = 10;
const MAX_DUCKING_RELEASE_MS: u64 = 3_000;
const ALLOWED_EXTENSIONS: [&str; 6] = ["mp3", "wav", "m4a", "aac", "ogg", "flac"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterludeConfig {
    pub enabled: bool,
    pub directory: Option<String>,
    pub interval_min_ms: u64,
    pub interval_max_ms: u64,
    pub volume_db: f64,
    pub ducking_depth_db: f64,
    pub ducking_attack_ms: u64,
    pub ducking_release_ms: u64,
}

impl Default for InterludeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            directory: None,
            interval_min_ms: 8_000,
            interval_max_ms: 13_000,
            volume_db: 0.0,
            ducking_depth_db: -12.0,
            ducking_attack_ms: 50,
            ducking_release_ms: 250,
        }
    }
}

impl InterludeConfig {
    pub fn validate(&self) -> Result<(), InterludeError> {
        if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&self.interval_min_ms) {
            return Err(InterludeError::IntervalMinOutOfRange);
        }
        if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&self.interval_max_ms) {
            return Err(InterludeError::IntervalMaxOutOfRange);
        }
        if self.interval_min_ms > self.interval_max_ms {
            return Err(InterludeError::IntervalOrderInvalid);
        }
        if !(MIN_VOLUME_DB..=MAX_VOLUME_DB).contains(&self.volume_db) {
            return Err(InterludeError::VolumeOutOfRange);
        }
        if !(MIN_DUCKING_DEPTH_DB..=MAX_DUCKING_DEPTH_DB).contains(&self.ducking_depth_db) {
            return Err(InterludeError::DuckingDepthOutOfRange);
        }
        if !(MIN_DUCKING_ATTACK_MS..=MAX_DUCKING_ATTACK_MS).contains(&self.ducking_attack_ms) {
            return Err(InterludeError::DuckingAttackOutOfRange);
        }
        if !(MIN_DUCKING_RELEASE_MS..=MAX_DUCKING_RELEASE_MS).contains(&self.ducking_release_ms) {
            return Err(InterludeError::DuckingReleaseOutOfRange);
        }
        if self.enabled && normalized_directory_input(self.directory.as_deref()).is_none() {
            return Err(InterludeError::MissingDirectory);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterludeCatalog {
    pub directory: String,
    pub audio_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterludeSnapshot {
    pub enabled: bool,
    pub directory: Option<String>,
    pub audio_files: Vec<String>,
    pub audio_count: u32,
    pub status: String,
    pub error: Option<String>,
    pub interval_min_ms: u64,
    pub interval_max_ms: u64,
    pub volume_db: f64,
    pub ducking_depth_db: f64,
    pub ducking_attack_ms: u64,
    pub ducking_release_ms: u64,
}

impl Default for InterludeSnapshot {
    fn default() -> Self {
        Self::from_config_and_catalog(&InterludeConfig::default(), None)
    }
}

impl InterludeSnapshot {
    pub fn from_config_and_catalog(
        config: &InterludeConfig,
        catalog: Option<&InterludeCatalog>,
    ) -> Self {
        let audio_files = catalog
            .map(|catalog| catalog.audio_files.clone())
            .unwrap_or_default();
        let directory = catalog
            .map(|catalog| Some(catalog.directory.clone()))
            .unwrap_or_else(|| config.directory.clone());
        let status = if config.enabled {
            if audio_files.is_empty() {
                "empty"
            } else {
                "ready"
            }
        } else {
            "disabled"
        };
        Self {
            enabled: config.enabled,
            directory,
            audio_count: audio_files.len().try_into().unwrap_or(u32::MAX),
            audio_files,
            status: status.to_owned(),
            error: None,
            interval_min_ms: config.interval_min_ms,
            interval_max_ms: config.interval_max_ms,
            volume_db: config.volume_db,
            ducking_depth_db: config.ducking_depth_db,
            ducking_attack_ms: config.ducking_attack_ms,
            ducking_release_ms: config.ducking_release_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterludeError {
    MissingDirectory,
    DirectoryUnavailable(String),
    DirectoryReadFailed(String),
    NoUsableAudioFiles,
    IntervalMinOutOfRange,
    IntervalMaxOutOfRange,
    IntervalOrderInvalid,
    VolumeOutOfRange,
    DuckingDepthOutOfRange,
    DuckingAttackOutOfRange,
    DuckingReleaseOutOfRange,
}

impl fmt::Display for InterludeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDirectory => formatter.write_str("启用插话前必须选择一个本地目录"),
            Self::DirectoryUnavailable(message) => {
                write!(formatter, "插话目录不可用：{message}")
            }
            Self::DirectoryReadFailed(message) => {
                write!(formatter, "插话目录读取失败：{message}")
            }
            Self::NoUsableAudioFiles => formatter
                .write_str("插话目录内至少需要一个可播放音频文件（mp3、wav、m4a、aac、ogg、flac）"),
            Self::IntervalMinOutOfRange => formatter.write_str("插话最小间隔必须在 500..=60000ms"),
            Self::IntervalMaxOutOfRange => formatter.write_str("插话最大间隔必须在 500..=60000ms"),
            Self::IntervalOrderInvalid => formatter.write_str("插话最小间隔不能大于最大间隔"),
            Self::VolumeOutOfRange => formatter.write_str("插话音量必须在 -60..=12 dB"),
            Self::DuckingDepthOutOfRange => formatter.write_str("闪避深度必须在 -60..=0 dB"),
            Self::DuckingAttackOutOfRange => formatter.write_str("闪避 Attack 必须在 5..=1000ms"),
            Self::DuckingReleaseOutOfRange => {
                formatter.write_str("闪避 Release 必须在 10..=3000ms")
            }
        }
    }
}

impl std::error::Error for InterludeError {}

pub fn prepare_interlude_snapshot(
    mut config: InterludeConfig,
) -> Result<(InterludeConfig, InterludeSnapshot), InterludeError> {
    config.directory = normalized_directory_input(config.directory.as_deref());
    config.validate()?;

    let catalog = match (config.enabled, config.directory.as_deref()) {
        (true, Some(directory)) => Some(scan_catalog(directory)?),
        (true, None) => None,
        (false, _) => None,
    };
    if config.enabled
        && catalog
            .as_ref()
            .is_some_and(|catalog| catalog.audio_files.is_empty())
    {
        return Err(InterludeError::NoUsableAudioFiles);
    }
    if let Some(catalog) = catalog.as_ref() {
        config.directory = Some(catalog.directory.clone());
    }

    let snapshot = InterludeSnapshot::from_config_and_catalog(&config, catalog.as_ref());
    Ok((config, snapshot))
}

pub fn resolve_effective_audio_source(
    voice_clone_status: &str,
    current_audio_source: Option<&str>,
    current_video_source: Option<&str>,
) -> &'static str {
    if voice_clone_status == "playing" {
        "voice_clone"
    } else if current_audio_source == Some("realtime_variant") {
        "realtime_variant"
    } else if current_video_source == Some("processed") {
        "processed_original"
    } else {
        "original"
    }
}

fn normalized_directory_input(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn scan_catalog(directory: &str) -> Result<InterludeCatalog, InterludeError> {
    let canonical_directory = canonicalize_directory(directory)?;
    let entries = std::fs::read_dir(&canonical_directory)
        .map_err(|error| InterludeError::DirectoryReadFailed(error.to_string()))?;
    let mut audio_files = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let entry_path = entry.path();
        if !file_type.is_file() || !has_allowed_extension(&entry_path) {
            continue;
        }
        let Ok(canonical_file) = std::fs::canonicalize(entry_path) else {
            continue;
        };
        audio_files.push(canonical_file.display().to_string());
    }
    audio_files.sort();
    Ok(InterludeCatalog {
        directory: canonical_directory.display().to_string(),
        audio_files,
    })
}

fn canonicalize_directory(directory: &str) -> Result<PathBuf, InterludeError> {
    let canonical = std::fs::canonicalize(directory)
        .map_err(|error| InterludeError::DirectoryUnavailable(error.to_string()))?;
    if !canonical.is_dir() {
        return Err(InterludeError::DirectoryUnavailable(
            "选择的路径不是目录".to_owned(),
        ));
    }
    Ok(canonical)
}

fn has_allowed_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .is_some_and(|value| ALLOWED_EXTENSIONS.iter().any(|allowed| *allowed == value))
}
