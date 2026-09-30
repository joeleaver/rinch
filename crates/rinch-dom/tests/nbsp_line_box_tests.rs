//! A non-breaking space is not collapsible white space (#1154).
//!
//! CSS Text 3 §4.1.1 collapses and removes only *document white space* —
//! spaces, tabs and segment breaks. U+00A0 NO-BREAK SPACE, and every other
//! Unicode `White_Space` character that is not ASCII (U+2009 THIN SPACE,
//! U+3000 IDEOGRAPHIC SPACE, …), is ordinary text: kept at the start and the
//! end of a block, and enough on its own to make a line box. `<div>&nbsp;</div>`
//! is one line tall in every browser — the spacer idiom rests on it.
//!
//! parley 0.11.1's `WhiteSpaceCollapse::Collapse` trims a span's start and end
//! with `str::trim_start`/`trim_end`, which remove every Unicode `White_Space`
//! character, so an NBSP-only block had no text and no line (0 tall), and an
//! NBSP at the start or end of a paragraph vanished from its width.
//!
//! Every number is Chrome 153's on the same markup (`* { margin: 0 }`). The
//! heights declare their `line-height`; the widths use the bundled Inter,
//! registered under an override name so the host's fonts cannot answer, and
//! Taffy rounds box edges to whole pixels, hence the half-pixel tolerance.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn doc() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    doc
}

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

fn height(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.height
}

fn width(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.width
}

/// A 300px container with a 20px line, as in the issue's table.
fn container(doc: &mut RinchDocument) -> NodeId {
    let body = doc.body();
    el(
        doc,
        body,
        "div",
        "width: 300px; font-size: 10px; line-height: 20px",
    )
}

/// Chrome 153: 20 for each — one line box. Before the fix every one was 0.
/// The collapsible spaces around the NBSP go (a `<div>   </div>` is 0, kept
/// here as the control that collapsible white space still makes no line).
#[test]
fn a_block_holding_only_non_collapsible_spaces_is_one_line_tall() {
    let cases: &[(&str, &[&str], f32)] = &[
        ("nbsp", &["\u{a0}"], 20.0),
        ("two nbsp", &["\u{a0}\u{a0}"], 20.0),
        ("nbsp between collapsible spaces", &[" \u{a0} "], 20.0),
        ("thin space", &["\u{2009}"], 20.0),
        ("ideographic space", &["\u{3000}"], 20.0),
        ("collapsible spaces only (control)", &["   "], 0.0),
    ];
    for (name, parts, want) in cases {
        let mut doc = doc();
        let c = container(&mut doc);
        let block = el(&mut doc, c, "div", "");
        for p in *parts {
            text(&mut doc, block, p);
        }
        doc.resolve_layout(800.0, 600.0);
        assert_eq!(height(&doc, block), *want, "{name}: Chrome 153 gives {want}");
    }
}

/// The same inside an inline element, which is a span of its own to parley
/// (its end trims too): `<div><span>&nbsp;</span></div>`. Chrome 153: 20.
#[test]
fn an_nbsp_inside_an_inline_element_makes_a_line() {
    let mut doc = doc();
    let c = container(&mut doc);
    let block = el(&mut doc, c, "div", "");
    let span = el(&mut doc, block, "span", "");
    text(&mut doc, span, "\u{a0}");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(height(&doc, block), 20.0, "Chrome 153: 20");
}

/// An NBSP at a paragraph's start or end keeps its advance. A shrink-to-fit
/// `inline-block` at Inter 16px; Chrome 153's widths, where `x` is 8.73 and a
/// space or an NBSP 4.5. The collapsible spaces beside an NBSP at an edge
/// still go (leading) or hang (trailing), so each of the middle four is `x`,
/// one NBSP and one space: 17.73.
#[test]
fn an_nbsp_at_an_edge_of_a_paragraph_keeps_its_width() {
    let cases: &[(&str, &[&str], f32)] = &[
        ("x (control)", &["x"], 8.73),
        ("x nbsp", &["x\u{a0}"], 13.23),
        ("nbsp x", &["\u{a0}x"], 13.23),
        ("space nbsp space x", &[" \u{a0} x"], 17.73),
        ("x space nbsp space", &["x \u{a0} "], 17.73),
        // Two text nodes: the space after the NBSP is not at the start of
        // anything — CSS keeps it, and the split must not make it look so.
        ("nbsp | space x", &["\u{a0}", " x"], 17.73),
        ("nbsp only", &["\u{a0}"], 4.5),
        ("thin space x", &["\u{2009}x"], 11.64),
        ("x nbsp x (interior, control)", &["x\u{a0}x"], 21.97),
    ];
    for (name, parts, want) in cases {
        let mut doc = doc();
        let body = doc.body();
        let c = el(&mut doc, body, "div", "width: 300px; font: 16px/25px ProbeFace");
        let ib = el(&mut doc, c, "span", "display: inline-block");
        for p in *parts {
            text(&mut doc, ib, p);
        }
        doc.resolve_layout(800.0, 600.0);
        let w = width(&doc, ib);
        assert!(
            (w - want).abs() <= 0.5 + 1e-3,
            "{name}: Chrome 153 gives {want}, rinch {w}"
        );
        assert_eq!(height(&doc, ib), 25.0, "{name}: one 25px line");
    }
}
