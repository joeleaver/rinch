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
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::Rc;

use super::{NodeHandle, NodeId};

/// What a container does with a subtree that landed beneath it.
type Observer = Rc<dyn Fn(&NodeHandle)>;

/// A multiply-xor hasher for the registry's `(doc_key, NodeId)` keys.
///
/// The default `SipHash` costs about as much per lookup as the rest of an
/// ancestor step put together, and this walk runs on **every** insertion once
/// anything on the thread is registered. The keys are integers this crate mints
/// — a document counter and a slab index — not anything a document's author
/// chooses, so there is no untrusted input here to need hash-flooding
/// resistance.
#[derive(Default)]
struct IdHasher(u64);

impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        // One last mix, so the low bits a `HashMap` buckets on see the high ones.
        let h = self.0;
        (h ^ (h >> 32)).wrapping_mul(0xD6E8_FEB8_6659_FD93)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.write_u64(u64::from(*byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0 ^ value).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }
}

type Registry = HashMap<(u64, NodeId), Observer, BuildHasherDefault<IdHasher>>;

thread_local! {
    /// Keyed by `(doc_key, node id)`: a node id is a per-document slab index, so
    /// keying by it alone collides across two documents on one thread (#134).
    static OBSERVERS: RefCell<Registry> = RefCell::new(Registry::default());

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
/// it, and seven places in this workspace do — but every one of them is a
/// **root mount into `<body>`** (`rinch/src/app/mod.rs`, `rinch-web/src/lib.rs`,
/// four in `rinch/src/menu/app_menu_bar.rs`) or a *detached* span/text pair
/// (`render_scope.rs`), so none of them can land a child inside a registered
/// container. A new direct call that could is the thing to watch for: it will
/// silently not notify.
pub(super) fn notify_inserted(parent: &NodeHandle, inserted: &NodeHandle) {
    if COUNT.with(|c| c.get()) == 0 || DISPATCHING.with(|d| d.get()) {
        return;
    }
    let Some(doc) = parent.doc_upgrade() else {
        return;
    };
    let doc_key = doc.borrow().doc_key();
    if doc_key == 0 {
        return;
    }

    // Walked as bare ids under one upgraded document, rather than as a chain of
    // `NodeHandle`s: this runs on **every** insertion once anything on the
    // thread is registered, and a handle per ancestor is a `Weak` clone and an
    // upgrade per ancestor. Measured at depth 8, that was most of the cost.
    //
    // Collected before any callback runs: a callback edits the tree and may
    // register or drop an observer, so neither the document borrow nor the
    // registry borrow may still be held when one is called.
    let mut observers: Vec<Observer> = Vec::new();
    OBSERVERS.with(|map| {
        let map = map.borrow();
        let tree = doc.borrow();
        let mut node = Some(parent.node_id());
        while let Some(current) = node {
            if let Some(observer) = map.get(&(doc_key, current)) {
                observers.push(observer.clone());
            }
            node = tree.parent_node(current);
        }
    });
    if observers.is_empty() {
        return;
    }

    DISPATCHING.with(|d| d.set(true));
    let _guard = DispatchGuard;
    for observer in observers {
        observer(inserted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::mock::MockDomDocument;
    use crate::dom::traits::DomDocument;

    /// A document and the handles a test needs from it.
    fn doc() -> (Rc<RefCell<MockDomDocument>>, NodeHandle) {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let weak: std::rc::Weak<RefCell<dyn super::super::traits::DomDocument>> =
            Rc::downgrade(&doc) as _;
        (doc.clone(), NodeHandle::new(body, weak))
    }

    fn element(doc: &Rc<RefCell<MockDomDocument>>, tag: &str) -> NodeHandle {
        let id = doc.borrow_mut().create_element(tag);
        let weak: std::rc::Weak<RefCell<dyn super::super::traits::DomDocument>> =
            Rc::downgrade(doc) as _;
        NodeHandle::new(id, weak)
    }

    /// The ids an observer was handed, shared between the test and the closure.
    type Seen = Rc<RefCell<Vec<NodeId>>>;

    fn recorder() -> (Seen, Seen) {
        let seen: Seen = Rc::new(RefCell::new(Vec::new()));
        (seen.clone(), seen)
    }

    #[test]
    fn each_of_the_four_insertion_verbs_tells_the_observer() {
        for verb in [
            "append_child",
            "insert_before",
            "insert_after",
            "replace_with",
        ] {
            let (d, body) = doc();
            let root = element(&d, "div");
            body.append_child(&root);
            let anchor = element(&d, "span");
            root.append_child(&anchor);

            let (seen, sink) = recorder();
            on_child_inserted(&root, move |node| sink.borrow_mut().push(node.node_id()));

            let fresh = element(&d, "b");
            match verb {
                "append_child" => root.append_child(&fresh),
                "insert_before" => root.insert_before(&fresh, &anchor),
                "insert_after" => anchor.insert_after(&fresh),
                _ => anchor.replace_with(&fresh),
            }

            assert_eq!(
                *seen.borrow(),
                vec![fresh.node_id()],
                "`{verb}` puts a node into a tree, so it has to tell the \
                 container that node landed in (issue #716)"
            );
            forget((root.doc_key(), root.node_id()));
        }
    }

    #[test]
    fn every_registered_ancestor_is_told_nearest_first() {
        let (d, body) = doc();
        let outer = element(&d, "div");
        let inner = element(&d, "div");
        body.append_child(&outer);
        outer.append_child(&inner);

        let order = Rc::new(RefCell::new(Vec::new()));
        let sink = order.clone();
        on_child_inserted(&outer, move |_| sink.borrow_mut().push("outer"));
        let sink = order.clone();
        on_child_inserted(&inner, move |_| sink.borrow_mut().push("inner"));

        let fresh = element(&d, "b");
        inner.append_child(&fresh);

        assert_eq!(
            *order.borrow(),
            vec!["inner", "outer"],
            "nearest first. Both are told because two containers of different \
             kinds can nest — a radio group holding a list — and each decides \
             for itself whether the node is its business"
        );
        forget((outer.doc_key(), outer.node_id()));
        forget((inner.doc_key(), inner.node_id()));
    }

    #[test]
    fn a_callbacks_own_edits_do_not_call_it_back() {
        let (d, body) = doc();
        let root = element(&d, "div");
        body.append_child(&root);

        let runs = Rc::new(RefCell::new(0));
        let sink = runs.clone();
        let d2 = d.clone();
        on_child_inserted(&root, move |node| {
            *sink.borrow_mut() += 1;
            // Exactly what a container's patch does: put something into the
            // subtree it is watching.
            let patch = element(&d2, "i");
            node.append_child(&patch);
        });

        let fresh = element(&d, "b");
        root.append_child(&fresh);

        assert_eq!(
            *runs.borrow(),
            1,
            "an observer that edits the tree it watches must not re-enter \
             itself — the alternative is an unbounded loop, not a missed patch"
        );
        forget((root.doc_key(), root.node_id()));
    }

    #[test]
    fn unmounting_the_container_releases_its_observer() {
        let (d, body) = doc();
        let root = element(&d, "div");
        body.append_child(&root);

        let runs = Rc::new(RefCell::new(0));
        {
            // A scope of the shape a component renders under. Registration ties
            // itself to the ambient owner, so dropping the scope has to take the
            // observer with it.
            let scope = crate::dom::RenderScope::new(
                d.clone() as Rc<RefCell<dyn DomDocument>>,
                body.node_id(),
            );
            let owner = scope.push_owner();
            let sink = runs.clone();
            on_child_inserted(&root, move |_| *sink.borrow_mut() += 1);
            drop(owner);

            root.append_child(&element(&d, "i"));
            assert_eq!(
                *runs.borrow(),
                1,
                "positive control: the observer fires while its scope is alive"
            );
        }

        root.append_child(&element(&d, "b"));
        assert_eq!(
            *runs.borrow(),
            1,
            "and not after it is gone. `on_child_inserted` is public, so a \
             third-party container may capture a `Signal` its scope owns — \
             firing an observer whose scope has been disposed is the #141 PR4 \
             use-after-free, and nothing else in this module catches it: \
             `discarding_the_container_drops_its_observer` pins the other \
             release path"
        );
    }

    #[test]
    fn discarding_the_container_drops_its_observer() {
        let (d, body) = doc();
        let root = element(&d, "div");
        body.append_child(&root);

        let runs = Rc::new(RefCell::new(0));
        let sink = runs.clone();
        on_child_inserted(&root, move |_| *sink.borrow_mut() += 1);

        let before = COUNT.with(|c| c.get());
        assert!(before > 0, "positive control: the observer was registered");
        root.discard();
        assert_eq!(
            COUNT.with(|c| c.get()),
            before - 1,
            "a discarded id may be handed to the next node the document mints, \
             and an observer left under it would fire for a container that no \
             longer exists"
        );
    }

    #[test]
    fn two_documents_on_one_thread_do_not_collide() {
        // Node ids are per-document slab indices, so the same id exists in both.
        let (first, first_body) = doc();
        let (second, second_body) = doc();
        let a = element(&first, "div");
        let b = element(&second, "div");
        first_body.append_child(&a);
        second_body.append_child(&b);
        assert_eq!(
            a.node_id(),
            b.node_id(),
            "precondition: the two containers share an id, which is what makes \
             this a test of the doc_key half of the key (issue #134)"
        );

        let runs = Rc::new(RefCell::new(0));
        let sink = runs.clone();
        on_child_inserted(&a, move |_| *sink.borrow_mut() += 1);

        b.append_child(&element(&second, "i"));
        assert_eq!(
            *runs.borrow(),
            0,
            "the other document's insertion is not this container's business"
        );
        a.append_child(&element(&first, "i"));
        assert_eq!(*runs.borrow(), 1, "positive control: this one is");
        forget((a.doc_key(), a.node_id()));
    }
}
