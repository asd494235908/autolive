const COMMANDS_SOURCE: &str = include_str!("../src/commands.rs");
const BACKEND_SOURCE: &str = include_str!("../src/realtime_video_backend.rs");
const RUNTIME_SOURCE: &str = include_str!("../src/realtime_video_runtime.rs");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SyncCursor {
    playback_generation: u64,
    clock_epoch: u64,
    loop_index: u64,
    paused: bool,
}

#[derive(Debug, Clone, Copy)]
struct SyncRequest {
    playback_generation: u64,
    clock_epoch: u64,
    loop_index: u64,
    position_ms: u64,
    paused: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Boundary {
    None,
    UserSeek,
    Loop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Pause(bool),
    SeekAbsoluteMs(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncRejected {
    PlaybackGeneration,
    ClockEpoch,
    LoopIndex,
    MultipleLoops,
}

fn source_section<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    source
        .split_once(start)
        .unwrap_or_else(|| panic!("missing source contract start: {start}"))
        .1
        .split_once(end)
        .unwrap_or_else(|| panic!("missing source contract end: {end}"))
        .0
}

fn assert_in_order(source: &str, first: &str, second: &str) {
    let first_index = source
        .find(first)
        .unwrap_or_else(|| panic!("missing first source contract: {first}"));
    let second_index = source
        .find(second)
        .unwrap_or_else(|| panic!("missing second source contract: {second}"));
    assert!(
        first_index < second_index,
        "source contract must keep `{first}` before `{second}`"
    );
}

fn resolve_sync(
    current: SyncCursor,
    request: SyncRequest,
) -> Result<(SyncCursor, Boundary), SyncRejected> {
    if request.playback_generation != current.playback_generation {
        return Err(SyncRejected::PlaybackGeneration);
    }
    if request.clock_epoch < current.clock_epoch {
        return Err(SyncRejected::ClockEpoch);
    }
    if request.loop_index < current.loop_index {
        return Err(SyncRejected::LoopIndex);
    }
    if request.loop_index > current.loop_index.saturating_add(1) {
        return Err(SyncRejected::MultipleLoops);
    }
    let boundary = if request.loop_index > current.loop_index {
        Boundary::Loop
    } else if request.clock_epoch > current.clock_epoch {
        Boundary::UserSeek
    } else {
        Boundary::None
    };
    Ok((
        SyncCursor {
            playback_generation: request.playback_generation,
            clock_epoch: request.clock_epoch,
            loop_index: request.loop_index,
            paused: request.paused,
        },
        boundary,
    ))
}

fn command_batch(
    current: SyncCursor,
    next: SyncCursor,
    boundary: Boundary,
    position_ms: u64,
) -> Vec<Command> {
    let pause = (next.paused != current.paused).then_some(Command::Pause(next.paused));
    if boundary == Boundary::None {
        return pause.into_iter().collect();
    }
    let seek = Command::SeekAbsoluteMs(position_ms);
    if next.paused {
        vec![Command::Pause(true), seek]
    } else {
        vec![seek, Command::Pause(false)]
    }
}

fn apply_sync(
    current: &mut SyncCursor,
    request: SyncRequest,
) -> Result<Vec<Command>, SyncRejected> {
    let (next, boundary) = resolve_sync(*current, request)?;
    let commands = command_batch(*current, next, boundary, request.position_ms);
    *current = next;
    Ok(commands)
}

fn cursor(paused: bool) -> SyncCursor {
    SyncCursor {
        playback_generation: 7,
        clock_epoch: 10,
        loop_index: 3,
        paused,
    }
}

fn request(clock_epoch: u64, loop_index: u64, position_ms: u64, paused: bool) -> SyncRequest {
    SyncRequest {
        playback_generation: 7,
        clock_epoch,
        loop_index,
        position_ms,
        paused,
    }
}

#[test]
fn seek_command_forwards_only_generation_and_position_to_runtime_intent() {
    let dto = source_section(
        COMMANDS_SOURCE,
        "pub struct SeekPlaybackRequestDto {",
        "\n}",
    );
    assert!(dto.contains("pub playback_generation: u64"));
    assert!(dto.contains("pub position_ms: u64"));
    assert!(!dto.contains("clock_epoch"));
    assert!(!dto.contains("loop_index"));
    assert!(!dto.contains("backend_epoch"));

    let handler = source_section(
        COMMANDS_SOURCE,
        "pub fn seek_playback(",
        "\n#[tauri::command]",
    );
    assert!(handler.contains("request.playback_generation != current.playback_generation"));
    assert!(handler.contains("request.position_ms >= duration_ms"));
    assert!(handler.contains("PlaybackIntent::Seek"));
    assert!(handler.contains("position_ms: request.position_ms"));

    let runtime_intent = source_section(RUNTIME_SOURCE, "pub enum PlaybackIntent {", "\n}");
    assert!(runtime_intent.contains("Seek { position_ms: u64 }"));
}

#[test]
fn renderer_session_requires_a_positive_source_duration() {
    for contract in ["PrepareRealtimeRenderer", "PrepareOriginalRenderer"] {
        let definition =
            source_section(RUNTIME_SOURCE, &format!("pub struct {contract} {{"), "\n}");
        assert!(
            definition.contains("pub source_duration_ms: u64"),
            "{contract} must carry the probed source duration"
        );
    }

    let session = source_section(RUNTIME_SOURCE, "struct RendererSessionContext {", "\n}");
    assert!(session.contains("source_duration_ms: u64"));

    let configure = source_section(
        COMMANDS_SOURCE,
        "pub async fn configure_realtime_video_cycle(",
        "\nfn command_error_from_realtime_video_prepare",
    );
    assert!(configure.contains("source_duration_ms"));
    assert!(configure.contains(".duration_ms"));

    let original = source_section(
        COMMANDS_SOURCE,
        "pub async fn ensure_original_video_renderer(",
        "\n#[tauri::command(async)]\npub async fn configure_realtime_video_cycle",
    );
    assert!(original.contains("source_duration_ms"));
    assert!(original.contains(".duration_ms"));
}

#[test]
fn av_sync_uses_checked_segment_source_pts_and_rejects_eof_seek() {
    let observe = source_section(RUNTIME_SOURCE, "    fn observe_av_sync(", "\n    fn tick(");
    for contract in [
        "MediaSegmentIdentity::try_new",
        "session.source_duration_ms",
        "snapshot_for_segment",
        "audible_source_pts_ms",
        "mpv_source_pts_ms",
        "target_source_pts_ms",
        "target >= session_source_duration_ms",
    ] {
        assert!(
            observe.contains(contract),
            "missing AV-sync contract: {contract}"
        );
    }
    assert!(!observe.contains("snapshot.audible_pts_ms"));
    assert!(!observe.contains("audible_audio_pts_ms:"));
    assert!(!observe.contains("mpv_presented_pts_ms:"));
    assert!(observe.contains("playing: snapshot.playing && !cursor.paused,"));
    assert!(!observe.contains("&& !playback_state.paused"));
    assert_in_order(
        observe,
        "MpvCommand::SeekAbsoluteMs",
        "MpvCommand::SetPause { paused: false }",
    );

    let source_duration_ms = 72_300_u64;
    let loop_index = 20_u64;
    let source_pts_ms = 27_000_u64;
    let presentation_pts_ms = loop_index
        .checked_mul(source_duration_ms)
        .and_then(|base| base.checked_add(source_pts_ms));
    assert_eq!(presentation_pts_ms, Some(1_473_000));
    let valid_seek_target = |target_ms| target_ms < source_duration_ms;
    assert!(valid_seek_target(source_pts_ms));
    assert!(!valid_seek_target(source_duration_ms));
}

#[test]
fn runtime_seeks_once_only_when_clock_or_loop_epoch_advances() {
    let transition = source_section(
        RUNTIME_SOURCE,
        "fn resolve_sync_transition(",
        "\nfn sync_command_batch(",
    );
    assert_in_order(
        transition,
        "let boundary = if request.loop_index > current.loop_index {",
        "} else if request.clock_epoch > current.clock_epoch {",
    );
    assert_in_order(
        transition,
        "} else if request.clock_epoch > current.clock_epoch {",
        "VideoScheduleBoundary::None",
    );

    let actor_sync = source_section(
        RUNTIME_SOURCE,
        "    fn synchronize(\n        &mut self,",
        "\n    fn tick(",
    );
    assert!(actor_sync.contains("resolve_sync_transition(cursor, request)?"));
    assert!(actor_sync.contains("sync_command_batch(cursor, transition, request.position_ms)"));
    assert!(actor_sync.contains("self.discard_pending_outside_cursor(transition.next)"));
    assert_in_order(
        actor_sync,
        "for command in commands.into_iter().flatten()",
        "self.sync_cursor = Some(transition.next)",
    );

    let batch = source_section(
        RUNTIME_SOURCE,
        "fn sync_command_batch(",
        "\nfn stale_sync_error(",
    );
    assert_eq!(batch.matches("MpvCommand::SeekAbsoluteMs").count(), 1);
    assert_in_order(
        batch,
        "matches!(transition.boundary, VideoScheduleBoundary::None)",
        "MpvCommand::SeekAbsoluteMs",
    );

    let mut current = cursor(false);
    assert_eq!(
        apply_sync(&mut current, request(10, 3, 2_100, false)),
        Ok(vec![])
    );
    assert_eq!(
        apply_sync(&mut current, request(11, 3, 4_500, false)),
        Ok(vec![Command::SeekAbsoluteMs(4_500), Command::Pause(false),])
    );
    assert_eq!(
        apply_sync(&mut current, request(11, 3, 4_500, false)),
        Ok(vec![])
    );
    assert_eq!(
        apply_sync(&mut current, request(11, 3, 4_900, false)),
        Ok(vec![])
    );
    assert_eq!(
        apply_sync(&mut current, request(11, 4, 0, false)),
        Ok(vec![Command::SeekAbsoluteMs(0), Command::Pause(false)])
    );
    assert_eq!(
        apply_sync(&mut current, request(11, 4, 0, false)),
        Ok(vec![])
    );
}

#[test]
fn pause_and_resume_seek_command_order_is_stable() {
    let batch = source_section(
        RUNTIME_SOURCE,
        "fn sync_command_batch(",
        "\nfn stale_sync_error(",
    );
    assert!(batch.contains("[pause, seek]"));
    assert!(batch.contains("[seek, resume]"));

    let mut playing = cursor(false);
    assert_eq!(
        apply_sync(&mut playing, request(11, 3, 6_000, true)),
        Ok(vec![Command::Pause(true), Command::SeekAbsoluteMs(6_000)])
    );

    let mut paused = cursor(true);
    assert_eq!(
        apply_sync(&mut paused, request(11, 3, 6_000, false)),
        Ok(vec![Command::SeekAbsoluteMs(6_000), Command::Pause(false)])
    );

    let mut already_playing = cursor(false);
    assert_eq!(
        apply_sync(&mut already_playing, request(10, 4, 0, false)),
        Ok(vec![Command::SeekAbsoluteMs(0), Command::Pause(false)])
    );

    let mut user_paused = cursor(true);
    assert_eq!(
        apply_sync(&mut user_paused, request(11, 3, 6_000, true)),
        Ok(vec![Command::Pause(true), Command::SeekAbsoluteMs(6_000)])
    );

    let mut ordinary_tick = cursor(false);
    assert_eq!(
        apply_sync(&mut ordinary_tick, request(10, 3, 6_500, false)),
        Ok(vec![])
    );
}

#[test]
fn stale_generation_epoch_and_multi_loop_sync_fail_closed() {
    let transition = source_section(
        RUNTIME_SOURCE,
        "fn resolve_sync_transition(",
        "\nfn sync_command_batch(",
    );
    for guard in [
        "request.playback_generation != current.playback_generation",
        "request.clock_epoch < current.clock_epoch",
        "request.loop_index < current.loop_index",
        "request.loop_index > current.loop_index.saturating_add(1)",
    ] {
        assert!(
            transition.contains(guard),
            "missing fail-closed guard: {guard}"
        );
    }
    assert_in_order(transition, "saturating_add(1)", "let boundary =");

    let initial = cursor(false);
    let rejected = [
        (
            SyncRequest {
                playback_generation: 6,
                ..request(10, 3, 1_000, false)
            },
            SyncRejected::PlaybackGeneration,
        ),
        (request(9, 3, 1_000, false), SyncRejected::ClockEpoch),
        (request(10, 2, 1_000, false), SyncRejected::LoopIndex),
        (request(11, 5, 0, false), SyncRejected::MultipleLoops),
    ];

    for (sync, expected) in rejected {
        let mut current = initial;
        assert_eq!(apply_sync(&mut current, sync), Err(expected));
        assert_eq!(current, initial, "rejected sync must not mutate the cursor");
    }
}

#[test]
fn sync_failures_are_typed_and_mapped_without_message_matching() {
    let error = source_section(
        BACKEND_SOURCE,
        "pub enum RealtimeVideoBackendError {",
        "\n}\n\nimpl fmt::Display",
    );
    assert!(error.contains("StaleSync"));
    assert!(error.contains("SyncSuperseded"));
    assert!(error.contains("InvalidSync"));

    let mapper = source_section(
        COMMANDS_SOURCE,
        "fn command_error_from_realtime_video_sync(",
        "\n}\n\n#[tauri::command(async)]\npub fn stop_realtime_video_renderer",
    );
    for code in [
        "stale_realtime_video_sync",
        "realtime_video_sync_superseded",
        "realtime_video_sync_busy",
        "realtime_video_sync_result_unknown",
        "realtime_video_sync_transport_failed",
        "realtime_video_sync_invalid",
    ] {
        assert!(mapper.contains(code), "missing stable sync code: {code}");
    }
    assert!(mapper.contains("RealtimeVideoBackendError::RuntimeTimeout"));
    assert!(mapper.contains("RealtimeVideoBackendError::IpcTimeout"));
    assert!(!mapper.contains("message.contains"));
    assert!(!mapper.contains("realtime_video_sync_exhausted"));
}

#[test]
fn newer_sync_revision_supersedes_queued_work() {
    let public_sync = source_section(
        RUNTIME_SOURCE,
        "    pub fn synchronize(",
        "\n    pub fn stop(",
    );
    assert!(public_sync.contains("operation_revision"));
    assert!(public_sync.contains("RuntimeCommand::Synchronize"));

    let actor_sync = source_section(
        RUNTIME_SOURCE,
        "    fn synchronize(\n        &mut self,",
        "\n    fn tick(",
    );
    assert!(actor_sync.contains("SyncSuperseded"));
    assert_in_order(
        actor_sync,
        "operation_revision",
        "resolve_sync_transition(cursor, request)?",
    );
}

#[test]
fn original_cannot_neutralize_a_healthy_effect_session() {
    let state = source_section(RUNTIME_SOURCE, "struct RuntimeState {", "\n}");
    assert!(state.contains("processing_enabled: bool"));

    let setter = source_section(
        RUNTIME_SOURCE,
        "    fn set_processing_enabled(",
        "\n    fn neutralize_to_source_with_fallback(",
    );
    assert!(setter.contains("self.processing_enabled = enabled"));

    let original = source_section(
        RUNTIME_SOURCE,
        "    fn ensure_original(\n        &mut self,",
        "\n    fn commit(",
    );
    assert!(original.contains("healthy_effect_session_for_generation"));
    assert_in_order(
        original,
        "healthy_effect_session_for_generation",
        "self.session = Some(RendererSessionContext",
    );

    let health = source_section(
        RUNTIME_SOURCE,
        "    fn healthy_effect_session_for_generation(",
        "\n    fn ensure_original(",
    );
    for guard in [
        "self.processing_enabled",
        "effect_session_is_owned_by_cycle",
        "self.cycle_controller.is_some()",
        "cycle_session_is_available",
        "session.playback_generation == playback_generation",
        "self.status.process_id.is_some()",
        "RendererLifecycleState::Active | RendererLifecycleState::Spawned",
    ] {
        assert!(health.contains(guard), "missing Original guard: {guard}");
    }
    assert!(health.contains("has_exited"));

    let ownership = source_section(
        RUNTIME_SOURCE,
        "fn effect_session_is_owned_by_cycle(",
        "\nfn renderer_host_window_changed(",
    );
    assert!(ownership.contains("VideoBackend::RealtimeGpu | VideoBackend::Cpu4"));
    assert!(ownership.contains("logical_backend == VideoBackend::Source"));
    assert!(ownership.contains("controller_present"));
    assert!(ownership.contains("cycle_session_available"));
}
