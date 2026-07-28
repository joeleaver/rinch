//! Scope: manages the lifetime of reactive primitives.
//!
//! # Ownership is ambient
//!
//! A [`Scope`] is both a disposal list (effects, cleanups, child scopes) and an
//! **owner**: while a scope is pushed as the ambient owner, every [`Signal`],
//! [`Memo`], [`Effect`] and event handler created is *attributed* to it.
//!
//! Attribution is recorded, not enforced — nothing is freed yet. Issue #141's
//! later PRs turn these records into actual reclamation; this layer exists so
//! that when they do, the answer to "who owns this?" is already there.
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
/// Write-only bookkeeping today. Every field is plain data, so dropping an
/// `Owned` runs no user code and cannot re-enter the disposal machinery.
#[derive(Default)]
pub(in crate::reactive) struct Owned {
    signals: Vec<SignalKey>,
    memos: Vec<MemoKey>,
    handlers: Vec<EventHandlerId>,
    /// Effects *created* under this owner.
    ///
    /// Deliberately distinct from [`ScopeInner::effects`], which holds effects
    /// *adopted* via [`Scope::add_effect`] and is what `dispose` acts on. The
    /// difference between the two counts is the number of fire-and-forget
    /// effects nothing will ever dispose.
    effects: Vec<ObserverId>,
}

