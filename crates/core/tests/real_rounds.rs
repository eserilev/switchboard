//! Review rounds on real PRs. For each open PR with several commits, a commit in the
//! middle of the PR plays the head of round 1. It reads GitHub and fetches PR commits
//! into the local clone; it sends nothing.
//!   SB_REAL_REPO=~/Documents/Code/Ethereum/Consensus/lighthouse SB_ROUND_REPO=sigp/lighthouse \
//!   cargo test -p switchboard-core --test real_rounds -- --ignored --nocapture

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;
use switchboard_core::repos::git;
use switchboard_core::{coverage, review};

fn gh_json(args: &[&str]) -> serde_json::Value {
    let out = Command::new("gh").args(args).output().unwrap();
    assert!(out.status.success(), "gh {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

/// The changed files of a model.
fn files(m: &coverage::DiffModel) -> BTreeSet<String> {
    m.files.iter().map(|f| f.path().to_owned()).collect()
}

#[test]
#[ignore]
fn rounds_show_only_the_later_changes() {
    let dir = PathBuf::from(std::env::var("SB_REAL_REPO").expect("set SB_REAL_REPO"));
    let repo = std::env::var("SB_ROUND_REPO").unwrap_or_else(|_| "sigp/lighthouse".into());
    let limit = std::env::var("SB_ROUND_COUNT").unwrap_or_else(|_| "40".into());
    let remote = review::find_remote(&dir, &repo).expect("no remote for the repo");
    let prs = gh_json(&["pr", "list", "-R", &repo, "--state", "open", "-L", &limit, "--json", "number,baseRefName,baseRefOid,headRefOid"]);
    let (mut plain, mut replayed, mut conflicts, mut skipped) = (0, 0, 0, 0);
    for pr in prs.as_array().unwrap() {
        let n = pr["number"].as_u64().unwrap();
        let head = pr["headRefOid"].as_str().unwrap();
        let base_tip = pr["baseRefOid"].as_str().unwrap();
        let have = |oid: &str| git(&dir, &["cat-file", "-e", &format!("{oid}^{{commit}}")]).is_ok();
        if !have(head) || !have(base_tip) {
            let base_ref = pr["baseRefName"].as_str().unwrap();
            if git(&dir, &["fetch", "-q", "--no-tags", &remote, &format!("pull/{n}/head"), base_ref]).is_err() {
                skipped += 1;
                continue;
            }
        }
        let Ok(new_base) = git(&dir, &["merge-base", base_tip, head]) else {
            skipped += 1;
            continue;
        };
        // The PR's own commits, oldest first, without the merges.
        let commits: Vec<String> = git(&dir, &["rev-list", "--reverse", "--no-merges", &format!("{new_base}..{head}")])
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        if commits.len() < 2 {
            skipped += 1;
            continue;
        }
        let since = &commits[commits.len() / 2 - 1];
        let (b, note) = review::round_base(&dir, since, &new_base, head).unwrap();
        let m = coverage::build(&dir, &b, head).unwrap_or_else(|e| panic!("#{n}: {e}"));
        coverage::rebuild_ok(&m).unwrap_or_else(|e| panic!("#{n}: {e}"));
        // The kernel accepts a completed guide of the round.
        let (g, _) = coverage::complete(&m, &serde_json::json!({"steps": [{"id": "s1", "title": "Round"}]})).unwrap();
        assert!(coverage::check_guide(&m, &g).is_ok(), "#{n}");
        // The files that the author's own commits after `since` change.
        let mut author: BTreeSet<String> = BTreeSet::new();
        // Commits of the base branch that a merge brought in are not the author's.
        let not_base = format!("^{new_base}");
        for c in git(&dir, &["rev-list", "--no-merges", &format!("{since}..{head}"), &not_base]).unwrap().lines() {
            for f in git(&dir, &["diff-tree", "--no-commit-id", "--name-only", "-r", "-M", c]).unwrap().lines() {
                author.insert(f.to_owned());
            }
        }
        match note {
            None => {
                // No rebase and no base merge: the round is the plain diff since round 1.
                assert_eq!(b, *since, "#{n}");
                let plain_files: BTreeSet<String> = git(&dir, &["diff", "--name-only", "-M", since, head]).unwrap().lines().map(str::to_owned).collect();
                assert_eq!(files(&m), plain_files, "#{n}");
                plain += 1;
            }
            Some(note) if !note.contains("conflict") => {
                // A base merge: no file outside the author's commits shows.
                let extra: Vec<String> = files(&m).difference(&author).cloned().collect();
                assert!(extra.is_empty(), "#{n}: files from the base branch show: {extra:?}");
                replayed += 1;
            }
            Some(_) => {
                // Only the files with a merge conflict can come from the base branch.
                let (_, conflicted) = review::replay(&dir, since, &new_base).unwrap();
                let conflicted: BTreeSet<String> = conflicted.into_iter().collect();
                let extra: Vec<String> = files(&m).difference(&author).filter(|f| !conflicted.contains(*f)).cloned().collect();
                assert!(extra.is_empty(), "#{n}: files from the base branch show: {extra:?}");
                println!("#{n}: {} conflicted files", conflicted.len());
                conflicts += 1;
            }
        }
        println!("#{n}: {} commits, since {}, {} files in the round", commits.len(), &since[..9], m.files.len());
    }
    println!("{plain} plain, {replayed} with a base merge or rebase, {conflicts} with conflicts, {skipped} skipped");
    assert!(plain + replayed > 0);
}
