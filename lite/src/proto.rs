//! The wire format. Every datagram is: magic, version, type, the sender's node id, the body, then a tag.
//! The tag is a hash keyed with a seed every copy of lolPing Lite shares: it is not secrecy (the seed is in the
//! program), just a cheap way to drop anything on the port that isn't ours.

use sha2::{Digest, Sha256};
use std::net::{Ipv4Addr, SocketAddrV4};

pub const PORT: u16 = 47475;
const MAGIC: [u8; 2] = *b"LQ";
const VERSION: u8 = 1;
/// Shared by every copy: it keys the packet tags and the master lottery.
const SEED: &[u8] = b"lolping-lite/v1";
const TAG_LEN: usize = 8;
const HEADER_LEN: usize = 2 + 1 + 1 + 8;
/// A hello lists at most this many members, so it always fits one datagram.
pub const MAX_LISTED: usize = 64;
/// Bigger than any packet we send; anything longer is not ours.
pub const MAX_PACKET: usize = 1200;

/// The only message there is: League's "?" (Enemy Missing).
pub const KIND_MISSING: u8 = 0;

const T_HELLO: u8 = 1;
const T_PING: u8 = 2;
const T_RELAY: u8 = 3;
const T_ACK: u8 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// "I'm here", every second, with the members the sender hears directly.
    Hello {
        members: Vec<Listed>,
    },
    /// "Please pass this on to everyone": to the master, which answers with an Ack.
    Ping {
        origin: u64,
        msg: u64,
        kind: u8,
    },
    /// "Show this": from the master, or broadcast by the sender itself when no master answers.
    Relay {
        origin: u64,
        msg: u64,
        kind: u8,
    },
    Ack {
        msg: u64,
    },
}

/// A member in a hello: who, where the sender hears it from, and how long ago it last did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listed {
    pub id: u64,
    pub addr: SocketAddrV4,
    pub age_ms: u16,
}

const LISTED_LEN: usize = 8 + 4 + 2 + 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    /// The node that sent this datagram (for a relay: the relaying node, not the origin).
    pub from: u64,
    pub body: Body,
}

fn tag(data: &[u8]) -> [u8; TAG_LEN] {
    let mut h = Sha256::new();
    h.update(SEED);
    h.update(b"/tag/");
    h.update(data);
    let d = h.finalize();
    let mut t = [0; TAG_LEN];
    t.copy_from_slice(&d[..TAG_LEN]);
    t
}

/// A node's ticket in the master lottery: the lowest one wins. Every node computes the same tickets from the ids.
pub fn rank(id: u64) -> u64 {
    let mut h = Sha256::new();
    h.update(SEED);
    h.update(b"/rank/");
    h.update(id.to_le_bytes());
    let d = h.finalize();
    u64::from_le_bytes(d[..8].try_into().unwrap())
}

pub fn encode(p: &Packet) -> Vec<u8> {
    let mut b = Vec::with_capacity(64);
    b.extend_from_slice(&MAGIC);
    b.push(VERSION);
    let (t, rest): (u8, Vec<u8>) = match &p.body {
        Body::Hello { members } => {
            let n = members.len().min(MAX_LISTED);
            let mut r = Vec::with_capacity(1 + n * LISTED_LEN);
            r.push(n as u8);
            for m in &members[..n] {
                r.extend_from_slice(&m.id.to_le_bytes());
                r.extend_from_slice(&m.addr.ip().octets());
                r.extend_from_slice(&m.addr.port().to_le_bytes());
                r.extend_from_slice(&m.age_ms.to_le_bytes());
            }
            (T_HELLO, r)
        }
        Body::Ping { origin, msg, kind } | Body::Relay { origin, msg, kind } => {
            let mut r = Vec::with_capacity(17);
            r.extend_from_slice(&origin.to_le_bytes());
            r.extend_from_slice(&msg.to_le_bytes());
            r.push(*kind);
            (if matches!(p.body, Body::Ping { .. }) { T_PING } else { T_RELAY }, r)
        }
        Body::Ack { msg } => (T_ACK, msg.to_le_bytes().to_vec()),
    };
    b.push(t);
    b.extend_from_slice(&p.from.to_le_bytes());
    b.extend_from_slice(&rest);
    let t = tag(&b);
    b.extend_from_slice(&t);
    b
}

fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
}

/// None for anything that isn't a well-formed packet of ours with a good tag.
pub fn decode(buf: &[u8]) -> Option<Packet> {
    if buf.len() < HEADER_LEN + TAG_LEN || buf.len() > MAX_PACKET {
        return None;
    }
    let (data, t) = buf.split_at(buf.len() - TAG_LEN);
    if data[..2] != MAGIC || data[2] != VERSION || tag(data) != t {
        return None;
    }
    let from = u64_at(data, 4)?;
    let rest = &data[HEADER_LEN..];
    let body = match data[3] {
        T_HELLO => {
            let n = *rest.first()? as usize;
            if n > MAX_LISTED || rest.len() != 1 + n * LISTED_LEN {
                return None;
            }
            let members = rest[1..]
                .as_chunks::<LISTED_LEN>()
                .0
                .iter()
                .map(|e| Listed {
                    id: u64::from_le_bytes(e[..8].try_into().unwrap()),
                    addr: SocketAddrV4::new(
                        Ipv4Addr::new(e[8], e[9], e[10], e[11]),
                        u16::from_le_bytes([e[12], e[13]]),
                    ),
                    age_ms: u16::from_le_bytes([e[14], e[15]]),
                })
                .collect();
            Body::Hello { members }
        }
        t @ (T_PING | T_RELAY) => {
            if rest.len() != 17 {
                return None;
            }
            let (origin, msg, kind) = (u64_at(rest, 0)?, u64_at(rest, 8)?, rest[16]);
            if t == T_PING { Body::Ping { origin, msg, kind } } else { Body::Relay { origin, msg, kind } }
        }
        T_ACK => {
            if rest.len() != 8 {
                return None;
            }
            Body::Ack { msg: u64_at(rest, 0)? }
        }
        _ => return None,
    };
    Some(Packet { from, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(last: u8, port: u16) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, last), port)
    }

    #[test]
    fn round_trips_every_body() {
        let bodies = [
            Body::Hello { members: vec![] },
            Body::Hello {
                members: vec![
                    Listed { id: 7, addr: addr(2, PORT), age_ms: 0 },
                    Listed { id: u64::MAX, addr: addr(254, 1), age_ms: u16::MAX },
                ],
            },
            Body::Ping { origin: 1, msg: 2, kind: KIND_MISSING },
            Body::Relay { origin: u64::MAX, msg: 0, kind: 9 },
            Body::Ack { msg: 0xdead_beef },
        ];
        for body in bodies {
            let p = Packet { from: 42, body };
            assert_eq!(decode(&encode(&p)), Some(p));
        }
    }

    #[test]
    fn caps_the_member_list() {
        let members = (0..100).map(|i| Listed { id: i, addr: addr(i as u8, PORT), age_ms: 500 }).collect();
        let b = encode(&Packet { from: 1, body: Body::Hello { members } });
        assert!(b.len() <= MAX_PACKET);
        let Some(Packet { body: Body::Hello { members }, .. }) = decode(&b) else { panic!() };
        assert_eq!(members.len(), MAX_LISTED);
    }

    #[test]
    fn drops_anything_tampered_with_or_foreign() {
        let good = encode(&Packet { from: 5, body: Body::Ping { origin: 5, msg: 6, kind: 0 } });
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 1;
            assert_eq!(decode(&bad), None, "byte {i} flipped");
        }
        assert_eq!(decode(&good[..good.len() - 1]), None);
        assert_eq!(decode(b""), None);
        assert_eq!(decode(&[0u8; 40]), None);
    }

    #[test]
    fn ranks_are_stable_and_spread() {
        assert_eq!(rank(1), rank(1));
        assert_ne!(rank(1), rank(2));
        // Small ids don't always win: the lottery isn't the id order.
        let wins = (0..200u64).filter(|i| rank(i * 2) < rank(i * 2 + 1)).count();
        assert!((60..140).contains(&wins), "{wins}");
    }
}
