//! The pending review on a real PR (SPEC 11.8.1). Both tests are ignored.
//!
//! Reads only:
//!   SB_LIVE_PR=https://github.com/sigp/lighthouse/pull/10242 \
//!   cargo test -p switchboard-core --test live_pending reads -- --ignored --nocapture
//!
//! WRITES to GitHub: it makes a pending review on the PR, adds, edits and deletes a
//! comment, and deletes the pending review. A pending review shows to you alone.
//! Use a PR of your own, with no pending review of yours on it:
//!   SB_LIVE_PR=https://github.com/you/repo/pull/1 SB_LIVE_PATH=src/lib.rs SB_LIVE_LINE=1 \
//!   cargo test -p switchboard-core --test live_pending writes -- --ignored --nocapture

use switchboard_core::gh::{Fate, Gh, RealGh};
use switchboard_core::post::Inline;
use switchboard_core::review;

fn pr() -> (String, review::PrRef) {
    let url = std::env::var("SB_LIVE_PR").expect("set SB_LIVE_PR");
    let r = review::parse_url(&url).expect("not a PR URL");
    (url, r)
}

#[test]
#[ignore]
fn reads_the_pending_review_and_the_diff_at_the_head() {
    let (url, r) = pr();
    let gh = RealGh;
    let info = gh.view(&url).unwrap();
    let p = gh.pending(&r.repo, r.number).unwrap();
    println!("PR node {}", p.pr_id);
    match &p.pending {
        Some(x) => println!("pending {} at {} with {} comments", x.id, x.commit, x.comments.len()),
        None => println!("no pending review"),
    }
    let diff = gh.diff_at(&r.repo, &url, &info.base_ref_name, &info.head_ref_oid).unwrap();
    let files = switchboard_core::post::parse_pr_diff(&diff);
    println!("diff at the head: {} files", files.len());
    assert!(!p.pr_id.is_empty());
}

/// Each mutation with an id that GitHub does not know: GitHub checks the query, finds
/// no node, and changes nothing. A query that GitHub refuses fails here.
#[test]
#[ignore]
fn reads_mutations_are_valid_queries() {
    let gh = RealGh;
    let none = "PRR_kwDOAAAAAAAAAAAAAAAAAA";
    let c = Inline { draft: 0, path: "a.rs".into(), side: "RIGHT".into(), line: 2, start_line: Some(1), text: "x".into() };
    let gone = |r: Result<String, String>| {
        let err = r.unwrap_err();
        assert!(err.contains("does not know") || err.contains("Could not resolve"), "{err}");
    };
    gone(gh.create_pending("PR_kwDOAAAAAAAAAAAAAAAAAA", "0000000000000000000000000000000000000000"));
    gone(gh.add_comment(none, &c));
    gone(gh.submit(none, "COMMENT", "x"));
    gone(gh.edit_comment(none, "x").map(|_| String::new()));
    gh.delete_comment(none).unwrap();
    gh.delete_review(none).unwrap();
    assert_eq!(gh.fate(none).unwrap(), Fate::Gone);
}

#[test]
#[ignore]
fn writes_makes_edits_and_deletes_a_pending_review() {
    let (url, r) = pr();
    let path = std::env::var("SB_LIVE_PATH").expect("set SB_LIVE_PATH");
    let line: u32 = std::env::var("SB_LIVE_LINE").expect("set SB_LIVE_LINE").parse().unwrap();
    let gh = RealGh;
    let info = gh.view(&url).unwrap();
    let p = gh.pending(&r.repo, r.number).unwrap();
    assert!(p.pending.is_none(), "you have a pending review on this PR already");
    let review = gh.create_pending(&p.pr_id, &info.head_ref_oid).unwrap();
    let c = Inline { draft: 0, path, side: "RIGHT".into(), line, start_line: None, text: "Switchboard live test".into() };
    let comment = gh.add_comment(&review, &c).unwrap();
    gh.edit_comment(&comment, "Switchboard live test, edited").unwrap();
    let now = gh.pending(&r.repo, r.number).unwrap().pending.unwrap();
    assert_eq!(now.id, review);
    assert_eq!(now.commit, info.head_ref_oid);
    assert_eq!(now.comments.len(), 1);
    assert_eq!(now.comments[0].body, "Switchboard live test, edited");
    let rest = gh.review_comments(&r.repo, r.number, now.db_id).unwrap();
    println!("REST: {rest}");
    assert_eq!(rest[0]["node_id"].as_str(), Some(comment.as_str()));
    gh.delete_comment(&comment).unwrap();
    assert!(gh.pending(&r.repo, r.number).unwrap().pending.unwrap().comments.is_empty());
    assert_eq!(gh.fate(&review).unwrap(), Fate::Pending);
    gh.delete_review(&review).unwrap();
    assert_eq!(gh.fate(&review).unwrap(), Fate::Gone);
    assert!(gh.pending(&r.repo, r.number).unwrap().pending.is_none());
}
