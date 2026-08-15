fn commands_source() -> String {
    std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the desktop crate")
}

fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} should exist"));
    let tail = &source[start..];
    let end = tail.find("\n#[tauri::command]").unwrap_or(tail.len());
    &tail[..end]
}

#[test]
fn voice_clone_capability_probe_is_not_run_synchronously_in_the_tauri_command() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the desktop crate");
    let command = source
        .split("#[tauri::command]")
        .find(|chunk| chunk.contains("get_voice_clone_worker_capabilities"))
        .expect("voice clone capability command should exist");

    assert!(command.contains("pub async fn get_voice_clone_worker_capabilities"));
    assert!(command.contains("spawn_blocking"));
}

#[test]
fn voice_clone_source_preparation_is_not_run_synchronously_in_the_tauri_command() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the desktop crate");
    let command = source
        .split("#[tauri::command]")
        .find(|chunk| chunk.contains("prepare_voice_clone_source"))
        .expect("voice clone preparation command should exist");

    assert!(command.contains("pub async fn prepare_voice_clone_source"));
    assert!(command.contains("spawn_blocking"));
}

#[test]
fn voice_clone_audio_commands_are_not_run_synchronously_in_the_tauri_command() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the desktop crate");

    for command_name in [
        "start_voice_clone_replacement",
        "start_voice_clone_playback",
        "start_voice_clone_pre_generation",
    ] {
        let command = source
            .split("#[tauri::command]")
            .find(|chunk| chunk.contains(&format!("{command_name}(")))
            .unwrap_or_else(|| panic!("{command_name} command should exist"));

        assert!(command.contains(&format!("pub async fn {command_name}")));
        assert!(command.contains("spawn_blocking"));
    }
}

#[test]
fn every_voice_clone_start_transaction_uses_the_launch_lock() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the desktop crate");

    for function_name in [
        "prepare_voice_clone_source_blocking",
        "start_voice_clone_replacement_blocking",
        "start_voice_clone_playback_blocking",
        "start_voice_clone_pre_generation_blocking",
    ] {
        let start = source
            .find(&format!("fn {function_name}("))
            .unwrap_or_else(|| panic!("{function_name} should exist"));
        let tail = &source[start..];
        let end = tail.find("\n#[tauri::command]").unwrap_or(tail.len());
        let function = &tail[..end];

        assert!(
            function.contains("lock_voice_clone_launch"),
            "{function_name} must acquire the shared voice clone launch lock"
        );
    }
}

#[test]
fn voice_clone_launch_transaction_uses_the_shared_lock() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the desktop crate");
    let start = source
        .find("fn with_voice_clone_launch<T>(")
        .expect("voice clone launch transaction helper should exist");
    let tail = &source[start..];
    let end = tail
        .find("\n    fn reap_finished_speech_worker")
        .expect("voice clone launch transaction helper should end before worker methods");
    let function = &tail[..end];

    assert!(function.contains("lock_voice_clone_launch"));
    assert!(function.contains("handler(self)"));
}

#[test]
fn voice_clone_stop_and_playback_state_changes_share_each_launch_transaction() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the desktop crate");

    for function_name in [
        "probe_local_mp4_blocking",
        "cancel_voice_clone_operation",
        "clear_voice_clone_replacement",
        "stop_playback",
        "complete_playback_loop",
    ] {
        let start = source
            .find(&format!("fn {function_name}("))
            .unwrap_or_else(|| panic!("{function_name} should exist"));
        let tail = &source[start..];
        let end = tail.find("\n#[tauri::command]").unwrap_or(tail.len());
        let function = &tail[..end];

        assert!(
            function.contains("with_voice_clone_launch"),
            "{function_name} must keep stop/join and playback state commit in one transaction"
        );
        assert!(
            !function.contains("stop_voice_clone_worker()?"),
            "{function_name} must not nest the launch lock"
        );
    }

    let probe_start = source
        .find("fn probe_local_mp4_blocking(")
        .expect("probe_local_mp4_blocking should exist");
    let probe_tail = &source[probe_start..];
    let probe_end = probe_tail
        .find("\n#[tauri::command]")
        .unwrap_or(probe_tail.len());
    let probe = &probe_tail[..probe_end];
    assert!(
        probe.find("probe_user_selected_mp4") < probe.find("with_voice_clone_launch"),
        "expensive media probing must finish before the launch transaction"
    );
}

