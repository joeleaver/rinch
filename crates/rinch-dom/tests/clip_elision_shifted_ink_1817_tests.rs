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
        (
            s(LEFT - 10.0),
            s(LEFT + 130.0),
            s(TOP + h),
            s(TOP + h + 40.0),
        ),
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

/// The bounding box of the opaque `rgb(0, 128, 0)` pixels, in device px.
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
fn a_shifted_element_s_background_moves_with_its_text() {
    // An element's box, background included, is where its `vertical-align`
    // puts its text (CSS 2.1 §10.8): a `<sup>` pill is around the raised
    // text. It used to stay on the line while the text moved.
    const PILL: &str = "background-color: rgb(0, 128, 0); padding: 1px 3px";
    for scale in [1.0, 2.0] {
        let (mut doc, _) = doc_with_cell(18.0, "visible", PILL);
        let on_line = green_bbox(&paint(&mut doc, scale));
        for shift in [12.0_f64, -7.0] {
            let (mut doc, _) = doc_with_cell(
                18.0,
                "visible",
                &format!("vertical-align: {shift}px; {PILL}"),
            );
            let moved = green_bbox(&paint(&mut doc, scale));
            let dy = moved.1 as f64 - on_line.1 as f64;
            assert!(
                (dy + shift * scale).abs() <= 1.0 && moved.0 == on_line.0,
                "shift {shift} scale {scale}: on the line {on_line:?}, shifted {moved:?}"
            );
        }
    }
}

/// `x <outer>a<inner>b</inner>c</outer>`, the inner one carrying the green
/// pill, in a visible 18px cell.
fn nested_doc(outer: &str, inner: &str) -> RinchDocument {
    let (mut doc, cell) = doc_with_cell(18.0, "visible", outer);
    // Replace the span's text with `a`, the pill, `c`.
    let span = doc.tree.get(cell.0).unwrap().children[1];
    let span = NodeId(span);
    doc.set_text_content(span, "");
    let a = doc.create_text("a");
    doc.append_child(span, a);
    let pill = doc.create_element("span");
    doc.set_attribute(
        pill,
        "style",
        &format!("{inner}; background-color: rgb(0, 128, 0); padding: 0 2px"),
    );
    doc.append_child(span, pill);
    let b = doc.create_text("b");
    doc.append_child(pill, b);
    let c = doc.create_text("c");
    doc.append_child(span, c);
    doc.resolve_layout(VW, VH + 1.0);
    doc
}

/// The dark (glyph) pixels between the pill's left and right edges: their
/// vertical extent, in device px.
fn glyph_rows_in_pill(p: &TinySkiaPainter, pill: (u32, u32, u32, u32)) -> (u32, u32) {
    let d = p.pixels();
    let (mut y0, mut y1) = (u32::MAX, 0);
    for y in 0..p.height() {
        for x in pill.0..pill.2 {
            let i = ((y * p.width() + x) * 4) as usize;
            if d[i + 3] > 200 && d[i] < 60 && d[i + 1] < 60 && d[i + 2] < 60 {
                y0 = y0.min(y);
                y1 = y1.max(y + 1);
            }
        }
    }
    (y0, y1)
}

#[test]
fn a_nested_shifted_element_s_background_is_where_its_text_is() {
    // `vertical-align` is not chained (#724: the text takes the innermost
    // shift). The background takes the same element's shift — the nearest
    // from its own element up — so the pill is around its glyph. Both
    // nestings, and a pill on an element of its own inside a shifted one.
    for (outer, inner) in [
        ("vertical-align: super", "vertical-align: sub"),
        ("vertical-align: sub", "vertical-align: 9px"),
        ("vertical-align: -10px", "font-style: normal"),
    ] {
        for scale in [1.0, 2.0] {
            let mut doc = nested_doc(outer, inner);
            let p = paint(&mut doc, scale);
            let pill = green_bbox(&p);
            assert!(pill.0 < pill.2, "{outer} / {inner}: no pill painted");
            let glyph = glyph_rows_in_pill(&p, pill);
            assert!(glyph.0 < glyph.1, "{outer} / {inner}: no glyph in the pill");
            assert!(
                glyph.0 + 1 >= pill.1 && glyph.1 <= pill.3 + 1,
                "{outer} / {inner} scale {scale}: glyph rows {glyph:?} outside pill {pill:?}"
            );
        }
    }
}
