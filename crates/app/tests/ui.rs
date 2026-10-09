//! Runs the window checks in headless Chromium (`web-tests/run.sh`).
//! With no Chromium on the machine, the script prints "skip" and passes.

#[test]
fn window_checks_pass() {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../web-tests/run.sh");
    let out = std::process::Command::new("bash")
        .arg(script)
        .output()
        .expect("run web-tests/run.sh");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
}
