//! Signal: a reactive container that holds a value and notifies subscribers when it changes.

use std::fmt;
use std::marker::PhantomData;

use super::{ObserverId, RUNTIME, SIGNAL_STORE};

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
        Self {
            id,
            generation,
            _phantom: PhantomData,
        }
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
                    rt.pending_effects.push(observer);
                }
            }

            tracing::debug!(
                "Signal({}).notify(): batching={}, pending_effects={}",
                self.id,
                rt.batching,
                rt.pending_effects.len()
            );

            // If not batching, flush immediately
            // Effects must run BEFORE on_signal_change callback so fine-grained
            // updates are queued before the callback decides whether to do a full re-render
            if !rt.batching {
                drop(rt);

                super::flush_effects();

                // Invoke the UI re-render callback AFTER Effects have run
                let callback = RUNTIME.with(|rt| rt.borrow().on_signal_change.clone());
                if let Some(callback) = callback {
                    callback();
                }
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
    /// Panics if the signal has been freed (use-after-free).
    pub fn get(&self) -> T {
        self.track();
        SIGNAL_STORE.with(|store| {
            let store = store.borrow();
            let slot = store
                .get_slot(self.id, self.generation)
                .expect("Signal::get() on freed signal");
            slot.value
                .downcast_ref::<T>()
                .expect("Signal type mismatch (internal error)")
                .clone()
        })
    }
}

impl<T: 'static> Signal<T> {
    /// Get a reference to the current value without cloning.
    ///
    /// If called inside an effect, this automatically subscribes the effect
    /// to this signal.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.track();
        SIGNAL_STORE.with(|store| {
            let store = store.borrow();
            let slot = store
                .get_slot(self.id, self.generation)
                .expect("Signal::with() on freed signal");
            f(slot
                .value
                .downcast_ref::<T>()
                .expect("Signal type mismatch (internal error)"))
        })
    }

    /// Set the signal to a new value.
    ///
    /// This will notify all subscribers to re-run.
    pub fn set(&self, value: T) {
        SIGNAL_STORE.with(|store| {
            let mut store = store.borrow_mut();
            let slot = store
                .get_slot_mut(self.id, self.generation)
                .expect("Signal::set() on freed signal");
            slot.value = Box::new(value);
        });
        self.notify();
    }

    /// Update the signal's value using a function.
    ///
    /// This will notify all subscribers to re-run.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        SIGNAL_STORE.with(|store| {
            let mut store = store.borrow_mut();
            let slot = store
                .get_slot_mut(self.id, self.generation)
                .expect("Signal::update() on freed signal");
            let value = slot
                .value
                .downcast_mut::<T>()
                .expect("Signal type mismatch (internal error)");
            f(value);
        });
        self.notify();
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
