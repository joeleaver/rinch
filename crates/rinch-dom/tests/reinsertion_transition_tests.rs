//! #699 — a subtree that left the document has no before-change style.
//!
//! `has_been_styled` is the whole of it. The cascade
//! (`apply_stylo_styles_to_taffy`) starts a `transition` only when the node it
//! is restyling has been styled before, and compares the fresh cascade against
//! the `computed_style` that node is carrying. A node styled while it was
//! *connected*, then detached, keeps both: so if an ancestor's class changes
//! while it is out, its re-insertion resolves to a different value, the cascade
//! reads old ≠ new on an already-styled node, and the box **animates in from a
//! style the user never saw**.
//!
//! A browser does not. A removed element is not rendered, it has no
//! before-change style, and re-insertion is a first style.
//!
//! #696 answered the other half — a node whose *first* resolution happened
//! while it was detached — by never styling a disconnected node at all. That
//! left this one untouched, and its reviewer measured it identical at base and
//! head. The cure has to be at the **detach**, because nothing at the
//! re-insertion can tell a returning subtree from one that never left:
//! `RinchDocument::detach_subtree_styles`, called from **four of the five places
//! in `dom_impl/dom_document_impl.rs` that write `parent = None`** — the fifth,
//! `set_inner_html`, frees the subtree outright through
//! `NodeTree::remove_subtree` and needs no reset. The fourth,
//! `set_text_content`, does not look like a detach at all and was missed on the
//! first pass; see `set_text_content_is_a_detach_too`.
//!
//! # Mutants, and what kills each
//!
//! Every attribution below is **measured** — each mutant was applied to the
//! committed source, the whole file run against it, and the source reverted.
//!
//! | mutant | killed by |
//! |---|---|
//! | no reset anywhere (`main` at `cbdfc5a`; re-measured for #704 as `detach_subtree_styles` made a no-op) | 10 of the 14 — everything but the two move fixtures, the unchanged-ancestor control and the `display: none` pin. It was 9 until #704 removed the inline `transition: none` the reactive helpers stamped; `the_reactive_branch_helpers_reach_this_fix_too` was blind to this mutant while that hammer stood, and is the tenth now |
//! | reset the detach root only, not the subtree | `a_deep_node_in_a_reinserted_subtree_does_not_animate_either`, **alone** |
//! | reset `has_been_styled`, leave `active_transitions` | `a_transition_running_when_the_subtree_is_detached_does_not_resume` and `set_text_content_is_a_detach_too` |
//! | drop `active_transitions` but not `active_animations` | `a_detached_animation_stops_asking_for_frames`, **alone** |
//! | `remove_child` and `replace_node` unhooked | `remove_child_is_a_detach_too`, `the_subtree_a_replace_displaces_is_a_detach_too`, `a_detached_subtree_keeps_the_style_it_last_had` |
//! | `set_text_content` unhooked — **the state this PR shipped in for one round** | `set_text_content_is_a_detach_too`, **alone** |
//! | also clear `computed_style` and `text_layout` in the reset | `a_detached_subtree_keeps_the_style_it_last_had` here, **and four fixtures in `detached_style_roots_tests`** |
//! | also reset on a reparenting `append_child`/`insert_before`/`insert_child` (over-reach) | `a_reparenting_move_does_not_restart_a_running_transition` and `a_keyed_for_reorder_does_not_restart_a_running_transition`, and nothing else |
//! | over-reach on `insert_before` + `insert_child` **only** | `a_keyed_for_reorder_does_not_restart_a_running_transition`, **alone** |
//! | never re-arm (`has_been_styled = true` deleted from the cascade) | 6 here **and 10 in `transition_tests`** — recorded because `a_subtree_that_has_been_round_tripped_can_still_transition` is **not** its only witness, and does not claim to be |
//!
//! Four rows are this PR's review round, and three of them are mutants that
//! **survived the first eleven fixtures** — the `set_text_content` row, the
//! `active_animations` row and the partial over-reach row. Each was found by
//! constructing a counter-case to a sentence rather than by reading the diff,
//! which is the project's standing lesson about confident negatives.
//!
//! The last row is the honest one. `a_subtree_that_has_been_round_tripped_can_
//! still_transition` kills no mutant on its own: deleting the cascade's
//! `has_been_styled = true` breaks sixteen fixtures across two files. It is here
//! because **re-armability is a property nothing else in the suite asserts**, and
//! "clear a flag on the way out" is one careless edit away from "clear it and
//! never set it again" — which is #704's failure mode exactly, and which every
//! other fixture in this file would be equally happy with.
//!
//! Three rows are worth reading twice.
//!
//! **The over-reach row.** A keyed `for` reorder moves rows with
//! `insert_after`, which is `insert_before`/`append_child` — the same three
//! lines that unlink a node from its old parent. Resetting there would restart
//! every mid-flight transition on every list reorder, and those two fixtures
//! are the only thing in the suite that says so.
//!
//! **The partial over-reach row.** `NodeHandle::insert_after` falls through to
//! `append_child` when its anchor has no next sibling, so a keyed rotation that
//! moves a row to the *end* never reaches `insert_before`. Both move fixtures
//! sat there in the first revision and the over-reach mutant applied to
//! `insert_before`/`insert_child` alone passed all eleven. The reorder fixture
//! now moves the last row to the **front**, which is the route every move but
//! the last-position one takes.
//!
//! **The `computed_style` row.** Clearing the stale value as well as the flag
//! is the obvious second half, and it is wrong: #696 pinned a detached node as
//! still *readable* (the debug `dom_tree(root_id: …)` path) and still carrying
//! its shaped text, and the re-insertion's own staleness gates read that same
//! `computed_style` to decide whether to re-shape (#654, #661, #678). The flag
//! is what the transition reads; the value is what everything else reads. Only
//! the flag has to go — measured, four of #696's nine fixtures fail if it does
//! not stay.
//!
//! # Which routes are live
//!
//! The defect is in `rinch-dom`'s `DomDocument` implementation, so **every**
//! tree-mutation caller reaches it. `NodeHandle::remove_child`, `RenderScope`'s
//! batched `DomUpdate::RemoveChild`, `NodeHandle::replace_with` (which
//! `rinch-editor-view`'s `ViewDesc` diff uses), any component that stashes a
//! `NodeHandle` and re-attaches it — the pattern #654 was reported from — and,
//! since #704, the three reactive branch helpers as well.
//!
//! Those three were the exception for one round, and not because of this fix.
//! `show_dom`, `match_dom` and `for_each_dom_typed` each called
//! `NodeHandle::clear_animations()` before `remove()`, which stamped an inline
//! `transition: none; animation: none` on the whole subtree and never took it
//! off again — a bigger hammer with a defect of its own (a branch hidden once
//! could never transition again, issue #704), and the reason the fixtures below
//! drive the DOM API directly: through a reactive helper, the mutant that
//! removes this entire fix used to pass. #704 deleted the method and all five
//! of its call sites, every one of which was `clear_animations(); remove();`
//! over a `NodeHandle::remove` that is `DomDocument::remove_node`.
//! `the_reactive_branch_helpers_reach_this_fix_too` is the same sequence turned
//! into a positive pin, and `branch_helper_transition_tests` is #704's own
//! file. Nothing below was rewritten to use a helper: driving the raw API keeps
//! each fixture's mutant attribution about *this* code rather than about five
//! call sites in `rinch-core`.
//!
//! # `display: none` is not a detach, and this file does not make it one
//!
//! `toggling_display_none_is_not_a_detach` pins today's behaviour rather than
//! the browser's: a node hidden with `display: none` stays in the document,
//! keeps `has_been_styled`, and **does** start a transition if its style
//! changes while it is hidden — which css-transitions-1 §3 does not, because a
//! non-rendered element has no before-change style either. That is a real
//! deviation and a separate one (issue #703); the pin is here so that a future
//! fix for it is a deliberate change with a test to update, not a silent
//! side-effect of this one.

