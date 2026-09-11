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
//!    minting the boxes in reverse run order is observable, since a single-run
//!    `run_boxes` reads the same either way.
//!  - **Text-node cardinality.** One text node in a wrapper is where "one IFC
//!    line" and "one bare Taffy block" agree (both 1 line tall), so the
//!    symptom-2 fixture has two.
//!  - **Wrapper cardinality.** An empty wrapper is arity 0 for the flatten
//!    recursion and has its own fixture.
//!  - **Run-head depth.** A run whose head is a direct child of the container is
//!    where "the box hangs off the container" and "the box hangs off the head's
//!    own parent" agree, so the wrapper fixtures come in pairs: one with the
//!    head in the container, one with it behind the wrapper.
//!
//! **Nothing is reparented** (#566): an anonymous block box is not a DOM node,
//! and a run is *recorded* — `run_members` on the box, `run_box` on each member,
//! `run_boxes` on the container. So an assertion about a container's or a
//! wrapper's `children` is true whatever the classification decides, and cannot
//! stand in for a claim about the run. The fixtures that used to make their
//! claims that way now make them of the three fields above.
//!
//! # `adoption`, the mutant three fixtures below share
//!
//! Three fixtures mutate the DOM *between* two layout passes and check that the
//! mutation lands where the author aimed it. What they pin is the invariant
//! above, end to end — and the mutant of it is not a one-line edit, so it is
//! named once here and referred to by name in their `Kills:` lines.
//!
//! **`adoption` re-instates the two halves #566 deleted.** Minting takes every
//! run member out of its DOM parent's `children`, recording `(origin, index,
//! child)`; `cleanup_anonymous_block_boxes` re-inserts each at that recorded
//! index on the next pass. The window between the two is exactly where an app
//! mutates the DOM, which is what all three fixtures build. (Hold the records
//! on the tree, not in a `thread_local` — a fixture that builds two documents
//! on one thread otherwise restores one document's ids into the other and
//! panics inside Taffy instead of producing the divergence.)
//!
//! Measured at `db9c64f`, wide scope — `cargo test --no-fail-fast -p rinch-dom
//! -p rinch`, **43 test executables, 1257 tests**; the narrow `-p rinch-dom` is
//! 35 at that base and reports false survivors. (Both counts move: the wide
//! scope was 41 four commits earlier. A count quoted without its base goes
//! stale silently, so quote the base.) `adoption` has 68 killers, and each of
//! the three fails on the assertion that **distinguishes its two arms**, its
//! control arm still passing.
//!
//! **That last clause is the point, and it is why these three sat out the sweep
//! that repaired this file's hollow assertions.** All three are also killed by
//! deleting the trailing run flush — but through the **#466 leaf-invariant
//! production `assert!`**, not through anything they assert. Two of them are
//! killed by emitting every run box at the container's head instead of at its
//! own run head's slot — but on their **control** arm, which has no insertion
//! in it at all. A kill is not automatically evidence for the half of a fixture
//! you are reading; only `adoption` exercises the insert.

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

// ── The run bookkeeping, which is where the question moved ────────────────
//
// Since #566 nothing is reparented: an anonymous box is not a DOM node, so a
// container's and a wrapper's `children` are the author's list whatever the
// classification decides, and `dom_children` cannot tell an adopted run from a
// recorded one. The three fields below are what a run *is* now, so they are
// what the fixtures that used to read DOM parentage ask instead.

/// The anonymous box a node is a run member of — the member's own back-pointer.
fn run_box(doc: &RinchDocument, id: NodeId) -> Option<usize> {
    doc.tree.get(id.0).unwrap().run_box
}

/// A box's run, in the order the box recorded it.
fn run_members(doc: &RinchDocument, bx: usize) -> Vec<usize> {
    doc.tree.get(bx).unwrap().run_members.clone()
}

/// The boxes a container records — invariant A's downward half, and what
/// `box_tree_children` decides on in O(1).
fn run_boxes(doc: &RinchDocument, id: NodeId) -> Vec<usize> {
    doc.tree.get(id.0).unwrap().run_boxes.clone()
}

/// The container an anonymous box hangs off.
fn box_parent(doc: &RinchDocument, bx: usize) -> Option<usize> {
    doc.tree.get(bx).unwrap().parent
}

