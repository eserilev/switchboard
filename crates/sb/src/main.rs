//! `sb`: the small CLI that hooks and you call (SPEC 9.3, 13.2, 13.3).
//!
//! `sb state` and `sb permit` run inside Claude hooks. They must never block
//! Claude by mistake: with no `SB_PANE` or no socket, they exit 0 at once.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;
use switchboard_core::paths::Paths;
use switchboard_core::proto::{Ack, PermitReply, Request};
use switchboard_core::config::Config;
use switchboard_core::{connections, hooks, now, repo_context};

mod mcp;

/// `sb` logs to `~/.switchboard/logs/sb.log`, and `sb serve` to `serve.log`. `SB_LOG`
/// sets the filter; the default is `info`.
fn init_logs(name: &str) {
    let dir = Paths::from_env().home.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(name))
    else {
        return;
    };
    let filter =
        tracing_subscriber::EnvFilter::try_from_env("SB_LOG").unwrap_or_else(|_| "info".into());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file))
        .try_init();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The MCP server talks on stdout, so it never logs there; the file is fine.
    init_logs(if args.first().map(String::as_str) == Some("serve") { "serve.log" } else { "sb.log" });
    let t0 = std::time::Instant::now();
    let code = match args.first().map(String::as_str) {
        Some("state") => state(args.get(1).map(String::as_str).unwrap_or("")),
        Some("permit") => permit(),
        Some("open") => open(&args[1..]),
        Some("task") => task(&args[1..]),
        Some("today") => today(args.get(1).map(String::as_str)),
        Some("context") => context(args.get(1).map(String::as_str)),
        Some("msg") => msg(&args[1..]),
        Some("copy") => copy(),
        Some("mcp") => match args.get(1).map(String::as_str) {
            Some("--pane") => mcp::serve_pane(),
            a => mcp::serve(a == Some("--ask")),
        },
        Some("resume") => resume(&args[1..]),
        Some("serve") => serve(),
        Some("attach") => attach(),
        Some("pair") => pair(),
        Some("devices") => devices(&args[1..]),
        Some("connect") if args.get(1).map(String::as_str) == Some("add") => {
            connect_add(&args[2..])
        }
        Some("--version") => {
            println!("sb {}", env!("CARGO_PKG_VERSION"));
            0
        }
        _ => {
            eprintln!("usage: sb open [path] [-w branch] [--nvim|--shell]\n       sb context [path]   (the repo notes an agent here gets)\n       sb today [job]      (what needs you on GitHub, ranked)\n       {TASK_USAGE}\n       sb connect add <name> [--endpoint <url> --model <m> --token <keyring:name>]\n       sb copy       (stdin goes to the clipboard of your window's computer)\n       sb serve      (the hub for remote windows)\n       sb attach     (a remote window's link; starts sb serve when none runs)\n       sb pair       (adds a phone)\n       sb devices [remove <id>]");
            2
        }
    };
    tracing::info!(target: "sb::cli", args = ?args, code, ms = t0.elapsed().as_millis() as u64, pane = ?std::env::var("SB_PANE").ok(), "sb");
    std::process::exit(code);
}

/// `sb context [path]`: the repo notes that an agent in `path` gets (SPEC 37), and
/// where each part came from.
fn context(dir: Option<&str>) -> i32 {
    let dir = dir.map(PathBuf::from).or_else(|| std::env::current_dir().ok()).unwrap_or_default();
    let paths = Paths::from_env();
    let config = match Config::load(&paths.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("sb context: {e}");
            return 1;
        }
    };
    let cdir = paths.config.parent().unwrap_or(Path::new("."));
    let ctx = repo_context::for_dir(&config, cdir, &dir);
    if ctx.repos.is_empty() {
        eprintln!("sb context: {} has no GitHub remote", dir.display());
        return 1;
    }
    for r in &ctx.repos {
        eprintln!("{r}: your own notes go in {}", cdir.join("context").join(format!("{r}.md")).display());
    }
    for s in &ctx.sources {
        match &s.read {
            Ok(n) => eprintln!("  {}: {n} bytes", s.path.display()),
            Err(e) => eprintln!("  {}: {e}", s.path.display()),
        }
    }
    if ctx.text.is_empty() {
        eprintln!("No notes for this repo.");
    } else {
        println!("{}", ctx.text);
    }
    0
}

// ---------- messages between agents (SPEC 40) ----------