#![cfg(feature = "software-renderer")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};
use rinch_core::reactive::Signal;
use rinch_dom::RinchDocument;

/// `font-size` and `line-height` are declared so that no box below is derived
/// from a font metric, and every width is a declaration a reader can check.
const CSS: &str = "
    .box  { width: 10px; height: 10px; font-size: 16px; line-height: 20px;
            transition: width 150ms linear; }
    .w--a .box { width: 20px; }
    .w--b .box { width: 30px; }
    .hidden .box { display: none; }
";

/// The node's computed `width` in px, or `None` when it is not a length.
///
/// `None` is the answer for `width: auto`, which is what a *cleared*
/// `computed_style` reads back as — so the fixtures that assert a kept style
/// go through this rather than a `matches!` that cannot tell the two apart.
fn width_px(doc: &RinchDocument, node: NodeId) -> Option<f32> {
    match doc.tree.get(node.0)?.computed_style.width {
        rinch_dom::computed_style::DimensionValue::Length(px) => Some(px),
        _ => None,
    }
}

/// How many transitions are running on a node.
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

/// `body > div.w--a > div.box`, laid out once with transitions armed.
///
/// Returns `(doc, wrapper, box)`.
fn mounted_box() -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "class", "w--a");
    doc.append_child(body, wrap);
    let boxed = doc.create_element("div");
    doc.set_attribute(boxed, "class", "box");
    doc.append_child(wrap, boxed);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        width_px(&doc, boxed),
        Some(20.0),
        "precondition: the mounted box takes its width from the wrapper's class"
    );
    assert_eq!(running(&doc, boxed), 0, "precondition: nothing running");
    (doc, wrap, boxed)
}

