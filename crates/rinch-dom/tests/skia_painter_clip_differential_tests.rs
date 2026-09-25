//! A randomized differential for the software painter's clip shortcuts
//! (#907, from the review of PR #959): random nested clip / layer / fill
//! sequences — rects and rounded rects at fractional, huge and negative
//! coordinates, under translate, scale, flip, rotate and skew — painted by
//! the fast painter and by the reference painter, which must agree byte for
//! byte.
//!
//! In a release build this kills the "fully covered rect's margin moved one
//! pixel outward" mutant by pixels, which `skia_painter_oracle_tests` kills
//! only through `push_clip`'s debug assertions. Debug builds run fewer cases
//! (the assertions re-rasterise every skipped clip); `CLIP_DIFF_CASES=n`
//! overrides the count.

#![cfg(feature = "software-renderer")]

use peniko::kurbo::{Affine, Rect, RoundedRect, RoundedRectRadii};
use peniko::{Brush, Color, Fill};
use rinch_dom::paint::painter::BlendMode;
use rinch_dom::paint::painter::{PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn f(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (self.next() as f64 / (1u64 << 31) as f64) * (hi - lo)
    }
    fn pick(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[derive(Clone, Debug)]
enum Op {
    Clip(Affine, PaintShape),
    Layer(f32),
    Fill(Affine, PaintShape, [u8; 4]),
    Pop,
}

const W: u32 = 97;
const H: u32 = 83;

fn coord(r: &mut Rng, lim: f64) -> f64 {
    match r.pick(6) {
        0 => r.f(-lim, 2.0 * lim).round(),
        1 => r.f(-lim, 2.0 * lim).round() + [0.0001, -0.0001, 0.5, 0.25, 0.999][r.pick(5) as usize],
        2 => [0.0, lim, -0.3, lim + 0.3][r.pick(4) as usize],
        3 => [-40000.0, 40000.0, -1.0e6, 1.0e6][r.pick(4) as usize],
        _ => r.f(-lim, 2.0 * lim),
    }
}

fn shape(r: &mut Rng) -> PaintShape {
    let (a, b) = (coord(r, W as f64), coord(r, W as f64));
    let (c, d) = (coord(r, H as f64), coord(r, H as f64));
    let rect = Rect::new(a.min(b), c.min(d), a.max(b), c.max(d));
    if r.pick(2) == 0 {
        PaintShape::Rect(rect)
    } else {
        let rad = |r: &mut Rng| [0.0, 0.5, 3.0, 7.3, 20.0, 200.0][r.pick(6) as usize];
        PaintShape::RoundedRect(RoundedRect::from_rect(
            rect,
            RoundedRectRadii::new(rad(r), rad(r), rad(r), rad(r)),
        ))
    }
}

fn transform(r: &mut Rng) -> Affine {
    let s = [1.0, 1.25, 1.5, 0.75, -1.0, 2.0][r.pick(6) as usize];
    let sy = if r.pick(3) == 0 {
        [1.0, 1.5, -1.25][r.pick(3) as usize]
    } else {
        s
    };
    let t = Affine::translate((r.f(-10.0, 10.0), r.f(-10.0, 10.0)));
    let base = match r.pick(8) {
        0 => Affine::IDENTITY,
        1 => Affine::rotate(r.f(-0.3, 0.3)),
        2 => Affine::rotate(std::f64::consts::FRAC_PI_2),
        3 => Affine::new([1.0, 0.0, 0.2, 1.0, 0.0, 0.0]),
        _ => Affine::scale_non_uniform(s, sy),
    };
    if s < 0.0 || sy < 0.0 {
        Affine::translate((W as f64, H as f64)) * base * t
    } else {
        t * base
    }
}

fn gen_ops(r: &mut Rng) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut depth = 0;
    // outer damage clip, like build_pixels
    let dx = r.f(0.0, W as f64 - 10.0).floor();
    let dy = r.f(0.0, H as f64 - 10.0).floor();
    if r.pick(4) != 0 {
        ops.push(Op::Clip(
            Affine::IDENTITY,
            PaintShape::Rect(Rect::new(
                dx,
                dy,
                (dx + r.f(1.0, 60.0)).floor().min(W as f64),
                (dy + r.f(1.0, 60.0)).floor().min(H as f64),
            )),
        ));
        depth += 1;
    }
    for _ in 0..(10 + r.pick(20)) {
        match r.pick(7) {
            0..=2 => {
                ops.push(Op::Clip(transform(r), shape(r)));
                depth += 1;
            }
            3 => {
                ops.push(Op::Layer([0.5, 1.0, 0.7][r.pick(3) as usize]));
                depth += 1;
            }
            4 if depth > 0 => {
                ops.push(Op::Pop);
                depth -= 1;
            }
            _ => {
                let c = [
                    r.pick(256) as u8,
                    r.pick(256) as u8,
                    r.pick(256) as u8,
                    [255u8, 128][r.pick(2) as usize],
                ];
                let s = if r.pick(2) == 0 {
                    PaintShape::Rect(Rect::new(-50.0, -50.0, 500.0, 500.0))
                } else {
                    shape(r)
                };
                ops.push(Op::Fill(transform(r), s, c));
            }
        }
    }
    for _ in 0..depth {
        ops.push(Op::Pop);
    }
    ops
}

fn run(p: &mut TinySkiaPainter, ops: &[Op]) -> Vec<u8> {
    p.fill_white();
    for op in ops {
        match op {
            Op::Clip(t, s) => p.push_clip(Fill::NonZero, *t, s),
            Op::Layer(o) => p.push_layer(
                BlendMode::default(),
                *o,
                Affine::IDENTITY,
                &PaintShape::Rect(Rect::new(0.0, 0.0, W as f64, H as f64)),
            ),
            Op::Fill(t, s, c) => p.fill(
                Fill::NonZero,
                *t,
                &Brush::Solid(Color::from_rgba8(c[0], c[1], c[2], c[3])),
                s,
            ),
            Op::Pop => p.pop_layer(),
        }
    }
    p.end_frame();
    p.pixels().to_vec()
}

#[test]
fn random_clip_sequences_match_the_reference_painter() {
    let default = if cfg!(debug_assertions) { 400 } else { 3000 };
    let n: u64 = std::env::var("CLIP_DIFF_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default);
    let mut fast = TinySkiaPainter::new(W, H);
    let mut refp = TinySkiaPainter::new(W, H);
    refp.set_reference_mode(true);
    let mut cases = 0u64;
    for seed in 0..n {
        let mut r = Rng(seed * 7919 + 13);
        let ops = gen_ops(&mut r);
        let a = run(&mut fast, &ops);
        let b = run(&mut refp, &ops);
        if a != b {
            let diff = a.iter().zip(&b).position(|(x, y)| x != y).unwrap();
            panic!(
                "seed {seed}: pixel differs at byte {diff} (px {},{}): {:?}",
                (diff / 4) as u32 % W,
                (diff / 4) as u32 / W,
                ops
            );
        }
        cases += 1;
    }
    let s = fast.stats();
    let rs = refp.stats();
    // Positive control: the cases ran, and the fast painter took its
    // shortcuts on them (the reference fills every clip over the surface).
    assert_eq!(cases, n);
    assert!(
        s.clip_mask_px < rs.clip_mask_px,
        "fast {} vs reference {}",
        s.clip_mask_px,
        rs.clip_mask_px
    );
}
