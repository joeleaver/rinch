//! Scope: manages the lifetime of reactive primitives.
//!
//! # Ownership is ambient
//!
//! A [`Scope`] is both a disposal list (effects, cleanups, child scopes) and an
//! **owner**: while a scope is pushed as the ambient owner, every [`Signal`],
//! [`Memo`], [`Effect`] and event handler created is *attributed* to it.
//!
//! Disposing a scope **frees** everything attributed to it. Reads of a freed
//! [`Signal`]/[`Memo`] panic and writes become warn-once no-ops (see the
//! [`reactive`](super) module's "Liveness" section); a freed event handler
//! simply stops being found by dispatch.
//!
//! **No ambient owner means app lifetime.** Resources created outside any
//! render — in `main()`, in a startup routine, in a detached callback — are
//! owned by nobody and live as long as the thread. That is what makes
//! ownership non-breaking to add.
//!
//! Two independent stacks are in play, and they are easy to confuse:
//!
//! | stack | suspended by | answers |
//! |---|---|---|
//! | observer stack | [`untracked`](super::untracked) | *who subscribes to this read* |
//! | owner stack | [`unowned`] | *who owns this allocation* |
//!
//! [`Signal`]: super::Signal
//! [`Memo`]: super::Memo

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::{Rc, Weak};

use super::{Effect, ObserverId, RUNTIME};
use crate::events::EventHandlerId;

// ============================================================================
// Resource keys
// ============================================================================

/// Identifies a slot in `SIGNAL_STORE`.
///
/// A distinct newtype from [`MemoKey`] rather than a shared `(u32, u32)` alias:
/// signals and memos are released by *different* functions (`SignalStore::free`
/// versus `free_memo`), and calling the wrong one is silently catastrophic —
/// `MemoStore::free` alone frees nothing and leaves the memo's dirty-marker
/// subscribed, so the next write to a dependency panics its dependents. Making
/// the two keys unmixable is the cheapest guard available.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::reactive) struct SignalKey {
    pub id: u32,
    pub generation: u32,
}

/// Identifies a slot in `MEMO_STORE`. See [`SignalKey`] for why this is a
/// separate type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::reactive) struct MemoKey {
    pub id: u32,
    pub generation: u32,
}

// ============================================================================
// The owned-resource bucket
// ============================================================================

/// Resources created while a scope was the ambient owner.
///
/// Every field is plain data — ids and keys, never the resource itself — so
/// taking an `Owned` apart runs no user code and cannot re-enter the disposal
/// machinery. The actual release happens later, in
/// [`run_dispose_fixpoint`], from the batches harvested here.
#[derive(Default)]
pub(in crate::reactive) struct Owned {
    signals: Vec<SignalKey>,
    memos: Vec<MemoKey>,
    handlers: Vec<EventHandlerId>,
    /// Effects *created* under this owner.
    ///
    /// Deliberately distinct from [`ScopeInner::effects`], which holds effects
    /// *adopted* via [`Scope::add_effect`]. Disposal releases both: the
    /// difference between the two counts is the fire-and-forget effects nothing
    /// ever adopted, which is precisely the leak #141 was filed for.
    effects: Vec<ObserverId>,
}

/// A snapshot of what a scope owns.
///
/// Introspection for tests and diagnostics. Disposal empties the bucket, so
/// after `dispose` every count reads zero.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OwnedCounts {
    pub signals: usize,
    pub memos: usize,
    pub handlers: usize,
    pub effects: usize,
}

impl Owned {
    fn counts(&self) -> OwnedCounts {
        OwnedCounts {
            signals: self.signals.len(),
            memos: self.memos.len(),
            handlers: self.handlers.len(),
            effects: self.effects.len(),
        }
    }
}

// ============================================================================
// Scope
// ============================================================================

/// A scope that manages the lifetime of reactive primitives.
///
/// Disposing a scope releases everything it owns and everything its child
/// scopes own — event handlers, effects, cleanups, memos, signals — and
/// dropping the scope does the same thing. See [`run_dispose_fixpoint`] for the
/// order and why it is a fixpoint.
///
/// A scope owns a resource if it was the *ambient owner* when that resource was
/// created — see [`run`](Scope::run) and the module docs. Effects are the one
/// thing that can also be **adopted** explicitly, via
/// [`add_effect`](Scope::add_effect), which is how an effect created elsewhere
/// is made to die here.
///
/// # Example
///
/// ```ignore
/// let scope = Scope::new();
///
/// scope.run(|| {
///     let signal = Signal::new(0);   // owned by this scope; freed on dispose
///     scope.add_effect(Effect::new(|| { /* ... */ }));
/// });
///
/// scope.dispose();
/// assert!(!signal.is_alive());
/// ```
pub struct Scope(Rc<ScopeInner>);

/// The reference-counted body of a [`Scope`].
///
/// Split out so that an [`Owner`] can hold a non-owning [`Weak`] reference to a
/// live scope from inside the ambient owner stack, an `EffectInner`, a
/// `MemoInner`, or an event-handler wrapper — all of which must observe a scope
/// without keeping it alive.
pub(in crate::reactive) struct ScopeInner {
    effects: RefCell<Vec<Effect>>,
    children: RefCell<Vec<Scope>>,
    cleanups: RefCell<Vec<Box<dyn FnOnce()>>>,
    disposed: Cell<bool>,
    owned: RefCell<Owned>,
}

impl Scope {
    /// Create a new scope.
    pub fn new() -> Self {
        Scope(Rc::new(ScopeInner {
            effects: RefCell::new(Vec::new()),
            children: RefCell::new(Vec::new()),
            cleanups: RefCell::new(Vec::new()),
            disposed: Cell::new(false),
            owned: RefCell::new(Owned::default()),
        }))
    }

