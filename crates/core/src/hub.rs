//! The hub: tmux, the store, the panes and the socket, in one place.
//!
//! The window calls the hub. The hub tells the window about changes through
//! a [`Sink`]. Every rule about lamps and unseen state lives here.

use crate::config::Config;
use crate::connections;
use crate::hooks;
use crate::lamp::{self, Lamp};
use crate::memory;
use crate::nvim;
use crate::paths::Paths;
use crate::proto::{Ack, PermitReply, Request, StateMsg};
use crate::repos::{self, Repo};
use crate::store::{PaneRow, Store};
use crate::tmux::{self, Tmux};
use crate::{expand, now};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;

/// Where the hub sends news for the window.
pub trait Sink: Send + Sync + 'static {
    fn emit(&self, event: &str, payload: Value);
    /// Bytes from the live pane.
    fn output(&self, pane: &str, data: &[u8]);
}

/// The text of Claude's folder trust dialog. A test pins it (SPEC 6.6).
pub const TRUST_TEXT: &str = "trust this folder";

const TAIL_LINES: usize = 6;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PaneView {
    pub id: String,
    pub kind: String,
    pub repo: String,
    pub tree: String,
    pub branch: String,
    pub title: Option<String>,
    pub connection: Option<String>,
    pub session: Option<String>,
    pub lamp: Lamp,
    pub unseen: bool,
    pub summary: Option<String>,
    pub tail: Vec<String>,
    pub rss: Option<u64>,
    pub permit: Option<PermitView>,
    pub trust: bool,
    pub live: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PermitView {
    pub id: String,
    pub summary: String,
}

pub(crate) struct Pane {
    pub row: PaneRow,
    pub tmux: Option<String>,
    pub pid: Option<u32>,
    pub branch: String,
    pub tail: Vec<String>,
    pub rss: Option<u64>,
    pub permit: Option<PermitView>,
    pub trust: bool,
    pub started: u64,
    /// The tick saw this process exit. It stops a second alert for the same exit.
    pub dead: bool,
}

impl Pane {
    pub fn lamp(&self) -> Lamp {
        Lamp::parse(&self.row.lamp)
    }
}

/// What to open. `place` is `main`, `new` (with `branch`) or `tree` (with `tree`).
#[derive(serde::Deserialize, Debug, Clone)]
pub struct OpenReq {
    pub repo: String,
    pub place: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub tree: Option<String>,
    /// `claude`, `nvim` or `shell`.
    pub run: String,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ConnView {
    pub active: String,
    pub list: Vec<ConnItem>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ConnItem {
    pub name: String,
    pub kind: String,
    pub limit_until: Option<u64>,
}

type Live = Arc<Mutex<Option<(String, String)>>>;

pub struct Hub {
    pub paths: Paths,
    pub(crate) config: RwLock<Config>,
    pub(crate) tmux: Arc<Tmux>,
    pub(crate) store: Mutex<Store>,
    pub(crate) panes: Mutex<Vec<Pane>>,
    /// (our id, tmux id) of the live pane. The tmux reader thread reads it.
    live: Live,
    permits: Mutex<HashMap<String, (String, Sender<PermitReply>)>>,
    pub(crate) sink: Arc<dyn Sink>,
    pub(crate) sb: PathBuf,
    pub(crate) reviews: Mutex<HashMap<String, crate::hub_review::ReviewState>>,
    /// The last size the window asked for.
    size: Mutex<(u16, u16)>,
    me: Weak<Hub>,
}

type Res<T> = Result<T, String>;
type Env = Vec<(String, String)>;

fn e(x: impl std::fmt::Display) -> String {
    x.to_string()
}

impl Hub {
    /// Starts everything: the tmux client, the store, the socket and the ticker.
    pub fn start(paths: Paths, sb: PathBuf, sink: Arc<dyn Sink>) -> Res<Arc<Hub>> {
        paths.create_all().map_err(e)?;
        std::fs::write(paths.tmux_conf(), tmux::CONF).map_err(e)?;
        let (config, config_error) = match Config::load(&paths.config) {
            Ok(c) => (c, None),
            Err(err) => (Config::default(), Some(err)),
        };
        let store = Store::open(&paths.store()).map_err(e)?;

        let live: Live = Arc::default();
        let reader_live = Arc::clone(&live);
        let reader_sink = Arc::clone(&sink);
        let socket = std::env::var("SB_TMUX_SOCKET").unwrap_or_else(|_| "switchboard".into());
        let tmux = Tmux::start(&socket, &paths.tmux_conf(), move |ev| match ev {
            tmux::Event::Output { pane, data } => {
                let live = reader_live.lock().unwrap().clone();
                if let Some((id, t)) = live {
                    if t == pane {
                        reader_sink.output(&id, &data);
                    }
                }
            }
            tmux::Event::Exit(reason) => reader_sink.emit("tmux_exit", json!(reason)),
            _ => {}
        })
        .map_err(e)?;

        let hub = Arc::new_cyclic(|me| Hub {
            paths,
            config: RwLock::new(config),
            tmux,
            store: Mutex::new(store),
            panes: Mutex::default(),
            live,
            permits: Mutex::default(),
            sink,
            sb,
            reviews: Mutex::default(),
            size: Mutex::new((200, 50)),
            me: me.clone(),
        });
        hub.keep_session()?;
        hub.reattach()?;
        hub.load_reviews();
        if let Some(err) = config_error {
            hub.sink
                .emit("error", json!({ "scope": "config", "message": err }));
        }
        hub.listen()?;
        hub.tick_loop();
        Ok(hub)
    }

    pub(crate) fn arc(&self) -> Arc<Hub> {
        self.me.upgrade().expect("the hub is alive")
    }

    pub fn config(&self) -> Config {
        self.config.read().unwrap().clone()
    }

    // ---------- start and restart (SPEC 16) ----------

    /// tmux ends a session when its last pane closes, and the control client
    /// with it. A hidden pane that never exits keeps the session alive.
    fn keep_session(&self) -> Res<()> {
        let tags = self.tmux.list("#{@sb_keep}").map_err(e)?;
        if tags.iter().any(|t| t == "1") {
            return Ok(());
        }
        let tid = self
            .tmux
            .new_pane(
                &crate::paths::home_dir(),
                &["sh", "-c", "exec tail -f /dev/null"],
                &[],
            )
            .map_err(e)?;
        self.tmux
            .set_pane_option(&tid, "@sb_keep", "1")
            .map_err(e)?;
        let _ = self.tmux.set_streaming(&tid, false);
        Ok(())
    }

    fn reattach(&self) -> Res<()> {
        let lines = self
            .tmux
            .list("#{pane_id}\t#{@sb_pane}\t#{pane_dead}\t#{pane_dead_status}\t#{pane_current_path}\t#{pane_pid}\t#{@sb_keep}")
            .map_err(e)?;
        let store = self.store.lock().unwrap();
        let rows = store.open_panes().map_err(e)?;
        let mut panes = vec![];
        let mut found = std::collections::HashSet::new();
        for line in &lines {
            let f: Vec<&str> = line.split('\t').collect();
            let (tid, tag, dead, path, pid) = (
                f[0],
                f.get(1).copied().unwrap_or(""),
                f.get(2) == Some(&"1"),
                f.get(4).copied().unwrap_or("/"),
                f.get(5).and_then(|p| p.parse().ok()),
            );
            let status: i32 = f.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
            if f.get(6) == Some(&"1") {
                continue;
            }
            let _ = self.tmux.set_streaming(tid, false);
            let row = match rows.iter().find(|r| r.id == tag) {
                Some(r) => r.clone(),
                None => {
                    // A pane we did not start, like the first window of the session. Adopt it as a shell.
                    let id = store.next_id("p").map_err(e)?;
                    let _ = self.tmux.set_pane_option(tid, "@sb_pane", &id);
                    let _ = self.tmux.set_pane_option(tid, "remain-on-exit", "on");
                    let row = PaneRow {
                        id,
                        kind: "shell".into(),
                        repo: Path::new(path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "/".into()),
                        tree: path.into(),
                        title: None,
                        connection: None,
                        session: None,
                        lamp: "none".into(),
                        unseen: false,
                        summary: None,
                        updated: now(),
                    };
                    store.save_pane(&row).map_err(e)?;
                    row
                }
            };
            found.insert(row.id.clone());
            if dead && row.kind != "claude" {
                // An nvim or a shell that quit while the app was closed.
                let _ = self.tmux.kill_pane(tid);
                store.close_pane(&row.id, now()).map_err(e)?;
                continue;
            }
            let mut p = self.pane_from(row, Some(tid.to_owned()), pid);
            if dead {
                p.dead = true;
                p.row.lamp = if status == 0 {
                    Lamp::Ended
                } else {
                    Lamp::Error
                }
                .as_str()
                .into();
                if status != 0 {
                    p.row.summary = Some(format!("Exited with {status}"));
                }
            }
            panes.push(p);
        }
        for row in rows.into_iter().filter(|r| !found.contains(&r.id)) {
            if row.kind == "claude" && row.session.is_some() {
                let mut p = self.pane_from(row, None, None);
                p.row.lamp = Lamp::Ended.as_str().into();
                panes.push(p);
            } else {
                store.close_pane(&row.id, now()).map_err(e)?;
            }
        }
        drop(store);
        // A state file newer than the store wins: hooks ran while the app was closed.
        for p in &mut panes {
            if let Ok(text) =
                std::fs::read_to_string(self.paths.state_dir().join(format!("{}.json", p.row.id)))
            {
                if let Ok(m) = serde_json::from_str::<StateMsg>(&text) {
                    if m.time > p.row.updated && p.tmux.is_some() {
                        apply_state(p, &m);
                    }
                }
            }
        }
        // Keep the saved board order. tmux lists panes in window order.
        let order: Vec<String> = self
            .store
            .lock()
            .unwrap()
            .open_panes()
            .map_err(e)?
            .into_iter()
            .map(|r| r.id)
            .collect();
        panes.sort_by_key(|p| {
            order
                .iter()
                .position(|id| *id == p.row.id)
                .unwrap_or(usize::MAX)
        });
        *self.panes.lock().unwrap() = panes;
        Ok(())
    }

    fn pane_from(&self, row: PaneRow, tmux: Option<String>, pid: Option<u32>) -> Pane {
        let branch =
            repos::git(Path::new(&row.tree), &["branch", "--show-current"]).unwrap_or_default();
        Pane {
            row,
            tmux,
            pid,
            branch,
            tail: vec![],
            rss: None,
            permit: None,
            trust: false,
            started: now(),
            dead: false,
        }
    }

    // ---------- views ----------

    fn view(&self, p: &Pane) -> PaneView {
        let live = self
            .live
            .lock()
            .unwrap()
            .as_ref()
            .map(|(id, _)| id == &p.row.id)
            .unwrap_or(false);
        PaneView {
            id: p.row.id.clone(),
            kind: p.row.kind.clone(),
            repo: p.row.repo.clone(),
            tree: p.row.tree.clone(),
            branch: p.branch.clone(),
            title: p.row.title.clone(),
            connection: p.row.connection.clone(),
            session: p.row.session.clone(),
            lamp: p.lamp(),
            unseen: p.row.unseen,
            summary: p.row.summary.clone(),
            tail: p.tail.clone(),
            rss: p.rss,
            permit: p.permit.clone(),
            trust: p.trust,
            live,
        }
    }

    pub fn views(&self) -> Vec<PaneView> {
        self.panes
            .lock()
            .unwrap()
            .iter()
            .map(|p| self.view(p))
            .collect()
    }

    fn emit_panes(&self) {
        self.sink.emit("panes", json!(self.views()));
    }

    fn emit_pane(&self, p: &Pane, alert: bool) {
        let mut v = json!(self.view(p));
        v["alert"] = json!(alert);
        self.sink.emit("pane", v);
    }

    /// Runs `f` on one pane, saves it, and tells the window.
    fn with_pane<T>(&self, id: &str, alert: bool, f: impl FnOnce(&mut Pane) -> T) -> Res<T> {
        let mut panes = self.panes.lock().unwrap();
        let p = panes
            .iter_mut()
            .find(|p| p.row.id == id)
            .ok_or(format!("no pane {id}"))?;
        let out = f(p);
        p.row.updated = now();
        self.store.lock().unwrap().save_pane(&p.row).map_err(e)?;
        self.emit_pane(p, alert);
        Ok(out)
    }

    /// Like `with_pane`, but `f` decides if the change alerts you.
    fn update_pane(&self, id: &str, f: impl FnOnce(&mut Pane) -> bool) -> Res<bool> {
        let mut panes = self.panes.lock().unwrap();
        let p = panes
            .iter_mut()
            .find(|p| p.row.id == id)
            .ok_or(format!("no pane {id}"))?;
        let alert = f(p);
        p.row.updated = now();
        self.store.lock().unwrap().save_pane(&p.row).map_err(e)?;
        self.emit_pane(p, alert);
        Ok(alert)
    }

    fn tmux_id(&self, id: &str) -> Res<String> {
        self.panes
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.row.id == id)
            .and_then(|p| p.tmux.clone())
            .ok_or(format!("pane {id} has no process"))
    }

    // ---------- open and close ----------

    pub fn open(&self, req: OpenReq) -> Res<PaneView> {
        let repo = PathBuf::from(&req.repo);
        let tree = match req.place.as_str() {
            "main" => repo.clone(),
            "new" => repos::add_worktree(
                &repo,
                req.branch
                    .as_deref()
                    .filter(|b| !b.is_empty())
                    .ok_or("a new worktree needs a branch")?,
            )?,
            "tree" => PathBuf::from(req.tree.as_deref().ok_or("no worktree given")?),
            other => return Err(format!("unknown place {other}")),
        };
        let name = repo
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match req.run.as_str() {
            "nvim" => {
                let id = self.nvim_open(&tree, None)?;
                self.views()
                    .into_iter()
                    .find(|v| v.id == id)
                    .ok_or("nvim pane is gone".into())
            }
            "claude" | "shell" => self.spawn(&req.run, &name, &tree, req.title, None),
            other => Err(format!("unknown run {other}")),
        }
    }

    /// Starts a pane. `resume` is a Claude session id.
    fn spawn(
        &self,
        kind: &str,
        repo: &str,
        tree: &Path,
        title: Option<String>,
        resume: Option<&str>,
    ) -> Res<PaneView> {
        let id = self.store.lock().unwrap().next_id("p").map_err(e)?;
        let connection = (kind == "claude").then(|| self.active_connection());
        let (argv, env) = self.command(kind, &id, tree, connection.as_deref(), resume)?;
        let argv_ref: Vec<&str> = argv.iter().map(String::as_str).collect();
        let env_ref: Vec<(&str, &str)> =
            env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let tid = self.tmux.new_pane(tree, &argv_ref, &env_ref).map_err(e)?;
        let _ = self.tmux.set_pane_option(&tid, "@sb_pane", &id);
        let _ = self.tmux.set_pane_option(&tid, "remain-on-exit", "on");
        let _ = self.tmux.set_streaming(&tid, false);
        let pid = self
            .tmux
            .display(&tid, "#{pane_pid}")
            .ok()
            .and_then(|p| p.parse().ok());
        let row = PaneRow {
            id,
            kind: kind.into(),
            repo: repo.into(),
            tree: tree.to_string_lossy().into_owned(),
            title,
            connection,
            session: resume.map(str::to_owned),
            lamp: if kind == "claude" {
                Lamp::Idle
            } else {
                Lamp::None
            }
            .as_str()
            .into(),
            unseen: false,
            summary: None,
            updated: now(),
        };
        self.store.lock().unwrap().save_pane(&row).map_err(e)?;
        let pane = self.pane_from(row, Some(tid), pid);
        let view = self.view(&pane);
        self.panes.lock().unwrap().push(pane);
        self.emit_panes();
        Ok(view)
    }

    /// The argv and env for a pane.
    fn command(
        &self,
        kind: &str,
        id: &str,
        tree: &Path,
        connection: Option<&str>,
        resume: Option<&str>,
    ) -> Res<(Vec<String>, Env)> {
        let mut env = vec![
            ("SB_PANE".to_owned(), id.to_owned()),
            (
                "SB_SOCK".to_owned(),
                self.paths.sock().to_string_lossy().into_owned(),
            ),
        ];
        if kind == "shell" {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".into());
            return Ok((vec![shell], env));
        }
        let settings = self.paths.settings_dir().join(format!("{id}.json"));
        let rust = tree.join("Cargo.toml").exists();
        std::fs::write(&settings, hooks::settings(&self.sb, rust).to_string()).map_err(e)?;
        env.extend(connections::env(
            &self.config(),
            connection.unwrap_or(connections::DEFAULT),
        )?);
        let mut argv = vec![
            "claude".to_owned(),
            "--settings".to_owned(),
            settings.to_string_lossy().into_owned(),
        ];
        if let Some(s) = resume {
            argv.extend(["--resume".into(), s.into()]);
        }
        Ok((argv, env))
    }

    pub fn close(&self, id: &str) -> Res<()> {
        let tid = {
            let mut panes = self.panes.lock().unwrap();
            let i = panes
                .iter()
                .position(|p| p.row.id == id)
                .ok_or(format!("no pane {id}"))?;
            panes.remove(i).tmux
        };
        if let Some(t) = tid {
            let _ = self.tmux.kill_pane(&t);
        }
        {
            let mut live = self.live.lock().unwrap();
            if live.as_ref().map(|(l, _)| l == id).unwrap_or(false) {
                *live = None;
            }
        }
        self.store
            .lock()
            .unwrap()
            .close_pane(id, now())
            .map_err(e)?;
        self.answer_permits(id, None);
        let _ = std::fs::remove_file(self.paths.settings_dir().join(format!("{id}.json")));
        self.emit_panes();
        Ok(())
    }

    /// Tests only: changes the kind of a pane, so lamps can be tested with no `claude`.
    #[doc(hidden)]
    pub fn debug_set_kind(&self, id: &str, kind: &str) -> Res<()> {
        self.with_pane(id, false, |p| {
            p.row.kind = kind.into();
            p.row.lamp = if kind == "claude" {
                Lamp::Idle
            } else {
                Lamp::None
            }
            .as_str()
            .into();
        })
    }

    /// Puts the tiles in this order. Ids not in the list keep their place at the end.
    pub fn reorder(&self, ids: &[String]) -> Res<()> {
        let order = {
            let mut panes = self.panes.lock().unwrap();
            panes.sort_by_key(|p| {
                ids.iter()
                    .position(|id| *id == p.row.id)
                    .unwrap_or(usize::MAX)
            });
            panes.iter().map(|p| p.row.id.clone()).collect::<Vec<_>>()
        };
        self.store.lock().unwrap().set_order(&order).map_err(e)?;
        self.emit_panes();
        Ok(())
    }

    pub fn rename(&self, id: &str, title: Option<String>) -> Res<()> {
        self.with_pane(id, false, |p| {
            p.row.title = title.filter(|t| !t.trim().is_empty())
        })
    }

    // ---------- the live pane ----------

    /// Makes a pane live and returns its screen: the scrollback, then the cursor.
    pub fn expand(&self, id: &str) -> Res<Vec<u8>> {
        let tid = self.tmux_id(id)?;
        let old = self
            .live
            .lock()
            .unwrap()
            .replace((id.to_owned(), tid.clone()));
        if let Some((_, t)) = old.filter(|(_, t)| *t != tid) {
            let _ = self.tmux.set_streaming(&t, false);
        }
        let lines = self.tmux.capture(&tid, "-").map_err(e)?;
        let (x, y) = self.tmux.cursor(&tid).map_err(e)?;
        self.tmux.set_streaming(&tid, true).map_err(e)?;
        let mut screen = b"\x1b[H\x1b[2J\x1b[3J".to_vec();
        screen.extend(lines.join("\r\n").into_bytes());
        screen.extend(format!("\x1b[{};{}H", y + 1, x + 1).into_bytes());
        self.seen(id)?;
        // Output in the gap between the capture and `continue` is lost. A size
        // change makes the program in the pane draw its whole screen again.
        let (cols, rows) = *self.size.lock().unwrap();
        let tmux = Arc::clone(&self.tmux);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            let _ = tmux.set_size(cols, rows.saturating_sub(1).max(2));
            std::thread::sleep(Duration::from_millis(80));
            let _ = tmux.set_size(cols, rows);
        });
        Ok(screen)
    }

    pub fn collapse(&self) {
        // Take the value first: an `if let` on the guard keeps the lock for the whole block.
        let taken = self.live.lock().unwrap().take();
        if let Some((id, t)) = taken {
            let _ = self.tmux.set_streaming(&t, false);
            if let Ok(panes) = self.panes.lock() {
                if let Some(p) = panes.iter().find(|p| p.row.id == id) {
                    self.emit_pane(p, false);
                }
            }
        }
    }

    pub fn input(&self, id: &str, bytes: &[u8]) -> Res<()> {
        let tid = self.tmux_id(id)?;
        self.tmux.send_bytes(&tid, bytes).map_err(e)?;
        // A key in the pane answers its prompt there, so the tile buttons go.
        if self.permits.lock().unwrap().values().any(|(p, _)| p == id) {
            self.answer_permits(id, None);
        }
        let unseen = self
            .panes
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.row.id == id && p.row.unseen);
        if unseen {
            self.seen(id)?;
        }
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Res<()> {
        *self.size.lock().unwrap() = (cols, rows);
        self.tmux.set_size(cols, rows).map_err(e)
    }

    pub fn seen(&self, id: &str) -> Res<()> {
        self.with_pane(id, false, |p| p.row.unseen = false)
    }

    // ---------- hook events (SPEC 6, 13.3) ----------

    pub fn on_state(&self, m: &StateMsg) {
        let mut checktime = None;
        let mut limit_conn = None;
        let res = self.update_pane(&m.pane, |p| {
            apply_state(p, m);
            if m.event == "edited" {
                checktime = Some(p.row.tree.clone());
            }
            if m.event == "limit" {
                limit_conn = Some(p.row.connection.clone());
            }
            // Only an event that lights a lamp alerts. `edited` and `session` never do.
            p.lamp().wants_you() && matches!(m.event.as_str(), "needs" | "turn" | "limit" | "error")
        });
        if res.is_err() {
            return;
        }
        if matches!(m.event.as_str(), "working" | "turn" | "limit" | "error") {
            // The prompt was answered in the pane, or the turn moved on.
            self.answer_permits(&m.pane, None);
        }
        if let Some(tree) = checktime {
            self.nvim_checktime(Path::new(&tree));
        }
        if let Some(c) = limit_conn {
            let c = c.unwrap_or_else(|| self.active_connection());
            let _ = self.store.lock().unwrap().set_limit(&c, now() + 3600);
            self.emit_connections();
        } else if m.event == "working" {
            let conn = self
                .panes
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.row.id == m.pane)
                .and_then(|p| p.row.connection.clone());
            if let Some(c) = conn {
                let had = self
                    .store
                    .lock()
                    .unwrap()
                    .limit_until(&c)
                    .ok()
                    .flatten()
                    .is_some();
                if had {
                    let _ = self.store.lock().unwrap().clear_limit(&c);
                    self.emit_connections();
                }
            }
        }
    }

    fn on_permit(&self, pane: &str, tool: &str, input: &Value, reply: Sender<PermitReply>) -> bool {
        let id = format!("{pane}-{}", now_nanos());
        let summary = hooks::permit_summary(tool, input);
        let ok = self
            .update_pane(pane, |p| {
                p.permit = Some(PermitView {
                    id: id.clone(),
                    summary: summary.clone(),
                });
                let (l, u) = lamp::apply(p.lamp(), p.row.unseen, "needs");
                p.row.lamp = l.as_str().into();
                p.row.unseen = u;
                p.row.summary = Some(summary.clone());
                true
            })
            .is_ok();
        if ok {
            self.permits
                .lock()
                .unwrap()
                .insert(id, (pane.to_owned(), reply));
        }
        ok
    }

    /// Answers one permission request from the tile.
    pub fn permit_answer(&self, permit: &str, allow: bool) -> Res<()> {
        let (pane, tx) = self
            .permits
            .lock()
            .unwrap()
            .remove(permit)
            .ok_or("the request is gone: it was answered in the pane")?;
        let _ = tx.send(PermitReply {
            behavior: Some(if allow { "allow" } else { "deny" }.into()),
        });
        self.with_pane(&pane, false, |p| {
            p.permit = None;
            p.row.unseen = false;
            if allow {
                p.row.lamp = Lamp::Working.as_str().into();
            }
        })
    }

    fn answer_permits(&self, pane: &str, behavior: Option<&str>) {
        let mut permits = self.permits.lock().unwrap();
        let ids: Vec<String> = permits
            .iter()
            .filter(|(_, (p, _))| p == pane)
            .map(|(k, _)| k.clone())
            .collect();
        for id in ids {
            if let Some((_, tx)) = permits.remove(&id) {
                let _ = tx.send(PermitReply {
                    behavior: behavior.map(str::to_owned),
                });
            }
        }
        drop(permits);
        let mut panes = self.panes.lock().unwrap();
        if let Some(p) = panes.iter_mut().find(|p| p.row.id == pane) {
            if p.permit.take().is_some() {
                self.emit_pane(p, false);
            }
        }
    }

    /// Answers the folder trust dialog (SPEC 6.6).
    pub fn trust(&self, id: &str, yes: bool) -> Res<()> {
        if !yes {
            return self.close(id);
        }
        let tid = self.tmux_id(id)?;
        self.tmux.send_bytes(&tid, b"\x1b[B\r").map_err(e)?;
        self.with_pane(id, false, |p| {
            p.trust = false;
            p.row.unseen = false;
            p.row.lamp = Lamp::Idle.as_str().into();
            p.row.summary = None;
        })
    }

    /// Starts an ended Claude pane again with its session.
    pub fn resume(&self, id: &str) -> Res<()> {
        let (session, tree, tid, connection) = {
            let panes = self.panes.lock().unwrap();
            let p = panes
                .iter()
                .find(|p| p.row.id == id)
                .ok_or(format!("no pane {id}"))?;
            (
                p.row.session.clone().ok_or("no session to resume")?,
                p.row.tree.clone(),
                p.tmux.clone(),
                p.row.connection.clone(),
            )
        };
        let connection = connection.unwrap_or_else(|| self.active_connection());
        self.respawn_claude(id, tid, Path::new(&tree), &connection, &session)
    }

    fn respawn_claude(
        &self,
        id: &str,
        tid: Option<String>,
        tree: &Path,
        connection: &str,
        session: &str,
    ) -> Res<()> {
        let (argv, env) = self.command("claude", id, tree, Some(connection), Some(session))?;
        let argv_ref: Vec<&str> = argv.iter().map(String::as_str).collect();
        let env_ref: Vec<(&str, &str)> =
            env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let tid = match tid {
            Some(t) => {
                self.tmux
                    .respawn(&t, tree, &argv_ref, &env_ref)
                    .map_err(e)?;
                t
            }
            None => {
                let t = self.tmux.new_pane(tree, &argv_ref, &env_ref).map_err(e)?;
                let _ = self.tmux.set_pane_option(&t, "@sb_pane", id);
                let _ = self.tmux.set_pane_option(&t, "remain-on-exit", "on");
                t
            }
        };
        let live = self
            .live
            .lock()
            .unwrap()
            .as_ref()
            .map(|(l, _)| l == id)
            .unwrap_or(false);
        let _ = self.tmux.set_streaming(&tid, live);
        let pid = self
            .tmux
            .display(&tid, "#{pane_pid}")
            .ok()
            .and_then(|p| p.parse().ok());
        if live {
            *self.live.lock().unwrap() = Some((id.to_owned(), tid.clone()));
        }
        self.with_pane(id, false, |p| {
            p.tmux = Some(tid);
            p.pid = pid;
            p.row.connection = Some(connection.to_owned());
            p.row.lamp = Lamp::Idle.as_str().into();
            p.row.unseen = false;
            p.row.summary = Some(format!("Resumed on {connection}"));
            p.started = now();
            p.dead = false;
        })
    }

    // ---------- connections (SPEC 8) ----------

    pub fn active_connection(&self) -> String {
        let names = connections::names(&self.config());
        let stored = self
            .store
            .lock()
            .unwrap()
            .get("active_connection")
            .ok()
            .flatten();
        stored
            .filter(|s| names.contains(s))
            .unwrap_or_else(|| names[0].clone())
    }

    pub fn connection_view(&self) -> ConnView {
        let config = self.config();
        let store = self.store.lock().unwrap();
        let list = connections::names(&config)
            .into_iter()
            .map(|name| ConnItem {
                kind: config
                    .connections
                    .get(&name)
                    .map(|c| c.kind.clone())
                    .unwrap_or_else(|| "claude".into()),
                limit_until: store
                    .limit_until(&name)
                    .ok()
                    .flatten()
                    .filter(|t| *t > now()),
                name,
            })
            .collect();
        drop(store);
        ConnView {
            active: self.active_connection(),
            list,
        }
    }

    fn emit_connections(&self) {
        self.sink.emit("connections", json!(self.connection_view()));
    }

    pub fn connection_set(&self, name: &str) -> Res<()> {
        if !connections::names(&self.config()).iter().any(|n| n == name) {
            return Err(format!("no connection named {name}"));
        }
        self.store
            .lock()
            .unwrap()
            .set("active_connection", name)
            .map_err(e)?;
        self.emit_connections();
        Ok(())
    }

    /// Moves a Limit pane to the next free connection and resumes its session there.
    pub fn connection_resume(&self, id: &str) -> Res<String> {
        let (session, tree, tid, current) = {
            let panes = self.panes.lock().unwrap();
            let p = panes
                .iter()
                .find(|p| p.row.id == id)
                .ok_or(format!("no pane {id}"))?;
            (
                p.row
                    .session
                    .clone()
                    .ok_or("this pane has no session yet")?,
                p.row.tree.clone(),
                p.tmux.clone(),
                p.row
                    .connection
                    .clone()
                    .unwrap_or_else(|| self.active_connection()),
            )
        };
        let names = connections::names(&self.config());
        let next = {
            let store = self.store.lock().unwrap();
            connections::next_free(&names, &current, |n| {
                store
                    .limit_until(n)
                    .ok()
                    .flatten()
                    .map(|t| t > now())
                    .unwrap_or(false)
            })
        }
        .ok_or("every other connection is at its limit")?;
        self.connection_set(&next)?;
        if let Some(t) = &tid {
            self.stop_claude(t);
        }
        self.respawn_claude(id, tid, Path::new(&tree), &next, &session)?;
        Ok(next)
    }

    /// Sends Ctrl+C twice and waits up to 5 s for the pane to die.
    fn stop_claude(&self, tid: &str) {
        let _ = self.tmux.send_bytes(tid, b"\x03");
        std::thread::sleep(Duration::from_millis(150));
        let _ = self.tmux.send_bytes(tid, b"\x03");
        for _ in 0..50 {
            if self
                .tmux
                .display(tid, "#{pane_dead}")
                .map(|d| d == "1")
                .unwrap_or(true)
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // ---------- nvim (SPEC 10) ----------

    /// Opens nvim in a worktree, at a file and line when given. Returns the pane id.
    pub fn nvim_open(&self, tree: &Path, file: Option<(&str, u32)>) -> Res<String> {
        if !nvim::installed() {
            return Err("nvim is not on PATH".into());
        }
        let sock = nvim::socket(&self.paths.nvim_dir(), tree);
        let existing = self
            .panes
            .lock()
            .unwrap()
            .iter()
            .find(|p| {
                p.row.kind == "nvim"
                    && Path::new(&p.row.tree) == tree
                    && p.tmux.is_some()
                    && p.lamp() != Lamp::Ended
            })
            .map(|p| p.row.id.clone());
        if let Some(id) = existing {
            if nvim::alive(&sock) {
                if let Some((f, l)) = file {
                    nvim::expr(&sock, &nvim::edit_expr(f, l))?;
                }
                return Ok(id);
            }
            self.close(&id)?;
        }
        let _ = std::fs::remove_file(&sock);
        let argv = nvim::argv(&sock, file);
        let argv_ref: Vec<&str> = argv.iter().map(String::as_str).collect();
        let id = self.store.lock().unwrap().next_id("p").map_err(e)?;
        let tid = self
            .tmux
            .new_pane(tree, &argv_ref, &[("SB_PANE", &id)])
            .map_err(e)?;
        let _ = self.tmux.set_pane_option(&tid, "@sb_pane", &id);
        let _ = self.tmux.set_pane_option(&tid, "remain-on-exit", "on");
        let _ = self.tmux.set_streaming(&tid, false);
        let repo = repos::main_checkout(tree)
            .and_then(|m| m.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "nvim".into());
        let row = PaneRow {
            id: id.clone(),
            kind: "nvim".into(),
            repo,
            tree: tree.to_string_lossy().into_owned(),
            title: None,
            connection: None,
            session: None,
            lamp: Lamp::None.as_str().into(),
            unseen: false,
            summary: file.map(|(f, l)| format!("{f}:{l}")),
            updated: now(),
        };
        self.store.lock().unwrap().save_pane(&row).map_err(e)?;
        let p = self.pane_from(row, Some(tid), None);
        self.panes.lock().unwrap().push(p);
        self.emit_panes();
        Ok(id)
    }

    fn nvim_checktime(&self, tree: &Path) {
        let sock = nvim::socket(&self.paths.nvim_dir(), tree);
        if sock.exists() {
            let _ = nvim::expr(&sock, nvim::CHECKTIME);
        }
    }

    // ---------- repos and layouts ----------

    pub fn repos(&self) -> Vec<Repo> {
        let roots: Vec<PathBuf> = self.config().scan.iter().map(|s| expand(s)).collect();
        repos::discover(&self.paths.claude.join("projects"), &roots)
    }

    pub fn layout_save(&self, name: &str) -> Res<()> {
        let list: Vec<Value> = self
            .panes
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.row.kind != "nvim")
            .map(|p| json!({ "repo": p.row.repo, "tree": p.row.tree, "kind": p.row.kind, "title": p.row.title }))
            .collect();
        self.store
            .lock()
            .unwrap()
            .save_layout(name, &Value::Array(list).to_string())
            .map_err(e)
    }

    pub fn layouts(&self) -> Vec<String> {
        self.store
            .lock()
            .unwrap()
            .layouts()
            .unwrap_or_default()
            .into_iter()
            .map(|(n, _)| n)
            .collect()
    }

    pub fn layout_open(&self, name: &str) -> Res<()> {
        let text = self
            .store
            .lock()
            .unwrap()
            .layouts()
            .map_err(e)?
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t)
            .ok_or(format!("no layout {name}"))?;
        let list: Vec<Value> = serde_json::from_str(&text).map_err(e)?;
        for item in list {
            let (repo, tree, kind) = (
                item["repo"].as_str().unwrap_or(""),
                item["tree"].as_str().unwrap_or(""),
                item["kind"].as_str().unwrap_or("claude"),
            );
            let running = self
                .panes
                .lock()
                .unwrap()
                .iter()
                .any(|p| p.row.tree == tree && p.row.kind == kind && p.lamp() != Lamp::Ended);
            if !running && Path::new(tree).is_dir() {
                self.spawn(
                    kind,
                    repo,
                    Path::new(tree),
                    item["title"].as_str().map(str::to_owned),
                    None,
                )?;
            }
        }
        Ok(())
    }

    // ---------- the socket (SPEC 13.3) ----------

    fn listen(&self) -> Res<()> {
        let sock = self.paths.sock();
        if sock.exists() {
            if UnixStream::connect(&sock).is_ok() {
                return Err(format!("another Switchboard listens on {}", sock.display()));
            }
            let _ = std::fs::remove_file(&sock);
        }
        let listener =
            UnixListener::bind(&sock).map_err(|err| format!("{}: {err}", sock.display()))?;
        let me = self.me.clone();
        std::thread::Builder::new()
            .name("sb-socket".into())
            .spawn(move || {
                for conn in listener.incoming().flatten() {
                    let Some(hub) = me.upgrade() else { break };
                    std::thread::spawn(move || hub.serve(conn));
                }
            })
            .map_err(e)?;
        Ok(())
    }

    fn serve(&self, conn: UnixStream) {
        let mut line = String::new();
        if BufReader::new(&conn).read_line(&mut line).is_err() {
            return;
        }
        let Ok(req) = serde_json::from_str::<Request>(&line) else {
            return;
        };
        let reply = match req {
            Request::State(m) => {
                self.on_state(&m);
                return;
            }
            Request::Permit { pane, tool, input } => {
                let (tx, rx) = mpsc::channel();
                if !self.on_permit(&pane, &tool, &input, tx) {
                    json!(PermitReply { behavior: None })
                } else {
                    json!(rx.recv().unwrap_or(PermitReply { behavior: None }))
                }
            }
            Request::Open {
                path,
                worktree,
                run,
            } => {
                let req = OpenReq {
                    repo: repos::main_checkout(Path::new(&path))
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or(path.clone()),
                    place: if worktree.is_some() {
                        "new".into()
                    } else {
                        "tree".into()
                    },
                    branch: worktree,
                    tree: Some(path),
                    run: run.unwrap_or_else(|| "claude".into()),
                    title: None,
                };
                match self.open(req) {
                    Ok(v) => {
                        self.sink.emit("focus", json!(v.id));
                        json!(Ack::ok())
                    }
                    Err(err) => json!(Ack::err(err)),
                }
            }
            Request::GuideSet { review, guide } => json!(match self.guide_set(&review, guide) {
                Ok(()) => Ack::ok(),
                Err(err) => Ack::err(err),
            }),
            Request::GuideUpdate {
                review,
                step,
                fields,
            } => json!(match self.guide_update(&review, &step, fields) {
                Ok(()) => Ack::ok(),
                Err(err) => Ack::err(err),
            }),
        };
        let mut w = &conn;
        let _ = writeln!(w, "{reply}");
    }

    // ---------- the ticker ----------

    fn tick_loop(&self) {
        let me = self.me.clone();
        std::thread::Builder::new()
            .name("sb-tick".into())
            .spawn(move || {
                let mut n: u64 = 0;
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    let Some(hub) = me.upgrade() else { break };
                    hub.tick(n);
                    n += 1;
                }
            })
            .expect("spawn the ticker");
    }

    fn tick(&self, n: u64) {
        let Ok(lines) = self
            .tmux
            .list("#{pane_id}\t#{pane_dead}\t#{pane_dead_status}\t#{pane_pid}")
        else {
            return;
        };
        let state: HashMap<String, (bool, i32, Option<u32>)> = lines
            .iter()
            .filter_map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                Some((
                    f.first()?.to_string(),
                    (
                        f.get(1) == Some(&"1"),
                        f.get(2).and_then(|s| s.parse().ok()).unwrap_or(0),
                        f.get(3).and_then(|s| s.parse().ok()),
                    ),
                ))
            })
            .collect();
        let idle_end = self.config().idle_end_minutes * 60;
        let ids: Vec<(String, Option<String>)> = self
            .panes
            .lock()
            .unwrap()
            .iter()
            .map(|p| (p.row.id.clone(), p.tmux.clone()))
            .collect();
        let mut gone = vec![];
        for (id, tid) in ids {
            let Some(tid) = tid else { continue };
            let Some(&(dead, status, pid)) = state.get(&tid) else {
                gone.push(id);
                continue;
            };
            let tail = self
                .tmux
                .capture(&tid, &format!("-{}", TAIL_LINES * 3))
                .unwrap_or_default();
            let tail: Vec<String> = trim_tail(tail);
            let rss = if n.is_multiple_of(5) {
                pid.and_then(memory::rust_analyzer_rss)
            } else {
                None
            };
            let mut panes = self.panes.lock().unwrap();
            let Some(p) = panes.iter_mut().find(|p| p.row.id == id) else {
                continue;
            };
            let mut changed = false;
            let mut alert = false;
            if p.tail != tail {
                p.tail = tail;
                changed = true;
            }
            if n.is_multiple_of(5) && p.rss != rss {
                p.rss = rss;
                changed = true;
            }
            p.pid = pid.or(p.pid);
            if dead && !p.dead {
                p.dead = true;
                if p.row.kind == "claude" {
                    p.row.lamp = if status == 0 {
                        Lamp::Ended
                    } else {
                        Lamp::Error
                    }
                    .as_str()
                    .into();
                    if status != 0 {
                        p.row.summary = Some(format!("Exited with {status}"));
                        p.row.unseen = true;
                        alert = true;
                    } else {
                        p.row.lamp = Lamp::Ended.as_str().into();
                    }
                    changed = true;
                } else {
                    // nvim or a shell quit: the tile goes away.
                    gone.push(id.clone());
                    continue;
                }
            }
            // The trust dialog was answered in the pane: no hook says so.
            if p.trust && !p.tail.join("\n").to_lowercase().contains(TRUST_TEXT) {
                p.trust = false;
                p.row.lamp = Lamp::Idle.as_str().into();
                p.row.unseen = false;
                p.row.summary = None;
                changed = true;
            }
            // Trust dialog: only a new Claude pane, before its first hook.
            if p.row.kind == "claude"
                && p.lamp() == Lamp::Idle
                && !p.trust
                && now() - p.started < 60
            {
                let text = p.tail.join("\n").to_lowercase();
                if text.contains(TRUST_TEXT) {
                    p.trust = true;
                    p.row.lamp = Lamp::Needs.as_str().into();
                    p.row.unseen = true;
                    p.row.summary = Some("Trust this folder?".into());
                    changed = true;
                    alert = true;
                }
            }
            if idle_end > 0
                && p.lamp() == Lamp::Turn
                && p.row.unseen
                && now().saturating_sub(p.row.updated) > idle_end
                && p.row.session.is_some()
            {
                let t = tid.clone();
                drop(panes);
                self.stop_claude(&t);
                continue;
            }
            if changed {
                let _ = self.store.lock().unwrap().save_pane(&p.row);
                self.emit_pane(p, alert);
            }
        }
        for id in gone {
            let kind = self
                .panes
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.row.id == id)
                .map(|p| p.row.kind.clone());
            match kind.as_deref() {
                Some("claude") => {
                    let _ = self.with_pane(&id, false, |p| {
                        p.tmux = None;
                        p.row.lamp = Lamp::Ended.as_str().into();
                    });
                }
                Some(_) => {
                    let _ = self.close(&id);
                }
                None => {}
            }
        }
        self.review_tick(n);
    }
}

/// Applies a hook event to a pane.
fn apply_state(p: &mut Pane, m: &StateMsg) {
    let (l, u) = lamp::apply(p.lamp(), p.row.unseen, &m.event);
    p.row.lamp = l.as_str().into();
    p.row.unseen = u;
    if let Some(s) = &m.session {
        p.row.session = Some(s.clone());
    }
    if m.summary.is_some() && !(m.event == "needs" && p.permit.is_some()) {
        p.row.summary = m.summary.clone();
    } else if m.event == "working" && p.permit.is_none() {
        p.row.summary = None;
    }
    if m.event != "edited" && m.event != "session" {
        p.trust = false;
    }
}

/// Drops trailing blank lines and keeps the last few.
fn trim_tail(mut lines: Vec<String>) -> Vec<String> {
    while lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        lines.pop();
    }
    let lines: Vec<String> = lines.into_iter().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    lines[start..].to_vec()
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_drops_blanks_and_keeps_the_end() {
        let lines: Vec<String> = ["a", "", "b", "c", "d", "e", "f", "g", "", ""]
            .map(String::from)
            .to_vec();
        assert_eq!(
            trim_tail(lines),
            ["b", "c", "d", "e", "f", "g"].map(String::from).to_vec()
        );
    }
}
