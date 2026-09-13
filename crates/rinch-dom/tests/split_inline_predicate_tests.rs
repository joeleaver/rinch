//! `Node::contributes_in_flow_block` and `Node::is_split_inline` — the
//! classifier #513's fix consumes, landed one PR ahead of it.
//!
//! The field is derived state recomputed bottom-up once per `ifc_dirty` pass
//! (`RinchDocument::recompute_contributes_in_flow_block`). Nothing consumes it
//! yet, so this file is the whole of its coverage, and it is written as a
//! **differential** rather than as a table of expected values:
//!
//! > for every node in the slab, the memoized field equals an independently
//! > written recursive scan of the same rule.
//!
//! That is deliberate. A table of twenty expected `true`/`false` values is a
//! restatement of the implementation, and this repo's recurring test failure is
//! a fixture that agrees with the code because it was read off the code. A
//! second implementation, structured differently — plain recursion here against
//! an iterative post-order there — disagrees wherever either one is wrong, and
//! [`assert_memo_matches_oracle`] refuses a shape whose nodes all answer the
//! same way, so "everything is false" cannot pass for agreement.
//!
//! The named assertions beside the differential are not a second copy of the
//! rule: they pin the handful of nodes each shape is *about*, so a reader can
//! see what the shape claims without running it, and so a future change to the
//! rule fails somewhere that says what it meant.
//!
//! # The one thing this PR must NOT do, and the construction that proves it
//!
//! `contributes_in_flow_block` answers the same question as `ifc.rs`'
//! `contents_is_inline_transparent` — *does this boxless thing wrap an in-flow
//! block-level box* — with one difference: it recurses through a
//! `display: inline` element and that scan does not. So they disagree about a
//! wrapper holding `<a>x<div/>y</a>`, and **at this base that disagreement is
//! observable**: the scan calls the wrapper transparent, so
//! `mark_inline_descendants` recurses into it and `walk_inline_children` flows
//! the text after it; the field would call it opaque, and both would stop.
//!
//! [`the_transparency_reader_now_uses_the_field_and_the_shape_still_renders`] is
//! where that lives. It asserted the *old* answer while the classifier landed on
//! its own — which is what kept that PR inert — and now asserts the new one,
//! because the split makes such a container stop being an IFC root so the
//! question is never asked. The fixture records both halves, because the reason
//! the switch was unsafe is the reason it is now safe.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::{DisplayMode, InlineFlowRole, RinchDocument};

const VW: f32 = 800.0;
const VH: f32 = 600.0;
const CONTAINER: &str = "width: 400px; line-height: 20px; font-size: 16px";
const BLK: &str = "width: 40px; height: 30px";

/// `(contributes_in_flow_block, is_split_inline)` — what every shape below
/// reports about the nodes it cares about.
type Flags = (bool, bool);
/// A plain in-flow block: contributes its own box, and is not an inline.
const BLOCK: Flags = (true, false);
/// A box that contributes nothing to its parent's block-level flow.
const NEITHER: Flags = (false, false);
/// A `display: inline` element broken around block-level content.
const SPLIT: Flags = (true, true);

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn txt(doc: &mut RinchDocument, parent: NodeId, s: &str) -> NodeId {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
    t
}

fn flags(doc: &RinchDocument, id: NodeId) -> Flags {
    let n = doc.tree.get(id.0).expect("node in slab");
    (n.contributes_in_flow_block, n.is_split_inline())
}

// ── The independent oracle ───────────────────────────────────────────────────

/// The rule on [`rinch_dom::Node::contributes_in_flow_block`], written again,
/// recursively, from the rule's prose rather than from its implementation.
///
/// Plain recursion against the production pass's iterative post-order, so the
/// two share no control flow to be wrong in the same way.
fn oracle(doc: &RinchDocument, id: usize) -> bool {
    let node = doc.tree.get(id).expect("node in slab");
    let role = node.inline_flow_role();
    if role == InlineFlowRole::InFlowBlock {
        return true;
    }
    let descends = role == InlineFlowRole::Contents
        || (role == InlineFlowRole::Inline
            && node.is_element()
            && node.display_mode == DisplayMode::Inline);
    descends && node.children.iter().any(|&c| oracle(doc, c))
}