    /// Check if this scope has been disposed.
    pub fn is_disposed(&self) -> bool {
        self.0.disposed.get()
    }

    /// Make this scope the ambient owner for the duration of `f`.
    ///
    /// Signals, memos, effects and event handlers created inside `f` are
    /// **attributed** to this scope. Effects are attributed but *not adopted* —
    /// [`dispose`](Scope::dispose) only disposes effects handed to
    /// [`add_effect`](Scope::add_effect).
    ///
    /// This takes `&self`, so it cannot be used where the surrounding code also
    /// needs `&mut` access to a value the closure borrows. The DOM render sites
    /// are exactly that shape and use the RAII `RenderScope::push_owner` guard
    /// instead.
    pub fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        let _owner = self.push_owner();
        f()
    }

    /// Make this scope the ambient owner until the returned guard drops.
    ///
    /// The RAII form of [`run`](Scope::run), for callers that cannot wrap the
    /// owned region in a closure.
    pub(crate) fn push_owner(&self) -> OwnerGuard {
        push_owner_weak(Rc::downgrade(&self.0))
    }

    /// A non-owning reference to this scope, for comparison and diagnostics.
    pub fn owner(&self) -> Owner {
        Owner(Rc::downgrade(&self.0))
    }

    /// What this scope currently owns. See [`OwnedCounts`].
    #[doc(hidden)]
    pub fn owned_counts(&self) -> OwnedCounts {
        self.0.owned.borrow().counts()
    }

    /// Register an effect with this scope, so `dispose` disposes it.
    pub fn add_effect(&self, effect: Effect) {
        self.0.effects.borrow_mut().push(effect);
    }

    /// Add a child scope to be disposed with this scope.
    pub fn add_child(&self, child: Scope) {
        self.0.children.borrow_mut().push(child);
    }

    /// Register a cleanup function to run when this scope is disposed.
    ///
    /// Cleanups run **after** every effect this scope owns has been disposed,
    /// and after the whole subtree beneath it has been marked disposed — so a
    /// cleanup cannot resurrect a disposed effect, and writing a signal from one
    /// wakes only observers outside the tree coming down. They run **before**
    /// the scope's memos and signals are freed, so a cleanup may still read
    /// them. See [`run_dispose_fixpoint`].
    ///
    /// A cleanup registered by another cleanup on the same scope still runs:
    /// the fixpoint takes the list out before running any of it, then loops.
    pub fn on_cleanup<F: FnOnce() + 'static>(&self, f: F) {
        self.0.cleanups.borrow_mut().push(Box::new(f));
    }

    /// Dispose of all effects, child scopes, and run cleanup functions.
    ///
    /// After dispose, this scope should not be used.
    pub fn dispose(&self) {
        self.0.dispose();
    }

    // NOTE: there was a `take_effects()` here that moved the adopted-effect list
    // out without disposing it. It had no callers, and it is unsound now that
    // disposal frees resources: it left the scope's signal/memo/handler records
    // behind, so the taken effects kept running against state the next dispose
    // freed — a use-after-free by construction (issue #141). If detaching
    // effects is ever needed, it has to move the matching `Owned` entries too.

    /// Get the number of adopted effects in this scope.
    pub fn effect_count(&self) -> usize {
        self.0.effects.borrow().len()
    }

    /// Get the number of child scopes.
    pub fn child_count(&self) -> usize {
        self.0.children.borrow().len()
    }

    /// Strong reference count of the inner scope. Test-only: it is what
    /// distinguishes a `Weak` owner stack from an `Rc` one.
    #[cfg(test)]
    pub(in crate::reactive) fn strong_count(&self) -> usize {
        Rc::strong_count(&self.0)
    }
}

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
    fn dispose(&self) {
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
        let cleanups: Vec<Box<dyn FnOnce()>> = self.cleanups.borrow_mut().drain(..).collect();
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
struct DisposeCtx {
    handlers: Vec<EventHandlerId>,
    effects: Vec<ObserverId>,
    cleanups: Vec<Box<dyn FnOnce()>>,
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
    static DISPOSE_CTX: RefCell<Option<DisposeCtx>> = const { RefCell::new(None) };
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
        // un-run cleanups and freed values, i.e. more arbitrary user `Drop`s.
        drop(leftover);
    }
}

