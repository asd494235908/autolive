mod build_support;

const WINDOWS_MPV_MANIFEST_REFERENCE: &str =
    include_str!("../third_party/mpv/x86_64-pc-windows-msvc/legal/mpv-runtime-manifest.json");

fn main() {
    let profile = std::env::var("PROFILE").ok();
    let external_override = std::env::var("TAURI_CONFIG").ok();
    let action = build_support::runtime_resource_config_action(
        profile.as_deref(),
        std::path::Path::new("runtime-resources.json").is_file(),
        external_override.as_deref(),
    )
    .unwrap_or_else(|message| panic!("{message}"));
    if !matches!(profile.as_deref(), Some("debug" | "test")) {
        build_support::validate_release_runtime_resource_config(
            include_str!("tauri.conf.json"),
            external_override.as_deref(),
        )
        .unwrap_or_else(|message| panic!("{message}"));
        let runtime_manifest = std::fs::read_to_string("runtime-resources.json")
            .unwrap_or_else(|error| panic!("无法读取 runtime-resources.json：{error}"));
        let build_target = std::env::var("TARGET")
            .unwrap_or_else(|error| panic!("无法读取 Cargo TARGET：{error}"));
        let windows_mpv_manifest_reference =
            (build_target == "x86_64-pc-windows-msvc").then_some(WINDOWS_MPV_MANIFEST_REFERENCE);
        build_support::validate_release_runtime_resource_tree(
            &runtime_manifest,
            std::path::Path::new("embedded-runtime-resources"),
            &build_target,
            windows_mpv_manifest_reference,
        )
        .unwrap_or_else(|message| panic!("{message}"));
    }
    if let build_support::RuntimeResourceConfigAction::UseDevelopmentOverride(config) = action {
        // build.rs 和 generate_context! 分属两个编译阶段，二者必须看到同一覆盖值。
        std::env::set_var("TAURI_CONFIG", &config);
        println!("cargo:rustc-env=TAURI_CONFIG={config}");
    }
    let commands = &[
        "probe_local_video",
        "probe_local_videos",
        "append_local_videos",
        "replace_playback_pool_item",
        "reorder_playback_pool_items",
        "remove_playback_pool_item",
        "clear_playback_pool",
        "probe_local_mp4",
        "get_device_runtime_info",
        "get_media_engine_capabilities",
        "get_media_video_backend_status",
        "get_audio_output_backend_status",
        "get_audio_cycle_diagnostic",
        "get_rtmp_output_status",
        "validate_rtmp_output_config",
        "start_rtmp_output",
        "stop_rtmp_output",
        "get_virtual_camera_status",
        "install_or_repair_virtual_camera",
        "start_virtual_camera_output",
        "stop_virtual_camera_output",
        "list_audio_output_devices",
        "list_portaudio_input_devices",
        "set_audio_output_backend",
        "sync_audio_output_source",
        "prepare_audio_cycle_candidate",
        "commit_audio_cycle_candidate",
        "cancel_audio_cycle_candidate",
        "play_portaudio_test_tone",
        "start_portaudio_interlude",
        "start_microphone_interlude",
        "switch_portaudio_interlude_preset",
        "set_portaudio_media_volume",
        "set_portaudio_interlude_volume",
        "prepare_webview_interlude",
        "release_webview_interlude_cache",
        "pause_portaudio_interlude",
        "resume_portaudio_interlude",
        "stop_portaudio_interlude",
        "stop_microphone_interlude",
        "get_microphone_interlude_status",
        "set_microphone_interlude_config",
        "get_runtime_resource_status",
        "install_runtime_resources",
        "cancel_runtime_resource_install",
        "import_runtime_resource_directory",
        "clear_runtime_resources",
        "cleanup_local_caches_command",
        "release_audio_media_candidate",
        "get_default_media_effect_params",
        "start_playback",
        "pause_playback",
        "resume_playback",
        "seek_playback",
        "update_playback_position",
        "stop_playback",
        "complete_playback_loop",
        "complete_playback_item",
        "commit_media_processing_if_ready",
        "commit_audio_media_candidate",
        "discard_audio_media_candidate",
        "prepare_audio_media_candidate",
        "discard_media_processing_candidate",
        "set_processing_switches",
        "set_audio_processing_profile",
        "set_interlude_config",
        "start_media_processing",
        "ensure_original_video_renderer",
        "configure_realtime_video_cycle",
        "stop_realtime_video_renderer",
        "release_media_processing_artifact",
        "restore_original_audio",
        "restore_original_video",
        "get_snapshot",
        "validate_media_effect_params",
        "get_or_create_control_plane_device_id",
        "login_control_plane_session",
        "restore_control_plane_session",
        "refresh_control_plane_session",
        "logout_control_plane_session",
        "retry_pending_control_plane_logout",
        "clear_legacy_auth_credentials",
        "start_douyin_live_probe",
        "get_douyin_live_probe_status",
        "stop_douyin_live_probe",
        "open_final_effect_window",
        "close_final_effect_window",
        "resize_final_effect_window",
    ];
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(commands));
    tauri_build::try_build(attributes).unwrap_or_else(|error| panic!("{error:#}"));
}
