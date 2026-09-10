//! `create_anonymous_block_boxes` classifies a container's **flattened** child
//! list (#568).
//!
//! `display: contents` generates no box, so the boxes a block container holds
//! are its own non-contents children plus whatever its contents children stand
//! for, at any depth. "Does this container hold both inline and block-level
//! content", and "where do the runs of inline content start and stop", are
//! questions about those boxes — and the scan asked them of raw
//! `nodes[parent].children`, which cannot see the flattening. That is #476's
//! false premise one step earlier, and it cost two visible things:
//!
//!  1. A transparent wrapper between two runs of text neither joined the run
//!     nor ended it, so its inline content stayed a DOM sibling of the
//!     anonymous box and Taffy laid it out as its own block. `before` and
//!     ` after` shared line 1 while the `MIDDLE` written *between* them sat on
//!     line 2 — a browser renders one line.
//!  2. A contents wrapper holding `text + block` was itself classified as mixed
//!     content (contents computes `DisplayMode::Block`) and minted the box
//!     inside a boxless element, where `ComputedStyle::for_anonymous_box` made
//!     the box boxless too. Its run was then laid out as bare Taffy children:
//!     two text nodes stacking as two blocks.
//!
//! **The oracle is the same content with the wrapper deleted**, built in the
//! same test wherever the shape allows it, so no line height is hard-coded and
//! nothing drifts with the font stack.
//!
//! **Fixed points stepped off on purpose** (this project's recurring test
//! failure — every fixture here was watched failing against a deliberately
//! broken build, see the `Kills:` lines):
//!
//!  - **Contents chain length.** One wrapper is where "flatten" and "take the
//!    children of the child" agree, so there are two- and three-deep fixtures.
//!  - **Run cardinality.** One run per container is where "place the box at the
//!    head's slot" and "place it at the container's slot" agree, so there is a
//!    two-runs-in-one-wrapper fixture — and it is also the only shape in which
//!    dissolving the boxes forwards instead of backwards is observable.
//!  - **Text-node cardinality.** One text node in a wrapper is where "one IFC
//!    line" and "one bare Taffy block" agree (both 1 line tall), so the
//!    symptom-2 fixture has two.
//!  - **Wrapper cardinality.** An empty wrapper is arity 0 for the flatten
//!    recursion and has its own fixture.
//!  - **Cross-parent runs.** A run whose members all share one parent is where
//!    "restore into the origin" and "restore into the box's parent" agree, so
//!    the round-trip fixtures all span a wrapper boundary.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;
/// Every container below fixes `line-height`, so one line is exactly this and
/// "one line" versus "two" is a 20px question rather than a font question.
const LINE: f32 = 20.0;
const CONTAINER: &str = "width: 400px; line-height: 20px; font-size: 16px";

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

fn txt(doc: &mut RinchDocument, parent: NodeId, s: &str) -> NodeId {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
    t
}

fn height_of(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.height
}

/// The IFC root a node's inline content belongs to, if any.
fn ifc_root(doc: &RinchDocument, id: NodeId) -> Option<usize> {
    doc.tree.get(id.0).unwrap().ifc_root
}

/// Whether a node ended up as inline content of an *anonymous* IFC root — i.e.
/// whether its container was classified as mixed content.
fn is_in_anonymous_root(doc: &RinchDocument, id: NodeId) -> bool {
    ifc_root(doc, id)
        .and_then(|r| doc.tree.get(r))
        .is_some_and(|r| r.is_anonymous_block_box)
}

/// A node's DOM children rendered as a comparable string: an element by tag, a
/// text node by content, an anonymous box as `ANON`.
fn dom_children(doc: &RinchDocument, id: NodeId) -> Vec<String> {
    doc.tree
        .get(id.0)
        .unwrap()
        .children
        .iter()
        .map(|&c| {
            let n = doc.tree.get(c).unwrap();
            if n.is_anonymous_block_box {
                "ANON".to_string()
            } else if let Some(tag) = n.tag() {
                format!("<{tag}>")
            } else {
                n.text_content().unwrap_or_default().to_string()
            }
        })
        .collect()
}

fn assert_consistent(doc: &RinchDocument, what: &str) {
    let v = doc.taffy_tree_violations();
    assert!(
        v.is_empty(),
        "{what}: Taffy tree is inconsistent:\n  {}",
        v.join("\n  ")
    );
}

// ── Symptom 1: a transparent wrapper belongs to the run around it ──────────

/// `before <w>MIDDLE</w> after` in a container that is mixed for an independent
/// reason is **one** line, exactly as the same markup with the wrapper deleted.
///
/// The oracle is built in the same test, so the assertion is "the boxless
/// wrapper changed nothing", not a font measurement.
///
/// Kills: reading `has_inline` / the run grouping from raw `node.children`
/// (the wrapper's span becomes its own block on a second line, +20px), and
/// dropping the adoption of a run member that lives behind a wrapper.
#[test]
fn a_transparent_wrapper_between_two_runs_of_text_is_one_line() {
    fn build(wrapped: bool) -> (RinchDocument, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let before = txt(&mut doc, c, "before ");
        let host = if wrapped {
            el(&mut doc, c, "span", "display: contents")
        } else {
            c
        };
        txt(&mut doc, host, "MIDDLE");
        txt(&mut doc, c, " after");
        el(&mut doc, c, "div", "width: 40px; height: 20px");
        doc.resolve_layout(VW, VH);
        (doc, c, before)
    }

    let (wdoc, wc, wbefore) = build(true);
    let (pdoc, pc, _) = build(false);

    assert_eq!(
        height_of(&wdoc, wc),
        height_of(&pdoc, pc),
        "a boxless wrapper must not add a line: wrapped={}, unwrapped={}",
        height_of(&wdoc, wc),
        height_of(&pdoc, pc)
    );
    // Off the fixed point: the control must itself be the one-line-plus-block
    // shape, or "both are two lines" would pass.
    assert_eq!(
        height_of(&pdoc, pc),
        LINE + 20.0,
        "control: one line of text plus the 20px block"
    );
    assert!(
        is_in_anonymous_root(&wdoc, wbefore),
        "the container is mixed, so its text is in an anonymous box"
    );
    assert_consistent(&wdoc, "transparent wrapper in a run");
}

