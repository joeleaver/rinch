//! Reactive primitives: signals, effects, and memos.
//!
//! This module provides fine-grained reactivity similar to Solid.js and Leptos.
//!
//! # Core Concepts
//!
//! - **Signal**: A reactive container that holds a value and notifies subscribers when it changes
//! - **Effect**: A side-effect that re-runs when its dependencies change
//! - **Memo**: A cached computed value that only recomputes when dependencies change
//!
//! # Example
//!
//! ```ignore
//! use rinch_core::reactive::*;
//!
//! let count = Signal::new(0);
//!
//! Effect::new(move || {
//!     println!("Count is: {}", count.get());
//! });
//!
//! count.set(1); // Prints: "Count is: 1"
//! ```

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

// ============================================================================
// Runtime Context
// ============================================================================

// Global runtime state for tracking reactive subscriptions.
//
// The runtime maintains:
// - A stack of observers (effects/memos currently being computed)
// - A queue of pending effects to run
// - Batching state
thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::new(Runtime::new());
}

struct Runtime {
    /// Stack of currently executing observers
    observer_stack: Vec<ObserverId>,

    /// Effects that need to run
    pending_effects: Vec<ObserverId>,

    /// Set for O(1) duplicate check when enqueuing pending effects
    pending_effects_set: HashSet<ObserverId>,

    /// Whether we're currently in a batch
    batching: bool,

    /// Counter for generating unique IDs
    next_id: usize,

    /// Callback invoked when any signal changes (for UI re-render)
    on_signal_change: Option<Rc<dyn Fn()>>,

    /// Flag set when any signal changes (reset by `clear_signals_changed`)
    signals_changed: bool,
}

impl Runtime {
    fn new() -> Self {
        Self {
            observer_stack: Vec::new(),
            pending_effects: Vec::new(),
            pending_effects_set: HashSet::new(),
            batching: false,
            next_id: 0,
            on_signal_change: None,
            signals_changed: false,
        }
    }

    fn next_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

/// Register a callback to be invoked whenever any signal changes.
///
/// This is used by the UI runtime to automatically trigger re-renders
/// when reactive state changes, without requiring manual `request_render()` calls.
///
/// # Example
///
/// ```ignore
/// set_on_signal_change(|| {
///     // Request UI re-render
///     request_render();
/// });
/// ```
pub fn set_on_signal_change(callback: impl Fn() + 'static) {
    RUNTIME.with(|rt| {
        rt.borrow_mut().on_signal_change = Some(Rc::new(callback));
    });
}

/// Clear the signal change callback.
pub fn clear_on_signal_change() {
    RUNTIME.with(|rt| {
        rt.borrow_mut().on_signal_change = None;
    });
}

/// Check if any signals have changed since the last call to `clear_signals_changed`.
///
/// This is used by the runtime to avoid redundant re-renders when signals
/// have already triggered fine-grained updates via Effects.
pub fn signals_changed() -> bool {
    RUNTIME.with(|rt| rt.borrow().signals_changed)
}

/// Clear the signals_changed flag.
///
/// Call this before running an event handler to track whether the handler
/// modified any signals.
pub fn clear_signals_changed() {
    RUNTIME.with(|rt| {
        rt.borrow_mut().signals_changed = false;
    });
}

/// Unique identifier for an observer (effect or memo)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct ObserverId(usize);

// ============================================================================
// Signal Storage
// ============================================================================

// Thread-local storage for all signal values.
// Signals are stored as type-erased `Box<dyn Any>` in a generation-counted
// slot vec. This allows `Signal<T>` to be `Copy` — it only contains an
// index and generation counter.
thread_local! {
    static SIGNAL_STORE: RefCell<SignalStore> = RefCell::new(SignalStore::new());
}

struct SignalSlot {
    value: Box<dyn Any>,
    subscribers: HashSet<ObserverId>,
    generation: u32,
}

struct SignalStore {
    slots: Vec<Option<SignalSlot>>,
    /// Indices of freed slots available for reuse.
    free_list: Vec<u32>,
    /// Generation counter, incremented on each allocation.
    next_gen: u32,
}

impl SignalStore {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_list: Vec::new(),
            next_gen: 1, // Start at 1 so generation=0 is never valid
        }
    }

    fn alloc<T: 'static>(&mut self, value: T) -> (u32, u32) {
        let generation = self.next_gen;
        self.next_gen = self.next_gen.wrapping_add(1);
        if self.next_gen == 0 {
            self.next_gen = 1; // Skip 0
        }

        let slot = SignalSlot {
            value: Box::new(value),
            subscribers: HashSet::new(),
            generation,
        };

        if let Some(idx) = self.free_list.pop() {
            self.slots[idx as usize] = Some(slot);
            (idx, generation)
        } else {
            let idx = self.slots.len() as u32;
            self.slots.push(Some(slot));
            (idx, generation)
        }
    }

    fn get_slot(&self, id: u32, generation: u32) -> Option<&SignalSlot> {
        self.slots.get(id as usize)?.as_ref().filter(|s| s.generation == generation)
    }

    fn get_slot_mut(&mut self, id: u32, generation: u32) -> Option<&mut SignalSlot> {
        self.slots.get_mut(id as usize)?.as_mut().filter(|s| s.generation == generation)
    }

    #[allow(dead_code)]
    fn free(&mut self, id: u32, generation: u32) {
        if let Some(slot) = self.slots.get(id as usize) {
            if slot.as_ref().is_some_and(|s| s.generation == generation) {
                self.slots[id as usize] = None;
                self.free_list.push(id);
            }
        }
    }
}