/// The issue's own case, on the plainest route.
///
/// Resolve, detach, change the ancestor's class while the box is out, put it
/// back. The re-insertion must land on 30px at once.
///
/// At `main` `cbdfc5a` it landed on **20.027164px with one transition
/// running** — the box was crawling toward 30 from a width it had under a class
/// its ancestor no longer carries.
///
/// The second `resolve_layout` uses a different viewport on purpose:
/// `resolve_layout` early-returns on `!layout_dirty`, so re-resolving at the
/// same size can test nothing.
#[test]
fn a_reinserted_subtree_does_not_animate_from_its_pre_detach_style() {
    let (mut doc, wrap, boxed) = mounted_box();

    doc.remove_node(boxed);
    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(800.0, 600.0);

    doc.append_child(wrap, boxed);
    doc.resolve_layout(801.0, 600.0);

    assert_eq!(
        running(&doc, boxed),
        0,
        "a re-inserted node has no before-change style, so nothing to animate"
    );
    assert_eq!(
        width_px(&doc, boxed),
        Some(30.0),
        "it must arrive at the new value, not approach it"
    );
    assert_eq!(
        doc.tree.get(boxed.0).unwrap().layout.width,
        30.0,
        "and the box laid out at that width, not at the interpolated one"
    );
}

/// The reset is over the whole removed **subtree**, not its root.
///
/// The transition is declared two levels below the node handed to
/// `remove_node`, so a reset that touches only `node_id` leaves this box's flag
/// set and it animates exactly as before. `.mid` carries no `transition` and no
/// width of its own, so nothing here can be satisfied by the detach root.
///
/// Kills the "reset the detach root only" mutant, and it is the only fixture
/// that does.
#[test]
fn a_deep_node_in_a_reinserted_subtree_does_not_animate_either() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "class", "w--a");
    doc.append_child(body, wrap);

    let panel = doc.create_element("div");
    let mid = doc.create_element("div");
    let boxed = doc.create_element("div");
    doc.set_attribute(boxed, "class", "box");
    doc.append_child(mid, boxed);
    doc.append_child(panel, mid);
    doc.append_child(wrap, panel);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width_px(&doc, boxed), Some(20.0), "precondition");

    doc.remove_node(panel);
    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(800.0, 600.0);

    assert!(
        !styled(&doc, boxed),
        "the grandchild's flag is what the cascade reads, so it is what must be reset"
    );

    doc.append_child(wrap, panel);
    doc.resolve_layout(801.0, 600.0);

    assert_eq!(
        running(&doc, boxed),
        0,
        "no transition two levels down either"
    );
    assert_eq!(width_px(&doc, boxed), Some(30.0));
}

