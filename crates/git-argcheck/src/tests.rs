use super::*;

/// The old check of `review_git.rs`, on `&str`, as a plain model.
mod model {
    fn list(t: &[u8]) -> Vec<&str> {
        std::str::from_utf8(t).unwrap().split(' ').collect()
    }

    fn has(t: &[u8], name: &str) -> bool {
        list(t).contains(&name)
    }

    fn digits(s: &str) -> bool {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
    }

    fn short_with_value(sub: &str, arg: &str) -> bool {
        let rest = &arg[1..];
        let mut chars = rest.chars();
        let Some(flag) = chars.next() else {
            return false;
        };
        let value = chars.as_str();
        if value.is_empty() {
            return false;
        }
        let diffs = matches!(sub, "diff" | "log" | "show");
        let logs = matches!(sub, "log" | "show");
        let blame = sub == "blame";
        match flag {
            'U' => diffs && digits(value),
            'M' | 'C' => (diffs || blame) && digits(value.trim_end_matches('%')),
            'l' => diffs && digits(value),
            'S' | 'G' => diffs,
            'L' => logs || blame,
            'n' => logs && digits(value),
            _ => false,
        }
    }

    pub fn refuse(sub: &str, arg: &str) -> bool {
        use crate::{BLAME_FLAGS, BLAME_VALUES, DIFF_FLAGS, DIFF_ONLY, DIFF_VALUES, LOG_FLAGS, LOG_VALUES, MERGE_BASE_FLAGS, REV_PARSE_FLAGS, REV_PARSE_VALUES};
        if arg.is_empty() || arg.contains('\0') || arg.contains('\n') {
            return true;
        }
        if arg == "--" {
            return false;
        }
        if let Some(name) = arg.strip_prefix("--") {
            let (name, value) = match name.split_once('=') {
                Some((n, v)) => (format!("--{n}"), Some(v)),
                None => (arg.to_owned(), None),
            };
            let n = name.as_str();
            let ok = match (sub, value) {
                ("diff", None) => has(DIFF_FLAGS, n) || has(DIFF_ONLY, n),
                ("diff", Some(_)) => has(DIFF_VALUES, n),
                ("log" | "show", None) => has(DIFF_FLAGS, n) || has(LOG_FLAGS, n),
                ("log" | "show", Some(_)) => has(DIFF_VALUES, n) || has(LOG_VALUES, n),
                ("blame", None) => has(BLAME_FLAGS, n),
                ("blame", Some(_)) => has(BLAME_VALUES, n),
                ("merge-base", None) => has(MERGE_BASE_FLAGS, n),
                ("rev-parse", None) => has(REV_PARSE_FLAGS, n),
                ("rev-parse", Some(_)) => has(REV_PARSE_VALUES, n),
                _ => false,
            };
            return !ok;
        }
        if let Some(short) = arg.strip_prefix('-') {
            let ok = match sub {
                "diff" => has(DIFF_FLAGS, arg),
                "log" | "show" => has(DIFF_FLAGS, arg) || has(LOG_FLAGS, arg) || (sub == "log" && digits(short)),
                "blame" => has(BLAME_FLAGS, arg),
                "merge-base" => has(MERGE_BASE_FLAGS, arg),
                "rev-parse" => has(REV_PARSE_FLAGS, arg),
                _ => false,
            } || short_with_value(sub, arg);
            return !ok;
        }
        if arg.starts_with('/') || arg.starts_with('~') {
            return true;
        }
        arg == ".." || arg.starts_with("../") || arg.contains("/../") || arg.ends_with("/..")
    }
}

const SUB_NAMES: [&str; 6] = ["diff", "log", "show", "blame", "merge-base", "rev-parse"];

fn ok(sub: &str, args: &[&str]) -> bool {
    let args: Vec<Vec<u8>> = args.iter().map(|a| a.as_bytes().to_vec()).collect();
    check(sub.as_bytes(), &args).is_ok()
}

