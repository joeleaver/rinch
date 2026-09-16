//! Escape-to-dismiss, shared by `Modal`, `Drawer` and `Popover` (issue #474).
//!
//! The three declared `close_on_escape` and did nothing with it. They all need
//! the same wiring, and getting any of them subtly different is the bug this
//! module exists to make impossible — in particular the **open check**, which
//! belongs at dispatch and not here:
//!
//! `render` runs *once*. `opened_fn` only toggles a class, so a closed overlay
//! stays mounted with its handler in place. Deciding "register only when open"
//! would arm the handler forever the first time the overlay opened, and Escape
//! would go on being swallowed by an invisible dialog. So the handler is armed
//! at mount and asked, afresh, on every Escape.
//!
//! See [`rinch_core::push_dismiss_handler`] for the stack itself — why it is a
//! stack rather than the single-slot keyboard interceptor, and how nesting,
//! liveness and two-documents-on-one-thread are handled.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, DismissHandle};

/// A reactive `opened` getter, as the three overlay components spell it.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

/// Arm Escape-to-close for an overlay, for as long as it stays mounted.
///
/// A no-op when `close_on_escape` is false or there is no `onclose` to invoke —
/// in both cases Escape is left to the app, which is what "off" has to mean.
///
/// `root` is only read for its
/// [`doc_key`](rinch_core::dom::NodeHandle::doc_key), so that two documents on
/// one thread do not answer each other's Escape.
pub fn arm_close_on_escape(
    scope: &mut RenderScope,
    root: &NodeHandle,
    close_on_escape: bool,
    opened: bool,
    opened_fn: Option<&ReactiveBool>,
    onclose: Option<&Callback>,
) {
    if !close_on_escape {
        return;
    }
    let Some(cb) = onclose.cloned() else {
        return;
    };
    // Without an `opened_fn` the only truth available is the static prop, which
    // is what the rest of the component renders from too.
    let is_open: ReactiveBool = match opened_fn {
        Some(f) => f.clone(),
        None => Rc::new(move || opened),
    };

    let handle = rinch_core::push_dismiss_handler(root.doc_key(), move || {
        if is_open() {
            cb.invoke();
            true
        } else {
            false
        }
    });
    // Unmounting releases it. Without this the entry would survive its overlay
    // and keep answering Escape — liveness catches it at dispatch, but only
    // once the component's *scope* is disposed, and only at the cost of a
    // stale entry the scan has to walk past first.
    scope.on_cleanup(move || drop(handle));
}

/// Arm Escape-to-close for an overlay that **opens after it has mounted**,
/// pushing its entry when it opens and releasing it when it closes.
///
/// The other policy in this file — [`arm_close_on_escape`] — registers once, at
/// mount, and asks `opened_fn` at dispatch. That is right for an overlay whose
/// open state is a *prop*, and it is wrong the moment such an overlay is
/// **statically nested inside another one**, because the stack is LIFO and a
/// component renders *after* its children: `Modal { ColorInput { … } }` pushes
/// the input's entry first and the modal's on top of it, so the modal answers
/// Escape even while the input's picker is the thing the user is looking at.
///
/// Pushing at *open* time is the fix, and it is not a new idea — it is exactly
/// what the desktop runtime's open-`<select>` entry does (issue #671,
/// `open_select_popup`): the popup opens after the modal mounted, so LIFO puts
/// it on top by the same rule that makes nested modals work, with no precedence
/// special case anywhere. It is available here because `is_open` is a signal
/// this component owns and writes, so the falling edge is observable; the three
/// prop-driven overlays have no such edge to hang a release on and keep the
/// simpler policy.
///
/// `onclose` is invoked and the key is **consumed unconditionally**: the entry
/// exists only while the overlay is open, so there is no closed state for it to
/// decline in. The effect is idempotent — a `Signal::set` notifies on every
/// write, equal value or not — so a redundant `true` does not push a second
/// entry.
///
/// Released on close, and on unmount-while-open through `on_cleanup`. Both are
/// needed: an unmount while open would otherwise leave an entry answering
/// Escape for a component that no longer exists (liveness catches it at
/// dispatch, but only after the scope is disposed and only at the cost of a
/// stale entry in the scan).
pub fn arm_close_on_escape_while_open(
    scope: &mut RenderScope,
    root: &NodeHandle,
    is_open: ReactiveBool,
    onclose: Callback,
) {
    let doc_key = root.doc_key();
    let slot: Rc<RefCell<Option<DismissHandle>>> = Rc::new(RefCell::new(None));

    let slot_effect = slot.clone();
    scope.create_effect(move || {
        let open = is_open();
        let held = slot_effect.borrow().is_some();
        // Deliberately not `if let` / a borrow held across the arm: closing
        // drops the handle, which mutates the dismiss stack, and the drop of a
        // handler is arbitrary user state. Every borrow here ends on its own
        // line.
        if open && !held {
            let cb = onclose.clone();
            let handle = rinch_core::push_dismiss_handler(doc_key, move || {
                cb.invoke();
                true
            });
            *slot_effect.borrow_mut() = Some(handle);
        } else if !open && held {
            let released = slot_effect.borrow_mut().take();
            drop(released);
        }
    });

    scope.on_cleanup(move || {
        let released = slot.borrow_mut().take();
        drop(released);
    });
}
