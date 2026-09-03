//! #517 — every DOM mutation path that detaches a node from a parent must
//! remove the node's *Taffy contribution*, which for a `display: contents`
//! node is not its own `taffy_id`.
//!
//! `sync_display_contents` polyfills `display: contents` by hiding the
//! wrapper's own Taffy node and splicing its children's Taffy ids directly
//! into the parent's Taffy child list. After the first layout pass the
//! wrapper's own id is therefore not in the parent's list — detaching it is
//! a silent no-op (`taffy_remove_child_safe` swallows it), and the spliced
//! children stay behind as invisible siblings claiming layout space forever.
//! `sync_display_contents` cannot heal the parent afterwards: it only
//! rebuilds parents that *still* have a contents descendant.
//!
//! PR #515 fixed `remove_node` (the reactive runtime's unmount path). This
//! suite covers the sibling entry points — `remove_child`, `replace_node`
//! (both its legs), `set_text_content`'s child-clearing, `set_inner_html`'s
//! child-clearing, and the reparent legs of `append_child`, `insert_before`
//! and `insert_child` — and deliberately does NOT touch `remove_node`, so
//! this branch and #515 stay disjoint.
//!
//! Fixture discipline (learned the hard way in the #515 review):
//! - wrappers carry TWO children or are NESTED — a one-child wrapper is the
//!   identity value where "remove the set" and "remove the first" agree, so
//!   a `.take(1)` mutant survives it;
//! - every fixed path also has a PLAIN-node twin asserting sibling geometry
//!   after an ordinary removal — without it, a mutant that treats *every*
//!   detach as a contents detach (dropping the own-id removal) is invisible
//!   to CI; that mutant survived the entire pre-existing rinch-dom suite.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn child_of(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let el = doc.create_element("div");
    doc.set_attribute(el, "style", style);
    doc.append_child(parent, el);
    el
}

fn height_of(doc: &RinchDocument, node: NodeId) -> f32 {
    doc.tree.get(node.0).unwrap().layout.height
}

fn flex_column(doc: &mut RinchDocument, height: f32) -> NodeId {
    let body = doc.body();
    child_of(
        doc,
        body,
        &format!("display: flex; flex-direction: column; height: {height}px;"),
    )
}

fn taffy_child_count(doc: &RinchDocument, node: NodeId) -> usize {
    let taffy_id = doc.tree.get(node.0).unwrap().taffy_id.unwrap();
    doc.tree.taffy.children(taffy_id).unwrap().len()
}

// ---------------------------------------------------------------------------
// remove_child
// ---------------------------------------------------------------------------

/// The #517 bug on `remove_child`, with a two-child wrapper.
/// Kills: the missing contents handling itself, and the `.take(1)` mutant
/// (150 here = one phantom left behind, 100 = both).
#[test]
fn remove_child_frees_both_spliced_slots() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let wrapper = child_of(&mut doc, col, "display: contents");
    let a1 = child_of(&mut doc, wrapper, "flex: 1;");
    let a2 = child_of(&mut doc, wrapper, "flex: 1;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, a1), 100.0, "sanity: three-way split");
    assert_eq!(height_of(&doc, a2), 100.0, "sanity: three-way split");
    assert_eq!(height_of(&doc, b), 100.0, "sanity: three-way split");

    doc.remove_child(col, wrapper);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b),
        300.0,
        "both spliced children must be freed; 150 = one phantom left, 100 = two"
    );
}

/// Nested contents wrappers removed via `remove_child`: grandchildren
/// flattened through the inner wrapper must be freed too.
/// Kills: a non-recursive flatten on the removal path.
#[test]
fn remove_child_nested_wrappers_flatten_recursively() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 400.0);

    let w1 = child_of(&mut doc, col, "display: contents");
    let w2 = child_of(&mut doc, w1, "display: contents");
    let a1 = child_of(&mut doc, w2, "flex: 1;");
    let a2 = child_of(&mut doc, w2, "flex: 1;");
    let a3 = child_of(&mut doc, w1, "flex: 1;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    for (n, name) in [(a1, "a1"), (a2, "a2"), (a3, "a3"), (b, "b")] {
        assert_eq!(height_of(&doc, n), 100.0, "sanity: four-way split ({name})");
    }

    doc.remove_child(col, w1);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b),
        400.0,
        "grandchildren through the nested wrapper must be freed too"
    );
}

