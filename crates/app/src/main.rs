//! The Switchboard window: Tauri glue around the hub.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::sync::{mpsc, Arc, Mutex};
use switchboard_core::config::Config;
use switchboard_core::dispatch;
use switchboard_core::hub::{Hub, Sink};
use switchboard_core::paths::Paths;
use switchboard_core::remote::{Client, News};
use tauri::{AppHandle, Emitter, Manager, UserAttentionType};

struct AppSink {
    handle: AppHandle,
    /// The machine of this sink: true for the server.
    remote: bool,
    /// The machine the window shows (SPEC 29.13). News of the other one does not
    /// reach the window, but its panes do, as `other_pane(s)`, for the counts in the rail.
    on_remote: Arc<AtomicBool>,
}

impl Sink for AppSink {
    fn emit(&self, event: &str, payload: Value) {
        if event == "pane" && payload["alert"] == true {
            self.alert(&payload);
        }
        if self.shown() {
            let _ = self.handle.emit(event, payload);
        } else if event == "pane" || event == "panes" {
            let _ = self.handle.emit(&format!("other_{event}"), payload);
        }
    }

    fn output(&self, pane: &str, data: &[u8]) {
        if !self.shown() {
            return;
        }
        let _ = self.handle.emit(
            "pane_output",
            json!({ "pane": pane, "data": B64.encode(data) }),
        );
    }
}

impl AppSink {
    fn shown(&self) -> bool {
        self.on_remote.load(Ordering::SeqCst) == self.remote
    }

    /// A desktop notification and the urgency hint, when the window is not focused (SPEC 6.3).
    fn alert(&self, pane: &Value) {
        let label = match pane["lamp"].as_str() {
            Some("needs") => "Needs you",
            Some("turn") => "Your turn",
            Some("limit") => "Limit",
            Some("error") => "Error",
            _ => return,
        };
        let Some(win) = self.handle.get_webview_window("main") else {
            return;
        };
        if win.is_focused().unwrap_or(false) {
            return;
        }
        let _ = win.request_user_attention(Some(UserAttentionType::Informational));
        let title = pane["title"]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                let branch = pane["branch"].as_str().unwrap_or("");
                format!("{} · {branch}", pane["repo"].as_str().unwrap_or(""))
            });
        let body = pane["summary"].as_str().unwrap_or("");
        let _ = notify_command(&format!("{title}: {label}"), body)
            .stdin(std::process::Stdio::null())
            .spawn();
    }
}

/// The command that shows a desktop notification: `osascript` on macOS, which
/// has no `notify-send`. The text goes in as arguments of `on run argv`, never
/// into the script text, so a quote in a title can not change the script.
fn notify_command(title: &str, body: &str) -> std::process::Command {
    #[cfg(target_os = "macos")]
    {
        let mut c = std::process::Command::new("osascript");
        c.args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "-e",
            "end run",
            // `--`: a title that starts with `-` is not a flag.
            "--",
            title,
            body,
        ]);
        c
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut c = std::process::Command::new("notify-send");
        // `--`: a title from a pane that starts with `-` is not a flag.
        c.args(["--app-name=Switchboard", "--", title, body]);
        c
    }
}

/// The command that shows an error that stops the app: an alert on macOS, a
/// notification elsewhere. The text goes in as arguments, as in `notify_command`.
fn alert_command(title: &str, body: &str) -> std::process::Command {
    #[cfg(target_os = "macos")]
    {
        let mut c = std::process::Command::new("osascript");
        c.args([
            "-e",
            "on run argv",
            "-e",
            "display alert (item 1 of argv) message (item 2 of argv) as critical",
            "-e",
            "end run",
            "--",
            title,
            body,
        ]);
        c
    }
    #[cfg(not(target_os = "macos"))]
    {
        notify_command(title, body)
    }
}

type Res<T> = Result<T, String>;

/// Where the commands go (SPEC 29.13): the hub in this process, the hub on the
/// server, or both at once. The window shows one of them; a switch is instant.
struct Backend {
    local: Option<Arc<Hub>>,
    remote: Option<Remote>,
    /// The window shows the server. Always true with no hub here.
    on_remote: Arc<AtomicBool>,
    /// This computer's machine name.
    here: String,
}

