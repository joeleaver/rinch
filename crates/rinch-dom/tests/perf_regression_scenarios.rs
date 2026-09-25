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
//! | `shape_paint` | `paint/mod.rs` (text with no cached layout) | [`a_text_leaf_with_no_cached_layout_is_shaped_by_paint`] — constructed: since #904 a text leaf keeps the layout its measure shaped, in either compute |
//! | `ellipsis_builds` | `ifc.rs`, IFC root | [`an_ifc_root_ellipsis`] |
//! | `ellipsis_builds` | `ifc.rs`, text leaf | none known since #998 — [`a_contents_wrapped_flex_item_ellipsis`] reached it until rinch blockified that span, and now pins the IFC-root site instead; [`a_text_leaf_ellipsis`] pins that a flex container's own text builds none |
//! | `shape_atomic_inline` | `ifc.rs`, `NodeContext::InlineRoot` | [`an_inline_block_holding_an_ifc`] |
//! | `shape_atomic_inline` | `ifc.rs`, `NodeContext::Text` | [`an_inline_flex_holding_a_text_leaf`] |
//! | `pseudo_element_passes` | `resolve.rs`, `::before` | [`only_before_rules`] |
//! | `pseudo_element_passes` | `resolve.rs`, `::after` | [`only_after_rules`] |
//! | `inset_shadow_mask_px` | `paint/borders.rs`, blurred inset shadow (full repaint: cropped to the window) | [`a_blurred_inset_shadow_builds_its_visible_area`] |
//! | `inset_shadow_mask_px` | the same, partial repaint (cropped to the damage) | [`a_partial_repaint_builds_only_the_damaged_part_of_an_inset_shadow`] |
//! | `ifc_hang_passes`, `ifc_hang_lines` | `ifc.rs` `build_ifc_layouts` (the paint layout) and `layout_engine.rs` (the root compute's measure) | [`a_double_spaced_pre_wrap_paragraph_hangs_in_one_pass`] — 40 lines from each site |
//! | `ifc_hang_passes`, `ifc_hang_lines` | `ifc.rs`, `NodeContext::InlineRoot` (an atomic inline's measure) | [`an_inline_block_hangs_its_spaces_in_one_pass`] |
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
            (PaintNodesVisited, 2),
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
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 2),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// The same declarations on a text node that is **not** in an IFC — the
/// direct text child of a flex container, laid out as a leaf — build **no**
/// ellipsis (`ellipsis_builds` 1 → 0, #904's second review): that text is an
/// anonymous flex item, which does not clip, and Chrome 153 draws it clipped
/// with no "…". The text-leaf rebuild site in `copy_cached_text_layouts` was
/// reached by one other leaf until #998 — see
/// [`a_contents_wrapped_flex_item_ellipsis`].
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
            (IfcMeasureInvalidations, 2),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 4),
            (PaintNodesVisited, 3),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// A `span` that is a flex item only through a `display: contents` wrapper.
