//! Popups (#2552) in the window registry.
//!
//! A popup's parent is a window id (`0`: a document that is no window of the
//! registry's, `rinch::testing`'s mounted app). The chain is the parent
//! relation, not ownership: a popup owned by the main window but anchored in a
//! menu is above that menu.

use super::*;

/// The window whose document is `doc`: the window mounted as it, else the
/// main window, else `0` (a document that is no registry window).
pub(crate) fn window_of_doc(doc: u64) -> u64 {
    with(|r| {
        r.entries
            .iter()
            .find(|e| e.doc == Some(doc))
            .map(|e| e.id)
            .or(r.main)
            .unwrap_or(0)
    })
}

/// Whether window `id` can be a popup's parent: open (the main window, any
/// other, a popup), or `0`.
pub(crate) fn can_parent(id: u64) -> bool {
    id == 0 || WindowHandle(id).is_open_untracked()
}

/// A press (a button, a key, a touch) reached `doc`'s window.
pub(crate) fn note_press(doc: u64) {
    let id = window_of_doc(doc);
    with(|r| {
        let now = rinch_core::timer::now();
        r.pressed.retain(|&(p, _)| p != id);
        r.pressed.push((id, now));
    });
}

/// `f` on popup `id`'s entry, if it is an open popup.
pub(crate) fn with_popup<R>(
    id: u64,
    f: impl FnOnce(&mut crate::popup::PopupEntry) -> R,
) -> Option<R> {
    with(|r| {
        r.entries
            .iter_mut()
            .find(|e| e.id == id)
            .and_then(|e| e.popup.as_mut())
            .map(f)
    })
}

/// The open popups, `(id, parent, kind)`, oldest first.
pub(crate) fn popups() -> Vec<(u64, u64, rinch_core::element::PopupKind)> {
    with(|r| {
        r.entries
            .iter()
            .filter_map(|e| e.popup.as_ref().map(|p| (e.id, p.parent, p.props.kind)))
            .collect()
    })
}

/// Whether the window `id` or a popup below it in its chain received a
/// press within [`crate::popup::PRESS_RECENT`]: what a menu's grab names. A
/// press long ago is no cause (a menu opened from D-Bus a minute after a
/// click would be refused by the compositor, or name a stale press).
pub(crate) fn chain_pressed(id: u64) -> bool {
    let links = popups();
    let pressed = with(|r| r.pressed.clone());
    let now = rinch_core::timer::now();
    let chain = crate::popup::chain::ancestors_and_self(id, &links);
    pressed.iter().any(|&(w, at)| {
        chain.contains(&w) && now.saturating_duration_since(at) <= crate::popup::PRESS_RECENT
    })
}

/// Record why popup `id` goes, unless a cause is recorded already: the
/// first cause is the one told.
pub(crate) fn note_dismissal(id: u64, why: rinch_core::element::Dismissal) {
    with_popup(id, |p| {
        if p.dismissal.is_none() {
            p.dismissal = Some(why);
        }
    });
}

/// Popup `id` goes for `why`: recorded, and closed (with every popup above
/// it, topmost first).
pub(crate) fn dismiss(id: u64, why: rinch_core::element::Dismissal) {
    note_dismissal(id, why);
    WindowHandle(id).close();
}
