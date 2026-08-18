use autolive_desktop_core::audio_processing::AudioProcessingProfile;
use autolive_desktop_core::cancellation::CancellationToken;
use autolive_desktop_core::direct_model::{
    DelegatedAccessCredential, DirectChatMessage, DirectChatRequest, DirectLeaseDescriptor,
    DirectLeaseSession, DirectModelError,
};
use autolive_desktop_core::errors::{MediaLibraryError, PlaybackError};
use autolive_desktop_core::hashing::hash_file_at_path;
use autolive_desktop_core::interlude_player::{
    prepare_interlude_snapshot, InterludeConfig, InterludeError, InterludeSnapshot,
};
use autolive_desktop_core::media_engine::{
    build_media_render_args, configured_media_engine_paths_with_resource_dir,
    configured_media_engine_status_with_resource_dir, render_media, target_triple,
    MediaEngineStatus, MediaRenderRequest, FFMPEG_PATH_ENV, FFPROBE_PATH_ENV,
};
use autolive_desktop_core::media_library::{
    probe_user_selected_video_with_ffprobe, MediaProbeRequestDto, MediaProbeResultDto,
    SourceMediaDto,
};
use autolive_desktop_core::research_params::{LocalResearchParams, ParameterValidationError};
use autolive_desktop_core::research_worker::{
    configured_research_worker_capabilities, configured_research_worker_executable, run_research,
    validate_research_identifier, ResearchAnalysisRequest, ResearchResult,
    ResearchWorkerCapabilities,
};
use autolive_desktop_core::runtime_resource_task::{
    RuntimeResourceTask, RuntimeResourceTaskShutdown,
};
use autolive_desktop_core::runtime_resources::{
    RuntimeResourceCatalog, RuntimeResourceComponent, RuntimeResourceInstaller,
    RuntimeResourceLayout, RuntimeResourceRoots, RuntimeResourceState, RuntimeResourceStatus,
};
use autolive_desktop_core::speech_to_speech::SpeechToSpeechWorkerCapabilities;
use autolive_desktop_core::speech_to_speech::{
    AudioTrackInput, AudioVariantCandidate, CandidateValidationError, SpeechToSpeechContext,
};
use autolive_desktop_core::speech_to_speech_worker::SpeechToSpeechWorkerError;
use autolive_desktop_core::speech_to_speech_worker::{
    configured_speech_to_speech_worker_capabilities,
    configured_speech_to_speech_worker_capabilities_with_resource_dir,
    run_configured_speech_to_speech_context_worker,
    run_configured_speech_to_speech_context_worker_with_resource_dir,
};
use autolive_desktop_core::window_sizing::{calculate_window_size, WindowSizingError};
use autolive_desktop_core::{PlaybackCore, PlaybackSnapshot};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use sysinfo::{Disks, System};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder, Window};
use tauri_runtime::dpi::{LogicalSize, PhysicalSize};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const MEDIA_CACHE_MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// 媒体处理产物只保留最近 N 个（外加当前/待切换保护文件）。
const MEDIA_CACHE_MAX_FILES: usize = 3;
const RESEARCH_CACHE_MAX_BYTES: u64 = 512 * 1024 * 1024;
const MACOS_NATIVE_TITLEBAR_HEIGHT: f64 = 32.0;
const MEDIA_IMPORT_PROBE_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug, Clone)]
pub struct AppState {
    main_window_label: &'static str,
    playback: Arc<Mutex<PlaybackCore>>,
    speech_worker: Arc<Mutex<Option<SpeechWorkerTask>>>,
    media_worker: Arc<Mutex<Option<MediaWorkerTask>>>,
    research_worker: Arc<Mutex<Option<ResearchWorkerTask>>>,
    research_status: Arc<Mutex<ResearchStatusDto>>,
    runtime_resource_task: Arc<RuntimeResourceTask>,
}

#[derive(Debug)]
struct SpeechWorkerTask {
    cancellation: CancellationToken,
    completed: Arc<AtomicBool>,
    handle: thread::JoinHandle<()>,
}

#[derive(Debug)]
struct MediaWorkerTask {
    cancellation: CancellationToken,
    completed: Arc<AtomicBool>,
    handle: thread::JoinHandle<()>,
}