/// The differential, over every node the recompute pass can reach — plus the
/// two classes it deliberately cannot, each asserted to hold its documented
/// value rather than skipped.
///
/// Refuses a shape whose reachable nodes all answer the same way: a
/// differential that passes because every answer is `false` is a run that never
/// happened wearing a green tick.
fn assert_memo_matches_oracle(doc: &RinchDocument, what: &str) {
    // Reachable from the document root through `children` — the walk the
    // recompute pass does.
    let mut reachable: Vec<usize> = Vec::new();
    let mut stack = vec![doc.tree.root_id];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.tree.get(id) else {
            continue;
        };
        reachable.push(id);
        stack.extend_from_slice(&node.children);
    }

    let (mut trues, mut falses) = (0usize, 0usize);
    for &id in &reachable {
        let node = doc.tree.get(id).unwrap();
        let expected = oracle(doc, id);
        assert_eq!(
            node.contributes_in_flow_block,
            expected,
            "{what}: node {id} <{}> display={:?} mode={:?} role={:?}: the \
             memoized `contributes_in_flow_block` disagrees with a fresh \
             recursive scan of the same rule",
            node.tag().unwrap_or("#text"),
            node.computed_style.display,
            node.display_mode,
            node.inline_flow_role(),
        );
        if expected {
            trues += 1;
        } else {
            falses += 1;
        }
    }
    assert!(
        trues > 0 && falses > 0,
        "{what}: the differential is not discriminating — {trues} true / \
         {falses} false. A shape where every reachable node answers the same \
         way cannot tell the memo from a constant."
    );

    for (id, node) in &doc.tree.nodes {
        if node.is_anonymous_block_box {
            // Not in anybody's `children`, so the pass never reaches it;
            // `create_anonymous_block_boxes` sets it, and the rule says `true`
            // because an anonymous block box is an in-flow block-level box.
            assert!(
                node.contributes_in_flow_block,
                "{what}: anonymous block box {id} must carry the value its \
                 minting site sets"
            );
        } else if !reachable.contains(&id) {
            // Unreachable from the root: the pass leaves `false`, which is the
            // pre-field behaviour (the node stays a unit of its parent).
            assert!(
                !node.contributes_in_flow_block,
                "{what}: node {id} is unreachable from the document root and \
                 must keep the conservative `false`"
            );
        }
    }
}

/// Build a shape inside a 400px block container, lay it out, run the
/// differential, and hand back the container plus whatever ids the shape named.
fn shape(
    what: &str,
    build: impl FnOnce(&mut RinchDocument, NodeId) -> Vec<NodeId>,
) -> (RinchDocument, NodeId, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let ids = build(&mut doc, c);
    doc.resolve_layout(VW, VH);
    assert_memo_matches_oracle(&doc, what);
    // The container generates an ordinary in-flow block box, so it contributes
    // one — to *its* parent. True of every shape here, and asserted once.
    assert_eq!(
        flags(&doc, c),
        BLOCK,
        "{what}: the container is a plain block"
    );
    (doc, c, ids)
}

// ── The shapes ───────────────────────────────────────────────────────────────

