//! The screens of the windows on one hub (SPEC 41.8).
//!
//! Each window has its own screen: a live pane and its parts. A tmux pane streams
//! while at least one window shows it. A terminal emulator follows each streamed
//! pane, so a window that lost bytes gets a fresh screen without a pause of tmux.
//! A pause stops the bytes for every window, and a slow phone then stops the desktop.

use std::collections::{BTreeMap, HashMap};
use std::num::NonZeroU16;

/// The id of a window. The app's own window is [`LOCAL`]; a remote window has the
/// epoch of its link, from 1 up.
pub type Win = u64;

/// The app's own window, and the window of the hub calls with no window.
pub const LOCAL: Win = 0;

/// (our id, tmux id) of each pane on one screen. The first is the live pane; the
/// others are its parts (SPEC 28).
pub type Screen = Vec<(String, String)>;

/// The screens of all windows.
#[derive(Default, Debug)]
pub struct Screens {
    wins: BTreeMap<Win, Screen>,
}

impl Screens {
    /// The screen of a window. A window with no screen has an empty one.
    pub fn of(&self, win: Win) -> &[(String, String)] {
        self.wins.get(&win).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The screen of a window, to change it.
    pub fn of_mut(&mut self, win: Win) -> &mut Screen {
        self.wins.entry(win).or_default()
    }

    /// Takes the screen of a window away and returns it.
    pub fn take(&mut self, win: Win) -> Screen {
        self.wins.remove(&win).unwrap_or_default()
    }

    /// The number of windows that show the tmux pane `tid`: its watch count.
    pub fn watch_count(&self, tid: &str) -> usize {
        self.wins.values().filter(|s| s.iter().any(|(_, t)| t == tid)).count()
    }

    /// True while some window shows the tmux pane `tid`: it streams.
    pub fn streams(&self, tid: &str) -> bool {
        self.watch_count(tid) > 0
    }

    /// The windows that show the tmux pane `tid`, with our id of it.
    pub fn watchers(&self, tid: &str) -> Vec<(Win, String)> {
        let mut out = vec![];
        for (w, s) in &self.wins {
            if let Some((id, _)) = s.iter().find(|(_, t)| t == tid) {
                out.push((*w, id.clone()));
            }
        }
        out
    }

    /// True when some window shows the pane `id`.
    pub fn shows(&self, id: &str) -> bool {
        self.wins.values().any(|s| s.iter().any(|(l, _)| l == id))
    }

    /// The tmux id of the pane `id` on the screen of `win`.
    pub fn tid_in(&self, win: Win, id: &str) -> Option<String> {
        self.of(win).iter().find(|(l, _)| l == id).map(|(_, t)| t.clone())
    }

    /// The pane `id` has a new tmux pane (a respawn): every screen follows it.
    pub fn retarget(&mut self, id: &str, tid: &str) {
        for s in self.wins.values_mut() {
            for l in s.iter_mut().filter(|(l, _)| l == id) {
                l.1 = tid.to_owned();
            }
        }
    }

    /// The pane `id` closes. On a screen where it is the live pane, its parts go
    /// with it; elsewhere it goes alone. Returns the tmux ids that left a screen.
    pub fn close(&mut self, id: &str) -> Vec<String> {
        let mut gone = vec![];
        for s in self.wins.values_mut() {
            if s.first().map(|(l, _)| l == id).unwrap_or(false) {
                gone.extend(s.drain(..).map(|(_, t)| t));
            } else {
                let (out, keep): (Screen, Screen) = s.drain(..).partition(|(l, _)| l == id);
                *s = keep;
                gone.extend(out.into_iter().map(|(_, t)| t));
            }
        }
        self.wins.retain(|_, s| !s.is_empty());
        gone
    }

    /// Of `tids`, the ones that no window shows any more: their streams stop.
    pub fn unwatched(&self, tids: impl IntoIterator<Item = String>) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for t in tids {
            if !self.streams(&t) && !out.contains(&t) {
                out.push(t);
            }
        }
        out
    }
}

/// A terminal emulator for each streamed tmux pane, fed by the same bytes as the windows.
#[derive(Default)]
pub struct Terms {
    by_tid: HashMap<String, vt100::Parser>,
}

fn dim(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n.max(1)).unwrap_or(NonZeroU16::MIN)
}

