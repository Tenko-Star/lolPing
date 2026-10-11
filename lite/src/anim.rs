//! The "?" ping's animation, drawn in software: a copy of the original app's `.ping` in overlay.css, at its default
//! size (110 px) and duration (3.2 s). Layers, bottom to top: a glow on the ground, three rings bursting out, and
//! the "?" popping in, bobbing, and fading over its last 13 %.
//!
//! Lengths below are CSS pixels, as in the stylesheet; `k` is the display's scale (1 at 96 dpi).

/// The art's size.
pub const SIZE: f64 = 110.0;
/// The time a ping stays up, in seconds.
pub const DURATION: f64 = 3.2;
/// When its window goes: the original removes a ping 150 ms after its animation.
pub const LIFETIME: f64 = DURATION + 0.15;
/// League's "?" yellow.
pub const COLOR: [f32; 3] = [0xf2 as f32 / 255.0, 0xc5 as f32 / 255.0, 0x18 as f32 / 255.0];
/// The canvas reaches this far from the ping's spot each way: the widest ring with its glow, and the art at its
/// biggest (mid-pop) with its glow.
const HALF: f64 = 122.0;

/// CSS `cubic-bezier(x1, y1, x2, y2)` at progress `p`.
pub fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, p: f64) -> f64 {
    if p <= 0.0 || p >= 1.0 {
        return p.clamp(0.0, 1.0);
    }
    let b = |a: f64, c: f64, t: f64| 3.0 * (1.0 - t) * (1.0 - t) * t * a + 3.0 * (1.0 - t) * t * t * c + t * t * t;
    // x(t) rises from 0 to 1 (x1, x2 are within 0…1): bisection always finds t.
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if b(x1, x2, mid) < p { lo = mid } else { hi = mid }
    }
    b(y1, y2, (lo + hi) / 2.0)
}

fn ease_out(p: f64) -> f64 {
    cubic_bezier(0.0, 0.0, 0.58, 1.0, p)
}

fn ease_in_out(p: f64) -> f64 {
    cubic_bezier(0.42, 0.0, 0.58, 1.0, p)
}

fn pop_curve(p: f64) -> f64 {
    cubic_bezier(0.2, 1.6, 0.4, 1.0, p)
}

/// A CSS keyframe track: (offset, value) stops, `ease` applied within each segment.
fn keyframes(stops: &[(f64, f64)], p: f64, ease: fn(f64) -> f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    for w in stops.windows(2) {
        let ((a, va), (b, vb)) = (w[0], w[1]);
        if p <= b {
            return va + (vb - va) * ease((p - a) / (b - a));
        }
    }
    stops.last().unwrap().1
}

/// Every layer's state at `t` seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub ground: f64,
    /// Each ring's (scale, opacity) while it plays.
    pub rings: [Option<(f64, f64)>; 3],
    pub art_scale: f64,
    pub art_opacity: f64,
    /// The bob, in CSS px (negative is up).
    pub art_dy: f64,
}

const RING_DELAYS: [f64; 3] = [0.0, 0.22, 1.4];
const RING_S: f64 = 0.65;
const POP_S: f64 = 0.28;
const BOB_DELAY: f64 = 0.3;
const BOB_S: f64 = 1.6;

pub fn pose(t: f64) -> Pose {
    let ground = if t >= DURATION {
        0.0
    } else {
        keyframes(&[(0.0, 0.0), (0.1, 0.75), (0.85, 0.55), (1.0, 0.0)], t / DURATION, ease_out)
    };
    let rings = RING_DELAYS.map(|d| {
        let lt = t - d;
        (0.0..RING_S).contains(&lt).then(|| {
            let p = ease_out(lt / RING_S);
            (0.25 + 1.1 * p, 0.95 * (1.0 - p))
        })
    });
    let lp = (t / POP_S).clamp(0.0, 1.0);
    let (art_scale, pop_opacity) = if lp < 0.6 {
        let e = pop_curve(lp / 0.6);
        (0.4 + 0.75 * e, e.clamp(0.0, 1.0))
    } else {
        (1.15 - 0.15 * pop_curve((lp - 0.6) / 0.4), 1.0)
    };
    let life = keyframes(&[(0.0, 1.0), (0.87, 1.0), (1.0, 0.0)], t / DURATION, |p| p);
    let art_dy = if t < BOB_DELAY {
        0.0
    } else {
        let ph = ((t - BOB_DELAY) % BOB_S) / BOB_S;
        if ph < 0.5 { -5.0 * ease_in_out(ph / 0.5) } else { -5.0 * (1.0 - ease_in_out((ph - 0.5) / 0.5)) }
    };
    Pose { ground, rings, art_scale, art_opacity: if t >= DURATION { 0.0 } else { pop_opacity * life }, art_dy }
}

