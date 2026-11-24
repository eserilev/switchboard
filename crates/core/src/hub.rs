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
use crate::screens::{Screens, Terms, Win, LOCAL};
use crate::store::{PaneRow, Store};
use crate::tmux::{self, Tmux};
use crate::{expand, now};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;

/// Where the hub sends news for the window.
pub trait Sink: Send + Sync + 'static {
    fn emit(&self, event: &str, payload: Value);
    /// Bytes from a pane on screen.
    fn output(&self, pane: &str, data: &[u8]);
    /// A pane goes on screen with a full screen: bytes that wait for it are old.
    fn reset(&self, _pane: &str) {}
    /// Bytes from a pane on the screen of one window (SPEC 41.8). A sink with one
    /// window takes [`Sink::output`].
    fn output_to(&self, _win: Win, pane: &str, data: &[u8]) {
        self.output(pane, data)
    }
    /// [`Sink::reset`] for one window.
    fn reset_for(&self, _win: Win, pane: &str) {
        self.reset(pane)
    }
}

/// The text of Claude's folder trust dialog. A test pins it (SPEC 6.6).
pub const TRUST_TEXT: &str = "trust this folder";
/// The title of the agent of the Work environment tab (SPEC 30). The window finds
/// that agent by it, so a rename never gives it to another pane or takes it away.
pub const WORK_TITLE: &str = "Work environment";

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
    /// A shell runs a command, not its prompt (SPEC 28).
    pub busy: bool,
    /// nvim has a buffer that is not saved (SPEC 28).
    pub modified: bool,
    /// Bash runs in Claude Code's sandbox (SPEC 31).
    pub sandboxed: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PermitView {
    pub id: String,
    pub summary: String,
}

/// How a claude pane starts.
#[derive(Default)]
pub(crate) struct Start {
    /// A Claude session id to resume.
    pub resume: Option<String>,
    /// Resume as a new session, so the old one stays as it is.
    pub fork: bool,
    /// More folders that the agent can read.
    pub dirs: Vec<String>,
    /// The first message.
    pub first: Option<String>,
    /// The connection. With none, the active one.
    pub connection: Option<String>,
    /// The rust-analyzer plugin on or off. With none, on when the tree has a `Cargo.toml`.
    pub rust: Option<bool>,
    /// Tool rules that run with no prompt, for the pane settings.
    pub allow: Vec<String>,
    /// `--setting-sources`. A review pane runs in the PR's own files, so it takes
    /// only your user settings: the PR's `.claude/settings.json` could hold hooks.
    pub setting_sources: Option<&'static str>,
    /// A review Ask pane: sandbox mode `review` covers it (SPEC 31).
    pub ask: bool,
    /// `--mcp-config`, with `--strict-mcp-config`: the Ask pane's `sb mcp --ask`.
    pub mcp: Option<String>,
    /// `--append-system-prompt`. One line: the command goes through tmux.
    pub system: Option<String>,
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
    /// The value of `Hub.spawns` when this process started. A tick whose tmux list
    /// is older than the start leaves the pane alone: the list does not show it yet.
    pub spawn: u64,
    pub busy: bool,
    pub modified: bool,
    pub sandboxed: bool,
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
    /// Start agent from a task (SPEC 27.6): the task to link the pane to.
    #[serde(default)]
    pub task: Option<String>,
    /// The first message, pasted with no Enter when the agent is ready.
    #[serde(default)]
    pub first: Option<String>,
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

/// The screen of each window: (our id, tmux id) of each pane on it. The first is
/// the live pane; the others are its parts, nvim and the shell (SPEC 28, 41.8).
type Live = Arc<Mutex<Screens>>;

pub struct Hub {
    pub paths: Paths,
    pub(crate) config: RwLock<Config>,
    pub(crate) tmux: Arc<Tmux>,
    pub(crate) store: Mutex<Store>,
    pub(crate) panes: Mutex<Vec<Pane>>,
    /// The panes on the screen of each window. The tmux reader thread reads it.
    live: Live,
    /// A terminal emulator for each streamed pane, for fresh screens (SPEC 41.8).
    /// Lock order: `terms`, then the sink's queues. The reader thread feeds it and
    /// queues the same bytes under it, so a snapshot and the bytes after it agree.
    terms: Arc<Mutex<Terms>>,
    /// Held while the screen of a window changes: a change and the streams it
    /// starts or stops are one step. Else a collapse in one window can stop the
    /// stream of a pane that another window just put on its screen
    /// (models/windows.qnt). The tmux reader thread never takes it.
    /// Lock order: `screen`, then `live`, `terms` and `panes`.
    screen: Mutex<()>,
    /// Waiting `sb permit` requests: permit id -> (pane id, reply channel).
    /// Lock order: `permits`, then `panes`, then `store`. `on_permit`,
    /// `permit_answer` and `answer_permits` hold `permits` while they change
    /// `Pane.permit`, so the two always agree (SPEC 25). No code takes
    /// `permits` while it holds `panes`.
    permits: Mutex<HashMap<String, (String, Sender<PermitReply>)>>,
    /// The number part of the next permit id. A counter, not the clock: the
    /// clock can repeat a value (SPEC 25, F5).
    next_permit: AtomicU64,
    /// Counts process starts in panes (new panes and resumes). The tick reads it
    /// before it lists tmux, so it knows which panes its list is too old for.
    spawns: AtomicU64,
    pub(crate) sink: Arc<dyn Sink>,
    pub(crate) sb: PathBuf,
    pub(crate) reviews: Mutex<HashMap<String, crate::hub_review::ReviewState>>,
    /// Held while a review worktree is made or removed, and while a close takes a
    /// review out. So a removal never meets a checkout of the same folder (SPEC 11.10).
    /// Lock order: `trees`, then `reviews`.
    pub(crate) trees: Mutex<()>,
    /// The last size each window asked for.
    size: Mutex<HashMap<Win, (u16, u16)>>,
    /// The size of each window on screen, by tmux id. The redraw after a show
    /// reads it again, so a resize in the meantime stays (SPEC 28).
    sizes: Arc<Mutex<HashMap<String, (u16, u16)>>>,
    /// The panes an expand resumes now, so a second click does not start a second process.
    resuming: Mutex<Vec<String>>,
    /// The panes that an idle end stops now (SPEC 15), so a tick starts one stop.
    stopping: Mutex<Vec<String>>,
    /// The task PR links that GitHub reports as merged (SPEC 27.6). Merged is final,
    /// so a link in it is not read again.
    pub(crate) merged: Mutex<Vec<String>>,
    /// True while a PR poll runs, so two never overlap.
    pub(crate) polling: std::sync::atomic::AtomicBool,
    /// True while the nvim poll runs, so two never overlap.
    nvim_polling: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) me: Weak<Hub>,
    /// Messages between agents (SPEC 40).
    pub(crate) mail: Mutex<crate::hub_mail::MailState>,
    /// GitHub for a send and for the pending review (SPEC 11.8.1). A test swaps it.
    pub(crate) gh: RwLock<Arc<dyn crate::gh::Gh>>,
    /// Today (SPEC 39): the last refresh.
    pub(crate) today: Mutex<crate::hub_today::TodayState>,
    /// A window asked `sb serve` to start again (SPEC 29.12).
    restart: std::sync::atomic::AtomicBool,
    /// The pending review sync runs. Off with `SB_HOME` set (tests), until a test
    /// gives a fake GitHub: a test never writes to GitHub.
    pub(crate) gh_sync: std::sync::atomic::AtomicBool,
    /// The lock on `hub.lock`. It lives as long as the hub.
    _lock: std::fs::File,
}

