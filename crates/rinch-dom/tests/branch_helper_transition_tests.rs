//! #704 — the reactive branch helpers no longer disarm a subtree permanently.
//!
//! `NodeHandle::clear_animations()` wrote two **inline** declarations,
//! `transition: none` and `animation: none`, over every node of a subtree, and
//! nothing ever took them off again. Five call sites ran it immediately before
//! `remove()`: `show_dom`, `match_dom`, `for_each_dom_typed`'s `Remove` arm and
//! `reclaim_displaced`, and the component re-render effect.
//!
//! For the four of those that discard the subtree it was merely redundant. For
//! a branch helper that keeps a `NodeHandle` and re-inserts the **same**
//! subtree — a reactive `if` whose branch closure returns a captured handle,
//! which is the shape #654 was reported from — it was permanent: the subtree
//! came back carrying `style="transition: none; animation: none"` for the rest
//! of the session, so an element hidden once could never animate again, for any
//! later change, related to the toggle or not.
//!
//! The cure is to stop writing it. What `clear_animations` was reaching for is
//! now done properly at the detach, by the document implementation rather than
//! by a caller stamping CSS: `RinchDocument::detach_subtree_styles` (#699)
//! clears `has_been_styled` and drops `active_transitions` and
//! `active_animations` for the whole removed subtree, and **all five call sites
//! funnel into it**, because each one is `clear_animations(); remove();` and
//! `NodeHandle::remove` is `DomDocument::remove_node`. On `rinch-web` the
//! browser does the same thing for free — a removed element is not rendered, so
//! its transitions are cancelled and its re-insertion is a first style.
//!
//! # Mutants, and what kills each
//!
//! Every attribution below is **measured**: the mutant was applied to the
//! committed source, this file plus `reinsertion_transition_tests` run against
//! it, and the source reverted.
//!
//! | mutant | killed by |
//! |---|---|
//! | restore the inline write in `show_dom` | `a_branch_hidden_once_can_still_transition_afterwards`, `a_reinserted_show_branch_snaps_to_its_new_style`, `a_transition_running_when_a_branch_is_hidden_does_not_resume` |
//! | restore it in `match_dom` only | `a_match_branch_switched_twice_can_still_transition`, alone |
//! | restore it in `for_each_dom_typed`'s `Remove` arm only | `a_for_row_reinserted_under_the_same_key_can_still_transition`, alone |
//! | restore it in the component re-render effect only | `a_rerendered_component_subtree_can_still_transition`, alone |
//! | remove the write **and** #699's `detach_subtree_styles` | `a_reinserted_show_branch_snaps_to_its_new_style`, `a_transition_running_when_a_branch_is_hidden_does_not_resume`, and most of `reinsertion_transition_tests` |
//!
//! The last row is the one that matters for reading this file as a whole. The
//! inline write was a hammer that hid #699's defect from every reactive route:
//! with it in place, deleting `detach_subtree_styles` changed nothing a branch
//! helper could see. Taking the hammer away is what puts those routes on the
//! real cure, so the fixtures below pin **both** halves — that a re-inserted
//! branch does not animate in from a style the user never saw, and that it can
//! still animate afterwards.
//!
//! `reclaim_displaced` (`for_loop.rs`) is the one of the five call sites with no
//! fixture here, and it is unreachable rather than untested: it runs only when
//! `prepare_keys` has already let a duplicate key through, which is guarded by a
//! `debug_assert!(clobbered.is_none())` one line above the call, so a debug test
//! panics on the assertion before it can get there. Its subtree is discarded and
//! unreachable afterwards, which is why it was harmless.

#![cfg(feature = "software-renderer")]

use std::cell::RefCell;
use std::collections::HashMap;
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
";

/// The node's computed `width` in px, or `None` when it is not a length.
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

/// The node's inline `style` attribute.
///
/// This is the assertion the whole file is built around: the defect was a
/// *literal string* left on the element, so the fixtures read it back rather
/// than inferring it from behaviour.
fn inline_style(doc: &RinchDocument, node: NodeId) -> Option<String> {
    doc.get_attribute(node, "style")
}

