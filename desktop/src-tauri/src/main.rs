#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio_cycle_switch;
mod commands;
mod control_plane_auth;

use autolive_desktop_core::runtime_resource_task::{
    handle_runtime_resource_exit, RuntimeResourceTaskShutdown,
};
use commands::{
    append_local_videos, cancel_audio_cycle_candidate, cancel_runtime_resource_install,
    cleanup_local_caches_command, cleanup_stale_generated_caches, clear_playback_pool,
    clear_runtime_resources, close_final_effect_window, commit_audio_cycle_candidate,
    commit_audio_media_candidate, commit_media_processing_if_ready, complete_playback_item,
    complete_playback_loop, configure_realtime_video_cycle, discard_audio_media_candidate,
    discard_media_processing_candidate, ensure_original_video_renderer, get_audio_cycle_diagnostic,
    get_audio_output_backend_status, get_default_media_effect_params, get_device_runtime_info,
    get_media_engine_capabilities, get_media_video_backend_status, get_runtime_resource_status,
    get_snapshot, import_runtime_resource_directory, install_runtime_resources,
    list_audio_output_devices, open_final_effect_window, pause_playback, pause_portaudio_interlude,
    play_portaudio_test_tone, prepare_audio_cycle_candidate, prepare_audio_media_candidate,
    prepare_webview_interlude, probe_local_mp4, probe_local_video, probe_local_videos,
    purge_media_processing_cache, release_audio_media_candidate, release_media_processing_artifact,
    release_webview_interlude_cache, remove_playback_pool_item, reorder_playback_pool_items,
    replace_playback_pool_item, resize_final_effect_window, restore_original_audio,
    restore_original_video, resume_playback, resume_portaudio_interlude, seek_playback,
    set_audio_output_backend, set_audio_processing_profile, set_interlude_config,
    set_portaudio_interlude_volume, set_portaudio_media_volume, set_processing_switches,
    start_media_processing, start_playback, start_portaudio_interlude, stop_playback,
    stop_portaudio_interlude, stop_realtime_video_renderer, switch_portaudio_interlude_preset,
    sync_audio_output_source, update_playback_position, validate_media_effect_params, AppState,
};
use control_plane_auth::{
    clear_legacy_auth_credentials, get_or_create_control_plane_device_id,
    login_control_plane_session, logout_control_plane_session, refresh_control_plane_session,
    restore_control_plane_session, retry_pending_control_plane_logout, ControlPlaneAuthState,
};
use std::process::ExitCode;
use std::time::Duration;
use tauri::{Manager, RunEvent};

// 历史兼容实现继续参与编译审计，但当前版本不向任何窗口注册这些命令。
#[allow(dead_code)]
fn retain_unregistered_realtime_speech_commands() {
    let _ = commands::get_speech_to_speech_worker_capabilities;
    let _ = commands::start_speech_to_speech_worker;
    let _ = commands::cancel_speech_to_speech_worker;
    let _ = commands::stage_audio_variant_candidate;
    let _ = commands::commit_audio_variant_candidate;
    let _ = commands::commit_audio_variant_candidate_if_due;
    let _ = commands::discard_audio_variant_candidate;
    let _ = commands::direct_model_chat;
}

