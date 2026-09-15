//! Telling a container that its children changed **after** it rendered —
//! issues #716 and #745.
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
//! So a container registers here instead, and is **told** each time the set of
//! nodes beneath it changes. The notification is synchronous with the mutation,
//! which is the whole reason it is a DOM hook and not a deferred task: the
//! default is in place before the frame that shows the change is laid out, so
//! nothing flashes and no host needs a drain site. (The two queues that could
//! have carried a deferred version cannot: `queue_main_callback` takes a `Send`
//! closure and a `NodeHandle` is `!Send`, and `rinch-web` drains it nowhere.)
//!
//! # The two halves
//!
//! [`on_child_inserted`] is told a subtree **landed**, and is handed that
//! subtree. [`on_child_removed`] is told a subtree **left**, and is handed the
//! node it left — the former parent — rather than the node that went.
//!
//! The asymmetry is forced, not a taste: a removed node is detached, so it has
//! no ancestor chain for an observer to walk, and after
//! [`NodeHandle::discard`] the backend may have retired it altogether (`remove`
//! and `discard` are one verb to a `for` reconcile — see `for_loop::release_row`
//! — so an observer cannot know which it got). The former parent is still in
//! the tree and is what the observer actually needs: a container reached by a
//! removal has to re-derive from the children it has *left*, which is exactly
//! what `Stepper::settle_steps` does.
//!
//! A container that wants both registers both. `Stepper` does, because a step
//! that leaves renumbers every step behind it just as one that arrives does
//! (issue #745); `List` and `RadioGroup` want only the insertion half, since
//! neither of their defaults depends on a sibling's position.
//!
//! # What an observer is told, and what it must decide
//!
//! **Every** registered ancestor of the changed parent is called, nearest
//! first. Nearest-only would be wrong for a radio group that holds a list, and
//! all-of-them would be wrong for a list nested inside another list's item — so
//! the boundary is the observer's own question to answer, from the node it was
//! handed back up to its own root. It is the same boundary the container's
//! render-time walk applies downwards; see `List::give_items_a_default_icon`
//! for the shape.
//!
//! A **move** — an insertion verb handed a node that already has a parent, which
//! is how a `for` reconcile relocates a row — is a removal for the parent it
//! came from and an insertion for the one it landed in, and fires both. It fires
//! only the insertion when those are the same node: a reorder inside one parent
//! changes no container's child *set*, and the insertion half already tells
//! every observer that something moved.
//!
//! # What bypasses this
//!
//! Both halves are fired from [`NodeHandle`] and nowhere else, so anything that
//! reaches for [`DomDocument`](super::traits::DomDocument) directly changes the
//! tree without telling anyone. Enumerated, because the useful thing to know is
//! which of them *could* land in or take a node out of a registered container:
//!
//! - **Root mounts into `<body>`** — `rinch/src/app/mod.rs`, two in
//!   `rinch-web/src/lib.rs` (one of them the teardown `remove_node`), four in
//!   `rinch/src/menu/app_menu_bar.rs` — and one *detached* span/text pair in
//!   `render_scope.rs`. None of these can reach inside a container: a container
//!   is never the body, and the detached pair has no parent yet.
//! - **`rinch/src/app/select_widget.rs`'s popup teardown**, which `remove_node`s
//!   a panel and a backdrop it mounted into `<body>` itself. Same reason.
//! - **[`UpdateBatch::apply`](super::UpdateBatch::apply)** — and this one *can*.
//!   Its `AppendChild` / `InsertBefore` / `RemoveChild` / `ReplaceNode` arms take
//!   arbitrary ids, and `UpdateBatch` is exported from the `rinch` prelude, so an
//!   app can move a node anywhere through it and no observer hears about it.
//!   Nothing in this workspace applies a structural arm — the one consumer is a
//!   unit test that batches `SetText` and `SetAttribute` — and it cannot be
//!   routed through the notifying verbs as it stands, because it is handed a
//!   `&mut dyn DomDocument` and a notification needs the `Rc` a `NodeHandle`
//!   holds a `Weak` of. Tracked as **#756**;
//!   `an_update_batch_bypasses_both_halves` pins the hole so the claim cannot go
//!   stale in either direction.
//!
//! A new direct call is the thing to watch for: it will silently not notify.
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

/// What a container does when the nodes beneath it change.
///
/// Handed the subtree that landed, for an insertion; the parent it left, for a
/// removal. See the module docs for why those differ.
type Observer = Rc<dyn Fn(&NodeHandle)>;

/// Which half of the contract a notification is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Half {
    Inserted,
    Removed,
}

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