/// A transition that was **already running** when the subtree was detached must
/// not resume when it comes back.
///
/// Resetting the flag alone does not achieve this, and that is the whole reason
/// the helper touches `tree.active_transitions` too: `tick_transitions` walks
/// that map, not the document, so a transition left behind by a detach goes on
/// writing interpolated values into `computed_style` — and would go on doing so
/// after the re-insertion, reinstating the very animation the flag reset
/// removes.
///
/// Kills the "reset `has_been_styled`, leave `active_transitions`" mutant, and
/// it is the only fixture that does.
#[test]
fn a_transition_running_when_the_subtree_is_detached_does_not_resume() {
    let (mut doc, wrap, boxed) = mounted_box();

    // Start a real 20 → 30 transition and leave it mid-flight.
    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(
        running(&doc, boxed),
        1,
        "precondition: a width transition is in flight"
    );

    doc.remove_node(boxed);
    assert_eq!(
        running(&doc, boxed),
        0,
        "removal cancels it — a transition belongs to a rendered element"
    );

    doc.append_child(wrap, boxed);
    doc.resolve_layout(802.0, 600.0);

    assert_eq!(
        running(&doc, boxed),
        0,
        "and it does not come back with the node"
    );
    assert_eq!(width_px(&doc, boxed), Some(30.0));

    // The tick is what a leftover entry would act through, so drive it: a
    // surviving 20 → 30 transition would write ~20 over the 30 just resolved.
    doc.tick_transitions();
    assert_eq!(
        width_px(&doc, boxed),
        Some(30.0),
        "nothing is left in the map to interpolate the box back down"
    );
}

/// Positive control: the reset does not *cause* anything.
///
/// Same round trip with the ancestor's class left alone. Nothing moves, the
/// style is the one it had, and no transition starts — which is also what
/// happens at `main`, so this fixture discriminates nothing on its own. It is
/// here to say that the fix's observable effect is confined to the case where
/// the resolved value actually changed.
#[test]
fn an_unchanged_ancestor_still_means_no_transition_and_the_same_style() {
    let (mut doc, wrap, boxed) = mounted_box();

    doc.remove_node(boxed);
    doc.resolve_layout(800.0, 600.0);
    doc.append_child(wrap, boxed);
    doc.resolve_layout(801.0, 600.0);

    assert_eq!(running(&doc, boxed), 0);
    assert_eq!(width_px(&doc, boxed), Some(20.0), "unchanged, and unmoved");
    assert_eq!(doc.tree.get(boxed.0).unwrap().layout.width, 20.0);
}

/// A **move** is not a detach, and a mid-flight transition survives one.
///
/// `append_child` and `insert_before` unlink the node from its old parent with
/// the same three lines `remove_child` uses, and it would be natural to put the
/// reset there too. It must not go there: the node is back in the document
/// before the call returns, so it never stopped being rendered, and a browser
/// keeps the transition running.
///
/// Kills the "also reset on a reparenting `append_child`/`insert_before`"
/// over-reach mutant.
#[test]
fn a_reparenting_move_does_not_restart_a_running_transition() {
    let (mut doc, wrap, boxed) = mounted_box();
    // A second wrapper carrying the *same* class, so the move changes nothing
    // about what the box resolves to and the only question is the transition.
    let body = doc.body();
    let other = doc.create_element("div");
    doc.set_attribute(other, "class", "w--a");
    doc.append_child(body, other);
    doc.resolve_layout(801.0, 600.0);

    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(running(&doc, boxed), 1, "precondition: in flight");
    let started_at = doc.tree.active_transitions[&boxed.0]
        .values()
        .next()
        .unwrap()
        .start_time_ms;

    doc.append_child(other, boxed);
    doc.resolve_layout(803.0, 600.0);

    assert!(
        styled(&doc, boxed),
        "a moved node never left the document, so it keeps its before-change style"
    );
    assert_eq!(
        running(&doc, boxed),
        1,
        "and its transition is still running — a reorder is not a remount"
    );
    assert_eq!(
        doc.tree.active_transitions[&boxed.0]
            .values()
            .next()
            .unwrap()
            .start_time_ms,
        started_at,
        "on the same clock it started on"
    );
}