const MSG_USAGE: &str = "sb msg agents            (the agents that can get messages)\n       sb msg send <address>    (the text on stdin; from this pane)";

/// The messages that wait for `pane`, from its hub, as one text.
fn mail_take(pane: &str) -> Option<String> {
    use switchboard_core::proto::MailOp;
    let line = send(&Request::Mail(MailOp::Take { pane: pane.into() }), true).ok()??;
    let v: Value = serde_json::from_str(&line).ok()?;
    v["text"].as_str().filter(|t| !t.is_empty()).map(str::to_owned)
}

/// Runs `sb msg <args>` on the mailbox host, with `input` on stdin, and gives
/// its reply line.
fn mail_remote(config: &Config, args: &[&str], input: &str) -> Result<String, String> {
    let argv = config.mail_argv(args).ok_or("no mailbox host")?;
    let mut child = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", argv[0]))?;
    child.stdin.take().ok_or("no stdin")?.write_all(input.as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let line = String::from_utf8_lossy(&out.stdout).lines().last().unwrap_or("").to_owned();
    if line.is_empty() {
        return Err(format!("the mailbox host did not answer: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(line)
}

/// The repo and branch of the folder, as the label of a sender on this computer.
fn here_label(pane: &str) -> String {
    let dir = std::env::current_dir().unwrap_or_default();
    let repo = switchboard_core::repos::main_checkout(&dir).unwrap_or(dir.clone());
    let name = repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let branch = switchboard_core::repos::git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    [name, branch, pane.to_owned()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
}

/// A mail request of a pane or of `sb msg`. On a computer whose mailbox is on the
/// server (`remote` in the config), sends and the agent list go there over ssh;
/// everything else goes to this computer's hub.
pub fn mail_route(req: &Request) -> Result<String, String> {
    use switchboard_core::proto::MailOp;
    let config = Config::load(&Paths::from_env().config).unwrap_or_default();
    if config.mail_host().is_none() {
        return Ok(send(req, true)?.unwrap_or_default());
    }
    let machine = switchboard_core::mail::machine_name(&config);
    match req {
        Request::Mail(MailOp::Send { pane: Some(p), to, text, .. }) => {
            let body = json!({ "from": format!("{p}@{machine}"), "label": here_label(p), "to": to, "text": text });
            mail_remote(&config, &["send-json"], &body.to_string())
        }
        Request::Mail(MailOp::Agents { pane }) => {
            let line = mail_remote(&config, &["agents"], "")?;
            // The mailbox does not know which pane asks: mark it here.
            let mut v: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
            let me = pane.as_ref().map(|p| format!("{p}@{machine}"));
            if let Some(list) = v["agents"].as_array_mut() {
                for a in list {
                    a["you"] = json!(me.is_some() && a["address"].as_str() == me.as_deref());
                }
            }
            Ok(v.to_string())
        }
        _ => Ok(send(req, true)?.unwrap_or_default()),
    }
}

fn copy() -> i32 {
    let pane = std::env::var("SB_PANE").ok().filter(|p| !p.is_empty());
    let mut text = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut text) {
        eprintln!("sb copy: {e}");
        return 1;
    }
    match send(&Request::Clip { pane, text }, true).map(|l| serde_json::from_str::<Ack>(&l.unwrap_or_default())) {
        Ok(Ok(Ack { ok: true, .. })) => 0,
        Ok(Ok(Ack { error, .. })) => {
            eprintln!("sb copy: {}", error.unwrap_or_else(|| "refused".into()));
            1
        }
        Ok(Err(e)) => {
            eprintln!("sb copy: {e}");
            1
        }
        Err(e) => {
            eprintln!("sb copy: {e}");
            1
        }
    }
}

fn msg(args: &[String]) -> i32 {
    use switchboard_core::proto::MailOp;
    let pane = std::env::var("SB_PANE").ok().filter(|p| !p.is_empty());
    let reply = match args.first().map(String::as_str) {
        Some("agents") => mail_route(&Request::Mail(MailOp::Agents { pane })),
        Some("send") if args.len() == 2 => {
            let mut text = String::new();
            let _ = std::io::stdin().read_to_string(&mut text);
            let Some(pane) = pane else {
                eprintln!("sb msg send: run it in an agent pane (no SB_PANE)");
                return 1;
            };
            mail_route(&Request::Mail(MailOp::Send { pane: Some(pane), from: None, label: None, to: args[1].clone(), text }))
        }
        // Over ssh from another machine's agent: the sender comes in the JSON.
        Some("send-json") => {
            let v = stdin_json();
            let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
            send(&Request::Mail(MailOp::Send { pane: None, from: Some(s("from")), label: Some(s("label")), to: s("to"), text: s("text") }), true)
                .map(Option::unwrap_or_default)
        }
        Some("watch") if args.len() == 2 => return msg_watch(&args[1]),
        _ => {
            eprintln!("usage: {MSG_USAGE}");
            return 2;
        }
    };
    match reply {
        Ok(line) => {
            println!("{}", line.trim());
            let ok: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
            if ok["ok"] == json!(true) { 0 } else { 1 }
        }
        Err(e) => {
            println!("{}", json!({ "ok": false, "error": e }));
            1
        }
    }
}

/// `sb msg watch <machine>`: the link of another machine's hub, over ssh. stdin
/// and stdout to a watch on this hub (SPEC 40.5).
fn msg_watch(machine: &str) -> i32 {
    use switchboard_core::proto::MailOp;
    let sock = Paths::from_env().sock();
    if safe_sock_dir(&sock).is_err() {
        return 1;
    }
    let Ok(mut s) = UnixStream::connect(&sock) else {
        eprintln!("sb msg watch: no Switchboard hub runs here");
        return 1;
    };
    let req = Request::Mail(MailOp::Watch { machine: machine.into() });
    if writeln!(s, "{}", serde_json::to_string(&req).unwrap_or_default()).is_err() {
        return 1;
    }
    let Ok(mut up) = s.try_clone() else { return 1 };
    std::thread::spawn(move || {
        relay(&mut std::io::stdin().lock(), &mut up);
        let _ = up.shutdown(std::net::Shutdown::Write);
    });
    relay(&mut s, &mut std::io::stdout().lock());
    0
}

// ---------- remote mode (SPEC 29) ----------

static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_stop(_: libc::c_int) {
    STOP.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// The hub with no window, for remote windows. It runs until SIGTERM or SIGINT.
fn serve() -> i32 {
    use std::sync::Arc;
    use switchboard_core::hub::Hub;
    use switchboard_core::remote::{self, RemoteSink};
    let paths = Paths::from_env();
    let sink = Arc::new(RemoteSink::default());
    let sb = std::env::current_exe().unwrap_or_else(|_| "sb".into());
    let hub = match Hub::start(paths.clone(), sb, sink.clone()) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("sb serve: {e}");
            return 1;
        }
    };
    // The hub lock is ours, so an old socket file belongs to a hub that is gone.
    let wsock = paths.window_sock();
    let _ = std::fs::remove_file(&wsock);
    let listener = match std::os::unix::net::UnixListener::bind(&wsock) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("sb serve: {}: {e}", wsock.display());
            return 1;
        }
    };
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&wsock, std::fs::Permissions::from_mode(0o600));
    }
    // SAFETY: the handler only stores to an atomic.
    unsafe {
        libc::signal(libc::SIGTERM, on_stop as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_stop as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    let pid = paths.runtime.join("serve.pid");
    let _ = std::fs::write(&pid, std::process::id().to_string());
    tracing::info!(target: "sb::serve", sock = %wsock.display(), "sb serve listens");
    eprintln!("sb serve: listening on {}", wsock.display());
    {
        let (hub, sink) = (Arc::downgrade(&hub), sink.clone());
        std::thread::spawn(move || remote::serve(listener, hub, sink));
    }
    if let Some(listen) = hub.config().devices.listen.clone() {
        if let Err(err) = switchboard_core::devices::serve(&listen, Arc::downgrade(&hub), sink.clone()) {
            tracing::error!(target: "sb::devices", %err, "no device listener");
            eprintln!("sb serve: {err}");
        }
    }
    while !STOP.load(std::sync::atomic::Ordering::SeqCst) && !hub.restart_asked() {
        std::thread::sleep(Duration::from_millis(200));
    }
    let restart = hub.restart_asked();
    tracing::info!(target: "sb::serve", restart, "sb serve stops");
    if restart {
        // The answer to the window goes out first.
        std::thread::sleep(Duration::from_millis(300));
    }
    // Review runs have process groups of their own: stop them, as the app does on quit.
    hub.stop_runs();
    let _ = std::fs::remove_file(&wsock);
    let _ = std::fs::remove_file(&pid);
    if restart {
        // The same pid and session, with the binary that is on disk now (SPEC 29.12).
        // Every file of ours closes on exec, the hub lock too. The agents run on in tmux.
        let err = std::os::unix::process::CommandExt::exec(std::process::Command::new(serve_exe()).arg("serve"));
        tracing::error!(target: "sb::serve", %err, "the restart failed; the next sb attach starts sb serve");
        return 1;
    }
    0
}

/// The `sb` binary on disk. After an install, `/proc/self/exe` names the old,
/// deleted file: drop that mark, so the new one at the same path runs.
fn serve_exe() -> std::path::PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| "sb".into());
    match exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
        Some(s) => s.into(),
        None => exe,
    }
}

