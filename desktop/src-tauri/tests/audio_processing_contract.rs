use autolive_desktop_core::audio_processing::AudioProcessingProfile;
use autolive_desktop_core::media_effect_params::AudioEffectParams;

#[test]
fn default_audio_processing_profile_is_valid_and_versioned() {
    let profile = AudioProcessingProfile {
        parameters_version: "audio_processing_v1".to_owned(),
        params: AudioEffectParams::default(),
    };

    assert!(profile.validate().is_ok());
}

#[test]
fn audio_processing_profile_rejects_missing_or_oversized_version() {
    let mut profile = AudioProcessingProfile {
        parameters_version: String::new(),
        params: AudioEffectParams::default(),
    };
    assert!(profile.validate().is_err());

    profile.parameters_version = "v".repeat(65);
    assert!(profile.validate().is_err());
}

#[test]
fn scheduled_audio_cycle_uses_catch_up_prebuffering() {
    let source = include_str!("../src/commands.rs");
    let (_, after_start) = source
        .split_once("    fn scheduled_audio_cycle_task(")
        .expect("scheduled_audio_cycle_task must remain present");
    let (task_body, _) = after_start
        .split_once("    fn commit_audio_mixer_candidate(")
        .expect("scheduled_audio_cycle_task boundary must remain present");

    assert!(task_body
        .contains("AudioMixerTask::start_short_cycle_candidate_with_filter_and_variant_count("));
    assert!(!task_body.contains("AudioMixerTask::start_candidate_with_filter_and_variant_count("));
    assert!(!task_body
        .contains("AudioMixerTask::start_scheduled_candidate_with_filter_and_variant_count("));
}
