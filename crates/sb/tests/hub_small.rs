//! Small hub checks on a real tmux: the Work environment title and what a close
//! takes with it.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use switchboard_core::hub::{Hub, OpenReq, Sink, WORK_TITLE};

/// The tests set process environment variables, so they run one at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

struct Quiet;

impl Sink for Quiet {
    fn emit(&self, _event: &str, _payload: Value) {}
    fn output(&self, _pane: &str, _data: &[u8]) {}
}

struct Board {
    root: PathBuf,
    socket: String,
    hub: Arc<Hub>,
    repo: PathBuf,
}

impl Drop for Board {
    fn drop(&mut self) {
        switchboard_core::tmux::kill_server(&self.socket);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn board(name: &str) -> Board {
    let root = std::env::temp_dir().join(format!("sb-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("work/demo");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "x"][..],
    ] {
        assert!(Command::new("git").arg("-C").arg(&repo).args(args).status().unwrap().success());
    }
    let socket = format!("sb-test-{name}-{}", std::process::id());
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", &socket);
    std::env::remove_var("SB_SOCK");
    let hub = Hub::start(
        switchboard_core::paths::Paths::from_env(),
        PathBuf::from(env!("CARGO_BIN_EXE_sb")),
        Arc::new(Quiet),
    )
    .unwrap();
    Board { root, socket, hub, repo }
}

impl Board {
    fn open(&self, title: Option<&str>) -> String {
        let v = self
            .hub
            .open(OpenReq {
                repo: self.repo.to_string_lossy().into(),
                place: "main".into(),
                branch: None,
                tree: None,
                run: "shell".into(),
                title: title.map(str::to_owned),
                task: None,
                first: None,
            })
            .unwrap();
        self.hub.debug_set_kind(&v.id, "claude").unwrap();
        v.id
    }

    fn title(&self, id: &str) -> Option<String> {
        self.hub.views().into_iter().find(|p| p.id == id).and_then(|p| p.title)
    }
}

/// The window finds the Work environment agent by its title (SPEC 30).
#[test]
fn the_work_title_is_never_given_or_taken_by_a_rename() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let b = board("work-title");
    let agent = b.open(None);
    let work = b.open(Some(WORK_TITLE));
    assert!(b.hub.rename(&agent, Some(WORK_TITLE.into())).is_err());
    assert!(b.hub.rename(&agent, Some(format!(" {WORK_TITLE} "))).is_err());
    assert_eq!(b.title(&agent), None);
    assert!(b.hub.rename(&work, Some("mine".into())).is_err());
    assert!(b.hub.rename(&work, None).is_err());
    assert_eq!(b.title(&work).as_deref(), Some(WORK_TITLE));
    b.hub.rename(&agent, Some("mine".into())).unwrap();
    assert_eq!(b.title(&agent).as_deref(), Some("mine"));
}

/// A close removes the pane's state file with its settings file.
#[test]
fn a_close_takes_the_state_file() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let b = board("close-state");
    let id = b.open(None);
    let paths = switchboard_core::paths::Paths::from_env();
    let state = paths.state_dir().join(format!("{id}.json"));
    std::fs::write(&state, "{}").unwrap();
    b.hub.close(&id).unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while state.exists() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!state.exists());
}

/// An image from the window lands in `~/.switchboard/paste` on the hub, and the
/// pane gets its path as a paste with no Enter (SPEC 29.10).
#[test]
fn a_pasted_image_lands_on_the_hub_and_its_path_in_the_pane() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let b = board("paste-image");
    let id = b.open(None);
    let png = [137u8, 80, 78, 71, 13, 10, 26, 10, 1, 2, 3];
    assert!(b.hub.paste_image(&id, &png, "exe").is_err());
    assert!(b.hub.paste_image(&id, &[], "png").is_err());
    let dir = switchboard_core::paths::Paths::from_env().home.join("paste");
    // An old paste goes at the next paste; a new one stays.
    std::fs::create_dir_all(&dir).unwrap();
    let old = dir.join("1-000000000.png");
    std::fs::write(&old, b"x").unwrap();
    let week = std::time::SystemTime::now() - Duration::from_secs(8 * 24 * 3600);
    std::fs::File::options().write(true).open(&old).unwrap().set_modified(week).unwrap();
    // Every pane of the test server: the hub keeps more than the agent's.
    let screen = || {
        let tmux = |args: &[&str]| String::from_utf8_lossy(&Command::new("tmux").arg("-L").arg(&b.socket).args(args).output().unwrap().stdout).into_owned();
        let ids = tmux(&["list-panes", "-a", "-F", "#{pane_id}"]);
        ids.lines().map(|p| tmux(&["capture-pane", "-p", "-J", "-t", p])).collect::<Vec<_>>().join("\n")
    };
    // The shell reads input once its prompt shows.
    let end = Instant::now() + Duration::from_secs(5);
    while !screen().lines().any(|l| l.trim_end().ends_with('$')) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    let path = b.hub.paste_image(&id, &png, "png").unwrap();
    assert!(path.starts_with(&dir.to_string_lossy().into_owned()) && path.ends_with(".png"), "{path}");
    assert_eq!(std::fs::read(&path).unwrap(), png);
    assert!(!old.exists());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    let end = Instant::now() + Duration::from_secs(5);
    while !screen().contains(&path) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    let s = screen();
    assert!(s.contains(&path), "{s}");
    // No Enter: the shell did not run the path.
    assert!(!s.contains("Permission denied") && !s.contains("command not found"), "{s}");
}
