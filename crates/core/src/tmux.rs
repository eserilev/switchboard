//! One control-mode client for the Switchboard tmux server.
//!
//! The server runs on its own socket (`tmux -L switchboard`), so your own
//! tmux is never touched. It outlives the app: agents keep running when the
//! window closes, and the next start attaches again.

use crate::control::{Message, Parser};
use std::collections::VecDeque;
use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The name of the one tmux session.
pub const SESSION: &str = "sb";

const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
const SEND_CHUNK: usize = 256;

/// What the server tells us without a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Output {
        pane: String,
        data: Vec<u8>,
    },
    WindowAdd(String),
    WindowClose(String),
    /// The control client ended. No more events come after this one.
    Exit(Option<String>),
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// tmux refused the command. The text is its error line.
    Tmux(String),
    /// The control client is gone.
    Closed,
    Timeout,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "tmux: {e}"),
            Error::Tmux(m) => write!(f, "tmux: {m}"),
            Error::Closed => f.write_str("tmux: the control client is closed"),
            Error::Timeout => f.write_str("tmux: no reply in 10 s"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

type Reply = Result<Vec<String>, Error>;

struct Link {
    stdin: ChildStdin,
    waiters: VecDeque<Sender<Reply>>,
    open: bool,
}

pub struct Tmux {
    socket: String,
    link: Arc<Mutex<Link>>,
    child: Mutex<Child>,
}

impl Tmux {
    /// Starts the server if it is not running, and attaches a control client.
    /// `on_event` runs on the reader thread for every event.
    pub fn start(
        socket: &str,
        conf: &Path,
        on_event: impl Fn(Event) + Send + 'static,
    ) -> Result<Arc<Tmux>, Error> {
        let mut child = Command::new("tmux")
            .args(["-L", socket, "-f"])
            .arg(conf)
            .args([
                "-C",
                "new-session",
                "-A",
                "-s",
                SESSION,
                "-x",
                "200",
                "-y",
                "50",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");

        // The attach command gets a reply block of its own. This waiter takes it.
        let (first_tx, first_rx) = mpsc::channel();
        let link = Arc::new(Mutex::new(Link {
            stdin,
            waiters: VecDeque::from([first_tx]),
            open: true,
        }));

        let reader_link = Arc::clone(&link);
        std::thread::Builder::new()
            .name("tmux-reader".into())
            .spawn(move || read_loop(BufReader::new(stdout), &reader_link, on_event))?;

        let tmux = Arc::new(Tmux {
            socket: socket.to_owned(),
            link,
            child: Mutex::new(child),
        });
        wait(first_rx)?;
        Ok(tmux)
    }

    /// Runs one tmux command and returns its reply lines.
    pub fn cmd(&self, command: &str) -> Reply {
        let rx = {
            let mut link = self.link.lock().unwrap();
            if !link.open {
                return Err(Error::Closed);
            }
            // Write and queue under one lock, so replies match waiters in order.
            writeln!(link.stdin, "{command}")?;
            link.stdin.flush()?;
            let (tx, rx) = mpsc::channel();
            link.waiters.push_back(tx);
            rx
        };
        wait(rx)
    }

    /// Opens a new window with one pane and returns the pane id, for example `%3`.
    pub fn new_pane(
        &self,
        cwd: &Path,
        argv: &[&str],
        env: &[(&str, &str)],
    ) -> Result<String, Error> {
        let mut c = format!(
            "new-window -d -P -F '#{{pane_id}}' -t {SESSION} -c {}",
            quote(&cwd.to_string_lossy())
        );
        for (k, v) in env {
            c += &format!(" -e {}", quote(&format!("{k}={v}")));
        }
        for a in argv {
            c += " ";
            c += &quote(a);
        }
        first_line(self.cmd(&c)?)
    }

    /// Sends raw bytes to a pane, as if typed.
    pub fn send_bytes(&self, pane: &str, bytes: &[u8]) -> Result<(), Error> {
        for chunk in bytes.chunks(SEND_CHUNK) {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
            self.cmd(&format!(
                "send-keys -t {} -H {}",
                quote(pane),
                hex.join(" ")
            ))?;
        }
        Ok(())
    }

    /// Starts or stops the output stream of a pane. Only the live pane streams.
    ///
    /// This uses `pause` and `continue`, not `off` and `on`. A paused pane
    /// still runs and tmux still reads it, so `capture` shows new text. With
    /// `off`, tmux stops reading the pane, and a busy program blocks.
    pub fn set_streaming(&self, pane: &str, on: bool) -> Result<(), Error> {
        let state = if on { "continue" } else { "pause" };
        self.cmd(&format!(
            "refresh-client -A {}",
            quote(&format!("{pane}:{state}"))
        ))
        .map(drop)
    }

    /// The pane text with colors. `start` is a line number: `-` for the whole history, `-4` for the last 4 lines.
    pub fn capture(&self, pane: &str, start: &str) -> Result<Vec<String>, Error> {
        self.cmd(&format!(
            "capture-pane -p -e -t {} -S {}",
            quote(pane),
            quote(start)
        ))
    }

    /// The cursor position of a pane, as (column, row) from the top left.
    pub fn cursor(&self, pane: &str) -> Result<(u16, u16), Error> {
        let line = first_line(self.cmd(&format!(
            "display -p -t {} '#{{cursor_x}} #{{cursor_y}}'",
            quote(pane)
        ))?)?;
        let mut it = line.split(' ').map(|n| n.parse::<u16>().unwrap_or(0));
        Ok((it.next().unwrap_or(0), it.next().unwrap_or(0)))
    }

    /// Sets the size of the client, and with it the size of the windows.
    pub fn set_size(&self, cols: u16, rows: u16) -> Result<(), Error> {
        self.cmd(&format!(
            "refresh-client -C {}x{}",
            cols.max(2),
            rows.max(2)
        ))
        .map(drop)
    }

    /// Every pane id in the session.
    pub fn panes(&self) -> Result<Vec<String>, Error> {
        self.cmd(&format!("list-panes -s -t {SESSION} -F '#{{pane_id}}'"))
    }

    /// A format string for one pane, for example `#{pane_current_path}`.
    pub fn display(&self, pane: &str, format: &str) -> Result<String, Error> {
        first_line(self.cmd(&format!("display -p -t {} {}", quote(pane), quote(format)))?)
    }

    /// Lines of `list-panes -s -F <format>` for the whole session.
    pub fn list(&self, format: &str) -> Result<Vec<String>, Error> {
        self.cmd(&format!("list-panes -s -t {SESSION} -F {}", quote(format)))
    }

    /// Sets a pane option, for example `@sb_pane` or `remain-on-exit`.
    pub fn set_pane_option(&self, pane: &str, key: &str, value: &str) -> Result<(), Error> {
        self.cmd(&format!(
            "set-option -p -t {} {} {}",
            quote(pane),
            quote(key),
            quote(value)
        ))
        .map(drop)
    }

    /// Starts a new program in a pane, in place of the old one.
    pub fn respawn(
        &self,
        pane: &str,
        cwd: &Path,
        argv: &[&str],
        env: &[(&str, &str)],
    ) -> Result<(), Error> {
        let mut c = format!(
            "respawn-pane -k -t {} -c {}",
            quote(pane),
            quote(&cwd.to_string_lossy())
        );
        for (k, v) in env {
            c += &format!(" -e {}", quote(&format!("{k}={v}")));
        }
        for a in argv {
            c += " ";
            c += &quote(a);
        }
        self.cmd(&c).map(drop)
    }

    pub fn kill_pane(&self, pane: &str) -> Result<(), Error> {
        self.cmd(&format!("kill-pane -t {}", quote(pane))).map(drop)
    }

    pub fn socket(&self) -> &str {
        &self.socket
    }

    /// Stops the whole server and every pane in it. Tests use it; the app never does.
    pub fn kill_server(&self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .status();
        let _ = self.child.lock().unwrap().wait();
    }
}

impl Drop for Tmux {
    fn drop(&mut self) {
        // Detach only. The server and its panes keep running.
        let _ = self.child.lock().unwrap().kill();
    }
}

fn read_loop(out: impl BufRead, link: &Mutex<Link>, on_event: impl Fn(Event)) {
    let mut parser = Parser::default();
    let mut reason = None;
    for line in out.split(b'\n') {
        let Ok(line) = line else { break };
        match parser.feed(&line) {
            Some(Message::Reply { ok, lines }) => {
                let waiter = link.lock().unwrap().waiters.pop_front();
                if let Some(w) = waiter {
                    let _ = w.send(if ok {
                        Ok(lines)
                    } else {
                        Err(Error::Tmux(lines.join("\n")))
                    });
                }
            }
            Some(Message::Output { pane, data }) => on_event(Event::Output { pane, data }),
            Some(Message::WindowAdd(w)) => on_event(Event::WindowAdd(w)),
            Some(Message::WindowClose(w)) => on_event(Event::WindowClose(w)),
            Some(Message::Exit(r)) => {
                reason = r;
                break;
            }
            _ => {}
        }
    }
    let mut l = link.lock().unwrap();
    l.open = false;
    for w in l.waiters.drain(..) {
        let _ = w.send(Err(Error::Closed));
    }
    drop(l);
    on_event(Event::Exit(reason));
}

fn wait(rx: mpsc::Receiver<Reply>) -> Reply {
    match rx.recv_timeout(REPLY_TIMEOUT) {
        Ok(r) => r,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(Error::Timeout),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(Error::Closed),
    }
}

fn first_line(lines: Vec<String>) -> Result<String, Error> {
    lines
        .into_iter()
        .next()
        .ok_or_else(|| Error::Tmux("empty reply".into()))
}

/// Quotes one argument for the tmux command parser.
///
/// Inside single quotes tmux takes every byte as is, except `'`. Close the
/// quote, add an escaped quote, and open it again: `it's` becomes `'it'\''s'`.
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The tmux config for the Switchboard server.
pub const CONF: &str = "\
set -g status off
set -g focus-events on
set -g history-limit 50000
set -g escape-time 0
set -g default-terminal tmux-256color
set -ga terminal-overrides ',*:Tc'
";

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn quote_wraps_and_escapes() {
        assert_eq!(quote("abc"), "'abc'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote("a b;c"), "'a b;c'");
    }
}
