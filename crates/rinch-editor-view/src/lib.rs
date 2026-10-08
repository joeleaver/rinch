//! The renderer-agnostic rich-text editor view.
//!
//! [`RinchDomEditorView`] implements the [`EditorView`](rinch_editor_core::EditorView)
//! seam from `rinch-editor-core`, projecting an immutable `EditorState { doc, selection }`
//! onto **any** [`DomDocument`](rinch_core::dom::DomDocument) host tree — the desktop
//! `rinch-dom` renderer or the browser `web_sys` DOM. The model is the single source of
//! truth; the host is *derived* from it on every transaction and never read back for
//! content (design §6).
//!
//! [`EditorHandle`] is the imperative API for app/component code (design A7);
//! [`mount_editor`] wires one into a [`RenderScope`] and registers it so the platform
//! runtime can drive its caret pass and route input. The platform-specific input glue
//! (key/pointer/IME → commands) lives in each runtime crate (`rinch` for desktop,
//! `rinch-web` for the browser), not here.

mod blink;
#[cfg(feature = "collaboration")]
mod collab;
mod component;
mod handle;
mod images;
mod keys;
mod links;
pub mod registry;
mod styles;
mod view;

pub use component::Editor;
pub use handle::{EditorHandle, REVEAL_PATIENCE, ScrollAlign, SelectionAnchor};
pub use images::{ImageInput, ImageInputSource, sniff_image_mime};
pub use keys::EditorKey;
pub use links::{LinkClick, LinkHover, LinkSpan};
#[cfg(feature = "collaboration")]
pub use registry::collab_receive_for;
pub use registry::{
    any_overlay_pass_owed, begin_drag, drag_anchor, editor_for, editor_for_doc, end_drag,
    link_hover_wanted, overlay_pass_owed, reveal_owed, set_blur_handler, set_focus_handler,
    set_link_hover, set_overlay_pass_scheduler, set_overlay_refresher, set_unregister_listener,
    unregister_editor, update_all_carets,
};
/// Which side of a soft wrap a caret is drawn on — see
/// [`EditorHandle::set_selection_with_affinity`].
pub use rinch_core::dom::CaretAffinity;
/// The collaboration error type (re-exported from `rinch-editor-collab`) returned by
/// the [`EditorHandle`] collaboration methods.
#[cfg(feature = "collaboration")]
pub use rinch_editor_collab::{CollabError, OversizedTable};
// Reconciliation ([`EditorHandle::collab_state_vector`] /
// [`EditorHandle::collab_sync_diff`]) needs no re-exported engine types: a state vector
// and a diff are both opaque `Vec<u8>`, and a diff is applied through the same
// [`EditorHandle::collab_receive`] as a broadcast delta.
pub use view::RinchDomEditorView;

use std::rc::Rc;
use std::time::Duration;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_editor_core::model::Fragment;
use rinch_editor_core::{Node, Schema, default_plugins};

/// Create an unmounted [`EditorHandle`] over a fresh (single empty paragraph)
/// document — the factory for the declarative API (mirrors `create_render_surface`).
///
/// The handle works **before** it is placed in the tree: `load_html`/`load_doc`/
/// `set_selection`/`command` operate on its owned state, which the view renders
/// when the [`Editor`] component mounts it. Hand the same handle to toolbar
/// buttons and to `Editor { editor: handle }`.
pub fn create_editor() -> EditorHandle {
    let schema = Rc::new(Schema::starter_kit());
    let empty = empty_doc(&schema);
    EditorHandle::unmounted(schema, empty, default_plugins())
}

/// Mount a new editor into `scope` imperatively: create the host container, build
/// a handle over a fresh document, register it with the runtime, and return both.
/// Equivalent to [`create_editor`] + [`EditorHandle::mount`]; prefer the
/// declarative [`Editor`] component in `rsx!` for new code.
///
/// The container is marked `data-pm-editor` (deliberately **not**
/// `contenteditable`). Focus is click-driven through the runtime's focus arbiter —
/// no auto-focus on mount (with multiple editors that would race the single focus
/// authority).
pub fn mount_editor(scope: &mut RenderScope) -> (NodeHandle, EditorHandle) {
    let handle = create_editor();
    let container = handle.mount(scope);
    (container, handle)
}

/// The outcome of a [`caret_blink_tick`]: the runtime should repaint if `redraw`
/// is set, then arm its next wake for `next` from now (the next toggle).
pub struct CaretBlink {
    /// The caret's visibility toggled this tick — request a redraw.
    pub redraw: bool,
    /// Time until the next phase toggle (the runtime's next wake).
    pub next: Duration,
}

/// Advance the caret blink for the focused editor (`focused` = its container id,
/// `None` if no editor is focused). Toggles the caret's `visibility` for the
/// current half-period and reports when the next toggle is due.
///
/// Returns `None` when nothing is blinking — no editor focused, the editor has no
/// caret (a non-collapsed selection), or it unmounted — in which case the runtime
/// should idle until the next input. The desktop runtime calls this from
/// `about_to_wait` every iteration, and an embedded `RinchContext` from every
/// `update` (design A3 phase 2 is for *geometry*; this is the standalone
/// animation tick).
///
/// There is one clock per **document** (issue #1149): several documents can
/// tick on one thread — each embedded `RinchContext`, and a desktop window
/// beside them — and each blinks its own focused editor on its own phase. (The
/// desktop DevTools panel is a second document on the window's thread, but
/// the runtime ticks only the app's.) The clock uses `std::time::Instant`, so a web runtime drives blink
/// with its own timer (`setInterval` → [`EditorHandle::set_caret_blink`])
/// rather than calling this.
///
/// `doc_key` is the calling runtime's document (see
/// [`DomDocument::doc_key`](rinch_core::dom::DomDocument::doc_key)) — `focused`
/// is a container id from that document's focus arbiter, and container ids
/// collide across documents on one thread (issue #134).
pub fn caret_blink_tick(doc_key: u64, focused: Option<usize>) -> Option<CaretBlink> {
    // Only a focused editor that is still registered gets a clock. The focus
    // arbiter keeps naming an editor that unmounted until the next key, and a
    // clock made for it here would outlive the document once the context is
    // dropped: `unregister_editor`, which drops a clock, has already run.
    let focused = focused.filter(|&id| registry::editor_for_doc(doc_key, id).is_some());
    let prev = blink::target(doc_key);
    // On a focus change, restore the previously-blinked caret to solid so a
    // blurred editor never freezes mid-blink with a hidden caret. Only this
    // document's: another document's caret is on its own clock.
    if prev != focused {
        if let Some(prev_id) = prev
            && let Some(h) = registry::editor_for_doc(doc_key, prev_id)
        {
            h.set_caret_blink(true);
        }
        blink::set_target(doc_key, focused);
    }
    let handle = registry::editor_for_doc(doc_key, focused?)?;
    let (visible, next) = blink::tick(doc_key);
    let redraw = handle.set_caret_blink(visible)?;
    Some(CaretBlink { redraw, next })
}

/// How many documents on this thread hold a caret blink clock. A structural
/// test hook (#1149: the clock list must not outlive the documents it is
/// kept for), not a stable API.
#[doc(hidden)]
pub fn blink_clock_count() -> usize {
    blink::clock_count()
}

/// A fresh document: one empty paragraph.
fn empty_doc(schema: &Schema) -> Node {
    schema
        .branch(
            "doc",
            Fragment::from_node(schema.branch("paragraph", Fragment::empty()).unwrap()),
        )
        .unwrap()
}
