//! `lock_scroll`, shared by `Modal` and `Drawer` (issue #474).
//!
//! Both declared the prop and did nothing with it. What they need is identical
//! and has three parts that are each easy to get subtly wrong on their own, so
//! they live here once:
//!
//! 1. **An edge guard.** The effect re-runs whenever anything it read changes,
//!    not only when `opened` flips — a `Memo` recomputing, a sibling signal in
//!    the same getter. Locking on every run would ratchet the backend's count up
//!    for ever and the page would never unlock.
//! 2. **A release on unmount.** An overlay can be taken out of the tree while
//!    still open, which is the ordinary shape of `if show { Modal { … } }`. The
//!    effect that would have unlocked it never runs again, so without
//!    `on_cleanup` the page stays unscrollable for the rest of the session —
//!    state armed by one event and cleared only by a second that may never
//!    arrive. The cleanup is the second, independent clearing condition.
//! 3. **The root node, not the body.** The lock is taken out *in the name of*
//!    this overlay's root: desktop keeps that subtree scrollable so the dialog's
//!    own `overflow: auto` body still works. `RenderScope::body_handle()` is the
//!    trap — on the web it is `<div id="rinch-body">`, not the page.
//!
//! See [`rinch_core::dom::DomDocument::set_scroll_locked`] for what each backend
//! then does, and why the count lives there rather than here.

use std::cell::Cell;
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};

use crate::overlay_dismiss::ReactiveBool;

/// Hold a page scroll lock for as long as this overlay is open and mounted.
///
/// A no-op when `lock_scroll` is false, which is what "off" has to mean: the
/// page behind goes on scrolling.
///
/// `root` is the overlay's own root element — the subtree that stays scrollable
/// while everything else is refused.
pub fn arm_lock_scroll(
    scope: &mut RenderScope,
    root: &NodeHandle,
    lock_scroll: bool,
    opened: bool,
    opened_fn: Option<&ReactiveBool>,
) {
    if !lock_scroll {
        return;
    }
    // Without an `opened_fn` the only truth available is the static prop, which
    // is what the rest of the component renders from too. The effect then runs
    // exactly once, which is correct: nothing can change it.
    let is_open: ReactiveBool = match opened_fn {
        Some(f) => f.clone(),
        None => Rc::new(move || opened),
    };

    // Whether *this* overlay is currently holding a lock. The backend counts
    // locks; this says whether we are one of them, so a re-run of the effect
    // that does not change `opened` neither double-locks nor double-releases.
    let held = Rc::new(Cell::new(false));

    let effect_root = root.clone();
    let effect_held = held.clone();
    scope.create_effect(move || {
        let open = is_open();
        if open && !effect_held.get() {
            effect_root.set_scroll_locked(true);
            effect_held.set(true);
        } else if !open && effect_held.get() {
            effect_root.set_scroll_locked(false);
            effect_held.set(false);
        }
    });

    let cleanup_root = root.clone();
    scope.on_cleanup(move || {
        if held.get() {
            cleanup_root.set_scroll_locked(false);
            held.set(false);
        }
    });
}
