//! Small checks of the `sb` command line that need no hub.

use std::process::Command;

fn sb(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_sb"))
        .args(args)
        .env("SB_HOME", home)
        .env("HOME", home)
        .env_remove("SB_SOCK")
        .output()
        .unwrap()
}

#[test]
fn open_of_a_missing_folder_says_so() {
    let home = std::env::temp_dir().join(format!("sb-cli-open-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let out = sb(&home, &["open", "/no/such/place"]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no such folder: /no/such/place"), "{err}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn connect_add_keeps_quotes_in_their_strings() {
    let home = std::env::temp_dir().join(format!("sb-cli-conn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("claude")).unwrap();
    let url = "https://h/\"x\\y\nscan = 1";
    let out = sb(&home, &["connect", "add", "ep", "--endpoint", url, "--model", "m\"1"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let c = switchboard_core::config::Config::load(&home.join("config.toml")).unwrap();
    assert_eq!(c.connections["ep"].url.as_deref(), Some(url));
    assert_eq!(c.connections["ep"].model.as_deref(), Some("m\"1"));
    // A second add changes nothing.
    let out = sb(&home, &["connect", "add", "ep"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("already"));
    let _ = std::fs::remove_dir_all(&home);
}
