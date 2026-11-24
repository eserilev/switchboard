//! The numbered events of a hub start (SPEC 41.8), around the ring of `evseq`.
//!
//! Each event gets `(boot, n)`: `boot` is new for each start of the hub, and `n`
//! counts from 1. A window that comes back sends the last `(boot, n)` it saw. With
//! the same boot and an `n` the ring still covers, it gets the events it missed,
//! the newest of each kind and id, in order. Else it loads everything again.

use serde_json::Value;
use std::collections::HashMap;

/// The events that the ring keeps, at most.
pub const CAP: usize = 2000;

/// Events that ask a window to act now, not state: a window that comes back
/// later does not get them. `focus` shows a pane; later, it jumps for no reason.
pub const TRANSIENT: &[&str] = &["focus"];

pub struct EventLog {
    pub boot: u64,
    next: u64,
    ring: evseq::Ring,
    /// The line of each event in the ring, by key.
    lines: HashMap<u64, String>,
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new(new_boot(), CAP)
    }
}

/// A boot id: the clock and the pid. Never 0, so 0 can mean "none".
fn new_boot() -> u64 {
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let b = (at.as_nanos() as u64) ^ ((std::process::id() as u64) << 40);
    b.max(1)
}

/// The id that an event folds by: the newest event of one kind and id replaces
/// the older ones. For example, the `pane` event of one pane, or the `review`
/// event of one review. An event with no id folds by its kind.
pub fn fold_id(payload: &Value) -> String {
    for field in ["id", "review", "pane"] {
        match payload.get(field) {
            Some(Value::String(s)) => return s.clone(),
            Some(Value::Number(n)) => return n.to_string(),
            _ => {}
        }
    }
    match payload {
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// FNV-1a over the kind, a zero byte, and the id.
fn key_of(kind: &str, id: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in kind.bytes().chain([0u8]).chain(id.bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

impl EventLog {
    pub fn new(boot: u64, cap: usize) -> Self {
        EventLog { boot, next: 1, ring: evseq::new(cap), lines: HashMap::new() }
    }

    /// Numbers the next event of `kind` with `payload`, keeps it, and returns its
    /// number. `line` makes the line of the event from its number.
    pub fn record(&mut self, kind: &str, payload: &Value, line: impl FnOnce(u64) -> String) -> (u64, String) {
        let n = self.next;
        self.next += 1;
        let l = line(n);
        if TRANSIENT.contains(&kind) {
            return (n, l);
        }
        let key = key_of(kind, &fold_id(payload));
        evseq::record(&mut self.ring, key, n);
        self.lines.insert(key, l.clone());
        if self.lines.len() > self.ring.evs.len() {
            let live: std::collections::HashSet<u64> = self.ring.evs.iter().map(|e| e.key).collect();
            self.lines.retain(|k, _| live.contains(k));
        }
        (n, l)
    }

    /// The lines a window missed after `(boot, n)`, oldest first. `None` when the
    /// window must load everything again: another start of the hub, or an event it
    /// did not see left the ring.
    pub fn since(&self, boot: u64, n: u64) -> Option<Vec<String>> {
        if boot != self.boot || n >= self.next {
            return None;
        }
        let evs = evseq::since(&self.ring, n)?;
        Some(evs.iter().filter_map(|e| self.lines.get(&e.key).cloned()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(log: &mut EventLog, kind: &str, p: Value) -> u64 {
        log.record(kind, &p, |n| format!("{kind}:{p}:{n}")).0
    }

    #[test]
    fn a_window_gets_the_newest_of_each_kind_and_id_it_missed() {
        let mut log = EventLog::new(9, 100);
        rec(&mut log, "panes", json!([1]));
        let seen = rec(&mut log, "review", json!({ "id": "r1", "v": 1 }));
        rec(&mut log, "review", json!({ "id": "r1", "v": 2 }));
        rec(&mut log, "review", json!({ "id": "r2", "v": 1 }));
        rec(&mut log, "panes", json!([2]));
        rec(&mut log, "review", json!({ "id": "r1", "v": 3 }));
        let got = log.since(9, seen).unwrap();
        assert_eq!(got.len(), 3, "{got:?}");
        assert!(got[0].starts_with("review:") && got[0].contains("r2"));
        assert!(got[1].starts_with("panes:[2]"));
        assert!(got[2].contains("\"v\":3"));
        // Nothing missed.
        assert_eq!(log.since(9, 6).unwrap().len(), 0);
    }

    #[test]
    fn another_boot_or_a_number_out_of_reach_resyncs() {
        let mut log = EventLog::new(9, 2);
        for i in 0..5 {
            rec(&mut log, "mail", json!({ "id": i }));
        }
        assert!(log.since(8, 4).is_none(), "another start of the hub");
        assert!(log.since(9, 1).is_none(), "events 2 and 3 left the ring");
        assert_eq!(log.since(9, 3).unwrap().len(), 2);
        assert!(log.since(9, 99).is_none(), "a number this hub never gave");
        assert!(log.lines.len() <= 2);
    }

    #[test]
    fn a_transient_event_gets_a_number_and_no_replay() {
        let mut log = EventLog::new(9, 10);
        rec(&mut log, "panes", json!([]));
        let n = rec(&mut log, "focus", json!("p3"));
        assert_eq!(n, 2);
        assert!(log.since(9, 1).unwrap().is_empty());
    }

    #[test]
    fn fold_ids() {
        assert_eq!(fold_id(&json!({ "id": "p3" })), "p3");
        assert_eq!(fold_id(&json!({ "review": 7, "note": "x" })), "7");
        assert_eq!(fold_id(&json!({ "pane": "p1", "text": "x" })), "p1");
        assert_eq!(fold_id(&json!("r9")), "r9");
        assert_eq!(fold_id(&json!([1, 2])), "");
        assert_eq!(fold_id(&Value::Null), "");
    }
}