/// Every in-flow shape the #513 falsifier covers, through the differential.
#[test]
fn the_memo_agrees_with_a_fresh_scan_on_every_block_in_inline_shape() {
    // `<a>text<blk>tail</a>` — the canonical case.
    let (doc, _, ids) = shape("middle", |d, c| {
        let a = el(d, c, "a", "");
        txt(d, a, "text");
        let b = el(d, a, "div", BLK);
        txt(d, b, "block");
        txt(d, a, "tail");
        vec![a, b]
    });
    assert_eq!(flags(&doc, ids[0]), SPLIT, "middle: the <a> is split");
    assert_eq!(flags(&doc, ids[1]), BLOCK, "middle: the block child");

    // Block first, last, only — the three positions, each a fixed point the
    // other two agree with, so each gets its own build.
    for (what, lead, trail) in [
        ("first", false, true),
        ("last", true, false),
        ("only", false, false),
    ] {
        let (doc, _, ids) = shape(what, |d, c| {
            let a = el(d, c, "a", "");
            if lead {
                txt(d, a, "text");
            }
            let b = el(d, a, "div", BLK);
            txt(d, b, "block");
            if trail {
                txt(d, a, "tail");
            }
            vec![a]
        });
        assert_eq!(
            flags(&doc, ids[0]),
            SPLIT,
            "{what}: the <a> is split wherever the block sits in it"
        );
    }

    // Two inlines deep: both are split, which is what Chrome's five-and-three
    // fragment counts say.
    let (doc, _, ids) = shape("nested", |d, c| {
        let a = el(d, c, "a", "");
        txt(d, a, "out");
        let b = el(d, a, "b", "");
        txt(d, b, "in");
        let blk = el(d, b, "div", BLK);
        txt(d, blk, "block");
        txt(d, b, "after");
        txt(d, a, "tailout");
        vec![a, b]
    });
    assert_eq!(flags(&doc, ids[0]), SPLIT, "nested: the outer <a>");
    assert_eq!(flags(&doc, ids[1]), SPLIT, "nested: the inner <b> too");

    // Two blocks in one inline: still one split element, arity 2 for the runs.
    let (doc, _, ids) = shape("two blocks", |d, c| {
        let a = el(d, c, "a", "");
        txt(d, a, "t1");
        let b1 = el(d, a, "div", BLK);
        txt(d, b1, "b1");
        txt(d, a, "t2");
        let b2 = el(d, a, "div", BLK);
        txt(d, b2, "b2");
        txt(d, a, "t3");
        vec![a]
    });
    assert_eq!(flags(&doc, ids[0]), SPLIT, "two blocks: one split <a>");
}

/// The two directions of `display: contents` around the split — a wrapper
/// inside the inline, and an inline inside the wrapper. Both must reach the
/// block, because the wrapper generates no box (CSS 2.1 §9.2.1.1).
#[test]
fn the_recursion_crosses_display_contents_in_both_directions() {
    let (doc, _, ids) = shape("contents inside the inline", |d, c| {
        let a = el(d, c, "a", "");
        txt(d, a, "text");
        let w = el(d, a, "span", "display: contents");
        let blk = el(d, w, "div", BLK);
        txt(d, blk, "block");
        txt(d, a, "tail");
        vec![a, w]
    });
    assert_eq!(
        flags(&doc, ids[0]),
        SPLIT,
        "a block behind a contents wrapper still splits the inline above it"
    );
    assert_eq!(
        flags(&doc, ids[1]),
        (true, false),
        "the wrapper contributes the block it flattens, and is not itself an \
         inline to split"
    );

    let (doc, _, ids) = shape("inline inside the contents wrapper", |d, c| {
        let w = el(d, c, "span", "display: contents");
        let a = el(d, w, "a", "");
        txt(d, a, "x");
        let blk = el(d, a, "div", BLK);
        txt(d, blk, "block");
        txt(d, a, "y");
        vec![w, a]
    });
    assert_eq!(
        flags(&doc, ids[0]),
        (true, false),
        "the wrapper contributes the block from two levels down"
    );
    assert_eq!(
        flags(&doc, ids[1]),
        SPLIT,
        "…and the <a> inside it is split"
    );
}

