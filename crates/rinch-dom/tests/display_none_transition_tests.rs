//! #703 — a transition does not start on an element that is not being rendered.
//!
//! css-transitions-1 §3 defines the before-change style only for an element
//! that is being rendered, so no transition starts for one that is not, and any
//! transition running on one that stops being rendered is cancelled. A
//! `display: none` element is not being rendered, and neither is any descendant
//! of one — `display` is not inherited, so the descendant's own computed
//! `display` stays `block` and says nothing about it.
//!
//! rinch started them. `apply_stylo_styles_to_taffy`'s gate asked only
//! `has_been_styled`, which a hidden node keeps: hiding is not a detach (#699),
//! the node stays in the document with the `computed_style` it had when it was
//! last visible. So a control hidden in an inactive tab, restyled by a theme or
//! a breakpoint change while it was hidden, animated from its old value when
//! the tab was shown.
//!
//! # The predicate, and what it costs
//!
//! "Not rendered" is `self or any ancestor has `display: none``, and it is
//! **not** derivable from the node's own `ComputedStyle` — measured:
//! `a_descendant_of_a_hidden_ancestor_still_computes_display_block` is the pin.
//! rinch caches no rendered bit either; `read_layout_results` learns it by
//! recursion (`zero_subtree_layout`) and its own comment says so ("*ancestor has
//! display:none — the node's own display may be Block but it's inside a hidden
//! subtree*"). So the cascade walks the ancestor chain — but only at the one
//! site where the answer can change an outcome: **after** `diff_animatable` has
//! found an animatable difference on a node that declares a `transition`. A
//! node with no `transition` declaration, or with no animatable change on this
//! cascade, never walks. The walk is O(depth) beside a full property diff that
//! is already O(properties), on a set that is a small fraction of the dirty
//! nodes.
//!
//! The chain is read from `computed_style`, which for an ancestor already
//! restyled on this pass holds its **new** display — the cascade pushes parents
//! before children. A node whose old display was `none` is therefore recorded
//! in a side list as it is processed, so an ancestor shown *in the same pass*
//! that retargets its descendant is still known to have been hidden before the
//! change. That list is a `Vec` that stays empty on every pass with nothing
//! hidden in it.
//!
//! # Mutants, and what kills each
//!
//! Each was applied to the committed source, this file plus
//! `reinsertion_transition_tests` run against it, and the source reverted.
//!
//! | mutant | killed by |
//! |---|---|
//! | the gate's `is_rendered` clause deleted (i.e. `main` before this change) | `a_change_while_hidden_does_not_start_a_transition`, `a_change_under_a_hidden_ancestor_does_not_start_a_transition`, `showing_a_hidden_node_whose_target_changed_does_not_transition` |
//! | the gate tests the **new** display only, not the old one | `showing_a_hidden_node_whose_target_changed_does_not_transition`, **alone** — every other fixture asks the question while the node is still hidden, where old and new agree |
//! | the gate tests the node's **own** display only, no ancestor walk | `a_change_under_a_hidden_ancestor_does_not_start_a_transition`, **alone** |
//! | the ancestor walk reads `computed_style` with no side list of old displays | `showing_a_hidden_ancestor_whose_descendant_is_retargeted_does_not_transition`, **alone** |
//! | the cancel half deleted | `hiding_a_node_cancels_its_running_transition`, `hiding_an_ancestor_cancels_a_descendant_transition` |
//! | the cancel half does not descend (the node only) | `hiding_an_ancestor_cancels_a_descendant_transition`, **alone** |
//! | the gate refuses whenever the node is not *visible* (`visibility` folded in) | `visibility_hidden_is_rendered_and_still_transitions`, **alone** |
//!
//! The last row is why `visibility: hidden` is here at all. It is the near
//! neighbour that looks like the same thing and is not: a `visibility: hidden`
//! box is generated, laid out and rendered — it is merely invisible — so it has
//! a before-change style and its transitions run. Getting that wrong would be
//! invisible in every fixture above.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// `font-size` and `line-height` are declared so that no box below is derived
/// from a font metric, and every width is a declaration a reader can check.
///
/// `.hidden` hides the **box**; `.gone` hides the **wrapper**, which is the
/// ancestor case — the box's own `display` stays `block` under it.
const CSS: &str = "
    .box  { width: 10px; height: 10px; font-size: 16px; line-height: 20px;
            transition: width 150ms linear; }
    .w--a .box { width: 20px; }
    .w--b .box { width: 30px; }
    .hidden .box { display: none; }
    .invisible .box { visibility: hidden; }
    .gone { display: none; }
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

