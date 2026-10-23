//! An `overflow: hidden` box clips the background of an inline element that
//! reaches past it, whatever its height (#1762).
//!
//! Paint does not push a clip that provably cuts nothing (card K43,
//! `layer_bounds::clip_cuts_nothing`). That proof measured an inline
//! formatting context root's content as its text's line boxes and left out
//! the rectangles `paint_inline_backgrounds` draws for an inline element's
//! background, which reach past the line box by the element's padding — on
//! the block axis always (an inline's vertical padding takes no room, in CSS
//! too), and on the inline axis while #1426 stands. So in a column of
//! identical cells the clip was elided, and the pill's padding painted
//! outside the cell, in exactly the cells whose text happened to fit: those
//! whose height rounded up to the line, not those whose height rounded below
//! it (where the text itself reached past the box and the clip was kept).
//!
//! Every assertion reads pixels of the software painter in a region where the
//! correct output is provably free of green: the strips just above, below and
//! left of the clipping box. A positive control paints the same document with
//! `overflow: visible` and requires green in those strips, so the fixture
//! cannot pass by having nothing overflow. Heights are sampled on both sides
//! of the line height and off whole pixels (18.4 rounds to 18, 18.6 to 19),
//! at scales 1 and 2.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 240.0;
const VH: f32 = 160.0;

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// The box's left and top edges (CSS px).
const LEFT: f64 = 40.0;
const TOP: f64 = 50.0;

fn doc_with_cell(height: f32, overflow: &str) -> (RinchDocument, NodeId) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(INTER)),
        Some(FontInfoOverride {
            family_name: Some("PillInter"),
            ..Default::default()
        }),
    );
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let cell = doc.create_element("div");
    doc.set_attribute(
        cell,
        "style",
        &format!(
            "position: absolute; left: {LEFT}px; top: {TOP}px; width: 120px; \
             height: {height}px; overflow: {overflow}; white-space: nowrap; \
             font-family: PillInter; font-size: 14px; line-height: 18px"
        ),
    );
    doc.append_child(body, cell);
    let pill = doc.create_element("span");
    doc.set_attribute(
        pill,
        "style",
        "background-color: rgb(0, 128, 0); padding: 6px 5px; border-radius: 0",
    );
    doc.append_child(cell, pill);
    let t = doc.create_text("Ag pill");
    doc.append_child(pill, t);
    doc.resolve_layout(VW, VH);
    (doc, cell)
}

fn paint(doc: &mut RinchDocument, scale: f64) -> TinySkiaPainter {
    let mut painter = TinySkiaPainter::new((VW as f64 * scale) as u32, (VH as f64 * scale) as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter
}

/// Pixels with any green in `[x0, x1) x [y0, y1)` (device px).
fn green_in(p: &TinySkiaPainter, (x0, x1, y0, y1): (u32, u32, u32, u32)) -> usize {
    let d = p.pixels();
    let mut n = 0;
    for y in y0..y1.min(p.height()) {
        for x in x0..x1.min(p.width()) {
            let i = ((y * p.width() + x) * 4) as usize;
            if d[i + 1] > 0 && d[i] == 0 {
                n += 1;
            }
        }
    }
    n
}

/// The green pixels in the strips above, below and left of the cell's box.
fn outside(doc: &mut RinchDocument, cell: NodeId, scale: f64) -> (usize, usize, usize) {
    let h = doc.tree.get(cell.0).unwrap().layout.height as f64;
    let p = paint(doc, scale);
    let s = |v: f64| (v * scale).round() as u32;
    let above = green_in(&p, (s(LEFT), s(LEFT + 120.0), s(TOP - 10.0), s(TOP)));
    let below = green_in(
        &p,
        (s(LEFT), s(LEFT + 120.0), s(TOP + h), s(TOP + h + 10.0)),
    );
    let left = green_in(&p, (s(LEFT - 10.0), s(LEFT), s(TOP), s(TOP + h)));
    (above, below, left)
}

const HEIGHTS: [f32; 8] = [12.6, 17.6, 18.0, 18.4, 18.6, 19.0, 19.4, 23.3];

#[test]
fn the_pill_overflows_a_visible_cell() {
    // Positive control: without the clip the padding reaches past the box on
    // both block-axis sides at every height sampled.
    for scale in [1.0, 2.0] {
        for h in HEIGHTS {
            let (mut doc, cell) = doc_with_cell(h, "visible");
            let (above, below, _left) = outside(&mut doc, cell, scale);
            assert!(
                above > 0 || below > 0,
                "height {h} scale {scale}: the pill does not overflow (above {above}, below {below})"
            );
        }
    }
}

#[test]
fn a_hidden_cell_clips_the_pill_at_every_height() {
    let mut failures = Vec::new();
    for scale in [1.0, 2.0] {
        for h in HEIGHTS {
            let (mut doc, cell) = doc_with_cell(h, "hidden");
            let got = outside(&mut doc, cell, scale);
            if got != (0, 0, 0) {
                failures.push(format!(
                    "height {h} scale {scale}: (above, below, left) = {got:?}"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "green painted outside the clip:\n{}",
        failures.join("\n")
    );
}

/// The bounding box of the opaque pill-green pixels, in device px.
fn green_bbox(p: &TinySkiaPainter) -> (u32, u32, u32, u32) {
    let d = p.pixels();
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..p.height() {
        for x in 0..p.width() {
            let i = ((y * p.width() + x) * 4) as usize;
            if d[i] == 0 && d[i + 1] == 128 && d[i + 2] == 0 && d[i + 3] == 255 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x0, y0, x1, y1)
}

#[test]
fn an_inline_background_scales_with_the_display() {
    // The geometry the clip proof reads is the geometry paint draws, so it
    // has to be right at a scale other than 1: the background used to be
    // drawn in CSS px into the device-px target, half its size and off its
    // text at scale 2.
    let (mut doc, _) = doc_with_cell(18.4, "visible");
    let one = green_bbox(&paint(&mut doc, 1.0));
    let two = green_bbox(&paint(&mut doc, 2.0));
    let doubled = (one.0 * 2, one.1 * 2, one.2 * 2, one.3 * 2);
    let near = |a: u32, b: u32| a.abs_diff(b) <= 2;
    assert!(
        near(two.0, doubled.0)
            && near(two.1, doubled.1)
            && near(two.2, doubled.2)
            && near(two.3, doubled.3),
        "scale 1 bbox {one:?}, scale 2 bbox {two:?}, expected about {doubled:?}"
    );
}