/// A document with [`CSS`] loaded, transitions armed, and a `.w--a` wrapper
/// under `<body>` for a branch helper to mount into.
///
/// Returns `(doc, scope, wrapper)`. The `RenderScope` is the caller's to build
/// with; the document is shared, so every read below goes through `borrow()`.
fn harness() -> (Rc<RefCell<RinchDocument>>, RenderScope, NodeHandle) {
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    doc.borrow_mut().load_css(CSS);
    doc.borrow_mut().tree.transitions_enabled = true;
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();

    let mut scope = RenderScope::new(dyn_doc, body);
    let wrap = scope.create_element("div");
    wrap.set_attribute("class", "w--a");
    scope.parent().append_child(&wrap);
    (doc, scope, wrap)
}

/// A `div.box` inside a `div`, built through `scope`, returned as the panel.
fn build_panel(scope: &mut RenderScope) -> NodeHandle {
    let panel = scope.create_element("div");
    let boxed = scope.create_element("div");
    boxed.set_attribute("class", "box");
    panel.append_child(&boxed);
    panel
}

/// `match_dom`'s branch type, which `rinch-core` keeps private.
type BranchFn = Box<dyn Fn(&mut RenderScope) -> NodeHandle>;

/// The `div.box` inside a panel built by [`build_panel`].
fn box_of(panel: &NodeHandle) -> NodeId {
    NodeId(panel.children()[0].node_id().0)
}

/// **The issue's own table, on the route it was reported from.**
///
/// A reactive `if` whose branch closure returns a captured `NodeHandle`: hide
/// it once, and at `main` `c717500` the box came back wearing
/// `style="transition: none; animation: none"` permanently. The last step is
/// the defect — an ordinary class change with nothing detached, months of user
/// interaction later, which snapped instead of animating.
///
/// | step | inline `style` at `c717500` | with this fix |
/// |---|---|---|
/// | mounted | `None` | `None` |
/// | hidden | `transition: none; animation: none` | `None` |
/// | shown again under a changed ancestor | unchanged | `None` |
/// | a later ordinary class change | snaps, 0 running | animates, 1 running |
///
/// Each `resolve_layout` uses a different viewport on purpose: it early-returns
/// on `!layout_dirty`, so re-resolving at the same size can test nothing.
///
/// Kills the "restore the inline write in `show_dom`" mutant.
#[test]
fn a_branch_hidden_once_can_still_transition_afterwards() {
    let (doc, mut scope, wrap) = harness();
    let built = build_panel(&mut scope);
    let boxed = box_of(&built);

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
        width_px(&doc.borrow(), boxed),
        Some(20.0),
        "precondition: the branch is mounted under .w--a"
    );
    assert_eq!(
        inline_style(&doc.borrow(), boxed),
        None,
        "precondition: nothing inline on the box while it is mounted"
    );

    showing.set(false);
    doc.borrow_mut().resolve_layout(801.0, 600.0);
    assert_eq!(
        inline_style(&doc.borrow(), boxed),
        None,
        "hiding a branch must not stamp CSS on it — the detach cancels its \
         transitions without touching the style attribute"
    );

    wrap.set_attribute("class", "w--b");
    doc.borrow_mut().resolve_layout(802.0, 600.0);
    showing.set(true);
    doc.borrow_mut().resolve_layout(803.0, 600.0);
    assert_eq!(
        inline_style(&doc.borrow(), boxed),
        None,
        "and re-showing must not have found one to leave behind"
    );

    // The row that made this a defect: nothing is detached here, and this
    // change has nothing to do with the toggle.
    wrap.set_attribute("class", "w--a");
    doc.borrow_mut().resolve_layout(804.0, 600.0);
    assert_eq!(
        running(&doc.borrow(), boxed),
        1,
        "an ordinary in-document class change after a hide/show must animate"
    );
    assert_ne!(
        width_px(&doc.borrow(), boxed),
        Some(20.0),
        "it crawls toward 20 from 30, it does not snap"
    );
}