/// R: every member of a box is still a run unit of the container the box hangs
/// off. Asserted beside the fixtures' own claims rather than folded into
/// [`assert_consistent`], so a mutant's kill is attributable to one fixture's
/// own assertion rather than to a helper every test in the file calls.
fn assert_bookkeeping(doc: &RinchDocument, what: &str) {
    let v = doc.run_bookkeeping_violations();
    assert!(
        v.is_empty(),
        "{what}: run bookkeeping is inconsistent:\n  {}",
        v.join("\n  ")
    );
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
/// (the wrapper's span becomes its own block on a second line, +20px), and a
/// `collect_run_units` that does not reach a run member living behind a
/// wrapper — measured, both still kill it. (It used to say "dropping the
/// *adoption* of" that member; nothing is adopted since #566.)
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

// ── The run is recorded, not moved — and dissolving takes the record away ──

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

/// A wrapper that stands for an **entire** inline run contributes its *text* to
/// that run, never itself.
///
/// The keep-whole rule this fixture was written for is deleted (#573): a
/// `display: contents` element generates no box (CSS 2.1 §9.2.1.1), so
/// `collect_run_units` recurses into every one of them and never pushes one. A
/// run therefore takes the flattened boxes, and a wrapper is not a run unit **by
/// construction** — which is the fact this now pins.
///
/// It used to assert that in DOM parentage: `dom_children(w) == ["MIDDLE"]` and
/// `mid.parent == Some(w)`. Since #566 nothing is reparented under *any* rule,
/// so both were true whatever the classification decided — they could no longer
/// tell an adopted run from a recorded one. This is one of the fixtures whose
/// DOM-parentage half stopped witnessing the keep-whole mutants (`K10`/`K12`)
/// without anybody editing it.
///
/// Kills: `collect_run_units` pushing a `Contents` child instead of recursing
/// into it, in both its naive form (the wrapper becomes a unit the
/// classification counts as nothing, so the text joins no run at all) and its
/// faithful one (the wrapper becomes the run member in the text's place). The
/// faithful form is the interesting number: measured on `49e33ba` at
/// `-p rinch-dom -p rinch`, it went from 11 killers to 13, and this fixture and
/// the chain below are the two that were added — the fixture that used to be
/// here, and was written for exactly this rule, survived it. (A mutation count
/// is only evidence with its base named: #587 moved this one from 4 → 6 on
/// `4fe65ed` by deleting a redundant IFC-root path.)
///
/// **Its run head is a direct child of the container, which is a fixed point
/// for "whose box is it".** A mutant hanging the box off the head's own DOM
/// parent lands on `c` here and cannot be seen; the fixture below, whose head
/// is two levels down, is the one that samples off that point.
#[test]
fn a_wrapper_that_is_a_whole_run_contributes_its_text_not_itself() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let before = txt(&mut doc, c, "before ");
    let w = el(&mut doc, c, "span", "display: contents");
    let mid = txt(&mut doc, w, "MIDDLE");
    let after = txt(&mut doc, c, " after");
    el(&mut doc, c, "div", "height: 30px");
    doc.resolve_layout(VW, VH);

    let bx = run_box(&doc, mid).expect("the wrapper's text is in the run");
    assert_eq!(
        run_box(&doc, w),
        None,
        "and the wrapper is in no run — it generates no box to put in one"
    );
    assert_eq!(
        run_members(&doc, bx),
        vec![before.0, mid.0, after.0],
        "one run across the wrapper, and its middle member is the text itself"
    );
    assert_eq!(
        box_parent(&doc, bx),
        Some(c.0),
        "the box hangs off the container whose flattened list was classified, \
         not off the wrapper the member happens to live in"
    );
    assert_eq!(run_boxes(&doc, c), vec![bx], "and the container records it");
    assert!(
        run_boxes(&doc, w).is_empty(),
        "the boxless wrapper records none"
    );

    // The geometry half, which never went hollow: one line, then the block.
    assert_eq!(height_of(&doc, c), LINE + 30.0);
    assert!(
        is_in_anonymous_root(&doc, mid),
        "its text still flows into the run"
    );
    assert_bookkeeping(&doc, "wrapper flattened into the run");
    assert_consistent(&doc, "wrapper flattened into the run");
}

