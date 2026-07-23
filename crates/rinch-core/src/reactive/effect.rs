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
        });

        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            let idx = id.0;
            if idx >= effects.len() {
                effects.resize(idx + 1, None);
            }
            effects[idx] = Some(inner);
        });

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

        // Push this effect as the current observer
        RUNTIME.with(|rt| {
            rt.borrow_mut().observer_stack.push(id);
        });

        // Run the effect
        (inner.f.borrow_mut())();

        tracing::debug!("run_effect({}): done", id.0);

        // Pop the observer
        RUNTIME.with(|rt| {
            rt.borrow_mut().observer_stack.pop();
        });
    } else {
        tracing::debug!("run_effect({}): SKIPPED - no effect found (dropped?)", id.0);
    }
}

/// Flush all pending effects
pub(super) fn flush_effects() {
    loop {
        let effect_id = RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            let id = rt.pending_effects.pop();
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
