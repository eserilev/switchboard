//! Tasks and sprints (SPEC 27): the rules, in the store.
//!
//! Every change runs in one transaction and adds one history row. A task in a
//! closed sprint never changes. `rank` orders the tasks of one sprint, or of the
//! backlog; a move renumbers that list, so the ranks in it are always unique.

use crate::store::Store;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

pub const STATUSES: [&str; 5] = ["todo", "doing", "blocked", "review", "done"];
pub const PRIORITIES: [&str; 3] = ["high", "normal", "low"];
/// The gap between two ranks after a renumber.
const STEP: i64 = 1024;

type T<X> = Result<X, String>;

fn e(x: rusqlite::Error) -> String {
    x.to_string()
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub notes: String,
    /// One of `STATUSES`.
    pub status: String,
    /// Who or what blocks the task. Set exactly when the status is `blocked`.
    pub blocked_by: Option<String>,
    pub priority: String,
    /// None: the backlog.
    pub sprint: Option<String>,
    pub repo: Option<String>,
    pub rank: i64,
    pub created: u64,
    pub changed: u64,
    /// The first move to `doing`.
    pub started: Option<u64>,
    /// Set exactly when the status is `done`.
    pub done: Option<u64>,
    /// 0 to 99, or none.
    #[serde(default)]
    pub points: Option<u32>,
    /// Short words, lowercase, for the filter.
    #[serde(default)]
    pub tags: Vec<String>,
    /// http(s) URLs, for example a PR or an issue.
    #[serde(default)]
    pub links: Vec<String>,
    /// Set when the task is in the archive (SPEC 27.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Sprint {
    pub id: String,
    pub name: String,
    pub goal: String,
    /// Local calendar dates, `YYYY-MM-DD`. The end date is a sprint day.
    pub start: String,
    pub end: String,
    /// `planned`, `open` or `closed`. At most one is open and at most one is planned.
    pub state: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TaskEvent {
    pub at: u64,
    /// `you` (the window), `cli`, or a pane id.
    pub by: String,
    /// `created`, `status`, `sprint`, `edited`, `copied`, `points`, `link`,
    /// `link_removed`, `linked`, `unlinked`, `note`, `deleted` or `restored`.
    pub kind: String,
    pub value: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct NewTask {
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub blocked_by: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub sprint: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub points: Option<u32>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub links: Vec<String>,
}

/// The fields of a task that an edit changes. An edit sends all of them.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct TaskFields {
    pub title: String,
    pub notes: String,
    pub priority: String,
    pub repo: Option<String>,
    #[serde(default)]
    pub points: Option<u32>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub links: Vec<String>,
}

/// A drag: into `sprint` (None: the backlog), with an optional new status, before
/// the task `before` or at the end.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct TaskMove {
    #[serde(default)]
    pub sprint: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub blocked_by: Option<String>,
    #[serde(default)]
    pub before: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NewSprint {
    pub name: String,
    #[serde(default)]
    pub goal: String,
    pub start: String,
    pub end: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TasksView {
    pub sprints: Vec<Sprint>,
    pub tasks: Vec<Task>,
    /// The panes linked to tasks (SPEC 27.6), also the closed ones.
    #[serde(default)]
    pub links: Vec<TaskLink>,
    /// The task repo folders that are not on disk. The hub fills it in.
    #[serde(default)]
    pub missing: Vec<String>,
    /// The deleted tasks, newest first (SPEC 27.2).
    #[serde(default)]
    pub archive: Vec<Task>,
    /// The status changes of the tasks in the open sprint, oldest first, for the
    /// burndown (SPEC 27.8).
    #[serde(default)]
    pub burn: Vec<StatusEvent>,
    /// The PR links that GitHub reports as merged. The hub fills it in (SPEC 27.6).
    #[serde(default)]
    pub merged: Vec<String>,
}

/// A status change of a task.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StatusEvent {
    pub task: String,
    pub at: u64,
    pub status: String,
}

/// A pane linked to a task, with what the store knows of the pane. A closed pane
/// keeps its link and its session, so the task can resume it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TaskLink {
    pub pane: String,
    pub task: String,
    pub kind: Option<String>,
    pub title: Option<String>,
    pub tree: Option<String>,
    pub session: Option<String>,
    pub connection: Option<String>,
    pub closed: bool,
}

/// A request on the socket, from `sb task` (SPEC 27.7). `pane` is `SB_PANE` of the
/// caller. It only labels the history: it is not proof of who calls.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TaskOp {
    List {
        #[serde(default)]
        sprint: Option<String>,
        #[serde(default)]
        status: Option<String>,
    },
    Show {
        #[serde(default)]
        id: Option<String>,
    },
    Add {
        title: String,
        #[serde(default)]
        notes: Option<String>,
        #[serde(default)]
        repo: Option<String>,
        #[serde(default)]
        sprint: Option<String>,
    },
    Note {
        id: String,
        text: String,
    },
    Status {
        id: String,
        status: String,
        #[serde(default)]
        blocked_by: Option<String>,
    },
}

impl TaskOp {
    /// True when the request can change the store.
    pub fn writes(&self) -> bool {
        !matches!(self, TaskOp::List { .. } | TaskOp::Show { .. })
    }
}

/// The statuses that the socket can set. `done` is for the window only (SPEC 27.7).
pub const SOCKET_STATUSES: [&str; 4] = ["todo", "doing", "blocked", "review"];

/// A branch name for a task: `<prefix><slug>`. The slug is the title in lowercase
/// ASCII, with `-` for each run of other characters, at most 40 characters; an empty
/// slug is `task-<id>`. When `taken` says a name is in use, `-2`, `-3` and so on
/// follow (SPEC 27.6).
pub fn branch_for(prefix: &str, title: &str, id: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut slug = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let mut slug: String = slug.chars().take(40).collect();
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug = format!("task-{id}");
    }
    let base = format!("{prefix}{slug}");
    if !taken(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}-{n}")).find(|b| !taken(b)).unwrap()
}

/// Where a task that is not done goes when its sprint closes.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Place {
    Next,
    Backlog,
    Done,
}

const TASK_COLS: &str =
    "id, title, notes, status, blocked_by, priority, sprint, repo, rank, created, changed, started, done, points, tags, links, deleted";

/// A JSON list column. A bad value reads as an empty list.
fn list_col(r: &Row, i: usize) -> rusqlite::Result<Vec<String>> {
    Ok(serde_json::from_str(&r.get::<_, String>(i)?).unwrap_or_default())
}

fn task_row(r: &Row) -> rusqlite::Result<Task> {
    let opt = |v: Option<i64>| v.map(|v| v as u64);
    Ok(Task {
        id: r.get(0)?,
        title: r.get(1)?,
        notes: r.get(2)?,
        status: r.get(3)?,
        blocked_by: r.get(4)?,
        priority: r.get(5)?,
        sprint: r.get(6)?,
        repo: r.get(7)?,
        rank: r.get(8)?,
        created: r.get::<_, i64>(9)? as u64,
        changed: r.get::<_, i64>(10)? as u64,
        started: opt(r.get(11)?),
        done: opt(r.get(12)?),
        points: r.get::<_, Option<i64>>(13)?.map(|v| v as u32),
        tags: list_col(r, 14)?,
        links: list_col(r, 15)?,
        deleted: opt(r.get(16)?),
    })
}

fn sprint_row(r: &Row) -> rusqlite::Result<Sprint> {
    Ok(Sprint { id: r.get(0)?, name: r.get(1)?, goal: r.get(2)?, start: r.get(3)?, end: r.get(4)?, state: r.get(5)? })
}

fn get_task(c: &Connection, id: &str) -> T<Task> {
    c.query_row(&format!("SELECT {TASK_COLS} FROM task WHERE id = ?1 AND deleted IS NULL"), [id], task_row)
        .optional()
        .map_err(e)?
        .ok_or_else(|| format!("no task {id}"))
}

fn get_archived(c: &Connection, id: &str) -> T<Task> {
    c.query_row(&format!("SELECT {TASK_COLS} FROM task WHERE id = ?1 AND deleted IS NOT NULL"), [id], task_row)
        .optional()
        .map_err(e)?
        .ok_or_else(|| format!("task {id} is not in the archive"))
}

fn get_sprint(c: &Connection, id: &str) -> T<Sprint> {
    c.query_row("SELECT id, name, goal, start_day, end_day, state FROM sprint WHERE id = ?1", [id], sprint_row)
        .optional()
        .map_err(e)?
        .ok_or_else(|| format!("no sprint {id}"))
}

