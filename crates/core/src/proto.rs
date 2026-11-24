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
    /// `review_show` of the Ask pane (SPEC 11.15): places for the review window.
    /// It only moves the view.
    ReviewShow {
        review: String,
        points: Value,
    },
    /// `sb resume <pane>`: starts a Claude pane again with its session, also one
    /// that runs. Its conversation stays; its tools, env and settings are new.
    Resume {
        pane: String,
    },
    /// `review_work` of the Ask pane (SPEC 11.16): a new agent on the PR's branch.
    ReviewWork {
        review: String,
        task: String,
    },
    /// `sb task` (SPEC 27.7). `pane` is the caller's `SB_PANE`: a label, not proof.
    Task {
        #[serde(default)]
        pane: Option<String>,
        op: crate::tasks::TaskOp,
    },
    /// `board_list` of the pane tools (SPEC 36): the panes in the caller's repo.
    /// Names, branches and states only: no text the agents wrote.
    Board {
        #[serde(default)]
        pane: Option<String>,
    },
    /// `sb today` and the Today pane tools (SPEC 39.8): the list of a job, or one
    /// item with `repo` and `number`. With a pane and no job, the job of its repo.
    Today {
        #[serde(default)]
        pane: Option<String>,
        #[serde(default)]
        job: Option<String>,
        #[serde(default)]
        repo: Option<String>,
        #[serde(default)]
        number: Option<u64>,
    },
    /// Messages between agents (SPEC 40).
    Mail(MailOp),
}

/// One mail request. `pane` is the caller's `SB_PANE`: a label, not proof.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MailOp {
    /// A message from a pane of this hub (`pane`), or from another machine
    /// (`from` and `label`, through `sb msg send` over ssh).
    Send {
        #[serde(default)]
        pane: Option<String>,
        #[serde(default)]
        from: Option<String>,
        #[serde(default)]
        label: Option<String>,
        to: String,
        text: String,
    },
    /// The agents that can get messages, on each machine.
    Agents {
        #[serde(default)]
        pane: Option<String>,
    },
    /// The Stop hook of a pane: the messages that wait for it, as one text.
    Take { pane: String },
    /// The link of another machine's hub: the connection stays open (SPEC 40.5).
    Watch { machine: String },
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