#[derive(Debug)]
struct ResearchWorkerTask {
    cancellation: CancellationToken,
    completed: Arc<AtomicBool>,
    handle: thread::JoinHandle<()>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinalEffectWindowDto {
    pub label: String,
    pub created: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResizeFinalEffectWindowRequestDto {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FinalEffectWindowSizeDto {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResearchStatusDto {
    pub state: String,
    pub analysis_id: Option<String>,
    pub source_mp4_sha256: Option<String>,
    pub input_mp4_sha256: Option<String>,
    pub current_mp4_sha256: Option<String>,
    pub report_path: Option<String>,
    pub report_sha256: Option<String>,
    pub report_version: Option<String>,
    pub algorithm_version: Option<String>,
    pub random_seed: Option<u64>,
    pub content_similarity_percent: Option<f64>,
    pub media_robustness_score: Option<f64>,
    pub invisible_mark_status: Option<String>,
    pub random_perturbation_applied: Option<bool>,
    pub content_fingerprint: Option<String>,
    pub error: Option<String>,
    #[serde(skip)]
    cache_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheCleanupResultDto {
    pub removed_files: u32,
    pub removed_bytes: u64,
    pub remaining_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdatePlaybackPositionRequestDto {
    pub position_ms: u64,
}

fn apply_research_result(
    status: &mut ResearchStatusDto,
    analysis_id: String,
    result: ResearchResult,
) {
    status.state = "ready".to_owned();
    status.analysis_id = Some(analysis_id);
    status.source_mp4_sha256 = Some(result.source_mp4_sha256);
    status.input_mp4_sha256 = Some(result.input_mp4_sha256);
    status.current_mp4_sha256 = Some(result.current_mp4_sha256);
    status.report_path = Some(result.report_path.display().to_string());
    status.report_sha256 = Some(result.report_sha256);
    status.report_version = Some(result.report_version);
    status.algorithm_version = Some(result.algorithm_version);
    status.random_seed = Some(result.random_seed);
    status.content_similarity_percent = Some(result.content_similarity_percent);
    status.media_robustness_score = Some(result.media_robustness_score);
    status.invisible_mark_status = Some(result.invisible_mark_status);
    status.random_perturbation_applied = Some(result.random_perturbation_applied);
    status.content_fingerprint = Some(result.content_fingerprint);
    status.error = None;
}

impl Default for ResearchStatusDto {
    fn default() -> Self {
        Self {
            state: "idle".to_owned(),
            analysis_id: None,
            source_mp4_sha256: None,
            input_mp4_sha256: None,
            current_mp4_sha256: None,
            report_path: None,
            report_sha256: None,
            report_version: None,
            algorithm_version: None,
            random_seed: None,
            content_similarity_percent: None,
            media_robustness_score: None,
            invisible_mark_status: None,
            random_perturbation_applied: None,
            content_fingerprint: None,
            error: None,
            cache_paths: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandErrorDto {
    pub code: String,
    pub message: String,
}

impl CommandErrorDto {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

fn writable_runtime_resource_layout(app: &AppHandle) -> Result<RuntimeResourceLayout, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("读取运行资源应用数据目录失败：{error}"))?;
    RuntimeResourceLayout::for_target(&app_data_dir, target_triple())
        .map_err(|error| format!("解析当前平台运行资源目录失败：{error}"))
}

fn runtime_resource_catalog_cache() -> &'static Mutex<Option<RuntimeResourceCatalog>> {
    static CATALOG: OnceLock<Mutex<Option<RuntimeResourceCatalog>>> = OnceLock::new();
    CATALOG.get_or_init(|| Mutex::new(None))
}

fn runtime_resource_catalog_blocking(
    app: &AppHandle,
) -> Result<RuntimeResourceCatalog, (String, PathBuf)> {
    let app_data_dir = app.path().app_data_dir().map_err(|error| {
        (
            format!("读取运行资源应用数据目录失败：{error}"),
            PathBuf::new(),
        )
    })?;
    let fallback_root = RuntimeResourceLayout::for_target(&app_data_dir, target_triple())
        .map_err(|error| {
            (
                format!("解析当前平台运行资源目录失败：{error}"),
                PathBuf::new(),
            )
        })?
        .version_root;
    let resource_dir = app.path().resource_dir().map_err(|error| {
        (
            format!("读取运行资源 manifest 所在资源目录失败：{error}"),
            fallback_root.clone(),
        )
    })?;
    let mut cached = runtime_resource_catalog_cache()
        .lock()
        .map_err(|_| ("运行资源目录缓存锁已损坏".to_owned(), fallback_root.clone()))?;
    if let Some(catalog) = cached.as_ref() {
        return Ok(catalog.clone());
    }
    let catalog = RuntimeResourceCatalog::from_resource_directory(
        &resource_dir,
        &app_data_dir,
        target_triple(),
    )
    .map_err(|error| (error.to_string(), fallback_root))?;
    *cached = Some(catalog.clone());
    Ok(catalog)
}

fn runtime_resource_roots(app: &AppHandle) -> Result<RuntimeResourceRoots, String> {
    runtime_resource_catalog_blocking(app)
        .map(|catalog| catalog.roots().clone())
        .map_err(|(error, _)| error)
}

fn runtime_resource_target_root(app: &AppHandle) -> Result<PathBuf, String> {
    let roots = runtime_resource_roots(app)?;
    Ok(roots.target_root)
}

fn runtime_resource_installer_blocking(
    app: &AppHandle,
) -> Result<RuntimeResourceInstaller, (String, PathBuf)> {
    let app_data_dir = app.path().app_data_dir().map_err(|error| {
        (
            format!("读取运行资源应用数据目录失败：{error}"),
            PathBuf::new(),
        )
    })?;
    let resource_root = RuntimeResourceLayout::for_target(&app_data_dir, target_triple())
        .map_err(|error| {
            (
                format!("解析当前平台运行资源目录失败：{error}"),
                PathBuf::new(),
            )
        })?
        .version_root;
    let resource_dir = app.path().resource_dir().map_err(|error| {
        (
            format!("读取运行资源 manifest 所在资源目录失败：{error}"),
            resource_root.clone(),
        )
    })?;
    RuntimeResourceInstaller::from_resource_directory(&resource_dir, &app_data_dir, target_triple())
        .map_err(|error| (error.to_string(), resource_root))
}

async fn runtime_resource_installer(
    component: RuntimeResourceComponent,
    app: AppHandle,
    task: Arc<RuntimeResourceTask>,
) -> Result<RuntimeResourceInstaller, RuntimeResourceStatus> {
    let loaded =
        tauri::async_runtime::spawn_blocking(move || runtime_resource_installer_blocking(&app))
            .await;
    match loaded {
        Ok(Ok(installer)) => Ok(installer),
        Ok(Err((error, resource_root))) => Err(record_runtime_resource_failure(
            &task,
            component,
            error,
            &resource_root,
        )),
        Err(error) => Err(record_runtime_resource_failure(
            &task,
            component,
            format!("运行资源 manifest 加载任务失败：{error}"),
            Path::new(""),
        )),
    }
}

async fn runtime_resource_catalog(
    component: RuntimeResourceComponent,
    app: AppHandle,
    task: Arc<RuntimeResourceTask>,
) -> Result<RuntimeResourceCatalog, RuntimeResourceStatus> {
    let loaded =
        tauri::async_runtime::spawn_blocking(move || runtime_resource_catalog_blocking(&app)).await;
    match loaded {
        Ok(Ok(catalog)) => Ok(catalog),
        Ok(Err((error, resource_root))) => Err(record_runtime_resource_failure(
            &task,
            component,
            error,
            &resource_root,
        )),
        Err(error) => Err(record_runtime_resource_failure(
            &task,
            component,
            format!("运行资源目录解析任务失败：{error}"),
            Path::new(""),
        )),
    }
}

fn bundled_clear_status(catalog: &RuntimeResourceCatalog) -> Option<RuntimeResourceStatus> {
    let mut status = catalog.bundled_status(RuntimeResourceComponent::Media)?;
    status.component = None;
    Some(status)
}

fn record_runtime_resource_failure(
    task: &RuntimeResourceTask,
    component: RuntimeResourceComponent,
    error: String,
    resource_root: &Path,
) -> RuntimeResourceStatus {
    task.record_failure(component, error.clone(), resource_root)
        .unwrap_or_else(|state_error| RuntimeResourceStatus {
            state: RuntimeResourceState::Failed,
            component: Some(component),
            current_file: None,
            downloaded_bytes: 0,
            total_bytes: 0,
            bytes_per_second: 0,
            installed_bytes: 0,
            resource_root: resource_root.display().to_string(),
            error: Some(format!("{error}；记录运行资源失败状态失败：{state_error}")),
        })
}

fn development_runtime_resource_status(
    component: RuntimeResourceComponent,
) -> Option<RuntimeResourceStatus> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let configured_file = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .is_some_and(|path| development_executable_ready(&path))
    };
    let ready = match component {
        RuntimeResourceComponent::Media => {
            configured_file(FFMPEG_PATH_ENV) && configured_file(FFPROBE_PATH_ENV)
        }
    };
    ready.then(|| RuntimeResourceStatus {
        state: RuntimeResourceState::Ready,
        component: Some(component),
        current_file: None,
        downloaded_bytes: 0,
        total_bytes: 0,
        bytes_per_second: 0,
        installed_bytes: 0,
        resource_root: "development-overrides".to_owned(),
        error: None,
    })
}

fn development_executable_ready(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}

#[tauri::command]
pub async fn get_runtime_resource_status(
    component: RuntimeResourceComponent,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RuntimeResourceStatus, String> {
    if let Some(status) = state.runtime_resource_task.running_status()? {
        return Ok(status);
    }
    if let Some(status) = development_runtime_resource_status(component) {
        return Ok(status);
    }
    let task = Arc::clone(&state.runtime_resource_task);
    let catalog = match runtime_resource_catalog(component, app.clone(), Arc::clone(&task)).await {
        Ok(catalog) => catalog,
        Err(status) => return Ok(status),
    };
    if let Some(status) = catalog.bundled_status(component) {
        return Ok(status);
    }
    let installer = match runtime_resource_installer(component, app, Arc::clone(&task)).await {
        Ok(installer) => installer,
        Err(status) => return Ok(status),
    };
    let resource_root = installer.resource_root().to_path_buf();
    match tauri::async_runtime::spawn_blocking(move || {
        task.inspect_when_idle(component, &installer)
    })
    .await
    {
        Ok(Ok(status)) => Ok(status),
        Ok(Err(error)) => Ok(record_runtime_resource_failure(
            &state.runtime_resource_task,
            component,
            error,
            &resource_root,
        )),
        Err(error) => Ok(record_runtime_resource_failure(
            &state.runtime_resource_task,
            component,
            format!("运行资源状态检查任务失败：{error}"),
            &resource_root,
        )),
    }
}

#[tauri::command]
pub async fn install_runtime_resources(
    component: RuntimeResourceComponent,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RuntimeResourceStatus, String> {
    if let Some(status) = state.runtime_resource_task.running_status()? {
        return Ok(status);
    }
    let catalog = match runtime_resource_catalog(
        component,
        app.clone(),
        Arc::clone(&state.runtime_resource_task),
    )
    .await
    {
        Ok(catalog) => catalog,
        Err(status) => return Ok(status),
    };
    if let Some(status) = catalog.bundled_status(component) {
        return Ok(status);
    }
    let installer =
        match runtime_resource_installer(component, app, Arc::clone(&state.runtime_resource_task))
            .await
        {
            Ok(installer) => installer,
            Err(status) => return Ok(status),
        };
    state
        .runtime_resource_task
        .start_install(component, installer)
}

#[tauri::command]
pub async fn cancel_runtime_resource_install(
    state: State<'_, AppState>,
) -> Result<RuntimeResourceStatus, String> {
    state.runtime_resource_task.cancel()
}

#[tauri::command]
pub async fn import_runtime_resource_directory(
    component: RuntimeResourceComponent,
    source_root: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RuntimeResourceStatus, String> {
    if let Some(status) = state.runtime_resource_task.running_status()? {
        return Ok(status);
    }
    let catalog = match runtime_resource_catalog(
        component,
        app.clone(),
        Arc::clone(&state.runtime_resource_task),
    )
    .await
    {
        Ok(catalog) => catalog,
        Err(status) => return Ok(status),
    };
    if let Some(status) = catalog.bundled_status(component) {
        return Ok(status);
    }
    let installer =
        match runtime_resource_installer(component, app, Arc::clone(&state.runtime_resource_task))
            .await
        {
            Ok(installer) => installer,
            Err(status) => return Ok(status),
        };
    state
        .runtime_resource_task
        .start_import(component, installer, PathBuf::from(source_root))
}

#[tauri::command]
pub async fn clear_runtime_resources(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RuntimeResourceStatus, String> {
    if let Some(status) = state.runtime_resource_task.running_status()? {
        return Ok(status);
    }
    let component = RuntimeResourceComponent::Media;
    let catalog = match runtime_resource_catalog(
        component,
        app.clone(),
        Arc::clone(&state.runtime_resource_task),
    )
    .await
    {
        Ok(catalog) => catalog,
        Err(status) => return Ok(status),
    };
    let writable_root = writable_runtime_resource_layout(&app)?.version_root;
    if !writable_root.exists() {
        return Ok(
            bundled_clear_status(&catalog).unwrap_or(RuntimeResourceStatus {
                state: RuntimeResourceState::NotInstalled,
                component: None,
                current_file: None,
                downloaded_bytes: 0,
                total_bytes: 0,
                bytes_per_second: 0,
                installed_bytes: 0,
                resource_root: writable_root.display().to_string(),
                error: None,
            }),
        );
    }
    let installer =
        match runtime_resource_installer(component, app, Arc::clone(&state.runtime_resource_task))
            .await
        {
            Ok(installer) => installer,
            Err(status) => return Ok(status),
        };
    state.runtime_resource_task.start_clear(installer)
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            main_window_label: "main",
            playback: Arc::new(Mutex::new(PlaybackCore::default())),
            speech_worker: Arc::new(Mutex::new(None)),
            media_worker: Arc::new(Mutex::new(None)),
            research_worker: Arc::new(Mutex::new(None)),
            research_status: Arc::new(Mutex::new(ResearchStatusDto::default())),
            runtime_resource_task: Arc::new(RuntimeResourceTask::default()),
        }
    }
}

impl AppState {
    pub fn shutdown_runtime_resources(
        &self,
        budget: Duration,
    ) -> Result<RuntimeResourceTaskShutdown, String> {
        self.runtime_resource_task.shutdown(budget)
    }

    fn reap_finished_speech_worker(&self) -> Result<bool, CommandErrorDto> {
        let task = {
            let mut worker = self.speech_worker.lock().map_err(|_| {
                CommandErrorDto::new("speech_worker_lock_failed", "Worker 状态锁已损坏")
            })?;
            if worker
                .as_ref()
                .is_some_and(|task| task.completed.load(Ordering::Acquire))
            {
                worker.take()
            } else {
                None
            }
        };
        if let Some(task) = task {
            let _join_result = task.handle.join();
            return Ok(true);
        }
        Ok(false)
    }

    fn stop_speech_worker(&self) -> Result<(), CommandErrorDto> {
        let task = self
            .speech_worker
            .lock()
            .map_err(|_| CommandErrorDto::new("speech_worker_lock_failed", "Worker 状态锁已损坏"))?
            .take();
        if let Some(task) = task {
            task.cancellation.cancel();
            let _join_result = task.handle.join();
        }
        Ok(())
    }

    fn stop_media_worker(&self) -> Result<(), CommandErrorDto> {
        let task = self
            .media_worker
            .lock()
            .map_err(|_| {
                CommandErrorDto::new("media_worker_lock_failed", "媒体 Worker 状态锁已损坏")
            })?
            .take();
        if let Some(task) = task {
            task.cancellation.cancel();
            let _join_result = task.handle.join();
            // 取消后必须清掉 processing，否则 UI 会一直卡在「处理中」。
            if let Ok(mut playback) = self.playback.lock() {
                let snapshot = playback.snapshot();
                if snapshot.audio_processing_status == "processing"
                    || snapshot.video_processing_status == "processing"
                {
                    playback.mark_media_processing_failed("媒体处理已取消，等待应用最新参数");
                }
            }
        }
        Ok(())
    }

    fn stop_research_worker(&self) -> Result<(), CommandErrorDto> {
        let task = self
            .research_worker
            .lock()
            .map_err(|_| {
                CommandErrorDto::new("research_worker_lock_failed", "研究 Worker 状态锁已损坏")
            })?
            .take();
        if let Some(task) = task {
            task.cancellation.cancel();
            let _join_result = task.handle.join();
            let _ = self.update_research_status(|status| {
                if status.state == "running" {
                    status.state = "cancelled".to_owned();
                    status.error = Some("研究分析 Worker 已停止".to_owned());
                }
            });
        }
        Ok(())
    }

    fn research_worker_is_running(&self) -> Result<bool, CommandErrorDto> {
        let worker = self.research_worker.lock().map_err(|_| {
            CommandErrorDto::new("research_worker_lock_failed", "研究 Worker 状态锁已损坏")
        })?;
        Ok(worker.is_some())
    }

    fn reap_finished_research_worker(&self) -> Result<bool, CommandErrorDto> {
        let task = {
            let mut worker = self.research_worker.lock().map_err(|_| {
                CommandErrorDto::new("research_worker_lock_failed", "研究 Worker 状态锁已损坏")
            })?;
            if worker
                .as_ref()
                .is_some_and(|task| task.completed.load(Ordering::Acquire))
            {
                worker.take()
            } else {
                None
            }
        };
        if let Some(task) = task {
            let _join_result = task.handle.join();
            return Ok(true);
        }
        Ok(false)
    }

    fn install_research_worker(&self, task: ResearchWorkerTask) -> Result<(), ResearchWorkerTask> {
        let mut worker = match self.research_worker.lock() {
            Ok(worker) => worker,
            Err(_) => return Err(task),
        };
        if worker.is_some() {
            return Err(task);
        }
        worker.replace(task);
        Ok(())
    }

    fn read_research_status(&self) -> Result<ResearchStatusDto, CommandErrorDto> {
        self.research_status
            .lock()
            .map_err(|_| CommandErrorDto::new("research_status_lock_failed", "研究状态锁已损坏"))
            .map(|status| status.clone())
    }

    fn update_research_status(
        &self,
        update: impl FnOnce(&mut ResearchStatusDto),
    ) -> Result<ResearchStatusDto, CommandErrorDto> {
        let mut status = self
            .research_status
            .lock()
            .map_err(|_| CommandErrorDto::new("research_status_lock_failed", "研究状态锁已损坏"))?;
        update(&mut status);
        Ok(status.clone())
    }

    fn media_worker_is_running(&self) -> Result<bool, CommandErrorDto> {
        let worker = self.media_worker.lock().map_err(|_| {
            CommandErrorDto::new("media_worker_lock_failed", "媒体 Worker 状态锁已损坏")
        })?;
        Ok(worker.is_some())
    }

    fn reap_finished_media_worker(&self) -> Result<bool, CommandErrorDto> {
        let task = {
            let mut worker = self.media_worker.lock().map_err(|_| {
                CommandErrorDto::new("media_worker_lock_failed", "媒体 Worker 状态锁已损坏")
            })?;
            if worker
                .as_ref()
                .is_some_and(|task| task.completed.load(Ordering::Acquire))
            {
                worker.take()
            } else {
                None
            }
        };
        if let Some(task) = task {
            let _join_result = task.handle.join();
            return Ok(true);
        }
        Ok(false)
    }

    fn install_speech_worker(&self, task: SpeechWorkerTask) -> Result<(), SpeechWorkerTask> {
        let mut worker = match self.speech_worker.lock() {
            Ok(worker) => worker,
            Err(_) => return Err(task),
        };
        if worker.is_some() {
            return Err(task);
        }
        worker.replace(task);
        Ok(())
    }

    fn install_media_worker(&self, task: MediaWorkerTask) -> Result<(), MediaWorkerTask> {
        let mut worker = match self.media_worker.lock() {
            Ok(worker) => worker,
            Err(_) => return Err(task),
        };
        if worker.is_some() {
            return Err(task);
        }
        worker.replace(task);
        Ok(())
    }

    fn ensure_main_window(&self, window: &Window) -> Result<(), CommandErrorDto> {
        if window.label() == self.main_window_label {
            Ok(())
        } else {
            Err(CommandErrorDto::new(
                "main_window_only",
                "当前命令只允许主窗口调用",
            ))
        }
    }

    fn ensure_playback_window(&self, window: &Window) -> Result<(), CommandErrorDto> {
        if matches!(window.label(), "main" | "final-effect") {
            Ok(())
        } else {
            Err(CommandErrorDto::new(
                "playback_window_only",
                "当前命令只允许主窗口或最终效果窗口调用",
            ))
        }
    }

    fn with_playback<T>(
        &self,
        window: &Window,
        handler: impl FnOnce(&mut PlaybackCore) -> Result<T, CommandErrorDto>,
    ) -> Result<T, CommandErrorDto> {
        self.ensure_main_window(window)?;
        let mut playback = self
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        playback
            .bind_window(window.label())
            .map_err(command_error_from_playback)?;
        handler(&mut playback)
    }

    fn with_playback_window<T>(
        &self,
        window: &Window,
        handler: impl FnOnce(&mut PlaybackCore) -> Result<T, CommandErrorDto>,
    ) -> Result<T, CommandErrorDto> {
        self.ensure_playback_window(window)?;
        let mut playback = self
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        if window.label() == self.main_window_label {
            playback
                .bind_window(window.label())
                .map_err(command_error_from_playback)?;
        }
        handler(&mut playback)
    }

    fn snapshot(&self, playback: &PlaybackCore) -> PlaybackSnapshotDto {
        PlaybackSnapshotDto::from(playback.snapshot())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlaybackSnapshotDto {
    pub window_id: Option<String>,
    pub playback_generation: u64,
    pub playback_state: String,
    pub source_media: Option<SourceMediaDto>,
    pub loop_index: u64,
    pub current_position_ms: u64,
    pub current_video_source: Option<String>,
    pub current_video_reference: Option<String>,
    pub current_video_sha256: Option<String>,
    pub pending_video_reference: Option<String>,
    pub pending_video_sha256: Option<String>,
    pub video_processing_enabled: bool,
    pub video_processing_status: String,
    pub audio_processing_enabled: bool,
    pub realtime_audio_variant_enabled: bool,
    pub current_audio_source: Option<String>,
    pub current_audio_reference: Option<String>,
    pub current_audio_start_at_ms: u64,
    pub current_mp4_sha256: Option<String>,
    pub current_audio_sha256: Option<String>,
    pub audio_decision: String,
    pub worker_status: String,
    pub fallback_reason: Option<String>,
    pub pending_audio_candidate: bool,
    pub pending_audio_reference: Option<String>,
    pub pending_audio_start_at_ms: Option<u64>,
    pub pending_audio_duration_ms: Option<u64>,
    pub audio_processing_parameters_version: String,
    pub audio_processing_status: String,
    pub audio_processing_runtime: bool,
    pub audio_processing_gain_db: f64,
    pub effective_audio_source: String,
    pub interlude: InterludeSnapshotDto,
}

pub type InterludeSnapshotDto = InterludeSnapshot;

impl From<PlaybackSnapshot> for PlaybackSnapshotDto {
    fn from(value: PlaybackSnapshot) -> Self {
        Self {
            window_id: value.window_id,
            playback_generation: value.playback_generation,
            playback_state: format!("{:?}", value.playback_state).to_ascii_lowercase(),
            source_media: value.source_media,
            loop_index: value.loop_index,
            current_position_ms: value.current_position_ms,
            current_video_source: value.current_video_source,
            current_video_reference: value.current_video_reference,
            current_video_sha256: value.current_video_sha256,
            pending_video_reference: value.pending_video_reference,
            pending_video_sha256: value.pending_video_sha256,
            video_processing_enabled: value.video_processing_enabled,
            video_processing_status: value.video_processing_status,
            audio_processing_enabled: value.audio_processing_enabled,
            realtime_audio_variant_enabled: value.realtime_audio_variant_enabled,
            current_audio_source: value.current_audio_source,
            current_audio_reference: value.current_audio_reference,
            current_audio_start_at_ms: value.current_audio_start_at_ms,
            current_mp4_sha256: value.current_mp4_sha256,
            current_audio_sha256: value.current_audio_sha256,
            audio_decision: value.audio_decision,
            worker_status: value.worker_status,
            fallback_reason: value.fallback_reason,
            pending_audio_candidate: value.pending_audio_candidate,
            pending_audio_reference: value.pending_audio_reference,
            pending_audio_start_at_ms: value.pending_audio_start_at_ms,
            pending_audio_duration_ms: value.pending_audio_duration_ms,
            audio_processing_parameters_version: value.audio_processing_parameters_version,
            audio_processing_status: value.audio_processing_status,
            audio_processing_runtime: value.audio_processing_runtime,
            audio_processing_gain_db: value.audio_processing_gain_db,
            effective_audio_source: value.effective_audio_source,
            interlude: value.interlude,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProcessingSwitchesRequestDto {
    pub video_processing_enabled: bool,
    pub audio_processing_enabled: bool,
    pub realtime_audio_variant_enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AudioProcessingProfileRequestDto {
    pub profile: AudioProcessingProfile,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetInterludeConfigRequestDto {
    pub enabled: bool,
    pub directory: Option<String>,
    pub interval_min_ms: u64,
    pub interval_max_ms: u64,
    pub volume_db: f64,
    pub ducking_depth_db: f64,
    pub ducking_attack_ms: u64,
    pub ducking_release_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceRuntimeInfoRequestDto {
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceRuntimeInfoDto {
    pub disk_free_bytes: u64,
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub cpu_logical_cores: u32,
    pub os_name: Option<String>,
    pub os_version: Option<String>,
    pub kernel_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MediaParameterValidationResultDto {
    pub valid: bool,
    pub errors: Vec<ParameterValidationError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartMediaProcessingRequestDto {
    pub params: LocalResearchParams,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartResearchAnalysisRequestDto {
    pub analysis_id: Option<String>,
    pub run_id: Option<String>,
    pub params: LocalResearchParams,
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub generate_output_mp4: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DirectModelChatRequestDto {
    pub lease_id: String,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub proxy_mode: String,
    pub direct_base_url: String,
    pub expires_at_unix_ms: u64,
    pub direct_access_token: Option<String>,
    pub credential_expires_at_unix_ms: Option<u64>,
    pub messages: Vec<DirectChatMessage>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirectModelChatResponseDto {
    pub text: String,
    pub model: String,
    pub finish_reason: Option<String>,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    pub latency_ms: u64,
}

fn prune_cache_dir(
    directory: &Path,
    max_bytes: u64,
    max_files: Option<usize>,
    protected_paths: &[PathBuf],
) -> std::io::Result<CacheCleanupResultDto> {
    if !directory.is_dir() {
        return Ok(CacheCleanupResultDto {
            removed_files: 0,
            removed_bytes: 0,
            remaining_bytes: 0,
        });
    }
    let protected = protected_paths
        .iter()
        .map(|path| path.to_path_buf())
        .collect::<std::collections::HashSet<_>>();
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            continue;
        }
        entries.push((
            path,
            metadata.len(),
            metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        ));
    }
    let mut remaining_bytes = entries.iter().map(|(_, size, _)| *size).sum::<u64>();
    let mut removed_files: u32 = 0;
    let mut removed_bytes: u64 = 0;
    let partial_expiry = SystemTime::now()
        .checked_sub(Duration::from_secs(10 * 60))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    // 新→旧，便于按“最近 N 个”保留。
    entries.sort_by(|a, b| b.2.cmp(&a.2));
    let mut kept_unprotected: usize = 0;
    for (path, size, modified) in entries {
        let is_partial = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains(".partial"));
        let is_protected = protected.contains(&path);
        let stale_partial = is_partial && modified <= partial_expiry;
        let over_file_limit = max_files.is_some_and(|limit| {
            !is_protected && !is_partial && {
                // 非保护成品：超过最近 N 个就删。
                let over = kept_unprotected >= limit;
                if !over {
                    kept_unprotected += 1;
                }
                over
            }
        });
        // 未完成 partial 不计入 N；仅过期 partial 删。保护文件永不因额度删。
        let over_budget = remaining_bytes > max_bytes && !is_protected;
        let should_remove = stale_partial || over_file_limit || over_budget;
        if is_protected && !stale_partial {
            continue;
        }
        if !should_remove {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            remaining_bytes = remaining_bytes.saturating_sub(size);
            removed_files += 1;
            removed_bytes = removed_bytes.saturating_add(size);
        }
    }
    Ok(CacheCleanupResultDto {
        removed_files,
        removed_bytes,
        remaining_bytes,
    })
}

#[cfg(test)]
mod cache_tests {
    use super::prune_cache_dir;
    use std::fs;

    #[test]
    fn cache_prune_keeps_protected_files_and_removes_old_partials() {
        let directory = std::env::temp_dir().join(format!(
            "autolive-cache-contract-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("cache directory should be created");
        let protected = directory.join("protected.mp4");
        let stale_partial = directory.join("old.partial.mp4");
        fs::write(&protected, b"protected").expect("protected file should be written");
        fs::write(&stale_partial, b"partial").expect("partial file should be written");
        let result = prune_cache_dir(&directory, 1, None, std::slice::from_ref(&protected))
            .expect("cache prune should succeed");

        assert!(protected.exists());
        assert!(!stale_partial.exists());
        assert!(result.remaining_bytes >= fs::metadata(&protected).unwrap().len());
        let _ignored = fs::remove_dir_all(directory);
    }

    #[test]
    fn cache_prune_keeps_only_recent_media_files() {
        let directory = std::env::temp_dir().join(format!(
            "autolive-cache-keep-n-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("cache directory should be created");
        let older = directory.join("processed-1.mp4");
        let mid = directory.join("processed-2.mp4");
        let newest = directory.join("processed-3.mp4");
        let fourth = directory.join("processed-4.mp4");
        fs::write(&older, b"1").expect("write");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&mid, b"2").expect("write");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&newest, b"3").expect("write");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&fourth, b"4").expect("write");

        let result = prune_cache_dir(&directory, u64::MAX, Some(3), &[])
            .expect("cache prune should succeed");

        assert!(!older.exists(), "oldest should be pruned");
        assert!(mid.exists());
        assert!(newest.exists());
        assert!(fourth.exists());
        assert_eq!(result.removed_files, 1);
        let _ignored = fs::remove_dir_all(directory);
    }
}

fn cleanup_local_caches(
    app: &AppHandle,
    state: &AppState,
) -> Result<CacheCleanupResultDto, CommandErrorDto> {
    let cache_root = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandErrorDto::new("cache_dir_failed", error.to_string()))?;
    let (current_video, pending_video, research_paths) = {
        let playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        let snapshot = playback.snapshot();
        let research_paths = state
            .research_status
            .lock()
            .map_err(|_| CommandErrorDto::new("research_status_lock_failed", "研究状态锁已损坏"))?
            .cache_paths
            .clone();
        (
            snapshot.current_video_reference.map(PathBuf::from),
            snapshot.pending_video_reference.map(PathBuf::from),
            research_paths,
        )
    };
    let media_result = prune_cache_dir(
        &cache_root.join("media-processing"),
        MEDIA_CACHE_MAX_BYTES,
        Some(MEDIA_CACHE_MAX_FILES),
        &[current_video, pending_video]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
    )
    .map_err(|error| CommandErrorDto::new("media_cache_cleanup_failed", error.to_string()))?;
    let research_result = prune_cache_dir(
        &cache_root.join("research-analysis"),
        RESEARCH_CACHE_MAX_BYTES,
        None,
        &research_paths,
    )
    .map_err(|error| CommandErrorDto::new("research_cache_cleanup_failed", error.to_string()))?;
    Ok(CacheCleanupResultDto {
        removed_files: media_result.removed_files + research_result.removed_files,
        removed_bytes: media_result.removed_bytes + research_result.removed_bytes,
        remaining_bytes: media_result.remaining_bytes + research_result.remaining_bytes,
    })
}

impl AppState {
    fn playback_action(
        &self,
        window: &Window,
        action: impl FnOnce(&mut PlaybackCore) -> Result<(), PlaybackError>,
    ) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
        self.ensure_playback_window(window)?;
        let mut playback = self
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        if window.label() == self.main_window_label {
            playback
                .bind_window(window.label())
                .map_err(command_error_from_playback)?;
        }
        action(&mut playback).map_err(command_error_from_playback)?;
        Ok(self.snapshot(&playback))
    }
}

#[tauri::command]
pub async fn probe_local_video(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: MediaProbeRequestDto,
) -> Result<MediaProbeResultDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        probe_local_video_blocking(window, app, state, request)
    })
    .await
    .map_err(|error| {
        CommandErrorDto::new("media_probe_failed", format!("视频导入任务失败：{error}"))
    })?
}

#[tauri::command]
pub async fn probe_local_mp4(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: MediaProbeRequestDto,
) -> Result<MediaProbeResultDto, CommandErrorDto> {
    probe_local_video(window, app, state, request).await
}

fn probe_local_video_blocking(
    window: Window,
    app: AppHandle,
    state: AppState,
    request: MediaProbeRequestDto,
) -> Result<MediaProbeResultDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let target_root = runtime_resource_target_root(&app).map_err(|error| {
        CommandErrorDto::new(
            "media_probe_failed",
            format!("读取已验证媒体运行资源目录失败：{error}"),
        )
    })?;
    let (_, ffprobe_path) = configured_media_engine_paths_with_resource_dir(&target_root)
        .map_err(|error| CommandErrorDto::new("media_probe_failed", error.to_string()))?;
    let cancellation = CancellationToken::new();
    let result = probe_user_selected_video_with_ffprobe(
        &request,
        &ffprobe_path,
        MEDIA_IMPORT_PROBE_TIMEOUT_MS,
        &cancellation,
    )
    .map_err(command_error_from_media_library)?;
    allow_local_playback_asset_file(
        &app,
        Path::new(&result.canonical_path),
        "source_media_asset_scope_failed",
        "源视频",
    )?;

    state.stop_speech_worker()?;
    state.stop_media_worker()?;
    state.stop_research_worker()?;
    state.with_playback(&window, |playback| {
        playback.set_source(result.source.clone());
        Ok(())
    })?;
    Ok(result)
}

#[tauri::command]
pub fn get_device_runtime_info(
    window: Window,
    state: State<'_, AppState>,
    request: DeviceRuntimeInfoRequestDto,
) -> Result<DeviceRuntimeInfoDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let disks = Disks::new_with_refreshed_list();
    let selected_path = request.path.as_deref().map(Path::new);
    let disk = selected_path
        .and_then(|path| {
            disks
                .list()
                .iter()
                .filter(|disk| path.starts_with(disk.mount_point()))
                .max_by_key(|disk| disk.mount_point().as_os_str().len())
        })
        .or_else(|| disks.list().first())
        .ok_or_else(|| CommandErrorDto::new("disk_info_unavailable", "无法读取磁盘信息"))?;
    let mut system = System::new();
    system.refresh_memory();
    Ok(DeviceRuntimeInfoDto {
        disk_free_bytes: disk.available_space(),
        memory_total_bytes: system.total_memory(),
        memory_available_bytes: system.available_memory(),
        cpu_logical_cores: std::thread::available_parallelism()
            .map(|value| value.get() as u32)
            .unwrap_or(0),
        os_name: System::name(),
        os_version: System::os_version(),
        kernel_version: System::kernel_version(),
    })
}

#[tauri::command]
pub fn get_speech_to_speech_worker_capabilities(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SpeechToSpeechWorkerCapabilities, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    match runtime_resource_target_root(&app) {
        Ok(target_root) => {
            Ok(configured_speech_to_speech_worker_capabilities_with_resource_dir(&target_root))
        }
        Err(error) => {
            let fallback = configured_speech_to_speech_worker_capabilities();
            if fallback.available {
                return Ok(fallback);
            }
            Ok(SpeechToSpeechWorkerCapabilities::unavailable_with_reason(
                format!(
                    "运行资源目录不可用：{error}；{}",
                    fallback
                        .reason
                        .unwrap_or_else(|| "speech-to-speech Worker 不可用".to_owned())
                ),
            ))
        }
    }
}

#[tauri::command]
pub fn get_media_engine_capabilities(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MediaEngineStatus, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let target_root = runtime_resource_target_root(&app)
        .map_err(|error| CommandErrorDto::new("media_resource_dir_failed", error))?;
    Ok(configured_media_engine_status_with_resource_dir(
        &target_root,
    ))
}

#[tauri::command]
pub fn get_research_worker_capabilities(
    window: Window,
    state: State<'_, AppState>,
) -> Result<ResearchWorkerCapabilities, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    Ok(configured_research_worker_capabilities())
}

#[tauri::command]
pub fn get_research_status(
    window: Window,
    state: State<'_, AppState>,
) -> Result<ResearchStatusDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    state.read_research_status()
}

#[tauri::command]
pub fn cleanup_local_caches_command(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CacheCleanupResultDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    cleanup_local_caches(&app, &state)
}

#[tauri::command]
pub async fn start_research_analysis(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: StartResearchAnalysisRequestDto,
) -> Result<ResearchStatusDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        start_research_analysis_blocking(window, app, state, request)
    })
    .await
    .map_err(|error| {
        CommandErrorDto::new(
            "research_analysis_failed",
            format!("研究分析任务启动失败：{error}"),
        )
    })?
}

fn start_research_analysis_blocking(
    window: Window,
    app: AppHandle,
    state: AppState,
    request: StartResearchAnalysisRequestDto,
) -> Result<ResearchStatusDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let _ = state.reap_finished_research_worker()?;
    if state.research_worker_is_running()? {
        return Err(CommandErrorDto::new(
            "research_worker_already_running",
            "当前已有研究分析 Worker 在执行",
        ));
    }
    if let Err(errors) = request.params.validate() {
        return Err(CommandErrorDto::new(
            "research_params_invalid",
            errors
                .iter()
                .map(|error| format!("{}: {}", error.field, error.message))
                .collect::<Vec<_>>()
                .join("；"),
        ));
    }
    let _ = cleanup_local_caches(&app, &state)?;
    let (input_mp4_path, source_mp4_path) = {
        let playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        let snapshot = playback.snapshot();
        let source = snapshot
            .source_media
            .as_ref()
            .ok_or_else(|| CommandErrorDto::new("source_media_required", "请先导入一个源视频"))?;
        let input_path = snapshot
            .current_video_reference
            .clone()
            .unwrap_or_else(|| source.source_path.clone());
        (
            PathBuf::from(input_path),
            PathBuf::from(&source.source_path),
        )
    };
    let input_mp4_sha256 =
        hash_file_at_path(&input_mp4_path, &CancellationToken::new()).map_err(|error| {
            CommandErrorDto::new(
                "current_hash_required",
                format!("当前视频完整 SHA-256 计算失败：{error}"),
            )
        })?;
    let source_mp4_sha256 = if input_mp4_path == source_mp4_path {
        input_mp4_sha256.clone()
    } else {
        hash_file_at_path(&source_mp4_path, &CancellationToken::new()).map_err(|error| {
            CommandErrorDto::new(
                "source_hash_required",
                format!("源视频完整 SHA-256 计算失败：{error}"),
            )
        })?
    };
    let executable = configured_research_worker_executable().map_err(|error| {
        let message = error.to_string();
        let _ = state.update_research_status(|status| {
            status.state = "unavailable".to_owned();
            status.error = Some(message.clone());
        });
        CommandErrorDto::new("research_worker_unavailable", message)
    })?;
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandErrorDto::new("research_cache_dir_failed", error.to_string()))?
        .join("research-analysis");
    std::fs::create_dir_all(&cache_dir)
        .map_err(|error| CommandErrorDto::new("research_cache_dir_failed", error.to_string()))?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| CommandErrorDto::new("research_cache_nonce_failed", error.to_string()))?
        .as_nanos();
    let analysis_id = request
        .analysis_id
        .unwrap_or_else(|| format!("analysis_{nonce}"));
    let run_id = request.run_id.unwrap_or_else(|| format!("run_{nonce}"));
    validate_research_identifier("analysis_id", &analysis_id)
        .map_err(|error| CommandErrorDto::new("research_identifier_invalid", error.to_string()))?;
    validate_research_identifier("run_id", &run_id)
        .map_err(|error| CommandErrorDto::new("research_identifier_invalid", error.to_string()))?;
    let params_path = cache_dir.join(format!("{analysis_id}-{run_id}.params.json"));
    let report_path = cache_dir.join(format!("{analysis_id}-{run_id}.report.json"));
    let output_mp4_path = request
        .generate_output_mp4
        .then(|| cache_dir.join(format!("{analysis_id}-{run_id}.output.mp4")));
    let params_bytes = serde_json::to_vec_pretty(&request.params).map_err(|error| {
        CommandErrorDto::new("research_params_serialize_failed", error.to_string())
    })?;
    let params_partial = params_path.with_extension("json.partial");
    let mut params_file = std::fs::File::create(&params_partial)
        .map_err(|error| CommandErrorDto::new("research_params_write_failed", error.to_string()))?;
    params_file
        .write_all(&params_bytes)
        .and_then(|_| params_file.sync_all())
        .map_err(|error| CommandErrorDto::new("research_params_write_failed", error.to_string()))?;
    std::fs::rename(&params_partial, &params_path).map_err(|error| {
        CommandErrorDto::new("research_params_commit_failed", error.to_string())
    })?;
    let timeout_seconds = request.timeout_seconds.unwrap_or(6 * 60 * 60);
    let analysis_request = ResearchAnalysisRequest {
        analysis_id: analysis_id.clone(),
        run_id: run_id.clone(),
        input_mp4_path,
        expected_input_mp4_sha256: input_mp4_sha256.clone(),
        source_mp4_sha256: source_mp4_sha256.clone(),
        params_path: params_path.clone(),
        research_executable: executable,
        output_report_path: report_path.clone(),
        output_mp4_path: output_mp4_path.clone(),
        timeout_seconds,
    };
    let initial = state.update_research_status(|status| {
        *status = ResearchStatusDto::default();
        status.state = "running".to_owned();
        status.analysis_id = Some(analysis_id.clone());
        status.source_mp4_sha256 = Some(source_mp4_sha256.clone());
        status.input_mp4_sha256 = Some(input_mp4_sha256.clone());
        status.report_path = Some(report_path.display().to_string());
        status.cache_paths = [
            params_path.clone(),
            report_path.clone(),
            output_mp4_path.clone().unwrap_or_default(),
        ]
        .into_iter()
        .filter(|path| !path.as_os_str().is_empty())
        .collect();
    })?;
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let completed = Arc::new(AtomicBool::new(false));
    let completed_for_thread = Arc::clone(&completed);
    let status = Arc::clone(&state.research_status);
    let thread_analysis_id = analysis_id.clone();
    let handle = thread::spawn(move || {
        let result = run_research(&analysis_request, &worker_cancellation);
        if let Ok(mut status) = status.lock() {
            match result {
                Ok(result) => apply_research_result(&mut status, thread_analysis_id, result),
                Err(error) => {
                    status.state = if matches!(
                        error,
                        autolive_desktop_core::research_worker::ResearchError::Cancelled
                    ) {
                        "cancelled".to_owned()
                    } else {
                        "failed".to_owned()
                    };
                    status.error = Some(error.to_string());
                    status.cache_paths.clear();
                }
            }
        }
        completed_for_thread.store(true, Ordering::Release);
    });
    if let Err(task) = state.install_research_worker(ResearchWorkerTask {
        cancellation,
        completed,
        handle,
    }) {
        task.cancellation.cancel();
        let _join_result = task.handle.join();
        let _ = state.update_research_status(|status| {
            status.state = "failed".to_owned();
            status.error = Some("当前已有研究分析 Worker 在执行".to_owned());
        });
        return Err(CommandErrorDto::new(
            "research_worker_already_running",
            "当前已有研究分析 Worker 在执行",
        ));
    }
    Ok(initial)
}

#[tauri::command]
pub fn cancel_research_analysis(
    window: Window,
    state: State<'_, AppState>,
) -> Result<ResearchStatusDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    state.stop_research_worker()?;
    state.update_research_status(|status| {
        if status.state == "running" {
            status.state = "cancelled".to_owned();
            status.error = Some("用户取消研究分析 Worker".to_owned());
        }
    })
}

#[tauri::command]
pub fn start_media_processing(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: StartMediaProcessingRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let _ = state.reap_finished_media_worker()?;
    if state.media_worker_is_running()? {
        return Err(CommandErrorDto::new(
            "media_worker_already_running",
            "当前已有本地媒体处理 Worker 在执行",
        ));
    }
    let validation = request.params.validate();
    if let Err(errors) = validation {
        return Err(CommandErrorDto::new(
            "media_processing_params_invalid",
            errors
                .iter()
                .map(|error| format!("{}: {}", error.field, error.message))
                .collect::<Vec<_>>()
                .join("；"),
        ));
    }
    let (source_path, generation, video_enabled, audio_enabled, source_audio_sample_rate_hz) = {
        let playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        let snapshot = playback.snapshot();
        let source_path = snapshot
            .source_media
            .as_ref()
            .map(|source| source.source_path.clone())
            .ok_or_else(|| CommandErrorDto::new("source_media_required", "请先导入一个源视频"))?;
        (
            source_path,
            snapshot.playback_generation,
            snapshot.video_processing_enabled,
            snapshot.audio_processing_enabled,
            snapshot
                .source_media
                .as_ref()
                .and_then(|source| source.audio_sample_rate_hz),
        )
    };
    if !video_enabled && !audio_enabled {
        return state.with_playback(&window, |playback| Ok(state.snapshot(playback)));
    }
    let target_root = match runtime_resource_target_root(&app) {
        Ok(target_root) => target_root,
        Err(error) => {
            return state.with_playback(&window, |playback| {
                playback.mark_media_processing_failed(format!("媒体运行资源目录不可用：{error}"));
                Ok(state.snapshot(playback))
            })
        }
    };
    let (ffmpeg_path, ffprobe_path) =
        match configured_media_engine_paths_with_resource_dir(&target_root) {
            Ok(paths) => paths,
            Err(error) => {
                return state.with_playback(&window, |playback| {
                    playback.mark_media_processing_failed(error.to_string());
                    Ok(state.snapshot(playback))
                })
            }
        };
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandErrorDto::new("media_cache_dir_failed", error.to_string()))?
        .join("media-processing");
    let _ = cleanup_local_caches(&app, &state)?;
    std::fs::create_dir_all(&cache_dir)
        .map_err(|error| CommandErrorDto::new("media_cache_dir_failed", error.to_string()))?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| CommandErrorDto::new("media_cache_nonce_failed", error.to_string()))?
        .as_nanos();
    let output_mp4_path = cache_dir.join(format!("processed-g{generation}-{nonce}.mp4"));
    let staging_output_path =
        cache_dir.join(format!("processed-g{generation}-{nonce}.partial.mp4"));
    let timeout_seconds = request.timeout_seconds.unwrap_or(6 * 60 * 60);
    let media_request = MediaRenderRequest {
        ffmpeg_path,
        ffprobe_path,
        input_mp4_path: PathBuf::from(&source_path),
        staging_output_path,
        output_mp4_path: output_mp4_path.clone(),
        video_processing_enabled: video_enabled,
        // 普通声音效果统一交给 FFmpeg。实时音频幻化是独立的候选音轨链路，
        // 不再通过 Web Audio 在播放端重复套用增益、EQ、混响或淡入淡出。
        audio_processing_enabled: audio_enabled,
        source_audio_sample_rate_hz,
        video: request.params.video,
        audio: request.params.audio,
        research: request.params.research,
        timeout_seconds,
    };
    if let Err(error) = build_media_render_args(&media_request) {
        state.with_playback(&window, |playback| {
            playback.mark_media_processing_failed(error.to_string());
            Ok(())
        })?;
        return state.with_playback(&window, |playback| Ok(state.snapshot(playback)));
    }
    state.with_playback(&window, |playback| {
        playback
            .mark_media_processing_running()
            .map_err(command_error_from_playback)
    })?;
    let cancellation = CancellationToken::new();
    let playback = Arc::clone(&state.playback);
    let completed = Arc::new(AtomicBool::new(false));
    let completed_for_thread = Arc::clone(&completed);
    let worker_cancellation = cancellation.clone();
    let thread_app = app.clone();
    let handle = thread::spawn(move || {
        let result = render_media(&media_request, &worker_cancellation);
        if let Ok(mut playback) = playback.lock() {
            if playback.snapshot().playback_generation == generation {
                match result {
                    Ok(rendered) => {
                        match allow_local_playback_asset_file(
                            &thread_app,
                            &rendered.output_mp4_path,
                            "media_processing_asset_scope_failed",
                            "处理后视频",
                        ) {
                            Ok(()) => {
                                let _ = playback.mark_media_processing_ready(
                                    generation,
                                    rendered.output_mp4_path.display().to_string(),
                                    rendered.output_mp4_sha256,
                                );
                            }
                            Err(error) => playback.mark_media_processing_failed(error.message),
                        }
                    }
                    Err(error) => playback.mark_media_processing_failed(error.to_string()),
                }
            }
        }
        completed_for_thread.store(true, Ordering::Release);
    });
    if let Err(task) = state.install_media_worker(MediaWorkerTask {
        cancellation,
        completed,
        handle,
    }) {
        task.cancellation.cancel();
        let _join_result = task.handle.join();
        let _ = state.with_playback(&window, |playback| {
            playback.mark_media_processing_failed("当前已有本地媒体处理 Worker 在执行");
            Ok(())
        });
        return Err(CommandErrorDto::new(
            "media_worker_already_running",
            "当前已有本地媒体处理 Worker 在执行",
        ));
    }
    state.with_playback(&window, |playback| Ok(state.snapshot(playback)))
}

#[tauri::command]
pub fn direct_model_chat(
    window: Window,
    state: State<'_, AppState>,
    request: DirectModelChatRequestDto,
) -> Result<DirectModelChatResponseDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let lease_expires_at = unix_ms_to_system_time(request.expires_at_unix_ms)?;
    let credential = match (
        request.direct_access_token,
        request.credential_expires_at_unix_ms,
    ) {
        (Some(access_token), Some(expires_at_unix_ms)) => Some(DelegatedAccessCredential {
            access_token,
            expires_at: unix_ms_to_system_time(expires_at_unix_ms)?,
        }),
        (Some(_), None) => {
            return Err(CommandErrorDto::new(
                "direct_model_credential_expiry_required",
                "直连凭证缺少过期时间",
            ));
        }
        (None, _) => None,
    };
    let session = DirectLeaseSession::new(
        DirectLeaseDescriptor {
            lease_id: request.lease_id,
            provider: request.provider,
            model: request.model,
            status: request.status,
            proxy_mode: request.proxy_mode,
            direct_base_url: request.direct_base_url,
            expires_at: lease_expires_at,
        },
        credential,
        SystemTime::now(),
    )
    .map_err(command_error_from_direct_model)?;
    let started_at = Instant::now();
    let result = session
        .chat_completions(
            &DirectChatRequest {
                messages: request.messages,
                temperature: request.temperature,
                max_tokens: request.max_tokens,
            },
            Duration::from_millis(request.timeout_ms),
            SystemTime::now(),
        )
        .map_err(command_error_from_direct_model)?;
    let (prompt_tokens, completion_tokens, total_tokens) = result
        .usage
        .map(|usage| {
            (
                Some(usage.prompt_tokens),
                Some(usage.completion_tokens),
                Some(usage.total_tokens),
            )
        })
        .unwrap_or((None, None, None));
    Ok(DirectModelChatResponseDto {
        text: result.text,
        model: result.model,
        finish_reason: result.finish_reason,
        prompt_tokens,
        completion_tokens,
        total_tokens,
        latency_ms: started_at.elapsed().as_millis().min(u64::MAX as u128) as u64,
    })
}

