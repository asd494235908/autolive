use serde_json::Value;

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

pub fn validate_release_runtime_resource_config(
    base_config: &str,
    external_override: Option<&str>,
) -> Result<(), &'static str> {
    let base_config: Value =
        serde_json::from_str(base_config).map_err(|_| "tauri.conf.json 不是有效 JSON")?;
    if !has_expected_resources(&base_config) {
        return Err("release/custom 构建要求 bundle.resources 精确为 runtime-resources.json");
    }
    let Some(external_override) = external_override else {
        return Ok(());
    };
    let external_override: Value = serde_json::from_str(external_override)
        .map_err(|_| "TAURI_CONFIG 不是有效 JSON merge patch")?;
    let Some(external_override) = external_override.as_object() else {
        return Err("TAURI_CONFIG 不能替换 Tauri 配置根对象");
    };
    let Some(bundle_override) = external_override.get("bundle") else {
        return Ok(());
    };
    let Some(bundle_override) = bundle_override.as_object() else {
        return Err("TAURI_CONFIG 不能移除或替换 bundle.resources");
    };
    if let Some(resources) = bundle_override.get("resources") {
        if !has_expected_resources(resources) {
            return Err("TAURI_CONFIG 不能移除、增加或替换 bundle.resources");
        }
    }
    Ok(())
}

fn has_expected_resources(value: &Value) -> bool {
    let Some(resources) = value
        .get("bundle")
        .and_then(Value::as_object)
        .and_then(|bundle| bundle.get("resources"))
    else {
        return value == &Value::Array(vec![Value::String("runtime-resources.json".into())]);
    };
    resources == &Value::Array(vec![Value::String("runtime-resources.json".into())])
}
