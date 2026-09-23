//! Performance regression scenarios on a bare `RinchDocument`: one per
//! **increment site** of the counters that have more than one (#879).
//!
//! `perf_counter_baselines.rs` pins each counter on the paths that matter for
//! responsiveness, and its positive control checks that every counter fires
//! *somewhere*. That is per-counter, not per-site: `shape_paint` is bumped at
//! three sites, and deleting the `<select>` label's increment survived the
//! whole suite, because the other two still fired. Each scenario here is a
//! document holding **only** the thing that reaches one site, so the frame's
//! exact value of that counter comes from that site alone, and deleting its
//! increment fails exactly one scenario.
//!
//! | counter | site | scenario |
//! |---|---|---|
//! | `shape_paint` | `paint/select.rs` (closed `<select>` label) | [`a_select_label_is_shaped_by_paint`] |
//! | `shape_paint` | `paint/contenteditable.rs` (`<input>` value) | [`an_input_value_is_shaped_by_paint`] |
//! | `shape_paint` | `paint/mod.rs` (text with no cached layout) | [`an_inline_flex_label_is_reshaped_by_every_paint`] — no `perf_counter_baselines` scenario reaches it (their chips are `inline-block`, whose text is an IFC) |
//! | `ellipsis_builds` | `ifc.rs`, IFC root | [`an_ifc_root_ellipsis`] |
//! | `ellipsis_builds` | `ifc.rs`, text leaf | [`a_text_leaf_ellipsis`] |
//! | `shape_atomic_inline` | `ifc.rs`, `NodeContext::InlineRoot` | [`an_inline_block_holding_an_ifc`] |
//! | `shape_atomic_inline` | `ifc.rs`, `NodeContext::Text` | [`an_inline_flex_holding_a_text_leaf`] |
//! | `pseudo_element_passes` | `resolve.rs`, `::before` | [`only_before_rules`] |
//! | `pseudo_element_passes` | `resolve.rs`, `::after` | [`only_after_rules`] |
//!
//! Every frame is asserted whole, #877's contract: every non-timing counter
//! exact, anything unlisted `0` (`support/perf_expect.rs`). A failure prints the
//! frame it saw, ready to paste.
//!
//! No number here moves with the host's font set, and that takes two things.
//! Line boxes are declared (`line-height`, fixed sizes), which pins the
//! vertical axis. Every run of text is set in the bundled Inter
//! (`assets/fonts/Inter-Regular.ttf`), which pins the horizontal one. A
//! declared line box alone does **not** make a width or a repainted area
//! font-independent: glyph advances still come from whatever font answered.

#![cfg(feature = "software-renderer")]

#[path = "support/perf_expect.rs"]
mod perf_expect;

use perf_expect::expect;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::perf::{Counter::*, FrameStats};

const VP: (f32, f32) = (400.0, 300.0);

/// Every run of text is set in this bundled face, registered under a name of
/// its own, so no horizontal extent depends on the host's fonts.
const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

const BASE_CSS: &str = "body, input, select, button { font-family: PerfInter; }
    body { font-size: 16px; line-height: 20px; }";

fn doc_with(css: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(INTER)),
        Some(FontInfoOverride {
            family_name: Some("PerfInter"),
            ..Default::default()
        }),
    );
    doc.load_css(BASE_CSS);
    if !css.is_empty() {
        doc.load_css(css);
    }
    doc
}

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, class: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !class.is_empty() {
        doc.set_attribute(e, "class", class);
    }
    doc.append_child(parent, e);
    e
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) -> NodeId {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
    t
}

fn paint(doc: &mut RinchDocument) {
    let mut painter = TinySkiaPainter::new(VP.0 as u32, VP.1 as u32);
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        VP,
        &mut doc.font_cx,
        &mut doc.layout_cx,
    );
}

/// Everything from an empty document to its first painted frame: the build,
/// two layouts (the second settles) and one full paint. The counters were
/// zeroed before the first node was created.
fn cold_frame(doc: &mut RinchDocument) -> FrameStats {
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    paint(doc);
    doc.tree.perf.end_frame()
}

/// A settled document painted again in full with nothing changed: layout
/// early-returns, and what is left is what paint does on every frame.
fn repaint_frame(doc: &mut RinchDocument) -> FrameStats {
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    paint(doc);
    doc.tree.perf.reset();
    doc.resolve_layout(VP.0, VP.1);
    paint(doc);
    doc.tree.perf.end_frame()
}

// ── shape_paint ────────────────────────────────────────────────────────────