/// The wrapper's own inline content joins that run — asserted on identity, not
/// only on the container's height, because a height can be right for the wrong
/// reason.
///
/// Kills: the same mutants, plus one that puts the wrapper's content in a
/// *second* anonymous box.
#[test]
fn the_wrapped_text_is_in_the_same_inline_run_as_the_text_around_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let before = txt(&mut doc, c, "before ");
    let w = el(&mut doc, c, "span", "display: contents");
    let mid = txt(&mut doc, w, "MIDDLE");
    let after = txt(&mut doc, c, " after");
    el(&mut doc, c, "div", "width: 40px; height: 20px");
    doc.resolve_layout(VW, VH);

    let root = ifc_root(&doc, before).expect("the text is inline content of some IFC");
    assert_eq!(ifc_root(&doc, mid), Some(root), "the wrapped text joins it");
    assert_eq!(
        ifc_root(&doc, after),
        Some(root),
        "and so does the text after"
    );
    assert!(
        doc.tree.get(root).unwrap().is_anonymous_block_box,
        "and that IFC is the anonymous box, not the container"
    );
    assert_consistent(&doc, "one run across a wrapper");
}

/// **Three** contents levels between the container and the text. One wrapper is
/// where "flatten recursively" and "take the contents child's own children"
/// agree, and every fixture above has exactly one.
///
/// Kills: a `flatten_effective_children` that descends a single level.
#[test]
fn a_three_deep_wrapper_chain_still_joins_the_run() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let before = txt(&mut doc, c, "before ");
    let w1 = el(&mut doc, c, "span", "display: contents");
    let w2 = el(&mut doc, w1, "span", "display: contents");
    let w3 = el(&mut doc, w2, "span", "display: contents");
    let mid = txt(&mut doc, w3, "MIDDLE");
    let after = txt(&mut doc, c, " after");
    el(&mut doc, c, "div", "width: 40px; height: 20px");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        height_of(&doc, c),
        LINE + 20.0,
        "three boxless wrappers are still no box"
    );
    let root = ifc_root(&doc, before).expect("inline content");
    assert_eq!(ifc_root(&doc, mid), Some(root));
    assert_eq!(ifc_root(&doc, after), Some(root));
    assert_consistent(&doc, "three-deep chain");
}

/// An **empty** wrapper is arity 0 for the flatten recursion: it contributes
/// nothing, so it must neither end the run nor add a box.
#[test]
fn an_empty_wrapper_between_two_runs_does_not_split_them() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let one = txt(&mut doc, c, "one ");
    el(&mut doc, c, "span", "display: contents");
    let two = txt(&mut doc, c, "two");
    el(&mut doc, c, "div", "height: 7px");
    doc.resolve_layout(VW, VH);

    assert_eq!(height_of(&doc, c), LINE + 7.0);
    assert_eq!(
        ifc_root(&doc, one),
        ifc_root(&doc, two),
        "an empty wrapper contributes no box, so it cannot split a run"
    );
    assert_consistent(&doc, "empty wrapper");
}

// ── Symptom 2: the wrapper is the mixed content, the container mints ───────

/// A wrapper holding `text text block` is the container's mixed content. Its
/// two text nodes are **one** line, not two blocks.
///
/// Two text nodes rather than one on purpose: with one, "one IFC line" and "one
/// bare Taffy text block" are both a single line tall and the fixture cannot
/// tell them apart.
///
/// Kills: dropping the `display: contents` container guard (the wrapper mints
/// the box itself, `for_anonymous_box` propagates `Contents` to it, and it
/// generates no box either — the two text nodes stack, +20px); and
/// `for_anonymous_box` propagating `Contents` at all.
#[test]
fn a_wrapper_holding_text_and_a_block_puts_its_text_on_one_line() {
    fn build(wrapped: bool) -> (RinchDocument, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let host = if wrapped {
            el(&mut doc, c, "span", "display: contents")
        } else {
            c
        };
        let t1 = txt(&mut doc, host, "t1 ");
        txt(&mut doc, host, "t2");
        el(&mut doc, host, "div", "height: 30px");
        doc.resolve_layout(VW, VH);
        (doc, c, t1)
    }

    let (wdoc, wc, wt1) = build(true);
    let (pdoc, pc, _) = build(false);

    assert_eq!(
        height_of(&pdoc, pc),
        LINE + 30.0,
        "control: one line of text plus the 30px block"
    );
    assert_eq!(
        height_of(&wdoc, wc),
        height_of(&pdoc, pc),
        "the same content behind a boxless wrapper must lay out identically: \
         wrapped={}, unwrapped={}",
        height_of(&wdoc, wc),
        height_of(&pdoc, pc)
    );
    assert!(
        is_in_anonymous_root(&wdoc, wt1),
        "the text is inline content of a real anonymous block box; a box that \
         inherited `display: contents` generates nothing and leaves it to Taffy"
    );
    assert_consistent(&wdoc, "wrapper is the mixed content");
}

/// The same shape two contents levels down. One level is where the guard and
/// the flattening cannot be told apart from a single step.
#[test]
fn a_wrapper_two_levels_down_holding_text_and_a_block_is_still_one_line() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let w1 = el(&mut doc, c, "span", "display: contents");
    let w2 = el(&mut doc, w1, "span", "display: contents");
    let t1 = txt(&mut doc, w2, "t1 ");
    txt(&mut doc, w2, "t2");
    el(&mut doc, w2, "div", "height: 30px");
    doc.resolve_layout(VW, VH);

    assert_eq!(height_of(&doc, c), LINE + 30.0);
    assert!(is_in_anonymous_root(&doc, t1));
    assert_consistent(&doc, "two levels down");
}

// ── Ordering: whose list the box goes in ──────────────────────────────────

/// A wrapper with a block **between** two runs mints two boxes, and the block
/// has to stay between them.
///
/// This is the run-cardinality fixed point: with one run, putting the box in
/// the head's own parent and putting it in the container are the same picture.
/// With two, anchoring both to the container flattens them out as
/// `box box block` where document order is `box block box` — visible as the
/// block moving to the bottom.
///
/// Kills: minting the box in `parent_id` rather than in the run head's own
/// parent.
#[test]
fn a_block_between_two_runs_inside_one_wrapper_stays_between_them() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let w = el(&mut doc, c, "span", "display: contents");
    let a = txt(&mut doc, w, "a ");
    let blk = el(&mut doc, w, "div", "height: 30px");
    let b = txt(&mut doc, w, "b ");
    doc.resolve_layout(VW, VH);

    assert_eq!(height_of(&doc, c), LINE + 30.0 + LINE);
    let ra = ifc_root(&doc, a).expect("inline content");
    let rb = ifc_root(&doc, b).expect("inline content");
    assert_ne!(ra, rb, "the block between them ends the first run");

    let y_a = doc.tree.get(ra).unwrap().layout.y;
    let y_blk = doc.tree.get(blk.0).unwrap().layout.y;
    let y_b = doc.tree.get(rb).unwrap().layout.y;
    assert_eq!(
        (y_a, y_blk, y_b),
        (0.0, LINE, LINE + 30.0),
        "document order must survive the boxing: line, block, line"
    );
    assert_consistent(&doc, "block between two runs in a wrapper");
}