/// A **nested** wrapper chain flattens all the way to the text at its bottom:
/// the run takes neither wrapper, and the box still hangs off the container two
/// levels above.
///
/// One wrapper is the arity fixed point for "how far down does the chain
/// flatten" — with a chain of length 1, "recurse" and "take the child's own
/// children" agree. With two, a collector that descends a single level would
/// find `w2` in the unit list, and `w2` is a `Contents` unit the classification
/// counts as nothing: the text would join no run, while the container's height
/// and every `ifc_root` in this file stayed exactly as they are.
///
/// **The chain is written first on purpose**, so the run's head is the text two
/// levels down rather than a direct child of the container. That is the sample
/// off the fixed point the fixture above sits on: a mutant that hangs the box
/// off the run head's own DOM parent produces the container either way when the
/// head is a direct child, and produces `w2` here.
///
/// It used to say the opposite, and said it in an assertion that could no
/// longer be wrong. `dom_children(w1) == ["<span>"]` carried the message *"the
/// run should have taken `w1`, not reached past it for `w2`"* — the deleted
/// keep-whole rule. The run takes **neither** wrapper now; and since #566
/// nothing is reparented under any rule, so `w1` kept its child either way and
/// the assertion passed while its message was false. This was mutant `K10`'s
/// documented witness.
///
/// Kills: the deleted keep-whole rule in both forms (it makes `w1` the member);
/// a `collect_run_units` that recurses one level and then pushes; recording the
/// run backwards; hanging the box off the head's own parent; and grouping the
/// run out of `c.children`, where the chain is one `Contents` node and the text
/// is nowhere.
#[test]
fn a_nested_wrapper_chain_flattens_to_the_text_at_its_bottom() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let w1 = el(&mut doc, c, "span", "display: contents");
    let w2 = el(&mut doc, w1, "span", "display: contents");
    let mid = txt(&mut doc, w2, "MIDDLE");
    let after = txt(&mut doc, c, " after");
    el(&mut doc, c, "div", "height: 30px");
    doc.resolve_layout(VW, VH);

    let bx = run_box(&doc, mid).expect("the text at the bottom of the chain is the run member");
    assert_eq!(run_box(&doc, w1), None, "not the outer wrapper");
    assert_eq!(run_box(&doc, w2), None, "and not the inner one either");
    assert_eq!(
        run_members(&doc, bx),
        vec![mid.0, after.0],
        "one run, in document order, starting two levels down and coming back \
         out to the container's own text"
    );
    assert_eq!(
        box_parent(&doc, bx),
        Some(c.0),
        "the box hangs off the container that was classified — two levels above \
         its own head, which is the wrapper's child"
    );
    assert_eq!(run_boxes(&doc, c), vec![bx]);
    assert!(
        run_boxes(&doc, w1).is_empty() && run_boxes(&doc, w2).is_empty(),
        "and no wrapper in the chain records a box of its own"
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
    assert_bookkeeping(&doc, "nested chain flattened to the bottom");
    assert_consistent(&doc, "nested chain flattened to the bottom");
}

