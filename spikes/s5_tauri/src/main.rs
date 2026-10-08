// S5: does a Tauri 2 window draw on this Wayland setup?
// The page calls `report` with what it found, and the app prints it and exits.
// If the page never reports in 10 s, the app exits with code 1.

#[tauri::command]
fn report(s: String) {
    println!("page: {s}");
    std::process::exit(0);
}

fn main() {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(10));
        println!("page: no report in 10 s");
        std::process::exit(1);
    });
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![report])
        .run(tauri::generate_context!())
        .expect("tauri run failed");
}
