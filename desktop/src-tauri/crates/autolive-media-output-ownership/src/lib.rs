use std::fmt;

/// C# 与 Rust/Tauri 桌面端共同争用的用户会话级媒体/输出资源锁。
pub const SHARED_MUTEX_NAME: &str = "Local\\GpAutoLive.MediaOutput.Owner.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaOutputOwnershipError {
    AlreadyOwned,
    ApiUnavailable(u32),
    Unsupported,
}

impl fmt::Display for MediaOutputOwnershipError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyOwned => formatter.write_str("媒体或输出资源已被另一桌面客户端占用"),
            Self::ApiUnavailable(code) => {
                write!(formatter, "无法建立媒体或输出资源锁（Win32 错误 {code}）")
            }
            Self::Unsupported => formatter.write_str("媒体或输出资源锁仅支持 Windows"),
        }
    }
}

impl std::error::Error for MediaOutputOwnershipError {}

/// 持有共享命名 Mutex 的租约。Drop 时释放 Windows Mutex 和句柄。
#[derive(Debug)]
pub struct MediaOutputOwnershipLease {
    #[cfg(windows)]
    handle: windows::Win32::Foundation::HANDLE,
}

impl MediaOutputOwnershipLease {
    /// 立即获取跨 C# / Rust 客户端共享的媒体输出锁，不等待其他客户端。
    pub fn try_acquire() -> Result<Self, MediaOutputOwnershipError> {
        #[cfg(windows)]
        {
            return Self::try_acquire_named(SHARED_MUTEX_NAME);
        }

        #[cfg(not(windows))]
        {
            Err(MediaOutputOwnershipError::Unsupported)
        }
    }

    #[cfg(windows)]
    pub(crate) fn try_acquire_named(name: &str) -> Result<Self, MediaOutputOwnershipError> {
        windows_impl::try_acquire(name)
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::{MediaOutputOwnershipError, MediaOutputOwnershipLease};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, WAIT_ABANDONED, WAIT_FAILED,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

    pub(super) fn try_acquire(
        name: &str,
    ) -> Result<MediaOutputOwnershipLease, MediaOutputOwnershipError> {
        if name.is_empty() || name.encode_utf16().count() > 260 || name.contains('\0') {
            return Err(MediaOutputOwnershipError::ApiUnavailable(
                ERROR_INVALID_PARAMETER.0,
            ));
        }

        let name_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mutex = unsafe {
            // SAFETY: name_wide 在调用期间保持有效且以 NUL 结尾；不传入自定义安全描述符。
            CreateMutexW(None, false, PCWSTR(name_wide.as_ptr()))
        }
        .map_err(|error| MediaOutputOwnershipError::ApiUnavailable(error.code().0 as u32))?;

        let wait_result = unsafe {
            // SAFETY: mutex 来自成功的 CreateMutexW，等待超时为 0，不阻塞 UI/运行时线程。
            WaitForSingleObject(mutex, 0)
        };

        if wait_result == WAIT_OBJECT_0 || wait_result == WAIT_ABANDONED {
            return Ok(MediaOutputOwnershipLease { handle: mutex });
        }

        let error_code = if wait_result == WAIT_FAILED {
            unsafe {
                // SAFETY: GetLastError 无参数且只读取当前线程的 Win32 错误码。
                GetLastError().0
            }
        } else {
            0
        };
        unsafe {
            // SAFETY: mutex 是本函数创建/打开且尚未转移给租约的句柄。
            let _ = CloseHandle(mutex);
        }

        if wait_result == WAIT_TIMEOUT {
            Err(MediaOutputOwnershipError::AlreadyOwned)
        } else {
            Err(MediaOutputOwnershipError::ApiUnavailable(error_code))
        }
    }

    pub(super) fn release(handle: windows::Win32::Foundation::HANDLE) {
        unsafe {
            // SAFETY: handle 由当前租约持有；释放失败时仍关闭句柄，避免关闭阶段阻塞。
            let _ = ReleaseMutex(handle);
            let _ = CloseHandle(handle);
        }
    }
}

#[cfg(windows)]
impl Drop for MediaOutputOwnershipLease {
    fn drop(&mut self) {
        windows_impl::release(self.handle);
    }
}

#[cfg(test)]
mod tests {
    use super::{MediaOutputOwnershipError, MediaOutputOwnershipLease};

    #[cfg(not(windows))]
    #[test]
    fn non_windows_is_explicitly_unsupported() {
        assert!(matches!(
            MediaOutputOwnershipLease::try_acquire(),
            Err(MediaOutputOwnershipError::Unsupported)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn shared_mutex_is_exclusive_and_reusable_after_drop() {
        let mutex_name = format!("Local\\GpAutoLive.Tests.{}", std::process::id());
        let first = MediaOutputOwnershipLease::try_acquire_named(&mutex_name)
            .expect("first lease should acquire");
        let second_name = mutex_name.clone();
        let second_is_rejected = std::thread::spawn(move || {
            matches!(
                MediaOutputOwnershipLease::try_acquire_named(&second_name),
                Err(MediaOutputOwnershipError::AlreadyOwned)
            )
        })
        .join()
        .expect("second acquisition thread should finish");
        assert!(second_is_rejected);
        drop(first);
        MediaOutputOwnershipLease::try_acquire_named(&mutex_name)
            .expect("released lease should be reusable");
    }
}
