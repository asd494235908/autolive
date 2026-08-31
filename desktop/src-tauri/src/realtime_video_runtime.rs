use crate::audible_audio_clock::{AudibleAudioClock, AudioClockBoundary, AudioClockIdentity};
use crate::media_av_sync::{
    compose_video_speed, AvSyncAction, AvSyncController, AvSyncObservation, SyncBoundary,
};
use crate::media_effect_params::{AdvancedEffectParams, VideoEffectParams};
use crate::media_timeline::MediaSegmentIdentity;
use crate::media_video_cycle::{
    VideoCycleAction, VideoCycleApplyResult, VideoCycleConfig, VideoCycleController,
    VideoCycleEvent, VideoCyclePlan as ControllerVideoCyclePlan, VideoCycleSegmentIdentity,
};
use crate::media_video_frame_scheduler::{
    VideoFrameScheduleObservation, VideoFrameScheduler, VideoScheduleAction, VideoScheduleBoundary,
};
use crate::media_video_gpu_effects::{
    build_gpu83_scheduled_shader_update, build_gpu83_shader_snapshot, Gpu83ParameterCapability,
    GPU83_PARAMETER_MAPPINGS,
};
use crate::realtime_video_backend::{
    estimated_video_fps_from_response, parse_eof_reached_response, playback_time_ms_from_response,
    BackendDemotion, Cpu4Snapshot, ManagedMpvProcess, MpvCommand, MpvGpuProfile, MpvGraphicsApi,
    MpvIpcOptions, MpvLaunchMode, MpvLaunchSpec, MpvPlaybackSpeed, MpvPlaybackState,
    MpvProcessExitEvidence, MpvShaderOptions, MpvVideoObservation, ParameterSupportReport,
    ParameterSupportResult, PendingMpvResponse, RealtimeVideoBackendError, RealtimeVideoPlan,
    RendererLifecycleState, VerifiedMpvExecutable, VerifiedMpvShader, VideoBackend,
    VideoBackendStateMachine, VideoCommitDecision, VideoCommitGate, VideoPlanIdentity,
    VideoPlanSlot,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CONTROL_CHANNEL_CAPACITY: usize = 8;
const WORKER_TICK_INTERVAL: Duration = Duration::from_millis(40);
// 当前锁定 Windows mpv 的本机命名管道探针中，time-pos 热响应中位数约 0.3ms，
// 但会话首响应达到约 102ms。单命令最多等待 250ms，同时整个空闲观测 tick
// 最多占用 350ms，保证控制邮箱能在 500ms 命令上限前重新取得 actor 所有权。
const OBSERVATION_COMMAND_DEADLINE: Duration = Duration::from_millis(250);
const OBSERVATION_TICK_DEADLINE: Duration = Duration::from_millis(350);
const MPV_PIPE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MPV_PIPE_CONNECT_POLL: Duration = Duration::from_millis(20);
const MPV_COMMAND_TIMEOUT: Duration = Duration::from_millis(500);
const PREPARE_REQUEST_DEADLINE: Duration = Duration::from_secs(30);
const COMMIT_REQUEST_DEADLINE: Duration = Duration::from_secs(10);
// 完整 GPU83 快照会与解码、PortAudio 和插话并发竞争 mpv 主线程；2s 会把仍在执行的
// 合法写入误判为断链。硬上限与提交请求预算一致，期间保持 result_unknown 且不降级。
const SHADER_APPLY_HARD_TIMEOUT: Duration = Duration::from_secs(8);
const SHADER_APPLY_SOFT_TIMEOUT: Duration = Duration::from_millis(500);
const SHADER_PLAN_MARKER_HIGH: &str = "al_runtime_plan_hi";
const SHADER_PLAN_MARKER_LOW: &str = "al_runtime_plan_lo";
// 播放意图与同步命令会在 actor 内等待最多一笔 5s 的只读观察事务排空；
// 调用预算必须覆盖排空和随后一次 500ms 控制确认，避免调用方先行进入结果未知。
const SYNCHRONIZE_REQUEST_DEADLINE: Duration = Duration::from_secs(10);
const EOF_RESUME_CONFIRMATION_DEADLINE: Duration = Duration::from_secs(2);
const CONTROL_REQUEST_DEADLINE: Duration = Duration::from_secs(10);
const TICK_FAILURE_DEMOTION_THRESHOLD: u8 = 3;
const TRANSIENT_OBSERVATION_FAILURE_THRESHOLD: u8 = 3;
// loadfile replace 后，mpv 会先恢复 time-pos，再在新 VO 建立后发布 estimated-vf-fps。
// 这个窗口内属性不可用是换源事务的合法中间态，不能触发后端降级。
const SOURCE_TRANSITION_FPS_GRACE: Duration = Duration::from_secs(3);
const SOURCE_TRANSITION_FPS_HARD_TIMEOUT: Duration = Duration::from_secs(5);
const PLAYBACK_OBSERVATION_HARD_TIMEOUT: Duration = Duration::from_secs(5);
const FRAME_BUDGET_OBSERVATION_INTERVAL: Duration = Duration::from_secs(1);
const FRAME_BUDGET_OBSERVATION_FAILURE_THRESHOLD: u8 = 3;
const FRAME_BUDGET_VIOLATION_THRESHOLD: u8 = 3;
const MPV_VO_PERF_SAMPLE_COUNT: usize = 256;
const PRESENTED_PTS_STALL_MIN_THRESHOLD: Duration = Duration::from_millis(1_500);
const PRESENTED_PTS_STALL_MAX_THRESHOLD: Duration = Duration::from_secs(5);
const PRESENTED_PTS_STALL_FRAME_WINDOW: f64 = 3.0;
const MAX_RUNTIME_LABEL_UTF16_UNITS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendActivation {
    Active,
    Configured,
    Available,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportCompleteness {
    Complete,
    Incomplete,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CycleSlotState {
    Active,
    Ready,
    Preparing,
    Planned,
    Late,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoApplyState {
    Idle,
    SourceTransitioning,
    Ready,
    Applying,
    ResultUnknown,
    ReadbackConfirmed,
    PresentedConfirmed,
    Active,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycleSlotStatus {
    pub sequence: u64,
    pub target_pts_ms: u64,
    pub status: CycleSlotState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoEofFact {
    pub playback_generation: u64,
    pub backend_epoch: u64,
    pub clock_epoch: u64,
    pub loop_index: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveVideoCycleSnapshot {
    pub sequence: u64,
    pub fingerprint: String,
    pub video: VideoEffectParams,
    pub advanced: AdvancedEffectParams,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaVideoBackendRuntimeStatus {
    pub status_revision: u64,
    pub playback_generation: Option<u64>,
    pub clock_epoch: Option<u64>,
    pub loop_index: Option<u64>,
    pub backend_epoch: u64,
    pub fallback_floor_mode: String,
    pub backend: VideoBackend,
    pub activation: BackendActivation,
    pub lifecycle: RendererLifecycleState,
    pub gpu_adapter: Option<String>,
    pub graphics_api: Option<String>,
    pub decoder: Option<String>,
    pub filter: Option<String>,
    pub n: Option<CycleSlotStatus>,
    pub n1: Option<CycleSlotStatus>,
    pub n2: Option<CycleSlotStatus>,
    pub apply_state: VideoApplyState,
    pub active_plan_fingerprint: Option<String>,
    pub active_cycle_snapshot: Option<ActiveVideoCycleSnapshot>,
    pub pending_plan_fingerprint: Option<String>,
    pub actual_source_fps: Option<f64>,
    pub confirmed_change_count: u64,
    pub cycle_drift_ms: Option<i64>,
    /// mpv 已呈现 PTS 减去 PortAudio 实际可听 PTS；仅身份匹配且时钟新鲜时存在。
    pub av_sync_drift_ms: Option<i64>,
    pub audible_audio_pts_ms: Option<u64>,
    pub audio_epoch: Option<u64>,
    pub demotion_reason: Option<String>,
    pub support_completeness: SupportCompleteness,
    pub process_id: Option<u32>,
    pub parameter_support: ParameterSupportReport,
    pub ignored_active_parameter_count: usize,
    pub ignored_active_parameter_examples: Vec<String>,
    pub unsupported_parameter_count: usize,
    pub last_demotion: Option<BackendDemotion>,
    pub demotion_history: Vec<BackendDemotion>,
    pub transition_started_at_unix_ms: Option<u64>,
    pub transition_completed_at_unix_ms: Option<u64>,
    pub resume_pts_ms: Option<u64>,
    /// mpv 当前已经呈现的源内视频 PTS；用于 WebView 不支持源封装时继续提供真实媒体时钟。
    pub presented_pts_ms: Option<u64>,
    /// 最近一次由 mpv `pause` 属性确认的物理暂停事实。
    pub physical_paused: Option<bool>,
    /// 最近一次由 mpv `eof-reached` 属性确认的物理 EOF 事实。
    pub physical_eof_reached: Option<bool>,
    pub gpu_pass_p99_ms: Option<f64>,
    pub frame_drop_count: Option<u64>,
    pub decoder_frame_drop_count: Option<u64>,
    pub mistimed_frame_count: Option<u64>,
    pub delayed_frame_count: Option<u64>,
    pub frame_budget_violation_windows: u8,
    pub eof: Option<VideoEofFact>,
}

impl Default for MediaVideoBackendRuntimeStatus {
    fn default() -> Self {
        Self {
            status_revision: 1,
            playback_generation: None,
            clock_epoch: None,
            loop_index: None,
            backend_epoch: 0,
            fallback_floor_mode: MpvLaunchMode::Gpu(MpvGpuProfile::D3d11ZeroCopy)
                .name()
                .to_owned(),
            backend: VideoBackend::Source,
            activation: BackendActivation::Available,
            lifecycle: RendererLifecycleState::Stopped,
            gpu_adapter: None,
            graphics_api: None,
            decoder: None,
            filter: None,
            n: None,
            n1: None,
            n2: None,
            apply_state: VideoApplyState::Idle,
            active_plan_fingerprint: None,
            active_cycle_snapshot: None,
            pending_plan_fingerprint: None,
            actual_source_fps: None,
            confirmed_change_count: 0,
            cycle_drift_ms: None,
            av_sync_drift_ms: None,
            audible_audio_pts_ms: None,
            audio_epoch: None,
            demotion_reason: None,
            support_completeness: SupportCompleteness::NotApplicable,
            process_id: None,
            parameter_support: ParameterSupportReport::empty(VideoBackend::Source),
            ignored_active_parameter_count: 0,
            ignored_active_parameter_examples: Vec::new(),
            unsupported_parameter_count: 0,
            last_demotion: None,
            demotion_history: Vec::new(),
            transition_started_at_unix_ms: None,
            transition_completed_at_unix_ms: None,
            resume_pts_ms: None,
            presented_pts_ms: None,
            physical_paused: None,
            physical_eof_reached: None,
            gpu_pass_p99_ms: None,
            frame_drop_count: None,
            decoder_frame_drop_count: None,
            mistimed_frame_count: None,
            delayed_frame_count: None,
            frame_budget_violation_windows: 0,
            eof: None,
        }
    }
}

#[derive(Debug)]
pub struct PrepareRealtimeRenderer {
    pub executable: VerifiedMpvExecutable,
    pub shader: VerifiedMpvShader,
    pub source_path: PathBuf,
    pub host_window_id: u64,
    pub source_start_ms: u64,
    pub source_duration_ms: u64,
    pub paused: bool,
    pub plan: RealtimeVideoPlan,
    pub n2: Option<CycleSlotStatus>,
    pub session_id: u64,
    pub video_params: VideoEffectParams,
    pub advanced_params: AdvancedEffectParams,
    pub clock_epoch: u64,
    pub loop_index: u64,
    pub backend_epoch: u64,
}

#[derive(Debug)]
pub struct PrepareOriginalRenderer {
    pub executable: VerifiedMpvExecutable,
    pub shader: Option<VerifiedMpvShader>,
    pub source_path: PathBuf,
    pub host_window_id: u64,
    pub source_start_ms: u64,
    pub source_duration_ms: u64,
    pub paused: bool,
    pub playback_generation: u64,
    pub clock_epoch: u64,
    pub loop_index: u64,
    pub backend_epoch: u64,
}

#[derive(Debug)]
pub struct ConfigureRealtimeVideoCycle {
    pub executable: VerifiedMpvExecutable,
    pub shader: VerifiedMpvShader,
    pub source_path: PathBuf,
    pub host_window_id: u64,
    pub source_start_ms: u64,
    pub source_duration_ms: u64,
    pub paused: bool,
    pub session_id: u64,
    pub playback_generation: u64,
    pub clock_epoch: u64,
    pub loop_index: u64,
    pub backend_epoch: u64,
    pub config: VideoCycleConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealtimeVideoSync {
    pub playback_generation: u64,
    pub clock_epoch: u64,
    pub loop_index: u64,
    pub backend_epoch: u64,
    pub position_ms: u64,
    pub paused: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackIntent {
    Play,
    Pause,
    Seek { position_ms: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackIntentRequest {
    pub playback_generation: u64,
    pub intent: PlaybackIntent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdvanceRealtimeVideoAfterEof {
    pub expected_eof: VideoEofFact,
    pub next_playback_generation: u64,
    pub next_loop_index: u64,
    pub next_source_path: PathBuf,
    pub next_source_duration_ms: u64,
    pub paused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeVideoCommit {
    pub gate: VideoCommitGate,
    pub clock_epoch: u64,
    pub loop_index: u64,
    pub backend_epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealtimeVideoOperationLease {
    playback_generation: u64,
    operation_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OperationCursor {
    playback_generation: u64,
    clock_epoch: u64,
    loop_index: u64,
}

#[derive(Debug)]
struct OperationState {
    revision: u64,
    latest_generation: u64,
    stopped_generation: u64,
    latest_cursor: Option<OperationCursor>,
}

impl Default for OperationState {
    fn default() -> Self {
        Self {
            revision: 1,
            latest_generation: 0,
            stopped_generation: 0,
            latest_cursor: None,
        }
    }
}

#[derive(Debug)]
enum RuntimeCommand {
    EnsureOriginal {
        request: PrepareOriginalRenderer,
        operation_revision: u64,
        reply: SyncSender<Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError>>,
    },
    Prepare {
        request: Box<PrepareRealtimeRenderer>,
        operation_revision: u64,
        now_unix_ms: u64,
        reply: SyncSender<Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError>>,
    },
    ConfigureCycle {
        request: Box<ConfigureRealtimeVideoCycle>,
        operation_revision: u64,
        now_unix_ms: u64,
        reply: SyncSender<Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError>>,
    },
    Commit {
        request: RealtimeVideoCommit,
        now_unix_ms: u64,
        reply: SyncSender<Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError>>,
    },
    Synchronize {
        request: RealtimeVideoSync,
        operation_revision: u64,
        now_unix_ms: u64,
        reply: SyncSender<Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError>>,
    },
    PlaybackIntent {
        request: PlaybackIntentRequest,
        operation_revision: u64,
        now_unix_ms: u64,
        reply: SyncSender<Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError>>,
    },
    AdvanceAfterEof {
        request: AdvanceRealtimeVideoAfterEof,
        operation_revision: u64,
        now_unix_ms: u64,
        reply: SyncSender<Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError>>,
    },
    Stop {
        reply: SyncSender<Result<(), RealtimeVideoBackendError>>,
    },
    Suspend {
        reply: SyncSender<Result<(), RealtimeVideoBackendError>>,
    },
    SetProcessingEnabled {
        enabled: bool,
        reply: SyncSender<Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError>>,
    },
    RecordSourceBackend {
        reason: String,
        reply: SyncSender<Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError>>,
    },
}

impl RuntimeCommand {
    fn requires_idle_ipc(&self) -> bool {
        !matches!(
            self,
            Self::Stop { .. } | Self::Suspend { .. } | Self::RecordSourceBackend { .. }
        )
    }

    fn aborts_deferred_commands(&self) -> bool {
        matches!(self, Self::Stop { .. } | Self::Suspend { .. })
    }

    fn is_play_intent(&self) -> bool {
        matches!(
            self,
            Self::PlaybackIntent {
                request: PlaybackIntentRequest {
                    intent: PlaybackIntent::Play,
                    ..
                },
                ..
            }
        )
    }

    fn reject(self, queue_full: bool) {
        let error = || {
            if queue_full {
                RealtimeVideoBackendError::IpcQueueFull
            } else {
                stale_sync_error("延后的视频控制已被停止或挂起覆盖")
            }
        };
        match self {
            Self::EnsureOriginal { reply, .. }
            | Self::Prepare { reply, .. }
            | Self::ConfigureCycle { reply, .. }
            | Self::Commit { reply, .. }
            | Self::Synchronize { reply, .. }
            | Self::PlaybackIntent { reply, .. }
            | Self::AdvanceAfterEof { reply, .. }
            | Self::SetProcessingEnabled { reply, .. }
            | Self::RecordSourceBackend { reply, .. } => {
                let _ignored = reply.try_send(Err(error()));
            }
            Self::Stop { reply } | Self::Suspend { reply } => {
                let _ignored = reply.try_send(Err(error()));
            }
        }
    }
}

#[derive(Debug)]
struct ControllerLifecycle {
    sender: Option<SyncSender<RuntimeCommand>>,
    worker: Option<JoinHandle<()>>,
}

#[derive(Debug)]
pub struct RealtimeVideoRuntime {
    lifecycle: Mutex<ControllerLifecycle>,
    status: Arc<RwLock<MediaVideoBackendRuntimeStatus>>,
    shutdown_requested: Arc<AtomicBool>,
    next_request_id: AtomicU64,
    operation_revision: Arc<AtomicU64>,
    operation_state: Mutex<OperationState>,
}

impl Default for RealtimeVideoRuntime {
    fn default() -> Self {
        Self::with_audible_audio_clock(Arc::new(AudibleAudioClock::default()))
    }
}

impl RealtimeVideoRuntime {
    pub fn with_audible_audio_clock(audible_audio_clock: Arc<AudibleAudioClock>) -> Self {
        Self::with_optional_eof_sender(audible_audio_clock, None)
    }

    pub fn with_audible_audio_clock_and_eof_sender(
        audible_audio_clock: Arc<AudibleAudioClock>,
        eof_event_sender: SyncSender<VideoEofFact>,
    ) -> Self {
        Self::with_optional_eof_sender(audible_audio_clock, Some(eof_event_sender))
    }

    fn with_optional_eof_sender(
        audible_audio_clock: Arc<AudibleAudioClock>,
        eof_event_sender: Option<SyncSender<VideoEofFact>>,
    ) -> Self {
        let status = Arc::new(RwLock::new(MediaVideoBackendRuntimeStatus::default()));
        let shutdown_requested = Arc::new(AtomicBool::new(false));
        let operation_revision = Arc::new(AtomicU64::new(1));
        let (sender, receiver) = mpsc::sync_channel(CONTROL_CHANNEL_CAPACITY);
        let worker_status = Arc::clone(&status);
        let worker_shutdown = Arc::clone(&shutdown_requested);
        let worker_operation_revision = Arc::clone(&operation_revision);
        let worker_audible_audio_clock = Arc::clone(&audible_audio_clock);
        let worker = thread::Builder::new()
            .name("realtime-video-runtime".to_owned())
            .spawn(move || {
                actor_loop(
                    receiver,
                    worker_status,
                    worker_shutdown,
                    worker_operation_revision,
                    worker_audible_audio_clock,
                    eof_event_sender,
                )
            });
        match worker {
            Ok(worker) => Self {
                lifecycle: Mutex::new(ControllerLifecycle {
                    sender: Some(sender),
                    worker: Some(worker),
                }),
                status,
                shutdown_requested,
                next_request_id: AtomicU64::new(1),
                operation_revision,
                operation_state: Mutex::new(OperationState::default()),
            },
            Err(error) => {
                if let Ok(mut snapshot) = status.write() {
                    snapshot.activation = BackendActivation::Failed;
                    snapshot.lifecycle = RendererLifecycleState::Failed;
                    snapshot.demotion_reason = Some(bounded_runtime_reason(
                        &format!("实时画面 worker 启动失败：{error}"),
                        &[],
                    ));
                }
                Self {
                    lifecycle: Mutex::new(ControllerLifecycle {
                        sender: None,
                        worker: None,
                    }),
                    status,
                    shutdown_requested,
                    next_request_id: AtomicU64::new(1),
                    operation_revision,
                    operation_state: Mutex::new(OperationState::default()),
                }
            }
        }
    }
}

impl RealtimeVideoRuntime {
    pub fn status(&self) -> MediaVideoBackendRuntimeStatus {
        match self.status.read() {
            Ok(status) => status.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn prepare(
        &self,
        request: PrepareRealtimeRenderer,
        now_unix_ms: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        let lease = self.reserve_operation(
            request.plan.identity.playback_generation,
            request.clock_epoch,
            request.loop_index,
        )?;
        self.prepare_with_lease(request, now_unix_ms, lease)
    }

    pub fn configure_cycle(
        &self,
        request: ConfigureRealtimeVideoCycle,
        now_unix_ms: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        let lease = self.reserve_operation(
            request.playback_generation,
            request.clock_epoch,
            request.loop_index,
        )?;
        let request_id = self.next_request_id();
        let sender = self.sender()?;
        let (reply, response) = mpsc::sync_channel(1);
        enqueue(
            &sender,
            RuntimeCommand::ConfigureCycle {
                request: Box::new(request),
                operation_revision: lease.operation_revision,
                now_unix_ms,
                reply,
            },
        )?;
        receive_reply(
            response,
            PREPARE_REQUEST_DEADLINE,
            request_id,
            "配置 Rust 视频周期",
        )?
    }

    pub fn reserve_operation(
        &self,
        playback_generation: u64,
        clock_epoch: u64,
        loop_index: u64,
    ) -> Result<RealtimeVideoOperationLease, RealtimeVideoBackendError> {
        let cursor = OperationCursor {
            playback_generation,
            clock_epoch,
            loop_index,
        };
        let mut state = self.operation_state.lock().map_err(|_| {
            RealtimeVideoBackendError::IpcDisconnected("实时画面操作租约状态锁已损坏".to_owned())
        })?;
        reject_stopped_generation(&state, playback_generation)?;
        let advanced = accept_operation_cursor(state.latest_cursor, cursor)?;
        if advanced {
            state.latest_cursor = Some(cursor);
            state.latest_generation = state.latest_generation.max(playback_generation);
            advance_operation_revision(&mut state, &self.operation_revision);
        }
        let operation_revision = state.revision;
        Ok(RealtimeVideoOperationLease {
            playback_generation,
            operation_revision,
        })
    }

    pub fn prepare_with_lease(
        &self,
        request: PrepareRealtimeRenderer,
        now_unix_ms: u64,
        lease: RealtimeVideoOperationLease,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        self.validate_operation_lease(request.plan.identity.playback_generation, lease)?;
        let request_id = self.next_request_id();
        let sender = self.sender()?;
        let (reply, response) = mpsc::sync_channel(1);
        enqueue(
            &sender,
            RuntimeCommand::Prepare {
                request: Box::new(request),
                operation_revision: lease.operation_revision,
                now_unix_ms,
                reply,
            },
        )?;
        receive_reply(
            response,
            PREPARE_REQUEST_DEADLINE,
            request_id,
            "准备 GPU 画面",
        )?
    }

    pub fn ensure_original(
        &self,
        request: PrepareOriginalRenderer,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        let lease = self.reserve_operation(
            request.playback_generation,
            request.clock_epoch,
            request.loop_index,
        )?;
        self.ensure_original_with_lease(request, lease)
    }

    pub fn ensure_original_with_lease(
        &self,
        request: PrepareOriginalRenderer,
        lease: RealtimeVideoOperationLease,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        self.validate_operation_lease(request.playback_generation, lease)?;
        let request_id = self.next_request_id();
        let sender = self.sender()?;
        let (reply, response) = mpsc::sync_channel(1);
        enqueue(
            &sender,
            RuntimeCommand::EnsureOriginal {
                request,
                operation_revision: lease.operation_revision,
                reply,
            },
        )?;
        receive_reply(
            response,
            PREPARE_REQUEST_DEADLINE,
            request_id,
            "启动 Original 画面",
        )?
    }

    pub fn commit(
        &self,
        request: &RealtimeVideoCommit,
        now_unix_ms: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        let request_id = self.next_request_id();
        let sender = self.sender()?;
        let (reply, response) = mpsc::sync_channel(1);
        enqueue(
            &sender,
            RuntimeCommand::Commit {
                request: request.clone(),
                now_unix_ms,
                reply,
            },
        )?;
        receive_reply(
            response,
            COMMIT_REQUEST_DEADLINE,
            request_id,
            "提交视频周期",
        )?
    }

    pub fn synchronize(
        &self,
        request: RealtimeVideoSync,
        now_unix_ms: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        let operation_revision = self.register_authoritative_cursor(
            request.playback_generation,
            request.clock_epoch,
            request.loop_index,
        )?;
        let request_id = self.next_request_id();
        let sender = self.sender()?;
        let (reply, response) = mpsc::sync_channel(1);
        enqueue(
            &sender,
            RuntimeCommand::Synchronize {
                request,
                operation_revision,
                now_unix_ms,
                reply,
            },
        )?;
        receive_reply(
            response,
            SYNCHRONIZE_REQUEST_DEADLINE,
            request_id,
            "同步播放时钟",
        )?
    }

    pub fn playback_intent(
        &self,
        request: PlaybackIntentRequest,
        now_unix_ms: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        let status = self.status();
        if status.playback_generation != Some(request.playback_generation) {
            return Err(RealtimeVideoBackendError::StaleSync {
                field: "播放意图代次",
            });
        }
        let current_clock_epoch =
            status
                .clock_epoch
                .ok_or(RealtimeVideoBackendError::StaleSync {
                    field: "播放意图时钟",
                })?;
        let loop_index = status
            .loop_index
            .ok_or(RealtimeVideoBackendError::StaleSync {
                field: "播放意图循环",
            })?;
        let next_clock_epoch = if matches!(request.intent, PlaybackIntent::Seek { .. }) {
            current_clock_epoch.checked_add(1).ok_or_else(|| {
                RealtimeVideoBackendError::InvalidSync {
                    message: "播放意图 clock_epoch 已耗尽".to_owned(),
                }
            })?
        } else {
            current_clock_epoch
        };
        let operation_revision = self.register_authoritative_cursor(
            request.playback_generation,
            next_clock_epoch,
            loop_index,
        )?;
        let request_id = self.next_request_id();
        let sender = self.sender()?;
        let (reply, response) = mpsc::sync_channel(1);
        enqueue(
            &sender,
            RuntimeCommand::PlaybackIntent {
                request,
                operation_revision,
                now_unix_ms,
                reply,
            },
        )?;
        receive_reply(
            response,
            SYNCHRONIZE_REQUEST_DEADLINE,
            request_id,
            "应用播放意图",
        )?
    }

    pub fn advance_after_eof(
        &self,
        request: AdvanceRealtimeVideoAfterEof,
        now_unix_ms: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        let next_clock_epoch =
            if request.next_playback_generation > request.expected_eof.playback_generation {
                request.expected_eof.clock_epoch.saturating_add(1)
            } else {
                request.expected_eof.clock_epoch
            };
        let operation_revision = self.register_authoritative_cursor(
            request.next_playback_generation,
            next_clock_epoch,
            request.next_loop_index,
        )?;
        let request_id = self.next_request_id();
        let sender = self.sender()?;
        let (reply, response) = mpsc::sync_channel(1);
        enqueue(
            &sender,
            RuntimeCommand::AdvanceAfterEof {
                request,
                operation_revision,
                now_unix_ms,
                reply,
            },
        )?;
        receive_reply(
            response,
            COMMIT_REQUEST_DEADLINE,
            request_id,
            "完成 EOF 播放闭环",
        )?
    }

    pub fn stop(&self, playback_generation: u64) -> Result<(), RealtimeVideoBackendError> {
        let request_id = self.next_request_id();
        let sender = self.sender()?;
        let (reply, response) = mpsc::sync_channel(1);
        {
            let mut state = match self.operation_state.lock() {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
            enqueue_control(&sender, RuntimeCommand::Stop { reply })?;
            let generation = state.latest_generation.max(playback_generation);
            if generation > 0 {
                state.stopped_generation = state.stopped_generation.max(generation);
            }
            advance_operation_revision(&mut state, &self.operation_revision);
        }
        receive_reply(response, CONTROL_REQUEST_DEADLINE, request_id, "停止画面")?
    }

    pub fn suspend(&self) -> Result<(), RealtimeVideoBackendError> {
        let request_id = self.next_request_id();
        let sender = self.sender()?;
        let (reply, response) = mpsc::sync_channel(1);
        self.enqueue_cancelling_control(&sender, RuntimeCommand::Suspend { reply })?;
        receive_reply(response, CONTROL_REQUEST_DEADLINE, request_id, "挂起画面")?
    }

    pub fn set_processing_enabled(
        &self,
        enabled: bool,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        let request_id = self.next_request_id();
        let sender = self.sender()?;
        let (reply, response) = mpsc::sync_channel(1);
        self.enqueue_cancelling_control(
            &sender,
            RuntimeCommand::SetProcessingEnabled { enabled, reply },
        )?;
        receive_reply(
            response,
            CONTROL_REQUEST_DEADLINE,
            request_id,
            "切换视频处理",
        )?
    }

    pub fn record_source_backend(
        &self,
        reason: impl Into<String>,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        let request_id = self.next_request_id();
        let sender = self.sender()?;
        let (reply, response) = mpsc::sync_channel(1);
        self.enqueue_cancelling_control(
            &sender,
            RuntimeCommand::RecordSourceBackend {
                reason: reason.into(),
                reply,
            },
        )?;
        receive_reply(
            response,
            CONTROL_REQUEST_DEADLINE,
            request_id,
            "记录 Source 兜底",
        )?
    }

    pub fn shutdown(&self) -> Result<(), RealtimeVideoBackendError> {
        self.cancel_inflight_operation();
        self.shutdown_requested.store(true, Ordering::Release);
        let worker = {
            let mut lifecycle = match self.lifecycle.lock() {
                Ok(lifecycle) => lifecycle,
                Err(poisoned) => poisoned.into_inner(),
            };
            lifecycle.sender.take();
            lifecycle.worker.take()
        };
        if let Some(worker) = worker {
            worker
                .join()
                .map_err(|_| RealtimeVideoBackendError::ProcessFailed {
                    operation: "worker 回收",
                    message: "实时画面 worker 异常退出".to_owned(),
                })?;
        }
        Ok(())
    }

    fn sender(&self) -> Result<SyncSender<RuntimeCommand>, RealtimeVideoBackendError> {
        if self.shutdown_requested.load(Ordering::Acquire) {
            return Err(runtime_closed_error());
        }
        let sender = self
            .lifecycle
            .lock()
            .map_err(|_| {
                RealtimeVideoBackendError::IpcDisconnected(
                    "实时画面运行时生命周期锁已损坏".to_owned(),
                )
            })?
            .sender
            .clone()
            .ok_or_else(runtime_closed_error)?;
        if self.shutdown_requested.load(Ordering::Acquire) {
            Err(runtime_closed_error())
        } else {
            Ok(sender)
        }
    }

    fn next_request_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::Relaxed)
    }

    fn cancel_inflight_operation(&self) {
        let mut state = match self.operation_state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        advance_operation_revision(&mut state, &self.operation_revision);
    }

    fn enqueue_cancelling_control(
        &self,
        sender: &SyncSender<RuntimeCommand>,
        command: RuntimeCommand,
    ) -> Result<(), RealtimeVideoBackendError> {
        let mut state = match self.operation_state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        enqueue_control(sender, command)?;
        advance_operation_revision(&mut state, &self.operation_revision);
        Ok(())
    }

    fn validate_operation_lease(
        &self,
        playback_generation: u64,
        lease: RealtimeVideoOperationLease,
    ) -> Result<(), RealtimeVideoBackendError> {
        let state = self.operation_state.lock().map_err(|_| {
            RealtimeVideoBackendError::IpcDisconnected("实时画面操作租约状态锁已损坏".to_owned())
        })?;
        reject_stopped_generation(&state, playback_generation)?;
        if lease.playback_generation != playback_generation
            || state.revision != lease.operation_revision
        {
            Err(stale_sync_error("视频启动 lease"))
        } else {
            Ok(())
        }
    }

    fn register_authoritative_cursor(
        &self,
        playback_generation: u64,
        clock_epoch: u64,
        loop_index: u64,
    ) -> Result<u64, RealtimeVideoBackendError> {
        let cursor = OperationCursor {
            playback_generation,
            clock_epoch,
            loop_index,
        };
        let mut state = self.operation_state.lock().map_err(|_| {
            RealtimeVideoBackendError::IpcDisconnected("实时画面操作租约状态锁已损坏".to_owned())
        })?;
        reject_stopped_generation(&state, playback_generation)?;
        if accept_operation_cursor(state.latest_cursor, cursor)? {
            state.latest_cursor = Some(cursor);
            state.latest_generation = state.latest_generation.max(playback_generation);
            advance_operation_revision(&mut state, &self.operation_revision);
        }
        Ok(state.revision)
    }
}

fn reject_stopped_generation(
    state: &OperationState,
    playback_generation: u64,
) -> Result<(), RealtimeVideoBackendError> {
    if state.stopped_generation > 0 && playback_generation <= state.stopped_generation {
        Err(stale_sync_error("已停止播放代次"))
    } else {
        Ok(())
    }
}

fn accept_operation_cursor(
    current: Option<OperationCursor>,
    next: OperationCursor,
) -> Result<bool, RealtimeVideoBackendError> {
    let Some(current) = current else {
        return Ok(true);
    };
    if next == current {
        return Ok(false);
    }
    if next.playback_generation > current.playback_generation {
        return Ok(true);
    }
    if next.playback_generation < current.playback_generation {
        return Err(stale_sync_error("播放代次"));
    }
    let same_loop_seek =
        next.loop_index == current.loop_index && next.clock_epoch > current.clock_epoch;
    let next_loop = next.loop_index == current.loop_index.saturating_add(1)
        && next.clock_epoch >= current.clock_epoch;
    if same_loop_seek || next_loop {
        Ok(true)
    } else {
        Err(stale_sync_error("时钟 cursor"))
    }
}

fn advance_operation_revision(state: &mut OperationState, published: &AtomicU64) {
    state.revision = state.revision.saturating_add(1);
    published.store(state.revision, Ordering::Release);
}

impl Drop for RealtimeVideoRuntime {
    fn drop(&mut self) {
        self.shutdown_requested.store(true, Ordering::Release);
        let lifecycle = match self.lifecycle.get_mut() {
            Ok(lifecycle) => lifecycle,
            Err(poisoned) => poisoned.into_inner(),
        };
        lifecycle.sender.take();
        if let Some(worker) = lifecycle.worker.take() {
            let _ignored = worker.join();
        }
    }
}

fn enqueue(
    sender: &SyncSender<RuntimeCommand>,
    command: RuntimeCommand,
) -> Result<(), RealtimeVideoBackendError> {
    sender.try_send(command).map_err(|error| match error {
        TrySendError::Full(_) => RealtimeVideoBackendError::IpcQueueFull,
        TrySendError::Disconnected(_) => runtime_closed_error(),
    })
}

fn enqueue_control(
    sender: &SyncSender<RuntimeCommand>,
    command: RuntimeCommand,
) -> Result<(), RealtimeVideoBackendError> {
    sender.send(command).map_err(|_| runtime_closed_error())
}

fn receive_reply<T>(
    response: Receiver<T>,
    deadline: Duration,
    request_id: u64,
    operation: &'static str,
) -> Result<T, RealtimeVideoBackendError> {
    response
        .recv_timeout(deadline)
        .map_err(|error| match error {
            RecvTimeoutError::Timeout => RealtimeVideoBackendError::RuntimeTimeout {
                request_id,
                operation,
            },
            RecvTimeoutError::Disconnected => runtime_closed_error(),
        })
}

fn runtime_closed_error() -> RealtimeVideoBackendError {
    RealtimeVideoBackendError::IpcDisconnected("实时画面运行时已关闭".to_owned())
}

fn actor_loop(
    receiver: Receiver<RuntimeCommand>,
    status_snapshot: Arc<RwLock<MediaVideoBackendRuntimeStatus>>,
    shutdown_requested: Arc<AtomicBool>,
    operation_revision: Arc<AtomicU64>,
    audible_audio_clock: Arc<AudibleAudioClock>,
    eof_event_sender: Option<SyncSender<VideoEofFact>>,
) {
    let mut state = RuntimeState {
        operation_revision,
        audible_audio_clock,
        eof_event_sender,
        ..RuntimeState::default()
    };
    let mut deferred_commands = VecDeque::new();
    publish_status(&status_snapshot, &mut state);
    while !shutdown_requested.load(Ordering::Acquire) {
        if !state.has_in_flight_ipc_transaction() {
            if let Some(command) = deferred_commands.pop_front() {
                state.handle(command, &status_snapshot);
                continue;
            }
        }
        match receiver.recv_timeout(WORKER_TICK_INTERVAL) {
            Ok(command) => {
                if shutdown_requested.load(Ordering::Acquire) {
                    break;
                }
                let can_unblock = state.command_can_unblock_shader_presentation(&command);
                if command.requires_idle_ipc()
                    && state.has_in_flight_ipc_transaction()
                    && !can_unblock
                {
                    if deferred_commands.len() >= CONTROL_CHANNEL_CAPACITY {
                        command.reject(true);
                    } else {
                        deferred_commands.push_back(command);
                    }
                    let status_changed = state.tick_in_flight_ipc_transaction();
                    if status_changed {
                        publish_status(&status_snapshot, &mut state);
                    }
                    state.notify_eof_supervisor();
                } else {
                    if command.aborts_deferred_commands() {
                        for deferred in deferred_commands.drain(..) {
                            deferred.reject(false);
                        }
                    }
                    state.handle(command, &status_snapshot);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if shutdown_requested.load(Ordering::Acquire) {
                    break;
                }
                let status_changed = if state.has_in_flight_ipc_transaction() {
                    state.tick_in_flight_ipc_transaction()
                } else {
                    state.tick()
                };
                if status_changed {
                    publish_status(&status_snapshot, &mut state);
                }
                state.notify_eof_supervisor();
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    for deferred in deferred_commands.drain(..) {
        deferred.reject(false);
    }
    state.stop();
    publish_status(&status_snapshot, &mut state);
}

fn publish_status(snapshot: &RwLock<MediaVideoBackendRuntimeStatus>, state: &mut RuntimeState) {
    state.reconcile_status_parameter_support();
    state.reconcile_active_cycle_snapshot();
    state.touch_status();
    if let Ok(mut current) = snapshot.write() {
        *current = state.status.clone();
    }
}

#[derive(Debug, Clone)]
struct PreparedScheduleContext {
    identity: VideoPlanIdentity,
    seed: u64,
    video_params: VideoEffectParams,
    advanced_params: AdvancedEffectParams,
    clock_epoch: u64,
    loop_index: u64,
    paused: bool,
}

#[derive(Debug)]
struct PendingShaderApply {
    response: PendingMpvResponse,
    expected: MpvShaderOptions,
    commit_media_pts_ms: u64,
    commit_source_pts_ms: u64,
    fingerprint: String,
    phase: PendingShaderApplyPhase,
    submitted_at: Instant,
    retry_count: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingShaderApplyPhase {
    AwaitingResponse,
    AwaitingPresentation,
    PresentedConfirmed,
}

fn shader_presentation_can_yield_to_eof(
    phase: PendingShaderApplyPhase,
    eof_observed: bool,
) -> bool {
    eof_observed && phase == PendingShaderApplyPhase::AwaitingPresentation
}

#[derive(Debug)]
struct PendingPlaybackObservation {
    response: PendingMpvResponse,
    phase: PendingPlaybackObservationPhase,
    started_at: Instant,
    cursor: Option<SyncCursor>,
    backend_epoch: u64,
    source_path: Option<PathBuf>,
    eof_reached: bool,
    presented_pts_ms: Option<u64>,
    paused_response: Option<serde_json::Value>,
    seeking_response: Option<serde_json::Value>,
    paused_for_cache_response: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingPlaybackObservationPhase {
    Eof,
    PresentedPts,
    Paused,
    Seeking,
    PausedForCache,
    EofConfirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlaybackObservation {
    eof_reached: bool,
    presented_pts_ms: Option<u64>,
    playback_state: MpvPlaybackState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SyncCursor {
    playback_generation: u64,
    clock_epoch: u64,
    loop_index: u64,
    paused: bool,
}

fn build_due_realtime_video_commit(
    pending: Option<&RealtimeVideoPlan>,
    cursor: Option<SyncCursor>,
    backend_epoch: u64,
    source_duration_ms: u64,
    presented_pts_ms: u64,
) -> Option<RealtimeVideoCommit> {
    let plan = pending?;
    let cursor = cursor.filter(|cursor| !cursor.paused)?;
    if source_duration_ms == 0 {
        return None;
    }
    let absolute_pts_ms = cursor
        .loop_index
        .saturating_mul(source_duration_ms)
        .saturating_add(presented_pts_ms.min(source_duration_ms));
    let gate = VideoCommitGate {
        identity: plan.identity.clone(),
        media_pts_ms: absolute_pts_ms,
    };
    (plan.commit_decision(&gate) == VideoCommitDecision::Commit).then_some(RealtimeVideoCommit {
        gate,
        clock_epoch: cursor.clock_epoch,
        loop_index: cursor.loop_index,
        backend_epoch,
    })
}

fn source_local_schedule_pts(
    absolute_pts_ms: u64,
    source_duration_ms: u64,
    loop_index: u64,
) -> Option<u64> {
    let loop_start_ms = loop_index.checked_mul(source_duration_ms)?;
    let source_pts_ms = absolute_pts_ms.checked_sub(loop_start_ms)?;
    (source_duration_ms > 0 && source_pts_ms <= source_duration_ms).then_some(source_pts_ms)
}

fn eof_fact_from_cursor(
    cursor: Option<SyncCursor>,
    backend_epoch: u64,
    eof_reached: bool,
) -> Option<VideoEofFact> {
    let cursor = eof_reached.then_some(cursor).flatten()?;
    if cursor.paused {
        return None;
    }
    Some(VideoEofFact {
        playback_generation: cursor.playback_generation,
        backend_epoch,
        clock_epoch: cursor.clock_epoch,
        loop_index: cursor.loop_index,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SyncTransition {
    next: SyncCursor,
    boundary: VideoScheduleBoundary,
}

#[derive(Debug, Clone)]
struct RendererSessionContext {
    executable: VerifiedMpvExecutable,
    shader: Option<VerifiedMpvShader>,
    session_id: u64,
    source_path: PathBuf,
    host_window_id: u64,
    playback_generation: u64,
    clock_epoch: u64,
    loop_index: u64,
    source_position_ms: u64,
    source_duration_ms: u64,
    paused: bool,
}

#[derive(Debug)]
struct ObservationTickBudget {
    started: Instant,
}

impl ObservationTickBudget {
    fn start() -> Self {
        Self {
            started: Instant::now(),
        }
    }

    fn remaining(&self) -> Duration {
        OBSERVATION_TICK_DEADLINE
            .saturating_sub(self.started.elapsed())
            .min(OBSERVATION_COMMAND_DEADLINE)
            .max(Duration::from_millis(1))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresentedPtsLiveness {
    Healthy,
    SoftRecover { position_ms: u64 },
    Stalled { position_ms: u64 },
}

#[derive(Debug, Default)]
struct PresentedPtsWatchdog {
    last_pts_ms: Option<u64>,
    stagnant_since: Option<Duration>,
    recovery_attempted: bool,
}

#[derive(Debug, Clone, Copy)]
struct PresentedPtsWatchdogFacts {
    process_exists: bool,
    paused: bool,
    eof_reached: bool,
    activation: BackendActivation,
    lifecycle: RendererLifecycleState,
    backend: VideoBackend,
}

fn should_watch_presented_pts(facts: PresentedPtsWatchdogFacts) -> bool {
    facts.process_exists
        && !facts.paused
        && !facts.eof_reached
        && matches!(
            facts.activation,
            BackendActivation::Active | BackendActivation::Available
        )
        && matches!(
            facts.lifecycle,
            RendererLifecycleState::Active | RendererLifecycleState::Spawned
        )
        && matches!(
            facts.backend,
            VideoBackend::RealtimeGpu | VideoBackend::Cpu4 | VideoBackend::Source
        )
}

fn presented_pts_stall_threshold(source_fps: Option<f64>) -> Duration {
    let Some(source_fps) = source_fps.filter(|fps| fps.is_finite() && (1.0..=240.0).contains(fps))
    else {
        return PRESENTED_PTS_STALL_MAX_THRESHOLD;
    };
    Duration::from_secs_f64(PRESENTED_PTS_STALL_FRAME_WINDOW / source_fps)
        .max(PRESENTED_PTS_STALL_MIN_THRESHOLD)
        .min(PRESENTED_PTS_STALL_MAX_THRESHOLD)
}

impl PresentedPtsWatchdog {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn observe(
        &mut self,
        now: Duration,
        eligible: bool,
        presented_pts_ms: u64,
        stall_threshold: Duration,
    ) -> PresentedPtsLiveness {
        if !eligible {
            self.reset();
            return PresentedPtsLiveness::Healthy;
        }
        if self.last_pts_ms != Some(presented_pts_ms) {
            self.last_pts_ms = Some(presented_pts_ms);
            self.stagnant_since = Some(now);
            self.recovery_attempted = false;
            return PresentedPtsLiveness::Healthy;
        }
        let Some(stagnant_since) = self.stagnant_since else {
            self.stagnant_since = Some(now);
            return PresentedPtsLiveness::Healthy;
        };
        if now.saturating_sub(stagnant_since) < stall_threshold {
            return PresentedPtsLiveness::Healthy;
        }
        if !self.recovery_attempted {
            self.recovery_attempted = true;
            self.stagnant_since = Some(now);
            return PresentedPtsLiveness::SoftRecover {
                position_ms: presented_pts_ms,
            };
        }
        PresentedPtsLiveness::Stalled {
            position_ms: presented_pts_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct VoPassTimingSeries {
    description: String,
    occurrence: usize,
    samples_ns: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
struct VoPassesTimingSnapshot {
    passes: Vec<VoPassTimingSeries>,
}

#[derive(Debug)]
struct RuntimeState {
    process: Option<ManagedMpvProcess>,
    processing_enabled: bool,
    process_launch_mode: Option<MpvLaunchMode>,
    process_host_window_id: Option<u64>,
    source_path: Option<PathBuf>,
    session: Option<RendererSessionContext>,
    backend_epoch: u64,
    backend: VideoBackendStateMachine,
    pending: Option<RealtimeVideoPlan>,
    current: Option<RealtimeVideoPlan>,
    pending_schedule: Option<PreparedScheduleContext>,
    current_schedule: Option<PreparedScheduleContext>,
    cycle_controller: Option<VideoCycleController>,
    pending_shader_apply: Option<PendingShaderApply>,
    pending_source_fps_response: Option<PendingMpvResponse>,
    pending_playback_observation: Option<PendingPlaybackObservation>,
    scheduler: VideoFrameScheduler,
    schedule_epoch: u64,
    sync_cursor: Option<SyncCursor>,
    pending_boundary: VideoScheduleBoundary,
    nominal_source_fps: Option<f64>,
    source_transition_started_at: Option<Instant>,
    last_playback_speed: Option<MpvPlaybackSpeed>,
    scheduler_base_speed: f64,
    av_sync_correction: f64,
    av_sync_controller: AvSyncController,
    av_sync_audio_identity: Option<AudioClockIdentity>,
    av_sync_control_epoch: u64,
    av_sync_monotonic_origin: Instant,
    audible_audio_clock: Arc<AudibleAudioClock>,
    last_shader_options: Option<MpvShaderOptions>,
    presented_pts_watchdog: PresentedPtsWatchdog,
    consecutive_tick_failures: u8,
    consecutive_transient_observation_failures: u8,
    last_frame_budget_observation: Option<Instant>,
    last_frame_budget_pts_ms: Option<u64>,
    last_vo_passes_snapshot: Option<VoPassesTimingSnapshot>,
    last_frame_health_counts: Option<(u64, u64, Option<u64>, Option<u64>)>,
    consecutive_frame_budget_observation_failures: u8,
    consecutive_frame_budget_violations: u8,
    status: MediaVideoBackendRuntimeStatus,
    status_revision_counter: u64,
    operation_revision: Arc<AtomicU64>,
    eof_event_sender: Option<SyncSender<VideoEofFact>>,
    last_notified_eof: Option<VideoEofFact>,
    eof_notification_backpressured: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GpuPrepareDisposition {
    RecoverLostSession,
    RestartRenderer,
    ReuseRenderer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GenerationTransition {
    Same,
    PreservedProcess,
    ResetProcess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GenerationReuseFacts {
    current_generation: u64,
    requested_generation: u64,
    process_present: bool,
    eof_matches_current_identity: bool,
}

fn can_preserve_process_for_next_generation(facts: GenerationReuseFacts) -> bool {
    facts.process_present
        && facts.eof_matches_current_identity
        && facts.requested_generation == facts.current_generation.saturating_add(1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GpuPrepareFacts {
    process_present: bool,
    process_exited: bool,
    backend_epoch: u64,
    lifecycle: RendererLifecycleState,
    renderer_ready_for_gpu: bool,
    source_matches: bool,
    host_window_matches: bool,
    generation_changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EffectSessionHealthFacts {
    processing_enabled: bool,
    effect_backend: bool,
    activation_healthy: bool,
    lifecycle_healthy: bool,
    status_has_pid: bool,
    generation_matches: bool,
    session_matches: bool,
    process_running: bool,
}

fn should_keep_effect_renderer(facts: EffectSessionHealthFacts) -> bool {
    facts.processing_enabled
        && facts.effect_backend
        && facts.activation_healthy
        && facts.lifecycle_healthy
        && facts.status_has_pid
        && facts.generation_matches
        && facts.session_matches
        && facts.process_running
}

fn effect_session_is_owned_by_cycle(
    logical_backend: VideoBackend,
    controller_present: bool,
    cycle_session_available: bool,
) -> bool {
    matches!(
        logical_backend,
        VideoBackend::RealtimeGpu | VideoBackend::Cpu4
    ) || (logical_backend == VideoBackend::Source && controller_present && cycle_session_available)
}

fn renderer_host_window_changed(
    process_present: bool,
    bound_host_window_id: Option<u64>,
    requested_host_window_id: u64,
) -> bool {
    process_present && bound_host_window_id != Some(requested_host_window_id)
}

fn retain_same_source_process(
    source_matches: bool,
    mode_matches: bool,
    host_matches: bool,
    process_present: bool,
) -> bool {
    source_matches && mode_matches && host_matches && process_present
}

fn renderer_ready_for_mode(
    logical_backend: VideoBackend,
    physical_mode: Option<MpvLaunchMode>,
    requested_mode: MpvLaunchMode,
) -> bool {
    logical_backend == requested_mode.backend()
        || (logical_backend == VideoBackend::Source && physical_mode == Some(requested_mode))
}

fn cycle_session_is_available(
    processing_enabled: bool,
    process_present: bool,
    session_present: bool,
    physical_mode: Option<MpvLaunchMode>,
) -> bool {
    processing_enabled
        && process_present
        && session_present
        && matches!(
            physical_mode,
            Some(MpvLaunchMode::Gpu(_) | MpvLaunchMode::Cpu4)
        )
}

fn neutral_source_launch_mode(
    shader_available: bool,
    fallback_mode: MpvLaunchMode,
) -> MpvLaunchMode {
    match fallback_mode {
        MpvLaunchMode::Gpu(_) if !shader_available => MpvLaunchMode::Original,
        mode => mode,
    }
}

fn gpu_prepare_disposition(facts: GpuPrepareFacts) -> GpuPrepareDisposition {
    if facts.process_exited
        || (!facts.process_present
            && facts.backend_epoch > 0
            && facts.lifecycle != RendererLifecycleState::Stopped)
    {
        GpuPrepareDisposition::RecoverLostSession
    } else if !facts.process_present
        || !facts.renderer_ready_for_gpu
        || !facts.source_matches
        || !facts.host_window_matches
        || facts.generation_changed
    {
        GpuPrepareDisposition::RestartRenderer
    } else {
        GpuPrepareDisposition::ReuseRenderer
    }
}

fn unexpected_mpv_exit_reason(
    backend_label: &str,
    evidence: Option<&MpvProcessExitEvidence>,
    status_error: Option<&RealtimeVideoBackendError>,
) -> String {
    if let Some(evidence) = evidence {
        return format!("{backend_label} mpv 会话异常退出：{}", evidence.summary());
    }
    if let Some(error) = status_error {
        return format!("{backend_label} mpv 状态检查失败：{error}");
    }
    format!("{backend_label} mpv 会话已退出或丢失")
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            process: None,
            processing_enabled: false,
            process_launch_mode: None,
            process_host_window_id: None,
            source_path: None,
            session: None,
            backend_epoch: 0,
            backend: VideoBackendStateMachine::new(),
            pending: None,
            current: None,
            pending_schedule: None,
            current_schedule: None,
            cycle_controller: None,
            pending_shader_apply: None,
            pending_source_fps_response: None,
            pending_playback_observation: None,
            scheduler: VideoFrameScheduler::new(),
            schedule_epoch: 0,
            sync_cursor: None,
            pending_boundary: VideoScheduleBoundary::None,
            nominal_source_fps: None,
            source_transition_started_at: None,
            last_playback_speed: None,
            scheduler_base_speed: 1.0,
            av_sync_correction: 1.0,
            av_sync_controller: AvSyncController::default(),
            av_sync_audio_identity: None,
            av_sync_control_epoch: 0,
            av_sync_monotonic_origin: Instant::now(),
            audible_audio_clock: Arc::new(AudibleAudioClock::default()),
            last_shader_options: None,
            presented_pts_watchdog: PresentedPtsWatchdog::default(),
            consecutive_tick_failures: 0,
            consecutive_transient_observation_failures: 0,
            last_frame_budget_observation: None,
            last_frame_budget_pts_ms: None,
            last_vo_passes_snapshot: None,
            last_frame_health_counts: None,
            consecutive_frame_budget_observation_failures: 0,
            consecutive_frame_budget_violations: 0,
            status: MediaVideoBackendRuntimeStatus::default(),
            status_revision_counter: 1,
            operation_revision: Arc::new(AtomicU64::new(1)),
            eof_event_sender: None,
            last_notified_eof: None,
            eof_notification_backpressured: false,
        }
    }
}

impl RuntimeState {
    fn has_in_flight_ipc_transaction(&self) -> bool {
        self.pending_shader_apply.is_some()
            || self.pending_source_fps_response.is_some()
            || self.pending_playback_observation.is_some()
    }

    fn command_can_unblock_shader_presentation(&self, command: &RuntimeCommand) -> bool {
        let play_can_unblock = command.is_play_intent()
            && self.sync_cursor.is_some_and(|cursor| cursor.paused)
            && self.pending_source_fps_response.is_none()
            && self.pending_playback_observation.is_none()
            && self.pending_shader_apply.as_ref().is_some_and(|apply| {
                matches!(
                    apply.phase,
                    PendingShaderApplyPhase::AwaitingPresentation
                        | PendingShaderApplyPhase::PresentedConfirmed
                )
            });
        let eof_can_unblock = matches!(
            command,
            RuntimeCommand::AdvanceAfterEof { request, .. }
                if self.status.eof.as_ref() == Some(&request.expected_eof)
                    && self.pending_shader_apply.as_ref().is_some_and(|apply| {
                        shader_presentation_can_yield_to_eof(apply.phase, true)
                    })
        );
        play_can_unblock || eof_can_unblock
    }

    fn tick_in_flight_ipc_transaction(&mut self) -> bool {
        if self.pending_shader_apply.is_some() {
            return self.tick();
        }
        if self.pending_source_fps_response.is_some()
            && self.sync_cursor.is_none_or(|cursor| cursor.paused)
        {
            return match self.poll_source_fps_response() {
                Ok(changed) => changed,
                Err(error) => self.record_tick_failure(error),
            };
        }
        if self.pending_playback_observation.is_some()
            && self.sync_cursor.is_none_or(|cursor| cursor.paused)
        {
            return match self.poll_playback_observation() {
                Ok(Some(observation)) => {
                    let mut changed = self.record_eof_observation(
                        observation.eof_reached,
                        observation.playback_state.paused,
                    );
                    if let Some(presented_pts_ms) = observation.presented_pts_ms {
                        changed |= self.record_presented_pts(presented_pts_ms);
                    }
                    self.record_observation_success();
                    changed
                }
                Ok(None) => false,
                Err(error) => self.record_tick_failure(error),
            };
        }
        self.tick()
    }

    fn reconcile_status_parameter_support(&mut self) {
        if self.status.parameter_support.backend == self.status.backend
            && (self.status.backend != VideoBackend::Source
                || self.status.support_completeness == SupportCompleteness::NotApplicable)
        {
            return;
        }
        let support = match self.status.backend {
            VideoBackend::Source => ParameterSupportReport::empty(VideoBackend::Source),
            VideoBackend::RealtimeGpu => gpu83_cycle_parameter_support(),
            VideoBackend::Cpu4 => cpu4_parameter_support_from(&gpu83_cycle_parameter_support()),
        };
        self.record_parameter_support(support);
    }

    fn reconcile_active_cycle_snapshot(&mut self) {
        let active = match (
            self.status.backend,
            self.status.activation,
            self.status.lifecycle,
            self.status.apply_state,
            self.status.n.as_ref(),
            self.status.active_plan_fingerprint.as_ref(),
            self.current_schedule.as_ref(),
        ) {
            (
                VideoBackend::RealtimeGpu | VideoBackend::Cpu4,
                BackendActivation::Active,
                RendererLifecycleState::Active,
                VideoApplyState::Active,
                Some(slot),
                Some(fingerprint),
                Some(schedule),
            ) if slot.status == CycleSlotState::Active
                && slot.sequence == schedule.identity.sequence =>
            {
                Some(ActiveVideoCycleSnapshot {
                    sequence: slot.sequence,
                    fingerprint: fingerprint.clone(),
                    video: schedule.video_params.clone(),
                    advanced: schedule.advanced_params.clone(),
                })
            }
            _ => None,
        };
        self.status.active_cycle_snapshot = active;
    }

    fn touch_status(&mut self) {
        self.status_revision_counter = self.status_revision_counter.saturating_add(1);
        self.status.status_revision = self.status_revision_counter;
    }

    fn set_apply_state(&mut self, state: VideoApplyState) {
        if self.status.apply_state != state {
            self.status.apply_state = state;
            self.touch_status();
        }
    }

    fn begin_source_transition(&mut self) {
        self.pending_shader_apply = None;
        self.pending_source_fps_response = None;
        self.pending_playback_observation = None;
        self.nominal_source_fps = None;
        self.source_transition_started_at = Some(Instant::now());
        self.record_observation_success();
        self.status.actual_source_fps = None;
        self.status.active_plan_fingerprint = None;
        self.status.pending_plan_fingerprint = None;
        self.status.n = None;
        self.status.n1 = None;
        self.status.n2 = None;
        if cycle_session_is_available(
            self.processing_enabled,
            self.process.is_some(),
            self.session.is_some(),
            self.process_launch_mode,
        ) {
            self.mark_cycle_status_available();
            self.set_apply_state(VideoApplyState::SourceTransitioning);
        } else {
            self.cycle_controller = None;
            self.set_apply_state(VideoApplyState::Idle);
        }
    }

    fn invalidate_cycle_before_source_recovery(&mut self) {
        self.pending = None;
        self.current = None;
        self.pending_schedule = None;
        self.current_schedule = None;
        if let Some(controller) = self.cycle_controller.as_mut() {
            controller.clear();
        }
        self.begin_source_transition();
    }

    fn handle(
        &mut self,
        command: RuntimeCommand,
        status_snapshot: &RwLock<MediaVideoBackendRuntimeStatus>,
    ) {
        match command {
            RuntimeCommand::EnsureOriginal {
                request,
                operation_revision,
                reply,
            } => {
                let result = self.ensure_original(request, operation_revision);
                publish_status(status_snapshot, self);
                let result = result.map(|_| self.status.clone());
                let _ignored = reply.try_send(result);
            }
            RuntimeCommand::PlaybackIntent {
                request,
                operation_revision,
                now_unix_ms,
                reply,
            } => {
                let result = self.apply_playback_intent(request, operation_revision, now_unix_ms);
                publish_status(status_snapshot, self);
                let result = result.map(|_| self.status.clone());
                let _ignored = reply.try_send(result);
            }
            RuntimeCommand::AdvanceAfterEof {
                request,
                operation_revision,
                now_unix_ms,
                reply,
            } => {
                if let Err(error) =
                    self.validate_advance_after_eof_identity(&request, operation_revision)
                {
                    let _ignored = reply.try_send(Err(error));
                    return;
                }
                self.invalidate_cycle_before_source_recovery();
                publish_status(status_snapshot, self);
                let result = self.advance_after_eof(request, operation_revision, now_unix_ms);
                publish_status(status_snapshot, self);
                let result = result.map(|_| self.status.clone());
                let _ignored = reply.try_send(result);
            }
            RuntimeCommand::Prepare {
                request,
                operation_revision,
                now_unix_ms,
                reply,
            } => {
                let result = self.prepare(*request, now_unix_ms, operation_revision);
                publish_status(status_snapshot, self);
                let result = result.map(|_| self.status.clone());
                let _ignored = reply.try_send(result);
            }
            RuntimeCommand::ConfigureCycle {
                request,
                operation_revision,
                now_unix_ms,
                reply,
            } => {
                if self.pending_shader_apply.is_some() {
                    // 在途 shader 请求无法从 mpv 撤销。重新配置若继续复用该 IPC
                    // 会话，旧命令可能在中性快照之后迟到执行，因此必须先失效进程。
                    self.current = None;
                    self.current_schedule = None;
                    self.last_shader_options = None;
                    self.stop_process();
                }
                self.invalidate_cycle_before_source_recovery();
                publish_status(status_snapshot, self);
                let result = self.configure_cycle(*request, now_unix_ms, operation_revision);
                publish_status(status_snapshot, self);
                let result = result.map(|_| self.status.clone());
                let _ignored = reply.try_send(result);
            }
            RuntimeCommand::Commit {
                request,
                now_unix_ms,
                reply,
            } => {
                let result = self.commit(&request, now_unix_ms);
                publish_status(status_snapshot, self);
                let result = result.map(|_| self.status.clone());
                let _ignored = reply.try_send(result);
            }
            RuntimeCommand::Synchronize {
                request,
                operation_revision,
                now_unix_ms,
                reply,
            } => {
                let result = self.synchronize(request, operation_revision, now_unix_ms);
                publish_status(status_snapshot, self);
                let result = result.map(|_| self.status.clone());
                let _ignored = reply.try_send(result);
            }
            RuntimeCommand::Stop { reply } => {
                self.stop();
                publish_status(status_snapshot, self);
                let _ignored = reply.try_send(Ok(()));
            }
            RuntimeCommand::Suspend { reply } => {
                self.suspend();
                publish_status(status_snapshot, self);
                let _ignored = reply.try_send(Ok(()));
            }
            RuntimeCommand::SetProcessingEnabled { enabled, reply } => {
                let result = self.set_processing_enabled(enabled);
                publish_status(status_snapshot, self);
                let result = result.map(|_| self.status.clone());
                let _ignored = reply.try_send(result);
            }
            RuntimeCommand::RecordSourceBackend { reason, reply } => {
                self.record_source_backend(reason);
                publish_status(status_snapshot, self);
                let _ignored = reply.try_send(Ok(self.status.clone()));
            }
        }
    }

    fn prepare(
        &mut self,
        request: PrepareRealtimeRenderer,
        now_unix_ms: u64,
        operation_revision: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        validate_prepare_request(&request)?;
        let canonical_source = request.source_path.canonicalize().map_err(|error| {
            RealtimeVideoBackendError::InvalidMediaPath {
                path: request.source_path.clone(),
                message: error.to_string(),
            }
        })?;
        if request
            .plan
            .parameter_support
            .ignored_active_parameter_count()
            > 0
        {
            return Err(RealtimeVideoBackendError::ProcessFailed {
                operation: "计划准备",
                message: "实时画面计划包含尚未接入的活动参数".to_owned(),
            });
        }
        let generation_transition =
            self.begin_generation(request.plan.identity.playback_generation)?;
        validate_prepare_backend_epoch(request.backend_epoch, self.backend_epoch)?;
        self.session = Some(RendererSessionContext {
            executable: request.executable.clone(),
            shader: Some(request.shader.clone()),
            session_id: request.session_id,
            source_path: canonical_source.clone(),
            host_window_id: request.host_window_id,
            playback_generation: request.plan.identity.playback_generation,
            clock_epoch: request.clock_epoch,
            loop_index: request.loop_index,
            source_position_ms: request.source_start_ms,
            source_duration_ms: request.source_duration_ms,
            paused: request.paused,
        });
        self.try_switch_preserved_process(
            generation_transition,
            &canonical_source,
            request.host_window_id,
            request.source_start_ms,
            request.source_duration_ms,
            request.paused,
            true,
            operation_revision,
        )?;
        match self.backend.launch_mode() {
            MpvLaunchMode::Cpu4 => {
                return self.prepare_cpu4(
                    request,
                    &canonical_source,
                    now_unix_ms,
                    operation_revision,
                )
            }
            MpvLaunchMode::Original => {
                let process_exited = self
                    .process
                    .as_mut()
                    .is_some_and(|process| process.has_exited().unwrap_or(true));
                let must_restart = self.process.is_none()
                    || self.process_launch_mode != Some(MpvLaunchMode::Original)
                    || self.source_path.as_ref() != Some(&canonical_source)
                    || renderer_host_window_changed(
                        self.process.is_some(),
                        self.process_host_window_id,
                        request.host_window_id,
                    )
                    || process_exited;
                if must_restart {
                    self.stop_process();
                    if let Err(error) = self.start_original_fallback(operation_revision) {
                        self.record_original_failure(error.to_string());
                        return Err(error);
                    }
                }
                self.record_neutral_source_status()?;
                return Ok(self.status.clone());
            }
            MpvLaunchMode::Gpu(_) => {}
        }
        self.current = self
            .current
            .take()
            .filter(|plan| same_playback_identity(&plan.identity, &request.plan.identity));
        let generation_changed = self.sync_cursor.is_some_and(|cursor| {
            cursor.playback_generation != request.plan.identity.playback_generation
        });
        let process_present = self.process.is_some();
        let (process_exit_evidence, process_status_error) = match self.process.as_mut() {
            Some(process) => match process.poll_exit_evidence() {
                Ok(evidence) => (evidence, None),
                Err(error) => (None, Some(error)),
            },
            None => (None, None),
        };
        let process_exited = process_exit_evidence.is_some() || process_status_error.is_some();
        let disposition = gpu_prepare_disposition(GpuPrepareFacts {
            process_present,
            process_exited,
            backend_epoch: self.backend_epoch,
            lifecycle: self.status.lifecycle,
            renderer_ready_for_gpu: renderer_ready_for_mode(
                self.status.backend,
                self.process_launch_mode,
                self.backend.launch_mode(),
            ),
            source_matches: self.source_path.as_ref() == Some(&canonical_source),
            host_window_matches: !renderer_host_window_changed(
                process_present,
                self.process_host_window_id,
                request.host_window_id,
            ),
            generation_changed,
        });
        if disposition == GpuPrepareDisposition::RecoverLostSession {
            let fallback_plan = cpu4_plan_from(&request.plan, &request.video_params)?;
            self.transition_to_fallback(
                unexpected_mpv_exit_reason(
                    "GPU",
                    process_exit_evidence.as_ref(),
                    process_status_error.as_ref(),
                ),
                now_unix_ms,
                Some(fallback_plan),
                false,
                operation_revision,
            );
            match self.backend.launch_mode() {
                MpvLaunchMode::Cpu4 => {
                    return self.prepare_cpu4(
                        request,
                        &canonical_source,
                        now_unix_ms,
                        operation_revision,
                    );
                }
                MpvLaunchMode::Original => return Ok(self.status.clone()),
                MpvLaunchMode::Gpu(_) if self.process.is_none() => {
                    return Err(RealtimeVideoBackendError::ProcessFailed {
                        operation: "GPU 恢复",
                        message: "下一级 GPU 后端未能启动".to_owned(),
                    });
                }
                MpvLaunchMode::Gpu(_) => {}
            }
        }
        let must_restart = disposition == GpuPrepareDisposition::RestartRenderer;
        if must_restart {
            let replayed_shader_options =
                gpu_replay_shader_options(self.current.as_ref(), self.last_shader_options.as_ref());
            self.stop_process();
            if let Err(error) = self.start_renderer(&request, &canonical_source, operation_revision)
            {
                if self.operation_cancelled(operation_revision) {
                    return Err(error);
                }
                let fallback_plan = cpu4_plan_from(&request.plan, &request.video_params)?;
                self.transition_to_fallback(
                    error.to_string(),
                    now_unix_ms,
                    Some(fallback_plan),
                    false,
                    operation_revision,
                );
                match self.backend.launch_mode() {
                    MpvLaunchMode::Cpu4 => {
                        return self.prepare_cpu4(
                            request,
                            &canonical_source,
                            now_unix_ms,
                            operation_revision,
                        );
                    }
                    MpvLaunchMode::Original => return Ok(self.status.clone()),
                    MpvLaunchMode::Gpu(_) if self.process.is_none() => return Err(error),
                    MpvLaunchMode::Gpu(_) => {}
                }
            }
            self.reset_scheduler(
                request.plan.identity.playback_generation,
                request.clock_epoch,
                request.loop_index,
                request.paused,
            );
            self.last_shader_options = replayed_shader_options;
        }
        self.pending_schedule = Some(PreparedScheduleContext {
            identity: request.plan.identity.clone(),
            seed: request.plan.seed,
            video_params: request.video_params,
            advanced_params: request.advanced_params,
            clock_epoch: request.clock_epoch,
            loop_index: request.loop_index,
            paused: request.paused,
        });
        self.pending = Some(request.plan.clone());
        self.claim_realtime_status();
        (self.status.activation, self.status.lifecycle) =
            prepared_renderer_state(self.current.is_some());
        self.status.n = self
            .current
            .as_ref()
            .map(|plan| slot_status(plan, CycleSlotState::Active));
        self.status.n1 = Some(slot_status(&request.plan, CycleSlotState::Ready));
        self.status.n2 = request.n2;
        self.status.pending_plan_fingerprint = self
            .status
            .actual_source_fps
            .map(|source_fps| {
                plan_fingerprint(
                    &request.plan,
                    request.clock_epoch,
                    request.loop_index,
                    source_fps,
                )
            })
            .transpose()?
            .flatten();
        if self.current.is_none() {
            self.set_apply_state(VideoApplyState::Ready);
        }
        self.status.process_id = self.process.as_ref().and_then(ManagedMpvProcess::pid);
        self.record_parameter_support(request.plan.parameter_support);
        Ok(self.status.clone())
    }

    fn configure_cycle(
        &mut self,
        request: ConfigureRealtimeVideoCycle,
        now_unix_ms: u64,
        operation_revision: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        let ConfigureRealtimeVideoCycle {
            executable,
            shader,
            source_path,
            host_window_id,
            source_start_ms,
            source_duration_ms,
            paused,
            session_id,
            playback_generation,
            clock_epoch,
            loop_index,
            backend_epoch,
            config,
        } = request;
        self.pending_shader_apply = None;
        let neutral_plan = RealtimeVideoPlan {
            slot: VideoPlanSlot::NPlus1,
            identity: VideoPlanIdentity {
                session_id,
                playback_generation,
                source_revision: 0,
                parameter_revision: 0,
                sequence: 0,
            },
            target_pts_ms: source_start_ms,
            period_ms: 0,
            seed: 0,
            prepared: true,
            commands: vec![MpvCommand::SetShaderOptions {
                options: neutral_gpu_shader_options()?,
            }],
            parameter_support: gpu83_cycle_parameter_support(),
        };
        self.prepare(
            PrepareRealtimeRenderer {
                executable,
                shader,
                source_path,
                host_window_id,
                source_start_ms,
                source_duration_ms,
                paused,
                plan: neutral_plan,
                n2: None,
                session_id,
                video_params: VideoEffectParams::default(),
                advanced_params: AdvancedEffectParams::default(),
                clock_epoch,
                loop_index,
                backend_epoch,
            },
            now_unix_ms,
            operation_revision,
        )?;
        if self.status.backend != VideoBackend::Source && self.process.is_some() {
            // 重新配置会废弃旧 N。先把物理后端恢复为完整中性快照，避免在新的
            // 首周期到期前继续呈现已经从权威状态中清除的旧 shader/CPU4 参数。
            self.last_shader_options = self.apply_neutral_process_parameters()?;
        }
        self.finish_cycle_configuration(config);
        Ok(self.status.clone())
    }

    fn finish_cycle_configuration(&mut self, config: VideoCycleConfig) {
        self.pending = None;
        self.current = None;
        self.pending_schedule = None;
        self.current_schedule = None;
        self.pending_shader_apply = None;
        self.pending_source_fps_response = None;
        self.pending_playback_observation = None;
        self.nominal_source_fps = None;
        self.status.actual_source_fps = None;
        self.status.n = None;
        self.status.n1 = None;
        self.status.n2 = None;
        self.status.active_plan_fingerprint = None;
        self.status.pending_plan_fingerprint = None;
        if !cycle_session_is_available(
            self.processing_enabled,
            self.process.is_some(),
            self.session.is_some(),
            self.process_launch_mode,
        ) {
            self.cycle_controller = None;
            self.set_apply_state(VideoApplyState::Idle);
            return;
        }
        self.cycle_controller = Some(VideoCycleController::new(config));
        // 首周期尚未产生 N 时也必须按物理启动模式发布一致的逻辑后端与参数支持。
        // 否则中性 Source 状态可能留下 Source 参数报告，却被 GPU/CPU4 backend 复用，
        // React 会按 fail-closed 拒绝整个 DTO，周期状态也失去可观察性。
        self.claim_cycle_backend_status();
        // prepare() 会在旧 N 存在时保留 Active。配置事务已经清空旧 N 后必须显式
        // 回到 Available，才能由下一次 mpv observation 读取新 FPS 并建立首个计划。
        self.mark_cycle_status_available();
        self.set_apply_state(VideoApplyState::SourceTransitioning);
    }

    fn healthy_effect_session_for_generation(&mut self, playback_generation: u64) -> bool {
        let process_running = self
            .process
            .as_mut()
            .is_some_and(|process| matches!(process.has_exited(), Ok(false)));
        let cycle_session_available = cycle_session_is_available(
            self.processing_enabled,
            self.process.is_some(),
            self.session.is_some(),
            self.process_launch_mode,
        );
        should_keep_effect_renderer(EffectSessionHealthFacts {
            processing_enabled: self.processing_enabled,
            // 首周期确认前逻辑后端仍是中性 Source。已经安装控制器且物理
            // GPU/CPU4 会话可用时，迟到的 Original ensure 必须幂等返回。
            effect_backend: effect_session_is_owned_by_cycle(
                self.status.backend,
                self.cycle_controller.is_some(),
                cycle_session_available,
            ),
            activation_healthy: matches!(
                self.status.activation,
                BackendActivation::Active | BackendActivation::Available
            ),
            lifecycle_healthy: matches!(
                self.status.lifecycle,
                RendererLifecycleState::Active | RendererLifecycleState::Spawned
            ),
            status_has_pid: self.status.process_id.is_some(),
            generation_matches: self.status.playback_generation == Some(playback_generation),
            session_matches: self
                .session
                .as_ref()
                .is_some_and(|session| session.playback_generation == playback_generation),
            process_running,
        })
    }

    fn ensure_original(
        &mut self,
        request: PrepareOriginalRenderer,
        operation_revision: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        if self.healthy_effect_session_for_generation(request.playback_generation) {
            return Ok(self.status.clone());
        }
        if request.clock_epoch == 0 {
            return Err(RealtimeVideoBackendError::ProcessFailed {
                operation: "Original 启动",
                message: "clock_epoch 必须大于 0".to_owned(),
            });
        }
        if !valid_source_position(request.source_start_ms, request.source_duration_ms) {
            return Err(RealtimeVideoBackendError::ProcessFailed {
                operation: "Original 启动",
                message: "Original 源时长必须大于 0，且启动位置必须位于源内".to_owned(),
            });
        }
        let canonical_source = request.source_path.canonicalize().map_err(|error| {
            RealtimeVideoBackendError::InvalidMediaPath {
                path: request.source_path.clone(),
                message: error.to_string(),
            }
        })?;
        let generation_transition = self.begin_generation(request.playback_generation)?;
        if request.backend_epoch != self.backend_epoch {
            return Err(stale_sync_error("Original 后端 epoch"));
        }
        if let Some(cursor) = self.sync_cursor {
            if request.playback_generation < cursor.playback_generation {
                return Err(stale_sync_error("播放代次"));
            }
            if request.playback_generation == cursor.playback_generation {
                resolve_sync_transition(
                    cursor,
                    RealtimeVideoSync {
                        playback_generation: request.playback_generation,
                        clock_epoch: request.clock_epoch,
                        loop_index: request.loop_index,
                        backend_epoch: request.backend_epoch,
                        position_ms: request.source_start_ms,
                        paused: request.paused,
                    },
                )?;
            }
        }
        self.session = Some(RendererSessionContext {
            executable: request.executable.clone(),
            shader: request.shader,
            session_id: request.playback_generation,
            source_path: canonical_source.clone(),
            host_window_id: request.host_window_id,
            playback_generation: request.playback_generation,
            clock_epoch: request.clock_epoch,
            loop_index: request.loop_index,
            source_position_ms: request.source_start_ms,
            source_duration_ms: request.source_duration_ms,
            paused: request.paused,
        });
        if matches!(self.backend.launch_mode(), MpvLaunchMode::Gpu(_))
            && self
                .session
                .as_ref()
                .is_some_and(|session| session.shader.is_none())
        {
            while matches!(self.backend.launch_mode(), MpvLaunchMode::Gpu(_)) {
                let support = self.status.parameter_support.clone();
                self.demote(
                    "GPU83 shader 资源缺失，按单向状态机降级",
                    unix_now_ms(),
                    support,
                );
            }
        }
        let requested_launch_mode = neutral_source_launch_mode(
            self.session
                .as_ref()
                .is_some_and(|session| session.shader.is_some()),
            self.backend.launch_mode(),
        );
        self.try_switch_preserved_process(
            generation_transition,
            &canonical_source,
            request.host_window_id,
            request.source_start_ms,
            request.source_duration_ms,
            request.paused,
            true,
            operation_revision,
        )?;
        let generation_changed = self
            .sync_cursor
            .is_some_and(|cursor| cursor.playback_generation != request.playback_generation);
        let must_restart = self.process.is_none()
            || self.process_launch_mode != Some(requested_launch_mode)
            || self.source_path.as_ref() != Some(&canonical_source)
            || renderer_host_window_changed(
                self.process.is_some(),
                self.process_host_window_id,
                request.host_window_id,
            )
            || generation_changed
            || self
                .process
                .as_mut()
                .is_some_and(|process| process.has_exited().unwrap_or(true));
        if must_restart {
            self.stop_process();
            if let Err(error) =
                self.start_neutral_process(requested_launch_mode, operation_revision)
            {
                if requested_launch_mode == MpvLaunchMode::Original {
                    self.record_original_failure(error.to_string());
                    return Err(error);
                }
                self.transition_to_fallback(
                    error.to_string(),
                    unix_now_ms(),
                    None,
                    false,
                    operation_revision,
                );
            }
        }
        if self.process.is_some() && self.status.backend != VideoBackend::Source {
            self.neutralize_to_source_with_fallback()?;
        }
        self.record_session_identity();
        Ok(self.status.clone())
    }

    fn commit(
        &mut self,
        request: &RealtimeVideoCommit,
        now_unix_ms: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        let Some(cursor) = self.sync_cursor else {
            return Err(stale_sync_error("提交时钟"));
        };
        if request.clock_epoch != cursor.clock_epoch
            || request.loop_index != cursor.loop_index
            || request.backend_epoch != self.backend_epoch
        {
            return Err(stale_sync_error("提交后端 epoch"));
        }
        let gate = &request.gate;
        if self.pending.is_none()
            && self.status.activation == BackendActivation::Active
            && self
                .current
                .as_ref()
                .is_some_and(|plan| plan.identity == gate.identity)
        {
            return Ok(self.status.clone());
        }
        let Some(mut plan) = self.pending.clone() else {
            return Err(stale_sync_error("待提交计划"));
        };
        match plan.commit_decision(gate) {
            VideoCommitDecision::Commit => {}
            decision => {
                return Err(RealtimeVideoBackendError::ProcessFailed {
                    operation: "计划提交",
                    message: format!("实时 N+1 提交门禁拒绝：{decision:?}"),
                });
            }
        }
        let drift = signed_delta(gate.media_pts_ms, plan.target_pts_ms);
        let context = self.pending_schedule.clone().ok_or_else(|| {
            RealtimeVideoBackendError::ProcessFailed {
                operation: "计划提交",
                message: "N+1 调度参数不存在".to_owned(),
            }
        })?;
        if self.process.is_none() {
            let fallback_plan = self
                .pending_schedule
                .as_ref()
                .map(|context| cpu4_plan_from(&plan, &context.video_params))
                .transpose()?;
            let operation_revision = self.operation_revision.load(Ordering::Acquire);
            self.transition_to_fallback(
                "参数提交时 mpv 会话不存在".to_owned(),
                now_unix_ms,
                fallback_plan,
                true,
                operation_revision,
            );
            return Ok(self.status.clone());
        }
        if plan.parameter_support.backend == VideoBackend::RealtimeGpu {
            let Some(actual_source_fps) = self.nominal_source_fps else {
                self.set_apply_state(VideoApplyState::SourceTransitioning);
                return Ok(self.status.clone());
            };
            let source_duration_ms = self
                .session
                .as_ref()
                .map(|session| session.source_duration_ms)
                .unwrap_or_default();
            let source_pts_ms = source_local_schedule_pts(
                gate.media_pts_ms,
                source_duration_ms,
                request.loop_index,
            )
            .ok_or_else(|| stale_sync_error("周期提交源内 PTS"))?;
            let decision = self
                .scheduler
                .decide(VideoFrameScheduleObservation {
                    identity: context.identity.clone(),
                    schedule_epoch: self.schedule_epoch,
                    media_pts_ms: source_pts_ms,
                    source_fps: actual_source_fps,
                    paused: context.paused,
                    boundary: self.pending_boundary,
                    seed: context.seed,
                    video_params: &context.video_params,
                    advanced_params: &context.advanced_params,
                })
                .map_err(|error| RealtimeVideoBackendError::ProcessFailed {
                    operation: "计划提交",
                    message: format!("周期静态调度快照无效：{error}"),
                })?;
            let schedule = decision
                .schedule
                .ok_or_else(|| stale_sync_error("周期静态调度快照"))?;
            let update = build_gpu83_scheduled_shader_update(
                &context.video_params,
                &context.advanced_params,
                &schedule,
            )
            .map_err(|error| RealtimeVideoBackendError::ProcessFailed {
                operation: "计划提交",
                message: format!("周期 GPU83 静态快照无效（{}:{}）", error.field, error.code),
            })?;
            let options = MpvShaderOptions::parse(update.value)?;
            let (options, _) = attach_shader_plan_fingerprint(
                &options,
                &plan.identity,
                context.clock_epoch,
                context.loop_index,
                actual_source_fps,
            )?;
            let mut shader_command_count = 0;
            for command in &mut plan.commands {
                if let MpvCommand::SetShaderOptions {
                    options: command_options,
                } = command
                {
                    *command_options = options.clone();
                    shader_command_count += 1;
                }
            }
            if shader_command_count != 1 {
                return Err(RealtimeVideoBackendError::ProcessFailed {
                    operation: "计划提交",
                    message: format!(
                        "GPU 周期必须且只能包含一条完整 shader 快照，实际为 {shader_command_count} 条"
                    ),
                });
            }
            self.scheduler_base_speed = schedule.base_video_speed;
            if decision.action == VideoScheduleAction::Apply {
                self.pending_boundary = VideoScheduleBoundary::None;
            }
        }
        if plan.parameter_support.backend == VideoBackend::RealtimeGpu {
            if self.pending_shader_apply.is_some() {
                return Ok(self.status.clone());
            }
            let expected = plan
                .commands
                .iter()
                .find_map(|command| match command {
                    MpvCommand::SetShaderOptions { options } => Some(options.clone()),
                    _ => None,
                })
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "计划提交",
                    message: "GPU 周期缺少 shader 快照".to_owned(),
                })?;
            let actual_source_fps = self.nominal_source_fps.ok_or_else(|| {
                RealtimeVideoBackendError::ProcessFailed {
                    operation: "计划提交",
                    message: "mpv 尚未确认当前源实际 FPS".to_owned(),
                }
            })?;
            let fingerprint = canonical_shader_plan_fingerprint(
                &expected,
                &plan.identity,
                context.clock_epoch,
                context.loop_index,
                actual_source_fps,
            )?;
            let source_duration_ms = self
                .session
                .as_ref()
                .map(|session| session.source_duration_ms)
                .filter(|duration| *duration > 0)
                .ok_or_else(|| stale_sync_error("shader 周期会话"))?;
            if let Some(controller) = self.cycle_controller.as_mut() {
                controller
                    .handle(VideoCycleEvent::ApplyResult {
                        sequence: plan.identity.sequence,
                        source_pts_ms: gate.media_pts_ms % source_duration_ms,
                        result: VideoCycleApplyResult::Applying {
                            fingerprint: fingerprint.clone(),
                        },
                    })
                    .map_err(video_cycle_runtime_error)?;
            }
            let response = self
                .process
                .as_ref()
                .ok_or_else(|| stale_sync_error("参数提交进程"))?
                .submit_shader_options(expected.clone())?;
            self.pending_shader_apply = Some(PendingShaderApply {
                response,
                expected,
                commit_media_pts_ms: gate.media_pts_ms,
                commit_source_pts_ms: gate.media_pts_ms % source_duration_ms,
                fingerprint: fingerprint.clone(),
                phase: PendingShaderApplyPhase::AwaitingResponse,
                submitted_at: Instant::now(),
                retry_count: 0,
            });
            self.status.pending_plan_fingerprint = Some(fingerprint);
            self.set_apply_state(VideoApplyState::Applying);
            return Ok(self.status.clone());
        }
        let actual_source_fps =
            self.nominal_source_fps
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "CPU4 计划提交",
                    message: "mpv 尚未确认当前源实际 FPS".to_owned(),
                })?;
        let fingerprint = canonical_cpu4_plan_fingerprint(
            &plan,
            context.clock_epoch,
            context.loop_index,
            actual_source_fps,
        )?;
        self.status.pending_plan_fingerprint = Some(fingerprint.clone());
        if let Some(controller) = self.cycle_controller.as_mut() {
            let source_duration_ms = self
                .session
                .as_ref()
                .map(|session| session.source_duration_ms)
                .filter(|duration| *duration > 0)
                .ok_or_else(|| stale_sync_error("CPU4 周期会话"))?;
            controller
                .handle(VideoCycleEvent::ApplyResult {
                    sequence: plan.identity.sequence,
                    source_pts_ms: gate.media_pts_ms % source_duration_ms,
                    result: VideoCycleApplyResult::Applying { fingerprint },
                })
                .map_err(video_cycle_runtime_error)?;
        }
        self.set_apply_state(VideoApplyState::Applying);
        for command in &plan.commands {
            let result = self
                .process
                .as_ref()
                .ok_or_else(|| stale_sync_error("参数提交进程"))?
                .send_command(command, MPV_COMMAND_TIMEOUT);
            if let Err(error) = result {
                let fallback_plan = self
                    .pending_schedule
                    .as_ref()
                    .map(|context| cpu4_plan_from(&plan, &context.video_params))
                    .transpose()?;
                let operation_revision = self.operation_revision.load(Ordering::Acquire);
                self.transition_to_fallback(
                    error.to_string(),
                    now_unix_ms,
                    fallback_plan,
                    true,
                    operation_revision,
                );
                return Ok(self.status.clone());
            }
        }
        self.finalize_pending_plan(plan, drift)
    }

    fn finalize_pending_plan(
        &mut self,
        mut plan: RealtimeVideoPlan,
        drift: i64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        self.begin_frame_budget_transition();
        self.last_shader_options = plan.commands.iter().find_map(|command| match command {
            MpvCommand::SetShaderOptions { options } => Some(options.clone()),
            _ => None,
        });
        let context = self.pending_schedule.take().ok_or_else(|| {
            RealtimeVideoBackendError::ProcessFailed {
                operation: "计划提交",
                message: "N+1 调度参数不存在".to_owned(),
            }
        })?;
        if self.sync_cursor.is_none() {
            self.reset_scheduler(
                context.identity.playback_generation,
                context.clock_epoch,
                context.loop_index,
                context.paused,
            );
        }
        self.claim_cycle_backend_status();
        if !matches!(
            self.status.backend,
            VideoBackend::RealtimeGpu | VideoBackend::Cpu4
        ) {
            return Err(stale_sync_error("周期物理后端"));
        }
        plan.parameter_support = self.status.parameter_support.clone();
        self.current_schedule = Some(context);
        self.current = Some(RealtimeVideoPlan {
            slot: VideoPlanSlot::N,
            ..plan.clone()
        });
        self.pending = None;
        self.status.activation = BackendActivation::Active;
        self.status.lifecycle = RendererLifecycleState::Active;
        self.status.n = Some(slot_status(&plan, CycleSlotState::Active));
        self.status.n1 = None;
        self.status.cycle_drift_ms = Some(drift);
        self.status.process_id = self.process.as_ref().and_then(ManagedMpvProcess::pid);
        self.status.active_plan_fingerprint = self.status.pending_plan_fingerprint.take();
        self.set_apply_state(VideoApplyState::Active);
        Ok(self.status.clone())
    }

    fn synchronize(
        &mut self,
        request: RealtimeVideoSync,
        operation_revision: u64,
        now_unix_ms: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        if operation_revision != self.operation_revision.load(Ordering::Acquire) {
            return Err(RealtimeVideoBackendError::SyncSuperseded {
                operation: "同步请求",
            });
        }
        let Some(cursor) = self.sync_cursor else {
            return Err(stale_sync_error("同步时钟"));
        };
        if request.backend_epoch != self.backend_epoch {
            return Err(stale_sync_error("同步后端 epoch"));
        }
        if self.process.is_none() {
            if self.backend.launch_mode() == MpvLaunchMode::Original {
                self.record_original_failure("播放同步时 Original mpv 会话不存在".to_owned());
            } else {
                let fallback_plan = self.current_cpu4_plan()?;
                let operation_revision = self.operation_revision.load(Ordering::Acquire);
                self.transition_to_fallback(
                    "播放同步时 mpv 会话不存在".to_owned(),
                    now_unix_ms,
                    fallback_plan,
                    true,
                    operation_revision,
                );
            }
            return Ok(self.status.clone());
        }
        let transition = resolve_sync_transition(cursor, request)?;
        let source_duration_ms = self
            .session
            .as_ref()
            .map(|session| session.source_duration_ms)
            .ok_or_else(|| stale_sync_error("媒体会话"))?;
        if !valid_source_position(request.position_ms, source_duration_ms) {
            return Err(RealtimeVideoBackendError::InvalidSync {
                message: "同步位置必须位于当前源媒体内".to_owned(),
            });
        }
        let commands = sync_command_batch(cursor, transition, request.position_ms);
        let next_schedule_epoch = if matches!(transition.boundary, VideoScheduleBoundary::None) {
            self.schedule_epoch
        } else {
            self.schedule_epoch.checked_add(1).ok_or_else(|| {
                RealtimeVideoBackendError::ProcessFailed {
                    operation: "播放同步",
                    message: "视频调度 epoch 已耗尽".to_owned(),
                }
            })?
        };
        let physical_position_ms =
            if matches!(transition.boundary, VideoScheduleBoundary::LoopBoundary)
                && !transition.next.paused
            {
                let Some(process) = self.process.as_ref() else {
                    return Err(stale_sync_error("同步进程"));
                };
                match process.resume_after_eof(
                    request.position_ms,
                    source_duration_ms,
                    EOF_RESUME_CONFIRMATION_DEADLINE,
                ) {
                    Ok(position_ms) => Some(position_ms),
                    Err(error @ RealtimeVideoBackendError::ProcessFailed { .. }) => {
                        return Err(error)
                    }
                    Err(error) if sync_command_result_is_unknown(&error) => return Err(error),
                    Err(error) => {
                        let fallback_plan = self.current_cpu4_plan().ok().flatten();
                        self.handle_process_failure(
                            error.to_string(),
                            now_unix_ms,
                            fallback_plan,
                            true,
                        );
                        return Ok(self.status.clone());
                    }
                }
            } else {
                for command in commands.into_iter().flatten() {
                    let Some(process) = self.process.as_ref() else {
                        return Err(stale_sync_error("同步进程"));
                    };
                    if let Err(error) = process.send_command(&command, MPV_COMMAND_TIMEOUT) {
                        if sync_command_result_is_unknown(&error) {
                            return Err(error);
                        }
                        let fallback_plan = self.current_cpu4_plan().ok().flatten();
                        self.handle_process_failure(
                            error.to_string(),
                            now_unix_ms,
                            fallback_plan,
                            true,
                        );
                        return Ok(self.status.clone());
                    }
                }
                None
            };
        if let Some(session) = self.session.as_mut() {
            session.clock_epoch = request.clock_epoch;
            session.loop_index = request.loop_index;
            session.source_position_ms = physical_position_ms.unwrap_or(request.position_ms);
            session.paused = request.paused;
        }
        if !matches!(transition.boundary, VideoScheduleBoundary::None) {
            self.discard_pending_outside_cursor(transition.next);
        }
        if !matches!(transition.boundary, VideoScheduleBoundary::None) {
            self.schedule_epoch = next_schedule_epoch;
            self.clear_eof_fact();
            self.pending_boundary =
                merge_pending_boundary(self.pending_boundary, transition.boundary);
            self.nominal_source_fps = None;
            self.begin_frame_budget_transition();
        }
        if !matches!(transition.boundary, VideoScheduleBoundary::None)
            || transition.next.paused != cursor.paused
        {
            self.presented_pts_watchdog.reset();
        }
        self.sync_cursor = Some(transition.next);
        if !matches!(transition.boundary, VideoScheduleBoundary::None) {
            self.reset_cycle_controller_for_boundary(
                transition.boundary,
                transition.next,
                physical_position_ms.unwrap_or(request.position_ms),
            )?;
        }
        if let Some(position_ms) = physical_position_ms {
            self.status.presented_pts_ms = Some(position_ms);
        }
        self.status.physical_paused = Some(request.paused);
        self.status.physical_eof_reached = Some(false);
        if transition.next.paused {
            self.clear_eof_fact();
        }
        self.record_session_identity();
        Ok(self.status.clone())
    }

    fn apply_playback_intent(
        &mut self,
        request: PlaybackIntentRequest,
        operation_revision: u64,
        now_unix_ms: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        let cursor = self
            .sync_cursor
            .ok_or_else(|| stale_sync_error("播放意图时钟"))?;
        if cursor.playback_generation != request.playback_generation {
            return Err(stale_sync_error("播放意图代次"));
        }
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| stale_sync_error("播放意图会话"))?;
        let (clock_epoch, position_ms, paused) = match request.intent {
            PlaybackIntent::Play => (cursor.clock_epoch, session.source_position_ms, false),
            PlaybackIntent::Pause => (cursor.clock_epoch, session.source_position_ms, true),
            PlaybackIntent::Seek { position_ms } => (
                cursor.clock_epoch.checked_add(1).ok_or_else(|| {
                    RealtimeVideoBackendError::InvalidSync {
                        message: "播放意图 clock_epoch 已耗尽".to_owned(),
                    }
                })?,
                position_ms,
                cursor.paused,
            ),
        };
        self.synchronize(
            RealtimeVideoSync {
                playback_generation: request.playback_generation,
                clock_epoch,
                loop_index: cursor.loop_index,
                backend_epoch: self.backend_epoch,
                position_ms,
                paused,
            },
            operation_revision,
            now_unix_ms,
        )
    }

    fn reset_cycle_controller_for_boundary(
        &mut self,
        boundary: VideoScheduleBoundary,
        cursor: SyncCursor,
        source_pts_ms: u64,
    ) -> Result<(), RealtimeVideoBackendError> {
        if self.cycle_controller.is_none() {
            return Ok(());
        }
        if self.processing_enabled && self.process.is_some() {
            self.last_shader_options = self.apply_neutral_process_parameters()?;
        }
        let backend_epoch = self.backend_epoch;
        let session = self
            .session
            .as_ref()
            .cloned()
            .ok_or_else(|| stale_sync_error("周期媒体会话"))?;
        let Some(controller) = self.cycle_controller.as_mut() else {
            return Ok(());
        };
        match boundary {
            VideoScheduleBoundary::UserSeek => {
                controller
                    .handle(VideoCycleEvent::PlaybackIntent {
                        paused: cursor.paused,
                        seek_source_pts_ms: Some(source_pts_ms),
                        clock_epoch: cursor.clock_epoch,
                    })
                    .map_err(video_cycle_runtime_error)?;
            }
            VideoScheduleBoundary::Startup
            | VideoScheduleBoundary::LoopBoundary
            | VideoScheduleBoundary::SourceChanged => {
                let media = MediaSegmentIdentity::try_new(
                    cursor.playback_generation,
                    session.source_path.clone(),
                    cursor.loop_index,
                    session.source_duration_ms,
                )
                .map_err(|error| RealtimeVideoBackendError::ProcessFailed {
                    operation: "周期边界重置",
                    message: format!("媒体段身份无效：{error:?}"),
                })?;
                let identity = VideoCycleSegmentIdentity::try_new(
                    session.session_id,
                    backend_epoch,
                    cursor.clock_epoch,
                    media,
                )
                .map_err(video_cycle_runtime_error)?;
                controller
                    .handle(VideoCycleEvent::SourceBoundary {
                        identity,
                        source_fps: None,
                        source_pts_ms,
                    })
                    .map_err(video_cycle_runtime_error)?;
            }
            VideoScheduleBoundary::None => return Ok(()),
        }
        self.pending = None;
        self.current = None;
        self.pending_schedule = None;
        self.current_schedule = None;
        self.begin_source_transition();
        self.prepare_next_controller_plan()
    }

    fn advance_after_eof(
        &mut self,
        request: AdvanceRealtimeVideoAfterEof,
        operation_revision: u64,
        now_unix_ms: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        self.ensure_operation_current(operation_revision, "EOF 播放闭环")?;
        let eof = self
            .status
            .eof
            .ok_or_else(|| stale_sync_error("EOF 事实"))?;
        if eof != request.expected_eof || eof.backend_epoch != self.backend_epoch {
            return Err(stale_sync_error("EOF 身份"));
        }
        let session = self
            .session
            .as_ref()
            .filter(|session| {
                session.playback_generation == eof.playback_generation
                    && session.clock_epoch == eof.clock_epoch
                    && session.loop_index == eof.loop_index
            })
            .cloned()
            .ok_or_else(|| stale_sync_error("EOF 媒体会话"))?;
        if request.paused || request.next_source_duration_ms == 0 {
            return Err(RealtimeVideoBackendError::InvalidSync {
                message: "EOF 完成后必须恢复播放且新源时长必须大于 0".to_owned(),
            });
        }
        let canonical_source = request.next_source_path.canonicalize().map_err(|error| {
            RealtimeVideoBackendError::InvalidMediaPath {
                path: request.next_source_path.clone(),
                message: error.to_string(),
            }
        })?;

        if request.next_playback_generation == eof.playback_generation {
            if request.next_loop_index != eof.loop_index.saturating_add(1)
                || canonical_source != session.source_path
                || request.next_source_duration_ms != session.source_duration_ms
            {
                return Err(RealtimeVideoBackendError::InvalidSync {
                    message: "单源 EOF 完成只能推进当前源的下一循环".to_owned(),
                });
            }
            // EOF 后没有下一真实帧可供本轮 shader 晋级；允许循环边界丢弃该事务，
            // 否则 AdvanceAfterEof 会永远排在 AwaitingPresentation 后面。
            self.pending_shader_apply = None;
            return self.synchronize(
                RealtimeVideoSync {
                    playback_generation: request.next_playback_generation,
                    clock_epoch: eof.clock_epoch,
                    loop_index: request.next_loop_index,
                    backend_epoch: eof.backend_epoch,
                    position_ms: 0,
                    paused: false,
                },
                operation_revision,
                now_unix_ms,
            );
        }

        if request.next_playback_generation != eof.playback_generation.saturating_add(1)
            || request.next_loop_index != 0
        {
            return Err(RealtimeVideoBackendError::InvalidSync {
                message: "多源 EOF 完成只能进入紧邻的新播放代次".to_owned(),
            });
        }
        if canonical_source == session.source_path {
            return Err(RealtimeVideoBackendError::InvalidSync {
                message: "多源 EOF 完成的新媒体路径必须发生变化".to_owned(),
            });
        }
        self.pending_shader_apply = None;
        let next_clock_epoch = eof.clock_epoch.saturating_add(1);
        let transition = self.begin_generation(request.next_playback_generation)?;
        self.session = Some(RendererSessionContext {
            executable: session.executable,
            shader: session.shader,
            session_id: request.next_playback_generation,
            source_path: canonical_source.clone(),
            host_window_id: session.host_window_id,
            playback_generation: request.next_playback_generation,
            clock_epoch: next_clock_epoch,
            loop_index: request.next_loop_index,
            source_position_ms: 0,
            source_duration_ms: request.next_source_duration_ms,
            paused: false,
        });
        let switched = self.try_switch_preserved_process(
            transition,
            &canonical_source,
            session.host_window_id,
            0,
            request.next_source_duration_ms,
            false,
            true,
            operation_revision,
        )?;
        if !switched {
            return Err(RealtimeVideoBackendError::ProcessFailed {
                operation: "EOF 多源换源",
                message: "受管 mpv 未能在原进程内完成换源与物理播放确认".to_owned(),
            });
        }
        if self.cycle_controller.is_some() {
            let media = MediaSegmentIdentity::try_new(
                request.next_playback_generation,
                canonical_source.clone(),
                request.next_loop_index,
                request.next_source_duration_ms,
            )
            .map_err(|error| RealtimeVideoBackendError::ProcessFailed {
                operation: "EOF 周期换源",
                message: format!("媒体段身份无效：{error:?}"),
            })?;
            let identity = VideoCycleSegmentIdentity::try_new(
                request.next_playback_generation,
                self.backend_epoch,
                next_clock_epoch,
                media,
            )
            .map_err(video_cycle_runtime_error)?;
            if let Some(controller) = self.cycle_controller.as_mut() {
                controller
                    .handle(VideoCycleEvent::SourceBoundary {
                        identity,
                        source_fps: None,
                        source_pts_ms: 0,
                    })
                    .map_err(video_cycle_runtime_error)?;
            }
            self.pending = None;
            self.current = None;
            self.pending_schedule = None;
            self.current_schedule = None;
            self.begin_source_transition();
        }
        self.pending_boundary = VideoScheduleBoundary::SourceChanged;
        self.clear_eof_fact();
        self.record_session_identity();
        Ok(self.status.clone())
    }

    fn validate_advance_after_eof_identity(
        &self,
        request: &AdvanceRealtimeVideoAfterEof,
        operation_revision: u64,
    ) -> Result<(), RealtimeVideoBackendError> {
        self.ensure_operation_current(operation_revision, "EOF 播放闭环")?;
        let eof = self
            .status
            .eof
            .ok_or_else(|| stale_sync_error("EOF 事实"))?;
        if eof != request.expected_eof || eof.backend_epoch != self.backend_epoch {
            return Err(stale_sync_error("EOF 身份"));
        }
        let _session = self
            .session
            .as_ref()
            .filter(|session| {
                session.playback_generation == eof.playback_generation
                    && session.clock_epoch == eof.clock_epoch
                    && session.loop_index == eof.loop_index
            })
            .ok_or_else(|| stale_sync_error("EOF 媒体会话"))?;
        if request.paused || request.next_source_duration_ms == 0 {
            return Err(RealtimeVideoBackendError::InvalidSync {
                message: "EOF 完成后必须恢复播放且新源时长必须大于 0".to_owned(),
            });
        }
        if request.next_playback_generation == eof.playback_generation {
            if request.next_loop_index != eof.loop_index.saturating_add(1) {
                return Err(RealtimeVideoBackendError::InvalidSync {
                    message: "单源 EOF 完成只能推进当前源的下一循环".to_owned(),
                });
            }
        } else if request.next_playback_generation != eof.playback_generation.saturating_add(1)
            || request.next_loop_index != 0
        {
            return Err(RealtimeVideoBackendError::InvalidSync {
                message: "多源 EOF 完成只能进入紧邻的新播放代次".to_owned(),
            });
        }
        Ok(())
    }

    fn discard_pending_outside_cursor(&mut self, cursor: SyncCursor) {
        let pending_matches_cursor = self
            .pending
            .as_ref()
            .zip(self.pending_schedule.as_ref())
            .is_some_and(|(plan, context)| {
                plan.identity == context.identity
                    && context.identity.playback_generation == cursor.playback_generation
                    && context.clock_epoch == cursor.clock_epoch
                    && context.loop_index == cursor.loop_index
            });
        if pending_matches_cursor {
            return;
        }
        self.pending = None;
        self.pending_schedule = None;
        self.status.n1 = None;
        self.status.n2 = None;
    }

    fn reset_av_sync(&mut self) {
        self.av_sync_correction = 1.0;
        self.av_sync_audio_identity = None;
        self.av_sync_controller = AvSyncController::default();
        self.clear_published_av_sync_status();
    }

    fn clear_published_av_sync_status(&mut self) {
        self.status.av_sync_drift_ms = None;
        self.status.audible_audio_pts_ms = None;
        self.status.audio_epoch = None;
    }

    fn mark_cycle_status_available(&mut self) {
        self.clear_published_av_sync_status();
        self.status.activation = BackendActivation::Available;
        self.status.lifecycle = RendererLifecycleState::Spawned;
    }

    fn publish_av_sync_status(
        &mut self,
        drift_ms: Option<i64>,
        audible_audio_pts_ms: u64,
        audio_epoch: u64,
    ) {
        let Some(drift_ms) = drift_ms else {
            self.clear_published_av_sync_status();
            return;
        };
        self.status.av_sync_drift_ms = Some(drift_ms);
        self.status.audible_audio_pts_ms = Some(audible_audio_pts_ms);
        self.status.audio_epoch = Some(audio_epoch);
    }

    fn apply_playback_speed(
        &mut self,
        audio_playback_rate: f64,
        scheduler_base_speed: f64,
        av_sync_correction: f64,
        deadline: Duration,
    ) -> Result<bool, RealtimeVideoBackendError> {
        let Some(speed) = final_playback_speed(
            audio_playback_rate,
            scheduler_base_speed,
            av_sync_correction,
        ) else {
            return Ok(false);
        };
        if self.last_playback_speed == Some(speed) {
            return Ok(false);
        }
        let Some(process) = self.process.as_ref() else {
            return Err(RealtimeVideoBackendError::ProcessFailed {
                operation: "音画同步",
                message: "mpv 进程不存在".to_owned(),
            });
        };
        process.send_command(&MpvCommand::SetPlaybackSpeed { speed }, deadline)?;
        self.last_playback_speed = Some(speed);
        Ok(true)
    }

    fn observe_av_sync(
        &mut self,
        mpv_source_pts_ms: u64,
        playback_state: MpvPlaybackState,
        budget: &ObservationTickBudget,
    ) -> Result<bool, RealtimeVideoBackendError> {
        let (Some(session), Some(cursor)) = (self.session.as_ref(), self.sync_cursor) else {
            self.reset_av_sync();
            return self.apply_playback_speed(
                1.0,
                self.scheduler_base_speed,
                1.0,
                budget.remaining(),
            );
        };
        let session_source_duration_ms = session.source_duration_ms;
        if session.playback_generation != cursor.playback_generation
            || session.loop_index != cursor.loop_index
        {
            self.reset_av_sync();
            return self.apply_playback_speed(
                1.0,
                self.scheduler_base_speed,
                1.0,
                budget.remaining(),
            );
        }
        let Ok(expected) = MediaSegmentIdentity::try_new(
            session.playback_generation,
            session.source_path.clone(),
            session.loop_index,
            session_source_duration_ms,
        ) else {
            self.reset_av_sync();
            return self.apply_playback_speed(
                1.0,
                self.scheduler_base_speed,
                1.0,
                budget.remaining(),
            );
        };
        let now = Instant::now();
        let Some(snapshot) = self
            .audible_audio_clock
            .snapshot_for_segment(&expected, now)
        else {
            if self.av_sync_audio_identity.is_some() {
                let observed_at_ms = now
                    .checked_duration_since(self.av_sync_monotonic_origin)
                    .map(|elapsed| elapsed.as_millis().min(u128::from(u64::MAX)) as u64)
                    .unwrap_or(0);
                let _ = self.av_sync_controller.decide(AvSyncObservation {
                    playback_generation: session.playback_generation,
                    sync_epoch: self.av_sync_control_epoch,
                    observed_at_ms,
                    audible_source_pts_ms: None,
                    mpv_source_pts_ms: i64::try_from(mpv_source_pts_ms).ok(),
                    playing: !cursor.paused,
                    buffering: playback_state.paused_for_cache,
                    seeking: playback_state.seeking,
                    boundary: SyncBoundary::None,
                });
            }
            self.av_sync_correction = 1.0;
            self.status.av_sync_drift_ms = None;
            self.status.audible_audio_pts_ms = None;
            self.status.audio_epoch = None;
            return self.apply_playback_speed(
                1.0,
                self.scheduler_base_speed,
                1.0,
                budget.remaining(),
            );
        };
        let identity_changed = self.av_sync_audio_identity.as_ref() != Some(&snapshot.identity);
        if identity_changed {
            self.av_sync_control_epoch =
                self.av_sync_control_epoch.checked_add(1).ok_or_else(|| {
                    RealtimeVideoBackendError::ProcessFailed {
                        operation: "音画同步",
                        message: "音画同步 epoch 已耗尽".to_owned(),
                    }
                })?;
            self.av_sync_audio_identity = Some(snapshot.identity.clone());
        }
        let observed_at_ms = snapshot
            .observed_at
            .checked_duration_since(self.av_sync_monotonic_origin)
            .map(|elapsed| elapsed.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0);
        let boundary = if identity_changed {
            sync_boundary_from_audio(snapshot.boundary)
        } else {
            SyncBoundary::None
        };
        let mut decision = self.av_sync_controller.decide(AvSyncObservation {
            playback_generation: snapshot.identity.playback_generation,
            sync_epoch: self.av_sync_control_epoch,
            observed_at_ms,
            audible_source_pts_ms: i64::try_from(snapshot.audible_source_pts_ms).ok(),
            mpv_source_pts_ms: i64::try_from(mpv_source_pts_ms).ok(),
            playing: snapshot.playing && !cursor.paused,
            buffering: playback_state.paused_for_cache,
            seeking: playback_state.seeking,
            boundary,
        });
        if matches!(decision.action, AvSyncAction::RequestRecovery { .. }) {
            self.av_sync_control_epoch =
                self.av_sync_control_epoch.checked_add(1).ok_or_else(|| {
                    RealtimeVideoBackendError::ProcessFailed {
                        operation: "音画同步恢复",
                        message: "音画同步恢复 epoch 已耗尽".to_owned(),
                    }
                })?;
            decision = self.av_sync_controller.decide(AvSyncObservation {
                playback_generation: snapshot.identity.playback_generation,
                sync_epoch: self.av_sync_control_epoch,
                observed_at_ms,
                audible_source_pts_ms: i64::try_from(snapshot.audible_source_pts_ms).ok(),
                mpv_source_pts_ms: i64::try_from(mpv_source_pts_ms).ok(),
                playing: snapshot.playing && !cursor.paused,
                buffering: playback_state.paused_for_cache,
                seeking: playback_state.seeking,
                boundary: SyncBoundary::Recovery,
            });
        }

        self.publish_av_sync_status(
            decision.drift_ms,
            snapshot.audible_presentation_pts_ms,
            snapshot.identity.audio_epoch,
        );
        let mut changed = false;
        if let AvSyncAction::HardSeek {
            target_source_pts_ms,
            ..
        } = decision.action
        {
            let Ok(target) = u64::try_from(target_source_pts_ms) else {
                self.reset_av_sync();
                return self.apply_playback_speed(
                    snapshot.playback_rate,
                    self.scheduler_base_speed,
                    1.0,
                    budget.remaining(),
                );
            };
            if target >= session_source_duration_ms {
                self.reset_av_sync();
                return self.apply_playback_speed(
                    snapshot.playback_rate,
                    self.scheduler_base_speed,
                    1.0,
                    budget.remaining(),
                );
            }
            {
                let Some(process) = self.process.as_ref() else {
                    return Err(RealtimeVideoBackendError::ProcessFailed {
                        operation: "音画同步定位",
                        message: "mpv 进程不存在".to_owned(),
                    });
                };
                process.send_command(
                    &MpvCommand::SeekAbsoluteMs {
                        position_ms: target,
                    },
                    budget.remaining(),
                )?;
            }
            // mpv 已经物理跳到新的源内位置；必须同步重置周期控制器的观察
            // 高水位，否则下一 tick 会把本次 Rust 主动纠偏误判为自然 PTS 回退。
            if let Some(controller) = self.cycle_controller.as_mut() {
                controller
                    .rebase_source_pts(target)
                    .map_err(video_cycle_runtime_error)?;
            }
            self.begin_frame_budget_transition();
            if !cursor.paused {
                let Some(process) = self.process.as_ref() else {
                    return Err(RealtimeVideoBackendError::ProcessFailed {
                        operation: "音画同步恢复",
                        message: "mpv 进程不存在".to_owned(),
                    });
                };
                process
                    .send_command(&MpvCommand::SetPause { paused: false }, budget.remaining())?;
            }
            self.presented_pts_watchdog.reset();
            changed = true;
        }
        self.av_sync_correction = if matches!(decision.action, AvSyncAction::AdjustSpeed) {
            decision.speed
        } else {
            1.0
        };
        Ok(self.apply_playback_speed(
            snapshot.playback_rate,
            self.scheduler_base_speed,
            self.av_sync_correction,
            budget.remaining(),
        )? || changed)
    }

    fn tick(&mut self) -> bool {
        if let Some(reason) = self
            .process
            .as_ref()
            .and_then(ManagedMpvProcess::fatal_render_failure)
        {
            let fallback_plan = self.current_cpu4_plan().ok().flatten();
            self.handle_process_failure(reason, unix_now_ms(), fallback_plan, true);
            return true;
        }
        let pending_shader_phase = self.pending_shader_apply.as_ref().map(|apply| apply.phase);
        if let Some(result) = self.poll_pending_shader_apply() {
            match result {
                Ok(true) => return true,
                Ok(false)
                    if pending_shader_phase == Some(PendingShaderApplyPhase::AwaitingResponse) =>
                {
                    return false;
                }
                Ok(false) => {}
                Err(error @ RealtimeVideoBackendError::IpcTimeout { .. }) => {
                    if self.rebuild_same_gpu_after_shader_timeout().is_ok() {
                        return true;
                    }
                    let fallback_plan = self.current_cpu4_plan().ok().flatten();
                    self.handle_process_failure(
                        format!("shader 硬超时且同后端重建失败：{error}"),
                        unix_now_ms(),
                        fallback_plan,
                        true,
                    );
                    return true;
                }
                Err(error) => {
                    let fallback_plan = self.current_cpu4_plan().ok().flatten();
                    self.handle_process_failure(
                        format!("shader 提交确认失败：{error}"),
                        unix_now_ms(),
                        fallback_plan,
                        true,
                    );
                    return true;
                }
            }
        }
        let budget = ObservationTickBudget::start();
        if self.cycle_controller.is_some()
            && cycle_session_is_available(
                self.processing_enabled,
                self.process.is_some(),
                self.session.is_some(),
                self.process_launch_mode,
            )
        {
            // 逻辑 Source 表示物理效果已经无法确认，必须 fail-closed 清除旧槽位；
            // 单纯的 support/backend 漂移只修复元数据，不能打断已确认或在途提交。
            if self.status.backend == VideoBackend::Source {
                self.claim_cycle_backend_status();
                self.invalidate_cycle_before_source_recovery();
            } else if self.status.parameter_support.backend != self.status.backend {
                self.claim_cycle_backend_status();
            }
        }
        if self.status.backend == VideoBackend::Source
            && self.status.lifecycle == RendererLifecycleState::Active
            && self.cycle_controller.is_none()
        {
            let backend_label = match self.process_launch_mode {
                Some(MpvLaunchMode::Gpu(_)) => "GPU 中性旁路",
                Some(MpvLaunchMode::Cpu4) => "CPU4 中性旁路",
                Some(MpvLaunchMode::Original) | None => "Original",
            };
            let failure = match self.process.as_mut() {
                Some(process) => match process.poll_exit_evidence() {
                    Ok(Some(evidence)) => Some(unexpected_mpv_exit_reason(
                        backend_label,
                        Some(&evidence),
                        None,
                    )),
                    Ok(None) => None,
                    Err(error) => Some(unexpected_mpv_exit_reason(
                        backend_label,
                        None,
                        Some(&error),
                    )),
                },
                None => Some(format!("{backend_label} mpv 进程不存在")),
            };
            if let Some(reason) = failure {
                self.handle_process_failure(reason, unix_now_ms(), None, false);
                return true;
            }
            let observation = match self.poll_playback_observation() {
                Ok(Some(observation)) => observation,
                Ok(None) => return false,
                Err(error) => return self.record_source_observation_failure(error),
            };
            let eof_reached = observation.eof_reached;
            let playback_state = observation.playback_state;
            let eof_changed = self.record_eof_observation(eof_reached, playback_state.paused);
            if eof_reached {
                self.record_observation_success();
                return eof_changed;
            }
            let Some(observation) = observation.presented_pts_ms else {
                return eof_changed
                    || self.record_source_observation_failure(missing_video_pts_error());
            };
            if self.observe_presented_pts_liveness(observation, eof_reached, &budget) {
                return true;
            }
            let presented_pts_changed = self.record_presented_pts(observation);
            let sync_changed = match self.observe_av_sync(observation, playback_state, &budget) {
                Ok(changed) => changed,
                Err(error) => {
                    return presented_pts_changed
                        || eof_changed
                        || self.record_source_observation_failure(error);
                }
            };
            self.record_observation_success();
            return presented_pts_changed || sync_changed || eof_changed;
        }
        if self.cycle_controller.is_some()
            && self.process.is_some()
            && self.sync_cursor.is_some_and(|cursor| !cursor.paused)
            && self.nominal_source_fps.is_none()
            && matches!(
                self.status.backend,
                VideoBackend::RealtimeGpu | VideoBackend::Cpu4
            )
            && matches!(
                self.status.activation,
                BackendActivation::Available | BackendActivation::Active
            )
        {
            if self.source_transition_started_at.is_none() {
                self.source_transition_started_at = Some(Instant::now());
            }
            self.status.actual_source_fps = None;
            self.set_apply_state(VideoApplyState::SourceTransitioning);
            return match self.poll_source_fps_response() {
                Ok(changed) => changed,
                Err(error @ RealtimeVideoBackendError::IpcTimeout { .. }) => {
                    if self.rebuild_same_backend_after_source_fps_timeout().is_ok() {
                        true
                    } else {
                        self.record_tick_failure(error)
                    }
                }
                Err(error) => self.record_tick_failure(error),
            };
        }
        if self.status.activation == BackendActivation::Available
            && self.process.is_some()
            && self.sync_cursor.is_some_and(|cursor| !cursor.paused)
        {
            let observation = match self.poll_playback_observation() {
                Ok(Some(observation)) => observation,
                Ok(None) => return false,
                Err(error) => return self.record_tick_failure(error),
            };
            let eof_reached = observation.eof_reached;
            let eof_changed =
                self.record_eof_observation(eof_reached, observation.playback_state.paused);
            if eof_reached {
                self.record_observation_success();
                return eof_changed;
            }
            let Some(media_pts_ms) = observation.presented_pts_ms else {
                return eof_changed || self.record_tick_failure(missing_video_observation_error());
            };
            let observation = MpvVideoObservation {
                media_pts_ms,
                source_fps: self.nominal_source_fps.unwrap_or_default(),
            };
            if self.observe_presented_pts_liveness(observation.media_pts_ms, eof_reached, &budget) {
                return true;
            }
            let presented_pts_changed =
                self.record_video_observation(observation.media_pts_ms, observation.source_fps);
            if let Err(error) = self.initialize_cycle_after_source_observation(
                observation.media_pts_ms,
                observation.source_fps,
            ) {
                return self.record_tick_failure(error);
            }
            if let Some(result) = self.commit_pending_if_due(observation.media_pts_ms) {
                return match result {
                    Ok(_) => {
                        self.record_observation_success();
                        true
                    }
                    Err(error) => self.record_tick_failure(error),
                };
            }
            self.record_observation_success();
            return presented_pts_changed || eof_changed;
        }
        if self.status.backend == VideoBackend::Cpu4
            && self.status.activation == BackendActivation::Active
            && self.sync_cursor.is_some_and(|cursor| !cursor.paused)
        {
            if self.process.is_none() {
                return self.record_tick_failure(RealtimeVideoBackendError::ProcessFailed {
                    operation: "CPU4 观察",
                    message: "CPU4 mpv 进程不存在".to_owned(),
                });
            }
            let observation = match self.poll_playback_observation() {
                Ok(Some(observation)) => observation,
                Ok(None) => return false,
                Err(error) => return self.record_tick_failure(error),
            };
            let eof_reached = observation.eof_reached;
            let playback_state = observation.playback_state;
            let eof_changed = self.record_eof_observation(eof_reached, playback_state.paused);
            if eof_reached {
                self.record_observation_success();
                return eof_changed;
            }
            return match observation.presented_pts_ms {
                Some(media_pts_ms) => {
                    let observation = MpvVideoObservation {
                        media_pts_ms,
                        source_fps: self.nominal_source_fps.unwrap_or_default(),
                    };
                    if self.observe_presented_pts_liveness(
                        observation.media_pts_ms,
                        eof_reached,
                        &budget,
                    ) {
                        return true;
                    }
                    let presented_pts_changed = self
                        .record_video_observation(observation.media_pts_ms, observation.source_fps);
                    if let Some(result) = self.commit_pending_if_due(observation.media_pts_ms) {
                        return match result {
                            Ok(_) => {
                                self.record_observation_success();
                                true
                            }
                            Err(error) => self.record_tick_failure(error),
                        };
                    }
                    self.nominal_source_fps
                        .get_or_insert(observation.source_fps);
                    let sync_changed = match self.observe_av_sync(
                        observation.media_pts_ms,
                        playback_state,
                        &budget,
                    ) {
                        Ok(changed) => changed,
                        Err(error) => return self.record_tick_failure(error),
                    };
                    self.record_observation_success();
                    self.observe_frame_budget(
                        observation.media_pts_ms,
                        eof_reached,
                        playback_state,
                        &budget,
                    ) || presented_pts_changed
                        || sync_changed
                        || eof_changed
                }
                None => eof_changed || self.record_tick_failure(missing_video_observation_error()),
            };
        }
        if self.status.backend != VideoBackend::RealtimeGpu
            || self.status.activation != BackendActivation::Active
            || self.current_schedule.is_none()
            || self.sync_cursor.is_none_or(|cursor| cursor.paused)
        {
            return false;
        }
        if self.process.is_none() {
            return self.record_tick_failure(RealtimeVideoBackendError::ProcessFailed {
                operation: "GPU 观察",
                message: "GPU mpv 进程不存在".to_owned(),
            });
        }
        let observation = match self.poll_playback_observation() {
            Ok(Some(observation)) => observation,
            Ok(None) => return false,
            Err(error) => return self.record_tick_failure(error),
        };
        let eof_reached = observation.eof_reached;
        let playback_state = observation.playback_state;
        let eof_changed = self.record_eof_observation(eof_reached, playback_state.paused);
        if eof_reached {
            self.record_observation_success();
            return eof_changed;
        }
        let Some(media_pts_ms) = observation.presented_pts_ms else {
            return eof_changed || self.record_tick_failure(missing_video_observation_error());
        };
        let observation = MpvVideoObservation {
            media_pts_ms,
            source_fps: self.nominal_source_fps.unwrap_or_default(),
        };
        if self.observe_presented_pts_liveness(observation.media_pts_ms, eof_reached, &budget) {
            return true;
        }
        let presented_pts_changed =
            self.record_video_observation(observation.media_pts_ms, observation.source_fps);
        if let Some(result) = self.commit_pending_if_due(observation.media_pts_ms) {
            return match result {
                Ok(_) => {
                    self.record_observation_success();
                    true
                }
                Err(error) => self.record_tick_failure(error),
            };
        }
        let nominal_source_fps = *self
            .nominal_source_fps
            .get_or_insert(observation.source_fps);
        if self.observe_frame_budget(
            observation.media_pts_ms,
            eof_reached,
            playback_state,
            &budget,
        ) {
            return true;
        }
        let Some(context) = self.current_schedule.clone() else {
            return presented_pts_changed || eof_changed;
        };
        let decision = match self.scheduler.decide(VideoFrameScheduleObservation {
            identity: context.identity.clone(),
            schedule_epoch: self.schedule_epoch,
            media_pts_ms: observation.media_pts_ms,
            source_fps: nominal_source_fps,
            paused: false,
            boundary: self.pending_boundary,
            seed: context.seed,
            video_params: &context.video_params,
            advanced_params: &context.advanced_params,
        }) {
            Ok(decision) => decision,
            Err(_) => return presented_pts_changed || eof_changed,
        };
        let mut changed = presented_pts_changed || eof_changed;
        if let Some(schedule) = decision.schedule {
            self.scheduler_base_speed = schedule.base_video_speed;
        }
        if decision.action == VideoScheduleAction::Apply {
            self.pending_boundary = VideoScheduleBoundary::None;
        }
        match self.observe_av_sync(observation.media_pts_ms, playback_state, &budget) {
            Ok(sync_changed) => changed |= sync_changed,
            Err(error) => return self.record_tick_failure(error),
        }
        self.record_observation_success();
        changed
    }

    fn commit_pending_if_due(
        &mut self,
        presented_pts_ms: u64,
    ) -> Option<Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError>> {
        if self.nominal_source_fps.is_none() {
            self.set_apply_state(VideoApplyState::SourceTransitioning);
            return None;
        }
        let shader_apply_in_flight = self.pending_shader_apply.is_some();
        if let Some(controller) = self.cycle_controller.as_mut() {
            let due = match controller.handle(VideoCycleEvent::MpvObservation {
                source_pts_ms: presented_pts_ms,
                source_fps: self.status.actual_source_fps,
                paused: false,
            }) {
                Ok(actions) => actions.into_iter().find_map(|action| match action {
                    VideoCycleAction::Apply(plan) => Some(*plan),
                    _ => None,
                }),
                Err(error) => return Some(Err(video_cycle_runtime_error(error))),
            };
            if shader_apply_in_flight {
                // `self.pending` 必须继续指向当前异步确认事务。即使 PTS 已越过 N+2，
                // 也不能在迟到回读完成前用更新计划覆盖它。
                return None;
            }
            let due = due?;
            let pending_sequence = self.pending.as_ref().map(|plan| plan.identity.sequence);
            if pending_sequence != Some(due.sequence) {
                let (mut plan, context) = match self.compile_controller_plan(&due) {
                    Ok(compiled) => compiled,
                    Err(error) => return Some(Err(error)),
                };
                if self.status.backend == VideoBackend::Cpu4 {
                    plan = match cpu4_plan_from(&plan, &due.video_params) {
                        Ok(plan) => plan,
                        Err(error) => return Some(Err(error)),
                    };
                }
                self.pending = Some(plan.clone());
                self.pending_schedule = Some(context);
                self.status.n1 = Some(slot_status(&plan, CycleSlotState::Ready));
            }
        }
        let source_duration_ms = self.session.as_ref()?.source_duration_ms;
        let commit = build_due_realtime_video_commit(
            self.pending.as_ref(),
            self.sync_cursor,
            self.backend_epoch,
            source_duration_ms,
            presented_pts_ms,
        )?;
        Some(self.commit(&commit, unix_now_ms()).and_then(|_| {
            if self.pending_shader_apply.is_some() {
                return Ok(self.status.clone());
            }
            if let Some(controller) = self.cycle_controller.as_mut() {
                controller
                    .handle(VideoCycleEvent::ApplyResult {
                        sequence: commit.gate.identity.sequence,
                        source_pts_ms: presented_pts_ms,
                        result: VideoCycleApplyResult::Active,
                    })
                    .map_err(video_cycle_runtime_error)?;
                self.status.confirmed_change_count = controller.confirmed_change_count();
                self.prepare_next_controller_plan()?;
            }
            Ok(self.status.clone())
        }))
    }

    fn poll_pending_shader_apply(&mut self) -> Option<Result<bool, RealtimeVideoBackendError>> {
        let phase = self.pending_shader_apply.as_ref()?.phase;
        if phase == PendingShaderApplyPhase::AwaitingPresentation {
            let crossed = self
                .status
                .presented_pts_ms
                .zip(
                    self.pending_shader_apply
                        .as_ref()
                        .map(|apply| apply.commit_source_pts_ms),
                )
                .is_some_and(|(presented, submitted)| presented > submitted);
            if !crossed {
                return Some(Ok(false));
            }
            if let Some(apply) = self.pending_shader_apply.as_mut() {
                apply.phase = PendingShaderApplyPhase::PresentedConfirmed;
            }
            if let (Some(controller), Some(apply), Some(plan)) = (
                self.cycle_controller.as_mut(),
                self.pending_shader_apply.as_ref(),
                self.pending.as_ref(),
            ) {
                if let Err(error) = controller.handle(VideoCycleEvent::ApplyResult {
                    sequence: plan.identity.sequence,
                    source_pts_ms: apply.commit_source_pts_ms,
                    result: VideoCycleApplyResult::PresentedConfirmed,
                }) {
                    return Some(Err(video_cycle_runtime_error(error)));
                }
            }
            self.set_apply_state(VideoApplyState::PresentedConfirmed);
            return Some(Ok(true));
        }
        if phase == PendingShaderApplyPhase::PresentedConfirmed {
            let apply = self.pending_shader_apply.take()?;
            let plan = match self.pending.clone() {
                Some(plan) => plan,
                None => return Some(Err(stale_sync_error("shader 待提交计划"))),
            };
            let sequence = plan.identity.sequence;
            let drift = signed_delta(apply.commit_media_pts_ms, plan.target_pts_ms);
            if let Err(error) = self.finalize_pending_plan(plan, drift) {
                return Some(Err(error));
            }
            if let Some(controller) = self.cycle_controller.as_mut() {
                if let Err(error) = controller.handle(VideoCycleEvent::ApplyResult {
                    sequence,
                    source_pts_ms: apply.commit_source_pts_ms,
                    result: VideoCycleApplyResult::Active,
                }) {
                    return Some(Err(video_cycle_runtime_error(error)));
                }
                self.status.confirmed_change_count = controller.confirmed_change_count();
            }
            self.status.active_plan_fingerprint = Some(apply.fingerprint);
            self.status.pending_plan_fingerprint = None;
            self.set_apply_state(VideoApplyState::Active);
            if let Err(error) = self.prepare_next_controller_plan() {
                return Some(Err(error));
            }
            return Some(Ok(true));
        }

        let mut result_unknown = false;
        let response = {
            let apply = self.pending_shader_apply.as_mut()?;
            match apply
                .response
                .poll_with_hard_timeout(SHADER_APPLY_HARD_TIMEOUT)
            {
                Ok(Some(response)) => Some(response),
                Ok(None) => {
                    if apply.submitted_at.elapsed() >= SHADER_APPLY_SOFT_TIMEOUT {
                        result_unknown = true;
                    }
                    None
                }
                Err(error) => return Some(Err(error)),
            }
        };
        let Some(response) = response else {
            if result_unknown {
                let phase_changed = self.status.apply_state != VideoApplyState::ResultUnknown;
                if phase_changed {
                    let sequence = self.pending.as_ref()?.identity.sequence;
                    if let Some(controller) = self.cycle_controller.as_mut() {
                        if let Err(error) = controller.handle(VideoCycleEvent::ApplyResult {
                            sequence,
                            source_pts_ms: self.status.presented_pts_ms.unwrap_or(0),
                            result: VideoCycleApplyResult::ResultUnknown,
                        }) {
                            return Some(Err(video_cycle_runtime_error(error)));
                        }
                    }
                    self.set_apply_state(VideoApplyState::ResultUnknown);
                }
                return Some(Ok(phase_changed));
            }
            return Some(Ok(false));
        };
        if response.get("error").and_then(serde_json::Value::as_str) != Some("success") {
            return Some(Err(RealtimeVideoBackendError::IpcProtocol(
                "mpv shader 写入响应未明确成功".to_owned(),
            )));
        }
        let expected = match self
            .pending_shader_apply
            .as_ref()
            .map(|apply| apply.expected.option_map())
        {
            Some(Ok(expected)) => expected,
            Some(Err(error)) => return Some(Err(error)),
            None => return Some(Err(stale_sync_error("shader 提交事务"))),
        };
        let readback = match self
            .process
            .as_ref()
            .ok_or_else(|| stale_sync_error("shader 读回进程"))
            .and_then(|process| process.read_shader_options(MPV_COMMAND_TIMEOUT))
        {
            Ok(readback) => readback,
            Err(error) => return Some(Err(error)),
        };
        if readback != expected {
            let retry = self
                .pending_shader_apply
                .as_ref()
                .is_some_and(|apply| apply.retry_count == 0);
            if retry {
                let expected_options = self.pending_shader_apply.as_ref()?.expected.clone();
                let response = match self
                    .process
                    .as_ref()
                    .ok_or_else(|| stale_sync_error("shader 幂等重发进程"))
                    .and_then(|process| process.submit_shader_options(expected_options))
                {
                    Ok(response) => response,
                    Err(error) => return Some(Err(error)),
                };
                if let Some(apply) = self.pending_shader_apply.as_mut() {
                    apply.response = response;
                    apply.submitted_at = Instant::now();
                    apply.retry_count = 1;
                }
                let sequence = self.pending.as_ref()?.identity.sequence;
                let fingerprint = self.pending_shader_apply.as_ref()?.fingerprint.clone();
                if let Some(controller) = self.cycle_controller.as_mut() {
                    if let Err(error) = controller.handle(VideoCycleEvent::ApplyResult {
                        sequence,
                        source_pts_ms: self.status.presented_pts_ms.unwrap_or(0),
                        result: VideoCycleApplyResult::Applying { fingerprint },
                    }) {
                        return Some(Err(video_cycle_runtime_error(error)));
                    }
                }
                self.set_apply_state(VideoApplyState::Applying);
                return Some(Ok(true));
            }
            return Some(Err(RealtimeVideoBackendError::ProcessFailed {
                operation: "shader 读回确认",
                message: "同一计划指纹幂等重发后读回仍不匹配".to_owned(),
            }));
        }
        if let Some(apply) = self.pending_shader_apply.as_mut() {
            apply.phase = PendingShaderApplyPhase::AwaitingPresentation;
        }
        let sequence = self.pending.as_ref()?.identity.sequence;
        if let Some(controller) = self.cycle_controller.as_mut() {
            if let Err(error) = controller.handle(VideoCycleEvent::ApplyResult {
                sequence,
                source_pts_ms: self.status.presented_pts_ms.unwrap_or(0),
                result: VideoCycleApplyResult::ReadbackConfirmed,
            }) {
                return Some(Err(video_cycle_runtime_error(error)));
            }
        }
        self.set_apply_state(VideoApplyState::ReadbackConfirmed);
        Some(Ok(true))
    }

    fn rebuild_same_gpu_after_shader_timeout(&mut self) -> Result<(), RealtimeVideoBackendError> {
        let profile = match self.process_launch_mode {
            Some(MpvLaunchMode::Gpu(profile)) => profile,
            _ => {
                return Err(RealtimeVideoBackendError::ProcessFailed {
                    operation: "shader 超时恢复",
                    message: "当前并非 GPU 会话，不能执行同后端重建".to_owned(),
                })
            }
        };
        if let Some(sequence) = self
            .pending_shader_apply
            .as_ref()
            .and_then(|_| self.pending.as_ref().map(|plan| plan.identity.sequence))
        {
            if let Some(controller) = self.cycle_controller.as_mut() {
                let _ = controller.handle(VideoCycleEvent::ApplyResult {
                    sequence,
                    source_pts_ms: self.status.presented_pts_ms.unwrap_or(0),
                    result: VideoCycleApplyResult::Failed,
                });
            }
        }
        self.pending_shader_apply = None;
        self.set_apply_state(VideoApplyState::ResultUnknown);
        self.stop_process();
        let operation_revision = self.operation_revision.load(Ordering::Acquire);
        self.start_gpu_fallback(profile, operation_revision)?;
        self.status.n1 = self
            .pending
            .as_ref()
            .map(|plan| slot_status(plan, CycleSlotState::Ready));
        self.set_apply_state(if self.current.is_some() {
            VideoApplyState::Active
        } else {
            VideoApplyState::Ready
        });
        Ok(())
    }

    fn rebuild_same_backend_after_source_fps_timeout(
        &mut self,
    ) -> Result<(), RealtimeVideoBackendError> {
        let mode =
            self.process_launch_mode
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "换源 FPS 超时恢复",
                    message: "当前不存在可重建的视频会话".to_owned(),
                })?;
        let cpu4_plan = if mode == MpvLaunchMode::Cpu4 {
            self.current_cpu4_plan()?
        } else {
            None
        };
        self.pending_source_fps_response = None;
        self.set_apply_state(VideoApplyState::ResultUnknown);
        self.stop_process();
        let operation_revision = self.operation_revision.load(Ordering::Acquire);
        match mode {
            MpvLaunchMode::Gpu(profile) => self.start_gpu_fallback(profile, operation_revision)?,
            MpvLaunchMode::Cpu4 => {
                self.start_cpu4_process(cpu4_plan.as_ref(), operation_revision)?
            }
            MpvLaunchMode::Original => self.start_original_fallback(operation_revision)?,
        }
        self.begin_source_transition();
        Ok(())
    }

    fn rebuild_same_backend_after_observation_transport_failure(
        &mut self,
    ) -> Result<(), RealtimeVideoBackendError> {
        let mode =
            self.process_launch_mode
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "播放观察恢复",
                    message: "当前不存在可重建的视频会话".to_owned(),
                })?;
        let cpu4_plan = if mode == MpvLaunchMode::Cpu4 {
            self.current_cpu4_plan()?
        } else {
            None
        };
        self.set_apply_state(VideoApplyState::ResultUnknown);
        self.stop_process();
        let operation_revision = self.operation_revision.load(Ordering::Acquire);
        match mode {
            MpvLaunchMode::Gpu(profile) => self.start_gpu_fallback(profile, operation_revision)?,
            MpvLaunchMode::Cpu4 => {
                self.start_cpu4_process(cpu4_plan.as_ref(), operation_revision)?
            }
            MpvLaunchMode::Original => self.start_original_fallback(operation_revision)?,
        }
        self.begin_source_transition();
        Ok(())
    }

    fn prepare_next_controller_plan(&mut self) -> Result<(), RealtimeVideoBackendError> {
        if self.pending.is_some() || !self.processing_enabled {
            return Ok(());
        }
        let Some(cycle_plan) = self
            .cycle_controller
            .as_ref()
            .and_then(|controller| controller.queue().n_plus_1.clone())
        else {
            self.refresh_controller_slot_status();
            return Ok(());
        };
        let (mut plan, context) = self.compile_controller_plan(&cycle_plan)?;
        if self.status.backend == VideoBackend::Cpu4 {
            plan = cpu4_plan_from(&plan, &cycle_plan.video_params)?;
        }
        self.pending = Some(plan.clone());
        self.pending_schedule = Some(context);
        self.status.n1 = Some(slot_status(&plan, CycleSlotState::Ready));
        let source_fps =
            cycle_plan
                .source_fps
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "周期准备",
                    message: "mpv 尚未确认当前源实际 FPS".to_owned(),
                })?;
        self.status.pending_plan_fingerprint = plan_fingerprint(
            &plan,
            cycle_plan.identity.clock_epoch,
            cycle_plan.identity.media.loop_index,
            source_fps,
        )?;
        if self.current.is_none() {
            self.set_apply_state(VideoApplyState::Ready);
        }
        self.refresh_controller_slot_status();
        Ok(())
    }

    fn compile_controller_plan(
        &self,
        cycle_plan: &ControllerVideoCyclePlan,
    ) -> Result<(RealtimeVideoPlan, PreparedScheduleContext), RealtimeVideoBackendError> {
        let target_pts_ms = cycle_plan
            .identity
            .media
            .presentation_pts_ms(cycle_plan.target_source_pts_ms)
            .ok_or_else(|| stale_sync_error("周期目标 PTS"))?;
        let snapshot =
            build_gpu83_shader_snapshot(&cycle_plan.video_params, &cycle_plan.advanced_params)
                .map_err(|error| RealtimeVideoBackendError::ProcessFailed {
                    operation: "周期准备",
                    message: format!("GPU83 参数快照无效（{}:{}）", error.field, error.code),
                })?;
        let identity = VideoPlanIdentity {
            session_id: cycle_plan.identity.session_id,
            playback_generation: cycle_plan.identity.media.playback_generation,
            source_revision: 0,
            parameter_revision: cycle_plan.sequence,
            sequence: cycle_plan.sequence,
        };
        let source_fps =
            cycle_plan
                .source_fps
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "周期准备",
                    message: "mpv 尚未确认当前源实际 FPS".to_owned(),
                })?;
        let options = MpvShaderOptions::parse(snapshot.mpv_property_update().value)?;
        let (options, _) = attach_shader_plan_fingerprint(
            &options,
            &identity,
            cycle_plan.identity.clock_epoch,
            cycle_plan.identity.media.loop_index,
            source_fps,
        )?;
        let support = self.status.parameter_support.clone();
        Ok((
            RealtimeVideoPlan {
                slot: VideoPlanSlot::NPlus1,
                identity: identity.clone(),
                target_pts_ms,
                period_ms: cycle_plan.period_ms,
                seed: u64::from(cycle_plan.seed),
                prepared: true,
                commands: vec![MpvCommand::SetShaderOptions { options }],
                parameter_support: support,
            },
            PreparedScheduleContext {
                identity,
                seed: u64::from(cycle_plan.seed),
                video_params: cycle_plan.video_params.clone(),
                advanced_params: cycle_plan.advanced_params.clone(),
                clock_epoch: cycle_plan.identity.clock_epoch,
                loop_index: cycle_plan.identity.media.loop_index,
                paused: false,
            },
        ))
    }

    fn refresh_controller_slot_status(&mut self) {
        self.status.n2 = self
            .cycle_controller
            .as_ref()
            .and_then(|controller| controller.queue().n_plus_2.as_ref())
            .and_then(controller_cycle_slot_status);
    }

    fn record_presented_pts(&mut self, presented_pts_ms: u64) -> bool {
        if self.pending_boundary == VideoScheduleBoundary::None
            && self
                .status
                .presented_pts_ms
                .is_some_and(|current| presented_pts_ms < current)
        {
            return false;
        }
        if let Some(session) = self.session.as_mut() {
            session.source_position_ms = presented_pts_ms;
        }
        let changed = self.status.presented_pts_ms != Some(presented_pts_ms);
        self.status.presented_pts_ms = Some(presented_pts_ms);
        if changed {
            self.touch_status();
        }
        changed
    }

    fn record_video_observation(&mut self, presented_pts_ms: u64, source_fps: f64) -> bool {
        let mut changed = self.record_presented_pts(presented_pts_ms);
        let previous_source_fps = self.status.actual_source_fps;
        self.record_source_fps(source_fps);
        if self.status.actual_source_fps != previous_source_fps {
            changed = true;
        }
        if self.status.actual_source_fps.is_some() {
            self.source_transition_started_at = None;
            if self.status.apply_state == VideoApplyState::SourceTransitioning
                && self.current.is_some()
                && self.pending_shader_apply.is_none()
            {
                self.set_apply_state(VideoApplyState::Active);
                changed = true;
            }
        }
        changed
    }

    fn poll_source_fps_response(&mut self) -> Result<bool, RealtimeVideoBackendError> {
        if self.nominal_source_fps.is_some() {
            return Ok(false);
        }
        if self.pending_source_fps_response.is_none() {
            let response = self
                .process
                .as_ref()
                .ok_or_else(|| stale_sync_error("换源 FPS 会话"))?
                .submit_estimated_video_fps()?;
            self.pending_source_fps_response = Some(response);
            return Ok(false);
        }
        let request_id = self
            .pending_source_fps_response
            .as_ref()
            .map(PendingMpvResponse::request_id)
            .unwrap_or_default();
        let response = self
            .pending_source_fps_response
            .as_mut()
            .ok_or_else(|| stale_sync_error("换源 FPS 响应"))?
            .poll_with_hard_timeout(SOURCE_TRANSITION_FPS_HARD_TIMEOUT);
        match response {
            Ok(None) => Ok(false),
            Ok(Some(response)) => {
                self.pending_source_fps_response = None;
                let Some(source_fps) = estimated_video_fps_from_response(&response)? else {
                    return self.source_fps_still_pending(request_id);
                };
                self.record_source_fps(source_fps);
                Ok(true)
            }
            Err(error @ RealtimeVideoBackendError::PropertyUnavailable { .. }) => {
                self.pending_source_fps_response = None;
                if self.source_transition_fps_waiting() {
                    Ok(false)
                } else {
                    Err(error)
                }
            }
            Err(error) => {
                self.pending_source_fps_response = None;
                Err(error)
            }
        }
    }

    fn poll_playback_observation(
        &mut self,
    ) -> Result<Option<PlaybackObservation>, RealtimeVideoBackendError> {
        if self.pending_playback_observation.is_none() {
            let response = self
                .process
                .as_ref()
                .ok_or_else(|| stale_sync_error("视频观察会话"))?
                .submit_eof_reached()?;
            self.pending_playback_observation = Some(PendingPlaybackObservation {
                response,
                phase: PendingPlaybackObservationPhase::Eof,
                started_at: Instant::now(),
                cursor: self.sync_cursor,
                backend_epoch: self.backend_epoch,
                source_path: self.source_path.clone(),
                eof_reached: false,
                presented_pts_ms: None,
                paused_response: None,
                seeking_response: None,
                paused_for_cache_response: None,
            });
            return Ok(None);
        }
        let mut pending = self
            .pending_playback_observation
            .take()
            .ok_or_else(|| stale_sync_error("视频观察响应"))?;
        let request_id = pending.response.request_id();
        let remaining =
            PLAYBACK_OBSERVATION_HARD_TIMEOUT.saturating_sub(pending.started_at.elapsed());
        let response = pending.response.poll_with_hard_timeout(remaining);
        match response {
            Ok(None) => {
                self.pending_playback_observation = Some(pending);
                Ok(None)
            }
            Ok(Some(response)) => {
                match pending.phase {
                    PendingPlaybackObservationPhase::Eof => {
                        pending.eof_reached = parse_eof_reached_response(&response)?;
                        if pending.eof_reached {
                            pending.phase = PendingPlaybackObservationPhase::Paused;
                            pending.response = self
                                .process
                                .as_ref()
                                .ok_or_else(|| stale_sync_error("暂停观察会话"))?
                                .submit_paused()?;
                        } else {
                            pending.phase = PendingPlaybackObservationPhase::PresentedPts;
                            pending.response = self
                                .process
                                .as_ref()
                                .ok_or_else(|| stale_sync_error("PTS 观察会话"))?
                                .submit_video_pts()?;
                        }
                    }
                    PendingPlaybackObservationPhase::PresentedPts => {
                        pending.presented_pts_ms = playback_time_ms_from_response(&response)?;
                        if pending.presented_pts_ms.is_none()
                            && !self.source_transition_fps_waiting()
                        {
                            return Err(RealtimeVideoBackendError::PropertyUnavailable {
                                request_id,
                                operation: "get time-pos",
                                property_error: "property data 尚不可用".to_owned(),
                            });
                        }
                        if pending.presented_pts_ms.is_none() {
                            return Ok(None);
                        }
                        pending.phase = PendingPlaybackObservationPhase::Paused;
                        pending.response = self
                            .process
                            .as_ref()
                            .ok_or_else(|| stale_sync_error("暂停观察会话"))?
                            .submit_paused()?;
                    }
                    PendingPlaybackObservationPhase::Paused => {
                        pending.paused_response = Some(response);
                        pending.phase = PendingPlaybackObservationPhase::Seeking;
                        pending.response = self
                            .process
                            .as_ref()
                            .ok_or_else(|| stale_sync_error("seek 观察会话"))?
                            .submit_seeking()?;
                    }
                    PendingPlaybackObservationPhase::Seeking => {
                        pending.seeking_response = Some(response);
                        pending.phase = PendingPlaybackObservationPhase::PausedForCache;
                        pending.response = self
                            .process
                            .as_ref()
                            .ok_or_else(|| stale_sync_error("缓存观察会话"))?
                            .submit_paused_for_cache()?;
                    }
                    PendingPlaybackObservationPhase::PausedForCache => {
                        pending.paused_for_cache_response = Some(response);
                        pending.phase = PendingPlaybackObservationPhase::EofConfirm;
                        pending.response = self
                            .process
                            .as_ref()
                            .ok_or_else(|| stale_sync_error("EOF 复核会话"))?
                            .submit_eof_reached()?;
                    }
                    PendingPlaybackObservationPhase::EofConfirm => {
                        let confirmed_eof = parse_eof_reached_response(&response)?;
                        if confirmed_eof != pending.eof_reached {
                            return Ok(None);
                        }
                        if pending.cursor != self.sync_cursor
                            || pending.backend_epoch != self.backend_epoch
                            || pending.source_path != self.source_path
                        {
                            return Ok(None);
                        }
                        let paused = pending
                            .paused_response
                            .as_ref()
                            .ok_or_else(|| stale_sync_error("暂停观察响应"))?;
                        let seeking = pending
                            .seeking_response
                            .as_ref()
                            .ok_or_else(|| stale_sync_error("seek 观察响应"))?;
                        let paused_for_cache = pending
                            .paused_for_cache_response
                            .as_ref()
                            .ok_or_else(|| stale_sync_error("缓存观察响应"))?;
                        let playback_state =
                            MpvPlaybackState::from_responses(paused, seeking, paused_for_cache)?;
                        return Ok(Some(PlaybackObservation {
                            eof_reached: confirmed_eof,
                            presented_pts_ms: pending.presented_pts_ms,
                            playback_state,
                        }));
                    }
                }
                self.pending_playback_observation = Some(pending);
                Ok(None)
            }
            Err(error @ RealtimeVideoBackendError::PropertyUnavailable { .. }) => {
                if self.source_transition_fps_waiting() {
                    Ok(None)
                } else {
                    Err(error)
                }
            }
            Err(error) => Err(error),
        }
    }

    fn source_fps_still_pending(&self, request_id: u64) -> Result<bool, RealtimeVideoBackendError> {
        if self.source_transition_fps_waiting() {
            Ok(false)
        } else {
            Err(RealtimeVideoBackendError::PropertyUnavailable {
                request_id,
                operation: "get estimated-vf-fps",
                property_error: "property data 尚不可用".to_owned(),
            })
        }
    }

    fn source_transition_fps_waiting(&self) -> bool {
        self.source_transition_started_at
            .is_some_and(|started| started.elapsed() < SOURCE_TRANSITION_FPS_GRACE)
    }

    fn record_source_fps(&mut self, source_fps: f64) {
        let actual_source_fps = source_fps
            .is_finite()
            .then_some(source_fps)
            .filter(|fps| (1.0..=240.0).contains(fps));
        if self.status.actual_source_fps != actual_source_fps {
            self.status.actual_source_fps = actual_source_fps;
            self.nominal_source_fps = actual_source_fps;
            self.touch_status();
        }
    }

    fn initialize_cycle_after_source_observation(
        &mut self,
        source_pts_ms: u64,
        source_fps: f64,
    ) -> Result<bool, RealtimeVideoBackendError> {
        let Some(needs_identity) = self
            .cycle_controller
            .as_ref()
            .map(|controller| controller.identity().is_none())
        else {
            return Ok(false);
        };
        if !source_fps.is_finite() || !(1.0..=240.0).contains(&source_fps) {
            self.set_apply_state(VideoApplyState::SourceTransitioning);
            return Ok(false);
        }
        if needs_identity {
            let session = self
                .session
                .as_ref()
                .cloned()
                .ok_or_else(|| stale_sync_error("周期源观察会话"))?;
            let media = MediaSegmentIdentity::try_new(
                session.playback_generation,
                session.source_path.clone(),
                session.loop_index,
                session.source_duration_ms,
            )
            .map_err(|error| RealtimeVideoBackendError::ProcessFailed {
                operation: "周期源观察",
                message: format!("媒体段身份无效：{error:?}"),
            })?;
            let identity = VideoCycleSegmentIdentity::try_new(
                session.session_id,
                self.backend_epoch,
                session.clock_epoch,
                media,
            )
            .map_err(video_cycle_runtime_error)?;
            self.cycle_controller
                .as_mut()
                .ok_or_else(|| stale_sync_error("周期控制器"))?
                .handle(VideoCycleEvent::SourceBoundary {
                    identity,
                    source_fps: Some(source_fps),
                    source_pts_ms,
                })
                .map_err(video_cycle_runtime_error)?;
        } else {
            let paused = self.sync_cursor.is_none_or(|cursor| cursor.paused);
            self.cycle_controller
                .as_mut()
                .ok_or_else(|| stale_sync_error("周期控制器"))?
                .handle(VideoCycleEvent::MpvObservation {
                    source_pts_ms,
                    source_fps: Some(source_fps),
                    paused,
                })
                .map_err(video_cycle_runtime_error)?;
        }
        self.prepare_next_controller_plan()?;
        Ok(true)
    }

    fn observe_presented_pts_liveness(
        &mut self,
        presented_pts_ms: u64,
        eof_reached: bool,
        budget: &ObservationTickBudget,
    ) -> bool {
        if self.pending_shader_apply.is_some() {
            self.presented_pts_watchdog.reset();
            return false;
        }
        if self.nominal_source_fps.is_none() {
            self.presented_pts_watchdog.reset();
            return false;
        }
        let eligible = should_watch_presented_pts(PresentedPtsWatchdogFacts {
            process_exists: self.process.is_some(),
            paused: self.sync_cursor.is_none_or(|cursor| cursor.paused),
            eof_reached,
            activation: self.status.activation,
            lifecycle: self.status.lifecycle,
            backend: self.status.backend,
        });
        let now = Instant::now().saturating_duration_since(self.av_sync_monotonic_origin);
        let stall_threshold = presented_pts_stall_threshold(self.nominal_source_fps);
        match self
            .presented_pts_watchdog
            .observe(now, eligible, presented_pts_ms, stall_threshold)
        {
            PresentedPtsLiveness::Healthy => false,
            PresentedPtsLiveness::SoftRecover { position_ms } => {
                let Some(source_duration_ms) = self
                    .session
                    .as_ref()
                    .map(|session| session.source_duration_ms)
                    .filter(|duration| *duration > 0)
                else {
                    return self.fail_presented_pts_liveness(
                        "mpv 物理 PTS 停滞且当前源时长无效".to_owned(),
                    );
                };
                let position_ms = position_ms.min(source_duration_ms.saturating_sub(1));
                let recovery = (|| {
                    let process = self.process.as_ref().ok_or_else(|| {
                        RealtimeVideoBackendError::ProcessFailed {
                            operation: "物理 PTS 软恢复",
                            message: "mpv 进程不存在".to_owned(),
                        }
                    })?;
                    process.send_command(
                        &MpvCommand::SeekAbsoluteMs { position_ms },
                        budget.remaining(),
                    )?;
                    process.send_command(
                        &MpvCommand::SetPause { paused: false },
                        budget.remaining(),
                    )?;
                    Ok::<(), RealtimeVideoBackendError>(())
                })();
                match recovery {
                    Ok(()) => {
                        eprintln!(
                            "[realtime-video-watchdog] stage=soft_recovery position_ms={position_ms} threshold_ms={}",
                            stall_threshold.as_millis()
                        );
                        false
                    }
                    Err(error) => self.fail_presented_pts_liveness(format!(
                        "mpv 物理 PTS 停滞，软恢复失败：{error}"
                    )),
                }
            }
            PresentedPtsLiveness::Stalled { position_ms } => self.fail_presented_pts_liveness(
                format!("mpv 物理 PTS 在 {position_ms}ms 持续停滞，单次软恢复后仍未推进"),
            ),
        }
    }

    fn fail_presented_pts_liveness(&mut self, reason: String) -> bool {
        let fallback_plan = self.current_cpu4_plan().ok().flatten();
        self.handle_process_failure(reason, unix_now_ms(), fallback_plan, true);
        true
    }

    fn record_eof_observation(&mut self, eof_reached: bool, physical_paused: bool) -> bool {
        if eof_reached {
            self.presented_pts_watchdog.reset();
        }
        let physical_eof_changed = self.status.physical_eof_reached != Some(eof_reached);
        self.status.physical_eof_reached = Some(eof_reached);
        let next = eof_fact_from_cursor(self.sync_cursor, self.backend_epoch, eof_reached);
        let eof_changed = self.status.eof != next;
        self.status.eof = next;
        if !eof_reached {
            self.last_notified_eof = None;
            self.eof_notification_backpressured = false;
        }
        // pause 与 EOF 属于同一个异步播放观察事务；禁止在 EOF 热路径追加同步
        // `get pause`，否则一次 Windows 管道调度迟到就会关闭仍健康的 mpv 会话。
        let physical_pause_changed = self.status.physical_paused != Some(physical_paused);
        self.status.physical_paused = Some(physical_paused);
        physical_eof_changed || physical_pause_changed || eof_changed
    }

    fn notify_eof_supervisor(&mut self) -> bool {
        let Some(eof) = self.status.eof else {
            return false;
        };
        if self.last_notified_eof == Some(eof) {
            return false;
        }
        let Some(sender) = self.eof_event_sender.as_ref() else {
            return false;
        };
        match sender.try_send(eof) {
            Ok(()) => {
                self.last_notified_eof = Some(eof);
                self.eof_notification_backpressured = false;
                eprintln!(
                    "[realtime-video-eof] stage=notified playback_generation={} backend_epoch={} clock_epoch={} loop_index={}",
                    eof.playback_generation, eof.backend_epoch, eof.clock_epoch, eof.loop_index
                );
                true
            }
            Err(TrySendError::Full(_)) => {
                if !self.eof_notification_backpressured {
                    eprintln!(
                        "[realtime-video-eof] stage=backpressured playback_generation={} backend_epoch={} clock_epoch={} loop_index={} channel_capacity=1",
                        eof.playback_generation, eof.backend_epoch, eof.clock_epoch, eof.loop_index
                    );
                    self.eof_notification_backpressured = true;
                }
                false
            }
            Err(TrySendError::Disconnected(_)) => {
                eprintln!(
                    "[realtime-video-eof] stage=supervisor_disconnected playback_generation={} backend_epoch={} clock_epoch={} loop_index={}",
                    eof.playback_generation, eof.backend_epoch, eof.clock_epoch, eof.loop_index
                );
                self.eof_event_sender = None;
                self.eof_notification_backpressured = false;
                false
            }
        }
    }

    fn clear_eof_fact(&mut self) {
        self.status.eof = None;
        self.last_notified_eof = None;
        self.eof_notification_backpressured = false;
    }

    fn record_observation_success(&mut self) {
        self.consecutive_tick_failures = 0;
        self.consecutive_transient_observation_failures = 0;
    }

    fn observation_failure_reached_threshold(&mut self, error: &RealtimeVideoBackendError) -> bool {
        if transient_observation_failure(error) {
            self.consecutive_tick_failures = 0;
            self.consecutive_transient_observation_failures = self
                .consecutive_transient_observation_failures
                .saturating_add(1);
            if self.consecutive_transient_observation_failures
                < TRANSIENT_OBSERVATION_FAILURE_THRESHOLD
            {
                return false;
            }
        } else {
            self.consecutive_transient_observation_failures = 0;
            self.consecutive_tick_failures = self.consecutive_tick_failures.saturating_add(1);
            if self.consecutive_tick_failures < TICK_FAILURE_DEMOTION_THRESHOLD {
                return false;
            }
        }
        self.record_observation_success();
        true
    }

    fn record_tick_failure(&mut self, error: RealtimeVideoBackendError) -> bool {
        if matches!(
            &error,
            RealtimeVideoBackendError::StaleSync { .. }
                | RealtimeVideoBackendError::SyncSuperseded { .. }
                | RealtimeVideoBackendError::StalePrepareBackendEpoch { .. }
        ) {
            self.pending = None;
            self.pending_schedule = None;
            self.status.n1 = None;
            self.record_observation_success();
            self.touch_status();
            return true;
        }
        if self.source_transition_fps_is_pending(&error) {
            self.consecutive_tick_failures = 0;
            self.consecutive_transient_observation_failures = 0;
            return false;
        }
        if matches!(&error, RealtimeVideoBackendError::IpcTimeout { .. })
            && !self.observation_failure_reached_threshold(&error)
        {
            // Windows 命名管道在 shader/VO 属性读回密集时允许出现单次迟到。
            // 超时只表示本轮结果未知，不能据此杀掉仍在连续呈现的 mpv。
            return false;
        }
        if matches!(
            &error,
            RealtimeVideoBackendError::IpcTimeout { .. }
                | RealtimeVideoBackendError::IpcDisconnected(_)
        ) {
            self.record_observation_success();
            let detail = error.to_string();
            if self
                .rebuild_same_backend_after_observation_transport_failure()
                .is_err()
            {
                let fallback_plan = self.current_cpu4_plan().ok().flatten();
                self.handle_process_failure(
                    format!("播放观察 IPC 失效且同后端重建失败：{detail}"),
                    unix_now_ms(),
                    fallback_plan,
                    true,
                );
            }
            return true;
        }
        if !self.observation_failure_reached_threshold(&error) {
            return false;
        }
        let reason = match self.process.as_mut() {
            Some(process) => match process.poll_exit_evidence() {
                Ok(Some(evidence)) => unexpected_mpv_exit_reason("视频后端", Some(&evidence), None),
                Ok(None) => error.to_string(),
                Err(status_error) => {
                    unexpected_mpv_exit_reason("视频后端", None, Some(&status_error))
                }
            },
            None => error.to_string(),
        };
        let fallback_plan = self.current_cpu4_plan().ok().flatten();
        let operation_revision = self.operation_revision.load(Ordering::Acquire);
        self.transition_to_fallback(
            reason,
            unix_now_ms(),
            fallback_plan,
            true,
            operation_revision,
        );
        true
    }

    fn source_transition_fps_is_pending(&self, error: &RealtimeVideoBackendError) -> bool {
        self.status.apply_state == VideoApplyState::SourceTransitioning
            && self.source_transition_fps_waiting()
            && matches!(
                error,
                RealtimeVideoBackendError::PropertyUnavailable {
                    operation: "get estimated-vf-fps",
                    ..
                }
            )
    }

    fn record_source_observation_failure(&mut self, error: RealtimeVideoBackendError) -> bool {
        if matches!(
            &error,
            RealtimeVideoBackendError::IpcTimeout { .. }
                | RealtimeVideoBackendError::IpcDisconnected(_)
        ) {
            return self.record_tick_failure(error);
        }
        if !self.observation_failure_reached_threshold(&error) {
            return false;
        }
        self.handle_process_failure(error.to_string(), unix_now_ms(), None, false);
        true
    }

    fn record_frame_budget_observation_failure(
        &mut self,
        error: RealtimeVideoBackendError,
    ) -> bool {
        if matches!(&error, RealtimeVideoBackendError::IpcDisconnected(_)) {
            self.consecutive_frame_budget_observation_failures = 0;
            let detail = error.to_string();
            if self
                .rebuild_same_backend_after_observation_transport_failure()
                .is_err()
            {
                let fallback_plan = self.current_cpu4_plan().ok().flatten();
                self.handle_process_failure(
                    format!("帧预算 IPC 失效且同后端重建失败：{detail}"),
                    unix_now_ms(),
                    fallback_plan,
                    true,
                );
            }
            return true;
        }
        self.consecutive_frame_budget_observation_failures = self
            .consecutive_frame_budget_observation_failures
            .saturating_add(1)
            .min(FRAME_BUDGET_OBSERVATION_FAILURE_THRESHOLD);
        self.last_frame_budget_pts_ms = None;
        self.last_vo_passes_snapshot = None;
        self.last_frame_health_counts = None;
        self.consecutive_frame_budget_violations = 0;
        self.status.frame_budget_violation_windows = 0;
        self.status.gpu_pass_p99_ms = None;
        self.status.frame_drop_count = None;
        self.status.decoder_frame_drop_count = None;
        self.status.mistimed_frame_count = None;
        self.status.delayed_frame_count = None;
        false
    }

    fn observe_frame_budget(
        &mut self,
        presented_pts_ms: u64,
        eof_reached: bool,
        playback_state: MpvPlaybackState,
        budget: &ObservationTickBudget,
    ) -> bool {
        if !self.frame_budget_observation_eligible(eof_reached, playback_state) {
            self.begin_frame_budget_transition();
            return false;
        }
        let now = Instant::now();
        if self.last_frame_budget_observation.is_some_and(|previous| {
            now.duration_since(previous) < FRAME_BUDGET_OBSERVATION_INTERVAL
        }) {
            return false;
        }
        self.last_frame_budget_observation = Some(now);
        let Some(process) = self.process.as_ref() else {
            return self.record_frame_budget_observation_failure(
                RealtimeVideoBackendError::ProcessFailed {
                    operation: "帧预算观察",
                    message: "mpv 会话不存在".to_owned(),
                },
            );
        };
        let observation = (|| {
            let vo_passes =
                process.send_command(&MpvCommand::GetVideoOutputPasses, budget.remaining())?;
            let frame_drop =
                process.send_command(&MpvCommand::GetFrameDropCount, budget.remaining())?;
            let decoder_drop =
                process.send_command(&MpvCommand::GetDecoderFrameDropCount, budget.remaining())?;
            let mistimed = optional_frame_timing_counter(
                process.send_command(&MpvCommand::GetMistimedFrameCount, budget.remaining()),
                "mistimed-frame-count",
            )?;
            let delayed = optional_frame_timing_counter(
                process.send_command(
                    &MpvCommand::GetVideoOutputDelayedFrameCount,
                    budget.remaining(),
                ),
                "vo-delayed-frame-count",
            )?;
            let vo_passes_snapshot = vo_passes_timing_snapshot(&vo_passes)?;
            Ok::<_, RealtimeVideoBackendError>((
                vo_passes_snapshot_p99_ms(&vo_passes_snapshot)?,
                vo_passes_snapshot,
                response_nonnegative_integer(&frame_drop, "frame-drop-count")?,
                response_nonnegative_integer(&decoder_drop, "decoder-frame-drop-count")?,
                mistimed,
                delayed,
            ))
        })();
        let (
            gpu_pass_p99_ms,
            vo_passes_snapshot,
            frame_drop_count,
            decoder_frame_drop_count,
            mistimed_frame_count,
            delayed_frame_count,
        ) = match observation {
            Ok(observation) => observation,
            Err(error) => return self.record_frame_budget_observation_failure(error),
        };
        self.consecutive_frame_budget_observation_failures = 0;
        let previous_frame_budget_pts_ms = self.last_frame_budget_pts_ms;
        let pts_regressed = previous_frame_budget_pts_ms
            .is_some_and(|previous_pts_ms| presented_pts_ms < previous_pts_ms);
        let pts_advanced = previous_frame_budget_pts_ms
            .is_some_and(|previous_pts_ms| presented_pts_ms > previous_pts_ms);
        self.last_frame_budget_pts_ms = Some(presented_pts_ms);
        let incremental_gpu_pass_p99_ms = self
            .last_vo_passes_snapshot
            .as_ref()
            .and_then(|previous| new_vo_passes_p99_ms(previous, &vo_passes_snapshot, pts_advanced));
        self.last_vo_passes_snapshot = Some(vo_passes_snapshot);
        let counters = (
            frame_drop_count,
            decoder_frame_drop_count,
            mistimed_frame_count,
            delayed_frame_count,
        );
        let counter_regressed = self
            .last_frame_health_counts
            .is_some_and(|previous| counters_regressed(previous, counters));
        let unhealthy_counter_increased =
            frame_health_counter_increase(self.last_frame_health_counts, counters);
        self.last_frame_health_counts = Some(counters);
        self.status.gpu_pass_p99_ms = Some(gpu_pass_p99_ms);
        self.status.frame_drop_count = Some(frame_drop_count);
        self.status.decoder_frame_drop_count = Some(decoder_frame_drop_count);
        self.status.mistimed_frame_count = mistimed_frame_count;
        self.status.delayed_frame_count = delayed_frame_count;
        let Some(source_fps) = self.nominal_source_fps else {
            return false;
        };
        let gpu_processing_budget_ms = if source_fps >= 59.0 {
            12.0
        } else {
            1_000.0 / source_fps
        };
        if pts_regressed || counter_regressed {
            self.consecutive_frame_budget_violations = 0;
            self.status.frame_budget_violation_windows = 0;
            return false;
        }
        if incremental_gpu_pass_p99_ms.is_none() && unhealthy_counter_increased != Some(true) {
            return false;
        }
        self.consecutive_frame_budget_violations = advance_hard_frame_health_violation_count(
            self.consecutive_frame_budget_violations,
            unhealthy_counter_increased,
        );
        self.status.frame_budget_violation_windows = self.consecutive_frame_budget_violations;
        if self.consecutive_frame_budget_violations < FRAME_BUDGET_VIOLATION_THRESHOLD {
            return false;
        }
        let fallback_plan = self.current_cpu4_plan().ok().flatten();
        let operation_revision = self.operation_revision.load(Ordering::Acquire);
        let mistimed_frame_count =
            mistimed_frame_count.map_or_else(|| "n/a".to_owned(), |count| count.to_string());
        let delayed_frame_count =
            delayed_frame_count.map_or_else(|| "n/a".to_owned(), |count| count.to_string());
        let incremental_gpu_pass_p99_ms = incremental_gpu_pass_p99_ms
            .map_or_else(|| "n/a".to_owned(), |p99_ms| format!("{p99_ms:.3}ms"));
        self.transition_to_fallback(
            format!(
                "连续渲染健康窗口超限：新增样本 GPU pass P99={incremental_gpu_pass_p99_ms}，滚动 P99={gpu_pass_p99_ms:.3}ms，处理预算={gpu_processing_budget_ms:.3}ms，丢帧={frame_drop_count}/{decoder_frame_drop_count}，时序异常={mistimed_frame_count}/{delayed_frame_count}"
            ),
            unix_now_ms(),
            fallback_plan,
            true,
            operation_revision,
        );
        true
    }

    fn frame_budget_observation_eligible(
        &self,
        eof_reached: bool,
        playback_state: MpvPlaybackState,
    ) -> bool {
        !eof_reached
            && !playback_state.paused
            && !playback_state.seeking
            && !playback_state.paused_for_cache
            && self.status.apply_state == VideoApplyState::Active
            && matches!(self.pending_boundary, VideoScheduleBoundary::None)
    }

    fn stop(&mut self) {
        self.stop_process();
        self.session = None;
        self.backend_epoch = 0;
        self.pending = None;
        self.current = None;
        self.pending_schedule = None;
        self.current_schedule = None;
        self.cycle_controller = None;
        self.backend = VideoBackendStateMachine::new();
        self.scheduler = VideoFrameScheduler::new();
        self.schedule_epoch = 0;
        self.sync_cursor = None;
        self.pending_boundary = VideoScheduleBoundary::None;
        self.nominal_source_fps = None;
        self.last_playback_speed = None;
        self.scheduler_base_speed = 1.0;
        self.last_shader_options = None;
        self.record_observation_success();
        self.reset_frame_budget_observation();
        self.status = MediaVideoBackendRuntimeStatus::default();
        self.reset_av_sync();
        self.touch_status();
    }

    fn suspend(&mut self) {
        let had_bound_session = self.session.is_some();
        self.stop_process();
        self.pending = None;
        self.current = None;
        self.pending_schedule = None;
        self.current_schedule = None;
        self.cycle_controller = None;
        self.scheduler = VideoFrameScheduler::new();
        self.pending_boundary = VideoScheduleBoundary::None;
        self.nominal_source_fps = None;
        self.last_playback_speed = None;
        self.scheduler_base_speed = 1.0;
        self.last_shader_options = None;
        self.record_observation_success();
        self.reset_frame_budget_observation();
        // operation revision 已经负责使 suspend 前的在途启动失效。只有已经建立过
        // 可发布完整身份的会话才推进 backend epoch；全新 runtime 若在首次开关时
        // 从 0 推进到 1，调用方无法取得绑定该 epoch 的四维身份，首个 prepare 会
        // 永久落入 stale 循环。
        self.backend_epoch = if had_bound_session {
            self.backend_epoch.saturating_add(1)
        } else {
            0
        };
        let last_demotion = self.backend.last_demotion().cloned();
        let demotion_reason = last_demotion
            .as_ref()
            .map(|demotion| demotion.reason.clone());
        self.status = MediaVideoBackendRuntimeStatus::default();
        self.reset_av_sync();
        self.status.demotion_reason = demotion_reason;
        self.status.last_demotion = last_demotion;
        self.status.demotion_history = self.backend.demotion_history().to_vec();
        self.status.activation = BackendActivation::Available;
        self.status.lifecycle = RendererLifecycleState::Stopped;
        self.record_session_identity();
    }

    fn set_processing_enabled(
        &mut self,
        enabled: bool,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        self.processing_enabled = enabled;
        if !enabled {
            self.cycle_controller = None;
        }
        if !enabled && self.process.is_some() {
            self.neutralize_to_source_with_fallback()?;
        }
        Ok(self.status.clone())
    }

    fn neutralize_to_source_with_fallback(&mut self) -> Result<(), RealtimeVideoBackendError> {
        loop {
            match self.neutralize_to_source() {
                Ok(()) => return Ok(()),
                Err(error)
                    if self.process_launch_mode != Some(MpvLaunchMode::Original)
                        && self.backend.current() != VideoBackend::Source =>
                {
                    let operation_revision = self.operation_revision.load(Ordering::Acquire);
                    self.transition_to_fallback(
                        error.to_string(),
                        unix_now_ms(),
                        None,
                        false,
                        operation_revision,
                    );
                    if self.process.is_none() {
                        return Err(error);
                    }
                }
                Err(error) => {
                    self.record_original_failure(error.to_string());
                    self.stop_process();
                    return Err(error);
                }
            }
        }
    }

    fn neutralize_to_source(&mut self) -> Result<(), RealtimeVideoBackendError> {
        if self.status.backend != VideoBackend::Source {
            let neutral_shader_options = self.apply_neutral_process_parameters()?;
            // 中性命令与旧提交共用串行 IPC worker；中性响应成功意味着它已经排在
            // 旧请求之后物理执行。此时必须丢弃旧事务，禁止后续 tick 再读回、重发
            // 或晋级已经被中性快照覆盖的 N+1。
            self.pending_shader_apply = None;
            self.pending = None;
            self.current = None;
            self.pending_schedule = None;
            self.current_schedule = None;
            self.scheduler = VideoFrameScheduler::new();
            self.schedule_epoch = 0;
            self.pending_boundary = VideoScheduleBoundary::None;
            self.nominal_source_fps = None;
            // 只清除视频周期自己的变速基数；最终 speed 仍由唯一写入器叠加现有 AV 修正。
            self.scheduler_base_speed = 1.0;
            self.last_shader_options = neutral_shader_options;
            self.record_neutral_source_status()?;
        }
        Ok(())
    }

    fn apply_neutral_process_parameters(
        &self,
    ) -> Result<Option<MpvShaderOptions>, RealtimeVideoBackendError> {
        let Some(mode) = self.process_launch_mode else {
            return Ok(None);
        };
        let process =
            self.process
                .as_ref()
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "关闭视频处理",
                    message: "mpv 进程不存在".to_owned(),
                })?;
        match mode {
            MpvLaunchMode::Gpu(_) => {
                let options = neutral_gpu_shader_options()?;
                process.send_command(
                    &MpvCommand::SetShaderOptions {
                        options: options.clone(),
                    },
                    MPV_COMMAND_TIMEOUT,
                )?;
                Ok(Some(options))
            }
            MpvLaunchMode::Cpu4 => {
                let defaults = VideoEffectParams::default();
                let snapshot = Cpu4Snapshot::from_ui(
                    defaults.brightness_percent,
                    defaults.contrast_percent,
                    defaults.saturation_percent,
                    defaults.hue_rotation_degrees,
                )?;
                for command in snapshot.commands() {
                    process.send_command(&command, MPV_COMMAND_TIMEOUT)?;
                }
                Ok(None)
            }
            MpvLaunchMode::Original => Ok(None),
        }
    }

    fn record_source_backend(&mut self, reason: String) {
        let reason = self.bounded_failure_reason(&reason);
        self.stop_process();
        self.session = None;
        self.backend_epoch = 0;
        self.pending = None;
        self.current = None;
        self.pending_schedule = None;
        self.current_schedule = None;
        self.cycle_controller = None;
        self.scheduler = VideoFrameScheduler::new();
        self.schedule_epoch = 0;
        self.sync_cursor = None;
        self.pending_boundary = VideoScheduleBoundary::None;
        self.nominal_source_fps = None;
        self.last_playback_speed = None;
        self.scheduler_base_speed = 1.0;
        self.last_shader_options = None;
        self.record_observation_success();
        self.status = MediaVideoBackendRuntimeStatus::default();
        self.reset_av_sync();
        self.status.demotion_reason = Some(reason);
        self.status.last_demotion = self.backend.last_demotion().cloned();
        self.status.demotion_history = self.backend.demotion_history().to_vec();
        self.status.fallback_floor_mode = self.backend.launch_mode().name().to_owned();
    }

    fn reset_scheduler(
        &mut self,
        playback_generation: u64,
        clock_epoch: u64,
        loop_index: u64,
        paused: bool,
    ) {
        self.clear_eof_fact();
        self.scheduler = VideoFrameScheduler::new();
        self.schedule_epoch = 1;
        self.sync_cursor = Some(SyncCursor {
            playback_generation,
            clock_epoch,
            loop_index,
            paused,
        });
        self.pending_boundary = VideoScheduleBoundary::Startup;
        self.pending_source_fps_response = None;
        self.pending_playback_observation = None;
        self.nominal_source_fps = None;
        self.source_transition_started_at = Some(Instant::now());
        self.status.actual_source_fps = None;
        self.last_playback_speed = None;
        self.scheduler_base_speed = 1.0;
        self.presented_pts_watchdog.reset();
        self.reset_av_sync();
        self.last_shader_options = None;
        self.record_observation_success();
        self.reset_frame_budget_observation();
    }

    fn reset_frame_budget_observation(&mut self) {
        self.begin_frame_budget_transition();
        self.status.gpu_pass_p99_ms = None;
        self.status.frame_drop_count = None;
        self.status.decoder_frame_drop_count = None;
        self.status.mistimed_frame_count = None;
        self.status.delayed_frame_count = None;
    }

    fn begin_frame_budget_transition(&mut self) {
        self.last_frame_budget_observation = None;
        self.last_frame_budget_pts_ms = None;
        self.last_vo_passes_snapshot = None;
        self.last_frame_health_counts = None;
        self.consecutive_frame_budget_observation_failures = 0;
        self.consecutive_frame_budget_violations = 0;
        self.status.frame_budget_violation_windows = 0;
    }

    fn begin_generation(
        &mut self,
        playback_generation: u64,
    ) -> Result<GenerationTransition, RealtimeVideoBackendError> {
        let current_generation = self
            .session
            .as_ref()
            .map(|session| session.playback_generation)
            .or_else(|| self.sync_cursor.map(|cursor| cursor.playback_generation));
        match current_generation {
            Some(current) if playback_generation < current => Err(stale_sync_error("播放代次")),
            Some(current) if playback_generation > current => {
                let eof_matches_current_identity = self.status.eof.as_ref().is_some_and(|eof| {
                    eof.playback_generation == current
                        && eof.backend_epoch == self.backend_epoch
                        && self.session.as_ref().is_some_and(|session| {
                            eof.clock_epoch == session.clock_epoch
                                && eof.loop_index == session.loop_index
                        })
                });
                let process_present = self
                    .process
                    .as_mut()
                    .is_some_and(|process| process.has_exited().is_ok_and(|exited| !exited));
                if can_preserve_process_for_next_generation(GenerationReuseFacts {
                    current_generation: current,
                    requested_generation: playback_generation,
                    process_present,
                    eof_matches_current_identity,
                }) {
                    self.reset_generation_state_preserving_process();
                    Ok(GenerationTransition::PreservedProcess)
                } else {
                    self.stop();
                    Ok(GenerationTransition::ResetProcess)
                }
            }
            _ => Ok(GenerationTransition::Same),
        }
    }

    fn reset_generation_state_preserving_process(&mut self) {
        self.session = None;
        self.backend_epoch = 0;
        self.pending = None;
        self.current = None;
        self.pending_schedule = None;
        self.current_schedule = None;
        self.pending_shader_apply = None;
        self.pending_source_fps_response = None;
        self.pending_playback_observation = None;
        self.scheduler = VideoFrameScheduler::new();
        self.schedule_epoch = 0;
        self.sync_cursor = None;
        self.pending_boundary = VideoScheduleBoundary::None;
        self.nominal_source_fps = None;
        self.last_playback_speed = None;
        self.scheduler_base_speed = 1.0;
        // 跨媒体代次不得保留上一源的运行时 shader 快照（尤其是 source_fps）。
        // 保留 mpv 进程只是一项资源优化，不代表旧媒体参数仍可复用。
        self.last_shader_options = None;
        self.record_observation_success();
        self.reset_frame_budget_observation();
        self.status = MediaVideoBackendRuntimeStatus::default();
        self.reset_av_sync();
    }

    #[allow(clippy::too_many_arguments)]
    fn try_switch_preserved_process(
        &mut self,
        transition: GenerationTransition,
        source: &Path,
        host_window_id: u64,
        source_start_ms: u64,
        source_duration_ms: u64,
        paused: bool,
        neutralize_before_switch: bool,
        operation_revision: u64,
    ) -> Result<bool, RealtimeVideoBackendError> {
        if transition != GenerationTransition::PreservedProcess {
            return Ok(false);
        }
        let requested_mode = self.backend.launch_mode();
        if retain_same_source_process(
            self.source_path.as_deref() == Some(source),
            self.process_launch_mode == Some(requested_mode),
            self.process_host_window_id == Some(host_window_id),
            self.process.is_some(),
        ) {
            // 同一媒体的周期配置只更新参数意图，不能为走“跨源复用”分支而
            // 终止健康 mpv。返回 false 让 prepare 的普通复用判定继续处理。
            return Ok(false);
        }
        let reusable = self.source_path.as_deref() != Some(source)
            && self.process_launch_mode == Some(requested_mode)
            && self.process_host_window_id == Some(host_window_id)
            && self.process.is_some();
        if !reusable {
            self.stop_process();
            return Ok(false);
        }
        if neutralize_before_switch {
            match self.apply_neutral_process_parameters() {
                Ok(options) => self.last_shader_options = options,
                Err(error) => {
                    self.stop_process();
                    if self.operation_cancelled(operation_revision) {
                        return Err(error);
                    }
                    return Ok(false);
                }
            }
        }

        let revision = Arc::clone(&self.operation_revision);
        let switch_result = match self.process.as_mut() {
            Some(process) => process.switch_media_source(
                source,
                source_start_ms,
                source_duration_ms,
                MPV_PIPE_CONNECT_TIMEOUT,
                paused,
                || revision.load(Ordering::Acquire) != operation_revision,
            ),
            None => return Ok(false),
        };
        let presented_pts_ms = match switch_result {
            Ok(position_ms) => position_ms,
            Err(RealtimeVideoBackendError::MediaSwitchCancelled) => {
                self.stop_process();
                return Err(stale_sync_error("视频换源"));
            }
            Err(_) => {
                // loadfile 的结果未知时不能再信任当前媒体身份；终止后由同一后端正常重启。
                self.stop_process();
                return Ok(false);
            }
        };
        if let Err(error) = self.ensure_operation_current(operation_revision, "视频换源") {
            self.stop_process();
            return Err(error);
        }

        let process_id = self
            .process
            .as_ref()
            .and_then(ManagedMpvProcess::pid)
            .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                operation: "视频换源",
                message: "换源后 mpv 进程标识丢失".to_owned(),
            })?;
        let decoder = match self
            .process
            .as_ref()
            .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                operation: "视频换源",
                message: "换源后 mpv 进程丢失".to_owned(),
            })?
            .read_active_decoder(MPV_COMMAND_TIMEOUT)
        {
            Ok(decoder) => decoder,
            Err(_) if requested_mode == MpvLaunchMode::Original => "unreported-original".to_owned(),
            Err(_) => {
                self.stop_process();
                return Ok(false);
            }
        };
        let (physical_paused, physical_eof_reached) = confirm_physical_process_facts(
            self.process
                .as_ref()
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "视频换源",
                    message: "换源后 mpv 进程丢失".to_owned(),
                })?,
            Some(paused),
            "视频换源",
        )?;
        if let Some(session) = self.session.as_mut() {
            session.source_position_ms = presented_pts_ms;
        }
        self.source_path = Some(source.to_path_buf());
        let session = self.session.as_ref().cloned().ok_or_else(|| {
            RealtimeVideoBackendError::ProcessFailed {
                operation: "视频换源",
                message: "换源后视频会话身份丢失".to_owned(),
            }
        })?;
        self.reset_scheduler(
            session.playback_generation,
            session.clock_epoch,
            session.loop_index,
            session.paused,
        );
        match requested_mode {
            MpvLaunchMode::Gpu(profile) => self.record_renderer_started(
                profile.graphics_api(),
                decoder,
                process_id,
                presented_pts_ms,
                physical_paused,
                physical_eof_reached,
            ),
            MpvLaunchMode::Cpu4 => {
                self.advance_backend_epoch();
                self.claim_cpu4_status();
                self.status.activation = BackendActivation::Active;
                self.status.lifecycle = RendererLifecycleState::Active;
                self.status.decoder = Some(decoder);
                self.record_confirmed_process_facts(
                    process_id,
                    presented_pts_ms,
                    physical_paused,
                    physical_eof_reached,
                );
            }
            MpvLaunchMode::Original => self.record_original_started(
                MpvGraphicsApi::D3d11,
                decoder,
                process_id,
                presented_pts_ms,
                physical_paused,
                physical_eof_reached,
            ),
        }
        Ok(true)
    }

    fn operation_cancelled(&self, operation_revision: u64) -> bool {
        self.operation_revision.load(Ordering::Acquire) != operation_revision
    }

    fn ensure_operation_current(
        &self,
        operation_revision: u64,
        operation: &'static str,
    ) -> Result<(), RealtimeVideoBackendError> {
        if self.operation_cancelled(operation_revision) {
            Err(RealtimeVideoBackendError::ProcessFailed {
                operation,
                message: "视频后端转换已取消".to_owned(),
            })
        } else {
            Ok(())
        }
    }

    fn prepare_cpu4(
        &mut self,
        request: PrepareRealtimeRenderer,
        source: &Path,
        now_unix_ms: u64,
        operation_revision: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        let plan = cpu4_plan_from(&request.plan, &request.video_params)?;
        let (process_exit_evidence, process_status_error) = match self.process.as_mut() {
            Some(process) => match process.poll_exit_evidence() {
                Ok(evidence) => (evidence, None),
                Err(error) => (None, Some(error)),
            },
            None => (None, None),
        };
        let process_exited = process_exit_evidence.is_some() || process_status_error.is_some();
        let process_lost = self.process.is_none()
            && self.backend_epoch > 0
            && self.status.lifecycle != RendererLifecycleState::Stopped;
        if process_exited || process_lost {
            self.transition_to_fallback(
                unexpected_mpv_exit_reason(
                    "CPU4",
                    process_exit_evidence.as_ref(),
                    process_status_error.as_ref(),
                ),
                now_unix_ms,
                Some(plan),
                false,
                operation_revision,
            );
            return Ok(self.status.clone());
        }
        let must_restart = self.process.is_none()
            || !renderer_ready_for_mode(
                self.status.backend,
                self.process_launch_mode,
                MpvLaunchMode::Cpu4,
            )
            || self.source_path.as_deref() != Some(source)
            || renderer_host_window_changed(
                self.process.is_some(),
                self.process_host_window_id,
                request.host_window_id,
            )
            || process_exited;
        if must_restart {
            let active_plan = self.current_cpu4_plan()?;
            self.stop_process();
            if let Err(error) = self.start_cpu4_process(active_plan.as_ref(), operation_revision) {
                if self.operation_cancelled(operation_revision) {
                    return Err(error);
                }
                self.transition_to_fallback(
                    error.to_string(),
                    now_unix_ms,
                    None,
                    true,
                    operation_revision,
                );
                return Ok(self.status.clone());
            }
        }
        self.pending_schedule = Some(PreparedScheduleContext {
            identity: request.plan.identity.clone(),
            seed: request.plan.seed,
            video_params: request.video_params,
            advanced_params: request.advanced_params,
            clock_epoch: request.clock_epoch,
            loop_index: request.loop_index,
            paused: request.paused,
        });
        self.pending = Some(plan.clone());
        self.claim_cpu4_status();
        (self.status.activation, self.status.lifecycle) =
            prepared_renderer_state(self.current.is_some());
        self.status.n = self
            .current
            .as_ref()
            .map(|current| slot_status(current, CycleSlotState::Active));
        self.status.n1 = Some(slot_status(&plan, CycleSlotState::Ready));
        self.status.n2 = request.n2;
        self.status.process_id = self.process.as_ref().and_then(ManagedMpvProcess::pid);
        self.record_parameter_support(plan.parameter_support);
        self.record_session_identity();
        Ok(self.status.clone())
    }

    fn current_cpu4_plan(&self) -> Result<Option<RealtimeVideoPlan>, RealtimeVideoBackendError> {
        let (Some(plan), Some(context)) = (self.current.as_ref(), self.current_schedule.as_ref())
        else {
            return Ok(None);
        };
        if plan.parameter_support.backend == VideoBackend::Cpu4 {
            Ok(Some(plan.clone()))
        } else {
            cpu4_plan_from(plan, &context.video_params).map(Some)
        }
    }

    fn start_cpu4_process(
        &mut self,
        active_plan: Option<&RealtimeVideoPlan>,
        operation_revision: u64,
    ) -> Result<(), RealtimeVideoBackendError> {
        let session =
            self.session
                .clone()
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "CPU4 初始化",
                    message: "缺少视频会话上下文".to_owned(),
                })?;
        let mut launch_session = session.clone();
        launch_session.paused = true;
        let (mut process, _, mut decoder) = launch_mode_renderer(
            &launch_session,
            MpvLaunchMode::Cpu4,
            &self.operation_revision,
            operation_revision,
        )
        .map_err(|message| RealtimeVideoBackendError::ProcessFailed {
            operation: "CPU4 初始化",
            message,
        })?;
        let process_id = process
            .pid()
            .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                operation: "CPU4 初始化",
                message: "CPU4 mpv 启动后未取得进程标识".to_owned(),
            })?;
        if let Some(plan) = active_plan {
            for command in &plan.commands {
                self.ensure_operation_current(operation_revision, "CPU4 参数提交")?;
                process.send_command(command, MPV_COMMAND_TIMEOUT)?;
            }
        }
        self.ensure_operation_current(operation_revision, "CPU4 启动")?;
        if !session.paused {
            process.send_command(&MpvCommand::SetPause { paused: false }, MPV_COMMAND_TIMEOUT)?;
            decoder = wait_for_video_output(
                &mut process,
                &self.operation_revision,
                operation_revision,
                true,
            )
            .map_err(|error| RealtimeVideoBackendError::ProcessFailed {
                operation: "CPU4 初始化",
                message: backend_error_message(startup_failure_error(&process, error)),
            })?;
        }
        let (physical_paused, physical_eof_reached) =
            confirm_physical_process_facts(&process, Some(session.paused), "CPU4 初始化")?;
        self.process = Some(process);
        self.process_launch_mode = Some(MpvLaunchMode::Cpu4);
        self.process_host_window_id = Some(session.host_window_id);
        self.source_path = Some(session.source_path);
        self.reset_scheduler(
            session.playback_generation,
            session.clock_epoch,
            session.loop_index,
            session.paused,
        );
        self.advance_backend_epoch();
        self.claim_cpu4_status();
        self.status.decoder = Some(decoder);
        let presented_pts_ms = self
            .session
            .as_ref()
            .map(|session| session.source_position_ms)
            .unwrap_or_default();
        self.record_confirmed_process_facts(
            process_id,
            presented_pts_ms,
            physical_paused,
            physical_eof_reached,
        );
        (self.status.activation, self.status.lifecycle) =
            prepared_renderer_state(active_plan.is_some());
        self.status.n = active_plan.map(|plan| slot_status(plan, CycleSlotState::Active));
        self.status.n1 = None;
        if let Some(plan) = active_plan {
            self.current = Some(RealtimeVideoPlan {
                slot: VideoPlanSlot::N,
                ..plan.clone()
            });
            if self
                .pending_schedule
                .as_ref()
                .is_some_and(|context| context.identity == plan.identity)
            {
                self.current_schedule = self.pending_schedule.take();
            }
            self.pending = None;
            self.record_parameter_support(plan.parameter_support.clone());
        }
        Ok(())
    }

    fn start_original_fallback(
        &mut self,
        operation_revision: u64,
    ) -> Result<(), RealtimeVideoBackendError> {
        let session =
            self.session
                .clone()
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "Original 初始化",
                    message: "缺少视频会话上下文".to_owned(),
                })?;
        let mut launch_session = session.clone();
        launch_session.paused = true;
        let (process, graphics_api, decoder) = launch_mode_renderer(
            &launch_session,
            MpvLaunchMode::Original,
            &self.operation_revision,
            operation_revision,
        )
        .map_err(|message| RealtimeVideoBackendError::ProcessFailed {
            operation: "Original 初始化",
            message,
        })?;
        let process_id = process
            .pid()
            .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                operation: "Original 初始化",
                message: "Original mpv 启动后未取得进程标识".to_owned(),
            })?;
        self.ensure_operation_current(operation_revision, "Original 启动")?;
        if !session.paused {
            process.send_command(&MpvCommand::SetPause { paused: false }, MPV_COMMAND_TIMEOUT)?;
        }
        let (physical_paused, physical_eof_reached) =
            confirm_physical_process_facts(&process, Some(session.paused), "Original 初始化")?;
        self.process = Some(process);
        self.process_launch_mode = Some(MpvLaunchMode::Original);
        self.process_host_window_id = Some(session.host_window_id);
        self.source_path = Some(session.source_path);
        self.reset_scheduler(
            session.playback_generation,
            session.clock_epoch,
            session.loop_index,
            session.paused,
        );
        self.record_original_started(
            graphics_api,
            decoder,
            process_id,
            session.source_position_ms,
            physical_paused,
            physical_eof_reached,
        );
        self.pending = None;
        self.current = None;
        self.pending_schedule = None;
        self.current_schedule = None;
        Ok(())
    }

    fn handle_process_failure(
        &mut self,
        reason: String,
        now_unix_ms: u64,
        cpu4_plan: Option<RealtimeVideoPlan>,
        apply_cpu4_plan: bool,
    ) {
        if let Some(sequence) = self.pending.as_ref().map(|plan| plan.identity.sequence) {
            if let Some(controller) = self.cycle_controller.as_mut() {
                let _ = controller.handle(VideoCycleEvent::ApplyResult {
                    sequence,
                    source_pts_ms: self.status.presented_pts_ms.unwrap_or(0),
                    result: VideoCycleApplyResult::Failed,
                });
            }
        }
        self.set_apply_state(VideoApplyState::Failed);
        let processing_bypassed = self.status.backend == VideoBackend::Source;
        if self.process_launch_mode.is_none()
            || self.process_launch_mode == Some(MpvLaunchMode::Original)
            || self.backend.current() == VideoBackend::Source
        {
            self.record_original_failure(reason);
            self.stop_process();
            return;
        }
        let operation_revision = self.operation_revision.load(Ordering::Acquire);
        self.transition_to_fallback(
            reason,
            now_unix_ms,
            cpu4_plan,
            apply_cpu4_plan,
            operation_revision,
        );
        if processing_bypassed
            && self.process.is_some()
            && self.status.backend != VideoBackend::Source
        {
            if let Err(error) = self.neutralize_to_source_with_fallback() {
                self.record_original_failure(format!("降级后无法恢复中性视频旁路：{error}"));
                self.stop_process();
            }
        }
    }

    fn transition_to_fallback(
        &mut self,
        reason: String,
        now_unix_ms: u64,
        cpu4_plan: Option<RealtimeVideoPlan>,
        apply_cpu4_plan: bool,
        operation_revision: u64,
    ) {
        if self.operation_cancelled(operation_revision) {
            return;
        }
        if self.backend.current() == VideoBackend::Source {
            self.record_original_failure(reason);
            self.stop_process();
            return;
        }
        let from_mode = self.backend.launch_mode().name().to_owned();
        let logged_reason = self.bounded_failure_reason(&reason);
        self.begin_frame_budget_transition();
        let observed_pts_ms = self.process.as_ref().and_then(|process| {
            process
                .read_video_observation(OBSERVATION_COMMAND_DEADLINE)
                .ok()
                .flatten()
                .map(|observation| observation.media_pts_ms)
        });
        if let (Some(session), Some(observed_pts_ms)) = (self.session.as_mut(), observed_pts_ms) {
            session.source_position_ms = observed_pts_ms;
        }
        let resume_pts_ms = self
            .session
            .as_ref()
            .map(|session| session.source_position_ms);
        self.status.transition_started_at_unix_ms = Some(now_unix_ms);
        self.status.transition_completed_at_unix_ms = None;
        self.status.resume_pts_ms = resume_pts_ms;
        self.demote(reason, now_unix_ms, self.status.parameter_support.clone());
        eprintln!(
            "[realtime-video-fallback] from={} to={} reason={}",
            from_mode,
            self.backend.launch_mode().name(),
            logged_reason
        );
        if self.backend.launch_mode() == MpvLaunchMode::Cpu4 {
            if let Some(plan) = cpu4_plan.as_ref() {
                self.record_parameter_support(plan.parameter_support.clone());
            }
        }
        self.stop_process();
        let result = match self.backend.launch_mode() {
            MpvLaunchMode::Gpu(gpu_profile) => {
                self.start_gpu_fallback(gpu_profile, operation_revision)
            }
            MpvLaunchMode::Cpu4 => self.start_cpu4_process(
                apply_cpu4_plan.then_some(cpu4_plan.as_ref()).flatten(),
                operation_revision,
            ),
            MpvLaunchMode::Original => self.start_original_fallback(operation_revision),
        };
        match result {
            Ok(()) => {
                self.status.transition_completed_at_unix_ms = Some(unix_now_ms());
                self.status.resume_pts_ms = resume_pts_ms;
                self.record_session_identity();
            }
            Err(error) if self.backend.launch_mode() != MpvLaunchMode::Original => {
                self.transition_to_fallback(
                    error.to_string(),
                    unix_now_ms(),
                    cpu4_plan,
                    apply_cpu4_plan,
                    operation_revision,
                );
            }
            Err(error) => {
                self.record_original_failure(error.to_string());
                self.stop_process();
            }
        }
    }

    fn start_renderer(
        &mut self,
        request: &PrepareRealtimeRenderer,
        source: &Path,
        operation_revision: u64,
    ) -> Result<(), RealtimeVideoBackendError> {
        let MpvLaunchMode::Gpu(gpu_profile) = self.backend.launch_mode() else {
            return Err(RealtimeVideoBackendError::ProcessFailed {
                operation: "GPU 初始化",
                message: "当前后端 floor 不是 GPU".to_owned(),
            });
        };
        let session =
            self.session
                .clone()
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "GPU 初始化",
                    message: "缺少视频会话上下文".to_owned(),
                })?;
        let replay_commands =
            gpu_replay_commands(self.current.as_ref(), self.last_shader_options.as_ref());
        let (process, graphics_api, decoder) = launch_gpu_renderer(
            &session,
            request.shader.clone(),
            gpu_profile,
            &replay_commands,
            &self.operation_revision,
            operation_revision,
        )
        .map_err(|message| RealtimeVideoBackendError::ProcessFailed {
            operation: "GPU 初始化",
            message,
        })?;
        let process_id = process
            .pid()
            .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                operation: "GPU 初始化",
                message: "GPU mpv 启动后未取得进程标识".to_owned(),
            })?;
        let (physical_paused, physical_eof_reached) =
            confirm_physical_process_facts(&process, Some(session.paused), "GPU 初始化")?;
        self.ensure_operation_current(operation_revision, "GPU 启动")?;
        self.process = Some(process);
        self.process_launch_mode = Some(MpvLaunchMode::Gpu(gpu_profile));
        self.process_host_window_id = Some(session.host_window_id);
        self.source_path = Some(source.to_path_buf());
        self.record_renderer_started(
            graphics_api,
            decoder,
            process_id,
            session.source_position_ms,
            physical_paused,
            physical_eof_reached,
        );
        Ok(())
    }

    fn start_gpu_fallback(
        &mut self,
        gpu_profile: MpvGpuProfile,
        operation_revision: u64,
    ) -> Result<(), RealtimeVideoBackendError> {
        let session =
            self.session
                .clone()
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "GPU 初始化",
                    message: "缺少视频会话上下文".to_owned(),
                })?;
        let shader =
            session
                .shader
                .clone()
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "GPU 初始化",
                    message: "缺少已验证 shader".to_owned(),
                })?;
        let replay_commands =
            gpu_replay_commands(self.current.as_ref(), self.last_shader_options.as_ref());
        let replayed_shader_options = replay_commands.iter().find_map(|command| match command {
            MpvCommand::SetShaderOptions { options } => Some(options.clone()),
            _ => None,
        });
        let (process, graphics_api, decoder) = launch_gpu_renderer(
            &session,
            shader,
            gpu_profile,
            &replay_commands,
            &self.operation_revision,
            operation_revision,
        )
        .map_err(|message| RealtimeVideoBackendError::ProcessFailed {
            operation: "GPU 初始化",
            message,
        })?;
        let process_id = process
            .pid()
            .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                operation: "GPU 初始化",
                message: "GPU mpv 启动后未取得进程标识".to_owned(),
            })?;
        let (physical_paused, physical_eof_reached) =
            confirm_physical_process_facts(&process, Some(session.paused), "GPU 降级启动")?;
        self.ensure_operation_current(operation_revision, "GPU 降级启动")?;
        self.process = Some(process);
        self.process_launch_mode = Some(MpvLaunchMode::Gpu(gpu_profile));
        self.process_host_window_id = Some(session.host_window_id);
        self.source_path = Some(session.source_path);
        self.reset_scheduler(
            session.playback_generation,
            session.clock_epoch,
            session.loop_index,
            session.paused,
        );
        self.last_shader_options = replayed_shader_options;
        self.record_renderer_started(
            graphics_api,
            decoder,
            process_id,
            session.source_position_ms,
            physical_paused,
            physical_eof_reached,
        );
        Ok(())
    }

    fn start_neutral_process(
        &mut self,
        mode: MpvLaunchMode,
        operation_revision: u64,
    ) -> Result<(), RealtimeVideoBackendError> {
        match mode {
            MpvLaunchMode::Gpu(profile) => self.start_gpu_fallback(profile, operation_revision),
            MpvLaunchMode::Cpu4 => self.start_cpu4_process(None, operation_revision),
            MpvLaunchMode::Original => self.start_original_fallback(operation_revision),
        }
    }

    fn record_original_failure(&mut self, reason: String) {
        let reason = self.bounded_failure_reason(&reason);
        let last_demotion = self.backend.last_demotion().cloned();
        let transition_started_at_unix_ms = self.status.transition_started_at_unix_ms;
        let resume_pts_ms = self.status.resume_pts_ms;
        self.status = MediaVideoBackendRuntimeStatus::default();
        self.status.activation = BackendActivation::Failed;
        self.status.lifecycle = RendererLifecycleState::Failed;
        self.status.demotion_reason = Some(reason);
        self.status.last_demotion = last_demotion;
        self.status.demotion_history = self.backend.demotion_history().to_vec();
        self.status.transition_started_at_unix_ms = transition_started_at_unix_ms;
        self.status.transition_completed_at_unix_ms = Some(unix_now_ms());
        self.status.resume_pts_ms = resume_pts_ms;
        self.record_session_identity();
    }

    fn record_original_started(
        &mut self,
        graphics_api: MpvGraphicsApi,
        decoder: String,
        process_id: u32,
        presented_pts_ms: u64,
        physical_paused: bool,
        physical_eof_reached: bool,
    ) {
        let last_demotion = self.backend.last_demotion().cloned();
        let demotion_reason = last_demotion
            .as_ref()
            .map(|demotion| demotion.reason.clone());
        let transition_started_at_unix_ms = self.status.transition_started_at_unix_ms;
        let resume_pts_ms = self.status.resume_pts_ms;
        self.status = MediaVideoBackendRuntimeStatus::default();
        self.advance_backend_epoch();
        self.status.activation = BackendActivation::Active;
        self.status.lifecycle = RendererLifecycleState::Active;
        self.status.graphics_api = Some(graphics_api_name(graphics_api).to_owned());
        self.status.decoder = Some(decoder);
        self.status.filter = Some("Original（无 shader）".to_owned());
        self.status.demotion_reason = demotion_reason;
        self.status.last_demotion = last_demotion;
        self.status.demotion_history = self.backend.demotion_history().to_vec();
        self.status.transition_started_at_unix_ms = transition_started_at_unix_ms;
        self.status.resume_pts_ms = resume_pts_ms;
        self.record_confirmed_process_facts(
            process_id,
            presented_pts_ms,
            physical_paused,
            physical_eof_reached,
        );
        self.record_session_identity();
    }

    fn record_neutral_source_status(&mut self) -> Result<(), RealtimeVideoBackendError> {
        let graphics_api = self.status.graphics_api.clone();
        let decoder = self.status.decoder.clone();
        let av_sync_drift_ms = self.status.av_sync_drift_ms;
        let audible_audio_pts_ms = self.status.audible_audio_pts_ms;
        let audio_epoch = self.status.audio_epoch;
        let process =
            self.process
                .as_ref()
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "关闭视频处理",
                    message: "mpv 进程不存在".to_owned(),
                })?;
        let process_id = process
            .pid()
            .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                operation: "关闭视频处理",
                message: "mpv 进程标识丢失".to_owned(),
            })?;
        let (physical_paused, physical_eof_reached) =
            confirm_physical_process_facts(process, None, "关闭视频处理")?;
        let last_demotion = self.backend.last_demotion().cloned();
        let demotion_reason = last_demotion
            .as_ref()
            .map(|demotion| demotion.reason.clone());
        let transition_started_at_unix_ms = self.status.transition_started_at_unix_ms;
        let transition_completed_at_unix_ms = self.status.transition_completed_at_unix_ms;
        let resume_pts_ms = self.status.resume_pts_ms;
        self.status = MediaVideoBackendRuntimeStatus::default();
        self.status.activation = BackendActivation::Active;
        self.status.lifecycle = RendererLifecycleState::Active;
        self.status.graphics_api = graphics_api;
        self.status.decoder = decoder;
        self.status.av_sync_drift_ms = av_sync_drift_ms;
        self.status.audible_audio_pts_ms = audible_audio_pts_ms;
        self.status.audio_epoch = audio_epoch;
        self.status.filter = Some(
            match self.process_launch_mode {
                Some(MpvLaunchMode::Gpu(_)) => "Original（中性 shader）",
                Some(MpvLaunchMode::Cpu4) => "Original（CPU4 中性参数）",
                Some(MpvLaunchMode::Original) | None => "Original（无 shader）",
            }
            .to_owned(),
        );
        self.status.demotion_reason = demotion_reason;
        self.status.last_demotion = last_demotion;
        self.status.demotion_history = self.backend.demotion_history().to_vec();
        self.status.transition_started_at_unix_ms = transition_started_at_unix_ms;
        self.status.transition_completed_at_unix_ms = transition_completed_at_unix_ms;
        self.status.resume_pts_ms = resume_pts_ms;
        let presented_pts_ms = self
            .session
            .as_ref()
            .map(|session| session.source_position_ms)
            .unwrap_or_default();
        self.record_confirmed_process_facts(
            process_id,
            presented_pts_ms,
            physical_paused,
            physical_eof_reached,
        );
        self.record_session_identity();
        Ok(())
    }

    fn demote(
        &mut self,
        reason: impl Into<String>,
        now_unix_ms: u64,
        support: ParameterSupportReport,
    ) {
        let reason = reason.into();
        let reason = self.bounded_failure_reason(&reason);
        let next = self.backend.demote(reason.clone(), now_unix_ms);
        self.status.backend = next;
        self.status.activation = BackendActivation::Configured;
        self.status.lifecycle = RendererLifecycleState::Probing;
        self.status.n = None;
        self.status.n1 = None;
        self.status.n2 = None;
        self.status.active_plan_fingerprint = None;
        self.status.pending_plan_fingerprint = None;
        self.pending_shader_apply = None;
        self.set_apply_state(VideoApplyState::Failed);
        self.status.demotion_reason = Some(reason);
        self.status.gpu_adapter = None;
        self.status.graphics_api = (next == VideoBackend::Cpu4).then(|| "d3d11".to_owned());
        self.status.decoder = (next == VideoBackend::Cpu4).then(|| "software".to_owned());
        self.status.filter =
            (next == VideoBackend::Cpu4).then(|| "libavfilter eq+hue（待启动）".to_owned());
        self.record_parameter_support(match next {
            VideoBackend::Cpu4 => cpu4_parameter_support_from(&support),
            VideoBackend::Source => ParameterSupportReport::empty(VideoBackend::Source),
            VideoBackend::RealtimeGpu => support,
        });
        self.status.last_demotion = self.backend.last_demotion().cloned();
        self.status.demotion_history = self.backend.demotion_history().to_vec();
        self.reset_av_sync();
        self.clear_physical_process_facts();
        self.record_session_identity();
    }

    fn record_parameter_support(&mut self, support: ParameterSupportReport) {
        self.status.unsupported_parameter_count = support
            .parameters
            .iter()
            .filter(|parameter| !parameter.supported)
            .count();
        self.status.ignored_active_parameter_count = support.ignored_active_parameter_count();
        self.status.ignored_active_parameter_examples =
            support.ignored_active_parameter_examples(3);
        self.status.support_completeness = if support.backend == VideoBackend::Source {
            SupportCompleteness::NotApplicable
        } else if support.fully_supported {
            SupportCompleteness::Complete
        } else {
            SupportCompleteness::Incomplete
        };
        self.status.parameter_support = support;
    }

    fn record_session_identity(&mut self) {
        if let Some(session) = self.session.as_ref() {
            self.status.playback_generation = Some(session.playback_generation);
            self.status.clock_epoch = Some(session.clock_epoch);
            self.status.loop_index = Some(session.loop_index);
        }
        self.status.backend_epoch = self.backend_epoch;
        self.status.fallback_floor_mode = self.backend.launch_mode().name().to_owned();
        if let Some(controller) = self.cycle_controller.as_ref() {
            self.status.confirmed_change_count = controller.confirmed_change_count();
        }
        self.touch_status();
    }

    fn advance_backend_epoch(&mut self) {
        self.clear_eof_fact();
        self.backend_epoch = self.backend_epoch.saturating_add(1);
        self.record_session_identity();
    }

    fn claim_realtime_status(&mut self) {
        self.status.backend = VideoBackend::RealtimeGpu;
        self.status.gpu_adapter = None;
        self.status.filter = Some("gpu-next/libplacebo".to_owned());
        self.status.last_demotion = self.backend.last_demotion().cloned();
        self.status.demotion_history = self.backend.demotion_history().to_vec();
        self.status.demotion_reason = self
            .status
            .last_demotion
            .as_ref()
            .map(|demotion| demotion.reason.clone());
        self.status.cycle_drift_ms = None;
        self.record_session_identity();
    }

    fn claim_cycle_backend_status(&mut self) {
        match self.process_launch_mode {
            Some(MpvLaunchMode::Gpu(_)) => {
                self.claim_realtime_status();
                self.record_parameter_support(gpu83_cycle_parameter_support());
            }
            Some(MpvLaunchMode::Cpu4) => {
                self.claim_cpu4_status();
                self.record_parameter_support(cpu4_parameter_support_from(
                    &gpu83_cycle_parameter_support(),
                ));
            }
            Some(MpvLaunchMode::Original) | None => {}
        }
    }

    fn claim_cpu4_status(&mut self) {
        self.status.backend = VideoBackend::Cpu4;
        self.status.gpu_adapter = None;
        self.status.graphics_api = Some("d3d11".to_owned());
        self.status.decoder = Some("software".to_owned());
        self.status.filter = Some("libavfilter eq+hue".to_owned());
        self.status.last_demotion = self.backend.last_demotion().cloned();
        self.status.demotion_history = self.backend.demotion_history().to_vec();
        self.status.demotion_reason = self
            .status
            .last_demotion
            .as_ref()
            .map(|demotion| demotion.reason.clone());
        self.record_session_identity();
    }

    fn record_renderer_started(
        &mut self,
        graphics_api: MpvGraphicsApi,
        decoder: String,
        process_id: u32,
        presented_pts_ms: u64,
        physical_paused: bool,
        physical_eof_reached: bool,
    ) {
        self.advance_backend_epoch();
        // fallback/rebuild 会在启动命令中重放最后已确认的 N；发布新进程事实时，
        // 生命周期和槽位必须一起恢复，禁止出现 Active N + Available。
        (self.status.activation, self.status.lifecycle) =
            prepared_renderer_state(self.current.is_some());
        self.status.n = self
            .current
            .as_ref()
            .map(|plan| slot_status(plan, CycleSlotState::Active));
        self.status.graphics_api = Some(graphics_api_name(graphics_api).to_owned());
        self.status.decoder = Some(decoder);
        self.status.filter = Some("gpu-next/libplacebo".to_owned());
        self.record_confirmed_process_facts(
            process_id,
            presented_pts_ms,
            physical_paused,
            physical_eof_reached,
        );
        self.claim_realtime_status();
    }

    fn record_confirmed_process_facts(
        &mut self,
        process_id: u32,
        presented_pts_ms: u64,
        physical_paused: bool,
        physical_eof_reached: bool,
    ) {
        self.status.process_id = Some(process_id);
        self.status.presented_pts_ms = Some(presented_pts_ms);
        self.status.physical_paused = Some(physical_paused);
        self.status.physical_eof_reached = Some(physical_eof_reached);
        if physical_eof_reached {
            self.status.eof =
                eof_fact_from_cursor(self.sync_cursor, self.backend_epoch, physical_eof_reached);
        } else {
            self.clear_eof_fact();
        }
    }

    fn clear_physical_process_facts(&mut self) {
        self.status.process_id = None;
        self.status.presented_pts_ms = None;
        self.status.physical_paused = None;
        self.status.physical_eof_reached = None;
        self.clear_eof_fact();
    }

    fn bounded_failure_reason(&self, reason: &str) -> String {
        let mut sensitive_paths = Vec::with_capacity(3);
        if let Some(session) = self.session.as_ref() {
            sensitive_paths.push(session.source_path.as_path());
            sensitive_paths.push(session.executable.as_path());
            if let Some(shader) = session.shader.as_ref() {
                sensitive_paths.push(shader.as_path());
            }
        }
        bounded_runtime_reason(reason, &sensitive_paths)
    }

    fn stop_process(&mut self) {
        self.pending_shader_apply = None;
        self.pending_source_fps_response = None;
        self.pending_playback_observation = None;
        self.presented_pts_watchdog.reset();
        if let Some(mut process) = self.process.take() {
            let _ignored = process.cancel();
        }
        self.process_launch_mode = None;
        self.process_host_window_id = None;
        self.source_path = None;
        self.clear_physical_process_facts();
    }
}

fn confirm_physical_process_facts(
    process: &ManagedMpvProcess,
    expected_paused: Option<bool>,
    operation: &'static str,
) -> Result<(bool, bool), RealtimeVideoBackendError> {
    let physical_paused = process.read_paused(MPV_COMMAND_TIMEOUT)?;
    if expected_paused.is_some_and(|expected| expected != physical_paused) {
        return Err(RealtimeVideoBackendError::ProcessFailed {
            operation,
            message: format!(
                "mpv 物理暂停状态未确认：期望 {expected_paused:?}，实际 {physical_paused}"
            ),
        });
    }
    let physical_eof_reached = process.read_eof_reached(MPV_COMMAND_TIMEOUT)?;
    Ok((physical_paused, physical_eof_reached))
}

fn launch_gpu_renderer(
    session: &RendererSessionContext,
    shader: VerifiedMpvShader,
    gpu_profile: MpvGpuProfile,
    startup_commands: &[MpvCommand],
    operation_revision: &AtomicU64,
    expected_operation_revision: u64,
) -> Result<(ManagedMpvProcess, MpvGraphicsApi, String), String> {
    let pipe = format!(
        r"\\.\pipe\autolive-mpv-{}-{}",
        std::process::id(),
        session.session_id
    );
    let spec = MpvLaunchSpec::new_with_shader(
        session.executable.clone(),
        &session.source_path,
        session.host_window_id,
        &pipe,
        gpu_profile,
        session.source_position_ms,
        session.paused,
        shader,
    )
    .map_err(backend_error_message)?;
    let mut process =
        spawn_connected_cancelable(&spec, operation_revision, expected_operation_revision)
            .map_err(backend_error_message)?;
    for command in startup_commands {
        if let Err(error) = process.send_command(command, MPV_COMMAND_TIMEOUT) {
            let error = startup_failure_error(&process, error);
            let _ignored = process.cancel();
            return Err(backend_error_message(error));
        }
    }
    if let Err(error) = restore_requested_playback_state(&process, session.paused) {
        let _ignored = process.cancel();
        return Err(backend_error_message(startup_failure_error(
            &process, error,
        )));
    }
    let decoder = match wait_for_video_output(
        &mut process,
        operation_revision,
        expected_operation_revision,
        !session.paused,
    ) {
        Ok(decoder) => decoder,
        Err(error) => {
            let _ignored = process.cancel();
            return Err(backend_error_message(startup_failure_error(
                &process, error,
            )));
        }
    };
    Ok((process, gpu_profile.graphics_api(), decoder))
}

fn launch_mode_renderer(
    session: &RendererSessionContext,
    mode: MpvLaunchMode,
    operation_revision: &AtomicU64,
    expected_operation_revision: u64,
) -> Result<(ManagedMpvProcess, MpvGraphicsApi, String), String> {
    let pipe = format!(
        r"\\.\pipe\autolive-mpv-{}-{}",
        std::process::id(),
        session.session_id
    );
    let spec = MpvLaunchSpec::new(
        session.executable.clone(),
        &session.source_path,
        session.host_window_id,
        &pipe,
        mode,
        session.source_position_ms,
        session.paused,
    )
    .map_err(backend_error_message)?;
    let graphics_api = spec.graphics_api();
    let mut process =
        spawn_connected_cancelable(&spec, operation_revision, expected_operation_revision)
            .map_err(backend_error_message)?;
    if let Err(error) = restore_requested_playback_state(&process, session.paused) {
        let _ignored = process.cancel();
        return Err(backend_error_message(startup_failure_error(
            &process, error,
        )));
    }
    let decoder = match wait_for_video_output(
        &mut process,
        operation_revision,
        expected_operation_revision,
        mode != MpvLaunchMode::Original && !session.paused,
    ) {
        Ok(decoder) => decoder,
        Err(error) => {
            let _ignored = process.cancel();
            return Err(backend_error_message(startup_failure_error(
                &process, error,
            )));
        }
    };
    Ok((process, graphics_api, decoder))
}

fn restore_requested_playback_state(
    process: &ManagedMpvProcess,
    paused: bool,
) -> Result<(), RealtimeVideoBackendError> {
    if !paused {
        process.send_command(&MpvCommand::SetPause { paused: false }, MPV_COMMAND_TIMEOUT)?;
    }
    Ok(())
}

fn spawn_connected_cancelable(
    spec: &MpvLaunchSpec,
    operation_revision: &AtomicU64,
    expected_operation_revision: u64,
) -> Result<ManagedMpvProcess, RealtimeVideoBackendError> {
    let mut process = ManagedMpvProcess::spawn(spec)?;
    let started = Instant::now();
    let mut last_error = None;
    while started.elapsed() < MPV_PIPE_CONNECT_TIMEOUT {
        if operation_revision.load(Ordering::Acquire) != expected_operation_revision {
            let _ignored = process.cancel();
            return Err(RealtimeVideoBackendError::ProcessFailed {
                operation: "mpv 初始化",
                message: "视频后端转换已取消".to_owned(),
            });
        }
        if process.has_exited()? {
            let error = RealtimeVideoBackendError::ProcessFailed {
                operation: "mpv 初始化",
                message: "mpv 在 IPC 连接前退出".to_owned(),
            };
            let _ignored = process.cancel();
            return Err(startup_failure_error(&process, error));
        }
        let options = MpvIpcOptions {
            connect_timeout: Duration::from_millis(100),
            connect_poll_interval: MPV_PIPE_CONNECT_POLL,
            ..MpvIpcOptions::default()
        };
        match process.connect_ipc(spec.ipc_pipe(), options) {
            Ok(()) => return Ok(process),
            Err(error) => last_error = Some(error),
        }
    }
    let error = last_error.unwrap_or_else(|| RealtimeVideoBackendError::ProcessFailed {
        operation: "mpv 初始化",
        message: "等待 mpv IPC 超时".to_owned(),
    });
    let _ignored = process.cancel();
    Err(startup_failure_error(&process, error))
}

fn graphics_api_name(graphics_api: MpvGraphicsApi) -> &'static str {
    match graphics_api {
        MpvGraphicsApi::D3d11 => "d3d11",
        MpvGraphicsApi::Vulkan => "vulkan",
    }
}

fn validate_prepare_request(
    request: &PrepareRealtimeRenderer,
) -> Result<(), RealtimeVideoBackendError> {
    if request.plan.slot != VideoPlanSlot::NPlus1 {
        return Err(RealtimeVideoBackendError::ProcessFailed {
            operation: "计划准备",
            message: "实时画面只允许准备 N+1".to_owned(),
        });
    }
    if request.clock_epoch == 0 {
        return Err(RealtimeVideoBackendError::ProcessFailed {
            operation: "计划准备",
            message: "实时画面 clock_epoch 必须大于 0".to_owned(),
        });
    }
    if !valid_source_position(request.source_start_ms, request.source_duration_ms) {
        return Err(RealtimeVideoBackendError::ProcessFailed {
            operation: "计划准备",
            message: "实时画面源时长必须大于 0，且启动位置必须位于源内".to_owned(),
        });
    }
    let mut errors = Vec::new();
    if let Err(mut video_errors) = request.video_params.validate() {
        errors.append(&mut video_errors);
    }
    if let Err(mut advanced_errors) = request.advanced_params.validate() {
        errors.append(&mut advanced_errors);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(RealtimeVideoBackendError::ProcessFailed {
            operation: "计划准备",
            message: format!("视频调度参数校验失败（{} 项）", errors.len()),
        })
    }
}

fn valid_source_position(position_ms: u64, source_duration_ms: u64) -> bool {
    source_duration_ms > 0 && position_ms < source_duration_ms
}

fn validate_prepare_backend_epoch(
    request_backend_epoch: u64,
    current_backend_epoch: u64,
) -> Result<(), RealtimeVideoBackendError> {
    if request_backend_epoch != current_backend_epoch {
        Err(RealtimeVideoBackendError::StalePrepareBackendEpoch {
            requested: request_backend_epoch,
            current: current_backend_epoch,
        })
    } else {
        Ok(())
    }
}

fn resolve_sync_transition(
    current: SyncCursor,
    request: RealtimeVideoSync,
) -> Result<SyncTransition, RealtimeVideoBackendError> {
    if request.playback_generation != current.playback_generation {
        return Err(stale_sync_error("播放代次"));
    }
    if request.clock_epoch < current.clock_epoch {
        return Err(stale_sync_error("时钟 epoch"));
    }
    if request.loop_index < current.loop_index {
        return Err(stale_sync_error("循环序号"));
    }
    if request.loop_index > current.loop_index.saturating_add(1) {
        return Err(stale_sync_error("循环序号跨越超过一轮"));
    }
    let boundary = if request.loop_index > current.loop_index {
        VideoScheduleBoundary::LoopBoundary
    } else if request.clock_epoch > current.clock_epoch {
        VideoScheduleBoundary::UserSeek
    } else {
        VideoScheduleBoundary::None
    };
    Ok(SyncTransition {
        next: SyncCursor {
            playback_generation: request.playback_generation,
            clock_epoch: request.clock_epoch,
            loop_index: request.loop_index,
            paused: request.paused,
        },
        boundary,
    })
}

fn sync_command_batch(
    current: SyncCursor,
    transition: SyncTransition,
    position_ms: u64,
) -> [Option<MpvCommand>; 2] {
    let pause = (transition.next.paused != current.paused).then_some(MpvCommand::SetPause {
        paused: transition.next.paused,
    });
    if matches!(transition.boundary, VideoScheduleBoundary::None) {
        return [pause, None];
    }
    let seek = Some(MpvCommand::SeekAbsoluteMs { position_ms });
    if transition.next.paused {
        // 先暂停再 seek，避免对齐完成后继续推进。
        let pause = Some(MpvCommand::SetPause { paused: true });
        [pause, seek]
    } else {
        // keep-open 在 EOF 后会留下 pause=true；边界定位后必须无条件恢复播放。
        let resume = Some(MpvCommand::SetPause { paused: false });
        [seek, resume]
    }
}

fn sync_command_result_is_unknown(error: &RealtimeVideoBackendError) -> bool {
    matches!(error, RealtimeVideoBackendError::IpcTimeout { .. })
}

fn stale_sync_error(field: &'static str) -> RealtimeVideoBackendError {
    RealtimeVideoBackendError::StaleSync { field }
}

fn video_cycle_runtime_error(
    error: crate::media_video_cycle::VideoCycleError,
) -> RealtimeVideoBackendError {
    RealtimeVideoBackendError::ProcessFailed {
        operation: "Rust 视频周期",
        message: format!("mpv PTS 周期推进失败：{error:?}"),
    }
}

fn merge_pending_boundary(
    current: VideoScheduleBoundary,
    incoming: VideoScheduleBoundary,
) -> VideoScheduleBoundary {
    use VideoScheduleBoundary::{LoopBoundary, None, SourceChanged, Startup, UserSeek};
    match (current, incoming) {
        (boundary, None) | (None, boundary) => boundary,
        (Startup, _) | (_, Startup) => Startup,
        (SourceChanged, _) | (_, SourceChanged) => SourceChanged,
        (LoopBoundary, _) | (_, LoopBoundary) => LoopBoundary,
        (UserSeek, UserSeek) => UserSeek,
    }
}

fn neutral_gpu_shader_options() -> Result<MpvShaderOptions, RealtimeVideoBackendError> {
    let snapshot = build_gpu83_shader_snapshot(
        &VideoEffectParams::default(),
        &AdvancedEffectParams::default(),
    )
    .map_err(|error| RealtimeVideoBackendError::ProcessFailed {
        operation: "关闭视频处理",
        message: format!("默认 GPU83 参数无效（{}:{}）", error.field, error.code),
    })?;
    MpvShaderOptions::parse(snapshot.mpv_property_update().value)
}

fn gpu83_cycle_parameter_support() -> ParameterSupportReport {
    let parameters = GPU83_PARAMETER_MAPPINGS
        .iter()
        .map(|mapping| {
            let supported = matches!(
                mapping.capability,
                Gpu83ParameterCapability::ShaderParameter
                    | Gpu83ParameterCapability::ScheduledParameter
            );
            ParameterSupportResult {
                field: mapping.field_path.to_owned(),
                active: supported,
                supported,
                mapping: match mapping.capability {
                    Gpu83ParameterCapability::ShaderParameter => {
                        Some(format!("mpv_shader_option:{}", mapping.shader_option))
                    }
                    Gpu83ParameterCapability::ScheduledParameter => {
                        Some("mpv_pts_scheduler".to_owned())
                    }
                    Gpu83ParameterCapability::Unavailable(_) => None,
                },
                reason: match mapping.capability {
                    Gpu83ParameterCapability::Unavailable(reason) => Some(reason.to_owned()),
                    Gpu83ParameterCapability::ShaderParameter
                    | Gpu83ParameterCapability::ScheduledParameter => None,
                },
            }
        })
        .collect();
    ParameterSupportReport {
        backend: VideoBackend::RealtimeGpu,
        fully_supported: true,
        parameters,
    }
}

fn gpu_replay_commands(
    active_plan: Option<&RealtimeVideoPlan>,
    last_shader_options: Option<&MpvShaderOptions>,
) -> Vec<MpvCommand> {
    gpu_replay_shader_options(active_plan, last_shader_options)
        .map(|options| vec![MpvCommand::SetShaderOptions { options }])
        .unwrap_or_default()
}

fn gpu_replay_shader_options(
    active_plan: Option<&RealtimeVideoPlan>,
    last_shader_options: Option<&MpvShaderOptions>,
) -> Option<MpvShaderOptions> {
    let active_plan = active_plan?;
    last_shader_options.cloned().or_else(|| {
        active_plan
            .commands
            .iter()
            .find_map(|command| match command {
                MpvCommand::SetShaderOptions { options } => Some(options.clone()),
                _ => None,
            })
    })
}

fn sync_boundary_from_audio(boundary: AudioClockBoundary) -> SyncBoundary {
    match boundary {
        AudioClockBoundary::Startup => SyncBoundary::Startup,
        AudioClockBoundary::SourceChanged => SyncBoundary::SourceChanged,
        AudioClockBoundary::UserSeek => SyncBoundary::UserSeek,
        AudioClockBoundary::LoopBoundary => SyncBoundary::LoopBoundary,
        AudioClockBoundary::None => SyncBoundary::ClockAuthorityChanged,
    }
}

fn final_playback_speed(
    audio_playback_rate: f64,
    scheduler_base_speed: f64,
    av_sync_correction: f64,
) -> Option<MpvPlaybackSpeed> {
    compose_video_speed(
        audio_playback_rate,
        scheduler_base_speed,
        av_sync_correction,
    )
    .and_then(|speed| MpvPlaybackSpeed::new(speed).ok())
}

fn transient_observation_failure(error: &RealtimeVideoBackendError) -> bool {
    matches!(
        error,
        RealtimeVideoBackendError::PropertyUnavailable { .. }
            | RealtimeVideoBackendError::IpcTimeout { .. }
            | RealtimeVideoBackendError::IpcQueueFull
    )
}

fn startup_property_pending(error: &RealtimeVideoBackendError) -> bool {
    matches!(error, RealtimeVideoBackendError::PropertyUnavailable { .. })
}

fn startup_command_deadline(started: Instant) -> Duration {
    MPV_PIPE_CONNECT_TIMEOUT
        .saturating_sub(started.elapsed())
        .max(Duration::from_millis(1))
}

fn missing_video_observation_error() -> RealtimeVideoBackendError {
    RealtimeVideoBackendError::IpcProtocol(
        "mpv 活动态缺少有效 time-pos 或 estimated-vf-fps".to_owned(),
    )
}

fn missing_video_pts_error() -> RealtimeVideoBackendError {
    RealtimeVideoBackendError::IpcProtocol("mpv 活动态缺少有效 time-pos".to_owned())
}

fn startup_failure_error(
    process: &ManagedMpvProcess,
    error: RealtimeVideoBackendError,
) -> RealtimeVideoBackendError {
    let Some(detail) = process.startup_stderr_diagnostic() else {
        return error;
    };
    match error {
        RealtimeVideoBackendError::ProcessFailed { operation, message } => {
            RealtimeVideoBackendError::ProcessFailed {
                operation,
                message: format!("{message}；mpv stderr 末尾：{detail}"),
            }
        }
        error => RealtimeVideoBackendError::ProcessFailed {
            operation: "mpv 初始化",
            message: format!("{error}；mpv stderr 末尾：{detail}"),
        },
    }
}

fn backend_error_message(error: RealtimeVideoBackendError) -> String {
    match error {
        RealtimeVideoBackendError::ProcessFailed { message, .. } => message,
        error => error.to_string(),
    }
}

fn slot_status(plan: &RealtimeVideoPlan, status: CycleSlotState) -> CycleSlotStatus {
    CycleSlotStatus {
        sequence: plan.identity.sequence,
        target_pts_ms: plan.target_pts_ms,
        status,
    }
}

fn attach_shader_plan_fingerprint(
    options: &MpvShaderOptions,
    identity: &VideoPlanIdentity,
    clock_epoch: u64,
    loop_index: u64,
    actual_source_fps: f64,
) -> Result<(MpvShaderOptions, String), RealtimeVideoBackendError> {
    let fingerprint = canonical_shader_plan_fingerprint(
        options,
        identity,
        clock_epoch,
        loop_index,
        actual_source_fps,
    )?;
    let bytes = fingerprint.as_bytes();
    let parse_marker = |offset: usize| -> Result<u32, RealtimeVideoBackendError> {
        u32::from_str_radix(
            std::str::from_utf8(&bytes[offset..offset + 6]).map_err(|_| {
                RealtimeVideoBackendError::IpcProtocol("shader 指纹编码无效".to_owned())
            })?,
            16,
        )
        .map_err(|_| RealtimeVideoBackendError::IpcProtocol("shader 指纹编码无效".to_owned()))
    };
    let high = parse_marker(0)?;
    let low = parse_marker(6)?;
    let base = options
        .option_map()?
        .as_map()
        .iter()
        .filter(|(key, _)| *key != SHADER_PLAN_MARKER_HIGH && *key != SHADER_PLAN_MARKER_LOW)
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",");
    let value = format!("{base},{SHADER_PLAN_MARKER_HIGH}={high},{SHADER_PLAN_MARKER_LOW}={low}");
    Ok((MpvShaderOptions::parse(value)?, fingerprint))
}

fn canonical_shader_plan_fingerprint(
    options: &MpvShaderOptions,
    identity: &VideoPlanIdentity,
    clock_epoch: u64,
    loop_index: u64,
    actual_source_fps: f64,
) -> Result<String, RealtimeVideoBackendError> {
    if !actual_source_fps.is_finite() || !(1.0..=240.0).contains(&actual_source_fps) {
        return Err(RealtimeVideoBackendError::IpcProtocol(
            "shader 指纹缺少有效实际源 FPS".to_owned(),
        ));
    }
    let map = options.option_map()?;
    let mut hasher = Sha256::new();
    for value in [
        identity.session_id,
        identity.playback_generation,
        identity.source_revision,
        identity.parameter_revision,
        identity.sequence,
        clock_epoch,
        loop_index,
    ] {
        hasher.update(value.to_le_bytes());
    }
    hasher.update(actual_source_fps.to_bits().to_le_bytes());
    for (key, value) in map.as_map() {
        if key == SHADER_PLAN_MARKER_HIGH || key == SHADER_PLAN_MARKER_LOW {
            continue;
        }
        hasher.update(key.as_bytes());
        hasher.update([0]);
        hasher.update(value.as_bytes());
        hasher.update([0xff]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>())
}

fn plan_fingerprint(
    plan: &RealtimeVideoPlan,
    clock_epoch: u64,
    loop_index: u64,
    actual_source_fps: f64,
) -> Result<Option<String>, RealtimeVideoBackendError> {
    let shader = plan.commands.iter().find_map(|command| match command {
        MpvCommand::SetShaderOptions { options } => Some(options),
        _ => None,
    });
    if let Some(options) = shader {
        return canonical_shader_plan_fingerprint(
            options,
            &plan.identity,
            clock_epoch,
            loop_index,
            actual_source_fps,
        )
        .map(Some);
    }
    if plan.parameter_support.backend == VideoBackend::Cpu4 {
        return canonical_cpu4_plan_fingerprint(plan, clock_epoch, loop_index, actual_source_fps)
            .map(Some);
    }
    Ok(None)
}

fn canonical_cpu4_plan_fingerprint(
    plan: &RealtimeVideoPlan,
    clock_epoch: u64,
    loop_index: u64,
    actual_source_fps: f64,
) -> Result<String, RealtimeVideoBackendError> {
    if !actual_source_fps.is_finite() || !(1.0..=240.0).contains(&actual_source_fps) {
        return Err(RealtimeVideoBackendError::IpcProtocol(
            "CPU4 指纹缺少有效实际源 FPS".to_owned(),
        ));
    }
    if plan.commands.len() != 4
        || !plan
            .commands
            .iter()
            .all(|command| matches!(command, MpvCommand::UpdateCpu4Filter { .. }))
    {
        return Err(RealtimeVideoBackendError::IpcProtocol(
            "CPU4 指纹要求完整四命令计划".to_owned(),
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(b"cpu4-plan-v1\0");
    for value in [
        plan.identity.session_id,
        plan.identity.playback_generation,
        plan.identity.source_revision,
        plan.identity.parameter_revision,
        plan.identity.sequence,
        clock_epoch,
        loop_index,
    ] {
        hasher.update(value.to_le_bytes());
    }
    hasher.update(actual_source_fps.to_bits().to_le_bytes());
    for command in &plan.commands {
        let canonical = serde_json::to_vec(command)
            .map_err(|error| RealtimeVideoBackendError::SerializeCommand(error.to_string()))?;
        hasher.update((canonical.len() as u64).to_le_bytes());
        hasher.update(canonical);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn controller_cycle_slot_status(plan: &ControllerVideoCyclePlan) -> Option<CycleSlotStatus> {
    Some(CycleSlotStatus {
        sequence: plan.sequence,
        target_pts_ms: plan
            .identity
            .media
            .presentation_pts_ms(plan.target_source_pts_ms)?,
        status: CycleSlotState::Planned,
    })
}

fn prepared_renderer_state(has_active_plan: bool) -> (BackendActivation, RendererLifecycleState) {
    if has_active_plan {
        (BackendActivation::Active, RendererLifecycleState::Active)
    } else {
        (
            BackendActivation::Available,
            RendererLifecycleState::Spawned,
        )
    }
}

fn same_playback_identity(left: &VideoPlanIdentity, right: &VideoPlanIdentity) -> bool {
    left.session_id == right.session_id
        && left.playback_generation == right.playback_generation
        && left.source_revision == right.source_revision
}

fn cpu4_plan_from(
    plan: &RealtimeVideoPlan,
    video: &VideoEffectParams,
) -> Result<RealtimeVideoPlan, RealtimeVideoBackendError> {
    let snapshot = Cpu4Snapshot::from_ui(
        video.brightness_percent,
        video.contrast_percent,
        video.saturation_percent,
        video.hue_rotation_degrees,
    )?;
    Ok(RealtimeVideoPlan {
        commands: snapshot.commands().into_iter().collect(),
        parameter_support: cpu4_parameter_support_from(&plan.parameter_support),
        ..plan.clone()
    })
}

fn cpu4_parameter_support_from(source: &ParameterSupportReport) -> ParameterSupportReport {
    let parameters = GPU83_PARAMETER_MAPPINGS
        .iter()
        .map(|parameter| {
            let mapping = match parameter.field_path {
                "video.brightness_percent" => Some("libavfilter:eq:brightness"),
                "video.contrast_percent" => Some("libavfilter:eq:contrast"),
                "video.saturation_percent" => Some("libavfilter:eq:saturation"),
                "video.hue_rotation_degrees" => Some("libavfilter:hue:h"),
                _ => None,
            };
            let active = source
                .parameters
                .iter()
                .find(|candidate| candidate.field == parameter.field_path)
                .is_some_and(|candidate| candidate.active);
            ParameterSupportResult {
                field: parameter.field_path.to_owned(),
                active,
                supported: mapping.is_some(),
                mapping: mapping.map(str::to_owned),
                reason: mapping
                    .is_none()
                    .then(|| "CPU4 仅执行亮度、对比度、饱和度和色相".to_owned()),
            }
        })
        .collect::<Vec<_>>();
    let fully_supported = parameters
        .iter()
        .all(|parameter| !parameter.active || parameter.supported);
    ParameterSupportReport {
        backend: VideoBackend::Cpu4,
        fully_supported,
        parameters,
    }
}

fn response_nonnegative_integer(
    response: &serde_json::Value,
    property: &'static str,
) -> Result<u64, RealtimeVideoBackendError> {
    if response.get("error").and_then(serde_json::Value::as_str) != Some("success") {
        return Err(RealtimeVideoBackendError::IpcProtocol(format!(
            "mpv {property} 查询失败：{response}"
        )));
    }
    let value = response
        .get("data")
        .and_then(|value| {
            value.as_u64().or_else(|| {
                value.as_f64().and_then(|number| {
                    (number.is_finite()
                        && number >= 0.0
                        && number.fract() == 0.0
                        && number < u64::MAX as f64)
                        .then_some(number as u64)
                })
            })
        })
        .ok_or_else(|| {
            RealtimeVideoBackendError::IpcProtocol(format!(
                "mpv {property} 不是非负整数：{response}"
            ))
        })?;
    Ok(value)
}

fn optional_frame_timing_counter(
    response: Result<serde_json::Value, RealtimeVideoBackendError>,
    property: &'static str,
) -> Result<Option<u64>, RealtimeVideoBackendError> {
    match response {
        Ok(response) => response_nonnegative_integer(&response, property).map(Some),
        Err(RealtimeVideoBackendError::PropertyUnavailable { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

fn vo_passes_timing_snapshot(
    response: &serde_json::Value,
) -> Result<VoPassesTimingSnapshot, RealtimeVideoBackendError> {
    if response.get("error").and_then(serde_json::Value::as_str) != Some("success") {
        return Err(RealtimeVideoBackendError::IpcProtocol(format!(
            "mpv vo-passes 查询失败：{response}"
        )));
    }
    let passes = response
        .get("data")
        .and_then(|data| data.get("fresh"))
        .and_then(serde_json::Value::as_array)
        .filter(|passes| !passes.is_empty())
        .ok_or_else(|| vo_passes_telemetry_unavailable("fresh 为空"))?;
    let mut parsed = Vec::with_capacity(passes.len());
    for pass in passes {
        let description = pass
            .get("desc")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let occurrence = parsed
            .iter()
            .filter(|candidate: &&VoPassTimingSeries| candidate.description == description)
            .count();
        let samples_ns = pass
            .get("samples")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| vo_passes_telemetry_unavailable("pass 缺少 samples"))?
            .iter()
            .map(|value| value.as_f64())
            .collect::<Option<Vec<_>>>()
            .filter(|samples| {
                samples
                    .iter()
                    .all(|sample| sample.is_finite() && *sample >= 0.0)
            })
            .ok_or_else(|| vo_passes_telemetry_unavailable("samples 无效"))?;
        parsed.push(VoPassTimingSeries {
            description,
            occurrence,
            samples_ns,
        });
    }
    Ok(VoPassesTimingSnapshot { passes: parsed })
}

fn vo_passes_telemetry_unavailable(reason: &str) -> RealtimeVideoBackendError {
    RealtimeVideoBackendError::PropertyUnavailable {
        request_id: 0,
        operation: "get vo-passes",
        property_error: format!("vo-passes 遥测暂不可用：{reason}"),
    }
}

fn pass_samples_p99_ns(samples: &[f64]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len() - 1) * 99).div_ceil(100);
    Some(sorted[index])
}

fn vo_passes_snapshot_p99_ms(
    snapshot: &VoPassesTimingSnapshot,
) -> Result<f64, RealtimeVideoBackendError> {
    let mut total_p99_ns = 0.0;
    let mut rendered_pass_count = 0usize;
    for pass in &snapshot.passes {
        if let Some(p99_ns) = pass_samples_p99_ns(&pass.samples_ns) {
            total_p99_ns += p99_ns;
            rendered_pass_count += 1;
        }
    }
    if rendered_pass_count == 0 {
        return Err(RealtimeVideoBackendError::PropertyUnavailable {
            request_id: 0,
            operation: "get vo-passes",
            property_error: "vo-passes 尚无已渲染样本".to_owned(),
        });
    }
    Ok(total_p99_ns / 1_000_000.0)
}

#[cfg(test)]
fn vo_passes_p99_ms(response: &serde_json::Value) -> Result<f64, RealtimeVideoBackendError> {
    vo_passes_snapshot_p99_ms(&vo_passes_timing_snapshot(response)?)
}

fn appended_vo_pass_samples<'a>(previous: &[f64], current: &'a [f64]) -> Option<&'a [f64]> {
    if current.is_empty() || previous == current {
        return None;
    }
    if previous.len() < MPV_VO_PERF_SAMPLE_COUNT {
        return current
            .strip_prefix(previous)
            .filter(|samples| !samples.is_empty());
    }
    if current.len() != MPV_VO_PERF_SAMPLE_COUNT {
        return None;
    }
    for overlap in (1..MPV_VO_PERF_SAMPLE_COUNT).rev() {
        if previous[previous.len() - overlap..] == current[..overlap] {
            return Some(&current[overlap..]);
        }
    }
    // 固定 256 项环形窗口完全没有重叠，说明上一窗口已经整体退出样本池。
    Some(current)
}

#[cfg(test)]
fn incremental_vo_passes_p99_ms(
    previous: &VoPassesTimingSnapshot,
    current: &VoPassesTimingSnapshot,
) -> Option<f64> {
    new_vo_passes_p99_ms(previous, current, false)
}

fn new_vo_passes_p99_ms(
    previous: &VoPassesTimingSnapshot,
    current: &VoPassesTimingSnapshot,
    pts_advanced: bool,
) -> Option<f64> {
    let mut total_p99_ns = 0.0;
    let mut observed_passes = 0usize;
    for current_pass in &current.passes {
        let Some(previous_pass) = previous.passes.iter().find(|previous_pass| {
            previous_pass.description == current_pass.description
                && previous_pass.occurrence == current_pass.occurrence
        }) else {
            continue;
        };
        let p99_ns = appended_vo_pass_samples(&previous_pass.samples_ns, &current_pass.samples_ns)
            .and_then(pass_samples_p99_ns)
            .or_else(|| {
                (pts_advanced && previous_pass.samples_ns == current_pass.samples_ns)
                    .then(|| current_pass.samples_ns.last().copied())
                    .flatten()
            });
        let Some(p99_ns) = p99_ns else {
            continue;
        };
        total_p99_ns += p99_ns;
        observed_passes += 1;
    }
    (observed_passes > 0).then_some(total_p99_ns / 1_000_000.0)
}

fn counters_regressed(
    previous: (u64, u64, Option<u64>, Option<u64>),
    current: (u64, u64, Option<u64>, Option<u64>),
) -> bool {
    current.0 < previous.0
        || current.1 < previous.1
        || matches!((previous.2, current.2), (Some(previous), Some(current)) if current < previous)
        || matches!((previous.3, current.3), (Some(previous), Some(current)) if current < previous)
}

fn counters_increased(
    previous: (u64, u64, Option<u64>, Option<u64>),
    current: (u64, u64, Option<u64>, Option<u64>),
) -> bool {
    current.0 > previous.0
        || current.1 > previous.1
        || matches!((previous.2, current.2), (Some(previous), Some(current)) if current > previous)
        || matches!((previous.3, current.3), (Some(previous), Some(current)) if current > previous)
}

fn frame_health_counter_increase(
    previous: Option<(u64, u64, Option<u64>, Option<u64>)>,
    current: (u64, u64, Option<u64>, Option<u64>),
) -> Option<bool> {
    let previous = previous?;
    (!counters_regressed(previous, current)).then(|| counters_increased(previous, current))
}

fn advance_health_violation_count(current: u8, violated: bool) -> u8 {
    if violated {
        current.saturating_add(1)
    } else {
        0
    }
}

fn advance_hard_frame_health_violation_count(
    current: u8,
    unhealthy_counter_increased: Option<bool>,
) -> u8 {
    advance_health_violation_count(current, unhealthy_counter_increased == Some(true))
}

fn signed_delta(left: u64, right: u64) -> i64 {
    if left >= right {
        i64::try_from(left - right).unwrap_or(i64::MAX)
    } else {
        -i64::try_from(right - left).unwrap_or(i64::MAX)
    }
}

fn wait_for_video_output(
    process: &mut ManagedMpvProcess,
    operation_revision: &AtomicU64,
    expected_operation_revision: u64,
    require_render_sample: bool,
) -> Result<String, RealtimeVideoBackendError> {
    let started = Instant::now();
    let mut video_output_configured = false;
    let mut render_sample_observed = false;
    loop {
        if operation_revision.load(Ordering::Acquire) != expected_operation_revision {
            let _ignored = process.cancel();
            return Err(RealtimeVideoBackendError::ProcessFailed {
                operation: "mpv 初始化",
                message: "视频后端转换已取消".to_owned(),
            });
        }
        if let Some(message) = process.fatal_render_failure() {
            return Err(RealtimeVideoBackendError::ProcessFailed {
                operation: "mpv 渲染初始化",
                message,
            });
        }
        if process.has_exited()? {
            return Err(RealtimeVideoBackendError::ProcessFailed {
                operation: "mpv 初始化",
                message: "mpv 在视频输出确认前退出".to_owned(),
            });
        }
        if !video_output_configured {
            match process.send_command(
                &MpvCommand::GetVideoOutputConfigured,
                startup_command_deadline(started),
            ) {
                Ok(response)
                    if response.get("data").and_then(serde_json::Value::as_bool) == Some(true) =>
                {
                    video_output_configured = true;
                }
                Ok(_) if started.elapsed() < MPV_PIPE_CONNECT_TIMEOUT => {
                    thread::sleep(MPV_PIPE_CONNECT_POLL);
                    continue;
                }
                Err(ref error)
                    if startup_property_pending(error)
                        && started.elapsed() < MPV_PIPE_CONNECT_TIMEOUT =>
                {
                    thread::sleep(MPV_PIPE_CONNECT_POLL);
                    continue;
                }
                Ok(response) => {
                    return Err(RealtimeVideoBackendError::ProcessFailed {
                        operation: "mpv 初始化",
                        message: format!("mpv 视频输出未就绪：{response}"),
                    });
                }
                Err(error) => return Err(error),
            }
        }
        if !require_render_sample {
            // Original 没有 shader/滤镜 pass 门禁，但 vo-configured 只证明 VO 已建立。
            // 只有取得有效视频 PTS 后才允许前端释放 WebView 兼容画面；Original
            // 不参与 GPU/CPU4 的 FPS 调度和帧预算，不应等待 estimated-vf-fps。
            match process.read_video_pts(startup_command_deadline(started)) {
                Ok(Some(_)) => {
                    // 解码器是展示事实而非 Original 就绪条件。部分 HEVC 会话不返回
                    // hwdec-current；最佳努力读取失败时明确标记未报告，不伪造硬解。
                    return Ok(process
                        .read_active_decoder(OBSERVATION_COMMAND_DEADLINE)
                        .unwrap_or_else(|_| "unreported-original".to_owned()));
                }
                Ok(None) if started.elapsed() < MPV_PIPE_CONNECT_TIMEOUT => {
                    thread::sleep(MPV_PIPE_CONNECT_POLL);
                    continue;
                }
                Err(ref error)
                    if startup_property_pending(error)
                        && started.elapsed() < MPV_PIPE_CONNECT_TIMEOUT =>
                {
                    thread::sleep(MPV_PIPE_CONNECT_POLL);
                    continue;
                }
                Ok(None) => {
                    return Err(RealtimeVideoBackendError::ProcessFailed {
                        operation: "mpv 初始化",
                        message: "Original 视频输出已配置但未观察到首帧时间线".to_owned(),
                    });
                }
                Err(error) => return Err(error),
            }
        }
        if !render_sample_observed {
            match process.send_command(
                &MpvCommand::GetVideoOutputPasses,
                startup_command_deadline(started),
            ) {
                Ok(response) if vo_passes_has_rendered_frame(&response) => {
                    render_sample_observed = true;
                }
                Ok(_) if started.elapsed() < MPV_PIPE_CONNECT_TIMEOUT => {
                    thread::sleep(MPV_PIPE_CONNECT_POLL);
                    continue;
                }
                Err(ref error)
                    if startup_property_pending(error)
                        && started.elapsed() < MPV_PIPE_CONNECT_TIMEOUT =>
                {
                    thread::sleep(MPV_PIPE_CONNECT_POLL);
                    continue;
                }
                Ok(response) => {
                    return Err(RealtimeVideoBackendError::ProcessFailed {
                        operation: "mpv 初始化",
                        message: format!("mpv 首帧尚未呈现：{response}"),
                    });
                }
                Err(error) => return Err(error),
            }
        }
        match process.read_active_decoder(startup_command_deadline(started)) {
            Ok(decoder) => return Ok(decoder),
            Err(ref error)
                if startup_property_pending(error)
                    && started.elapsed() < MPV_PIPE_CONNECT_TIMEOUT =>
            {
                thread::sleep(MPV_PIPE_CONNECT_POLL);
            }
            Err(error) => return Err(error),
        }
    }
}

fn vo_passes_has_rendered_frame(response: &serde_json::Value) -> bool {
    response.get("error").and_then(serde_json::Value::as_str) == Some("success")
        && response
            .get("data")
            .and_then(|data| data.get("fresh"))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|passes| {
                !passes.is_empty()
                    && passes.iter().any(|pass| {
                        pass.get("count")
                            .and_then(serde_json::Value::as_u64)
                            .is_some_and(|count| count > 0)
                            || pass
                                .get("samples")
                                .and_then(serde_json::Value::as_array)
                                .is_some_and(|samples| !samples.is_empty())
                    })
            })
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn bounded_runtime_reason(reason: &str, sensitive_paths: &[&Path]) -> String {
    let mut sanitized = reason.to_owned();
    for path in sensitive_paths {
        let path = path.to_string_lossy();
        if !path.is_empty() {
            sanitized = sanitized.replace(path.as_ref(), "<path>");
        }
    }
    let compact = sanitized.split_whitespace().collect::<Vec<_>>().join(" ");
    let compact = if compact.is_empty() {
        "视频后端失败"
    } else {
        compact.as_str()
    };
    let mut bounded = String::new();
    let mut utf16_units = 0;
    for character in compact.chars() {
        let next_units = character.len_utf16();
        if utf16_units + next_units > MAX_RUNTIME_LABEL_UTF16_UNITS {
            break;
        }
        bounded.push(character);
        utf16_units += next_units;
    }
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_video_gpu_effects::GPU83_PARAMETER_MAPPINGS;
    use crate::realtime_video_backend::{
        resolve_mpv_executable, resolve_mpv_shader, ParameterSupportReport, VideoPlanIdentity,
    };
    use std::fs;
    use std::sync::{Arc, Barrier};

    fn sync_cursor() -> SyncCursor {
        SyncCursor {
            playback_generation: 7,
            clock_epoch: 10,
            loop_index: 3,
            paused: false,
        }
    }

    #[test]
    fn preparing_next_cycle_preserves_an_active_renderer_state() {
        assert_eq!(
            prepared_renderer_state(false),
            (
                BackendActivation::Available,
                RendererLifecycleState::Spawned
            )
        );
        assert_eq!(
            prepared_renderer_state(true),
            (BackendActivation::Active, RendererLifecycleState::Active)
        );
        let source = include_str!("realtime_video_runtime.rs");
        let prepare_next = source
            .split("    fn prepare_next_controller_plan(")
            .nth(1)
            .expect("controller prepare")
            .split("    fn compile_controller_plan(")
            .next()
            .expect("controller prepare end");
        assert!(prepare_next.contains(
            "if self.current.is_none() {\n            self.set_apply_state(VideoApplyState::Ready);"
        ));
    }

    #[test]
    fn original_gate_keeps_only_a_healthy_same_generation_effect_session() {
        let healthy = EffectSessionHealthFacts {
            processing_enabled: true,
            effect_backend: true,
            activation_healthy: true,
            lifecycle_healthy: true,
            status_has_pid: true,
            generation_matches: true,
            session_matches: true,
            process_running: true,
        };
        assert!(should_keep_effect_renderer(healthy));

        for failed in [
            EffectSessionHealthFacts {
                processing_enabled: false,
                ..healthy
            },
            EffectSessionHealthFacts {
                effect_backend: false,
                ..healthy
            },
            EffectSessionHealthFacts {
                lifecycle_healthy: false,
                ..healthy
            },
            EffectSessionHealthFacts {
                status_has_pid: false,
                ..healthy
            },
            EffectSessionHealthFacts {
                generation_matches: false,
                ..healthy
            },
            EffectSessionHealthFacts {
                session_matches: false,
                ..healthy
            },
            EffectSessionHealthFacts {
                process_running: false,
                ..healthy
            },
        ] {
            assert!(!should_keep_effect_renderer(failed));
        }
    }

    #[test]
    fn source_transition_with_an_installed_cycle_controller_owns_the_effect_session() {
        assert!(effect_session_is_owned_by_cycle(
            VideoBackend::RealtimeGpu,
            false,
            false,
        ));
        assert!(effect_session_is_owned_by_cycle(
            VideoBackend::Cpu4,
            false,
            false,
        ));
        assert!(effect_session_is_owned_by_cycle(
            VideoBackend::Source,
            true,
            true,
        ));
        assert!(!effect_session_is_owned_by_cycle(
            VideoBackend::Source,
            false,
            true,
        ));
        assert!(!effect_session_is_owned_by_cycle(
            VideoBackend::Source,
            true,
            false,
        ));
    }

    #[test]
    fn cycle_backend_claim_keeps_backend_and_support_report_consistent() {
        let mut gpu = RuntimeState {
            process_launch_mode: Some(MpvLaunchMode::Gpu(MpvGpuProfile::D3d11ZeroCopy)),
            ..RuntimeState::default()
        };
        gpu.claim_cycle_backend_status();
        assert_eq!(gpu.status.backend, VideoBackend::RealtimeGpu);
        assert_eq!(gpu.status.parameter_support.backend, gpu.status.backend);

        let mut cpu4 = RuntimeState {
            process_launch_mode: Some(MpvLaunchMode::Cpu4),
            ..RuntimeState::default()
        };
        cpu4.claim_cycle_backend_status();
        assert_eq!(cpu4.status.backend, VideoBackend::Cpu4);
        assert_eq!(cpu4.status.parameter_support.backend, cpu4.status.backend);
    }

    #[test]
    fn cycle_owned_status_recovery_separates_source_from_support_mismatch() {
        let source = include_str!("realtime_video_runtime.rs");
        let tick = source
            .split("    fn tick(&mut self)")
            .nth(1)
            .expect("runtime tick")
            .split(
                "if self.status.backend == VideoBackend::Source\n            && self.status.lifecycle",
            )
            .next()
            .expect("cycle-owned recovery end");
        let recovery = tick
            .split("if self.cycle_controller.is_some()")
            .nth(1)
            .expect("cycle-owned status recovery");
        assert!(!recovery.contains(
            "self.status.backend == VideoBackend::Source\n                || self.status.parameter_support.backend != self.status.backend",
        ));
        let source_status = recovery
            .find("self.status.backend == VideoBackend::Source")
            .expect("neutral Source branch");
        let transition = recovery
            .find("self.invalidate_cycle_before_source_recovery()")
            .expect("Source branch invalidates both runtime and controller plans");
        let support_mismatch = recovery
            .find("self.status.parameter_support.backend != self.status.backend")
            .expect("support-only recovery branch");
        assert!(source_status < transition && transition < support_mismatch);
        assert_eq!(
            recovery
                .matches("self.claim_cycle_backend_status()")
                .count(),
            2
        );
        assert!(!recovery[support_mismatch..]
            .contains("self.invalidate_cycle_before_source_recovery()"));
        assert!(!recovery.contains("self.status.activation = BackendActivation::Available"));

        let transition_body = source
            .split("    fn begin_source_transition(&mut self)")
            .nth(1)
            .expect("source transition")
            .split("    fn handle(")
            .next()
            .expect("source transition end");
        for stale_field in [
            "self.status.n = None",
            "self.status.n1 = None",
            "self.status.n2 = None",
            "self.status.active_plan_fingerprint = None",
            "self.status.pending_plan_fingerprint = None",
        ] {
            assert!(transition_body.contains(stale_field));
        }
    }

    #[test]
    fn source_recovery_invalidates_runtime_and_controller_cycle_state() {
        let source = include_str!("realtime_video_runtime.rs");
        let invalidation = source
            .split("    fn invalidate_cycle_before_source_recovery(&mut self)")
            .nth(1)
            .expect("source recovery invalidation")
            .split("    fn handle(")
            .next()
            .expect("source recovery invalidation end");
        for stale_state in [
            "self.pending = None",
            "self.current = None",
            "self.pending_schedule = None",
            "self.current_schedule = None",
            "controller.clear()",
            "self.begin_source_transition()",
        ] {
            assert!(invalidation.contains(stale_state));
        }
    }

    #[test]
    fn renderer_restart_normalizes_activation_and_confirmed_n_together() {
        let source = include_str!("realtime_video_runtime.rs");
        let started = source
            .split("    fn record_renderer_started(")
            .nth(1)
            .expect("renderer started")
            .split("    fn record_confirmed_process_facts(")
            .next()
            .expect("renderer started end");
        let state = started
            .find("prepared_renderer_state(self.current.is_some())")
            .expect("activation follows confirmed current plan");
        let slot = started
            .find("self.status.n = self")
            .expect("confirmed N follows current plan");
        assert!(state < slot);
    }

    #[test]
    fn active_plan_promotion_reclaims_support_from_the_physical_launch_mode() {
        let source = include_str!("realtime_video_runtime.rs");
        let finalize = source
            .split("    fn finalize_pending_plan(")
            .nth(1)
            .expect("pending plan promotion")
            .split("    fn synchronize(")
            .next()
            .expect("pending plan promotion end");

        assert!(finalize.contains("self.claim_cycle_backend_status()"));
        assert!(!finalize.contains("self.record_parameter_support(plan.parameter_support)"));
    }

    #[test]
    fn cpu4_prepare_restart_replays_the_last_confirmed_plan_before_publishing_active() {
        let source = include_str!("realtime_video_runtime.rs");
        let prepare_cpu4 = source
            .split("    fn prepare_cpu4(")
            .nth(1)
            .expect("CPU4 prepare")
            .split("    fn current_cpu4_plan(")
            .next()
            .expect("CPU4 prepare end");
        let restart = prepare_cpu4
            .split("        if must_restart {")
            .nth(1)
            .expect("CPU4 restart branch");
        let replay = restart
            .find("self.current_cpu4_plan()?")
            .expect("last confirmed CPU4 plan snapshot");
        let stop = restart.find("self.stop_process()").expect("old CPU4 stop");
        let start = restart
            .find("self.start_cpu4_process(active_plan.as_ref(), operation_revision)")
            .expect("CPU4 restart with the confirmed plan");

        assert!(replay < stop && stop < start);
    }

    #[test]
    fn same_source_process_is_retained_only_for_the_same_mode_and_host() {
        assert!(retain_same_source_process(true, true, true, true));
        assert!(!retain_same_source_process(false, true, true, true));
        assert!(!retain_same_source_process(true, false, true, true));
        assert!(!retain_same_source_process(true, true, false, true));
        assert!(!retain_same_source_process(true, true, true, false));
    }

    #[test]
    fn sync_ipc_timeout_keeps_session_for_status_confirmation() {
        assert!(sync_command_result_is_unknown(
            &RealtimeVideoBackendError::IpcTimeout {
                request_id: 11,
                operation: "set pause",
            }
        ));
        assert!(!sync_command_result_is_unknown(
            &RealtimeVideoBackendError::IpcDisconnected("pipe closed".to_owned())
        ));
    }

    fn demote_all_gpu_profiles(runtime: &mut RuntimeState) {
        for (index, reason) in [
            "D3D11 zero-copy 失败",
            "D3D11 copy 失败",
            "Vulkan copy 失败",
            "GPU 软件解码失败",
        ]
        .into_iter()
        .enumerate()
        {
            runtime.demote(
                reason.to_owned(),
                index as u64 + 1,
                ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
            );
        }
        assert_eq!(runtime.backend.launch_mode(), MpvLaunchMode::Cpu4);
    }

    fn stale_epoch_prepare_request(root: &Path) -> PrepareRealtimeRenderer {
        let binaries = root.join("binaries");
        fs::create_dir_all(&binaries).expect("create fake runtime root");
        fs::write(binaries.join("mpv.exe"), b"fake mpv").expect("write fake mpv");
        fs::write(root.join("gpu83.hook"), b"//!HOOK MAIN\n").expect("write fake shader");
        let source_path = root.join("source.mp4");
        fs::write(&source_path, b"fake media").expect("write fake source");

        PrepareRealtimeRenderer {
            executable: resolve_mpv_executable(root).expect("resolve fake mpv"),
            shader: resolve_mpv_shader(root, &root.join("gpu83.hook"))
                .expect("resolve fake shader"),
            source_path,
            host_window_id: 1,
            source_start_ms: 0,
            source_duration_ms: 72_300,
            paused: false,
            plan: RealtimeVideoPlan {
                slot: VideoPlanSlot::NPlus1,
                identity: VideoPlanIdentity {
                    session_id: 7,
                    playback_generation: 7,
                    source_revision: 0,
                    parameter_revision: 1,
                    sequence: 1,
                },
                target_pts_ms: 1_000,
                period_ms: 1_000,
                seed: 1,
                prepared: true,
                commands: Vec::new(),
                parameter_support: ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
            },
            n2: None,
            session_id: 7,
            clock_epoch: 10,
            loop_index: 3,
            backend_epoch: 6,
            video_params: VideoEffectParams::default(),
            advanced_params: AdvancedEffectParams::default(),
        }
    }

    fn assert_runtime_status_contract(status: &MediaVideoBackendRuntimeStatus) {
        assert_eq!(status.parameter_support.backend, status.backend);
        assert!(status.demotion_history.len() <= 5);
        assert_eq!(
            status.last_demotion.as_ref(),
            status.demotion_history.last()
        );
        assert!(status
            .demotion_reason
            .as_deref()
            .is_none_or(|reason| !reason.trim().is_empty()
                && reason.encode_utf16().count() <= MAX_RUNTIME_LABEL_UTF16_UNITS));
        if status.process_id.is_some() {
            assert!(status.physical_paused.is_some());
            assert!(status.physical_eof_reached.is_some());
        } else {
            assert_eq!(status.presented_pts_ms, None);
            assert_eq!(status.physical_paused, None);
            assert_eq!(status.physical_eof_reached, None);
            assert_eq!(status.eof, None);
        }
        let has_audio_clock = status.audible_audio_pts_ms.is_some();
        assert_eq!(status.audio_epoch.is_some(), has_audio_clock);
        assert_eq!(status.av_sync_drift_ms.is_some(), has_audio_clock);
        if has_audio_clock {
            assert_eq!(status.activation, BackendActivation::Active);
            assert!(status.process_id.is_some());
        }
        if status
            .n
            .as_ref()
            .is_some_and(|slot| slot.status == CycleSlotState::Active)
        {
            assert_eq!(status.activation, BackendActivation::Active);
        }
        if matches!(
            status.backend,
            VideoBackend::RealtimeGpu | VideoBackend::Cpu4
        ) && status.activation == BackendActivation::Active
        {
            assert!(status
                .n
                .as_ref()
                .is_some_and(|slot| slot.status == CycleSlotState::Active));
        }
        match status.lifecycle {
            RendererLifecycleState::Stopped | RendererLifecycleState::Unavailable => {
                assert_ne!(status.activation, BackendActivation::Active);
                assert_eq!(status.process_id, None);
            }
            RendererLifecycleState::Probing => {
                assert_eq!(status.activation, BackendActivation::Configured);
                assert_eq!(status.process_id, None);
            }
            RendererLifecycleState::Spawned => {
                assert_eq!(status.activation, BackendActivation::Available);
                assert!(status.process_id.is_some());
            }
            RendererLifecycleState::Active => {
                assert_eq!(status.activation, BackendActivation::Active);
                assert!(status.process_id.is_some());
            }
            RendererLifecycleState::Failed => {
                assert_eq!(status.activation, BackendActivation::Failed);
                assert_eq!(status.process_id, None);
            }
        }
        serde_json::to_value(status).expect("runtime status serializes");
    }

    #[test]
    fn av_sync_status_stays_grouped_while_audio_clock_reanchors() {
        let mut runtime = RuntimeState::default();
        runtime.status.activation = BackendActivation::Active;
        runtime.status.lifecycle = RendererLifecycleState::Active;
        runtime.status.process_id = Some(41);
        runtime.status.physical_paused = Some(false);
        runtime.status.physical_eof_reached = Some(false);

        runtime.publish_av_sync_status(None, 72_350, 2);
        assert_eq!(runtime.status.av_sync_drift_ms, None);
        assert_eq!(runtime.status.audible_audio_pts_ms, None);
        assert_eq!(runtime.status.audio_epoch, None);
        assert_runtime_status_contract(&runtime.status);

        runtime.publish_av_sync_status(Some(-12), 72_420, 2);
        assert_eq!(runtime.status.av_sync_drift_ms, Some(-12));
        assert_eq!(runtime.status.audible_audio_pts_ms, Some(72_420));
        assert_eq!(runtime.status.audio_epoch, Some(2));
        assert_runtime_status_contract(&runtime.status);

        runtime.mark_cycle_status_available();
        assert_eq!(runtime.status.activation, BackendActivation::Available);
        assert_eq!(runtime.status.lifecycle, RendererLifecycleState::Spawned);
        assert_eq!(runtime.status.av_sync_drift_ms, None);
        assert_eq!(runtime.status.audible_audio_pts_ms, None);
        assert_eq!(runtime.status.audio_epoch, None);
        assert_runtime_status_contract(&runtime.status);
    }

    #[test]
    fn observation_deadline_allows_windows_ipc_scheduling_slack() {
        assert_eq!(OBSERVATION_COMMAND_DEADLINE, Duration::from_millis(250));
        assert_eq!(OBSERVATION_TICK_DEADLINE, Duration::from_millis(350));
        assert!(OBSERVATION_COMMAND_DEADLINE > Duration::from_millis(101));
        assert!(OBSERVATION_TICK_DEADLINE < MPV_COMMAND_TIMEOUT);
        let budget = ObservationTickBudget::start();
        assert!(budget.remaining() <= OBSERVATION_COMMAND_DEADLINE);
        assert!(budget.remaining() <= OBSERVATION_TICK_DEADLINE);
    }

    #[test]
    fn only_recoverable_observation_failures_are_transient() {
        assert!(transient_observation_failure(
            &RealtimeVideoBackendError::PropertyUnavailable {
                request_id: 7,
                operation: "get time-pos",
                property_error: "property unavailable".to_owned(),
            }
        ));
        assert!(transient_observation_failure(
            &RealtimeVideoBackendError::IpcQueueFull
        ));
        assert!(transient_observation_failure(
            &RealtimeVideoBackendError::IpcTimeout {
                request_id: 8,
                operation: "get time-pos",
            }
        ));
        assert!(!transient_observation_failure(
            &RealtimeVideoBackendError::IpcDisconnected("closed".to_owned())
        ));
        assert!(!transient_observation_failure(
            &RealtimeVideoBackendError::IpcProtocol("invalid".to_owned())
        ));
        assert!(!transient_observation_failure(
            &missing_video_observation_error()
        ));
    }

    #[test]
    fn source_transition_waits_for_new_fps_without_retaining_the_old_snapshot() {
        let mut runtime = RuntimeState {
            nominal_source_fps: Some(22.001_848),
            ..RuntimeState::default()
        };
        runtime.begin_source_transition();
        runtime.status.apply_state = VideoApplyState::SourceTransitioning;

        assert_eq!(runtime.nominal_source_fps, None);
        assert!(runtime.source_transition_started_at.is_some());
        let unavailable = RealtimeVideoBackendError::PropertyUnavailable {
            request_id: 48,
            operation: "get estimated-vf-fps",
            property_error: "property unavailable".to_owned(),
        };
        assert!(runtime.source_transition_fps_is_pending(&unavailable));

        runtime.record_video_observation(1_000, 26.0);
        assert_eq!(runtime.nominal_source_fps, Some(26.0));
        assert_eq!(runtime.status.actual_source_fps, Some(26.0));
        assert!(runtime.source_transition_started_at.is_none());
        assert!(!runtime.source_transition_fps_is_pending(&unavailable));
    }

    #[test]
    fn source_transition_fps_grace_is_bounded_and_property_specific() {
        let mut runtime = RuntimeState::default();
        runtime.status.apply_state = VideoApplyState::SourceTransitioning;
        runtime.source_transition_started_at =
            Instant::now().checked_sub(SOURCE_TRANSITION_FPS_GRACE + Duration::from_millis(1));
        let fps_unavailable = RealtimeVideoBackendError::PropertyUnavailable {
            request_id: 49,
            operation: "get estimated-vf-fps",
            property_error: "property unavailable".to_owned(),
        };
        assert!(!runtime.source_transition_fps_is_pending(&fps_unavailable));

        runtime.source_transition_started_at = Some(Instant::now());
        let time_unavailable = RealtimeVideoBackendError::PropertyUnavailable {
            request_id: 50,
            operation: "get time-pos",
            property_error: "property unavailable".to_owned(),
        };
        assert!(!runtime.source_transition_fps_is_pending(&time_unavailable));
    }

    #[test]
    fn source_fps_is_owned_by_the_async_observation_path_not_commit() {
        let source = include_str!("realtime_video_runtime.rs");
        let tick = source
            .split("    fn tick(&mut self)")
            .nth(1)
            .expect("runtime tick")
            .split("    fn poll_pending_shader_apply(")
            .next()
            .expect("runtime tick end");
        let effect_fps_observation = tick
            .split(
                "        if self.cycle_controller.is_some()\n            && self.process.is_some()",
            )
            .nth(1)
            .expect("effect source FPS observation")
            .split("        if self.status.activation == BackendActivation::Available")
            .next()
            .expect("effect source FPS observation end");
        assert!(effect_fps_observation
            .contains("BackendActivation::Available | BackendActivation::Active"));
        let fps_poll = effect_fps_observation
            .find("self.poll_source_fps_response()")
            .expect("source FPS async poll");
        let available_observation = tick
            .split("        if self.status.activation == BackendActivation::Available")
            .nth(1)
            .expect("available source observation")
            .split("        if self.status.backend == VideoBackend::Cpu4")
            .next()
            .expect("available source observation end");
        let playback_poll = available_observation
            .find("self.poll_playback_observation()")
            .expect("source EOF/PTS async observation");
        assert!(fps_poll < effect_fps_observation.len());
        assert!(playback_poll < available_observation.len());

        let commit = source
            .split("    fn commit(")
            .nth(1)
            .expect("runtime commit")
            .split("    fn finalize_pending_plan(")
            .next()
            .expect("runtime commit end");
        assert!(!commit.contains("read_video_observation"));
        assert!(!commit.contains("GetEstimatedVideoFps"));
    }

    #[test]
    fn playback_state_is_part_of_the_async_observation_transaction() {
        let source = include_str!("realtime_video_runtime.rs");
        let poll = source
            .split("    fn poll_playback_observation(")
            .nth(1)
            .expect("playback observation")
            .split("    fn source_fps_still_pending(")
            .next()
            .expect("playback observation end");
        for async_property in [
            "submit_eof_reached()",
            "submit_video_pts()",
            "submit_paused()",
            "submit_seeking()",
            "submit_paused_for_cache()",
        ] {
            assert!(poll.contains(async_property));
        }

        let av_sync = source
            .split("    fn observe_av_sync(")
            .nth(1)
            .expect("AV sync observation")
            .split("    fn tick(&mut self)")
            .next()
            .expect("AV sync observation end");
        assert!(!av_sync.contains("read_playback_state"));

        let eof = source
            .split("    fn record_eof_observation(")
            .nth(1)
            .expect("EOF observation")
            .split("    fn notify_eof_supervisor(")
            .next()
            .expect("EOF observation end");
        assert!(!eof.contains("read_paused"));
    }

    #[test]
    fn actor_defers_ipc_controls_until_the_current_transaction_is_drained() {
        let (reply, _response) = mpsc::sync_channel(1);
        assert!(RuntimeCommand::SetProcessingEnabled {
            enabled: true,
            reply: reply.clone(),
        }
        .requires_idle_ipc());
        assert!(!RuntimeCommand::Stop {
            reply: mpsc::sync_channel(1).0,
        }
        .requires_idle_ipc());
        assert!(!RuntimeCommand::RecordSourceBackend {
            reason: "diagnostic".to_owned(),
            reply,
        }
        .requires_idle_ipc());

        let source = include_str!("realtime_video_runtime.rs");
        let actor = source
            .split("fn actor_loop(")
            .nth(1)
            .expect("actor loop")
            .split("fn publish_status(")
            .next()
            .expect("actor loop end");
        assert!(actor.contains("deferred_commands.push_back(command)"));
        assert!(actor.contains("state.has_in_flight_ipc_transaction()"));
        assert!(actor.contains("state.tick_in_flight_ipc_transaction()"));
    }

    #[test]
    fn playback_observation_uses_one_total_deadline_and_eof_seqlock() {
        let source = include_str!("realtime_video_runtime.rs");
        let poll = source
            .split("    fn poll_playback_observation(")
            .nth(1)
            .expect("playback observation")
            .split("    fn source_fps_still_pending(")
            .next()
            .expect("playback observation end");
        assert!(poll.contains("PLAYBACK_OBSERVATION_HARD_TIMEOUT.saturating_sub"));
        assert!(poll.contains("PendingPlaybackObservationPhase::EofConfirm"));
        assert!(poll.contains("confirmed_eof != pending.eof_reached"));
        assert!(poll.contains("pending.cursor != self.sync_cursor"));
        assert!(poll.matches("submit_eof_reached()").count() >= 2);
    }

    #[test]
    fn scheduler_reset_discards_old_media_fps_and_observation_transactions() {
        let mut runtime = RuntimeState {
            nominal_source_fps: Some(22.001_848),
            ..RuntimeState::default()
        };
        runtime.status.actual_source_fps = Some(22.001_848);

        runtime.reset_scheduler(7, 10, 3, false);

        assert_eq!(runtime.nominal_source_fps, None);
        assert_eq!(runtime.status.actual_source_fps, None);
        assert!(runtime.source_transition_started_at.is_some());
        assert!(runtime.pending_source_fps_response.is_none());
        assert!(runtime.pending_playback_observation.is_none());
    }

    #[test]
    fn due_cycle_cannot_commit_before_current_process_fps_is_confirmed() {
        let mut runtime = RuntimeState::default();
        runtime.status.apply_state = VideoApplyState::Ready;

        assert!(runtime.commit_pending_if_due(1_000).is_none());
        assert_eq!(
            runtime.status.apply_state,
            VideoApplyState::SourceTransitioning
        );
    }

    #[test]
    fn sustained_transient_observation_failure_is_bounded_and_resets_nontransient_streak() {
        let mut runtime = RuntimeState::default();
        let unavailable = RealtimeVideoBackendError::PropertyUnavailable {
            request_id: 7,
            operation: "get time-pos",
            property_error: "property unavailable".to_owned(),
        };
        assert!(!runtime.observation_failure_reached_threshold(&unavailable));
        assert!(!runtime.observation_failure_reached_threshold(&unavailable));
        assert!(runtime.observation_failure_reached_threshold(&unavailable));
        assert_eq!(runtime.consecutive_transient_observation_failures, 0);

        let protocol = RealtimeVideoBackendError::IpcProtocol("invalid".to_owned());
        assert!(!runtime.observation_failure_reached_threshold(&protocol));
        assert!(!runtime.observation_failure_reached_threshold(&unavailable));
        assert_eq!(runtime.consecutive_tick_failures, 0);
        assert!(!runtime.observation_failure_reached_threshold(&protocol));
        runtime.record_observation_success();
        assert_eq!(runtime.consecutive_tick_failures, 0);
        assert_eq!(runtime.consecutive_transient_observation_failures, 0);
    }

    #[test]
    fn an_isolated_observation_timeout_does_not_rebuild_a_playing_renderer() {
        let mut runtime = RuntimeState {
            backend_epoch: 7,
            ..RuntimeState::default()
        };
        runtime.status.backend = VideoBackend::RealtimeGpu;
        runtime.status.backend_epoch = 7;

        assert!(
            !runtime.record_tick_failure(RealtimeVideoBackendError::IpcTimeout {
                request_id: 41,
                operation: "get time-pos",
            })
        );
        assert_eq!(runtime.backend_epoch, 7);
        assert_eq!(runtime.status.backend, VideoBackend::RealtimeGpu);
        assert_eq!(runtime.consecutive_transient_observation_failures, 1);
        assert!(runtime.status.demotion_history.is_empty());
    }

    #[test]
    fn startup_retries_only_property_unavailable() {
        assert!(startup_property_pending(
            &RealtimeVideoBackendError::PropertyUnavailable {
                request_id: 11,
                operation: "get time-pos",
                property_error: "property unavailable".to_owned(),
            }
        ));
        assert!(!startup_property_pending(
            &RealtimeVideoBackendError::IpcTimeout {
                request_id: 11,
                operation: "get time-pos",
            }
        ));
        assert!(!startup_property_pending(
            &RealtimeVideoBackendError::IpcDisconnected("pipe closed".to_owned())
        ));
    }

    #[test]
    fn startup_command_uses_remaining_total_budget() {
        let started = Instant::now();
        let remaining = startup_command_deadline(started);
        assert!(remaining > MPV_COMMAND_TIMEOUT);
        assert!(remaining <= MPV_PIPE_CONNECT_TIMEOUT);
    }

    #[test]
    fn nested_startup_failure_keeps_only_the_inner_reason() {
        let message = backend_error_message(RealtimeVideoBackendError::ProcessFailed {
            operation: "mpv 初始化",
            message: "首帧尚未呈现".to_owned(),
        });
        assert_eq!(message, "首帧尚未呈现");
    }

    #[test]
    fn malformed_frame_budget_telemetry_never_triggers_renderer_fallback() {
        let mut runtime = RuntimeState {
            last_frame_budget_pts_ms: Some(42_000),
            last_vo_passes_snapshot: Some(VoPassesTimingSnapshot { passes: Vec::new() }),
            last_frame_health_counts: Some((1, 2, Some(3), Some(4))),
            ..RuntimeState::default()
        };
        runtime.status.gpu_pass_p99_ms = Some(12.0);
        let failure = || RealtimeVideoBackendError::IpcProtocol("invalid vo-passes".to_owned());
        assert!(!runtime.record_frame_budget_observation_failure(failure()));
        runtime.record_observation_success();
        assert_eq!(runtime.consecutive_frame_budget_observation_failures, 1);
        assert!(runtime.last_frame_budget_pts_ms.is_none());
        assert!(runtime.last_vo_passes_snapshot.is_none());
        assert!(runtime.last_frame_health_counts.is_none());
        assert!(runtime.status.gpu_pass_p99_ms.is_none());
        assert!(!runtime.record_frame_budget_observation_failure(failure()));
        assert!(!runtime.record_frame_budget_observation_failure(failure()));
        assert_eq!(
            runtime.consecutive_frame_budget_observation_failures,
            FRAME_BUDGET_OBSERVATION_FAILURE_THRESHOLD
        );
        assert_eq!(runtime.status.backend, VideoBackend::Source);
    }

    #[test]
    fn frame_budget_timeout_is_telemetry_loss_not_a_renderer_failure() {
        let mut runtime = RuntimeState {
            backend_epoch: 9,
            ..RuntimeState::default()
        };
        runtime.status.backend = VideoBackend::RealtimeGpu;
        runtime.status.backend_epoch = 9;

        assert!(!runtime.record_frame_budget_observation_failure(
            RealtimeVideoBackendError::IpcTimeout {
                request_id: 42,
                operation: "get vo-passes",
            }
        ));
        assert_eq!(runtime.backend_epoch, 9);
        assert_eq!(runtime.status.backend, VideoBackend::RealtimeGpu);
        assert_eq!(runtime.consecutive_frame_budget_observation_failures, 1);
        assert!(runtime.status.demotion_history.is_empty());
    }

    #[test]
    fn frame_budget_sampling_requires_stable_active_playback() {
        let mut runtime = RuntimeState::default();
        runtime.status.apply_state = VideoApplyState::Active;
        let playing = MpvPlaybackState {
            paused: false,
            seeking: false,
            paused_for_cache: false,
        };
        assert!(runtime.frame_budget_observation_eligible(false, playing));
        assert!(!runtime.frame_budget_observation_eligible(true, playing));
        for playback_state in [
            MpvPlaybackState {
                paused: true,
                ..playing
            },
            MpvPlaybackState {
                seeking: true,
                ..playing
            },
            MpvPlaybackState {
                paused_for_cache: true,
                ..playing
            },
        ] {
            assert!(!runtime.frame_budget_observation_eligible(false, playback_state));
        }
        runtime.pending_boundary = VideoScheduleBoundary::SourceChanged;
        assert!(!runtime.frame_budget_observation_eligible(false, playing));
        runtime.pending_boundary = VideoScheduleBoundary::None;
        runtime.status.apply_state = VideoApplyState::Applying;
        assert!(!runtime.frame_budget_observation_eligible(false, playing));
    }

    #[test]
    fn source_status_never_claims_effects_are_active() {
        let status = MediaVideoBackendRuntimeStatus::default();
        assert_eq!(status.backend, VideoBackend::Source);
        assert_eq!(status.activation, BackendActivation::Available);
        assert_eq!(status.lifecycle, RendererLifecycleState::Stopped);
        assert_eq!(status.process_id, None);
        assert_eq!(
            status.support_completeness,
            SupportCompleteness::NotApplicable
        );
        assert_runtime_status_contract(&status);
    }

    #[test]
    fn every_published_status_reconciles_backend_and_parameter_support_atomically() {
        for (backend, launch_mode, stale_support) in [
            (
                VideoBackend::Source,
                Some(MpvLaunchMode::Original),
                gpu83_cycle_parameter_support(),
            ),
            (
                VideoBackend::RealtimeGpu,
                Some(MpvLaunchMode::Gpu(MpvGpuProfile::D3d11ZeroCopy)),
                ParameterSupportReport::empty(VideoBackend::Source),
            ),
            (
                VideoBackend::Cpu4,
                Some(MpvLaunchMode::Cpu4),
                ParameterSupportReport::empty(VideoBackend::Source),
            ),
        ] {
            let snapshot = RwLock::new(MediaVideoBackendRuntimeStatus::default());
            let mut runtime = RuntimeState {
                process_launch_mode: launch_mode,
                ..RuntimeState::default()
            };
            runtime.status.backend = backend;
            runtime.status.parameter_support = stale_support;

            publish_status(&snapshot, &mut runtime);

            let published = snapshot.read().expect("published status");
            assert_eq!(published.parameter_support.backend, backend);
            assert_eq!(
                published.support_completeness,
                match backend {
                    VideoBackend::Source => SupportCompleteness::NotApplicable,
                    VideoBackend::RealtimeGpu => SupportCompleteness::Complete,
                    VideoBackend::Cpu4 => SupportCompleteness::Incomplete,
                }
            );
        }
    }

    #[test]
    fn published_status_exposes_only_the_confirmed_active_cycle_parameters() {
        let video_params = VideoEffectParams {
            brightness_percent: 17.5,
            ..VideoEffectParams::default()
        };
        let advanced_params = AdvancedEffectParams {
            wave_intensity: 0.75,
            ..AdvancedEffectParams::default()
        };
        let identity = VideoPlanIdentity {
            session_id: 7,
            playback_generation: 9,
            source_revision: 3,
            parameter_revision: 4,
            sequence: 12,
        };
        let mut runtime = RuntimeState {
            current_schedule: Some(PreparedScheduleContext {
                identity,
                seed: 99,
                video_params: video_params.clone(),
                advanced_params: advanced_params.clone(),
                clock_epoch: 5,
                loop_index: 2,
                paused: false,
            }),
            ..RuntimeState::default()
        };
        runtime.status.backend = VideoBackend::RealtimeGpu;
        runtime.status.activation = BackendActivation::Active;
        runtime.status.lifecycle = RendererLifecycleState::Active;
        runtime.status.apply_state = VideoApplyState::Active;
        runtime.status.n = Some(CycleSlotStatus {
            sequence: 12,
            target_pts_ms: 8_000,
            status: CycleSlotState::Active,
        });
        runtime.status.active_plan_fingerprint = Some("confirmed-plan".to_owned());
        let snapshot = RwLock::new(MediaVideoBackendRuntimeStatus::default());

        publish_status(&snapshot, &mut runtime);

        let published = snapshot.read().expect("published active status");
        let active = published
            .active_cycle_snapshot
            .as_ref()
            .expect("confirmed active parameter snapshot");
        assert_eq!(active.sequence, 12);
        assert_eq!(active.fingerprint, "confirmed-plan");
        assert_eq!(active.video, video_params);
        assert_eq!(active.advanced, advanced_params);
        drop(published);

        runtime.status.apply_state = VideoApplyState::ResultUnknown;
        publish_status(&snapshot, &mut runtime);
        assert!(snapshot
            .read()
            .expect("published uncertain status")
            .active_cycle_snapshot
            .is_none());
    }

    #[test]
    fn gpu_cpu4_and_original_active_statuses_match_the_runtime_contract() {
        let mut gpu = RuntimeState::default();
        gpu.record_renderer_started(
            MpvGraphicsApi::D3d11,
            "d3d11va".to_owned(),
            51,
            0,
            false,
            false,
        );
        gpu.record_parameter_support(ParameterSupportReport::empty(VideoBackend::RealtimeGpu));
        assert_runtime_status_contract(&gpu.status);
        gpu.status.activation = BackendActivation::Active;
        gpu.status.lifecycle = RendererLifecycleState::Active;
        gpu.status.n = Some(CycleSlotStatus {
            sequence: 1,
            target_pts_ms: 1_000,
            status: CycleSlotState::Active,
        });
        assert_runtime_status_contract(&gpu.status);

        let mut cpu4 = RuntimeState::default();
        demote_all_gpu_profiles(&mut cpu4);
        cpu4.claim_cpu4_status();
        cpu4.status.activation = BackendActivation::Active;
        cpu4.status.lifecycle = RendererLifecycleState::Active;
        cpu4.record_confirmed_process_facts(52, 0, false, false);
        cpu4.status.n = Some(CycleSlotStatus {
            sequence: 1,
            target_pts_ms: 1_000,
            status: CycleSlotState::Active,
        });
        assert_runtime_status_contract(&cpu4.status);

        let mut original = RuntimeState::default();
        original.record_original_started(
            MpvGraphicsApi::D3d11,
            "unreported-original".to_owned(),
            53,
            0,
            false,
            false,
        );
        assert_runtime_status_contract(&original.status);
    }

    #[test]
    fn stopped_probing_and_failed_statuses_publish_no_physical_process_facts() {
        let mut runtime = RuntimeState::default();
        runtime.record_confirmed_process_facts(61, 1_000, false, true);
        runtime.stop_process();
        assert_runtime_status_contract(&runtime.status);

        runtime.record_confirmed_process_facts(62, 2_000, false, true);
        runtime.demote(
            "GPU 初始化失败",
            1,
            ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
        );
        assert_runtime_status_contract(&runtime.status);

        runtime.record_confirmed_process_facts(63, 3_000, false, true);
        runtime.record_original_failure("Original 初始化失败".to_owned());
        assert_runtime_status_contract(&runtime.status);
    }

    #[test]
    fn unsupported_parameters_are_observable_without_demoting_realtime_gpu() {
        let mut runtime = RuntimeState::default();
        runtime.status.backend = VideoBackend::RealtimeGpu;
        runtime.record_parameter_support(ParameterSupportReport {
            backend: VideoBackend::RealtimeGpu,
            fully_supported: false,
            parameters: vec![crate::realtime_video_backend::ParameterSupportResult {
                field: "video.blur_radius_px".to_owned(),
                active: true,
                supported: false,
                mapping: None,
                reason: Some("未映射".to_owned()),
            }],
        });
        let result = runtime.status.clone();
        assert_eq!(result.backend, VideoBackend::RealtimeGpu);
        assert_eq!(result.support_completeness, SupportCompleteness::Incomplete);
        assert_eq!(result.ignored_active_parameter_count, 1);
        assert_eq!(
            result.ignored_active_parameter_examples,
            ["video.blur_radius_px"]
        );
        assert!(result.last_demotion.is_none());
    }

    #[test]
    fn cpu4_plan_reports_exact_four_supported_and_seventy_nine_discarded_fields() {
        let identity = VideoPlanIdentity {
            session_id: 1,
            playback_generation: 1,
            source_revision: 1,
            parameter_revision: 1,
            sequence: 1,
        };
        let plan = RealtimeVideoPlan {
            slot: VideoPlanSlot::NPlus1,
            identity,
            target_pts_ms: 1_000,
            period_ms: 1_000,
            seed: 1,
            prepared: true,
            commands: Vec::new(),
            parameter_support: ParameterSupportReport {
                backend: VideoBackend::RealtimeGpu,
                fully_supported: true,
                parameters: GPU83_PARAMETER_MAPPINGS
                    .iter()
                    .map(|mapping| ParameterSupportResult {
                        field: mapping.field_path.to_owned(),
                        active: true,
                        supported: true,
                        mapping: Some("gpu".to_owned()),
                        reason: None,
                    })
                    .collect(),
            },
        };
        let cpu4 = cpu4_plan_from(&plan, &VideoEffectParams::default()).expect("CPU4 plan");
        assert_eq!(cpu4.commands.len(), 4);
        assert_eq!(cpu4.parameter_support.parameters.len(), 83);
        assert_eq!(
            cpu4.parameter_support
                .parameters
                .iter()
                .filter(|parameter| parameter.supported)
                .count(),
            4
        );
        assert_eq!(
            cpu4.parameter_support
                .parameters
                .iter()
                .filter(|parameter| !parameter.supported)
                .count(),
            79
        );
        assert!(!cpu4.parameter_support.fully_supported);

        let fingerprint = canonical_cpu4_plan_fingerprint(&cpu4, 3, 5, 29.97)
            .expect("canonical CPU4 fingerprint");
        assert_eq!(fingerprint.len(), 64);
        assert_eq!(
            fingerprint,
            canonical_cpu4_plan_fingerprint(&cpu4, 3, 5, 29.97)
                .expect("deterministic CPU4 fingerprint")
        );
        assert_ne!(
            fingerprint,
            canonical_cpu4_plan_fingerprint(&cpu4, 4, 5, 29.97).expect("clock identity")
        );
        assert_ne!(
            fingerprint,
            canonical_cpu4_plan_fingerprint(&cpu4, 3, 6, 29.97).expect("loop identity")
        );
        assert_ne!(
            fingerprint,
            canonical_cpu4_plan_fingerprint(&cpu4, 3, 5, 30.0).expect("actual fps")
        );
        let changed_video = VideoEffectParams {
            brightness_percent: 1.0,
            ..VideoEffectParams::default()
        };
        let changed_cpu4 = cpu4_plan_from(&plan, &changed_video).expect("changed CPU4 plan");
        assert_ne!(
            fingerprint,
            canonical_cpu4_plan_fingerprint(&changed_cpu4, 3, 5, 29.97)
                .expect("full command identity")
        );

        let mut runtime = RuntimeState {
            process_launch_mode: Some(MpvLaunchMode::Cpu4),
            pending_schedule: Some(PreparedScheduleContext {
                identity: cpu4.identity.clone(),
                seed: cpu4.seed,
                video_params: VideoEffectParams::default(),
                advanced_params: AdvancedEffectParams::default(),
                clock_epoch: 3,
                loop_index: 5,
                paused: false,
            }),
            status: MediaVideoBackendRuntimeStatus {
                pending_plan_fingerprint: Some(fingerprint.clone()),
                ..MediaVideoBackendRuntimeStatus::default()
            },
            ..RuntimeState::default()
        };
        runtime
            .finalize_pending_plan(cpu4, 0)
            .expect("CPU4 activation");
        assert_eq!(runtime.status.active_plan_fingerprint, Some(fingerprint));
        assert_eq!(runtime.status.apply_state, VideoApplyState::Active);
    }

    #[test]
    fn backend_floor_resets_only_for_a_new_playback_generation() {
        let mut runtime = RuntimeState {
            sync_cursor: Some(sync_cursor()),
            ..RuntimeState::default()
        };
        demote_all_gpu_profiles(&mut runtime);
        assert_eq!(runtime.backend.launch_mode(), MpvLaunchMode::Cpu4);

        runtime.begin_generation(7).expect("same generation");
        assert_eq!(runtime.backend.current(), VideoBackend::Cpu4);
        assert!(runtime.begin_generation(6).is_err());
        assert_eq!(runtime.backend.current(), VideoBackend::Cpu4);

        runtime.begin_generation(8).expect("new generation");
        assert_eq!(runtime.backend.current(), VideoBackend::RealtimeGpu);
        assert_eq!(runtime.backend_epoch, 0);
    }

    #[test]
    fn generation_process_reuse_requires_exact_successor_live_process_and_matching_eof() {
        let exact = GenerationReuseFacts {
            current_generation: 7,
            requested_generation: 8,
            process_present: true,
            eof_matches_current_identity: true,
        };
        assert!(can_preserve_process_for_next_generation(exact));
        assert!(!can_preserve_process_for_next_generation(
            GenerationReuseFacts {
                requested_generation: 9,
                ..exact
            }
        ));
        assert!(!can_preserve_process_for_next_generation(
            GenerationReuseFacts {
                process_present: false,
                ..exact
            }
        ));
        assert!(!can_preserve_process_for_next_generation(
            GenerationReuseFacts {
                eof_matches_current_identity: false,
                ..exact
            }
        ));
    }

    #[test]
    fn preserved_process_transition_keeps_the_active_fallback_floor() {
        let mut runtime = RuntimeState::default();
        demote_all_gpu_profiles(&mut runtime);
        assert_eq!(runtime.backend.launch_mode(), MpvLaunchMode::Cpu4);

        runtime.reset_generation_state_preserving_process();

        assert_eq!(runtime.backend.launch_mode(), MpvLaunchMode::Cpu4);
        assert_eq!(runtime.backend_epoch, 0);
        assert!(runtime.session.is_none());
        assert!(runtime.sync_cursor.is_none());
    }

    #[test]
    fn newer_requested_generation_cancels_an_unpublished_launch() {
        let runtime = RealtimeVideoRuntime::default();
        let first = runtime.reserve_operation(7, 10, 3).expect("first lease");
        assert_eq!(
            runtime.reserve_operation(7, 10, 3).expect("same lease"),
            first
        );

        let newer = runtime.reserve_operation(8, 1, 0).expect("new generation");
        assert!(newer.operation_revision > first.operation_revision);
        assert!(runtime.validate_operation_lease(7, first).is_err());
    }

    #[test]
    fn stopped_generation_is_a_tombstone_for_late_prepare_requests() {
        let runtime = RealtimeVideoRuntime::default();
        let lease = runtime
            .reserve_operation(7, 10, 3)
            .expect("reserve operation");
        runtime.stop(7).expect("stop runtime");

        assert!(runtime.validate_operation_lease(7, lease).is_err());
        assert!(runtime.reserve_operation(7, 10, 3).is_err());
        assert!(runtime.reserve_operation(6, 10, 3).is_err());
        assert!(runtime.reserve_operation(8, 1, 0).is_ok());
    }

    #[test]
    fn surface_suspend_allows_the_same_generation_to_bind_again() {
        let runtime = RealtimeVideoRuntime::default();
        let first = runtime
            .reserve_operation(7, 10, 3)
            .expect("reserve surface operation");
        runtime.suspend().expect("suspend surface");

        let rebound = runtime
            .reserve_operation(7, 10, 3)
            .expect("same generation may bind a new surface");
        assert!(rebound.operation_revision > first.operation_revision);
        assert!(runtime.validate_operation_lease(7, first).is_err());
        assert!(runtime.validate_operation_lease(7, rebound).is_ok());
    }

    #[test]
    fn newer_seek_cursor_invalidates_an_unpublished_launch_lease() {
        let runtime = RealtimeVideoRuntime::default();
        let old = runtime.reserve_operation(7, 10, 3).expect("old lease");
        let current = runtime.reserve_operation(7, 11, 3).expect("new lease");

        assert!(runtime.validate_operation_lease(7, old).is_err());
        assert!(runtime.validate_operation_lease(7, current).is_ok());
    }

    #[test]
    fn natural_loop_cursor_invalidates_a_launch_without_advancing_the_seek_epoch() {
        let runtime = RealtimeVideoRuntime::default();
        let old = runtime.reserve_operation(7, 10, 3).expect("old lease");
        let current = runtime
            .reserve_operation(7, 10, 4)
            .expect("natural loop cursor");

        assert!(runtime.validate_operation_lease(7, old).is_err());
        assert!(runtime.validate_operation_lease(7, current).is_ok());
    }

    #[test]
    fn old_or_mixed_cursor_cannot_reserve_after_a_newer_cursor() {
        let runtime = RealtimeVideoRuntime::default();
        runtime.reserve_operation(7, 11, 4).expect("new cursor");

        assert!(runtime.reserve_operation(7, 10, 4).is_err());
        assert!(runtime.reserve_operation(7, 10, 5).is_err());
        assert!(runtime.reserve_operation(7, 12, 3).is_err());
    }

    #[test]
    fn concurrent_stop_either_rejects_reserve_or_invalidates_its_lease() {
        for _ in 0..16 {
            let runtime = Arc::new(RealtimeVideoRuntime::default());
            let barrier = Arc::new(Barrier::new(2));
            let reserve_runtime = Arc::clone(&runtime);
            let reserve_barrier = Arc::clone(&barrier);
            let reserve = thread::spawn(move || {
                reserve_barrier.wait();
                reserve_runtime.reserve_operation(7, 10, 3)
            });
            barrier.wait();
            runtime.stop(7).expect("stop runtime");
            if let Ok(lease) = reserve.join().expect("reserve thread") {
                assert!(runtime.validate_operation_lease(7, lease).is_err());
            }
        }
    }

    #[test]
    fn concurrent_stop_and_seek_cannot_resurrect_the_stopped_generation() {
        for _ in 0..16 {
            let runtime = Arc::new(RealtimeVideoRuntime::default());
            runtime.reserve_operation(7, 10, 3).expect("initial cursor");
            let barrier = Arc::new(Barrier::new(2));
            let seek_runtime = Arc::clone(&runtime);
            let seek_barrier = Arc::clone(&barrier);
            let seek = thread::spawn(move || {
                seek_barrier.wait();
                seek_runtime.register_authoritative_cursor(7, 11, 3)
            });

            barrier.wait();
            runtime.stop(7).expect("stop runtime");
            let _seek_result = seek.join().expect("seek thread");

            assert!(runtime.reserve_operation(7, 11, 3).is_err());
            assert_eq!(runtime.status().lifecycle, RendererLifecycleState::Stopped);
        }
    }

    #[test]
    fn commit_without_a_pending_plan_is_rejected() {
        let mut runtime = RuntimeState {
            sync_cursor: Some(sync_cursor()),
            backend_epoch: 3,
            ..RuntimeState::default()
        };
        let result = runtime.commit(
            &RealtimeVideoCommit {
                gate: VideoCommitGate {
                    identity: VideoPlanIdentity {
                        session_id: 1,
                        playback_generation: 7,
                        source_revision: 1,
                        parameter_revision: 1,
                        sequence: 1,
                    },
                    media_pts_ms: 1_000,
                },
                clock_epoch: 10,
                loop_index: 3,
                backend_epoch: 3,
            },
            1,
        );
        assert!(result.is_err());
    }

    #[test]
    fn prepared_plan_auto_commit_uses_the_observed_absolute_pts() {
        let identity = VideoPlanIdentity {
            session_id: 1,
            playback_generation: 7,
            source_revision: 1,
            parameter_revision: 2,
            sequence: 2,
        };
        let pending = RealtimeVideoPlan {
            slot: VideoPlanSlot::NPlus1,
            identity: identity.clone(),
            target_pts_ms: 31_500,
            period_ms: 5_000,
            seed: 9,
            prepared: true,
            commands: Vec::new(),
            parameter_support: ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
        };
        let cursor = SyncCursor {
            playback_generation: 7,
            clock_epoch: 10,
            loop_index: 3,
            paused: false,
        };

        assert!(
            build_due_realtime_video_commit(Some(&pending), Some(cursor), 4, 10_000, 1_499,)
                .is_none()
        );
        let commit =
            build_due_realtime_video_commit(Some(&pending), Some(cursor), 4, 10_000, 1_500)
                .expect("target PTS must create an automatic commit");
        assert_eq!(commit.gate.identity, identity);
        assert_eq!(commit.gate.media_pts_ms, 31_500);
        assert_eq!(commit.clock_epoch, 10);
        assert_eq!(commit.loop_index, 3);
        assert_eq!(commit.backend_epoch, 4);
    }

    #[test]
    fn shader_scheduler_uses_source_local_pts_across_loops() {
        assert_eq!(source_local_schedule_pts(31_500, 10_000, 3), Some(1_500));
        assert_eq!(source_local_schedule_pts(1_500, 10_000, 0), Some(1_500));
        assert_eq!(source_local_schedule_pts(1_500, 0, 0), None);
        assert_eq!(source_local_schedule_pts(31_500, 10_000, 2), None);
    }

    #[test]
    fn repeated_commit_of_the_active_plan_is_idempotent() {
        let identity = VideoPlanIdentity {
            session_id: 1,
            playback_generation: 7,
            source_revision: 1,
            parameter_revision: 2,
            sequence: 2,
        };
        let current = RealtimeVideoPlan {
            slot: VideoPlanSlot::N,
            identity: identity.clone(),
            target_pts_ms: 1_500,
            period_ms: 5_000,
            seed: 9,
            prepared: true,
            commands: Vec::new(),
            parameter_support: ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
        };
        let mut runtime = RuntimeState {
            current: Some(current),
            sync_cursor: Some(sync_cursor()),
            backend_epoch: 3,
            status: MediaVideoBackendRuntimeStatus {
                activation: BackendActivation::Active,
                lifecycle: RendererLifecycleState::Active,
                ..MediaVideoBackendRuntimeStatus::default()
            },
            ..RuntimeState::default()
        };

        let status = runtime
            .commit(
                &RealtimeVideoCommit {
                    gate: VideoCommitGate {
                        identity,
                        media_pts_ms: 1_600,
                    },
                    clock_epoch: 10,
                    loop_index: 3,
                    backend_epoch: 3,
                },
                1,
            )
            .expect("late duplicate commit must acknowledge the active plan");
        assert_eq!(status.activation, BackendActivation::Active);
    }

    #[test]
    fn video_output_requires_rendered_vo_passes_not_only_a_timeline() {
        let empty = serde_json::json!({
            "error": "success",
            "data": { "fresh": [] }
        });
        let rendered = serde_json::json!({
            "error": "success",
            "data": { "fresh": [{ "count": 1, "samples": [1000] }] }
        });
        assert!(!vo_passes_has_rendered_frame(&empty));
        assert!(vo_passes_has_rendered_frame(&rendered));
    }

    #[test]
    fn gpu_pass_budget_uses_the_sum_of_per_pass_p99_samples() {
        let response = serde_json::json!({
            "error": "success",
            "data": {
                "fresh": [
                    { "samples": [1_000_000, 2_000_000, 3_000_000] },
                    { "samples": [500_000, 1_000_000, 1_500_000] }
                ]
            }
        });
        assert_eq!(vo_passes_p99_ms(&response).expect("P99"), 4.5);
    }

    #[test]
    fn gpu_pass_budget_ignores_legal_empty_startup_passes() {
        let response = serde_json::json!({
            "error": "success",
            "data": {
                "fresh": [
                    { "count": 0, "samples": [] },
                    { "count": 2, "samples": [4_000, 6_000] }
                ]
            }
        });
        assert_eq!(vo_passes_p99_ms(&response).expect("P99"), 0.006);

        let pending = serde_json::json!({
            "error": "success",
            "data": { "fresh": [{ "count": 0, "samples": [] }] }
        });
        assert!(matches!(
            vo_passes_p99_ms(&pending),
            Err(RealtimeVideoBackendError::PropertyUnavailable { .. })
        ));
    }

    #[test]
    fn unavailable_vo_pass_samples_do_not_become_protocol_failures() {
        for fresh in [
            serde_json::json!([]),
            serde_json::json!([{ "desc": "missing" }]),
            serde_json::json!([{ "desc": "null", "samples": [null] }]),
            serde_json::json!([{ "desc": "negative", "samples": [-1] }]),
        ] {
            let response = serde_json::json!({
                "error": "success",
                "data": { "fresh": fresh }
            });
            assert!(matches!(
                vo_passes_timing_snapshot(&response),
                Err(RealtimeVideoBackendError::PropertyUnavailable { .. })
            ));
        }
    }

    #[test]
    fn rolling_gpu_spike_is_not_recounted_as_a_new_health_window() {
        let baseline = vo_passes_timing_snapshot(&serde_json::json!({
            "error": "success",
            "data": { "fresh": [{
                "desc": "gpu83",
                "count": 3,
                "samples": [1_000_000, 100_000_000, 1_000_000]
            }] }
        }))
        .expect("baseline");
        let next = vo_passes_timing_snapshot(&serde_json::json!({
            "error": "success",
            "data": { "fresh": [{
                "desc": "gpu83",
                "count": 4,
                "samples": [1_000_000, 100_000_000, 1_000_000, 2_000_000]
            }] }
        }))
        .expect("next");

        assert_eq!(incremental_vo_passes_p99_ms(&baseline, &next), Some(2.0));
    }

    #[test]
    fn saturated_vo_pass_window_uses_only_the_new_tail_samples() {
        let previous_samples = (0..MPV_VO_PERF_SAMPLE_COUNT)
            .map(|index| index as u64 * 1_000)
            .collect::<Vec<_>>();
        let mut current_samples = previous_samples[3..].to_vec();
        current_samples.extend([2_000_000_u64, 3_000_000, 4_000_000]);
        let previous = vo_passes_timing_snapshot(&serde_json::json!({
            "error": "success",
            "data": { "fresh": [{
                "desc": "gpu83",
                "count": MPV_VO_PERF_SAMPLE_COUNT,
                "samples": previous_samples
            }] }
        }))
        .expect("previous");
        let current = vo_passes_timing_snapshot(&serde_json::json!({
            "error": "success",
            "data": { "fresh": [{
                "desc": "gpu83",
                "count": MPV_VO_PERF_SAMPLE_COUNT,
                "samples": current_samples
            }] }
        }))
        .expect("current");

        assert_eq!(incremental_vo_passes_p99_ms(&previous, &current), Some(4.0));
    }

    #[test]
    fn identical_saturated_overload_uses_pts_progress_as_new_frame_evidence() {
        let snapshot = vo_passes_timing_snapshot(&serde_json::json!({
            "error": "success",
            "data": { "fresh": [{
                "desc": "gpu83",
                "count": MPV_VO_PERF_SAMPLE_COUNT,
                "samples": vec![50_000_000_u64; MPV_VO_PERF_SAMPLE_COUNT]
            }] }
        }))
        .expect("snapshot");

        assert_eq!(new_vo_passes_p99_ms(&snapshot, &snapshot, true), Some(50.0));
        assert_eq!(new_vo_passes_p99_ms(&snapshot, &snapshot, false), None);
    }

    #[test]
    fn pts_progress_counts_an_unchanged_pass_alongside_a_changed_pass() {
        let previous = vo_passes_timing_snapshot(&serde_json::json!({
            "error": "success",
            "data": { "fresh": [
                { "desc": "constant", "samples": [8_000_000] },
                { "desc": "changing", "samples": [1_000_000] }
            ] }
        }))
        .expect("previous");
        let current = vo_passes_timing_snapshot(&serde_json::json!({
            "error": "success",
            "data": { "fresh": [
                { "desc": "constant", "samples": [8_000_000] },
                { "desc": "changing", "samples": [1_000_000, 2_000_000] }
            ] }
        }))
        .expect("current");

        assert_eq!(new_vo_passes_p99_ms(&previous, &current, true), Some(10.0));
        assert_eq!(incremental_vo_passes_p99_ms(&previous, &current), Some(2.0));
    }

    #[test]
    fn sustained_new_gpu_timing_pressure_remains_telemetry_only() {
        let snapshots = [1, 2, 3, 4].map(|count| {
            vo_passes_timing_snapshot(&serde_json::json!({
                "error": "success",
                "data": { "fresh": [{
                    "desc": "gpu83",
                    "count": count,
                    "samples": vec![50_000_000_u64; count]
                }] }
            }))
            .expect("snapshot")
        });
        let mut windows = 0;
        for pair in snapshots.windows(2) {
            let p99 = incremental_vo_passes_p99_ms(&pair[0], &pair[1])
                .expect("each snapshot contains a new sample");
            assert!(p99 > 12.0);
            windows = advance_hard_frame_health_violation_count(windows, Some(false));
        }
        assert_eq!(windows, 0);
    }

    #[test]
    fn frame_counter_regression_rebuilds_the_baseline_without_a_violation() {
        assert_eq!(
            frame_health_counter_increase(
                Some((10, 5, Some(8), Some(3))),
                (1, 0, Some(0), Some(0)),
            ),
            None
        );
        assert_eq!(
            frame_health_counter_increase(Some((1, 0, Some(0), Some(0))), (2, 0, Some(0), Some(0)),),
            Some(true)
        );
    }

    #[test]
    fn frame_budget_transition_keeps_telemetry_but_discards_old_evidence() {
        let last_vo_passes_snapshot = vo_passes_timing_snapshot(&serde_json::json!({
            "error": "success",
            "data": { "fresh": [{
                "desc": "gpu83",
                "count": 1,
                "samples": [100_000_000]
            }] }
        }))
        .expect("snapshot");
        let status = MediaVideoBackendRuntimeStatus {
            frame_budget_violation_windows: 2,
            gpu_pass_p99_ms: Some(100.0),
            ..MediaVideoBackendRuntimeStatus::default()
        };
        let mut runtime = RuntimeState {
            last_vo_passes_snapshot: Some(last_vo_passes_snapshot),
            last_frame_health_counts: Some((4, 2, Some(1), Some(1))),
            last_frame_budget_pts_ms: Some(8_000),
            consecutive_frame_budget_violations: 2,
            status,
            ..RuntimeState::default()
        };

        runtime.begin_frame_budget_transition();

        assert!(runtime.last_vo_passes_snapshot.is_none());
        assert!(runtime.last_frame_health_counts.is_none());
        assert!(runtime.last_frame_budget_pts_ms.is_none());
        assert_eq!(runtime.consecutive_frame_budget_violations, 0);
        assert_eq!(runtime.status.frame_budget_violation_windows, 0);
        assert_eq!(runtime.status.gpu_pass_p99_ms, Some(100.0));
    }

    #[test]
    fn health_demotion_requires_three_consecutive_violations() {
        let mut windows = 0;
        windows = advance_health_violation_count(windows, true);
        assert_eq!(windows, 1);
        windows = advance_health_violation_count(windows, true);
        assert_eq!(windows, 2);
        windows = advance_health_violation_count(windows, false);
        assert_eq!(windows, 0);
        for expected in 1..=FRAME_BUDGET_VIOLATION_THRESHOLD {
            windows = advance_health_violation_count(windows, true);
            assert_eq!(windows, expected);
        }
    }

    #[test]
    fn timing_only_pressure_does_not_advance_hard_frame_health_demotion() {
        let timing_over_budget = true;
        let mut windows = 0;
        for _ in 0..FRAME_BUDGET_VIOLATION_THRESHOLD {
            assert!(timing_over_budget);
            windows = advance_hard_frame_health_violation_count(windows, Some(false));
        }
        assert_eq!(windows, 0);
    }

    #[test]
    fn frame_health_counter_growth_for_three_windows_reaches_demotion_threshold() {
        let mut windows = 0;
        for expected in 1..=FRAME_BUDGET_VIOLATION_THRESHOLD {
            windows = advance_hard_frame_health_violation_count(windows, Some(true));
            assert_eq!(windows, expected);
        }
    }

    #[test]
    fn healthy_frame_health_window_clears_hard_demotion_count() {
        let mut windows = advance_hard_frame_health_violation_count(0, Some(true));
        windows = advance_hard_frame_health_violation_count(windows, Some(true));
        assert_eq!(windows, 2);

        windows = advance_hard_frame_health_violation_count(windows, Some(false));
        assert_eq!(windows, 0);
    }

    #[test]
    fn optional_frame_timing_property_unavailable_is_not_an_observation_failure() {
        for (operation, property) in [
            ("get mistimed-frame-count", "mistimed-frame-count"),
            ("get vo-delayed-frame-count", "vo-delayed-frame-count"),
        ] {
            let unavailable = RealtimeVideoBackendError::PropertyUnavailable {
                request_id: 73,
                operation,
                property_error: "property unavailable".to_owned(),
            };
            assert_eq!(
                optional_frame_timing_counter(Err(unavailable), property)
                    .expect("optional timing metric may be unavailable"),
                None
            );
        }

        let protocol_error = RealtimeVideoBackendError::IpcProtocol("broken response".to_owned());
        assert!(matches!(
            optional_frame_timing_counter(Err(protocol_error), "vo-delayed-frame-count"),
            Err(RealtimeVideoBackendError::IpcProtocol(_))
        ));

        assert!(!counters_increased(
            (1, 2, Some(3), Some(4)),
            (1, 2, None, None)
        ));
        assert!(counters_increased((1, 2, None, None), (2, 2, None, None)));
    }

    #[test]
    fn phase4_launch_failure_demotes_zero_copy_to_d3d11_copy_without_skipping() {
        let mut runtime = RuntimeState::default();
        runtime.demote(
            "mpv 启动失败".to_owned(),
            1,
            ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
        );

        assert_eq!(
            runtime.backend.launch_mode(),
            MpvLaunchMode::Gpu(MpvGpuProfile::D3d11Copy)
        );
        assert_eq!(runtime.status.activation, BackendActivation::Configured);
        assert_eq!(runtime.status.lifecycle, RendererLifecycleState::Probing);
        assert_eq!(runtime.status.demotion_history.len(), 1);
        assert_eq!(runtime.status.demotion_history[0].reason, "mpv 启动失败");
        assert_runtime_status_contract(&runtime.status);
    }

    #[test]
    fn phase4_demotion_clears_audio_clock_without_an_active_process() {
        let mut runtime = RuntimeState::default();
        runtime.status.activation = BackendActivation::Active;
        runtime.status.lifecycle = RendererLifecycleState::Active;
        runtime.status.process_id = Some(41);
        runtime.status.presented_pts_ms = Some(2_000);
        runtime.status.physical_paused = Some(false);
        runtime.status.physical_eof_reached = Some(false);
        runtime.status.av_sync_drift_ms = Some(17);
        runtime.status.audible_audio_pts_ms = Some(2_017);
        runtime.status.audio_epoch = Some(7);

        runtime.demote(
            "D3D11 设备丢失".to_owned(),
            1,
            ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
        );

        assert_eq!(runtime.status.av_sync_drift_ms, None);
        assert_eq!(runtime.status.audible_audio_pts_ms, None);
        assert_eq!(runtime.status.audio_epoch, None);
        assert_runtime_status_contract(&runtime.status);
    }

    #[test]
    fn phase4_shader_failure_demotes_d3d11_copy_to_vulkan_copy_without_skipping() {
        let mut runtime = RuntimeState::default();
        runtime.demote(
            "D3D11 初始化失败".to_owned(),
            1,
            ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
        );
        runtime.demote(
            "shader 首帧未呈现".to_owned(),
            2,
            ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
        );

        assert_eq!(
            runtime.backend.launch_mode(),
            MpvLaunchMode::Gpu(MpvGpuProfile::VulkanCopy)
        );
        assert_eq!(runtime.status.backend, VideoBackend::RealtimeGpu);
        assert_eq!(runtime.status.demotion_history.len(), 2);
        assert_eq!(
            runtime.status.demotion_history[1].reason,
            "shader 首帧未呈现"
        );
    }

    #[test]
    fn phase4_device_loss_demotes_only_one_backend_step() {
        let mut runtime = RuntimeState::default();
        runtime.status.resume_pts_ms = Some(4_250);
        runtime.demote(
            "D3D11 设备丢失".to_owned(),
            1,
            ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
        );

        assert_eq!(
            runtime.backend.launch_mode(),
            MpvLaunchMode::Gpu(MpvGpuProfile::D3d11Copy)
        );
        assert_eq!(runtime.status.resume_pts_ms, Some(4_250));
        assert_eq!(runtime.status.demotion_history.len(), 1);
    }

    #[test]
    fn phase4_broken_pipe_demotes_once_and_preserves_prior_history() {
        let mut runtime = RuntimeState::default();
        runtime.demote(
            "D3D11 初始化失败".to_owned(),
            1,
            ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
        );
        runtime.demote(
            "mpv IPC 已断开".to_owned(),
            2,
            ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
        );

        assert_eq!(
            runtime.backend.launch_mode(),
            MpvLaunchMode::Gpu(MpvGpuProfile::VulkanCopy)
        );
        assert_eq!(runtime.status.demotion_history.len(), 2);
        assert_eq!(runtime.status.demotion_history[0].to_mode, "gpu_d3d11_copy");
        assert_eq!(
            runtime.status.demotion_history[1].to_mode,
            "gpu_vulkan_copy"
        );
    }

    #[test]
    fn phase4_cpu4_overload_demotes_to_original_without_retrying_gpu() {
        let mut runtime = RuntimeState::default();
        demote_all_gpu_profiles(&mut runtime);
        runtime.demote(
            "CPU4 连续渲染健康窗口超限".to_owned(),
            5,
            ParameterSupportReport::empty(VideoBackend::Cpu4),
        );

        assert_eq!(runtime.backend.launch_mode(), MpvLaunchMode::Original);
        assert_eq!(runtime.status.backend, VideoBackend::Source);
        assert_eq!(
            runtime.status.parameter_support.backend,
            VideoBackend::Source
        );
        assert_eq!(
            runtime.status.support_completeness,
            SupportCompleteness::NotApplicable
        );
        assert_eq!(runtime.status.demotion_history.len(), 5);
        assert_eq!(runtime.status.demotion_history[4].from_mode, "cpu4");
        assert_eq!(runtime.status.demotion_history[4].to_mode, "original");
    }

    #[test]
    fn phase4_original_recovery_is_active_and_keeps_complete_history() {
        let mut runtime = RuntimeState::default();
        demote_all_gpu_profiles(&mut runtime);
        runtime.demote(
            "CPU4 过载".to_owned(),
            5,
            ParameterSupportReport::empty(VideoBackend::Cpu4),
        );
        runtime.status.transition_started_at_unix_ms = Some(3);
        runtime.status.resume_pts_ms = Some(8_500);
        runtime.record_original_started(
            MpvGraphicsApi::D3d11,
            "unreported-original".to_owned(),
            41,
            8_500,
            false,
            false,
        );

        assert_eq!(runtime.backend.launch_mode(), MpvLaunchMode::Original);
        assert_eq!(runtime.status.backend, VideoBackend::Source);
        assert_eq!(runtime.status.activation, BackendActivation::Active);
        assert_eq!(runtime.status.lifecycle, RendererLifecycleState::Active);
        assert_eq!(runtime.status.process_id, Some(41));
        assert_eq!(runtime.status.resume_pts_ms, Some(8_500));
        assert_eq!(runtime.status.demotion_history.len(), 5);
        assert_eq!(runtime.status.fallback_floor_mode, "original");
    }

    #[test]
    fn cpu4_support_report_is_complete_even_without_a_prepared_plan() {
        let support =
            cpu4_parameter_support_from(&ParameterSupportReport::empty(VideoBackend::RealtimeGpu));
        assert_eq!(support.parameters.len(), 83);
        assert_eq!(
            support
                .parameters
                .iter()
                .filter(|parameter| parameter.supported)
                .count(),
            4
        );
        assert_eq!(
            support
                .parameters
                .iter()
                .filter(|parameter| !parameter.supported)
                .count(),
            79
        );
    }

    #[test]
    fn supported_subset_plan_passes_commit_gate() {
        let identity = VideoPlanIdentity {
            session_id: 1,
            playback_generation: 2,
            source_revision: 3,
            parameter_revision: 4,
            sequence: 5,
        };
        let plan = RealtimeVideoPlan {
            slot: VideoPlanSlot::NPlus1,
            identity: identity.clone(),
            target_pts_ms: 8_000,
            period_ms: 8_000,
            seed: 7,
            prepared: true,
            commands: vec![MpvCommand::SetPause { paused: false }],
            parameter_support: ParameterSupportReport {
                backend: VideoBackend::RealtimeGpu,
                fully_supported: false,
                parameters: vec![crate::realtime_video_backend::ParameterSupportResult {
                    field: "video.blur_radius_px".to_owned(),
                    active: true,
                    supported: false,
                    mapping: None,
                    reason: Some("未映射".to_owned()),
                }],
            },
        };
        assert_eq!(
            plan.commit_decision(&VideoCommitGate {
                identity,
                media_pts_ms: 8_000
            }),
            VideoCommitDecision::Commit
        );
    }

    #[test]
    fn realtime_renderer_start_clears_stale_cpu4_diagnostics() {
        let mut runtime = RuntimeState::default();
        runtime.status.backend = VideoBackend::Cpu4;
        runtime.status.graphics_api = Some("software".to_owned());
        runtime.status.decoder = Some("software".to_owned());
        runtime.status.filter = Some("libavfilter eq+hue".to_owned());
        runtime.status.demotion_reason = Some("旧降级原因".to_owned());
        runtime.status.last_demotion = Some(BackendDemotion {
            from: VideoBackend::RealtimeGpu,
            to: VideoBackend::Cpu4,
            from_mode: "gpu_vulkan".to_owned(),
            to_mode: "cpu4".to_owned(),
            reason: "旧降级原因".to_owned(),
            at_unix_ms: 1,
        });
        runtime.status.cycle_drift_ms = Some(34_827);
        runtime.record_renderer_started(
            MpvGraphicsApi::D3d11,
            "d3d11va".to_owned(),
            42,
            0,
            false,
            false,
        );
        let status = runtime.status.clone();
        assert_eq!(status.backend, VideoBackend::RealtimeGpu);
        assert_eq!(status.gpu_adapter, None);
        assert_eq!(status.graphics_api.as_deref(), Some("d3d11"));
        assert_eq!(status.decoder.as_deref(), Some("d3d11va"));
        assert_eq!(status.filter.as_deref(), Some("gpu-next/libplacebo"));
        assert_eq!(status.demotion_reason, None);
        assert_eq!(status.last_demotion, None);
        assert_eq!(status.cycle_drift_ms, None);
    }

    #[test]
    fn realtime_renderer_reports_observed_software_decoder_instead_of_requested_hwdec() {
        let mut runtime = RuntimeState::default();
        runtime.record_renderer_started(
            MpvGraphicsApi::D3d11,
            "software".to_owned(),
            43,
            0,
            false,
            false,
        );

        assert_eq!(runtime.status.graphics_api.as_deref(), Some("d3d11"));
        assert_eq!(runtime.status.decoder.as_deref(), Some("software"));
    }

    #[test]
    fn original_renderer_fixes_the_floor_until_a_new_generation() {
        let mut runtime = RuntimeState::default();
        while runtime.backend.launch_mode() != MpvLaunchMode::Original {
            runtime.backend.demote("进入 Original", 1);
        }
        runtime.record_original_started(
            MpvGraphicsApi::D3d11,
            "unreported-original".to_owned(),
            44,
            0,
            false,
            false,
        );

        assert_eq!(runtime.backend.launch_mode(), MpvLaunchMode::Original);
        assert_eq!(runtime.status.backend, VideoBackend::Source);
        assert_eq!(runtime.status.activation, BackendActivation::Active);
        assert_eq!(runtime.status.lifecycle, RendererLifecycleState::Active);
        assert_eq!(runtime.status.graphics_api.as_deref(), Some("d3d11"));
        assert_eq!(
            runtime.status.decoder.as_deref(),
            Some("unreported-original")
        );
        assert_eq!(
            runtime.status.filter.as_deref(),
            Some("Original（无 shader）")
        );
        assert_eq!(
            runtime
                .status
                .last_demotion
                .as_ref()
                .map(|demotion| demotion.to_mode.as_str()),
            Some("original")
        );
    }

    #[test]
    fn proactive_original_does_not_consume_the_gpu_fallback_floor() {
        let mut runtime = RuntimeState::default();
        runtime.record_original_started(
            MpvGraphicsApi::D3d11,
            "d3d11va".to_owned(),
            45,
            0,
            false,
            false,
        );

        assert_eq!(
            runtime.backend.launch_mode(),
            MpvLaunchMode::Gpu(MpvGpuProfile::D3d11ZeroCopy)
        );
        assert!(runtime.status.demotion_history.is_empty());
        assert_eq!(runtime.status.fallback_floor_mode, "gpu_d3d11_zero_copy");
        assert_eq!(runtime.status.backend, VideoBackend::Source);
    }

    #[test]
    fn prepare_decision_reuses_only_a_gpu_ready_neutral_source_renderer() {
        let mut runtime = RuntimeState::default();
        runtime.backend.demote("D3D11 初始化失败", 1);
        runtime.sync_cursor = Some(sync_cursor());
        runtime.record_original_started(
            MpvGraphicsApi::D3d11,
            "d3d11va-copy".to_owned(),
            46,
            0,
            false,
            false,
        );

        assert!(
            validate_prepare_backend_epoch(runtime.backend_epoch, runtime.backend_epoch).is_ok()
        );
        assert!(validate_prepare_backend_epoch(
            runtime.backend_epoch.saturating_sub(1),
            runtime.backend_epoch,
        )
        .is_err());
        assert_eq!(
            gpu_prepare_disposition(GpuPrepareFacts {
                process_present: true,
                process_exited: false,
                backend_epoch: runtime.backend_epoch,
                lifecycle: runtime.status.lifecycle,
                renderer_ready_for_gpu: false,
                source_matches: true,
                host_window_matches: true,
                generation_changed: false,
            }),
            GpuPrepareDisposition::RestartRenderer
        );
        assert_eq!(
            gpu_prepare_disposition(GpuPrepareFacts {
                process_present: false,
                process_exited: false,
                backend_epoch: runtime.backend_epoch,
                lifecycle: runtime.status.lifecycle,
                renderer_ready_for_gpu: false,
                source_matches: true,
                host_window_matches: true,
                generation_changed: false,
            }),
            GpuPrepareDisposition::RecoverLostSession
        );
        assert_eq!(
            gpu_prepare_disposition(GpuPrepareFacts {
                process_present: true,
                process_exited: false,
                backend_epoch: runtime.backend_epoch,
                lifecycle: runtime.status.lifecycle,
                renderer_ready_for_gpu: true,
                source_matches: true,
                host_window_matches: false,
                generation_changed: false,
            }),
            GpuPrepareDisposition::RestartRenderer
        );
        assert_eq!(
            gpu_prepare_disposition(GpuPrepareFacts {
                process_present: false,
                process_exited: false,
                backend_epoch: runtime.backend_epoch,
                lifecycle: RendererLifecycleState::Stopped,
                renderer_ready_for_gpu: true,
                source_matches: true,
                host_window_matches: true,
                generation_changed: false,
            }),
            GpuPrepareDisposition::RestartRenderer
        );
        assert_eq!(
            gpu_prepare_disposition(GpuPrepareFacts {
                process_present: true,
                process_exited: false,
                backend_epoch: runtime.backend_epoch,
                lifecycle: RendererLifecycleState::Active,
                renderer_ready_for_gpu: true,
                source_matches: true,
                host_window_matches: true,
                generation_changed: false,
            }),
            GpuPrepareDisposition::ReuseRenderer
        );
        runtime.record_renderer_started(
            MpvGraphicsApi::D3d11,
            "d3d11va-copy".to_owned(),
            47,
            0,
            false,
            false,
        );

        assert_eq!(
            runtime.backend.launch_mode(),
            MpvLaunchMode::Gpu(MpvGpuProfile::D3d11Copy)
        );
        assert_eq!(runtime.status.backend, VideoBackend::RealtimeGpu);
        assert_eq!(runtime.status.graphics_api.as_deref(), Some("d3d11"));
        assert_eq!(runtime.status.fallback_floor_mode, "gpu_d3d11_copy");
        assert_eq!(runtime.status.demotion_history.len(), 1);
        assert_eq!(
            runtime.status.demotion_history[0].reason,
            "D3D11 初始化失败"
        );
    }

    #[test]
    fn neutral_source_preloads_the_current_floor_and_reuses_it_for_processing() {
        let gpu = MpvLaunchMode::Gpu(MpvGpuProfile::D3d11Copy);
        assert_eq!(neutral_source_launch_mode(true, gpu), gpu);
        assert_eq!(
            neutral_source_launch_mode(false, gpu),
            MpvLaunchMode::Original
        );
        assert_eq!(
            neutral_source_launch_mode(false, MpvLaunchMode::Cpu4),
            MpvLaunchMode::Cpu4
        );
        assert!(renderer_ready_for_mode(
            VideoBackend::Source,
            Some(gpu),
            gpu
        ));
        assert!(renderer_ready_for_mode(
            VideoBackend::Source,
            Some(MpvLaunchMode::Cpu4),
            MpvLaunchMode::Cpu4
        ));
        assert!(!renderer_ready_for_mode(
            VideoBackend::Source,
            Some(MpvLaunchMode::Original),
            gpu
        ));
    }

    #[test]
    fn neutral_gpu_snapshot_is_a_valid_atomic_shader_update() {
        let options = neutral_gpu_shader_options().expect("default GPU83 options");
        assert!(options.as_str().contains("al_brightness_percent=0"));
        assert!(options.as_str().contains("al_saturation_percent=100"));
    }

    #[test]
    fn stale_prepare_backend_epoch_is_non_destructive_and_does_not_touch_mpv() {
        let root = std::env::temp_dir().join(format!(
            "autolive-stale-video-epoch-{}-{}",
            std::process::id(),
            unix_now_ms()
        ));
        let request = stale_epoch_prepare_request(&root);
        let mut runtime = RuntimeState {
            backend_epoch: 7,
            sync_cursor: Some(sync_cursor()),
            ..RuntimeState::default()
        };
        runtime.backend.demote("existing floor", 1);
        runtime.status.backend_epoch = runtime.backend_epoch;
        let status_before = runtime.status.clone();
        let floor_before = runtime.backend.launch_mode();
        let operation_revision = runtime.operation_revision.load(Ordering::Acquire);

        let error = runtime
            .prepare(request, 123, operation_revision)
            .expect_err("stale epoch must be rejected");

        assert!(matches!(
            error,
            RealtimeVideoBackendError::StalePrepareBackendEpoch {
                requested: 6,
                current: 7
            }
        ));
        assert_eq!(runtime.status, status_before);
        assert_eq!(runtime.backend.launch_mode(), floor_before);
        assert_eq!(runtime.backend_epoch, 7);
        assert_eq!(runtime.sync_cursor, Some(sync_cursor()));
        assert!(runtime.process.is_none());
        assert!(runtime.session.is_none());
        assert!(runtime.source_path.is_none());
        assert!(runtime.pending.is_none());
        assert!(runtime.current.is_none());

        fs::remove_dir_all(root).expect("remove fake runtime root");
    }

    #[test]
    fn renderer_reuse_requires_the_exact_bound_host_window() {
        assert!(!renderer_host_window_changed(false, None, 101));
        assert!(!renderer_host_window_changed(true, Some(101), 101));
        assert!(renderer_host_window_changed(true, Some(100), 101));
        assert!(renderer_host_window_changed(true, None, 101));
    }

    #[test]
    fn source_status_preserves_the_existing_demotion_history() {
        let mut runtime = RuntimeState::default();
        demote_all_gpu_profiles(&mut runtime);
        runtime.record_source_backend("保持 Original".to_owned());

        assert_eq!(runtime.status.demotion_history.len(), 4);
        assert_eq!(runtime.status.fallback_floor_mode, "cpu4");
        assert_eq!(
            runtime
                .status
                .last_demotion
                .as_ref()
                .map(|item| item.to_mode.as_str()),
            Some("cpu4")
        );
    }

    #[test]
    fn presented_pts_is_publishable_on_every_real_clock_advance() {
        let mut runtime = RuntimeState::default();

        assert!(runtime.record_presented_pts(1_000));
        assert!(!runtime.record_presented_pts(1_000));
        assert!(runtime.record_presented_pts(1_040));
        assert_eq!(runtime.status.presented_pts_ms, Some(1_040));
    }

    #[test]
    fn presented_pts_regression_is_rejected_until_an_explicit_boundary() {
        let mut runtime = RuntimeState::default();
        assert!(runtime.record_presented_pts(10_000));
        assert!(!runtime.record_presented_pts(7_000));
        assert_eq!(runtime.status.presented_pts_ms, Some(10_000));

        runtime.pending_boundary = VideoScheduleBoundary::UserSeek;
        assert!(runtime.record_presented_pts(7_000));
        assert_eq!(runtime.status.presented_pts_ms, Some(7_000));
    }

    #[test]
    fn presented_pts_watchdog_is_bounded_and_resets_on_progress() {
        let mut watchdog = PresentedPtsWatchdog::default();
        let threshold = Duration::from_secs(1);

        assert_eq!(
            watchdog.observe(Duration::ZERO, true, 1_000, threshold),
            PresentedPtsLiveness::Healthy
        );
        assert_eq!(
            watchdog.observe(Duration::from_millis(999), true, 1_000, threshold),
            PresentedPtsLiveness::Healthy
        );
        assert_eq!(
            watchdog.observe(Duration::from_millis(1_000), true, 1_000, threshold),
            PresentedPtsLiveness::SoftRecover { position_ms: 1_000 }
        );
        assert_eq!(
            watchdog.observe(Duration::from_millis(1_999), true, 1_000, threshold),
            PresentedPtsLiveness::Healthy
        );
        assert_eq!(
            watchdog.observe(Duration::from_millis(2_000), true, 1_000, threshold),
            PresentedPtsLiveness::Stalled { position_ms: 1_000 }
        );

        assert_eq!(
            watchdog.observe(Duration::from_millis(2_001), true, 1_040, threshold),
            PresentedPtsLiveness::Healthy
        );
        assert_eq!(
            watchdog.observe(Duration::from_millis(3_000), true, 1_040, threshold),
            PresentedPtsLiveness::Healthy
        );
    }

    #[test]
    fn presented_pts_watchdog_threshold_allows_low_frame_rate_sources() {
        assert_eq!(
            presented_pts_stall_threshold(Some(60.0)),
            Duration::from_millis(1_500)
        );
        assert_eq!(
            presented_pts_stall_threshold(Some(1.0)),
            Duration::from_secs(3)
        );
        assert_eq!(
            presented_pts_stall_threshold(Some(f64::NAN)),
            PRESENTED_PTS_STALL_MAX_THRESHOLD
        );
    }

    #[test]
    fn presented_pts_watchdog_ignores_pause_and_eof_gaps() {
        let mut watchdog = PresentedPtsWatchdog::default();
        let threshold = Duration::from_secs(1);
        assert_eq!(
            watchdog.observe(Duration::ZERO, true, 1_000, threshold),
            PresentedPtsLiveness::Healthy
        );
        assert_eq!(
            watchdog.observe(Duration::from_secs(5), false, 1_000, threshold),
            PresentedPtsLiveness::Healthy
        );
        assert_eq!(
            watchdog.observe(Duration::from_secs(6), true, 1_000, threshold),
            PresentedPtsLiveness::Healthy
        );
    }

    #[test]
    fn presented_pts_watchdog_only_runs_for_a_live_managed_playback() {
        let active = PresentedPtsWatchdogFacts {
            process_exists: true,
            paused: false,
            eof_reached: false,
            activation: BackendActivation::Active,
            lifecycle: RendererLifecycleState::Active,
            backend: VideoBackend::RealtimeGpu,
        };
        assert!(should_watch_presented_pts(active));
        assert!(should_watch_presented_pts(PresentedPtsWatchdogFacts {
            backend: VideoBackend::Cpu4,
            ..active
        }));
        assert!(should_watch_presented_pts(PresentedPtsWatchdogFacts {
            backend: VideoBackend::Source,
            ..active
        }));
        for inactive in [
            PresentedPtsWatchdogFacts {
                process_exists: false,
                ..active
            },
            PresentedPtsWatchdogFacts {
                paused: true,
                ..active
            },
            PresentedPtsWatchdogFacts {
                eof_reached: true,
                ..active
            },
            PresentedPtsWatchdogFacts {
                activation: BackendActivation::Failed,
                ..active
            },
            PresentedPtsWatchdogFacts {
                lifecycle: RendererLifecycleState::Stopped,
                ..active
            },
        ] {
            assert!(!should_watch_presented_pts(inactive));
        }
    }

    #[test]
    fn ordinary_runtime_ticks_only_delegate_shader_writes_to_due_commit() {
        let source = include_str!("realtime_video_runtime.rs");
        let tick_start = source.find("    fn tick(&mut self)").expect("tick start");
        let tick_end = source[tick_start..]
            .find("    fn commit_pending_if_due")
            .map(|offset| tick_start + offset)
            .expect("tick end");
        let tick = &source[tick_start..tick_end];
        assert!(!tick.contains("SetShaderOptions"));
        assert!(!tick.contains("build_gpu83_scheduled_shader_update"));
        assert!(tick.contains("commit_pending_if_due"));
        assert!(tick.contains("poll_pending_shader_apply"));

        let commit_start = source.find("    fn commit(").expect("commit start");
        let commit_end = source[commit_start..]
            .find("    fn synchronize(")
            .map(|offset| commit_start + offset)
            .expect("commit end");
        let commit = &source[commit_start..commit_end];
        assert_eq!(
            commit
                .matches("build_gpu83_scheduled_shader_update")
                .count(),
            1
        );
    }

    #[test]
    fn available_gpu_and_cpu4_ticks_share_the_same_due_commit_boundary() {
        let source = include_str!("realtime_video_runtime.rs");
        let tick_start = source.find("    fn tick(&mut self)").expect("tick start");
        let tick_end = source[tick_start..]
            .find("    fn record_presented_pts")
            .map(|offset| tick_start + offset)
            .expect("tick end");
        let tick = &source[tick_start..tick_end];

        assert_eq!(tick.matches("self.commit_pending_if_due(").count(), 3);
    }

    #[test]
    fn suspend_then_resume_preserves_the_complete_demotion_history() {
        let root = std::env::temp_dir().join(format!(
            "autolive-suspend-status-{}-{}",
            std::process::id(),
            unix_now_ms()
        ));
        let request = stale_epoch_prepare_request(&root);
        let mut runtime = RuntimeState {
            session: Some(RendererSessionContext {
                executable: request.executable,
                shader: Some(request.shader),
                session_id: request.session_id,
                source_path: request.source_path,
                host_window_id: request.host_window_id,
                playback_generation: request.plan.identity.playback_generation,
                clock_epoch: request.clock_epoch,
                loop_index: request.loop_index,
                source_position_ms: request.source_start_ms,
                source_duration_ms: request.source_duration_ms,
                paused: request.paused,
            }),
            ..RuntimeState::default()
        };
        demote_all_gpu_profiles(&mut runtime);
        runtime.record_original_started(
            MpvGraphicsApi::D3d11,
            "d3d11va".to_owned(),
            48,
            request.source_start_ms,
            request.paused,
            false,
        );
        assert_eq!(
            runtime.status.presented_pts_ms,
            Some(request.source_start_ms)
        );

        runtime.suspend();
        assert_eq!(runtime.status.activation, BackendActivation::Available);
        assert_eq!(runtime.status.lifecycle, RendererLifecycleState::Stopped);
        assert_eq!(runtime.status.backend, VideoBackend::Source);
        assert_eq!(runtime.status.process_id, None);
        assert_eq!(runtime.status.presented_pts_ms, None);
        assert_eq!(runtime.status.playback_generation, Some(7));
        assert_eq!(runtime.status.clock_epoch, Some(10));
        assert_eq!(runtime.status.loop_index, Some(3));
        assert_eq!(runtime.status.fallback_floor_mode, "cpu4");
        assert_eq!(runtime.status.demotion_history.len(), 4);
        assert_eq!(
            runtime.status.demotion_reason.as_deref(),
            Some("GPU 软件解码失败")
        );
        assert_runtime_status_contract(&runtime.status);

        runtime.claim_cpu4_status();
        assert_eq!(runtime.status.backend, VideoBackend::Cpu4);
        assert_eq!(runtime.status.fallback_floor_mode, "cpu4");
        assert_eq!(runtime.status.demotion_history.len(), 4);
        assert_eq!(
            runtime.status.demotion_history[0].reason,
            "D3D11 zero-copy 失败"
        );
        assert_eq!(
            runtime.status.demotion_history[3].reason,
            "GPU 软件解码失败"
        );

        fs::remove_dir_all(root).expect("remove fake runtime root");
    }

    #[test]
    fn original_renderer_failure_is_observable_without_cpu4_demotion() {
        let mut runtime = RuntimeState::default();
        runtime.record_original_failure("无法创建原生画面".to_owned());

        assert_eq!(runtime.backend.current(), VideoBackend::RealtimeGpu);
        assert_eq!(runtime.status.backend, VideoBackend::Source);
        assert_eq!(runtime.status.activation, BackendActivation::Failed);
        assert_eq!(runtime.status.lifecycle, RendererLifecycleState::Failed);
        assert_eq!(runtime.status.process_id, None);
        assert_eq!(
            runtime.status.parameter_support.backend,
            VideoBackend::Source
        );
        assert!(runtime.status.transition_completed_at_unix_ms.is_some());
        assert_eq!(
            runtime.status.demotion_reason.as_deref(),
            Some("无法创建原生画面")
        );
        assert!(runtime.status.last_demotion.is_none());
        assert_runtime_status_contract(&runtime.status);
    }

    #[test]
    fn runtime_failure_labels_are_bounded_compact_and_redact_known_paths() {
        let sensitive = Path::new(r"C:\Users\alice\Videos\private.mp4");
        let raw = format!(
            "mpv failed for {}\n{}",
            sensitive.display(),
            "error ".repeat(100)
        );
        let sanitized = bounded_runtime_reason(&raw, &[sensitive]);

        assert!(!sanitized.contains("private.mp4"));
        assert!(!sanitized.contains('\n'));
        assert!(sanitized.encode_utf16().count() <= 256);
        assert!(!sanitized.trim().is_empty());
    }

    #[test]
    fn complete_fallback_failure_is_a_bounded_serializable_source_terminal() {
        let mut runtime = RuntimeState::default();
        let long_reason = format!("Original failed\n{}", "x".repeat(600));
        demote_all_gpu_profiles(&mut runtime);
        runtime.demote(
            "CPU4 failed".to_owned(),
            5,
            ParameterSupportReport::empty(VideoBackend::Cpu4),
        );
        runtime.record_original_failure(long_reason);

        assert_eq!(runtime.status.fallback_floor_mode, "original");
        assert_eq!(runtime.status.backend, VideoBackend::Source);
        assert_eq!(runtime.status.activation, BackendActivation::Failed);
        assert_eq!(runtime.status.lifecycle, RendererLifecycleState::Failed);
        assert_eq!(runtime.status.process_id, None);
        assert_eq!(
            runtime.status.parameter_support.backend,
            VideoBackend::Source
        );
        assert_eq!(runtime.status.demotion_history.len(), 5);
        assert_eq!(
            runtime.status.last_demotion.as_ref(),
            runtime.status.demotion_history.last()
        );
        assert!(runtime.status.demotion_history.iter().all(|item| item
            .reason
            .encode_utf16()
            .count()
            <= 256));
        assert!(runtime
            .status
            .demotion_reason
            .as_deref()
            .is_some_and(|reason| reason.encode_utf16().count() <= 256 && !reason.contains('\n')));
        serde_json::to_value(&runtime.status).expect("terminal status serializes");
        assert_runtime_status_contract(&runtime.status);
    }

    #[test]
    fn missing_original_process_is_published_as_failed_without_cpu4_demotion() {
        let mut runtime = RuntimeState::default();
        runtime.status.lifecycle = RendererLifecycleState::Active;

        assert!(runtime.tick());
        assert_eq!(runtime.backend.current(), VideoBackend::RealtimeGpu);
        assert_eq!(runtime.status.backend, VideoBackend::Source);
        assert_eq!(runtime.status.activation, BackendActivation::Failed);
        assert_eq!(runtime.status.lifecycle, RendererLifecycleState::Failed);
        assert_eq!(runtime.status.process_id, None);
        assert!(runtime
            .status
            .demotion_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("进程不存在")));
    }

    #[test]
    fn repeated_sync_values_do_not_reset_epoch() {
        let transition = resolve_sync_transition(
            sync_cursor(),
            RealtimeVideoSync {
                playback_generation: 7,
                clock_epoch: 10,
                loop_index: 3,
                backend_epoch: 0,
                position_ms: 2_000,
                paused: true,
            },
        )
        .expect("same boundary should be accepted");
        assert_eq!(transition.boundary, VideoScheduleBoundary::None);
        assert!(transition.next.paused);
    }

    #[test]
    fn loop_boundary_wins_when_loop_and_clock_advance_together() {
        let transition = resolve_sync_transition(
            sync_cursor(),
            RealtimeVideoSync {
                playback_generation: 7,
                clock_epoch: 11,
                loop_index: 4,
                backend_epoch: 0,
                position_ms: 0,
                paused: false,
            },
        )
        .expect("new loop should be accepted");
        assert_eq!(transition.boundary, VideoScheduleBoundary::LoopBoundary);
        assert_eq!(transition.next.clock_epoch, 11);
        assert_eq!(transition.next.loop_index, 4);
    }

    #[test]
    fn natural_loop_boundary_does_not_require_a_seek_epoch_advance() {
        let transition = resolve_sync_transition(
            sync_cursor(),
            RealtimeVideoSync {
                playback_generation: 7,
                clock_epoch: 10,
                loop_index: 4,
                backend_epoch: 0,
                position_ms: 0,
                paused: false,
            },
        )
        .expect("natural loop should be accepted");
        assert_eq!(transition.boundary, VideoScheduleBoundary::LoopBoundary);
        assert_eq!(transition.next.clock_epoch, 10);
        assert_eq!(transition.next.loop_index, 4);
    }

    #[test]
    fn unconsumed_loop_boundary_cannot_be_downgraded_by_a_seek() {
        assert_eq!(
            merge_pending_boundary(
                VideoScheduleBoundary::LoopBoundary,
                VideoScheduleBoundary::UserSeek,
            ),
            VideoScheduleBoundary::LoopBoundary
        );
        assert_eq!(
            merge_pending_boundary(
                VideoScheduleBoundary::UserSeek,
                VideoScheduleBoundary::LoopBoundary,
            ),
            VideoScheduleBoundary::LoopBoundary
        );
    }

    #[test]
    fn gpu_restart_replays_the_latest_active_shader_before_resuming() {
        let committed = MpvShaderOptions::parse("al_brightness_percent=1".to_owned())
            .expect("committed shader options");
        let scheduled = MpvShaderOptions::parse("al_brightness_percent=2".to_owned())
            .expect("latest scheduled shader options");
        let plan = RealtimeVideoPlan {
            slot: VideoPlanSlot::NPlus1,
            identity: VideoPlanIdentity {
                session_id: 1,
                playback_generation: 1,
                source_revision: 1,
                parameter_revision: 1,
                sequence: 1,
            },
            target_pts_ms: 1_000,
            period_ms: 1_000,
            seed: 1,
            prepared: true,
            commands: vec![MpvCommand::SetShaderOptions {
                options: committed.clone(),
            }],
            parameter_support: ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
        };

        assert_eq!(
            gpu_replay_commands(Some(&plan), Some(&scheduled)),
            vec![MpvCommand::SetShaderOptions { options: scheduled }]
        );
        assert_eq!(
            gpu_replay_commands(Some(&plan), None),
            vec![MpvCommand::SetShaderOptions {
                options: committed.clone(),
            }]
        );
        assert!(gpu_replay_commands(None, Some(&committed)).is_empty());
    }

    #[test]
    fn gpu_launch_replays_active_parameters_before_resuming_playback() {
        let source = include_str!("realtime_video_runtime.rs");
        let launch_start = source
            .find("fn launch_gpu_renderer(")
            .expect("GPU launch function");
        let launch_end = source[launch_start..]
            .find("fn launch_mode_renderer(")
            .map(|offset| launch_start + offset)
            .expect("next launch function");
        let launch = &source[launch_start..launch_end];

        let connect = launch
            .find("spawn_connected_cancelable")
            .expect("connect mpv");
        let replay = launch
            .find("for command in startup_commands")
            .expect("replay active parameters");
        let resume = launch
            .find("restore_requested_playback_state")
            .expect("restore playback state");
        let first_frame = launch
            .find("wait_for_video_output")
            .expect("wait for video output");

        assert!(connect < replay);
        assert!(replay < resume);
        assert!(resume < first_frame);

        let fallback_start = source
            .find("fn start_gpu_fallback(")
            .expect("GPU fallback function");
        let fallback_end = source[fallback_start..]
            .find("fn record_original_failure(")
            .map(|offset| fallback_start + offset)
            .expect("next runtime function");
        let fallback = &source[fallback_start..fallback_end];
        let fallback_reset = fallback
            .find("self.reset_scheduler(")
            .expect("reset fallback scheduler");
        let fallback_restore = fallback
            .find("self.last_shader_options = replayed_shader_options")
            .expect("restore replayed shader snapshot");
        assert!(fallback_reset < fallback_restore);

        let restart_start = source
            .find("let must_restart = disposition == GpuPrepareDisposition::RestartRenderer;")
            .expect("GPU restart branch");
        let restart_end = source[restart_start..]
            .find("self.pending_schedule = Some")
            .map(|offset| restart_start + offset)
            .expect("GPU restart branch end");
        let restart = &source[restart_start..restart_end];
        let preserve = restart
            .find("let replayed_shader_options =")
            .expect("preserve active shader snapshot");
        let restart_renderer = restart
            .find("self.start_renderer(")
            .expect("restart renderer");
        let restart_reset = restart
            .find("self.reset_scheduler(")
            .expect("reset restarted scheduler");
        let restart_restore = restart
            .find("self.last_shader_options = replayed_shader_options")
            .expect("restore restarted shader snapshot");
        assert!(preserve < restart_renderer);
        assert!(restart_renderer < restart_reset);
        assert!(restart_reset < restart_restore);
    }

    #[test]
    fn audible_audio_scheduler_and_av_sync_share_one_bounded_speed_value() {
        assert_eq!(
            final_playback_speed(1.2, 1.0, 0.99).map(MpvPlaybackSpeed::as_f64),
            Some(1.188)
        );
        assert_eq!(
            final_playback_speed(2.0, 1.5, 1.02).map(MpvPlaybackSpeed::as_f64),
            Some(3.06)
        );
        assert!(final_playback_speed(f64::NAN, 1.0, 1.0).is_none());
    }

    #[test]
    fn av_sync_hard_seek_rebases_the_cycle_only_after_seek_success() {
        let source = include_str!("realtime_video_runtime.rs");
        let observe = source
            .split("    fn observe_av_sync(")
            .nth(1)
            .expect("AV sync observation")
            .split("    fn tick(&mut self)")
            .next()
            .expect("AV sync observation end");
        let hard_seek = observe
            .find("if let AvSyncAction::HardSeek")
            .expect("hard seek branch");
        let seek = observe[hard_seek..]
            .find("&MpvCommand::SeekAbsoluteMs")
            .map(|offset| hard_seek + offset)
            .expect("physical hard seek");
        let rebase = observe[hard_seek..]
            .find(".rebase_source_pts(target)")
            .map(|offset| hard_seek + offset)
            .expect("cycle PTS rebase");
        let reset_frame_budget = observe[hard_seek..]
            .find("self.begin_frame_budget_transition();")
            .map(|offset| hard_seek + offset)
            .expect("frame health baseline reset after hard seek");
        let resume = observe[hard_seek..]
            .find("if !cursor.paused")
            .map(|offset| hard_seek + offset)
            .expect("playback resume after seek");

        assert!(
            hard_seek < seek
                && seek < rebase
                && rebase < reset_frame_budget
                && reset_frame_budget < resume
        );
        assert!(observe[seek..rebase].contains(")?;"));
        assert!(!observe[hard_seek..seek].contains(".rebase_source_pts(target)"));
    }

    #[test]
    fn a_transient_audio_clock_gap_preserves_av_identity_and_shader_apply_disables_watchdog() {
        let source = include_str!("realtime_video_runtime.rs");
        let observe = source
            .split("    fn observe_av_sync(")
            .nth(1)
            .expect("AV sync observation")
            .split("        let Some(snapshot) = self")
            .nth(1)
            .expect("audio snapshot branch")
            .split("        let identity_changed")
            .next()
            .expect("missing audio clock branch");
        assert!(observe.contains("self.av_sync_audio_identity.is_some()"));
        assert!(observe.contains("audible_source_pts_ms: None"));
        assert!(!observe
            .contains("self.reset_av_sync();\n            return self.apply_playback_speed"));

        let watchdog = source
            .split("    fn observe_presented_pts_liveness(")
            .nth(1)
            .expect("PTS watchdog")
            .split("        if self.nominal_source_fps.is_none()")
            .next()
            .expect("watchdog early guards");
        assert!(watchdog.contains("if self.pending_shader_apply.is_some()"));
        assert!(watchdog.contains("self.presented_pts_watchdog.reset()"));
    }

    #[test]
    fn clock_epoch_advance_without_loop_is_a_seek() {
        let transition = resolve_sync_transition(
            sync_cursor(),
            RealtimeVideoSync {
                playback_generation: 7,
                clock_epoch: 11,
                loop_index: 3,
                backend_epoch: 0,
                position_ms: 4_500,
                paused: false,
            },
        )
        .expect("new clock epoch should be accepted");
        assert_eq!(transition.boundary, VideoScheduleBoundary::UserSeek);
    }

    #[test]
    fn stale_sync_is_rejected_without_mutating_current_cursor() {
        let current = sync_cursor();
        let stale = RealtimeVideoSync {
            playback_generation: 7,
            clock_epoch: 9,
            loop_index: 3,
            backend_epoch: 0,
            position_ms: 1_000,
            paused: true,
        };
        assert!(matches!(
            resolve_sync_transition(current, stale),
            Err(RealtimeVideoBackendError::StaleSync { .. })
        ));
        assert_eq!(current, sync_cursor());
    }

    #[test]
    fn newer_operation_revision_supersedes_sync_before_state_mutation() {
        let mut runtime = RuntimeState::default();
        runtime.operation_revision.store(2, Ordering::Release);
        let error = runtime
            .synchronize(
                RealtimeVideoSync {
                    playback_generation: 7,
                    clock_epoch: 10,
                    loop_index: 3,
                    backend_epoch: 0,
                    position_ms: 1_000,
                    paused: false,
                },
                1,
                0,
            )
            .expect_err("older sync revision must be superseded");
        assert!(matches!(
            error,
            RealtimeVideoBackendError::SyncSuperseded { .. }
        ));
        assert!(runtime.sync_cursor.is_none());
    }

    #[test]
    fn eof_fact_is_bound_to_the_complete_runtime_identity() {
        let fact = eof_fact_from_cursor(Some(sync_cursor()), 4, true).expect("EOF fact");
        assert_eq!(fact.playback_generation, 7);
        assert_eq!(fact.backend_epoch, 4);
        assert_eq!(fact.clock_epoch, 10);
        assert_eq!(fact.loop_index, 3);
        assert!(eof_fact_from_cursor(Some(sync_cursor()), 4, false).is_none());
        assert!(eof_fact_from_cursor(None, 4, true).is_none());
        assert!(eof_fact_from_cursor(
            Some(SyncCursor {
                paused: true,
                ..sync_cursor()
            }),
            4,
            true,
        )
        .is_none());
    }

    #[test]
    fn stale_eof_request_does_not_publish_or_enter_source_transition() {
        let current_eof = VideoEofFact {
            playback_generation: 7,
            backend_epoch: 4,
            clock_epoch: 3,
            loop_index: 2,
        };
        let mut runtime = RuntimeState {
            backend_epoch: 4,
            status: MediaVideoBackendRuntimeStatus {
                status_revision: 41,
                apply_state: VideoApplyState::Active,
                active_plan_fingerprint: Some("confirmed".to_owned()),
                eof: Some(current_eof),
                ..MediaVideoBackendRuntimeStatus::default()
            },
            status_revision_counter: 41,
            ..RuntimeState::default()
        };
        let before = runtime.status.clone();
        let snapshot = RwLock::new(before.clone());
        let (reply, response) = mpsc::sync_channel(1);
        runtime.handle(
            RuntimeCommand::AdvanceAfterEof {
                request: AdvanceRealtimeVideoAfterEof {
                    expected_eof: VideoEofFact {
                        clock_epoch: 2,
                        ..current_eof
                    },
                    next_playback_generation: 7,
                    next_loop_index: 3,
                    next_source_path: PathBuf::from("stale.mp4"),
                    next_source_duration_ms: 60_000,
                    paused: false,
                },
                operation_revision: 1,
                now_unix_ms: 0,
                reply,
            },
            &snapshot,
        );

        assert!(matches!(
            response.recv().expect("stale reply"),
            Err(RealtimeVideoBackendError::StaleSync {
                field: "EOF 身份"
            })
        ));
        assert_eq!(runtime.status, before);
        assert_eq!(*snapshot.read().expect("snapshot"), before);
    }

    #[test]
    fn seek_loop_and_new_session_clear_the_previous_eof_fact() {
        let mut runtime = RuntimeState::default();
        runtime.reset_scheduler(7, 10, 3, false);
        runtime.status.eof = eof_fact_from_cursor(runtime.sync_cursor, 1, true);
        assert!(runtime.status.eof.is_some());

        runtime.clear_eof_fact();
        assert!(runtime.status.eof.is_none());
        runtime.status.eof = eof_fact_from_cursor(runtime.sync_cursor, 1, true);
        runtime.reset_scheduler(7, 11, 4, false);
        assert!(runtime.status.eof.is_none());
        runtime.status.eof = eof_fact_from_cursor(runtime.sync_cursor, 1, true);
        runtime.stop();
        assert!(runtime.status.eof.is_none());
    }

    #[test]
    fn stop_process_clears_every_physical_process_fact() {
        let mut runtime = RuntimeState::default();
        runtime.reset_scheduler(7, 10, 3, false);
        runtime.status.process_id = Some(42);
        runtime.status.presented_pts_ms = Some(1_000);
        runtime.status.physical_paused = Some(false);
        runtime.status.physical_eof_reached = Some(true);
        runtime.status.eof = eof_fact_from_cursor(runtime.sync_cursor, 1, true);

        runtime.stop_process();

        assert_eq!(runtime.status.process_id, None);
        assert_eq!(runtime.status.presented_pts_ms, None);
        assert_eq!(runtime.status.physical_paused, None);
        assert_eq!(runtime.status.physical_eof_reached, None);
        assert_eq!(runtime.status.eof, None);
    }

    #[test]
    fn sync_cannot_skip_more_than_one_loop_boundary() {
        let current = sync_cursor();
        let skipped = RealtimeVideoSync {
            playback_generation: 7,
            clock_epoch: 11,
            loop_index: 5,
            backend_epoch: 0,
            position_ms: 0,
            paused: false,
        };
        assert!(resolve_sync_transition(current, skipped).is_err());
        assert_eq!(current, sync_cursor());
    }

    #[test]
    fn authoritative_loop_advance_discards_only_the_old_pending_plan() {
        let active = RealtimeVideoPlan {
            slot: VideoPlanSlot::N,
            identity: VideoPlanIdentity {
                session_id: 7,
                playback_generation: 7,
                source_revision: 1,
                parameter_revision: 1,
                sequence: 1,
            },
            target_pts_ms: 1_000,
            period_ms: 1_000,
            seed: 1,
            prepared: true,
            commands: Vec::new(),
            parameter_support: ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
        };
        let pending = RealtimeVideoPlan {
            slot: VideoPlanSlot::NPlus1,
            identity: VideoPlanIdentity {
                parameter_revision: 2,
                sequence: 2,
                ..active.identity.clone()
            },
            target_pts_ms: 2_000,
            period_ms: 1_000,
            seed: 2,
            prepared: true,
            commands: Vec::new(),
            parameter_support: ParameterSupportReport::empty(VideoBackend::RealtimeGpu),
        };
        let mut runtime = RuntimeState {
            current: Some(active.clone()),
            pending: Some(pending.clone()),
            pending_schedule: Some(PreparedScheduleContext {
                identity: pending.identity.clone(),
                seed: pending.seed,
                video_params: VideoEffectParams::default(),
                advanced_params: AdvancedEffectParams::default(),
                clock_epoch: 10,
                loop_index: 3,
                paused: false,
            }),
            ..RuntimeState::default()
        };
        runtime.status.n = Some(slot_status(&active, CycleSlotState::Active));
        runtime.status.n1 = Some(slot_status(&pending, CycleSlotState::Ready));
        runtime.status.n2 = Some(CycleSlotStatus {
            sequence: 3,
            target_pts_ms: 3_000,
            status: CycleSlotState::Planned,
        });

        runtime.discard_pending_outside_cursor(SyncCursor {
            loop_index: 4,
            ..sync_cursor()
        });

        assert_eq!(
            runtime.current.as_ref().map(|plan| plan.identity.sequence),
            Some(1)
        );
        assert_eq!(runtime.status.n.as_ref().map(|slot| slot.sequence), Some(1));
        assert!(runtime.pending.is_none());
        assert!(runtime.pending_schedule.is_none());
        assert!(runtime.status.n1.is_none());
        assert!(runtime.status.n2.is_none());

        runtime.pending = Some(pending.clone());
        runtime.pending_schedule = Some(PreparedScheduleContext {
            identity: pending.identity.clone(),
            seed: pending.seed,
            video_params: VideoEffectParams::default(),
            advanced_params: AdvancedEffectParams::default(),
            clock_epoch: 10,
            loop_index: 4,
            paused: false,
        });
        runtime.status.n1 = Some(slot_status(&pending, CycleSlotState::Ready));

        runtime.discard_pending_outside_cursor(SyncCursor {
            loop_index: 4,
            ..sync_cursor()
        });

        assert_eq!(
            runtime.pending.as_ref().map(|plan| plan.identity.sequence),
            Some(2)
        );
        assert!(runtime.pending_schedule.is_some());
        assert_eq!(
            runtime.status.n1.as_ref().map(|slot| slot.sequence),
            Some(2)
        );
    }

    #[test]
    fn sync_commands_seek_only_on_a_new_epoch_and_preserve_pause_order() {
        let current = sync_cursor();
        let repeated = SyncTransition {
            next: current,
            boundary: VideoScheduleBoundary::None,
        };
        assert_eq!(sync_command_batch(current, repeated, 9_000), [None, None]);
        let pause_only = SyncTransition {
            next: SyncCursor {
                paused: true,
                ..current
            },
            boundary: VideoScheduleBoundary::None,
        };
        assert_eq!(
            sync_command_batch(current, pause_only, 9_000),
            [Some(MpvCommand::SetPause { paused: true }), None]
        );

        let pause_and_seek = SyncTransition {
            next: SyncCursor {
                clock_epoch: 11,
                paused: true,
                ..current
            },
            boundary: VideoScheduleBoundary::UserSeek,
        };
        assert_eq!(
            sync_command_batch(current, pause_and_seek, 4_500),
            [
                Some(MpvCommand::SetPause { paused: true }),
                Some(MpvCommand::SeekAbsoluteMs { position_ms: 4_500 }),
            ]
        );

        let paused = pause_and_seek.next;
        let seek_and_resume = SyncTransition {
            next: SyncCursor {
                clock_epoch: 12,
                paused: false,
                ..paused
            },
            boundary: VideoScheduleBoundary::UserSeek,
        };
        assert_eq!(
            sync_command_batch(paused, seek_and_resume, 7_250),
            [
                Some(MpvCommand::SeekAbsoluteMs { position_ms: 7_250 }),
                Some(MpvCommand::SetPause { paused: false }),
            ]
        );

        let loop_boundary = SyncTransition {
            next: SyncCursor {
                loop_index: 4,
                ..current
            },
            boundary: VideoScheduleBoundary::LoopBoundary,
        };
        assert_eq!(
            sync_command_batch(current, loop_boundary, 0),
            [
                Some(MpvCommand::SeekAbsoluteMs { position_ms: 0 }),
                Some(MpvCommand::SetPause { paused: false }),
            ]
        );
    }

    #[test]
    fn every_timeline_boundary_discards_candidates_from_the_old_cursor() {
        let source = include_str!("realtime_video_runtime.rs");
        let synchronize = source
            .split("    fn synchronize(")
            .nth(1)
            .expect("runtime synchronize")
            .split("    fn advance_after_eof(")
            .next()
            .expect("runtime synchronize end");

        assert!(synchronize.contains(
            "if !matches!(transition.boundary, VideoScheduleBoundary::None) {\n            self.discard_pending_outside_cursor(transition.next);"
        ));
    }

    #[test]
    fn source_position_equal_to_duration_is_rejected() {
        assert!(valid_source_position(72_299, 72_300));
        assert!(!valid_source_position(72_300, 72_300));
        assert!(!valid_source_position(0, 0));
    }

    #[test]
    fn full_control_channel_maps_to_stable_queue_error() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let (first_reply, _first_response) = mpsc::sync_channel(1);
        enqueue(&sender, RuntimeCommand::Stop { reply: first_reply }).expect("first slot");
        let (second_reply, _second_response) = mpsc::sync_channel(1);
        assert!(matches!(
            enqueue(
                &sender,
                RuntimeCommand::Stop {
                    reply: second_reply
                }
            ),
            Err(RealtimeVideoBackendError::IpcQueueFull)
        ));
    }

    #[test]
    fn actor_timeout_publishes_status_before_eof_notification() {
        let source = include_str!("realtime_video_runtime.rs");
        let actor_start = source.find("fn actor_loop(").expect("actor loop start");
        let actor_end = source[actor_start..]
            .find("fn publish_status(")
            .map(|offset| actor_start + offset)
            .expect("actor loop end");
        let actor = &source[actor_start..actor_end];
        let tick = actor.find("state.tick()").expect("runtime tick");
        let publish = actor[tick..]
            .find("publish_status(&status_snapshot, &mut state)")
            .map(|offset| tick + offset)
            .expect("status publication after tick");
        let notify = actor[tick..]
            .find("state.notify_eof_supervisor()")
            .map(|offset| tick + offset)
            .expect("EOF notification after tick");

        assert!(tick < publish);
        assert!(publish < notify);
    }

    #[test]
    fn physical_eof_waits_for_published_status_and_notifies_once() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut runtime = RuntimeState {
            eof_event_sender: Some(sender),
            ..RuntimeState::default()
        };
        runtime.backend_epoch = 9;
        runtime.sync_cursor = Some(SyncCursor {
            playback_generation: 7,
            clock_epoch: 3,
            loop_index: 2,
            paused: false,
        });

        assert!(runtime.record_eof_observation(true, true));
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        let snapshot = RwLock::new(MediaVideoBackendRuntimeStatus::default());
        publish_status(&snapshot, &mut runtime);
        assert!(runtime.notify_eof_supervisor());
        let notified = receiver.try_recv().expect("first EOF notification");
        assert_eq!(
            notified,
            VideoEofFact {
                playback_generation: 7,
                backend_epoch: 9,
                clock_epoch: 3,
                loop_index: 2,
            }
        );
        assert_eq!(snapshot.read().expect("snapshot").eof, Some(notified));
        assert!(!runtime.record_eof_observation(true, true));
        assert!(!runtime.notify_eof_supervisor());
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));

        assert!(runtime.record_eof_observation(false, false));
        assert!(runtime.record_eof_observation(true, true));
        publish_status(&snapshot, &mut runtime);
        assert!(runtime.notify_eof_supervisor());
        assert_eq!(
            receiver.try_recv().expect("next-loop EOF notification"),
            VideoEofFact {
                playback_generation: 7,
                backend_epoch: 9,
                clock_epoch: 3,
                loop_index: 2,
            }
        );
    }

    #[test]
    fn full_eof_channel_retries_and_successful_notification_is_not_duplicated() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let blocking_eof = VideoEofFact {
            playback_generation: 1,
            backend_epoch: 1,
            clock_epoch: 1,
            loop_index: 1,
        };
        sender.try_send(blocking_eof).expect("fill EOF channel");
        let mut runtime = RuntimeState {
            eof_event_sender: Some(sender),
            backend_epoch: 9,
            sync_cursor: Some(SyncCursor {
                playback_generation: 7,
                clock_epoch: 3,
                loop_index: 2,
                paused: false,
            }),
            ..RuntimeState::default()
        };

        assert!(runtime.record_eof_observation(true, true));
        assert!(!runtime.notify_eof_supervisor());
        assert_eq!(receiver.try_recv().expect("blocking EOF"), blocking_eof);
        assert!(runtime.notify_eof_supervisor());
        assert_eq!(
            receiver.try_recv().expect("retried EOF notification"),
            runtime.status.eof.expect("current EOF")
        );
        assert!(!runtime.notify_eof_supervisor());
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn shutdown_joins_worker_and_seals_new_requests() {
        let runtime = RealtimeVideoRuntime::default();
        runtime.shutdown().expect("shutdown actor");
        let lifecycle = runtime.lifecycle.lock().expect("lifecycle");
        assert!(lifecycle.sender.is_none());
        assert!(lifecycle.worker.is_none());
        drop(lifecycle);
        assert!(matches!(
            runtime.sender(),
            Err(RealtimeVideoBackendError::IpcDisconnected(_))
        ));
    }

    #[test]
    fn stop_publishes_stopped_status_before_returning() {
        let runtime = RealtimeVideoRuntime::default();
        runtime.stop(7).expect("stop actor");
        assert_eq!(runtime.status().lifecycle, RendererLifecycleState::Stopped);
    }

    #[test]
    fn dropping_sender_lets_actor_stop_without_a_detached_join_handle() {
        let status = Arc::new(RwLock::new(MediaVideoBackendRuntimeStatus::default()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(CONTROL_CHANNEL_CAPACITY);
        let worker_status = Arc::clone(&status);
        let worker_shutdown = Arc::clone(&shutdown);
        let operation_revision = Arc::new(AtomicU64::new(1));
        let worker = thread::spawn(move || {
            actor_loop(
                receiver,
                worker_status,
                worker_shutdown,
                operation_revision,
                Arc::new(AudibleAudioClock::default()),
                None,
            )
        });
        drop(sender);
        worker.join().expect("actor exits after channel disconnect");
        assert_eq!(
            status.read().expect("status").lifecycle,
            RendererLifecycleState::Stopped
        );
    }

    #[test]
    fn canonical_plan_fingerprint_covers_options_identity_epochs_loop_and_actual_fps() {
        let options = MpvShaderOptions::parse("z=2,a=1".to_owned()).expect("options");
        let reordered = MpvShaderOptions::parse("a=1,z=2".to_owned()).expect("options");
        let identity = VideoPlanIdentity {
            session_id: 7,
            playback_generation: 11,
            source_revision: 13,
            parameter_revision: 17,
            sequence: 19,
        };
        let base = canonical_shader_plan_fingerprint(&options, &identity, 23, 29, 30.0)
            .expect("fingerprint");
        assert_eq!(
            base,
            canonical_shader_plan_fingerprint(&reordered, &identity, 23, 29, 30.0)
                .expect("canonical order")
        );
        assert_ne!(
            base,
            canonical_shader_plan_fingerprint(&options, &identity, 24, 29, 30.0)
                .expect("clock identity")
        );
        assert_ne!(
            base,
            canonical_shader_plan_fingerprint(&options, &identity, 23, 30, 30.0)
                .expect("loop identity")
        );
        assert_ne!(
            base,
            canonical_shader_plan_fingerprint(&options, &identity, 23, 29, 29.97)
                .expect("actual fps")
        );
        let mut next_identity = identity.clone();
        next_identity.sequence += 1;
        assert_ne!(
            base,
            canonical_shader_plan_fingerprint(&options, &next_identity, 23, 29, 30.0)
                .expect("sequence identity")
        );
        assert_ne!(
            base,
            canonical_shader_plan_fingerprint(
                &MpvShaderOptions::parse("a=1,z=3".to_owned()).expect("changed options"),
                &identity,
                23,
                29,
                30.0,
            )
            .expect("complete options")
        );

        let (marked, fingerprint) =
            attach_shader_plan_fingerprint(&options, &identity, 23, 29, 30.0).expect("markers");
        assert_eq!(fingerprint, base);
        let marked = marked.option_map().expect("marked map");
        for key in [SHADER_PLAN_MARKER_HIGH, SHADER_PLAN_MARKER_LOW] {
            let value = marked
                .get(key)
                .expect("marker exists")
                .parse::<u32>()
                .expect("24-bit integer");
            assert!(value <= 0x00ff_ffff);
        }
    }

    #[test]
    fn status_revision_stays_monotonic_across_default_status_replacements() {
        let snapshot = RwLock::new(MediaVideoBackendRuntimeStatus::default());
        let mut runtime = RuntimeState::default();
        publish_status(&snapshot, &mut runtime);
        let before_reset = runtime.status.status_revision;

        runtime.reset_generation_state_preserving_process();
        publish_status(&snapshot, &mut runtime);
        let after_generation_reset = runtime.status.status_revision;
        assert!(after_generation_reset > before_reset);

        runtime.record_source_backend("source fallback".to_owned());
        publish_status(&snapshot, &mut runtime);
        assert!(runtime.status.status_revision > after_generation_reset);
    }

    #[test]
    fn source_transition_is_published_before_long_running_cycle_configuration() {
        let source = include_str!("realtime_video_runtime.rs");
        let arm = source
            .split("RuntimeCommand::ConfigureCycle {")
            .nth(2)
            .expect("configure actor arm")
            .split("RuntimeCommand::Commit {")
            .next()
            .expect("configure actor body");
        let transition = arm
            .find("self.invalidate_cycle_before_source_recovery()")
            .expect("transition");
        let publish = arm
            .find("publish_status(status_snapshot, self)")
            .expect("publish");
        let configure = arm.find("self.configure_cycle(").expect("configure");
        assert!(transition < publish && publish < configure);
    }

    #[test]
    fn logical_source_with_a_physical_effect_session_is_cycle_capable() {
        for physical_mode in [
            MpvLaunchMode::Gpu(MpvGpuProfile::D3d11ZeroCopy),
            MpvLaunchMode::Cpu4,
        ] {
            assert!(cycle_session_is_available(
                true,
                true,
                true,
                Some(physical_mode),
            ));
        }
    }

    #[test]
    fn original_or_missing_physical_session_cannot_own_a_video_cycle() {
        assert!(!cycle_session_is_available(
            true,
            true,
            true,
            Some(MpvLaunchMode::Original),
        ));
        assert!(!cycle_session_is_available(
            true,
            false,
            true,
            Some(MpvLaunchMode::Gpu(MpvGpuProfile::D3d11ZeroCopy)),
        ));
        assert!(!cycle_session_is_available(
            true,
            true,
            false,
            Some(MpvLaunchMode::Cpu4),
        ));

        let mut runtime = RuntimeState::default();
        runtime.status.backend = VideoBackend::Source;
        runtime.status.activation = BackendActivation::Active;
        runtime.status.lifecycle = RendererLifecycleState::Active;
        runtime.status.apply_state = VideoApplyState::SourceTransitioning;

        runtime.finish_cycle_configuration(
            VideoCycleConfig::try_new(
                true,
                5_000,
                8_000,
                crate::media_effect_params::MediaEffectParams::default(),
            )
            .expect("valid cycle config"),
        );

        assert_eq!(runtime.status.activation, BackendActivation::Active);
        assert_eq!(runtime.status.lifecycle, RendererLifecycleState::Active);
        assert_eq!(runtime.status.apply_state, VideoApplyState::Idle);
        assert!(runtime.cycle_controller.is_none());
    }

    #[test]
    fn source_observation_fast_path_does_not_preempt_an_installed_cycle_controller() {
        let source = include_str!("realtime_video_runtime.rs");
        let fast_path = source
            .split(
                "if self.status.backend == VideoBackend::Source\n            && self.status.lifecycle",
            )
            .nth(1)
            .expect("source observation fast path")
            .split("if self.status.activation == BackendActivation::Available")
            .next()
            .expect("source observation fast path end");

        assert!(fast_path.contains("self.cycle_controller.is_none()"));
    }

    #[test]
    fn first_observation_completes_fps_after_a_boundary_created_the_identity() {
        let config = VideoCycleConfig::try_new(
            true,
            5_000,
            8_000,
            crate::media_effect_params::MediaEffectParams::default(),
        )
        .expect("valid cycle config");
        let media = MediaSegmentIdentity::try_new(7, PathBuf::from("new.ts"), 3, 60_000)
            .expect("valid media identity");
        let identity =
            VideoCycleSegmentIdentity::try_new(11, 13, 17, media).expect("valid cycle identity");
        let mut controller = VideoCycleController::new(config);
        controller
            .handle(VideoCycleEvent::SourceBoundary {
                identity,
                source_fps: None,
                source_pts_ms: 0,
            })
            .expect("source boundary without fps");
        let mut runtime = RuntimeState {
            cycle_controller: Some(controller),
            sync_cursor: Some(sync_cursor()),
            ..RuntimeState::default()
        };

        assert!(runtime
            .initialize_cycle_after_source_observation(500, 30.0)
            .expect("complete first source observation"));

        let controller = runtime.cycle_controller.as_ref().expect("controller kept");
        assert_eq!(controller.source_fps(), Some(30.0));
        assert!(controller.queue().n_plus_1.is_some());
        assert!(controller.queue().n_plus_2.is_some());
    }

    #[test]
    fn first_controller_plan_compiles_a_non_neutral_shader_snapshot() {
        let config = VideoCycleConfig::try_new(
            true,
            4_000,
            4_000,
            crate::media_effect_params::MediaEffectParams::default(),
        )
        .expect("valid cycle config");
        let media = MediaSegmentIdentity::try_new(7, PathBuf::from("first.mp4"), 0, 60_000)
            .expect("valid media identity");
        let identity =
            VideoCycleSegmentIdentity::try_new(7, 0, 1, media).expect("valid cycle identity");
        let mut controller = VideoCycleController::new(config);
        controller
            .handle(VideoCycleEvent::SourceBoundary {
                identity,
                source_fps: Some(30.0),
                source_pts_ms: 0,
            })
            .expect("first source boundary");
        let mut runtime = RuntimeState {
            processing_enabled: true,
            process_launch_mode: Some(MpvLaunchMode::Gpu(MpvGpuProfile::D3d11ZeroCopy)),
            cycle_controller: Some(controller),
            ..RuntimeState::default()
        };
        runtime.record_parameter_support(gpu83_cycle_parameter_support());

        runtime
            .prepare_next_controller_plan()
            .expect("compile first controller plan");

        let first_options = runtime
            .pending
            .as_ref()
            .and_then(|plan| {
                plan.commands.iter().find_map(|command| match command {
                    MpvCommand::SetShaderOptions { options } => Some(options),
                    _ => None,
                })
            })
            .expect("first shader snapshot")
            .option_map()
            .expect("first shader option map");
        let neutral_options = neutral_gpu_shader_options()
            .expect("neutral shader snapshot")
            .option_map()
            .expect("neutral shader option map");
        assert_ne!(
            first_options.get("al_noise_percent"),
            neutral_options.get("al_noise_percent")
        );
    }

    #[test]
    fn eof_boundary_only_yields_an_unpresented_readback_transaction() {
        assert!(shader_presentation_can_yield_to_eof(
            PendingShaderApplyPhase::AwaitingPresentation,
            true,
        ));
        assert!(!shader_presentation_can_yield_to_eof(
            PendingShaderApplyPhase::AwaitingResponse,
            true,
        ));
        assert!(!shader_presentation_can_yield_to_eof(
            PendingShaderApplyPhase::PresentedConfirmed,
            true,
        ));
        assert!(!shader_presentation_can_yield_to_eof(
            PendingShaderApplyPhase::AwaitingPresentation,
            false,
        ));
    }

    #[test]
    fn cycle_boundary_reset_requires_a_cycle_capable_physical_session() {
        let config = VideoCycleConfig::try_new(
            true,
            5_000,
            8_000,
            crate::media_effect_params::MediaEffectParams::default(),
        )
        .expect("valid cycle config");
        let mut runtime = RuntimeState {
            cycle_controller: Some(VideoCycleController::new(config)),
            ..RuntimeState::default()
        };
        runtime.status.activation = BackendActivation::Active;
        runtime.status.lifecycle = RendererLifecycleState::Active;
        runtime.status.apply_state = VideoApplyState::Active;
        runtime.process_launch_mode = Some(MpvLaunchMode::Original);

        runtime.begin_source_transition();

        assert_eq!(runtime.status.activation, BackendActivation::Active);
        assert_eq!(runtime.status.lifecycle, RendererLifecycleState::Active);
        assert_eq!(runtime.status.apply_state, VideoApplyState::Idle);
        assert!(runtime.cycle_controller.is_none());

        let source = include_str!("realtime_video_runtime.rs");
        let transition = source
            .split("    fn begin_source_transition(&mut self)")
            .nth(1)
            .expect("source transition")
            .split("    fn handle(")
            .next()
            .expect("source transition end");
        assert!(transition.contains("cycle_session_is_available("));
        assert!(transition.contains("self.mark_cycle_status_available()"));
        assert!(transition.contains("self.set_apply_state(VideoApplyState::SourceTransitioning)"));
        assert!(transition.contains("self.cycle_controller = None"));
        assert!(transition.contains("self.set_apply_state(VideoApplyState::Idle)"));
        let reset = source
            .split("    fn reset_cycle_controller_for_boundary(")
            .nth(1)
            .expect("boundary reset")
            .split("    fn advance_after_eof(")
            .next()
            .expect("boundary reset end");
        assert!(reset.contains("self.begin_source_transition()"));
        let advance = source
            .split("    fn advance_after_eof(")
            .nth(1)
            .expect("EOF advance")
            .split("    fn validate_advance_after_eof_identity(")
            .next()
            .expect("EOF advance end");
        assert!(advance.contains("self.begin_source_transition()"));
    }

    #[test]
    fn in_flight_shader_apply_is_guarded_before_pending_plan_replacement() {
        let source = include_str!("realtime_video_runtime.rs");
        let commit = source
            .split("    fn commit_pending_if_due(")
            .nth(1)
            .expect("cycle commit function")
            .split("    fn poll_pending_shader_apply(")
            .next()
            .expect("cycle commit function end");
        let in_flight = commit
            .find("let shader_apply_in_flight = self.pending_shader_apply.is_some()")
            .expect("in-flight snapshot");
        let guard = commit
            .find("if shader_apply_in_flight")
            .expect("in-flight guard");
        let replacement = commit
            .find("self.pending = Some(plan.clone())")
            .expect("pending plan replacement");
        assert!(in_flight < guard && guard < replacement);
    }

    #[test]
    fn reconfiguration_invalidates_an_uncancellable_shader_session_before_transition() {
        let source = include_str!("realtime_video_runtime.rs");
        let arm = source
            .split("RuntimeCommand::ConfigureCycle {")
            .nth(2)
            .expect("configure actor arm")
            .split("RuntimeCommand::Commit {")
            .next()
            .expect("configure actor body");
        let pending = arm
            .find("if self.pending_shader_apply.is_some()")
            .expect("in-flight check");
        let stop = arm
            .find("self.stop_process()")
            .expect("session invalidation");
        let transition = arm
            .find("self.invalidate_cycle_before_source_recovery()")
            .expect("source transition");
        assert!(pending < stop && stop < transition);
    }

    #[test]
    fn successful_neutralization_discards_the_superseded_shader_transaction() {
        let source = include_str!("realtime_video_runtime.rs");
        let neutralize = source
            .split("    fn neutralize_to_source(")
            .nth(1)
            .expect("neutralization function")
            .split("    fn apply_neutral_process_parameters(")
            .next()
            .expect("neutralization function end");
        let physical_neutral = neutralize
            .find("self.apply_neutral_process_parameters()?")
            .expect("physical neutral command");
        let discard_transaction = neutralize
            .find("self.pending_shader_apply = None")
            .expect("transaction discard");
        let discard_plan = neutralize
            .find("self.pending = None")
            .expect("plan discard");
        assert!(physical_neutral < discard_transaction && discard_transaction < discard_plan);
    }

    #[test]
    fn fatal_render_evidence_is_checked_before_shader_presentation_promotion() {
        let source = include_str!("realtime_video_runtime.rs");
        let tick = source
            .split("    fn tick(&mut self)")
            .nth(1)
            .expect("runtime tick")
            .split("        let budget = ObservationTickBudget::start()")
            .next()
            .expect("tick prelude");
        let fatal = tick
            .find("ManagedMpvProcess::fatal_render_failure")
            .expect("fatal render check");
        let promotion = tick
            .find("self.poll_pending_shader_apply()")
            .expect("shader apply poll");
        assert!(fatal < promotion);
    }

    #[test]
    fn unresolved_shader_response_blocks_followup_sync_ipc_in_the_same_tick() {
        let source = include_str!("realtime_video_runtime.rs");
        let tick = source
            .split("    fn tick(&mut self)")
            .nth(1)
            .expect("runtime tick")
            .split("        let budget = ObservationTickBudget::start()")
            .next()
            .expect("tick IPC prelude");
        let phase = tick
            .find("let pending_shader_phase")
            .expect("shader phase snapshot");
        let poll = tick
            .find("self.poll_pending_shader_apply()")
            .expect("shader apply poll");
        let awaiting_response = tick
            .find("PendingShaderApplyPhase::AwaitingResponse")
            .expect("awaiting response guard");
        let stop_tick = tick[awaiting_response..]
            .find("return false")
            .map(|offset| awaiting_response + offset)
            .expect("no synchronous observation while shader owns the worker");

        assert!(phase < poll && poll < awaiting_response && awaiting_response < stop_tick);
    }

    #[test]
    fn shader_hard_timeout_allows_bounded_combined_media_load() {
        assert_eq!(SHADER_APPLY_SOFT_TIMEOUT, Duration::from_millis(500));
        assert!(SHADER_APPLY_HARD_TIMEOUT >= Duration::from_secs(8));
        assert!(SHADER_APPLY_HARD_TIMEOUT > SHADER_APPLY_SOFT_TIMEOUT);
    }
}