/// The same property one level up, through the reconciler an app actually
/// reaches: a keyed `for` list reordered while one of its rows is mid-flight.
///
/// `ListOp::Move` splices the row with `NodeHandle::insert_after`, so this is
/// the shape the over-reach mutant would break in production — every mid-flight
/// transition in a list restarted on every reorder.
///
/// **The row is moved to the FRONT, and that is the whole point of the
/// rotation chosen.** `insert_after` falls through to `append_child` when the
/// anchor has no next sibling, so a rotation that moves a row to the *end*
/// exercises `append_child` — which the fixture above already covers. Moving
/// the last row to the front gives the marker a live next sibling and takes
/// `insert_before`, which is the route a keyed reorder uses for **every move
/// that is not to the last position**. An earlier revision rotated
/// `[a,b,c] → [b,c,a]`; both move fixtures then sat on `append_child`, and the
/// over-reach mutant applied to `insert_before`/`insert_child` alone passed all
/// eleven — measured by this PR's reviewer, which is how the hole was found.
#[test]
fn a_keyed_for_reorder_does_not_restart_a_running_transition() {
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    doc.borrow_mut().load_css(CSS);
    doc.borrow_mut().tree.transitions_enabled = true;
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();

    let mut scope = RenderScope::new(dyn_doc, body);
    let wrap = scope.create_element("div");
    wrap.set_attribute("class", "w--a");
    scope.parent().append_child(&wrap);

    let order = Signal::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    let rows: Rc<RefCell<Vec<(String, NodeId)>>> = Rc::new(RefCell::new(Vec::new()));
    let rows_view = rows.clone();
    let _marker = rinch_core::for_loop::for_each_dom_typed(
        &mut scope,
        &wrap,
        move || order.get(),
        |k: &String| k.clone(),
        move |k: String, s: &mut RenderScope| {
            let row = s.create_element("div");
            row.set_attribute("class", "box");
            rows_view.borrow_mut().push((k, NodeId(row.node_id().0)));
            row
        },
    );
    doc.borrow_mut().resolve_layout(800.0, 600.0);

    let row_c = rows
        .borrow()
        .iter()
        .find(|(k, _)| k == "c")
        .map(|(_, id)| *id)
        .expect("row c was rendered");
    assert_eq!(
        width_px(&doc.borrow(), row_c),
        Some(20.0),
        "precondition: the rows take their width from the wrapper"
    );

    // Put row c mid-flight, then rotate it to the front.
    wrap.set_attribute("class", "w--b");
    doc.borrow_mut().resolve_layout(801.0, 600.0);
    assert_eq!(
        running(&doc.borrow(), row_c),
        1,
        "precondition: row c is transitioning"
    );

    order.set(vec!["c".to_string(), "a".to_string(), "b".to_string()]);
    doc.borrow_mut().resolve_layout(802.0, 600.0);

    assert_eq!(
        rows.borrow().len(),
        3,
        "positive control: the reorder moved the rows, it did not re-render them"
    );
    assert_eq!(
        running(&doc.borrow(), row_c),
        1,
        "a row moved to the front keeps transitioning"
    );
}

/// `remove_child` is a detach route of its own — `NodeHandle::remove_child` and
/// `RenderScope`'s batched `DomUpdate::RemoveChild` both reach it without going
/// near `remove_node`.
///
/// Kills the "reset in `remove_node` only" mutant.
#[test]
fn remove_child_is_a_detach_too() {
    let (mut doc, wrap, boxed) = mounted_box();

    doc.remove_child(wrap, boxed);
    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(800.0, 600.0);

    doc.append_child(wrap, boxed);
    doc.resolve_layout(801.0, 600.0);

    assert_eq!(running(&doc, boxed), 0);
    assert_eq!(width_px(&doc, boxed), Some(30.0));
}

/// `replace_node` detaches the subtree it displaces, and that subtree is a
/// handle the caller may well put back — it is how a `for` row is re-rendered
/// in place (`old_state.node.insert_after(&new_node); old_state.node.remove()`
/// is the *other* spelling).
///
/// Only `old` is reset. `new` was spliced in, which is a move.
///
/// Kills the "reset in `remove_node` only" mutant.
#[test]
fn the_subtree_a_replace_displaces_is_a_detach_too() {
    let (mut doc, wrap, boxed) = mounted_box();
    let stand_in = doc.create_element("div");

    doc.replace_node(boxed, stand_in);
    assert!(
        !styled(&doc, boxed),
        "the displaced subtree has left the document"
    );

    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(800.0, 600.0);

    doc.replace_node(stand_in, boxed);
    doc.resolve_layout(801.0, 600.0);

    assert_eq!(running(&doc, boxed), 0);
    assert_eq!(width_px(&doc, boxed), Some(30.0));
}