fn main() -> ExitCode {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_http::init())
        .manage(AppState::default())
        .manage(ControlPlaneAuthState::default())
        .invoke_handler(tauri::generate_handler![
            probe_local_video,
            probe_local_videos,
            append_local_videos,
            replace_playback_pool_item,
            reorder_playback_pool_items,
            remove_playback_pool_item,
            clear_playback_pool,
            probe_local_mp4,
            get_device_runtime_info,
            get_media_engine_capabilities,
            get_media_video_backend_status,
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
            switch_portaudio_interlude_preset,
            set_portaudio_media_volume,
            set_portaudio_interlude_volume,
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
            release_audio_media_candidate,
            release_media_processing_artifact,
            discard_audio_media_candidate,
            discard_media_processing_candidate,
            get_default_media_effect_params,
            start_playback,
            pause_playback,
            resume_playback,
            seek_playback,
            update_playback_position,
            stop_playback,
            complete_playback_loop,
            complete_playback_item,
            commit_media_processing_if_ready,
            commit_audio_media_candidate,
            set_processing_switches,
            set_audio_processing_profile,
            set_interlude_config,
            start_media_processing,
            ensure_original_video_renderer,
            configure_realtime_video_cycle,
            stop_realtime_video_renderer,
            prepare_audio_media_candidate,
            restore_original_audio,
            restore_original_video,
            get_snapshot,
            validate_media_effect_params,
            get_or_create_control_plane_device_id,
            login_control_plane_session,
            restore_control_plane_session,
            refresh_control_plane_session,
            logout_control_plane_session,
            retry_pending_control_plane_logout,
            clear_legacy_auth_credentials,
            open_final_effect_window,
            close_final_effect_window,
            resize_final_effect_window,
        ]);

    let app = match builder.build(tauri::generate_context!()) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("failed to build tauri desktop shell: {error}");
            return ExitCode::FAILURE;
        }
    };
    app.run(|app_handle, event| {
        if matches!(&event, RunEvent::Ready) {
            let state = app_handle.state::<AppState>();
            if let Err(error) = state.start_realtime_video_eof_supervisor(app_handle.clone()) {
                eprintln!("failed to start realtime video EOF supervisor: {error}");
            }
            if let Err(error) = purge_media_processing_cache(app_handle) {
                eprintln!("failed to purge stale media processing cache: {error}");
            }
            if let Err(error) = cleanup_stale_generated_caches(app_handle) {
                eprintln!("failed to cleanup stale generated caches: {error}");
            }
        }
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
            if let Err(error) = purge_media_processing_cache(app_handle) {
                eprintln!("failed to purge media processing cache on exit: {error}");
            }
            if let Err(error) = cleanup_stale_generated_caches(app_handle) {
                eprintln!("failed to cleanup generated caches on exit: {error}");
            }
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
    fn current_release_does_not_register_or_grant_realtime_speech_commands() {
        let source = include_str!("main.rs");
        let handler_start = source
            .find(".invoke_handler(tauri::generate_handler![")
            .expect("invoke handler should exist");
        let handler_end = source[handler_start..]
            .find("]);")
            .map(|offset| handler_start + offset)
            .expect("invoke handler should terminate");
        let invoke_handler = &source[handler_start..handler_end];
        let main_permissions = std::fs::read_to_string("permissions/command-sets.toml")
            .expect("command permission sets should exist");
        let final_effect = std::fs::read_to_string("capabilities/final-effect.json")
            .expect("final-effect capability should exist");

        for command in [
            "get_speech_to_speech_worker_capabilities",
            "start_speech_to_speech_worker",
            "cancel_speech_to_speech_worker",
            "stage_audio_variant_candidate",
            "commit_audio_variant_candidate",
            "commit_audio_variant_candidate_if_due",
            "discard_audio_variant_candidate",
            "direct_model_chat",
        ] {
            assert!(
                !invoke_handler.contains(command),
                "registered command: {command}"
            );
            let permission = format!("allow-{}", command.replace('_', "-"));
            assert!(
                !main_permissions.contains(&permission),
                "main window grants {permission}"
            );
            assert!(
                !final_effect.contains(&permission),
                "final-effect window grants {permission}"
            );
        }
    }

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
    fn media_candidate_discard_is_registered_for_the_final_effect_window() {
        let source = include_str!("main.rs");
        let build = include_str!("../build.rs");
        let permissions = std::fs::read_to_string("permissions/command-sets.toml")
            .expect("command permission sets should exist");

        assert!(source.contains("discard_media_processing_candidate,"));
        assert!(build.contains("\"discard_media_processing_candidate\""));
        assert!(
            permissions
                .matches("allow-discard-media-processing-candidate")
                .count()
                >= 2
        );
    }

    #[test]
    fn independent_audio_media_commands_are_registered_for_both_windows() {
        let source = include_str!("main.rs");
        let build = include_str!("../build.rs");
        let permissions = std::fs::read_to_string("permissions/command-sets.toml")
            .expect("command permission sets should exist");

        for command in [
            "prepare_audio_media_candidate",
            "commit_audio_media_candidate",
            "discard_audio_media_candidate",
            "release_audio_media_candidate",
        ] {
            assert!(source.contains(command));
            assert!(build.contains(&format!("\"{command}\"")));
        }
        for permission in [
            "allow-commit-audio-media-candidate",
            "allow-discard-audio-media-candidate",
            "allow-release-audio-media-candidate",
        ] {
            assert!(permissions.matches(permission).count() >= 2);
        }
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
        let final_effect = std::fs::read_to_string("capabilities/final-effect.json")
            .expect("capabilities/final-effect.json should exist");

        assert!(config.contains("\"identifier\": \"http:default\""));
        assert!(config.contains("https://admin.example.com/**"));
        assert!(!config.contains("http://127.0.0.1:18090/**"));
        assert!(!config.contains("http://127.0.0.1:8080/**"));
        assert!(!config.contains("http://localhost:8080/**"));
        assert!(!config.contains("\"final-effect\""));
        assert!(!config.contains("http://*/**"));
        assert!(!config.contains("https://*/**"));
        assert!(final_effect.contains("\"final-effect\""));
        assert!(!final_effect.contains("http:"));
        assert!(!final_effect.contains("dialog:"));
    }

    #[test]
    fn production_csp_excludes_development_origins() {
        let production =
            std::fs::read_to_string("tauri.conf.json").expect("tauri.conf.json should exist");
        let development = std::fs::read_to_string("tauri.test.conf.json")
            .expect("tauri.test.conf.json should exist");
        let production: serde_json::Value =
            serde_json::from_str(&production).expect("production config should be valid JSON");
        let production_csp = production["app"]["security"]["csp"].to_string();

        assert!(!production_csp.contains("localhost:5173"));
        assert!(!production_csp.contains("ws://"));
        assert!(development.contains("localhost:5173"));
        assert!(development.contains("ws://localhost:5173"));
    }
}
