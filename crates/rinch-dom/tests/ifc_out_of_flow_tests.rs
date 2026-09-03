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
/// of an absolute box became two runs → two anonymous boxes → two lines. And
/// `has_block` counted the out-of-flow child as block content at all, so a box
/// CSS would never create was minted — with #466's measure-child in place,
/// none is (#406 complete).
///
/// The anon-count assertion is the load-bearing half: the height stays green
/// *via the anonymous box* if `has_block` regresses, so without this pin the
/// predicate flip is unpinned.
#[test]
fn text_either_side_of_an_absolute_child_stays_on_one_line_with_no_anon_box() {
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
        0,
        "an out-of-flow child is not block content — no anonymous box (CSS 2.1 \
         §9.2.1.1); the container's own IFC flows the text and its measure \
         fires through the #466 measure leaf"
    );
    let h = height_of(&doc, row);
    assert!(
        (h - 20.0).abs() < 1.0,
        "both text runs share one line box, so the row is one line tall, got {h}"
    );
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
}

// ── the trap: fixing `has_block` alone strands the text ─────────────────────
//
// These two were written as a **canary for #466**: they were green *because*
// the anonymous-box split kept each run in its own IFC, and removing those
// boxes was what would make them fail first. #466's measure-child landed, the
// anonymous boxes are gone for this shape, and these now hold through the
// container's own IFC — which makes them the ink proof of
// `walk_inline_children`'s out-of-flow arm.
//
// That arm is what they kill now: without it, an out-of-flow child (blockified,
// so it lands in the catch-all) `break`s the walk, and text after it — already
// Taffy-detached and marked by `mark_inline_descendants` — reaches neither
// Parley nor Taffy and simply vanishes. The container's height stays one line
// either way, so a layout number cannot see this; ink can.

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

// ── #466: the measure-child ─────────────────────────────────────────────────
//
// With no anonymous box, a `text + absolute` container is itself the IFC root
// while its out-of-flow children stay attached as Taffy children — and Taffy
// consults a measure function only on a childless node, so the root's measure
// would be structurally unreachable and its auto height would collapse to 0.
// `setup_inline_formatting_contexts` therefore hands the `InlineRoot` context
// to a Taffy-only anonymous leaf child (index 0), leaving the out-of-flow
// children attached so Taffy keeps doing 100% of their layout. Each test below
// names the production mutation it kills.

use rinch_dom::NodeContext;

fn layout_of(doc: &RinchDocument, node: NodeId) -> (f32, f32, f32, f32) {
    let l = doc.tree.get(node.0).unwrap().layout;
    (l.x, l.y, l.width, l.height)
}

/// The headline defect: an auto-height `text + absolute` container gets its
/// line height (plus its own padding — asymmetric, so a mutant that drops or
/// doubles one side cannot hide at a zero/identity value), in both the
/// `position: relative` and `position: static` rows of #466's matrix.
///
/// Kills: flipping `has_block` without the measure-child — the container
/// becomes a non-leaf `InlineRoot` carrier, the in-setup validator panics in
/// debug builds, and the height collapses to padding-only without it.
#[test]
fn a_text_plus_absolute_container_gets_its_line_height() {
    for container_position in ["position: relative; ", ""] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = child_of(
            &mut doc,
            body,
            "div",
            &format!(
                "{container_position}font-size: 16px; line-height: 20px; \
                 width: 400px; padding: 7px 0 3px 0"
            ),
        );
        text_in(&mut doc, container, "hello world");
        child_of(
            &mut doc,
            container,
            "div",
            "position: absolute; left: 32px; top: 9px; width: 50px; height: 10px",
        );
        doc.resolve_layout(VW, VH);

        assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
        let h = height_of(&doc, container);
        assert!(
            (h - 30.0).abs() < 2.0,
            "one 20px line + 7px/3px padding = 30 expected for \
             '{container_position}' container, got {h}"
        );
    }
}