#[test]
fn the_lists_have_one_space_between_names() {
    for t in [
        SUBS, DIFF_FLAGS, DIFF_VALUES, DIFF_ONLY, LOG_FLAGS, LOG_VALUES, BLAME_FLAGS, BLAME_VALUES,
        MERGE_BASE_FLAGS, REV_PARSE_FLAGS, REV_PARSE_VALUES,
    ] {
        let s = std::str::from_utf8(t).unwrap();
        assert!(s.split(' ').all(|w| !w.is_empty()), "{s}");
    }
    assert_eq!(std::str::from_utf8(SUBS).unwrap().split(' ').collect::<Vec<_>>(), SUB_NAMES);
}

#[test]
fn subcommands() {
    for (k, s) in SUB_NAMES.iter().enumerate() {
        assert_eq!(sub_code(s.as_bytes()), k as u8);
    }
    for s in ["", "dif", "diffs", "push", "-c", "Diff"] {
        assert_eq!(sub_code(s.as_bytes()), NO_SUB);
        assert!(!ok(s, &[]));
    }
}

#[test]
fn the_prefix() {
    let args = vec![b"HEAD".to_vec()];
    let out = |s: &str| check(s.as_bytes(), &args).ok().unwrap();
    assert_eq!(out("diff"), vec![b"--no-ext-diff".to_vec(), b"--no-textconv".to_vec(), b"HEAD".to_vec()]);
    assert_eq!(out("log").len(), 3);
    assert_eq!(out("show").len(), 3);
    assert_eq!(out("blame"), vec![b"--no-textconv".to_vec(), b"HEAD".to_vec()]);
    assert_eq!(out("rev-parse"), vec![b"HEAD".to_vec()]);
    assert_eq!(out("merge-base"), vec![b"HEAD".to_vec()]);
}

#[test]
fn refusals_name_the_arg() {
    let args: Vec<Vec<u8>> = ["HEAD", "-U0", "/etc/passwd", "x"].iter().map(|a| a.as_bytes().to_vec()).collect();
    let r = check(b"diff", &args).err().unwrap();
    assert_eq!((r.why, r.at), (PATH, 2));
    let r = check(b"push", &args).err().unwrap();
    assert_eq!((r.why, r.at), (BAD_SUB, 0));
    assert_eq!(refuse(DIFF, b""), EMPTY);
    assert_eq!(refuse(DIFF, b"a\nb"), CONTROL);
    assert_eq!(refuse(DIFF, b"a\0b"), CONTROL);
    assert_eq!(refuse(DIFF, b"--output=x"), OPTION);
    assert_eq!(refuse(DIFF, b"../x"), PATH);
}

#[test]
fn rel_ok_paths() {
    for p in ["a", "a/b", "a..b", "...", "a/.../b", ".", "./a", "a/", "HEAD..main", "x/y z.rs", ".hidden", "..a", "a.."] {
        assert!(rel_ok(p.as_bytes()), "{p}");
    }
    for p in ["", "/", "/a", "~", "~/a", "..", "../a", "a/..", "a/../b", "a\0b", "a//../b"] {
        assert!(!rel_ok(p.as_bytes()), "{p}");
    }
}

#[test]
fn digit_strings() {
    assert!(is_digits(b"0"));
    assert!(is_digits(b"0123456789"));
    assert!(!is_digits(b""));
    assert!(!is_digits(b"1a"));
    assert!(!is_digits(b"/"));
    assert!(!is_digits(b":"));
}

