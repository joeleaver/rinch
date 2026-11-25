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
//!
//! - **A batched `DomUpdate::SetText` on an element with children**
//!   ([`UpdateBatch::apply`](super::UpdateBatch::apply)). `set_text_content`
//!   on an element replaces its child list with the text, so this one *can*
//!   take children out of a registered container, and it tells nobody: `apply`
//!   is handed the document already mutably borrowed, and an observer edits
//!   the tree through `NodeHandle`s, which borrow it again. Use
//!   [`NodeHandle::set_text`], which does tell (below).
//!
//! # Writes that replace a child list
//!
//! [`NodeHandle::set_text`] on an **element** and
//! [`NodeHandle::set_inner_html`] replace every child the node has, and fire
//! the removal half for it when it had any (issue #1440): once, with the node
//! itself, which is the parent the children left. `set_text` asks the document
//! afterwards whether the list changed — an element whose only child is a
//! text node keeps that node on `rinch-dom` (it is written in place), has no
//! listed child on the mock or in a browser, and so tells nobody on any
//! backend; `set_inner_html` frees
//! the children and always tells. Neither fires the **insertion** half for
//! what arrives: the text has no node of its own in a browser, and markup
//! parsed by `set_inner_html` is not reported — a container whose items are
//! written as an HTML string does not give them its default.
//! `set_inner_html` also drops the observers registered on the nodes it
//! frees, as a discard drops its node's.
//!
//! [`UpdateBatch`](super::UpdateBatch) used to be on this list for four
//! structural `DomUpdate` variants that took arbitrary ids; #756 removed them,
//! and a batch carries property writes only.
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
//!
//! The cleanup releases **the registration it was made for**, by a token, not
//! whatever stands under the node's id when it runs (issue #1490). The two
//! differ when the registration has been replaced meanwhile — by a second
//! `on_child_*` call on the node, or, on `rinch-dom`, by another container
//! altogether: that backend hands a freed node id to the next node it mints,
//! and a cleanup that forgot by id then dropped the new container's observer.

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

/// One registered observer and the token of the `on_child_*` call that
/// registered it (issue #1490). The token is what that call's scope cleanup
/// names, so it can only release its own registration.
type Registered = (u64, Observer);

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
    inserted: Option<Registered>,
    removed: Option<Registered>,
}

impl Entry {
    fn slot(&mut self, half: Half) -> &mut Option<Registered> {
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
        .map(|(_, observer)| observer)
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

    /// The next registration token (issue #1490). Never reused.
    static NEXT_TOKEN: Cell<u64> = const { Cell::new(0) };
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

/// How many insertion observers are registered on this thread: once it is
/// above zero, every insertion on the thread walks its parent's ancestors.
/// What a test asserts a container leaves alone (#3062). Not part of the API.
#[doc(hidden)]
pub fn insert_observer_count() -> usize {
    count_of(Half::Inserted)
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
    let token = NEXT_TOKEN.with(|next| {
        let token = next.get();
        next.set(token + 1);
        token
    });
    OBSERVERS.with(|map| {
        if map
            .borrow_mut()
            .entry(key)
            .or_default()
            .slot(half)
            .replace((token, observer))
            .is_none()
        {
            bump(half, 1);
        }
    });
    // By token: by the time the scope goes, the node may have been discarded
    // or freed and its id handed to another container, whose registration
    // under the same key is not this scope's to release (issue #1490).
    crate::reactive::on_cleanup(move || release(key, half, Some(token)));
}

/// Drop one half of the registration for `key`, if it is there — and, given
/// `only`, if it is still the registration that token was issued for.
///
/// `only` is the scope-cleanup route (issue #1490): the registration may have
/// been replaced since, and the replacement belongs to somebody else. `None`
/// is the node itself going away, which takes whatever is registered on it.
///
/// Only that half: the two are registered independently — possibly from two
/// scopes with different lifetimes — so one being released must not take the
/// other with it. The entry goes when nothing is left in it.
fn release(key: (u64, NodeId), half: Half, only: Option<u64>) {
    OBSERVERS.with(|map| {
        let mut map = map.borrow_mut();
        let Some(entry) = map.get_mut(&key) else {
            return;
        };
        let slot = entry.slot(half);
        if slot
            .as_ref()
            .is_some_and(|(token, _)| only.is_none_or(|only| only == *token))
        {
            *slot = None;
            bump(half, -1);
        }
        if entry.inserted.is_none() && entry.removed.is_none() {
            map.remove(&key);
        }
    });
}

/// Drop whatever is registered for one half of `key`: the node is going away.
fn forget(key: (u64, NodeId), half: Half) {
    release(key, half, None);
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

crate::reactive::restore::restore_slot!(DispatchingSlot: bool = DISPATCHING);

/// Run `f` with no child observer told of what it does — for a caller taking
/// nodes out of a subtree that is about to be retired, whose observers could
/// only patch what is being thrown away (issue #1487).
pub(super) fn without_notifications<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            let was = self.0;
            DISPATCHING.with(|d| d.set(was));
        }
    }
    let _restore = Restore(DISPATCHING.with(|d| d.replace(true)));
    f()
}