/// The context is a **move** onto a childless measure leaf: the leaf carries
/// `InlineRoot(container)`, sits at index 0, and the container itself — a
/// non-leaf now — carries no `InlineRoot`.
///
/// Kills: copying the context instead of moving it (the container assertion —
/// and the validator panics in-setup in debug builds); appending the leaf
/// instead of inserting at 0 (the index assertion, behaviorally pinned by the
/// static-position test below); dropping the leaf from
/// `ifc_leaf_invariant_violations`' walk is killed separately below.
#[test]
fn the_measure_child_carries_the_moved_context() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, container, "hello world");
    let abs = child_of(
        &mut doc,
        container,
        "div",
        "position: absolute; left: 32px; top: 9px; width: 50px; height: 10px",
    );
    doc.resolve_layout(VW, VH);

    let container_taffy = doc.tree.get(container.0).unwrap().taffy_id.unwrap();
    let &leaf = doc
        .tree
        .ifc_measure_leaves
        .get(&container.0)
        .expect("a text+absolute root gets a measure leaf");
    assert!(
        matches!(
            doc.tree.taffy.get_node_context(leaf),
            Some(NodeContext::InlineRoot(id)) if *id == container.0
        ),
        "the leaf carries InlineRoot naming the container"
    );
    assert_eq!(
        doc.tree.taffy.children(leaf).unwrap().len(),
        0,
        "the carrier must be childless — Taffy never consults the measure of \
         a node with children"
    );
    let children = doc.tree.taffy.children(container_taffy).unwrap();
    assert_eq!(
        children.first().copied(),
        Some(leaf),
        "the leaf sits at index 0, before the out-of-flow children"
    );
    let abs_taffy = doc.tree.get(abs.0).unwrap().taffy_id.unwrap();
    assert!(
        children.contains(&abs_taffy),
        "the absolute child stays attached — Taffy does its layout"
    );
    assert!(
        !matches!(
            doc.tree.taffy.get_node_context(container_taffy),
            Some(NodeContext::InlineRoot(_))
        ),
        "a move, not a copy: the container (a non-leaf) must not keep the context"
    );
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
}

/// The absolute child is laid out by Taffy — and **painted** — at its
/// container-relative insets, with `ifc_root == None`: the IFC only measures
/// the inline content, it never claims the out-of-flow box. The ink assertion
/// is on the rasterised pixmap because `ifc_root` means "drawn by that IFC",
/// and the IFC never draws this box — a layout number stays plausible for a
/// box paint skips.
///
/// Kills: stamping `ifc_root` on out-of-flow children (paint skips the box —
/// no red ink, and the direct `ifc_root` assertion fails first), and a
/// canonicalization that drops the child (its layout stays zero).
#[test]
fn an_absolute_child_lays_out_and_paints_at_container_relative_insets() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; \
         width: 400px; height: 100px; background: rgb(0, 0, 255)",
    );
    text_in(&mut doc, container, "hello world");
    let abs = child_of(
        &mut doc,
        container,
        "div",
        "position: absolute; left: 32px; top: 9px; width: 50px; height: 10px; \
         background: rgb(255, 0, 0)",
    );
    doc.resolve_layout(VW, VH);

    assert_eq!(
        doc.tree.get(abs.0).unwrap().ifc_root,
        None,
        "an out-of-flow child is nobody's inline content"
    );
    let (x, y, w, h) = layout_of(&doc, abs);
    assert!(
        (x - 32.0).abs() < 0.5 && (y - 9.0).abs() < 0.5,
        "insets resolve against the positioned container, got ({x}, {y})"
    );
    assert!(
        (w - 50.0).abs() < 0.5 && (h - 10.0).abs() < 0.5,
        "explicit size honored, got ({w}, {h})"
    );

    // Ink: the red box sits exactly `insets` inside the blue container's
    // painted origin, wherever the container itself landed on screen.
    let px = rasterize(&mut doc);
    let (cx, cy, ..) = color_bbox(&px, (0, 0, 255)).expect("the container is painted");
    let (rx0, ry0, rx1, ry1) = color_bbox(&px, (255, 0, 0)).expect(
        "the absolute child is painted at all — no red ink means paint \
         skipped it (an `ifc_root` stamp does exactly that)",
    );
    assert_eq!(
        (rx0, ry0, rx1, ry1),
        (cx + 32, cy + 9, cx + 32 + 50, cy + 9 + 10),
        "the box is inked exactly at its container-relative insets"
    );
}

/// An absolute child with **auto** insets keeps its static position — below
/// the text line, where its flow position would be — because the measure leaf
/// goes in at index 0, ahead of it.
///
/// Kills: appending the leaf instead of inserting it at index 0, which would
/// silently move every auto-inset absolute child to content-top (y = 0 — the
/// natural identity value; the 20px line above it is what samples off it).
#[test]
fn an_auto_inset_absolute_child_keeps_its_below_the_line_static_position() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, container, "hello world");
    let abs = child_of(
        &mut doc,
        container,
        "div",
        "position: absolute; width: 50px; height: 10px",
    );
    doc.resolve_layout(VW, VH);

    let (_x, y, _w, _h) = layout_of(&doc, abs);
    assert!(
        (y - 20.0).abs() < 2.0,
        "static position sits below the 20px text line, got y = {y}"
    );
}

