mod auth_session;
mod commands;

use auth_session::{delete_refresh_token, load_refresh_token, store_refresh_token};
use commands::{
    cancel_research_analysis, cancel_speech_to_speech_worker, cancel_voice_clone_operation,
    cleanup_local_caches_command, clear_voice_clone_replacement, close_final_effect_window,
    commit_audio_variant_candidate, commit_audio_variant_candidate_if_due,
    commit_media_processing_if_ready, complete_playback_loop, direct_model_chat,
    discard_audio_variant_candidate, get_default_local_research_params, get_device_runtime_info,
    get_media_engine_capabilities, get_research_status, get_research_worker_capabilities,
    get_snapshot, get_speech_to_speech_worker_capabilities, get_voice_clone_worker_capabilities,
    hash_local_mp4_sha256, open_final_effect_window, pause_playback, prepare_voice_clone_source,
    probe_local_mp4, resize_final_effect_window, restore_original_audio, resume_playback,
    set_audio_processing_profile, set_processing_switches, stage_audio_variant_candidate,
    start_media_processing, start_playback, start_research_analysis, start_speech_to_speech_worker,
    start_voice_clone_replacement, stop_playback, validate_local_research_params, AppState,
};

fn main() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_http::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            probe_local_mp4,
            hash_local_mp4_sha256,
            get_device_runtime_info,
            get_media_engine_capabilities,
            get_research_worker_capabilities,
            get_research_status,
            cleanup_local_caches_command,
            get_speech_to_speech_worker_capabilities,
            get_voice_clone_worker_capabilities,
            get_default_local_research_params,
            start_playback,
            pause_playback,
            resume_playback,
            stop_playback,
            complete_playback_loop,
            prepare_voice_clone_source,
            start_voice_clone_replacement,
            cancel_voice_clone_operation,
            clear_voice_clone_replacement,
            commit_media_processing_if_ready,
            set_processing_switches,
            set_audio_processing_profile,
            start_media_processing,
            start_research_analysis,
            cancel_research_analysis,
            stage_audio_variant_candidate,
            start_speech_to_speech_worker,
            cancel_speech_to_speech_worker,
            restore_original_audio,
            commit_audio_variant_candidate,
            commit_audio_variant_candidate_if_due,
            discard_audio_variant_candidate,
            get_snapshot,
            validate_local_research_params,
            store_refresh_token,
            load_refresh_token,
            delete_refresh_token,
            open_final_effect_window,
            close_final_effect_window,
            resize_final_effect_window,
            direct_model_chat
        ]);

    if let Err(error) = builder.run(tauri::generate_context!()) {
        eprintln!("failed to run tauri desktop shell: {error}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tauri_config_enables_asset_protocol_and_media_csp() {
        let config =
            std::fs::read_to_string("tauri.conf.json").expect("tauri.conf.json should exist");

        assert!(config.contains("\"assetProtocol\""));
        assert!(config.contains("\"enable\": true"));
        assert!(config.contains("\"media-src\""));
        assert!(config.contains("\"main\""));
    }

    #[test]
    fn tauri_config_bundles_cross_platform_media_binaries() {
        let config =
            std::fs::read_to_string("tauri.conf.json").expect("tauri.conf.json should exist");

        assert!(config.contains("\"active\": true"));
        assert!(config.contains("\"resources\""));
        assert!(config.contains("binaries/"));
    }

    #[test]
    fn tauri_bundle_has_a_windows_icon() {
        assert!(std::path::Path::new("icons/icon.ico").is_file());

        let config =
            std::fs::read_to_string("tauri.conf.json").expect("tauri.conf.json should exist");
        assert!(config.contains("\"icons/icon.ico\""));
    }

    #[test]
    fn capabilities_allow_remote_control_plane_only() {
        let config = std::fs::read_to_string("capabilities/default.json")
            .expect("capabilities/default.json should exist");

        assert!(config.contains("\"identifier\": \"http:default\""));
        assert!(config.contains("http://192.168.100.213:18090/**"));
        assert!(!config.contains("http://127.0.0.1:8080/**"));
        assert!(!config.contains("http://localhost:8080/**"));
        assert!(config.contains("\"final-effect\""));
        assert!(!config.contains("http://*/**"));
        assert!(!config.contains("https://*/**"));
    }
}