/// The reset takes the **flag**, not the value.
///
/// A detached node still reads back as it last did in the document, on every
/// route. #696 pinned that for `remove_node`
/// (`detached_style_roots_tests::a_detached_node_is_still_readable`, which the
/// debug `dom_tree(root_id: …)` path depends on, and
/// `a_detached_subtree_keeps_its_text_layout`); this extends it to the two
/// routes this change added, and says why it matters here as well as there.
/// The re-insertion's own staleness gates read that same `computed_style` to
/// decide whether to re-shape the text (#654, #661, #678) — against a cleared
/// one they would answer "stale" for every re-inserted node, forever.
///
/// Kills the "also clear `computed_style` in the reset" mutant: a cleared style
/// reads back `width: auto`, which `width_px` reports as `None`.
#[test]
fn a_detached_subtree_keeps_the_style_it_last_had() {
    for route in ["remove_node", "remove_child", "replace_node"] {
        let (mut doc, wrap, boxed) = mounted_box();
        match route {
            "remove_node" => doc.remove_node(boxed),
            "remove_child" => doc.remove_child(wrap, boxed),
            _ => {
                let stand_in = doc.create_element("div");
                doc.replace_node(boxed, stand_in);
            }
        }
        assert!(!styled(&doc, boxed), "{route}: the flag goes");
        assert_eq!(
            width_px(&doc, boxed),
            Some(20.0),
            "{route}: and the value it last had in the document stays"
        );
    }
}

/// `display: none` is **not** a detach, and this change must not make it one.
///
/// This pins today's behaviour, which is not the browser's: the hidden node
/// keeps `has_been_styled`, so a style change while it is hidden starts a
/// transition, and it is still running when the node is shown again.
/// css-transitions-1 §3 starts no transition for an element that is not being
/// rendered, and `display: none` is one — but that is a separate deviation
/// (issue #703) with a separate cure, and conflating the two here would have
/// made this change's blast radius impossible to attribute.
#[test]
fn toggling_display_none_is_not_a_detach() {
    let (mut doc, wrap, boxed) = mounted_box();

    doc.set_attribute(wrap, "class", "w--a hidden");
    doc.resolve_layout(801.0, 600.0);
    assert!(
        styled(&doc, boxed),
        "a hidden node is still in the document, so it keeps its before-change style"
    );

    doc.set_attribute(wrap, "class", "w--b hidden");
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(
        running(&doc, boxed),
        1,
        "today rinch starts a transition on a non-rendered element (#703)"
    );

    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(803.0, 600.0);
    assert_eq!(
        running(&doc, boxed),
        1,
        "and it is still running when the node is shown — unchanged by #699"
    );
}

/// `set_text_content` is the **fourth** detach route, and the one that does not
/// look like one.
///
/// Called on an element that has children, it orphans every one of them
/// (`dom_document_impl.rs`, the `_ =>` arm) and replaces them with a single text
/// node. What makes it #699's shape rather than a tidy teardown is that it
/// **does not free the slab entry**: unlike `set_inner_html`, which reaches
/// `NodeTree::remove_subtree` and drops the node along with both animation maps,
/// these children stay alive, styled, and transitioning, reachable by any
/// `NodeHandle` the app still holds.
///
/// Missed on this PR's first pass, where the doc said "the three routes by which
/// a subtree leaves the document" in five places. Found by the reviewer with a
/// constructed counter-case, measured at head: the box came back at
/// **20.647852px with one transition running**, and the transition went on
/// ticking on an unreachable node in between.
///
/// Kills the "hook only `remove_node` / `remove_child` / `replace_node`" mutant,
/// which is the state this PR shipped in for one round.
#[test]
fn set_text_content_is_a_detach_too() {
    let (mut doc, wrap, _unused) = mounted_box();
    // A holder between the wrapper and the box, so `set_text_content` on the
    // holder displaces the box rather than the whole subtree under test.
    let holder = doc.create_element("div");
    doc.append_child(wrap, holder);
    let boxed = doc.create_element("div");
    doc.set_attribute(boxed, "class", "box");
    doc.append_child(holder, boxed);
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(width_px(&doc, boxed), Some(20.0), "precondition");

    // Mid-flight when it is displaced, so both halves are under test: the
    // running transition, and the flag the re-insertion would read.
    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(running(&doc, boxed), 1, "precondition: in flight");

    doc.set_text_content(holder, "replaced");
    assert!(
        doc.tree.get(boxed.0).is_some(),
        "precondition: orphaned, NOT freed — that is what makes this reachable"
    );
    assert!(doc.tree.get(boxed.0).unwrap().parent.is_none());
    assert!(!styled(&doc, boxed), "the flag goes");
    assert_eq!(running(&doc, boxed), 0, "and the transition with it");

    // Put it back under an ancestor that changed while it was out.
    doc.set_attribute(wrap, "class", "w--a");
    doc.resolve_layout(803.0, 600.0);
    doc.append_child(wrap, boxed);
    doc.resolve_layout(804.0, 600.0);

    assert_eq!(running(&doc, boxed), 0, "no transition on re-insertion");
    assert_eq!(
        width_px(&doc, boxed),
        Some(20.0),
        "it snaps, it does not crawl"
    );
}

