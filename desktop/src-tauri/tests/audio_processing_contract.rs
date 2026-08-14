use autolive_desktop_core::audio_processing::AudioProcessingProfile;
use autolive_desktop_core::research_params::AudioResearchParams;

#[test]
fn default_audio_processing_profile_is_valid_and_versioned() {
    let profile = AudioProcessingProfile {
        parameters_version: "audio_processing_v1".to_owned(),
        params: AudioResearchParams::default(),
    };

    assert!(profile.validate().is_ok());
}

#[test]
fn audio_processing_profile_rejects_missing_or_oversized_version() {
    let mut profile = AudioProcessingProfile {
        parameters_version: String::new(),
        params: AudioResearchParams::default(),
    };
    assert!(profile.validate().is_err());

    profile.parameters_version = "v".repeat(65);
    assert!(profile.validate().is_err());
}
