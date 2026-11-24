//! Today for any repo (SPEC 39.13): issues, and the open PRs of a repo you triage.
//! The rules of a "reports" repo: bug reports with severity labels. Pure: the
//! fetched JSON and the config in, items out.

use crate::today::{ago, Ctx, Item, Pr, Reason, RepoRules, Section};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct IComment {
    pub author: String,
    /// OWNER, MEMBER or COLLABORATOR: the repo's team.
    pub team: bool,
    pub at: u64,
    pub body: String,
}

/// A PR that names the issue.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Fix {
    pub repo: String,
    pub number: u64,
    /// OPEN, MERGED or CLOSED.
    pub state: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Issue {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    pub author_team: bool,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub created: u64,
    pub updated: u64,
    pub comments: Vec<IComment>,
    pub fixes: Vec<Fix>,
}

/// The fields of an issue that the rules read (SPEC 39.13.2).
pub const ISSUE_FIELDS: &str = "number title url createdAt updatedAt author { login } authorAssociation repository { nameWithOwner }
  labels(first: 30) { nodes { name } } assignees(first: 10) { nodes { login } }
  comments(last: 10) { nodes { author { login } authorAssociation createdAt body } }
  timelineItems(itemTypes: [CROSS_REFERENCED_EVENT, CONNECTED_EVENT], last: 10) { nodes {
    ... on CrossReferencedEvent { source { ... on PullRequest { number state repository { nameWithOwner } } } }
    ... on ConnectedEvent { subject { ... on PullRequest { number state repository { nameWithOwner } } } } } }";

