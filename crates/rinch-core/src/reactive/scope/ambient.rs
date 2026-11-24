//! Recording against the ambient owner: the `record_*` calls that attribute
//! a new resource to it, the `forget_*` calls `leak` uses to detach one, and
//! [`on_cleanup`] / [`on_cleanup_removable`].

use std::rc::Weak;

use super::handle::{Cleanup, CleanupHandle, NEXT_CLEANUP_KEY, ScopeInner};
use super::owned::Owned;
use super::{MemoKey, SignalKey};
use crate::events::EventHandlerId;
use crate::reactive::{ObserverId, RUNTIME};

/// Run `f` against the live ambient owner's scope, if there is one.
///
/// "Live" is load-bearing in both directions: a dead **or disposed** top entry
/// resolves to *no owner*, and the stack is deliberately not walked down past it
/// — it is not an ancestor chain (see [`Owner::push`](super::Owner::push)).
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
/// `run_dispose_fixpoint`), so a cleanup may still read them.
///
/// A cleanup that writes a signal wakes that signal's observers — except an
/// effect whose own body is disposing the scope, which does not re-run for it
/// (see [`Effect`](crate::reactive::Effect), issue #343).
///
/// Returns `false` when there is no live ambient owner — outside any render, or
/// under [`unowned`](super::unowned) — in which case the caller's resource keeps **app
/// lifetime**, the #141 SD2 default. A dead or disposed top entry also counts as
/// *no owner* and does not fall through to the next entry down: the owner stack
/// is not an ancestor chain (see `Owner::push`).
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

/// [`on_cleanup`], with a handle to take the cleanup back before it runs.
///
/// For something an owner would otherwise collect one of per use: a long-lived
/// owner (an app-lifetime component, a tray icon, a list row) that opens a
/// window, starts a request or subscribes to something on every click, and
/// whose cleanup is only needed while that thing is still there. With
/// [`on_cleanup`] each use leaves its closure on the owner until the owner
/// goes; with this one the thing removes its own cleanup when it ends first
/// ([`CleanupHandle::remove`]), and the owner keeps nothing of it.
///
/// It runs in its place among the owner's other cleanups, in registration
/// order. Returns `None`, registering nothing, where [`on_cleanup`] answers
/// `false`: no live ambient owner.
pub fn on_cleanup_removable(f: impl FnOnce() + 'static) -> Option<CleanupHandle> {
    let owner = RUNTIME.with(|rt| rt.borrow().owner_stack.last().cloned())?;
    let inner = owner.upgrade()?;
    if inner.disposed.get() {
        return None;
    }
    let key = NEXT_CLEANUP_KEY.with(|k| {
        let key = k.get() + 1;
        k.set(key);
        key
    });
    let Ok(mut cleanups) = inner.cleanups.try_borrow_mut() else {
        tracing::warn!(
            "a cleanup was registered while its owning scope was already disposing; \
             it will not run"
        );
        return None;
    };
    cleanups.push(Cleanup {
        key,
        run: Box::new(f),
    });
    Some(CleanupHandle { owner, key })
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
                cleanups.push(Cleanup {
                    key: 0,
                    run: Box::new(f),
                });
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