/// The negative controls — every shape that looks like block-in-inline one
/// field away and is **not** it. Each of these is a case the split must leave
/// alone, so each steps off one of the predicate's own fixed points.
#[test]
fn the_predicate_refuses_every_neighbouring_shape() {
    // An out-of-flow child does not split an inline (CSS 2.1 §9.4.2). This is
    // #591's shape and the split must not claim it — that fix gives the box a
    // Taffy parent without touching the classification.
    let (doc, _, ids) = shape("out-of-flow child", |d, c| {
        d.set_attribute(c, "style", &format!("{CONTAINER}; position: relative"));
        let a = el(d, c, "a", "");
        txt(d, a, "text");
        let b = el(d, a, "div", &format!("{BLK}; position: absolute"));
        txt(d, b, "block");
        txt(d, a, "tail");
        vec![a, b]
    });
    assert_eq!(
        flags(&doc, ids[0]),
        NEITHER,
        "an absolutely positioned child does not split its inline — #591 was a \
         different defect, fixed without splitting"
    );
    assert_eq!(
        flags(&doc, ids[1]),
        NEITHER,
        "…and an out-of-flow box contributes nothing to block-level flow"
    );

    // `display: none` generates no box at all.
    let (doc, _, ids) = shape("hidden child", |d, c| {
        let a = el(d, c, "a", "");
        txt(d, a, "text");
        let b = el(d, a, "div", &format!("{BLK}; display: none"));
        txt(d, b, "block");
        txt(d, a, "tail");
        vec![a, b]
    });
    assert_eq!(
        flags(&doc, ids[0]),
        NEITHER,
        "a hidden child generates no box, so there is nothing to split around"
    );
    assert_eq!(flags(&doc, ids[1]), NEITHER, "…nor does it contribute one");

    // The recursion stops at **every** atomic inline: a block inside an
    // `inline-block`, `inline-flex` or `inline-grid` is that box's own business
    // (#592). `inline-grid` joined the list in #607 and is here because #513's
    // recursion reads `DisplayMode::Inline` exactly rather than
    // `is_inline_level`, so a new inline-level variant is a case it has to be
    // checked against rather than one it inherits.
    for display in ["inline-block", "inline-flex", "inline-grid"] {
        let (doc, _, ids) = shape("block inside an atomic inline", |d, c| {
            let a = el(d, c, "a", "");
            txt(d, a, "text");
            let ib = el(d, a, "span", &format!("display: {display}"));
            let b = el(d, ib, "div", BLK);
            txt(d, b, "block");
            txt(d, a, "tail");
            vec![a, ib]
        });
        assert_eq!(
            flags(&doc, ids[0]),
            NEITHER,
            "{display}: the recursion stops at an atomic inline, so the <a> \
             above it is not split"
        );
        assert_eq!(
            flags(&doc, ids[1]),
            NEITHER,
            "{display}: an atomic inline is inline-level, so it contributes no \
             block-level box to its parent's flow either — which is what keeps \
             #592 a separate defect rather than a case of this one"
        );
    }

    // An inline holding only inline content.
    let (doc, _, ids) = shape("all-inline", |d, c| {
        let a = el(d, c, "a", "");
        txt(d, a, "text");
        let i = el(d, a, "i", "");
        txt(d, i, "em");
        txt(d, a, "tail");
        vec![a, i]
    });
    assert_eq!(flags(&doc, ids[0]), NEITHER, "no block, no split");
    assert_eq!(flags(&doc, ids[1]), NEITHER, "nor the inline inside it");
}

/// `<button>`, whose UA `display` is an atomic inline, holding a block — the
/// negative control #513's design comment asks for by name, because it is the
/// shape an over-wide recursion fires on.
#[test]
fn a_block_inside_a_button_inside_an_inline_does_not_split_the_inline() {
    let (doc, _, ids) = shape("button", |d, c| {
        let a = el(d, c, "a", "");
        txt(d, a, "text");
        let btn = el(d, a, "button", "");
        let b = el(d, btn, "div", BLK);
        txt(d, b, "block");
        txt(d, a, "tail");
        vec![a, btn]
    });
    assert_eq!(
        flags(&doc, ids[0]),
        NEITHER,
        "a <button> is an atomic inline, so a block inside it is laid out by \
         the button and does not split the <a>"
    );
}

