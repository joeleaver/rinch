//! #468, round 2 — tiling edge cases and the features the first round's
//! fixtures left unpinned. Written by the review of PR #1143; every expected
//! value is Chrome 153's (headless, measured, `--force-device-scale-factor`
//! for DPR), except where a fixture says it is a control.
//!
//! - `r1`, `r2` — **seams** (review F1). Each tile used to be filled as its own
//!   antialiased rect; two tiles meeting mid device pixel each covered it by
//!   half and composited to 75%, a page-coloured line at every boundary.
//!   Chrome draws none. Fixed by snapping tile edges to device pixels
//!   (`LayerGeometry::tiles`). Both red before that, green after.
//! - `r4` — **many tiles** (review F2). An 8px tile on a 600x600 box is 5625
//!   tiles; past the old 4096 cap the layer was painted as one stretched tile
//!   (left half red, right half blue). It is now one repeating pattern
//!   (`Painter::fill_repeating`), whatever the count. Red before, green after.
//! - `r3`, `r5`, `r6` — rounded clip, the underlay while an image loads, a
//!   transformed box (which takes the tile-by-tile path). Pass at both.
//! - `f1`–`f5` — `content-box` origin, `cover`, `contain`, an image with one
//!   `auto` axis keeping its ratio, keyframe `right 10px` and `center`. Each
//!   kills a mutant the first round's fixtures let survive (content-box
//!   padding ignored; cover/contain swapped; end-side offset sign; ratio
//!   inverted; `center` read as 0).
//! - `r8`, `r9` — an unrelated animation pins `background-position` /
//!   `opacity`: #781, pre-existing, `#[ignore]`d here.
//! - `r7` — a timing probe, `#[ignore]`d (run with `--ignored --nocapture`).
#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const SOLID: &str = "background-image: linear-gradient(rgb(255, 0, 0), rgb(255, 0, 0))";
const RB: &str =
    "background-image: linear-gradient(to right, rgb(255, 0, 0) 0 50%, rgb(0, 0, 255) 50% 100%)";

