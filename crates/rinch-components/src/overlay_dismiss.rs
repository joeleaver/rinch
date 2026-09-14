//! Escape-to-dismiss, shared by `Modal`, `Drawer` and `Popover` (issue #474).
//!
//! The three declared `close_on_escape` and did nothing with it. They all need
//! the same wiring, and getting any of them subtly different is the bug this
//! module exists to make impossible — in particular the **open check**, which
//! belongs at dispatch and not here:
//!
//! `render` runs *once*. `opened_fn` only rewrites a class, so a closed overlay
//! stays mounted with its handler in place. Deciding "register only when open"
//! would arm the handler forever the first time the overlay opened, and Escape
//! would go on being swallowed by an invisible dialog. So the handler is armed
//! at mount and asked, afresh, on every Escape.
//!
//! See [`rinch_core::push_dismiss_handler`] for the stack itself — why it is a
//! stack rather than the single-slot keyboard interceptor, and how nesting,
//! liveness and two-documents-on-one-thread are handled.

use std::rc::Rc;

use rinch_core::Callback;
use rinch_core::dom::{NodeHandle, RenderScope};

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