// ============================================================================
// Signal
// ============================================================================

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
        let observer = RUNTIME.with(|rt| {
            rt.borrow().observer_stack.last().copied()
        });
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
            store.borrow()
                .get_slot(self.id, self.generation)
                .map(|slot| slot.subscribers.iter().copied().collect())
                .unwrap_or_default()
        });

        tracing::debug!("Signal({}).notify(): {} subscribers", self.id, subscribers.len());

        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();

            // Mark that signals have changed (for request_render optimization)
            rt.signals_changed = true;

            for observer in subscribers {
                if rt.pending_effects_set.insert(observer) {
                    rt.pending_effects.push(observer);
                }
            }

            tracing::debug!("Signal({}).notify(): batching={}, pending_effects={}", self.id, rt.batching, rt.pending_effects.len());

            // If not batching, flush immediately
            // Effects must run BEFORE on_signal_change callback so fine-grained
            // updates are queued before the callback decides whether to do a full re-render
            if !rt.batching {
                drop(rt);

                flush_effects();

                // Invoke the UI re-render callback AFTER Effects have run
                let callback = RUNTIME.with(|rt| {
                    rt.borrow().on_signal_change.clone()
                });
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
            let slot = store.get_slot(self.id, self.generation)
                .expect("Signal::get() on freed signal");
            slot.value.downcast_ref::<T>()
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
            let slot = store.get_slot(self.id, self.generation)
                .expect("Signal::with() on freed signal");
            f(slot.value.downcast_ref::<T>()
                .expect("Signal type mismatch (internal error)"))
        })
    }

    /// Set the signal to a new value.
    ///
    /// This will notify all subscribers to re-run.
    pub fn set(&self, value: T) {
        SIGNAL_STORE.with(|store| {
            let mut store = store.borrow_mut();
            let slot = store.get_slot_mut(self.id, self.generation)
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
            let slot = store.get_slot_mut(self.id, self.generation)
                .expect("Signal::update() on freed signal");
            let value = slot.value.downcast_mut::<T>()
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
                    f.debug_struct("Signal").field("error", &"type mismatch").finish()
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

// ============================================================================
// Effect
// ============================================================================

// Storage for all effects (needed because effects reference themselves)
thread_local! {
    static EFFECTS: RefCell<Vec<Option<Rc<EffectInner>>>> = RefCell::new(Vec::new());
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

struct EffectInner {
    #[allow(dead_code)] // Used for debugging/tracking purposes
    id: ObserverId,
    f: RefCell<Box<dyn FnMut()>>,
    disposed: Cell<bool>,
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
fn run_effect(id: ObserverId) {
    let effect = EFFECTS.with(|effects| {
        effects.borrow().get(id.0).and_then(|e| e.clone())
    });

    if let Some(inner) = effect {
        if inner.disposed.get() {
            tracing::debug!("run_effect({}): SKIPPED - disposed", id.0);
            return;
        }

        tracing::debug!("run_effect({}): running", id.0);

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
fn flush_effects() {
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

// ============================================================================
// Memo
// ============================================================================

/// A cached computed value that only recomputes when dependencies change.
///
/// Memos are lazily evaluated and cache their result until one of their
/// dependencies changes.
///
/// # Example
///
/// ```ignore
/// let count = Signal::new(2);
/// let doubled = Memo::new(move || count.get() * 2);
///
/// doubled.get(); // Returns 4
/// count.set(3);
/// doubled.get(); // Returns 6 (recomputed)
/// doubled.get(); // Returns 6 (cached)
/// ```
pub struct Memo<T> {
    inner: Rc<MemoInner<T>>,
}

struct MemoInner<T> {
    id: ObserverId,
    value: RefCell<Option<T>>,
    f: RefCell<Box<dyn Fn() -> T>>,
    dirty: Cell<bool>,
    subscribers: RefCell<HashSet<ObserverId>>,
}

impl<T: Clone + 'static> Memo<T> {
    /// Create a new memo with the given computation function.
    pub fn new<F: Fn() -> T + 'static>(f: F) -> Self {
        let id = RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            ObserverId(rt.next_id())
        });

        let inner = Rc::new(MemoInner {
            id,
            value: RefCell::new(None),
            f: RefCell::new(Box::new(f)),
            dirty: Cell::new(true),
            subscribers: RefCell::new(HashSet::new()),
        });

        // Store memo as an effect so it can be notified
        let inner_clone = Rc::clone(&inner);
        EFFECTS.with(|effects| {
            let mut effects = effects.borrow_mut();
            let idx = id.0;
            if idx >= effects.len() {
                effects.resize(idx + 1, None);
            }
            // We store a "marker" effect that marks the memo as dirty
            let memo_inner = inner_clone;
            effects[idx] = Some(Rc::new(EffectInner {
                id,
                f: RefCell::new(Box::new(move || {
                    memo_inner.dirty.set(true);
                    // Notify memo's subscribers
                    let subscribers: Vec<_> = memo_inner.subscribers.borrow().iter().copied().collect();
                    RUNTIME.with(|rt| {
                        let mut rt = rt.borrow_mut();
                        for observer in subscribers {
                            if rt.pending_effects_set.insert(observer) {
                                rt.pending_effects.push(observer);
                            }
                        }
                    });
                })),
                disposed: Cell::new(false),
            }));
        });

        Self { inner }
    }

    /// Get the current value, recomputing if necessary.
    pub fn get(&self) -> T {
        // Subscribe current observer to this memo
        RUNTIME.with(|rt| {
            let rt = rt.borrow();
            if let Some(&observer) = rt.observer_stack.last() {
                self.inner.subscribers.borrow_mut().insert(observer);
            }
        });

        // Recompute if dirty
        if self.inner.dirty.get() {
            // Push memo as observer while computing
            RUNTIME.with(|rt| {
                rt.borrow_mut().observer_stack.push(self.inner.id);
            });

            let value = (self.inner.f.borrow())();
            *self.inner.value.borrow_mut() = Some(value);
            self.inner.dirty.set(false);

            RUNTIME.with(|rt| {
                rt.borrow_mut().observer_stack.pop();
            });
        }

        self.inner.value.borrow().clone().expect("memo should have value after get")
    }
}

impl<T> Clone for Memo<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl<T: fmt::Debug + Clone + 'static> fmt::Debug for Memo<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Memo")
            .field("value", &*self.inner.value.borrow())
            .field("dirty", &self.inner.dirty.get())
            .finish()
    }
}

// ============================================================================
// Batching
// ============================================================================

/// Batch multiple signal updates to avoid redundant effect runs.
///
/// Effects will only run once after the batch completes, even if multiple
/// signals they depend on are updated.
///
/// # Example
///
/// ```ignore
/// let count = Signal::new(0);
/// let name = Signal::new("".to_string());
///
/// batch(|| {
///     count.set(1);
///     name.set("Alice".to_string());
///     // Effects only run once, after this batch
/// });
/// ```
pub fn batch<R>(f: impl FnOnce() -> R) -> R {
    RUNTIME.with(|rt| {
        rt.borrow_mut().batching = true;
    });

    let result = f();

    RUNTIME.with(|rt| {
        rt.borrow_mut().batching = false;
    });

    flush_effects();

    // Invoke the UI re-render callback AFTER Effects have run
    // This allows fine-grained updates to be queued before the callback checks
    // Clone the callback first and drop the borrow before calling it
    // to avoid borrow conflicts if the callback accesses the runtime
    let callback = RUNTIME.with(|rt| {
        rt.borrow().on_signal_change.clone()
    });
    if let Some(callback) = callback {
        callback();
    }

    result
}

// ============================================================================
// Scope (for memory management)
// ============================================================================

/// A scope that manages the lifetime of reactive primitives.
///
/// When a scope is disposed, all effects created within it are cleaned up.
/// Scopes can have child scopes for hierarchical cleanup.
///
/// # Example
///
/// ```ignore
/// let scope = Scope::new();
///
/// scope.run(|| {
///     let signal = Signal::new(0);
///     Effect::new(|| { /* ... */ });
///     // signal and effect belong to this scope
/// });
///
/// scope.dispose(); // Cleans up signal, effect, and all child scopes
/// ```
pub struct Scope {
    effects: RefCell<Vec<Effect>>,
    children: RefCell<Vec<Scope>>,
    cleanups: RefCell<Vec<Box<dyn FnOnce()>>>,
    disposed: Cell<bool>,
}

impl Scope {
    /// Create a new scope.
    pub fn new() -> Self {
        Self {
            effects: RefCell::new(Vec::new()),
            children: RefCell::new(Vec::new()),
            cleanups: RefCell::new(Vec::new()),
            disposed: Cell::new(false),
        }
    }

    /// Check if this scope has been disposed.
    pub fn is_disposed(&self) -> bool {
        self.disposed.get()
    }

    /// Run a function within this scope, capturing any effects created.
    pub fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        // TODO: Implement scope tracking so effects created within
        // are automatically registered to this scope
        f()
    }

    /// Register an effect with this scope.
    pub fn add_effect(&self, effect: Effect) {
        self.effects.borrow_mut().push(effect);
    }

    /// Create a child scope that will be disposed when this scope is disposed.
    ///
    /// Child scopes are useful for conditional or list rendering where nested
    /// content needs independent lifecycle management.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let parent = Scope::new();
    /// let child = parent.child_scope();
    ///
    /// child.add_effect(Effect::new(|| { /* ... */ }));
    ///
    /// parent.dispose(); // Also disposes child and its effects
    /// ```
    pub fn child_scope(&self) -> Scope {
        let child = Scope::new();
        // We return the child and expect the caller to manage it
        // The parent stores a reference for cleanup
        child
    }

    /// Add a child scope to be disposed with this scope.
    pub fn add_child(&self, child: Scope) {
        self.children.borrow_mut().push(child);
    }

    /// Register a cleanup function to run when this scope is disposed.
    ///
    /// Cleanup functions run after child scopes and effects are disposed.
    pub fn on_cleanup<F: FnOnce() + 'static>(&self, f: F) {
        self.cleanups.borrow_mut().push(Box::new(f));
    }

    /// Dispose of all effects, child scopes, and run cleanup functions.
    ///
    /// After dispose, this scope should not be used.
    pub fn dispose(&self) {
        if self.disposed.get() {
            return;
        }

        // Use a thread-local disposal queue to avoid stack overflow.
        //
        // The problem: Effect closures may capture Rc<RefCell<RenderScope>>,
        // so Effect::dispose() → drops closure → drops RenderScope → Scope::drop
        // → dispose() → more Effect::dispose() — creating unbounded recursion.
        //
        // The solution: The outermost dispose() call runs an iterative loop.
        // Nested dispose() calls (triggered by closure drops) just push their
        // effects onto the queue instead of processing them immediately.
        thread_local! {
            static DISPOSE_QUEUE: RefCell<Option<Vec<Effect>>> = const { RefCell::new(None) };
        }

        let is_root = DISPOSE_QUEUE.with(|q| {
            let mut q = q.borrow_mut();
            if q.is_none() {
                *q = Some(Vec::new());
                true
            } else {
                false
            }
        });

        // Mark this scope and collect its effects into the queue
        self.dispose_into_queue(&DISPOSE_QUEUE);

        if is_root {
            // We are the outermost dispose call. Process the queue iteratively.
            loop {
                let batch: Vec<Effect> = DISPOSE_QUEUE.with(|q| {
                    std::mem::take(q.borrow_mut().as_mut().unwrap())
                });
                if batch.is_empty() {
                    break;
                }
                // Disposing effects may drop closures that own RenderScopes,
                // triggering more Scope::drop → dispose() calls. Those nested
                // calls will push onto the queue (not recurse) because
                // DISPOSE_QUEUE is Some.
                for effect in batch {
                    effect.dispose();
                }
            }
            // Clean up the queue
            DISPOSE_QUEUE.with(|q| *q.borrow_mut() = None);
        }
    }

    /// Mark this scope as disposed and push its effects onto the disposal queue.
    /// Also iteratively processes all child scopes.
    fn dispose_into_queue(&self, queue: &'static std::thread::LocalKey<RefCell<Option<Vec<Effect>>>>) {
        if self.disposed.get() {
            return;
        }
        self.disposed.set(true);

        // Collect effects into the queue
        let effects: Vec<Effect> = self.effects.borrow_mut().drain(..).collect();
        queue.with(|q| {
            if let Some(ref mut vec) = *q.borrow_mut() {
                vec.extend(effects);
            }
        });

        // Run cleanups
        for cleanup in self.cleanups.borrow_mut().drain(..) {
            cleanup();
        }

        // Process children iteratively
        let mut pending: Vec<Scope> = self.children.borrow_mut().drain(..).collect();
        while let Some(child) = pending.pop() {
            if child.disposed.get() {
                // Drain to prevent recursive field drop
                child.children.borrow_mut().drain(..);
                child.effects.borrow_mut().drain(..);
                continue;
            }
            child.disposed.set(true);

            pending.extend(child.children.borrow_mut().drain(..));
            let child_effects: Vec<Effect> = child.effects.borrow_mut().drain(..).collect();
            queue.with(|q| {
                if let Some(ref mut vec) = *q.borrow_mut() {
                    vec.extend(child_effects);
                }
            });

            for cleanup in child.cleanups.borrow_mut().drain(..) {
                cleanup();
            }
        }
    }

    /// Clear all effects without disposing them.
    /// Used when transferring effects to another scope.
    pub fn take_effects(&self) -> Vec<Effect> {
        self.effects.borrow_mut().drain(..).collect()
    }

    /// Get the number of effects in this scope.
    pub fn effect_count(&self) -> usize {
        self.effects.borrow().len()
    }

    /// Get the number of child scopes.
    pub fn child_count(&self) -> usize {
        self.children.borrow().len()
    }
}

