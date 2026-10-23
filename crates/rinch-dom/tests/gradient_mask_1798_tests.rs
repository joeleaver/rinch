//! #1798 — a `repeating-linear-gradient` with length stops, a sized and
//! positioned `radial-gradient`, and `mask-image`.
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const CASE_CSS: &str = "html, body { margin: 0; background: #fff; }
div { position: absolute; width: 320px; height: 200px; }
#a { left: 0; top: 0; background: repeating-linear-gradient(90deg, transparent 0 31px, #262a33 31px 32px); }
#b { left: 330px; top: 0; background: radial-gradient(120% 60% at 50% -8%, rgba(224,162,74,1), transparent 55%); }
#c { left: 0; top: 210px; background: red; mask-image: linear-gradient(180deg, #000, transparent 62%); }
#d { left: 330px; top: 210px; background: radial-gradient(120% 60% at 50% -8%, rgba(224,162,74,0.5), transparent 55%), repeating-linear-gradient(90deg, transparent 0 31px, #262a33 31px 32px); mask-image: linear-gradient(180deg, #000, transparent 62%); }
#e { left: 0; top: 420px; background: radial-gradient(circle 40px at 30px 50px, blue, transparent); }
#f { left: 330px; top: 420px; background: repeating-linear-gradient(45deg, red 0 10px, blue 10px 20px); }";

fn paint_case(css: &str, ids: &[&str], w: u32, h: u32) -> TinySkiaPainter {
    let mut doc = RinchDocument::new();
    doc.load_css(css);
    let body = doc.body();
    for id in ids {
        let n = doc.create_element("div");
        doc.set_attribute(n, "id", id);
        doc.append_child(body, n);
    }
    doc.resolve_layout(w as f32, h as f32);
    let mut painter = TinySkiaPainter::new(w, h);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (w as f32, h as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter
}

#[test]
#[ignore]
fn dump_case_png() {
    let p = paint_case(CASE_CSS, &["a", "b", "c", "d", "e", "f"], 660, 630);
    let path = std::env::var("RINCH_1798_DUMP").expect("RINCH_1798_DUMP");
    image::save_buffer(&path, p.pixels(), 660, 630, image::ColorType::Rgba8).unwrap();
}

fn px(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2]]
}

/// Every expectation is Chrome 155's pixel (headless,
/// `--force-device-scale-factor=1`, the same markup), within `tol` per channel.
fn expect(p: &TinySkiaPainter, want: &[((u32, u32), [u8; 3])], tol: u8) {
    let bad: Vec<_> = want
        .iter()
        .filter_map(|&((x, y), w)| {
            let got = px(p, x, y);
            (got.iter().zip(w).any(|(a, b)| a.abs_diff(b) > tol)).then_some(((x, y), got, w))
        })
        .collect();
    assert!(bad.is_empty(), "(point, rinch, chrome): {bad:?}");
}

fn case() -> TinySkiaPainter {
    paint_case(CASE_CSS, &["a", "b", "c", "d", "e", "f"], 660, 630)
}

/// `repeating-linear-gradient(90deg, transparent 0 31px, #262a33 31px 32px)`:
/// a 1px line at x = 31 + 32k. The two-position stops and their lengths are
/// px along the gradient line, not hundredths of it: rinch read `31px` as
/// 31% and painted `#262a33` from 99px to the box's end, never repeating.
#[test]
fn a_repeating_linear_gradient_with_length_stops_draws_a_line_every_32px() {
    let p = case();
    let dark = [38, 42, 51];
    let white = [255, 255, 255];
    expect(
        &p,
        &[
            ((0, 100), white),
            ((30, 100), white),
            ((31, 100), dark),
            ((32, 100), white),
            ((63, 100), dark),
            ((159, 100), dark),
            ((200, 100), white),
        ],
        2,
    );
    let lines = (0..320).filter(|&x| px(&p, x, 100) == dark).count();
    assert_eq!(lines, 10, "Chrome: x = 31, 63, ..., 319");
}

/// `radial-gradient(120% 60% at 50% -8%, orange, transparent 55%)`: an
/// ellipse 384x120 about (160, -16), fading over 55% of it, interpolated
/// premultiplied (orange fading out, not through grey). rinch drew a circle
/// about the box's centre and faded through grey.
#[test]
fn b_radial_gradient_takes_its_size_and_position() {
    let p = case();
    expect(
        &p,
        &[
            ((490, 0), [231, 185, 119]),
            ((490, 10), [236, 199, 146]),
            ((490, 30), [245, 227, 201]),
            ((490, 50), [255, 255, 255]),
            ((330, 0), [248, 236, 218]),
            ((330, 20), [253, 249, 243]),
            ((600, 5), [243, 219, 186]),
            ((490, 100), [255, 255, 255]),
        ],
        3,
    );
}