fn disable_media_processing_on_final_effect_close(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let _ = state.stop_media_worker();
    let _ = state.stop_speech_worker();
    // ponytail: 关最终效果窗时关掉声音/视频处理；实时幻化一并关
    let playback = Arc::clone(&state.playback);
    let lock_result = playback.lock();
    if let Ok(mut guard) = lock_result {
        guard.set_processing_switches(false, false, false);
    }
}

fn attach_final_effect_close_cleanup(app: &AppHandle, window: &tauri::WebviewWindow) {
    let app_handle = app.clone();
    let _ = window.on_window_event(move |event| {
        if matches!(
            event,
            tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
        ) {
            disable_media_processing_on_final_effect_close(&app_handle);
        }
    });
}

#[tauri::command]
pub async fn open_final_effect_window(
    app: AppHandle,
    request: Option<ResizeFinalEffectWindowRequestDto>,
) -> Result<FinalEffectWindowDto, CommandErrorDto> {
    let created = if let Some(window) = app.get_webview_window("final-effect") {
        window
            .show()
            .and_then(|_| window.set_focus())
            .map_err(|error| {
                CommandErrorDto::new("final_effect_window_show_failed", error.to_string())
            })?;
        false
    } else {
        // ponytail: 先默认尺寸创建，有分辨率时立刻按工作区 clamp 调整
        let window = WebviewWindowBuilder::new(&app, "final-effect", WebviewUrl::App("index.html".into()))
            .title("autolive-desktop-core 最终效果")
            .inner_size(1280.0, 720.0)
            .min_inner_size(320.0, 180.0)
            .resizable(true)
            .center()
            .build()
            .map_err(|error| {
                CommandErrorDto::new("final_effect_window_create_failed", error.to_string())
            })?;
        attach_final_effect_close_cleanup(&app, &window);
        true
    };
    if let Some(request) = request {
        let _ = resize_final_effect_window_for_app(&app, request)?;
    }
    Ok(FinalEffectWindowDto {
        label: "final-effect".to_owned(),
        created,
    })
}

