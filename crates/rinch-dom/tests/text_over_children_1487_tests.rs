//! Text written over an element's children, on `rinch-dom` (issue #1487).
//!
//! This backend reclaims nothing on a discard (#723), so what is pinned here
//! is the minting table, and the one shape where the write must **not**
//! discard: `rinch-dom` writes an element's lone text child in place (#1440),
//! so that child is still a child afterwards and is not the write's to retire.
//! The mock's twins are `reinsertion_tests::text_over_children_1487` in
//! `rinch-core`.

use rinch_core::dom::{__minted_by_len, DomDocument, NodeHandle, RenderScope};
use rinch_dom::RinchDocument;
use std::cell::RefCell;
use std::rc::Rc;

fn mounted() -> (Rc<RefCell<dyn DomDocument>>, NodeHandle, RenderScope) {
    let doc: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(RinchDocument::new()));
    let body = doc.borrow().body();
    let scope = RenderScope::new(doc.clone(), body);
    let body = NodeHandle::new(body, Rc::downgrade(&doc));
    (doc, body, scope)
}

#[test]
fn text_over_scope_built_children_go_with_their_scope() {
    let (doc, body, _table) = mounted();
    let mut scope = RenderScope::new(doc.clone(), body.node_id());
    let base = __minted_by_len();
    let host = scope.create_element("div");
    body.append_child(&host);
    for i in 0..50 {
        let c = scope.create_element("p");
        c.append_child(&scope.create_text(&i.to_string()));
        host.append_child(&c);
        host.append_child(&scope.create_element("span"));
        host.set_text("cleared");
        assert_eq!(doc.borrow().parent_node(c.node_id()), None);
    }
    assert_eq!(
        __minted_by_len(),
        base + 1 + 150,
        "while the scope lives the orphans are kept, records and all"
    );
    drop(scope);
    // The text node `rinch-dom` mints for "cleared" is raw: it has no record.
    assert_eq!(
        __minted_by_len(),
        base + 1,
        "#1487: the orphaned children's records went with their scope; `host` \
         is still in the document"
    );
}

#[test]
fn an_elements_lone_text_child_written_in_place_keeps_its_record() {
    let (doc, body, mut scope) = mounted();
    let label = scope.create_element("p");
    let text = scope.create_text("one");
    label.append_child(&text);
    body.append_child(&label);
    let base = __minted_by_len();

    label.set_text("two");

    assert_eq!(
        doc.borrow().get_children(label.node_id()),
        vec![text.node_id()],
        "precondition: the text child was written in place"
    );
    assert_eq!(
        __minted_by_len(),
        base,
        "a child the write left where it was is still the scope's"
    );
    assert_eq!(label.text_content().as_deref(), Some("two"));
}
