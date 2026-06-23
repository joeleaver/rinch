//! A thread-local registry of mounted editors, so the runtime can drive each
//! editor's post-layout caret pass and look one up by container id.
//!
//! This is the cleaner successor to the old CE thread-local API registry
//! (`set_ce_api_factory`/`with_active_ce_api`): an [`Editor`](super::Editor)
//! registers its [`EditorHandle`] keyed by its container node id when it mounts,
//! and the runtime looks editors up by container id (for input) or sweeps them
//! all (for the caret pass).
//!
//! Keyboard focus is **not** tracked here — that is the runtime's single
//! [`FocusTarget`](crate::app) authority (design A10). The runtime holds
//! `FocusTarget::Editor(container_id)` and resolves it to a handle via
//! [`editor_for`].

use std::cell::RefCell;

use super::handle::EditorHandle;

thread_local! {
    /// `(container node id, handle)` for every mounted editor.
    static EDITORS: RefCell<Vec<(usize, EditorHandle)>> = const { RefCell::new(Vec::new()) };
    /// An in-progress pointer drag-select: `(container id, anchor position)`.
    static DRAG: RefCell<Option<(usize, usize)>> = const { RefCell::new(None) };
}

/// Begin a pointer drag-select at `anchor` (a `Pos.0`) in editor `container`.
pub fn begin_drag(container: usize, anchor: usize) {
    DRAG.with(|d| *d.borrow_mut() = Some((container, anchor)));
}

/// The active drag-select `(container id, anchor position)`, if a drag is in
/// progress.
pub fn drag_anchor() -> Option<(usize, usize)> {
    DRAG.with(|d| *d.borrow())
}

/// End any in-progress pointer drag-select.
pub fn end_drag() {
    DRAG.with(|d| *d.borrow_mut() = None);
}

/// Register `handle` under its `container_id` (replacing any prior registration).
pub fn register_editor(container_id: usize, handle: EditorHandle) {
    EDITORS.with(|e| {
        let mut e = e.borrow_mut();
        e.retain(|(id, _)| *id != container_id);
        e.push((container_id, handle));
    });
}

/// Forget the editor mounted at `container_id`.
pub fn unregister_editor(container_id: usize) {
    EDITORS.with(|e| e.borrow_mut().retain(|(id, _)| *id != container_id));
    super::virtualization::forget(container_id);
}

/// Every mounted editor as `(container id, handle)`. The virtualization driver
/// sweeps these each layout to maintain a per-editor block window.
pub(crate) fn all_editors() -> Vec<(usize, EditorHandle)> {
    EDITORS.with(|e| e.borrow().iter().map(|(id, h)| (*id, h.clone())).collect())
}

/// The handle registered for `container_id`, if any. The runtime resolves its
/// `FocusTarget::Editor(container_id)` to a handle through this.
pub fn editor_for(container_id: usize) -> Option<EditorHandle> {
    EDITORS.with(|e| {
        e.borrow()
            .iter()
            .find(|(id, _)| *id == container_id)
            .map(|(_, h)| h.clone())
    })
}

/// Re-render mounted editors' overlays. The runtime calls this **after** layout
/// (design A3, phase 2), when the host has fresh geometry, passing the focused
/// editor's container id (from the `FocusTarget` arbiter).
///
/// Only the **focused** editor renders its caret and selection highlight; every
/// other mounted editor hides its overlays — so a blurred editor shows neither a
/// caret nor a selection (and the caret blink, which only runs for the focused
/// editor, idles for the rest).
pub fn update_all_carets(focused: Option<usize>) -> bool {
    let editors: Vec<(usize, EditorHandle)> =
        EDITORS.with(|e| e.borrow().iter().map(|(id, h)| (*id, h.clone())).collect());
    let mut moved = false;
    for (id, handle) in editors {
        if Some(id) == focused {
            moved |= handle.update_caret();
        } else {
            moved |= handle.hide_overlays();
        }
    }
    moved
}
