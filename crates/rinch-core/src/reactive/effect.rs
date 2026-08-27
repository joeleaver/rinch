//! Effect: a side-effect that re-runs when its dependencies change.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::Rc;

use super::{ObserverId, RUNTIME};

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
    /// The scope that owned this effect when it was created, re-entered on every
    /// run so resources the body creates are attributed to the component that
    /// built it rather than to whatever happened to be rendering when the flush
    /// fired (issue #141). Weak by construction — see [`Owner`](super::Owner);
    /// a strong reference here would make every scope immortal, since `EFFECTS`
    /// holds the `Rc<EffectInner>` until `Scope::dispose` removes it.
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
            f: RefCell::new(Box::new(f)),
            disposed: Cell::new(false),
            root: crate::context::current_context_root(),
            owner: super::Owner::current(),
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

        let inner = Rc::new(EffectInner {
            f: RefCell::new(Box::new(f)),
            disposed: Cell::new(false),
            root: crate::context::current_context_root(),
            owner: super::Owner::current(),
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
    /// Also removes it from the global [`EFFECTS`] registry, allowing the
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
    let inner = EFFECTS.with(|effects| effects.borrow_mut().remove(&id));
    if let Some(inner) = &inner {
        // Belt and braces. Removal alone already makes `run_effect` a no-op,
        // since its only read of `disposed` follows a fresh lookup — but an
        // `Rc` handed out by an earlier lookup can still be in flight (that is
        // exactly what `run_effect` holds across the body it is running), and
        // this is the flag that tells such a run it has been retired.
        inner.disposed.set(true);
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
    let effect = EFFECTS.with(|effects| effects.borrow().get(&id).cloned());

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
            // Remove from set when dequeuing.
            if let Some(ref observer) = id {
                rt.pending_effects_set.remove(observer);
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
mod hasher_tests {
    use super::*;

    /// Every write folds, so a key that hashes more than one word is not
    /// reduced to its last word.
    ///
    /// `ObserverId` writes exactly one `usize` today, so this is a guard on the
    /// hasher's stated contract rather than on current behaviour: an assigning
    /// `write_usize` collides `(1, 2)` with `(9, 2)` outright, silently, the day
    /// the key type grows a field.
    #[test]
    fn writes_fold_rather_than_overwrite() {
        fn hash_words(words: &[usize]) -> u64 {
            let mut h = ObserverIdHasher::default();
            for &w in words {
                h.write_usize(w);
            }
            h.finish()
        }

        assert_ne!(hash_words(&[1, 2]), hash_words(&[9, 2]));
        assert_ne!(hash_words(&[1, 2]), hash_words(&[2, 1]));
        // The single-word case is unchanged by folding: the state starts at 0.
        assert_eq!(hash_words(&[7]), 7u64.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    }

    /// Consecutive ids — the only keys this map ever sees — land in distinct
    /// buckets *and* distinct control bytes.
    ///
    /// The multiply is what buys the second half: an identity hash leaves the
    /// top 7 bits (hashbrown's control byte) constant across every id a real
    /// program allocates.
    #[test]
    fn consecutive_ids_spread_across_buckets_and_control_bytes() {
        use std::collections::HashSet;

        let hashes: Vec<u64> = (0..256usize)
            .map(|n| {
                let mut h = ObserverIdHasher::default();
                h.write_usize(n);
                h.finish()
            })
            .collect();

        let low: HashSet<u64> = hashes.iter().map(|h| h & 0xFF).collect();
        assert_eq!(low.len(), 256, "bucket index must be a permutation");
        let control: HashSet<u64> = hashes.iter().map(|h| h >> 57).collect();
        assert!(
            control.len() > 64,
            "control bytes must vary, got {}",
            control.len()
        );
    }
}

#[cfg(test)]
mod reclamation_tests {
    use super::*;
    use crate::reactive::{Memo, Scope, Signal};
    use std::cell::Cell;

    /// Disposing a scope returns the effect registry to its previous size.
    ///
    /// The last of the five leaks in issue #141: `ObserverId`s are monotonic, so
    /// an id-indexed vec grew by one entry per observer ever created and never
    /// shrank. Disposal emptied the slot; the slot itself stayed forever.
    ///
    /// The loop count matters. A single cycle cannot distinguish "reclaimed"
    /// from "the container happens to be this size" — the pre-fix vec is *also*
    /// small after one cycle. 100 cycles makes the two answers differ by two
    /// orders of magnitude.
    #[test]
    fn disposing_a_scope_returns_the_effect_registry_to_its_baseline() {
        // Positive control, created *outside* the loop: proves the shrinking
        // below is entries being reclaimed rather than the registry being
        // wrongly emptied. An implementation that removed too much passes a
        // bare length assertion and fails here.
        let survivor_source = Signal::new(0);
        let survivor_runs = Rc::new(Cell::new(0));
        let hits = survivor_runs.clone();
        let _survivor = Effect::new(move || {
            survivor_source.get();
            hits.set(hits.get() + 1);
        });
        assert_eq!(survivor_runs.get(), 1);

        let baseline = registry_len_for_tests();
        let baseline_ids = registry_ids_for_tests();

        for _ in 0..100 {
            let scope = Scope::new();
            scope.run(|| {
                let source = Signal::new(1);
                let doubled = Memo::new(move || source.get() * 2);
                Effect::new(move || {
                    // Read the memo so its marker is genuinely subscribed, not
                    // an inert entry nothing ever touches.
                    let _ = doubled.get();
                });
            });
            scope.dispose();
        }

        assert_eq!(
            registry_len_for_tests(),
            baseline,
            "every disposed scope's effect and memo marker must leave the registry; \
             pre-fix this grows by ~2 per cycle"
        );
        // Assert *which* entries survived, not merely how many — a removal that
        // evicts the wrong id keeps the count right and the contents wrong.
        assert_eq!(registry_ids_for_tests(), baseline_ids);

        // And the survivor is still wired up, not merely still counted.
        survivor_source.set(1);
        assert_eq!(
            survivor_runs.get(),
            2,
            "a live effect outside the disposed scopes still runs"
        );
    }

    /// The two removal paths are independent and both must reclaim.
    ///
    /// An effect leaves via `dispose_effect`; a memo's dirty marker leaves via
    /// `free_memo`, which is reached from a different level of the disposal
    /// fixpoint. Testing them only together lets a fix for one hide a miss on
    /// the other.
    #[test]
    fn both_removal_paths_reclaim_independently() {
        let baseline = registry_len_for_tests();

        // Effect-only: `dispose_effect`.
        let scope = Scope::new();
        scope.run(|| {
            Effect::new(|| {});
        });
        assert_eq!(registry_len_for_tests(), baseline + 1);
        scope.dispose();
        assert_eq!(
            registry_len_for_tests(),
            baseline,
            "dispose_effect must remove the entry, not empty it"
        );

        // Memo-only: `free_memo`'s marker removal.
        let source = Signal::new(1);
        let scope = Scope::new();
        scope.run(|| {
            let doubled = Memo::new(move || source.get() * 2);
            assert_eq!(doubled.get(), 2);
        });
        assert_eq!(
            registry_len_for_tests(),
            baseline + 1,
            "a memo registers exactly one marker"
        );
        scope.dispose();
        assert_eq!(
            registry_len_for_tests(),
            baseline,
            "free_memo must remove the marker entry, not empty it"
        );
    }

    /// Explicit `Effect::dispose` reclaims too, not just scope teardown.
    #[test]
    fn explicitly_disposing_an_effect_removes_its_entry() {
        let baseline = registry_len_for_tests();
        let effect = Effect::new(|| {});
        assert_eq!(registry_len_for_tests(), baseline + 1);
        effect.dispose();
        assert_eq!(registry_len_for_tests(), baseline);
        // Idempotent: a second dispose is a no-op, not an underflow or a panic.
        effect.dispose();
        assert_eq!(registry_len_for_tests(), baseline);
    }

    /// Dropping an `Effect` handle deliberately does **not** reclaim.
    ///
    /// Pinned because it is the surprising half of the contract and the reason
    /// scope ownership exists: an unowned `Effect::new(..)` whose handle is
    /// dropped keeps running for the life of the thread.
    #[test]
    fn dropping_an_effect_handle_does_not_remove_its_entry() {
        let baseline = registry_len_for_tests();
        drop(Effect::new(|| {}));
        assert_eq!(
            registry_len_for_tests(),
            baseline + 1,
            "the handle is not the owner — disposal is explicit or via a scope"
        );
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
