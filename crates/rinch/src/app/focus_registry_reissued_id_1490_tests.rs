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
