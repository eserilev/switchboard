//! The phone's end of a hub link (SPEC 41.4, 41.5): pairing from the text of the
//! QR code that `sb pair` shows, and a window link over the Noise session.
//!
//! The link is a [`Client`] as the desktop's ssh link is: two threads carry its
//! lines through the session, so every command and event works the same way.

use crate::remote::{Client, News};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use session::{KeyPair, Opener, Sealer};
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const DIAL: Duration = Duration::from_secs(10);

/// What the QR code holds: `SWITCHBOARD1 <hub key> <secret> <pairing address> <hub address>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Qr {
    pub hub_key: Vec<u8>,
    pub secret: [u8; 32],
    pub pair_at: String,
    pub hub_at: String,
}

pub fn parse_qr(text: &str) -> Result<Qr, String> {
    let bad = || "This is not a Switchboard pairing code. Run sb pair on the computer and scan the code it shows.".to_owned();
    let f: Vec<&str> = text.split_whitespace().collect();
    if f.len() != 5 || f[0] != "SWITCHBOARD1" {
        return Err(bad());
    }
    let hub_key = session::unhex(f[1]).filter(|k| k.len() == session::KEY).ok_or_else(bad)?;
    let secret: [u8; 32] = session::unhex(f[2]).and_then(|s| s.try_into().ok()).ok_or_else(bad)?;
    for a in [f[3], f[4]] {
        a.parse::<SocketAddr>().map_err(|_| bad())?;
    }
    Ok(Qr { hub_key, secret, pair_at: f[3].to_owned(), hub_at: f[4].to_owned() })
}

/// This phone's pairing with one hub, kept on the phone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Paired {
    /// The id the hub gave this phone.
    pub id: String,
    pub hub_key: String,
    pub hub_at: String,
    /// This phone's session key pair, in hex.
    pub private: String,
    pub public: String,
}

impl Paired {
    pub fn key(&self) -> Result<KeyPair, String> {
        let k = |h: &str| session::unhex(h).filter(|k| k.len() == session::KEY).ok_or("the saved key is broken");
        Ok(KeyPair { private: k(&self.private)?, public: k(&self.public)? })
    }

    /// Reads the pairing file. `None` when there is none.
    pub fn load(path: &Path) -> Option<Paired> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }

    /// Writes the pairing file, readable only by this app: it holds the private key.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("tmp");
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
        f.write_all(serde_json::to_string(self).map_err(|e| e.to_string())?.as_bytes()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }
}

fn dial(at: &str) -> Result<TcpStream, String> {
    let addr = at.to_socket_addrs().map_err(|e| format!("{at}: {e}"))?.next().ok_or(format!("{at}: no address"))?;
    let tcp = TcpStream::connect_timeout(&addr, DIAL).map_err(|e| format!("can not reach {at}: {e}"))?;
    let _ = tcp.set_nodelay(true);
    Ok(tcp)
}

/// A pairing that waits for Allow on the computer. Both screens show [`PairWait::code`].
pub struct PairWait {
    pub code: String,
    tcp: TcpStream,
    open: Opener,
    qr: Qr,
    me: KeyPair,
}

/// Starts pairing: the handshake with the secret of the QR code, then this phone's
/// name. The hub's key must be the one of the QR code, or the handshake fails.
pub fn pair_start(qr: &Qr, name: &str, platform: &str) -> Result<PairWait, String> {
    let me = session::keypair().map_err(|e| e.to_string())?;
    let tcp = dial(&qr.pair_at)?;
    tcp.set_read_timeout(Some(DIAL)).map_err(|e| e.to_string())?;
    let (code, mut seal, open): (String, Sealer, _) =
        session::pair_connect(&mut &tcp, &me, &qr.hub_key, &qr.secret).map_err(|e| format!("the pairing handshake failed: {e}"))?;
    seal.send(&mut &tcp, json!({ "name": name, "platform": platform }).to_string().as_bytes()).map_err(|e| e.to_string())?;
    Ok(PairWait { code, tcp, open, qr: qr.clone(), me })
}

