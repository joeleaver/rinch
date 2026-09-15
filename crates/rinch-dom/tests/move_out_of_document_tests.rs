//! #702 — a mounted node moved **into a detached parent** has left the document.
//!
//! #699 gave a subtree that leaves the document no before-change style, at the
//! four routes that write `parent = None`. A fifth shape leaves and was not
//! covered: `append_child(detached_parent, mounted_node)`, and the same through
//! `insert_before` / `insert_child`. The node keeps *a* parent, so it fails the
//! `parent = None` test those four share — but it is no longer reachable from
//! `tree.root_id`, which #696 established is the question that actually matters.
//! Splice that parent in somewhere else later and the node animates in from its
//! pre-move style, which is #699's symptom exactly.
//!
//! # Why this could not just be hooked
//!
//! Those three functions are also the **move** routes. A keyed `for` reorder
//! splices rows with `insert_after`, which is `insert_before` or `append_child`,
//! and a move must not reset anything: the node is back in the document before
//! the call returns, so it never stopped being rendered.
//! `reinsertion_transition_tests::a_reparenting_move_does_not_restart_a_running_transition`
//! and `a_keyed_for_reorder_does_not_restart_a_running_transition` pin that, and
//! they are exactly the two fixtures that kill #699's over-reach mutant.
//!
//! Telling the two apart is a connectivity question, and the answer has to be
//! cheap on the hottest DOM verb in the framework. Two guards make it so, and
//! **the first is what keeps a reorder free**:
//!
//! - **The new parent must differ from the old one.** A move within one
//!   container cannot change whether the child is connected — the child's
//!   reachability *is* its parent's, and the parent has not changed. A keyed
//!   reorder is that move, every time, so it pays one integer comparison.
//! - **The child must already have a parent.** A node created moments ago cannot
//!   be a move, so a node appended straight into its final parent never reaches
//!   the helper.
//!
//! Only a **reparenting** move reaches `depth_if_connected`, one O(depth) walk
//! on the new parent.
//!
//! **Counted, not argued** (counters on the helper, the walk, the reset and the
//! nodes the reset visits, instrumented on the committed source and reverted —
//! the timing on a loaded host cannot resolve a difference this small, so the
//! deterministic question is the one worth asking):
//!
//! | workload | helper | walks | resets | nodes reset |
//! |---|---|---|---|---|
//! | 500 rows built straight into their final parent | 0 | 0 | 0 | 0 |
//! | 500-row keyed reorder, each row to the front | 499 | **0** | 0 | 0 |
//! | 500 rows reparented into a second connected list | 500 | 500 | 0 | 0 |
//! | **500 `rsx!` component sites**, 20 nodes each | 500 | 500 | 500 | **10,000** |
//!
//! The second row is the claim the hot path rests on. The third and fourth are
//! its positive controls — a counter reading 0 everywhere would say the
//! instrument never fired.
//!
//! **The fourth row is the one that corrects an earlier draft of this file**,
//! which said the initial build of a tree pays nothing. That is true only of
//! nodes appended straight into their final parent. `rsx!` does not build a
//! component site that way: `component_codegen` puts the site's children into a
//! `<template>` attached to nothing (#719) and `Component::render` adopts them
//! into a root that is *also* still detached, so every adoption is a move into a
//! detached parent — it walks, and it takes the whole subtree reset. The reset is
//! semantically a no-op there (a node created moments ago is already unstyled
//! with empty animation maps) and the cost does not show — best of 40, release,
//! three alternated rounds, the build *with* the helper was the faster of the two
//! every time, 1801–1825ms against 1809–1830ms — but the sentence was wrong about
//! the framework's own render path and is now right about it.
//!
//! # Which routes there are
//!
//! **Four**, not the three the issue names. `replace_node` splices an incoming
//! `new` into `old`'s parent, and when that parent is detached a mounted `new`
//! leaves the document by exactly this shape — the function's own comment
//! asserted the opposite ("`new` has not [left the document] — it was spliced
//! in, which is a move, and a move resets nothing"), which is true of every
//! destination but a detached one. `grep -n '\.parent = Some('` over
//! `dom_impl/dom_document_impl.rs` is the count to re-check; the other matches
//! there and in `pseudo.rs` / `ifc.rs` are nodes created moments earlier, which
//! cannot be moves.
//!
//! # Mutants, and what kills each
//!
//! Every attribution is **measured**: each mutant applied to the committed
//! source, this file *and* `reinsertion_transition_tests` run with
//! `--no-fail-fast` (without which the second binary never runs once the first
//! has failed, and its column would be a silence), source reverted from the
//! commit.
//!
//! | mutant | killed by |
//! |---|---|
//! | the reset deleted — i.e. `main` before this change | **7 of the 8 here**; only `a_reparenting_move_between_two_connected_parents_resets_nothing` survives, which is what that control is for |
//! | over-reach: reset on every **reparenting** move, connected or not | `a_reparenting_move_between_two_connected_parents_resets_nothing` **and** `reinsertion_transition_tests::a_reparenting_move_does_not_restart_a_running_transition` |
//! | over-reach: reset on **every** move, same-parent included | those two **and** `a_keyed_for_reorder_does_not_restart_a_running_transition` — the reorder pin is what the third one adds |
//! | the `old_parent != new_parent` short-circuit dropped | **nothing, and that is recorded rather than hidden.** It is a *cost* guard, not a correctness one: a same-parent move cannot change connectivity, so walking gives the same answer more slowly. The counted table above is what defends it, and `move_reorder_bench` is where it is timed |
//! | the reset applies to the moved node only, not its subtree | `a_deep_node_moved_out_under_its_parent_does_not_animate_either`, **alone** |
//! | `append_child` unhooked | 4 of the 8 |
//! | `insert_before` + `insert_child` unhooked | `insert_before_into_a_detached_parent_is_a_detach_too` and `insert_child_into_a_detached_parent_is_a_detach_too`, exclusively |
//! | `replace_node` unhooked | `replace_node_into_a_detached_parent_is_a_detach_too`, **alone** — the fourth route, and the one no fixture would have covered if the issue's list of three had been taken as complete |

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// `font-size` and `line-height` are declared so that no box below is derived
/// from a font metric, and every width is a declaration a reader can check.
const CSS: &str = "
    .box  { width: 10px; height: 10px; font-size: 16px; line-height: 20px;
            transition: width 150ms linear; }
    .w--a .box { width: 20px; }
    .w--b .box { width: 30px; }
