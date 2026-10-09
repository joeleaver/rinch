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
//! | `shape_select_label` | `select.rs` `widest_select_label` (an auto-width select's size) | [`an_auto_width_select_shapes_its_labels_only_when_they_change`] |
//! | `shape_form_control_metrics` | `form_control.rs` `cached_char_metrics` (a text control's intrinsic width, #1177) | [`a_text_control_finds_its_font_only_when_the_font_changes`] |
//! | `shape_form_control_label` | `form_control.rs` `cached_label_width` (a button label or picker representative string, #1195, review of #1302) | [`a_form_control_label_is_shaped_only_when_the_font_changes`] |
//! | `shape_paint` | `paint/contenteditable.rs` (`<input>` value) | [`an_input_value_is_shaped_by_paint`] |
//! | `shape_paint` | `paint/mod.rs` (text with no cached layout) | [`a_text_leaf_with_no_cached_layout_is_shaped_by_paint`] — constructed: since #904 a text leaf keeps the layout its measure shaped, in either compute |
//! | `ellipsis_builds` | `ifc.rs`, IFC root | [`an_ifc_root_ellipsis`] |
//! | `ellipsis_shapes` | `ifc.rs`, the per-line cut (no search shapes) | [`a_pre_block_ellipsis_cuts_every_line_without_shaping`] |
//! | `ellipsis_shapes` | `ifc.rs`, the whole-text prefix search | [`a_rich_nowrap_root_ellipsis_shapes_its_prefix_search`] |
//! | `ellipsis_builds` | `ifc.rs`, text leaf — **deleted in #982** | none: [`a_text_leaf_ellipsis`] pins that a flex container's own text builds none, and [`a_contents_wrapped_flex_item_ellipsis`], the last route to the site until #998, now reaches the IFC-root one |
//! | `shape_atomic_inline` | `ifc.rs`, `NodeContext::InlineRoot` | [`an_inline_block_holding_an_ifc`] |
//! | `shape_atomic_inline` | `ifc.rs`, `NodeContext::Text` | [`an_inline_flex_holding_a_text_leaf`] |
//! | `pseudo_element_passes` | `resolve.rs`, `::before` | [`only_before_rules`] |
//! | `pseudo_element_passes` | `resolve.rs`, `::after` | [`only_after_rules`] |
//! | `inset_shadow_mask_px` | `paint/borders.rs`, blurred inset shadow (full repaint: cropped to the window) | [`a_blurred_inset_shadow_builds_its_visible_area`] |
//! | `inset_shadow_mask_px` | the same, partial repaint (cropped to the damage) | [`a_partial_repaint_builds_only_the_damaged_part_of_an_inset_shadow`] |
//! | `text_shadow_masks_rasterised` | `paint/text_shadow.rs`, a blurred text-shadow's first paint | [`a_blurred_text_shadow_is_rasterised_once`] |
//! | `ifc_hang_passes`, `ifc_hang_lines` | `ifc.rs` `build_ifc_layouts` (the paint layout) and `layout_engine.rs` (the root compute's measure) | [`a_double_spaced_pre_wrap_paragraph_hangs_in_one_pass`] — 40 lines from each site |
//! | `ifc_hang_passes`, `ifc_hang_lines` | `ifc.rs`, `NodeContext::InlineRoot` (an atomic inline's measure) | [`an_inline_block_hangs_its_spaces_in_one_pass`] |
//! | `ifc_unglue_rebreaks` | `ifc.rs` `build_ifc_layouts` and `layout_engine.rs`'s measure (an IFC root) | [`an_nbsp_glued_chain_is_rebroken_in_logarithmically_many_breaks`] |
//! | `ifc_unglue_rebreaks` | `ifc.rs` `NodeContext::Text` and `layout_engine.rs`'s leaf measure (`break_leaf_lines`) | [`an_nbsp_glued_chain_in_a_flex_items_own_text_is_rebroken_the_same_way`] |
//! | `ifc_phantom_rebreaks` | `ifc.rs` `build_ifc_layouts` and `layout_engine.rs`'s measure, together | [`an_overflowing_last_chip_is_rebroken_without_parleys_empty_line`] — 1 from each |
//! | `ifc_phantom_rebreaks` | `layout_engine.rs`'s measure alone (a min-content measure) | [`a_flex_items_paragraph_ending_in_a_chip_pays_one_rebreak_per_min_content_measure`] |
//! | `inline_block_computes` | `ifc.rs` `resolve_percentage_inline_blocks`, a `fit-content`/`stretch` atomic inline (#691) | [`fit_content_inline_blocks_are_not_remeasured_for_an_unrelated_change`] |
//! | `abs_containing_block_passes` | `layout_engine.rs` `resolve_layout`'s fixpoint (#386) | [`an_absolute_box_under_a_non_parent_containing_block_pays_one_compute_per_resize`] (1 per resize of the block), [`a_position_only_absolute_box_under_a_grandparent_pays_no_compute`] (0) |
//! | `inline_font_family_resolves` | `ifc.rs` `inline_style_props` (review of #1326's perf finding) — fires only when a span's own `font-family` differs from the enclosing style's | [`a_spans_unchanged_font_family_resolves_nothing`] (0), [`a_spans_changed_font_family_resolves_once_per_rebuild`] (nonzero) |
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

// ── shape_form_control_metrics ─────────────────────────────────────────────

/// A text control's intrinsic width is `size` / `cols` times its primary
/// font's average character width (#1177). Finding that font shapes one `0`
/// when the control is first sized, and again only when its font family,
/// weight or style changes: the metrics are cached on the node, so a colour
/// restyle and a `font-size` change (which scales them) shape none.
#[test]
fn a_text_control_finds_its_font_only_when_the_font_changes() {
    let mut doc = doc_with("");
    let body = doc.body();
    let input = el(&mut doc, body, "input", "");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc.tree.perf.reset();

    doc.set_style(input, "color", "rgb(255, 0, 0)");
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "input colour restyle",
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
        ],
    );

    doc.set_style(input, "font-size", "20px");
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "input font-size change",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 2),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (InlineBlockComputes, 1),
        ],
    );

    doc.set_style(input, "font-weight", "700");
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "input font-weight change",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (ShapeFormControlMetrics, 1),
            (IfcMeasureInvalidations, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (InlineBlockComputes, 1),
        ],
    );
}

// ── shape_select_label ─────────────────────────────────────────────────────

