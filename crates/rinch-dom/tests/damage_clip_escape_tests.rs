//! #1007 (from the review of PR #1031): a draw `push_damage_clip` lets through
//! unmasked must never write a pixel outside the damage rects — its bounds
//! test (fill pad, stroke reach with the miter limit) is all that clips it.
//!
//! The fuzz runs 300 cases by default; `R1031_N=n` overrides (the review ran
//! 6000).
#![cfg(feature = "software-renderer")]

use peniko::kurbo::{Affine, BezPath, Cap, Circle, Join, Line, Point, Rect, RoundedRect, Stroke};
use peniko::{Brush, Color, Fill};
use rinch_dom::paint::DamageRegion;
use rinch_dom::paint::painter::{BlendMode, PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const W: u32 = 96;
const H: u32 = 80;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    #[allow(dead_code)]
    fn f(&mut self) -> f64 {
        (self.next() % 1_000_000) as f64 / 1_000_000.0
    }
    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[(self.next() % xs.len() as u64) as usize]
    }
}

fn backdrop(p: &mut TinySkiaPainter) {
    for y in 0..H {
        for x in 0..W {
            let c = Color::from_rgba8(
                (37 + x * 2) as u8,
                (200 - y * 2) as u8,
                ((x * 7 + y * 5) % 251) as u8,
                255,
            );
            p.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                &Brush::Solid(c),
                &PaintShape::Rect(Rect::new(
                    x as f64,
                    y as f64,
                    x as f64 + 1.0,
                    y as f64 + 1.0,
                )),
            );
        }
    }
}

/// An edge value near `e`: on it, or a hair either side.
fn near(r: &mut Rng, e: f64) -> f64 {
    e + r.pick(&[
        0.0, 1e-4, -1e-4, 1e-7, -1e-7, 0.25, -0.25, 0.5, -0.5, 0.999, -0.999, 1.5, -1.5, 3.0, -3.0,
    ])
}