impl PairWait {
    /// Waits for your answer on the computer, at most `time`.
    pub fn wait(mut self, time: Duration) -> Result<Paired, String> {
        self.tcp.set_read_timeout(Some(time)).map_err(|e| e.to_string())?;
        let frame = self.open.recv(&mut &self.tcp, 4096).map_err(|_| "The computer said no, or the time ran out. Nothing was paired.".to_owned())?;
        let v: Value = serde_json::from_slice(&frame).map_err(|_| "the computer's answer does not parse")?;
        let id = v["id"].as_str().filter(|_| v["ok"] == true).ok_or("the computer refused this phone")?;
        Ok(Paired {
            id: id.to_owned(),
            hub_key: session::hex(&self.qr.hub_key),
            hub_at: self.qr.hub_at.clone(),
            private: session::hex(&self.me.private),
            public: session::hex(&self.me.public),
        })
    }
}

/// A window link to the hub of `p`, over a new session. `news` gets the events and
/// the end of the link, as with the desktop's ssh link.
pub fn connect(p: &Paired, instance: &str, since: Option<(u64, u64)>, news: impl Fn(News) + Send + 'static) -> Result<Arc<Client>, String> {
    let me = p.key()?;
    let hub_key = session::unhex(&p.hub_key).ok_or("the saved hub key is broken")?;
    let tcp = dial(&p.hub_at)?;
    tcp.set_read_timeout(Some(DIAL)).map_err(|e| e.to_string())?;
    let (mut seal, mut open) = session::connect(&mut &tcp, &me, &p.id, &hub_key).map_err(|e| format!("the hub did not accept this phone ({e}). It may have been removed: pair it again."))?;
    tcp.set_read_timeout(None).map_err(|e| e.to_string())?;
    let (mine, theirs) = UnixStream::pair().map_err(|e| e.to_string())?;
    // Up: each line the client writes is one frame.
    {
        let (tcp, theirs) = (tcp.try_clone().map_err(|e| e.to_string())?, theirs.try_clone().map_err(|e| e.to_string())?);
        std::thread::spawn(move || {
            let mut r = BufReader::new(&theirs);
            let mut line = vec![];
            loop {
                line.clear();
                match r.read_until(b'\n', &mut line) {
                    Ok(n) if n > 0 && line.len() <= session::MAX_COMMAND => {}
                    _ => break,
                }
                if line.last() == Some(&b'\n') {
                    line.pop();
                }
                if seal.send(&mut &tcp, &line).is_err() {
                    break;
                }
            }
            let _ = tcp.shutdown(Shutdown::Both);
        });
    }
    // Down: each frame of the hub is one line for the client.
    {
        let (tcp, theirs) = (tcp.try_clone().map_err(|e| e.to_string())?, theirs);
        std::thread::spawn(move || {
            let mut w = &theirs;
            while let Ok(mut frame) = open.recv(&mut &tcp, session::MAX_NEWS) {
                frame.push(b'\n');
                if w.write_all(&frame).is_err() {
                    break;
                }
            }
            let _ = tcp.shutdown(Shutdown::Both);
            let _ = theirs.shutdown(Shutdown::Both);
        });
    }
    let input = mine.try_clone().map_err(|e| e.to_string())?;
    Client::over_streams(Box::new(input), mine, instance, since, news)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_text() {
        let k = "ab".repeat(32);
        let q = parse_qr(&format!("SWITCHBOARD1 {k} {k} 100.1.2.3:41000 100.1.2.3:7800")).unwrap();
        assert_eq!(q.hub_at, "100.1.2.3:7800");
        assert_eq!(q.secret, [0xab; 32]);
        assert!(parse_qr("SWITCHBOARD1 zz").is_err());
        assert!(parse_qr(&format!("SWITCHBOARD1 {k} {k} nowhere 100.1.2.3:7800")).is_err());
    }
}