/// An auto-width `<select>` is sized from its widest option label, shaped at
/// style sync (`style_resolution`, #1098) — one shape per label, and none on a
/// restyle that moves neither the labels nor the font (the width is cached on
/// the node). An edited label shapes them all again.
#[test]
fn an_auto_width_select_shapes_its_labels_only_when_they_change() {
    let mut doc = doc_with("");
    let body = doc.body();
    let sel = el(&mut doc, body, "select", "");
    let mut last = None;
    for label in ["Apple", "Banana"] {
        let o = el(&mut doc, sel, "option", "");
        last = Some(text(&mut doc, o, label));
    }
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc.tree.perf.reset();

    doc.set_style(sel, "color", "rgb(255, 0, 0)");
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "select colour restyle",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 3),
            (StyleNodesVisited, 3),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 3),
            // 3 → 1 (#509, #826): each `<option>` is `display: none` by the
            // UA sheet, so before the fix each one was *also* wrongly
            // classified as an IFC root over its own label text — a Parley
            // build nobody ever paints. Only the `<select>` itself (a value
            // control, root whatever its children are) still builds.
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 1),
            (LayoutResolves, 1),
            (LayoutSkippedTextOnly, 1),
        ],
    );

    doc.set_text_content(last.unwrap(), "Blueberry");
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "select label edit",
        &s,
        &[
            (ShapeSelectLabel, 2),
            (StyleResolves, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (ShapeMeasureIfc, 1),
            // 2 → 1 (#509, #826): the edited `<option>`'s label is no
            // longer its own dead IFC build either — only the `<select>`'s
            // own root still builds.
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 4),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (InlineBlockComputes, 1),
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
            (TaffyStyleSyncs, 3),
            (TaffyStyleChanges, 2),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (EllipsisBuilds, 1),
            (EllipsisShapes, 2),
            (IfcMeasureInvalidations, 2),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// A `pre` block of 40 lines, **every** one overflowing, each cut with its
/// own "…" (#1091): the cut is found by walking the clusters the paint layout
/// already shaped, so `ellipsis_shapes` is the "…" and the rebuilt layout —
/// 2, however many lines are cut. Shaping each candidate prefix per line, as
/// #1103's first round did, was about 7 shapes a line (#1103's review: a
/// 300-line block went 34 → 163 ms with `ellipsis_builds` unmoved).
#[test]
fn a_pre_block_ellipsis_cuts_every_line_without_shaping() {
    let mut doc = doc_with(
        ".clip { width: 60px; overflow: hidden; white-space: pre;
                 text-overflow: ellipsis; }",
    );
    doc.tree.perf.reset();
    let body = doc.body();
    let clip = el(&mut doc, body, "div", "clip");
    let block = vec!["a line much too long for sixty pixels"; 40].join("\n");
    text(&mut doc, clip, &block);
    let s = cold_frame(&mut doc);
    expect(
        "ellipsis (40-line pre block)",
        &s,
        &[
            (StyleResolves, 2),
            (ElementsCascaded, 3),
            (StyleNodesVisited, 7),
            (FullStyleWalks, 2),
            (TaffyStyleSyncs, 3),
            (TaffyStyleChanges, 2),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (EllipsisBuilds, 1),
            (EllipsisShapes, 2),
            (IfcMeasureInvalidations, 2),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// A `nowrap` root whose content the flat rebuild cannot represent (a
/// coloured span) keeps the behaviour before #1091: the whole text cut to one
/// prefix by a binary search that shapes each candidate — every one counted.
#[test]
fn a_rich_nowrap_root_ellipsis_shapes_its_prefix_search() {
    let mut doc = doc_with(
        ".clip { width: 60px; overflow: hidden; white-space: nowrap;
                 text-overflow: ellipsis; }
         .red { color: rgb(255, 0, 0); }",
    );
    doc.tree.perf.reset();
    let body = doc.body();
    let clip = el(&mut doc, body, "div", "clip");
    let red = el(&mut doc, clip, "span", "red");
    text(&mut doc, red, "a line much");
    text(&mut doc, clip, " too long for sixty pixels");
    let s = cold_frame(&mut doc);
    expect(
        "ellipsis (rich nowrap root)",
        &s,
        &[
            (StyleResolves, 3),
            (ElementsCascaded, 4),
            (StyleNodesVisited, 13),
            (FullStyleWalks, 3),
            (TaffyStyleSyncs, 4),
            (TaffyStyleChanges, 3),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (EllipsisBuilds, 1),
            (EllipsisShapes, 8),
            (IfcMeasureInvalidations, 4),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// The same declarations on a text node that is **not** in an IFC — the
/// direct text child of a flex container, laid out as a leaf — build **no**
/// ellipsis (`ellipsis_builds` 1 → 0, #904's second review): that text is an
/// anonymous flex item, which does not clip, and Chrome 153 draws it clipped
/// with no "…". `copy_cached_text_layouts` had a text-leaf rebuild site of
/// its own until #982, reached by one other leaf until #998 — see
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
            (TaffyStyleSyncs, 3),
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
/// a Taffy text leaf, and a text-leaf rebuild in `copy_cached_text_layouts`
/// was what drew the "…". That was the only route found to that site, which
/// #982 then deleted; `ifc_root` below is what moved.
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
            (TaffyStyleSyncs, 5),
            (TaffyStyleChanges, 4),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (EllipsisBuilds, 1),
            (EllipsisShapes, 2),
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
            (TaffyStyleSyncs, 4),
            (TaffyStyleChanges, 3),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 2),
            (ShapeAtomicInline, 1),
            (IfcMeasureInvalidations, 4),
            (IfcSignatureChanges, 2),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
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
            (TaffyStyleSyncs, 4),
            (TaffyStyleChanges, 3),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (ShapeAtomicInline, 4),
            (IfcMeasureInvalidations, 4),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
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

/// A paragraph of ordinary (IFC) text part-way through a `transition:
/// color`, and the frame that ends it (#679). A tick writes the colour with no
/// cascade; paint draws each text range in the colour its element computes
/// now, so a running frame is a tick and a paint. The ending frame rebuilds
/// the one layout the colour was baked into, so later paints are back on the
/// layout's own brushes.
fn colour_transition_frames() -> (FrameStats, FrameStats, FrameStats) {
    let mut doc = doc_with(
        ".row { width: 200px; color: rgb(10, 10, 10); transition: color 100s linear; } \
         .row.hot { color: rgb(200, 10, 10); }",
    );
    let body = doc.body();
    let d = el(&mut doc, body, "div", "row");
    text(&mut doc, d, "a paragraph of text");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    paint(&mut doc);
    doc.set_attribute(d, "class", "row hot");
    doc.resolve_layout(VP.0, VP.1);
    paint(&mut doc);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1000.0;
    let frame = |doc: &mut RinchDocument, ago_ms: f64| {
        for t in doc
            .tree
            .active_transitions
            .get_mut(&d.0)
            .into_iter()
            .flat_map(|props| props.values_mut())
        {
            t.start_time_ms = now - ago_ms;
        }
        doc.tree.perf.reset();
        doc.tick_transitions();
        doc.resolve_layout(VP.0, VP.1);
        paint(doc);
        doc.tree.perf.end_frame()
    };
    let running = frame(&mut doc, 40_000.0);
    let ending = frame(&mut doc, 200_000.0);
    let after = frame(&mut doc, 200_000.0);
    (running, ending, after)
}

#[test]
fn a_colour_transition_frame_on_ifc_text_shapes_nothing() {
    let (running, _, _) = colour_transition_frames();
    expect(
        "colour transition, running frame",
        &running,
        &[
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

#[test]
fn the_frame_that_ends_a_colour_transition_rebuilds_one_layout() {
    let (_, ending, after) = colour_transition_frames();
    // One paint layout rebuilt (`ShapeIfcBuild`), with its cached measures
    // dropped beside it (`IfcMeasureInvalidations`), and no Taffy compute:
    // `LayoutSkippedTextOnly`. What a colour-only hover costs.
    expect(
        "colour transition, ending frame",
        &ending,
        &[
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 1),
            (LayoutResolves, 1),
            (LayoutSkippedTextOnly, 1),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
    expect(
        "colour transition, the frame after",
        &after,
        &[
            (LayoutResolves, 1),
            (LayoutSkippedPaintOnly, 1),
            (PaintNodesVisited, 2),
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
            (TaffyStyleSyncs, 4),
            (TaffyStyleChanges, 3),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 3),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
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
            (TaffyStyleSyncs, 4),
            (TaffyStyleChanges, 3),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 3),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
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

// ── text_shadow_masks_rasterised ───────────────────────────────────────────

/// A line with a blurred `text-shadow` rasterises its mask on its first
/// paint and serves it from the cache on the next (#980): 1, then none
/// (unlisted, so asserted 0).
#[test]
fn a_blurred_text_shadow_is_rasterised_once() {
    let mut doc = doc_with(".t { text-shadow: 0 2px 4px rgba(0, 0, 0, 0.5); }");
    let body = doc.body();
    let t = el(&mut doc, body, "div", "t");
    text(&mut doc, t, "Shadowed");
    rinch_dom::paint::clear_text_shadow_cache();
    let s = cold_frame(&mut doc);
    expect(
        "blurred text-shadow, first paint",
        &s,
        &[
            (StyleResolves, 2),
            (ElementsCascaded, 3),
            (StyleNodesVisited, 7),
            (FullStyleWalks, 2),
            (TaffyStyleSyncs, 3),
            (TaffyStyleChanges, 2),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 2),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
            (TextShadowMasksRasterised, 1),
        ],
    );
    doc.resolve_layout(VP.0, VP.1);
    paint(&mut doc);
    let s = doc.tree.perf.end_frame();
    expect(
        "blurred text-shadow, repaint",
        &s,
        &[
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
            (TaffyStyleSyncs, 3),
            (TaffyStyleChanges, 2),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
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
            (TaffyMeasureCalls, 1),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// A paragraph of 1600 words all glued by NBSPs (#1218, review of #1257 F1),
/// at a width parley hangs an NBSP at: one line, overflowing. The re-break
/// that moves parley's break off the NBSP searches the line by galloping, so
/// it is a handful of breaks of the line — not one per glued word, which made
/// this paragraph quadratic (62 ms where main took 4).
#[test]
fn an_nbsp_glued_chain_is_rebroken_in_logarithmically_many_breaks() {
    let mut doc = doc_with(".p { width: 40px; }");
    doc.tree.perf.reset();
    let body = doc.body();
    let p = el(&mut doc, body, "div", "p");
    text(&mut doc, p, &vec!["ab"; 1600].join("\u{a0}"));
    let s = cold_frame(&mut doc);
    expect(
        "nbsp-glued chain",
        &s,
        &[
            (StyleResolves, 2),
            (ElementsCascaded, 3),
            (StyleNodesVisited, 7),
            (FullStyleWalks, 2),
            (TaffyStyleSyncs, 3),
            (TaffyStyleChanges, 2),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 2),
            (IfcSignatureChanges, 1),
            // One hang pass, one line re-broken, 15 breaks of it, from each
            // site (the measure and the paint layout).
            (IfcHangPasses, 2),
            (IfcHangLines, 2),
            (IfcUnglueRebreaks, 30),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// The text-leaf site of the same re-break: a flex item's own text.
#[test]
fn an_nbsp_glued_chain_in_a_flex_items_own_text_is_rebroken_the_same_way() {
    let mut doc = doc_with(".p { width: 40px; display: flex; flex-direction: column; }");
    doc.tree.perf.reset();
    let body = doc.body();
    let p = el(&mut doc, body, "div", "p");
    text(&mut doc, p, &vec!["ab"; 1600].join("\u{a0}"));
    let s = cold_frame(&mut doc);
    expect(
        "nbsp-glued chain, text leaf",
        &s,
        &[
            (StyleResolves, 2),
            (ElementsCascaded, 3),
            (StyleNodesVisited, 7),
            (FullStyleWalks, 2),
            (TaffyStyleSyncs, 3),
            (TaffyStyleChanges, 2),
            (ShapeMeasureText, 4),
            (IfcMeasureInvalidations, 2),
            // 15 breaks of the one line per measure, four measures.
            (IfcHangPasses, 4),
            (IfcHangLines, 4),
            (IfcUnglueRebreaks, 60),
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
            (TaffyStyleSyncs, 4),
            (TaffyStyleChanges, 3),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 2),
            (ShapeAtomicInline, 2),
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
            (TaffyMeasureCalls, 1),
            (InlineBlockComputes, 2),
            (PaintNodesVisited, 3),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// #1050: a paragraph whose last item is an inline box too wide for its
/// line. Parley commits an empty line after it, which `ifc_phantom_rebreaks`
/// counts breaking the paragraph once more without.
#[test]
fn an_overflowing_last_chip_is_rebroken_without_parleys_empty_line() {
    let mut doc = doc_with(
        ".p { width: 200px; } .chip { display: inline-block; width: 300px; height: 80px; }",
    );
    doc.tree.perf.reset();
    let body = doc.body();
    let p = el(&mut doc, body, "div", "p");
    text(&mut doc, p, "x ");
    el(&mut doc, p, "span", "chip");
    let s = cold_frame(&mut doc);
    expect(
        "overflowing last chip",
        &s,
        &[
            (StyleResolves, 3),
            (ElementsCascaded, 4),
            (StyleNodesVisited, 13),
            (FullStyleWalks, 3),
            (TaffyStyleSyncs, 4),
            (TaffyStyleChanges, 3),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 3),
            (IfcSignatureChanges, 1),
            (IfcPhantomRebreaks, 2),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (InlineBlockComputes, 1),
            (PaintNodesVisited, 3),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// #1050's cost on the common shape: a flex item's paragraph that *ends* in a
/// small chip. Its min-content measure breaks at width 0, where every box
/// takes parley's emergency branch, so the chip is an overflowing last item
/// there and that measure is broken once more.
#[test]
fn a_flex_items_paragraph_ending_in_a_chip_pays_one_rebreak_per_min_content_measure() {
    let mut doc = doc_with(
        ".row { display: flex; width: 300px; } .chip { display: inline-block; width: 16px; height: 16px; }",
    );
    doc.tree.perf.reset();
    let body = doc.body();
    let row = el(&mut doc, body, "div", "row");
    let p = el(&mut doc, row, "div", "");
    text(&mut doc, p, &"lorem ipsum ".repeat(30));
    el(&mut doc, p, "span", "chip");
    let s = cold_frame(&mut doc);
    expect(
        "flex item ending in a chip",
        &s,
        &[
            (StyleResolves, 4),
            (ElementsCascaded, 5),
            (StyleNodesVisited, 19),
            (FullStyleWalks, 4),
            (TaffyStyleSyncs, 5),
            (TaffyStyleChanges, 4),
            (ShapeMeasureIfc, 3),
            (ShapeIfcBuild, 1),
            (IfcMeasureCacheHits, 1),
            (IfcMeasureInvalidations, 4),
            (IfcSignatureChanges, 1),
            (IfcPhantomRebreaks, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 4),
            (InlineBlockComputes, 1),
            (PaintNodesVisited, 4),
            (StackingOrderBuilds, 1),
        ],
    );
}

// ── a detached subtree is not measured (#1040) ─────────────────────────────
//
// `set_text_content` orphans an element's children without freeing them, so
// the atomic-inline registry still names every `inline-block`/`-flex`/`-grid`
// in the orphaned subtree. Each of the three functions that size an atomic
// inline used to measure such a box on the next layout — a Taffy compute and a
// Parley shape for a box nothing paints. One scenario per function, each the
// only route to its site in its frame, plus one that attaches the orphan again
// and compares it with a fresh layout. `build_ifc_layouts` shaped an orphaned
// IFC root too (#1069); since #1073 the orphans carry no IFC marks, and a root
// out of the document is not shaped even when a whole-document pass marks it.

const DETACH_CSS: &str = ".ib { display: inline-block; padding: 2px; }
    .ifx { display: inline-flex; padding: 3px; }
    .pct { width: 50%; }";

/// `body > p > ["before ", em.ib > div > ["in ", span.ifx > "chip"]]`, laid out
/// and painted. `flex_class` is added to the `inline-flex`'s class list.
///
/// The `div` is what keeps the `inline-flex` an atomic inline *inside* the
/// orphaned subtree — in an IFC of its own, the `div`'s — so the
/// whole-document pass, which marks every node in the slab and therefore
/// detached subtrees too, registers it again.
fn detach_doc(flex_class: &str) -> (RinchDocument, NodeId, NodeId, NodeId) {
    let mut doc = doc_with(DETACH_CSS);
    let body = doc.body();
    let p = el(&mut doc, body, "p", "");
    text(&mut doc, p, "before ");
    let em = el(&mut doc, p, "em", "ib");
    let wrap = el(&mut doc, em, "div", "");
    text(&mut doc, wrap, "in ");
    let flex = el(&mut doc, wrap, "span", &format!("ifx {flex_class}"));
    text(&mut doc, flex, "chip");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    paint(&mut doc);
    (doc, p, em, flex)
}

/// The shape the issue reports (#1040, found by #982's scoped differential at
/// seeds 2 and 41): content appended inside the `inline-flex`, then the `div`
/// holding it orphaned by `set_text_content` on the `inline-block` around it.
/// The append left the `inline-flex` in `dirty_atomic_inlines`, and
/// `remeasure_dirty_atomic_inlines` measured it though it had left the
/// document: `inline_block_computes` 2 → 1 (the `em`, whose text changed, is
/// the one left) and `shape_atomic_inline` 7 → 1 (the orphan's text leaves are
/// no longer shaped). `shape_ifc_build` is 2, one per root the document holds:
/// it was 3 until #1073, the third the orphaned `div`, dirtied by the append
/// and shaped by `build_ifc_layouts` while it was out (#1069). The orphans now
/// lose their `ifc_root` marks at the detach, so the `div` is no IFC root.
#[test]
fn a_detached_atomic_inline_is_not_remeasured() {
    let (mut doc, _p, em, flex) = detach_doc("");
    let x = el(&mut doc, flex, "div", "x");
    text(&mut doc, x, "a long sentence that is never on screen");
    doc.set_text_content(em, "word");
    doc.tree.perf.reset();
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "detached atomic inline: remeasure",
        &s,
        &[
            (StyleResolves, 1),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 2),
            (ShapeAtomicInline, 1),
            (IfcMeasureInvalidations, 1),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (IfcScopedPasses, 1),
            (IfcScopeContainers, 1),
            (IfcScopeNodes, 2),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (InlineBlockComputes, 1),
        ],
    );
}

/// `build_ifc_layouts` after a whole-document pass that reached a **removed**
/// IFC root (#1069). The whole-document structural pass still marks every slab
/// node, detached subtrees included, so the removed `div` is an IFC root again,
/// and the text appended into it while it was out left it with no paint
/// layout: it was shaped though nothing paints it, `shape_ifc_build` 1 → 0
/// (the `p`, unchanged, keeps its layout).
#[test]
fn a_removed_ifc_root_is_not_shaped_by_the_whole_document_pass() {
    let mut doc = doc_with("");
    let body = doc.body();
    let p = el(&mut doc, body, "p", "");
    text(&mut doc, p, "attached");
    let gone = el(&mut doc, body, "div", "");
    text(&mut doc, gone, "removed");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    paint(&mut doc);
    doc.remove_child(body, gone);
    text(&mut doc, gone, " and grown while out");
    doc.recompute_all_styles_full();
    doc.tree.perf.reset();
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "removed IFC root: whole-document pass",
        &s,
        &[
            (StyleResolves, 1),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullTheme, 1),
            (TaffyRootComputes, 1),
        ],
    );
    // Turned away for good, not once: its stale layout is dropped and so is
    // its registration, so the next layout does not ask about it again.
    let n = doc.tree.get(gone.0).unwrap();
    assert!(n.text_layout.is_none(), "the removed root keeps no layout");
    assert!(
        !doc.tree.ifc_root_registry.contains(&gone.0),
        "the removed root is no longer registered"
    );
    assert!(
        doc.tree.ifc_root_registry.contains(&p.0),
        "control: the attached root is"
    );
}

/// `remeasure_dirty_atomic_inlines` after a whole-document pass re-marked an
/// orphaned subtree. The orphans lose their marks at the detach (#1073), which
/// is what now keeps [`a_detached_atomic_inline_is_not_remeasured`]'s box from
/// being measured. But a whole-document pass (a theme change) marks detached
/// subtrees again, and a later edit inside the orphaned `inline-flex` queues
/// it in the dirty set with its mark in place: the connectivity filter there
/// (#1040) is what turns it away.
#[test]
fn a_remarked_detached_atomic_inline_is_not_remeasured() {
    let (mut doc, _p, em, flex) = detach_doc("");
    let chip = doc.tree.get(flex.0).unwrap().children[0];
    doc.set_text_content(em, "word");
    doc.resolve_layout(VP.0, VP.1);
    doc.recompute_all_styles_full();
    doc.resolve_layout(VP.0, VP.1);
    assert!(
        doc.tree.get(flex.0).unwrap().ifc_root.is_some(),
        "precondition: the whole-document pass marked the orphaned box again"
    );
    doc.set_text_content(NodeId(chip), "a chip edited while it is out");
    doc.tree.perf.reset();
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "re-marked detached atomic inline: remeasure",
        &s,
        &[(LayoutResolves, 1), (TaffyRootComputes, 1)],
    );
}

/// The whole-document pass (`compute_inline_block_layouts`, through
/// `inline_block_measure_roots`) after the same orphaning. That pass marks
/// every slab node, the orphaned `div`'s IFC included, so the `inline-flex`
/// is registered again and was measured: `inline_block_computes` 2 → 1.
#[test]
fn a_detached_atomic_inline_is_not_measured_by_the_whole_document_pass() {
    let (mut doc, _p, em, _flex) = detach_doc("");
    doc.set_text_content(em, "word");
    doc.resolve_layout(VP.0, VP.1);
    doc.recompute_all_styles_full();
    doc.tree.perf.reset();
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "detached atomic inline: whole-document pass",
        &s,
        &[
            (StyleResolves, 1),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullTheme, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (InlineBlockComputes, 1),
        ],
    );
}

/// The percentage re-measure (`resolve_percentage_inline_blocks`), which runs
/// after every root compute for a box with a percentage inline size. An
/// orphaned `width: 50%` `inline-flex` still named its IFC root, the orphaned
/// `div`, whose box kept the width it was last laid out at, so it was
/// re-measured against that width: `inline_block_computes` 2 → 1.
#[test]
fn a_detached_percentage_atomic_inline_is_not_remeasured() {
    let (mut doc, _p, em, _flex) = detach_doc("pct");
    doc.set_text_content(em, "word");
    doc.tree.perf.reset();
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "detached atomic inline: percentage",
        &s,
        &[
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 2),
            (ShapeAtomicInline, 1),
            (IfcMeasureInvalidations, 1),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (IfcScopedPasses, 1),
            (IfcScopeContainers, 1),
            (IfcScopeNodes, 2),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (InlineBlockComputes, 1),
        ],
    );
}

/// Not measuring the orphan must not leave it stale: attached again, it is
/// sized as a fresh document of the same final state sizes it. Content is
/// appended inside it while it is in the document and then it is orphaned
/// before a layout measures it, so its cached measure is stale when it
/// leaves; optionally it changes again while out, and it is attached again
/// under a scoped pass or a whole-document one.
#[test]
fn a_detached_atomic_inline_is_sized_again_when_reattached() {
    for class in ["", "pct"] {
        for change_while_out in [false, true] {
            for whole_document in [false, true] {
                let case = format!(
                    "class {class:?}, changed while out {change_while_out}, \
                     whole-document pass {whole_document}"
                );
                let (mut doc, p, em, flex) = detach_doc(class);
                let x = el(&mut doc, flex, "div", "x");
                text(&mut doc, x, "grown while attached");
                doc.set_text_content(em, "word");
                doc.resolve_layout(VP.0, VP.1);
                if change_while_out {
                    let y = el(&mut doc, flex, "div", "y");
                    text(&mut doc, y, "and grown again while detached");
                }
                doc.append_child(p, flex);
                if whole_document {
                    doc.recompute_all_styles_full();
                }
                doc.resolve_layout(VP.0, VP.1);
                let got = {
                    let n = &doc.tree.nodes[flex.0];
                    (n.layout.width, n.layout.height)
                };

                let mut fresh = doc_with(DETACH_CSS);
                let body = fresh.body();
                let p2 = el(&mut fresh, body, "p", "");
                text(&mut fresh, p2, "before ");
                let em2 = el(&mut fresh, p2, "em", "ib");
                text(&mut fresh, em2, "word");
                let flex2 = el(&mut fresh, p2, "span", &format!("ifx {class}"));
                text(&mut fresh, flex2, "chip");
                let x2 = el(&mut fresh, flex2, "div", "x");
                text(&mut fresh, x2, "grown while attached");
                if change_while_out {
                    let y2 = el(&mut fresh, flex2, "div", "y");
                    text(&mut fresh, y2, "and grown again while detached");
                }
                fresh.resolve_layout(VP.0, VP.1);
                fresh.resolve_layout(VP.0, VP.1);
                let want = {
                    let n = &fresh.tree.nodes[flex2.0];
                    (n.layout.width, n.layout.height)
                };
                assert!(
                    want.0 > 100.0,
                    "{case}: positive control, the fresh box holds the text ({want:?})"
                );
                assert_eq!(got, want, "{case}: re-attached box vs a fresh layout");
            }
        }
    }
}

// ── inline_block_computes: keyword atomic inlines (#691) ─────────────────────

/// A `fit-content` atomic inline is sized by three computes (its max-content,
/// its min-content, and the pinned pass) and a `stretch` one by one, once its
/// containing block has a width (`resolve_percentage_inline_blocks`). Before
/// the review of #1281 that ran after **every** root compute, so twenty such
/// boxes cost 60 + 20 detached computes on a frame that changed something
/// else entirely. They are cached on the containing block's width now: the
/// unrelated change pays **none**, and a change of that width (the second
/// frame, the positive control) pays them all again.
#[test]
fn fit_content_inline_blocks_are_not_remeasured_for_an_unrelated_change() {
    const N: usize = 10;
    let mut doc = doc_with(
        ".cb { width: 300px; font-size: 0; line-height: 0 }
         .fit { display: inline-block; width: fit-content }
         .str { display: inline-block; width: stretch }
         .c { display: inline-block; width: 100px; height: 20px }
         .other { height: 10px }",
    );
    let body = doc.body();
    let cb = el(&mut doc, body, "div", "cb");
    for class in ["fit", "str"] {
        for _ in 0..N {
            let k = el(&mut doc, cb, "span", class);
            el(&mut doc, k, "span", "c");
            el(&mut doc, k, "span", "c");
        }
    }
    let other = el(&mut doc, body, "div", "other");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc.tree.perf.reset();

    doc.set_style(other, "width", "50px");
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "an unrelated width change",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
        ],
    );

    doc.set_style(cb, "width", "150px");
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    // 10 fit-content boxes x 3 + 10 stretch boxes x 1.
    expect(
        "the containing block's width changes",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 21),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (ShapeMeasureIfc, 2),
            (ShapeIfcBuild, 21),
            (ShapeAtomicInline, 40),
            (IfcMeasureInvalidations, 20),
            (IfcPhantomRebreaks, 11),
            (LayoutResolves, 1),
            (TaffyRootComputes, 2),
            // 4 → 2 and the 2 cache hits gone with #1260: `<body>`'s flex
            // basis is no longer measured, so the content is asked once.
            (TaffyMeasureCalls, 2),
            (InlineBlockComputes, 40),
        ],
    );
}

/// `shape_form_control_label`: `form_control.rs` `cached_label_width` (a
/// `submit`/`reset`/`button` label's or a date/time/`file` picker's
/// representative-string shape, #1195, review of #1302). A colour-only
/// restyle reshapes neither; a font-size change reshapes both.
#[test]
fn a_form_control_label_is_shaped_only_when_the_font_changes() {
    let mut doc = doc_with("");
    let body = doc.body();
    let submit = el(&mut doc, body, "input", "");
    doc.set_attribute(submit, "type", "submit");
    let date = el(&mut doc, body, "input", "");
    doc.set_attribute(date, "type", "date");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc.tree.perf.reset();

    doc.set_style(submit, "color", "red");
    doc.set_style(date, "color", "red");
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "form control label colour restyle",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 2),
            (StyleNodesVisited, 2),
            (StyleInvalidations, 2),
            (TaffyStyleSyncs, 2),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 2),
            (LayoutResolves, 1),
            (LayoutSkippedTextOnly, 1),
        ],
    );

    doc.set_style(submit, "font-size", "20px");
    doc.set_style(date, "font-size", "20px");
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "form control label font-size change",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 2),
            (StyleNodesVisited, 2),
            (StyleInvalidations, 2),
            (TaffyStyleSyncs, 2),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (ShapeFormControlLabel, 2),
            (IfcMeasureInvalidations, 4),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            // 3 → 1 and no cache hits since #1260: `<body>`'s flex basis is
            // no longer measured.
            (TaffyMeasureCalls, 1),
            (InlineBlockComputes, 2),
        ],
    );
}

