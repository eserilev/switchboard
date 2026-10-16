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
use switchboard_core::{connections, hooks, now};

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
        Some("mcp") => mcp::serve(),
        Some("serve") => serve(),
        Some("attach") => attach(),
        Some("connect") if args.get(1).map(String::as_str) == Some("add") => {
            connect_add(&args[2..])
        }
        Some("--version") => {
            println!("sb {}", env!("CARGO_PKG_VERSION"));
            0
        }
        _ => {
            eprintln!("usage: sb open [path] [-w branch] [--nvim|--shell]\n       {TASK_USAGE}\n       sb connect add <name> [--endpoint <url> --model <m> --token <keyring:name>]\n       sb serve      (the hub for remote windows)\n       sb attach     (a remote window's link; starts sb serve when none runs)");
            2
        }
    };
    tracing::info!(target: "sb::cli", args = ?args, code, ms = t0.elapsed().as_millis() as u64, pane = ?std::env::var("SB_PANE").ok(), "sb");
    std::process::exit(code);
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
    while !STOP.load(std::sync::atomic::Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(200));
    }
    tracing::info!(target: "sb::serve", "sb serve stops");
    // Review runs have process groups of their own: stop them, as the app does on quit.
    hub.stop_runs();
    let _ = std::fs::remove_file(&wsock);
    let _ = std::fs::remove_file(&pid);
    0
}

/// The link of a remote window: stdin and stdout to the window socket. With no
/// `sb serve`, it starts one that keeps running after the link ends.
fn attach() -> i32 {
    let wsock = Paths::from_env().window_sock();
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

/// Sends one request. With `reply`, waits for one line back.
pub fn send(req: &Request, reply: bool) -> Result<Option<String>, String> {
    let sock = Paths::from_env().sock();
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
    let input = stdin_json();
    let Some(msg) = hooks::state_msg(&pane, arg, &input, now()) else {
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
    0
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
    let path = std::fs::canonicalize(path.unwrap_or_else(|| ".".into())).unwrap_or_default();
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
    let mut section = format!(
        "\n[connections.{name}]\nkind = \"{}\"\ndir = \"~/.claude-{name}\"\n",
        if url.is_some() { "endpoint" } else { "claude" }
    );
    if let Some(u) = &url {
        section += &format!("url = \"{u}\"\n");
    }
    if let Some(m) = &model {
        section += &format!("model = \"{m}\"\n");
    }
    if let Some(t) = &token {
        section += &format!("token = \"{t}\"\n");
    }
    let config = std::fs::read_to_string(&paths.config).unwrap_or_default();
    if config.contains(&format!("[connections.{name}]")) {
        println!("{name} is already in {}", paths.config.display());
    } else {
        if let Some(p) = paths.config.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        if let Err(e) = append(&paths.config, &section) {
            eprintln!("sb connect add: {}: {e}", paths.config.display());
            return 1;
        }
        println!("Added {name} to {}", paths.config.display());
    }
    if url.is_none() {
        println!(
            "Log in once:\n  CLAUDE_CONFIG_DIR={} claude\nthen run /login.",
            dir.display()
        );
    }
    0
}

fn append(path: &Path, text: &str) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(text.as_bytes())
}