/// A runtime `static → absolute` toggle re-runs IFC setup, and the reverse
/// toggle undoes it: `apply_stylo_styles_to_taffy` must set `ifc_dirty` on a
/// Taffy-`position` change, not only on a display change.
///
/// Kills: omitting the position trigger — the toggle then never re-runs setup,
/// the stale anonymous box survives the first leg (count stays 1) and never
/// comes back on the second (count stays 0).
#[test]
fn a_runtime_position_toggle_reruns_ifc_setup() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, container, "hello world");
    let child = child_of(&mut doc, container, "div", "width: 50px; height: 30px");
    doc.resolve_layout(VW, VH);

    assert_eq!(anon_box_count(&doc), 1, "precondition: mixed content");
    assert!(
        !doc.tree.ifc_measure_leaves.contains_key(&container.0),
        "precondition: no measure leaf while an in-flow block is present"
    );
    let h = height_of(&doc, container);
    assert!(
        (h - 50.0).abs() < 2.0,
        "precondition: line + block, got {h}"
    );

    // static → absolute: the anonymous box must go, the measure leaf must come.
    doc.set_attribute(
        child,
        "style",
        "position: absolute; left: 5px; top: 5px; width: 50px; height: 30px",
    );
    doc.resolve_layout(VW, VH);
    assert_eq!(
        anon_box_count(&doc),
        0,
        "the toggle must re-run IFC setup — the anonymous box is stale"
    );
    assert!(
        doc.tree.ifc_measure_leaves.contains_key(&container.0),
        "the leaf decision must be re-made for the now-out-of-flow child"
    );
    let h = height_of(&doc, container);
    assert!(
        (h - 20.0).abs() < 2.0,
        "one line, block out of flow, got {h}"
    );
    let (x, y, ..) = layout_of(&doc, child);
    assert!(
        (x - 5.0).abs() < 0.5 && (y - 5.0).abs() < 0.5,
        "the toggled child is laid out at its insets, got ({x}, {y})"
    );
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());

    // absolute → static: the anonymous box must come back, the leaf must go.
    doc.set_attribute(child, "style", "width: 50px; height: 30px");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        anon_box_count(&doc),
        1,
        "in flow again: mixed content again"
    );
    assert!(
        !doc.tree.ifc_measure_leaves.contains_key(&container.0),
        "the measure leaf is torn down when no out-of-flow child remains"
    );
    let h = height_of(&doc, container);
    assert!((h - 50.0).abs() < 2.0, "line + block again, got {h}");
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
}

/// A text edit reaches the measure through Taffy's per-node cache: marking the
/// *root* dirty is not enough, because dirty propagates up and the measure is
/// cached on the leaf below it.
///
/// Kills: omitting `mark_ifc_measure_dirty` from `invalidate_parent_ifc` — the
/// leaf serves its cached one-line measure and the container's height never
/// changes. (The edit goes through `set_text_content`, which sets
/// `layout_dirty` but not `ifc_dirty`, so the leaf survives the edit — the
/// exact configuration in which the cache is the only thing consulted.)
#[test]
fn a_text_edit_updates_a_measure_child_container_height() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; width: 120px",
    );
    let text = text_in(&mut doc, container, "hi");
    child_of(
        &mut doc,
        container,
        "div",
        "position: absolute; left: 100px; top: 0; width: 10px; height: 10px",
    );
    doc.resolve_layout(VW, VH);

    let h1 = height_of(&doc, container);
    assert!((h1 - 20.0).abs() < 2.0, "precondition: one line, got {h1}");
    assert!(
        doc.tree.ifc_measure_leaves.contains_key(&container.0),
        "precondition: the measure leaf exists"
    );

    doc.set_text_content(text, "aaaa bbbb cccc dddd eeee ffff gggg");
    doc.resolve_layout(VW, VH);

    let h2 = height_of(&doc, container);
    assert!(
        h2 > 30.0,
        "the longer text wraps in a 120px column, so the container must grow \
         past one line; got {h2} (was {h1}) — a stale leaf measure serves the \
         old height"
    );
}