// ── inline_font_family_resolves ─────────────────────────────────────────────

/// A `<span>` that declares no `font-family` of its own inherits the root's
/// unchanged — `inline_style_props` skips `fonts::parley_font_family`
/// entirely for it (review of #1326's perf finding). `inline_font_family_resolves`
/// is absent from the baseline below, i.e. `0`.
#[test]
fn a_spans_unchanged_font_family_resolves_nothing() {
    let mut doc = doc_with("");
    doc.tree.perf.reset();
    let body = doc.body();
    let p = el(&mut doc, body, "div", "");
    let span = el(&mut doc, p, "span", "");
    text(&mut doc, span, "hello");
    let s = cold_frame(&mut doc);
    expect(
        "unchanged font-family span",
        &s,
        &[
            (StyleResolves, 3),
            (ElementsCascaded, 4),
            (StyleNodesVisited, 12),
            (FullStyleWalks, 3),
            (TaffyStyleSyncs, 4),
            (TaffyStyleChanges, 3),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 3),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

/// A `<span style="font-family: monospace">` whose family differs from the
/// root's resolves it exactly once per `build_inline_layout` rebuild (one for
/// the measure, one for the painted layout) — the counter this PR adds to
/// prove the skip above is not a tautology: something still fires when the
/// family actually changes.
#[test]
fn a_spans_changed_font_family_resolves_once_per_rebuild() {
    let mut doc = doc_with("");
    doc.tree.perf.reset();
    let body = doc.body();
    let p = el(&mut doc, body, "div", "");
    let span = el(&mut doc, p, "span", "");
    doc.set_attribute(span, "style", "font-family: monospace");
    text(&mut doc, span, "hello");
    let s = cold_frame(&mut doc);
    expect(
        "changed font-family span",
        &s,
        &[
            (StyleResolves, 3),
            (ElementsCascaded, 5),
            (StyleNodesVisited, 12),
            (StyleInvalidations, 1),
            (FullStyleWalks, 3),
            (TaffyStyleSyncs, 5),
            (TaffyStyleChanges, 3),
            (InlineFontFamilyResolves, 2),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 3),
            (IfcSignatureChanges, 1),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (PaintNodesVisited, 2),
            (StackingOrderBuilds, 1),
        ],
    );
}

// ── abs_containing_block_passes ────────────────────────────────────────────

/// `.cb` (positioned, 200x100) > `.mid` (static, 100x50) > `.abs`, whose
/// containing block is therefore an ancestor Taffy does not lay it out in
/// (#386). Returns the document and `.cb`.
fn absolute_under_a_grandparent(abs_css: &str) -> (RinchDocument, NodeId) {
    let mut doc = doc_with(&format!(
        ".cb {{ position: relative; width: 200px; height: 100px; }}
         .mid {{ width: 100px; height: 50px; margin-left: 20px; }}
         .abs {{ position: absolute; {abs_css} }}
         .kid {{ width: 50%; height: 50%; }}"
    ));
    doc.tree.perf.reset();
    let body = doc.body();
    let cb = el(&mut doc, body, "div", "cb");
    let mid = el(&mut doc, cb, "div", "mid");
    let abs = el(&mut doc, mid, "div", "abs");
    el(&mut doc, abs, "div", "kid");
    (doc, cb)
}

/// The only site of `abs_containing_block_passes`: `resolve_layout`'s
/// fixpoint, when `resolve_ancestor_absolutes` rewrote a Taffy style.
///
/// A box sized by its containing block (`inset: 0`) pays **one** extra root
/// compute when that block comes out of a compute at a size the box was not
/// baked for: the document's first layout (the block had no size yet), and a
/// resize of the block. A pass in which the block keeps its size — here a
/// viewport-height change, which re-syncs every absolute box's Taffy style —
/// pays none: the style site bakes the size the block already has.
#[test]
fn an_absolute_box_under_a_non_parent_containing_block_pays_one_compute_per_resize() {
    let (mut doc, cb) = absolute_under_a_grandparent("inset: 0;");
    let s = cold_frame(&mut doc);
    expect(
        "abs under a grandparent, cold",
        &s,
        &[
            (StyleResolves, 5),
            (ElementsCascaded, 6),
            (StyleNodesVisited, 24),
            (FullStyleWalks, 5),
            (TaffyStyleSyncs, 7),
            (TaffyStyleChanges, 6),
            (IfcMeasureInvalidations, 4),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 2),
            (TaffyMeasureCalls, 2),
            (AbsContainingBlockPasses, 1),
            (AbsBoxesVisited, 3),
            (PaintNodesVisited, 5),
            (StackingOrderBuilds, 1),
        ],
    );

    doc.tree.perf.reset();
    doc.resolve_layout(VP.0, VP.1 + 20.0);
    let s = doc.tree.perf.end_frame();
    expect(
        "abs under a grandparent, a pass that leaves the block alone",
        &s,
        &[
            (StyleResolves, 1),
            (TaffyStyleSyncs, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (AbsBoxesVisited, 2),
        ],
    );

    doc.set_attribute(cb, "style", "width: 240px");
    doc.tree.perf.reset();
    doc.resolve_layout(VP.0, VP.1 + 20.0);
    let s = doc.tree.perf.end_frame();
    expect(
        "abs under a grandparent, the block resized",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 2),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 2),
            (LayoutResolves, 1),
            (TaffyRootComputes, 2),
            (TaffyMeasureCalls, 1),
            (AbsContainingBlockPasses, 1),
            (AbsBoxesVisited, 3),
        ],
    );
}

