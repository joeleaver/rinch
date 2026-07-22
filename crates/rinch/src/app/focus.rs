//! The focus arbiter (design A10).
//!
//! [`FocusTarget`](super::FocusTarget) is the single source of truth for which
//! widget owns keyboard/IME input. All focus changes go through
//! [`RinchApp::set_focus_target`], which tears the previous owner down before the
//! caller installs the next — so the three engines (render surfaces, `<input>`,
//! and the editor) can never be focused at once, and the `KeyDown`/`KeyUp`
//! routing is a single exhaustive match instead of the old
//! interceptor-then-fallback chain.

use super::*;

/// The IME state the focus arbiter wants the window to be in. The runtime
/// ([`crate::shell`]) diffs this against the window's current IME state each tick
/// and issues a winit `request_ime_update` only on change — so enable/disable and
/// candidate-box placement follow focus and the caret uniformly across targets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ImeState {
    /// Whether a text target is focused and wants IME composition.
    pub enabled: bool,
    /// The caret's logical window-space rect `(x, y, w, h)` for candidate-box
    /// placement, when known.
    pub cursor_area: Option<(f32, f32, f32, f32)>,
}

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
                self.focused_input_preedit = None;
            }
            FocusTarget::Select(_) => {
                // Tearing down select focus dismisses its popup: remove the
                // app-created backdrop + panel nodes.
                self.remove_select_popup_nodes();
            }
            #[cfg(feature = "desktop")]
            FocusTarget::Editor(prev) => {
                // Hide the blurred editor's caret and selection highlight so an
                // unfocused editor shows neither. (Its model state lives in the
                // `EditorHandle` — this only clears the overlays.) A pure focus
                // change doesn't dirty layout, so the post-layout caret pass may
                // short-circuit; hide explicitly here at the focus choke-point.
                if let Some(handle) = crate::editor::editor_for_doc(self.doc_key(), prev) {
                    handle.hide_overlays();
                }
            }
        }
        self.focus_target = target;
        self.scene_dirty = true;
        true
    }

    /// The IME state the focused target wants — enable + caret rect for a text
    /// target, disabled otherwise. The runtime applies it via the window. This is
    /// the single bridge from focus → the platform IME surface (see
    /// [`ImeState`]).
    pub(crate) fn ime_state(&self) -> ImeState {
        match self.focus_target {
            #[cfg(feature = "desktop")]
            FocusTarget::Editor(container) => {
                let cursor_area = crate::editor::editor_for_doc(self.doc_key(), container)
                    .and_then(|handle| {
                        let head = handle.selection().head();
                        self.editor_caret_point(&handle, head)
                            .map(|(x, y, h)| (x, y, 1.0, h))
                    });
                ImeState {
                    enabled: true,
                    cursor_area,
                }
            }
            FocusTarget::Input(node_id) => ImeState {
                enabled: true,
                cursor_area: self.input_caret_area(node_id),
            },
            // Surfaces and no focus do not drive desktop IME.
            _ => ImeState {
                enabled: false,
                cursor_area: None,
            },
        }
    }

    /// Whether a text input element is currently focused.
    pub fn has_focused_input(&self) -> bool {
        matches!(self.focus_target, FocusTarget::Input(_))
    }

    /// The container id of the focused new-editor, if one holds focus. Drives the
    /// runtime's caret-blink tick.
    #[cfg(feature = "desktop")]
    pub(crate) fn focused_editor_id(&self) -> Option<usize> {
        match self.focus_target {
            FocusTarget::Editor(id) => Some(id),
            _ => None,
        }
    }

    /// Whether a rich-text editor currently holds focus. Kept (and repointed at
    /// `FocusTarget`) for the embed and Android soft-keyboard callers (design A9).
    /// The name is retained for those callers; the editor is desktop-only, so
    /// non-desktop builds have no rich-text focus target.
    pub fn has_focused_contenteditable(&self) -> bool {
        #[cfg(feature = "desktop")]
        {
            matches!(self.focus_target, FocusTarget::Editor(_))
        }
        #[cfg(not(feature = "desktop"))]
        {
            false
        }
    }
}
