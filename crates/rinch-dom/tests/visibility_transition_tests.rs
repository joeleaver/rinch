//! `visibility` transitions, and a transition whose property stops matching
//! (issues #759, #693, and through them #413).
//!
//! # What was wrong
//!
//! `TransitionProperty` had no `visibility` variant, so a declared
//! `transition: visibility …` was dropped at extraction and the property
//! flipped discretely on the pass that changed it. An overlay could therefore
//! animate *in* — its hidden state is `visibility: hidden`, which is rendered,
//! so the panel inside it transitions (#751) — and never *out*: the root went
//! `visibility: hidden` in the same pass the panel retargeted, and the slide
//! ran for 300ms behind a box nobody could see. A browser, given the same CSS,
//! keeps the box visible for the length of the transition — css-transitions-1
//! / css-values-4 interpolate `visibility` as a discrete step in which **every
//! progress strictly between 0 and 1 is `visible`** when either end is — so the
//! close animated in Chrome and snapped on desktop.
//!
//! # The three things this file pins
//!
//! 1. `visibility` transitions, with that discrete rule (not a 50% flip, which
//!    is what a plain discrete property does — the fixtures sample at 0.9 and
//!    0.1 of the way, where the two rules disagree).
//! 2. The transitioned value is **inherited**. `visibility` inherits, and a
//!    browser inherits the *animated* value; rinch's descendants take their
//!    style from Stylo, which knows nothing of rinch's transitions, so without
//!    propagation a panel under a root that is still visible would vanish on
//!    the first frame of the close. A descendant that declares its own
//!    `visibility` is not reached — it does not inherit.
//! 3. css-transitions-1 §3 item 3 (#693): a running transition whose property
//!    no longer matches `transition-property` is **cancelled**. That is what
//!    makes the canonical `transition: visibility 0s linear 300ms`-on-the-hidden-
//!    state pattern safe to reopen mid-close: the open state declares no
//!    visibility transition, so the delayed one is cancelled rather than left to
//!    hide the overlay 300ms later.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::VisibilityValue;
use rinch_dom::transition::TransitionProperty;

/// Every size is declared, so no assertion rests on a font metric.
///
/// `.fade` transitions visibility both ways over 300ms linear — so progress is
/// elapsed/300 and a sample at 270ms is progress 0.9.
///
/// `.delayed.shut` is the canonical overlay pattern: the hidden state carries
/// `visibility 0s linear 300ms`, the shown state carries no visibility
/// transition at all.
const CSS: &str = "
    div { font-size: 16px; line-height: 20px; }
    .root  { width: 200px; height: 100px; }
    .fade  { transition: visibility 300ms linear; }
    .shut  { visibility: hidden; }
    .delayed.shut { transition: visibility 0s linear 300ms; }
    .own-hidden { visibility: hidden; }
    .own-visible { visibility: visible; }
    .kid { width: 50px; height: 20px; }
    .kid.tall { height: 30px; }
    .box { width: 10px; height: 10px; }
    .box.anim { transition: width 150ms linear; }
    .box.wide { width: 40px; }
";

fn vis(doc: &RinchDocument, node: NodeId) -> VisibilityValue {
    doc.tree.get(node.0).unwrap().computed_style.visibility
}

fn running(doc: &RinchDocument, node: NodeId) -> usize {
    doc.tree
        .active_transitions
        .get(&node.0)
        .map(|m| m.len())
        .unwrap_or(0)
}

fn start_of(doc: &RinchDocument, node: NodeId, prop: TransitionProperty) -> f64 {
    doc.tree.active_transitions[&node.0][&prop].start_time_ms
}

fn tick(doc: &mut RinchDocument, at_ms: f64) {
    rinch_dom::transition::tick_transitions(&mut doc.tree, at_ms);
}

