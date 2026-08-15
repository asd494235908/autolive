#[path = "../build_support.rs"]
mod build_support;

#[test]
fn release_and_custom_profiles_fail_closed_without_the_manifest() {
    use build_support::RuntimeResourceConfigAction;

    assert_eq!(
        build_support::runtime_resource_config_action(Some("release"), false, false),
        Err("release/custom 构建缺少 runtime-resources.json"),
    );
    assert_eq!(
        build_support::runtime_resource_config_action(Some("release"), false, true),
        Err("release/custom 构建缺少 runtime-resources.json"),
    );
    assert_eq!(
        build_support::runtime_resource_config_action(Some("custom"), false, true),
        Err("release/custom 构建缺少 runtime-resources.json"),
    );
    assert_eq!(
        build_support::runtime_resource_config_action(Some("release"), true, false),
        Ok(RuntimeResourceConfigAction::Keep),
    );
}

#[test]
fn debug_and_test_profiles_only_add_an_empty_override_when_needed() {
    use build_support::RuntimeResourceConfigAction;

    assert_eq!(
        build_support::runtime_resource_config_action(Some("debug"), false, false),
        Ok(RuntimeResourceConfigAction::UseEmptyDevelopmentOverride),
    );
    assert_eq!(
        build_support::runtime_resource_config_action(Some("test"), false, false),
        Ok(RuntimeResourceConfigAction::UseEmptyDevelopmentOverride),
    );
    assert_eq!(
        build_support::runtime_resource_config_action(Some("debug"), false, true),
        Ok(RuntimeResourceConfigAction::Keep),
    );
    assert_eq!(
        build_support::runtime_resource_config_action(Some("debug"), true, false),
        Ok(RuntimeResourceConfigAction::Keep),
    );
}
