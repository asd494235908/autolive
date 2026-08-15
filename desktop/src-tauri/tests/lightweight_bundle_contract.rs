#[path = "../build_support.rs"]
mod build_support;

#[test]
fn release_and_custom_profiles_fail_closed_without_the_manifest() {
    use build_support::RuntimeResourceConfigAction;

    assert_eq!(
        build_support::runtime_resource_config_action(Some("release"), false, None),
        Err("release/custom 构建缺少 runtime-resources.json"),
    );
    assert_eq!(
        build_support::runtime_resource_config_action(Some("release"), false, Some("{}")),
        Err("release/custom 构建缺少 runtime-resources.json"),
    );
    assert_eq!(
        build_support::runtime_resource_config_action(Some("custom"), false, Some("{}")),
        Err("release/custom 构建缺少 runtime-resources.json"),
    );
    assert_eq!(
        build_support::runtime_resource_config_action(Some("release"), true, None),
        Ok(RuntimeResourceConfigAction::Keep),
    );
}

#[test]
fn debug_and_test_profiles_merge_empty_resources_into_the_existing_override() {
    use build_support::RuntimeResourceConfigAction;
    use serde_json::{json, Value};

    let assert_development_override = |input: Option<&str>, expected: Value| {
        let action = build_support::runtime_resource_config_action(Some("debug"), false, input)
            .expect("debug 缺清单必须能构造开发 override");
        let RuntimeResourceConfigAction::UseDevelopmentOverride(override_json) = action else {
            panic!("debug 缺清单必须返回合并后的开发 override");
        };
        assert_eq!(
            serde_json::from_str::<Value>(&override_json).expect("override 必须是 JSON"),
            expected,
        );
    };

    assert_development_override(None, json!({ "bundle": { "resources": [] } }));
    assert_development_override(
        Some(r#"{"productName":"test"}"#),
        json!({ "productName": "test", "bundle": { "resources": [] } }),
    );
    assert_development_override(
        Some(r#"{"bundle":{"active":false}}"#),
        json!({ "bundle": { "active": false, "resources": [] } }),
    );
    let test_action = build_support::runtime_resource_config_action(Some("test"), false, None)
        .expect("test 缺清单必须能构造开发 override");
    assert!(matches!(
        test_action,
        RuntimeResourceConfigAction::UseDevelopmentOverride(_)
    ));

    for invalid_override in ["{", "[]", r#"{"bundle":null}"#] {
        assert!(
            build_support::runtime_resource_config_action(
                Some("debug"),
                false,
                Some(invalid_override),
            )
            .is_err(),
            "{invalid_override} 不能替换 Tauri 配置根对象或 bundle",
        );
    }

    assert_eq!(
        build_support::runtime_resource_config_action(
            Some("debug"),
            true,
            Some("{\"productName\":\"test\"}")
        ),
        Ok(RuntimeResourceConfigAction::Keep),
    );
}

#[test]
fn release_config_allows_unrelated_override_but_rejects_resource_changes() {
    let base_config = include_str!("../tauri.conf.json");

    assert_eq!(
        build_support::runtime_resource_config_action(Some("release"), true, Some("{}")),
        Ok(build_support::RuntimeResourceConfigAction::Keep),
    );
    assert!(build_support::validate_release_runtime_resource_config(
        base_config,
        Some(r#"{"productName":"test build"}"#),
    )
    .is_ok());
    assert!(build_support::validate_release_runtime_resource_config(
        base_config,
        Some(r#"{"bundle":{"resources":["runtime-resources.json"]}}"#),
    )
    .is_ok());
    assert!(build_support::validate_release_runtime_resource_config(
        base_config,
        Some(r#"{"bundle":{"resources":[]}}"#),
    )
    .is_err());
    assert!(build_support::validate_release_runtime_resource_config(
        base_config,
        Some(r#"{"bundle":{"resources":null}}"#),
    )
    .is_err());
    assert!(build_support::validate_release_runtime_resource_config(
        base_config,
        Some(r#"{"bundle":null}"#),
    )
    .is_err());
    assert!(build_support::validate_release_runtime_resource_config(
        base_config,
        Some(r#"{"bundle":{"resources":["runtime-resources.json","extra.json"]}}"#),
    )
    .is_err());
    assert!(build_support::validate_release_runtime_resource_config(
        base_config,
        Some(r#"{"bundle":{"resources":{"bundle":{"resources":["runtime-resources.json"]}}}}"#),
    )
    .is_err());
    assert!(build_support::validate_release_runtime_resource_config(
        r#"{"bundle":{"resources":[]}}"#,
        None,
    )
    .is_err());
}