fn save_task(c: &Connection, t: &Task) -> T<()> {
    let json = |l: &Vec<String>| serde_json::to_string(l).unwrap_or_else(|_| "[]".into());
    c.execute(
        "INSERT INTO task (id, title, notes, status, blocked_by, priority, sprint, repo, rank, created, changed, started, done,
           points, tags, links, deleted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
         ON CONFLICT(id) DO UPDATE SET title=?2, notes=?3, status=?4, blocked_by=?5, priority=?6, sprint=?7, repo=?8,
           rank=?9, changed=?11, started=?12, done=?13, points=?14, tags=?15, links=?16, deleted=?17",
        params![
            t.id, t.title, t.notes, t.status, t.blocked_by, t.priority, t.sprint, t.repo, t.rank,
            t.created as i64, t.changed as i64, t.started.map(|v| v as i64), t.done.map(|v| v as i64),
            t.points.map(|v| v as i64), json(&t.tags), json(&t.links), t.deleted.map(|v| v as i64)
        ],
    )
    .map_err(e)?;
    Ok(())
}

fn event(c: &Connection, task: &str, at: u64, by: &str, kind: &str, value: Option<&str>) -> T<()> {
    c.execute(
        "INSERT INTO task_event (task, at, who, kind, value) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![task, at as i64, by, kind, value],
    )
    .map_err(e)?;
    Ok(())
}

/// A sprint that a task can go into: none (the backlog), or one that is not closed.
fn writable_sprint(c: &Connection, sprint: Option<&str>) -> T<()> {
    match sprint {
        None => Ok(()),
        Some(id) if get_sprint(c, id)?.state == "closed" => Err("that sprint is closed".into()),
        Some(_) => Ok(()),
    }
}

/// A task in a closed sprint never changes (SPEC 27.2).
fn editable(c: &Connection, t: &Task) -> T<()> {
    match t.sprint.as_deref() {
        Some(id) if get_sprint(c, id)?.state == "closed" => {
            Err("this task is in a closed sprint. Copy it to the backlog to work on it again".into())
        }
        _ => Ok(()),
    }
}

fn check_title(title: &str) -> T<String> {
    let t = title.trim();
    if t.is_empty() {
        return Err("a task needs a title".into());
    }
    if t.contains('\n') || t.contains('\r') {
        return Err("a title is one line".into());
    }
    if t.chars().count() > 200 {
        return Err("a title has at most 200 characters".into());
    }
    Ok(t.to_owned())
}

fn check_priority(p: &str) -> T<()> {
    if PRIORITIES.contains(&p) {
        Ok(())
    } else {
        Err(format!("the priority is one of {}", PRIORITIES.join(", ")))
    }
}

fn check_points(p: Option<u32>) -> T<()> {
    match p {
        Some(p) if p > 99 => Err("points are a whole number from 0 to 99".into()),
        _ => Ok(()),
    }
}

/// Tags are short words: lowercase, with no `#` in front, no space, at most 30
/// characters, at most 10 of them. A tag twice counts once.
fn check_tags(tags: &[String]) -> T<Vec<String>> {
    let mut out: Vec<String> = vec![];
    for t in tags {
        let t = t.trim().trim_start_matches('#').to_lowercase();
        if t.is_empty() {
            continue;
        }
        if t.chars().any(|c| c.is_whitespace() || c.is_control() || c == ',') {
            return Err(format!("a tag is one word: {t:?}"));
        }
        if t.chars().count() > 30 {
            return Err("a tag has at most 30 characters".into());
        }
        if !out.contains(&t) {
            out.push(t);
        }
    }
    if out.len() > 10 {
        return Err("a task has at most 10 tags".into());
    }
    Ok(out)
}

/// Links are http(s) URLs with no space, at most 20 of them. A link twice counts once.
fn check_links(links: &[String]) -> T<Vec<String>> {
    let mut out: Vec<String> = vec![];
    for l in links {
        let l = l.trim();
        if l.is_empty() {
            continue;
        }
        let scheme = l.starts_with("https://") || l.starts_with("http://");
        if !scheme || l.chars().any(|c| c.is_whitespace() || c.is_control()) || l.len() > 500 {
            return Err(format!("a link is an http or https URL: {l:?}"));
        }
        if !out.iter().any(|x| x == l) {
            out.push(l.to_owned());
        }
    }
    if out.len() > 20 {
        return Err("a task has at most 20 links".into());
    }
    Ok(out)
}

/// The PR of a GitHub link, as `https://github.com/<owner>/<repo>/pull/<n>`, or None
/// for any other link. `#…`, `?…` and a path after the number do not count.
pub fn pr_url(link: &str) -> Option<String> {
    let rest = link.trim().strip_prefix("https://github.com/")?;
    let mut parts = rest.split(['/', '#', '?']);
    let (owner, repo, pull, n) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let word = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if !word(owner) || !word(repo) || pull != "pull" || n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("https://github.com/{owner}/{repo}/pull/{n}"))
}

/// `YYYY-MM-DD`, with a real month and day.
fn check_day(d: &str) -> T<()> {
    let b = d.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    let ok = b.len() == 10 && b[4] == b'-' && b[7] == b'-' && digits(0..4) && digits(5..7) && digits(8..10) && {
        let m: u32 = d[5..7].parse().unwrap_or(0);
        let day: u32 = d[8..10].parse().unwrap_or(0);
        (1..=12).contains(&m) && (1..=31).contains(&day)
    };
    if ok {
        Ok(())
    } else {
        Err(format!("{d:?} is not a date (YYYY-MM-DD)"))
    }
}

fn check_sprint(s: &NewSprint) -> T<NewSprint> {
    let name = s.name.trim();
    if name.is_empty() {
        return Err("a sprint needs a name".into());
    }
    check_day(&s.start)?;
    check_day(&s.end)?;
    if s.end < s.start {
        return Err("the sprint ends before it starts".into());
    }
    Ok(NewSprint { name: name.to_owned(), goal: s.goal.trim().to_owned(), start: s.start.clone(), end: s.end.clone() })
}

/// Sets the status with its rules. Returns false when nothing changed.
fn apply_status(t: &mut Task, status: &str, blocked_by: Option<&str>, at: u64) -> T<bool> {
    if !STATUSES.contains(&status) {
        return Err(format!("the status is one of {}", STATUSES.join(", ")));
    }
    let blocked_by = if status == "blocked" {
        match blocked_by.map(str::trim).filter(|b| !b.is_empty()) {
            Some(b) => Some(b.to_owned()),
            None => return Err("a blocked task needs who or what blocks it".into()),
        }
    } else {
        None
    };
    if t.status == status && t.blocked_by == blocked_by {
        return Ok(false);
    }
    if status == "done" && t.status != "done" {
        t.done = Some(at);
    }
    if status != "done" {
        t.done = None;
    }
    if status == "doing" && t.started.is_none() {
        t.started = Some(at);
    }
    t.status = status.to_owned();
    t.blocked_by = blocked_by;
    t.changed = at;
    Ok(true)
}

/// Puts `id` in the list of `sprint` (None: the backlog), before `before` or at the
/// end, and renumbers that list. The task row must already have its new sprint.
fn place(c: &Connection, id: &str, sprint: Option<&str>, before: Option<&str>) -> T<()> {
    let mut ids: Vec<String> = {
        let mut s = c
            .prepare("SELECT id FROM task WHERE sprint IS ?1 AND deleted IS NULL AND id != ?2 ORDER BY rank, rowid")
            .map_err(e)?;
        let rows = s.query_map(params![sprint, id], |r| r.get(0)).map_err(e)?;
        rows.collect::<Result<_, _>>().map_err(e)?
    };
    let at = before.and_then(|b| ids.iter().position(|x| x == b)).unwrap_or(ids.len());
    ids.insert(at, id.to_owned());
    for (i, x) in ids.iter().enumerate() {
        c.execute("UPDATE task SET rank = ?1 WHERE id = ?2", params![(i as i64 + 1) * STEP, x]).map_err(e)?;
    }
    Ok(())
}

