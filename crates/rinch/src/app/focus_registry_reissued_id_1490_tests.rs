//! A focus target is released by its own registration, on `rinch-dom`, where a
//! freed node id really is handed to the next node minted (issue #1490; from
//! the review of PR #1499).

use crate::focus_registry::{FocusEntry, is_registered, register_focus_target, wants_key_routing};
use rinch_core::dom::{DomDocument, NodeHandle, NodeId};
use rinch_core::reactive::Scope;
use rinch_dom::RinchDocument;
use std::cell::RefCell;
use std::rc::Rc;

fn el(doc: &Rc<RefCell<dyn DomDocument>>, tag: &str) -> NodeHandle {
    let id = doc.borrow_mut().create_element(tag);
    NodeHandle::new(id, Rc::downgrade(doc))
}

/// Mint elements until one is handed `id` (rinch-dom pops the most recently
/// freed slab key first).
fn mint_at(doc: &Rc<RefCell<dyn DomDocument>>, id: NodeId) -> NodeHandle {
    for _ in 0..64 {
        let n = el(doc, "div");
        if n.node_id() == id {
            return n;
        }
    }
    panic!("precondition: the freed id was not re-issued");
}

#[test]
fn a_focus_target_on_a_reissued_id_survives_the_old_scopes_cleanup() {
    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(RinchDocument::new()));
    let body = NodeHandle::new(doc.borrow().body(), Rc::downgrade(&doc));
    let host = el(&doc, "div");
    body.append_child(&host);
    let a = el(&doc, "div");
    host.append_child(&a);
    let x = a.node_id();
    let old = Scope::new();
    old.run(|| register_focus_target(&a, FocusEntry::new()));

    host.set_inner_html("");
    let b = mint_at(&doc, x);
    host.append_child(&b);
    let new = Scope::new();
    new.run(|| register_focus_target(&b, FocusEntry::new().on_key(|_| true)));

    old.dispose();
    assert!(
        wants_key_routing(b.doc_key(), x.0),
        "#1490: the old scope's cleanup left B's registration"
    );
    new.dispose();
    assert!(!is_registered(b.doc_key(), x.0));
}

/// #1509 (from the review of PR #1499): `set_inner_html` frees the children it
/// replaces and `rinch-dom` hands their ids to the next nodes it mints, so a
/// focus target freed that way must not leave its entry under the id — or an
/// unregistered plain `<div>` minted there answers as a registered target with
/// the dead component's `on_key` and IME claim, while that component's scope
/// is still alive. The target sits two levels below the replaced element, so
/// a drop that looks only at direct children misses it; a target outside the
/// replaced subtree must keep its registration.
#[test]
fn an_unregistered_node_on_a_freed_targets_id_is_not_a_registered_target() {
    use crate::focus_registry::wants_ime;
    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(RinchDocument::new()));
    let body = NodeHandle::new(doc.borrow().body(), Rc::downgrade(&doc));
    let host = el(&doc, "div");
    body.append_child(&host);
    let wrapper = el(&doc, "div");
    host.append_child(&wrapper);
    let a = el(&doc, "div");
    wrapper.append_child(&a);
    let outside = el(&doc, "div");
    body.append_child(&outside);
    let x = a.node_id();
    let old = Scope::new();
    old.run(|| {
        register_focus_target(&a, FocusEntry::new().on_key(|_| true).on_ime(|_| {}));
        register_focus_target(&outside, FocusEntry::new().on_key(|_| true));
    });
    assert!(is_registered(a.doc_key(), x.0), "control: registered");

    host.set_inner_html("");
    let plain = mint_at(&doc, x);
    host.append_child(&plain);
    assert!(
        !is_registered(plain.doc_key(), x.0),
        "a plain <div> nobody registered answers as a registered focus target"
    );
    assert!(!wants_key_routing(plain.doc_key(), x.0));
    assert!(!wants_ime(plain.doc_key(), x.0));
    assert!(
        wants_key_routing(outside.doc_key(), outside.node_id().0),
        "a target outside the replaced subtree keeps its registration"
    );
    old.dispose();
    assert!(!is_registered(outside.doc_key(), outside.node_id().0));
}
