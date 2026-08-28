use crate::media_library::SUPPORTED_SOURCE_MEDIA_EXTENSIONS;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

const MIN_INTERVAL_MS: u64 = 500;
const MAX_INTERVAL_MS: u64 = 60_000;
const MIN_VOLUME_DB: f64 = -60.0;
const MAX_VOLUME_DB: f64 = 12.0;
const MIN_DUCKING_DEPTH_DB: f64 = -60.0;
const MAX_DUCKING_DEPTH_DB: f64 = 0.0;
const MIN_DUCKING_ATTACK_MS: u64 = 0;
const MAX_DUCKING_ATTACK_MS: u64 = 1_000;
const MIN_DUCKING_RELEASE_MS: u64 = 0;
const MAX_DUCKING_RELEASE_MS: u64 = 3_000;
const MIN_AUDIO_VARIATION_PERIOD_MS: u64 = 1_000;
const MAX_AUDIO_VARIATION_PERIOD_MS: u64 = 600_000;
const MAX_AUDIO_PRESET_IDS: usize = 22;
const MIN_AUDIO_MIX_PICK: u8 = 1;
const MAX_AUDIO_MIX_PICK: u8 = 4;
pub const MAX_INTERLUDE_AUDIO_FILES: usize = 1_000;
const MAX_INTERLUDE_PATH_UTF8_BYTES: usize = 4_096;
const MAX_INTERLUDE_CATALOG_PATH_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterludeAudioSelectionMode {
    Fixed,
    #[default]
    Random,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterludeAudioVariationMode {
    #[default]
    EachPlayback,
    Periodic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterludeConfig {
    pub enabled: bool,
    pub directory: Option<String>,
    #[serde(default)]
    pub audio_selection_mode: InterludeAudioSelectionMode,
    #[serde(default = "default_audio_fixed_preset_id")]
    pub audio_fixed_preset_id: String,
    #[serde(default = "default_audio_preset_ids")]
    pub audio_preset_ids: Vec<String>,
    #[serde(default)]
    pub audio_mix_enabled: bool,
    #[serde(default = "default_audio_mix_pick_min")]
    pub audio_mix_pick_min: u8,
    #[serde(default = "default_audio_mix_pick_max")]
    pub audio_mix_pick_max: u8,
    #[serde(default)]
    pub audio_variation_mode: InterludeAudioVariationMode,
    #[serde(default = "default_audio_variation_period_min_ms")]
    pub audio_variation_period_min_ms: u64,
    #[serde(default = "default_audio_variation_period_max_ms")]
    pub audio_variation_period_max_ms: u64,
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
            audio_selection_mode: InterludeAudioSelectionMode::default(),
            audio_fixed_preset_id: default_audio_fixed_preset_id(),
            audio_preset_ids: default_audio_preset_ids(),
            audio_mix_enabled: false,
            audio_mix_pick_min: default_audio_mix_pick_min(),
            audio_mix_pick_max: default_audio_mix_pick_max(),
            audio_variation_mode: InterludeAudioVariationMode::default(),
            audio_variation_period_min_ms: default_audio_variation_period_min_ms(),
            audio_variation_period_max_ms: default_audio_variation_period_max_ms(),
            interval_min_ms: 8_000,
            interval_max_ms: 13_000,
            volume_db: 0.0,
            ducking_depth_db: -60.0,
            ducking_attack_ms: 50,
            ducking_release_ms: 250,
        }
    }
}

