//! Effect: a side-effect that re-runs when its dependencies change.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::Rc;

use super::{DepKey, ObserverId, RUNTIME};

thread_local! {
    /// Storage for all effects, needed because effects reference themselves: an
    /// effect is queued and run by *id*, so its closure has to live somewhere
    /// other than the [`Effect`] handle, which the caller is free to drop.
    pub(super) static EFFECTS: RefCell<EffectRegistry> =
        const { RefCell::new(HashMap::with_hasher(BuildHasherDefault::new())) };
}

/// The effect registry: [`ObserverId`] → effect.
///
/// # Why a map and not an id-indexed vec
///
/// `ObserverId`s are monotonic and never reused (see [`ObserverId`]), so a vec
/// indexed by id can only ever grow to the number of observers the thread has
/// *ever* created. Disposal emptied the slot but never removed it, so every
/// mount/unmount cycle left 8 bytes of spine behind forever — the last of the
/// five leaks in issue #141.
///
/// Reusing ids to keep the vec dense is not an option: ascending `ObserverId`
/// **is** registration order, which is what makes the `BTreeSet` subscriber
/// sets order observers correctly (issue #154). Recycling ids would silently
/// break execution order, not just uniqueness. So the id stays monotonic and
/// the container becomes sparse.
///
/// Nothing iterates this map — every access site is a point lookup by id — so
/// its unordered iteration cannot leak into that ordering contract.
pub(super) type EffectRegistry = HashMap<ObserverId, Rc<EffectInner>, ObserverIdBuildHasher>;

/// The `BuildHasher` for every [`ObserverId`]-keyed container on the reactive
/// hot path — the [`EffectRegistry`] here and `Runtime::pending_effects_set`,
/// which is probed once per enqueue and once per dequeue.
pub(crate) type ObserverIdBuildHasher = BuildHasherDefault<ObserverIdHasher>;

/// Hasher for [`ObserverId`] keys: one multiply, no SipHash.
///
/// `run_effect` does one registry lookup per observer drained from the pending
/// queue, and that count is dominated by **misses** — a signal's subscriber set
/// still holds the ids of effects that have since been disposed (issue #171),
/// so a long-lived signal accumulates dead subscribers and every write probes
/// the registry once for each. std's SipHash costs ~6x an integer hash on that
/// path, which would make an existing pathology measurably worse for no benefit
/// on a key that is already a unique integer.
///
/// The multiplier is the golden-ratio constant, and it is load-bearing: a plain
/// identity hash is pathological here, because hashbrown derives its control
/// byte from the *top* 7 bits and monotonically allocated ids would all land in
/// one control group. Multiplying distributes the low bits upward.
///
/// Every `write_*` **folds** into the running state rather than replacing it, so
/// the hasher stays correct if a key type ever writes more than once — a
/// two-field key whose last field matched would otherwise collide outright. The
/// fold is free for the single-field case: the state starts at `0`, and
/// `(0 ^ n) * K == n * K`.
#[derive(Default)]
pub(crate) struct ObserverIdHasher(u64);

impl ObserverIdHasher {
    /// Fold one word into the state. See the type's docs for why it folds.
    #[inline]
    fn fold(&mut self, n: u64) {
        self.0 = (self.0 ^ n).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

impl Hasher for ObserverIdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        // Never called for `ObserverId`, whose derived `Hash` forwards to
        // `write_usize`. Implemented anyway so the hasher is total.
        for &b in bytes {
            self.fold(u64::from(b));
        }
    }