/// A run that starts in the container, crosses into a wrapper and comes back
/// out is one run — and the block after it still follows.
///
/// This is the cross-parent fixture: every member of the run has a different
/// DOM parent from at least one other.
#[test]
fn a_run_that_crosses_into_a_wrapper_and_back_is_one_line() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let x = txt(&mut doc, c, "x ");
    let w = el(&mut doc, c, "span", "display: contents");
    let a = txt(&mut doc, w, "a ");
    let y = txt(&mut doc, c, "y");
    el(&mut doc, c, "div", "height: 30px");
    doc.resolve_layout(VW, VH);

    assert_eq!(height_of(&doc, c), LINE + 30.0);
    let root = ifc_root(&doc, x).expect("inline content");
    assert_eq!(ifc_root(&doc, a), Some(root));
    assert_eq!(ifc_root(&doc, y), Some(root));
    assert_consistent(&doc, "run crossing a wrapper boundary");
}

/// The full shape from the issue's third measurement: inline content on both
/// sides of a wrapper that itself holds a block between two runs. Two lines
/// with the block between them, and each line pairs one container text node
/// with one wrapper text node.
#[test]
fn inline_content_on_both_sides_of_a_split_wrapper_pairs_up_correctly() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let x = txt(&mut doc, c, "x ");
    let w = el(&mut doc, c, "span", "display: contents");
    let a = txt(&mut doc, w, "a ");
    let blk = el(&mut doc, w, "div", "height: 30px");
    let b = txt(&mut doc, w, "b ");
    let y = txt(&mut doc, c, "y");
    doc.resolve_layout(VW, VH);

    assert_eq!(height_of(&doc, c), LINE + 30.0 + LINE);
    let first = ifc_root(&doc, x).expect("inline content");
    let second = ifc_root(&doc, b).expect("inline content");
    assert_eq!(ifc_root(&doc, a), Some(first), "`x` and `a` share a line");
    assert_eq!(ifc_root(&doc, y), Some(second), "`b` and `y` share a line");
    assert_ne!(first, second, "the block splits them");
    assert_eq!(
        (
            doc.tree.get(first).unwrap().layout.y,
            doc.tree.get(blk.0).unwrap().layout.y,
            doc.tree.get(second).unwrap().layout.y,
        ),
        (0.0, LINE, LINE + 30.0)
    );
    assert_consistent(&doc, "split wrapper with text on both sides");
}

// ── The round trip: dissolving the boxes must restore the author's DOM ────

/// A grandchild in a run **never leaves its wrapper** (#568, #566).
///
/// This replaces a round-trip test — "adopted out, and put back" — with the
/// stronger statement that there is no trip. Since #566 a run is *recorded*
/// (`run_members` / `run_box`), not reparented, so the author's tree is never
/// rewritten and the whole class of restore bugs the old test guarded against
/// cannot arise: no anchor to lose, no slot to mis-compute, nothing to undo on
/// a document mutated between passes.
#[test]
fn a_grandchild_in_a_run_never_leaves_its_wrapper() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    txt(&mut doc, c, "x ");
    let w = el(&mut doc, c, "span", "display: contents");
    let a = txt(&mut doc, w, "a ");
    let blk = el(&mut doc, w, "div", "height: 30px");
    txt(&mut doc, c, "y");
    doc.resolve_layout(VW, VH);

    let author_w = vec!["a ".to_string(), "<div>".to_string()];
    let author_c = vec!["x ".to_string(), "<span>".to_string(), "y".to_string()];
    assert_eq!(
        dom_children(&doc, w),
        author_w,
        "the wrapper keeps its own children while its text is in a run"
    );
    assert_eq!(dom_children(&doc, c), author_c, "and so does the container");
    // The run is recorded instead: the grandchild names its box, and the box
    // hangs off the container whose flattened list was classified.
    let bx = doc
        .tree
        .get(a.0)
        .unwrap()
        .run_box
        .expect("`a ` is in a run");
    assert!(doc.tree.get(bx).unwrap().run_members.contains(&a.0));
    assert_eq!(doc.tree.get(bx).unwrap().parent, Some(c.0));

    // Hide the block: the container is no longer mixed, so this pass dissolves
    // the boxes and mints none. Nothing has to be put back.
    doc.set_attribute(blk, "style", "display: none");
    doc.resolve_layout(VW, VH);

    assert_eq!(dom_children(&doc, w), author_w, "still the author's list");
    assert_eq!(
        dom_children(&doc, c),
        author_c,
        "and still the author's list"
    );
    assert_eq!(
        doc.tree.get(a.0).unwrap().run_box,
        None,
        "and the run is gone"
    );
    assert_consistent(&doc, "after the dissolving pass");
}

/// A wrapper that stands for an **entire** inline run is adopted whole: the
/// box takes the wrapper, and the wrapper keeps its own children.
///
/// This is what keeps #568 from deepening #566. An anonymous box adopts its run
/// out of the author's tree until the next pass dissolves it, and
/// `insert_before(parent, new, adopted)` cannot then find its reference — so
/// the run moves the **shallowest** node that stands for exactly its content.
/// Flattening the wrapper away would move content the author put *inside* it,
/// one level further from anything they can address.
///
/// Kills: flattening a contents wrapper unconditionally in `collect_run_units`.
#[test]
fn a_wrapper_that_is_a_whole_run_is_adopted_whole() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    txt(&mut doc, c, "before ");
    let w = el(&mut doc, c, "span", "display: contents");
    let mid = txt(&mut doc, w, "MIDDLE");
    txt(&mut doc, c, " after");
    el(&mut doc, c, "div", "height: 30px");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        dom_children(&doc, w),
        vec!["MIDDLE".to_string()],
        "the wrapper's own child must not be taken out of it"
    );
    assert_eq!(
        doc.tree.get(mid.0).unwrap().parent,
        Some(w.0),
        "and it is still the wrapper's child, not the box's"
    );
    // The wrapper itself is what moved, and it is still on the same line.
    assert_eq!(height_of(&doc, c), LINE + 30.0);
    assert!(
        is_in_anonymous_root(&doc, mid),
        "its text still flows into the run"
    );
    assert_consistent(&doc, "wrapper adopted whole");
}

