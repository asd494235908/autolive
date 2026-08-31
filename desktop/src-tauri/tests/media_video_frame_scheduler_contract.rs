use autolive_desktop_core::media_effect_params::MediaEffectParams;
use autolive_desktop_core::media_video_frame_scheduler::{
    VideoFrameScheduleObservation, VideoFrameScheduler, VideoScheduleAction, VideoScheduleBoundary,
    VideoScheduleIgnoreReason,
};
use autolive_desktop_core::realtime_video_backend::VideoPlanIdentity;

const SOURCE_FPS: f64 = 60.0;
const SEED: u64 = 41;

fn identity(playback_generation: u64, sequence: u64) -> VideoPlanIdentity {
    VideoPlanIdentity {
        session_id: 11,
        playback_generation,
        source_revision: 13,
        parameter_revision: 17,
        sequence,
    }
}

fn observation<'a>(
    identity: &'a VideoPlanIdentity,
    params: &'a MediaEffectParams,
    schedule_epoch: u64,
    media_pts_ms: u64,
    paused: bool,
    boundary: VideoScheduleBoundary,
    seed: u64,
) -> VideoFrameScheduleObservation<'a> {
    VideoFrameScheduleObservation {
        identity: identity.clone(),
        schedule_epoch,
        media_pts_ms,
        source_fps: SOURCE_FPS,
        paused,
        boundary,
        seed,
        video_params: &params.video,
        advanced_params: &params.advanced,
    }
}

fn randomized_params() -> MediaEffectParams {
    let mut params = MediaEffectParams::default();
    params.video.frame_rate_jitter_percent = 2.0;
    params.video.frame_rate_perturbation_amplitude_fps = 2.0;
    params.video.frame_rate_perturbation_frequency_hz = 2.0;
    params.video.frame_inner_perturbation_percent = 2.0;
    params.video.frame_inter_perturbation_percent = 20.0;
    params.advanced.frame_perturbation_probability_percent = 20.0;
    params.advanced.picture_in_picture_enabled = true;
    params.advanced.picture_in_picture_pixel_jitter_px = 4.0;
    params.advanced.asynchronous_rotation_enabled = true;
    params.advanced.asynchronous_rotation_min_degrees = -15.0;
    params.advanced.asynchronous_rotation_max_degrees = 15.0;
    params
}

#[test]
fn identity_seed_and_pts_are_deterministic_but_sequence_or_seed_changes_decisions() {
    let params = randomized_params();
    let base_identity = identity(5, 29);
    let mut first = VideoFrameScheduler::new();
    let mut replay = VideoFrameScheduler::new();
    let input = || {
        observation(
            &base_identity,
            &params,
            7,
            1_234,
            false,
            VideoScheduleBoundary::Startup,
            SEED,
        )
    };
    let expected = first.decide(input()).expect("first schedule");
    assert_eq!(replay.decide(input()).expect("replayed schedule"), expected);

    let expected = expected.schedule.expect("applied schedule");
    let expected_signature = (
        expected.frame_inner_active,
        expected.frame_inter_active,
        expected.frame_probability_active,
        expected.pip_jitter_x_px,
        expected.pip_jitter_y_px,
    );

    for (changed_identity, seed, label) in [
        (identity(5, 30), SEED, "sequence"),
        (base_identity.clone(), SEED + 1, "seed"),
    ] {
        let mut changed = VideoFrameScheduler::new();
        let schedule = changed
            .decide(observation(
                &changed_identity,
                &params,
                7,
                1_234,
                false,
                VideoScheduleBoundary::Startup,
                seed,
            ))
            .expect("changed schedule")
            .schedule
            .expect("applied schedule");
        assert_ne!(
            (
                schedule.frame_inner_active,
                schedule.frame_inter_active,
                schedule.frame_probability_active,
                schedule.pip_jitter_x_px,
                schedule.pip_jitter_y_px,
            ),
            expected_signature,
            "{label} must participate in probability or offset decisions"
        );
    }
}

#[test]
fn pause_holds_same_pts_rejects_progress_and_resume_continues_same_epoch() {
    let params = randomized_params();
    let plan = identity(5, 29);
    let mut scheduler = VideoFrameScheduler::new();
    let first = scheduler
        .decide(observation(
            &plan,
            &params,
            7,
            1_000,
            false,
            VideoScheduleBoundary::Startup,
            SEED,
        ))
        .expect("startup")
        .schedule
        .expect("startup schedule");
    assert_eq!((first.epoch_elapsed_ms, first.frame_index), (0, 0));

    let paused = || {
        observation(
            &plan,
            &params,
            7,
            1_000,
            true,
            VideoScheduleBoundary::None,
            SEED,
        )
    };
    let first_hold = scheduler.decide(paused()).expect("first hold");
    let repeated_hold = scheduler.decide(paused()).expect("repeated hold");
    assert_eq!(first_hold.action, VideoScheduleAction::Hold);
    assert_eq!(repeated_hold, first_hold);

    let invalid_progress = scheduler
        .decide(observation(
            &plan,
            &params,
            7,
            1_001,
            true,
            VideoScheduleBoundary::None,
            SEED,
        ))
        .expect("structured rejection");
    assert_eq!(
        invalid_progress.action,
        VideoScheduleAction::Ignore {
            reason: VideoScheduleIgnoreReason::PausedPtsAdvanced,
        }
    );

    assert_eq!(
        scheduler
            .decide(observation(
                &plan,
                &params,
                7,
                1_000,
                false,
                VideoScheduleBoundary::None,
                SEED,
            ))
            .expect("resume at held frame")
            .action,
        VideoScheduleAction::Hold
    );
    let resumed = scheduler
        .decide(observation(
            &plan,
            &params,
            7,
            1_017,
            false,
            VideoScheduleBoundary::None,
            SEED,
        ))
        .expect("resumed schedule")
        .schedule
        .expect("applied schedule");
    assert_eq!((resumed.epoch_elapsed_ms, resumed.frame_index), (17, 1));
}

