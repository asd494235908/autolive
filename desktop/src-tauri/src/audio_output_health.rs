#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CounterProgress {
    pub last_value: u64,
    pub last_progress_ms: u64,
    pub stalled_ms: Option<u64>,
    pub stalled: bool,
}

pub fn observe_counter(
    now_ms: u64,
    current_value: u64,
    previous_value: u64,
    previous_progress_ms: u64,
    enabled: bool,
    stall_after_ms: u64,
) -> CounterProgress {
    if !enabled || current_value != previous_value || previous_progress_ms == 0 {
        return CounterProgress {
            last_value: current_value,
            last_progress_ms: now_ms,
            stalled_ms: enabled.then_some(0),
            stalled: false,
        };
    }

    let stalled_ms = now_ms.saturating_sub(previous_progress_ms);
    CounterProgress {
        last_value: current_value,
        last_progress_ms: previous_progress_ms,
        stalled_ms: Some(stalled_ms),
        stalled: stalled_ms > stall_after_ms,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputHealthInput {
    pub output_exists: bool,
    pub application_running: bool,
    pub hardware_active: bool,
    pub callback_paused: bool,
    pub callback_stalled: bool,
    pub pcm_stalled: bool,
    pub sustained_underrun: bool,
    pub current_mixer_exists: bool,
    pub mixer_failed: bool,
    pub timeline_present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputHealth {
    pub running: bool,
    pub recovery_required: bool,
}

pub fn evaluate_output_health(input: OutputHealthInput) -> OutputHealth {
    let consumer_ready = input.output_exists
        && input.application_running
        && input.hardware_active
        && !input.callback_paused
        && !input.callback_stalled;
    let producer_ready = input.current_mixer_exists
        && !input.mixer_failed
        && input.timeline_present
        && !input.pcm_stalled
        && !input.sustained_underrun;

    OutputHealth {
        running: consumer_ready && producer_ready,
        recovery_required: consumer_ready && !producer_ready,
    }
}

pub fn output_resume_required(input: OutputHealthInput, playback_active: bool) -> bool {
    playback_active
        && input.output_exists
        && input.application_running
        && input.hardware_active
        && input.callback_paused
        && input.current_mixer_exists
        && !input.mixer_failed
        && input.timeline_present
}

#[cfg(test)]
mod tests {
    use super::{
        evaluate_output_health, observe_counter, output_resume_required, OutputHealthInput,
    };

    fn healthy_input() -> OutputHealthInput {
        OutputHealthInput {
            output_exists: true,
            application_running: true,
            hardware_active: true,
            callback_paused: false,
            callback_stalled: false,
            pcm_stalled: false,
            sustained_underrun: false,
            current_mixer_exists: true,
            mixer_failed: false,
            timeline_present: true,
        }
    }

    #[test]
    fn callback_activity_cannot_hide_a_stalled_pcm_counter() {
        let callback = observe_counter(4_000, 120, 100, 2_000, true, 1_500);
        let pcm = observe_counter(4_000, 80, 80, 2_000, true, 1_500);

        assert!(!callback.stalled);
        assert!(pcm.stalled);
        assert_eq!(pcm.stalled_ms, Some(2_000));
    }

    #[test]
    fn active_hardware_without_a_current_mixer_is_not_running() {
        let mut input = healthy_input();
        input.current_mixer_exists = false;

        let health = evaluate_output_health(input);

        assert!(!health.running);
        assert!(health.recovery_required);
    }

    #[test]
    fn missing_timeline_or_stalled_pcm_requires_recovery() {
        let mut missing_timeline = healthy_input();
        missing_timeline.timeline_present = false;
        assert!(evaluate_output_health(missing_timeline).recovery_required);

        let mut stalled_pcm = healthy_input();
        stalled_pcm.pcm_stalled = true;
        assert!(evaluate_output_health(stalled_pcm).recovery_required);
    }

    #[test]
    fn sustained_underrun_is_a_producer_failure_that_requires_recovery() {
        let mut input = healthy_input();
        input.sustained_underrun = true;

        let health = evaluate_output_health(input);

        assert!(!health.running);
        assert!(health.recovery_required);
    }

    #[test]
    fn paused_output_does_not_request_recovery() {
        let mut input = healthy_input();
        input.callback_paused = true;

        let health = evaluate_output_health(input);

        assert!(!health.running);
        assert!(!health.recovery_required);
    }

    #[test]
    fn playing_output_with_a_live_mixer_resumes_a_paused_callback() {
        let mut input = healthy_input();
        input.callback_paused = true;

        assert!(output_resume_required(input, true));
        assert!(!output_resume_required(input, false));

        input.mixer_failed = true;
        assert!(!output_resume_required(input, true));
        input.mixer_failed = false;
        input.timeline_present = false;
        assert!(!output_resume_required(input, true));
    }

    #[test]
    fn fully_healthy_output_is_running() {
        let health = evaluate_output_health(healthy_input());

        assert!(health.running);
        assert!(!health.recovery_required);
    }
}