/// The link to a remote hub, and what the window needs to know about it.
struct Remote {
    host: String,
    argv: Vec<String>,
    client: Mutex<Option<Arc<Client>>>,
    /// Why the link is down, or `None` while it is up.
    down: Arc<Mutex<Option<String>>>,
    sink: Arc<AppSink>,
    /// The leader key of this computer's config, not the server's.
    leader: String,
    /// The number of the newest link. News of an older link is dropped: its
    /// reader can end after the next link is up.
    gen: Arc<AtomicU64>,
    /// Held for the whole of a connect, so two connects (the start and a click
    /// on Reconnect) never run at once: the second could take over the first,
    /// and the window would keep the client that was taken over.
    connecting: Mutex<()>,
    /// Another window took the hub. The link then stays down until you take it back.
    taken: Arc<AtomicBool>,
}

impl Remote {
    /// Connects, and tells the window. On an error the link stays down with the reason.
    fn connect(&self) -> Res<()> {
        // Lock order: `connecting`, then `client`, then `down`.
        let _one = self.connecting.lock().unwrap();
        let old = self.client.lock().unwrap().take();
        if let Some(old) = old {
            old.close();
        }
        let g = self.gen.fetch_add(1, Ordering::SeqCst) + 1;
        let (sink, down, host, gen, taken_flag) = (Arc::clone(&self.sink), Arc::clone(&self.down), self.host.clone(), Arc::clone(&self.gen), Arc::clone(&self.taken));
        let news = move |n: News| {
            if gen.load(Ordering::SeqCst) != g {
                return;
            }
            match n {
                News::Event(ev, payload) => sink.emit(&ev, payload),
                News::Down { why, taken } => {
                    tracing::warn!(target: "sb::remote", %why, taken, "the link is down");
                    let mut d = down.lock().unwrap();
                    *d = Some(why.clone());
                    taken_flag.store(taken, Ordering::SeqCst);
                    let _ = sink.handle.emit("link", json!({ "up": false, "host": host, "why": why, "taken": taken, "retry": !taken }));
                }
            }
        };
        match Client::connect(&self.argv, news) {
            Ok(c) => {
                self.taken.store(false, Ordering::SeqCst);
                *self.client.lock().unwrap() = Some(Arc::clone(&c));
                // Under the `down` lock: a link that already went down (a takeover
                // right after the hello) keeps its why, and its banner.
                let mut d = self.down.lock().unwrap();
                if c.is_up() {
                    *d = None;
                    let _ = self.sink.handle.emit("link", json!({ "up": true, "host": self.host }));
                }
                Ok(())
            }
            Err(why) => {
                tracing::warn!(target: "sb::remote", %why, "connect failed");
                *self.down.lock().unwrap() = Some(why.clone());
                let _ = self.sink.handle.emit("link", json!({ "up": false, "host": self.host, "why": why, "taken": false, "retry": true }));
                Err(why)
            }
        }
    }

    fn is_up(&self) -> bool {
        self.down.lock().unwrap().is_none() && self.client.lock().unwrap().as_ref().is_some_and(|c| c.is_up())
    }

    /// Keeps the link up (SPEC 29.12): a link that went down connects again on its
    /// own, after 1 s, then 2, 4, 8 and every 15 s. Not after a takeover: two windows
    /// would take the hub from each other for ever.
    fn keep_up(&self) {
        let mut wait = Duration::from_secs(1);
        let mut next = Instant::now() + wait;
        loop {
            std::thread::sleep(Duration::from_millis(250));
            if self.is_up() || self.taken.load(Ordering::SeqCst) {
                wait = Duration::from_secs(1);
                next = Instant::now() + wait;
                continue;
            }
            if Instant::now() < next {
                continue;
            }
            tracing::info!(target: "sb::remote", secs = wait.as_secs(), "connect again on its own");
            let _ = self.connect();
            wait = (wait * 2).min(Duration::from_secs(15));
            next = Instant::now() + wait;
        }
    }