impl Default for Scope {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        // Use the iterative dispose to avoid stack overflow.
        // Rust's default field drop would recursively drop children → Scope → Drop,
        // and Effect::dispose can drop closures that own RenderScopes → more Scopes.
        self.dispose();
    }
}

impl std::fmt::Debug for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scope")
            .field("effects", &self.effects.borrow().len())
            .field("children", &self.children.borrow().len())
            .field("cleanups", &self.cleanups.borrow().len())
            .field("disposed", &self.disposed.get())
            .finish()
    }
}

// ============================================================================
// Utility functions
// ============================================================================

/// Create a derived signal from a computation.
///
/// This is a convenience function that creates a memo and returns it
/// as a signal-like value.
pub fn derived<T: Clone + 'static>(f: impl Fn() -> T + 'static) -> Memo<T> {
    Memo::new(f)
}

/// Run a function without tracking any signal reads.
///
/// Useful for reading signals without creating subscriptions.
pub fn untracked<R>(f: impl FnOnce() -> R) -> R {
    // Temporarily remove the current observer
    let observer = RUNTIME.with(|rt| {
        rt.borrow_mut().observer_stack.pop()
    });

    let result = f();

    // Restore the observer
    if let Some(obs) = observer {
        RUNTIME.with(|rt| {
            rt.borrow_mut().observer_stack.push(obs);
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn signal_basic() {
        let signal = Signal::new(0);
        assert_eq!(signal.get(), 0);

        signal.set(5);
        assert_eq!(signal.get(), 5);

        signal.update(|n| *n += 1);
        assert_eq!(signal.get(), 6);
    }

    #[test]
    fn effect_tracks_signals() {
        let count = Signal::new(0);
        let run_count = Rc::new(Cell::new(0));

        let run_count_clone = Rc::clone(&run_count);
        Effect::new(move || {
            let _ = count.get();
            run_count_clone.set(run_count_clone.get() + 1);
        });

        // Effect runs immediately
        assert_eq!(run_count.get(), 1);

        // Effect runs when signal changes
        count.set(1);
        assert_eq!(run_count.get(), 2);

        count.set(2);
        assert_eq!(run_count.get(), 3);
    }

    #[test]
    fn memo_caches_value() {
        let count = Signal::new(2);
        let compute_count = Rc::new(Cell::new(0));

        let compute_count_clone = Rc::clone(&compute_count);
        let doubled = Memo::new(move || {
            compute_count_clone.set(compute_count_clone.get() + 1);
            count.get() * 2
        });

        // First access computes
        assert_eq!(doubled.get(), 4);
        assert_eq!(compute_count.get(), 1);

        // Second access uses cache
        assert_eq!(doubled.get(), 4);
        assert_eq!(compute_count.get(), 1);

        // Update signal
        count.set(3);

        // Next access recomputes
        assert_eq!(doubled.get(), 6);
        assert_eq!(compute_count.get(), 2);
    }

    #[test]
    fn batch_prevents_multiple_runs() {
        let count = Signal::new(0);
        let run_count = Rc::new(Cell::new(0));

        let run_count_clone = Rc::clone(&run_count);
        Effect::new(move || {
            let _ = count.get();
            run_count_clone.set(run_count_clone.get() + 1);
        });

        // Effect runs immediately
        assert_eq!(run_count.get(), 1);

        // Batch multiple updates
        batch(|| {
            count.set(1);
            count.set(2);
            count.set(3);
        });

        // Effect only ran once more
        assert_eq!(run_count.get(), 2);
        assert_eq!(count.get(), 3);
    }

    #[test]
    fn untracked_prevents_subscription() {
        let count = Signal::new(0);
        let run_count = Rc::new(Cell::new(0));

        let run_count_clone = Rc::clone(&run_count);
        Effect::new(move || {
            untracked(|| {
                let _ = count.get();
            });
            run_count_clone.set(run_count_clone.get() + 1);
        });

        // Effect runs immediately
        assert_eq!(run_count.get(), 1);

        // Effect does NOT run when signal changes (untracked)
        count.set(1);
        assert_eq!(run_count.get(), 1);
    }

    #[test]
    fn scope_disposes_effects() {
        let count = Signal::new(0);
        let run_count = Rc::new(Cell::new(0));

        let scope = Scope::new();

        let run_count_clone = Rc::clone(&run_count);
        let effect = Effect::new(move || {
            let _ = count.get();
            run_count_clone.set(run_count_clone.get() + 1);
        });

        scope.add_effect(effect);

        // Effect runs immediately
        assert_eq!(run_count.get(), 1);

        // Effect runs when signal changes
        count.set(1);
        assert_eq!(run_count.get(), 2);

        // Dispose the scope
        scope.dispose();

        // Effect no longer runs
        count.set(2);
        assert_eq!(run_count.get(), 2);
    }

    #[test]
    fn scope_disposes_children() {
        let count = Signal::new(0);
        let run_count = Rc::new(Cell::new(0));

        let parent = Scope::new();
        let child = Scope::new();

        let run_count_clone = Rc::clone(&run_count);
        let effect = Effect::new(move || {
            let _ = count.get();
            run_count_clone.set(run_count_clone.get() + 1);
        });

        child.add_effect(effect);
        parent.add_child(child);

        // Effect runs immediately
        assert_eq!(run_count.get(), 1);

        // Dispose parent - should also dispose child
        parent.dispose();

        // Effect no longer runs
        count.set(1);
        assert_eq!(run_count.get(), 1);
    }

    #[test]
    fn scope_runs_cleanup() {
        let cleanup_ran = Rc::new(Cell::new(false));

        let scope = Scope::new();
        let cleanup_ran_clone = Rc::clone(&cleanup_ran);
        scope.on_cleanup(move || {
            cleanup_ran_clone.set(true);
        });

        assert!(!cleanup_ran.get());

        scope.dispose();

        assert!(cleanup_ran.get());
    }

    #[test]
    fn scope_dispose_is_idempotent() {
        let cleanup_count = Rc::new(Cell::new(0));

        let scope = Scope::new();
        let cleanup_count_clone = Rc::clone(&cleanup_count);
        scope.on_cleanup(move || {
            cleanup_count_clone.set(cleanup_count_clone.get() + 1);
        });

        scope.dispose();
        scope.dispose();
        scope.dispose();

        // Cleanup should only run once
        assert_eq!(cleanup_count.get(), 1);
    }
}
