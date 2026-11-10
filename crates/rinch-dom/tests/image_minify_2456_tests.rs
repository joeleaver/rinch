//! #2456: a picture drawn well below its natural size keeps its thin lines.
//!
//! The fixture is the issue's synthetic picture without its text: 300x200
//! white, twenty 1px black vertical lines (`x = 10 + 7k`, rows 10..90) and
//! fourteen 1px horizontal ones (`y = 100 + 7k`, columns 10..150), drawn at
//! `width: 60%` of a 300px box, so 180x120 (0.6x). The line spacing (4.2
//! device px) puts the lines at every phase against the device grid.
//!
//! Before #2456 the software painter sampled it nearest-neighbour and dropped
//! 8 of the 20 vertical lines outright (darkness 0); Vello sampled it
//! bilinearly with no mipmaps. Chrome 157 (`--force-device-scale-factor=1`)
//! keeps every line (its faintest is 80), and so must rinch, within
//! [`CHROME_MIN`]'s margin; Chrome's lines are [`CHROME_VERTICAL`] and
//! [`CHROME_HORIZONTAL`]. `gpu_image_minify_2456` (in `rinch`) holds Vello
//! to Chrome's faintest at 0.4x.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::image_cache::DecodedImage;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const W: u32 = 300;
const H: u32 = 260;
const SCALE: f64 = 0.6;

/// Chrome 157's darkness (`255 - min grey`) per vertical line, left to right,
/// in the device columns the line covers at row 30.
const CHROME_VERTICAL: [u8; 20] = [
    176, 240, 160, 80, 96, 176, 240, 160, 80, 96, 176, 240, 160, 80, 96, 176, 240, 160, 80, 96,
];
/// The same for the horizontal lines, top to bottom, at column 48.
const CHROME_HORIZONTAL: [u8; 14] = [
    176, 255, 160, 80, 96, 176, 255, 160, 80, 96, 176, 255, 160, 80,
];
/// The faintest line Chrome draws (80), less 10: every line is at least this
/// dark. Chrome's mipmaps are drawn down bilinearly from the level above, and
/// so is rinch's quarter-octave copy (`image_minify::LEVELS_PER_OCTAVE`),
/// whose faintest line here is 78. Nearest sampling's is 0.
const CHROME_MIN: u8 = 70;

fn picture() -> DecodedImage {
    let (w, h) = (300usize, 200usize);
    let mut px = vec![255u8; w * h * 4];
    let mut ink = |x: usize, y: usize| px[(y * w + x) * 4..(y * w + x) * 4 + 3].fill(0);
    for k in 0..20 {
        for y in 10..90 {
            ink(10 + 7 * k, y);
        }
    }
    for k in 0..14 {
        for x in 10..150 {
            ink(x, 100 + 7 * k);
        }
    }
    DecodedImage::new(px, w as u32, h as u32)
}

fn document(img_css: &str) -> RinchDocument {
    document_with_nodes(img_css).0
}

/// [`document`], and its `<div>` and `<img>`.
fn document_with_nodes(
    img_css: &str,
) -> (
    RinchDocument,
    rinch_core::dom::NodeId,
    rinch_core::dom::NodeId,
) {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!(
        "html, body {{ margin: 0; background: #fff; }} div {{ width: 300px; }}
         img {{ display: block; width: 60%; {img_css} }}"
    ));
    doc.tree
        .image_cache
        .insert_decoded("lines.png".to_string(), picture());
    let body = doc.body();
    let div = doc.create_element("div");
    let img = doc.create_element("img");
    doc.set_attribute(img, "src", "lines.png");
    doc.append_child(div, img);
    doc.append_child(body, div);
    (doc, div, img)
}