/// A **nested** wrapper chain is adopted at its **outermost** level: the run
/// takes `w1`, and `w2` stays inside it.
///
/// One wrapper is the arity fixed point of the keep-whole test — with a chain
/// of length 1 there is no "outermost" to get wrong. With two, a rule that
/// counted only leaf inline content would find `w1`'s contribution to be a
/// single `Contents` unit, judge it to hold no inline content, flatten it, and
/// adopt `w2` out of `w1` instead. Same line, same pixels, and one more DOM
/// edge broken than necessary — invisible to every geometry assertion in this
/// file.
///
/// Kills: dropping the nested-`Contents` case from the inline-content test in
/// `collect_run_units` (the induction that makes a chain answer as one).
#[test]
fn a_nested_wrapper_chain_is_adopted_at_its_outermost_level() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    txt(&mut doc, c, "before ");
    let w1 = el(&mut doc, c, "span", "display: contents");
    let w2 = el(&mut doc, w1, "span", "display: contents");
    let mid = txt(&mut doc, w2, "MIDDLE");
    txt(&mut doc, c, " after");
    el(&mut doc, c, "div", "height: 30px");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        dom_children(&doc, w1),
        vec!["<span>".to_string()],
        "the inner wrapper must still be inside the outer one — the run should \
         have taken `w1`, not reached past it for `w2`"
    );
    assert_eq!(
        dom_children(&doc, w2),
        vec!["MIDDLE".to_string()],
        "and the text must still be inside the inner wrapper"
    );
    assert_eq!(
        height_of(&doc, c),
        LINE + 30.0,
        "still one line, then the block"
    );
    assert!(
        is_in_anonymous_root(&doc, mid),
        "and the text flows into the run"
    );
    assert_consistent(&doc, "nested chain adopted whole");
}

/// The chain that is a whole run **inside** and only part of one **outside**:
/// `w1` holds `w2` *and a block*, so `w1` is broken up and `w2` — which is a
/// whole run — is what the run takes.
///
/// This is the level-boundary case: the keep-whole test passes at `w2` and
/// fails at `w1`, in one chain. A rule that decided per chain rather than per
/// level gets one of the two wrong, and which one depends on whether it looks
/// top-down or bottom-up.
#[test]
fn a_chain_whole_inside_and_partial_outside_is_split_at_the_right_level() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    txt(&mut doc, c, "before ");
    let w1 = el(&mut doc, c, "span", "display: contents");
    let w2 = el(&mut doc, w1, "span", "display: contents");
    let mid = txt(&mut doc, w2, "MIDDLE");
    let blk = el(&mut doc, w1, "div", "height: 30px");
    txt(&mut doc, c, " after");
    doc.resolve_layout(VW, VH);

    // **Which level is kept whole**, asked of the run bookkeeping rather than
    // of the DOM. The old form read this off the wrapper's `children`, which
    // only worked while a run was adopted *out* of them; since #566 nothing is
    // adopted, so both wrappers keep the author's list either way and that
    // reading can no longer distinguish the two cases. The fact itself is
    // unchanged and is exactly what `collect_run_units` decides.
    assert_eq!(
        dom_children(&doc, w2),
        vec!["MIDDLE".to_string()],
        "nothing is taken from anybody now"
    );
    assert_eq!(
        dom_children(&doc, w1),
        vec!["<span>".to_string(), "<div>".to_string()],
        "including the outer wrapper, which keeps both its children"
    );
    assert!(
        doc.tree.get(w2.0).unwrap().run_box.is_some(),
        "the inner wrapper is a whole run, so it joins one as a single unit"
    );
    assert_eq!(
        doc.tree.get(w1.0).unwrap().run_box,
        None,
        "the outer one is only partly taken, so it is broken up and is no unit"
    );
    // "before MIDDLE" / block / " after" — the block splits the two runs.
    assert_eq!(height_of(&doc, c), LINE + 30.0 + LINE);
    let (y_first, y_blk) = (
        doc.tree.get(ifc_root(&doc, mid).unwrap()).unwrap().layout.y,
        doc.tree.get(blk.0).unwrap().layout.y,
    );
    assert_eq!((y_first, y_blk), (0.0, LINE), "line, then block, then line");
    assert_consistent(&doc, "chain split at the right level");
}

/// The #566 consequence, measured end to end: after a layout pass, inserting a
/// block **into a wrapper, before the text it holds** still inserts rather than
/// appending.
///
/// `C { block(7) <w>"two"</w> }` — a container that is mixed only because the
/// flattened scan can see the wrapper's text (#568), so on `main` no box was
/// minted here at all. Correct is `y = 7`: the new block goes above the text.
/// With the wrapper flattened into the run its text is adopted into a box, the
/// `position()` lookup in `insert_before` fails, the block is appended, and
/// `y = 27`.
///
/// Kills: flattening a contents wrapper unconditionally in `collect_run_units`.
#[test]
fn an_insert_before_a_wrapped_text_node_still_inserts() {
    fn build(mutate: bool) -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        el(&mut doc, c, "div", "height: 7px");
        let w = el(&mut doc, c, "span", "display: contents");
        if mutate {
            let two = txt(&mut doc, w, "two");
            doc.resolve_layout(VW, VH);
            let x = doc.create_element("div");
            doc.set_attribute(x, "style", "height: 40px");
            doc.insert_before(w, x, two);
            doc.resolve_layout(VW, VH);
            (doc, x)
        } else {
            let x = el(&mut doc, w, "div", "height: 40px");
            txt(&mut doc, w, "two");
            doc.resolve_layout(VW, VH);
            (doc, x)
        }
    }

    let (cdoc, cx) = build(false);
    let (mdoc, mx) = build(true);
    let expected = cdoc.tree.get(cx.0).unwrap().layout.y;
    assert_eq!(
        expected, 7.0,
        "control: the inserted block sits under the 7px one"
    );
    assert_eq!(
        mdoc.tree.get(mx.0).unwrap().layout.y,
        expected,
        "inserting before a node the author can still see must insert, not \
         append — {} means the reference was adopted out of the wrapper and \
         `insert_before` fell through to `push`",
        mdoc.tree.get(mx.0).unwrap().layout.y
    );
}