";

fn width_px(doc: &RinchDocument, node: NodeId) -> Option<f32> {
    match doc.tree.get(node.0)?.computed_style.width {
        rinch_dom::computed_style::DimensionValue::Length(px) => Some(px),
        _ => None,
    }
}

fn running(doc: &RinchDocument, node: NodeId) -> usize {
    doc.tree
        .active_transitions
        .get(&node.0)
        .map(|m| m.len())
        .unwrap_or(0)
}

fn styled(doc: &RinchDocument, node: NodeId) -> bool {
    doc.tree.get(node.0).unwrap().has_been_styled
}

/// `body > div.w--a > div.box` plus an empty `body > div.w--b` to move into,
/// laid out once with transitions armed.
///
/// Returns `(doc, destination, box)` — the destination is the `.w--b` holder.
fn mounted() -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();

    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "class", "w--a");
    doc.append_child(body, wrap);
    let boxed = doc.create_element("div");
    doc.set_attribute(boxed, "class", "box");
    doc.append_child(wrap, boxed);

    let holder = doc.create_element("div");
    doc.set_attribute(holder, "class", "w--b");
    doc.append_child(body, holder);

    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        width_px(&doc, boxed),
        Some(20.0),
        "precondition: the box takes its width from the wrapper it is mounted under"
    );
    assert_eq!(running(&doc, boxed), 0, "precondition: nothing running");
    assert!(styled(&doc, boxed), "precondition: it has been styled");
    (doc, holder, boxed)
}

