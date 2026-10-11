import { createSocket, type Socket } from 'node:dgram';
import { EventEmitter } from 'node:events';
import { networkInterfaces } from 'node:os';
import { DEFAULT_FLOOD_LIMITS, FloodGuard } from './floodGuard';
import { MAX_PACKET, type RoomKeys } from './roomCrypto';
import type { PeerKey, Transport, TransportStatus } from './transport';

export const LAN_PORT = 47474;
/** The fixed multicast group every lolPing on the network joins (administratively scoped, so routers keep it local). */
export const LAN_GROUP = '239.255.74.74';
/** One hop: multicast stays on the local network segment. */
const MULTICAST_TTL = 1;
const INTERFACES_MS = 5000;
/** Longest a closing socket waits for queued packets (a bye) to go out. */
const DRAIN_MS = 250;

/** A socket plus its sends still in flight: closing it earlier would drop them. */
interface Sock {
  socket: Socket;
  pending: number;
  drained: (() => void) | null;
  /** Interfaces whose group membership this socket holds. */
  joined: Set<string>;
  /** Multicast sends waiting their turn: each picks its interface first, so they go out one at a time. */
  queue: { packet: Uint8Array; iface: string | null }[];
  sending: boolean;
}

export interface LanOptions {
  port?: number;
  bindAddress?: string;
  /** Off in tests that shouldn't spray the real network. */
  multicast?: boolean;
  /** The multicast group to join and send to. */
  group?: string;
  /** IPv4 addresses of the interfaces to multicast on; default: every interface that is up. Tests pass loopback. */
  interfaces?: () => string[];
  /** Hear our own multicast: only for tests, where both ends live on one machine. */
  loopback?: boolean;
  /** Lets a test listener share the port. Never in the app: another program could take our unicast. */
  reuseAddr?: boolean;
  /** Addresses that get every broadcast as unicast too. */
  seeds?: { host: string; port: number }[];
  /** Packets per second accepted from one IP before decryption is even tried. */
  floodPerSecond?: number;
  /** Packets per second accepted from all IPs together. */
  floodTotalPerSecond?: number;
}

/**
 * The address of every IPv4 interface that is up. Windows sends multicast out of one interface only (the one with the
 * default route), which on a machine with a VPN or a virtual adapter is often not the LAN: so it goes out of each.
 */
function interfaceAddresses(): string[] {
  const out = new Set<string>();
  for (const list of Object.values(networkInterfaces())) {
    for (const i of list ?? []) if (i.family === 'IPv4' && !i.internal) out.add(i.address);
  }
  return [...out];
}

/**
 * Room packets over UDP on the local network: presence goes to the multicast group LAN_GROUP, everything else straight
 * to the address a peer's packets came from. No connections, so a lost packet or a network change heals itself.
 * Apps before 0.6.0 broadcast instead: their broadcasts still reach this socket, and the unicast answer reaches them.
 */
export class LanTransport extends EventEmitter implements Transport {
  readonly kind = 'lan';
  readonly shared = false;
  status: TransportStatus = 'off';
  private sock: Sock | null = null;
  /** Resolves once the previous socket has released the port. */
  private closing: Promise<void> = Promise.resolve();
  private running = false;
  /** The interfaces to multicast on: empty means the system's default one. */
  private ifaces: string[] = [];
  private refresh: NodeJS.Timeout | null = null;
  private readonly flood: FloodGuard;
  private readonly port: number;
  private readonly group: string;

  constructor(private readonly opts: LanOptions = {}) {
    super();
    this.port = opts.port ?? LAN_PORT;
    this.group = opts.group ?? LAN_GROUP;
    this.flood = new FloodGuard(Date.now, {
      ...DEFAULT_FLOOD_LIMITS,
      perIp: opts.floodPerSecond ?? DEFAULT_FLOOD_LIMITS.perIp,
      total: opts.floodTotalPerSecond ?? DEFAULT_FLOOD_LIMITS.total,
    });
  }

  start(_keys: RoomKeys): void {
    if (this.running) return;
    this.running = true;
    this.readInterfaces();
    this.refresh = setInterval(() => this.readInterfaces(), INTERFACES_MS);
    this.open();
  }

  stop(): void {
    this.running = false;
    if (this.refresh) clearInterval(this.refresh);
    this.refresh = null;
    this.close();
    this.setStatus('off');
  }

  /** Resolves once every socket this transport closed has released the port (and sent what was queued). */
  whenClosed(): Promise<void> {
    return this.closing;
  }

  /** After sleep or a network change: a fresh socket on the same port. */
  rebind(): void {
    if (!this.running) return;
    this.close();
    this.readInterfaces();
    this.open();
  }

  sendTo(peer: PeerKey, packet: Uint8Array): void {
    const m = /^lan:(.+):(\d+)$/.exec(peer);
    if (m) this.send(packet, Number(m[2]), m[1]);
  }