/// The link of a remote window: stdin and stdout to the window socket. With no
/// `sb serve`, it starts one that keeps running after the link ends.
fn attach() -> i32 {
    let wsock = Paths::from_env().window_sock();
    if let Err(e) = safe_sock_dir(&wsock) {
        // A missing folder is fine: `sb serve` makes it below.
        if wsock.parent().is_some_and(|d| std::fs::symlink_metadata(d).is_ok()) {
            eprintln!("sb attach: {e}");
            return 1;
        }
    }
    let stream = match UnixStream::connect(&wsock) {
        Ok(s) => s,
        Err(_) => {
            if let Err(e) = start_serve() {
                eprintln!("sb attach: {e}");
                return 1;
            }
            let mut got = None;
            for _ in 0..150 {
                if let Ok(s) = UnixStream::connect(&wsock) {
                    got = Some(s);
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            match got {
                Some(s) => s,
                None => {
                    eprintln!("sb attach: sb serve did not start. See ~/.switchboard/logs/serve.log.");
                    return 1;
                }
            }
        }
    };
    let Ok(mut up) = stream.try_clone() else { return 1 };
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        relay(&mut stdin.lock(), &mut up);
        let _ = up.shutdown(std::net::Shutdown::Write);
    });
    let mut down = stream;
    relay(&mut down, &mut std::io::stdout().lock());
    0
}

