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
//! # Execution order
//!
//! When several observers depend on the same signal, they run in **registration
//! order** — the order their [`Effect`]s/[`Memo`]s were created. This is a
//! guaranteed contract, not an implementation detail (issue #154), and it makes
//! the "run me last" idiom well-defined: an effect registered *after* an `rsx!`
//! tree observes the post-patch DOM in the same synchronous flush, so measuring
//! effects can be layered on top of rendering effects.
//!
//! Two mechanisms enforce it, and both are load-bearing:
//!
//! 1. Subscriber sets are [`BTreeSet<ObserverId>`](std::collections::BTreeSet).
//!    `ObserverId`s come from one monotonic counter and are never reused, so
//!    ascending id *is* registration order — the ordering is intrinsic to the
//!    data structure rather than a sort someone can forget to apply.
//! 2. The pending queue is drained FIFO, so the order observers were queued in
//!    is the order they run in. (An effect that writes a signal while running
//!    queues that signal's observers *behind* the current flush, rather than
//!    ahead of it.)
//!
//! # Liveness
//!
//! [`Signal`] and [`Memo`] are `Copy` index handles, not owners. Their storage
//! is freed when the scope that owns it is disposed, and a handle can outlive
//! that — captured by a detached worker thread, a global callback, or a drag in
//! flight. The contract for a handle whose storage is gone splits by direction:
//!
//! > **You may always write to a handle; you may only read a live one.**
//!
//! - **Reads panic.** [`Signal::get`]/[`Signal::with`] and [`Memo::get`] have no
//!   value to return — `T` is not `Default` — so a lenient read is not
//!   expressible. Use [`try_get`](Signal::try_get)/[`try_with`](Signal::try_with)
//!   for the `Option` form.
//! - **Writes are dropped, with a warning.** [`Signal::set`],
//!   [`set_if_changed`](Signal::set_if_changed) and [`update`](Signal::update)
//!   no-op and log once per call site. A background thread cannot check-then-write
//!   without a race, and panicking there would take down the app for a write
//!   nobody is waiting on. The cost is that side effects inside an `update`
//!   closure are lost with it.
//! - **[`is_alive`](Signal::is_alive) asks directly**, and does not subscribe —
//!   liveness is not reactive.
//!
//! Registries that drive signals ([`poll_signal`], [`register_bounds_signal`])
//! derive their own lifetime from this: an entry is dropped once the signal it
//! writes is gone, so they self-prune rather than spinning on dead handles.
//!
//! See issue #141.
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

mod bounds;
mod effect;
mod memo;
mod poll;
mod scope;
mod scoped;
mod signal;

pub use bounds::{
    ElementBounds, register_bounds_signal, registered_bounds_nodes, update_bounds_signals,
};
pub use effect::Effect;
pub use memo::Memo;
pub use poll::{PollRate, drain_polls, poll_signal};
pub use scope::{OwnedCounts, Owner, OwnerGuard, Scope, current_owner, on_cleanup, unowned};
/// Ambient-owner hooks for the rest of the crate: `crate::events` attributes
/// handlers to the scope currently rendering, and `crate::context` ties a
/// store/context entry to the scope that created it (issue #141).
pub(crate) use scope::{on_cleanup_for_ambient_owner, record_handler};
pub use scoped::{
    DocScopedSlotMap, clear_doc_scoped_slot, clear_scoped_slot, install_doc_scoped_slot,
    install_scoped_slot, read_doc_scoped_slot, read_scoped_slot,
};
pub use signal::Signal;

use std::any::Any;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

// The effect-queue drain; both flush sites reach it via `flush_effects_and_notify`.
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

    /// Effects that need to run, drained front-to-back.
    ///
    /// FIFO, not LIFO: the queue order *is* the execution order, so the
    /// registration-order guarantee established when enqueuing survives the
    /// flush. See the module-level "Execution order" docs.
    pub(crate) pending_effects: VecDeque<ObserverId>,

    /// Set for O(1) duplicate check when enqueuing pending effects.
    ///
    /// Same fast integer hasher as the effect registry
    /// ([`ObserverIdBuildHasher`](effect::ObserverIdBuildHasher)): this set is
    /// probed once per enqueue (every observer of every signal write) and once
    /// per dequeue, which is at least as hot as the registry lookup that hasher
    /// was written for.
    pub(crate) pending_effects_set: HashSet<ObserverId, effect::ObserverIdBuildHasher>,

    /// Whether we're currently in a batch
    pub(crate) batching: bool,

    /// Counter for generating unique IDs
    next_id: usize,

    /// Stack of scopes that own newly created resources.
    ///
    /// Deliberately separate from `observer_stack`: that one answers "who
    /// subscribes to this read" and is suspended by [`untracked`]; this one
    /// answers "who owns this allocation" and is suspended by [`unowned`].
    ///
    /// Entries are [`Weak`](std::rc::Weak), never `Rc` — see [`Owner`].
    /// Module-private, so the private `scope::ScopeInner` can appear in its type.
    owner_stack: Vec<std::rc::Weak<scope::ScopeInner>>,

    /// Callbacks invoked when any signal changes (for UI re-render / dirty
    /// tracking). Multi-subscriber: each entry is `(subscription id, callback)`;
    /// a [`SignalChangeSubscription`] removes only its own entry on drop, so
    /// several consumers (e.g. concurrent embed `RinchContext`s plus a mounted
    /// shell/web root, issue #134) can each observe signal changes without
    /// evicting one another.
    pub(crate) on_signal_change: Vec<(u64, Rc<dyn Fn()>)>,

    /// Counter for [`SignalChangeSubscription`] ids.
    next_signal_change_sub: u64,

    /// Flag set when any signal changes (reset by `clear_signals_changed`)
    pub(crate) signals_changed: bool,
}

