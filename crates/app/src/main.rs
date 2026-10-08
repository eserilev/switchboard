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
        let Some(win) = self.handle.get_webview_window("main") else {
            return;
        };
        if win.is_focused().unwrap_or(false) {
            return;
        }
        let _ = win.request_user_attention(Some(UserAttentionType::Informational));
        let label = match pane["lamp"].as_str() {
            Some("needs") => "Needs you",
            Some("turn") => "Your turn",
            Some("limit") => "Limit",
            Some("error") => "Error",
            _ => return,
        };
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

/// Runs blocking hub work off the async runtime.
async fn run<T: Send + 'static>(
    hub: &Arc<Hub>,
    f: impl FnOnce(&Hub) -> Res<T> + Send + 'static,
) -> Res<T> {
    let hub = Arc::clone(hub);
    tauri::async_runtime::spawn_blocking(move || f(&hub))
        .await
        .map_err(|e| e.to_string())?
}

macro_rules! commands {
    ($( $name:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty => |$h:ident| $body:expr; )*) => {
        $(
            #[tauri::command]
            async fn $name(hub: H<'_>, $($arg: $ty),*) -> Res<$ret> {
                run(&hub, move |$h: &Hub| $body).await
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
    review_mark(id: String, step: String, checked: bool) -> () => |h| h.review_mark(&id, &step, checked);
    review_ask(id: String, step: String, anchor: Option<Anchor>, thread: Option<String>, text: String) -> String => |h| h.review_ask(&id, &step, anchor, thread, &text);
    review_pin(id: String, thread: String) -> () => |h| h.review_pin(&id, &thread);
    review_draft(id: String, thread: String) -> () => |h| h.review_draft(&id, &thread);
    review_pin_edit(id: String, pin: i64, text: Option<String>) -> () => |h| h.review_pin_edit(&id, pin, text);
    review_draft_edit(id: String, draft: i64, text: Option<String>) -> () => |h| h.review_draft_edit(&id, draft, text);
    review_drafts_text(id: String) -> String => |h| h.review_drafts_text(&id);
    review_update(id: String) -> () => |h| h.review_update(&id);
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
    run(&hub, move |h| h.input(&id, &bytes)).await
}

fn sb_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .map(|p| p.with_file_name("sb"))
        .filter(|p| p.exists())
        .unwrap_or_else(|| "sb".into())
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let sink = Arc::new(AppSink {
                handle: app.handle().clone(),
            });
            let hub = Hub::start(Paths::from_env(), sb_path(), sink)?;
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
            review_mark,
            review_ask,
            review_pin,
            review_draft,
            review_pin_edit,
            review_draft_edit,
            review_drafts_text,
            review_update,
            review_retry,
            review_close,
            review_nvim,
        ])
        .run(tauri::generate_context!())
        .expect("the window failed to start");
}
