//! The diff model and the verified rebuild check on the real history of a repo.
//! It reads git only. Run it with a repo path:
//!   SB_REAL_REPO=~/Documents/Code/Ethereum/Consensus/lighthouse cargo test -p switchboard-core --test real_diffs -- --ignored --nocapture

use std::path::PathBuf;
use switchboard_core::coverage;
use switchboard_core::repos::git;

#[test]
#[ignore]
fn every_recent_commit_rebuilds_and_completes() {
    let repo = PathBuf::from(std::env::var("SB_REAL_REPO").expect("set SB_REAL_REPO"));
    let n: usize = std::env::var("SB_REAL_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    let log = git(
        &repo,
        &[
            "log",
            "--first-parent",
            "--format=%H",
            "-n",
            &n.to_string(),
            "HEAD",
        ],
    )
    .unwrap();
    let (mut files, mut lines, mut renames, mut binary) = (0, 0, 0, 0);
    let t0 = std::time::Instant::now();
    for c in log.lines() {
        let parent = format!("{c}^");
        let Ok(base) = git(&repo, &["rev-parse", &parent]) else {
            continue;
        };
        let m = coverage::build(&repo, &base, c).unwrap_or_else(|e| panic!("{c}: {e}"));
        coverage::rebuild_ok(&m).unwrap_or_else(|e| panic!("{c}: {e}"));
        // `complete` on an empty guide must give a guide the kernel accepts.
        let (g, missed) = coverage::complete(
            &m,
            &serde_json::json!({"steps": [{"id": "s1", "title": "PR"}]}),
        )
        .unwrap_or_else(|e| panic!("{c}: {e}"));
        assert!(coverage::check_guide(&m, &g).is_ok(), "{c}");
        // The rows the window draws rebuild both versions of every file.
        for f in m.files.iter().filter(|f| !f.needs_name()) {
            let all = coverage::rows(f, usize::MAX);
            let strip = |ls: &Vec<Vec<u8>>| {
                ls.iter().map(|l| String::from_utf8_lossy(l.strip_suffix(b"\n").unwrap_or(l)).into_owned()).collect::<Vec<_>>()
            };
            let new: Vec<String> = all.iter().filter(|r| r.kind == '+' || r.kind == ' ').map(|r| r.text.clone()).collect();
            let old: Vec<String> = all.iter().filter(|r| r.kind == '-' || r.kind == ' ').map(|r| r.text.clone()).collect();
            assert_eq!(new, strip(&f.new), "{c} {}", f.path());
            assert_eq!(old, strip(&f.old), "{c} {}", f.path());
            // In the cut that the window draws, every number names the line that the row shows.
            let (old, new) = (strip(&f.old), strip(&f.new));
            for r in coverage::rows(f, 12).iter().filter(|r| r.kind != '@') {
                if let Some(n) = r.old {
                    assert_eq!(old[n as usize - 1], r.text, "{c} {}", f.path());
                }
                if let Some(n) = r.new {
                    assert_eq!(new[n as usize - 1], r.text, "{c} {}", f.path());
                }
            }
        }
        // Each range of the guide as its own step: the step rows are rows of the full
        // diff, each header names the next row, and together the steps show every change.
        let ranges: Vec<coverage::Range> = g["steps"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(coverage::step_ranges)
            .collect();
        for f in m.files.iter().filter(|f| !f.needs_name()) {
            let key = |r: &switchboard_core::diff::Row| (r.kind, r.old, r.new);
            let full: std::collections::HashSet<_> = coverage::rows(f, usize::MAX).iter().filter(|r| r.kind != '@').map(key).collect();
            let mut seen = std::collections::HashSet::new();
            for g in ranges.iter().filter(|g| {
                let p = if g.side == "old" { &f.old_path } else { &f.new_path };
                p.as_deref() == Some(g.file.as_str())
            }) {
                let (rows, _) = coverage::step_rows(f, std::slice::from_ref(g), 6);
                for (k, r) in rows.iter().enumerate() {
                    if r.kind == '@' {
                        if let Some(q) = rows.get(k + 1).filter(|q| q.kind != '@') {
                            let want = format!("@@ old {} · new {} @@", q.old.map_or_else(|| "?".into(), |n| n.to_string()), q.new.map_or_else(|| "?".into(), |n| n.to_string()));
                            let (o, n) = (q.old.is_some(), q.new.is_some());
                            let parts: Vec<&str> = r.text.split(' ').collect();
                            assert!(!o || parts[2] == want.split(' ').nth(2).unwrap(), "{c} {} {} vs {want}", f.path(), r.text);
                            assert!(!n || parts[5] == want.split(' ').nth(5).unwrap(), "{c} {} {} vs {want}", f.path(), r.text);
                        }
                        continue;
                    }
                    assert!(full.contains(&key(r)), "{c} {}: {r:?}", f.path());
                    if r.kind != ' ' {
                        seen.insert(key(r));
                    }
                }
            }
            assert_eq!(seen.len(), f.changed_lines(), "{c} {}: the steps show every change", f.path());
        }
        let changed: usize = m.files.iter().map(|f| f.changed_lines()).sum();
        assert_eq!(
            missed, changed,
            "{c}: every changed line goes into the added step"
        );
        files += m.files.len();
        lines += changed;
        renames += m
            .files
            .iter()
            .filter(|f| f.old_path.is_some() && f.new_path.is_some() && f.old_path != f.new_path)
            .count();
        binary += m.files.iter().filter(|f| f.binary).count();
    }
    println!(
        "{} commits: {files} files, {lines} changed lines, {renames} renames, {binary} binary files, {:?}",
        log.lines().count(),
        t0.elapsed()
    );
}
