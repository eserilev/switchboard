//! A plain model of the spec, and random tests that compare `check` with it.

use super::*;

/// The lines that a mask keeps.
fn keep<'a>(lines: &'a [Vec<u8>], mask: &[bool]) -> Vec<&'a Vec<u8>> {
    lines
        .iter()
        .zip(mask)
        .filter(|(_, m)| !**m)
        .map(|(l, _)| l)
        .collect()
}

fn is_covered(spans: &[Span], f: usize, old: bool, line: usize) -> bool {
    spans
        .iter()
        .any(|s| s.file == f && s.old == old && s.from <= line && line <= s.to)
}

/// The spec of `check`, as the Lean statements say it.
fn spec(files: &[FileDiff], spans: &[Span], pad: usize) -> bool {
    let files_good = files.iter().enumerate().all(|(f, d)| {
        d.removed.len() == d.old.len()
            && d.added.len() == d.new.len()
            && keep(&d.old, &d.removed) == keep(&d.new, &d.added)
            && d.removed
                .iter()
                .enumerate()
                .all(|(k, m)| !m || is_covered(spans, f, true, k + 1))
            && d.added
                .iter()
                .enumerate()
                .all(|(k, m)| !m || is_covered(spans, f, false, k + 1))
    });
    let spans_good = spans.iter().all(|s| {
        let Some(d) = files.get(s.file) else {
            return false;
        };
        let mask = if s.old { &d.removed } else { &d.added };
        1 <= s.from
            && s.from <= s.to
            && s.to <= mask.len()
            && (s.from..=s.to).any(|l| mask[l - 1])
            && (s.from..=s.to).all(|l| {
                mask.iter()
                    .enumerate()
                    .any(|(k, m)| *m && (k + 1).abs_diff(l) <= pad)
            })
    });
    files_good && spans_good
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
}

/// A random file pair, and a mask that is right most of the time.
fn random_file(r: &mut Rng) -> FileDiff {
    let pool: Vec<Vec<u8>> = (0..4).map(|i| format!("line {i}\n").into_bytes()).collect();
    let n = r.below(9) as usize;
    let old: Vec<Vec<u8>> = (0..n).map(|_| pool[r.below(4) as usize].clone()).collect();
    // Edit old into new and record the edit, so the masks are often right.
    let mut new = vec![];
    let mut removed = vec![false; old.len()];
    let mut added = vec![];
    for (k, line) in old.iter().enumerate() {
        if r.chance(25) {
            removed[k] = true;
        } else {
            new.push(line.clone());
            added.push(false);
        }
        if r.chance(20) {
            new.push(pool[r.below(4) as usize].clone());
            added.push(true);
        }
    }
    // Sometimes break it: flip a mask bit, or drop the final newline.
    if r.chance(15) && !removed.is_empty() {
        let k = r.below(removed.len() as u64) as usize;
        removed[k] = !removed[k];
    }
    if r.chance(15) && !added.is_empty() {
        let k = r.below(added.len() as u64) as usize;
        added[k] = !added[k];
    }
    if r.chance(5) {
        if let Some(last) = new.last_mut() {
            last.pop();
        }
    }
    if r.chance(3) {
        added.push(false);
    }
    FileDiff {
        old,
        new,
        removed,
        added,
    }
}

fn random_span(r: &mut Rng, files: &[FileDiff]) -> Span {
    let file = r.below(files.len() as u64 + 1) as usize;
    let old = r.chance(50);
    let from = r.below(10) as usize;
    let to = from + r.below(4) as usize;
    Span {
        file,
        old,
        from,
        to,
    }
}

/// Spans that cover every changed line, one span per changed line.
fn exact_spans(files: &[FileDiff]) -> Vec<Span> {
    let mut out = vec![];
    for (f, d) in files.iter().enumerate() {
        for (k, m) in d.removed.iter().enumerate() {
            if *m {
                out.push(Span {
                    file: f,
                    old: true,
                    from: k + 1,
                    to: k + 1,
                });
            }
        }
        for (k, m) in d.added.iter().enumerate() {
            if *m {
                out.push(Span {
                    file: f,
                    old: false,
                    from: k + 1,
                    to: k + 1,
                });
            }
        }
    }
    out
}

#[test]
fn check_agrees_with_the_spec_on_random_inputs() {
    let mut r = Rng(0x5eed_1234_abcd_ef01);
    let (mut accepted, mut rejected) = (0, 0);
    for _ in 0..40_000 {
        let files: Vec<FileDiff> = (0..1 + r.below(3)).map(|_| random_file(&mut r)).collect();
        let mut spans = if r.chance(60) {
            exact_spans(&files)
        } else {
            vec![]
        };
        for _ in 0..r.below(4) {
            spans.push(random_span(&mut r, &files));
        }
        if r.chance(30) && !spans.is_empty() {
            let i = r.below(spans.len() as u64) as usize;
            spans.remove(i);
        }
        let pad = r.below(4) as usize;
        let want = spec(&files, &spans, pad);
        assert_eq!(check(&files, &spans, pad), want);
        if want {
            accepted += 1;
        } else {
            rejected += 1;
        }
    }
    // Both answers must come up often, or the test proves little.
    assert!(accepted > 3000, "accepted {accepted}");
    assert!(rejected > 3000, "rejected {rejected}");
}

