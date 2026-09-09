//! The anonymous-box passes rebuild a Taffy child list from the **same**
//! authority `sync_display_contents` does (#476).
//!
//! `sync_display_contents` builds a parent's Taffy child list from
//! `collect_effective_taffy_children`, which replaces every `display: contents`
//! child with the boxes it flattens. `cleanup_anonymous_block_boxes` and
//! `create_anonymous_block_boxes` ran immediately afterwards in the same
//! `ifc_dirty` block and rebuilt the same list from **raw
//! `nodes[parent].children`** — a rule that cannot see the flattening. They
//! therefore re-added the wrapper's own boxless Taffy node and dropped the
//! grandchildren it stands for; and because a flattened grandchild is not a DOM
//! child of the parent, nothing re-added it anywhere. It was left **orphaned**
//! in Taffy: laid out `0x0`, painted not at all, on that pass and on every pass
//! after it.
//!
//! Only a **plain block** parent could reach this: `create_anonymous_block_boxes`
//! skips `DisplayMode::Flex` outright, and the whole shipped `display: contents`
//! suite is written over flex containers — which is why a defect that erases
//! content lived behind a green board. Every fixture here uses a block parent,
//! and the flex twin is kept as the control that says so.
//!
//! **Fixed points avoided on purpose** (this project's recurring test failure):
//! one block in the wrapper cannot tell "orphan the set" from "orphan the
//! first", so there is a two-block fixture; a block at the end cannot tell an
//! off-by-one from a drop, so there is a block-first fixture; and `y == 0` is
//! where a laid-out box and an orphan agree, so the text sibling above gives
//! every other fixture a non-zero expected offset.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

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

fn rect(doc: &RinchDocument, id: NodeId) -> (f32, f32, f32, f32) {
    let l = doc.tree.get(id.0).unwrap().layout;
    (l.x, l.y, l.width, l.height)
}

fn height_of(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.height
}

/// Whether a node's box is reachable from the Taffy root at all. An orphan —
/// a node with a `taffy_id` that no parent's child list holds — answers
/// `false`, which is the #476 symptom stated structurally rather than through
/// the geometry it produces.
fn attached_to_taffy(doc: &RinchDocument, id: NodeId) -> bool {
    doc.tree
        .get(id.0)
        .and_then(|n| n.taffy_id)
        .and_then(|t| doc.tree.taffy.parent(t))
        .is_some()
}

/// `<div>label<div style="display: contents"><div h=40/></div></div>` — the
/// issue's own markup — plus the identical markup with the wrapper removed.
/// Returns `(doc, container, block)` for each.
fn wrapped_and_plain() -> (
    (RinchDocument, NodeId, NodeId),
    (RinchDocument, NodeId, NodeId),
) {
    let wrapped = {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let outer = child_of(&mut doc, body, "div", "font-size: 16px");
        text_in(&mut doc, outer, "label");
        let wrapper = child_of(&mut doc, outer, "div", "display: contents");
        let blk = child_of(&mut doc, wrapper, "div", "height: 40px; background: red");
        doc.resolve_layout(VW, VH);
        (doc, outer, blk)
    };
    let plain = {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let outer = child_of(&mut doc, body, "div", "font-size: 16px");
        text_in(&mut doc, outer, "label");
        let blk = child_of(&mut doc, outer, "div", "height: 40px; background: red");
        doc.resolve_layout(VW, VH);
        (doc, outer, blk)
    };
    (wrapped, plain)
}

// ── The reported bug ────────────────────────────────────────────────────────

