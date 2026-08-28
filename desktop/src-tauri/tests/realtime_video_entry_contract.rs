use std::fs;
use std::path::PathBuf;

fn workspace_file(relative: &str) -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(manifest.join(relative)).expect("contract source should be readable")
}

#[test]
fn command_prepares_the_managed_mpv_renderer_instead_of_webview_css() {
    let commands = workspace_file("src/commands.rs");
    let main = workspace_file("src/main.rs");
    let prepare = commands
        .split("pub fn prepare_realtime_video_plan")
        .nth(1)
        .expect("prepare realtime command")
        .split("pub fn commit_realtime_video_plan")
        .next()
        .expect("prepare realtime command body");

    assert!(prepare.contains("compile_gpu83_realtime_parameters"));
    assert!(prepare.contains("PrepareRealtimeRenderer {"));
    assert!(prepare.contains(".prepare("));
    assert!(!prepare.contains("prepare_webview"));
    for legacy in [
        "prepare_media_video_stream_period",
        "read_media_video_stream_chunk",
        "ack_media_video_stream_chunk",
        "commit_media_video_stream_period",
        "seek_media_video_stream",
        "stop_media_video_stream",
    ] {
        assert!(
            !commands.contains(legacy),
            "legacy command remains: {legacy}"
        );
        assert!(
            !main.contains(legacy),
            "legacy command remains registered: {legacy}"
        );
    }
}

#[test]
fn desktop_routes_formal_video_cycles_only_to_mpv() {
    let app = workspace_file("../ui/src/App.tsx");
    let apply_start = app
        .find("async function applyVideoProcessing")
        .expect("video processing entry");
    let apply_end = app[apply_start..]
        .find("function commitPreparedRealtimeVideoCandidate")
        .map(|offset| apply_start + offset)
        .expect("video processing entry end");
    let apply = &app[apply_start..apply_end];

    let realtime = apply
        .find("'prepare_realtime_video_plan'")
        .expect("managed mpv prepare call");
    let commit = app
        .find("'commit_realtime_video_plan'")
        .expect("managed mpv commit call");
    assert!(apply.contains("mediaCandidate.realtimePrepared = true"));
    assert!(
        apply_start + realtime < commit,
        "mpv prepare must happen before commit"
    );
    assert!(app.contains("'stop_realtime_video_renderer'"));
    for legacy in [
        "prepare_media_video_stream_period",
        "read_media_video_stream_chunk",
        "ack_media_video_stream_chunk",
        "commit_media_video_stream_period",
        "seek_media_video_stream",
        "stop_media_video_stream",
        "VideoMse",
        "video_stream",
    ] {
        assert!(
            !app.contains(legacy),
            "legacy video contract remains: {legacy}"
        );
    }
    assert!(app.contains("const MPV_REALTIME_VIDEO_ENABLED = true"));
}

#[test]
fn production_bundle_contains_the_only_allowed_gpu83_shader() {
    let config = workspace_file("tauri.conf.json");
    let shader = workspace_file("resources/shaders/gpu83.hook");

    assert!(config.contains(r#""resources/shaders/gpu83.hook""#));
    assert!(shader.contains("//!DESC AutoLive verified realtime pixel effects"));
    assert!(shader.contains("//!HOOK MAIN"));
}
