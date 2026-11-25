//! Pairing a phone by QR code (SPEC 41.4).
//!
//! `sb pair` runs all of it: it makes the secret, listens for one phone, asks you
//! on the terminal, and writes the store. No socket command pairs a key, so an
//! agent that reaches the hub's sockets can not.

use crate::store::{Device, Store};
use serde_json::{json, Value};
use session::{KeyPair, Opener, Sealer};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// Pairing mode lasts this long.
pub const OPEN_FOR: Duration = Duration::from_secs(600);
const HANDSHAKE: Duration = Duration::from_secs(10);
const MAX_OPENING: usize = 16;
const MAX_HELLO: usize = 4096;

/// A phone that proved it read the QR code, and waits for your answer.
pub struct Asking {
    pub name: String,
    pub platform: String,
    /// Both screens show it.
    pub code: String,
    key: Vec<u8>,
    push_key: Option<Vec<u8>>,
    tcp: TcpStream,
    seal: Sealer,
}

pub struct Pairing {
    listener: TcpListener,
    secret: [u8; 32],
}

/// Text from the phone for your terminal: no control characters, and short.
fn plain(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_graphic() || *c == ' ' || (c.is_alphanumeric() && !c.is_ascii())).take(40).collect::<String>().trim().to_owned()
}

impl Pairing {
    /// Starts pairing mode on an address of this computer, with any free port.
    pub fn open(ip: std::net::IpAddr) -> Result<Pairing, String> {
        let listener = TcpListener::bind((ip, 0)).map_err(|e| format!("{ip}: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(Pairing { listener, secret: session::secret().map_err(|e| e.to_string())? })
    }

    /// The text of the QR code. `hub` is where the sessions go later.
    pub fn qr(&self, hub: SocketAddr, hub_key: &[u8]) -> Result<String, String> {
        let at = self.listener.local_addr().map_err(|e| e.to_string())?;
        Ok(format!("SWITCHBOARD1 {} {} {at} {hub}", session::hex(hub_key), session::hex(&self.secret)))
    }

    /// Waits for the first phone that proves the secret. The secret is then used
    /// up: pairing mode ends with this call. `None` when the time ends first.
    pub fn wait(self, hub: KeyPair, time: Duration) -> Option<Asking> {
        let end = Instant::now() + time;
        let (hub, secret) = (Arc::new(hub), self.secret);
        let (tx, rx) = mpsc::channel();
        let used = Arc::new(AtomicBool::new(false));
        let opening = Arc::new(AtomicUsize::new(0));
        while Instant::now() < end {
            if let Ok(a) = rx.try_recv() {
                return Some(a);
            }
            match self.listener.accept() {
                Ok((tcp, _)) => {
                    if opening.load(Ordering::SeqCst) >= MAX_OPENING {
                        continue;
                    }
                    opening.fetch_add(1, Ordering::SeqCst);
                    let (hub, tx, used, opening) = (Arc::clone(&hub), tx.clone(), Arc::clone(&used), Arc::clone(&opening));
                    std::thread::spawn(move || {
                        match ask(tcp, &hub, &secret, &used) {
                            Ok(a) => drop(tx.send(a)),
                            Err(err) => tracing::info!(target: "sb::devices", %err, "a pairing link ended"),
                        }
                        opening.fetch_sub(1, Ordering::SeqCst);
                    });
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
        used.store(true, Ordering::SeqCst);
        rx.try_recv().ok()
    }
}

fn ask(tcp: TcpStream, hub: &KeyPair, secret: &[u8; 32], used: &AtomicBool) -> std::io::Result<Asking> {
    let bad = |why: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, why);
    tcp.set_nonblocking(false)?;
    tcp.set_write_timeout(Some(HANDSHAKE))?;
    let mut s = crate::devices::Until(&tcp, Instant::now() + HANDSHAKE);
    let (key, code, seal, mut open) = session::pair_accept(&mut s, hub, secret)?;
    // Only a phone with the secret can make a frame that opens.
    let frame = open.recv(&mut s, MAX_HELLO)?;
    if used.swap(true, Ordering::SeqCst) {
        return Err(bad("the code is used up"));
    }
    let v: Value = serde_json::from_slice(&frame).map_err(|_| bad("not a pairing request"))?;
    let push_key = match v["push_key"].as_str() {
        Some(h) => Some(session::unhex(h).filter(|k| k.len() == session::KEY).ok_or_else(|| bad("a bad push key"))?),
        None => None,
    };
    let text = |k: &str| plain(v[k].as_str().unwrap_or(""));
    Ok(Asking { name: text("name"), platform: text("platform"), code, key, push_key, tcp, seal })
}

impl Asking {
    /// You said Allow: the key goes on the list, and the phone gets its id.
    pub fn allow(mut self, store: &Store, name: &str, now: u64) -> Result<String, String> {
        let id = format!("d{}", session::hex(&session::secret().map_err(|e| e.to_string())?[..8]));
        store
            .device_add(&Device {
                id: id.clone(),
                name: name.to_owned(),
                platform: self.platform.clone(),
                session_key: self.key.clone(),
                push_key: self.push_key.take(),
                classes: scope_check::PHONE,
                paired_at: now,
                last_seen: None,
                revoked_at: None,
                generation: 0,
            })
            .map_err(|e| format!("this phone's key is on the list already, or was removed: {e}"))?;
        // A phone that went away meanwhile does not learn its id: undo, so no key stays that nobody uses.
        let sent = self.seal.send(&mut &self.tcp, json!({ "ok": true, "id": id }).to_string().as_bytes());
        if let Err(err) = sent {
            let _ = store.device_revoke(&id, now);
            return Err(format!("the phone went away ({err}). Make a new key on the phone and pair again."));
        }
        Ok(id)
    }

    /// You said no. The link closes, and the key is nowhere.
    pub fn refuse(mut self) {
        let _ = self.seal.send(&mut &self.tcp, json!({ "ok": false }).to_string().as_bytes());
    }
}

/// The phone's end: reads the QR text, proves the secret, and gives the check code
/// to show, with the link to wait on for the answer ([`answer`]).
pub fn phone(qr: &str, me: &KeyPair, hello: &Value) -> Result<(String, TcpStream, Opener), String> {
    let parts: Vec<&str> = qr.split(' ').collect();
    let [tag, hub, secret, at, _] = parts[..] else { return Err("not a Switchboard code".into()) };
    if tag != "SWITCHBOARD1" {
        return Err("not a Switchboard code".into());
    }
    let hub = session::unhex(hub).filter(|k| k.len() == session::KEY).ok_or("a bad hub key")?;
    let secret: [u8; 32] = session::unhex(secret).and_then(|s| s.try_into().ok()).ok_or("a bad secret")?;
    let at: SocketAddr = at.parse().map_err(|_| "a bad address")?;
    let e = |e: std::io::Error| e.to_string();
    let mut tcp = TcpStream::connect_timeout(&at, HANDSHAKE).map_err(e)?;
    tcp.set_read_timeout(Some(OPEN_FOR)).map_err(e)?;
    let (code, mut seal, open) = session::pair_connect(&mut tcp, me, &hub, &secret).map_err(e)?;
    seal.send(&mut tcp, hello.to_string().as_bytes()).map_err(e)?;
    Ok((code, tcp, open))
}

/// Waits for Allow on the computer, and gives the phone its device id.
pub fn answer(tcp: &mut TcpStream, open: &mut Opener) -> Result<String, String> {
    let f = open.recv(tcp, MAX_HELLO).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&f).map_err(|e| e.to_string())?;
    match v["id"].as_str() {
        Some(id) if v["ok"] == true => Ok(id.to_owned()),
        _ => Err("refused on the computer".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HUB: &str = "100.64.0.1:7800";

    fn start(time: Duration) -> (String, std::thread::JoinHandle<Option<Asking>>) {
        let hub = session::keypair().unwrap();
        let p = Pairing::open("127.0.0.1".parse().unwrap()).unwrap();
        let qr = p.qr(HUB.parse().unwrap(), &hub.public).unwrap();
        (qr, std::thread::spawn(move || p.wait(hub, time)))
    }

    fn hello() -> Value {
        json!({ "name": "my\u{1b}[2J phone", "platform": "ios" })
    }

    #[test]
    fn allow_puts_the_key_on_the_list() {
        let (qr, wait) = start(Duration::from_secs(10));
        let me = session::keypair().unwrap();
        let (code, mut tcp, mut open) = phone(&qr, &me, &hello()).unwrap();
        let asking = wait.join().unwrap().unwrap();
        assert_eq!(asking.code, code);
        assert_eq!(code.len(), 6);
        assert_eq!(asking.name, "my[2J phone", "no escape reaches the terminal");
        let s = Store::memory().unwrap();
        assert!(s.devices().unwrap().is_empty(), "nothing before Allow");
        let id = asking.allow(&s, "Phone", 5).unwrap();
        assert_eq!(answer(&mut tcp, &mut open).unwrap(), id);
        let d = s.device_by_key(&me.public).unwrap().unwrap();
        assert_eq!((d.id, d.name.as_str(), d.classes), (id, "Phone", scope_check::PHONE));
    }

    #[test]
    fn a_no_lists_nothing() {
        let (qr, wait) = start(Duration::from_secs(10));
        let (_, mut tcp, mut open) = phone(&qr, &session::keypair().unwrap(), &hello()).unwrap();
        wait.join().unwrap().unwrap().refuse();
        assert!(answer(&mut tcp, &mut open).unwrap_err().contains("refused"));
    }

    /// A wrong secret proves nothing and does not use up the right one.
    #[test]
    fn a_wrong_secret_does_not_pair_or_spend_the_code() {
        let (qr, wait) = start(Duration::from_secs(10));
        let mut p: Vec<String> = qr.split(' ').map(str::to_owned).collect();
        p[2] = "00".repeat(32);
        assert!(phone(&p.join(" "), &session::keypair().unwrap(), &hello()).is_err());
        let me = session::keypair().unwrap();
        let (code, _tcp, _open) = phone(&qr, &me, &hello()).unwrap();
        let asking = wait.join().unwrap().unwrap();
        assert_eq!((asking.code, asking.key), (code, me.public));
    }

    #[test]
    fn the_code_works_once() {
        let (qr, wait) = start(Duration::from_secs(10));
        let one = phone(&qr, &session::keypair().unwrap(), &hello());
        assert!(wait.join().unwrap().is_some());
        let two = phone(&qr, &session::keypair().unwrap(), &hello());
        assert!(one.is_ok() && two.is_err());
    }

    #[test]
    fn a_removed_key_never_pairs_again() {
        let s = Store::memory().unwrap();
        let me = session::keypair().unwrap();
        for turn in 0..2 {
            let (qr, wait) = start(Duration::from_secs(10));
            let (_, _tcp, _open) = phone(&qr, &me, &hello()).unwrap();
            let r = wait.join().unwrap().unwrap().allow(&s, "x", 1);
            assert_eq!(r.is_ok(), turn == 0);
            if let Ok(id) = r {
                s.device_revoke(&id, 2).unwrap();
            }
        }
    }

    #[test]
    fn pairing_mode_ends() {
        let (qr, wait) = start(Duration::from_millis(100));
        assert!(wait.join().unwrap().is_none());
        assert!(phone(&qr, &session::keypair().unwrap(), &hello()).is_err());
    }
}
