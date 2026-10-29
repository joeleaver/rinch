//! Telling a node-keyed registry outside `rinch-core` that nodes were freed
//! (issue #1509).
//!
//! [`NodeHandle::set_inner_html`](super::NodeHandle::set_inner_html) frees the
//! children it replaces, and [`NodeHandle::discard`](super::NodeHandle::discard)
//! frees the node and its subtree (on `rinch-dom` since #723; `rinch-web`
//! drops them from its node maps). An entry left under a freed id is dead:
//! since #723 a freed slot is re-issued under a new **generation**, so the id
//! never names the node minted into the slot next — but the entry would stay
//! for good if nothing removed it, and before #723 `set_inner_html` handed the
//! very same id to the markup it parsed. The child-observer registry, which
//! lives in this crate, forgets its own entries directly (#1489,
//! `late_child::forget_descendants`); a registry in a crate `rinch-core`
//! cannot name (the focus-target registry in `rinch`) registers a
//! [`FreedNodesListener`] here instead.
//!
//! `remove` frees nothing (the node can be inserted again, #719), and a text
//! write orphans children without freeing them.

use std::cell::RefCell;

use super::{NodeHandle, NodeId};

/// A registry that wants to forget nodes [`NodeHandle::set_inner_html`] and
/// [`NodeHandle::discard`] free. See the [module docs](self).
///
/// [`NodeHandle::set_inner_html`]: super::NodeHandle::set_inner_html
/// [`NodeHandle::discard`]: super::NodeHandle::discard
#[derive(Clone, Copy)]
pub struct FreedNodesListener {
    /// Whether the registry holds anything a freed node could be under. While
    /// every listener answers `false`, a `set_inner_html` or a `discard`
    /// walks nothing.
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
    forget(node, false);
}

/// Tell the listeners that want it about `node` **and** every node beneath
/// it, before a discard frees them (#723).
pub(super) fn forget_subtree(node: &NodeHandle) {
    forget(node, true);
}

fn forget(node: &NodeHandle, with_root: bool) {
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
    if with_root {
        ids.push(node.node_id());
    }
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