/// `mask-image: linear-gradient(180deg, #000, transparent 62%)` on a red box:
/// fades out over the top 62% (124px), nothing below. rinch ignored it.
#[test]
fn c_mask_image_gradient_fades_the_element() {
    let p = case();
    expect(
        &p,
        &[
            ((100, 210), [255, 1, 1]),
            ((100, 240), [255, 63, 63]),
            ((100, 280), [255, 145, 145]),
            ((100, 330), [255, 248, 248]),
            ((100, 340), [255, 255, 255]),
            ((100, 400), [255, 255, 255]),
        ],
        3,
    );
}

/// The issue's rule (on white, the glow at 50% alpha): both layers, masked.
#[test]
fn d_the_layers_together_under_the_mask() {
    let p = case();
    expect(
        &p,
        &[
            ((361, 215), [75, 69, 64]),
            ((361, 260), [127, 129, 134]),
            ((361, 330), [249, 249, 249]),
            ((490, 212), [244, 222, 190]),
            ((370, 212), [249, 238, 222]),
            ((360, 260), [255, 255, 255]),
            ((450, 400), [255, 255, 255]),
        ],
        3,
    );
}

/// `radial-gradient(circle 40px at 30px 50px, blue, transparent)`.
#[test]
fn e_circle_radius_and_length_position() {
    let p = case();
    expect(
        &p,
        &[
            ((30, 470), [5, 5, 255]),
            ((60, 470), [194, 194, 255]),
            ((5, 470), [157, 157, 255]),
            ((30, 430), [252, 252, 255]),
            ((100, 470), [255, 255, 255]),
        ],
        3,
    );
}