/// Between the fields of a tmux format. Printable ASCII: tmux puts `_` in place of a
/// tab or any other control character for a client it thinks has no UTF-8.
const FIELD_SEP: &str = "|~|";

/// A tmux format of `fields` joined by `FIELD_SEP`. Only the last field may hold
/// any text, such as a path.
fn fields_format(fields: &[&str]) -> String {
    fields.join(FIELD_SEP)
}

/// The `n` fields of one line of `fields_format` output, or None when the line is
/// not one: wrong count, or no pane id first.
fn split_fields(line: &str, n: usize) -> Option<Vec<&str>> {
    let f: Vec<&str> = line.splitn(n, FIELD_SEP).collect();
    (f.len() == n && f[0].starts_with('%')).then_some(f)
}

type Res<T> = Result<T, String>;
type Env = Vec<(String, String)>;

/// Takes the lock that keeps a second hub away from the same home. With two hubs,
/// both would reattach tmux, sweep worktrees and take the socket.
fn take_lock(path: &Path) -> Res<std::fs::File> {
    use std::os::fd::AsRawFd;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|err| format!("{}: {err}", path.display()))?;
    // SAFETY: flock on a file descriptor that `f` owns.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(format!("another Switchboard runs with {}", path.parent().unwrap_or(path).display()));
    }
    Ok(f)
}

/// True when tmux names a shell as the program in front, so a shell pane waits at
/// its prompt.
fn is_shell(command: &str) -> bool {
    let own = std::env::var("SHELL").unwrap_or_default();
    let own = own.rsplit('/').next().unwrap_or("");
    // Nix runs a shell as `.zsh-wrapped`, and a login shell shows as `-zsh`.
    let name = command.trim_start_matches(['.', '-']).trim_end_matches("-wrapped");
    command.is_empty()
        || name == own
        || matches!(name, "bash" | "zsh" | "fish" | "sh" | "dash" | "ksh" | "nu" | "tcsh" | "csh")
}

/// The image types that Claude Code reads from a pasted path.
const PASTE_TYPES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];
/// The largest image the window can paste. A screenshot of a big screen is a few MB.
const PASTE_MAX: usize = 32 << 20;
/// How long a pasted image stays. The agent reads it in its next turn.
const PASTE_KEEP: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);

/// Removes the pasted images older than `keep`.
fn sweep_pastes(dir: &Path, keep: std::time::Duration) {
    let Ok(list) = std::fs::read_dir(dir) else { return };
    for f in list.flatten() {
        let old = f.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > keep);
        if old {
            let _ = std::fs::remove_file(f.path());
        }
    }
}

/// `text` as one bracketed paste, with no control characters but newline and tab.
/// So a text from an agent (a task note, a draft) can never end the paste early or
/// press Enter: you read the paste and send it yourself.
pub fn bracketed_paste(text: &str) -> String {
    let clean: String = text
        .chars()
        .filter(|&c| c == '\n' || c == '\t' || !(c.is_control() || ('\u{80}'..='\u{9f}').contains(&c)))
        .collect();
    format!("\x1b[200~{clean}\x1b[201~")
}

fn e(x: impl std::fmt::Display) -> String {
    x.to_string()
}