/// The same document with a box whose **size** does not depend on its
/// containing block: it is placed against the block and never re-sized, so
/// neither the first layout nor a resize of the block runs a second compute.
#[test]
fn a_position_only_absolute_box_under_a_grandparent_pays_no_compute() {
    let (mut doc, cb) =
        absolute_under_a_grandparent("right: 5px; bottom: 5px; width: 40px; height: 20px;");
    let s = cold_frame(&mut doc);
    expect(
        "position-only abs under a grandparent, cold",
        &s,
        &[
            (StyleResolves, 5),
            (ElementsCascaded, 6),
            (StyleNodesVisited, 24),
            (FullStyleWalks, 5),
            (TaffyStyleSyncs, 7),
            (TaffyStyleChanges, 5),
            (IfcMeasureInvalidations, 4),
            (LayoutResolves, 2),
            (LayoutSkippedPaintOnly, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (AbsBoxesVisited, 2),
            (PaintNodesVisited, 5),
            (StackingOrderBuilds, 1),
        ],
    );

    doc.set_attribute(cb, "style", "width: 240px");
    doc.tree.perf.reset();
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "position-only abs under a grandparent, the block resized",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 2),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (AbsBoxesVisited, 2),
        ],
    );
}

// ── abs_boxes_visited ──────────────────────────────────────────────────────

