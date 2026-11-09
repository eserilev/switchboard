//! Tests: a plain model of git's quoting, and of the old parsers.

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

/// git's C quoting of the bytes of a path, without the outer quotes.
fn quote(s: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    for &x in s {
        match x {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            7 => out.extend_from_slice(b"\\a"),
            8 => out.extend_from_slice(b"\\b"),
            9 => out.extend_from_slice(b"\\t"),
            10 => out.extend_from_slice(b"\\n"),
            11 => out.extend_from_slice(b"\\v"),
            12 => out.extend_from_slice(b"\\f"),
            13 => out.extend_from_slice(b"\\r"),
            x if x < 0x20 || x >= 0x7f => out.extend_from_slice(format!("\\{x:03o}").as_bytes()),
            x => out.push(x),
        }
    }
    out
}

/// The old `unquote` of `post.rs`, with the octal value in u32 so it cannot panic.
fn old_unquote(b: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = vec![];
    let mut k = 0;
    while k < b.len() {
        if b[k] != b'\\' || k + 1 >= b.len() {
            out.push(b[k]);
            k += 1;
            continue;
        }
        let c = b[k + 1];
        let octal = |x: u8| (b'0'..=b'7').contains(&x);
        if octal(c) && k + 3 < b.len() && octal(b[k + 2]) && octal(b[k + 3]) {
            let v = (c - b'0') as u32 * 64 + (b[k + 2] - b'0') as u32 * 8 + (b[k + 3] - b'0') as u32;
            out.push(v as u8);
            k += 4;
            continue;
        }
        out.push(match c {
            b'n' => b'\n',
            b't' => b'\t',
            b'a' => 7,
            b'b' => 8,
            b'f' => 12,
            b'r' => b'\r',
            b'v' => 11,
            other => other,
        });
        k += 2;
    }
    out
}

#[test]
fn unquote_examples() {
    assert_eq!(unquote(b"caf\\303\\251.rs"), "café.rs".as_bytes());
    assert_eq!(unquote(b"a\\\"b\\\\c\\td.rs"), b"a\"b\\c\td.rs");
    assert_eq!(unquote(b"plain/path.rs"), b"plain/path.rs");
    assert_eq!(unquote(b"end\\"), b"end\\");
    assert_eq!(unquote(b""), b"");
}

#[test]
fn unquote_keeps_a_backslash_that_starts_no_git_escape() {
    // The old code computed (4 - 0) * 64 in u8 here: a panic in debug.
    assert_eq!(unquote(b"\\400"), b"\\400");
    assert_eq!(unquote(b"\\777x"), b"\\777x");
    assert_eq!(unquote(b"\\q"), b"\\q");
    assert_eq!(unquote(b"\\12"), b"\\12");
    assert_eq!(unquote(b"\\377"), [255]);
    assert_eq!(unquote(b"\\000"), [0]);
}

#[test]
fn every_byte_round_trips() {
    for x in 0..=255u8 {
        assert_eq!(unquote(&quote(&[x])), [x], "byte {x}");
        assert_eq!(unquote(&quote(&[x, b'7', b'\\', x])), [x, b'7', b'\\', x], "byte {x}");
    }
}

#[test]
fn random_paths_round_trip_and_match_the_old_code() {
    let alphabet = b"\\\"01234567abfnrtvq/. \x7f\x80\xff\x01";
    for seed in 1..20_000u64 {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let n = r.below(12) as usize;
        let s: Vec<u8> = (0..n).map(|_| r.below(256) as u8).collect();
        assert_eq!(unquote(&quote(&s)), s);
        // On text that git writes, the new code agrees with the old one.
        assert_eq!(unquote(&quote(&s)), old_unquote(&quote(&s)));
        // Any bytes: no panic, never longer.
        let t: Vec<u8> = (0..n).map(|_| alphabet[r.below(alphabet.len() as u64) as usize]).collect();
        assert!(unquote(&t).len() <= t.len());
    }
}

fn hdr(h: &str) -> Option<(u32, u32, u32, u32)> {
    header(h.as_bytes()).map(|c| (c.old, c.old_end, c.new, c.new_end))
}

#[test]
fn header_reads_the_numbers() {
    assert_eq!(hdr("-10,4 +10,5 @@ fn x() {"), Some((10, 14, 10, 15)));
    assert_eq!(hdr("-3 +4 @@"), Some((3, 4, 4, 5)));
    assert_eq!(hdr("-0,0 +1,3 @@"), Some((0, 0, 1, 4)));
    assert_eq!(hdr("-1 +0,0"), Some((1, 2, 0, 0)));
    assert_eq!(hdr("-007,2 +1 @@"), Some((7, 9, 1, 2)));
    assert_eq!(hdr("-4294967294,1 +1 @@"), Some((4294967294, MAX, 1, 2)));
}

#[test]
fn header_refuses_other_forms() {
    for h in ["", "-", "-1", "-1 ", "-1 +", "-a +1 @@", "+1 -1 @@", "-1,+1 @@", "-1 +1,x @@", "-1  +1 @@", "-1 +1@@", "-1, +1"] {
        assert_eq!(hdr(h), None, "{h:?}");
    }
}

#[test]
fn header_refuses_an_end_over_max() {
    // The old code counted from 4294967295 and overflowed at the first row.
    assert_eq!(hdr("-1 +4294967295 @@"), None);
    assert_eq!(hdr("-1 +4294967295,0 @@"), Some((1, 2, MAX, MAX)));
    assert_eq!(hdr("-4294967296,0 +1 @@"), None);
    assert_eq!(hdr("-1,4294967295 +1 @@"), None);
    assert_eq!(hdr("-99999999999999999999 +1 @@"), None);
}

#[test]
fn step_counts_and_stops_at_the_end() {
    let c = header(b"-10,2 +10,1 @@").unwrap();
    let c = step(&c, true, true).unwrap();
    assert_eq!((c.old, c.new), (11, 11));
    assert!(step(&c, true, true).is_none());
    let c = step(&c, true, false).unwrap();
    assert_eq!((c.old, c.new), (12, 11));
    assert!(step(&c, true, false).is_none());
    assert!(step(&c, false, true).is_none());
    let full = Counter { old: MAX, new: MAX, old_end: MAX, new_end: MAX };
    assert!(step(&full, true, false).is_none());
    assert!(step(&full, false, true).is_none());
}

#[test]
fn random_headers_match_a_model() {
    for seed in 1..20_000u64 {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut num = || -> u64 {
            match r.below(4) {
                0 => r.below(10),
                1 => r.below(1000),
                2 => 4294967290 + r.below(10),
                _ => r.next() >> (20 + r.below(44)),
            }
        };
        let (a, b, c, d) = (num(), num(), num(), num());
        let (with_b, with_d) = (seed % 2 == 0, seed % 3 == 0);
        let mut h = format!("-{a}");
        if with_b {
            h += &format!(",{b}");
        }
        h += &format!(" +{c}");
        if with_d {
            h += &format!(",{d}");
        }
        h += " @@";
        let (b, d) = (if with_b { b } else { 1 }, if with_d { d } else { 1 });
        let m = MAX as u64;
        let want = if a + b <= m && c + d <= m {
            Some((a as u32, (a + b) as u32, c as u32, (c + d) as u32))
        } else {
            None
        };
        assert_eq!(hdr(&h), want, "{h}");
    }
}