fn transform(r: &mut Rng) -> Affine {
    match r.next() % 9 {
        0 => Affine::IDENTITY,
        1 => Affine::translate((r.pick(&[0.3, -0.7, 0.5, 1e-5]), r.pick(&[0.25, -0.5, 0.0]))),
        2 => Affine::new([
            r.pick(&[0.5, 1.25, 2.0, 0.333]),
            0.0,
            0.0,
            r.pick(&[0.5, 1.25, 0.75]),
            0.4,
            -0.3,
        ]),
        3 => Affine::new([-1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        4 => Affine::rotate(r.pick(&[1e-6, 0.01, 0.3, -0.2, std::f64::consts::FRAC_PI_2])),
        5 => Affine::skew(r.pick(&[0.0, 0.1, 1e-6]), r.pick(&[0.05, 0.0])),
        6 => Affine::scale(r.pick(&[1.5, 0.25, 3.0])),
        7 => Affine::new([1.0, 0.0, 0.0, 1.0, 0.123456, 0.987654]),
        _ => Affine::new([1.0 + 1e-6, 0.0, 0.0, 1.0 - 1e-6, 0.0, 0.0]),
    }
}

fn shape(r: &mut Rng, b: Rect) -> PaintShape {
    match r.next() % 6 {
        0 => PaintShape::Rect(b),
        1 => PaintShape::RoundedRect(RoundedRect::new(
            b.x0,
            b.y0,
            b.x1,
            b.y1,
            r.pick(&[0.5, 3.3, 9.3, 40.0]),
        )),
        2 => {
            let c = b.center();
            PaintShape::Circle(Circle::new(c, (b.width().min(b.height()) / 2.0).abs()))
        }
        3 => {
            // A cubic whose control points overshoot the endpoints' box.
            let mut p = BezPath::new();
            p.move_to((b.x0, b.y1));
            p.curve_to((b.x0, b.y0), (b.x1, b.y0), (b.x1, b.y1));
            p.close_path();
            PaintShape::BezPath(p)
        }
        4 => {
            // An acute spike, for a miter join.
            let mut p = BezPath::new();
            p.move_to((b.x0, b.y0));
            p.line_to((b.x1, b.center().y));
            p.line_to((b.x0, b.y0 + 0.4));
            p.close_path();
            PaintShape::BezPath(p)
        }
        _ => PaintShape::Line(Line::new(Point::new(b.x0, b.y0), Point::new(b.x1, b.y1))),
    }
}

#[test]
fn no_unmasked_draw_escapes_the_damage() {
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let seeds: usize = std::env::var("R1031_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let mut base = TinySkiaPainter::new(W, H);
    backdrop(&mut base);
    let backdrop_px = base.pixels().to_vec();
    let mut escapes = Vec::new();
    let mut partial_ne_full_inside = 0usize;
    for case in 0..seeds {
        // One or two whole-pixel damage rects.
        let x0 = (rng.next() % 40) as f64 + 1.0;
        let y0 = (rng.next() % 30) as f64 + 1.0;
        let d0 = Rect::new(
            x0,
            y0,
            x0 + 10.0 + (rng.next() % 40) as f64,
            y0 + 8.0 + (rng.next() % 35) as f64,
        );
        let mut region = DamageRegion::new(W as f64, H as f64);
        region.add(d0);
        if rng.next() % 3 == 0 {
            region.add(Rect::new(0.0, 0.0, 6.0, 5.0));
        }
        let ts = transform(&mut rng);
        // Draw bounds near the damage edges, in DEVICE space; map back into local.
        let dev = Rect::new(
            near(&mut rng, d0.x0),
            near(&mut rng, d0.y0),
            near(&mut rng, d0.x1),
            near(&mut rng, d0.y1),
        );
        let inv = ts.inverse();
        let local = inv.transform_rect_bbox(dev);
        let sh = shape(&mut rng, local);
        let stroke_it = matches!(sh, PaintShape::Line(_)) || rng.next() % 2 == 0;
        let st = Stroke::new(rng.pick(&[0.0, 0.3, 1.0, 2.5, 6.0]))
            .with_join(rng.pick(&[Join::Miter, Join::Round, Join::Bevel]))
            .with_miter_limit(rng.pick(&[1.0, 4.0, 10.0]))
            .with_caps(rng.pick(&[Cap::Butt, Cap::Square, Cap::Round]));
        let layer = rng.next() % 4 == 0;
        let brush = Brush::Solid(Color::from_rgba8(250, 10, 250, rng.pick(&[255u8, 128])));
        let draw = |p: &mut TinySkiaPainter| {
            if layer {
                p.push_layer(
                    BlendMode::Normal,
                    0.5,
                    Affine::IDENTITY,
                    &PaintShape::Rect(d0),
                );
            }
            if stroke_it {
                p.stroke(&st, ts, &brush, &sh);
            } else {
                p.fill(Fill::NonZero, ts, &brush, &sh);
            }
            if layer {
                p.pop_layer();
            }
        };
        let mut part = TinySkiaPainter::new(W, H);
        backdrop(&mut part);
        part.push_damage_clip(&region);
        draw(&mut part);
        part.pop_layer();
        let mut full = TinySkiaPainter::new(W, H);
        backdrop(&mut full);
        draw(&mut full);
        let rects = region.rects().to_vec();
        let (a, b, f) = (&backdrop_px, part.pixels(), full.pixels());
        for y in 0..H {
            for x in 0..W {
                let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
                let inside = rects.iter().any(|r| r.contains(Point::new(fx, fy)));
                let i = ((y * W + x) * 4) as usize;
                if !inside && a[i..i + 4] != b[i..i + 4] {
                    escapes.push(format!("case {case}: ({x},{y}) ts={ts:?} dev={dev:?} damage={rects:?} stroke={stroke_it} w={} layer={layer} shape={sh:?}", st.width));
                }
                if inside && f[i..i + 4] != b[i..i + 4] {
                    partial_ne_full_inside += 1;
                }
            }
        }
    }
    eprintln!(
        "R1031: cases={seeds} escaped_px={} partial!=full inside px={partial_ne_full_inside}",
        escapes.len()
    );
    for e in escapes.iter().take(20) {
        eprintln!("{e}");
    }
    assert!(
        escapes.is_empty(),
        "{} pixels outside the damage changed",
        escapes.len()
    );
}

/// A 30-degree miter spike whose tip lands past the damage: the stroke reach
/// must count the miter limit, or the unmasked stroke escapes.
#[test]
fn a_miter_spike_does_not_escape() {
    let d = Rect::new(20.0, 20.0, 70.0, 60.0);
    let mut region = DamageRegion::new(W as f64, H as f64);
    region.add(d);
    let mut p = BezPath::new();
    // Apex at x = 62, 30 degrees; 6px stroke miter reaches ~11.6px past it.
    let half = (15f64).to_radians().tan() * 30.0;
    p.move_to((32.0, 40.0 - half));
    p.line_to((62.0, 40.0));
    p.line_to((32.0, 40.0 + half));
    p.close_path();
    let sh = PaintShape::BezPath(p);
    for limit in [4.0, 10.0] {
        let st = Stroke::new(6.0)
            .with_join(Join::Miter)
            .with_miter_limit(limit);
        let mut base = TinySkiaPainter::new(W, H);
        backdrop(&mut base);
        let mut part = TinySkiaPainter::new(W, H);
        backdrop(&mut part);
        part.push_damage_clip(&region);
        part.stroke(
            &st,
            Affine::IDENTITY,
            &Brush::Solid(Color::from_rgba8(0, 0, 0, 255)),
            &sh,
        );
        part.pop_layer();
        let mut full = TinySkiaPainter::new(W, H);
        backdrop(&mut full);
        full.stroke(
            &st,
            Affine::IDENTITY,
            &Brush::Solid(Color::from_rgba8(0, 0, 0, 255)),
            &sh,
        );
        let (a, b, f) = (base.pixels(), part.pixels(), full.pixels());
        let mut out = 0;
        let mut full_out = 0;
        for y in 0..H {
            for x in 0..W {
                let i = ((y * W + x) * 4) as usize;
                let inside = (20..70).contains(&x) && (20..60).contains(&y);
                if !inside && a[i..i + 4] != b[i..i + 4] {
                    out += 1;
                }
                if !inside && a[i..i + 4] != f[i..i + 4] {
                    full_out += 1;
                }
            }
        }
        eprintln!(
            "R1031 miter limit {limit}: partial escaped {out}, full paints {full_out} outside (positive control)"
        );
        assert!(full_out > 0 || limit < 5.0);
        assert_eq!(out, 0);
    }
}

/// Not reachable from `build_pixels`, which pushes one damage clip with no clip
/// open: a second damage clip pushed inside the first is an ordinary clip, so
/// after its pop the outer clip's own rects decide — not the inner ones.
#[test]
fn a_nested_damage_clip_leaves_the_outer_rects_in_force() {
    let a = Rect::new(10.0, 10.0, 30.0, 30.0);
    let b = Rect::new(2.0, 2.0, 90.0, 70.0);
    let mut ra = DamageRegion::new(W as f64, H as f64);
    ra.add(a);
    let mut rb = DamageRegion::new(W as f64, H as f64);
    rb.add(b);
    let mut base = TinySkiaPainter::new(W, H);
    backdrop(&mut base);
    let mut p = TinySkiaPainter::new(W, H);
    backdrop(&mut p);
    p.push_damage_clip(&ra);
    p.push_damage_clip(&rb);
    p.pop_layer();
    p.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        &Brush::Solid(Color::from_rgba8(0, 0, 0, 255)),
        &PaintShape::Rect(Rect::new(50.0, 40.0, 60.0, 50.0)),
    );
    p.pop_layer();
    let i = ((45 * W + 55) * 4) as usize;
    eprintln!(
        "R1031 nested: pixel (55,45) outside the outer damage: before {:?} after {:?}",
        &base.pixels()[i..i + 4],
        &p.pixels()[i..i + 4]
    );
    assert_eq!(&base.pixels()[i..i + 4], &p.pixels()[i..i + 4]);
}