/// Removing one wrapper must not disturb a sibling wrapper's splice.
/// Kills: over-removal — a detach that strips the parent's whole effective
/// child set instead of only the removed node's contribution.
#[test]
fn remove_child_leaves_sibling_wrapper_spliced() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let w1 = child_of(&mut doc, col, "display: contents");
    let a = child_of(&mut doc, w1, "flex: 1;");
    let w2 = child_of(&mut doc, col, "display: contents");
    let c = child_of(&mut doc, w2, "flex: 1;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, a), 100.0, "sanity");
    assert_eq!(height_of(&doc, c), 100.0, "sanity");

    doc.remove_child(col, w1);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, c),
        150.0,
        "the survivor wrapper's child grows"
    );
    assert_eq!(height_of(&doc, b), 150.0, "the plain survivor grows");
}

/// BLIND-SPOT twin: removing a PLAIN sized node via `remove_child` must free
/// the node's own Taffy slot and let the sibling claim the space.
/// Kills: dropping the own-id removal / treating every detach as a contents
/// detach — the mutant that survived the entire pre-existing suite.
#[test]
fn remove_child_plain_node_frees_its_own_slot() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 200.0);

    let a = child_of(&mut doc, col, "flex: 1;");
    let _a_inner = child_of(&mut doc, a, "height: 10px;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b), 100.0, "sanity: even split");

    doc.remove_child(col, a);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b),
        200.0,
        "a plain removed node's own Taffy slot must be freed"
    );
}

/// Removing the wrapper BEFORE any layout pass: styles are not yet resolved,
/// no splice has happened, and the wrapper's own Taffy id genuinely is the
/// parent's child — the plain-node leg must detach it.
/// Guards the pre-first-layout window (the `else`/own-id half of the fix).
#[test]
fn remove_child_pre_layout_detaches_the_wrapper_itself() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 200.0);

    let wrapper = child_of(&mut doc, col, "display: contents");
    let _a = child_of(&mut doc, wrapper, "flex: 1;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.remove_child(col, wrapper);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b),
        200.0,
        "no phantom from pre-layout removal"
    );
}

/// Full navigation cycle via `remove_child`: away from the wrapped route,
/// then a fresh mount. The newcomer and the survivor split evenly — no ghost
/// left behind, and nothing over-removed either.
#[test]
fn remove_child_then_mount_new_content_splits_evenly() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 200.0);

    let wrapper = child_of(&mut doc, col, "display: contents");
    let _a = child_of(&mut doc, wrapper, "flex: 1;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    doc.remove_child(col, wrapper);
    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b), 200.0, "ghost gone after removal");

    let newcomer = child_of(&mut doc, col, "flex: 1;");
    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b), 100.0, "even split with the newcomer");
    assert_eq!(
        height_of(&doc, newcomer),
        100.0,
        "the newcomer gets its half"
    );
}

// ---------------------------------------------------------------------------
// replace_node — both legs
// ---------------------------------------------------------------------------

/// The #517 bug on `replace_node`'s removal leg, two-child wrapper.
/// Kills: missing contents handling when detaching `old`, and `.take(1)`.
#[test]
fn replace_node_frees_both_spliced_slots() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let wrapper = child_of(&mut doc, col, "display: contents");
    let _a1 = child_of(&mut doc, wrapper, "flex: 1;");
    let _a2 = child_of(&mut doc, wrapper, "flex: 1;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b), 100.0, "sanity: three-way split");

    let replacement = doc.create_element("div");
    doc.set_attribute(replacement, "style", "flex: 1;");
    doc.replace_node(wrapper, replacement);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b),
        150.0,
        "b splits the column with the replacement alone; 100 or 75 = phantoms"
    );
    assert_eq!(
        height_of(&doc, replacement),
        150.0,
        "the replacement gets its half"
    );
}

