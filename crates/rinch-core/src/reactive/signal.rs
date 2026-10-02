//! Signal: a reactive container that holds a value and notifies subscribers when it changes.

use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;
use std::marker::PhantomData;
use std::panic::Location;

use super::{ObserverId, RUNTIME, SIGNAL_STORE};

/// Build the off-main-thread panic message used by `set`, `set_if_changed`, and `update`.
///
/// Names the offending method and the exact cross-thread alternative so the panic
/// message itself documents the fix.
#[cold]
#[inline(never)]
fn panic_off_main(called: &str, alt: &str) -> ! {
    panic!(
        "Signal::{called}() must run on the main thread. \
         Replace `signal.{called}(...)` with `signal.{alt}(...)` to dispatch \
         across threads, or wrap the call in `rinch::run_on_main_thread(...)` \
         to schedule it manually."
    );
}

/// Build the read-after-free panic message used by `get` and `with`.
///
/// Reads are strict because there is nothing to return: `T` is not `Default`, so
/// a lenient read has no value to hand back. Writes take the other branch — see
/// [`warn_write_to_freed`].
#[cold]
#[inline(never)]
fn panic_read_freed(called: &str) -> ! {
    panic!(
        "Signal::{called}() on a freed signal. The scope that owned this signal \
         was disposed (its component was removed from the tree) while this handle \
         was still reachable. Guard the read with `signal.is_alive()`, or use \
         `signal.try_{called}(...)` to get an `Option` instead of panicking."
    );
}