    fn write_usize(&mut self, n: usize) {
        self.fold(n as u64);
    }
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
///
/// # An effect does not wake itself
///
/// A write that reaches the effect *while its own body is running* is dropped
/// for that effect, not queued (issue #343). A write made with no batch open
/// flushes at once — and every run a flush makes is in that state — so the
/// flush reaches this effect one frame down its own stack and skips it. The
/// write lands and every other observer runs; this effect keeps what it
/// computed and sees the value on its next run for another reason. That
/// includes a write made by code the body calls, most often an
/// [`on_cleanup`](super::on_cleanup) of a scope the body disposes.
///
/// It is deliberate: queued instead, an effect that disposes a scope on every
/// run whose cleanup writes a signal it reads would re-run without end.
///
/// A body running while a batch is **open** — [`Effect::new`] or
/// [`Effect::run`] called inside [`batch`](super::batch), which includes every
/// event handler — queues its writes instead, and the effect does run again
/// for its own write when the batch flushes.
pub struct Effect {
    id: ObserverId,
}

pub(super) struct EffectInner {
    pub(super) f: RefCell<Box<dyn FnMut()>>,
    pub(super) disposed: Cell<bool>,
    /// The context root current when this effect was created; re-entered on
    /// every run so `use_context`/`use_store` resolve the same namespace as at
    /// build time (issue #136). `0` = the thread-global fallback root.
    pub(super) root: u64,
    /// The document current when this effect was created
    /// ([`current_dispatching_doc`](crate::context::current_dispatching_doc)),
    /// re-entered as the dispatching-document marker on every run (issue #295).
    ///
    /// The effect queue is thread-global: an effect owned by one document is
    /// flushed wherever the write that woke it happened, and with two
    /// documents on one thread (an app and its DevTools panel, two embedded
    /// `RinchContext`s) that is routinely inside the *other* document's
    /// `handle_event`. Everything the body does that reads the marker — arming
    /// a `Drag`, registering a keyboard/paste/selection interceptor, asking
    /// `Drag::is_active` — must be attributed to the document the effect
    /// belongs to, not to whichever one was dispatching. Raw `doc_key`: `0`
    /// (created outside any document — `main`, a timer armed there, rinch-web) re-enters
    /// "no document", which every consumer reads as "nobody's" — never a
    /// borrowed document.
    pub(super) doc: u64,
    /// The scope that owned this effect when it was created, re-entered on every
    /// run so resources the body creates are attributed to the component that
    /// built it rather than to whatever happened to be rendering when the flush
    /// fired (issue #141). Weak by construction — see [`Owner`](super::Owner);
    /// a strong reference here would make every scope immortal, since `EFFECTS`
    /// holds the `Rc<EffectInner>` until `Scope::dispose` removes it.
    pub(super) owner: super::Owner,
    /// The subscriptions this observer holds right now, in the order it took
    /// them out during its current run.
    ///
    /// One entry per subscriber set this observer's id sits in: pushed when a
    /// read actually *inserts* it, drained back out when the observer re-runs or
    /// is disposed. Keeping the two sides in step is what stops a signal that
    /// outlives its observers accumulating dead ids (issue #171); clearing it
    /// per run is what keeps the vec bounded by the observer's live dependency
    /// count rather than by everything it has ever read.
    pub(super) deps: RefCell<Vec<DepKey>>,
    /// Whether a *run* of this observer is what re-reads its dependencies.
    ///
    /// True for an effect: the body is what tracks, so each run replaces the dep
    /// set wholesale. False for a memo's dirty-marker, whose body only flips a
    /// flag and queues the memo's dependents — a memo's dependencies are read by
    /// the lazy recompute in `Memo::try_get`, which pushes this same id and does
    /// the replacing there. Releasing them on a marker run would unsubscribe the
    /// memo from its own sources, and a dirty memo nobody happens to read would
    /// then stay disconnected from them.
    pub(super) body_tracks_deps: bool,
    /// `Some` for a memo's dirty-marker: the memo it stands for. Read by the
    /// eager staleness walk in `Signal::notify` (which marks the memo stale the
    /// moment a source is written) and by `flush_effects` (which skips a
    /// marker whose memo nobody reads). `None` for an ordinary effect.
    pub(super) memo: Option<Rc<dyn super::memo::MemoNode>>,
}

impl Effect {
    /// Create a new effect that runs immediately and re-runs when dependencies change.
    pub fn new<F: FnMut() + 'static>(f: F) -> Self {
        let id = RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            ObserverId(rt.next_id())
        });

        let (root, doc) = crate::context::current_reactive_frame();
        let inner = Rc::new(EffectInner {
            f: RefCell::new(Box::new(f)),
            disposed: Cell::new(false),
            root,
            doc,
            owner: super::Owner::current(),
            deps: RefCell::new(Vec::new()),
            body_tracks_deps: true,
            memo: None,
        });

        // Store the effect
        register(id, Rc::clone(&inner));

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

        let (root, doc) = crate::context::current_reactive_frame();
        let inner = Rc::new(EffectInner {
            f: RefCell::new(Box::new(f)),
            disposed: Cell::new(false),
            root,
            doc,
            owner: super::Owner::current(),
            deps: RefCell::new(Vec::new()),
            body_tracks_deps: true,
            memo: None,
        });

        register(id, inner);

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
    /// Also removes it from the global `EFFECTS` registry, allowing the
    /// `Rc<EffectInner>` — and everything its closure captures — to be
    /// reclaimed.
    pub fn dispose(&self) {
        dispose_effect(self.id);
    }
}

