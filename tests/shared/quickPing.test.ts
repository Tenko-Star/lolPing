import { describe, expect, it } from 'vitest';
import { DONUT, donutDensity, donutPoint, QUICK_PING } from '../../src/shared/quickPing';

/** A small seeded generator, so the statistics below are the same on every run. */
function mulberry32(seed: number): () => number {
  let a = seed;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const span = 0.5 - DONUT.margin;
/** Elliptical radius of a point: 0 in the middle, 1 at the middle of an edge (inside the margin). */
const radius = (p: { x: number; y: number }): number => Math.hypot((p.x - 0.5) / span, (p.y - 0.5) / span);

describe('quick "?" ping', () => {
  it('is the "?" ping', () => {
    expect(QUICK_PING).toBe('missing');
  });

  it('is a donut: low in the middle, highest on the ring, low at the edges and lower still in the corners', () => {
    expect(donutDensity(0, 0)).toBeLessThan(0.01);
    expect(donutDensity(DONUT.ring, 0)).toBe(1);
    expect(donutDensity(0, -DONUT.ring)).toBe(1);
    expect(donutDensity(1, 0)).toBeLessThan(0.05);
    expect(donutDensity(1, 1)).toBeLessThan(donutDensity(1, 0));
  });

  it('draws most points near the ring, few in the middle or at the edges, all inside the margins', () => {
    const random = mulberry32(42);
    const n = 20_000;
    const points = Array.from({ length: n }, () => donutPoint(random));
    for (const p of points) {
      expect(p.x).toBeGreaterThanOrEqual(DONUT.margin);
      expect(p.x).toBeLessThanOrEqual(1 - DONUT.margin);
      expect(p.y).toBeGreaterThanOrEqual(DONUT.margin);
      expect(p.y).toBeLessThanOrEqual(1 - DONUT.margin);
    }
    const share = (lo: number, hi: number): number => points.filter((p) => radius(p) >= lo && radius(p) < hi).length / n;
    expect(share(0, 0.2)).toBeLessThan(0.02);
    expect(share(0.35, 0.75)).toBeGreaterThan(0.75);
    expect(share(0.9, 2)).toBeLessThan(0.05);
    // Per unit of area, the ring is far denser than the middle or the edges.
    const density = (lo: number, hi: number): number => share(lo, hi) / (Math.PI * (hi * hi - lo * lo));
    expect(density(0.5, 0.6)).toBeGreaterThan(20 * density(0, 0.2));
    expect(density(0.5, 0.6)).toBeGreaterThan(10 * density(0.9, 1));
    // Every direction gets its share: left, right, top and bottom of the ring.
    const sides = [points.filter((p) => p.x < 0.5).length, points.filter((p) => p.y < 0.5).length].map((c) => c / n);
    for (const s of sides) expect(Math.abs(s - 0.5)).toBeLessThan(0.02);
  });

  it('still returns a spot on the ring if every try is refused', () => {
    expect(donutPoint(() => 0.999)).toEqual({ x: 0.5 + DONUT.ring * span, y: 0.5 });
  });
});
