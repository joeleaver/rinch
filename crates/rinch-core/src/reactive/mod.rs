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
//!    ahead of it — except the running effect itself, which a write flushed
//!    during its own body skips rather than queues; see [`Effect`], #343.)
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

#[cfg(test)]
mod batch_tests;
mod batching;
mod bounds;
mod caller_owner;
#[cfg(test)]
mod caller_owner_tests;
mod counters;
#[cfg(test)]
mod drain_batching_tests;
mod effect;
mod main_thread;
mod memo;
#[cfg(test)]
mod memo_cutoff_tests;
#[cfg(test)]
mod notify_tests;
mod poll;
pub(crate) mod restore;
#[cfg(test)]
mod runtime_tests;
mod scope;
pub(crate) use scope::disposal_available;
#[cfg(test)]
mod scope_tests;
mod scoped;
mod signal;
mod store;
mod tracking;

/// Ambient-owner hooks for the rest of the crate: `crate::events` attributes
/// handlers to the scope currently rendering, and `crate::context` ties a
/// store/context entry to the scope that created it (issue #141).
pub use bounds::{
    ElementBounds, register_bounds_signal, registered_bounds_nodes, update_bounds_signals,
};
pub use caller_owner::{CallerHandler, CreatorOwner};
pub use effect::Effect;
pub use memo::Memo;
pub use poll::{
    PollRate, drain_polls, next_poll_due, poll_signal, polls_want_every_frame, set_poll_waker,
};
pub use scope::{
    CleanupHandle, OwnedCounts, Owner, OwnerGuard, Scope, current_owner, on_cleanup,
    on_cleanup_removable, unowned,
};
pub(crate) use scope::{on_cleanup_for_ambient_owner, record_handler};
pub use scoped::{
    DocScopedSlotMap, clear_doc_scoped_slot, clear_scoped_slot, install_doc_scoped_slot,
    install_scoped_slot, read_doc_scoped_slot, read_scoped_slot,
};
pub use signal::Signal;
pub use tracking::{TrackingSuspended, suspend_tracking, untracked, untracked_handler};

use batching::HOST_CHANGE_OWED;
pub use batching::{
    EffectFlushSuppressed, batch, flush_pending_effects, note_host_change, suppress_effect_flush,
};
pub(crate) use batching::{ReactiveDepthGuard, flush_effects_and_notify};
pub use counters::{
    ReactiveCounters, count_doc_reactive_work, counted_doc_count, doc_reactive_counters,
    reactive_counters, unattributed_reactive_counters,
};
pub(crate) use counters::{count_effect_run, count_signal_notify};
use main_thread::is_host_tag;
pub(crate) use main_thread::{
    WriteRoute, WriteThread, dispatch_main_callback_for, dispatch_to_main_thread_for, route_write,
    signal_owner, write_thread,
};
pub use main_thread::{
    clear_main_callbacks, dispatch_main_callback, drain_main_callbacks, is_main_thread,
    main_callbacks_pending, mark_host_thread, queue_main_callback, register_main_thread,
    run_on_main_thread, set_cross_thread_dispatcher, set_cross_thread_dispatcher_if_unset,
};
#[cfg(test)]
pub(crate) use store::free_signal_for_tests;
use store::{MEMO_STORE, SIGNAL_STORE};
pub(crate) use store::{free_memo, free_signal, mark_memos_stale};

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::rc::Rc;

// ============================================================================
// Runtime Context
// ============================================================================

// Global runtime state for tracking reactive subscriptions.
//
// The runtime maintains:
// - The current observer (the effect/memo run in progress) and the
//   suspensions in force (`tracking`)
// - A queue of pending effects to run
// - Batching state
thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::new(Runtime::new());
}

pub(crate) struct Runtime {
    /// The observer a signal read subscribes right now: the effect run or memo
    /// recompute in progress, or none. One slot, not a stack: a run nested
    /// inside another hides the outer one entirely (see [`tracking`], #3047).
    pub(crate) observer: Option<ObserverId>,

    /// The `untracked` / `suspend_tracking` suspensions in force, per run; a
    /// read subscribes [`observer`](Self::observer) only while there are none
    /// in the current run (see [`tracking::Suspensions`]).
    pub(crate) suspensions: tracking::Suspensions,

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

    /// The pending observers that were queued by a **signal** they read — as
    /// opposed to being woken through a memo. These run unconditionally; any
    /// other pending observer is a "maybe" that `flush_effects` runs only if a
    /// memo it read moved to a new version (the memo equality cut-off).
    pub(crate) definite_effects: HashSet<ObserverId, effect::ObserverIdBuildHasher>,

    /// Whether we're currently in a batch
    pub(crate) batching: bool,

    /// Counter for generating unique IDs
    next_id: usize,

    /// Stack of scopes that own newly created resources.
    ///
    /// Deliberately separate from `observer`: that one answers "who
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
            observer: None,
            suspensions: tracking::Suspensions::default(),
            pending_effects: VecDeque::new(),
            pending_effects_set: HashSet::default(),
            definite_effects: HashSet::default(),
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
    // Every notification tells every host, so no host change is owed after it.
    HOST_CHANGE_OWED.with(|c| c.set(false));
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
/// Rinch's own runtimes no longer read it: whether a wake draws a frame is
/// asked of the document, since a signal that changed may have changed
/// nothing on screen (#2608). Kept as public API for hosts and tools that
/// track "did this handler write state".
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
    Signal {
        id: u32,
        generation: u32,
    },
    /// `seen` is the memo's version as of the read that took the
    /// subscription out — what `flush_effects` compares against to decide
    /// whether an observer woken through this memo really has to run.
    Memo {
        id: u32,
        generation: u32,
        seen: u64,
    },
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
                if let Some(inner) = store.borrow().get_inner(id, generation) {
                    inner.subscribers.borrow_mut().remove(&observer);
                }
            }),
            DepKey::Memo { id, generation, .. } => {
                MEMO_STORE.with(|store| store.borrow().unsubscribe(id, generation, observer));
            }
        }
    }
}

// ============================================================================
// Utility functions
// ============================================================================

/// Create a derived signal from a computation.
///
/// This is a convenience function that creates a memo and returns it
/// as a signal-like value.
pub fn derived<T: Clone + PartialEq + 'static>(f: impl Fn() -> T + 'static) -> Memo<T> {
    Memo::new(f)
}
