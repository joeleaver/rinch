//! The new desktop rich-text editor view (M5+), behind the `new-editor` feature.
//!
//! [`RinchDomEditorView`] implements the renderer-agnostic
//! [`EditorView`](rinch_editor_core::EditorView) seam from `rinch-editor-core`,
//! projecting an immutable `EditorState { doc, selection }` onto the rinch-dom
//! host tree. The model is the single source of truth; the host is *derived* from
//! it on every transaction and never read back for content (design §6). This is
//! the desktop replacement for the old contenteditable engine, which the M8 rip
//! deletes.
//!
//! [`EditorHandle`] is the imperative API for app/component code (design A7);
//! [`mount_editor`] wires one into a [`RenderScope`] and registers it so the
//! runtime can drive its caret pass and route input.

mod blink;
mod component;
mod handle;
mod registry;
mod view;
mod virtualization;

pub use component::Editor;
pub use handle::EditorHandle;
pub use registry::{
    begin_drag, drag_anchor, editor_for, end_drag, unregister_editor, update_all_carets,
};
pub use view::RinchDomEditorView;
pub(crate) use virtualization::{
    post_layout as virtualization_post_layout, pre_layout as virtualization_pre_layout,
};

use std::rc::Rc;
use std::time::Duration;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_editor_core::model::Fragment;
use rinch_editor_core::{Node, Schema, default_plugins};

/// Create an unmounted [`EditorHandle`] over a fresh (single empty paragraph)
/// document — the factory for the declarative API (mirrors
/// [`create_render_surface`](crate::render_surface::create_render_surface)).
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
/// `contenteditable`, so the old CE engine does not also activate on it). Focus is
/// click-driven through the runtime's `FocusTarget` arbiter (A10) — no auto-focus
/// on mount (with multiple editors that would race the single focus authority).
pub fn mount_editor(scope: &mut RenderScope) -> (NodeHandle, EditorHandle) {
    let handle = create_editor();
    let container = handle.mount(scope);
    (container, handle)
}

/// The outcome of a [`caret_blink_tick`]: the runtime should repaint if `redraw`
/// is set, then arm `ControlFlow::WaitUntil(now + next)` for the next toggle.
pub struct CaretBlink {
    /// The caret's visibility toggled this tick — request a redraw.
    pub redraw: bool,
    /// Time until the next phase toggle (the runtime's next wake).
    pub next: Duration,
}

/// Advance the caret blink for the focused editor (`focused` = its container id,
/// `None` if no editor is focused). Toggles the caret's `display` for the current
/// half-period and reports when the next toggle is due.
///
/// Returns `None` when nothing is blinking — no editor focused, the editor has no
/// caret (a non-collapsed selection), or it unmounted — in which case the runtime
/// should return the event loop to `ControlFlow::Wait` (zero idle CPU). The
/// runtime calls this from `about_to_wait` every iteration (design A3 phase 2 is
/// for *geometry*; this is the standalone animation tick).
pub fn caret_blink_tick(focused: Option<usize>) -> Option<CaretBlink> {
    // On a focus change, restore the previously-blinked caret to solid so a
    // blurred editor never freezes mid-blink with a hidden caret.
    let prev = blink::target();
    if prev != focused {
        if let Some(prev_id) = prev
            && let Some(h) = registry::editor_for(prev_id)
        {
            h.set_caret_blink(true);
        }
        blink::set_target(focused);
        blink::reset();
    }
    let handle = registry::editor_for(focused?)?;
    let (visible, next) = blink::tick();
    let redraw = handle.set_caret_blink(visible)?;
    Some(CaretBlink { redraw, next })
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