/// `repeating-linear-gradient(45deg, red 0 10px, blue 10px 20px)`: stripes
/// 10px wide across the 45° gradient line through the box's centre, checked
/// on a grid against that formula (which Chrome's own pixels match at every
/// one of the 5777 sampled points more than 1px from a stripe edge).
#[test]
fn f_repeating_linear_gradient_at_an_angle() {
    let p = case();
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let len = 320.0 * s + 200.0 * s;
    let (sx, sy) = (160.0 - s * len / 2.0, 100.0 + s * len / 2.0);
    let mut bad = Vec::new();
    for y in (0..200).step_by(3) {
        for x in (0..320).step_by(3) {
            let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
            let t = ((fx - sx) * s - (fy - sy) * s).rem_euclid(20.0);
            if t.min((t - 10.0).abs()).min(20.0 - t) < 1.0 {
                continue;
            }
            let want = if t < 10.0 { [255, 0, 0] } else { [0, 0, 255] };
            let got = px(&p, 330 + x, 420 + y);
            if got.iter().zip(want).any(|(a, b)| a.abs_diff(b) > 3) {
                bad.push((x, y, got));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} off: {:?}",
        bad.len(),
        &bad[..bad.len().min(8)]
    );
}

const ISSUE_CSS: &str = "html, body { margin: 0; background: #14161b; }
.loom { position: fixed; inset: 0; z-index: 0; pointer-events: none;
  background: radial-gradient(120% 60% at 50% -8%, rgba(224, 162, 74, 0.05), transparent 55%),
    repeating-linear-gradient(90deg, transparent 0 31px, #262a33 31px 32px);
  mask-image: linear-gradient(180deg, #000, transparent 62%); }
#g { position: absolute; left: 20px; top: 20px; width: 100px; height: 50px; background: blue;
  mask-image: linear-gradient(#000, #000); }
#g > div { width: 300px; height: 100px; background: red; }
#k { position: absolute; left: 20px; top: 200px; width: 100px; height: 50px; background: blue;
  mask-image: linear-gradient(90deg, #000, transparent); }
#k > div { position: absolute; left: 50px; top: 20px; width: 200px; height: 100px; background: red; }";

fn issue() -> TinySkiaPainter {
    let mut doc = RinchDocument::new();
    doc.load_css(ISSUE_CSS);
    let body = doc.body();
    let loom = doc.create_element("div");
    doc.set_attribute(loom, "class", "loom");
    doc.append_child(body, loom);
    let g = doc.create_element("div");
    doc.set_attribute(g, "id", "g");
    doc.append_child(body, g);
    let child = doc.create_element("div");
    doc.append_child(g, child);
    let k = doc.create_element("div");
    doc.set_attribute(k, "id", "k");
    doc.append_child(body, k);
    let abs = doc.create_element("div");
    doc.append_child(k, abs);
    doc.resolve_layout(1440.0, 900.0);
    let mut painter = TinySkiaPainter::new(1440, 900);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (1440.0, 900.0),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter
}

/// #1798's own rule at 1440x900: faint lines every 32px fading downward, a
/// faint glow at the top, no slab.
#[test]
fn g_the_issue_backdrop_renders_like_chrome() {
    let p = issue();
    expect(
        &p,
        &[
            ((30, 300), [20, 22, 27]),
            ((31, 300), [28, 30, 38]),
            ((31, 5), [40, 42, 50]),
            ((31, 100), [35, 39, 47]),
            ((31, 500), [21, 23, 29]),
            ((31, 600), [20, 22, 27]),
            ((720, 0), [27, 27, 27]),
            ((720, 40), [25, 26, 28]),
            ((200, 5), [24, 24, 28]),
            ((1300, 5), [23, 24, 27]),
            ((10, 800), [20, 22, 27]),
            ((1439, 300), [28, 30, 38]),
            ((1407, 300), [28, 30, 38]),
        ],
        2,
    );
}

/// A mask is clipped to the border box (`mask-clip: border-box`): a child
/// overflowing a masked box is not drawn outside it.
#[test]
fn h_a_mask_hides_overflow_outside_the_border_box() {
    let p = issue();
    expect(
        &p,
        &[
            ((50, 30), [255, 0, 0]),
            ((110, 60), [255, 0, 0]),
            ((150, 30), [22, 23, 27]),
            ((130, 40), [22, 23, 27]),
            ((110, 80), [21, 22, 26]),
        ],
        2,
    );
}

/// A positioned descendant, which paints in the masked box's own stacking
/// sequence, is masked and clipped with it: Chrome 155's pixels.
#[test]
fn k_a_positioned_descendant_is_masked_too() {
    let p = issue();
    expect(
        &p,
        &[
            ((30, 210), [2, 2, 230]),
            ((80, 240), [113, 13, 16]),
            ((110, 240), [42, 19, 24]),
            ((100, 260), [20, 22, 27]),
            ((200, 240), [20, 22, 27]),
            ((150, 280), [20, 22, 27]),
        ],
        3,
    );
}

/// Review of #1811: the background tile cache was keyed by raster size only,
/// while stop positions are absolute lengths in the tile's own space. Box A's
/// 40px tile and box B's 20px tile under `scale(2)` both rasterise at 40x40,
/// so B took A's tile (black 0..5 of 40 instead of 0..5 of 20) and drew white
/// at device x = 7.
#[test]
fn l_tiles_of_one_raster_size_and_different_tile_sizes_are_not_shared() {
    let css = "html, body { margin: 0; background: #fff; }
div { position: absolute; left: 0; }
#a { top: 0; width: 200px; height: 40px; background: linear-gradient(90deg, black 0 5px, white 5px); background-size: 40px 40px; }
#b { top: 100px; width: 100px; height: 20px; background: linear-gradient(90deg, black 0 5px, white 5px); background-size: 20px 20px; transform: scale(2); transform-origin: 0 0; }";
    let alone = paint_case(css, &["b"], 300, 200);
    assert_eq!(px(&alone, 7, 110), [0, 0, 0]);
    let both = paint_case(css, &["a", "b"], 300, 200);
    assert_eq!(px(&both, 7, 10), [255, 255, 255]);
    assert_eq!(px(&both, 7, 110), [0, 0, 0]);
}

/// Review of #1811: a repeating radial gradient whose period is rotated to
/// start at radius 0 must end the period with the colour just *before* that
/// point. `red -5px, red 0, blue 0, blue 15px` is red from radius 15 to 20;
/// Chrome 155's pixels along the radius.
#[test]
fn m_a_repeating_radial_period_closes_on_the_colour_before_its_start() {
    let css = "html, body { margin: 0; background: #fff; }
#r { position: absolute; left: 0; top: 0; width: 100px; height: 100px;
  background: repeating-radial-gradient(circle at 50px 50px, red -5px, red 0px, blue 0px, blue 15px); }";
    let p = paint_case(css, &["r"], 100, 100);
    let (red, blue) = ([255, 0, 0], [0, 0, 255]);
    expect(
        &p,
        &[
            ((52, 50), blue),
            ((60, 50), blue),
            ((64, 50), blue),
            ((66, 50), red),
            ((67, 50), red),
            ((68, 50), red),
            ((69, 50), red),
            ((71, 50), blue),
            ((80, 50), blue),
            ((86, 50), red),
            ((87, 50), red),
        ],
        3,
    );
}
