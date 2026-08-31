//! PortAudio 可听时钟驱动的 mpv 视频同步决策器。
//!
//! 本模块不执行 IPC、不读取系统时钟。调用方负责校验媒体段身份与时长，并只提交
//! 同一源内坐标、同一代次、同一同步 epoch 的单调观测；控制器只返回有界速度、
//! 迟到帧策略或特殊边界的一次性对齐动作。

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvSyncConfig {
    /// 稳态允许的最大音画偏差。
    pub stable_drift_ms: i64,
    /// 超过此值时，视频落后可请求丢弃已经迟到的帧。
    pub moderate_drift_ms: i64,
    /// 超过此值时暂停新参数提交，并等待收敛或请求恢复。
    pub critical_drift_ms: i64,
    /// 相对基础速度允许的最大瞬时调整量。
    pub max_speed_delta: f64,
    /// 严重偏差持续多久后请求调用方建立恢复 epoch。
    pub recovery_request_after_ms: u64,
}

impl Default for AvSyncConfig {
    fn default() -> Self {
        Self {
            stable_drift_ms: 20,
            moderate_drift_ms: 60,
            critical_drift_ms: 80,
            max_speed_delta: 0.02,
            recovery_request_after_ms: 240,
        }
    }
}

impl AvSyncConfig {
    fn normalized(self) -> Self {
        let stable_drift_ms = self.stable_drift_ms.max(0);
        let moderate_drift_ms = self.moderate_drift_ms.max(stable_drift_ms);
        Self {
            stable_drift_ms,
            moderate_drift_ms,
            critical_drift_ms: self.critical_drift_ms.max(moderate_drift_ms),
            max_speed_delta: if self.max_speed_delta.is_finite() {
                self.max_speed_delta.clamp(0.0, 0.02)
            } else {
                Self::default().max_speed_delta
            },
            recovery_request_after_ms: self.recovery_request_after_ms.max(1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncBoundary {
    None,
    Startup,
    SourceChanged,
    UserSeek,
    LoopBoundary,
    Recovery,
    ClockAuthorityChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvSyncObservation {
    pub playback_generation: u64,
    pub sync_epoch: u64,
    /// 调用方提供的单调时间，仅用于判断持续偏差，不得使用墙钟。
    pub observed_at_ms: u64,
    /// 当前媒体文件内的 PortAudio 可听位置；身份无效或越界时调用方必须传 `None`。
    pub audible_source_pts_ms: Option<i64>,
    /// 当前媒体文件内的 mpv 已呈现位置；身份无效或越界时调用方必须传 `None`。
    pub mpv_source_pts_ms: Option<i64>,
    pub playing: bool,
    pub buffering: bool,
    pub seeking: bool,
    pub boundary: SyncBoundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreReason {
    StaleGeneration,
    StaleEpoch,
    OutOfOrderSample,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldReason {
    NotPlaying,
    Buffering,
    Seeking,
    MissingClock,
    AwaitingRecoveryEpoch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryReason {
    SustainedCriticalDrift,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardSeekReason {
    Startup,
    SourceChanged,
    UserSeek,
    LoopBoundary,
    Recovery,
    ClockAuthorityChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvSyncAction {
    Ignore {
        reason: IgnoreReason,
    },
    Hold {
        reason: HoldReason,
    },
    Maintain,
    AdjustSpeed,
    RequestRecovery {
        reason: RecoveryReason,
    },
    HardSeek {
        target_source_pts_ms: i64,
        reason: HardSeekReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvSyncDecision {
    pub action: AvSyncAction,
    /// `mpv_source_pts_ms - audible_source_pts_ms`；负值表示视频落后。
    pub drift_ms: Option<i64>,
    pub speed: f64,
    pub drop_late_frames: bool,
    pub pause_parameter_commits: bool,
}

impl AvSyncDecision {
    fn new(action: AvSyncAction, drift_ms: Option<i64>) -> Self {
        Self {
            action,
            drift_ms,
            speed: 1.0,
            drop_late_frames: false,
            pause_parameter_commits: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SyncIdentity {
    playback_generation: u64,
    sync_epoch: u64,
}

#[derive(Debug)]
pub struct AvSyncController {
    config: AvSyncConfig,
    identity: Option<SyncIdentity>,
    last_observed_at_ms: Option<u64>,
    critical_since_ms: Option<u64>,
    recovery_requested: bool,
    pending_hard_alignment: Option<HardSeekReason>,
}

impl Default for AvSyncController {
    fn default() -> Self {
        Self::new(AvSyncConfig::default())
    }
}

impl AvSyncController {
    pub fn new(config: AvSyncConfig) -> Self {
        Self {
            config: config.normalized(),
            identity: None,
            last_observed_at_ms: None,
            critical_since_ms: None,
            recovery_requested: false,
            pending_hard_alignment: None,
        }
    }

    /// 计算一次控制动作。
    ///
    /// 特殊边界应同时递增 `sync_epoch`；同一 epoch 重复携带边界不会再次触发硬定位。
    /// `RequestRecovery` 只请求调用方建立新的恢复 epoch，本方法不会在普通 epoch 内
    /// 直接执行或重复请求硬定位。
    pub fn decide(&mut self, observation: AvSyncObservation) -> AvSyncDecision {
        let incoming = SyncIdentity {
            playback_generation: observation.playback_generation,
            sync_epoch: observation.sync_epoch,
        };
        if let Some(reason) = self.stale_reason(incoming) {
            return AvSyncDecision::new(AvSyncAction::Ignore { reason }, None);
        }

        if self
            .last_observed_at_ms
            .is_some_and(|last| observation.observed_at_ms < last)
        {
            return AvSyncDecision::new(
                AvSyncAction::Ignore {
                    reason: IgnoreReason::OutOfOrderSample,
                },
                None,
            );
        }

        let identity_changed = self.identity != Some(incoming);
        if identity_changed {
            let first_identity = self.identity.is_none();
            self.identity = Some(incoming);
            self.critical_since_ms = None;
            self.recovery_requested = false;
            self.pending_hard_alignment = hard_seek_reason(observation.boundary)
                .or(first_identity.then_some(HardSeekReason::Startup));
        }
        self.last_observed_at_ms = Some(observation.observed_at_ms);

        if observation.seeking {
            return self.hold(HoldReason::Seeking);
        }
        if observation.buffering {
            return self.hold(HoldReason::Buffering);
        }
        if !observation.playing {
            return self.hold(HoldReason::NotPlaying);
        }

        let (Some(audio_source_pts_ms), Some(mpv_source_pts_ms)) = (
            observation.audible_source_pts_ms,
            observation.mpv_source_pts_ms,
        ) else {
            return self.hold(HoldReason::MissingClock);
        };
        let drift_ms = mpv_source_pts_ms.saturating_sub(audio_source_pts_ms);
        let absolute_drift_ms = drift_ms.saturating_abs();

        if let Some(reason) = self.pending_hard_alignment.take() {
            let would_seek_backward = drift_ms > 0
                && matches!(
                    reason,
                    HardSeekReason::Recovery | HardSeekReason::ClockAuthorityChanged
                );
            if absolute_drift_ms > self.config.stable_drift_ms && !would_seek_backward {
                self.critical_since_ms = None;
                self.recovery_requested = false;
                return AvSyncDecision::new(
                    AvSyncAction::HardSeek {
                        target_source_pts_ms: audio_source_pts_ms,
                        reason,
                    },
                    Some(drift_ms),
                );
            }
        }

        if absolute_drift_ms <= self.config.stable_drift_ms {
            self.critical_since_ms = None;
            self.recovery_requested = false;
            return AvSyncDecision::new(AvSyncAction::Maintain, Some(drift_ms));
        }

        if absolute_drift_ms >= self.config.critical_drift_ms && drift_ms < 0 {
            if self.recovery_requested {
                let mut decision = AvSyncDecision::new(
                    AvSyncAction::Hold {
                        reason: HoldReason::AwaitingRecoveryEpoch,
                    },
                    Some(drift_ms),
                );
                decision.pause_parameter_commits = true;
                return decision;
            }
            let critical_since_ms = *self
                .critical_since_ms
                .get_or_insert(observation.observed_at_ms);
            if observation.observed_at_ms.saturating_sub(critical_since_ms)
                >= self.config.recovery_request_after_ms
            {
                self.recovery_requested = true;
                let mut decision = AvSyncDecision::new(
                    AvSyncAction::RequestRecovery {
                        reason: RecoveryReason::SustainedCriticalDrift,
                    },
                    Some(drift_ms),
                );
                decision.pause_parameter_commits = true;
                return decision;
            }
        } else {
            self.critical_since_ms = None;
            self.recovery_requested = false;
        }

        let correction_delta = if absolute_drift_ms > self.config.moderate_drift_ms {
            self.config.max_speed_delta
        } else {
            self.config.max_speed_delta / 2.0
        };
        let mut decision = AvSyncDecision::new(AvSyncAction::AdjustSpeed, Some(drift_ms));
        decision.speed = if drift_ms < 0 {
            1.0 + correction_delta
        } else {
            1.0 - correction_delta
        };
        decision.drop_late_frames = drift_ms < -self.config.moderate_drift_ms;
        decision.pause_parameter_commits =
            absolute_drift_ms >= self.config.critical_drift_ms && drift_ms < 0;
        decision
    }

    fn stale_reason(&self, incoming: SyncIdentity) -> Option<IgnoreReason> {
        let current = self.identity?;
        if incoming.playback_generation < current.playback_generation {
            Some(IgnoreReason::StaleGeneration)
        } else if incoming.playback_generation == current.playback_generation
            && incoming.sync_epoch < current.sync_epoch
        {
            Some(IgnoreReason::StaleEpoch)
        } else {
            None
        }
    }

    fn hold(&mut self, reason: HoldReason) -> AvSyncDecision {
        self.critical_since_ms = None;
        AvSyncDecision::new(AvSyncAction::Hold { reason }, None)
    }
}

/// 将 PortAudio 实际播放速率、视频调度速度与音画同步修正合并为唯一 mpv speed。
pub fn compose_video_speed(
    audio_playback_rate: f64,
    scheduler_base_speed: f64,
    sync_correction: f64,
) -> Option<f64> {
    if !audio_playback_rate.is_finite()
        || !(0.5..=2.0).contains(&audio_playback_rate)
        || !scheduler_base_speed.is_finite()
        || !(0.5..=1.5).contains(&scheduler_base_speed)
        || !sync_correction.is_finite()
        || !(0.98..=1.02).contains(&sync_correction)
    {
        return None;
    }
    Some((audio_playback_rate * scheduler_base_speed * sync_correction).clamp(0.25, 4.0))
}

fn hard_seek_reason(boundary: SyncBoundary) -> Option<HardSeekReason> {
    match boundary {
        SyncBoundary::None => None,
        SyncBoundary::Startup => Some(HardSeekReason::Startup),
        SyncBoundary::SourceChanged => Some(HardSeekReason::SourceChanged),
        SyncBoundary::UserSeek => Some(HardSeekReason::UserSeek),
        SyncBoundary::LoopBoundary => Some(HardSeekReason::LoopBoundary),
        SyncBoundary::Recovery => Some(HardSeekReason::Recovery),
        SyncBoundary::ClockAuthorityChanged => Some(HardSeekReason::ClockAuthorityChanged),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(drift_ms: i64, observed_at_ms: u64) -> AvSyncObservation {
        AvSyncObservation {
            playback_generation: 7,
            sync_epoch: 3,
            observed_at_ms,
            audible_source_pts_ms: Some(10_000),
            mpv_source_pts_ms: Some(10_000 + drift_ms),
            playing: true,
            buffering: false,
            seeking: false,
            boundary: SyncBoundary::None,
        }
    }

    fn running_controller() -> AvSyncController {
        let mut controller = AvSyncController::default();
        let decision = controller.decide(observation(0, 0));
        assert_eq!(decision.action, AvSyncAction::Maintain);
        controller
    }

    #[test]
    fn threshold_table_uses_bounded_speed_and_late_frame_policy() {
        struct Case {
            drift_ms: i64,
            expected_action: AvSyncAction,
            speed_relation: std::cmp::Ordering,
            drop_late_frames: bool,
            pause_parameter_commits: bool,
        }

        let cases = [
            Case {
                drift_ms: -20,
                expected_action: AvSyncAction::Maintain,
                speed_relation: std::cmp::Ordering::Equal,
                drop_late_frames: false,
                pause_parameter_commits: false,
            },
            Case {
                drift_ms: 20,
                expected_action: AvSyncAction::Maintain,
                speed_relation: std::cmp::Ordering::Equal,
                drop_late_frames: false,
                pause_parameter_commits: false,
            },
            Case {
                drift_ms: -21,
                expected_action: AvSyncAction::AdjustSpeed,
                speed_relation: std::cmp::Ordering::Greater,
                drop_late_frames: false,
                pause_parameter_commits: false,
            },
            Case {
                drift_ms: 60,
                expected_action: AvSyncAction::AdjustSpeed,
                speed_relation: std::cmp::Ordering::Less,
                drop_late_frames: false,
                pause_parameter_commits: false,
            },
            Case {
                drift_ms: -61,
                expected_action: AvSyncAction::AdjustSpeed,
                speed_relation: std::cmp::Ordering::Greater,
                drop_late_frames: true,
                pause_parameter_commits: false,
            },
            Case {
                drift_ms: 79,
                expected_action: AvSyncAction::AdjustSpeed,
                speed_relation: std::cmp::Ordering::Less,
                drop_late_frames: false,
                pause_parameter_commits: false,
            },
            Case {
                drift_ms: -80,
                expected_action: AvSyncAction::AdjustSpeed,
                speed_relation: std::cmp::Ordering::Greater,
                drop_late_frames: true,
                pause_parameter_commits: true,
            },
            Case {
                drift_ms: 80,
                expected_action: AvSyncAction::AdjustSpeed,
                speed_relation: std::cmp::Ordering::Less,
                drop_late_frames: false,
                pause_parameter_commits: false,
            },
        ];

        for case in cases {
            let mut controller = running_controller();
            let decision = controller.decide(observation(case.drift_ms, 40));
            assert_eq!(
                decision.action, case.expected_action,
                "drift={}",
                case.drift_ms
            );
            assert_eq!(
                decision.speed.total_cmp(&1.0),
                case.speed_relation,
                "drift={}",
                case.drift_ms
            );
            assert_eq!(decision.drop_late_frames, case.drop_late_frames);
            assert_eq!(
                decision.pause_parameter_commits,
                case.pause_parameter_commits
            );
            let expected_delta = if case.drift_ms.unsigned_abs() > 60 {
                0.02
            } else if case.drift_ms.unsigned_abs() > 20 {
                0.01
            } else {
                0.0
            };
            assert!(((decision.speed - 1.0).abs() - expected_delta).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn sustained_critical_drift_requests_a_new_recovery_epoch_once() {
        let mut controller = running_controller();

        let converging = controller.decide(observation(-100, 40));
        assert_eq!(converging.action, AvSyncAction::AdjustSpeed);
        assert!(converging.pause_parameter_commits);

        let request = controller.decide(observation(-100, 280));
        assert_eq!(
            request.action,
            AvSyncAction::RequestRecovery {
                reason: RecoveryReason::SustainedCriticalDrift,
            }
        );
        assert!(request.pause_parameter_commits);

        let mut buffering = observation(-100, 300);
        buffering.buffering = true;
        assert_eq!(
            controller.decide(buffering).action,
            AvSyncAction::Hold {
                reason: HoldReason::Buffering,
            }
        );

        let duplicate = controller.decide(observation(-100, 320));
        assert_eq!(
            duplicate.action,
            AvSyncAction::Hold {
                reason: HoldReason::AwaitingRecoveryEpoch,
            }
        );
    }

    #[test]
    fn special_boundary_allows_only_one_hard_alignment_per_epoch() {
        let mut controller = running_controller();
        let mut recovery = observation(-120, 400);
        recovery.sync_epoch = 4;
        recovery.boundary = SyncBoundary::Recovery;

        let first = controller.decide(recovery);
        assert_eq!(
            first.action,
            AvSyncAction::HardSeek {
                target_source_pts_ms: 10_000,
                reason: HardSeekReason::Recovery,
            }
        );

        recovery.observed_at_ms = 440;
        recovery.boundary = SyncBoundary::None;
        let second = controller.decide(recovery);
        assert_ne!(second.action, first.action);
        assert!(!matches!(second.action, AvSyncAction::HardSeek { .. }));
    }

    #[test]
    fn recovery_never_seeks_backward_to_an_audio_clock_behind_video() {
        let mut controller = running_controller();
        let mut recovery = observation(3_000, 400);
        recovery.sync_epoch = 4;
        recovery.boundary = SyncBoundary::Recovery;

        let decision = controller.decide(recovery);

        assert_eq!(decision.action, AvSyncAction::AdjustSpeed);
        assert!(decision.speed < 1.0);
        assert!(!decision.pause_parameter_commits);
    }

    #[test]
    fn first_startup_with_large_drift_aligns_once() {
        let mut controller = AvSyncController::default();
        let first = controller.decide(observation(100, 0));
        assert_eq!(
            first.action,
            AvSyncAction::HardSeek {
                target_source_pts_ms: 10_000,
                reason: HardSeekReason::Startup,
            }
        );

        let second = controller.decide(observation(100, 40));
        assert_eq!(second.action, AvSyncAction::AdjustSpeed);
    }

    #[test]
    fn paused_buffering_and_seeking_do_not_accumulate_critical_time() {
        for blocked in [
            (false, false, false, HoldReason::NotPlaying),
            (true, true, false, HoldReason::Buffering),
            (true, false, true, HoldReason::Seeking),
        ] {
            let mut controller = running_controller();
            let mut input = observation(-120, 40);
            input.playing = blocked.0;
            input.buffering = blocked.1;
            input.seeking = blocked.2;
            let held = controller.decide(input);
            assert_eq!(held.action, AvSyncAction::Hold { reason: blocked.3 });
            assert_eq!(held.speed, 1.0);
            assert!(!held.drop_late_frames);

            let resumed = controller.decide(observation(-120, 1_000));
            assert_eq!(resumed.action, AvSyncAction::AdjustSpeed);
        }
    }

    #[test]
    fn stale_epoch_and_out_of_order_samples_cannot_mutate_current_control() {
        let mut controller = running_controller();
        let mut next_epoch = observation(0, 100);
        next_epoch.sync_epoch = 4;
        next_epoch.boundary = SyncBoundary::UserSeek;
        assert_eq!(controller.decide(next_epoch).action, AvSyncAction::Maintain);

        let mut stale = observation(-200, 1_000);
        stale.sync_epoch = 3;
        assert_eq!(
            controller.decide(stale).action,
            AvSyncAction::Ignore {
                reason: IgnoreReason::StaleEpoch,
            }
        );

        let mut out_of_order = observation(-200, 90);
        out_of_order.sync_epoch = 4;
        assert_eq!(
            controller.decide(out_of_order).action,
            AvSyncAction::Ignore {
                reason: IgnoreReason::OutOfOrderSample,
            }
        );

        let current = controller.decide({
            let mut value = observation(0, 140);
            value.sync_epoch = 4;
            value
        });
        assert_eq!(current.action, AvSyncAction::Maintain);

        let mut stale_generation = observation(-200, 1_000);
        stale_generation.playback_generation = 6;
        stale_generation.sync_epoch = 99;
        assert_eq!(
            controller.decide(stale_generation).action,
            AvSyncAction::Ignore {
                reason: IgnoreReason::StaleGeneration,
            }
        );
    }

    #[test]
    fn out_of_order_new_epoch_does_not_replace_the_current_identity() {
        let mut controller = running_controller();
        assert_eq!(
            controller.decide(observation(0, 100)).action,
            AvSyncAction::Maintain
        );

        let mut out_of_order_next_epoch = observation(-200, 90);
        out_of_order_next_epoch.sync_epoch = 4;
        out_of_order_next_epoch.boundary = SyncBoundary::UserSeek;
        assert_eq!(
            controller.decide(out_of_order_next_epoch).action,
            AvSyncAction::Ignore {
                reason: IgnoreReason::OutOfOrderSample,
            }
        );

        assert_eq!(
            controller.decide(observation(0, 140)).action,
            AvSyncAction::Maintain
        );
    }

    #[test]
    fn missing_clock_holds_base_speed_without_false_drift() {
        let mut controller = running_controller();
        let mut missing_audio = observation(0, 40);
        missing_audio.audible_source_pts_ms = None;
        let decision = controller.decide(missing_audio);
        assert_eq!(
            decision.action,
            AvSyncAction::Hold {
                reason: HoldReason::MissingClock,
            }
        );
        assert_eq!(decision.drift_ms, None);
        assert_eq!(decision.speed, 1.0);
    }

    #[test]
    fn speed_property_holds_for_the_supported_drift_domain() {
        let config = AvSyncConfig::default();
        for drift_ms in -2_000..=2_000 {
            let mut controller = running_controller();
            let decision = controller.decide(observation(drift_ms, 40));
            assert!(decision.speed.is_finite());
            assert!(decision.speed >= 1.0 - config.max_speed_delta);
            assert!(decision.speed <= 1.0 + config.max_speed_delta);
            if drift_ms < -config.stable_drift_ms {
                assert!(decision.speed > 1.0);
            } else if drift_ms > config.stable_drift_ms {
                assert!(decision.speed < 1.0);
            } else {
                assert_eq!(decision.speed, 1.0);
            }
            assert!(!matches!(decision.action, AvSyncAction::HardSeek { .. }));
        }
    }

    #[test]
    fn synchronization_uses_one_and_two_percent_bands() {
        let mut controller = running_controller();
        assert_eq!(controller.decide(observation(-21, 40)).speed, 1.01);
        assert_eq!(controller.decide(observation(60, 80)).speed, 0.99);
        assert_eq!(controller.decide(observation(-61, 120)).speed, 1.02);
        assert_eq!(controller.decide(observation(80, 160)).speed, 0.98);
    }

    #[test]
    fn audio_scheduler_and_sync_speed_share_one_bounded_composition_point() {
        assert_eq!(compose_video_speed(1.2, 1.0, 1.01), Some(1.212));
        assert_eq!(compose_video_speed(2.0, 1.5, 1.02), Some(3.06));
        assert_eq!(compose_video_speed(0.5, 0.5, 0.98), Some(0.25));
        assert_eq!(compose_video_speed(f64::NAN, 1.0, 1.0), None);
        assert_eq!(compose_video_speed(0.49, 1.0, 1.0), None);
        assert_eq!(compose_video_speed(2.01, 1.0, 1.0), None);
        assert_eq!(compose_video_speed(1.0, 0.49, 1.0), None);
        assert_eq!(compose_video_speed(1.0, 1.51, 1.0), None);
        assert_eq!(compose_video_speed(1.0, 1.0, 0.97), None);
        assert_eq!(compose_video_speed(1.0, 1.0, 1.03), None);
        assert_eq!(compose_video_speed(1.0, 1.0, f64::INFINITY), None);
    }

    #[test]
    fn twentieth_loop_uses_source_pts_instead_of_presentation_pts() {
        let source_duration_ms = 72_300_i64;
        let loop_index = 20_i64;
        let audible_source_pts_ms = 27_000_i64;
        let audible_presentation_pts_ms = loop_index * source_duration_ms + audible_source_pts_ms;
        assert_eq!(audible_presentation_pts_ms, 1_473_000);

        let mut controller = running_controller();
        let decision = controller.decide(AvSyncObservation {
            playback_generation: 7,
            sync_epoch: 4,
            observed_at_ms: 40,
            audible_source_pts_ms: Some(audible_source_pts_ms),
            mpv_source_pts_ms: Some(27_100),
            playing: true,
            buffering: false,
            seeking: false,
            boundary: SyncBoundary::LoopBoundary,
        });

        assert_eq!(decision.drift_ms, Some(100));
        assert_eq!(
            decision.action,
            AvSyncAction::HardSeek {
                target_source_pts_ms: audible_source_pts_ms,
                reason: HardSeekReason::LoopBoundary,
            }
        );
    }

    #[test]
    fn caller_rejects_invalid_segment_identity_by_withholding_source_clock() {
        let mut controller = running_controller();
        let decision = controller.decide(AvSyncObservation {
            playback_generation: 8,
            sync_epoch: 4,
            observed_at_ms: 40,
            audible_source_pts_ms: None,
            mpv_source_pts_ms: Some(27_000),
            playing: true,
            buffering: false,
            seeking: false,
            boundary: SyncBoundary::SourceChanged,
        });

        assert_eq!(
            decision.action,
            AvSyncAction::Hold {
                reason: HoldReason::MissingClock,
            }
        );
        assert_eq!(decision.drift_ms, None);
    }

    #[test]
    fn paused_user_seek_boundary_never_emits_a_hard_seek() {
        let mut controller = running_controller();
        let decision = controller.decide(AvSyncObservation {
            playback_generation: 7,
            sync_epoch: 4,
            observed_at_ms: 40,
            audible_source_pts_ms: Some(27_000),
            mpv_source_pts_ms: Some(27_100),
            playing: false,
            buffering: false,
            seeking: false,
            boundary: SyncBoundary::UserSeek,
        });

        assert_eq!(
            decision.action,
            AvSyncAction::Hold {
                reason: HoldReason::NotPlaying,
            }
        );
        assert_eq!(decision.speed, 1.0);
        assert!(!matches!(decision.action, AvSyncAction::HardSeek { .. }));
    }

    #[test]
    fn hard_seek_is_limited_to_explicit_alignment_boundaries() {
        for (boundary, reason) in [
            (SyncBoundary::Startup, HardSeekReason::Startup),
            (SyncBoundary::SourceChanged, HardSeekReason::SourceChanged),
            (SyncBoundary::UserSeek, HardSeekReason::UserSeek),
            (SyncBoundary::LoopBoundary, HardSeekReason::LoopBoundary),
        ] {
            let mut controller = running_controller();
            let mut input = observation(100, 40);
            input.sync_epoch = 4;
            input.boundary = boundary;

            assert_eq!(
                controller.decide(input).action,
                AvSyncAction::HardSeek {
                    target_source_pts_ms: 10_000,
                    reason,
                }
            );
        }

        let mut controller = running_controller();
        assert!(!matches!(
            controller.decide(observation(100, 40)).action,
            AvSyncAction::HardSeek { .. }
        ));

        for boundary in [SyncBoundary::Recovery, SyncBoundary::ClockAuthorityChanged] {
            let mut controller = running_controller();
            let mut input = observation(100, 40);
            input.sync_epoch = 4;
            input.boundary = boundary;
            assert!(!matches!(
                controller.decide(input).action,
                AvSyncAction::HardSeek { .. }
            ));
        }
    }
}
