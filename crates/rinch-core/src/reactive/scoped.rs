//! Scoped global callback registries (issue #183).
//!
//! A thread-local registry that holds a user callback outlives the component
//! that filled it. Since #141 PR4 a scope *owns* the signals and memos created
//! while it was the ambient owner, and disposing it frees them — so a stale
//! callback that captured one of those handles hands the next event a disposed
//! component's state, and a read of a freed [`Signal`](crate::reactive::Signal)
//! panics.
//!
//! This module holds the one copy of the fix: register the callback, and tie its
//! *removal* to the scope that registered it via
//! [`on_cleanup`](crate::reactive::on_cleanup). It is the "cleanup template",
//! first written inline for the paste interceptor in
//! [`crate::events::set_paste_interceptor`] and extracted here so the keyboard,
//! selection and paste interceptors share it verbatim rather than each
//! paraphrasing it. Those slots are its callers, and the shape they have in
//! common is the shape it fits: **one slot, written once per component.**
//! Since #340/#478 those interceptors use the doc-keyed variant below —
//! [`install_doc_scoped_slot`] — which is this template replicated per
//! document, one slot per `(document)` with the same lifetime rules per entry.
//!
//! It is deliberately not the only such template in the codebase, and the other
//! one is not a lesser variant of it — see [When *not* to use this](#when-not-to-use-this).
//!
//! # The three rules it encodes
//!
//! 1. **Ownerless registration keeps app lifetime.** `on_cleanup` returns
//!    `false` when there is no live ambient owner — outside any render, from
//!    `main`, from a timer — and does nothing. That is the pre-#141 default and
//!    must stay: menus, for instance, are built from `main` before the event
//!    loop starts, and requiring an owner would stop them working.
//! 2. **Only reclaim what is still yours.** The cleanup upgrades a `Weak` to the
//!    value it installed and compares it with [`Rc::ptr_eq`] against what the
//!    registry holds *now*. Without that check an earlier component unmounting
//!    would clobber a later component's registration. A failed upgrade means a
//!    later registration already replaced yours and owns the slot, so returning
//!    early is correct rather than a leak.
//! 3. **Drop the displaced value after the borrow ends.** The value being
//!    replaced is user code whose `Drop` may re-enter the registry; dropping it
//!    inside the `borrow_mut` panics. Every write here binds it to a `let` that
//!    outlives the borrow. The read and clear halves —
//!    [`read_scoped_slot`] and [`clear_scoped_slot`] — encode the same rule, so
//!    a registry's `dispatch`/`clear` pair does not have to paraphrase it
//!    either.
//!
//! A cleanup runs from `Scope::dispose`, which is reachable from a `Drop` at
//! thread exit (a TLS destructor, when the slot's own thread-local may already
//! be gone) and from a drop on the unwind path. Both cleanups therefore use
//! `try_with`/`try_borrow_mut` and degrade to "not reclaimed" rather than
//! panicking — the same stance as [`drain_polls`](crate::reactive::drain_polls).
//!
//! # When *not* to use this
//!
//! One cleanup is registered per call, and the scope's cleanup vec grows with
//! it. That is right for a registry written once (or a handful of times) per
//! component, which is how every in-tree caller uses it today.
//!
//! It is **not** bounded for a registry written repeatedly from inside a live
//! component, and these are public APIs, so that is reachable: an event handler
//! re-enters its registration-time owner on dispatch (see
//! `crate::events::register_handler`), and an [`Effect`](crate::reactive::Effect)
//! re-pushes its creation-time owner on every run. So an `onclick` that installs
//! an interceptor, or an interceptor installed from a re-running effect, appends
//! one boxed cleanup — plus a `Weak` that pins the old allocation — per
//! invocation, for as long as the component lives. Such a registry (a debounce
//! parking a fresh callback each keystroke is the archetype) must instead carry
//! an [`Owner`](crate::reactive::Owner) beside the callback and check
//! [`is_alive`](crate::reactive::Owner::is_alive) at dispatch, the way
//! [`crate::main_thread::park_main_callback`] does. Tightening
//! `install_scoped_slot` itself — one cleanup per (slot, owner), the later
//! install updating a shared cell rather than queueing another release — would
//! close it here instead, and is the obvious follow-up if a repeat-registering
//! caller ever appears.
//!
//! This is not hypothetical. A keyed `install_scoped_entry` used to live beside
//! `install_scoped_slot`, shipped ahead of the two registries it was written
//! for. **Both of them rejected it, on exactly the grounds above, and it was
//! removed without a caller ever being written** (issue #376):
//!
//! - The **menu** registry (`rinch::menu`) found it reclaims by the wrong scope.
//!   Removal is tied to whichever scope is ambient when the entry is
//!   *installed* — the component that **built** the menu, not the ones that own
//!   the individual items. A component assembling a menu out of items
//!   contributed by other, still-live components silently disabled all of them
//!   when it unmounted. It also appended one cleanup and one pinning `Weak` per
//!   item per rebuild, because a menu callback dispatches under its own owner,
//!   so a rebuild from inside a callback re-registers with that owner ambient.
//! - The **websocket** registry (`rinch-ws`) declined it before adopting it:
//!   registration there is per-`on_message` call and a component may re-register
//!   freely, so one cleanup per call grows without bound.
//!
//! Both took the owner-beside-the-callback template instead. The lesson is not
//! that a keyed form is impossible, but that *keying* is itself the warning
//! sign: a registry is keyed because it holds many entries that churn, which is
//! precisely the shape this template is least suited to. For a keyed registry,
//! reach for the dispatch check first.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};
use std::thread::LocalKey;

