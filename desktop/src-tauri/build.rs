mod build_support;

fn main() {
    let profile = std::env::var("PROFILE").ok();
    let external_override = std::env::var("TAURI_CONFIG").ok();
    let action = build_support::runtime_resource_config_action(
        profile.as_deref(),
        std::path::Path::new("runtime-resources.json").is_file(),
        external_override.as_deref(),
    )
    .unwrap_or_else(|message| panic!("{message}"));
    if !matches!(profile.as_deref(), Some("debug" | "test")) {
        build_support::validate_release_runtime_resource_config(
            include_str!("tauri.conf.json"),
            external_override.as_deref(),
        )
        .unwrap_or_else(|message| panic!("{message}"));
    }
    if let build_support::RuntimeResourceConfigAction::UseDevelopmentOverride(config) = action {
        // build.rs 和 generate_context! 分属两个编译阶段，二者必须看到同一覆盖值。
        std::env::set_var("TAURI_CONFIG", &config);
        println!("cargo:rustc-env=TAURI_CONFIG={config}");
    }
    tauri_build::build()
}
