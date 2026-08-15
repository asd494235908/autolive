pub fn should_use_empty_resource_override(
    profile: Option<&str>,
    manifest_exists: bool,
    override_exists: bool,
) -> bool {
    matches!(profile, Some("debug" | "test")) && !manifest_exists && !override_exists
}
