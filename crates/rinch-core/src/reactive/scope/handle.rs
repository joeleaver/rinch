//! The [`Scope`] handle, its reference-counted body [`ScopeInner`], and the
//! cleanups registered on it ([`CleanupHandle`] takes a removable one back).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use super::dispose::with_dispose_ctx;
use super::owned::Owned;
use super::owner::push_owner_weak;
use super::{OwnedCounts, Owner, OwnerGuard};
use crate::reactive::Effect;

// ============================================================================
// Scope
// ============================================================================

/// A scope that manages the lifetime of reactive primitives.
///
/// Disposing a scope releases everything it owns and everything its child
/// scopes own — event handlers, effects, cleanups, memos, signals — and
/// dropping the scope does the same thing. See `run_dispose_fixpoint` for the
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
pub struct Scope(pub(super) Rc<ScopeInner>);

/// The reference-counted body of a [`Scope`].
///
/// Split out so that an [`Owner`] can hold a non-owning [`Weak`] reference to a
/// live scope from inside the ambient owner stack, an `EffectInner`, a
/// `MemoInner`, or an event-handler wrapper — all of which must observe a scope
/// without keeping it alive.
pub(in crate::reactive) struct ScopeInner {
    pub(super) effects: RefCell<Vec<Effect>>,
    pub(super) children: RefCell<Vec<Scope>>,
    pub(super) cleanups: RefCell<Vec<Cleanup>>,
    pub(super) disposed: Cell<bool>,
    pub(super) owned: RefCell<Owned>,
}

/// A registered cleanup: its closure, and the key a [`CleanupHandle`] removes
/// it by (`0` for one registered with no handle). One list for both, so every
/// cleanup runs in the order it was registered.
pub(super) struct Cleanup {
    pub(super) key: u64,
    pub(super) run: Box<dyn FnOnce()>,
}

thread_local! {
    /// The last key given to a removable cleanup; `0` is never given.
    pub(super) static NEXT_CLEANUP_KEY: Cell<u64> = const { Cell::new(0) };
}

/// A cleanup registered with [`on_cleanup_removable`](super::on_cleanup_removable), which can be taken back
/// before its owner runs it.
///
/// Holds a weak reference to the owner, so it keeps nothing alive. `!Send`:
/// scopes are the UI thread's.
#[must_use = "dropping the handle keeps the cleanup; call `remove` to take it back"]
pub struct CleanupHandle {
    pub(super) owner: Weak<ScopeInner>,
    pub(super) key: u64,
}

impl CleanupHandle {
    /// Take the cleanup back: it never runs, and its closure (with whatever
    /// it captured) is dropped now. Removing it while its owner is being
    /// disposed, before it has run (from another cleanup of that disposal),
    /// still keeps it from running. Once the owner is gone, or its disposal
    /// has finished, there is nothing to remove and this does nothing.
    pub fn remove(self) {
        let removed = self.owner.upgrade().and_then(|inner| {
            let mut cleanups = inner.cleanups.try_borrow_mut().ok()?;
            let at = cleanups.iter().position(|c| c.key == self.key)?;
            Some(cleanups.remove(at))
        });
        if removed.is_none() {
            // Already moved into an in-flight disposal, or gone.
            with_dispose_ctx(|ctx| {
                if let Some(at) = ctx.cleanups.iter().position(|c| c.key == self.key) {
                    ctx.cleanups.remove(at);
                } else {
                    // In the batch running now: skipped when its turn comes.
                    // Its owner can be gone and the cleanup still due: a
                    // child scope is dropped as the disposal collects it,
                    // while its cleanups wait in the batch. So a key is
                    // marked even when no batch may hold it (a handle whose
                    // owner went long ago, removed during an unrelated
                    // disposal); such a stray goes with this disposal's
                    // context when it ends (`DisposeCtxGuard`).
                    ctx.removed.push(self.key);
                }
            });
        }
        // The closure drops here, outside every borrow: its captures' `Drop`s
        // may run user code.
        drop(removed);
    }
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
    /// **attributed** to this scope, and [`dispose`](Scope::dispose) frees every
    /// one of them (issue #141) — including a fire-and-forget effect whose
    /// handle was dropped on the spot. [`add_effect`](Scope::add_effect) is the
    /// separate, manual path, for an effect built *outside* the scope that
    /// should still die with it.
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
    /// wakes only observers outside the tree coming down — and not the effect
    /// doing the disposing, if an effect's body is what called `dispose`: a
    /// running effect does not wake itself (see [`Effect`],
    /// issue #343). They run **before** the scope's memos and signals are
    /// freed, so a cleanup may still read them. See `run_dispose_fixpoint`.
    ///
    /// A cleanup registered by another cleanup on the same scope still runs:
    /// the fixpoint takes the list out before running any of it, then loops.
    pub fn on_cleanup<F: FnOnce() + 'static>(&self, f: F) {
        self.0.cleanups.borrow_mut().push(Cleanup {
            key: 0,
            run: Box::new(f),
        });
    }

    /// How many cleanups are registered and not yet run: for tests of what an
    /// owner keeps (a long-lived owner should keep no cleanup of a thing that
    /// ended, [`on_cleanup_removable`]).
    #[doc(hidden)]
    pub fn cleanup_count(&self) -> usize {
        self.0.cleanups.borrow().len()
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

impl Default for Scope {
    fn default() -> Self {
        Self::new()
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