/// An absolute child inserted between frames is attached by the
/// canonicalization, even when the DOM-index → Taffy-index computation
/// misfires: `compute_taffy_child_index` counts detached (inline) siblings
/// that still have a `taffy_id`, overshoots, and Taffy's out-of-range error is
/// swallowed — the child is silently attached nowhere (#477). The next setup
/// pass rebuilds the container's Taffy children from the DOM.
///
/// Kills: omitting the canonicalization, or deciding the leaf branch from the
/// current Taffy attachment instead of the DOM — either leaves the new box
/// unattached, never laid out, layout stuck at zero. (Three detached text
/// siblings make the computed index overshoot the two attached children, so
/// the raw insert really does fail first.)
#[test]
fn a_late_inserted_absolute_child_is_attached_by_canonicalization() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, container, "aa ");
    text_in(&mut doc, container, "bb ");
    text_in(&mut doc, container, "cc");
    let abs1 = child_of(
        &mut doc,
        container,
        "div",
        "position: absolute; left: 200px; top: 0; width: 20px; height: 6px",
    );
    doc.resolve_layout(VW, VH);
    assert!(
        doc.tree.ifc_measure_leaves.contains_key(&container.0),
        "precondition: the measure leaf exists"
    );

    // Between frames: insert a second absolute child before the first.
    let abs2 = doc.create_element("div");
    doc.set_attribute(
        abs2,
        "style",
        "position: absolute; left: 40px; top: 4px; width: 20px; height: 6px",
    );
    doc.insert_before(container, abs2, abs1);
    doc.resolve_layout(VW, VH);

    let (x, y, w, h) = layout_of(&doc, abs2);
    assert!(
        (x - 40.0).abs() < 0.5
            && (y - 4.0).abs() < 0.5
            && (w - 20.0).abs() < 0.5
            && (h - 6.0).abs() < 0.5,
        "the late-inserted absolute child must be laid out at its insets — \
         got ({x}, {y}, {w}, {h}); all-zero means it was never attached"
    );
    let (x1, y1, ..) = layout_of(&doc, abs1);
    assert!(
        (x1 - 200.0).abs() < 0.5 && (y1 - 0.0).abs() < 0.5,
        "the original absolute child keeps its own insets, got ({x1}, {y1})"
    );
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
}

/// Block virtualization rides the same leaf: a collapsed `text + absolute`
/// block returns its estimate. The `estimated_height` early return lives
/// *inside* the measure closure, which only runs at all because the carrier is
/// a leaf.
///
/// Kills: a leaf decision that mishandles a collapsed root (wrong context
/// payload panics or measures the wrong node; a non-leaf carrier trips the
/// in-setup validator in debug builds).
#[test]
fn a_virtualized_text_plus_absolute_root_collapses_to_its_estimate() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; width: 150px; height: 300px; overflow-y: auto",
    );
    let mut blocks = Vec::new();
    for i in 0..3 {
        let p = child_of(
            &mut doc,
            container,
            "p",
            "font-size: 16px; line-height: 20px",
        );
        text_in(
            &mut doc,
            p,
            &format!("Block {i} with plenty of words here so it wraps across several lines"),
        );
        child_of(
            &mut doc,
            p,
            "div",
            "position: absolute; left: 3px; top: 2px; width: 8px; height: 6px",
        );
        blocks.push(p);
    }
    doc.resolve_layout(VW, VH);

    let probe = blocks[2];
    let full_h = height_of(&doc, probe);
    assert!(
        full_h > 30.0,
        "precondition: a multi-line block before collapse, got {full_h}"
    );

    // Mimic CeVirtualWindow exactly: the estimate plus its dirty markers.
    doc.tree.nodes[probe.0].estimated_height = Some(24.0);
    doc.tree.style_dirty_nodes.push(probe.0);
    doc.tree.layout_dirty = true;
    doc.tree.ifc_dirty = true;
    doc.tree.styles_dirty = true;
    doc.resolve_layout(VW, VH);

    let collapsed_h = height_of(&doc, probe);
    assert!(
        (collapsed_h - 24.0).abs() < 1.0,
        "the collapsed block must take its 24px estimate, got {collapsed_h} \
         (full height was {full_h})"
    );
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
}

/// The invariant predicate covers the measure leaves — PR 1's coverage note
/// required exactly this when the context moved onto a carrier with no DOM
/// identity. A doctored child under the leaf, or a leaf stripped of its
/// context, is reported under the root's DOM id.
///
/// Kills: leaving the leaves out of `ifc_leaf_invariant_violations`' walk
/// (both halves answer empty), or dropping either conjunct of the leaf check.
#[test]
fn the_predicate_reports_a_doctored_measure_child() {
    // A child attached under the leaf makes the measure unreachable.
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, container, "hello world");
    child_of(
        &mut doc,
        container,
        "div",
        "position: absolute; left: 5px; top: 5px; width: 10px; height: 10px",
    );
    doc.resolve_layout(VW, VH);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());

    let &leaf = doc.tree.ifc_measure_leaves.get(&container.0).unwrap();
    let extra = doc.tree.taffy.new_leaf(Default::default()).unwrap();
    doc.tree.taffy.add_child(leaf, extra).unwrap();
    assert_eq!(
        doc.ifc_leaf_invariant_violations(),
        vec![container.0],
        "a non-leaf measure child is a violation, reported under its root"
    );

    // A leaf whose context was stripped can never fire the measure either.
    let mut doc2 = RinchDocument::new();
    let body2 = doc2.body();
    let container2 = child_of(
        &mut doc2,
        body2,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc2, container2, "hello world");
    child_of(
        &mut doc2,
        container2,
        "div",
        "position: absolute; left: 5px; top: 5px; width: 10px; height: 10px",
    );
    doc2.resolve_layout(VW, VH);
    let &leaf2 = doc2.tree.ifc_measure_leaves.get(&container2.0).unwrap();
    if let Some(ctx) = doc2.tree.taffy.get_node_context_mut(leaf2) {
        *ctx = NodeContext::Element;
    }
    assert_eq!(
        doc2.ifc_leaf_invariant_violations(),
        vec![container2.0],
        "a context-stripped measure child is a violation too"
    );
}

