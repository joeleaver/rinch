//! Signal: a reactive container that holds a value and notifies subscribers when it changes.

use std::fmt;
use std::marker::PhantomData;
use std::panic::Location;

use super::{ObserverId, RUNTIME, SIGNAL_STORE};

mod diagnostics;
mod send;

#[cfg(test)]
use diagnostics::warn_count_for_tests;
use diagnostics::{panic_off_main, panic_read_freed, warn_write_dropped};

/// A reactive container that holds a value and notifies subscribers when it changes.
///
/// `Signal<T>` implements `Copy` — no `.clone()` needed before closures.
/// Values are stored in a thread-local slot vec and accessed via index + generation.
///
/// # Example
///
/// ```ignore
/// let count = Signal::new(0);
///
/// // No clone needed — Signal is Copy!
/// let increment = move || count.update(|n| *n += 1);
///
/// // Read the value
/// let value = count.get();
///
/// // Update the value (triggers subscribers)
/// count.set(5);
///
/// // Update based on current value
/// count.update(|n| *n += 1);
/// ```
pub struct Signal<T: 'static> {
    id: u32,
    generation: u32,
    _phantom: PhantomData<T>,
}

impl<T: 'static> Signal<T> {
    /// Get the internal signal ID (for debugging).
    pub fn debug_id(&self) -> u32 {
        self.id
    }

    /// The signal's identity: slot id and generation. Unlike
    /// [`debug_id`](Self::debug_id), two signals never share it, even after
    /// one's slot has been freed and reused by the other.
    #[doc(hidden)]
    pub fn identity(&self) -> (u32, u32) {
        (self.id, self.generation)
    }
}

// Manual Copy/Clone because PhantomData<T> would require T: Copy for derive
impl<T: 'static> Copy for Signal<T> {}