/// A 100px-high scroller of twenty 20px rows, each holding a 4px leaf and one
/// absolute badge. Returns the document, the scroller and the first leaf.
///
/// - `direct`: each row is `position: relative` and the badge is its child —
///   Taffy's own answer, the ordinary badge;
/// - `ancestor`: the same row with a static wrapper around the badge, so the
///   row is a containing block Taffy does not know (#386);
/// - `icb`: nothing is positioned, so every badge resolves against the
///   initial containing block from inside the scroller (#204).
fn badge_rows(variant: &str) -> (RinchDocument, NodeId, NodeId) {
    let row_position = if variant == "icb" {
        ""
    } else {
        "position: relative;"
    };
    let mut doc = doc_with(&format!(
        ".s {{ height: 100px; width: 300px; overflow: auto; }}
         .row {{ {row_position} height: 20px; }}
         .leaf {{ height: 4px; }}
         .badge {{ position: absolute; right: 5px; top: 2px; width: 10px; height: 10px; }}"
    ));
    let body = doc.body();
    let s = el(&mut doc, body, "div", "s");
    let mut first_leaf = None;
    for _ in 0..20 {
        let row = el(&mut doc, s, "div", "row");
        let leaf = el(&mut doc, row, "div", "leaf");
        first_leaf.get_or_insert(leaf);
        let holder = if variant == "ancestor" {
            el(&mut doc, row, "div", "")
        } else {
            row
        };
        el(&mut doc, holder, "div", "badge");
    }
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    (doc, s, first_leaf.unwrap())
}

