import type { PingId } from './pings';

/** The quick ping is League's "?" (Enemy Missing). */
export const QUICK_PING: PingId = 'missing';

/**
 * Where quick pings land, in "elliptical radius": 0 is the middle of the display, 1 the middle of an edge. The display's
 * own aspect ratio stretches the circle into an ellipse, so the ring hugs the screen's shape.
 * Density is a Gaussian ring, steeper outside than in: highest at `ring`, ~1 % of that at the edges and less in the
 * corners, and inside it falls off slowly, so only a small hole (about 5 % of the display around its middle) stays quiet.
 */
export const DONUT = {
  ring: 0.55,
  /** How fast the density falls off towards the middle. */
  inner: 0.2,
  /** How fast it falls off towards the edges. */
  outer: 0.15,
  /** Kept clear at each edge (fraction of the display), so a ping is never cut in half. */
  margin: 0.05,
} as const;

/** Relative density at (u, v), both -1…1 from the middle: 1 on the ring. */
export function donutDensity(u: number, v: number): number {
  const r = Math.hypot(u, v);
  const z = (r - DONUT.ring) / (r < DONUT.ring ? DONUT.inner : DONUT.outer);
  return Math.exp(-0.5 * z * z);
}

/** Rejection sampling gives up after this many tries (practically never: about a third of tries are kept). */
const MAX_TRIES = 1000;

/** A random spot as a fraction of the display (0–1 each way), drawn from the donut. `random` returns 0 ≤ n < 1. */
export function donutPoint(random: () => number): { x: number; y: number } {
  let u: number = DONUT.ring;
  let v = 0;
  for (let i = 0; i < MAX_TRIES; i++) {
    const cu = random() * 2 - 1;
    const cv = random() * 2 - 1;
    if (random() < donutDensity(cu, cv)) {
      u = cu;
      v = cv;
      break;
    }
  }
  const span = 0.5 - DONUT.margin;
  return { x: 0.5 + u * span, y: 0.5 + v * span };
}