fn paint_at(doc: &mut RinchDocument, scale: f64, vw: f32, vh: f32) -> TinySkiaPainter {
    let mut painter = TinySkiaPainter::new((vw as f64 * scale) as u32, (vh as f64 * scale) as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (vw, vh),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter
}

fn px(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

fn mount(style: &str, vw: f32, vh: f32) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css("html, body { margin: 0; background: rgb(255, 255, 255); }");
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(
        node,
        "style",
        &format!("position: absolute; left: 0; top: 0; {style}"),
    );
    doc.append_child(body, node);
    doc.resolve_layout(vw, vh);
    (doc, node)
}

/// A solid tile at a fractional position (what every frame of the Progress
/// stripe animation is): Chrome paints the seams solid.
#[test]
fn r1_no_seam_at_a_fractional_position() {
    let (mut doc, _) = mount(
        &format!(
            "width: 100px; height: 20px; {SOLID}; background-size: 10px 20px; background-position: 0.5px 0"
        ),
        200.0,
        100.0,
    );
    let p = paint_at(&mut doc, 1.0, 200.0, 100.0);
    let bad: Vec<(u32, [u8; 4])> = (0..100)
        .map(|x| (x, px(&p, x, 10)))
        .filter(|(_, c)| *c != [255, 0, 0, 255])
        .collect();
    assert!(bad.is_empty(), "seams: {bad:?}");
}

/// `r1` on the tile-by-tile path: `repeat-x` is not a pattern (it repeats on
/// one axis), so each tile is filled on its own and only the device-pixel
/// snap keeps the seams shut. Chrome 153 (measured): every column pure red.
#[test]
fn r1b_no_seam_at_a_fractional_position_tile_by_tile() {
    let (mut doc, _) = mount(
        &format!(
            "width: 100px; height: 20px; {SOLID}; background-size: 10px 20px; \
             background-position: 0.5px 0; background-repeat: repeat-x"
        ),
        200.0,
        100.0,
    );
    let p = paint_at(&mut doc, 1.0, 200.0, 100.0);
    let bad: Vec<(u32, [u8; 4])> = (0..100)
        .map(|x| (x, px(&p, x, 10)))
        .filter(|(_, c)| *c != [255, 0, 0, 255])
        .collect();
    assert!(bad.is_empty(), "seams: {bad:?}");
}

/// The default `Painter::fill_repeating` (one `draw_image` per repetition,
/// what a painter without a pattern primitive gets) and the software
/// painter's `Pattern` override paint the same pixels for the same tile.
#[test]
fn the_default_fill_repeating_agrees_with_the_software_pattern() {
    use peniko::Fill;
    use peniko::kurbo::{Affine, Rect};
    use rinch_dom::paint::painter::{PaintShape, Painter, RepeatTile};

    /// Everything forwarded to a `TinySkiaPainter` except `fill_repeating`,
    /// which takes the trait default.
    struct Plain(TinySkiaPainter);
    impl Painter for Plain {
        fn reset(&mut self) {
            self.0.reset()
        }
        fn fill(&mut self, f: Fill, t: Affine, b: &Brush, s: &PaintShape) {
            self.0.fill(f, t, b, s)
        }
        fn stroke(&mut self, st: &peniko::kurbo::Stroke, t: Affine, b: &Brush, s: &PaintShape) {
            self.0.stroke(st, t, b, s)
        }
        #[allow(clippy::too_many_arguments)]
        fn draw_glyphs(
            &mut self,
            font: &peniko::FontData,
            size: f32,
            t: Affine,
            gt: Option<Affine>,
            b: &Brush,
            hint: bool,
            coords: &[i16],
            glyphs: &[rinch_dom::paint::painter::PaintGlyph],
        ) {
            self.0
                .draw_glyphs(font, size, t, gt, b, hint, coords, glyphs)
        }
        fn draw_image(&mut self, i: &rinch_dom::paint::painter::PaintImage<'_>, t: Affine) {
            self.0.draw_image(i, t)
        }
        fn push_clip(&mut self, f: Fill, t: Affine, s: &PaintShape) {
            self.0.push_clip(f, t, s)
        }
        fn push_layer(
            &mut self,
            b: rinch_dom::paint::painter::BlendMode,
            o: f32,
            t: Affine,
            s: &PaintShape,
        ) {
            self.0.push_layer(b, o, t, s)
        }
        fn pop_layer(&mut self) {
            self.0.pop_layer()
        }
    }

    // A 7x5 tile of distinct, partly translucent (premultiplied) pixels.
    let mut data = Vec::new();
    for y in 0..5u8 {
        for x in 0..7u8 {
            let a = 128 + x * 16;
            data.extend_from_slice(&[(x * 30).min(a), (y * 50).min(a), 20.min(a), a]);
        }
    }
    let pixels = peniko::Blob::from(data);
    let tile = RepeatTile {
        pixels: &pixels,
        width: 7,
        height: 5,
        transform: Affine::translate((3.0, -2.0)),
        smooth: false,
    };
    let shape = PaintShape::Rect(Rect::new(4.0, 6.0, 57.0, 38.0));
    let t = Affine::translate((5.0, 1.0));

    let mut pattern = TinySkiaPainter::new(70, 50);
    pattern.fill_repeating(Fill::NonZero, t, &tile, &shape);
    let mut plain = Plain(TinySkiaPainter::new(70, 50));
    plain.fill_repeating(Fill::NonZero, t, &tile, &shape);

    let (a, b) = (pattern.pixels(), plain.0.pixels());
    assert!(
        a.iter().any(|&v| v != 0),
        "positive control: the pattern drew"
    );
    let worst = a
        .iter()
        .zip(b)
        .map(|(x, y)| (*x as i16 - *y as i16).abs())
        .max()
        .unwrap();
    assert!(
        worst <= 1,
        "the two paths differ by up to {worst} per channel"
    );
}

/// A solid 10px tile at DPR 1.25 (12.5 device px) at an integer CSS position.
#[test]
fn r2_no_seam_at_hidpi() {
    let (mut doc, _) = mount(
        &format!("width: 100px; height: 20px; {SOLID}; background-size: 10px 20px"),
        200.0,
        100.0,
    );
    let p = paint_at(&mut doc, 1.25, 200.0, 100.0);
    let bad: Vec<(u32, [u8; 4])> = (0..124)
        .map(|x| (x, px(&p, x, 12)))
        .filter(|(_, c)| *c != [255, 0, 0, 255])
        .collect();
    assert!(bad.is_empty(), "seams: {bad:?}");
}

/// Tiling under a rounded box stays inside the corner.
#[test]
fn r3_tiles_are_clipped_to_the_rounded_corner() {
    let (mut doc, _) = mount(
        &format!(
            "width: 100px; height: 40px; border-radius: 16px; {RB}; background-size: 40px 20px; background-position: 30px 0"
        ),
        200.0,
        100.0,
    );
    let p = paint_at(&mut doc, 1.0, 200.0, 100.0);
    assert_eq!(px(&p, 0, 0), [255, 255, 255, 255], "top-left corner");
    assert_eq!(px(&p, 99, 39), [255, 255, 255, 255], "bottom-right corner");
    assert_eq!(px(&p, 40, 20), [255, 0, 0, 255], "mid tile");
}

/// 8px tiles on a 600x600 box = 5625, past the tile-by-tile cap. Chrome 153
/// (measured): x = 2 R, 6 B, 402 R, 406 B, and no pixel of the row is anything
/// but red or blue.
#[test]
fn r4_many_tiles_keep_tiling() {
    let (mut doc, _) = mount(
        &format!("width: 600px; height: 600px; {RB}; background-size: 8px 8px"),
        700.0,
        700.0,
    );
    let p = paint_at(&mut doc, 1.0, 700.0, 700.0);
    let row: Vec<[u8; 4]> = [2, 6, 402, 406].iter().map(|&x| px(&p, x, 300)).collect();
    assert_eq!(
        row,
        vec![
            [255, 0, 0, 255],
            [0, 0, 255, 255],
            [255, 0, 0, 255],
            [0, 0, 255, 255]
        ]
    );
}

/// A not-yet-loaded url() still paints its background-color.
#[test]
fn r5_underlay_paints_while_the_image_loads() {
    let (mut doc, _) = mount(
        "width: 100px; height: 20px; background: rgb(0, 255, 0) url(r1143-not-loaded.png)",
        200.0,
        100.0,
    );
    let p = paint_at(&mut doc, 1.0, 200.0, 100.0);
    assert_eq!(px(&p, 50, 10), [0, 255, 0, 255]);
}

/// A transformed box: the tile set moves with the box.
#[test]
fn r6_scaled_box_scales_its_tiles() {
    let (mut doc, _) = mount(
        &format!(
            "width: 100px; height: 20px; transform-origin: 0 0; transform: scale(2); {RB}; background-size: 40px 20px; background-position: 30px 0; background-repeat: no-repeat"
        ),
        300.0,
        100.0,
    );
    let p = paint_at(&mut doc, 1.0, 300.0, 100.0);
    // Chrome: W 0-60, R 60-100, B 100-140, W 140-200 on y=20.
    let row: Vec<[u8; 4]> = [30, 80, 120, 170].iter().map(|&x| px(&p, x, 20)).collect();
    assert_eq!(
        row,
        vec![
            [255, 255, 255, 255],
            [255, 0, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 255, 255]
        ]
    );
}

/// Cost of a tiled layer: one tile, 4096 tiles and 5625 tiles.
#[test]
#[ignore = "timing probe, not a test"]
fn r7_timing_near_the_cap() {
    for (label, size) in [
        ("1 tile", "600px 600px"),
        ("4096 tiles", "9.375px 9.375px"),
        ("5625 (fallback)", "8px 8px"),
    ] {
        let (mut doc, _) = mount(
            &format!("width: 600px; height: 600px; {RB}; background-size: {size}"),
            700.0,
            700.0,
        );
        let mut best = f64::MAX;
        for _ in 0..5 {
            let t = std::time::Instant::now();
            let _ = paint_at(&mut doc, 1.0, 700.0, 700.0);
            best = best.min(t.elapsed().as_secs_f64() * 1e3);
        }
        eprintln!("r7 {label}: {best:.2} ms");
    }
}

// ---------------------------------------------------------------------------
// Chrome-153-measured fixtures for features the PR's own tests leave unpinned
// (each kills a surviving mutant; see the review report).
// ---------------------------------------------------------------------------

fn class(p: &TinySkiaPainter, x: u32, y: u32) -> char {
    match px(p, x, y) {
        [255, 0, 0, 255] => 'R',
        [0, 0, 255, 255] => 'B',
        [255, 255, 255, 255] | [_, _, _, 0] => 'W',
        _ => '?',
    }
}
fn row_of(p: &TinySkiaPainter, y: u32, xs: &[u32]) -> String {
    xs.iter().map(|&x| class(p, x, y)).collect()
}
fn half_image(w: u32, h: u32) -> rinch_dom::image_cache::DecodedImage {
    let mut data = Vec::new();
    for _ in 0..h {
        for x in 0..w {
            data.extend_from_slice(if x < w / 2 {
                &[255, 0, 0, 255]
            } else {
                &[0, 0, 255, 255]
            });
        }
    }
    rinch_dom::image_cache::DecodedImage::new(data, w, h)
}
fn with_image(style: &str, w: u32, h: u32) -> TinySkiaPainter {
    let (mut doc, _) = mount(style, 200.0, 100.0);
    doc.tree
        .image_cache
        .insert_decoded("r1143.png".to_string(), half_image(w, h));
    paint_at(&mut doc, 1.0, 200.0, 100.0)
}

/// Chrome 153: content box 70x20 at (15,15): y=25 W 0-15, R 15-50, B 50-85, W 85-100; y=12 W.
#[test]
fn f1_content_box_origin() {
    let (mut doc, _) = mount(
        &format!(
            "width: 100px; height: 50px; border: 10px solid transparent; padding: 5px; {RB}; background-repeat: no-repeat; background-origin: content-box"
        ),
        200.0,
        100.0,
    );
    let p = paint_at(&mut doc, 1.0, 200.0, 100.0);
    assert_eq!(row_of(&p, 25, &[10, 20, 45, 55, 80, 90]), "WRRBBW");
    assert_eq!(row_of(&p, 12, &[20, 80]), "WW");
}

/// Chrome 153: a 10x10 image, `contain` in 100x20 → 20x20: R 0-10, B 10-20, W after.
#[test]
fn f2_contain() {
    let p = with_image(
        "width: 100px; height: 20px; background-image: url(r1143.png); background-size: contain; background-repeat: no-repeat",
        10,
        10,
    );
    assert_eq!(row_of(&p, 10, &[5, 15, 30]), "RBW");
}

/// Chrome 153: `cover` → 100x100: R 0-50, B 50-100.
#[test]
fn f2_cover() {
    let p = with_image(
        "width: 100px; height: 20px; background-image: url(r1143.png); background-size: cover; background-repeat: no-repeat",
        10,
        10,
    );
    assert_eq!(row_of(&p, 10, &[30, 70]), "RB");
}

/// Chrome 153: a 10x5 image at `20px auto` keeps its ratio → 20x10: y=5 R B W, y=15 W.
#[test]
fn f5_one_auto_axis_keeps_the_ratio() {
    let p = with_image(
        "width: 100px; height: 20px; background-image: url(r1143.png); background-size: 20px auto; background-repeat: no-repeat",
        10,
        5,
    );
    assert_eq!(row_of(&p, 5, &[5, 15, 30]), "RBW");
    assert_eq!(row_of(&p, 15, &[5, 15]), "WW");
}

fn keyframed(position: &str) -> TinySkiaPainter {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!(
        "html, body {{ margin: 0; background: rgb(255, 255, 255); }}
         @keyframes r1143k {{ from, to {{ background-position: {position}; }} }}
         .k {{ animation: r1143k 1000ms linear infinite; }}"
    ));
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(
        node,
        "style",
        &format!("position: absolute; left: 0; top: 0; width: 100px; height: 20px; {RB}; background-size: 40px 20px; background-repeat: no-repeat"),
    );
    doc.append_child(body, node);
    doc.resolve_layout(200.0, 100.0);
    doc.set_attribute(node, "class", "k");
    doc.resolve_layout(201.0, 100.0);
    let t0 = doc.tree.active_animations[&node.0][0].start_time_ms;
    rinch_dom::animation::tick_animations(&mut doc.tree, t0 + 300.0);
    paint_at(&mut doc, 1.0, 200.0, 100.0)
}