/// Tell every registered ancestor of `parent` that `inserted` landed.
///
/// Called from the four [`NodeHandle`] methods that put a node into a tree.
/// Anything that reaches for [`super::traits::DomDocument`] directly bypasses
/// it — see **What bypasses this** in the module docs for the enumeration.
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

/// The children `node` has now, when a write that **replaces its child list**
/// (`NodeHandle::set_text` on an element, `NodeHandle::set_inner_html`) could
/// take them out from under a removal observer — issue #1440.
///
/// `None` when nothing on the thread watches for removals, during a dispatch,
/// for a text node (what every reactive `{|| …}` in `rsx!` writes to — asked
/// through [`DomTree::is_text_node`](super::traits::DomTree::is_text_node),
/// with no child list read), and for an element with no children. So a write
/// nobody could be told about costs the `Cell` read and no document call.
pub(super) fn children_at_stake(node: &NodeHandle) -> Option<Vec<NodeId>> {
    if count_of(Half::Removed) == 0 || DISPATCHING.with(|d| d.get()) {
        return None;
    }
    let doc = node.doc_upgrade()?;
    let doc = doc.borrow();
    // A text node has no child list to read: this is the write of every
    // reactive text, and it stays one kind check with an observer registered.
    if doc.is_text_node(node.node_id()) {
        return None;
    }
    let children = doc.get_children(node.node_id());
    (!children.is_empty()).then_some(children)
}

/// Tell the observers above `node` that children left it, after a text write
/// that had `before` ([`children_at_stake`]) to lose.
///
/// Asked of the document rather than assumed, because a backend may keep the
/// list: `rinch-dom` writes an element's lone text child in place, and the
/// child list is then the one it was.
pub(super) fn notify_children_replaced(node: &NodeHandle, before: Option<Vec<NodeId>>) {
    let Some(before) = before else {
        return;
    };
    let Some(doc) = node.doc_upgrade() else {
        return;
    };
    let after = doc.borrow().get_children(node.node_id());
    if after != before {
        notify_removed(node);
    }
}

/// Drop the observers registered on any node **beneath** `node`, before a
/// write that frees those nodes (`NodeHandle::set_inner_html`).
///
/// The same reason [`forget_node`] exists: `rinch-dom` hands a freed slab id
/// to the next node it mints — the very markup the write parses — and an
/// observer left under that id would answer for a node that is not its
/// container.
pub(super) fn forget_descendants(node: &NodeHandle) {
    if count_of(Half::Inserted) == 0 && count_of(Half::Removed) == 0 {
        return;
    }
    let Some(doc) = node.doc_upgrade() else {
        return;
    };
    let doc_key = node.doc_key();
    let mut stack = doc.borrow().get_children(node.node_id());
    while let Some(id) = stack.pop() {
        forget((doc_key, id), Half::Inserted);
        forget((doc_key, id), Half::Removed);
        stack.extend(doc.borrow().get_children(id));
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

    // Restores `DISPATCHING` however a callback leaves the stack.
    let _guard = crate::reactive::restore::RestoreCell::<DispatchingSlot>::replace(true);
    // Untracked (#931): an insertion or removal is very often made from inside
    // an effect — every `for` reconcile, `if` branch swap and component
    // re-render — and an observer's reads are not that effect's dependencies.
    crate::reactive::untracked_handler(|| {
        for observer in observers {
            observer(subject);
        }
    });
}

#[cfg(test)]
mod insertion_tests;
#[cfg(test)]
mod observer_tests;
#[cfg(test)]
mod removal_tests;
#[cfg(test)]
mod replace_tests;
#[cfg(test)]
mod test_support;