/// A snapshot of what a scope owns.
///
/// Introspection for tests and diagnostics: because nothing is freed yet, these
/// counts are the only observable evidence that ownership is being tracked.
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
/// When a scope is disposed, all effects **adopted** into it (via
/// [`add_effect`](Scope::add_effect)) are disposed, its cleanups run, and its
/// child scopes are disposed too. Dropping the scope does the same thing.
///
/// Separately, a scope can be made the *ambient owner* — see
/// [`run`](Scope::run) and the module docs. Ownership currently only records
/// attribution; it does not free anything.
///
/// # Example
///
/// ```ignore
/// let scope = Scope::new();
///
/// scope.run(|| {
///     let signal = Signal::new(0);   // attributed to this scope
///     // Effects are attributed too, but adoption is explicit:
///     scope.add_effect(Effect::new(|| { /* ... */ }));
/// });
///
/// scope.dispose(); // Disposes adopted effects, runs cleanups, disposes children
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
    /// Today cleanups run *before* this scope's child scopes are processed, and
    /// before its effects are actually disposed — disposal only enqueues them.
    /// **Do not depend on either**: issue #141's PR4 reorders disposal to
    /// handlers → effects → cleanups so that a cleanup cannot resurrect a
    /// disposed effect.
    ///
    /// A cleanup must not register another cleanup on the *same* scope: the
    /// cleanup list is borrowed for the duration of the drain that runs it.
    pub fn on_cleanup<F: FnOnce() + 'static>(&self, f: F) {
        self.0.cleanups.borrow_mut().push(Box::new(f));
    }

    /// Dispose of all effects, child scopes, and run cleanup functions.
    ///
    /// After dispose, this scope should not be used.
    pub fn dispose(&self) {
        self.0.dispose();
    }

    /// Clear all adopted effects without disposing them.
    ///
    /// # Warning
    ///
    /// This moves only the *adopted effect* list. The scope's owned-resource
    /// records (signals, memos, handlers) stay behind, so the effects and the
    /// resources they close over end up with different owners.
    pub fn take_effects(&self) -> Vec<Effect> {
        self.0.effects.borrow_mut().drain(..).collect()
    }

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
    fn dispose(&self) {
        if self.disposed.get() {
            return;
        }

        // Use a thread-local disposal queue to avoid stack overflow.
        //
        // The problem: Effect closures may capture Rc<RefCell<RenderScope>>,
        // so Effect::dispose() -> drops closure -> drops RenderScope -> Scope::drop
        // -> dispose() -> more Effect::dispose() -- creating unbounded recursion.
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
                let batch: Vec<Effect> =
                    DISPOSE_QUEUE.with(|q| std::mem::take(q.borrow_mut().as_mut().unwrap()));
                if batch.is_empty() {
                    break;
                }
                // Disposing effects may drop closures that own RenderScopes,
                // triggering more Scope::drop -> dispose() calls. Those nested
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
    fn dispose_into_queue(
        &self,
        queue: &'static std::thread::LocalKey<RefCell<Option<Vec<Effect>>>>,
    ) {
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

        // #141 PR4: release `self.owned` here, in priority order
        // (handlers -> effects -> cleanups -> memos -> signals -> value-drops).
        // PR2 deliberately leaves the bucket intact so the attribution stays
        // observable after disposal.

        // Process children iteratively
        let mut pending: Vec<Scope> = self.children.borrow_mut().drain(..).collect();
        while let Some(child) = pending.pop() {
            if child.0.disposed.get() {
                // Drain to prevent recursive field drop
                child.0.children.borrow_mut().drain(..);
                child.0.effects.borrow_mut().drain(..);
                continue;
            }
            child.0.disposed.set(true);

            pending.extend(child.0.children.borrow_mut().drain(..));
            let child_effects: Vec<Effect> = child.0.effects.borrow_mut().drain(..).collect();
            queue.with(|q| {
                if let Some(ref mut vec) = *q.borrow_mut() {
                    vec.extend(child_effects);
                }
            });

            for cleanup in child.0.cleanups.borrow_mut().drain(..) {
                cleanup();
            }

            // #141 PR4: release `child.0.owned` here too.
        }
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

/// Register a cleanup to run when the ambient owner is disposed.
///
/// Returns `false` when there is no live ambient owner, in which case the
/// caller's resource keeps **app lifetime** — the #141 SD2 default. Like
/// [`with_ambient_owned`], a dead or disposed top entry counts as *no owner*
/// and does not fall through to the next entry down: the owner stack is not an
/// ancestor chain.
///
/// Called from [`crate::context`] to tie a store/context entry to the scope
/// that created it.
pub(crate) fn on_cleanup_for_ambient_owner(f: impl FnOnce() + 'static) -> bool {
    with_ambient_scope(|inner| {
        // `try_borrow_mut`, not `borrow_mut`: `dispose_into_queue` drains this
        // vec with the borrow held across every cleanup it runs, so reaching a
        // mid-drain scope here would panic.
        //
        // Reaching one is not easy — `dispose` pushes no owner, so the stack is
        // normally empty by then — but it is possible: a cleanup writes a
        // signal, the synchronous flush re-runs an effect that is
        // enqueued-but-not-yet-disposed, and `run_effect` re-pushes that
        // effect's owner, which is the disposing scope.
        //
        // Two guards cover it and either alone suffices: `with_ambient_scope`
        // rejects a disposed owner (`disposed` is set before the drain begins),
        // and this `try_borrow_mut`. Kept redundant deliberately — degrading to
        // "not registered" beats taking the app down, matching the lenient-write
        // stance PR1 established.
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

    /// PR2 is metrics-only: `dispose` must not free or drain anything.
    #[test]
    fn dispose_leaves_the_owned_bucket_intact() {
        let scope = Scope::new();
        let (signal, memo, handler) = scope.run(|| {
            let s = Signal::new(1);
            let m = Memo::new(|| 2);
            Effect::new(|| {});
            let h = crate::events::register_handler(std::rc::Rc::new(|| {}));
            (s, m, h)
        });
        let before = scope.owned_counts();
        assert_eq!(before.signals, 1);
        assert_eq!(before.memos, 1);
        assert_eq!(before.effects, 1);
        assert_eq!(before.handlers, 1);

        scope.dispose();

        assert_eq!(
            scope.owned_counts(),
            before,
            "PR2 records ownership; PR4 is what acts on it"
        );
        // The counts alone cannot detect a free, because PR2 does not drain the
        // bucket on dispose — so probe the resources themselves.
        assert!(signal.is_alive(), "nothing is freed yet");
        assert!(memo.is_alive(), "nothing is freed yet");
        assert!(
            crate::events::dispatch_event(handler),
            "the handler is still registered — PR4 is what unregisters it"
        );
    }
}