  /** To everyone on the network: the multicast group, out of every interface. */
  broadcast(packet: Uint8Array): void {
    const sock = this.sock;
    if (this.opts.multicast !== false && sock && this.status === 'ok') {
      for (const iface of this.ifaces.length > 0 ? this.ifaces : [null]) {
        sock.pending++;
        sock.queue.push({ packet, iface });
      }
      this.pump(sock);
    }
    for (const s of this.opts.seeds ?? []) this.send(packet, s.port, s.host);
  }

  private send(packet: Uint8Array, port: number, host: string): void {
    const sock = this.sock;
    if (this.status !== 'ok' || !sock) return;
    sock.pending++;
    // Errors (unreachable network, interface gone) arrive in the callback; a lost packet is fine.
    sock.socket.send(packet, port, host, () => this.sent(sock));
  }

  private sent(sock: Sock): void {
    if (--sock.pending === 0) sock.drained?.();
  }

  /**
   * The next multicast send. The outgoing interface is a socket option that a send reads only when it actually goes
   * out, so the next interface is picked in the previous send's callback, never ahead of it.
   */
  private pump(sock: Sock): void {
    if (sock.sending) return;
    const next = sock.queue.shift();
    if (!next) return;
    try {
      if (next.iface) sock.socket.setMulticastInterface(next.iface);
    } catch {
      // The interface just went away: skip it.
      this.sent(sock);
      this.pump(sock);
      return;
    }
    sock.sending = true;
    try {
      sock.socket.send(next.packet, this.port, this.group, () => {
        sock.sending = false;
        this.sent(sock);
        this.pump(sock);
      });
    } catch {
      // Closed after an error: drop what is left.
      sock.sending = false;
      for (let n = sock.queue.length + 1; n > 0; n--) this.sent(sock);
      sock.queue = [];
    }
  }

  private open(): void {
    this.setStatus('starting');
    const socket = createSocket({ type: 'udp4', reuseAddr: this.opts.reuseAddr ?? false });
    const sock: Sock = { socket, pending: 0, drained: null, joined: new Set(), queue: [], sending: false };
    this.sock = sock;
    socket.on('message', (msg, rinfo) => {
      if (msg.length > MAX_PACKET) return;
      if (!this.flood.allow(rinfo.address)) return;
      this.emit('packet', `lan:${rinfo.address}:${rinfo.port}`, msg);
    });
    socket.on('error', () => {
      if (this.sock !== sock) return;
      this.close();
      this.setStatus('unavailable');
    });
    void this.closing.then(() => {
      if (this.sock !== sock) return;
      socket.bind(this.port, this.opts.bindAddress ?? '0.0.0.0', () => {
        if (this.sock !== sock) return;
        if (this.opts.multicast !== false) {
          try {
            socket.setMulticastTTL(MULTICAST_TTL);
            socket.setMulticastLoopback(this.opts.loopback ?? false); // our own packets would only be dropped again
          } catch {
            // Multicast options are best effort: unicast still works.
          }
          this.joinGroup(sock);
        }
        this.setStatus('ok');
      });
    });
  }

  /** Closes the current socket once its queued packets are out, so a bye sent just before still arrives. */
  private close(): void {
    const sock = this.sock;
    this.sock = null;
    if (!sock) return;
    const previous = this.closing;
    const closed = new Promise<void>((resolve) => {
      let done = false;
      const finish = (): void => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        try {
          sock.socket.close(() => resolve());
        } catch {
          resolve(); // already closed after an error
        }
      };
      const timer = setTimeout(finish, DRAIN_MS);
      if (sock.pending === 0) finish();
      else sock.drained = finish;
    });
    // Chained: two quick rebinds must not let a new socket bind while an older one still holds the port.
    this.closing = Promise.all([previous, closed]).then(() => undefined);
  }

  private readInterfaces(): void {
    this.ifaces = this.opts.interfaces?.() ?? interfaceAddresses();
    if (this.sock && this.status === 'ok' && this.opts.multicast !== false) this.joinGroup(this.sock);
  }

  /**
   * Group membership on every interface, kept in step as interfaces come and go (Wi-Fi reconnects, a VPN starts).
   * With no interface up yet it joins on the default one, and once a real one appears it joins there too.
   */
  private joinGroup(sock: Sock): void {
    const want = new Set(this.ifaces.length > 0 ? this.ifaces : ['']);
    for (const iface of sock.joined) {
      if (want.has(iface)) continue;
      sock.joined.delete(iface);
      try {
        if (iface) sock.socket.dropMembership(this.group, iface);
        else sock.socket.dropMembership(this.group);
      } catch {
        // Already gone with its interface.
      }
    }
    for (const iface of want) {
      if (sock.joined.has(iface)) continue;
      try {
        if (iface) sock.socket.addMembership(this.group, iface);
        else sock.socket.addMembership(this.group);
        sock.joined.add(iface);
      } catch {
        // Not ready yet (no address, no multicast on it): the next interface refresh tries again.
      }
    }
  }

  private setStatus(s: TransportStatus): void {
    if (s === this.status) return;
    this.status = s;
    this.emit('status', s);
  }
}
