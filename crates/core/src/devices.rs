//! Device windows over the network (SPEC 41.4, 41.5): the hub's key, and a listener
//! for the Noise sessions of listed devices. Off by default (SPEC 41.13).
//!
//! A session ends in a socket pair: one end is a window as any other, with the
//! device's rules, and two threads carry its lines to and from the network.

use crate::hub::Hub;
use crate::paths::Paths;
use crate::remote::{self, Device, RemoteSink};
use session::KeyPair;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

/// Links that have not ended their handshake, at most.
const MAX_OPENING: usize = 16;
/// The time for the handshake and the hello.
const OPENING: Duration = Duration::from_secs(10);
/// A session lives this long. The device then makes a new handshake, with new keys.
const LIFE: Duration = Duration::from_secs(3600);

/// The hub's key pair. The first call makes it, in a file that only you can read.
pub fn hub_key(paths: &Paths) -> Result<KeyPair, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = paths.device_key();
    let e = |err: std::io::Error| format!("{}: {err}", path.display());
    let new = session::keypair().map_err(e)?;
    match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
        Ok(mut f) => {
            f.write_all(&[new.private.clone(), new.public.clone()].concat()).and_then(|()| f.sync_all()).map_err(e)?;
            Ok(new)
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let b = std::fs::read(&path).map_err(e)?;
            if b.len() != 2 * session::KEY {
                return Err(format!("{}: not a key", path.display()));
            }
            let (private, public) = b.split_at(session::KEY);
            Ok(KeyPair { private: private.to_vec(), public: public.to_vec() })
        }
        Err(err) => Err(e(err)),
    }
}

/// The address to listen on: one address of this computer, never all of them.
pub fn address(listen: &str) -> Result<SocketAddr, String> {
    let a: SocketAddr = listen.parse().map_err(|_| format!("devices.listen = {listen:?} is not <ip>:<port>"))?;
    let all = match a.ip() {
        IpAddr::V4(i) => i.is_unspecified() || i.is_broadcast() || i.is_multicast(),
        IpAddr::V6(i) => i.is_unspecified() || i.is_multicast() || i.to_ipv4_mapped().is_some_and(|m| m.is_unspecified()),
    };
    if all {
        return Err(format!("devices.listen = {listen:?} must name one address of this computer"));
    }
    Ok(a)
}

