//! 由受管 mpv 源内呈现时间驱动的视频参数周期计划。

use crate::media_effect_params::{
    AdvancedEffectParams, MediaEffectParams, VideoEffectParams, VISUAL_BAND_FREQUENCIES_HZ,
};
pub use crate::media_timeline::MediaSegmentIdentity;

pub const VIDEO_PERIOD_MIN_MS: u64 = 1_000;
pub const VIDEO_PERIOD_MAX_MS: u64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCycleConfigError {
    PeriodOutOfRange,
    InvalidPeriodRange,
    InvalidBaseline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCycleError {
    InvalidIdentity,
    InvalidPresentedPts,
    NoActiveSegment,
    PresentedPtsRegressed {
        previous_ms: u64,
        observed_ms: u64,
        delta_ms: u64,
        tolerance_ms: u64,
    },
    StalePlan,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoCycleConfig {
    enabled: bool,
    period_min_ms: u64,
    period_max_ms: u64,
    baseline: MediaEffectParams,
}

impl VideoCycleConfig {
    pub fn try_new(
        enabled: bool,
        period_min_ms: u64,
        period_max_ms: u64,
        baseline: MediaEffectParams,
    ) -> Result<Self, VideoCycleConfigError> {
        if !(VIDEO_PERIOD_MIN_MS..=VIDEO_PERIOD_MAX_MS).contains(&period_min_ms)
            || !(VIDEO_PERIOD_MIN_MS..=VIDEO_PERIOD_MAX_MS).contains(&period_max_ms)
        {
            return Err(VideoCycleConfigError::PeriodOutOfRange);
        }
        if period_min_ms > period_max_ms {
            return Err(VideoCycleConfigError::InvalidPeriodRange);
        }
        if baseline.video.validate().is_err() || baseline.advanced.validate().is_err() {
            return Err(VideoCycleConfigError::InvalidBaseline);
        }
        Ok(Self {
            enabled,
            period_min_ms,
            period_max_ms,
            baseline,
        })
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn period_range_ms(&self) -> (u64, u64) {
        (self.period_min_ms, self.period_max_ms)
    }

    pub fn baseline(&self) -> &MediaEffectParams {
        &self.baseline
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoCycleSegmentIdentity {
    pub session_id: u64,
    pub backend_epoch: u64,
    pub clock_epoch: u64,
    pub media: MediaSegmentIdentity,
}

impl VideoCycleSegmentIdentity {
    pub fn try_new(
        session_id: u64,
        backend_epoch: u64,
        clock_epoch: u64,
        media: MediaSegmentIdentity,
    ) -> Result<Self, VideoCycleError> {
        if session_id == 0 {
            return Err(VideoCycleError::InvalidIdentity);
        }
        Ok(Self {
            session_id,
            backend_epoch,
            clock_epoch,
            media,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AutomaticVideoParameterSnapshot {
    pub seed: u32,
    pub video: VideoEffectParams,
    pub advanced: AdvancedEffectParams,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoCyclePlan {
    pub identity: VideoCycleSegmentIdentity,
    pub sequence: u64,
    pub target_source_pts_ms: u64,
    pub period_ms: u64,
    pub seed: u32,
    pub source_fps: Option<f64>,
    pub video_params: VideoEffectParams,
    pub advanced_params: AdvancedEffectParams,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct VideoCycleQueue {
    pub n: Option<VideoCyclePlan>,
    pub n_plus_1: Option<VideoCyclePlan>,
    pub n_plus_2: Option<VideoCyclePlan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCycleApplyStage {
    Applying,
    ResultUnknown,
    ReadbackConfirmed,
    PresentedConfirmed,
    Active,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoCycleApplyTransaction {
    pub sequence: u64,
    pub fingerprint: String,
    pub stage: VideoCycleApplyStage,
    pub retry_count: u8,
    pub result_unknown: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoCycleApplyResult {
    Applying { fingerprint: String },
    ResultUnknown,
    ReadbackConfirmed,
    PresentedConfirmed,
    Active,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VideoCycleEvent {
    Configure {
        config: Box<VideoCycleConfig>,
        current_source_pts_ms: u64,
    },
    PlaybackIntent {
        paused: bool,
        seek_source_pts_ms: Option<u64>,
        clock_epoch: u64,
    },
    SourceBoundary {
        identity: VideoCycleSegmentIdentity,
        source_fps: Option<f64>,
        source_pts_ms: u64,
    },
    MpvObservation {
        source_pts_ms: u64,
        source_fps: Option<f64>,
        paused: bool,
    },
    ApplyResult {
        sequence: u64,
        source_pts_ms: u64,
        result: VideoCycleApplyResult,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoCyclePublishedState {
    pub config_revision: u64,
    pub apply_transaction: Option<VideoCycleApplyTransaction>,
    pub confirmed_change_count: u64,
    pub source_fps: Option<f64>,
    pub queue: VideoCycleQueue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VideoCycleAction {
    PrepareNext(Box<VideoCyclePlan>),
    Apply(Box<VideoCyclePlan>),
    Clear,
    Publish(Box<VideoCyclePublishedState>),
}

#[derive(Debug, Clone)]
pub struct VideoCycleController {
    config: VideoCycleConfig,
    config_revision: u64,
    identity: Option<VideoCycleSegmentIdentity>,
    source_fps: Option<f64>,
    queue: VideoCycleQueue,
    next_sequence: u64,
    last_source_pts_ms: Option<u64>,
    apply_transaction: Option<VideoCycleApplyTransaction>,
    confirmed_change_count: u64,
}

impl VideoCycleController {
    pub fn new(config: VideoCycleConfig) -> Self {
        Self {
            config,
            config_revision: 1,
            identity: None,
            source_fps: None,
            queue: VideoCycleQueue::default(),
            next_sequence: 1,
            last_source_pts_ms: None,
            apply_transaction: None,
            confirmed_change_count: 0,
        }
    }

    pub fn config(&self) -> &VideoCycleConfig {
        &self.config
    }

    pub fn identity(&self) -> Option<&VideoCycleSegmentIdentity> {
        self.identity.as_ref()
    }

    pub fn config_revision(&self) -> u64 {
        self.config_revision
    }

    pub fn applying_sequence(&self) -> Option<u64> {
        self.apply_transaction
            .as_ref()
            .map(|transaction| transaction.sequence)
    }

    pub fn apply_transaction(&self) -> Option<&VideoCycleApplyTransaction> {
        self.apply_transaction.as_ref()
    }

    pub fn confirmed_change_count(&self) -> u64 {
        self.confirmed_change_count
    }

    pub fn source_fps(&self) -> Option<f64> {
        self.source_fps
    }

    pub fn queue(&self) -> &VideoCycleQueue {
        &self.queue
    }

    pub fn handle(
        &mut self,
        event: VideoCycleEvent,
    ) -> Result<Vec<VideoCycleAction>, VideoCycleError> {
        let mut actions = Vec::with_capacity(3);
        match event {
            VideoCycleEvent::Configure {
                config,
                current_source_pts_ms,
            } => {
                self.configure(*config, current_source_pts_ms)?;
                actions.push(VideoCycleAction::Clear);
                self.push_prepare_action(&mut actions);
            }
            VideoCycleEvent::PlaybackIntent {
                paused: _,
                seek_source_pts_ms,
                clock_epoch,
            } => {
                if let Some(source_pts_ms) = seek_source_pts_ms {
                    self.reset_for_seek(clock_epoch, source_pts_ms)?;
                    actions.push(VideoCycleAction::Clear);
                    self.push_prepare_action(&mut actions);
                }
                // 暂停只冻结 PTS 推进。已经写入 mpv、尚待回读或呈现确认的事务
                // 必须保留；否则恢复播放后的迟到响应会被错误判为陈旧提交。
            }
            VideoCycleEvent::SourceBoundary {
                identity,
                source_fps,
                source_pts_ms,
            } => {
                self.replace_segment(identity, source_fps, source_pts_ms)?;
                actions.push(VideoCycleAction::Clear);
                self.push_prepare_action(&mut actions);
            }
            VideoCycleEvent::MpvObservation {
                source_pts_ms,
                source_fps,
                paused,
            } => {
                if let Some(plan) = self.observe_source(source_pts_ms, source_fps, paused)? {
                    actions.push(VideoCycleAction::Apply(Box::new(plan)));
                }
            }
            VideoCycleEvent::ApplyResult {
                sequence,
                source_pts_ms,
                result,
            } => match result {
                VideoCycleApplyResult::Applying { fingerprint } => {
                    self.begin_apply(sequence, fingerprint)?
                }
                VideoCycleApplyResult::ResultUnknown => {
                    self.update_apply_stage(sequence, VideoCycleApplyStage::ResultUnknown)?;
                    if let Some(transaction) = self.apply_transaction.as_mut() {
                        transaction.result_unknown = true;
                    }
                }
                VideoCycleApplyResult::ReadbackConfirmed => {
                    self.update_apply_stage(sequence, VideoCycleApplyStage::ReadbackConfirmed)?;
                }
                VideoCycleApplyResult::PresentedConfirmed => {
                    self.update_apply_stage(sequence, VideoCycleApplyStage::PresentedConfirmed)?;
                }
                VideoCycleApplyResult::Active => {
                    self.advance_due_plan(sequence, source_pts_ms)?;
                    self.update_apply_stage(sequence, VideoCycleApplyStage::Active)?;
                    self.push_prepare_action(&mut actions);
                }
                VideoCycleApplyResult::Failed => {
                    self.update_apply_stage(sequence, VideoCycleApplyStage::Failed)?;
                }
            },
        }
        actions.push(VideoCycleAction::Publish(Box::new(self.published_state())));
        Ok(actions)
    }

    fn push_prepare_action(&self, actions: &mut Vec<VideoCycleAction>) {
        if let Some(plan) = self.queue.n_plus_1.clone() {
            actions.push(VideoCycleAction::PrepareNext(Box::new(plan)));
        }
    }

    fn published_state(&self) -> VideoCyclePublishedState {
        VideoCyclePublishedState {
            config_revision: self.config_revision,
            apply_transaction: self.apply_transaction.clone(),
            confirmed_change_count: self.confirmed_change_count,
            source_fps: self.source_fps,
            queue: self.queue.clone(),
        }
    }

    pub fn configure(
        &mut self,
        config: VideoCycleConfig,
        current_source_pts_ms: u64,
    ) -> Result<(), VideoCycleError> {
        self.ensure_source_pts(current_source_pts_ms)?;
        self.config = config;
        self.config_revision = self.config_revision.saturating_add(1);
        self.queue = VideoCycleQueue::default();
        self.apply_transaction = None;
        self.last_source_pts_ms = Some(current_source_pts_ms);
        self.fill_future_from(current_source_pts_ms);
        Ok(())
    }

    /// 开始新媒体、换源或进入新 loop；旧计划和旧源 FPS 会一次性清除。
    pub fn replace_segment(
        &mut self,
        identity: VideoCycleSegmentIdentity,
        source_fps: Option<f64>,
        source_pts_ms: u64,
    ) -> Result<(), VideoCycleError> {
        if source_pts_ms >= identity.media.source_duration_ms {
            return Err(VideoCycleError::InvalidPresentedPts);
        }
        self.identity = Some(identity);
        self.source_fps = normalized_source_fps(source_fps);
        self.queue = VideoCycleQueue::default();
        self.next_sequence = 1;
        self.last_source_pts_ms = Some(source_pts_ms);
        self.apply_transaction = None;
        if self.source_fps.is_some() {
            self.fill_future_from(source_pts_ms);
        }
        Ok(())
    }

    /// seek 必须携带新的时钟代次，防止 seek 前计划再次成为有效计划。
    pub fn reset_for_seek(
        &mut self,
        clock_epoch: u64,
        source_pts_ms: u64,
    ) -> Result<(), VideoCycleError> {
        self.ensure_source_pts(source_pts_ms)?;
        let identity = self
            .identity
            .as_mut()
            .ok_or(VideoCycleError::NoActiveSegment)?;
        identity.clock_epoch = clock_epoch;
        self.queue = VideoCycleQueue::default();
        self.next_sequence = 1;
        self.last_source_pts_ms = Some(source_pts_ms);
        self.apply_transaction = None;
        self.fill_future_from(source_pts_ms);
        Ok(())
    }

    /// Rust 音画同步在同一媒体段内完成硬对齐后，只重置自然观察的单调高水位。
    /// 该内部纠偏不改变媒体身份，也不清除已经确认或正在确认的周期事务。
    pub fn rebase_source_pts(&mut self, source_pts_ms: u64) -> Result<(), VideoCycleError> {
        self.ensure_source_pts(source_pts_ms)?;
        self.last_source_pts_ms = Some(source_pts_ms);
        Ok(())
    }

    pub fn clear(&mut self) {
        self.identity = None;
        self.source_fps = None;
        self.queue = VideoCycleQueue::default();
        self.next_sequence = 1;
        self.last_source_pts_ms = None;
        self.apply_transaction = None;
    }

    /// 窥视本次应提交的唯一计划，不提前把未确认的物理提交晋级为 N。
    pub fn due_plan(
        &mut self,
        source_pts_ms: u64,
        paused: bool,
    ) -> Result<Option<VideoCyclePlan>, VideoCycleError> {
        self.ensure_source_pts(source_pts_ms)?;
        if paused {
            return Ok(None);
        }
        let source_pts_ms = match self.last_source_pts_ms {
            Some(last_source_pts_ms) if source_pts_ms < last_source_pts_ms => {
                let regression_ms = last_source_pts_ms.saturating_sub(source_pts_ms);
                let tolerance_ms = source_pts_regression_tolerance_ms(
                    self.source_fps,
                    is_mpeg_transport_stream(self.identity.as_ref()),
                );
                if regression_ms > tolerance_ms {
                    return Err(VideoCycleError::PresentedPtsRegressed {
                        previous_ms: last_source_pts_ms,
                        observed_ms: source_pts_ms,
                        delta_ms: regression_ms,
                        tolerance_ms,
                    });
                }
                // TS/B-frame 时间戳可能在同一帧周期内轻微回摆。周期控制器仍以
                // 已观测到的最大 PTS 推进，避免连续抖动累计成逻辑时钟倒退。
                last_source_pts_ms
            }
            _ => source_pts_ms,
        };
        self.last_source_pts_ms = Some(source_pts_ms);

        let n_plus_1_due = self
            .queue
            .n_plus_1
            .as_ref()
            .is_some_and(|plan| source_pts_ms >= plan.target_source_pts_ms);
        let n_plus_2_due = self
            .queue
            .n_plus_2
            .as_ref()
            .is_some_and(|plan| source_pts_ms >= plan.target_source_pts_ms);
        if !n_plus_1_due {
            return Ok(None);
        }
        Ok(if n_plus_2_due {
            self.queue.n_plus_2.clone()
        } else {
            self.queue.n_plus_1.clone()
        })
    }

    /// 使用最新 mpv observation 更新源 FPS；每次观测都覆盖旧值，跨媒体不得继承。
    pub fn observe_source(
        &mut self,
        source_pts_ms: u64,
        source_fps: Option<f64>,
        paused: bool,
    ) -> Result<Option<VideoCyclePlan>, VideoCycleError> {
        self.source_fps = normalized_source_fps(source_fps);
        if self.source_fps.is_some()
            && self.queue.n_plus_1.is_none()
            && self.queue.n_plus_2.is_none()
        {
            self.fill_future_from(source_pts_ms);
        }
        self.due_plan(source_pts_ms, paused)
    }

    pub fn begin_apply(
        &mut self,
        sequence: u64,
        fingerprint: String,
    ) -> Result<(), VideoCycleError> {
        let planned = self.queue.n_plus_1.as_ref().map(|plan| plan.sequence) == Some(sequence)
            || self.queue.n_plus_2.as_ref().map(|plan| plan.sequence) == Some(sequence);
        if !planned || fingerprint.is_empty() {
            return Err(VideoCycleError::StalePlan);
        }
        if let Some(transaction) = self.apply_transaction.as_mut() {
            if transaction.sequence == sequence && transaction.fingerprint == fingerprint {
                transaction.stage = VideoCycleApplyStage::Applying;
                transaction.retry_count = transaction.retry_count.saturating_add(1);
                return Ok(());
            }
            if !matches!(
                transaction.stage,
                VideoCycleApplyStage::Active | VideoCycleApplyStage::Failed
            ) {
                return Err(VideoCycleError::StalePlan);
            }
        }
        self.apply_transaction = Some(VideoCycleApplyTransaction {
            sequence,
            fingerprint,
            stage: VideoCycleApplyStage::Applying,
            retry_count: 0,
            result_unknown: false,
        });
        Ok(())
    }

    pub fn cancel_apply(&mut self, sequence: u64) {
        if let Some(transaction) = self
            .apply_transaction
            .as_mut()
            .filter(|transaction| transaction.sequence == sequence)
        {
            transaction.stage = VideoCycleApplyStage::Failed;
        }
    }

    fn update_apply_stage(
        &mut self,
        sequence: u64,
        stage: VideoCycleApplyStage,
    ) -> Result<(), VideoCycleError> {
        let transaction = self
            .apply_transaction
            .as_mut()
            .filter(|transaction| transaction.sequence == sequence)
            .ok_or(VideoCycleError::StalePlan)?;
        transaction.stage = stage;
        Ok(())
    }

    /// 仅在 runtime 已确认物理提交后晋级；序号不再是当前最新到期计划时拒绝。
    pub fn advance_due_plan(
        &mut self,
        sequence: u64,
        source_pts_ms: u64,
    ) -> Result<VideoCyclePlan, VideoCycleError> {
        self.ensure_source_pts(source_pts_ms)?;
        let n_plus_2_due = self
            .queue
            .n_plus_2
            .as_ref()
            .is_some_and(|plan| source_pts_ms >= plan.target_source_pts_ms);
        let due = if n_plus_2_due {
            self.queue.n_plus_2.as_ref()
        } else {
            self.queue
                .n_plus_1
                .as_ref()
                .filter(|plan| source_pts_ms >= plan.target_source_pts_ms)
        }
        .filter(|plan| plan.sequence == sequence)
        .cloned()
        .ok_or(VideoCycleError::StalePlan)?;
        if self
            .apply_transaction
            .as_ref()
            .map(|transaction| transaction.sequence)
            != Some(sequence)
        {
            return Err(VideoCycleError::StalePlan);
        }

        self.queue.n = Some(due.clone());

        if n_plus_2_due {
            self.queue.n_plus_2 = None;
            self.queue.n_plus_1 = None;
            self.fill_future_from(source_pts_ms);
        } else {
            self.queue.n_plus_1 = self.queue.n_plus_2.take();
            let anchor = self
                .queue
                .n_plus_1
                .as_ref()
                .map_or(source_pts_ms, |plan| plan.target_source_pts_ms);
            self.queue.n_plus_2 = self.build_plan_after(anchor);
        }
        self.confirmed_change_count = self.confirmed_change_count.saturating_add(1);
        Ok(due)
    }

    /// 纯调度调用的组合入口；有物理提交的 runtime 应使用 due_plan + advance_due_plan。
    pub fn observe_presented_pts(
        &mut self,
        source_pts_ms: u64,
        paused: bool,
    ) -> Result<Option<VideoCyclePlan>, VideoCycleError> {
        let Some(due) = self.due_plan(source_pts_ms, paused)? else {
            return Ok(None);
        };
        self.begin_apply(due.sequence, format!("local:{}", due.sequence))?;
        let advanced = self.advance_due_plan(due.sequence, source_pts_ms)?;
        self.update_apply_stage(due.sequence, VideoCycleApplyStage::Active)?;
        Ok(Some(advanced))
    }

    fn ensure_source_pts(&self, source_pts_ms: u64) -> Result<(), VideoCycleError> {
        let identity = self
            .identity
            .as_ref()
            .ok_or(VideoCycleError::NoActiveSegment)?;
        (source_pts_ms < identity.media.source_duration_ms)
            .then_some(())
            .ok_or(VideoCycleError::InvalidPresentedPts)
    }

    fn fill_future_from(&mut self, source_pts_ms: u64) {
        if !self.config.enabled {
            return;
        }
        if self.queue.n_plus_1.is_none() {
            self.queue.n_plus_1 = self.build_plan_after(source_pts_ms);
        }
        if self.queue.n_plus_2.is_none() {
            let anchor = self
                .queue
                .n_plus_1
                .as_ref()
                .map_or(source_pts_ms, |plan| plan.target_source_pts_ms);
            self.queue.n_plus_2 = self.build_plan_after(anchor);
        }
    }

    fn build_plan_after(&mut self, source_pts_ms: u64) -> Option<VideoCyclePlan> {
        let identity = self.identity.clone()?;
        let sequence = self.next_sequence;
        let next_sequence = sequence.checked_add(1)?;
        let seed = video_cycle_seed(&identity, sequence);
        let period_ms = sample_period_ms(
            seed ^ 0xa511_e9b3,
            self.config.period_min_ms,
            self.config.period_max_ms,
        );
        let requested_target = source_pts_ms.checked_add(period_ms)?;
        let target_source_pts_ms = align_to_video_frame(requested_target, self.source_fps);
        if target_source_pts_ms >= identity.media.source_duration_ms {
            return None;
        }
        let snapshot = sample_automatic_video_parameters(seed, &self.config.baseline);
        self.next_sequence = next_sequence;
        Some(VideoCyclePlan {
            identity,
            sequence,
            target_source_pts_ms,
            period_ms: target_source_pts_ms.saturating_sub(source_pts_ms),
            seed,
            source_fps: self.source_fps,
            video_params: snapshot.video,
            advanced_params: snapshot.advanced,
        })
    }
}

pub fn video_cycle_seed(identity: &VideoCycleSegmentIdentity, sequence: u64) -> u32 {
    let mut hash = 0x811c_9dc5_u32;
    for value in [
        identity.session_id,
        identity.media.playback_generation,
        identity.clock_epoch,
        identity.media.loop_index,
        sequence,
    ] {
        for byte in value.to_le_bytes() {
            hash ^= u32::from(byte);
            hash = hash.wrapping_mul(0x0100_0193);
        }
    }
    hash
}

pub fn sample_automatic_video_parameters(
    seed: u32,
    baseline: &MediaEffectParams,
) -> AutomaticVideoParameterSnapshot {
    let mut random = Mulberry32::new(seed);
    let band_weights = VISUAL_BAND_FREQUENCIES_HZ
        .iter()
        .enumerate()
        .map(|(index, frequency_hz)| {
            let direction = if index % 2 == 0 { -1.0 } else { 1.0 };
            (
                *frequency_hz,
                round_to(1.0 + direction * random.in_range(0.001, 0.003), 4),
            )
        })
        .collect();

    let mut video = VideoEffectParams {
        brightness_percent: round_to(random.signed(0.1, 0.35), 3),
        saturation_percent: round_to(100.0 + random.signed(0.1, 0.3), 3),
        blur_radius_px: round_to(random.in_range(0.01, 0.05), 3),
        contrast_percent: round_to(100.0 + random.signed(0.1, 0.3), 3),
        hue_rotation_degrees: round_to(random.signed(0.05, 0.2), 3),
        sharpen_percent: round_to(random.in_range(0.1, 0.4), 3),
        noise_percent: 1.0,
        detail_enhancement_percent: round_to(random.in_range(0.1, 0.35), 3),
        crop_edge_smoothing: round_to(random.in_range(0.7, 0.9), 3),
        frame_rate_jitter_percent: round_to(random.in_range(0.01, 0.04), 3),
        frame_rate_perturbation_frequency_hz: round_to(random.in_range(0.05, 0.18), 3),
        frame_rate_perturbation_amplitude_fps: round_to(random.in_range(0.01, 0.04), 3),
        pixel_scale_percent: round_to(100.0 + random.signed(0.1, 0.2), 3),
        pixel_jitter_px: round_to(random.in_range(0.125, 0.375), 3),
        dynamic_crop_percent: round_to(random.in_range(0.03, 0.1), 3),
        frame_inner_perturbation_percent: round_to(random.in_range(0.01, 0.04), 3),
        frame_inter_perturbation_percent: round_to(random.in_range(0.03, 0.1), 3),
        space_x_offset_px: random.signed(0.5, 0.5),
        space_y_offset_px: random.signed(0.5, 0.5),
        color_space_conversion_strength_percent: round_to(random.in_range(0.1, 0.4), 3),
        color_space_conversion_enabled: true,
        horizontal_flip_enabled: false,
        vertical_flip_enabled: false,
        rotation_degrees: round_to(random.signed(0.01, 0.04), 3),
        vignette_percent: round_to(random.in_range(0.05, 0.2), 3),
        highlights_percent: round_to(random.signed(0.05, 0.2), 3),
        shadows_percent: round_to(random.signed(0.05, 0.2), 3),
        red_channel_lock_enabled: true,
        edge_softness_percent: round_to(random.in_range(0.05, 0.2), 3),
        image_repair_enabled: true,
        image_repair_strength_percent: round_to(random.in_range(0.05, 0.2), 3),
        frame_rate_lock_enabled: true,
    };

    let mut advanced = AdvancedEffectParams {
        band_weights,
        target_frequency_hz: Some(random.integer(65, 20_000) as f64),
        core_frequency_hz: Some(random.integer(65, 20_000) as f64),
        wave_intensity: round_to(random.in_range(0.084, 0.1), 4),
        wave_level: round_to(random.in_range(0.251, 0.3), 4),
        wave_grain_count: random.integer(7, 13) as u32,
        dynamic_eq_threshold: round_to(random.in_range(0.1, 0.3), 3),
        channel_offset_percent: round_to(random.signed(0.1, 0.3), 3),
        space_dimension: random.integer(2, 3) as u8,
        frequency_space_x_offset_px: round_to(random.signed(0.1, 0.3), 3),
        frequency_space_y_offset_px: round_to(random.signed(0.1, 0.3), 3),
        frame_perturbation_probability_percent: round_to(random.in_range(0.1, 0.3), 3),
        random_graphic_opacity_percent: round_to(random.in_range(0.8, 1.2), 3),
        random_graphic_size_px: round_to(random.in_range(1.0, 2.0), 3),
        abstract_face_count: 1,
        abstract_face_size_percent: round_to(random.in_range(1.0, 1.5), 3),
        abstract_face_opacity_percent: round_to(random.in_range(0.8, 1.2), 3),
        overlay_offset_px: random.signed(0.5, 0.5),
        slice_length_ms: random.stepped_integer(500, 800, 100),
        slice_min_length_ms: random.stepped_integer(1_000, 1_500, 100),
        slice_trigger_interval_ms: random.stepped_integer(5_000, 8_000, 100),
        random_graphic_enabled: true,
        random_graphic_count: 1,
        picture_in_picture_enabled: true,
        picture_in_picture_scale_percent: round_to(random.in_range(10.0, 12.0), 3),
        picture_in_picture_opacity_percent: round_to(random.in_range(0.8, 1.2), 3),
        picture_in_picture_rotation_degrees: round_to(random.signed(0.1, 0.2), 3),
        picture_in_picture_pixel_jitter_px: round_to(random.in_range(0.125, 0.375), 3),
        picture_in_picture_timeline_locked: false,
        local_blur_enabled: true,
        local_blur_region_percent: round_to(random.in_range(5.0, 7.0), 3),
        local_blur_radius_px: round_to(random.in_range(0.1, 0.2), 3),
        local_blur_interval_ms: random.stepped_integer(5_000, 8_000, 100),
        edge_fill_enabled: true,
        edge_feather_percent: round_to(random.in_range(0.1, 0.4), 3),
        transform_smoothing_enabled: true,
        transform_smoothing_duration_ms: random.stepped_integer(100, 250, 10),
        highlight_perturbation_enabled: true,
        highlight_perturbation_interval_ms: random.stepped_integer(5_000, 8_000, 100),
        asynchronous_rotation_enabled: true,
        asynchronous_rotation_min_degrees: round_to(-random.in_range(0.1, 0.2), 3),
        asynchronous_rotation_max_degrees: round_to(random.in_range(0.1, 0.2), 3),
    };

    // 外部 mpv JSON IPC 尚不能原子执行的 4 项保持配置基线。
    video.color_space_conversion_strength_percent =
        baseline.video.color_space_conversion_strength_percent;
    video.color_space_conversion_enabled = baseline.video.color_space_conversion_enabled;
    advanced.slice_min_length_ms = baseline.advanced.slice_min_length_ms;
    advanced.picture_in_picture_timeline_locked =
        baseline.advanced.picture_in_picture_timeline_locked;

    AutomaticVideoParameterSnapshot {
        seed,
        video,
        advanced,
    }
}

fn normalized_source_fps(source_fps: Option<f64>) -> Option<f64> {
    source_fps.filter(|fps| fps.is_finite() && (1.0..=240.0).contains(fps))
}

fn source_pts_regression_tolerance_ms(source_fps: Option<f64>, transport_stream: bool) -> u64 {
    let (frame_window, hard_limit_ms) = if transport_stream {
        // 真实 TS 样本在 22fps 下出现过 227ms（约五帧）的自然 PTS 回摆。
        // 六帧窗口覆盖容器重排，但仍以 500ms 拒绝明显不连续跳变。
        (6_000.0, 500)
    } else {
        (2_000.0, 100)
    };
    source_fps
        // 额外 2ms 吸收 mpv 秒值转毫秒时的两端舍入误差。
        .map(|fps| {
            ((frame_window / fps).ceil() as u64)
                .saturating_add(2)
                .min(hard_limit_ms)
        })
        .unwrap_or(0)
}

fn is_mpeg_transport_stream(identity: Option<&VideoCycleSegmentIdentity>) -> bool {
    identity
        .and_then(|identity| identity.media.source_path.extension())
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("ts") || extension.eq_ignore_ascii_case("m2ts")
        })
}

fn align_to_video_frame(target_source_pts_ms: u64, source_fps: Option<f64>) -> u64 {
    let Some(source_fps) = source_fps else {
        return target_source_pts_ms;
    };
    let frame_position = target_source_pts_ms as f64 * source_fps / 1_000.0;
    let frame_index = (frame_position - 1e-9).ceil();
    let aligned = (frame_index * 1_000.0 / source_fps).round();
    (aligned as u64).max(target_source_pts_ms)
}

fn sample_period_ms(seed: u32, minimum: u64, maximum: u64) -> u64 {
    if minimum == maximum {
        return minimum;
    }
    let mut random = Mulberry32::new(seed);
    minimum + (random.next_f64() * (maximum - minimum + 1) as f64).floor() as u64
}

fn round_to(value: f64, digits: i32) -> f64 {
    let scale = 10_f64.powi(digits);
    (value * scale + 0.5).floor() / scale
}

#[derive(Debug, Clone, Copy)]
struct Mulberry32 {
    state: u32,
}

impl Mulberry32 {
    fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    fn next_f64(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x6d2b_79f5);
        let mut value = self.state;
        value = (value ^ (value >> 15)).wrapping_mul(value | 1);
        value ^= value.wrapping_add((value ^ (value >> 7)).wrapping_mul(value | 61));
        f64::from(value ^ (value >> 14)) / 4_294_967_296.0
    }

    fn in_range(&mut self, minimum: f64, maximum: f64) -> f64 {
        minimum + self.next_f64() * (maximum - minimum)
    }

    fn signed(&mut self, minimum_magnitude: f64, maximum_magnitude: f64) -> f64 {
        let direction = if self.next_f64() < 0.5 { -1.0 } else { 1.0 };
        direction * self.in_range(minimum_magnitude, maximum_magnitude)
    }

    fn integer(&mut self, minimum: u64, maximum: u64) -> u64 {
        minimum + (self.next_f64() * (maximum - minimum + 1) as f64).floor() as u64
    }

    fn stepped_integer(&mut self, minimum: u64, maximum: u64, step: u64) -> u64 {
        self.integer(minimum.div_ceil(step), maximum / step) * step
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn config(period_ms: u64) -> VideoCycleConfig {
        VideoCycleConfig::try_new(true, period_ms, period_ms, MediaEffectParams::default())
            .expect("valid video cycle config")
    }

    fn identity(
        source: &str,
        playback_generation: u64,
        loop_index: u64,
        clock_epoch: u64,
    ) -> VideoCycleSegmentIdentity {
        VideoCycleSegmentIdentity::try_new(
            41,
            7,
            clock_epoch,
            MediaSegmentIdentity::try_new(
                playback_generation,
                PathBuf::from(source),
                loop_index,
                60_000,
            )
            .expect("valid media segment"),
        )
        .expect("valid video cycle identity")
    }

    #[test]
    fn mpv_pts_alone_continuously_advances_cycles_without_ui_ticks() {
        let mut controller = VideoCycleController::new(config(1_000));
        controller
            .replace_segment(identity("one.mp4", 9, 0, 3), Some(25.0), 0)
            .expect("segment starts");

        assert_eq!(
            controller
                .queue()
                .n_plus_1
                .as_ref()
                .map(|plan| plan.sequence),
            Some(1)
        );
        let first = controller
            .observe_presented_pts(1_000, false)
            .expect("valid pts")
            .expect("first plan is due");
        let second = controller
            .observe_presented_pts(2_000, false)
            .expect("valid pts")
            .expect("second plan is due");

        assert_eq!((first.sequence, second.sequence), (1, 2));
        assert_eq!(
            controller.queue().n.as_ref().map(|plan| plan.sequence),
            Some(2)
        );
        assert_eq!(
            controller
                .queue()
                .n_plus_1
                .as_ref()
                .map(|plan| plan.sequence),
            Some(3)
        );
        assert_eq!(
            controller
                .queue()
                .n_plus_2
                .as_ref()
                .map(|plan| plan.sequence),
            Some(4)
        );
    }

    #[test]
    fn pause_freezes_the_queue_even_if_an_observation_reaches_the_target() {
        let mut controller = VideoCycleController::new(config(1_000));
        controller
            .replace_segment(identity("one.mp4", 9, 0, 3), Some(25.0), 0)
            .expect("segment starts");
        let before = controller.queue().clone();

        assert_eq!(controller.observe_presented_pts(1_500, true), Ok(None));
        assert_eq!(controller.queue(), &before);
        assert!(controller
            .observe_presented_pts(1_000, false)
            .expect("valid resumed pts")
            .is_some());
    }

    #[test]
    fn pause_intent_preserves_an_unknown_apply_transaction() {
        let mut controller = VideoCycleController::new(config(1_000));
        controller
            .replace_segment(identity("one.mp4", 9, 0, 3), Some(30.0), 0)
            .expect("segment starts");
        let sequence = controller
            .due_plan(1_000, false)
            .expect("valid pts")
            .expect("plan is due")
            .sequence;
        controller
            .begin_apply(sequence, "pending-fingerprint".to_owned())
            .expect("apply starts");
        controller
            .handle(VideoCycleEvent::ApplyResult {
                sequence,
                source_pts_ms: 1_000,
                result: VideoCycleApplyResult::ResultUnknown,
            })
            .expect("result becomes unknown");

        controller
            .handle(VideoCycleEvent::PlaybackIntent {
                paused: true,
                seek_source_pts_ms: None,
                clock_epoch: 3,
            })
            .expect("pause freezes the clock");

        let transaction = controller
            .apply_transaction()
            .expect("transaction retained");
        assert_eq!(transaction.sequence, sequence);
        assert_eq!(transaction.stage, VideoCycleApplyStage::ResultUnknown);
    }

    #[test]
    fn pts_jump_coalesces_expired_plans_instead_of_replaying_them() {
        let mut controller = VideoCycleController::new(config(1_000));
        controller
            .replace_segment(identity("one.mp4", 9, 0, 3), Some(30.0), 0)
            .expect("segment starts");

        let due = controller
            .observe_presented_pts(3_500, false)
            .expect("valid pts")
            .expect("one coalesced plan");

        assert_eq!(due.sequence, 2);
        assert_eq!(
            controller
                .queue()
                .n_plus_1
                .as_ref()
                .map(|plan| plan.sequence),
            Some(3)
        );
        assert_eq!(
            controller
                .queue()
                .n_plus_1
                .as_ref()
                .map(|plan| plan.target_source_pts_ms),
            Some(4_500)
        );
        assert_eq!(controller.observe_presented_pts(3_500, false), Ok(None));
    }

    #[test]
    fn due_plan_does_not_advance_until_physical_apply_is_confirmed() {
        let mut controller = VideoCycleController::new(config(1_000));
        controller
            .replace_segment(identity("one.mp4", 9, 0, 3), Some(30.0), 0)
            .expect("segment starts");

        let due = controller
            .due_plan(1_000, false)
            .expect("valid pts")
            .expect("plan is due");

        assert!(controller.queue().n.is_none());
        assert_eq!(controller.queue().n_plus_1.as_ref(), Some(&due));
        assert_eq!(
            controller.advance_due_plan(due.sequence + 1, 1_000),
            Err(VideoCycleError::StalePlan)
        );
        assert_eq!(
            controller.advance_due_plan(due.sequence, 1_000),
            Err(VideoCycleError::StalePlan)
        );
        assert!(controller.queue().n.is_none());

        controller
            .begin_apply(due.sequence, "confirmed-physical-apply".to_owned())
            .expect("apply transaction starts");
        controller
            .advance_due_plan(due.sequence, 1_000)
            .expect("confirmed apply advances the queue");
        assert_eq!(controller.queue().n.as_ref(), Some(&due));
    }

    #[test]
    fn source_replacement_clears_old_n_and_future_plans() {
        let mut controller = VideoCycleController::new(config(1_000));
        controller
            .replace_segment(identity("old.ts", 9, 0, 3), Some(22.001_848), 0)
            .expect("old segment starts");
        let old_target = controller
            .queue()
            .n_plus_1
            .as_ref()
            .map(|plan| plan.target_source_pts_ms)
            .expect("old target exists");
        controller
            .observe_presented_pts(old_target, false)
            .expect("valid pts")
            .expect("old plan is due");

        let replacement = identity("new.mp4", 10, 0, 4);
        controller
            .replace_segment(replacement.clone(), Some(30.0), 0)
            .expect("new segment starts");

        assert!(controller.queue().n.is_none());
        assert_eq!(
            controller
                .queue()
                .n_plus_1
                .as_ref()
                .map(|plan| &plan.identity),
            Some(&replacement)
        );
        assert_eq!(
            controller
                .queue()
                .n_plus_1
                .as_ref()
                .map(|plan| plan.sequence),
            Some(1)
        );
    }

    #[test]
    fn loop_replacement_clears_old_slots_and_changes_the_seed_namespace() {
        let mut controller = VideoCycleController::new(config(1_000));
        controller
            .replace_segment(identity("one.mp4", 9, 0, 3), Some(30.0), 0)
            .expect("first loop starts");
        let first_loop_seed = controller
            .queue()
            .n_plus_1
            .as_ref()
            .map(|plan| plan.seed)
            .expect("first loop plan");

        controller
            .replace_segment(identity("one.mp4", 9, 1, 4), Some(30.0), 0)
            .expect("second loop starts");

        assert!(controller.queue().n.is_none());
        assert_ne!(
            controller.queue().n_plus_1.as_ref().map(|plan| plan.seed),
            Some(first_loop_seed)
        );
        assert_eq!(
            controller
                .queue()
                .n_plus_1
                .as_ref()
                .map(|plan| plan.identity.media.loop_index),
            Some(1)
        );
    }

    #[test]
    fn same_identity_and_sequence_reproduce_the_same_seed_and_snapshot() {
        let identity = identity("one.mp4", 9, 2, 5);
        let seed = video_cycle_seed(&identity, 17);
        let first = sample_automatic_video_parameters(seed, &MediaEffectParams::default());
        let repeated = sample_automatic_video_parameters(seed, &MediaEffectParams::default());
        let different = sample_automatic_video_parameters(
            video_cycle_seed(&identity, 18),
            &MediaEffectParams::default(),
        );

        assert_eq!(first, repeated);
        assert_ne!(first, different);
    }

    #[test]
    fn sampler_matches_the_existing_typescript_fixture() {
        let snapshot = sample_automatic_video_parameters(20_260_825, &MediaEffectParams::default());

        assert_eq!(snapshot.video.brightness_percent, -0.123);
        assert_eq!(snapshot.video.saturation_percent, 99.773);
        assert_eq!(snapshot.video.rotation_degrees, -0.014);
        assert_eq!(snapshot.advanced.band_weights.get(&65), Some(&0.9972));
        assert_eq!(snapshot.advanced.target_frequency_hz, Some(590.0));
        assert_eq!(snapshot.advanced.asynchronous_rotation_max_degrees, 0.152);
    }

    #[test]
    fn unavailable_four_fields_keep_the_baseline() {
        let mut baseline = MediaEffectParams::default();
        baseline.video.color_space_conversion_strength_percent = 37.0;
        baseline.video.color_space_conversion_enabled = true;
        baseline.advanced.slice_min_length_ms = 12_300;
        baseline.advanced.picture_in_picture_timeline_locked = false;

        let snapshot = sample_automatic_video_parameters(20_260_825, &baseline);

        assert_eq!(snapshot.video.color_space_conversion_strength_percent, 37.0);
        assert!(snapshot.video.color_space_conversion_enabled);
        assert_eq!(snapshot.advanced.slice_min_length_ms, 12_300);
        assert!(!snapshot.advanced.picture_in_picture_timeline_locked);
        assert_ne!(
            snapshot.video.brightness_percent,
            baseline.video.brightness_percent
        );
    }

    #[test]
    fn replacement_source_fps_never_inherits_the_previous_source_value() {
        let mut controller = VideoCycleController::new(config(1_000));
        controller
            .replace_segment(identity("old.ts", 9, 0, 3), Some(22.001_848), 0)
            .expect("old segment starts");
        assert_eq!(
            controller
                .queue()
                .n_plus_1
                .as_ref()
                .and_then(|plan| plan.source_fps),
            Some(22.001_848)
        );

        controller
            .replace_segment(identity("new.mp4", 10, 0, 4), Some(30.0), 0)
            .expect("new segment starts");

        assert_eq!(controller.source_fps(), Some(30.0));
        assert_eq!(
            controller
                .queue()
                .n_plus_1
                .as_ref()
                .and_then(|plan| plan.source_fps),
            Some(30.0)
        );
        assert_eq!(
            controller
                .queue()
                .n_plus_2
                .as_ref()
                .and_then(|plan| plan.source_fps),
            Some(30.0)
        );
    }

    #[test]
    fn source_without_mpv_fps_stays_empty_until_an_actual_observation_arrives() {
        let mut controller = VideoCycleController::new(config(1_000));
        controller
            .replace_segment(identity("new.ts", 10, 0, 4), None, 0)
            .expect("source transition starts");
        assert!(controller.queue().n_plus_1.is_none());
        assert!(controller.queue().n_plus_2.is_none());

        assert_eq!(controller.observe_source(500, None, false), Ok(None));
        assert!(controller.queue().n_plus_1.is_none());
        controller
            .observe_source(500, Some(22.001_848), false)
            .expect("actual mpv fps");
        assert_eq!(controller.source_fps(), Some(22.001_848));
        assert!(controller.queue().n_plus_1.is_some());
        assert!(controller.queue().n_plus_2.is_some());
    }

    #[test]
    fn transport_stream_pts_regression_uses_six_frames_and_keeps_the_high_water_mark() {
        let source_fps = 22.001_848;
        let regression_tolerance_ms = ((6_000.0_f64 / source_fps).ceil() as u64 + 2).min(500);
        let high_water_pts_ms = 20_423;
        let mut controller = VideoCycleController::new(config(1_000));
        controller
            .handle(VideoCycleEvent::SourceBoundary {
                identity: identity("new.ts", 10, 0, 4),
                source_fps: Some(source_fps),
                source_pts_ms: 20_000,
            })
            .expect("TS source boundary");
        let first = controller
            .handle(VideoCycleEvent::MpvObservation {
                source_pts_ms: high_water_pts_ms,
                source_fps: Some(source_fps),
                paused: false,
            })
            .expect("monotonic TS observation");
        assert!(!first
            .iter()
            .any(|action| matches!(action, VideoCycleAction::Apply(_))));

        let tolerated = controller
            .handle(VideoCycleEvent::MpvObservation {
                source_pts_ms: 20_196,
                source_fps: Some(source_fps),
                paused: false,
            })
            .expect("the observed 227ms TS timestamp regression is tolerated");
        assert!(!tolerated
            .iter()
            .any(|action| matches!(action, VideoCycleAction::Apply(_))));
        assert!(matches!(
            controller.handle(VideoCycleEvent::MpvObservation {
                source_pts_ms: high_water_pts_ms - regression_tolerance_ms - 1,
                source_fps: Some(source_fps),
                paused: false,
            }),
            Err(VideoCycleError::PresentedPtsRegressed { .. })
        ));

        let resumed = controller
            .handle(VideoCycleEvent::MpvObservation {
                source_pts_ms: high_water_pts_ms + 1,
                source_fps: Some(source_fps),
                paused: false,
            })
            .expect("PTS above the high-water mark resumes normally");
        assert!(!resumed
            .iter()
            .any(|action| matches!(action, VideoCycleAction::Apply(_))));

        controller
            .handle(VideoCycleEvent::PlaybackIntent {
                paused: false,
                seek_source_pts_ms: Some(100),
                clock_epoch: 5,
            })
            .expect("explicit seek boundary accepts a lower PTS");
        controller
            .handle(VideoCycleEvent::SourceBoundary {
                identity: identity("new.ts", 10, 1, 6),
                source_fps: Some(source_fps),
                source_pts_ms: 0,
            })
            .expect("explicit loop/source boundary accepts a reset PTS");

        let low_fps = 1.0;
        let mut capped = VideoCycleController::new(config(1_000));
        capped
            .replace_segment(identity("low-fps.m2ts", 11, 0, 7), Some(low_fps), 1_000)
            .expect("low-fps source");
        capped
            .observe_source(500, Some(low_fps), false)
            .expect("transport-stream PTS regression tolerance is capped at 500ms");
        assert!(matches!(
            capped.observe_source(499, Some(low_fps), false),
            Err(VideoCycleError::PresentedPtsRegressed { .. })
        ));
    }

    #[test]
    fn non_transport_stream_pts_regression_keeps_the_two_frame_100ms_limit() {
        let source_fps = 30.0;
        let regression_tolerance_ms = ((2_000.0_f64 / source_fps).ceil() as u64 + 2).min(100);
        let high_water_pts_ms = 20_423;
        let mut controller = VideoCycleController::new(config(1_000));
        controller
            .replace_segment(identity("ordinary.mp4", 10, 0, 4), Some(source_fps), 20_000)
            .expect("MP4 source");

        controller
            .observe_source(high_water_pts_ms, Some(source_fps), false)
            .expect("monotonic MP4 observation");
        controller
            .observe_source(
                high_water_pts_ms - regression_tolerance_ms,
                Some(source_fps),
                false,
            )
            .expect("two MP4 source frames plus 2ms are tolerated");
        assert!(matches!(
            controller.observe_source(
                high_water_pts_ms - regression_tolerance_ms - 1,
                Some(source_fps),
                false,
            ),
            Err(VideoCycleError::PresentedPtsRegressed { .. })
        ));
    }

    #[test]
    fn hard_alignment_rebases_pts_without_clearing_cycle_state() {
        let source_fps = 30.0;
        let mut controller = VideoCycleController::new(config(1_000));
        controller
            .replace_segment(identity("ordinary.mp4", 12, 0, 8), Some(source_fps), 0)
            .expect("source starts");
        let first_sequence = controller
            .observe_source(1_100, Some(source_fps), false)
            .expect("first cycle observation")
            .expect("first plan is due")
            .sequence;
        for result in [
            VideoCycleApplyResult::Applying {
                fingerprint: "first-plan".to_owned(),
            },
            VideoCycleApplyResult::ReadbackConfirmed,
            VideoCycleApplyResult::PresentedConfirmed,
            VideoCycleApplyResult::Active,
        ] {
            controller
                .handle(VideoCycleEvent::ApplyResult {
                    sequence: first_sequence,
                    source_pts_ms: 1_100,
                    result,
                })
                .expect("activate the first plan");
        }
        let applying_sequence = controller
            .observe_source(2_100, Some(source_fps), false)
            .expect("second cycle observation")
            .expect("second plan is due")
            .sequence;
        controller
            .handle(VideoCycleEvent::ApplyResult {
                sequence: applying_sequence,
                source_pts_ms: 2_100,
                result: VideoCycleApplyResult::Applying {
                    fingerprint: "second-plan".to_owned(),
                },
            })
            .expect("second plan starts applying");
        controller
            .observe_source(3_533, Some(source_fps), false)
            .expect("high-water observation");
        assert!(controller.queue().n.is_some());
        assert!(controller.queue().n_plus_1.is_some());
        assert!(controller.queue().n_plus_2.is_some());
        let queue_before_rebase = controller.queue().clone();
        let apply_before_rebase = controller.apply_transaction().cloned();

        controller
            .rebase_source_pts(3_233)
            .expect("hard alignment rebases the same media identity");

        controller
            .observe_source(3_233, Some(source_fps), false)
            .expect("the rebased PTS is not reported as regressed");
        assert_eq!(controller.queue(), &queue_before_rebase);
        assert_eq!(controller.apply_transaction(), apply_before_rebase.as_ref());
        controller
            .observe_source(3_534, Some(source_fps), false)
            .expect("natural PTS progression continues after rebase");
    }

    #[test]
    fn seek_resets_all_slots_and_changes_clock_identity() {
        let mut controller = VideoCycleController::new(config(1_000));
        controller
            .replace_segment(identity("one.mp4", 9, 0, 3), Some(30.0), 0)
            .expect("segment starts");
        controller
            .observe_presented_pts(1_000, false)
            .expect("valid pts")
            .expect("plan is due");

        controller.reset_for_seek(4, 10_000).expect("seek resets");

        assert!(controller.queue().n.is_none());
        assert_eq!(
            controller
                .queue()
                .n_plus_1
                .as_ref()
                .map(|plan| plan.target_source_pts_ms),
            Some(11_000)
        );
        assert_eq!(
            controller
                .queue()
                .n_plus_1
                .as_ref()
                .map(|plan| plan.identity.clock_epoch),
            Some(4)
        );
    }

    #[test]
    fn event_boundary_covers_five_inputs_four_outputs_and_apply_transaction() {
        let mut controller = VideoCycleController::new(config(1_000));
        let source_actions = controller
            .handle(VideoCycleEvent::SourceBoundary {
                identity: identity("one.mp4", 9, 0, 3),
                source_fps: Some(30.0),
                source_pts_ms: 0,
            })
            .expect("source boundary");
        assert!(source_actions
            .iter()
            .any(|action| matches!(action, VideoCycleAction::Clear)));
        assert!(source_actions
            .iter()
            .any(|action| matches!(action, VideoCycleAction::PrepareNext(_))));
        assert!(source_actions
            .iter()
            .any(|action| matches!(action, VideoCycleAction::Publish(_))));

        let observation_actions = controller
            .handle(VideoCycleEvent::MpvObservation {
                source_pts_ms: 1_000,
                source_fps: Some(30.0),
                paused: false,
            })
            .expect("mpv observation");
        let sequence = observation_actions
            .iter()
            .find_map(|action| match action {
                VideoCycleAction::Apply(plan) => Some(plan.sequence),
                _ => None,
            })
            .expect("apply output");

        for result in [
            VideoCycleApplyResult::Applying {
                fingerprint: "0123456789abcdef".to_owned(),
            },
            VideoCycleApplyResult::ResultUnknown,
            VideoCycleApplyResult::Applying {
                fingerprint: "0123456789abcdef".to_owned(),
            },
            VideoCycleApplyResult::ReadbackConfirmed,
            VideoCycleApplyResult::PresentedConfirmed,
            VideoCycleApplyResult::Active,
        ] {
            controller
                .handle(VideoCycleEvent::ApplyResult {
                    sequence,
                    source_pts_ms: 1_000,
                    result,
                })
                .expect("apply transition");
        }
        let transaction = controller.apply_transaction().expect("transaction kept");
        assert_eq!(transaction.stage, VideoCycleApplyStage::Active);
        assert_eq!(transaction.retry_count, 1);
        assert!(transaction.result_unknown);
        assert_eq!(controller.confirmed_change_count(), 1);

        let playback_actions = controller
            .handle(VideoCycleEvent::PlaybackIntent {
                paused: false,
                seek_source_pts_ms: Some(10_000),
                clock_epoch: 4,
            })
            .expect("seek intent");
        assert!(playback_actions
            .iter()
            .any(|action| matches!(action, VideoCycleAction::Clear)));
        let previous_revision = controller.config_revision();
        controller
            .handle(VideoCycleEvent::Configure {
                config: Box::new(config(2_000)),
                current_source_pts_ms: 10_000,
            })
            .expect("configure input");
        assert!(controller.config_revision() > previous_revision);
    }
}
