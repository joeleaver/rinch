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

/// A `data-onchange` commit collected while tearing down a focused input
/// (issue #226), to be dispatched via [`RinchApp::fire_input_commit`] once the
/// caller has finished installing the new focus owner's state. The commit
/// handler is user code: dispatching it while the caller still holds
/// pre-transition captures (the new input's value, a hit-test node id) lets a
/// handler that mutates the DOM invalidate them — the empirically confirmed
/// stale-install / recycled-slot bugs from the #244 review.
#[must_use]
pub(crate) struct PendingInputCommit {
    handler_id: usize,
    value: String,
}

impl RinchApp {
    /// Switch keyboard/IME focus to `target`, tearing down whatever was focused
    /// before. Returns `true` if the focus target actually changed.
    ///
    /// Tearing down an `Input` whose value changed since focus dispatches its
    /// `data-onchange` commit (issue #226) **before returning**. Callers that
    /// install new-input state after the transition must use
    /// [`Self::set_focus_target_deferred`] instead, and fire the returned
    /// commit only once their installation is complete.
    pub(crate) fn set_focus_target(&mut self, target: FocusTarget) -> bool {
        let (changed, commit) = self.set_focus_target_deferred(target);
        Self::fire_input_commit(commit);
        changed
    }

    /// Dispatch a commit collected by [`Self::set_focus_target_deferred`], if
    /// any. Must be called with no outstanding borrow of `self.doc` — the
    /// handler is user code and may mutate the DOM.
    pub(crate) fn fire_input_commit(commit: Option<PendingInputCommit>) {
        if let Some(c) = commit {
            events::dispatch_input_event(events::EventHandlerId(c.handler_id), c.value);
        }
    }

    /// Walk up from `start` looking for `attr`, returning the first parseable
    /// handler id — the desktop analogue of the web backend's delegation walk
    /// (`change`/`input` bubble in the browser, so a handler may sit on an
    /// ancestor of the control).
    pub(crate) fn input_attr_handler_up(
        tree: &rinch_dom::NodeTree,
        start: usize,
        attr: &str,
    ) -> Option<usize> {
        let mut cur = Some(start);
        while let Some(nid) = cur {
            let node = tree.get(nid)?;
            if let Some(s) = node.attributes.get(attr) {
                return s.parse::<usize>().ok();
            }
            cur = node.parent;
        }
        None
    }

