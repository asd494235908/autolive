fn command_body<'a>(source: &'a str, signature: &str, next_command: &str) -> &'a str {
    let start = source.find(signature).expect("command should exist");
    let end = source[start..]
        .find(next_command)
        .map(|offset| start + offset)
        .unwrap_or(source.len());
    &source[start..end]
}

#[test]
fn imported_source_video_is_registered_with_tauri_asset_scope() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the crate root");
    let command = command_body(
        &source,
        "fn probe_local_video_blocking(",
        "#[tauri::command]\npub fn get_device_runtime_info(",
    );

    let registration = command
        .find("allow_local_playback_asset_file(")
        .expect("the validated source video should enter the asset scope");
    let state_commit = command
        .find("playback.set_source(result.source.clone())")
        .expect("the validated source video should be committed");

    assert!(
        registration < state_commit,
        "scope registration must succeed before playback state is committed"
    );
}

#[test]
fn processed_video_is_registered_before_it_becomes_pending_playback() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the crate root");
    let command = command_body(
        &source,
        "pub fn start_media_processing(",
        "#[tauri::command]\npub fn direct_model_chat(",
    );

    let registration = command
        .find("allow_local_playback_asset_file(")
        .expect("the rendered video should enter the asset scope");
    let state_commit = command
        .find("playback.mark_media_processing_ready(")
        .expect("the rendered video should become pending playback");

    assert!(
        registration < state_commit,
        "scope registration must succeed before processed playback is exposed"
    );
}

#[test]
fn interlude_audio_files_are_registered_before_the_catalog_is_exposed() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the crate root");
    let command = command_body(
        &source,
        "pub fn set_interlude_config(",
        "#[tauri::command]\npub async fn start_portaudio_interlude(",
    );

    let registration = command
        .find("allow_local_playback_asset_file(")
        .expect("validated interlude files should enter the asset scope");
    let state_commit = command
        .find("playback.set_interlude_snapshot(snapshot.clone())")
        .expect("the interlude catalog should be committed");

    assert!(
        registration < state_commit,
        "asset scope registration must succeed before interlude files reach the WebView"
    );
}
