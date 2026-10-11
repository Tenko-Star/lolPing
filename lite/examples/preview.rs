//! Renders what lolPing Lite draws, for checking by eye:
//!   cargo run --release --example preview -- frames out.png   the "?" animation, frame by frame
//!   cargo run --release --example preview -- spots out.png    where 400 000 "?" land on a 16:9 display

use lolping_lite::anim::{self, Canvas, Sprite};
use lolping_lite::spot;

const TIMES: [f64; 10] = [0.05, 0.1, 0.17, 0.3, 0.5, 1.0, 1.55, 2.5, 2.95, 3.12];

fn save(path: &str, w: usize, h: usize, rgb: &[u8]) {
    let f = std::fs::File::create(path).expect("output file");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.write_header().unwrap().write_image_data(rgb).unwrap();
}

fn frames(path: &str) {
    let (rgba, iw, ih) = anim::art();
    let sprite = Sprite::new(&rgba, iw, ih, 1.0);
    let mut c = Canvas::new(1.0);
    let (w, h) = (c.w * TIMES.len(), c.h);
    let bg = [0x2a, 0x30, 0x38];
    let mut out = vec![0u8; w * h * 3];
    for (n, &t) in TIMES.iter().enumerate() {
        c.draw(t, &sprite);
        for y in 0..c.h {
            for x in 0..c.w {
                let p = &c.px[(y * c.w + x) * 4..][..4];
                let keep = 1.0 - p[3] as f32 / 255.0;
                let o = (y * w + n * c.w + x) * 3;
                // Premultiplied BGRA over the background.
                for (ch, v) in [(0, p[2]), (1, p[1]), (2, p[0])] {
                    out[o + ch] = (v as f32 + bg[ch] as f32 * keep).round().min(255.0) as u8;
                }
            }
        }
    }
    save(path, w, h, &out);
}

fn spots(path: &str) {
    let (w, h) = (160, 90);
    let mut grid = vec![0u32; w * h];
    let mut rng = fastrand::Rng::with_seed(1);
    for _ in 0..400_000 {
        let (x, y) = spot::point(&mut rng);
        grid[((y * h as f64) as usize).min(h - 1) * w + ((x * w as f64) as usize).min(w - 1)] += 1;
    }
    let max = *grid.iter().max().unwrap() as f64;
    // Black, through purple and orange, to pale yellow.
    let stops = [
        [0, 0, 4],
        [40, 11, 84],
        [101, 21, 110],
        [159, 42, 99],
        [212, 72, 66],
        [245, 125, 21],
        [250, 193, 39],
        [252, 255, 164],
    ];
    let s = 6;
    let mut out = vec![0u8; w * s * h * s * 3];
    for y in 0..h * s {
        for x in 0..w * s {
            let v = grid[(y / s) * w + x / s] as f64 / max * (stops.len() - 1) as f64;
            let (i, f) = ((v.floor() as usize).min(stops.len() - 2), v - v.floor().min((stops.len() - 2) as f64));
            for ch in 0..3 {
                out[(y * w * s + x) * 3 + ch] =
                    (stops[i][ch] as f64 * (1.0 - f) + stops[i + 1][ch] as f64 * f).round() as u8;
            }
        }
    }
    save(path, w * s, h * s, &out);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match (args.get(1).map(String::as_str), args.get(2)) {
        (Some("frames"), Some(p)) => frames(p),
        (Some("spots"), Some(p)) => spots(p),
        _ => eprintln!("usage: preview frames|spots out.png"),
    }
}