/// Run `f` against the in-flight disposal context, if there is one.
///
/// Holds the context borrow across `f`, so `f` must be allocation-only: pushing
/// work on, or taking a batch off. Never user code.
fn with_dispose_ctx<R>(f: impl FnOnce(&mut DisposeCtx) -> R) -> Option<R> {
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
/// - **Handlers first.** Unregistering them is the only step that runs no user
///   code at all, and doing it first means a re-entrant dispatch arriving during
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
/// handler ids are removed from the registries and never re-added, effect slots
/// are emptied, cleanups are `FnOnce`, memo and signal slots are generation
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
/// `Rc<EffectInner>` across the whole body, so `dispose_effect` can only empty
/// the registry slot — the closure, and any child `RenderScope` it captures, is
/// not dropped until that body returns, by which point this fixpoint has exited
/// and the drop starts a fresh one.
///
/// Nothing is missed; it just happens one turn later. The practical consequence
/// is for tests: after a dispose triggered from inside an effect, assert
/// liveness once the flush has settled, not on the line after `dispose()`.
fn run_dispose_fixpoint() {
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
                super::effect::dispose_effect(id);
            }
            continue;
        }

        // 3. Cleanups.
        let cleanups = take_batch(|ctx| &mut ctx.cleanups);
        if !cleanups.is_empty() {
            for cleanup in cleanups {
                cleanup();
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
                super::free_memo(key.id, key.generation);
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
                .filter_map(|key| super::free_signal(key.id, key.generation))
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

impl Default for Scope {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ScopeInner {
    fn drop(&mut self) {
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

impl std::fmt::Debug for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let owned = self.0.owned.borrow();
        f.debug_struct("Scope")
            .field("effects", &self.0.effects.borrow().len())
            .field("children", &self.0.children.borrow().len())
            .field("cleanups", &self.0.cleanups.borrow().len())
            .field("disposed", &self.0.disposed.get())
            .field("owned_signals", &owned.signals.len())
            .field("owned_memos", &owned.memos.len())
            .field("owned_handlers", &owned.handlers.len())
            .field("owned_effects", &owned.effects.len())
            .finish()
    }
}

// ============================================================================
// Owner
// ============================================================================

/// A non-owning reference to a [`Scope`].
///
/// Holds a [`Weak`], never an `Rc`: an `Owner` is stored in long-lived places
/// (the ambient owner stack, every effect, every memo, every event-handler
/// wrapper), and a strong reference from any of them would keep the scope —
/// and therefore everything it will eventually free — alive forever.
#[derive(Clone)]
pub struct Owner(Weak<ScopeInner>);

impl Owner {
    /// The "no owner" sentinel, which resolves to app lifetime.
    pub(crate) fn none() -> Self {
        Owner(Weak::new())
    }

    /// The ambient owner at this point, or the sentinel if there is none.
    ///
    /// Unlike [`current_owner`], this is total — it never returns `None` — so
    /// callers can store and re-push it unconditionally. That matters: an
    /// `Option<OwnerGuard>` routed through `Option::map` silently discards the
    /// `#[must_use]`, which is exactly how an owner push gets lost.
    pub(crate) fn current() -> Self {
        RUNTIME.with(|rt| {
            rt.borrow()
                .owner_stack
                .last()
                .map(|w| Owner(w.clone()))
                .unwrap_or_else(Owner::none)
        })
    }

    /// Make this owner ambient until the returned guard drops.
    ///
    /// A dead or disposed owner pushes as "no owner" rather than falling
    /// through to the caller's ambient owner. The owner stack is not an
    /// ancestor chain — `run_effect` splices a creation-time owner on top of an
    /// unrelated dispatch-time one — so falling through can attribute a
    /// resource to a scope in a different component tree.
    pub(crate) fn push(&self) -> OwnerGuard {
        push_owner_weak(self.0.clone())
    }

    /// Whether the referenced scope still exists and has not been disposed.
    pub fn is_alive(&self) -> bool {
        self.0.upgrade().is_some_and(|inner| !inner.disposed.get())
    }

    /// Run `f` with this scope as the ambient owner.
    ///
    /// If the scope is already gone or disposed (see [`is_alive`](Owner::is_alive)),
    /// resources created inside `f` get **app lifetime**. They are *not* handed
    /// to the caller's ambient owner: the owner stack is not an ancestor chain —
    /// `run_effect` splices a creation-time owner on top of an unrelated
    /// dispatch-time one — so falling through could attribute a resource to a
    /// scope in a different component tree.
    ///
    /// [`Signal::leak`](super::Signal::leak) is the one deliberate exception: it
    /// *does* look past a dead entry, because it is searching for a record that
    /// already exists rather than choosing where to put a new one.
    pub fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        let _owner = self.push();
        f()
    }

    /// What the referenced scope owns, or `None` if it is gone.
    #[doc(hidden)]
    pub fn owned_counts(&self) -> Option<OwnedCounts> {
        self.0.upgrade().map(|inner| inner.owned.borrow().counts())
    }
}

impl PartialEq for Owner {
    fn eq(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Owner {}

impl std::fmt::Debug for Owner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Owner")
            .field("alive", &self.is_alive())
            .finish()
    }
}

// ============================================================================
// The ambient owner stack
// ============================================================================

/// RAII guard for the ambient owner stack.
///
/// Restores the stack to its pre-push depth on drop, including while unwinding.
#[must_use = "the ambient owner is popped the instant this guard drops — bind it to a named \
              local (`let _owner = ...;`). `let _ = ...` drops it immediately and the push \
              is lost"]
pub struct OwnerGuard {
    restore_len: usize,
    /// The runtime is thread-local — keep the guard `!Send`/`!Sync`, or a guard
    /// moved across threads would truncate the wrong stack.
    _not_send: PhantomData<*const ()>,
}

pub(in crate::reactive) fn push_owner_weak(owner: Weak<ScopeInner>) -> OwnerGuard {
    let restore_len = RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        let depth = rt.owner_stack.len();
        rt.owner_stack.push(owner);
        depth
    });
    OwnerGuard {
        restore_len,
        _not_send: PhantomData,
    }
}

impl Drop for OwnerGuard {
    fn drop(&mut self) {
        // `try_with`: TLS may already be torn down at thread exit, exactly as in
        // `ObserverGuard::drop`. That case is legitimate and silent.
        let in_order = RUNTIME.try_with(|rt| match rt.try_borrow_mut() {
            Ok(mut rt) => {
                let lifo = rt.owner_stack.len() == self.restore_len + 1;
                // `truncate`, not `pop`: restores exactly the pre-push depth, so
                // a lost inner guard is repaired rather than compounded, and it
                // can never panic on an empty stack.
                rt.owner_stack.truncate(self.restore_len);
                lifo
            }
            Err(_) => false,
        });

        // A stranded owner is worse than a stranded observer — every later
        // resource would be attributed to a corpse — so this does not silently
        // swallow the failure. The assert is gated on `!panicking()` because a
        // panic inside a `Drop` during unwind aborts the process, and this guard
        // is live during exactly that unwind in the panic-safety tests.
        if matches!(in_order, Ok(false)) {
            tracing::error!("owner guard dropped out of LIFO order, or RUNTIME was borrowed");
            if !std::thread::panicking() {
                debug_assert!(false, "owner guards must drop in LIFO order (issue #141)");
            }
        }
    }
}

