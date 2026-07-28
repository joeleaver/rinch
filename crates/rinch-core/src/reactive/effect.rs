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

    /// Dispose of this effect, preventing it from running again.
    ///
    /// Also clears the slot in the global EFFECTS vec, allowing the
    /// `Rc<EffectInner>` to be reclaimed.
    pub fn dispose(&self) {
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            if let Some(slot) = effects.get_mut(self.id.0) {
                if let Some(inner) = slot.as_ref() {
                    inner.disposed.set(true);
                }
                // Release the Rc to reclaim memory
                *slot = None;
            }
        });
    }
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

        // Run the effect
        (inner.f.borrow_mut())();

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