/// CSS blockifies it — Chrome 153 computes `display: block`, a 60px box, and
/// draws the "…" — and since #998 so does rinch: the span is an IFC root, and
/// its "…" is built by the IFC-root site, as [`an_ifc_root_ellipsis`]'s is.
///
/// Before #998 rinch kept the span `display: inline`, its text was measured as
/// a Taffy text leaf, and the text-leaf rebuild in `copy_cached_text_layouts`
/// was what drew the "…" (#982). That was the only route found to that site;
/// `ifc_root` below is what moved.
#[test]
fn a_contents_wrapped_flex_item_ellipsis() {
    let mut doc = doc_with(
        ".flex { display: flex; } .c { display: contents; }
         .clip { width: 60px; overflow: hidden; white-space: nowrap;
                 text-overflow: ellipsis; }",
    );
    doc.tree.perf.reset();
    let body = doc.body();
    let flex = el(&mut doc, body, "div", "flex");
    let wrapper = el(&mut doc, flex, "div", "c");
    let clip = el(&mut doc, wrapper, "span", "clip");
    let t = text(&mut doc, clip, "a line much too long for sixty pixels");
    let s = cold_frame(&mut doc);
    assert_eq!(
        doc.tree.get(t.0).unwrap().ifc_root,
        Some(clip.0),
        "the blockified span is the text's IFC root"
    );
    let layout = doc
        .tree
        .get(clip.0)
        .unwrap()
        .text_layout
        .as_ref()
        .expect("the span keeps an inline layout");
    assert!(
        layout.text_content.ends_with('\u{2026}'),
        "the painted line ends in an ellipsis: {:?}",
        layout.text_content
    );
    let w = layout.layout.width();
    assert!(w <= 60.0, "the painted line is truncated to the box: {w}");
    expect(
        "ellipsis (blockified span behind display: contents)",
        &s,
        &[
            (StyleResolves, 4),
            (ElementsCascaded, 5),
            (StyleNodesVisited, 18),
            (FullStyleWalks, 4),
            (TaffyStyleSyncs, 9),
            (TaffyStyleChanges, 5),
            (ShapeMeasureIfc, 2),
            (ShapeIfcBuild, 1),
            (EllipsisBuilds, 1),
            (IfcMeasureCacheHits, 1),
            (IfcMeasureInvalidations, 4),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 3),
            (PaintNodesVisited, 4),
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
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
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
/// Its first paint shapes nothing since #904 (`shape_paint` 1 → 0): the
/// compute keeps the layout its measure built.
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
            (IfcMeasureCacheHits, 1),
            (IfcMeasureInvalidations, 4),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 2),
            (InlineBlockComputes, 1),
            (PaintNodesVisited, 4),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// **Was a finding, #904; fixed.** The same `inline-flex` painted again with
/// nothing changed. Its text is a flex item inside a detached atomic compute
/// (`measure_inline_blocks`), so no root-compute measure and no IFC reaches it;
/// until #904 that compute kept none of the layouts its measure built, and
/// paint's fallback shaped the label on **every** frame (`shape_paint` 1 on an
/// idle repaint). `Button`, `Badge`, `ActionIcon`, `Pagination` and `Center`
/// are all `inline-flex`. The compute now keeps them
/// (`NodeTree::atomic_leaf_layouts`), so this repaint shapes nothing — the
/// same frame as [`a_flex_item_text_leaf_is_not_reshaped_by_paint`] plus the
/// paragraph around the chip. The fallback site keeps its coverage in
/// [`a_text_leaf_with_no_cached_layout_is_shaped_by_paint`].
#[test]
fn an_inline_flex_label_is_not_reshaped_by_paint() {
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

/// `shape_paint`'s third site (`paint/mod.rs`): a text leaf that reaches paint
/// with no cached layout is shaped by paint itself. Since #904 a text leaf
/// keeps the layout its measure shaped, in the root compute and the atomic one
/// alike, so this state is **constructed** — the
/// control's flex text with its cached layout taken away — and it is what
/// keeps that increment site covered. Exactly 1: one leaf, one frame.
#[test]
fn a_text_leaf_with_no_cached_layout_is_shaped_by_paint() {
    let mut doc = doc_with(".row { display: flex; width: 200px; }");
    let body = doc.body();
    let d = el(&mut doc, body, "div", "row");
    let t = text(&mut doc, d, "flex item text");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc.tree.nodes[t.0].cached_text_parley = None;
    let s = repaint_frame(&mut doc);
    expect(
        "text leaf with no cached layout",
        &s,
        &[
            (ShapePaint, 1),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 3),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// A colour-only restyle of a container whose **own** text is a flex item —
/// a hover, in effect — after one settled, painted frame. Since #904 paint
/// draws a text leaf in its parent's current colour, so this takes the cheap
/// path: no Taffy compute, no shape of any kind. #904's first cut rebuilt the
/// leaf's layout through a compute instead, 3 µs → 308 µs per hover over 500
/// rows (its review), which is what these two pin against.
fn colour_hover_frame(display: &str) -> FrameStats {
    let mut doc = doc_with(&format!(
        ".row {{ display: {display}; width: 200px; }} .row.hot {{ color: rgb(200, 10, 10); }}"
    ));
    let body = doc.body();
    let d = el(&mut doc, body, "div", "row");
    text(&mut doc, d, "flex item text");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    paint(&mut doc);
    doc.tree.perf.reset();
    doc.set_attribute(d, "class", "row hot");
    doc.resolve_layout(VP.0, VP.1);
    paint(&mut doc);
    doc.tree.perf.end_frame()
}

#[test]
fn a_colour_hover_on_a_flex_items_text_skips_layout() {
    let s = colour_hover_frame("flex");
    expect(
        "colour hover, flex text leaf",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 3),
            (StackingOrderBuilds, 1),
        ],
    );
}

#[test]
fn a_colour_hover_on_an_inline_flex_label_skips_layout() {
    let s = colour_hover_frame("inline-flex");
    // The one shape is the **line around** the chip: an `inline-flex` is a
    // member of the IFC it sits in, and a member's restyle rebuilds that IFC
    // (`ShapeIfcBuild`, no compute: `LayoutSkippedTextOnly`). The label itself
    // is neither measured nor shaped.
    expect(
        "colour hover, inline-flex label",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 1),
            (LayoutResolves, 1),
            (LayoutSkippedTextOnly, 1),
            (PaintNodesVisited, 3),
            (StackingOrderBuilds, 1),
        ],
    );
}