/// Every name of every list, and some pieces around them.
fn words() -> Vec<String> {
    let mut w = vec![
        "", "-", "--", "---", "-U", "-U0", "-U12", "-Ux", "-M", "-M50", "-M50%", "-M50%%", "-M%", "-C9", "-l3",
        "-Sx", "-G.*", "-L1,2:a", "-L", "-n5", "-n", "-5", "-55", "-c", "-cx", "-o", "-ox", "-O", "-Ox", "-pw",
        "-é", "-éx", "--=x", "--stat=", "--stat=3", "--text", "--textconv", "--output", "--output=x", "--out",
        "--no-index", "--ext-diff", "--exec-path=x", "--git-dir=x", "--contents", "a", "HEAD", "a..b", "a...b",
        "..", "../a", "a/..", "a/../b", "/a", "~a", "x\ny", "x\0y", "@{-1}", ":/fix", "HEAD:src/a.rs",
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<_>>();
    for t in [
        DIFF_FLAGS, DIFF_VALUES, DIFF_ONLY, LOG_FLAGS, LOG_VALUES, BLAME_FLAGS, BLAME_VALUES, MERGE_BASE_FLAGS,
        REV_PARSE_FLAGS, REV_PARSE_VALUES,
    ] {
        for n in std::str::from_utf8(t).unwrap().split(' ') {
            w.push(n.to_string());
            w.push(format!("{n}=1"));
            w.push(format!("{n}x"));
            w.push(n[..n.len() - 1].to_string());
        }
    }
    w
}

#[test]
fn agrees_with_the_old_check_on_every_name() {
    for sub in SUB_NAMES {
        let code = sub_code(sub.as_bytes());
        for a in words() {
            let old = model::refuse(sub, &a);
            assert_eq!(refuse(code, a.as_bytes()) != 0, old, "{sub} {a:?}");
        }
    }
}

#[test]
fn agrees_with_the_old_check_on_random_args() {
    // Bytes that matter to the check, so short random args hit every branch.
    let alphabet: &[u8] = b"-=%/.~0123456789UMClSGLncoOpwaxe\n ";
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let names = words();
    for _ in 0..200_000 {
        let sub = SUB_NAMES[(next() % 6) as usize];
        let mut a: Vec<u8> = Vec::new();
        if next() % 3 == 0 {
            a.extend_from_slice(names[(next() % names.len() as u64) as usize].as_bytes());
        }
        let n = next() % 6;
        for _ in 0..n {
            a.push(alphabet[(next() % alphabet.len() as u64) as usize]);
        }
        let s = String::from_utf8(a).unwrap();
        assert_eq!(refuse(sub_code(sub.as_bytes()), s.as_bytes()) != 0, model::refuse(sub, &s), "{sub} {s:?}");
    }
}

#[test]
fn normal_calls_pass() {
    assert!(ok("diff", &["-U0", "abc123", "HEAD"]));
    assert!(ok("diff", &["--stat", "main...HEAD", "--", "src/a.rs"]));
    assert!(ok("log", &["--oneline", "-n20", "-5", "--format=%H %s", "base..HEAD"]));
    assert!(ok("blame", &["-L", "10,20", "HEAD", "--", "src/lib.rs"]));
    assert!(!ok("diff", &["--", "--output=x"]));
    assert!(!ok("diff", &["--text", "--textconv"]));
}

mod refs {
    use crate::refs::*;

    fn repo(u: &str) -> Option<String> {
        let r = github_repo(u.as_bytes())?;
        Some(format!("{}/{}", &u[r.owner_start..r.owner_end], &u[r.name_start..r.name_end]))
    }

    #[test]
    fn refs() {
        for good in ["main", "origin/main", "HEAD~2", "a^{tree}", "é", &"x".repeat(REF_MAX)] {
            assert!(ref_ok(good.as_bytes()), "{good:?}");
        }
        for bad in ["", "-x", "--all", "a b", "a\tb", "a\nb", "a\0b", "a\x7fb", &"x".repeat(REF_MAX + 1)] {
            assert!(!ref_ok(bad.as_bytes()), "{bad:?}");
        }
    }

    #[test]
    fn remotes() {
        let r: Vec<Vec<u8>> = ["origin", "a", "a/b"].iter().map(|s| s.as_bytes().to_vec()).collect();
        assert_eq!(split_remote(b"origin/unstable", &r), Some(6));
        assert_eq!(split_remote(b"a/b/x", &r), Some(3));
        assert_eq!(split_remote(b"a/x/y", &r), Some(1));
        assert_eq!(split_remote(b"origin", &r), None);
        assert_eq!(split_remote(b"origin/", &r), None);
        assert_eq!(split_remote(b"originx/y", &r), None);
        assert_eq!(split_remote(b"a/b/", &r), Some(1));
    }

    #[test]
    fn github_urls() {
        for (u, want) in [
            ("git@github.com:eserilev/lighthouse.git", "eserilev/lighthouse"),
            ("https://github.com/sigp/lighthouse", "sigp/lighthouse"),
            ("https://github.com/sigp/lighthouse.git/", "sigp/lighthouse"),
            ("https://github.com/sigp/lighthouse/", "sigp/lighthouse"),
            ("http://GitHub.COM/a/b", "a/b"),
            ("git://github.com/a/b.git", "a/b"),
            ("ssh://git@github.com/a/b.git", "a/b"),
            ("ssh://git@github.com:22/a/b", "a/b"),
            ("https://github.com/a/b.c", "a/b.c"),
        ] {
            assert_eq!(repo(u).as_deref(), Some(want), "{u}");
        }
        for u in [
            "https://github.com.evil.io/sigp/lighthouse",
            "https://evil.io/github.com/a/b",
            "https://github.com/o/n/pulls",
            "https://github.com/o",
            "https://github.com/o/",
            "https://github.com//n",
            "https://github.com/o/n.git.git",
            "https://github.com/o/.git",
            "https://github.com/o/n//",
            "ssh://git@github.com:/a/b",
            "ssh://git@github.com:2x/a/b",
            "git@github.com/a/b",
            "/home/me/up",
            "https://gitlab.com/a/b",
            "github.com/a/b",
            "",
        ] {
            assert_eq!(repo(u), None, "{u}");
        }
    }

    /// `GitHubUrl` of `proofs/GitArgCheck/RefSpec.lean`, as a plain model: every way to
    /// read `u` as a prefix, the owner, `/`, the name, `.git` or not, `/` or not.
    fn model(u: &str) -> Vec<String> {
        let mut pres: Vec<usize> = vec![];
        let host = |h: &str| h.len() == 10 && h.eq_ignore_ascii_case("github.com");
        for (scheme, sep) in [("https://", "/"), ("http://", "/"), ("git://", "/"), ("git@", ":"), ("ssh://git@", "/")] {
            if let Some(rest) = u.strip_prefix(scheme) {
                if rest.get(..10).is_some_and(host) && rest[10..].starts_with(sep) {
                    pres.push(scheme.len() + 11);
                }
            }
        }
        if let Some(rest) = u.strip_prefix("ssh://git@") {
            if rest.get(..10).is_some_and(host) && rest[10..].starts_with(':') {
                let port = &rest[11..];
                if let Some(k) = port.find('/') {
                    if k > 0 && port[..k].bytes().all(|b| b.is_ascii_digit()) {
                        pres.push(10 + 11 + k + 1);
                    }
                }
            }
        }
        let mut out = vec![];
        for p in pres {
            let rest = &u[p..];
            for g in ["", ".git"] {
                for t in ["", "/"] {
                    let Some(mid) = rest.strip_suffix(t).and_then(|r| r.strip_suffix(g)) else { continue };
                    let parts: Vec<&str> = mid.split('/').collect();
                    if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() && !parts[1].ends_with(".git") {
                        out.push(format!("{}/{}", parts[0], parts[1]));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn github_repo_agrees_with_the_model() {
        let pieces = [
            "https://", "http://", "git://", "git@", "ssh://git@", "github.com", "GitHub.Com", "github.co", "evil.io",
            "/", "/", "/", ":", ":22", "2", "x", "a", "sigp", ".git", ".", "git", "-", "",
        ];
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut found = 0;
        for _ in 0..200_000 {
            let n = 2 + next() % 8;
            let mut u = String::new();
            if next() % 2 == 0 {
                u.push_str(["https://github.com/", "git@github.com:", "ssh://git@github.com:7/"][(next() % 3) as usize]);
            }
            for _ in 0..n {
                u.push_str(pieces[(next() % pieces.len() as u64) as usize]);
            }
            let want = model(&u);
            assert!(want.len() <= 1, "{u}: {want:?}");
            assert_eq!(repo(&u), want.first().cloned(), "{u}");
            found += want.len();
        }
        assert!(found > 1000, "{found}");
    }
}