/// A `display: contents` wrapper generates no box, so wrapping a block in one
/// must not move it, resize it, or remove it. The oracle is the identical
/// markup with the wrapper deleted, measured in the same test — no hard-coded
/// line height to drift.
///
/// Kills: rebuilding the parent's Taffy children from raw
/// `nodes[parent].children` in `create_anonymous_block_boxes`. With that mutant
/// the block is orphaned — `(0, 0, 0x0)`, no Taffy parent — and the container
/// keeps only the text's height.
#[test]
fn a_wrapped_block_beside_text_is_laid_out_exactly_where_an_unwrapped_one_is() {
    let ((wdoc, wouter, wblk), (pdoc, pouter, pblk)) = wrapped_and_plain();

    assert!(
        attached_to_taffy(&wdoc, wblk),
        "the wrapped block must be reachable from the Taffy root; an orphan is \
         laid out by nothing and painted by nothing"
    );
    assert_eq!(
        rect(&wdoc, wblk),
        rect(&pdoc, pblk),
        "a boxless wrapper must not change its child's box"
    );
    assert_eq!(
        height_of(&wdoc, wouter),
        height_of(&pdoc, pouter),
        "nor the container's height"
    );

    // Off the fixed point: the text line above gives the block a non-zero
    // offset, so "laid out" and "collapsed at the origin" cannot agree.
    let (_, y, w, h) = rect(&wdoc, wblk);
    assert!(
        y > 0.0 && w > 0.0 && h > 0.0,
        "the fixture must not sit at the origin, or an orphan would pass: got \
         y={y}, {w}x{h}"
    );
}

/// The same wrapper with the block **first** and the text after it. A drop and
/// an off-by-one insert are the same picture when the wrapped box is last, so
/// this is the half of the pair that tells them apart: here the block belongs
/// at `y == 0` and the text below it.
///
/// Kills: the same raw-DOM rebuild, and any "append the flattened boxes at the
/// end" repair of it.
#[test]
fn a_wrapped_block_before_the_text_is_laid_out_at_the_top() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child_of(&mut doc, body, "div", "font-size: 16px");
    let wrapper = child_of(&mut doc, outer, "div", "display: contents");
    let blk = child_of(&mut doc, wrapper, "div", "height: 40px; background: red");
    text_in(&mut doc, outer, "label");
    doc.resolve_layout(VW, VH);

    let (x, y, w, h) = rect(&doc, blk);
    assert_eq!(
        (x, y, w, h),
        (0.0, 0.0, VW, 40.0),
        "the wrapped block comes first in document order, so it takes the top"
    );
    assert!(
        height_of(&doc, outer) > 40.0,
        "and the text line still follows it: container height {} should exceed \
         the block's 40",
        height_of(&doc, outer)
    );
}

/// **Two** blocks under one wrapper. One is the arity fixed point — "orphan
/// every flattened box" and "orphan the first" produce the same picture — so
/// the single-block fixtures above cannot distinguish a rebuild that drops the
/// whole flattened run from one that drops all but its head.
///
/// Kills: a rebuild that flattens only the wrapper's first child.
#[test]
fn both_blocks_under_one_wrapper_are_laid_out() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child_of(&mut doc, body, "div", "font-size: 16px");
    text_in(&mut doc, outer, "label");
    let wrapper = child_of(&mut doc, outer, "div", "display: contents");
    let first = child_of(&mut doc, wrapper, "div", "height: 40px; background: red");
    let second = child_of(&mut doc, wrapper, "div", "height: 25px; background: red");
    doc.resolve_layout(VW, VH);

    assert!(attached_to_taffy(&doc, first), "first block orphaned");
    assert!(attached_to_taffy(&doc, second), "second block orphaned");

    let (_, y1, _, h1) = rect(&doc, first);
    let (_, y2, _, h2) = rect(&doc, second);
    assert_eq!((h1, h2), (40.0, 25.0), "both keep their declared heights");
    assert!(y2 > y1, "and stack in document order: y1={y1}, y2={y2}");
    assert_eq!(
        height_of(&doc, outer),
        y1 + h1 + h2,
        "the container's height counts both"
    );
}

