//! The shared inline-flow classifier, and mark ≡ walk agreement (#366).
//!
//! `mark_inline_descendants` and `walk_inline_children` are supposed to cover
//! the same set of boxes — "mark exactly what the walk flows" is the invariant
//! the IFC pass rests on. They diverged on one case (#366): reaching an
//! **in-flow block-level** child, the walker `break`s while the marking pass
//! `continue`d, stamping `ifc_root` on boxes no IFC ever lays out or draws —
//! and every consumer of that field (paint's `already_drawn_inline` skip, IFC
//! invalidation routing, the inline-block special cases) then believed an IFC
//! owned them. They also diverged on `display: none`, from the other side:
//! the mark detaches it and continues, while the walk `break`s — so inline
//! content after it was marked but never flowed.
//!
//! Both sides now consume one classifier, [`Node::inline_flow_role`], whose
//! contract is **display before position, always**. This sequence paid twice
//! for hand-rolled copies of that rule drifting (#466), so the tests here pin
//! the classifier itself, the mark ≡ walk agreement over each role, and — per
//! site — that a precedence flip inside the classifier is caught at that
//! site's observable behaviour, not only by a debug assert.
//!
//! Two oracles, following `ifc_out_of_flow_tests.rs`: the `ifc_root` marks
//! against the root's built `InlineLayout` (what the walk actually flowed),
//! and rasterised ink where the interesting claim is "this text is drawn at
//! all" — a `debug_assert` witness stops working in release.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::{InlineFlowRole, RinchDocument};

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

fn ifc_root_of(doc: &RinchDocument, node: NodeId) -> Option<usize> {
    doc.tree.get(node.0).unwrap().ifc_root
}

fn role_of(doc: &RinchDocument, node: NodeId) -> InlineFlowRole {
    doc.tree.get(node.0).unwrap().inline_flow_role()
}

/// Whether `root`'s built `InlineLayout` actually flowed `node` — as a text
/// range (text nodes, `<br>`) or a positioned inline child (inline elements,
/// inline-blocks). This is the walk's output, so it is the ground truth for
/// "the IFC lays this box out".
fn flowed_by(doc: &RinchDocument, root: usize, node: usize) -> bool {
    let Some(layout) = doc.tree.get(root).and_then(|n| n.text_layout.as_ref()) else {
        return false;
    };
    layout.text_ranges.iter().any(|r| r.node_id == node)
        || layout.child_positions.iter().any(|(id, _)| *id == node)
}

/// The invariant #366 is about, checked over the whole tree: a box carries
/// `ifc_root = Some(r)` **iff** `r`'s IFC lays it out.
///
/// Exemptions, each a node the walk deliberately flows nothing for while the
/// mark still stamps it: comments (no box; the mark keeps IFC discovery
/// finding comment-only containers), transparent `display:contents` wrappers
/// (no box; only their descendants flow), and empty text nodes (the walk
/// pushes no range for them).
fn assert_marks_match_flow(doc: &RinchDocument) {
    use rinch_dom::computed_style::DisplayValue;
    for (id, node) in &doc.tree.nodes {
        let Some(r) = node.ifc_root else { continue };
        if node.is_comment()
            || node.computed_style.display == DisplayValue::Contents
            || node.text_content().is_some_and(str::is_empty)
        {
            continue;
        }
        assert!(
            flowed_by(doc, r, id),
            "node {id} carries ifc_root = Some({r}) but that IFC never lays it \
             out — every consumer of the mark (paint skipping, invalidation \
             routing, the inline-block special cases) now believes an IFC owns \
             a box no IFC draws (#366)"
        );
    }
    for (id, node) in &doc.tree.nodes {
        let Some(layout) = node.text_layout.as_ref() else {
            continue;
        };
        for &flowed in layout
            .text_ranges
            .iter()
            .map(|r| &r.node_id)
            .chain(layout.child_positions.iter().map(|(cid, _)| cid))
        {
            assert_eq!(
                doc.tree.get(flowed).and_then(|n| n.ifc_root),
                Some(id),
                "node {flowed} is flowed by IFC {id} but does not carry its \
                 mark — the paint tree-walk would draw it a second time"
            );
        }
    }
}