impl<T: 'static> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Signal<T> {
    /// Create a new signal with the given initial value.
    pub fn new(value: T) -> Self {
        let (id, generation) = SIGNAL_STORE.with(|store| store.borrow_mut().alloc(value));
        // Attribute this signal to the ambient owner, if any. No ambient owner
        // means app lifetime (issue #141).
        super::scope::record_signal(super::scope::SignalKey { id, generation });
        Self {
            id,
            generation,
            _phantom: PhantomData,
        }
    }

    /// Detach this signal from its owning scope, giving it app lifetime.
    ///
    /// Signals created during a render are attributed to the scope being built,
    /// and will be freed when that scope is disposed. `leak` opts out — for a
    /// signal that is deliberately handed to something longer-lived than the
    /// component that created it.
    ///
    /// Call it in the same render that created the signal: it searches the
    /// owner stack as it stands *now*, so from a later callback (a timer, a
    /// resumed continuation) the stack is empty and this is a no-op.
    ///
    /// Returns the signal, so it composes: `let s = Signal::new(0).leak();`
    #[track_caller]
    pub fn leak(self) -> Self {
        if !super::scope::forget_signal(super::scope::SignalKey {
            id: self.id,
            generation: self.generation,
        }) {
            tracing::debug!(
                "Signal::leak() at {}: no ambient owner held this signal; it already had \
                 app lifetime",
                std::panic::Location::caller()
            );
        }
        self
    }

    /// Subscribe the current observer (if any) to this signal.
    ///
    /// A subscription that is actually *new* is also recorded on the observer,
    /// so it can be released when that observer re-runs or is disposed (issue
    /// #171). A second read in the same run finds the id already in the set and
    /// records nothing, which keeps the registry lookup to once per dependency
    /// per run rather than once per read.
    fn track(&self) {
        let Some(observer) = super::tracking::current_observer() else {
            return;
        };
        // Clone the signal's own cell out of the store under a brief shared
        // borrow (issue #546), then subscribe on *that* cell's own
        // `RefCell<subscribers>` — never on `SIGNAL_STORE` mutably. That is
        // what lets this run while another signal's `with`/`update` closure
        // is on the stack holding only *its own* cell: different signal,
        // different `RefCell`, no conflict.
        let Some(inner) =
            SIGNAL_STORE.with(|store| store.borrow().get_inner(self.id, self.generation))
        else {
            return;
        };
        let subscribed = inner.subscribers.borrow_mut().insert(observer);
        if subscribed {
            super::effect::record_dep(
                observer,
                super::DepKey::Signal {
                    id: self.id,
                    generation: self.generation,
                },
            );
        }
    }

    /// Notify all subscribers that the value has changed.
    ///
    /// Subscribers are queued in registration order (the `BTreeSet` iterates
    /// ascending `ObserverId`) and the queue drains FIFO, so effects sharing a
    /// signal run in the order they were created — see the "Execution order"
    /// section of the [`reactive`](crate::reactive) module docs.
    fn notify(&self) {
        super::count_signal_notify();
        let subscribers: Vec<ObserverId> = SIGNAL_STORE
            .with(|store| store.borrow().get_inner(self.id, self.generation))
            .map(|inner| inner.subscribers.borrow().iter().copied().collect())
            .unwrap_or_default();

        tracing::debug!(
            "Signal({}).notify(): {} subscribers",
            self.id,
            subscribers.len()
        );

        // Mark every memo downstream of this signal stale *now*, before any
        // flush and whether or not a batch is open, so a read straight after
        // the write — in the same handler, inside the same batch — recomputes
        // rather than answering from the cache (see `memo`'s module docs).
        super::mark_memos_stale(&subscribers);

        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();

            // Mark that signals have changed (for request_render optimization)
            rt.signals_changed = true;

            for observer in subscribers {
                // A direct reader of a signal that changed runs whatever its
                // memos say: it is *definite*, not a maybe (`flush_effects`).
                rt.definite_effects.insert(observer);
                if rt.pending_effects_set.insert(observer) {
                    rt.pending_effects.push_back(observer);
                }
            }

            tracing::debug!(
                "Signal({}).notify(): batching={}, pending_effects={}",
                self.id,
                rt.batching,
                rt.pending_effects.len()
            );

            // If not batching, flush immediately: effects, then the UI
            // re-render callbacks — the ordering contract lives in
            // `flush_effects_and_notify`.
            if !rt.batching {
                drop(rt);

                super::flush_effects_and_notify();
            }
        });
    }
}

impl<T: Clone + 'static> Signal<T> {
    /// Get the current value of the signal.
    ///
    /// If called inside an effect, this automatically subscribes the effect
    /// to this signal.
    ///
    /// # Panics
    ///
    /// Panics if the signal has been freed (use-after-free). Use
    /// [`try_get`](Signal::try_get) when the signal may legitimately be gone.
    pub fn get(&self) -> T {
        match self.try_get() {
            Some(value) => value,
            None => panic_read_freed("get"),
        }
    }

    /// Get the current value, or `None` if the signal has been freed.
    ///
    /// The non-panicking counterpart to [`get`](Signal::get). Still subscribes
    /// the current observer (if any) when the signal is live, so a `try_get`
    /// inside an effect is reactive exactly like a `get`.
    ///
    /// ```ignore
    /// // A worker that stops pushing once its UI is gone.
    /// if let Some(current) = progress.try_get() {
    ///     progress.set(current + 1);
    /// }
    /// ```
    pub fn try_get(&self) -> Option<T> {
        self.track();
        // The store borrow (taken only to clone `inner` out) is gone by the
        // time `inner.value` is borrowed — see `SignalCell`'s docs (#546).
        let inner =
            SIGNAL_STORE.with(|store| store.borrow().get_inner(self.id, self.generation))?;
        Some(
            inner
                .value
                .borrow()
                .downcast_ref::<T>()
                .expect("Signal type mismatch (internal error)")
                .clone(),
        )
    }
}

