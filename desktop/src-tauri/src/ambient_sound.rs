use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

const ALLOWED_EXTENSIONS: [&str; 6] = ["mp3", "wav", "m4a", "aac", "ogg", "flac"];
const MAX_PATH_BYTES: usize = 32 * 1024;
pub const BUNDLED_AMBIENT_SOUND_RELATIVE_PATH: &str = "ambient/low-level-room-tone.wav";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmbientSoundSource {
    User,
    Bundled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAmbientSound {
    pub path: PathBuf,
    pub source: AmbientSoundSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AmbientSoundError {
    PathTooLong,
    Missing { source: AmbientSoundSource },
    UnsupportedFormat,
}

impl Display for AmbientSoundError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PathTooLong => formatter.write_str("环境声素材路径超过允许长度"),
            Self::Missing { source } => match source {
                AmbientSoundSource::User => formatter.write_str("用户环境声素材不存在或不可读取"),
                AmbientSoundSource::Bundled => {
                    formatter.write_str("程序内置环境底噪资源不存在或不可读取")
                }
            },
            Self::UnsupportedFormat => {
                formatter.write_str("环境声素材必须是 mp3、wav、m4a、aac、ogg 或 flac 文件")
            }
        }
    }
}

impl std::error::Error for AmbientSoundError {}

pub fn resolve_ambient_sound(
    resource_dir: &Path,
    user_path: Option<&Path>,
    required: bool,
) -> Result<Option<ResolvedAmbientSound>, AmbientSoundError> {
    if !required {
        return Ok(None);
    }
    if let Some(path) = user_path {
        return canonical_audio_file(path, AmbientSoundSource::User).map(Some);
    }
    canonical_audio_file(
        &resource_dir.join(BUNDLED_AMBIENT_SOUND_RELATIVE_PATH),
        AmbientSoundSource::Bundled,
    )
    .map(Some)
}

pub fn revalidate_ambient_sound(
    selection: &ResolvedAmbientSound,
) -> Result<ResolvedAmbientSound, AmbientSoundError> {
    canonical_audio_file(&selection.path, selection.source)
}

fn canonical_audio_file(
    path: &Path,
    source: AmbientSoundSource,
) -> Result<ResolvedAmbientSound, AmbientSoundError> {
    if path.as_os_str().len() > MAX_PATH_BYTES {
        return Err(AmbientSoundError::PathTooLong);
    }
    let canonical =
        std::fs::canonicalize(path).map_err(|_| AmbientSoundError::Missing { source })?;
    let extension_allowed = canonical
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|extension| ALLOWED_EXTENSIONS.contains(&extension.as_str()));
    if !canonical.is_file() {
        return Err(AmbientSoundError::Missing { source });
    }
    if !extension_allowed {
        return Err(AmbientSoundError::UnsupportedFormat);
    }
    Ok(ResolvedAmbientSound {
        path: canonical,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        resolve_ambient_sound, revalidate_ambient_sound, AmbientSoundError, AmbientSoundSource,
        BUNDLED_AMBIENT_SOUND_RELATIVE_PATH,
    };
    use std::fs;
    use std::path::Path;

    fn fixture_root(name: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "autolive-ambient-{name}-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn selected_user_file_resolves_to_a_canonical_path() {
        let root = fixture_root("priority");
        let user = root.join("selected.wav");
        fs::create_dir_all(&root).expect("create fixture");
        fs::write(&user, b"user").expect("write user fixture");

        let resolved = resolve_ambient_sound(&root, Some(&user), true)
            .expect("user ambient should resolve")
            .expect("required ambient");

        assert_eq!(resolved.source, AmbientSoundSource::User);
        assert_eq!(
            resolved.path,
            fs::canonicalize(&user).expect("canonical user")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zero_mix_does_not_require_an_ambient_selection() {
        assert_eq!(
            resolve_ambient_sound(Path::new("missing-resource-root"), None, false),
            Ok(None)
        );
        assert_eq!(
            resolve_ambient_sound(
                Path::new("missing-resource-root"),
                Some(Path::new("missing-user.wav")),
                false,
            ),
            Ok(None)
        );
    }

    #[test]
    fn required_ambient_falls_back_to_packaged_audio_without_a_user_selection() {
        let root = fixture_root("packaged");
        let packaged = root.join(BUNDLED_AMBIENT_SOUND_RELATIVE_PATH);
        fs::create_dir_all(packaged.parent().expect("packaged parent"))
            .expect("create packaged fixture directory");
        fs::write(&packaged, b"packaged").expect("write packaged fixture");
        let resolved = resolve_ambient_sound(&root, None, true)
            .expect("packaged ambient should resolve")
            .expect("required ambient");

        assert_eq!(resolved.source, AmbientSoundSource::Bundled);
        assert_eq!(
            resolved.path,
            fs::canonicalize(packaged).expect("canonical")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_packaged_fallback_is_a_recoverable_source_specific_error() {
        let root = fixture_root("missing-packaged");

        assert!(matches!(
            resolve_ambient_sound(&root, None, true),
            Err(AmbientSoundError::Missing {
                source: AmbientSoundSource::Bundled
            })
        ));
    }

    #[test]
    fn missing_and_unsupported_inputs_are_recoverable_errors() {
        let root = fixture_root("invalid");
        fs::create_dir_all(&root).expect("create fixture");
        assert!(matches!(
            resolve_ambient_sound(&root, Some(Path::new("missing.wav")), true),
            Err(AmbientSoundError::Missing {
                source: AmbientSoundSource::User
            })
        ));

        let unsupported = root.join("ambient.txt");
        fs::write(&unsupported, b"fixture").expect("write fixture");
        assert_eq!(
            resolve_ambient_sound(&root, Some(&unsupported), true),
            Err(AmbientSoundError::UnsupportedFormat)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_removed_resolved_file_is_rejected_before_reuse() {
        let root = fixture_root("removed");
        let user = root.join("selected.wav");
        fs::create_dir_all(&root).expect("create fixture");
        fs::write(&user, b"fixture").expect("write fixture");
        let resolved = resolve_ambient_sound(&root, Some(&user), true)
            .expect("resolve fixture")
            .expect("required fixture");
        fs::remove_file(user).expect("remove fixture");

        assert!(matches!(
            revalidate_ambient_sound(&resolved),
            Err(AmbientSoundError::Missing {
                source: AmbientSoundSource::User
            })
        ));
        let _ = fs::remove_dir_all(root);
    }
}
