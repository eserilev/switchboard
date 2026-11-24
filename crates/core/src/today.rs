//! Today (SPEC 39): what needs you on GitHub, ranked. The rules here are pure:
//! fetched PR JSON, the config and the store rows in, the list out. No network.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The `[today]` block of the config (SPEC 39.3).
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct TodayCfg {
    /// Your GitHub login. With none, the login of `gh`.
    pub user: Option<String>,
    /// 0: only when you press refresh.
    pub refresh_minutes: u64,
    /// Added to the built-in list of bots.
    pub ignore_bots: Vec<String>,
    /// A review asked of one of these teams (`org/team`) counts as yours.
    pub teams: Vec<String>,
    /// `owner/name` of each repo Today reads. Empty: every repo. Your fork is a
    /// different repo: leave it out to see only the upstream.
    pub only: Vec<String>,
    pub jobs: BTreeMap<String, JobCfg>,
    pub repos: BTreeMap<String, RepoRules>,
}

impl Default for TodayCfg {
    fn default() -> Self {
        TodayCfg { user: None, refresh_minutes: 10, ignore_bots: vec![], teams: vec![], only: vec![], jobs: BTreeMap::new(), repos: BTreeMap::new() }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields, default)]
pub struct JobCfg {
    /// `owner/name` for each repo.
    pub repos: Vec<String>,
    /// What the job reads: "prs" and "issues" (SPEC 39.13). Default: PRs.
    pub read: Vec<String>,
    /// "mine" (default): items that involve you. "all": every open item of the repos.
    pub scope: String,
}