/// Warn — at most once per call site — that a write to a freed signal was dropped.
///
/// Writes are lenient because the calling thread frequently cannot know the
/// signal is gone: a detached worker holding a `Copy` handle keeps calling
/// `send`/`update_send` long after the UI that owned the signal was torn down,
/// and panicking there would take down the app for a write nobody was waiting on.
///
/// The caller's location is passed in explicitly rather than read from
/// `Location::caller()` here. `send`/`update_send` dispatch a closure that
/// performs the write *later, on another stack*, where `#[track_caller]`
/// information no longer exists — so they capture their caller's location up
/// front and hand it down. Reading it here would key every cross-thread write
/// to one line inside this file, collapsing the per-call-site dedup into a
/// single global warning.
#[cold]
#[inline(never)]
fn warn_write_to_freed(called: &str, loc: &'static Location<'static>) {
    thread_local! {
        /// `(file, line, column)` of call sites already warned about. `file()`
        /// is `&'static str`, so the key borrows nothing.
        static WARNED: RefCell<HashSet<(&'static str, u32, u32)>> =
            RefCell::new(HashSet::new());
    }

    let first = WARNED.with(|w| {
        w.borrow_mut()
            .insert((loc.file(), loc.line(), loc.column()))
    });
    if !first {
        return;
    }

    #[cfg(test)]
    WARN_COUNT.with(|c| c.set(c.get() + 1));

    tracing::warn!(
        "Signal::{called}() on a freed signal at {}:{}:{} — the write was dropped. \
         The scope that owned this signal was disposed while this handle was still \
         reachable. Check `signal.is_alive()` before writing if the write matters. \
         (Warned once per call site.)",
        loc.file(),
        loc.line(),
        loc.column(),
    );
}

#[cfg(test)]
thread_local! {
    /// Number of warnings [`warn_write_to_freed`] actually emitted, i.e. the
    /// number of *distinct call sites* it saw. Lets the dedup be asserted
    /// without installing a `tracing` subscriber.
    static WARN_COUNT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Warnings emitted so far on this thread. Test-only.
#[cfg(test)]
pub(crate) fn warn_count_for_tests() -> u32 {
    WARN_COUNT.with(|c| c.get())
}

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
        let Some(observer) = RUNTIME.with(|rt| rt.borrow().observer_stack.last().copied()) else {
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
        self.set_at(value, Location::caller());
    }

    /// [`set`](Signal::set), with the reporting location supplied explicitly so
    /// [`send`](Signal::send) can attribute a deferred cross-thread write to the
    /// site that requested it.
    pub(crate) fn set_at(&self, value: T, loc: &'static Location<'static>) {
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
            warn_write_to_freed("set", loc);
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
        self.set_if_changed_at(value, Location::caller());
    }

    /// [`set_if_changed`](Signal::set_if_changed) with an explicit reporting
    /// location — see [`set_at`](Signal::set_at).
    pub(crate) fn set_if_changed_at(&self, value: T, loc: &'static Location<'static>)
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
            warn_write_to_freed("set_if_changed", loc);
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
        self.update_at(f, Location::caller());
    }

    /// [`update`](Signal::update) with an explicit reporting location — see
    /// [`set_at`](Signal::set_at).
    ///
    /// `f` runs under only this signal's own `value` borrow (issue #546),
    /// exactly like [`try_with`](Signal::try_with) — no `SIGNAL_STORE` borrow
    /// is held while `f` runs, so a nested read or write of a *different*
    /// signal inside `f` is legal, in an effect or out of one.
    pub(crate) fn update_at(&self, f: impl FnOnce(&mut T), loc: &'static Location<'static>) {
        if !super::is_main_thread() {
            panic_off_main("update", "update_send");
        }
        let Some(inner) =
            SIGNAL_STORE.with(|store| store.borrow().get_inner(self.id, self.generation))
        else {
            drop(f);
            warn_write_to_freed("update", loc);
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

impl<T: Send + 'static> Signal<T> {
    /// Set the signal from any thread.
    ///
    /// If called from the main thread, behaves identically to [`set()`](Signal::set).
    /// If called from a background thread, automatically dispatches to the main
    /// thread where the signal store lives.
    ///
    /// Requires `T: Send` since the value may cross thread boundaries.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let level = Signal::new(0.0f32);
    ///
    /// std::thread::spawn(move || {
    ///     // This just works — no run_on_main_thread() wrapper needed
    ///     level.send(0.5);
    /// });
    /// ```
    #[track_caller]
    pub fn send(&self, value: T) {
        // Captured here, not inside the closure: the closure runs later on the
        // main thread, where `#[track_caller]` no longer reaches this call site.
        // Without this every dropped cross-thread write would be attributed to
        // one line in this file and the warn-once dedup would silence them all.
        let loc = Location::caller();
        if super::is_main_thread() {
            self.set_at(value, loc);
        } else {
            let signal = *self;
            super::dispatch_to_main_thread(Box::new(move || {
                signal.set_at(value, loc);
            }));
        }
    }

    /// Update the signal from any thread.
    ///
    /// If called from the main thread, behaves identically to [`update()`](Signal::update).
    /// If called from a background thread, automatically dispatches to the main
    /// thread where the signal store lives.
    ///
    /// Requires the closure to be `Send + 'static` since it may cross thread boundaries.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let items = Signal::new(vec![1, 2, 3]);
    ///
    /// std::thread::spawn(move || {
    ///     items.update_send(|list| list.push(4));
    /// });
    /// ```
    #[track_caller]
    pub fn update_send(&self, f: impl FnOnce(&mut T) + Send + 'static) {
        // See `send` — the location must be captured before the thread hop.
        let loc = Location::caller();
        if super::is_main_thread() {
            self.update_at(f, loc);
        } else {
            let signal = *self;
            super::dispatch_to_main_thread(Box::new(move || {
                signal.update_at(f, loc);
            }));
        }
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
mod liveness_tests {
    use super::*;
    use crate::reactive::Effect;
    use std::cell::Cell;
    use std::rc::Rc;

    // ---- liveness queries ---------------------------------------------------

    #[test]
    fn is_alive_flips_when_the_slot_is_freed() {
        let s = Signal::new(1);
        assert!(s.is_alive());
        s.free_for_tests();
        assert!(!s.is_alive());
    }

    #[test]
    fn try_get_and_try_with_return_none_after_free() {
        let s = Signal::new(7);
        assert_eq!(s.try_get(), Some(7));
        assert_eq!(s.try_with(|v| *v + 1), Some(8));

        s.free_for_tests();
        assert_eq!(s.try_get(), None);
        assert_eq!(s.try_with(|v| *v + 1), None);
    }

    #[test]
    fn try_with_does_not_call_its_closure_on_a_freed_signal() {
        let calls = Rc::new(Cell::new(0));
        let s = Signal::new(0);
        s.free_for_tests();

        let c = Rc::clone(&calls);
        let out: Option<()> = s.try_with(move |_| c.set(c.get() + 1));

        assert!(out.is_none());
        assert_eq!(calls.get(), 0, "closure must not run on a freed signal");
    }

    #[test]
    fn try_get_subscribes_the_current_observer_just_like_get() {
        let s = Signal::new(0);
        let runs = Rc::new(Cell::new(0));

        let r = Rc::clone(&runs);
        let _e = Effect::new(move || {
            let _ = s.try_get();
            r.set(r.get() + 1);
        });
        assert_eq!(runs.get(), 1);

        s.set(1);
        assert_eq!(runs.get(), 2, "try_get must track like get");
    }

    #[test]
    #[should_panic(expected = "Signal::get() on a freed signal")]
    fn get_panics_after_free() {
        let s = Signal::new(1);
        s.free_for_tests();
        let _ = s.get();
    }

    #[test]
    #[should_panic(expected = "Signal::with() on a freed signal")]
    fn with_panics_after_free() {
        let s = Signal::new(1);
        s.free_for_tests();
        s.with(|v| *v);
    }

    // ---- lenient writes -----------------------------------------------------

    #[test]
    fn every_write_to_a_freed_signal_is_a_no_op_rather_than_a_panic() {
        let s = Signal::new(1i32);
        s.free_for_tests();

        // None of these may panic — a detached worker holding a Copy handle
        // cannot know the signal is gone (issue #141, SD1).
        s.set(2);
        s.set_if_changed(3);
        s.update(|v| *v += 1);

        assert!(!s.is_alive());
    }

    #[test]
    fn update_does_not_run_its_closure_on_a_freed_signal() {
        let ran = Rc::new(Cell::new(false));
        let s = Signal::new(0);
        s.free_for_tests();

        let r = Rc::clone(&ran);
        s.update(move |v| {
            *v += 1;
            r.set(true);
        });

        assert!(
            !ran.get(),
            "the closure — and any side effect in it — is dropped with the write"
        );
    }

    #[test]
    fn a_write_to_a_freed_signal_does_not_notify_its_own_observers() {
        // The observer must subscribe to `doomed` *before* it is freed —
        // otherwise the test proves nothing, since an effect that never read
        // the signal would not re-run either way.
        let doomed = Signal::new(0);
        let runs = Rc::new(Cell::new(0));

        let r = Rc::clone(&runs);
        let _e = Effect::new(move || {
            // try_get, not get: after the free this effect must be able to run
            // without panicking if anything else queues it.
            let _ = doomed.try_get();
            r.set(r.get() + 1);
        });
        assert_eq!(runs.get(), 1);

        // Positive control: while alive, a write DOES re-run the observer.
        doomed.set(1);
        assert_eq!(runs.get(), 2, "control — the effect really observes it");

        doomed.free_for_tests();
        doomed.set(99);

        assert_eq!(runs.get(), 2, "a dropped write must not flush effects");
    }

    #[test]
    fn a_deferred_cross_thread_write_reports_the_send_call_site() {
        // `send`/`update_send` perform the write inside a closure that runs
        // later, where #[track_caller] no longer reaches the caller. If the
        // location is not captured up front, every cross-thread freed write
        // dedups to one line inside signal.rs and all but the first go silent.
        let before = warn_count_for_tests();
        let a = Signal::new(0i32);
        let b = Signal::new(0i32);
        a.free_for_tests();
        b.free_for_tests();

        a.send(1);
        b.send(2);

        assert_eq!(
            warn_count_for_tests() - before,
            2,
            "two distinct send sites must produce two warnings"
        );

        let c = Signal::new(0i32);
        c.free_for_tests();
        c.update_send(|v| *v += 1);
        assert_eq!(
            warn_count_for_tests() - before,
            3,
            "update_send is attributed to its own call site too"
        );
    }

    #[test]
    fn a_freed_write_warns_once_per_call_site() {
        let before = warn_count_for_tests();
        let s = Signal::new(0);
        s.free_for_tests();

        // One call site, hammered — exactly one warning.
        for _ in 0..50 {
            s.set(1);
        }
        assert_eq!(
            warn_count_for_tests() - before,
            1,
            "a hot loop must not spam the log"
        );

        // A *second*, distinct call site is still reported.
        s.set(2);
        assert_eq!(
            warn_count_for_tests() - before,
            2,
            "dedup is per call site, not global"
        );
    }

    // ---- values are dropped outside the store borrow ------------------------

    /// A value whose `Drop` writes to another signal — the shape that made
    /// dropping a displaced value *inside* `SIGNAL_STORE.borrow_mut()` a
    /// `BorrowMutError`.
    struct TouchesASignalOnDrop(Signal<i32>);

    impl Drop for TouchesASignalOnDrop {
        fn drop(&mut self) {
            self.0.update(|n| *n += 1);
        }
    }

    #[test]
    fn replacing_a_value_whose_drop_touches_a_signal_does_not_reenter_the_store() {
        let drops = Signal::new(0);
        let holder = Signal::new(TouchesASignalOnDrop(drops));

        // `set` displaces the old value; it must be dropped after the store
        // borrow is released, or this is a BorrowMutError.
        holder.set(TouchesASignalOnDrop(drops));
        assert_eq!(drops.get(), 1);

        holder.set(TouchesASignalOnDrop(drops));
        assert_eq!(drops.get(), 2);

        // Free explicitly rather than leaving the value for the thread-local
        // store's destructor: its `Drop` touches `SIGNAL_STORE`, which is
        // unreachable once TLS teardown has begun.
        holder.free_for_tests();
    }

    #[test]
    fn discarding_a_write_to_a_freed_signal_drops_the_value_outside_the_store() {
        let drops = Signal::new(0);
        let holder = Signal::new(TouchesASignalOnDrop(drops));

        // Freeing drops the stored value, itself outside the borrow.
        holder.free_for_tests();
        let after_free = drops.get();
        assert_eq!(after_free, 1);

        // The incoming value now has nowhere to go; dropping it must also
        // happen outside the borrow.
        holder.set(TouchesASignalOnDrop(drops));
        assert_eq!(drops.get(), after_free + 1);
    }
}

/// Issue #546: `Signal::try_with`/`update` used to hold a shared/mutable
/// borrow of the **whole** `SIGNAL_STORE` across the user closure. `track()`
/// needs `SIGNAL_STORE.borrow_mut()` only when an observer is on the stack —
/// so a nested read of a *different* signal worked outside an effect (two
/// shared borrows coexist) and `BorrowMutError`'d inside one. These fixtures
/// pin the fix: the store borrow is released before the per-signal cell is
/// ever touched, so nested access to a *different* signal is legal, in or out
/// of an effect, and only genuine same-signal reentrancy still panics.
#[cfg(test)]
mod nested_read_tests {
    use super::*;
    use crate::reactive::Effect;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn nested_with_of_different_signals_works_outside_an_effect() {
        let a = Signal::new(1i32);
        let b = Signal::new(2i32);
        let sum = a.with(|av| b.with(|bv| av + bv));
        assert_eq!(sum, 3);
    }

    #[test]
    fn nested_with_of_different_signals_works_inside_an_effect() {
        // The exact repro from #546: outside an effect `track()` never takes
        // `borrow_mut()`, so this shape passed before the fix too. Only
        // inside an effect did `b`'s `track()` collide with `a`'s still-live
        // shared borrow. An observer must be on the stack for `track()` to
        // reach the mutable path at all, so this has to run inside `Effect`.
        let a = Signal::new(10i32);
        let b = Signal::new(20i32);
        let sum = Rc::new(Cell::new(0));
        let s = Rc::clone(&sum);

        let _e = Effect::new(move || {
            let total = a.with(|av| b.with(|bv| *av + *bv));
            s.set(total);
        });

        assert_eq!(
            sum.get(),
            30,
            "the nested read must not panic inside an effect"
        );

        // Changing either source re-runs the effect and both reads still work.
        a.set(11);
        assert_eq!(sum.get(), 31);
        b.set(21);
        assert_eq!(sum.get(), 32);
    }

    #[test]
    fn nested_get_inside_with_works_inside_an_effect() {
        let a = Signal::new(1i32);
        let b = Signal::new(2i32);
        let sum = Rc::new(Cell::new(0));
        let s = Rc::clone(&sum);

        let _e = Effect::new(move || {
            // `b.get()` tracks exactly like `b.with()`, from inside `a`'s
            // closure — the other spelling named in the issue.
            let total = a.with(|av| *av + b.get());
            s.set(total);
        });

        assert_eq!(sum.get(), 3);
    }

    #[test]
    fn nested_update_of_a_different_signal_works_inside_update() {
        // `update` held `borrow_mut()` across its own closure unconditionally
        // before the fix, so *any* nested signal access inside it panicked,
        // in or out of an effect. This checks it is fixed outright.
        let a = Signal::new(1i32);
        let b = Signal::new(10i32);

        a.update(|av| {
            b.update(|bv| *bv += *av);
        });

        assert_eq!(b.get(), 11);
    }

    #[test]
    fn nested_update_of_a_different_signal_works_inside_an_effect() {
        let a = Signal::new(1i32);
        let b = Signal::new(10i32);
        let runs = Rc::new(Cell::new(0));
        let r = Rc::clone(&runs);

        let _e = Effect::new(move || {
            let _ = a.get();
            a.update(|av| {
                b.update(|bv| *bv += *av);
            });
            r.set(r.get() + 1);
        });

        assert_eq!(runs.get(), 1);
        assert_eq!(b.get(), 11);
    }

    #[test]
    fn three_signals_nest_in_an_effect() {
        // Not just a pairwise coincidence: N independent signals must all be
        // reachable while the others' borrows are live.
        let a = Signal::new(1i32);
        let b = Signal::new(2i32);
        let c = Signal::new(3i32);
        let sum = Rc::new(Cell::new(0));
        let s = Rc::clone(&sum);

        let _e = Effect::new(move || {
            let total = a.with(|av| b.with(|bv| c.with(|cv| av + bv + cv)));
            s.set(total);
        });

        assert_eq!(sum.get(), 6);
    }

    #[test]
    fn a_memo_computation_can_nest_reads_of_different_signals() {
        // A memo's refresh runs with the memo's marker on the observer stack
        // too, so `track()` takes the mutable path there as well.
        use crate::reactive::Memo;

        let a = Signal::new(1i32);
        let b = Signal::new(2i32);
        let m = Memo::new(move || a.with(|av| b.with(|bv| av + bv)));

        assert_eq!(m.get(), 3);
        a.set(5);
        assert_eq!(m.get(), 7);
    }

    #[test]
    fn reading_a_signal_inside_its_own_with_does_not_panic() {
        // Two SHARED borrows of one `RefCell` coexist happily — `with` holds
        // a shared borrow of `a`'s own value cell, and a nested `a.get()`
        // takes another shared borrow of that same cell. This is not the
        // reentrancy case: nothing here ever asks for `borrow_mut()`.
        let a = Signal::new(1i32);
        let seen = a.with(|v| *v + a.get());
        assert_eq!(seen, 2);
    }

    #[test]
    #[should_panic(expected = "already borrowed")]
    fn a_signal_writing_itself_inside_its_own_with_still_panics() {
        // Genuine reentrancy on ONE signal's own cell, not an artifact of a
        // shared store lock — this must still fail. A fix that makes the
        // whole class of nested access "just work" without preserving this
        // is wrong: it would mean `with` no longer borrows the value it
        // hands out at all. `with` holds a *shared* borrow of `a`'s cell
        // across the closure; `a.set()` inside it needs `borrow_mut()` on
        // that very same cell, which must fail.
        let a = Signal::new(1i32);
        a.with(|_| {
            a.set(2);
        });
    }

    #[test]
    #[should_panic(expected = "already borrowed")]
    fn a_signal_writing_itself_inside_its_own_update_still_panics() {
        let a = Signal::new(1i32);
        a.update(|_| {
            a.set(2);
        });
    }
}
