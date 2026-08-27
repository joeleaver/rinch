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

/// User code owed to the widget that just lost focus, collected during teardown
/// and dispatched by [`RinchApp::fire_focus_work`] once the caller has finished
/// installing the new focus owner's state.
///
/// **User code must never run mid-teardown.** Dispatching it while the caller
/// still holds pre-transition captures (the new input's value, a hit-test node
/// id) lets a handler that mutates the DOM invalidate them — the empirically
/// confirmed stale-install / recycled-slot bugs from the #244 review — and the
/// arbiter's own state is inconsistent while a transition is half-done, so a
/// re-entrant focus change would compound it.
///
/// The two payloads are mutually exclusive in practice (there is one previous
/// owner, and it is either an `<input>` or a registered node), but the struct
/// carries both so the mechanism stays one thing rather than two parallel ones.
#[must_use]
pub(crate) struct PendingFocusWork {
    /// A blurred `<input>`'s `data-onchange` commit as `(handler id, value)`
    /// (issue #226).
    input_commit: Option<(usize, String)>,
    /// A blurred registered focus target's `on_focus_lost`, as its
    /// `(doc_key, node id)` (issue #147).
    focus_lost: Option<(u64, usize)>,
}

impl RinchApp {
    /// Switch keyboard/IME focus to `target`, tearing down whatever was focused
    /// before. Returns `true` if the focus target actually changed.
    ///
    /// Tearing down an `Input` whose value changed since focus dispatches its
    /// `data-onchange` commit (issue #226), and tearing down a registered focus
    /// target fires its `on_focus_lost` (issue #147) — both **before
    /// returning**. Callers that install new-owner state after the transition
    /// must use [`Self::set_focus_target_deferred`] instead, and fire the
    /// returned work only once their installation is complete.
    pub(crate) fn set_focus_target(&mut self, target: FocusTarget) -> bool {
        let (changed, work) = self.set_focus_target_deferred(target);
        Self::fire_focus_work(work);
        changed
    }

    /// Dispatch the work collected by [`Self::set_focus_target_deferred`], if
    /// any. Returns whether an input's `data-onchange` commit actually fired —
    /// the input paths re-adopt the DOM value afterwards, because the handler
    /// may have rewritten the very field being focused.
    ///
    /// Must be called with no outstanding borrow of `self.doc`: everything here
    /// is user code and may mutate the DOM.
    pub(crate) fn fire_focus_work(work: Option<PendingFocusWork>) -> bool {
        let Some(work) = work else { return false };
        let committed = work.input_commit.is_some();
        if let Some((handler_id, value)) = work.input_commit {
            events::dispatch_input_event(events::EventHandlerId(handler_id), value);
        }
        if let Some((doc_key, node_id)) = work.focus_lost {
            crate::focus_registry::notify_focus_lost(doc_key, node_id);
        }
        committed
    }