/// A detached subtree stops asking the shell for frames.
///
/// The transition half of the reset self-limits — a transition has a declared
/// duration and dies after it. An **animation** does not: `animation: spin 1s
/// linear infinite` on a removed-but-not-freed node runs forever, and the
/// desktop shell decides whether to schedule another frame from
/// `!tree.active_animations.is_empty()` (`rinch/src/app/event_dispatch.rs`), so a
/// removed `Loader` kept a desktop app rendering at full rate with nothing on
/// screen to show for it. Since #704 deleted `NodeHandle::clear_animations`,
/// whose inline `animation: none` stopped the frames for the three reactive
/// helpers (at the cost of disarming the subtree forever), this is the only
/// thing that stops them on any route at all.
///
/// This is why the helper drops `active_animations` beside `active_transitions`,
/// which is also what `NodeTree::remove_subtree` does when it frees a subtree.
/// Kills the "drop `active_transitions` only" mutant; no other fixture does,
/// because an animation writes `computed_style` without consulting
/// `has_been_styled`, so #699's own symptom cannot see it.
#[test]
fn a_detached_animation_stops_asking_for_frames() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "@keyframes sp { from { width: 10px; } to { width: 100px; } } \
         .spin { animation: sp 1000s linear infinite; width: 10px; height: 10px; \
                 font-size: 16px; line-height: 20px; }",
    );
    let body = doc.body();
    let spinner = doc.create_element("div");
    doc.set_attribute(spinner, "class", "spin");
    doc.append_child(body, spinner);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    assert!(
        !doc.tree.active_animations.is_empty(),
        "precondition: the shell is being asked for frames"
    );

    doc.remove_node(spinner);

    assert!(
        doc.tree.active_animations.is_empty(),
        "a removed spinner must stop asking for frames — it has no duration to \
         expire and nothing else ever clears it"
    );
    assert!(
        !doc.tick_animations(),
        "and the tick must agree there is nothing left to advance"
    );
}

/// The reset must not **permanently** disarm, which is the exact thing #704 got
/// wrong.
///
/// Two full round trips, then an ordinary in-document class change with nothing
/// detached: it has to animate. `has_been_styled` is set again by the
/// re-insertion's own resolution, so the node is re-armed the moment it is back
/// — but nothing in the suite said so, and "clear a flag on the way out" is one
/// careless edit away from "clear it and never set it again", which is
/// indistinguishable from a working fix on every other fixture here.
///
/// The second trip is not decoration: it is what says the reset is idempotent
/// rather than a one-shot that a second detach corrupts.
///
/// **It kills no mutant on its own**, and does not claim to — deleting the
/// cascade's `has_been_styled = true` breaks six fixtures here and ten in
/// `transition_tests`. It is a pin on a property, not a discriminator, which is
/// the honest reason to keep it: no other fixture in this file would notice a
/// change that made the reset permanent.
#[test]
fn a_subtree_that_has_been_round_tripped_can_still_transition() {
    let (mut doc, wrap, boxed) = mounted_box();

    for (out_at, in_at, class, expect) in [
        (801.0_f32, 802.0_f32, "w--b", 30.0_f32),
        (803.0, 804.0, "w--a", 20.0),
    ] {
        doc.remove_node(boxed);
        doc.set_attribute(wrap, "class", class);
        doc.resolve_layout(out_at, 600.0);
        doc.append_child(wrap, boxed);
        doc.resolve_layout(in_at, 600.0);
        assert_eq!(running(&doc, boxed), 0, "{class}: no transition on return");
        assert_eq!(width_px(&doc, boxed), Some(expect), "{class}: arrived");
        assert!(
            styled(&doc, boxed),
            "{class}: and re-armed by that resolution"
        );
    }

    // Nothing detached this time. This one must animate.
    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(805.0, 600.0);
    assert_eq!(
        running(&doc, boxed),
        1,
        "an ordinary change after a round trip still transitions"
    );
}