/// Closes the sessions of removed devices. `sb devices remove` writes only the
/// store, so the hub looks there.
fn watch(hub: Weak<Hub>, sink: Arc<RemoteSink>) {
    while let Some(h) = hub.upgrade() {
        sink.end_unlisted(&h);
        drop(h);
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Listens for devices until the hub goes. The address can come later (Tailscale
/// starts after the hub): it tries again.
pub fn serve(listen: &str, hub: Weak<Hub>, sink: Arc<RemoteSink>) -> Result<(), String> {
    let addr = address(listen)?;
    let key = {
        let h = hub.upgrade().ok_or("the hub is gone")?;
        Arc::new(hub_key(&h.paths)?)
    };
    {
        let (hub, sink) = (hub.clone(), Arc::clone(&sink));
        std::thread::spawn(move || watch(hub, sink));
    }
    std::thread::spawn(move || {
        let opening = Arc::new(AtomicUsize::new(0));
        while hub.strong_count() > 0 {
            match TcpListener::bind(addr) {
                Ok(l) => {
                    tracing::info!(target: "sb::devices", %addr, "listens for devices");
                    accept_all(&l, &hub, &sink, &key, &opening);
                }
                Err(err) => tracing::warn!(target: "sb::devices", %addr, %err, "can not listen yet"),
            }
            std::thread::sleep(Duration::from_secs(30));
        }
    });
    Ok(())
}

/// One link that is still opening.
struct Opening(Arc<AtomicUsize>);

impl Drop for Opening {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn accept_all(l: &TcpListener, hub: &Weak<Hub>, sink: &Arc<RemoteSink>, key: &Arc<KeyPair>, opening: &Arc<AtomicUsize>) {
    for s in l.incoming() {
        let s = match s {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(target: "sb::devices", %err, "accept");
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
        };
        let Some(hub) = hub.upgrade() else { return };
        let was = opening.fetch_add(1, Ordering::SeqCst);
        let slot = Opening(Arc::clone(opening));
        if was >= MAX_OPENING {
            continue;
        }
        let (sink, key) = (Arc::clone(sink), Arc::clone(key));
        std::thread::spawn(move || {
            if let Err(err) = link(hub, sink, s, &key, slot) {
                tracing::info!(target: "sb::devices", %err, "a device link ended");
            }
        });
    }
}

/// A stream that fails past a time, however slowly its bytes come.
pub(crate) struct Until<'a>(pub &'a TcpStream, pub Instant);

impl Read for Until<'_> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let left = self.1.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.0.set_read_timeout(Some(left))?;
        self.0.read(b)
    }
}

impl Write for Until<'_> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

fn link(hub: Arc<Hub>, sink: Arc<RemoteSink>, tcp: TcpStream, key: &KeyPair, slot: Opening) -> std::io::Result<()> {
    let start = Instant::now();
    let _ = tcp.set_nodelay(true);
    tcp.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut s = Until(&tcp, start + OPENING);
    let mut device = None;
    let (id, mut seal, mut open) = session::accept(&mut s, key, |id, k| {
        device = hub.device_of_key(k).filter(|d| d.id == id && d.revoked_at.is_none());
        device.is_some()
    })?;
    let Some(d) = device else { return Ok(()) };
    // The window registers only after this opens: a replayed message 1 gets no window.
    let hello = open.recv(&mut s, 64 << 10)?;
    drop(slot);
    tracing::info!(target: "sb::devices", device = %id, "a device session starts");
    hub.device_seen(&id);

    let (mine, theirs) = UnixStream::pair()?;
    let who = Device { id, generation: d.generation, classes: d.classes };
    std::thread::spawn(move || remote::window(hub, sink, theirs, Some(who)));
    let (out, down) = (tcp.try_clone()?, mine.try_clone()?);
    std::thread::spawn(move || {
        let mut w = &out;
        let mut r = BufReader::new(&down);
        let mut line = vec![];
        loop {
            line.clear();
            match r.read_until(b'\n', &mut line) {
                Ok(n) if n > 0 && line.len() <= session::MAX_NEWS => {}
                _ => break,
            }
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            if seal.send(&mut w, &line).is_err() {
                break;
            }
        }
        let _ = out.shutdown(Shutdown::Both);
        let _ = down.shutdown(Shutdown::Both);
    });
    let up = || -> std::io::Result<()> {
        let mut s = Until(&tcp, start + LIFE);
        let mut w = &mine;
        let mut frame = hello;
        loop {
            // One frame is one line: no second command hides in it.
            if frame.contains(&b'\n') {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            frame.push(b'\n');
            w.write_all(&frame)?;
            frame = open.recv(&mut s, session::MAX_COMMAND)?;
        }
    };
    let r = up();
    let _ = tcp.shutdown(Shutdown::Both);
    let _ = mine.shutdown(Shutdown::Both);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_named_address() {
        for ok in ["100.64.0.1:7800", "127.0.0.1:1", "[fd7a::1]:7800"] {
            assert!(address(ok).is_ok(), "{ok}");
        }
        for no in ["", "0.0.0.0:1", "[::]:1", "[::ffff:0.0.0.0]:1", "255.255.255.255:1", "224.0.0.1:1", "box:1", "100.64.0.1"] {
            assert!(address(no).is_err(), "{no}");
        }
    }

    #[test]
    fn the_hub_key_stays_and_only_you_read_it() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("sb-hubkey-{}", std::process::id()));
        let paths = Paths::under(root.clone());
        std::fs::create_dir_all(&paths.data).unwrap();
        let a = hub_key(&paths).unwrap();
        let b = hub_key(&paths).unwrap();
        assert!(a.private == b.private && a.public == b.public);
        let mode = std::fs::metadata(paths.device_key()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::write(paths.device_key(), b"short").unwrap();
        assert!(hub_key(&paths).is_err(), "a broken key is never replaced");
        let _ = std::fs::remove_dir_all(root);
    }
}
