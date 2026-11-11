//! #1753 — a scaled `url()` background tile is sampled once, bilinearly, from
//! the image, as Chrome samples it.
//!
//! The repeating pattern used to be drawn from a raster of the tile at its
//! device size, and that raster was drawn from the image nearest (the
//! software painter's `draw_image` of an opaque image whose edges are whole
//! pixels is a row copy), so every edge inside and between scaled tiles was
//! hard where Chrome blends it. The pattern now takes the image's own pixels
//! (or, well below its size, its #2456 box-filtered copy) and scales them
//! itself: one bilinear sample per device pixel. An axis that does not repeat
//! clamps its samples to the image's edge, as Chrome's does, instead of
//! blending the far edge in.
//!
//! Every expected value is Chrome 155's (`google-chrome --headless
//! --disable-gpu --force-device-scale-factor=<dpr> --screenshot`, a 60x20
//! div at `left: 0; top: 0` with `body { margin: 0 }`), read on row 10 unless
//! said otherwise. Chrome's (Skia's) bilinear sampler keeps a texel
//! fraction to 1/16, so its blend is up to 1/16 of the step off the exact
//! one: 207 for pixel 0 below where the exact blend is 212.5, 31 for pixel
//! 14 where it is 42.5. [`TOL`] is that 1/16; the nearest raster these
//! replace was 48 or more off at every blended pixel.
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::DecodedImage;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const TOL: i32 = 16;

const R: [u8; 3] = [255, 0, 0];
const B: [u8; 3] = [0, 0, 255];
const K: [u8; 3] = [0, 0, 0];
const W: [u8; 3] = [255, 255, 255];

/// A `w` x `h` opaque image whose pixel `(x, y)` is `px(x, y)`.
fn image(w: u32, h: u32, px: impl Fn(u32, u32) -> [u8; 3]) -> DecodedImage {
    let mut data = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let [r, g, b] = px(x, y);
            data.extend_from_slice(&[r, g, b, 255]);
        }
    }
    DecodedImage::new(data, w, h)
}

/// 10x10, left half red, right half blue.
fn left_right() -> DecodedImage {
    image(10, 10, |x, _| if x < 5 { R } else { B })
}

fn render(style: &str, scale: f64, img: DecodedImage) -> TinySkiaPainter {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: rgb(255, 255, 255); }");
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(
        node,
        "style",
        &format!("position: absolute; left: 0; top: 0; width: 60px; height: 20px; {style}"),
    );
    doc.append_child(body, node);
    doc.tree
        .image_cache
        .insert_decoded("t.png".to_string(), img);
    doc.resolve_layout(100.0, 40.0);
    let mut painter = TinySkiaPainter::new((100.0 * scale) as u32, (40.0 * scale) as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (100.0, 40.0),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter
}

fn rgb(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2]]
}

