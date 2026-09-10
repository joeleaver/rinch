//! The anonymous block box is not in the DOM tree (#566).
//!
//! `create_anonymous_block_boxes` used to make the box a DOM node: it sat in
//! its container's `children` and the run's members were **reparented into
//! it**. So the box lied about the author's tree to everything that reads
//! `node.parent` or `node.children` — and CSS is unambiguous that it should
//! not, because an anonymous box is a *box*-tree construct (CSS 2.1 §9.2.1.1)
//! while inheritance and selector matching both operate on the **element**
//! tree.
//!
//! Three defects followed from that one fact, two of them live on `main`:
//!
//! - **#566** — `remove_child` / `insert_before` / `next_sibling` on a node in
//!   a run. `children.retain` was a no-op, `position()` failed and fell through
//!   to `push`. A deleted row stayed painted; an inserted row never rendered.
//! - **#579** — `+`, `~`, `:first-child`, `:nth-child` counted the box, so
//!   every element after a run shifted by one.
//! - the **inheritance** defect — an adopted element re-cascaded against the
//!   box rather than its real parent.
//!
//! All three share a trigger set that is worth stating once, because it is why
//! they hid: **the first layout pass is correct.** `resolve_styles` runs before
//! the boxes are minted, so nothing is wrong until something re-cascades or
//! re-reads the tree — `set_attribute` (any name), `set_style`, or a **window
//! resize**. And then it never heals.
//!
//! The box now has `parent: None`, sits in no `children` list, and the run's
//! members are never reparented; the box records `run_members` and each member
//! records its `run_box`.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

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

fn stylesheet(doc: &mut RinchDocument, css: &str) {
    let s = doc.create_element("style");
    let t = doc.create_text(css);
    doc.append_child(s, t);
    let body = doc.body();
    doc.append_child(body, s);
}

/// Is this element's background the red the fixture's rule sets?
fn is_red(doc: &RinchDocument, id: NodeId) -> bool {
    format!(
        "{:?}",
        doc.tree.get(id.0).unwrap().computed_style.background
    )
    .contains("[1.0, 0.0, 0.0, 1.0]")
}

fn marks(doc: &RinchDocument, ids: &[NodeId]) -> String {
    ids.iter()
        .map(|&i| if is_red(doc, i) { 'R' } else { '.' })
        .collect()
}

/// `div { "lead " <p/> <p/> <p/> }` — mixed content, so the leading text is
/// wrapped in an anonymous box — with the same markup minus the text as the
/// oracle. Returns `(doc, [p1, p2, p3])`.
fn build(css: &str, with_run: bool) -> (RinchDocument, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    stylesheet(&mut doc, css);
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 400px");
    if with_run {
        txt(&mut doc, c, "lead ");
    }
    let ps = vec![
        el(&mut doc, c, "p", "height: 10px"),
        el(&mut doc, c, "p", "height: 10px"),
        el(&mut doc, c, "p", "height: 10px"),
    ];
    doc.resolve_layout(VW, VH);
    (doc, ps)
}

// ── #579: selector matching counts the box ─────────────────────────────────

/// `:nth-child` and `:first-child` must answer the same with a run present as
/// without one.
///
/// **The re-cascade is the whole fixture.** On the first pass the answers are
/// already correct, because `resolve_styles` runs before the boxes exist — so a
/// version of this test without the `set_attribute` passes on `main` and pins
/// nothing. Poking each `<p>` invalidates its `stylo_element_data`, the next
/// pass re-cascades it with the box in the container's `children`, and every
/// index shifts by one.
///
/// Measured on `main` `334cf37`: `p:nth-child(2)` matches **p1** and
/// `p:first-child` matches **nothing**.
#[test]
fn selector_indices_are_unchanged_by_a_run() {
    for (name, css) in [
        ("p:nth-child(2)", "p:nth-child(2) { background: red }"),
        ("p:first-child", "p:first-child { background: red }"),
        ("p + p", "p + p { background: red }"),
        ("p:last-child", "p:last-child { background: red }"),
    ] {
        let (mut with_run, run_ps) = build(css, true);
        let (mut without, plain_ps) = build(css, false);
        // Invalidate every <p> so the next pass re-cascades it with the boxes
        // of the previous pass already in the tree.
        for (doc, ps) in [(&mut with_run, &run_ps), (&mut without, &plain_ps)] {
            for &p in ps.iter() {
                doc.set_attribute(p, "data-poke", "1");
            }
            doc.resolve_layout(VW, VH);
        }
        let got = marks(&with_run, &run_ps);
        let oracle = marks(&without, &plain_ps);
        assert_eq!(
            got, oracle,
            "{name}: an anonymous box must not shift element indices. \
             with a run = {got}, without = {oracle} (p1p2p3)"
        );
    }
}