    /// Announce `on_focus_gained` to the registered target now holding the
    /// keyboard, once the caller has finished installing it.
    ///
    /// A no-op unless `node_id` is *currently* the arbiter's `Node` target — so
    /// a path that bailed part-way (a vanished node, a malformed handler, an
    /// `on_focus_lost` that moved focus again) never announces a gain that did
    /// not happen — and a no-op for an unregistered node: every generic
    /// `tabindex` node takes `FocusTarget::Node`, only some registered for the
    /// news.
    pub(crate) fn notify_node_focus_gained(&self, node_id: usize) {
        if self.focus_target == FocusTarget::Node(node_id) {
            crate::focus_registry::notify_focus_gained(self.doc_key(), node_id);
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

    /// [`Self::set_focus_target`], except the blurred owner's user code (an
    /// input's `data-onchange` commit, a registered target's `on_focus_lost`)
    /// is **returned instead of dispatched**, for callers that finish
    /// installing the new owner's state after the transition (the input click
    /// path, programmatic focus, the mousedown claim): fire it via
    /// [`Self::fire_focus_work`] once installation is complete, then adopt any
    /// rewrite the handler made (`adopt_focused_input_value_from_dom`).
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
    ) -> (bool, Option<PendingFocusWork>) {
        if self.focus_target == target {
            return (false, None);
        }
        // Everything the blurred owner is owed, collected in its arm and
        // dispatched only after the transition completes — see
        // [`PendingFocusWork`] for why it cannot run here.
        let mut pending = PendingFocusWork {
            input_commit: None,
            focus_lost: None,
        };
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
                        pending.input_commit =
                            Self::input_attr_handler_up(&d.tree, prev, "data-onchange")
                                .filter(|&hid| {
                                    events::has_input_handler(events::EventHandlerId(hid))
                                })
                                .map(|hid| (hid, payload));
                    }
                }
                self.clear_input_focus_attrs();
                self.focused_input_handler_id = None;
                self.focused_input_value.clear();
                self.focused_input_baseline.clear();
                self.focused_input_state = None;
                self.focused_input_node_id = None;
                self.focused_input_preedit = None;
                self.focused_input_deferred_value = None;
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
                // Tell a registered custom widget it lost the keyboard (issue
                // #147) — deferred, like the input commit above.
                //
                // An *unmounted* target is silently absent here: its scope
                // disposal already deregistered it, so this finds nothing and
                // nothing is announced. That is deliberate (decision 5) —
                // calling back after disposal reads freed signals and panics
                // (issue #141 PR4).
                let doc_key = self.doc_key();
                if crate::focus_registry::is_registered(doc_key, prev) {
                    pending.focus_lost = Some((doc_key, prev));
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
        let has_work = pending.input_commit.is_some() || pending.focus_lost.is_some();
        (true, has_work.then_some(pending))
    }

    /// The IME state the focused target wants — enable + caret rect for a text
    /// target, disabled otherwise. The runtime applies it via the window. This is
    /// the single bridge from focus → the platform IME surface (see
    /// [`ImeState`]).
    pub(crate) fn ime_state(&self) -> ImeState {
        // A blurred window drives no IME, whatever holds the in-document claim
        // (issue #147): the claim is deliberately *kept* across a window blur,
        // so without this gate the OS candidate window would keep following a
        // caret in a window that no longer has the keyboard.
        if !self.window_focused {
            return ImeState {
                enabled: false,
                cursor_area: None,
            };
        }
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
        self.focused_input_node().is_some()
    }

    /// The focused `<input>`'s node id, if one holds focus.
    ///
    /// The Android shell watches this rather than [`Self::has_focused_input`]:
    /// a soft keyboard's composing region belongs to the field it was started
    /// in, and moving between two fields is invisible to Android (one
    /// `RinchInputView` holds focus throughout), so the shell has to notice the
    /// move itself and restart the IME. Crate-internal, like the
    /// `focused_editor_id` below it — the shell is in this crate, and
    /// embedders are served by `has_focused_input`.
    pub(crate) fn focused_input_node(&self) -> Option<usize> {
        match self.focus_target {
            FocusTarget::Input(id) => Some(id),
            _ => None,
        }
    }

    /// Whether the focused text control accepts a line break.
    ///
    /// True for a focused `<textarea>` and nothing else: an `<input>` holds a
    /// single-line value, and a rich-text editor is not a `FocusTarget::Input`.
    ///
    /// A soft keyboard has to be told this per field, because it draws a
    /// different Enter key for each — an action (Go, Send, ✓) that *ends* the
    /// input session, or a newline that stays in it. Android learns of it
    /// through `EditorInfo`, which is built once per input session, so the
    /// shell watches this value and restarts the session when it changes.
    /// Rinch moves focus between its own fields without Android seeing
    /// anything (one `RinchInputView` holds focus throughout), so nothing else
    /// would tell the keyboard.
    pub fn focused_input_is_multiline(&self) -> bool {
        let FocusTarget::Input(node_id) = self.focus_target else {
            return false;
        };
        let Some(doc) = &self.doc else { return false };
        let d = doc.borrow();
        Self::node_is_textarea(&d.tree, node_id)
    }

    /// Whether `node_id` is a `<textarea>` — the one control tag whose value
    /// can hold a line break. One predicate for both readers of it (the Enter
    /// key path's insert-or-submit decision and the soft-keyboard flag above),
    /// so they cannot drift apart.
    pub(crate) fn node_is_textarea(tree: &rinch_dom::NodeTree, node_id: usize) -> bool {
        tree.get(node_id).and_then(|n| n.tag()) == Some("textarea")
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
