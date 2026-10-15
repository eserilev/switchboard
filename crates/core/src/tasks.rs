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
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Sprint {
    pub id: String,
    pub name: String,
    pub goal: String,
    /// Local calendar dates, `YYYY-MM-DD`. The end date is a sprint day.
    pub start: String,
    pub end: String,
    /// `open` or `closed`. Planned sprints come in a later phase (SPEC 27.13).
    pub state: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TaskEvent {
    pub at: u64,
    /// `you` (the window), `cli`, or a pane id.
    pub by: String,
    /// `created`, `status`, `sprint`, `edited` or `copied`.
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
}

/// The text fields of a task. An edit sends all of them.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TaskFields {
    pub title: String,
    pub notes: String,
    pub priority: String,
    pub repo: Option<String>,
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
}

/// Where a task that is not done goes when its sprint closes.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Place {
    Next,
    Backlog,
    Done,
}

const TASK_COLS: &str = "id, title, notes, status, blocked_by, priority, sprint, repo, rank, created, changed, started, done";

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

fn get_sprint(c: &Connection, id: &str) -> T<Sprint> {
    c.query_row("SELECT id, name, goal, start_day, end_day, state FROM sprint WHERE id = ?1", [id], sprint_row)
        .optional()
        .map_err(e)?
        .ok_or_else(|| format!("no sprint {id}"))
}

fn save_task(c: &Connection, t: &Task) -> T<()> {
    c.execute(
        "INSERT INTO task (id, title, notes, status, blocked_by, priority, sprint, repo, rank, created, changed, started, done)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(id) DO UPDATE SET title=?2, notes=?3, status=?4, blocked_by=?5, priority=?6, sprint=?7, repo=?8,
           rank=?9, changed=?11, started=?12, done=?13",
        params![
            t.id, t.title, t.notes, t.status, t.blocked_by, t.priority, t.sprint, t.repo, t.rank,
            t.created as i64, t.changed as i64, t.started.map(|v| v as i64), t.done.map(|v| v as i64)
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
        Ok(TasksView { sprints, tasks })
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

    /// Changes the text fields. The history gets one `edited` row.
    pub fn task_edit(&self, id: &str, f: TaskFields, by: &str, at: u64) -> T<Task> {
        let title = check_title(&f.title)?;
        check_priority(&f.priority)?;
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let mut t = get_task(&tx, id)?;
        editable(&tx, &t)?;
        let repo = f.repo.filter(|r| !r.trim().is_empty());
        if t.title == title && t.notes == f.notes && t.priority == f.priority && t.repo == repo {
            return Ok(t);
        }
        t.title = title;
        t.notes = f.notes;
        t.priority = f.priority;
        t.repo = repo;
        t.changed = at;
        save_task(&tx, &t)?;
        event(&tx, id, at, by, "edited", None)?;
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

    /// Removes a task and its history. Phase 1 has no archive (SPEC 27.13).
    pub fn task_delete(&self, id: &str) -> T<()> {
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let t = get_task(&tx, id)?;
        editable(&tx, &t)?;
        tx.execute("DELETE FROM task WHERE id = ?1", [id]).map_err(e)?;
        tx.execute("DELETE FROM task_event WHERE task = ?1", [id]).map_err(e)?;
        tx.execute("DELETE FROM task_pane WHERE task = ?1", [id]).map_err(e)?;
        tx.commit().map_err(e)?;
        Ok(())
    }

    /// A new task in the backlog with the title, notes, priority and repo of `id`.
    /// The way to work again on a task of a closed sprint.
    pub fn task_copy(&self, id: &str, by: &str, at: u64) -> T<Task> {
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let t = get_task(&tx, id)?;
        let n = NewTask { title: t.title, notes: t.notes, priority: Some(t.priority), repo: t.repo, ..NewTask::default() };
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

    /// A new open sprint. At most one sprint is open.
    pub fn sprint_new(&self, s: &NewSprint) -> T<Sprint> {
        let s = check_sprint(s)?;
        let tx = self.db.unchecked_transaction().map_err(e)?;
        let open: i64 = tx.query_row("SELECT COUNT(*) FROM sprint WHERE state = 'open'", [], |r| r.get(0)).map_err(e)?;
        if open > 0 {
            return Err("a sprint is open. Close it first".into());
        }
        let id = self.next_id("s").map_err(e)?;
        tx.execute(
            "INSERT INTO sprint (id, name, goal, start_day, end_day, state) VALUES (?1, ?2, ?3, ?4, ?5, 'open')",
            params![id, s.name, s.goal, s.start, s.end],
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
    /// is not done. `next` opens a new sprint for the tasks that go to `Next`.
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
        if next.is_none() && plan.iter().any(|(_, p)| *p == Place::Next) {
            return Err("the next sprint needs a name and dates".into());
        }
        tx.execute("UPDATE sprint SET state = 'closed' WHERE id = ?1", [id]).map_err(e)?;
        let next_id = match &next {
            Some(s) => {
                let nid = self.next_id("s").map_err(e)?;
                tx.execute(
                    "INSERT INTO sprint (id, name, goal, start_day, end_day, state) VALUES (?1, ?2, ?3, ?4, ?5, 'open')",
                    params![nid, s.name, s.goal, s.start, s.end],
                )
                .map_err(e)?;
                Some(nid)
            }
            None => None,
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
        assert!(s.sprint_new(&sprint("S2")).unwrap_err().contains("is open"));
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
        assert!(s.task_delete(&done.id).is_err());
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
                // Any task, also one in a closed sprint: the rules refuse those changes.
                let ids: Vec<String> = v.tasks.iter().map(|t| t.id.clone()).collect();
                let mut pick = |extra: u64| ids.get(rnd(ids.len() as u64 + extra) as usize).cloned();
                let any = pick(0);
                let before = pick(1);
                let sp = if rnd(2) == 0 { open.clone() } else { None };
                let status = STATUSES[rnd(5) as usize];
                let reason = if rnd(3) == 0 { None } else { Some("someone") };
                match rnd(9) {
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
                                let _ = s.task_delete(id);
                            } else {
                                let _ = s.task_copy(id, "you", at);
                            }
                        }
                    }
                    7 => {
                        assert_eq!(s.sprint_new(&sprint(&format!("S{step}"))).is_ok(), open.is_none());
                    }
                    _ => {
                        if let Some(sp) = &open {
                            let places: Vec<(String, Place)> = v.tasks.iter().filter(|t| t.sprint.as_ref() == Some(sp) && t.status != "done")
                                .map(|t| (t.id.clone(), [Place::Next, Place::Backlog, Place::Done][rnd(3) as usize])).collect();
                            let next = if rnd(2) == 0 { Some(sprint("N")) } else { None };
                            let r = s.sprint_close(sp, &places, next.as_ref(), "you", at);
                            assert_eq!(r.is_ok(), next.is_some() || places.iter().all(|(_, p)| *p != Place::Next));
                        }
                    }
                }
                // The rules.
                let v = s.tasks_view().unwrap();
                assert!(v.sprints.iter().filter(|x| x.state == "open").count() <= 1);
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
                    let last = h.iter().rev().find(|x| x.kind != "sprint" && x.kind != "edited").unwrap();
                    let was = if last.kind == "copied" { Some("todo") } else { last.value.as_deref() };
                    assert_eq!(was, Some(t.status.as_str()), "{t:?} {h:?}");
                }
            }
        }
    }
}
