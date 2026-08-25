use crate::runtime_resources::{
    RuntimeResourceComponent, RuntimeResourceInstaller, RuntimeResourceState, RuntimeResourceStatus,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

type StatusUpdate = Box<dyn Fn(RuntimeResourceStatus) + Send + 'static>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeResourceTaskShutdown {
    Idle,
    Joined,
    TimedOut,
}

pub fn handle_runtime_resource_exit(
    result: Result<RuntimeResourceTaskShutdown, String>,
    force_exit: impl FnOnce(),
) -> Result<RuntimeResourceTaskShutdown, String> {
    if matches!(&result, Ok(RuntimeResourceTaskShutdown::TimedOut) | Err(_)) {
        force_exit();
    }
    result
}

#[derive(Debug)]
pub struct RuntimeResourceTask {
    status: Arc<Mutex<RuntimeResourceStatus>>,
    cancel: Arc<AtomicBool>,
    shutting_down: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Default for RuntimeResourceTask {
    fn default() -> Self {
        Self {
            status: Arc::new(Mutex::new(RuntimeResourceStatus {
                state: RuntimeResourceState::NotInstalled,
                component: None,
                current_file: None,
                downloaded_bytes: 0,
                total_bytes: 0,
                bytes_per_second: 0,
                installed_bytes: 0,
                resource_root: String::new(),
                error: None,
            })),
            cancel: Arc::new(AtomicBool::new(false)),
            shutting_down: AtomicBool::new(false),
            worker: Mutex::new(None),
        }
    }
}

impl RuntimeResourceTask {
    pub fn start_install(
        &self,
        component: RuntimeResourceComponent,
        installer: RuntimeResourceInstaller,
    ) -> Result<RuntimeResourceStatus, String> {
        let initial = operation_status(component, installer.resource_root());
        self.start_operation(initial, move |cancel, update| {
            installer
                .install(component, cancel.as_ref(), update)
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
    }

    pub fn start_import(
        &self,
        component: RuntimeResourceComponent,
        installer: RuntimeResourceInstaller,
        source_root: PathBuf,
    ) -> Result<RuntimeResourceStatus, String> {
        let initial = operation_status(component, installer.resource_root());
        self.start_operation(initial, move |cancel, update| {
            installer
                .import_directory(component, &source_root, cancel.as_ref(), update)
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
    }

    pub fn start_clear(
        &self,
        installer: RuntimeResourceInstaller,
    ) -> Result<RuntimeResourceStatus, String> {
        let initial = RuntimeResourceStatus {
            component: None,
            ..operation_status(RuntimeResourceComponent::Media, installer.resource_root())
        };
        self.start_operation(initial, move |cancel, update| {
            installer
                .clear_current_release(cancel.as_ref(), update)
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
    }

    pub fn status(
        &self,
        component: RuntimeResourceComponent,
    ) -> Result<RuntimeResourceStatus, String> {
        let mut worker = self.lock_worker()?;
        self.reap_finished_locked(&mut worker)?;
        let mut status = self.lock_status()?.clone();
        if status.component.is_none() && status.state == RuntimeResourceState::NotInstalled {
            status.component = Some(component);
        }
        Ok(status)
    }

    pub fn running_status(&self) -> Result<Option<RuntimeResourceStatus>, String> {
        let mut worker = self.lock_worker()?;
        self.reap_finished_locked(&mut worker)?;
        if worker.is_some() {
            return Ok(Some(self.lock_status()?.clone()));
        }
        Ok(None)
    }

    pub fn inspect_when_idle(
        &self,
        component: RuntimeResourceComponent,
        installer: &RuntimeResourceInstaller,
    ) -> Result<RuntimeResourceStatus, String> {
        if let Some(status) = self.running_status()? {
            return Ok(status);
        }
        let previous = self.lock_status()?.clone();
        if previous.component == Some(component)
            && matches!(
                previous.state,
                RuntimeResourceState::Failed | RuntimeResourceState::Cancelled
            )
        {
            return Ok(previous);
        }
        let inspected = installer
            .inspect(component)
            .map_err(|error| error.to_string())?;
        Ok(self.running_status()?.unwrap_or(inspected))
    }

    pub fn record_failure(
        &self,
        component: RuntimeResourceComponent,
        error: impl Into<String>,
        resource_root: &std::path::Path,
    ) -> Result<RuntimeResourceStatus, String> {
        let mut worker = self.lock_worker()?;
        self.reap_finished_locked(&mut worker)?;
        if worker.is_some() {
            return Ok(self.lock_status()?.clone());
        }
        let status = RuntimeResourceStatus {
            state: RuntimeResourceState::Failed,
            component: Some(component),
            current_file: None,
            downloaded_bytes: 0,
            total_bytes: 0,
            bytes_per_second: 0,
            installed_bytes: 0,
            resource_root: resource_root.display().to_string(),
            error: Some(error.into()),
        };
        *self.lock_status()? = status.clone();
        Ok(status)
    }

    pub fn cancel(&self) -> Result<RuntimeResourceStatus, String> {
        self.cancel.store(true, Ordering::Release);
        Ok(self.lock_status()?.clone())
    }

    pub fn shutdown(&self, budget: Duration) -> Result<RuntimeResourceTaskShutdown, String> {
        self.begin_shutdown()?;
        let deadline = Instant::now() + budget;
        loop {
            {
                let mut worker = self.lock_worker()?;
                if worker.is_none() {
                    return Ok(RuntimeResourceTaskShutdown::Idle);
                }
                if worker.as_ref().is_some_and(JoinHandle::is_finished) {
                    self.reap_finished_locked(&mut worker)?;
                    return Ok(RuntimeResourceTaskShutdown::Joined);
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(RuntimeResourceTaskShutdown::TimedOut);
            }
            thread::sleep(
                deadline
                    .saturating_duration_since(now)
                    .min(Duration::from_millis(10)),
            );
        }
    }

    pub fn begin_shutdown(&self) -> Result<(), String> {
        let _worker = self.lock_worker()?;
        self.shutting_down.store(true, Ordering::Release);
        self.cancel.store(true, Ordering::Release);
        Ok(())
    }

    fn start_operation(
        &self,
        initial: RuntimeResourceStatus,
        operation: impl FnOnce(Arc<AtomicBool>, StatusUpdate) -> Result<(), String> + Send + 'static,
    ) -> Result<RuntimeResourceStatus, String> {
        let mut worker = self.lock_worker()?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("应用正在退出，不再接受新的运行资源任务".to_owned());
        }
        self.reap_finished_locked(&mut worker)?;
        if worker.is_some() {
            return Ok(self.lock_status()?.clone());
        }

        self.cancel.store(false, Ordering::Release);
        *self.lock_status()? = initial.clone();
        let cancel = Arc::clone(&self.cancel);
        let status = Arc::clone(&self.status);
        let handle = match thread::Builder::new()
            .name("runtime-resource-operation".to_owned())
            .spawn(move || {
                let status_for_update = Arc::clone(&status);
                let update: StatusUpdate = Box::new(move |next| {
                    *status_for_update
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = next;
                });
                let result = operation(cancel, update);
                let mut current = status
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if !is_terminal(current.state) {
                    current.state = RuntimeResourceState::Failed;
                    current.error = Some(
                        result
                            .err()
                            .unwrap_or_else(|| "运行资源任务结束但未报告终态".to_owned()),
                    );
                }
            }) {
            Ok(handle) => handle,
            Err(error) => {
                let message = format!("无法启动运行资源任务：{error}");
                *self.lock_status()? = RuntimeResourceStatus {
                    state: RuntimeResourceState::Failed,
                    error: Some(message.clone()),
                    ..initial
                };
                return Err(message);
            }
        };
        *worker = Some(handle);
        Ok(initial)
    }

    fn reap_finished_locked(
        &self,
        worker: &mut MutexGuard<'_, Option<JoinHandle<()>>>,
    ) -> Result<(), String> {
        if !worker.as_ref().is_some_and(JoinHandle::is_finished) {
            return Ok(());
        }
        let Some(handle) = worker.take() else {
            return Ok(());
        };
        if handle.join().is_err() {
            let previous = self.lock_status()?.clone();
            *self.lock_status()? = RuntimeResourceStatus {
                state: RuntimeResourceState::Failed,
                error: Some("运行资源任务异常终止".to_owned()),
                ..previous
            };
        }
        Ok(())
    }

    fn lock_worker(&self) -> Result<MutexGuard<'_, Option<JoinHandle<()>>>, String> {
        self.worker
            .lock()
            .map_err(|_| "运行资源任务句柄锁已损坏".to_owned())
    }

    fn lock_status(&self) -> Result<MutexGuard<'_, RuntimeResourceStatus>, String> {
        self.status
            .lock()
            .map_err(|_| "运行资源状态锁已损坏".to_owned())
    }

    #[cfg(test)]
    fn worker_is_owned(&self) -> Result<bool, String> {
        Ok(self.lock_worker()?.is_some())
    }
}

impl Drop for RuntimeResourceTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        let worker = self
            .worker
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(worker) = worker {
            let _ignored = worker.join();
        }
    }
}

fn operation_status(
    component: RuntimeResourceComponent,
    resource_root: &std::path::Path,
) -> RuntimeResourceStatus {
    RuntimeResourceStatus {
        state: RuntimeResourceState::Checking,
        component: Some(component),
        current_file: None,
        downloaded_bytes: 0,
        total_bytes: 0,
        bytes_per_second: 0,
        installed_bytes: 0,
        resource_root: resource_root.display().to_string(),
        error: None,
    }
}

fn is_terminal(state: RuntimeResourceState) -> bool {
    matches!(
        state,
        RuntimeResourceState::NotInstalled
            | RuntimeResourceState::Ready
            | RuntimeResourceState::Failed
            | RuntimeResourceState::Cancelled
    )
}

#[cfg(test)]
mod tests {
    use super::{handle_runtime_resource_exit, RuntimeResourceTask, RuntimeResourceTaskShutdown};
    use crate::runtime_resources::{
        RuntimeResourceComponent, RuntimeResourceInstaller, RuntimeResourceManifest,
        RuntimeResourceState, RuntimeResourceStatus, PRODUCTION_BASE_URL,
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn checking_status() -> RuntimeResourceStatus {
        RuntimeResourceStatus {
            state: RuntimeResourceState::Checking,
            component: Some(RuntimeResourceComponent::Media),
            current_file: None,
            downloaded_bytes: 0,
            total_bytes: 0,
            bytes_per_second: 0,
            installed_bytes: 0,
            resource_root: "/app-data/runtime-resources/v0.1.0".to_owned(),
            error: None,
        }
    }

    fn ready_status() -> RuntimeResourceStatus {
        RuntimeResourceStatus {
            state: RuntimeResourceState::Ready,
            ..checking_status()
        }
    }

    #[test]
    fn exit_policy_forces_only_timeout_and_shutdown_errors() {
        for shutdown in [
            RuntimeResourceTaskShutdown::Idle,
            RuntimeResourceTaskShutdown::Joined,
        ] {
            let forced = AtomicUsize::new(0);
            let result = handle_runtime_resource_exit(Ok(shutdown), || {
                forced.fetch_add(1, Ordering::Relaxed);
            });
            assert_eq!(result, Ok(shutdown));
            assert_eq!(forced.load(Ordering::Relaxed), 0);
        }

        let forced = AtomicUsize::new(0);
        let result =
            handle_runtime_resource_exit(Ok(RuntimeResourceTaskShutdown::TimedOut), || {
                forced.fetch_add(1, Ordering::Relaxed);
            });
        assert_eq!(result, Ok(RuntimeResourceTaskShutdown::TimedOut));
        assert_eq!(forced.load(Ordering::Relaxed), 1);

        let forced = AtomicUsize::new(0);
        let result = handle_runtime_resource_exit(Err("shutdown failed".to_owned()), || {
            forced.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(result, Err("shutdown failed".to_owned()));
        assert_eq!(forced.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn duplicate_start_reuses_the_running_task() {
        let task = RuntimeResourceTask::default();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        task.start_operation(checking_status(), move |_cancel, update| {
            started_tx.send(()).expect("start signal should send");
            release_rx.recv().expect("release signal should arrive");
            update(ready_status());
            Ok(())
        })
        .expect("first task should start");
        started_rx.recv().expect("worker should start");

        let duplicate_ran = Arc::new(AtomicBool::new(false));
        let duplicate_flag = Arc::clone(&duplicate_ran);
        let reused = task
            .start_operation(checking_status(), move |_cancel, _update| {
                duplicate_flag.store(true, Ordering::Release);
                Ok(())
            })
            .expect("duplicate start should reuse current task");

        assert_eq!(reused.state, RuntimeResourceState::Checking);
        assert!(!duplicate_ran.load(Ordering::Acquire));
        release_tx.send(()).expect("worker should be released");
    }

    #[test]
    fn operation_error_without_status_callback_becomes_failed() {
        let task = RuntimeResourceTask::default();
        task.start_operation(checking_status(), move |_cancel, _update| {
            Err("operation failed before progress".to_owned())
        })
        .expect("task should start");

        let deadline = Instant::now() + Duration::from_secs(1);
        let status = loop {
            let status = task
                .status(RuntimeResourceComponent::Media)
                .expect("status should be readable");
            if status.state == RuntimeResourceState::Failed || Instant::now() >= deadline {
                break status;
            }
            std::thread::yield_now();
        };

        assert_eq!(status.state, RuntimeResourceState::Failed);
        assert_eq!(
            status.error.as_deref(),
            Some("operation failed before progress")
        );
    }

    #[test]
    fn inspect_keeps_a_terminal_failure_and_its_progress_until_retry() {
        let task = RuntimeResourceTask::default();
        *task.status.lock().expect("status lock should work") = RuntimeResourceStatus {
            state: RuntimeResourceState::Failed,
            component: Some(RuntimeResourceComponent::Media),
            current_file: Some("x86_64-pc-windows-msvc/binaries/ffmpeg.exe".to_owned()),
            downloaded_bytes: 7,
            total_bytes: 10,
            bytes_per_second: 0,
            installed_bytes: 3,
            resource_root: "/app-data/runtime-resources/v0.1.0".to_owned(),
            error: Some("download failed".to_owned()),
        };
        let root = std::env::temp_dir().join(format!(
            "autolive-runtime-task-inspect-{}",
            std::process::id()
        ));
        let installer = RuntimeResourceInstaller::from_manifest(
            RuntimeResourceManifest {
                schema_version: 1,
                release: "v0.1.0".to_owned(),
                target: "x86_64-pc-windows-msvc".to_owned(),
                base_url: PRODUCTION_BASE_URL.to_owned(),
                files: Vec::new(),
            },
            &root,
        )
        .expect("installer fixture");

        let status = task
            .inspect_when_idle(RuntimeResourceComponent::Media, &installer)
            .expect("terminal status should remain visible");
        assert_eq!(status.state, RuntimeResourceState::Failed);
        assert_eq!(status.downloaded_bytes, 7);
        assert_eq!(status.error.as_deref(), Some("download failed"));

        drop(installer);
        std::fs::remove_dir_all(root).expect("test directory cleanup");
    }

    #[test]
    fn record_failure_does_not_overwrite_a_running_operation() {
        let task = RuntimeResourceTask::default();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        task.start_operation(checking_status(), move |_cancel, update| {
            started_tx.send(()).expect("start signal should send");
            release_rx.recv().expect("release signal should arrive");
            update(ready_status());
            Ok(())
        })
        .expect("operation should start");
        started_rx.recv().expect("operation should be running");

        let status = task
            .record_failure(
                RuntimeResourceComponent::Media,
                "late manifest failure",
                std::path::Path::new("/different-root"),
            )
            .expect("running status should win");

        assert_eq!(status.state, RuntimeResourceState::Checking);
        assert_eq!(status.component, Some(RuntimeResourceComponent::Media));
        release_tx.send(()).expect("operation should finish");
    }

    #[test]
    fn terminal_status_reaps_the_finished_join_handle() {
        let task = RuntimeResourceTask::default();
        task.start_operation(checking_status(), move |_cancel, update| {
            update(ready_status());
            Ok(())
        })
        .expect("task should start");

        let deadline = Instant::now() + Duration::from_secs(1);
        while task.worker_is_owned().expect("worker lock should work") && Instant::now() < deadline
        {
            let _status = task
                .status(RuntimeResourceComponent::Media)
                .expect("status should be readable");
            std::thread::yield_now();
        }

        assert_eq!(
            task.status(RuntimeResourceComponent::Media)
                .expect("terminal status should remain visible")
                .state,
            RuntimeResourceState::Ready
        );
        assert!(!task.worker_is_owned().expect("worker should be reaped"));
    }

    #[test]
    fn shutdown_cancels_and_joins_a_cooperative_task() {
        let task = RuntimeResourceTask::default();
        let (started_tx, started_rx) = mpsc::channel();
        task.start_operation(checking_status(), move |cancel, update| {
            started_tx.send(()).expect("start signal should send");
            while !cancel.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            update(RuntimeResourceStatus {
                state: RuntimeResourceState::Cancelled,
                ..checking_status()
            });
            Ok(())
        })
        .expect("task should start");
        started_rx.recv().expect("worker should start");

        assert_eq!(
            task.shutdown(Duration::from_secs(1))
                .expect("shutdown should succeed"),
            RuntimeResourceTaskShutdown::Joined
        );
        assert!(!task.worker_is_owned().expect("worker should be reaped"));
    }

    #[test]
    fn shutdown_gate_rejects_a_late_runtime_resource_task() {
        let task = RuntimeResourceTask::default();
        assert_eq!(
            task.shutdown(Duration::ZERO)
                .expect("idle shutdown should succeed"),
            RuntimeResourceTaskShutdown::Idle
        );

        let error = task
            .start_operation(checking_status(), move |_cancel, _update| Ok(()))
            .expect_err("shutdown gate must reject a late task");

        assert!(error.contains("正在退出"));
        assert!(!task
            .worker_is_owned()
            .expect("no worker should be installed"));
    }

    #[test]
    fn shutdown_timeout_keeps_ownership_of_an_uninterruptible_worker() {
        let task = RuntimeResourceTask::default();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        task.start_operation(checking_status(), move |_cancel, update| {
            started_tx.send(()).expect("start signal should send");
            release_rx.recv().expect("release signal should arrive");
            update(ready_status());
            Ok(())
        })
        .expect("task should start");
        started_rx.recv().expect("worker should start");

        assert_eq!(
            task.shutdown(Duration::from_millis(20))
                .expect("bounded shutdown should return"),
            RuntimeResourceTaskShutdown::TimedOut
        );
        assert!(task
            .worker_is_owned()
            .expect("timed out worker stays owned"));

        release_tx.send(()).expect("worker should be released");
        let deadline = Instant::now() + Duration::from_secs(1);
        while task.worker_is_owned().expect("worker lock should work") && Instant::now() < deadline
        {
            let _status = task
                .status(RuntimeResourceComponent::Media)
                .expect("status should reap completed worker");
            std::thread::yield_now();
        }
        assert!(!task.worker_is_owned().expect("worker should be reaped"));
    }

    #[test]
    fn drop_cancels_and_joins_a_cooperative_worker() {
        let task = RuntimeResourceTask::default();
        let (finished_tx, finished_rx) = mpsc::channel();
        task.start_operation(checking_status(), move |cancel, update| {
            while !cancel.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            std::thread::sleep(Duration::from_millis(30));
            update(RuntimeResourceStatus {
                state: RuntimeResourceState::Cancelled,
                ..checking_status()
            });
            finished_tx.send(()).expect("finish signal should send");
            Ok(())
        })
        .expect("task should start");

        drop(task);

        finished_rx
            .try_recv()
            .expect("drop must wait until the worker has finished");
    }
}