/// The orphaning was **stable**: `sync_display_contents` re-spliced the block
/// into the container on every pass and the anonymous-box rebuild orphaned it
/// again, so it never self-healed. A repeated-pass fixture is what says the fix
/// is a fix and not a first-pass coincidence.
///
/// Kills: any repair applied only to the first layout pass.
#[test]
fn the_wrapped_block_stays_laid_out_across_repeated_passes() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child_of(&mut doc, body, "div", "font-size: 16px");
    text_in(&mut doc, outer, "label");
    let wrapper = child_of(&mut doc, outer, "div", "display: contents");
    let blk = child_of(&mut doc, wrapper, "div", "height: 40px; background: red");

    let mut seen = Vec::new();
    for _ in 0..4 {
        doc.resolve_layout(VW, VH);
        seen.push(rect(&doc, blk));
    }

    assert!(
        seen.iter().all(|r| *r == seen[0]),
        "the geometry must be stable across passes: {seen:?}"
    );
    let (_, y, w, h) = seen[0];
    assert!(
        y > 0.0 && (w, h) == (VW, 40.0),
        "and it must be the laid-out geometry, not a stable zero: {:?}",
        seen[0]
    );
}

/// Wrappers nested to any depth flatten the same way — the authority recurses,
/// so the rebuild that consumes it must inherit that for free.
#[test]
fn nested_wrappers_flatten_the_same_way() {
    let ((_, _, _), (pdoc, pouter, pblk)) = wrapped_and_plain();

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child_of(&mut doc, body, "div", "font-size: 16px");
    text_in(&mut doc, outer, "label");
    let w1 = child_of(&mut doc, outer, "div", "display: contents");
    let w2 = child_of(&mut doc, w1, "div", "display: contents");
    let blk = child_of(&mut doc, w2, "div", "height: 40px; background: red");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        rect(&doc, blk),
        rect(&pdoc, pblk),
        "two boxless wrappers are still no box"
    );
    assert_eq!(height_of(&doc, outer), height_of(&pdoc, pouter));
}

/// The control that names the culprit. `create_anonymous_block_boxes` skips
/// `DisplayMode::Flex`, so a flex parent never reached the broken rebuild and
/// laid the wrapped block out correctly the whole time. It must still.
///
/// Without this, "the contents flattening is broken" would be an equally good
/// explanation of the block fixtures, and a fix aimed at `sync_display_contents`
/// would look right.
#[test]
fn a_flex_parent_was_never_affected_and_still_is_not() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; display: flex; flex-direction: column",
    );
    text_in(&mut doc, outer, "label");
    let wrapper = child_of(&mut doc, outer, "div", "display: contents");
    let blk = child_of(&mut doc, wrapper, "div", "height: 40px; background: red");
    doc.resolve_layout(VW, VH);

    let (_, y, w, h) = rect(&doc, blk);
    assert!(
        y > 0.0 && (w, h) == (VW, 40.0),
        "the flex control must be laid out below the text: {:?}",
        rect(&doc, blk)
    );
}

// ── The same defect reached through the other flattened roles ───────────────

/// A **transparent** wrapper (one holding only an out-of-flow box, #518) inside
/// a container that is mixed for an independent reason. The container's
/// anonymous-box rebuild flattens it exactly like an opaque one, so its
/// absolute has to survive too — it was orphaned before, `0x0`, never painted.
///
/// This is the shape `ifc_contents_transparency_tests`'
/// `a_transparent_wrapper_does_not_split_a_run_in_an_independently_mixed_container`
/// already builds and only asserts the run grouping of.
///
/// Kills: flattening only wrappers the run loop judged opaque.
#[test]
fn an_absolute_under_a_transparent_wrapper_in_a_mixed_container_is_laid_out() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = child_of(&mut doc, body, "div", "width: 400px; line-height: 20px");
    text_in(&mut doc, c, "before ");
    let wrapper = child_of(&mut doc, c, "span", "display: contents");
    let abs = child_of(
        &mut doc,
        wrapper,
        "div",
        "position: absolute; left: 0; top: 0; width: 10px; height: 10px",
    );
    text_in(&mut doc, c, " after");
    // The independent reason the container is mixed.
    child_of(&mut doc, c, "div", "width: 40px; height: 20px");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        rect(&doc, abs),
        (0.0, 0.0, 10.0, 10.0),
        "the absolute must be laid out; (0, 0, 0, 0) is the orphan"
    );
    assert!(attached_to_taffy(&doc, abs));
}