impl InterludeConfig {
    pub fn validate(&self) -> Result<(), InterludeError> {
        if !is_allowed_audio_preset_id(&self.audio_fixed_preset_id) {
            return Err(InterludeError::AudioPresetInvalid);
        }
        if !(1..=MAX_AUDIO_PRESET_IDS).contains(&self.audio_preset_ids.len()) {
            return Err(InterludeError::AudioPresetCountOutOfRange);
        }
        for (index, preset_id) in self.audio_preset_ids.iter().enumerate() {
            if !is_allowed_audio_preset_id(preset_id) {
                return Err(InterludeError::AudioPresetInvalid);
            }
            if self.audio_preset_ids[..index].contains(preset_id) {
                return Err(InterludeError::AudioPresetDuplicate);
            }
        }
        if !(MIN_AUDIO_MIX_PICK..=MAX_AUDIO_MIX_PICK).contains(&self.audio_mix_pick_min) {
            return Err(InterludeError::AudioMixPickMinOutOfRange);
        }
        if !(MIN_AUDIO_MIX_PICK..=MAX_AUDIO_MIX_PICK).contains(&self.audio_mix_pick_max) {
            return Err(InterludeError::AudioMixPickMaxOutOfRange);
        }
        if self.audio_mix_pick_min > self.audio_mix_pick_max {
            return Err(InterludeError::AudioMixPickOrderInvalid);
        }
        if !(MIN_AUDIO_VARIATION_PERIOD_MS..=MAX_AUDIO_VARIATION_PERIOD_MS)
            .contains(&self.audio_variation_period_min_ms)
        {
            return Err(InterludeError::AudioVariationPeriodMinOutOfRange);
        }
        if !(MIN_AUDIO_VARIATION_PERIOD_MS..=MAX_AUDIO_VARIATION_PERIOD_MS)
            .contains(&self.audio_variation_period_max_ms)
        {
            return Err(InterludeError::AudioVariationPeriodMaxOutOfRange);
        }
        if self.audio_variation_period_min_ms > self.audio_variation_period_max_ms {
            return Err(InterludeError::AudioVariationPeriodOrderInvalid);
        }
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
    #[serde(default)]
    pub audio_selection_mode: InterludeAudioSelectionMode,
    #[serde(default = "default_audio_fixed_preset_id")]
    pub audio_fixed_preset_id: String,
    #[serde(default = "default_audio_preset_ids")]
    pub audio_preset_ids: Vec<String>,
    #[serde(default)]
    pub audio_mix_enabled: bool,
    #[serde(default = "default_audio_mix_pick_min")]
    pub audio_mix_pick_min: u8,
    #[serde(default = "default_audio_mix_pick_max")]
    pub audio_mix_pick_max: u8,
    #[serde(default)]
    pub audio_variation_mode: InterludeAudioVariationMode,
    #[serde(default = "default_audio_variation_period_min_ms")]
    pub audio_variation_period_min_ms: u64,
    #[serde(default = "default_audio_variation_period_max_ms")]
    pub audio_variation_period_max_ms: u64,
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
            audio_selection_mode: config.audio_selection_mode,
            audio_fixed_preset_id: config.audio_fixed_preset_id.clone(),
            audio_preset_ids: config.audio_preset_ids.clone(),
            audio_mix_enabled: config.audio_mix_enabled,
            audio_mix_pick_min: config.audio_mix_pick_min,
            audio_mix_pick_max: config.audio_mix_pick_max,
            audio_variation_mode: config.audio_variation_mode,
            audio_variation_period_min_ms: config.audio_variation_period_min_ms,
            audio_variation_period_max_ms: config.audio_variation_period_max_ms,
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
    AudioPresetCountOutOfRange,
    AudioPresetInvalid,
    AudioPresetDuplicate,
    AudioMixPickMinOutOfRange,
    AudioMixPickMaxOutOfRange,
    AudioMixPickOrderInvalid,
    AudioVariationPeriodMinOutOfRange,
    AudioVariationPeriodMaxOutOfRange,
    AudioVariationPeriodOrderInvalid,
    MissingDirectory,
    DirectoryUnavailable(String),
    DirectoryReadFailed(String),
    NoUsableAudioFiles,
    TooManyAudioFiles { count: usize, max_files: usize },
    PathTooLong { bytes: usize, max_bytes: usize },
    CatalogPathBytesExceeded { bytes: usize, max_bytes: usize },
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
            Self::AudioPresetCountOutOfRange => {
                formatter.write_str("插话声音预设池必须包含 1..=22 套预设")
            }
            Self::AudioPresetInvalid => formatter.write_str("插话声音预设必须是 p01..=p22"),
            Self::AudioPresetDuplicate => formatter.write_str("插话声音预设池不能包含重复项"),
            Self::AudioMixPickMinOutOfRange => formatter.write_str("插话最少随机轨数必须在 1..=4"),
            Self::AudioMixPickMaxOutOfRange => formatter.write_str("插话最多随机轨数必须在 1..=4"),
            Self::AudioMixPickOrderInvalid => {
                formatter.write_str("插话最少随机轨数不能大于最多随机轨数")
            }
            Self::AudioVariationPeriodMinOutOfRange => {
                formatter.write_str("插话声音变化最小周期必须在 1000..=60000ms")
            }
            Self::AudioVariationPeriodMaxOutOfRange => {
                formatter.write_str("插话声音变化最大周期必须在 1000..=60000ms")
            }
            Self::AudioVariationPeriodOrderInvalid => {
                formatter.write_str("插话声音变化最小周期不能大于最大周期")
            }
            Self::MissingDirectory => formatter.write_str("启用插话前必须选择一个本地目录"),
            Self::DirectoryUnavailable(message) => {
                write!(formatter, "插话目录不可用：{message}")
            }
            Self::DirectoryReadFailed(message) => {
                write!(formatter, "插话目录读取失败：{message}")
            }
            Self::NoUsableAudioFiles => formatter
                .write_str("插话目录内至少需要一个受支持的音频或视频文件（视频仅使用音轨）"),
            Self::TooManyAudioFiles { count, max_files } => {
                write!(
                    formatter,
                    "插话目录内有 {count} 个受支持的媒体文件，最多允许 {max_files} 个"
                )
            }
            Self::PathTooLong { bytes, max_bytes } => {
                write!(
                    formatter,
                    "插话规范化路径长度为 {bytes} 字节，最多允许 {max_bytes} 字节"
                )
            }
            Self::CatalogPathBytesExceeded { bytes, max_bytes } => write!(
                formatter,
                "插话目录及音频路径累计长度为 {bytes} 字节，最多允许 {max_bytes} 字节"
            ),
            Self::IntervalMinOutOfRange => formatter.write_str("插话最小间隔必须在 500..=60000ms"),
            Self::IntervalMaxOutOfRange => formatter.write_str("插话最大间隔必须在 500..=60000ms"),
            Self::IntervalOrderInvalid => formatter.write_str("插话最小间隔不能大于最大间隔"),
            Self::VolumeOutOfRange => formatter.write_str("插话音量必须在 -60..=12 dB"),
            Self::DuckingDepthOutOfRange => formatter.write_str("闪避深度必须在 -60..=0 dB"),
            Self::DuckingAttackOutOfRange => formatter.write_str("闪避 Attack 必须在 0..=1000ms"),
            Self::DuckingReleaseOutOfRange => formatter.write_str("闪避 Release 必须在 0..=3000ms"),
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
    current_audio_source: Option<&str>,
    current_video_source: Option<&str>,
) -> &'static str {
    if current_audio_source == Some("realtime_variant") {
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

fn default_audio_preset_ids() -> Vec<String> {
    (1..=20).map(|number| format!("p{number:02}")).collect()
}

fn default_audio_fixed_preset_id() -> String {
    "p01".to_owned()
}

fn default_audio_mix_pick_min() -> u8 {
    1
}

fn default_audio_mix_pick_max() -> u8 {
    2
}

fn default_audio_variation_period_min_ms() -> u64 {
    8_000
}

fn default_audio_variation_period_max_ms() -> u64 {
    15_000
}

fn is_allowed_audio_preset_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 3
        && bytes[0] == b'p'
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && (1..=22).contains(&((bytes[1] - b'0') * 10 + bytes[2] - b'0'))
}

fn scan_catalog(directory: &str) -> Result<InterludeCatalog, InterludeError> {
    let canonical_directory = canonicalize_directory(directory)?;
    let directory = canonical_directory.display().to_string();
    let mut catalog_path_bytes = 0;
    add_catalog_path_bytes(&mut catalog_path_bytes, &directory)?;
    let entries = std::fs::read_dir(&canonical_directory)
        .map_err(|error| InterludeError::DirectoryReadFailed(error.to_string()))?;
    let mut directories = vec![(canonical_directory.clone(), entries)];
    let mut audio_files = Vec::new();
    while let Some((_, entries)) = directories.last_mut() {
        let Some(entry) = entries.next() else {
            directories.pop();
            continue;
        };
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let entry_path = entry.path();
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            let Ok(canonical_subdirectory) = std::fs::canonicalize(entry_path) else {
                continue;
            };
            if !canonical_subdirectory.starts_with(&canonical_directory)
                || directories
                    .iter()
                    .any(|(path, _)| path == &canonical_subdirectory)
            {
                continue;
            }
            ensure_path_bytes(&canonical_subdirectory.display().to_string())?;
            let Ok(entries) = std::fs::read_dir(&canonical_subdirectory) else {
                continue;
            };
            directories.push((canonical_subdirectory, entries));
            continue;
        }
        if !file_type.is_file() || !has_allowed_extension(&entry_path) {
            continue;
        }
        let Ok(canonical_file) = std::fs::canonicalize(entry_path) else {
            continue;
        };
        if !canonical_file.starts_with(&canonical_directory) {
            continue;
        }
        let count = audio_files.len() + 1;
        if count > MAX_INTERLUDE_AUDIO_FILES {
            return Err(InterludeError::TooManyAudioFiles {
                count,
                max_files: MAX_INTERLUDE_AUDIO_FILES,
            });
        }
        let canonical_file = canonical_file.display().to_string();
        add_catalog_path_bytes(&mut catalog_path_bytes, &canonical_file)?;
        audio_files.push(canonical_file);
    }
    audio_files.sort();
    Ok(InterludeCatalog {
        directory,
        audio_files,
    })
}

