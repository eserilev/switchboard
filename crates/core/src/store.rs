//! SQLite store (SPEC 16). Schema version in `PRAGMA user_version`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

const SCHEMA: &[&str] = &[
    // Version 1.
    "CREATE TABLE setting (key TEXT PRIMARY KEY, value TEXT NOT NULL);
     CREATE TABLE connection_limit (name TEXT PRIMARY KEY, until INTEGER NOT NULL);
     CREATE TABLE pane (
       id TEXT PRIMARY KEY, kind TEXT NOT NULL, repo TEXT NOT NULL, tree TEXT NOT NULL,
       title TEXT, connection TEXT, session TEXT, lamp TEXT NOT NULL, unseen INTEGER NOT NULL,
       summary TEXT, updated INTEGER NOT NULL, closed INTEGER);
     CREATE TABLE layout (name TEXT PRIMARY KEY, panes TEXT NOT NULL);
     CREATE TABLE review (
       id TEXT PRIMARY KEY, url TEXT NOT NULL, repo TEXT NOT NULL, number INTEGER NOT NULL,
       title TEXT NOT NULL, head TEXT NOT NULL, base TEXT NOT NULL, tree TEXT NOT NULL,
       connection TEXT, guide_session TEXT, guide TEXT, opened INTEGER NOT NULL, closed INTEGER);
     CREATE TABLE step_state (review TEXT, step TEXT, checked INTEGER NOT NULL, stale INTEGER NOT NULL,
       PRIMARY KEY (review, step));
     CREATE TABLE thread (
       id TEXT PRIMARY KEY, review TEXT NOT NULL, step TEXT NOT NULL, path TEXT, side TEXT,
       line INTEGER, line_text TEXT, fork_session TEXT, removed INTEGER NOT NULL DEFAULT 0);
     CREATE TABLE message (id INTEGER PRIMARY KEY, thread TEXT NOT NULL, me INTEGER NOT NULL,
       text TEXT NOT NULL, time INTEGER NOT NULL);
     CREATE TABLE pin (id INTEGER PRIMARY KEY, review TEXT NOT NULL, step TEXT NOT NULL, text TEXT NOT NULL);
     CREATE TABLE draft (id INTEGER PRIMARY KEY, review TEXT NOT NULL, path TEXT, side TEXT,
       line INTEGER, text TEXT NOT NULL);",
    // Version 2: the board order. A new pane has no position and goes last.
    "ALTER TABLE pane ADD COLUMN pos INTEGER;",
    // Version 3: comments you write yourself, line ranges, the review summary, and
    // the reviews sent to GitHub. Older drafts came from the agent.
    "ALTER TABLE draft ADD COLUMN start_line INTEGER;
     ALTER TABLE draft ADD COLUMN agent INTEGER NOT NULL DEFAULT 1;
     ALTER TABLE review ADD COLUMN summary TEXT;
     CREATE TABLE posted (id INTEGER PRIMARY KEY, review TEXT NOT NULL, event TEXT NOT NULL,
       url TEXT NOT NULL, comments INTEGER NOT NULL, time INTEGER NOT NULL);",
    // Version 4: review rounds. A later round reviews only the changes since `since`,
    // and a sent review keeps its comments for the next round.
    "ALTER TABLE review ADD COLUMN round INTEGER NOT NULL DEFAULT 1;
     ALTER TABLE review ADD COLUMN since TEXT;
     ALTER TABLE review ADD COLUMN scope_note TEXT;
     ALTER TABLE posted ADD COLUMN head TEXT;
     ALTER TABLE posted ADD COLUMN body TEXT;
     ALTER TABLE posted ADD COLUMN lines TEXT;",
    // Version 5: a draft keeps the head and the text of its lines, so it can move
    // with the code after an update. `stale`: the text is gone from the new head.
    "ALTER TABLE draft ADD COLUMN head TEXT;
     ALTER TABLE draft ADD COLUMN line_text TEXT;
     ALTER TABLE draft ADD COLUMN stale INTEGER NOT NULL DEFAULT 0;",
    // Version 6: up to 2 lines of code above and below a draft, so a draft on a common
    // line (`}`, a blank line) moves only to the same place. A draft from before
    // version 5 belongs to the head of its review.
    "ALTER TABLE draft ADD COLUMN before TEXT;
     ALTER TABLE draft ADD COLUMN after TEXT;
     UPDATE draft SET head = (SELECT head FROM review WHERE review.id = draft.review)
       WHERE head IS NULL AND line IS NOT NULL;",
    // Version 7: threads keep the head and the code around their line, as drafts do.
    // A draft with no stored code around it can be on a wrong line from an older
    // build, so it is stale once and you place it again.
    "ALTER TABLE thread ADD COLUMN head TEXT;
     ALTER TABLE thread ADD COLUMN before TEXT;
     ALTER TABLE thread ADD COLUMN after TEXT;
     UPDATE draft SET stale = 1 WHERE line IS NOT NULL AND before IS NULL AND after IS NULL;",
];

