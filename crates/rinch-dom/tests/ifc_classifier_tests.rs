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

// ── #513: a block inside an inline splits it, so neither pass meets the block ─

/// #513's own markup: `<div><a>text<div>block</div>tail</a></div>`.
///
/// **This fixture used to assert the opposite, and the change is the fix.** The
/// `<a>` was inline content of the container's IFC, the marking pass and the walk
/// both stopped at the inner block, and `tail` reached no line at all — which is
/// what #366 made *consistent* (mark exactly what the walk flows) and what #513
/// then made *correct*. The `<a>` is now a **split inline**: it is flattened into
/// the container's units, an anonymous block box takes each inline run, and the
/// block is their sibling.
///
/// So the rule this section is named for is still the subject — the marks must
/// still describe exactly what some IFC lays out, which `assert_marks_match_flow`
/// checks over the whole tree — but the *stopping* is gone from this shape,
/// because there is no longer a pass that walks into the `<a>` at all.
///
/// **What that costs is recorded rather than hidden:** the
/// `InFlowBlock => break` arm in `mark_inline_descendants` and the `_ => break`
/// arm in `walk_inline_children` were reachable only through *attached* markup
/// like this, and no longer are. They were filed as #615 and believed unreachable
/// outright; that was wrong, and the two fixtures at the end of this file are the
/// witnesses — a **detached** subtree reaches both, because the flattening this
/// fixture relies on reads a field that is deliberately `false` outside the
/// document. This fixture is still deliberately **not** written to reach them: a
/// fixture that contrived a shape purely to keep an arm covered would be testing
/// the contrivance, and the detached route is not a contrivance.
#[test]
fn a_block_inside_an_inline_splits_it_rather_than_stopping_the_walk() {
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

    assert!(
        doc.tree.get(link.0).unwrap().is_split_inline(),
        "precondition: an inline holding an in-flow block is split (#513)"
    );
    assert_eq!(
        ifc_root_of(&doc, link),
        None,
        "a split inline is not inline *content* of anything — it generates no box \
         of its own, and its pieces are the container's"
    );
    assert!(
        doc.tree.get(container.0).unwrap().text_layout.is_none(),
        "…and the container is no longer an IFC root: every inline unit it has \
         now carries a `run_box`"
    );

    // Each side of the block is laid out by an anonymous block box, and they are
    // *different* boxes — which is what makes them different line boxes.
    let text_root = ifc_root_of(&doc, text).expect("`text` belongs to an IFC");
    let tail_root = ifc_root_of(&doc, tail).expect("`tail` belongs to an IFC");
    assert!(
        doc.tree.get(text_root).unwrap().is_anonymous_block_box
            && doc.tree.get(tail_root).unwrap().is_anonymous_block_box,
        "both sides are laid out by anonymous block boxes"
    );
    assert_ne!(
        text_root, tail_root,
        "the two sides are different boxes — one line each, with the block \
         between them. Same box would mean one line and a block drawn over it, \
         which is #490's shape."
    );
    assert!(
        flowed_by(&doc, text_root, text.0) && flowed_by(&doc, tail_root, tail.0),
        "…and each box really flows its own side"
    );
    assert_eq!(
        ifc_root_of(&doc, block),
        None,
        "an in-flow block is never IFC content"
    );

    assert_marks_match_flow(&doc);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    assert!(
        doc.taffy_tree_violations().is_empty(),
        "nothing is stranded any more: {:?}",
        doc.taffy_tree_violations()
    );
}

