#[derive(Debug, PartialEq, Eq)]
pub enum RuntimeResourceConfigAction {
    Keep,
    UseEmptyDevelopmentOverride,
}

pub fn runtime_resource_config_action(
    profile: Option<&str>,
    manifest_exists: bool,
    override_exists: bool,
) -> Result<RuntimeResourceConfigAction, &'static str> {
    if manifest_exists || (matches!(profile, Some("debug" | "test")) && override_exists) {
        return Ok(RuntimeResourceConfigAction::Keep);
    }
    if matches!(profile, Some("debug" | "test")) {
        return Ok(RuntimeResourceConfigAction::UseEmptyDevelopmentOverride);
    }
    Err("release/custom 构建缺少 runtime-resources.json")
}
