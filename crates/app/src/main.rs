//! The Switchboard window: Tauri glue around the hub.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use switchboard_core::hub::{Hub, OpenReq, Sink};
use switchboard_core::hub_review::Anchor;
use switchboard_core::paths::Paths;
use tauri::{AppHandle, Emitter, Manager, State, UserAttentionType};

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
            .args(["--app-name=Switchboard", &format!("{title}: {label}"), body])
            .stdin(std::process::Stdio::null())
            .spawn();
    }
}

type Res<T> = Result<T, String>;
type H<'a> = State<'a, Arc<Hub>>;

/// Runs blocking hub work off the async runtime. Errors and slow calls go to the log.
async fn run<T: Send + 'static>(
    name: &'static str,
    hub: &Arc<Hub>,
    f: impl FnOnce(&Hub) -> Res<T> + Send + 'static,
) -> Res<T> {
    let hub = Arc::clone(hub);
    let t0 = std::time::Instant::now();
    let out = tauri::async_runtime::spawn_blocking(move || f(&hub))
        .await
        .map_err(|e| e.to_string())?;
    let ms = t0.elapsed().as_millis() as u64;
    match &out {
        Err(err) => tracing::warn!(target: "sb::cmd", cmd = name, ms, %err, "command failed"),
        Ok(_) if ms > 1000 => tracing::warn!(target: "sb::cmd", cmd = name, ms, "slow command"),
        Ok(_) if name == "pane_input" => {
            tracing::trace!(target: "sb::cmd", cmd = name, ms, "command")
        }
        Ok(_) => tracing::debug!(target: "sb::cmd", cmd = name, ms, "command"),
    }
    out
}

macro_rules! commands {
    ($( $name:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty => |$h:ident| $body:expr; )*) => {
        $(
            #[tauri::command]
            async fn $name(hub: H<'_>, $($arg: $ty),*) -> Res<$ret> {
                run(stringify!($name), &hub, move |$h: &Hub| $body).await
            }
        )*
    };
}

commands! {
    panes() -> Value => |h| Ok(json!(h.views()));
    pane_open(req: OpenReq) -> Value => |h| h.open(req).map(|v| json!(v));
    pane_expand(id: String) -> String => |h| h.expand(&id).map(|s| B64.encode(s));
    pane_collapse() -> () => |h| { h.collapse(); Ok(()) };
    pane_resize(cols: u16, rows: u16) -> () => |h| h.resize(cols, rows);
    pane_seen(id: String) -> () => |h| h.seen(&id);
    pane_close(id: String) -> () => |h| h.close(&id);
    pane_rename(id: String, title: Option<String>) -> () => |h| h.rename(&id, title);
    pane_reorder(ids: Vec<String>) -> () => |h| h.reorder(&ids);
    pane_resume(id: String) -> () => |h| h.resume(&id);
    permit_answer(permit: String, allow: bool) -> () => |h| h.permit_answer(&permit, allow);
    trust_answer(id: String, yes: bool) -> () => |h| h.trust(&id, yes);
    connections() -> Value => |h| Ok(json!(h.connection_view()));
    connection_set(name: String) -> () => |h| h.connection_set(&name);
    connection_resume(id: String) -> String => |h| h.connection_resume(&id);
    nvim_open(tree: String, file: Option<String>, line: Option<u32>) -> String => |h| {
        let f = file.as_deref().map(|f| (f, line.unwrap_or(1)));
        h.nvim_open(std::path::Path::new(&tree), f)
    };
    repos() -> Value => |h| Ok(json!(h.repos()));
    layouts() -> Vec<String> => |h| Ok(h.layouts());
    layout_save(name: String) -> () => |h| h.layout_save(&name);
    layout_open(name: String) -> () => |h| h.layout_open(&name);
    settings() -> Value => |h| {
        let c = h.config();
        Ok(json!({ "leader": c.leader, "nvim": switchboard_core::nvim::installed() }))
    };
    reviews() -> Value => |h| Ok(json!(h.reviews()));
    review_open(url: String) -> String => |h| h.review_open(&url);
    review_view(id: String) -> Value => |h| h.review_view(&id).map(|v| json!(v));
    review_diff(id: String, step: String) -> Value => |h| h.review_diff(&id, &step).map(|v| json!(v));
    review_file_rows(id: String, path: String) -> Value => |h| h.review_file_rows(&id, &path).map(|v| json!(v));
    review_files(id: String) -> Value => |h| h.review_files(&id).map(|v| json!(v));
    review_file_diff(id: String, path: String) -> Value => |h| h.review_file_diff(&id, &path).map(|v| json!(v));
    review_mark(id: String, step: String, checked: bool) -> () => |h| h.review_mark(&id, &step, checked);
    review_ask(id: String, step: String, anchor: Option<Anchor>, thread: Option<String>, text: String) -> String => |h| h.review_ask(&id, &step, anchor, thread, &text);
    review_pin(id: String, thread: String) -> () => |h| h.review_pin(&id, &thread);
    review_draft(id: String, thread: String) -> () => |h| h.review_draft(&id, &thread);
    review_pin_edit(id: String, pin: i64, text: Option<String>) -> () => |h| h.review_pin_edit(&id, pin, text);
    review_draft_edit(id: String, draft: i64, text: Option<String>) -> () => |h| h.review_draft_edit(&id, draft, text);
    review_drafts_text(id: String) -> String => |h| h.review_drafts_text(&id);
    review_comment(id: String, path: String, side: String, line: u32, start_line: Option<u32>, text: String) -> () => |h| h.review_comment(&id, &path, &side, line, start_line, &text);
    review_summary(id: String, text: String) -> () => |h| h.review_summary(&id, &text);
    review_post_preview(id: String) -> Value => |h| h.review_post_preview(&id).map(|v| json!(v));
    review_post(id: String, event: String, summary: String, token: String) -> String => |h| h.review_post(&id, &event, &summary, &token);
    review_update(id: String) -> () => |h| h.review_update(&id);
    review_round(id: String) -> String => |h| h.review_round(&id);
    review_retry(id: String) -> () => |h| h.review_retry(&id);
    review_close(id: String) -> () => |h| h.review_close(&id);
    review_nvim(id: String, path: String, line: u32) -> String => |h| h.review_nvim(&id, &path, line);
}