/// All three cases of the three-way rule in one markup, inside an inline
/// element: `<a>t1<abs/>t2<div/>t3</a>`.
///
/// **The rule survives #513 intact and is now asserted where it actually
/// lives** — in the run grouping rather than in the marking walk. An out-of-flow
/// box neither joins a run nor ends one (#406), so `t1` and `t2` land in the
/// **same** anonymous block box; the in-flow block ends that run, so `t3` lands
/// in a different one. Flip either half of the rule and this fixture moves:
/// ending the run at the absolute puts `t1` and `t2` in different boxes and on
/// different lines, and *not* ending it at the block puts `t3` in with them.
///
/// One measured side effect worth stating rather than claiming: the absolute is
/// a unit of the container, so it becomes a Taffy child of the container and
/// **is laid out**. That was not what closed #591 — an out-of-flow child of an
/// inline that holds *no* in-flow block does not split it and was still
/// stranded; the IFC root loop now collects that one into the root's own Taffy
/// list (`out_of_flow_in_inline_tests`), so the two routes agree on the edge.
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

    let r1 = ifc_root_of(&doc, t1).expect("t1 belongs to an IFC");
    let r2 = ifc_root_of(&doc, t2).expect("t2 belongs to an IFC");
    let r3 = ifc_root_of(&doc, t3).expect("t3 belongs to an IFC");
    assert_eq!(
        r1, r2,
        "inline content carries on across an out-of-flow sibling (CSS 2.1 \
         §9.4.2) — the same run, so the same anonymous box and the same line"
    );
    assert_ne!(
        r2, r3,
        "an in-flow block-level box ends the run (#366) — a different box, and \
         the block between them"
    );
    assert!(
        flowed_by(&doc, r1, t1.0) && flowed_by(&doc, r2, t2.0) && flowed_by(&doc, r3, t3.0),
        "every one of the three is flowed by the box that claims it"
    );
    assert_eq!(
        ifc_root_of(&doc, abs),
        None,
        "an out-of-flow box is never IFC content (#289) — unmarked, it paints \
         from its stacking root"
    );
    assert_eq!(ifc_root_of(&doc, block), None);

    // The side effect, measured here for the split shape; the non-split one is
    // `out_of_flow_in_inline_tests`' subject.
    assert_eq!(
        doc.tree
            .taffy
            .parent(doc.tree.get(abs.0).unwrap().taffy_id.unwrap())
            .and_then(|t| doc.tree.taffy_map.get(&t).copied()),
        Some(container.0),
        "the absolute is a unit of the container now, so the container's Taffy \
         node holds it and something lays it out"
    );

    assert_marks_match_flow(&doc);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    assert!(
        doc.taffy_tree_violations().is_empty(),
        "{:?}",
        doc.taffy_tree_violations()
    );
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