/// The move matters most on the *second* pass: a container that was all-inline
/// carries `InlineRoot` on its own Taffy node; when an absolute child arrives
/// and the measure leaf takes over, that context must be cleared off the now
/// non-leaf root. A fresh single-pass fixture cannot see this — a new root's
/// context starts as `None`, the natural identity value where a copy-mutant
/// hides.
///
/// Kills: skipping the context clear in the leaf branch — the root keeps
/// `InlineRoot` with children, and the in-setup validator panics in debug
/// builds on the second resolve (the context assertion fails without it).
#[test]
fn a_root_that_gains_an_absolute_child_hands_its_context_to_the_leaf() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, container, "hello world");
    doc.resolve_layout(VW, VH);
    let container_taffy = doc.tree.get(container.0).unwrap().taffy_id.unwrap();
    assert!(
        matches!(
            doc.tree.taffy.get_node_context(container_taffy),
            Some(NodeContext::InlineRoot(_))
        ),
        "precondition: the all-inline root carries the context itself"
    );

    child_of(
        &mut doc,
        container,
        "div",
        "position: absolute; left: 5px; top: 5px; width: 10px; height: 10px",
    );
    doc.resolve_layout(VW, VH);

    assert!(
        doc.tree.ifc_measure_leaves.contains_key(&container.0),
        "the measure leaf takes over"
    );
    assert!(
        !matches!(
            doc.tree.taffy.get_node_context(container_taffy),
            Some(NodeContext::InlineRoot(_))
        ),
        "the old context must be cleared off the now-non-leaf root"
    );
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    let h = height_of(&doc, container);
    assert!((h - 20.0).abs() < 2.0, "still one measured line, got {h}");
}

/// The percentage-inline-block second pass reaches the measure leaf too:
/// `resolve_percentage_inline_blocks` re-measures a `width: N%` inline-block
/// against its containing block's real width, and when the size changed it
/// marks the enclosing IFC root dirty so the second Taffy compute re-breaks
/// the lines — with the measure on a leaf, the root mark alone cannot reach
/// it (#120 meets #466).
///
/// Kills: omitting `mark_ifc_measure_dirty` from
/// `resolve_percentage_inline_blocks` — the second compute serves the leaf's
/// cached measure, taken while the inline-block was collapsed to min-content
/// (tall and narrow), and the container keeps that stale tall height.
#[test]
fn a_percentage_inline_block_recorrection_reaches_the_measure_child() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; width: 200px",
    );
    text_in(&mut doc, container, "x ");
    let ib = child_of(
        &mut doc,
        container,
        "span",
        "display: inline-block; width: 50%",
    );
    text_in(&mut doc, ib, "mm mm mm mm mm");
    child_of(
        &mut doc,
        container,
        "div",
        "position: absolute; left: 5px; top: 5px; width: 10px; height: 10px",
    );
    doc.resolve_layout(VW, VH);

    assert!(
        doc.tree.ifc_measure_leaves.contains_key(&container.0),
        "precondition: the measure leaf exists"
    );
    let (.., ib_w, ib_h) = layout_of(&doc, ib);
    assert!(
        (ib_w - 100.0).abs() < 2.0,
        "50% of the 200px containing block — proves the second pass armed and \
         fired at all; got {ib_w}"
    );
    let h = height_of(&doc, container);
    assert!(
        (h - ib_h).abs() < 6.0 && h < ib_h + 30.0,
        "the container's line re-breaks around the corrected inline-block \
         (~{ib_h}px tall), not the stale min-content-collapsed one; got {h}"
    );
}