    fn call(&self, cmd: &str, args: Value) -> Res<Value> {
        let c = self.client.lock().unwrap().clone();
        match c {
            Some(c) => c.call(cmd, args),
            None => Err("the link to the server is down".into()),
        }
    }
}

/// A command that started. A remote one waits for its answer.
enum Pending {
    Done(Res<Value>),
    Wait(mpsc::Receiver<Res<Value>>),
}

impl Backend {
    /// The server link, when the window shows it.
    fn shown_remote(&self) -> Option<&Remote> {
        self.remote.as_ref().filter(|_| self.on_remote.load(Ordering::SeqCst) || self.local.is_none())
    }

    /// Shows the other machine: true for the server. The agents of both keep running.
    fn show(&self, remote: bool) -> Res<()> {
        if remote && self.remote.is_none() {
            return Err("No server: set remote = \"me@box\" in the config.".into());
        }
        if !remote && self.local.is_none() {
            return Err("SB_REMOTE runs this window on the server only.".into());
        }
        tracing::info!(target: "sb::app", remote, "show the machine");
        self.on_remote.store(remote, Ordering::SeqCst);
        Ok(())
    }

    /// The panes of the machine the window does not show, for the counts in the rail.
    fn other_panes(&self) -> Value {
        let out = if self.shown_remote().is_some() {
            self.local.as_ref().map(|h| dispatch::dispatch(h, "panes", &json!({})))
        } else {
            self.remote.as_ref().filter(|r| r.is_up()).map(|r| r.call("panes", json!({})))
        };
        out.and_then(Result::ok).unwrap_or_else(|| json!([]))
    }

    /// Starts one command without a wait for a remote answer, so the next one can
    /// go out behind it in order. A local one runs to the end here.
    fn start(&self, cmd: &str, args: Value) -> Pending {
        match (self.shown_remote(), &self.local) {
            (None, Some(hub)) => Pending::Done(dispatch::dispatch(hub, cmd, &args)),
            (None, None) => Pending::Done(Err("no hub".into())),
            (Some(r), _) => {
                let c = r.client.lock().unwrap().clone();
                match c.map(|c| c.send(cmd, args)) {
                    Some(Ok(rx)) => Pending::Wait(rx),
                    Some(Err(e)) => Pending::Done(Err(e)),
                    None => Pending::Done(Err("the link to the server is down".into())),
                }
            }
        }
    }

    /// One command of the window. It blocks: run it off the async runtime.
    fn call(&self, cmd: &str, args: Value) -> Res<Value> {
        match (self.shown_remote(), &self.local) {
            (None, Some(hub)) => dispatch::dispatch(hub, cmd, &args),
            (None, None) => Err("no hub".into()),
            (Some(r), _) => {
                let mut v = r.call(cmd, args)?;
                // The keys are of this computer.
                if cmd == "settings" {
                    v["leader"] = json!(r.leader);
                }
                Ok(v)
            }
        }
    }

    /// The state of the link the window shows, and `box`: the server link's own,
    /// for its dot in the rail. `both`: the rail lists the two machines.
    fn link_state(&self) -> Value {
        let server = self.remote.as_ref().map(|r| {
            let why = r.down.lock().unwrap().clone();
            let up = r.client.lock().unwrap().as_ref().is_some_and(|c| c.is_up());
            json!({ "host": r.host, "up": up && why.is_none(), "why": why, "taken": r.taken.load(Ordering::SeqCst) })
        });
        let mut v = match (self.shown_remote(), &server) {
            (Some(_), Some(b)) => json!({ "remote": true, "host": b["host"], "up": b["up"], "why": b["why"], "taken": b["taken"] }),
            _ => json!({ "remote": false, "up": true }),
        };
        v["box"] = server.unwrap_or(Value::Null);
        v["both"] = json!(self.local.is_some() && self.remote.is_some());
        v["here"] = json!(self.here);
        v
    }
}