/// A flex item: Stylo blockifies it, so `display: inline` never survives to be
/// asked. Measured rather than assumed — the predicate cannot fire here because
/// the mode is already `Block` by the time the pass runs.
#[test]
fn a_blockified_flex_item_is_not_a_split_inline() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "display: flex; width: 400px");
    let a = el(&mut doc, c, "a", "");
    txt(&mut doc, a, "text");
    let b = el(&mut doc, a, "div", BLK);
    txt(&mut doc, b, "block");
    doc.resolve_layout(VW, VH);

    let n = doc.tree.get(a.0).unwrap();
    assert_eq!(
        n.display_mode,
        DisplayMode::Block,
        "precondition: Stylo blockifies a flex item, so the <a> is not inline"
    );
    assert_eq!(
        flags(&doc, a),
        BLOCK,
        "a blockified flex item contributes its own box and is not a split \
         inline"
    );
    assert_memo_matches_oracle(&doc, "flex item");
}

/// The field survives a **transition**, in both directions — which is the
/// failure mode this region has historically had: derived state that is only
/// ever set goes stale in the direction nobody notices (`ifc_root`, #597).
///
/// Every re-resolve uses a **changed viewport**. `resolve_layout` early-returns
/// on `!layout_dirty` before the IFC block, so a repeat at the same size runs
/// neither the pass nor the differential, and the fixture would test nothing
/// while passing.
///
/// **What this fixture does NOT cover, measured:** the clearing loop at the top
/// of `recompute_contributes_in_flow_block`. The mutant that deletes it survives
/// this fixture and the whole of `-p rinch-dom -p rinch`, because the fold
/// *writes* every node the walk reaches — `false` included — so nothing reachable
/// can carry a stale value. The clear only bites for a node that has left the
/// document root, which nothing reads while it is detached. That is recorded at
/// the loop itself; it is stated here too so this fixture's name is not read as a
/// claim it does not support.
#[test]
fn the_field_is_recomputed_in_both_directions() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let a = el(&mut doc, c, "a", "");
    txt(&mut doc, a, "text");
    doc.resolve_layout(VW, VH);
    assert_eq!(flags(&doc, a), NEITHER, "an all-inline <a> is not split");
    assert_memo_matches_oracle(&doc, "before the block arrives");

    // Enters the shape structurally.
    let blk = el(&mut doc, a, "div", BLK);
    txt(&mut doc, blk, "block");
    doc.resolve_layout(VW - 1.0, VH);
    assert_eq!(
        flags(&doc, a),
        SPLIT,
        "appending a block child must make the <a> split"
    );
    assert_memo_matches_oracle(&doc, "after the block arrives");

    // Leaves it again, without a structural change.
    doc.set_attribute(blk, "style", &format!("{BLK}; display: none"));
    doc.resolve_layout(VW - 2.0, VH);
    assert_eq!(
        flags(&doc, a),
        NEITHER,
        "hiding the block child must clear the field"
    );
    assert_memo_matches_oracle(&doc, "after the block is hidden");

    // And back.
    doc.set_attribute(blk, "style", BLK);
    doc.resolve_layout(VW - 3.0, VH);
    assert_eq!(flags(&doc, a), SPLIT, "showing it again must set it again");
    assert_memo_matches_oracle(&doc, "after the block is shown again");

    // The wrapper's own display crossing — the one #597 had to make re-run the
    // IFC pass at all, because `DisplayValue::to_taffy` is not injective and
    // `inline` <-> `block` changes no Taffy field. (`grid` <-> `inline-grid` is
    // the newest aliasing pair, #607. It is not a problem for the same reason:
    // it crosses `is_inline_level`, which is the condition #597 added at
    // `style_resolution/mod.rs`'s `ifc_dirty` trigger.)
    doc.set_attribute(a, "style", "display: block");
    doc.resolve_layout(VW - 4.0, VH);
    assert_eq!(
        flags(&doc, a),
        BLOCK,
        "a block container is not a split inline, whatever it holds — and it \
         contributes an in-flow block-level box: itself"
    );
    assert_memo_matches_oracle(&doc, "wrapper restyled to block");

    doc.set_attribute(a, "style", "display: inline");
    doc.resolve_layout(VW - 5.0, VH);
    assert_eq!(flags(&doc, a), SPLIT, "back to inline: split again");
    assert_memo_matches_oracle(&doc, "wrapper restyled back to inline");
}

