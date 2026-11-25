//! [`Target`]: what a [`TestApp`] method acts on.

use super::*;

/// What a [`TestApp`] method acts on: a CSS selector (`"#row"`, `".item"`,
/// the first match), or a [`NodeHandle`] the component kept.
///
/// Sealed: implemented for `&str`, `String`, `NodeHandle` and `&NodeHandle`
/// only.
pub trait Target: sealed::Sealed {
    /// The node this names in `app`'s document. Panics when it names none.
    fn node_id(&self, app: &TestApp) -> NodeId {
        self.in_doc(app.doc())
    }
}

mod sealed {
    use super::{NodeId, Rc, RefCell, RinchDocument};

    pub trait Sealed {
        /// The node this names in `doc`. Panics when it names none.
        fn in_doc(&self, doc: &Rc<RefCell<RinchDocument>>) -> NodeId;
    }

    impl Sealed for &str {
        fn in_doc(&self, doc: &Rc<RefCell<RinchDocument>>) -> NodeId {
            super::read::query_all(doc, self)
                .into_iter()
                .next()
                .unwrap_or_else(|| panic!("no element matches {self:?}"))
                .node_id()
        }
    }

    impl Sealed for String {
        fn in_doc(&self, doc: &Rc<RefCell<RinchDocument>>) -> NodeId {
            self.as_str().in_doc(doc)
        }
    }

    impl Sealed for super::NodeHandle {
        fn in_doc(&self, doc: &Rc<RefCell<RinchDocument>>) -> NodeId {
            // A node id names a node in one document only: a handle from
            // another window's document would read some other node here.
            let (mine, here) = (self.doc_key(), doc.borrow().doc_key());
            assert_eq!(
                mine, here,
                "a node of document {mine} used on the window of document {here}"
            );
            super::NodeHandle::node_id(self)
        }
    }

    impl Sealed for &super::NodeHandle {
        fn in_doc(&self, doc: &Rc<RefCell<RinchDocument>>) -> NodeId {
            (*self).clone().in_doc(doc)
        }
    }
}

impl Target for &str {}

impl Target for String {}

impl Target for NodeHandle {}

impl Target for &NodeHandle {}