// ── the classifier itself ───────────────────────────────────────────────────

/// [`Node::inline_flow_role`]'s precedence, asserted at the source: display
/// before position, always. The two mixed rows are the load-bearing ones — a
/// `display: none` or `display: contents` element that also declares
/// `position: absolute` classifies by its display, because a boxless element
/// has no box to take out of flow (Stylo does not blockify contents).
///
/// Kills: reordering the checks in `inline_flow_role` so `is_out_of_flow`
/// speaks before `display` — both mixed rows flip to `OutOfFlow` — and every
/// coarser mutation (dropping an arm, inverting the inline test). The
/// per-site tests below prove each consumer routes through this rule; this
/// one names the rule.
#[test]
fn the_classifier_is_display_first() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(&mut doc, body, "div", "");
    let text = text_in(&mut doc, container, "text");
    let comment = doc.create_comment("marker");
    doc.append_child(container, comment);
    let span = child_of(&mut doc, container, "span", "");
    let inline_block = child_of(&mut doc, container, "button", "");
    let block = child_of(&mut doc, container, "div", "");
    let abs = child_of(&mut doc, container, "div", "position: absolute");
    let fixed = child_of(&mut doc, container, "div", "position: fixed");
    let none = child_of(&mut doc, container, "div", "display: none");
    let contents = child_of(&mut doc, container, "span", "display: contents");
    let none_abs = child_of(
        &mut doc,
        container,
        "div",
        "display: none; position: absolute",
    );
    let contents_abs = child_of(
        &mut doc,
        container,
        "span",
        "display: contents; position: absolute",
    );
    doc.resolve_layout(VW, VH);

    assert_eq!(role_of(&doc, text), InlineFlowRole::Inline);
    assert_eq!(role_of(&doc, comment), InlineFlowRole::Comment);
    assert_eq!(role_of(&doc, span), InlineFlowRole::Inline);
    assert_eq!(role_of(&doc, inline_block), InlineFlowRole::Inline);
    assert_eq!(role_of(&doc, block), InlineFlowRole::InFlowBlock);
    assert_eq!(role_of(&doc, abs), InlineFlowRole::OutOfFlow);
    assert_eq!(role_of(&doc, fixed), InlineFlowRole::OutOfFlow);
    assert_eq!(role_of(&doc, none), InlineFlowRole::NoBox);
    assert_eq!(role_of(&doc, contents), InlineFlowRole::Contents);
    assert_eq!(
        role_of(&doc, none_abs),
        InlineFlowRole::NoBox,
        "display before position: a hidden element has no box to take out of flow"
    );
    assert_eq!(
        role_of(&doc, contents_abs),
        InlineFlowRole::Contents,
        "display before position: a contents wrapper is boxless — browsers \
         ignore `position` on it, and Stylo leaves it un-blockified"
    );
}

// ── #366: mark breaks at an in-flow block, like walk ────────────────────────

/// The issue's own markup: `<div><a>text<div>block</div>tail</a></div>`.
/// Parley stops building the line at the inner block, so `tail` never reaches
/// a line — and the marking pass used to keep going and stamp it anyway.
/// `create_anonymous_block_boxes` skips `Inline` containers, so mixed content
/// inside `<a>` is never normalized and the divergence was reachable.
///
/// Kills: reverting `mark_inline_descendants`' `InFlowBlock` arm to fall
/// through (`continue`) — `tail` comes back marked, failing both the direct
/// assertion and the whole-tree oracle.
#[test]
fn mark_stops_at_an_in_flow_block_exactly_where_walk_stops() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    let link = child_of(&mut doc, container, "a", "");
    let text = text_in(&mut doc, link, "text");
    let block = child_of(&mut doc, link, "div", "width: 30px; height: 30px");
    let tail = text_in(&mut doc, link, "tail");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        ifc_root_of(&doc, link),
        Some(container.0),
        "precondition: the all-inline container establishes the IFC and the \
         link joins it"
    );
    assert_eq!(
        ifc_root_of(&doc, text),
        Some(container.0),
        "text before the block is this IFC's content"
    );
    assert!(
        flowed_by(&doc, container.0, text.0),
        "…and the walk flows it"
    );
    assert_eq!(
        ifc_root_of(&doc, block),
        None,
        "an in-flow block is never IFC content"
    );
    assert_eq!(
        ifc_root_of(&doc, tail),
        None,
        "the walk breaks at the block, so `tail` never reaches a line — \
         marking it would hand every consumer of `ifc_root` a box no IFC \
         draws (#366)"
    );
    assert!(
        !flowed_by(&doc, container.0, tail.0),
        "precondition for the assertion above: the walk really does not flow \
         `tail`"
    );
    assert_marks_match_flow(&doc);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
}

