#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio_cycle_switch;
mod auth_session;
mod commands;

use auth_session::{
    delete_auth_form_credential, delete_refresh_token, load_auth_form_credential,
    load_refresh_token, store_auth_form_credential, store_refresh_token,
};
use autolive_desktop_core::runtime_resource_task::{
    handle_runtime_resource_exit, RuntimeResourceTaskShutdown,
};
use commands::{
    cancel_audio_cycle_candidate, cancel_runtime_resource_install, cancel_speech_to_speech_worker,
    cleanup_local_caches_command, clear_runtime_resources, close_final_effect_window,
    commit_audio_cycle_candidate, commit_audio_variant_candidate,
    commit_audio_variant_candidate_if_due, commit_media_processing_if_ready,
    complete_playback_item, complete_playback_loop, direct_model_chat,
    discard_audio_variant_candidate, get_audio_cycle_diagnostic, get_audio_output_backend_status,
    get_default_media_effect_params, get_device_runtime_info, get_media_engine_capabilities,
    get_runtime_resource_status, get_snapshot, get_speech_to_speech_worker_capabilities,
    import_runtime_resource_directory, install_runtime_resources, list_audio_output_devices,
    open_final_effect_window, pause_playback, pause_portaudio_interlude, play_portaudio_test_tone,
    prepare_audio_cycle_candidate, prepare_webview_interlude, probe_local_mp4, probe_local_video,
    probe_local_videos, release_webview_interlude_cache, resize_final_effect_window,
    restore_original_audio, resume_playback, resume_portaudio_interlude, set_audio_output_backend,
    set_audio_processing_profile, set_interlude_config, set_processing_switches,
    stage_audio_variant_candidate, start_media_processing, start_playback,
    start_portaudio_interlude, start_speech_to_speech_worker, stop_playback,
    stop_portaudio_interlude, sync_audio_output_source, update_playback_position,
    validate_media_effect_params, AppState,
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
            probe_local_video,
            probe_local_videos,
            probe_local_mp4,
            get_device_runtime_info,
            get_media_engine_capabilities,
            get_audio_output_backend_status,
            get_audio_cycle_diagnostic,
            list_audio_output_devices,
            set_audio_output_backend,
            sync_audio_output_source,
            prepare_audio_cycle_candidate,
            commit_audio_cycle_candidate,
            cancel_audio_cycle_candidate,
            play_portaudio_test_tone,
            start_portaudio_interlude,
            prepare_webview_interlude,
            release_webview_interlude_cache,
            pause_portaudio_interlude,
            resume_portaudio_interlude,
            stop_portaudio_interlude,
            get_runtime_resource_status,
            install_runtime_resources,
            cancel_runtime_resource_install,
            import_runtime_resource_directory,
            clear_runtime_resources,
            cleanup_local_caches_command,
            get_speech_to_speech_worker_capabilities,
            get_default_media_effect_params,
            start_playback,
            pause_playback,
            resume_playback,
            update_playback_position,
            stop_playback,
            complete_playback_loop,
            complete_playback_item,
            commit_media_processing_if_ready,
            set_processing_switches,
            set_audio_processing_profile,
            set_interlude_config,
            start_media_processing,
            stage_audio_variant_candidate,
            start_speech_to_speech_worker,
            cancel_speech_to_speech_worker,
            restore_original_audio,
            commit_audio_variant_candidate,
            commit_audio_variant_candidate_if_due,
            discard_audio_variant_candidate,
            get_snapshot,
            validate_media_effect_params,
            store_refresh_token,
            load_refresh_token,
            delete_refresh_token,
            store_auth_form_credential,
            load_auth_form_credential,
            delete_auth_form_credential,
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
            let shutdown = state.shutdown_all(Duration::from_secs(3));
            match &shutdown {
                Ok(RuntimeResourceTaskShutdown::TimedOut) => {
                    eprintln!(
                        "desktop background tasks did not stop within the 3 second exit budget"
                    );
                }
                Err(error) => eprintln!("failed to stop desktop background tasks: {error}"),
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
    fn release_build_hides_the_windows_console_window() {
        let source = include_str!("main.rs");

        assert!(
            source.contains("#![cfg_attr(not(debug_assertions), windows_subsystem = \"windows\")]")
        );
    }

    #[test]
    fn tauri_config_enables_asset_protocol_and_media_csp() {
        let config =
            std::fs::read_to_string("tauri.conf.json").expect("tauri.conf.json should exist");

        assert!(config.contains("\"assetProtocol\""));
        assert!(config.contains("\"enable\": true"));
        assert!(config.contains("\"media-src\""));
        assert!(config.contains("\"main\""));
        assert!(config.contains("\"backgroundColor\": \"#0b0b0f\""));
    }

    #[test]
    fn tauri_config_bundles_the_manifest_and_embedded_runtime_resources() {
        let config =
            std::fs::read_to_string("tauri.conf.json").expect("tauri.conf.json should exist");

        assert!(config.contains("\"active\": true"));
        assert!(config.contains("\"resources\""));
        assert!(config.contains("runtime-resources.json"));
        assert!(config.contains("embedded-runtime-resources"));
        let config: serde_json::Value =
            serde_json::from_str(&config).expect("tauri.conf.json should be valid JSON");
        let resources = config["bundle"]["resources"]
            .as_array()
            .expect("bundle resources should be an array");
        assert!(resources
            .iter()
            .any(|resource| resource.as_str() == Some("ambient/low-level-room-tone.wav")));
        assert!(resources
            .iter()
            .any(|resource| resource.as_str() == Some("ambient/LICENSE.txt")));
    }

    #[test]
    fn webview_interlude_processing_command_is_registered() {
        let source = include_str!("main.rs");

        assert!(source.contains("prepare_webview_interlude,"));
        assert!(source.contains("release_webview_interlude_cache,"));
        assert!(source.contains("start_portaudio_interlude,"));
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
        assert!(config.contains("http://127.0.0.1:18090/**"));
        assert!(config.contains("https://admin.example.com/**"));
        assert!(!config.contains("http://127.0.0.1:8080/**"));
        assert!(!config.contains("http://localhost:8080/**"));
        assert!(config.contains("\"final-effect\""));
        assert!(!config.contains("http://*/**"));
        assert!(!config.contains("https://*/**"));
    }
}
