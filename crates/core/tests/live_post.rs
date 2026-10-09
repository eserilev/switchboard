//! The send plan against GitHub's real diff of a PR. It reads only: nothing is sent.
//! It makes a draft on every changed line and checks that each one goes on its line.
//!   SB_REAL_REPO=~/Documents/Code/Ethereum/Consensus/lighthouse \
//!   SB_LIVE_PR=https://github.com/sigp/lighthouse/pull/10182 \
//!   cargo test -p switchboard-core --test live_post -- --ignored --nocapture

use std::path::PathBuf;
use switchboard_core::repos::git;
use switchboard_core::store::DraftRow;
use switchboard_core::{coverage, post, review};

/// What one PR had, for the totals.
#[derive(Default)]
struct Seen {
    drafts: usize,
    files: usize,
    renames: usize,
    deletes: usize,
    binary: usize,
    odd_paths: usize,
}

/// Makes a draft on every changed line of one PR and checks the plan. Panics on a
/// line that does not go on its line.
fn check_pr(repo: &std::path::Path, url: &str) -> Seen {
    let pr = review::gh_view(url).unwrap();
    let have = |oid: &str| git(repo, &["cat-file", "-e", &format!("{oid}^{{commit}}")]).is_ok();
    if !have(&pr.head_ref_oid) || !have(&pr.base_ref_oid) {
        let remote = review::find_remote(repo, &repo_name(url)).expect("no remote for the PR");
        git(repo, &["fetch", "-q", "--no-tags", &remote, &format!("pull/{}/head", pr.number), &pr.base_ref_name]).unwrap();
    }
    let base = git(repo, &["merge-base", &pr.base_ref_oid, &pr.head_ref_oid]).unwrap();
    let m = coverage::build(repo, &base, &pr.head_ref_oid).unwrap();
    let gh = post::parse_pr_diff(&review::gh_pr_diff(&repo_name(url), pr.number).unwrap());

    let mut drafts = vec![];
    let mut id = 0;
    for f in &m.files {
        for (side, mask, path) in [("old", &f.removed, &f.old_path), ("new", &f.added, &f.new_path)] {
            for (k, changed) in mask.iter().enumerate() {
                if *changed {
                    id += 1;
                    drafts.push(DraftRow {
                        id,
                        path: path.clone(),
                        side: Some(side.into()),
                        line: Some(k as u32 + 1),
                        start_line: None,
                        text: "x".into(),
                        agent: false,
                    });
                }
            }
        }
    }
    let p = post::plan(&m, &gh, &drafts, true);
    assert!(p.errors.is_empty(), "#{}: {:?}", pr.number, &p.errors[..p.errors.len().min(10)]);
    assert!(p.outside.is_empty(), "#{}: {:?}", pr.number, &p.outside[..p.outside.len().min(10)]);
    assert_eq!(p.inline.len(), drafts.len(), "#{}", pr.number);
    let odd = |p: &str| p.contains(' ') || !p.is_ascii() || p.contains('"');
    Seen {
        drafts: drafts.len(),
        files: m.files.len(),
        renames: m.files.iter().filter(|f| f.old_path.is_some() && f.new_path.is_some() && f.old_path != f.new_path).count(),
        deletes: m.files.iter().filter(|f| f.new_path.is_none()).count(),
        binary: m.files.iter().filter(|f| f.binary).count(),
        odd_paths: m.files.iter().filter(|f| odd(f.path())).count(),
    }
}

#[test]
#[ignore]
fn every_changed_line_goes_on_its_line() {
    let repo = PathBuf::from(std::env::var("SB_REAL_REPO").expect("set SB_REAL_REPO"));
    let url = std::env::var("SB_LIVE_PR").expect("set SB_LIVE_PR");
    let s = check_pr(&repo, &url);
    println!("{} files, {} drafts, all on their lines", s.files, s.drafts);
}

/// The same check on many open PRs of a repo.
///   SB_REAL_REPO=~/Documents/Code/Ethereum/Consensus/lighthouse SB_POST_REPO=sigp/lighthouse \
///   SB_POST_COUNT=60 cargo test -p switchboard-core --test live_post -- --ignored --nocapture many_prs
#[test]
#[ignore]
fn many_prs() {
    let repo = PathBuf::from(std::env::var("SB_REAL_REPO").expect("set SB_REAL_REPO"));
    let name = std::env::var("SB_POST_REPO").unwrap_or_else(|_| "sigp/lighthouse".into());
    let count = std::env::var("SB_POST_COUNT").unwrap_or_else(|_| "60".into());
    // `open` or `merged`. Merged PRs have more renames.
    let state = std::env::var("SB_POST_STATE").unwrap_or_else(|_| "open".into());
    let out = std::process::Command::new("gh")
        .args(["pr", "list", "-R", &name, "--state", &state, "-L", &count, "--json", "url,isDraft"])
        .output()
        .unwrap();
    let prs: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut total = Seen::default();
    let mut n = 0;
    for pr in prs.as_array().unwrap() {
        let url = pr["url"].as_str().unwrap();
        let s = check_pr(&repo, url);
        total.drafts += s.drafts;
        total.files += s.files;
        total.renames += s.renames;
        total.deletes += s.deletes;
        total.binary += s.binary;
        total.odd_paths += s.odd_paths;
        n += 1;
    }
    println!(
        "{n} PRs: {} files, {} drafts on their lines; {} renames, {} deleted files, {} binary files, {} paths with spaces, quotes or non-ASCII",
        total.files, total.drafts, total.renames, total.deletes, total.binary, total.odd_paths
    );
}

fn repo_name(url: &str) -> String {
    let parts: Vec<&str> = url.trim_end_matches('/').split('/').collect();
    format!("{}/{}", parts[parts.len() - 4], parts[parts.len() - 3])
}

/// The duplicate check reads real reviews: each review of the PR with line comments
/// must match itself. It reads only.
///   SB_LIVE_PR=https://github.com/sigp/lighthouse/pull/10182 \
///   cargo test -p switchboard-core --test live_post -- --ignored --nocapture each_review_matches_itself
#[test]
#[ignore]
fn each_review_matches_itself() {
    let url = std::env::var("SB_LIVE_PR").expect("set SB_LIVE_PR");
    let pr = review::gh_view(&url).unwrap();
    let repo = repo_name(&url);
    let reviews = review::gh_reviews(&repo, pr.number).unwrap();
    let mut checked = 0;
    for r in reviews.as_array().unwrap() {
        let id = r["id"].as_u64().unwrap();
        let comments = review::gh_review_comments(&repo, pr.number, id).unwrap();
        if comments.as_array().unwrap().is_empty() {
            continue;
        }
        let event = match r["state"].as_str().unwrap() {
            "APPROVED" => "APPROVE",
            "CHANGES_REQUESTED" => "REQUEST_CHANGES",
            _ => "COMMENT",
        };
        let payload = serde_json::json!({
            "commit_id": r["commit_id"], "event": event, "body": r["body"],
            "comments": comments.as_array().unwrap().iter().map(|c| serde_json::json!({
                "path": c["path"], "side": c["side"], "body": c["body"],
                "line": if c["line"].is_null() { c["original_line"].clone() } else { c["line"].clone() },
            })).collect::<Vec<_>>(),
        });
        let login = r["user"]["login"].as_str().unwrap();
        let found = post::sent_candidates(&reviews, login, &payload);
        assert!(found.iter().any(|(f, _)| *f == id), "review {id} is not a candidate");
        assert!(post::same_comments(&comments, &payload), "review {id}");
        checked += 1;
    }
    println!("{checked} reviews with line comments match themselves");
    assert!(checked > 0);
}