impl Store {
    /// Every sprint and every task.
    pub fn tasks_view(&self) -> T<TasksView> {
        let mut s = self
            .db
            .prepare("SELECT id, name, goal, start_day, end_day, state FROM sprint ORDER BY start_day, rowid")
            .map_err(e)?;
        let sprints = s.query_map([], sprint_row).map_err(e)?.collect::<Result<_, _>>().map_err(e)?;
        let mut s = self
            .db
            .prepare(&format!("SELECT {TASK_COLS} FROM task WHERE deleted IS NULL ORDER BY sprint, rank, rowid"))
            .map_err(e)?;
        let tasks = s.query_map([], task_row).map_err(e)?.collect::<Result<_, _>>().map_err(e)?;
        let mut s = self
            .db
            .prepare(&format!("SELECT {TASK_COLS} FROM task WHERE deleted IS NOT NULL ORDER BY deleted DESC, rowid DESC"))
            .map_err(e)?;
        let archive = s.query_map([], task_row).map_err(e)?.collect::<Result<_, _>>().map_err(e)?;
        let mut s = self
            .db
            .prepare(
                "SELECT e.task, e.at, e.value FROM task_event e JOIN task t ON t.id = e.task AND t.deleted IS NULL
                 JOIN sprint s ON s.id = t.sprint AND s.state = 'open'
                 WHERE e.kind IN ('status', 'created') AND e.value IS NOT NULL ORDER BY e.id",
            )
            .map_err(e)?;
        let burn = s
            .query_map([], |r| Ok(StatusEvent { task: r.get(0)?, at: r.get::<_, i64>(1)? as u64, status: r.get(2)? }))
            .map_err(e)?
            .collect::<Result<_, _>>()
            .map_err(e)?;
        let links = self.task_links()?;
        Ok(TasksView { sprints, tasks, links, missing: vec![], archive, burn, merged: vec![] })
    }

    pub fn task(&self, id: &str) -> T<Task> {
        get_task(&self.db, id)
    }

    pub fn task_add(&self, n: NewTask, by: &str, at: u64) -> T<Task> {
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let id = self.add_in(&tx, n, by, at, None)?;
        tx.commit().map_err(e)?;
        get_task(&self.db, &id)
    }

    /// Adds a task in the open transaction. The first history row is `created` with
    /// the status, or `copied` with the id of the task it copies.
    fn add_in(&self, tx: &Connection, n: NewTask, by: &str, at: u64, copy_of: Option<&str>) -> T<String> {
        let title = check_title(&n.title)?;
        let priority = n.priority.unwrap_or_else(|| "normal".into());
        check_priority(&priority)?;
        check_points(n.points)?;
        let tags = check_tags(&n.tags)?;
        let links = check_links(&n.links)?;
        writable_sprint(tx, n.sprint.as_deref())?;
        let id = self.next_id("t").map_err(e)?;
        let mut t = Task {
            id,
            title,
            notes: n.notes,
            status: "todo".into(),
            blocked_by: None,
            priority,
            sprint: n.sprint,
            repo: n.repo.filter(|r| !r.trim().is_empty()),
            rank: 0,
            created: at,
            changed: at,
            started: None,
            done: None,
            points: n.points,
            tags,
            links,
            deleted: None,
        };
        let status = n.status.unwrap_or_else(|| "todo".into());
        apply_status(&mut t, &status, n.blocked_by.as_deref(), at)?;
        save_task(tx, &t)?;
        place(tx, &t.id, t.sprint.as_deref(), None)?;
        match copy_of {
            Some(from) => event(tx, &t.id, at, by, "copied", Some(from))?,
            None => event(tx, &t.id, at, by, "created", Some(&t.status))?,
        }
        Ok(t.id)
    }

    /// Changes the fields. The history gets a `points` row for new points, a `link`
    /// or `link_removed` row for each link, and one `edited` row for the rest.
    pub fn task_edit(&self, id: &str, f: TaskFields, by: &str, at: u64) -> T<Task> {
        let title = check_title(&f.title)?;
        check_priority(&f.priority)?;
        check_points(f.points)?;
        let tags = check_tags(&f.tags)?;
        let links = check_links(&f.links)?;
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let mut t = get_task(&tx, id)?;
        editable(&tx, &t)?;
        let repo = f.repo.filter(|r| !r.trim().is_empty());
        let points = t.points != f.points;
        let added: Vec<String> = links.iter().filter(|l| !t.links.contains(l)).cloned().collect();
        let removed: Vec<String> = t.links.iter().filter(|l| !links.contains(l)).cloned().collect();
        // A new order of the same links is an edit too.
        let reorder = added.is_empty() && removed.is_empty() && t.links != links;
        let text = t.title != title || t.notes != f.notes || t.priority != f.priority || t.repo != repo || t.tags != tags || reorder;
        if !text && !points && added.is_empty() && removed.is_empty() {
            return Ok(t);
        }
        t.title = title;
        t.notes = f.notes;
        t.priority = f.priority;
        t.repo = repo;
        t.tags = tags;
        t.points = f.points;
        t.links = links;
        t.changed = at;
        save_task(&tx, &t)?;
        if text {
            event(&tx, id, at, by, "edited", None)?;
        }
        if points {
            event(&tx, id, at, by, "points", t.points.map(|p| p.to_string()).as_deref())?;
        }
        for l in &added {
            event(&tx, id, at, by, "link", Some(l))?;
        }
        for l in &removed {
            event(&tx, id, at, by, "link_removed", Some(l))?;
        }
        tx.commit().map_err(e)?;
        Ok(t)
    }

    pub fn task_status(&self, id: &str, status: &str, blocked_by: Option<&str>, by: &str, at: u64) -> T<Task> {
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let mut t = get_task(&tx, id)?;
        editable(&tx, &t)?;
        if apply_status(&mut t, status, blocked_by, at)? {
            save_task(&tx, &t)?;
            event(&tx, id, at, by, "status", Some(status))?;
            tx.commit().map_err(e)?;
        }
        Ok(t)
    }

    pub fn task_move(&self, id: &str, m: &TaskMove, by: &str, at: u64) -> T<Task> {
        let (sprint, status, blocked_by, before) =
            (m.sprint.as_deref(), m.status.as_deref(), m.blocked_by.as_deref(), m.before.as_deref());
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let mut t = get_task(&tx, id)?;
        editable(&tx, &t)?;
        writable_sprint(&tx, sprint)?;
        let status_changed = match status {
            Some(s) => {
                // A drag inside Blocked keeps the reason.
                let reason = blocked_by.map(str::to_owned).or(t.blocked_by.clone());
                apply_status(&mut t, s, reason.as_deref(), at)?
            }
            None => false,
        };
        let sprint_changed = t.sprint.as_deref() != sprint;
        t.sprint = sprint.map(str::to_owned);
        t.changed = at;
        save_task(&tx, &t)?;
        place(&tx, id, sprint, before)?;
        if status_changed {
            event(&tx, id, at, by, "status", Some(&t.status))?;
        }
        if sprint_changed {
            event(&tx, id, at, by, "sprint", sprint)?;
        }
        tx.commit().map_err(e)?;
        get_task(&self.db, id)
    }

    /// Moves a task to the archive. It loses its pane links; its panes stay.
    pub fn task_delete(&self, id: &str, by: &str, at: u64) -> T<()> {
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let mut t = get_task(&tx, id)?;
        editable(&tx, &t)?;
        t.deleted = Some(at);
        t.changed = at;
        save_task(&tx, &t)?;
        tx.execute("DELETE FROM task_pane WHERE task = ?1", [id]).map_err(e)?;
        event(&tx, id, at, by, "deleted", None)?;
        tx.commit().map_err(e)?;
        Ok(())
    }

    /// Takes a task out of the archive, at the end of its list. When its sprint
    /// closed in the meantime, it goes to the backlog.
    pub fn task_restore(&self, id: &str, by: &str, at: u64) -> T<Task> {
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let mut t = get_archived(&tx, id)?;
        if let Some(sp) = t.sprint.clone() {
            let gone = match get_sprint(&tx, &sp) {
                Ok(x) => x.state == "closed",
                Err(_) => true,
            };
            if gone {
                t.sprint = None;
            }
        }
        t.deleted = None;
        t.changed = at;
        save_task(&tx, &t)?;
        place(&tx, id, t.sprint.as_deref(), None)?;
        event(&tx, id, at, by, "restored", t.sprint.as_deref())?;
        tx.commit().map_err(e)?;
        get_task(&self.db, id)
    }

