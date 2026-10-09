//! The send plan against GitHub's real diff of a PR. It reads only: nothing is sent.
//! It makes a draft on every changed line and checks that each one goes on its line.
//!   SB_REAL_REPO=~/Documents/Code/Ethereum/Consensus/lighthouse \
//!   SB_LIVE_PR=https://github.com/sigp/lighthouse/pull/10182 \
//!   cargo test -p switchboard-core --test live_post -- --ignored --nocapture

use std::path::PathBuf;
use switchboard_core::repos::git;
use switchboard_core::store::DraftRow;
use switchboard_core::{coverage, post, review};

#[test]
#[ignore]
fn every_changed_line_goes_on_its_line() {
    let repo = PathBuf::from(std::env::var("SB_REAL_REPO").expect("set SB_REAL_REPO"));
    let url = std::env::var("SB_LIVE_PR").expect("set SB_LIVE_PR");
    let pr = review::gh_view(&url).unwrap();
    let have = |oid: &str| git(&repo, &["cat-file", "-e", &format!("{oid}^{{commit}}")]).is_ok();
    if !have(&pr.head_ref_oid) || !have(&pr.base_ref_oid) {
        let remote = review::find_remote(&repo, &repo_name(&url)).expect("no remote for the PR");
        git(&repo, &["fetch", "-q", "--no-tags", &remote, &format!("pull/{}/head", pr.number), &pr.base_ref_name]).unwrap();
    }
    let base = git(&repo, &["merge-base", &pr.base_ref_oid, &pr.head_ref_oid]).unwrap();
    let m = coverage::build(&repo, &base, &pr.head_ref_oid).unwrap();
    let gh = post::parse_pr_diff(&review::gh_pr_diff(&repo_name(&url), pr.number).unwrap());

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
    println!(
        "{} files, {} drafts: {} on lines, {} into the summary, {} errors",
        m.files.len(),
        drafts.len(),
        p.inline.len(),
        p.outside.len(),
        p.errors.len()
    );
    for o in p.outside.iter().take(10) {
        println!("summary: {} ({})", o.at, o.reason);
    }
    assert!(p.errors.is_empty(), "{:?}", &p.errors[..p.errors.len().min(10)]);
    assert_eq!(p.inline.len(), drafts.len());
}

fn repo_name(url: &str) -> String {
    let parts: Vec<&str> = url.trim_end_matches('/').split('/').collect();
    format!("{}/{}", parts[parts.len() - 4], parts[parts.len() - 3])
}
