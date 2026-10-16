//! The Switchboard window: Tauri glue around the hub.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use switchboard_core::config::Config;
use switchboard_core::dispatch;
use switchboard_core::hub::{Hub, Sink};
use switchboard_core::paths::Paths;
use switchboard_core::remote::{Client, News};
use tauri::{AppHandle, Emitter, Manager, UserAttentionType};

struct AppSink {
    handle: AppHandle,
}

impl Sink for AppSink {
    fn emit(&self, event: &str, payload: Value) {
        if event == "pane" && payload["alert"] == true {
            self.alert(&payload);
        }
        let _ = self.handle.emit(event, payload);
    }

    fn output(&self, pane: &str, data: &[u8]) {
        let _ = self.handle.emit(
            "pane_output",
            json!({ "pane": pane, "data": B64.encode(data) }),
        );
    }
}

impl AppSink {
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
        let _ = std::process::Command::new("notify-send")
            // `--`: a title from a pane that starts with `-` is not a flag.
            .args(["--app-name=Switchboard", "--", &format!("{title}: {label}"), body])
            .stdin(std::process::Stdio::null())
            .spawn();
    }
}

type Res<T> = Result<T, String>;

/// Where the commands go: the hub in this process, or a remote hub (SPEC 29).
enum Backend {
    Local(Arc<Hub>),
    Remote(Remote),
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
}

impl Remote {
    /// Connects, and tells the window. On an error the link stays down with the reason.
    fn connect(&self) -> Res<()> {
        if let Some(old) = self.client.lock().unwrap().take() {
            old.close();
        }
        let (sink, down, host) = (Arc::clone(&self.sink), Arc::clone(&self.down), self.host.clone());
        let news = move |n: News| match n {
            News::Event(ev, payload) => sink.emit(&ev, payload),
            News::Down { why, taken } => {
                tracing::warn!(target: "sb::remote", %why, taken, "the link is down");
                *down.lock().unwrap() = Some(why.clone());
                let _ = sink.handle.emit("link", json!({ "up": false, "host": host, "why": why, "taken": taken }));
            }
        };
        match Client::connect(&self.argv, news) {
            Ok(c) => {
                *self.client.lock().unwrap() = Some(c);
                *self.down.lock().unwrap() = None;
                let _ = self.sink.handle.emit("link", json!({ "up": true, "host": self.host }));
                Ok(())
            }
            Err(why) => {
                tracing::warn!(target: "sb::remote", %why, "connect failed");
                *self.down.lock().unwrap() = Some(why.clone());
                let _ = self.sink.handle.emit("link", json!({ "up": false, "host": self.host, "why": why, "taken": false }));
                Err(why)
            }
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

impl Backend {
    /// One command of the window. It blocks: run it off the async runtime.
    fn call(&self, cmd: &str, args: Value) -> Res<Value> {
        match self {
            Backend::Local(hub) => dispatch::dispatch(hub, cmd, &args),
            Backend::Remote(r) => {
                let mut v = r.call(cmd, args)?;
                // The keys are of this computer.
                if cmd == "settings" {
                    v["leader"] = json!(r.leader);
                }
                Ok(v)
            }
        }
    }

    fn link_state(&self) -> Value {
        match self {
            Backend::Local(_) => json!({ "remote": false, "up": true }),
            Backend::Remote(r) => {
                let why = r.down.lock().unwrap().clone();
                json!({ "remote": true, "host": r.host, "up": why.is_none(), "why": why })
            }
        }
    }
}

/// Runs blocking work off the async runtime. Errors and slow calls go to the log.
async fn run(name: String, backend: Arc<Backend>, args: Value) -> Res<Value> {
    let t0 = std::time::Instant::now();
    let n = name.clone();
    let out = tauri::async_runtime::spawn_blocking(move || backend.call(&n, args))
        .await
        .map_err(|e| e.to_string())?;
    let ms = t0.elapsed().as_millis() as u64;
    match &out {
        Err(err) => tracing::warn!(target: "sb::cmd", cmd = %name, ms, %err, "command failed"),
        Ok(_) if ms > 1000 => tracing::warn!(target: "sb::cmd", cmd = %name, ms, "slow command"),
        Ok(_) if name == "pane_input" => tracing::trace!(target: "sb::cmd", cmd = %name, ms, "command"),
        Ok(_) => tracing::debug!(target: "sb::cmd", cmd = %name, ms, "command"),
    }
    out
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

/// Log lines from the window.
fn log(level: &str, message: &str) {
    match level {
        "error" => tracing::error!(target: "sb::web", "{message}"),
        "warn" => tracing::warn!(target: "sb::web", "{message}"),
        "debug" => tracing::debug!(target: "sb::web", "{message}"),
        _ => tracing::info!(target: "sb::web", "{message}"),
    }
}

/// The commands that run in the app, in remote mode too. Every other command goes
/// to the backend. A test keeps this list short (SPEC 29).
const LOCAL: &[&str] = &["log", "open_url", "link_state", "link_reconnect"];

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
        "link_state" => invoke.resolver.resolve(backend.link_state()),
        "link_reconnect" => invoke.resolver.respond_async(async move {
            let b = Arc::clone(&backend);
            let r = tauri::async_runtime::spawn_blocking(move || match &*b {
                Backend::Remote(r) => r.connect(),
                Backend::Local(_) => Ok(()),
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
            let sink = Arc::new(AppSink {
                handle: app.handle().clone(),
            });
            let paths = Paths::from_env();
            let config = Config::load(&paths.config).unwrap_or_default();
            let backend = match config.remote_argv() {
                Some((host, argv)) => {
                    tracing::info!(target: "sb::app", %host, ?argv, "remote mode");
                    let remote = Remote {
                        host,
                        argv,
                        client: Mutex::default(),
                        down: Arc::new(Mutex::new(Some("connecting".into()))),
                        sink,
                        leader: config.leader.clone(),
                    };
                    Arc::new(Backend::Remote(remote))
                }
                None => {
                    let hub = Hub::start(paths, sb_path(), sink).inspect_err(
                        |err| tracing::error!(target: "sb::app", %err, "the hub did not start"),
                    )?;
                    Arc::new(Backend::Local(hub))
                }
            };
            app.manage(Arc::clone(&backend));
            // The link comes up in the background: the window shows while ssh connects.
            if matches!(*backend, Backend::Remote(_)) {
                std::thread::spawn(move || {
                    if let Backend::Remote(r) = &*backend {
                        let _ = r.connect();
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
                    match &**b {
                        Backend::Local(hub) => hub.stop_runs(),
                        Backend::Remote(r) => {
                            if let Some(c) = r.client.lock().unwrap().take() {
                                c.close();
                            }
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
        assert_eq!(LOCAL, &["log", "open_url", "link_state", "link_reconnect"]);
        for c in LOCAL {
            assert!(!dispatch::COMMANDS.contains(c), "{c} is a hub command too");
        }
    }

    #[test]
    fn open_url_takes_only_web_links() {
        assert!(open_url("file:///etc/passwd").is_err());
        assert!(open_url("javascript:alert(1)").is_err());
        assert!(open_url("-x").is_err());
    }
}