/// Two runs, one in the container and one inside the wrapper, **disturb
/// neither list** (#568, #566).
///
/// The old form checked that each box took the slot its run's head vacated and
/// that both lists were restored afterwards. No slot is vacated now, so the
/// interesting content is what remains: two runs are formed at the right
/// levels, and both DOM lists are the author's throughout.
#[test]
fn a_run_that_spans_the_container_and_a_wrapper_disturbs_neither() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let x = txt(&mut doc, c, "x ");
    let w = el(&mut doc, c, "span", "display: contents");
    let a = txt(&mut doc, w, "a ");
    let blk = el(&mut doc, w, "div", "height: 30px");
    let b = txt(&mut doc, w, "b ");
    txt(&mut doc, c, "y");
    doc.resolve_layout(VW, VH);

    let author_c = vec!["x ".to_string(), "<span>".to_string(), "y".to_string()];
    let author_w = vec!["a ".to_string(), "<div>".to_string(), "b ".to_string()];
    assert_eq!(
        dom_children(&doc, c),
        author_c,
        "the container is untouched"
    );
    assert_eq!(dom_children(&doc, w), author_w, "and so is the wrapper");

    // `x ` and `a ` are one run (the block inside the wrapper ends it); `b `
    // and `y` are the next. Both boxes belong to the container, because that
    // is whose flattened child list was classified.
    let first = doc
        .tree
        .get(x.0)
        .unwrap()
        .run_box
        .expect("`x ` is in a run");
    let second = doc
        .tree
        .get(b.0)
        .unwrap()
        .run_box
        .expect("`b ` is in a run");
    assert_ne!(first, second, "the block between them splits the runs");
    assert_eq!(doc.tree.get(a.0).unwrap().run_box, Some(first));
    assert_eq!(doc.tree.get(first).unwrap().parent, Some(c.0));
    assert_eq!(doc.tree.get(second).unwrap().parent, Some(c.0));

    doc.set_attribute(blk, "style", "display: none");
    doc.resolve_layout(VW, VH);

    assert_eq!(dom_children(&doc, c), author_c, "still the author's list");
    assert_eq!(dom_children(&doc, w), author_w, "still the author's list");
    assert_consistent(&doc, "after the dissolving pass");
}

/// Two runs inside one wrapper are restored **in document order**, with the
/// block that split them back between them.
///
/// **Honest about what this does and does not catch.** It is a round-trip
/// guard for the two-box shape, and it is *not* what pins the order the boxes
/// are dissolved in: reversing that loop leaves this — and the whole workspace
/// — green, measured, because every head is restored at the slot its own box
/// is holding *at the moment it is dissolved*, which no other box's dissolution
/// can move. The `.rev()` is kept as the exact inverse of creation and for the
/// index fallback, not because a case here needs it.
#[test]
fn two_runs_inside_one_wrapper_are_restored_in_document_order() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let w = el(&mut doc, c, "span", "display: contents");
    txt(&mut doc, w, "a ");
    let blk = el(&mut doc, w, "div", "height: 30px");
    txt(&mut doc, w, "b ");
    doc.resolve_layout(VW, VH);

    doc.set_attribute(blk, "style", "display: none");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        dom_children(&doc, w),
        vec!["a ".to_string(), "<div>".to_string(), "b ".to_string()],
        "the wrapper's children must come back in the order the author wrote"
    );
    assert_consistent(&doc, "two runs restored");
}

/// Two **adjacent** members of one run come back in the order they were
/// written.
///
/// This is where the sibling anchors have to be read *before* the run is
/// detached. Reading them while detaching gives the second member the same
/// surviving predecessor as the first — both "after the block", or both
/// "at the front" — and the restore puts them back reversed. The line then
/// reads `two one` on the next pass, which is a silent text scramble no
/// geometry assertion can see.
///
/// One member per run is the cardinality fixed point here: with one, "the
/// anchor before detaching" and "the anchor while detaching" are the same node.
///
/// Kills: collecting the anchors inside the removal loop.
#[test]
fn two_adjacent_members_of_one_run_come_back_in_order() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    txt(&mut doc, c, "one ");
    txt(&mut doc, c, "two");
    let blk = el(&mut doc, c, "div", "height: 7px");
    doc.resolve_layout(VW, VH);

    // The container stops being mixed, so this pass dissolves and mints
    // nothing — the restore is the only thing that decides the order.
    doc.set_attribute(blk, "style", "display: none");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        dom_children(&doc, c),
        vec!["one ".to_string(), "two".to_string(), "<div>".to_string()],
        "the two text nodes must come back in the order they were written"
    );
}

/// The same, for a run that follows a block — so the anchor is a real
/// preceding sibling rather than "the front of the list", and a mutant that
/// gets the head right by luck still has the second member to answer for.
#[test]
fn two_adjacent_members_of_a_later_run_come_back_in_order() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    txt(&mut doc, c, "head ");
    let blk = el(&mut doc, c, "div", "height: 7px");
    txt(&mut doc, c, "one ");
    txt(&mut doc, c, "two");
    doc.resolve_layout(VW, VH);

    doc.set_attribute(blk, "style", "display: none");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        dom_children(&doc, c),
        vec![
            "head ".to_string(),
            "<div>".to_string(),
            "one ".to_string(),
            "two".to_string()
        ],
        "the second run's members must come back in document order too"
    );
}

/// Two adjacent members behind a wrapper **keep their slots** (#568, #566).
///
/// The old form asserted they came back in order after being adopted out. The
/// order of a list nothing removes from cannot change, so the test now asserts
/// that directly — and additionally that both are members of the *same* box,
/// which is the fact the ordering was standing in for.
#[test]
fn two_adjacent_members_behind_a_wrapper_keep_their_slots() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    txt(&mut doc, c, "head ");
    let blk = el(&mut doc, c, "div", "height: 7px");
    let w = el(&mut doc, c, "span", "display: contents");
    let one = txt(&mut doc, w, "one ");
    let two = txt(&mut doc, w, "two");
    let inner = el(&mut doc, w, "div", "height: 5px");
    doc.resolve_layout(VW, VH);

    let author_w = vec!["one ".to_string(), "two".to_string(), "<div>".to_string()];
    assert_eq!(
        dom_children(&doc, w),
        author_w,
        "the author's list, untouched"
    );
    let b1 = doc
        .tree
        .get(one.0)
        .unwrap()
        .run_box
        .expect("`one ` is in a run");
    let b2 = doc
        .tree
        .get(two.0)
        .unwrap()
        .run_box
        .expect("`two` is in a run");
    assert_eq!(b1, b2, "adjacent inline siblings share one box");
    assert_eq!(
        doc.tree.get(b1).unwrap().run_members,
        vec![one.0, two.0],
        "and the box records them in document order"
    );

    // Both blocks go away, so nothing is mixed any more: this pass dissolves
    // and mints none.
    doc.set_attribute(blk, "style", "display: none");
    doc.set_attribute(inner, "style", "display: none");
    doc.resolve_layout(VW, VH);

    assert_eq!(dom_children(&doc, w), author_w, "still the author's list");
    assert_consistent(&doc, "after the dissolving pass");
}

