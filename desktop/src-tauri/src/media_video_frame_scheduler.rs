//! 基于媒体 PTS 的确定性视频帧调度决策。
//!
//! 本模块不读墙钟、不执行 mpv IPC，也不持有任何媒体资源。调用方负责在启动、
//! 换源、seek 和循环边界递增 `schedule_epoch` 并提交对应边界。

use std::f64::consts::TAU;
use std::fmt;

use crate::media_effect_params::{
    AdvancedEffectParams, ParameterValidationError, VideoEffectParams,
};
use crate::realtime_video_backend::VideoPlanIdentity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoScheduleBoundary {
    None,
    Startup,
    SourceChanged,
    UserSeek,
    LoopBoundary,
}

impl VideoScheduleBoundary {
    fn is_reset(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoScheduleIgnoreReason {
    StaleGeneration,
    StaleEpoch,
    StaleSequence,
    OutOfOrderPts,
    EpochWithoutBoundary,
    PausedPtsAdvanced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoScheduleAction {
    Apply,
    Hold,
    Ignore { reason: VideoScheduleIgnoreReason },
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoFrameSchedule {
    pub identity: VideoPlanIdentity,
    pub schedule_epoch: u64,
    pub media_pts_ms: u64,
    pub epoch_elapsed_ms: u64,
    pub frame_index: u64,
    pub target_fps: f64,
    pub base_video_speed: f64,
    pub frame_rate_locked: bool,
    pub frame_inner_active: bool,
    pub frame_inter_active: bool,
    pub frame_probability_active: bool,
    pub slice_active: bool,
    pub random_graphic_seed: u32,
    pub local_blur_active: bool,
    pub highlight_active: bool,
    pub pip_jitter_x_px: f64,
    pub pip_jitter_y_px: f64,
    pub asynchronous_rotation_degrees: f64,
    pub transform_easing: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoFrameTimingSchedule {
    /// 当前触发窗口序号；仅描述计划边界，不代表 shader 已执行源片段选择。
    pub slice_window_index: u64,
    /// 当前触发窗口在源媒体时间轴上的起点。
    pub slice_window_start_pts_ms: u64,
    /// 当前可见叠加窗口在源媒体时间轴上的结束点。
    pub slice_window_end_pts_ms: u64,
    /// 下游选择源片段时必须满足的最小时长。
    pub slice_source_min_length_ms: u64,
    /// 画中画是否与主画面使用同一 PTS；这里只输出调度语义，不创建第二播放器。
    pub pip_timeline_locked: bool,
    pub pip_timeline_offset_ms: u64,
    pub pip_timeline_pts_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoFrameScheduleDecision {
    pub action: VideoScheduleAction,
    pub schedule: Option<VideoFrameSchedule>,
    pub timing: Option<VideoFrameTimingSchedule>,
}

impl VideoFrameScheduleDecision {
    fn ignore(reason: VideoScheduleIgnoreReason) -> Self {
        Self {
            action: VideoScheduleAction::Ignore { reason },
            schedule: None,
            timing: None,
        }
    }

    fn hold(schedule: VideoFrameSchedule, timing: VideoFrameTimingSchedule) -> Self {
        Self {
            action: VideoScheduleAction::Hold,
            schedule: Some(schedule),
            timing: Some(timing),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VideoFrameScheduleObservation<'a> {
    pub identity: VideoPlanIdentity,
    pub schedule_epoch: u64,
    pub media_pts_ms: u64,
    pub source_fps: f64,
    pub paused: bool,
    pub boundary: VideoScheduleBoundary,
    pub seed: u64,
    pub video_params: &'a VideoEffectParams,
    pub advanced_params: &'a AdvancedEffectParams,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VideoFrameScheduleError {
    InvalidSourceFps { source_fps: f64 },
    InvalidParams(Vec<ParameterValidationError>),
}

impl fmt::Display for VideoFrameScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSourceFps { source_fps } => {
                write!(
                    formatter,
                    "source_fps 必须是 1–240 的有限值，实际为 {source_fps}"
                )
            }
            Self::InvalidParams(errors) => {
                write!(formatter, "媒体参数校验失败（{} 项）", errors.len())
            }
        }
    }
}

impl std::error::Error for VideoFrameScheduleError {}

#[derive(Debug, Clone)]
struct SchedulerState {
    identity: VideoPlanIdentity,
    schedule_epoch: u64,
    epoch_anchor_pts_ms: u64,
    last_media_pts_ms: u64,
    paused: bool,
    schedule: VideoFrameSchedule,
    timing: VideoFrameTimingSchedule,
}

#[derive(Debug, Default)]
pub struct VideoFrameScheduler {
    state: Option<SchedulerState>,
}

impl VideoFrameScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn decide(
        &mut self,
        observation: VideoFrameScheduleObservation<'_>,
    ) -> Result<VideoFrameScheduleDecision, VideoFrameScheduleError> {
        validate_observation(&observation)?;

        let Some(current) = self.state.as_ref() else {
            if !observation.boundary.is_reset() {
                return Ok(VideoFrameScheduleDecision::ignore(
                    VideoScheduleIgnoreReason::EpochWithoutBoundary,
                ));
            }
            return Ok(self.apply(&observation, observation.media_pts_ms));
        };

        if observation.identity.session_id != current.identity.session_id
            || observation.identity.playback_generation < current.identity.playback_generation
        {
            return Ok(VideoFrameScheduleDecision::ignore(
                VideoScheduleIgnoreReason::StaleGeneration,
            ));
        }

        let generation_advanced =
            observation.identity.playback_generation > current.identity.playback_generation;
        let epoch_advanced =
            generation_advanced || observation.schedule_epoch > current.schedule_epoch;
        if !generation_advanced && observation.schedule_epoch < current.schedule_epoch {
            return Ok(VideoFrameScheduleDecision::ignore(
                VideoScheduleIgnoreReason::StaleEpoch,
            ));
        }
        if !epoch_advanced && observation.identity.sequence < current.identity.sequence {
            return Ok(VideoFrameScheduleDecision::ignore(
                VideoScheduleIgnoreReason::StaleSequence,
            ));
        }

        let source_changed =
            observation.identity.source_revision != current.identity.source_revision;
        if generation_advanced && !observation.boundary.is_reset()
            || !generation_advanced
                && source_changed
                && (!epoch_advanced
                    || !matches!(observation.boundary, VideoScheduleBoundary::SourceChanged))
            || epoch_advanced && !observation.boundary.is_reset()
        {
            return Ok(VideoFrameScheduleDecision::ignore(
                VideoScheduleIgnoreReason::EpochWithoutBoundary,
            ));
        }

        if epoch_advanced {
            return Ok(self.apply(&observation, observation.media_pts_ms));
        }

        if observation.identity.sequence > current.identity.sequence {
            // TS 解复用和呈现线程可能在周期边界给出一次很小的源内 PTS 回退。
            // 新 sequence 是已经到期的参数事务，不能因此丢掉完整静态快照；同时
            // 调度时间保持单调，避免把回退值写成后续观察的高水位。
            let epoch_anchor_pts_ms = current.epoch_anchor_pts_ms;
            let mut monotonic_observation = observation.clone();
            monotonic_observation.media_pts_ms =
                observation.media_pts_ms.max(current.last_media_pts_ms);
            return Ok(self.apply(&monotonic_observation, epoch_anchor_pts_ms));
        }
        if observation.media_pts_ms < current.last_media_pts_ms {
            return Ok(VideoFrameScheduleDecision::ignore(
                VideoScheduleIgnoreReason::OutOfOrderPts,
            ));
        }
        if current.paused
            && observation.paused
            && observation.media_pts_ms > current.last_media_pts_ms
        {
            return Ok(VideoFrameScheduleDecision::ignore(
                VideoScheduleIgnoreReason::PausedPtsAdvanced,
            ));
        }
        if observation.media_pts_ms == current.last_media_pts_ms {
            let schedule = current.schedule.clone();
            let timing = current.timing.clone();
            if let Some(state) = self.state.as_mut() {
                state.paused = observation.paused;
            }
            return Ok(VideoFrameScheduleDecision::hold(schedule, timing));
        }

        let epoch_anchor_pts_ms = current.epoch_anchor_pts_ms;
        Ok(self.hold_updated(&observation, epoch_anchor_pts_ms))
    }

    fn apply(
        &mut self,
        observation: &VideoFrameScheduleObservation<'_>,
        epoch_anchor_pts_ms: u64,
    ) -> VideoFrameScheduleDecision {
        let (schedule, timing) = build_schedule(observation, epoch_anchor_pts_ms);
        self.state = Some(SchedulerState {
            identity: observation.identity.clone(),
            schedule_epoch: observation.schedule_epoch,
            epoch_anchor_pts_ms,
            last_media_pts_ms: observation.media_pts_ms,
            paused: observation.paused,
            schedule: schedule.clone(),
            timing: timing.clone(),
        });
        VideoFrameScheduleDecision {
            action: VideoScheduleAction::Apply,
            schedule: Some(schedule),
            timing: Some(timing),
        }
    }

    fn hold_updated(
        &mut self,
        observation: &VideoFrameScheduleObservation<'_>,
        epoch_anchor_pts_ms: u64,
    ) -> VideoFrameScheduleDecision {
        let (schedule, timing) = build_schedule(observation, epoch_anchor_pts_ms);
        self.state = Some(SchedulerState {
            identity: observation.identity.clone(),
            schedule_epoch: observation.schedule_epoch,
            epoch_anchor_pts_ms,
            last_media_pts_ms: observation.media_pts_ms,
            paused: observation.paused,
            schedule: schedule.clone(),
            timing: timing.clone(),
        });
        VideoFrameScheduleDecision::hold(schedule, timing)
    }
}

fn validate_observation(
    observation: &VideoFrameScheduleObservation<'_>,
) -> Result<(), VideoFrameScheduleError> {
    if !observation.source_fps.is_finite() || !(1.0..=240.0).contains(&observation.source_fps) {
        return Err(VideoFrameScheduleError::InvalidSourceFps {
            source_fps: observation.source_fps,
        });
    }
    let mut errors = Vec::new();
    if let Err(mut video_errors) = observation.video_params.validate() {
        errors.append(&mut video_errors);
    }
    if let Err(mut advanced_errors) = observation.advanced_params.validate() {
        errors.append(&mut advanced_errors);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(VideoFrameScheduleError::InvalidParams(errors))
    }
}

fn build_schedule(
    observation: &VideoFrameScheduleObservation<'_>,
    epoch_anchor_pts_ms: u64,
) -> (VideoFrameSchedule, VideoFrameTimingSchedule) {
    let video = observation.video_params;
    let advanced = observation.advanced_params;
    let epoch_elapsed_ms = observation.media_pts_ms.saturating_sub(epoch_anchor_pts_ms);
    let frame_index = ((epoch_elapsed_ms as f64 * observation.source_fps) / 1_000.0).floor() as u64;

    let (target_fps, base_video_speed) = if video.frame_rate_lock_enabled {
        (observation.source_fps, 1.0)
    } else {
        let elapsed_seconds = epoch_elapsed_ms as f64 / 1_000.0;
        let fps_amplitude = observation.source_fps * video.frame_rate_jitter_percent / 100.0
            + video.frame_rate_perturbation_amplitude_fps;
        let requested_target_fps = observation.source_fps
            + fps_amplitude
                * (TAU * video.frame_rate_perturbation_frequency_hz * elapsed_seconds).sin();
        let base_video_speed = (requested_target_fps / observation.source_fps).clamp(0.5, 1.5);
        (observation.source_fps * base_video_speed, base_video_speed)
    };

    let slice_phase_ms = epoch_elapsed_ms % advanced.slice_trigger_interval_ms;
    let slice_window_index = epoch_elapsed_ms / advanced.slice_trigger_interval_ms;
    let slice_window_start_pts_ms = epoch_anchor_pts_ms
        .saturating_add(slice_window_index.saturating_mul(advanced.slice_trigger_interval_ms));
    let slice_window_end_pts_ms =
        slice_window_start_pts_ms.saturating_add(advanced.slice_length_ms);
    let slice_active = slice_phase_ms < advanced.slice_length_ms;
    let local_blur_pulse_ms = advanced
        .slice_length_ms
        .min(advanced.local_blur_interval_ms / 2)
        .max(100);
    let local_blur_active = advanced.local_blur_enabled
        && epoch_elapsed_ms % advanced.local_blur_interval_ms < local_blur_pulse_ms;
    let highlight_active = advanced.highlight_perturbation_enabled
        && epoch_elapsed_ms % advanced.highlight_perturbation_interval_ms < 50;

    let pip_timeline_offset_ms =
        if advanced.picture_in_picture_enabled && !advanced.picture_in_picture_timeline_locked {
            (random_unit(
                &observation.identity,
                observation.seed,
                FIELD_PIP_TIMELINE,
                0,
            ) * advanced.slice_min_length_ms as f64)
                .floor() as u64
        } else {
            0
        };
    let pip_timeline_pts_ms = observation
        .media_pts_ms
        .saturating_sub(pip_timeline_offset_ms);

    let jitter_amplitude = if advanced.picture_in_picture_enabled {
        advanced.picture_in_picture_pixel_jitter_px
    } else {
        0.0
    };
    let jitter_angle = random_unit(
        &observation.identity,
        observation.seed,
        FIELD_PIP_JITTER,
        frame_index,
    ) * TAU;

    let asynchronous_rotation_degrees = if advanced.asynchronous_rotation_enabled {
        let minimum = advanced.asynchronous_rotation_min_degrees;
        let maximum = advanced.asynchronous_rotation_max_degrees;
        let midpoint = (minimum + maximum) / 2.0;
        let amplitude = (maximum - minimum) / 2.0;
        let rotation_phase = epoch_elapsed_ms % advanced.slice_trigger_interval_ms;
        midpoint
            + amplitude
                * (TAU * rotation_phase as f64 / advanced.slice_trigger_interval_ms as f64).sin()
    } else {
        0.0
    };

    let schedule = VideoFrameSchedule {
        identity: observation.identity.clone(),
        schedule_epoch: observation.schedule_epoch,
        media_pts_ms: observation.media_pts_ms,
        epoch_elapsed_ms,
        frame_index,
        target_fps,
        base_video_speed,
        frame_rate_locked: video.frame_rate_lock_enabled,
        frame_inner_active: probability_gate(
            &observation.identity,
            observation.seed,
            FIELD_FRAME_INNER,
            frame_index,
            video.frame_inner_perturbation_percent,
        ),
        frame_inter_active: probability_gate(
            &observation.identity,
            observation.seed,
            FIELD_FRAME_INTER,
            frame_index,
            video.frame_inter_perturbation_percent,
        ),
        frame_probability_active: probability_gate(
            &observation.identity,
            observation.seed,
            FIELD_FRAME_PROBABILITY,
            frame_index,
            advanced.frame_perturbation_probability_percent,
        ),
        slice_active,
        random_graphic_seed: plan_random_graphic_seed(&observation.identity, observation.seed),
        local_blur_active,
        highlight_active,
        pip_jitter_x_px: jitter_amplitude * jitter_angle.cos(),
        pip_jitter_y_px: jitter_amplitude * jitter_angle.sin(),
        asynchronous_rotation_degrees,
        transform_easing: transform_easing(advanced, slice_phase_ms, slice_active),
    };
    let timing = VideoFrameTimingSchedule {
        slice_window_index,
        slice_window_start_pts_ms,
        slice_window_end_pts_ms,
        slice_source_min_length_ms: advanced.slice_min_length_ms,
        pip_timeline_locked: advanced.picture_in_picture_timeline_locked,
        pip_timeline_offset_ms,
        pip_timeline_pts_ms,
    };
    (schedule, timing)
}

fn transform_easing(
    advanced: &AdvancedEffectParams,
    slice_phase_ms: u64,
    slice_active: bool,
) -> f64 {
    if !slice_active {
        return 0.0;
    }
    if !advanced.transform_smoothing_enabled {
        return 1.0;
    }
    let duration_ms = advanced
        .transform_smoothing_duration_ms
        .min(advanced.slice_length_ms / 2)
        .max(1);
    let distance_to_edge_ms = slice_phase_ms.min(advanced.slice_length_ms - slice_phase_ms);
    smoothstep((distance_to_edge_ms as f64 / duration_ms as f64).clamp(0.0, 1.0))
}

fn smoothstep(value: f64) -> f64 {
    value * value * (3.0 - 2.0 * value)
}

const FIELD_FRAME_INNER: u64 = 0x6672_616d_655f_696e;
const FIELD_FRAME_INTER: u64 = 0x6672_616d_655f_6974;
const FIELD_FRAME_PROBABILITY: u64 = 0x6672_616d_655f_7072;
const FIELD_PIP_TIMELINE: u64 = 0x7069_705f_7469_6d65;
const FIELD_PIP_JITTER: u64 = 0x7069_705f_6a69_7474;
const FIELD_RANDOM_GRAPHIC_SEED: u64 = 0x7261_6e64_5f67_7261;
pub const MAX_RANDOM_GRAPHIC_SEED: u32 = 16_777_215;

fn probability_gate(
    identity: &VideoPlanIdentity,
    caller_seed: u64,
    field_id: u64,
    bucket: u64,
    probability_percent: f64,
) -> bool {
    probability_percent > 0.0
        && random_unit(identity, caller_seed, field_id, bucket) < probability_percent / 100.0
}

fn random_unit(identity: &VideoPlanIdentity, caller_seed: u64, field_id: u64, bucket: u64) -> f64 {
    let state = deterministic_state(identity, caller_seed, field_id, bucket);
    (state >> 11) as f64 * (1.0 / ((1_u64 << 53) as f64))
}

fn plan_random_graphic_seed(identity: &VideoPlanIdentity, caller_seed: u64) -> u32 {
    (deterministic_state(identity, caller_seed, FIELD_RANDOM_GRAPHIC_SEED, 0)
        & u64::from(MAX_RANDOM_GRAPHIC_SEED)) as u32
}

fn deterministic_state(
    identity: &VideoPlanIdentity,
    caller_seed: u64,
    field_id: u64,
    bucket: u64,
) -> u64 {
    let mut state = splitmix64(caller_seed ^ field_id);
    for component in [
        identity.session_id,
        identity.playback_generation,
        identity.sequence,
        field_id,
        bucket,
    ] {
        state = splitmix64(state ^ splitmix64(component));
    }
    state
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_effect_params::MediaEffectParams;

    fn identity(generation: u64, sequence: u64) -> VideoPlanIdentity {
        VideoPlanIdentity {
            session_id: 11,
            playback_generation: generation,
            source_revision: 3,
            parameter_revision: sequence,
            sequence,
        }
    }

    fn observation<'a>(
        identity: &VideoPlanIdentity,
        params: &'a MediaEffectParams,
        epoch: u64,
        pts_ms: u64,
        paused: bool,
        boundary: VideoScheduleBoundary,
    ) -> VideoFrameScheduleObservation<'a> {
        VideoFrameScheduleObservation {
            identity: identity.clone(),
            schedule_epoch: epoch,
            media_pts_ms: pts_ms,
            source_fps: 60.0,
            paused,
            boundary,
            seed: 29,
            video_params: &params.video,
            advanced_params: &params.advanced,
        }
    }

    fn schedule(decision: &VideoFrameScheduleDecision) -> &VideoFrameSchedule {
        decision.schedule.as_ref().expect("schedule should exist")
    }

    fn timing(decision: &VideoFrameScheduleDecision) -> &VideoFrameTimingSchedule {
        decision.timing.as_ref().expect("timing should exist")
    }

    #[test]
    fn validates_input_and_requires_a_first_boundary() {
        let identity = identity(5, 10);
        let params = MediaEffectParams::default();
        let mut scheduler = VideoFrameScheduler::new();
        let mut invalid_fps = observation(
            &identity,
            &params,
            1,
            1_000,
            false,
            VideoScheduleBoundary::Startup,
        );
        invalid_fps.source_fps = f64::NAN;
        assert!(matches!(
            scheduler.decide(invalid_fps),
            Err(VideoFrameScheduleError::InvalidSourceFps { .. })
        ));

        let mut invalid_params = params.clone();
        invalid_params.video.brightness_percent = 101.0;
        assert!(matches!(
            scheduler.decide(observation(
                &identity,
                &invalid_params,
                1,
                1_000,
                false,
                VideoScheduleBoundary::Startup,
            )),
            Err(VideoFrameScheduleError::InvalidParams(_))
        ));

        let decision = scheduler
            .decide(observation(
                &identity,
                &params,
                1,
                1_000,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("valid input");
        assert_eq!(
            decision.action,
            VideoScheduleAction::Ignore {
                reason: VideoScheduleIgnoreReason::EpochWithoutBoundary
            }
        );
        assert!(decision.schedule.is_none());
    }

    #[test]
    fn the_same_inputs_are_stable_across_instances() {
        let identity = identity(5, 10);
        let mut params = MediaEffectParams::default();
        params.video.frame_inner_perturbation_percent = 2.0;
        params.video.frame_inter_perturbation_percent = 20.0;
        params.advanced.frame_perturbation_probability_percent = 20.0;
        params.advanced.picture_in_picture_enabled = true;
        params.advanced.picture_in_picture_pixel_jitter_px = 4.0;
        params.advanced.asynchronous_rotation_enabled = true;

        let mut first = VideoFrameScheduler::new();
        let mut second = VideoFrameScheduler::new();
        let observation = || {
            observation(
                &identity,
                &params,
                7,
                12_345,
                false,
                VideoScheduleBoundary::Startup,
            )
        };
        let first = first.decide(observation()).expect("first decision");
        let second = second.decide(observation()).expect("second decision");
        assert_eq!(first, second);
        assert_eq!(first.action, VideoScheduleAction::Apply);
    }

    #[test]
    fn random_graphic_seed_is_plan_stable_and_exact_in_shader_numbers() {
        let identity = identity(5, 10);
        let params = MediaEffectParams::default();
        let mut scheduler = VideoFrameScheduler::new();
        let first = scheduler
            .decide(observation(
                &identity,
                &params,
                7,
                1_000,
                false,
                VideoScheduleBoundary::Startup,
            ))
            .expect("first frame");
        let later = scheduler
            .decide(observation(
                &identity,
                &params,
                7,
                12_345,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("later frame in the same plan");
        let seed = schedule(&first).random_graphic_seed;

        assert_eq!(seed, 329_672);
        assert_eq!(schedule(&later).random_graphic_seed, seed);
        assert!(seed <= MAX_RANDOM_GRAPHIC_SEED);
        assert_eq!(seed as f64 as u32, seed);
        assert_eq!(seed as f32 as u32, seed);
    }

    #[test]
    fn random_graphic_seed_changes_for_fixed_plan_identity_samples() {
        let base = identity(5, 10);
        let mut changed_session = base.clone();
        changed_session.session_id = 12;

        assert_eq!(
            [
                plan_random_graphic_seed(&base, 29),
                plan_random_graphic_seed(&base, 30),
                plan_random_graphic_seed(&identity(5, 11), 29),
                plan_random_graphic_seed(&identity(6, 10), 29),
                plan_random_graphic_seed(&changed_session, 29),
            ],
            [329_672, 11_101_661, 4_773_634, 8_494_857, 7_495_126]
        );
    }

    #[test]
    fn stale_and_out_of_order_observations_do_not_pollute_state() {
        let params = MediaEffectParams::default();
        let current = identity(5, 10);
        let stale_generation = identity(4, 99);
        let stale_sequence = identity(5, 9);
        let mut scheduler = VideoFrameScheduler::new();
        scheduler
            .decide(observation(
                &current,
                &params,
                2,
                1_000,
                false,
                VideoScheduleBoundary::Startup,
            ))
            .expect("startup");

        for (incoming, epoch, pts, reason) in [
            (
                &stale_generation,
                2,
                1_100,
                VideoScheduleIgnoreReason::StaleGeneration,
            ),
            (&current, 1, 1_100, VideoScheduleIgnoreReason::StaleEpoch),
            (
                &stale_sequence,
                2,
                1_100,
                VideoScheduleIgnoreReason::StaleSequence,
            ),
            (&current, 2, 999, VideoScheduleIgnoreReason::OutOfOrderPts),
        ] {
            let decision = scheduler
                .decide(observation(
                    incoming,
                    &params,
                    epoch,
                    pts,
                    false,
                    VideoScheduleBoundary::None,
                ))
                .expect("ignored observation");
            assert_eq!(decision.action, VideoScheduleAction::Ignore { reason });
            assert!(decision.schedule.is_none());
        }

        let accepted = scheduler
            .decide(observation(
                &current,
                &params,
                2,
                1_100,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("state should remain usable");
        assert_eq!(schedule(&accepted).epoch_elapsed_ms, 100);
        assert_eq!(schedule(&accepted).frame_index, 6);
    }

    #[test]
    fn pause_holds_a_stable_snapshot_and_resume_continues_the_epoch() {
        let identity = identity(5, 10);
        let params = MediaEffectParams::default();
        let mut scheduler = VideoFrameScheduler::new();
        scheduler
            .decide(observation(
                &identity,
                &params,
                2,
                1_000,
                false,
                VideoScheduleBoundary::Startup,
            ))
            .expect("startup");
        let paused = scheduler
            .decide(observation(
                &identity,
                &params,
                2,
                1_100,
                true,
                VideoScheduleBoundary::None,
            ))
            .expect("pause frame");
        assert_eq!(paused.action, VideoScheduleAction::Hold);

        let advanced = scheduler
            .decide(observation(
                &identity,
                &params,
                2,
                1_200,
                true,
                VideoScheduleBoundary::None,
            ))
            .expect("advanced paused PTS is ignored");
        assert_eq!(
            advanced.action,
            VideoScheduleAction::Ignore {
                reason: VideoScheduleIgnoreReason::PausedPtsAdvanced
            }
        );

        let held = scheduler
            .decide(observation(
                &identity,
                &params,
                2,
                1_100,
                true,
                VideoScheduleBoundary::None,
            ))
            .expect("same paused frame");
        assert_eq!(held.action, VideoScheduleAction::Hold);
        assert_eq!(held.schedule, paused.schedule);

        let resumed = scheduler
            .decide(observation(
                &identity,
                &params,
                2,
                1_100,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("resume at same frame");
        assert_eq!(resumed.action, VideoScheduleAction::Hold);
        let next = scheduler
            .decide(observation(
                &identity,
                &params,
                2,
                1_200,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("resume advances");
        assert_eq!(schedule(&next).epoch_elapsed_ms, 200);
    }

    #[test]
    fn epochs_reset_once_and_repeated_boundaries_do_not_move_the_anchor() {
        let identity = identity(5, 10);
        let params = MediaEffectParams::default();
        let mut scheduler = VideoFrameScheduler::new();
        scheduler
            .decide(observation(
                &identity,
                &params,
                1,
                1_000,
                false,
                VideoScheduleBoundary::Startup,
            ))
            .expect("startup");

        let missing_boundary = scheduler
            .decide(observation(
                &identity,
                &params,
                2,
                200,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("new epoch must fail closed");
        assert_eq!(
            missing_boundary.action,
            VideoScheduleAction::Ignore {
                reason: VideoScheduleIgnoreReason::EpochWithoutBoundary
            }
        );

        let seek = scheduler
            .decide(observation(
                &identity,
                &params,
                2,
                200,
                false,
                VideoScheduleBoundary::UserSeek,
            ))
            .expect("seek epoch");
        assert_eq!(schedule(&seek).epoch_elapsed_ms, 0);
        let repeated = scheduler
            .decide(observation(
                &identity,
                &params,
                2,
                300,
                false,
                VideoScheduleBoundary::UserSeek,
            ))
            .expect("repeated boundary");
        assert_eq!(schedule(&repeated).epoch_elapsed_ms, 100);

        let looped = scheduler
            .decide(observation(
                &identity,
                &params,
                3,
                0,
                false,
                VideoScheduleBoundary::LoopBoundary,
            ))
            .expect("loop epoch");
        assert_eq!(schedule(&looped).epoch_elapsed_ms, 0);
    }

    #[test]
    fn loop_epoch_accepts_the_new_segment_sequence_restart() {
        let params = MediaEffectParams::default();
        let mut scheduler = VideoFrameScheduler::new();
        scheduler
            .decide(observation(
                &identity(5, 14),
                &params,
                1,
                70_000,
                false,
                VideoScheduleBoundary::Startup,
            ))
            .expect("first loop");

        let looped = scheduler
            .decide(observation(
                &identity(5, 1),
                &params,
                2,
                0,
                false,
                VideoScheduleBoundary::LoopBoundary,
            ))
            .expect("next loop");

        assert_eq!(looped.action, VideoScheduleAction::Apply);
        assert_eq!(schedule(&looped).epoch_elapsed_ms, 0);
    }

    #[test]
    fn timing_rules_follow_media_pts() {
        let identity = identity(5, 10);
        let mut params = MediaEffectParams::default();
        params.video.frame_rate_jitter_percent = 2.0;
        params.video.frame_rate_perturbation_amplitude_fps = 2.0;
        params.video.frame_rate_perturbation_frequency_hz = 1.0;
        params.video.frame_rate_lock_enabled = false;
        params.advanced.slice_length_ms = 500;
        params.advanced.slice_trigger_interval_ms = 5_000;
        params.advanced.local_blur_enabled = true;
        params.advanced.local_blur_interval_ms = 500;
        params.advanced.highlight_perturbation_enabled = true;
        params.advanced.highlight_perturbation_interval_ms = 500;
        params.advanced.picture_in_picture_enabled = true;
        params.advanced.picture_in_picture_pixel_jitter_px = 4.0;
        params.advanced.asynchronous_rotation_enabled = true;
        params.advanced.asynchronous_rotation_min_degrees = -2.0;
        params.advanced.asynchronous_rotation_max_degrees = 6.0;
        params.advanced.transform_smoothing_enabled = true;
        params.advanced.transform_smoothing_duration_ms = 100;

        let mut scheduler = VideoFrameScheduler::new();
        let start = scheduler
            .decide(observation(
                &identity,
                &params,
                4,
                0,
                false,
                VideoScheduleBoundary::Startup,
            ))
            .expect("start");
        let start = schedule(&start);
        assert!(start.slice_active && start.local_blur_active && start.highlight_active);
        assert_eq!(start.transform_easing, 0.0);
        assert_eq!(start.asynchronous_rotation_degrees, 2.0);
        let jitter_length = (start.pip_jitter_x_px.powi(2) + start.pip_jitter_y_px.powi(2)).sqrt();
        assert!((jitter_length - 4.0).abs() < 1e-12);

        let at_50 = scheduler
            .decide(observation(
                &identity,
                &params,
                4,
                50,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("50ms");
        assert!(!schedule(&at_50).highlight_active);
        assert!((schedule(&at_50).transform_easing - 0.5).abs() < 1e-12);

        let quarter_second = scheduler
            .decide(observation(
                &identity,
                &params,
                4,
                250,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("250ms");
        let quarter_second = schedule(&quarter_second);
        assert!(!quarter_second.local_blur_active);
        assert!((quarter_second.target_fps - 63.2).abs() < 1e-12);
        assert!((quarter_second.base_video_speed - 63.2 / 60.0).abs() < 1e-12);
        assert!(!quarter_second.frame_rate_locked);

        let outside_slice = scheduler
            .decide(observation(
                &identity,
                &params,
                4,
                500,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("slice end");
        assert!(!schedule(&outside_slice).slice_active);
        assert_eq!(schedule(&outside_slice).transform_easing, 0.0);

        let rotation_peak = scheduler
            .decide(observation(
                &identity,
                &params,
                4,
                1_250,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("rotation peak");
        assert!((schedule(&rotation_peak).asynchronous_rotation_degrees - 6.0).abs() < 1e-12);
    }

    #[test]
    fn frame_rate_lock_pins_the_source_timebase() {
        let identity = identity(5, 10);
        let mut params = MediaEffectParams::default();
        params.video.frame_rate_jitter_percent = 2.0;
        params.video.frame_rate_perturbation_amplitude_fps = 2.0;
        params.video.frame_rate_perturbation_frequency_hz = 1.0;
        params.video.frame_rate_lock_enabled = true;
        let mut scheduler = VideoFrameScheduler::new();

        scheduler
            .decide(observation(
                &identity,
                &params,
                1,
                0,
                false,
                VideoScheduleBoundary::Startup,
            ))
            .expect("locked schedule startup");

        let decision = scheduler
            .decide(observation(
                &identity,
                &params,
                1,
                250,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("locked schedule");

        assert_eq!(schedule(&decision).target_fps, 60.0);
        assert_eq!(schedule(&decision).base_video_speed, 1.0);
        assert!(schedule(&decision).frame_rate_locked);
    }

    #[test]
    fn asynchronous_rotation_remains_inside_the_configured_range() {
        let identity = identity(5, 10);
        let mut params = MediaEffectParams::default();
        params.advanced.asynchronous_rotation_enabled = true;
        params.advanced.asynchronous_rotation_min_degrees = -2.0;
        params.advanced.asynchronous_rotation_max_degrees = 6.0;
        params.advanced.slice_trigger_interval_ms = 5_000;
        let mut scheduler = VideoFrameScheduler::new();

        for (index, pts_ms) in [0, 625, 1_250, 1_875, 2_500, 3_750, 4_999]
            .into_iter()
            .enumerate()
        {
            let decision = scheduler
                .decide(observation(
                    &identity,
                    &params,
                    1,
                    pts_ms,
                    false,
                    if index == 0 {
                        VideoScheduleBoundary::Startup
                    } else {
                        VideoScheduleBoundary::None
                    },
                ))
                .expect("bounded asynchronous rotation");
            assert!((-2.0..=6.0).contains(&schedule(&decision).asynchronous_rotation_degrees));
        }
    }

    #[test]
    fn ordinary_pts_ticks_hold_the_committed_shader_snapshot() {
        let identity = identity(5, 10);
        let mut params = MediaEffectParams::default();
        params.video.frame_rate_jitter_percent = 1.0;
        params.advanced.asynchronous_rotation_enabled = true;
        let mut scheduler = VideoFrameScheduler::new();

        let committed = scheduler
            .decide(observation(
                &identity,
                &params,
                1,
                1_000,
                false,
                VideoScheduleBoundary::Startup,
            ))
            .expect("commit boundary");
        assert_eq!(committed.action, VideoScheduleAction::Apply);

        for tick in 1..=200 {
            let decision = scheduler
                .decide(observation(
                    &identity,
                    &params,
                    1,
                    1_000 + tick * 40,
                    false,
                    VideoScheduleBoundary::None,
                ))
                .expect("ordinary PTS tick");
            assert_eq!(decision.action, VideoScheduleAction::Hold);
            assert_eq!(schedule(&decision).media_pts_ms, 1_000 + tick * 40);
        }
    }

    #[test]
    fn slice_schedule_exposes_minimum_source_length_and_trigger_boundaries() {
        let identity = identity(5, 10);
        let mut params = MediaEffectParams::default();
        params.advanced.slice_length_ms = 1_000;
        params.advanced.slice_min_length_ms = 4_000;
        params.advanced.slice_trigger_interval_ms = 5_000;
        let mut scheduler = VideoFrameScheduler::new();

        let start = scheduler
            .decide(observation(
                &identity,
                &params,
                1,
                10_000,
                false,
                VideoScheduleBoundary::Startup,
            ))
            .expect("slice start");
        assert_eq!(timing(&start).slice_window_index, 0);
        assert_eq!(timing(&start).slice_window_start_pts_ms, 10_000);
        assert_eq!(timing(&start).slice_window_end_pts_ms, 11_000);
        assert_eq!(timing(&start).slice_source_min_length_ms, 4_000);

        let second = scheduler
            .decide(observation(
                &identity,
                &params,
                1,
                15_000,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("second slice window");
        assert_eq!(timing(&second).slice_window_index, 1);
        assert_eq!(timing(&second).slice_window_start_pts_ms, 15_000);
        assert_eq!(timing(&second).slice_window_end_pts_ms, 16_000);
        assert!(schedule(&second).slice_active);
    }

    #[test]
    fn pip_timeline_lock_controls_only_the_pts_schedule() {
        let identity = identity(5, 10);
        let mut locked_params = MediaEffectParams::default();
        locked_params.advanced.picture_in_picture_enabled = true;
        locked_params.advanced.picture_in_picture_timeline_locked = true;
        locked_params.advanced.slice_min_length_ms = 4_000;
        let mut unlocked_params = locked_params.clone();
        unlocked_params.advanced.picture_in_picture_timeline_locked = false;

        let mut locked_scheduler = VideoFrameScheduler::new();
        let locked = locked_scheduler
            .decide(observation(
                &identity,
                &locked_params,
                1,
                12_345,
                false,
                VideoScheduleBoundary::Startup,
            ))
            .expect("locked PIP timeline");
        assert!(timing(&locked).pip_timeline_locked);
        assert_eq!(timing(&locked).pip_timeline_offset_ms, 0);
        assert_eq!(timing(&locked).pip_timeline_pts_ms, 12_345);

        let mut first = VideoFrameScheduler::new();
        let mut second = VideoFrameScheduler::new();
        let first = first
            .decide(observation(
                &identity,
                &unlocked_params,
                1,
                12_345,
                false,
                VideoScheduleBoundary::Startup,
            ))
            .expect("first unlocked PIP timeline");
        let second = second
            .decide(observation(
                &identity,
                &unlocked_params,
                1,
                12_345,
                false,
                VideoScheduleBoundary::Startup,
            ))
            .expect("second unlocked PIP timeline");
        assert_eq!(first, second);
        assert!(!timing(&first).pip_timeline_locked);
        assert!(timing(&first).pip_timeline_offset_ms < 4_000);
        assert_eq!(
            timing(&first).pip_timeline_pts_ms,
            12_345 - timing(&first).pip_timeline_offset_ms
        );
    }

    #[test]
    fn splitmix_and_probability_fields_are_independent() {
        assert_eq!(splitmix64(0), 0xe220_a839_7b1d_cdaf);
        let identity = identity(5, 10);
        assert!(!probability_gate(&identity, 29, FIELD_FRAME_INNER, 42, 0.0,));
        assert!(probability_gate(
            &identity,
            29,
            FIELD_FRAME_INNER,
            42,
            100.0,
        ));
        assert_ne!(
            random_unit(&identity, 29, FIELD_FRAME_INNER, 42),
            random_unit(&identity, 29, FIELD_FRAME_INTER, 42)
        );
    }

    #[test]
    fn pip_jitter_is_stable_within_the_same_source_frame() {
        let identity = identity(5, 10);
        let mut params = MediaEffectParams::default();
        params.advanced.picture_in_picture_enabled = true;
        params.advanced.picture_in_picture_pixel_jitter_px = 4.0;
        let mut scheduler = VideoFrameScheduler::new();

        let first = scheduler
            .decide(observation(
                &identity,
                &params,
                1,
                1_000,
                false,
                VideoScheduleBoundary::Startup,
            ))
            .expect("first frame sample");
        let same_frame = scheduler
            .decide(observation(
                &identity,
                &params,
                1,
                1_010,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("same frame sampled again");

        assert_eq!(
            schedule(&first).frame_index,
            schedule(&same_frame).frame_index
        );
        assert_eq!(
            schedule(&first).pip_jitter_x_px,
            schedule(&same_frame).pip_jitter_x_px
        );
        assert_eq!(
            schedule(&first).pip_jitter_y_px,
            schedule(&same_frame).pip_jitter_y_px
        );
    }

    #[test]
    fn a_new_sequence_applies_at_the_same_pts_and_rejects_the_old_sequence() {
        let params = MediaEffectParams::default();
        let first_identity = identity(5, 10);
        let next_identity = identity(5, 11);
        let mut scheduler = VideoFrameScheduler::new();
        scheduler
            .decide(observation(
                &first_identity,
                &params,
                1,
                1_000,
                false,
                VideoScheduleBoundary::Startup,
            ))
            .expect("first sequence");

        let applied = scheduler
            .decide(observation(
                &next_identity,
                &params,
                1,
                1_000,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("next sequence");
        assert_eq!(applied.action, VideoScheduleAction::Apply);
        assert_eq!(schedule(&applied).identity.sequence, 11);
        assert_eq!(schedule(&applied).epoch_elapsed_ms, 0);

        let stale = scheduler
            .decide(observation(
                &first_identity,
                &params,
                1,
                1_000,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("old sequence is rejected");
        assert_eq!(
            stale.action,
            VideoScheduleAction::Ignore {
                reason: VideoScheduleIgnoreReason::StaleSequence
            }
        );
    }

    #[test]
    fn a_new_sequence_survives_a_small_source_pts_regression_without_losing_the_schedule() {
        let params = MediaEffectParams::default();
        let first_identity = identity(5, 10);
        let next_identity = identity(5, 11);
        let mut scheduler = VideoFrameScheduler::new();
        scheduler
            .decide(observation(
                &first_identity,
                &params,
                1,
                10_000,
                false,
                VideoScheduleBoundary::Startup,
            ))
            .expect("first sequence");

        let applied = scheduler
            .decide(observation(
                &next_identity,
                &params,
                1,
                9_950,
                false,
                VideoScheduleBoundary::None,
            ))
            .expect("next sequence with a bounded demux PTS regression");

        assert_eq!(applied.action, VideoScheduleAction::Apply);
        assert_eq!(schedule(&applied).identity.sequence, 11);
        assert_eq!(schedule(&applied).media_pts_ms, 10_000);
    }
}
