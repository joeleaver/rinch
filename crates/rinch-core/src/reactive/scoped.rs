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
//! [`crate::events::set_paste_interceptor`] and extracted here so keyboard,
//! selection and (later) the websocket and menu registries share it verbatim
//! rather than each paraphrasing it.
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

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::{Rc, Weak};
use std::thread::LocalKey;

use super::on_cleanup;

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

/// Install `value` under `key` in a keyed registry, tying its removal to the
/// scope that is currently rendering.
///
/// The keyed twin of [`install_scoped_slot`], with the same three rules: the
/// cleanup removes `key` only if the entry is *still* this value, so a
/// re-registration at the same key (a menu rebuilt, a connection id reused)
/// survives an earlier scope's disposal. This is what lets such a map shrink as
/// well as grow.
pub fn install_scoped_entry<K, T>(
    map: &'static LocalKey<RefCell<HashMap<K, Rc<T>>>>,
    key: K,
    value: Rc<T>,
) -> bool
where
    K: Eq + Hash + Clone + 'static,
    T: ?Sized + 'static,
{
    let mine: Weak<T> = Rc::downgrade(&value);
    let doomed = key.clone();
    // Rule 3, as in `install_scoped_slot`.
    let _previous = map.with(|m| m.borrow_mut().insert(key, value));
    on_cleanup(move || {
        let Some(ours) = mine.upgrade() else {
            return;
        };
        // `try_with`/`try_borrow_mut`, as in `install_scoped_slot`.
        let _displaced = map.try_with(|m| {
            let Ok(mut entries) = m.try_borrow_mut() else {
                return None;
            };
            if entries
                .get(&doomed)
                .is_some_and(|installed| Rc::ptr_eq(installed, &ours))
            {
                entries.remove(&doomed)
            } else {
                None
            }
        });
    })
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

    /// The keyed form removes its entry, so the map shrinks rather than only
    /// ever growing.
    #[test]
    fn a_scoped_map_entry_is_removed_on_disposal_and_the_map_shrinks() {
        thread_local! {
            static MAP: RefCell<HashMap<String, Probe>> = RefCell::new(HashMap::new());
        }
        fn len() -> usize {
            MAP.with(|m| m.borrow().len())
        }

        let scope = Scope::new();
        scope.run(|| {
            install_scoped_entry(&MAP, "a".to_string(), Rc::new(|| 1u32) as Probe);
            install_scoped_entry(&MAP, "b".to_string(), Rc::new(|| 2u32) as Probe);
        });
        assert_eq!(len(), 2);

        scope.dispose();
        assert_eq!(len(), 0, "disposal removes the entries the scope installed");
    }

    #[test]
    fn a_scoped_entry_re_registered_at_the_same_key_survives_the_earlier_scopes_disposal() {
        thread_local! {
            static MAP: RefCell<HashMap<u8, Probe>> = RefCell::new(HashMap::new());
        }
        fn read(key: u8) -> Option<u32> {
            MAP.with(|m| m.borrow().get(&key).cloned()).map(|f| f())
        }

        let first = Scope::new();
        first.run(|| install_scoped_entry(&MAP, 7u8, Rc::new(|| 1u32) as Probe));
        let second = Scope::new();
        second.run(|| install_scoped_entry(&MAP, 7u8, Rc::new(|| 2u32) as Probe));

        first.dispose();
        assert_eq!(
            read(7),
            Some(2),
            "the later registration at the same key must survive"
        );

        second.dispose();
        assert_eq!(read(7), None);
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
}
