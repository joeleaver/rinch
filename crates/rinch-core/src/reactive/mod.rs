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

mod effect;
mod memo;
mod scope;
mod signal;

pub use effect::Effect;
pub use memo::Memo;
pub use scope::Scope;
pub use signal::Signal;

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

// Re-export flush_effects for use by signal::notify and batch
use effect::flush_effects;

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

pub(crate) struct Runtime {
    /// Stack of currently executing observers
    pub(crate) observer_stack: Vec<ObserverId>,

    /// Effects that need to run
    pub(crate) pending_effects: Vec<ObserverId>,

    /// Set for O(1) duplicate check when enqueuing pending effects
    pub(crate) pending_effects_set: HashSet<ObserverId>,

    /// Whether we're currently in a batch
    pub(crate) batching: bool,

    /// Counter for generating unique IDs
    next_id: usize,

    /// Callback invoked when any signal changes (for UI re-render)
    pub(crate) on_signal_change: Option<Rc<dyn Fn()>>,

    /// Flag set when any signal changes (reset by `clear_signals_changed`)
    pub(crate) signals_changed: bool,
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

    pub(crate) fn next_id(&mut self) -> usize {
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

// ============================================================================
// Cross-Thread Signal Dispatch
// ============================================================================

/// The thread ID of the main (UI) thread, set once at runtime startup.
static MAIN_THREAD_ID: OnceLock<std::thread::ThreadId> = OnceLock::new();

/// Dispatcher function for sending closures to the main thread.
type DispatchFn = fn(Box<dyn FnOnce() + Send>);
static CROSS_THREAD_DISPATCHER: Mutex<Option<DispatchFn>> = Mutex::new(None);

/// Register the current thread as the main (UI) thread.
///
/// Called by the rinch runtime at startup. Must be called from the main thread
/// before any signals are used cross-thread.
pub fn register_main_thread() {
    MAIN_THREAD_ID.get_or_init(|| std::thread::current().id());
}

/// Register a function that dispatches closures to the main thread.
///
/// Called by the rinch runtime at startup. The dispatcher should queue the
/// closure for execution on the main thread and wake the event loop.
pub fn set_cross_thread_dispatcher(dispatcher: fn(Box<dyn FnOnce() + Send>)) {
    *CROSS_THREAD_DISPATCHER.lock().unwrap() = Some(dispatcher);
}

/// Check if the current thread is the main (UI) thread.
///
/// Returns `true` if `register_main_thread()` hasn't been called yet
/// (backwards compatibility for tests and non-rinch usage).
pub(crate) fn is_main_thread() -> bool {
    match MAIN_THREAD_ID.get() {
        Some(id) => std::thread::current().id() == *id,
        None => true,
    }
}

/// Dispatch a closure to the main thread via the registered dispatcher.
///
/// Panics if no dispatcher has been registered (i.e., rinch runtime not initialized).
pub(crate) fn dispatch_to_main_thread(f: Box<dyn FnOnce() + Send>) {
    let dispatcher = CROSS_THREAD_DISPATCHER.lock().unwrap();
    if let Some(dispatch) = *dispatcher {
        dispatch(f);
    } else {
        panic!(
            "Signal::send() called but no cross-thread dispatcher is registered. \
             Ensure the rinch runtime is initialized before using send() from background threads."
        );
    }
}

/// Unique identifier for an observer (effect or memo)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct ObserverId(pub(crate) usize);

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

pub(crate) struct SignalSlot {
    pub(crate) value: Box<dyn Any>,
    pub(crate) subscribers: HashSet<ObserverId>,
    pub(crate) generation: u32,
}

pub(crate) struct SignalStore {
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

    pub(crate) fn alloc<T: 'static>(&mut self, value: T) -> (u32, u32) {
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

    pub(crate) fn get_slot(&self, id: u32, generation: u32) -> Option<&SignalSlot> {
        self.slots
            .get(id as usize)?
            .as_ref()
            .filter(|s| s.generation == generation)
    }

    pub(crate) fn get_slot_mut(&mut self, id: u32, generation: u32) -> Option<&mut SignalSlot> {
        self.slots
            .get_mut(id as usize)?
            .as_mut()
            .filter(|s| s.generation == generation)
    }

    #[allow(dead_code)]
    fn free(&mut self, id: u32, generation: u32) {
        if let Some(slot) = self.slots.get(id as usize)
            && slot.as_ref().is_some_and(|s| s.generation == generation)
        {
            self.slots[id as usize] = None;
            self.free_list.push(id);
        }
    }
}

// ============================================================================
// Memo Storage
// ============================================================================

thread_local! {
    static MEMO_STORE: RefCell<MemoStore> = RefCell::new(MemoStore::new());
}

struct MemoSlot {
    inner: Rc<dyn Any>, // Type-erased Rc<MemoInner<T>>
    generation: u32,
}

pub(crate) struct MemoStore {
    slots: Vec<Option<MemoSlot>>,
    free_list: Vec<u32>,
    next_gen: u32,
}

impl MemoStore {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_list: Vec::new(),
            next_gen: 1,
        }
    }

    pub(crate) fn alloc(&mut self, inner: Rc<dyn Any>) -> (u32, u32) {
        let generation = self.next_gen;
        self.next_gen = self.next_gen.wrapping_add(1);
        if self.next_gen == 0 {
            self.next_gen = 1;
        }

        let slot = MemoSlot { inner, generation };

        if let Some(idx) = self.free_list.pop() {
            self.slots[idx as usize] = Some(slot);
            (idx, generation)
        } else {
            let idx = self.slots.len() as u32;
            self.slots.push(Some(slot));
            (idx, generation)
        }
    }

    pub(crate) fn get_inner(&self, id: u32, generation: u32) -> Option<Rc<dyn Any>> {
        self.slots
            .get(id as usize)?
            .as_ref()
            .filter(|s| s.generation == generation)
            .map(|s| Rc::clone(&s.inner))
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
    let callback = RUNTIME.with(|rt| rt.borrow().on_signal_change.clone());
    if let Some(callback) = callback {
        callback();
    }

    result
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
    let observer = RUNTIME.with(|rt| rt.borrow_mut().observer_stack.pop());

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

    #[test]
    fn memo_is_copy() {
        let count = Signal::new(2);
        let doubled = Memo::new(move || count.get() * 2);

        // Memo is Copy - can use in multiple closures without .clone()
        let a = doubled;
        let b = doubled;
        assert_eq!(a.get(), 4);
        assert_eq!(b.get(), 4);

        count.set(5);
        assert_eq!(a.get(), 10);
        assert_eq!(b.get(), 10);
    }
}
