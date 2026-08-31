use std::io;
use std::process::Child;

#[cfg(windows)]
mod platform {
    use super::{io, Child};
    use std::os::windows::io::AsRawHandle;
    use win32job::{ExtendedLimitInfo, Job};

    /// 只拥有受管 mpv 会话的 Job Object；`Job::drop` 关闭唯一句柄。
    #[derive(Debug)]
    pub(crate) struct ManagedMpvJob {
        job: Job,
    }

    impl ManagedMpvJob {
        pub(crate) fn create() -> io::Result<Self> {
            let mut limits = ExtendedLimitInfo::new();
            limits.limit_kill_on_job_close();
            Job::create_with_limit_info(&limits)
                .map(|job| Self { job })
                .map_err(io::Error::from)
        }

        pub(crate) fn assign_process(&self, child: &Child) -> io::Result<()> {
            self.job
                .assign_process(child.as_raw_handle() as isize)
                .map_err(io::Error::from)
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{io, Child};

    /// 非 Windows 保留同一所有权接口，进程回收继续走标准 `Child` 路径。
    #[derive(Debug)]
    pub(crate) struct ManagedMpvJob;

    impl ManagedMpvJob {
        pub(crate) fn create() -> io::Result<Self> {
            Ok(Self)
        }

        pub(crate) fn assign_process(&self, _child: &Child) -> io::Result<()> {
            Ok(())
        }
    }
}

pub(crate) use platform::ManagedMpvJob;