/// Runs blocking work off the async runtime. Errors and slow calls go to the log.
async fn run(name: String, backend: Arc<Backend>, args: Value) -> Res<Value> {
    let t0 = std::time::Instant::now();
    let n = name.clone();
    let out = tauri::async_runtime::spawn_blocking(move || backend.call(&n, args))
        .await
        .map_err(|e| e.to_string())?;
    note(&name, t0, &out);
    out
}

/// The commands that change the screen or type, in the order of the window (SPEC 29).
/// One thread starts them one by one; their answers come back in any order.
type Job = (String, Value, tauri::ipc::InvokeResolver<tauri::Wry>);
struct Lane(mpsc::Sender<Job>);

fn lane(backend: Arc<Backend>, jobs: mpsc::Receiver<Job>) {
    for (name, args, resolver) in jobs {
        let t0 = std::time::Instant::now();
        match backend.start(&name, args) {
            Pending::Done(out) => {
                note(&name, t0, &out);
                resolver.respond(out.map_err(Into::into));
            }
            Pending::Wait(rx) => {
                tauri::async_runtime::spawn_blocking(move || {
                    let out = rx.recv().unwrap_or_else(|_| Err("the link to the server is down".into()));
                    note(&name, t0, &out);
                    resolver.respond(out.map_err(Into::into));
                });
            }
        }
    }
}

/// Errors and slow commands go to the log.
fn note(name: &str, t0: std::time::Instant, out: &Res<Value>) {
    let ms = t0.elapsed().as_millis() as u64;
    match out {
        Err(err) => tracing::warn!(target: "sb::cmd", cmd = %name, ms, %err, "command failed"),
        Ok(_) if ms > 1000 => tracing::warn!(target: "sb::cmd", cmd = %name, ms, "slow command"),
        Ok(_) if name == "pane_input" => tracing::trace!(target: "sb::cmd", cmd = %name, ms, "command"),
        Ok(_) => tracing::debug!(target: "sb::cmd", cmd = %name, ms, "command"),
    }
}

