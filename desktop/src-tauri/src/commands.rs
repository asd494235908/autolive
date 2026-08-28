use crate::audio_cycle_switch::{
    AudioCycleCandidate, AudioMixerSourceIdentity, PendingAudioMixerKind, PendingAudioMixerTask,
};
use autolive_desktop_core::ambient_sound::{
    resolve_ambient_sound, revalidate_ambient_sound, AmbientSoundSource, ResolvedAmbientSound,
};
use autolive_desktop_core::audio_cycle_output::{
    AudioCycleOutputControl, AudioCycleOutputTask, AudioInterludeMixConfig, AudioOutputConfig,
    AudioTestTone, AudioTrackTimeline, AUDIO_CANDIDATE_DIGITAL_SILENCE_ERROR,
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
    prepare_interlude_snapshot, InterludeAudioSelectionMode, InterludeAudioVariationMode,
    InterludeConfig, InterludeError, InterludeSnapshot,
};
use autolive_desktop_core::media_compatibility::{
    cleanup_compatibility_files, prepare_media_compatibility, MediaCompatibilityRequest,
};
use autolive_desktop_core::media_effect_params::{
    AudioEffectParams, MediaEffectParams, ParameterValidationError,
};
use autolive_desktop_core::media_engine::{
    build_audio_stream_filter_graph_with_ambient, build_media_render_args,
    configured_media_engine_paths_with_resource_dir,
    configured_media_engine_status_with_resource_dir, render_media_with_progress, target_triple,
    validate_audio_input_decodable, MediaEngineStatus, MediaRenderRequest, MediaRenderTarget,
    FFMPEG_PATH_ENV, FFPROBE_PATH_ENV,
};
use autolive_desktop_core::media_library::{
    probe_user_selected_video_with_ffprobe, MediaKind, MediaProbeRequestDto, MediaProbeResultDto,
    SourceMediaDto,
};
use autolive_desktop_core::media_video_effects::build_atomic_media_video_effect_plan;
use autolive_desktop_core::media_video_gpu_effects::{
    build_gpu83_shader_snapshot, Gpu83ParameterCapability, GPU83_PARAMETER_MAPPINGS,
};
use autolive_desktop_core::realtime_video_backend::{
    resolve_mpv_executable, resolve_mpv_shader, CompiledRealtimeParameters, MpvCommand,
    MpvShaderOptions, ParameterSupportReport, ParameterSupportResult, RealtimeVideoParameter,
    RealtimeVideoPlan, VideoBackend, VideoCommitGate, VideoPlanIdentity, VideoPlanSlot,
};
use autolive_desktop_core::realtime_video_runtime::{
    CycleSlotState, CycleSlotStatus, MediaVideoBackendRuntimeStatus, PrepareRealtimeRenderer,
    RealtimeVideoRuntime,
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
use autolive_desktop_core::webview_interlude_cache::{
    cleanup_webview_interlude_cache, render_webview_interlude_cache, WebViewInterludeCacheRequest,
};
use autolive_desktop_core::window_sizing::{calculate_window_size, WindowSizingError};
use autolive_desktop_core::{
    MediaProcessingCommitOutcome, PendingAudioMediaCandidateIdentity,
    PendingMediaCandidateIdentity, PlaybackCore, PlaybackSnapshot, PlaybackState,
    ValidatedAudioStreamConfiguration, MAX_SOURCE_MEDIA_POOL_ITEMS,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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

const MACOS_NATIVE_TITLEBAR_HEIGHT: f64 = 32.0;
const MEDIA_IMPORT_PROBE_TIMEOUT_MS: u64 = 10_000;
const MEDIA_IMPORT_COMPATIBILITY_TIMEOUT_SECONDS: u64 = 6 * 60 * 60;
const MAX_SOURCE_MEDIA_PATH_BYTES: usize = 32 * 1024;
const PORTAUDIO_CALLBACK_STALL_MS: u64 = 1_500;
const PORTAUDIO_PCM_STALL_MS: u64 = 1_500;
// FFmpeg 的多支路/特征滤镜需要完成初始化后才会输出首批 PCM。
const PORTAUDIO_SWITCH_READY_TIMEOUT_MS: u64 = 5_000;
const PORTAUDIO_SWITCH_READY_POLL_MS: u64 = 10;
const AUDIO_CYCLE_CANDIDATE_PRE_ROLL_MS: u64 = 250;
const AUDIO_CYCLE_CANDIDATE_BUFFER_MS: usize = 750;
const MAX_INTERLUDE_MEDIA_POSITION_MS: u64 = 24 * 60 * 60 * 1_000;
const VIDEO_BACKEND_SOURCE_REVISION: u64 = 0;
const MEDIA_CANDIDATE_TIMEOUT_SECONDS: u64 = 30 * 60;
const MEDIA_WORKER_STOP_BUDGET: Duration = Duration::from_secs(3);
struct PreparedMediaImports {
    results: Vec<MediaProbeResultDto>,
    created_compatibility_files: Vec<PathBuf>,
}

#[derive(Clone)]
struct MediaImportLease {
    generation: u64,
    cancellation: CancellationToken,
}

impl PreparedMediaImports {
    fn retain_created_files(&mut self) {
        self.created_compatibility_files.clear();
    }
}

impl Drop for PreparedMediaImports {
    fn drop(&mut self) {
        cleanup_compatibility_files(std::mem::take(&mut self.created_compatibility_files));
    }
}
const AUDIO_CYCLE_TARGET_HORIZON_MS: u64 = 60_000;
const AUDIO_SYNC_CLOCK_EXPIRED: &str = "绝对媒体时间已经过期";
const AUDIO_OUTPUT_RESUME_REQUIRED_CODE: &str = "audio_output_resume_required";
const AUDIO_CANDIDATE_OUTSIDE_SOURCE_AUDIO_WINDOW_CODE: &str =
    "audio_candidate_outside_source_audio_window";

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
            | "audio_mixer_candidate_silent"
            | "audio_mixer_start_stale"
    )
}

fn audio_crossfade_error_code(error: &str) -> &'static str {
    if error == AUDIO_CANDIDATE_DIGITAL_SILENCE_ERROR {
        "audio_mixer_candidate_silent"
    } else {
        "audio_mixer_crossfade_failed"
    }
}

fn audio_cycle_crossfade_failure(error: String) -> (String, Option<&'static str>) {
    if error == AUDIO_CANDIDATE_DIGITAL_SILENCE_ERROR {
        let code = audio_crossfade_error_code(&error);
        (error, Some(code))
    } else {
        (format!("候选音轨交叉淡化失败：{error}"), None)
    }
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

fn audio_cycle_target_is_within_horizon(
    observed_absolute_position_ms: u64,
    observed_elapsed_ms: u64,
    playback_rate: f64,
    target_absolute_position_ms: u64,
) -> bool {
    let live_absolute_position_ms = add_wall_clock_delay_to_media_position_ms(
        observed_absolute_position_ms,
        observed_elapsed_ms,
        playback_rate,
    );
    target_absolute_position_ms > live_absolute_position_ms
        && target_absolute_position_ms
            <= live_absolute_position_ms.saturating_add(AUDIO_CYCLE_TARGET_HORIZON_MS)
}

fn audio_cycle_target_validation_code(
    observed_absolute_position_ms: u64,
    observed_elapsed_ms: u64,
    playback_rate: f64,
    target_absolute_position_ms: u64,
) -> Option<&'static str> {
    let live_absolute_position_ms = add_wall_clock_delay_to_media_position_ms(
        observed_absolute_position_ms,
        observed_elapsed_ms,
        playback_rate,
    );
    if target_absolute_position_ms <= live_absolute_position_ms {
        Some("audio_candidate_stale")
    } else if !audio_cycle_target_is_within_horizon(
        observed_absolute_position_ms,
        observed_elapsed_ms,
        playback_rate,
        target_absolute_position_ms,
    ) {
        Some("audio_candidate_target_invalid")
    } else {
        None
    }
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
                "源媒体时长不可用，无法建立候选音轨绝对时间轴",
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
    shutting_down: Arc<AtomicBool>,
    playback: Arc<Mutex<PlaybackCore>>,
    /// 串行化播放池原子替换和播放结束推进；禁止持有 playback 锁等待资源退出。
    playback_transition: Arc<Mutex<()>>,
    /// 最终效果窗口最近一次上报视频 currentTime 的单调近似墙钟时间。
    playback_position_observed_at: Arc<Mutex<Option<Instant>>>,
    speech_worker: Arc<Mutex<Option<BackgroundWorkerTask>>>,
    video_media_worker: Arc<Mutex<Option<BackgroundWorkerTask>>>,
    audio_media_worker: Arc<Mutex<Option<BackgroundWorkerTask>>>,
    /// 导入批次的最新取消令牌；新的导入、池编辑和退出会使旧批次停止并回滚。
    media_import_cancellation: Arc<Mutex<Option<CancellationToken>>>,
    media_import_generation: Arc<AtomicU64>,
    runtime_resource_task: Arc<RuntimeResourceTask>,
    /// 按值拥有 PortAudio 流和唯一环缓生产线程；None = WebView。
    audio_cycle_output: Arc<Mutex<Option<AudioCycleOutputTask>>>,
    /// FFmpeg 解码线程 → 音频混音线程；PortAudio 失败时整体停止并回退 WebView。
    audio_mixer: Arc<Mutex<Option<AudioMixerTask>>>,
    /// 随机插话只生产 PCM；实际叠加和 duck 仍由唯一 audio_cycle_output 完成。
    interlude_mixer: Arc<Mutex<Option<ActiveInterludeMixer>>>,
    /// 串行化插话的创建/停止，避免并发 IPC 短暂生成多个 FFmpeg 解码器。
    interlude_prepare_lock: Arc<Mutex<()>>,
    /// 每次准备/切换/停止都会推进；阻塞解码只允许提交最新操作。
    interlude_prepare_token: Arc<AtomicU64>,
    /// 活动插话会话代际；同一文件内的预设切换必须匹配该代际。
    interlude_session_generation: Arc<AtomicU64>,
    /// WebView 插话缓存生成与显式清理互斥；不阻塞 PortAudio 插话控制。
    webview_interlude_cache_lock: Arc<Mutex<()>>,
    /// 已交给 WebView 播放或待播放的缓存；显式 release 前清理命令必须保留。
    protected_webview_interlude_paths: Arc<Mutex<HashSet<PathBuf>>>,
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
    /// 经路径校验和 FFmpeg 短时解码探测的环境声；用户素材优先，否则使用打包底噪。
    ambient_sound: Arc<Mutex<Option<ResolvedAmbientSound>>>,
    /// 单一实时画面进程所有者；任何窗口关闭、停止或应用退出都从这里回收 mpv。
    realtime_video_runtime: Arc<Mutex<RealtimeVideoRuntime>>,
}

#[derive(Debug)]
struct ActiveInterludeMixer {
    generation: u64,
    source_path: PathBuf,
    task: AudioMixerTask,
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
struct BackgroundWorkerTask {
    cancellation: CancellationToken,
    completed: Arc<AtomicBool>,
    handle: thread::JoinHandle<()>,
}

fn cancel_background_worker(
    worker: &Mutex<Option<BackgroundWorkerTask>>,
    lock_error: &str,
) -> Result<(), String> {
    let worker = worker.lock().map_err(|_| lock_error.to_owned())?;
    if let Some(task) = worker.as_ref() {
        task.cancellation.cancel();
    }
    Ok(())
}

fn join_background_worker_until(
    worker: &Mutex<Option<BackgroundWorkerTask>>,
    lock_error: &str,
    started_at: Instant,
    budget: Duration,
) -> Result<RuntimeResourceTaskShutdown, String> {
    loop {
        let finished = {
            let mut worker = worker.lock().map_err(|_| lock_error.to_owned())?;
            let Some(task) = worker.as_ref() else {
                return Ok(RuntimeResourceTaskShutdown::Idle);
            };
            if task.handle.is_finished() {
                worker.take()
            } else {
                None
            }
        };
        if let Some(task) = finished {
            task.handle
                .join()
                .map_err(|_| "后台 Worker 异常终止".to_owned())?;
            return Ok(RuntimeResourceTaskShutdown::Joined);
        }
        let elapsed = started_at.elapsed();
        if elapsed >= budget {
            return Ok(RuntimeResourceTaskShutdown::TimedOut);
        }
        thread::sleep(
            budget
                .saturating_sub(elapsed)
                .min(Duration::from_millis(10)),
        );
    }
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

fn validate_audio_sync_clock(
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
        return Err(AUDIO_SYNC_CLOCK_EXPIRED);
    }
    Ok(requested)
}

fn resolve_audio_sync_clock(
    requested: AudioSyncClock,
    playback_generation: u64,
    loop_index: u64,
    current_position_ms: u64,
    duration_ms: u64,
) -> Result<AudioSyncClock, &'static str> {
    match validate_audio_sync_clock(
        requested,
        playback_generation,
        loop_index,
        current_position_ms,
        duration_ms,
    ) {
        Err(AUDIO_SYNC_CLOCK_EXPIRED) => Ok(AudioSyncClock {
            playback_generation,
            loop_index,
            position_ms: current_position_ms.min(duration_ms),
            duration_ms,
            absolute_position_ms: absolute_media_position_ms(
                loop_index,
                current_position_ms,
                duration_ms,
            ),
        }),
        result => result,
    }
}

fn audio_sync_clock_rejection_code(reason: &str) -> Option<&'static str> {
    (reason == AUDIO_SYNC_CLOCK_EXPIRED).then_some("audio_mixer_candidate_stale")
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

