//! One lolPing Lite on the network, as a state machine: packets and the clock go in, packets to send and pings to
//! show come out. No sockets here, so the tests run whole networks in memory.
//!
//! Everyone broadcasts a hello every second. The master is whoever holds the lowest lottery ticket
//! (`proto::rank`, a hash of the node id and the shared seed) among the members a node knows: every node works it
//! out alone and they agree, because they know the same members. A ping goes to the master, which acks it and
//! passes it on to every member by unicast (and one broadcast); with no master, or no ack in time, the sender
//! broadcasts it itself. Every ping carries its origin and a random id, so whichever way it arrives it shows once.

use crate::limiter::{self, Limiter};
use crate::proto::{self, Body, Listed, Packet};
use std::collections::HashMap;
use std::net::SocketAddrV4;

pub const HELLO_MS: u64 = 1000;
/// A member not heard from for this long is gone (three hellos and a half missed).
pub const EXPIRE_MS: u64 = 3500;
/// How long a sender waits for the master's ack before broadcasting the ping itself.
pub const ACK_MS: u64 = 300;
/// How long a ping's id is remembered, so a late copy is still recognised.
const SEEN_MS: u64 = 30_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    Unicast(SocketAddrV4, Vec<u8>),
    Broadcast(Vec<u8>),
    /// Show a ping: `own` when this node sent it.
    Show {
        origin: u64,
        kind: u8,
        own: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub is_master: bool,
    /// Members known, this node included.
    pub online: usize,
}

#[derive(Debug, Clone, Copy)]
struct Member {
    addr: SocketAddrV4,
    last: u64,
    rank: u64,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    origin: u64,
    msg: u64,
    kind: u8,
    deadline: u64,
}

pub struct Node {
    id: u64,
    rank: u64,
    rng: fastrand::Rng,
    /// Members heard directly, at the address their packets come from.
    peers: HashMap<u64, Member>,
    /// Members only known from another member's hello: they may not reach us, but they count, and may be the master.
    listed: HashMap<u64, Member>,
    seen: HashMap<(u64, u64), u64>,
    limits: HashMap<u64, Limiter>,
    send_limit: Limiter,
    pending: Vec<Pending>,
    next_hello: u64,
    last_master: u64,
    out: Vec<Out>,
}

