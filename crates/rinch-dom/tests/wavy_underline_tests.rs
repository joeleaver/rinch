//! `text-decoration-style: wavy` and `text-decoration-color` — the spellcheck
//! squiggle, end to end.
//!
//! # Why a wave is not a Parley style
//!
//! Parley models a decoration as a brush, an offset and a size — a straight
//! line, stroked in `paint/text.rs::render_text` from the run metrics. There is
//! no wavy variant and no way to express one as a `StyleProperty`. So a wavy
//! underline leaves the style path entirely: `RinchDocument::push_inline_spans`
//! records it as an `InlineDecorationSpan` (a byte range over the flat IFC text,
//! exactly the shape an inline *background* takes) and `paint_wavy_decorations`
//! draws a zigzag under the run.
//!
//! The corollary is the part that would be easy to get wrong twice over: the
//! same element must **not** also push Parley's straight `Underline`, or a
//! squiggle is drawn over a rule. `wavy_is_not_also_a_straight_underline` is the
//! pin for that, and it works by difference against a `solid` twin rather than
//! by counting ink, so it says nothing about the local font.
//!
//! Only *relative* facts are asserted — "the wave covers more rows than the
//! line", "these pixels are the decoration's colour and not the text's" — for
//! the reason `underline_offset_tests.rs` gives: an absolute row or an ink count
//! is a pin on the font set, not on the declaration.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::TextDecorationStyleValue;

const VW: f32 = 400.0;
const VH: f32 = 120.0;
const LINE: &str = "line-height: 40px; font-size: 20px";

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn txt(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

/// A `<span>` carrying `decl`, inside a sized block, laid out.
fn styled_span(decl: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", &format!("width: 400px; {LINE}"));
    let s = el(&mut doc, c, "span", decl);
    txt(&mut doc, s, "recieve");
    doc.resolve_layout(VW, VH);
    (doc, s)
}

// ---------------------------------------------------------------------------
// 1. The cascade
// ---------------------------------------------------------------------------

#[test]
fn the_shorthand_and_the_longhands_both_reach_computed_style() {
    for decl in [
        "text-decoration: underline wavy #d1242f",
        "text-decoration-line: underline; text-decoration-style: wavy; \
         text-decoration-color: #d1242f",
    ] {
        let (doc, s) = styled_span(decl);
        let td = &doc.tree.nodes[s.0].computed_style.text_decoration;
        assert!(td.underline, "`{decl}` lost the line");
        assert_eq!(td.style, TextDecorationStyleValue::Wavy, "`{decl}`");
        assert_eq!(
            td.color.map(|c| c.to_rgba8().to_u32()),
            Some(
                peniko::Color::from_rgb8(0xd1, 0x24, 0x2f)
                    .to_rgba8()
                    .to_u32()
            ),
            "`{decl}` lost the decoration colour"
        );
        assert!(td.is_wavy_underline());
    }
}

#[test]
fn every_decoration_style_keyword_parses_and_only_wavy_is_wavy() {
    for (decl, want) in [
        ("solid", TextDecorationStyleValue::Solid),
        ("double", TextDecorationStyleValue::Double),
        ("dotted", TextDecorationStyleValue::Dotted),
        ("dashed", TextDecorationStyleValue::Dashed),
        ("wavy", TextDecorationStyleValue::Wavy),
    ] {
        let (doc, s) = styled_span(&format!(
            "text-decoration: underline; text-decoration-style: {decl}"
        ));
        let td = &doc.tree.nodes[s.0].computed_style.text_decoration;
        assert_eq!(td.style, want, "text-decoration-style: {decl}");
        assert_eq!(
            td.is_wavy_underline(),
            want == TextDecorationStyleValue::Wavy,
            "text-decoration-style: {decl}"
        );
    }
}

/// `currentcolor` is `text-decoration-color`'s initial value and computes to
/// `None` here **on purpose**: Parley already falls back to the text brush when
/// no decoration brush is pushed, so carrying the text colour instead would only
/// make two elements that agree on everything look like they differ (and cost a
/// spare Parley style span per inline element in the document).
#[test]
fn currentcolor_computes_to_none_rather_than_to_the_text_colour() {
    for decl in [
        "text-decoration: underline",
        "color: #336699; text-decoration: underline",
        "color: #336699; text-decoration: underline; text-decoration-color: currentcolor",
    ] {
        let (doc, s) = styled_span(decl);
        let cs = &doc.tree.nodes[s.0].computed_style;
        assert!(
            cs.text_decoration.underline,
            "`{decl}` is measuring nothing"
        );
        assert_eq!(cs.text_decoration.color, None, "`{decl}`");
    }
    // ...and a real colour is not swallowed by the same path.
    let (doc, s) = styled_span("color: #336699; text-decoration: underline red");
    let cs = &doc.tree.nodes[s.0].computed_style;
    assert_eq!(
        cs.text_decoration.color.map(|c| c.to_rgba8().to_u32()),
        Some(peniko::Color::from_rgb8(255, 0, 0).to_rgba8().to_u32())
    );
}

/// `text-decoration-line` does not inherit, so a wavy underline on a span is one
/// decoration span over that span's range and not one per descendant element —
/// which is what keeps a squiggle from being painted twice over a bold word
/// inside it.
#[test]
fn the_line_does_not_inherit_to_child_elements() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", &format!("width: 400px; {LINE}"));
    let outer = el(&mut doc, c, "span", "text-decoration: underline wavy red");
    let inner = el(&mut doc, outer, "b", "");
    txt(&mut doc, inner, "recieve");
    doc.resolve_layout(VW, VH);

    assert!(
        doc.tree.nodes[outer.0]
            .computed_style
            .text_decoration
            .is_wavy_underline()
    );
    assert!(
        !doc.tree.nodes[inner.0]
            .computed_style
            .text_decoration
            .underline,
        "text-decoration-line inherited to a child, so the squiggle would paint twice"
    );
}

