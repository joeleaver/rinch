//! The focus arbiter (design A10).
//!
//! [`FocusTarget`](super::FocusTarget) is the single source of truth for which
//! widget owns keyboard/IME input. All focus changes go through
//! [`RinchApp::set_focus_target`], which tears the previous owner down before the
//! caller installs the next — so the four engines (render surfaces, `<input>`,
//! the legacy contenteditable, and the new editor) can never be focused at once,
//! and the `KeyDown`/`KeyUp` routing is a single exhaustive match instead of the
//! old interceptor-then-fallback chain.

use super::*;

impl RinchApp {
    /// Switch keyboard/IME focus to `target`, tearing down whatever was focused
    /// before. Returns `true` if the focus target actually changed.
    ///
    /// This only handles **teardown** of the previous owner; the caller installs
    /// the new owner's state (the rich per-engine state — `EditableState`, the CE
    /// cursor, the editor selection — lives outside the enum). Re-focusing the
    /// same target is a no-op (returns `false`) so a re-click inside the focused
    /// widget keeps its state rather than tearing itself down.
    ///
    /// Must be called with no outstanding borrow of `self.doc` (it writes DOM
    /// attributes while clearing input/CE focus).
    pub(crate) fn set_focus_target(&mut self, target: FocusTarget) -> bool {
        if self.focus_target == target {
            return false;
        }
        match self.focus_target {
            FocusTarget::None => {}
            FocusTarget::Surface(_) => {
                // `set_focused_surface` dispatches `FocusLost` to the old surface.
                crate::render_surface::set_focused_surface(None);
            }
            FocusTarget::Input(_) => {
                self.clear_input_focus_attrs();
                self.focused_input_handler_id = None;
                self.focused_input_value.clear();
                self.focused_input_state = None;
                self.focused_input_node_id = None;
            }
            FocusTarget::ContentEditable(prev) => {
                ce::clear_active_ce_api();
                self.ce_ops = None;
                self.set_contenteditable_attributes(prev, false, 0, 0);
                self.focused_contenteditable = None;
            }
            #[cfg(feature = "new-editor")]
            FocusTarget::Editor(prev) => {
                // Hide the blurred editor's caret and selection highlight so an
                // unfocused editor shows neither. (Its model state lives in the
                // `EditorHandle` — this only clears the overlays.) A pure focus
                // change doesn't dirty layout, so the post-layout caret pass may
                // short-circuit; hide explicitly here at the focus choke-point.
                if let Some(handle) = crate::editor::editor_for(prev) {
                    handle.hide_overlays();
                }
            }
        }
        self.focus_target = target;
        self.scene_dirty = true;
        true
    }

    /// Whether a text input element is currently focused.
    pub fn has_focused_input(&self) -> bool {
        matches!(self.focus_target, FocusTarget::Input(_))
    }

    /// The container id of the focused new-editor, if one holds focus. Drives the
    /// runtime's caret-blink tick.
    #[cfg(feature = "new-editor")]
    pub(crate) fn focused_editor_id(&self) -> Option<usize> {
        match self.focus_target {
            FocusTarget::Editor(id) => Some(id),
            _ => None,
        }
    }

    /// Whether a rich-text editor (legacy contenteditable or the new editor) is
    /// currently focused. Kept (and repointed at `FocusTarget`) for the embed and
    /// Android soft-keyboard callers (design A9).
    pub fn has_focused_contenteditable(&self) -> bool {
        #[cfg(feature = "new-editor")]
        {
            matches!(
                self.focus_target,
                FocusTarget::ContentEditable(_) | FocusTarget::Editor(_)
            )
        }
        #[cfg(not(feature = "new-editor"))]
        {
            matches!(self.focus_target, FocusTarget::ContentEditable(_))
        }
    }
}
