use super::{DomDocument, ElementBounds, NodeHandle, NodeId, Rc, RefCell, RinchDocument, Weak};

/// `doc` as a weak handle, for [`NodeHandle`]s.
pub(super) fn weak(doc: &Rc<RefCell<RinchDocument>>) -> Weak<RefCell<dyn DomDocument>> {
    let doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    Rc::downgrade(&doc)
}

/// Every element matching `selector`, in document order.
pub(super) fn query_all(doc: &Rc<RefCell<RinchDocument>>, selector: &str) -> Vec<NodeHandle> {
    let ids = doc.borrow().query_selector_all(selector);
    let weak = weak(doc);
    ids.into_iter()
        .map(|id| NodeHandle::new(id, weak.clone()))
        .collect()
}

/// The text in `id`'s subtree.
pub(super) fn text(doc: &Rc<RefCell<RinchDocument>>, id: NodeId) -> String {
    rinch_dom::testing::get_text_content(&doc.borrow().tree, id.0)
}

/// `id`'s attribute `name`.
pub(super) fn attribute(
    doc: &Rc<RefCell<RinchDocument>>,
    id: NodeId,
    name: &str,
) -> Option<String> {
    let d = doc.borrow();
    d.tree.get(id.0)?.attributes.get(name).cloned()
}

/// The box `id` is painted in, in logical pixels from its window's
/// top-left.
pub(super) fn bounds(doc: &Rc<RefCell<RinchDocument>>, id: NodeId) -> ElementBounds {
    let (x, y, width, height) = crate::app::painted_element_box(&doc.borrow().tree, id.0);
    ElementBounds {
        x,
        y,
        width,
        height,
    }
}
