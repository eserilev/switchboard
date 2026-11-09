//! The Claude Code hooks that Switchboard installs for each pane (SPEC 13.2),
//! and how `sb state` turns a hook input into a state message.

use crate::proto::{first_line, StateMsg, VERSION};
use serde_json::{json, Value};
use std::path::Path;

pub const RUST_PLUGIN: &str = "rust-analyzer-lsp@claude-plugins-official";

/// The folders with keys and logins. A pane in the sandbox can not read or write
/// them, with Bash or with the Read and Edit tools (SPEC 31).
pub const SECRET_DIRS: [&str; 4] = ["~/.ssh", "~/.aws", "~/.gnupg", "~/.config/gh"];

/// What goes in the settings file of one pane.
pub struct PaneSettings<'a> {
    /// The `sb` program that the hooks run.
    pub sb: &'a Path,
    /// False for repos with no `Cargo.toml`: then the rust-analyzer plugin is off.
    pub rust: bool,
    /// Tool rules that run with no prompt, such as `WebFetch(domain:x)`.
    pub allow: &'a [String],
    /// With none, Bash runs outside Claude Code's sandbox.
    pub sandbox: Option<Sandbox<'a>>,
}

/// The sandbox of one pane (SPEC 31).
pub struct Sandbox<'a> {
    /// The hosts that commands can reach.
    pub domains: &'a [String],
    /// Commands that run outside the sandbox.
    pub unsandboxed: &'a [String],
    /// Full paths that commands can write, beside the tree of the pane.
    pub write: Vec<String>,
    /// Full paths under `write` that commands can not write, such as `.git/hooks`.
    pub deny_write: Vec<String>,
}