// ── The reader that moved ────────────────────────────────────────────────────

/// **`contents_is_inline_transparent` reads this field now, and this fixture is
/// the construction that says why it could not before.**
///
/// The two answers differ: the field recurses through a `display: inline`
/// element and the old `scan_contents_children` did not. So for
///
/// ```html
/// <div>lead<span style="display: contents"><a>x<div/>y</a></span>tail</div>
/// ```
///
/// the scan said *transparent* — its walk reached the `<a>`, recorded "inline
/// content" and stopped — and the field says *contributes*. When the classifier
/// landed (one PR earlier) that difference was **observable and harmful**:
/// nothing flattened the `<a>` yet, so every one of the container's units was
/// inline-level, no anonymous box was minted, the container really was an IFC
/// root, and calling the wrapper opaque would have stopped both the marking pass
/// and the walk at it — dropping `tail` entirely. This fixture asserted the old
/// answer, and that is what kept the reader where it was.
///
/// It is safe now, and for the reason that was false then: the `<a>` is a **split
/// inline**, so it is flattened into the container's units, the container holds an
/// in-flow block-level unit, an anonymous box takes each inline run, and the
/// container is **not an IFC root at all** — so nothing asks the question. The
/// assertions below are the new answer, and the last of them is the one that
/// matters: the shape renders exactly like the same content with the wrapper
/// chain deleted.
#[test]
fn the_transparency_reader_now_uses_the_field_and_the_shape_still_renders() {
    fn build(wrapped: bool) -> (RinchDocument, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        txt(&mut doc, c, "lead");
        // Wrapped: `contents` wrapper -> `<a>` -> [x, block, y]. Unwrapped: the
        // same five boxes straight in the container, which is the shape rinch
        // has always rendered correctly.
        let host = if wrapped {
            let w = el(&mut doc, c, "span", "display: contents");
            el(&mut doc, w, "a", "")
        } else {
            c
        };
        txt(&mut doc, host, "x");
        let blk = el(&mut doc, host, "div", BLK);
        txt(&mut doc, blk, "block");
        txt(&mut doc, host, "y");
        let tail = txt(&mut doc, c, "tail");
        doc.resolve_layout(VW, VH);
        (doc, c, tail)
    }

    let (wdoc, wc, wtail) = build(true);
    let (pdoc, pc, ptail) = build(false);

    // The container is no longer a root, which is the whole reason the switch is
    // safe — so the question the reader answers is never put to it.
    assert!(
        wdoc.tree.get(wc.0).unwrap().text_layout.is_none(),
        "the container must not be an IFC root any more: its inline content \
         belongs to the anonymous boxes minted around the split"
    );
    // `tail` is inline content of an anonymous box, not of the container.
    let tail_root = wdoc
        .tree
        .get(wtail.0)
        .unwrap()
        .ifc_root
        .expect("tail has an IFC");
    assert!(
        wdoc.tree.get(tail_root).unwrap().is_anonymous_block_box,
        "`tail` is laid out by an anonymous block box, not by the container"
    );
    assert_eq!(
        wdoc.tree.get(ptail.0).unwrap().ifc_root.is_some(),
        pdoc.tree.get(ptail.0).unwrap().ifc_root.is_some(),
        "the twin agrees about whether `tail` belongs to an IFC"
    );

    // And the answer that matters: the wrapper chain changes nothing.
    let h = |d: &RinchDocument, id: NodeId| d.tree.get(id.0).unwrap().layout.height;
    assert_eq!(
        h(&pdoc, pc),
        20.0 + 30.0 + 20.0,
        "control: `lead x` / block / `y tail` is three bands tall"
    );
    assert_eq!(
        h(&wdoc, wc),
        h(&pdoc, pc),
        "a `display: contents` wrapper around a split inline must change \
         nothing — this is the assertion the reader switch had to earn"
    );
    assert!(
        wdoc.taffy_tree_violations().is_empty(),
        "{:?}",
        wdoc.taffy_tree_violations()
    );
    assert_memo_matches_oracle(&wdoc, "the wrapper the two answers disagreed about");
}