/// A `display: contents` wrapper that also declares `position: absolute` is
/// `Contents` **before** it is `OutOfFlow` — display before position, always —
/// and #513 moved where that precedence is observable without weakening it.
///
/// It used to be observable in the *stopping*: the wrapper stood for the block it
/// wrapped, so both the mark and the walk stopped at it, and flipping the
/// classifier to position-first made both skip it instead, bringing `tail` back
/// marked and flowed.
///
/// Now it is observable in the *split*. `Contents` means the unit collector
/// recurses into the wrapper, so the block it holds becomes a unit of the
/// container, the `<a>` therefore contributes an in-flow block-level box and is
/// split around it, and all three pieces render. Position-first would classify the
/// wrapper `OutOfFlow`, push the **wrapper** as a unit, never reach the block, and
/// leave the `<a>` unsplit — losing the block exactly as before #513, with the
/// container back to one line.
///
/// Kills: flipping the classifier to position-first (the `<a>` stops being split
/// and the container collapses to one line); and any change that stops the split
/// recursion crossing a `Contents` wrapper.
#[test]
fn an_absolute_contents_wrapper_is_boxless_first_so_its_block_still_splits() {
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

    assert_eq!(
        role_of(&doc, wrapper),
        InlineFlowRole::Contents,
        "display before position: the wrapper generates no box, so `position` \
         has no box to take out of flow"
    );
    assert!(
        doc.tree.get(link.0).unwrap().is_split_inline(),
        "…so the block behind it is reached, and the <a> is split around it. \
         Position-first would push the wrapper itself as a unit, never see the \
         block, and leave this `false` — which is how the block used to vanish."
    );
    assert_eq!(
        ifc_root_of(&doc, wrapper),
        None,
        "a boxless wrapper is not IFC content — marking it would make paint \
         skip the block it stands for"
    );

    let text_root = ifc_root_of(&doc, text).expect("`text` belongs to an IFC");
    let tail_root = ifc_root_of(&doc, tail).expect("`tail` belongs to an IFC");
    assert_ne!(
        text_root, tail_root,
        "both sides render, one line each, with the block between them"
    );
    assert!(flowed_by(&doc, text_root, text.0) && flowed_by(&doc, tail_root, tail.0));
    let h = doc.tree.get(container.0).unwrap().layout.height;
    assert_eq!(
        h, 70.0,
        "one 20px line, the 30px block, one 20px line. The block has no text \
         child, so it contributes its declared height and no line box. Off the \
         one-line fixed point (20), which is what position-first would give."
    );
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

// ── #615: the two `break` arms, reached through a detached subtree ──────────
//
// `mark_inline_descendants`' `InFlowBlock => break` and `walk_inline_children`'s
// `_ => break` lost all three of their witnesses to #513's fix, which flattens a
// `display: inline` element holding an in-flow block into its container's units
// so that neither pass ever meets the block. They were filed as #615 and argued
// to be unreachable. They are not.
//
// The flattening step reads `Node::contributes_in_flow_block`, and
// `recompute_contributes_in_flow_block` folds that field from the **document
// root** down, so a node unreachable from it keeps `false` — deliberately, as
// that function's doc says, because `false` is the pre-#513 classification and
// the conservative direction. Outside the document, therefore, an inline element
// holding a block is *not* split, stays a unit of its container, leaves
// `has_block` false, mints no anonymous box, and leaves the container an IFC root
// whose marking pass and inline walk both walk into the inline element and meet
// the block — exactly as they did before #513.
//
// So these two arms are what makes that conservative direction safe: they are
// #366's rule ("mark exactly what the walk flows") doing its job in the one place
// that still needs it. Each of the three fixtures below asserts **both** halves on
// one document, because the two mutants are not caught by one assertion: making the
// mark `continue` stamps `ifc_root` on the text after the block, and making the walk
// `continue` flows that text into the line ahead of the block. The first two share
// `assert_both_passes_stop_at_the_block`; the third decides on a node one level
// deeper and spells its own, for the reason its doc gives.
//
// Three histories, so that closing one leaves witnesses. None is contrived: rsx
// removes a branch and its `NodeHandle`s can outlive the removal, every `rsx!`
// subtree is built before it is appended, and the third is the first two with a
// `display: contents` wrapper in the way, which is what rsx puts there for every
// `if`/`match`/`for`.
//
// **One change would kill all three at once: #628.** The IFC passes iterate the whole
// slab, so they run over detached subtrees at all — doing Parley work for a subtree
// that paints nothing, on every layout pass. Skipping them is the optimisation #628
// proposes, and it would silently delete the *only* route that reaches either arm.
// If you are that change: these arms need a new witness in the same commit, or an
// explicit decision that they are dead (#615 is the history, and "no test covers it"
// was already not good enough once). Deleting these fixtures as obsolete is the
// `ANCHOR-MISSING` failure mode; they are the witnesses, not decoration. A
// reachability skip also has to seed `tree.anonymous_block_boxes` explicitly — a box
// is not in the element tree (#566), so a naive walk over `children` skips every one
// and takes three unrelated fixtures with it.
//
// The third arm filed with them — `contents_is_inline_transparent`'s *opaque*
// answer — is **not** witnessed by this route and cannot be: the same clearing
// that un-splits the inline element also makes every wrapper in the subtree
// answer *transparent*. Its own argument lives at that function.

/// Assert, on a document whose `container` holds `<a>text<div/>tail</a>` and is
/// **not** part of the document tree, that both passes stop at the block.
///
/// Split out because the two fixtures below differ only in how the subtree came
/// to be detached, and the claim is identical.
fn assert_both_passes_stop_at_the_block(
    doc: &RinchDocument,
    container: NodeId,
    link: NodeId,
    text: NodeId,
    block: NodeId,
    tail: NodeId,
) {
    // Preconditions. Each is load-bearing: if any of them stops holding, the
    // assertions below would pass for the wrong reason (nothing reaches the arms
    // at all), and this fixture would go hollow without failing.
    let a = doc.tree.get(link.0).unwrap();
    assert!(
        !a.contributes_in_flow_block,
        "precondition: the fold does not reach a detached subtree, so the field \
         stays `false` — that is the whole route to these arms"
    );
    assert!(
        !a.is_split_inline(),
        "precondition: and therefore the inline element is not split, so it is \
         still a unit of its container"
    );
    assert!(
        doc.tree.get(container.0).unwrap().text_layout.is_some(),
        "precondition: so the container is an IFC root again"
    );
    assert_eq!(
        ifc_root_of(doc, link),
        Some(container.0),
        "precondition: and the marking pass really walked into the inline element"
    );
    assert_eq!(
        ifc_root_of(doc, text),
        Some(container.0),
        "precondition: the side before the block joined the IFC"
    );
    assert!(
        flowed_by(doc, container.0, text.0),
        "precondition: …and was flowed into the line"
    );

    // `mark_inline_descendants`' `InFlowBlock => break`. With `continue` instead,
    // `tail` is stamped with the root's id while no IFC lays it out — #366's
    // divergence, the one this arm exists to prevent.
    assert_eq!(
        ifc_root_of(doc, tail),
        None,
        "the marking pass stops at the in-flow block: everything after it stays \
         in Taffy, unmarked (#366/#615)"
    );
    assert_eq!(
        ifc_root_of(doc, block),
        None,
        "an in-flow block is never IFC content"
    );

    // `walk_inline_children`'s `_ => break`. With `continue` instead, `tail`'s
    // text is appended to the line *ahead* of the block it comes after.
    assert!(
        !flowed_by(doc, container.0, tail.0),
        "the inline walk stops at the same block, so `tail` reaches no line"
    );
    let layout = doc
        .tree
        .get(container.0)
        .unwrap()
        .text_layout
        .as_ref()
        .unwrap();
    assert_eq!(
        layout.text_content, "text",
        "and the line holds only the side before the block — `texttail` is what \
         walking past it produces"
    );

    // The invariant the arms are one half of, plus the validators, so a future
    // change cannot buy this behaviour with a stranded box.
    assert_marks_match_flow(doc);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    let verdict = doc.tree_check_verdict();
    assert!(
        verdict.fatal.is_empty(),
        "a detached subtree is otherwise healthy: {:?}",
        verdict.fatal
    );
}

/// Route 1: the subtree was laid out **attached** — so every computed style is
/// real, and the `<a>` was genuinely split on the first pass — and then removed
/// from the document while its nodes stayed alive.
///
/// This is the stronger of the two, because nothing about the classification is
/// owed to unresolved styles: the first `resolve_layout` splits the `<a>`, and the
/// second un-splits it purely because the fold can no longer reach it.
///
/// Kills: `InFlowBlock => break` → `continue` in `mark_inline_descendants`
/// (measured: this test plus `a_never_attached_subtree_…`), and `_ => break` →
/// no-op in `walk_inline_children` (same two).
#[test]
fn a_removed_subtree_still_stops_the_mark_and_the_walk_at_a_block() {
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

    assert!(
        doc.tree.get(link.0).unwrap().is_split_inline(),
        "control: attached, the same markup is #513's split inline"
    );
    assert_eq!(
        ifc_root_of(&doc, link),
        None,
        "control: …so it is not inline content of anything"
    );

    doc.remove_child(body, container);
    // A changed viewport, or `resolve_layout` early-returns on `!layout_dirty`
    // and neither pass runs at all.
    doc.resolve_layout(VW - 7.0, VH);

    assert_both_passes_stop_at_the_block(&doc, container, link, text, block, tail);
}

/// Route 2: the subtree was **never** appended — the state every `rsx!` tree
/// passes through between `create_element` and `append_child`.
///
/// Kept beside route 1 rather than folded into it: the two reach the arms by
/// different histories (a real split undone, versus one that never happened), so
/// closing one leaves the other as the witness.
#[test]
fn a_never_attached_subtree_stops_them_at_a_block_too() {
    let mut doc = RinchDocument::new();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "font-size: 16px; line-height: 20px; width: 400px",
    );
    let link = child_of(&mut doc, container, "a", "");
    let text = text_in(&mut doc, link, "text");
    let block = child_of(&mut doc, link, "div", "width: 30px; height: 30px");
    let tail = text_in(&mut doc, link, "tail");
    doc.resolve_layout(VW, VH);

    assert_both_passes_stop_at_the_block(&doc, container, link, text, block, tail);
}

