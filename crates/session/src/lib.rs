//! The session between a device and a hub (SPEC 41.5): Noise `IK`, then frames in
//! encrypted chunks. The phone and the hub both use this crate.
//!
//! On the wire, each message is `len u16` (big-endian), then `len` bytes. The first
//! message is the device id in the clear, the next two are the handshake, and every
//! later one is an encrypted chunk: one byte "more", then the data.
//!
//! The nonces of a session start at 0 and never wrap: a session ends after
//! [`MAX_CHUNKS`] chunks one way, and a new handshake gives new keys.

#![forbid(unsafe_code)]

use snow::{Builder, HandshakeState, StatelessTransportState};
use std::io::{self, Read, Write};
use std::sync::Arc;

const PATTERN: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
/// The data in one chunk: a Noise message, less the tag and the "more" byte.
pub const CHUNK: usize = 65535 - 16 - 1;
pub const MAX_CHUNKS: u64 = 1 << 20;
/// The largest command of a device, and the largest frame of a hub.
pub const MAX_COMMAND: usize = 1 << 20;
pub const MAX_NEWS: usize = 40 << 20;
pub const KEY: usize = 32;
/// The longest device id, in bytes.
const MAX_ID: usize = 64;

pub struct KeyPair {
    pub private: Vec<u8>,
    pub public: Vec<u8>,
}

fn bad(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

fn builder<'a>() -> io::Result<Builder<'a>> {
    Ok(Builder::new(PATTERN.parse().map_err(bad)?))
}

pub fn keypair() -> io::Result<KeyPair> {
    let k = builder()?.generate_keypair().map_err(bad)?;
    Ok(KeyPair { private: k.private, public: k.public })
}

/// `"switchboard/1" ‖ hub key ‖ len of the device id ‖ device id ‖ role`. The hub
/// key is its id until the relay gives one. The length keeps the fields apart.
fn prologue(hub: &[u8], device: &str) -> Vec<u8> {
    let mut p = b"switchboard/1".to_vec();
    p.extend(hub);
    p.push(device.len() as u8);
    p.extend(device.as_bytes());
    p.extend(b"device");
    p
}

fn write_msg(w: &mut impl Write, m: &[u8]) -> io::Result<()> {
    let len = u16::try_from(m.len()).map_err(bad)?;
    let mut out = Vec::with_capacity(m.len() + 2);
    out.extend(len.to_be_bytes());
    out.extend(m);
    w.write_all(&out)?;
    w.flush()
}

fn read_msg(r: &mut impl Read, max: usize) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 2];
    r.read_exact(&mut len)?;
    let n = u16::from_be_bytes(len) as usize;
    if n > max {
        return Err(bad("the message is too long"));
    }
    let mut m = vec![0; n];
    r.read_exact(&mut m)?;
    Ok(m)
}

fn finish(hs: HandshakeState) -> io::Result<(Sealer, Opener)> {
    let t = Arc::new(hs.into_stateless_transport_mode().map_err(bad)?);
    Ok((Sealer { t: Arc::clone(&t), n: 0 }, Opener { t, n: 0 }))
}

/// The hub's end. `listed` gets the device id and its key, before the hub answers:
/// with `false`, the other end gets no answer. Nothing runs yet after this: a
/// replayed first message comes this far, and the first frame then does not open.
pub fn accept(
    s: &mut (impl Read + Write),
    hub: &KeyPair,
    listed: impl FnOnce(&str, &[u8]) -> bool,
) -> io::Result<(String, Sealer, Opener)> {
    let id = String::from_utf8(read_msg(s, MAX_ID)?).map_err(bad)?;
    let p = prologue(&hub.public, &id);
    let mut hs = builder()?
        .local_private_key(&hub.private)
        .map_err(bad)?
        .prologue(&p)
        .map_err(bad)?
        .build_responder()
        .map_err(bad)?;
    let mut buf = [0u8; 256];
    let m1 = read_msg(s, 256)?;
    // Message 1 carries no data.
    if hs.read_message(&m1, &mut buf).map_err(bad)? != 0 {
        return Err(bad("message 1 carries data"));
    }
    let key = hs.get_remote_static().ok_or_else(|| bad("no device key"))?.to_vec();
    if !listed(&id, &key) {
        return Err(bad("not a listed device"));
    }
    let n = hs.write_message(&[], &mut buf).map_err(bad)?;
    write_msg(s, &buf[..n])?;
    let (seal, open) = finish(hs)?;
    Ok((id, seal, open))
}

