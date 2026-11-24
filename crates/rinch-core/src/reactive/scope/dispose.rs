//! Disposal: [`ScopeInner::dispose`] collects a scope's subtree into the
//! thread's in-flight [`DisposeCtx`], and [`run_dispose_fixpoint`] releases
//! it in priority order, with the observer stack suspended
//! ([`SuspendObservers`]) so teardown reads subscribe nobody.

use std::any::Any;
use std::cell::RefCell;

use super::handle::{Cleanup, Scope, ScopeInner};
use super::{MemoKey, SignalKey};
use crate::events::EventHandlerId;
use crate::reactive::{ObserverId, RUNTIME};

impl ScopeInner {
    /// Dispose this scope and everything beneath it.
    ///
    /// Two phases, and the split is what keeps teardown iterative:
    ///
    /// 1. **Collect** — mark this scope and its whole subtree disposed and move
    ///    their work into the thread-local [`DisposeCtx`]. Runs no user code.
    /// 2. **Drain** — the *outermost* call runs [`run_dispose_fixpoint`].
    ///
    /// Every nested `dispose()` — and there are many, because an effect closure
    /// commonly owns the only handle to a child `RenderScope`, so dropping it
    /// disposes that scope — finds the context already installed, contributes to
    /// it, and returns. Only the outermost frame drains, so the recursion depth
    /// stays at "one collect walk" instead of growing with the component tree.
    pub(super) fn dispose(&self) {
        if self.disposed.get() {
            return;
        }

        let is_root = DISPOSE_CTX.with(|ctx| {
            let mut ctx = ctx.borrow_mut();
            if ctx.is_none() {
                *ctx = Some(DisposeCtx::default());
                true
            } else {
                false
            }
        });

        self.collect();

        if is_root {
            // RAII, because the fixpoint runs arbitrary user code and it can
            // panic — see [`DisposeCtxGuard`].
            let _ctx_guard = DisposeCtxGuard;
            run_dispose_fixpoint();
        }
    }

    /// Mark this scope and its whole subtree disposed, moving their work into
    /// the in-flight [`DisposeCtx`].
    ///
    /// Iterative rather than recursive: the subtree can be as deep as the
    /// component tree, and `Scope` is not `Clone`, so popping a handle off the
    /// worklist is the last reference to it.
    fn collect(&self) {
        let mut pending: Vec<Scope> = Vec::new();
        self.collect_one(&mut pending);
        while let Some(child) = pending.pop() {
            child.0.collect_one(&mut pending);
            // `child` drops here. Its `disposed` flag is already set, so the
            // `Drop for ScopeInner` that fires is an immediate no-op rather
            // than a second, recursive teardown.
        }
    }

    /// Move one scope's work into the context. Allocation-only — no user code
    /// runs here, so no borrow taken below can be observed re-entrantly.
    fn collect_one(&self, pending: &mut Vec<Scope>) {
        // Children are harvested even from an already-disposed scope: Rust's
        // default field drop would otherwise walk them recursively when the last
        // handle dies, which is exactly the stack overflow the worklist exists
        // to prevent. (A live scope's list is normally empty by then.)
        pending.extend(self.children.borrow_mut().drain(..));

        if self.disposed.get() {
            return;
        }
        self.disposed.set(true);

        // Adopted effects and *owned* effects are both released. The two lists
        // overlap for the common case (created and adopted under the same
        // scope); the difference is the fire-and-forget effects that nothing
        // ever adopted, which before #141 leaked for the life of the thread.
        // Disposing an id twice is a no-op, so the overlap needs no dedup.
        let effects: Vec<ObserverId> = self
            .effects
            .borrow_mut()
            .drain(..)
            .map(|effect| effect.id())
            .collect();
        let cleanups: Vec<Cleanup> = self.cleanups.borrow_mut().drain(..).collect();
        let owned = std::mem::take(&mut *self.owned.borrow_mut());

        with_dispose_ctx(|ctx| {
            ctx.effects.extend(effects);
            ctx.effects.extend(owned.effects);
            ctx.cleanups.extend(cleanups);
            ctx.handlers.extend(owned.handlers);
            ctx.memos.extend(owned.memos);
            ctx.signals.extend(owned.signals);
        });
    }
}

// ============================================================================
// The disposal fixpoint
// ============================================================================