/// A node inserted **before** an anonymous box, between two layout passes,
/// stays before the run that box stood for.
///
/// The restore is therefore anchored on siblings, not on the indices recorded
/// when the box was minted: an insertion anywhere ahead of a recorded slot
/// makes that index mean a different position, and the children come back
/// re-ordered — which then merges two runs into one and loses a line.
///
/// Kills: restoring an adopted child at its recorded index.
#[test]
fn a_node_inserted_before_a_box_does_not_reorder_the_restored_run() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    txt(&mut doc, c, "one ");
    el(&mut doc, c, "div", "height: 7px");
    txt(&mut doc, c, "two");
    doc.resolve_layout(VW, VH);
    let before = height_of(&doc, c);
    assert_eq!(
        before,
        LINE + 7.0 + LINE,
        "precondition: two lines and a block"
    );

    // An out-of-flow box, so it contributes no height of its own and any change
    // in the container's height is the runs being re-grouped.
    let x = doc.create_element("div");
    doc.set_attribute(
        x,
        "style",
        "position: absolute; left: 10px; top: 10px; width: 20px; height: 6px",
    );
    let first = NodeId(doc.tree.get(c.0).unwrap().children[0]);
    doc.insert_before(c, x, first);
    doc.resolve_layout(VW, VH);

    assert_eq!(
        height_of(&doc, c),
        before,
        "inserting an out-of-flow box must not change the line structure"
    );
    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, c), before, "and must stay that way");
    assert_consistent(&doc, "after an insert ahead of a box");
}

/// An **empty** wrapper as the container's *only* inline candidate mints no box.
///
/// `an_empty_wrapper_between_two_runs_does_not_split_them` above cannot see
/// this: there is real text on either side, so `has_inline` is already true and
/// whether the empty wrapper counts changes nothing. Here the wrapper is the
/// only thing that could make the container mixed, so "a wrapper with no inline
/// content is still inline content" produces an anonymous box around nothing —
/// and an extra line of height where a browser renders none.
///
/// Kills: dropping the inline-content requirement from `collect_run_units`'s
/// keep-whole test, which lets an empty wrapper survive as a `Contents` unit
/// and be counted by `has_inline`.
#[test]
fn an_empty_wrapper_alone_with_a_block_mints_no_box() {
    fn build(with_wrapper: bool) -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        if with_wrapper {
            el(&mut doc, c, "span", "display: contents");
        }
        el(&mut doc, c, "div", "height: 30px");
        doc.resolve_layout(VW, VH);
        (doc, c)
    }

    let (wdoc, wc) = build(true);
    let (pdoc, pc) = build(false);
    assert_eq!(
        height_of(&pdoc, pc),
        30.0,
        "control: with nothing but the block the container is exactly the block"
    );
    assert_eq!(
        height_of(&wdoc, wc),
        height_of(&pdoc, pc),
        "an empty boxless wrapper must add nothing — an anonymous box around no \
         content would add a line"
    );
    assert!(
        !dom_children(&wdoc, wc).contains(&"ANON".to_string()),
        "and no box may be minted at all: {:?}",
        dom_children(&wdoc, wc)
    );
    assert_consistent(&wdoc, "empty wrapper alone with a block");
}

/// The run's **head** is restored at the slot its own box is holding, not at the
/// slot the head's recorded sibling implies.
///
/// The two agree until something is inserted *between* them, which is what this
/// builds: a **block** inserted immediately before the first anonymous box. If
/// the head is restored by its `prev` anchor (`None` — it was first) it goes to
/// index 0 and the inserted block ends up *after* the first line instead of
/// above it. The container's height is identical either way, which is why
/// `a_node_inserted_before_a_box_does_not_reorder_the_restored_run` — whose
/// inserted node is out-of-flow — cannot see it: only the **order** changes.
///
/// Kills: removing the `i == 0 && adopted.origin == parent_id` head branch from
/// `cleanup_anonymous_block_boxes`.
#[test]
fn a_block_inserted_before_the_first_box_stays_above_the_line() {
    fn build(mutate: bool) -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        if mutate {
            txt(&mut doc, c, "one ");
            el(&mut doc, c, "div", "height: 7px");
            txt(&mut doc, c, "two");
            doc.resolve_layout(VW, VH);
            let x = doc.create_element("div");
            doc.set_attribute(x, "style", "height: 40px");
            let first = NodeId(doc.tree.get(c.0).unwrap().children[0]);
            doc.insert_before(c, x, first);
            doc.resolve_layout(VW, VH);
            (doc, x)
        } else {
            let x = el(&mut doc, c, "div", "height: 40px");
            txt(&mut doc, c, "one ");
            el(&mut doc, c, "div", "height: 7px");
            txt(&mut doc, c, "two");
            doc.resolve_layout(VW, VH);
            (doc, x)
        }
    }

    let (cdoc, cx) = build(false);
    let (mdoc, mx) = build(true);
    assert_eq!(
        cdoc.tree.get(cx.0).unwrap().layout.y,
        0.0,
        "control: written first, the block takes the top"
    );
    assert_eq!(
        mdoc.tree.get(mx.0).unwrap().layout.y,
        0.0,
        "inserted before the box that stands for the first run, it must still \
         take the top — y={} means the run's head was restored ahead of it",
        mdoc.tree.get(mx.0).unwrap().layout.y
    );
    assert_consistent(&mdoc, "block inserted before the first box");
}

// ── Controls: what must NOT change ────────────────────────────────────────

/// A flex container is skipped, so a mixed wrapper inside one mints nothing —
/// its children blockify into flex items instead (#41). This is the control
/// that stops "a contents wrapper always gets an anonymous box" being written
/// down.
#[test]
fn a_flex_parent_mints_no_box_for_a_mixed_wrapper() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "display: flex; flex-direction: column; width: 400px; line-height: 20px; \
         font-size: 16px",
    );
    let w = el(&mut doc, c, "span", "display: contents");
    let t1 = txt(&mut doc, w, "t1");
    let blk = el(&mut doc, w, "div", "height: 30px");
    doc.resolve_layout(VW, VH);

    assert_eq!(height_of(&doc, c), LINE + 30.0);
    assert!(
        !is_in_anonymous_root(&doc, t1),
        "a flex container's children are flex items, not an anonymous block box"
    );
    assert!(doc.tree.get(blk.0).unwrap().layout.height == 30.0);
    assert_consistent(&doc, "flex control");
}