/// One leaf's margin changes and the document is laid out: no containing
/// block resizes, so no badge is re-sized or moves.
fn badge_relayout(doc: &mut RinchDocument, leaf: NodeId) -> FrameStats {
    doc.tree.perf.reset();
    let next = if doc.get_attribute(leaf, "style").is_some() {
        "margin-right: 3px"
    } else {
        "margin-left: 3px"
    };
    doc.set_attribute(leaf, "style", next);
    doc.resolve_layout(VP.0, VP.1);
    doc.tree.perf.end_frame()
}

/// The scroller scrolls by 7px; no layout runs.
fn badge_scroll(doc: &mut RinchDocument, scroller: NodeId) -> FrameStats {
    doc.tree.perf.reset();
    doc.set_scroll_top(scroller, 7.0);
    doc.tree.perf.end_frame()
}

/// The contract the review of #1409 asked for: the passes that resolve an
/// absolute box against a containing block Taffy does not know cost
/// **nothing** where there is no such box. Twenty badges that are children of
/// their own positioned row are looked at by none of them — not by a layout,
/// not by a scroll (`abs_boxes_visited` is absent from both frames). When
/// every absolute node was in one registry, each was classified twice per
/// layout and once per scroll.
#[test]
fn absolute_children_of_their_positioned_parent_are_in_no_out_of_flow_pass() {
    let (mut doc, scroller, leaf) = badge_rows("direct");
    let s = badge_relayout(&mut doc, leaf);
    expect(
        "direct badges, a leaf restyled",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
        ],
    );
    let s = badge_scroll(&mut doc, scroller);
    expect("direct badges, the scroller scrolled", &s, &[]);
}

/// Twenty badges each resolved against a non-parent ancestor (its row). A
/// layout looks at each twice — the size check after the compute and the
/// placement as it is read back — and re-sizes none; nothing on the way to a
/// row moved late, so there is no second placement. A scroll of the scroller
/// looks at none: it holds the rows, and lies between no badge and its
/// containing block.
#[test]
fn ancestor_resolved_boxes_are_visited_twice_per_layout_and_not_by_an_outer_scroll() {
    let (mut doc, scroller, leaf) = badge_rows("ancestor");
    let s = badge_relayout(&mut doc, leaf);
    expect(
        "ancestor badges, a leaf restyled",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (AbsBoxesVisited, 40),
        ],
    );
    let s = badge_scroll(&mut doc, scroller);
    expect("ancestor badges, the scroller scrolled", &s, &[]);
}