// ── #566: the DOM stays the author's ───────────────────────────────────────

/// A run's members keep their real parent, and the container keeps them in its
/// `children`. This is the structural statement of the whole change, and the
/// one every other fixture here rests on.
#[test]
fn a_runs_members_keep_their_parent_and_their_slot() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "width: 400px; line-height: 20px; font-size: 16px",
    );
    let lead = txt(&mut doc, c, "lead ");
    let blk = el(&mut doc, c, "div", "height: 10px");
    let tail = txt(&mut doc, c, " tail");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        doc.tree.get(lead.0).unwrap().parent,
        Some(c.0),
        "the run's member must still be the container's child"
    );
    assert_eq!(
        doc.tree.get(tail.0).unwrap().parent,
        Some(c.0),
        "and so must the second run's"
    );
    assert_eq!(
        doc.tree.get(c.0).unwrap().children,
        vec![lead.0, blk.0, tail.0],
        "the container's children must be exactly what the author wrote"
    );
}

/// `remove_child` on a node inside a run actually removes it.
///
/// On `main` `children.retain(|&x| x != c)` was a no-op — the node was in the
/// **box's** children — and the next pass's cleanup restored it, so a deleted
/// row stayed on screen.
#[test]
fn removing_a_node_from_a_run_removes_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "width: 400px; line-height: 20px; font-size: 16px",
    );
    let lead = txt(&mut doc, c, "lead ");
    el(&mut doc, c, "div", "height: 10px");
    doc.resolve_layout(VW, VH);

    doc.remove_child(c, lead);
    doc.resolve_layout(VW, VH);

    assert!(
        !doc.tree.get(c.0).unwrap().children.contains(&lead.0),
        "the removed node must not be a child any more"
    );
    assert_eq!(
        doc.tree.get(c.0).unwrap().layout.height,
        10.0,
        "and its line must be gone from the container's height — a line left \
         here is the removed node still being laid out"
    );
}

/// `insert_before` a node inside a run inserts rather than appending.
#[test]
fn inserting_before_a_node_in_a_run_inserts() {
    fn build_it(mutate: bool) -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            "width: 400px; line-height: 20px; font-size: 16px",
        );
        el(&mut doc, c, "div", "height: 7px");
        if mutate {
            let t = txt(&mut doc, c, "two");
            doc.resolve_layout(VW, VH);
            let x = doc.create_element("div");
            doc.set_attribute(x, "style", "height: 40px");
            doc.insert_before(c, x, t);
            doc.resolve_layout(VW, VH);
            (doc, x)
        } else {
            let x = el(&mut doc, c, "div", "height: 40px");
            txt(&mut doc, c, "two");
            doc.resolve_layout(VW, VH);
            (doc, x)
        }
    }
    let (cdoc, cx) = build_it(false);
    let (mdoc, mx) = build_it(true);
    assert_eq!(cdoc.tree.get(cx.0).unwrap().layout.y, 7.0, "control");
    assert_eq!(
        mdoc.tree.get(mx.0).unwrap().layout.y,
        7.0,
        "inserted before a node in a run, the block must land above it"
    );
}

/// `DomDocument::next_sibling` answers with the author's next sibling, not the
/// box's. This is the public mechanism behind the `insert_after` failure.
#[test]
fn next_sibling_skips_no_one_and_invents_no_one() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "width: 400px; line-height: 20px; font-size: 16px",
    );
    let lead = txt(&mut doc, c, "lead ");
    let blk = el(&mut doc, c, "div", "height: 10px");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        doc.next_sibling(lead),
        Some(blk),
        "the text's next sibling is the block the author wrote after it"
    );
    assert_eq!(doc.next_sibling(blk), None, "and the block has none");
}

// ── the ancestor chain, which cross-parent adoption would break ────────────