impl Runtime {
    fn new() -> Self {
        Self {
            observer_stack: Vec::new(),
            pending_effects: VecDeque::new(),
            pending_effects_set: HashSet::default(),
            batching: false,
            next_id: 0,
            owner_stack: Vec::new(),
            on_signal_change: Vec::new(),
            next_signal_change_sub: 0,
            signals_changed: false,
        }
    }

    pub(crate) fn next_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

/// A live signal-change subscription. Dropping it removes **only its own**
/// callback, leaving every other subscriber attached.
///
/// Returned by [`subscribe_signal_change`]. Hold it for as long as the callback
/// should fire (typically as a field of the consumer, e.g. `RinchContext`).
/// Thread-bound: the reactive runtime is thread-local, so the subscription must
/// be dropped on the thread that created it.
#[must_use = "dropping the subscription immediately detaches the callback"]
pub struct SignalChangeSubscription {
    id: u64,
    /// The runtime is thread-local — keep the guard `!Send`/`!Sync`.
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Drop for SignalChangeSubscription {
    fn drop(&mut self) {
        // `try_with`: TLS may already be torn down at thread exit; `try_borrow_mut`
        // guards a (pathological) drop from inside a signal-change callback.
        let _ = RUNTIME.try_with(|rt| {
            if let Ok(mut rt) = rt.try_borrow_mut() {
                rt.on_signal_change.retain(|(id, _)| *id != self.id);
            }
        });
    }
}

/// Register a callback invoked whenever any signal changes, alongside every
/// other subscriber. Returns a guard that detaches **only this** callback on
/// drop.
///
/// This is the multi-consumer form of [`set_on_signal_change`]: use it when the
/// consumer has a bounded lifetime (an embedded `RinchContext`, a plugin, a
/// debug overlay) so that creating or dropping one consumer never silences
/// another (issue #134).
///
/// Callbacks run on the signal's thread, after that change's effects have
/// flushed, in subscription order.
pub fn subscribe_signal_change(callback: impl Fn() + 'static) -> SignalChangeSubscription {
    let id = RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        let id = rt.next_signal_change_sub;
        rt.next_signal_change_sub += 1;
        rt.on_signal_change.push((id, Rc::new(callback)));
        id
    });
    SignalChangeSubscription {
        id,
        _not_send: std::marker::PhantomData,
    }
}

thread_local! {
    /// Backing subscription for the legacy [`set_on_signal_change`] /
    /// [`clear_on_signal_change`] API, preserving its historical
    /// single-occupancy (last-write-wins) semantics for the *legacy slot only* —
    /// guard-based [`subscribe_signal_change`] subscribers are unaffected.
    static LEGACY_SIGNAL_CHANGE_SUB: RefCell<Option<SignalChangeSubscription>> =
        const { RefCell::new(None) };
}

/// Register the process-wide UI re-render callback (legacy single-slot API).
///
/// This replaces any callback previously installed **through this function**;
/// subscribers registered with [`subscribe_signal_change`] are not affected.
/// Long-lived runtimes (the desktop shell, `rinch-web`) use this; consumers
/// with a bounded lifetime should prefer [`subscribe_signal_change`].
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
    let sub = subscribe_signal_change(callback);
    LEGACY_SIGNAL_CHANGE_SUB.with(|slot| {
        *slot.borrow_mut() = Some(sub);
    });
}