/// Insert an effect into the registry under its id.
///
/// The single registration path: both [`Effect`] constructors and `Memo`'s
/// dirty-marker go through here, so the borrow discipline below is stated once
/// instead of being re-derived at three call sites.
///
/// The displaced entry — always `None`, since ids are never reused — comes back
/// out of the closure rather than dropping inside it, for the same reason
/// [`dispose_effect`] moves the `Rc` out: dropping an `EffectInner` under the
/// `EFFECTS` borrow can re-enter the registry. The `debug_assert` is the cheap
/// tripwire for the only way that value could ever be `Some`: someone starting
/// to recycle `ObserverId`s, which would break the execution-order contract
/// silently and everywhere else.
pub(super) fn register(id: ObserverId, inner: Rc<EffectInner>) {
    let displaced = EFFECTS.with(|effects| effects.borrow_mut().insert(id, inner));
    debug_assert!(
        displaced.is_none(),
        "ObserverId {} was reused — ascending id is the registration-order contract (issue #154)",
        id.0
    );
    drop(displaced);
}

/// Record that `observer` has just subscribed to `dep`.
///
/// Called only from a read that actually inserted the id into a subscriber set,
/// so the list holds no duplicates within a run and the registry is probed once
/// per dependency per run rather than once per read.
///
/// The push happens under the `EFFECTS` borrow deliberately: a [`DepKey`] is
/// `Copy` and displaces nothing, so this cannot run user code — the borrow
/// discipline the rest of this module keeps has nothing to trip over here.
pub(super) fn record_dep(observer: ObserverId, dep: DepKey) {
    EFFECTS.with(|effects| {
        if let Some(inner) = effects.borrow().get(&observer) {
            inner.deps.borrow_mut().push(dep);
        }
    });
}

/// Release every subscription `observer` currently holds.
///
/// `drain` rather than `take`, so the buffer stays with the observer and a
/// re-running effect re-records its deps without reallocating. The `deps` borrow
/// is held across the removals, which touch only the signal and memo stores —
/// no user code runs under it, so nothing can re-enter this `RefCell`.
pub(super) fn unsubscribe_deps_of(inner: &EffectInner, observer: ObserverId) {
    for dep in inner.deps.borrow_mut().drain(..) {
        dep.unsubscribe(observer);
    }
}

/// Release every subscription the observer registered under `id` holds.
///
/// The `Rc` is cloned out of the registry first: the removals borrow
/// `SIGNAL_STORE`/`MEMO_STORE` mutably, and nothing should reach into another
/// reactive store while `EFFECTS` is borrowed.
pub(super) fn unsubscribe_deps(id: ObserverId) {
    let Some(inner) = EFFECTS.with(|effects| effects.borrow().get(&id).cloned()) else {
        return;
    };
    unsubscribe_deps_of(&inner, id);
}

/// Dispose the effect with this id, if it is still registered.
///
/// The by-id counterpart of [`Effect::dispose`], and its implementation.
///
/// The `Rc<EffectInner>` is moved out and dropped **after** the registry borrow
/// is released. That is load-bearing, not tidiness: the `Rc` owns the effect's
/// closure, which captures arbitrary user state — very often the only handle to
/// a child `RenderScope`. Dropping it in place runs that state's `Drop` while
/// `EFFECTS` is mutably borrowed, so a `Drop` that writes a signal flushes
/// effects synchronously into `run_effect`, whose `EFFECTS.borrow()` then panics
/// with a `BorrowMutError` (issue #141).
///
/// Removing the entry — rather than emptying a slot that stays in the container
/// forever — is what makes disposal actually reclaim (issue #141, final bullet).
pub(super) fn dispose_effect(id: ObserverId) {
    // Dropping the closure below runs user `Drop` code, and a read from it
    // must not subscribe whoever is mid-run (issue #494). The dispose fixpoint
    // already suspends tracking for its own call here; this covers the direct
    // `Effect::dispose` path, which runs outside any fixpoint. (A second
    // suspension only raises the run's suspension count, so the two compose.)
    let _untracked = super::tracking::SuspendObservers::take();
    let inner = EFFECTS.with(|effects| effects.borrow_mut().remove(&id));
    if let Some(inner) = &inner {
        // Belt and braces. Removal alone already makes `run_effect` a no-op,
        // since its only read of `disposed` follows a fresh lookup — but an
        // `Rc` handed out by an earlier lookup can still be in flight (that is
        // exactly what `run_effect` holds across the body it is running), and
        // this is the flag that tells such a run it has been retired.
        inner.disposed.set(true);
        // Release the subscriptions its last run took out, so a signal that
        // outlives this effect stops holding — and stops queueing — a dead
        // `ObserverId` (issue #171).
        unsubscribe_deps_of(inner, id);
    }
    drop(inner);
}