use super::on_cleanup;
use crate::context::current_dispatching_doc;

/// Install `value` into a single-slot registry, tying its removal to the scope
/// that is currently rendering.
///
/// Returns `true` when a cleanup was registered — i.e. there was a live ambient
/// owner. `false` means there was none, so `value` keeps **app lifetime**, which
/// is the correct outcome for a registration made from `main`, a timer or a
/// detached callback; it is not a failure.
///
/// Two further rules make it correct, and both are load-bearing:
///
/// - **Only reclaim what is still yours.** The cleanup compares the value it
///   installed against what the slot holds *now* ([`Rc::ptr_eq`]); without that,
///   an earlier component unmounting would clobber a later one's registration.
/// - **Drop the displaced value after the borrow ends.** It is user code whose
///   `Drop` may re-enter the slot, which inside the `borrow_mut` would panic.
pub fn install_scoped_slot<T>(slot: &'static LocalKey<RefCell<Option<Rc<T>>>>, value: Rc<T>) -> bool
where
    T: ?Sized + 'static,
{
    let mine: Weak<T> = Rc::downgrade(&value);
    // Rule 3: the displaced value is dropped when `_previous` goes out of scope
    // at the end of this function, long after the `borrow_mut` has ended.
    let _previous = slot.with(|s| s.borrow_mut().replace(value));
    on_cleanup(move || {
        let Some(ours) = mine.upgrade() else {
            // Rule 2: already replaced by a later registration, which owns the
            // slot now. Leaving it alone is the point of the check.
            return;
        };
        // `try_with`/`try_borrow_mut`: this can run from a TLS destructor at
        // thread exit (when `slot` may already be gone) or while unwinding, and
        // must degrade to "not reclaimed" rather than panic-in-panic.
        let _displaced = slot.try_with(|s| {
            let Ok(mut current) = s.try_borrow_mut() else {
                return None;
            };
            if current
                .as_ref()
                .is_some_and(|installed| Rc::ptr_eq(installed, &ours))
            {
                current.take()
            } else {
                None
            }
        });
    })
}

/// Clone the value out of a single-slot registry so it can be **called** with no
/// borrow held.
///
/// The read half of [`install_scoped_slot`]: holding the slot's `borrow()`
/// across a user callback makes it a double-borrow panic for that callback to
/// install its replacement (or clear the slot), which the setters here allow.
pub fn read_scoped_slot<T>(slot: &'static LocalKey<RefCell<Option<Rc<T>>>>) -> Option<Rc<T>>
where
    T: ?Sized + 'static,
{
    slot.with(|s| s.borrow().clone())
}

/// Empty a single-slot registry, dropping the value **after** the borrow ends.
///
/// The clear half of [`install_scoped_slot`], and rule 3 in one place: the value
/// being removed is user code whose `Drop` may re-enter the slot, which under
/// the `borrow_mut` would panic. Any cleanup the registering scope holds is left
/// in place and becomes a no-op — its `Weak` can no longer upgrade.
pub fn clear_scoped_slot<T>(slot: &'static LocalKey<RefCell<Option<Rc<T>>>>)
where
    T: ?Sized + 'static,
{
    let _previous = slot.with(|s| s.borrow_mut().take());
}

// ============================================================================
// Doc-keyed slots (issues #340, #478)
// ============================================================================

/// The map behind a doc-keyed slot: one entry per registering document, plus
/// the `None` entry for registrations made outside any document.
///
/// **Every entry carries the order it was written in, and a read takes the
/// newer of the caller's own entry and the fallback** (review of PR #960).
/// Since #295 a `RinchApp` mount and every effect a document owns are marked,
/// so a component body's registration lands under its document — while a
/// timer, `run_on_main_thread`, an http/ws completion or an effect created in
/// `main` is unmarked and lands on the fallback. Were the document's entry to
/// win unconditionally, a single-document app could never replace, nor clear,
/// the interceptor it registered at mount. Taking the newer keeps the rule the
/// single slot this replaced had: the last registration wins, and a clear
/// clears — see [`clear_doc_scoped_slot`] for the clear's half.
///
/// `BTreeMap` rather than `HashMap` for the `const` initializer — the key set
/// is a handful of documents, so lookup cost is irrelevant.
pub struct DocScopedSlotMap<T: ?Sized> {
    entries: BTreeMap<Option<u64>, SlotEntry<T>>,
    /// Monotonic per slot; the next write's stamp.
    next_epoch: u64,
}

/// One document's entry. `value: None` is a **clear made inside that
/// document** while a fallback exists: it shadows the fallback for that
/// document alone, until something newer is written. Only `Some` keys ever
/// hold one, and they are purged as soon as there is no fallback left for them
/// to shadow, so they are bounded by the number of documents.
struct SlotEntry<T: ?Sized> {
    epoch: u64,
    value: Option<Rc<T>>,
}

impl<T: ?Sized> DocScopedSlotMap<T> {
    /// An empty slot — `const`, for a `thread_local!` initializer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            next_epoch: 1,
        }
    }

    fn stamp(&mut self) -> u64 {
        let epoch = self.next_epoch;
        self.next_epoch += 1;
        epoch
    }

    /// The entry a caller in `caller`'s document is served: the newer of its
    /// own and the fallback. `None` for a clear that shadows the fallback.
    fn resolve(&self, caller: Option<u64>) -> Option<&Rc<T>> {
        let fallback = self.entries.get(&None);
        let own = caller.and_then(|doc| self.entries.get(&Some(doc)));
        let winner = match (own, fallback) {
            (Some(own), Some(fallback)) => {
                if own.epoch > fallback.epoch {
                    own
                } else {
                    fallback
                }
            }
            (Some(entry), None) | (None, Some(entry)) => entry,
            (None, None) => return None,
        };
        winner.value.as_ref()
    }

    /// Write `value` under `key`, returning what it displaced (to be dropped
    /// by the caller once the borrow is released).
    fn install(&mut self, key: Option<u64>, value: Rc<T>) -> Option<Rc<T>> {
        let epoch = self.stamp();
        let entry = SlotEntry {
            epoch,
            value: Some(value),
        };
        let displaced = self.entries.insert(key, entry).and_then(|e| e.value);
        if key.is_none() {
            // A fallback newer than every clear: no clear shadows anything now.
            self.purge_clears();
        }
        displaced
    }

    /// Drop every clear-marker: called whenever the fallback is replaced or
    /// removed, after which none of them shadows anything.
    fn purge_clears(&mut self) {
        self.entries.retain(|_, e| e.value.is_some());
    }

    /// The clear rule; see [`clear_doc_scoped_slot`].
    fn clear(&mut self, caller: Option<u64>) -> Vec<Rc<T>> {
        match caller {
            None => std::mem::take(&mut self.entries)
                .into_values()
                .filter_map(|e| e.value)
                .collect(),
            Some(doc) => {
                let own = self.entries.get(&Some(doc)).map(|e| e.epoch);
                let fallback = self.entries.get(&None).map(|e| e.epoch);
                let mut removed = Vec::new();
                match (own, fallback) {
                    (None, None) => {}
                    // The fallback is what this document is served (it has no
                    // entry, or only an older one): the fallback goes — for
                    // everyone, the #478 rule — and so does this document's
                    // older entry, which must not resurface for it.
                    (None, Some(_)) => removed.extend(self.remove_fallback()),
                    (Some(own), Some(fallback)) if fallback > own => {
                        removed.extend(self.remove_fallback());
                        removed.extend(self.entries.remove(&Some(doc)).and_then(|e| e.value));
                    }
                    // Its own entry is what it is served: that goes, and an
                    // older fallback — which serves the other documents and
                    // stays for them — is shadowed for this one by a
                    // clear-marker, so it does not come back here.
                    (Some(_), fallback) => {
                        removed.extend(self.entries.remove(&Some(doc)).and_then(|e| e.value));
                        if fallback.is_some() {
                            let epoch = self.stamp();
                            self.entries
                                .insert(Some(doc), SlotEntry { epoch, value: None });
                        }
                    }
                }
                removed
            }
        }
    }

    /// Remove the fallback and, with it, every clear-marker (which only ever
    /// shadowed it).
    fn remove_fallback(&mut self) -> Option<Rc<T>> {
        let removed = self.entries.remove(&None).and_then(|e| e.value);
        self.purge_clears();
        removed
    }

    /// Remove `key`'s entry if it still holds `ours` (the unmount cleanup's
    /// rule 2).
    fn remove_if_ours(&mut self, key: Option<u64>, ours: &Rc<T>) -> Option<Rc<T>> {
        let is_ours = self
            .entries
            .get(&key)
            .and_then(|e| e.value.as_ref())
            .is_some_and(|installed| Rc::ptr_eq(installed, ours));
        if !is_ours {
            return None;
        }
        let removed = self.entries.remove(&key).and_then(|e| e.value);
        if key.is_none() {
            self.purge_clears();
        }
        removed
    }
}

impl<T: ?Sized> DocScopedSlotMap<T> {
    /// Whether no registration is live in any document (clear-markers are
    /// not registrations).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.values().all(|e| e.value.is_none())
    }
}

