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
    pub line: Option<u32>,
    pub text: String,
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
            "INSERT INTO review (id, url, repo, number, title, head, base, tree, connection, guide_session, guide, opened)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET title=?5, head=?6, base=?7, tree=?8, connection=?9, guide_session=?10, guide=?11",
            params![r.id, r.url, r.repo, r.number as i64, r.title, r.head, r.base, r.tree, r.connection, r.guide_session, r.guide, opened as i64],
        )?;
        Ok(())
    }

    pub fn open_reviews(&self) -> R<Vec<ReviewRow>> {
        let mut s = self.db.prepare(
            "SELECT id, url, repo, number, title, head, base, tree, connection, guide_session, guide
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
            "INSERT INTO thread (id, review, step, path, side, line, line_text, fork_session, removed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET line = ?6, line_text = ?7, fork_session = ?8, removed = ?9",
            params![t.id, t.review, t.step, t.path, t.side, t.line, t.line_text, t.fork_session, t.removed],
        )?;
        Ok(())
    }

    pub fn threads(&self, review: &str) -> R<Vec<ThreadRow>> {
        let mut s = self.db.prepare(
            "SELECT id, review, step, path, side, line, line_text, fork_session, removed FROM thread WHERE review = ?1 ORDER BY rowid",
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
            "INSERT INTO draft (review, path, side, line, text) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![review, d.path, d.side, d.line, d.text],
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

    pub fn delete_draft(&self, id: i64) -> R<()> {
        self.db.execute("DELETE FROM draft WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn drafts(&self, review: &str) -> R<Vec<DraftRow>> {
        let mut s = self.db.prepare(
            "SELECT id, path, side, line, text FROM draft WHERE review = ?1 ORDER BY id",
        )?;
        let rows = s.query_map([review], |r| {
            Ok(DraftRow {
                id: r.get(0)?,
                path: r.get(1)?,
                side: r.get(2)?,
                line: r.get(3)?,
                text: r.get(4)?,
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
        assert_eq!(s.version().unwrap(), 2);
        assert_eq!(s.open_panes().unwrap().len(), 1);
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
                    text: "nit".into(),
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
        s.delete_draft(d).unwrap();
        assert!(s.drafts("r1").unwrap().is_empty());
    }
}