impl Drop for Effect {
    fn drop(&mut self) {
        // Note: We don't automatically dispose here to allow effects to outlive
        // their handles. Use dispose() explicitly if needed.
    }
}

/// Number of live entries in the effect registry, on this thread.
///
/// Test-only: the reclamation contract is about entries *disappearing*, which is
/// otherwise unobservable — the registry is `pub(super)` and nothing counts it.
#[cfg(test)]
pub(crate) fn registry_len_for_tests() -> usize {
    EFFECTS.with(|effects| effects.borrow().len())
}

/// The ids currently live in the effect registry, ascending.
///
/// Test-only, and separate from [`registry_len_for_tests`] on purpose: a
/// reclamation assertion has to check *which* entries survived, not merely how
/// many did. A rewrite that removes the wrong entry keeps the count right and
/// the contents wrong.
#[cfg(test)]
pub(crate) fn registry_ids_for_tests() -> Vec<usize> {
    EFFECTS.with(|effects| {
        let mut ids: Vec<usize> = effects.borrow().keys().map(|id| id.0).collect();
        ids.sort_unstable();
        ids
    })
}

/// Run a specific effect by ID
pub(super) fn run_effect(id: ObserverId) {
    let effect = EFFECTS.with(|effects| effects.borrow().get(&id).cloned());

    if let Some(inner) = effect {
        if inner.disposed.get() {
            tracing::debug!("run_effect({}): SKIPPED - disposed", id.0);
            return;
        }

        tracing::debug!("run_effect({}): running", id.0);

        // Re-enter the context root the effect was created under, so
        // use_context/use_store resolve the same namespace as at build time
        // (issue #136) — and the document it was created in, for the same
        // reason and for the same kind of consumer: the queue drains under
        // whichever document happens to be dispatching (issue #295), and a
        // `Drag::start()` or interceptor registration made from the body must
        // be attributed to the effect's own document. One guard for both, so
        // the document rides the root's TLS access for free.
        let _frame_guard = crate::context::enter_reactive_frame(inner.root, inner.doc);

        // Re-enter the scope that owned this effect at creation, for the same
        // reason: `flush_effects` runs from arbitrary stacks (an event handler,
        // a timer, the cross-thread drain), so the ambient owner at flush time
        // is unrelated to the component this effect belongs to (issue #141).
        let _owner_guard = inner.owner.push();

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
        // next genuine change. The wake is not parked anywhere for later: this
        // run was dequeued before it got here. User-facing statement of the
        // rule, and its batch-open exception (a write queued rather than
        // flushed is still pending when the body returns): `Effect`'s docs and
        // the guide, issue #343; pinned by `self_wake_tests`.
        //
        // The borrow is taken *before* the observer is pushed, and that order is
        // load-bearing: a tracking push also releases the previous run's
        // subscriptions (issue #171), and doing that on a run we are about to
        // skip would strip the subscriptions of the run in progress one frame
        // below — leaving an effect that never wakes again.
        let Ok(mut body) = inner.f.try_borrow_mut() else {
            tracing::debug!(
                "run_effect({}): SKIPPED - re-entered while already running",
                id.0
            );
            return;
        };

        // Make this effect the current observer, hiding whichever run this
        // one is nested inside: its reads are this effect's alone, an
        // `untracked` region in it hides it without exposing the run beneath
        // (issue #3047). RAII so a panic in the effect body cannot strand it
        // as current (issue #141). Retracking
        // only if the body is what reads this observer's dependencies — a memo's
        // marker keeps the subscriptions its recompute took out.
        let _observer_guard = if inner.body_tracks_deps {
            super::tracking::ObserverGuard::push_retracking(id)
        } else {
            super::tracking::ObserverGuard::push(id)
        };

        // A memo's marker is bookkeeping, not an effect body anyone wrote: it
        // only passes a wake on. Counting it would make `effect_runs` scale
        // with the number of memos a write *reached* rather than the effects
        // that actually re-ran.
        if inner.memo.is_none() {
            super::count_effect_run(inner.doc);
        }
        let _depth = super::ReactiveDepthGuard::enter();
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
///
/// **The memo equality cut-off lives here.** An observer queued by a signal it
/// reads directly is *definite* and runs. One queued only by a memo's marker is
/// a *maybe*: it runs only if at least one memo it read has moved to a new
/// version since it read it ([`memo_sources_changed`]), which brings those
/// memos up to date to find out. A skipped observer keeps its place in no
/// queue and its subscriptions untouched — it has seen nothing new. Skipping
/// never reorders anything: the observers that do run still run in the order
/// they were queued.
pub(super) fn flush_effects() {
    loop {
        let next = RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            let id = rt.pending_effects.pop_front()?;
            // Remove from set when dequeuing.
            rt.pending_effects_set.remove(&id);
            let definite = rt.definite_effects.remove(&id);
            Some((id, definite))
        });

        match next {
            Some((id, true)) => run_effect(id),
            Some((id, false)) => {
                if maybe_needs_run(id) {
                    run_effect(id);
                }
            }
            None => break,
        }
    }
}