impl Node {
    pub fn new(id: u64, rng: fastrand::Rng) -> Self {
        Self {
            id,
            rank: proto::rank(id),
            rng,
            peers: HashMap::new(),
            listed: HashMap::new(),
            seen: HashMap::new(),
            limits: HashMap::new(),
            send_limit: Limiter::new(limiter::SEND),
            pending: Vec::new(),
            next_hello: 0,
            last_master: id,
            out: Vec::new(),
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// What to send and show since the last call.
    pub fn take(&mut self) -> Vec<Out> {
        std::mem::take(&mut self.out)
    }

    fn alive(m: &Member, now: u64) -> bool {
        now < m.last + EXPIRE_MS
    }

    /// Every member we know of and where to reach it: heard directly first, else as listed by someone who did.
    fn members(&self, now: u64) -> HashMap<u64, Member> {
        let mut all: HashMap<u64, Member> =
            self.listed.iter().filter(|(_, m)| Self::alive(m, now)).map(|(&id, &m)| (id, m)).collect();
        all.extend(self.peers.iter().filter(|(_, m)| Self::alive(m, now)).map(|(&id, &m)| (id, m)));
        all.remove(&self.id);
        all
    }

    /// The master's id, and its address unless it is us.
    fn master(&self, now: u64) -> (u64, Option<SocketAddrV4>) {
        self.members(now)
            .into_iter()
            .filter(|(id, m)| (m.rank, *id) < (self.rank, self.id))
            .min_by_key(|(id, m)| (m.rank, *id))
            .map_or((self.id, None), |(id, m)| (id, Some(m.addr)))
    }

    pub fn status(&self, now: u64) -> Status {
        Status { is_master: self.master(now).0 == self.id, online: self.members(now).len() + 1 }
    }

    /// The hotkey: false (and nothing sent) when over the limit.
    pub fn send(&mut self, now: u64, kind: u8) -> bool {
        if !self.send_limit.allow(now) {
            return false;
        }
        let msg = self.rng.u64(..);
        self.seen.insert((self.id, msg), now);
        self.out.push(Out::Show { origin: self.id, kind, own: true });
        match self.master(now) {
            (_, Some(addr)) => {
                let p = Packet { from: self.id, body: Body::Ping { origin: self.id, msg, kind } };
                self.out.push(Out::Unicast(addr, proto::encode(&p)));
                self.pending.push(Pending { origin: self.id, msg, kind, deadline: now + ACK_MS });
            }
            // We are the master, or alone.
            _ => self.relay(now, self.id, msg, kind, &[]),
        }
        true
    }

    /// To every member but the ones in `skip`, and once to the whole network for anyone we don't know yet.
    fn relay(&mut self, now: u64, origin: u64, msg: u64, kind: u8, skip: &[SocketAddrV4]) {
        let p = proto::encode(&Packet { from: self.id, body: Body::Relay { origin, msg, kind } });
        for (id, m) in self.members(now) {
            if id != origin && !skip.contains(&m.addr) {
                self.out.push(Out::Unicast(m.addr, p.clone()));
            }
        }
        self.out.push(Out::Broadcast(p));
    }

    /// A ping we haven't seen, from an origin within its limit: shown (and true). Anything else: false.
    fn accept(&mut self, now: u64, origin: u64, msg: u64, kind: u8) -> bool {
        if self.seen.insert((origin, msg), now).is_some() || origin == self.id {
            return false;
        }
        if !self.limits.entry(origin).or_insert_with(|| Limiter::new(limiter::RECEIVE)).allow(now) {
            log::debug!("{origin:016x} is over the limit: dropped");
            return false;
        }
        self.out.push(Out::Show { origin, kind, own: false });
        true
    }

    pub fn receive(&mut self, now: u64, src: SocketAddrV4, data: &[u8]) {
        let Some(Packet { from, body }) = proto::decode(data) else { return };
        if from == self.id {
            return; // our own broadcast, back
        }
        let rank = self.peers.get(&from).map_or_else(|| proto::rank(from), |m| m.rank);
        self.peers.insert(from, Member { addr: src, last: now, rank });
        match body {
            Body::Hello { members } => {
                // A listed member is as alive as the lister last heard it: once nobody hears it, it expires everywhere
                // when it would have expired for them.
                for Listed { id, addr, age_ms } in members {
                    if id == self.id {
                        continue;
                    }
                    let last = now.saturating_sub(age_ms as u64);
                    match self.listed.get_mut(&id) {
                        Some(m) if m.last >= last => {}
                        Some(m) => *m = Member { addr, last, rank: m.rank },
                        None => {
                            self.listed.insert(id, Member { addr, last, rank: proto::rank(id) });
                        }
                    }
                }
            }
            Body::Ping { origin, msg, kind } => {
                let ack = proto::encode(&Packet { from: self.id, body: Body::Ack { msg } });
                self.out.push(Out::Unicast(src, ack));
                // Whoever gets a ping passes it on, master or not: two nodes that briefly disagree on the master
                // still deliver it.
                if self.accept(now, origin, msg, kind) {
                    self.relay(now, origin, msg, kind, &[src]);
                }
            }
            Body::Relay { origin, msg, kind } => {
                self.accept(now, origin, msg, kind);
            }
            Body::Ack { msg } => self.pending.retain(|p| p.msg != msg),
        }
    }

    pub fn tick(&mut self, now: u64) {
        let (late, waiting): (Vec<_>, Vec<_>) = self.pending.drain(..).partition(|p| now >= p.deadline);
        self.pending = waiting;
        for p in late {
            log::debug!("no ack from the master: broadcasting {:016x}", p.msg);
            let b = proto::encode(&Packet {
                from: self.id,
                body: Body::Relay { origin: p.origin, msg: p.msg, kind: p.kind },
            });
            self.out.push(Out::Broadcast(b));
        }

        if now >= self.next_hello {
            self.next_hello = now + HELLO_MS;
            self.hello(now);
            self.peers.retain(|_, m| Self::alive(m, now));
            self.listed.retain(|_, m| Self::alive(m, now));
            self.seen.retain(|_, &mut t| now < t + SEEN_MS);
            self.limits.retain(|_, l| !l.idle(now));
            let (master, _) = self.master(now);
            if master != self.last_master {
                self.last_master = master;
                log::info!("master is now {master:016x}{}", if master == self.id { " (this machine)" } else { "" });
            }
        }
    }

    fn hello(&mut self, now: u64) {
        let mut direct: Vec<(u64, Member)> =
            self.peers.iter().filter(|(_, m)| Self::alive(m, now)).map(|(&id, &m)| (id, m)).collect();
        direct.sort_by_key(|(_, m)| std::cmp::Reverse(m.last));
        let members = direct
            .iter()
            .take(proto::MAX_LISTED)
            .map(|&(id, m)| Listed { id, addr: m.addr, age_ms: (now - m.last).min(u16::MAX as u64) as u16 })
            .collect();
        let p = proto::encode(&Packet { from: self.id, body: Body::Hello { members } });
        match self.master(now) {
            // Straight to the master too, in case our broadcasts don't reach it.
            (_, Some(addr)) => self.out.push(Out::Unicast(addr, p.clone())),
            // As the master, to everyone, in case theirs don't reach us.
            (_, None) => {
                for (_, m) in self.members(now) {
                    self.out.push(Out::Unicast(m.addr, p.clone()));
                }
            }
        }
        self.out.push(Out::Broadcast(p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::net::Ipv4Addr;

    /// Nodes on an imaginary LAN. `deaf` pairs (a, b): broadcasts from a never reach b. Unicast always arrives.
    struct Lan {
        nodes: Vec<Node>,
        up: Vec<bool>,
        deaf: HashSet<(usize, usize)>,
        shown: Vec<Vec<(u64, bool)>>,
        broadcasts: usize,
        now: u64,
    }

    fn addr(i: usize) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, i as u8 + 1), proto::PORT)
    }

    impl Lan {
        fn new(ids: &[u64]) -> Self {
            let nodes: Vec<Node> = ids.iter().map(|&id| Node::new(id, fastrand::Rng::with_seed(id))).collect();
            let n = nodes.len();
            Self { nodes, up: vec![true; n], deaf: HashSet::new(), shown: vec![vec![]; n], broadcasts: 0, now: 1_000 }
        }

        /// Delivers everything queued, until the network is quiet.
        fn flush(&mut self) {
            loop {
                let mut moved = false;
                for i in 0..self.nodes.len() {
                    if !self.up[i] {
                        self.nodes[i].take();
                        continue;
                    }
                    for o in self.nodes[i].take() {
                        moved = true;
                        match o {
                            Out::Show { origin, own, .. } => self.shown[i].push((origin, own)),
                            Out::Unicast(to, data) => {
                                let j = (to.ip().octets()[3] - 1) as usize;
                                if self.up[j] {
                                    self.nodes[j].receive(self.now, addr(i), &data);
                                }
                            }
                            Out::Broadcast(data) => {
                                self.broadcasts += 1;
                                for j in 0..self.nodes.len() {
                                    if j != i && self.up[j] && !self.deaf.contains(&(i, j)) {
                                        self.nodes[j].receive(self.now, addr(i), &data);
                                    }
                                }
                            }
                        }
                    }
                }
                if !moved {
                    return;
                }
            }
        }

        /// Lets `ms` pass in 50 ms steps.
        fn run(&mut self, ms: u64) {
            let end = self.now + ms;
            while self.now < end {
                self.now += 50;
                for i in 0..self.nodes.len() {
                    if self.up[i] {
                        self.nodes[i].tick(self.now);
                    }
                }
                self.flush();
            }
        }

        fn status(&self, i: usize) -> Status {
            self.nodes[i].status(self.now)
        }

        fn masters(&self) -> Vec<u64> {
            (0..self.nodes.len()).filter(|&i| self.up[i]).map(|i| self.nodes[i].master(self.now).0).collect()
        }

        fn send(&mut self, i: usize) -> bool {
            let ok = self.nodes[i].send(self.now, proto::KIND_MISSING);
            self.flush();
            ok
        }

        fn clear(&mut self) {
            self.shown.iter_mut().for_each(Vec::clear);
            self.broadcasts = 0;
        }
    }

    /// The id among `ids` that wins the lottery.
    fn winner(ids: &[u64]) -> u64 {
        *ids.iter().min_by_key(|&&id| (proto::rank(id), id)).unwrap()
    }

    const IDS: [u64; 4] = [11, 22, 33, 44];

    #[test]
    fn everyone_agrees_on_the_master_and_the_count() {
        let mut lan = Lan::new(&IDS);
        lan.run(2_000);
        let w = winner(&IDS);
        assert_eq!(lan.masters(), vec![w; 4]);
        for (i, &id) in IDS.iter().enumerate() {
            assert_eq!(lan.status(i), Status { is_master: id == w, online: 4 });
        }
    }

    #[test]
    fn alone_is_master_of_one() {
        let mut lan = Lan::new(&[5]);
        lan.run(1_500);
        assert_eq!(lan.status(0), Status { is_master: true, online: 1 });
        assert!(lan.send(0));
        assert_eq!(lan.shown[0], vec![(5, true)]);
    }

    #[test]
    fn a_ping_goes_through_the_master_and_shows_once_everywhere() {
        let mut lan = Lan::new(&IDS);
        lan.run(2_000);
        let m = IDS.iter().position(|&id| id == winner(&IDS)).unwrap();
        let s = (m + 1) % 4;
        lan.clear();
        assert!(lan.send(s));
        for i in 0..4 {
            let want = if i == s { vec![(IDS[s], true)] } else { vec![(IDS[s], false)] };
            assert_eq!(lan.shown[i], want, "node {i}");
        }
        // Acked: nothing waits for a fallback broadcast.
        assert!(lan.nodes[s].pending.is_empty());
        lan.run(400);
        lan.shown.iter().for_each(|v| assert_eq!(v.len(), 1));
    }

    #[test]
    fn the_master_reaches_members_whose_broadcasts_dont_meet() {
        // 0 and 3 never hear each other's broadcasts; only the master links them.
        let ids = IDS;
        let m = ids.iter().position(|&id| id == winner(&ids)).unwrap();
        let others: Vec<usize> = (0..4).filter(|&i| i != m).collect();
        let (a, b) = (others[0], others[1]);
        let mut lan = Lan::new(&ids);
        lan.deaf.insert((a, b));
        lan.deaf.insert((b, a));
        lan.run(3_000);
        assert_eq!(lan.masters(), vec![ids[m]; 4]);
        lan.clear();
        lan.send(a);
        assert_eq!(lan.shown[b], vec![(ids[a], false)]);
        assert_eq!(lan.status(b).online, 4);
    }

    #[test]
    fn a_member_that_only_the_master_hears_still_gets_everything() {
        // Node c hears no broadcast at all, and nobody hears its: it only knows the master through others' hellos.
        let m = IDS.iter().position(|&id| id == winner(&IDS)).unwrap();
        let c = (m + 2) % 4;
        let mut lan = Lan::new(&IDS);
        for j in 0..4 {
            if j != c {
                lan.deaf.insert((j, c));
                lan.deaf.insert((c, j));
            }
        }
        // c learns of the master once (say, from an old hello), then everything goes by unicast.
        let hello = proto::encode(&Packet { from: IDS[m], body: Body::Hello { members: vec![] } });
        lan.nodes[c].receive(lan.now, addr(m), &hello);
        lan.run(3_000);
        assert_eq!(lan.masters(), vec![IDS[m]; 4]);
        assert_eq!(lan.status(c).online, 4);
        let s = (c + 1) % 4;
        lan.clear();
        lan.send(s);
        assert_eq!(lan.shown[c], vec![(IDS[s], false)]);
        lan.clear();
        lan.send(c);
        for i in (0..4).filter(|&i| i != c) {
            assert_eq!(lan.shown[i], vec![(IDS[c], false)], "node {i}");
        }
    }

    #[test]
    fn a_new_master_takes_over_when_the_old_one_goes() {
        let mut lan = Lan::new(&IDS);
        lan.run(2_000);
        let m = IDS.iter().position(|&id| id == winner(&IDS)).unwrap();
        lan.up[m] = false;
        lan.run(EXPIRE_MS + 1_500);
        let rest: Vec<u64> = IDS.iter().copied().filter(|&id| id != IDS[m]).collect();
        assert_eq!(lan.masters(), vec![winner(&rest); 3]);
        for i in (0..4).filter(|&i| i != m) {
            assert_eq!(lan.status(i).online, 3);
        }
        let s = (m + 1) % 4;
        lan.clear();
        lan.send(s);
        for i in (0..4).filter(|&i| i != m && i != s) {
            assert_eq!(lan.shown[i], vec![(IDS[s], false)]);
        }
    }

    #[test]
    fn with_the_master_silent_the_sender_broadcasts_after_the_ack_timeout() {
        let mut lan = Lan::new(&IDS);
        lan.run(2_000);
        let m = IDS.iter().position(|&id| id == winner(&IDS)).unwrap();
        lan.up[m] = false; // just died: the others still think it's the master
        let s = (m + 1) % 4;
        lan.clear();
        lan.send(s);
        assert!(lan.shown.iter().enumerate().all(|(i, v)| i == s || v.is_empty()));
        lan.run(ACK_MS + 100);
        for i in (0..4).filter(|&i| i != m && i != s) {
            assert_eq!(lan.shown[i], vec![(IDS[s], false)]);
        }
    }

    #[test]
    fn the_sender_is_limited_to_one_per_100ms_and_twenty_per_5s() {
        let mut lan = Lan::new(&IDS);
        lan.run(2_000);
        lan.clear();
        let mut sent = 0;
        for _ in 0..80 {
            if lan.send(1) {
                sent += 1;
            }
            lan.run(50);
        }
        assert_eq!(sent, 20); // 4 s of hammering: twenty went out
        assert_eq!(lan.shown[2].len(), 20);
    }

    #[test]
    fn receivers_drop_a_flood_from_a_client_that_ignores_the_limit() {
        let mut lan = Lan::new(&IDS);
        lan.run(2_000);
        lan.clear();
        // A modified client: forty relays at once, straight to everyone.
        for msg in 0..40 {
            let p = proto::encode(&Packet { from: 99, body: Body::Relay { origin: 99, msg, kind: 0 } });
            for j in 0..4 {
                lan.nodes[j].receive(lan.now + msg * 60, addr(9), &p);
            }
        }
        lan.flush();
        for j in 0..4 {
            assert_eq!(lan.shown[j].len(), 20, "node {j}");
        }
    }

    #[test]
    fn duplicates_and_own_echoes_are_ignored() {
        let mut n = Node::new(1, fastrand::Rng::with_seed(1));
        let p = proto::encode(&Packet { from: 2, body: Body::Relay { origin: 2, msg: 7, kind: 0 } });
        n.receive(0, addr(1), &p);
        n.receive(500, addr(1), &p);
        let echo = proto::encode(&Packet { from: 3, body: Body::Relay { origin: 1, msg: 8, kind: 0 } });
        n.receive(600, addr(2), &echo);
        let mine = proto::encode(&Packet { from: 1, body: Body::Hello { members: vec![] } });
        n.receive(700, addr(0), &mine);
        let shows: Vec<_> = n.take().into_iter().filter(|o| matches!(o, Out::Show { .. })).collect();
        assert_eq!(shows, vec![Out::Show { origin: 2, kind: 0, own: false }]);
        assert!(!n.peers.contains_key(&1));
    }

    #[test]
    fn listed_members_fade_once_nobody_lists_them() {
        let mut lan = Lan::new(&IDS);
        lan.deaf.insert((3, 0));
        lan.deaf.insert((0, 3));
        lan.run(2_000);
        assert_eq!(lan.status(0).online, 4);
        lan.up[3] = false;
        lan.run(2 * EXPIRE_MS + 1_000);
        assert_eq!(lan.status(0).online, 3);
    }
}