/// The other half, and the reason the hammer could not simply be left in place:
/// with `clear_animations` gone, a re-shown branch is on #699's real cure.
///
/// Hide the branch, change the ancestor's class while it is out, show it again.
/// It must arrive at 30px with nothing running — a returning subtree has no
/// before-change style, so there is nothing to animate from.
///
/// This is `reinsertion_transition_tests`'
/// `a_reinserted_subtree_does_not_animate_from_its_pre_detach_style` driven
/// through the reactive helper instead of the raw `DomDocument` API. That
/// fixture could not be written this way before: the inline write suppressed
/// the animation whether or not #699's fix existed, so the helper routes were
/// blind to it.
///
/// Kills the "restore the inline write in `show_dom`" mutant (it snaps for the
/// wrong reason, and the inline assertion catches that) and the "remove the
/// write and `detach_subtree_styles` too" mutant (it animates).
#[test]
fn a_reinserted_show_branch_snaps_to_its_new_style() {
    let (doc, mut scope, wrap) = harness();
    let built = build_panel(&mut scope);
    let boxed = box_of(&built);

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
    assert_eq!(width_px(&doc.borrow(), boxed), Some(20.0), "precondition");

    showing.set(false);
    wrap.set_attribute("class", "w--b");
    doc.borrow_mut().resolve_layout(801.0, 600.0);

    showing.set(true);
    doc.borrow_mut().resolve_layout(802.0, 600.0);

    assert_eq!(
        running(&doc.borrow(), boxed),
        0,
        "a returning subtree has no before-change style"
    );
    assert_eq!(
        width_px(&doc.borrow(), boxed),
        Some(30.0),
        "it arrives at the new value rather than approaching it"
    );
    assert_eq!(
        inline_style(&doc.borrow(), boxed),
        None,
        "and it got there without any CSS being stamped on it"
    );
}

/// A transition **already running** when the branch is hidden must not resume
/// from its interpolated value when the branch comes back.
///
/// This was #699's guarantee and it was unobservable through a reactive helper,
/// because `clear_animations` stopped the interpolation by a different route.
/// Now that the hammer is gone, `detach_subtree_styles` dropping the node's
/// `active_transitions` entry is the only thing standing between the user and a
/// box that resumes crawling from 24.3px.
///
/// The start value matters: the box is caught mid-flight between 20 and 30, so
/// a resumed transition would land on neither endpoint. That is the
/// fixed-point check — a fixture that hid the branch at rest would pass against
/// a mutant that resumed, because resuming from 20 and starting fresh at 20
/// look identical.
///
/// This is the one fixture in the file whose *behavioural* assertions were
/// already green at `main` `c717500`: the hammer stopped the transition too,
/// just by stamping CSS. The `inline_style` assertion is what tells the two
/// apart, and it is why the check is here rather than left implicit.
#[test]
fn a_transition_running_when_a_branch_is_hidden_does_not_resume() {
    let (doc, mut scope, wrap) = harness();
    let built = build_panel(&mut scope);
    let boxed = box_of(&built);

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

    // Start a real 20 → 30 transition and leave it mid-flight.
    wrap.set_attribute("class", "w--b");
    doc.borrow_mut().resolve_layout(801.0, 600.0);
    assert_eq!(
        running(&doc.borrow(), boxed),
        1,
        "precondition: a width transition is in flight"
    );

    showing.set(false);
    assert_eq!(
        running(&doc.borrow(), boxed),
        0,
        "hiding the branch cancels it — a transition belongs to a rendered \
         element"
    );
    assert_eq!(
        inline_style(&doc.borrow(), boxed),
        None,
        "and it was cancelled by the detach, not by CSS stamped on the element"
    );
    doc.borrow_mut().resolve_layout(802.0, 600.0);

    showing.set(true);
    doc.borrow_mut().resolve_layout(803.0, 600.0);
    assert_eq!(
        running(&doc.borrow(), boxed),
        0,
        "and it does not pick up where it left off"
    );
    assert_eq!(
        width_px(&doc.borrow(), boxed),
        Some(30.0),
        "the box is at its declared width, not at the value it was passing \
         through when it was hidden"
    );
}

