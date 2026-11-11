//! The GitHub calls of a send and of the pending review (SPEC 11.8, 11.8.1). One
//! trait, so a test can give the hub a fake GitHub (`Hub::debug_set_gh`).

use crate::post::{GhComment, Inline};
use crate::review::{self, PrInfo};
use serde_json::{json, Value};
use std::process::{Command, Stdio};

type Res<T> = Result<T, String>;

/// Your pending review on a PR.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    /// The GraphQL node id.
    pub id: String,
    /// The REST id, for the REST list of its comments.
    pub db_id: u64,
    pub commit: String,
    pub comments: Vec<GhComment>,
}

/// The PR's node id, and your pending review on it.
#[derive(Debug, Clone, PartialEq)]
pub struct PrPending {
    pub pr_id: String,
    pub pending: Option<Pending>,
}

/// What became of a pending review that the app made.
#[derive(Debug, Clone, PartialEq)]
pub enum Fate {
    /// Still pending.
    Pending,
    /// Sent, with this state (`COMMENTED`, `APPROVED`, `CHANGES_REQUESTED`) and link.
    Sent { state: String, url: String },
    /// Deleted, or GitHub does not know it.
    Gone,
}

pub trait Gh: Send + Sync {
    fn view(&self, url: &str) -> Res<PrInfo>;
    fn pr_diff(&self, repo: &str, number: u64) -> Res<String>;
    /// GitHub's diff of the PR at commit `head`, on the base branch `base`.
    fn diff_at(&self, repo: &str, url: &str, base: &str, head: &str) -> Res<String>;
    fn login(&self) -> Res<String>;
    /// The reviews of the PR (REST).
    fn reviews(&self, repo: &str, number: u64) -> Res<Value>;
    /// The comments of one review (REST): with `path`, `side`, `line`, `start_line`,
    /// `start_side`, `body`, `node_id` and `in_reply_to_id`.
    fn review_comments(&self, repo: &str, number: u64, review: u64) -> Res<Value>;
    /// One REST call that sends a whole review. Returns its link.
    fn submit_rest(&self, repo: &str, number: u64, payload: &Value) -> Res<String>;
    fn pending(&self, repo: &str, number: u64) -> Res<PrPending>;
    fn fate(&self, review: &str) -> Res<Fate>;
    /// Makes a pending review at `commit`. Returns its node id.
    fn create_pending(&self, pr_id: &str, commit: &str) -> Res<String>;
    /// Adds a comment to a pending review. Returns the comment's node id.
    fn add_comment(&self, review: &str, c: &Inline) -> Res<String>;
    fn edit_comment(&self, comment: &str, body: &str) -> Res<()>;
    fn delete_comment(&self, comment: &str) -> Res<()>;
    fn delete_review(&self, review: &str) -> Res<()>;
    /// Sends a pending review. Returns its link.
    fn submit(&self, review: &str, event: &str, body: &str) -> Res<String>;
}

/// GitHub through the `gh` CLI.
pub struct RealGh;