/// An out-of-flow box behind a wrapper is still not block content (#406/#518),
/// so a container holding only that and text mints no box at all.
#[test]
fn a_wrapper_holding_only_an_absolute_still_makes_no_mixed_container() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let t = txt(&mut doc, c, "text");
    let w = el(&mut doc, c, "span", "display: contents");
    let abs = el(
        &mut doc,
        w,
        "div",
        "position: absolute; left: 10px; top: 10px; width: 40px; height: 20px",
    );
    doc.resolve_layout(VW, VH);

    assert!(
        !is_in_anonymous_root(&doc, t),
        "an out-of-flow box is not block content, so the container is not mixed"
    );
    let l = doc.tree.get(abs.0).unwrap().layout;
    assert_eq!((l.x, l.y, l.width, l.height), (10.0, 10.0, 40.0, 20.0));
    assert_consistent(&doc, "absolute behind a wrapper");
}

/// A wrapper holding an out-of-flow box beside its text is **flattened**, not
/// kept whole — so that box keeps the Taffy parent, and therefore the static
/// position, it has without the wrapper.
///
/// The oracle is the same markup with the wrapper deleted. An auto-inset
/// absolute takes its static position from where it sits in the flow, so
/// adopting the wrapper *whole* into the anonymous box would move that box
/// inside the run and shift the absolute — a boxless wrapper changing layout,
/// which is the one thing `display: contents` must never do.
///
/// (Both answers here are the engine's, not a browser's: rinch puts the
/// absolute after the anonymous box where Chromium puts it on the box's own
/// line. This fixture pins "the wrapper changed nothing", not "the number is
/// right".)
///
/// Kills: keeping a wrapper whole when its contribution includes an
/// out-of-flow box.
#[test]
fn a_wrapper_holding_an_absolute_beside_text_does_not_move_it() {
    fn build(wrapped: bool) -> (RinchDocument, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            "position: relative; width: 400px; line-height: 20px; font-size: 16px",
        );
        el(&mut doc, c, "div", "height: 13px");
        let host = if wrapped {
            el(&mut doc, c, "span", "display: contents")
        } else {
            c
        };
        txt(&mut doc, host, "MID");
        let abs = el(
            &mut doc,
            host,
            "div",
            "position: absolute; width: 20px; height: 6px",
        );
        txt(&mut doc, c, " after");
        el(&mut doc, c, "div", "height: 9px");
        doc.resolve_layout(VW, VH);
        (doc, c, abs)
    }

    let (wdoc, wc, wabs) = build(true);
    let (pdoc, pc, pabs) = build(false);

    let p = pdoc.tree.get(pabs.0).unwrap().layout;
    assert!(
        p.y > 0.0,
        "the control must not sit at the origin, or a moved box would pass: y={}",
        p.y
    );
    let w = wdoc.tree.get(wabs.0).unwrap().layout;
    assert_eq!(
        (w.x, w.y, w.width, w.height),
        (p.x, p.y, p.width, p.height),
        "a boxless wrapper must not move the absolute it holds"
    );
    assert_eq!(
        height_of(&wdoc, wc),
        height_of(&pdoc, pc),
        "nor the container's height"
    );
    assert_consistent(&wdoc, "absolute beside text in a wrapper");
}

/// A `display: none` child behind a wrapper generates no box either (#366), so
/// it neither makes a container mixed nor splits a run.
#[test]
fn a_hidden_child_behind_a_wrapper_neither_mixes_nor_splits() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let one = txt(&mut doc, c, "one ");
    let w = el(&mut doc, c, "span", "display: contents");
    el(&mut doc, w, "div", "display: none; height: 50px");
    let two = txt(&mut doc, c, "two");
    doc.resolve_layout(VW, VH);

    assert_eq!(height_of(&doc, c), LINE, "one line, nothing else");
    assert_eq!(ifc_root(&doc, one), ifc_root(&doc, two));
    assert!(
        !is_in_anonymous_root(&doc, one),
        "with no block content the container is not mixed and mints no box"
    );
    assert_consistent(&doc, "hidden child behind a wrapper");
}

/// Repeated passes are stable — the boxes are dissolved and re-minted every
/// `ifc_dirty` pass, so a restore that drifts by one shows up here and nowhere
/// else.
#[test]
fn the_flattened_classification_is_stable_across_repeated_passes() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    txt(&mut doc, c, "x ");
    let w = el(&mut doc, c, "span", "display: contents");
    txt(&mut doc, w, "a ");
    el(&mut doc, w, "div", "height: 30px");
    txt(&mut doc, w, "b ");
    txt(&mut doc, c, "y");
    doc.resolve_layout(VW, VH);

    let mut seen = Vec::new();
    for _ in 0..4 {
        doc.resolve_layout(VW, VH);
        seen.push(height_of(&doc, c));
        assert_consistent(&doc, "repeated pass");
    }
    assert!(
        seen.iter().all(|&x| x == LINE + 30.0 + LINE),
        "the container must keep the same two lines and block: {seen:?}"
    );
}

// ── The local pixel oracle ────────────────────────────────────────────────