/// **The reactive branch helpers reach this fix too** (issue #704 closed the
/// gap).
///
/// This fixture used to be a counter-oracle. `show_dom`, `match_dom` and
/// `for_each_dom_typed` called `NodeHandle::clear_animations()` immediately
/// before `remove()`, which stamped a literal inline
/// `transition: none; animation: none` on every node of the subtree and never
/// took it off again — so a branch hidden once could not transition on its way
/// back **or ever after**, and through one of those helpers the mutant that
/// removes this whole fix still passed. That hammer is gone: all five of its
/// call sites were `clear_animations(); remove();`, and `NodeHandle::remove` is
/// `DomDocument::remove_node`, which is the first row of the table in
/// `detach_subtree_styles`' own doc.
///
/// So the same sequence now measures the real cure. The box goes out under
/// `.w--a`, the ancestor changes to `.w--b` while it is out, and it comes back
/// at 30px with nothing running — for the reason every fixture above states,
/// not because CSS was written over it. The last two steps are #704's own
/// defect read as a positive: an ordinary in-document class change afterwards
/// **animates**.
///
/// Keep the `style` assertions. They are what says the snap came from the flag
/// reset rather than from a stamp, and restoring the inline write in `show_dom`
/// alone fails this fixture on the first of them.
/// `branch_helper_transition_tests` covers the other four call sites and the
/// per-helper mutants; this one stays here because it is the bridge between the
/// two files — the fixture that says #699's cure is what the reactive routes
/// now rest on.
#[test]
fn the_reactive_branch_helpers_reach_this_fix_too() {
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    doc.borrow_mut().load_css(CSS);
    doc.borrow_mut().tree.transitions_enabled = true;
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();

    let mut scope = RenderScope::new(dyn_doc, body);
    let wrap = scope.create_element("div");
    wrap.set_attribute("class", "w--a");
    scope.parent().append_child(&wrap);

    let built: NodeHandle = {
        let panel = scope.create_element("div");
        let boxed = scope.create_element("div");
        boxed.set_attribute("class", "box");
        panel.append_child(&boxed);
        panel
    };
    let boxed_id = NodeId(built.children()[0].node_id().0);

    let showing = Signal::new(true);
    let then_branch = built.clone();
    let _marker = rinch_core::show::show_dom(
        &mut scope,
        &wrap,
        move || showing.get(),
        move |_: &mut RenderScope| then_branch.clone(),
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    doc.borrow_mut().resolve_layout(800.0, 600.0);
    assert_eq!(
        width_px(&doc.borrow(), boxed_id),
        Some(20.0),
        "precondition: the branch is mounted under .w--a"
    );
    assert_eq!(
        doc.borrow().get_attribute(boxed_id, "style"),
        None,
        "precondition: nothing inline on the box while it is mounted"
    );

    showing.set(false);
    doc.borrow_mut().resolve_layout(801.0, 600.0);
    assert_eq!(
        doc.borrow().get_attribute(boxed_id, "style"),
        None,
        "hiding a branch cancels its transitions at the detach, without \
         writing anything onto the element"
    );
    assert!(
        !styled(&doc.borrow(), boxed_id),
        "and it is the flag that was cleared — this is the same reset every \
         fixture above drives through the raw API"
    );

    wrap.set_attribute("class", "w--b");
    doc.borrow_mut().resolve_layout(802.0, 600.0);
    showing.set(true);
    doc.borrow_mut().resolve_layout(803.0, 600.0);
    assert_eq!(
        width_px(&doc.borrow(), boxed_id),
        Some(30.0),
        "the branch arrives at its new width rather than approaching it"
    );
    assert_eq!(running(&doc.borrow(), boxed_id), 0);

    // #704's defect, read as a positive: an ordinary in-document class change,
    // nothing detached, must animate.
    wrap.set_attribute("class", "w--a");
    doc.borrow_mut().resolve_layout(804.0, 600.0);
    assert_eq!(
        doc.borrow().get_attribute(boxed_id, "style"),
        None,
        "still nothing inline to disarm it"
    );
    assert_eq!(
        running(&doc.borrow(), boxed_id),
        1,
        "a branch that was hidden once can transition again (#704)"
    );
    assert_ne!(
        width_px(&doc.borrow(), boxed_id),
        Some(20.0),
        "it crawls toward 20 from 30, where the stamped branch snapped"
    );
}