    /// [`Self::set_focus_target`], except a blurred input's `data-onchange`
    /// commit is **returned instead of dispatched**, for callers that finish
    /// installing the new owner's state after the transition (the input click
    /// path, programmatic input focus): fire it via
    /// [`Self::fire_input_commit`] once installation is complete, then adopt
    /// any rewrite the handler made (`resync_input_state_from_dom`).
    ///
    /// This only handles **teardown** of the previous owner; the caller installs
    /// the new owner's state (the rich per-engine state — `EditableState`, the CE
    /// cursor, the editor selection — lives outside the enum). Re-focusing the
    /// same target is a no-op (returns `false`) so a re-click inside the focused
    /// widget keeps its state rather than tearing itself down.
    ///
    /// Must be called with no outstanding borrow of `self.doc` (it writes DOM
    /// attributes while clearing input/CE focus).
    pub(crate) fn set_focus_target_deferred(
        &mut self,
        target: FocusTarget,
    ) -> (bool, Option<PendingInputCommit>) {
        if self.focus_target == target {
            return (false, None);
        }
        // The `data-onchange` commit for a blurred input (issue #226): collected
        // in the Input arm, dispatched only after the transition completes —
        // user code must never run mid-teardown, where the arbiter state is
        // inconsistent and a re-entrant focus change would compound it.
        let mut pending_change: Option<PendingInputCommit> = None;
        match self.focus_target {
            FocusTarget::None => {}
            FocusTarget::Surface(_) => {
                // `set_focused_surface` dispatches `FocusLost` to the old surface.
                crate::render_surface::set_focused_surface(None);
            }
            FocusTarget::Input(prev) => {
                // Blur commits a pending IME composition first, matching the
                // browser's compositionend-before-blur: the composed text
                // enters the buffer and the `value` attribute and fires
                // `oninput` exactly as a normal IME commit would, so the
                // change payload below includes it. This runs before any
                // teardown mutation, with arbiter state fully consistent —
                // equivalent to the commit event arriving just before the
                // blur.
                if let Some((preedit, _)) = self.focused_input_preedit.take() {
                    self.dispatch_input_ime(prev, ImeEvent::Commit(preedit));
                }
                // Focus leaving the input ends the typed gesture: fire
                // `data-onchange` with the final text, HTML-style — only if the
                // value actually changed since the gesture began (baseline), and
                // only through a still-live handler (a scope-disposal self-heal
                // lands here with the input's handlers already freed).
                if self.focused_input_value != self.focused_input_baseline
                    && let Some(doc) = &self.doc
                {
                    let d = doc.borrow();
                    // The slot must still be the gesture's input: node ids are
                    // recycled slab indices, so `prev` could name an unrelated
                    // element by now — and that element's handlers being live
                    // would not make the commit right.
                    let still_ours = d
                        .tree
                        .get(prev)
                        .and_then(|n| n.attributes.get("data-oninput"))
                        .and_then(|s| s.parse::<usize>().ok())
                        .is_some_and(|h| Some(h) == self.focused_input_handler_id);
                    if still_ours {
                        // Payload: the live `value` attribute — what the field
                        // displays and what the web backend's listener
                        // delivers — not the private keystroke buffer, which a
                        // programmatic mid-gesture rewrite never reaches. The
                        // buffer-vs-baseline gate above stays authoritative
                        // for "did the user change anything" (a purely
                        // programmatic change never commits, like the
                        // browser's dirty flag).
                        let payload = d
                            .tree
                            .get(prev)
                            .and_then(|n| n.attributes.get("value").cloned())
                            .unwrap_or_else(|| self.focused_input_value.clone());
                        pending_change =
                            Self::input_attr_handler_up(&d.tree, prev, "data-onchange")
                                .filter(|&hid| {
                                    events::has_input_handler(events::EventHandlerId(hid))
                                })
                                .map(|hid| PendingInputCommit {
                                    handler_id: hid,
                                    value: payload,
                                });
                    }
                }
                self.clear_input_focus_attrs();
                self.focused_input_handler_id = None;
                self.focused_input_value.clear();
                self.focused_input_baseline.clear();
                self.focused_input_state = None;
                self.focused_input_node_id = None;
                self.focused_input_preedit = None;
                // Clear the input's DOM focus and keyboard focus ring, exactly
                // like the Node arm below — otherwise a blur that never goes
                // through a left-mousedown (a click into the rich-text editor,
                // a right/middle click, the stale-handler self-heal) leaves the
                // input painting `:focus-visible` while something else owns the
                // keyboard. The `focused_node` guard keeps a successor that
                // already moved DOM focus from being blurred by this teardown.
                if let Some(doc) = &self.doc {
                    let mut d = doc.borrow_mut();
                    d.set_focus_visible(prev, false);
                    if d.tree.focused_node == Some(prev) {
                        d.update_focus(None);
                    }
                }
            }
            FocusTarget::Select(_) => {
                // Tearing down select focus dismisses its popup: remove the
                // app-created backdrop + panel nodes.
                self.remove_select_popup_nodes();
            }
            FocusTarget::Node(prev) => {
                // Clear the node's DOM focus and keyboard focus ring. The
                // `focused_node` guard keeps a successor that already moved DOM
                // focus (the pointer path updates it on mousedown, before the
                // arbiter runs) from being blurred by this teardown.
                if let Some(doc) = &self.doc {
                    let mut d = doc.borrow_mut();
                    d.set_focus_visible(prev, false);
                    if d.tree.focused_node == Some(prev) {
                        d.update_focus(None);
                    }
                }
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
        (true, pending_change)
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
            // A generic focusable node is not a text target — explicit rather
            // than folded into `_` so a future text-capable variant can't land
            // here silently (issue #176 documents that trap for Surface).
            FocusTarget::Node(_) => ImeState {
                enabled: false,
                cursor_area: None,
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

    /// The focused `<input>`'s node id, if one holds focus.
    ///
    /// The Android shell watches this rather than [`Self::has_focused_input`]:
    /// a soft keyboard's composing region belongs to the field it was started
    /// in, and moving between two fields is invisible to Android (one
    /// `RinchInputView` holds focus throughout), so the shell has to notice the
    /// move itself and restart the IME.
    pub fn focused_input_node(&self) -> Option<usize> {
        match self.focus_target {
            FocusTarget::Input(id) => Some(id),
            _ => None,
        }
    }

    /// Whether a generic focusable node (`tabindex`, `FocusTarget::Node`,
    /// issue #228) holds focus. It consumes Enter/Space (and anchors Tab), so
    /// embed hosts must route keyboard input to rinch while one is focused
    /// (`RinchContext::wants_keyboard` includes it). Public beside
    /// [`Self::has_focused_input`] / [`Self::has_focused_contenteditable`].
    pub fn has_focused_node(&self) -> bool {
        matches!(self.focus_target, FocusTarget::Node(_))
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