/// Clear the callback installed by [`set_on_signal_change`] (legacy API).
///
/// Subscribers registered with [`subscribe_signal_change`] are not affected.
pub fn clear_on_signal_change() {
    LEGACY_SIGNAL_CHANGE_SUB.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

/// Invoke every live signal-change callback, with no runtime borrow held while
/// each runs (a callback may itself touch signals). Iterates a snapshot, but
/// re-checks membership before each invocation so a subscription dropped by an
/// *earlier* callback in the same notification is honored immediately — keeping
/// the [`SignalChangeSubscription`] "dropping detaches the callback" contract
/// exact even mid-notification.
pub(crate) fn notify_signal_change() {
    let snapshot: Vec<(u64, Rc<dyn Fn()>)> =
        RUNTIME.with(|rt| rt.borrow().on_signal_change.clone());
    for (id, cb) in snapshot {
        let still_subscribed =
            RUNTIME.with(|rt| rt.borrow().on_signal_change.iter().any(|(i, _)| *i == id));
        if still_subscribed {
            cb();
        }
    }
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

/// Register `dispatcher`, but only if no dispatcher is registered yet.
///
/// [`set_cross_thread_dispatcher`] is last-wins, which is what a host owning the
/// process wants: the desktop shell's dispatcher also wakes its winit event
/// loop, and it must win. An embedded context arms cross-thread dispatch through
/// this door instead, so creating one inside a desktop app cannot displace the
/// runtime's waking dispatcher with a queue-only one. Either way both hosts push
/// onto the same [`queue_main_callback`] queue, so whichever dispatcher is
/// installed, every drain site sees every closure.
pub fn set_cross_thread_dispatcher_if_unset(dispatcher: fn(Box<dyn FnOnce() + Send>)) {
    let mut slot = CROSS_THREAD_DISPATCHER.lock().unwrap();
    if slot.is_none() {
        *slot = Some(dispatcher);
    }
}

/// Closures queued from a background thread, waiting to run on the main thread.
///
/// The queue lives here, next to the dispatcher it backs, because *every* host
/// needs the same one: the desktop shell drains it before each paint and on each
/// event-loop wake, the Android loop drains it once per frame, and an embedded
/// `RinchContext` drains it at the top of `update()`. It used to be a `static`
/// private to `shell/rinch_runtime.rs`, which is `desktop`-gated — so an
/// `embed`-only build had nowhere to put a cross-thread closure and
/// [`Signal::send`](crate::Signal::send) panicked in the one mode most likely to
/// want it (issue #172).
static MAIN_QUEUE: Mutex<Vec<Box<dyn FnOnce() + Send>>> = Mutex::new(Vec::new());

/// Push `f` onto the shared main-thread queue.
///
/// Returns `true` if the queue was empty before the push — a host's cue to wake
/// its event loop, which it then does once per batch rather than once per
/// closure. A host with nothing to wake (an embedded context, whose game loop is
/// already turning) ignores it.
pub fn queue_main_callback(f: Box<dyn FnOnce() + Send>) -> bool {
    let mut queue = MAIN_QUEUE.lock().unwrap();
    let was_empty = queue.is_empty();
    queue.push(f);
    was_empty
}

/// Run every queued main-thread callback. Call from the main thread only.
///
/// The queue is process-global, so a host that drains it runs the work queued
/// against every host on the thread. That is correct rather than merely
/// tolerable: the payload is a `Send` closure that writes its own signals, and
/// signals are thread-local — not per-document — so it does the same thing
/// whoever runs it. The document it ultimately touches is still repainted by its
/// own host, because a signal change notifies every subscriber (issue #134).
pub fn drain_main_callbacks() {
    let callbacks: Vec<Box<dyn FnOnce() + Send>> = MAIN_QUEUE.lock().unwrap().drain(..).collect();
    for callback in callbacks {
        callback();
    }
}

/// Drop every queued main-thread callback without running it.
///
/// For host shutdown only: a queued closure typically captures app state that is
/// about to be torn down, so running it there would be worse than losing it.
pub fn clear_main_callbacks() {
    MAIN_QUEUE.lock().unwrap().clear();
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
/// Panics if no dispatcher has been registered (i.e., rinch runtime not
/// initialized). Reaching this at all means [`register_main_thread`] was called,
/// which only a host does — so a host that registers the main thread without
/// also registering a dispatcher is strictly worse off than one that registers
/// neither, where [`is_main_thread`] answers `true` and `send()` degrades to a
/// direct `set()`. Registering both is the contract; the panic is what catches a
/// host that forgot (issue #172).
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

/// Run a closure on the main (UI) thread.
///
/// If called from the main thread, runs `f` immediately. Otherwise dispatches it
/// via the registered cross-thread dispatcher (installed by the rinch runtime) to
/// run on the next event-loop wake. Panics only if called from a background thread
/// with no dispatcher registered.
///
/// This is the transport half of the main-thread callback machinery; see
/// [`crate::main_thread`] for parking a `!Send` continuation and resuming it here.
pub fn run_on_main_thread(f: impl FnOnce() + Send + 'static) {
    if is_main_thread() {
        f();
    } else {
        dispatch_to_main_thread(Box::new(f));
    }
}

/// Unique identifier for an observer (effect or memo).
///
/// Allocated from a single monotonic counter ([`Runtime::next_id`]) and never
/// reused, so **ascending id is registration order** — which is what makes the
/// `BTreeSet` subscriber sets order observers correctly. Anything that starts
/// recycling ids breaks the execution-order contract, not just uniqueness.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) struct ObserverId(pub(crate) usize);

/// One subscription an observer currently holds: a slot in [`SIGNAL_STORE`] or
/// in [`MEMO_STORE`], named the way the `Copy` handles name it — index plus
/// generation.
///
/// Recorded on the observer's `EffectInner` as the subscription is taken out,
/// and walked back out to release it when the observer re-runs or is disposed
/// (issue #171). Without it, a signal that outlives its observers accumulates
/// dead `ObserverId`s forever, and every write to it queues each one for a
/// `run_effect` that finds nothing to run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DepKey {
    Signal { id: u32, generation: u32 },
    Memo { id: u32, generation: u32 },
}

impl DepKey {
    /// Remove `observer` from this dependency's subscriber set.
    ///
    /// **Generation-safe.** Both arms resolve the slot through the same
    /// generation-filtered lookup every other reader uses, so a slot that has
    /// since been freed and handed to a new signal or memo is a miss rather than
    /// a write into the new occupant's set. (Belt and braces: `ObserverId`s are
    /// monotonic and never reused — that is the execution-order contract of
    /// issue #154 — so even a generation-blind removal could only ever look for
    /// an id the new occupant's set does not contain.)
    pub(crate) fn unsubscribe(self, observer: ObserverId) {
        match self {
            DepKey::Signal { id, generation } => SIGNAL_STORE.with(|store| {
                if let Some(slot) = store.borrow_mut().get_slot_mut(id, generation) {
                    slot.subscribers.remove(&observer);
                }
            }),
            DepKey::Memo { id, generation } => {
                MEMO_STORE.with(|store| store.borrow().unsubscribe(id, generation, observer));
            }
        }
    }
}

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
    /// Observers to notify, ordered. `BTreeSet` (not `HashSet`) so iteration
    /// yields ascending `ObserverId` = registration order; see the module-level
    /// "Execution order" docs.
    pub(crate) subscribers: BTreeSet<ObserverId>,
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
            subscribers: BTreeSet::new(),
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

    /// Free a slot, **returning** its value rather than dropping it in place.
    ///
    /// The caller must drop the returned box after releasing the store borrow:
    /// a value whose `Drop` touches a signal would otherwise `BorrowMutError`
    /// (issue #141, SD4). Returns `None` if the slot was already freed or the
    /// generation does not match.
    fn free(&mut self, id: u32, generation: u32) -> Option<Box<dyn Any>> {
        let slot = self.slots.get_mut(id as usize)?;
        if !slot.as_ref().is_some_and(|s| s.generation == generation) {
            return None;
        }
        let taken = slot.take();
        self.free_list.push(id);
        taken.map(|s| s.value)
    }
}

/// Free a signal slot, **returning** its value for the caller to drop once every
/// borrow is released. See [`SignalStore::free`].
///
/// The disposal fixpoint parks the returned value and drops it at the very end,
/// so that a value's `Drop` sees a deterministic world rather than an arbitrary
/// prefix of its siblings.
pub(crate) fn free_signal(id: u32, generation: u32) -> Option<Box<dyn Any>> {
    SIGNAL_STORE.with(|store| store.borrow_mut().free(id, generation))
}

/// Free a signal slot and drop its value, standing in for scope disposal.
/// Test-only convenience over [`free_signal`].
#[cfg(test)]
pub(crate) fn free_signal_for_tests(id: u32, generation: u32) {
    // Dropped out here, after the store borrow is released.
    let _value = free_signal(id, generation);
}

// ============================================================================
// Memo Storage
// ============================================================================

thread_local! {
    static MEMO_STORE: RefCell<MemoStore> = RefCell::new(MemoStore::new());
}

struct MemoSlot {
    inner: Rc<dyn Any>, // Type-erased Rc<MemoInner<T>>
    /// The memo's dirty-marker effect, which holds the *second* strong
    /// reference to the same `MemoInner`. Recorded here because the slot is
    /// type-erased: freeing a memo has to remove the marker's `EFFECTS` entry
    /// too, and a type-erased caller cannot downcast to reach `MemoInner::id`.
    observer: ObserverId,
    /// The memo's subscriber set, shared with its `MemoInner`.
    ///
    /// Shared rather than owned for the same type-erasure reason as `observer`:
    /// unsubscribing a disposed observer (issue #171) starts from a [`DepKey`],
    /// which names a slot, and a type-erased caller cannot downcast an
    /// `Rc<dyn Any>` to `MemoInner<T>` to reach the set inside it.
    subscribers: Rc<RefCell<BTreeSet<ObserverId>>>,
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

    pub(crate) fn alloc(
        &mut self,
        inner: Rc<dyn Any>,
        observer: ObserverId,
        subscribers: Rc<RefCell<BTreeSet<ObserverId>>>,
    ) -> (u32, u32) {
        let generation = self.next_gen;
        self.next_gen = self.next_gen.wrapping_add(1);
        if self.next_gen == 0 {
            self.next_gen = 1;
        }

        let slot = MemoSlot {
            inner,
            observer,
            subscribers,
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

    pub(crate) fn get_inner(&self, id: u32, generation: u32) -> Option<Rc<dyn Any>> {
        self.slots
            .get(id as usize)?
            .as_ref()
            .filter(|s| s.generation == generation)
            .map(|s| Rc::clone(&s.inner))
    }

    /// Remove an observer from a memo's subscriber set, if this slot still
    /// holds the memo the caller recorded.
    ///
    /// Type-erased on purpose: the caller holds a [`DepKey`], not a `Memo<T>`,
    /// and an `Rc<dyn Any>` cannot be downcast without `T`. Hence the set being
    /// shared with the slot rather than living only inside `MemoInner`.
    pub(crate) fn unsubscribe(&self, id: u32, generation: u32, observer: ObserverId) {
        if let Some(slot) = self
            .slots
            .get(id as usize)
            .and_then(|s| s.as_ref())
            .filter(|s| s.generation == generation)
        {
            slot.subscribers.borrow_mut().remove(&observer);
        }
    }

    /// Free a slot, **returning** its `Rc` and the marker's [`ObserverId`]
    /// rather than dropping in place — same rule and same reason as
    /// [`SignalStore::free`]: a `MemoInner`'s cached value can own arbitrary
    /// user data whose `Drop` may touch the reactive stores.
    ///
    /// Freeing the slot alone does **not** release the memo. Use
    /// [`free_memo`], which also removes the marker effect's registry entry.
    pub(crate) fn free(&mut self, id: u32, generation: u32) -> Option<(Rc<dyn Any>, ObserverId)> {
        let slot = self.slots.get_mut(id as usize)?;
        if !slot.as_ref().is_some_and(|s| s.generation == generation) {
            return None;
        }
        let taken = slot.take();
        self.free_list.push(id);
        taken.map(|s| (s.inner, s.observer))
    }
}

/// Release a memo completely: its store slot **and** its dirty-marker effect.
///
/// `Memo::new` puts two strong `Rc<MemoInner>` references into two different
/// registries — one in `MEMO_STORE`, one captured by the marker closure in
/// `EFFECTS`. Dropping only the store slot frees nothing (the marker keeps the
/// cached value and the computation closure alive) and is actively harmful: the
/// marker stays subscribed to the memo's sources, so the next write to any of
/// them re-queues the memo's dependents, whose `Memo::get()` then panics on the
/// now-empty slot.
///
/// Both `Rc`s are dropped after every borrow is released, since the cached value
/// is arbitrary user data whose `Drop` may touch the reactive stores.
pub(crate) fn free_memo(id: u32, generation: u32) {
    let Some((inner, observer)) = MEMO_STORE.with(|store| store.borrow_mut().free(id, generation))
    else {
        return;
    };
    let marker = effect::EFFECTS.with(|effects| effects.borrow_mut().remove(&observer));
    if let Some(marker) = &marker {
        // Same reason `dispose_effect` sets it: removal alone only stops *future*
        // lookups, and an `Rc` handed out by an earlier one can still be in
        // flight. This is the flag that tells such a run it has been retired.
        marker.disposed.set(true);
        // And release what the memo's *computation* subscribed to: the marker is
        // the id a memo's sources hold, so leaving its deps recorded would strand
        // a dead `ObserverId` in every signal the computation read (issue #171).
        effect::unsubscribe_deps_of(marker, observer);
    }
    drop(marker);
    drop(inner);
}

// ============================================================================
// Batching
// ============================================================================

/// RAII guard for the batching flag.
///
/// Saves the previous flag value on construction and restores it on drop —
/// including while unwinding. A bare set/clear pair has two failure modes:
/// a panic between them leaks `batching = true`, after which every later
/// write queues observers that nothing ever flushes (a silent UI freeze,
/// issue #232); and a nested `batch()` clearing to `false` ends the *outer*
/// batch's transaction early. Restoring the saved value fixes both — only
/// the outermost guard restores `false`.
struct BatchGuard {
    prev: bool,
}

impl BatchGuard {
    fn raise() -> Self {
        let prev = RUNTIME.with(|rt| std::mem::replace(&mut rt.borrow_mut().batching, true));
        BatchGuard { prev }
    }
}

impl Drop for BatchGuard {
    fn drop(&mut self) {
        // `try_with`: TLS may already be torn down at thread exit.
        let _ = RUNTIME.try_with(|rt| {
            if let Ok(mut rt) = rt.try_borrow_mut() {
                rt.batching = self.prev;
            } else {
                // Unreachable today: if a caller held the borrow, `raise()`
                // would have panicked before a guard existed, and unwinding
                // releases inner `RefMut`s before this frame drops. But a
                // *silent* skip here would latch `batching = true` for the
                // rest of the thread's life — the #232 freeze this guard
                // exists to prevent, now unrecoverable because nothing
                // clears the flag unconditionally anymore. Fail loud.
                tracing::error!(
                    "BatchGuard could not restore the batching flag (runtime already \
                     borrowed); reactive updates may stop flushing"
                );
            }
        });
    }
}

/// Batch multiple signal updates to avoid redundant effect runs.
///
/// Effects will only run once after the batch completes, even if multiple
/// signals they depend on are updated.
///
/// Batches nest: a `batch()` inside another batch's *closure* joins the outer
/// transaction, and effects flush once, when the outermost batch exits. A
/// `batch()` entered during a *flush* — from inside an effect the outer batch
/// woke — is instead a fresh outermost batch (the outer flag has already been
/// restored by then), so it keeps the top-level contract: the flush completes
/// synchronously before `batch()` returns.
///
/// That flush-time rule is about flushes, not effect bodies in general: an
/// effect body that runs while a batch is still *open* — e.g. `Effect::new`'s
/// immediate first run inside a batch closure — is inside that batch, and a
/// `batch()` opened there joins the outer transaction. Code that pairs a
/// batch with an RAII window (raise flag → batch writes → drop flag) and
/// needs observers to run inside the window must account for this case.
///
/// Until the flush, nothing has run: inside the closure — including after a
/// *nested* `batch()` returns — effects have not executed, and [`Memo::get`]
/// still returns the pre-batch value (a memo is re-marked dirty by a queued
/// marker effect, which is itself deferred by the batch).
///
/// # Panics
///
/// A panic inside the closure propagates, and the batching flag is restored on
/// the way out. Nothing is flushed while unwinding — running arbitrary effect
/// code mid-panic would be worse than the panic itself — so the effects the
/// aborted batch queued stay pending and run at the next flush (the next
/// signal write outside a batch, or the next outermost batch exit). No such
/// flush is *scheduled*: if nothing ever writes again, the queued effects stay
/// pending and no re-render is requested for the writes that did land before
/// the panic.
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
    let guard = BatchGuard::raise();
    let outermost = !guard.prev;

    let result = f();

    // Restore the flag *before* flushing: `Signal::set` inside a flushed
    // effect must see `batching = false` again, and a `batch()` opened there
    // must count as a fresh outermost batch.
    drop(guard);

    if outermost {
        flush_effects_and_notify();
    }

    result
}

/// Run every pending effect, then the UI re-render callbacks.
///
/// The order is a contract: effects run BEFORE the callbacks, so fine-grained
/// updates are queued by the time a callback decides whether a full re-render
/// is needed. Both flush sites — `Signal::notify` (unbatched writes) and the
/// outermost [`batch`] exit — go through here, so the policy cannot drift
/// between them.
pub(crate) fn flush_effects_and_notify() {
    flush_effects();
    notify_signal_change();
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
///
/// # See also
///
/// [`unowned`] suspends the *owner* stack instead of the *observer* stack —
/// it changes who owns resources created inside `f`, not who subscribes to
/// signals read inside it. The two are independent, and picking the wrong one
/// fails silently.
pub fn untracked<R>(f: impl FnOnce() -> R) -> R {
    /// Pushes the popped observer back on drop — including while unwinding.
    ///
    /// A bare pop/push pair has the same failure mode `BatchGuard` closes for
    /// [`batch`] (#232): a panic in `f` that is caught upstream skips the
    /// restore, and because the enclosing `ObserverGuard`'s drop pops the top
    /// of the stack blindly, the missing entry makes it remove a *lower*
    /// frame's observer — an unrelated effect silently stops subscribing.
    struct RestoreObserver {
        observer: Option<ObserverId>,
    }

    impl Drop for RestoreObserver {
        fn drop(&mut self) {
            if let Some(obs) = self.observer.take() {
                // `try_with`: TLS may already be torn down at thread exit.
                let _ = RUNTIME.try_with(|rt| {
                    if let Ok(mut rt) = rt.try_borrow_mut() {
                        rt.observer_stack.push(obs);
                    } else {
                        tracing::error!(
                            "untracked() could not restore the observer (runtime already \
                             borrowed); an effect may silently stop subscribing"
                        );
                    }
                });
            }
        }
    }

    // Temporarily remove the current observer; the guard restores it on the
    // way out, panic or not.
    let _restore = RestoreObserver {
        observer: RUNTIME.with(|rt| rt.borrow_mut().observer_stack.pop()),
    };

    f()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    /// How many observers each ordering test registers.
    ///
    /// Large enough that hash-iteration order coinciding with registration
    /// order is a 1-in-40320 accident rather than a coin flip — these tests
    /// have to *fail* against the old `HashSet` + LIFO pair to be worth having.
    const OBSERVERS: usize = 8;

    /// Effects sharing a signal run in registration order (#154).
    #[test]
    fn same_signal_effects_run_in_registration_order() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let sig = Signal::new(0);

        let _effects: Vec<Effect> = (0..OBSERVERS)
            .map(|i| {
                let log = log.clone();
                Effect::new(move || {
                    sig.get();
                    log.borrow_mut().push(i);
                })
            })
            .collect();

        // Discard the runs that `Effect::new` performs at creation.
        log.borrow_mut().clear();
        sig.set(1);

        assert_eq!(
            *log.borrow(),
            (0..OBSERVERS).collect::<Vec<_>>(),
            "effects on one signal must run in the order they were created"
        );
    }

    /// The same guarantee for a memo's dependents: notification flows through
    /// `MemoInner::subscribers`, which is a separate set from the signal's.
    #[test]
    fn memo_dependents_run_in_registration_order() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let sig = Signal::new(0);
        let doubled = Memo::new(move || sig.get() * 2);

        let _effects: Vec<Effect> = (0..OBSERVERS)
            .map(|i| {
                let log = log.clone();
                Effect::new(move || {
                    doubled.get();
                    log.borrow_mut().push(i);
                })
            })
            .collect();

        log.borrow_mut().clear();
        sig.set(1);

        assert_eq!(
            *log.borrow(),
            (0..OBSERVERS).collect::<Vec<_>>(),
            "effects on one memo must run in the order they were created"
        );
    }

    /// The idiom the contract exists to protect: an effect registered *after*
    /// a tree of rendering effects observes their post-write state in the same
    /// synchronous flush ("run me last").
    ///
    /// Here `dom` stands in for the patched DOM — the measuring effect must
    /// read the width the rendering effect just wrote, not the previous one.
    #[test]
    fn an_effect_registered_last_observes_earlier_effects_writes() {
        let dom = Rc::new(RefCell::new(String::new()));
        let measured = Rc::new(RefCell::new(Vec::new()));
        let width = Signal::new(1);

        let render_dom = dom.clone();
        let _render = Effect::new(move || {
            *render_dom.borrow_mut() = "x".repeat(width.get());
        });

        let measure_dom = dom.clone();
        let measure_log = measured.clone();
        let _measure = Effect::new(move || {
            width.get();
            let len = measure_dom.borrow().len();
            measure_log.borrow_mut().push(len);
        });

        width.set(5);

        assert_eq!(
            *measured.borrow(),
            vec![1, 5],
            "the later-registered effect must observe the post-write state"
        );
    }

    /// A signal written *during* a flush queues its observers behind the rest
    /// of that flush (FIFO), rather than jumping ahead of already-queued work.
    #[test]
    fn effects_queued_during_a_flush_run_after_the_already_queued_ones() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let trigger = Signal::new(0);
        let cascade = Signal::new(0);

        let writer_log = log.clone();
        let _writer = Effect::new(move || {
            writer_log.borrow_mut().push("writer");
            cascade.set(trigger.get());
        });

        let other_log = log.clone();
        let _other = Effect::new(move || {
            trigger.get();
            other_log.borrow_mut().push("other");
        });

        let cascaded_log = log.clone();
        let _cascaded = Effect::new(move || {
            cascade.get();
            cascaded_log.borrow_mut().push("cascaded");
        });

        log.borrow_mut().clear();
        trigger.set(1);

        assert_eq!(
            *log.borrow(),
            vec!["writer", "other", "cascaded"],
            "the cascade must not preempt an effect already queued for this flush"
        );
    }

    /// A panic inside an effect must not strand its `ObserverId` on the
    /// observer stack (issue #141).
    ///
    /// The stack is what `Signal::track` reads to decide who subscribes, so a
    /// stranded id silently subscribes itself to *every* signal read for the
    /// rest of the thread's life — and that observer's slot is gone, so the
    /// symptom is unbounded queue churn far from the panic that caused it.
    #[test]
    fn a_panicking_effect_does_not_strand_its_observer() {
        let depth_before = RUNTIME.with(|rt| rt.borrow().observer_stack.len());

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Effect::new(|| panic!("boom"));
        }));
        assert!(result.is_err(), "the effect body must have panicked");

        let depth_after = RUNTIME.with(|rt| rt.borrow().observer_stack.len());
        assert_eq!(
            depth_after, depth_before,
            "the observer stack must unwind with the panic"
        );

        // Positive control: the runtime is still usable afterwards, and a
        // signal read at top level (outside any effect) does not wake anyone.
        // With a stranded observer this signal would acquire a subscriber whose
        // slot is gone, and every later `set` would queue it forever.
        let sig = Signal::new(0);
        let runs = Rc::new(Cell::new(0));
        let r = runs.clone();
        let _e = Effect::new(move || {
            sig.get();
            r.set(r.get() + 1);
        });
        assert_eq!(runs.get(), 1, "the new effect runs once at creation");

        sig.get(); // top-level read: must not subscribe anything
        sig.set(1);
        assert_eq!(
            runs.get(),
            2,
            "exactly the one live subscriber re-runs, no more"
        );
    }

    /// The same guarantee for a memo, whose user computation runs lazily in
    /// `Memo::get` rather than in its dirty-marker effect.
    #[test]
    fn a_panicking_memo_computation_does_not_strand_its_observer() {
        let depth_before = RUNTIME.with(|rt| rt.borrow().observer_stack.len());

        let memo = Memo::new(|| -> i32 { panic!("boom") });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| memo.get()));
        assert!(result.is_err(), "the memo computation must have panicked");

        let depth_after = RUNTIME.with(|rt| rt.borrow().observer_stack.len());
        assert_eq!(
            depth_after, depth_before,
            "the observer stack must unwind with the panic"
        );
    }

    #[test]
    fn signal_change_subscriptions_are_independent() {
        let a = Rc::new(Cell::new(0));
        let b = Rc::new(Cell::new(0));

        let a_hits = a.clone();
        let sub_a = subscribe_signal_change(move || a_hits.set(a_hits.get() + 1));
        let b_hits = b.clone();
        let sub_b = subscribe_signal_change(move || b_hits.set(b_hits.get() + 1));

        // Both subscribers see every change.
        let sig = Signal::new(0);
        sig.set(1);
        assert_eq!(a.get(), 1, "first subscriber fires");
        assert_eq!(b.get(), 1, "second subscriber fires alongside the first");

        // Dropping one guard detaches only its own callback (#134).
        drop(sub_b);
        sig.set(2);
        assert_eq!(a.get(), 2, "surviving subscriber still fires");
        assert_eq!(b.get(), 1, "dropped subscriber no longer fires");

        drop(sub_a);
        sig.set(3);
        assert_eq!(a.get(), 2, "dropped subscriber no longer fires");
    }

    #[test]
    fn subscription_dropped_mid_notification_does_not_fire() {
        // Subscriber A (registered first) drops B's guard from inside its own
        // callback — B must NOT fire for that same notification: the guard's
        // contract is that dropping detaches immediately.
        let b_hits = Rc::new(Cell::new(0));
        let b_guard: Rc<RefCell<Option<SignalChangeSubscription>>> = Rc::new(RefCell::new(None));

        let b_guard_for_a = b_guard.clone();
        let _sub_a = subscribe_signal_change(move || {
            *b_guard_for_a.borrow_mut() = None; // drop B mid-notification
        });
        let b_hits_clone = b_hits.clone();
        *b_guard.borrow_mut() = Some(subscribe_signal_change(move || {
            b_hits_clone.set(b_hits_clone.get() + 1);
        }));

        let sig = Signal::new(0);
        sig.set(1);
        assert_eq!(
            b_hits.get(),
            0,
            "a subscription dropped by an earlier callback in the same \
             notification must not fire"
        );
    }

    #[test]
    fn legacy_slot_is_last_write_wins_but_spares_guard_subscribers() {
        let legacy = Rc::new(Cell::new(0));
        let guard = Rc::new(Cell::new(0));

        let guard_hits = guard.clone();
        let _sub = subscribe_signal_change(move || guard_hits.set(guard_hits.get() + 1));

        // Legacy slot: second set_on_signal_change replaces the first…
        let first = Rc::new(Cell::new(0));
        let first_hits = first.clone();
        set_on_signal_change(move || first_hits.set(first_hits.get() + 1));
        let legacy_hits = legacy.clone();
        set_on_signal_change(move || legacy_hits.set(legacy_hits.get() + 1));

        let sig = Signal::new(0);
        sig.set(1);
        assert_eq!(first.get(), 0, "evicted legacy callback does not fire");
        assert_eq!(legacy.get(), 1, "current legacy callback fires");
        assert_eq!(
            guard.get(),
            1,
            "guard-based subscriber unaffected by legacy churn"
        );

        // …and clear_on_signal_change removes only the legacy slot.
        clear_on_signal_change();
        sig.set(2);
        assert_eq!(legacy.get(), 1, "cleared legacy callback does not fire");
        assert_eq!(guard.get(), 2, "guard-based subscriber survives the clear");
    }

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

    /// A panic inside the closure must not leak `batching = true` (#232).
    ///
    /// A leaked flag makes every later write queue observers that nothing ever
    /// flushes — the UI silently freezes, which is strictly worse than the
    /// panic itself. Nothing flushes *during* the unwind either: the queued
    /// effects stay pending and run at the next flush. The panic this test
    /// provokes is deliberate — its message on stderr is expected output.
    #[test]
    fn a_panicking_batch_does_not_leak_the_batching_flag() {
        let count = Signal::new(0);
        let runs = Rc::new(Cell::new(0));

        let r = runs.clone();
        let _effect = Effect::new(move || {
            count.get();
            r.set(r.get() + 1);
        });
        assert_eq!(runs.get(), 1, "the effect runs once at creation");

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            batch(|| {
                count.set(1);
                panic!("boom");
            });
        }));
        assert!(result.is_err(), "the batch closure must have panicked");
        assert_eq!(runs.get(), 1, "nothing flushes while unwinding");
        assert!(
            !RUNTIME.with(|rt| rt.borrow().batching),
            "the flag must be restored by the unwind"
        );

        // The observer the aborted batch queued is not lost: the next
        // unbatched write flushes it along with its own.
        count.set(2);
        assert_eq!(runs.get(), 2, "later writes must still flush");
        assert_eq!(count.get(), 2);
    }

    /// Nested batches are one transaction (#232): nothing flushes until the
    /// outermost batch exits — not at the inner exit, and not on a write made
    /// *after* the inner batch (which is what would leak if the inner guard
    /// cleared the flag instead of restoring it). At the single flush,
    /// observers of the *same* signal run in registration order (#154's
    /// BTreeSet path, exercised through nesting: `first` before `third`),
    /// while across different signals the order is the FIFO enqueue order —
    /// the order the signals were first written.
    #[test]
    fn nested_batches_flush_once_at_the_outermost_exit() {
        let a = Signal::new(0);
        let b = Signal::new(0);

        let log = Rc::new(RefCell::new(Vec::new()));

        let l = Rc::clone(&log);
        let _first = Effect::new(move || {
            a.get();
            l.borrow_mut().push("first");
        });
        let l = Rc::clone(&log);
        let _second = Effect::new(move || {
            b.get();
            l.borrow_mut().push("second");
        });
        let l = Rc::clone(&log);
        let _third = Effect::new(move || {
            a.get();
            l.borrow_mut().push("third");
        });
        log.borrow_mut().clear(); // discard the creation runs

        let l = Rc::clone(&log);
        batch(|| {
            a.set(1);
            batch(|| b.set(1));
            // This write is the probe for a guard that *clears* rather than
            // restores: with the outer flag gone it would flush right here.
            a.set(2);
            assert!(
                l.borrow().is_empty(),
                "the inner batch must not end the outer transaction"
            );
        });

        assert_eq!(
            *log.borrow(),
            vec!["first", "third", "second"],
            "one flush at the outermost exit: same-signal observers in \
             registration order, signals in first-write order"
        );
    }

    /// A panicking *inner* batch restores `batching` to `true`, not `false`:
    /// the outer transaction survives a caught panic, keeps batching writes
    /// made after it, and still flushes exactly once at the outermost exit.
    /// The panic this test provokes is deliberate — its message on stderr is
    /// expected output.
    #[test]
    fn a_caught_panic_in_an_inner_batch_leaves_the_outer_transaction_open() {
        let a = Signal::new(0);
        let runs = Rc::new(Cell::new(0));

        let r = runs.clone();
        let _effect = Effect::new(move || {
            a.get();
            r.set(r.get() + 1);
        });
        assert_eq!(runs.get(), 1, "the effect runs once at creation");

        batch(|| {
            a.set(1);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                batch(|| {
                    a.set(2);
                    panic!("inner boom");
                });
            }));
            assert!(result.is_err(), "the inner closure must have panicked");
            // The inner unwind restored `true`: this write still batches
            // instead of flushing mid-transaction.
            a.set(3);
            assert_eq!(runs.get(), 1, "the outer transaction must still be open");
        });

        assert_eq!(runs.get(), 2, "one flush at the outermost exit");
        assert_eq!(a.get(), 3);
    }

    /// A batch opened *during* another batch's flush — from inside an effect —
    /// is a fresh outermost batch and flushes synchronously before it returns.
    ///
    /// ColorPicker's `value_fn` echo path relies on this composition: the
    /// batched writes its `ApplyGuard` spans must flush inside the guard's
    /// window, even though that whole apply runs from an effect another
    /// batch woke.
    #[test]
    fn a_batch_inside_a_flush_still_flushes_before_returning() {
        let outer_sig = Signal::new(0);
        let inner_sig = Signal::new(0);

        let inner_runs = Rc::new(Cell::new(0));
        let r = Rc::clone(&inner_runs);
        let _inner_effect = Effect::new(move || {
            inner_sig.get();
            r.set(r.get() + 1);
        });

        // What `inner_runs` read as, right after the nested batch returned.
        let seen_after_nested_batch = Rc::new(Cell::new(0));
        let seen = Rc::clone(&seen_after_nested_batch);
        let ir = Rc::clone(&inner_runs);
        let _outer_effect = Effect::new(move || {
            if outer_sig.get() == 0 {
                return; // skip the creation run
            }
            batch(|| inner_sig.set(7));
            seen.set(ir.get());
        });

        let runs_before = inner_runs.get();
        batch(|| outer_sig.set(1));
        assert_eq!(
            seen_after_nested_batch.get(),
            runs_before + 1,
            "the nested batch must have flushed inside the outer effect's body"
        );
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

    /// An effect re-runs under the scope that *created* it, not under whatever
    /// happened to be rendering when the flush fired (issue #141).
    ///
    /// `flush_effects` is reached from arbitrary stacks — an event handler, a
    /// timer, the cross-thread drain — so without the restore in `run_effect`
    /// the third run below would attribute its signal to `b`.
    #[test]
    fn an_effect_reruns_under_its_creation_time_owner() {
        let a = Scope::new();
        let b = Scope::new();
        let trigger = Signal::new(0);
        let seen: Rc<RefCell<Vec<Option<Owner>>>> = Rc::new(RefCell::new(Vec::new()));

        let log = seen.clone();
        let effect = a.run(|| {
            Effect::new(move || {
                trigger.get();
                Signal::new(0);
                log.borrow_mut().push(current_owner());
            })
        });
        a.add_effect(effect);

        trigger.set(1); // re-run with an empty owner stack
        b.run(|| trigger.set(2)); // re-run with an unrelated owner ambient

        let seen = seen.borrow();
        assert_eq!(seen.len(), 3, "creation run plus two re-runs");
        for (i, owner) in seen.iter().enumerate() {
            assert_eq!(
                *owner,
                Some(a.owner()),
                "run {i} must observe the creation-time owner"
            );
        }
        assert_eq!(
            b.owned_counts().signals,
            0,
            "the ambient owner at flush time must not capture the effect's work"
        );
        assert_eq!(a.owned_counts().signals, 3);
    }

    /// A memo's user computation runs lazily, in the *reader's* frame, so it
    /// must restore its creation-time owner too — the ownership analogue of the
    /// context-root bug `MemoInner::root` exists to close.
    #[test]
    fn a_memo_recomputes_under_its_creation_time_owner() {
        let a = Scope::new();
        let b = Scope::new();
        let source = Signal::new(1);

        let memo = a.run(|| {
            Memo::new(move || {
                Signal::new(0);
                source.get() * 2
            })
        });
        assert_eq!(
            a.owned_counts().signals,
            0,
            "a memo is lazy: nothing has computed yet"
        );

        // First read happens from inside a DIFFERENT scope.
        assert_eq!(b.run(|| memo.get()), 2);

        assert_eq!(
            b.owned_counts().signals,
            0,
            "the reading scope must not capture the memo's computation"
        );
        assert_eq!(a.owned_counts().signals, 1);
    }

    /// The owner guard pops while unwinding, and — critically — does not abort
    /// the process doing so. A panic raised inside a `Drop` during unwind is a
    /// hard abort, which would take down the whole test binary rather than fail
    /// one test.
    #[test]
    fn an_owner_guard_pops_while_unwinding() {
        let scope = Scope::new();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scope.run(|| panic!("boom"));
        }));
        assert!(result.is_err(), "the body must have panicked");

        assert!(
            current_owner().is_none(),
            "the guard must pop while unwinding"
        );
        Signal::new(0);
        assert_eq!(
            scope.owned_counts().signals,
            0,
            "a stranded owner would capture every later allocation"
        );
    }

    /// Nested guards restore by depth, so an inner scope never clobbers the
    /// outer one's entry.
    #[test]
    fn nested_owner_guards_restore_by_depth() {
        let a = Scope::new();
        let b = Scope::new();

        a.run(|| {
            let outer = current_owner().expect("a is ambient");
            b.run(|| {
                let inner = current_owner().expect("b is ambient");
                assert_ne!(inner, outer, "the inner scope is a distinct owner");
            });
            assert_eq!(current_owner(), Some(outer), "the outer owner is restored");
        });

        assert!(current_owner().is_none(), "the stack drains completely");
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