/// `replace_node`'s OTHER leg: `new` is itself a spliced wrapper being moved
/// in from another parent. Its old parent must not keep the spliced slots.
/// (Passed pre-fix via the `set_children` self-heal — see the reparent-leg
/// note below; guards the detach-`new`-from-old-parent leg against
/// over-removal.)
#[test]
fn replace_node_detaches_moved_wrappers_old_slots() {
    let mut doc = RinchDocument::new();
    let col_a = flex_column(&mut doc, 200.0);
    let col_b = flex_column(&mut doc, 200.0);

    let wrapper = child_of(&mut doc, col_a, "display: contents");
    let a1 = child_of(&mut doc, wrapper, "height: 50px;");
    let a2 = child_of(&mut doc, wrapper, "height: 50px;");
    let b_a = child_of(&mut doc, col_a, "flex: 1;");

    let target = child_of(&mut doc, col_b, "flex: 1;");
    let b_b = child_of(&mut doc, col_b, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b_a), 100.0, "sanity: colA = 50+50+flex");
    assert_eq!(height_of(&doc, b_b), 100.0, "sanity: colB even split");

    // Move the wrapper across parents by replacing colB's first child with it.
    doc.replace_node(target, wrapper);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b_a),
        200.0,
        "colA must give the survivor everything; 100 = phantoms stayed behind"
    );
    assert_eq!(
        height_of(&doc, a1),
        50.0,
        "the moved wrapper splices into colB"
    );
    assert_eq!(
        height_of(&doc, a2),
        50.0,
        "the moved wrapper splices into colB"
    );
    assert_eq!(height_of(&doc, b_b), 100.0, "colB: 50+50+flex leaves 100");
}

/// BLIND-SPOT twin for `replace_node`: replacing a PLAIN node must free the
/// node's own slot (its inner child makes the contents-only mutant's
/// effective-set removal a detectable no-op).
#[test]
fn replace_node_plain_node_frees_its_own_slot() {
    let mut doc = RinchDocument::new();
    let col = flex_column(&mut doc, 300.0);

    let a = child_of(&mut doc, col, "flex: 1;");
    let _a_inner = child_of(&mut doc, a, "height: 10px;");
    let b = child_of(&mut doc, col, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b), 150.0, "sanity: even split");

    let replacement = doc.create_element("div");
    doc.set_attribute(replacement, "style", "flex: 1;");
    doc.replace_node(a, replacement);
    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b), 150.0, "b keeps exactly half");
    assert_eq!(
        height_of(&doc, replacement),
        150.0,
        "the replacement claims the replaced node's whole slot; 100 = phantom"
    );
}

// ---------------------------------------------------------------------------
// set_text_content — child-clearing
// ---------------------------------------------------------------------------

/// The #517 bug on `set_text_content`'s child-clearing: replacing a parent's
/// children with text must free the slots its contents wrapper had spliced in.
/// Kills: missing contents handling in the clearing loop, and `.take(1)`
/// (the two 50px phantoms shrink one at a time).
///
/// A healthy post-state has ZERO Taffy children: `p` becomes an IFC root for
/// its new text child, and `setup_inline_formatting_contexts` detaches the
/// text leaf so the root's measure function is reachable. That is also why,
/// pre-fix, this test dies inside `resolve_layout` on the #466 leaf
/// invariant instead of reaching the asserts — the phantoms make the
/// InlineRoot a non-leaf carrier.
#[test]
fn set_text_content_clears_spliced_slots() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let p = child_of(&mut doc, body, "");

    let wrapper = child_of(&mut doc, p, "display: contents");
    let _a1 = child_of(&mut doc, wrapper, "height: 50px;");
    let _a2 = child_of(&mut doc, wrapper, "height: 50px;");

    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, p),
        100.0,
        "sanity: two spliced 50px children"
    );

    doc.set_text_content(p, "");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        taffy_child_count(&doc, p),
        0,
        "the spliced slots must be freed (the text leaf itself is detached \
         by IFC setup, so a healthy list is empty)"
    );
    assert!(
        height_of(&doc, p) < 50.0,
        "the spliced 50px children must be gone, got {}",
        height_of(&doc, p)
    );
}

