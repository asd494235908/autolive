#[path = "../build_support.rs"]
mod build_support;

#[test]
fn empty_runtime_resource_override_is_limited_to_debug_and_test_profiles() {
    assert!(build_support::should_use_empty_resource_override(
        Some("debug"),
        false,
        false,
    ));
    assert!(build_support::should_use_empty_resource_override(
        Some("test"),
        false,
        false,
    ));
    assert!(!build_support::should_use_empty_resource_override(
        Some("release"),
        false,
        false,
    ));
    assert!(!build_support::should_use_empty_resource_override(
        Some("custom"),
        false,
        false,
    ));
    assert!(!build_support::should_use_empty_resource_override(
        Some("debug"),
        true,
        false,
    ));
    assert!(!build_support::should_use_empty_resource_override(
        Some("debug"),
        false,
        true,
    ));
}