/// Chrome 153: `right 10px top 0` in a keyframe → tile at 50: W R B W.
#[test]
fn f3_keyframe_end_side_offset() {
    assert_eq!(
        row_of(&keyframed("right 10px top 0"), 10, &[40, 60, 80, 95]),
        "WRBW"
    );
}

/// Chrome 153: `center 0` in a keyframe → tile at 30: W R B W.
#[test]
fn f4_keyframe_center() {
    assert_eq!(
        row_of(&keyframed("center 0"), 10, &[20, 40, 60, 80]),
        "WRBW"
    );
}

/// An unrelated animation with an implicit `from` stop (auto-generated from
/// the base style, which now carries background-position) must not pin the
/// background-position a later class change sets. Chrome 153: the class's
/// position applies — the animation does not name background-position.
#[test]
#[ignore = "#781: an animation's auto-generated stop pins every base property"]
fn r8_an_unrelated_animation_does_not_pin_background_position() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "html, body { margin: 0; background: rgb(255, 255, 255); }
         @keyframes r1143o { to { opacity: 1; } }
         .a { animation: r1143o 1000ms linear infinite; }
         .a.moved { background-position: 30px 0; }",
    );
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(
        node,
        "style",
        &format!("position: absolute; left: 0; top: 0; width: 100px; height: 20px; {RB}; background-size: 40px 20px; background-repeat: no-repeat"),
    );
    doc.append_child(body, node);
    doc.resolve_layout(200.0, 100.0);
    doc.set_attribute(node, "class", "a");
    doc.resolve_layout(201.0, 100.0);
    let t0 = doc.tree.active_animations[&node.0][0].start_time_ms;
    doc.set_attribute(node, "class", "a moved");
    doc.resolve_layout(202.0, 100.0);
    rinch_dom::animation::tick_animations(&mut doc.tree, t0 + 300.0);
    let p = paint_at(&mut doc, 1.0, 200.0, 100.0);
    // Chrome: tile at 30: W 0-30, R 30-50, B 50-70, W 70-100.
    assert_eq!(row_of(&p, 10, &[15, 40, 60, 85]), "WRBW");
}

/// Control for r8: is the same pin pre-existing for `opacity` (a property
/// `extract_base_style_values` already carried before #1143)?
#[test]
#[ignore = "#781 (control for r8)"]
fn r9_control_opacity_is_pinned_the_same_way() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "html, body { margin: 0; }
         @keyframes r1143t { to { transform: translateX(0px); } }
         .a { animation: r1143t 1000ms linear infinite; }
         .a.moved { opacity: 0.5; }",
    );
    let body = doc.body();
    let node = doc.create_element("div");
    doc.set_attribute(node, "style", "width: 100px; height: 20px");
    doc.append_child(body, node);
    doc.resolve_layout(200.0, 100.0);
    doc.set_attribute(node, "class", "a");
    doc.resolve_layout(201.0, 100.0);
    let t0 = doc.tree.active_animations[&node.0][0].start_time_ms;
    doc.set_attribute(node, "class", "a moved");
    doc.resolve_layout(202.0, 100.0);
    rinch_dom::animation::tick_animations(&mut doc.tree, t0 + 300.0);
    assert_eq!(doc.tree.nodes[node.0].computed_style.opacity, 0.5);
}