/// Whether an observer woken only through a memo has anything new to see.
fn maybe_needs_run(id: ObserverId) -> bool {
    let Some(inner) = EFFECTS.with(|effects| effects.borrow().get(&id).cloned()) else {
        return false;
    };
    if inner.disposed.get() {
        return false;
    }
    // A memo's marker is never cut off here, only when nobody subscribes to
    // its memo. Asking whether the *memo's* sources moved is the wrong
    // question: a read in between (inside a batch, say) can already have
    // recomputed the memo and re-recorded its sources, so they look unmoved
    // while the memo's value has moved on from what its dependents last saw.
    // Only each dependent knows which version it saw, so the marker passes the
    // wake on and every dependent makes its own comparison.
    if let Some(memo) = &inner.memo {
        return memo.has_subscribers();
    }
    deps_changed(&inner)
}

/// Whether any memo `observer` read has moved to a new version since.
///
/// Used for a *maybe* observer (above) and for a memo in the `Check` state
/// (`MemoNode::refresh`), whose dependencies are recorded on its marker under
/// the same id. Refreshes each memo it asks about — that is the "pull".
pub(super) fn memo_sources_changed(observer: ObserverId) -> bool {
    let Some(inner) = EFFECTS.with(|effects| effects.borrow().get(&observer).cloned()) else {
        return true;
    };
    deps_changed(&inner)
}

fn deps_changed(inner: &EffectInner) -> bool {
    // Copied out: refreshing a memo runs user code, which must not find this
    // observer's `deps` borrowed.
    let memo_deps: Vec<(u32, u32, u64)> = inner
        .deps
        .borrow()
        .iter()
        .filter_map(|dep| match *dep {
            DepKey::Memo {
                id,
                generation,
                seen,
            } => Some((id, generation, seen)),
            DepKey::Signal { .. } => None,
        })
        .collect();
    // Woken through a memo but holding no record of reading one: nothing to
    // compare against, so run rather than guess.
    if memo_deps.is_empty() {
        return true;
    }
    memo_deps.into_iter().any(|(id, generation, seen)| {
        let node = super::MEMO_STORE.with(|store| store.borrow().get_node(id, generation));
        // A freed memo counts as moved. The reader was computed from its old
        // value and nothing will ever wake it again (the memo's marker went
        // with it), so leaving it clean would strand it on that value for good
        // — a `try_get` reader must get its chance to see `None`. Costs one
        // recompute, once.
        node.is_none_or(|node| node.refresh() != seen)
    })
}

#[cfg(test)]
mod hasher_tests;

#[cfg(test)]
mod reclamation_tests;

#[cfg(test)]
mod dispose_tests;

#[cfg(test)]
mod unsubscribe_tests;

/// The self-wake rule (issue #343): a write an effect's own body causes to a
/// signal it reads is **dropped** for that effect when the write is flushed
/// while the body is still running, and **kept** when the write is only queued.
///
/// The guide states this rule next to the execution-order contract
/// (`docs/src/guide/reactivity.md`, "An effect does not wake itself"); these
/// fixtures are what that paragraph rests on.
#[cfg(test)]
mod self_wake_tests;
