//! `NodeHandle::insert_after` of a node that already sits right after the
//! anchor (issue #356).
//!
//! `a.insert_after(b)` where `b` is already `a`'s next sibling used to reach the
//! backend as `insert_before(parent, b, b)` — a reference that is the child
//! itself. The browser follows the DOM's rule ("if child is node, set child to
//! node's next sibling") and leaves `b` where it is. The two host backends did
//! not: `rinch-dom` could not find the reference once `b` was unlinked and
//! appended `b` at the **end** of the parent, and the mock dropped it from the
//! parent's child list altogether while keeping its parent pointer.
//!
//! A `for` reconcile reaches this shape since #356: a removed row now stays in
//! the DOM until its scope is disposed, so a memoising `view` that hands the
//! departing row's node to an arriving key inserts it right after the row it
//! already follows.
//!
//! The mock's twin is `dom::tests::inserting_a_node_after_the_node_it_already_follows_leaves_it_there`
//! in `rinch-core` (the mock is not exported to this crate). The fixture has a
//! sibling *after* `b`, so the append-at-the-end answer is
//! distinguishable from the right one.

use rinch_core::dom::{DomDocument, NodeHandle};
use rinch_dom::RinchDocument;
use std::cell::RefCell;
use std::rc::Rc;

fn order_after_in_place_insert(doc: Rc<RefCell<dyn DomDocument>>) -> Vec<String> {
    let weak = Rc::downgrade(&doc);
    let (a, b) = {
        let mut d = doc.borrow_mut();
        let body = d.body();
        let mut ids = Vec::new();
        for name in ["a", "b", "c"] {
            let id = d.create_element("div");
            d.set_attribute(id, "data-name", name);
            d.append_child(body, id);
            ids.push(id);
        }
        (ids[0], ids[1])
    };
    let a = NodeHandle::new(a, weak.clone());
    let b = NodeHandle::new(b, weak);

    a.insert_after(&b);

    let parent = a.parent_node().expect("a is mounted");
    parent
        .children()
        .iter()
        .filter_map(|n| n.get_attribute("data-name"))
        .collect()
}

#[test]
fn inserting_a_node_after_the_node_it_already_follows_leaves_it_there_on_rinch_dom() {
    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(RinchDocument::new()));
    assert_eq!(
        order_after_in_place_insert(doc),
        vec!["a", "b", "c"],
        "#356: rinch-dom appended `b` at the end instead of leaving it in place"
    );
}
