mod build_support;

fn main() {
    let profile = std::env::var("PROFILE").ok();
    if build_support::should_use_empty_resource_override(
        profile.as_deref(),
        std::path::Path::new("runtime-resources.json").is_file(),
        std::env::var_os("TAURI_CONFIG").is_some(),
    ) {
        let test_config = r#"{"bundle":{"resources":[]}}"#;
        // build.rs 和 generate_context! 分属两个编译阶段，二者必须看到同一覆盖值。
        std::env::set_var("TAURI_CONFIG", test_config);
        println!("cargo:rustc-env=TAURI_CONFIG={test_config}");
    }
    tauri_build::build()
}
