const PROCESS_SOURCES: &[&str] = &[
    "src/commands.rs",
    "src/media_engine.rs",
    "src/media_library/mod.rs",
    "src/research_worker.rs",
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