pub struct Store {
    db: Connection,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PaneRow {
    pub id: String,
    /// `claude`, `nvim` or `shell`.
    pub kind: String,
    pub repo: String,
    pub tree: String,
    pub title: Option<String>,
    pub connection: Option<String>,
    pub session: Option<String>,
    pub lamp: String,
    pub unseen: bool,
    pub summary: Option<String>,
    pub updated: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ReviewRow {
    pub id: String,
    pub url: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub head: String,
    pub base: String,
    pub tree: String,
    pub connection: Option<String>,
    pub guide_session: Option<String>,
    pub guide: Option<String>,
    /// 1 for a full review. A later round reviews only the changes since `since`.
    #[serde(default = "first_round")]
    pub round: u32,
    /// The head that the round before reviewed.
    #[serde(default)]
    pub since: Option<String>,
    /// Why the scope of a round is not a plain diff, for example after a rebase.
    #[serde(default)]
    pub scope_note: Option<String>,
}

fn first_round() -> u32 {
    1
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ThreadRow {
    pub id: String,
    pub review: String,
    pub step: String,
    pub path: Option<String>,
    pub side: Option<String>,
    pub line: Option<u32>,
    pub line_text: Option<String>,
    pub fork_session: Option<String>,
    pub removed: bool,
    /// The head that `line` belongs to, and up to 2 lines of code around it.
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MessageRow {
    pub id: i64,
    pub thread: String,
    pub me: bool,
    pub text: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PinRow {
    pub id: i64,
    pub step: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct DraftRow {
    pub id: i64,
    pub path: Option<String>,
    pub side: Option<String>,
    /// The last line of the comment. With `start_line`, the comment covers a range.
    pub line: Option<u32>,
    #[serde(default)]
    pub start_line: Option<u32>,
    pub text: String,
    /// True when the agent wrote the text. You see and edit it before it goes out.
    #[serde(default)]
    pub agent: bool,
    /// The head that `line` and `start_line` belong to.
    #[serde(default)]
    pub head: Option<String>,
    /// The text of the lines from `start_line` (or `line`) to `line`, joined with "\n".
    #[serde(default)]
    pub line_text: Option<String>,
    /// True when an update did not find `line_text` in the new head. The draft cannot
    /// go out until you place it again.
    #[serde(default)]
    pub stale: bool,
    /// Up to 2 lines above and below the lines, joined with "\n".
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PostedRow {
    pub event: String,
    pub url: String,
    pub comments: u32,
    pub time: u64,
    /// The commit that the review was sent on.
    #[serde(default)]
    pub head: Option<String>,
    /// The summary as sent.
    #[serde(default)]
    pub body: Option<String>,
    /// The line comments as sent, one `path:line: text` for each line.
    #[serde(default)]
    pub lines: Option<String>,
}

type R<T> = rusqlite::Result<T>;

impl Store {
    pub fn open(path: &Path) -> R<Store> {
        Store::init(Connection::open(path)?)
    }

    pub fn memory() -> R<Store> {
        Store::init(Connection::open_in_memory()?)
    }

    fn init(db: Connection) -> R<Store> {
        db.pragma_update(None, "journal_mode", "WAL")?;
        // With WAL, NORMAL is safe and skips a disk sync on every write.
        db.pragma_update(None, "synchronous", "NORMAL")?;
        let version: usize = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (i, sql) in SCHEMA.iter().enumerate().skip(version) {
            db.execute_batch(&format!(
                "BEGIN; {sql}; PRAGMA user_version = {}; COMMIT;",
                i + 1
            ))?;
        }
        Ok(Store { db })
    }

    pub fn version(&self) -> R<usize> {
        self.db
            .pragma_query_value(None, "user_version", |r| r.get(0))
    }

    // ---- settings ----

    pub fn get(&self, key: &str) -> R<Option<String>> {
        self.db
            .query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()
    }

    pub fn set(&self, key: &str, value: &str) -> R<()> {
        self.db.execute("INSERT INTO setting (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2", [key, value])?;
        Ok(())
    }

    /// A new id with a prefix, for example `p12`.
    pub fn next_id(&self, prefix: &str) -> R<String> {
        let key = format!("next_{prefix}");
        let n: u64 = self.get(&key)?.and_then(|v| v.parse().ok()).unwrap_or(1);
        self.set(&key, &(n + 1).to_string())?;
        Ok(format!("{prefix}{n}"))
    }

    // ---- connections ----

    pub fn set_limit(&self, name: &str, until: u64) -> R<()> {
        self.db.execute(
            "INSERT INTO connection_limit (name, until) VALUES (?1, ?2) ON CONFLICT(name) DO UPDATE SET until = ?2",
            params![name, until as i64],
        )?;
        Ok(())
    }

    pub fn limit_until(&self, name: &str) -> R<Option<u64>> {
        let v: Option<i64> = self
            .db
            .query_row(
                "SELECT until FROM connection_limit WHERE name = ?1",
                [name],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.map(|v| v as u64))
    }

    pub fn clear_limit(&self, name: &str) -> R<()> {
        self.db
            .execute("DELETE FROM connection_limit WHERE name = ?1", [name])?;
        Ok(())
    }

    // ---- panes ----

    pub fn save_pane(&self, p: &PaneRow) -> R<()> {
        self.db.execute(
            "INSERT INTO pane (id, kind, repo, tree, title, connection, session, lamp, unseen, summary, updated, closed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL)
             ON CONFLICT(id) DO UPDATE SET kind=?2, repo=?3, tree=?4, title=?5, connection=?6, session=?7,
               lamp=?8, unseen=?9, summary=?10, updated=?11",
            params![p.id, p.kind, p.repo, p.tree, p.title, p.connection, p.session, p.lamp, p.unseen, p.summary, p.updated as i64],
        )?;
        Ok(())
    }

    pub fn open_panes(&self) -> R<Vec<PaneRow>> {
        let mut s = self.db.prepare(
            "SELECT id, kind, repo, tree, title, connection, session, lamp, unseen, summary, updated
             FROM pane WHERE closed IS NULL ORDER BY COALESCE(pos, 1000000000), rowid",
        )?;
        let rows = s.query_map([], |r| {
            Ok(PaneRow {
                id: r.get(0)?,
                kind: r.get(1)?,
                repo: r.get(2)?,
                tree: r.get(3)?,
                title: r.get(4)?,
                connection: r.get(5)?,
                session: r.get(6)?,
                lamp: r.get(7)?,
                unseen: r.get(8)?,
                summary: r.get(9)?,
                updated: r.get::<_, i64>(10)? as u64,
            })
        })?;
        rows.collect()
    }

    /// Saves the board order: each id gets its index as its position.
    pub fn set_order(&self, ids: &[String]) -> R<()> {
        for (i, id) in ids.iter().enumerate() {
            self.db.execute(
                "UPDATE pane SET pos = ?2 WHERE id = ?1",
                params![id, i as i64],
            )?;
        }
        Ok(())
    }

    pub fn close_pane(&self, id: &str, at: u64) -> R<()> {
        self.db.execute(
            "UPDATE pane SET closed = ?2 WHERE id = ?1",
            params![id, at as i64],
        )?;
        Ok(())
    }

    // ---- layouts ----

    pub fn save_layout(&self, name: &str, panes_json: &str) -> R<()> {
        self.db.execute(
            "INSERT INTO layout (name, panes) VALUES (?1, ?2) ON CONFLICT(name) DO UPDATE SET panes = ?2",
            [name, panes_json],
        )?;
        Ok(())
    }

    pub fn layouts(&self) -> R<Vec<(String, String)>> {
        let mut s = self
            .db
            .prepare("SELECT name, panes FROM layout ORDER BY name")?;
        let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    // ---- reviews ----

    pub fn save_review(&self, r: &ReviewRow, opened: u64) -> R<()> {
        self.db.execute(
            "INSERT INTO review (id, url, repo, number, title, head, base, tree, connection, guide_session, guide, opened, round, since, scope_note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(id) DO UPDATE SET title=?5, head=?6, base=?7, tree=?8, connection=?9, guide_session=?10, guide=?11, round=?13, since=?14, scope_note=?15",
            params![r.id, r.url, r.repo, r.number as i64, r.title, r.head, r.base, r.tree, r.connection, r.guide_session, r.guide, opened as i64, r.round, r.since, r.scope_note],
        )?;
        Ok(())
    }

    pub fn open_reviews(&self) -> R<Vec<ReviewRow>> {
        let mut s = self.db.prepare(
            "SELECT id, url, repo, number, title, head, base, tree, connection, guide_session, guide, round, since, scope_note
             FROM review WHERE closed IS NULL ORDER BY opened",
        )?;
        let rows = s.query_map([], |r| {
            Ok(ReviewRow {
                id: r.get(0)?,
                url: r.get(1)?,
                repo: r.get(2)?,
                number: r.get::<_, i64>(3)? as u64,
                title: r.get(4)?,
                head: r.get(5)?,
                base: r.get(6)?,
                tree: r.get(7)?,
                connection: r.get(8)?,
                guide_session: r.get(9)?,
                guide: r.get(10)?,
                round: r.get(11)?,
                since: r.get(12)?,
                scope_note: r.get(13)?,
            })
        })?;
        rows.collect()
    }

    pub fn close_review(&self, id: &str, at: u64) -> R<()> {
        self.db.execute(
            "UPDATE review SET closed = ?2 WHERE id = ?1",
            params![id, at as i64],
        )?;
        Ok(())
    }

    pub fn set_step(
        &self,
        review: &str,
        step: &str,
        checked: Option<bool>,
        stale: Option<bool>,
    ) -> R<()> {
        self.db.execute(
            "INSERT INTO step_state (review, step, checked, stale) VALUES (?1, ?2, COALESCE(?3, 0), COALESCE(?4, 0))
             ON CONFLICT(review, step) DO UPDATE SET checked = COALESCE(?3, checked), stale = COALESCE(?4, stale)",
            params![review, step, checked, stale],
        )?;
        Ok(())
    }

    /// (step, checked, stale) for a review.
    pub fn steps(&self, review: &str) -> R<Vec<(String, bool, bool)>> {
        let mut s = self
            .db
            .prepare("SELECT step, checked, stale FROM step_state WHERE review = ?1")?;
        let rows = s.query_map([review], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    }

    pub fn save_thread(&self, t: &ThreadRow) -> R<()> {
        self.db.execute(
            "INSERT INTO thread (id, review, step, path, side, line, line_text, fork_session, removed, head, before, after)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET line = ?6, line_text = ?7, fork_session = ?8, removed = ?9,
               head = ?10, before = ?11, after = ?12",
            params![t.id, t.review, t.step, t.path, t.side, t.line, t.line_text, t.fork_session, t.removed, t.head, t.before, t.after],
        )?;
        Ok(())
    }

    pub fn threads(&self, review: &str) -> R<Vec<ThreadRow>> {
        let mut s = self.db.prepare(
            "SELECT id, review, step, path, side, line, line_text, fork_session, removed, head, before, after
             FROM thread WHERE review = ?1 ORDER BY rowid",
        )?;
        let rows = s.query_map([review], |r| {
            Ok(ThreadRow {
                id: r.get(0)?,
                review: r.get(1)?,
                step: r.get(2)?,
                path: r.get(3)?,
                side: r.get(4)?,
                line: r.get(5)?,
                line_text: r.get(6)?,
                fork_session: r.get(7)?,
                removed: r.get(8)?,
                head: r.get(9)?,
                before: r.get(10)?,
                after: r.get(11)?,
            })
        })?;
        rows.collect()
    }

    pub fn add_message(&self, thread: &str, me: bool, text: &str, at: u64) -> R<i64> {
        self.db.execute(
            "INSERT INTO message (thread, me, text, time) VALUES (?1, ?2, ?3, ?4)",
            params![thread, me, text, at as i64],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    pub fn messages(&self, review: &str) -> R<Vec<MessageRow>> {
        let mut s = self.db.prepare(
            "SELECT m.id, m.thread, m.me, m.text FROM message m JOIN thread t ON t.id = m.thread
             WHERE t.review = ?1 ORDER BY m.id",
        )?;
        let rows = s.query_map([review], |r| {
            Ok(MessageRow {
                id: r.get(0)?,
                thread: r.get(1)?,
                me: r.get(2)?,
                text: r.get(3)?,
            })
        })?;
        rows.collect()
    }

    pub fn add_pin(&self, review: &str, step: &str, text: &str) -> R<i64> {
        self.db.execute(
            "INSERT INTO pin (review, step, text) VALUES (?1, ?2, ?3)",
            [review, step, text],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    pub fn edit_pin(&self, id: i64, text: &str) -> R<()> {
        self.db
            .execute("UPDATE pin SET text = ?2 WHERE id = ?1", params![id, text])?;
        Ok(())
    }

    pub fn delete_pin(&self, id: i64) -> R<()> {
        self.db.execute("DELETE FROM pin WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn pins(&self, review: &str) -> R<Vec<PinRow>> {
        let mut s = self
            .db
            .prepare("SELECT id, step, text FROM pin WHERE review = ?1 ORDER BY id")?;
        let rows = s.query_map([review], |r| {
            Ok(PinRow {
                id: r.get(0)?,
                step: r.get(1)?,
                text: r.get(2)?,
            })
        })?;
        rows.collect()
    }

    pub fn add_draft(&self, review: &str, d: &DraftRow) -> R<i64> {
        self.db.execute(
            "INSERT INTO draft (review, path, side, line, start_line, text, agent, head, line_text, stale, before, after)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![review, d.path, d.side, d.line, d.start_line, d.text, d.agent, d.head, d.line_text, d.stale, d.before, d.after],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    pub fn edit_draft(&self, id: i64, text: &str) -> R<()> {
        self.db.execute(
            "UPDATE draft SET text = ?2 WHERE id = ?1",
            params![id, text],
        )?;
        Ok(())
    }

    /// Puts a draft on its lines at a head, with the text of the lines and of the code
    /// around them. With `stale`, only the head and the flag change.
    pub fn place_draft(&self, id: i64, head: &str, at: Option<&crate::post::Anchor>) -> R<()> {
        match at {
            Some(a) => self.db.execute(
                "UPDATE draft SET line = ?2, start_line = ?3, head = ?4, line_text = ?5, before = ?6, after = ?7, stale = 0
                 WHERE id = ?1",
                params![id, a.line, a.start_line, head, a.text, a.before, a.after],
            )?,
            None => self.db.execute("UPDATE draft SET head = ?2, stale = 1 WHERE id = ?1", params![id, head])?,
        };
        Ok(())
    }

    pub fn delete_draft(&self, id: i64) -> R<()> {
        self.db.execute("DELETE FROM draft WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn drafts(&self, review: &str) -> R<Vec<DraftRow>> {
        let mut s = self.db.prepare(
            "SELECT id, path, side, line, start_line, text, agent, head, line_text, stale, before, after FROM draft
             WHERE review = ?1 ORDER BY id",
        )?;
        let rows = s.query_map([review], |r| {
            Ok(DraftRow {
                id: r.get(0)?,
                path: r.get(1)?,
                side: r.get(2)?,
                line: r.get(3)?,
                start_line: r.get(4)?,
                text: r.get(5)?,
                agent: r.get(6)?,
                head: r.get(7)?,
                line_text: r.get(8)?,
                stale: r.get(9)?,
                before: r.get(10)?,
                after: r.get(11)?,
            })
        })?;
        rows.collect()
    }

    pub fn summary(&self, review: &str) -> R<String> {
        let s: Option<Option<String>> = self
            .db
            .query_row("SELECT summary FROM review WHERE id = ?1", [review], |r| r.get(0))
            .optional()?;
        Ok(s.flatten().unwrap_or_default())
    }

    pub fn set_summary(&self, review: &str, text: &str) -> R<()> {
        self.db.execute(
            "UPDATE review SET summary = ?2 WHERE id = ?1",
            params![review, text],
        )?;
        Ok(())
    }

    /// Records a review that GitHub accepted, and removes the drafts that it sent.
    /// One transaction, so a draft is never both sent and still pending.
    pub fn record_posted(&self, review: &str, p: &PostedRow, drafts: &[i64]) -> R<()> {
        let tx = self.db.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO posted (review, event, url, comments, time, head, body, lines)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![review, p.event, p.url, p.comments, p.time, p.head, p.body, p.lines],
        )?;
        for d in drafts {
            tx.execute("DELETE FROM draft WHERE id = ?1 AND review = ?2", params![d, review])?;
        }
        tx.execute("UPDATE review SET summary = NULL WHERE id = ?1", [review])?;
        tx.commit()
    }

    /// The reviews sent in one round of a PR, also from a closed round.
    pub fn posted_for(&self, repo: &str, number: u64, round: u32) -> R<Vec<PostedRow>> {
        let mut s = self.db.prepare(
            "SELECT p.event, p.url, p.comments, p.time, p.head, p.body, p.lines FROM posted p
             JOIN review r ON r.id = p.review
             WHERE r.repo = ?1 AND r.number = ?2 AND r.round = ?3 ORDER BY p.id",
        )?;
        let rows = s.query_map(params![repo, number as i64, round], |r| {
            Ok(PostedRow {
                event: r.get(0)?,
                url: r.get(1)?,
                comments: r.get(2)?,
                time: r.get(3)?,
                head: r.get(4)?,
                body: r.get(5)?,
                lines: r.get(6)?,
            })
        })?;
        rows.collect()
    }

    pub fn posted(&self, review: &str) -> R<Vec<PostedRow>> {
        let mut s = self.db.prepare(
            "SELECT event, url, comments, time, head, body, lines FROM posted WHERE review = ?1 ORDER BY id",
        )?;
        let rows = s.query_map([review], |r| {
            Ok(PostedRow {
                event: r.get(0)?,
                url: r.get(1)?,
                comments: r.get(2)?,
                time: r.get(3)?,
                head: r.get(4)?,
                body: r.get(5)?,
                lines: r.get(6)?,
            })
        })?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str) -> PaneRow {
        PaneRow {
            id: id.into(),
            kind: "claude".into(),
            repo: "lighthouse".into(),
            tree: "/x".into(),
            title: None,
            connection: Some("a".into()),
            session: None,
            lamp: "idle".into(),
            unseen: false,
            summary: None,
            updated: 1,
        }
    }

    #[test]
    fn migrates_to_the_latest_version_and_reopens() {
        let path = std::env::temp_dir().join(format!("sb-store-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let s = Store::open(&path).unwrap();
            assert_eq!(s.version().unwrap(), SCHEMA.len());
            s.set("k", "v").unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.get("k").unwrap().as_deref(), Some("v"));
        drop(s);
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
        }
    }

    #[test]
    fn ids_count_up_per_prefix() {
        let s = Store::memory().unwrap();
        assert_eq!(s.next_id("p").unwrap(), "p1");
        assert_eq!(s.next_id("p").unwrap(), "p2");
        assert_eq!(s.next_id("r").unwrap(), "r1");
    }

    #[test]
    fn panes_save_update_and_close() {
        let s = Store::memory().unwrap();
        s.save_pane(&pane("p1")).unwrap();
        s.save_pane(&pane("p2")).unwrap();
        let mut p = pane("p1");
        p.lamp = "turn".into();
        p.unseen = true;
        s.save_pane(&p).unwrap();
        s.close_pane("p2", 5).unwrap();
        assert_eq!(s.open_panes().unwrap(), vec![p]);
    }

    #[test]
    fn order_is_saved_and_new_panes_go_last() {
        let s = Store::memory().unwrap();
        for id in ["p1", "p2", "p3"] {
            s.save_pane(&pane(id)).unwrap();
        }
        s.set_order(&["p3".into(), "p1".into(), "p2".into()])
            .unwrap();
        s.save_pane(&pane("p4")).unwrap();
        let ids: Vec<String> = s.open_panes().unwrap().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["p3", "p1", "p2", "p4"]);
    }

    #[test]
    fn version_1_store_migrates_to_version_2() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(&format!("{}; PRAGMA user_version = 1;", SCHEMA[0]))
            .unwrap();
        db.execute("INSERT INTO pane (id, kind, repo, tree, lamp, unseen, updated) VALUES ('p1', 'shell', 'r', '/', 'none', 0, 1)", []).unwrap();
        let s = Store::init(db).unwrap();
        assert_eq!(s.version().unwrap(), SCHEMA.len());
        assert_eq!(s.open_panes().unwrap().len(), 1);
    }

    /// From every old version, with random rows: the migration ends at the last
    /// version, keeps every row, and gives the new columns their defaults.
    #[test]
    fn every_old_version_migrates_with_its_rows() {
        let mut seed: u64 = 7;
        let mut rnd = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        for from in 1..SCHEMA.len() {
            for _ in 0..20 {
                let db = Connection::open_in_memory().unwrap();
                db.execute_batch(&format!("{}; PRAGMA user_version = {from};", SCHEMA[..from].join(";"))).unwrap();
                let (panes, reviews, drafts) = (rnd(5), rnd(4), rnd(6));
                for i in 0..panes {
                    db.execute("INSERT INTO pane (id, kind, repo, tree, lamp, unseen, updated) VALUES (?1, 'claude', 'r', '/t', 'none', 0, ?2)", params![format!("p{i}"), i as i64]).unwrap();
                }
                for i in 0..reviews {
                    db.execute("INSERT INTO review (id, url, repo, number, title, head, base, tree, opened) VALUES (?1, 'u', 'o/r', ?2, 't', 'h', 'b', '/w', 1)", params![format!("r{i}"), i as i64]).unwrap();
                }
                for i in 0..drafts {
                    db.execute("INSERT INTO draft (review, path, side, line, text) VALUES ('r0', 'a.rs', 'new', ?1, 'd')", [i as i64 + 1]).unwrap();
                }
                let s = Store::init(db).unwrap();
                assert_eq!(s.version().unwrap(), SCHEMA.len());
                assert_eq!(s.open_panes().unwrap().len() as u64, panes);
                let rs = s.open_reviews().unwrap();
                assert_eq!(rs.len() as u64, reviews);
                assert!(rs.iter().all(|r| r.round == 1 && r.since.is_none()));
                let ds = s.drafts("r0").unwrap();
                assert_eq!(ds.len() as u64, drafts);
                // Old drafts on a line have no stored code around them: stale once.
                assert!(ds.iter().all(|d| d.agent && d.start_line.is_none() && d.stale && d.line_text.is_none()));
            }
        }
    }

    #[test]
    fn version_4_drafts_get_the_head_of_their_review() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(&format!("{}; PRAGMA user_version = 4;", SCHEMA[..4].join(";"))).unwrap();
        db.execute("INSERT INTO review (id, url, repo, number, title, head, base, tree, opened) VALUES ('r1', 'u', 'o/r', 1, 't', 'headA', 'b', '/w', 1)", []).unwrap();
        db.execute("INSERT INTO draft (review, path, side, line, text) VALUES ('r1', 'a.rs', 'new', 3, 'on a line')", []).unwrap();
        db.execute("INSERT INTO draft (review, text) VALUES ('r1', 'general')", []).unwrap();
        let s = Store::init(db).unwrap();
        let d = s.drafts("r1").unwrap();
        assert_eq!(d[0].head.as_deref(), Some("headA"));
        assert_eq!(d[1].head, None);
    }

    #[test]
    fn version_2_drafts_migrate_as_agent_drafts() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(&format!("{}; {}; PRAGMA user_version = 2;", SCHEMA[0], SCHEMA[1]))
            .unwrap();
        db.execute("INSERT INTO draft (review, path, side, line, text) VALUES ('r1', 'a.rs', 'new', 3, 'old draft')", []).unwrap();
        let s = Store::init(db).unwrap();
        let d = &s.drafts("r1").unwrap()[0];
        assert!(d.agent);
        assert_eq!((d.line, d.start_line), (Some(3), None));
    }

    #[test]
    fn limits() {
        let s = Store::memory().unwrap();
        assert_eq!(s.limit_until("a").unwrap(), None);
        s.set_limit("a", 100).unwrap();
        s.set_limit("a", 200).unwrap();
        assert_eq!(s.limit_until("a").unwrap(), Some(200));
        s.clear_limit("a").unwrap();
        assert_eq!(s.limit_until("a").unwrap(), None);
    }

    #[test]
    fn review_threads_pins_and_drafts() {
        let s = Store::memory().unwrap();
        let r = ReviewRow {
            id: "r1".into(),
            url: "u".into(),
            repo: "sigp/lighthouse".into(),
            number: 10071,
            title: "t".into(),
            head: "abc".into(),
            base: "def".into(),
            tree: "/wt".into(),
            connection: None,
            guide_session: None,
            guide: None,
            round: 2,
            since: Some("abc0".into()),
            scope_note: None,
        };
        s.save_review(&r, 1).unwrap();
        let t = ThreadRow {
            id: "t1".into(),
            review: "r1".into(),
            step: "s3".into(),
            path: Some("a.rs".into()),
            side: Some("new".into()),
            line: Some(10),
            line_text: Some("x".into()),
            fork_session: None,
            removed: false,
            head: None,
            before: None,
            after: None,
        };
        s.save_thread(&t).unwrap();
        s.add_message("t1", true, "q", 1).unwrap();
        s.add_message("t1", false, "a", 2).unwrap();
        let pin = s.add_pin("r1", "s3", "one line").unwrap();
        s.edit_pin(pin, "edited").unwrap();
        let d = s
            .add_draft(
                "r1",
                &DraftRow {
                    id: 0,
                    path: Some("a.rs".into()),
                    side: None,
                    line: Some(10),
                    start_line: Some(8),
                    text: "nit".into(),
                    agent: false,
                    head: None,
                    line_text: None,
                    stale: false,
                    before: None,
                    after: None,
                },
            )
            .unwrap();
        s.edit_draft(d, "nit: use safe_sub").unwrap();
        s.set_step("r1", "s3", Some(true), None).unwrap();
        s.set_step("r1", "s3", None, Some(true)).unwrap();

        assert_eq!(s.open_reviews().unwrap(), vec![r]);
        assert_eq!(s.threads("r1").unwrap(), vec![t]);
        assert_eq!(s.messages("r1").unwrap().len(), 2);
        assert_eq!(s.pins("r1").unwrap()[0].text, "edited");
        assert_eq!(s.drafts("r1").unwrap()[0].text, "nit: use safe_sub");
        assert_eq!(s.steps("r1").unwrap(), vec![("s3".into(), true, true)]);
        assert_eq!(s.drafts("r1").unwrap()[0].start_line, Some(8));
        s.delete_draft(d).unwrap();
        assert!(s.drafts("r1").unwrap().is_empty());
    }

    #[test]
    fn a_posted_review_removes_only_its_drafts() {
        let s = Store::memory().unwrap();
        let draft = |text: &str| DraftRow {
            id: 0,
            path: Some("a.rs".into()),
            side: Some("new".into()),
            line: Some(1),
            start_line: None,
            text: text.into(),
            agent: false,
            head: None,
            line_text: None,
            stale: false,
            before: None,
            after: None,
        };
        let sent = s.add_draft("r1", &draft("sent")).unwrap();
        s.add_draft("r1", &draft("kept")).unwrap();
        let p = PostedRow {
            event: "COMMENT".into(),
            url: "https://github.com/o/r/pull/1#pullrequestreview-9".into(),
            comments: 1,
            time: 5,
            head: Some("h1".into()),
            body: Some("Please fix.".into()),
            lines: Some("a.rs:1: sent".into()),
        };
        s.record_posted("r1", &p, &[sent]).unwrap();
        let left: Vec<String> = s.drafts("r1").unwrap().into_iter().map(|d| d.text).collect();
        assert_eq!(left, ["kept"]);
        assert_eq!(s.posted("r1").unwrap(), vec![p]);
    }
}