// ── pseudo_element_passes ──────────────────────────────────────────────────

/// A sheet with a `::before` rule and no `::after` rule: one pass per cascaded
/// element, all of them from the `::before` site. The generated box is not
/// itself cascaded (#1004): it carries its pseudo cascade's style.
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
            (ElementsCascaded, 3),
            (StyleNodesVisited, 9),
            (PseudoElementPasses, 3),
            (FullStyleWalks, 2),
            (TaffyStyleSyncs, 5),
            (TaffyStyleChanges, 3),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureCacheHits, 1),
            (IfcMeasureInvalidations, 3),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
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
            (ElementsCascaded, 3),
            (StyleNodesVisited, 9),
            (PseudoElementPasses, 3),
            (FullStyleWalks, 2),
            (TaffyStyleSyncs, 5),
            (TaffyStyleChanges, 3),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureCacheHits, 1),
            (IfcMeasureInvalidations, 3),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 2),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

// ── inset_shadow_mask_px ───────────────────────────────────────────────────

/// A 300x200 panel with a blurred inset shadow, half of it past the right
/// edge of the 400x300 window.
fn inset_shadow_panel() -> RinchDocument {
    let mut doc = doc_with(
        ".panel { position: absolute; left: 250px; top: 50px; width: 300px; height: 200px; \
         box-shadow: inset 0 0 12px rgb(0, 0, 0); }",
    );
    let body = doc.body();
    el(&mut doc, body, "div", "panel");
    doc
}

