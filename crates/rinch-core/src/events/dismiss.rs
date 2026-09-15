//! The dismiss stack: who gets Escape, and in what order (issue #474).
//!
//! An overlay that closes on Escape — a `Modal`, a `Drawer`, a `Popover` — has
//! to answer a key nobody focused it to receive. The obvious home for that is
//! [`set_keyboard_interceptor`](super::set_keyboard_interceptor), and it is the
//! wrong shape: that is **one slot per document**, last-registration-wins. Two
//! open modals and the inner one replaces the outer; when the inner unmounts
//! its cleanup *removes* the slot rather than restoring what it displaced, and
//! the outer modal's Escape is dead for the rest of the session.
//!
//! So this is a stack, scanned **back to front**. Each entry answers "is this
//! mine?" at dispatch time by returning `true` (consumed) or `false` (not
//! open — pass it on), and the first `true` wins. Nesting then works by
//! construction: the most recently mounted overlay is asked first.
//!
//! ## The three rules, and why each is here
//!
//! - **The open check happens at dispatch, not at registration.** `Modal::render`
//!   runs *once*; `opened_fn` only toggles a class. A closed modal stays
//!   mounted, so an entry registered "because the modal is open" would go on
//!   swallowing Escape long after it closed. Every handler is asked afresh.
//! - **Liveness beside the callback, checked at dispatch.** This registry is
//!   written repeatedly from live components, which is the shape CLAUDE.md
//!   prescribes the owner-beside-the-callback template for (the one
//!   [`park_main_callback`](crate::main_thread::park_main_callback), `rinch-ws`'s
//!   handler map and the menu registry use) rather than a doc-scoped slot. A
//!   dead owner's handler closes over signals its component already freed, so
//!   it is dropped rather than run. The [`DismissHandle`] is the *eager* release;
//!   liveness is what covers a handle that was leaked or outlived by its state.
//! - **Two *marked* documents do not answer each other's Escape.** Two
//!   `RinchContext`s on one thread share every thread-local here (#134/#139), so
//!   an entry carries the document it was registered against and dispatch
//!   applies [`doc_matches`], whose rule this is: only two `Some` keys that
//!   differ are refused. Everything else is permissive by construction, which
//!   is what keeps a registration made with no document (a test, a headless
//!   host) working, and what keeps `rinch-web` — which marks no dispatching
//!   document at all, so two islands on one page share one stack — working
//!   too.
//!
//! Dispatch is wired into
//! [`dispatch_keyboard_event`](super::dispatch_keyboard_event), *after* the
//! interceptor and only for an Escape **press**. That placement is what makes
//! this backend-agnostic: desktop and rinch-web both already call that function,
//! and neither needed a line changed.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::context::{current_dispatching_doc, doc_identity, doc_matches};
use crate::reactive::{Owner, current_owner};

/// A registered dismiss handler: `true` means "I consumed the key".
type DismissFn = Rc<dyn Fn() -> bool>;

struct Entry {
    id: u64,
    /// The document this handler belongs to, or `None` for one registered
    /// against no document in particular. See [`doc_matches`].
    doc: Option<u64>,
    /// The scope that was rendering when this was registered, if any.
    owner: Option<Owner>,
    handler: DismissFn,
}

thread_local! {
    /// Registration order, oldest first. Dispatch reads it backwards.
    static STACK: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
}

/// A live entry in the dismiss stack. The handler is released when this drops.
///
/// `#[must_use]` because dropping it immediately unregisters the handler that
/// was just pushed, which looks exactly like a working registration and is not
/// one. Store it for as long as the overlay is mounted — a component's
/// [`RenderScope::on_cleanup`](crate::dom::RenderScope::on_cleanup) is the
/// usual home:
///
/// ```ignore
/// let handle = push_dismiss_handler(root.doc_key(), move || { … });
/// __scope.on_cleanup(move || drop(handle));
/// ```
#[must_use = "the handler is released the instant this handle drops — store it for as long as \
              the overlay is mounted, e.g. `__scope.on_cleanup(move || drop(handle))`"]
pub struct DismissHandle(u64);

impl std::fmt::Debug for DismissHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DismissHandle").field(&self.0).finish()
    }
}

impl Drop for DismissHandle {
    fn drop(&mut self) {
        remove(self.0);
    }
}

