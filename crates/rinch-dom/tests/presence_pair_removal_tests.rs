//! Desktop's half of issue #687: the falsey write of a `checked` / `selected`
//! binding, and what it costs when it erases nothing.
//!
//! #687 is a **web** defect. A browser sets a dirty-checkedness flag on the
//! first user toggle and from then on the content attribute is only the
//! control's default, so `getAttribute("checked")` can read absent while the box
//! reads checked — and `NodeHandle::write_attribute`, whose removal was guarded
//! by "is the attribute already absent", wrote nothing and left the control on.
//!
//! Desktop cannot reach that state. The attribute *is* the whole of the state
//! here: `:checked` (`stylo_impl.rs`) and `<option>` selectedness (`select.rs`)
//! read presence and nothing else (pinned in `boolean_attribute_readers.rs`),
//! and no desktop input path writes a raw `<input>`'s `checked` — the shell's
//! pointer and key dispatch never touch it, and `Checkbox` / `Switch` / `Radio`
//! route a click through `onchange` into the app's own signal, then write the
//! attribute themselves. So the two halves of "off" cannot drift apart, and the
//! first test here is what says so.
//!
//! The fix lets `checked` / `selected` through that guard unconditionally, which
//! makes a falsey write reach `RinchDocument::remove_attribute` on desktop too —
//! including the very common case where there is nothing to remove. The second
//! test is the compensating guard: removing an attribute the node does not carry
//! now returns before invalidating anything, as `removeAttribute` does in a
//! browser.

use rinch_core::dom::{DomDocument, NodeId, RenderScope};
use rinch_dom::RinchDocument;
use std::cell::RefCell;
use std::rc::Rc;

/// A document plus a `RenderScope` over it, so a test can drive the real
/// `NodeHandle::write_attribute` rather than the backend primitive underneath.
fn scoped() -> (Rc<RefCell<RinchDocument>>, RenderScope) {
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    (doc.clone(), RenderScope::new(dyn_doc, body))
}

fn opacity(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().computed_style.opacity
}

/// A falsey `checked` write turns a desktop checkbox off, from either starting
/// state — the attribute is the whole of the state, so the #687 divergence has
/// nowhere to live.
///
/// Both directions on purpose. Starting from *on* is what proves the write
/// lands at all; starting from *off* is the shape #687 reported, and on desktop
/// it has to be a no-op that leaves the box off rather than a no-op that leaves
/// it on.
#[test]
fn a_falsey_checked_write_leaves_a_desktop_checkbox_off_from_either_state() {
    let (doc, mut scope) = scoped();
    doc.borrow_mut().load_css("input:checked { opacity: 0.5 }");

    let on = scope.create_element("input");
    on.set_attribute("style", "width: 80px; height: 30px");
    on.set_attribute("type", "checkbox");
    let off = scope.create_element("input");
    off.set_attribute("style", "width: 80px; height: 30px");
    off.set_attribute("type", "checkbox");
    let body = scope.parent();
    body.append_child(&on);
    body.append_child(&off);

    // One starts checked, the other never was.
    on.write_attribute("checked", "true");
    doc.borrow_mut().resolve_layout(800.0, 600.0);
    assert_eq!(
        opacity(&doc.borrow(), on.node_id()),
        0.5,
        "precondition: on"
    );
    assert_eq!(
        opacity(&doc.borrow(), off.node_id()),
        1.0,
        "precondition: off"
    );

    on.write_attribute("checked", "false");
    off.write_attribute("checked", "false");
    // A fresh viewport: `resolve_layout` early-returns on a clean tree, so
    // re-resolving at 800x600 would measure nothing.
    doc.borrow_mut().resolve_layout(801.0, 600.0);

    assert_eq!(
        opacity(&doc.borrow(), on.node_id()),
        1.0,
        "a falsey write must uncheck the box"
    );
    assert_eq!(
        doc.borrow().get_attribute(on.node_id(), "checked"),
        None,
        "and it must uncheck it by *removing* the attribute — a present \
         `checked=\"false\"` is checked, in HTML and here (#551)"
    );
    assert_eq!(
        opacity(&doc.borrow(), off.node_id()),
        1.0,
        "and one that was already off stays off"
    );
}

/// Removing an attribute the node does not carry invalidates nothing.
///
/// This is the guard that keeps #687's unconditional removal free on desktop.
/// `remove_attribute` otherwise clears the node's Stylo data, pushes it as a
/// style root and marks its whole subtree — a full restyle for a write that
/// erased nothing, paid on every falsey `checked` / `selected` an effect writes.
///
/// The positive control is the load-bearing half: an assertion that a list is
/// empty says nothing unless the same instrument is shown filling it.
#[test]
fn removing_an_absent_attribute_dirties_nothing_and_a_present_one_does() {
    let mut doc = RinchDocument::new();
    let el = doc.create_element("div");
    let body = doc.body();
    doc.append_child(body, el);
    doc.set_attribute(el, "data-state", "open");
    doc.resolve_layout(800.0, 600.0);
    doc.take_dirty_nodes();

    // Nothing to erase: no invalidation.
    doc.remove_attribute(el, "data-absent");
    assert!(
        !doc.take_dirty_nodes().contains(&el),
        "removing an attribute the node does not carry must not restyle it"
    );

    // Positive control, same instrument: a real removal does dirty it.
    doc.remove_attribute(el, "data-state");
    assert!(
        doc.take_dirty_nodes().contains(&el),
        "a removal that erases something must still invalidate"
    );
    assert_eq!(doc.get_attribute(el, "data-state"), None);

    // And now that it is gone, removing it again is free as well — the state
    // the #687 writer lands in every time an effect re-runs while already off.
    doc.remove_attribute(el, "data-state");
    assert!(
        !doc.take_dirty_nodes().contains(&el),
        "a repeated removal must not restyle either"
    );
}
