use autolive_desktop_core::audio_processing::AudioProcessingProfile;
use autolive_desktop_core::cancellation::CancellationToken;
use autolive_desktop_core::direct_model::{
    DelegatedAccessCredential, DirectChatMessage, DirectChatRequest, DirectLeaseDescriptor,
    DirectLeaseSession, DirectModelError,
};
use autolive_desktop_core::errors::{FileHashError, MediaLibraryError, PlaybackError};
use autolive_desktop_core::hashing::{
    hash_file_at_path, hash_mp4_sha256_dto, FileHashRequestDto, Mp4Sha256Dto,
};
use autolive_desktop_core::media_engine::{
    build_media_render_args, configured_media_engine_paths,
    configured_media_engine_paths_with_resource_dir,
    configured_media_engine_status_with_resource_dir, render_media, MediaEngineStatus,
    MediaRenderRequest,
};
use autolive_desktop_core::media_library::{
    probe_user_selected_mp4, MediaProbeRequestDto, MediaProbeResultDto, SourceMediaDto,
};
use autolive_desktop_core::research_params::{LocalResearchParams, ParameterValidationError};
use autolive_desktop_core::research_worker::{
    configured_research_worker_capabilities, configured_research_worker_executable, run_research,
    validate_research_identifier, ResearchAnalysisRequest, ResearchResult,
    ResearchWorkerCapabilities,
};
use autolive_desktop_core::speech_to_speech::SpeechToSpeechWorkerCapabilities;
use autolive_desktop_core::speech_to_speech::{
    AudioTrackInput, AudioVariantCandidate, CandidateValidationError, SpeechToSpeechContext,
};
use autolive_desktop_core::speech_to_speech_worker::configured_speech_to_speech_worker_capabilities;
use autolive_desktop_core::speech_to_speech_worker::run_configured_speech_to_speech_context_worker;
use autolive_desktop_core::speech_to_speech_worker::SpeechToSpeechWorkerError;
use autolive_desktop_core::voice_clone::{
    validate_replacement_result, VoiceClonePrepareRequest, VoiceCloneReplacementRequest,
    VoiceCloneReplacementResult, VoiceCloneSegment, VoiceCloneSourceIndex,
};
use autolive_desktop_core::window_sizing::{calculate_window_size, WindowSizingError};
use autolive_desktop_core::{
    PlaybackCore, PlaybackSnapshot, VoiceCloneCommittedReplacement, VoiceClonePreparedSource,
    VoiceCloneRuntimeError,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use sysinfo::{Disks, System};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder, Window};
use tauri_runtime::dpi::{LogicalSize, PhysicalSize};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const MEDIA_CACHE_MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const RESEARCH_CACHE_MAX_BYTES: u64 = 512 * 1024 * 1024;
const MACOS_NATIVE_TITLEBAR_HEIGHT: f64 = 32.0;
const VOICE_CLONE_WORKER_ENV: &str = "AUTOLIVE_VOICE_CLONE_WORKER";
const VOICE_CLONE_CAPABILITY_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_VOICE_CLONE_TIMEOUT_MS: u64 = 15 * 60 * 1_000;
const VOICE_CLONE_AUDIO_PROBE_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug)]
pub struct AppState {
    main_window_label: &'static str,
    playback: Arc<Mutex<PlaybackCore>>,
    speech_worker: Arc<Mutex<Option<SpeechWorkerTask>>>,
    voice_clone_worker: Arc<Mutex<Option<VoiceCloneWorkerTask>>>,
    media_worker: Arc<Mutex<Option<MediaWorkerTask>>>,
    research_worker: Arc<Mutex<Option<ResearchWorkerTask>>>,
    research_status: Arc<Mutex<ResearchStatusDto>>,
}

#[derive(Debug)]
struct SpeechWorkerTask {
    cancellation: CancellationToken,
    completed: Arc<AtomicBool>,
    handle: thread::JoinHandle<()>,
}

