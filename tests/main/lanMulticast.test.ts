import { createSocket, type Socket } from 'node:dgram';
import { afterEach, describe, expect, it } from 'vitest';
import { LAN_GROUP, LanTransport } from '../../src/main/lanTransport';
import type { RoomKeys } from '../../src/main/roomCrypto';

// Real multicast over loopback: the transport and a plain listener share the port, as two machines would.
const PORT = 47_521;
const KEYS: RoomKeys = { msgKey: Buffer.alloc(32), relayTopic: '' };

async function until(cond: () => boolean, ms = 3000): Promise<void> {
  const end = Date.now() + ms;
  while (!cond()) {
    if (Date.now() > end) throw new Error('timed out');
    await new Promise((r) => setTimeout(r, 20));
  }
}

function listener(): Promise<{ socket: Socket; got: { data: string; port: number }[] }> {
  const socket = createSocket({ type: 'udp4', reuseAddr: true });
  const got: { data: string; port: number }[] = [];
  socket.on('message', (m, r) => got.push({ data: m.toString(), port: r.port }));
  return new Promise((resolve) => socket.bind(PORT, '0.0.0.0', () => {
    socket.addMembership(LAN_GROUP, '127.0.0.1');
    socket.setMulticastInterface('127.0.0.1');
    socket.setMulticastLoopback(true);
    resolve({ socket, got });
  }));
}

let cleanup: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const c of cleanup) await c();
  cleanup = [];
});

function transport(interfaces: string[]): LanTransport {
  const lan = new LanTransport({ port: PORT, interfaces: () => interfaces, reuseAddr: true, loopback: true });
  cleanup.push(() => {
    lan.stop();
    return lan.whenClosed();
  });
  return lan;
}

describe('LAN multicast', () => {
  it('sends presence to the group and hears what others send to it', async () => {
    const l = await listener();
    cleanup.push(() => new Promise((r) => l.socket.close(() => r())));
    const lan = transport(['127.0.0.1']);
    const heard: string[] = [];
    lan.on('packet', (peer: string, p: Uint8Array) => heard.push(`${peer} ${Buffer.from(p).toString()}`));
    lan.start(KEYS);
    await until(() => lan.status === 'ok');

    lan.broadcast(Buffer.from('hello group'));
    await until(() => l.got.some((g) => g.data === 'hello group'));
    expect(l.got.find((g) => g.data === 'hello group')!.port).toBe(PORT); // answers come back to the room socket

    l.socket.send('from a peer', PORT, LAN_GROUP);
    await until(() => heard.some((h) => h.endsWith('from a peer')));
    expect(heard.find((h) => h.endsWith('from a peer'))).toMatch(/^lan:127\.0\.0\.1:47521 /);
  });

  it('skips an interface it cannot send on, still reaches the others, and closes cleanly', async () => {
    const l = await listener();
    cleanup.push(() => new Promise((r) => l.socket.close(() => r())));
    // 192.0.2.1 is a documentation address no machine has: picking it as the outgoing interface fails.
    const lan = transport(['192.0.2.1', '127.0.0.1']);
    lan.start(KEYS);
    await until(() => lan.status === 'ok');
    for (let i = 0; i < 3; i++) lan.broadcast(Buffer.from(`n${i}`));
    await until(() => ['n0', 'n1', 'n2'].every((n) => l.got.some((g) => g.data === n)));
    lan.stop();
    await lan.whenClosed();
    expect(lan.status).toBe('off');
  });

  it('sends nothing to the group with multicast off', async () => {
    const l = await listener();
    cleanup.push(() => new Promise((r) => l.socket.close(() => r())));
    const lan = new LanTransport({ port: PORT, interfaces: () => ['127.0.0.1'], reuseAddr: true, loopback: true, multicast: false });
    cleanup.push(() => {
      lan.stop();
      return lan.whenClosed();
    });
    lan.start(KEYS);
    await until(() => lan.status === 'ok');
    lan.broadcast(Buffer.from('quiet'));
    await new Promise((r) => setTimeout(r, 150));
    expect(l.got).toEqual([]);
  });
});