/// `match_dom`, switched away and back twice, can still transition.
///
/// Two round trips rather than one, for the same reason
/// `a_subtree_that_has_been_round_tripped_can_still_transition` does it: one
/// says the reset happened, two say it is idempotent rather than a one-shot
/// that a second switch corrupts.
///
/// Kills the "restore the inline write in `match_dom`" mutant, and it is the
/// only fixture that does — `show_dom` and `match_dom` have separate copies of
/// the same three lines.
#[test]
fn a_match_branch_switched_twice_can_still_transition() {
    let (doc, mut scope, wrap) = harness();
    let built = build_panel(&mut scope);
    let boxed = box_of(&built);

    let which = Signal::new(0usize);
    let arm0 = built.clone();
    let branches: Vec<BranchFn> = vec![
        Box::new(move |_: &mut RenderScope| arm0.clone()),
        Box::new(|s: &mut RenderScope| s.create_element("span")),
    ];
    let _marker =
        rinch_core::match_dom::match_dom(&mut scope, &wrap, move || which.get(), branches);
    doc.borrow_mut().resolve_layout(800.0, 600.0);
    assert_eq!(width_px(&doc.borrow(), boxed), Some(20.0), "precondition");

    let mut viewport = 801.0_f32;
    for _ in 0..2 {
        which.set(1);
        doc.borrow_mut().resolve_layout(viewport, 600.0);
        assert_eq!(
            inline_style(&doc.borrow(), boxed),
            None,
            "switching away must not stamp CSS on the branch it left"
        );
        viewport += 1.0;
        which.set(0);
        doc.borrow_mut().resolve_layout(viewport, 600.0);
        viewport += 1.0;
    }

    // Nothing switched this time.
    wrap.set_attribute("class", "w--b");
    doc.borrow_mut().resolve_layout(viewport, 600.0);
    assert_eq!(
        running(&doc.borrow(), boxed),
        1,
        "an ordinary change after two round trips still transitions"
    );
}

/// `for_each_dom_typed`'s `Remove` arm, on a list whose view function returns a
/// memoised row.
///
/// A plain `for` builds a fresh node for every row, so the stamp had nowhere to
/// persist — which is exactly why this defect went unnoticed on the list route.
/// A view that caches its rows by key (the same trick that makes a reactive
/// `if` re-insert its captured branch) puts the removed node straight back, and
/// at `main` it came back disarmed.
///
/// Kills the "restore the inline write in the `Remove` arm" mutant, and it is
/// the only fixture that does.
#[test]
fn a_for_row_reinserted_under_the_same_key_can_still_transition() {
    let (doc, mut scope, wrap) = harness();

    let cache: Rc<RefCell<HashMap<u32, NodeHandle>>> = Rc::new(RefCell::new(HashMap::new()));
    let rows = Signal::new(vec![1u32]);

    let view_cache = cache.clone();
    let _marker = rinch_core::for_loop::for_each_dom_typed(
        &mut scope,
        &wrap,
        move || rows.get(),
        |id: &u32| id.to_string(),
        move |id: u32, s: &mut RenderScope| {
            let cached = view_cache.borrow().get(&id).cloned();
            cached.unwrap_or_else(|| {
                let panel = build_panel(s);
                view_cache.borrow_mut().insert(id, panel.clone());
                panel
            })
        },
    );
    doc.borrow_mut().resolve_layout(800.0, 600.0);
    let boxed = box_of(&cache.borrow()[&1]);
    assert_eq!(width_px(&doc.borrow(), boxed), Some(20.0), "precondition");

    rows.set(vec![]);
    doc.borrow_mut().resolve_layout(801.0, 600.0);
    assert_eq!(
        inline_style(&doc.borrow(), boxed),
        None,
        "removing a row must not stamp CSS on the node it removed"
    );

    rows.set(vec![1]);
    doc.borrow_mut().resolve_layout(802.0, 600.0);

    wrap.set_attribute("class", "w--b");
    doc.borrow_mut().resolve_layout(803.0, 600.0);
    assert_eq!(
        running(&doc.borrow(), boxed),
        1,
        "a row that left the list and came back can still animate"
    );
}