/// The "?" art at one display scale, with its glow (`drop-shadow(0 0 10px)`), premultiplied RGBA.
pub struct Sprite {
    side: usize,
    px: Vec<[f32; 4]>,
}

impl Sprite {
    /// From straight RGBA8 art of `iw`×`ih`.
    pub fn new(rgba: &[u8], iw: usize, ih: usize, k: f64) -> Self {
        let n = (SIZE * k).round().max(1.0) as usize;
        let sigma = 5.0 * k; // a 10 px blur radius
        let pad = (3.0 * sigma).ceil() as usize;
        let side = n + 2 * pad;
        // The art scaled to n×n: 4×4 bilinear samples per pixel, premultiplied.
        let src = |x: f64, y: f64| -> [f32; 4] {
            let (x, y) = ((x - 0.5).clamp(0.0, iw as f64 - 1.0), (y - 0.5).clamp(0.0, ih as f64 - 1.0));
            let (x0, y0) = (x.floor() as usize, y.floor() as usize);
            let (x1, y1) = ((x0 + 1).min(iw - 1), (y0 + 1).min(ih - 1));
            let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
            let at = |x: usize, y: usize| {
                let p = &rgba[(y * iw + x) * 4..][..4];
                let a = p[3] as f32 / 255.0;
                [p[0] as f32 / 255.0 * a, p[1] as f32 / 255.0 * a, p[2] as f32 / 255.0 * a, a]
            };
            let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
            std::array::from_fn(|i| (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy)
        };
        let mut art = vec![[0f32; 4]; side * side];
        let (sx, sy) = (iw as f64 / n as f64, ih as f64 / n as f64);
        for y in 0..n {
            for x in 0..n {
                let mut acc = [0f32; 4];
                for j in 0..4 {
                    for i in 0..4 {
                        let s = src((x as f64 + (i as f64 + 0.5) / 4.0) * sx, (y as f64 + (j as f64 + 0.5) / 4.0) * sy);
                        acc.iter_mut().zip(s).for_each(|(a, v)| *a += v / 16.0);
                    }
                }
                art[(y + pad) * side + x + pad] = acc;
            }
        }
        // The glow: the art's alpha blurred, in the ping colour, under the art.
        let glow = blur(&art.iter().map(|p| p[3]).collect::<Vec<_>>(), side, sigma);
        let px = art
            .iter()
            .zip(glow)
            .map(|(p, g)| {
                let under = (1.0 - p[3]) * g.min(1.0);
                [p[0] + COLOR[0] * under, p[1] + COLOR[1] * under, p[2] + COLOR[2] * under, p[3] + under]
            })
            .collect();
        Self { side, px }
    }

    /// Bilinear sample at (x, y) in sprite pixels; transparent outside.
    fn sample(&self, x: f64, y: f64) -> [f32; 4] {
        let (x, y) = (x - 0.5, y - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
        let at = |xi: f64, yi: f64| -> [f32; 4] {
            if xi < 0.0 || yi < 0.0 || xi >= self.side as f64 || yi >= self.side as f64 {
                [0.0; 4]
            } else {
                self.px[yi as usize * self.side + xi as usize]
            }
        };
        let (a, b, c, d) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
        std::array::from_fn(|i| (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy)
    }
}

/// A separable Gaussian blur of a `side`×`side` channel.
fn blur(src: &[f32], side: usize, sigma: f64) -> Vec<f32> {
    let r = (3.0 * sigma).ceil() as isize;
    let kernel: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f64 / (2.0 * sigma * sigma)).exp() as f32).collect();
    let sum: f32 = kernel.iter().sum();
    let kernel: Vec<f32> = kernel.iter().map(|k| k / sum).collect();
    let pass = |inp: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = vec![0f32; inp.len()];
        for y in 0..side as isize {
            for x in 0..side as isize {
                let mut acc = 0.0;
                for (i, k) in (-r..=r).zip(&kernel) {
                    let (sx, sy) = if horizontal { (x + i, y) } else { (x, y + i) };
                    if sx >= 0 && sy >= 0 && sx < side as isize && sy < side as isize {
                        acc += inp[sy as usize * side + sx as usize] * k;
                    }
                }
                out[y as usize * side + x as usize] = acc;
            }
        }
        out
    };
    pass(&pass(src, true), false)
}

/// The standard normal's upper tail: how much of a Gaussian blur's edge reaches `x` sigmas out.
fn tail(x: f64) -> f64 {
    let z = x.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98 + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let erfc = t * poly.exp();
    0.5 * if x >= 0.0 { erfc } else { 2.0 - erfc }
}