#[test]
fn seek_and_loop_boundaries_start_new_epochs_at_zero() {
    let params = randomized_params();
    let plan = identity(5, 29);
    let mut scheduler = VideoFrameScheduler::new();
    scheduler
        .decide(observation(
            &plan,
            &params,
            7,
            8_000,
            false,
            VideoScheduleBoundary::Startup,
            SEED,
        ))
        .expect("startup");
    let advanced = scheduler
        .decide(observation(
            &plan,
            &params,
            7,
            8_017,
            false,
            VideoScheduleBoundary::None,
            SEED,
        ))
        .expect("advanced schedule")
        .schedule
        .expect("applied schedule");
    assert_eq!((advanced.epoch_elapsed_ms, advanced.frame_index), (17, 1));

    for (epoch, pts, boundary) in [
        (8, 2_000, VideoScheduleBoundary::UserSeek),
        (9, 0, VideoScheduleBoundary::LoopBoundary),
    ] {
        let reset = scheduler
            .decide(observation(
                &plan, &params, epoch, pts, false, boundary, SEED,
            ))
            .expect("boundary reset")
            .schedule
            .expect("applied schedule");
        assert_eq!((reset.epoch_elapsed_ms, reset.frame_index), (0, 0));
    }
}

#[test]
fn rejected_observations_do_not_pollute_the_next_legal_sample() {
    let params = randomized_params();
    let plan = identity(5, 29);
    let mut scheduler = VideoFrameScheduler::new();
    let mut clean = VideoFrameScheduler::new();
    for controller in [&mut scheduler, &mut clean] {
        controller
            .decide(observation(
                &plan,
                &params,
                7,
                1_000,
                false,
                VideoScheduleBoundary::Startup,
                SEED,
            ))
            .expect("startup");
    }

    let stale_generation = identity(4, 29);
    let stale_sequence = identity(5, 28);
    for (incoming, epoch, pts, reason) in [
        (
            &stale_generation,
            99,
            9_999,
            VideoScheduleIgnoreReason::StaleGeneration,
        ),
        (&plan, 6, 9_999, VideoScheduleIgnoreReason::StaleEpoch),
        (
            &stale_sequence,
            7,
            9_999,
            VideoScheduleIgnoreReason::StaleSequence,
        ),
        (&plan, 7, 999, VideoScheduleIgnoreReason::OutOfOrderPts),
        (
            &plan,
            8,
            2_000,
            VideoScheduleIgnoreReason::EpochWithoutBoundary,
        ),
    ] {
        let rejected = scheduler
            .decide(observation(
                incoming,
                &params,
                epoch,
                pts,
                false,
                VideoScheduleBoundary::None,
                SEED,
            ))
            .expect("structured rejection");
        assert_eq!(rejected.action, VideoScheduleAction::Ignore { reason });
        assert!(rejected.schedule.is_none());
    }

    let legal = |controller: &mut VideoFrameScheduler| {
        controller
            .decide(observation(
                &plan,
                &params,
                7,
                1_017,
                false,
                VideoScheduleBoundary::None,
                SEED,
            ))
            .expect("legal sample")
    };
    assert_eq!(legal(&mut scheduler), legal(&mut clean));
}

#[test]
fn a_new_parameter_sequence_applies_without_advancing_pts() {
    let params = randomized_params();
    let first_plan = identity(5, 29);
    let next_plan = identity(5, 30);
    let mut scheduler = VideoFrameScheduler::new();
    scheduler
        .decide(observation(
            &first_plan,
            &params,
            7,
            1_000,
            false,
            VideoScheduleBoundary::Startup,
            SEED,
        ))
        .expect("first sequence");

    let next = scheduler
        .decide(observation(
            &next_plan,
            &params,
            7,
            1_000,
            false,
            VideoScheduleBoundary::None,
            SEED,
        ))
        .expect("new sequence at the same pts");
    assert_eq!(next.action, VideoScheduleAction::Apply);
    assert_eq!(
        next.schedule.expect("applied schedule").identity.sequence,
        30
    );

    assert_eq!(
        scheduler
            .decide(observation(
                &first_plan,
                &params,
                7,
                1_000,
                false,
                VideoScheduleBoundary::None,
                SEED,
            ))
            .expect("old sequence rejection")
            .action,
        VideoScheduleAction::Ignore {
            reason: VideoScheduleIgnoreReason::StaleSequence,
        }
    );
}

