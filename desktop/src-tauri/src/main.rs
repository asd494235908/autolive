mod auth_session;
mod commands;

use auth_session::{delete_refresh_token, load_refresh_token, store_refresh_token};
use autolive_desktop_core::runtime_resource_task::{
    handle_runtime_resource_exit, RuntimeResourceTaskShutdown,
};
use commands::{
    cancel_research_analysis, cancel_runtime_resource_install, cancel_speech_to_speech_worker,
    cancel_voice_clone_operation, cleanup_local_caches_command, clear_runtime_resources,
    clear_voice_clone_replacement, close_final_effect_window, commit_audio_variant_candidate,
    commit_audio_variant_candidate_if_due, commit_media_processing_if_ready,
    complete_playback_loop, direct_model_chat, discard_audio_variant_candidate,
    fail_voice_clone_playback, finish_voice_clone_playback, get_default_local_research_params,
    get_device_runtime_info, get_media_engine_capabilities, get_research_status,
    get_research_worker_capabilities, get_runtime_resource_status, get_snapshot,
    get_speech_to_speech_worker_capabilities, get_voice_clone_worker_capabilities,
    import_runtime_resource_directory, install_runtime_resources, open_final_effect_window,
    pause_playback, prepare_voice_clone_source, probe_local_mp4, resize_final_effect_window,
    restore_original_audio, resume_playback, set_audio_processing_profile, set_interlude_config,
    set_processing_switches, stage_audio_variant_candidate, start_media_processing, start_playback,
    start_research_analysis, start_speech_to_speech_worker, start_voice_clone_playback,
    start_voice_clone_pre_generation, start_voice_clone_replacement, stop_playback,
    update_playback_position, validate_local_research_params, AppState,
};
use std::process::ExitCode;
use std::time::Duration;
use tauri::{Manager, RunEvent};

fn main() -> ExitCode {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_http::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            probe_local_mp4,
            get_device_runtime_info,
            get_media_engine_capabilities,
            get_runtime_resource_status,
            install_runtime_resources,
            cancel_runtime_resource_install,
            import_runtime_resource_directory,
            clear_runtime_resources,
            get_research_worker_capabilities,
            get_research_status,
            cleanup_local_caches_command,
            get_speech_to_speech_worker_capabilities,
            get_voice_clone_worker_capabilities,
            get_default_local_research_params,
            start_playback,
            pause_playback,
            resume_playback,
            update_playback_position,
            stop_playback,
            complete_playback_loop,
            prepare_voice_clone_source,
            start_voice_clone_playback,
            start_voice_clone_pre_generation,
            start_voice_clone_replacement,
            finish_voice_clone_playback,
            fail_voice_clone_playback,
            cancel_voice_clone_operation,
            clear_voice_clone_replacement,
            commit_media_processing_if_ready,
            set_processing_switches,
            set_audio_processing_profile,
            set_interlude_config,
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

    let app = match builder.build(tauri::generate_context!()) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("failed to build tauri desktop shell: {error}");
            return ExitCode::FAILURE;
        }
    };
    app.run(|app_handle, event| {
        if let RunEvent::ExitRequested { code, .. } = event {
            let state = app_handle.state::<AppState>();
            let shutdown = state.shutdown_runtime_resources(Duration::from_secs(3));
            match &shutdown {
                Ok(RuntimeResourceTaskShutdown::TimedOut) => {
                    eprintln!("runtime resource task did not stop within the 3 second exit budget");
                }
                Err(error) => eprintln!("failed to stop runtime resource task: {error}"),
                Ok(RuntimeResourceTaskShutdown::Idle | RuntimeResourceTaskShutdown::Joined) => {}
            }
            let exit_code = code.unwrap_or(0);
            let _result = handle_runtime_resource_exit(shutdown, || {
                std::process::exit(exit_code);
            });
        }
    });
    ExitCode::SUCCESS
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
        assert!(config.contains("\"backgroundColor\": \"#ffffff\""));
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
