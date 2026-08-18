#[test]
fn final_effect_window_loads_the_bundled_index_without_a_query_path() {
    let source = std::fs::read_to_string("src/commands.rs")
        .expect("commands.rs should be readable from the crate root");
    let start = source
        .find("pub async fn open_final_effect_window(")
        .expect("open_final_effect_window should exist");
    let end = source[start..]
        .find("#[tauri::command]\npub fn close_final_effect_window(")
        .map(|offset| start + offset)
        .unwrap_or(source.len());
    let command = &source[start..end];

    assert!(command.contains("WebviewUrl::App(\"index.html\".into())"));
    assert!(!command.contains("index.html?"));
    assert!(command.contains("request: Option<ResizeFinalEffectWindowRequestDto>"));
    assert!(command.contains("resize_final_effect_window_for_app"));
}