/// Work harvested from scopes that have been marked disposed, drained in
/// priority order by [`run_dispose_fixpoint`].
///
/// Every field is plain data or a boxed closure — pushing to and taking from a
/// `DisposeCtx` never runs user code, which is what lets the whole struct be
/// touched under a `RefCell` borrow safely.
#[derive(Default)]
pub(super) struct DisposeCtx {
    handlers: Vec<EventHandlerId>,
    effects: Vec<ObserverId>,
    pub(super) cleanups: Vec<Cleanup>,
    /// Keys of removable cleanups removed while the batch holding them was
    /// running ([`CleanupHandle::remove`](super::CleanupHandle::remove) from another cleanup): not run.
    pub(super) removed: Vec<u64>,
    memos: Vec<MemoKey>,
    signals: Vec<SignalKey>,
    /// Values taken out of freed signal slots, dropped at the lowest priority.
    values: Vec<Box<dyn Any>>,
}

thread_local! {
    /// The in-flight disposal, or `None` when nothing is being disposed.
    ///
    /// `Some` also *means* "a fixpoint is already draining above me", which is
    /// how [`ScopeInner::dispose`] tells an outermost call from a nested one.
    pub(super) static DISPOSE_CTX: RefCell<Option<DisposeCtx>> = const { RefCell::new(None) };
}

/// Whether a scope can be disposed on this thread now: `false` once the thread
/// has begun destroying the locals disposal reads, which happens when a
/// [`Scope`] is dropped from a `thread_local!` at thread exit after them (the
/// destruction order of thread locals is not under anyone's control).
/// [`ScopeInner`]'s `Drop` asks it and skips disposal when it answers `false`.
pub(crate) fn disposal_available() -> bool {
    DISPOSE_CTX.try_with(|_| ()).is_ok()
        && RUNTIME.try_with(|_| ()).is_ok()
        && crate::reactive::effect::EFFECTS.try_with(|_| ()).is_ok()
        && crate::reactive::SIGNAL_STORE.try_with(|_| ()).is_ok()
        && crate::reactive::MEMO_STORE.try_with(|_| ()).is_ok()
}

/// Suspends the **entire** observer stack until dropped.
///
/// Teardown's counterpart to [`untracked`](crate::reactive::untracked), which pops
/// exactly one observer and is the right depth for a *render* boundary: user
/// render code always runs exactly one boundary deep, and every boundary wraps
/// its render in one `untracked`, so a single pop leaves the stack balanced at
/// any nesting depth (the #491 invariant). Disposal has no such structural
/// position — the fixpoint can start from a reconcile effect (one observer
/// live), from a bare `Effect::new` first run nested inside another effect
/// (two live, with nothing having balanced the outer frame), or from
/// `run_effect` dropping a retired closure after its own observer already
/// popped (whatever the surrounding context holds). "A teardown read
/// subscribes nobody" is depth-independent, so the guard is too: every frame
/// is suspended, and restored on drop — including while unwinding, for the
/// same reason `untracked`'s `RestoreObserver` restores (#232): a panic caught
/// upstream must not leave the stack short.
///
/// Effects that *run* during the suspension track normally — a cleanup that
/// writes a signal flushes effects synchronously, and a cleanup may even
/// create one; each pushes its own observer onto the emptied stack. Only the
/// observers that were mid-run when disposal began are hidden, and those are
/// exactly the ones a teardown read must not subscribe (issue #494).
pub(in crate::reactive) struct SuspendObservers {
    saved: Vec<ObserverId>,
}

impl SuspendObservers {
    pub(in crate::reactive) fn take() -> Self {
        SuspendObservers {
            saved: RUNTIME.with(|rt| std::mem::take(&mut rt.borrow_mut().observer_stack)),
        }
    }
}

impl Drop for SuspendObservers {
    fn drop(&mut self) {
        if self.saved.is_empty() {
            return;
        }
        let saved = std::mem::take(&mut self.saved);
        let _ = crate::reactive::restore::with_runtime_on_drop(
            "the observer stack disposal suspended",
            |rt| {
                // The saved frames go back *under* anything still on the stack:
                // an observer pushed during the suspension and not yet popped
                // belongs to a frame above this guard, and its `ObserverGuard`
                // pops blindly from the top. (In normal flow the stack is empty
                // again here — every run that started during the suspension has
                // finished — so this is belt and braces for unwinding.)
                let since = std::mem::replace(&mut rt.observer_stack, saved);
                rt.observer_stack.extend(since);
            },
        );
    }
}

/// Clears the in-flight [`DisposeCtx`] on drop, including while unwinding.
///
/// The fixpoint runs arbitrary user code — cleanups, closure drops, value drops
/// — and any of it can panic. Clearing the context on the normal path only would
/// leave it installed for the rest of the thread's life, and then every later
/// `dispose()` would see it, conclude it was *nested*, collect into it and never
/// drain. One panicking cleanup would silently and permanently disable disposal
/// process-wide, which is a far worse failure than the panic itself.
struct DisposeCtxGuard;