/// Twenty badges with no positioned ancestor, written inside a scroller: the
/// initial containing block is theirs, and the scroller is between each and
/// it. A layout looks at each once (the placement as it is read back; there
/// is no size check, the viewport's size is known before the compute), and a
/// scroll of the scroller at each once — every one of them has to be written
/// again to stay where it is. A layout that clamps the scroller's offset
/// looks at each twice: the read-back placed them against the old offset.
#[test]
fn icb_boxes_inside_a_scroller_are_visited_once_per_layout_and_once_per_scroll() {
    let (mut doc, scroller, leaf) = badge_rows("icb");
    let s = badge_relayout(&mut doc, leaf);
    expect(
        "icb badges, a leaf restyled",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (AbsBoxesVisited, 20),
        ],
    );
    let s = badge_scroll(&mut doc, scroller);
    expect(
        "icb badges, the scroller scrolled",
        &s,
        &[(AbsBoxesVisited, 20)],
    );

    // Scrolled to the end, then the content shrinks: the clamp pulls the
    // offset in after the boxes were placed.
    doc.set_scroll_top(scroller, 300.0);
    doc.tree.perf.reset();
    let row = NodeId(doc.tree.get(leaf.0).unwrap().parent.unwrap());
    doc.set_attribute(row, "style", "height: 5px");
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    assert_eq!(doc.tree.get(scroller.0).unwrap().scroll_offset.1, 285.0);
    expect(
        "icb badges, a layout that clamps the scroll",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 3),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 2),
            (AbsBoxesVisited, 40),
        ],
    );

    // The next layout clamps nothing, and is back to one look per box.
    let s = badge_relayout(&mut doc, leaf);
    expect(
        "icb badges, the layout after the clamp",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (AbsBoxesVisited, 20),
        ],
    );

    // The scroller becomes positioned: it is every badge's containing block
    // now, so its own scroll carries them and it lies between none of them
    // and their containing block any more. Its flag from the layouts above
    // must not outlive them.
    doc.set_attribute(scroller, "style", "position: relative");
    doc.resolve_layout(VP.0, VP.1);
    doc.tree.perf.reset();
    doc.set_scroll_top(scroller, 40.0);
    let s = doc.tree.perf.end_frame();
    expect(
        "icb badges, their scroller now the containing block",
        &s,
        &[],
    );
}

/// A box that stops being ancestor-resolved — its static parent becomes
/// positioned, so Taffy's own answer is right again — leaves
/// `NodeTree::ancestor_absolutes` on the next layout and is looked at by no
/// pass after it.
#[test]
fn a_box_that_stops_being_ancestor_resolved_leaves_the_passes() {
    let (mut doc, cb) =
        absolute_under_a_grandparent("right: 5px; bottom: 5px; width: 40px; height: 20px;");
    cold_frame(&mut doc);
    let mid = NodeId(doc.tree.get(cb.0).unwrap().children[0]);
    doc.set_attribute(mid, "style", "position: relative");
    doc.resolve_layout(VP.0, VP.1);

    doc.tree.perf.reset();
    doc.resolve_layout(VP.0, VP.1 + 20.0);
    let s = doc.tree.perf.end_frame();
    expect(
        "once ancestor-resolved, now its parent's: a later layout",
        &s,
        &[
            (StyleResolves, 1),
            (TaffyStyleSyncs, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
        ],
    );
}

// ── a containing block that is an inline span (#631) ───────────────────────

/// Twenty 20px lines in a 100px-high scroller, each `a <span rel>b[badge]</span>`
/// with a 4px leaf under the last. The span is each badge's containing block
/// and has no box: it is measured in its line. Returns the document, the
/// scroller, the leaf and the first span's text.
fn span_badge_rows(badge_css: &str) -> (RinchDocument, NodeId, NodeId, NodeId) {
    let mut doc = doc_with(&format!(
        ".s {{ height: 100px; width: 300px; overflow: auto; }}
         .line {{ height: 20px; }}
         .rel {{ position: relative; }}
         .leaf {{ height: 4px; }}
         .badge {{ position: absolute; {badge_css} }}"
    ));
    let body = doc.body();
    let s = el(&mut doc, body, "div", "s");
    let mut first_text = None;
    for _ in 0..20 {
        let line = el(&mut doc, s, "div", "line");
        text(&mut doc, line, "a ");
        let span = el(&mut doc, line, "span", "rel");
        let t = text(&mut doc, span, "b");
        first_text.get_or_insert(t);
        el(&mut doc, span, "div", "badge");
    }
    let leaf = el(&mut doc, s, "div", "leaf");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    (doc, s, leaf, first_text.unwrap())
}

/// A badge hung from a span by its position alone. A layout looks at each
/// four times — the size check after the compute, the placement as it is
/// read back, and both again once the lines the span is measured in are
/// built — and runs **one** compute: no size depends on the span. A scroll of
/// the scroller looks at none (it holds the lines; each badge's chain ends at
/// its own line's block).
#[test]
fn position_only_boxes_hung_from_a_span_cost_no_compute() {
    let (mut doc, scroller, leaf, _) =
        span_badge_rows("right: 2px; top: 2px; width: 6px; height: 6px;");
    let s = badge_relayout(&mut doc, leaf);
    expect(
        "span badges, a leaf restyled",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (AbsBoxesVisited, 80),
        ],
    );
    let s = badge_scroll(&mut doc, scroller);
    expect("span badges, the scroller scrolled", &s, &[]);
}

/// A badge **sized** from its span (`inset: 0`). While no span changes size
/// a layout is the same as above. When one span's text grows, the compute
/// runs with that badge baked for the old fragment, the lines are built, the
/// check after them finds the new size, and the layout goes round once:
/// `abs_containing_block_passes` 1, two computes, one style rewritten, the
/// line shaped once to measure and once to paint as for any text edit (the
/// second round re-shapes nothing), and every badge looked at four times per
/// round.
#[test]
fn a_box_sized_from_a_span_costs_one_more_compute_when_the_span_resizes() {
    let (mut doc, _, leaf, first_text) = span_badge_rows("inset: 0;");
    let s = badge_relayout(&mut doc, leaf);
    expect(
        "sized span badges, a leaf restyled",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 1),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (AbsBoxesVisited, 80),
        ],
    );

    doc.tree.perf.reset();
    doc.set_text_content(first_text, "bbbb");
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "sized span badges, one span's text grows",
        &s,
        &[
            (TaffyStyleChanges, 1),
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 3),
            (LayoutResolves, 1),
            (TaffyRootComputes, 2),
            (TaffyMeasureCalls, 2),
            (AbsContainingBlockPasses, 1),
            (AbsBoxesVisited, 160),
            (AbsInlineMeasures, 1),
            (AbsInlineMeasureSteps, 23),
        ],
    );
}

/// `n` badges, each hung from its own span, in **one** paragraph:
/// `w <span rel>x[badge]</span> ` n times. Returns the document and the
/// paragraph.
fn span_badges_in_one_paragraph(n: usize) -> (RinchDocument, NodeId) {
    let mut doc = doc_with(
        ".p { width: 380px; }
         .rel { position: relative; }
         .badge { position: absolute; inset: 0; }",
    );
    let body = doc.body();
    let p = el(&mut doc, body, "div", "p");
    for _ in 0..n {
        text(&mut doc, p, "w ");
        let span = el(&mut doc, p, "span", "rel");
        text(&mut doc, span, "x");
        el(&mut doc, span, "div", "badge");
        text(&mut doc, p, " ");
    }
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    (doc, p)
}

/// The measurement of the spans boxes hang from is **linear in the
/// paragraph**, however many of them it holds (review of #1434, F1: it was
/// one walk of the whole paragraph per box per pass, so 100 boxes in one
/// paragraph cost four times what 50 did, and no counter moved with it).
/// A text edit rebuilds the paragraph's lines; they are measured once, for
/// every span together, and `abs_inline_measure_steps` — entries, ancestors,
/// line items and clusters looked at — doubles when the paragraph does: 854
/// steps for 50 badges, 1,708 for 100 (about 17 a badge).
#[test]
fn spans_in_one_paragraph_are_measured_in_one_linear_walk() {
    let frame = |n: usize| {
        let (mut doc, p) = span_badges_in_one_paragraph(n);
        let first = NodeId(doc.tree.get(p.0).unwrap().children[0]);
        doc.tree.perf.reset();
        doc.set_text_content(first, "ww ");
        doc.resolve_layout(VP.0, VP.1);
        doc.tree.perf.end_frame()
    };
    let (small, large) = (frame(50), frame(100));
    expect(
        "50 span badges in one paragraph, its text edited",
        &small,
        &[
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 3),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (AbsBoxesVisited, 200),
            (AbsInlineMeasures, 1),
            (AbsInlineMeasureSteps, 854),
        ],
    );
    expect(
        "100 span badges in one paragraph, its text edited",
        &large,
        &[
            (ShapeMeasureIfc, 1),
            (ShapeIfcBuild, 1),
            (IfcMeasureInvalidations, 3),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 1),
            (AbsBoxesVisited, 400),
            (AbsInlineMeasures, 1),
            (AbsInlineMeasureSteps, 1708),
        ],
    );
}

