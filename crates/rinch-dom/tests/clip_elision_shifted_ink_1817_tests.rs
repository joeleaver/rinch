//! An `overflow: hidden` box clips text shifted by `vertical-align`, a shifted
//! element's background, and a wavy underline, whatever its height (#1817).
//!
//! Paint does not push a clip that provably cuts nothing (card K43,
//! `layer_bounds::clip_cuts_nothing`). That proof measured an inline
//! formatting context root's content as its **unshifted** line boxes plus its
//! inline backgrounds (#1762). Text that `vertical-align` moves off its line
//! (#724), the background around such text, and a wavy underline's wave are
//! drawn past those, so in a cell exactly as tall as its line the clip was
//! elided and they painted outside the cell.
//!
//! Every assertion reads the software painter's pixels in the strips just
//! above, below and left of the clipping box, where the correct output is
//! provably empty (the document has nothing else in it). A positive control
//! paints each document with `overflow: visible` and requires ink in those
//! strips, so a case cannot pass by having nothing to clip. Heights sit on
//! both sides of the 18px line and off whole pixels, at scales 1 and 2.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 240.0;
const VH: f32 = 200.0;

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

const LEFT: f64 = 40.0;
const TOP: f64 = 70.0;

/// Each case: a name, the inner span's style, and its text. The cell holds
/// `x` then the span, so the line has unshifted text too.
const CASES: [(&str, &str); 6] = [
    ("sup", "vertical-align: super"),
    ("up 12px", "vertical-align: 12px"),
    ("down 12px", "vertical-align: -12px"),
    (
        "sup pill",
        "vertical-align: super; background-color: rgb(0, 128, 0); padding: 1px 3px",
    ),
    (
        "sub bordered",
        "vertical-align: sub; border: 2px solid rgb(0, 0, 200)",
    ),
    (
        "wavy",
        "text-decoration: underline wavy rgb(200, 0, 0); text-underline-offset: 0px",
    ),
];

fn doc_with_cell(height: f32, overflow: &str, span_style: &str) -> (RinchDocument, NodeId) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(INTER)),
        Some(FontInfoOverride {
            family_name: Some("ShiftInter"),
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
             font-family: ShiftInter; font-size: 14px; line-height: 18px"
        ),
    );
    doc.append_child(body, cell);
    let lead = doc.create_text("x ");
    doc.append_child(cell, lead);
    let span = doc.create_element("span");
    doc.set_attribute(span, "style", span_style);
    doc.append_child(cell, span);
    let t = doc.create_text("Agy");
    doc.append_child(span, t);
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

/// Pixels with any alpha in `[x0, x1) x [y0, y1)` (device px).
fn ink_in(p: &TinySkiaPainter, (x0, x1, y0, y1): (u32, u32, u32, u32)) -> usize {
    let d = p.pixels();
    let mut n = 0;
    for y in y0..y1.min(p.height()) {
        for x in x0..x1.min(p.width()) {
            let i = ((y * p.width() + x) * 4) as usize;
            if d[i + 3] > 0 {
                n += 1;
            }
        }
    }
    n
}

/// The ink in the strips above, below and left of the cell's box.
fn outside(doc: &mut RinchDocument, cell: NodeId, scale: f64) -> (usize, usize, usize) {
    let h = doc.tree.get(cell.0).unwrap().layout.height as f64;
    let p = paint(doc, scale);
    let s = |v: f64| (v * scale).round() as u32;
    let above = ink_in(&p, (s(LEFT - 10.0), s(LEFT + 130.0), s(TOP - 40.0), s(TOP)));
    let below = ink_in(
        &p,
        (s(LEFT - 10.0), s(LEFT + 130.0), s(TOP + h), s(TOP + h + 40.0)),
    );
    let left = ink_in(&p, (s(LEFT - 10.0), s(LEFT), s(TOP), s(TOP + h)));
    (above, below, left)
}

const HEIGHTS: [f32; 4] = [17.6, 18.0, 18.4, 21.3];

#[test]
fn each_case_overflows_a_visible_cell() {
    for scale in [1.0, 2.0] {
        for (name, style) in CASES {
            // The tallest sampled cell: a case that overflows it overflows
            // every shorter one too.
            let (mut doc, cell) = doc_with_cell(18.0, "visible", style);
            let (above, below, _) = outside(&mut doc, cell, scale);
            assert!(
                above + below > 0,
                "{name} scale {scale}: nothing reaches past an 18px cell (above {above}, below {below})"
            );
        }
    }
}

#[test]
fn a_hidden_cell_clips_shifted_ink_at_every_height() {
    let mut failures = Vec::new();
    for scale in [1.0, 2.0] {
        for (name, style) in CASES {
            for h in HEIGHTS {
                let (mut doc, cell) = doc_with_cell(h, "hidden", style);
                let got = outside(&mut doc, cell, scale);
                if got != (0, 0, 0) {
                    failures.push(format!(
                        "{name} height {h} scale {scale}: (above, below, left) = {got:?}"
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "ink painted outside the clip:\n{}",
        failures.join("\n")
    );
}
