//! The node on a real UDP socket, on its own thread.

use crate::node::{Node, Out, Status};
use crate::proto::{self, PORT};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How long a receive waits before the thread looks at its commands and the clock again: the hotkey's latency.
const POLL: Duration = Duration::from_millis(10);
/// Interfaces come and go (Wi-Fi, VPN): their broadcast addresses are read again this often.
const INTERFACES_EVERY: Duration = Duration::from_secs(5);

pub enum Cmd {
    Send(u8),
}

/// The network thread, from the UI's side. Dropping it stops the thread.
pub struct Net {
    tx: Sender<Cmd>,
    status: Arc<Mutex<Status>>,
}

impl Net {
    pub fn send(&self, kind: u8) {
        let _ = self.tx.send(Cmd::Send(kind));
    }

    pub fn status(&self) -> Status {
        *self.status.lock().unwrap()
    }
}

/// An interface's own broadcast address, from its address and prefix: none for a single-host or odd prefix.
fn directed_broadcast(ip: Ipv4Addr, prefix: u8) -> Option<Ipv4Addr> {
    if ip.is_loopback() || !(8..=30).contains(&prefix) {
        return None;
    }
    let host = u32::MAX >> prefix;
    Some(Ipv4Addr::from(u32::from(ip) | host))
}

/// Where a broadcast goes: everywhere at once, and each interface's own broadcast address, because Windows sends
/// 255.255.255.255 out of one interface only. Worked out from the prefix rather than trusting what each system
/// reports as the broadcast address.
fn broadcast_targets() -> Vec<SocketAddrV4> {
    let mut out = vec![SocketAddrV4::new(Ipv4Addr::BROADCAST, PORT)];
    for i in if_addrs::get_if_addrs().unwrap_or_default() {
        let if_addrs::IfAddr::V4(v4) = i.addr else { continue };
        if let Some(b) = directed_broadcast(v4.ip, v4.prefixlen) {
            let t = SocketAddrV4::new(b, PORT);
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

/// Binds the port and starts the thread. `on_show` runs on that thread for every ping to show.
pub fn start(on_show: impl Fn(u64, bool) + Send + 'static) -> io::Result<Net> {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT))?;
    socket.set_broadcast(true)?;
    socket.set_read_timeout(Some(POLL))?;
    let (tx, rx) = mpsc::channel();
    let status = Arc::new(Mutex::new(Status { is_master: true, online: 1 }));
    let shared = status.clone();
    thread::Builder::new().name("net".into()).spawn(move || run(socket, rx, shared, on_show))?;
    Ok(Net { tx, status })
}

fn run(socket: UdpSocket, rx: Receiver<Cmd>, status: Arc<Mutex<Status>>, on_show: impl Fn(u64, bool)) {
    let mut rng = fastrand::Rng::new();
    let id = rng.u64(..);
    log::info!("node {id:016x} on port {PORT}");
    let mut node = Node::new(id, rng);
    let start = Instant::now();
    let mut targets = broadcast_targets();
    log::info!("broadcasting to {targets:?}");
    let mut targets_at = Instant::now();
    let mut buf = [0u8; proto::MAX_PACKET + 1];
    loop {
        let now = start.elapsed().as_millis() as u64;
        loop {
            match rx.try_recv() {
                Ok(Cmd::Send(kind)) => {
                    if node.send(now, kind) {
                        log::debug!(
                            "sent ({})",
                            if node.status(now).is_master { "as the master" } else { "to the master" }
                        );
                    } else {
                        log::debug!("over the send limit");
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        match socket.recv_from(&mut buf) {
            Ok((n, SocketAddr::V4(src))) => node.receive(start.elapsed().as_millis() as u64, src, &buf[..n]),
            Ok(_) => {}
            // Timeouts, and on Windows a "connection reset" left behind by a unicast to a machine that has gone.
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(e) => log::debug!("receive: {e}"),
        }
        let now = start.elapsed().as_millis() as u64;
        node.tick(now);
        if targets_at.elapsed() >= INTERFACES_EVERY {
            let fresh = broadcast_targets();
            if fresh != targets {
                log::info!("broadcasting to {fresh:?}");
            }
            targets = fresh;
            targets_at = Instant::now();
        }
        for o in node.take() {
            match o {
                Out::Show { origin, own, .. } => on_show(origin, own),
                Out::Unicast(to, data) => {
                    if let Err(e) = socket.send_to(&data, to) {
                        log::debug!("send to {to}: {e}");
                    }
                }
                Out::Broadcast(data) => {
                    for &to in &targets {
                        if let Err(e) = socket.send_to(&data, to) {
                            log::debug!("broadcast to {to}: {e}");
                        }
                    }
                }
            }
        }
        *status.lock().unwrap() = node.status(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn works_out_each_interfaces_broadcast_address() {
        let ip = |s: &str| s.parse::<Ipv4Addr>().unwrap();
        assert_eq!(directed_broadcast(ip("192.168.1.23"), 24), Some(ip("192.168.1.255")));
        assert_eq!(directed_broadcast(ip("10.9.0.1"), 16), Some(ip("10.9.255.255")));
        assert_eq!(directed_broadcast(ip("172.20.5.9"), 20), Some(ip("172.20.15.255")));
        assert_eq!(directed_broadcast(ip("127.0.0.1"), 8), None);
        assert_eq!(directed_broadcast(ip("10.0.0.5"), 32), None);
        assert_eq!(directed_broadcast(ip("10.0.0.5"), 0), None);
    }
}