impl Drop for DisposeCtxGuard {
    fn drop(&mut self) {
        // `try_with`: TLS may already be torn down at thread exit, exactly as in
        // `ObserverGuard::drop`. `try_borrow_mut` guards the (pathological) case
        // of unwinding out of `with_dispose_ctx` itself.
        let leftover = DISPOSE_CTX
            .try_with(|ctx| ctx.try_borrow_mut().ok().and_then(|mut ctx| ctx.take()))
            .ok()
            .flatten();
        // Dropped out here with no borrow held: an abandoned batch still holds
        // un-run cleanups and freed values, i.e. more arbitrary user `Drop`s —
        // still teardown user code, so still untracked (issue #494), even on
        // this unwinding path.
        if leftover.is_some() {
            let _untracked = SuspendObservers::take();
            drop(leftover);
        }
    }
}

/// Run `f` against the in-flight disposal context, if there is one.
///
/// Holds the context borrow across `f`, so `f` must be allocation-only: pushing
/// work on, or taking a batch off. Never user code.
pub(super) fn with_dispose_ctx<R>(f: impl FnOnce(&mut DisposeCtx) -> R) -> Option<R> {
    DISPOSE_CTX.with(|ctx| ctx.borrow_mut().as_mut().map(f))
}

/// Take one batch off the context, leaving it empty.
fn take_batch<T>(f: impl FnOnce(&mut DisposeCtx) -> &mut Vec<T>) -> Vec<T> {
    with_dispose_ctx(|ctx| std::mem::take(f(ctx))).unwrap_or_default()
}

