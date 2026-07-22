//! A thread-local registry of mounted editors, so the runtime can drive each
//! editor's post-layout caret pass and look one up by container id.
//!
//! This is the cleaner successor to the old CE thread-local API registry
//! (`set_ce_api_factory`/`with_active_ce_api`): an [`Editor`](crate::Editor)
//! registers its [`EditorHandle`] keyed by its container node id when it mounts,
//! and the runtime looks editors up by container id (for input) or sweeps them
//! all (for the caret pass).
//!
//! Keyboard focus is **not** tracked here — that is the platform runtime's single
//! focus-arbiter authority (design A10). The runtime holds the focused editor's
//! container id and resolves it to a handle via [`editor_for`].

use std::cell::RefCell;

use crate::handle::EditorHandle;

thread_local! {
    /// `(doc_key, container node id, handle)` for every mounted editor. The
    /// [`doc_key`](rinch_core::dom::DomDocument::doc_key) scopes entries per
    /// document: container node ids are per-document slab indices, so two
    /// documents on one thread (two embedded `RinchContext`s, or two desktop
    /// windows) can both hold an editor at the same container id (issue #134).
    static EDITORS: RefCell<Vec<(u64, usize, EditorHandle)>> = const { RefCell::new(Vec::new()) };
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

/// Register `handle` under its document's `doc_key` and `container_id`
/// (replacing any prior registration **for that same document + container** —
/// another document's editor at a colliding container id is left registered).
pub fn register_editor(doc_key: u64, container_id: usize, handle: EditorHandle) {
    EDITORS.with(|e| {
        let mut e = e.borrow_mut();
        e.retain(|(dk, id, _)| !(*dk == doc_key && *id == container_id));
        e.push((doc_key, container_id, handle));
    });
}

/// Forget the editor mounted at `container_id`.
///
/// Desktop block-virtualization state is *not* freed here: its per-editor window
/// driver already drops windows for editors no longer in [`all_editors`] on its
/// next sweep, and a stale window is never read in between (it is only consulted
/// for currently-registered editors). So removal from the registry is sufficient.
pub fn unregister_editor(doc_key: u64, container_id: usize) {
    EDITORS.with(|e| {
        e.borrow_mut()
            .retain(|(dk, id, _)| !(*dk == doc_key && *id == container_id))
    });
}

/// Every mounted editor as `(doc_key, container id, handle)`. The desktop
/// virtualization driver sweeps these each layout — filtered to its own
/// document — to maintain a per-editor block window.
pub fn all_editors() -> Vec<(u64, usize, EditorHandle)> {
    EDITORS.with(|e| {
        e.borrow()
            .iter()
            .map(|(dk, id, h)| (*dk, *id, h.clone()))
            .collect()
    })
}

/// The handle registered for `container_id` **in the document identified by
/// `doc_key`**. Runtimes that resolved `container_id` from their own document
/// (hit-testing, `FocusTarget::Editor`) must use this form — container ids
/// collide across documents on one thread (issue #134).
pub fn editor_for_doc(doc_key: u64, container_id: usize) -> Option<EditorHandle> {
    EDITORS.with(|e| {
        e.borrow()
            .iter()
            .find(|(dk, id, _)| *dk == doc_key && *id == container_id)
            .map(|(_, _, h)| h.clone())
    })
}

/// The first handle registered for `container_id` in **any** document.
///
/// For callers without a document in hand (the web input glue, collab routing,
/// the caret blink tick). Exact when container ids are unique on the thread —
/// the common single-document case; with several documents a collision resolves
/// to the earliest registration. Prefer [`editor_for_doc`] where the document
/// is known.
pub fn editor_for(container_id: usize) -> Option<EditorHandle> {
    EDITORS.with(|e| {
        e.borrow()
            .iter()
            .find(|(_, id, _)| *id == container_id)
            .map(|(_, _, h)| h.clone())
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
/// `doc_key`: pass `Some(key)` to touch only that document's editors (a desktop
/// or embed runtime driving its own post-layout pass); `None` sweeps every
/// mounted editor (the web runtime, which drives all islands from one place).
pub fn update_all_carets(doc_key: Option<u64>, focused: Option<usize>) -> bool {
    let editors = all_editors();
    let mut moved = false;
    for (dk, id, handle) in editors {
        if doc_key.is_some_and(|k| k != dk) {
            continue;
        }
        if Some(id) == focused {
            moved |= handle.update_caret();
        } else {
            moved |= handle.hide_overlays();
        }
    }
    moved
}

/// Deliver a remote collaboration `delta` to the editor mounted at `container_id`
/// (design M9). A no-op if no such editor is mounted or it isn't collaborating.
/// **Must be called on the main thread** — each platform runtime supplies its own
/// `Send`-safe `post_remote_delta` that marshals a transport-thread delta onto the
/// main thread and calls this. Returns whether the editor's document changed.
#[cfg(feature = "collaboration")]
pub fn collab_receive_for(container_id: usize, delta: &[u8]) -> bool {
    match editor_for(container_id) {
        Some(handle) => handle.collab_receive(delta),
        None => false,
    }
}