/// `body > div.root.<class> > [div.kid > div.kid(grandchild)], div.kid.own-hidden`
///
/// Returns `(doc, root, child, grandchild, own_hidden)`.
fn mounted(class: &str) -> (RinchDocument, NodeId, NodeId, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let root = doc.create_element("div");
    doc.set_attribute(root, "class", class);
    doc.append_child(body, root);
    let child = doc.create_element("div");
    doc.set_attribute(child, "class", "kid");
    doc.append_child(root, child);
    let grandchild = doc.create_element("div");
    doc.set_attribute(grandchild, "class", "kid");
    doc.append_child(child, grandchild);
    let text = doc.create_text("label");
    doc.append_child(grandchild, text);
    let own_hidden = doc.create_element("div");
    doc.set_attribute(own_hidden, "class", "kid own-hidden");
    doc.append_child(root, own_hidden);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    (doc, root, child, grandchild, own_hidden)
}

// ── 1. visibility transitions, with the discrete-with-a-twist rule ──────────

/// Closing: `visible → hidden` over 300ms linear stays visible until the end.
///
/// Sampled at 270ms, progress 0.9 — where a plain discrete property (flip at
/// 50%) would already read `hidden`, so this cannot pass by the generic rule.
#[test]
fn a_visibility_transition_stays_visible_until_it_ends() {
    let (mut doc, root, ..) = mounted("root fade");
    assert_eq!(vis(&doc, root), VisibilityValue::Visible, "precondition");

    doc.set_attribute(root, "class", "root fade shut");
    doc.resolve_layout(801.0, 600.0);

    assert_eq!(
        running(&doc, root),
        1,
        "`transition: visibility` starts a transition (#759)"
    );
    assert_eq!(
        vis(&doc, root),
        VisibilityValue::Visible,
        "at t = 0 the closing box is still visible"
    );

    let start = start_of(&doc, root, TransitionProperty::Visibility);
    tick(&mut doc, start + 270.0);
    assert_eq!(
        vis(&doc, root),
        VisibilityValue::Visible,
        "at progress 0.9 the box is still visible — any progress strictly \
         between 0 and 1 is `visible` when one end is"
    );

    tick(&mut doc, start + 301.0);
    assert_eq!(
        vis(&doc, root),
        VisibilityValue::Hidden,
        "once the transition ends the box takes its after-change value"
    );
    assert_eq!(running(&doc, root), 0, "and the transition is gone");
}

/// Opening: `hidden → visible` is visible as soon as progress leaves 0.
///
/// Sampled at 30ms, progress 0.1 — where a 50% flip would still read `hidden`.
#[test]
fn an_opening_visibility_transition_is_visible_from_the_first_step() {
    let (mut doc, root, ..) = mounted("root fade shut");
    assert_eq!(vis(&doc, root), VisibilityValue::Hidden, "precondition");

    doc.set_attribute(root, "class", "root fade");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(running(&doc, root), 1, "the opening transition runs");

    let start = start_of(&doc, root, TransitionProperty::Visibility);
    tick(&mut doc, start + 30.0);
    assert_eq!(
        vis(&doc, root),
        VisibilityValue::Visible,
        "at progress 0.1 the opening box is already visible"
    );
}

// ── 2. the animated value is inherited ──────────────────────────────────────

/// The children of a closing box stay visible with it, and vanish with it.
///
/// Without propagation the child takes Stylo's `hidden` on the close pass while
/// its parent reads `visible`: the panel of a closing drawer would disappear on
/// the first frame and leave an empty, visible root sliding nothing.
#[test]
fn descendants_inherit_the_transitioning_visibility() {
    let (mut doc, root, child, grandchild, _) = mounted("root fade");

    doc.set_attribute(root, "class", "root fade shut");
    doc.resolve_layout(801.0, 600.0);

    assert_eq!(
        vis(&doc, child),
        VisibilityValue::Visible,
        "the child inherits the root's still-visible value"
    );
    assert_eq!(
        vis(&doc, grandchild),
        VisibilityValue::Visible,
        "and so does the grandchild, through the child"
    );

    let start = start_of(&doc, root, TransitionProperty::Visibility);
    tick(&mut doc, start + 270.0);
    assert_eq!(vis(&doc, grandchild), VisibilityValue::Visible, "mid-way");

    tick(&mut doc, start + 301.0);
    assert_eq!(
        vis(&doc, child),
        VisibilityValue::Hidden,
        "when the root's transition ends, its children go with it"
    );
    assert_eq!(vis(&doc, grandchild), VisibilityValue::Hidden);
}