/// The device's end. `hub` is the hub's public key, from the pairing.
pub fn connect(s: &mut (impl Read + Write), me: &KeyPair, id: &str, hub: &[u8]) -> io::Result<(Sealer, Opener)> {
    if id.len() > MAX_ID {
        return Err(bad("the device id is too long"));
    }
    let p = prologue(hub, id);
    let mut hs = builder()?
        .local_private_key(&me.private)
        .map_err(bad)?
        .remote_public_key(hub)
        .map_err(bad)?
        .prologue(&p)
        .map_err(bad)?
        .build_initiator()
        .map_err(bad)?;
    let mut buf = [0u8; 256];
    let n = hs.write_message(&[], &mut buf).map_err(bad)?;
    write_msg(s, id.as_bytes())?;
    write_msg(s, &buf[..n])?;
    let m2 = read_msg(s, 256)?;
    if hs.read_message(&m2, &mut buf).map_err(bad)? != 0 {
        return Err(bad("message 2 carries data"));
    }
    finish(hs)
}

/// Encrypts the frames of one direction.
pub struct Sealer {
    t: Arc<StatelessTransportState>,
    n: u64,
}

impl Sealer {
    pub fn send(&mut self, w: &mut impl Write, frame: &[u8]) -> io::Result<()> {
        let mut buf = vec![0u8; 65535];
        let mut from = 0;
        loop {
            if self.n >= MAX_CHUNKS {
                return Err(bad("the session is used up"));
            }
            let to = frame.len().min(from + CHUNK);
            let more = to < frame.len();
            let mut plain = Vec::with_capacity(to - from + 1);
            plain.push(more as u8);
            plain.extend(&frame[from..to]);
            let n = self.t.write_message(self.n, &plain, &mut buf).map_err(bad)?;
            self.n += 1;
            write_msg(w, &buf[..n])?;
            from = to;
            if !more {
                return Ok(());
            }
        }
    }
}

/// Decrypts the frames of one direction.
pub struct Opener {
    t: Arc<StatelessTransportState>,
    n: u64,
}