impl<T: 'static> Signal<T> {
    /// Whether this signal's backing slot is still live.
    ///
    /// A signal is freed when the scope that owns it is disposed. Reads
    /// ([`get`](Signal::get), [`with`](Signal::with)) panic afterwards and
    /// writes ([`set`](Signal::set), [`set_if_changed`](Signal::set_if_changed),
    /// [`update`](Signal::update)) become warn-once no-ops, so this is the check
    /// to make from a long-lived callback or a background worker holding a
    /// `Copy` handle.
    ///
    /// Does **not** subscribe the current observer — liveness is not reactive,
    /// and tracking it would resurrect the dependency you are trying to drop.
    pub fn is_alive(&self) -> bool {
        SIGNAL_STORE.with(|store| store.borrow().get_inner(self.id, self.generation).is_some())
    }

    /// Get a reference to the current value without cloning.
    ///
    /// If called inside an effect, this automatically subscribes the effect
    /// to this signal.
    ///
    /// `f` runs under **this signal's own** borrow, not a lock on every
    /// signal in the store (issue #546): a nested read of a *different*
    /// signal inside `f` — `a.with(|av| b.get())`, or the same written the
    /// other way round — is legal, in an effect or out of one. Only a nested
    /// access to *this same* signal from inside `f` still panics, which is
    /// genuine reentrancy on one value, not an artifact of a shared store
    /// lock.
    ///
    /// # Panics
    ///
    /// Panics if the signal has been freed. Use [`try_with`](Signal::try_with)
    /// when the signal may legitimately be gone.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        match self.try_with(f) {
            Some(result) => result,
            None => panic_read_freed("with"),
        }
    }

    /// Borrow the current value, or return `None` if the signal has been freed.
    ///
    /// The non-panicking counterpart to [`with`](Signal::with). `f` is not
    /// called when the signal is gone. See `with`'s docs for what running `f`
    /// under this signal's own borrow (rather than the whole store's) means
    /// for nested reads of other signals.
    pub fn try_with<R>(&self, f: impl FnOnce(&T) -> R) -> Option<R> {
        self.track();
        // Clone the cell out under a brief shared store borrow, then drop
        // that borrow before touching `inner.value` at all. `f` therefore
        // never runs with any borrow of `SIGNAL_STORE` itself held — only
        // `inner.value`'s own `RefCell`, which is private to this signal.
        let inner =
            SIGNAL_STORE.with(|store| store.borrow().get_inner(self.id, self.generation))?;
        let value = inner.value.borrow();
        Some(f(value
            .downcast_ref::<T>()
            .expect("Signal type mismatch (internal error)")))
    }

    /// Set the signal to a new value.
    ///
    /// This will notify all subscribers to re-run.
    ///
    /// Writing to a **freed** signal is a no-op: it logs a `warn` once per call
    /// site and returns. See the [module docs](crate::reactive) — you may always
    /// write to a handle, but you may only read a live one.
    ///
    /// # Panics
    ///
    /// Panics if called from a background thread. Use [`send()`](Signal::send)
    /// for automatic cross-thread dispatch.
    #[track_caller]
    pub fn set(&self, value: T) {
        self.set_at(value, "set", Location::caller());
    }

    /// [`set`](Signal::set), with the reporting location supplied explicitly so
    /// [`send`](Signal::send) can attribute a deferred cross-thread write to the
    /// site that requested it.
    pub(crate) fn set_at(&self, value: T, called: &str, loc: &'static Location<'static>) {
        if !super::is_main_thread() {
            panic_off_main("set", "send");
        }
        // Clone the cell out under a brief store borrow (issue #546); the
        // store borrow is gone before the value is ever touched.
        let Some(inner) =
            SIGNAL_STORE.with(|store| store.borrow().get_inner(self.id, self.generation))
        else {
            // Nothing was borrowed above, so `value` is just an owned `T` here
            // — dropping it needs no special placement.
            drop(value);
            warn_write_dropped(self.id, called, loc);
            return;
        };
        // The displaced value is dropped *after* this signal's own
        // `RefCell<value>` borrow ends (the block below), not inside it: a
        // `T` whose `Drop` touches this same signal would otherwise
        // `BorrowMutError` — genuine reentrancy, same as any other `RefCell`.
        let displaced = {
            let mut slot_value = inner.value.borrow_mut();
            std::mem::replace(&mut *slot_value, Box::new(value))
        };
        drop(displaced);
        self.notify();
    }

    /// Set the signal's value only if it differs from the current value.
    ///
    /// This avoids unnecessary effect re-runs when pushing the same data.
    ///
    /// Writing to a **freed** signal is a no-op that warns once per call site.
    ///
    /// # Panics
    ///
    /// Panics if called from a background thread.
    #[track_caller]
    pub fn set_if_changed(&self, value: T)
    where
        T: PartialEq,
    {
        self.set_if_changed_at(value, "set_if_changed", Location::caller());
    }

    /// [`set_if_changed`](Signal::set_if_changed) with an explicit reporting
    /// location — see [`set_at`](Signal::set_at).
    pub(crate) fn set_if_changed_at(&self, value: T, called: &str, loc: &'static Location<'static>)
    where
        T: PartialEq,
    {
        if !super::is_main_thread() {
            panic_off_main("set_if_changed", "send");
        }

        let Some(inner) =
            SIGNAL_STORE.with(|store| store.borrow().get_inner(self.id, self.generation))
        else {
            drop(value);
            warn_write_dropped(self.id, called, loc);
            return;
        };

        /// Whether the comparison found a change, carrying whichever value
        /// (the displaced old one, or the unchanged incoming one) still needs
        /// dropping — deferred until this signal's own `value` borrow below
        /// has ended, same reason as `set_at`.
        enum Outcome<T> {
            Changed(Box<dyn std::any::Any>),
            Unchanged(T),
        }

        let outcome = {
            let mut slot_value = inner.value.borrow_mut();
            let old = slot_value
                .downcast_ref::<T>()
                .expect("Signal type mismatch (internal error)");
            if *old == value {
                Outcome::Unchanged(value)
            } else {
                Outcome::Changed(std::mem::replace(&mut *slot_value, Box::new(value)))
            }
        };

        match outcome {
            Outcome::Unchanged(value) => drop(value),
            Outcome::Changed(displaced) => {
                drop(displaced);
                self.notify();
            }
        }
    }

    /// Update the signal's value using a function.
    ///
    /// This will notify all subscribers to re-run.
    ///
    /// On a **freed** signal `f` is never called and the update is dropped, with
    /// a warn once per call site. Side effects in `f` are lost along with it —
    /// keep them out of the closure if they must happen regardless.
    ///
    /// # Panics
    ///
    /// Panics if called from a background thread. Use [`update_send()`](Signal::update_send)
    /// for automatic cross-thread dispatch.
    #[track_caller]
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        self.update_at(f, "update", Location::caller());
    }

    /// [`update`](Signal::update) with an explicit reporting location — see
    /// [`set_at`](Signal::set_at).
    ///
    /// `f` runs under only this signal's own `value` borrow (issue #546),
    /// exactly like [`try_with`](Signal::try_with) — no `SIGNAL_STORE` borrow
    /// is held while `f` runs, so a nested read or write of a *different*
    /// signal inside `f` is legal, in an effect or out of one.
    pub(crate) fn update_at(
        &self,
        f: impl FnOnce(&mut T),
        called: &str,
        loc: &'static Location<'static>,
    ) {
        if !super::is_main_thread() {
            panic_off_main("update", "update_send");
        }
        let Some(inner) =
            SIGNAL_STORE.with(|store| store.borrow().get_inner(self.id, self.generation))
        else {
            drop(f);
            warn_write_dropped(self.id, called, loc);
            return;
        };
        {
            let mut slot_value = inner.value.borrow_mut();
            let value = slot_value
                .downcast_mut::<T>()
                .expect("Signal type mismatch (internal error)");
            f(value);
        }
        self.notify();
    }
}