/// A full repaint builds the blurred shadow over the part of the panel the
/// window can show: 300x200 at x = 250 in a 400px window whose cull reaches
/// 64px past its edge (the ink margin), so 214 columns of the 300 — 214 x
/// 200 = 42,800 pixels, not the panel's 60,000.
#[test]
fn a_blurred_inset_shadow_builds_its_visible_area() {
    let mut doc = inset_shadow_panel();
    let s = repaint_frame(&mut doc);
    expect(
        "blurred inset shadow, full repaint",
        &s,
        &[
            (InsetShadowMaskPx, 42_800),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// A 20x20 partial repaint inside the same panel — a caret blinking in it —
/// builds 20x20 pixels of shadow. Round 2 of #1014's review measured the
/// mask ignoring the damage (it lives outside the painter's clip tracking):
/// the whole visible panel, rebuilt every frame, 34 MB of it at 4K.
#[test]
fn a_partial_repaint_builds_only_the_damaged_part_of_an_inset_shadow() {
    use peniko::kurbo::{Affine, Rect};
    use rinch_dom::paint::painter::{PaintShape, Painter};
    let mut doc = inset_shadow_panel();
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    paint(&mut doc);
    doc.tree.perf.reset();
    doc.resolve_layout(VP.0, VP.1);
    let damage = Rect::new(300.0, 100.0, 320.0, 120.0);
    let mut painter = TinySkiaPainter::new(VP.0 as u32, VP.1 as u32);
    rinch_dom::paint::set_dirty_rects(Some(&[damage]));
    painter.push_clip(
        peniko::Fill::NonZero,
        Affine::IDENTITY,
        &PaintShape::Rect(damage),
    );
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        VP,
        &mut doc.font_cx,
        &mut doc.layout_cx,
    );
    painter.pop_layer();
    rinch_dom::paint::set_dirty_region(None);
    let s = doc.tree.perf.end_frame();
    expect(
        "blurred inset shadow, 20x20 partial repaint",
        &s,
        &[
            (InsetShadowMaskPx, 400),
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// A 40-line `pre-wrap` paragraph whose every line wraps at a double space:
/// each line hangs its second space (#1018). `ifc_hang_passes` is the
/// linearity pin — one re-break per layout however many lines it fixes, where
/// #1018's first version re-broke the paragraph once per fixed line, 40 times
/// per layout and quadratic in the paragraph (review of #1018, F1).
/// `ifc_hang_lines` is the lines it fixed, summed over every layout the frame
/// built: each measure the root compute made and the paint layout.
#[test]
fn a_double_spaced_pre_wrap_paragraph_hangs_in_one_pass() {
    let mut doc = doc_with(".p { width: 36px; white-space: pre-wrap; }");
    doc.tree.perf.reset();
    let body = doc.body();
    let p = el(&mut doc, body, "div", "p");
    text(&mut doc, p, &"river  ".repeat(40));
    let s = cold_frame(&mut doc);
    expect(
        "pre-wrap paragraph, hanging",
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
            (IfcMeasureCacheHits, 1),
            (IfcMeasureInvalidations, 2),
            (IfcSignatureChanges, 1),
            (IfcHangPasses, 2),
            (IfcHangLines, 80),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 2),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// The atomic-inline site of the same counters: an `inline-block` whose
/// `pre-wrap` text wraps at double spaces inside it.
#[test]
fn an_inline_block_hangs_its_spaces_in_one_pass() {
    let mut doc =
        doc_with(".chip { display: inline-block; max-width: 36px; white-space: pre-wrap; }");
    doc.tree.perf.reset();
    let body = doc.body();
    let p = el(&mut doc, body, "p", "");
    let chip = el(&mut doc, p, "span", "chip");
    text(&mut doc, chip, &"river  ".repeat(10));
    let s = cold_frame(&mut doc);
    expect(
        "inline-block, hanging",
        &s,
        &[
            (StyleResolves, 3),
            (ElementsCascaded, 4),
            (StyleNodesVisited, 12),
            (FullStyleWalks, 3),
            (TaffyStyleSyncs, 7),
            (TaffyStyleChanges, 4),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 2),
            (ShapeAtomicInline, 2),
            (IfcMeasureCacheHits, 1),
            (IfcMeasureInvalidations, 3),
            (IfcSignatureChanges, 2),
            (IfcHangPasses, 2),
            (IfcHangLines, 20),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 2),
            (InlineBlockComputes, 2),
            (PaintNodesVisited, 3),
            (StackingOrderBuilds, 1),
        ],
    );
}
