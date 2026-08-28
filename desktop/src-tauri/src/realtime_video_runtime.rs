use crate::realtime_video_backend::{
    BackendDemotion, ManagedMpvProcess, MpvCommand, MpvGraphicsApi, MpvIpcOptions, MpvLaunchSpec,
    ParameterSupportReport, RealtimeVideoBackendError, RealtimeVideoPlan, RendererLifecycleState,
    VerifiedMpvExecutable, VerifiedMpvShader, VideoBackend, VideoBackendStateMachine,
    VideoCommitDecision, VideoCommitGate, VideoPlanIdentity, VideoPlanSlot,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const MPV_PIPE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MPV_PIPE_CONNECT_POLL: Duration = Duration::from_millis(20);
const MPV_COMMAND_TIMEOUT: Duration = Duration::from_millis(500);

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycleSlotStatus {
    pub sequence: u64,
    pub target_pts_ms: u64,
    pub status: CycleSlotState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaVideoBackendRuntimeStatus {
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
    pub cycle_drift_ms: Option<i64>,
    pub demotion_reason: Option<String>,
    pub support_completeness: SupportCompleteness,
    pub process_id: Option<u32>,
    pub parameter_support: ParameterSupportReport,
    pub ignored_active_parameter_count: usize,
    pub ignored_active_parameter_examples: Vec<String>,
    pub last_demotion: Option<BackendDemotion>,
}

impl Default for MediaVideoBackendRuntimeStatus {
    fn default() -> Self {
        Self {
            backend: VideoBackend::Source,
            activation: BackendActivation::Active,
            lifecycle: RendererLifecycleState::Stopped,
            gpu_adapter: None,
            graphics_api: None,
            decoder: None,
            filter: None,
            n: None,
            n1: None,
            n2: None,
            cycle_drift_ms: None,
            demotion_reason: None,
            support_completeness: SupportCompleteness::NotApplicable,
            process_id: None,
            parameter_support: ParameterSupportReport::empty(VideoBackend::Source),
            ignored_active_parameter_count: 0,
            ignored_active_parameter_examples: Vec::new(),
            last_demotion: None,
        }
    }
}

#[derive(Debug)]
pub struct PrepareRealtimeRenderer<'a> {
    pub executable: VerifiedMpvExecutable,
    pub shader: VerifiedMpvShader,
    pub source_path: &'a Path,
    pub host_window_id: u64,
    pub source_start_ms: u64,
    pub paused: bool,
    pub plan: RealtimeVideoPlan,
    pub n2: Option<CycleSlotStatus>,
    pub session_id: u64,
}

#[derive(Debug, Default)]
pub struct RealtimeVideoRuntime {
    process: Option<ManagedMpvProcess>,
    source_path: Option<PathBuf>,
    backend: VideoBackendStateMachine,
    pending: Option<RealtimeVideoPlan>,
    current: Option<RealtimeVideoPlan>,
    status: MediaVideoBackendRuntimeStatus,
}

impl RealtimeVideoRuntime {
    pub fn status(&self) -> MediaVideoBackendRuntimeStatus {
        self.status.clone()
    }

    pub fn prepare(
        &mut self,
        request: PrepareRealtimeRenderer<'_>,
        now_unix_ms: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        if request.plan.slot != VideoPlanSlot::NPlus1 {
            return Err(RealtimeVideoBackendError::ProcessFailed {
                operation: "计划准备",
                message: "实时画面只允许准备 N+1".to_owned(),
            });
        }
        if self.backend.current() != VideoBackend::RealtimeGpu {
            return Ok(self.status());
        }

        let canonical_source = request.source_path.canonicalize().map_err(|error| {
            RealtimeVideoBackendError::InvalidMediaPath {
                path: request.source_path.to_path_buf(),
                message: error.to_string(),
            }
        })?;
        self.current = self
            .current
            .take()
            .filter(|plan| same_playback_identity(&plan.identity, &request.plan.identity));
        let must_restart = self.process.is_none()
            || self.status.backend != VideoBackend::RealtimeGpu
            || self.source_path.as_ref() != Some(&canonical_source)
            || self
                .process
                .as_mut()
                .is_some_and(|process| process.has_exited().unwrap_or(true));
        if must_restart {
            self.stop_process();
            self.start_renderer(&request, &canonical_source, now_unix_ms)?;
        }
        self.pending = Some(request.plan.clone());
        self.claim_realtime_status();
        self.status.activation = BackendActivation::Available;
        self.status.lifecycle = RendererLifecycleState::Spawned;
        self.status.n = self
            .current
            .as_ref()
            .map(|plan| slot_status(plan, CycleSlotState::Active));
        self.status.n1 = Some(slot_status(&request.plan, CycleSlotState::Ready));
        self.status.n2 = request.n2;
        self.status.process_id = self.process.as_ref().and_then(ManagedMpvProcess::pid);
        self.record_parameter_support(request.plan.parameter_support);
        Ok(self.status())
    }

    pub fn commit(
        &mut self,
        gate: &VideoCommitGate,
        now_unix_ms: u64,
    ) -> Result<MediaVideoBackendRuntimeStatus, RealtimeVideoBackendError> {
        let Some(plan) = self.pending.clone() else {
            return Ok(self.status());
        };
        match plan.commit_decision(gate) {
            VideoCommitDecision::Commit => {}
            decision => {
                return Err(RealtimeVideoBackendError::ProcessFailed {
                    operation: "计划提交",
                    message: format!("实时 N+1 提交门禁拒绝：{decision:?}"),
                })
            }
        }
        let drift = signed_delta(gate.media_pts_ms, plan.target_pts_ms);
        let process =
            self.process
                .as_ref()
                .ok_or_else(|| RealtimeVideoBackendError::ProcessFailed {
                    operation: "参数提交",
                    message: "mpv 会话尚未连接".to_owned(),
                })?;
        for command in &plan.commands {
            if let Err(error) = process.send_command(command, MPV_COMMAND_TIMEOUT) {
                self.demote(error.to_string(), now_unix_ms, plan.parameter_support);
                self.stop_process();
                return Ok(self.status());
            }
        }
        self.current = Some(RealtimeVideoPlan {
            slot: VideoPlanSlot::N,
            ..plan.clone()
        });
        self.pending = None;
        self.claim_realtime_status();
        self.status.activation = BackendActivation::Active;
        self.status.lifecycle = RendererLifecycleState::Active;
        self.status.n = Some(slot_status(&plan, CycleSlotState::Active));
        self.status.n1 = None;
        self.status.cycle_drift_ms = Some(drift);
        self.status.process_id = self.process.as_ref().and_then(ManagedMpvProcess::pid);
        self.record_parameter_support(plan.parameter_support);
        Ok(self.status())
    }

    pub fn synchronize(
        &mut self,
        _position_ms: u64,
        paused: bool,
        now_unix_ms: u64,
    ) -> Result<(), RealtimeVideoBackendError> {
        let Some(process) = self.process.as_ref() else {
            return Ok(());
        };
        // 普通播放/暂停同步绝不执行 exact seek；硬定位只由 seek、换源、循环或恢复边界触发。
        if let Err(error) =
            process.send_command(&MpvCommand::SetPause { paused }, MPV_COMMAND_TIMEOUT)
        {
            self.demote(
                error.to_string(),
                now_unix_ms,
                self.status.parameter_support.clone(),
            );
            self.stop_process();
            return Ok(());
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        self.stop_process();
        self.pending = None;
        self.current = None;
        self.backend = VideoBackendStateMachine::new();
        self.status = MediaVideoBackendRuntimeStatus::default();
    }

    pub fn record_source_backend(&mut self, reason: impl Into<String>) {
        self.stop_process();
        self.pending = None;
        self.current = None;
        self.status = MediaVideoBackendRuntimeStatus::default();
        self.status.demotion_reason = Some(reason.into());
    }

    fn start_renderer(
        &mut self,
        request: &PrepareRealtimeRenderer<'_>,
        source: &Path,
        now_unix_ms: u64,
    ) -> Result<(), RealtimeVideoBackendError> {
        let mut last_error = None;
        for graphics_api in MpvGraphicsApi::attempt_order() {
            let pipe = format!(
                r"\\.\pipe\autolive-mpv-{}-{}",
                std::process::id(),
                request.session_id
            );
            let spec = match MpvLaunchSpec::new_with_shader(
                request.executable.clone(),
                source,
                request.host_window_id,
                &pipe,
                graphics_api,
                request.source_start_ms,
                request.paused,
                request.shader.clone(),
            ) {
                Ok(spec) => spec,
                Err(error) => {
                    last_error = Some(error.to_string());
                    break;
                }
            };
            let mut process =
                match ManagedMpvProcess::spawn_connected(&spec, MpvIpcOptions::default()) {
                    Ok(process) => process,
                    Err(error) => {
                        last_error = Some(error.to_string());
                        continue;
                    }
                };
            match wait_for_video_output(&mut process) {
                Ok(()) => {
                    self.record_renderer_started(graphics_api);
                    self.process = Some(process);
                    self.source_path = Some(source.to_path_buf());
                    return Ok(());
                }
                Err(error) => {
                    last_error = Some(error.to_string());
                    let _ignored = process.cancel();
                }
            }
        }
        let reason = last_error.unwrap_or_else(|| "mpv GPU 输出初始化失败".to_owned());
        self.demote(
            reason.clone(),
            now_unix_ms,
            request.plan.parameter_support.clone(),
        );
        Err(RealtimeVideoBackendError::ProcessFailed {
            operation: "GPU 初始化",
            message: reason,
        })
    }

    fn demote(
        &mut self,
        reason: impl Into<String>,
        now_unix_ms: u64,
        support: ParameterSupportReport,
    ) {
        let reason = reason.into();
        let next = self.backend.demote(reason.clone(), now_unix_ms);
        self.status.backend = next;
        self.status.activation = BackendActivation::Configured;
        self.status.lifecycle = RendererLifecycleState::Failed;
        self.status.demotion_reason = Some(reason);
        self.status.gpu_adapter = None;
        self.status.graphics_api = (next == VideoBackend::Cpu4).then(|| "software".to_owned());
        self.status.decoder = (next == VideoBackend::Cpu4).then(|| "software".to_owned());
        self.status.filter =
            (next == VideoBackend::Cpu4).then(|| "libavfilter eq+hue（待启动）".to_owned());
        self.record_parameter_support(if next == VideoBackend::Cpu4 {
            ParameterSupportReport {
                backend: VideoBackend::Cpu4,
                fully_supported: false,
                parameters: Vec::new(),
            }
        } else {
            support
        });
        self.status.last_demotion = self.backend.last_demotion().cloned();
        self.status.process_id = None;
    }

    fn record_parameter_support(&mut self, support: ParameterSupportReport) {
        self.status.ignored_active_parameter_count = support.ignored_active_parameter_count();
        self.status.ignored_active_parameter_examples =
            support.ignored_active_parameter_examples(3);
        self.status.support_completeness = if support.fully_supported {
            SupportCompleteness::Complete
        } else {
            SupportCompleteness::Incomplete
        };
        self.status.parameter_support = support;
    }

    fn claim_realtime_status(&mut self) {
        self.status.backend = VideoBackend::RealtimeGpu;
        self.status.gpu_adapter = None;
        self.status.demotion_reason = None;
        self.status.last_demotion = None;
        self.status.cycle_drift_ms = None;
    }

    fn record_renderer_started(&mut self, graphics_api: MpvGraphicsApi) {
        self.status.graphics_api = Some(
            match graphics_api {
                MpvGraphicsApi::D3d11 => "d3d11",
                MpvGraphicsApi::Vulkan => "vulkan",
            }
            .to_owned(),
        );
        self.status.decoder = Some(
            match graphics_api {
                MpvGraphicsApi::D3d11 => "d3d11va",
                MpvGraphicsApi::Vulkan => "d3d11va-copy",
            }
            .to_owned(),
        );
        self.status.filter = Some("gpu-next/libplacebo".to_owned());
        self.claim_realtime_status();
    }

    fn stop_process(&mut self) {
        if let Some(mut process) = self.process.take() {
            let _ignored = process.cancel();
        }
        self.source_path = None;
        self.status.process_id = None;
    }
}

impl Drop for RealtimeVideoRuntime {
    fn drop(&mut self) {
        self.stop_process();
    }
}

fn slot_status(plan: &RealtimeVideoPlan, status: CycleSlotState) -> CycleSlotStatus {
    CycleSlotStatus {
        sequence: plan.identity.sequence,
        target_pts_ms: plan.target_pts_ms,
        status,
    }
}

fn same_playback_identity(left: &VideoPlanIdentity, right: &VideoPlanIdentity) -> bool {
    left.session_id == right.session_id
        && left.playback_generation == right.playback_generation
        && left.source_revision == right.source_revision
}

fn signed_delta(left: u64, right: u64) -> i64 {
    if left >= right {
        i64::try_from(left - right).unwrap_or(i64::MAX)
    } else {
        -i64::try_from(right - left).unwrap_or(i64::MAX)
    }
}

fn wait_for_video_output(process: &mut ManagedMpvProcess) -> Result<(), RealtimeVideoBackendError> {
    let started = Instant::now();
    loop {
        if process.has_exited()? {
            return Err(RealtimeVideoBackendError::ProcessFailed {
                operation: "GPU 初始化",
                message: "mpv 在视频输出确认前退出".to_owned(),
            });
        }
        match process.send_command(&MpvCommand::GetVideoOutputConfigured, MPV_COMMAND_TIMEOUT) {
            Ok(response)
                if response.get("data").and_then(serde_json::Value::as_bool) == Some(true) =>
            {
                return Ok(())
            }
            Ok(_) | Err(_) if started.elapsed() < MPV_PIPE_CONNECT_TIMEOUT => {
                thread::sleep(MPV_PIPE_CONNECT_POLL);
            }
            Ok(response) => {
                return Err(RealtimeVideoBackendError::ProcessFailed {
                    operation: "GPU 初始化",
                    message: format!("mpv 视频输出未就绪：{response}"),
                })
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::realtime_video_backend::{ParameterSupportReport, VideoPlanIdentity};

    #[test]
    fn source_status_never_claims_effects_are_active() {
        let status = MediaVideoBackendRuntimeStatus::default();
        assert_eq!(status.backend, VideoBackend::Source);
        assert_eq!(status.activation, BackendActivation::Active);
        assert_eq!(
            status.support_completeness,
            SupportCompleteness::NotApplicable
        );
    }

    #[test]
    fn unsupported_parameters_are_observable_without_demoting_realtime_gpu() {
        let mut runtime = RealtimeVideoRuntime::default();
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

        let result = runtime.status();
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
                media_pts_ms: 8_000,
            }),
            VideoCommitDecision::Commit
        );
    }

    #[test]
    fn realtime_renderer_start_clears_stale_cpu4_diagnostics() {
        let mut runtime = RealtimeVideoRuntime::default();
        runtime.status.backend = VideoBackend::Cpu4;
        runtime.status.graphics_api = Some("software".to_owned());
        runtime.status.decoder = Some("software".to_owned());
        runtime.status.filter = Some("libavfilter eq+hue".to_owned());
        runtime.status.demotion_reason = Some("旧降级原因".to_owned());
        runtime.status.last_demotion = Some(BackendDemotion {
            from: VideoBackend::RealtimeGpu,
            to: VideoBackend::Cpu4,
            reason: "旧降级原因".to_owned(),
            at_unix_ms: 1,
        });
        runtime.status.cycle_drift_ms = Some(34_827);

        runtime.record_renderer_started(MpvGraphicsApi::D3d11);
        let status = runtime.status();

        assert_eq!(status.backend, VideoBackend::RealtimeGpu);
        assert_eq!(status.gpu_adapter, None);
        assert_eq!(status.graphics_api.as_deref(), Some("d3d11"));
        assert_eq!(status.decoder.as_deref(), Some("d3d11va"));
        assert_eq!(status.filter.as_deref(), Some("gpu-next/libplacebo"));
        assert_eq!(status.demotion_reason, None);
        assert_eq!(status.last_demotion, None);
        assert_eq!(status.cycle_drift_ms, None);
    }
}
