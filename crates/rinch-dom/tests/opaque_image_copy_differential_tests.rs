//! A randomized differential for `TinySkiaPainter::copy_opaque_image` (#361)
//! against the ordinary premultiply-and-`draw_pixmap` path (the same painter,
//! the same pixels submitted `opaque: false`), under random clip stacks —
//! rects, rounded rects and damage-like unions, whole-pixel and fractional —
//! opacity layers, scales (non-uniform included) and offsets. Written by the
//! second review of PR #1002. 3000 cases in release, 600 in a debug build.

#![cfg(feature = "software-renderer")]

use peniko::Fill;
use peniko::kurbo::{Affine, BezPath, Rect, RoundedRect, Shape};
use rinch_dom::paint::painter::{BlendMode, PaintImage, PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const SW: u32 = 173;
const SH: u32 = 131;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn f(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * (self.below(1_000_000) as f64 / 1_000_000.0)
    }
    fn coin(&mut self) -> bool {
        self.below(2) == 0
    }
}

fn noise(w: u32, h: u32, seed: u32, opaque: bool) -> Vec<u8> {
    (0..w * h)
        .flat_map(|i| {
            let v = i.wrapping_mul(2_654_435_761).wrapping_add(seed);
            let a = if opaque { 255 } else { (v >> 3) as u8 | 1 };
            [(v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8, a]
        })
        .collect()
}

#[derive(Clone, Debug)]
enum Op {
    Clip(PaintShape),
    Layer(f32),
}

fn rand_shape(r: &mut Rng) -> PaintShape {
    let int = r.coin();
    let mut c = |lo: f64, hi: f64| {
        let v = r.f(lo, hi);
        if int { v.round() } else { v }
    };
    let (x0, y0) = (c(-20.0, 120.0), c(-20.0, 90.0));
    let (x1, y1) = (x0 + c(5.0, 150.0), y0 + c(5.0, 120.0));
    match r.below(3) {
        0 => Rect::new(x0, y0, x1, y1).into(),
        1 => RoundedRect::from_rect(Rect::new(x0, y0, x1, y1), r.f(1.0, 30.0)).into(),
        _ => {
            // A damage-like union of up to three whole-pixel rects.
            let mut p = BezPath::new();
            for _ in 0..1 + r.below(3) {
                let ax = r.f(-10.0, 150.0).round();
                let ay = r.f(-10.0, 110.0).round();
                let rr = Rect::new(
                    ax,
                    ay,
                    ax + r.f(2.0, 90.0).round(),
                    ay + r.f(2.0, 70.0).round(),
                );
                p.extend(rr.path_elements(0.1));
            }
            PaintShape::BezPath(p)
        }
    }
}

fn run(
    ops: &[Op],
    img: &[u8],
    w: u32,
    h: u32,
    ts: Affine,
    opaque: bool,
    reference: bool,
) -> (Vec<u8>, u64) {
    let mut p = TinySkiaPainter::new(SW, SH);
    p.set_reference_mode(reference);
    let bg = noise(SW, SH, 7, false);
    p.draw_image(
        &PaintImage {
            data: &bg,
            width: SW,
            height: SH,
            decoded: None,
            opaque: false,
        },
        Affine::IDENTITY,
    );
    let before = p.stats().opaque_image_copies;
    let full: PaintShape = Rect::new(0.0, 0.0, SW as f64, SH as f64).into();
    for op in ops {
        match op {
            Op::Clip(s) => p.push_clip(Fill::NonZero, Affine::IDENTITY, s),
            Op::Layer(o) => p.push_layer(BlendMode::Normal, *o, Affine::IDENTITY, &full),
        }
    }
    p.draw_image(
        &PaintImage {
            data: img,
            width: w,
            height: h,
            decoded: None,
            opaque,
        },
        ts,
    );
    // Draw something after, inside the same stack, so a lost clip shows.
    let after = noise(20, 20, 3, true);
    p.draw_image(
        &PaintImage {
            data: &after,
            width: 20,
            height: 20,
            decoded: None,
            opaque: false,
        },
        Affine::translate((150.0, 110.0)),
    );
    for _ in ops {
        p.pop_layer();
    }
    let copies = p.stats().opaque_image_copies - before;
    (p.pixels().to_vec(), copies)
}

#[test]
fn the_copy_matches_draw_pixmap_randomized() {
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    let scales = [
        1.0,
        1.0,
        1.0,
        0.5,
        0.25,
        2.0,
        3.0,
        1.5,
        0.75,
        1.25,
        2.5,
        0.6,
        7.0 / 3.0,
    ];
    let mut copied = 0u64;
    let mut cases = 0u64;
    let mut clipped_copies = 0u64;
    let mut layered_copies = 0u64;
    let n_cases: u32 = if cfg!(debug_assertions) { 600 } else { 3000 };
    let per_3000 = |n: u64| n * n_cases as u64 / 3000;
    for case in 0..n_cases {
        let w = 1 + r.below(90) as u32;
        let h = 1 + r.below(70) as u32;
        let img = noise(w, h, case, true);
        let (sx, sy) = if r.below(4) == 0 {
            // A destination size picked in whole pixels, as a viewport would.
            let dw = 1 + r.below(160);
            let dh = 1 + r.below(120);
            (dw as f64 / w as f64, dh as f64 / h as f64)
        } else {
            let s = scales[r.below(scales.len() as u64) as usize];
            (
                s,
                if r.below(5) == 0 {
                    scales[r.below(scales.len() as u64) as usize]
                } else {
                    s
                },
            )
        };
        let (tx, ty) = if r.below(6) == 0 {
            (r.f(-40.0, 150.0), r.f(-40.0, 120.0))
        } else {
            (r.f(-40.0, 150.0).round(), r.f(-40.0, 120.0).round())
        };
        let ts = Affine::translate((tx, ty)) * Affine::scale_non_uniform(sx, sy);
        let mut ops = Vec::new();
        for _ in 0..r.below(4) {
            if r.below(4) == 0 {
                ops.push(Op::Layer([0.5f32, 0.25, 0.8][r.below(3) as usize]));
            } else {
                ops.push(Op::Clip(rand_shape(&mut r)));
            }
        }
        let (fast, n) = run(&ops, &img, w, h, ts, true, false);
        let (slow, m) = run(&ops, &img, w, h, ts, false, false);
        assert_eq!(m, 0);
        cases += 1;
        copied += n;
        if n > 0 && ops.iter().any(|o| matches!(o, Op::Clip(_))) {
            clipped_copies += 1;
        }
        if n > 0 && ops.iter().any(|o| matches!(o, Op::Layer(_))) {
            layered_copies += 1;
        }
        if let Some(i) = fast.chunks(4).zip(slow.chunks(4)).position(|(a, b)| a != b) {
            panic!(
                "case {case}: copy differs from draw_pixmap at ({}, {}): {:?} vs {:?}\n\
                 img {w}x{h} ts {ts:?} ops {ops:?} copied {n}",
                i as u32 % SW,
                i as u32 / SW,
                &fast[i * 4..i * 4 + 4],
                &slow[i * 4..i * 4 + 4]
            );
        }
    }
    eprintln!("cases {cases} copied {copied} clipped {clipped_copies} layered {layered_copies}");
    assert!(
        copied > per_3000(500),
        "positive control: the copy path ran ({copied})"
    );
    assert!(
        clipped_copies > per_3000(100),
        "copies under clips ran ({clipped_copies})"
    );
    assert!(
        layered_copies > per_3000(50),
        "copies in layers ran ({layered_copies})"
    );
}
