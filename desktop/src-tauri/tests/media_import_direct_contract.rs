fn function_body<'a>(source: &'a str, signature: &str, next_signature: &str) -> &'a str {
    let start = source.find(signature).expect("function should exist");
    let end = source[start..]
        .find(next_signature)
        .map(|offset| start + offset)
        .unwrap_or(source.len());
    &source[start..end]
}

#[test]
fn media_import_keeps_the_ffprobe_result_without_compatibility_transcoding() {
    let commands = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the crate root");
    let import = function_body(
        &commands,
        "fn probe_local_video_paths(",
        "fn probe_local_video_pool_blocking(",
    );

    assert!(import.contains("probe_user_selected_video_with_ffprobe("));
    assert!(import.contains("insert_canonical_source_path("));
    assert!(import.contains("allow_local_playback_asset_file("));
    assert!(!import.contains("prepare_media_compatibility"));
    assert!(!import.contains("ffmpeg_path"));
    assert!(!import.contains("media-compatibility"));
    assert!(!import.contains("playback_reference ="));
    assert!(!import.contains("compatibility_mode ="));

    let library = std::fs::read_to_string("src/lib.rs")
        .expect("lib.rs should be readable from the crate root");
    assert!(!library.contains("pub mod media_compatibility;"));
}
