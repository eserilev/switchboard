//! The parts of the live pane (SPEC 28) on a real tmux server: a shell and nvim
//! next to an agent, each on screen at its own size.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use switchboard_core::hub::{Hub, OpenReq, Sink};

#[derive(Default)]
struct Events {
    out: Mutex<Vec<(String, Vec<u8>)>>,
    emitted: Mutex<Vec<(String, Value)>>,
}

impl Sink for Events {
    fn emit(&self, event: &str, payload: Value) {
        self.emitted.lock().unwrap().push((event.to_owned(), payload));
    }
    fn output(&self, pane: &str, data: &[u8]) {
        self.out.lock().unwrap().push((pane.to_owned(), data.to_vec()));
    }
}

impl Events {
    fn text_of(&self, pane: &str) -> String {
        let out = self.out.lock().unwrap();
        out.iter()
            .filter(|(p, _)| p == pane)
            .map(|(_, d)| String::from_utf8_lossy(d).into_owned())
            .collect()
    }
}

fn wait_for(what: &str, secs: u64, f: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out: {what}");
}

#[test]
fn parts_stream_at_their_own_size() {
    let root = std::env::temp_dir().join(format!("sb-parts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("work/demo");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(Command::new("git").arg("-C").arg(&repo).args(["init", "-q", "-b", "main"]).status().unwrap().success());
    let socket = format!("sb-test-parts-{}", std::process::id());
    std::env::set_var("SB_HOME", &root);
    std::env::set_var("SB_TMUX_SOCKET", &socket);
    std::env::remove_var("SB_SOCK");
    std::env::set_var("NVIM_APPNAME", format!("sb-test-nvim-{}", std::process::id()));
    std::env::set_var("SHELL", "/bin/sh");
    let events = Arc::new(Events::default());
    let hub = Hub::start(
        switchboard_core::paths::Paths::from_env(),
        PathBuf::from(env!("CARGO_BIN_EXE_sb")),
        events.clone(),
    )
    .unwrap();
    struct Kill(String);
    impl Drop for Kill {
        fn drop(&mut self) {
            switchboard_core::tmux::kill_server(&self.0);
        }
    }
    let _kill = Kill(socket.clone());
    let size_of = |pane: &str| -> String {
        let out = Command::new("tmux")
            .args(["-L", &socket, "list-panes", "-a", "-F", "#{@sb_pane} #{window_width}x#{window_height}"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{pane} ")).map(str::to_owned))
            .unwrap_or_default()
    };
    let live = |id: &str| hub.views().iter().any(|p| p.id == id && p.live);

    // An agent: a shell pane that the hub takes as a claude pane.
    let agent = hub
        .open(OpenReq {
            repo: repo.to_string_lossy().into(),
            place: "main".into(),
            branch: None,
            tree: None,
            run: "shell".into(),
            title: None,
            task: None,
            first: None,
        })
        .unwrap()
        .id;
    hub.debug_set_kind(&agent, "claude").unwrap();

    // One shell for each worktree.
    let shell = hub.shell_open(&repo).unwrap();
    assert_ne!(shell, agent);
    assert_eq!(hub.shell_open(&repo).unwrap(), shell);
    assert!(hub.views().iter().any(|p| p.id == shell && p.kind == "shell"));

    // A part needs a live pane.
    assert!(hub.attach(&shell, &agent, 50, 12).is_err());

    hub.resize(100, 30).unwrap();
    hub.expand(&agent).unwrap();
    wait_for("the agent window size", 3, || size_of(&agent) == "100x30");
    let screen = hub.attach(&shell, &agent, 50, 12).unwrap();
    assert!(screen.starts_with(b"\x1b[H"));
    assert!(live(&agent) && live(&shell));
    wait_for("the shell window size", 3, || size_of(&shell) == "50x12");
    // A part size leaves the agent as it is, after the redraw of the expand.
    wait_for("the agent keeps its size", 3, || size_of(&agent) == "100x30");

    // Its output reaches the window with its own id.
    hub.input(&shell, b"echo part-$((40+2))\r").unwrap();
    wait_for("shell output", 5, || events.text_of(&shell).contains("part-42"));
    assert!(!events.text_of(&agent).contains("part-42"));

    // A part resize sizes that window only.
    hub.part_resize(&shell, 60, 14).unwrap();
    wait_for("the new shell size", 3, || size_of(&shell) == "60x14");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(size_of(&agent), "100x30");

    // A command that runs marks the shell busy, and its end clears it.
    hub.input(&shell, b"sleep 2\r").unwrap();
    wait_for("busy", 5, || hub.views().iter().any(|p| p.id == shell && p.busy));
    wait_for("not busy", 6, || hub.views().iter().any(|p| p.id == shell && !p.busy));

    // A detach takes the part off the screen; the shell keeps running.
    hub.detach(&shell);
    assert!(!live(&shell) && live(&agent));
    let before = events.text_of(&shell).len();
    hub.input(&shell, b"echo after-$((1+1))\r").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(events.text_of(&shell).len(), before, "no output after a detach");
    assert!(hub.views().iter().any(|p| p.id == shell));
    // A detach never takes the live pane itself.
    hub.detach(&agent);
    assert!(live(&agent));

    // A late attach for another agent does nothing.
    assert!(hub.attach(&shell, &shell, 50, 12).is_err());
    assert!(!live(&shell));

    // A resize right after an expand stays: the redraw reads the size again.
    hub.expand(&agent).unwrap();
    hub.resize(80, 24).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(size_of(&agent), "80x24");
    hub.resize(100, 30).unwrap();

    // A new expand takes the old parts off.
    hub.attach(&shell, &agent, 50, 12).unwrap();
    hub.expand(&agent).unwrap();
    assert!(!live(&shell) && live(&agent));

    // A close of a part takes it off the screen.
    hub.attach(&shell, &agent, 50, 12).unwrap();
    hub.close(&shell).unwrap();
    assert!(!live(&shell) && live(&agent));

    // A close of the live pane takes its parts off too.
    let other = hub.shell_open(&root).unwrap();
    hub.attach(&other, &agent, 40, 10).unwrap();
    let agent2 = hub
        .open(OpenReq {
            repo: repo.to_string_lossy().into(),
            place: "main".into(),
            branch: None,
            tree: None,
            run: "shell".into(),
            title: None,
            task: None,
            first: None,
        })
        .unwrap()
        .id;
    hub.expand(&agent2).unwrap();
    hub.attach(&other, &agent2, 40, 10).unwrap();
    hub.close(&agent2).unwrap();
    assert!(!hub.views().iter().any(|p| p.live));
    hub.close(&other).unwrap();
    hub.expand(&agent).unwrap();

    // nvim reports a buffer that is not saved.
    if Command::new("nvim").arg("--version").output().is_ok_and(|o| o.status.success()) {
        let nv = hub.nvim_open(&repo, None).unwrap();
        hub.attach(&nv, &agent, 70, 20).unwrap();
        wait_for("nvim starts", 5, || !events.text_of(&nv).is_empty());
        std::thread::sleep(Duration::from_millis(500));
        assert!(!hub.views().iter().any(|p| p.id == nv && p.modified));
        hub.input(&nv, b":enew\rihello\x1b").unwrap();
        wait_for("modified", 8, || hub.views().iter().any(|p| p.id == nv && p.modified));
        hub.input(&nv, b":bd!\r").unwrap();
        wait_for("saved again", 8, || hub.views().iter().any(|p| p.id == nv && !p.modified));
        hub.close(&nv).unwrap();
    }

    hub.collapse();
    assert!(!hub.views().iter().any(|p| p.live));
    let _ = std::fs::remove_dir_all(&root);
}