/// The issue's own shape, through `append_child`.
///
/// Move the mounted box into a parent that is **not** in the document, resolve
/// (which must not style it — #696), then splice that parent in under a
/// different ancestor chain. The box must arrive at 30px with nothing running.
#[test]
fn append_child_into_a_detached_parent_is_a_detach() {
    let (mut doc, holder, boxed) = mounted();

    let orphan = doc.create_element("div");
    doc.append_child(orphan, boxed);
    doc.resolve_layout(801.0, 600.0);

    assert_eq!(
        doc.parent_node(boxed),
        Some(orphan),
        "precondition: it kept a parent, which is why the parent = None routes miss it"
    );
    assert!(
        !styled(&doc, boxed),
        "a node that is no longer reachable from the root has no before-change style"
    );

    doc.append_child(holder, orphan);
    doc.resolve_layout(802.0, 600.0);

    assert_eq!(
        running(&doc, boxed),
        0,
        "so its re-entry is a first style, not a change to animate"
    );
    assert_eq!(
        width_px(&doc, boxed),
        Some(30.0),
        "and it appears at its new value"
    );
}

/// The same through `insert_before`.
#[test]
fn insert_before_into_a_detached_parent_is_a_detach_too() {
    let (mut doc, holder, boxed) = mounted();

    let orphan = doc.create_element("div");
    let anchor = doc.create_element("span");
    doc.append_child(orphan, anchor);

    doc.insert_before(orphan, boxed, anchor);
    doc.resolve_layout(801.0, 600.0);
    assert!(!styled(&doc, boxed), "it left the document");

    doc.append_child(holder, orphan);
    doc.resolve_layout(802.0, 600.0);

    assert_eq!(running(&doc, boxed), 0, "no transition on its re-entry");
    assert_eq!(width_px(&doc, boxed), Some(30.0), "at its new value");
}

/// The same through `insert_child`.
#[test]
fn insert_child_into_a_detached_parent_is_a_detach_too() {
    let (mut doc, holder, boxed) = mounted();

    let orphan = doc.create_element("div");
    doc.insert_child(orphan, boxed, 0);
    doc.resolve_layout(801.0, 600.0);
    assert!(!styled(&doc, boxed), "it left the document");

    doc.append_child(holder, orphan);
    doc.resolve_layout(802.0, 600.0);

    assert_eq!(running(&doc, boxed), 0, "no transition on its re-entry");
    assert_eq!(width_px(&doc, boxed), Some(30.0), "at its new value");
}

/// `replace_node` is the **fourth** move route, and the one the issue does not
/// name.
///
/// Replacing a node whose parent is detached splices the incoming `new` into a
/// parent that is not in the document — so a mounted `new` leaves it. The
/// function's own comment asserted the opposite in so many words ("`new` has
/// not [left the document] — it was spliced in, which is a move, and a move
/// resets nothing"), which is true of every destination but a detached one.
#[test]
fn replace_node_into_a_detached_parent_is_a_detach_too() {
    let (mut doc, holder, boxed) = mounted();

    // An orphan holding a placeholder to replace.
    let orphan = doc.create_element("div");
    let placeholder = doc.create_element("span");
    doc.append_child(orphan, placeholder);

    doc.replace_node(placeholder, boxed);
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(
        doc.parent_node(boxed),
        Some(orphan),
        "precondition: it took the placeholder's place in the detached parent"
    );
    assert!(!styled(&doc, boxed), "so it left the document");

    doc.append_child(holder, orphan);
    doc.resolve_layout(802.0, 600.0);

    assert_eq!(running(&doc, boxed), 0, "no transition on its re-entry");
    assert_eq!(width_px(&doc, boxed), Some(30.0), "at its new value");
}