#[tauri::command]
pub fn close_final_effect_window(app: AppHandle) -> Result<bool, CommandErrorDto> {
    let Some(window) = app.get_webview_window("final-effect") else {
        return Ok(false);
    };
    disable_media_processing_on_final_effect_close(&app);
    window.close().map_err(|error| {
        CommandErrorDto::new("final_effect_window_close_failed", error.to_string())
    })?;
    Ok(true)
}

#[tauri::command]
pub fn resize_final_effect_window(
    window: Window,
    app: AppHandle,
    request: ResizeFinalEffectWindowRequestDto,
) -> Result<FinalEffectWindowSizeDto, CommandErrorDto> {
    if !matches!(window.label(), "main" | "final-effect") {
        return Err(CommandErrorDto::new(
            "playback_window_not_allowed",
            "当前窗口不允许调整播放窗口尺寸",
        ));
    }
    resize_final_effect_window_for_app(&app, request)
}

fn resolve_final_effect_monitor(
    window: &tauri::WebviewWindow,
) -> Result<tauri::Monitor, CommandErrorDto> {
    for _ in 0..5 {
        if let Ok(Some(monitor)) = window.current_monitor() {
            return Ok(monitor);
        }
        if let Ok(Some(monitor)) = window.primary_monitor() {
            return Ok(monitor);
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err(CommandErrorDto::new(
        "final_effect_monitor_unavailable",
        "无法读取当前播放窗口所在显示器",
    ))
}

fn resize_final_effect_window_for_app(
    app: &AppHandle,
    request: ResizeFinalEffectWindowRequestDto,
) -> Result<FinalEffectWindowSizeDto, CommandErrorDto> {
    let window = app.get_webview_window("final-effect").ok_or_else(|| {
        CommandErrorDto::new(
            "final_effect_window_missing",
            "最终效果窗口尚未打开，无法按视频分辨率调整",
        )
    })?;

    let monitor = resolve_final_effect_monitor(&window)?;
    let scale_factor = monitor.scale_factor();
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return Err(CommandErrorDto::new(
            "final_effect_monitor_unavailable",
            "当前显示器缩放比例无效",
        ));
    }

    let work_area = monitor.work_area().size;
    let previous_size = window.inner_size().map_err(|error| {
        CommandErrorDto::new("final_effect_window_resize_failed", error.to_string())
    })?;
    let outer_size = window.outer_size().map_err(|error| {
        CommandErrorDto::new("final_effect_window_resize_failed", error.to_string())
    })?;
    let measured_titlebar_height =
        f64::from(outer_size.height.saturating_sub(previous_size.height)) / scale_factor;
    let native_titlebar_height = if measured_titlebar_height > 0.0 {
        measured_titlebar_height
    } else if cfg!(target_os = "macos") {
        MACOS_NATIVE_TITLEBAR_HEIGHT
    } else {
        0.0
    };
    let target = calculate_window_size(
        request.width,
        request.height,
        f64::from(work_area.width) / scale_factor,
        f64::from(work_area.height) / scale_factor,
        native_titlebar_height,
    )
    .map_err(|error| match error {
        WindowSizingError::InvalidVideoDimensions => CommandErrorDto::new(
            "invalid_video_dimensions",
            "视频宽高必须为正数且不超过安全上限",
        ),
        WindowSizingError::InvalidWorkArea => CommandErrorDto::new(
            "final_effect_monitor_unavailable",
            "当前显示器工作区尺寸无效",
        ),
    })?;

    window
        .set_size(LogicalSize::new(
            f64::from(target.width),
            f64::from(target.height),
        ))
        .map_err(|error| {
            CommandErrorDto::new("final_effect_window_resize_failed", error.to_string())
        })?;
    if let Err(error) = window.center() {
        let _ = window.set_size(PhysicalSize::new(previous_size.width, previous_size.height));
        return Err(CommandErrorDto::new(
            "final_effect_window_resize_failed",
            error.to_string(),
        ));
    }

    Ok(FinalEffectWindowSizeDto {
        width: target.width,
        height: target.height,
    })
}

#[tauri::command]
pub fn start_playback(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.playback_action(&window, PlaybackCore::start)
}

#[tauri::command]
pub fn pause_playback(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.playback_action(&window, PlaybackCore::pause)
}

#[tauri::command]
pub fn resume_playback(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.playback_action(&window, PlaybackCore::resume)
}

#[tauri::command]
pub fn update_playback_position(
    window: Window,
    state: State<'_, AppState>,
    request: UpdatePlaybackPositionRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.with_playback_window(&window, |playback| {
        playback.set_playback_position(request.position_ms);
        Ok(state.snapshot(playback))
    })
}

#[tauri::command]
pub fn stop_playback(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.stop_speech_worker()?;
    state.stop_media_worker()?;
    state.stop_research_worker()?;
    state.with_playback_window(&window, |playback| {
        playback.stop();
        Ok(state.snapshot(playback))
    })
}

#[tauri::command]
pub fn complete_playback_loop(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.stop_speech_worker()?;
    state.with_playback_window(&window, |playback| {
        playback
            .complete_loop()
            .map_err(command_error_from_playback)?;
        Ok(state.snapshot(playback))
    })
}

#[tauri::command]
pub fn commit_media_processing_if_ready(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.playback_action(&window, |playback| {
        playback.commit_media_processing_if_ready();
        Ok(())
    })
}

#[tauri::command]
pub fn set_processing_switches(
    window: Window,
    state: State<'_, AppState>,
    request: ProcessingSwitchesRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    let should_stop_media_worker = {
        let playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        let snapshot = playback.snapshot();
        snapshot.video_processing_enabled != request.video_processing_enabled
            || snapshot.audio_processing_enabled != request.audio_processing_enabled
            || (snapshot.audio_processing_enabled
                && snapshot.realtime_audio_variant_enabled
                    != request.realtime_audio_variant_enabled)
    };
    if should_stop_media_worker {
        state.stop_media_worker()?;
    }
    if !request.realtime_audio_variant_enabled {
        state.stop_speech_worker()?;
    }
    state.with_playback(&window, |playback| {
        playback.set_processing_switches(
            request.video_processing_enabled,
            request.audio_processing_enabled,
            request.realtime_audio_variant_enabled,
        );
        Ok(state.snapshot(playback))
    })
}

#[tauri::command]
pub fn set_audio_processing_profile(
    window: Window,
    state: State<'_, AppState>,
    request: AudioProcessingProfileRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    // 只更新配置，不打断正在跑的 FFmpeg；新参数由下一轮 start_media_processing 消费。
    let _ = state.reap_finished_media_worker()?;
    state.with_playback(&window, |playback| {
        playback
            .set_audio_processing_profile(request.profile)
            .map_err(|errors| {
                CommandErrorDto::new(
                    "audio_processing_profile_invalid",
                    errors
                        .iter()
                        .map(|error| format!("{}: {}", error.field, error.message))
                        .collect::<Vec<_>>()
                        .join("；"),
                )
            })?;
        Ok(state.snapshot(playback))
    })
}

#[tauri::command]
pub fn set_interlude_config(
    window: Window,
    state: State<'_, AppState>,
    request: SetInterludeConfigRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let (_, snapshot) = prepare_interlude_snapshot(InterludeConfig {
        enabled: request.enabled,
        directory: request.directory,
        interval_min_ms: request.interval_min_ms,
        interval_max_ms: request.interval_max_ms,
        volume_db: request.volume_db,
        ducking_depth_db: request.ducking_depth_db,
        ducking_attack_ms: request.ducking_attack_ms,
        ducking_release_ms: request.ducking_release_ms,
    })
    .map_err(command_error_from_interlude)?;
    state.with_playback(&window, move |playback| {
        playback.set_interlude_snapshot(snapshot.clone());
        Ok(PlaybackSnapshotDto::from(playback.snapshot()))
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct StageAudioVariantCandidateRequestDto {
    pub input: AudioTrackInput,
    pub context: SpeechToSpeechContext,
    pub candidate: AudioVariantCandidate,
    pub max_sync_offset_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitAudioVariantCandidateRequestDto {
    pub playback_generation: u64,
    pub loop_index: u64,
    pub segment_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscardAudioVariantCandidateRequestDto {
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitAudioVariantIfDueRequestDto {
    pub position_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartSpeechToSpeechRequestDto {
    pub input: AudioTrackInput,
    pub context: SpeechToSpeechContext,
    pub max_sync_offset_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpeechToSpeechStartResultDto {
    pub snapshot: PlaybackSnapshotDto,
    pub accepted: bool,
}

#[tauri::command]
pub fn start_speech_to_speech_worker(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: StartSpeechToSpeechRequestDto,
) -> Result<SpeechToSpeechStartResultDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let _ = state.reap_finished_speech_worker()?;
    let (target_root, target_root_error) = match runtime_resource_target_root(&app) {
        Ok(target_root) => (Some(target_root), None),
        Err(error) => (None, Some(format!("运行资源目录不可用：{error}"))),
    };
    let cancellation = CancellationToken::new();
    let playback_generation = request.context.playback_generation;
    let loop_index = request.context.loop_index;
    let input = request.input;
    let context = request.context;
    let max_sync_offset_ms = request.max_sync_offset_ms;
    let playback = Arc::clone(&state.playback);
    let completed = Arc::new(AtomicBool::new(false));
    let completed_for_thread = Arc::clone(&completed);

    let snapshot = state.with_playback_window(&window, |playback| {
        let current = playback.snapshot();
        if !current.realtime_audio_variant_enabled {
            return Err(CommandErrorDto::new(
                "realtime_audio_variant_disabled",
                "实时话术幻化开关未开启",
            ));
        }
        if current.playback_generation != playback_generation || current.loop_index != loop_index {
            return Err(CommandErrorDto::new(
                "audio_variant_candidate_stale",
                "话术上下文不属于当前播放代际或轮次",
            ));
        }
        if input.track_id != context.track_id {
            return Err(CommandErrorDto::new(
                "audio_track_context_mismatch",
                "音频输入与话术上下文的 track_id 不一致",
            ));
        }
        validate_speech_to_speech_source(&current, &input, &context)?;
        playback
            .mark_speech_to_speech_worker_running()
            .map_err(command_error_from_candidate)?;
        Ok(state.snapshot(playback))
    })?;

    let thread_context = context.clone();
    let thread_input = input.clone();
    let worker_cancellation = cancellation.clone();
    let thread_target_root = target_root.clone();
    let thread_target_root_error = target_root_error.clone();
    let handle = thread::spawn(move || {
        let result = match thread_target_root.as_deref() {
            Some(target_root) => run_configured_speech_to_speech_context_worker_with_resource_dir(
                &thread_context,
                &worker_cancellation,
                target_root,
            ),
            None => run_configured_speech_to_speech_context_worker(
                &thread_context,
                &worker_cancellation,
            ),
        };
        if let Ok(mut playback) = playback.lock() {
            let current = playback.snapshot();
            if current.playback_generation == thread_context.playback_generation
                && current.loop_index == thread_context.loop_index
            {
                match result {
                    Ok(worker_result) => match worker_result.decision {
                        autolive_desktop_core::speech_to_speech::SpeechToSpeechDecision::KeepOriginal => {
                            let _ = playback.finish_speech_to_speech_keep_original(
                                thread_context.playback_generation,
                                thread_context.loop_index,
                            );
                        }
                        autolive_desktop_core::speech_to_speech::SpeechToSpeechDecision::Rewrite => {
                            let candidate_result = build_audio_variant_candidate(
                                &thread_input,
                                &thread_context,
                                &worker_result,
                            )
                            .and_then(|candidate| {
                                playback
                                    .stage_audio_variant_candidate(
                                        &thread_input,
                                        &thread_context,
                                        candidate,
                                        max_sync_offset_ms,
                                    )
                                    .map_err(|error| error.to_string())
                            });
                            if let Err(reason) = candidate_result {
                                playback.discard_audio_variant_candidate(reason);
                            }
                        }
                    },
                    Err(error) => {
                        let reason = match thread_target_root_error.as_deref() {
                            Some(resource_error) => {
                                format!("{resource_error}；speech-to-speech Worker：{error}")
                            }
                            None => error.to_string(),
                        };
                        if matches!(error, SpeechToSpeechWorkerError::Cancelled) {
                            playback.mark_speech_to_speech_worker_cancelled(reason);
                        } else {
                            playback.fallback_audio_runtime(reason);
                        }
                    }
                }
            }
        }
        completed_for_thread.store(true, Ordering::Release);
    });
    if let Err(task) = state.install_speech_worker(SpeechWorkerTask {
        cancellation,
        completed,
        handle,
    }) {
        task.cancellation.cancel();
        let _join_result = task.handle.join();
        let _ = state.with_playback_window(&window, |playback| {
            playback.fallback_audio_runtime("当前播放片段已有实时话术 Worker 在执行");
            Ok(())
        });
        return Err(CommandErrorDto::new(
            "speech_worker_already_running",
            "当前播放片段已有实时话术 Worker 在执行",
        ));
    }
    Ok(SpeechToSpeechStartResultDto {
        snapshot,
        accepted: true,
    })
}

#[tauri::command]
pub fn cancel_speech_to_speech_worker(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    state.stop_speech_worker()?;
    state.with_playback_window(&window, |playback| {
        playback.mark_speech_to_speech_worker_cancelled("用户取消实时话术 Worker");
        Ok(state.snapshot(playback))
    })
}

#[tauri::command]
pub fn restore_original_audio(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    state.with_playback_window(&window, |playback| {
        playback.restore_original_audio();
        Ok(state.snapshot(playback))
    })
}

#[tauri::command]
pub fn stage_audio_variant_candidate(
    window: Window,
    state: State<'_, AppState>,
    request: StageAudioVariantCandidateRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.with_playback(&window, |playback| {
        if !playback.snapshot().realtime_audio_variant_enabled {
            return Err(CommandErrorDto::new(
                "realtime_audio_variant_disabled",
                "实时话术幻化开关未开启",
            ));
        }
        playback
            .stage_audio_variant_candidate(
                &request.input,
                &request.context,
                request.candidate,
                request.max_sync_offset_ms,
            )
            .map_err(command_error_from_candidate)?;
        Ok(state.snapshot(playback))
    })
}

#[tauri::command]
pub fn commit_audio_variant_candidate(
    window: Window,
    state: State<'_, AppState>,
    request: CommitAudioVariantCandidateRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.with_playback(&window, |playback| {
        playback
            .commit_audio_variant_candidate(
                request.playback_generation,
                request.loop_index,
                &request.segment_id,
            )
            .map_err(command_error_from_candidate)?;
        Ok(state.snapshot(playback))
    })
}

#[tauri::command]
pub fn commit_audio_variant_candidate_if_due(
    window: Window,
    state: State<'_, AppState>,
    request: CommitAudioVariantIfDueRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.with_playback_window(&window, |playback| {
        playback
            .commit_audio_variant_candidate_if_due(request.position_ms)
            .map_err(command_error_from_candidate)?;
        Ok(state.snapshot(playback))
    })
}

#[tauri::command]
pub fn discard_audio_variant_candidate(
    window: Window,
    state: State<'_, AppState>,
    request: DiscardAudioVariantCandidateRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.with_playback(&window, |playback| {
        playback.discard_audio_variant_candidate(request.reason);
        Ok(state.snapshot(playback))
    })
}

#[tauri::command]
pub fn get_snapshot(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    // 轮询时回收已完成 Worker，避免状态一直停在 processing。
    let _ = state.reap_finished_media_worker()?;
    if window.label() == "final-effect" {
        let playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        return Ok(state.snapshot(&playback));
    }
    state.with_playback(&window, |playback| Ok(state.snapshot(playback)))
}

#[tauri::command]
pub fn validate_local_research_params(
    window: Window,
    state: State<'_, AppState>,
    request: LocalResearchParams,
) -> Result<MediaParameterValidationResultDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let errors = request.validate().err().unwrap_or_default();
    Ok(MediaParameterValidationResultDto {
        valid: errors.is_empty(),
        errors,
    })
}

#[tauri::command]
pub fn get_default_local_research_params(
    window: Window,
    state: State<'_, AppState>,
) -> Result<LocalResearchParams, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    Ok(LocalResearchParams::default())
}

fn allow_local_playback_asset_file(
    app: &AppHandle,
    asset_path: &Path,
    error_code: &'static str,
    asset_label: &'static str,
) -> Result<(), CommandErrorDto> {
    let canonical_asset_path = std::fs::canonicalize(asset_path).map_err(|error| {
        CommandErrorDto::new(error_code, format!("{asset_label}路径无法规范化：{error}"))
    })?;
    let metadata = std::fs::metadata(&canonical_asset_path).map_err(|error| {
        CommandErrorDto::new(error_code, format!("{asset_label}文件无法读取：{error}"))
    })?;
    if !metadata.is_file() {
        return Err(CommandErrorDto::new(
            error_code,
            format!("{asset_label}路径不是文件"),
        ));
    }
    app.asset_protocol_scope()
        .allow_file(&canonical_asset_path)
        .map_err(|error| {
            CommandErrorDto::new(
                error_code,
                format!("{asset_label}无法加入 asset scope：{error}"),
            )
        })
}

fn command_error_from_playback(error: PlaybackError) -> CommandErrorDto {
    let code = match error {
        PlaybackError::EmptyWindowId => "empty_window_id",
        PlaybackError::WindowAlreadyBound { .. } => "playback_window_already_bound",
        PlaybackError::SourceMediaRequired => "source_media_required",
        PlaybackError::InvalidMediaProcessingOutput => "invalid_media_processing_output",
        PlaybackError::StaleMediaProcessing => "stale_media_processing",
        PlaybackError::InvalidTransition { .. } => "invalid_playback_transition",
    };
    CommandErrorDto::new(code, error.to_string())
}

fn command_error_from_media_library(error: MediaLibraryError) -> CommandErrorDto {
    CommandErrorDto::new("media_probe_failed", error.to_string())
}

fn command_error_from_candidate(error: CandidateValidationError) -> CommandErrorDto {
    CommandErrorDto::new("audio_variant_candidate_rejected", error.to_string())
}

fn command_error_from_interlude(error: InterludeError) -> CommandErrorDto {
    let code = match error {
        InterludeError::MissingDirectory => "interlude_directory_required",
        InterludeError::DirectoryUnavailable(_) | InterludeError::DirectoryReadFailed(_) => {
            "interlude_directory_invalid"
        }
        InterludeError::NoUsableAudioFiles => "interlude_audio_files_empty",
        InterludeError::IntervalMinOutOfRange
        | InterludeError::IntervalMaxOutOfRange
        | InterludeError::IntervalOrderInvalid
        | InterludeError::VolumeOutOfRange
        | InterludeError::DuckingDepthOutOfRange
        | InterludeError::DuckingAttackOutOfRange
        | InterludeError::DuckingReleaseOutOfRange => "interlude_config_invalid",
    };
    CommandErrorDto::new(code, error.to_string())
}

fn unix_ms_to_system_time(value: u64) -> Result<SystemTime, CommandErrorDto> {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_millis(value))
        .ok_or_else(|| CommandErrorDto::new("direct_model_expiry_invalid", "租约过期时间无效"))
}

fn command_error_from_direct_model(error: DirectModelError) -> CommandErrorDto {
    let (code, message): (&str, String) = match error {
        DirectModelError::DelegatedCredentialRequired => (
            "direct_model_credential_required",
            "当前模型租约没有供应商委托凭证，Rust 直连不可用；不会回退使用完整 API Key".to_owned(),
        ),
        DirectModelError::LeaseExpired => (
            "direct_model_lease_expired",
            "模型租约或直连凭证已过期".to_owned(),
        ),
        DirectModelError::LeaseUnavailable => (
            "direct_model_lease_unavailable",
            "模型租约当前不可用".to_owned(),
        ),
        DirectModelError::InvalidLeaseUrl => (
            "direct_model_lease_url_invalid",
            "模型租约直连地址不符合安全限制".to_owned(),
        ),
        DirectModelError::InvalidRequest(message) => ("direct_model_request_invalid", message),
        DirectModelError::RequestFailed(message) => ("direct_model_request_failed", message),
        DirectModelError::SupplierRejected { status } => (
            "direct_model_supplier_rejected",
            format!("模型供应商拒绝请求（HTTP {status}）"),
        ),
        DirectModelError::InvalidResponse(message) => ("direct_model_response_invalid", message),
    };
    CommandErrorDto::new(code, message)
}

fn validate_speech_to_speech_source(
    snapshot: &PlaybackSnapshot,
    input: &AudioTrackInput,
    context: &SpeechToSpeechContext,
) -> Result<(), CommandErrorDto> {
    if input.source_kind != "local_file" || context.source_kind != "local_file" {
        return Err(CommandErrorDto::new(
            "audio_source_kind_unsupported",
            "当前版本只支持已探测源视频的 local_file 音轨，实时流接入尚未开放",
        ));
    }
    if input.audio_path_or_stream_ref != context.audio_path_or_stream_ref {
        return Err(CommandErrorDto::new(
            "audio_source_context_mismatch",
            "音频输入路径与话术上下文路径不一致",
        ));
    }
    let expected = snapshot
        .source_media
        .as_ref()
        .map(|source| source.source_path.as_str())
        .ok_or_else(|| CommandErrorDto::new("source_media_required", "当前没有已探测源视频"))?;
    let actual = input
        .audio_path_or_stream_ref
        .strip_prefix("file://")
        .unwrap_or(&input.audio_path_or_stream_ref);
    let expected_path = std::fs::canonicalize(expected)
        .map_err(|_| CommandErrorDto::new("source_media_invalid", "当前源视频路径无法规范化"))?;
    let actual_path = std::fs::canonicalize(actual)
        .map_err(|_| CommandErrorDto::new("audio_source_invalid", "话术音轨路径无法规范化"))?;
    if expected_path != actual_path {
        return Err(CommandErrorDto::new(
            "audio_source_not_current_media",
            "话术音轨必须来自当前已探测源视频",
        ));
    }
    Ok(())
}

fn build_audio_variant_candidate(
    input: &AudioTrackInput,
    context: &SpeechToSpeechContext,
    result: &autolive_desktop_core::speech_to_speech::SpeechToSpeechResult,
) -> Result<AudioVariantCandidate, String> {
    result
        .validate_against(context, 3)
        .map_err(|error| error.to_string())?;
    let reference = result
        .audio_path_or_stream_ref
        .as_deref()
        .ok_or_else(|| "Worker 未返回候选音频引用".to_owned())?;
    let raw_path = reference.strip_prefix("file://").unwrap_or(reference);
    let path = PathBuf::from(raw_path);
    if !path.is_absolute() || !path.is_file() {
        return Err("候选音频必须是存在的本地文件".to_owned());
    }
    let canonical_path =
        std::fs::canonicalize(&path).map_err(|_| "候选音频路径规范化失败".to_owned())?;
    let actual_sha256 = hash_file_at_path(&canonical_path, &CancellationToken::new())
        .map_err(|error| error.to_string())?;
    let expected_sha256 = result
        .audio_sha256
        .as_deref()
        .ok_or_else(|| "Worker 未返回候选音频 SHA-256".to_owned())?;
    if actual_sha256 != expected_sha256 {
        return Err("候选音频 SHA-256 与实际文件不一致".to_owned());
    }
    let duration_ms = result
        .duration_ms
        .ok_or_else(|| "Worker 未返回候选音频时长".to_owned())?;
    let sync_offset_ms = result
        .sync_offset_ms
        .ok_or_else(|| "Worker 未返回候选音频同步偏移".to_owned())?;
    let sample_rate_hz = result
        .sample_rate_hz
        .ok_or_else(|| "Worker 未返回候选音频采样率".to_owned())?;
    let channel_count = result
        .channel_count
        .ok_or_else(|| "Worker 未返回候选音频声道数".to_owned())?;
    if sample_rate_hz != input.sample_rate_hz || channel_count != input.channel_count {
        return Err("候选音频采样率或声道与输入音轨不一致".to_owned());
    }
    Ok(AudioVariantCandidate {
        playback_generation: context.playback_generation,
        loop_index: context.loop_index,
        segment_id: context.segment_id.clone(),
        start_at_ms: context.start_at_ms,
        variant_mode: "rewrite".to_owned(),
        audio_path_or_stream_ref: canonical_path.display().to_string(),
        audio_sha256: actual_sha256,
        duration_ms,
        sync_offset_ms,
        sample_rate_hz,
        channel_count,
        ready: true,
    })
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::development_executable_ready;
    use super::AppState;

    #[cfg(unix)]
    #[test]
    fn development_resource_gate_requires_an_executable_file() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "autolive-runtime-resource-executable-{}",
            std::process::id()
        ));
        std::fs::write(&path, b"fixture").expect("fixture should be written");
        let mut permissions = std::fs::metadata(&path)
            .expect("fixture metadata should be readable")
            .permissions();
        permissions.set_mode(0o644);
        std::fs::set_permissions(&path, permissions).expect("permissions should be set");
        assert!(!development_executable_ready(&path));

        let mut permissions = std::fs::metadata(&path)
            .expect("fixture metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("permissions should be set");
        assert!(development_executable_ready(&path));
        let _ignored = std::fs::remove_file(path);
    }

    #[test]
    fn state_starts_without_a_source_or_queue() {
        let state = AppState::default();
        let playback = state.playback.lock().expect("playback lock");
        let snapshot = playback.snapshot();
        assert!(snapshot.source_media.is_none());
        assert_eq!(snapshot.loop_index, 0);
    }
}