/// A two-deep chain whose outer wrapper also holds a block: **every level
/// flattens**, and the layout is the same as the un-wrapped shape.
///
/// This was the level-boundary case for the keep-whole rule — the test passed
/// at `w2` and failed at `w1`, in one chain — and it is kept because the
/// geometry half outlived the rule. Wrapper depth must make no difference to
/// where anything lands, which is the claim worth pinning whether or not a rule
/// decides levels differently.
#[test]
fn a_nested_wrapper_chain_flattens_at_every_level_and_lays_out_the_same() {
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

    // **Every wrapper is flattened, at every level.** The keep-whole rule this
    // fixture used to pin is gone: a run takes the boxes a wrapper stands for,
    // never the wrapper, because a `display: contents` element generates none
    // (CSS 2.1 §9.2.1.1). So the run's member here is the *text*, and neither
    // wrapper is a unit at all.
    //
    // Nothing is taken from anybody either way — that is #566 — so both
    // wrappers keep the author's list, and the run bookkeeping is where the
    // question is now asked.
    assert_eq!(
        dom_children(&doc, w2),
        vec!["MIDDLE".to_string()],
        "the inner wrapper keeps its text"
    );
    assert_eq!(
        dom_children(&doc, w1),
        vec!["<span>".to_string(), "<div>".to_string()],
        "and the outer keeps both its children"
    );
    assert!(
        doc.tree.get(mid.0).unwrap().run_box.is_some(),
        "the flattened text itself is the run member"
    );
    assert_eq!(
        doc.tree.get(w2.0).unwrap().run_box,
        None,
        "not the inner wrapper — it generates no box to put in a run"
    );
    assert_eq!(
        doc.tree.get(w1.0).unwrap().run_box,
        None,
        "and not the outer one"
    );
    // The geometry is the half of this fixture that outlived the rule, and it
    // is the load-bearing half: flattening at *any* depth must lay the chain
    // out exactly as the un-wrapped shape does.
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
/// flattened scan can see the wrapper's text (#568). Correct is `y = 7`: the
/// new block goes above the text, exactly as the control arm gets by writing it
/// there in the first place.
///
/// **`y = 27` is a mutant's number, not a shipped one.** The doc that stood
/// here read it as what *flattening the wrapper* produces and the `Kills:` line
/// named that flattening — which is the shipped behaviour, so it named no
/// mutant at all. Nothing is adopted out of a wrapper on any build in this
/// repo; #573 measured `y = 7` both with the keep-whole rule and without it,
/// which is one of the three grounds on which that rule was deleted.
///
/// What does produce 27 is `adoption` (module header): take `"two"` out of
/// `w.children` when its box is minted and `insert_before`'s `position()`
/// lookup fails one pass later, so the block is appended and the line it should
/// sit above ends up above it. The panic message below is therefore an accurate
/// diagnosis and is kept — measured, `adoption` is the only mutant of the
/// thirteen tried that reaches that assertion at all.
///
/// Kills: `adoption`. Fails at `left: 27.0, right: 7.0`, the control arm still
/// reading 7. Deleting the trailing run flush and classifying `children`
/// instead of the units fail it too, but both through the #466 production
/// `assert!`; emitting every box at the container's head fails its **control**
/// arm, which contains no insertion.
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

/// Two runs inside one wrapper mint two boxes, and both hang off the
/// **container** — in run order, and cleared together when the split heals.
///
/// "Restored in document order" was the old claim, and it described a round
/// trip that no longer happens: the boxes were never in `w.children`, nothing
/// was removed from that list, and the order of a list nothing removes from
/// cannot change. So `dom_children(w)` was true whatever the classification
/// decided.
///
/// What the round trip was standing in for is the two-box correspondence, and
/// that is asked of the bookkeeping instead: which boxes exist, whose they are,
/// which member is in which, and that dissolving takes all of it away. The
/// container is `c` and not `w` even though every member is `w`'s child —
/// `w` generates no box, so it can hold none.
///
/// Kills: hanging a box off the run head's own DOM parent (both boxes would be
/// the wrapper's); minting the boxes in reverse run order; a dissolving pass
/// that leaves `c.run_boxes` naming freed slab entries; one that leaves a
/// member's `run_box` naming a box that no longer exists; and dropping the
/// block-boundary flush, which makes this one run and one box rather than two.
#[test]
fn two_runs_inside_one_wrapper_are_two_boxes_on_the_container() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let w = el(&mut doc, c, "span", "display: contents");
    let a = txt(&mut doc, w, "a ");
    let blk = el(&mut doc, w, "div", "height: 30px");
    let b = txt(&mut doc, w, "b ");
    doc.resolve_layout(VW, VH);

    let b1 = run_box(&doc, a).expect("`a ` is in a run");
    let b2 = run_box(&doc, b).expect("`b ` is in a run");
    assert_ne!(b1, b2, "the block between them splits the runs");
    assert_eq!(run_members(&doc, b1), vec![a.0]);
    assert_eq!(run_members(&doc, b2), vec![b.0]);
    assert_eq!(
        (box_parent(&doc, b1), box_parent(&doc, b2)),
        (Some(c.0), Some(c.0)),
        "both boxes hang off the container — the wrapper generates no box, so \
         it can hold none"
    );
    assert_eq!(
        run_boxes(&doc, c),
        vec![b1, b2],
        "and the container records both, in run order"
    );
    assert!(run_boxes(&doc, w).is_empty(), "the wrapper records neither");
    assert_eq!(
        height_of(&doc, c),
        LINE + 30.0 + LINE,
        "line, block, line — the shape the two boxes stand for"
    );
    assert_bookkeeping(&doc, "two runs inside one wrapper");

    // Heal the split: with the block gone the container is no longer mixed, so
    // this pass dissolves both boxes and mints none.
    doc.set_attribute(blk, "style", "display: none");
    doc.resolve_layout(VW, VH);

    assert_eq!(run_box(&doc, a), None, "the run is gone");
    assert_eq!(run_box(&doc, b), None, "and so is the second");
    assert!(
        run_boxes(&doc, c).is_empty(),
        "and the container records no box — a list left naming freed slab \
         entries is what invariant C fires on, and what sends \
         `box_tree_children` down the substituting path with nothing to \
         substitute"
    );
    assert_eq!(
        height_of(&doc, c),
        LINE,
        "`a b` is one line once nothing splits it"
    );
    assert_bookkeeping(&doc, "after the dissolving pass");
    assert_consistent(&doc, "after the dissolving pass");
}