/// A descendant-combinator rule keyed on an ancestor still matches through a
/// run. The box adds no link and removes none.
#[test]
fn a_descendant_rule_still_matches_through_a_run() {
    let mut doc = RinchDocument::new();
    stylesheet(&mut doc, ".outer .child { background: red }");
    let body = doc.body();
    let outer = el(&mut doc, body, "div", "width: 400px");
    doc.set_attribute(outer, "class", "outer");
    txt(&mut doc, outer, "lead ");
    let span = el(&mut doc, outer, "span", "");
    doc.set_attribute(span, "class", "child");
    txt(&mut doc, span, "x");
    el(&mut doc, outer, "div", "height: 10px");
    doc.resolve_layout(VW, VH);
    doc.set_attribute(span, "data-poke", "1");
    doc.resolve_layout(VW, VH);

    assert!(
        is_red(&doc, span),
        "an inline element inside a run must still match `.outer .child` — the \
         box must not remove a link from its ancestor chain"
    );
}

// ── #578: the DOM invariant the redesign makes checkable ───────────────────

/// `dom_tree_violations()` is clean across every shape in this file's family.
///
/// The invariant it checks — `parent` and `children` agreeing in both
/// directions — **could not be asserted before #566**, because the anonymous
/// box violated it on purpose: it reparented a run, so a member's `parent`
/// disagreed with the author's tree by design and the check would have fired
/// on legitimate state. Nothing is reparented now, so it is true of every node.
///
/// This is the repair for #578, whose finding was that
/// `taffy_tree_violations()` returned `[]` for both reconciler failures — it
/// compares the Taffy tree against itself, and those were DOM corruption with a
/// consistent Taffy tree underneath.
#[test]
fn the_dom_tree_invariant_holds_across_run_shapes() {
    type Shape = (&'static str, Box<dyn Fn(&mut RinchDocument, NodeId)>);
    let shapes: Vec<Shape> = vec![
        (
            "text + block",
            Box::new(|d: &mut RinchDocument, c: NodeId| {
                txt(d, c, "one ");
                el(d, c, "div", "height: 10px");
            }),
        ),
        (
            "text + block + text (two runs)",
            Box::new(|d: &mut RinchDocument, c: NodeId| {
                txt(d, c, "one ");
                el(d, c, "div", "height: 10px");
                txt(d, c, "two");
            }),
        ),
        (
            "run split by a comment",
            Box::new(|d: &mut RinchDocument, c: NodeId| {
                txt(d, c, "one ");
                let cm = d.create_comment("m");
                d.append_child(c, cm);
                txt(d, c, "two");
                el(d, c, "div", "height: 10px");
            }),
        ),
        (
            "run beside an absolute",
            Box::new(|d: &mut RinchDocument, c: NodeId| {
                txt(d, c, "one ");
                el(d, c, "div", "position: absolute; width: 5px; height: 5px");
                txt(d, c, "two");
                el(d, c, "div", "height: 10px");
            }),
        ),
        (
            "run behind a display:contents wrapper",
            Box::new(|d: &mut RinchDocument, c: NodeId| {
                let w = el(d, c, "span", "display: contents");
                txt(d, w, "one ");
                el(d, w, "div", "height: 10px");
            }),
        ),
    ];

    for (name, build_it) in shapes {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let c = el(
            &mut doc,
            body,
            "div",
            "width: 400px; line-height: 20px; font-size: 16px",
        );
        build_it(&mut doc, c);
        doc.resolve_layout(VW, VH);
        assert_eq!(
            doc.dom_tree_violations(),
            Vec::<String>::new(),
            "{name}: the DOM tree must be self-consistent after a layout pass"
        );
        // And across a second pass, which dissolves and re-mints the boxes.
        doc.resolve_layout(VW, VH);
        assert_eq!(
            doc.dom_tree_violations(),
            Vec::<String>::new(),
            "{name}: and after the boxes have been dropped and re-minted"
        );
    }
}

/// The validator actually fires on the shape it exists for: a node whose
/// `parent` disagrees with its parent's `children`, which is precisely what a
/// reparenting anonymous box produced and what `taffy_tree_violations()` could
/// not see.
#[test]
fn the_dom_tree_invariant_catches_a_one_way_link() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 400px");
    let child = el(&mut doc, c, "div", "height: 10px");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        doc.dom_tree_violations(),
        Vec::<String>::new(),
        "clean first"
    );

    // Reproduce #566's corruption directly: the child stops pointing at its
    // parent while the parent still lists it. This is what `remove_child`'s
    // `retain` left behind when the node had been adopted into a box.
    doc.tree.nodes[child.0].parent = None;
    let v = doc.dom_tree_violations();
    assert!(
        v.iter().any(|s| s.starts_with("A one-way")),
        "a one-way parent link must be reported, got {v:?}"
    );
}

