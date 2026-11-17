//! A window's display scale changing (the window dragged to a display with a
//! different scale factor, a density change).
//!
//! A runtime calls [`notify_display_scale_changed`] with the document's key
//! when the scale its window paints at changes; anything that must react —
//! `DropdownMenuSub` closes an open flyout, as Chrome closes its menus on a
//! display change — reads [`display_scale_changes`] inside an effect, which
//! subscribes it. The value is a counter, not the scale: what matters is that
//! it changed.

use crate::doc_keyed::DocMap;
use crate::reactive::{Scope, Signal};

crate::doc_keyed! {
    /// Each document's counter, with the scope that owns its signal: the
    /// document's sweep (#2797) drops the scope, which frees the signal.
    static CHANGES: DocMap<(Scope, Signal<u64>)> = DocMap::new();
}

fn signal(doc_key: u64) -> Signal<u64> {
    if let Some(s) = CHANGES.with(|c| c.get(doc_key).map(|(_, s)| *s))
        && s.is_alive()
    {
        return s;
    }
    let scope = Scope::new();
    let s = scope.run(|| Signal::new(0u64));
    // A replaced entry (its signal freed some other way) is dropped after the
    // registry is let go.
    let _replaced = CHANGES.with_mut(|c| c.insert(doc_key, (scope, s)));
    s
}

/// How many times the display scale of document `doc_key`'s window has
/// changed. Read inside an effect to re-run on the next change.
pub fn display_scale_changes(doc_key: u64) -> u64 {
    signal(doc_key).get()
}

/// Called by a runtime when the scale document `doc_key`'s window paints at
/// changes. Not app API.
#[doc(hidden)]
///
/// Nothing is written when nothing has ever read the counter of that document:
/// a frame with no listener costs no signal write (the perf baseline
/// `a_scale_factor_change_restyles_and_repaints_in_full` counts none). A
/// reader that arrives later starts from the counter as it is then.
pub fn notify_display_scale_changed(doc_key: u64) {
    let existing = CHANGES.with(|c| c.get(doc_key).map(|(_, s)| *s));
    if let Some(s) = existing
        && s.is_alive()
    {
        s.update(|n| *n += 1);
    }
}
