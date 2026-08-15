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

#[derive(Debug)]
pub struct RuntimeResourceTask {
    status: Arc<Mutex<RuntimeResourceStatus>>,
    cancel: Arc<AtomicBool>,
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
        self.start_operation(initial.clone(), move |_cancel, update| {
            match installer.clear_current_release() {
                Ok(status) => {
                    update(status);
                    Ok(())
                }
                Err(error) => Err(error.to_string()),
            }
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

    pub fn cancel(&self) -> Result<RuntimeResourceStatus, String> {
        self.cancel.store(true, Ordering::Release);
        Ok(self.lock_status()?.clone())
    }

    pub fn shutdown(&self, budget: Duration) -> Result<RuntimeResourceTaskShutdown, String> {
        self.cancel.store(true, Ordering::Release);
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

    fn start_operation(
        &self,
        initial: RuntimeResourceStatus,
        operation: impl FnOnce(Arc<AtomicBool>, StatusUpdate) -> Result<(), String> + Send + 'static,
    ) -> Result<RuntimeResourceStatus, String> {
        let mut worker = self.lock_worker()?;
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
    use super::{RuntimeResourceTask, RuntimeResourceTaskShutdown};
    use crate::runtime_resources::{
        RuntimeResourceComponent, RuntimeResourceState, RuntimeResourceStatus,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
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
}
