//! Where a "?" lands. Each machine picks its own spot: the packet carries no position.
//!
//! The first "?" of a burst lands at random on a filled ellipse the shape of the display: densest on a ring a little
//! over half way out, about a third as dense in the middle (quieter, never empty), fading to almost nothing at the
//! edges. The rest of the burst lands around that first one.

use std::collections::HashMap;

/// The densest ring, in "elliptical radius": 0 is the middle of the display, 1 the middle of an edge.
pub const RING: f64 = 0.55;
/// How slowly the density falls towards the middle: the middle keeps about 30 % of the ring's density.
pub const INNER: f64 = 0.35;
/// How fast it falls towards the edges: about 1 % of the ring's density is left there.
pub const OUTER: f64 = 0.15;
/// Kept clear at each edge (a fraction of the display), so a "?" is never cut in half.
pub const MARGIN: f64 = 0.05;
/// Another "?" from the same person this soon after their last one belongs to the same burst: the time one stays up.
pub const BURST_GAP_MS: u64 = 3200;
/// A burst's later pings land this far from its first one, in sizes of a "?".
const NEAR: (f64, f64) = (0.4, 1.0);

/// Relative density at (u, v), both -1…1 from the middle: 1 on the ring.
pub fn density(u: f64, v: f64) -> f64 {
    let r = u.hypot(v);
    let z = (r - RING) / if r < RING { INNER } else { OUTER };
    (-0.5 * z * z).exp()
}

/// A random spot as a fraction of the display (0–1 each way), by rejection sampling: about half the tries are kept.
pub fn point(rng: &mut fastrand::Rng) -> (f64, f64) {
    let (mut u, mut v) = (RING, 0.0);
    for _ in 0..1000 {
        let (cu, cv) = (rng.f64() * 2.0 - 1.0, rng.f64() * 2.0 - 1.0);
        if rng.f64() < density(cu, cv) {
            (u, v) = (cu, cv);
            break;
        }
    }
    let span = 0.5 - MARGIN;
    (0.5 + u * span, 0.5 + v * span)
}

/// A display's area in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    fn w(&self) -> f64 {
        (self.right - self.left) as f64
    }
    fn h(&self) -> f64 {
        (self.bottom - self.top) as f64
    }
}

struct Burst {
    rect: Rect,
    anchor: (f64, f64),
    last: u64,
}

/// Each person's current burst: where its first "?" landed, and when their last one came.
#[derive(Default)]
pub struct Bursts {
    map: HashMap<u64, Burst>,
}

