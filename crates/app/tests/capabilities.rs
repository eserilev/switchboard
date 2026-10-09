//! The window needs `core:default`, or every `listen` call fails and no pane output reaches it.

#[test]
fn main_window_may_listen_to_events() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/capabilities/default.json"
    ))
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(v["windows"].as_array().unwrap().iter().any(|w| w == "main"));
    assert!(v["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p == "core:default" || p == "core:event:default"));
}