/// All three cases of the three-way rule in one markup, inside an inline
/// element: `<a>t1<abs/>t2<div/>t3</a>`. Inline flows, out-of-flow is walked
/// past (both sides), an in-flow block stops both sides.
///
/// Kills: `mark`'s `OutOfFlow` arm turning into a `break` (t2 loses its
/// mark); `walk`'s out-of-flow skip turning into a `break` (t2 is marked but
/// not flowed — the whole-tree oracle); `mark`'s `InFlowBlock` arm turning
/// back into a fall-through (t3 comes back marked).
#[test]
fn mark_and_walk_agree_on_all_three_cases_of_the_rule() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; width: 400px",
    );
    let link = child_of(&mut doc, container, "a", "");
    let t1 = text_in(&mut doc, link, "one ");
    let abs = child_of(
        &mut doc,
        link,
        "div",
        "position: absolute; top: 0; right: 0; width: 10px; height: 10px",
    );
    let t2 = text_in(&mut doc, link, "two");
    let block = child_of(&mut doc, link, "div", "width: 30px; height: 30px");
    let t3 = text_in(&mut doc, link, "three");
    doc.resolve_layout(VW, VH);

    assert_eq!(ifc_root_of(&doc, t1), Some(container.0));
    assert!(flowed_by(&doc, container.0, t1.0));
    assert_eq!(
        ifc_root_of(&doc, abs),
        None,
        "an out-of-flow box is never IFC content (#289) — unmarked, it paints \
         from its stacking root"
    );
    assert_eq!(
        ifc_root_of(&doc, t2),
        Some(container.0),
        "inline content carries on across an out-of-flow sibling (CSS 2.1 \
         §9.4.2) — the mark must not stop at it"
    );
    assert!(
        flowed_by(&doc, container.0, t2.0),
        "…and the walk flows it, same rule, same classifier"
    );
    assert_eq!(ifc_root_of(&doc, block), None);
    assert_eq!(
        ifc_root_of(&doc, t3),
        None,
        "after the in-flow block, both sides have stopped (#366)"
    );
    assert_marks_match_flow(&doc);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
}

// ── display:none — the walk side, and the ink that proves it ────────────────