#[cfg(test)]
impl<T: 'static> Signal<T> {
    /// Free this signal's slot, standing in for the scope disposal that #141's
    /// dispose fixpoint (PR4) will perform. Test-only.
    pub(crate) fn free_for_tests(&self) {
        super::free_signal_for_tests(self.id, self.generation);
    }

    /// How many observers are subscribed to this signal, or 0 if it is freed.
    ///
    /// Test-only: the subscriber set is private, and "the set is empty again
    /// once the observers are gone" is the whole contract of issue #171.
    pub(crate) fn subscriber_count_for_tests(&self) -> usize {
        SIGNAL_STORE
            .with(|store| store.borrow().get_inner(self.id, self.generation))
            .map_or(0, |inner| inner.subscribers.borrow().len())
    }
}

impl<T: fmt::Debug + 'static> fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = SIGNAL_STORE.with(|store| store.borrow().get_inner(self.id, self.generation));
        match inner {
            Some(inner) => match inner.value.borrow().downcast_ref::<T>() {
                Some(value) => f.debug_struct("Signal").field("value", value).finish(),
                None => f
                    .debug_struct("Signal")
                    .field("error", &"type mismatch")
                    .finish(),
            },
            None => f.debug_struct("Signal").field("error", &"freed").finish(),
        }
    }
}