/// Runs one GraphQL request. Returns `data`. A `NOT_FOUND` error comes back as
/// `Ok(None)`, so a deleted node is not a failure.
fn graphql(query: &str, vars: Value) -> Res<Option<Value>> {
    use std::io::Write;
    let input = json!({ "query": query, "variables": vars }).to_string();
    let mut child = Command::new("gh")
        .args(["api", "graphql", "--input", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("gh: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes()).map_err(|e| format!("gh: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("gh: {e}"))?;
    let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    if err.contains("auth login") || err.contains("not logged") {
        return Err("gh is not logged in. Run: gh auth login".into());
    }
    let body: Value = serde_json::from_slice(&out.stdout).map_err(|_| format!("gh api graphql: {err}"))?;
    if let Some(errors) = body["errors"].as_array().filter(|e| !e.is_empty()) {
        if errors.iter().all(|e| e["type"] == "NOT_FOUND") {
            return Ok(None);
        }
        let why: Vec<String> = errors.iter().map(|e| e["message"].as_str().unwrap_or("error").to_owned()).collect();
        return Err(format!("GitHub: {}", why.join("; ")));
    }
    if !out.status.success() {
        return Err(format!("gh api graphql: {err}"));
    }
    Ok(Some(body["data"].clone()))
}

fn need(v: Option<Value>, what: &str) -> Res<Value> {
    v.ok_or_else(|| format!("GitHub does not know {what}"))
}

fn comments_of(conn: &Value) -> Vec<GhComment> {
    conn["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| GhComment {
            id: c["id"].as_str().unwrap_or("").to_owned(),
            body: c["body"].as_str().unwrap_or("").to_owned(),
            reply: !c["replyTo"].is_null(),
        })
        .collect()
}

const COMMENTS: &str = "comments(first: 100, after: $after) { nodes { id body replyTo { id } } pageInfo { hasNextPage endCursor } }";

impl Gh for RealGh {
    fn view(&self, url: &str) -> Res<PrInfo> {
        review::gh_view(url)
    }

    fn pr_diff(&self, repo: &str, number: u64) -> Res<String> {
        review::gh_pr_diff(repo, number)
    }

    fn diff_at(&self, repo: &str, url: &str, base: &str, head: &str) -> Res<String> {
        review::gh_compare_diff(repo, review::gh_head_repo(url).as_deref(), base, head)
    }

    fn login(&self) -> Res<String> {
        review::gh_login()
    }

    fn reviews(&self, repo: &str, number: u64) -> Res<Value> {
        review::gh_reviews(repo, number)
    }

    fn review_comments(&self, repo: &str, number: u64, review: u64) -> Res<Value> {
        review::gh_review_comments(repo, number, review)
    }

    fn submit_rest(&self, repo: &str, number: u64, payload: &Value) -> Res<String> {
        review::gh_submit_review(repo, number, payload)
    }

    fn pending(&self, repo: &str, number: u64) -> Res<PrPending> {
        let (owner, name) = repo.split_once('/').ok_or(format!("not a repo: {repo}"))?;
        let q = format!(
            "query($owner: String!, $name: String!, $number: Int!, $after: String) {{ repository(owner: $owner, name: $name) {{ pullRequest(number: $number) {{ id reviews(states: [PENDING], first: 10) {{ nodes {{ id fullDatabaseId viewerDidAuthor commit {{ oid }} {COMMENTS} }} }} }} }} }}"
        );
        let data = need(graphql(&q, json!({ "owner": owner, "name": name, "number": number, "after": null }))?, repo)?;
        let pr = &data["repository"]["pullRequest"];
        let pr_id = pr["id"].as_str().ok_or(format!("GitHub has no PR {repo}#{number}"))?.to_owned();
        let Some(r) = pr["reviews"]["nodes"].as_array().into_iter().flatten().find(|r| r["viewerDidAuthor"] == true) else {
            return Ok(PrPending { pr_id, pending: None });
        };
        let id = r["id"].as_str().unwrap_or("").to_owned();
        let mut comments = comments_of(&r["comments"]);
        let mut page = r["comments"]["pageInfo"].clone();
        // More than 100 comments: the next pages, from the review node.
        while page["hasNextPage"] == true {
            let q = format!("query($id: ID!, $after: String) {{ node(id: $id) {{ ... on PullRequestReview {{ {COMMENTS} }} }} }}");
            let data = need(graphql(&q, json!({ "id": id, "after": page["endCursor"] }))?, "the pending review")?;
            comments.extend(comments_of(&data["node"]["comments"]));
            page = data["node"]["comments"]["pageInfo"].clone();
        }
        let db_id = r["fullDatabaseId"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .or_else(|| r["fullDatabaseId"].as_u64())
            .unwrap_or(0);
        Ok(PrPending {
            pr_id,
            pending: Some(Pending {
                id,
                db_id,
                commit: r["commit"]["oid"].as_str().unwrap_or("").to_owned(),
                comments,
            }),
        })
    }

    fn fate(&self, review: &str) -> Res<Fate> {
        let q = "query($id: ID!) { node(id: $id) { ... on PullRequestReview { state url } } }";
        let Some(data) = graphql(q, json!({ "id": review }))? else {
            return Ok(Fate::Gone);
        };
        let node = &data["node"];
        Ok(match node["state"].as_str() {
            None => Fate::Gone,
            Some("PENDING") => Fate::Pending,
            Some(state) => Fate::Sent {
                state: state.to_owned(),
                url: node["url"].as_str().unwrap_or("").to_owned(),
            },
        })
    }

    fn create_pending(&self, pr_id: &str, commit: &str) -> Res<String> {
        let q = "mutation($pr: ID!, $commit: GitObjectID!) { addPullRequestReview(input: { pullRequestId: $pr, commitOID: $commit }) { pullRequestReview { id } } }";
        let data = need(graphql(q, json!({ "pr": pr_id, "commit": commit }))?, "the PR")?;
        data["addPullRequestReview"]["pullRequestReview"]["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or("GitHub made no pending review".into())
    }

    fn add_comment(&self, review: &str, c: &Inline) -> Res<String> {
        let q = "mutation($review: ID!, $path: String!, $body: String!, $line: Int!, $side: DiffSide!, $start: Int, $startSide: DiffSide) { addPullRequestReviewThread(input: { pullRequestReviewId: $review, path: $path, body: $body, line: $line, side: $side, startLine: $start, startSide: $startSide }) { thread { comments(first: 1) { nodes { id } } } } }";
        let vars = json!({
            "review": review,
            "path": c.path,
            "body": c.text,
            "line": c.line,
            "side": c.side,
            "start": c.start_line,
            "startSide": c.start_line.map(|_| c.side.clone()),
        });
        let data = need(graphql(q, vars)?, "the pending review")?;
        data["addPullRequestReviewThread"]["thread"]["comments"]["nodes"][0]["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or(format!("GitHub did not take the comment on {}:{}", c.path, c.line))
    }

    fn edit_comment(&self, comment: &str, body: &str) -> Res<()> {
        let q = "mutation($id: ID!, $body: String!) { updatePullRequestReviewComment(input: { pullRequestReviewCommentId: $id, body: $body }) { clientMutationId } }";
        need(graphql(q, json!({ "id": comment, "body": body }))?, "the comment").map(|_| ())
    }

    fn delete_comment(&self, comment: &str) -> Res<()> {
        let q = "mutation($id: ID!) { deletePullRequestReviewComment(input: { id: $id }) { clientMutationId } }";
        // A comment that is gone already is what the delete wants.
        graphql(q, json!({ "id": comment })).map(|_| ())
    }

    fn delete_review(&self, review: &str) -> Res<()> {
        let q = "mutation($id: ID!) { deletePullRequestReview(input: { pullRequestReviewId: $id }) { clientMutationId } }";
        graphql(q, json!({ "id": review })).map(|_| ())
    }

    fn submit(&self, review: &str, event: &str, body: &str) -> Res<String> {
        let q = "mutation($id: ID!, $event: PullRequestReviewEvent!, $body: String) { submitPullRequestReview(input: { pullRequestReviewId: $id, event: $event, body: $body }) { pullRequestReview { url } } }";
        let data = need(graphql(q, json!({ "id": review, "event": event, "body": body }))?, "the pending review")?;
        Ok(data["submitPullRequestReview"]["pullRequestReview"]["url"].as_str().unwrap_or("").to_owned())
    }
}
