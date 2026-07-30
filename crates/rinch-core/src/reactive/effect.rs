//! Effect: a side-effect that re-runs when its dependencies change.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::{ObserverId, RUNTIME};

// Storage for all effects (needed because effects reference themselves)
thread_local! {
    pub(super) static EFFECTS: RefCell<Vec<Option<Rc<EffectInner>>>> = const { RefCell::new(Vec::new()) };
}

/// A side-effect that re-runs when its dependencies change.
///
/// Effects automatically track which signals they read and re-run when
/// any of those signals change.
///
/// # Example
///
/// ```ignore
/// let count = Signal::new(0);
///
/// Effect::new(move || {
///     println!("Count is now: {}", count.get());
/// });
///
/// count.set(1); // Prints: "Count is now: 1"
/// count.set(2); // Prints: "Count is now: 2"
/// ```
pub struct Effect {
    id: ObserverId,
}

pub(super) struct EffectInner {
    #[allow(dead_code)] // Used for debugging/tracking purposes
    pub(super) id: ObserverId,
    pub(super) f: RefCell<Box<dyn FnMut()>>,
    pub(super) disposed: Cell<bool>,
    /// The context root current when this effect was created; re-entered on
    /// every run so `use_context`/`use_store` resolve the same namespace as at
    /// build time (issue #136). `0` = the thread-global fallback root.
    pub(super) root: u64,
    /// The scope that owned this effect when it was created, re-entered on every
    /// run so resources the body creates are attributed to the component that
    /// built it rather than to whatever happened to be rendering when the flush
    /// fired (issue #141). Weak by construction — see [`Owner`](super::Owner);
    /// a strong reference here would make every scope immortal, since `EFFECTS`
    /// holds the `Rc<EffectInner>` until `Scope::dispose` clears it.
    pub(super) owner: super::Owner,
}

impl Effect {
    /// Create a new effect that runs immediately and re-runs when dependencies change.
    pub fn new<F: FnMut() + 'static>(f: F) -> Self {
        let id = RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            ObserverId(rt.next_id())
        });

        let inner = Rc::new(EffectInner {
            id,
            f: RefCell::new(Box::new(f)),
            disposed: Cell::new(false),
            root: crate::context::current_context_root(),
            owner: super::Owner::current(),
        });

        // Store the effect
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            let idx = id.0;
            if idx >= effects.len() {
                effects.resize(idx + 1, None);
            }
            effects[idx] = Some(Rc::clone(&inner));
        });

        // Attribute before the first run: the body runs synchronously below and
        // may create resources of its own (issue #141).
        super::scope::record_effect(id);

        // Run the effect immediately
        run_effect(id);

        Effect { id }
    }

    /// Create an effect that doesn't run immediately.
    pub fn new_deferred<F: FnMut() + 'static>(f: F) -> Self {
        let id = RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            ObserverId(rt.next_id())
        });

        let inner = Rc::new(EffectInner {
            id,
            f: RefCell::new(Box::new(f)),
            disposed: Cell::new(false),
            root: crate::context::current_context_root(),
            owner: super::Owner::current(),
        });

        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            let idx = id.0;
            if idx >= effects.len() {
                effects.resize(idx + 1, None);
            }
            effects[idx] = Some(inner);
        });

        super::scope::record_effect(id);

        Effect { id }
    }

    /// Manually trigger this effect to run.
    pub fn run(&self) {
        run_effect(self.id);
    }

    /// This effect's observer id, for the disposal fixpoint — which works from
    /// the ids a scope recorded rather than from `Effect` handles (issue #141).
    pub(super) fn id(&self) -> ObserverId {
        self.id
    }

    /// Dispose of this effect, preventing it from running again.
    ///
    /// Also clears the slot in the global EFFECTS vec, allowing the
    /// `Rc<EffectInner>` to be reclaimed.
    pub fn dispose(&self) {
        dispose_effect(self.id);
    }
}