fn lines(text: &str) -> Vec<Vec<u8>> {
    text.split_inclusive('\n')
        .map(|l| l.as_bytes().to_vec())
        .collect()
}

fn one(old: &str, new: &str, removed: &[bool], added: &[bool]) -> Vec<FileDiff> {
    vec![FileDiff {
        old: lines(old),
        new: lines(new),
        removed: removed.to_vec(),
        added: added.to_vec(),
    }]
}

#[test]
fn a_right_diff_and_a_full_guide_pass() {
    let files = one(
        "a\nb\nc\n",
        "a\nB\nc\nd\n",
        &[false, true, false],
        &[false, true, false, true],
    );
    let spans = [
        Span {
            file: 0,
            old: true,
            from: 2,
            to: 2,
        },
        Span {
            file: 0,
            old: false,
            from: 2,
            to: 4,
        },
    ];
    assert!(check(&files, &spans, 1));
}

#[test]
fn a_change_outside_the_diff_fails_the_rebuild() {
    // Line 3 changed, but the diff marks only line 2.
    let files = one(
        "a\nb\nc\n",
        "a\nB\nC\n",
        &[false, true, false],
        &[false, true, false],
    );
    let spans = [
        Span {
            file: 0,
            old: true,
            from: 2,
            to: 2,
        },
        Span {
            file: 0,
            old: false,
            from: 2,
            to: 2,
        },
    ];
    assert!(!check(&files, &spans, 0));
}

#[test]
fn a_dropped_final_newline_is_a_change() {
    let files = one("a\n", "a", &[false], &[false]);
    assert!(!check(&files, &[], 0));
    let files = one("a\n", "a", &[true], &[true]);
    let spans = [
        Span {
            file: 0,
            old: true,
            from: 1,
            to: 1,
        },
        Span {
            file: 0,
            old: false,
            from: 1,
            to: 1,
        },
    ];
    assert!(check(&files, &spans, 0));
}

#[test]
fn a_missed_line_fails_coverage() {
    let files = one("a\n", "a\nb\nc\n", &[false], &[false, true, true]);
    let spans = [Span {
        file: 0,
        old: false,
        from: 2,
        to: 2,
    }];
    assert!(!check(&files, &spans, 5));
}

#[test]
fn a_span_with_no_change_or_too_wide_fails() {
    let files = one(
        "a\nb\nc\nd\ne\n",
        "a\nb\nC\nd\ne\n",
        &[false, false, true, false, false],
        &[false, false, true, false, false],
    );
    let good = [
        Span {
            file: 0,
            old: true,
            from: 3,
            to: 3,
        },
        Span {
            file: 0,
            old: false,
            from: 2,
            to: 4,
        },
    ];
    assert!(check(&files, &good, 1));
    let wide = [
        Span {
            file: 0,
            old: true,
            from: 3,
            to: 3,
        },
        Span {
            file: 0,
            old: false,
            from: 1,
            to: 5,
        },
    ];
    assert!(!check(&files, &wide, 1));
    assert!(check(&files, &wide, 2));
    let empty = [
        Span {
            file: 0,
            old: true,
            from: 3,
            to: 3,
        },
        Span {
            file: 0,
            old: false,
            from: 3,
            to: 3,
        },
        Span {
            file: 0,
            old: false,
            from: 5,
            to: 5,
        },
    ];
    assert!(!check(&files, &empty, 9));
}

#[test]
fn bad_span_bounds_fail() {
    let files = one("a\n", "b\n", &[true], &[true]);
    let base = [
        Span {
            file: 0,
            old: true,
            from: 1,
            to: 1,
        },
        Span {
            file: 0,
            old: false,
            from: 1,
            to: 1,
        },
    ];
    assert!(check(&files, &base, 0));
    for bad in [
        Span {
            file: 1,
            old: true,
            from: 1,
            to: 1,
        },
        Span {
            file: 0,
            old: true,
            from: 0,
            to: 1,
        },
        Span {
            file: 0,
            old: true,
            from: 2,
            to: 1,
        },
        Span {
            file: 0,
            old: false,
            from: 1,
            to: 2,
        },
    ] {
        let spans = [Span { ..base[0] }, Span { ..base[1] }, bad];
        assert!(!check(&files, &spans, 3));
    }
}

#[test]
fn big_files_are_fast_enough() {
    let old: Vec<Vec<u8>> = (0..9000)
        .map(|i| format!("fn line_{i}() {{}}\n").into_bytes())
        .collect();
    let mut new = old.clone();
    let mut added = vec![false; new.len()];
    for k in (100..9000).step_by(300) {
        new[k] = b"changed\n".to_vec();
        added[k] = true;
    }
    let removed = added.clone();
    let files = vec![FileDiff {
        old,
        new,
        removed,
        added: added.clone(),
    }];
    let mut spans = vec![];
    for (k, m) in added.iter().enumerate() {
        if *m {
            spans.push(Span {
                file: 0,
                old: true,
                from: k + 1,
                to: k + 1,
            });
            spans.push(Span {
                file: 0,
                old: false,
                from: k.saturating_sub(5) + 1,
                to: k + 6,
            });
        }
    }
    let t0 = std::time::Instant::now();
    assert!(check(&files, &spans, 15));
    assert!(t0.elapsed().as_secs_f64() < 2.0, "{:?}", t0.elapsed());
}
