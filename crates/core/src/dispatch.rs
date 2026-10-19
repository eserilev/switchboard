//! The commands of the window, for the app and for `sb serve` (SPEC 29). One table,
//! so the two can never drift apart. Arguments come as a JSON object, with the
//! camelCase names that Tauri uses, or the snake_case names.

use crate::hub::{Hub, OpenReq};
use crate::hub_review::{Anchor, NewComment};
use crate::tasks::{NewSprint, NewTask, Place, TaskFields, TaskMove};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};

type Res<T> = Result<T, String>;

fn b64(s: &[u8]) -> String {
    B64.encode(s)
}

/// `round_of` as Tauri names it: `roundOf`.
fn camel(name: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in name.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// One argument. A missing one is null, so an `Option` gets `None`.
fn arg<T: serde::de::DeserializeOwned>(args: &Value, name: &str) -> Res<T> {
    let v = args.get(camel(name)).or_else(|| args.get(name)).cloned().unwrap_or(Value::Null);
    serde_json::from_value(v).map_err(|e| format!("argument {name}: {e}"))
}

/// The commands that run in the order the window sends them (SPEC 29): they change
/// what the window shows, or type in a pane. The app sends them in one lane, and
/// `sb serve` runs them in its read loop. The other commands run side by side.
pub const ORDERED: &[&str] = &[
    "pane_expand",
    "pane_attach",
    "pane_detach",
    "pane_collapse",
    "pane_resize",
    "part_resize",
    "pane_input",
];

macro_rules! commands {
    ($( $name:ident ( $($a:ident : $ty:ty),* ) -> $ret:ty => |$h:ident| $body:expr; )*) => {
        /// The name of every command that [`dispatch`] knows.
        pub const COMMANDS: &[&str] = &[$(stringify!($name)),*];

        /// Runs one command of the window on the hub. It can block: call it off the UI thread.
        pub fn dispatch(hub: &Hub, name: &str, args: &Value) -> Res<Value> {
            match name {
                $(
                    stringify!($name) => {
                        $( let $a: $ty = arg(args, stringify!($a))?; )*
                        let $h: &Hub = hub;
                        let out: Res<$ret> = $body;
                        out.and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
                    }
                )*
                other => Err(format!("unknown command {other}")),
            }
        }
    };
}

commands! {
    panes() -> Value => |h| Ok(json!(h.views()));
    pane_open(req: OpenReq) -> Value => |h| h.open(req).map(|v| json!(v));
    pane_expand(id: String) -> String => |h| h.expand(&id).map(|s| b64(&s));
    pane_collapse() -> () => |h| { h.collapse(); Ok(()) };
    pane_resize(cols: u16, rows: u16) -> () => |h| h.resize(cols, rows);
    pane_attach(id: String, owner: String, cols: u16, rows: u16) -> String => |h| h.attach(&id, &owner, cols, rows).map(|s| b64(&s));
    pane_detach(id: String) -> () => |h| { h.detach(&id); Ok(()) };
    part_resize(id: String, cols: u16, rows: u16) -> () => |h| h.part_resize(&id, cols, rows);
    shell_open(tree: String) -> String => |h| h.shell_open(std::path::Path::new(&tree));
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
        Ok(json!({ "leader": c.leader, "nvim": crate::nvim::installed(), "sprint_days": c.tasks.sprint_days }))
    };
    reviews() -> Value => |h| Ok(json!(h.reviews()));
    review_open(url: String, round_of: Option<String>) -> String => |h| h.review_open(&url, round_of.as_deref());
    review_prior(url: String) -> Value => |h| h.review_prior(&url).map(|v| json!(v));
    review_open_local(pane: String) -> String => |h| h.review_open_local(&pane);
    review_chat(id: String) -> Value => |h| h.review_chat(&id).map(|v| json!(v));
    review_step_seen(id: String, step: String) -> () => |h| h.review_step_seen(&id, &step);
    review_dismiss(id: String, note: String, on: bool) -> () => |h| h.review_dismiss(&id, &note, on);
    review_retake(id: String, step: Option<String>) -> () => |h| h.review_retake(&id, step.as_deref());
    review_send_agent(id: String, note: String) -> usize => |h| h.review_send_agent(&id, &note);
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
    review_comment(id: String, comment: NewComment) -> () => |h| h.review_comment(&id, comment);
    review_summary(id: String, text: String) -> () => |h| h.review_summary(&id, &text);
    review_post_preview(id: String) -> Value => |h| h.review_post_preview(&id).map(|v| json!(v));
    review_post(id: String, event: String, summary: String, token: String) -> String => |h| h.review_post(&id, &event, &summary, &token);
    review_update(id: String) -> () => |h| h.review_update(&id);
    review_round(id: String) -> String => |h| h.review_round(&id);
    review_term(id: String, step: Option<String>, start: bool) -> Option<String> => |h| h.review_term(&id, step.as_deref(), start);
    review_step_note(id: String, step: String) -> String => |h| h.review_step_note(&id, &step);
    review_retry(id: String) -> () => |h| h.review_retry(&id);
    review_restart(id: String) -> () => |h| h.review_restart(&id);
    review_close(id: String, keep: bool, discard: Option<Vec<String>>) -> Value => |h| h.review_close(&id, keep, discard).map(|v| json!(v));
    review_nvim(id: String, path: String, line: u32) -> String => |h| h.review_nvim(&id, &path, line);
    tasks() -> Value => |h| h.tasks().map(|v| json!(v));
    task_add(task: NewTask) -> Value => |h| h.task_add(task).map(|v| json!(v));
    task_edit(id: String, fields: TaskFields) -> Value => |h| h.task_edit(&id, fields).map(|v| json!(v));
    task_status(id: String, status: String, blocked_by: Option<String>) -> Value => |h| h.task_status(&id, &status, blocked_by.as_deref()).map(|v| json!(v));
    task_move(id: String, to: TaskMove) -> Value => |h| h.task_move(&id, to).map(|v| json!(v));
    task_delete(id: String) -> () => |h| h.task_delete(&id);
    task_copy(id: String) -> Value => |h| h.task_copy(&id).map(|v| json!(v));
    task_restore(id: String) -> Value => |h| h.task_restore(&id).map(|v| json!(v));
    task_history(id: String) -> Value => |h| h.task_history(&id).map(|v| json!(v));
    sprint_new(sprint: NewSprint) -> Value => |h| h.sprint_new(sprint).map(|v| json!(v));
    sprint_edit(id: String, sprint: NewSprint) -> Value => |h| h.sprint_edit(&id, sprint).map(|v| json!(v));
    task_start_plan(id: String) -> Value => |h| h.task_start_plan(&id).map(|v| json!(v));
    task_link(task: String, pane: String) -> () => |h| h.task_link(&task, &pane);
    task_unlink(pane: String) -> () => |h| h.task_unlink(&pane);
    task_resume(pane: String) -> String => |h| h.task_resume(&pane);
    sprint_close(id: String, places: Vec<(String, Place)>, next: Option<NewSprint>) -> Value => |h| h.sprint_close(&id, places, next).map(|v| json!(v));
    pane_input(id: String, data: String, binary: bool) -> () => |h| {
        // `binary` data (mouse reports) is one byte per char.
        let bytes: Vec<u8> = if binary { data.chars().map(|c| c as u8).collect() } else { data.into_bytes() };
        h.input(&id, &bytes)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_names() {
        assert_eq!(camel("round_of"), "roundOf");
        assert_eq!(camel("id"), "id");
        assert_eq!(camel("blocked_by"), "blockedBy");
    }

    #[test]
    fn arg_takes_both_names_and_null() {
        let a = json!({"roundOf": "r1", "blocked_by": "x"});
        assert_eq!(arg::<Option<String>>(&a, "round_of").unwrap().as_deref(), Some("r1"));
        assert_eq!(arg::<Option<String>>(&a, "blocked_by").unwrap().as_deref(), Some("x"));
        assert_eq!(arg::<Option<String>>(&a, "missing").unwrap(), None);
        assert!(arg::<String>(&a, "missing").is_err());
    }

    #[test]
    fn every_command_once() {
        let mut names: Vec<&str> = COMMANDS.to_vec();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(names.len(), n);
        assert!(COMMANDS.contains(&"pane_input"));
    }
}
