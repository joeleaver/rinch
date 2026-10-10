//! A late-child observer is released by its own registration, not by the node
//! id it was registered under (issue #1490).
//!
//! `rinch-dom` frees slab keys (`set_inner_html`, pseudo-element pruning) and
//! hands a freed key to the next node it mints. A container registered inside
//! a scope used to leave that scope a cleanup that forgot "whatever is
//! registered at `(doc, id)`", so once the id was re-issued the old scope's
//! disposal dropped the observers of the container that now holds it. The
//! mock never re-issues an id; its twin
//! (`dom::late_child::tests::a_replaced_registration_survives_the_first_scopes_cleanup`)
//! reaches the same cleanup by registering twice on one node.

use rinch_core::dom::{DomDocument, NodeHandle, on_child_inserted, on_child_removed};
use rinch_core::reactive::Scope;
use rinch_dom::RinchDocument;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn new_doc() -> (Rc<RefCell<dyn DomDocument>>, NodeHandle) {
    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(RinchDocument::new()));
    let body = doc.borrow().body();
    let body = NodeHandle::new(body, Rc::downgrade(&doc));
    (doc, body)
}

fn el(doc: &Rc<RefCell<dyn DomDocument>>, tag: &str) -> NodeHandle {
    let id = doc.borrow_mut().create_element(tag);
    NodeHandle::new(id, Rc::downgrade(doc))
}

/// One child in and out of `container`.
fn churn(doc: &Rc<RefCell<dyn DomDocument>>, container: &NodeHandle) {
    let k = el(doc, "span");
    container.append_child(&k);
    k.remove();
}

#[test]
fn a_new_container_on_a_reissued_id_keeps_its_observers_when_the_old_scope_is_disposed() {
    let (doc, body) = new_doc();
    let host = el(&doc, "div");
    body.append_child(&host);
    let old = el(&doc, "section");
    host.append_child(&old);

    let old_heard = Rc::new(Cell::new(0usize));
    let old_scope = Scope::new();
    old_scope.run(|| {
        let (a, b) = (old_heard.clone(), old_heard.clone());
        on_child_removed(&old, move |_| a.set(a.get() + 1));
        on_child_inserted(&old, move |_| b.set(b.get() + 1));
    });

    // Frees `old`; the next node minted is handed its id.
    host.set_inner_html("");
    let fresh = el(&doc, "article");
    assert_eq!(
        fresh.node_id(),
        old.node_id(),
        "precondition: the freed id was re-issued"
    );
    host.append_child(&fresh);
    let removed = Rc::new(Cell::new(0usize));
    let inserted = Rc::new(Cell::new(0usize));
    let (r, i) = (removed.clone(), inserted.clone());
    on_child_removed(&fresh, move |_| r.set(r.get() + 1));
    on_child_inserted(&fresh, move |_| i.set(i.get() + 1));

    churn(&doc, &fresh);
    assert_eq!(
        (inserted.get(), removed.get()),
        (1, 1),
        "positive control: the new container's observers are live"
    );

    // The old container's component finally unmounts.
    old_scope.dispose();

    churn(&doc, &fresh);
    assert_eq!(
        (inserted.get(), removed.get()),
        (2, 2),
        "#1490: the old scope's cleanup released its own registration, which \
         was already gone, and not the one now under the same id"
    );
    assert_eq!(
        old_heard.get(),
        0,
        "and the freed container's observers never answered for the new one"
    );
}
