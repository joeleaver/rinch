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
    fn track(&self) {
        let observer = RUNTIME.with(|rt| rt.borrow().observer_stack.last().copied());
        if let Some(observer) = observer {
            SIGNAL_STORE.with(|store| {
                if let Some(slot) = store.borrow_mut().get_slot_mut(self.id, self.generation) {
                    slot.subscribers.insert(observer);
                }
            });
        }
    }

    /// Notify all subscribers that the value has changed.
    ///
    /// Subscribers are queued in registration order (the `BTreeSet` iterates
    /// ascending `ObserverId`) and the queue drains FIFO, so effects sharing a
    /// signal run in the order they were created — see the "Execution order"
    /// section of the [`reactive`](crate::reactive) module docs.
    fn notify(&self) {
        let subscribers: Vec<ObserverId> = SIGNAL_STORE.with(|store| {
            store
                .borrow()
                .get_slot(self.id, self.generation)
                .map(|slot| slot.subscribers.iter().copied().collect())
                .unwrap_or_default()
        });

        tracing::debug!(
            "Signal({}).notify(): {} subscribers",
            self.id,
            subscribers.len()
        );

        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();

            // Mark that signals have changed (for request_render optimization)
            rt.signals_changed = true;

            for observer in subscribers {
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
        SIGNAL_STORE.with(|store| {
            let store = store.borrow();
            let slot = store.get_slot(self.id, self.generation)?;
            Some(
                slot.value
                    .downcast_ref::<T>()
                    .expect("Signal type mismatch (internal error)")
                    .clone(),
            )
        })
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
        SIGNAL_STORE.with(|store| store.borrow().get_slot(self.id, self.generation).is_some())
    }

    /// Get a reference to the current value without cloning.
    ///
    /// If called inside an effect, this automatically subscribes the effect
    /// to this signal.
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
    /// called when the signal is gone.
    pub fn try_with<R>(&self, f: impl FnOnce(&T) -> R) -> Option<R> {
        self.track();
        // Hold `f` in an Option so that on the freed path it is dropped *here*,
        // after the store borrow is released, rather than inside it. `f`'s
        // captures are user values whose `Drop` may touch signals.
        let mut f = Some(f);
        SIGNAL_STORE.with(|store| {
            let store = store.borrow();
            let slot = store.get_slot(self.id, self.generation)?;
            Some(f.take().unwrap()(
                slot.value
                    .downcast_ref::<T>()
                    .expect("Signal type mismatch (internal error)"),
            ))
        })
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
        // Both the incoming value (on the freed path) and the outgoing one
        // (on the live path) must be dropped *outside* the store borrow: a `T`
        // whose `Drop` touches a signal would otherwise `BorrowMutError`.
        let mut value = Some(value);
        let displaced = SIGNAL_STORE.with(|store| {
            let mut store = store.borrow_mut();
            let slot = store.get_slot_mut(self.id, self.generation)?;
            Some(std::mem::replace(
                &mut slot.value,
                Box::new(value.take().unwrap()),
            ))
        });
        drop(displaced);
        if value.is_some() {
            drop(value);
            warn_write_to_freed("set", loc);
            return;
        }
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

        /// What `set_if_changed` found in the store.
        enum Outcome {
            Freed,
            Unchanged,
            /// Changed; carries the displaced value so it drops outside the borrow.
            Changed(Box<dyn std::any::Any>),
        }

        let mut value = Some(value);
        let outcome = SIGNAL_STORE.with(|store| {
            let mut store = store.borrow_mut();
            let Some(slot) = store.get_slot_mut(self.id, self.generation) else {
                return Outcome::Freed;
            };
            let old = slot
                .value
                .downcast_ref::<T>()
                .expect("Signal type mismatch (internal error)");
            if old == value.as_ref().unwrap() {
                return Outcome::Unchanged;
            }
            Outcome::Changed(std::mem::replace(
                &mut slot.value,
                Box::new(value.take().unwrap()),
            ))
        });

        match outcome {
            Outcome::Freed => {
                drop(value);
                warn_write_to_freed("set_if_changed", loc);
            }
            Outcome::Unchanged => drop(value),
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
    pub(crate) fn update_at(&self, f: impl FnOnce(&mut T), loc: &'static Location<'static>) {
        if !super::is_main_thread() {
            panic_off_main("update", "update_send");
        }
        // As in `try_with`: on the freed path `f` must drop after the borrow.
        let mut f = Some(f);
        let applied = SIGNAL_STORE.with(|store| {
            let mut store = store.borrow_mut();
            let Some(slot) = store.get_slot_mut(self.id, self.generation) else {
                return false;
            };
            let value = slot
                .value
                .downcast_mut::<T>()
                .expect("Signal type mismatch (internal error)");
            f.take().unwrap()(value);
            true
        });
        if !applied {
            drop(f);
            warn_write_to_freed("update", loc);
            return;
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
}

impl<T: fmt::Debug + 'static> fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        SIGNAL_STORE.with(|store| {
            let store = store.borrow();
            if let Some(slot) = store.get_slot(self.id, self.generation) {
                if let Some(value) = slot.value.downcast_ref::<T>() {
                    f.debug_struct("Signal").field("value", value).finish()
                } else {
                    f.debug_struct("Signal")
                        .field("error", &"type mismatch")
                        .finish()
                }
            } else {
                f.debug_struct("Signal").field("error", &"freed").finish()
            }
        })
    }
}

impl<T: fmt::Display + 'static> fmt::Display for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        SIGNAL_STORE.with(|store| {
            let store = store.borrow();
            if let Some(slot) = store.get_slot(self.id, self.generation) {
                if let Some(value) = slot.value.downcast_ref::<T>() {
                    fmt::Display::fmt(value, f)
                } else {
                    write!(f, "<type mismatch>")
                }
            } else {
                write!(f, "<freed signal>")
            }
        })
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
