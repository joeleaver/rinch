//! A window's display scale changing (the window dragged to a display with a
//! different scale factor, a density change).
//!
//! A runtime calls [`notify_display_scale_changed`] with the document's key
//! when the scale its window paints at changes; anything that must react —
//! `DropdownMenuSub` closes an open flyout, as Chrome closes its menus on a
//! display change — reads [`display_scale_changes`] inside an effect, which
//! subscribes it. The value is a counter, not the scale: what matters is that
//! it changed.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::reactive::Signal;

thread_local! {
    static CHANGES: RefCell<HashMap<u64, Signal<u64>>> = RefCell::new(HashMap::new());
}

fn signal(doc_key: u64) -> Signal<u64> {
    CHANGES.with(|c| {
        let mut c = c.borrow_mut();
        match c.get(&doc_key) {
            Some(s) if s.is_alive() => *s,
            _ => {
                let s = crate::reactive::unowned(|| Signal::new(0u64));
                c.insert(doc_key, s);
                s
            }
        }
    })
}

/// How many times the display scale of document `doc_key`'s window has
/// changed. Read inside an effect to re-run on the next change.
pub fn display_scale_changes(doc_key: u64) -> u64 {
    signal(doc_key).get()
}

/// Called by a runtime when the scale document `doc_key`'s window paints at
/// changes. Not app API.
#[doc(hidden)]
pub fn notify_display_scale_changed(doc_key: u64) {
    signal(doc_key).update(|n| *n += 1);
}