fn paint(doc: &mut RinchDocument, w: u32, h: u32) -> Vec<[u8; 4]> {
    doc.resolve_layout(w as f32, h as f32);
    let mut painter = TinySkiaPainter::new(w, h);
    painter.fill_white();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (w as f32, h as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

/// Darkness per line, as `measure.py` takes it from Chrome's screenshot:
/// `255 - min grey` over the device pixels each source line covers.
pub fn line_darkness(pixels: &[[u8; 4]], width: u32) -> (Vec<u8>, Vec<u8>) {
    let grey = |x: u32, y: u32| {
        let p = pixels[(y * width + x) as usize];
        ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 3) as u8
    };
    let covered = |s: u32| (SCALE * s as f64) as u32..=(SCALE * (s + 1) as f64) as u32;
    let vertical = (0..20)
        .map(|k| 255 - covered(10 + 7 * k).map(|x| grey(x, 30)).min().unwrap())
        .collect();
    let horizontal = (0..14)
        .map(|k| 255 - covered(100 + 7 * k).map(|y| grey(48, y)).min().unwrap())
        .collect();
    (vertical, horizontal)
}

#[test]
fn every_thin_line_survives_a_0_6x_draw() {
    let pixels = paint(&mut document(""), W, H);
    let (v, h) = line_darkness(&pixels, W);
    let faintest = *v.iter().chain(&h).min().unwrap();
    assert!(
        faintest >= CHROME_MIN,
        "a line fainter than Chrome's faintest ({CHROME_MIN}): vertical {v:?}, \
         horizontal {h:?}; Chrome {CHROME_VERTICAL:?} / {CHROME_HORIZONTAL:?}"
    );
}

/// The ink is conserved: across row 30 the 20 lines' darkness adds up to
/// what 20 lines 0.6 px wide hold (3060), as a box filter keeps it; nearest
/// sampling keeps whole lines or none (12 of 20 here, 3060 only by luck of
/// the count), so this is held to the per-pixel spread as well.
#[test]
fn the_filter_spreads_each_line_over_the_pixels_it_covers() {
    let pixels = paint(&mut document(""), W, H);
    let row: Vec<u32> = (0..95)
        .map(|x| {
            let p = pixels[(30 * W + x) as usize];
            255 - (p[0] as u32 + p[1] as u32 + p[2] as u32) / 3
        })
        .collect();
    let ink: u32 = row.iter().sum();
    assert!(
        (2900..=3220).contains(&ink),
        "row 30 holds {ink} of ink, not ~3060: {row:?}"
    );
    // A box filter at 0.6x leaves no line at full black.
    assert!(row.iter().all(|&d| d < 255), "{row:?}");
}

/// `image-rendering: pixelated` asks for nearest-neighbour, and keeps it:
/// every pixel is the black or white of a texel, never a blend.
#[test]
fn pixelated_stays_nearest() {
    let made = rinch_dom::image_minify::made();
    let pixels = paint(&mut document("image-rendering: pixelated;"), W, H);
    for y in 0..120 {
        for x in 0..180 {
            let p = pixels[(y * W + x) as usize];
            assert!(p[0] == 0 || p[0] == 255, "({x}, {y}) is {p:?}");
        }
    }
    assert_eq!(rinch_dom::image_minify::made(), made, "no copy is made");
}

/// The copy is made once per level and kept: a repaint makes none, a new
/// level makes one, and a draw at 0.75x or more makes none.
#[test]
fn one_copy_per_drawn_size() {
    let (mut doc, _, img) = document_with_nodes("");
    let made = rinch_dom::image_minify::made();
    paint(&mut doc, W, H);
    assert_eq!(rinch_dom::image_minify::made(), made + 1);
    // Another viewport, so layout runs again; the same drawn size.
    paint(&mut doc, W + 10, H);
    assert_eq!(rinch_dom::image_minify::made(), made + 1, "kept");

    // The same image drawn smaller within its level (0.62x, still the
    // 0.707 level): no new copy; drawn at another level (0.45x): one.
    doc.set_attribute(img, "style", "width: 62%");
    paint(&mut doc, W, H);
    assert_eq!(rinch_dom::image_minify::made(), made + 1, "the same level");
    doc.set_attribute(img, "style", "width: 45%");
    paint(&mut doc, W, H);
    assert_eq!(rinch_dom::image_minify::made(), made + 2, "a new level");

    let mut doc = document("width: 50%;");
    paint(&mut doc, W, H);
    assert_eq!(rinch_dom::image_minify::made(), made + 3, "another image");

    let mut doc = document("width: 80%;");
    paint(&mut doc, W, H);
    assert_eq!(
        rinch_dom::image_minify::made(),
        made + 3,
        "0.8x is not minified"
    );
}

/// A picture the frame's budget cannot afford is drawn unfiltered that frame
/// and painted again on the next (review of #2456): the paint records it, the
/// next layout marks it paint-dirty, and the next paint filters it.
#[test]
fn a_picture_over_budget_is_filtered_on_the_next_frame() {
    let _budget = rinch_dom::image_minify::test_work_budget(300 * 200);
    let (mut doc, div, _) = document_with_nodes("");
    doc.tree
        .image_cache
        .insert_decoded("second.png".to_string(), picture());
    let second = doc.create_element("img");
    doc.set_attribute(second, "src", "second.png");
    doc.append_child(div, second);
    let made = rinch_dom::image_minify::made();
    paint(&mut doc, W, H);
    assert_eq!(
        rinch_dom::image_minify::made(),
        made + 1,
        "one fits the budget"
    );
    assert!(doc.tree.has_deferred_minify(), "the other is owed a frame");
    doc.resolve_layout(W as f32, H as f32);
    assert!(!doc.tree.has_deferred_minify(), "drained");
    assert!(
        doc.tree.paint_dirty_nodes.contains(&second.0),
        "the deferred picture is paint-dirty"
    );
    paint(&mut doc, W, H);
    assert_eq!(rinch_dom::image_minify::made(), made + 2, "filtered");
    assert!(!doc.tree.has_deferred_minify());
}

/// A repeating background tile drawn while the budget refused its copy is
/// not kept in the tile cache (review of #2456): the next paint rasterises it
/// again, filtered, instead of drawing the unfiltered tile for good.
#[test]
fn a_tile_drawn_over_budget_is_not_cached() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "html, body { margin: 0; background: #fff; }
         div { width: 360px; height: 120px;
               background: url(lines.png) repeat; background-size: 180px 120px; }",
    );
    doc.tree
        .image_cache
        .insert_decoded("lines.png".to_string(), picture());
    let body = doc.body();
    let div = doc.create_element("div");
    doc.append_child(body, div);
    let made = rinch_dom::image_minify::made();
    let refused = rinch_dom::image_minify::refusals();
    {
        let _budget = rinch_dom::image_minify::test_work_budget(0);
        paint(&mut doc, 400, 200);
    }
    assert!(rinch_dom::image_minify::refusals() > refused, "refused");
    assert_eq!(rinch_dom::image_minify::made(), made);
    paint(&mut doc, 410, 200);
    assert_eq!(
        rinch_dom::image_minify::made(),
        made + 1,
        "the tile was rasterised again, filtered"
    );
}