/// Each `((x, y), [r, g, b])` within [`TOL`] of every channel.
fn check(p: &TinySkiaPainter, want: &[((u32, u32), [u8; 3])]) {
    let bad: Vec<String> = want
        .iter()
        .filter(|((x, y), c)| {
            let g = rgb(p, *x, *y);
            (0..3).any(|k| (g[k] as i32 - c[k] as i32).abs() > TOL)
        })
        .map(|((x, y), c)| format!("({x}, {y}): got {:?}, Chrome {c:?}", rgb(p, *x, *y)))
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

fn row(y: u32, want: &[(u32, [u8; 3])]) -> Vec<((u32, u32), [u8; 3])> {
    want.iter().map(|&(x, c)| ((x, y), c)).collect()
}

/// The issue's case: a 10x10 image at `10px 10px`, DPR 1.5 (15 device px a
/// tile). Pixel 7's centre is the tile's middle, an even red/blue blend, and
/// pixels 0 and 14 blend across the seam between two tiles. Nearest from a
/// device-size raster, they read red, blue and blue.
#[test]
fn a_scaled_image_tile_blends_inside_and_between_tiles() {
    let p = render("background: url(t.png) 0 0 / 10px 10px", 1.5, left_right());
    check(
        &p,
        &row(
            10,
            &[
                (0, [207, 0, 47]),
                (1, R),
                (6, R),
                (7, [127, 0, 127]),
                (8, B),
                (13, B),
                (14, [31, 0, 223]),
                (15, [207, 0, 47]),
                (22, [127, 0, 127]),
                (29, [31, 0, 223]),
            ],
        ),
    );
}

/// Off the fixed points of the case above: a 13px tile (19.5 device px, a
/// 1.95x upscale) at `0.3px`, so no edge falls on a pixel centre or a half.
#[test]
fn an_unevenly_scaled_image_tile_at_a_fraction_matches_chrome() {
    let p = render(
        "background: url(t.png) 0.3px 0 / 13px 10px",
        1.5,
        left_right(),
    );
    check(
        &p,
        &row(
            10,
            &[
                (0, [127, 0, 127]),
                (1, R),
                (8, R),
                (9, [223, 0, 31]),
                (10, [95, 0, 159]),
                (11, B),
                (18, B),
                (19, [63, 0, 191]),
                (20, [191, 0, 63]),
                (21, R),
                (29, [159, 0, 95]),
                (30, [31, 0, 223]),
                (39, [127, 0, 127]),
            ],
        ),
    );
}

/// #1151's case, which pinned this loosely: `0.5px` at DPR 1.5. Chrome reads
/// `(79, 0, 175)` at pixel 0 and blends the inner edge at pixel 8.
#[test]
fn a_scaled_image_tile_at_half_a_pixel_matches_chrome() {
    let p = render(
        "background: url(t.png) 0.5px 0 / 10px 10px",
        1.5,
        left_right(),
    );
    check(
        &p,
        &row(
            10,
            &[
                (0, [79, 0, 175]),
                (1, R),
                (7, R),
                (8, [95, 0, 159]),
                (9, B),
                (14, B),
                (15, [79, 0, 175]),
                (23, [95, 0, 159]),
            ],
        ),
    );
}

/// An axis that does not repeat clamps: a 10x10 image, top half red, bottom
/// half blue, `repeat-x` at DPR 1.5. Rows 0 and 14 (texel centres 0.33 and
/// 9.67) take the top and bottom texel alone, where a wrapping sample would
/// blend the far edge's colour in; row 7 is the inner edge.
#[test]
fn an_axis_that_does_not_repeat_clamps_to_the_image() {
    let img = image(10, 10, |_, y| if y < 5 { R } else { B });
    let p = render("background: url(t.png) 0 0 / 10px 10px repeat-x", 1.5, img);
    let mut want = Vec::new();
    for x in [0, 7, 22] {
        want.extend([
            ((x, 0), R),
            ((x, 1), R),
            ((x, 6), R),
            ((x, 7), [128, 0, 128]),
            ((x, 8), B),
            ((x, 13), B),
            ((x, 14), B),
            ((x, 15), W),
        ]);
    }
    check(&p, &want);
}

/// Well below its size the tile is drawn from the image's #2456 box-filtered
/// copy: a 20x20 image whose every fourth column is black, at `10px 10px`
/// (0.5x), DPR 1. Chrome's mipmap averages each pair of columns, so a black
/// column is grey in every other pixel; sampled straight from the image, it
/// would be dropped or kept whole.
#[test]
fn a_minified_image_tile_counts_every_source_pixel() {
    let img = image(20, 20, |x, _| if x % 4 == 0 { K } else { W });
    let p = render("background: url(t.png) 0 0 / 10px 10px", 1.0, img);
    let mut want = Vec::new();
    for x in 0..24 {
        want.push((x, if x % 2 == 0 { [127, 127, 127] } else { W }));
    }
    check(&p, &row(5, &want));
}

/// #1757's case, the gradient sibling: a 7px tile at DPR 1.5 (10.5 device
/// px, rasterised at 11 and sampled back down) at `0.5px`. The inner edge
/// lands where Chrome's does (pixel 6 a blend, pixel 16 still red); the
/// issue measured it one pixel to the left. Already so at this change's
/// base: pinned here, not changed. A 40% stop, off the tie at a texel
/// centre, matches too.
#[test]
fn a_fractional_gradient_tile_puts_its_inner_edge_where_chrome_does() {
    let half = "linear-gradient(to right, rgb(255, 0, 0) 0 50%, rgb(0, 0, 255) 50% 100%)";
    let p = render(
        &format!("background: {half} 0.5px 0 / 7px 10px"),
        1.5,
        left_right(),
    );
    check(
        &p,
        &row(
            10,
            &[
                (0, [61, 0, 194]),
                (5, R),
                (6, [121, 0, 134]),
                (7, B),
                (11, [194, 0, 61]),
                (16, R),
                (17, B),
                (21, [61, 0, 194]),
            ],
        ),
    );
    let forty = "linear-gradient(to right, rgb(255, 0, 0) 0 40%, rgb(0, 0, 255) 40% 100%)";
    let p = render(
        &format!("background: {forty} 0.5px 0 / 7px 10px"),
        1.5,
        left_right(),
    );
    check(
        &p,
        &row(
            10,
            &[
                (3, R),
                (4, [146, 0, 109]),
                (5, B),
                (11, [194, 0, 61]),
                (14, R),
                (15, [12, 0, 243]),
                (16, B),
            ],
        ),
    );
}