/// A ping's pixels: premultiplied BGRA, ready for a layered window. The ping's spot is the canvas's middle.
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
    k: f64,
}

impl Canvas {
    pub fn new(k: f64) -> Self {
        let w = (2.0 * HALF * k).ceil() as usize;
        Self { w, h: w, px: vec![0; w * w * 4], k }
    }

    /// Where the ping's spot is on the canvas.
    pub fn origin(&self) -> (f64, f64) {
        (self.w as f64 / 2.0, self.h as f64 / 2.0)
    }

    /// Paints premultiplied `c` (with alpha `c[3]`) over pixel `i`.
    #[inline]
    fn over(&mut self, i: usize, c: [f32; 4]) {
        let d = &mut self.px[i * 4..i * 4 + 4];
        let keep = 1.0 - c[3].clamp(0.0, 1.0);
        // BGRA
        for (ch, v) in [(0, c[2]), (1, c[1]), (2, c[0]), (3, c[3])] {
            d[ch] = ((v * 255.0 + d[ch] as f32 * keep).round()).clamp(0.0, 255.0) as u8;
        }
    }

    /// The pixel range covering a box around (cx, cy) on the canvas.
    fn span(&self, cx: f64, cy: f64, rx: f64, ry: f64) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
        let clip = |v: f64, max: usize| v.clamp(0.0, max as f64) as usize;
        (
            clip((cx - rx).floor(), self.w)..clip((cx + rx).ceil(), self.w),
            clip((cy - ry).floor(), self.h)..clip((cy + ry).ceil(), self.h),
        )
    }

    fn solid(alpha: f64) -> [f32; 4] {
        let a = alpha as f32;
        [COLOR[0] * a, COLOR[1] * a, COLOR[2] * a, a]
    }

    /// Draws the ping as it is `t` seconds in.
    pub fn draw(&mut self, t: f64, sprite: &Sprite) {
        self.px.fill(0);
        let p = pose(t);
        let k = self.k;
        let (ox, oy) = self.origin();

        // The ground glow: an ellipse fading from the colour in the middle to nothing at its edge.
        if p.ground > 0.0 {
            let (cx, cy, rx, ry) = (ox, oy + 0.32 * SIZE * k, 0.42 * SIZE * k, 0.12 * SIZE * k);
            let (xs, ys) = self.span(cx, cy, rx, ry);
            for y in ys {
                for x in xs.clone() {
                    let r = ((x as f64 + 0.5 - cx) / rx).hypot((y as f64 + 0.5 - cy) / ry);
                    if r < 1.0 {
                        self.over(y * self.w + x, Self::solid((1.0 - r) * p.ground));
                    }
                }
            }
        }

        // The rings: a 3 px border with an 18 px glow outside and a 12 px one inside, all scaled with the ring.
        for (scale, opacity) in p.rings.into_iter().flatten() {
            let s = k * scale;
            let (outer, border) = (0.545 * SIZE * s, 3.0 * s);
            let inner = outer - border;
            let (so, si) = (9.0 * s, 6.0 * s);
            let reach = outer + 3.0 * so;
            let (xs, ys) = self.span(ox, oy, reach, reach);
            for y in ys {
                for x in xs.clone() {
                    let d = (x as f64 + 0.5 - ox).hypot(y as f64 + 0.5 - oy);
                    if d > reach || d < inner - 3.0 * si {
                        continue;
                    }
                    let line = (d - inner + 0.5).clamp(0.0, 1.0) * (outer - d + 0.5).clamp(0.0, 1.0);
                    let glow = if d > outer {
                        tail((d - outer) / so)
                    } else if d < inner {
                        tail((inner - d) / si)
                    } else {
                        0.0
                    };
                    let a = line + (1.0 - line) * glow;
                    if a > 0.002 {
                        self.over(y * self.w + x, Self::solid(a * opacity));
                    }
                }
            }
        }

        // The art: popped to `art_scale` about its middle, which sits 0.23 of its size above the spot.
        if p.art_opacity > 0.0 {
            let (cx, cy) = (ox, oy - 0.23 * SIZE * k + p.art_dy * k);
            let half = sprite.side as f64 / 2.0 * p.art_scale;
            let (xs, ys) = self.span(cx, cy, half, half);
            let (inv, o) = (1.0 / p.art_scale, p.art_opacity as f32);
            for y in ys {
                for x in xs.clone() {
                    let sx = (x as f64 + 0.5 - cx) * inv + sprite.side as f64 / 2.0;
                    let sy = (y as f64 + 0.5 - cy) * inv + sprite.side as f64 / 2.0;
                    let c = sprite.sample(sx, sy);
                    if c[3] > 0.0 {
                        self.over(y * self.w + x, c.map(|v| v * o));
                    }
                }
            }
        }
    }
}