/// Push a dismiss handler onto the top of the stack for `doc_key`'s document.
///
/// `f` is asked, on every Escape press, whether it wants the key: `true`
/// consumes it and stops the scan, `false` passes it to the next handler down
/// and ultimately to the app. An overlay answers `false` while it is closed —
/// see the module docs on why the check cannot be made here instead.
///
/// `doc_key` is the document the handler belongs to, as
/// [`NodeHandle::doc_key`](crate::dom::NodeHandle::doc_key) reports it; `0`
/// (the "no document yet" sentinel) and a document-less dispatch are both read
/// permissively by [`doc_matches`], so a single-document app, a headless test
/// and rinch-web all behave as if the key were not there.
///
/// The handler is additionally tied to the scope that is rendering, if any: it
/// runs *inside* that scope (so a signal it creates belongs to the component)
/// and stops being offered the key once the scope is disposed.
pub fn push_dismiss_handler(doc_key: u64, f: impl Fn() -> bool + 'static) -> DismissHandle {
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    let entry = Entry {
        id,
        doc: doc_identity(doc_key),
        owner: current_owner(),
        handler: Rc::new(f),
    };
    STACK.with(|s| s.borrow_mut().push(entry));
    DismissHandle(id)
}

/// Offer "dismiss" to the stack, topmost first. Returns whether a handler took it.
///
/// [`dispatch_keyboard_event`](super::dispatch_keyboard_event) calls this for an
/// Escape press. A shell with another gesture that means *close the topmost
/// overlay* — Android's system Back, say — can call it directly.
///
/// **A direct caller owns whatever its handlers deferred.** A handler that
/// cannot finish the job in an `Fn() -> bool` will park the request somewhere
/// and consume the key, trusting its caller to answer it: the desktop runtime's
/// open-`<select>` entry does exactly that, and `RinchApp::handle_event` drains
/// it the moment this function returns. Call this from somewhere else and that
/// drain does not happen, so the gesture looks inert. Drain the same state the
/// Escape path does.
///
/// Handlers run with **no borrow of the stack held**, because closing an overlay
/// is very likely to unmount it, which drops its [`DismissHandle`] and mutates
/// the stack from inside the handler.
///
/// The scan works from a **snapshot**, and re-checks liveness and the document
/// per candidate but not *"is this entry still registered"*. So a handler that
/// answers `false` and, as a side effect, releases another entry's handle would
/// leave that released handler still reachable from the snapshot for the rest
/// of this scan. No shipped handler can do it — every component's `false` path
/// is a bare `opened_fn()` read with no side effect, and the `true` path stops
/// the scan — but a handler that does real work on its `false` path should know
/// the snapshot is not revalidated.
pub fn dispatch_dismiss() -> bool {
    let caller = current_dispatching_doc();

    // Snapshot topmost-first. Cloning the `Rc`s is what lets user code run
    // outside the borrow; the ids let us prune afterwards even though the stack
    // may have been rearranged in the meantime.
    let candidates: Vec<(u64, Option<u64>, Option<Owner>, DismissFn)> = STACK.with(|s| {
        s.borrow()
            .iter()
            .rev()
            .map(|e| (e.id, e.doc, e.owner.clone(), e.handler.clone()))
            .collect()
    });

    let mut dead: Vec<u64> = Vec::new();
    let mut consumed = false;
    for (id, doc, owner, handler) in candidates {
        // A disposed component's handler reads freed signals — never run it.
        if owner.as_ref().is_some_and(|o| !o.is_alive()) {
            dead.push(id);
            continue;
        }
        if !doc_matches(doc, caller) {
            continue;
        }
        let took = match &owner {
            Some(o) => o.run(|| handler()),
            None => handler(),
        };
        if took {
            consumed = true;
            break;
        }
    }

    // Prune what we found dead. Entries are moved out and dropped *after* the
    // borrow ends: a handler is arbitrary user state whose `Drop` may re-enter.
    if !dead.is_empty() {
        let reaped = STACK.with(|s| {
            let Ok(mut stack) = s.try_borrow_mut() else {
                return Vec::new();
            };
            let mut out = Vec::new();
            let mut i = 0;
            while i < stack.len() {
                if dead.contains(&stack[i].id) {
                    out.push(stack.remove(i));
                } else {
                    i += 1;
                }
            }
            out
        });
        drop(reaped);
    }

    consumed
}

/// Remove the entry with `id`, if it is still there.
///
/// The entry is taken out under the borrow and dropped outside it — its handler
/// closes over user state whose `Drop` may itself touch this stack.
/// `try_borrow_mut` because this runs from [`DismissHandle`]'s `Drop`, which a
/// panic can drive while the stack is already borrowed.
fn remove(id: u64) {
    let reaped = STACK.with(|s| {
        let Ok(mut stack) = s.try_borrow_mut() else {
            // Unreachable today: every mutable borrow of `STACK` either runs no
            // user code under it (`push`) or moves entries out rather than
            // dropping them under it (the prune). Said out loud anyway, because
            // failing here leaves a *released* handler in the stack, still
            // answering Escape, and a silent no-op in a release path is the
            // family this codebase keeps being bitten by.
            debug_assert!(false, "dismiss stack borrowed while releasing entry {id}");
            tracing::warn!("dismiss stack borrowed while releasing entry {id}; entry leaked");
            return None;
        };
        stack
            .iter()
            .position(|e| e.id == id)
            .map(|i| stack.remove(i))
    });
    drop(reaped);
}