/// The scope that currently owns newly created resources, if any.
///
/// Returns `None` when there is no ambient owner (app lifetime), and also when
/// the ambient entry refers to a scope that has been dropped or disposed — a
/// disposed scope owns nothing further, and the stack is not an ancestor chain,
/// so there is no meaningful owner to fall back to.
pub fn current_owner() -> Option<Owner> {
    let owner = Owner::current();
    owner.is_alive().then_some(owner)
}

/// Run `f` with no ambient owner, so resources created inside it live for the
/// lifetime of the thread.
///
/// The owner-stack counterpart to [`untracked`](super::untracked), which
/// suspends the *observer* stack instead. The two are independent and easy to
/// confuse; picking the wrong one fails silently.
///
/// # Warning
///
/// `unowned` converts a lifetime bug into a leak. If a resource is being freed
/// too early, prefer fixing the ownership — or reading it with
/// [`Signal::try_get`](super::Signal::try_get) /
/// [`Signal::is_alive`](super::Signal::is_alive) — over opting out of
/// reclamation entirely.
pub fn unowned<R>(f: impl FnOnce() -> R) -> R {
    let _owner = Owner::none().push();
    f()
}

/// Run `f` against the live ambient owner's scope, if there is one.
///
/// "Live" is load-bearing in both directions: a dead **or disposed** top entry
/// resolves to *no owner*, and the stack is deliberately not walked down past it
/// — it is not an ancestor chain (see [`Owner::push`]).
///
/// Holds a strong `Rc<ScopeInner>` across `f`, so `f` must stay allocation-only.
/// No user code may run inside it.
fn with_ambient_scope<R>(f: impl FnOnce(&ScopeInner) -> R) -> Option<R> {
    let inner = RUNTIME.with(|rt| rt.borrow().owner_stack.last().and_then(|w| w.upgrade()))?;
    if inner.disposed.get() {
        return None;
    }
    Some(f(&inner))
}

/// Run `f` against the ambient owner's bucket, if there is a live one.
fn with_ambient_owned<R>(f: impl FnOnce(&mut Owned) -> R) -> Option<R> {
    with_ambient_scope(|inner| f(&mut inner.owned.borrow_mut()))
}

/// Walk the owner stack top-down and run `f` against the first live bucket for
/// which it returns `true`. Used by `leak` to detach a resource from whichever
/// ancestor recorded it.
fn with_owner_stack(mut f: impl FnMut(&mut Owned) -> bool) -> bool {
    let stack: Vec<Weak<ScopeInner>> = RUNTIME.with(|rt| rt.borrow().owner_stack.clone());
    for weak in stack.iter().rev() {
        let Some(inner) = weak.upgrade() else {
            continue;
        };
        if inner.disposed.get() {
            continue;
        }
        if f(&mut inner.owned.borrow_mut()) {
            return true;
        }
    }
    false
}

pub(in crate::reactive) fn record_signal(key: SignalKey) {
    with_ambient_owned(|owned| owned.signals.push(key));
}

pub(in crate::reactive) fn record_memo(key: MemoKey) {
    with_ambient_owned(|owned| owned.memos.push(key));
}

pub(in crate::reactive) fn record_effect(id: ObserverId) {
    with_ambient_owned(|owned| owned.effects.push(id));
}

/// Record an event handler against the ambient owner.
///
/// Called from [`crate::events`] at registration time.
pub(crate) fn record_handler(id: EventHandlerId) {
    with_ambient_owned(|owned| owned.handlers.push(id));
}

/// Register a cleanup to run when the scope currently rendering is disposed.
///
/// This is the escape hatch for a **process-lifetime registry that holds a
/// handle to scope-owned state** — a list of active players, a subscription
/// table, anything polled from the event loop. Disposal frees the signals such a
/// handle points at, so the registry has to be told to let go; registering that
/// here ties it to the same lifetime automatically, rather than depending on the
/// component author to remember.
///
/// Cleanups run *before* the scope's signals and memos are freed (see
/// [`run_dispose_fixpoint`]), so a cleanup may still read them.
///
/// Returns `false` when there is no live ambient owner — outside any render, or
/// under [`unowned`] — in which case the caller's resource keeps **app
/// lifetime**, the #141 SD2 default. A dead or disposed top entry also counts as
/// *no owner* and does not fall through to the next entry down: the owner stack
/// is not an ancestor chain (see [`Owner::push`]).
///
/// ```ignore
/// let player = VideoPlayer::new(backend);
/// REGISTRY.with(|r| r.borrow_mut().push(player.clone()));
/// let doomed = player.clone();
/// on_cleanup(move || REGISTRY.with(|r| r.borrow_mut().retain(|p| p != &doomed)));
/// ```
pub fn on_cleanup(f: impl FnOnce() + 'static) -> bool {
    on_cleanup_for_ambient_owner(f)
}