/// Dispose the effect with this id, if it is still registered.
///
/// The by-id counterpart of [`Effect::dispose`], and its implementation.
///
/// The `Rc<EffectInner>` is moved out and dropped **after** the registry borrow
/// is released. That is load-bearing, not tidiness: the `Rc` owns the effect's
/// closure, which captures arbitrary user state — very often the only handle to
/// a child `RenderScope`. Dropping it in place (`*slot = None`) runs that state's
/// `Drop` while `EFFECTS` is mutably borrowed, so a `Drop` that writes a signal
/// flushes effects synchronously into `run_effect`, whose `EFFECTS.borrow()`
/// then panics with a `BorrowMutError` (issue #141).
pub(super) fn dispose_effect(id: ObserverId) {
    let inner = EFFECTS.with(|effects| {
        let mut effects = effects.borrow_mut();
        let slot = effects.get_mut(id.0)?;
        if let Some(inner) = slot.as_ref() {
            inner.disposed.set(true);
        }
        slot.take()
    });
    drop(inner);
}

impl Drop for Effect {
    fn drop(&mut self) {
        // Note: We don't automatically dispose here to allow effects to outlive
        // their handles. Use dispose() explicitly if needed.
    }
}

/// RAII guard for the observer stack.
///
/// Pushes an [`ObserverId`] on construction and pops it on drop — including
/// while unwinding. A bare push/pop pair leaks the id when user code panics
/// between them, and a stale observer then subscribes itself to every signal
/// read for the rest of the thread's life (issue #141).
pub(super) struct ObserverGuard;

impl ObserverGuard {
    pub(super) fn push(id: ObserverId) -> Self {
        RUNTIME.with(|rt| rt.borrow_mut().observer_stack.push(id));
        ObserverGuard
    }
}

impl Drop for ObserverGuard {
    fn drop(&mut self) {
        // `try_with`: TLS may already be torn down at thread exit.
        let _ = RUNTIME.try_with(|rt| {
            if let Ok(mut rt) = rt.try_borrow_mut() {
                rt.observer_stack.pop();
            }
        });
    }
}

/// Run a specific effect by ID
pub(super) fn run_effect(id: ObserverId) {
    let effect = EFFECTS.with(|effects| effects.borrow().get(id.0).and_then(|e| e.clone()));

    if let Some(inner) = effect {
        if inner.disposed.get() {
            tracing::debug!("run_effect({}): SKIPPED - disposed", id.0);
            return;
        }

        tracing::debug!("run_effect({}): running", id.0);

        // Re-enter the context root the effect was created under, so
        // use_context/use_store resolve the same namespace as at build time
        // (issue #136).
        let _root_guard = crate::context::push_context_root(inner.root);

        // Re-enter the scope that owned this effect at creation, for the same
        // reason: `flush_effects` runs from arbitrary stacks (an event handler,
        // a timer, the cross-thread drain), so the ambient owner at flush time
        // is unrelated to the component this effect belongs to (issue #141).
        let _owner_guard = inner.owner.push();

        // Push this effect as the current observer. RAII so a panic in the
        // effect body cannot strand it on the stack (issue #141).
        let _observer_guard = ObserverGuard::push(id);

        // An effect never re-enters itself.
        //
        // `f` is borrowed for the whole body, so a *synchronous* re-entry would
        // be a `BorrowMutError` — and re-entry is reachable: a write inside the
        // body flushes effects immediately (outside `batch`), and if the effect
        // observes that signal it is queued and run right there, one frame down
        // its own stack. Since #141 this has a routine trigger — an effect that
        // disposes a scope (every control-flow swap does) runs that scope's
        // cleanups, and a cleanup that writes a signal the effect reads lands
        // exactly here.
        //
        // Skipping is deliberate over re-queuing: re-queuing an effect that is
        // still running turns a self-triggering body into a hang, which is
        // strictly harder to debug than a stale value. The effect re-runs on the
        // next genuine change.
        let Ok(mut body) = inner.f.try_borrow_mut() else {
            tracing::debug!(
                "run_effect({}): SKIPPED - re-entered while already running",
                id.0
            );
            return;
        };
        body();
        drop(body);

        tracing::debug!("run_effect({}): done", id.0);
    } else {
        tracing::debug!("run_effect({}): SKIPPED - no effect found (dropped?)", id.0);
    }
}