/// Keys from xterm.js. `binary` data (mouse reports) is one byte per char.
#[tauri::command]
async fn pane_input(hub: H<'_>, id: String, data: String, binary: bool) -> Res<()> {
    let bytes: Vec<u8> = if binary {
        data.chars().map(|c| c as u8).collect()
    } else {
        data.into_bytes()
    };
    run("pane_input", &hub, move |h| h.input(&id, &bytes)).await
}

/// Opens an http(s) link in the browser.
#[tauri::command]
fn open_url(url: String) -> Res<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only http and https links open".into());
    }
    tracing::info!(target: "sb::app", %url, "open link");
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(target_os = "macos"))]
    let opener = "xdg-open";
    std::process::Command::new(opener)
        .arg(&url)
        .stdin(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

/// Log lines from the window.
#[tauri::command]
fn log(level: String, message: String) {
    match level.as_str() {
        "error" => tracing::error!(target: "sb::web", "{message}"),
        "warn" => tracing::warn!(target: "sb::web", "{message}"),
        "debug" => tracing::debug!(target: "sb::web", "{message}"),
        _ => tracing::info!(target: "sb::web", "{message}"),
    }
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
            let hub = Hub::start(Paths::from_env(), sb_path(), sink).inspect_err(
                |err| tracing::error!(target: "sb::app", %err, "the hub did not start"),
            )?;
            app.manage(hub);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            panes,
            pane_open,
            pane_expand,
            pane_collapse,
            pane_input,
            pane_resize,
            pane_seen,
            pane_close,
            pane_rename,
            pane_reorder,
            pane_resume,
            permit_answer,
            trust_answer,
            connections,
            connection_set,
            connection_resume,
            nvim_open,
            repos,
            layouts,
            layout_save,
            layout_open,
            settings,
            reviews,
            review_open,
            review_view,
            review_diff,
            review_file_rows,
            review_files,
            review_file_diff,
            review_mark,
            review_ask,
            review_pin,
            review_draft,
            review_pin_edit,
            review_draft_edit,
            review_drafts_text,
            review_comment,
            review_summary,
            review_post_preview,
            review_post,
            review_update,
            review_round,
            review_retry,
            review_close,
            review_nvim,
            log,
            open_url,
        ])
        .run(tauri::generate_context!())
        .expect("the window failed to start");
}