/// Crate-internal spelling of [`on_cleanup`], used by [`crate::context`] to tie a
/// store/context entry to the scope that created it.
pub(crate) fn on_cleanup_for_ambient_owner(f: impl FnOnce() + 'static) -> bool {
    with_ambient_scope(|inner| {
        // The load-bearing guard is `with_ambient_scope` rejecting a *disposed*
        // owner, and that is reachable: `record_effect` declines to record
        // against a disposed ambient owner while `Owner::current` still captures
        // it, so such an effect is never disposed by anyone, and every later run
        // re-pushes its dead owner. A cleanup registered there would never run.
        //
        // `try_borrow_mut` is the belt to that suspenders. It used to be
        // essential — disposal drained this vec with the borrow held across
        // every cleanup — but the fixpoint now moves cleanups out and releases
        // the borrow before running any of them. Kept anyway: degrading to "not
        // registered" beats taking the app down if that pairing is ever broken,
        // matching the lenient-write stance PR1 established.
        match inner.cleanups.try_borrow_mut() {
            Ok(mut cleanups) => {
                cleanups.push(Box::new(f));
                true
            }
            Err(_) => {
                tracing::warn!(
                    "a cleanup was registered while its owning scope was already disposing; \
                     it will not run"
                );
                false
            }
        }
    })
    .unwrap_or(false)
}

/// Detach a signal from whichever owner on the stack recorded it.
///
/// Searches the whole stack, not just the top: a `leak()` written inside a
/// nested `if`/`for`/`match` branch runs with the branch's child scope ambient,
/// while the entry sits in the enclosing scope's bucket. Removing the entry from
/// an ancestor is safe because `(id, generation)` is globally unique per store —
/// the entry found *is* this signal's entry.
pub(in crate::reactive) fn forget_signal(key: SignalKey) -> bool {
    with_owner_stack(|owned| {
        if let Some(pos) = owned.signals.iter().position(|k| *k == key) {
            owned.signals.remove(pos);
            true
        } else {
            false
        }
    })
}

