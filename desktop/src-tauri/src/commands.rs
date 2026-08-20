use crate::audio_cycle_switch::{
    AudioCycleCandidate, AudioMixerSourceIdentity, PendingAudioMixerKind, PendingAudioMixerTask,
};
use autolive_desktop_core::audio_cycle_output::{
    AudioCycleOutputControl, AudioCycleOutputTask, AudioOutputConfig, AudioTestTone,
    AudioTrackTimeline,
};
use autolive_desktop_core::audio_mixer::{AudioMixerTask, AUDIO_CANDIDATE_COMMIT_TAIL_MS};
use autolive_desktop_core::audio_output_diagnostic::AudioLowFrequencyDiagnosticSnapshot;
use autolive_desktop_core::audio_output_health::{
    evaluate_output_health, observe_counter, output_resume_required, OutputHealthInput,
};
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
    build_audio_stream_filter_graph, build_media_render_args,
    configured_media_engine_paths_with_resource_dir,
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
use autolive_desktop_core::{
    PlaybackCore, PlaybackSnapshot, PlaybackState, ValidatedAudioStreamConfiguration,
};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
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
const PORTAUDIO_CALLBACK_STALL_MS: u64 = 1_500;
const PORTAUDIO_PCM_STALL_MS: u64 = 1_500;
// FFmpeg 的 loudnorm + 多支路滤镜需要完成初始化后才会输出首批 PCM。
const PORTAUDIO_SWITCH_READY_TIMEOUT_MS: u64 = 5_000;
const PORTAUDIO_SWITCH_READY_POLL_MS: u64 = 10;
const AUDIO_CYCLE_CANDIDATE_PRE_ROLL_MS: u64 = 250;
const AUDIO_CYCLE_CANDIDATE_BUFFER_MS: usize = 750;
const AUDIO_CYCLE_TARGET_HORIZON_MS: u64 = 60_000;
const AUDIO_OUTPUT_RESUME_REQUIRED_CODE: &str = "audio_output_resume_required";

fn scheduled_candidate_commit_tail_ms(playback_watermark_ms: u64) -> usize {
    usize::try_from(playback_watermark_ms)
        .unwrap_or(AUDIO_CYCLE_CANDIDATE_BUFFER_MS)
        .clamp(
            AUDIO_CANDIDATE_COMMIT_TAIL_MS,
            AUDIO_CYCLE_CANDIDATE_BUFFER_MS,
        )
}

fn is_retryable_audio_mixer_error(code: &str) -> bool {
    matches!(
        code,
        "audio_mixer_candidate_not_caught_up"
            | "audio_mixer_candidate_superseded"
            | "audio_mixer_candidate_stale"
            | "audio_mixer_start_stale"
    )
}

fn audio_cycle_cancel_matches_pending(
    pending_is_audio_cycle: bool,
    pending_candidate_id: Option<u64>,
    requested_candidate_id: Option<u64>,
) -> bool {
    pending_is_audio_cycle
        && requested_candidate_id
            .is_none_or(|candidate_id| pending_candidate_id == Some(candidate_id))
}

fn take_pending_audio_mixer<T>(slot: &Mutex<Option<T>>) -> Result<Option<T>, CommandErrorDto> {
    slot.lock()
        .map_err(|_| {
            CommandErrorDto::new(
                "audio_mixer_pending_lock_failed",
                "待切换音频混音状态锁已损坏",
            )
        })
        .map(|mut pending| pending.take())
}

fn add_wall_clock_delay_to_media_position_ms(
    media_position_ms: u64,
    wall_clock_delay_ms: u64,
    playback_rate: f64,
) -> u64 {
    let playback_rate = if playback_rate.is_finite() && playback_rate > 0.0 {
        playback_rate
    } else {
        1.0
    };
    let media_delay_ms = (wall_clock_delay_ms as f64 * playback_rate)
        .round()
        .clamp(0.0, u64::MAX as f64) as u64;
    media_position_ms.saturating_add(media_delay_ms)
}

fn resolve_audio_commit_position_ms(
    requested_position_ms: Option<u64>,
    current_position_ms: u64,
    preparation_elapsed_ms: u64,
    playback_rate: f64,
    pending_ms: u64,
    output_latency_ms: u64,
) -> u64 {
    let requested_now_ms = requested_position_ms.map(|position_ms| {
        add_wall_clock_delay_to_media_position_ms(
            position_ms,
            preparation_elapsed_ms,
            playback_rate,
        )
    });
    add_wall_clock_delay_to_media_position_ms(
        requested_now_ms
            .unwrap_or(current_position_ms)
            .max(current_position_ms),
        pending_ms.saturating_add(output_latency_ms),
        playback_rate,
    )
}

fn resolve_audio_output_latency_ms(
    output_latency_us: Option<u64>,
    callback_dac_lead_us: i64,
) -> u64 {
    let callback_dac_lead_us = u64::try_from(callback_dac_lead_us).unwrap_or(0);
    output_latency_us
        .unwrap_or(0)
        .max(callback_dac_lead_us)
        .saturating_add(999)
        .saturating_div(1_000)
}

fn resolve_audio_candidate_pcm_position_ms(
    candidate_start_position_ms: u64,
    media_position_ms: u64,
    playback_rate: f64,
) -> u64 {
    let playback_rate = if playback_rate.is_finite() && playback_rate > 0.0 {
        playback_rate
    } else {
        1.0
    };
    let media_elapsed_ms = media_position_ms.saturating_sub(candidate_start_position_ms);
    let pcm_elapsed_ms = (media_elapsed_ms as f64 / playback_rate)
        .round()
        .clamp(0.0, u64::MAX as f64) as u64;
    candidate_start_position_ms.saturating_add(pcm_elapsed_ms)
}