impl<T: ?Sized> Default for DocScopedSlotMap<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// [`install_scoped_slot`], replicated **per document** (issues #340, #478).
///
/// A single slot shared by every document on the thread is last-wins: two
/// documents pumping their event streams through one thread-local (a desktop
/// app and its DevTools window, two embedded `RinchContext`s) let the second
/// registration silently disable the first, and whichever remains then drives
/// both documents — the class #134 fixed for the editor/bounds registries and
/// #139 for the pointer-capture drag. Here each document gets its own entry,
/// keyed by [`current_dispatching_doc`] at install time — which covers a
/// `RinchApp` mount and every effect a document owns, wherever it is flushed
/// (issue #295); installing with no document marked — from `main`, a timer, a
/// `run_on_main_thread` callback, or on a backend that never marks one
/// (rinch-web) — fills the ownerless `None` entry, which
/// [`read_doc_scoped_slot`] serves to every document **whose own entry is
/// older**. So the single-document app keeps the single slot's rule wherever
/// it registers from: the last registration wins.
///
/// Same growth characteristics as [`install_scoped_slot`] — one entry and one
/// cleanup per (document, component) registration, written once per component.
/// The #376 warning about keyed registries is about keys that *churn*; a
/// document key does not.
///
/// The three rules of [`install_scoped_slot`] carry over unchanged, with the
/// cleanup reclaiming only **its own document's entry** (the key is captured at
/// install, not re-read at dispose — the scope may be disposed while another
/// document, or none, is dispatching) and only while that entry still holds the
/// value it installed ([`Rc::ptr_eq`]).
pub fn install_doc_scoped_slot<T>(
    slot: &'static LocalKey<RefCell<DocScopedSlotMap<T>>>,
    value: Rc<T>,
) -> bool
where
    T: ?Sized + 'static,
{
    let key = current_dispatching_doc();
    let mine: Weak<T> = Rc::downgrade(&value);
    // Rule 3: the displaced value is dropped when `_previous` goes out of scope
    // at the end of this function, long after the `borrow_mut` has ended.
    let _previous = slot.with(|s| s.borrow_mut().install(key, value));
    on_cleanup(move || {
        let Some(ours) = mine.upgrade() else {
            // Rule 2: already replaced by a later registration from the same
            // document, which owns the entry now.
            return;
        };
        let _displaced = slot.try_with(|s| {
            let Ok(mut current) = s.try_borrow_mut() else {
                return None;
            };
            current.remove_if_ours(key, &ours)
        });
    })
}