impl Hub {
    /// Starts everything: the tmux client, the store, the socket and the ticker.
    pub fn start(paths: Paths, sb: PathBuf, sink: Arc<dyn Sink>) -> Res<Arc<Hub>> {
        tracing::info!(target: "sb::hub", home = %paths.home.display(), sock = %paths.sock().display(), "hub starting");
        paths.create_all().map_err(e)?;
        // One hub for each home, before any change to tmux, the store or the socket.
        let lock = take_lock(&paths.home.join("hub.lock"))?;
        std::fs::write(paths.tmux_conf(), tmux::CONF).map_err(e)?;
        // Sockets of nvims that are gone (SPEC 10). The hub lock is ours, so no
        // other hub makes one meanwhile.
        let swept = nvim::sweep(&paths.nvim_dir());
        if swept > 0 {
            tracing::info!(target: "sb::hub", swept, "removed old nvim sockets");
        }
        let (config, config_error) = match Config::load(&paths.config) {
            Ok(c) => (c, None),
            Err(err) => (Config::default(), Some(err)),
        };
        let store = Store::open(&paths.store()).map_err(|err| format!("{}: {err}", paths.store().display()))?;

        let live: Live = Arc::default();
        let terms: Arc<Mutex<Terms>> = Arc::default();
        let reader_live = Arc::clone(&live);
        let reader_terms = Arc::clone(&terms);
        let reader_sink = Arc::clone(&sink);
        let socket = std::env::var("SB_TMUX_SOCKET").unwrap_or_else(|_| "switchboard".into());
        let tmux = Tmux::start(&socket, &paths.tmux_conf(), move |ev| match ev {
            tmux::Event::Output { pane, data } => {
                let to = reader_live.lock().unwrap().watchers(&pane);
                if !to.is_empty() {
                    let mut terms = reader_terms.lock().unwrap();
                    terms.feed(&pane, &data);
                    for (win, id) in &to {
                        reader_sink.output_to(*win, id, &data);
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
            terms,
            screen: Mutex::default(),
            permits: Mutex::default(),
            next_permit: AtomicU64::new(1),
            spawns: AtomicU64::new(0),
            sink,
            sb,
            reviews: Mutex::default(),
            size: Mutex::default(),
            sizes: Arc::default(),
            resuming: Mutex::default(),
            stopping: Mutex::default(),
            trees: Mutex::new(()),
            merged: Mutex::default(),
            polling: std::sync::atomic::AtomicBool::new(false),
            nvim_polling: Arc::default(),
            me: me.clone(),
            mail: Mutex::default(),
            gh: RwLock::new(Arc::new(crate::gh::RealGh)),
            today: Mutex::default(),
            restart: Default::default(),
            gh_sync: std::sync::atomic::AtomicBool::new(std::env::var_os("SB_HOME").is_none()),
            _lock: lock,
        });
        hub.keep_session()?;
        hub.reattach()?;
        hub.load_reviews();
        hub.start_local();
        hub.prune_tasks();
        // Old review worktrees go in the background: a removal can take seconds.
        let h = hub.clone();
        std::thread::spawn(move || h.sweep_closed());
        if let Some(err) = config_error {
            hub.sink
                .emit("error", json!({ "scope": "config", "message": err }));
        }
        hub.listen()?;
        hub.tick_loop();
        hub.mail_link_start();
        hub.mail_restore();
        Ok(hub)
    }

    pub(crate) fn arc(&self) -> Arc<Hub> {
        self.me.upgrade().expect("the hub is alive")
    }

    /// `hub_restart`: asks `sb serve` to start again with its newest build. The
    /// app's own hub ignores it: the app restarts itself.
    pub fn ask_restart(&self) -> Result<(), String> {
        tracing::info!(target: "sb::hub", "a restart is asked");
        self.restart.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    pub fn restart_asked(&self) -> bool {
        self.restart.load(std::sync::atomic::Ordering::SeqCst)
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
            .list(&fields_format(&["#{pane_id}", "#{@sb_pane}", "#{pane_dead}", "#{pane_dead_status}", "#{pane_pid}", "#{@sb_keep}", "#{pane_current_path}"]))
            .map_err(e)?;
        // Never adopt or close a pane from output we cannot read: a client that tmux
        // thinks has no UTF-8 gets `_` in place of a tab, and every pane would look new.
        let parsed: Vec<Vec<&str>> = lines
            .iter()
            .map(|l| split_fields(l, 7))
            .collect::<Option<_>>()
            .ok_or_else(|| format!("tmux pane list did not parse: {:?}", lines.first()))?;
        let store = self.store.lock().unwrap();
        let rows = store.open_panes().map_err(e)?;
        let mut panes = vec![];
        let mut found = std::collections::HashSet::new();
        for f in &parsed {
            let (tid, tag, dead, path, pid) = (
                f[0],
                f[1],
                f[2] == "1",
                if f[6].is_empty() { "/" } else { f[6] },
                f[4].parse().ok(),
            );
            let status: i32 = f[3].parse().unwrap_or(0);
            if f[5] == "1" {
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
        tracing::info!(target: "sb::hub", panes = panes.len(), tmux_panes = lines.len(), "reattached");
        *self.panes.lock().unwrap() = panes;
        Ok(())
    }

    /// A new mark for a process start. It is above every mark a tick took before.
    fn next_spawn(&self) -> u64 {
        self.spawns.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
    }

    fn pane_from(&self, row: PaneRow, tmux: Option<String>, pid: Option<u32>) -> Pane {
        let branch =
            repos::git(Path::new(&row.tree), &["branch", "--show-current"]).unwrap_or_default();
        let sandboxed = row.kind == "claude" && self.sandbox_file_on(&row.id);
        Pane {
            sandboxed,
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
            spawn: self.next_spawn(),
            busy: false,
            modified: false,
        }
    }

    // ---------- views ----------

    fn view(&self, p: &Pane) -> PaneView {
        let live = self.live.lock().unwrap().shows(&p.row.id);
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
            busy: p.busy,
            modified: p.modified,
            sandboxed: p.sandboxed,
        }
    }

    /// True when the settings file of a pane turns the sandbox on (SPEC 31).
    fn sandbox_file_on(&self, id: &str) -> bool {
        crate::sandbox::file_on(&self.paths.settings_dir().join(format!("{id}.json")))
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
        tracing::info!(target: "sb::hub", repo = %req.repo, place = %req.place, branch = ?req.branch, tree = ?req.tree, run = %req.run, "open");
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
            "claude" | "shell" => {
                let v = self.spawn(&req.run, &name, &tree, req.title, Start::default())?;
                if let Some(task) = &req.task {
                    self.task_started(task, &v.id);
                }
                if let Some(first) = req.first.filter(|f| !f.trim().is_empty()) {
                    if req.run == "claude" {
                        self.paste_when_ready(v.id.clone(), first);
                    }
                }
                Ok(v)
            }
            other => Err(format!("unknown run {other}")),
        }
    }

    /// Starts a pane.
    pub(crate) fn spawn(
        &self,
        kind: &str,
        repo: &str,
        tree: &Path,
        title: Option<String>,
        start: Start,
    ) -> Res<PaneView> {
        let id = self.store.lock().unwrap().next_id("p").map_err(e)?;
        let connection = (kind == "claude")
            .then(|| start.connection.clone().unwrap_or_else(|| self.active_connection()));
        let (argv, env) = self.command(kind, &id, tree, connection.as_deref(), &start)?;
        let argv_ref: Vec<&str> = argv.iter().map(String::as_str).collect();
        let env_ref: Vec<(&str, &str)> =
            env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let env_names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        tracing::info!(target: "sb::hub", pane = %id, kind, tree = %tree.display(), ?argv, env = ?env_names, "spawn");
        let tid = self.tmux.new_pane(tree, &argv_ref, &env_ref).map_err(e)?;
        tracing::info!(target: "sb::hub", pane = %id, tmux = %tid, "spawned");
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
            // A fork gets a new session id. The hooks report it.
            session: start.resume.clone().filter(|_| !start.fork),
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
        start: &Start,
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
        let rust = start.rust.unwrap_or_else(|| tree.join("Cargo.toml").exists());
        let config = self.config();
        // A pane that starts again keeps its sandbox: a resumed Ask pane has no `ask`.
        let sandboxed = config.sandbox.covers(start.ask) || crate::sandbox::file_on(&settings);
        // The pane tools (SPEC 36): every agent pane but one with its own server (Ask).
        let tools = config.pane_tools.enabled && start.mcp.is_none();
        let mut allow = start.allow.clone();
        if tools {
            allow.push(hooks::PANE_TOOLS_RULE.to_owned());
            env.push(("SB_TOOLS".to_owned(), "1".to_owned()));
        }
        let pane = hooks::PaneSettings {
            sb: &self.sb,
            rust,
            allow: &allow,
            sandbox: sandboxed.then(|| crate::sandbox::block(&config.sandbox, tree)),
            private: &[self.paths.data.to_string_lossy().into_owned()],
        };
        std::fs::write(&settings, hooks::settings(&pane).to_string()).map_err(e)?;
        tracing::info!(target: "sb::hub", pane = id, sandboxed, "pane settings");
        env.extend(connections::env(
            &self.config(),
            connection.unwrap_or(connections::DEFAULT),
        )?);
        // Full-screen mode scrolls inside Claude Code: the terminal keeps no history
        // and shows no scroll bar. The normal mode wins over `"tui": "fullscreen"`.
        if !config.claude_fullscreen {
            env.push(("CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN".to_owned(), "1".to_owned()));
        }
        // The first message goes first: `--add-dir` takes a list.
        let mut argv = vec!["claude".to_owned()];
        argv.extend(start.first.clone());
        argv.extend(["--settings".to_owned(), settings.to_string_lossy().into_owned()]);
        // The pane hooks come from `--settings`, which loads with any sources.
        if let Some(s) = start.setting_sources {
            argv.extend(["--setting-sources".to_owned(), s.to_owned()]);
        }
        if let Some(s) = &start.resume {
            argv.extend(["--resume".into(), s.clone()]);
            if start.fork {
                argv.push("--fork-session".into());
            }
        }
        if let Some(p) = &start.system {
            argv.extend(["--append-system-prompt".into(), p.clone()]);
        }
        // `--mcp-config` and `--add-dir` take lists: a flag ends each one.
        if let Some(cfg) = &start.mcp {
            argv.extend(["--mcp-config".into(), cfg.clone(), "--strict-mcp-config".into()]);
        } else if tools {
            // Not strict: the user's own servers still load beside it.
            argv.extend(["--mcp-config".into(), hooks::pane_tools_config(&self.sb, id, &self.paths.sock()).to_string()]);
        }
        for d in &start.dirs {
            argv.extend(["--add-dir".into(), d.clone()]);
        }
        Ok((argv, env))
    }

    pub fn close(&self, id: &str) -> Res<()> {
        tracing::info!(target: "sb::hub", pane = id, "close");
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
            self.sizes.lock().unwrap().remove(&t);
        }
        // The live pane takes its parts with it. A part goes alone.
        let _screen = self.screen.lock().unwrap();
        let off = {
            let mut live = self.live.lock().unwrap();
            let gone = live.close(id);
            live.unwatched(gone)
        };
        self.stop_streams(&off);
        self.store
            .lock()
            .unwrap()
            .close_pane(id, now())
            .map_err(e)?;
        self.answer_permits(id, None);
        let _ = std::fs::remove_file(self.paths.settings_dir().join(format!("{id}.json")));
        let _ = std::fs::remove_file(self.paths.state_dir().join(format!("{id}.json")));
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
        tracing::info!(target: "sb::hub", ?ids, "reorder");
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
        tracing::info!(target: "sb::hub", pane = id, ?title, "rename");
        let title = title.filter(|t| !t.trim().is_empty());
        // The Work environment tab finds its agent by this title (SPEC 30): no
        // other pane gets it, and that agent keeps it.
        let (kind, old) = self
            .panes
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.row.id == id)
            .map(|p| (p.row.kind.clone(), p.row.title.clone()))
            .ok_or(format!("no pane {id}"))?;
        let work = |t: &Option<String>| t.as_deref().map(str::trim) == Some(WORK_TITLE);
        if kind == "claude" && work(&old) && !work(&title) {
            return Err(format!("The {WORK_TITLE} agent keeps its name."));
        }
        if work(&title) && !work(&old) {
            return Err(format!("\"{WORK_TITLE}\" is the name of the agent of the {WORK_TITLE} tab. Pick another name."));
        }
        self.with_pane(id, false, |p| p.row.title = title)
    }

    // ---------- the live pane ----------

    /// Makes a pane live in the app's own window. See [`Hub::expand_in`].
    pub fn expand(&self, id: &str) -> Res<Vec<u8>> {
        self.expand_in(LOCAL, id)
    }

    /// Makes a pane live in one window and returns its screen: the scrollback, then
    /// the cursor. The parts of the window's old live pane leave its screen too.
    /// Other windows keep their screens (SPEC 41.8).
    pub fn expand_in(&self, win: Win, id: &str) -> Res<Vec<u8>> {
        tracing::info!(target: "sb::hub", win, pane = id, "expand");
        self.resume_if_ended(id)?;
        let _screen = self.screen.lock().unwrap();
        let tid = self.tmux_id(id)?;
        let off = {
            let mut live = self.live.lock().unwrap();
            let old = std::mem::replace(live.of_mut(win), vec![(id.to_owned(), tid.clone())]);
            live.unwatched(old.into_iter().map(|(_, t)| t))
        };
        self.stop_streams(&off);
        let (cols, rows) = self.size_of(win);
        self.show(win, id, &tid, cols, rows)
    }

    /// The last size a window asked for.
    fn size_of(&self, win: Win) -> (u16, u16) {
        self.size.lock().unwrap().get(&win).copied().unwrap_or((200, 50))
    }

    /// The streams of tmux panes that no window shows any more stop.
    fn stop_streams(&self, tids: &[String]) {
        for t in tids {
            let _ = self.tmux.set_streaming(t, false);
            self.terms.lock().unwrap().stop(t);
        }
    }

    /// An ended Claude pane with a session starts again before it shows, so a click
    /// on its tile opens the agent and not an error. Its process is gone, for example
    /// after a restart of the Mac. A pane with a process is left alone.
    fn resume_if_ended(&self, id: &str) -> Res<()> {
        let ended = self
            .panes
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.row.id == id && p.tmux.is_none() && p.row.kind == "claude" && p.row.session.is_some());
        if !ended {
            return Ok(());
        }
        {
            let mut r = self.resuming.lock().unwrap();
            if r.iter().any(|x| x == id) {
                return Err("the agent is starting again".into());
            }
            r.push(id.to_owned());
        }
        tracing::info!(target: "sb::hub", pane = id, "expand resumes an ended pane");
        let res = self.resume(id);
        self.resuming.lock().unwrap().retain(|x| x != id);
        res
    }

    /// Adds a part to the screen of the app's own window. See [`Hub::attach_in`].
    pub fn attach(&self, id: &str, owner: &str, cols: u16, rows: u16) -> Res<Vec<u8>> {
        self.attach_in(LOCAL, id, owner, cols, rows)
    }

    /// Adds a part (nvim or the shell) to the screen of a window, at its own size,
    /// and returns its screen (SPEC 28). The window sizes its terminal first.
    /// `owner` is the live pane the window shows, so a late attach never adds a
    /// part to another agent.
    pub fn attach_in(&self, win: Win, id: &str, owner: &str, cols: u16, rows: u16) -> Res<Vec<u8>> {
        tracing::info!(target: "sb::hub", win, pane = id, owner, cols, rows, "attach");
        let _screen = self.screen.lock().unwrap();
        let tid = self.tmux_id(id)?;
        {
            let mut live = self.live.lock().unwrap();
            let screen = live.of_mut(win);
            if screen.first().map(|(l, _)| l != owner).unwrap_or(true) {
                return Err(format!("{owner} is not the live pane"));
            }
            if !screen.iter().any(|(l, _)| l == id) {
                screen.push((id.to_owned(), tid.clone()));
            }
        }
        self.show(win, id, &tid, cols, rows)
    }

    /// Takes a part off the screen of the app's own window. See [`Hub::detach_in`].
    pub fn detach(&self, id: &str) {
        self.detach_in(LOCAL, id)
    }

    /// Takes a part off the screen of a window. Its process keeps running.
    pub fn detach_in(&self, win: Win, id: &str) {
        tracing::info!(target: "sb::hub", win, pane = id, "detach");
        let _screen = self.screen.lock().unwrap();
        let off = {
            let mut live = self.live.lock().unwrap();
            let screen = live.of_mut(win);
            // The live pane itself goes only with a collapse.
            if screen.first().map(|(l, _)| l == id).unwrap_or(false) {
                return;
            }
            let (out, keep): (Vec<_>, Vec<_>) = screen.drain(..).partition(|(l, _)| l == id);
            *screen = keep;
            live.unwatched(out.into_iter().map(|(_, t)| t))
        };
        self.stop_streams(&off);
    }

    /// Sets the size of a tmux pane, and of its terminal emulator.
    fn set_pane_size(&self, tid: &str, cols: u16, rows: u16) -> Res<()> {
        self.sizes.lock().unwrap().insert(tid.to_owned(), (cols, rows));
        self.terms.lock().unwrap().resize(tid, cols, rows);
        self.tmux.set_window_size(tid, cols, rows).map_err(e)
    }

    /// The screen of a pane that goes on the screen of a window, and its stream on.
    fn show(&self, win: Win, id: &str, tid: &str, cols: u16, rows: u16) -> Res<Vec<u8>> {
        self.sink.reset_for(win, id);
        let _ = self.set_pane_size(tid, cols, rows);
        let lines = self.tmux.capture(tid, "-").map_err(e)?;
        let (x, y) = self.tmux.cursor(tid).map_err(e)?;
        let mut screen = b"\x1b[H\x1b[2J\x1b[3J".to_vec();
        screen.extend(lines.join("\r\n").into_bytes());
        screen.extend(format!("\x1b[{};{}H", y + 1, x + 1).into_bytes());
        // The emulator starts before the stream: no byte of the stream misses it.
        // A pane that streams for another window already keeps its emulator.
        self.terms.lock().unwrap().start(tid, cols, rows, &screen);
        self.tmux.set_streaming(tid, true).map_err(e)?;
        self.seen(id)?;
        // Output in the gap between the capture and `continue` is lost. A size
        // change makes the program in the pane draw its whole screen again.
        // The size is read again before each step: the window can resize meanwhile.
        let tmux = Arc::clone(&self.tmux);
        let sizes = Arc::clone(&self.sizes);
        let terms = Arc::clone(&self.terms);
        let tid = tid.to_owned();
        std::thread::spawn(move || {
            let now = || sizes.lock().unwrap().get(&tid).copied().unwrap_or((cols, rows));
            let set = |c: u16, r: u16| {
                terms.lock().unwrap().resize(&tid, c, r);
                let _ = tmux.set_window_size(&tid, c, r);
            };
            std::thread::sleep(Duration::from_millis(80));
            let (c, r) = now();
            set(c, r.saturating_sub(1).max(2));
            std::thread::sleep(Duration::from_millis(80));
            let (c, r) = now();
            set(c, r);
        });
        Ok(screen)
    }

    /// A fresh screen of a pane on the screen of a window, for a window that lost
    /// bytes (SPEC 41.8). It comes from the pane's terminal emulator, so tmux never
    /// pauses and no other window loses a byte. `f` gets the screen, or `None` when
    /// the window does not show the pane. It runs under the lock that the reader
    /// thread holds while it queues bytes: so the screen holds every byte before it,
    /// and the queue gets every byte after it.
    pub fn fresh_screen<T>(&self, win: Win, id: &str, f: impl FnOnce(Option<Vec<u8>>) -> T) -> T {
        let tid = self.live.lock().unwrap().tid_in(win, id);
        let terms = self.terms.lock().unwrap();
        f(tid.and_then(|t| terms.snapshot(&t)))
    }

    /// Takes the screen of the app's own window away. See [`Hub::collapse_in`].
    pub fn collapse(&self) {
        self.collapse_in(LOCAL)
    }

    /// Takes the screen of a window away: when it leaves, or shows no pane. The
    /// screens of other windows stay.
    pub fn collapse_in(&self, win: Win) {
        tracing::info!(target: "sb::hub", win, "collapse");
        // A panic under the lock poisons it: the value is still good. A window that
        // leaves after a panic collapses too.
        let _screen = self.screen.lock().unwrap_or_else(|e| e.into_inner());
        // Take the value first: an `if let` on the guard keeps the lock for the whole block.
        let (first, off) = {
            let mut live = self.live.lock().unwrap();
            let taken = live.take(win);
            let first = taken.first().map(|(id, _)| id.clone());
            (first, live.unwatched(taken.into_iter().map(|(_, t)| t)))
        };
        self.stop_streams(&off);
        if let Some(id) = first {
            if let Ok(panes) = self.panes.lock() {
                if let Some(p) = panes.iter().find(|p| p.row.id == id) {
                    self.emit_pane(p, false);
                }
            }
        }
    }

    /// An image from the window's clipboard (SPEC 29.10). The hub saves it in
    /// `~/.switchboard/paste` and types its path into the pane as a paste, with no
    /// Enter. Claude Code reads the path as an image. So an image reaches an agent on
    /// a remote hub, where the agent has no access to the window's clipboard.
    pub fn paste_image(&self, id: &str, data: &[u8], ext: &str) -> Res<String> {
        if !PASTE_TYPES.contains(&ext) {
            return Err(format!("{ext} is not an image type the agent reads"));
        }
        if data.is_empty() || data.len() > PASTE_MAX {
            return Err(format!("the image has {} bytes; the limit is {} MB", data.len(), PASTE_MAX >> 20));
        }
        let dir = self.paths.home.join("paste");
        std::fs::create_dir_all(&dir).map_err(e)?;
        sweep_pastes(&dir, PASTE_KEEP);
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let path = dir.join(format!("{}-{:09}.{ext}", at.as_secs(), at.subsec_nanos()));
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).map_err(e)?;
            f.write_all(data).map_err(e)?;
        }
        let text = path.to_string_lossy().into_owned();
        tracing::info!(target: "sb::hub", pane = id, path = %text, bytes = data.len(), "paste image");
        // The space after the path ends it, so text you type next is not part of it.
        self.input(id, bracketed_paste(&format!("{text} ")).as_bytes())?;
        Ok(text)
    }

    pub fn input(&self, id: &str, bytes: &[u8]) -> Res<()> {
        tracing::trace!(target: "sb::hub", pane = id, len = bytes.len(), "input");
        let tid = self
            .tmux_id(id)
            .inspect_err(|err| tracing::warn!(target: "sb::hub", pane = id, %err, "input to a pane with no process"))?;
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

    /// The size of the live pane's terminal in the app's own window. See [`Hub::resize_in`].
    pub fn resize(&self, cols: u16, rows: u16) -> Res<()> {
        self.resize_in(LOCAL, cols, rows)
    }

    /// The size of the live pane's terminal in a window. Its next expand takes it too.
    pub fn resize_in(&self, win: Win, cols: u16, rows: u16) -> Res<()> {
        tracing::debug!(target: "sb::hub", win, cols, rows, "resize");
        self.size.lock().unwrap().insert(win, (cols, rows));
        self.tmux.set_size(cols, rows).map_err(e)?;
        let first = self.live.lock().unwrap().of(win).first().map(|(_, t)| t.clone());
        if let Some(t) = first {
            self.set_pane_size(&t, cols, rows)?;
        }
        Ok(())
    }

    /// The size of a part's terminal (SPEC 28).
    pub fn part_resize(&self, id: &str, cols: u16, rows: u16) -> Res<()> {
        tracing::debug!(target: "sb::hub", pane = id, cols, rows, "part resize");
        let tid = self.tmux_id(id)?;
        self.set_pane_size(&tid, cols, rows)
    }

    pub fn seen(&self, id: &str) -> Res<()> {
        tracing::debug!(target: "sb::hub", pane = id, "seen");
        self.with_pane(id, false, |p| p.row.unseen = false)
    }

    // ---------- hook events (SPEC 6, 13.3) ----------

    pub fn on_state(&self, m: &StateMsg) {
        tracing::info!(target: "sb::hook", pane = %m.pane, event = %m.event, session = ?m.session, summary = ?m.summary, "hook event");
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
        if m.event == "turn" {
            let tree = self.panes.lock().unwrap().iter().find(|p| p.row.id == m.pane && p.row.kind == "claude").map(|p| p.row.tree.clone());
            if let Some(tree) = tree {
                self.turn_snapshot(&tree);
            }
            self.turn_mentions(&m.pane, m.detail.get("text").and_then(Value::as_str).unwrap_or(""));
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
        let id = permit_id(&self.next_permit, pane);
        let summary = hooks::permit_summary(tool, input);
        // Hold `permits` across the tile change and the insert. Else a
        // `working` line between the two takes the buttons and leaves the entry.
        let mut permits = self.permits.lock().unwrap();
        // A request from a process that `tick` saw exit gets no buttons.
        let ok = self
            .update_pane(pane, |p| {
                if p.dead {
                    return false;
                }
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
            .unwrap_or(false);
        if ok {
            permits.insert(id, (pane.to_owned(), reply));
        }
        ok
    }

    /// Answers one permission request from the tile.
    pub fn permit_answer(&self, permit: &str, allow: bool) -> Res<()> {
        tracing::info!(target: "sb::hub", permit, allow, "permit answer");
        let mut permits = self.permits.lock().unwrap();
        let (pane, tx) = permits
            .remove(permit)
            .ok_or("the request is gone: it was answered in the pane")?;
        let _ = tx.send(PermitReply {
            behavior: Some(if allow { "allow" } else { "deny" }.into()),
        });
        // Only the buttons of this request go. A newer request keeps its own.
        let res = self.with_pane(&pane, false, |p| {
            if p.permit.as_ref().map(|v| v.id == permit).unwrap_or(false) {
                p.permit = None;
                p.row.unseen = false;
                if allow {
                    p.row.lamp = Lamp::Working.as_str().into();
                }
            }
        });
        drop(permits);
        res
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
        // Still under `permits`: an `on_permit` cannot run between the two parts.
        let mut panes = self.panes.lock().unwrap();
        if let Some(p) = panes.iter_mut().find(|p| p.row.id == pane) {
            if p.permit.take().is_some() {
                self.emit_pane(p, false);
            }
        }
        drop(panes);
        drop(permits);
    }

    /// Answers the folder trust dialog (SPEC 6.6).
    pub fn trust(&self, id: &str, yes: bool) -> Res<()> {
        tracing::info!(target: "sb::hub", pane = id, yes, "trust answer");
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
        tracing::info!(target: "sb::hub", pane = id, "resume");
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
        // A folder that is gone (a closed review's worktree): Claude would start in
        // the home folder and ask to trust it.
        if !Path::new(&tree).is_dir() {
            return Err(format!("{tree} is gone, so the agent cannot start there again. Close the pane."));
        }
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
        // A request of the old process is gone: its buttons go too.
        self.answer_permits(id, None);
        // An Ask pane gets its tool, its rules and its user-only settings again
        // (SPEC 11.15): it runs in the PR's files.
        let (mcp, system, allow) = self.ask_extras(id);
        let ask = mcp.is_some();
        // A session with no conversation (an agent that never got a message) has no
        // file, and `--resume` fails: start a new one then.
        let file = crate::handover::project_dir(&self.claude_dir(Some(connection)), tree).join(format!("{session}.jsonl"));
        let has_talk = crate::handover::session_ok(session) && file.is_file();
        if !has_talk {
            tracing::info!(target: "sb::hub", pane = id, session, "no conversation to resume; a new session");
        }
        let start = Start {
            resume: has_talk.then(|| session.to_owned()),
            mcp,
            system,
            allow,
            setting_sources: ask.then_some(crate::review::SETTING_SOURCES),
            rust: ask.then_some(false),
            ask,
            ..Start::default()
        };
        let (argv, env) = self.command("claude", id, tree, Some(connection), &start)?;
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
        let screen = self.screen.lock().unwrap();
        let live = self.live.lock().unwrap().shows(id);
        if live {
            // The new process draws its screen from the start.
            let (cols, rows) = self.sizes.lock().unwrap().get(&tid).copied().unwrap_or_else(|| self.size_of(LOCAL));
            let mut terms = self.terms.lock().unwrap();
            terms.stop(&tid);
            terms.start(&tid, cols, rows, b"");
            drop(terms);
            // The screens follow the new tmux pane before its stream starts.
            self.live.lock().unwrap().retarget(id, &tid);
        }
        let _ = self.tmux.set_streaming(&tid, live);
        drop(screen);
        let pid = self
            .tmux
            .display(&tid, "#{pane_pid}")
            .ok()
            .and_then(|p| p.parse().ok());
        let sandboxed = self.sandbox_file_on(id);
        let spawn = self.next_spawn();
        self.with_pane(id, false, |p| {
            p.spawn = spawn;
            p.tmux = Some(tid);
            p.pid = pid;
            p.sandboxed = sandboxed;
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
        tracing::info!(target: "sb::hub", name, "connection set");
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
        tracing::info!(target: "sb::hub", pane = id, "connection resume");
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

    /// `stop_claude` on a thread, once per pane at a time.
    fn stop_in_thread(&self, id: &str, tid: &str) {
        {
            let mut stopping = self.stopping.lock().unwrap();
            if stopping.iter().any(|s| s == id) {
                return;
            }
            stopping.push(id.to_owned());
        }
        let (me, id, tid) = (self.me.clone(), id.to_owned(), tid.to_owned());
        let spawned = std::thread::Builder::new()
            .name("sb-idle-end".into())
            .spawn(move || {
                let Some(hub) = me.upgrade() else { return };
                tracing::info!(target: "sb::hub", pane = %id, "idle end");
                hub.stop_claude(&tid);
                hub.stopping.lock().unwrap().retain(|s| s != &id);
            });
        if spawned.is_err() {
            self.stopping.lock().unwrap().clear();
        }
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
        tracing::info!(target: "sb::hub", tree = %tree.display(), file = ?file, "nvim open");
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
            .map(|p| (p.row.id.clone(), p.tmux.clone(), p.dead));
        if let Some((id, tid, dead)) = existing {
            if nvim::alive(&sock) {
                if let Some((f, l)) = file {
                    nvim::expr(&sock, &nvim::edit_expr(f, l))?;
                }
                return Ok(id);
            }
            // No answer is not a dead nvim: one that is busy or waits at "Press
            // ENTER" never answers. Its buffers may hold unsaved work, so it stays.
            // The window shows it, and the file opens once it answers again.
            let runs = !dead
                && tid
                    .as_deref()
                    .is_some_and(|t| self.tmux.display(t, "#{pane_dead}").is_ok_and(|d| d == "0"));
            if runs {
                tracing::warn!(target: "sb::hub", pane = %id, "nvim does not answer; it may wait at a prompt");
                if let Some((f, l)) = file {
                    nvim::when_ready(sock, nvim::edit_expr(f, l), Duration::from_secs(60), "open file");
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
        if sock.exists() && nvim::expr(&sock, nvim::CHECKTIME).is_ok() {
            // The window says so in the nvim part (SPEC 28).
            self.sink.emit("nvim_reloaded", json!(tree.to_string_lossy()));
        }
    }

    /// Asks each nvim if it has a buffer that is not saved, on its own thread: an
    /// nvim at a prompt takes 3 s to fail (SPEC 28).
    fn poll_nvim(&self) {
        use std::sync::atomic::Ordering::SeqCst;
        let list: Vec<(String, String)> = self
            .panes
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.row.kind == "nvim" && p.tmux.is_some() && !p.dead)
            .map(|p| (p.row.id.clone(), p.row.tree.clone()))
            .collect();
        if list.is_empty() || self.nvim_polling.swap(true, SeqCst) {
            return;
        }
        let hub = self.arc();
        let flag = Arc::clone(&self.nvim_polling);
        std::thread::spawn(move || {
            struct Done(Arc<std::sync::atomic::AtomicBool>);
            impl Drop for Done {
                fn drop(&mut self) {
                    self.0.store(false, std::sync::atomic::Ordering::SeqCst);
                }
            }
            let _done = Done(flag);
            for (id, tree) in list {
                let sock = nvim::socket(&hub.paths.nvim_dir(), Path::new(&tree));
                let Ok(n) = nvim::expr(&sock, nvim::MODIFIED) else { continue };
                let modified = n.trim().parse::<u32>().map(|n| n > 0).unwrap_or(false);
                let mut panes = hub.panes.lock().unwrap();
                if let Some(p) = panes.iter_mut().find(|p| p.row.id == id && p.modified != modified) {
                    p.modified = modified;
                    hub.emit_pane(p, false);
                }
            }
        });
    }

    /// The shell of a worktree (SPEC 28): the one that runs there, or a new one.
    pub fn shell_open(&self, tree: &Path) -> Res<String> {
        tracing::info!(target: "sb::hub", tree = %tree.display(), "shell open");
        let existing = self
            .panes
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.row.kind == "shell" && Path::new(&p.row.tree) == tree && p.tmux.is_some() && !p.dead)
            .map(|p| p.row.id.clone());
        if let Some(id) = existing {
            return Ok(id);
        }
        let repo = repos::main_checkout(tree)
            .and_then(|m| m.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "shell".into());
        Ok(self.spawn("shell", &repo, tree, None, Start::default())?.id)
    }

    // ---------- repos and layouts ----------

    pub fn repos(&self) -> Vec<Repo> {
        let t0 = std::time::Instant::now();
        let list = {
            let roots: Vec<PathBuf> = self.config().scan.iter().map(|s| expand(s)).collect();
            repos::discover(&self.paths.claude.join("projects"), &roots)
        };
        tracing::info!(target: "sb::hub", ms = t0.elapsed().as_millis() as u64, count = list.len(), "repo list");
        list
    }

    /// The Settings fields, the config file, and the scan folders that do not exist
    /// (SPEC 14.1).
    pub fn settings_view(&self) -> Value {
        let c = self.config();
        let missing: Vec<&String> = c.scan.iter().chain(&c.review.spec_clones).filter(|s| !expand(s).is_dir()).collect();
        json!({
            "leader": c.leader,
            "nvim": crate::nvim::installed(),
            "sprint_days": c.tasks.sprint_days,
            "scan": c.scan,
            "spec_clones": c.review.spec_clones,
            "web_domains": c.review.web_domains,
            "web_search": c.review.web_search,
            "sandbox": c.sandbox,
            // Linux needs these for the sandbox (SPEC 31).
            "sandbox_missing": crate::sandbox::missing_tools(),
            "missing": missing,
            "config": self.paths.config.to_string_lossy(),
        })
    }

    /// Saves the Settings fields to the config file. They work at once: the next
    /// repo list, review and Ask pane read the new config.
    pub fn settings_save(&self, set: &crate::config::SettingsEdit) -> Res<Value> {
        let config = Config::edit(&self.paths.config, set)?;
        tracing::info!(target: "sb::hub", scan = ?config.scan, web = ?config.review.web_domains, "settings saved");
        *self.config.write().unwrap() = config;
        Ok(self.settings_view())
    }

    pub fn layout_save(&self, name: &str) -> Res<()> {
        tracing::info!(target: "sb::hub", name, "layout save");
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
        tracing::info!(target: "sb::hub", name, "layout open");
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
                    Start::default(),
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
            tracing::warn!(target: "sb::socket", line = %line.trim(), "bad request on the socket");
            return;
        };
        tracing::debug!(target: "sb::socket", kind = request_kind(&req), "request");
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
                    task: None,
                    first: None,
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
            Request::ReviewShow { review, points } => json!(match self.review_show(&review, &points) {
                Ok(()) => Ack::ok(),
                Err(err) => Ack::err(err),
            }),
            Request::Resume { pane } => json!(match self.resume(&pane) {
                Ok(()) => Ack::ok(),
                Err(err) => Ack::err(err),
            }),
            Request::ReviewWork { review, task } => match self.review_work(&review, &task) {
                Ok(v) => {
                    // The window shows the new agent.
                    if let Some(p) = v["pane"].as_str() {
                        self.sink.emit("focus", json!(p));
                    }
                    let mut v = v;
                    v["ok"] = json!(true);
                    v
                }
                Err(err) => json!(Ack::err(err)),
            },
            Request::Task { pane, op } => self.task_socket(pane.as_deref(), op),
            Request::Board { pane } => self.board_socket(pane.as_deref()),
            Request::Today { pane, job, repo, number } => {
                let item = repo.zip(number);
                self.today_socket(pane.as_deref(), job.as_deref(), item)
            }
            // The link of another machine keeps the connection (SPEC 40.5).
            Request::Mail(crate::proto::MailOp::Watch { machine }) => return self.mail_watch(&machine, conn),
            Request::Mail(op) => self.mail_socket(op),
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
                    let t0 = std::time::Instant::now();
                    hub.tick(n);
                    hub.mail_tick();
                    if n.is_multiple_of(30) {
                        hub.today_tick();
                    }
                    let ms = t0.elapsed().as_millis() as u64;
                    if ms > 800 {
                        tracing::warn!(target: "sb::tick", ms, "slow tick");
                    }
                    n += 1;
                }
            })
            .expect("spawn the ticker");
    }

    fn tick(&self, n: u64) {
        // Before the list: a pane that starts after this mark is not in the list
        // as it is now, so the tick leaves it alone (no "gone", no "dead").
        let seen = self.spawns.load(std::sync::atomic::Ordering::SeqCst);
        let Ok(lines) = self
            .tmux
            .list(&fields_format(&["#{pane_id}", "#{pane_dead}", "#{pane_dead_status}", "#{pane_pid}", "#{pane_current_command}"]))
        else {
            return;
        };
        // A list we cannot read would make every pane look gone: skip this tick.
        let Some(parsed) = lines.iter().map(|l| split_fields(l, 5)).collect::<Option<Vec<_>>>() else {
            tracing::warn!(target: "sb::tick", line = ?lines.first(), "tmux pane list did not parse; tick skipped");
            return;
        };
        if n.is_multiple_of(2) {
            self.poll_nvim();
        }
        let state: HashMap<String, (bool, i32, Option<u32>, String)> = parsed
            .iter()
            .filter_map(|f| {
                Some((
                    f.first()?.to_string(),
                    (
                        f.get(1) == Some(&"1"),
                        f.get(2).and_then(|s| s.parse().ok()).unwrap_or(0),
                        f.get(3).and_then(|s| s.parse().ok()),
                        f.get(4).copied().unwrap_or("").to_owned(),
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
            .filter(|p| p.spawn <= seen)
            .map(|p| (p.row.id.clone(), p.tmux.clone()))
            .collect();
        let mut gone = vec![];
        let mut ended = vec![];
        for (id, tid) in ids {
            let Some(tid) = tid else { continue };
            let Some((dead, status, pid, command)) = state.get(&tid) else {
                gone.push(id);
                continue;
            };
            let (dead, status, pid) = (*dead, *status, *pid);
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
            // Resumed while this tick ran: the list is older than the process.
            if p.spawn > seen || p.tmux.as_deref() != Some(tid.as_str()) {
                continue;
            }
            let row_before = p.row.clone();
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
            let busy = p.row.kind == "shell" && !dead && !is_shell(command);
            if p.busy != busy {
                p.busy = busy;
                changed = true;
            }
            if !dead && p.dead {
                // The pane runs again (the list is newer than its start).
                p.dead = false;
            }
            if dead && !p.dead {
                p.dead = true;
                if p.row.kind == "claude" {
                    ended.push(id.clone());
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
                // The stop waits up to 5 s, so it runs on a thread of its own:
                // the tick must not wait. `stopping` stops a second thread.
                drop(panes);
                self.stop_in_thread(&id, &tid);
                continue;
            }
            if changed {
                // The tail and RSS are not stored. Write only when the row changed:
                // a write holds the pane lock, and keys wait for that lock.
                if p.row != row_before {
                    let _ = self.store.lock().unwrap().save_pane(&p.row);
                }
                self.emit_pane(p, alert);
            }
        }
        // After the loop: `answer_permits` takes `permits`, and that lock
        // comes before `panes`.
        for id in ended {
            self.answer_permits(&id, None);
        }
        for id in gone {
            // A pane resumed since the list is not gone.
            let kind = self
                .panes
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.row.id == id && p.spawn <= seen)
                .map(|p| p.row.kind.clone());
            match kind.as_deref() {
                Some("claude") => {
                    let _ = self.with_pane(&id, false, |p| {
                        if p.spawn <= seen {
                            p.tmux = None;
                            p.row.lamp = Lamp::Ended.as_str().into();
                        }
                    });
                    self.answer_permits(&id, None);
                }
                Some(_) => {
                    let _ = self.close(&id);
                }
                None => {}
            }
        }
        self.review_tick(n);
        self.task_tick(n);
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

/// A new permit id: the pane, then the counter. Two calls never give the same id.
fn permit_id(next: &AtomicU64, pane: &str) -> String {
    format!("{pane}-{}", next.fetch_add(1, Ordering::Relaxed))
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

fn request_kind(r: &Request) -> &'static str {
    match r {
        Request::State(_) => "state",
        Request::Permit { .. } => "permit",
        Request::Open { .. } => "open",
        Request::GuideSet { .. } => "guide_set",
        Request::GuideUpdate { .. } => "guide_update",
        Request::ReviewShow { .. } => "review_show",
        Request::ReviewWork { .. } => "review_work",
        Request::Resume { .. } => "resume",
        Request::Task { .. } => "task",
        Request::Board { .. } => "board",
        Request::Today { .. } => "today",
        Request::Mail(_) => "mail",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tmux_fields_split_and_refuse_bad_lines() {
        let fmt = super::fields_format(&["#{pane_id}", "#{pane_dead}", "#{pane_current_path}"]);
        assert_eq!(fmt, "#{pane_id}|~|#{pane_dead}|~|#{pane_current_path}");
        assert_eq!(super::split_fields("%3|~|0|~|/a/b|~|c", 3), Some(vec!["%3", "0", "/a/b|~|c"]));
        assert_eq!(super::split_fields("%3|~||~|", 3), Some(vec!["%3", "", ""]));
        // What tmux sent a client with no UTF-8: tabs turned into `_`.
        assert_eq!(super::split_fields("%0_p42_0__/Users/eitan_1473_", 7), None);
        assert_eq!(super::split_fields("%3|~|0", 3), None);
        assert_eq!(super::split_fields("x|~|0|~|/", 3), None);
    }

    use super::*;

    #[test]
    fn shell_names_mean_the_prompt() {
        for c in ["bash", "zsh", "-zsh", ".zsh-wrapped", "fish", ""] {
            assert!(is_shell(c), "{c}");
        }
        for c in ["cargo", "sleep", "nvim", "python3"] {
            assert!(!is_shell(c), "{c}");
        }
    }

    #[test]
    fn permit_ids_never_repeat() {
        // The old ids came from the clock, and two calls in the same
        // nanosecond gave the same id (SPEC 25, F5).
        let next = AtomicU64::new(1);
        let a = permit_id(&next, "p1");
        let b = permit_id(&next, "p1");
        assert_ne!(a, b);
        assert!(a.starts_with("p1-") && b.starts_with("p1-"));
    }

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

    #[test]
    fn a_paste_has_no_control_characters() {
        assert_eq!(bracketed_paste("a\nb\tc"), "\x1b[200~a\nb\tc\x1b[201~");
        // An agent's note that tries to end the paste and press Enter.
        assert_eq!(bracketed_paste("x\x1b[201~\rrm -rf /\u{9b}2~\x07"), "\x1b[200~x[201~rm -rf /2~\x1b[201~");
    }
}
