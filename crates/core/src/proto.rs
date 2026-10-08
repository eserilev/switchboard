//! Messages on the Switchboard socket. One JSON object per line.
//!
//! `sb state` sends a message and closes. `sb permit`, `sb open` and the
//! MCP tools wait for one reply line.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    State(StateMsg),
    Permit {
        pane: String,
        tool: String,
        input: Value,
    },
    Open {
        path: String,
        worktree: Option<String>,
        run: Option<String>,
    },
    GuideSet {
        review: String,
        guide: Value,
    },
    GuideUpdate {
        review: String,
        step: String,
        fields: Value,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StateMsg {
    pub v: u32,
    pub pane: String,
    /// One of `session`, `working`, `needs`, `turn`, `limit`, `error`, `edited`.
    pub event: String,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Seconds since the Unix epoch.
    pub time: u64,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub detail: Value,
}

/// The reply to `Permit`. `None` means no decision: Claude keeps its own prompt.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PermitReply {
    pub behavior: Option<String>,
}

/// The reply to `Open`, `GuideSet` and `GuideUpdate`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Ack {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
}

impl Ack {
    pub fn ok() -> Ack {
        Ack {
            ok: true,
            error: None,
        }
    }
    pub fn err(e: impl Into<String>) -> Ack {
        Ack {
            ok: false,
            error: Some(e.into()),
        }
    }
}

/// The first line of a text, cut to `max` characters.
pub fn first_line(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trip() {
        let r = Request::State(StateMsg {
            v: 1,
            pane: "p3".into(),
            event: "turn".into(),
            session: Some("s".into()),
            cwd: None,
            time: 5,
            summary: Some("done".into()),
            detail: Value::Null,
        });
        let line = serde_json::to_string(&r).unwrap();
        assert!(line.contains(r#""kind":"state""#));
        assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), r);
    }

    #[test]
    fn first_line_skips_blank_lines_and_cuts() {
        assert_eq!(first_line("\n\n  hello there\nmore", 100), "hello there");
        assert_eq!(first_line("abcdef", 3), "abc…");
        assert_eq!(first_line("", 3), "");
    }
}