/// `<a>AAAA<span style="display:none">XX</span>BBBB</a>`: a hidden inline
/// sibling neither hides what follows it nor shows itself. The walk used to
/// `break` at the `display:none` child — while the mark detached it and
/// *continued* — so `BBBB` was marked but never flowed: neither Parley nor
/// Taffy laid it out, and it silently vanished. Same divergence class as
/// #366, opposite side.
///
/// Kills: `walk_inline_children` dropping `NoBox` from its skip arm — the
/// inked span collapses to `AAAA`'s width and the whole-tree oracle fails.
/// The `XX` assertion kills marking or flowing the hidden subtree.
#[test]
fn text_after_a_hidden_inline_sibling_is_still_painted() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 40px; line-height: 48px; width: 700px; color: rgb(0, 0, 255)",
    );
    let link = child_of(&mut doc, container, "a", "");
    text_in(&mut doc, link, "AAAA");
    let hidden = child_of(&mut doc, link, "span", "display: none");
    let hidden_text = text_in(&mut doc, hidden, "XX");
    let tail = text_in(&mut doc, link, "BBBB");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        ifc_root_of(&doc, tail),
        Some(container.0),
        "content after the hidden sibling still belongs to the IFC"
    );
    assert!(flowed_by(&doc, container.0, tail.0));
    assert_eq!(
        ifc_root_of(&doc, hidden),
        None,
        "the hidden span is detached, not marked — it is not this IFC's content"
    );
    // Note: `hidden_text` may carry a mark from the hidden span's *own* IFC —
    // the decision loop filters Inline/InlineBlock/Flex/Contents root
    // candidates but not `display: none`, a pre-existing (and unpainted)
    // oddity this PR leaves alone. What matters here is that the *container's*
    // IFC claims nothing under the hidden subtree.
    assert_ne!(ifc_root_of(&doc, hidden_text), Some(container.0));
    assert!(!flowed_by(&doc, container.0, hidden.0));
    assert!(!flowed_by(&doc, container.0, hidden_text.0));
    assert_marks_match_flow(&doc);

    let px = rasterize(&mut doc);
    let bbox = color_bbox(&px, (0, 0, 255)).expect("the link's text is painted at all");
    let width = bbox.2 - bbox.0;
    assert!(
        width > 120,
        "AAAA and BBBB are both drawn, so the inked span is wide; got \
         {width}px — a narrow span means the walk still breaks at \
         display:none"
    );
}

/// At the root level, browsers render `a<none/>b` as **one line**: a
/// `display: none` child generates no box, so it neither forces anonymous
/// block boxes nor splits the inline run. `has_block` used to count it as
/// block content and the run grouping ended the run at it — two anonymous
/// boxes, two lines, for markup Chrome renders on one.
///
/// Kills: `has_block` counting `NoBox` (anonymous boxes return — the count
/// assertion), the run grouping ending the run at `NoBox` (two lines — the
/// ink assertion), and `walk` breaking at `NoBox` now that the container is
/// the root (the second text vanishes — ink again).
#[test]
fn a_hidden_block_sibling_neither_splits_the_line_nor_mints_a_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 40px; line-height: 48px; width: 700px; color: rgb(200, 0, 40)",
    );
    text_in(&mut doc, container, "AAAA");
    let hidden = child_of(
        &mut doc,
        container,
        "div",
        "display: none; width: 50px; height: 50px",
    );
    text_in(&mut doc, container, "BBBB");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        doc.tree.anonymous_block_boxes.len(),
        0,
        "a display:none child is no box at all — not block content, no \
         anonymous boxes (it minted two on main)"
    );
    assert_eq!(ifc_root_of(&doc, hidden), None);
    let h = doc.tree.get(container.0).unwrap().layout.height;
    assert!(
        (h - 48.0).abs() < 2.0,
        "both runs share one line box, so the container is one line tall \
         (browsers agree); got {h} — ~96 means the hidden child still splits \
         the run"
    );
    assert_marks_match_flow(&doc);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());

    let px = rasterize(&mut doc);
    let bbox = color_bbox(&px, (200, 0, 40)).expect("the text is painted at all");
    let (width, height) = (bbox.2 - bbox.0, bbox.3 - bbox.1);
    assert!(
        width > 120,
        "AAAA and BBBB both drawn on the line; got {width}px wide"
    );
    assert!(
        height <= 50,
        "one line of ink, not two stacked lines; got {height}px tall"
    );
}

// ── precedence, observed per site ───────────────────────────────────────────

