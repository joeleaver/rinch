//! Out-of-flow boxes and the inline formatting context (#406, #289).
//!
//! CSS 2.1 is unambiguous: an out-of-flow box (§9.3) is neither inline content
//! nor in-flow block content. Per §9.2.1.1 it does not force anonymous block
//! box generation, and per §9.4.2 it does not break an inline formatting
//! context — its inline siblings carry on across it, on the same line.
//!
//! `ifc.rs` had **no** out-of-flow awareness at all: grepping it for
//! `PositionValue` or `out_of_flow` returned nothing. Out-of-flow children were
//! excluded from IFC decisions only *incidentally*, because Stylo blockifies
//! them and so `is_inline()` answers `false` — which is the right answer at the
//! sites that ask "is this inline content" and the **wrong** one at the sites
//! that ask "does this break the flow".
//!
//! The oracle here is the rasterised pixmap, following
//! `paint_layout_agreement_tests.rs`: a unique opaque fill per box, exact bbox,
//! no tolerance. Two helpers agreeing with each other proves nothing when the
//! failure mode is *both walks being wrong together*, which is this whole
//! class. Where the interesting claim is "this text is drawn at all", the
//! assertion is on ink, not on a layout number.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn child_of(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let el = doc.create_element(tag);
    doc.set_attribute(el, "style", style);
    doc.append_child(parent, el);
    el
}

fn text_in(doc: &mut RinchDocument, parent: NodeId, text: &str) -> NodeId {
    let t = doc.create_text(text);
    doc.append_child(parent, t);
    t
}

fn rasterize(doc: &mut RinchDocument) -> Vec<u8> {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.pixels().to_vec()
}

/// Bounding box of every pixel painted in exactly `rgb`, `(x0, y0, x1, y1)`
/// with the maxima exclusive.
fn color_bbox(px: &[u8], rgb: (u8, u8, u8)) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (VW as u32, VH as u32);
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            if (px[i], px[i + 1], px[i + 2]) == rgb {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 == u32::MAX {
        None
    } else {
        Some((x0, y0, x1, y1))
    }
}

fn anon_box_count(doc: &RinchDocument) -> usize {
    doc.tree.anonymous_block_boxes.len()
}

fn height_of(doc: &RinchDocument, node: NodeId) -> f32 {
    doc.tree.get(node.0).unwrap().layout.height
}

// ── #406: an out-of-flow child does not mint an anonymous block box ─────────

/// The layout consequence, and the one a user sees. The run-grouping loop
/// terminated the current run on **any** non-inline child, so text either side
/// of an absolute box became two runs → two anonymous boxes → two lines.
#[test]
fn text_either_side_of_an_absolute_child_stays_on_one_line() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let row = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, row, "aaa ");
    child_of(
        &mut doc,
        row,
        "div",
        "position: absolute; top: 0; right: 0; width: 10px; height: 10px",
    );
    text_in(&mut doc, row, " bbb");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        anon_box_count(&doc),
        1,
        "one run, so one anonymous box — not two. (That there is a box at all \
         is #406's remaining half, tracked as #466 and blocked on the leaf \
         invariant; what this pins is that the run is not *split*.)"
    );
    let h = height_of(&doc, row);
    assert!(
        (h - 20.0).abs() < 1.0,
        "both text runs share one line box, so the row is one line tall, got {h}"
    );
}

// ── the trap: fixing `has_block` alone strands the text ─────────────────────
//
// These two are a **canary for #466**, not evidence the trap is gone. They are
// green today *because* the anonymous-box split keeps each run in its own IFC;
// #466 removes those boxes, and the moment it does, these are what fail first.
//
// `walk_inline_children`'s catch-all arm is `break`. An out-of-flow child is
// blockified, so it lands there. Today the anonymous-box split hides that,
// because each run becomes its own IFC. Remove the anonymous boxes and the
// container becomes one IFC whose walk **stops at the absolute box** — and
// `mark_inline_descendants` has already Taffy-detached the text, so it has no
// box either and simply vanishes.
//
// These two assert on ink for that reason: a layout number would still look
// plausible for text that is never drawn.

/// Text *after* an absolute child is still painted.
#[test]
fn text_after_an_absolute_child_is_still_painted() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let row = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 40px; line-height: 48px; width: 700px; \
         color: rgb(0, 0, 255)",
    );
    text_in(&mut doc, row, "AAAA");
    child_of(
        &mut doc,
        row,
        "div",
        "position: absolute; top: 0; right: 0; width: 10px; height: 10px",
    );
    text_in(&mut doc, row, "BBBB");
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    let bbox = color_bbox(&px, (0, 0, 255)).expect("the row's text is painted at all");
    // Both runs are on one line, so the painted text is wider than one run.
    let width = bbox.2 - bbox.0;
    assert!(
        width > 120,
        "both AAAA and BBBB are drawn, so the inked span is wide; got {width}px \
         — a narrow span means the walk stopped at the absolute box"
    );
}

/// Text whose *only* preceding sibling is an absolute child is painted.
/// `<div><abs/>hello</div>` renders empty if the walk breaks at the box.
#[test]
fn text_after_a_leading_absolute_child_is_painted() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let row = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 40px; line-height: 48px; width: 700px; \
         color: rgb(0, 128, 0)",
    );
    child_of(
        &mut doc,
        row,
        "div",
        "position: absolute; top: 0; right: 0; width: 10px; height: 10px",
    );
    text_in(&mut doc, row, "HELLO");
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    assert!(
        color_bbox(&px, (0, 128, 0)).is_some(),
        "text preceded only by an out-of-flow box must still be drawn"
    );
}

// ── guard rails ────────────────────────────────────────────────────────────

/// Genuine mixed content still mints an anonymous box. The predicate must
/// narrow the rule, not delete it.
#[test]
fn genuine_mixed_content_still_mints_an_anonymous_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(&mut doc, body, "div", "font-size: 16px; width: 400px");
    text_in(&mut doc, container, "inline text");
    child_of(&mut doc, container, "div", "height: 10px");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        anon_box_count(&doc),
        1,
        "an in-flow block sibling is still mixed content"
    );
}

/// An in-flow block-level child still breaks the inline flow — the `break` in
/// `walk_inline_children` must survive for everything that is not out of flow.
#[test]
fn an_in_flow_block_child_still_breaks_the_line() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, container, "aaa");
    child_of(&mut doc, container, "div", "height: 30px");
    text_in(&mut doc, container, "bbb");
    doc.resolve_layout(VW, VH);

    let h = height_of(&doc, container);
    assert!(
        h > 50.0,
        "two line boxes plus a 30px block — the block still breaks the flow, got {h}"
    );
}

/// A `position: static` child is in flow and unaffected: the predicate keys on
/// `position`, and `Static` is the default, so text nodes (which never get
/// style resolution) answer `false` — the #342 hazard.
#[test]
fn a_static_child_is_unaffected() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(&mut doc, body, "div", "font-size: 16px; width: 400px");
    text_in(&mut doc, container, "inline");
    child_of(&mut doc, container, "div", "position: static; height: 10px");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        anon_box_count(&doc),
        1,
        "static is in flow — still mixed content"
    );
}
