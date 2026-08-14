use crate::cancellation::CancellationToken;
use crate::errors::FileHashError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHashRequestDto {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mp4Sha256Dto {
    pub mp4_sha256: String,
}

pub fn hash_file_sha256_hex(
    request: &FileHashRequestDto,
    cancellation: &CancellationToken,
) -> Result<String, FileHashError> {
    let path = resolve_file_path(&request.path, cancellation)?;
    hash_file_at_path(&path, cancellation)
}

pub fn hash_mp4_sha256_dto(
    request: &FileHashRequestDto,
    cancellation: &CancellationToken,
) -> Result<Mp4Sha256Dto, FileHashError> {
    let path = resolve_file_path(&request.path, cancellation)?;
    let extension = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_ascii_lowercase);
    if extension.as_deref() != Some("mp4") {
        return Err(FileHashError::UnsupportedExtension {
            path: path.display().to_string(),
        });
    }

    let mp4_sha256 = hash_file_at_path(&path, cancellation)?;
    Ok(Mp4Sha256Dto { mp4_sha256 })
}

pub fn hash_file_at_path(
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<String, FileHashError> {
    if cancellation.is_cancelled() {
        return Err(FileHashError::Cancelled);
    }

    let mut file = File::open(path).map_err(|_| FileHashError::FileOpenFailed {
        path: path.display().to_string(),
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];

    loop {
        if cancellation.is_cancelled() {
            return Err(FileHashError::Cancelled);
        }

        let bytes_read = file
            .read(&mut buffer)
            .map_err(|_| FileHashError::FileReadFailed {
                path: path.display().to_string(),
            })?;
        if bytes_read == 0 {
            break;
        }

        hasher.update(&buffer[..bytes_read]);
    }

    let digest = hasher.finalize();
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ignored = write!(&mut output, "{byte:02x}");
    }
    Ok(output)
}

fn resolve_file_path(
    raw_path: &str,
    cancellation: &CancellationToken,
) -> Result<PathBuf, FileHashError> {
    if cancellation.is_cancelled() {
        return Err(FileHashError::Cancelled);
    }

    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(FileHashError::EmptyPath);
    }

    let canonical_path =
        std::fs::canonicalize(trimmed).map_err(|_| FileHashError::CanonicalizeFailed {
            path: trimmed.to_owned(),
        })?;
    let metadata =
        std::fs::metadata(&canonical_path).map_err(|_| FileHashError::CanonicalizeFailed {
            path: canonical_path.display().to_string(),
        })?;

    if !metadata.is_file() {
        return Err(FileHashError::NotAFile {
            path: canonical_path.display().to_string(),
        });
    }

    Ok(canonical_path)
}

#[cfg(test)]
mod tests {
    use super::{hash_file_sha256_hex, hash_mp4_sha256_dto, FileHashRequestDto};
    use crate::cancellation::CancellationToken;
    use crate::errors::FileHashError;
    use std::fs;
    use std::path::PathBuf;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(suffix: &str) -> Self {
            let unique = format!(
                "autolive-desktop-core-{}-{}-{}",
                suffix,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time should be after unix epoch")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("test directory should be created");
            Self { path }
        }

        fn path(&self) -> &PathBuf {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ignored = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn hash_request_is_future_ipc_friendly() {
        let _request = FileHashRequestDto {
            path: String::from("/tmp/example.mp4"),
        };
    }

    #[test]
    fn hash_boundary_can_be_called() {
        let cancellation = CancellationToken::new();
        let request = FileHashRequestDto {
            path: String::from("/tmp/example.mp4"),
        };

        let _result = hash_file_sha256_hex(&request, &cancellation);
    }

    #[test]
    fn hash_rejects_empty_path() {
        let cancellation = CancellationToken::new();
        let request = FileHashRequestDto {
            path: String::new(),
        };

        let result = hash_file_sha256_hex(&request, &cancellation);

        assert_eq!(result, Err(FileHashError::EmptyPath));
    }

    #[test]
    fn hash_rejects_directory_path() {
        let cancellation = CancellationToken::new();
        let directory = TestDir::new("hash-dir");
        let request = FileHashRequestDto {
            path: directory.path().display().to_string(),
        };

        let result = hash_file_sha256_hex(&request, &cancellation);

        assert!(matches!(result, Err(FileHashError::NotAFile { .. })));
    }

    #[test]
    fn hash_matches_known_sha256_vector() {
        let cancellation = CancellationToken::new();
        let directory = TestDir::new("hash-vector");
        let file_path = directory.path().join("hello.txt");
        fs::write(&file_path, b"hello world").expect("test file should be written");

        let request = FileHashRequestDto {
            path: file_path.display().to_string(),
        };
        let result =
            hash_file_sha256_hex(&request, &cancellation).expect("hash calculation should succeed");

        assert_eq!(
            result,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn mp4_hash_requires_mp4_extension() {
        let cancellation = CancellationToken::new();
        let directory = TestDir::new("hash-mp4-extension");
        let file_path = directory.path().join("sample.txt");
        fs::write(&file_path, b"hello world").expect("test file should be written");

        let request = FileHashRequestDto {
            path: file_path.display().to_string(),
        };
        let result = hash_mp4_sha256_dto(&request, &cancellation);

        assert!(matches!(
            result,
            Err(FileHashError::UnsupportedExtension { .. })
        ));
    }

    #[test]
    fn hash_stops_when_cancelled() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let request = FileHashRequestDto {
            path: String::from("/tmp/example.mp4"),
        };
        let result = hash_file_sha256_hex(&request, &cancellation);

        assert_eq!(result, Err(FileHashError::Cancelled));
    }
}