/// Release everything collected into the [`DisposeCtx`], to a fixpoint.
///
/// # Why a fixpoint and not a pipeline
///
/// A scope's subtree is not enumerable up front. Child `RenderScope`s live
/// *inside* their parent effect's closure, so a subtree only becomes reachable
/// once the parent effect is disposed and its closure dropped. Every level below
/// therefore runs code that can mark more scopes disposed — dropping a closure,
/// running a cleanup, dropping a user value — and each of those pushes fresh
/// work onto the context. Draining the levels once, in order, would leave that
/// late-arriving work stranded.
///
/// So each level, after doing any work at all, **restarts from the top**. The
/// loop exits only when a full sweep finds every level empty.
///
/// # Why this order
///
/// Strictly: handlers → effects → cleanups → memos → signals → value drops.
///
/// - **Handlers first.** Unregistering them runs no user *callbacks* (dropping
///   a callback still drops its captured state, so user `Drop`s can run — the
///   observer guard above covers them like every other step), and doing it
///   first means a re-entrant dispatch arriving during
///   teardown (a cleanup that pumps the event loop, a `Drop` that fires a
///   callback) finds nothing to call, instead of calling into a component whose
///   signals are about to be freed.
/// - **Cleanups after effects**, so a cleanup cannot resurrect an effect that
///   was already disposed (issue #141, maintainer decision).
/// - **Memos before signals**, because a memo recompute *reads* signals; a memo
///   still reachable after its sources were freed would panic on the next read.
/// - **Signals second to last**, because everything above — effect closures,
///   cleanups — legitimately reads them while running.
///
/// # Termination
///
/// Every level either does nothing or permanently consumes at least one item:
/// handler ids are removed from the registries and never re-added, effect
/// entries are removed from the registry, cleanups are `FnOnce`, memo and
/// signal slots are generation
/// guarded so a second free is a no-op, and values are dropped. New work only
/// arrives by marking a not-yet-disposed scope disposed, and `disposed` is
/// one-way. The loop is therefore bounded by the number of live scopes, plus
/// whatever user code creates *and* disposes while unwinding — which is
/// unbounded only in the same sense that any user `Drop` is.
///
/// # Completeness is per invocation, not per instant
///
/// One case defers to a *later* fixpoint rather than joining this one: a scope
/// disposed from inside a **running effect**. `run_effect` holds a strong
/// `Rc<EffectInner>` across the whole body, so `dispose_effect` can only remove
/// the registry entry — the closure, and any child `RenderScope` it captures, is
/// not dropped until that body returns, by which point this fixpoint has exited
/// and the drop starts a fresh one.
///
/// Nothing is missed; it just happens one turn later. The practical consequence
/// is for tests: after a dispose triggered from inside an effect, assert
/// liveness once the flush has settled, not on the line after `dispose()`.
fn run_dispose_fixpoint() {
    // Teardown is untracked (issue #494). Every level below runs user code —
    // step 1 drops handler closures, step 2 effect closures, step 3 runs
    // `on_cleanup`s, step 4 drops memo cached values, step 6 drops signal
    // values — and the fixpoint runs inside whatever effect asked for the
    // disposal (every reconcile entry point disposes outgoing scopes from its
    // own effect). A signal read anywhere in that user code would otherwise
    // subscribe the observers that were mid-run when disposal began, and the
    // reconcile would re-run on every later write to that signal, driven by a
    // scope that no longer exists. One guard across the whole drain covers
    // cascades too: a cleanup that disposes a further scope only pushes work
    // onto the in-flight context, and that work drains in later iterations of
    // this same loop, still under this guard.
    let _untracked = SuspendObservers::take();
    loop {
        // 1. Handlers.
        let handlers = take_batch(|ctx| &mut ctx.handlers);
        if !handlers.is_empty() {
            for id in handlers {
                crate::events::unregister_handler(id);
            }
            continue;
        }

        // 2. Effects. Dropping an effect's closure is the step that reaches
        //    child scopes, so this is where the subtree grows.
        let effects = take_batch(|ctx| &mut ctx.effects);
        if !effects.is_empty() {
            for id in effects {
                crate::reactive::effect::dispose_effect(id);
            }
            continue;
        }

        // 3. Cleanups.
        let cleanups = take_batch(|ctx| &mut ctx.cleanups);
        if !cleanups.is_empty() {
            for cleanup in cleanups {
                let removed = cleanup.key != 0
                    && with_dispose_ctx(|ctx| {
                        let at = ctx.removed.iter().position(|k| *k == cleanup.key);
                        at.map(|at| ctx.removed.swap_remove(at)).is_some()
                    })
                    .unwrap_or(false);
                if !removed {
                    (cleanup.run)();
                }
            }
            continue;
        }

        // 4. Memos. `free_memo`, never `MemoStore::free`: a memo holds two
        //    strong `Rc<MemoInner>`, one in the store and one captured by its
        //    dirty-marker effect, and releasing only the store slot frees
        //    nothing *and* leaves the marker subscribed — so the next write to a
        //    source re-queues the memo's dependents, whose `get()` then panics
        //    on the empty slot. `free_memo` drops the cached value here, while
        //    signals are still live, which is strictly friendlier than deferring
        //    it: a derived value's `Drop` may still read the state it derives
        //    from.
        let memos = take_batch(|ctx| &mut ctx.memos);
        if !memos.is_empty() {
            for key in memos {
                crate::reactive::free_memo(key.id, key.generation);
            }
            continue;
        }

        // 5. Signals. The value comes back out rather than dropping under the
        //    store borrow, and is parked for level 6 rather than dropped here:
        //    dropping in place would let one value's `Drop` observe an arbitrary
        //    prefix of its siblings already freed. Deferring makes the state a
        //    value's `Drop` sees deterministic — *everything* this scope owned
        //    is gone — which matches the documented contract for reading a
        //    handle after its scope was disposed.
        let signals = take_batch(|ctx| &mut ctx.signals);
        if !signals.is_empty() {
            let values: Vec<Box<dyn Any>> = signals
                .into_iter()
                .filter_map(|key| crate::reactive::free_signal(key.id, key.generation))
                .collect();
            with_dispose_ctx(|ctx| ctx.values.extend(values));
            continue;
        }

        // 6. Value drops.
        let values = take_batch(|ctx| &mut ctx.values);
        if !values.is_empty() {
            drop(values);
            continue;
        }

        break;
    }
}

impl Drop for ScopeInner {
    fn drop(&mut self) {
        // A scope kept in a `thread_local!` (directly, or inside a menu
        // registration or a tray) is dropped at thread exit, possibly after
        // the runtime locals disposal uses. Disposing then panics inside a TLS
        // destructor, which aborts the process. The thread is ending, so
        // there is nothing left to release: mark the scope disposed and let
        // the fields drop as plain data.
        if !disposal_available() {
            self.disposed.set(true);
            return;
        }
        // Use the iterative dispose to avoid stack overflow.
        // Rust's default field drop would recursively drop children -> Scope -> Drop,
        // and Effect::dispose can drop closures that own RenderScopes -> more Scopes.
        //
        // This lives on `ScopeInner`, not `Scope`, so it fires when the last
        // handle dies. `Scope` is deliberately not `Clone`, so that is the same
        // instant the single handle drops.
        self.dispose();
    }
}
