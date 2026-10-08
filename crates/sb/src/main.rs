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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("state") => state(args.get(1).map(String::as_str).unwrap_or("")),
        Some("permit") => permit(),
        Some("open") => open(&args[1..]),
        Some("mcp") => mcp::serve(),
        Some("connect") if args.get(1).map(String::as_str) == Some("add") => {
            connect_add(&args[2..])
        }
        Some("--version") => {
            println!("sb {}", env!("CARGO_PKG_VERSION"));
            0
        }
        _ => {
            eprintln!("usage: sb open [path] [-w branch] [--nvim|--shell]\n       sb connect add <name> [--endpoint <url> --model <m> --token <keyring:name>]");
            2
        }
    };
    std::process::exit(code);
}

fn stdin_json() -> Value {
    let mut s = String::new();
    let _ = std::io::stdin().read_to_string(&mut s);
    serde_json::from_str(&s).unwrap_or(Value::Null)
}

/// Sends one request. With `reply`, waits for one line back.
pub fn send(req: &Request, reply: bool) -> Result<Option<String>, String> {
    let sock = Paths::from_env().sock();
    let mut s = UnixStream::connect(&sock).map_err(|e| format!("{}: {e}", sock.display()))?;
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