    /// Removes the tasks that went to the archive before `before`, with their history.
    /// Returns how many.
    pub fn task_prune(&self, before: u64) -> T<usize> {
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let gone = "SELECT id FROM task WHERE deleted IS NOT NULL AND deleted < ?1";
        tx.execute(&format!("DELETE FROM task_event WHERE task IN ({gone})"), [before as i64]).map_err(e)?;
        tx.execute(&format!("DELETE FROM task_pane WHERE task IN ({gone})"), [before as i64]).map_err(e)?;
        let n = tx.execute("DELETE FROM task WHERE deleted IS NOT NULL AND deleted < ?1", [before as i64]).map_err(e)?;
        tx.commit().map_err(e)?;
        Ok(n)
    }

    /// A new task in the backlog with the fields of `id`, in To do. The way to work
    /// again on a task of a closed sprint.
    pub fn task_copy(&self, id: &str, by: &str, at: u64) -> T<Task> {
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let t = get_task(&tx, id)?;
        let n = NewTask {
            title: t.title,
            notes: t.notes,
            priority: Some(t.priority),
            repo: t.repo,
            points: t.points,
            tags: t.tags,
            links: t.links,
            ..NewTask::default()
        };
        let new = self.add_in(&tx, n, by, at, Some(id))?;
        tx.commit().map_err(e)?;
        get_task(&self.db, &new)
    }

    pub fn task_history(&self, id: &str) -> T<Vec<TaskEvent>> {
        let mut s = self
            .db
            .prepare("SELECT at, who, kind, value FROM task_event WHERE task = ?1 ORDER BY id")
            .map_err(e)?;
        let rows = s
            .query_map([id], |r| {
                Ok(TaskEvent { at: r.get::<_, i64>(0)? as u64, by: r.get(1)?, kind: r.get(2)?, value: r.get(3)? })
            })
            .map_err(e)?;
        rows.collect::<Result<_, _>>().map_err(e)
    }

    /// A new sprint. With no open sprint, it opens. With an open one, it is the
    /// planned next sprint. At most one is open and at most one is planned.
    pub fn sprint_new(&self, s: &NewSprint) -> T<Sprint> {
        let s = check_sprint(s)?;
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let count = |state: &str| -> T<i64> {
            tx.query_row("SELECT COUNT(*) FROM sprint WHERE state = ?1", [state], |r| r.get(0)).map_err(e)
        };
        let state = match (count("open")?, count("planned")?) {
            (0, _) => "open",
            (_, 0) => "planned",
            _ => return Err("the next sprint is planned already. Close the open sprint first".into()),
        };
        let id = self.next_id("s").map_err(e)?;
        tx.execute(
            "INSERT INTO sprint (id, name, goal, start_day, end_day, state) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, s.name, s.goal, s.start, s.end, state],
        )
        .map_err(e)?;
        tx.commit().map_err(e)?;
        get_sprint(&self.db, &id)
    }

    pub fn sprint_edit(&self, id: &str, s: &NewSprint) -> T<Sprint> {
        let s = check_sprint(s)?;
        if get_sprint(&self.db, id)?.state == "closed" {
            return Err("that sprint is closed".into());
        }
        self.db
            .execute(
                "UPDATE sprint SET name = ?2, goal = ?3, start_day = ?4, end_day = ?5 WHERE id = ?1",
                params![id, s.name, s.goal, s.start, s.end],
            )
            .map_err(e)?;
        get_sprint(&self.db, id)
    }

    /// Closes the open sprint `id`. `places` gives a place for each task in it that
    /// is not done. A planned sprint opens, and the tasks that go to `Next` go there.
    /// With no planned sprint, `next` opens a new one for them.
    pub fn sprint_close(
        &self,
        id: &str,
        places: &[(String, Place)],
        next: Option<&NewSprint>,
        by: &str,
        at: u64,
    ) -> T<Option<Sprint>> {
        let next = next.map(check_sprint).transpose()?;
        let tx = self.db.unchecked_transaction().map_err(e)?;
        if get_sprint(&tx, id)?.state != "open" {
            return Err("that sprint is not open".into());
        }
        let open: Vec<Task> = {
            let mut s = tx
                .prepare(&format!(
                    "SELECT {TASK_COLS} FROM task WHERE sprint = ?1 AND deleted IS NULL AND status != 'done' ORDER BY rank"
                ))
                .map_err(e)?;
            let rows = s.query_map([id], task_row).map_err(e)?;
            rows.collect::<Result<_, _>>().map_err(e)?
        };
        let mut plan = vec![];
        for t in &open {
            match places.iter().find(|(x, _)| x == &t.id) {
                Some((_, p)) => plan.push((t.clone(), *p)),
                None => return Err(format!("choose a place for \"{}\"", t.title)),
            }
        }
        let planned: Option<String> =
            tx.query_row("SELECT id FROM sprint WHERE state = 'planned'", [], |r| r.get(0)).optional().map_err(e)?;
        if planned.is_some() && next.is_some() {
            return Err("the next sprint is planned already".into());
        }
        if planned.is_none() && next.is_none() && plan.iter().any(|(_, p)| *p == Place::Next) {
            return Err("the next sprint needs a name and dates".into());
        }
        tx.execute("UPDATE sprint SET state = 'closed' WHERE id = ?1", [id]).map_err(e)?;
        let next_id = match &next {
            None => {
                if let Some(p) = &planned {
                    tx.execute("UPDATE sprint SET state = 'open' WHERE id = ?1", [p]).map_err(e)?;
                }
                planned
            }
            Some(s) => {
                let nid = self.next_id("s").map_err(e)?;
                tx.execute(
                    "INSERT INTO sprint (id, name, goal, start_day, end_day, state) VALUES (?1, ?2, ?3, ?4, ?5, 'open')",
                    params![nid, s.name, s.goal, s.start, s.end],
                )
                .map_err(e)?;
                Some(nid)
            }
        };
        for (mut t, p) in plan {
            match p {
                Place::Done => {
                    apply_status(&mut t, "done", None, at)?;
                    save_task(&tx, &t)?;
                    event(&tx, &t.id, at, by, "status", Some("done"))?;
                }
                Place::Next | Place::Backlog => {
                    let to = if p == Place::Next { next_id.clone() } else { None };
                    t.sprint = to.clone();
                    t.changed = at;
                    save_task(&tx, &t)?;
                    place(&tx, &t.id, to.as_deref(), None)?;
                    event(&tx, &t.id, at, by, "sprint", to.as_deref())?;
                }
            }
        }
        tx.commit().map_err(e)?;
        next_id.map(|n| get_sprint(&self.db, &n)).transpose()
    }
}

impl Store {
    pub fn task_links(&self) -> T<Vec<TaskLink>> {
        let mut s = self
            .db
            .prepare(
                "SELECT l.pane, l.task, p.kind, p.title, p.tree, COALESCE(p.session, l.session), p.connection, p.closed IS NOT NULL OR p.id IS NULL
                 FROM task_pane l JOIN task t ON t.id = l.task AND t.deleted IS NULL LEFT JOIN pane p ON p.id = l.pane
                 ORDER BY l.rowid",
            )
            .map_err(e)?;
        let rows = s
            .query_map([], |r| {
                Ok(TaskLink {
                    pane: r.get(0)?,
                    task: r.get(1)?,
                    kind: r.get(2)?,
                    title: r.get(3)?,
                    tree: r.get(4)?,
                    session: r.get(5)?,
                    connection: r.get(6)?,
                    closed: r.get(7)?,
                })
            })
            .map_err(e)?;
        rows.collect::<Result<_, _>>().map_err(e)
    }

    /// Links `pane` to `task`. A pane links to at most one task: a second link moves it.
    pub fn task_link(&self, task: &str, pane: &str, session: Option<&str>, by: &str, at: u64) -> T<()> {
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let t = get_task(&tx, task)?;
        editable(&tx, &t)?;
        let was: Option<String> =
            tx.query_row("SELECT task FROM task_pane WHERE pane = ?1", [pane], |r| r.get(0)).optional().map_err(e)?;
        if was.as_deref() == Some(task) {
            return Ok(());
        }
        tx.execute(
            "INSERT INTO task_pane (pane, task, session) VALUES (?1, ?2, ?3)
             ON CONFLICT(pane) DO UPDATE SET task = ?2, session = ?3",
            params![pane, task, session],
        )
        .map_err(e)?;
        if let Some(old) = was {
            event(&tx, &old, at, by, "unlinked", Some(pane))?;
        }
        event(&tx, task, at, by, "linked", Some(pane))?;
        tx.commit().map_err(e)
    }