/// A transparent wrapper holding **inline** content, in the same
/// independently-mixed container. The wrapper's `<span>` was orphaned and its
/// text vanished; it must be reachable.
///
/// Deliberately weak about *where*: the run grouping declines to put a
/// transparent wrapper in the inline run, so today the span is laid out by
/// Taffy as its own box on its own line rather than joining the anonymous box's
/// line. That is a separate, pre-existing gap in the run grouping; asserting
/// only reachability lets it be closed without rewriting this test.
///
/// Kills: a fix that flattens only the roles the run loop happens to look at.
#[test]
fn a_transparent_wrappers_inline_content_stays_reachable() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = child_of(&mut doc, body, "div", "width: 400px; line-height: 20px");
    text_in(&mut doc, c, "before ");
    let wrapper = child_of(&mut doc, c, "span", "display: contents");
    let span = child_of(&mut doc, wrapper, "span", "");
    let inner = text_in(&mut doc, span, "MIDDLE");
    text_in(&mut doc, c, " after");
    child_of(&mut doc, c, "div", "width: 40px; height: 20px");
    doc.resolve_layout(VW, VH);

    let claimed_by_an_ifc = doc.tree.get(span.0).unwrap().ifc_root.is_some();
    assert!(
        attached_to_taffy(&doc, span) || claimed_by_an_ifc,
        "the wrapped inline content must be laid out by somebody — Taffy or an \
         IFC. Neither is the orphan."
    );
    let (_, _, w, h) = rect(&doc, inner);
    assert!(
        w > 0.0 && h > 0.0,
        "and its text must have a box: got {w}x{h}"
    );
}

// ── The cleanup half of the pair, isolated ─────────────────────────────────

/// `cleanup_anonymous_block_boxes` rebuilds the same list, and on a pass where
/// the container stops being mixed it is the **only** rebuild that runs —
/// `create_anonymous_block_boxes` returns before its own.
///
/// The fixture reaches that: pass 1 is `span + wrapper > block`, which is mixed
/// and mints an anonymous box; pass 2 hides the span, so `has_inline` is false,
/// creation is skipped, and the cleanup rebuild alone decides where the
/// wrapped block goes. With cleanup on raw DOM order the block is orphaned on
/// pass 2 — laid out and painted on pass 1, gone on pass 2, from a change that
/// did not touch it.
///
/// Kills: fixing `create_anonymous_block_boxes` and leaving
/// `cleanup_anonymous_block_boxes` on raw `nodes[parent].children`.
#[test]
fn the_cleanup_rebuild_uses_the_same_authority() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = child_of(&mut doc, body, "div", "font-size: 16px");
    let label = child_of(&mut doc, outer, "span", "");
    text_in(&mut doc, label, "label");
    let wrapper = child_of(&mut doc, outer, "div", "display: contents");
    let blk = child_of(&mut doc, wrapper, "div", "height: 40px; background: red");

    doc.resolve_layout(VW, VH);
    assert!(
        attached_to_taffy(&doc, blk),
        "precondition: the creation pass attaches it"
    );
    let before = rect(&doc, blk);

    // Hiding the inline sibling makes the container un-mixed, so this pass
    // runs the cleanup rebuild and skips the creation one.
    doc.set_attribute(label, "style", "display: none");
    doc.resolve_layout(VW, VH);

    assert!(
        attached_to_taffy(&doc, blk),
        "the block was attached on pass 1 and orphaned on pass 2 — the cleanup \
         rebuild dropped it"
    );
    let (_, y, w, h) = rect(&doc, blk);
    assert_eq!(
        (w, h),
        (VW, 40.0),
        "and it keeps its box: was {before:?}, now {:?}",
        rect(&doc, blk)
    );
    assert_eq!(
        y, 0.0,
        "with the text hidden it moves to the top, not to nowhere"
    );
    assert_eq!(height_of(&doc, outer), 40.0);
}