/// One container's registration: either half, or both.
///
/// One entry per node rather than two parallel registries, so the ancestor walk
/// is written once and a discarded id releases both halves in one lookup.
#[derive(Default)]
struct Entry {
    inserted: Option<Observer>,
    removed: Option<Observer>,
}

impl Entry {
    fn slot(&mut self, half: Half) -> &mut Option<Observer> {
        match half {
            Half::Inserted => &mut self.inserted,
            Half::Removed => &mut self.removed,
        }
    }

    fn observer(&self, half: Half) -> Option<&Observer> {
        match half {
            Half::Inserted => self.inserted.as_ref(),
            Half::Removed => self.removed.as_ref(),
        }
    }
}

type Registry = HashMap<(u64, NodeId), Entry, BuildHasherDefault<IdHasher>>;

thread_local! {
    /// Keyed by `(doc_key, node id)`: a node id is a per-document slab index, so
    /// keying by it alone collides across two documents on one thread (#134).
    static OBSERVERS: RefCell<Registry> = RefCell::new(Registry::default());

    /// How many observers are registered for insertions, so the overwhelmingly
    /// common case — a thread with no container that has a default to give —
    /// costs one `Cell` read per insertion rather than a `RefCell` borrow and a
    /// hash lookup per ancestor.
    static INSERT_COUNT: Cell<usize> = const { Cell::new(0) };

    /// The same for removals, counted **separately**: a removal verb has to read
    /// the node's parent *before* the detach to have anything to notify, and an
    /// app whose only container is a `List` should not pay that read. `Stepper`
    /// is the one container in this workspace that raises this count.
    static REMOVE_COUNT: Cell<usize> = const { Cell::new(0) };

    /// Set while a callback runs. See the re-entrancy note above.
    static DISPATCHING: Cell<bool> = const { Cell::new(false) };
}

/// The live count for one half.
fn count_of(half: Half) -> usize {
    match half {
        Half::Inserted => INSERT_COUNT.with(|c| c.get()),
        Half::Removed => REMOVE_COUNT.with(|c| c.get()),
    }
}

fn bump(half: Half, delta: isize) {
    let cell = match half {
        Half::Inserted => &INSERT_COUNT,
        Half::Removed => &REMOVE_COUNT,
    };
    cell.with(|c| {
        c.set(if delta < 0 {
            c.get().saturating_sub(1)
        } else {
            c.get() + 1
        })
    });
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
    register(root, Half::Inserted, f);
}

/// Call `f` whenever a subtree is removed from anywhere beneath `root`
/// (issue #745).
///
/// `f` is handed the node the subtree **left** — its former parent — not the
/// node that went, and is called **after** the removal, so what it walks is the
/// children the container has left. That is the asymmetry with
/// [`on_child_inserted`], and it is forced: a removed node is detached, so it
/// has no chain up to `root` for an observer to test, and after
/// [`NodeHandle::discard`] the backend may have retired it. See the module docs.
///
/// A container whose per-item derivation depends on a sibling's *position* —
/// `Stepper`'s index and state — wants this half as well as the other. One whose
/// default is positionless — `List::icon`, `RadioGroup::size` — does not.
/// Registering twice on one node replaces the first observer.
pub fn on_child_removed(root: &NodeHandle, f: impl Fn(&NodeHandle) + 'static) {
    register(root, Half::Removed, f);
}

fn register(root: &NodeHandle, half: Half, f: impl Fn(&NodeHandle) + 'static) {
    let key = (root.doc_key(), root.node_id());
    let observer: Observer = Rc::new(f);
    OBSERVERS.with(|map| {
        if map
            .borrow_mut()
            .entry(key)
            .or_default()
            .slot(half)
            .replace(observer)
            .is_none()
        {
            bump(half, 1);
        }
    });
    crate::reactive::on_cleanup(move || forget(key, half));
}

/// Drop one half of the registration for `key`, if it is there.
///
/// Only that half: the two are registered independently — possibly from two
/// scopes with different lifetimes — so one being released must not take the
/// other with it. The entry goes when nothing is left in it.
fn forget(key: (u64, NodeId), half: Half) {
    OBSERVERS.with(|map| {
        let mut map = map.borrow_mut();
        let Some(entry) = map.get_mut(&key) else {
            return;
        };
        if entry.slot(half).take().is_some() {
            bump(half, -1);
        }
        if entry.inserted.is_none() && entry.removed.is_none() {
            map.remove(&key);
        }
    });
}