/// One word for `sh`: in single quotes, with each `'` as `'\''`.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The settings file for one pane, passed with `claude --settings`.
pub fn settings(p: &PaneSettings) -> Value {
    let sb = sh_quote(&p.sb.to_string_lossy());
    let cmd = |arg: &str| json!([{ "hooks": [{ "type": "command", "command": format!("{sb} {arg}") }] }]);
    let with = |matcher: &str, arg: &str, timeout: Option<u64>| {
        let mut h = json!({ "type": "command", "command": format!("{sb} {arg}") });
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
    if !p.allow.is_empty() {
        s["permissions"] = json!({ "allow": p.allow });
    }
    if !p.rust {
        s["enabledPlugins"] = json!({ RUST_PLUGIN: false });
    }
    if let Some(b) = &p.sandbox {
        s["sandbox"] = json!({
            "enabled": true,
            // With no sandbox on this computer, claude stops at start, not runs open.
            "failIfUnavailable": true,
            // A command that fails in the sandbox can not try again outside it.
            "allowUnsandboxedCommands": false,
            "excludedCommands": b.unsandboxed,
            "filesystem": {
                "allowWrite": b.write,
                "denyWrite": b.deny_write,
                "denyRead": SECRET_DIRS,
            },
            "network": { "allowedDomains": b.domains },
        });
        // The sandbox covers only Bash: these rules keep the Read and Edit tools out too.
        let deny: Vec<String> = SECRET_DIRS
            .iter()
            .flat_map(|d| [format!("Read({d}/**)"), format!("Edit({d}/**)")])
            .collect();
        if !s["permissions"].is_object() {
            s["permissions"] = json!({});
        }
        s["permissions"]["deny"] = json!(deny);
    }
    s
}

/// Builds the state message for a hook event. `None` means the event changes nothing.
pub fn state_msg(pane: &str, arg: &str, input: &Value, time: u64) -> Option<StateMsg> {
    let s = |k: &str| input.get(k).and_then(Value::as_str);
    let (event, summary, detail) = match arg {
        "session" | "working" => (arg, None, Value::Null),
        // The whole answer goes too: the hub finds what it points to (SPEC 11.15).
        "turn" => (
            "turn",
            s("last_assistant_message").map(|m| first_line(m, 120)),
            s("last_assistant_message").map_or(Value::Null, |m| json!({ "text": cut(m, crate::mentions::MAX_TEXT) })),
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

/// The first `max` bytes of a text, on a char boundary.
fn cut(text: &str, max: usize) -> &str {
    let mut n = text.len().min(max);
    while !text.is_char_boundary(n) {
        n -= 1;
    }
    &text[..n]
}

/// The text of the last assistant message in a Claude session file (JSON lines).
/// For a Stop hook input with no `last_assistant_message`. Only the last part of a
/// long file is read.
pub fn last_assistant_text(jsonl: &str) -> Option<String> {
    for line in jsonl.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let content = v.pointer("/message/content");
        let text = match content {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(parts)) => parts
                .iter()
                .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        if !text.trim().is_empty() {
            return Some(text);
        }
    }
    None
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

    fn settings(sb: &Path, rust: bool, allow: &[String]) -> Value {
        super::settings(&PaneSettings { sb, rust, allow, sandbox: None })
    }

    #[test]
    fn sandbox_block_only_when_asked() {
        let s = settings(Path::new("/sb"), true, &[]);
        assert!(s.get("sandbox").is_none());
        let domains = ["github.com".to_owned(), "*.npmjs.org".to_owned()];
        let out = ["gh *".to_owned()];
        let allow = ["WebFetch(domain:github.com)".to_owned()];
        let s = super::settings(&PaneSettings {
            sb: Path::new("/sb"),
            rust: true,
            allow: &allow,
            sandbox: Some(Sandbox {
                domains: &domains,
                unsandboxed: &out,
                write: vec!["/c/repo/.git".into()],
                deny_write: vec!["/c/repo/.git/hooks".into()],
            }),
        });
        let b = &s["sandbox"];
        assert_eq!((&b["enabled"], &b["failIfUnavailable"], &b["allowUnsandboxedCommands"]), (&json!(true), &json!(true), &json!(false)));
        assert_eq!(b["network"]["allowedDomains"], json!(domains));
        assert_eq!(b["excludedCommands"], json!(out));
        assert_eq!(b["filesystem"]["allowWrite"], json!(["/c/repo/.git"]));
        assert_eq!(b["filesystem"]["denyWrite"], json!(["/c/repo/.git/hooks"]));
        assert_eq!(b["filesystem"]["denyRead"], json!(SECRET_DIRS));
        // The allow rules stay, and the Read tool can not read the keys either.
        assert_eq!(s["permissions"]["allow"], json!(allow));
        let deny = s["permissions"]["deny"].as_array().unwrap();
        assert!(deny.contains(&json!("Read(~/.ssh/**)")) && deny.contains(&json!("Edit(~/.aws/**)")), "{deny:?}");
        // The hooks stay: they run outside the sandbox.
        assert!(s["hooks"]["PermissionRequest"].is_array());
    }

    #[test]
    fn settings_have_every_hook_and_quote_the_path() {
        let s = settings(Path::new("/opt/my sb/sb"), true, &[]);
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
        assert!(s.get("permissions").is_none());
        let allow = ["WebFetch(domain:github.com)".to_owned()];
        assert_eq!(
            settings(Path::new("/sb"), true, &allow)["permissions"]["allow"],
            json!(allow)
        );
        assert_eq!(
            settings(Path::new("/sb"), false, &[])["enabledPlugins"][RUST_PLUGIN],
            false
        );
    }

    #[test]
    fn a_quote_in_the_sb_path_stays_in_the_word() {
        let path = "/Users/o'neil/My Apps/sb";
        let s = settings(Path::new(path), true, &[]);
        let cmd = s["hooks"]["Stop"][0]["hooks"][0]["command"].as_str().unwrap();
        assert_eq!(cmd, r"'/Users/o'\''neil/My Apps/sb' state turn");
        // The shell splits it into the path and the two arguments.
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("printf '%s|' {}", cmd))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), format!("{path}|state|turn|"));
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
        assert_eq!(m.detail["text"], "\nMoved links.\nMore");
        let long = "é".repeat(crate::mentions::MAX_TEXT);
        let m = state_msg("p1", "turn", &json!({"last_assistant_message": long}), 9).unwrap();
        assert!(m.detail["text"].as_str().unwrap().len() <= crate::mentions::MAX_TEXT);
        assert_eq!(state_msg("p1", "turn", &json!({}), 9).unwrap().detail, Value::Null);
    }

    #[test]
    fn the_last_assistant_text_of_a_session_file() {
        let file = [
            json!({"type":"user","message":{"content":"where is it?"}}),
            json!({"type":"assistant","message":{"content":[{"type":"text","text":"See a.rs:3."},{"type":"tool_use","name":"x"}]}}),
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"y"}]}}),
            json!({"type":"system"}),
        ]
        .map(|v| v.to_string())
        .join("\n");
        assert_eq!(last_assistant_text(&format!("{file}\nnot json")).as_deref(), Some("See a.rs:3."));
        assert_eq!(last_assistant_text(""), None);
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