// ── the upward chain: a member's coordinates run through its box ───────────

/// A run member's absolute position includes the **anonymous box's** own
/// offset, not just its container's.
///
/// A member's `layout` is assigned by the IFC **relative to the box**, so a
/// parent-chain sum that steps straight from the member to its DOM parent
/// drops the box's offset. That is invisible for a document's first run, whose
/// box sits at `y = 0` — which is why this fixture puts the run **after** a
/// block, so the box is at `y != 0` and the two answers differ by exactly the
/// box's `y`.
///
/// Kills: `box_tree_parent` ignoring `run_box`, and `box_tree_parent`
/// answering `None` for a box. Both were caught only by a focus test in the
/// `rinch` crate before this existed — a kill that lives in another crate is
/// one refactor away from being no kill at all.
#[test]
fn a_run_members_absolute_position_includes_its_box() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    // A spacer, so the **container** is not at `y = 0` either. Without it the
    // container contributes nothing to the sum and a walk that stops at the box
    // scores the same as one that continues past it — the second fixed point in
    // this fixture, and the one that let a `box_tree_parent` answering `None`
    // for a box survive its first mutant run.
    el(&mut doc, body, "div", "height: 25px");
    let c = el(
        &mut doc,
        body,
        "div",
        "width: 400px; line-height: 20px; font-size: 16px",
    );
    txt(&mut doc, c, "first ");
    el(&mut doc, c, "div", "height: 30px");
    // An inline-block: an `Inline` role, so a run member, and a real box whose
    // absolute position is worth asking for.
    let btn = el(&mut doc, c, "button", "width: 40px; height: 12px");
    doc.resolve_layout(VW, VH);

    let member = doc.tree.get(btn.0).unwrap();
    let box_id = member.run_box.expect("the inline-block is in a run");
    let box_y = doc.tree.get(box_id).unwrap().layout.y as f64;
    assert!(
        box_y > 40.0,
        "the fixture must put the second run's box well below the origin, or \
         the two answers agree and it pins nothing: box_y={box_y}"
    );

    let (_, abs_y) = rinch_dom::paint::compute_absolute_position(&doc.tree, btn.0, 1.0);
    let container_y = doc.tree.get(c.0).unwrap().layout.y as f64;
    assert!(
        container_y > 0.0,
        "the container must be off the origin too: container_y={container_y}"
    );
    let member_y = member.layout.y as f64;
    assert_eq!(
        abs_y,
        container_y + box_y + member_y,
        "the sum must pass through the box: container={container_y}, \
         box={box_y}, member={member_y}, got {abs_y}"
    );
}

/// Dropping a box clears its members' back-pointers.
///
/// A member left pointing at a dropped box names a **freed slab index**, and
/// `box_tree_children` would put that dead id in its container's box-tree
/// children — where the Taffy rebuild silently skips it (`nodes.get` returns
/// `None`) and the member's own box goes with it. Silent, and stable: the node
/// is laid out by nothing from then on.
///
/// The pass that proves it is one where the container stops being mixed, so no
/// box is minted to overwrite the stale pointer.
///
/// Kills: `cleanup_anonymous_block_boxes` not clearing `run_box`.
#[test]
fn dropping_a_box_clears_its_members_back_pointers() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        "width: 400px; line-height: 20px; font-size: 16px",
    );
    let t = txt(&mut doc, c, "text");
    let blk = el(&mut doc, c, "div", "height: 30px");
    doc.resolve_layout(VW, VH);
    assert!(
        doc.tree.get(t.0).unwrap().run_box.is_some(),
        "precondition: the text is in a run"
    );

    // No longer mixed: the boxes are dropped and none is minted.
    doc.set_attribute(blk, "style", "display: none");
    doc.resolve_layout(VW, VH);

    assert_eq!(
        doc.tree.get(t.0).unwrap().run_box,
        None,
        "a member of a dropped box must not still point at it"
    );
    assert_eq!(
        doc.tree.get(c.0).unwrap().layout.height,
        20.0,
        "and its line must still be laid out — a stale pointer names a freed \
         slab index and drops the member from the container's box tree"
    );
}
