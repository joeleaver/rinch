//! Telling a node-keyed registry outside `rinch-core` that nodes were freed
//! (issue #1509).
//!
//! [`NodeHandle::set_inner_html`](super::NodeHandle::set_inner_html) frees the
//! children it replaces, and `rinch-dom` hands their ids to the next nodes it
//! mints — the very markup the write parses. A registry keyed by node id that
//! still holds an entry under a freed id answers for an unrelated node. The
//! child-observer registry, which lives in this crate, forgets its own
//! entries directly (#1489, `late_child::forget_descendants`); a registry in a
//! crate `rinch-core` cannot name (the focus-target registry in `rinch`)
//! registers a [`FreedNodesListener`] here instead.
//!
//! Nothing else frees a node through a [`NodeHandle`](super::NodeHandle):
//! `discard` and `remove` detach on `rinch-dom` (#723), and a text write
//! orphans children without freeing them.

use std::cell::RefCell;

use super::{NodeHandle, NodeId};

/// A registry that wants to forget nodes [`NodeHandle::set_inner_html`]
/// frees. See the [module docs](self).
///
/// [`NodeHandle::set_inner_html`]: super::NodeHandle::set_inner_html
#[derive(Clone, Copy)]
pub struct FreedNodesListener {
    /// Whether the registry holds anything a freed node could be under. While
    /// every listener answers `false`, a `set_inner_html` walks nothing.
    pub wants: fn() -> bool,
    /// Forget every entry under one of `ids` in the document `doc_key`. Called
    /// before the write frees them, with no document borrow held.
    pub forget: fn(doc_key: u64, ids: &[NodeId]),
}

thread_local! {
    static LISTENERS: RefCell<Vec<FreedNodesListener>> = const { RefCell::new(Vec::new()) };
}

/// Register `listener` for this thread. Registering the same `forget` twice
/// keeps one: a registry may call this from every registration it makes.
#[doc(hidden)]
pub fn on_nodes_freed(listener: FreedNodesListener) {
    LISTENERS.with(|l| {
        let mut l = l.borrow_mut();
        if !l
            .iter()
            .any(|x| std::ptr::fn_addr_eq(x.forget, listener.forget))
        {
            l.push(listener);
        }
    });
}

/// Tell the listeners that want it about every node **beneath** `node`, before
/// a write that frees them.
pub(super) fn forget_descendants(node: &NodeHandle) {
    let wanting: Vec<FreedNodesListener> = LISTENERS.with(|l| {
        l.borrow()
            .iter()
            .filter(|listener| (listener.wants)())
            .copied()
            .collect()
    });
    if wanting.is_empty() {
        return;
    }
    let Some(doc) = node.doc_upgrade() else {
        return;
    };
    let doc_key = node.doc_key();
    let mut ids = Vec::new();
    {
        let doc = doc.borrow();
        let mut stack = doc.get_children(node.node_id());
        while let Some(id) = stack.pop() {
            stack.extend(doc.get_children(id));
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return;
    }
    for listener in wanting {
        (listener.forget)(doc_key, &ids);
    }
}