/// Clone the value a dispatch should reach out of a doc-keyed slot, so it can
/// be **called** with no borrow held.
///
/// The dispatching document is served the **newer** of its own entry and the
/// ownerless `None` entry (see [`DocScopedSlotMap`]). A dispatch outside any
/// document reaches the `None` entry only — with several documents' entries
/// live there is no one right answer for "whose", and rinch-web (which never
/// marks dispatch) only ever *fills* the `None` entry, so this is also the
/// consistent one.
pub fn read_doc_scoped_slot<T>(
    slot: &'static LocalKey<RefCell<DocScopedSlotMap<T>>>,
) -> Option<Rc<T>>
where
    T: ?Sized + 'static,
{
    let caller = current_dispatching_doc();
    slot.with(|s| s.borrow().resolve(caller).cloned())
}

/// Clear a doc-keyed slot, so that "a clear clears" holds as it did for the
/// single slot this replaced.
///
/// - **With no document marked** (from `main`, a timer, a `run_on_main_thread`
///   callback, rinch-web): every entry goes. Unmarked code speaks for every
///   document, exactly as its install serves every document — and it is the
///   only way such code can reach a registration a mount made.
/// - **Inside a document**: the entry that document is served goes, and
///   nothing older comes back for it. When that is the fallback (the document
///   has no entry, or an older one) the fallback is removed for everyone — the
///   #478 rule, so a handler can clear what `main` registered — along with the
///   document's older entry. When it is the document's own entry, an older
///   fallback stays for the other documents and is *shadowed for this one* by
///   a clear-marker, stamped like a write. Anything written afterwards, from
///   anywhere, is newer than the marker and wins.
///
/// The values are dropped **after** the borrow ends (rule 3).
pub fn clear_doc_scoped_slot<T>(slot: &'static LocalKey<RefCell<DocScopedSlotMap<T>>>)
where
    T: ?Sized + 'static,
{
    let caller = current_dispatching_doc();
    let _previous = slot.with(|s| s.borrow_mut().clear(caller));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    use crate::reactive::Scope;

    type Probe = Rc<dyn Fn() -> u32>;

    /// The cleanup reclaims the slot only when it still holds the value that
    /// registered it, so an earlier unmount cannot clobber a later install.
    #[test]
    fn a_scoped_slot_is_reclaimed_only_by_the_scope_that_filled_it() {
        thread_local! {
            static SLOT: RefCell<Option<Probe>> = const { RefCell::new(None) };
        }
        fn read() -> Option<u32> {
            SLOT.with(|s| s.borrow().clone()).map(|f| f())
        }

        let first = Scope::new();
        assert!(
            first.run(|| install_scoped_slot(&SLOT, Rc::new(|| 1u32) as Probe)),
            "a live ambient owner means a cleanup was registered"
        );
        let second = Scope::new();
        second.run(|| install_scoped_slot(&SLOT, Rc::new(|| 2u32) as Probe));

        first.dispose();
        assert_eq!(
            read(),
            Some(2),
            "the first scope must not reclaim a slot it no longer owns"
        );

        second.dispose();
        assert_eq!(read(), None, "the owner of the slot reclaims it");
    }

    /// Rule 2 on its own, with the `Weak` guard taken out of the picture.
    ///
    /// [`a_scoped_slot_is_reclaimed_only_by_the_scope_that_filled_it`] looks like
    /// it covers this and does not: it installs `Rc::new(..)` inline, so when the
    /// second scope displaces the first value nothing holds a strong reference to
    /// it, the first scope's `Weak` fails to upgrade, and the cleanup returns
    /// early without ever reaching the [`Rc::ptr_eq`] comparison. Deleting that
    /// comparison entirely leaves that test green.
    ///
    /// Here the caller keeps its own clone alive — an ordinary thing to do, and
    /// what an interceptor the component also invokes directly looks like — so
    /// the upgrade succeeds and `ptr_eq` is the only thing standing between the
    /// first scope's disposal and a clobber of the second's registration.
    #[test]
    fn a_disposing_scope_whose_value_is_still_alive_elsewhere_does_not_clobber_the_slot() {
        thread_local! {
            static SLOT: RefCell<Option<Probe>> = const { RefCell::new(None) };
        }
        fn read() -> Option<u32> {
            SLOT.with(|s| s.borrow().clone()).map(|f| f())
        }

        // Retained by the caller, so it outlives its eviction from the slot.
        let retained: Probe = Rc::new(|| 1u32);
        let first = Scope::new();
        first.run({
            let retained = retained.clone();
            move || install_scoped_slot(&SLOT, retained)
        });

        let second = Scope::new();
        second.run(|| install_scoped_slot(&SLOT, Rc::new(|| 2u32) as Probe));

        assert_eq!(
            Rc::strong_count(&retained),
            1,
            "precondition: the slot has released the first value, but the test \
             still holds it — so the cleanup's `Weak` will upgrade and the \
             `ptr_eq` guard is actually reached"
        );

        first.dispose();
        assert_eq!(
            read(),
            Some(2),
            "the first scope's value is still alive, so its cleanup upgrades — \
             only the `Rc::ptr_eq` check stops it reclaiming a slot that now \
             belongs to the second scope"
        );

        second.dispose();
        assert_eq!(read(), None);
        drop(retained);
    }

    /// Registration outside any render has no owner. Nothing removes it, and
    /// that is the documented app-lifetime default — not a failure.
    #[test]
    fn installing_with_no_ambient_owner_returns_false_and_leaves_the_value_installed() {
        thread_local! {
            static SLOT: RefCell<Option<Probe>> = const { RefCell::new(None) };
        }

        assert!(
            !install_scoped_slot(&SLOT, Rc::new(|| 9u32) as Probe),
            "no ambient owner means no cleanup was registered"
        );
        Scope::new().dispose();
        assert_eq!(
            SLOT.with(|s| s.borrow().clone()).map(|f| f()),
            Some(9),
            "an ownerless registration keeps app lifetime"
        );
    }

    /// Rule 3: the value being replaced is user code, and its `Drop` may re-enter
    /// the registry. Dropping it inside the `borrow_mut` would panic.
    #[test]
    fn the_displaced_value_is_dropped_after_the_slots_borrow_ends() {
        thread_local! {
            static SLOT: RefCell<Option<Probe>> = const { RefCell::new(None) };
            static DROPPED: Cell<bool> = const { Cell::new(false) };
        }

        struct Reenter;
        impl Drop for Reenter {
            fn drop(&mut self) {
                // Would be a double borrow if the drop ran under the install's
                // `borrow_mut`.
                let occupied = SLOT.with(|s| s.borrow().is_some());
                assert!(occupied, "the replacement is installed by now");
                DROPPED.with(|d| d.set(true));
            }
        }

        let guard = Reenter;
        install_scoped_slot(
            &SLOT,
            Rc::new(move || {
                let _ = &guard;
                1u32
            }) as Probe,
        );
        install_scoped_slot(&SLOT, Rc::new(|| 2u32) as Probe);
        assert!(DROPPED.with(|d| d.get()), "the displaced value was dropped");
        SLOT.with(|s| s.borrow_mut().take());
    }

    /// Rule 3 for the clear half, which every registry's `clear_*` now routes
    /// through: the removed value is user code, and its `Drop` must not run
    /// under the slot's `borrow_mut`.
    #[test]
    fn clear_scoped_slot_drops_the_value_after_the_borrow_ends() {
        thread_local! {
            static SLOT: RefCell<Option<Probe>> = const { RefCell::new(None) };
            static REINSTALLED: Cell<bool> = const { Cell::new(false) };
        }

        struct Reenter;
        impl Drop for Reenter {
            fn drop(&mut self) {
                // A double borrow if the drop ran under `clear_scoped_slot`'s
                // `borrow_mut` — this is the exact shape the rule protects.
                SLOT.with(|s| *s.borrow_mut() = Some(Rc::new(|| 5u32) as Probe));
                REINSTALLED.with(|r| r.set(true));
            }
        }

        let guard = Reenter;
        install_scoped_slot(
            &SLOT,
            Rc::new(move || {
                let _ = &guard;
                1u32
            }) as Probe,
        );
        clear_scoped_slot(&SLOT);
        assert!(
            REINSTALLED.with(|r| r.get()),
            "the cleared value's Drop ran, and outside the borrow"
        );
        SLOT.with(|s| s.borrow_mut().take());
    }

    // ── the doc-keyed variant (issues #340, #478) ────────────────────────────

    /// The cleanup reclaims the entry under the key it *installed* at, however
    /// the ambient marker has moved by dispose time — and only while that
    /// entry still holds its value.
    #[test]
    fn a_doc_keyed_cleanup_reclaims_its_own_documents_entry_wherever_disposal_happens() {
        use crate::context::push_dispatching_doc;

        thread_local! {
            static SLOT: RefCell<DocScopedSlotMap<dyn Fn() -> u32>> =
                const { RefCell::new(DocScopedSlotMap::new()) };
        }

        let scope = Scope::new();
        {
            let _a = push_dispatching_doc(1);
            scope.run(|| install_doc_scoped_slot(&SLOT, Rc::new(|| 1u32) as Probe));
        }
        {
            let _b = push_dispatching_doc(2);
            install_doc_scoped_slot(&SLOT, Rc::new(|| 2u32) as Probe);
        }

        // Disposal happens while document 2 — not 1 — is dispatching.
        {
            let _b = push_dispatching_doc(2);
            scope.dispose();
        }
        {
            let _a = push_dispatching_doc(1);
            assert!(
                read_doc_scoped_slot(&SLOT).is_none(),
                "the cleanup removed document 1's entry, the one it installed"
            );
        }
        {
            let _b = push_dispatching_doc(2);
            assert_eq!(
                read_doc_scoped_slot(&SLOT).map(|f| f()),
                Some(2),
                "document 2's entry — under the marker current at dispose — was not touched"
            );
            clear_doc_scoped_slot(&SLOT);
        }
    }

    /// Rule 2 per entry: a disposing scope whose value was already replaced by
    /// a later registration *from the same document* leaves the entry alone.
    #[test]
    fn a_doc_keyed_cleanup_does_not_clobber_a_later_registration_under_the_same_key() {
        use crate::context::push_dispatching_doc;

        thread_local! {
            static SLOT: RefCell<DocScopedSlotMap<dyn Fn() -> u32>> =
                const { RefCell::new(DocScopedSlotMap::new()) };
        }

        // Retained by the caller so the cleanup's `Weak` upgrades and the
        // `ptr_eq` guard is actually reached (see the single-slot twin above).
        let retained: Probe = Rc::new(|| 1u32);
        let first = Scope::new();
        {
            let _a = push_dispatching_doc(1);
            first.run({
                let retained = retained.clone();
                move || install_doc_scoped_slot(&SLOT, retained)
            });
            install_doc_scoped_slot(&SLOT, Rc::new(|| 2u32) as Probe);
        }

        first.dispose();
        {
            let _a = push_dispatching_doc(1);
            assert_eq!(
                read_doc_scoped_slot(&SLOT).map(|f| f()),
                Some(2),
                "the first scope's cleanup must not reclaim an entry that now \
                 belongs to a later registration"
            );
            clear_doc_scoped_slot(&SLOT);
        }
        drop(retained);
    }

    /// Rule 3 on the doc-keyed install path: the displaced value's `Drop` may
    /// re-enter the map and must run after the `borrow_mut` ends.
    #[test]
    fn the_doc_keyed_displaced_value_is_dropped_after_the_maps_borrow_ends() {
        thread_local! {
            static SLOT: RefCell<DocScopedSlotMap<dyn Fn() -> u32>> =
                const { RefCell::new(DocScopedSlotMap::new()) };
            static DROPPED: Cell<bool> = const { Cell::new(false) };
        }

        struct Reenter;
        impl Drop for Reenter {
            fn drop(&mut self) {
                // A double borrow if the drop ran under the install's
                // `borrow_mut`.
                let occupied = SLOT.with(|s| !s.borrow().is_empty());
                assert!(occupied, "the replacement is installed by now");
                DROPPED.with(|d| d.set(true));
            }
        }

        let guard = Reenter;
        install_doc_scoped_slot(
            &SLOT,
            Rc::new(move || {
                let _ = &guard;
                1u32
            }) as Probe,
        );
        install_doc_scoped_slot(&SLOT, Rc::new(|| 2u32) as Probe);
        assert!(DROPPED.with(|d| d.get()), "the displaced value was dropped");
        clear_doc_scoped_slot(&SLOT);
    }

    /// The read half must not hold the slot's borrow across the call, so a
    /// callback may install its own replacement (or clear the slot) from inside
    /// its own dispatch.
    #[test]
    fn read_scoped_slot_does_not_hold_the_borrow_across_the_call() {
        thread_local! {
            static SLOT: RefCell<Option<Probe>> = const { RefCell::new(None) };
        }

        install_scoped_slot(
            &SLOT,
            Rc::new(|| {
                // Re-entrant write while the value is being called.
                install_scoped_slot(&SLOT, Rc::new(|| 2u32) as Probe);
                1u32
            }) as Probe,
        );

        assert_eq!(read_scoped_slot(&SLOT).map(|f| f()), Some(1));
        assert_eq!(
            read_scoped_slot(&SLOT).map(|f| f()),
            Some(2),
            "the replacement installed from inside the call is now live"
        );
        clear_scoped_slot(&SLOT);
        assert!(read_scoped_slot(&SLOT).is_none());
    }

    // ── newest registration wins; a clear clears (review of PR #960) ─────────

    /// Since #295 a `RinchApp` mount is marked, so a component body's
    /// registration lands under its document — and a later one from a timer,
    /// `run_on_main_thread`, an http completion or a `main()` effect, which are
    /// all unmarked, lands on the fallback. The newer of the two must win, or a
    /// single-document app can never replace its mount-time interceptor.
    #[test]
    fn the_newer_of_the_documents_entry_and_the_fallback_wins() {
        use crate::context::push_dispatching_doc;

        thread_local! {
            static SLOT: RefCell<DocScopedSlotMap<dyn Fn() -> u32>> =
                const { RefCell::new(DocScopedSlotMap::new()) };
        }
        let read_as = |doc: u64| {
            let _d = push_dispatching_doc(doc);
            read_doc_scoped_slot(&SLOT).map(|f| f())
        };

        {
            let _a = push_dispatching_doc(1);
            install_doc_scoped_slot(&SLOT, Rc::new(|| 10u32) as Probe); // mount
        }
        install_doc_scoped_slot(&SLOT, Rc::new(|| 20u32) as Probe); // a timer
        assert_eq!(
            read_as(1),
            Some(20),
            "the later, unmarked registration wins"
        );
        {
            let _a = push_dispatching_doc(1);
            install_doc_scoped_slot(&SLOT, Rc::new(|| 30u32) as Probe); // a handler
        }
        assert_eq!(read_as(1), Some(30), "…and a later marked one wins back");
        assert_eq!(
            read_as(2),
            Some(20),
            "another document still sees the fallback"
        );

        // An unmarked clear speaks for every document, as its install serves
        // every document.
        clear_doc_scoped_slot(&SLOT);
        assert_eq!(read_as(1), None, "the clear cleared document 1's entry too");
        assert_eq!(read_as(2), None);
        assert_eq!(read_doc_scoped_slot(&SLOT).map(|f| f()), None);
    }

    /// A clear made **inside** a document leaves that document with nothing,
    /// whichever entry was in effect for it — "clear clears", as the single
    /// slot this replaced did. It removes the entry in effect (the #478 rule:
    /// when that is the fallback, it goes for everyone); when that is the
    /// document's own entry, an older fallback stays for the other documents
    /// and is shadowed for this one.
    #[test]
    fn a_documents_clear_leaves_it_nothing_and_others_untouched() {
        use crate::context::push_dispatching_doc;

        thread_local! {
            static SLOT: RefCell<DocScopedSlotMap<dyn Fn() -> u32>> =
                const { RefCell::new(DocScopedSlotMap::new()) };
        }
        let read_as = |doc: u64| {
            let _d = push_dispatching_doc(doc);
            read_doc_scoped_slot(&SLOT).map(|f| f())
        };
        let clear_as = |doc: u64| {
            let _d = push_dispatching_doc(doc);
            clear_doc_scoped_slot(&SLOT);
        };

        // Fallback older than the document's entry: the clear must not
        // resurrect the fallback for document 1.
        install_doc_scoped_slot(&SLOT, Rc::new(|| 1u32) as Probe); // main()
        {
            let _a = push_dispatching_doc(1);
            install_doc_scoped_slot(&SLOT, Rc::new(|| 2u32) as Probe);
        }
        clear_as(1);
        assert_eq!(
            read_as(1),
            None,
            "document 1 cleared: nothing, not main()'s"
        );
        assert_eq!(read_as(2), Some(1), "document 2 keeps the fallback");

        // A registration after the clear, from anywhere, is the newest again.
        install_doc_scoped_slot(&SLOT, Rc::new(|| 3u32) as Probe);
        assert_eq!(
            read_as(1),
            Some(3),
            "a later unmarked registration reaches doc 1"
        );

        // Fallback newer than the document's entry: the clear removes both
        // for document 1 (the mount entry must not come back).
        {
            let _a = push_dispatching_doc(1);
            install_doc_scoped_slot(&SLOT, Rc::new(|| 4u32) as Probe);
        }
        install_doc_scoped_slot(&SLOT, Rc::new(|| 5u32) as Probe);
        clear_as(1);
        assert_eq!(
            read_as(1),
            None,
            "neither the fallback nor the older mount entry"
        );
        assert_eq!(
            read_as(2),
            None,
            "the fallback was the entry in effect, and a clear removes the entry in effect (#478)"
        );

        // Two documents' own entries stay isolated through all of it.
        {
            let _b = push_dispatching_doc(2);
            install_doc_scoped_slot(&SLOT, Rc::new(|| 6u32) as Probe);
        }
        clear_as(1);
        assert_eq!(read_as(2), Some(6));
        clear_doc_scoped_slot(&SLOT);
        assert_eq!(read_as(2), None);
    }

    /// The same rules, through the public API of each of the four slots that
    /// share this discipline: a mount-time registration (document 1), then one
    /// made outside any document, then an unmarked clear.
    #[test]
    fn each_doc_keyed_slot_honours_last_registration_wins_and_clear_clears() {
        use crate::context::push_dispatching_doc;
        use crate::events::{
            KeyEventData, PasteEventData, SelectionAction, clear_keyboard_interceptor,
            clear_paste_interceptor, clear_selection_callback, clear_selection_sync_callback,
            dispatch_keyboard_event, dispatch_paste_event, dispatch_selection, fire_selection_sync,
            set_keyboard_interceptor, set_paste_interceptor, set_selection_callback,
            set_selection_sync_callback,
        };

        let hits: Rc<RefCell<Vec<String>>> = Rc::default();
        let tag = |slot: &'static str, who: &'static str| {
            let h = hits.clone();
            move || h.borrow_mut().push(format!("{slot}:{who}"))
        };

        for who in ["mount", "outside"] {
            let _mount = (who == "mount").then(|| push_dispatching_doc(1));
            let k = tag("key", who);
            set_keyboard_interceptor(move |_| {
                k();
                false
            });
            let p = tag("paste", who);
            set_paste_interceptor(move |_| {
                p();
                false
            });
            let s = tag("sel", who);
            set_selection_callback(move |_| {
                s();
                Vec::new()
            });
            let y = tag("sync", who);
            set_selection_sync_callback(move |_| y());
        }
        let fire_all = || {
            let _a = push_dispatching_doc(1);
            dispatch_keyboard_event(&KeyEventData::new("q", "KeyQ"));
            dispatch_paste_event(&PasteEventData {
                text: Some("x".into()),
                html: None,
            });
            dispatch_selection(SelectionAction::QueryRanges);
            fire_selection_sync(); // also dispatches a selection query first
        };
        fire_all();
        assert_eq!(
            *hits.borrow(),
            [
                "key:outside",
                "paste:outside",
                "sel:outside",
                "sel:outside",
                "sync:outside"
            ],
            "the newest registration of each slot is the one reached"
        );
        hits.borrow_mut().clear();

        clear_keyboard_interceptor();
        clear_paste_interceptor();
        clear_selection_callback();
        clear_selection_sync_callback();
        fire_all();
        assert!(
            hits.borrow().is_empty(),
            "each clear cleared: {:?}",
            hits.borrow()
        );
    }
}