#[derive(Debug)]
struct VoiceCloneWorkerTask {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceCloneWorkerCapabilitiesDto {
    pub available: bool,
    pub status: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrepareVoiceCloneSourceRequestDto {
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartVoiceCloneReplacementRequestDto {
    pub text: String,
    pub position_ms: u64,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdatePlaybackPositionRequestDto {
    pub position_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct VoiceClonePrepareWorkerResult {
    status: String,
    provider: Option<String>,
    model: Option<String>,
    reason: Option<String>,
    operation_id: String,
    source_path: Option<String>,
    source_sha256: Option<String>,
    reference_audio_path: Option<String>,
    reference_audio_sha256: Option<String>,
    sample_rate_hz: Option<u32>,
    channel_count: Option<u16>,
    duration_ms: Option<u64>,
    #[serde(default)]
    segments: Vec<VoiceCloneSegment>,
}

#[derive(Debug, Clone, Deserialize)]
struct VoiceCloneReplaceWorkerResult {
    status: String,
    provider: Option<String>,
    model: Option<String>,
    reason: Option<String>,
    operation_id: String,
    source_path: Option<String>,
    source_sha256: Option<String>,
    replacement_audio_path: Option<String>,
    replacement_sha256: Option<String>,
    replace_at_ms: Option<u64>,
    resume_at_ms: Option<u64>,
    total_duration_ms: Option<u64>,
    sample_rate_hz: Option<u32>,
    channel_count: Option<u16>,
    input_text: Option<String>,
    source_generation: Option<u64>,
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

impl Default for AppState {
    fn default() -> Self {
        Self {
            main_window_label: "main",
            playback: Arc::new(Mutex::new(PlaybackCore::default())),
            speech_worker: Arc::new(Mutex::new(None)),
            voice_clone_worker: Arc::new(Mutex::new(None)),
            media_worker: Arc::new(Mutex::new(None)),
            research_worker: Arc::new(Mutex::new(None)),
            research_status: Arc::new(Mutex::new(ResearchStatusDto::default())),
        }
    }
}

impl AppState {
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

    fn speech_worker_is_running(&self) -> Result<bool, CommandErrorDto> {
        let worker = self.speech_worker.lock().map_err(|_| {
            CommandErrorDto::new("speech_worker_lock_failed", "Worker 状态锁已损坏")
        })?;
        Ok(worker.is_some())
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

    fn reap_finished_voice_clone_worker(&self) -> Result<bool, CommandErrorDto> {
        let task = {
            let mut worker = self.voice_clone_worker.lock().map_err(|_| {
                CommandErrorDto::new(
                    "voice_clone_worker_lock_failed",
                    "固定话术 Worker 状态锁已损坏",
                )
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

    fn voice_clone_worker_is_running(&self) -> Result<bool, CommandErrorDto> {
        let worker = self.voice_clone_worker.lock().map_err(|_| {
            CommandErrorDto::new(
                "voice_clone_worker_lock_failed",
                "固定话术 Worker 状态锁已损坏",
            )
        })?;
        Ok(worker.is_some())
    }

    fn stop_voice_clone_worker(&self) -> Result<(), CommandErrorDto> {
        let task = self
            .voice_clone_worker
            .lock()
            .map_err(|_| {
                CommandErrorDto::new(
                    "voice_clone_worker_lock_failed",
                    "固定话术 Worker 状态锁已损坏",
                )
            })?
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

    fn install_voice_clone_worker(
        &self,
        task: VoiceCloneWorkerTask,
    ) -> Result<(), VoiceCloneWorkerTask> {
        let mut worker = match self.voice_clone_worker.lock() {
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
    pub voice_clone_replacement: autolive_desktop_core::VoiceCloneReplacementState,
}

impl From<PlaybackSnapshot> for PlaybackSnapshotDto {
    fn from(value: PlaybackSnapshot) -> Self {
        Self {
            window_id: value.window_id,
            playback_generation: value.playback_generation,
            playback_state: format!("{:?}", value.playback_state).to_ascii_lowercase(),
            source_media: value.source_media,
            loop_index: value.loop_index,
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
            voice_clone_replacement: value.voice_clone_replacement,
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
    entries.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, modified) in entries {
        let is_partial = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains(".partial"));
        let over_budget = remaining_bytes > max_bytes;
        let stale_partial = is_partial && modified <= partial_expiry;
        if protected.contains(&path) || (!over_budget && !stale_partial) {
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

fn prune_cache_tree(
    directory: &Path,
    max_bytes: u64,
    protected_paths: &[PathBuf],
) -> std::io::Result<CacheCleanupResultDto> {
    if !directory.is_dir() {
        return Ok(CacheCleanupResultDto {
            removed_files: 0,
            removed_bytes: 0,
            remaining_bytes: 0,
        });
    }
    let mut entries = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in std::fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                entries.push((
                    path,
                    metadata.len(),
                    metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                ));
            }
        }
    }
    let mut remaining_bytes = entries.iter().map(|(_, size, _)| *size).sum::<u64>();
    let mut removed_files: u32 = 0;
    let mut removed_bytes: u64 = 0;
    let partial_expiry = SystemTime::now()
        .checked_sub(Duration::from_secs(10 * 60))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    entries.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, modified) in entries {
        let is_partial = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains(".partial"));
        let over_budget = remaining_bytes > max_bytes;
        let stale_partial = is_partial && modified <= partial_expiry;
        let protected = protected_paths
            .iter()
            .any(|protected_path| path == *protected_path || path.starts_with(protected_path));
        if protected || (!over_budget && !stale_partial) {
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
        let result = prune_cache_dir(&directory, 1, std::slice::from_ref(&protected))
            .expect("cache prune should succeed");

        assert!(protected.exists());
        assert!(!stale_partial.exists());
        assert!(result.remaining_bytes >= fs::metadata(&protected).unwrap().len());
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
    let (current_video, pending_video, research_paths, voice_clone_protected_paths) = {
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
        let voice_clone_protected_paths = voice_clone_artifact_dirs(&playback, true)
            .into_iter()
            .filter_map(|path| path.parent().map(Path::to_path_buf))
            .collect::<Vec<_>>();
        (
            snapshot.current_video_reference.map(PathBuf::from),
            snapshot.pending_video_reference.map(PathBuf::from),
            research_paths,
            voice_clone_protected_paths,
        )
    };
    let media_result = prune_cache_dir(
        &cache_root.join("media-processing"),
        MEDIA_CACHE_MAX_BYTES,
        &[current_video, pending_video]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
    )
    .map_err(|error| CommandErrorDto::new("media_cache_cleanup_failed", error.to_string()))?;
    let research_result = prune_cache_dir(
        &cache_root.join("research-analysis"),
        RESEARCH_CACHE_MAX_BYTES,
        &research_paths,
    )
    .map_err(|error| CommandErrorDto::new("research_cache_cleanup_failed", error.to_string()))?;
    let voice_clone_result = prune_cache_tree(
        &cache_root.join("voice-clone"),
        1_024 * 1024 * 1024,
        &voice_clone_protected_paths,
    )
    .map_err(|error| CommandErrorDto::new("voice_clone_cache_cleanup_failed", error.to_string()))?;
    Ok(CacheCleanupResultDto {
        removed_files: media_result.removed_files
            + research_result.removed_files
            + voice_clone_result.removed_files,
        removed_bytes: media_result.removed_bytes
            + research_result.removed_bytes
            + voice_clone_result.removed_bytes,
        remaining_bytes: media_result.remaining_bytes
            + research_result.remaining_bytes
            + voice_clone_result.remaining_bytes,
    })
}

impl AppState {
    fn set_source(
        &self,
        window: &Window,
        source: SourceMediaDto,
    ) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
        self.with_playback(window, |playback| {
            playback.set_source(source);
            Ok(self.snapshot(playback))
        })
    }

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
pub fn probe_local_mp4(
    window: Window,
    state: State<'_, AppState>,
    request: MediaProbeRequestDto,
) -> Result<MediaProbeResultDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    state.stop_speech_worker()?;
    state.stop_voice_clone_worker()?;
    state.stop_media_worker()?;
    state.stop_research_worker()?;
    let cleanup_paths = {
        let playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        voice_clone_artifact_dirs(&playback, true)
    };
    cleanup_voice_clone_paths(cleanup_paths);

    let cancellation = CancellationToken::new();
    let result = probe_user_selected_mp4(&request, &cancellation)
        .map_err(command_error_from_media_library)?;
    let snapshot = state.set_source(&window, result.source.clone())?;
    let playback_handle = Arc::clone(&state.playback);
    let path = result.canonical_path.clone();
    let generation = snapshot.playback_generation;
    thread::spawn(move || {
        let hash_result =
            hash_mp4_sha256_dto(&FileHashRequestDto { path }, &CancellationToken::new());
        if let Ok(mut playback) = playback_handle.lock() {
            match hash_result {
                Ok(hash) => playback.set_mp4_sha256_for_generation(generation, hash.mp4_sha256),
                Err(_) => playback.set_mp4_hash_failed_for_generation(generation),
            }
        }
    });
    Ok(result)
}

#[tauri::command]
pub fn hash_local_mp4_sha256(
    window: Window,
    state: State<'_, AppState>,
    request: FileHashRequestDto,
) -> Result<Mp4Sha256Dto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    hash_mp4_sha256_dto(&request, &CancellationToken::new()).map_err(command_error_from_file_hash)
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
    state: State<'_, AppState>,
) -> Result<SpeechToSpeechWorkerCapabilities, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    Ok(configured_speech_to_speech_worker_capabilities())
}

#[tauri::command]
pub fn get_voice_clone_worker_capabilities(
    window: Window,
    state: State<'_, AppState>,
) -> Result<VoiceCloneWorkerCapabilitiesDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let executable = match configured_voice_clone_worker_executable() {
        Ok(executable) => executable,
        Err(reason) => return Ok(unavailable_voice_clone_capabilities(reason)),
    };
    let temp_root = std::env::temp_dir().join(format!(
        "autolive-voice-clone-capabilities-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|error| {
                CommandErrorDto::new("voice_clone_capability_probe_failed", error.to_string())
            })?
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_root).map_err(|error| {
        CommandErrorDto::new("voice_clone_capability_probe_failed", error.to_string())
    })?;
    let output_json = temp_root.join("capabilities.json");
    let args = vec![
        "--capabilities-json".to_owned(),
        output_json.display().to_string(),
    ];
    let result = match run_voice_clone_worker_process(
        &executable,
        &args,
        VOICE_CLONE_CAPABILITY_TIMEOUT_MS,
        &CancellationToken::new(),
    ) {
        Ok(()) => read_json_file::<VoiceCloneWorkerCapabilitiesDto>(
            &output_json,
            "voice_clone_capability_probe_failed",
            "固定话术能力 JSON 无法解析",
        )
        .unwrap_or_else(|error| unavailable_voice_clone_capabilities(error.message)),
        Err(VoiceCloneWorkerRunError::Timeout { timeout_ms }) => {
            unavailable_voice_clone_capabilities(format!(
                "固定话术 Worker 能力探测超时（{timeout_ms}ms）"
            ))
        }
        Err(VoiceCloneWorkerRunError::Cancelled) => {
            unavailable_voice_clone_capabilities("固定话术 Worker 能力探测被取消")
        }
        Err(VoiceCloneWorkerRunError::Failed(reason)) => {
            unavailable_voice_clone_capabilities(reason)
        }
    };
    cleanup_voice_clone_paths([temp_root]);
    Ok(result)
}

#[tauri::command]
pub fn get_media_engine_capabilities(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MediaEngineStatus, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|error| CommandErrorDto::new("media_resource_dir_failed", error.to_string()))?;
    Ok(configured_media_engine_status_with_resource_dir(
        &resource_dir,
    ))
}

#[tauri::command]
pub fn prepare_voice_clone_source(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: PrepareVoiceCloneSourceRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let _ = state.reap_finished_voice_clone_worker()?;
    if state.voice_clone_worker_is_running()? {
        return Err(CommandErrorDto::new(
            "voice_clone_worker_already_running",
            "当前已有固定话术 Worker 在执行",
        ));
    }
    let timeout_ms = request.timeout_ms.unwrap_or(DEFAULT_VOICE_CLONE_TIMEOUT_MS);
    let executable = configured_voice_clone_worker_executable()
        .map_err(|reason| CommandErrorDto::new("voice_clone_worker_unavailable", reason))?;
    let cache_root = voice_clone_cache_root(&app)?;
    let (
        source_generation,
        canonical_source_path,
        source_sha256,
        operation_id,
        stale_paths,
        snapshot,
    ) = {
        let mut playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        playback
            .bind_window(window.label())
            .map_err(command_error_from_playback)?;
        let snapshot = playback.snapshot();
        let source = snapshot
            .source_media
            .as_ref()
            .ok_or_else(|| CommandErrorDto::new("source_media_required", "请先导入一个源视频"))?;
        let source_sha256 = source.mp4_sha256.clone().ok_or_else(|| {
            CommandErrorDto::new("source_hash_required", "源视频完整 SHA-256 尚未完成")
        })?;
        let canonical_source_path = std::fs::canonicalize(&source.source_path).map_err(|_| {
            CommandErrorDto::new("source_media_invalid", "当前源视频路径无法规范化")
        })?;
        let operation_id = voice_clone_operation_id("prepare", snapshot.playback_generation)?;
        let stale_paths = voice_clone_artifact_dirs(&playback, true);
        playback
            .mark_voice_clone_preparing(&operation_id)
            .map_err(command_error_from_voice_clone_runtime)?;
        (
            snapshot.playback_generation,
            canonical_source_path,
            source_sha256,
            operation_id,
            stale_paths,
            state.snapshot(&playback),
        )
    };
    cleanup_voice_clone_paths(stale_paths);
    let prepared_root = cache_root.join(&source_sha256);
    std::fs::create_dir_all(&prepared_root)
        .map_err(|error| CommandErrorDto::new("voice_clone_cache_dir_failed", error.to_string()))?;
    let request_json = prepared_root.join(format!("{operation_id}.prepare.json"));
    let output_json = prepared_root.join(format!("{operation_id}.prepare-result.json"));
    let request_payload = VoiceClonePrepareRequest {
        source_generation,
        source_path: canonical_source_path.display().to_string(),
        operation_id: operation_id.clone(),
    };
    write_json_file(&request_json, &request_payload)?;

    let playback = Arc::clone(&state.playback);
    let completed = Arc::new(AtomicBool::new(false));
    let completed_for_thread = Arc::clone(&completed);
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let thread_executable = executable.clone();
    let thread_request_json = request_json.clone();
    let thread_output_json = output_json.clone();
    let thread_prepared_root = prepared_root.clone();
    let thread_canonical_source_path = canonical_source_path.clone();
    let thread_source_sha256 = source_sha256.clone();
    let thread_operation_id = operation_id.clone();
    let handle = thread::spawn(move || {
        let args = vec![
            "--prepare-json".to_owned(),
            thread_request_json.display().to_string(),
            "--output-json".to_owned(),
            thread_output_json.display().to_string(),
        ];
        let worker_result = run_voice_clone_worker_process(
            &thread_executable,
            &args,
            timeout_ms,
            &worker_cancellation,
        );
        let prepared = match worker_result {
            Ok(()) => read_json_file::<VoiceClonePrepareWorkerResult>(
                &thread_output_json,
                "voice_clone_prepare_failed",
                "固定话术准备结果 JSON 无法解析",
            )
            .and_then(|result| {
                validate_voice_clone_prepare_result(
                    &thread_prepared_root,
                    source_generation,
                    &thread_canonical_source_path,
                    &thread_source_sha256,
                    &thread_operation_id,
                    result,
                )
            }),
            Err(VoiceCloneWorkerRunError::Cancelled) => {
                if let Ok(mut playback) = playback.lock() {
                    let current = playback.snapshot();
                    if current.playback_generation == source_generation
                        && current
                            .source_media
                            .as_ref()
                            .map(|source| source.source_path.as_str())
                            == Some(thread_canonical_source_path.to_string_lossy().as_ref())
                    {
                        playback.mark_voice_clone_cancelled("用户取消固定话术准备");
                    }
                }
                cleanup_voice_clone_paths([
                    thread_request_json.clone(),
                    thread_output_json.clone(),
                    thread_prepared_root.join(&thread_operation_id),
                ]);
                completed_for_thread.store(true, Ordering::Release);
                return;
            }
            Err(VoiceCloneWorkerRunError::Timeout { timeout_ms }) => Err(CommandErrorDto::new(
                "voice_clone_prepare_failed",
                format!("固定话术准备超时（{timeout_ms}ms）"),
            )),
            Err(VoiceCloneWorkerRunError::Failed(reason)) => {
                Err(CommandErrorDto::new("voice_clone_prepare_failed", reason))
            }
        };

        if let Ok(mut playback) = playback.lock() {
            match prepared {
                Ok(prepared_source) => {
                    if playback
                        .set_voice_clone_prepared_source(prepared_source)
                        .is_err()
                    {
                        cleanup_voice_clone_paths(
                            [thread_prepared_root.join(&thread_operation_id)],
                        );
                    }
                }
                Err(error) => {
                    cleanup_voice_clone_paths([thread_prepared_root.join(&thread_operation_id)]);
                    playback.mark_voice_clone_failed(error.message);
                }
            }
        } else {
            cleanup_voice_clone_paths([thread_prepared_root.join(&thread_operation_id)]);
        }
        cleanup_voice_clone_paths([thread_request_json, thread_output_json]);
        completed_for_thread.store(true, Ordering::Release);
    });
    if let Err(task) = state.install_voice_clone_worker(VoiceCloneWorkerTask {
        cancellation,
        completed,
        handle,
    }) {
        task.cancellation.cancel();
        let _join_result = task.handle.join();
        cleanup_voice_clone_paths([request_json, output_json, prepared_root.join(&operation_id)]);
        return Err(CommandErrorDto::new(
            "voice_clone_worker_already_running",
            "当前已有固定话术 Worker 在执行",
        ));
    }
    Ok(snapshot)
}

#[tauri::command]
pub fn start_voice_clone_replacement(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: StartVoiceCloneReplacementRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let _ = state.reap_finished_voice_clone_worker()?;
    if state.voice_clone_worker_is_running()? {
        return Err(CommandErrorDto::new(
            "voice_clone_worker_already_running",
            "当前已有固定话术 Worker 在执行",
        ));
    }
    let timeout_ms = request.timeout_ms.unwrap_or(DEFAULT_VOICE_CLONE_TIMEOUT_MS);
    let executable = configured_voice_clone_worker_executable()
        .map_err(|reason| CommandErrorDto::new("voice_clone_worker_unavailable", reason))?;
    let cache_root = voice_clone_cache_root(&app)?;
    let (_, ffprobe_path) = configured_media_engine_paths().map_err(|error| {
        CommandErrorDto::new("voice_clone_media_engine_unavailable", error.to_string())
    })?;
    let _ = state.reap_finished_speech_worker()?;
    let speech_worker_running = state.speech_worker_is_running()?;
    let (plan, stale_replacement_paths, snapshot) = {
        let mut playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        playback
            .bind_window(window.label())
            .map_err(command_error_from_playback)?;
        let occupancy = speech_worker_running || realtime_audio_is_occupying(&playback.snapshot());
        let operation_id =
            voice_clone_operation_id("replace", playback.snapshot().playback_generation)?;
        let stale_replacement_paths = voice_clone_artifact_dirs(&playback, false);
        let plan = playback
            .start_voice_clone_replacement(
                &request.text,
                request.position_ms,
                &operation_id,
                occupancy,
            )
            .map_err(command_error_from_voice_clone_runtime)?;
        (plan, stale_replacement_paths, state.snapshot(&playback))
    };
    cleanup_voice_clone_paths(stale_replacement_paths);
    let replacement_root = cache_root.join(&plan.source_sha256);
    std::fs::create_dir_all(&replacement_root)
        .map_err(|error| CommandErrorDto::new("voice_clone_cache_dir_failed", error.to_string()))?;
    let request_json = replacement_root.join(format!("{}.replace.json", plan.operation_id));
    let output_json = replacement_root.join(format!("{}.replace-result.json", plan.operation_id));
    let request_payload = VoiceCloneReplacementRequest {
        source_generation: plan.source_generation,
        source_path: plan.source_path.clone(),
        source_sha256: plan.source_sha256.clone(),
        audio_base_path: plan.audio_base_path.clone(),
        source_duration_ms: plan.source_duration_ms,
        sample_rate_hz: plan.sample_rate_hz,
        channel_count: plan.channel_count,
        operation_id: plan.operation_id.clone(),
        reference_audio_path: plan.reference_audio_path.clone(),
        text: plan.input_text.clone(),
        replace_at_ms: plan.replace_at_ms,
        resume_at_ms: plan.resume_at_ms,
    };
    write_json_file(&request_json, &request_payload)?;

    let playback = Arc::clone(&state.playback);
    let completed = Arc::new(AtomicBool::new(false));
    let completed_for_thread = Arc::clone(&completed);
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let thread_executable = executable.clone();
    let thread_request_json = request_json.clone();
    let thread_output_json = output_json.clone();
    let thread_replacement_root = replacement_root.clone();
    let thread_plan = plan.clone();
    let thread_ffprobe_path = ffprobe_path.clone();
    let handle = thread::spawn(move || {
        let args = vec![
            "--replace-json".to_owned(),
            thread_request_json.display().to_string(),
            "--output-json".to_owned(),
            thread_output_json.display().to_string(),
        ];
        let worker_result = run_voice_clone_worker_process(
            &thread_executable,
            &args,
            timeout_ms,
            &worker_cancellation,
        );
        let replacement = match worker_result {
            Ok(()) => read_json_file::<VoiceCloneReplaceWorkerResult>(
                &thread_output_json,
                "voice_clone_replace_failed",
                "固定话术替换结果 JSON 无法解析",
            )
            .and_then(|result| {
                validate_voice_clone_replace_result(
                    &thread_replacement_root,
                    &thread_plan,
                    &thread_ffprobe_path,
                    result,
                )
            }),
            Err(VoiceCloneWorkerRunError::Cancelled) => {
                if let Ok(mut playback) = playback.lock() {
                    playback.mark_voice_clone_cancelled("用户取消固定话术替换");
                }
                cleanup_voice_clone_paths([
                    thread_request_json.clone(),
                    thread_output_json.clone(),
                    thread_replacement_root.join(&thread_plan.operation_id),
                ]);
                completed_for_thread.store(true, Ordering::Release);
                return;
            }
            Err(VoiceCloneWorkerRunError::Timeout { timeout_ms }) => Err(CommandErrorDto::new(
                "voice_clone_replace_failed",
                format!("固定话术替换超时（{timeout_ms}ms）"),
            )),
            Err(VoiceCloneWorkerRunError::Failed(reason)) => {
                Err(CommandErrorDto::new("voice_clone_replace_failed", reason))
            }
        };
        if let Ok(mut playback) = playback.lock() {
            match replacement {
                Ok(replacement) => {
                    if playback.apply_voice_clone_replacement(replacement).is_err() {
                        cleanup_voice_clone_paths([
                            thread_replacement_root.join(&thread_plan.operation_id)
                        ]);
                    }
                }
                Err(error) => {
                    cleanup_voice_clone_paths([
                        thread_replacement_root.join(&thread_plan.operation_id)
                    ]);
                    playback.mark_voice_clone_failed(error.message);
                }
            }
        } else {
            cleanup_voice_clone_paths([thread_replacement_root.join(&thread_plan.operation_id)]);
        }
        cleanup_voice_clone_paths([thread_request_json, thread_output_json]);
        completed_for_thread.store(true, Ordering::Release);
    });
    if let Err(task) = state.install_voice_clone_worker(VoiceCloneWorkerTask {
        cancellation,
        completed,
        handle,
    }) {
        task.cancellation.cancel();
        let _join_result = task.handle.join();
        cleanup_voice_clone_paths([
            request_json,
            output_json,
            replacement_root.join(&plan.operation_id),
        ]);
        return Err(CommandErrorDto::new(
            "voice_clone_worker_already_running",
            "当前已有固定话术 Worker 在执行",
        ));
    }
    Ok(snapshot)
}

#[tauri::command]
pub fn cancel_voice_clone_operation(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let _ = state.reap_finished_voice_clone_worker()?;
    let had_worker = state.voice_clone_worker_is_running()?;
    state.stop_voice_clone_worker()?;
    state.with_playback(&window, |playback| {
        if had_worker
            || matches!(
                playback.snapshot().voice_clone_replacement.status.as_str(),
                "preparing" | "generating"
            )
        {
            playback.mark_voice_clone_cancelled("用户取消固定话术 Worker");
        }
        Ok(state.snapshot(playback))
    })
}

#[tauri::command]
pub fn clear_voice_clone_replacement(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    state.stop_voice_clone_worker()?;
    let cleanup_paths = {
        let playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        voice_clone_artifact_dirs(&playback, false)
    };
    let snapshot = state.with_playback(&window, |playback| {
        playback.clear_voice_clone_replacement();
        Ok(state.snapshot(playback))
    })?;
    cleanup_voice_clone_paths(cleanup_paths);
    Ok(snapshot)
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
pub fn start_research_analysis(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
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
    let (input_mp4_path, input_mp4_sha256, source_mp4_sha256) = {
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
        let hash = snapshot.current_mp4_sha256.clone().ok_or_else(|| {
            CommandErrorDto::new(
                "current_hash_required",
                "当前视频完整 SHA-256 尚未完成，暂不能执行研究分析",
            )
        })?;
        let source_hash = source.mp4_sha256.clone().ok_or_else(|| {
            CommandErrorDto::new("source_hash_required", "源视频完整 SHA-256 尚未完成")
        })?;
        (PathBuf::from(input_path), hash, source_hash)
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
    let (
        source_path,
        generation,
        video_enabled,
        audio_enabled,
        realtime_audio_enabled,
        source_audio_sample_rate_hz,
    ) = {
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
        let source_hash_ready = snapshot
            .source_media
            .as_ref()
            .is_some_and(|source| source.mp4_hash_status == "ready" && source.mp4_sha256.is_some());
        if !source_hash_ready {
            return Err(CommandErrorDto::new(
                "source_hash_required",
                "源视频完整 SHA-256 尚未完成，暂不能提交媒体处理",
            ));
        }
        (
            source_path,
            snapshot.playback_generation,
            snapshot.video_processing_enabled,
            snapshot.audio_processing_enabled,
            snapshot.realtime_audio_variant_enabled,
            snapshot
                .source_media
                .as_ref()
                .and_then(|source| source.audio_sample_rate_hz),
        )
    };
    if !video_enabled && !audio_enabled {
        return state.with_playback(&window, |playback| Ok(state.snapshot(playback)));
    }
    if realtime_audio_enabled && audio_enabled {
        let supported_runtime = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
            .snapshot()
            .audio_processing_runtime;
        if !supported_runtime {
            state.with_playback(&window, |playback| {
                playback.mark_audio_processing_unavailable(
                    "实时音轨的当前普通声音参数尚未接入运行时 DSP；已保持原音轨，请先恢复默认增益参数",
                );
                Ok(state.snapshot(playback))
            })?;
            if !video_enabled {
                return state.with_playback(&window, |playback| Ok(state.snapshot(playback)));
            }
        }
        if !video_enabled {
            return state.with_playback(&window, |playback| Ok(state.snapshot(playback)));
        }
    }
    let resource_dir = match app.path().resource_dir() {
        Ok(resource_dir) => resource_dir,
        Err(error) => {
            return state.with_playback(&window, |playback| {
                playback.mark_media_processing_failed(format!("媒体资源目录不可用：{error}"));
                Ok(state.snapshot(playback))
            })
        }
    };
    let (ffmpeg_path, ffprobe_path) =
        match configured_media_engine_paths_with_resource_dir(&resource_dir) {
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
        // 实时幻化开启时，普通声音处理由最终效果窗口的音频图作用于当前
        // 原声/候选音轨；这里不能把普通效果提前写入基础视频音轨。
        // 实时音轨的普通处理只允许已实现的运行时增益路径；其余音频参数必须
        // 通过真实 DSP Worker 后再启用，不能静默丢弃。
        audio_processing_enabled: audio_enabled && !realtime_audio_enabled,
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
    let handle = thread::spawn(move || {
        let result = render_media(&media_request, &worker_cancellation);
        if let Ok(mut playback) = playback.lock() {
            if playback.snapshot().playback_generation == generation {
                match result {
                    Ok(rendered) => {
                        let _ = playback.mark_media_processing_ready(
                            generation,
                            rendered.output_mp4_path.display().to_string(),
                            rendered.output_mp4_sha256,
                        );
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

#[tauri::command]
pub fn open_final_effect_window(app: AppHandle) -> Result<FinalEffectWindowDto, CommandErrorDto> {
    if let Some(window) = app.get_webview_window("final-effect") {
        window
            .show()
            .and_then(|_| window.set_focus())
            .map_err(|error| {
                CommandErrorDto::new("final_effect_window_show_failed", error.to_string())
            })?;
        return Ok(FinalEffectWindowDto {
            label: "final-effect".to_owned(),
            created: false,
        });
    }
    WebviewWindowBuilder::new(
        &app,
        "final-effect",
        WebviewUrl::App("index.html?view=final-effect".into()),
    )
    .title("autoLive 最终效果")
    .inner_size(1280.0, 760.0)
    .min_inner_size(320.0, 180.0)
    .resizable(true)
    .center()
    .build()
    .map_err(|error| {
        CommandErrorDto::new("final_effect_window_create_failed", error.to_string())
    })?;
    Ok(FinalEffectWindowDto {
        label: "final-effect".to_owned(),
        created: true,
    })
}

#[tauri::command]
pub fn close_final_effect_window(app: AppHandle) -> Result<bool, CommandErrorDto> {
    let Some(window) = app.get_webview_window("final-effect") else {
        return Ok(false);
    };
    window.close().map_err(|error| {
        CommandErrorDto::new("final_effect_window_close_failed", error.to_string())
    })?;
    Ok(true)
}

#[tauri::command]
pub fn resize_final_effect_window(
    window: Window,
    request: ResizeFinalEffectWindowRequestDto,
) -> Result<FinalEffectWindowSizeDto, CommandErrorDto> {
    if !matches!(window.label(), "main" | "final-effect") {
        return Err(CommandErrorDto::new(
            "playback_window_not_allowed",
            "当前窗口不允许调整播放窗口尺寸",
        ));
    }

    let monitor = window
        .current_monitor()
        .map_err(|error| {
            CommandErrorDto::new("final_effect_monitor_unavailable", error.to_string())
        })?
        .ok_or_else(|| {
            CommandErrorDto::new(
                "final_effect_monitor_unavailable",
                "无法读取当前播放窗口所在显示器",
            )
        })?;
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
    state.stop_voice_clone_worker()?;
    state.stop_media_worker()?;
    state.stop_research_worker()?;
    let cleanup_paths = {
        let playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        voice_clone_artifact_dirs(&playback, true)
    };
    let snapshot = state.playback_action(&window, |playback| {
        playback.stop();
        Ok(())
    })?;
    cleanup_voice_clone_paths(cleanup_paths);
    Ok(snapshot)
}

#[tauri::command]
pub fn complete_playback_loop(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.stop_speech_worker()?;
    state.stop_voice_clone_worker()?;
    let cleanup_paths = {
        let playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        voice_clone_artifact_dirs(&playback, false)
    };
    let snapshot = state.playback_action(&window, PlaybackCore::complete_loop)?;
    cleanup_voice_clone_paths(cleanup_paths);
    Ok(snapshot)
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
    state.stop_media_worker()?;
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
    state: State<'_, AppState>,
    request: StartSpeechToSpeechRequestDto,
) -> Result<SpeechToSpeechStartResultDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let _ = state.reap_finished_speech_worker()?;
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
    let handle = thread::spawn(move || {
        let result =
            run_configured_speech_to_speech_context_worker(&thread_context, &worker_cancellation);
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
                        if matches!(error, SpeechToSpeechWorkerError::Cancelled) {
                            playback.mark_speech_to_speech_worker_cancelled(error.to_string());
                        } else {
                            playback.fallback_audio_runtime(error.to_string());
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum VoiceCloneWorkerRunError {
    Cancelled,
    Timeout { timeout_ms: u64 },
    Failed(String),
}

fn unavailable_voice_clone_capabilities(
    reason: impl Into<String>,
) -> VoiceCloneWorkerCapabilitiesDto {
    VoiceCloneWorkerCapabilitiesDto {
        available: false,
        status: "unavailable".to_owned(),
        provider: None,
        model: None,
        reason: Some(reason.into()),
    }
}

fn configured_voice_clone_worker_executable() -> Result<PathBuf, String> {
    let configured = std::env::var(VOICE_CLONE_WORKER_ENV)
        .map_err(|_| format!("未设置 {VOICE_CLONE_WORKER_ENV}"))?;
    let trimmed = configured.trim();
    if trimmed.is_empty() {
        return Err(format!("{VOICE_CLONE_WORKER_ENV} 不能为空"));
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err(format!("{VOICE_CLONE_WORKER_ENV} 必须是绝对路径"));
    }
    if !path.is_file() {
        return Err("固定话术 Worker 文件不存在".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = path
            .metadata()
            .map_err(|error| format!("无法读取固定话术 Worker 权限：{error}"))?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err("固定话术 Worker 文件不可执行，请先赋予执行权限".to_owned());
        }
    }
    Ok(path)
}

fn voice_clone_cache_root(app: &AppHandle) -> Result<PathBuf, CommandErrorDto> {
    let cache_root = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandErrorDto::new("voice_clone_cache_dir_failed", error.to_string()))?
        .join("voice-clone");
    std::fs::create_dir_all(&cache_root)
        .map_err(|error| CommandErrorDto::new("voice_clone_cache_dir_failed", error.to_string()))?;
    Ok(cache_root)
}

fn voice_clone_operation_id(prefix: &str, generation: u64) -> Result<String, CommandErrorDto> {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| {
            CommandErrorDto::new("voice_clone_operation_id_failed", error.to_string())
        })?
        .as_nanos();
    Ok(format!("{prefix}-g{generation}-{nonce}"))
}

fn write_json_file(path: &Path, value: &impl Serialize) -> Result<(), CommandErrorDto> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            CommandErrorDto::new("voice_clone_json_dir_failed", error.to_string())
        })?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| {
        CommandErrorDto::new("voice_clone_json_serialize_failed", error.to_string())
    })?;
    let partial = path.with_extension("json.partial");
    let mut file = std::fs::File::create(&partial).map_err(|error| {
        CommandErrorDto::new("voice_clone_json_write_failed", error.to_string())
    })?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            CommandErrorDto::new("voice_clone_json_write_failed", error.to_string())
        })?;
    std::fs::rename(&partial, path).map_err(|error| {
        CommandErrorDto::new("voice_clone_json_commit_failed", error.to_string())
    })?;
    Ok(())
}

fn read_json_file<T: DeserializeOwned>(
    path: &Path,
    code: &'static str,
    message: &'static str,
) -> Result<T, CommandErrorDto> {
    let bytes =
        std::fs::read(path).map_err(|error| CommandErrorDto::new(code, error.to_string()))?;
    serde_json::from_slice::<T>(&bytes)
        .map_err(|error| CommandErrorDto::new(code, format!("{message}：{error}")))
}

fn kill_child(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let process_group = format!("-{}", child.id());
        let _kill_group_result = Command::new("/bin/kill")
            .args(["-KILL", process_group.as_str()])
            .status();
    }
    #[cfg(windows)]
    {
        let _kill_tree_result = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .status();
    }
    let _kill_result = child.kill();
    let _wait_result = child.wait();
}

fn run_voice_clone_worker_process(
    executable: &Path,
    args: &[String],
    timeout_ms: u64,
    cancellation: &CancellationToken,
) -> Result<(), VoiceCloneWorkerRunError> {
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|error| VoiceCloneWorkerRunError::Failed(error.to_string()))?;
    let started_at = Instant::now();
    loop {
        if cancellation.is_cancelled() {
            kill_child(&mut child);
            return Err(VoiceCloneWorkerRunError::Cancelled);
        }
        if started_at.elapsed() >= Duration::from_millis(timeout_ms) {
            kill_child(&mut child);
            return Err(VoiceCloneWorkerRunError::Timeout { timeout_ms });
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                }
                let stderr = child
                    .stderr
                    .take()
                    .and_then(|mut stream| {
                        use std::io::Read;
                        let mut content = String::new();
                        match stream.read_to_string(&mut content) {
                            Ok(_) => Some(content),
                            Err(_) => None,
                        }
                    })
                    .unwrap_or_default();
                let message = if stderr.trim().is_empty() {
                    format!("固定话术 Worker 退出码异常：{status}")
                } else {
                    stderr.trim().to_owned()
                };
                return Err(VoiceCloneWorkerRunError::Failed(message));
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(error) => {
                kill_child(&mut child);
                return Err(VoiceCloneWorkerRunError::Failed(error.to_string()));
            }
        }
    }
}

fn cleanup_path(path: &Path) {
    if !path.exists() {
        return;
    }
    if path.is_dir() {
        let _ignored = std::fs::remove_dir_all(path);
    } else {
        let _ignored = std::fs::remove_file(path);
    }
}

fn cleanup_voice_clone_paths(paths: impl IntoIterator<Item = PathBuf>) {
    let mut unique = Vec::<PathBuf>::new();
    for path in paths {
        if !unique.iter().any(|existing| existing == &path) {
            unique.push(path);
        }
    }
    for path in unique {
        cleanup_path(&path);
    }
}

fn voice_clone_artifact_dirs(playback: &PlaybackCore, include_prepared: bool) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if include_prepared {
        if let Some(prepared) = playback.voice_clone_prepared_source() {
            if let Some(parent) = Path::new(&prepared.reference_audio_path).parent() {
                paths.push(parent.to_path_buf());
            }
        }
    }
    let snapshot = playback.snapshot();
    if let Some(reference) = snapshot
        .voice_clone_replacement
        .replacement_audio_reference
        .as_deref()
    {
        if let Some(parent) = Path::new(reference).parent() {
            paths.push(parent.to_path_buf());
        }
    }
    paths
}

fn realtime_audio_is_occupying(snapshot: &PlaybackSnapshot) -> bool {
    snapshot.worker_status == "running"
        || snapshot.current_audio_source.as_deref() == Some("realtime_variant")
        || snapshot.pending_audio_candidate
}

fn canonicalize_file_within_root(
    root: &Path,
    path: &str,
    code: &'static str,
    message: &'static str,
) -> Result<PathBuf, CommandErrorDto> {
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|error| CommandErrorDto::new(code, error.to_string()))?;
    let canonical_path = std::fs::canonicalize(path)
        .map_err(|error| CommandErrorDto::new(code, format!("{message}：{error}")))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(CommandErrorDto::new(code, message));
    }
    Ok(canonical_path)
}

fn voice_clone_hash_matches(
    path: &Path,
    expected_sha256: &str,
    code: &'static str,
    mismatch_message: &'static str,
) -> Result<String, CommandErrorDto> {
    let actual_sha256 = hash_file_at_path(path, &CancellationToken::new())
        .map_err(|error| CommandErrorDto::new(code, error.to_string()))?;
    if actual_sha256 != expected_sha256 {
        return Err(CommandErrorDto::new(code, mismatch_message));
    }
    Ok(actual_sha256)
}

fn probe_voice_clone_audio(
    ffprobe_path: &Path,
    audio_path: &Path,
) -> Result<(u32, u16, u64), CommandErrorDto> {
    let mut child = Command::new(ffprobe_path);
    child
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=sample_rate,channels,duration",
            "-show_entries",
            "format=duration",
            "-of",
            "json",
        ])
        .arg(audio_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = child.spawn().map_err(|error| {
        CommandErrorDto::new(
            "voice_clone_replace_failed",
            format!("无法探测替换音频：{error}"),
        )
    })?;
    let started_at = Instant::now();
    let status = loop {
        if started_at.elapsed() >= Duration::from_millis(VOICE_CLONE_AUDIO_PROBE_TIMEOUT_MS) {
            kill_child(&mut child);
            return Err(CommandErrorDto::new(
                "voice_clone_replace_failed",
                "替换音频可读性探测超时",
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                kill_child(&mut child);
                return Err(CommandErrorDto::new(
                    "voice_clone_replace_failed",
                    format!("替换音频探测失败：{error}"),
                ));
            }
        }
    };
    let mut output = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        use std::io::Read;
        stdout.read_to_end(&mut output).map_err(|error| {
            CommandErrorDto::new(
                "voice_clone_replace_failed",
                format!("替换音频探测结果读取失败：{error}"),
            )
        })?;
    }
    if !status.success() {
        return Err(CommandErrorDto::new(
            "voice_clone_replace_failed",
            "替换音频文件不可读",
        ));
    }
    let payload: serde_json::Value = serde_json::from_slice(&output).map_err(|error| {
        CommandErrorDto::new(
            "voice_clone_replace_failed",
            format!("替换音频探测结果无效：{error}"),
        )
    })?;
    let stream = payload
        .get("streams")
        .and_then(serde_json::Value::as_array)
        .and_then(|streams| streams.first())
        .ok_or_else(|| CommandErrorDto::new("voice_clone_replace_failed", "替换音频缺少音频流"))?;
    let sample_rate_hz = stream
        .get("sample_rate")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or_default();
    let channel_count = stream
        .get("channels")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or_default();
    let duration_seconds = stream
        .get("duration")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| {
            payload
                .get("format")
                .and_then(|format| format.get("duration"))
                .and_then(serde_json::Value::as_str)
                .and_then(|value| value.parse::<f64>().ok())
        })
        .unwrap_or_default();
    let duration_ms = if duration_seconds.is_finite() && duration_seconds > 0.0 {
        (duration_seconds * 1_000.0).round() as u64
    } else {
        0
    };
    if sample_rate_hz == 0 || channel_count == 0 || duration_ms == 0 {
        return Err(CommandErrorDto::new(
            "voice_clone_replace_failed",
            "替换音频采样率、声道或时长无效",
        ));
    }
    Ok((sample_rate_hz, channel_count, duration_ms))
}

fn validate_voice_clone_prepare_result(
    prepared_root: &Path,
    source_generation: u64,
    canonical_source_path: &Path,
    source_sha256: &str,
    operation_id: &str,
    worker_result: VoiceClonePrepareWorkerResult,
) -> Result<VoiceClonePreparedSource, CommandErrorDto> {
    if worker_result.status != "success" {
        return Err(CommandErrorDto::new(
            "voice_clone_prepare_failed",
            worker_result
                .reason
                .unwrap_or_else(|| "固定话术准备失败".to_owned()),
        ));
    }
    if worker_result.operation_id != operation_id {
        return Err(CommandErrorDto::new(
            "voice_clone_prepare_failed",
            "固定话术准备返回了过期的操作 ID",
        ));
    }
    let worker_source_path = worker_result.source_path.ok_or_else(|| {
        CommandErrorDto::new("voice_clone_prepare_failed", "固定话术准备缺少源路径")
    })?;
    let canonical_worker_source = std::fs::canonicalize(&worker_source_path).map_err(|_| {
        CommandErrorDto::new("voice_clone_prepare_failed", "固定话术准备源路径无法规范化")
    })?;
    if canonical_worker_source != canonical_source_path {
        return Err(CommandErrorDto::new(
            "voice_clone_prepare_failed",
            "固定话术准备结果不属于当前源视频",
        ));
    }
    let worker_source_sha256 = worker_result.source_sha256.ok_or_else(|| {
        CommandErrorDto::new("voice_clone_prepare_failed", "固定话术准备缺少源哈希")
    })?;
    if worker_source_sha256 != source_sha256 {
        return Err(CommandErrorDto::new(
            "voice_clone_prepare_failed",
            "固定话术准备结果的源哈希与当前源视频不一致",
        ));
    }
    let reference_audio_path = worker_result.reference_audio_path.ok_or_else(|| {
        CommandErrorDto::new("voice_clone_prepare_failed", "固定话术准备缺少参考音频")
    })?;
    if !Path::new(&reference_audio_path).is_absolute() {
        return Err(CommandErrorDto::new(
            "voice_clone_prepare_failed",
            "参考音频路径必须是绝对路径",
        ));
    }
    let reference_audio_sha256 = worker_result.reference_audio_sha256.ok_or_else(|| {
        CommandErrorDto::new("voice_clone_prepare_failed", "固定话术准备缺少参考音频哈希")
    })?;
    let canonical_reference_path = canonicalize_file_within_root(
        prepared_root,
        &reference_audio_path,
        "voice_clone_prepare_failed",
        "参考音频必须位于固定话术缓存目录内",
    )?;
    let actual_reference_sha256 = voice_clone_hash_matches(
        &canonical_reference_path,
        &reference_audio_sha256,
        "voice_clone_prepare_failed",
        "参考音频哈希与当前文件不一致",
    )?;
    let source_index = VoiceCloneSourceIndex {
        source_generation,
        source_path: canonical_source_path.display().to_string(),
        segments: worker_result.segments,
    };
    let sample_rate_hz = worker_result.sample_rate_hz.ok_or_else(|| {
        CommandErrorDto::new("voice_clone_prepare_failed", "固定话术准备缺少采样率")
    })?;
    let channel_count = worker_result.channel_count.ok_or_else(|| {
        CommandErrorDto::new("voice_clone_prepare_failed", "固定话术准备缺少声道数")
    })?;
    let total_duration_ms = worker_result.duration_ms.ok_or_else(|| {
        CommandErrorDto::new("voice_clone_prepare_failed", "固定话术准备缺少总时长")
    })?;
    if total_duration_ms == 0 {
        return Err(CommandErrorDto::new(
            "voice_clone_prepare_failed",
            "固定话术准备总时长必须大于 0",
        ));
    }
    Ok(VoiceClonePreparedSource {
        operation_id: operation_id.to_owned(),
        source_index,
        source_sha256: source_sha256.to_owned(),
        reference_audio_path: canonical_reference_path.display().to_string(),
        reference_audio_sha256: actual_reference_sha256,
        sample_rate_hz,
        channel_count,
        total_duration_ms,
        model: worker_result.model.or(worker_result.provider),
    })
}

fn validate_voice_clone_replace_result(
    replacement_root: &Path,
    plan: &autolive_desktop_core::VoiceCloneReplacementPlan,
    ffprobe_path: &Path,
    worker_result: VoiceCloneReplaceWorkerResult,
) -> Result<VoiceCloneCommittedReplacement, CommandErrorDto> {
    if worker_result.status != "success" {
        return Err(CommandErrorDto::new(
            "voice_clone_replace_failed",
            worker_result
                .reason
                .unwrap_or_else(|| "固定话术替换失败".to_owned()),
        ));
    }
    let replacement_audio_path = worker_result.replacement_audio_path.ok_or_else(|| {
        CommandErrorDto::new("voice_clone_replace_failed", "固定话术替换缺少输出音频")
    })?;
    let replacement_sha256 = worker_result.replacement_sha256.ok_or_else(|| {
        CommandErrorDto::new("voice_clone_replace_failed", "固定话术替换缺少输出音频哈希")
    })?;
    let canonical_replacement_path = canonicalize_file_within_root(
        replacement_root,
        &replacement_audio_path,
        "voice_clone_replace_failed",
        "固定话术输出必须位于固定话术缓存目录内",
    )?;
    let actual_sha256 = voice_clone_hash_matches(
        &canonical_replacement_path,
        &replacement_sha256,
        "voice_clone_replace_failed",
        "固定话术输出哈希与当前文件不一致",
    )?;
    let file_size = std::fs::metadata(&canonical_replacement_path)
        .map_err(|error| CommandErrorDto::new("voice_clone_replace_failed", error.to_string()))?
        .len();
    if file_size < 44 {
        return Err(CommandErrorDto::new(
            "voice_clone_replace_failed",
            "固定话术输出文件过小或不完整",
        ));
    }
    let (actual_sample_rate_hz, actual_channel_count, actual_duration_ms) =
        probe_voice_clone_audio(ffprobe_path, &canonical_replacement_path)?;
    let source_generation = worker_result.source_generation.ok_or_else(|| {
        CommandErrorDto::new("voice_clone_replace_failed", "固定话术替换缺少源代际")
    })?;
    let source_path = worker_result.source_path.ok_or_else(|| {
        CommandErrorDto::new("voice_clone_replace_failed", "固定话术替换缺少源路径")
    })?;
    let reported_duration_ms = worker_result.total_duration_ms.ok_or_else(|| {
        CommandErrorDto::new("voice_clone_replace_failed", "固定话术替换缺少总时长")
    })?;
    let replacement_result = VoiceCloneReplacementResult {
        source_generation,
        source_path: source_path.clone(),
        operation_id: worker_result.operation_id.clone(),
        audio_reference: canonical_replacement_path.display().to_string(),
        audio_sha256: actual_sha256.clone(),
        replacement_duration_ms: reported_duration_ms,
    };
    let replacement_request = VoiceCloneReplacementRequest {
        source_generation: plan.source_generation,
        source_path: plan.source_path.clone(),
        source_sha256: plan.source_sha256.clone(),
        audio_base_path: plan.audio_base_path.clone(),
        source_duration_ms: plan.source_duration_ms,
        sample_rate_hz: plan.sample_rate_hz,
        channel_count: plan.channel_count,
        operation_id: plan.operation_id.clone(),
        reference_audio_path: plan.reference_audio_path.clone(),
        text: plan.input_text.clone(),
        replace_at_ms: plan.replace_at_ms,
        resume_at_ms: plan.resume_at_ms,
    };
    validate_replacement_result(&replacement_result, &replacement_request)
        .map_err(|error| CommandErrorDto::new("voice_clone_replace_failed", error.to_string()))?;
    if worker_result.source_sha256.as_deref() != Some(plan.source_sha256.as_str())
        || worker_result.replace_at_ms != Some(plan.replace_at_ms)
        || worker_result.resume_at_ms != Some(plan.resume_at_ms)
        || worker_result.sample_rate_hz != Some(plan.sample_rate_hz)
        || worker_result.channel_count != Some(plan.channel_count)
        || worker_result.input_text.as_deref() != Some(plan.input_text.as_str())
    {
        return Err(CommandErrorDto::new(
            "voice_clone_replace_failed",
            "固定话术替换结果与当前请求上下文不一致",
        ));
    }
    if actual_sample_rate_hz != plan.sample_rate_hz
        || actual_channel_count != plan.channel_count
        || actual_duration_ms.abs_diff(reported_duration_ms) > 100
        || actual_duration_ms.abs_diff(plan.source_duration_ms) > 1_000
    {
        return Err(CommandErrorDto::new(
            "voice_clone_replace_failed",
            "固定话术输出的实际音频参数与请求不一致",
        ));
    }
    Ok(VoiceCloneCommittedReplacement {
        source_generation,
        source_path,
        operation_id: worker_result.operation_id,
        input_text: plan.input_text.clone(),
        replacement_audio_reference: canonical_replacement_path.display().to_string(),
        replacement_audio_sha256: actual_sha256,
        replacement_duration_ms: replacement_result.replacement_duration_ms,
        replace_at_ms: plan.replace_at_ms,
        resume_at_ms: plan.resume_at_ms,
        model: worker_result.model.or(worker_result.provider),
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

fn command_error_from_voice_clone_runtime(error: VoiceCloneRuntimeError) -> CommandErrorDto {
    let (code, message) = match error {
        VoiceCloneRuntimeError::SourceMediaRequired => {
            ("source_media_required", "请先导入一个源视频".to_owned())
        }
        VoiceCloneRuntimeError::VoiceCloneTextInvalid(error) => {
            ("voice_clone_text_invalid", error.to_string())
        }
        VoiceCloneRuntimeError::PlaybackNotPlaying => (
            "voice_clone_playback_not_playing",
            "固定话术替换只允许在播放中启动".to_owned(),
        ),
        VoiceCloneRuntimeError::VoiceCloneSourceNotPrepared => (
            "voice_clone_source_not_prepared",
            "请先准备当前源视频的人声参考".to_owned(),
        ),
        VoiceCloneRuntimeError::VoiceClonePreparedSourceStale => (
            "voice_clone_source_stale",
            "当前固定话术参考已经过期，请重新准备".to_owned(),
        ),
        VoiceCloneRuntimeError::CurrentVoiceSegmentNotFound => (
            "voice_clone_segment_not_found",
            "当前位置没有可替换的话术片段".to_owned(),
        ),
        VoiceCloneRuntimeError::VoiceCloneAudioProcessingNotReady => (
            "voice_clone_audio_processing_not_ready",
            "请先应用当前普通声音处理参数，再替换固定话术".to_owned(),
        ),
        VoiceCloneRuntimeError::RealtimeAudioWorkerBusy => (
            "voice_clone_realtime_audio_busy",
            "实时音频候选正在占用音频，当前不能启动固定话术替换".to_owned(),
        ),
        VoiceCloneRuntimeError::VoiceCloneReplacementStale => (
            "voice_clone_replacement_stale",
            "固定话术结果已经过期，未应用到当前播放".to_owned(),
        ),
    };
    CommandErrorDto::new(code, message)
}

fn command_error_from_media_library(error: MediaLibraryError) -> CommandErrorDto {
    CommandErrorDto::new("media_probe_failed", error.to_string())
}

fn command_error_from_file_hash(error: FileHashError) -> CommandErrorDto {
    CommandErrorDto::new("file_hash_failed", error.to_string())
}

fn command_error_from_candidate(error: CandidateValidationError) -> CommandErrorDto {
    CommandErrorDto::new("audio_variant_candidate_rejected", error.to_string())
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
    use super::AppState;

    #[test]
    fn state_starts_without_a_source_or_queue() {
        let state = AppState::default();
        let playback = state.playback.lock().expect("playback lock");
        let snapshot = playback.snapshot();
        assert!(snapshot.source_media.is_none());
        assert_eq!(snapshot.loop_index, 0);
    }
}