impl Opener {
    /// The next frame. One longer than `max` ends the session.
    pub fn recv(&mut self, r: &mut impl Read, max: usize) -> io::Result<Vec<u8>> {
        let mut out: Vec<u8> = vec![];
        let mut buf = vec![0u8; 65535];
        loop {
            if self.n >= MAX_CHUNKS {
                return Err(bad("the session is used up"));
            }
            let m = read_msg(r, 65535)?;
            let n = self.t.read_message(self.n, &m, &mut buf).map_err(bad)?;
            self.n += 1;
            let Some((more, data)) = buf[..n].split_first() else {
                return Err(bad("an empty chunk"));
            };
            if *more > 1 || (*more == 1 && data.is_empty()) {
                return Err(bad("a bad chunk"));
            }
            if out.len() + data.len() > max {
                return Err(bad("the frame is too long"));
            }
            out.extend(data);
            if *more == 0 {
                return Ok(out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    type Ends = ((UnixStream, Sealer, Opener), (UnixStream, Sealer, Opener));

    fn pair() -> Ends {
        let (hub, me) = (keypair().unwrap(), keypair().unwrap());
        let (mut a, mut b) = UnixStream::pair().unwrap();
        let (pk, hk) = (me.public.clone(), hub.public.clone());
        let t = std::thread::spawn(move || {
            let (id, s, o) = accept(&mut a, &hub, |id, k| id == "d1" && k == pk).unwrap();
            assert_eq!(id, "d1");
            (a, s, o)
        });
        let (s, o) = connect(&mut b, &me, "d1", &hk).unwrap();
        (t.join().unwrap(), (b, s, o))
    }

    #[test]
    fn frames_of_every_size_go_both_ways() {
        let ((mut a, mut hs, mut ho), (mut b, mut ds, mut d_o)) = pair();
        for len in [0, 1, CHUNK - 1, CHUNK, CHUNK + 1, 2 * CHUNK, 300_000] {
            let f: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let g = f.clone();
            let t = std::thread::spawn(move || {
                ds.send(&mut b, &g).unwrap();
                (b, ds)
            });
            assert!(ho.recv(&mut a, MAX_COMMAND).unwrap() == f);
            (b, ds) = t.join().unwrap();
            let g = f.clone();
            let t = std::thread::spawn(move || {
                hs.send(&mut a, &g).unwrap();
                (a, hs)
            });
            assert!(d_o.recv(&mut b, MAX_NEWS).unwrap() == f);
            (a, hs) = t.join().unwrap();
        }
    }

    #[test]
    fn a_frame_above_the_maximum_ends_it() {
        let ((mut a, _, mut ho), (mut b, mut ds, _)) = pair();
        ds.send(&mut b, &[7; 101]).unwrap();
        assert!(ho.recv(&mut a, 100).is_err());
    }

    /// A chunk that changed, came twice or came in another order does not open.
    #[test]
    fn a_changed_or_replayed_chunk_is_refused() {
        for how in 0..3 {
            let ((_a, _, mut ho), (_b, mut ds, _)) = pair();
            let (mut one, mut two) = (vec![], vec![]);
            ds.send(&mut one, b"one").unwrap();
            ds.send(&mut two, b"two").unwrap();
            match how {
                0 => {
                    let last = one.len() - 1;
                    one[last] ^= 1;
                    assert!(ho.recv(&mut &one[..], 100).is_err());
                }
                1 => {
                    assert_eq!(ho.recv(&mut &one[..], 100).unwrap(), b"one");
                    assert!(ho.recv(&mut &one[..], 100).is_err());
                }
                _ => assert!(ho.recv(&mut &two[..], 100).is_err()),
            }
        }
    }

    fn refused(id: &str, listed: bool, right_hub: bool, change_id: bool) {
        let (hub, other, me) = (keypair().unwrap(), keypair().unwrap(), keypair().unwrap());
        let hk = if right_hub { hub.public.clone() } else { other.public };
        let (mut a, mut b) = UnixStream::pair().unwrap();
        let t = std::thread::spawn(move || accept(&mut a, &hub, |_, _| listed).map(|_| ()));
        let r = if change_id {
            // The clear id on the wire differs from the one in the prologue.
            let mut sent = vec![];
            struct Tap<'a>(&'a mut Vec<u8>);
            impl Read for Tap<'_> {
                fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                    Ok(0)
                }
            }
            impl Write for Tap<'_> {
                fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                    self.0.extend(b);
                    Ok(b.len())
                }
                fn flush(&mut self) -> io::Result<()> {
                    Ok(())
                }
            }
            let _ = connect(&mut Tap(&mut sent), &me, id, &hk);
            sent[2] ^= 1;
            b.write_all(&sent).unwrap();
            read_msg(&mut b, 256).map(|_| ())
        } else {
            connect(&mut b, &me, id, &hk).map(|_| ())
        };
        assert!(t.join().unwrap().is_err());
        assert!(r.is_err(), "the device got an answer");
    }

    #[test]
    fn a_stranger_gets_no_answer() {
        refused("d1", false, true, false);
        refused("d1", true, false, false);
        refused("d1", true, true, true);
    }
}
