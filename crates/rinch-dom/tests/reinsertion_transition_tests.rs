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
//! `RinchDocument::detach_subtree_styles`, called from the three routes by
//! which a subtree leaves the document.
//!
//! # Mutants, and what kills each
//!
//! Every attribution below is **measured** — each mutant was applied to the
//! committed source, the whole file run against it, and the source reverted.
//!
//! | mutant | killed by |
//! |---|---|
//! | no reset at all (`main` at `cbdfc5a`) | 6 of the 11 — everything but the two move fixtures, the unchanged-ancestor control, the `display: none` pin and the `clear_animations` counter-oracle |
//! | reset the detach root only, not the subtree | `a_deep_node_in_a_reinserted_subtree_does_not_animate_either`, **alone** |
//! | reset `has_been_styled`, leave `active_transitions` | `a_transition_running_when_the_subtree_is_detached_does_not_resume`, **alone** |
//! | reset in `remove_node` only | `remove_child_is_a_detach_too`, `the_subtree_a_replace_displaces_is_a_detach_too`, `a_detached_subtree_keeps_the_style_it_last_had` |
//! | also clear `computed_style` and `text_layout` in the reset | `a_detached_subtree_keeps_the_style_it_last_had` here, **and four fixtures in `detached_style_roots_tests`** |
//! | also reset on a reparenting `append_child`/`insert_before`/`insert_child` (over-reach) | `a_reparenting_move_does_not_restart_a_running_transition` and `a_keyed_for_reorder_does_not_restart_a_running_transition`, and nothing else |
//!
//! Two rows are worth reading twice.
//!
//! **The over-reach row.** A keyed `for` reorder moves rows with
//! `insert_after`, which is `insert_before`/`append_child` — the same three
//! lines that unlink a node from its old parent. Resetting there would restart
//! every mid-flight transition on every list reorder, and those two fixtures
//! are the only thing in the suite that says so.
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
//! The defect is in `rinch-dom`'s `DomDocument` implementation, so every
//! tree-mutation caller reaches it — **except** the three reactive branch
//! helpers, and not because of this fix. `show_dom`, `match_dom` and
//! `for_each_dom_typed` each call `NodeHandle::clear_animations()` before
//! `remove()`, which stamps an inline `transition: none; animation: none` on
//! the whole subtree and never takes it off again. That is a bigger hammer with
//! a defect of its own (a branch hidden once can never transition again,
//! issue #704), and it is why the fixtures below drive the DOM API directly:
//! through a reactive helper, the mutant that removes this entire fix still
//! passes. `the_reactive_branch_helpers_are_neutralised_by_clear_animations_not_by_this_fix`
//! is the measurement, and the standing note for whoever fixes #704.
//!
//! What is live today: `NodeHandle::remove_child`, `RenderScope`'s batched
//! `DomUpdate::RemoveChild`, `NodeHandle::replace_with` (which
//! `rinch-editor-view`'s `ViewDesc` diff uses), and any component that stashes
//! a `NodeHandle` and re-attaches it — the pattern #654 was reported from.
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
/// `ListOp::Move` splices the row with `insert_after`, so this is the shape the
/// over-reach mutant would break in production — every mid-flight transition in
/// a list restarted on every reorder.
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

    let row_a = rows
        .borrow()
        .iter()
        .find(|(k, _)| k == "a")
        .map(|(_, id)| *id)
        .expect("row a was rendered");
    assert_eq!(
        width_px(&doc.borrow(), row_a),
        Some(20.0),
        "precondition: the rows take their width from the wrapper"
    );

    // Put row a mid-flight, then rotate the list.
    wrap.set_attribute("class", "w--b");
    doc.borrow_mut().resolve_layout(801.0, 600.0);
    assert_eq!(
        running(&doc.borrow(), row_a),
        1,
        "precondition: row a is transitioning"
    );

    order.set(vec!["b".to_string(), "c".to_string(), "a".to_string()]);
    doc.borrow_mut().resolve_layout(802.0, 600.0);

    assert_eq!(
        rows.borrow().len(),
        3,
        "positive control: the reorder moved the rows, it did not re-render them"
    );
    assert_eq!(
        running(&doc.borrow(), row_a),
        1,
        "a moved row keeps transitioning"
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

/// **Counter-oracle: the three reactive helpers do not reach this defect, and
/// the reason is not this fix.**
///
/// `show_dom`, `match_dom` and `for_each_dom_typed` all call
/// `NodeHandle::clear_animations()` immediately before `remove()`, and that
/// stamps a literal inline `transition: none; animation: none` on every node of
/// the subtree. Nothing ever takes it off again — measured below — so a branch
/// that has been hidden once cannot transition on its way back **or ever
/// after**. That is a far blunter instrument than the flag reset, and a
/// separate defect of its own (issue #704).
///
/// It is pinned here for two reasons. It is why every fixture above drives the
/// `DomDocument` API directly instead of a reactive helper: through one of
/// them, the mutant that removes the whole fix still passes. And it is the
/// standing evidence that #699's live routes are the ones *without* that
/// hammer — `NodeHandle::remove_child`, `RenderScope`'s batched
/// `DomUpdate::RemoveChild`, `NodeHandle::replace_with` (which
/// `rinch-editor-view`'s `ViewDesc` diff uses), and any component that stashes
/// a handle and re-attaches it. If #704 is ever fixed by deleting
/// `clear_animations`, this fixture is what says #699's cure has to already be
/// in place underneath it.
#[test]
fn the_reactive_branch_helpers_are_neutralised_by_clear_animations_not_by_this_fix() {
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
        doc.borrow().get_attribute(boxed_id, "style").as_deref(),
        Some("transition: none; animation: none"),
        "the hammer: `show_dom` disables the branch's transitions inline on the \
         way out"
    );

    wrap.set_attribute("class", "w--b");
    doc.borrow_mut().resolve_layout(802.0, 600.0);
    showing.set(true);
    doc.borrow_mut().resolve_layout(803.0, 600.0);
    assert_eq!(width_px(&doc.borrow(), boxed_id), Some(30.0));
    assert_eq!(running(&doc.borrow(), boxed_id), 0);

    // And the part that makes it a defect rather than a cure: the inline
    // declaration is still there, so an ordinary in-document class change —
    // nothing detached, nothing re-inserted — cannot animate either.
    wrap.set_attribute("class", "w--a");
    doc.borrow_mut().resolve_layout(804.0, 600.0);
    assert_eq!(
        doc.borrow().get_attribute(boxed_id, "style").as_deref(),
        Some("transition: none; animation: none"),
        "nothing ever takes it off again"
    );
    assert_eq!(
        running(&doc.borrow(), boxed_id),
        0,
        "so a branch that was hidden once can never transition again (#704)"
    );
    assert_eq!(
        width_px(&doc.borrow(), boxed_id),
        Some(20.0),
        "it snaps, where an untouched box would have animated"
    );
}