/// Drop **both** observers registered on `node`, if any — called when a node is
/// discarded, since the backend may then hand its id to something else.
pub(super) fn forget_node(node: &NodeHandle) {
    if count_of(Half::Inserted) == 0 && count_of(Half::Removed) == 0 {
        return;
    }
    let key = (node.doc_key(), node.node_id());
    forget(key, Half::Inserted);
    forget(key, Half::Removed);
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
/// it — see **What bypasses this** in the module docs for the enumeration, and
/// for the one entry there that can genuinely land a child inside a registered
/// container ([`UpdateBatch::apply`](super::UpdateBatch::apply), #756).
pub(super) fn notify_inserted(parent: &NodeHandle, inserted: &NodeHandle) {
    notify(parent, inserted, Half::Inserted);
}

/// Tell every registered ancestor of `parent` that a subtree left it
/// (issue #745).
///
/// `parent` is the node the subtree was taken **out of**, so every caller has to
/// read it *before* the detach — [`vacated_parent`] is that read, and it is the
/// reason this half is not simply a flag on the other one. The observers are
/// handed `parent` itself; see the module docs for why not the node that went.
///
/// Called from the four [`NodeHandle`] methods that take a node out of a tree
/// (`remove_child`, `remove`, `discard`, and `replace_with` for the node it
/// displaces) plus the implicit detach an insertion verb performs when handed a
/// node that already has a parent. The same direct-`DomDocument` caveat applies
/// — but **not** to the same list: the seven sites named for the insertion half
/// are all insertions. The module docs' **What bypasses this** enumerates the
/// removal ones separately.
pub(super) fn notify_removed(parent: &NodeHandle) {
    notify(parent, parent, Half::Removed);
}

/// The parent `node` is about to be taken out of, or `None` when no observer on
/// the thread is watching for removals.
///
/// Every removal verb calls this **before** it mutates the document, because
/// afterwards there is nothing left to ask. The `Cell` read is what keeps that
/// from costing an upgrade-and-borrow on every removal in an app whose
/// containers only care about arrivals.
pub(super) fn vacated_parent(node: &NodeHandle) -> Option<NodeHandle> {
    if count_of(Half::Removed) == 0 || DISPATCHING.with(|d| d.get()) {
        return None;
    }
    node.parent_node()
}

/// Tell `vacated` a child left it, unless that is the node the child landed in.
///
/// A reorder inside one parent is a detach and a re-attach under the same node,
/// and its child set is unchanged; the insertion half already tells every
/// observer that something moved there, so a second call would only make an
/// idempotent container re-derive twice.
pub(super) fn notify_vacated(vacated: Option<&NodeHandle>, landed_in: &NodeHandle) {
    if let Some(vacated) = vacated
        && vacated.node_id() != landed_in.node_id()
    {
        notify_removed(vacated);
    }
}

/// The shared ancestor walk. `subject` is what the observers are handed.
fn notify(parent: &NodeHandle, subject: &NodeHandle, half: Half) {
    if count_of(half) == 0 || DISPATCHING.with(|d| d.get()) {
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
            if let Some(observer) = map.get(&(doc_key, current)).and_then(|e| e.observer(half)) {
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
        observer(subject);
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
            forget((root.doc_key(), root.node_id()), Half::Inserted);
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
        forget((outer.doc_key(), outer.node_id()), Half::Inserted);
        forget((inner.doc_key(), inner.node_id()), Half::Inserted);
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
        forget((root.doc_key(), root.node_id()), Half::Inserted);
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

        let before = count_of(Half::Inserted);
        assert!(before > 0, "positive control: the observer was registered");
        root.discard();
        assert_eq!(
            count_of(Half::Inserted),
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
        forget((a.doc_key(), a.node_id()), Half::Inserted);
    }

    // ------------------------------------------- the removal half (issue #745)

    /// Register a removal observer and hand back the ids it was told about.
    fn watch_removals(root: &NodeHandle) -> Seen {
        let (seen, sink) = recorder();
        on_child_removed(root, move |node| sink.borrow_mut().push(node.node_id()));
        seen
    }

    #[test]
    fn each_of_the_four_removal_verbs_tells_the_observer() {
        for verb in ["remove_child", "remove", "discard", "replace_with"] {
            let (d, body) = doc();
            let root = element(&d, "div");
            body.append_child(&root);
            let doomed = element(&d, "span");
            root.append_child(&doomed);

            let seen = watch_removals(&root);

            match verb {
                "remove_child" => root.remove_child(&doomed),
                "remove" => doomed.remove(),
                "discard" => doomed.discard(),
                _ => doomed.replace_with(&element(&d, "b")),
            }

            assert_eq!(
                *seen.borrow(),
                vec![root.node_id()],
                "`{verb}` takes a node out of a tree, so it has to tell the \
                 container that node left — and it is handed the container, not \
                 the node, which is detached by then and may be retired \
                 (issue #745)"
            );
            forget((root.doc_key(), root.node_id()), Half::Removed);
        }
    }

    #[test]
    fn an_insertion_does_not_fire_the_removal_half_or_the_other_way_round() {
        let (d, body) = doc();
        let root = element(&d, "div");
        body.append_child(&root);
        let child = element(&d, "span");
        root.append_child(&child);

        let (inserted, sink) = recorder();
        on_child_inserted(&root, move |node| sink.borrow_mut().push(node.node_id()));
        let removed = watch_removals(&root);

        let fresh = element(&d, "b");
        root.append_child(&fresh);
        assert_eq!(
            *inserted.borrow(),
            vec![fresh.node_id()],
            "positive control: the insertion half fired"
        );
        assert!(
            removed.borrow().is_empty(),
            "an arrival is not a departure: a container that registered only \
             for removals must not be woken by growth"
        );

        child.remove();
        assert_eq!(
            *removed.borrow(),
            vec![root.node_id()],
            "positive control: the removal half fired"
        );
        assert_eq!(
            inserted.borrow().len(),
            1,
            "and the insertion half was not fired again by it"
        );

        forget((root.doc_key(), root.node_id()), Half::Inserted);
        forget((root.doc_key(), root.node_id()), Half::Removed);
    }

    #[test]
    fn a_move_tells_the_container_it_left_as_well_as_the_one_it_landed_in() {
        let (d, body) = doc();
        let from = element(&d, "div");
        let to = element(&d, "div");
        body.append_child(&from);
        body.append_child(&to);
        let child = element(&d, "span");
        from.append_child(&child);

        let left = watch_removals(&from);
        let (arrived, sink) = recorder();
        on_child_inserted(&to, move |node| sink.borrow_mut().push(node.node_id()));

        // An insertion verb handed a node that already has a parent: the
        // implicit detach a `for` reconcile relocates a row with.
        to.append_child(&child);

        assert_eq!(
            *arrived.borrow(),
            vec![child.node_id()],
            "positive control: the receiving container was told"
        );
        assert_eq!(
            *left.borrow(),
            vec![from.node_id()],
            "#745: and so was the one it came out of — a move is a removal for \
             the parent it leaves, which no removal *verb* ever runs for"
        );

        forget((from.doc_key(), from.node_id()), Half::Removed);
        forget((to.doc_key(), to.node_id()), Half::Inserted);
    }

    #[test]
    fn a_reorder_inside_one_parent_is_not_reported_as_a_removal() {
        let (d, body) = doc();
        let root = element(&d, "div");
        body.append_child(&root);
        let first = element(&d, "span");
        let second = element(&d, "b");
        root.append_child(&first);
        root.append_child(&second);

        let removed = watch_removals(&root);

        // What a keyed `for` reorder does: relocate a live node under the parent
        // it is already in.
        root.insert_before(&second, &first);

        assert!(
            removed.borrow().is_empty(),
            "a reorder leaves the container's child *set* alone, and the \
             insertion half already reports it. Firing both would make an \
             idempotent container re-derive twice per moved row (issue #745)"
        );

        forget((root.doc_key(), root.node_id()), Half::Removed);
    }

    #[test]
    fn every_registered_ancestor_is_told_of_a_removal_nearest_first() {
        let (d, body) = doc();
        let outer = element(&d, "div");
        let inner = element(&d, "div");
        body.append_child(&outer);
        outer.append_child(&inner);
        let leaf = element(&d, "span");
        inner.append_child(&leaf);

        let order = Rc::new(RefCell::new(Vec::new()));
        let sink = order.clone();
        on_child_removed(&outer, move |_| sink.borrow_mut().push("outer"));
        let sink = order.clone();
        on_child_removed(&inner, move |_| sink.borrow_mut().push("inner"));

        leaf.remove();

        assert_eq!(
            *order.borrow(),
            vec!["inner", "outer"],
            "the removal half walks the same chain the insertion half does, so \
             a container nested in another still decides for itself"
        );
        forget((outer.doc_key(), outer.node_id()), Half::Removed);
        forget((inner.doc_key(), inner.node_id()), Half::Removed);
    }

    #[test]
    fn a_removal_callbacks_own_edits_do_not_call_it_back() {
        let (d, body) = doc();
        let root = element(&d, "div");
        body.append_child(&root);
        let a = element(&d, "span");
        let b = element(&d, "span");
        root.append_child(&a);
        root.append_child(&b);

        let runs = Rc::new(RefCell::new(0));
        let sink = runs.clone();
        let b_in_callback = b.clone();
        on_child_removed(&root, move |_| {
            *sink.borrow_mut() += 1;
            // A removal observer re-deriving its container may well remove
            // something: an unguarded dispatch is an unbounded loop here as much
            // as on the insertion side.
            b_in_callback.remove();
        });

        a.remove();

        assert_eq!(
            *runs.borrow(),
            1,
            "suppression covers both halves — they share one `DISPATCHING` flag"
        );
        forget((root.doc_key(), root.node_id()), Half::Removed);
    }

    #[test]
    fn releasing_one_half_leaves_the_other_registered() {
        let (d, body) = doc();
        let root = element(&d, "div");
        body.append_child(&root);
        let child = element(&d, "span");
        root.append_child(&child);

        let inserts = Rc::new(RefCell::new(0));
        let removals = Rc::new(RefCell::new(0));
        {
            let scope = crate::dom::RenderScope::new(
                d.clone() as Rc<RefCell<dyn DomDocument>>,
                body.node_id(),
            );
            let owner = scope.push_owner();
            let sink = removals.clone();
            on_child_removed(&root, move |_| *sink.borrow_mut() += 1);
            drop(owner);
            // The other half registered outside that scope, so it keeps app
            // lifetime: the two are separate registrations with separate owners
            // and one being released must not take the other with it.
            let sink = inserts.clone();
            on_child_inserted(&root, move |_| *sink.borrow_mut() += 1);

            child.remove();
            assert_eq!(*removals.borrow(), 1, "positive control");
        }

        root.append_child(&child);
        assert_eq!(
            *inserts.borrow(),
            1,
            "the insertion half survives its sibling's release"
        );
        child.remove();
        assert_eq!(
            *removals.borrow(),
            1,
            "and the removal half is gone with the scope that registered it"
        );

        forget((root.doc_key(), root.node_id()), Half::Inserted);
    }

    #[test]
    fn discarding_the_container_drops_both_of_its_observers() {
        let (d, body) = doc();
        let root = element(&d, "div");
        body.append_child(&root);

        on_child_inserted(&root, |_| {});
        on_child_removed(&root, |_| {});
        assert_eq!(
            (count_of(Half::Inserted), count_of(Half::Removed)),
            (1, 1),
            "positive control: both halves were registered"
        );

        root.discard();
        assert_eq!(
            (count_of(Half::Inserted), count_of(Half::Removed)),
            (0, 0),
            "a discarded id may be handed to the next node the document mints, \
             so `forget_node` has to release both halves and not only the one \
             it knew about before #745"
        );
    }

    #[test]
    fn an_update_batch_bypasses_both_halves() {
        // **This pins a documented hole, not a behaviour worth keeping** — see
        // "What bypasses this" in the module docs and issue #756. `UpdateBatch`
        // is prelude-exported and its four structural arms take arbitrary ids,
        // so a node moved through one reaches no observer. If you route those
        // arms through `NodeHandle`'s verbs, this test and those two doc
        // paragraphs go together.
        use crate::dom::{DomUpdate, UpdateBatch};

        let (d, body) = doc();
        let root = element(&d, "div");
        body.append_child(&root);
        let child = element(&d, "span");
        root.append_child(&child);

        let (inserted, sink) = recorder();
        on_child_inserted(&root, move |node| sink.borrow_mut().push(node.node_id()));
        let removed = watch_removals(&root);

        let fresh = element(&d, "b");
        let mut batch = UpdateBatch::new();
        batch.push(DomUpdate::AppendChild {
            parent: root.node_id(),
            child: fresh.node_id(),
        });
        batch.push(DomUpdate::RemoveChild {
            parent: root.node_id(),
            child: child.node_id(),
        });
        {
            let mut doc = d.borrow_mut();
            batch.apply(&mut *doc);
        }

        assert_eq!(
            root.children().len(),
            1,
            "positive control: the batch really did restructure the tree — \
             without this the two zeroes below would be a test of nothing"
        );
        assert!(
            inserted.borrow().is_empty() && removed.borrow().is_empty(),
            "neither half hears a batch: it is handed a `&mut dyn DomDocument` \
             and a notification needs the `Rc` a `NodeHandle` holds a `Weak` of"
        );

        forget((root.doc_key(), root.node_id()), Half::Inserted);
        forget((root.doc_key(), root.node_id()), Half::Removed);
    }
}
