//! #1151 — a repeating background keeps its sub-pixel position.
//!
//! #1143 rounded a background's tile phase to whole device pixels, so a
//! `background-position` animation (the `Progress` stripes) moved in
//! one-device-pixel steps. Chrome keeps the fraction on an axis that
//! repeats and blends the edge pixel; on an axis that does **not** repeat it
//! rounds the tile's origin to a device pixel, as rinch does.
//!
//! Every expected value is Chrome 155's (`google-chrome --headless
//! --disable-gpu --force-device-scale-factor=<dpr> --screenshot`, a 60x20
//! div with `body { margin: 0 }`), read at the given device pixel. The tile
//! is 10x10 CSS px, left half red and right half blue, as a gradient or as a
//! 10x10 image.
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const RB: &str = "linear-gradient(to right, rgb(255, 0, 0) 0 50%, rgb(0, 0, 255) 50% 100%)";

fn render(style: &str, scale: f64, image: bool) -> TinySkiaPainter {
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
    if image {
        let mut data = Vec::new();
        for _ in 0..10 {
            for x in 0..10 {
                data.extend_from_slice(if x < 5 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 0, 255, 255]
                });
            }
        }
        doc.tree.image_cache.insert_decoded(
            "t1151.png".to_string(),
            rinch_dom::image_cache::DecodedImage::new(data, 10, 10),
        );
    }
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