/// A `display: none; position: absolute` child is `NoBox` before it is
/// `OutOfFlow`. The decision loop must not collect it as an out-of-flow
/// child: the mark detached it (#487), so the root's own Taffy node is
/// childless and carries the measure context itself — no measure leaf.
///
/// Kills: flipping the classifier to position-first at the decision loop —
/// the hidden child classifies `OutOfFlow`, stays attached, and a measure
/// leaf is minted for a container that needs none (the leaf-map assertion);
/// in mark, the same flip leaves the child attached (the invariant check).
#[test]
fn a_hidden_absolute_child_is_no_box_before_it_is_out_of_flow() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "position: relative; font-size: 16px; line-height: 20px; width: 400px",
    );
    text_in(&mut doc, container, "hello");
    child_of(
        &mut doc,
        container,
        "div",
        "display: none; position: absolute; left: 5px; top: 5px; \
         width: 10px; height: 10px",
    );
    doc.resolve_layout(VW, VH);

    assert!(
        !doc.tree.ifc_measure_leaves.contains_key(&container.0),
        "the hidden child was detached as NoBox, not collected as OutOfFlow — \
         the root is childless and needs no measure leaf"
    );
    let h = doc.tree.get(container.0).unwrap().layout.height;
    assert!(
        (h - 20.0).abs() < 2.0,
        "the root's own measure fires (one line); got {h}"
    );
    assert_marks_match_flow(&doc);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
}

/// An *opaque* `display: contents` wrapper that also declares
/// `position: absolute` is `Contents` before it is `OutOfFlow`: it stands
/// for the block it wraps, so **both** the mark and the walk stop at it —
/// walking past it while the mark broke (or vice versa) is the double-draw /
/// silent-disappearance pair this classifier exists to kill.
///
/// Kills: flipping the classifier to position-first at mark or walk — the
/// wrapper classifies `OutOfFlow`, both sides skip it, and `tail` comes back
/// marked and flowed.
#[test]
fn an_opaque_absolute_contents_wrapper_stops_mark_and_walk_together() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    let link = child_of(&mut doc, container, "a", "");
    let text = text_in(&mut doc, link, "text");
    let wrapper = child_of(
        &mut doc,
        link,
        "span",
        "display: contents; position: absolute",
    );
    child_of(&mut doc, wrapper, "div", "width: 30px; height: 30px");
    let tail = text_in(&mut doc, link, "tail");
    doc.resolve_layout(VW, VH);

    assert_eq!(ifc_root_of(&doc, text), Some(container.0));
    assert!(flowed_by(&doc, container.0, text.0));
    assert_eq!(
        ifc_root_of(&doc, wrapper),
        None,
        "an opaque wrapper is not IFC content — marking it would make paint \
         skip the block it wraps"
    );
    assert_eq!(
        ifc_root_of(&doc, tail),
        None,
        "both sides stop at the wrapper, exactly as at the block it stands for"
    );
    assert!(!flowed_by(&doc, container.0, tail.0));
    assert_marks_match_flow(&doc);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
}

/// Guard rail: everything the classifier calls `Inline` still flows — text,
/// `<br>`, a styled inline span, and an inline-block — and each carries the
/// mark its flowing implies. This is the arm-mapping test for the walk's
/// dispatch *within* the `Inline` role.
///
/// Kills: mis-mapping any `Inline`-role kind in the walk's restructured
/// match (e.g. the inline-block arm no longer firing) — the box drops out of
/// `child_positions`/`text_ranges` and the oracle fails.
#[test]
fn every_inline_role_kind_still_flows_and_is_marked() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = child_of(
        &mut doc,
        body,
        "div",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    let t = text_in(&mut doc, container, "a");
    let br = child_of(&mut doc, container, "br", "");
    let span = child_of(&mut doc, container, "span", "font-weight: 700");
    let span_text = text_in(&mut doc, span, "b");
    let button = child_of(&mut doc, container, "button", "width: 40px; height: 10px");
    doc.resolve_layout(VW, VH);

    for (name, node) in [
        ("text", t),
        ("br", br),
        ("span", span),
        ("span text", span_text),
        ("inline-block", button),
    ] {
        assert_eq!(
            ifc_root_of(&doc, node),
            Some(container.0),
            "{name} is IFC content"
        );
        assert!(
            flowed_by(&doc, container.0, node.0),
            "{name} is flowed by the IFC"
        );
    }
    assert_marks_match_flow(&doc);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
}
