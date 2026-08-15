mod build_support;

fn main() {
    let profile = std::env::var("PROFILE").ok();
    let action = build_support::runtime_resource_config_action(
        profile.as_deref(),
        std::path::Path::new("runtime-resources.json").is_file(),
        std::env::var_os("TAURI_CONFIG").is_some(),
    )
    .unwrap_or_else(|message| panic!("{message}"));
    if action == build_support::RuntimeResourceConfigAction::UseEmptyDevelopmentOverride {
        const TEST_CONFIG: &str = r#"{"bundle":{"resources":[]}}"#;
        // build.rs 和 generate_context! 分属两个编译阶段，二者必须看到同一覆盖值。
        std::env::set_var("TAURI_CONFIG", TEST_CONFIG);
        println!("cargo:rustc-env=TAURI_CONFIG={TEST_CONFIG}");
    }
    tauri_build::build()
}