fn add_catalog_path_bytes(total: &mut usize, path: &str) -> Result<(), InterludeError> {
    ensure_path_bytes(path)?;
    let path_bytes = path.len();
    let next_total = total.checked_add(path_bytes).unwrap_or(usize::MAX);
    if next_total > MAX_INTERLUDE_CATALOG_PATH_BYTES {
        return Err(InterludeError::CatalogPathBytesExceeded {
            bytes: next_total,
            max_bytes: MAX_INTERLUDE_CATALOG_PATH_BYTES,
        });
    }
    *total = next_total;
    Ok(())
}

fn ensure_path_bytes(path: &str) -> Result<(), InterludeError> {
    let path_bytes = path.len();
    if path_bytes > MAX_INTERLUDE_PATH_UTF8_BYTES {
        return Err(InterludeError::PathTooLong {
            bytes: path_bytes,
            max_bytes: MAX_INTERLUDE_PATH_UTF8_BYTES,
        });
    }
    Ok(())
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
        .is_some_and(|value| SUPPORTED_SOURCE_MEDIA_EXTENSIONS.contains(&value.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn expected_default_audio_preset_ids() -> Vec<String> {
        (1..=20).map(|number| format!("p{number:02}")).collect()
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "autolive-interlude-limit-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system time")
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).expect("test directory");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn defaults_use_the_low_perception_random_pool_and_keep_p01_fixed() {
        let config = InterludeConfig::default();

        assert_eq!(config.audio_preset_ids, expected_default_audio_preset_ids());
        assert_eq!(config.audio_fixed_preset_id, "p01");
        assert_eq!(config.ducking_depth_db, -60.0);
        assert!(!config
            .audio_preset_ids
            .iter()
            .any(|id| id == "p21" || id == "p22"));
    }

    #[test]
    fn legacy_config_and_snapshot_without_preset_fields_use_the_low_perception_pool() {
        let config: InterludeConfig = serde_json::from_value(serde_json::json!({
            "enabled": false,
            "directory": null,
            "interval_min_ms": 8_000,
            "interval_max_ms": 13_000,
            "volume_db": 0.0,
            "ducking_depth_db": -12.0,
            "ducking_attack_ms": 50,
            "ducking_release_ms": 250
        }))
        .expect("legacy interlude config should deserialize");
        assert_eq!(config.audio_fixed_preset_id, "p01");
        assert_eq!(config.audio_preset_ids, expected_default_audio_preset_ids());

        let snapshot: InterludeSnapshot = serde_json::from_value(serde_json::json!({
            "enabled": false,
            "directory": null,
            "audio_files": [],
            "audio_count": 0,
            "status": "disabled",
            "error": null,
            "interval_min_ms": 8_000,
            "interval_max_ms": 13_000,
            "volume_db": 0.0,
            "ducking_depth_db": -12.0,
            "ducking_attack_ms": 50,
            "ducking_release_ms": 250
        }))
        .expect("legacy interlude snapshot should deserialize");
        assert_eq!(snapshot.audio_fixed_preset_id, "p01");
        assert_eq!(
            snapshot.audio_preset_ids,
            expected_default_audio_preset_ids()
        );
    }

    #[test]
    fn catalog_accepts_1000_audio_files_and_rejects_the_1001st() {
        let temp = TempDir::new();
        for index in 0..MAX_INTERLUDE_AUDIO_FILES {
            std::fs::write(temp.0.join(format!("audio-{index:03}.wav")), b"wav")
                .expect("bounded audio file");
        }
        let directory = temp.0.display().to_string();
        assert_eq!(
            scan_catalog(&directory)
                .expect("1000 audio files should be accepted")
                .audio_files
                .len(),
            MAX_INTERLUDE_AUDIO_FILES
        );

        std::fs::write(temp.0.join("audio-1000.wav"), b"wav").expect("1001st audio file");
        assert_eq!(
            scan_catalog(&directory).expect_err("1001 audio files should be rejected"),
            InterludeError::TooManyAudioFiles {
                count: MAX_INTERLUDE_AUDIO_FILES + 1,
                max_files: MAX_INTERLUDE_AUDIO_FILES,
            }
        );
    }

    #[test]
    fn catalog_path_accepts_4096_bytes_and_rejects_4097_bytes() {
        let mut total = 0;
        add_catalog_path_bytes(&mut total, &"a".repeat(MAX_INTERLUDE_PATH_UTF8_BYTES))
            .expect("4096-byte path should be accepted");
        assert_eq!(total, MAX_INTERLUDE_PATH_UTF8_BYTES);

        assert_eq!(
            add_catalog_path_bytes(&mut total, &"a".repeat(MAX_INTERLUDE_PATH_UTF8_BYTES + 1))
                .expect_err("4097-byte path should be rejected"),
            InterludeError::PathTooLong {
                bytes: MAX_INTERLUDE_PATH_UTF8_BYTES + 1,
                max_bytes: MAX_INTERLUDE_PATH_UTF8_BYTES,
            }
        );
    }

    #[test]
    fn catalog_paths_accept_512_kib_and_reject_the_next_byte() {
        let mut total = MAX_INTERLUDE_CATALOG_PATH_BYTES - 1;
        add_catalog_path_bytes(&mut total, "a").expect("512 KiB total should be accepted");
        assert_eq!(total, MAX_INTERLUDE_CATALOG_PATH_BYTES);

        assert_eq!(
            add_catalog_path_bytes(&mut total, "a")
                .expect_err("more than 512 KiB should be rejected"),
            InterludeError::CatalogPathBytesExceeded {
                bytes: MAX_INTERLUDE_CATALOG_PATH_BYTES + 1,
                max_bytes: MAX_INTERLUDE_CATALOG_PATH_BYTES,
            }
        );
    }
}