/// BLIND-SPOT twin for `set_text_content`: clearing PLAIN children (each with
/// an inner child) must free their own slots.
#[test]
fn set_text_content_clears_plain_children() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let p = child_of(&mut doc, body, "");

    let a = child_of(&mut doc, p, "height: 50px;");
    let _a_inner = child_of(&mut doc, a, "height: 10px;");
    let c = child_of(&mut doc, p, "height: 50px;");
    let _c_inner = child_of(&mut doc, c, "height: 10px;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, p), 100.0, "sanity: two 50px children");

    doc.set_text_content(p, "");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        taffy_child_count(&doc, p),
        0,
        "the old children's slots must be freed (the text leaf itself is \
         detached by IFC setup, so a healthy list is empty)"
    );
    assert!(
        height_of(&doc, p) < 50.0,
        "the plain 50px children must be gone, got {}",
        height_of(&doc, p)
    );
}

// ---------------------------------------------------------------------------
// set_inner_html — child-clearing
// ---------------------------------------------------------------------------

/// The #517 bug on `set_inner_html`'s child-clearing. `remove_subtree` only
/// clears the slab, never Taffy, so without the contents-aware detach the
/// spliced Taffy leaves live on with no DOM node behind them at all.
#[test]
fn set_inner_html_clears_spliced_slots() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let p = child_of(&mut doc, body, "");

    let wrapper = child_of(&mut doc, p, "display: contents");
    let _a1 = child_of(&mut doc, wrapper, "height: 50px;");
    let _a2 = child_of(&mut doc, wrapper, "height: 50px;");

    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, p),
        100.0,
        "sanity: two spliced 50px children"
    );

    doc.set_inner_html(p, "");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        taffy_child_count(&doc, p),
        0,
        "no Taffy children may survive clearing the subtree"
    );
    // An empty IFC root may still report one empty line box (~19px);
    // the discriminating value is the phantom pair's 100px.
    assert!(
        height_of(&doc, p) < 50.0,
        "the spliced 50px children must be gone, got {}",
        height_of(&doc, p)
    );
}

/// BLIND-SPOT twin for `set_inner_html`: clearing PLAIN children must free
/// their own slots.
#[test]
fn set_inner_html_clears_plain_children() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let p = child_of(&mut doc, body, "");

    let a = child_of(&mut doc, p, "height: 50px;");
    let _a_inner = child_of(&mut doc, a, "height: 10px;");
    let c = child_of(&mut doc, p, "height: 50px;");
    let _c_inner = child_of(&mut doc, c, "height: 10px;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, p), 100.0, "sanity: two 50px children");

    doc.set_inner_html(p, "");
    doc.resolve_layout(VW, VH);
    assert_eq!(
        taffy_child_count(&doc, p),
        0,
        "no Taffy children may survive"
    );
    // An empty IFC root may still report one empty line box (~19px);
    // the discriminating value is the phantom pair's 100px.
    assert!(
        height_of(&doc, p) < 50.0,
        "the plain 50px children must be gone, got {}",
        height_of(&doc, p)
    );
}

// ---------------------------------------------------------------------------
// Reparent legs: append_child / insert_before / insert_child moving an
// already-parented node away.
//
// A plain move does NOT leak on its own: `sync_display_contents` rebuilds
// the new parent with taffy's `set_children`, and taffy removes each new
// child from its previous parent as it adopts it
// (taffy-0.12.2/src/tree/taffy_tree.rs, `set_children`'s
// remove-from-previous-parent loop) — so the OLD parent heals at the next
// layout pass whether or not it retains another contents child. The
// straight-move tests below passed before the fix and stand as guards that
// the contribution-aware detach does not over-remove on the happy path.
//
// What DOES leak pre-fix is a move whose self-heal never arrives: move the
// wrapper, then remove it again before any layout pass runs. No sync ever
// re-adopts the spliced children, so the old parent keeps them forever.
// The fix detaches the contribution at move time, which closes that window
// (and makes the detach invariant local instead of resting on the next
// layout's rebuild).
// ---------------------------------------------------------------------------