/// Copies bytes as they come. Not `io::copy`: on Linux it can splice, and a splice
/// from a pipe can hold bytes back.
fn relay(from: &mut impl Read, to: &mut impl Write) {
    let mut buf = vec![0u8; 64 << 10];
    loop {
        match from.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                if to.write_all(&buf[..n]).and_then(|_| to.flush()).is_err() {
                    return;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

/// `sb serve` in a session of its own, so the end of the ssh link does not stop it.
fn start_serve() -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let log = Paths::from_env().home.join("logs");
    let _ = std::fs::create_dir_all(&log);
    let err = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log.join("serve.out"))
        .map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("serve")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(err);
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().map(drop).map_err(|e| format!("start sb serve: {e}"))
}

fn stdin_json() -> Value {
    let mut s = String::new();
    let _ = std::io::stdin().read_to_string(&mut s);
    serde_json::from_str(&s).unwrap_or(Value::Null)
}

/// An error when the folder of a socket is a symlink or belongs to another user:
/// then the socket may not be Switchboard's, and `sb` does not talk to it.
fn safe_sock_dir(sock: &Path) -> Result<(), String> {
    let Some(dir) = sock.parent() else { return Ok(()) };
    let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
    switchboard_core::paths::check_own_dir(dir).map_err(|e| {
        tracing::warn!(target: "sb::cli", sock = %sock.display(), %e, "unsafe socket folder");
        format!("{}: {e}", sock.display())
    })
}

/// Sends one request. With `reply`, waits for one line back.
pub fn send(req: &Request, reply: bool) -> Result<Option<String>, String> {
    let sock = Paths::from_env().sock();
    safe_sock_dir(&sock)?;
    let mut s = UnixStream::connect(&sock).map_err(|e| {
        tracing::warn!(target: "sb::cli", sock = %sock.display(), %e, "no Switchboard socket");
        format!("{}: {e}", sock.display())
    })?;
    writeln!(
        s,
        "{}",
        serde_json::to_string(req).map_err(|e| e.to_string())?
    )
    .map_err(|e| e.to_string())?;
    if !reply {
        return Ok(None);
    }
    let mut line = String::new();
    BufReader::new(s)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    Ok(Some(line))
}

fn state(arg: &str) -> i32 {
    let Ok(pane) = std::env::var("SB_PANE") else {
        return 0;
    };
    let mut input = stdin_json();
    if arg == "turn" {
        with_transcript_text(&mut input);
    }
    // The end of a turn: messages from other agents that wait for this one go
    // in as the reason to go on (SPEC 40.4). Then the agent works, not waits.
    let mail = if arg == "turn" { mail_take(&pane) } else { None };
    let event = if mail.is_some() { "working" } else { arg };
    let Some(msg) = hooks::state_msg(&pane, event, &input, now()) else {
        return 0;
    };
    // The state file first: it is the record when the app is closed.
    let dir = Paths::from_env().state_dir();
    if std::fs::create_dir_all(&dir).is_ok() {
        let tmp = dir.join(format!(".{pane}.{}.tmp", std::process::id()));
        if std::fs::write(&tmp, serde_json::to_string(&msg).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, dir.join(format!("{pane}.json")));
        }
    }
    let _ = send(&Request::State(msg), false);
    if let Some(text) = mail {
        println!("{}", json!({ "decision": "block", "reason": text }));
        return 0;
    }
    if arg == "session" {
        // A pane with the pane tools learns at the start that it runs in Switchboard
        // (SPEC 36), and every pane gets your notes for its repo (SPEC 37).
        let mut parts: Vec<String> = vec![];
        if std::env::var("SB_TOOLS").as_deref() == Ok("1") {
            parts.push(hooks::pane_tools_note(&pane));
        }
        let dir = input.get("cwd").and_then(Value::as_str).map(PathBuf::from).or_else(|| std::env::current_dir().ok());
        if let Some(dir) = dir {
            let paths = Paths::from_env();
            let config = Config::load(&paths.config).unwrap_or_default();
            let ctx = repo_context::for_dir(&config, paths.config.parent().unwrap_or(Path::new(".")), &dir);
            if !ctx.text.is_empty() {
                parts.push(ctx.text);
            }
        }
        if !parts.is_empty() {
            println!("{}", hooks::session_start_output(&parts.join("\n\n")));
        }
    }
    0
}