/// The search for issues, `n` of them. `$q` is the search text.
pub fn search_query(n: u32) -> String {
    format!("query($q: String!) {{ search(query: $q, type: ISSUE, first: {n}) {{ nodes {{ ... on Issue {{ {ISSUE_FIELDS} }} }} }} }}")
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

fn is_team(assoc: &Value) -> bool {
    matches!(assoc.as_str(), Some("OWNER" | "MEMBER" | "COLLABORATOR"))
}

/// One issue from a search node. `None` for a node that is not an issue.
pub fn parse_issue(n: &Value) -> Option<Issue> {
    let number = n["number"].as_u64()?;
    let repo = n["repository"]["nameWithOwner"].as_str()?.to_owned();
    let mut fixes: Vec<Fix> = vec![];
    for e in nodes(&n["timelineItems"]) {
        let pr = if e["source"].is_object() { &e["source"] } else { &e["subject"] };
        if let (Some(num), Some(r)) = (pr["number"].as_u64(), pr["repository"]["nameWithOwner"].as_str()) {
            if !fixes.iter().any(|f| f.number == num && f.repo == r) {
                fixes.push(Fix { repo: r.to_owned(), number: num, state: s(&pr["state"]) });
            }
        }
    }
    Some(Issue {
        repo,
        number,
        title: s(&n["title"]),
        url: s(&n["url"]),
        author: s(&n["author"]["login"]),
        author_team: is_team(&n["authorAssociation"]),
        labels: nodes(&n["labels"]).map(|l| s(&l["name"])).collect(),
        assignees: nodes(&n["assignees"]).map(|a| s(&a["login"])).collect(),
        created: time(&n["createdAt"]),
        updated: time(&n["updatedAt"]),
        comments: nodes(&n["comments"])
            .map(|c| IComment { author: s(&c["author"]["login"]), team: is_team(&c["authorAssociation"]), at: time(&c["createdAt"]), body: s(&c["body"]) })
            .collect(),
        fixes,
    })
}

/// The issues of a search answer (`data`).
pub fn parse_search(data: &Value) -> Vec<Issue> {
    nodes(&data["search"]).filter_map(parse_issue).collect()
}

/// The severity label of an issue and its place in the list: 0 is the worst.
pub fn severity(labels: &[String], rules: &RepoRules) -> Option<(usize, String)> {
    rules.severity.iter().enumerate().find_map(|(i, s)| labels.iter().find(|l| l.eq_ignore_ascii_case(s)).map(|l| (i, l.clone())))
}

/// The rank bonus of a severity: the worst of five gets 40, the next 30, … 0.
fn severity_bonus(place: usize, of: usize) -> u32 {
    if of <= 1 {
        return 20;
    }
    let step = 40 / (of as u32 - 1).max(1);
    40u32.saturating_sub(step * place as u32)
}

fn days(now: u64, since: u64) -> u64 {
    now.saturating_sub(since) / 86400
}

/// True when the issue involves you: yours, assigned to you, or you wrote on it.
pub fn involves(i: &Issue, ctx: &Ctx) -> bool {
    ctx.mine(&i.author) || i.assignees.iter().any(|a| ctx.mine(a)) || i.comments.iter().any(|c| ctx.mine(&c.author) || c.body.to_lowercase().contains(&format!("@{}", ctx.me)))
}

/// A person on the team: the repo's own, or you. Bots never count.
fn team(c: &IComment, ctx: &Ctx) -> bool {
    !ctx.cfg.is_bot(&c.author) && (c.team || ctx.mine(&c.author))
}

/// The items of the open issues (SPEC 39.13.3). `seen` keeps one row per issue.
pub fn issue_items(issues: &[Issue], ctx: &Ctx, seen: &mut BTreeSet<(String, u64)>) -> Vec<Item> {
    let mut out = vec![];
    for i in issues {
        if !seen.insert((i.repo.to_lowercase(), i.number)) {
            continue;
        }
        let rules = ctx.cfg.rules(&i.repo);
        if i.labels.iter().any(|l| rules.hide_labels.iter().any(|h| h.eq_ignore_ascii_case(l))) {
            continue;
        }
        if let Some(it) = issue_item(i, ctx, &rules) {
            out.push(it);
        }
    }
    out
}

fn issue_item(i: &Issue, ctx: &Ctx, rules: &RepoRules) -> Option<Item> {
    let now = ctx.now;
    let sev = severity(&i.labels, rules);
    let mine = i.assignees.iter().any(|a| ctx.mine(a));
    let my_last = i.comments.iter().filter(|c| ctx.mine(&c.author)).map(|c| c.at).max().unwrap_or(0);
    let people: Vec<&IComment> = i.comments.iter().filter(|c| !ctx.cfg.is_bot(&c.author)).collect();
    let last_team = people.iter().filter(|c| team(c, ctx)).map(|c| c.at).max();
    // A reporter's newest comment, when it is newer than every team comment.
    let reporter_last = people.iter().rev().find(|c| !team(c, ctx)).filter(|c| last_team.is_none_or(|t| c.at > t));
    let mut reasons: Vec<Reason> = vec![];
    let mut detail: Vec<String> = vec![];
    let r = |rule: &str, line: String, weight: u32, at: u64| Reason { rule: rule.into(), line, weight, at };

    // Asked by name since your last comment.
    let asked = people.iter().rev().find(|c| ctx.other(&c.author) && c.at > my_last && c.body.to_lowercase().contains(&format!("@{}", ctx.me)));
    if let Some(c) = asked {
        reasons.push(r("mentioned", format!("{} asked you {}.", c.author, ago(days(now, c.at))), 40, c.at));
    }
    if mine {
        if let Some(c) = reporter_last.filter(|c| c.at > my_last) {
            reasons.push(r("reporter_replied", format!("{} replied {}. Answer.", c.author, ago(days(now, c.at))), 45, c.at));
        }
        let open_fix = i.fixes.iter().find(|f| f.state == "OPEN");
        let any_fix = i.fixes.iter().any(|f| f.state == "OPEN" || f.state == "MERGED");
        match open_fix {
            Some(f) => reasons.push(r("fix_open", format!("Fix PR {}#{} is open: review it.", f.repo, f.number), 25, i.updated)),
            None if !any_fix => reasons.push(r("no_fix", "No fix PR yet.".into(), 30, i.created)),
            None => {}
        }
    }
    if sev.is_none() && !rules.severity.is_empty() {
        reasons.push(r("no_severity", format!("New report, no severity yet. Opened {}.", ago(days(now, i.created))), 35, i.created));
    }
    if last_team.is_none() && days(now, i.created) >= rules.reply_days {
        reasons.push(r("no_team_reply", format!("No reply in {} days.", days(now, i.created)), 30, i.created));
    }
    let waiting = reporter_last.filter(|c| days(now, c.at) >= rules.reply_days && last_team.is_some());
    if let Some(c) = waiting {
        reasons.push(r("reporter_waiting", format!("{} wrote last, {}. No team reply since.", c.author, ago(days(now, c.at))), 30, c.at));
    }
    if i.assignees.is_empty() {
        if let Some((_, name)) = &sev {
            reasons.push(r("unassigned", format!("{name} report with no owner."), 20, i.created));
        }
    }
    if days(now, i.updated) >= rules.stale_days {
        reasons.push(r("stale", format!("No activity in {} days.", days(now, i.updated)), 10, i.updated));
    }

    let section = if mine || asked.is_some() {
        Section::Assigned
    } else if i.assignees.is_empty() || sev.is_none() {
        Section::Triage
    } else if waiting.is_some() {
        Section::Waiting
    } else {
        // Someone on the team owns it and nothing waits: not on the list.
        return None;
    };
    if reasons.is_empty() {
        if section != Section::Triage {
            return None;
        }
        reasons.push(r("unassigned", "A report with no owner.".into(), 15, i.created));
    }
    reasons.sort_by_key(|x| std::cmp::Reverse(x.weight));
    if let Some((place, _)) = &sev {
        reasons[0].weight += severity_bonus(*place, rules.severity.len());
    }

    detail.push(format!("Reporter: {}{}. Opened {}.", i.author, if i.author_team { " (team)" } else { "" }, ago(days(now, i.created))));
    detail.push(match i.assignees.as_slice() {
        [] => "No owner.".into(),
        a => format!("Assigned to {}.", a.join(", ")),
    });
    if let Some(t) = last_team {
        detail.push(format!("Last team comment: {}.", ago(days(now, t))));
    }
    if let Some(c) = reporter_last {
        detail.push(format!("Last reporter comment: {}, {}.", c.author, ago(days(now, c.at))));
    }
    detail.push(match i.fixes.as_slice() {
        [] => "No PR names this report.".into(),
        f => format!("PRs that name it: {}.", f.iter().map(|x| format!("{}#{} ({})", x.repo, x.number, x.state.to_lowercase())).collect::<Vec<_>>().join(", ")),
    });
    let tags: Vec<String> = i.labels.iter().filter(|l| rules.tag_labels.iter().any(|t| t.eq_ignore_ascii_case(l))).cloned().collect();
    let last_event = people.iter().filter(|c| ctx.other(&c.author)).map(|c| c.at).max().unwrap_or(i.created);
    Some(Item {
        repo: i.repo.clone(),
        number: i.number,
        title: i.title.clone(),
        url: i.url.clone(),
        section,
        line: reasons[0].line.clone(),
        tags,
        reasons,
        detail,
        size: 0,
        release: false,
        last_event,
        came_back: None,
        done_at: None,
        later_until: None,
        why: String::new(),
        labels: i.labels.clone(),
        claude: None,
        severity: sev.map(|(_, n)| n),
        issue: true,
        wide: !involves(i, ctx),
    })
}

/// The open PRs of a repo you triage that are not on the list already: proofs of
/// concept in a reports repo (SPEC 39.13.3).
pub fn poc_items(prs: &[Pr], ctx: &Ctx, seen: &mut BTreeSet<(String, u64)>) -> Vec<Item> {
    let mut out = vec![];
    for p in prs {
        if !seen.insert((p.repo.to_lowercase(), p.number)) {
            continue;
        }
        let rules = ctx.cfg.rules(&p.repo);
        if p.labels.iter().any(|l| rules.hide_labels.iter().any(|h| h.eq_ignore_ascii_case(l))) {
            continue;
        }
        let d = days(ctx.now, p.created);
        let line = if p.reviews.is_empty() { format!("Open {d} days, no review.") } else { format!("Open {d} days. Last update {}.", ago(days(ctx.now, p.updated))) };
        out.push(Item {
            repo: p.repo.clone(),
            number: p.number,
            title: p.title.clone(),
            url: p.url.clone(),
            section: Section::Poc,
            line: line.clone(),
            tags: vec![],
            reasons: vec![Reason { rule: "poc".into(), line, weight: 10, at: p.created }],
            detail: vec![format!("By {}. Opened {}.", p.author, ago(d))],
            size: p.size,
            release: false,
            last_event: crate::today::last_event(p, ctx),
            came_back: None,
            done_at: None,
            later_until: None,
            why: String::new(),
            labels: p.labels.clone(),
            claude: None,
            severity: None,
            issue: false,
            wide: !ctx.mine(&p.author),
        });
    }
    out
}

/// What a repo looks like, for Add a job (SPEC 39.13.4): its open counts, labels,
/// and a guess at its kind and severity order.
pub fn repo_guess(v: &Value) -> Value {
    let r = &v["repository"];
    let labels: Vec<String> = nodes(&r["labels"]).map(|l| s(&l["name"])).collect();
    let issues = r["issues"]["totalCount"].as_u64().unwrap_or(0);
    let prs = r["pullRequests"]["totalCount"].as_u64().unwrap_or(0);
    let order = ["critical", "high", "medium", "low", "informational", "info"];
    let mut severity: Vec<String> = vec![];
    for o in order {
        if let Some(l) = labels.iter().find(|l| l.eq_ignore_ascii_case(o)) {
            severity.push(l.clone());
        }
    }
    let reports = severity.len() >= 2 && issues > prs;
    let hide: Vec<&String> = labels.iter().filter(|l| ["wontfix", "duplicate", "invalid", "resolved"].contains(&l.to_lowercase().as_str())).collect();
    serde_json::json!({
        "ok": true, "private": r["isPrivate"].as_bool().unwrap_or(false),
        "issues": issues, "prs": prs, "labels": labels,
        "kind": if reports { "reports" } else { "code" },
        "severity": severity, "hide_labels": hide,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::today::TodayCfg;
    use serde_json::json;

    const NOW: u64 = 1_800_000_000;
    const DAY: u64 = 86400;

    fn at(t: u64) -> String {
        // ISO time of an epoch, as GitHub writes it.
        let d = crate::hub_today::ymd(t);
        let s = t % DAY;
        format!("{d}T{:02}:{:02}:{:02}Z", s / 3600, s % 3600 / 60, s % 60)
    }

    fn cfg() -> TodayCfg {
        let mut c = TodayCfg::default();
        c.repos.insert(
            "eb/lh".into(),
            RepoRules {
                kind: "reports".into(),
                severity: vec!["Critical".into(), "High".into(), "Medium".into(), "Low".into(), "Informational".into()],
                hide_labels: vec!["wontfix".into()],
                tag_labels: vec!["Bug Bounty".into()],
                ..Default::default()
            },
        );
        c
    }

    fn issue(n: u64, extra: Value) -> Issue {
        let mut v = json!({
            "number": n, "title": format!("Report {n}"), "url": "u", "createdAt": at(NOW - 5 * DAY), "updatedAt": at(NOW - DAY),
            "author": { "login": "rep" }, "authorAssociation": "NONE", "repository": { "nameWithOwner": "eb/lh" },
            "labels": { "nodes": [] }, "assignees": { "nodes": [] }, "comments": { "nodes": [] }, "timelineItems": { "nodes": [] }
        });
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        parse_issue(&v).unwrap()
    }

    fn labels(l: &[&str]) -> Value {
        json!({ "nodes": l.iter().map(|x| json!({ "name": x })).collect::<Vec<_>>() })
    }

    fn comment(who: &str, assoc: &str, days_ago: u64, body: &str) -> Value {
        json!({ "author": { "login": who }, "authorAssociation": assoc, "createdAt": at(NOW - days_ago * DAY), "body": body })
    }

    fn one(i: Issue) -> Option<Item> {
        let c = cfg();
        let ctx = Ctx { cfg: &c, me: "me".into(), now: NOW };
        issue_items(&[i], &ctx, &mut BTreeSet::new()).into_iter().next()
    }

    #[test]
    fn a_new_report_needs_triage() {
        let it = one(issue(1, json!({ "labels": labels(&["Bug Bounty"]) }))).unwrap();
        assert_eq!(it.section, Section::Triage);
        assert_eq!(it.reasons[0].rule, "no_severity");
        assert!(it.reasons.iter().any(|r| r.rule == "no_team_reply" && r.line == "No reply in 5 days."));
        assert_eq!((it.tags.clone(), it.issue, it.wide), (vec!["Bug Bounty".to_owned()], true, true));
    }

    #[test]
    fn assigned_to_you_with_a_reporter_reply_ranks_by_severity() {
        let base = json!({ "assignees": { "nodes": [{ "login": "Me" }] }, "comments": { "nodes": [
            comment("me", "MEMBER", 4, "Looking."), comment("rep", "NONE", 2, "More detail.") ] } });
        let mut high = base.clone();
        high["labels"] = labels(&["High"]);
        let mut low = base;
        low["labels"] = labels(&["Low"]);
        let h = one(issue(2, high)).unwrap();
        let l = one(issue(3, low)).unwrap();
        assert_eq!((h.section, h.reasons[0].rule.as_str(), h.line.as_str()), (Section::Assigned, "reporter_replied", "rep replied 2 days ago. Answer."));
        assert!(h.weight() > l.weight(), "High above Low");
        assert_eq!(h.severity.as_deref(), Some("High"));
        assert!(h.reasons.iter().any(|r| r.rule == "no_fix"));
        assert!(!h.wide);
    }

    #[test]
    fn near_misses_do_not_fire() {
        // Your own last comment: no reporter_replied. An open fix PR: fix_open, not no_fix.
        let it = one(issue(4, json!({ "labels": labels(&["Medium"]), "assignees": { "nodes": [{ "login": "me" }] },
            "comments": { "nodes": [ comment("rep", "NONE", 3, "x"), comment("me", "MEMBER", 1, "Fixing.") ] },
            "timelineItems": { "nodes": [ { "source": { "number": 9, "state": "OPEN", "repository": { "nameWithOwner": "sigp/lh" } } } ] } }))).unwrap();
        assert!(!it.reasons.iter().any(|r| r.rule == "reporter_replied" || r.rule == "no_fix"));
        assert_eq!(it.line, "Fix PR sigp/lh#9 is open: review it.");
        // A bot's comment is no team reply; a hidden label never shows.
        let b = one(issue(5, json!({ "labels": labels(&["Low"]), "comments": { "nodes": [ comment("ci-bot", "MEMBER", 1, "x") ] } }))).unwrap();
        assert!(b.reasons.iter().any(|r| r.rule == "no_team_reply"));
        assert!(one(issue(6, json!({ "labels": labels(&["wontfix"]) }))).is_none());
        // Owned by someone on the team, with a team reply last: not on the list.
        assert!(one(issue(7, json!({ "labels": labels(&["Low"]), "assignees": { "nodes": [{ "login": "ann" }] },
            "comments": { "nodes": [ comment("ann", "MEMBER", 1, "On it.") ] } }))).is_none());
    }

    #[test]
    fn a_reporter_waiting_on_the_owner_goes_to_waiting() {
        let it = one(issue(8, json!({ "labels": labels(&["Low"]), "assignees": { "nodes": [{ "login": "ann" }] },
            "comments": { "nodes": [ comment("ann", "MEMBER", 6, "Looking."), comment("rep", "NONE", 3, "Any news?") ] } }))).unwrap();
        assert_eq!((it.section, it.line.as_str()), (Section::Waiting, "rep wrote last, 3 days ago. No team reply since."));
    }

    #[test]
    fn a_mention_comes_to_you() {
        let it = one(issue(9, json!({ "labels": labels(&["Low"]), "assignees": { "nodes": [{ "login": "ann" }] },
            "comments": { "nodes": [ comment("ann", "MEMBER", 1, "@me can you check?") ] } }))).unwrap();
        assert_eq!((it.section, it.line.as_str()), (Section::Assigned, "ann asked you yesterday."));
    }

    #[test]
    fn the_repo_guess_finds_reports_and_their_labels() {
        let names: Vec<Value> = ["bug", "High", "Low", "Critical", "Medium", "wontfix", "Informational"].iter().map(|n| json!({ "name": n })).collect();
        let v = json!({ "repository": { "isPrivate": true, "issues": { "totalCount": 40 }, "pullRequests": { "totalCount": 3 }, "labels": { "nodes": names } } });
        let g = repo_guess(&v);
        assert_eq!(g["kind"], "reports");
        assert_eq!(g["severity"], json!(["Critical", "High", "Medium", "Low", "Informational"]));
        assert_eq!(g["hide_labels"], json!(["wontfix"]));
        let code = json!({ "repository": { "issues": { "totalCount": 10 }, "pullRequests": { "totalCount": 50 }, "labels": { "nodes": [] } } });
        assert_eq!(repo_guess(&code)["kind"], "code");
    }
}
