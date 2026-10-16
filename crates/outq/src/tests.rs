//! Random steps against a plain model of the queue: a list of (key, state, bytes).

use super::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// What the window gets for one pane: bytes, or a fresh screen.
#[derive(Debug, PartialEq, Clone)]
enum Got {
    Bytes(Vec<u8>),
    Screen(Vec<u8>),
}

#[test]
fn random_steps_keep_budget_order_and_no_gap() {
    for seed in 1..400u64 {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let budget = 1 + r.below(64) as usize;
        let mut q = new(budget);
        // Per key: every byte pushed since the last fresh screen, while live, and
        // whether a drop happened since then.
        let mut sent: Vec<(u32, Vec<u8>, bool)> = vec![];
        let mut got: Vec<(u32, Vec<Got>)> = vec![];
        for _ in 0..300 {
            let key = r.below(4) as u32;
            match r.below(6) {
                0..=2 => {
                    let n = r.below(budget as u64 + 8) as usize;
                    let data: Vec<u8> = (0..n).map(|_| r.below(256) as u8).collect();
                    let before = q.panes.iter().find(|p| p.key == key).map(|p| (p.state, p.bytes.clone()));
                    push(&mut q, key, &data);
                    let p = &q.panes[find(&q, key)];
                    let (st, bs) = before.unwrap_or((LIVE, vec![]));
                    if st == LIVE && bs.len() + data.len() <= budget {
                        assert_eq!(p.bytes, [bs, data.clone()].concat(), "order");
                        let s = sent.iter_mut().find(|s| s.0 == key);
                        match s {
                            Some(s) => s.1.extend(&data),
                            None => sent.push((key, data, false)),
                        }
                    } else if st == LIVE {
                        assert!(p.bytes.is_empty() && p.state == DROPPING, "a drop");
                        match sent.iter_mut().find(|s| s.0 == key) {
                            Some(s) => s.2 = true,
                            None => sent.push((key, vec![], true)),
                        }
                    } else {
                        assert_eq!((p.state, &p.bytes), (st, &bs), "a waiting pane takes nothing");
                    }
                    assert!(p.state != LIVE || p.bytes.len() <= budget, "budget");
                }
                3 => {
                    if !q.panes.is_empty() {
                        let i = r.below(q.panes.len() as u64) as usize;
                        let k = q.panes[i].key;
                        let b = take(&mut q, i);
                        if !b.is_empty() {
                            match got.iter_mut().find(|g| g.0 == k) {
                                Some(g) => g.1.push(Got::Bytes(b)),
                                None => got.push((k, vec![Got::Bytes(b)])),
                            }
                        }
                    }
                }
                4 => {
                    if !q.panes.is_empty() {
                        let i = r.below(q.panes.len() as u64) as usize;
                        let was = q.panes[i].state;
                        assert_eq!(want_screen(&mut q, i), was == DROPPING);
                        assert!(!want_screen(&mut q, i), "only once");
                    }
                }
                _ => {
                    let i = find(&q, key);
                    let waits = i < q.panes.len() && q.panes[i].state == RESYNC;
                    let screen = vec![0xEE, key as u8];
                    fill(&mut q, key, screen.clone());
                    if waits {
                        assert_eq!(q.panes[i].bytes, screen);
                        // A fresh screen starts the record again.
                        if let Some(s) = sent.iter_mut().find(|s| s.0 == key) {
                            *s = (key, vec![], false);
                        }
                        match got.iter_mut().find(|g| g.0 == key) {
                            Some(g) => g.1 = vec![Got::Screen(screen)],
                            None => got.push((key, vec![Got::Screen(screen)])),
                        }
                        // The screen itself is taken next as bytes; skip it in the record.
                        let b = take(&mut q, i);
                        assert_eq!(b, vec![0xEE, key as u8]);
                    }
                }
            }
            // No gap: with no drop since the last screen, the window has every byte
            // in order: what it got plus what waits equals what was sent.
            for (k, bytes, dropped) in &sent {
                if *dropped {
                    continue;
                }
                let mut seen: Vec<u8> = vec![];
                if let Some(g) = got.iter().find(|g| g.0 == *k) {
                    for x in &g.1 {
                        if let Got::Bytes(b) = x {
                            seen.extend(b);
                        }
                    }
                }
                let i = find(&q, *k);
                if i < q.panes.len() {
                    seen.extend(&q.panes[i].bytes);
                }
                assert_eq!(&seen, bytes, "seed {seed} key {k}");
            }
        }
    }
}

#[test]
fn fits_has_no_overflow() {
    assert!(fits(0, usize::MAX, usize::MAX));
    assert!(!fits(1, usize::MAX, usize::MAX));
    assert!(!fits(5, 0, 4));
    assert!(fits(4, 0, 4));
}

#[test]
fn forget_makes_a_pane_live_and_empty() {
    let mut q = new(2);
    push(&mut q, 7, b"abc");
    assert_eq!(q.panes[0].state, DROPPING);
    forget(&mut q, 7);
    assert_eq!((q.panes[0].state, q.panes[0].bytes.len()), (LIVE, 0));
    fill(&mut q, 7, b"zz".to_vec());
    assert!(q.panes[0].bytes.is_empty(), "a fill for a pane that does not wait does nothing");
}