/// The reset covers the moved node's whole subtree, not just the node moved.
///
/// A descendant carries its own flag, its own `transition` declaration and its
/// own resolved value, and resolution reaches it by its own recursion — so a
/// reset that stops at the move root leaves it animating.
#[test]
fn a_deep_node_moved_out_under_its_parent_does_not_animate_either() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();

    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "class", "w--a");
    doc.append_child(body, wrap);
    // wrap > mid > deep.box — the move is of `mid`, the assertion is on `deep`.
    let mid = doc.create_element("div");
    doc.append_child(wrap, mid);
    let deep = doc.create_element("div");
    doc.set_attribute(deep, "class", "box");
    doc.append_child(mid, deep);

    let holder = doc.create_element("div");
    doc.set_attribute(holder, "class", "w--b");
    doc.append_child(body, holder);

    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width_px(&doc, deep), Some(20.0), "precondition: 20px");

    let orphan = doc.create_element("div");
    doc.append_child(orphan, mid);
    doc.resolve_layout(801.0, 600.0);
    assert!(
        !styled(&doc, deep),
        "the descendant left the document with its parent"
    );

    doc.append_child(holder, orphan);
    doc.resolve_layout(802.0, 600.0);

    assert_eq!(running(&doc, deep), 0, "so it does not animate in");
    assert_eq!(width_px(&doc, deep), Some(30.0), "it appears at 30px");
}

/// A transition already in flight when the node is moved out is cancelled.
///
/// `tick_transitions` walks `tree.active_transitions`, not the document, so one
/// left behind goes on writing interpolated values into a node nobody can
/// reach — and goes on doing so after the node comes back, reinstating the
/// animation the flag reset removed.
#[test]
fn a_transition_running_when_the_node_is_moved_out_does_not_resume() {
    let (mut doc, holder, boxed) = mounted();

    // Retarget in place so a transition is genuinely in flight.
    let wrap = doc.parent_node(boxed).unwrap();
    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(
        running(&doc, boxed),
        1,
        "precondition: a transition is in flight"
    );

    let orphan = doc.create_element("div");
    doc.append_child(orphan, boxed);
    doc.resolve_layout(802.0, 600.0);

    assert_eq!(
        running(&doc, boxed),
        0,
        "leaving the document cancels it, as it does on every other detach route"
    );

    doc.append_child(holder, orphan);
    doc.resolve_layout(803.0, 600.0);
    assert_eq!(running(&doc, boxed), 0, "and it does not come back");
}

/// The control that says the reset is confined to a move **out** of the
/// document: a reparenting move between two *connected* parents resets nothing.
///
/// `reinsertion_transition_tests::a_reparenting_move_does_not_restart_a_running_transition`
/// is the other half of this — it asserts a running transition survives such a
/// move. This one asserts the flag does, which is what the reset would take.
#[test]
fn a_reparenting_move_between_two_connected_parents_resets_nothing() {
    let (mut doc, holder, boxed) = mounted();

    doc.append_child(holder, boxed);
    doc.resolve_layout(801.0, 600.0);

    assert!(
        styled(&doc, boxed),
        "the node never left the document, so it keeps its before-change style"
    );
    assert_eq!(
        running(&doc, boxed),
        1,
        "and the value it resolves to under the new ancestor is a change to animate"
    );
}

/// A node moved out and never brought back can still transition once it is
/// mounted somewhere for real.
///
/// "Clear a flag on the way out" is one careless edit away from "clear it and
/// never set it again" — #704's failure mode — and no other fixture here would
/// notice.
#[test]
fn a_node_moved_out_and_mounted_again_can_still_transition() {
    let (mut doc, holder, boxed) = mounted();

    let orphan = doc.create_element("div");
    doc.append_child(orphan, boxed);
    doc.resolve_layout(801.0, 600.0);

    doc.append_child(holder, orphan);
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(width_px(&doc, boxed), Some(30.0), "mounted at 30px");
    assert_eq!(running(&doc, boxed), 0, "with nothing running");

    // Now an ordinary in-document change: this one *must* animate.
    doc.set_attribute(holder, "class", "w--a");
    doc.resolve_layout(803.0, 600.0);
    assert_eq!(
        running(&doc, boxed),
        1,
        "the node is re-armed — the reset is not permanent"
    );
}