/// Two **adjacent** members of a run the *block boundary* flushes are recorded
/// in the order they were written.
///
/// This used to be about sibling anchors: the run was adopted out of the
/// container, and the anchors had to be read *before* the detach or the two
/// members came back reversed and the line read `two one`. Since #566 nothing
/// is detached, so `dom_children(c)` after the dissolving pass was the author's
/// list whatever anything did — the order of a list nothing removes from cannot
/// change. The ordering claim is real, but it now lives in `run_members`, which
/// is the only place a run has an order at all.
///
/// This is the un-wrapped, run-at-the-head sample: two adjacent text nodes and
/// nothing between them and the start of the container.
///
/// One member per run is still the cardinality fixed point — with one member,
/// forwards and backwards are the same list.
///
/// Kills: recording the run backwards, and both halves of the dissolving pass —
/// a cleanup that leaves `c.run_boxes` naming a freed box, and one that leaves a
/// member's `run_box` naming it.
///
/// **What it cannot see, stated rather than implied.** Its container holds
/// exactly one run, so *both* flush sites produce it: deleting the
/// `InFlowBlock` arm's flush leaves `one two` as a trailing run, and deleting
/// the trailing flush leaves it as a block-flushed one. Measured, both mutants
/// survive this fixture and are killed by its pair below, whose container has a
/// run at each site. Its kill set is therefore a **subset** of that pair's; it
/// is kept as the second sample rather than deleted, and this paragraph is here
/// so nobody reads it as independent coverage.
#[test]
fn two_adjacent_members_of_one_run_are_recorded_in_order() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let one = txt(&mut doc, c, "one ");
    let two = txt(&mut doc, c, "two");
    let blk = el(&mut doc, c, "div", "height: 7px");
    doc.resolve_layout(VW, VH);

    let bx = run_box(&doc, one).expect("`one ` is in a run");
    assert_eq!(
        run_box(&doc, two),
        Some(bx),
        "adjacent inline siblings share one box"
    );
    assert_eq!(
        run_members(&doc, bx),
        vec![one.0, two.0],
        "and the box records them in the order they were written"
    );
    assert_eq!(box_parent(&doc, bx), Some(c.0));
    assert_eq!(
        run_boxes(&doc, c),
        vec![bx],
        "one run, flushed by the block after it, so exactly one box"
    );
    assert_eq!(height_of(&doc, c), LINE + 7.0, "one line, then the block");
    assert_bookkeeping(&doc, "a run flushed by a block");

    // The container stops being mixed, so this pass dissolves and mints
    // nothing.
    doc.set_attribute(blk, "style", "display: none");
    doc.resolve_layout(VW, VH);

    assert_eq!(run_box(&doc, one), None, "the run is gone");
    assert_eq!(run_box(&doc, two), None);
    assert!(run_boxes(&doc, c).is_empty());
    assert_eq!(height_of(&doc, c), LINE, "and `one two` is the whole box");
    assert_bookkeeping(&doc, "after the dissolving pass");
    assert_consistent(&doc, "after the dissolving pass");
}

