//! The lamp of a tile, and how hook events change it (SPEC 6.2, 6.3).

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Lamp {
    /// A new agent before its first prompt.
    Idle,
    Working,
    Needs,
    /// The turn ended: Claude finished or asked in plain text.
    Turn,
    Limit,
    Error,
    /// The process exited. The tile can resume it.
    Ended,
    /// nvim and shell panes. They have no lamp.
    None,
}

impl Lamp {
    pub fn as_str(self) -> &'static str {
        match self {
            Lamp::Idle => "idle",
            Lamp::Working => "working",
            Lamp::Needs => "needs",
            Lamp::Turn => "turn",
            Lamp::Limit => "limit",
            Lamp::Error => "error",
            Lamp::Ended => "ended",
            Lamp::None => "none",
        }
    }

    pub fn parse(s: &str) -> Lamp {
        match s {
            "idle" => Lamp::Idle,
            "working" => Lamp::Working,
            "needs" => Lamp::Needs,
            "turn" => Lamp::Turn,
            "limit" => Lamp::Limit,
            "error" => Lamp::Error,
            "ended" => Lamp::Ended,
            _ => Lamp::None,
        }
    }

    /// Lamps that want you. Only these set a tile to unseen.
    pub fn wants_you(self) -> bool {
        matches!(self, Lamp::Needs | Lamp::Turn | Lamp::Limit | Lamp::Error)
    }

    /// The order for "next unseen" and the tally: the most urgent first.
    pub fn urgency(self) -> u8 {
        match self {
            Lamp::Needs => 0,
            Lamp::Limit => 1,
            Lamp::Error => 2,
            Lamp::Turn => 3,
            _ => 9,
        }
    }
}

/// The new lamp for a hook event, or `None` when the event does not change it.
pub fn next(current: Lamp, event: &str) -> Option<Lamp> {
    if current == Lamp::None {
        return None;
    }
    match event {
        // SessionStart also fires on resume. It only wakes an ended tile.
        "session" => (current == Lamp::Ended).then_some(Lamp::Idle),
        "working" => Some(Lamp::Working),
        "needs" => Some(Lamp::Needs),
        "turn" => Some(Lamp::Turn),
        "limit" => Some(Lamp::Limit),
        "error" => Some(Lamp::Error),
        _ => None,
    }
}

/// Applies an event to (lamp, unseen). A lamp that wants you sets unseen,
/// also when it is the same lamp again: a second turn is news too.
pub fn apply(lamp: Lamp, unseen: bool, event: &str) -> (Lamp, bool) {
    match next(lamp, event) {
        Some(l) if l.wants_you() => (l, true),
        Some(l) => (l, unseen && lamp.wants_you() && l == lamp),
        None => (lamp, unseen),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_map_to_lamps() {
        assert_eq!(next(Lamp::Idle, "working"), Some(Lamp::Working));
        assert_eq!(next(Lamp::Working, "needs"), Some(Lamp::Needs));
        assert_eq!(next(Lamp::Needs, "working"), Some(Lamp::Working));
        assert_eq!(next(Lamp::Working, "turn"), Some(Lamp::Turn));
        assert_eq!(next(Lamp::Working, "limit"), Some(Lamp::Limit));
        assert_eq!(next(Lamp::Working, "error"), Some(Lamp::Error));
        assert_eq!(next(Lamp::Turn, "edited"), None);
    }

    #[test]
    fn session_only_wakes_an_ended_tile() {
        assert_eq!(next(Lamp::Turn, "session"), None);
        assert_eq!(next(Lamp::Ended, "session"), Some(Lamp::Idle));
    }

    #[test]
    fn nvim_and_shell_panes_never_light() {
        assert_eq!(next(Lamp::None, "turn"), None);
    }

    #[test]
    fn unseen_rules() {
        assert_eq!(apply(Lamp::Working, false, "turn"), (Lamp::Turn, true));
        assert_eq!(apply(Lamp::Turn, false, "turn"), (Lamp::Turn, true));
        // Working clears unseen: you typed, or the agent went on.
        assert_eq!(apply(Lamp::Turn, true, "working"), (Lamp::Working, false));
        assert_eq!(apply(Lamp::Turn, true, "edited"), (Lamp::Turn, true));
    }

    #[test]
    fn urgency_order() {
        let mut l = vec![Lamp::Turn, Lamp::Error, Lamp::Needs, Lamp::Limit];
        l.sort_by_key(|l| l.urgency());
        assert_eq!(l, vec![Lamp::Needs, Lamp::Limit, Lamp::Error, Lamp::Turn]);
    }
}