impl<T: fmt::Display + 'static> fmt::Display for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = SIGNAL_STORE.with(|store| store.borrow().get_inner(self.id, self.generation));
        match inner {
            Some(inner) => match inner.value.borrow().downcast_ref::<T>() {
                Some(value) => fmt::Display::fmt(value, f),
                None => write!(f, "<type mismatch>"),
            },
            None => write!(f, "<freed signal>"),
        }
    }
}

#[cfg(test)]
mod liveness_tests;

/// Issue #546: `Signal::try_with`/`update` used to hold a shared/mutable
/// borrow of the **whole** `SIGNAL_STORE` across the user closure. `track()`
/// needs `SIGNAL_STORE.borrow_mut()` only when an observer is on the stack —
/// so a nested read of a *different* signal worked outside an effect (two
/// shared borrows coexist) and `BorrowMutError`'d inside one. These fixtures
/// pin the fix: the store borrow is released before the per-signal cell is
/// ever touched, so nested access to a *different* signal is legal, in or out
/// of an effect, and only genuine same-signal reentrancy still panics.
#[cfg(test)]
mod nested_read_tests;

/// `SignalStore::get_inner`'s generation filter (issue #546, PR #1312) is the
/// one check that makes a stale `Signal<T>` handle read as dead after its
/// slot is recycled by a new signal — every read/write/track/notify call
/// site goes through it. `a_dep_whose_slot_was_recycled_does_not_disturb_the_new_occupant`
/// (`effect/unsubscribe_tests.rs`) looks like it would catch a dropped filter
/// but does not: it only exercises `DepKey::unsubscribe`, and `ObserverId`s
/// are never reused, so a stale key's `.remove()` is a no-op on the new
/// occupant's subscriber set whether or not `get_inner` filters by generation.
#[cfg(test)]
mod slot_reuse_tests;

/// `set_at`'s comment claims the displaced value is dropped strictly after
/// this signal's own `value` borrow ends, so that a `T` whose `Drop` reads or
/// writes *the same signal it was just displaced from* does not
/// `BorrowMutError` (issue #546, PR #1312). Every `TouchesASignalOnDrop`
/// fixture in `signal/liveness_tests.rs` touches a *different* signal, so that
/// specific claim was unpinned until this module.
#[cfg(test)]
mod self_referential_drop_tests;
