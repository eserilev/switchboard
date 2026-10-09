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