/// A closed `<select>` paints its selected option's label, shaped by paint
/// every frame (`paint/select.rs`). The label is the only text on the page, so
/// `shape_paint` is exactly 1 and all of it comes from that site.
#[test]
fn a_select_label_is_shaped_by_paint() {
    let mut doc = doc_with("select { width: 120px; height: 24px; }");
    let body = doc.body();
    let sel = el(&mut doc, body, "select", "");
    for label in ["Apple", "Banana"] {
        let o = el(&mut doc, sel, "option", "");
        text(&mut doc, o, label);
    }
    let s = repaint_frame(&mut doc);
    expect(
        "select label repaint",
        &s,
        &[
            (ShapePaint, 1),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 4),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// An `<input>`'s value is shaped by paint every frame
/// (`paint/contenteditable.rs`).
#[test]
fn an_input_value_is_shaped_by_paint() {
    let mut doc = doc_with("input { width: 120px; height: 24px; }");
    let body = doc.body();
    let input = el(&mut doc, body, "input", "");
    doc.set_attribute(input, "value", "typed");
    let s = repaint_frame(&mut doc);
    expect(
        "input value repaint",
        &s,
        &[
            (ShapePaint, 1),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

// ── ellipsis_builds ────────────────────────────────────────────────────────

/// `text-overflow: ellipsis` on a block that is an IFC root: the truncation is
/// built by the IFC layout pass.
#[test]
fn an_ifc_root_ellipsis() {
    let mut doc = doc_with(
        ".clip { width: 60px; overflow: hidden; white-space: nowrap;
                 text-overflow: ellipsis; }",
    );
    doc.tree.perf.reset();
    let body = doc.body();
    let clip = el(&mut doc, body, "div", "clip");
    text(&mut doc, clip, "a line much too long for sixty pixels");
    let s = cold_frame(&mut doc);
    expect(
        "ellipsis (IFC root)",
        &s,
        &[
            (StyleResolves, 2),
            (ElementsCascaded, 3),
            (StyleNodesVisited, 7),
            (FullStyleWalks, 2),
            (TaffyStyleSyncs, 5),
            (TaffyStyleChanges, 3),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (EllipsisBuilds, 1),
            (IfcMeasureCacheHits, 1),
            (IfcMeasureInvalidations, 2),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 2),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// The same truncation on a text node that is **not** in an IFC — the direct
/// text child of a flex container, laid out as a leaf — takes the text-leaf
/// rebuild instead.
#[test]
fn a_text_leaf_ellipsis() {
    let mut doc = doc_with(
        ".clip { display: flex; width: 60px; overflow: hidden; white-space: nowrap;
                 text-overflow: ellipsis; }",
    );
    doc.tree.perf.reset();
    let body = doc.body();
    let clip = el(&mut doc, body, "div", "clip");
    text(&mut doc, clip, "a line much too long for sixty pixels");
    let s = cold_frame(&mut doc);
    expect(
        "ellipsis (text leaf)",
        &s,
        &[
            (StyleResolves, 2),
            (ElementsCascaded, 3),
            (StyleNodesVisited, 7),
            (FullStyleWalks, 2),
            (TaffyStyleSyncs, 5),
            (TaffyStyleChanges, 2),
            (ShapeMeasureText, 4),
            (EllipsisBuilds, 1),
            (IfcMeasureInvalidations, 2),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 4),
            (PaintNodesVisited, 3),
            (StackingOrderBuilds, 1),
        ],
    );
}

// ── shape_atomic_inline ────────────────────────────────────────────────────

/// An `inline-block` whose content is an inline formatting context: its
/// standalone compute measures it through `NodeContext::InlineRoot`.
#[test]
fn an_inline_block_holding_an_ifc() {
    let mut doc = doc_with(".chip { display: inline-block; padding: 2px; }");
    doc.tree.perf.reset();
    let body = doc.body();
    let p = el(&mut doc, body, "p", "");
    text(&mut doc, p, "before ");
    let chip = el(&mut doc, p, "span", "chip");
    text(&mut doc, chip, "chip");
    let s = cold_frame(&mut doc);
    expect(
        "inline-block (IFC inside)",
        &s,
        &[
            (StyleResolves, 3),
            (ElementsCascaded, 4),
            (StyleNodesVisited, 14),
            (FullStyleWalks, 3),
            (TaffyStyleSyncs, 7),
            (TaffyStyleChanges, 4),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 2),
            (ShapeAtomicInline, 1),
            (IfcMeasureCacheHits, 1),
            (IfcMeasureInvalidations, 4),
            (IfcSignatureChanges, 2),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 2),
            (InlineBlockComputes, 1),
            (PaintNodesVisited, 3),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// An `inline-flex` whose only child is a text node: that text is a flex item,
/// measured as a leaf through `NodeContext::Text` inside the atomic compute.
#[test]
fn an_inline_flex_holding_a_text_leaf() {
    let mut doc = doc_with(".chip { display: inline-flex; padding: 2px; }");
    doc.tree.perf.reset();
    let body = doc.body();
    let p = el(&mut doc, body, "p", "");
    text(&mut doc, p, "before ");
    let chip = el(&mut doc, p, "span", "chip");
    text(&mut doc, chip, "chip");
    let s = cold_frame(&mut doc);
    expect(
        "inline-flex (text leaf inside)",
        &s,
        &[
            (StyleResolves, 3),
            (ElementsCascaded, 4),
            (StyleNodesVisited, 14),
            (FullStyleWalks, 3),
            (TaffyStyleSyncs, 7),
            (TaffyStyleChanges, 4),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (ShapeAtomicInline, 4),
            (ShapePaint, 1),
            (IfcMeasureCacheHits, 1),
            (IfcMeasureInvalidations, 4),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 2),
            (InlineBlockComputes, 1),
            (PaintNodesVisited, 4),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// **A finding, pinned as it is — #904; a fix must LOWER this number, and its PR updates the pin.** The same `inline-flex` painted again with
/// nothing changed: its text has no cached layout (it is a flex item inside a
/// detached atomic compute, so neither `build_ifc_layouts` nor the text-leaf
/// cache holds one), and paint's fallback shapes it — on **every** frame
/// (`shape_paint` 1 on an idle repaint). `Button`, `Badge`, `ActionIcon`,
/// `Pagination` and `Center` are all `inline-flex`, so a label directly inside
/// one is re-shaped by each paint that reaches it. This is also the only
/// scenario that reaches `shape_paint`'s third site (`paint/mod.rs`). A fix
/// that caches the layout moves this to 0; update it and say so.
#[test]
fn an_inline_flex_label_is_reshaped_by_every_paint() {
    let mut doc = doc_with(".chip { display: inline-flex; padding: 2px; }");
    let body = doc.body();
    let p = el(&mut doc, body, "p", "");
    text(&mut doc, p, "before ");
    let chip = el(&mut doc, p, "span", "chip");
    text(&mut doc, chip, "chip");
    let s = repaint_frame(&mut doc);
    expect(
        "inline-flex label repaint",
        &s,
        &[
            (ShapePaint, 1),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 4),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// The control for the finding above: a text leaf of a plain (block-level)
/// flex container keeps the layout its measure built, so repainting it shapes
/// nothing.
#[test]
fn a_flex_item_text_leaf_is_not_reshaped_by_paint() {
    let mut doc = doc_with(".row { display: flex; width: 200px; }");
    let body = doc.body();
    let d = el(&mut doc, body, "div", "row");
    text(&mut doc, d, "flex item text");
    let s = repaint_frame(&mut doc);
    expect(
        "flex text leaf repaint",
        &s,
        &[
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 3),
            (StackingOrderBuilds, 1),
        ],
    );
}

// ── pseudo_element_passes ──────────────────────────────────────────────────

/// A sheet with a `::before` rule and no `::after` rule: one pass per cascaded
/// element, all of them from the `::before` site.
#[test]
fn only_before_rules() {
    let mut doc = doc_with(".gen::before { content: \"*\"; }");
    doc.tree.perf.reset();
    let body = doc.body();
    let d = el(&mut doc, body, "div", "gen");
    text(&mut doc, d, "x");
    let s = cold_frame(&mut doc);
    expect(
        "::before only",
        &s,
        &[
            (StyleResolves, 2),
            (ElementsCascaded, 4),
            (StyleNodesVisited, 11),
            (PseudoElementPasses, 4),
            (FullStyleWalks, 2),
            (TaffyStyleSyncs, 6),
            (TaffyStyleChanges, 3),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureCacheHits, 1),
            (IfcMeasureInvalidations, 3),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 2),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// The mirror: an `::after` rule only.
#[test]
fn only_after_rules() {
    let mut doc = doc_with(".gen::after { content: \"*\"; }");
    doc.tree.perf.reset();
    let body = doc.body();
    let d = el(&mut doc, body, "div", "gen");
    text(&mut doc, d, "x");
    let s = cold_frame(&mut doc);
    expect(
        "::after only",
        &s,
        &[
            (StyleResolves, 2),
            (ElementsCascaded, 4),
            (StyleNodesVisited, 11),
            (PseudoElementPasses, 4),
            (FullStyleWalks, 2),
            (TaffyStyleSyncs, 6),
            (TaffyStyleChanges, 3),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureCacheHits, 1),
            (IfcMeasureInvalidations, 3),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 2),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}