#[test]
fn slice_local_blur_and_highlight_use_half_open_interval_boundaries() {
    let mut params = MediaEffectParams::default();
    params.advanced.slice_length_ms = 500;
    params.advanced.slice_trigger_interval_ms = 5_000;
    params.advanced.local_blur_enabled = true;
    params.advanced.local_blur_interval_ms = 1_000;
    params.advanced.highlight_perturbation_enabled = true;
    params.advanced.highlight_perturbation_interval_ms = 1_000;
    let plan = identity(5, 29);
    let mut scheduler = VideoFrameScheduler::new();

    for (pts_ms, expected) in [
        (0, (true, true, true)),
        (49, (true, true, true)),
        (50, (true, true, false)),
        (499, (true, true, false)),
        (500, (false, false, false)),
        (999, (false, false, false)),
        (1_000, (false, true, true)),
        (1_049, (false, true, true)),
        (1_050, (false, true, false)),
        (1_499, (false, true, false)),
        (1_500, (false, false, false)),
        (4_999, (false, false, false)),
        (5_000, (true, true, true)),
        (5_499, (true, true, false)),
        (5_500, (false, false, false)),
    ] {
        let boundary = if pts_ms == 0 {
            VideoScheduleBoundary::Startup
        } else {
            VideoScheduleBoundary::None
        };
        let schedule = scheduler
            .decide(observation(
                &plan, &params, 7, pts_ms, false, boundary, SEED,
            ))
            .expect("interval sample")
            .schedule
            .expect("applied schedule");
        assert_eq!(
            (
                schedule.slice_active,
                schedule.local_blur_active,
                schedule.highlight_active,
            ),
            expected,
            "pts={pts_ms}"
        );
    }
}

#[test]
fn speed_is_finite_bounded_and_neutral_without_perturbation() {
    let plan = identity(5, 29);
    let neutral_params = MediaEffectParams::default();
    let mut neutral = VideoFrameScheduler::new();
    for (pts_ms, boundary) in [
        (0, VideoScheduleBoundary::Startup),
        (1_000, VideoScheduleBoundary::None),
    ] {
        let speed = neutral
            .decide(observation(
                &plan,
                &neutral_params,
                7,
                pts_ms,
                false,
                boundary,
                SEED,
            ))
            .expect("neutral sample")
            .schedule
            .expect("applied schedule")
            .base_video_speed;
        assert_eq!(speed, 1.0);
    }

    let params = randomized_params();
    let mut perturbed = VideoFrameScheduler::new();
    for pts_ms in (0..=10_000).step_by(16) {
        let boundary = if pts_ms == 0 {
            VideoScheduleBoundary::Startup
        } else {
            VideoScheduleBoundary::None
        };
        let speed = perturbed
            .decide(observation(
                &plan, &params, 7, pts_ms, false, boundary, SEED,
            ))
            .expect("perturbed sample")
            .schedule
            .expect("applied schedule")
            .base_video_speed;
        assert!(speed.is_finite(), "pts={pts_ms}, speed={speed}");
        assert!((0.5..=1.5).contains(&speed), "pts={pts_ms}, speed={speed}");
    }
}

#[test]
fn unrelated_audio_validation_cannot_block_video_scheduling() {
    let mut params = MediaEffectParams::default();
    params.audio.playback_speed = 99.0;
    assert!(params.audio.validate().is_err());

    let plan = identity(5, 29);
    let decision = VideoFrameScheduler::new()
        .decide(observation(
            &plan,
            &params,
            7,
            0,
            false,
            VideoScheduleBoundary::Startup,
            SEED,
        ))
        .expect("video scheduler validates only video domains");
    assert_eq!(decision.action, VideoScheduleAction::Apply);
}

#[test]
fn output_is_independent_of_system_time() {
    let params = randomized_params();
    let plan = identity(5, 29);
    let samples = [0, 17, 33, 50, 1_000, 5_000];
    let run = |scheduler: &mut VideoFrameScheduler| {
        samples
            .into_iter()
            .map(|pts| {
                let boundary = if pts == 0 {
                    VideoScheduleBoundary::Startup
                } else {
                    VideoScheduleBoundary::None
                };
                scheduler
                    .decide(observation(&plan, &params, 7, pts, false, boundary, SEED))
                    .expect("schedule sequence")
            })
            .collect::<Vec<_>>()
    };
    let expected = run(&mut VideoFrameScheduler::new());
    assert_eq!(run(&mut VideoFrameScheduler::new()), expected);

    let scheduler_source = include_str!("../src/media_video_frame_scheduler.rs");
    for forbidden in ["std::time", "SystemTime", "Instant::", "thread::sleep"] {
        assert!(
            !scheduler_source.contains(forbidden),
            "scheduler must not depend on {forbidden}"
        );
    }
}
