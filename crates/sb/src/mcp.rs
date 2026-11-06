//! `sb mcp`: the MCP server with the guide tools (SPEC 11.4), over stdio.
//!
//! Each guide call goes to the app over the socket. The app checks the guide
//! and keeps it. The agent never writes a file.
//!
//! The `git` tool runs here, in the review worktree (`SB_DIR`). It takes the place
//! of Bash for the guide agent: see `switchboard_core::review_git`.
//!
//! `sb mcp --ask` is the server of the review's Ask pane (SPEC 11.15). It has only
//! `review_show`, which moves the review window to code. It has no guide tool and no
//! `git`, and it refuses them by name too.

use crate::send;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use switchboard_core::proto::{Ack, Request};
use switchboard_core::review_git;

pub fn tools() -> Value {
    let range = json!({
        "type": "object",
        "properties": {
            "file": { "type": "string", "description": "Path in that version of the file." },
            "side": { "type": "string", "enum": ["new", "old"], "description": "new: lines at the PR head. old: removed lines at the base." },
            "from": { "type": "integer" },
            "to": { "type": "integer" }
        },
        "required": ["file", "side", "from", "to"]
    });
    let note = json!({
        "type": "object",
        "properties": {
            "severity": { "type": "string", "enum": ["must", "should", "nit"] },
            "text": { "type": "string" },
            "file": { "type": "string" },
            "side": { "type": "string", "enum": ["new", "old"] },
            "line": { "type": "integer", "description": "A line of a changed file on that side. Leave out for a note with no line." }
        },
        "required": ["severity", "text"]
    });
    let take = json!({
        "type": "object",
        "description": "Your opinion. The reviewer sees it marked as not verified.",
        "properties": {
            "verdict": { "type": "string", "enum": ["good", "change", "question"] },
            "checked": { "type": "string", "description": "What you checked. Required for good." },
            "summary": { "type": "string" },
            "notes": { "type": "array", "items": note }
        },
        "required": ["verdict"]
    });
    let step = json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "title": { "type": "string" },
            "what": { "type": "string" },
            "why": { "type": "string", "description": "Why this step matters." },
            "why_split": { "type": "string", "description": "Why it is a step of its own." },
            "check": { "type": "array", "items": { "type": "string" } },
            "check_answers": { "type": "array", "items": { "type": "string" }, "description": "Your short answer to each check item, in order." },
            "take": take.clone(),
            "ranges": { "type": "array", "items": range, "description": "Every changed line of the PR must be in some range." },
            "files": { "type": "array", "items": { "type": "string" }, "description": "Binary files this step covers." },
            "context": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["id", "title"]
    });
    json!([
        {
            "name": "guide_set_steps",
            "description": "Set the full review guide. A verified checker refuses a guide that misses a changed line; the error lists what to fix.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pr": { "type": "object" },
                    "context": { "type": "array", "items": { "type": "object" } },
                    "take": take,
                    "steps": { "type": "array", "items": step }
                },
                "required": ["steps"]
            }
        },
        {
            "name": "guide_update_step",
            "description": "Change named fields of one step. The whole guide must still pass the checker. Pins and threads stay.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" }, "fields": { "type": "object" } },
                "required": ["id", "fields"]
            }
        },
        {
            "name": "git",
            "description": "Run one read-only git command in the review worktree. No shell: each arg is one item, with no quotes. Args are revs, ranges, paths in the tree, and read options such as -U0, --stat, --name-only, --format=..., -n20, -L10,20:file. An option that writes a file or runs a program is refused. The output is cut at 256 KB.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "subcommand": { "type": "string", "enum": review_git::SUBCOMMANDS },
                    "args": { "type": "array", "items": { "type": "string" }, "description": "For example [\"-U0\", \"abc123\", \"HEAD\", \"--\", \"src/lib.rs\"]." }
                },
                "required": ["subcommand"]
            }
        }
    ])
}