/// Decodes the bundled "?" art to straight RGBA8.
pub fn art() -> (Vec<u8>, usize, usize) {
    let mut dec =
        png::Decoder::new(std::io::Cursor::new(&include_bytes!("../../assets/textures/enemy_missing.png")[..]));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut r = dec.read_info().expect("bundled art");
    let mut buf = vec![0; r.output_buffer_size().expect("art size")];
    let info = r.next_frame(&mut buf).expect("bundled art");
    buf.truncate(info.buffer_size());
    (buf, info.width as usize, info.height as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezier_matches_known_points() {
        assert!((cubic_bezier(0.0, 0.0, 1.0, 1.0, 0.3) - 0.3).abs() < 1e-6); // linear
        assert_eq!(ease_out(0.0), 0.0);
        assert!((ease_out(1.0) - 1.0).abs() < 1e-9);
        assert!(ease_out(0.5) > 0.5);
        assert!((ease_in_out(0.5) - 0.5).abs() < 1e-6);
        // The pop's curve overshoots.
        assert!((0..100).map(|i| pop_curve(i as f64 / 100.0)).fold(0.0, f64::max) > 1.05);
    }

    #[test]
    fn follows_the_original_timeline() {
        let start = pose(0.0);
        assert_eq!(start.art_opacity, 0.0);
        assert!((start.art_scale - 0.4).abs() < 1e-9);
        assert!(start.rings[0].is_some() && start.rings[1].is_none() && start.rings[2].is_none());
        // Popped in by 0.28 s, a little big halfway through.
        assert!(pose(0.15).art_scale > 1.1);
        assert!((pose(0.3).art_scale - 1.0).abs() < 1e-9 && pose(0.3).art_opacity == 1.0);
        assert!(pose(0.3).rings[1].is_some());
        assert!(pose(1.0).rings.iter().all(Option::is_none));
        assert!(pose(1.5).rings[2].is_some());
        // Bobs 5 px, a cycle every 1.6 s.
        assert!((pose(0.3 + 0.8).art_dy + 5.0).abs() < 1e-6);
        assert!(pose(0.3 + 1.6).art_dy.abs() < 1e-6);
        // Fully there until 87 %, then fades; the ground glow too.
        assert_eq!(pose(0.87 * DURATION - 0.01).art_opacity, 1.0);
        assert!(pose(0.95 * DURATION).art_opacity < 0.5);
        assert!((pose(0.1 * DURATION).ground - 0.75).abs() < 1e-6);
        assert_eq!(pose(DURATION).art_opacity, 0.0);
        assert_eq!(pose(DURATION).ground, 0.0);
    }

    #[test]
    fn draws_the_art_with_a_glow_and_clears_between_frames() {
        let (rgba, w, h) = art();
        assert_eq!((w, h), (128, 128));
        let sprite = Sprite::new(&rgba, w, h, 1.0);
        let mut c = Canvas::new(1.0);
        c.draw(1.0, &sprite);
        let alpha = |c: &Canvas, x: usize, y: usize| c.px[(y * c.w + x) * 4 + 3];
        let (ox, oy) = c.origin();
        // Opaque art above the spot, nothing in the corners.
        let art_y = (oy - 0.23 * SIZE) as usize;
        assert!(alpha(&c, ox as usize, art_y) > 200);
        assert_eq!(alpha(&c, 0, 0), 0);
        // Premultiplied: no channel exceeds alpha.
        for p in c.px.chunks(4) {
            assert!(p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]);
        }
        c.draw(DURATION + 0.05, &sprite);
        assert!(c.px.iter().all(|&v| v == 0));
    }

    #[test]
    fn the_canvas_holds_the_widest_ring_and_biggest_art() {
        // Nothing drawn touches the canvas edge at any time.
        let (rgba, w, h) = art();
        for k in [1.0, 1.5] {
            let sprite = Sprite::new(&rgba, w, h, k);
            let mut c = Canvas::new(k);
            for i in 0..=64 {
                c.draw(i as f64 * DURATION / 64.0, &sprite);
                for x in 0..c.w {
                    for y in [0, c.h - 1] {
                        assert!(c.px[(y * c.w + x) * 4 + 3] <= 2, "k {k} t {i} x {x} y {y}");
                        assert!(c.px[(x * c.w + y) * 4 + 3] <= 2, "k {k} t {i} x {y} y {x}");
                    }
                }
            }
        }
    }
}