// ── Whose list, not just what is in it ─────────────────────────────────────

/// A `display: contents` wrapper computes `DisplayMode::Block`, so a wrapper
/// holding `text + block` is itself classified as mixed content and mints the
/// anonymous box **inside a boxless element**. The net structure is still
/// right, because the flattening lifts that anonymous box into the ancestor's
/// list — but only if the ancestor's list is rebuilt after the box exists.
/// `sync_display_contents` built it before, and nothing re-runs it.
///
/// Kills: rebuilding only the anonymous box's own DOM parent. Taffy's
/// `set_children` steals each adopted child from its previous parent, so
/// rebuilding just the wrapper empties the container's list and collapses it to
/// `h = 0` with its content laid out nowhere.
#[test]
fn an_anonymous_box_minted_inside_a_wrapper_does_not_strand_the_ancestor() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    let wrapper = child_of(&mut doc, container, "span", "display: contents");
    text_in(&mut doc, wrapper, "VISIBLE");
    let blk = child_of(&mut doc, wrapper, "div", "height: 30px; background: red");
    doc.resolve_layout(VW, VH);

    assert!(
        attached_to_taffy(&doc, blk),
        "the wrapped block is orphaned"
    );
    let (_, y, _, h) = rect(&doc, blk);
    assert_eq!(h, 30.0);
    assert!(
        y > 0.0,
        "the block sits below the wrapped text's line, got y={y}"
    );
    assert_eq!(
        height_of(&doc, container),
        y + h,
        "the container measures the text line plus the block, not 0"
    );
}

// ── The local pixel oracle ─────────────────────────────────────────────────

/// Layout numbers say the box exists; only pixels say it is drawn. The
/// fixtures below fill exactly one region in pure red and nothing else is red,
/// so the correct count is provably `width * height` and the broken one is
/// provably **zero** — the local oracle a whole-screen comparison cannot give
/// (a defect this size is a fraction of a percent of the frame).
#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    /// Count of pixels that are exactly opaque `rgb(255, 0, 0)`.
    fn red_pixels(doc: &mut RinchDocument) -> usize {
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
        painter
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| **p == [255, 0, 0, 255])
            .count()
    }

    /// The wrapped block paints the same 800x40 = 32000 red pixels the
    /// unwrapped one does. Broken: **0** — not mispositioned, absent.
    #[test]
    fn a_wrapped_block_paints_the_same_pixels_as_an_unwrapped_one() {
        let ((mut wdoc, _, _), (mut pdoc, _, _)) = wrapped_and_plain();

        let plain = red_pixels(&mut pdoc);
        assert_eq!(
            plain,
            (VW as usize) * 40,
            "control: the unwrapped block fills its whole box"
        );
        assert_eq!(
            red_pixels(&mut wdoc),
            plain,
            "the wrapped block must paint exactly the same pixels; 0 is the \
             orphan, which is drawn by nothing"
        );
    }

    /// Two wrapped blocks paint both boxes — 800x40 + 800x25. One block cannot
    /// tell "paints none of them" from "paints all but the first".
    #[test]
    fn two_wrapped_blocks_both_paint() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let outer = child_of(&mut doc, body, "div", "font-size: 16px");
        text_in(&mut doc, outer, "label");
        let wrapper = child_of(&mut doc, outer, "div", "display: contents");
        child_of(&mut doc, wrapper, "div", "height: 40px; background: red");
        child_of(&mut doc, wrapper, "div", "height: 25px; background: red");
        doc.resolve_layout(VW, VH);

        assert_eq!(
            red_pixels(&mut doc),
            (VW as usize) * 65,
            "both wrapped blocks must be drawn"
        );
    }
}