/// The one tool of `sb mcp --ask`.
pub fn ask_tools() -> Value {
    json!([{
        "name": "review_show",
        "description": "Show places in the code to the reviewer: the review window jumps to the first one and the reviewer steps through them with n and p. Use it when asked where something is, what to fix, or what to look at, worst first. It only moves the view; it changes no file and posts nothing. Paths are from the repo root, lines at the reviewed head (side new) or at the base for removed lines (side old).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "points": {
                    "type": "array",
                    "maxItems": switchboard_core::hub_show::MAX_POINTS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "file": { "type": "string", "description": "Path from the repo root, for example beacon_node/store/src/hot_cold_store.rs." },
                            "line": { "type": "integer", "minimum": 1 },
                            "end": { "type": "integer", "description": "The last line of a range." },
                            "side": { "type": "string", "enum": ["new", "old"], "description": "new (the default): the reviewed head. old: a removed line at the base." },
                            "kind": { "type": "string", "enum": ["must", "should", "nit", "question"] },
                            "note": { "type": "string", "description": "One or two short sentences: what is here and why it matters." }
                        },
                        "required": ["file", "line", "note"]
                    }
                }
            },
            "required": ["points"]
        }
    }])
}

/// Handles one JSON-RPC message. `None` for a notification. With `ask`, only the
/// tool of the Ask pane.
pub fn handle(
    msg: &Value,
    review: &str,
    dir: &Path,
    ask: bool,
    call: &dyn Fn(Request) -> Result<Ack, String>,
) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let result = match method {
        "initialize" => {
            let v = msg
                .pointer("/params/protocolVersion")
                .cloned()
                .unwrap_or(json!("2025-06-18"));
            json!({ "protocolVersion": v, "capabilities": { "tools": {} }, "serverInfo": { "name": "sb", "version": env!("CARGO_PKG_VERSION") } })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": if ask { ask_tools() } else { tools() } }),
        "tools/call" => {
            let name = msg
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = msg
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
            if name == "git" && !ask {
                return Some(json!({ "jsonrpc": "2.0", "id": id, "result": git_call(dir, &args) }));
            }
            let req = match name {
                "review_show" if ask => Some(Request::ReviewShow {
                    review: review.into(),
                    points: args.get("points").cloned().unwrap_or(Value::Null),
                }),
                _ if ask => None,
                "guide_set_steps" => Some(Request::GuideSet {
                    review: review.into(),
                    guide: args,
                }),
                "guide_update_step" => Some(Request::GuideUpdate {
                    review: review.into(),
                    step: args.get("id").and_then(Value::as_str).unwrap_or("").into(),
                    fields: args.get("fields").cloned().unwrap_or(json!({})),
                }),
                _ => None,
            };
            let (text, error) = match req.map(call) {
                Some(Ok(Ack { ok: true, .. })) if ask => ("The review window shows the places. Now answer in text too.".to_owned(), false),
                Some(Ok(Ack { ok: true, .. })) => ("ok".to_owned(), false),
                Some(Ok(Ack { error, .. })) => (error.unwrap_or_else(|| "refused".into()), true),
                Some(Err(e)) => (format!("Switchboard is not reachable: {e}"), true),
                None => (format!("unknown tool {name}"), true),
            };
            json!({ "content": [{ "type": "text", "text": text }], "isError": error })
        }
        _ => {
            return Some(
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("unknown method {method}") } }),
            )
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// Runs the `git` tool. A bad call is a tool error, so the agent can try again.
fn git_call(dir: &Path, args: &Value) -> Value {
    let sub = args.get("subcommand").and_then(Value::as_str).unwrap_or("");
    let list: Option<Vec<String>> = match args.get("args") {
        None | Some(Value::Null) => Some(vec![]),
        Some(Value::Array(a)) => a.iter().map(|x| x.as_str().map(String::from)).collect(),
        Some(_) => None,
    };
    let (text, error) = match list {
        None => ("args must be a list of strings".to_owned(), true),
        Some(list) => match review_git::run(dir, sub, &list) {
            Ok(t) if t.is_empty() => ("(no output)".to_owned(), false),
            Ok(t) => (t, false),
            Err(e) => (e, true),
        },
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": error })
}

pub fn serve(ask: bool) -> i32 {
    let review = std::env::var("SB_REVIEW").unwrap_or_default();
    // The review worktree. Claude starts the server there too, so the current
    // folder is the fallback.
    let dir = std::env::var_os("SB_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let call = |req: Request| -> Result<Ack, String> {
        let line = send(&req, true)?.unwrap_or_default();
        serde_json::from_str(&line).map_err(|e| e.to_string())
    };
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(reply) = handle(&msg, &review, &dir, ask, &call) {
            let _ = writeln!(out, "{reply}");
            let _ = out.flush();
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(_: Request) -> Result<Ack, String> {
        Ok(Ack::ok())
    }

    #[test]
    fn initialize_echoes_the_version() {
        let r = handle(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}), "r1", Path::new("."), false, &ok).unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert!(r["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn notifications_get_no_reply() {
        assert!(handle(
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            "r1",
            Path::new("."),
            false,
            &ok
        )
        .is_none());
    }

    #[test]
    fn list_has_the_tools() {
        let r = handle(
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            "r1",
            Path::new("."),
            false,
            &ok,
        )
        .unwrap();
        let names: Vec<&str> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["guide_set_steps", "guide_update_step", "git"]);
    }

    #[test]
    fn calls_go_to_the_app_with_the_review_id() {
        let seen = std::cell::RefCell::new(None);
        let call = |req: Request| {
            *seen.borrow_mut() = Some(req);
            Ok(Ack::err("step s1: line 9 is outside a.rs"))
        };
        let r = handle(
            &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"guide_update_step","arguments":{"id":"s1","fields":{"lines":[9,9]}}}}),
            "r7",
            Path::new("."),
            false,
            &call,
        )
        .unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("outside"));
        match seen.into_inner().unwrap() {
            Request::GuideUpdate { review, step, .. } => {
                assert_eq!((review.as_str(), step.as_str()), ("r7", "s1"))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn git_runs_here_and_refuses_a_write() {
        let sent = std::cell::Cell::new(false);
        let call = |_: Request| {
            sent.set(true);
            Ok(Ack::ok())
        };
        let r = handle(
            &json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"git","arguments":{"subcommand":"diff","args":["--output=x.txt","HEAD"]}}}),
            "r1",
            Path::new("."),
            false,
            &call,
        )
        .unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("not allowed"));
        assert!(!Path::new("x.txt").exists());
        let r = handle(
            &json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"git","arguments":{"subcommand":"rev-parse","args":["--verify","HEAD"]}}}),
            "r1",
            Path::new(env!("CARGO_MANIFEST_DIR")),
            false,
            &call,
        )
        .unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        assert_eq!(r["result"]["content"][0]["text"].as_str().unwrap().trim().len(), 40);
        let r = handle(
            &json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"git","arguments":{"subcommand":"push","args":"x"}}}),
            "r1",
            Path::new("."),
            false,
            &call,
        )
        .unwrap();
        assert_eq!(r["result"]["isError"], true);
        // The git tool never goes to the app.
        assert!(!sent.get());
    }

    #[test]
    fn ask_mode_has_only_review_show() {
        let sent = std::cell::RefCell::new(vec![]);
        let call = |req: Request| {
            sent.borrow_mut().push(req);
            Ok(Ack::ok())
        };
        let msg = |id: u32, name: &str, args: Value| json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}});
        let r = handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}), "r3", Path::new("."), true, &call).unwrap();
        let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["review_show"]);
        // The guide tools and git are refused by name, and nothing goes to the app.
        for (name, args) in [
            ("guide_set_steps", json!({"steps": []})),
            ("guide_update_step", json!({"id": "s1", "fields": {}})),
            ("git", json!({"subcommand": "rev-parse", "args": ["HEAD"]})),
        ] {
            let r = handle(&msg(2, name, args), "r3", Path::new(env!("CARGO_MANIFEST_DIR")), true, &call).unwrap();
            assert_eq!(r["result"]["isError"], true, "{name}");
        }
        assert!(sent.borrow().is_empty());
        let points = json!([{"file": "a.rs", "line": 3, "note": "here"}]);
        let r = handle(&msg(3, "review_show", json!({ "points": points })), "r3", Path::new("."), true, &call).unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        match &sent.borrow()[0] {
            Request::ReviewShow { review, points: p } => assert_eq!((review.as_str(), p), ("r3", &points)),
            other => panic!("{other:?}"),
        }
        // Without --ask, there is no review_show.
        let r = handle(&msg(4, "review_show", json!({ "points": points })), "r3", Path::new("."), false, &call).unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(sent.borrow().len(), 1);
        // A refusal from the app goes back as a tool error with its reason.
        let no = |_: Request| Ok(Ack::err("point 1: ../x leaves the review"));
        let r = handle(&msg(5, "review_show", json!({ "points": [{"file": "../x", "line": 1}] })), "r3", Path::new("."), true, &no).unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("leaves the review"));
    }

    #[test]
    fn unknown_method_is_an_error() {
        let r = handle(
            &json!({"jsonrpc":"2.0","id":4,"method":"resources/list"}),
            "r1",
            Path::new("."),
            false,
            &ok,
        )
        .unwrap();
        assert_eq!(r["error"]["code"], -32601);
    }
}
