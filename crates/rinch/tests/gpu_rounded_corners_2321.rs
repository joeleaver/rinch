//! #2321 on the GPU painter: rounded corners (clips, backgrounds, a border),
//! painted by `VelloPainter` and rendered by Vello (Area), against Chrome
//! 156's pixels at 1x and 2x and against the true area of each corner pixel.
//!
//! Vello flattens every curve to lines at a fixed 0.25 device px, and each
//! chord lies inside the arc, so a rounded corner drawn from kurbo's curves
//! covered up to 52 levels less than the area at 1x. `VelloPainter` now
//! hands Vello the curves of a fill, stroke or clip already flattened at a
//! finer tolerance, so the corner is the area to within a few levels.
//!
//! The page is `tests/fixtures/rounded_corners_2321.html`; Chrome's
//! screenshots of it (`--headless=new --disable-gpu`, so software raster, the
//! red channel) are `tests/fixtures/rounded_corners_2321_chrome_{1,2}x.png`.
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_rounded_corners_2321 -- --ignored`.
#![cfg(feature = "gpu")]

#[path = "support/vello_pixels.rs"]
mod pixels;

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::vello_painter::VelloPainter;

const W: u32 = 240;
const H: u32 = 140;

/// The page Chrome drew: its `<style>` and its `<body>` are loaded as they are.
const PAGE: &str = include_str!("fixtures/rounded_corners_2321.html");

/// The text of `PAGE` between `open` and `close`.
fn part(open: &str, close: &str) -> &'static str {
    let start = PAGE.find(open).unwrap() + open.len();
    &PAGE[start..start + PAGE[start..].find(close).unwrap()]
}

/// What a case draws, for its area oracle.
#[derive(Clone, Copy)]
enum Kind {
    /// A black box clipped by one rounded `overflow: hidden` box.
    Clip,
    /// The same inside a second rounded clip of the same shape: the two
    /// coverages multiply (in Chrome too), so the area squared.
    Nested,
    /// A rounded black background, no clip.
    Background,
    /// A rounded border of this width on a white box.
    Border(f64),
    /// The same with a different colour each side, which paint draws as a
    /// stroke of each side's arc path. The strokes' round caps overlap where
    /// two sides meet, on the corner's diagonal, and the overlap compounds
    /// the edge pixels' coverage there (#2375); those pixels are left out.
    SideBorders(f64),
}

/// (name, border box `x, y, w, h` in CSS px as drawn, radius, kind). Every edge is
/// at an odd CSS px, so no corner sits on a fixed point of the 2x grid.
const CASES: &[(&str, [f64; 4], f64, Kind)] = &[
    ("clip r3", [11.0, 7.0, 30.0, 20.0], 3.0, Kind::Clip),
    ("clip r8", [53.0, 9.0, 40.0, 30.0], 8.0, Kind::Clip),
    ("clip r25", [105.0, 13.0, 60.0, 60.0], 25.0, Kind::Clip),
    (
        "nested clips r8",
        [179.0, 11.0, 40.0, 30.0],
        8.0,
        Kind::Nested,
    ),
    (
        "background r8",
        [11.0, 81.0, 40.0, 30.0],
        8.0,
        Kind::Background,
    ),
    (
        "border 3px r8",
        [63.0, 83.0, 40.0, 30.0],
        8.0,
        Kind::Border(3.0),
    ),
    (
        "background r25",
        [115.0, 79.0, 50.0, 50.0],
        25.0,
        Kind::Background,
    ),
    // The same border in four colours, all with no red: drawn as a stroke of
    // each side's arc path where one colour is a filled ring.
    (
        "border 3px r8, four colours",
        [11.0, 45.0, 40.0, 30.0],
        8.0,
        Kind::SideBorders(3.0),
    ),
    // A 12x10 clip with radius 3 under `transform: scale(3)`: the painter
    // gets the small shape and the transform, so its curves are flattened
    // finer by the transform's stretch.
    (
        "clip r3 scaled 3x",
        [175.0, 79.0, 36.0, 30.0],
        9.0,
        Kind::Clip,
    ),
];

/// The most a corner pixel may differ from its true area, in levels: the
/// polygon `VelloPainter` draws strays up to its `CURVE_TOLERANCE` (0.02
/// device px) from the arc, which costs a pixel up to 5, plus a level of
/// rounding and the oracle's sampling (6 measured, at radius 25). Two nested
/// clips multiply, so their error is up to twice one clip's. With Vello's own
/// flattening it was up to 71.
const AREA_TOL: i32 = 7;
const NESTED_AREA_TOL: i32 = 10;

/// Chrome's software raster is itself up to 73 levels from the area here
/// (Skia flattens curves at 0.25 device px for large radii, and supersamples),
/// so Chrome is pinned by how far it is from the truth: Vello's worst corner
/// pixel against Chrome may be no further than Chrome's own worst against
/// the area, plus [`AREA_TOL`]. Before #2321 Vello was up to 107 levels from
/// Chrome (2x, radius 25).
fn chrome_tol(samples: &[Sample]) -> i32 {
    samples.iter().map(|s| (s.2 - s.3).abs()).max().unwrap() + AREA_TOL
}

fn render(scale: f64) -> (u32, Vec<[u8; 4]>) {
    let (pw, ph) = (
        (W as f64 * scale).round() as u32,
        (H as f64 * scale).round() as u32,
    );
    let mut doc = RinchDocument::new();
    doc.load_css(part("<style>", "</style>"));
    let body = doc.body();
    doc.set_inner_html(body, part("<body>", "</body>"));
    doc.resolve_layout(W as f32, H as f32);
    let mut painter = VelloPainter::new();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    (pw, pixels::render(painter.scene(), pw, ph))
}

/// Chrome's red channel at `scale`, row-major.
fn chrome(scale: f64) -> (u32, Vec<u8>) {
    let bytes: &[u8] = if scale == 1.0 {
        include_bytes!("fixtures/rounded_corners_2321_chrome_1x.png")
    } else {
        include_bytes!("fixtures/rounded_corners_2321_chrome_2x.png")
    };
    let mut reader = png::Decoder::new(bytes).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Grayscale);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    buf.truncate((info.width * info.height) as usize);
    (info.width, buf)
}

/// Whether the device point `(x, y)` is inside the rounded rect `r` with
/// circular radius `rad` (device px).
fn inside(x: f64, y: f64, r: [f64; 4], rad: f64) -> bool {
    let [x0, y0, x1, y1] = r;
    if x < x0 || y < y0 || x > x1 || y > y1 {
        return false;
    }
    let cx = x.clamp(x0 + rad, x1 - rad);
    let cy = y.clamp(y0 + rad, y1 - rad);
    (x - cx).powi(2) + (y - cy).powi(2) <= rad * rad
}

/// The area of device pixel `(px, py)` the rounded rect covers, 0..=1, by
/// 64x64 samples (exact to about 1/4096 of a pixel).
fn area(px: u32, py: u32, r: [f64; 4], rad: f64) -> f64 {
    const N: u32 = 64;
    let mut n = 0;
    for j in 0..N {
        for i in 0..N {
            let x = px as f64 + (i as f64 + 0.5) / N as f64;
            let y = py as f64 + (j as f64 + 0.5) / N as f64;
            n += inside(x, y, r, rad) as u32;
        }
    }
    n as f64 / (N * N) as f64
}

/// The coverage (255 = black) a case's pixel should have, from the area.
fn expected(kind: Kind, px: u32, py: u32, outer: [f64; 4], rad: f64, scale: f64) -> i32 {
    let a = area(px, py, outer, rad);
    let c = match kind {
        Kind::Clip | Kind::Background => a,
        Kind::Nested => a * a,
        Kind::Border(w) | Kind::SideBorders(w) => {
            let w = w * scale;
            let inner = [outer[0] + w, outer[1] + w, outer[2] - w, outer[3] - w];
            a - area(px, py, inner, (rad - w).max(0.0))
        }
    };
    (c * 255.0).round() as i32
}

/// One corner pixel's coverage, as `(pixel, vello, chrome, area)`.
type Sample = ((u32, u32), i32, i32, i32);

/// Every pixel of the top-left and bottom-right corner squares of each case
/// (one pixel wider than the radius), at `scale`.
fn samples(scale: f64) -> Vec<(&'static str, Vec<Sample>)> {
    let (pw, vello) = render(scale);
    let (cw, chrome) = chrome(scale);
    assert_eq!(pw, cw, "Chrome's screenshot is the page at {scale}x");
    CASES
        .iter()
        .map(|&(name, [x, y, w, h], rad, kind)| {
            let outer = [x * scale, y * scale, (x + w) * scale, (y + h) * scale];
            let rad = rad * scale;
            let mut out = Vec::new();
            let squares = [(outer[0], outer[1]), (outer[2] - rad, outer[3] - rad)];
            for (i, (sx, sy)) in squares.into_iter().enumerate() {
                // The arc's centre, for the seam on its diagonal.
                let (cx, cy) = if i == 0 {
                    (sx + rad, sy + rad)
                } else {
                    (sx, sy)
                };
                let (x0, y0) = (sx.floor() as u32 - 1, sy.floor() as u32 - 1);
                let (x1, y1) = ((sx + rad).ceil() as u32 + 1, (sy + rad).ceil() as u32 + 1);
                for py in y0..y1 {
                    for px in x0..x1 {
                        let (dx, dy) = (px as f64 + 0.5 - cx, py as f64 + 0.5 - cy);
                        if matches!(kind, Kind::SideBorders(_))
                            && (dx - dy).abs() / std::f64::consts::SQRT_2 < 1.5
                        {
                            continue;
                        }
                        let i = (py * pw + px) as usize;
                        out.push((
                            (px, py),
                            255 - vello[i][0] as i32,
                            255 - chrome[i] as i32,
                            expected(kind, px, py, outer, rad, scale),
                        ));
                    }
                }
            }
            (name, out)
        })
        .collect()
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_rounded_corners_match_chrome_and_the_area() {
    let mut bad = Vec::new();
    for scale in [1.0, 2.0] {
        for ((name, samples), &(.., kind)) in samples(scale).into_iter().zip(CASES) {
            let area_tol = match kind {
                Kind::Nested => NESTED_AREA_TOL,
                _ => AREA_TOL,
            };
            let chrome_tol = chrome_tol(&samples);
            let worst = |f: &dyn Fn(&Sample) -> i32| {
                samples
                    .iter()
                    .max_by_key(|s| f(s))
                    .map(|s| (f(s), *s))
                    .unwrap()
            };
            let (dc, at_c) = worst(&|s| (s.1 - s.2).abs());
            let (da, at_a) = worst(&|s| (s.1 - s.3).abs());
            if dc > chrome_tol || da > area_tol {
                bad.push(format!(
                    "{scale}x {name}: off Chrome by {dc} (at most {chrome_tol}) at {at_c:?}, \
                     off the area by {da} (at most {area_tol}) at {at_a:?} \
                     ((pixel, vello, chrome, area))"
                ));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