/// Flush all pending effects, in the order they were queued.
///
/// FIFO (`pop_front`), so the registration-order guarantee established when
/// enqueuing survives the flush. Draining from the back would run same-signal
/// effects in *reverse* registration order, and would run effects queued by a
/// running effect ahead of ones queued before it. See the "Execution order"
/// section of the [`reactive`](crate::reactive) module docs.
pub(super) fn flush_effects() {
    loop {
        let effect_id = RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            let id = rt.pending_effects.pop_front();
            if id.is_some() {
                // Remove from set when dequeuing
                if let Some(ref observer) = id {
                    rt.pending_effects_set.remove(observer);
                }
            }
            id
        });

        match effect_id {
            Some(id) => run_effect(id),
            None => break,
        }
    }
}

#[cfg(test)]
mod dispose_tests {
    use super::*;
    use crate::reactive::Signal;
    use std::cell::Cell;

    /// Disposing an effect drops its closure **after** the `EFFECTS` borrow is
    /// released.
    ///
    /// The closure captures arbitrary user state. If that state's `Drop` writes
    /// a signal, the write flushes effects synchronously into `run_effect`,
    /// whose `EFFECTS.borrow()` conflicts with a `borrow_mut` still held by the
    /// dispose — a `BorrowMutError`, not a leak. Rare before #141, routine now
    /// that disposal drops every closure a scope owns.
    #[test]
    fn disposing_an_effect_drops_its_closure_outside_the_registry_borrow() {
        struct Noisy(Signal<i32>);
        impl Drop for Noisy {
            fn drop(&mut self) {
                // Wakes `_observer` below, re-entering `run_effect`.
                self.0.set(1);
            }
        }

        let signal = Signal::new(0);
        let woken = Rc::new(Cell::new(0));

        let hits = woken.clone();
        let _observer = Effect::new(move || {
            signal.get();
            hits.set(hits.get() + 1);
        });
        assert_eq!(woken.get(), 1);

        let noisy = Noisy(signal);
        let effect = Effect::new(move || {
            let _held = &noisy;
        });

        effect.dispose(); // must not panic

        assert_eq!(
            woken.get(),
            2,
            "the drop's write reached the surviving observer"
        );
    }

    /// An effect that writes a signal it observes does not re-enter itself.
    ///
    /// `EffectInner::f` is borrowed for the whole body, and a write outside
    /// `batch` flushes synchronously — so a self-observing write lands back in
    /// `run_effect` one frame down its own stack. Unguarded that is a
    /// `BorrowMutError`, and since #141 it has a routine trigger: an effect that
    /// disposes a scope runs that scope's cleanups, and a cleanup that writes a
    /// signal the effect reads arrives exactly here.
    #[test]
    fn an_effect_that_writes_a_signal_it_observes_does_not_re_enter_itself() {
        let signal = Signal::new(0);
        let runs = Rc::new(Cell::new(0));

        let hits = runs.clone();
        let _effect = Effect::new(move || {
            let seen = signal.get();
            hits.set(hits.get() + 1);
            if seen == 0 {
                // Flushes synchronously and re-enters this very effect.
                signal.set(1);
            }
        });

        assert_eq!(runs.get(), 1, "the re-entrant run is skipped, not panicked");
        assert_eq!(signal.get(), 1, "and the write itself still landed");

        // The effect is not wedged: a later genuine change still runs it.
        signal.set(2);
        assert_eq!(runs.get(), 2);
    }
}