/// Opens an http(s) link in the browser. It runs here, in remote mode too.
fn open_url(url: &str) -> Res<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only http and https links open".into());
    }
    tracing::info!(target: "sb::app", %url, "open link");
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(target_os = "macos"))]
    let opener = "xdg-open";
    std::process::Command::new(opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

/// Writes text to the clipboard of this computer, for a copy of a program in a pane
/// (OSC 52, SPEC 29.11). It runs here, in remote mode too. A pane can only write
/// the clipboard, never read it.
fn clip_write(text: &str) -> Res<()> {
    use std::io::Write;
    if text.len() > 1 << 20 {
        return Err("the copy is over 1 MB".into());
    }
    #[cfg(target_os = "macos")]
    let tools: &[&[&str]] = &[&["pbcopy"]];
    #[cfg(not(target_os = "macos"))]
    let tools: &[&[&str]] = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        &[&["wl-copy"], &["xclip", "-selection", "clipboard"], &["xsel", "-ib"]]
    } else {
        &[&["xclip", "-selection", "clipboard"], &["xsel", "-ib"], &["wl-copy"]]
    };
    for argv in tools {
        let Ok(mut child) = std::process::Command::new(argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        // wl-copy and xclip stay to serve the clipboard: wait only for the copy to start.
        if child.wait().map(|s| s.success()).unwrap_or(false) {
            tracing::debug!(target: "sb::app", bytes = text.len(), tool = argv[0], "copy");
            return Ok(());
        }
    }
    Err("no clipboard tool: install pbcopy, wl-copy or xclip".into())
}

/// The handover on this computer (SPEC 38.2). It runs here, in remote mode only:
/// the work comes from the server the window is linked to. `go` false is the dry run.
fn handover_here(remote: bool, go: bool, plan: Value) -> Res<Value> {
    use switchboard_core::handover;
    if !remote {
        return Err("The window is not linked to a server.".into());
    }
    let host = switchboard_core::config::sb_remote_env()
        .or_else(|| SWITCH.get().and_then(|s| s.host.clone()))
        .ok_or("A handover needs remote = \"me@box\" in the config: git fetches over ssh from that host.")?;
    let plan: handover::Plan = serde_json::from_value(plan).map_err(|e| format!("the plan does not parse: {e}"))?;
    let paths = Paths::from_env();
    let config = Config::load(&paths.config).unwrap_or_default();
    let source = handover::source(&host, &plan.tree);
    tracing::info!(target: "sb::app", %source, go, "handover here");
    if go {
        handover::bring_in_here(&paths, &config, &source, &host, &plan).map(|p| json!(p))
    } else {
        handover::target(&paths, &config, &source, &plan).map(|t| json!(t))
    }
}

/// Log lines from the window.
fn log(level: &str, message: &str) {
    match level {
        "error" => tracing::error!(target: "sb::web", "{message}"),
        "warn" => tracing::warn!(target: "sb::web", "{message}"),
        "debug" => tracing::debug!(target: "sb::web", "{message}"),
        _ => tracing::info!(target: "sb::web", "{message}"),
    }
}

/// The parse error of this computer's config, when it has one.
static CONFIG_ERROR: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// What the switch between here and the server needs: this computer's config
/// file, and the host to switch to. No host, or `SB_REMOTE` set: no switch.
struct Switch {
    config: PathBuf,
    host: Option<String>,
}
static SWITCH: std::sync::OnceLock<Switch> = std::sync::OnceLock::new();

/// The machine the window shows goes into this computer's config, so the next
/// start opens on it (`remote_off`, SPEC 29.13). A failure only goes to the log.
fn remember_machine(remote: bool) {
    let Some(s) = SWITCH.get() else { return };
    if s.host.is_none() || switchboard_core::config::sb_remote_env().is_some() {
        return;
    }
    if let Err(err) = Config::set_remote_off(&s.config, !remote) {
        tracing::warn!(target: "sb::app", %err, "the machine shown was not saved");
    }
}

/// The commands that run in the app, in remote mode too. Every other command goes
/// to the backend. A test keeps this list short (SPEC 29).
const LOCAL: &[&str] = &["log", "open_url", "clip_write", "handover_check", "handover_in", "link_state", "link_reconnect", "machine_show", "machine_other", "app_restart"];

/// The one handler of every `invoke` of the window.
fn handle(invoke: tauri::ipc::Invoke) -> bool {
    let cmd = invoke.message.command().to_owned();
    let args = match invoke.message.payload() {
        tauri::ipc::InvokeBody::Json(v) => v.clone(),
        _ => json!({}),
    };
    let webview = invoke.message.webview();
    let Some(backend) = webview.try_state::<Arc<Backend>>().map(|s| Arc::clone(s.inner())) else {
        invoke.resolver.reject("the app is starting");
        return true;
    };
    if dispatch::ORDERED.contains(&cmd.as_str()) {
        let Some(lane) = webview.try_state::<Lane>() else {
            invoke.resolver.reject("the app is starting");
            return true;
        };
        if let Err(mpsc::SendError((_, _, resolver))) = lane.0.send((cmd, args, invoke.resolver)) {
            resolver.reject("the app is stopping");
        }
        return true;
    }
    if !LOCAL.contains(&cmd.as_str()) {
        invoke.resolver.respond_async(async move { run(cmd, backend, args).await.map_err(Into::into) });
        return true;
    }
    let text = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    match cmd.as_str() {
        "log" => {
            log(&text("level"), &text("message"));
            invoke.resolver.resolve(Value::Null);
        }
        "open_url" => invoke.resolver.respond(open_url(&text("url")).map(|_| Value::Null).map_err(Into::into)),
        // The handover on this computer (SPEC 38.2): the dry run, then the real one.
        "handover_check" | "handover_in" => {
            let remote = backend.shown_remote().is_some();
            let plan = args.get("plan").cloned().unwrap_or(Value::Null);
            invoke.resolver.respond_async(async move {
                tauri::async_runtime::spawn_blocking(move || handover_here(remote, cmd == "handover_in", plan))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r)
                    .map_err(Into::into)
            });
        }
        "clip_write" => {
            let t = text("text");
            invoke.resolver.respond_async(async move {
                tauri::async_runtime::spawn_blocking(move || clip_write(&t)).await.map_err(|e| e.to_string()).and_then(|r| r).map(|_| Value::Null).map_err(Into::into)
            });
        }
        "link_state" => {
            let mut v = backend.link_state();
            if let Some(err) = CONFIG_ERROR.get() {
                v["config_error"] = json!(format!("The config does not parse, so the defaults are in use: {err}"));
            }
            invoke.resolver.resolve(v)
        }
        // The window shows the other machine, at once (SPEC 29.13). The next start
        // opens on it too.
        "machine_show" => {
            let remote = args.get("remote").and_then(Value::as_bool).unwrap_or(false);
            match backend.show(remote) {
                Ok(()) => {
                    remember_machine(remote);
                    invoke.resolver.resolve(backend.link_state());
                }
                Err(e) => invoke.resolver.reject(e),
            }
        }
        "machine_other" => invoke.resolver.respond_async(async move {
            tauri::async_runtime::spawn_blocking(move || backend.other_panes()).await.map_err(|e| e.to_string().into())
        }),
        // Restart (SPEC 29.12). `hub`: the hub on the server starts again with its
        // newest build, and the link comes back on its own. Else this app restarts;
        // a hub here restarts with it. Agents keep running in tmux.
        "app_restart" => {
            let hub = args.get("hub").and_then(Value::as_bool).unwrap_or(false);
            match backend.shown_remote() {
                Some(_) if hub => invoke.resolver.respond_async(async move { run("hub_restart".into(), backend, json!({})).await.map_err(Into::into) }),
                _ => {
                    tracing::info!(target: "sb::app", "restart asked");
                    invoke.resolver.resolve(Value::Null);
                    webview.app_handle().request_restart();
                }
            }
        }
        "link_reconnect" => invoke.resolver.respond_async(async move {
            let b = Arc::clone(&backend);
            let r = tauri::async_runtime::spawn_blocking(move || match &b.remote {
                Some(r) => r.connect(),
                None => Ok(()),
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
            r.map(|_| backend.link_state()).map_err(Into::into)
        }),
        _ => invoke.resolver.reject(format!("unknown command {cmd}")),
    }
    true
}

/// Logs go to `~/.switchboard/logs/switchboard.log` and to stderr.
/// `SB_LOG` sets the filter, for example `SB_LOG=sb=trace`. The default is `info,sb=debug`.
fn init_logs(paths: &Paths) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::prelude::*;
    let dir = paths.home.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let (file, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::never(&dir, "switchboard.log"));
    let filter = tracing_subscriber::EnvFilter::try_from_env("SB_LOG")
        .unwrap_or_else(|_| "info,sb=debug".into());
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(file)
                .with_ansi(false),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();
    tracing::info!(target: "sb::app", version = env!("CARGO_PKG_VERSION"), log = %dir.join("switchboard.log").display(), "Switchboard starting");
    guard
}

fn sb_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .map(|p| p.with_file_name("sb"))
        .filter(|p| p.exists())
        .unwrap_or_else(|| "sb".into())
}