/// How many handlers are registered on this thread. Diagnostics and tests only.
#[doc(hidden)]
pub fn dismiss_handler_count() -> usize {
    STACK.with(|s| s.borrow().len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::push_dispatching_doc;
    use crate::reactive::{Scope, Signal};
    use std::cell::Cell as StdCell;

    /// Every test shares one thread-local stack, and `cargo test` reuses
    /// threads. Start from empty so an earlier test's leaked handle cannot
    /// answer this one's Escape.
    fn clear_stack() {
        let reaped = STACK.with(|s| std::mem::take(&mut *s.borrow_mut()));
        drop(reaped);
    }

    /// A handler that records that it ran and consumes when `open`.
    fn recorder(open: bool) -> (Rc<StdCell<u32>>, impl Fn() -> bool) {
        let hits = Rc::new(StdCell::new(0u32));
        let h = hits.clone();
        (hits, move || {
            h.set(h.get() + 1);
            open
        })
    }

    #[test]
    fn the_topmost_open_handler_consumes_and_the_ones_below_are_not_asked() {
        clear_stack();
        let (outer_hits, outer) = recorder(true);
        let (inner_hits, inner) = recorder(true);
        let _outer = push_dismiss_handler(0, outer);
        let _inner = push_dismiss_handler(0, inner);

        assert!(dispatch_dismiss(), "an open handler consumes Escape");
        assert_eq!(inner_hits.get(), 1, "the topmost handler is asked");
        assert_eq!(
            outer_hits.get(),
            0,
            "a consumed key must not reach the handler below — LIFO, not broadcast"
        );
    }

    /// The fixed point this whole module exists to avoid: ask the stack from the
    /// *front* and the two-modal case still looks right whenever both are open,
    /// because either answer closes *a* modal. It only diverges when the outer
    /// one is closed, which is the case a front-to-back scan gets wrong — and
    /// the case `Modal`'s "stays mounted while closed" makes ordinary.
    #[test]
    fn a_closed_handler_is_passed_over_rather_than_consuming() {
        clear_stack();
        let (closed_hits, closed) = recorder(false);
        let (open_hits, open) = recorder(true);
        let _closed_below = push_dismiss_handler(0, closed);
        let _open_above = push_dismiss_handler(0, open);

        assert!(dispatch_dismiss());
        assert_eq!(open_hits.get(), 1);
        assert_eq!(closed_hits.get(), 0, "the open one above it consumed first");

        clear_stack();
        let (closed_hits, closed) = recorder(false);
        let (open_hits, open) = recorder(true);
        let _open_below = push_dismiss_handler(0, open);
        let _closed_above = push_dismiss_handler(0, closed);

        assert!(dispatch_dismiss());
        assert_eq!(closed_hits.get(), 1, "the closed one is asked first");
        assert_eq!(
            open_hits.get(),
            1,
            "and declining passes the key down to the open one below"
        );
    }

    #[test]
    fn a_stack_of_closed_handlers_leaves_the_key_to_the_app() {
        clear_stack();
        let (hits, closed) = recorder(false);
        let _a = push_dismiss_handler(0, closed);
        let (_hits_b, closed_b) = recorder(false);
        let _b = push_dismiss_handler(0, closed_b);

        assert!(
            !dispatch_dismiss(),
            "nothing consumed it, so the app still sees Escape"
        );
        assert_eq!(hits.get(), 1, "both were asked");
    }

    #[test]
    fn dropping_the_handle_releases_the_handler() {
        clear_stack();
        let (hits, open) = recorder(true);
        let handle = push_dismiss_handler(0, open);
        assert!(dispatch_dismiss());
        assert_eq!(hits.get(), 1);

        drop(handle);
        assert!(!dispatch_dismiss(), "a released handler is not asked");
        assert_eq!(hits.get(), 1);
        assert_eq!(dismiss_handler_count(), 0, "and its entry is gone");
    }

    /// Liveness is not redundant with the handle: a handler whose handle was
    /// leaked (or outlived by its component's state) must still stop being
    /// asked, because it closes over signals the component already freed.
    /// This is the #183 rule, and it is what the handle alone cannot give.
    #[test]
    fn a_handler_whose_scope_was_disposed_is_never_run_and_does_not_block_the_one_below() {
        clear_stack();
        let (below_hits, below) = recorder(true);
        let _below = push_dismiss_handler(0, below);

        let (doomed_hits, doomed) = recorder(true);
        let scope = Scope::new();
        // Leak the handle deliberately: the point is that liveness alone is
        // enough, with nothing eagerly removing the entry.
        let leaked = scope.run(|| push_dismiss_handler(0, doomed));
        std::mem::forget(leaked);
        assert_eq!(dismiss_handler_count(), 2);

        scope.dispose();
        assert!(
            dispatch_dismiss(),
            "the disposed handler must not swallow the key"
        );
        assert_eq!(
            doomed_hits.get(),
            0,
            "a disposed component's handler never runs"
        );
        assert_eq!(
            below_hits.get(),
            1,
            "the key falls through to the one below"
        );
        assert_eq!(
            dismiss_handler_count(),
            1,
            "and the dead entry is reaped as it is passed"
        );
    }

    /// The actual #183 shape: the handler reads a signal its component owns.
    /// Reading a freed signal panics, so "never run again" is load-bearing.
    #[test]
    fn a_disposed_handlers_freed_signal_is_never_read() {
        clear_stack();
        let scope = Scope::new();
        let leaked = scope.run(|| {
            let open = Signal::new(true);
            push_dismiss_handler(0, move || open.get())
        });
        std::mem::forget(leaked);
        scope.dispose();
        assert!(!dispatch_dismiss(), "would panic on a freed signal");
    }

    /// The handler runs inside its own scope, so whatever it allocates belongs
    /// to the component rather than to whatever the event loop was doing.
    #[test]
    fn a_handler_runs_with_its_own_scope_as_the_ambient_owner() {
        clear_stack();
        let scope = Scope::new();
        let seen = Rc::new(StdCell::new(false));
        let s = seen.clone();
        let owner = scope.run(current_owner).expect("a live scope");
        let handle = scope.run(|| {
            push_dismiss_handler(0, move || {
                s.set(current_owner().is_some_and(|o| o == owner));
                true
            })
        });
        assert!(dispatch_dismiss());
        assert!(seen.get(), "the handler's ambient owner is its own scope");
        drop(handle);
    }

    /// #134/#139: two documents on one thread share this stack. One document's
    /// Escape must not close the other's modal.
    #[test]
    fn another_documents_escape_does_not_reach_this_documents_overlay() {
        clear_stack();
        let (doc_one_hits, one) = recorder(true);
        let _one = push_dismiss_handler(1, one);

        {
            let _dispatching = push_dispatching_doc(2);
            assert!(
                !dispatch_dismiss(),
                "document 2's Escape must not close document 1's overlay"
            );
        }
        assert_eq!(doc_one_hits.get(), 0);

        {
            let _dispatching = push_dispatching_doc(1);
            assert!(dispatch_dismiss(), "its own document's Escape does");
        }
        assert_eq!(doc_one_hits.get(), 1);
    }

    /// Both halves of the permissive rule: a handler registered with no document
    /// serves every document (a headless host, a test), and a dispatch with no
    /// document reaches every handler (rinch-web, which never marks dispatch).
    #[test]
    fn a_document_less_registration_or_dispatch_stays_permissive() {
        clear_stack();
        let (hits, open) = recorder(true);
        let _ownerless = push_dismiss_handler(0, open);
        {
            let _dispatching = push_dispatching_doc(7);
            assert!(dispatch_dismiss(), "doc 0 is the no-document sentinel");
        }
        assert_eq!(hits.get(), 1);

        clear_stack();
        let (hits, open) = recorder(true);
        let _in_doc_7 = push_dismiss_handler(7, open);
        assert!(
            dispatch_dismiss(),
            "a dispatch outside any document reaches every handler"
        );
        assert_eq!(hits.get(), 1);
    }

    /// Closing an overlay unmounts it, which drops its handle from *inside* the
    /// handler. A borrow held across the call would make that a `BorrowMutError`.
    #[test]
    fn a_handler_may_release_itself_while_it_runs() {
        clear_stack();
        let slot: Rc<RefCell<Option<DismissHandle>>> = Rc::new(RefCell::new(None));
        let s = slot.clone();
        let handle = push_dismiss_handler(0, move || {
            s.borrow_mut().take();
            true
        });
        *slot.borrow_mut() = Some(handle);

        assert!(dispatch_dismiss());
        assert_eq!(
            dismiss_handler_count(),
            0,
            "the handler released itself from inside its own call"
        );
        assert!(!dispatch_dismiss());
    }

    /// And the other re-entrant direction: a handler that opens a new overlay.
    #[test]
    fn a_handler_may_push_a_new_handler_while_it_runs() {
        clear_stack();
        let slot: Rc<RefCell<Vec<DismissHandle>>> = Rc::new(RefCell::new(Vec::new()));
        let s = slot.clone();
        let first = push_dismiss_handler(0, move || {
            s.borrow_mut().push(push_dismiss_handler(0, || true));
            true
        });
        assert!(dispatch_dismiss());
        assert_eq!(dismiss_handler_count(), 2);
        drop(first);
        drop(slot);
        assert_eq!(dismiss_handler_count(), 0);
    }
}
