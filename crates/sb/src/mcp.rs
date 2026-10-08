//! `sb mcp`: the MCP server with the guide tools (SPEC 11.4), over stdio.
//!
//! Each tool call goes to the app over the socket. The app checks the guide
//! and keeps it. The agent never writes a file.

use crate::send;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use switchboard_core::proto::{Ack, Request};

pub fn tools() -> Value {
    let step = json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "title": { "type": "string" },
            "file": { "type": ["string", "null"], "description": "Path at the PR head, or null for a step with no code." },
            "side": { "type": "string", "enum": ["new", "old"] },
            "lines": { "type": "array", "items": { "type": "integer" }, "description": "[first, last]" },
            "what": { "type": "string" },
            "check": { "type": "array", "items": { "type": "string" } },
            "context": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["id", "title"]
    });
    json!([
        {
            "name": "guide_set_steps",
            "description": "Set the full review guide. Call it once with every step.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pr": { "type": "object" },
                    "context": { "type": "array", "items": { "type": "object" } },
                    "steps": { "type": "array", "items": step }
                },
                "required": ["steps"]
            }
        },
        {
            "name": "guide_update_step",
            "description": "Change named fields of one step. Pins and threads stay.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" }, "fields": { "type": "object" } },
                "required": ["id", "fields"]
            }
        }
    ])
}

/// Handles one JSON-RPC message. `None` for a notification.
pub fn handle(
    msg: &Value,
    review: &str,
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
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => {
            let name = msg
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let args = msg
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
            let req = match name {
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

pub fn serve() -> i32 {
    let review = std::env::var("SB_REVIEW").unwrap_or_default();
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
        if let Some(reply) = handle(&msg, &review, &call) {
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
        let r = handle(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}), "r1", &ok).unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert!(r["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn notifications_get_no_reply() {
        assert!(handle(
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            "r1",
            &ok
        )
        .is_none());
    }

    #[test]
    fn list_has_both_tools() {
        let r = handle(
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            "r1",
            &ok,
        )
        .unwrap();
        let names: Vec<&str> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["guide_set_steps", "guide_update_step"]);
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
    fn unknown_method_is_an_error() {
        let r = handle(
            &json!({"jsonrpc":"2.0","id":4,"method":"resources/list"}),
            "r1",
            &ok,
        )
        .unwrap();
        assert_eq!(r["error"]["code"], -32601);
    }
}