/// Each `(x, [r, g, b])` within `tol` of channel, on row `y`.
fn check(p: &TinySkiaPainter, y: u32, want: &[(u32, [u8; 3])], tol: i32) {
    let bad: Vec<String> = want
        .iter()
        .filter(|(x, c)| {
            let g = rgb(p, *x, y);
            (0..3).any(|k| (g[k] as i32 - c[k] as i32).abs() > tol)
        })
        .map(|(x, c)| format!("x={x}: got {:?}, Chrome {c:?}", rgb(p, *x, y)))
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

const R: [u8; 3] = [255, 0, 0];
const B: [u8; 3] = [0, 0, 255];
const W: [u8; 3] = [255, 255, 255];

/// A repeating gradient tile at `0.5px`, DPR 1: every tile edge is a 50/50
/// blend. Rounded, pixel 0 was the far side's colour and nothing blended.
#[test]
fn a_gradient_tile_at_half_a_pixel_blends_its_edges() {
    let p = render(&format!("background: {RB} 0.5px 0 / 10px 10px"), 1.0, false);
    check(
        &p,
        7,
        &[
            (0, [128, 0, 128]),
            (1, R),
            (4, R),
            (5, [128, 0, 128]),
            (6, B),
            (9, B),
            (10, [128, 0, 128]),
            (15, [128, 0, 128]),
        ],
        3,
    );
}

/// Off the half-pixel fixed point: `0.25px` is a 75/25 blend, the weight on
/// the side the pixel's centre falls in.
#[test]
fn a_gradient_tile_at_a_quarter_pixel_blends_by_the_fraction() {
    let p = render(
        &format!("background: {RB} 0.25px 0 / 10px 10px"),
        1.0,
        false,
    );
    check(
        &p,
        7,
        &[
            (0, [191, 0, 64]),
            (1, R),
            (5, [64, 0, 191]),
            (6, B),
            (10, [191, 0, 64]),
            (15, [64, 0, 191]),
        ],
        3,
    );
}

/// DPR 1.5: `0.5px` is `0.75` device px, the 15-device-px tile's edges fall
/// a quarter into pixels 0, 7 and 15.
#[test]
fn a_gradient_tile_at_dpr_1_5_keeps_its_fraction() {
    let p = render(&format!("background: {RB} 0.5px 0 / 10px 10px"), 1.5, false);
    check(
        &p,
        10,
        &[
            (0, [64, 0, 191]),
            (1, R),
            (6, R),
            (7, [191, 0, 64]),
            (8, B),
            (14, B),
            (15, [64, 0, 191]),
            (16, R),
            (22, [191, 0, 64]),
        ],
        3,
    );
}

/// A repeating image tile at DPR 1, at `0.5px` and at `0.3px` (Chrome's
/// bilinear weights are sixteenths: 175 where an exact 0.7 is 178.5).
#[test]
fn an_image_tile_keeps_its_fraction() {
    let p = render("background: url(t1151.png) 0.5px 0 / 10px 10px", 1.0, true);
    check(
        &p,
        7,
        &[
            (0, [127, 0, 127]),
            (1, R),
            (5, [127, 0, 127]),
            (6, B),
            (10, [127, 0, 127]),
        ],
        3,
    );
    let p = render("background: url(t1151.png) 0.3px 0 / 10px 10px", 1.0, true);
    check(
        &p,
        7,
        &[
            (0, [175, 0, 79]),
            (1, R),
            (5, [79, 0, 175]),
            (6, B),
            (10, [175, 0, 79]),
        ],
        5,
    );
}

/// A `no-repeat` tile is rounded to a device pixel, in Chrome as in rinch:
/// `0.5px` lands at 1, with no blended edge.
#[test]
fn a_no_repeat_tile_is_placed_on_a_whole_device_pixel() {
    let p = render(
        "background: rgb(255, 255, 255) url(t1151.png) 0.5px 0 / 10px 10px no-repeat",
        1.0,
        true,
    );
    check(
        &p,
        7,
        &[(0, W), (1, R), (5, R), (6, B), (10, B), (11, W)],
        0,
    );
}

/// `repeat-x` at `0 0.5px`: the axis that does not repeat is rounded (rows
/// 1-5 red, 6-10 blue of a top/bottom tile), the one that does is not.
#[test]
fn only_the_repeating_axis_keeps_its_fraction() {
    let tb = "linear-gradient(rgb(255, 0, 0) 0 50%, rgb(0, 0, 255) 50% 100%)";
    let p = render(
        &format!("background: rgb(255, 255, 255) {tb} 0.5px 0.5px / 10px 10px repeat-x"),
        1.0,
        false,
    );
    for (y, c) in [(0, W), (1, R), (5, R), (6, B), (10, B), (11, W)] {
        check(&p, y, &[(3, c)], 0);
    }
    // And with a red/blue tile on the repeating x axis at the same position,
    // its edge pixel blends.
    let p = render(
        &format!("background: rgb(255, 255, 255) {RB} 0.5px 0.5px / 10px 10px repeat-x"),
        1.0,
        false,
    );
    check(&p, 3, &[(0, [128, 0, 128]), (1, R), (5, [128, 0, 128])], 3);
}

/// What #1151 is about: an animated `background-position` moves smoothly.
/// Pixel 0 of the gradient tile takes a different value at each quarter
/// pixel; rounded, 0 and 0.25 painted alike, and so did 0.5 and 0.75.
#[test]
fn a_position_animation_moves_by_less_than_a_pixel() {
    let reds: Vec<u8> = ["0px", "0.25px", "0.5px", "0.75px", "1px"]
        .iter()
        .map(|pos| {
            let p = render(&format!("background: {RB} {pos} 0 / 10px 10px"), 1.0, false);
            rgb(&p, 0, 7)[0]
        })
        .collect();
    // Pixel 0 is red at 0, then blue creeps in from the left until it is
    // blue at 1px. Chrome: 255, 191, 128, 64, 0.
    for (got, want) in reds.iter().zip([255u8, 191, 128, 64, 0]) {
        assert!(
            (*got as i32 - want as i32).abs() <= 3,
            "red channel at pixel 0 per quarter pixel: {reds:?}"
        );
    }
}

/// An image tile at DPR 1.5 and `0.5px` (`0.75` device px), scaled 1.5x:
/// pixel 0 blends the tile's first column with the last, as Chrome's does.
/// Chrome reads `(79, 0, 175)` there; rinch reads `(64, 0, 191)` because it
/// rasterises a scaled image tile nearest and samples that raster again
/// (#1753), which is also why its inner red/blue edge is not blended. The
/// fraction itself is kept: rounded, pixel 0 was pure blue.
#[test]
fn an_image_tile_at_dpr_1_5_keeps_its_fraction() {
    let p = render("background: url(t1151.png) 0.5px 0 / 10px 10px", 1.5, true);
    for x in [0, 15] {
        let [r, g, b] = rgb(&p, x, 10);
        assert!(
            g == 0 && (40..=120).contains(&r) && r as u32 + b as u32 >= 250,
            "pixel {x}: {:?}, not a red/blue blend",
            [r, g, b]
        );
    }
    check(&p, 10, &[(1, R), (6, R), (9, B), (14, B)], 3);
}

/// One repeating tile that covers the whole box (drawn as one fill, no
/// pattern) keeps its fraction too: a 2px red-to-blue ramp at `49px..51px`
/// of a 100px tile at `-0.3px` reads 153 red at pixel 49 and 26 at 50.
/// Rounded to `0`, it read what `0` reads, 191 and 64.
#[test]
fn a_covering_repeating_tile_keeps_its_fraction() {
    let ramp = "linear-gradient(to right, rgb(255, 0, 0) 49px, rgb(0, 0, 255) 51px)";
    let p = render(
        &format!("background: {ramp} -0.3px 0 / 100px 40px"),
        1.0,
        false,
    );
    check(
        &p,
        7,
        &[(48, R), (49, [153, 0, 102]), (50, [26, 0, 229]), (51, B)],
        4,
    );
    let p = render(&format!("background: {ramp} 0 0 / 100px 40px"), 1.0, false);
    check(&p, 7, &[(49, [191, 0, 64]), (50, [64, 0, 191])], 4);
}

/// The mirror of `only_the_repeating_axis_keeps_its_fraction`: `repeat-y`
/// at `0.5px 0.5px` rounds x (columns 1-5 red, 6-10 blue, 11 white) and
/// keeps y (rows 0, 5 and 10 blend a top/bottom tile).
#[test]
fn repeat_y_rounds_x_and_keeps_y() {
    let p = render(
        &format!("background: rgb(255, 255, 255) {RB} 0.5px 0.5px / 10px 10px repeat-y"),
        1.0,
        false,
    );
    check(
        &p,
        7,
        &[(0, W), (1, R), (5, R), (6, B), (10, B), (11, W)],
        0,
    );
    let tb = "linear-gradient(rgb(255, 0, 0) 0 50%, rgb(0, 0, 255) 50% 100%)";
    let p = render(
        &format!("background: rgb(255, 255, 255) {tb} 0.5px 0.5px / 10px 10px repeat-y"),
        1.0,
        false,
    );
    for (y, c) in [
        (0, [128, 0, 128]),
        (1, R),
        (5, [128, 0, 128]),
        (6, B),
        (10, [128, 0, 128]),
    ] {
        check(&p, y, &[(3, c)], 3);
    }
}
