#[test]
fn windows_job_dependency_is_exact_and_keeps_unsafe_forbidden() {
    let cargo = include_str!("../Cargo.toml");

    assert!(cargo.contains("[target.'cfg(windows)'.dependencies]"));
    assert!(cargo.contains("win32job = \"=2.0.3\""));
    assert!(cargo.contains("unsafe_code = \"forbid\""));
}

#[test]
fn job_owner_configures_kill_on_close_without_local_unsafe() {
    let source = include_str!("../src/realtime_video_job.rs");

    assert!(source.contains("ExtendedLimitInfo::new()"));
    assert!(source.contains("limit_kill_on_job_close()"));
    assert!(source.contains("Job::create_with_limit_info"));
    assert!(source.contains(".assign_process(child.as_raw_handle() as isize)"));
    assert!(!source.contains("unsafe {"));
}

#[test]
fn managed_mpv_process_owns_job_and_fails_closed_on_binding_error() {
    let source = include_str!("../src/realtime_video_backend.rs");
    let create = source
        .find("ManagedMpvJob::create().map_err")
        .expect("Job Object must be created before mpv starts");
    let spawn = source[create..]
        .find(".spawn()")
        .map(|offset| create + offset)
        .expect("managed mpv spawn must remain explicit");

    assert!(create < spawn, "Job Object creation must precede mpv spawn");
    assert!(source.contains("job: Option<ManagedMpvJob>"));
    assert!(source.contains("if let Err(error) = job.assign_process(&child)"));
    assert!(source.contains("terminate_unmanaged_child(&mut child)"));
    assert!(source.contains("let _job_kill_guard = self.job.take()"));
    assert!(source.contains("impl Drop for ManagedMpvProcess"));
    assert!(source.contains("let _ignored = self.cancel()"));
}