/// A `display: contents` child that also declares `position: absolute` is
/// **not** out of flow for classification: Stylo does not blockify
/// `display: contents` (`equivalent_block_display` maps `DisplayOutside::None`
/// to itself), so the wrapper keeps `display: Contents` while
/// `is_out_of_flow()` answers `true` — and a boxless element has no box to
/// take out of flow (browsers ignore `position` on it). `has_block` must
/// classify it by **display first**: it counts as block content exactly as on
/// main, keeping the anonymous-box path, so the #466 flip never newly roots
/// this shape. (Adapted from the #497 review's probe A, which falsified the
/// "no newly-rooted shape reaches the fall-through" claim: excluding the
/// wrapper as out-of-flow rooted the container, the decision loop's
/// display-first Contents arm said opaque, and the fall-through stamped
/// `InlineRoot` on a root with attached children — a debug_assert panic on
/// markup main renders.)
///
/// Kills: applying the out-of-flow exclusion to `Contents` children — this
/// resolve panics on the in-setup validator in debug builds.
#[test]
fn an_absolutely_positioned_contents_wrapper_still_counts_as_block_content() {
    use rinch_dom::computed_style::DisplayValue;

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, container, "aaa ");
    let wrapper = child_of(
        &mut doc,
        container,
        "span",
        "display: contents; position: absolute",
    );
    child_of(&mut doc, wrapper, "div", "width: 30px; height: 30px");
    text_in(&mut doc, container, " bbb");
    doc.resolve_layout(VW, VH);

    // The wrapper really is the pathological combination this pins.
    assert_eq!(
        doc.tree.get(wrapper.0).unwrap().computed_style.display,
        DisplayValue::Contents,
        "precondition: Stylo left the wrapper un-blockified"
    );
    assert!(
        doc.tree.get(wrapper.0).unwrap().is_out_of_flow(),
        "precondition: the position predicate alone calls it out of flow"
    );

    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    assert_eq!(
        anon_box_count(&doc),
        1,
        "the wrapper counts as block content (display-first), so the text run \
         is wrapped exactly as on main — the container is never newly rooted"
    );
    assert!(
        !doc.tree.ifc_measure_leaves.contains_key(&container.0),
        "no measure leaf: this container is not an IFC root"
    );
}

// ── #289: an out-of-flow box behind a `display:contents` wrapper ────────────
//
// `scan_contents_children` classified any non-inline child as "a real
// block-level box", out-of-flow ones included, so a `display:contents` wrapper
// whose only non-inline content is absolutely positioned was judged opaque and
// pushed out of the IFC: its texts stayed attached to the container as bare
// Taffy text leaves and stacked as blocks, one per line. #289 could not ship
// the obvious one-line skip on its own — marking the wrapper transparent
// leaves the reparented absolute grandchild attached to the container's Taffy
// node, and before #466's measure-child that made the container a non-leaf
// whose inline measure could never fire (h = 0). With PR 2 merged the skip is
// safe: the measure leaf supplies the height, and the decision loop collects
// the wrapper's flattened out-of-flow boxes so canonicalization keeps them
// laid out by Taffy.
//
// Every test here was run against the pre-fix tree (origin/main with PR 2
// merged and the scan un-skipped) or the named mutant, and failed.

/// #289's own markup: the text behind the wrapper flows in the container's IFC
/// at line height, **and** the absolute grandchild's ink is present — both
/// asserted, because the historical failure mode was fixing one by losing the
/// other (the naive skip hid the absolute box; the opaque wrapper stacked the
/// text as blocks).
///
/// Kills: reverting the scan's out-of-flow skip (= main: the two text runs
/// stack as bare Taffy leaves, one per line — height 106, narrow ink — instead
/// of sharing one 48px line); and stamping `ifc_root` on the absolute
/// grandchild (paint skips it — no red ink).
#[test]
fn text_behind_a_contents_wrapper_flows_and_its_absolute_grandchild_paints() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 40px; line-height: 48px; width: 700px; \
         padding: 7px 0 3px 0; color: rgb(0, 128, 0); background: rgb(0, 0, 255)",
    );
    // The marker comment from #289's fixture — `show_dom` emits one beside
    // every branch root, so the wrapper idiom always has it in practice.
    let marker = doc.create_comment("m");
    doc.append_child(container, marker);
    let wrapper = child_of(&mut doc, container, "div", "display: contents");
    text_in(&mut doc, wrapper, "AAAA");
    child_of(
        &mut doc,
        wrapper,
        "div",
        "position: absolute; left: 32px; top: 9px; width: 50px; height: 10px; \
         background: rgb(255, 0, 0)",
    );
    text_in(&mut doc, wrapper, "BBBB");
    doc.resolve_layout(VW, VH);

    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    assert_eq!(
        anon_box_count(&doc),
        0,
        "no direct inline children, no anonymous box"
    );
    let h = height_of(&doc, container);
    assert!(
        (h - 58.0).abs() < 2.0,
        "one 48px line + 7px/3px padding = 58 expected — the wrapped text \
         flows in the container's IFC, got {h}"
    );

    let px = rasterize(&mut doc);
    let (gx0, _, gx1, _) =
        color_bbox(&px, (0, 128, 0)).expect("the wrapped text is painted at all");
    assert!(
        gx1 - gx0 > 120,
        "AAAA and BBBB share one line box, so the inked span is wide; got \
         {}px — a narrow span means the runs stacked as separate blocks",
        gx1 - gx0
    );
    let (cx, cy, ..) = color_bbox(&px, (0, 0, 255)).expect("the container is painted");
    let red = color_bbox(&px, (255, 0, 0)).expect(
        "the absolute grandchild is painted at all — no red ink means marking \
         the wrapper transparent hid the box (#289's historical trap)",
    );
    assert_eq!(
        red,
        (cx + 32, cy + 9, cx + 32 + 50, cy + 9 + 10),
        "the box is inked exactly at its container-relative insets"
    );
}

