//! The event ring of a hub (SPEC 41.8).
//!
//! Each event of a hub start gets a number `n`, from 1 up. The ring keeps the
//! newest event of each key (a kind and an id, for example the `review` event of
//! one review), so it covers hours, not minutes. A window that comes back says the
//! last number it saw, and gets the events after it, in order. When the ring cannot
//! tell, the window loads everything again: a "resync".
//!
//! The statements to prove in Lean with Aeneas (SPEC 41.14):
//!
//! - **Fold.** After a record, the ring holds at most one event of each key, and
//!   it is the newest.
//! - **Order.** The ring holds its events in the order of their numbers.
//! - **Since.** For a number at or above the floor, `since` returns exactly the
//!   events of the ring with a larger number, in order. Below the floor, it
//!   returns `None`.
//! - **Cap.** The ring never holds more than `cap` events.
//!
//! The code stays in the Rust subset that Aeneas translates, as `outq` does:
//! `while` loops with an index, no iterators, no closures, no `remove`.

#![forbid(unsafe_code)]
// Nested `if`s on purpose: the kernel avoids `&&` so the Aeneas translation stays plain.
#![allow(clippy::collapsible_if)]

/// One event in the ring: its key and its number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ev {
    pub key: u64,
    pub n: u64,
}

pub struct Ring {
    /// The events, oldest first.
    pub evs: Vec<Ev>,
    /// The number of the newest event that left the ring because of the cap.
    /// A window that saw less than it missed that event: it must resync.
    pub floor: u64,
    pub cap: usize,
}

pub fn new(cap: usize) -> Ring {
    Ring { evs: Vec::new(), floor: 0, cap }
}

/// Records event `n` of `key`. The older event of the same key leaves the ring.
/// When the ring then holds more than `cap` events, the oldest leaves, and the
/// floor rises to its number. The numbers must grow from call to call.
pub fn record(r: &mut Ring, key: u64, n: u64) {
    let mut kept: Vec<Ev> = Vec::new();
    let mut i: usize = 0;
    while i < r.evs.len() {
        let e = r.evs[i];
        if e.key != key {
            kept.push(e);
        }
        i += 1;
    }
    kept.push(Ev { key, n });
    if kept.len() > r.cap {
        let oldest = kept[0].n;
        let mut rest: Vec<Ev> = Vec::new();
        let mut j: usize = 1;
        while j < kept.len() {
            rest.push(kept[j]);
            j += 1;
        }
        r.floor = oldest;
        r.evs = rest;
    } else {
        r.evs = kept;
    }
}

/// The events after number `n`, oldest first. `None` when `n` is below the floor:
/// an event the window did not see left the ring.
pub fn since(r: &Ring, n: u64) -> Option<Vec<Ev>> {
    if n < r.floor {
        return None;
    }
    let mut out: Vec<Ev> = Vec::new();
    let mut i: usize = 0;
    while i < r.evs.len() {
        let e = r.evs[i];
        if e.n > n {
            out.push(e);
        }
        i += 1;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(v: &[Ev]) -> Vec<(u64, u64)> {
        v.iter().map(|e| (e.key, e.n)).collect()
    }

    #[test]
    fn the_newest_event_of_a_key_stays() {
        let mut r = new(10);
        record(&mut r, 1, 1);
        record(&mut r, 2, 2);
        record(&mut r, 1, 3);
        assert_eq!(keys(&r.evs), vec![(2, 2), (1, 3)]);
        assert_eq!(keys(&since(&r, 0).unwrap()), vec![(2, 2), (1, 3)]);
        assert_eq!(keys(&since(&r, 2).unwrap()), vec![(1, 3)]);
        assert!(since(&r, 3).unwrap().is_empty());
    }

    #[test]
    fn past_the_cap_the_oldest_goes_and_an_older_window_resyncs() {
        let mut r = new(2);
        record(&mut r, 1, 1);
        record(&mut r, 2, 2);
        record(&mut r, 3, 3);
        assert_eq!(keys(&r.evs), vec![(2, 2), (3, 3)]);
        assert_eq!(r.floor, 1);
        assert!(since(&r, 0).is_none(), "event 1 left the ring");
        assert_eq!(keys(&since(&r, 1).unwrap()), vec![(2, 2), (3, 3)]);
    }

    /// Against a full log: the events after `n` in the ring are the newest of
    /// each key among the events after `n`, in order.
    #[test]
    fn since_matches_a_full_log() {
        let mut seed: u64 = 7;
        for _ in 0..200 {
            let mut r = new(5);
            let mut log: Vec<Ev> = vec![];
            for n in 1..40u64 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let key = (seed >> 33) % 4;
                record(&mut r, key, n);
                log.push(Ev { key, n });
            }
            for from in 0..40u64 {
                let want: Vec<Ev> = log
                    .iter()
                    .filter(|e| e.n > from)
                    .filter(|e| !log.iter().any(|x| x.key == e.key && x.n > e.n))
                    .copied()
                    .collect();
                match since(&r, from) {
                    Some(got) => assert_eq!(got, want, "from {from}"),
                    None => assert!(from < r.floor),
                }
            }
        }
    }
}
