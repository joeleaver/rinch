//! Telling a container that a child arrived **after** it rendered — issue #716.
//!
//! A parent component renders *after* its children: `rsx!` builds them into a
//! `<template>` and hands the finished [`NodeHandle`]s to `Component::render`,
//! so nothing a container knows can travel to an item as a *prop*. A container
//! whose prop is a default for its items — `List::icon`, `RadioGroup::size`,
//! `Stepper::active` — therefore finds the rendered items and patches them.
//!
//! That patch runs once. A child inserted later — by a `for` reconcile, a
//! `show_dom` branch, a hand-rolled [`NodeHandle::append_child`] — was never
//! reached, and came out with the container's default missing: `rows=2 icon
//! boxes=1`, a ragged list.
//!
//! So a container registers here instead, and is **told** each time a subtree
//! lands anywhere beneath it. The notification is synchronous with the
//! insertion, which is the whole reason it is a DOM hook and not a deferred
//! task: the default is in place before the frame that shows the new child is
//! laid out, so nothing flashes and no host needs a drain site. (The two queues
//! that could have carried a deferred version cannot: `queue_main_callback`
//! takes a `Send` closure and a `NodeHandle` is `!Send`, and `rinch-web` drains
//! it nowhere.)
//!
//! # What an observer is told, and what it must decide
//!
//! **Every** registered ancestor of the insertion point is called, nearest
//! first, with the node that was just inserted. Nearest-only would be wrong for
//! a radio group that holds a list, and all-of-them would be wrong for a list
//! nested inside another list's item — so the boundary is the observer's own
//! question to answer, from the node it was handed back up to its own root. It
//! is the same boundary the container's render-time walk applies downwards; see
//! `List::give_items_a_default_icon` for the shape.
//!
//! # Re-entrancy
//!
//! An observer patches the tree — that is the point — and those edits land
//! inside the very subtree it was registered for. Dispatch is therefore
//! suppressed for the duration of a callback, so a container's own edits never
//! call it back. A nested container's genuinely-new child *during* another
//! container's callback is missed by the same rule; nothing in the component
//! library builds one there, and the alternative is an unbounded loop.
//!
//! # Lifetime
//!
//! Registration is released by the ambient scope's `on_cleanup`, the discipline
//! the focus registry uses (issue #147), so unmounting a container drops its
//! observer. A container rendered outside any scope — from `main`, or straight
//! onto a `MockDomDocument` in a test — has no owner and keeps app lifetime,
//! which is the same answer every other rinch registry gives.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use super::{NodeHandle, NodeId};

/// What a container does with a subtree that landed beneath it.
type Observer = Rc<dyn Fn(&NodeHandle)>;

thread_local! {
    /// Keyed by `(doc_key, node id)`: a node id is a per-document slab index, so
    /// keying by it alone collides across two documents on one thread (#134).
    static OBSERVERS: RefCell<HashMap<(u64, NodeId), Observer>> = RefCell::new(HashMap::new());

    /// `OBSERVERS.len()`, so the overwhelmingly common case — a document with no
    /// container that has a default to give — costs one `Cell` read per
    /// insertion rather than a `RefCell` borrow and a hash lookup per ancestor.
    static COUNT: Cell<usize> = const { Cell::new(0) };

    /// Set while a callback runs. See the re-entrancy note above.
    static DISPATCHING: Cell<bool> = const { Cell::new(false) };
}

/// Call `f` whenever a subtree is inserted anywhere beneath `root`.
///
/// `f` is handed the newly inserted node — which may be an item, or a wrapper
/// holding several — and is called **after** the insertion, so the node is
/// already in place and its ancestors can be walked. Registering twice on one
/// node replaces the first observer.
///
/// See the module docs for the boundary rule, the re-entrancy rule, and the
/// lifetime rule.
pub fn on_child_inserted(root: &NodeHandle, f: impl Fn(&NodeHandle) + 'static) {
    let key = (root.doc_key(), root.node_id());
    let observer: Observer = Rc::new(f);
    OBSERVERS.with(|map| {
        if map.borrow_mut().insert(key, observer).is_none() {
            COUNT.with(|c| c.set(c.get() + 1));
        }
    });
    crate::reactive::on_cleanup(move || forget(key));
}

/// Drop the observer registered for `key`, if any.
fn forget(key: (u64, NodeId)) {
    OBSERVERS.with(|map| {
        if map.borrow_mut().remove(&key).is_some() {
            COUNT.with(|c| c.set(c.get().saturating_sub(1)));
        }
    });
}

/// Drop the observer registered on `node`, if any — called when a node is
/// discarded, since the backend may then hand its id to something else.
pub(super) fn forget_node(node: &NodeHandle) {
    if COUNT.with(|c| c.get()) == 0 {
        return;
    }
    forget((node.doc_key(), node.node_id()));
}

/// Restores [`DISPATCHING`] however a callback leaves the stack.
struct DispatchGuard;

impl Drop for DispatchGuard {
    fn drop(&mut self) {
        DISPATCHING.with(|d| d.set(false));
    }
}

/// Tell every registered ancestor of `parent` that `inserted` landed.
///
/// Called from the four [`NodeHandle`] methods that put a node into a tree.
/// Anything that reaches for [`super::traits::DomDocument`] directly bypasses
/// it; inside this workspace nothing outside `NodeHandle` does.
pub(super) fn notify_inserted(parent: &NodeHandle, inserted: &NodeHandle) {
    if COUNT.with(|c| c.get()) == 0 || DISPATCHING.with(|d| d.get()) {
        return;
    }
    let doc_key = parent.doc_key();
    if doc_key == 0 {
        return;
    }

    // Collected before any callback runs: a callback may register or drop an
    // observer, and it must not do so through a borrow this walk is holding.
    let mut observers: Vec<Observer> = Vec::new();
    let mut node = Some(parent.clone());
    while let Some(current) = node {
        let found = OBSERVERS.with(|map| map.borrow().get(&(doc_key, current.node_id())).cloned());
        if let Some(observer) = found {
            observers.push(observer);
        }
        node = current.parent_node();
    }
    if observers.is_empty() {
        return;
    }

    DISPATCHING.with(|d| d.set(true));
    let _guard = DispatchGuard;
    for observer in observers {
        observer(inserted);
    }
}
