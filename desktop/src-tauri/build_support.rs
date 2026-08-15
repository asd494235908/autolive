use serde_json::{Map, Value};

#[derive(Debug, PartialEq, Eq)]
pub enum RuntimeResourceConfigAction {
    Keep,
    UseDevelopmentOverride(String),
}

pub fn runtime_resource_config_action(
    profile: Option<&str>,
    manifest_exists: bool,
    external_override: Option<&str>,
) -> Result<RuntimeResourceConfigAction, &'static str> {
    if manifest_exists {
        return Ok(RuntimeResourceConfigAction::Keep);
    }
    if matches!(profile, Some("debug" | "test")) {
        return Ok(RuntimeResourceConfigAction::UseDevelopmentOverride(
            merge_development_override(external_override)?,
        ));
    }
    Err("release/custom 构建缺少 runtime-resources.json")
}

fn merge_development_override(external_override: Option<&str>) -> Result<String, &'static str> {
    let mut override_value = match external_override {
        Some(value) => {
            serde_json::from_str(value).map_err(|_| "TAURI_CONFIG 不是有效 JSON merge patch")?
        }
        None => Value::Object(Map::new()),
    };
    let root = override_value
        .as_object_mut()
        .ok_or("TAURI_CONFIG 不能替换 Tauri 配置根对象")?;
    let bundle = root
        .entry("bundle")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or("TAURI_CONFIG 不能替换 bundle 对象")?;
    bundle.insert("resources".into(), Value::Array(Vec::new()));
    serde_json::to_string(&override_value).map_err(|_| "无法生成开发 TAURI_CONFIG")
}

pub fn validate_release_runtime_resource_config(
    base_config: &str,
    external_override: Option<&str>,
) -> Result<(), &'static str> {
    let base_config: Value =
        serde_json::from_str(base_config).map_err(|_| "tauri.conf.json 不是有效 JSON")?;
    if !has_expected_config_resources(&base_config) {
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
        if !has_expected_resource_list(resources) {
            return Err("TAURI_CONFIG 不能移除、增加或替换 bundle.resources");
        }
    }
    Ok(())
}

fn has_expected_config_resources(config: &Value) -> bool {
    config
        .get("bundle")
        .and_then(Value::as_object)
        .and_then(|bundle| bundle.get("resources"))
        .is_some_and(has_expected_resource_list)
}

fn has_expected_resource_list(value: &Value) -> bool {
    value.as_array().is_some_and(|resources| {
        resources.len() == 1 && resources[0].as_str() == Some("runtime-resources.json")
    })
}