/// The structural half: marking the wrapper transparent leaves the reparented
/// absolute grandchild attached, so the decision loop must see it *through*
/// the wrapper and arm the measure leaf — the scan's skip alone stamps
/// `InlineRoot` on a container that still has a Taffy child.
///
/// Kills: skipping out-of-flow children in the scan **without** collecting
/// them through transparent wrappers in the decision loop (the in-setup
/// validator panics in debug builds; no leaf is minted); and, run pre-fix,
/// the whole change reverted (no leaf on main either).
#[test]
fn a_contents_wrapped_absolute_grandchild_gets_the_measure_leaf() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; width: 400px",
    );
    let wrapper = child_of(&mut doc, container, "div", "display: contents");
    let text = text_in(&mut doc, wrapper, "hello world");
    let abs = child_of(
        &mut doc,
        wrapper,
        "div",
        "position: absolute; left: 32px; top: 9px; width: 50px; height: 10px",
    );
    doc.resolve_layout(VW, VH);

    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    assert_eq!(
        doc.tree.get(wrapper.0).unwrap().ifc_root,
        Some(container.0),
        "the wrapper is transparent to the container's IFC"
    );
    assert_eq!(
        doc.tree.get(text.0).unwrap().ifc_root,
        Some(container.0),
        "the wrapped text is the container's inline content"
    );
    assert_eq!(
        doc.tree.get(abs.0).unwrap().ifc_root,
        None,
        "an out-of-flow grandchild is nobody's inline content"
    );

    let container_taffy = doc.tree.get(container.0).unwrap().taffy_id.unwrap();
    let &leaf = doc
        .tree
        .ifc_measure_leaves
        .get(&container.0)
        .expect("a root whose attached children are all out-of-flow gets a measure leaf");
    assert!(
        matches!(
            doc.tree.taffy.get_node_context(leaf),
            Some(NodeContext::InlineRoot(id)) if *id == container.0
        ),
        "the leaf carries InlineRoot naming the container"
    );
    let children = doc.tree.taffy.children(container_taffy).unwrap();
    assert_eq!(
        children.first().copied(),
        Some(leaf),
        "the leaf sits at index 0, before the out-of-flow grandchild"
    );
    let abs_taffy = doc.tree.get(abs.0).unwrap().taffy_id.unwrap();
    assert!(
        children.contains(&abs_taffy),
        "the absolute grandchild stays attached — Taffy does its layout"
    );
    assert!(
        !matches!(
            doc.tree.taffy.get_node_context(container_taffy),
            Some(NodeContext::InlineRoot(_))
        ),
        "the container (a non-leaf) must not keep the context"
    );
}

/// Classification stays **display-first** in the scan: a `display: contents;
/// position: absolute` wrapper is boxless (Stylo does not blockify contents),
/// so it has no box to take out of flow — the scan recurses into it like any
/// other wrapper rather than skipping it as out-of-flow. PR 2's review found
/// exactly this precedence disagreement live in `has_block`; this pins the
/// scan against the same mistake.
///
/// Kills: placing the out-of-flow skip ahead of the `Contents` arm — the
/// wrapper is then skipped, the container never becomes a root, and the two
/// texts stack as bare Taffy leaves (height 96, `ifc_root == None`).
#[test]
fn an_absolutely_positioned_contents_wrapper_recurses_display_first_in_the_scan() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 40px; line-height: 48px; width: 700px",
    );
    let wrapper = child_of(
        &mut doc,
        container,
        "span",
        "display: contents; position: absolute",
    );
    let a = text_in(&mut doc, wrapper, "AAAA");
    text_in(&mut doc, wrapper, "BBBB");
    doc.resolve_layout(VW, VH);

    assert!(
        doc.tree.get(wrapper.0).unwrap().is_out_of_flow(),
        "precondition: the position predicate alone calls the wrapper out of flow"
    );
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    assert_eq!(
        doc.tree.get(a.0).unwrap().ifc_root,
        Some(container.0),
        "the wrapped text flows in the container's IFC — the boxless wrapper \
         was recursed into, not skipped as out-of-flow"
    );
    let h = height_of(&doc, container);
    assert!(
        (h - 48.0).abs() < 1.0,
        "both runs share one 48px line, got {h} — 96 means the wrapper was \
         skipped and the texts stacked as blocks"
    );
}