/// The move-then-remove interleaving that defeats the next-layout self-heal:
/// after a splice, move the wrapper to another parent and remove it again
/// before any `resolve_layout`. Pre-fix the spliced children stay in the OLD
/// parent's Taffy list forever (nothing rebuilds either parent once the
/// wrapper is gone). Kills: leaving the reparent legs on own-id-only detach.
#[test]
fn move_then_remove_before_layout_leaves_no_phantom() {
    let mut doc = RinchDocument::new();
    let col_a = flex_column(&mut doc, 200.0);
    let col_b = flex_column(&mut doc, 200.0);

    let wrapper = child_of(&mut doc, col_a, "display: contents");
    let _a1 = child_of(&mut doc, wrapper, "height: 50px;");
    let _a2 = child_of(&mut doc, wrapper, "height: 50px;");
    let b_a = child_of(&mut doc, col_a, "flex: 1;");
    let b_b = child_of(&mut doc, col_b, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b_a), 100.0, "sanity: colA = 50+50+flex");

    // One reactive flush, two mutations, no layout in between.
    doc.append_child(col_b, wrapper);
    doc.remove_child(col_b, wrapper);
    doc.resolve_layout(VW, VH);

    assert_eq!(
        height_of(&doc, b_a),
        200.0,
        "colA must not keep the moved-away wrapper's spliced slots"
    );
    assert_eq!(
        height_of(&doc, b_b),
        200.0,
        "colB must not keep anything of the removed wrapper either"
    );
}

/// `append_child` moving a spliced wrapper away: the old parent must be
/// clean after the next layout. (Passed pre-fix via the `set_children`
/// self-heal; with the fix the old parent is healed at move time. Guards
/// the detach leg against over-removal either way.)
#[test]
fn append_child_move_away_heals_old_parent() {
    let mut doc = RinchDocument::new();
    let col_a = flex_column(&mut doc, 200.0);
    let col_b = flex_column(&mut doc, 200.0);

    let wrapper = child_of(&mut doc, col_a, "display: contents");
    let a1 = child_of(&mut doc, wrapper, "height: 50px;");
    let a2 = child_of(&mut doc, wrapper, "height: 50px;");
    let b_a = child_of(&mut doc, col_a, "flex: 1;");
    let b_b = child_of(&mut doc, col_b, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b_a), 100.0, "sanity: colA = 50+50+flex");
    assert_eq!(height_of(&doc, b_b), 200.0, "sanity: colB has one child");

    doc.append_child(col_b, wrapper);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b_a),
        200.0,
        "colA must give the survivor everything; 100 = phantoms stayed behind"
    );
    assert_eq!(height_of(&doc, a1), 50.0, "the wrapper splices into colB");
    assert_eq!(height_of(&doc, a2), 50.0, "the wrapper splices into colB");
    assert_eq!(height_of(&doc, b_b), 100.0, "colB: 50+50+flex leaves 100");
}

/// `insert_before` moving a spliced wrapper away — same contract as the
/// `append_child` variant above (passed pre-fix via the self-heal; guards
/// the detach leg against over-removal).
#[test]
fn insert_before_move_away_heals_old_parent() {
    let mut doc = RinchDocument::new();
    let col_a = flex_column(&mut doc, 200.0);
    let col_b = flex_column(&mut doc, 200.0);

    let wrapper = child_of(&mut doc, col_a, "display: contents");
    let a1 = child_of(&mut doc, wrapper, "height: 50px;");
    let a2 = child_of(&mut doc, wrapper, "height: 50px;");
    let b_a = child_of(&mut doc, col_a, "flex: 1;");
    let b_b = child_of(&mut doc, col_b, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b_a), 100.0, "sanity: colA = 50+50+flex");

    doc.insert_before(col_b, wrapper, b_b);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b_a),
        200.0,
        "colA must give the survivor everything; 100 = phantoms stayed behind"
    );
    assert_eq!(height_of(&doc, a1), 50.0, "the wrapper splices into colB");
    assert_eq!(height_of(&doc, a2), 50.0, "the wrapper splices into colB");
    assert_eq!(height_of(&doc, b_b), 100.0, "colB: 50+50+flex leaves 100");
}