/// The component re-render effect (`reactive_component_dom`).
///
/// Its removal arm is the fifth call site, and it re-renders rather than
/// re-inserts, so like the `for` route it needed a memoised render function to
/// expose the stamp. That is not a contrived shape: a `#[component]` whose
/// `render` hands back a handle it built once is exactly what the `for` fixture
/// above caches, and it is the pattern #654 was reported from.
///
/// Kills the "restore the inline write in the component re-render effect"
/// mutant, and it is the only fixture that does.
#[test]
fn a_rerendered_component_subtree_can_still_transition() {
    let (doc, mut scope, wrap) = harness();
    let built = build_panel(&mut scope);
    let boxed = box_of(&built);

    let version = Signal::new(0u32);
    let memoised = built.clone();
    let _marker = rinch_core::dom::reactive_component_dom(&mut scope, &wrap, move |_| {
        // Read the signal so the effect re-runs, but hand back the same subtree.
        let _ = version.get();
        memoised.clone()
    });
    doc.borrow_mut().resolve_layout(800.0, 600.0);
    assert_eq!(width_px(&doc.borrow(), boxed), Some(20.0), "precondition");

    version.set(1);
    doc.borrow_mut().resolve_layout(801.0, 600.0);
    assert_eq!(
        inline_style(&doc.borrow(), boxed),
        None,
        "a re-render must not stamp CSS on the subtree it swapped out"
    );

    wrap.set_attribute("class", "w--b");
    doc.borrow_mut().resolve_layout(802.0, 600.0);
    assert_eq!(
        running(&doc.borrow(), boxed),
        1,
        "a re-rendered component subtree can still animate"
    );
}

/// A spinner inside a hidden branch stops asking the shell for frames.
///
/// A transition self-limits — it has a declared duration and dies after it. A
/// `@keyframes` animation does not, and the desktop shell decides whether to
/// schedule another frame from `!tree.active_animations.is_empty()`
/// (`rinch/src/app/event_dispatch.rs`), so a `Loader` left running on a removed
/// node keeps an app rendering at full rate with nothing on screen to show for
/// it.
///
/// `reinsertion_transition_tests::a_detached_animation_stops_asking_for_frames`
/// pins that for the raw `DomDocument` API. This fixture is the reactive-route
/// twin, and it exists because the property's *witness* changed rather than the
/// mechanism: the inline `animation: none` that `clear_animations` stamped was
/// a second, independent thing stopping the frames on exactly these three
/// helpers, and with it gone `detach_subtree_styles` dropping
/// `active_animations` is the only one left. A `Loader` in a reactive `if` is
/// the commonest shape in the component library, so the claim wants its own
/// pin rather than an inference from a fixture that drives `remove_node`
/// directly.
///
/// Kills a mutant that drops `active_transitions` but not `active_animations`,
/// which every other fixture in this file is happy with — a transition's
/// 150ms bound hides it.
#[test]
fn a_spinner_in_a_hidden_branch_stops_asking_for_frames() {
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    doc.borrow_mut().load_css(
        "@keyframes sp { from { width: 10px; } to { width: 100px; } } \
         .spin { animation: sp 1000s linear infinite; width: 10px; height: 10px; \
                 font-size: 16px; line-height: 20px; }",
    );
    doc.borrow_mut().tree.transitions_enabled = true;
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut scope = RenderScope::new(dyn_doc, body);
    let wrap = scope.create_element("div");
    scope.parent().append_child(&wrap);

    let spinner = scope.create_element("div");
    spinner.set_attribute("class", "spin");
    let spinner_id = spinner.node_id();

    let showing = Signal::new(true);
    let branch = spinner.clone();
    let _marker = rinch_core::show::show_dom(
        &mut scope,
        &wrap,
        move || showing.get(),
        move |_: &mut RenderScope| branch.clone(),
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    doc.borrow_mut().resolve_layout(800.0, 600.0);
    assert!(
        !doc.borrow().tree.active_animations.is_empty(),
        "precondition: the shell is being asked for frames"
    );

    showing.set(false);
    assert!(
        doc.borrow().tree.active_animations.is_empty(),
        "hiding the branch must stop the frames — the animation has no duration \
         to expire and nothing else ever clears it"
    );
    assert!(
        !doc.borrow_mut().tick_animations(),
        "and the tick must agree there is nothing left to advance"
    );
    assert_eq!(
        inline_style(&doc.borrow(), spinner_id),
        None,
        "stopped by the detach, not by an inline `animation: none`"
    );
}