    pub fn task_unlink(&self, pane: &str, by: &str, at: u64) -> T<()> {
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let was: Option<String> =
            tx.query_row("SELECT task FROM task_pane WHERE pane = ?1", [pane], |r| r.get(0)).optional().map_err(e)?;
        let Some(task) = was else { return Ok(()) };
        tx.execute("DELETE FROM task_pane WHERE pane = ?1", [pane]).map_err(e)?;
        event(&tx, &task, at, by, "unlinked", Some(pane))?;
        tx.commit().map_err(e)
    }

    /// The task linked to `pane`.
    pub fn task_of_pane(&self, pane: &str) -> T<Option<String>> {
        self.db.query_row("SELECT task FROM task_pane WHERE pane = ?1", [pane], |r| r.get(0)).optional().map_err(e)
    }

    /// Adds a note at the end of the notes, with who wrote it. The history keeps the text.
    pub fn task_note(&self, id: &str, text: &str, by: &str, at: u64) -> T<Task> {
        let text = text.trim();
        if text.is_empty() {
            return Err("the note is empty".into());
        }
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let mut t = get_task(&tx, id)?;
        editable(&tx, &t)?;
        let head = format!("**Note from {by}:**");
        t.notes = if t.notes.trim().is_empty() { format!("{head}\n{text}") } else { format!("{}\n\n{head}\n{text}", t.notes.trim_end()) };
        t.changed = at;
        save_task(&tx, &t)?;
        event(&tx, id, at, by, "note", Some(text))?;
        tx.commit().map_err(e)?;
        Ok(t)
    }