/// A Stop hook input with no `last_assistant_message` gets it from the session file
/// (`transcript_path`): the last 2 MB of it.
fn with_transcript_text(input: &mut Value) {
    if input.get("last_assistant_message").and_then(Value::as_str).is_some() {
        return;
    }
    let Some(path) = input.get("transcript_path").and_then(Value::as_str) else { return };
    let Ok(mut f) = std::fs::File::open(path) else { return };
    use std::io::{Seek, SeekFrom};
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(2 << 20)));
    let mut buf = vec![];
    if f.read_to_end(&mut buf).is_err() {
        return;
    }
    if let Some(text) = hooks::last_assistant_text(&String::from_utf8_lossy(&buf)) {
        input["last_assistant_message"] = json!(text);
    }
}

fn permit() -> i32 {
    let Ok(pane) = std::env::var("SB_PANE") else {
        return 0;
    };
    let input = stdin_json();
    let tool = input
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("tool")
        .to_owned();
    let tool_input = input.get("tool_input").cloned().unwrap_or(Value::Null);
    let Ok(Some(line)) = send(
        &Request::Permit {
            pane,
            tool,
            input: tool_input,
        },
        true,
    ) else {
        return 0;
    };
    let Ok(r) = serde_json::from_str::<PermitReply>(&line) else {
        return 0;
    };
    let decision = match r.behavior.as_deref() {
        Some("allow") => json!({ "behavior": "allow" }),
        Some("deny") => json!({ "behavior": "deny", "message": "Denied from Switchboard." }),
        _ => return 0,
    };
    println!(
        "{}",
        json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } })
    );
    0
}