/// Layout numbers say the line exists; only pixels say the text is drawn on
/// it. Each fixture paints text into a region where the correct output is
/// provably non-empty and the broken one is provably empty.
#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    /// Count of pixels that are not the white background, per horizontal band
    /// of `LINE` pixels — one entry per line of the container.
    fn ink_per_band(doc: &mut RinchDocument, bands: usize) -> Vec<usize> {
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
        let px = painter.pixels().as_chunks::<4>().0.to_vec();
        (0..bands)
            .map(|b| {
                let y0 = (b as f32 * LINE) as usize;
                let y1 = y0 + LINE as usize;
                (y0..y1)
                    .flat_map(|y| {
                        let row = y * VW as usize;
                        px[row..row + VW as usize].iter()
                    })
                    .filter(|p| p[0] < 200 && p[3] > 0)
                    .count()
            })
            .collect()
    }

    /// `before <w>MIDDLE</w> after` draws ink on the **first** band and none on
    /// the second. Broken: the wrapper's span is its own block on line 2, so
    /// band 1 is inked too.
    #[test]
    fn the_wrapped_text_is_drawn_on_the_same_line_as_its_neighbours() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        txt(&mut doc, c, "before ");
        let w = el(&mut doc, c, "span", "display: contents");
        txt(&mut doc, w, "MIDDLE");
        txt(&mut doc, c, " after");
        el(&mut doc, c, "div", "height: 20px");
        doc.resolve_layout(VW, VH);

        let bands = ink_per_band(&mut doc, 2);
        assert!(bands[0] > 0, "the line must be drawn: {bands:?}");
        assert_eq!(
            bands[1], 0,
            "nothing may be drawn on a second line — the wrapped text belongs \
             on the first: {bands:?}"
        );
    }

    /// Count of pixels that are recognisably red.
    fn red_ink(doc: &mut RinchDocument) -> usize {
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
            .filter(|p| p[0] > 150 && p[1] < 80 && p[2] < 80 && p[3] > 0)
            .count()
    }

    /// The anonymous box takes its inherited properties from **its own DOM
    /// parent**, which is the `display: contents` wrapper whenever the run was
    /// found behind one — so the wrapper's `color` reaches the text it wraps.
    ///
    /// A boxless element is still in the inheritance chain, so this is what CSS
    /// asks for; and the box is the IFC root, so its `color` is the one the run
    /// is drawn in. Taking the inherited set from the *container* instead
    /// draws the wrapper's text in the container's colour, which nothing else
    /// in the suite can see: the geometry is identical.
    ///
    /// Kills: `for_anonymous_box(&nodes[parent_id].computed_style)`.
    #[test]
    fn a_wrappers_own_colour_reaches_the_run_it_wraps() {
        fn build(on_wrapper: bool) -> RinchDocument {
            let mut doc = RinchDocument::new();
            let body = doc.body();
            let c = el(&mut doc, body, "div", CONTAINER);
            let w = el(
                &mut doc,
                c,
                "span",
                if on_wrapper {
                    "display: contents; color: red"
                } else {
                    "display: contents"
                },
            );
            txt(&mut doc, w, "t1 t2");
            el(&mut doc, w, "div", "height: 10px");
            doc.resolve_layout(VW, VH);
            doc
        }

        let mut red = build(true);
        let mut plain = build(false);
        assert_eq!(
            red_ink(&mut plain),
            0,
            "control: with no colour on the wrapper nothing is red"
        );
        assert!(
            red_ink(&mut red) > 0,
            "the wrapper's colour must reach the text it wraps: got {} red pixels",
            red_ink(&mut red)
        );
    }

    /// The bounding box of everything drawn — `(min_x, min_y, max_x, max_y)`,
    /// or `None` if nothing was drawn.
    fn ink_bbox(doc: &mut RinchDocument) -> Option<(usize, usize, usize, usize)> {
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
        let px = painter.pixels().as_chunks::<4>().0.to_vec();
        let w = VW as usize;
        let mut bb: Option<(usize, usize, usize, usize)> = None;
        for (i, p) in px.iter().enumerate() {
            if p[3] > 0 && (p[0] as u16 + p[1] as u16 + p[2] as u16) < 700 {
                let (x, y) = (i % w, i / w);
                bb = Some(match bb {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
        bb
    }

    /// `div { <w>inline…</w> block }` with the wrapper deleted as the oracle.
    ///
    /// This is the shape the `has_inline` widening changes: on `main` the
    /// container's raw child list holds no `Inline`, so it is **not** mixed and
    /// the wrapper's text is a bare Taffy text leaf; here the flattened scan
    /// sees through the wrapper, the container **is** mixed, and the text
    /// becomes inline content of an anonymous IFC root. Two different measure
    /// paths for the same markup.
    fn widened(wrapped: bool, container: &str, pieces: &[&str]) -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", container);
        let host = if wrapped {
            el(&mut doc, c, "span", "display: contents")
        } else {
            c
        };
        for piece in pieces {
            txt(&mut doc, host, piece);
        }
        el(&mut doc, c, "div", "height: 30px");
        doc.resolve_layout(VW, VH);
        (doc, c)
    }

    const WIDE: &str = "width: 400px; line-height: 20px; font-size: 16px";

    /// **Why these three fixtures exist.** Height cannot tell a bare Taffy text
    /// leaf from a one-line IFC root — both are one line tall, which is the
    /// fixed point that left the widening with *no* coverage at all (reverting
    /// it failed nothing in the shipped suite; found by an adversarial
    /// reviewer's mutant, and these are equivalents of the fixtures it wrote).
    /// What separates them is a property only an IFC has: collapsing whitespace
    /// **across a node boundary**, applying the container's `text-transform`,
    /// and aligning the line. Each compares against the same markup with the
    /// wrapper deleted.
    ///
    /// Two adjacent text nodes with collapsible whitespace at the join: an IFC
    /// collapses it to one space, two independent Taffy leaves cannot.
    #[test]
    fn the_widening_collapses_whitespace_across_a_node_boundary() {
        let (mut w, wc) = widened(true, WIDE, &["alpha   ", "   beta"]);
        let (mut p, pc) = widened(false, WIDE, &["alpha   ", "   beta"]);
        assert_eq!(height_of(&w, wc), height_of(&p, pc), "container height");
        assert_eq!(ink_bbox(&mut w), ink_bbox(&mut p), "rendered ink");
    }

    /// `text-transform` is inherited from the container and applied by the
    /// inline walk, so it reaches the wrapper's text only if that text is in
    /// the IFC.
    #[test]
    fn the_widening_applies_the_containers_text_transform() {
        const STYLE: &str =
            "width: 400px; line-height: 20px; font-size: 16px; text-transform: uppercase";
        let (mut w, _) = widened(true, STYLE, &["hello ", "world"]);
        let (mut p, _) = widened(false, STYLE, &["hello ", "world"]);
        assert_eq!(ink_bbox(&mut w), ink_bbox(&mut p), "uppercased ink");
    }

    /// `text-align: right` is a line-box property: only an IFC can honour it.
    #[test]
    fn the_widening_applies_text_align() {
        const STYLE: &str = "width: 400px; line-height: 20px; font-size: 16px; text-align: right";
        let (mut w, _) = widened(true, STYLE, &["aligned"]);
        let (mut p, _) = widened(false, STYLE, &["aligned"]);
        let (wb, pb) = (ink_bbox(&mut w), ink_bbox(&mut p));
        assert_eq!(wb, pb, "aligned ink");
        assert!(
            wb.is_some_and(|(x0, _, _, _)| x0 > 200),
            "control: the fixture must actually be right-aligned, or alignment \
             is not being tested at all: {wb:?}"
        );
    }

    /// A wrapper holding `t1 t2 block` draws both text nodes on band 0 and
    /// nothing on band 1. Broken: they stack as two blocks and band 1 is inked.
    #[test]
    fn a_mixed_wrappers_two_text_nodes_are_drawn_on_one_line() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(&mut doc, body, "div", CONTAINER);
        let w = el(&mut doc, c, "span", "display: contents");
        txt(&mut doc, w, "t1 ");
        txt(&mut doc, w, "t2");
        el(&mut doc, w, "div", "height: 30px");
        doc.resolve_layout(VW, VH);

        let bands = ink_per_band(&mut doc, 2);
        assert!(bands[0] > 0, "the line must be drawn: {bands:?}");
        assert_eq!(
            bands[1], 0,
            "the second text node belongs on the same line: {bands:?}"
        );
    }
}