/// Detach a memo from whichever owner on the stack recorded it. See
/// [`forget_signal`].
pub(in crate::reactive) fn forget_memo(key: MemoKey) -> bool {
    with_owner_stack(|owned| {
        if let Some(pos) = owned.memos.iter().position(|k| *k == key) {
            owned.memos.remove(pos);
            true
        } else {
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactive::{Memo, Signal};
    use std::cell::Cell;

    /// The owner stack holds `Weak`, so dropping the last handle to the scope
    /// that is *currently ambient* disposes it right then — not deferred to the
    /// guard's epilogue.
    ///
    /// This is the only test that distinguishes a `Weak` owner stack from an
    /// `Rc` one. With `Rc`, the strong count reads 2 while pushed and the log
    /// comes back `["after-drop", "cleanup", "after-guard"]` — teardown having
    /// silently migrated out of the drop and into the guard.
    #[test]
    fn a_scope_dropped_while_ambient_disposes_immediately() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let scope = Scope::new();
        let cleanup_log = log.clone();
        scope.on_cleanup(move || cleanup_log.borrow_mut().push("cleanup"));

        let mut holder = Some(scope);
        {
            let _owner = holder.as_ref().unwrap().push_owner();
            assert_eq!(
                holder.as_ref().unwrap().strong_count(),
                1,
                "the owner stack must not hold a strong reference"
            );
            holder.take(); // drops the only `Scope` handle
            log.borrow_mut().push("after-drop");
        }
        log.borrow_mut().push("after-guard");

        assert_eq!(
            *log.borrow(),
            ["cleanup", "after-drop", "after-guard"],
            "disposal must happen when the handle drops, not when the guard pops"
        );
    }

    /// Regression anchor for PR2's riskiest single edit: moving `Drop` from
    /// `Scope` to `ScopeInner`. Every other scope test disposes explicitly and
    /// would survive that impl being dropped on the floor.
    #[test]
    fn dropping_a_bare_scope_handle_runs_its_cleanup() {
        let ran = Rc::new(Cell::new(false));
        {
            let scope = Scope::new();
            let flag = ran.clone();
            scope.on_cleanup(move || flag.set(true));
            assert!(!ran.get(), "cleanup must not run early");
        }
        assert!(ran.get(), "dropping the last handle must dispose the scope");
    }

    /// `run` attributes signals, memos and effects — and adopts none of them.
    /// Adoption stays explicit via `add_effect`, because that is what `dispose`
    /// acts on.
    #[test]
    fn run_attributes_resources_but_adopts_nothing() {
        let scope = Scope::new();

        scope.run(|| {
            Signal::new(0);
            Memo::new(|| 1);
            Effect::new(|| {});
        });

        assert_eq!(
            scope.owned_counts(),
            OwnedCounts {
                signals: 1,
                memos: 1,
                handlers: 0,
                effects: 1,
            },
            "everything created inside `run` is attributed to the scope"
        );
        assert_eq!(
            scope.effect_count(),
            0,
            "`run` must not adopt effects — only `add_effect` does"
        );

        // The guard popped: later allocations are not attributed.
        Signal::new(0);
        assert_eq!(
            scope.owned_counts().signals,
            1,
            "the guard must have popped"
        );
    }

    /// The non-breaking-ness contract (#141 SD2): with no ambient owner, a
    /// resource has app lifetime and no scope claims it.
    #[test]
    fn no_ambient_owner_means_app_lifetime() {
        assert!(
            current_owner().is_none(),
            "tests start with an empty owner stack"
        );

        let orphan = Signal::new(7);

        let scope = Scope::new();
        assert_eq!(scope.owned_counts(), OwnedCounts::default());

        scope.dispose();
        assert!(orphan.is_alive(), "an unowned signal outlives any scope");
        assert_eq!(orphan.get(), 7);
    }

    /// `unowned` suspends the owner stack, nests, and — unlike a naive
    /// pop/run/push-back — restores the owner when the body panics.
    #[test]
    fn unowned_suspends_the_owner_and_is_panic_safe() {
        let scope = Scope::new();

        scope.run(|| {
            let owner = current_owner().expect("scope is ambient");

            unowned(|| {
                assert!(current_owner().is_none(), "unowned clears the owner");
                Signal::new(0);
                unowned(|| assert!(current_owner().is_none(), "nesting is a no-op"));
            });
            assert_eq!(current_owner(), Some(owner.clone()), "owner is restored");

            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                unowned(|| panic!("boom"));
            }));
            assert!(panicked.is_err(), "the body must have panicked");
            assert_eq!(
                current_owner(),
                Some(owner),
                "a panic inside `unowned` must not strand the suspension"
            );
        });

        assert_eq!(
            scope.owned_counts().signals,
            0,
            "the signal created under `unowned` belongs to nobody"
        );
    }

    /// A disposed scope owns nothing further, and ownership does **not** fall
    /// through to the next entry down: the owner stack is not an ancestor chain
    /// (`run_effect` splices a creation-time owner onto an unrelated
    /// dispatch-time one), so falling through can cross component trees.
    ///
    /// Reachable in production: `dispose_into_queue` marks a child disposed
    /// while `pending` still holds a strong reference, so the `Weak` upgrades
    /// onto a corpse.
    #[test]
    fn a_disposed_scope_on_the_owner_stack_owns_nothing() {
        let outer = Scope::new();
        let inner = Scope::new();

        let signal = outer.run(|| {
            let _inner_owner = inner.push_owner();
            inner.dispose(); // disposed while still ambient
            assert!(current_owner().is_none(), "a disposed owner is no owner");
            Signal::new(0)
        });

        assert_eq!(inner.owned_counts().signals, 0, "the corpse claims nothing");
        assert_eq!(
            outer.owned_counts().signals,
            0,
            "and ownership must not fall through to the next entry down"
        );
        assert!(signal.is_alive(), "the signal simply has app lifetime");
    }

    /// The cleanup hook `crate::context` uses to tie a store entry to its
    /// creating scope registers against a **live ambient** owner only.
    #[test]
    fn on_cleanup_for_ambient_owner_registers_only_against_a_live_ambient_owner() {
        let fired = Rc::new(Cell::new(0));

        // No ambient owner => not registered (the resource keeps app lifetime).
        assert!(
            !on_cleanup_for_ambient_owner(|| {}),
            "an empty owner stack must not accept a cleanup"
        );

        let scope = Scope::new();
        let hits = fired.clone();
        scope.run(|| {
            assert!(
                on_cleanup_for_ambient_owner(move || hits.set(hits.get() + 1)),
                "a live ambient owner accepts the cleanup"
            );
            // `unowned` suspends the owner stack, so nothing to register against.
            unowned(|| {
                assert!(
                    !on_cleanup_for_ambient_owner(|| {}),
                    "`unowned` must hide the ambient owner from the cleanup hook too"
                );
            });
        });

        assert_eq!(fired.get(), 0, "cleanups do not run before disposal");
        scope.dispose();
        assert_eq!(fired.get(), 1, "the registered cleanup ran on dispose");

        // A disposed top entry counts as no owner, and is NOT walked past.
        let outer = Scope::new();
        let inner = Scope::new();
        outer.run(|| {
            let _inner = inner.push_owner();
            inner.dispose();
            assert!(
                !on_cleanup_for_ambient_owner(|| {}),
                "a disposed ambient owner must not accept a cleanup, and must not \
                 fall through to the live scope beneath it"
            );
        });
    }

    /// `leak` searches the whole owner stack, not just its top.
    ///
    /// A `leak()` written inside a nested branch runs with the branch's child
    /// scope ambient while the entry sits in the enclosing scope's bucket. A
    /// top-of-stack-only implementation leaves `outer` at 1 here — a silent
    /// no-op whose PR4 failure mode is a use-after-free panic.
    #[test]
    fn leak_detaches_from_an_ancestor_owner_not_just_the_innermost() {
        let outer = Scope::new();
        let signal = outer.run(|| Signal::new(0));
        assert_eq!(outer.owned_counts().signals, 1);

        let inner = Scope::new();
        outer.run(|| {
            inner.run(|| {
                // Decoy. Without it the innermost bucket is empty, so an
                // implementation that removed "the first entry it found"
                // instead of matching on `(id, generation)` would fall through
                // to `outer` and pass by luck.
                Signal::new(99);
                let _ = signal.leak();
            })
        });

        assert_eq!(
            outer.owned_counts().signals,
            0,
            "leak must reach an owner further down the stack"
        );
        assert_eq!(
            inner.owned_counts().signals,
            1,
            "and must match on the key rather than raiding the nearest bucket"
        );
        assert!(signal.is_alive());
    }

    /// `leak` with no ambient owner is a quiet no-op, and it works for memos.
    #[test]
    fn leak_is_quiet_without_an_owner_and_works_for_memos() {
        // The defensive `Signal::new(x).leak()` idiom must stay noise-free:
        // 30+ in-tree constructors run outside any render.
        let orphan = Signal::new(5).leak();
        assert_eq!(orphan.get(), 5);

        let scope = Scope::new();
        let source = Signal::new(2);

        // A memo created under the scope and left alone is owned by it...
        scope.run(|| Memo::new(move || source.get() * 2));
        assert_eq!(scope.owned_counts().memos, 1);

        // ...but `leak`, called in the same render, detaches it. Calling it
        // later would be a no-op: `leak` searches the owner stack as it stands,
        // and outside a render that stack is empty.
        let doubled = scope.run(|| Memo::new(move || source.get() * 2).leak());
        assert_eq!(
            scope.owned_counts().memos,
            1,
            "the leaked memo is not added; only the first one is owned"
        );

        // Detaching must not disturb the memo itself — the marker effect is
        // still subscribed, so invalidation still propagates.
        assert_eq!(doubled.get(), 4);
        source.set(10);
        assert_eq!(doubled.get(), 20, "the dirty marker still fires");
        assert!(doubled.is_alive());
    }

    /// The headline of #141: disposing a scope releases every kind of resource
    /// attributed to it, not just the effects explicitly adopted into it.
    #[test]
    fn dispose_frees_everything_the_scope_owns() {
        let scope = Scope::new();
        let (signal, memo, handler) = scope.run(|| {
            let s = Signal::new(1);
            let m = Memo::new(|| 2);
            Effect::new(|| {});
            let h = crate::events::register_handler(std::rc::Rc::new(|| {}));
            (s, m, h)
        });
        assert_eq!(
            scope.owned_counts(),
            OwnedCounts {
                signals: 1,
                memos: 1,
                handlers: 1,
                effects: 1,
            }
        );
        assert!(signal.is_alive());
        assert!(memo.is_alive());
        assert!(crate::events::dispatch_event(handler));

        scope.dispose();

        assert_eq!(
            scope.owned_counts(),
            OwnedCounts::default(),
            "the bucket is emptied as it is released"
        );
        assert!(!signal.is_alive(), "the signal slot is freed");
        assert!(!memo.is_alive(), "the memo slot is freed");
        assert!(
            !crate::events::dispatch_event(handler),
            "the handler is deregistered"
        );
    }

    /// The leak #141 was filed for: an effect created during a render but never
    /// handed to [`Scope::add_effect`] used to run for the life of the thread.
    ///
    /// Ownership catches it even though adoption did not.
    #[test]
    fn an_effect_nobody_adopted_still_dies_with_its_owner() {
        let trigger = Signal::new(0);
        let runs = Rc::new(Cell::new(0));

        let scope = Scope::new();
        let hits = runs.clone();
        scope.run(|| {
            // Deliberately dropped on the floor — no `add_effect`.
            Effect::new(move || {
                trigger.get();
                hits.set(hits.get() + 1);
            });
        });
        assert_eq!(runs.get(), 1, "ran once at creation");
        assert_eq!(scope.effect_count(), 0, "and was never adopted");

        trigger.set(1);
        assert_eq!(runs.get(), 2);

        scope.dispose();

        trigger.set(2);
        assert_eq!(
            runs.get(),
            2,
            "a fire-and-forget effect must not outlive the scope that created it"
        );
    }

    /// Why disposal has to be a **fixpoint** rather than one pass down the
    /// priority list.
    ///
    /// A scope's subtree is not enumerable up front: `child` here is reachable
    /// only through the cleanup closure, so it is not marked disposed until
    /// cleanups run — which is *after* the handler level. A single pass would
    /// leave its handler registered forever; the fixpoint loops back to the top
    /// and catches it.
    #[test]
    fn a_scope_reached_only_by_a_cleanup_still_has_its_handlers_freed() {
        let outer = Scope::new();

        let child = Scope::new();
        let handler = child.run(|| crate::events::register_handler(std::rc::Rc::new(|| {})));
        assert!(crate::events::dispatch_event(handler));

        // The only handle to `child` lives inside the cleanup closure, so the
        // handler cannot be discovered before cleanups run.
        let mut held = Some(child);
        outer.on_cleanup(move || drop(held.take()));

        outer.dispose();

        assert!(
            !crate::events::dispatch_event(handler),
            "the fixpoint must revisit the handler level after cleanups ran"
        );
    }

    /// A grandchild scope reachable *only* through an effect closure is still
    /// fully released.
    ///
    /// This is the shape every control-flow construct produces: `show_dom`,
    /// `match_dom`, `for_each_dom` and `reactive_component_dom` all keep their
    /// child scope inside the closure of the effect that swaps it. So the child
    /// is not enumerable when the parent is collected — it only becomes
    /// reachable once that effect is disposed and its closure dropped, one level
    /// down the fixpoint.
    ///
    /// Without this, `dispose_frees_everything_the_scope_owns` is trivially
    /// satisfiable by a single flat pass and proves nothing about the fixpoint.
    #[test]
    fn a_grandchild_reachable_only_through_an_effect_closure_is_released() {
        let outer = Scope::new();

        // Two levels down, each hidden inside the closure above it.
        let child = Scope::new();
        let grandchild = Scope::new();
        let deep_signal = grandchild.run(|| Signal::new(41));
        let deep_handler = grandchild.run(|| crate::events::register_handler(Rc::new(|| {})));

        // The grandchild handle lives *inside* an effect closure owned by
        // `child`; nothing else references it. Same for `child` one level up.
        let holder = child.run(|| {
            let held = grandchild;
            Effect::new(move || {
                let _ = &held;
            })
        });
        child.add_effect(holder);

        let outer_holder = outer.run(|| {
            let held = child;
            Effect::new(move || {
                let _ = &held;
            })
        });
        outer.add_effect(outer_holder);

        assert!(deep_signal.is_alive());
        assert!(crate::events::dispatch_event(deep_handler));

        outer.dispose();

        assert!(
            !deep_signal.is_alive(),
            "the fixpoint must reach two levels of effect-closure indirection"
        );
        assert!(
            !crate::events::dispatch_event(deep_handler),
            "including levels above the one that discovered the scope"
        );
    }

    /// A panicking cleanup must not disable disposal for the rest of the
    /// thread's life.
    ///
    /// The fixpoint installs a thread-local context and uses "is it installed?"
    /// to tell an outermost `dispose` from a nested one. Clearing it only on the
    /// normal path would leave it stranded after a panic, and every later
    /// `dispose()` would then collect into it and never drain — silently, and
    /// for every scope in the process, not just this one.
    #[test]
    fn a_panicking_cleanup_does_not_strand_the_disposal_context() {
        let scope = Scope::new();
        scope.on_cleanup(|| panic!("boom"));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scope.dispose()));
        assert!(result.is_err(), "the cleanup must have panicked");

        // The positive control: a completely unrelated scope disposed
        // afterwards still frees what it owns.
        let later = Scope::new();
        let signal = later.run(|| Signal::new(0));
        assert!(signal.is_alive());

        later.dispose();

        assert!(
            !signal.is_alive(),
            "disposal must still work after a cleanup panicked in an earlier scope"
        );
    }

    /// A cleanup runs after the scope's effects are disposed, so it cannot
    /// resurrect one — the ordering ratified for #141 (SD4).
    #[test]
    fn a_cleanup_cannot_resurrect_a_disposed_effect() {
        let trigger = Signal::new(0);
        let runs = Rc::new(Cell::new(0));

        let scope = Scope::new();
        let hits = runs.clone();
        let effect = scope.run(|| {
            Effect::new(move || {
                trigger.get();
                hits.set(hits.get() + 1);
            })
        });
        scope.add_effect(effect);
        assert_eq!(runs.get(), 1);

        // Writing a signal from a cleanup flushes effects synchronously.
        scope.on_cleanup(move || trigger.set(1));
        scope.dispose();

        assert_eq!(
            runs.get(),
            1,
            "the effect was already disposed when the cleanup wrote its dependency"
        );
    }

    /// Cleanups run *before* the scope's signals and memos are freed, so a
    /// cleanup can still read the state it is tearing down.
    ///
    /// This is what makes "save my draft on unmount" expressible at all.
    #[test]
    fn a_cleanup_can_still_read_the_signals_it_is_tearing_down() {
        let saved = Rc::new(RefCell::new(String::new()));

        let scope = Scope::new();
        let draft = scope.run(|| Signal::new(String::from("unsent")));

        let sink = saved.clone();
        scope.on_cleanup(move || *sink.borrow_mut() = draft.get());

        scope.dispose();

        assert_eq!(*saved.borrow(), "unsent", "the read must not have panicked");
        assert!(!draft.is_alive(), "and the signal is freed afterwards");
    }

    /// A signal's value is dropped with no store borrow held, and only once
    /// every sibling has been freed — so a `Drop` that touches the reactive
    /// stores neither panics nor observes a half-torn-down scope.
    #[test]
    fn freed_values_drop_outside_the_store_borrow_and_after_their_siblings() {
        struct Probe {
            sibling: Signal<i32>,
            sibling_was_alive: Rc<Cell<bool>>,
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                // Touching SIGNAL_STORE here is a BorrowMutError if the value
                // is dropped in place inside `SignalStore::free`.
                self.sibling_was_alive.set(self.sibling.is_alive());
            }
        }

        let sibling_was_alive = Rc::new(Cell::new(true));
        let scope = Scope::new();
        let flag = sibling_was_alive.clone();
        scope.run(|| {
            let sibling = Signal::new(0);
            Signal::new(Probe {
                sibling,
                sibling_was_alive: flag,
            });
        });

        scope.dispose(); // must not panic

        assert!(
            !sibling_was_alive.get(),
            "value drops are deferred until every signal the scope owned is gone"
        );
    }

    /// Disposal is re-entrant: a cleanup may dispose another scope, and that
    /// scope's own work joins the fixpoint already in flight instead of
    /// starting a nested one.
    #[test]
    fn a_cleanup_may_dispose_another_scope() {
        let log = Rc::new(RefCell::new(Vec::new()));

        let inner = Scope::new();
        let inner_signal = inner.run(|| Signal::new(0));
        let inner_log = log.clone();
        inner.on_cleanup(move || inner_log.borrow_mut().push("inner"));

        let outer = Scope::new();
        let outer_log = log.clone();
        outer.on_cleanup(move || {
            outer_log.borrow_mut().push("outer");
            inner.dispose();
        });

        outer.dispose();

        assert_eq!(*log.borrow(), ["outer", "inner"]);
        assert!(
            !inner_signal.is_alive(),
            "the nested scope was fully released"
        );
    }

    /// Freeing a memo releases its dirty-marker effect too, so writing a source
    /// afterwards neither resurrects its dependents nor panics.
    ///
    /// The distinction between `free_memo` and `MemoStore::free`, pinned at the
    /// level that actually calls it.
    #[test]
    fn disposing_a_scope_releases_a_memos_marker_effect_as_well() {
        let source = Signal::new(1);
        let scope = Scope::new();
        let memo = scope.run(|| Memo::new(move || source.get() * 2));
        assert_eq!(memo.get(), 2);

        let runs = Rc::new(Cell::new(0));
        let hits = runs.clone();
        // Lives outside the scope, so it survives the dispose below.
        let _dependent = Effect::new(move || {
            source.get();
            hits.set(hits.get() + 1);
        });
        assert_eq!(runs.get(), 1);

        scope.dispose();
        source.set(5); // must not panic

        assert!(!memo.is_alive());
        assert_eq!(runs.get(), 2, "the surviving observer still runs");
    }
}