/// The other half of `contents_is_inline_transparent`'s argument, made checkable —
/// and the `break` arms reached a third way, one level deeper.
///
/// The opaque branch of that predicate has no witness, and the reason is structural:
/// `collect_run_units` flattens a `display: contents` wrapper whatever the field
/// says, so an *attached* opaque wrapper always puts its block into its container's
/// units and costs the container its roothood. This pins the detached half. The route
/// that witnessed the `break` arms **inverts** here, because the same clearing that
/// un-splits an inline element also makes every wrapper in the subtree answer
/// *transparent*.
///
/// **The wrapper is inside the `<a>`, and that placement is the whole point.**
/// Directly under the container it is never reached at all (the induction's first
/// case), so a fixture shaped that way would assert the field while claiming a reach
/// that was not happening. Behind a detached — and therefore unsplit — `<a>`, the
/// marking pass walks in, asks the question, and gets `true`; and **only the
/// transparent branch marks**, because the opaque one `break`s before the write. So
/// `wrapper.ifc_root == Some(container)` is the observable proof that the function ran
/// and what it answered.
///
/// **The wrapper holds content after its block, and that is not decoration either.**
/// With the block as its only child, `break` and `continue` do the same thing — the
/// loop has nothing left to skip — so a fixture shaped that way would *reach* both
/// arms and discriminate neither, which is this repo's fixed-point trap in its purest
/// form. `post` is what the arms decide. And the stop is **scoped to the wrapper's own
/// child loop**, not to the line: `tail`, back out in the `<a>`, is still marked and
/// still flowed. That is the pre-#513 behaviour these arms preserve, and asserting it
/// here keeps the next reader from mistaking the scope.
///
/// Kills: deleting the clear in `recompute_contributes_in_flow_block` (its
/// precondition goes first), and both `break` arms, by a different route from the two
/// fixtures above — an earlier version of this doc claimed it killed nothing.
#[test]
fn a_detached_contents_wrapper_answers_transparent_inside_a_detached_inline() {
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
    let wrapper = child_of(&mut doc, link, "span", "display: contents");
    let pre = text_in(&mut doc, wrapper, "pre");
    let block = child_of(&mut doc, wrapper, "div", "width: 30px; height: 30px");
    let post = text_in(&mut doc, wrapper, "post");
    let tail = text_in(&mut doc, link, "tail");
    doc.resolve_layout(VW, VH);

    assert!(
        doc.tree.get(wrapper.0).unwrap().contributes_in_flow_block,
        "control: attached, the wrapper is opaque — it really does hold a block"
    );
    assert!(
        doc.tree.get(link.0).unwrap().is_split_inline(),
        "control: …so the `<a>` around it is split"
    );
    assert!(
        doc.tree.get(container.0).unwrap().text_layout.is_none(),
        "control: …and that is exactly why the container is not an IFC root, so \
         nothing ever asks the wrapper the question while it is attached"
    );

    doc.remove_child(body, container);
    doc.resolve_layout(VW - 7.0, VH);

    // Preconditions: the detached route reopened, one level deeper than the two
    // fixtures above.
    assert!(
        !doc.tree.get(link.0).unwrap().contributes_in_flow_block
            && !doc.tree.get(link.0).unwrap().is_split_inline(),
        "precondition: the fold does not reach a detached subtree, so the `<a>` is \
         no longer split"
    );
    assert!(
        doc.tree.get(container.0).unwrap().text_layout.is_some(),
        "precondition: so the container is an IFC root again"
    );
    assert_eq!(
        role_of(&doc, wrapper),
        InlineFlowRole::Contents,
        "the wrapper is still a `display: contents` element — its computed style \
         survives the detach"
    );
    assert_eq!(
        role_of(&doc, block),
        InlineFlowRole::InFlowBlock,
        "…and it still holds an in-flow block-level box"
    );

    // `contents_is_inline_transparent` answered `true`, and the mark is the proof.
    assert!(
        !doc.tree.get(wrapper.0).unwrap().contributes_in_flow_block,
        "the field is `false` here: `recompute_contributes_in_flow_block` folds from \
         the document root, so the wrapper answers *transparent*. The detached route \
         cannot witness the opaque branch — it inverts it."
    );
    assert_eq!(
        ifc_root_of(&doc, wrapper),
        Some(container.0),
        "and this is the proof the predicate was consulted and answered `true`: only \
         the transparent branch of `mark_inline_descendants`' `Contents` arm writes \
         this mark, the opaque one `break`s before it"
    );
    assert_eq!(
        ifc_root_of(&doc, pre),
        Some(container.0),
        "so the wrapper's content before the block joined the IFC"
    );
    assert!(
        flowed_by(&doc, container.0, pre.0),
        "…and was flowed into the line"
    );

    // The two `break` arms, decided on `post` rather than on `tail`.
    assert_eq!(
        ifc_root_of(&doc, post),
        None,
        "`mark_inline_descendants` stops at the block *inside the wrapper*: what \
         follows it there is left unmarked (#366/#615)"
    );
    assert_eq!(
        ifc_root_of(&doc, block),
        None,
        "an in-flow block is never IFC content"
    );
    assert!(
        !flowed_by(&doc, container.0, post.0),
        "`walk_inline_children` stops at the same block, so `post` reaches no line"
    );

    // The stop is scoped to the wrapper's own loop — `tail` is outside it.
    assert_eq!(
        ifc_root_of(&doc, tail),
        Some(container.0),
        "`tail` is a child of the `<a>`, not of the wrapper, so the wrapper's `break` \
         does not reach it: the `<a>`'s own loop carries on"
    );
    let layout = doc
        .tree
        .get(container.0)
        .unwrap()
        .text_layout
        .as_ref()
        .unwrap();
    assert_eq!(
        layout.text_content, "textpretail",
        "the line holds everything but `post` — `textpreposttail` is what walking \
         past the block produces"
    );
    assert!(
        flowed_by(&doc, container.0, text.0) && flowed_by(&doc, container.0, tail.0),
        "…and both sides outside the wrapper really are in it"
    );

    assert_marks_match_flow(&doc);
    assert_eq!(doc.ifc_leaf_invariant_violations(), Vec::<usize>::new());
    let verdict = doc.tree_check_verdict();
    assert!(
        verdict.fatal.is_empty(),
        "a detached subtree is otherwise healthy: {:?}",
        verdict.fatal
    );
}