    /// One `sb task` request. The socket never sets `done`, never deletes and never
    /// closes a sprint, whoever calls (SPEC 27.7). `by` labels the history.
    pub fn task_request(&self, op: TaskOp, pane: Option<&str>, by: &str, at: u64) -> T<serde_json::Value> {
        use serde_json::json;
        let v = self.tasks_view()?;
        // `current`, `backlog`, or a sprint name. Some(None) is the backlog.
        let sprint_of = |name: &str, open_only: bool| -> T<Option<String>> {
            match name {
                "backlog" => Ok(None),
                "current" => v.sprints.iter().find(|s| s.state == "open").map(|s| Some(s.id.clone())).ok_or("no sprint is open".into()),
                n => v
                    .sprints
                    .iter()
                    .rev()
                    .find(|s| (s.name == n || s.id == n) && (!open_only || s.state != "closed"))
                    .map(|s| Some(s.id.clone()))
                    .ok_or(format!("no sprint {n:?}")),
            }
        };
        match op {
            TaskOp::List { sprint, status } => {
                let sp = sprint.as_deref().map(|n| sprint_of(n, false)).transpose()?;
                let tasks: Vec<&Task> = v
                    .tasks
                    .iter()
                    .filter(|t| sp.as_ref().is_none_or(|s| &t.sprint == s))
                    .filter(|t| status.as_deref().is_none_or(|s| t.status == s))
                    .collect();
                Ok(json!({ "tasks": tasks, "sprints": v.sprints }))
            }
            TaskOp::Show { id } => {
                let id = match id {
                    Some(id) => id,
                    None => {
                        let p = pane.ok_or("give a task id. Outside a Switchboard pane there is no linked task")?;
                        self.task_of_pane(p)?.ok_or("no task is linked to this pane")?
                    }
                };
                let t = get_task(&self.db, &id)?;
                let sprint = t.sprint.as_ref().and_then(|s| v.sprints.iter().find(|x| &x.id == s)).map(|s| s.name.clone());
                let panes: Vec<&TaskLink> = v.links.iter().filter(|l| l.task == id).collect();
                Ok(json!({ "task": t, "sprint": sprint, "panes": panes, "history": self.task_history(&id)? }))
            }
            TaskOp::Add { title, notes, repo, sprint } => {
                let sp = match sprint.as_deref() {
                    Some(n) => sprint_of(n, true)?,
                    None => None,
                };
                let n = NewTask { title, notes: notes.unwrap_or_default(), repo, sprint: sp, ..NewTask::default() };
                Ok(json!({ "task": self.task_add(n, by, at)? }))
            }
            TaskOp::Note { id, text } => Ok(json!({ "task": self.task_note(&id, &text, by, at)? })),
            TaskOp::Status { id, status, blocked_by } => {
                if !SOCKET_STATUSES.contains(&status.as_str()) {
                    return Err(if status == "done" {
                        "only the Switchboard window sets a task to done".into()
                    } else {
                        format!("the status is one of {}", SOCKET_STATUSES.join(", "))
                    });
                }
                if get_task(&self.db, &id)?.status == "done" {
                    return Err("this task is done. Only the Switchboard window opens it again".into());
                }
                Ok(json!({ "task": self.task_status(&id, &status, blocked_by.as_deref(), by, at)? }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(s: &Store, title: &str, sprint: Option<&str>) -> Task {
        s.task_add(NewTask { title: title.into(), sprint: sprint.map(Into::into), ..NewTask::default() }, "you", 10).unwrap()
    }

    fn sprint(name: &str) -> NewSprint {
        NewSprint { name: name.into(), goal: "ship".into(), start: "2026-10-12".into(), end: "2026-10-25".into() }
    }

    #[test]
    fn status_rules() {
        let s = Store::memory().unwrap();
        let t = add(&s, "Write the spec", None);
        assert_eq!((t.status.as_str(), t.priority.as_str(), t.done, t.started), ("todo", "normal", None, None));
        assert!(s.task_status(&t.id, "blocked", None, "you", 11).unwrap_err().contains("blocks it"));
        assert!(s.task_status(&t.id, "blocked", Some("  "), "you", 11).is_err());
        let t = s.task_status(&t.id, "blocked", Some("review from Sean"), "you", 11).unwrap();
        assert_eq!(t.blocked_by.as_deref(), Some("review from Sean"));
        let t = s.task_status(&t.id, "doing", None, "you", 12).unwrap();
        assert_eq!((t.blocked_by, t.started), (None, Some(12)));
        let t = s.task_status(&t.id, "done", None, "you", 13).unwrap();
        assert_eq!(t.done, Some(13));
        let t = s.task_status(&t.id, "doing", None, "you", 14).unwrap();
        assert_eq!((t.done, t.started), (None, Some(12)), "started stays at the first move");
        assert!(s.task_status(&t.id, "finished", None, "you", 15).is_err());
        // The same status again adds no history row.
        s.task_status(&t.id, "doing", None, "you", 16).unwrap();
        let kinds: Vec<String> = s.task_history(&t.id).unwrap().into_iter().map(|h| h.kind).collect();
        assert_eq!(kinds, ["created", "status", "status", "status", "status"]);
    }

    #[test]
    fn titles_priorities_and_dates_are_checked() {
        let s = Store::memory().unwrap();
        for bad in ["", "   ", "two\nlines", &"x".repeat(201)] {
            assert!(s.task_add(NewTask { title: bad.into(), ..NewTask::default() }, "you", 1).is_err(), "{bad:?}");
        }
        assert_eq!(add(&s, "  trim me ", None).title, "trim me");
        assert!(s.task_add(NewTask { title: "t".into(), priority: Some("urgent".into()), ..NewTask::default() }, "you", 1).is_err());
        for (start, end) in [("2026-13-01", "2026-13-02"), ("2026-10-1", "2026-10-20"), ("2026-10-20", "2026-10-19")] {
            assert!(s.sprint_new(&NewSprint { name: "x".into(), goal: "".into(), start: start.into(), end: end.into() }).is_err());
        }
        assert!(s.sprint_new(&NewSprint { name: " ".into(), ..sprint("x") }).is_err());
        // A one-day sprint is fine: the end date is a sprint day.
        assert!(s.sprint_new(&NewSprint { end: "2026-10-12".into(), ..sprint("x") }).is_ok());
    }

    #[test]
    fn moves_keep_the_order_and_the_history() {
        let s = Store::memory().unwrap();
        let sp = s.sprint_new(&sprint("S1")).unwrap();
        assert_eq!(s.sprint_new(&sprint("S2")).unwrap().state, "planned", "a second sprint is the planned one");
        let a = add(&s, "a", Some(&sp.id));
        let b = add(&s, "b", Some(&sp.id));
        let c = add(&s, "c", None);
        // c from the backlog into the sprint, before b, as In progress.
        s.task_move(&c.id, &TaskMove { sprint: Some(sp.id.clone()), status: Some("doing".into()), blocked_by: None, before: Some(b.id.clone()) }, "you", 20).unwrap();
        let v = s.tasks_view().unwrap();
        let order: Vec<&str> = v.tasks.iter().filter(|t| t.sprint.as_deref() == Some(sp.id.as_str())).map(|t| t.title.as_str()).collect();
        assert_eq!(order, ["a", "c", "b"]);
        let kinds: Vec<String> = s.task_history(&c.id).unwrap().into_iter().map(|h| h.kind).collect();
        assert_eq!(kinds, ["created", "status", "sprint"]);
        // A move to Blocked needs the reason.
        assert!(s.task_move(&a.id, &TaskMove { sprint: Some(sp.id.clone()), status: Some("blocked".into()), blocked_by: None, before: None }, "you", 21).is_err());
        assert_eq!(s.task(&a.id).unwrap().status, "todo", "a refused move changes nothing");
        // A move inside the list keeps the status and adds no history row.
        s.task_move(&b.id, &TaskMove { sprint: Some(sp.id.clone()), status: None, blocked_by: None, before: Some(a.id.clone()) }, "you", 22).unwrap();
        assert_eq!(s.task_history(&b.id).unwrap().len(), 1);
    }

    #[test]
    fn close_moves_each_open_task_and_freezes_the_sprint() {
        let s = Store::memory().unwrap();
        let sp = s.sprint_new(&sprint("S1")).unwrap();
        let done = add(&s, "done", Some(&sp.id));
        s.task_status(&done.id, "done", None, "you", 11).unwrap();
        let next = add(&s, "next", Some(&sp.id));
        let back = add(&s, "back", Some(&sp.id));
        let fin = add(&s, "finish", Some(&sp.id));
        // Every open task needs a place, and Next needs a sprint.
        assert!(s.sprint_close(&sp.id, &[(next.id.clone(), Place::Backlog)], None, "you", 12).unwrap_err().contains("choose a place"));
        let places = vec![(next.id.clone(), Place::Next), (back.id.clone(), Place::Backlog), (fin.id.clone(), Place::Done)];
        assert!(s.sprint_close(&sp.id, &places, None, "you", 12).unwrap_err().contains("next sprint"));
        let s2 = s.sprint_close(&sp.id, &places, Some(&sprint("S2")), "you", 12).unwrap().unwrap();
        assert_eq!(s2.state, "open");
        assert_eq!(s.task(&next.id).unwrap().sprint.as_deref(), Some(s2.id.as_str()));
        assert_eq!(s.task(&back.id).unwrap().sprint, None);
        let f = s.task(&fin.id).unwrap();
        assert_eq!((f.status.as_str(), f.done, f.sprint.as_deref()), ("done", Some(12), Some(sp.id.as_str())));
        // The closed sprint is read-only.
        assert!(s.task_status(&done.id, "doing", None, "you", 13).unwrap_err().contains("closed sprint"));
        assert!(s.task_delete(&done.id, "you", 13).is_err());
        assert!(s.task_move(&back.id, &TaskMove { sprint: Some(sp.id.clone()), status: None, blocked_by: None, before: None }, "you", 13).is_err());
        assert!(s.sprint_edit(&sp.id, &sprint("again")).is_err());
        // Copy to backlog makes a new task.
        let c = s.task_copy(&done.id, "you", 14).unwrap();
        assert_eq!((c.title.as_str(), c.status.as_str(), c.sprint.clone()), ("done", "todo", None));
        let h = s.task_history(&c.id).unwrap();
        assert_eq!((h.len(), h[0].kind.as_str(), h[0].value.as_deref()), (1, "copied", Some(done.id.as_str())), "one history row");
        // Close with no next sprint leaves no sprint open.
        s.sprint_close(&s2.id, &[(next.id.clone(), Place::Backlog)], None, "you", 15).unwrap();
        assert!(s.tasks_view().unwrap().sprints.iter().all(|x| x.state == "closed"));
    }

    /// Random adds, moves, status changes, deletes, copies and sprint closes. After
    /// each step the rules of SPEC 27 hold.
    #[test]
    fn random_actions_keep_the_rules() {
        let mut seed: u64 = 7;
        let mut rnd = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n.max(1)
        };
        for _run in 0..20 {
            let s = Store::memory().unwrap();
            for step in 0..150u64 {
                let at = 100 + step;
                let v = s.tasks_view().unwrap();
                let closed = |v: &TasksView| -> Vec<Task> {
                    v.tasks.iter().filter(|t| t.sprint.as_ref().is_some_and(|sp| v.sprints.iter().any(|x| &x.id == sp && x.state == "closed"))).cloned().collect()
                };
                let frozen = closed(&v);
                let open = v.sprints.iter().find(|x| x.state == "open").map(|x| x.id.clone());
                let planned = v.sprints.iter().find(|x| x.state == "planned").map(|x| x.id.clone());
                // Any task, also one in a closed sprint: the rules refuse those changes.
                let ids: Vec<String> = v.tasks.iter().map(|t| t.id.clone()).collect();
                let mut pick = |extra: u64| ids.get(rnd(ids.len() as u64 + extra) as usize).cloned();
                let any = pick(0);
                let before = pick(1);
                let sp = match rnd(3) {
                    0 => open.clone(),
                    1 => planned.clone(),
                    _ => None,
                };
                let status = STATUSES[rnd(5) as usize];
                let reason = if rnd(3) == 0 { None } else { Some("someone") };
                match rnd(10) {
                    0 | 1 => {
                        let _ = s.task_add(NewTask { title: format!("t{step}"), sprint: sp, ..NewTask::default() }, "you", at);
                    }
                    2 | 3 => {
                        if let Some(id) = &any {
                            let _ = s.task_status(id, status, reason, "you", at);
                        }
                    }
                    4 | 5 => {
                        if let Some(id) = &any {
                            let st = if rnd(2) == 0 { Some(status) } else { None };
                            let _ = s.task_move(id, &TaskMove { sprint: sp.as_deref().map(Into::into), status: st.map(Into::into), blocked_by: reason.map(Into::into), before: before.as_deref().map(Into::into) }, "you", at);
                        }
                    }
                    6 => {
                        if let Some(id) = &any {
                            if rnd(3) == 0 {
                                let _ = s.task_delete(id, "you", at);
                            } else {
                                let _ = s.task_copy(id, "you", at);
                            }
                        }
                    }
                    7 => {
                        assert_eq!(s.sprint_new(&sprint(&format!("S{step}"))).is_ok(), open.is_none() || planned.is_none());
                    }
                    8 => {
                        if let Some(t) = v.archive.get(rnd(v.archive.len() as u64) as usize) {
                            let back = s.task_restore(&t.id, "you", at).unwrap();
                            assert!(back.sprint.is_none() || back.sprint == open || back.sprint == planned, "a restore never goes into a closed sprint");
                        }
                    }
                    _ => {
                        if let Some(sp) = &open {
                            let places: Vec<(String, Place)> = v.tasks.iter().filter(|t| t.sprint.as_ref() == Some(sp) && t.status != "done")
                                .map(|t| (t.id.clone(), [Place::Next, Place::Backlog, Place::Done][rnd(3) as usize])).collect();
                            let next = if rnd(2) == 0 { Some(sprint("N")) } else { None };
                            let r = s.sprint_close(sp, &places, next.as_ref(), "you", at);
                            let ok = match &planned {
                                Some(_) => next.is_none(),
                                None => next.is_some() || places.iter().all(|(_, p)| *p != Place::Next),
                            };
                            assert_eq!(r.is_ok(), ok, "{r:?}");
                            if let (Ok(opened), Some(_)) = (&r, &planned) {
                                assert_eq!(opened.clone(), Some(s.tasks_view().unwrap().sprints.into_iter().find(|x| x.state == "open").unwrap()), "the planned sprint opens");
                            }
                        }
                    }
                }
                // The rules.
                let v = s.tasks_view().unwrap();
                let n_open = v.sprints.iter().filter(|x| x.state == "open").count();
                let n_planned = v.sprints.iter().filter(|x| x.state == "planned").count();
                assert!(n_open <= 1 && n_planned <= 1, "at most one open and one planned sprint");
                assert!(n_planned == 0 || n_open == 1, "a planned sprint comes after an open one");
                assert!(v.archive.iter().all(|t| t.deleted.is_some()) && v.tasks.iter().all(|t| t.deleted.is_none()));
                assert!(v.links.iter().all(|l| v.tasks.iter().any(|t| t.id == l.task)), "an archived task has no links");
                let mut keys: Vec<(Option<String>, i64)> = v.tasks.iter().map(|t| (t.sprint.clone(), t.rank)).collect();
                let n = keys.len();
                keys.sort();
                keys.dedup();
                assert_eq!(keys.len(), n, "ranks are unique in each list");
                let now_closed = closed(&v);
                for t in &frozen {
                    assert!(now_closed.contains(t), "a task of a closed sprint never changes: {t:?}");
                }
                for t in &v.tasks {
                    assert!(STATUSES.contains(&t.status.as_str()));
                    assert_eq!(t.status == "done", t.done.is_some(), "{t:?}");
                    assert_eq!(t.status == "blocked", t.blocked_by.is_some(), "{t:?}");
                    if let Some(sp) = &t.sprint {
                        let sp = v.sprints.iter().find(|x| &x.id == sp).expect("the sprint exists");
                        if sp.state == "closed" {
                            assert_eq!(t.status, "done", "a closed sprint holds only done tasks");
                        }
                    }
                    // The history starts with `created` or `copied` (a copy starts as To do),
                    // and its last status is the status.
                    let h = s.task_history(&t.id).unwrap();
                    assert!(h[0].kind == "created" || h[0].kind == "copied", "{h:?}");
                    let last = h.iter().rev().find(|x| ["created", "status", "copied"].contains(&x.kind.as_str())).unwrap();
                    let was = if last.kind == "copied" { Some("todo") } else { last.value.as_deref() };
                    assert_eq!(was, Some(t.status.as_str()), "{t:?} {h:?}");
                }
            }
        }
    }

    #[test]
    fn points_tags_and_links() {
        let s = Store::memory().unwrap();
        let t = add(&s, "a", None);
        let f = |points, tags: &[&str], links: &[&str]| TaskFields {
            title: "a".into(),
            notes: "".into(),
            priority: "normal".into(),
            repo: None,
            points,
            tags: tags.iter().map(|x| x.to_string()).collect(),
            links: links.iter().map(|x| x.to_string()).collect(),
        };
        assert!(s.task_edit(&t.id, f(Some(100), &[], &[]), "you", 2).unwrap_err().contains("0 to 99"));
        assert!(s.task_edit(&t.id, f(None, &["two words"], &[]), "you", 2).is_err());
        assert!(s.task_edit(&t.id, f(None, &[&"x".repeat(31)], &[]), "you", 2).is_err());
        let many: Vec<String> = (0..11).map(|i| format!("t{i}")).collect();
        assert!(s.task_edit(&t.id, f(None, &many.iter().map(|x| x.as_str()).collect::<Vec<_>>(), &[]), "you", 2).is_err());
        for bad in ["javascript:alert(1)", "ftp://x", "https://a b", "file:///etc/passwd"] {
            assert!(s.task_edit(&t.id, f(None, &[], &[bad]), "you", 2).is_err(), "{bad}");
        }
        let pr = "https://github.com/sigp/lighthouse/pull/10071";
        let t2 = s.task_edit(&t.id, f(Some(3), &["#Infra", "infra", " ci "], &[pr, pr, "https://example.com/x"]), "you", 3).unwrap();
        assert_eq!((t2.points, t2.tags.clone(), t2.links.len()), (Some(3), vec!["infra".to_owned(), "ci".to_owned()], 2));
        assert_eq!(s.task(&t.id).unwrap(), t2, "the fields stay in the store");
        // Points 0 is a value, not none.
        let t3 = s.task_edit(&t.id, f(Some(0), &["infra", "ci"], &[pr]), "you", 4).unwrap();
        assert_eq!(t3.points, Some(0));
        let h: Vec<(String, Option<String>)> = s.task_history(&t.id).unwrap().into_iter().skip(1).map(|x| (x.kind, x.value)).collect();
        let row = |k: &str, v: Option<&str>| (k.to_owned(), v.map(str::to_owned));
        assert_eq!(h, [
            row("edited", None), row("points", Some("3")), row("link", Some(pr)), row("link", Some("https://example.com/x")),
            row("points", Some("0")), row("link_removed", Some("https://example.com/x")),
        ]);
        // The same fields again add no row. A copy keeps them.
        s.task_edit(&t.id, f(Some(0), &["infra", "ci"], &[pr]), "you", 5).unwrap();
        assert_eq!(s.task_history(&t.id).unwrap().len(), 7);
        let c = s.task_copy(&t.id, "you", 6).unwrap();
        assert_eq!((c.points, c.tags, c.links), (Some(0), t3.tags, t3.links));
    }

    #[test]
    fn the_archive_keeps_a_task_for_restore() {
        let s = Store::memory().unwrap();
        let sp = s.sprint_new(&sprint("S1")).unwrap();
        let a = add(&s, "a", Some(&sp.id));
        let b = add(&s, "b", Some(&sp.id));
        let c = add(&s, "c", Some(&sp.id));
        s.task_delete(&b.id, "you", 20).unwrap();
        let v = s.tasks_view().unwrap();
        assert_eq!(v.tasks.len(), 2);
        assert_eq!((v.archive.len(), v.archive[0].deleted), (1, Some(20)));
        assert!(s.task(&b.id).is_err() && s.task_status(&b.id, "doing", None, "you", 21).is_err(), "an archived task does not change");
        assert!(s.task_restore(&a.id, "you", 21).is_err(), "only an archived task restores");
        let back = s.task_restore(&b.id, "you", 22).unwrap();
        let order: Vec<String> = s.tasks_view().unwrap().tasks.into_iter().map(|t| t.title).collect();
        assert_eq!((back.sprint.as_deref(), order), (Some(sp.id.as_str()), vec!["a".into(), "c".into(), "b".into()]), "it comes back at the end");
        // Its sprint closed while it was in the archive: it comes back to the backlog.
        s.task_delete(&c.id, "you", 23).unwrap();
        s.sprint_close(&sp.id, &[(a.id.clone(), Place::Done), (b.id.clone(), Place::Done)], None, "you", 24).unwrap();
        let back = s.task_restore(&c.id, "you", 25).unwrap();
        assert_eq!(back.sprint, None);
        let kinds: Vec<String> = s.task_history(&c.id).unwrap().into_iter().map(|h| h.kind).collect();
        assert_eq!(kinds, ["created", "deleted", "restored"]);
        // The prune removes what went to the archive before the cut, with its history.
        s.task_delete(&c.id, "you", 30).unwrap();
        let d = add(&s, "d", None);
        s.task_delete(&d.id, "you", 50).unwrap();
        assert_eq!(s.task_prune(40).unwrap(), 1);
        let v = s.tasks_view().unwrap();
        assert_eq!(v.archive.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["d"]);
        assert!(s.task_history(&c.id).unwrap().is_empty());
    }

    #[test]
    fn a_planned_sprint_opens_at_close() {
        let s = Store::memory().unwrap();
        let s1 = s.sprint_new(&sprint("S1")).unwrap();
        let s2 = s.sprint_new(&sprint("S2")).unwrap();
        assert_eq!((s1.state.as_str(), s2.state.as_str()), ("open", "planned"));
        assert!(s.sprint_new(&sprint("S3")).unwrap_err().contains("planned already"));
        // You plan the next sprint: tasks go into it, and it can change.
        let p = add(&s, "planned", Some(&s2.id));
        assert!(s.sprint_edit(&s2.id, &NewSprint { goal: "later".into(), ..sprint("S2") }).is_ok());
        let a = add(&s, "a", Some(&s1.id));
        // A new sprint at close is refused when one is planned.
        assert!(s.sprint_close(&s1.id, &[(a.id.clone(), Place::Next)], Some(&sprint("X")), "you", 5).unwrap_err().contains("planned already"));
        let opened = s.sprint_close(&s1.id, &[(a.id.clone(), Place::Next)], None, "you", 5).unwrap().unwrap();
        assert_eq!((opened.id.as_str(), opened.state.as_str()), (s2.id.as_str(), "open"));
        let order: Vec<String> = s.tasks_view().unwrap().tasks.into_iter().filter(|t| t.sprint.as_deref() == Some(s2.id.as_str())).map(|t| t.title).collect();
        assert_eq!(order, [p.title.clone(), "a".into()], "the moved task goes after the planned ones");
        // Now a new sprint is planned again.
        assert_eq!(s.sprint_new(&sprint("S3")).unwrap().state, "planned");
    }

    #[test]
    fn the_burn_has_the_status_changes_of_the_open_sprint() {
        let s = Store::memory().unwrap();
        let sp = s.sprint_new(&sprint("S1")).unwrap();
        let a = add(&s, "a", Some(&sp.id));
        let b = add(&s, "b", None);
        s.task_status(&a.id, "done", None, "you", 30).unwrap();
        s.task_status(&b.id, "done", None, "you", 31).unwrap();
        let burn = s.tasks_view().unwrap().burn;
        let rows: Vec<(&str, u64, &str)> = burn.iter().map(|x| (x.task.as_str(), x.at, x.status.as_str())).collect();
        assert_eq!(rows, [(a.id.as_str(), 10, "todo"), (a.id.as_str(), 30, "done")], "only the tasks of the open sprint");
    }

    #[test]
    fn pr_urls() {
        let n = "https://github.com/sigp/lighthouse/pull/10071";
        for ok in [n, "https://github.com/sigp/lighthouse/pull/10071/files", "https://github.com/sigp/lighthouse/pull/10071#issuecomment-1", " https://github.com/sigp/lighthouse/pull/10071?x=1"] {
            assert_eq!(pr_url(ok).as_deref(), Some(n), "{ok}");
        }
        for no in ["https://github.com/sigp/lighthouse/issues/1", "https://github.com/sigp/lighthouse/pull/", "https://github.com/sigp/lighthouse/pull/12a", "http://github.com/a/b/pull/1", "https://gitlab.com/a/b/pull/1", "https://github.com/a b/c/pull/1", "https://github.com/a/b/pull/1;rm"] {
            assert_eq!(pr_url(no), None, "{no}");
        }
    }

    #[test]
    fn branch_names() {
        let none = |_: &str| false;
        assert_eq!(branch_for("sb/", "Fix the slot-0 test!", "t1", none), "sb/fix-the-slot-0-test");
        assert_eq!(branch_for("sb/", "  ÉÀ 🎉 ", "t7", none), "sb/task-t7");
        assert_eq!(branch_for("", &"a".repeat(50), "t1", none).len(), 40);
        assert_eq!(branch_for("sb/", "word ".repeat(10).as_str(), "t1", none), "sb/word-word-word-word-word-word-word-word");
        let taken = |b: &str| b == "sb/x" || b == "sb/x-2";
        assert_eq!(branch_for("sb/", "X", "t1", taken), "sb/x-3");
    }

    #[test]
    fn links_move_and_stay_after_the_pane_closes() {
        let s = Store::memory().unwrap();
        let a = add(&s, "a", None);
        let b = add(&s, "b", None);
        s.db.execute("INSERT INTO pane (id, kind, repo, tree, title, session, lamp, unseen, updated) VALUES ('p1', 'claude', 'r', '/t', 'agent', 'sess1', 'idle', 0, 1)", []).unwrap();
        s.task_link(&a.id, "p1", None, "you", 2).unwrap();
        s.task_link(&b.id, "p1", None, "you", 3).unwrap();
        let l = s.task_links().unwrap();
        assert_eq!(l.len(), 1, "a pane links to at most one task");
        assert_eq!((l[0].task.as_str(), l[0].session.as_deref(), l[0].closed), (b.id.as_str(), Some("sess1"), false));
        let kinds = |id: &str| s.task_history(id).unwrap().into_iter().map(|h| h.kind).collect::<Vec<_>>();
        assert_eq!(kinds(&a.id), ["created", "linked", "unlinked"]);
        s.close_pane("p1", 4).unwrap();
        assert!(s.task_links().unwrap()[0].closed, "the link stays and shows ended");
        s.task_unlink("p1", "you", 5).unwrap();
        assert!(s.task_links().unwrap().is_empty());
        s.task_link(&b.id, "p1", None, "you", 6).unwrap();
        s.task_delete(&b.id, "you", 7).unwrap();
        assert!(s.task_links().unwrap().is_empty(), "a deleted task loses its links");
        s.task_restore(&b.id, "you", 8).unwrap();
        assert!(s.task_links().unwrap().is_empty(), "a restore does not bring the links back");
    }

    #[test]
    fn socket_requests() {
        let s = Store::memory().unwrap();
        let sp = s.sprint_new(&sprint("S1")).unwrap();
        let r = s.task_request(TaskOp::Add { title: "From an agent".into(), notes: None, repo: None, sprint: Some("current".into()) }, Some("p3"), "p3", 1).unwrap();
        let id = r["task"]["id"].as_str().unwrap().to_owned();
        assert_eq!(r["task"]["sprint"], sp.id.as_str());
        let err = s.task_request(TaskOp::Status { id: id.clone(), status: "done".into(), blocked_by: None }, None, "cli", 2).unwrap_err();
        assert!(err.contains("only the Switchboard window"), "{err}");
        s.task_request(TaskOp::Status { id: id.clone(), status: "review".into(), blocked_by: None }, Some("p3"), "p3", 3).unwrap();
        s.task_request(TaskOp::Note { id: id.clone(), text: "tests pass".into() }, Some("p3"), "p3", 4).unwrap();
        s.task_request(TaskOp::Note { id: id.clone(), text: "second".into() }, None, "cli", 5).unwrap();
        let t = s.task(&id).unwrap();
        assert_eq!(t.notes, "**Note from p3:**\ntests pass\n\n**Note from cli:**\nsecond");
        let h = s.task_history(&id).unwrap();
        assert_eq!((h[1].by.as_str(), h[1].kind.as_str()), ("p3", "status"));
        // Show with no id: the task linked to the pane.
        assert!(s.task_request(TaskOp::Show { id: None }, Some("p3"), "p3", 6).unwrap_err().contains("no task is linked"));
        s.db.execute("INSERT INTO pane (id, kind, repo, tree, lamp, unseen, updated) VALUES ('p3', 'claude', 'r', '/t', 'idle', 0, 1)", []).unwrap();
        s.task_link(&id, "p3", None, "you", 7).unwrap();
        let show = s.task_request(TaskOp::Show { id: None }, Some("p3"), "p3", 8).unwrap();
        assert_eq!((show["task"]["id"].as_str(), show["sprint"].as_str()), (Some(id.as_str()), Some("S1")));
        let list = s.task_request(TaskOp::List { sprint: Some("backlog".into()), status: None }, None, "cli", 9).unwrap();
        assert_eq!(list["tasks"].as_array().unwrap().len(), 0);
        assert!(s.task_request(TaskOp::List { sprint: Some("nope".into()), status: None }, None, "cli", 9).is_err());
    }

    /// Random socket requests, with random ids, statuses and sprints, never set a task
    /// to done, never remove a task, and never change a sprint (SPEC 27.7).
    #[test]
    fn the_socket_never_finishes_deletes_or_closes() {
        let mut seed: u64 = 11;
        let mut rnd = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n.max(1)
        };
        for _run in 0..10 {
            let s = Store::memory().unwrap();
            s.sprint_new(&sprint("S1")).unwrap();
            for i in 0..6 {
                let t = add(&s, &format!("seed {i}"), None);
                // Some tasks are done: the socket never opens them again.
                if i % 2 == 0 {
                    s.task_status(&t.id, "done", None, "you", 1).unwrap();
                }
            }
            for step in 0..300u64 {
                let before = s.tasks_view().unwrap();
                let ids: Vec<String> = before.tasks.iter().map(|t| t.id.clone()).chain(["t999".to_owned()]).collect();
                let id = ids[rnd(ids.len() as u64) as usize].clone();
                let words = ["todo", "doing", "blocked", "review", "done", "DONE", "", "current", "backlog", "S1"];
                let w = || words[(step as usize * 7 + id.len()) % words.len()].to_owned();
                let op = match rnd(5) {
                    0 => TaskOp::List { sprint: Some(w()), status: Some(w()) },
                    1 => TaskOp::Show { id: Some(id.clone()) },
                    2 => TaskOp::Add { title: format!("x{step}"), notes: None, repo: None, sprint: Some(w()) },
                    3 => TaskOp::Note { id: id.clone(), text: w() },
                    _ => TaskOp::Status { id: id.clone(), status: STATUSES[rnd(5) as usize].into(), blocked_by: Some("x".into()) },
                };
                let _ = s.task_request(op, Some("p1"), "p1", 100 + step);
                let after = s.tasks_view().unwrap();
                assert_eq!(after.sprints, before.sprints, "the socket never changes a sprint");
                for t in &before.tasks {
                    let now = after.tasks.iter().find(|x| x.id == t.id).expect("the socket never removes a task");
                    assert_eq!(now.status == "done", t.status == "done", "the socket never sets or clears done: {t:?}");
                }
                assert!(after.tasks.iter().filter(|t| !before.tasks.iter().any(|b| b.id == t.id)).all(|t| t.status != "done"));
            }
        }
    }
}
