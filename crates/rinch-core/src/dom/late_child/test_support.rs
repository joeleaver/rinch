//! The fixtures the late-child tests share: a mock document, its nodes,
//! and observers that record what they are told.

use super::*;
use crate::dom::mock::MockDomDocument;
use crate::dom::traits::DomTree;

/// A document and the handles a test needs from it.
pub(super) fn doc() -> (Rc<RefCell<MockDomDocument>>, NodeHandle) {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let weak: std::rc::Weak<RefCell<dyn super::super::traits::DomDocument>> =
        Rc::downgrade(&doc) as _;
    (doc.clone(), NodeHandle::new(body, weak))
}

pub(super) fn element(doc: &Rc<RefCell<MockDomDocument>>, tag: &str) -> NodeHandle {
    let id = doc.borrow_mut().create_element(tag);
    let weak: std::rc::Weak<RefCell<dyn super::super::traits::DomDocument>> =
        Rc::downgrade(doc) as _;
    NodeHandle::new(id, weak)
}

/// The ids an observer was handed, shared between the test and the closure.
pub(super) type Seen = Rc<RefCell<Vec<NodeId>>>;

pub(super) fn recorder() -> (Seen, Seen) {
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    (seen.clone(), seen)
}

pub(super) fn text(doc: &Rc<RefCell<MockDomDocument>>, content: &str) -> NodeHandle {
    let id = doc.borrow_mut().create_text(content);
    let weak: std::rc::Weak<RefCell<dyn super::super::traits::DomDocument>> =
        Rc::downgrade(doc) as _;
    NodeHandle::new(id, weak)
}

/// Register a removal observer and hand back the ids it was told about.
pub(super) fn watch_removals(root: &NodeHandle) -> Seen {
    let (seen, sink) = recorder();
    on_child_removed(root, move |node| sink.borrow_mut().push(node.node_id()));
    seen
}