fn open(args: &[String]) -> i32 {
    let mut path = None;
    let mut worktree = None;
    let mut run = "claude";
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-w" => worktree = it.next().cloned(),
            "--nvim" => run = "nvim",
            "--shell" => run = "shell",
            p => path = Some(p.to_owned()),
        }
    }
    let path = path.unwrap_or_else(|| ".".into());
    let path = match std::fs::canonicalize(&path) {
        Ok(p) if p.is_dir() => p,
        _ => {
            eprintln!("sb open: no such folder: {path}");
            return 1;
        }
    };
    let req = Request::Open {
        path: path.to_string_lossy().into_owned(),
        worktree,
        run: Some(run.into()),
    };
    let mut started = false;
    for _ in 0..100 {
        match send(&req, true) {
            Ok(Some(line)) => {
                let ack: Ack = serde_json::from_str(&line).unwrap_or(Ack::err("bad reply"));
                if let Some(e) = ack.error {
                    eprintln!("sb open: {e}");
                    return 1;
                }
                return 0;
            }
            _ if !started => {
                started = true;
                if let Err(e) = start_app() {
                    eprintln!("sb open: Switchboard is not running, and it did not start: {e}");
                    return 1;
                }
            }
            _ => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    eprintln!("sb open: Switchboard did not answer in 10 s");
    1
}

const TASK_USAGE: &str = "sb task list [--sprint current|backlog|<name>] [--status <s>] [--json]
       sb task show [<id>] [--json]
       sb task add <title> [--notes <text>] [--repo <dir>] [--sprint current|backlog|<name>]
       sb task note <id> <text>
       sb task status <id> todo|doing|review|blocked [--by <who or what blocks it>]";

/// `sb task` (SPEC 27.7). It never sets done, deletes or closes a sprint: only the
/// window does that. `SB_PANE` labels the history row.
fn task(args: &[String]) -> i32 {
    use switchboard_core::tasks::TaskOp;
    let usage = || {
        eprintln!("usage: {TASK_USAGE}");
        2
    };
    let mut free: Vec<String> = vec![];
    let mut flags: std::collections::HashMap<&str, String> = Default::default();
    let mut json_out = false;
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => json_out = true,
            f @ ("--sprint" | "--status" | "--notes" | "--repo" | "--by") => match it.next() {
                Some(v) => {
                    flags.insert(f, v.clone());
                }
                None => return usage(),
            },
            f if f.starts_with("--") => {
                eprintln!("sb task: unknown flag {f}");
                return usage();
            }
            v => free.push(v.to_owned()),
        }
    }
    let flag = |k: &str| flags.get(k).cloned();
    let op = match (args.first().map(String::as_str), free.as_slice()) {
        (Some("list"), []) => TaskOp::List { sprint: flag("--sprint"), status: flag("--status") },
        (Some("show"), [] | [_]) => TaskOp::Show { id: free.first().cloned() },
        (Some("add"), [title]) => TaskOp::Add {
            title: title.clone(),
            notes: flag("--notes"),
            repo: flag("--repo").map(|r| std::fs::canonicalize(&r).map(|p| p.to_string_lossy().into_owned()).unwrap_or(r)),
            sprint: flag("--sprint"),
        },
        (Some("note"), [id, text]) => TaskOp::Note { id: id.clone(), text: text.clone() },
        (Some("status"), [id, status]) => TaskOp::Status { id: id.clone(), status: status.clone(), blocked_by: flag("--by") },
        _ => return usage(),
    };
    let show = matches!(op, TaskOp::Show { .. });
    let list = matches!(op, TaskOp::List { .. });
    let req = Request::Task { pane: std::env::var("SB_PANE").ok().filter(|p| !p.is_empty()), op };
    let line = match send(&req, true) {
        Ok(Some(line)) => line,
        _ => {
            eprintln!("sb task: Switchboard is not running");
            return 1;
        }
    };
    let v: Value = serde_json::from_str(&line).unwrap_or(json!({ "ok": false, "error": "bad reply" }));
    if v["ok"] != json!(true) {
        eprintln!("sb task: {}", v["error"].as_str().unwrap_or("failed"));
        return 1;
    }
    if json_out {
        let mut v = v;
        if let Some(o) = v.as_object_mut() {
            o.remove("ok");
        }
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return 0;
    }
    let sprint_name = |id: &Value| -> String {
        v["sprints"].as_array().into_iter().flatten().find(|s| &s["id"] == id).and_then(|s| s["name"].as_str()).unwrap_or("backlog").to_owned()
    };
    let one = |t: &Value| format!("{:<6} {:<8} {}", t["id"].as_str().unwrap_or(""), t["status"].as_str().unwrap_or(""), t["title"].as_str().unwrap_or(""));
    if list {
        for t in v["tasks"].as_array().into_iter().flatten() {
            println!("{}  ({})", one(t), sprint_name(&t["sprint"]));
        }
    } else if show {
        let t = &v["task"];
        println!("{}", one(t));
        println!("sprint: {}", v["sprint"].as_str().unwrap_or("backlog"));
        if let Some(r) = t["repo"].as_str() {
            println!("repo:   {r}");
        }
        if let Some(b) = t["blocked_by"].as_str() {
            println!("blocked by: {b}");
        }
        if let Some(p) = t["points"].as_u64() {
            println!("points: {p}");
        }
        let words = |k: &str| t[k].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>();
        if !words("tags").is_empty() {
            println!("tags:   {}", words("tags").join(", "));
        }
        for l in words("links") {
            println!("link:   {l}");
        }
        let notes = t["notes"].as_str().unwrap_or("").trim();
        if !notes.is_empty() {
            println!("\n{notes}");
        }
    } else {
        println!("{}", one(&v["task"]));
    }
    0
}