#[test]
fn video_import_does_not_probe_or_hash_on_the_tauri_command_thread() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the desktop crate");
    let command = source
        .split("#[tauri::command]")
        .find(|chunk| chunk.contains("probe_local_mp4"))
        .expect("video import command should exist");

    assert!(command.contains("pub async fn probe_local_mp4"));
    assert!(command.contains("spawn_blocking"));
    assert!(!command.contains("hash_mp4_sha256_dto"));
}

#[test]
fn voice_clone_prepare_is_session_scoped_without_a_prepared_cache_or_warmup_branch() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the desktop crate");
    let start = source
        .find("fn prepare_voice_clone_source_blocking(")
        .expect("prepare blocking function should exist");
    let tail = &source[start..];
    let end = tail.find("\n#[tauri::command]").unwrap_or(tail.len());
    let function = &tail[..end];

    assert!(function
        .contains("let source_sha256 = voice_clone_prepare_session_identity(&operation_id);"));
    for removed in [
        "VOICE_CLONE_PREPARED_CACHE_VERSION",
        "VoiceClonePreparedCache",
        "load_cached_voice_clone_prepared_source",
        "write_voice_clone_prepared_cache",
        "--warmup-json",
        "voice_clone_source_identity",
    ] {
        assert!(
            !source.contains(removed),
            "prepared cross-session cache symbol must be removed: {removed}"
        );
    }
}

#[test]
fn voice_clone_prepare_publishes_busy_only_after_fallible_request_setup() {
    let commands = commands_source();
    let function = function_body(&commands, "fn prepare_voice_clone_source_blocking(");

    let create_dir = function
        .find("std::fs::create_dir_all(&prepared_root)")
        .expect("prepare should create its session directory");
    let write_request = function
        .find("write_json_file(&request_json, &request_payload)")
        .expect("prepare should write its worker request");
    let publish_busy = function
        .find("mark_voice_clone_preparing")
        .expect("prepare should publish its busy state");

    assert!(create_dir < publish_busy);
    assert!(write_request < publish_busy);
}

#[test]
fn cache_cleanup_is_serialized_against_voice_clone_worker_launches() {
    let commands = commands_source();
    let function = function_body(&commands, "pub fn cleanup_local_caches_command(");

    assert!(function.contains("lock_voice_clone_launch"));
    assert!(function.contains("reap_finished_voice_clone_worker"));
    assert!(function.contains("voice_clone_worker_is_running"));
    assert!(function.contains("voice_clone_cache_cleanup_busy"));
}

#[test]
fn voice_clone_prepare_becomes_reapable_before_its_final_playback_commit() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the desktop crate");
    let start = source
        .find("fn prepare_voice_clone_source_blocking(")
        .expect("prepare blocking function should exist");
    let tail = &source[start..];
    let end = tail.find("\n#[tauri::command]").unwrap_or(tail.len());
    let function = &tail[..end];
    let completed = function
        .rfind("completed_for_thread.store(true, Ordering::Release)")
        .expect("prepare worker should publish completion");
    let ready = function
        .find(".set_voice_clone_prepared_source(prepared_source)")
        .expect("prepare worker should commit the prepared source");

    assert!(
        completed < ready,
        "prepare task must be reapable before ready is published; Join closes the visibility window"
    );
}

#[test]
fn source_generation_changes_remove_the_prepared_session_cache() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the desktop crate");
    for function_name in ["probe_local_mp4_blocking", "stop_playback"] {
        let start = source
            .find(&format!("fn {function_name}("))
            .unwrap_or_else(|| panic!("{function_name} should exist"));
        let tail = &source[start..];
        let end = tail.find("\n#[tauri::command]").unwrap_or(tail.len());
        let function = &tail[..end];

        assert!(
            function.contains("voice_clone_artifact_dirs(playback, true)"),
            "{function_name} must remove the whole prepared session before changing generation"
        );
    }
}