/// A row of a list, as an app writes one: a flex row holding a chip in a
/// cell, a label, and a second chip in a cell. The chips are atomic inlines
/// on the lines of **flex items**, whose automatic minimum size is a
/// min-content measure — so each chip is asked for its min-content width
/// (#1476), once.
fn chip_rows() -> (RinchDocument, NodeId, NodeId) {
    let mut doc = doc_with(
        ".list { width: 380px; } .row { display: flex; } .row.hot { color: rgb(200, 10, 10); } \
         .chip { display: inline-flex; padding: 0 4px; } .tag { display: inline-block; }",
    );
    let body = doc.body();
    let list = el(&mut doc, body, "div", "list");
    let mut first = None;
    for _ in 0..10 {
        let row = el(&mut doc, list, "div", "row");
        let cell = el(&mut doc, row, "div", "");
        let chip = el(&mut doc, cell, "span", "chip");
        let chip_text = text(&mut doc, chip, "Needs review");
        let label = el(&mut doc, row, "div", "");
        text(&mut doc, label, "a label");
        let cell = el(&mut doc, row, "div", "");
        let tag = el(&mut doc, cell, "span", "tag");
        text(&mut doc, tag, "two words");
        first.get_or_insert((row, chip_text));
    }
    let (row, chip_text) = first.unwrap();
    (doc, row, chip_text)
}

/// The first layout of ten such rows: each of the twenty chips is measured,
/// then measured at min-content and as `auto` again when its flex item asks
/// (three computes a chip where main has one), and the root is computed a
/// second time with the real widths. main: `inline_block_computes` 20,
/// `taffy_root_computes` 1, `shape_atomic_inline` 50, `shape_measure_ifc` 90,
/// `taffy_measure_calls` 120.
#[test]
fn chips_in_flex_items_first_layout() {
    let (mut doc, _, _) = chip_rows();
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "chips in flex items, first layout",
        &s,
        &[
            (StyleResolves, 62),
            (ElementsCascaded, 63),
            (StyleNodesVisited, 2966),
            (FullStyleWalks, 62),
            (TaffyStyleSyncs, 63),
            (TaffyStyleChanges, 62),
            (ShapeMeasureIfc, 150),
            (ShapeIfcBuild, 40),
            (ShapeAtomicInline, 100),
            (IfcMeasureCacheHits, 50),
            (IfcMeasureInvalidations, 131),
            (IfcSignatureChanges, 40),
            (IfcPhantomRebreaks, 40),
            (LayoutResolves, 1),
            (IfcSetupPasses, 1),
            (IfcFullPasses, 1),
            (IfcFullInitial, 1),
            (TaffyRootComputes, 2),
            (TaffyMeasureCalls, 200),
            (InlineBlockComputes, 60),
        ],
    );
}

/// A colour-only hover of one row costs what it costs with no chip in a
/// flex item: the chips are measured again (their glyphs are new), their
/// min-content widths stand, and nothing asks for them — no min-content
/// compute, no second root compute.
#[test]
fn a_colour_hover_on_a_row_of_chips_in_flex_items() {
    let (mut doc, row, _) = chip_rows();
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc.tree.perf.reset();
    doc.set_attribute(row, "class", "row hot");
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "colour hover on a row of chips in flex items",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 6),
            (StyleNodesVisited, 6),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 6),
            (ShapeIfcBuild, 4),
            (IfcMeasureInvalidations, 6),
            (LayoutResolves, 1),
            (LayoutSkippedTextOnly, 1),
        ],
    );
}

/// The same hover in a frame that also lays out (the list is resized by a
/// pixel): the row's two chips are measured again, once each, as on main.
/// Their min-content widths stand, since a colour re-wraps nothing — dropped,
/// each chip paid the min-content compute and the `auto` measure after it
/// (6 computes) and the root was computed twice (review of #1488).
#[test]
fn a_colour_hover_with_a_relayout_on_a_row_of_chips_in_flex_items() {
    let (mut doc, row, _) = chip_rows();
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc.tree.perf.reset();
    doc.set_attribute(row, "class", "row hot");
    let list = doc.tree.get(row.0).unwrap().parent.unwrap();
    doc.set_style(NodeId(list), "width", "379px");
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "colour hover with a relayout on a row of chips in flex items",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 7),
            (StyleNodesVisited, 16),
            (StyleInvalidations, 2),
            (TaffyStyleSyncs, 7),
            (TaffyStyleChanges, 1),
            (ShapeMeasureIfc, 9),
            (ShapeIfcBuild, 4),
            (ShapeAtomicInline, 5),
            (IfcMeasureCacheHits, 57),
            (IfcMeasureInvalidations, 6),
            (IfcPhantomRebreaks, 2),
            (LayoutResolves, 1),
            (TaffyRootComputes, 1),
            (TaffyMeasureCalls, 66),
            (InlineBlockComputes, 2),
        ],
    );
}

/// A text edit in one chip: its min-content width is measured again (the
/// compute, and the `auto` measure after it), and the root computed twice.
/// main: `inline_block_computes` 1, `taffy_root_computes` 1,
/// `shape_atomic_inline` 4, `shape_measure_ifc` 3, `taffy_measure_calls` 4.
#[test]
fn a_text_edit_in_a_chip_in_a_flex_item() {
    let (mut doc, _, chip_text) = chip_rows();
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc.tree.perf.reset();
    doc.set_text_content(chip_text, "Was reviewed");
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "text edit in a chip in a flex item",
        &s,
        &[
            (ShapeMeasureIfc, 6),
            (ShapeIfcBuild, 1),
            (ShapeAtomicInline, 7),
            (IfcMeasureCacheHits, 2),
            (IfcMeasureInvalidations, 6),
            (IfcPhantomRebreaks, 2),
            (LayoutResolves, 1),
            (TaffyRootComputes, 2),
            (TaffyMeasureCalls, 8),
            (InlineBlockComputes, 3),
        ],
    );
}

/// A one-pixel resize of the list while every chip is **capped** (the list
/// is narrower than a row's content, so each cell is narrower than its
/// chip): each chip is resolved against a cell that moved, which is one
/// layout at the new width (its min- and max-content widths are remembered;
/// it was two computes until the max-content probe was skipped for a box
/// still resolved) and a second root compute — a cost per chip per pixel,
/// where main (whose chips are never capped, so the layout is not Chrome's)
/// measures and shapes nothing: `inline_block_computes` 0,
/// `taffy_root_computes` 1, no shape.
#[test]
fn a_one_pixel_resize_with_every_chip_capped() {
    let (mut doc, row, _) = chip_rows();
    let list = NodeId(doc.tree.get(row.0).unwrap().parent.unwrap());
    doc.set_style(list, "width", "150px");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc.tree.perf.reset();
    doc.set_style(list, "width", "149px");
    doc.resolve_layout(VP.0, VP.1);
    let s = doc.tree.perf.end_frame();
    expect(
        "one-pixel resize with every chip capped",
        &s,
        &[
            (StyleResolves, 1),
            (ElementsCascaded, 1),
            (StyleNodesVisited, 11),
            (StyleInvalidations, 1),
            (TaffyStyleSyncs, 1),
            (TaffyStyleChanges, 1),
            (ShapeMeasureIfc, 50),
            (ShapeIfcBuild, 10),
            (ShapeAtomicInline, 30),
            (IfcMeasureCacheHits, 50),
            (IfcMeasureInvalidations, 10),
            (IfcPhantomRebreaks, 10),
            (LayoutResolves, 1),
            (TaffyRootComputes, 2),
            (TaffyMeasureCalls, 100),
            (InlineBlockComputes, 20),
        ],
    );
}
