#[test]
fn final_effect_window_loads_the_bundled_index_without_a_query_path() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the crate root");
    let start = source
        .find("pub async fn open_final_effect_window(")
        .expect("open_final_effect_window should exist");
    let end = source[start..]
        .find("#[tauri::command]\npub fn close_final_effect_window(")
        .map(|offset| start + offset)
        .unwrap_or(source.len());
    let command = &source[start..end];

    assert!(command.contains("WebviewUrl::App(\"index.html\".into())"));
    assert!(!command.contains("index.html?"));
    assert!(command.contains("request: Option<ResizeFinalEffectWindowRequestDto>"));
    assert!(command.contains("resize_final_effect_window_for_app"));
}

#[test]
fn newly_created_final_effect_window_defers_resize_until_its_page_loads() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the crate root");
    let start = source
        .find("pub async fn open_final_effect_window(")
        .expect("open_final_effect_window should exist");
    let end = source[start..]
        .find("#[tauri::command]\npub fn close_final_effect_window(")
        .map(|offset| start + offset)
        .unwrap_or(source.len());
    let command = &source[start..end];

    assert!(command.contains("if !created {"));
    assert!(command.contains("resize_final_effect_window_for_app(&app, request, false)"));
    assert!(!command.contains("resize_final_effect_window_for_app(&app, request, created)"));
    assert!(source.contains("resize_final_effect_window_for_app(&app, request, false)"));

    let resize_helper = source
        .split_once("fn resize_final_effect_window_for_app(")
        .map(|(_, tail)| tail)
        .expect("resize helper should exist");
    assert!(resize_helper.contains("center_after_resize: bool"));
    assert!(resize_helper.contains("if center_after_resize {"));
}

#[test]
fn destroying_the_final_effect_window_purges_session_media_artifacts() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the crate root");
    let release_surface = source
        .split_once("fn disable_media_processing_on_final_effect_close(")
        .map(|(_, tail)| tail)
        .expect("final effect cleanup should exist")
        .split("fn attach_final_effect_close_cleanup(")
        .next()
        .expect("surface cleanup should have a bounded body");
    let cleanup = source
        .split_once("fn attach_final_effect_close_cleanup(")
        .map(|(_, tail)| tail)
        .expect("final effect cleanup hook should exist")
        .split("#[tauri::command]")
        .next()
        .expect("cleanup hook should have a bounded body");

    assert!(cleanup.contains("tauri::WindowEvent::Destroyed"));
    assert!(cleanup.contains("purge_media_processing_cache(&app_handle)"));
    assert!(cleanup.contains("cleanup_started.swap(true, Ordering::AcqRel)"));
    assert!(release_surface.contains("release_final_effect_video_host(app)"));
    assert!(!release_surface.contains("state.stop_realtime_video_runtime()"));

    let native_host_release = source
        .split_once("fn release_final_effect_video_host(")
        .map(|(_, tail)| tail)
        .expect("native video host release should exist")
        .split("fn disable_media_processing_on_final_effect_close(")
        .next()
        .expect("native video host release should have a bounded body");
    assert!(native_host_release.contains("state.suspend_realtime_video_runtime()"));
    assert!(native_host_release.contains("host.destroy()"));

    let close_command = source
        .split_once("pub fn close_final_effect_window(")
        .map(|(_, tail)| tail)
        .expect("close command should exist")
        .split("#[tauri::command]")
        .next()
        .expect("close command should have a bounded body");
    assert!(!close_command.contains("disable_media_processing_on_final_effect_close"));
}
