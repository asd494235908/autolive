const PROCESS_SOURCES: &[&str] = &[
    "src/audio_mixer.rs",
    "src/commands.rs",
    "src/media_engine.rs",
    "src/media_library/mod.rs",
    "src/speech_to_speech_worker.rs",
];

#[test]
fn every_background_process_uses_the_shared_hidden_command_builder() {
    for path in PROCESS_SOURCES {
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("failed to read {path}: {error}"));
        assert!(
            !source.contains("Command::new("),
            "{path} must use background_command so Windows never flashes a console window"
        );
    }
}

#[test]
fn windows_background_command_uses_the_native_no_window_flag() {
    let source = std::fs::read_to_string("src/background_process.rs")
        .expect("background process module should exist");

    assert!(source.contains("std::os::windows::process::CommandExt"));
    assert!(source.contains("CREATE_NO_WINDOW"));
    assert!(source.contains("command.creation_flags(CREATE_NO_WINDOW)"));
}

#[test]
fn release_gui_audio_decoder_is_hidden_without_swallowing_spawn_errors() {
    let main = std::fs::read_to_string("src/main.rs").expect("main.rs should exist");
    let mixer = std::fs::read_to_string("src/audio_mixer.rs").expect("audio mixer should exist");

    assert!(main.contains("any(not(debug_assertions), feature = \"package-gui\")"));
    assert!(mixer.contains("let mut command = background_command(ffmpeg_path);"));
    let spawn = mixer
        .split_once("let mut child = match command.spawn()")
        .expect("audio decoder must inspect spawn failures")
        .1
        .split_once("let stderr_reader")
        .expect("spawn failure branch must end before stderr reading")
        .0;
    assert!(spawn.contains("Err(error)"));
    assert!(spawn.contains("AudioMixerFailure::Ffmpeg"));
    assert!(spawn.contains("FFmpeg 音频解码启动失败"));
    assert!(spawn.contains("sanitize_error_detail"));
}