fn main() {
    // A macOS app from Finder has a short PATH: take the login shell's PATH.
    #[cfg(target_os = "macos")]
    {
        let path = switchboard_core::paths::login_path()
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default();
        std::env::set_var("PATH", switchboard_core::paths::with_tool_dirs(&path));
    }
    let paths = Paths::from_env();
    let _logs = init_logs(&paths);
    std::panic::set_hook(Box::new(
        |info| tracing::error!(target: "sb::app", %info, "panic"),
    ));
    tauri::Builder::default()
        .setup(|app| {
            let paths = Paths::from_env();
            // A config that does not parse: the defaults, with the error in the log
            // and on the screen. `link_state` carries it, so the window shows it
            // even when it loads after an event would have gone.
            let config = Config::load(&paths.config).unwrap_or_else(|err| {
                tracing::error!(target: "sb::app", %err, "the config does not parse; the defaults are used");
                let _ = CONFIG_ERROR.set(err);
                Config::default()
            });
            let _ = SWITCH.set(Switch { config: paths.config.clone(), host: config.remote.clone() });
            // SB_REMOTE: the server only, for one run. Else a hub here always runs,
            // and with `remote` the server's link too (SPEC 29.13).
            let only_server = switchboard_core::config::sb_remote_env().is_some();
            let server = config.server_argv();
            let on_remote = Arc::new(AtomicBool::new(only_server || (server.is_some() && !config.remote_off)));
            let sink_of = |remote: bool| Arc::new(AppSink { handle: app.handle().clone(), remote, on_remote: Arc::clone(&on_remote) });
            let remote = server.map(|(host, argv)| {
                tracing::info!(target: "sb::app", %host, ?argv, "server link");
                Remote {
                    host,
                    argv,
                    client: Mutex::default(),
                    down: Arc::new(Mutex::new(Some("connecting".into()))),
                    sink: sink_of(true),
                    leader: config.leader.clone(),
                    gen: Arc::default(),
                    connecting: Mutex::default(),
                    taken: Arc::default(),
                }
            });
            let local = if only_server {
                None
            } else {
                match Hub::start(paths, sb_path(), sink_of(false)) {
                    Ok(h) => Some(h),
                    // With a server, the window still opens, on it. Else no window
                    // can show it yet: an alert, then quit.
                    Err(err) if remote.is_some() => {
                        tracing::error!(target: "sb::app", %err, "the hub here did not start; the server only");
                        on_remote.store(true, Ordering::SeqCst);
                        None
                    }
                    Err(err) => {
                        tracing::error!(target: "sb::app", %err, "the hub did not start");
                        let _ = alert_command("Switchboard did not start", &err)
                            .stdin(std::process::Stdio::null())
                            .status();
                        std::process::exit(1);
                    }
                }
            };
            let here = switchboard_core::mail::machine_name(&config);
            let backend = Arc::new(Backend { local, remote, on_remote, here });
            app.manage(Arc::clone(&backend));
            let (jobs, rx) = mpsc::channel();
            {
                let backend = Arc::clone(&backend);
                std::thread::spawn(move || lane(backend, rx));
            }
            app.manage(Lane(jobs));
            // The link comes up in the background: the window shows while ssh connects.
            if backend.remote.is_some() {
                std::thread::spawn(move || {
                    if let Some(r) = &backend.remote {
                        let _ = r.connect();
                        r.keep_up();
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(handle)
        .build(tauri::generate_context!())
        .expect("the window failed to start")
        .run(|app, event| {
            // Review runs have process groups of their own: no terminal signal and
            // no watchdog reaches them after a quit. A remote hub keeps its runs.
            if let tauri::RunEvent::Exit = event {
                if let Some(b) = app.try_state::<Arc<Backend>>() {
                    if let Some(hub) = &b.local {
                        hub.stop_runs();
                    }
                    if let Some(r) = &b.remote {
                        if let Some(c) = r.client.lock().unwrap().take() {
                            c.close();
                        }
                    }
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In remote mode only these run on this computer. A server can make the app do
    /// nothing else here (SPEC 29).
    #[test]
    fn the_local_commands_stay_few() {
        assert_eq!(LOCAL, &["log", "open_url", "clip_write", "handover_check", "handover_in", "link_state", "link_reconnect", "machine_show", "machine_other", "app_restart"]);
        for c in LOCAL {
            assert!(!dispatch::COMMANDS.contains(c), "{c} is a hub command too");
        }
    }

    /// The text of a notification is an argument, never part of the script.
    #[test]
    fn a_notification_keeps_its_text_out_of_the_command() {
        let c = notify_command("-it's \"x\"", "b");
        let args: Vec<String> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        let n = args.len();
        assert_eq!(&args[n - 3..], &["--", "-it's \"x\"", "b"]);
        assert!(args[..n - 3].iter().all(|a| !a.contains("it's")));
    }

    #[test]
    fn open_url_takes_only_web_links() {
        assert!(open_url("file:///etc/passwd").is_err());
        assert!(open_url("javascript:alert(1)").is_err());
        assert!(open_url("-x").is_err());
    }
}