// ---------------------------------------------------------------------------
// 2. The paint
// ---------------------------------------------------------------------------

#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    /// Every painted pixel, as `(x, y, [r, g, b, a])`, skipping the white page.
    fn ink(doc: &mut RinchDocument) -> Vec<(usize, usize, [u8; 4])> {
        let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
            (VW, VH),
            &mut doc.font_cx,
            &mut cx,
        );
        let px: Vec<[u8; 4]> = painter.pixels().as_chunks::<4>().0.to_vec();
        px.iter()
            .enumerate()
            .filter(|(_, p)| p[3] > 0 && !(p[0] > 240 && p[1] > 240 && p[2] > 240))
            .map(|(i, p)| (i % VW as usize, i / VW as usize, *p))
            .collect()
    }

    /// Pixels that are recognisably red rather than the black text — the
    /// decoration, isolated from the glyphs without counting them.
    fn reddish(doc: &mut RinchDocument) -> Vec<(usize, usize)> {
        ink(doc)
            .into_iter()
            .filter(|(_, _, p)| p[0] > 100 && p[0] as i16 - p[1] as i16 > 40)
            .map(|(x, y, _)| (x, y))
            .collect()
    }

    fn rows(pixels: &[(usize, usize)]) -> Vec<usize> {
        let mut ys: Vec<usize> = pixels.iter().map(|&(_, y)| y).collect();
        ys.sort_unstable();
        ys.dedup();
        ys
    }

    /// The whole feature in one fixture: black text, a red decoration, and a wave
    /// that occupies strictly more scanlines than the straight line it replaces.
    #[test]
    fn a_wavy_underline_paints_a_taller_line_than_a_solid_one() {
        let (mut none, _) = styled_span("color: black");
        let (mut solid, _) = styled_span("color: black; text-decoration: underline solid red");
        let (mut wavy, _) = styled_span("color: black; text-decoration: underline wavy red");

        assert!(
            reddish(&mut none).is_empty(),
            "undecorated text painted something red; the colour filter is measuring the glyphs"
        );
        let solid_px = reddish(&mut solid);
        let wavy_px = reddish(&mut wavy);
        assert!(
            !solid_px.is_empty(),
            "`text-decoration-color: red` never reached the painter"
        );
        assert!(
            !wavy_px.is_empty(),
            "a wavy underline painted nothing at all"
        );

        let (solid_rows, wavy_rows) = (rows(&solid_px), rows(&wavy_px));
        assert!(
            wavy_rows.len() > solid_rows.len(),
            "the wave is no taller than a straight rule \
             (wavy rows {wavy_rows:?} vs solid rows {solid_rows:?}) — it is probably being \
             drawn as a straight line"
        );
        // And it sits under the text, not through it: every wave pixel is below
        // the top of the straight underline it replaces.
        assert!(
            wavy_rows[wavy_rows.len() - 1] >= solid_rows[solid_rows.len() - 1],
            "the wave rides above the underline position"
        );
    }

    /// The trap: a wavy element that *also* pushed Parley's `Underline(true)`
    /// would paint both, and every ink assertion above would still pass. Measured
    /// by difference against the solid twin — the wave must not contain the
    /// straight rule's own rows *solidly*, i.e. the wave leaves gaps in the row a
    /// straight line fills edge to edge.
    #[test]
    fn wavy_is_not_also_a_straight_underline() {
        let (mut solid, _) = styled_span("color: black; text-decoration: underline solid red");
        let (mut wavy, _) = styled_span("color: black; text-decoration: underline wavy red");
        let solid_px = reddish(&mut solid);
        let wavy_px = reddish(&mut wavy);

        // The row a straight rule fills is its widest; count the wave's widest row.
        let widest = |px: &[(usize, usize)]| {
            rows(px)
                .into_iter()
                .map(|y| px.iter().filter(|&&(_, yy)| yy == y).count())
                .max()
                .unwrap_or(0)
        };
        assert!(
            widest(&wavy_px) < widest(&solid_px),
            "the wave's densest scanline is as full as a straight rule's, so a \
             straight underline is being drawn underneath it"
        );
    }

    /// A wavy underline declared on the **block** (the IFC root itself) is drawn
    /// too — it is pushed over the whole flat text after the walk, not by the
    /// inline-element path.
    #[test]
    fn the_ifc_root_gets_its_own_wave() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            &format!("width: 400px; {LINE}; color: black; text-decoration: underline wavy red"),
        );
        txt(&mut doc, c, "recieve");
        doc.resolve_layout(VW, VH);
        assert!(
            !reddish(&mut doc).is_empty(),
            "a wavy underline on the IFC root painted nothing"
        );
    }
}