fn display_of(doc: &RinchDocument, node: NodeId) -> rinch_dom::computed_style::DisplayValue {
    doc.tree.get(node.0).unwrap().computed_style.display
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

/// The control the whole file rests on: a change on a **rendered** box does
/// start a transition.
///
/// Without this every "0 transitions" assertion below would also pass against a
/// build where transitions never start at all.
#[test]
fn a_change_on_a_rendered_box_does_start_a_transition() {
    let (mut doc, wrap, boxed) = mounted_box();

    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(801.0, 600.0);

    assert_eq!(
        running(&doc, boxed),
        1,
        "positive control: a visible box transitions"
    );
}

/// The issue's own table, rows 2 and 3.
///
/// Hide the box, retarget it while it is hidden. A browser starts nothing: the
/// box is not being rendered, so it has no before-change style.
#[test]
fn a_change_while_hidden_does_not_start_a_transition() {
    let (mut doc, wrap, boxed) = mounted_box();

    doc.set_attribute(wrap, "class", "w--a hidden");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(
        display_of(&doc, boxed),
        rinch_dom::computed_style::DisplayValue::None,
        "precondition: the box is hidden by its own computed display"
    );
    assert_eq!(running(&doc, boxed), 0, "precondition: nothing running");

    doc.set_attribute(wrap, "class", "w--b hidden");
    doc.resolve_layout(802.0, 600.0);

    assert_eq!(
        running(&doc, boxed),
        0,
        "a non-rendered element has no before-change style (css-transitions-1 §3)"
    );
    assert_eq!(
        width_px(&doc, boxed),
        Some(30.0),
        "and the new value is taken directly, not interpolated towards"
    );

    // Row 4: shown again, at the new value, with nothing running.
    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(803.0, 600.0);
    assert_eq!(
        running(&doc, boxed),
        0,
        "and nothing starts when it is shown"
    );
    assert_eq!(
        width_px(&doc, boxed),
        Some(30.0),
        "the box appears at 30px, the way a browser shows it"
    );
}

/// The same question asked in **one** step, which is the only one the old
/// display answers.
///
/// Hide first, then show and retarget with a single class write. At that
/// cascade the box's *new* display is `block`; only its *old* display says it
/// was not rendered before the change, and the before-change style of an
/// unrendered element is its after-change style, so nothing transitions.
#[test]
fn showing_a_hidden_node_whose_target_changed_does_not_transition() {
    let (mut doc, wrap, boxed) = mounted_box();

    doc.set_attribute(wrap, "class", "w--a hidden");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(
        display_of(&doc, boxed),
        rinch_dom::computed_style::DisplayValue::None,
        "precondition: hidden"
    );

    // One write: un-hides *and* retargets.
    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(802.0, 600.0);

    assert_eq!(
        display_of(&doc, boxed),
        rinch_dom::computed_style::DisplayValue::Block,
        "precondition: the box is rendered again"
    );
    assert_eq!(
        running(&doc, boxed),
        0,
        "the before-change style of an element that was not rendered is its \
         after-change style, so there is nothing to transition"
    );
    assert_eq!(width_px(&doc, boxed), Some(30.0), "it appears at 30px");
}

/// `display` is not inherited, so the box under a hidden wrapper computes
/// `display: block` and its own style cannot tell you it is not rendered.
///
/// This is a measurement, not a wish: it is the reason the gate walks the
/// ancestor chain rather than reading one field.
#[test]
fn a_descendant_of_a_hidden_ancestor_still_computes_display_block() {
    let (mut doc, wrap, boxed) = mounted_box();

    doc.set_attribute(wrap, "class", "w--a gone");
    doc.resolve_layout(801.0, 600.0);

    assert_eq!(
        display_of(&doc, wrap),
        rinch_dom::computed_style::DisplayValue::None,
        "the wrapper is the one that is hidden"
    );
    assert_eq!(
        display_of(&doc, boxed),
        rinch_dom::computed_style::DisplayValue::Block,
        "and the descendant's own display says nothing about it"
    );
}

/// The ancestor case of the issue: hidden by the wrapper, retargeted while
/// hidden.
#[test]
fn a_change_under_a_hidden_ancestor_does_not_start_a_transition() {
    let (mut doc, wrap, boxed) = mounted_box();

    doc.set_attribute(wrap, "class", "w--a gone");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(running(&doc, boxed), 0, "precondition: nothing running");

    doc.set_attribute(wrap, "class", "w--b gone");
    doc.resolve_layout(802.0, 600.0);

    assert_eq!(
        running(&doc, boxed),
        0,
        "a descendant of a `display: none` element is not being rendered either"
    );
    assert_eq!(
        width_px(&doc, boxed),
        Some(30.0),
        "and it takes the new value directly"
    );

    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(803.0, 600.0);
    assert_eq!(running(&doc, boxed), 0, "nothing starts when it is shown");
    assert_eq!(width_px(&doc, boxed), Some(30.0), "shown at 30px");
}

/// The one-step ancestor case: the wrapper is shown and the box retargeted by a
/// single class write.
///
/// The wrapper is restyled **before** the box on the same pass (the cascade
/// pushes parents first), so by the time the box is reached the wrapper's
/// `computed_style.display` already reads `block`. Only a record of what it was
/// before the change answers this one.
#[test]
fn showing_a_hidden_ancestor_whose_descendant_is_retargeted_does_not_transition() {
    let (mut doc, wrap, boxed) = mounted_box();

    doc.set_attribute(wrap, "class", "w--a gone");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(
        display_of(&doc, wrap),
        rinch_dom::computed_style::DisplayValue::None,
        "precondition: the ancestor is hidden"
    );

    // One write: un-hides the ancestor *and* retargets the descendant.
    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(802.0, 600.0);

    assert_eq!(
        display_of(&doc, wrap),
        rinch_dom::computed_style::DisplayValue::Block,
        "precondition: the ancestor is rendered again"
    );
    assert_eq!(
        running(&doc, boxed),
        0,
        "the descendant was not rendered before the change either"
    );
    assert_eq!(width_px(&doc, boxed), Some(30.0), "it appears at 30px");
}

/// The cancel half: a transition running when the box is hidden stops.
#[test]
fn hiding_a_node_cancels_its_running_transition() {
    let (mut doc, wrap, boxed) = mounted_box();

    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(
        running(&doc, boxed),
        1,
        "precondition: a transition is in flight"
    );

    doc.set_attribute(wrap, "class", "w--b hidden");
    doc.resolve_layout(802.0, 600.0);

    assert_eq!(
        running(&doc, boxed),
        0,
        "an element that stops being rendered has its transitions cancelled"
    );
}

/// The cancel half descends: hiding an **ancestor** cancels the descendant's
/// transition too.
///
/// The descendant is not restyled by this class change at all — nothing in its
/// own cascade changes — so the cancel cannot be a per-node check in the gate.
#[test]
fn hiding_an_ancestor_cancels_a_descendant_transition() {
    let (mut doc, wrap, boxed) = mounted_box();

    doc.set_attribute(wrap, "class", "w--b");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(
        running(&doc, boxed),
        1,
        "precondition: a transition is in flight"
    );

    doc.set_attribute(wrap, "class", "w--b gone");
    doc.resolve_layout(802.0, 600.0);

    assert_eq!(
        running(&doc, boxed),
        0,
        "the descendant of a hidden element is not being rendered either"
    );
}

/// `visibility: hidden` is **rendered**, and this is the opposite pin.
///
/// The box is generated, laid out and takes up space — it is merely not
/// painted. It has a before-change style and its transitions run, exactly as
/// they do while it is visible.
#[test]
fn visibility_hidden_is_rendered_and_still_transitions() {
    let (mut doc, wrap, boxed) = mounted_box();

    doc.set_attribute(wrap, "class", "w--a invisible");
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(
        doc.tree.get(boxed.0).unwrap().computed_style.visibility,
        rinch_dom::computed_style::VisibilityValue::Hidden,
        "precondition: the box is invisible"
    );
    assert_eq!(
        display_of(&doc, boxed),
        rinch_dom::computed_style::DisplayValue::Block,
        "precondition: and it still generates a box"
    );

    doc.set_attribute(wrap, "class", "w--b invisible");
    doc.resolve_layout(802.0, 600.0);

    assert_eq!(
        running(&doc, boxed),
        1,
        "`visibility: hidden` is not `display: none` — the element is rendered, \
         so it transitions"
    );
}