/// The same two members, in the run the **trailing** flush produces — the run
/// that ends because the child list does, with a block ahead of it rather than
/// behind it.
///
/// The two flush sites are separately deletable, and one run per container
/// cannot tell them apart — which is the whole reason this fixture has two.
/// Dropping the `if !current_run.is_empty()` after the grouping loop mints no
/// box at all for `one two`; dropping the `InFlowBlock` arm's flush merges
/// `head one two` into a single run across the block. Both are measured, and
/// both survive the fixture above, whose one run either site would produce.
///
/// Kills: dropping the trailing flush; dropping the block flush; recording a
/// run backwards; minting the boxes in reverse run order (`c.run_boxes` names
/// the second run's box first); and both halves of the dissolving pass.
///
/// The reverse-mint-order one is worth a number, because it is a claim about
/// the whole workspace rather than about this file: measured on `e8fb6e8` at
/// `-p rinch-dom -p rinch` (41 executables, 1247 tests), that mutant has
/// **zero** killers with this fixture and its sibling reverted, and exactly
/// those two with them. It changes no pixel and no geometry —
/// `box_tree_children` emits each box at its first member, so the Taffy order
/// is unaffected — and `run_boxes` is the only thing that reads differently.
#[test]
fn two_adjacent_members_of_a_later_run_are_recorded_in_order() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", CONTAINER);
    let head = txt(&mut doc, c, "head ");
    let blk = el(&mut doc, c, "div", "height: 7px");
    let one = txt(&mut doc, c, "one ");
    let two = txt(&mut doc, c, "two");
    doc.resolve_layout(VW, VH);

    let first = run_box(&doc, head).expect("`head ` is in a run");
    let second = run_box(&doc, one).expect("`one ` is in a run");
    assert_ne!(first, second, "the block between them splits the runs");
    assert_eq!(run_members(&doc, first), vec![head.0]);
    assert_eq!(
        run_members(&doc, second),
        vec![one.0, two.0],
        "the trailing run records both its members, in document order"
    );
    assert_eq!(run_box(&doc, two), Some(second));
    assert_eq!(
        run_boxes(&doc, c),
        vec![first, second],
        "two runs, one per flush site, recorded in run order"
    );
    assert_eq!(
        height_of(&doc, c),
        LINE + 7.0 + LINE,
        "line, block, line — the trailing run has a box of its own"
    );
    assert_bookkeeping(&doc, "a run flushed by the end of the list");

    doc.set_attribute(blk, "style", "display: none");
    doc.resolve_layout(VW, VH);

    assert_eq!(run_box(&doc, head), None, "both runs are gone");
    assert_eq!(run_box(&doc, one), None);
    assert_eq!(run_box(&doc, two), None);
    assert!(run_boxes(&doc, c).is_empty());
    assert_eq!(height_of(&doc, c), LINE, "and the three join one line");
    assert_bookkeeping(&doc, "after the dissolving pass");
    assert_consistent(&doc, "after the dissolving pass");
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
/// **There is no restore path to anchor on anything.** The doc that stood here
/// described one anchored on siblings rather than on recorded indices; nothing
/// is moved (#566), so nothing comes back, and the old `Kills:` line —
/// "restoring an adopted child at its recorded index" — named a mechanism this
/// crate does not have. What the fixture pins is the invariant that makes the
/// restore unnecessary: the box minted for `"one "` is in nobody's `children`,
/// so `children[0]` is still the author's own text and an insert ahead of it
/// lands where the author aimed.
///
/// That recorded-index restore is now the **mutant** instead of the code. Under
/// `adoption` (module header) both runs are re-inserted at the indices they
/// held when they were taken — which the insert has since made mean something
/// else. Probed rather than reasoned: `c.children` reads `[x, "one ", block,
/// "two"]` unmutated and `[x, block]` under the mutant at the same point, the
/// two texts having been restored as `["one ", "two", x, block]` at the top of
/// the pass and adopted straight back out. They come back **adjacent**, the
/// block no longer separates them, the two runs merge into one, and the
/// container loses a line.
///
/// Kills: `adoption`. Fails at the post-insert assertion, `left: 27.0, right:
/// 47.0`, with the precondition still passing.
///
/// **What its kill list does not establish.** Deleting the run flush at a block
/// boundary fails this fixture at its *precondition* — before the insert has
/// happened — and deleting the trailing flush fails it through the #466
/// production `assert!`. Neither is evidence for the half this fixture exists
/// for; `adoption` is the one that reaches it.
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
/// **The mechanism that decides it today**, since the keep-whole test the old
/// `Kills:` line named no longer exists: `collect_run_units` recurses into the
/// wrapper and it contributes **zero** units, so the container's unit list is
/// just the block, `has_inline` is false, and `!(has_inline && has_block)`
/// skips the container before any run is grouped.
///
/// Kills: reviving the keep-whole rule *without* its inline-content
/// requirement — push a wrapper whole when everything it stands for is inline,
/// which an empty wrapper satisfies vacuously, then count a `Contents` unit in
/// `has_inline` and let it join a run. That mints a box around nothing.
///
/// **Its own hollow half is repaired here too.** It asserted no box by looking
/// for `"ANON"` in `dom_children`, and since #566 a box is not in anybody's
/// `children` — so that read could not find one however many were minted.
/// Measured: with the box minted around an empty wrapper, the height oracle
/// above cannot see it either (an IFC over no content measures 0), so
/// `run_boxes` is the only thing in this fixture that can.
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
        run_boxes(&wdoc, wc).is_empty(),
        "and no box may be minted at all — an empty wrapper contributes no \
         unit, so the container is not mixed content: {:?}",
        run_boxes(&wdoc, wc)
    );
    assert_bookkeeping(&wdoc, "empty wrapper alone with a block");
    assert_consistent(&wdoc, "empty wrapper alone with a block");
}