fn start_app() -> std::io::Result<()> {
    let me = std::env::current_exe()?;
    let app = me.with_file_name("switchboard");
    std::process::Command::new(if app.exists() {
        app
    } else {
        PathBuf::from("switchboard")
    })
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()
    .map(drop)
}

/// Your terminal, or a message why not. Never stdin: a pipe can not answer.
fn terminal() -> Result<std::fs::File, String> {
    // Not a wall, since an agent with Bash and no sandbox runs as you (SPEC 41.12):
    // it stops one that tries in good faith.
    if std::env::var_os("CLAUDECODE").is_some() {
        return Err("run this yourself, in your own terminal".into());
    }
    std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty").map_err(|e| format!("needs a terminal: {e}"))
}

fn ask_line(tty: &std::fs::File, text: &str) -> Result<String, String> {
    use std::io::{BufRead, Write};
    let mut w = tty;
    write!(w, "{text}").and_then(|()| w.flush()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    std::io::BufReader::new(tty).read_line(&mut line).map_err(|e| e.to_string())?;
    Ok(line.trim().to_owned())
}

/// Adds a phone (SPEC 41.4): shows a QR code, then asks you.
fn pair() -> i32 {
    match pair_run() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("sb pair: {err}");
            1
        }
    }
}

fn pair_run() -> Result<(), String> {
    use std::io::Write;
    use switchboard_core::{config::Config, devices, pairing, store::Store};
    let tty = terminal()?;
    let paths = Paths::from_env();
    let listen = Config::load(&paths.config)?.devices.listen.ok_or(format!(
        "no address for phones. Put this in {}, with an address of this computer that the phone reaches, then restart sb serve:\n\n[devices]\nlisten = \"100.x.y.z:7800\"",
        paths.config.display()
    ))?;
    let hub = devices::address(&listen)?;
    let key = devices::hub_key(&paths)?;
    let store = Store::open(&paths.store()).map_err(|e| e.to_string())?;
    let p = pairing::Pairing::open(hub.ip())?;
    let qr = p.qr(hub, &key.public)?;
    let code = qrcode::QrCode::new(&qr).map_err(|e| e.to_string())?;
    let art = code.render::<qrcode::render::unicode::Dense1x2>().build();
    // Black on white on any terminal: a scanner can miss an inverted code.
    let art: String = art.lines().map(|l| format!("\x1b[38;5;16;48;5;231m{l}\x1b[0m\n")).collect();
    writeln!(&tty, "{art}\nScan this with the Switchboard app on your phone. It works once, for 10 minutes.\nNo camera? Paste this text in the app instead:\n\n{qr}\n").map_err(|e| e.to_string())?;
    let asking = p.wait(key, pairing::OPEN_FOR).ok_or("no phone came in 10 minutes")?;
    writeln!(&tty, "\n\"{}\" asks to pair. Its screen must show this code:\n\n    {}\n", asking.name, asking.code).map_err(|e| e.to_string())?;
    let yes = ask_line(&tty, "Does the phone show the same code? Type yes to allow it: ")?;
    if yes != "yes" {
        asking.refuse();
        return Err("refused. Nothing was added.".into());
    }
    let name = ask_line(&tty, &format!("A name for it [{}]: ", asking.name))?;
    let name = if name.is_empty() { asking.name.clone() } else { name };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let id = asking.allow(&store, &name, now)?;
    println!("{name} is paired, as {id}. Remove it with: sb devices remove {id}");
    Ok(())
}