fn should_complete_playback_item(
    current_generation: u64,
    current_loop_index: u64,
    current_source_media_index: usize,
    requested_generation: u64,
    requested_loop_index: u64,
    requested_source_media_index: usize,
) -> Result<bool, &'static str> {
    if requested_generation < current_generation {
        return Ok(false);
    }
    if requested_generation > current_generation {
        return Err("完成项播放代次超前于当前播放状态");
    }
    if requested_source_media_index != current_source_media_index {
        return Err("完成项播放池序号与当前播放状态不一致");
    }
    if requested_loop_index < current_loop_index {
        return Ok(false);
    }
    if requested_loop_index > current_loop_index {
        return Err("完成项循环序号超前于当前播放状态");
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

#[derive(Debug, Clone, Deserialize)]
pub struct CompletePlaybackItemRequestDto {
    pub playback_generation: u64,
    pub loop_index: u64,
    pub source_media_index: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletePlaybackItemResultDto {
    pub snapshot: PlaybackSnapshotDto,
    pub source_changed: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProbeLocalVideosRequestDto {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReplacePlaybackPoolItemRequestDto {
    pub source_path: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReorderPlaybackPoolItemsRequestDto {
    pub source_paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemovePlaybackPoolItemRequestDto {
    pub source_path: String,
}

fn validate_source_media_pool_count(item_count: usize) -> Result<(), CommandErrorDto> {
    if (1..=MAX_SOURCE_MEDIA_POOL_ITEMS).contains(&item_count) {
        Ok(())
    } else {
        Err(CommandErrorDto::new(
            "invalid_source_media_pool_count",
            format!(
                "播放池必须包含 1..={MAX_SOURCE_MEDIA_POOL_ITEMS} 个媒体，实际收到 {item_count} 个"
            ),
        ))
    }
}

fn validate_source_media_paths(paths: &[String]) -> Result<(), CommandErrorDto> {
    for (index, path) in paths.iter().enumerate() {
        if path.trim().is_empty() {
            return Err(CommandErrorDto::new(
                "empty_source_media_path",
                format!("播放池第 {} 个媒体路径不能为空", index + 1),
            ));
        }
        if path.len() > MAX_SOURCE_MEDIA_PATH_BYTES {
            return Err(CommandErrorDto::new(
                "source_media_path_too_long",
                format!(
                    "播放池第 {} 个媒体路径超过 {MAX_SOURCE_MEDIA_PATH_BYTES} 字节上限",
                    index + 1
                ),
            ));
        }
    }
    Ok(())
}

fn validate_audio_candidate_source_window(
    source_duration_ms: Option<u64>,
    audio_start_ms: Option<u64>,
    audio_end_ms: Option<u64>,
    source_start_ms: u64,
    output_duration_ms: u64,
    loop_source: bool,
) -> Result<(), CommandErrorDto> {
    let (Some(source_duration_ms), Some(audio_start_ms), Some(audio_end_ms)) =
        (source_duration_ms, audio_start_ms, audio_end_ms)
    else {
        return Ok(());
    };
    // 缺失或不一致的旧元数据继续交给既有请求/产物校验，避免把兼容输入误判为跳过。
    if source_duration_ms == 0
        || source_start_ms >= source_duration_ms
        || output_duration_ms == 0
        || audio_start_ms >= audio_end_ms
        || audio_end_ms > source_duration_ms
    {
        return Ok(());
    }

    let overlaps = |candidate_start_ms: u128, candidate_end_ms: u128| {
        candidate_start_ms < u128::from(audio_end_ms)
            && u128::from(audio_start_ms) < candidate_end_ms
    };
    let source_duration_ms = u128::from(source_duration_ms);
    let source_start_ms = u128::from(source_start_ms);
    let output_duration_ms = u128::from(output_duration_ms);
    let intersects = if !loop_source {
        overlaps(
            source_start_ms,
            source_start_ms
                .saturating_add(output_duration_ms)
                .min(source_duration_ms),
        )
    } else if output_duration_ms >= source_duration_ms {
        true
    } else {
        let candidate_end_ms = source_start_ms.saturating_add(output_duration_ms);
        overlaps(source_start_ms, candidate_end_ms.min(source_duration_ms))
            || (candidate_end_ms > source_duration_ms
                && overlaps(0, candidate_end_ms - source_duration_ms))
    };

    if intersects {
        Ok(())
    } else {
        Err(CommandErrorDto::new(
            AUDIO_CANDIDATE_OUTSIDE_SOURCE_AUDIO_WINDOW_CODE,
            "声音候选窗口未覆盖源媒体的有效音轨，跳过本轮",
        ))
    }
}

fn source_media_index_by_path(
    source_media_pool: &[SourceMediaDto],
    source_path: &str,
) -> Result<usize, CommandErrorDto> {
    validate_source_media_paths(&[source_path.to_owned()])?;
    source_media_pool
        .iter()
        .position(|source| source.source_path == source_path)
        .ok_or_else(|| {
            CommandErrorDto::new(
                "source_media_not_found",
                "播放池中不存在指定源媒体，列表可能已经变化",
            )
        })
}

fn reorder_source_media_pool(
    source_media_pool: &[SourceMediaDto],
    source_paths: &[String],
) -> Result<Vec<SourceMediaDto>, CommandErrorDto> {
    if source_paths.len() != source_media_pool.len() {
        return Err(CommandErrorDto::new(
            "invalid_source_media_pool_order",
            "重排必须提交当前播放池全部条目的稳定路径",
        ));
    }
    validate_source_media_paths(source_paths)?;
    let mut unique_paths = HashSet::with_capacity(source_paths.len());
    let mut reordered = Vec::with_capacity(source_paths.len());
    for source_path in source_paths {
        insert_canonical_source_path(&mut unique_paths, source_path)?;
        let index = source_media_index_by_path(source_media_pool, source_path)?;
        reordered.push(source_media_pool[index].clone());
    }
    Ok(reordered)
}

fn apply_source_media_order(
    playback: &mut PlaybackCore,
    source_paths: &[String],
) -> Result<(), CommandErrorDto> {
    let snapshot = playback.snapshot();
    let edited = reorder_source_media_pool(&snapshot.source_media_pool, source_paths)?;
    if edited == snapshot.source_media_pool {
        return Ok(());
    }
    playback
        .set_source_pool(edited)
        .map_err(command_error_from_playback)
}

fn resolve_user_ambient_sound(
    app: &AppHandle,
    ffmpeg_path: &Path,
    user_path: Option<&str>,
    required: bool,
) -> Result<Option<ResolvedAmbientSound>, CommandErrorDto> {
    if !required {
        return Ok(None);
    }
    let user_path = user_path.map(str::trim).filter(|path| !path.is_empty());
    if user_path.is_some_and(|path| path.len() > MAX_SOURCE_MEDIA_PATH_BYTES) {
        return Err(CommandErrorDto::new(
            "ambient_sound_path_too_long",
            "环境声素材路径超过允许长度",
        ));
    }
    let resource_dir = if user_path.is_none() {
        app.path().resource_dir().map_err(|error| {
            CommandErrorDto::new(
                "ambient_sound_resource_dir_failed",
                format!("读取内置环境声资源目录失败：{error}，保持原声"),
            )
        })?
    } else {
        PathBuf::new()
    };
    let resolved =
        resolve_ambient_sound(&resource_dir, user_path.map(Path::new), true).map_err(|error| {
            CommandErrorDto::new("ambient_sound_unavailable", format!("{error}，保持原声"))
        })?;
    if let Some(selection) = resolved.as_ref() {
        validate_audio_input_decodable(ffmpeg_path, &selection.path, 5_000).map_err(|_| {
            CommandErrorDto::new(
                "ambient_sound_decode_failed",
                "环境声素材无法解码，保持原声",
            )
        })?;
    }
    Ok(resolved)
}

fn audio_mix_requires_ambient_sound(
    audio: &AudioEffectParams,
    variants: &[AudioEffectParams],
) -> bool {
    audio.ambient_sound_mix_percent > f64::EPSILON
        || variants
            .iter()
            .any(|variant| variant.ambient_sound_mix_percent > f64::EPSILON)
}

fn resolve_active_ambient_sound(
    app: &AppHandle,
    ffmpeg_path: &Path,
    current: Option<&ResolvedAmbientSound>,
    required: bool,
) -> Result<Option<ResolvedAmbientSound>, CommandErrorDto> {
    if !required {
        return Ok(None);
    }
    if let Some(current) = current {
        let selection = revalidate_ambient_sound(current).map_err(|error| {
            CommandErrorDto::new("ambient_sound_unavailable", format!("{error}，保持原声"))
        })?;
        validate_audio_input_decodable(ffmpeg_path, &selection.path, 5_000).map_err(|_| {
            CommandErrorDto::new(
                "ambient_sound_decode_failed",
                "环境声素材无法解码，保持原声",
            )
        })?;
        return Ok(Some(selection));
    }
    resolve_user_ambient_sound(app, ffmpeg_path, None, required)
}

fn insert_canonical_source_path(
    canonical_paths: &mut HashSet<String>,
    canonical_path: &str,
) -> Result<(), CommandErrorDto> {
    let duplicate_key = if cfg!(windows) {
        canonical_path.to_ascii_lowercase()
    } else {
        canonical_path.to_owned()
    };
    if canonical_paths.insert(duplicate_key) {
        Ok(())
    } else {
        Err(CommandErrorDto::new(
            "duplicate_source_media_path",
            format!("播放池包含重复媒体：{canonical_path}"),
        ))
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
            shutting_down: Arc::new(AtomicBool::new(false)),
            playback: Arc::new(Mutex::new(PlaybackCore::default())),
            playback_transition: Arc::new(Mutex::new(())),
            playback_position_observed_at: Arc::new(Mutex::new(None)),
            speech_worker: Arc::new(Mutex::new(None)),
            video_media_worker: Arc::new(Mutex::new(None)),
            audio_media_worker: Arc::new(Mutex::new(None)),
            media_import_cancellation: Arc::new(Mutex::new(None)),
            media_import_generation: Arc::new(AtomicU64::new(0)),
            runtime_resource_task: Arc::new(RuntimeResourceTask::default()),
            audio_cycle_output: Arc::new(Mutex::new(None)),
            audio_mixer: Arc::new(Mutex::new(None)),
            interlude_mixer: Arc::new(Mutex::new(None)),
            interlude_prepare_lock: Arc::new(Mutex::new(())),
            interlude_prepare_token: Arc::new(AtomicU64::new(0)),
            interlude_session_generation: Arc::new(AtomicU64::new(0)),
            webview_interlude_cache_lock: Arc::new(Mutex::new(())),
            protected_webview_interlude_paths: Arc::new(Mutex::new(HashSet::new())),
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
            ambient_sound: Arc::new(Mutex::new(None)),
            realtime_video_runtime: Arc::new(Mutex::new(RealtimeVideoRuntime::default())),
        }
    }
}

impl AppState {
    fn stop_realtime_video_runtime(&self) -> Result<(), CommandErrorDto> {
        self.realtime_video_runtime
            .lock()
            .map_err(|_| {
                CommandErrorDto::new("realtime_video_lock_failed", "实时画面运行时锁已损坏")
            })?
            .stop();
        Ok(())
    }

    fn begin_media_import(&self) -> Result<MediaImportLease, CommandErrorDto> {
        self.ensure_running()?;
        let mut active = self.media_import_cancellation.lock().map_err(|_| {
            CommandErrorDto::new("media_import_lock_failed", "媒体导入取消状态锁已损坏")
        })?;
        let cancellation = CancellationToken::new();
        if let Some(previous) = active.replace(cancellation.clone()) {
            previous.cancel();
        }
        let generation = self
            .media_import_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        Ok(MediaImportLease {
            generation,
            cancellation,
        })
    }

    fn cancel_media_import(&self) -> Result<(), CommandErrorDto> {
        let active = self.media_import_cancellation.lock().map_err(|_| {
            CommandErrorDto::new("media_import_lock_failed", "媒体导入取消状态锁已损坏")
        })?;
        if let Some(cancellation) = active.as_ref() {
            cancellation.cancel();
        }
        Ok(())
    }

    fn ambient_sound_selection(&self) -> Result<Option<ResolvedAmbientSound>, CommandErrorDto> {
        self.ambient_sound
            .lock()
            .map(|path| path.clone())
            .map_err(|_| {
                CommandErrorDto::new("ambient_sound_path_lock_failed", "环境声素材状态锁已损坏")
            })
    }

    fn protect_webview_interlude_path(&self, path: PathBuf) -> Result<(), CommandErrorDto> {
        self.protected_webview_interlude_paths
            .lock()
            .map_err(|_| {
                CommandErrorDto::new(
                    "webview_interlude_protection_lock_failed",
                    "WebView 插话缓存保护状态锁已损坏",
                )
            })?
            .insert(path);
        Ok(())
    }

    fn release_webview_interlude_path(
        &self,
        path: &str,
    ) -> Result<ReleaseWebViewInterludeCacheResultDto, CommandErrorDto> {
        let _cache_guard = self.webview_interlude_cache_lock.lock().map_err(|_| {
            CommandErrorDto::new(
                "webview_interlude_cache_lock_failed",
                "WebView 插话缓存锁已损坏",
            )
        })?;
        let requested = Path::new(path.trim());
        let mut paths = self.protected_webview_interlude_paths.lock().map_err(|_| {
            CommandErrorDto::new(
                "webview_interlude_protection_lock_failed",
                "WebView 插话缓存保护状态锁已损坏",
            )
        })?;
        if !paths.contains(requested) {
            return Ok(ReleaseWebViewInterludeCacheResultDto {
                released: false,
                removed: false,
            });
        }
        let removed = match std::fs::remove_file(requested) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(CommandErrorDto::new(
                    "webview_interlude_cache_remove_failed",
                    format!("删除 WebView 插话缓存失败：{error}"),
                ));
            }
        };
        paths.remove(requested);
        Ok(ReleaseWebViewInterludeCacheResultDto {
            released: true,
            removed,
        })
    }

    fn protected_webview_interlude_paths(&self) -> Result<Vec<PathBuf>, CommandErrorDto> {
        self.protected_webview_interlude_paths
            .lock()
            .map_err(|_| {
                CommandErrorDto::new(
                    "webview_interlude_protection_lock_failed",
                    "WebView 插话缓存保护状态锁已损坏",
                )
            })
            .map(|paths| paths.iter().cloned().collect())
    }

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
        self.ensure_running()?;
        let _switch_guard = self.audio_mixer_switch_lock.lock().map_err(|_| {
            CommandErrorDto::new("audio_mixer_switch_lock_failed", "音频切换锁已损坏")
        })?;
        self.ensure_running()?;
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
        self.ensure_running()?;
        let _switch_guard = self.audio_mixer_switch_lock.lock().map_err(|_| {
            CommandErrorDto::new("audio_mixer_switch_lock_failed", "音频切换锁已损坏")
        })?;
        self.ensure_running()?;
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
        self.ensure_running()?;
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

    pub fn shutdown_all(&self, budget: Duration) -> Result<RuntimeResourceTaskShutdown, String> {
        self.shutting_down.store(true, Ordering::Release);
        let started_at = Instant::now();
        let mut first_error = None;

        match self.realtime_video_runtime.lock() {
            Ok(mut runtime) => runtime.stop(),
            Err(_) => {
                first_error.get_or_insert("实时画面运行时锁已损坏".to_owned());
            }
        }

        match self.protected_webview_interlude_paths.lock() {
            Ok(mut paths) => paths.clear(),
            Err(_) => {
                first_error.get_or_insert("WebView 插话缓存保护状态锁已损坏".to_owned());
            }
        }

        for result in [
            cancel_background_worker(&self.speech_worker, "话术 Worker 状态锁已损坏"),
            cancel_background_worker(&self.video_media_worker, "视频 Worker 状态锁已损坏"),
            cancel_background_worker(&self.audio_media_worker, "声音 Worker 状态锁已损坏"),
            self.cancel_media_import().map_err(|error| error.message),
            self.runtime_resource_task.begin_shutdown(),
        ] {
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
        }
        if let Err(error) = self.stop_audio_for_shutdown() {
            first_error.get_or_insert(error.message);
        }
        if let Ok(playback) = self.playback.lock() {
            cleanup_compatibility_files(
                playback
                    .snapshot()
                    .source_media_pool
                    .into_iter()
                    .filter(|source| source.playback_reference != source.source_path)
                    .map(|source| PathBuf::from(source.playback_reference)),
            );
        }

        let mut joined_any = false;
        let mut timed_out = false;
        for result in [
            join_background_worker_until(
                &self.speech_worker,
                "话术 Worker 状态锁已损坏",
                started_at,
                budget,
            ),
            join_background_worker_until(
                &self.video_media_worker,
                "视频 Worker 状态锁已损坏",
                started_at,
                budget,
            ),
            join_background_worker_until(
                &self.audio_media_worker,
                "声音 Worker 状态锁已损坏",
                started_at,
                budget,
            ),
        ] {
            match result {
                Ok(RuntimeResourceTaskShutdown::Joined) => joined_any = true,
                Ok(RuntimeResourceTaskShutdown::TimedOut) => timed_out = true,
                Ok(RuntimeResourceTaskShutdown::Idle) => {}
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }

        match self
            .runtime_resource_task
            .shutdown(budget.saturating_sub(started_at.elapsed()))
        {
            Ok(RuntimeResourceTaskShutdown::Joined) => joined_any = true,
            Ok(RuntimeResourceTaskShutdown::TimedOut) => timed_out = true,
            Ok(RuntimeResourceTaskShutdown::Idle) => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }

        if let Some(error) = first_error {
            Err(error)
        } else if timed_out {
            Ok(RuntimeResourceTaskShutdown::TimedOut)
        } else if joined_any {
            Ok(RuntimeResourceTaskShutdown::Joined)
        } else {
            Ok(RuntimeResourceTaskShutdown::Idle)
        }
    }

    fn audio_output_control(&self) -> Result<AudioCycleOutputControl, CommandErrorDto> {
        self.audio_output_control_if_started()?.ok_or_else(|| {
            CommandErrorDto::new("audio_cycle_output_missing", "音频周期输出线程尚未启动")
        })
    }

    fn audio_output_control_if_started(
        &self,
    ) -> Result<Option<AudioCycleOutputControl>, CommandErrorDto> {
        let control = self
            .audio_cycle_output
            .lock()
            .map_err(|_| {
                CommandErrorDto::new("audio_cycle_output_lock_failed", "音频周期输出状态锁已损坏")
            })?
            .as_ref()
            .map(AudioCycleOutputTask::control);
        Ok(control)
    }

    fn audio_output_control_for_resume(
        &self,
    ) -> Result<Option<AudioCycleOutputControl>, CommandErrorDto> {
        let preferred = *self.audio_output_preferred.lock().map_err(|_| {
            CommandErrorDto::new("audio_output_lock_failed", "音频出口状态锁已损坏")
        })?;
        if !preferred {
            return Ok(None);
        }
        self.audio_output_control_if_started()
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

    fn take_interlude_mixer_task(&self) -> Result<Option<ActiveInterludeMixer>, CommandErrorDto> {
        self.interlude_mixer
            .lock()
            .map_err(|_| {
                CommandErrorDto::new(
                    "interlude_mixer_lock_failed",
                    "PortAudio 插话混音状态锁已损坏",
                )
            })
            .map(|mut task| task.take())
    }

    fn stop_interlude_mixer_current(&self) -> Result<(), CommandErrorDto> {
        let stop_result = self.audio_output_control().ok().map(|control| {
            control
                .stop_interlude()
                .map_err(|error| CommandErrorDto::new("interlude_output_stop_failed", error))
        });
        if let Some(mut active) = self.take_interlude_mixer_task()? {
            active.task.stop_preserving_output();
        }
        if let Some(result) = stop_result {
            result?;
        }
        Ok(())
    }

    fn stop_interlude_mixer(&self) -> Result<(), CommandErrorDto> {
        self.interlude_prepare_token.fetch_add(1, Ordering::AcqRel);
        self.interlude_session_generation
            .fetch_add(1, Ordering::AcqRel);
        self.stop_interlude_mixer_current()
    }

    fn stop_audio_cycle_output(&self) -> Result<(), CommandErrorDto> {
        self.stop_interlude_mixer()?;
        if let Some(task) = self.take_audio_cycle_output_task()? {
            task.shutdown()
                .map_err(|error| CommandErrorDto::new("audio_cycle_output_stop_failed", error))?;
        }
        Ok(())
    }

    fn stop_audio_for_shutdown(&self) -> Result<(), CommandErrorDto> {
        self.interlude_prepare_token.fetch_add(1, Ordering::AcqRel);
        let _interlude_guard = self.interlude_prepare_lock.lock().map_err(|_| {
            CommandErrorDto::new(
                "interlude_prepare_lock_failed",
                "PortAudio 插话创建锁已损坏",
            )
        })?;
        self.stop_audio_mixer()?;
        self.stop_audio_cycle_output()
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
        self.interlude_prepare_token.fetch_add(1, Ordering::AcqRel);
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
        self.interlude_prepare_token.fetch_add(1, Ordering::AcqRel);
        self.interlude_session_generation
            .fetch_add(1, Ordering::AcqRel);
        if let Ok(control) = self.audio_output_control() {
            control
                .clear()
                .map_err(|error| CommandErrorDto::new("audio_output_clear_failed", error))?;
        }
        if let Some(mut active) = self.take_interlude_mixer_task()? {
            active.task.stop_preserving_output();
        }
        self.stop_audio_mixer()
    }

    fn resume_audio_output(&self, app: &AppHandle) -> Result<(), CommandErrorDto> {
        // 首播时最终效果窗口尚未建立真实时钟，先保持 WebView 原声；后续源同步再接管。
        let Some(control) = self.audio_output_control_for_resume()? else {
            return Ok(());
        };
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
        self.ensure_running()?;
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
        let ambient_sound_required = {
            let playback = self
                .playback
                .lock()
                .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
            let snapshot = playback.snapshot();
            let (audio, variants) = playback.audio_stream_configuration();
            snapshot.audio_processing_enabled
                && snapshot.current_audio_source.as_deref() != Some("realtime_variant")
                && audio_mix_requires_ambient_sound(&audio, &variants)
        };
        let target_root = runtime_resource_target_root(app)
            .map_err(|error| CommandErrorDto::new("media_resource_dir_failed", error))?;
        let (ffmpeg_path, _) = configured_media_engine_paths_with_resource_dir(&target_root)
            .map_err(|error| CommandErrorDto::new("media_engine_unavailable", error.to_string()))?;
        let current_ambient_sound = self.ambient_sound_selection()?;
        let ambient_sound = resolve_active_ambient_sound(
            app,
            &ffmpeg_path,
            current_ambient_sound.as_ref(),
            ambient_sound_required,
        )?;
        let ambient_sound_path = ambient_sound.map(|selection| selection.path);
        let (
            source_path,
            start_clock,
            has_audio,
            filter_graph,
            quality_pitch,
            pcm_effects,
            required_ambient_sound_path,
            source_identity,
            playback_rate,
        ) = {
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
                if stream_processed_audio || source.playback_reference != source.source_path {
                    Some(PathBuf::from(source.source_path.clone()))
                } else {
                    snapshot
                        .current_video_reference
                        .clone()
                        .or_else(|| Some(source.source_path.clone()))
                        .map(PathBuf::from)
                }
            });
            let (
                filter_graph,
                quality_pitch,
                pcm_effects,
                required_ambient_sound_path,
                playback_rate,
            ) = if stream_processed_audio {
                let (audio, variants) = playback.audio_stream_configuration();
                let playback_rate = audio.playback_speed;
                let plan = build_audio_stream_filter_graph_with_ambient(
                    &audio,
                    &variants,
                    source.audio_sample_rate_hz,
                    sample_rate_hz,
                    ambient_sound_path.is_some(),
                )
                .map_err(|error| {
                    CommandErrorDto::new("audio_stream_filter_invalid", error.to_string())
                })?;
                let required_ambient_sound_path = plan
                    .requires_ambient_input
                    .then(|| ambient_sound_path.clone())
                    .flatten();
                (
                    Some(plan.filter_graph),
                    plan.quality_pitch,
                    plan.pcm_effects,
                    required_ambient_sound_path,
                    playback_rate,
                )
            } else {
                (None, None, None, None, 1.0)
            };
            (
                source_path,
                start_clock,
                source.audio_sample_rate_hz.is_some() || has_realtime_audio,
                filter_graph,
                quality_pitch,
                pcm_effects,
                required_ambient_sound_path,
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
        let task = AudioMixerTask::start_candidate_with_filter_and_variant_count(
            ffmpeg_path,
            source_path,
            sample_rate_hz,
            seek_position_ms,
            timeline_start_position_ms,
            filter_graph,
            quality_pitch,
            pcm_effects,
            required_ambient_sound_path,
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

        let (snapshot, playback_rate) = {
            let playback = self
                .playback
                .lock()
                .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
            (
                playback.snapshot(),
                playback.audio_stream_configuration().0.playback_speed,
            )
        };
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
        let observed_elapsed_ms = self
            .playback_position_observed_at
            .lock()
            .map_err(|_| {
                CommandErrorDto::new(
                    "playback_position_clock_lock_failed",
                    "播放位置时钟锁已损坏",
                )
            })?
            .map(|observed_at| observed_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0);
        if let Some(code) = audio_cycle_target_validation_code(
            current_absolute_position_ms,
            observed_elapsed_ms,
            playback_rate,
            request.target_absolute_position_ms,
        ) {
            let message = if code == "audio_candidate_stale" {
                "候选目标绝对媒体时间已经过期，保持当前音轨并等待最新计划".to_owned()
            } else {
                format!(
                    "候选目标必须位于当前绝对媒体时间之后 {}ms 内",
                    AUDIO_CYCLE_TARGET_HORIZON_MS
                )
            };
            return Err(CommandErrorDto::new(code, message));
        }
        let source = snapshot
            .source_media
            .as_ref()
            .ok_or_else(|| CommandErrorDto::new("source_media_required", "请先导入一个源媒体"))?;
        if source.audio_sample_rate_hz.is_none() {
            return Err(CommandErrorDto::new(
                "audio_candidate_source_missing",
                "源媒体没有可供周期处理的音频流",
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
        let target_root = runtime_resource_target_root(app)
            .map_err(|error| CommandErrorDto::new("media_resource_dir_failed", error))?;
        let (ffmpeg_path, _) = configured_media_engine_paths_with_resource_dir(&target_root)
            .map_err(|error| CommandErrorDto::new("media_engine_unavailable", error.to_string()))?;
        let current_ambient_sound = self.ambient_sound_selection()?;
        let ambient_sound = resolve_active_ambient_sound(
            app,
            &ffmpeg_path,
            current_ambient_sound.as_ref(),
            audio_mix_requires_ambient_sound(&request.audio, &request.audio_variants),
        )?;
        let ambient_sound_path = ambient_sound.map(|selection| selection.path);
        let filter_plan = build_audio_stream_filter_graph_with_ambient(
            &request.audio,
            &request.audio_variants,
            source.audio_sample_rate_hz,
            sample_rate_hz,
            ambient_sound_path.is_some(),
        )
        .map_err(|error| {
            CommandErrorDto::new("audio_candidate_params_invalid", error.to_string())
        })?;
        let candidate_start_position_ms = request
            .target_absolute_position_ms
            .saturating_sub(AUDIO_CYCLE_CANDIDATE_PRE_ROLL_MS);
        let seek_position_ms = candidate_start_position_ms % duration_ms;
        let preparation_started_at = Instant::now();
        // 周期候选必须追上从 prepare 到 target 之间已经经过的墙钟时间；只用固定
        // 750ms readiness 会在多支路/特征滤镜冷启动后刚出首批 PCM 时过早切轨。
        let task = AudioMixerTask::start_short_cycle_candidate_with_filter_and_variant_count(
            ffmpeg_path,
            PathBuf::from(&source.source_path),
            sample_rate_hz,
            seek_position_ms,
            candidate_start_position_ms,
            Some(filter_plan.filter_graph),
            filter_plan.quality_pitch,
            filter_plan.pcm_effects,
            filter_plan
                .requires_ambient_input
                .then_some(ambient_sound_path)
                .flatten(),
            request.audio.playback_speed,
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
            return Err(CommandErrorDto::new(
                audio_crossfade_error_code(&error),
                error,
            ));
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

    fn stop_media_worker_slot(
        &self,
        worker: &Mutex<Option<BackgroundWorkerTask>>,
        lock_code: &str,
        lock_message: &str,
        worker_label: &str,
        budget: Duration,
    ) -> Result<bool, CommandErrorDto> {
        {
            let worker = worker
                .lock()
                .map_err(|_| CommandErrorDto::new(lock_code, lock_message))?;
            let Some(task) = worker.as_ref() else {
                return Ok(false);
            };
            task.cancellation.cancel();
        }
        match join_background_worker_until(worker, lock_message, Instant::now(), budget) {
            Ok(RuntimeResourceTaskShutdown::Joined | RuntimeResourceTaskShutdown::Idle) => Ok(true),
            Ok(RuntimeResourceTaskShutdown::TimedOut) => Err(CommandErrorDto::new(
                "media_worker_stop_timeout",
                format!(
                    "{worker_label}未在 {} 毫秒停止预算内退出",
                    budget.as_millis()
                ),
            )),
            Err(error) => Err(CommandErrorDto::new("media_worker_stop_failed", error)),
        }
    }

    fn stop_video_media_worker(&self) -> Result<(), CommandErrorDto> {
        if self.stop_media_worker_slot(
            &self.video_media_worker,
            "video_media_worker_lock_failed",
            "视频 Worker 状态锁已损坏",
            "视频 Worker",
            MEDIA_WORKER_STOP_BUDGET,
        )? {
            if let Ok(mut playback) = self.playback.lock() {
                if playback.snapshot().video_processing_status == "processing" {
                    playback.mark_media_processing_failed("视频处理已取消，等待应用最新参数");
                }
            }
        }
        Ok(())
    }

    fn stop_audio_media_worker(&self) -> Result<(), CommandErrorDto> {
        if self.stop_media_worker_slot(
            &self.audio_media_worker,
            "audio_media_worker_lock_failed",
            "声音 Worker 状态锁已损坏",
            "声音 Worker",
            MEDIA_WORKER_STOP_BUDGET,
        )? {
            if let Ok(mut playback) = self.playback.lock() {
                if playback.snapshot().audio_processing_status == "processing" {
                    playback
                        .mark_audio_media_processing_failed("声音候选处理已取消，等待应用最新参数");
                }
            }
        }
        Ok(())
    }

    fn stop_media_workers(&self) -> Result<(), CommandErrorDto> {
        let mut first_error = None;
        for result in [
            self.stop_video_media_worker(),
            self.stop_audio_media_worker(),
        ] {
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn media_worker_slot_is_running(
        &self,
        worker: &Mutex<Option<BackgroundWorkerTask>>,
        lock_code: &str,
        lock_message: &str,
    ) -> Result<bool, CommandErrorDto> {
        let worker = worker
            .lock()
            .map_err(|_| CommandErrorDto::new(lock_code, lock_message))?;
        Ok(worker.is_some())
    }

    fn video_media_worker_is_running(&self) -> Result<bool, CommandErrorDto> {
        self.media_worker_slot_is_running(
            &self.video_media_worker,
            "video_media_worker_lock_failed",
            "视频 Worker 状态锁已损坏",
        )
    }

    fn audio_media_worker_is_running(&self) -> Result<bool, CommandErrorDto> {
        self.media_worker_slot_is_running(
            &self.audio_media_worker,
            "audio_media_worker_lock_failed",
            "声音 Worker 状态锁已损坏",
        )
    }

    fn reap_finished_media_worker_slot(
        &self,
        worker: &Mutex<Option<BackgroundWorkerTask>>,
        lock_code: &str,
        lock_message: &str,
    ) -> Result<bool, CommandErrorDto> {
        let task = {
            let mut worker = worker
                .lock()
                .map_err(|_| CommandErrorDto::new(lock_code, lock_message))?;
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

    fn reap_finished_video_media_worker(&self) -> Result<bool, CommandErrorDto> {
        self.reap_finished_media_worker_slot(
            &self.video_media_worker,
            "video_media_worker_lock_failed",
            "视频 Worker 状态锁已损坏",
        )
    }

    fn reap_finished_audio_media_worker(&self) -> Result<bool, CommandErrorDto> {
        self.reap_finished_media_worker_slot(
            &self.audio_media_worker,
            "audio_media_worker_lock_failed",
            "声音 Worker 状态锁已损坏",
        )
    }

    fn install_speech_worker(
        &self,
        task: BackgroundWorkerTask,
    ) -> Result<(), BackgroundWorkerTask> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(task);
        }
        let mut worker = match self.speech_worker.lock() {
            Ok(worker) => worker,
            Err(_) => return Err(task),
        };
        if self.shutting_down.load(Ordering::Acquire) || worker.is_some() {
            return Err(task);
        }
        worker.replace(task);
        Ok(())
    }

    fn install_media_worker_slot(
        &self,
        worker: &Mutex<Option<BackgroundWorkerTask>>,
        task: BackgroundWorkerTask,
    ) -> Result<(), BackgroundWorkerTask> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(task);
        }
        let mut worker = match worker.lock() {
            Ok(worker) => worker,
            Err(_) => return Err(task),
        };
        if self.shutting_down.load(Ordering::Acquire) || worker.is_some() {
            return Err(task);
        }
        worker.replace(task);
        Ok(())
    }

    fn install_video_media_worker(
        &self,
        task: BackgroundWorkerTask,
    ) -> Result<(), BackgroundWorkerTask> {
        self.install_media_worker_slot(&self.video_media_worker, task)
    }

    fn install_audio_media_worker(
        &self,
        task: BackgroundWorkerTask,
    ) -> Result<(), BackgroundWorkerTask> {
        self.install_media_worker_slot(&self.audio_media_worker, task)
    }

    fn ensure_main_window(&self, window: &Window) -> Result<(), CommandErrorDto> {
        self.ensure_running()?;
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
        self.ensure_running()?;
        if matches!(window.label(), "main" | "final-effect") {
            Ok(())
        } else {
            Err(CommandErrorDto::new(
                "playback_window_only",
                "当前命令只允许主窗口或最终效果窗口调用",
            ))
        }
    }

    fn ensure_running(&self) -> Result<(), CommandErrorDto> {
        if self.shutting_down.load(Ordering::Acquire) {
            Err(CommandErrorDto::new(
                "app_shutting_down",
                "应用正在退出，不再接受新的后台任务",
            ))
        } else {
            Ok(())
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
        self.snapshot_from_core(playback.snapshot())
    }

    fn snapshot_from_core(&self, snapshot: PlaybackSnapshot) -> PlaybackSnapshotDto {
        PlaybackSnapshotDto::from(snapshot)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlaybackSnapshotDto {
    pub window_id: Option<String>,
    pub playback_generation: u64,
    pub playback_state: String,
    pub source_media: Option<SourceMediaDto>,
    pub source_media_pool: Vec<SourceMediaDto>,
    pub source_media_index: usize,
    pub playback_pool_cycle: u64,
    pub loop_index: u64,
    pub current_position_ms: u64,
    pub current_video_source: Option<String>,
    pub current_video_reference: Option<String>,
    pub current_video_sha256: Option<String>,
    pub current_media_plan_id: Option<String>,
    pub current_media_sequence: Option<u64>,
    pub current_media_playback_generation: Option<u64>,
    pub current_media_source_revision: Option<u64>,
    pub current_media_target_absolute_position_ms: Option<u64>,
    pub current_media_source_start_ms: Option<u64>,
    pub current_media_output_duration_ms: Option<u64>,
    pub current_media_valid_until_absolute_position_ms: Option<u64>,
    pub pending_video_reference: Option<String>,
    pub pending_video_sha256: Option<String>,
    pub pending_media_plan_id: Option<String>,
    pub pending_media_sequence: Option<u64>,
    pub pending_media_playback_generation: Option<u64>,
    pub pending_media_source_revision: Option<u64>,
    pub pending_media_target_absolute_position_ms: Option<u64>,
    pub pending_media_source_start_ms: Option<u64>,
    pub pending_media_output_duration_ms: Option<u64>,
    pub pending_media_valid_until_absolute_position_ms: Option<u64>,
    pub video_processing_enabled: bool,
    pub video_processing_status: String,
    pub video_processing_progress_percent: u8,
    pub audio_processing_enabled: bool,
    pub audio_processing_progress_percent: u8,
    pub current_audio_artifact_reference: Option<String>,
    pub current_audio_artifact_sha256: Option<String>,
    pub current_audio_media_plan_id: Option<String>,
    pub current_audio_media_sequence: Option<u64>,
    pub current_audio_media_playback_generation: Option<u64>,
    pub current_audio_media_source_revision: Option<u64>,
    pub current_audio_media_target_absolute_position_ms: Option<u64>,
    pub current_audio_media_source_start_ms: Option<u64>,
    pub current_audio_media_output_duration_ms: Option<u64>,
    pub current_audio_media_valid_until_absolute_position_ms: Option<u64>,
    pub pending_audio_artifact_reference: Option<String>,
    pub pending_audio_artifact_sha256: Option<String>,
    pub pending_audio_media_plan_id: Option<String>,
    pub pending_audio_media_sequence: Option<u64>,
    pub pending_audio_media_playback_generation: Option<u64>,
    pub pending_audio_media_source_revision: Option<u64>,
    pub pending_audio_media_target_absolute_position_ms: Option<u64>,
    pub pending_audio_media_source_start_ms: Option<u64>,
    pub pending_audio_media_output_duration_ms: Option<u64>,
    pub pending_audio_media_valid_until_absolute_position_ms: Option<u64>,
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
    pub audio_stream_params: autolive_desktop_core::media_effect_params::AudioEffectParams,
    pub audio_stream_variants: Vec<autolive_desktop_core::media_effect_params::AudioEffectParams>,
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
            source_media_pool: value.source_media_pool,
            source_media_index: value.source_media_index,
            playback_pool_cycle: value.playback_pool_cycle,
            loop_index: value.loop_index,
            current_position_ms: value.current_position_ms,
            current_video_source: value.current_video_source,
            current_video_reference: value.current_video_reference,
            current_video_sha256: value.current_video_sha256,
            current_media_plan_id: value.current_media_plan_id,
            current_media_sequence: value.current_media_sequence,
            current_media_playback_generation: value.current_media_playback_generation,
            current_media_source_revision: value.current_media_source_revision,
            current_media_target_absolute_position_ms: value
                .current_media_target_absolute_position_ms,
            current_media_source_start_ms: value.current_media_source_start_ms,
            current_media_output_duration_ms: value.current_media_output_duration_ms,
            current_media_valid_until_absolute_position_ms: value
                .current_media_valid_until_absolute_position_ms,
            pending_video_reference: value.pending_video_reference,
            pending_video_sha256: value.pending_video_sha256,
            pending_media_plan_id: value.pending_media_plan_id,
            pending_media_sequence: value.pending_media_sequence,
            pending_media_playback_generation: value.pending_media_playback_generation,
            pending_media_source_revision: value.pending_media_source_revision,
            pending_media_target_absolute_position_ms: value
                .pending_media_target_absolute_position_ms,
            pending_media_source_start_ms: value.pending_media_source_start_ms,
            pending_media_output_duration_ms: value.pending_media_output_duration_ms,
            pending_media_valid_until_absolute_position_ms: value
                .pending_media_valid_until_absolute_position_ms,
            video_processing_enabled: value.video_processing_enabled,
            video_processing_status: value.video_processing_status,
            video_processing_progress_percent: value.video_processing_progress_percent,
            audio_processing_enabled: value.audio_processing_enabled,
            audio_processing_progress_percent: value.audio_processing_progress_percent,
            current_audio_artifact_reference: value.current_audio_artifact_reference,
            current_audio_artifact_sha256: value.current_audio_artifact_sha256,
            current_audio_media_plan_id: value.current_audio_media_plan_id,
            current_audio_media_sequence: value.current_audio_media_sequence,
            current_audio_media_playback_generation: value.current_audio_media_playback_generation,
            current_audio_media_source_revision: value.current_audio_media_source_revision,
            current_audio_media_target_absolute_position_ms: value
                .current_audio_media_target_absolute_position_ms,
            current_audio_media_source_start_ms: value.current_audio_media_source_start_ms,
            current_audio_media_output_duration_ms: value.current_audio_media_output_duration_ms,
            current_audio_media_valid_until_absolute_position_ms: value
                .current_audio_media_valid_until_absolute_position_ms,
            pending_audio_artifact_reference: value.pending_audio_artifact_reference,
            pending_audio_artifact_sha256: value.pending_audio_artifact_sha256,
            pending_audio_media_plan_id: value.pending_audio_media_plan_id,
            pending_audio_media_sequence: value.pending_audio_media_sequence,
            pending_audio_media_playback_generation: value.pending_audio_media_playback_generation,
            pending_audio_media_source_revision: value.pending_audio_media_source_revision,
            pending_audio_media_target_absolute_position_ms: value
                .pending_audio_media_target_absolute_position_ms,
            pending_audio_media_source_start_ms: value.pending_audio_media_source_start_ms,
            pending_audio_media_output_duration_ms: value.pending_audio_media_output_duration_ms,
            pending_audio_media_valid_until_absolute_position_ms: value
                .pending_audio_media_valid_until_absolute_position_ms,
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
            audio_stream_params: value.audio_stream_params,
            audio_stream_variants: value.audio_stream_variants,
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
    pub audio_selection_mode: InterludeAudioSelectionMode,
    pub audio_fixed_preset_id: String,
    pub audio_preset_ids: Vec<String>,
    pub audio_mix_enabled: bool,
    pub audio_mix_pick_min: u8,
    pub audio_mix_pick_max: u8,
    pub audio_variation_mode: InterludeAudioVariationMode,
    pub audio_variation_period_min_ms: u64,
    pub audio_variation_period_max_ms: u64,
    pub interval_min_ms: u64,
    pub interval_max_ms: u64,
    pub volume_db: f64,
    pub ducking_depth_db: f64,
    pub ducking_attack_ms: u64,
    pub ducking_release_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartPortAudioInterludeRequestDto {
    pub path: String,
    pub audio: AudioEffectParams,
    #[serde(default)]
    pub audio_variants: Vec<AudioEffectParams>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetPortAudioMediaVolumeRequestDto {
    pub volume: f64,
    pub muted: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetPortAudioInterludeVolumeRequestDto {
    pub volume_db: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartPortAudioInterludeResultDto {
    pub generation: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SwitchPortAudioInterludePresetRequestDto {
    pub generation: u64,
    pub media_position_ms: u64,
    pub audio: AudioEffectParams,
    #[serde(default)]
    pub audio_variants: Vec<AudioEffectParams>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SwitchPortAudioInterludePresetResultDto {
    pub generation: u64,
    pub media_position_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrepareWebViewInterludeResultDto {
    pub state: String,
    pub path: String,
    pub output_size_bytes: Option<u64>,
    pub ambient_sound_source: Option<String>,
    pub reason_code: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseWebViewInterludeCacheRequestDto {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReleaseWebViewInterludeCacheResultDto {
    pub released: bool,
    pub removed: bool,
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
    pub params: MediaEffectParams,
    /// 多虚拟轨音频参数；空则只用 params.audio。
    #[serde(default)]
    pub audio_variants: Option<Vec<autolive_desktop_core::media_effect_params::AudioEffectParams>>,
    /// 仅在环境声混合比例非零时使用；必须来自用户选择的本地音频文件。
    #[serde(default)]
    pub ambient_sound_path: Option<String>,
    /// 本次只更新 video、audio，或同时更新；旧客户端省略时按 both。
    #[serde(default)]
    pub scope: Option<String>,
    /// 自动视频周期要求 83 个 UI 参数项（排除两个翻转）作为一个整体通过准入。
    /// 手动恢复默认和历史客户端省略时保持兼容，不借此伪装成正式全量周期。
    #[serde(default)]
    pub require_atomic_video_admission: bool,
    pub plan_id: String,
    pub sequence: u64,
    pub playback_generation: u64,
    pub source_revision: u64,
    pub source_start_ms: u64,
    pub output_duration_ms: u64,
    pub loop_source: bool,
    pub target_absolute_position_ms: u64,
    pub valid_until_absolute_position_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrepareRealtimeVideoPlanRequestDto {
    pub params: MediaEffectParams,
    pub sequence: u64,
    pub playback_generation: u64,
    pub source_revision: u64,
    pub target_absolute_position_ms: u64,
    pub period_ms: u64,
    pub seed: u64,
    #[serde(default)]
    pub next_sequence: Option<u64>,
    #[serde(default)]
    pub next_target_absolute_position_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitRealtimeVideoPlanRequestDto {
    pub sequence: u64,
    pub playback_generation: u64,
    pub source_revision: u64,
    pub media_pts_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SyncRealtimeVideoRendererRequestDto {
    pub playback_generation: u64,
    pub position_ms: u64,
    pub paused: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitMediaProcessingRequestDto {
    pub plan_id: String,
    pub sequence: u64,
    pub playback_generation: u64,
    pub source_revision: u64,
    pub observed_absolute_position_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscardMediaProcessingCandidateRequestDto {
    pub plan_id: String,
    pub sequence: u64,
    pub playback_generation: u64,
    pub source_revision: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseMediaProcessingArtifactRequestDto {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrepareAudioMediaCandidateRequestDto {
    pub params: AudioEffectParams,
    #[serde(default)]
    pub audio_variants: Vec<AudioEffectParams>,
    #[serde(default)]
    pub ambient_sound_path: Option<String>,
    pub plan_id: String,
    pub sequence: u64,
    pub playback_generation: u64,
    pub source_revision: u64,
    pub source_start_ms: u64,
    pub output_duration_ms: u64,
    pub loop_source: bool,
    pub target_absolute_position_ms: u64,
    pub valid_until_absolute_position_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AudioMediaCandidateIdentityRequestDto {
    pub plan_id: String,
    pub sequence: u64,
    pub playback_generation: u64,
    pub source_revision: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscardAudioMediaCandidateRequestDto {
    pub plan_id: String,
    pub sequence: u64,
    pub playback_generation: u64,
    pub source_revision: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseAudioMediaCandidateRequestDto {
    pub path: String,
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
    let mut remaining_bytes = 0_u64;
    let mut removed_files: u32 = 0;
    let mut removed_bytes: u64 = 0;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            continue;
        }
        let size = metadata.len();
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            remaining_bytes = remaining_bytes.saturating_add(size);
            continue;
        };
        let managed = name.starts_with("processed-") || name.starts_with("ts-compat-");
        let is_partial = managed && name.contains(".partial");
        let is_completed =
            managed && (name.ends_with(".mp4") || name.ends_with(".m4a")) && !is_partial;
        let stale_partial = is_partial && modified <= partial_expiry;
        if protected.contains(&path) || (!is_completed && !stale_partial) {
            remaining_bytes = remaining_bytes.saturating_add(size);
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {
                removed_files = removed_files.saturating_add(1);
                removed_bytes = removed_bytes.saturating_add(size);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
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

fn validate_managed_media_processing_artifact_path(
    cache_dir: &Path,
    path: &Path,
) -> Result<(), CommandErrorDto> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let parent = path.parent().and_then(|value| value.canonicalize().ok());
    let cache_root = cache_dir.canonicalize().ok();
    if parent.is_none()
        || cache_root.is_none()
        || parent.as_ref() != cache_root.as_ref()
        || !(file_name.starts_with("processed-g") || file_name.starts_with("processed-audio-g"))
        || !matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("mp4" | "m4a")
        )
    {
        return Err(CommandErrorDto::new(
            "media_artifact_path_invalid",
            "只允许释放媒体处理缓存目录中的成品文件",
        ));
    }
    Ok(())
}

fn remove_managed_media_processing_artifact(
    cache_dir: &Path,
    path: &Path,
) -> Result<bool, CommandErrorDto> {
    validate_managed_media_processing_artifact_path(cache_dir, path)?;
    const RETRY_DELAYS_MS: [u64; 4] = [25, 50, 100, 200];
    let mut retry_delays = RETRY_DELAYS_MS.into_iter();
    loop {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::PermissionDenied
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Other
                ) =>
            {
                let Some(delay_ms) = retry_delays.next() else {
                    return Err(CommandErrorDto::new(
                        "media_artifact_release_failed",
                        error.to_string(),
                    ));
                };
                std::thread::sleep(Duration::from_millis(delay_ms));
            }
            Err(error) => {
                return Err(CommandErrorDto::new(
                    "media_artifact_release_failed",
                    error.to_string(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod cache_tests {
    use super::{
        remove_managed_media_processing_artifact, remove_media_processing_cache_files_at,
        validate_managed_media_processing_artifact_path,
    };
    use std::fs;
    use std::time::{Duration, SystemTime};

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
        let completed_audio = directory.join("processed-audio-old.m4a");
        let stale_partial = directory.join("processed-old.partial.mp4");
        let fresh_partial = directory.join("processed-fresh.partial.mp4");
        let unrelated = directory.join("other.mp4");
        fs::write(&protected, b"keep").expect("write");
        fs::write(&completed, b"done").expect("write");
        fs::write(&completed_audio, b"audio").expect("write");
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
        assert!(!completed_audio.exists());
        assert_eq!(result.removed_files, 3);
        assert_eq!(result.removed_bytes, 14);
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

    #[test]
    fn managed_media_artifact_deletion_validates_scope_and_treats_missing_as_idempotent() {
        let directory = std::env::temp_dir().join(format!(
            "autolive-media-artifact-delete-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("cache directory should be created");
        let missing = directory.join("processed-g7-s2.mp4");
        let missing_audio = directory.join("processed-audio-g7-s2.m4a");
        assert!(validate_managed_media_processing_artifact_path(&directory, &missing).is_ok());
        assert!(
            validate_managed_media_processing_artifact_path(&directory, &missing_audio).is_ok()
        );
        assert!(
            !remove_managed_media_processing_artifact(&directory, &missing)
                .expect("missing artifact deletion should be idempotent")
        );
        assert!(validate_managed_media_processing_artifact_path(
            &directory,
            &directory.join("unmanaged.mp4")
        )
        .is_err());
        let _ignored = fs::remove_dir_all(directory);
    }
}

fn cleanup_media_processing_cache(
    app: &AppHandle,
    state: &AppState,
) -> Result<CacheCleanupResultDto, CommandErrorDto> {
    let _ = state.reap_finished_video_media_worker()?;
    let _ = state.reap_finished_audio_media_worker()?;
    if state.video_media_worker_is_running()? || state.audio_media_worker_is_running()? {
        return Err(CommandErrorDto::new(
            "media_cache_cleanup_busy",
            "媒体处理仍在进行，请完成后再删除已生成缓存",
        ));
    }
    let app_cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandErrorDto::new("cache_dir_failed", error.to_string()))?;
    let cache_dir = app_cache_dir.join("media-processing");
    let compatibility_cache_dir = app_cache_dir.join("media-compatibility");
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
        let mut protected = [
            snapshot.current_video_reference,
            snapshot.pending_video_reference,
            snapshot.current_audio_artifact_reference,
            snapshot.pending_audio_artifact_reference,
            snapshot.current_audio_reference,
            snapshot.pending_audio_reference,
        ]
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
        protected.extend(
            snapshot
                .source_media_pool
                .into_iter()
                .map(|source| PathBuf::from(source.playback_reference)),
        );
        protected
    };
    let media = remove_media_processing_cache_files(&cache_dir, &protected_paths)
        .map_err(|error| CommandErrorDto::new("media_cache_cleanup_failed", error.to_string()))?;
    let compatibility =
        remove_media_processing_cache_files(&compatibility_cache_dir, &protected_paths).map_err(
            |error| {
                CommandErrorDto::new(
                    "media_compatibility_cache_cleanup_failed",
                    error.to_string(),
                )
            },
        )?;
    let _webview_cache_guard = state
        .webview_interlude_cache_lock
        .try_lock()
        .map_err(|error| match error {
            std::sync::TryLockError::WouldBlock => CommandErrorDto::new(
                "webview_interlude_cache_cleanup_busy",
                "WebView 插话缓存仍在生成，请稍后再清理",
            ),
            std::sync::TryLockError::Poisoned(_) => CommandErrorDto::new(
                "webview_interlude_cache_lock_failed",
                "WebView 插话缓存锁已损坏",
            ),
        })?;
    let protected_webview_paths = state.protected_webview_interlude_paths()?;
    let webview = cleanup_webview_interlude_cache(
        &app.path()
            .app_cache_dir()
            .map_err(|error| CommandErrorDto::new("cache_dir_failed", error.to_string()))?
            .join("webview-interlude"),
        &protected_webview_paths,
    )
    .map_err(|error| {
        CommandErrorDto::new("webview_interlude_cache_cleanup_failed", error.to_string())
    })?;
    Ok(CacheCleanupResultDto {
        removed_files: media
            .removed_files
            .saturating_add(compatibility.removed_files)
            .saturating_add(webview.removed_files),
        removed_bytes: media
            .removed_bytes
            .saturating_add(compatibility.removed_bytes)
            .saturating_add(webview.removed_bytes),
        remaining_bytes: media
            .remaining_bytes
            .saturating_add(compatibility.remaining_bytes)
            .saturating_add(webview.remaining_bytes),
    })
}

pub fn purge_media_processing_cache(app: &AppHandle) -> Result<(), String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?
        .join("media-processing");
    let mut retry_delays = [25_u64, 50, 100, 200].into_iter();
    loop {
        match std::fs::remove_dir_all(&cache_dir) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::PermissionDenied
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Other
                ) =>
            {
                let Some(delay_ms) = retry_delays.next() else {
                    return Err(error.to_string());
                };
                std::thread::sleep(Duration::from_millis(delay_ms));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

pub fn cleanup_stale_generated_caches(app: &AppHandle) -> Result<(), String> {
    let app_cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?;
    match std::fs::remove_dir_all(app_cache_dir.join("media-video-stream")) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    remove_media_processing_cache_files(&app_cache_dir.join("media-compatibility"), &[])
        .map_err(|error| error.to_string())?;
    cleanup_webview_interlude_cache(&app_cache_dir.join("webview-interlude"), &[])
        .map_err(|error| error.to_string())?;
    Ok(())
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
        CommandErrorDto::new("media_probe_failed", format!("媒体导入任务失败：{error}"))
    })?
}

#[tauri::command]
pub async fn probe_local_videos(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: ProbeLocalVideosRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        probe_local_video_pool_blocking(window, app, state, request.paths)
            .map(|(_, snapshot)| snapshot)
    })
    .await
    .map_err(|error| {
        CommandErrorDto::new(
            "media_probe_failed",
            format!("媒体批量导入任务失败：{error}"),
        )
    })?
}

#[tauri::command]
pub async fn append_local_videos(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: ProbeLocalVideosRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let import = state.begin_media_import()?;
        let mut prepared = probe_local_video_paths(&app, request.paths, &import.cancellation)?;
        let sources: Vec<_> = prepared
            .results
            .iter()
            .map(|result| result.source.clone())
            .collect();
        let result = commit_playback_pool_edit(&window, &state, Some(&import), move |playback| {
            playback
                .append_source_pool(sources.clone())
                .map_err(command_error_from_playback)
        });
        if result.is_ok() {
            prepared.retain_created_files();
        }
        result
    })
    .await
    .map_err(|error| {
        CommandErrorDto::new(
            "media_probe_failed",
            format!("媒体追加导入任务失败：{error}"),
        )
    })?
}

#[tauri::command]
pub async fn replace_playback_pool_item(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: ReplacePlaybackPoolItemRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let import = state.begin_media_import()?;
        let mut prepared = probe_local_video_paths(&app, vec![request.path], &import.cancellation)?;
        let replacement = prepared
            .results
            .pop()
            .ok_or_else(|| CommandErrorDto::new("media_probe_failed", "替换媒体未返回探测结果"))?;
        let result = commit_playback_pool_edit(&window, &state, Some(&import), move |playback| {
            let snapshot = playback.snapshot();
            let index =
                source_media_index_by_path(&snapshot.source_media_pool, &request.source_path)?;
            playback
                .replace_source_at(index, replacement.source.clone())
                .map_err(command_error_from_playback)
        });
        match result {
            Ok(snapshot) => {
                prepared.retain_created_files();
                Ok(snapshot)
            }
            Err(error) => Err(error),
        }
    })
    .await
    .map_err(|error| {
        CommandErrorDto::new("media_probe_failed", format!("媒体替换任务失败：{error}"))
    })?
}

#[tauri::command]
pub fn reorder_playback_pool_items(
    window: Window,
    state: State<'_, AppState>,
    request: ReorderPlaybackPoolItemsRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    commit_playback_pool_edit(&window, state.inner(), None, move |playback| {
        apply_source_media_order(playback, &request.source_paths)
    })
}

#[tauri::command]
pub fn remove_playback_pool_item(
    window: Window,
    state: State<'_, AppState>,
    request: RemovePlaybackPoolItemRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    commit_playback_pool_edit(&window, state.inner(), None, move |playback| {
        let snapshot = playback.snapshot();
        let index = source_media_index_by_path(&snapshot.source_media_pool, &request.source_path)?;
        playback
            .remove_source(index)
            .map_err(command_error_from_playback)
    })
}

#[tauri::command]
pub fn clear_playback_pool(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    commit_playback_pool_edit(&window, state.inner(), None, |playback| {
        playback.clear_source_pool();
        Ok(())
    })
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
    let (results, _) = probe_local_video_pool_blocking(window, app, state, vec![request.path])?;
    results
        .into_iter()
        .next()
        .ok_or_else(|| CommandErrorDto::new("media_probe_failed", "单媒体导入未返回探测结果"))
}

fn probe_local_video_paths(
    app: &AppHandle,
    paths: Vec<String>,
    cancellation: &CancellationToken,
) -> Result<PreparedMediaImports, CommandErrorDto> {
    validate_source_media_pool_count(paths.len())?;
    validate_source_media_paths(&paths)?;
    let target_root = runtime_resource_target_root(app).map_err(|error| {
        CommandErrorDto::new(
            "media_probe_failed",
            format!("读取已验证媒体运行资源目录失败：{error}"),
        )
    })?;
    let (ffmpeg_path, ffprobe_path) = configured_media_engine_paths_with_resource_dir(&target_root)
        .map_err(|error| CommandErrorDto::new("media_probe_failed", error.to_string()))?;
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandErrorDto::new("media_cache_dir_failed", error.to_string()))?
        .join("media-compatibility");
    let mut canonical_paths = HashSet::with_capacity(paths.len());
    let mut results = Vec::with_capacity(paths.len());
    let mut created_compatibility_files = Vec::new();
    let prepared = (|| {
        for path in paths {
            let mut result = probe_user_selected_video_with_ffprobe(
                &MediaProbeRequestDto { path },
                &ffprobe_path,
                MEDIA_IMPORT_PROBE_TIMEOUT_MS,
                cancellation,
            )
            .map_err(command_error_from_media_library)?;
            insert_canonical_source_path(&mut canonical_paths, &result.canonical_path)?;
            let compatibility = prepare_media_compatibility(
                &MediaCompatibilityRequest {
                    ffmpeg_path: ffmpeg_path.clone(),
                    ffprobe_path: ffprobe_path.clone(),
                    cache_dir: cache_dir.clone(),
                    source: result.source.clone(),
                    timeout_seconds: MEDIA_IMPORT_COMPATIBILITY_TIMEOUT_SECONDS,
                },
                cancellation,
            )
            .map_err(|error| {
                CommandErrorDto::new("media_compatibility_failed", error.to_string())
            })?;
            result.source.playback_reference = compatibility.playback_reference;
            result.source.compatibility_mode = compatibility.mode;
            if let Some(path) = compatibility.created_file {
                created_compatibility_files.push(path);
            }
            results.push(result);
        }
        if cancellation.is_cancelled() {
            return Err(command_error_from_media_library(
                MediaLibraryError::Cancelled,
            ));
        }
        for result in &results {
            allow_local_playback_asset_file(
                app,
                Path::new(&result.source.source_path),
                "source_media_asset_scope_failed",
                "源媒体",
            )?;
            allow_local_playback_asset_file(
                app,
                Path::new(&result.source.playback_reference),
                "playback_media_asset_scope_failed",
                "媒体播放引用",
            )?;
        }
        Ok(PreparedMediaImports {
            results,
            created_compatibility_files: created_compatibility_files.clone(),
        })
    })();
    if prepared.is_err() {
        cleanup_compatibility_files(created_compatibility_files);
    }
    prepared
}

fn probe_local_video_pool_blocking(
    window: Window,
    app: AppHandle,
    state: AppState,
    paths: Vec<String>,
) -> Result<(Vec<MediaProbeResultDto>, PlaybackSnapshotDto), CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let import = state.begin_media_import()?;
    let mut prepared = probe_local_video_paths(&app, paths, &import.cancellation)?;
    let results = prepared.results.clone();

    let _transition_guard = state.playback_transition.lock().map_err(|_| {
        CommandErrorDto::new("playback_transition_lock_failed", "播放池转换锁已损坏")
    })?;
    let _import_guard = lock_current_media_import(&state, &import)?;
    state.stop_media_workers()?;
    state.stop_audio_for_playback()?;
    state.stop_speech_worker()?;
    state.stop_realtime_video_runtime()?;
    let before = state.with_playback(&window, |playback| Ok(playback.snapshot()))?;
    let sources = results.iter().map(|result| result.source.clone()).collect();
    let snapshot = state.with_playback(&window, |playback| {
        playback
            .set_source_pool(sources)
            .map_err(command_error_from_playback)?;
        Ok(state.snapshot(playback))
    });
    let snapshot = snapshot?;
    prepared.retain_created_files();
    cleanup_replaced_compatibility_files(&before, &snapshot);
    Ok((results, snapshot))
}

fn lock_current_media_import<'a>(
    state: &'a AppState,
    media_import: &MediaImportLease,
) -> Result<std::sync::MutexGuard<'a, Option<CancellationToken>>, CommandErrorDto> {
    let guard = state.media_import_cancellation.lock().map_err(|_| {
        CommandErrorDto::new("media_import_lock_failed", "媒体导入取消状态锁已损坏")
    })?;
    if media_import.cancellation.is_cancelled()
        || state.media_import_generation.load(Ordering::Acquire) != media_import.generation
        || guard.is_none()
    {
        return Err(command_error_from_media_library(
            MediaLibraryError::Cancelled,
        ));
    }
    Ok(guard)
}

fn commit_playback_pool_edit(
    window: &Window,
    state: &AppState,
    media_import: Option<&MediaImportLease>,
    edit: impl Fn(&mut PlaybackCore) -> Result<(), CommandErrorDto>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_main_window(window)?;
    if media_import.is_none() {
        state.cancel_media_import()?;
    }
    let _transition_guard = state.playback_transition.lock().map_err(|_| {
        CommandErrorDto::new("playback_transition_lock_failed", "播放池转换锁已损坏")
    })?;
    let _import_guard = media_import
        .map(|media_import| lock_current_media_import(state, media_import))
        .transpose()?;
    let mut validated = state.with_playback(window, |playback| Ok(playback.clone()))?;
    let current = validated.snapshot();
    edit(&mut validated)?;
    if validated.snapshot() == current {
        return Ok(state.snapshot_from_core(current));
    }

    state.stop_media_workers()?;
    state.stop_audio_for_playback()?;
    state.stop_speech_worker()?;
    state.stop_realtime_video_runtime()?;
    let snapshot = state.with_playback(window, |playback| {
        edit(playback)?;
        Ok(state.snapshot(playback))
    })?;
    cleanup_replaced_compatibility_files(&current, &snapshot);
    Ok(snapshot)
}

fn cleanup_replaced_compatibility_files(before: &PlaybackSnapshot, after: &PlaybackSnapshotDto) {
    let retained: HashSet<&str> = after
        .source_media_pool
        .iter()
        .map(|source| source.playback_reference.as_str())
        .collect();
    let removed = before
        .source_media_pool
        .iter()
        .filter(|source| {
            source.playback_reference != source.source_path
                && !retained.contains(source.playback_reference.as_str())
        })
        .map(|source| PathBuf::from(&source.playback_reference));
    cleanup_compatibility_files(removed);
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

fn collect_realtime_video_parameters(
    params: &MediaEffectParams,
) -> Result<Vec<RealtimeVideoParameter>, CommandErrorDto> {
    fn walk(
        path: &str,
        current: &serde_json::Value,
        defaults: &serde_json::Value,
        output: &mut Vec<RealtimeVideoParameter>,
    ) {
        if let serde_json::Value::Object(fields) = current {
            for (name, value) in fields {
                let next_path = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}.{name}")
                };
                walk(
                    &next_path,
                    value,
                    defaults.get(name).unwrap_or(&serde_json::Value::Null),
                    output,
                );
            }
            return;
        }
        let active = current != defaults;
        let value = current
            .as_f64()
            .or_else(|| {
                current
                    .as_bool()
                    .map(|enabled| if enabled { 1.0 } else { 0.0 })
            })
            .unwrap_or(0.0);
        output.push(if active {
            RealtimeVideoParameter::active(path, value)
        } else {
            RealtimeVideoParameter::inactive(path, value)
        });
    }

    let current = serde_json::json!({
        "video": &params.video,
        "advanced": &params.advanced,
    });
    let defaults = MediaEffectParams::default();
    let defaults = serde_json::json!({
        "video": &defaults.video,
        "advanced": &defaults.advanced,
    });
    let mut output = Vec::new();
    walk("", &current, &defaults, &mut output);
    if output.is_empty() {
        return Err(CommandErrorDto::new(
            "realtime_video_params_empty",
            "实时画面参数快照为空",
        ));
    }
    Ok(output)
}

fn compile_gpu83_realtime_parameters(
    params: &MediaEffectParams,
) -> Result<CompiledRealtimeParameters, CommandErrorDto> {
    let active_fields = collect_realtime_video_parameters(params)?
        .into_iter()
        .filter(|parameter| parameter.active)
        .map(|parameter| parameter.field)
        .collect::<HashSet<_>>();
    let snapshot =
        build_gpu83_shader_snapshot(&params.video, &params.advanced).map_err(|error| {
            CommandErrorDto::new(
                "gpu83_snapshot_invalid",
                format!("GPU83 参数快照无效（{}:{}）", error.field, error.code),
            )
        })?;
    let update = snapshot.mpv_property_update();
    let options = MpvShaderOptions::parse(update.value)
        .map_err(|error| CommandErrorDto::new("gpu83_snapshot_invalid", error.to_string()))?;
    let parameters = snapshot
        .entries
        .into_iter()
        .map(|entry| {
            let mapping = GPU83_PARAMETER_MAPPINGS
                .iter()
                .find(|mapping| mapping.field_path == entry.field_path);
            let supported = entry.capability == Gpu83ParameterCapability::ShaderParameter;
            ParameterSupportResult {
                field: entry.field_path.to_owned(),
                active: active_fields.contains(entry.field_path),
                supported,
                mapping: supported.then(|| {
                    format!(
                        "mpv_shader_option:{}",
                        mapping
                            .map(|mapping| mapping.shader_option)
                            .unwrap_or("unknown")
                    )
                }),
                reason: match entry.capability {
                    Gpu83ParameterCapability::ShaderParameter => None,
                    Gpu83ParameterCapability::Unavailable(reason) => Some(reason.to_owned()),
                },
            }
        })
        .collect::<Vec<_>>();
    let fully_supported = parameters
        .iter()
        .all(|parameter| !parameter.active || parameter.supported);
    Ok(CompiledRealtimeParameters {
        commands: vec![MpvCommand::SetShaderOptions { options }],
        support: ParameterSupportReport {
            backend: VideoBackend::RealtimeGpu,
            fully_supported,
            parameters,
        },
    })
}

fn record_realtime_video_fallback(
    state: &AppState,
    reason: String,
) -> Result<MediaVideoBackendRuntimeStatus, CommandErrorDto> {
    let mut runtime = state.realtime_video_runtime.lock().map_err(|_| {
        CommandErrorDto::new("realtime_video_lock_failed", "实时画面运行时锁已损坏")
    })?;
    runtime.record_source_backend(reason);
    Ok(runtime.status())
}

#[cfg(windows)]
fn final_effect_host_window_id(window: &tauri::WebviewWindow) -> Result<u64, CommandErrorDto> {
    let hwnd = window.hwnd().map_err(|error| {
        CommandErrorDto::new(
            "final_effect_window_handle_failed",
            format!("读取最终效果窗口句柄失败：{error}"),
        )
    })?;
    let value = hwnd.0 as usize as u64;
    if value == 0 {
        return Err(CommandErrorDto::new(
            "final_effect_window_handle_failed",
            "最终效果窗口句柄无效",
        ));
    }
    Ok(value)
}

#[cfg(not(windows))]
fn final_effect_host_window_id(_window: &tauri::WebviewWindow) -> Result<u64, CommandErrorDto> {
    Err(CommandErrorDto::new(
        "realtime_video_platform_unsupported",
        "mpv 实时画面主链当前只支持 Windows",
    ))
}

#[tauri::command]
pub fn get_media_video_backend_status(
    window: Window,
    state: State<'_, AppState>,
) -> Result<MediaVideoBackendRuntimeStatus, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    state
        .realtime_video_runtime
        .lock()
        .map_err(|_| CommandErrorDto::new("realtime_video_lock_failed", "实时画面运行时锁已损坏"))
        .map(|runtime| runtime.status())
}

#[tauri::command]
pub fn prepare_realtime_video_plan(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: PrepareRealtimeVideoPlanRequestDto,
) -> Result<MediaVideoBackendRuntimeStatus, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let _transition_guard = state.playback_transition.lock().map_err(|_| {
        CommandErrorDto::new("playback_transition_lock_failed", "播放池转换锁已损坏")
    })?;
    state.ensure_running()?;
    request.params.validate().map_err(|errors| {
        CommandErrorDto::new(
            "media_processing_params_invalid",
            errors
                .iter()
                .map(|error| format!("{}: {}", error.field, error.message))
                .collect::<Vec<_>>()
                .join("；"),
        )
    })?;
    if request.sequence == 0 || request.period_ms == 0 {
        return Err(CommandErrorDto::new(
            "realtime_video_plan_invalid",
            "实时画面 sequence 和 period_ms 必须大于 0",
        ));
    }
    let snapshot = state
        .playback
        .lock()
        .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
        .snapshot();
    if request.playback_generation != snapshot.playback_generation
        || request.source_revision != VIDEO_BACKEND_SOURCE_REVISION
    {
        return Err(CommandErrorDto::new(
            "stale_realtime_video_plan",
            "实时画面计划绑定的播放代次已过期",
        ));
    }
    let source = snapshot
        .source_media
        .as_ref()
        .ok_or_else(|| CommandErrorDto::new("source_media_required", "请先导入一个源媒体"))?;
    if source.media_kind != MediaKind::Video || !snapshot.video_processing_enabled {
        return record_realtime_video_fallback(
            &state,
            "当前源不是视频或视频处理开关未开启".to_owned(),
        );
    }
    let generation = snapshot.playback_generation;
    let source_path = PathBuf::from(&source.source_path);
    let source_start_ms = snapshot.current_position_ms;
    let paused = snapshot.playback_state != PlaybackState::Playing;

    let compiled = compile_gpu83_realtime_parameters(&request.params)?;
    if !compiled.support.fully_supported {
        let count = compiled.support.ignored_active_parameter_count();
        let examples = compiled
            .support
            .ignored_active_parameter_examples(3)
            .join("、");
        return record_realtime_video_fallback(
            &state,
            format!("当前快照含 {count} 个 GPU83 未接入参数：{examples}"),
        );
    }
    let final_effect_window = app.get_webview_window("final-effect").ok_or_else(|| {
        CommandErrorDto::new(
            "final_effect_window_missing",
            "最终效果窗口尚未打开，无法准备实时画面",
        )
    })?;
    let host_window_id = final_effect_host_window_id(&final_effect_window)?;
    let runtime_root = runtime_resource_target_root(&app)
        .map_err(|error| CommandErrorDto::new("media_resource_dir_failed", error))?;
    let executable = resolve_mpv_executable(&runtime_root)
        .map_err(|error| CommandErrorDto::new("mpv_resource_invalid", error.to_string()))?;
    let resource_dir = app.path().resource_dir().map_err(|error| {
        CommandErrorDto::new(
            "mpv_shader_resource_dir_failed",
            format!("读取 mpv shader 资源目录失败：{error}"),
        )
    })?;
    let shader_path = resource_dir.join("resources/shaders/gpu83.hook");
    let shader = resolve_mpv_shader(&resource_dir, &shader_path)
        .map_err(|error| CommandErrorDto::new("mpv_shader_invalid", error.to_string()))?;

    let identity = VideoPlanIdentity {
        session_id: generation,
        playback_generation: generation,
        source_revision: request.source_revision,
        parameter_revision: request.sequence,
        sequence: request.sequence,
    };
    let plan = RealtimeVideoPlan {
        slot: VideoPlanSlot::NPlus1,
        identity,
        target_pts_ms: request.target_absolute_position_ms,
        period_ms: request.period_ms,
        seed: request.seed,
        prepared: true,
        commands: compiled.commands,
        parameter_support: compiled.support,
    };
    let n2 = request
        .next_sequence
        .zip(request.next_target_absolute_position_ms)
        .map(|(sequence, target_pts_ms)| CycleSlotStatus {
            sequence,
            target_pts_ms,
            status: CycleSlotState::Planned,
        });
    let mut runtime = state.realtime_video_runtime.lock().map_err(|_| {
        CommandErrorDto::new("realtime_video_lock_failed", "实时画面运行时锁已损坏")
    })?;
    state.ensure_running()?;
    runtime
        .prepare(
            PrepareRealtimeRenderer {
                executable,
                shader,
                source_path: &source_path,
                host_window_id,
                source_start_ms,
                paused,
                plan,
                n2,
                session_id: generation,
            },
            unix_now_ms(),
        )
        .map_err(|error| CommandErrorDto::new("realtime_video_prepare_failed", error.to_string()))
}

#[tauri::command]
pub fn commit_realtime_video_plan(
    window: Window,
    state: State<'_, AppState>,
    request: CommitRealtimeVideoPlanRequestDto,
) -> Result<MediaVideoBackendRuntimeStatus, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    state.ensure_running()?;
    let snapshot = state
        .playback
        .lock()
        .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
        .snapshot();
    if request.playback_generation != snapshot.playback_generation
        || request.source_revision != VIDEO_BACKEND_SOURCE_REVISION
    {
        return Err(CommandErrorDto::new(
            "stale_realtime_video_plan",
            "实时画面提交绑定的播放代次已过期",
        ));
    }
    let identity = VideoPlanIdentity {
        session_id: request.playback_generation,
        playback_generation: request.playback_generation,
        source_revision: request.source_revision,
        parameter_revision: request.sequence,
        sequence: request.sequence,
    };
    let mut runtime = state.realtime_video_runtime.lock().map_err(|_| {
        CommandErrorDto::new("realtime_video_lock_failed", "实时画面运行时锁已损坏")
    })?;
    state.ensure_running()?;
    runtime
        .commit(
            &VideoCommitGate {
                identity,
                media_pts_ms: request.media_pts_ms,
            },
            unix_now_ms(),
        )
        .map_err(|error| CommandErrorDto::new("realtime_video_commit_failed", error.to_string()))
}

#[tauri::command]
pub fn sync_realtime_video_renderer(
    window: Window,
    state: State<'_, AppState>,
    request: SyncRealtimeVideoRendererRequestDto,
) -> Result<MediaVideoBackendRuntimeStatus, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    state.ensure_running()?;
    let generation = state
        .playback
        .lock()
        .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
        .snapshot()
        .playback_generation;
    if generation != request.playback_generation {
        return Err(CommandErrorDto::new(
            "stale_realtime_video_sync",
            "实时画面同步消息属于旧播放代次",
        ));
    }
    let mut runtime = state.realtime_video_runtime.lock().map_err(|_| {
        CommandErrorDto::new("realtime_video_lock_failed", "实时画面运行时锁已损坏")
    })?;
    state.ensure_running()?;
    runtime
        .synchronize(request.position_ms, request.paused, unix_now_ms())
        .map_err(|error| CommandErrorDto::new("realtime_video_sync_failed", error.to_string()))?;
    Ok(runtime.status())
}

#[tauri::command]
pub fn stop_realtime_video_renderer(
    window: Window,
    state: State<'_, AppState>,
) -> Result<MediaVideoBackendRuntimeStatus, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let mut runtime = state.realtime_video_runtime.lock().map_err(|_| {
        CommandErrorDto::new("realtime_video_lock_failed", "实时画面运行时锁已损坏")
    })?;
    runtime.stop();
    Ok(runtime.status())
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
    pub mfcc: Vec<f32>,
    pub mfcc_available: bool,
    pub noise_floor_dbfs: f32,
    pub snr_db: Option<f32>,
    pub formants_hz: [Option<f32>; 3],
    pub current_formant_hz: Option<f32>,
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
    pub audio: autolive_desktop_core::media_effect_params::AudioEffectParams,
    #[serde(default)]
    pub audio_variants: Vec<autolive_desktop_core::media_effect_params::AudioEffectParams>,
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
    pub reason_code: Option<&'static str>,
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
    // 多支路/特征滤镜初始化可能跨过若干回调。仅在运行约 5 秒且至少 75% 回调欠载
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
    let mfcc_dimensions = state
        .playback
        .lock()
        .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
        .audio_stream_configuration()
        .0
        .mfcc_dimensions;
    Ok(audio_cycle_diagnostic_dto(snapshot, mfcc_dimensions))
}

fn audio_cycle_diagnostic_dto(
    mut snapshot: AudioLowFrequencyDiagnosticSnapshot,
    mfcc_dimensions: u8,
) -> AudioCycleDiagnosticDto {
    let line = if snapshot.has_pcm {
        snapshot.line.to_vec()
    } else {
        Vec::new()
    };
    snapshot.mfcc.truncate(usize::from(mfcc_dimensions));
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
        mfcc: snapshot.mfcc,
        mfcc_available: snapshot.mfcc_available,
        noise_floor_dbfs: snapshot.noise_floor_dbfs,
        snr_db: snapshot.snr_db,
        formants_hz: snapshot.formants_hz,
        current_formant_hz: snapshot.current_formant_hz,
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
    state.ensure_running()?;
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
        let mut output_slot = state.audio_cycle_output.lock().map_err(|_| {
            CommandErrorDto::new("audio_cycle_output_lock_failed", "音频周期输出状态锁已损坏")
        })?;
        if let Err(error) = state.ensure_running() {
            drop(output_slot);
            let _ = output.shutdown();
            return Err(error);
        }
        output_slot.replace(output);
        drop(output_slot);

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
    let snapshot_dto = || state.snapshot_from_core(snapshot.clone());
    let duration_ms = source_duration_ms(&snapshot)?;
    let live_clock = match validate_audio_sync_clock(
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
                reason_code: audio_sync_clock_rejection_code(reason),
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
            reason_code: None,
            snapshot: snapshot_dto(),
        });
    };
    if pending.candidate_id() != Some(request.candidate_id) {
        return Ok(CommitAudioCycleCandidateResultDto {
            candidate_id: request.candidate_id,
            committed: false,
            reason: Some("候选 ID 已过期，保持当前音轨".to_owned()),
            reason_code: None,
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
            reason_code: Some("audio_mixer_candidate_stale"),
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
            reason_code: None,
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
            reason_code: None,
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
            reason_code: None,
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
            reason_code: None,
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
            reason_code: None,
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
            reason_code: None,
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
            reason_code: None,
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
        let (reason, reason_code) = audio_cycle_crossfade_failure(reason);
        return Ok(CommitAudioCycleCandidateResultDto {
            candidate_id: request.candidate_id,
            committed: false,
            reason: Some(reason),
            reason_code,
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
            reason_code: None,
            snapshot: snapshot_dto(),
        });
    };
    let committed_snapshot = {
        let mut playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        playback.commit_validated_audio_stream_configuration(candidate_state.configuration);
        state.snapshot(&playback)
    };
    if let Some(mut old) = old {
        old.stop_preserving_output();
    }
    Ok(CommitAudioCycleCandidateResultDto {
        candidate_id: request.candidate_id,
        committed: true,
        reason: None,
        reason_code: None,
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
pub fn cleanup_local_caches_command(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CacheCleanupResultDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    cleanup_media_processing_cache(&app, &state)
}

#[tauri::command]
pub fn release_media_processing_artifact(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: ReleaseMediaProcessingArtifactRequestDto,
) -> Result<bool, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandErrorDto::new("media_cache_dir_failed", error.to_string()))?
        .join("media-processing");
    let path = PathBuf::from(request.path);
    let protected = state
        .playback
        .lock()
        .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
        .snapshot();
    if protected.current_video_reference.as_deref() == path.to_str()
        || protected.pending_video_reference.as_deref() == path.to_str()
    {
        return Ok(false);
    }
    remove_managed_media_processing_artifact(&cache_dir, &path)
}

#[tauri::command]
pub fn discard_media_processing_candidate(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: DiscardMediaProcessingCandidateRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let reason = request.reason.trim();
    if reason.is_empty() || reason.len() > 1_024 {
        return Err(CommandErrorDto::new(
            "media_candidate_discard_reason_invalid",
            "候选丢弃原因必须为 1..=1024 字节",
        ));
    }
    let _ = state.reap_finished_video_media_worker()?;
    let (artifact_path, snapshot) = state.with_playback_window(&window, |playback| {
        let artifact_path = playback.discard_media_processing_candidate(
            request.plan_id.trim(),
            request.sequence,
            request.playback_generation,
            request.source_revision,
            reason,
        );
        Ok((artifact_path, state.snapshot(playback)))
    })?;
    if let Some(path) = artifact_path {
        let cache_dir = app
            .path()
            .app_cache_dir()
            .map_err(|error| CommandErrorDto::new("media_cache_dir_failed", error.to_string()))?
            .join("media-processing");
        remove_managed_media_processing_artifact(&cache_dir, Path::new(&path))?;
    }
    Ok(snapshot)
}

#[tauri::command]
pub fn release_audio_media_candidate(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: ReleaseAudioMediaCandidateRequestDto,
) -> Result<bool, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandErrorDto::new("media_cache_dir_failed", error.to_string()))?
        .join("media-processing");
    let path = PathBuf::from(request.path);
    let snapshot = state
        .playback
        .lock()
        .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
        .snapshot();
    if snapshot.current_audio_artifact_reference.as_deref() == path.to_str()
        || snapshot.pending_audio_artifact_reference.as_deref() == path.to_str()
    {
        return Ok(false);
    }
    remove_managed_media_processing_artifact(&cache_dir, &path)
}

#[tauri::command]
pub fn discard_audio_media_candidate(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: DiscardAudioMediaCandidateRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let reason = request.reason.trim();
    if reason.is_empty() || reason.len() > 1_024 {
        return Err(CommandErrorDto::new(
            "audio_media_candidate_discard_reason_invalid",
            "声音候选丢弃原因必须为 1..=1024 字节",
        ));
    }
    let _ = state.reap_finished_audio_media_worker()?;
    let (artifact_path, snapshot) = state.with_playback_window(&window, |playback| {
        let artifact_path = playback.discard_audio_media_candidate(
            request.plan_id.trim(),
            request.sequence,
            request.playback_generation,
            request.source_revision,
            reason,
        );
        Ok((artifact_path, state.snapshot(playback)))
    })?;
    if let Some(path) = artifact_path {
        let cache_dir = app
            .path()
            .app_cache_dir()
            .map_err(|error| CommandErrorDto::new("media_cache_dir_failed", error.to_string()))?
            .join("media-processing");
        remove_managed_media_processing_artifact(&cache_dir, Path::new(&path))?;
    }
    Ok(snapshot)
}

#[tauri::command]
pub fn commit_audio_media_candidate(
    window: Window,
    state: State<'_, AppState>,
    request: AudioMediaCandidateIdentityRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    let _ = state.reap_finished_audio_media_worker()?;
    state.playback_action(&window, |playback| {
        playback.commit_audio_media_candidate(
            request.plan_id.trim(),
            request.sequence,
            request.playback_generation,
            request.source_revision,
        );
        Ok(())
    })
}

#[tauri::command]
pub fn prepare_audio_media_candidate(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: PrepareAudioMediaCandidateRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let _transition_guard = state.playback_transition.lock().map_err(|_| {
        CommandErrorDto::new("playback_transition_lock_failed", "播放池转换锁已损坏")
    })?;
    let _ = state.reap_finished_audio_media_worker()?;
    if state.audio_media_worker_is_running()? {
        return Err(CommandErrorDto::new(
            "media_worker_already_running",
            "当前已有本地媒体候选 FFmpeg 在执行",
        ));
    }
    let plan_id = request.plan_id.trim().to_owned();
    if plan_id.is_empty() || plan_id.len() > 128 {
        return Err(CommandErrorDto::new(
            "audio_media_plan_id_invalid",
            "声音候选 plan_id 必须为 1..=128 字节的非空字符串",
        ));
    }
    if request.sequence == 0 {
        return Err(CommandErrorDto::new(
            "audio_media_sequence_invalid",
            "声音候选 sequence 必须大于 0",
        ));
    }
    if request.valid_until_absolute_position_ms <= request.target_absolute_position_ms {
        return Err(CommandErrorDto::new(
            "audio_media_target_window_invalid",
            "声音候选有效终点必须晚于目标绝对媒体时间",
        ));
    }
    let covered_duration_ms = request
        .valid_until_absolute_position_ms
        .checked_sub(request.target_absolute_position_ms)
        .ok_or_else(|| {
            CommandErrorDto::new(
                "audio_media_target_window_invalid",
                "声音候选绝对媒体窗口计算失败",
            )
        })?;
    if covered_duration_ms > request.output_duration_ms {
        return Err(CommandErrorDto::new(
            "audio_media_output_window_too_short",
            "声音候选输出时长不能覆盖目标绝对媒体窗口",
        ));
    }

    let (
        source_path,
        source_duration_ms,
        source_audio_start_ms,
        source_audio_end_ms,
        source_sample_rate_hz,
        generation,
        revision,
        single,
    ) = {
        let playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        let snapshot = playback.snapshot();
        if snapshot.pending_audio_media_plan_id.is_some() {
            return Err(CommandErrorDto::new(
                "audio_media_candidate_pending",
                "已有待提交声音候选，请先提交或丢弃",
            ));
        }
        let source = snapshot
            .source_media
            .as_ref()
            .ok_or_else(|| CommandErrorDto::new("source_media_required", "请先导入一个源媒体"))?;
        if !snapshot.audio_processing_enabled {
            return Err(CommandErrorDto::new(
                "audio_processing_disabled",
                "普通声音处理尚未开启",
            ));
        }
        (
            source.source_path.clone(),
            source.duration_ms,
            source.audio_start_ms,
            source.audio_end_ms,
            source.audio_sample_rate_hz,
            snapshot.playback_generation,
            snapshot.audio_stream_revision,
            snapshot.source_media_pool.len() == 1,
        )
    };
    if request.playback_generation != generation || request.source_revision != revision {
        return Err(CommandErrorDto::new(
            "stale_audio_media_candidate",
            "声音候选绑定的播放代次或源修订已过期",
        ));
    }
    if request.loop_source && !single {
        return Err(CommandErrorDto::new(
            "audio_media_source_loop_invalid",
            "只有单项播放池声音候选允许跨 EOF 循环当前源媒体",
        ));
    }
    validate_audio_candidate_source_window(
        source_duration_ms,
        source_audio_start_ms,
        source_audio_end_ms,
        request.source_start_ms,
        request.output_duration_ms,
        request.loop_source,
    )?;

    let target_root = runtime_resource_target_root(&app)
        .map_err(|error| CommandErrorDto::new("runtime_resource_directory_unavailable", error))?;
    let (ffmpeg_path, ffprobe_path) = configured_media_engine_paths_with_resource_dir(&target_root)
        .map_err(|error| CommandErrorDto::new("media_engine_unavailable", error.to_string()))?;
    let ambient_sound = resolve_user_ambient_sound(
        &app,
        &ffmpeg_path,
        request.ambient_sound_path.as_deref(),
        audio_mix_requires_ambient_sound(&request.params, &request.audio_variants),
    )?;
    let ambient_sound_path = ambient_sound
        .as_ref()
        .map(|selection| selection.path.clone());
    build_audio_stream_filter_graph_with_ambient(
        &request.params,
        &request.audio_variants,
        source_sample_rate_hz,
        autolive_portaudio_output::DEFAULT_SAMPLE_RATE_HZ,
        ambient_sound_path.is_some(),
    )
    .map_err(|error| CommandErrorDto::new("audio_stream_filter_invalid", error.to_string()))?;
    let configuration = ValidatedAudioStreamConfiguration::new(
        request.params.clone(),
        request.audio_variants.clone(),
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

    let candidate = state.with_playback(&window, |playback| {
        let current = playback.snapshot();
        if current.playback_generation != request.playback_generation
            || current.audio_stream_revision != request.source_revision
        {
            return Err(CommandErrorDto::new(
                "stale_audio_media_candidate",
                "声音候选绑定的播放代次或源修订已过期",
            ));
        }
        let candidate = PendingAudioMediaCandidateIdentity {
            plan_id: plan_id.clone(),
            sequence: request.sequence,
            playback_generation: current.playback_generation,
            source_revision: current.audio_stream_revision,
            target_absolute_position_ms: request.target_absolute_position_ms,
            source_start_ms: request.source_start_ms,
            output_duration_ms: request.output_duration_ms,
            valid_until_absolute_position_ms: request.valid_until_absolute_position_ms,
        };
        playback
            .mark_audio_media_processing_running(candidate, configuration.clone())
            .map_err(command_error_from_playback)
    })?;
    *state.ambient_sound.lock().map_err(|_| {
        CommandErrorDto::new("ambient_sound_path_lock_failed", "环境声素材状态锁已损坏")
    })? = ambient_sound;

    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandErrorDto::new("media_cache_dir_failed", error.to_string()))?
        .join("media-processing");
    std::fs::create_dir_all(&cache_dir)
        .map_err(|error| CommandErrorDto::new("media_cache_dir_failed", error.to_string()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CommandErrorDto::new("media_cache_nonce_failed", error.to_string()))?
        .as_nanos();
    let output_path = cache_dir.join(format!(
        "processed-audio-g{generation}-s{}-{nonce}.m4a",
        candidate.sequence
    ));
    let staging_path = cache_dir.join(format!(
        "processed-audio-g{generation}-s{}-{nonce}.partial.m4a",
        candidate.sequence
    ));
    let defaults = MediaEffectParams::default();
    let media_request = MediaRenderRequest {
        ffmpeg_path,
        ffprobe_path,
        input_mp4_path: PathBuf::from(source_path),
        // 独立声音候选明确不映射视频流，即使输入是视频文件。
        source_has_video: false,
        ambient_input_path: ambient_sound_path,
        source_duration_ms,
        source_start_ms: request.source_start_ms,
        output_duration_ms: request.output_duration_ms,
        loop_source: request.loop_source,
        staging_output_path: staging_path,
        output_mp4_path: output_path.clone(),
        video_processing_enabled: false,
        audio_processing_enabled: true,
        source_audio_sample_rate_hz: source_sample_rate_hz,
        video: defaults.video,
        audio: request.params,
        audio_variants: request.audio_variants,
        advanced: defaults.advanced,
        timeout_seconds: MEDIA_CANDIDATE_TIMEOUT_SECONDS,
        target: MediaRenderTarget::StandardMp4,
    };
    if let Err(error) = build_media_render_args(&media_request) {
        state.with_playback(&window, |playback| {
            playback.mark_audio_media_processing_failed(error.to_string());
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
    let worker_candidate = candidate.clone();
    let handle = thread::spawn(move || {
        let progress_playback = Arc::clone(&playback);
        let result = render_media_with_progress(&media_request, &worker_cancellation, |percent| {
            if let Ok(mut playback) = progress_playback.lock() {
                playback.mark_audio_media_processing_progress(percent);
            }
        });
        if let Ok(mut playback) = playback.lock() {
            match result {
                Ok(rendered) => match allow_local_playback_asset_file(
                    &thread_app,
                    &rendered.output_mp4_path,
                    "audio_media_asset_scope_failed",
                    "处理后声音",
                ) {
                    Ok(()) => {
                        if playback
                            .mark_audio_media_processing_ready(
                                &worker_candidate,
                                rendered.output_mp4_path.display().to_string(),
                                rendered.output_mp4_sha256,
                            )
                            .is_err()
                        {
                            let _ = std::fs::remove_file(rendered.output_mp4_path);
                        }
                    }
                    Err(error) => {
                        let _ = std::fs::remove_file(rendered.output_mp4_path);
                        playback.mark_audio_media_processing_failed(error.message);
                    }
                },
                Err(error) => playback.mark_audio_media_processing_failed(error.to_string()),
            }
        } else if let Ok(rendered) = result {
            let _ = std::fs::remove_file(rendered.output_mp4_path);
        }
        completed_for_thread.store(true, Ordering::Release);
    });
    if let Err(task) = state.install_audio_media_worker(BackgroundWorkerTask {
        cancellation,
        completed,
        handle,
    }) {
        task.cancellation.cancel();
        let _ = task.handle.join();
        let _ = std::fs::remove_file(output_path);
        state.with_playback(&window, |playback| {
            playback.mark_audio_media_processing_failed("当前已有本地媒体候选 FFmpeg 在执行");
            Ok(())
        })?;
        return Err(CommandErrorDto::new(
            "media_worker_already_running",
            "当前已有本地媒体候选 FFmpeg 在执行",
        ));
    }
    state.with_playback(&window, |playback| Ok(state.snapshot(playback)))
}

#[tauri::command]
pub fn start_media_processing(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: StartMediaProcessingRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let _transition_guard = state.playback_transition.lock().map_err(|_| {
        CommandErrorDto::new("playback_transition_lock_failed", "播放池转换锁已损坏")
    })?;
    let _ = state.reap_finished_video_media_worker()?;
    if state.video_media_worker_is_running()? {
        return Err(CommandErrorDto::new(
            "media_worker_already_running",
            "当前已有本地媒体处理 Worker 在执行",
        ));
    }
    let plan_id = request.plan_id.trim().to_owned();
    if plan_id.is_empty() || plan_id.len() > 128 {
        return Err(CommandErrorDto::new(
            "media_processing_plan_id_invalid",
            "媒体候选 plan_id 必须为 1..=128 字节的非空字符串",
        ));
    }
    if request.sequence == 0 {
        return Err(CommandErrorDto::new(
            "media_processing_sequence_invalid",
            "媒体候选 sequence 必须大于 0",
        ));
    }
    if request.valid_until_absolute_position_ms <= request.target_absolute_position_ms {
        return Err(CommandErrorDto::new(
            "media_processing_target_window_invalid",
            "媒体候选有效终点必须晚于目标绝对媒体时间",
        ));
    }
    let covered_absolute_duration_ms = request
        .valid_until_absolute_position_ms
        .checked_sub(request.target_absolute_position_ms)
        .ok_or_else(|| {
            CommandErrorDto::new(
                "media_processing_target_window_invalid",
                "媒体候选绝对媒体窗口计算失败",
            )
        })?;
    if covered_absolute_duration_ms > request.output_duration_ms {
        return Err(CommandErrorDto::new(
            "media_processing_output_window_too_short",
            "媒体候选输出时长不能覆盖目标绝对媒体窗口",
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
    let scope = request.scope.as_deref().unwrap_or("both");
    match scope {
        "video" | "audio" | "both" => {}
        _ => {
            return Err(CommandErrorDto::new(
                "media_processing_scope_invalid",
                "媒体处理 scope 只能是 video、audio 或 both",
            ));
        }
    };
    if request.require_atomic_video_admission {
        if scope != "video" {
            return Err(CommandErrorDto::new(
                "media_processing_atomic_video_scope_invalid",
                "83 项原子视频准入只允许用于独立 video 候选",
            ));
        }
        build_atomic_media_video_effect_plan(&request.params.video, &request.params.advanced)
            .map_err(|errors| {
                CommandErrorDto::new(
                    "media_processing_atomic_video_params_invalid",
                    errors
                        .iter()
                        .map(|error| format!("{}: {}", error.field, error.reason))
                        .collect::<Vec<_>>()
                        .join("；"),
                )
            })?;
    }
    let (
        source_path,
        source_duration_ms,
        generation,
        source_has_video,
        video_enabled,
        audio_enabled,
        source_audio_start_ms,
        source_audio_end_ms,
        source_audio_sample_rate_hz,
        audio_stream_revision,
        single_source_pool,
    ) = {
        let playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        let snapshot = playback.snapshot();
        let source = snapshot
            .source_media
            .as_ref()
            .ok_or_else(|| CommandErrorDto::new("source_media_required", "请先导入一个源媒体"))?;
        let source_path = source.source_path.clone();
        (
            source_path,
            source.duration_ms,
            snapshot.playback_generation,
            source.media_kind == MediaKind::Video,
            snapshot.video_processing_enabled
                && source.media_kind == MediaKind::Video
                && matches!(scope, "video" | "both"),
            snapshot.audio_processing_enabled && matches!(scope, "audio" | "both"),
            source.audio_start_ms,
            source.audio_end_ms,
            source.audio_sample_rate_hz,
            snapshot.audio_stream_revision,
            snapshot.source_media_pool.len() == 1,
        )
    };
    if request.loop_source && !single_source_pool {
        return Err(CommandErrorDto::new(
            "media_processing_source_loop_invalid",
            "只有单项播放池候选允许跨 EOF 循环当前源媒体",
        ));
    }
    if request.playback_generation != generation
        || (scope != "video" && request.source_revision != audio_stream_revision)
    {
        return Err(CommandErrorDto::new(
            "stale_media_processing_candidate",
            "媒体候选绑定的播放代次或源修订已过期",
        ));
    }
    if audio_enabled {
        validate_audio_candidate_source_window(
            source_duration_ms,
            source_audio_start_ms,
            source_audio_end_ms,
            request.source_start_ms,
            request.output_duration_ms,
            request.loop_source,
        )?;
    }
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
    let ambient_sound = if audio_enabled {
        resolve_user_ambient_sound(
            &app,
            &ffmpeg_path,
            request.ambient_sound_path.as_deref(),
            audio_mix_requires_ambient_sound(&request.params.audio, &audio_variants),
        )?
    } else {
        None
    };
    let ambient_sound_path = ambient_sound
        .as_ref()
        .map(|selection| selection.path.clone());
    if audio_enabled {
        build_audio_stream_filter_graph_with_ambient(
            &request.params.audio,
            &audio_variants,
            source_audio_sample_rate_hz,
            autolive_portaudio_output::DEFAULT_SAMPLE_RATE_HZ,
            ambient_sound_path.is_some(),
        )
        .map_err(|error| CommandErrorDto::new("audio_stream_filter_invalid", error.to_string()))?;
    }
    let candidate_identity = state.with_playback(&window, |playback| {
        let current = playback.snapshot();
        if current.playback_generation != request.playback_generation
            || (scope != "video" && current.audio_stream_revision != request.source_revision)
        {
            return Err(CommandErrorDto::new(
                "stale_media_processing_candidate",
                "媒体候选绑定的播放代次或源修订已过期",
            ));
        }
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
        let candidate_identity = PendingMediaCandidateIdentity {
            plan_id: plan_id.clone(),
            sequence: request.sequence,
            playback_generation: current.playback_generation,
            source_revision: playback.snapshot().audio_stream_revision,
            target_absolute_position_ms: request.target_absolute_position_ms,
            source_start_ms: request.source_start_ms,
            output_duration_ms: request.output_duration_ms,
            valid_until_absolute_position_ms: request.valid_until_absolute_position_ms,
        };
        playback
            .mark_media_processing_running_with_candidate(candidate_identity.clone())
            .map_err(command_error_from_playback)?;
        Ok(candidate_identity)
    })?;
    if audio_enabled {
        *state.ambient_sound.lock().map_err(|_| {
            CommandErrorDto::new("ambient_sound_path_lock_failed", "环境声素材状态锁已损坏")
        })? = ambient_sound;
    }
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandErrorDto::new("media_cache_dir_failed", error.to_string()))?
        .join("media-processing");
    std::fs::create_dir_all(&cache_dir)
        .map_err(|error| CommandErrorDto::new("media_cache_dir_failed", error.to_string()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CommandErrorDto::new("media_cache_nonce_failed", error.to_string()))?
        .as_nanos();
    let sequence = candidate_identity.sequence;
    let output_mp4_path =
        cache_dir.join(format!("processed-g{generation}-s{sequence}-{nonce}.mp4"));
    let staging_output_path = cache_dir.join(format!(
        "processed-g{generation}-s{sequence}-{nonce}.partial.mp4"
    ));
    let media_request = MediaRenderRequest {
        ffmpeg_path,
        ffprobe_path,
        input_mp4_path: PathBuf::from(&source_path),
        source_has_video,
        ambient_input_path: ambient_sound_path,
        source_duration_ms,
        source_start_ms: request.source_start_ms,
        output_duration_ms: request.output_duration_ms,
        loop_source: request.loop_source,
        staging_output_path,
        output_mp4_path: output_mp4_path.clone(),
        video_processing_enabled: video_enabled,
        audio_processing_enabled: audio_enabled,
        source_audio_sample_rate_hz,
        video: request.params.video.clone(),
        audio: request.params.audio.clone(),
        audio_variants,
        advanced: request.params.advanced.clone(),
        timeout_seconds: MEDIA_CANDIDATE_TIMEOUT_SECONDS,
        target: MediaRenderTarget::StandardMp4,
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
    let worker_candidate_identity = candidate_identity.clone();
    let handle = thread::spawn(move || {
        let progress_playback = Arc::clone(&playback);
        let result = render_media_with_progress(&media_request, &worker_cancellation, |percent| {
            if let Ok(mut playback) = progress_playback.lock() {
                if playback.snapshot().playback_generation == generation {
                    playback.mark_video_processing_progress(percent);
                }
            }
        });
        if let Ok(mut playback) = playback.lock() {
            if playback.snapshot().playback_generation == generation {
                match result {
                    Ok(rendered) => match allow_local_playback_asset_file(
                        &thread_app,
                        &rendered.output_mp4_path,
                        "media_processing_asset_scope_failed",
                        "处理后音视频",
                    ) {
                        Ok(()) => {
                            let accepted = playback.mark_media_processing_ready_for_candidate(
                                &worker_candidate_identity,
                                rendered.output_mp4_path.display().to_string(),
                                rendered.output_mp4_sha256,
                            );
                            if accepted.is_err() {
                                let _ = std::fs::remove_file(rendered.output_mp4_path);
                            }
                        }
                        Err(error) => {
                            let _ = std::fs::remove_file(rendered.output_mp4_path);
                            playback.mark_media_processing_failed(error.message)
                        }
                    },
                    Err(error) => playback.mark_media_processing_failed(error.to_string()),
                }
            } else if let Ok(rendered) = result {
                let _ = std::fs::remove_file(rendered.output_mp4_path);
            }
        } else if let Ok(rendered) = result {
            let _ = std::fs::remove_file(rendered.output_mp4_path);
        }
        completed_for_thread.store(true, Ordering::Release);
    });
    if let Err(task) = state.install_video_media_worker(BackgroundWorkerTask {
        cancellation,
        completed,
        handle,
    }) {
        task.cancellation.cancel();
        let _ = task.handle.join();
        let _ = std::fs::remove_file(output_mp4_path);
        state.with_playback(&window, |playback| {
            playback.mark_media_processing_failed("当前已有本地媒体处理 Worker 在执行");
            Ok(())
        })?;
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
    let _ = state.stop_media_workers();
    let _ = state.stop_speech_worker();
    let _ = state.stop_audio_mixer();
    if let Ok(mut runtime) = state.realtime_video_runtime.lock() {
        runtime.stop();
    }
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
        let destroyed = matches!(event, tauri::WindowEvent::Destroyed);
        if matches!(
            event,
            tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
        ) {
            disable_media_processing_on_final_effect_close(&app_handle);
        }
        if destroyed {
            if let Err(error) = purge_media_processing_cache(&app_handle) {
                eprintln!(
                    "failed to purge media processing cache after final effect close: {error}"
                );
            }
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
        // 先按默认尺寸创建；窗口加载后由最终效果页的 resize effect 调整。
        let window =
            WebviewWindowBuilder::new(&app, "final-effect", WebviewUrl::App("index.html".into()))
                .title("GpAutoLive 最终效果")
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
    // Wry 创建新窗口时只把任务排入主事件循环；此时立即读取
    // monitor/size 会在 WebView2 仍在创建时无界等待。已有窗口才在此处调整。
    if !created {
        if let Some(request) = request {
            let _ = resize_final_effect_window_for_app(&app, request, false)?;
        }
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
    resize_final_effect_window_for_app(&app, request, false)
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
    center_after_resize: bool,
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
    if center_after_resize {
        if let Err(error) = window.center() {
            let _ = window.set_size(PhysicalSize::new(previous_size.width, previous_size.height));
            return Err(CommandErrorDto::new(
                "final_effect_window_resize_failed",
                error.to_string(),
            ));
        }
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
    let _transition_guard = state.playback_transition.lock().map_err(|_| {
        CommandErrorDto::new("playback_transition_lock_failed", "播放池转换锁已损坏")
    })?;
    let snapshot = state.playback_action(&window, PlaybackCore::start)?;
    state.resume_audio_output(&app)?;
    Ok(snapshot)
}

#[tauri::command]
pub fn pause_playback(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    let _transition_guard = state.playback_transition.lock().map_err(|_| {
        CommandErrorDto::new("playback_transition_lock_failed", "播放池转换锁已损坏")
    })?;
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
    let _transition_guard = state.playback_transition.lock().map_err(|_| {
        CommandErrorDto::new("playback_transition_lock_failed", "播放池转换锁已损坏")
    })?;
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
    let _transition_guard = state.playback_transition.lock().map_err(|_| {
        CommandErrorDto::new("playback_transition_lock_failed", "播放池转换锁已损坏")
    })?;
    state.stop_media_workers()?;
    state.stop_audio_for_playback()?;
    state.stop_speech_worker()?;
    state
        .realtime_video_runtime
        .lock()
        .map_err(|_| CommandErrorDto::new("realtime_video_lock_failed", "实时画面运行时锁已损坏"))?
        .stop();
    state.with_playback_window(&window, |playback| {
        playback.stop();
        Ok(state.snapshot(playback))
    })
}

#[tauri::command]
pub fn complete_playback_loop(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: CompletePlaybackLoopRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    let current = state.with_playback_window(&window, |playback| Ok(playback.snapshot()))?;
    match should_complete_playback_loop(
        current.playback_generation,
        current.loop_index,
        request.playback_generation,
        request.target_loop_index,
    ) {
        Ok(false) => return Ok(state.snapshot_from_core(current)),
        Err(message) => {
            return Err(CommandErrorDto::new(
                "invalid_playback_loop_target",
                message,
            ));
        }
        Ok(true) => {}
    }
    complete_playback_item_inner(
        &window,
        &app,
        state.inner(),
        &CompletePlaybackItemRequestDto {
            playback_generation: request.playback_generation,
            loop_index: request.target_loop_index.saturating_sub(1),
            source_media_index: current.source_media_index,
        },
    )
    .map(|result| result.snapshot)
}

#[tauri::command]
pub fn complete_playback_item(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: CompletePlaybackItemRequestDto,
) -> Result<CompletePlaybackItemResultDto, CommandErrorDto> {
    complete_playback_item_inner(&window, &app, state.inner(), &request)
}

fn mark_audio_resume_failure(playback: &mut PlaybackCore, error: &CommandErrorDto) {
    playback.mark_audio_processing_unavailable(format!(
        "PortAudio 音频输出恢复失败，已回退 WebView：{}",
        error.message
    ));
}

fn complete_playback_item_inner(
    window: &Window,
    app: &AppHandle,
    state: &AppState,
    request: &CompletePlaybackItemRequestDto,
) -> Result<CompletePlaybackItemResultDto, CommandErrorDto> {
    state.ensure_playback_window(window)?;
    let _transition_guard = state.playback_transition.lock().map_err(|_| {
        CommandErrorDto::new("playback_transition_lock_failed", "播放池转换锁已损坏")
    })?;
    let current = state.with_playback_window(window, |playback| Ok(playback.snapshot()))?;
    let should_advance = should_complete_playback_item(
        current.playback_generation,
        current.loop_index,
        current.source_media_index,
        request.playback_generation,
        request.loop_index,
        request.source_media_index,
    )
    .map_err(|message| CommandErrorDto::new("invalid_playback_item_identity", message))?;
    if !should_advance {
        return Ok(CompletePlaybackItemResultDto {
            snapshot: state.snapshot_from_core(current),
            source_changed: false,
        });
    }

    let source_will_change = current.source_media_pool.len() > 1;
    if source_will_change {
        state.stop_audio_media_worker()?;
        state.stop_audio_for_playback()?;
        state.stop_speech_worker()?;
    } else {
        state.stop_speech_worker()?;
    }

    if source_will_change {
        state.stop_video_media_worker()?;
        state.stop_realtime_video_runtime()?;
    }

    let (mut snapshot, source_changed) = state.with_playback_window(window, |playback| {
        let snapshot = playback.snapshot();
        let should_advance = should_complete_playback_item(
            snapshot.playback_generation,
            snapshot.loop_index,
            snapshot.source_media_index,
            request.playback_generation,
            request.loop_index,
            request.source_media_index,
        )
        .map_err(|message| CommandErrorDto::new("invalid_playback_item_identity", message))?;
        if !should_advance {
            return Ok((state.snapshot(playback), false));
        }
        let source_changed = playback
            .complete_item()
            .map_err(command_error_from_playback)?;
        Ok((state.snapshot(playback), source_changed))
    })?;
    if source_changed {
        if let Err(error) = state.resume_audio_output(app) {
            let committed_generation = snapshot.playback_generation;
            let committed_source_index = snapshot.source_media_index;
            snapshot = state.with_playback_window(window, |playback| {
                let current = playback.snapshot();
                if current.playback_generation == committed_generation
                    && current.source_media_index == committed_source_index
                {
                    mark_audio_resume_failure(playback, &error);
                }
                Ok(state.snapshot(playback))
            })?;
        }
    }
    Ok(CompletePlaybackItemResultDto {
        snapshot,
        source_changed,
    })
}

#[tauri::command]
pub fn commit_media_processing_if_ready(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: CommitMediaProcessingRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    let _ = state.reap_finished_video_media_worker()?;
    let (outcome, snapshot) = state.with_playback_window(&window, |playback| {
        let outcome = playback.commit_media_processing_if_ready(
            request.plan_id.trim(),
            request.sequence,
            request.playback_generation,
            request.source_revision,
            request.observed_absolute_position_ms,
        );
        Ok((outcome, state.snapshot(playback)))
    })?;
    if let MediaProcessingCommitOutcome::Expired {
        artifact_path: Some(path),
    } = outcome
    {
        let cache_dir = app
            .path()
            .app_cache_dir()
            .map_err(|error| CommandErrorDto::new("media_cache_dir_failed", error.to_string()))?
            .join("media-processing");
        remove_managed_media_processing_artifact(&cache_dir, Path::new(&path))?;
    }
    Ok(snapshot)
}

#[tauri::command]
pub fn set_processing_switches(
    window: Window,
    state: State<'_, AppState>,
    request: ProcessingSwitchesRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    let (video_switch_changed, audio_switch_changed) = {
        let playback = state
            .playback
            .lock()
            .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?;
        let snapshot = playback.snapshot();
        (
            snapshot.video_processing_enabled != request.video_processing_enabled,
            snapshot.audio_processing_enabled != request.audio_processing_enabled,
        )
    };
    if video_switch_changed {
        state.stop_video_media_worker()?;
        state.stop_realtime_video_runtime()?;
    }
    if audio_switch_changed {
        state.stop_audio_media_worker()?;
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
    let _ = state.reap_finished_audio_media_worker()?;
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

fn validate_interlude_processing_request(
    request: &StartPortAudioInterludeRequestDto,
) -> Result<(), CommandErrorDto> {
    validate_interlude_audio_params(&request.audio, &request.audio_variants)
}

fn validate_interlude_audio_params(
    audio: &AudioEffectParams,
    audio_variants: &[AudioEffectParams],
) -> Result<(), CommandErrorDto> {
    ValidatedAudioStreamConfiguration::new(audio.clone(), audio_variants.to_vec()).map_err(
        |errors| {
            CommandErrorDto::new(
                "interlude_audio_stream_params_invalid",
                errors
                    .iter()
                    .map(|error| format!("{}: {}", error.field, error.message))
                    .collect::<Vec<_>>()
                    .join("；"),
            )
        },
    )?;
    if let Some((index, _)) = audio_variants
        .iter()
        .enumerate()
        .find(|(_, variant)| variant.playback_speed != audio.playback_speed)
    {
        return Err(CommandErrorDto::new(
            "interlude_audio_variant_speed_mismatch",
            format!("插话多轨支路 audio_variants[{index}].playback_speed 必须与主参数一致"),
        ));
    }
    Ok(())
}

fn validate_interlude_audio_path(
    snapshot: &PlaybackSnapshot,
    requested_path: &str,
) -> Result<PathBuf, CommandErrorDto> {
    if snapshot.playback_state != PlaybackState::Playing {
        return Err(CommandErrorDto::new(
            "interlude_playback_not_running",
            "视频未处于播放状态，不能启动随机插话",
        ));
    }
    let interlude = &snapshot.interlude;
    if !interlude.enabled || interlude.audio_files.is_empty() {
        return Err(CommandErrorDto::new(
            "interlude_not_ready",
            "随机插话尚未启用或目录内没有可用音频",
        ));
    }
    let canonical_path = std::fs::canonicalize(requested_path.trim()).map_err(|error| {
        CommandErrorDto::new(
            "interlude_audio_file_unavailable",
            format!("插话音频路径无法规范化：{error}"),
        )
    })?;
    if !interlude
        .audio_files
        .iter()
        .any(|path| Path::new(path) == canonical_path.as_path())
    {
        return Err(CommandErrorDto::new(
            "interlude_audio_file_not_allowed",
            "请求的插话音频不在当前已校验目录中",
        ));
    }
    Ok(canonical_path)
}

fn webview_interlude_original_fallback(
    path: &Path,
    ambient_sound_source: Option<String>,
    reason_code: impl Into<String>,
    reason: impl Into<String>,
) -> PrepareWebViewInterludeResultDto {
    PrepareWebViewInterludeResultDto {
        state: "original_fallback".to_owned(),
        path: path.display().to_string(),
        output_size_bytes: None,
        ambient_sound_source,
        reason_code: Some(reason_code.into()),
        reason: Some(reason.into()),
    }
}

fn ambient_sound_source_label(source: AmbientSoundSource) -> String {
    match source {
        AmbientSoundSource::User => "user".to_owned(),
        AmbientSoundSource::Bundled => "bundled".to_owned(),
    }
}

#[tauri::command]
pub fn set_interlude_config(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: SetInterludeConfigRequestDto,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let (_, snapshot) = prepare_interlude_snapshot(InterludeConfig {
        enabled: request.enabled,
        directory: request.directory,
        audio_selection_mode: request.audio_selection_mode,
        audio_fixed_preset_id: request.audio_fixed_preset_id,
        audio_preset_ids: request.audio_preset_ids,
        audio_mix_enabled: request.audio_mix_enabled,
        audio_mix_pick_min: request.audio_mix_pick_min,
        audio_mix_pick_max: request.audio_mix_pick_max,
        audio_variation_mode: request.audio_variation_mode,
        audio_variation_period_min_ms: request.audio_variation_period_min_ms,
        audio_variation_period_max_ms: request.audio_variation_period_max_ms,
        interval_min_ms: request.interval_min_ms,
        interval_max_ms: request.interval_max_ms,
        volume_db: request.volume_db,
        ducking_depth_db: request.ducking_depth_db,
        ducking_attack_ms: request.ducking_attack_ms,
        ducking_release_ms: request.ducking_release_ms,
    })
    .map_err(command_error_from_interlude)?;
    for audio_path in &snapshot.audio_files {
        allow_local_playback_asset_file(
            &app,
            Path::new(audio_path),
            "interlude_asset_scope_failed",
            "插话音频",
        )?;
    }
    let _prepare_guard = state.interlude_prepare_lock.lock().map_err(|_| {
        CommandErrorDto::new(
            "interlude_prepare_lock_failed",
            "PortAudio 插话创建锁已损坏",
        )
    })?;
    state.ensure_running()?;
    state.with_playback(&window, |playback| {
        playback.set_interlude_snapshot(snapshot.clone());
        Ok(state.snapshot(playback))
    })
}

#[tauri::command]
pub async fn start_portaudio_interlude(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: StartPortAudioInterludeRequestDto,
) -> Result<StartPortAudioInterludeResultDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        start_portaudio_interlude_blocking(app, state, request)
    })
    .await
    .map_err(|error| {
        CommandErrorDto::new(
            "interlude_start_task_failed",
            format!("PortAudio 插话后台启动任务失败：{error}"),
        )
    })?
}

#[tauri::command]
pub async fn prepare_webview_interlude(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: StartPortAudioInterludeRequestDto,
) -> Result<PrepareWebViewInterludeResultDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        prepare_webview_interlude_blocking(app, state, request)
    })
    .await
    .map_err(|error| {
        CommandErrorDto::new(
            "webview_interlude_task_failed",
            format!("WebView 插话后台处理任务失败：{error}"),
        )
    })?
}

#[tauri::command]
pub fn release_webview_interlude_cache(
    window: Window,
    state: State<'_, AppState>,
    request: ReleaseWebViewInterludeCacheRequestDto,
) -> Result<ReleaseWebViewInterludeCacheResultDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let path = request.path.trim();
    if path.is_empty() || path.len() > MAX_SOURCE_MEDIA_PATH_BYTES {
        return Err(CommandErrorDto::new(
            "webview_interlude_cache_path_invalid",
            "WebView 插话缓存路径为空或超过允许长度",
        ));
    }
    state.release_webview_interlude_path(path)
}

fn prepare_webview_interlude_blocking(
    app: AppHandle,
    state: AppState,
    request: StartPortAudioInterludeRequestDto,
) -> Result<PrepareWebViewInterludeResultDto, CommandErrorDto> {
    let _cache_guard = state.webview_interlude_cache_lock.lock().map_err(|_| {
        CommandErrorDto::new(
            "webview_interlude_cache_lock_failed",
            "WebView 插话缓存锁已损坏",
        )
    })?;
    state.ensure_running()?;
    validate_interlude_processing_request(&request)?;
    let timeout_seconds = request.timeout_seconds.unwrap_or(120);
    if !(1..=600).contains(&timeout_seconds) {
        return Err(CommandErrorDto::new(
            "webview_interlude_timeout_invalid",
            "WebView 插话处理超时必须在 1–600 秒之间",
        ));
    }
    let snapshot = state
        .playback
        .lock()
        .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
        .snapshot();
    let canonical_path = validate_interlude_audio_path(&snapshot, &request.path)?;
    let target_root = runtime_resource_target_root(&app)
        .map_err(|error| CommandErrorDto::new("media_resource_dir_failed", error))?;
    let (ffmpeg_path, _) = configured_media_engine_paths_with_resource_dir(&target_root)
        .map_err(|error| CommandErrorDto::new("media_engine_unavailable", error.to_string()))?;
    let ambient_sound = match resolve_active_ambient_sound(
        &app,
        &ffmpeg_path,
        state.ambient_sound_selection()?.as_ref(),
        audio_mix_requires_ambient_sound(&request.audio, &request.audio_variants),
    ) {
        Ok(selection) => selection,
        Err(error) => {
            return Ok(webview_interlude_original_fallback(
                &canonical_path,
                None,
                error.code,
                error.message,
            ))
        }
    };
    let ambient_sound_source = ambient_sound
        .as_ref()
        .map(|selection| ambient_sound_source_label(selection.source));
    let filter_plan = match build_audio_stream_filter_graph_with_ambient(
        &request.audio,
        &request.audio_variants,
        None,
        autolive_portaudio_output::DEFAULT_SAMPLE_RATE_HZ,
        ambient_sound.is_some(),
    ) {
        Ok(plan) => plan,
        Err(error) => {
            return Ok(webview_interlude_original_fallback(
                &canonical_path,
                ambient_sound_source,
                "webview_interlude_filter_invalid",
                format!("插话参数处理失败，已回退原声：{error}"),
            ))
        }
    };
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| CommandErrorDto::new("cache_dir_failed", error.to_string()))?
        .join("webview-interlude");
    let result = match render_webview_interlude_cache(WebViewInterludeCacheRequest {
        ffmpeg_path,
        source_path: canonical_path.clone(),
        ambient_source_path: ambient_sound.map(|selection| selection.path),
        filter_graph: filter_plan.filter_graph,
        quality_pitch: filter_plan.quality_pitch,
        pcm_effects: filter_plan.pcm_effects,
        sample_rate_hz: request.audio.sample_rate_hz.unwrap_or(48_000),
        output_bitrate_kbps: request.audio.output_bitrate_kbps,
        cache_dir,
        timeout: Duration::from_secs(timeout_seconds),
    }) {
        Ok(result) => result,
        Err(error) => {
            return Ok(webview_interlude_original_fallback(
                &canonical_path,
                ambient_sound_source,
                "webview_interlude_processing_failed",
                format!("插话处理失败，已回退原声：{error}"),
            ))
        }
    };
    if let Err(error) = state.ensure_running() {
        let _ = std::fs::remove_file(&result.output_path);
        return Err(error);
    }
    if let Err(error) = allow_local_playback_asset_file(
        &app,
        &result.output_path,
        "webview_interlude_asset_scope_failed",
        "WebView 插话缓存",
    ) {
        let _ = std::fs::remove_file(&result.output_path);
        return Ok(webview_interlude_original_fallback(
            &canonical_path,
            ambient_sound_source,
            error.code,
            format!("处理缓存无法授权播放，已回退原声：{}", error.message),
        ));
    }
    if let Err(error) = state.protect_webview_interlude_path(result.output_path.clone()) {
        let _ = std::fs::remove_file(&result.output_path);
        return Ok(webview_interlude_original_fallback(
            &canonical_path,
            ambient_sound_source,
            error.code,
            format!("处理缓存无法登记播放保护，已回退原声：{}", error.message),
        ));
    }
    Ok(PrepareWebViewInterludeResultDto {
        state: "processed".to_owned(),
        path: result.output_path.display().to_string(),
        output_size_bytes: Some(result.output_size_bytes),
        ambient_sound_source,
        reason_code: None,
        reason: None,
    })
}

fn start_portaudio_interlude_blocking(
    app: AppHandle,
    state: AppState,
    request: StartPortAudioInterludeRequestDto,
) -> Result<StartPortAudioInterludeResultDto, CommandErrorDto> {
    let operation_token = state
        .interlude_prepare_token
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    let _prepare_guard = state.interlude_prepare_lock.lock().map_err(|_| {
        CommandErrorDto::new(
            "interlude_prepare_lock_failed",
            "PortAudio 插话创建锁已损坏",
        )
    })?;
    state.ensure_running()?;
    ensure_interlude_operation_current(&state, operation_token)?;
    validate_interlude_processing_request(&request)?;
    let snapshot = state
        .playback
        .lock()
        .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
        .snapshot();
    let canonical_path = validate_interlude_audio_path(&snapshot, &request.path)?;
    let interlude = &snapshot.interlude;

    let output_control = state.audio_output_control()?;
    let output_status = output_control
        .status()
        .filter(|status| {
            status.health.application_running
                && matches!(
                    status.health.hardware_state,
                    autolive_portaudio_output::PortAudioHardwareState::Active
                )
                && !status.callback_paused
        })
        .ok_or_else(|| {
            CommandErrorDto::new(
                "interlude_portaudio_inactive",
                "PortAudio 实际出口未处于活动状态，随机插话保持 WebView 回退",
            )
        })?;

    let target_root = runtime_resource_target_root(&app)
        .map_err(|error| CommandErrorDto::new("media_resource_dir_failed", error))?;
    let (ffmpeg_path, _) = configured_media_engine_paths_with_resource_dir(&target_root)
        .map_err(|error| CommandErrorDto::new("media_engine_unavailable", error.to_string()))?;
    let ambient_sound = resolve_active_ambient_sound(
        &app,
        &ffmpeg_path,
        state.ambient_sound_selection()?.as_ref(),
        audio_mix_requires_ambient_sound(&request.audio, &request.audio_variants),
    )?;
    let ambient_sound_path = ambient_sound.map(|selection| selection.path);
    let filter_plan = build_audio_stream_filter_graph_with_ambient(
        &request.audio,
        &request.audio_variants,
        None,
        output_status.sample_rate_hz,
        ambient_sound_path.is_some(),
    )
    .map_err(|error| CommandErrorDto::new("interlude_audio_filter_invalid", error.to_string()))?;

    state.stop_interlude_mixer_current()?;
    let minimum_ready_ms = usize::try_from(output_status.playback_watermark_ms)
        .unwrap_or(AUDIO_CANDIDATE_COMMIT_TAIL_MS)
        .max(AUDIO_CANDIDATE_COMMIT_TAIL_MS);
    // 插话是立即叠加，不是远期候选；只预热硬件水位即可，避免固定等待 1 秒。
    let buffer_ms = minimum_ready_ms;
    let started_at = Instant::now();
    let mut task = AudioMixerTask::start_finite_scheduled_candidate_with_filter_and_variant_count(
        ffmpeg_path,
        canonical_path.clone(),
        output_status.sample_rate_hz,
        0,
        0,
        Some(filter_plan.filter_graph),
        filter_plan.quality_pitch,
        filter_plan.pcm_effects,
        filter_plan
            .requires_ambient_input
            .then_some(ambient_sound_path)
            .flatten(),
        request.audio.playback_speed,
        buffer_ms,
        minimum_ready_ms,
        started_at,
    )
    .map_err(|error| CommandErrorDto::new("interlude_decoder_start_failed", error))?;
    while !task.is_ready() {
        if let Err(error) = ensure_interlude_operation_current(&state, operation_token) {
            task.stop_preserving_output();
            return Err(error);
        }
        if let Err(error) = state.ensure_running() {
            task.stop_preserving_output();
            return Err(error);
        }
        if let Some(error) = task.failure() {
            task.stop_preserving_output();
            return Err(CommandErrorDto::new(
                "interlude_decode_failed",
                format!("插话音频解码失败：{error}"),
            ));
        }
        if started_at.elapsed() >= Duration::from_millis(PORTAUDIO_SWITCH_READY_TIMEOUT_MS) {
            task.stop_preserving_output();
            return Err(CommandErrorDto::new(
                "interlude_decode_timeout",
                "插话音频未在 5000ms 内达到 PortAudio 安全水位",
            ));
        }
        thread::sleep(Duration::from_millis(PORTAUDIO_SWITCH_READY_POLL_MS));
    }
    if let Err(error) = state.ensure_running() {
        task.stop_preserving_output();
        return Err(error);
    }
    ensure_interlude_operation_current(&state, operation_token)?;
    let generation = state
        .interlude_session_generation
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    if let Err(error) = output_control.start_interlude(
        task.output_track(),
        AudioInterludeMixConfig {
            volume_gain: db_to_linear_gain(interlude.volume_db, 4.0),
            duck_gain: db_to_linear_gain(interlude.ducking_depth_db, 1.0),
            attack_ms: interlude.ducking_attack_ms,
            release_ms: interlude.ducking_release_ms,
        },
    ) {
        task.stop_preserving_output();
        return Err(CommandErrorDto::new("interlude_output_start_failed", error));
    }
    if let Err(error) = ensure_interlude_operation_current(&state, operation_token) {
        let _ = output_control.stop_interlude();
        task.stop_preserving_output();
        return Err(error);
    }
    let replaced = match state.interlude_mixer.lock() {
        Ok(mut current) => current.replace(ActiveInterludeMixer {
            generation,
            source_path: canonical_path,
            task,
        }),
        Err(_) => {
            let _ = output_control.stop_interlude();
            return Err(CommandErrorDto::new(
                "interlude_mixer_lock_failed",
                "PortAudio 插话混音状态锁已损坏",
            ));
        }
    };
    if let Some(mut replaced) = replaced {
        replaced.task.stop_preserving_output();
    }
    Ok(StartPortAudioInterludeResultDto { generation })
}

fn ensure_interlude_operation_current(
    state: &AppState,
    operation_token: u64,
) -> Result<(), CommandErrorDto> {
    if state.interlude_prepare_token.load(Ordering::Acquire) == operation_token {
        Ok(())
    } else {
        Err(CommandErrorDto::new(
            "interlude_operation_stale",
            "插话操作已被更新的开始、切换或停止请求取消",
        ))
    }
}

fn db_to_linear_gain(db: f64, maximum: f32) -> f32 {
    if !db.is_finite() {
        return 0.0;
    }
    if db <= -60.0 {
        return 0.0;
    }
    (10_f64.powf(db / 20.0) as f32).clamp(0.0, maximum)
}

fn validate_portaudio_media_gain(
    request: &SetPortAudioMediaVolumeRequestDto,
) -> Result<f32, CommandErrorDto> {
    if !request.volume.is_finite() || !(0.0..=1.0).contains(&request.volume) {
        return Err(CommandErrorDto::new(
            "portaudio_media_volume_invalid",
            "PortAudio 主媒体音量必须是 0–1 的有限数值",
        ));
    }
    Ok(if request.muted {
        0.0
    } else {
        request.volume as f32
    })
}

fn validate_portaudio_interlude_gain(volume_db: f64) -> Result<f32, CommandErrorDto> {
    if !volume_db.is_finite() || !(-60.0..=12.0).contains(&volume_db) {
        return Err(CommandErrorDto::new(
            "portaudio_interlude_volume_invalid",
            "PortAudio 插话音量必须是 -60–12dB 的有限数值",
        ));
    }
    Ok(db_to_linear_gain(volume_db, 4.0))
}

#[tauri::command]
pub fn set_portaudio_media_volume(
    window: Window,
    state: State<'_, AppState>,
    request: SetPortAudioMediaVolumeRequestDto,
) -> Result<(), CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let gain = validate_portaudio_media_gain(&request)?;
    state
        .audio_output_control()?
        .set_media_gain(gain)
        .map_err(|error| CommandErrorDto::new("portaudio_media_volume_update_failed", error))
}

#[tauri::command]
pub fn set_portaudio_interlude_volume(
    window: Window,
    state: State<'_, AppState>,
    request: SetPortAudioInterludeVolumeRequestDto,
) -> Result<(), CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let gain = validate_portaudio_interlude_gain(request.volume_db)?;
    state
        .audio_output_control()?
        .set_interlude_gain(gain)
        .map_err(|error| CommandErrorDto::new("portaudio_interlude_volume_update_failed", error))
}

#[tauri::command]
pub async fn switch_portaudio_interlude_preset(
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
    request: SwitchPortAudioInterludePresetRequestDto,
) -> Result<SwitchPortAudioInterludePresetResultDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        switch_portaudio_interlude_preset_blocking(app, state, request)
    })
    .await
    .map_err(|error| {
        CommandErrorDto::new(
            "interlude_switch_task_failed",
            format!("PortAudio 插话声音预设切换任务失败：{error}"),
        )
    })?
}

fn switch_portaudio_interlude_preset_blocking(
    app: AppHandle,
    state: AppState,
    request: SwitchPortAudioInterludePresetRequestDto,
) -> Result<SwitchPortAudioInterludePresetResultDto, CommandErrorDto> {
    if request.generation == 0 {
        return Err(CommandErrorDto::new(
            "interlude_generation_invalid",
            "插话会话代际必须大于 0",
        ));
    }
    if request.media_position_ms > MAX_INTERLUDE_MEDIA_POSITION_MS {
        return Err(CommandErrorDto::new(
            "interlude_media_position_invalid",
            "插话媒体位置超过允许的 24 小时上限",
        ));
    }
    validate_interlude_audio_params(&request.audio, &request.audio_variants)?;
    let _prepare_guard = state.interlude_prepare_lock.lock().map_err(|_| {
        CommandErrorDto::new(
            "interlude_prepare_lock_failed",
            "PortAudio 插话创建锁已损坏",
        )
    })?;
    state.ensure_running()?;
    let operation_token = state
        .interlude_prepare_token
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    let source_path = {
        let current = state.interlude_mixer.lock().map_err(|_| {
            CommandErrorDto::new(
                "interlude_mixer_lock_failed",
                "PortAudio 插话混音状态锁已损坏",
            )
        })?;
        let active = current.as_ref().ok_or_else(|| {
            CommandErrorDto::new("interlude_not_active", "当前没有活动的 PortAudio 插话")
        })?;
        if active.generation != request.generation {
            return Err(CommandErrorDto::new(
                "interlude_generation_stale",
                "插话会话代际已过期，保留当前插话预设",
            ));
        }
        active.source_path.clone()
    };
    let snapshot = state
        .playback
        .lock()
        .map_err(|_| CommandErrorDto::new("playback_lock_failed", "播放状态锁已损坏"))?
        .snapshot();
    let source_path_text = source_path.to_string_lossy();
    let canonical_path = validate_interlude_audio_path(&snapshot, &source_path_text)?;
    let output_control = state.audio_output_control()?;
    let output_status = output_control
        .status()
        .filter(|status| {
            status.health.application_running
                && matches!(
                    status.health.hardware_state,
                    autolive_portaudio_output::PortAudioHardwareState::Active
                )
                && !status.callback_paused
        })
        .ok_or_else(|| {
            CommandErrorDto::new(
                "interlude_portaudio_inactive",
                "PortAudio 实际出口未处于活动状态，保留当前插话预设",
            )
        })?;
    let target_root = runtime_resource_target_root(&app)
        .map_err(|error| CommandErrorDto::new("media_resource_dir_failed", error))?;
    let (ffmpeg_path, _) = configured_media_engine_paths_with_resource_dir(&target_root)
        .map_err(|error| CommandErrorDto::new("media_engine_unavailable", error.to_string()))?;
    let ambient_sound = resolve_active_ambient_sound(
        &app,
        &ffmpeg_path,
        state.ambient_sound_selection()?.as_ref(),
        audio_mix_requires_ambient_sound(&request.audio, &request.audio_variants),
    )?;
    let ambient_sound_path = ambient_sound.map(|selection| selection.path);
    let filter_plan = build_audio_stream_filter_graph_with_ambient(
        &request.audio,
        &request.audio_variants,
        None,
        output_status.sample_rate_hz,
        ambient_sound_path.is_some(),
    )
    .map_err(|error| CommandErrorDto::new("interlude_audio_filter_invalid", error.to_string()))?;
    let minimum_ready_ms = usize::try_from(output_status.playback_watermark_ms)
        .unwrap_or(AUDIO_CANDIDATE_COMMIT_TAIL_MS)
        .max(AUDIO_CANDIDATE_COMMIT_TAIL_MS);
    let started_at = Instant::now();
    let mut pending =
        AudioMixerTask::start_finite_scheduled_candidate_with_filter_and_variant_count(
            ffmpeg_path,
            canonical_path,
            output_status.sample_rate_hz,
            request.media_position_ms,
            request.media_position_ms,
            Some(filter_plan.filter_graph),
            filter_plan.quality_pitch,
            filter_plan.pcm_effects,
            filter_plan
                .requires_ambient_input
                .then_some(ambient_sound_path)
                .flatten(),
            request.audio.playback_speed,
            minimum_ready_ms,
            minimum_ready_ms,
            started_at,
        )
        .map_err(|error| CommandErrorDto::new("interlude_decoder_start_failed", error))?;
    while !pending.is_ready() {
        if let Err(error) = ensure_interlude_operation_current(&state, operation_token) {
            pending.stop_preserving_output();
            return Err(error);
        }
        if let Err(error) = state.ensure_running() {
            pending.stop_preserving_output();
            return Err(error);
        }
        if let Some(error) = pending.failure() {
            pending.stop_preserving_output();
            return Err(CommandErrorDto::new(
                "interlude_decode_failed",
                format!("插话候选解码失败，保留当前预设：{error}"),
            ));
        }
        if started_at.elapsed() >= Duration::from_millis(PORTAUDIO_SWITCH_READY_TIMEOUT_MS) {
            pending.stop_preserving_output();
            return Err(CommandErrorDto::new(
                "interlude_decode_timeout",
                "插话候选未在 5000ms 内达到安全水位，保留当前预设",
            ));
        }
        thread::sleep(Duration::from_millis(PORTAUDIO_SWITCH_READY_POLL_MS));
    }
    ensure_interlude_operation_current(&state, operation_token)?;
    let generation_is_current = {
        let current = state.interlude_mixer.lock().map_err(|_| {
            CommandErrorDto::new(
                "interlude_mixer_lock_failed",
                "PortAudio 插话混音状态锁已损坏",
            )
        })?;
        current
            .as_ref()
            .is_some_and(|active| active.generation == request.generation)
    };
    if !generation_is_current {
        pending.stop_preserving_output();
        return Err(CommandErrorDto::new(
            "interlude_generation_stale",
            "插话会话代际已过期，保留当前插话预设",
        ));
    }
    output_control
        .crossfade_interlude_to(pending.output_track())
        .map_err(|error| {
            pending.stop_preserving_output();
            CommandErrorDto::new("interlude_output_switch_failed", error)
        })?;
    if let Err(error) = ensure_interlude_operation_current(&state, operation_token) {
        let _ = output_control.stop_interlude();
        pending.stop_preserving_output();
        return Err(error);
    }
    let replaced = {
        let mut current = state.interlude_mixer.lock().map_err(|_| {
            CommandErrorDto::new(
                "interlude_mixer_lock_failed",
                "PortAudio 插话混音状态锁已损坏",
            )
        })?;
        match current.as_mut() {
            Some(active) if active.generation == request.generation => {
                std::mem::swap(&mut active.task, &mut pending);
                true
            }
            _ => false,
        }
    };
    if !replaced {
        pending.stop_preserving_output();
        return Err(CommandErrorDto::new(
            "interlude_generation_stale",
            "插话在提交候选前已经停止或切换，候选已释放",
        ));
    }
    pending.stop_preserving_output();
    Ok(SwitchPortAudioInterludePresetResultDto {
        generation: request.generation,
        media_position_ms: request.media_position_ms,
    })
}

#[tauri::command]
pub fn pause_portaudio_interlude(
    window: Window,
    state: State<'_, AppState>,
) -> Result<(), CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    state.interlude_prepare_token.fetch_add(1, Ordering::AcqRel);
    state
        .audio_output_control()?
        .pause_interlude()
        .map_err(|error| CommandErrorDto::new("interlude_output_pause_failed", error))
}

#[tauri::command]
pub fn resume_portaudio_interlude(
    window: Window,
    state: State<'_, AppState>,
) -> Result<(), CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    state
        .audio_output_control()?
        .resume_interlude()
        .map_err(|error| CommandErrorDto::new("interlude_output_resume_failed", error))
}

#[tauri::command]
pub fn stop_portaudio_interlude(
    window: Window,
    state: State<'_, AppState>,
) -> Result<(), CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    state.interlude_prepare_token.fetch_add(1, Ordering::AcqRel);
    state
        .interlude_session_generation
        .fetch_add(1, Ordering::AcqRel);
    if let Ok(control) = state.audio_output_control() {
        let _ = control.stop_interlude();
    }
    let _prepare_guard = state.interlude_prepare_lock.lock().map_err(|_| {
        CommandErrorDto::new(
            "interlude_prepare_lock_failed",
            "PortAudio 插话创建锁已损坏",
        )
    })?;
    state.stop_interlude_mixer_current()
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
    if let Err(task) = state.install_speech_worker(BackgroundWorkerTask {
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
pub fn restore_original_video(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PlaybackSnapshotDto, CommandErrorDto> {
    state.ensure_playback_window(&window)?;
    state.stop_realtime_video_runtime()?;
    state.with_playback_window(&window, |playback| {
        playback.restore_original_video();
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
    // 轮询时分别回收已完成 Worker，避免任一领域一直停在 processing。
    let _ = state.reap_finished_video_media_worker()?;
    let _ = state.reap_finished_audio_media_worker()?;
    let video_worker_running = state.video_media_worker_is_running()?;
    let audio_worker_running = state.audio_media_worker_is_running()?;
    let clear_stale_processing = |playback: &mut PlaybackCore| {
        let snapshot = playback.snapshot();
        if !video_worker_running && snapshot.video_processing_status == "processing" {
            playback.mark_media_processing_failed(
                "视频处理 Worker 已结束但状态未更新，已恢复可用（请重新应用）",
            );
        }
        if !audio_worker_running && snapshot.audio_processing_status == "processing" {
            playback.mark_audio_media_processing_failed(
                "声音处理 Worker 已结束但状态未更新，已恢复可用（请重新应用）",
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
pub fn validate_media_effect_params(
    window: Window,
    state: State<'_, AppState>,
    request: MediaEffectParams,
) -> Result<MediaParameterValidationResultDto, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    let errors = request.validate().err().unwrap_or_default();
    Ok(MediaParameterValidationResultDto {
        valid: errors.is_empty(),
        errors,
    })
}

#[tauri::command]
pub fn get_default_media_effect_params(
    window: Window,
    state: State<'_, AppState>,
) -> Result<MediaEffectParams, CommandErrorDto> {
    state.ensure_main_window(&window)?;
    Ok(MediaEffectParams::default())
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
        PlaybackError::SourceMediaPoolEmpty => "source_media_pool_empty",
        PlaybackError::SourceMediaPoolTooLarge { .. } => "source_media_pool_too_large",
        PlaybackError::SourceMediaPoolIndexOutOfBounds { .. } => {
            "source_media_pool_index_out_of_bounds"
        }
        PlaybackError::SourceMediaPathEmpty { .. } => "empty_source_media_path",
        PlaybackError::DuplicateSourceMediaPath { .. } => "duplicate_source_media_path",
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
        InterludeError::AudioPresetCountOutOfRange => "interlude_audio_preset_count_invalid",
        InterludeError::AudioPresetInvalid => "interlude_audio_preset_invalid",
        InterludeError::AudioPresetDuplicate => "interlude_audio_preset_duplicate",
        InterludeError::AudioMixPickMinOutOfRange
        | InterludeError::AudioMixPickMaxOutOfRange
        | InterludeError::AudioMixPickOrderInvalid => "interlude_audio_mix_invalid",
        InterludeError::AudioVariationPeriodMinOutOfRange
        | InterludeError::AudioVariationPeriodMaxOutOfRange
        | InterludeError::AudioVariationPeriodOrderInvalid => {
            "interlude_audio_variation_period_invalid"
        }
        InterludeError::MissingDirectory => "interlude_directory_required",
        InterludeError::DirectoryUnavailable(_) | InterludeError::DirectoryReadFailed(_) => {
            "interlude_directory_invalid"
        }
        InterludeError::NoUsableAudioFiles => "interlude_audio_files_empty",
        InterludeError::TooManyAudioFiles { .. } => "interlude_audio_files_too_many",
        InterludeError::PathTooLong { .. } => "interlude_path_too_long",
        InterludeError::CatalogPathBytesExceeded { .. } => "interlude_catalog_paths_too_large",
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
        ambient_sound_source_label, apply_source_media_order, audio_crossfade_error_code,
        audio_cycle_cancel_matches_pending, audio_cycle_commit_due, audio_cycle_crossfade_failure,
        audio_cycle_target_is_within_horizon, audio_cycle_target_validation_code,
        audio_mix_requires_ambient_sound, audio_sync_clock_rejection_code,
        compile_gpu83_realtime_parameters, db_to_linear_gain, ensure_interlude_operation_current,
        insert_canonical_source_path, is_retryable_audio_mixer_error, join_background_worker_until,
        lock_current_media_import, mark_audio_resume_failure, reorder_source_media_pool,
        resolve_audio_candidate_pcm_position_ms, resolve_audio_commit_position_ms,
        resolve_audio_output_latency_ms, resolve_audio_sync_clock,
        scheduled_candidate_commit_tail_ms, should_complete_playback_item,
        should_complete_playback_loop, should_defer_source_sync_for_pending_candidate,
        signed_millis_delta, source_media_index_by_path, take_pending_audio_mixer,
        validate_audio_candidate_source_window, validate_audio_sync_clock,
        validate_portaudio_interlude_gain, validate_portaudio_media_gain,
        validate_source_media_paths, validate_source_media_pool_count,
        webview_interlude_original_fallback, AmbientSoundSource, AppState, AudioSyncClock,
        BackgroundWorkerTask, CommandErrorDto, PlaybackSnapshotDto,
        SetPortAudioMediaVolumeRequestDto, AUDIO_CANDIDATE_OUTSIDE_SOURCE_AUDIO_WINDOW_CODE,
        AUDIO_SYNC_CLOCK_EXPIRED, MAX_SOURCE_MEDIA_PATH_BYTES,
    };
    use autolive_desktop_core::cancellation::CancellationToken;
    use autolive_desktop_core::media_effect_params::{AudioEffectParams, MediaEffectParams};
    use autolive_desktop_core::media_library::{MediaCompatibilityMode, MediaKind, SourceMediaDto};
    use autolive_desktop_core::realtime_video_backend::MpvCommand;
    use autolive_desktop_core::runtime_resource_task::RuntimeResourceTaskShutdown;
    use autolive_desktop_core::PlaybackCore;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn playback_test_source(file_name: &str) -> SourceMediaDto {
        SourceMediaDto {
            source_path: format!("C:/videos/{file_name}"),
            playback_reference: format!("C:/videos/{file_name}"),
            media_kind: MediaKind::Video,
            compatibility_mode: MediaCompatibilityMode::Direct,
            file_name: file_name.to_owned(),
            file_size_bytes: 1,
            duration_ms: Some(1_000),
            width: Some(1280),
            height: Some(720),
            frame_rate_fps: Some(30.0),
            audio_start_ms: Some(0),
            audio_end_ms: Some(1_000),
            audio_sample_rate_hz: Some(48_000),
            audio_channel_count: Some(2),
            video_codec_name: Some("h264".to_owned()),
            audio_codec_name: Some("aac".to_owned()),
            mp4_sha256: None,
            mp4_hash_status: "disabled".to_owned(),
        }
    }

    #[test]
    fn gpu83_cycle_compiles_to_one_atomic_shader_options_command() {
        let compiled = compile_gpu83_realtime_parameters(&MediaEffectParams::default())
            .expect("default GPU83 snapshot");
        assert_eq!(compiled.commands.len(), 1);
        assert!(matches!(
            &compiled.commands[0],
            MpvCommand::SetShaderOptions { options }
                if options.as_str().contains("al_brightness_percent=0")
        ));
        assert!(compiled.support.fully_supported);

        let mut unavailable = MediaEffectParams::default();
        unavailable.video.frame_rate_jitter_percent = 1.0;
        let compiled = compile_gpu83_realtime_parameters(&unavailable)
            .expect("valid but not-yet-mapped GPU83 snapshot");
        assert_eq!(compiled.commands.len(), 1);
        assert!(!compiled.support.fully_supported);
        assert!(compiled.support.parameters.iter().any(|parameter| {
            parameter.field == "video.frame_rate_jitter_percent"
                && parameter.active
                && !parameter.supported
        }));
    }

    #[test]
    fn audio_candidate_source_window_accepts_unknown_metadata() {
        assert!(validate_audio_candidate_source_window(
            Some(60_000),
            None,
            None,
            55_000,
            5_000,
            false,
        )
        .is_ok());
    }

    #[test]
    fn audio_candidate_source_window_rejects_disjoint_multi_item_tail() {
        let error = validate_audio_candidate_source_window(
            Some(44_468),
            Some(0),
            Some(44_102),
            44_200,
            268,
            false,
        )
        .expect_err("the container-only tail must not start an audio worker");

        assert_eq!(error.code, AUDIO_CANDIDATE_OUTSIDE_SOURCE_AUDIO_WINDOW_CODE);
    }

    #[test]
    fn audio_candidate_source_window_accepts_partial_multi_item_overlap() {
        assert!(validate_audio_candidate_source_window(
            Some(44_468),
            Some(1_000),
            Some(44_102),
            43_800,
            668,
            false,
        )
        .is_ok());
    }

    #[test]
    fn audio_candidate_source_window_accepts_single_item_wrap_into_next_audio_window() {
        assert!(validate_audio_candidate_source_window(
            Some(44_468),
            Some(0),
            Some(44_102),
            44_200,
            1_000,
            true,
        )
        .is_ok());
    }

    #[test]
    fn audio_candidate_source_window_rejects_single_item_gap_without_wrap_overlap() {
        let error = validate_audio_candidate_source_window(
            Some(60_000),
            Some(10_000),
            Some(20_000),
            30_000,
            5_000,
            true,
        )
        .expect_err("a looped request can still be wholly inside the source audio gap");

        assert_eq!(error.code, AUDIO_CANDIDATE_OUTSIDE_SOURCE_AUDIO_WINDOW_CODE);
    }

    #[test]
    fn audio_candidate_source_window_treats_touching_boundaries_as_disjoint() {
        assert!(validate_audio_candidate_source_window(
            Some(60_000),
            Some(10_000),
            Some(20_000),
            20_000,
            5_000,
            false,
        )
        .is_err());
        assert!(validate_audio_candidate_source_window(
            Some(60_000),
            Some(10_000),
            Some(20_000),
            5_000,
            5_000,
            false,
        )
        .is_err());
    }

    #[test]
    fn ambient_sound_is_required_when_any_audio_variant_uses_it() {
        let base = AudioEffectParams::default();
        let mut variant = base.clone();
        variant.ambient_sound_mix_percent = 0.1;

        assert!(!audio_mix_requires_ambient_sound(&base, &[]));
        assert!(audio_mix_requires_ambient_sound(&base, &[variant]));
    }

    #[test]
    fn superseded_media_import_cannot_acquire_commit_ownership() {
        let state = AppState::default();
        let first = state
            .begin_media_import()
            .expect("first import should start");
        let second = state
            .begin_media_import()
            .expect("second import should supersede the first");

        assert!(first.cancellation.is_cancelled());
        assert!(lock_current_media_import(&state, &first).is_err());
        assert!(lock_current_media_import(&state, &second).is_ok());
    }

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

    fn cancellable_background_worker() -> BackgroundWorkerTask {
        let cancellation = CancellationToken::new();
        let cancellation_for_thread = cancellation.clone();
        let completed = Arc::new(AtomicBool::new(false));
        let completed_for_thread = Arc::clone(&completed);
        let handle = std::thread::spawn(move || {
            while !cancellation_for_thread.is_cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
            completed_for_thread.store(true, Ordering::Release);
        });
        BackgroundWorkerTask {
            cancellation,
            completed,
            handle,
        }
    }

    fn release_controlled_background_worker() -> (
        BackgroundWorkerTask,
        CancellationToken,
        std::sync::mpsc::Sender<()>,
    ) {
        let cancellation = CancellationToken::new();
        let cancellation_observer = cancellation.clone();
        let completed = Arc::new(AtomicBool::new(false));
        let completed_for_thread = Arc::clone(&completed);
        let (release, released) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let _ = released.recv();
            completed_for_thread.store(true, Ordering::Release);
        });
        (
            BackgroundWorkerTask {
                cancellation,
                completed,
                handle,
            },
            cancellation_observer,
            release,
        )
    }

    #[test]
    fn media_worker_stop_timeout_keeps_the_slot_owned_and_rejects_replacement() {
        let state = AppState::default();
        let (worker, cancellation, release) = release_controlled_background_worker();
        state
            .install_video_media_worker(worker)
            .expect("video worker should be installed");

        let started_at = Instant::now();
        let error = state
            .stop_media_worker_slot(
                &state.video_media_worker,
                "video_media_worker_lock_failed",
                "视频 Worker 状态锁已损坏",
                "视频 Worker",
                Duration::from_millis(20),
            )
            .expect_err("a worker that ignores cancellation should time out");

        assert_eq!(error.code, "media_worker_stop_timeout");
        assert!(started_at.elapsed() < Duration::from_secs(1));
        assert!(cancellation.is_cancelled());
        assert!(state.video_media_worker.lock().unwrap().is_some());

        let replacement = cancellable_background_worker();
        let replacement = state
            .install_video_media_worker(replacement)
            .expect_err("timed out worker must keep the domain slot occupied");
        replacement.cancellation.cancel();
        replacement
            .handle
            .join()
            .expect("rejected replacement should stop");

        release.send(()).expect("blocked worker should be released");
        assert_eq!(
            join_background_worker_until(
                &state.video_media_worker,
                "视频 Worker 状态锁已损坏",
                Instant::now(),
                Duration::from_secs(1),
            )
            .expect("released worker should join"),
            RuntimeResourceTaskShutdown::Joined
        );
    }

    #[test]
    fn app_shutdown_cancels_and_joins_every_background_worker() {
        let state = AppState::default();
        state
            .protect_webview_interlude_path(PathBuf::from("C:/cache/interlude-active.m4a"))
            .expect("cache protection should be registered");
        state
            .install_speech_worker(cancellable_background_worker())
            .expect("speech worker should be installed");
        state
            .install_video_media_worker(cancellable_background_worker())
            .expect("video worker should be installed");
        state
            .install_audio_media_worker(cancellable_background_worker())
            .expect("audio worker should be installed");

        let shutdown = state
            .shutdown_all(Duration::from_secs(1))
            .expect("shutdown should succeed");

        assert_eq!(shutdown, RuntimeResourceTaskShutdown::Joined);
        assert!(state
            .protected_webview_interlude_paths()
            .expect("cache protections should be readable")
            .is_empty());
        assert!(state.speech_worker.lock().unwrap().is_none());
        assert!(state.video_media_worker.lock().unwrap().is_none());
        assert!(state.audio_media_worker.lock().unwrap().is_none());
        assert_eq!(
            state
                .begin_audio_mixer_prepare()
                .expect_err("shutdown gate must reject a late audio candidate")
                .code,
            "app_shutting_down"
        );

        let late_worker = cancellable_background_worker();
        let late_worker = state
            .install_video_media_worker(late_worker)
            .expect_err("shutdown gate must reject a late video worker");
        late_worker.cancellation.cancel();
        late_worker.handle.join().expect("late worker should stop");

        let late_worker = cancellable_background_worker();
        let late_worker = state
            .install_audio_media_worker(late_worker)
            .expect_err("shutdown gate must reject a late audio worker");
        late_worker.cancellation.cancel();
        late_worker.handle.join().expect("late worker should stop");
    }

    #[test]
    fn playback_snapshot_dto_serializes_committed_audio_stream_configuration() {
        let mut playback = PlaybackCore::default();
        let params = AudioEffectParams {
            input_gain_db: 2.5,
            ..Default::default()
        };
        let variants = vec![AudioEffectParams {
            pitch_shift_semitones: 0.5,
            ..params.clone()
        }];
        playback
            .set_audio_stream_configuration(params, variants)
            .expect("configuration should be valid");

        let serialized = serde_json::to_value(PlaybackSnapshotDto::from(playback.snapshot()))
            .expect("snapshot DTO should serialize");
        assert_eq!(serialized["audio_stream_params"]["input_gain_db"], 2.5);
        assert_eq!(
            serialized["audio_stream_variants"][0]["pitch_shift_semitones"],
            0.5
        );
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
    fn stale_source_sync_clock_reanchors_to_the_current_absolute_media_time() {
        let requested = AudioSyncClock {
            playback_generation: 9,
            loop_index: 1,
            position_ms: 1_000,
            duration_ms: 72_300,
            absolute_position_ms: 73_300,
        };

        assert_eq!(
            resolve_audio_sync_clock(requested, 9, 1, 3_000, 72_300),
            Ok(AudioSyncClock {
                position_ms: 3_000,
                absolute_position_ms: 75_300,
                ..requested
            })
        );
        assert_eq!(
            validate_audio_sync_clock(requested, 9, 1, 3_000, 72_300),
            Err(AUDIO_SYNC_CLOCK_EXPIRED)
        );
        assert_eq!(
            audio_sync_clock_rejection_code(AUDIO_SYNC_CLOCK_EXPIRED),
            Some("audio_mixer_candidate_stale")
        );
    }

    #[test]
    fn sixty_second_candidate_uses_the_live_media_clock_at_double_speed() {
        // 最终窗口在 2x 下已经从 10_000ms 前进了 1_000ms；即使 Rust 最近一次
        // position 快照仍停在 10_000ms，目标相对真实媒体时钟仍恰好是 60 秒。
        assert!(audio_cycle_target_is_within_horizon(
            10_000, 500, 2.0, 71_000,
        ));
        assert!(!audio_cycle_target_is_within_horizon(
            10_000, 500, 2.0, 71_001,
        ));
        assert_eq!(
            audio_cycle_target_validation_code(10_000, 500, 2.0, 11_000),
            Some("audio_candidate_stale")
        );
        assert_eq!(
            audio_cycle_target_validation_code(10_000, 500, 2.0, 11_001),
            None
        );
        assert_eq!(
            audio_cycle_target_validation_code(10_000, 500, 2.0, 71_000),
            None
        );
        assert_eq!(
            audio_cycle_target_validation_code(10_000, 500, 2.0, 71_001),
            Some("audio_candidate_target_invalid")
        );
    }

    #[test]
    fn interlude_decibel_values_are_bounded_for_the_portaudio_bus() {
        assert!((db_to_linear_gain(0.0, 4.0) - 1.0).abs() < f32::EPSILON);
        assert!((db_to_linear_gain(-12.0, 1.0) - 0.251_188_64).abs() < 1e-6);
        assert_eq!(db_to_linear_gain(-60.0, 1.0), 0.0);
        assert_eq!(db_to_linear_gain(12.0, 1.0), 1.0);
        assert_eq!(db_to_linear_gain(f64::NAN, 1.0), 0.0);
    }

    #[test]
    fn portaudio_media_volume_validation_keeps_mute_on_the_main_bus_only() {
        assert_eq!(
            validate_portaudio_media_gain(&SetPortAudioMediaVolumeRequestDto {
                volume: 0.4,
                muted: false,
            })
            .unwrap(),
            0.4
        );
        assert_eq!(
            validate_portaudio_media_gain(&SetPortAudioMediaVolumeRequestDto {
                volume: 0.4,
                muted: true,
            })
            .unwrap(),
            0.0
        );
        for volume in [-0.01, 1.01, f64::NAN] {
            assert_eq!(
                validate_portaudio_media_gain(&SetPortAudioMediaVolumeRequestDto {
                    volume,
                    muted: false,
                })
                .unwrap_err()
                .code,
                "portaudio_media_volume_invalid"
            );
        }
    }

    #[test]
    fn portaudio_interlude_volume_keeps_its_independent_decibel_range() {
        assert_eq!(validate_portaudio_interlude_gain(-60.0).unwrap(), 0.0);
        assert!((validate_portaudio_interlude_gain(12.0).unwrap() - 3.981_071_7).abs() < 1e-6);
        for volume_db in [-60.01, 12.01, f64::INFINITY] {
            assert_eq!(
                validate_portaudio_interlude_gain(volume_db)
                    .unwrap_err()
                    .code,
                "portaudio_interlude_volume_invalid"
            );
        }
    }

    #[test]
    fn newer_interlude_operation_invalidates_stale_candidate_commit() {
        let state = AppState::default();
        let token = state
            .interlude_prepare_token
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        assert!(ensure_interlude_operation_current(&state, token).is_ok());

        state.interlude_prepare_token.fetch_add(1, Ordering::AcqRel);

        assert_eq!(
            ensure_interlude_operation_current(&state, token)
                .unwrap_err()
                .code,
            "interlude_operation_stale"
        );
    }

    #[test]
    fn webview_interlude_failure_is_an_explicit_original_fallback() {
        let fallback = webview_interlude_original_fallback(
            std::path::Path::new("C:/audio/interlude.wav"),
            Some("user".to_owned()),
            "webview_interlude_processing_failed",
            "decode failed",
        );

        assert_eq!(fallback.state, "original_fallback");
        assert!(fallback.path.ends_with("interlude.wav"));
        assert_eq!(fallback.ambient_sound_source.as_deref(), Some("user"));
        assert_eq!(
            fallback.reason_code.as_deref(),
            Some("webview_interlude_processing_failed")
        );
        assert_eq!(fallback.reason.as_deref(), Some("decode failed"));
        assert_eq!(fallback.output_size_bytes, None);
    }

    #[test]
    fn ambient_sound_result_preserves_user_and_bundled_source_labels() {
        assert_eq!(ambient_sound_source_label(AmbientSoundSource::User), "user");
        assert_eq!(
            ambient_sound_source_label(AmbientSoundSource::Bundled),
            "bundled"
        );
    }

    #[test]
    fn webview_interlude_cache_release_is_idempotent() {
        let state = AppState::default();
        let cache_dir = std::env::temp_dir().join(format!(
            "autolive-webview-interlude-release-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("test clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&cache_dir).expect("create release cache fixture");
        let path = cache_dir.join("interlude-active.m4a");
        std::fs::write(&path, b"cache").expect("write registered cache fixture");
        state
            .protect_webview_interlude_path(path.clone())
            .expect("cache protection should be registered");

        let released = state
            .release_webview_interlude_path(path.to_str().expect("UTF-8 fixture path"))
            .expect("first release should succeed");
        assert_eq!(
            released,
            super::ReleaseWebViewInterludeCacheResultDto {
                released: true,
                removed: true,
            }
        );
        assert!(!path.exists());

        let duplicate = state
            .release_webview_interlude_path(path.to_str().expect("UTF-8 fixture path"))
            .expect("duplicate release should be idempotent");
        assert_eq!(
            duplicate,
            super::ReleaseWebViewInterludeCacheResultDto {
                released: false,
                removed: false,
            }
        );

        let forged = cache_dir.join("interlude-forged.m4a");
        std::fs::write(&forged, b"keep").expect("write unregistered cache fixture");
        let forged_release = state
            .release_webview_interlude_path(forged.to_str().expect("UTF-8 fixture path"))
            .expect("unregistered release should be ignored");
        assert_eq!(
            forged_release,
            super::ReleaseWebViewInterludeCacheResultDto {
                released: false,
                removed: false,
            }
        );
        assert!(forged.exists());

        let missing = cache_dir.join("interlude-missing.m4a");
        state
            .protect_webview_interlude_path(missing.clone())
            .expect("missing cache protection should be registered");
        let missing_release = state
            .release_webview_interlude_path(missing.to_str().expect("UTF-8 fixture path"))
            .expect("missing registered cache release should succeed");
        assert_eq!(
            missing_release,
            super::ReleaseWebViewInterludeCacheResultDto {
                released: true,
                removed: false,
            }
        );
        assert!(state
            .protected_webview_interlude_paths()
            .expect("cache protections should be readable")
            .is_empty());
        let _ = std::fs::remove_dir_all(cache_dir);
    }

    #[test]
    fn portaudio_and_webview_interludes_share_validation_and_ambient_resolution() {
        let source = include_str!("commands.rs");
        let portaudio = source
            .split("fn start_portaudio_interlude_blocking")
            .nth(1)
            .expect("PortAudio interlude function")
            .split("fn db_to_linear_gain")
            .next()
            .expect("PortAudio interlude function end");
        let webview = source
            .split("fn prepare_webview_interlude_blocking")
            .nth(1)
            .expect("WebView interlude function")
            .split("fn start_portaudio_interlude_blocking")
            .next()
            .expect("WebView interlude function end");

        for function in [portaudio, webview] {
            assert!(function.contains("validate_interlude_processing_request(&request)"));
            assert!(function.contains("validate_interlude_audio_path(&snapshot, &request.path)"));
            let resolve_at = function
                .find("resolve_active_ambient_sound(")
                .expect("ambient input must be validated");
            let build_at = function
                .find("build_audio_stream_filter_graph_with_ambient(")
                .expect("audio filter graph must be built");
            assert!(resolve_at < build_at);
        }
    }

    #[test]
    fn ordinary_audio_paths_validate_real_ambient_input_before_building_filters() {
        let source = include_str!("commands.rs");
        let initial_processing = source
            .split("pub fn start_media_processing")
            .nth(1)
            .expect("start media processing function")
            .split("pub fn direct_model_chat")
            .next()
            .expect("start media processing function end");
        let ordinary_stream = source
            .split("fn audio_mixer_task_from_snapshot")
            .nth(1)
            .expect("ordinary audio stream function")
            .split("fn validate_audio_cycle_candidate_request")
            .next()
            .expect("ordinary audio stream function end");
        let scheduled_cycle = source
            .split("fn scheduled_audio_cycle_task")
            .nth(1)
            .expect("scheduled audio cycle function")
            .split("fn commit_audio_mixer_candidate")
            .next()
            .expect("scheduled audio cycle function end");

        for (function, resolver) in [
            (initial_processing, "resolve_user_ambient_sound("),
            (ordinary_stream, "resolve_active_ambient_sound("),
            (scheduled_cycle, "resolve_active_ambient_sound("),
        ] {
            let resolve_at = function
                .find(resolver)
                .expect("ambient input must be validated");
            let build_at = function
                .find("build_audio_stream_filter_graph_with_ambient(")
                .expect("audio filter graph must be built");
            assert!(resolve_at < build_at);
        }
    }

    #[test]
    fn independent_audio_candidate_is_bounded_audio_only_and_uses_its_worker_slot() {
        let source = include_str!("commands.rs");
        let function = source
            .split("pub fn prepare_audio_media_candidate")
            .nth(1)
            .expect("independent audio candidate command")
            .split("pub fn start_media_processing")
            .next()
            .expect("independent audio candidate command end");

        assert!(function.contains("source_has_video: false"));
        assert!(function.contains("video_processing_enabled: false"));
        assert!(function.contains("audio_processing_enabled: true"));
        assert!(function.contains(".partial.m4a"));
        assert!(function.contains("processed-audio-g"));
        assert!(function.contains("state.install_audio_media_worker"));
        assert!(!function.contains("thread::spawn(move || thread::spawn"));
    }

    #[test]
    fn audio_source_window_is_checked_before_processing_state_changes() {
        let source = include_str!("commands.rs");
        let independent_audio = source
            .split("pub fn prepare_audio_media_candidate")
            .nth(1)
            .expect("independent audio candidate command")
            .split("pub fn start_media_processing")
            .next()
            .expect("independent audio candidate command end");
        let combined_media = source
            .split("pub fn start_media_processing")
            .nth(1)
            .expect("combined media command")
            .split("pub async fn direct_model_chat")
            .next()
            .expect("combined media command end");

        let independent_validation = independent_audio
            .find("validate_audio_candidate_source_window(")
            .expect("independent audio source window validation");
        let independent_state_change = independent_audio
            .find("mark_audio_media_processing_running(")
            .expect("independent audio processing state change");
        assert!(independent_validation < independent_state_change);

        let combined_validation = combined_media
            .find("validate_audio_candidate_source_window(")
            .expect("combined media audio source window validation");
        let combined_state_change = combined_media
            .find("mark_media_processing_running_with_candidate(")
            .expect("combined media processing state change");
        assert!(combined_validation < combined_state_change);
    }

    #[test]
    fn candidate_discard_commands_never_stop_processing_workers() {
        let source = include_str!("commands.rs");
        let video_discard = source
            .split("pub fn discard_media_processing_candidate")
            .nth(1)
            .expect("video discard command")
            .split("pub fn release_audio_media_candidate")
            .next()
            .expect("video discard command end");
        let audio_discard = source
            .split("pub fn discard_audio_media_candidate")
            .nth(1)
            .expect("audio discard command")
            .split("pub fn commit_audio_media_candidate")
            .next()
            .expect("audio discard command end");

        assert!(!video_discard.contains("stop_video_media_worker"));
        assert!(!audio_discard.contains("stop_audio_media_worker"));
    }

    #[test]
    fn media_processing_respects_independent_video_and_audio_scopes() {
        let source = include_str!("commands.rs");
        let request = source
            .split("pub fn start_media_processing")
            .nth(1)
            .expect("start_media_processing function")
            .split("pub fn direct_model_chat")
            .next()
            .expect("start_media_processing function end");

        assert!(request.contains("matches!(scope, \"video\" | \"both\")"));
        assert!(request.contains("matches!(scope, \"audio\" | \"both\")"));
        assert!(request.contains("scope != \"video\" && request.source_revision"));
        assert!(request.contains("if audio_enabled"));
        assert!(request.contains("MediaRenderRequest"));
        assert!(request.contains("audio_processing_enabled: audio_enabled"));
        assert!(request.contains("render_media_with_progress("));
        assert!(request.contains(".partial.mp4"));
        let audio_commit = request
            .find(".set_audio_stream_configuration(")
            .expect("audio configuration should commit");
        let candidate_identity = request
            .find("let candidate_identity = PendingMediaCandidateIdentity")
            .expect("candidate identity should be created");
        assert!(audio_commit < candidate_identity);
        assert!(request.contains("source_revision: playback.snapshot().audio_stream_revision"));
        assert!(!request.contains("spawn_video_effect_stream_session("));
    }

    #[test]
    fn video_backend_plan_identity_does_not_depend_on_audio_stream_revision() {
        let source = include_str!("commands.rs");
        let prepare = source
            .split("pub fn prepare_realtime_video_plan")
            .nth(1)
            .expect("prepare realtime video function")
            .split("pub fn commit_realtime_video_plan")
            .next()
            .expect("prepare realtime video function end");
        let commit = source
            .split("pub fn commit_realtime_video_plan")
            .nth(1)
            .expect("commit realtime video function")
            .split("pub fn sync_realtime_video_renderer")
            .next()
            .expect("commit realtime video function end");

        for function in [prepare, commit] {
            assert!(function.contains("playback_generation"));
            assert!(!function.contains("snapshot.audio_stream_revision"));
        }
        assert!(prepare.contains("request.source_revision != VIDEO_BACKEND_SOURCE_REVISION"));
        assert!(commit.contains("request.source_revision != VIDEO_BACKEND_SOURCE_REVISION"));
        assert!(prepare.contains("source_revision: request.source_revision"));
        assert!(commit.contains("source_revision: request.source_revision"));
        let support_gate = prepare
            .find("if !compiled.support.fully_supported")
            .expect("unsupported GPU83 fields must be rejected");
        let renderer_prepare = prepare
            .find("runtime\n        .prepare")
            .expect("renderer prepare call");
        assert!(support_gate < renderer_prepare);
    }

    #[test]
    fn realtime_video_fallback_is_truthful_original_without_old_ffmpeg_ipc() {
        let source = include_str!("commands.rs");
        let fallback = source
            .split("fn record_realtime_video_fallback")
            .nth(1)
            .expect("realtime fallback helper")
            .split("fn final_effect_host_window_id")
            .next()
            .expect("realtime fallback helper end");

        assert!(fallback.contains("record_source_backend"));
        assert!(!fallback.contains("record_ffmpeg_backend"));
        let removed_command = ["pub fn activate_", "ffmpeg_video_backend"].concat();
        assert!(!source.contains(&removed_command));
    }

    #[test]
    fn changing_file_processing_switches_stops_only_the_matching_worker() {
        let source = include_str!("commands.rs");
        let command = source
            .split("pub fn set_processing_switches")
            .nth(1)
            .expect("set_processing_switches function")
            .split("pub fn set_audio_processing_profile")
            .next()
            .expect("set_processing_switches function end");

        assert!(command.contains("if video_switch_changed"));
        assert!(command.contains("state.stop_video_media_worker()?;"));
        assert!(!command.contains("media_video_stream"));
        assert!(command.contains("if audio_switch_changed"));
        assert!(command.contains("state.stop_audio_media_worker()?"));
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
    fn playback_item_completion_accepts_only_the_current_completed_item_identity() {
        assert_eq!(should_complete_playback_item(7, 3, 1, 7, 3, 1), Ok(true));
        assert_eq!(should_complete_playback_item(8, 0, 0, 7, 3, 1), Ok(false));
        assert_eq!(should_complete_playback_item(7, 4, 1, 7, 3, 1), Ok(false));
        assert!(should_complete_playback_item(7, 3, 1, 8, 0, 0).is_err());
        assert!(should_complete_playback_item(7, 3, 1, 7, 4, 1).is_err());
        assert!(should_complete_playback_item(7, 3, 1, 7, 3, 0).is_err());
    }

    #[test]
    fn audio_resume_failure_does_not_reverse_a_committed_source_change() {
        let mut playback = PlaybackCore::default();
        playback
            .set_source_pool(vec![
                playback_test_source("a.mp4"),
                playback_test_source("b.mp4"),
            ])
            .expect("pool should be valid");
        playback.set_processing_switches(false, true, false);
        playback.start().expect("pool should start");
        assert!(playback.complete_item().expect("source should advance"));
        let committed = playback.snapshot();

        mark_audio_resume_failure(
            &mut playback,
            &CommandErrorDto::new("audio_output_failed", "设备不可用"),
        );

        let snapshot = playback.snapshot();
        assert_eq!(snapshot.playback_generation, committed.playback_generation);
        assert_eq!(snapshot.source_media_index, 1);
        assert_eq!(snapshot.source_media.expect("source").file_name, "b.mp4");
        assert_eq!(snapshot.audio_processing_status, "unavailable");
        assert!(snapshot
            .fallback_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("设备不可用")));

        let source = include_str!("commands.rs");
        let completion = source
            .split("fn complete_playback_item_inner")
            .nth(1)
            .expect("completion function")
            .split("pub fn commit_media_processing_if_ready")
            .next()
            .expect("completion function end");
        assert!(completion.contains("if let Err(error) = state.resume_audio_output(app)"));
        assert!(!completion.contains("state.resume_audio_output(app)?"));
    }

    #[test]
    fn first_play_defers_portaudio_resume_until_the_output_task_exists() {
        let state = AppState::default();

        assert!(*state.audio_output_preferred.lock().unwrap());
        assert!(state.audio_cycle_output.lock().unwrap().is_none());
        assert!(state
            .audio_output_control_for_resume()
            .expect("default audio output state should be readable")
            .is_none());
    }

    #[test]
    fn playback_pool_import_rejects_invalid_counts_and_duplicate_canonical_paths() {
        assert!(validate_source_media_pool_count(0).is_err());
        assert!(validate_source_media_pool_count(1).is_ok());
        assert!(validate_source_media_pool_count(100).is_ok());
        assert!(validate_source_media_pool_count(101).is_err());

        let mut paths = HashSet::new();
        assert!(insert_canonical_source_path(&mut paths, "C:/videos/a.mp4").is_ok());
        assert!(insert_canonical_source_path(&mut paths, "C:/videos/a.mp4").is_err());
    }

    #[test]
    fn playback_pool_import_bounds_each_utf8_path_before_media_probe() {
        assert!(validate_source_media_paths(&[String::new()]).is_err());
        assert!(validate_source_media_paths(&["   ".to_owned()]).is_err());
        assert!(validate_source_media_paths(&["a".repeat(MAX_SOURCE_MEDIA_PATH_BYTES)]).is_ok());
        assert!(
            validate_source_media_paths(&["界".repeat(MAX_SOURCE_MEDIA_PATH_BYTES / 3)]).is_ok()
        );
        assert!(
            validate_source_media_paths(&["a".repeat(MAX_SOURCE_MEDIA_PATH_BYTES + 1)]).is_err()
        );
    }

    #[test]
    fn playback_pool_path_identity_rejects_stale_or_invalid_reorders() {
        let pool = vec![playback_test_source("a.mp4"), playback_test_source("b.mp4")];
        let a = pool[0].source_path.clone();
        let b = pool[1].source_path.clone();

        assert_eq!(
            source_media_index_by_path(&pool, &b).expect("stable path should resolve"),
            1
        );
        let reordered = reorder_source_media_pool(&pool, &[b.clone(), a.clone()])
            .expect("complete unique order should be accepted");
        assert_eq!(reordered[0].source_path, b);
        assert_eq!(reordered[1].source_path, a.clone());

        assert!(source_media_index_by_path(&pool, "missing.mp4").is_err());
        assert!(reorder_source_media_pool(&pool, std::slice::from_ref(&a)).is_err());
        assert!(reorder_source_media_pool(&pool, &[a.clone(), a]).is_err());
    }

    #[test]
    fn playback_pool_same_order_is_an_idempotent_no_op() {
        let mut playback = PlaybackCore::default();
        playback
            .set_source_pool(vec![
                playback_test_source("a.mp4"),
                playback_test_source("b.mp4"),
            ])
            .expect("pool should be valid");
        playback.start().expect("pool should start");
        let before = playback.snapshot();
        let source_paths = before
            .source_media_pool
            .iter()
            .map(|source| source.source_path.clone())
            .collect::<Vec<_>>();

        apply_source_media_order(&mut playback, &source_paths)
            .expect("same order should be accepted");

        assert_eq!(playback.snapshot(), before);
    }

    #[test]
    fn playback_commands_share_the_pool_transition_boundary() {
        let source = include_str!("commands.rs");
        for (start, end) in [
            ("pub fn start_media_processing", "pub fn direct_model_chat"),
            ("pub fn start_playback", "pub fn pause_playback"),
            ("pub fn pause_playback", "pub fn resume_playback"),
            ("pub fn resume_playback", "pub fn update_playback_position"),
            ("pub fn stop_playback", "pub fn complete_playback_loop"),
        ] {
            let body = source
                .split(start)
                .nth(1)
                .expect("command should exist")
                .split(end)
                .next()
                .expect("command end should exist");
            assert!(
                body.contains("playback_transition.lock()"),
                "{start} must serialize with pool edits"
            );
        }
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
        assert!(is_retryable_audio_mixer_error(
            "audio_mixer_candidate_silent"
        ));
        assert!(is_retryable_audio_mixer_error("audio_mixer_start_stale"));
        assert!(!is_retryable_audio_mixer_error(
            "audio_mixer_candidate_ffmpeg_failed"
        ));
    }

    #[test]
    fn silent_candidate_crossfade_has_a_stable_non_fatal_code() {
        let (reason, reason_code) =
            audio_cycle_crossfade_failure("候选音轨首段为数字静音，保持当前音轨".to_owned());
        assert_eq!(reason, "候选音轨首段为数字静音，保持当前音轨");
        assert_eq!(reason_code, Some("audio_mixer_candidate_silent"));

        let (reason, reason_code) =
            audio_cycle_crossfade_failure("PortAudio 环缓写入失败".to_owned());
        assert_eq!(reason, "候选音轨交叉淡化失败：PortAudio 环缓写入失败");
        assert_eq!(reason_code, None);

        assert_eq!(
            audio_crossfade_error_code("候选音轨首段为数字静音，保持当前音轨"),
            "audio_mixer_candidate_silent"
        );
        assert_eq!(
            audio_crossfade_error_code("PortAudio 环缓写入失败"),
            "audio_mixer_crossfade_failed"
        );
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