/// The run's **head** stays where the author left it, so a block inserted
/// immediately before it still takes the top of the container.
///
/// **The machinery the old doc described is gone.** It weighed restoring the
/// head "at the slot its own box is holding" against "the slot its recorded
/// sibling implies", and `Kills:` named an `i == 0 && adopted.origin ==
/// parent_id` head branch in `cleanup_anonymous_block_boxes`. All of it went
/// with `AdoptedChild`: no restore, no recorded slot, no such branch, and no
/// `prev` anchor to be wrong about.
///
/// The *shape* survives and still discriminates, because `adoption` (module
/// header) puts a restore back. The head is re-inserted at the index it held
/// before the insert, lands ahead of the newly inserted block, and the block
/// drops below the first line. Its `y` is the whole test: the container's
/// height is the same either way, so only the **order** moves — which is why
/// this is a separate fixture from
/// `a_node_inserted_before_a_box_does_not_reorder_the_restored_run`, whose
/// out-of-flow insert makes its signal a height instead.
///
/// Kills: `adoption`. Fails at the mutate arm's assertion, `y = 20` where 0 is
/// correct, with the control arm still reading 0. Deleting the trailing run
/// flush fails it through the #466 production `assert!`, and emitting every box
/// at the container's head fails its **control** arm; neither reaches the
/// insert.
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

/// Repeated passes are stable — the boxes are dropped and re-minted every
/// `ifc_dirty` pass, so a classification that is not a function of the DOM alone
/// shows up here.
///
/// **Not "a restore that drifts by one shows up here and nowhere else"**, which
/// is what this doc used to say. Nothing is restored (#566), and the drift it
/// named needs a DOM mutated *between* passes — which this fixture never does.
/// Measured: of the four fixtures whose docs this commit repairs, this is the
/// only one `adoption` (module header) does **not** kill, precisely because it
/// gives a restore nothing to drift against. "Nowhere else" was false twice
/// over: the mutants it does catch are caught by other fixtures too.
///
/// Kills: deleting the run flush in the `InFlowBlock` arm — the two runs merge
/// and all four passes read a stable, wrong `50` where the container is `LINE +
/// 30 + LINE`; and `box_tree_children` reading `children` instead of
/// `collect_run_units`' output, which reads `110` for all four. Deleting the
/// trailing flush, classifying `children` in the *grouping*, and never recursing
/// in `collect_run_units` fail it too, but all three through the #466 production
/// `assert!` rather than through anything it asserts.
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