impl Terms {
    /// Starts to follow a pane at a size, from its screen. A pane it follows already keeps its state.
    pub fn start(&mut self, tid: &str, cols: u16, rows: u16, screen: &[u8]) {
        if self.by_tid.contains_key(tid) {
            return;
        }
        let mut p = vt100::Parser::new(dim(rows), dim(cols), 0);
        p.process(screen);
        self.by_tid.insert(tid.to_owned(), p);
    }

    /// Stops following a pane.
    pub fn stop(&mut self, tid: &str) {
        self.by_tid.remove(tid);
    }

    /// True while it follows the pane.
    pub fn follows(&self, tid: &str) -> bool {
        self.by_tid.contains_key(tid)
    }

    /// Bytes of a pane. A pane it does not follow ignores them.
    pub fn feed(&mut self, tid: &str, data: &[u8]) {
        if let Some(p) = self.by_tid.get_mut(tid) {
            p.process(data);
        }
    }

    /// The pane has a new size.
    pub fn resize(&mut self, tid: &str, cols: u16, rows: u16) {
        if let Some(p) = self.by_tid.get_mut(tid) {
            p.screen_mut().set_size(dim(rows), dim(cols));
        }
    }

    /// The bytes that draw the pane's screen as it is now: clear, the cells, the
    /// cursor and the input modes. `None` for a pane it does not follow. It starts
    /// as the screen of an expand starts, so a window knows a full screen.
    pub fn snapshot(&self, tid: &str) -> Option<Vec<u8>> {
        self.by_tid.get(tid).map(|p| {
            let mut out = b"\x1b[H\x1b[2J".to_vec();
            out.extend(p.screen().state_formatted().into_bytes());
            out
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: &str, t: &str) -> (String, String) {
        (id.to_owned(), t.to_owned())
    }

    #[test]
    fn a_pane_streams_while_one_window_shows_it() {
        let mut sc = Screens::default();
        *sc.of_mut(1) = vec![s("p1", "%1")];
        *sc.of_mut(2) = vec![s("p1", "%1"), s("p2", "%2")];
        assert_eq!(sc.watch_count("%1"), 2);
        assert_eq!(sc.watchers("%1"), vec![(1, "p1".to_owned()), (2, "p1".to_owned())]);
        let old = sc.take(2);
        assert_eq!(sc.unwatched(old.into_iter().map(|(_, t)| t)), vec!["%2".to_owned()]);
        assert!(sc.streams("%1"));
        sc.take(1);
        assert!(!sc.streams("%1"));
    }

    #[test]
    fn a_closed_live_pane_takes_its_parts_on_its_own_screen_only() {
        let mut sc = Screens::default();
        *sc.of_mut(1) = vec![s("p1", "%1"), s("p3", "%3")];
        *sc.of_mut(2) = vec![s("p2", "%2"), s("p1", "%1")];
        let mut gone = sc.close("p1");
        gone.sort();
        assert_eq!(gone, vec!["%1", "%1", "%3"]);
        assert!(sc.of(1).is_empty());
        assert_eq!(sc.of(2), &[s("p2", "%2")]);
    }

    #[test]
    fn a_respawn_moves_every_screen() {
        let mut sc = Screens::default();
        *sc.of_mut(1) = vec![s("p1", "%1")];
        *sc.of_mut(2) = vec![s("p1", "%1")];
        sc.retarget("p1", "%9");
        assert_eq!(sc.tid_in(1, "p1").as_deref(), Some("%9"));
        assert_eq!(sc.tid_in(2, "p1").as_deref(), Some("%9"));
    }

    /// The snapshot draws the same screen in a fresh terminal.
    #[test]
    fn a_snapshot_draws_the_same_screen() {
        let mut t = Terms::default();
        t.start("%1", 20, 4, b"\x1b[H\x1b[2Jhello");
        t.feed("%1", b"\r\n\x1b[31mred\x1b[0m line\r\nthird");
        let snap = t.snapshot("%1").unwrap();
        let mut fresh = vt100::Parser::new(dim(4), dim(20), 0);
        fresh.process(b"old junk everywhere\r\nmore junk");
        fresh.process(&snap);
        let want = t.by_tid["%1"].screen();
        assert_eq!(fresh.screen().contents(), want.contents());
        assert_eq!(fresh.screen().cursor_position(), want.cursor_position());
        assert_eq!(fresh.screen().cell(1, 0).unwrap().fgcolor(), vt100::Color::Idx(1));
        // A second start keeps the state.
        t.start("%1", 20, 4, b"\x1b[2J");
        assert!(t.snapshot("%1").unwrap() == snap);
        t.stop("%1");
        assert!(t.snapshot("%1").is_none());
    }
}
