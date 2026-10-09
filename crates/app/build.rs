fn main() {
    // Tauri embeds the window files when the app compiles. Cargo does not see
    // them, so a change to only `web/` would not rebuild the app without this.
    println!("cargo:rerun-if-changed=../../web");
    tauri_build::build()
}
