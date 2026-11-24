use super::*;

const LISTS: [&[u8]; 6] = [READ_CMDS, REVIEW_CMDS, ASK_CMDS, ACT_CMDS, TERMINAL_CMDS, ADMIN_CMDS];

fn names() -> Vec<&'static [u8]> {
    LISTS.iter().flat_map(|l| l.split(|b| *b == b' ').filter(|w| !w.is_empty())).collect()
}

#[test]
fn each_command_is_in_one_class() {
    let all = names();
    for n in &all {
        let in_lists = LISTS.iter().filter(|l| in_list(n, l)).count();
        assert_eq!(in_lists, 1, "{}", String::from_utf8_lossy(n));
        assert_ne!(class_of(n), 0);
    }
    // No double space, so no empty name.
    for l in LISTS {
        assert!(!l.windows(2).any(|w| w == b"  "));
        assert!(!l.contains(&b'\n'));
    }
}

#[test]
fn an_unknown_command_is_refused_for_everyone() {
    assert_eq!(check(TRUSTED, 0, b"pane_kill", b""), UNKNOWN);
    assert_eq!(check(DEVICE, 255, b"", b""), UNKNOWN);
    assert_eq!(check(DEVICE, 255, b"panes ", b""), UNKNOWN);
    assert_eq!(check(DEVICE, 255, b"pane", b""), UNKNOWN);
}

#[test]
fn a_trusted_window_runs_everything_in_the_table() {
    for n in names() {
        assert_eq!(check(TRUSTED, 0, n, b""), OK);
    }
}

#[test]
fn a_device_never_runs_admin() {
    for n in ADMIN_CMDS.split(|b| *b == b' ') {
        assert_eq!(check(DEVICE, 255, n, b""), NEVER);
    }
}

#[test]
fn a_device_needs_the_class() {
    assert_eq!(check(DEVICE, PHONE, b"panes", b""), OK);
    assert_eq!(check(DEVICE, PHONE, b"permit_answer", b""), OK);
    assert_eq!(check(DEVICE, PHONE, b"pane_input", b""), CLASS);
    assert_eq!(check(DEVICE, PHONE | TERMINAL, b"pane_input", b""), OK);
    assert_eq!(check(DEVICE, READ | REVIEW | ASK, b"review_work", b""), CLASS);
    assert_eq!(check(DEVICE, 0, b"panes", b""), CLASS);
}

#[test]
fn more_classes_never_refuse_more() {
    for n in names() {
        for a in 0..=255u8 {
            if check(DEVICE, a, n, b"https://github.com/o/r/pull/1") == OK {
                for b in 0..=255u8 {
                    assert_eq!(check(DEVICE, a | b, n, b"https://github.com/o/r/pull/1"), OK);
                }
            }
        }
    }
}

#[test]
fn review_open_takes_only_a_pr_url_from_a_device() {
    let ok = |u: &str| check(DEVICE, PHONE, b"review_open", u.as_bytes()) == OK;
    assert!(ok("https://github.com/o/r/pull/12"));
    assert!(ok("https://github.com/sigp/lighthouse/pull/7001/files"));
    assert!(ok("https://github.com/a-b/c.d_e/pull/1#discussion_r1"));
    assert!(ok("https://github.com/o/r/pull/1?x=1"));
    assert!(!ok(""));
    assert!(!ok("http://github.com/o/r/pull/1"));
    assert!(!ok("https://github.com.evil/o/r/pull/1"));
    assert!(!ok("https://github.com//r/pull/1"));
    assert!(!ok("https://github.com/o//pull/1"));
    assert!(!ok("https://github.com/o/r/pull/"));
    assert!(!ok("https://github.com/o/r/pull/1x"));
    assert!(!ok("https://github.com/o/r/issues/1"));
    assert!(!ok("https://github.com/o/r/pull/12345678901"));
    assert!(!ok("https://github.com/o"));
    assert!(!ok("https://github.com/o/r"));
    assert!(!ok("https://github.com/o/r/pull"));
    assert!(!ok("file:///etc/passwd"));
    // A trusted window keeps what the hub accepts.
    assert_eq!(check(TRUSTED, 0, b"review_open", b"github.com/o/r/pull/1"), OK);
}

/// No input panics: every prefix of hostile URLs.
#[test]
fn pr_url_ok_never_panics() {
    for u in ["https://github.com/o/r/pull/123/x", "https://github.com/\u{ff}/r", "https://github.com/o/r/pul"] {
        let b = u.as_bytes();
        for i in 0..=b.len() {
            let _ = pr_url_ok(&b[..i]);
        }
    }
}