/// The skip narrows the block arm, it does not delete it: a wrapper mixing
/// text with an **in-flow** block stays opaque, exactly as on main — mixed
/// content behind `display:contents` is still the anonymous-box problem, not
/// this IFC's.
///
/// Kills: a scan that skips every non-inline child — the wrapper turns
/// transparent, the decision loop's collector (rightly) refuses the in-flow
/// block, and canonicalization detaches it: the block loses its box and its
/// ink, and the container shrinks to one line.
#[test]
fn a_wrapper_mixing_text_and_an_in_flow_block_stays_opaque() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 40px; line-height: 48px; width: 700px",
    );
    let wrapper = child_of(&mut doc, container, "div", "display: contents");
    text_in(&mut doc, wrapper, "hello");
    let block = child_of(
        &mut doc,
        wrapper,
        "div",
        "width: 60px; height: 30px; background: rgb(255, 0, 255)",
    );
    doc.resolve_layout(VW, VH);

    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    assert_eq!(
        doc.tree.get(wrapper.0).unwrap().ifc_root,
        None,
        "an in-flow block keeps the wrapper opaque"
    );
    let (_, _, bw, bh) = layout_of(&doc, block);
    assert!(
        (bw - 60.0).abs() < 0.5 && (bh - 30.0).abs() < 0.5,
        "the in-flow block keeps its box, got {bw}x{bh}"
    );
    let h = height_of(&doc, container);
    assert!(
        h >= 78.0 - 1.0,
        "one 48px line plus the 30px block, got {h} — a smaller height means \
         the block was detached from layout"
    );
    let px = rasterize(&mut doc);
    assert!(
        color_bbox(&px, (255, 0, 255)).is_some(),
        "the in-flow block is still painted"
    );
}

/// The collector flattens through *nested* transparent wrappers, exactly like
/// the scan and the marking pass do: absolute boxes one **and** two
/// `display:contents` levels down are both handed to Taffy by the
/// canonicalization. Two depths on purpose — with only the deep box, an
/// empty collection skips the leaf branch entirely and the validator fires
/// first; with a depth-1 box collected, a partial recursion arms the leaf
/// branch and `set_children` is what drops the deep box, so the ink is the
/// only witness.
///
/// Kills: a collector that only looks at the wrapper's direct children — the
/// depth-2 absolute box is dropped by `set_children`, never laid out, and
/// paints no ink at its insets (while the validator stays green).
#[test]
fn a_nested_transparent_wrapper_still_hands_its_absolute_box_to_taffy() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; width: 400px; \
         background: rgb(0, 0, 255)",
    );
    let outer = child_of(&mut doc, container, "div", "display: contents");
    text_in(&mut doc, outer, "hello");
    let shallow = child_of(
        &mut doc,
        outer,
        "div",
        "position: absolute; left: 200px; top: 3px; width: 40px; height: 8px; \
         background: rgb(255, 128, 0)",
    );
    let inner = child_of(&mut doc, outer, "div", "display: contents");
    let abs = child_of(
        &mut doc,
        inner,
        "div",
        "position: absolute; left: 32px; top: 9px; width: 50px; height: 10px; \
         background: rgb(255, 0, 0)",
    );
    text_in(&mut doc, inner, "world");
    doc.resolve_layout(VW, VH);

    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    assert!(
        doc.tree.ifc_measure_leaves.contains_key(&container.0),
        "the container gets its measure leaf"
    );
    let (sx, sy, ..) = layout_of(&doc, shallow);
    assert!(
        (sx - 200.0).abs() < 0.5 && (sy - 3.0).abs() < 0.5,
        "the depth-1 absolute box is laid out at its insets, got ({sx}, {sy})"
    );
    let (x, y, w, h) = layout_of(&doc, abs);
    assert!(
        (x - 32.0).abs() < 0.5
            && (y - 9.0).abs() < 0.5
            && (w - 50.0).abs() < 0.5
            && (h - 10.0).abs() < 0.5,
        "the depth-2 absolute box is laid out at its insets, got ({x}, {y}, {w}, {h})"
    );
    let px = rasterize(&mut doc);
    let (cx, cy, ..) = color_bbox(&px, (0, 0, 255)).expect("the container is painted");
    let orange = color_bbox(&px, (255, 128, 0)).expect("the depth-1 absolute box is painted");
    assert_eq!(
        orange,
        (cx + 200, cy + 3, cx + 200 + 40, cy + 3 + 8),
        "the depth-1 box is inked at its container-relative insets"
    );
    let red = color_bbox(&px, (255, 0, 0)).expect(
        "the depth-2 absolute box is painted — dropping it from the \
                 canonicalization loses its only route to layout",
    );
    assert_eq!(
        red,
        (cx + 32, cy + 9, cx + 32 + 50, cy + 9 + 10),
        "inked exactly at its container-relative insets"
    );
}