impl JobCfg {
    pub fn issues(&self) -> bool {
        self.read.iter().any(|r| r == "issues")
    }
    pub fn prs(&self) -> bool {
        self.read.is_empty() || self.read.iter().any(|r| r == "prs")
    }
    pub fn all(&self) -> bool {
        self.scope == "all"
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct RepoRules {
    pub release_labels: Vec<String>,
    pub tiny_lines: u64,
    pub no_review_days: u64,
    pub stale_days: u64,
    pub waiting_labels: Vec<String>,
    pub stale_labels: Vec<String>,
    pub areas: BTreeMap<String, AreaCfg>,
    /// "code" (default) or "reports": bug reports with severity labels (SPEC 39.13).
    pub kind: String,
    /// Severity labels, the worst first. A worse one ranks a report higher.
    pub severity: Vec<String>,
    /// An item with one of these labels never shows.
    pub hide_labels: Vec<String>,
    /// Labels shown as a tag on the row.
    pub tag_labels: Vec<String>,
    /// A report with no team reply after this many days is late.
    pub reply_days: u64,
}

impl Default for RepoRules {
    fn default() -> Self {
        RepoRules {
            release_labels: vec![],
            tiny_lines: 30,
            no_review_days: 3,
            stale_days: 14,
            waiting_labels: vec!["waiting-on-author".into()],
            stale_labels: vec!["stale".into()],
            areas: BTreeMap::new(),
            kind: "code".into(),
            severity: vec![],
            hide_labels: vec![],
            tag_labels: vec![],
            reply_days: 2,
        }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields, default)]
pub struct AreaCfg {
    pub labels: Vec<String>,
    /// A prefix of a changed file, or a glob with `*` and `**`.
    pub paths: Vec<String>,
}

/// The job with no repos: every repo.
pub const EVERYTHING: &str = "Everything";

/// Bots that never count as a person.
const BOTS: &[&str] = &["github-actions", "dependabot", "mergify", "cla-assistant", "codecov", "netlify", "vercel", "copilot"];

impl TodayCfg {
    pub fn rules(&self, repo: &str) -> RepoRules {
        self.repos.iter().find(|(k, _)| k.eq_ignore_ascii_case(repo)).map(|(_, v)| v.clone()).unwrap_or_default()
    }

    /// Everything first, then the jobs by name.
    pub fn job_names(&self) -> Vec<String> {
        let mut v = vec![EVERYTHING.to_owned()];
        v.extend(self.jobs.keys().filter(|k| k.as_str() != EVERYTHING).cloned());
        v
    }

    /// The repos of a job, in lower case. Empty: every repo.
    pub fn job_repos(&self, job: &str) -> Vec<String> {
        self.jobs.get(job).map(|j| j.repos.iter().map(|r| r.to_lowercase()).collect()).unwrap_or_default()
    }

    /// The repos that have areas, for the area search. With `only`, only those in it.
    pub fn area_repos(&self) -> Vec<String> {
        self.repos.iter().filter(|(k, r)| !r.areas.is_empty() && self.reads(k)).map(|(k, _)| k.clone()).collect()
    }

    /// True when Today reads this repo: `only` is empty or holds it, or a job
    /// names it (SPEC 39.13.1).
    pub fn reads(&self, repo: &str) -> bool {
        self.only.is_empty() || self.only.iter().any(|r| r.trim().eq_ignore_ascii_case(repo)) || self.jobs.values().any(|j| j.repos.iter().any(|r| r.trim().eq_ignore_ascii_case(repo)))
    }

    /// The repos that the searches name with `only`: its own, and every job's.
    pub fn limit_repos(&self) -> Vec<String> {
        if self.only.is_empty() {
            return vec![];
        }
        let mut v: Vec<String> = self.only.iter().chain(self.jobs.values().flat_map(|j| j.repos.iter())).map(|r| r.trim().to_owned()).filter(|r| !r.is_empty()).collect();
        v.sort_by_key(|r| r.to_lowercase());
        v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        v
    }

    pub fn is_bot(&self, login: &str) -> bool {
        let l = login.to_lowercase();
        if l.is_empty() || l.ends_with("[bot]") || l.ends_with("-bot") {
            return true;
        }
        BOTS.contains(&l.as_str()) || self.ignore_bots.iter().any(|b| b.eq_ignore_ascii_case(&l) || b.to_lowercase().trim_end_matches("[bot]") == l)
    }
}

// ---------- the PR, from GitHub's answer ----------

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Ev {
    pub author: String,
    pub at: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Review {
    pub author: String,
    pub state: String,
    pub at: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Comment {
    pub author: String,
    pub at: u64,
    pub body: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Thread {
    pub resolved: bool,
    pub last_author: String,
    pub last_at: u64,
    pub path: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Check {
    pub name: String,
    /// SUCCESS, FAILURE, ERROR, TIMED_OUT, ..., or PENDING while it runs.
    pub conclusion: String,
    pub at: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Pr {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub draft: bool,
    pub author: String,
    pub labels: Vec<String>,
    pub milestone: Option<String>,
    pub size: u64,
    /// MERGEABLE, CONFLICTING or UNKNOWN.
    pub mergeable: String,
    /// APPROVED, CHANGES_REQUESTED, REVIEW_REQUIRED or empty.
    pub decision: String,
    pub base: String,
    /// OPEN, MERGED or CLOSED.
    pub state: String,
    pub created: u64,
    pub updated: u64,
    pub closed: Option<u64>,
    pub in_queue: bool,
    /// Logins and `org/team` slugs, with the time of the request when known.
    pub requests: Vec<Ev>,
    pub reviews: Vec<Review>,
    pub commits: Vec<Ev>,
    pub comments: Vec<Comment>,
    pub threads: Vec<Thread>,
    /// SUCCESS, FAILURE, PENDING, ERROR or empty, for all checks.
    pub checks_state: String,
    pub checks: Vec<Check>,
    pub files: Vec<String>,
}

fn time(v: &Value) -> u64 {
    v.as_str().and_then(crate::chat::epoch).unwrap_or(0)
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_owned()
}

fn nodes(v: &Value) -> impl Iterator<Item = &Value> {
    v["nodes"].as_array().into_iter().flatten().filter(|n| !n.is_null())
}

/// The fields of a PR that the rules read (SPEC 39.6).
pub const PR_FIELDS: &str = "number title url isDraft state createdAt updatedAt closedAt
  author { login } repository { nameWithOwner }
  labels(first: 20) { nodes { name } } milestone { title }
  additions deletions mergeable reviewDecision baseRefName
  mergeQueueEntry { id }
  reviewRequests(first: 20) { nodes { requestedReviewer { ... on User { login } ... on Team { combinedSlug } } } }
  timelineItems(itemTypes: [REVIEW_REQUESTED_EVENT], last: 10) { nodes { ... on ReviewRequestedEvent { createdAt requestedReviewer { ... on User { login } ... on Team { combinedSlug } } } } }
  latestReviews(first: 20) { nodes { author { login } state submittedAt } }
  commits(last: 10) { nodes { commit { committedDate authors(first: 1) { nodes { user { login } } } statusCheckRollup { state contexts(first: 40) { nodes { ... on CheckRun { name conclusion status completedAt } ... on StatusContext { context state createdAt } } } } } } }
  comments(last: 15) { nodes { author { login } createdAt body } }
  reviewThreads(last: 25) { nodes { isResolved path comments(last: 1) { nodes { author { login } createdAt } } } }
  files(first: 60) { nodes { path } }";

/// The search for one bucket. `$q` is the search text.
pub fn search_query() -> String {
    search_query_of(25)
}

/// The same with `n` PRs: a retry after a GitHub timeout asks for fewer.
pub fn search_query_of(n: u32) -> String {
    format!("query($q: String!) {{ search(query: $q, type: ISSUE, first: {n}) {{ nodes {{ ... on PullRequest {{ {PR_FIELDS} }} }} }} }}")
}

/// One PR from a search node. `None` for a node that is not a PR.
pub fn parse_pr(n: &Value) -> Option<Pr> {
    let number = n["number"].as_u64()?;
    let repo = n["repository"]["nameWithOwner"].as_str()?.to_owned();
    let login = |v: &Value| -> String {
        let r = &v["requestedReviewer"];
        r["login"].as_str().or_else(|| r["combinedSlug"].as_str()).unwrap_or("").to_owned()
    };
    let asked_at: HashMap<String, u64> = nodes(&n["timelineItems"]).map(|e| (login(e).to_lowercase(), time(&e["createdAt"]))).collect();
    let requests = nodes(&n["reviewRequests"])
        .map(|r| {
            let who = login(r);
            let at = asked_at.get(&who.to_lowercase()).copied().unwrap_or(0);
            Ev { author: who, at }
        })
        .filter(|e| !e.author.is_empty())
        .collect();
    let commits_nodes: Vec<&Value> = nodes(&n["commits"]).collect();
    let commits = commits_nodes
        .iter()
        .map(|c| Ev {
            author: c["commit"]["authors"]["nodes"][0]["user"]["login"].as_str().unwrap_or("").to_owned(),
            at: time(&c["commit"]["committedDate"]),
        })
        .collect();
    let rollup = commits_nodes.last().map(|c| c["commit"]["statusCheckRollup"].clone()).unwrap_or(Value::Null);
    let checks = nodes(&rollup["contexts"])
        .map(|c| {
            if c.get("context").is_some() {
                Check { name: s(&c["context"]), conclusion: s(&c["state"]), at: time(&c["createdAt"]) }
            } else {
                let done = c["status"].as_str() == Some("COMPLETED");
                Check { name: s(&c["name"]), conclusion: if done { s(&c["conclusion"]) } else { "PENDING".into() }, at: time(&c["completedAt"]) }
            }
        })
        .collect();
    Some(Pr {
        repo,
        number,
        title: s(&n["title"]),
        url: s(&n["url"]),
        draft: n["isDraft"].as_bool().unwrap_or(false),
        author: s(&n["author"]["login"]),
        labels: nodes(&n["labels"]).map(|l| s(&l["name"])).collect(),
        milestone: n["milestone"]["title"].as_str().map(str::to_owned),
        size: n["additions"].as_u64().unwrap_or(0) + n["deletions"].as_u64().unwrap_or(0),
        mergeable: s(&n["mergeable"]),
        decision: s(&n["reviewDecision"]),
        base: s(&n["baseRefName"]),
        state: s(&n["state"]),
        created: time(&n["createdAt"]),
        updated: time(&n["updatedAt"]),
        closed: n["closedAt"].as_str().and_then(crate::chat::epoch),
        in_queue: !n["mergeQueueEntry"].is_null(),
        requests,
        reviews: nodes(&n["latestReviews"])
            .map(|r| Review { author: s(&r["author"]["login"]), state: s(&r["state"]), at: time(&r["submittedAt"]) })
            .collect(),
        commits,
        comments: nodes(&n["comments"])
            .map(|c| Comment { author: s(&c["author"]["login"]), at: time(&c["createdAt"]), body: s(&c["body"]) })
            .collect(),
        threads: nodes(&n["reviewThreads"])
            .map(|t| {
                let last = nodes(&t["comments"]).last().cloned().unwrap_or(Value::Null);
                Thread {
                    resolved: t["isResolved"].as_bool().unwrap_or(false),
                    last_author: s(&last["author"]["login"]),
                    last_at: time(&last["createdAt"]),
                    path: t["path"].as_str().map(str::to_owned),
                }
            })
            .collect(),
        checks_state: s(&rollup["state"]),
        checks,
        files: nodes(&n["files"]).map(|f| s(&f["path"])).collect(),
    })
}

/// The PRs of a search answer (`data`).
pub fn parse_search(data: &Value) -> Vec<Pr> {
    nodes(&data["search"]).filter_map(parse_pr).collect()
}

// ---------- the list ----------

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Section {
    Mine,
    Reviews,
    Area,
    /// Issues assigned to you, or asking you by name (SPEC 39.13.3).
    Assigned,
    Triage,
    Waiting,
    Poc,
    Changed,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Mine => "Your PRs",
            Section::Reviews => "Reviews",
            Section::Area => "In your area",
            Section::Assigned => "For you",
            Section::Triage => "Needs triage",
            Section::Waiting => "Waiting on the team",
            Section::Poc => "Proofs of concept",
            Section::Changed => "What changed",
        }
    }
    pub const ALL: [Section; 8] = [Section::Mine, Section::Reviews, Section::Area, Section::Assigned, Section::Triage, Section::Waiting, Section::Poc, Section::Changed];
    /// The sections that a tab counts: what waits on you.
    pub fn counts(self) -> bool {
        matches!(self, Section::Mine | Section::Reviews | Section::Assigned | Section::Triage)
    }
}

/// One reason a rule found.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Reason {
    pub rule: String,
    pub line: String,
    pub weight: u32,
    /// The time the reason started, for the rank.
    pub at: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Item {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub section: Section,
    /// The strongest reason's line.
    pub line: String,
    pub tags: Vec<String>,
    pub reasons: Vec<Reason>,
    /// Facts for the panel: the threads, the failing checks, the conflict.
    pub detail: Vec<String>,
    pub size: u64,
    pub release: bool,
    /// The newest event of another person on the PR (SPEC 39.2).
    pub last_event: u64,
    /// The reason it came back after Done, when it did.
    #[serde(default)]
    pub came_back: Option<String>,
    #[serde(default)]
    pub done_at: Option<u64>,
    #[serde(default)]
    pub later_until: Option<u64>,
    /// "Ranked 1st in Your PRs: ...", set by [`view`].
    #[serde(default)]
    pub why: String,
    #[serde(default)]
    pub labels: Vec<String>,
    /// What Claude made of your notes for this PR (39.12).
    #[serde(default)]
    pub claude: Option<crate::today_notes::Verdict>,
    /// The severity label of a report (39.13).
    #[serde(default)]
    pub severity: Option<String>,
    /// An issue, not a PR.
    #[serde(default)]
    pub issue: bool,
    /// From a job that reads every open item, and not about you: only that job shows it.
    #[serde(default)]
    pub wide: bool,
}

impl Item {
    pub fn weight(&self) -> u32 {
        self.reasons.first().map(|r| r.weight).unwrap_or(0)
    }
}

/// What the rules read besides the PRs.
pub struct Ctx<'a> {
    pub cfg: &'a TodayCfg,
    /// Your login, in lower case.
    pub me: String,
    pub now: u64,
}

impl Ctx<'_> {
    pub(crate) fn mine(&self, login: &str) -> bool {
        login.eq_ignore_ascii_case(&self.me)
    }
    /// A person other than you.
    pub(crate) fn other(&self, login: &str) -> bool {
        !self.mine(login) && !self.cfg.is_bot(login)
    }
    fn days(&self, since: u64) -> u64 {
        self.now.saturating_sub(since) / 86400
    }
}

/// A conflict found since the last refresh (SPEC 39.4, upstream_breaks).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Break {
    pub repo: String,
    pub number: u64,
    pub at: u64,
    pub cause: Option<u64>,
}

/// Everything one refresh fetched.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Data {
    /// Open PRs: yours, asked of you, reviewed by you, mentioning you.
    pub open: Vec<Pr>,
    /// Open PRs of the area search.
    pub area: Vec<Pr>,
    /// Your PRs closed or merged in the last days.
    pub closed: Vec<Pr>,
    /// PRs merged lately into the repos of your conflicting PRs.
    pub merged: Vec<Pr>,
    /// Open issues of the jobs that read issues (SPEC 39.13).
    #[serde(default)]
    pub issues: Vec<crate::today_issues::Issue>,
    /// Open PRs of the jobs that read every open item.
    #[serde(default)]
    pub wide_prs: Vec<Pr>,
}

fn has_label(pr: &Pr, list: &[String]) -> Option<String> {
    pr.labels.iter().find(|l| list.iter().any(|x| x.eq_ignore_ascii_case(l))).cloned()
}

fn is_release(pr: &Pr, rules: &RepoRules) -> bool {
    has_label(pr, &rules.release_labels).is_some() || pr.milestone.as_ref().is_some_and(|m| rules.release_labels.iter().any(|x| x.eq_ignore_ascii_case(m)))
}

const FAILED: &[&str] = &["FAILURE", "ERROR", "TIMED_OUT", "STARTUP_FAILURE", "ACTION_REQUIRED", "CANCELLED"];

fn failed(c: &Check) -> bool {
    FAILED.contains(&c.conclusion.as_str()) && c.conclusion != "CANCELLED"
}

/// Does `path` match an area pattern: a prefix, or a glob with `*` and `**`.
pub fn path_matches(pat: &str, path: &str) -> bool {
    if !pat.contains('*') {
        return path.starts_with(pat);
    }
    glob(pat.as_bytes(), path.as_bytes())
}

fn glob(p: &[u8], s: &[u8]) -> bool {
    if p.is_empty() {
        return s.is_empty();
    }
    if p.starts_with(b"**") {
        let rest = p[2..].strip_prefix(b"/").unwrap_or(&p[2..]);
        if rest.is_empty() {
            return true;
        }
        return (0..=s.len()).any(|i| glob(rest, &s[i..]));
    }
    if p[0] == b'*' {
        let mut i = 0;
        loop {
            if glob(&p[1..], &s[i..]) {
                return true;
            }
            if i == s.len() || s[i] == b'/' {
                return false;
            }
            i += 1;
        }
    }
    !s.is_empty() && p[0] == s[0] && glob(&p[1..], &s[1..])
}

/// Words that name nothing in a check name.
const PLAIN_WORDS: &[&str] = &["test", "tests", "check", "checks", "lint", "build", "ci", "unit", "rust", "cargo", "ubuntu", "latest", "windows", "macos", "linux", "release", "debug", "the", "and", "job", "run", "suite", "src", "crates", "x86_64", "aarch64", "stable", "nightly", "beta"];

/// The folders and crates of a repo that a check name can name: the first two parts
/// of each known path, with `-` and `_` as one.
fn known_dirs(paths: &[String]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for p in paths {
        for seg in p.split('/').take(2) {
            let w = seg.to_lowercase().replace('-', "_");
            if w.len() >= 3 && !w.contains('.') && !PLAIN_WORDS.contains(&w.as_str()) {
                out.insert(w);
            }
        }
    }
    out
}

fn words(name: &str) -> Vec<String> {
    name.to_lowercase()
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .flat_map(|w| {
            let w = w.replace('-', "_");
            let mut v = vec![w.clone()];
            // `network_tests` names `network`.
            for suf in ["_tests", "_test", "_check"] {
                if let Some(b) = w.strip_suffix(suf) {
                    v.push(b.to_owned());
                }
            }
            v
        })
        .collect()
}

/// The failed checks that look like flakes: each names a folder or crate that the
/// PR does not touch (SPEC 39.5). Empty when any failure is not one.
fn flakes(pr: &Pr, known: &BTreeSet<String>) -> Vec<String> {
    let touched = known_dirs(&pr.files);
    let bad: Vec<&Check> = pr.checks.iter().filter(|c| failed(c)).collect();
    if bad.is_empty() {
        return vec![];
    }
    let mut out = vec![];
    for c in bad {
        let named: Vec<String> = words(&c.name).into_iter().filter(|w| known.contains(w)).collect();
        if named.is_empty() || named.iter().any(|w| touched.contains(w)) {
            return vec![];
        }
        out.push(c.name.clone());
    }
    out
}

/// The newest PR merged into `pr`'s base whose files overlap `pr`'s (SPEC 39.5).
pub fn conflict_cause(pr: &Pr, merged: &[Pr]) -> Option<u64> {
    merged
        .iter()
        .filter(|m| m.repo.eq_ignore_ascii_case(&pr.repo) && m.base == pr.base && m.number != pr.number && m.state == "MERGED")
        .filter(|m| m.files.iter().any(|f| pr.files.contains(f)))
        .max_by_key(|m| m.closed.unwrap_or(0))
        .map(|m| m.number)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn quote(body: &str, me: &str) -> String {
    let at = format!("@{me}");
    let line = body.lines().find(|l| l.to_lowercase().contains(&at)).unwrap_or(body).trim();
    let short: String = line.chars().take(80).collect();
    if short.len() < line.len() {
        format!("{short}…")
    } else {
        short
    }
}

/// The newest event of a person other than you: a commit, a review, a comment, a
/// thread reply, or a failed check (SPEC 39.2). CI is not a person, but a new
/// failure needs you.
pub fn last_event(pr: &Pr, ctx: &Ctx) -> u64 {
    let mut t = 0;
    for c in &pr.commits {
        if ctx.other(&c.author) {
            t = t.max(c.at);
        }
    }
    for r in &pr.reviews {
        if ctx.other(&r.author) {
            t = t.max(r.at);
        }
    }
    for c in &pr.comments {
        if ctx.other(&c.author) {
            t = t.max(c.at);
        }
    }
    for th in &pr.threads {
        if ctx.other(&th.last_author) {
            t = t.max(th.last_at);
        }
    }
    if ctx.mine(&pr.author) {
        for c in &pr.checks {
            if failed(c) {
                t = t.max(c.at);
            }
        }
    }
    for r in &pr.requests {
        t = t.max(r.at);
    }
    t
}

/// The rules for one of your open PRs.
fn rules_mine(pr: &Pr, ctx: &Ctx, rules: &RepoRules, merged: &[Pr], known: &BTreeSet<String>, detail: &mut Vec<String>) -> Vec<Reason> {
    let mut out = vec![];
    let my_push = pr.commits.iter().filter(|c| ctx.mine(&c.author)).map(|c| c.at).max().unwrap_or(0);
    // changes_requested: a reviewer's latest review, with no push of yours after it.
    let asked: Vec<&Review> = pr.reviews.iter().filter(|r| r.state == "CHANGES_REQUESTED" && ctx.other(&r.author) && r.at > my_push).collect();
    if let Some(first) = asked.first() {
        let who: Vec<&str> = asked.iter().map(|r| r.author.as_str()).collect();
        out.push(Reason { rule: "changes_requested".into(), line: format!("Address {}'s requested changes.", who.join(" and ")), weight: 100, at: first.at });
    }
    // open_threads: unresolved, the last comment by another person.
    let open: Vec<&Thread> = pr.threads.iter().filter(|t| !t.resolved && ctx.other(&t.last_author)).collect();
    if !open.is_empty() {
        let mut by: BTreeMap<&str, usize> = BTreeMap::new();
        for t in &open {
            *by.entry(t.last_author.as_str()).or_default() += 1;
        }
        let names: Vec<&str> = by.keys().copied().collect();
        let line = if names.len() == 1 {
            format!("Reply to {}'s {}.", names[0], plural(open.len(), "open thread", "open threads"))
        } else {
            format!("Reply to {} ({}).", plural(open.len(), "open thread", "open threads"), names.join(", "))
        };
        let at = open.iter().map(|t| t.last_at).min().unwrap_or(0);
        for t in &open {
            detail.push(format!("Open thread{} waits on you: last comment by {}.", t.path.as_ref().map(|p| format!(" on `{p}`")).unwrap_or_default(), t.last_author));
        }
        out.push(Reason { rule: "open_threads".into(), line, weight: 90, at });
    }
    if pr.mergeable == "CONFLICTING" {
        let line = match conflict_cause(pr, merged) {
            Some(n) => {
                detail.push(format!("#{n} merged into {} and changed files this PR changes.", pr.base));
                format!("Rebase: it conflicts with #{n}.")
            }
            None => "Rebase: it conflicts.".into(),
        };
        out.push(Reason { rule: "conflict".into(), line, weight: 80, at: pr.updated });
    }
    let bad: Vec<&Check> = pr.checks.iter().filter(|c| failed(c)).collect();
    if !bad.is_empty() {
        for c in &bad {
            detail.push(format!("Check `{}`: {}.", c.name, c.conclusion.to_lowercase().replace('_', " ")));
        }
        let at = bad.iter().map(|c| c.at).max().unwrap_or(0);
        let flaky = flakes(pr, known);
        if !flaky.is_empty() {
            out.push(Reason {
                rule: "ci_flaky".into(),
                line: format!("Re-run CI: `{}` failed in code this PR does not touch. Likely a flake.", flaky[0]),
                weight: 30,
                at,
            });
        } else {
            let more = if bad.len() > 1 { format!(" and {} more", bad.len() - 1) } else { String::new() };
            out.push(Reason { rule: "ci_failed".into(), line: format!("Fix CI: `{}` fails{more}.", bad[0].name), weight: 70, at });
        }
    }
    let blocked = pr.labels.iter().any(|l| {
        let l = l.to_lowercase();
        l.contains("do-not-merge") || l.contains("do not merge") || l.contains("blocked")
    });
    if pr.decision == "APPROVED" && pr.checks_state == "SUCCESS" && !pr.draft && !pr.in_queue && pr.mergeable != "CONFLICTING" && !blocked {
        let at = pr.reviews.iter().filter(|r| r.state == "APPROVED").map(|r| r.at).max().unwrap_or(pr.updated);
        out.push(Reason { rule: "ready".into(), line: "Approved. Add it to the merge queue.".into(), weight: 60, at });
    }
    let waits_on_me = out.iter().any(|r| r.rule == "changes_requested" || r.rule == "open_threads");
    if let Some(l) = has_label(pr, &rules.waiting_labels) {
        if !waits_on_me {
            out.push(Reason { rule: "label_wrong".into(), line: format!("Label `{l}` says it waits on you, but nothing does. Change it."), weight: 40, at: pr.updated });
        }
    }
    if let Some(l) = has_label(pr, &rules.stale_labels) {
        if ctx.days(pr.updated) < rules.stale_days {
            out.push(Reason { rule: "label_wrong".into(), line: format!("Label `{l}` is wrong: the PR changed {} ago.", days_text(ctx.days(pr.updated))), weight: 40, at: pr.updated });
        }
    }
    // In a repo of your own (a fork), nobody is there to review.
    let own_repo = pr.repo.split('/').next().is_some_and(|o| ctx.mine(o));
    if !own_repo && !pr.draft && pr.reviews.iter().all(|r| !ctx.other(&r.author)) && ctx.days(pr.created) >= rules.no_review_days && pr.decision != "APPROVED" {
        out.push(Reason { rule: "no_reviews".into(), line: format!("No review in {}. Ask someone.", days_text(ctx.days(pr.created))), weight: 30, at: pr.created });
    }
    if pr.draft && ctx.days(pr.updated) >= rules.stale_days {
        out.push(Reason { rule: "draft_old".into(), line: format!("Draft idle {}. Finish or close it.", days_text(ctx.days(pr.updated))), weight: 10, at: pr.updated });
    }
    out
}

fn days_text(d: u64) -> String {
    match d {
        0 => "today".into(),
        1 => "1 day".into(),
        n => format!("{n} days"),
    }
}

pub(crate) fn ago(d: u64) -> String {
    match d {
        0 => "today".into(),
        1 => "yesterday".into(),
        n => format!("{n} days ago"),
    }
}

/// The rules for an open PR of someone else.
fn rules_review(pr: &Pr, ctx: &Ctx, detail: &mut Vec<String>) -> Vec<Reason> {
    let mut out = vec![];
    let teams = &ctx.cfg.teams;
    let mine_review = pr.reviews.iter().filter(|r| ctx.mine(&r.author)).map(|r| r.at).max();
    // asked_you: a person @mentions you after your review, or lately.
    let since = mine_review.unwrap_or(ctx.now.saturating_sub(14 * 86400));
    let at_me = format!("@{}", ctx.me);
    if let Some(c) = pr.comments.iter().rev().find(|c| c.at > since && ctx.other(&c.author) && c.body.to_lowercase().contains(&at_me)) {
        out.push(Reason { rule: "asked_you".into(), line: format!("{} asks: \"{}\"", c.author, quote(&c.body, &ctx.me)), weight: 90, at: c.at });
    }
    if let Some(r) = pr.requests.iter().find(|r| ctx.mine(&r.author) || teams.iter().any(|t| t.eq_ignore_ascii_case(&r.author))) {
        let at = if r.at > 0 { r.at } else { pr.created };
        let via = if ctx.mine(&r.author) { String::new() } else { format!(" (through {})", r.author) };
        out.push(Reason { rule: "requested".into(), line: format!("Review asked of you {}{via}.", ago(ctx.days(at))), weight: 85, at });
    }
    if let Some(mine) = mine_review {
        let commits: Vec<&Ev> = pr.commits.iter().filter(|c| c.at > mine && !ctx.mine(&c.author)).collect();
        let replied = pr.comments.iter().any(|c| c.at > mine && ctx.other(&c.author)) || pr.threads.iter().any(|t| t.last_at > mine && ctx.other(&t.last_author));
        if !commits.is_empty() {
            detail.push(format!("{} since your review {}.", plural(commits.len(), "commit", "commits"), ago(ctx.days(mine))));
            out.push(Reason { rule: "re_review".into(), line: format!("They pushed {} after your review.", plural(commits.len(), "commit", "commits")), weight: 70, at: commits[0].at });
        } else if replied {
            out.push(Reason { rule: "re_review".into(), line: "They replied after your review.".into(), weight: 70, at: mine });
        }
    }
    out
}

/// The area rule for an open PR of someone else that you are not on yet.
fn rules_area(pr: &Pr, rules: &RepoRules) -> Option<(Reason, bool)> {
    for (name, area) in &rules.areas {
        let label = pr.labels.iter().find(|l| area.labels.iter().any(|x| x.eq_ignore_ascii_case(l)));
        let path = area.paths.iter().find(|p| pr.files.iter().any(|f| path_matches(p, f)));
        let title = {
            let mut c = name.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        };
        let line = match (path, label) {
            (Some(p), _) => format!("{title}: touches `{}`.", p.trim_end_matches("**").trim_end_matches('/')),
            (None, Some(l)) => format!("{title}: labelled `{l}`."),
            (None, None) => continue,
        };
        let release = is_release(pr, rules);
        return Some((Reason { rule: "area".into(), line, weight: if release { 25 } else { 15 }, at: pr.updated }, release));
    }
    None
}

/// Every item of the fetched data, before Done and Later (SPEC 39.4).
pub fn items(data: &Data, breaks: &[Break], ctx: &Ctx) -> Vec<Item> {
    let mut out: Vec<Item> = vec![];
    let mut seen: BTreeSet<(String, u64)> = BTreeSet::new();
    // The folders each repo has, from every PR seen, for the flake guess.
    let mut paths: HashMap<String, Vec<String>> = HashMap::new();
    for pr in data.open.iter().chain(&data.area).chain(&data.merged) {
        paths.entry(pr.repo.to_lowercase()).or_default().extend(pr.files.iter().cloned());
    }
    for (repo, r) in &ctx.cfg.repos {
        for a in r.areas.values() {
            paths.entry(repo.to_lowercase()).or_default().extend(a.paths.iter().map(|p| p.replace('*', "")));
        }
    }
    let empty = vec![];
    let opens = data.open.iter().map(|p| (p, false)).chain(data.area.iter().map(|p| (p, true)));
    for (pr, from_area) in opens {
        if pr.state != "OPEN" || !seen.insert((pr.repo.to_lowercase(), pr.number)) {
            continue;
        }
        let rules = ctx.cfg.rules(&pr.repo);
        let known = known_dirs(paths.get(&pr.repo.to_lowercase()).unwrap_or(&empty));
        let mut detail = vec![];
        let mut tags = vec![];
        let release = is_release(pr, &rules);
        let (section, mut reasons) = if ctx.mine(&pr.author) {
            (Section::Mine, rules_mine(pr, ctx, &rules, &data.merged, &known, &mut detail))
        } else {
            let r = rules_review(pr, ctx, &mut detail);
            let on_it = !r.is_empty() || pr.reviews.iter().any(|x| ctx.mine(&x.author)) || pr.requests.iter().any(|x| ctx.mine(&x.author));
            if !on_it {
                match rules_area(pr, &rules) {
                    Some((mut reason, _)) => {
                        if pr.size <= rules.tiny_lines && pr.checks_state == "SUCCESS" && !pr.draft {
                            reason.weight += 10;
                            reason.line = format!("{} Tiny, {}. Quick approve?", reason.line, plural(pr.size as usize, "line", "lines"));
                            tags.push("tiny".to_owned());
                        }
                        (Section::Area, vec![reason])
                    }
                    None => continue,
                }
            } else {
                (Section::Reviews, r)
            }
        };
        let _ = from_area;
        if reasons.is_empty() {
            continue;
        }
        reasons.sort_by_key(|r| std::cmp::Reverse(r.weight));
        if release {
            let l = has_label(pr, &rules.release_labels).or_else(|| pr.milestone.clone()).unwrap_or_default();
            tags.push(l);
        }
        if reasons.iter().any(|r| r.rule == "ci_flaky") {
            tags.push("flaky CI?".into());
        }
        out.push(Item {
            repo: pr.repo.clone(),
            number: pr.number,
            title: pr.title.clone(),
            url: pr.url.clone(),
            section,
            line: reasons[0].line.clone(),
            tags,
            reasons,
            detail,
            size: pr.size,
            release,
            last_event: last_event(pr, ctx),
            came_back: None,
            done_at: None,
            later_until: None,
            why: String::new(),
            labels: pr.labels.clone(),
            claude: None,
            severity: None,
            issue: false,
            wide: false,
        });
    }
    // What changed: the last 3 days.
    let since = ctx.now.saturating_sub(3 * 86400);
    for pr in &data.closed {
        let Some(at) = pr.closed.filter(|t| *t >= since) else { continue };
        if !ctx.mine(&pr.author) || !seen.insert((pr.repo.to_lowercase(), pr.number)) {
            continue;
        }
        let (rule, line) = if pr.state == "MERGED" {
            ("merged_mine", format!("Your #{} merged.", pr.number))
        } else {
            ("closed_mine", format!("Your #{} was closed.", pr.number))
        };
        out.push(changed(pr.repo.clone(), pr.number, pr.title.clone(), pr.url.clone(), Reason { rule: rule.into(), line, weight: 20, at }));
    }
    for b in breaks.iter().filter(|b| b.at >= since) {
        let Some(pr) = data.open.iter().find(|p| p.repo.eq_ignore_ascii_case(&b.repo) && p.number == b.number) else { continue };
        let line = match b.cause {
            Some(n) => format!("#{n} merged. Your #{} now conflicts.", b.number),
            None => format!("Your #{} now conflicts with {}.", b.number, pr.base),
        };
        let mut it = changed(pr.repo.clone(), pr.number, pr.title.clone(), pr.url.clone(), Reason { rule: "upstream_breaks".into(), line, weight: 20, at: b.at });
        // Its own row: the Your PRs row of the same PR says what to do.
        it.number = pr.number;
        out.push(it);
    }
    // Issues and the open PRs of repos you triage (SPEC 39.13).
    out.extend(crate::today_issues::issue_items(&data.issues, ctx, &mut seen));
    out.extend(crate::today_issues::poc_items(&data.wide_prs, ctx, &mut seen));
    // A list fetched before `only` changed: drop the repos it leaves out now.
    out.retain(|i| ctx.cfg.reads(&i.repo));
    out
}

fn changed(repo: String, number: u64, title: String, url: String, r: Reason) -> Item {
    Item {
        repo,
        number,
        title,
        url,
        section: Section::Changed,
        line: r.line.clone(),
        tags: vec![],
        last_event: r.at,
        reasons: vec![r],
        detail: vec![],
        size: 0,
        release: false,
        came_back: None,
        done_at: None,
        later_until: None,
        labels: vec![],
        claude: None,
        severity: None,
        issue: false,
        wide: false,
        why: String::new(),
    }
}

/// Rank inside a section (SPEC 39.5).
pub fn rank(items: &mut [Item]) {
    items.sort_by(|a, b| {
        let older_first = |i: &Item| matches!(i.reasons.first().map(|r| r.rule.as_str()), Some("requested" | "no_reviews"));
        let at = |i: &Item| i.reasons.first().map(|r| r.at).unwrap_or(0);
        // Your notes (39.12): Claude's "up" goes first in its section, "down" last.
        let nudge = |i: &Item| match i.claude.as_ref().map(|c| c.act) {
            Some(crate::today_notes::Act::Up) => 0,
            Some(crate::today_notes::Act::Down) => 2,
            _ => 1,
        };
        a.section
            .cmp(&b.section)
            .then(nudge(a).cmp(&nudge(b)))
            .then(b.weight().cmp(&a.weight()))
            .then(b.release.cmp(&a.release))
            .then_with(|| if older_first(a) { at(a).cmp(&at(b)) } else { at(b).cmp(&at(a)) })
            .then(a.size.cmp(&b.size))
            .then(a.number.cmp(&b.number))
    });
}

/// Done and Later of one PR, from the store.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Seen {
    pub done_at: Option<u64>,
    /// The newest event when you marked it done: only a newer one brings it back.
    pub done_event: Option<u64>,
    pub later_until: Option<u64>,
}

/// The key of a PR in `today_seen`, and of a What changed row.
pub fn key(it: &Item) -> (String, u64) {
    let n = if it.section == Section::Changed { it.number + CHANGED_KEY } else { it.number };
    (it.repo.to_lowercase(), n)
}

/// What changed rows keep their own Done, apart from the PR's row.
pub const CHANGED_KEY: u64 = 1 << 40;

/// What came back since Done, in words.
fn came_back_text(pr_event: u64, done_event: u64, it: &Item) -> String {
    let _ = (pr_event, done_event);
    if it.reasons.iter().any(|r| r.rule == "re_review" && r.line.contains("pushed")) {
        return "New commits after you marked it done.".into();
    }
    "Someone acted on it after you marked it done.".into()
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SectionView {
    pub key: Section,
    pub title: String,
    pub items: Vec<Item>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct JobTab {
    pub name: String,
    /// Your PRs and Reviews that show.
    pub count: usize,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct View {
    pub job: String,
    pub jobs: Vec<JobTab>,
    /// The job's repos; empty for every repo.
    pub repos: Vec<String>,
    pub sections: Vec<SectionView>,
    pub done: Vec<Item>,
    pub later: Vec<Item>,
    /// Hidden by Claude, from your notes (39.12).
    pub hidden: Vec<Item>,
    /// Since you last looked, or `None` with nothing new.
    pub since: Option<String>,
}

fn in_job(repo: &str, repos: &[String]) -> bool {
    repos.is_empty() || repos.iter().any(|r| r.eq_ignore_ascii_case(repo))
}

fn ordinal(n: usize) -> String {
    let suf = match (n % 10, n % 100) {
        (1, x) if x != 11 => "st",
        (2, x) if x != 12 => "nd",
        (3, x) if x != 13 => "rd",
        _ => "th",
    };
    format!("{n}{suf}")
}

/// Splits the items of one job into what shows, Done, Later and Hidden (SPEC 39.2, 39.12).
fn split(all: &[Item], repos: &[String], wide_ok: bool, seen: &HashMap<(String, u64), Seen>, now: u64) -> (Vec<Item>, Vec<Item>, Vec<Item>, Vec<Item>) {
    let (mut show, mut done, mut later, mut hidden) = (vec![], vec![], vec![], vec![]);
    // An item that is not about you shows only in a job that reads every open item.
    for it in all.iter().filter(|i| in_job(&i.repo, repos) && (wide_ok || !i.wide)) {
        // The area section is for jobs: Everything has no areas.
        if repos.is_empty() && it.section == Section::Area {
            continue;
        }
        let mut it = it.clone();
        if it.claude.as_ref().is_some_and(|c| c.act == crate::today_notes::Act::Hide) {
            hidden.push(it);
            continue;
        }
        let st = seen.get(&key(&it)).cloned().unwrap_or_default();
        if let Some(until) = st.later_until.filter(|u| *u > now) {
            it.later_until = Some(until);
            later.push(it);
            continue;
        }
        if let Some(d) = st.done_at {
            let mark = st.done_event.unwrap_or(d);
            if it.section == Section::Changed || it.last_event <= mark {
                it.done_at = Some(d);
                done.push(it);
                continue;
            }
            it.came_back = Some(came_back_text(it.last_event, mark, &it));
            it.tags.push("came back".into());
        }
        show.push(it);
    }
    (show, done, later, hidden)
}

/// The list of one job (SPEC 39.1). `looked` is when you last looked at it.
pub fn view(all: &[Item], cfg: &TodayCfg, job: &str, seen: &HashMap<(String, u64), Seen>, looked: Option<u64>, now: u64) -> View {
    let job = if cfg.jobs.contains_key(job) { job.to_owned() } else { EVERYTHING.to_owned() };
    let repos = cfg.job_repos(&job);
    let mut all = all.to_vec();
    rank(&mut all);
    let wide_ok = cfg.jobs.get(&job).is_some_and(|j| j.all());
    let (show, done, later, hidden) = split(&all, &repos, wide_ok, seen, now);
    let jobs = cfg
        .job_names()
        .into_iter()
        .map(|name| {
            let r = cfg.job_repos(&name);
            let w = cfg.jobs.get(&name).is_some_and(|j| j.all());
            let (s, _, _, _) = split(&all, &r, w, seen, now);
            JobTab { count: s.iter().filter(|i| i.section.counts()).count(), name }
        })
        .collect();
    let sections = Section::ALL
        .iter()
        .map(|sec| {
            let mut items: Vec<Item> = show.iter().filter(|i| i.section == *sec).cloned().collect();
            let n = items.len();
            for (i, it) in items.iter_mut().enumerate() {
                let rest: Vec<&str> = it.reasons.iter().skip(1).map(|r| r.rule.as_str()).collect();
                let also = if rest.is_empty() { String::new() } else { format!(" Also: {}.", rest.iter().map(|r| rule_words(r)).collect::<Vec<_>>().join(", ")) };
                it.why = format!("Ranked {} of {n} in {}: {}.{also}", ordinal(i + 1), sec.title(), rule_words(&it.reasons[0].rule));
                if let Some(c) = &it.claude {
                    let moved = if c.act == crate::today_notes::Act::Up { "up" } else { "down" };
                    it.why.push_str(&format!(" Claude moved it {moved}, from your notes: {}.", c.why.trim_end_matches('.')));
                }
            }
            SectionView { key: *sec, title: sec.title().to_owned(), items }
        })
        .collect::<Vec<_>>();
    let since = looked.and_then(|t| {
        let mut parts = vec![];
        for s in &sections {
            for i in &s.items {
                if i.section == Section::Changed && i.last_event > t {
                    parts.push(i.line.trim_end_matches('.').to_owned());
                }
            }
        }
        let fresh = sections.iter().flat_map(|s| &s.items).filter(|i| i.section != Section::Changed && i.came_back.is_none() && i.last_event > t).count();
        if fresh > 0 {
            parts.push(format!("{} with news", plural(fresh, "item", "items")));
        }
        for i in sections.iter().flat_map(|s| &s.items).filter(|i| i.came_back.is_some()) {
            parts.push(format!("#{} came back", i.number));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    });
    View { job, jobs, repos, sections, done, later, hidden, since }
}

/// A rule's name in words, for Why.
pub fn rule_words(rule: &str) -> &'static str {
    match rule {
        "changes_requested" => "changes asked",
        "open_threads" => "threads wait on you",
        "conflict" => "it conflicts",
        "ci_failed" => "CI fails",
        "ci_flaky" => "CI failed, likely a flake",
        "ready" => "approved and ready",
        "label_wrong" => "a label is wrong",
        "no_reviews" => "no review yet",
        "draft_old" => "an old draft",
        "asked_you" => "someone asks you",
        "requested" => "a review is asked of you",
        "re_review" => "news after your review",
        "area" => "in your area",
        "merged_mine" => "your PR merged",
        "closed_mine" => "your PR was closed",
        "upstream_breaks" => "a merge broke your PR",
        _ => "a rule matched",
    }
}

/// The list as plain lines, for `sb today` and the pane tool (SPEC 39.8).
pub fn text(v: &View, many_repos: bool) -> String {
    let mut out = format!("Today · {}\n", v.job);
    if let Some(s) = &v.since {
        out.push_str(&format!("Since you last looked: {s}\n"));
    }
    let mut any = false;
    let names = short_names(v.sections.iter().flat_map(|s| &s.items).map(|i| i.repo.as_str()));
    for s in &v.sections {
        if s.items.is_empty() {
            continue;
        }
        any = true;
        out.push_str(&format!("\n{} ({})\n", s.title.to_uppercase(), s.items.len()));
        for i in &s.items {
            let name = if many_repos { format!("{}#{}", names.get(i.repo.as_str()).copied().unwrap_or(&i.repo), i.number) } else { format!("#{}", i.number) };
            let all_tags: Vec<String> = i.severity.iter().cloned().chain(i.tags.iter().cloned()).collect();
            let tags = if all_tags.is_empty() { String::new() } else { format!("  [{}]", all_tags.join(", ")) };
            out.push_str(&format!("- {name}  {}{tags}\n    {}\n", i.line, i.title));
        }
    }
    if !any {
        out.push_str("\nAll done for today.\n");
    }
    if !v.hidden.is_empty() {
        let list: Vec<String> = v.hidden.iter().map(|i| format!("#{}", i.number)).collect();
        out.push_str(&format!("\nHidden by the user's notes to Claude: {}.\n", list.join(", ")));
    }
    out
}

/// The name of each repo on a row: `name`, or `owner/name` when two repos share a name.
pub fn short_names<'a>(repos: impl Iterator<Item = &'a str>) -> HashMap<&'a str, &'a str> {
    let all: BTreeSet<&str> = repos.collect();
    let short = |r: &'a str| r.rsplit('/').next().unwrap_or(r);
    all.iter()
        .map(|r| {
            let s = short(r);
            let shared = all.iter().any(|o| o != r && short(o).eq_ignore_ascii_case(s));
            (*r, if shared { *r } else { s })
        })
        .collect()
}

/// One item in full, for the pane tool `today_item`.
pub fn item_json(i: &Item) -> Value {
    json!({
        "repo": i.repo, "number": i.number, "url": i.url, "section": i.section.title(),
        "title": format!("<github-data>{}</github-data>", i.title),
        "todo": i.line, "why": i.why,
        "reasons": i.reasons.iter().map(|r| &r.line).collect::<Vec<_>>(),
        "detail": i.detail, "tags": i.tags, "came_back": i.came_back, "severity": i.severity, "kind": if i.issue { "issue" } else { "pr" },
        "claude": i.claude.as_ref().map(|c| json!({ "act": c.act, "why": c.why })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86400;
    const NOW: u64 = 1_800_000_000;

    fn cfg() -> TodayCfg {
        TodayCfg::default()
    }

    fn ctx(c: &TodayCfg) -> Ctx<'_> {
        Ctx { cfg: c, me: "me".into(), now: NOW }
    }

    fn pr(n: u64, author: &str) -> Pr {
        Pr {
            repo: "o/r".into(),
            number: n,
            title: format!("PR {n}"),
            url: format!("https://github.com/o/r/pull/{n}"),
            author: author.into(),
            mergeable: "MERGEABLE".into(),
            base: "main".into(),
            state: "OPEN".into(),
            created: NOW - DAY,
            updated: NOW - DAY,
            checks_state: "SUCCESS".into(),
            ..Pr::default()
        }
    }

    fn rules_of(items: &[Item], n: u64) -> Vec<String> {
        items.iter().filter(|i| i.number == n).flat_map(|i| i.reasons.iter().map(|r| r.rule.clone())).collect()
    }

    fn run(open: Vec<Pr>) -> Vec<Item> {
        let c = cfg();
        items(&Data { open, ..Data::default() }, &[], &ctx(&c))
    }

    #[test]
    fn changes_requested_fires_until_you_push() {
        let mut p = pr(1, "me");
        p.reviews.push(Review { author: "michael".into(), state: "CHANGES_REQUESTED".into(), at: NOW - 3600 });
        let it = run(vec![p.clone()]);
        assert_eq!(rules_of(&it, 1), ["changes_requested"]);
        assert_eq!(it[0].line, "Address michael's requested changes.");
        p.commits.push(Ev { author: "me".into(), at: NOW - 60 });
        assert!(rules_of(&run(vec![p]), 1).is_empty());
    }

    #[test]
    fn open_threads_skip_your_bots_and_resolved_ones() {
        let mut p = pr(2, "me");
        let t = |who: &str, resolved: bool| Thread { resolved, last_author: who.into(), last_at: NOW - 100, path: Some("a.rs".into()) };
        p.threads = vec![t("me", false), t("mergify[bot]", false), t("bob", true)];
        assert!(rules_of(&run(vec![p.clone()]), 2).is_empty());
        p.threads.push(t("bob", false));
        p.threads.push(t("bob", false));
        let it = run(vec![p]);
        assert_eq!(it[0].line, "Reply to bob's 2 open threads.");
        assert_eq!(it[0].detail.len(), 2);
    }

    #[test]
    fn conflict_names_the_merge_that_overlaps() {
        let mut p = pr(3, "me");
        p.mergeable = "CONFLICTING".into();
        p.files = vec!["a/x.rs".into()];
        let mut m1 = pr(10, "x");
        m1.state = "MERGED".into();
        m1.files = vec!["a/x.rs".into()];
        m1.closed = Some(NOW - 500);
        let mut m2 = m1.clone();
        m2.number = 11;
        m2.files = vec!["b.rs".into()];
        m2.closed = Some(NOW - 100);
        let c = cfg();
        let it = items(&Data { open: vec![p.clone()], merged: vec![m1, m2], ..Data::default() }, &[], &ctx(&c));
        assert_eq!(it[0].line, "Rebase: it conflicts with #10.");
        assert_eq!(run(vec![p])[0].line, "Rebase: it conflicts.");
    }

    #[test]
    fn ci_failure_and_the_flake_guess() {
        let mut p = pr(4, "me");
        p.checks_state = "FAILURE".into();
        p.files = vec!["beacon_node/store/src/a.rs".into()];
        p.checks = vec![Check { name: "network-tests".into(), conclusion: "FAILURE".into(), at: NOW - 10 }];
        // Nothing knows `network`: a plain failure.
        assert_eq!(rules_of(&run(vec![p.clone()]), 4), ["ci_failed"]);
        // Another PR touches beacon_node/network: the name is a folder this PR leaves alone.
        let mut other = pr(5, "x");
        other.files = vec!["beacon_node/network/src/b.rs".into()];
        let it = run(vec![p.clone(), other]);
        assert_eq!(rules_of(&it, 4), ["ci_flaky"]);
        assert!(it[0].tags.contains(&"flaky CI?".to_owned()));
        // A failure in code the PR touches is not a flake.
        p.files.push("beacon_node/network/src/c.rs".into());
        assert_eq!(rules_of(&run(vec![p]), 4), ["ci_failed"]);
    }

    #[test]
    fn ready_needs_approval_green_checks_and_no_queue() {
        let mut p = pr(6, "me");
        p.decision = "APPROVED".into();
        p.reviews.push(Review { author: "bob".into(), state: "APPROVED".into(), at: NOW - 3 * DAY });
        assert_eq!(rules_of(&run(vec![p.clone()]), 6), ["ready"]);
        p.in_queue = true;
        assert!(rules_of(&run(vec![p.clone()]), 6).is_empty());
        p.in_queue = false;
        p.labels = vec!["do-not-merge".into()];
        assert!(rules_of(&run(vec![p]), 6).is_empty());
    }

    #[test]
    fn wrong_labels_no_reviews_and_old_drafts() {
        let mut p = pr(7, "me");
        p.labels = vec!["waiting-on-author".into()];
        p.created = NOW - 5 * DAY;
        let r = rules_of(&run(vec![p.clone()]), 7);
        assert!(r.contains(&"label_wrong".to_owned()) && r.contains(&"no_reviews".to_owned()), "{r:?}");
        p.threads.push(Thread { resolved: false, last_author: "bob".into(), last_at: NOW, path: None });
        assert!(!rules_of(&run(vec![p]), 7).contains(&"label_wrong".to_owned()));
        let mut d = pr(8, "me");
        d.draft = true;
        d.updated = NOW - 20 * DAY;
        d.created = NOW - 30 * DAY;
        assert_eq!(rules_of(&run(vec![d]), 8), ["draft_old"]);
    }

    #[test]
    fn reviews_requested_re_review_and_asked_you() {
        let mut p = pr(20, "bob");
        p.requests.push(Ev { author: "Me".into(), at: NOW - 2 * DAY });
        let it = run(vec![p]);
        assert_eq!(it[0].section, Section::Reviews);
        assert_eq!(it[0].line, "Review asked of you 2 days ago.");
        let mut q = pr(21, "bob");
        q.reviews.push(Review { author: "me".into(), state: "COMMENTED".into(), at: NOW - DAY });
        q.commits = vec![Ev { author: "bob".into(), at: NOW - 100 }, Ev { author: "bob".into(), at: NOW - 50 }];
        assert_eq!(run(vec![q.clone()])[0].line, "They pushed 2 commits after your review.");
        q.commits.clear();
        q.comments.push(Comment { author: "pawan".into(), at: NOW - 10, body: "hey @me do you want to re-review?".into() });
        let it = run(vec![q]);
        assert_eq!(it[0].line, "pawan asks: \"hey @me do you want to re-review?\"");
        // A team request counts with the team in the config.
        let mut t = pr(22, "bob");
        t.requests.push(Ev { author: "sigp/core".into(), at: 0 });
        assert!(run(vec![t.clone()]).is_empty());
        let c = TodayCfg { teams: vec!["sigp/core".into()], ..cfg() };
        let it = items(&Data { open: vec![t], ..Data::default() }, &[], &ctx(&c));
        assert_eq!(it[0].line, "Review asked of you yesterday (through sigp/core).");
    }

    #[test]
    fn areas_by_label_prefix_and_glob_and_tiny() {
        let mut c = cfg();
        let mut r = RepoRules { release_labels: vec!["v9.0.0".into()], ..RepoRules::default() };
        r.areas.insert("store".into(), AreaCfg { labels: vec!["database".into()], paths: vec!["beacon_node/store/".into()] });
        r.areas.insert("sync".into(), AreaCfg { labels: vec![], paths: vec!["beacon_node/**/sync/**".into()] });
        c.repos.insert("o/r".into(), r);
        let mut a = pr(30, "x");
        a.files = vec!["beacon_node/network/src/sync/a.rs".into()];
        a.size = 200;
        let mut b = pr(31, "x");
        b.labels = vec!["database".into(), "v9.0.0".into()];
        b.size = 12;
        let mut n = pr(32, "x");
        n.files = vec!["book/a.md".into()];
        let it = items(&Data { area: vec![a, b, n], ..Data::default() }, &[], &ctx(&c));
        let by = |n: u64| it.iter().find(|i| i.number == n).cloned();
        assert_eq!(by(30).unwrap().line, "Sync: touches `beacon_node/**/sync`.");
        let b = by(31).unwrap();
        assert!(b.line.starts_with("Store: labelled `database`.") && b.line.ends_with("Quick approve?"), "{}", b.line);
        assert!(b.tags.contains(&"tiny".to_owned()) && b.tags.contains(&"v9.0.0".to_owned()));
        assert_eq!(b.weight(), 35);
        assert!(by(32).is_none());
        assert!(path_matches("beacon_node/**/sync/**", "beacon_node/network/src/sync/x.rs"));
        assert!(path_matches("a/*.rs", "a/b.rs") && !path_matches("a/*.rs", "a/b/c.rs"));
    }

    #[test]
    fn rank_orders_sections_weights_and_ages() {
        let mut a = pr(1, "me");
        a.decision = "APPROVED".into();
        let mut b = pr(2, "me");
        b.reviews.push(Review { author: "x".into(), state: "CHANGES_REQUESTED".into(), at: NOW - 10 });
        let mut r1 = pr(3, "x");
        r1.requests.push(Ev { author: "me".into(), at: NOW - DAY });
        let mut r2 = pr(4, "x");
        r2.requests.push(Ev { author: "me".into(), at: NOW - 5 * DAY });
        let mut it = run(vec![r1, a, r2, b]);
        rank(&mut it);
        let order: Vec<u64> = it.iter().map(|i| i.number).collect();
        // Changes asked (100) before ready (60); the older review request first.
        assert_eq!(order, [2, 1, 4, 3]);
    }

    #[test]
    fn done_hides_until_another_person_acts_and_later_until_its_time() {
        let c = cfg();
        let mut p = pr(40, "me");
        p.threads.push(Thread { resolved: false, last_author: "bob".into(), last_at: NOW - 100, path: None });
        let all = items(&Data { open: vec![p.clone()], ..Data::default() }, &[], &ctx(&c));
        let mut seen = HashMap::new();
        seen.insert(("o/r".to_owned(), 40), Seen { done_at: Some(NOW - 50), done_event: Some(NOW - 100), later_until: None });
        let v = view(&all, &c, EVERYTHING, &seen, None, NOW);
        assert!(v.sections[0].items.is_empty() && v.done.len() == 1);
        // Your own push and a bot do not bring it back.
        p.commits.push(Ev { author: "me".into(), at: NOW - 10 });
        p.comments.push(Comment { author: "github-actions[bot]".into(), at: NOW - 5, body: "x".into() });
        let all = items(&Data { open: vec![p.clone()], ..Data::default() }, &[], &ctx(&c));
        assert!(view(&all, &c, EVERYTHING, &seen, None, NOW).done.len() == 1);
        // Bob replies: it comes back with the tag.
        p.threads[0].last_at = NOW - 1;
        let all = items(&Data { open: vec![p.clone()], ..Data::default() }, &[], &ctx(&c));
        let v = view(&all, &c, EVERYTHING, &seen, None, NOW);
        let it = &v.sections[0].items[0];
        assert!(it.came_back.is_some() && it.tags.contains(&"came back".to_owned()));
        // Later hides until its time, news or not.
        seen.insert(("o/r".to_owned(), 40), Seen { later_until: Some(NOW + DAY), ..Seen::default() });
        let v = view(&all, &c, EVERYTHING, &seen, None, NOW);
        assert!(v.sections[0].items.is_empty() && v.later.len() == 1);
        assert_eq!(view(&all, &c, EVERYTHING, &seen, None, NOW + 2 * DAY).sections[0].items.len(), 1);
    }

    #[test]
    fn what_changed_and_jobs() {
        let mut c = cfg();
        c.jobs.insert("Dev".into(), JobCfg { repos: vec!["O/R".into()], ..Default::default() });
        let mut m = pr(50, "me");
        m.state = "MERGED".into();
        m.closed = Some(NOW - DAY);
        let mut old = pr(51, "me");
        old.state = "CLOSED".into();
        old.closed = Some(NOW - 5 * DAY);
        let mut elsewhere = pr(52, "x");
        elsewhere.repo = "a/b".into();
        elsewhere.requests.push(Ev { author: "me".into(), at: NOW });
        let mut broke = pr(53, "me");
        broke.mergeable = "CONFLICTING".into();
        let data = Data { open: vec![elsewhere, broke], closed: vec![m, old], ..Data::default() };
        let all = items(&data, &[Break { repo: "o/r".into(), number: 53, at: NOW - 10, cause: Some(9) }], &ctx(&c));
        let v = view(&all, &c, "Dev", &HashMap::new(), Some(NOW - 2 * DAY), NOW);
        let changed: Vec<&str> = v.sections[7].items.iter().map(|i| i.line.as_str()).collect();
        assert_eq!(changed, ["#9 merged. Your #53 now conflicts.", "Your #50 merged."]);
        assert!(v.sections[1].items.is_empty(), "a/b is not in the job");
        assert_eq!(v.jobs.iter().map(|j| (j.name.as_str(), j.count)).collect::<Vec<_>>(), [("Everything", 2), ("Dev", 1)]);
        assert!(v.since.unwrap().contains("Your #50 merged"));
        // Done on a What changed row hides only that row.
        let mut seen = HashMap::new();
        seen.insert(("o/r".to_owned(), 53 + CHANGED_KEY), Seen { done_at: Some(NOW), ..Seen::default() });
        let v = view(&all, &c, "Dev", &seen, None, NOW);
        assert_eq!(v.sections[7].items.len(), 1);
        assert!(v.sections[0].items.iter().any(|i| i.number == 53));
        // An unknown job is Everything.
        assert_eq!(view(&all, &c, "nope", &HashMap::new(), None, NOW).job, EVERYTHING);
    }

    #[test]
    fn parse_reads_a_search_answer() {
        let data = json!({ "search": { "nodes": [ {
            "number": 8123, "title": "Fix it", "url": "u", "isDraft": false, "state": "OPEN",
            "createdAt": "2026-11-20T10:00:00Z", "updatedAt": "2026-11-22T10:00:00Z", "closedAt": null,
            "author": { "login": "me" }, "repository": { "nameWithOwner": "sigp/lighthouse" },
            "labels": { "nodes": [ { "name": "v9.0.0" } ] }, "milestone": null,
            "additions": 10, "deletions": 2, "mergeable": "CONFLICTING", "reviewDecision": "CHANGES_REQUESTED", "baseRefName": "unstable",
            "mergeQueueEntry": null,
            "reviewRequests": { "nodes": [ { "requestedReviewer": { "combinedSlug": "sigp/core" } } ] },
            "timelineItems": { "nodes": [ { "createdAt": "2026-11-21T00:00:00Z", "requestedReviewer": { "combinedSlug": "sigp/core" } } ] },
            "latestReviews": { "nodes": [ { "author": { "login": "michael" }, "state": "CHANGES_REQUESTED", "submittedAt": "2026-11-22T09:00:00Z" } ] },
            "commits": { "nodes": [ { "commit": { "committedDate": "2026-11-20T10:00:00Z", "authors": { "nodes": [ { "user": { "login": "me" } } ] },
                "statusCheckRollup": { "state": "FAILURE", "contexts": { "nodes": [
                    { "name": "test", "conclusion": "FAILURE", "status": "COMPLETED", "completedAt": "2026-11-20T11:00:00Z" },
                    { "context": "ci/x", "state": "SUCCESS", "createdAt": "2026-11-20T11:00:00Z" } ] } } } } ] },
            "comments": { "nodes": [] },
            "reviewThreads": { "nodes": [ { "isResolved": false, "path": "a.rs", "comments": { "nodes": [ { "author": { "login": "michael" }, "createdAt": "2026-11-22T09:00:00Z" } ] } } ] },
            "files": { "nodes": [ { "path": "a.rs" } ] }
        }, {} ] } });
        let prs = parse_search(&data);
        assert_eq!(prs.len(), 1);
        let p = &prs[0];
        assert_eq!((p.number, p.size, p.mergeable.as_str(), p.base.as_str()), (8123, 12, "CONFLICTING", "unstable"));
        assert_eq!(p.requests, [Ev { author: "sigp/core".into(), at: crate::chat::epoch("2026-11-21T00:00:00Z").unwrap() }]);
        assert_eq!(p.checks.len(), 2);
        assert_eq!(p.checks[0].conclusion, "FAILURE");
        assert_eq!(p.threads[0].last_author, "michael");
        assert!(search_query().contains("search(query: $q"));
    }

    #[test]
    fn text_lists_sections_and_all_done() {
        let c = cfg();
        let v = view(&[], &c, EVERYTHING, &HashMap::new(), None, NOW);
        assert!(text(&v, true).contains("All done for today."));
        let mut p = pr(60, "me");
        p.decision = "APPROVED".into();
        let all = items(&Data { open: vec![p], ..Data::default() }, &[], &ctx(&c));
        let v = view(&all, &c, EVERYTHING, &HashMap::new(), None, NOW);
        let t = text(&v, true);
        assert!(t.contains("YOUR PRS (1)") && t.contains("r#60  Approved. Add it to the merge queue."), "{t}");
        assert_eq!(v.sections[0].items[0].why, "Ranked 1st of 1 in Your PRs: approved and ready.");
    }

    #[test]
    fn no_reviews_skips_your_own_fork_and_names_stay_clear() {
        let mut p = pr(9, "me");
        p.created = NOW - 10 * DAY;
        assert_eq!(rules_of(&run(vec![p.clone()]), 9), ["no_reviews"]);
        p.repo = "Me/lighthouse".into();
        assert!(rules_of(&run(vec![p]), 9).is_empty());
        let n = short_names(["sigp/lighthouse", "me/lighthouse", "o/specs"].into_iter());
        assert_eq!((n["sigp/lighthouse"], n["me/lighthouse"], n["o/specs"]), ("sigp/lighthouse", "me/lighthouse", "specs"));
    }

    #[test]
    fn bots_by_name_and_config() {
        let c = TodayCfg { ignore_bots: vec!["lighthouse-ci".into()], ..cfg() };
        assert!(c.is_bot("dependabot[bot]") && c.is_bot("mergify") && c.is_bot("Lighthouse-CI") && c.is_bot(""));
        assert!(!c.is_bot("michael"));
    }
}