impl Bursts {
    /// Where `origin`'s new "?" goes, in physical pixels, and on which display. `display` is where a new burst lands
    /// (the one under the mouse); `size` is a "?" in physical pixels.
    pub fn place(
        &mut self,
        origin: u64,
        now: u64,
        display: Rect,
        size: f64,
        rng: &mut fastrand::Rng,
    ) -> (Rect, (f64, f64)) {
        self.map.retain(|_, b| now < b.last + BURST_GAP_MS);
        if let Some(b) = self.map.get_mut(&origin) {
            b.last = now;
            let r = b.rect;
            let a = rng.f64() * std::f64::consts::TAU;
            let d = size * (NEAR.0 + rng.f64() * (NEAR.1 - NEAR.0));
            let (mx, my) = (r.w() * MARGIN, r.h() * MARGIN);
            let x = (b.anchor.0 + d * a.cos()).clamp(r.left as f64 + mx, r.right as f64 - mx);
            let y = (b.anchor.1 + d * a.sin()).clamp(r.top as f64 + my, r.bottom as f64 - my);
            return (r, (x, y));
        }
        let (fx, fy) = point(rng);
        let anchor = (display.left as f64 + fx * display.w(), display.top as f64 + fy * display.h());
        self.map.insert(origin, Burst { rect: display, anchor, last: now });
        (display, anchor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPAN: f64 = 0.5 - MARGIN;
    fn radius((x, y): (f64, f64)) -> f64 {
        ((x - 0.5) / SPAN).hypot((y - 0.5) / SPAN)
    }

    #[test]
    fn is_a_filled_disc_with_a_quieter_middle() {
        assert!((0.2..0.4).contains(&density(0.0, 0.0)), "{}", density(0.0, 0.0));
        assert_eq!(density(RING, 0.0), 1.0);
        assert_eq!(density(0.0, -RING), 1.0);
        assert!(density(1.0, 0.0) < 0.02);
        assert!(density(1.0, 1.0) < density(1.0, 0.0));
        // Rises steadily from the middle to the ring: no hole anywhere.
        let mut last = density(0.0, 0.0);
        for i in 1..=55 {
            let d = density(i as f64 / 100.0, 0.0);
            assert!(d > last);
            last = d;
        }
    }

    #[test]
    fn draws_the_disc_inside_the_margins() {
        let mut rng = fastrand::Rng::with_seed(42);
        let n = 40_000;
        let pts: Vec<_> = (0..n).map(|_| point(&mut rng)).collect();
        for &(x, y) in &pts {
            assert!((MARGIN..=1.0 - MARGIN).contains(&x) && (MARGIN..=1.0 - MARGIN).contains(&y));
        }
        let share = |lo: f64, hi: f64| pts.iter().filter(|&&p| (lo..hi).contains(&radius(p))).count() as f64 / n as f64;
        let per_area = |lo: f64, hi: f64| share(lo, hi) / (std::f64::consts::PI * (hi * hi - lo * lo));
        // The middle is quieter than the ring but far from empty; the edges are nearly empty.
        let (middle, ring, edge) = (per_area(0.0, 0.15), per_area(0.5, 0.6), per_area(0.9, 1.0));
        assert!(middle > 0.2 * ring && middle < 0.5 * ring, "{middle} vs {ring}");
        assert!(edge < 0.1 * ring, "{edge} vs {ring}");
        assert!(share(0.0, 0.15) > 0.01);
        // Every direction gets its share.
        let left = pts.iter().filter(|p| p.0 < 0.5).count() as f64 / n as f64;
        let top = pts.iter().filter(|p| p.1 < 0.5).count() as f64 / n as f64;
        assert!((left - 0.5).abs() < 0.02 && (top - 0.5).abs() < 0.02);
    }

    const SCREEN: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1080 };
    const SIDE: Rect = Rect { left: 1920, top: -200, right: 3840, bottom: 880 };

    #[test]
    fn a_burst_gathers_around_its_first_ping() {
        let mut rng = fastrand::Rng::with_seed(7);
        let mut b = Bursts::default();
        let (r, first) = b.place(1, 0, SCREEN, 100.0, &mut rng);
        assert_eq!(r, SCREEN);
        for i in 1..20 {
            // The mouse moved to the other display meanwhile: the burst stays where it started.
            let (r, p) = b.place(1, i * 150, SIDE, 100.0, &mut rng);
            assert_eq!(r, SCREEN);
            let d = (p.0 - first.0).hypot(p.1 - first.1);
            assert!(d <= 100.0 + 1e-9, "{d}");
        }
    }

    #[test]
    fn a_pause_starts_a_new_burst_and_people_burst_apart() {
        let mut rng = fastrand::Rng::with_seed(9);
        let mut b = Bursts::default();
        let (_, first) = b.place(1, 0, SCREEN, 100.0, &mut rng);
        let (r2, _) = b.place(2, 100, SIDE, 100.0, &mut rng);
        assert_eq!(r2, SIDE); // someone else's ping starts their own burst
        let (_, near) = b.place(1, 3000, SIDE, 100.0, &mut rng);
        assert!((near.0 - first.0).hypot(near.1 - first.1) <= 100.0 + 1e-9);
        let (r, _) = b.place(1, 3000 + BURST_GAP_MS, SIDE, 100.0, &mut rng);
        assert_eq!(r, SIDE);
    }

    #[test]
    fn a_burst_near_the_edge_stays_on_the_display() {
        let mut rng = fastrand::Rng::with_seed(3);
        let mut b = Bursts::default();
        b.map.insert(1, Burst { rect: SCREEN, anchor: (100.0, 60.0), last: 0 });
        for i in 1..50 {
            let (_, (x, y)) = b.place(1, i * 10, SCREEN, 100.0, &mut rng);
            assert!(x >= 96.0 && y >= 54.0 && x <= 1824.0 && y <= 1026.0, "{x} {y}");
        }
    }
}