/// A descendant that declares its own `visibility` does not inherit, so the
/// root's transition does not reach it.
///
/// The case that matters in practice: a closed `Popover` inside a closing
/// `Drawer` — its dropdown declares `visibility: hidden` and must not flash
/// into view for the drawer's 300ms.
#[test]
fn a_descendant_with_its_own_visibility_is_not_reached() {
    let (mut doc, root, child, _, own_hidden) = mounted("root fade");
    assert_eq!(vis(&doc, own_hidden), VisibilityValue::Hidden, "precondition");

    doc.set_attribute(root, "class", "root fade shut");
    doc.resolve_layout(801.0, 600.0);

    assert_eq!(
        vis(&doc, child),
        VisibilityValue::Visible,
        "positive control: an inheriting sibling IS reached"
    );
    assert_eq!(
        vis(&doc, own_hidden),
        VisibilityValue::Hidden,
        "a box that declares `visibility: hidden` stays hidden"
    );
}

/// A descendant restyled while the root's transition runs still inherits the
/// animated value — the propagation is re-applied after every cascade, not
/// only when the transition starts.
#[test]
fn a_descendant_restyled_mid_transition_keeps_inheriting() {
    let (mut doc, root, child, _, _) = mounted("root fade");

    doc.set_attribute(root, "class", "root fade shut");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(vis(&doc, child), VisibilityValue::Visible, "precondition");

    doc.set_attribute(child, "class", "kid tall");
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(
        vis(&doc, child),
        VisibilityValue::Visible,
        "the restyled child re-cascades to Stylo's `hidden` and must be \
         brought back to the root's animated value"
    );
}

// ── 3. §3 item 3: a transition whose property stops matching is cancelled ───

/// The canonical overlay pattern: close with a delayed `visibility`, reopen
/// before the delay ends.
///
/// The open state declares no visibility transition, so css-transitions-1 §3
/// item 3 cancels the running one. Without the cancel it stays in the map and,
/// 300ms after the close, hides a box the user has just reopened.
#[test]
fn reopening_during_a_delayed_close_cancels_it() {
    let (mut doc, root, child, ..) = mounted("root delayed");

    doc.set_attribute(root, "class", "root delayed shut");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(running(&doc, root), 1, "the delayed close is running");
    assert_eq!(
        vis(&doc, root),
        VisibilityValue::Visible,
        "and holds the box visible through its delay"
    );
    let start = start_of(&doc, root, TransitionProperty::Visibility);

    tick(&mut doc, start + 100.0);
    doc.set_attribute(root, "class", "root delayed");
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(
        running(&doc, root),
        0,
        "reopening cancels the close — its property no longer matches any \
         `transition-property` (#693)"
    );

    tick(&mut doc, start + 400.0);
    assert_eq!(
        vis(&doc, root),
        VisibilityValue::Visible,
        "and the reopened box is still visible after the close would have ended"
    );
    assert_eq!(vis(&doc, child), VisibilityValue::Visible);
}

/// #693's own measurement: a width transition whose `transition-property` is
/// removed part way through lands on its target and stays there.
///
/// At HEAD this read 40px on the restyle and jumped back to an interpolated
/// width on the next tick.
#[test]
fn a_transition_whose_property_stops_matching_is_cancelled() {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let boxed = doc.create_element("div");
    doc.set_attribute(boxed, "class", "box anim");
    doc.append_child(body, boxed);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);

    doc.set_attribute(boxed, "class", "box anim wide");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(running(&doc, boxed), 1, "precondition: the width transition runs");
    let start = start_of(&doc, boxed, TransitionProperty::Width);
    tick(&mut doc, start + 50.0);

    doc.set_attribute(boxed, "class", "box wide");
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(running(&doc, boxed), 0, "the transition is cancelled");

    tick(&mut doc, start + 75.0);
    let w = match doc.tree.get(boxed.0).unwrap().computed_style.width {
        rinch_dom::computed_style::DimensionValue::Length(px) => px,
        other => panic!("width is not a length: {other:?}"),
    };
    assert_eq!(w, 40.0, "and the box stays at its target, got {w}px");
}
