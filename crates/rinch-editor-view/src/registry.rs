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

// Whether a drag-select owned by `owner` may be read or ended while `caller`'s
// events are being dispatched (issue #139). The rule lives in rinch-core so the
// pointer-capture drag (`ActiveDrag::is_foreign`) and this anchor cannot drift
// apart: `None` on either side means "no document in particular", and only two
// `Some` keys that differ are refused. rinch-web passes `None` throughout — one
// page, one pointer stream, one mouseup listener that must be able to end
// whatever is in flight.
use rinch_core::doc_matches as same_doc;

use crate::handle::EditorHandle;

thread_local! {
    /// `(doc_key, container node id, handle)` for every mounted editor. The
    /// [`doc_key`](rinch_core::dom::DomDocument::doc_key) scopes entries per
    /// document: container node ids are per-document slab indices, so two
    /// documents on one thread (two embedded `RinchContext`s, or two desktop
    /// windows) can both hold an editor at the same container id (issue #134).
    static EDITORS: RefCell<Vec<(u64, usize, EditorHandle)>> = const { RefCell::new(Vec::new()) };
    /// In-progress pointer drag-selects as `(doc, container id, anchor
    /// position)` — **at most one per document**, exactly as [`EDITORS`] holds
    /// at most one handle per `(doc, container)`.
    ///
    /// Keyed by document for the same reason [`EDITORS`] is: container ids are
    /// per-document slab indices, and a thread can pump two documents' pointer
    /// streams through this registry (issue #139). A *list*, not one slot, for
    /// the second half of that reason — with a single slot the two documents
    /// still contend for it, so B's mousedown would wipe the anchor A is still
    /// dragging from, and A's mouseup could then no longer clear the entry it no
    /// longer owns.
    static DRAG: RefCell<Vec<(Option<u64>, usize, usize)>> = const { RefCell::new(Vec::new()) };
}

/// Begin a pointer drag-select at `anchor` (a `Pos.0`) in editor `container` of
/// the document `doc` (`None` for a runtime with a single page-wide pointer
/// stream — see [`drag_anchor`]).
///
/// Replaces `doc`'s own in-progress drag-select, and only that one: a second
/// document's anchor is left in place, because the gesture it belongs to is
/// still being made.
pub fn begin_drag(doc: Option<u64>, container: usize, anchor: usize) {
    DRAG.with(|d| {
        let mut drags = d.borrow_mut();
        drags.retain(|(owner, _, _)| !same_doc(*owner, doc));
        drags.push((doc, container, anchor));
    });
}

/// The active drag-select `(container id, anchor position)` for the document
/// `doc`, if one is in progress there.
///
/// Answers `None` for a drag-select armed by a *different* document, so a second
/// `RinchApp` on the same thread (a DevTools window, a second embedded context)
/// neither extends nor reads the first one's selection.
pub fn drag_anchor(doc: Option<u64>) -> Option<(usize, usize)> {
    DRAG.with(|d| {
        d.borrow()
            .iter()
            .find(|(owner, _, _)| same_doc(*owner, doc))
            .map(|(_, container, anchor)| (*container, *anchor))
    })
}

/// End any in-progress pointer drag-select **owned by `doc`**. Another
/// document's drag-select is left alone: its mouseup has not happened yet.
pub fn end_drag(doc: Option<u64>) {
    DRAG.with(|d| {
        d.borrow_mut()
            .retain(|(owner, _, _)| !same_doc(*owner, doc))
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The drag-select anchor is per-document (issue #139).
    ///
    /// Container ids are per-document slab indices, so two documents on one
    /// thread — a desktop window and its DevTools panel, two embedded
    /// `RinchContext`s — routinely hold editors at the *same* container id.
    /// Unkeyed, B applies A's anchor to B's own document, and any mouseup in B
    /// silently ends the drag-select the user is still making in A.
    #[test]
    fn a_drag_select_belongs_to_the_document_that_began_it() {
        let (dk_a, dk_b) = (11, 22);
        end_drag(None); // defensive: the slot is thread-local, but reset it anyway

        begin_drag(Some(dk_a), 7, 3);
        assert_eq!(
            drag_anchor(Some(dk_b)),
            None,
            "another document must not see A's drag-select"
        );
        assert_eq!(
            drag_anchor(Some(dk_a)),
            Some((7, 3)),
            "…while A itself still does"
        );

        end_drag(Some(dk_b));
        assert_eq!(
            drag_anchor(Some(dk_a)),
            Some((7, 3)),
            "and B's mouseup must not end A's drag-select"
        );

        end_drag(Some(dk_a));
        assert_eq!(drag_anchor(Some(dk_a)), None, "A's own mouseup ends it");
    }

    /// Two documents may drag-select **at once**, and neither one's mousedown
    /// disturbs the other's anchor (issue #139).
    ///
    /// Scoping only the *reads* would be half a fix: with a single shared slot
    /// B's mousedown still overwrites the anchor A is dragging from, and A's
    /// mouseup can then no longer clear the entry — because A no longer owns it
    /// — so B's stale anchor outlives its own gesture and A's is simply gone.
    #[test]
    fn two_documents_drag_select_without_disturbing_each_other() {
        let (dk_a, dk_b) = (33, 44);
        end_drag(None);

        begin_drag(Some(dk_a), 7, 3);
        begin_drag(Some(dk_b), 7, 90); // same container id — per-document slab indices collide

        assert_eq!(
            drag_anchor(Some(dk_a)),
            Some((7, 3)),
            "B's mousedown must not overwrite the anchor A is still dragging from"
        );
        assert_eq!(drag_anchor(Some(dk_b)), Some((7, 90)), "…nor B's own");

        end_drag(Some(dk_a));
        assert_eq!(drag_anchor(Some(dk_a)), None, "A's mouseup ends A's");
        assert_eq!(
            drag_anchor(Some(dk_b)),
            Some((7, 90)),
            "and leaves B's, whose mouseup has not happened yet"
        );

        end_drag(Some(dk_b));
        assert_eq!(drag_anchor(Some(dk_b)), None);
    }

    /// A runtime with one page-wide pointer stream (rinch-web) passes `None`
    /// throughout and keeps the old unscoped behaviour: its single document-level
    /// mouseup listener has no container in hand and must still end whatever is
    /// in flight.
    #[test]
    fn an_unkeyed_drag_select_stays_drivable_by_anyone() {
        end_drag(None);

        begin_drag(None, 4, 9);
        assert_eq!(drag_anchor(None), Some((4, 9)));
        assert_eq!(
            drag_anchor(Some(11)),
            Some((4, 9)),
            "an unowned drag-select is readable by any document"
        );

        end_drag(None);
        assert_eq!(drag_anchor(None), None);
    }
}