/// `insert_child` moving a spliced wrapper away — same contract as the
/// `append_child` variant above (passed pre-fix via the self-heal; guards
/// the detach leg against over-removal).
#[test]
fn insert_child_move_away_heals_old_parent() {
    let mut doc = RinchDocument::new();
    let col_a = flex_column(&mut doc, 200.0);
    let col_b = flex_column(&mut doc, 200.0);

    let wrapper = child_of(&mut doc, col_a, "display: contents");
    let a1 = child_of(&mut doc, wrapper, "height: 50px;");
    let a2 = child_of(&mut doc, wrapper, "height: 50px;");
    let b_a = child_of(&mut doc, col_a, "flex: 1;");
    let b_b = child_of(&mut doc, col_b, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b_a), 100.0, "sanity: colA = 50+50+flex");

    doc.insert_child(col_b, wrapper, 0);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b_a),
        200.0,
        "colA must give the survivor everything; 100 = phantoms stayed behind"
    );
    assert_eq!(height_of(&doc, a1), 50.0, "the wrapper splices into colB");
    assert_eq!(height_of(&doc, a2), 50.0, "the wrapper splices into colB");
    assert_eq!(height_of(&doc, b_b), 100.0, "colB: 50+50+flex leaves 100");
}

/// BLIND-SPOT twin for the reparent legs: moving a PLAIN node (with an inner
/// child) away must free its own slot in the old parent. One test per leg.
#[test]
fn append_child_move_plain_node_heals_old_parent() {
    let mut doc = RinchDocument::new();
    let col_a = flex_column(&mut doc, 200.0);
    let col_b = flex_column(&mut doc, 200.0);

    let a = child_of(&mut doc, col_a, "flex: 1;");
    let _a_inner = child_of(&mut doc, a, "height: 10px;");
    let b_a = child_of(&mut doc, col_a, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b_a), 100.0, "sanity: even split");

    doc.append_child(col_b, a);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b_a),
        200.0,
        "a plain moved node frees its slot"
    );
    assert_eq!(height_of(&doc, a), 200.0, "and claims its new parent");
}

#[test]
fn insert_before_move_plain_node_heals_old_parent() {
    let mut doc = RinchDocument::new();
    let col_a = flex_column(&mut doc, 200.0);
    let col_b = flex_column(&mut doc, 200.0);

    let a = child_of(&mut doc, col_a, "flex: 1;");
    let _a_inner = child_of(&mut doc, a, "height: 10px;");
    let b_a = child_of(&mut doc, col_a, "flex: 1;");
    let b_b = child_of(&mut doc, col_b, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b_a), 100.0, "sanity: even split");

    doc.insert_before(col_b, a, b_b);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b_a),
        200.0,
        "a plain moved node frees its slot"
    );
    assert_eq!(
        height_of(&doc, a),
        100.0,
        "and splits its new parent evenly"
    );
}

#[test]
fn insert_child_move_plain_node_heals_old_parent() {
    let mut doc = RinchDocument::new();
    let col_a = flex_column(&mut doc, 200.0);
    let col_b = flex_column(&mut doc, 200.0);

    let a = child_of(&mut doc, col_a, "flex: 1;");
    let _a_inner = child_of(&mut doc, a, "height: 10px;");
    let b_a = child_of(&mut doc, col_a, "flex: 1;");
    let b_b = child_of(&mut doc, col_b, "flex: 1;");

    doc.resolve_layout(VW, VH);
    assert_eq!(height_of(&doc, b_a), 100.0, "sanity: even split");

    doc.insert_child(col_b, a, 0);
    doc.resolve_layout(VW, VH);
    assert_eq!(
        height_of(&doc, b_a),
        200.0,
        "a plain moved node frees its slot"
    );
    assert_eq!(
        height_of(&doc, b_b),
        100.0,
        "and splits its new parent evenly"
    );
}
