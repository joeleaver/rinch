//! [`Owner`], a non-owning reference to a scope, and the ambient owner
//! stack: [`OwnerGuard`], [`current_owner`] and [`unowned`].

use std::marker::PhantomData;
use std::rc::Weak;

use super::OwnedCounts;
use super::handle::ScopeInner;
use crate::reactive::RUNTIME;

// ============================================================================
// Owner
// ============================================================================

/// A non-owning reference to a [`Scope`](super::Scope).
///
/// Holds a [`Weak`], never an `Rc`: an `Owner` is stored in long-lived places
/// (the ambient owner stack, every effect, every memo, every event-handler
/// wrapper), and a strong reference from any of them would keep the scope —
/// and therefore everything it will eventually free — alive forever.
#[derive(Clone)]
pub struct Owner(pub(super) Weak<ScopeInner>);

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
    /// [`Signal::leak`](crate::reactive::Signal::leak) is the one deliberate exception: it
    /// *does* look past a dead entry, because it is searching for a record that
    /// already exists rather than choosing where to put a new one.
    pub fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        let _owner = self.push();
        f()
    }

    /// An identity for the referenced scope, stable for as long as this
    /// `Owner` (or any clone of it) exists: the `Weak` keeps the allocation, so
    /// the address is not reused while a key taken from it is still held.
    pub(crate) fn key(&self) -> usize {
        self.0.as_ptr() as usize
    }

    /// What the referenced scope owns, or `None` if it is gone.
    #[doc(hidden)]
    pub fn owned_counts(&self) -> Option<OwnedCounts> {
        self.0.upgrade().map(|inner| inner.owned.borrow().counts())
    }

    /// Everything the referenced scope holds that code run under it can add
    /// to — owned resources, adopted effects, child scopes and cleanups (a
    /// spawned task registers one) — or `None` if it is gone. Two equal
    /// footprints taken around some code mean it created nothing under this
    /// owner (issue #3030).
    pub(in crate::reactive) fn footprint(&self) -> Option<Footprint> {
        self.0.upgrade().map(|inner| Footprint {
            owned: inner.owned.borrow().counts(),
            effects: inner.effects.borrow().len(),
            children: inner.children.borrow().len(),
            cleanups: inner.cleanups.borrow().len(),
        })
    }
}

/// What [`Owner::footprint`] answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::reactive) struct Footprint {
    owned: OwnedCounts,
    effects: usize,
    children: usize,
    cleanups: usize,
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
        // TLS already torn down at thread exit is legitimate and silent
        // (`Unrestored::TlsGone`); a borrowed runtime is logged by the helper
        // and asserted on below.
        let restore_len = self.restore_len;
        let in_order = crate::reactive::restore::with_runtime_on_drop("the owner stack", |rt| {
            let lifo = rt.owner_stack.len() == restore_len + 1;
            // `truncate`, not `pop`: restores exactly the pre-push depth, so
            // a lost inner guard is repaired rather than compounded, and it
            // can never panic on an empty stack.
            rt.owner_stack.truncate(restore_len);
            lifo
        });

        // A stranded owner is worse than a stranded observer — every later
        // resource would be attributed to a corpse — so this does not silently
        // swallow the failure. The assert is gated on `!panicking()` because a
        // panic inside a `Drop` during unwind aborts the process, and this guard
        // is live during exactly that unwind in the panic-safety tests.
        if matches!(
            in_order,
            Ok(false) | Err(crate::reactive::restore::Unrestored::Borrowed)
        ) {
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
/// The owner-stack counterpart to [`untracked`](crate::reactive::untracked), which
/// suspends the *observer* stack instead. The two are independent and easy to
/// confuse; picking the wrong one fails silently.
///
/// # Warning
///
/// `unowned` converts a lifetime bug into a leak. If a resource is being freed
/// too early, prefer fixing the ownership — or reading it with
/// [`Signal::try_get`](crate::reactive::Signal::try_get) /
/// [`Signal::is_alive`](crate::reactive::Signal::is_alive) — over opting out of
/// reclamation entirely.
pub fn unowned<R>(f: impl FnOnce() -> R) -> R {
    let _owner = Owner::none().push();
    f()
}
