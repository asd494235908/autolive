#[test]
fn fixed_voice_audio_paths_are_registered_with_tauri_asset_scope() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the crate root");

    assert!(
        source.contains("asset_protocol_scope()") && source.contains(".allow_file("),
        "validated fixed voice audio must be registered with Tauri asset scope"
    );
    assert!(
        source.matches("allow_voice_clone_audio_file(").count() >= 4,
        "generated playback, cached playback, and replacement must cross the shared registration boundary"
    );
}

#[test]
fn playback_completion_command_accepts_the_final_effect_window() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the crate root");
    let start = source
        .find("pub fn finish_voice_clone_playback(")
        .expect("finish_voice_clone_playback should exist");
    let end = source[start..]
        .find("#[tauri::command]")
        .map(|offset| start + offset)
        .unwrap_or(source.len());
    let command = &source[start..end];

    assert!(command.contains("ensure_playback_window"));
    assert!(command.contains("with_playback_window"));
    assert!(!command.contains("ensure_main_window"));
    assert!(!command.contains("with_playback(&window"));
}