/// Lists the devices, or removes one. The hub closes its sessions within seconds.
fn devices(args: &[String]) -> i32 {
    use switchboard_core::store::Store;
    let run = || -> Result<(), String> {
        let store = Store::open(&Paths::from_env().store()).map_err(|e| e.to_string())?;
        match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
            [] => {
                for d in store.devices().map_err(|e| e.to_string())? {
                    let state = if d.revoked_at.is_some() { "removed" } else { "paired" };
                    println!("{}  {state}  {}  {}", d.id, d.platform, d.name);
                }
                Ok(())
            }
            // Removing only takes rights away, so it asks nothing.
            ["remove", id] => {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                store.device_revoke(id, now).map_err(|e| e.to_string())?.ok_or(format!("no device {id}"))?;
                println!("{id} is removed.");
                Ok(())
            }
            _ => Err("usage: sb devices [remove <id>]".into()),
        }
    };
    match run() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("sb devices: {err}");
            1
        }
    }
}

fn connect_add(args: &[String]) -> i32 {
    let Some(name) = args.first() else {
        eprintln!(
            "usage: sb connect add <name> [--endpoint <url> --model <m> --token <keyring:name>]"
        );
        return 2;
    };
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        eprintln!("sb connect add: a name has only letters, digits, - and _");
        return 2;
    }
    let (mut url, mut model, mut token) = (None, None, None);
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--endpoint" => url = it.next().cloned(),
            "--model" => model = it.next().cloned(),
            "--token" => token = it.next().cloned(),
            other => {
                eprintln!("sb connect add: unknown argument {other}");
                return 2;
            }
        }
    }
    if token.as_deref().is_some_and(|t| !t.starts_with("keyring:")) {
        eprintln!("sb connect add: --token takes keyring:<name>. Store the token with:\n  secret-tool store --label=switchboard service switchboard name <name>");
        return 2;
    }
    let paths = Paths::from_env();
    let home = switchboard_core::paths::home_dir();
    let dir = home.join(format!(".claude-{name}"));
    if let Err(e) = connections::make_folder(&dir, &paths.claude) {
        eprintln!("sb connect add: {}: {e}", dir.display());
        return 1;
    }
    let conn = switchboard_core::config::ConnectionCfg {
        kind: if url.is_some() { "endpoint" } else { "claude" }.into(),
        dir: Some(format!("~/.claude-{name}")),
        url: url.clone(),
        model,
        token,
    };
    match switchboard_core::config::Config::add_connection(&paths.config, name, &conn) {
        Ok(true) => println!("Added {name} to {}", paths.config.display()),
        Ok(false) => println!("{name} is already in {}", paths.config.display()),
        Err(e) => {
            eprintln!("sb connect add: {e}");
            return 1;
        }
    }
    if url.is_none() {
        println!(
            "Log in once:\n  CLAUDE_CONFIG_DIR={} claude\nthen run /login.",
            dir.display()
        );
    }
    0
}


/// `sb today [job]` (SPEC 39.8): the Today list in the terminal.
/// `sb resume <pane>...`: starts Claude panes again with their sessions, so they
/// get a new build's tools and settings. Their conversations stay.
fn resume(panes: &[String]) -> i32 {
    if panes.is_empty() {
        eprintln!("usage: sb resume <pane>...");
        return 2;
    }
    let mut code = 0;
    for p in panes {
        let req = Request::Resume { pane: p.clone() };
        let v: Value = match send(&req, true) {
            Ok(Some(line)) => serde_json::from_str(&line).unwrap_or(json!({ "ok": false, "error": "bad reply" })),
            _ => {
                eprintln!("sb resume: Switchboard is not running");
                return 1;
            }
        };
        if v["ok"] == json!(true) {
            println!("{p}: started again");
        } else {
            eprintln!("sb resume: {p}: {}", v["error"].as_str().unwrap_or("failed"));
            code = 1;
        }
    }
    code
}

fn today(job: Option<&str>) -> i32 {
    let req = Request::Today { pane: None, job: job.map(str::to_owned), repo: None, number: None };
    let line = match send(&req, true) {
        Ok(Some(line)) => line,
        _ => {
            eprintln!("sb today: Switchboard is not running");
            return 1;
        }
    };
    let v: Value = serde_json::from_str(&line).unwrap_or(json!({ "ok": false, "error": "bad reply" }));
    if v["ok"] != json!(true) {
        eprintln!("sb today: {}", v["error"].as_str().unwrap_or("failed"));
        return 1;
    }
    print!("{}", v["text"].as_str().unwrap_or(""));
    0
}