fn elapsed_millis(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn source_duration_ms(snapshot: &PlaybackSnapshot) -> Result<u64, CommandErrorDto> {
    snapshot
        .source_media
        .as_ref()
        .and_then(|source| source.duration_ms)
        .filter(|duration_ms| *duration_ms > 0)
        .ok_or_else(|| {
            CommandErrorDto::new(
                "audio_candidate_source_duration_missing",
                "源视频时长不可用，无法建立候选音轨绝对时间轴",
            )
        })
}

fn absolute_media_position_ms(loop_index: u64, position_ms: u64, duration_ms: u64) -> u64 {
    loop_index
        .saturating_mul(duration_ms)
        .saturating_add(position_ms.min(duration_ms))
}

fn signed_millis_delta(left_ms: u64, right_ms: u64) -> i64 {
    if left_ms >= right_ms {
        i64::try_from(left_ms - right_ms).unwrap_or(i64::MAX)
    } else {
        -i64::try_from(right_ms - left_ms).unwrap_or(i64::MAX)
    }
}

#[derive(Debug, Clone)]
pub struct AppState {
    main_window_label: &'static str,
    playback: Arc<Mutex<PlaybackCore>>,
    /// 最终效果窗口最近一次上报视频 currentTime 的单调近似墙钟时间。
    playback_position_observed_at: Arc<Mutex<Option<Instant>>>,
    speech_worker: Arc<Mutex<Option<SpeechWorkerTask>>>,
    media_worker: Arc<Mutex<Option<MediaWorkerTask>>>,
    research_worker: Arc<Mutex<Option<ResearchWorkerTask>>>,
    research_status: Arc<Mutex<ResearchStatusDto>>,
    runtime_resource_task: Arc<RuntimeResourceTask>,
    /// 按值拥有 PortAudio 流和唯一环缓生产线程；None = WebView。
    audio_cycle_output: Arc<Mutex<Option<AudioCycleOutputTask>>>,
    /// FFmpeg 解码线程 → 音频混音线程；PortAudio 失败时整体停止并回退 WebView。
    audio_mixer: Arc<Mutex<Option<AudioMixerTask>>>,
    /// 尚未提交的候选音轨；预热期间不占用当前音轨槽位，停止/暂停可取消并 Join。
    audio_mixer_pending: Arc<Mutex<Option<PendingAudioMixerTask>>>,
    /// 所有音轨生命周期操作共享的代次；停止、暂停、重新同步或新预热会使旧操作失效。
    audio_mixer_pending_token: Arc<AtomicU64>,
    /// 只保护 current/pending 槽位的短事务；禁止在持锁期间等待 FFmpeg、PortAudio 或 Join。
    audio_mixer_switch_lock: Arc<Mutex<()>>,
    /// 仅串行化候选 FFmpeg 的创建，避免两个并发请求短暂生成多个候选进程。
    audio_mixer_prepare_lock: Arc<Mutex<()>>,
    /// 普通源同步和 PCM 恢复全程保留独占身份，避免 N+1 prepare/cancel 推进其 token。
    audio_mixer_recovery_in_progress: Arc<AtomicBool>,
    /// 防止新 prepare/cancel 在 output 已完成 crossfade、状态槽尚未切换时使候选失效。
    audio_cycle_commit_in_progress: Arc<AtomicBool>,
    audio_output_preferred: Arc<Mutex<bool>>,
    audio_output_reconfiguring: Arc<AtomicBool>,
    /// 轮询时用回调计数确认 PortAudio 仍在实际推进，而不是只看对象是否存在。
    audio_output_last_callback_count: Arc<AtomicU64>,
    audio_output_last_callback_progress_ms: Arc<AtomicU64>,
    /// 回调会在缺少 PCM 时继续补零，必须单独观察实际消费的 PCM frame。
    audio_output_last_pcm_frame_count: Arc<AtomicU64>,
    audio_output_last_pcm_progress_ms: Arc<AtomicU64>,
}

#[derive(Debug)]
struct AudioCycleCommitGuard {
    in_progress: Arc<AtomicBool>,
}

impl Drop for AudioCycleCommitGuard {
    fn drop(&mut self) {
        self.in_progress.store(false, Ordering::Release);
    }
}

#[derive(Debug)]
struct AudioMixerRecoveryGuard {
    in_progress: Arc<AtomicBool>,
}

impl Drop for AudioMixerRecoveryGuard {
    fn drop(&mut self) {
        self.in_progress.store(false, Ordering::Release);
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AudioSyncClock {
    playback_generation: u64,
    loop_index: u64,
    position_ms: u64,
    duration_ms: u64,
    absolute_position_ms: u64,
}

fn resolve_audio_sync_clock(
    requested: AudioSyncClock,
    playback_generation: u64,
    loop_index: u64,
    current_position_ms: u64,
    duration_ms: u64,
) -> Result<AudioSyncClock, &'static str> {
    if duration_ms == 0 || requested.duration_ms != duration_ms {
        return Err("视频时长已经变化");
    }
    if requested.playback_generation != playback_generation {
        return Err("播放代次已经变化");
    }
    if requested.loop_index != loop_index {
        return Err("视频循环代次已经变化");
    }
    if requested.position_ms > duration_ms
        || requested.absolute_position_ms
            != absolute_media_position_ms(requested.loop_index, requested.position_ms, duration_ms)
    {
        return Err("绝对媒体时间与循环位置不一致");
    }
    let current_absolute_position_ms =
        absolute_media_position_ms(loop_index, current_position_ms, duration_ms);
    if requested.absolute_position_ms.saturating_add(1_000) < current_absolute_position_ms {
        return Err("绝对媒体时间已经过期");
    }
    Ok(requested)
}

fn snapshot_audio_sync_clock(
    snapshot: &PlaybackSnapshot,
) -> Result<AudioSyncClock, CommandErrorDto> {
    let duration_ms = source_duration_ms(snapshot)?;
    Ok(AudioSyncClock {
        playback_generation: snapshot.playback_generation,
        loop_index: snapshot.loop_index,
        position_ms: snapshot.current_position_ms.min(duration_ms),
        duration_ms,
        absolute_position_ms: absolute_media_position_ms(
            snapshot.loop_index,
            snapshot.current_position_ms,
            duration_ms,
        ),
    })
}

fn audio_cycle_commit_due(
    current_absolute_position_ms: u64,
    target_absolute_position_ms: u64,
    pending_ms: u64,
    output_latency_ms: u64,
    playback_rate: f64,
) -> bool {
    add_wall_clock_delay_to_media_position_ms(
        current_absolute_position_ms,
        pending_ms.saturating_add(output_latency_ms),
        playback_rate,
    ) >= target_absolute_position_ms
}

fn should_complete_playback_loop(
    current_generation: u64,
    current_loop_index: u64,
    requested_generation: u64,
    target_loop_index: u64,
) -> Result<bool, &'static str> {
    if current_generation != requested_generation {
        return Err("播放代次已经变化，旧循环完成消息已拒绝");
    }
    if target_loop_index <= current_loop_index {
        return Ok(false);
    }
    if target_loop_index != current_loop_index.saturating_add(1) {
        return Err("目标循环序号必须紧邻当前循环序号");
    }
    Ok(true)
}

fn should_defer_source_sync_for_pending_candidate(
    pending_in_progress: bool,
    recover_unhealthy: bool,
    _reanchor_loop_boundary: bool,
) -> bool {
    pending_in_progress && !recover_unhealthy
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompletePlaybackLoopRequestDto {
    pub playback_generation: u64,
    pub target_loop_index: u64,
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
            playback_position_observed_at: Arc::new(Mutex::new(None)),
            speech_worker: Arc::new(Mutex::new(None)),
            media_worker: Arc::new(Mutex::new(None)),
            research_worker: Arc::new(Mutex::new(None)),
            research_status: Arc::new(Mutex::new(ResearchStatusDto::default())),
            runtime_resource_task: Arc::new(RuntimeResourceTask::default()),
            audio_cycle_output: Arc::new(Mutex::new(None)),
            audio_mixer: Arc::new(Mutex::new(None)),
            audio_mixer_pending: Arc::new(Mutex::new(None)),
            audio_mixer_pending_token: Arc::new(AtomicU64::new(0)),
            audio_mixer_switch_lock: Arc::new(Mutex::new(())),
            audio_mixer_prepare_lock: Arc::new(Mutex::new(())),
            audio_mixer_recovery_in_progress: Arc::new(AtomicBool::new(false)),
            audio_cycle_commit_in_progress: Arc::new(AtomicBool::new(false)),
            audio_output_preferred: Arc::new(Mutex::new(true)),
            audio_output_reconfiguring: Arc::new(AtomicBool::new(false)),
            audio_output_last_callback_count: Arc::new(AtomicU64::new(0)),
            audio_output_last_callback_progress_ms: Arc::new(AtomicU64::new(0)),
            audio_output_last_pcm_frame_count: Arc::new(AtomicU64::new(0)),
            audio_output_last_pcm_progress_ms: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl AppState {
    fn next_audio_mixer_pending_token(&self) -> u64 {
        let previous = self
            .audio_mixer_pending_token
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                Some(current.wrapping_add(1).max(1))
            })
            .unwrap_or_else(|current| current);
        previous.wrapping_add(1).max(1)
    }

    fn audio_mixer_operation_is_current(&self, token: u64) -> bool {
        self.audio_mixer_pending_token.load(Ordering::Acquire) == token
    }

    fn begin_audio_mixer_prepare(
        &self,
    ) -> Result<(u64, Option<PendingAudioMixerTask>), CommandErrorDto> {
        self.begin_audio_mixer_prepare_inner(false)
    }

    fn begin_audio_source_sync_prepare(
        &self,
    ) -> Result<(u64, Option<PendingAudioMixerTask>), CommandErrorDto> {
        self.begin_audio_mixer_prepare_inner(true)
    }

    fn begin_audio_mixer_prepare_inner(
        &self,
        source_sync_owns_recovery_guard: bool,
    ) -> Result<(u64, Option<PendingAudioMixerTask>), CommandErrorDto> {
        let _switch_guard = self.audio_mixer_switch_lock.lock().map_err(|_| {
            CommandErrorDto::new("audio_mixer_switch_lock_failed", "音频切换锁已损坏")
        })?;
        if !source_sync_owns_recovery_guard
            && self
                .audio_mixer_recovery_in_progress
                .load(Ordering::Acquire)
        {
            return Err(CommandErrorDto::new(
                "audio_candidate_recovery_in_progress",
                "PortAudio 音频源正在同步，N+1 候选将在同步完成后重新准备",
            ));
        }
        if self.audio_cycle_commit_in_progress.load(Ordering::Acquire) {
            return Err(CommandErrorDto::new(
                "audio_cycle_commit_in_progress",
                "音轨正在完成交叉淡化，请稍后准备下一候选",
            ));
        }
        let operation_token = self.next_audio_mixer_pending_token();
        let previous = take_pending_audio_mixer(&self.audio_mixer_pending)?;
        Ok((operation_token, previous))
    }

    fn begin_audio_cycle_commit(&self) -> Result<AudioCycleCommitGuard, CommandErrorDto> {
        let _switch_guard = self.audio_mixer_switch_lock.lock().map_err(|_| {
            CommandErrorDto::new("audio_mixer_switch_lock_failed", "音频切换锁已损坏")
        })?;
        self.audio_cycle_commit_in_progress
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                CommandErrorDto::new(
                    "audio_cycle_commit_in_progress",
                    "已有候选音轨正在完成交叉淡化",
                )
            })?;
        Ok(AudioCycleCommitGuard {
            in_progress: Arc::clone(&self.audio_cycle_commit_in_progress),
        })
    }

    fn begin_audio_mixer_recovery(&self) -> Result<AudioMixerRecoveryGuard, CommandErrorDto> {
        self.audio_mixer_recovery_in_progress
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                CommandErrorDto::new(
                    "audio_mixer_recovery_in_progress",
                    "PortAudio 音频源同步或 PCM 恢复正在处理最新请求",
                )
            })?;
        Ok(AudioMixerRecoveryGuard {
            in_progress: Arc::clone(&self.audio_mixer_recovery_in_progress),
        })
    }

    pub fn shutdown_runtime_resources(
        &self,
        budget: Duration,
    ) -> Result<RuntimeResourceTaskShutdown, String> {
        self.stop_audio_mixer().map_err(|error| error.message)?;
        self.stop_audio_cycle_output()
            .map_err(|error| error.message)?;
        self.runtime_resource_task.shutdown(budget)
    }

    fn audio_output_control(&self) -> Result<AudioCycleOutputControl, CommandErrorDto> {
        self.audio_cycle_output
            .lock()
            .map_err(|_| {
                CommandErrorDto::new("audio_cycle_output_lock_failed", "音频周期输出状态锁已损坏")
            })?
            .as_ref()
            .map(AudioCycleOutputTask::control)
            .ok_or_else(|| {
                CommandErrorDto::new("audio_cycle_output_missing", "音频周期输出线程尚未启动")
            })
    }

    fn take_audio_cycle_output_task(
        &self,
    ) -> Result<Option<AudioCycleOutputTask>, CommandErrorDto> {
        self.audio_cycle_output
            .lock()
            .map_err(|_| {
                CommandErrorDto::new("audio_cycle_output_lock_failed", "音频周期输出状态锁已损坏")
            })
            .map(|mut output| output.take())
    }

    fn stop_audio_cycle_output(&self) -> Result<(), CommandErrorDto> {
        if let Some(task) = self.take_audio_cycle_output_task()? {
            task.shutdown()
                .map_err(|error| CommandErrorDto::new("audio_cycle_output_stop_failed", error))?;
        }
        Ok(())
    }

    fn stop_audio_mixer(&self) -> Result<(), CommandErrorDto> {
        self.next_audio_mixer_pending_token();
        let (current, pending) = {
            let _switch_guard = self.audio_mixer_switch_lock.lock().map_err(|_| {
                CommandErrorDto::new("audio_mixer_switch_lock_failed", "音频切换锁已损坏")
            })?;
            let pending = take_pending_audio_mixer(&self.audio_mixer_pending)?;
            let current = self
                .audio_mixer
                .lock()
                .map_err(|_| {
                    CommandErrorDto::new("audio_mixer_lock_failed", "音频混音状态锁已损坏")
                })?
                .take();
            (current, pending)
        };
        if let Some(mut pending) = pending {
            pending.stop_preserving_output();
        }
        if let Some(mut current) = current {
            current.stop();
        }
        Ok(())
    }

    fn pause_audio_output(&self) -> Result<(), CommandErrorDto> {
        self.next_audio_mixer_pending_token();
        let pause_result = self.audio_output_control().and_then(|control| {
            control
                .pause()
                .map_err(|error| CommandErrorDto::new("audio_output_pause_failed", error))
        });
        let pending = {
            let _switch_guard = self.audio_mixer_switch_lock.lock().map_err(|_| {
                CommandErrorDto::new("audio_mixer_switch_lock_failed", "音频切换锁已损坏")
            })?;
            take_pending_audio_mixer(&self.audio_mixer_pending)?
        };
        if let Some(mut pending) = pending {
            pending.stop_preserving_output();
        }
        pause_result
    }

    fn stop_audio_for_playback(&self) -> Result<(), CommandErrorDto> {
        if let Ok(control) = self.audio_output_control() {
            control
                .clear()
                .map_err(|error| CommandErrorDto::new("audio_output_clear_failed", error))?;
        }
        self.stop_audio_mixer()
    }

    fn resume_audio_output(&self, app: &AppHandle) -> Result<(), CommandErrorDto> {
        let preferred = *self.audio_output_preferred.lock().map_err(|_| {
            CommandErrorDto::new("audio_output_lock_failed", "音频出口状态锁已损坏")
        })?;
        if !preferred {
            return Ok(());
        }
        let control = self.audio_output_control()?;
        let sample_rate_hz = control
            .status()
            .map(|status| status.sample_rate_hz)
            .unwrap_or(autolive_portaudio_output::DEFAULT_SAMPLE_RATE_HZ);
        let mixer_healthy = self
            .audio_mixer
            .lock()
            .map_err(|_| CommandErrorDto::new("audio_mixer_lock_failed", "音频混音状态锁已损坏"))?
            .as_ref()
            .is_some_and(|mixer| mixer.failure().is_none());
        if !mixer_healthy {
            self.stop_audio_mixer()?;
            self.start_audio_mixer_from_snapshot_unlocked(app, sample_rate_hz, None)?;
        }
        control
            .resume()
            .map_err(|error| CommandErrorDto::new("audio_output_resume_failed", error))?;
        Ok(())
    }

    fn audio_output_timing_ms(&self, fallback_sample_rate_hz: u32) -> (u64, u64, u64) {
        self.audio_output_control()
            .ok()
            .and_then(|control| control.status())
            .map(|status| {
                let health = status.health;
                let sample_rate_hz = health
                    .actual_sample_rate_hz
                    .unwrap_or(fallback_sample_rate_hz)
                    .max(1);
                let channels = u64::from(status.channels.max(1));
                let pending_ms = (health.ring_len_samples as u64 / channels)
                    .saturating_mul(1_000)
                    .saturating_div(u64::from(sample_rate_hz));
                let output_latency_ms = resolve_audio_output_latency_ms(
                    health.output_latency_us,
                    health.callback_output_buffer_dac_time_delta_us,
                );
                (pending_ms, output_latency_ms, status.playback_watermark_ms)
            })
            .unwrap_or((0, 0, 0))
    }

    #[allow(clippy::too_many_arguments)]
    fn wait_for_audio_mixer_commit_coverage(
        &self,
        task: &AudioMixerTask,
        sample_rate_hz: u32,
        observed_absolute_position_ms: u64,
        candidate_start_absolute_position_ms: u64,
        preparation_started_at: Instant,
        playback_rate: f64,
        source_identity: &AudioMixerSourceIdentity,
        operation_token: u64,
    ) -> Result<(u64, u64), CommandErrorDto> {
        let timeout = Duration::from_millis(PORTAUDIO_SWITCH_READY_TIMEOUT_MS);
        loop {
            if !self.audio_mixer_operation_is_current(operation_token) {
                return Err(CommandErrorDto::new(
                    "audio_mixer_candidate_stale",
                    "候选追赶期间收到更新的停止、暂停或切换请求",
                ));
            }
            self.ensure_audio_mixer_source_current(source_identity)?;
            if let Some(reason) = task.failure() {
                return Err(CommandErrorDto::new(
                    "audio_mixer_candidate_ffmpeg_failed",
                    format!("候选 FFmpeg 已退出：{reason}"),
                ));
            }

            let current_absolute_position_ms = self
                .playback
                .lock()
                .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))
                .and_then(|playback| snapshot_audio_sync_clock(&playback.snapshot()))?
                .absolute_position_ms;
            let (pending_ms, output_latency_ms, _) = self.audio_output_timing_ms(sample_rate_hz);
            let elapsed_ms = elapsed_millis(preparation_started_at);
            let commit_absolute_position_ms = resolve_audio_commit_position_ms(
                Some(observed_absolute_position_ms),
                current_absolute_position_ms,
                elapsed_ms,
                playback_rate,
                pending_ms,
                output_latency_ms,
            );
            let candidate_pcm_position_ms = resolve_audio_candidate_pcm_position_ms(
                candidate_start_absolute_position_ms,
                commit_absolute_position_ms,
                playback_rate,
            );
            match task.validate_commit_at_position(candidate_pcm_position_ms) {
                Ok(()) => {
                    return Ok((commit_absolute_position_ms, candidate_pcm_position_ms));
                }
                Err(reason) if preparation_started_at.elapsed() >= timeout => {
                    return Err(CommandErrorDto::new(
                        "audio_mixer_candidate_not_caught_up",
                        format!("{reason}；已在同一单调时钟上等待 {elapsed_ms}ms，稍后自动重试"),
                    ));
                }
                Err(_) => thread::sleep(Duration::from_millis(PORTAUDIO_SWITCH_READY_POLL_MS)),
            }
        }
    }

    fn ensure_audio_mixer_source_current(
        &self,
        expected: &AudioMixerSourceIdentity,
    ) -> Result<(), CommandErrorDto> {
        let current = self
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
            .snapshot();
        if !expected.matches_playing(&current) {
            return Err(CommandErrorDto::new(
                "audio_mixer_candidate_superseded",
                format!(
                    "候选预热期间播放代次、媒体源或声音参数已经变化，保持旧轨并等待最新请求；期望 generation/revision={}/{}, 实际={}/{}",
                    expected.playback_generation,
                    expected.audio_stream_revision,
                    current.playback_generation,
                    current.audio_stream_revision,
                ),
            ));
        }
        Ok(())
    }

    fn start_audio_mixer_from_snapshot_unlocked(
        &self,
        app: &AppHandle,
        sample_rate_hz: u32,
        requested_clock: Option<AudioSyncClock>,
    ) -> Result<(), CommandErrorDto> {
        let operation_token = self.next_audio_mixer_pending_token();
        let preparation_started_at = Instant::now();
        let snapshot = self
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
            .snapshot();
        let observed_clock = match requested_clock {
            Some(requested) => resolve_audio_sync_clock(
                requested,
                snapshot.playback_generation,
                snapshot.loop_index,
                snapshot.current_position_ms,
                source_duration_ms(&snapshot)?,
            )
            .map_err(|reason| CommandErrorDto::new("audio_sync_clock_invalid", reason))?,
            None => snapshot_audio_sync_clock(&snapshot)?,
        };
        let has_fresh_ui_position = requested_clock.is_some();
        let (pending_ms, output_latency_ms, playback_watermark_ms) =
            self.audio_output_timing_ms(sample_rate_hz);
        let Some((task, source_identity, playback_rate, candidate_start_absolute_position_ms)) =
            self.audio_mixer_task_from_snapshot(
                app,
                sample_rate_hz,
                observed_clock,
                preparation_started_at,
                pending_ms.saturating_add(output_latency_ms),
                playback_watermark_ms,
            )?
        else {
            return Ok(());
        };
        let mut task = task;
        if !self.audio_mixer_operation_is_current(operation_token) {
            task.stop_preserving_output();
            return Err(CommandErrorDto::new(
                "audio_mixer_start_stale",
                "音轨启动期间收到更新的停止、暂停或切换请求",
            ));
        }
        if let Err(error) = self.ensure_audio_mixer_source_current(&source_identity) {
            task.stop_preserving_output();
            return Err(error);
        }
        let dynamic_observed_position_ms = if has_fresh_ui_position {
            observed_clock.absolute_position_ms
        } else {
            snapshot_audio_sync_clock(&snapshot)?.absolute_position_ms
        };
        let (commit_absolute_position_ms, candidate_pcm_position_ms) = self
            .wait_for_audio_mixer_commit_coverage(
                &task,
                sample_rate_hz,
                dynamic_observed_position_ms,
                candidate_start_absolute_position_ms,
                preparation_started_at,
                playback_rate,
                &source_identity,
                operation_token,
            )?;
        if let Err(error) = task.commit_at_position(candidate_pcm_position_ms) {
            task.stop_preserving_output();
            return Err(CommandErrorDto::new("audio_mixer_commit_failed", error));
        }
        let _commit_guard = self.begin_audio_cycle_commit()?;
        if !self.audio_mixer_operation_is_current(operation_token) {
            task.stop_preserving_output();
            return Err(CommandErrorDto::new(
                "audio_mixer_start_stale",
                "音轨提交前收到更新的停止、暂停或切换请求",
            ));
        }
        if let Err(error) = self.audio_output_control()?.set_current(
            task.output_track(),
            AudioTrackTimeline {
                media_position_ms: commit_absolute_position_ms,
                playback_rate,
            },
        ) {
            task.stop_preserving_output();
            return Err(CommandErrorDto::new(
                "audio_cycle_output_commit_failed",
                error,
            ));
        }
        let old = {
            let _switch_guard = self.audio_mixer_switch_lock.lock().map_err(|_| {
                CommandErrorDto::new("audio_mixer_switch_lock_failed", "音频切换锁已损坏")
            })?;
            if !self.audio_mixer_operation_is_current(operation_token) {
                drop(_switch_guard);
                let _ = self.audio_output_control().and_then(|control| {
                    control
                        .clear()
                        .map_err(|error| CommandErrorDto::new("audio_output_clear_failed", error))
                });
                task.stop_preserving_output();
                return Err(CommandErrorDto::new(
                    "audio_mixer_start_stale",
                    "音轨提交期间收到更新的停止、暂停或切换请求",
                ));
            }
            self.audio_mixer
                .lock()
                .map_err(|_| {
                    CommandErrorDto::new("audio_mixer_lock_failed", "音频混音状态锁已损坏")
                })?
                .replace(task)
        };
        if let Some(mut old) = old {
            old.stop_preserving_output();
        }
        Ok(())
    }

    fn audio_mixer_task_from_snapshot(
        &self,
        app: &AppHandle,
        sample_rate_hz: u32,
        requested_clock: AudioSyncClock,
        preparation_started_at: Instant,
        output_delay_ms: u64,
        playback_watermark_ms: u64,
    ) -> Result<Option<(AudioMixerTask, AudioMixerSourceIdentity, f64, u64)>, CommandErrorDto> {
        let (source_path, start_clock, has_audio, filter_graph, source_identity, playback_rate) = {
            let playback = self
                .playback
                .lock()
                .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
            let snapshot = playback.snapshot();
            let Some(source) = snapshot.source_media.as_ref() else {
                return Ok(None);
            };
            let start_clock = resolve_audio_sync_clock(
                requested_clock,
                snapshot.playback_generation,
                snapshot.loop_index,
                snapshot.current_position_ms,
                source_duration_ms(&snapshot)?,
            )
            .map_err(|reason| CommandErrorDto::new("audio_sync_clock_invalid", reason))?;
            let realtime_audio_path = (snapshot.current_audio_source.as_deref()
                == Some("realtime_variant"))
            .then(|| snapshot.current_audio_reference.clone())
            .flatten()
            .map(PathBuf::from);
            let has_realtime_audio = realtime_audio_path.is_some();
            let stream_processed_audio =
                snapshot.audio_processing_enabled && realtime_audio_path.is_none();
            let source_path = realtime_audio_path.or_else(|| {
                if stream_processed_audio {
                    Some(PathBuf::from(source.source_path.clone()))
                } else {
                    snapshot
                        .current_video_reference
                        .clone()
                        .or_else(|| Some(source.source_path.clone()))
                        .map(PathBuf::from)
                }
            });
            let (filter_graph, playback_rate) = if stream_processed_audio {
                let (audio, variants) = playback.audio_stream_configuration();
                let playback_rate = audio.playback_speed;
                (
                    Some(
                        build_audio_stream_filter_graph(
                            &audio,
                            &variants,
                            source.audio_sample_rate_hz,
                            sample_rate_hz,
                        )
                        .map_err(|error| {
                            CommandErrorDto::new("audio_stream_filter_invalid", error.to_string())
                        })?,
                    ),
                    playback_rate,
                )
            } else {
                (None, 1.0)
            };
            (
                source_path,
                start_clock,
                source.audio_sample_rate_hz.is_some() || has_realtime_audio,
                filter_graph,
                AudioMixerSourceIdentity::from(&snapshot),
                playback_rate,
            )
        };
        if !has_audio {
            return Ok(None);
        }
        let Some(source_path) = source_path else {
            return Ok(None);
        };
        let timeline_start_position_ms = add_wall_clock_delay_to_media_position_ms(
            start_clock.absolute_position_ms,
            output_delay_ms,
            playback_rate,
        );
        let seek_position_ms = timeline_start_position_ms % start_clock.duration_ms;
        let target_root = runtime_resource_target_root(app)
            .map_err(|error| CommandErrorDto::new("media_resource_dir_failed", error))?;
        let (ffmpeg_path, _) = configured_media_engine_paths_with_resource_dir(&target_root)
            .map_err(|error| CommandErrorDto::new("media_engine_unavailable", error.to_string()))?;
        let task = AudioMixerTask::start_candidate_with_filter_and_variant_count(
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            seek_position_ms,
            timeline_start_position_ms,
            filter_graph,
            playback_rate,
            usize::try_from(playback_watermark_ms).unwrap_or(usize::MAX),
            preparation_started_at,
        )
        .map_err(|error| CommandErrorDto::new("audio_mixer_start_failed", error))?;
        Ok(Some((
            task,
            source_identity,
            playback_rate,
            timeline_start_position_ms,
        )))
    }

    fn validate_audio_cycle_candidate_request(
        &self,
        request: &PrepareAudioCycleCandidateRequestDto,
    ) -> Result<(), CommandErrorDto> {
        let snapshot = self
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
            .snapshot();
        if snapshot.playback_state != PlaybackState::Playing || !snapshot.audio_processing_enabled {
            return Err(CommandErrorDto::new(
                "audio_candidate_stale",
                "当前没有处于播放状态的普通声音处理轨",
            ));
        }
        if snapshot.playback_generation != request.playback_generation
            || snapshot.audio_stream_revision != request.base_audio_stream_revision
        {
            return Err(CommandErrorDto::new(
                "audio_candidate_stale",
                "播放代次或声音参数版本已经变化，拒绝旧候选",
            ));
        }
        Ok(())
    }

    fn scheduled_audio_cycle_task(
        &self,
        app: &AppHandle,
        request: &PrepareAudioCycleCandidateRequestDto,
        operation_token: u64,
    ) -> Result<PendingAudioMixerTask, CommandErrorDto> {
        if request.candidate_id == 0 {
            return Err(CommandErrorDto::new(
                "audio_candidate_id_invalid",
                "候选 ID 必须大于 0",
            ));
        }
        let configuration = ValidatedAudioStreamConfiguration::new(
            request.audio.clone(),
            request.audio_variants.clone(),
        )
        .map_err(|errors| {
            CommandErrorDto::new(
                "audio_candidate_params_invalid",
                errors
                    .iter()
                    .map(|error| format!("{}: {}", error.field, error.message))
                    .collect::<Vec<_>>()
                    .join("；"),
            )
        })?;

        let snapshot = self
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
            .snapshot();
        if snapshot.playback_state != PlaybackState::Playing || !snapshot.audio_processing_enabled {
            return Err(CommandErrorDto::new(
                "audio_candidate_stale",
                "当前没有处于播放状态的普通声音处理轨",
            ));
        }
        if snapshot.current_audio_source.as_deref() == Some("realtime_variant") {
            return Err(CommandErrorDto::new(
                "audio_candidate_stale",
                "实时话术音轨正在生效，普通声音周期候选不能覆盖它",
            ));
        }
        if snapshot.playback_generation != request.playback_generation
            || snapshot.audio_stream_revision != request.base_audio_stream_revision
        {
            return Err(CommandErrorDto::new(
                "audio_candidate_stale",
                "播放代次或声音参数版本已经变化，拒绝旧候选",
            ));
        }
        let duration_ms = source_duration_ms(&snapshot)?;
        let current_absolute_position_ms = absolute_media_position_ms(
            snapshot.loop_index,
            snapshot.current_position_ms,
            duration_ms,
        );
        if request.target_absolute_position_ms <= current_absolute_position_ms
            || request.target_absolute_position_ms
                > current_absolute_position_ms.saturating_add(AUDIO_CYCLE_TARGET_HORIZON_MS)
        {
            return Err(CommandErrorDto::new(
                "audio_candidate_target_invalid",
                format!(
                    "候选目标必须位于当前绝对媒体时间之后 {}ms 内",
                    AUDIO_CYCLE_TARGET_HORIZON_MS
                ),
            ));
        }
        let source = snapshot
            .source_media
            .as_ref()
            .ok_or_else(|| CommandErrorDto::new("source_media_required", "请先导入一个源视频"))?;
        if source.audio_sample_rate_hz.is_none() {
            return Err(CommandErrorDto::new(
                "audio_candidate_source_missing",
                "源视频没有可供周期处理的音频流",
            ));
        }

        let (sample_rate_hz, playback_watermark_ms) = self
            .audio_output_control()?
            .status()
            .filter(|status| {
                let health = status.health;
                health.application_running
                    && matches!(
                        health.hardware_state,
                        autolive_portaudio_output::PortAudioHardwareState::Active
                    )
            })
            .map(|status| (status.sample_rate_hz, status.playback_watermark_ms))
            .ok_or_else(|| {
                CommandErrorDto::new(
                    "audio_output_inactive",
                    "PortAudio 硬件出口未处于活动状态，保持当前输出",
                )
            })?;
        let filter_graph = build_audio_stream_filter_graph(
            &request.audio,
            &request.audio_variants,
            source.audio_sample_rate_hz,
            sample_rate_hz,
        )
        .map_err(|error| {
            CommandErrorDto::new("audio_candidate_params_invalid", error.to_string())
        })?;
        let target_root = runtime_resource_target_root(app)
            .map_err(|error| CommandErrorDto::new("media_resource_dir_failed", error))?;
        let (ffmpeg_path, _) = configured_media_engine_paths_with_resource_dir(&target_root)
            .map_err(|error| CommandErrorDto::new("media_engine_unavailable", error.to_string()))?;
        let candidate_start_position_ms = request
            .target_absolute_position_ms
            .saturating_sub(AUDIO_CYCLE_CANDIDATE_PRE_ROLL_MS);
        let seek_position_ms = candidate_start_position_ms % duration_ms;
        let preparation_started_at = Instant::now();
        let task = AudioMixerTask::start_scheduled_candidate_with_filter_and_variant_count(
            ffmpeg_path,
            PathBuf::from(&source.source_path),
            sample_rate_hz,
            seek_position_ms,
            candidate_start_position_ms,
            Some(filter_graph),
            request.audio.playback_speed,
            AUDIO_CYCLE_CANDIDATE_BUFFER_MS,
            scheduled_candidate_commit_tail_ms(playback_watermark_ms),
            preparation_started_at,
        )
        .map_err(|error| CommandErrorDto::new("audio_candidate_start_failed", error))?;

        Ok(PendingAudioMixerTask {
            token: operation_token,
            task,
            source_identity: AudioMixerSourceIdentity::from(&snapshot),
            sample_rate_hz,
            observed_absolute_position_ms: current_absolute_position_ms,
            candidate_start_absolute_position_ms: candidate_start_position_ms,
            preparation_started_at,
            playback_rate: request.audio.playback_speed,
            kind: PendingAudioMixerKind::AudioCycle(Box::new(AudioCycleCandidate {
                candidate_id: request.candidate_id,
                configuration,
                target_absolute_position_ms: request.target_absolute_position_ms,
            })),
        })
    }

    // 切换提交边界需要同时携带两个时钟、一次预热事务和候选身份，拆成一次性结构会隐藏校验关系。
    #[allow(clippy::too_many_arguments)]
    fn commit_audio_mixer_candidate(
        &self,
        mut candidate: AudioMixerTask,
        sample_rate_hz: u32,
        observed_absolute_position_ms: u64,
        candidate_start_absolute_position_ms: u64,
        preparation_started_at: Instant,
        playback_rate: f64,
        source_identity: &AudioMixerSourceIdentity,
        operation_token: u64,
    ) -> Result<(), CommandErrorDto> {
        if !self.audio_mixer_operation_is_current(operation_token) {
            candidate.stop_preserving_output();
            return Err(CommandErrorDto::new(
                "audio_mixer_candidate_stale",
                "候选操作代次已经失效，保持当前音轨",
            ));
        }
        if let Err(error) = self.ensure_audio_mixer_source_current(source_identity) {
            candidate.stop_preserving_output();
            return Err(error);
        }
        // 预热和提交共用 preparation_started_at；候选只差少量 PCM 时继续生产，
        // 每次按最新画面、环缓和 DAC 延迟重新检查，最长等待 5 秒。
        let (commit_absolute_position_ms, candidate_pcm_position_ms) = match self
            .wait_for_audio_mixer_commit_coverage(
                &candidate,
                sample_rate_hz,
                observed_absolute_position_ms,
                candidate_start_absolute_position_ms,
                preparation_started_at,
                playback_rate,
                source_identity,
                operation_token,
            ) {
            Ok(coverage) => coverage,
            Err(error) => {
                candidate.stop_preserving_output();
                return Err(error);
            }
        };
        let _commit_guard = self.begin_audio_cycle_commit()?;
        if !self.audio_mixer_operation_is_current(operation_token) {
            candidate.stop_preserving_output();
            return Err(CommandErrorDto::new(
                "audio_mixer_candidate_stale",
                "候选覆盖就绪后收到更新的停止、暂停或切换请求",
            ));
        }
        if let Err(error) = self.ensure_audio_mixer_source_current(source_identity) {
            candidate.stop_preserving_output();
            return Err(error);
        }

        // 提交前再次确认硬件消费者仍在推进；如果 callback 已停，必须保留旧任务和旧环缓，
        // 不能先停旧轨再把候选写入一个不会被消费的输出。
        let output_control = self.audio_output_control()?;
        let output_consumer_active = output_control.status().is_some_and(|status| {
            let health = status.health;
            health.application_running
                && matches!(
                    health.hardware_state,
                    autolive_portaudio_output::PortAudioHardwareState::Active
                )
        });
        if !output_consumer_active {
            let mut candidate = candidate;
            candidate.stop_preserving_output();
            return Err(CommandErrorDto::new(
                "audio_output_consumer_inactive",
                "PortAudio 硬件消费者未处于活动状态，保持旧轨",
            ));
        }

        let old_exists = self
            .audio_mixer
            .lock()
            .map_err(|_| CommandErrorDto::new("audio_mixer_lock_failed", "音频混音状态锁已损坏"))?
            .is_some();
        if !old_exists {
            candidate.stop_preserving_output();
            return Err(CommandErrorDto::new(
                "audio_mixer_old_track_missing",
                "旧音轨不存在，取消候选切换",
            ));
        }
        if let Err(error) = self.ensure_audio_mixer_source_current(source_identity) {
            candidate.stop_preserving_output();
            return Err(error);
        }
        if let Err(error) = candidate.commit_at_position(candidate_pcm_position_ms) {
            candidate.stop_preserving_output();
            return Err(CommandErrorDto::new("audio_mixer_commit_failed", error));
        }
        if let Err(error) = output_control.crossfade_to(
            candidate.output_track(),
            AudioTrackTimeline {
                media_position_ms: commit_absolute_position_ms,
                playback_rate,
            },
        ) {
            candidate.stop_preserving_output();
            return Err(CommandErrorDto::new("audio_mixer_crossfade_failed", error));
        }
        let mut candidate = Some(candidate);
        let replacement = {
            let _switch_guard = self.audio_mixer_switch_lock.lock().map_err(|_| {
                CommandErrorDto::new("audio_mixer_switch_lock_failed", "音频切换锁已损坏")
            })?;
            if !self.audio_mixer_operation_is_current(operation_token) {
                None
            } else {
                let next = candidate.take().ok_or_else(|| {
                    CommandErrorDto::new("audio_mixer_candidate_missing", "候选音轨所有权已丢失")
                })?;
                let old = self
                    .audio_mixer
                    .lock()
                    .map_err(|_| {
                        CommandErrorDto::new("audio_mixer_lock_failed", "音频混音状态锁已损坏")
                    })?
                    .replace(next);
                Some(old)
            }
        };
        let Some(old) = replacement else {
            let _ = output_control.clear();
            if let Some(mut candidate) = candidate {
                candidate.stop_preserving_output();
            }
            return Err(CommandErrorDto::new(
                "audio_mixer_candidate_stale",
                "候选提交期间收到更新的停止、暂停或切换请求",
            ));
        };
        if let Some(mut old) = old {
            old.stop_preserving_output();
        }
        Ok(())
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
            let finished = worker.as_ref().is_some_and(|task| {
                task.completed.load(Ordering::Acquire) || task.handle.is_finished()
            });
            if finished {
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
    pub audio_stream_variant_count: usize,
    pub audio_stream_revision: u64,
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
            audio_stream_variant_count: value.audio_stream_variant_count,
            audio_stream_revision: value.audio_stream_revision,
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
    /// 多虚拟轨音频参数；空则只用 params.audio。
    #[serde(default)]
    pub audio_variants: Option<Vec<autolive_desktop_core::research_params::AudioResearchParams>>,
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
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.2));
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
        if is_protected {
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

fn remove_media_processing_cache_files_at(
    directory: &Path,
    protected_paths: &[PathBuf],
    now: SystemTime,
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
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let partial_expiry = now
        .checked_sub(Duration::from_secs(10 * 60))
        .unwrap_or(SystemTime::UNIX_EPOCH);
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
    for (path, size, modified) in entries {
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let is_partial = name.starts_with("processed-") && name.contains(".partial");
        let is_completed = name.starts_with("processed-") && name.ends_with(".mp4") && !is_partial;
        let stale_partial = is_partial && modified <= partial_expiry;
        if protected.contains(&path) || (!is_completed && !stale_partial) {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {
                remaining_bytes = remaining_bytes.saturating_sub(size);
                removed_files += 1;
                removed_bytes = removed_bytes.saturating_add(size);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                remaining_bytes = remaining_bytes.saturating_sub(size);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(CacheCleanupResultDto {
        removed_files,
        removed_bytes,
        remaining_bytes,
    })
}

fn remove_media_processing_cache_files(
    directory: &Path,
    protected_paths: &[PathBuf],
) -> std::io::Result<CacheCleanupResultDto> {
    remove_media_processing_cache_files_at(directory, protected_paths, SystemTime::now())
}

#[cfg(test)]
mod cache_tests {
    use super::{prune_cache_dir, remove_media_processing_cache_files_at};
    use std::fs;
    use std::time::{Duration, SystemTime};

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

    #[test]
    fn media_cache_cleanup_removes_only_unprotected_completed_files_and_stale_partials() {
        let directory = std::env::temp_dir().join(format!(
            "autolive-media-cache-delete-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("cache directory should be created");
        let protected = directory.join("processed-current.mp4");
        let completed = directory.join("processed-old.mp4");
        let stale_partial = directory.join("processed-old.partial.mp4");
        let fresh_partial = directory.join("processed-fresh.partial.mp4");
        let unrelated = directory.join("other.mp4");
        fs::write(&protected, b"keep").expect("write");
        fs::write(&completed, b"done").expect("write");
        fs::write(&stale_partial, b"stale").expect("write");
        fs::write(&unrelated, b"other").expect("write");

        let result = remove_media_processing_cache_files_at(
            &directory,
            std::slice::from_ref(&protected),
            SystemTime::now() + Duration::from_secs(11 * 60),
        )
        .expect("media cache cleanup should succeed");

        assert!(protected.exists());
        assert!(!completed.exists());
        assert!(!stale_partial.exists());
        assert!(unrelated.exists());
        assert_eq!(result.removed_files, 2);
        assert_eq!(result.removed_bytes, 9);
        assert_eq!(result.remaining_bytes, 9);

        fs::write(&fresh_partial, b"fresh").expect("write");
        let fresh_result = remove_media_processing_cache_files_at(
            &directory,
            std::slice::from_ref(&protected),
            SystemTime::now(),
        )
        .expect("fresh partial cleanup should succeed");
        assert!(fresh_partial.exists());
        assert_eq!(fresh_result.removed_files, 0);
        assert_eq!(fresh_result.remaining_bytes, 14);
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
    let (current_video, pending_video, current_audio, pending_audio, research_paths) = {
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
            snapshot.current_audio_reference.map(PathBuf::from),
            snapshot.pending_audio_reference.map(PathBuf::from),
            research_paths,
        )
    };
    let media_result = prune_cache_dir(
        &cache_root.join("media-processing"),
        MEDIA_CACHE_MAX_BYTES,
        Some(MEDIA_CACHE_MAX_FILES),
        &[current_video, pending_video, current_audio, pending_audio]
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

fn cleanup_media_processing_cache(
    app: &AppHandle,
    state: &AppState,
) -> Result<CacheCleanupResultDto, CommandErrorDto> {
    let _ = state.reap_finished_media_worker()?;
    if state.media_worker_is_running()? {
        return Err(CommandErrorDto::new(
            "media_cache_cleanup_busy",
            "媒体处理仍在进行，请完成后再删除已生成缓存",
        ));
    }
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandErrorDto::new("cache_dir_failed", error.to_string()))?
        .join("media-processing");
    let protected_paths = {
        let playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        let snapshot = playback.snapshot();
        if snapshot.video_processing_status == "processing"
            || snapshot.audio_processing_status == "processing"
        {
            return Err(CommandErrorDto::new(
                "media_cache_cleanup_busy",
                "媒体处理仍在进行，请完成后再删除已生成缓存",
            ));
        }
        if matches!(snapshot.playback_state, PlaybackState::Playing)
            && snapshot.audio_processing_enabled
        {
            return Err(CommandErrorDto::new(
                "media_cache_cleanup_playing",
                "声音处理正在播放，请先暂停或停止播放后再删除缓存",
            ));
        }
        [
            snapshot.current_video_reference,
            snapshot.pending_video_reference,
            snapshot.current_audio_reference,
            snapshot.pending_audio_reference,
        ]
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .collect::<Vec<_>>()
    };
    remove_media_processing_cache_files(&cache_dir, &protected_paths)
        .map_err(|error| CommandErrorDto::new("media_cache_cleanup_failed", error.to_string()))
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AudioOutputBackendStatusDto {
    pub available: bool,
    pub selected_backend: String,
    pub preferred_portaudio: bool,
    pub running: bool,
    pub reason: Option<String>,
    pub reason_code: Option<String>,
    pub retryable: bool,
    pub xrun_count: u64,
    pub hardware_state: String,
    pub callback_status_flags: u64,
    pub callback_status_flags_count: u64,
    pub callback_underrun_count: u64,
    pub producer_drop_count: u64,
    /// PortAudio 报告的实际输出/DAC 提前量，取两种硬件时钟中的较大值。
    pub output_latency_ms: u64,
    /// 根据采样率和硬件 callback 帧数计算的有效播放水位；与 KiB 容量相互独立。
    pub playback_watermark_ms: u64,
    pub actual_sample_rate_hz: Option<u32>,
    pub callback_pcm_frames_total: u64,
    pub audio_timeline_position_ms: Option<u64>,
    /// 音频可听时间减去视频绝对时间；负值表示音频落后。
    pub av_offset_ms: Option<i64>,
    pub callback_stalled_ms: Option<u64>,
    /// 实际有效 PCM frame 未增长的持续时间；暂停/硬件不可用时为 None。
    pub pcm_stalled_ms: Option<u64>,
    /// 仅表示当前硬件消费者仍健康、但 PCM 生产任务需要受控重建一次。
    pub recovery_required: bool,
    pub ring_len_samples: u64,
    pub ring_capacity_samples: u64,
    pub device_index: Option<i32>,
    pub memory_buffer_kib: u32,
    pub frames_per_buffer: u32,
    pub sample_rate_hz: u32,
    pub channels: u16,
    /// 当前音轨与候选音轨的任务数；最多为 2（current + pending）。
    pub audio_task_count: usize,
    pub current_audio_ffmpeg_pid: Option<u32>,
    pub pending_audio_ffmpeg_pid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AudioCycleDiagnosticDto {
    pub sequence: u64,
    pub captured_at_ms: u64,
    pub sample_rate_hz: u32,
    pub captured_frame_count: u64,
    pub line: Vec<f32>,
    pub rms_dbfs: f32,
    pub peak_dbfs: f32,
    pub low_band_rms_dbfs: f32,
    pub cutoff_hz: f32,
    pub has_pcm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AudioOutputDeviceDto {
    pub id: String,
    pub name: String,
    pub host_api: String,
    pub max_output_channels: u16,
    pub default_sample_rate_hz: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetAudioOutputBackendRequestDto {
    /// true = 尝试 PortAudio；false = WebView。
    pub prefer_portaudio: bool,
    /// PortAudio 设备索引；None = 默认输出。
    pub device_index: Option<i32>,
    /// PortAudio 硬件回调帧数；仅供内部兼容和诊断，不是 UI 的内存缓冲。
    pub frames_per_buffer: Option<u32>,
    /// 应用侧 PCM 环形内存缓冲，范围 128–2048 KiB；省略时默认 1024 KiB。
    pub memory_buffer_kib: Option<u32>,
    /// 与 Web Audio Context 对齐；None = 48k。
    pub sample_rate_hz: Option<u32>,
    /// 以下字段共同描述最终播放窗口的未回绕绝对媒体时钟。
    pub position_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub absolute_position_ms: Option<u64>,
    pub loop_index: Option<u64>,
    pub playback_generation: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SyncAudioOutputSourceRequestDto {
    pub position_ms: u64,
    pub duration_ms: u64,
    pub absolute_position_ms: u64,
    pub loop_index: u64,
    pub playback_generation: u64,
    /// 仅在状态明确标记 recovery_required 时，预热替代生产者并原子替换。
    #[serde(default)]
    pub recover_unhealthy: bool,
    /// 视频循环已由后端确认时，源同步优先替换普通 N+1 候选，避免沿用上一轮时间轴。
    #[serde(default)]
    pub reanchor_loop_boundary: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrepareAudioCycleCandidateRequestDto {
    pub candidate_id: u64,
    pub audio: autolive_desktop_core::research_params::AudioResearchParams,
    #[serde(default)]
    pub audio_variants: Vec<autolive_desktop_core::research_params::AudioResearchParams>,
    pub playback_generation: u64,
    pub base_audio_stream_revision: u64,
    /// 未按视频时长回绕的目标媒体时间。
    pub target_absolute_position_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrepareAudioCycleCandidateResultDto {
    pub candidate_id: u64,
    pub state: String,
    pub target_absolute_position_ms: u64,
    pub buffered_ms: u64,
    pub ffmpeg_pid: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitAudioCycleCandidateRequestDto {
    pub candidate_id: u64,
    pub playback_generation: u64,
    pub loop_index: u64,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub absolute_position_ms: u64,
}

impl SetAudioOutputBackendRequestDto {
    fn audio_sync_clock(&self) -> Option<AudioSyncClock> {
        Some(AudioSyncClock {
            playback_generation: self.playback_generation?,
            loop_index: self.loop_index?,
            position_ms: self.position_ms?,
            duration_ms: self.duration_ms?,
            absolute_position_ms: self.absolute_position_ms?,
        })
    }
}

impl From<&SyncAudioOutputSourceRequestDto> for AudioSyncClock {
    fn from(request: &SyncAudioOutputSourceRequestDto) -> Self {
        Self {
            playback_generation: request.playback_generation,
            loop_index: request.loop_index,
            position_ms: request.position_ms,
            duration_ms: request.duration_ms,
            absolute_position_ms: request.absolute_position_ms,
        }
    }
}

impl From<&CommitAudioCycleCandidateRequestDto> for AudioSyncClock {
    fn from(request: &CommitAudioCycleCandidateRequestDto) -> Self {
        Self {
            playback_generation: request.playback_generation,
            loop_index: request.loop_index,
            position_ms: request.position_ms,
            duration_ms: request.duration_ms,
            absolute_position_ms: request.absolute_position_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitAudioCycleCandidateResultDto {
    pub candidate_id: u64,
    pub committed: bool,
    pub reason: Option<String>,
    pub snapshot: PlaybackSnapshotDto,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CancelAudioCycleCandidateRequestDto {
    pub candidate_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CancelAudioCycleCandidateResultDto {
    pub candidate_id: Option<u64>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlayPortAudioTestToneRequestDto {
    pub frequency_hz: Option<f32>,
    pub duration_ms: Option<u32>,
    pub amplitude: Option<f32>,
}

fn host_api_label(kind: autolive_portaudio_output::HostApiKind) -> &'static str {
    match kind {
        autolive_portaudio_output::HostApiKind::Default => "default",
        autolive_portaudio_output::HostApiKind::Wasapi => "wasapi",
        autolive_portaudio_output::HostApiKind::Asio => "asio",
        autolive_portaudio_output::HostApiKind::Mme => "mme",
        autolive_portaudio_output::HostApiKind::DirectSound => "dsound",
        autolive_portaudio_output::HostApiKind::Wdmks => "wdmks",
        autolive_portaudio_output::HostApiKind::Other => "other",
    }
}

fn portaudio_hardware_state_label(
    state: autolive_portaudio_output::PortAudioHardwareState,
) -> &'static str {
    match state {
        autolive_portaudio_output::PortAudioHardwareState::Unsupported => "unsupported",
        autolive_portaudio_output::PortAudioHardwareState::NotCreated => "not_created",
        autolive_portaudio_output::PortAudioHardwareState::Active => "active",
        autolive_portaudio_output::PortAudioHardwareState::Stopped => "stopped",
        autolive_portaudio_output::PortAudioHardwareState::Inactive => "inactive",
        autolive_portaudio_output::PortAudioHardwareState::Unknown => "unknown",
        autolive_portaudio_output::PortAudioHardwareState::QueryError(_) => "query_error",
    }
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn reset_audio_output_callback_observation(state: &AppState) {
    state
        .audio_output_last_callback_count
        .store(0, Ordering::Relaxed);
    state
        .audio_output_last_callback_progress_ms
        .store(0, Ordering::Relaxed);
    state
        .audio_output_last_pcm_frame_count
        .store(0, Ordering::Relaxed);
    state
        .audio_output_last_pcm_progress_ms
        .store(0, Ordering::Relaxed);
}

fn audio_output_status_dto(
    state: &AppState,
) -> Result<AudioOutputBackendStatusDto, CommandErrorDto> {
    let preferred = *state
        .audio_output_preferred
        .lock()
        .map_err(|_| CommandErrorDto::new("audio_output_lock_failed", "音频出口状态锁已损坏"))?;
    let control = state
        .audio_cycle_output
        .lock()
        .map_err(|_| {
            CommandErrorDto::new("audio_cycle_output_lock_failed", "音频周期输出状态锁已损坏")
        })?
        .as_ref()
        .map(AudioCycleOutputTask::control);
    let output = control.as_ref().and_then(AudioCycleOutputControl::status);
    let writer_failure = control.as_ref().and_then(AudioCycleOutputControl::failure);
    let now_ms = unix_now_ms();
    let health = output.map(|output| output.health);
    let callback_count = health.map(|snapshot| snapshot.callback_count).unwrap_or(0);
    let application_running = health.is_some_and(|snapshot| snapshot.application_running);
    let hardware_state = health
        .map(|snapshot| snapshot.hardware_state)
        .unwrap_or(autolive_portaudio_output::PortAudioHardwareState::NotCreated);
    let hardware_active = matches!(
        hardware_state,
        autolive_portaudio_output::PortAudioHardwareState::Active
    );
    let hardware_unhealthy = output.is_some() && !hardware_active;
    let hardware_inactive = matches!(
        hardware_state,
        autolive_portaudio_output::PortAudioHardwareState::Stopped
            | autolive_portaudio_output::PortAudioHardwareState::Inactive
            | autolive_portaudio_output::PortAudioHardwareState::Unknown
            | autolive_portaudio_output::PortAudioHardwareState::QueryError(_)
    );
    let probe = if output.is_some() {
        autolive_portaudio_output::OutputBackendStatus {
            available: true,
            selected_backend: "portaudio",
            reason: None,
            xrun_count: 0,
        }
    } else {
        autolive_portaudio_output::probe_portaudio()
    };
    let xrun_count = health
        .map(|snapshot| snapshot.xrun_count)
        .unwrap_or(probe.xrun_count);
    let callback_status_flags = health
        .map(|snapshot| snapshot.callback_last_status_flags)
        .unwrap_or(0);
    let callback_status_flags_count = health
        .map(|snapshot| snapshot.callback_status_flags_count)
        .unwrap_or(0);
    let callback_underrun_count = health
        .map(|snapshot| snapshot.callback_underrun_count)
        .unwrap_or(0);
    let producer_drop_count = health
        .map(|snapshot| snapshot.producer_drop_count)
        .unwrap_or(0);
    let output_latency_ms = health
        .map(|snapshot| {
            resolve_audio_output_latency_ms(
                snapshot.output_latency_us,
                snapshot.callback_output_buffer_dac_time_delta_us,
            )
        })
        .unwrap_or(0);
    let playback_watermark_ms = output
        .map(|snapshot| snapshot.playback_watermark_ms)
        .unwrap_or(0);
    let actual_sample_rate_hz = health.and_then(|snapshot| snapshot.actual_sample_rate_hz);
    let callback_pcm_frames_total = health
        .map(|snapshot| snapshot.callback_pcm_frames_total)
        .unwrap_or(0);
    let audio_timeline_position_ms = output.and_then(|output| output.timeline_media_position_ms);
    let video_timeline_position_ms = state.playback.lock().ok().and_then(|playback| {
        let snapshot = playback.snapshot();
        let duration_ms = snapshot
            .source_media
            .as_ref()
            .and_then(|source| source.duration_ms)
            .filter(|duration_ms| *duration_ms > 0)?;
        let base_position_ms = absolute_media_position_ms(
            snapshot.loop_index,
            snapshot.current_position_ms,
            duration_ms,
        );
        if snapshot.playback_state != PlaybackState::Playing {
            return Some(base_position_ms);
        }
        let observed_elapsed_ms =
            state
                .playback_position_observed_at
                .lock()
                .ok()
                .and_then(|observed_at| {
                    observed_at.map(|observed_at| {
                        observed_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
                    })
                });
        let Some(observed_elapsed_ms) = observed_elapsed_ms else {
            return Some(base_position_ms);
        };
        let playback_rate = playback.audio_stream_configuration().0.playback_speed;
        let playback_rate = if playback_rate.is_finite() && playback_rate > 0.0 {
            playback_rate
        } else {
            1.0
        };
        let elapsed_media_ms = (observed_elapsed_ms as f64 * playback_rate)
            .round()
            .clamp(0.0, u64::MAX as f64) as u64;
        Some(base_position_ms.saturating_add(elapsed_media_ms))
    });
    let av_offset_ms = audio_timeline_position_ms
        .zip(video_timeline_position_ms)
        .map(|(audio_ms, video_ms)| signed_millis_delta(audio_ms, video_ms));
    let ring_len_samples = health
        .map(|snapshot| snapshot.ring_len_samples as u64)
        .unwrap_or(0);
    let ring_capacity_samples = health
        .map(|snapshot| snapshot.ring_capacity_samples as u64)
        .unwrap_or(0);
    let (
        current_mixer_exists,
        mixer_failure,
        audio_task_count,
        current_audio_ffmpeg_pid,
        pending_audio_ffmpeg_pid,
    ) = {
        let mixer = state.audio_mixer.lock().ok();
        let current_task = mixer.as_ref().and_then(|mixer| mixer.as_ref());
        let pending = state.audio_mixer_pending.lock().ok();
        let pending_task = pending.as_ref().and_then(|pending| pending.as_ref());
        (
            current_task.is_some(),
            current_task
                .and_then(AudioMixerTask::failure)
                .or(writer_failure),
            (if current_task.is_some() { 1 } else { 0 })
                + (if pending_task.is_some() { 1 } else { 0 }),
            current_task.and_then(AudioMixerTask::ffmpeg_pid),
            pending_task.and_then(|pending| pending.task.ffmpeg_pid()),
        )
    };
    let callback_paused = output.is_some_and(|output| output.callback_paused);
    let consumer_observation_enabled =
        output.is_some() && application_running && hardware_active && !callback_paused;
    let previous_callback_count = state
        .audio_output_last_callback_count
        .load(Ordering::Relaxed);
    let previous_callback_progress_ms = state
        .audio_output_last_callback_progress_ms
        .load(Ordering::Relaxed);
    let callback_progress = observe_counter(
        now_ms,
        callback_count,
        previous_callback_count,
        previous_callback_progress_ms,
        consumer_observation_enabled,
        PORTAUDIO_CALLBACK_STALL_MS,
    );
    state
        .audio_output_last_callback_count
        .store(callback_progress.last_value, Ordering::Relaxed);
    state
        .audio_output_last_callback_progress_ms
        .store(callback_progress.last_progress_ms, Ordering::Relaxed);
    let producer_observation_enabled = consumer_observation_enabled
        && current_mixer_exists
        && mixer_failure.is_none()
        && audio_timeline_position_ms.is_some();
    let pcm_progress = observe_counter(
        now_ms,
        callback_pcm_frames_total,
        state
            .audio_output_last_pcm_frame_count
            .load(Ordering::Relaxed),
        state
            .audio_output_last_pcm_progress_ms
            .load(Ordering::Relaxed),
        producer_observation_enabled,
        PORTAUDIO_PCM_STALL_MS,
    );
    state
        .audio_output_last_pcm_frame_count
        .store(pcm_progress.last_value, Ordering::Relaxed);
    state
        .audio_output_last_pcm_progress_ms
        .store(pcm_progress.last_progress_ms, Ordering::Relaxed);
    let callback_stalled = callback_progress.stalled;
    let callback_stalled_ms = callback_progress.stalled_ms;
    let pcm_stalled = pcm_progress.stalled;
    let pcm_stalled_ms = pcm_progress.stalled_ms;
    // 启动阶段的短暂欠载不应立刻切走 PortAudio：应用侧内存环缓和 FFmpeg
    // loudnorm 初始化可能跨过若干回调。仅在运行约 5 秒且至少 75% 回调欠载
    // 时认定为持续无声，保留真正无源时的 WebView 回退能力。
    let sustained_underrun = output.is_some()
        && !callback_paused
        && callback_count > 1_024
        && callback_underrun_count > 32
        && callback_underrun_count.saturating_mul(4) >= callback_count.saturating_mul(3);
    let health_input = OutputHealthInput {
        output_exists: output.is_some(),
        application_running,
        hardware_active,
        callback_paused,
        callback_stalled,
        pcm_stalled,
        sustained_underrun,
        current_mixer_exists,
        mixer_failed: mixer_failure.is_some(),
        timeline_present: audio_timeline_position_ms.is_some(),
    };
    let output_health = evaluate_output_health(health_input);
    let playback_active = state
        .playback
        .lock()
        .ok()
        .is_some_and(|playback| playback.snapshot().playback_state == PlaybackState::Playing);
    let resume_required = output_resume_required(health_input, playback_active);
    let running = output_health.running;
    let recovery_required = output_health.recovery_required;
    let portaudio_unhealthy = output.is_some() && !callback_paused && !running;
    let device_index = output.and_then(|output| output.device_index);
    let frames_per_buffer = output
        .map(|output| output.frames_per_buffer)
        .unwrap_or(autolive_portaudio_output::DEFAULT_FRAMES_PER_BUFFER);
    let memory_buffer_kib = output
        .map(|output| output.memory_buffer_kib)
        .unwrap_or(autolive_portaudio_output::DEFAULT_RING_CAPACITY_KIB);
    let sample_rate_hz = output
        .map(|output| output.sample_rate_hz)
        .unwrap_or(autolive_portaudio_output::DEFAULT_SAMPLE_RATE_HZ);
    let channels = output.map(|output| output.channels).unwrap_or(2);
    let hardware_state_label = portaudio_hardware_state_label(hardware_state).to_owned();
    if preferred && output.is_some() && (portaudio_unhealthy || mixer_failure.is_some()) {
        eprintln!(
            "autolive audio output health: hardware_state={}, application_running={}, current_mixer_exists={}, recovery_required={}, callback_count={}, callback_status_flags=0x{:x}, callback_status_flags_count={}, callback_underrun_count={}, producer_drop_count={}, output_latency_ms={}, playback_watermark_ms={}, actual_sample_rate_hz={:?}, callback_pcm_frames_total={}, audio_timeline_position_ms={:?}, av_offset_ms={:?}, callback_stalled_ms={:?}, pcm_stalled_ms={:?}, xrun_count={}, ring_len_samples={}, ring_capacity_samples={}",
            hardware_state_label,
            application_running,
            current_mixer_exists,
            recovery_required,
            callback_count,
            callback_status_flags,
            callback_status_flags_count,
            callback_underrun_count,
            producer_drop_count,
            output_latency_ms,
            playback_watermark_ms,
            actual_sample_rate_hz,
            callback_pcm_frames_total,
            audio_timeline_position_ms,
            av_offset_ms,
            callback_stalled_ms,
            pcm_stalled_ms,
            xrun_count,
            ring_len_samples,
            ring_capacity_samples,
        );
    }
    if preferred && running {
        return Ok(AudioOutputBackendStatusDto {
            available: true,
            selected_backend: "portaudio".to_owned(),
            preferred_portaudio: true,
            running: true,
            reason: None,
            reason_code: None,
            retryable: false,
            xrun_count,
            hardware_state: hardware_state_label.clone(),
            callback_status_flags,
            callback_status_flags_count,
            callback_underrun_count,
            producer_drop_count,
            output_latency_ms,
            playback_watermark_ms,
            actual_sample_rate_hz,
            callback_pcm_frames_total,
            audio_timeline_position_ms,
            av_offset_ms,
            callback_stalled_ms,
            pcm_stalled_ms,
            recovery_required: false,
            ring_len_samples,
            ring_capacity_samples,
            device_index,
            memory_buffer_kib,
            frames_per_buffer,
            sample_rate_hz,
            channels,
            audio_task_count,
            current_audio_ffmpeg_pid,
            pending_audio_ffmpeg_pid,
        });
    }
    if preferred && !probe.available {
        return Ok(AudioOutputBackendStatusDto {
            available: false,
            selected_backend: "webview".to_owned(),
            preferred_portaudio: true,
            running: false,
            reason: probe
                .reason
                .or_else(|| Some("PortAudio 不可用，已回退 WebView".to_owned())),
            reason_code: None,
            retryable: false,
            xrun_count,
            hardware_state: hardware_state_label.clone(),
            callback_status_flags,
            callback_status_flags_count,
            callback_underrun_count,
            producer_drop_count,
            output_latency_ms,
            playback_watermark_ms,
            actual_sample_rate_hz,
            callback_pcm_frames_total,
            audio_timeline_position_ms,
            av_offset_ms,
            callback_stalled_ms,
            pcm_stalled_ms,
            recovery_required: false,
            ring_len_samples,
            ring_capacity_samples,
            device_index: None,
            memory_buffer_kib: autolive_portaudio_output::DEFAULT_RING_CAPACITY_KIB,
            frames_per_buffer: autolive_portaudio_output::DEFAULT_FRAMES_PER_BUFFER,
            sample_rate_hz: autolive_portaudio_output::DEFAULT_SAMPLE_RATE_HZ,
            channels: 2,
            audio_task_count,
            current_audio_ffmpeg_pid,
            pending_audio_ffmpeg_pid,
        });
    }
    Ok(AudioOutputBackendStatusDto {
        available: probe.available,
        selected_backend: "webview".to_owned(),
        preferred_portaudio: preferred,
        running: false,
        reason: mixer_failure.or_else(|| {
            if preferred && !current_mixer_exists {
                Some("PortAudio 当前 PCM 生产任务不存在".to_owned())
            } else if preferred && audio_timeline_position_ms.is_none() && !callback_paused {
                Some("PortAudio 当前 PCM 时间轴未建立".to_owned())
            } else if preferred && pcm_stalled {
                Some(format!(
                    "PortAudio PCM 生产已停止（{}ms 无有效样本）",
                    pcm_stalled_ms.unwrap_or(0)
                ))
            } else if preferred && sustained_underrun {
                Some("PortAudio 环形缓冲持续欠载".to_owned())
            } else if preferred && hardware_inactive {
                Some(format!("PortAudio 硬件流已停止（{hardware_state_label}）"))
            } else if preferred && hardware_unhealthy {
                Some(format!(
                    "PortAudio 硬件流状态异常（{hardware_state_label}）"
                ))
            } else if preferred && callback_stalled {
                Some("PortAudio 回调已停止或无进度".to_owned())
            } else if preferred && resume_required {
                Some("PortAudio 输出已暂停，等待恢复现有音轨".to_owned())
            } else if preferred {
                Some("PortAudio 已选择但尚未输出".to_owned())
            } else {
                probe.reason
            }
        }),
        reason_code: resume_required.then(|| AUDIO_OUTPUT_RESUME_REQUIRED_CODE.to_owned()),
        retryable: resume_required,
        xrun_count,
        hardware_state: hardware_state_label,
        callback_status_flags,
        callback_status_flags_count,
        callback_underrun_count,
        producer_drop_count,
        output_latency_ms,
        playback_watermark_ms,
        actual_sample_rate_hz,
        callback_pcm_frames_total,
        audio_timeline_position_ms,
        av_offset_ms,
        callback_stalled_ms,
        pcm_stalled_ms,
        recovery_required,
        ring_len_samples,
        ring_capacity_samples,
        device_index,
        memory_buffer_kib,
        frames_per_buffer,
        sample_rate_hz,
        channels,
        audio_task_count,
        current_audio_ffmpeg_pid,
        pending_audio_ffmpeg_pid,
    })
}

#[tauri::command]
pub fn get_audio_output_backend_status(
    window: Window,
    state: State<'_, AppState>,
) -> Result<AudioOutputBackendStatusDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    audio_output_status_dto(&state)
}

#[tauri::command]
pub fn get_audio_cycle_diagnostic(
    window: Window,
    state: State<'_, AppState>,
) -> Result<AudioCycleDiagnosticDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let snapshot = state
        .audio_cycle_output
        .lock()
        .map_err(|_| {
            CommandErrorDto::new("audio_cycle_output_lock_failed", "音频周期输出状态锁已损坏")
        })?
        .as_ref()
        .map(AudioCycleOutputTask::control)
        .and_then(|control| control.diagnostic())
        .unwrap_or_else(|| {
            AudioLowFrequencyDiagnosticSnapshot::empty(
                autolive_portaudio_output::DEFAULT_SAMPLE_RATE_HZ,
            )
        });
    Ok(audio_cycle_diagnostic_dto(snapshot))
}

fn audio_cycle_diagnostic_dto(
    snapshot: AudioLowFrequencyDiagnosticSnapshot,
) -> AudioCycleDiagnosticDto {
    let line = if snapshot.has_pcm {
        snapshot.line.to_vec()
    } else {
        Vec::new()
    };
    AudioCycleDiagnosticDto {
        sequence: snapshot.sequence,
        captured_at_ms: snapshot.captured_at_ms,
        sample_rate_hz: snapshot.sample_rate_hz,
        captured_frame_count: snapshot.captured_frame_count,
        line,
        rms_dbfs: snapshot.rms_dbfs,
        peak_dbfs: snapshot.peak_dbfs,
        low_band_rms_dbfs: snapshot.low_band_rms_dbfs,
        cutoff_hz: snapshot.cutoff_hz,
        has_pcm: snapshot.has_pcm,
    }
}

#[tauri::command]
pub fn list_audio_output_devices(
    window: Window,
    state: State<'_, AppState>,
) -> Result<Vec<AudioOutputDeviceDto>, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    autolive_portaudio_output::list_output_devices()
        .map(|devices| {
            devices
                .into_iter()
                .map(|device| AudioOutputDeviceDto {
                    id: device.id,
                    name: device.name,
                    host_api: host_api_label(device.host_api).to_owned(),
                    max_output_channels: device.max_output_channels,
                    default_sample_rate_hz: device.default_sample_rate_hz,
                })
                .collect()
        })
        .map_err(|message| CommandErrorDto::new("audio_output_devices_failed", message))
}

#[tauri::command]
pub async fn set_audio_output_backend(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: SetAudioOutputBackendRequestDto,
) -> Result<AudioOutputBackendStatusDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        set_audio_output_backend_blocking(app, state, request)
    })
    .await
    .map_err(|error| {
        CommandErrorDto::new(
            "audio_output_task_failed",
            format!("PortAudio 后台切换任务失败：{error}"),
        )
    })?
}

fn set_audio_output_backend_blocking(
    app: AppHandle,
    state: AppState,
    request: SetAudioOutputBackendRequestDto,
) -> Result<AudioOutputBackendStatusDto, CommandErrorDto> {
    if let Some(capacity_kib) = request.memory_buffer_kib {
        autolive_portaudio_output::validate_ring_capacity_kib(capacity_kib).map_err(|message| {
            CommandErrorDto::new("audio_output_invalid_memory_buffer", message)
        })?;
    }
    if let Some(frames) = request.frames_per_buffer {
        autolive_portaudio_output::validate_frames_per_buffer(frames).map_err(|message| {
            CommandErrorDto::new("audio_output_invalid_frames_per_buffer", message)
        })?;
    }
    if state
        .audio_output_reconfiguring
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(CommandErrorDto::new(
            "audio_output_busy",
            "PortAudio 设备正在切换，请稍候",
        ));
    }
    let result = (|| {
        {
            let mut preferred = state.audio_output_preferred.lock().map_err(|_| {
                CommandErrorDto::new("audio_output_lock_failed", "音频出口状态锁已损坏")
            })?;
            *preferred = request.prefer_portaudio;
        }
        state.stop_audio_mixer()?;
        state.stop_audio_cycle_output()?;
        reset_audio_output_callback_observation(&state);
        if !request.prefer_portaudio {
            return audio_output_status_dto(&state);
        }

        let probe = autolive_portaudio_output::probe_portaudio();
        if !probe.available {
            let mut status = audio_output_status_dto(&state)?;
            status.reason = probe
                .reason
                .or_else(|| Some("PortAudio 不可用，保持 WebView".to_owned()));
            return Ok(status);
        }

        let sample_rate_hz = request
            .sample_rate_hz
            .filter(|hz| matches!(*hz, 44_100 | 48_000))
            .unwrap_or(autolive_portaudio_output::DEFAULT_SAMPLE_RATE_HZ);
        let config = AudioOutputConfig {
            device_index: request.device_index,
            sample_rate_hz,
            memory_buffer_kib: request
                .memory_buffer_kib
                .unwrap_or(autolive_portaudio_output::DEFAULT_RING_CAPACITY_KIB),
            frames_per_buffer: request
                .frames_per_buffer
                .unwrap_or(autolive_portaudio_output::DEFAULT_FRAMES_PER_BUFFER),
            channels: 2,
        };
        let output = AudioCycleOutputTask::start(config)
            .map_err(|error| CommandErrorDto::new("portaudio_start_failed", error))?;
        state
            .audio_cycle_output
            .lock()
            .map_err(|_| {
                CommandErrorDto::new("audio_cycle_output_lock_failed", "音频周期输出状态锁已损坏")
            })?
            .replace(output);

        let should_start_audio_mixer = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
            .snapshot()
            .playback_state
            == PlaybackState::Playing;
        if should_start_audio_mixer {
            if let Err(error) = state
                .start_audio_mixer_from_snapshot_unlocked(
                    &app,
                    sample_rate_hz,
                    request.audio_sync_clock(),
                )
                .and_then(|()| {
                    state.audio_output_control()?.resume().map_err(|reason| {
                        CommandErrorDto::new("audio_output_resume_failed", reason)
                    })
                })
            {
                if is_retryable_audio_mixer_error(&error.code) {
                    let mut status = audio_output_status_dto(&state)?;
                    status.reason = Some(format!(
                        "PortAudio PCM 尚未达到安全水位，保持 WebView 并等待自动重试：{}",
                        error.message
                    ));
                    status.reason_code = Some(error.code);
                    status.retryable = true;
                    return Ok(status);
                }
                let _ = state.stop_audio_mixer();
                let _ = state.stop_audio_cycle_output();
                if let Ok(mut preferred) = state.audio_output_preferred.lock() {
                    *preferred = false;
                }
                let mut status = audio_output_status_dto(&state)?;
                status.reason = Some(format!(
                    "PortAudio 启动或混音启用失败，已回退 WebView：{}",
                    error.message
                ));
                return Ok(status);
            }
        }
        audio_output_status_dto(&state)
    })();
    state
        .audio_output_reconfiguring
        .store(false, Ordering::Release);
    result
}

#[tauri::command]
pub async fn sync_audio_output_source(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: Option<SyncAudioOutputSourceRequestDto>,
) -> Result<AudioOutputBackendStatusDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        sync_audio_output_source_blocking(app, state, request)
    })
    .await
    .map_err(|error| {
        CommandErrorDto::new(
            "audio_source_sync_task_failed",
            format!("PortAudio 音频源后台同步任务失败：{error}"),
        )
    })?
}

#[tauri::command]
pub async fn prepare_audio_cycle_candidate(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: PrepareAudioCycleCandidateRequestDto,
) -> Result<PrepareAudioCycleCandidateResultDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _prepare_guard = state.audio_mixer_prepare_lock.lock().map_err(|_| {
            CommandErrorDto::new("audio_mixer_prepare_lock_failed", "候选音轨创建锁已损坏")
        })?;
        state.validate_audio_cycle_candidate_request(&request)?;
        let (operation_token, previous) = state.begin_audio_mixer_prepare()?;
        if let Some(mut previous) = previous {
            previous.stop_preserving_output();
        }
        if !state.audio_mixer_operation_is_current(operation_token) {
            return Err(CommandErrorDto::new(
                "audio_candidate_stale",
                "候选创建前收到更新的停止、暂停或切换请求",
            ));
        }
        let mut candidate = state.scheduled_audio_cycle_task(&app, &request, operation_token)?;
        let result = PrepareAudioCycleCandidateResultDto {
            candidate_id: request.candidate_id,
            state: if candidate.task.is_ready() {
                "ready".to_owned()
            } else {
                "preparing".to_owned()
            },
            target_absolute_position_ms: request.target_absolute_position_ms,
            buffered_ms: candidate.task.prebuffered_ms(),
            ffmpeg_pid: candidate.task.ffmpeg_pid(),
        };
        let replaced = {
            let _switch_guard = state.audio_mixer_switch_lock.lock().map_err(|_| {
                CommandErrorDto::new("audio_mixer_switch_lock_failed", "音频切换锁已损坏")
            })?;
            if !state.audio_mixer_operation_is_current(operation_token) {
                drop(_switch_guard);
                candidate.stop_preserving_output();
                return Err(CommandErrorDto::new(
                    "audio_candidate_stale",
                    "候选创建期间收到更新的停止、暂停或切换请求",
                ));
            }
            state
                .audio_mixer_pending
                .lock()
                .map_err(|_| {
                    CommandErrorDto::new(
                        "audio_mixer_pending_lock_failed",
                        "待切换音频混音状态锁已损坏",
                    )
                })?
                .replace(candidate)
        };
        if let Some(mut replaced) = replaced {
            replaced.stop_preserving_output();
        }
        Ok(result)
    })
    .await
    .map_err(|error| {
        CommandErrorDto::new(
            "audio_candidate_prepare_task_failed",
            format!("候选音轨后台准备任务失败：{error}"),
        )
    })?
}

#[tauri::command]
pub async fn commit_audio_cycle_candidate(
    window: Window,
    state: State<'_, AppState>,
    request: CommitAudioCycleCandidateRequestDto,
) -> Result<CommitAudioCycleCandidateResultDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        commit_audio_cycle_candidate_blocking(state, request)
    })
    .await
    .map_err(|error| {
        CommandErrorDto::new(
            "audio_candidate_commit_task_failed",
            format!("候选音轨后台提交任务失败：{error}"),
        )
    })?
}

fn commit_audio_cycle_candidate_blocking(
    state: AppState,
    request: CommitAudioCycleCandidateRequestDto,
) -> Result<CommitAudioCycleCandidateResultDto, CommandErrorDto> {
    let snapshot = state
        .playback
        .lock()
        .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
        .snapshot();
    let snapshot_dto = || PlaybackSnapshotDto::from(snapshot.clone());
    let duration_ms = source_duration_ms(&snapshot)?;
    let live_clock = match resolve_audio_sync_clock(
        AudioSyncClock::from(&request),
        snapshot.playback_generation,
        snapshot.loop_index,
        snapshot.current_position_ms,
        duration_ms,
    ) {
        Ok(clock) => clock,
        Err(reason) => {
            return Ok(CommitAudioCycleCandidateResultDto {
                candidate_id: request.candidate_id,
                committed: false,
                reason: Some(format!("绝对媒体时钟已失效：{reason}")),
                snapshot: snapshot_dto(),
            });
        }
    };

    let _commit_guard = match state.begin_audio_cycle_commit() {
        Ok(guard) => guard,
        Err(error) if error.code == "audio_cycle_commit_in_progress" => {
            return Err(CommandErrorDto::new(
                "audio_candidate_commit_busy",
                error.message,
            ));
        }
        Err(error) => return Err(error),
    };

    let live_absolute_position_ms = live_clock.absolute_position_ms;
    let rust_absolute_position_ms = absolute_media_position_ms(
        snapshot.loop_index,
        snapshot.current_position_ms,
        duration_ms,
    );
    let mut pending_guard = state.audio_mixer_pending.lock().map_err(|_| {
        CommandErrorDto::new(
            "audio_mixer_pending_lock_failed",
            "待切换音频混音状态锁已损坏",
        )
    })?;
    let Some(pending) = pending_guard.as_ref() else {
        return Ok(CommitAudioCycleCandidateResultDto {
            candidate_id: request.candidate_id,
            committed: false,
            reason: Some("候选音轨不存在或已取消".to_owned()),
            snapshot: snapshot_dto(),
        });
    };
    if pending.candidate_id() != Some(request.candidate_id) {
        return Ok(CommitAudioCycleCandidateResultDto {
            candidate_id: request.candidate_id,
            committed: false,
            reason: Some("候选 ID 已过期，保持当前音轨".to_owned()),
            snapshot: snapshot_dto(),
        });
    }
    let operation_token = pending.token;
    if !state.audio_mixer_operation_is_current(operation_token) {
        let stale = pending_guard.take();
        drop(pending_guard);
        if let Some(mut stale) = stale {
            stale.stop_preserving_output();
        }
        return Ok(CommitAudioCycleCandidateResultDto {
            candidate_id: request.candidate_id,
            committed: false,
            reason: Some("候选操作代次已经失效，保持当前音轨".to_owned()),
            snapshot: snapshot_dto(),
        });
    }
    if let Some(reason) = pending.task.failure() {
        let Some(mut failed) = pending_guard.take() else {
            return Err(CommandErrorDto::new(
                "audio_candidate_state_invalid",
                "候选状态在提交期间意外丢失",
            ));
        };
        drop(pending_guard);
        failed.stop_preserving_output();
        return Ok(CommitAudioCycleCandidateResultDto {
            candidate_id: request.candidate_id,
            committed: false,
            reason: Some(format!("候选 FFmpeg 已退出：{reason}")),
            snapshot: snapshot_dto(),
        });
    }
    if !pending.task.is_ready() {
        return Err(CommandErrorDto::new(
            "audio_candidate_not_ready",
            format!(
                "候选音轨仍在准备，当前已缓冲 {}ms",
                pending.task.prebuffered_ms()
            ),
        ));
    }
    let PendingAudioMixerKind::AudioCycle(candidate_state) = &pending.kind else {
        return Ok(CommitAudioCycleCandidateResultDto {
            candidate_id: request.candidate_id,
            committed: false,
            reason: Some("候选槽正在处理首次音频同步".to_owned()),
            snapshot: snapshot_dto(),
        });
    };
    let (pending_ms, output_latency_ms, _) = state.audio_output_timing_ms(pending.sample_rate_hz);
    let effective_absolute_position_ms = live_absolute_position_ms.max(rust_absolute_position_ms);
    if !audio_cycle_commit_due(
        effective_absolute_position_ms,
        candidate_state.target_absolute_position_ms,
        pending_ms,
        output_latency_ms,
        pending.playback_rate,
    ) {
        return Err(CommandErrorDto::new(
            "audio_candidate_not_due",
            "尚未到达候选音轨切换点，保留已预热候选",
        ));
    }
    if !pending.source_identity.matches_playing(&snapshot) {
        let Some(mut stale) = pending_guard.take() else {
            return Err(CommandErrorDto::new(
                "audio_candidate_state_invalid",
                "候选状态在过期清理期间意外丢失",
            ));
        };
        drop(pending_guard);
        stale.stop_preserving_output();
        return Ok(CommitAudioCycleCandidateResultDto {
            candidate_id: request.candidate_id,
            committed: false,
            reason: Some("播放轮次、媒体源或声音版本已经变化，旧候选已取消".to_owned()),
            snapshot: snapshot_dto(),
        });
    }

    let commit_absolute_position_ms = add_wall_clock_delay_to_media_position_ms(
        effective_absolute_position_ms,
        pending_ms.saturating_add(output_latency_ms),
        pending.playback_rate,
    );
    let candidate_pcm_position_ms = resolve_audio_candidate_pcm_position_ms(
        pending.candidate_start_absolute_position_ms,
        commit_absolute_position_ms,
        pending.playback_rate,
    );
    if let Err(reason) = pending
        .task
        .validate_commit_at_position(candidate_pcm_position_ms)
    {
        return Err(CommandErrorDto::new(
            "audio_candidate_not_ready",
            format!("候选音轨未覆盖当前画面，保留候选并继续预热：{reason}"),
        ));
    }
    let output_control = state.audio_output_control()?;
    let output_active = output_control.status().is_some_and(|status| {
        let health = status.health;
        health.application_running
            && matches!(
                health.hardware_state,
                autolive_portaudio_output::PortAudioHardwareState::Active
            )
    });
    if !output_active {
        let inactive = pending_guard.take();
        drop(pending_guard);
        if let Some(mut inactive) = inactive {
            inactive.stop_preserving_output();
        }
        return Ok(CommitAudioCycleCandidateResultDto {
            candidate_id: request.candidate_id,
            committed: false,
            reason: Some("PortAudio 硬件消费者未处于活动状态".to_owned()),
            snapshot: snapshot_dto(),
        });
    }

    let Some(pending) = pending_guard.take() else {
        return Err(CommandErrorDto::new(
            "audio_candidate_state_invalid",
            "候选状态在提交期间意外丢失",
        ));
    };
    drop(pending_guard);
    let PendingAudioMixerTask {
        token: operation_token,
        task: candidate,
        source_identity,
        playback_rate,
        kind: PendingAudioMixerKind::AudioCycle(candidate_state),
        ..
    } = pending
    else {
        return Err(CommandErrorDto::new(
            "audio_candidate_state_invalid",
            "候选状态类型不一致",
        ));
    };
    let old_exists = state
        .audio_mixer
        .lock()
        .map_err(|_| CommandErrorDto::new("audio_mixer_lock_failed", "音频混音状态锁已损坏"))?
        .is_some();
    if !old_exists {
        let mut candidate = candidate;
        candidate.stop_preserving_output();
        return Ok(CommitAudioCycleCandidateResultDto {
            candidate_id: request.candidate_id,
            committed: false,
            reason: Some("当前音轨不存在，不能安全切换候选".to_owned()),
            snapshot: snapshot_dto(),
        });
    }
    if let Err(reason) = state.ensure_audio_mixer_source_current(&source_identity) {
        let mut candidate = candidate;
        candidate.stop_preserving_output();
        return Ok(CommitAudioCycleCandidateResultDto {
            candidate_id: request.candidate_id,
            committed: false,
            reason: Some(reason.message),
            snapshot: snapshot_dto(),
        });
    }
    if let Err(reason) = candidate.commit_at_position(candidate_pcm_position_ms) {
        let mut candidate = candidate;
        candidate.stop_preserving_output();
        return Ok(CommitAudioCycleCandidateResultDto {
            candidate_id: request.candidate_id,
            committed: false,
            reason: Some(format!("候选音轨提交失败：{reason}")),
            snapshot: snapshot_dto(),
        });
    }
    if let Err(reason) = output_control.crossfade_to(
        candidate.output_track(),
        AudioTrackTimeline {
            media_position_ms: commit_absolute_position_ms,
            playback_rate,
        },
    ) {
        let mut candidate = candidate;
        candidate.stop_preserving_output();
        return Ok(CommitAudioCycleCandidateResultDto {
            candidate_id: request.candidate_id,
            committed: false,
            reason: Some(format!("候选音轨交叉淡化失败：{reason}")),
            snapshot: snapshot_dto(),
        });
    }
    let mut candidate = Some(candidate);
    let replacement = {
        let _switch_guard = state.audio_mixer_switch_lock.lock().map_err(|_| {
            CommandErrorDto::new("audio_mixer_switch_lock_failed", "音频切换锁已损坏")
        })?;
        if !state.audio_mixer_operation_is_current(operation_token) {
            None
        } else {
            let next = candidate.take().ok_or_else(|| {
                CommandErrorDto::new("audio_candidate_state_invalid", "候选音轨所有权已丢失")
            })?;
            let old = state
                .audio_mixer
                .lock()
                .map_err(|_| {
                    CommandErrorDto::new("audio_mixer_lock_failed", "音频混音状态锁已损坏")
                })?
                .replace(next);
            Some(old)
        }
    };
    let Some(old) = replacement else {
        let _ = output_control.clear();
        if let Some(mut candidate) = candidate {
            candidate.stop_preserving_output();
        }
        return Ok(CommitAudioCycleCandidateResultDto {
            candidate_id: request.candidate_id,
            committed: false,
            reason: Some("提交期间收到更新的停止、暂停或切换请求".to_owned()),
            snapshot: snapshot_dto(),
        });
    };
    let committed_snapshot = {
        let mut playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        playback.commit_validated_audio_stream_configuration(candidate_state.configuration);
        PlaybackSnapshotDto::from(playback.snapshot())
    };
    if let Some(mut old) = old {
        old.stop_preserving_output();
    }
    Ok(CommitAudioCycleCandidateResultDto {
        candidate_id: request.candidate_id,
        committed: true,
        reason: None,
        snapshot: committed_snapshot,
    })
}

#[tauri::command]
pub async fn cancel_audio_cycle_candidate(
    window: Window,
    state: State<'_, AppState>,
    request: CancelAudioCycleCandidateRequestDto,
) -> Result<CancelAudioCycleCandidateResultDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 与 prepare 使用同一串行锁；等待创建完成后才能判断 ID，避免旧 cancel
        // 在新候选尚未写入 pending 时误推进全局 token。
        let _prepare_guard = state.audio_mixer_prepare_lock.lock().map_err(|_| {
            CommandErrorDto::new("audio_mixer_prepare_lock_failed", "候选音轨创建锁已损坏")
        })?;
        let (matches, candidate) = {
            let _switch_guard = state.audio_mixer_switch_lock.lock().map_err(|_| {
                CommandErrorDto::new("audio_mixer_switch_lock_failed", "音频切换锁已损坏")
            })?;
            let mut pending = state.audio_mixer_pending.lock().map_err(|_| {
                CommandErrorDto::new(
                    "audio_mixer_pending_lock_failed",
                    "待切换音频混音状态锁已损坏",
                )
            })?;
            let matches = pending.as_ref().is_some_and(|candidate| {
                audio_cycle_cancel_matches_pending(
                    matches!(candidate.kind, PendingAudioMixerKind::AudioCycle(_)),
                    candidate.candidate_id(),
                    request.candidate_id,
                )
            });
            if matches {
                state.next_audio_mixer_pending_token();
            }
            let candidate = matches.then(|| pending.take()).flatten();
            (matches, candidate)
        };
        let cancelled_id = candidate
            .as_ref()
            .and_then(PendingAudioMixerTask::candidate_id);
        if let Some(mut candidate) = candidate {
            candidate.stop_preserving_output();
        }
        Ok(CancelAudioCycleCandidateResultDto {
            candidate_id: cancelled_id.or(request.candidate_id),
            cancelled: matches,
        })
    })
    .await
    .map_err(|error| {
        CommandErrorDto::new(
            "audio_candidate_cancel_task_failed",
            format!("候选音轨后台取消任务失败：{error}"),
        )
    })?
}

fn sync_audio_output_source_blocking(
    app: AppHandle,
    state: AppState,
    request: Option<SyncAudioOutputSourceRequestDto>,
) -> Result<AudioOutputBackendStatusDto, CommandErrorDto> {
    let request = match request {
        Some(request) => request,
        None => {
            let snapshot = state
                .playback
                .lock()
                .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
                .snapshot();
            let clock = snapshot_audio_sync_clock(&snapshot)?;
            SyncAudioOutputSourceRequestDto {
                position_ms: clock.position_ms,
                duration_ms: clock.duration_ms,
                absolute_position_ms: clock.absolute_position_ms,
                loop_index: clock.loop_index,
                playback_generation: clock.playback_generation,
                recover_unhealthy: false,
                reanchor_loop_boundary: false,
            }
        }
    };
    let preferred = *state
        .audio_output_preferred
        .lock()
        .map_err(|_| CommandErrorDto::new("audio_output_lock_failed", "音频出口状态锁已损坏"))?;
    if !preferred {
        return audio_output_status_dto(&state);
    }
    let snapshot = state
        .playback
        .lock()
        .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
        .snapshot();
    if snapshot.playback_state != PlaybackState::Playing {
        state.pause_audio_output()?;
        return audio_output_status_dto(&state);
    }
    let requested_clock = resolve_audio_sync_clock(
        AudioSyncClock::from(&request),
        snapshot.playback_generation,
        snapshot.loop_index,
        snapshot.current_position_ms,
        source_duration_ms(&snapshot)?,
    )
    .map_err(|reason| CommandErrorDto::new("audio_sync_clock_invalid", reason))?;
    let sample_rate_hz = state
        .audio_output_control()?
        .status()
        .map(|status| status.sample_rate_hz)
        .unwrap_or(autolive_portaudio_output::DEFAULT_SAMPLE_RATE_HZ);
    let status_before_sync = audio_output_status_dto(&state)?;
    if status_before_sync.reason_code.as_deref() == Some(AUDIO_OUTPUT_RESUME_REQUIRED_CODE) {
        state
            .audio_output_control()?
            .resume()
            .map_err(|reason| CommandErrorDto::new("audio_output_resume_failed", reason))?;
        reset_audio_output_callback_observation(&state);
        return audio_output_status_dto(&state);
    }
    if request.recover_unhealthy {
        let status = audio_output_status_dto(&state)?;
        if !status.recovery_required {
            return Ok(status);
        }
    }
    let _source_sync_guard = match state.begin_audio_mixer_recovery() {
        Ok(guard) => guard,
        Err(error) => {
            let mut status = audio_output_status_dto(&state)?;
            status.reason = Some(error.message);
            status.reason_code = Some(error.code);
            status.retryable = true;
            return Ok(status);
        }
    };
    let had_previous_mixer = state
        .audio_mixer
        .lock()
        .map_err(|_| CommandErrorDto::new("audio_mixer_lock_failed", "音频混音状态锁已损坏"))?
        .is_some();
    if had_previous_mixer {
        let pending_in_progress = state
            .audio_mixer_pending
            .lock()
            .map_err(|_| {
                CommandErrorDto::new(
                    "audio_mixer_pending_lock_failed",
                    "待切换音频混音状态锁已损坏",
                )
            })?
            .is_some();
        if should_defer_source_sync_for_pending_candidate(
            pending_in_progress,
            request.recover_unhealthy,
            request.reanchor_loop_boundary,
        ) {
            let mut status = audio_output_status_dto(&state)?;
            status.reason = Some("候选音轨正在预热，继续保持旧轨".to_owned());
            return Ok(status);
        }

        let (pending_ms, output_latency_ms, playback_watermark_ms) =
            state.audio_output_timing_ms(sample_rate_hz);
        let preparation_started_at = Instant::now();
        let (operation_token, previous) = match state.begin_audio_source_sync_prepare() {
            Ok(operation) => operation,
            Err(error) if error.code == "audio_cycle_commit_in_progress" => {
                let mut status = audio_output_status_dto(&state)?;
                status.reason = Some(error.message);
                status.reason_code = Some(error.code);
                status.retryable = true;
                return Ok(status);
            }
            Err(error) => return Err(error),
        };
        if let Some(mut previous) = previous {
            previous.stop_preserving_output();
        }
        let prepare_guard = state.audio_mixer_prepare_lock.lock().map_err(|_| {
            CommandErrorDto::new("audio_mixer_prepare_lock_failed", "候选音轨创建锁已损坏")
        })?;
        if !state.audio_mixer_operation_is_current(operation_token) {
            return audio_output_status_dto(&state);
        }
        let Some((candidate, source_identity, playback_rate, candidate_start_absolute_position_ms)) =
            state.audio_mixer_task_from_snapshot(
                &app,
                sample_rate_hz,
                requested_clock,
                preparation_started_at,
                pending_ms.saturating_add(output_latency_ms),
                playback_watermark_ms,
            )?
        else {
            return audio_output_status_dto(&state);
        };
        let candidate = PendingAudioMixerTask {
            token: operation_token,
            task: candidate,
            source_identity: source_identity.clone(),
            sample_rate_hz,
            observed_absolute_position_ms: requested_clock.absolute_position_ms,
            candidate_start_absolute_position_ms,
            preparation_started_at,
            playback_rate,
            kind: PendingAudioMixerKind::SourceSync,
        };
        let mut candidate = Some(candidate);
        let previous = {
            let _switch_guard = state.audio_mixer_switch_lock.lock().map_err(|_| {
                CommandErrorDto::new("audio_mixer_switch_lock_failed", "音频切换锁已损坏")
            })?;
            if !state.audio_mixer_operation_is_current(operation_token) {
                None
            } else {
                let next = candidate.take().ok_or_else(|| {
                    CommandErrorDto::new("audio_mixer_candidate_missing", "候选音轨所有权已丢失")
                })?;
                Some(
                    state
                        .audio_mixer_pending
                        .lock()
                        .map_err(|_| {
                            CommandErrorDto::new(
                                "audio_mixer_pending_lock_failed",
                                "待切换音频混音状态锁已损坏",
                            )
                        })?
                        .replace(next),
                )
            }
        };
        let Some(previous) = previous else {
            if let Some(mut candidate) = candidate {
                candidate.stop_preserving_output();
            }
            return audio_output_status_dto(&state);
        };
        if let Some(mut previous) = previous {
            previous.stop_preserving_output();
        }
        drop(prepare_guard);

        let candidate = {
            let _switch_guard = state.audio_mixer_switch_lock.lock().map_err(|_| {
                CommandErrorDto::new("audio_mixer_switch_lock_failed", "音频切换锁已损坏")
            })?;
            let mut pending = state.audio_mixer_pending.lock().map_err(|_| {
                CommandErrorDto::new(
                    "audio_mixer_pending_lock_failed",
                    "待切换音频混音状态锁已损坏",
                )
            })?;
            if pending
                .as_ref()
                .is_some_and(|candidate| candidate.token == operation_token)
            {
                pending.take()
            } else {
                None
            }
        };
        let Some(candidate) = candidate else {
            return audio_output_status_dto(&state);
        };
        let PendingAudioMixerTask {
            token: _,
            task,
            source_identity,
            sample_rate_hz,
            observed_absolute_position_ms,
            candidate_start_absolute_position_ms,
            preparation_started_at,
            playback_rate,
            kind: _,
        } = candidate;
        let switch_result = state.commit_audio_mixer_candidate(
            task,
            sample_rate_hz,
            observed_absolute_position_ms,
            candidate_start_absolute_position_ms,
            preparation_started_at,
            playback_rate,
            &source_identity,
            operation_token,
        );
        if let Err(error) = switch_result {
            if is_retryable_audio_mixer_error(&error.code) {
                let mut status = audio_output_status_dto(&state)?;
                status.reason = Some(format!(
                    "PortAudio 候选仍在追赶，保持 WebView/旧轨并等待自动重试：{}",
                    error.message
                ));
                status.reason_code = Some(error.code);
                status.retryable = true;
                return Ok(status);
            }
            if request.recover_unhealthy {
                if let Ok(mut preferred) = state.audio_output_preferred.lock() {
                    *preferred = false;
                }
                let _ = state.stop_audio_mixer();
                let _ = state.stop_audio_cycle_output();
                reset_audio_output_callback_observation(&state);
                let mut status = audio_output_status_dto(&state)?;
                status.reason = Some(format!(
                    "PortAudio PCM 生产恢复失败，已回退 WebView：{}",
                    error.message
                ));
                return Ok(status);
            }
            let mut status = audio_output_status_dto(&state)?;
            status.reason = Some(format!("候选音轨未就绪，继续保持旧轨：{}", error.message));
            return Ok(status);
        }
        if request.recover_unhealthy {
            reset_audio_output_callback_observation(&state);
        }
        return audio_output_status_dto(&state);
    }

    let switch_result = state
        .start_audio_mixer_from_snapshot_unlocked(&app, sample_rate_hz, Some(requested_clock))
        .and_then(|()| {
            state
                .audio_output_control()?
                .resume()
                .map_err(|reason| CommandErrorDto::new("audio_output_resume_failed", reason))
        });
    if let Err(error) = switch_result {
        if is_retryable_audio_mixer_error(&error.code) {
            let mut status = audio_output_status_dto(&state)?;
            status.reason = Some(format!(
                "PortAudio PCM 尚未达到安全水位，保持 WebView 并等待自动重试：{}",
                error.message
            ));
            status.reason_code = Some(error.code);
            status.retryable = true;
            return Ok(status);
        }
        if let Ok(mut preferred) = state.audio_output_preferred.lock() {
            *preferred = false;
        }
        let _ = state.stop_audio_cycle_output();
        let mut status = audio_output_status_dto(&state)?;
        status.reason = Some(format!(
            "PortAudio 音频源不可用，已回退 WebView：{}",
            error.message
        ));
        return Ok(status);
    }
    audio_output_status_dto(&state)
}

#[tauri::command]
pub fn play_portaudio_test_tone(
    window: Window,
    state: State<'_, AppState>,
    request: PlayPortAudioTestToneRequestDto,
) -> Result<AudioOutputBackendStatusDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let preferred = *state
        .audio_output_preferred
        .lock()
        .map_err(|_| CommandErrorDto::new("audio_output_lock_failed", "音频出口状态锁已损坏"))?;
    if !preferred {
        return Err(CommandErrorDto::new(
            "portaudio_not_preferred",
            "请先切换到 PortAudio 出口",
        ));
    }
    state
        .audio_output_control()?
        .play_test_tone(AudioTestTone {
            frequency_hz: request.frequency_hz.unwrap_or(440.0),
            duration_ms: request.duration_ms.unwrap_or(500).min(2_000),
            amplitude: request.amplitude.unwrap_or(0.15),
        })
        .map_err(|error| CommandErrorDto::new("portaudio_tone_failed", error))?;
    audio_output_status_dto(&state)
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
    cleanup_media_processing_cache(&app, &state)
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
    let audio_variants = request.audio_variants.unwrap_or_default();
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
    if audio_enabled {
        build_audio_stream_filter_graph(
            &request.params.audio,
            &audio_variants,
            source_audio_sample_rate_hz,
            autolive_portaudio_output::DEFAULT_SAMPLE_RATE_HZ,
        )
        .map_err(|error| CommandErrorDto::new("audio_stream_filter_invalid", error.to_string()))?;
    }
    state.with_playback(&window, |playback| {
        if audio_enabled {
            playback
                .set_audio_stream_configuration(
                    request.params.audio.clone(),
                    audio_variants.clone(),
                )
                .map_err(|errors| {
                    CommandErrorDto::new(
                        "audio_stream_params_invalid",
                        errors
                            .iter()
                            .map(|error| format!("{}: {}", error.field, error.message))
                            .collect::<Vec<_>>()
                            .join("；"),
                    )
                })?;
        }
        playback
            .mark_media_processing_running()
            .map_err(command_error_from_playback)
    })?;
    // 普通声音处理不生成临时 MP4。PortAudio 直接从源视频解码并套用滤镜图；
    // 没有硬件出口时，WebView 仍播放原视频流并保留实时预览回退。
    if audio_enabled && !video_enabled {
        return state.with_playback(&window, |playback| {
            playback.mark_audio_processing_runtime();
            Ok(state.snapshot(playback))
        });
    }
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandErrorDto::new("media_cache_dir_failed", error.to_string()))?
        .join("media-processing");
    // ponytail: 开渲前不清缓存；正在播的 processed 被删会静音
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
        // 声音处理走 PortAudio 的实时 FFmpeg PCM 流，不写入视频缓存；
        // 视频缓存只编码视觉效果，并复制源 AAC 作为 WebView 回退音轨。
        audio_processing_enabled: false,
        source_audio_sample_rate_hz,
        video: request.params.video.clone(),
        audio: request.params.audio.clone(),
        audio_variants,
        research: request.params.research.clone(),
        timeout_seconds,
    };
    if let Err(error) = build_media_render_args(&media_request) {
        state.with_playback(&window, |playback| {
            playback.mark_media_processing_failed(error.to_string());
            Ok(())
        })?;
        return state.with_playback(&window, |playback| Ok(state.snapshot(playback)));
    }
    let cancellation = CancellationToken::new();
    let playback = Arc::clone(&state.playback);
    let completed = Arc::new(AtomicBool::new(false));
    let completed_for_thread = Arc::clone(&completed);
    let worker_cancellation = cancellation.clone();
    let thread_app = app.clone();
    let handle = thread::spawn(move || {
        let result = render_media(&media_request, &worker_cancellation);
        if let Ok(mut playback) = playback.lock() {
            let current_generation = playback.snapshot().playback_generation;
            if current_generation == generation {
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
            } else {
                // 根因：generation 漂移时若不清 processing，UI 会永久卡死并拒启新任务
                playback.mark_media_processing_failed(format!(
                    "媒体处理结果已过期（generation {generation} → {current_generation}），请重新应用"
                ));
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
    let _ = state.stop_audio_mixer();
    // 关窗释放 PortAudio，避免设备被占。
    if let Ok(mut preferred) = state.audio_output_preferred.lock() {
        *preferred = false;
    }
    let _ = state.stop_audio_cycle_output();
    // ponytail: 关最终效果窗时关掉声音/视频处理；实时幻化一并关
    let playback = Arc::clone(&state.playback);
    let lock_result = playback.lock();
    if let Ok(mut guard) = lock_result {
        guard.set_processing_switches(false, false, false);
    }
}

fn attach_final_effect_close_cleanup(app: &AppHandle, window: &tauri::WebviewWindow) {
    let app_handle = app.clone();
    window.on_window_event(move |event| {
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
        let window =
            WebviewWindowBuilder::new(&app, "final-effect", WebviewUrl::App("index.html".into()))
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

    let previous_logical_width = (f64::from(previous_size.width) / scale_factor).round() as u32;
    let previous_logical_height = (f64::from(previous_size.height) / scale_factor).round() as u32;
    // ponytail: 同尺寸不 center，避免声音处理 ready 时整窗跳回屏幕中央
    if previous_logical_width == target.width && previous_logical_height == target.height {
        return Ok(FinalEffectWindowSizeDto {
            width: target.width,
            height: target.height,
        });
    }

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
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    let snapshot = state.playback_action(&window, PlaybackCore::start)?;
    state.resume_audio_output(&app)?;
    Ok(snapshot)
}

#[tauri::command]
pub fn pause_playback(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    let snapshot = state.playback_action(&window, PlaybackCore::pause)?;
    state.pause_audio_output()?;
    Ok(snapshot)
}

#[tauri::command]
pub fn resume_playback(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    let snapshot = state.playback_action(&window, PlaybackCore::resume)?;
    state.resume_audio_output(&app)?;
    Ok(snapshot)
}

#[tauri::command]
pub fn update_playback_position(
    window: Window,
    state: State<'_, AppState>,
    request: UpdatePlaybackPositionRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    let snapshot = state.with_playback_window(&window, |playback| {
        playback.set_playback_position(request.position_ms);
        Ok(state.snapshot(playback))
    })?;
    if let Ok(mut observed_at) = state.playback_position_observed_at.lock() {
        *observed_at = Some(Instant::now());
    }
    Ok(snapshot)
}

#[tauri::command]
pub fn stop_playback(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.stop_audio_for_playback()?;
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
    request: CompletePlaybackLoopRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.stop_speech_worker()?;
    state.with_playback_window(&window, |playback| {
        let snapshot = playback.snapshot();
        match should_complete_playback_loop(
            snapshot.playback_generation,
            snapshot.loop_index,
            request.playback_generation,
            request.target_loop_index,
        ) {
            Ok(false) => return Ok(state.snapshot(playback)),
            Err(message) => {
                return Err(CommandErrorDto::new(
                    "invalid_playback_loop_target",
                    message,
                ));
            }
            Ok(true) => {}
        }
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
    let snapshot = state.with_playback(&window, |playback| {
        playback.set_processing_switches(
            request.video_processing_enabled,
            request.audio_processing_enabled,
            request.realtime_audio_variant_enabled,
        );
        Ok(state.snapshot(playback))
    })?;
    if !request.audio_processing_enabled && !request.realtime_audio_variant_enabled {
        state.stop_audio_mixer()?;
    }
    Ok(snapshot)
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
    let worker_running = state.media_worker_is_running()?;
    let clear_stale_processing = |playback: &mut PlaybackCore| {
        let snapshot = playback.snapshot();
        let stuck = !worker_running
            && (snapshot.audio_processing_status == "processing"
                || snapshot.video_processing_status == "processing");
        if stuck {
            playback.mark_media_processing_failed(
                "媒体处理 Worker 已结束但状态未更新，已恢复可用（请重新应用）",
            );
        }
    };
    if window.label() == "final-effect" {
        let mut playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        clear_stale_processing(&mut playback);
        return Ok(state.snapshot(&playback));
    }
    state.with_playback(&window, |playback| {
        clear_stale_processing(playback);
        Ok(state.snapshot(playback))
    })
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
    use super::{
        absolute_media_position_ms, add_wall_clock_delay_to_media_position_ms,
        audio_cycle_cancel_matches_pending, audio_cycle_commit_due, is_retryable_audio_mixer_error,
        resolve_audio_candidate_pcm_position_ms, resolve_audio_commit_position_ms,
        resolve_audio_output_latency_ms, resolve_audio_sync_clock,
        scheduled_candidate_commit_tail_ms, should_complete_playback_loop,
        should_defer_source_sync_for_pending_candidate, signed_millis_delta,
        take_pending_audio_mixer, AppState, AudioSyncClock,
    };
    use std::sync::atomic::Ordering;

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

    #[test]
    fn absolute_audio_sync_clock_survives_a_loop_boundary() {
        let requested = AudioSyncClock {
            playback_generation: 9,
            loop_index: 1,
            position_ms: 2_700,
            duration_ms: 72_300,
            absolute_position_ms: 75_000,
        };
        assert_eq!(
            resolve_audio_sync_clock(requested, 9, 1, 2_500, 72_300),
            Ok(requested)
        );
        assert!(resolve_audio_sync_clock(requested, 9, 0, 72_100, 72_300).is_err());
        assert!(resolve_audio_sync_clock(
            AudioSyncClock {
                absolute_position_ms: 2_700,
                ..requested
            },
            9,
            1,
            2_500,
            72_300,
        )
        .is_err());
    }

    #[test]
    fn audio_commit_position_includes_preheat_and_queued_output_delay() {
        assert_eq!(
            resolve_audio_commit_position_ms(Some(22_000), 21_475, 500, 1.0, 50, 100),
            22_650
        );
        assert_eq!(
            resolve_audio_commit_position_ms(Some(21_017), 21_475, 500, 1.0, 50, 100),
            21_667
        );
        assert_eq!(
            resolve_audio_commit_position_ms(None, 21_475, 500, 2.0, 50, 100),
            21_775
        );
        assert_eq!(
            resolve_audio_commit_position_ms(Some(22_000), 21_475, 500, 1.25, 50, 100),
            22_813
        );
    }

    #[test]
    fn candidate_commit_becomes_due_before_target_by_queued_audio() {
        assert!(!audio_cycle_commit_due(9_749, 10_000, 200, 50, 1.0));
        assert!(audio_cycle_commit_due(9_750, 10_000, 200, 50, 1.0));
        assert!(audio_cycle_commit_due(9_500, 10_000, 200, 50, 2.0));
    }

    #[test]
    fn scheduled_candidate_commit_tail_covers_the_dynamic_playback_watermark() {
        assert_eq!(scheduled_candidate_commit_tail_ms(0), 130);
        assert_eq!(scheduled_candidate_commit_tail_ms(200), 200);
        assert_eq!(scheduled_candidate_commit_tail_ms(500), 500);
        assert_eq!(scheduled_candidate_commit_tail_ms(10_000), 750);
    }

    #[test]
    fn wall_clock_output_delay_scales_to_media_time_safely() {
        assert_eq!(
            add_wall_clock_delay_to_media_position_ms(10_000, 1_000, 0.5),
            10_500
        );
        assert_eq!(
            add_wall_clock_delay_to_media_position_ms(10_000, 1_000, 1.0),
            11_000
        );
        assert_eq!(
            add_wall_clock_delay_to_media_position_ms(10_000, 1_000, 2.0),
            12_000
        );
        for invalid_rate in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0] {
            assert_eq!(
                add_wall_clock_delay_to_media_position_ms(10_000, 1_000, invalid_rate),
                11_000
            );
        }
        assert_eq!(
            add_wall_clock_delay_to_media_position_ms(u64::MAX - 10, 100, 2.0),
            u64::MAX
        );
        assert_eq!(
            add_wall_clock_delay_to_media_position_ms(1, u64::MAX, 2.0),
            u64::MAX
        );
    }

    #[test]
    fn audio_output_latency_uses_the_larger_portaudio_clock_without_double_counting() {
        assert_eq!(resolve_audio_output_latency_ms(Some(100_001), 150_000), 150);
        assert_eq!(resolve_audio_output_latency_ms(Some(100_001), -10), 101);
        assert_eq!(resolve_audio_output_latency_ms(None, 49_001), 50);
        assert_eq!(resolve_audio_output_latency_ms(None, 0), 0);
    }

    #[test]
    fn candidate_trim_converts_media_time_to_post_atempo_pcm_time() {
        assert_eq!(
            resolve_audio_candidate_pcm_position_ms(10_000, 11_000, 1.0),
            11_000
        );
        assert_eq!(
            resolve_audio_candidate_pcm_position_ms(10_000, 11_000, 2.0),
            10_500
        );
        assert_eq!(
            resolve_audio_candidate_pcm_position_ms(10_000, 11_000, 0.5),
            12_000
        );
        assert_eq!(
            resolve_audio_candidate_pcm_position_ms(10_000, 9_000, 2.0),
            10_000
        );
    }

    #[test]
    fn absolute_media_position_keeps_loop_identity_without_overflow() {
        assert_eq!(absolute_media_position_ms(2, 1_500, 1_000), 3_000);
        assert_eq!(
            absolute_media_position_ms(u64::MAX, u64::MAX, 1_000),
            u64::MAX
        );
    }

    #[test]
    fn playback_loop_completion_is_idempotent_and_rejects_stale_or_skipped_targets() {
        assert_eq!(should_complete_playback_loop(7, 3, 7, 4), Ok(true));
        assert_eq!(should_complete_playback_loop(7, 4, 7, 4), Ok(false));
        assert!(should_complete_playback_loop(7, 3, 6, 4).is_err());
        assert!(should_complete_playback_loop(7, 3, 7, 5).is_err());
    }

    #[test]
    fn loop_boundary_reanchor_keeps_a_regular_pending_candidate() {
        assert!(should_defer_source_sync_for_pending_candidate(
            true, false, false
        ));
        assert!(should_defer_source_sync_for_pending_candidate(
            true, false, true
        ));
        assert!(!should_defer_source_sync_for_pending_candidate(
            true, true, false
        ));
    }

    #[test]
    fn signed_audio_offset_preserves_direction() {
        assert_eq!(signed_millis_delta(3_050, 3_000), 50);
        assert_eq!(signed_millis_delta(2_950, 3_000), -50);
    }

    #[test]
    fn candidate_transition_is_retryable_without_closing_portaudio() {
        assert!(is_retryable_audio_mixer_error(
            "audio_mixer_candidate_not_caught_up"
        ));
        assert!(is_retryable_audio_mixer_error(
            "audio_mixer_candidate_superseded"
        ));
        assert!(is_retryable_audio_mixer_error(
            "audio_mixer_candidate_stale"
        ));
        assert!(is_retryable_audio_mixer_error("audio_mixer_start_stale"));
        assert!(!is_retryable_audio_mixer_error(
            "audio_mixer_candidate_ffmpeg_failed"
        ));
    }

    #[test]
    fn source_sync_reservation_blocks_competing_sync_and_releases_on_drop() {
        let state = AppState::default();
        let recovery = state
            .begin_audio_mixer_recovery()
            .expect("first PCM recovery should reserve the mixer");

        let error = state
            .begin_audio_mixer_recovery()
            .expect_err("a second PCM recovery must not replace the active one");
        assert_eq!(error.code, "audio_mixer_recovery_in_progress");
        assert!(state
            .audio_mixer_recovery_in_progress
            .load(Ordering::Acquire));

        drop(recovery);
        assert!(!state
            .audio_mixer_recovery_in_progress
            .load(Ordering::Acquire));
        assert!(state.begin_audio_mixer_recovery().is_ok());
    }

    #[test]
    fn source_sync_reservation_blocks_cycle_prepare_without_advancing_token() {
        let state = AppState::default();
        let _source_sync = state
            .begin_audio_mixer_recovery()
            .expect("source sync should reserve the mixer");
        let initial_token = state.audio_mixer_pending_token.load(Ordering::Acquire);

        let error = state
            .begin_audio_mixer_prepare()
            .expect_err("N+1 prepare must wait for source sync to finish");

        assert_eq!(error.code, "audio_candidate_recovery_in_progress");
        assert_eq!(
            state.audio_mixer_pending_token.load(Ordering::Acquire),
            initial_token
        );

        let (source_sync_token, previous) = state
            .begin_audio_source_sync_prepare()
            .expect("the guard owner must still be able to prepare its source");
        assert!(source_sync_token > initial_token);
        assert!(previous.is_none());

        let cancelled_token = state.next_audio_mixer_pending_token();
        assert!(cancelled_token > source_sync_token);
    }

    #[test]
    fn stale_cycle_cancel_cannot_invalidate_an_unpublished_candidate() {
        assert!(audio_cycle_cancel_matches_pending(true, Some(7), Some(7)));
        assert!(!audio_cycle_cancel_matches_pending(true, Some(7), Some(8)));
        assert!(audio_cycle_cancel_matches_pending(true, Some(7), None));
        assert!(!audio_cycle_cancel_matches_pending(false, None, None));
    }

    #[test]
    fn pending_candidate_slot_is_independent_from_switch_lock() {
        let state = AppState::default();
        let _switch_guard = state
            .audio_mixer_switch_lock
            .lock()
            .expect("switch lock should be available");
        assert!(state.audio_mixer_pending.try_lock().is_ok());
    }

    #[test]
    fn candidate_prepare_cannot_invalidate_an_active_crossfade_commit() {
        let state = AppState::default();
        let initial_token = state.audio_mixer_pending_token.load(Ordering::Acquire);
        let commit_guard = state
            .begin_audio_cycle_commit()
            .expect("first commit guard should start");

        let error = state
            .begin_audio_mixer_prepare()
            .expect_err("prepare must wait until the crossfade commit finishes");
        assert_eq!(error.code, "audio_cycle_commit_in_progress");
        assert_eq!(
            state.audio_mixer_pending_token.load(Ordering::Acquire),
            initial_token
        );

        drop(commit_guard);
        let (next_token, previous) = state
            .begin_audio_mixer_prepare()
            .expect("prepare should resume after commit guard drops");
        assert!(next_token > initial_token);
        assert!(previous.is_none());
    }

    #[test]
    fn cancelling_pending_candidate_does_not_touch_current_slot() {
        let pending = std::sync::Mutex::new(Some("candidate"));
        let current = Some("old");

        assert_eq!(
            take_pending_audio_mixer(&pending).expect("pending lock"),
            Some("candidate")
        );
        assert_eq!(current, Some("old"));
        assert_eq!(
            take_pending_audio_mixer(&pending).expect("pending lock"),
            None
        );
    }
}
