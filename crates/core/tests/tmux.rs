//! Tests against a real tmux server on a socket of its own.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};
use switchboard_core::tmux::{Event, Tmux, CONF};

struct Server {
    tmux: Arc<Tmux>,
    events: Receiver<Event>,
    dir: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.tmux.kill_server();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn server(name: &str) -> Server {
    let socket = format!("sb-test-{}-{name}", std::process::id());
    let dir = std::env::temp_dir().join(&socket);
    std::fs::create_dir_all(&dir).unwrap();
    let conf = dir.join("tmux.conf");
    std::fs::write(&conf, CONF).unwrap();
    let (tx, events) = mpsc::channel();
    let tmux = Tmux::start(&socket, &conf, move |e| {
        let _ = tx.send(e);
    })
    .expect("tmux starts");
    Server { tmux, events, dir }
}

/// Collects the output of one pane until `want` shows up, or 5 s pass.
fn output_until(s: &Server, pane: &str, want: &str) -> String {
    let end = Instant::now() + Duration::from_secs(5);
    let mut got = Vec::new();
    while Instant::now() < end {
        if let Ok(Event::Output { pane: p, data }) =
            s.events.recv_timeout(Duration::from_millis(100))
        {
            if p == pane {
                got.extend(data);
                if String::from_utf8_lossy(&got).contains(want) {
                    break;
                }
            }
        }
    }
    String::from_utf8_lossy(&got).into_owned()
}

fn drain(s: &Server) {
    while s.events.recv_timeout(Duration::from_millis(200)).is_ok() {}
}

#[test]
fn new_pane_streams_its_output() {
    let s = server("stream");
    let pane = s
        .tmux
        .new_pane(
            &s.dir,
            &["sh", "-c", "printf 'hello from sb'; sleep 5"],
            &[],
        )
        .unwrap();
    assert!(pane.starts_with('%'), "pane id: {pane}");
    assert!(output_until(&s, &pane, "hello from sb").contains("hello from sb"));
}

#[test]
fn arguments_with_quotes_survive() {
    let s = server("quote");
    let pane = s
        .tmux
        .new_pane(
            &s.dir,
            &["sh", "-c", "printf '%s' \"it's; fine\"; sleep 5"],
            &[],
        )
        .unwrap();
    assert!(output_until(&s, &pane, "it's; fine").contains("it's; fine"));
}

#[test]
fn env_reaches_the_pane() {
    let s = server("env");
    let pane = s
        .tmux
        .new_pane(
            &s.dir,
            &["sh", "-c", "printf \"pane=$SB_PANE\"; sleep 5"],
            &[("SB_PANE", "p7")],
        )
        .unwrap();
    assert!(output_until(&s, &pane, "pane=p7").contains("pane=p7"));
}

#[test]
fn send_bytes_types_into_the_pane() {
    let s = server("send");
    let pane = s.tmux.new_pane(&s.dir, &["cat"], &[]).unwrap();
    s.tmux
        .send_bytes(&pane, "typed ✓ text\r".as_bytes())
        .unwrap();
    assert!(output_until(&s, &pane, "typed ✓ text").contains("typed ✓ text"));
}

#[test]
fn long_input_goes_in_chunks() {
    let s = server("chunks");
    let pane = s.tmux.new_pane(&s.dir, &["cat"], &[]).unwrap();
    let long = "x".repeat(700) + "END";
    s.tmux
        .send_bytes(&pane, format!("{long}\r").as_bytes())
        .unwrap();
    assert!(output_until(&s, &pane, "END").contains(&long));
}

#[test]
fn a_paused_pane_runs_and_capture_reads_it() {
    let s = server("off");
    let pane = s.tmux.new_pane(&s.dir, &["cat"], &[]).unwrap();
    drain(&s);
    s.tmux.set_streaming(&pane, false).unwrap();
    s.tmux.send_bytes(&pane, b"quiet line\r").unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let streamed: Vec<_> = s
        .events
        .try_iter()
        .filter(|e| matches!(e, Event::Output { pane: p, .. } if *p == pane))
        .collect();
    assert!(streamed.is_empty(), "got output while paused: {streamed:?}");
    let text = s.tmux.capture(&pane, "-").unwrap().join("\n");
    assert!(text.contains("quiet line"), "capture: {text}");

    s.tmux.set_streaming(&pane, true).unwrap();
    s.tmux.send_bytes(&pane, b"loud line\r").unwrap();
    let got = output_until(&s, &pane, "loud line");
    assert!(got.contains("loud line"), "after continue: {got:?}");
    assert!(
        !got.contains("quiet line"),
        "continue streams only new output: {got:?}"
    );
}

#[test]
fn bad_command_returns_the_tmux_error() {
    let s = server("error");
    let err = s.tmux.capture("%999", "-").unwrap_err();
    assert!(err.to_string().contains("can't find pane"), "{err}");
    // The client still works after an error.
    assert!(!s.tmux.panes().unwrap().is_empty());
}

#[test]
fn panes_lists_the_first_and_new_panes() {
    let s = server("list");
    let first = s.tmux.panes().unwrap();
    assert_eq!(first.len(), 1, "new-session makes one pane: {first:?}");
    let pane = s.tmux.new_pane(&s.dir, &["cat"], &[]).unwrap();
    let all = s.tmux.panes().unwrap();
    assert_eq!(all.len(), 2);
    assert!(all.contains(&pane));
}

#[test]
fn size_and_cursor() {
    let s = server("size");
    let pane = s.tmux.panes().unwrap().remove(0);
    s.tmux.set_size(100, 30).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        s.tmux
            .display(&pane, "#{pane_width}x#{pane_height}")
            .unwrap(),
        "100x30"
    );
    let (x, y) = s.tmux.cursor(&pane).unwrap();
    assert!(x < 100 && y < 30);
}

#[test]
fn a_second_start_attaches_to_the_running_server() {
    let s = server("attach");
    let pane = s.tmux.new_pane(&s.dir, &["cat"], &[]).unwrap();
    let conf = s.dir.join("tmux.conf");
    let again = Tmux::start(s.tmux.socket(), &conf, |_| {}).unwrap();
    assert!(again.panes().unwrap().contains(&pane));
}

#[test]
fn kill_server_ends_the_client() {
    let s = server("exit");
    s.tmux.kill_server();
    let end = Instant::now() + Duration::from_secs(5);
    let mut exited = false;
    while Instant::now() < end && !exited {
        exited = matches!(
            s.events.recv_timeout(Duration::from_millis(100)),
            Ok(Event::Exit(_))
        );
    }
    assert!(exited);
    assert!(s.tmux.panes().is_err());
}
