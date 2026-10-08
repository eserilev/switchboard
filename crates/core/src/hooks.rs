//! The Claude Code hooks that Switchboard installs for each pane (SPEC 13.2),
//! and how `sb state` turns a hook input into a state message.

use crate::proto::{first_line, StateMsg, VERSION};
use serde_json::{json, Value};
use std::path::Path;

pub const RUST_PLUGIN: &str = "rust-analyzer-lsp@claude-plugins-official";

/// The settings file for one pane, passed with `claude --settings`.
/// `rust` is false for repos with no `Cargo.toml`: then the rust-analyzer plugin is off.
pub fn settings(sb: &Path, rust: bool) -> Value {
    let sb = sb.to_string_lossy();
    let cmd = |arg: &str| json!([{ "hooks": [{ "type": "command", "command": format!("'{sb}' {arg}") }] }]);
    let with = |matcher: &str, arg: &str, timeout: Option<u64>| {
        let mut h = json!({ "type": "command", "command": format!("'{sb}' {arg}") });
        if let Some(t) = timeout {
            h["timeout"] = json!(t);
        }
        json!([{ "matcher": matcher, "hooks": [h] }])
    };
    let mut s = json!({
        "hooks": {
            "SessionStart": cmd("state session"),
            "UserPromptSubmit": cmd("state working"),
            "PreToolUse": with("*", "state working", None),
            "PermissionRequest": with("*", "permit", Some(600)),
            "Notification": cmd("state notify"),
            "PostToolUse": with("Edit|Write|MultiEdit|NotebookEdit", "state edited", None),
            "Stop": cmd("state turn"),
            "StopFailure": cmd("state failure"),
        }
    });
    if !rust {
        s["enabledPlugins"] = json!({ RUST_PLUGIN: false });
    }
    s
}

/// Builds the state message for a hook event. `None` means the event changes nothing.
pub fn state_msg(pane: &str, arg: &str, input: &Value, time: u64) -> Option<StateMsg> {
    let s = |k: &str| input.get(k).and_then(Value::as_str);
    let (event, summary, detail) = match arg {
        "session" | "working" => (arg, None, Value::Null),
        "turn" => (
            "turn",
            s("last_assistant_message").map(|m| first_line(m, 120)),
            Value::Null,
        ),
        "edited" => {
            let file = input
                .pointer("/tool_input/file_path")
                .or_else(|| input.pointer("/tool_input/notebook_path"));
            ("edited", None, json!({ "file": file }))
        }
        "notify" => match s("notification_type")? {
            "permission_prompt"
            | "elicitation_dialog"
            | "elicitation_url_dialog"
            | "agent_needs_input" => (
                "needs",
                s("message").map(|m| first_line(m, 120)),
                Value::Null,
            ),
            "quota_auto_resume_fired" => ("working", None, Value::Null),
            "quota_auto_resume_disabled" | "quota_auto_resume_stale" => (
                "limit",
                s("message").map(|m| first_line(m, 120)),
                Value::Null,
            ),
            _ => return None,
        },
        "failure" => {
            let event = if s("error") == Some("rate_limit") {
                "limit"
            } else {
                "error"
            };
            let msg = s("last_assistant_message")
                .or(s("error_details"))
                .or(s("error"));
            (
                event,
                msg.map(|m| first_line(m, 120)),
                json!({ "error": s("error"), "details": s("error_details") }),
            )
        }
        _ => return None,
    };
    Some(StateMsg {
        v: VERSION,
        pane: pane.to_owned(),
        event: event.to_owned(),
        session: s("session_id").map(str::to_owned),
        cwd: s("cwd").map(str::to_owned),
        time,
        summary,
        detail,
    })
}

/// The summary for a permission request, for example `Bash: cargo test`.
pub fn permit_summary(tool: &str, input: &Value) -> String {
    let arg = ["command", "file_path", "url", "pattern", "path"]
        .iter()
        .find_map(|k| input.get(k).and_then(Value::as_str))
        .unwrap_or("");
    first_line(&format!("{tool}: {arg}"), 120)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_have_every_hook_and_quote_the_path() {
        let s = settings(Path::new("/opt/my sb/sb"), true);
        for h in [
            "SessionStart",
            "UserPromptSubmit",
            "PreToolUse",
            "PermissionRequest",
            "Notification",
            "PostToolUse",
            "Stop",
            "StopFailure",
        ] {
            assert!(s["hooks"][h].is_array(), "{h}");
        }
        assert_eq!(
            s["hooks"]["Stop"][0]["hooks"][0]["command"],
            "'/opt/my sb/sb' state turn"
        );
        assert_eq!(
            s["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"],
            600
        );
        assert!(s.get("enabledPlugins").is_none());
        assert_eq!(
            settings(Path::new("/sb"), false)["enabledPlugins"][RUST_PLUGIN],
            false
        );
    }

    #[test]
    fn stop_uses_last_assistant_message() {
        let m = state_msg(
            "p1",
            "turn",
            &json!({"session_id":"s1","last_assistant_message":"\nMoved links.\nMore"}),
            9,
        )
        .unwrap();
        assert_eq!(
            (m.event.as_str(), m.summary.as_deref(), m.session.as_deref()),
            ("turn", Some("Moved links."), Some("s1"))
        );
    }

    #[test]
    fn notifications_map_by_type() {
        let ev = |t: &str| {
            state_msg(
                "p",
                "notify",
                &json!({"notification_type": t, "message": "m"}),
                0,
            )
            .map(|m| m.event)
        };
        assert_eq!(ev("permission_prompt").as_deref(), Some("needs"));
        assert_eq!(ev("elicitation_dialog").as_deref(), Some("needs"));
        assert_eq!(ev("quota_auto_resume_fired").as_deref(), Some("working"));
        assert_eq!(ev("quota_auto_resume_disabled").as_deref(), Some("limit"));
        assert_eq!(ev("idle_prompt"), None);
        assert_eq!(state_msg("p", "notify", &json!({}), 0), None);
    }

    #[test]
    fn failure_rate_limit_is_limit_and_others_are_error() {
        let m = state_msg(
            "p",
            "failure",
            &json!({"error":"rate_limit","last_assistant_message":"API Error: Rate limit reached"}),
            0,
        )
        .unwrap();
        assert_eq!(
            (m.event.as_str(), m.summary.as_deref()),
            ("limit", Some("API Error: Rate limit reached"))
        );
        assert_eq!(
            state_msg("p", "failure", &json!({"error":"overloaded"}), 0)
                .unwrap()
                .event,
            "error"
        );
    }

    #[test]
    fn edited_carries_the_file() {
        let m = state_msg(
            "p",
            "edited",
            &json!({"tool_input":{"file_path":"/a/b.rs"}}),
            0,
        )
        .unwrap();
        assert_eq!(m.detail["file"], "/a/b.rs");
    }

    #[test]
    fn permit_summary_picks_the_main_argument() {
        assert_eq!(
            permit_summary("Bash", &json!({"command":"cargo test","description":"x"})),
            "Bash: cargo test"
        );
        assert_eq!(
            permit_summary("Edit", &json!({"file_path":"/a.rs"})),
            "Edit: /a.rs"
        );
    }
}
