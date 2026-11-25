//! The key arms of event dispatch: a `KeyDown` and a `KeyUp`, offered to the
//! handlers in the order `docs/internals/focus-and-keyboard.md` "The key
//! handler order" sets out (issue #3062).

use super::event_dispatch::hook_key_str;
use super::*;

impl RinchApp {
    /// Offer a key to the focused element's `onkeydown` / `onkeyup` handler
    /// and then its ancestors', nearest first, stopping at the first that
    /// consumes it (issue #1678). Answers whether one did.
    ///
    /// The walk starts at the node the arbiter's claim names — a text control
    /// (`FocusTarget::Input`) or a focusable node (`FocusTarget::Node`: a
    /// `tabindex` element, a button, a link, a closed `<select>`). The
    /// rich-text editor (its keys are `EditorHandle::on_key`'s), a render
    /// surface (`set_key_handler`) and an open `<select>` list are not offered,
    /// nor is a key while the focused field shows an IME composition: a key
    /// the input method is composing with is the input method's, which a
    /// browser marks `isComposing`. The ids are read before any handler runs,
    /// so a handler that moves or removes nodes does not change who is asked.
    pub(super) fn offer_element_key(&mut self, data: &events::KeyEventData) -> bool {
        let start = match self.focus_target {
            FocusTarget::Input(id) => id,
            FocusTarget::Node(id) if self.node_target_is_live(id) => id,
            _ => return false,
        };
        let data = &data
            .clone()
            .with_target(Some(rinch_core::dom::NodeId(start)));
        let attr = if data.is_up() {
            "data-onkeyup"
        } else {
            "data-onkeydown"
        };
        let ids: Vec<events::EventHandlerId> = {
            let Some(doc) = self.doc.as_ref() else {
                return false;
            };
            let d = doc.borrow();
            if d.tree
                .get(start)
                .and_then(|n| n.attributes.get("data-preedit"))
                .is_some_and(|p| !p.is_empty())
            {
                return false;
            }
            let mut ids = Vec::new();
            let mut cur = Some(start);
            while let Some(id) = cur {
                let Some(node) = d.tree.get(id) else { break };
                if let Some(h) = node
                    .attributes
                    .get(attr)
                    .and_then(|v| v.parse::<usize>().ok())
                {
                    ids.push(events::EventHandlerId(h));
                }
                cur = node.parent;
            }
            ids
        };
        !ids.is_empty() && events::dispatch_key_handler_chain(ids, data)
    }

    /// Offer a key to the app-wide listeners (`rinch_core::on_app_key`, issue
    /// #1784). [`Self::handle_key_down`] calls it for a press nothing earlier
    /// consumed — after the text context menu, an open `<select>` list, the
    /// interceptor, the focused editor's or node view's `on_key`, the focused
    /// element's `onkeydown` and its ancestors', the popups' key entries and
    /// the dismiss stack, and then (by the focus arbiter's holder) a focused
    /// registered widget's `FocusEntry::on_key` or a focused render surface's
    /// claim — and before rinch's defaults, the rich-text editor's keymap
    /// included (#3062). [`Self::handle_key_up`] calls it for every release,
    /// after the interceptor and the element's `onkeyup`; `dispatch_app_key`
    /// offers a release to the listeners that heard its press.
    ///
    /// A press is left out while the built-in text context menu is open, or
    /// while the focused field or rich-text editor shows an IME composition:
    /// a key the input method is composing with is the input method's, as
    /// [`Self::offer_element_key`] and `EditorHandle::offer_key` leave it. A
    /// focus move a listener made is applied before the key's default, as an
    /// element handler's is.
    pub(super) fn offer_app_key(&mut self, data: &events::KeyEventData) -> bool {
        if data.is_down() && (self.open_text_menu.is_some() || self.ime_composing()) {
            return false;
        }
        let consumed = events::dispatch_app_key(data);
        self.drain_focus_requests(false);
        consumed
    }

    /// Whether the keyboard's holder shows an IME composition: a focused
    /// field's `data-preedit`, or the focused rich-text editor's preedit.
    fn ime_composing(&self) -> bool {
        match self.focus_target {
            FocusTarget::Input(id) => self.doc.as_ref().is_some_and(|doc| {
                doc.borrow()
                    .tree
                    .get(id)
                    .and_then(|n| n.attributes.get("data-preedit"))
                    .is_some_and(|p| !p.is_empty())
            }),
            #[cfg(feature = "windowing")]
            FocusTarget::Editor(container) => {
                crate::editor::editor_for_doc(self.doc_key(), container)
                    .is_some_and(|h| h.is_composing())
            }
            _ => false,
        }
    }

    /// A key press, offered to each handler in turn until one consumes it
    /// (issue #3062; the table is `docs/internals/focus-and-keyboard.md` "The
    /// key handler order"). Before it gets here, `handle_event` has ended a
    /// pointer lock on Escape and the shell has run the app's menu
    /// accelerators.
    ///
    /// Everything rinch does with a key is a browser *default action*: it
    /// runs only after every app hook has had the key and left it. The app
    /// beats rinch everywhere except where Chrome beats the page: Escape
    /// ending a drag, and an open native popup (the text context menu, a
    /// `<select>` list).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle_key_down(
        &mut self,
        key: KeyCode,
        logical_key: Option<String>,
        text: Option<String>,
        modifiers: Modifiers,
        repeat: KeyRepeat,
        (vp_w, vp_h): (f32, f32),
        actions: &mut Vec<AppAction>,
    ) {
        // 1. Rinch's keyboard owners. Escape cancels an active drag: the same
        //    teardown a `PointerCancel` runs (issue #333) — it used to spell
        //    its own, and spelled less: the ghost a drag had suppressed stayed
        //    suppressed into the next drag, and a surface under the drag never
        //    heard it leave.
        if key == KeyCode::Escape {
            if self.cancel_active_dnd(vp_w, vp_h) {
                actions.push(AppAction::RequestRedraw);
                return;
            }
            if self.pending_drag.take().is_some() {
                return;
            }
        }

        let shift = modifiers.shift;
        let ctrl = modifiers.primary();
        let alt = modifiers.alt;

        // Build key string for the user keyboard hook + global fallback.
        let key_str: Option<String> = hook_key_str(key, text.as_deref(), logical_key.as_deref());

        tracing::trace!(?key, ?text, ?key_str, shift, ctrl, alt, "KeyDown event");

        //    The built-in text context menu (#813) and an open native
        //    `<select>` list each take every key while open, ahead of the
        //    app's hooks, as a browser's native popup takes the keyboard
        //    before the page sees it. The menu's target keeps the *claim* —
        //    the arbiter is untouched — so it is asked by state, not by
        //    `focus_target`. Escape closes either here.
        if self.open_text_menu.is_some() && self.handle_text_menu_key(key, vp_w, vp_h) {
            actions.push(AppAction::RequestRedraw);
            return;
        }
        if matches!(self.focus_target, FocusTarget::Select(_))
            && self.handle_select_key(key, text.as_deref(), vp_w, vp_h)
        {
            actions.push(AppAction::RequestRedraw);
            return;
        }

        let key_data = key_str.as_ref().map(|ks| {
            events::KeyEventData::new(ks.clone(), format!("{:?}", key)).with_modifiers(
                ctrl,
                shift,
                alt,
                modifiers.meta,
            )
        });

        // 2. The keyboard interceptor (`set_keyboard_interceptor`), the app's
        //    capture-phase hook: ahead of every handler below, element
        //    `onkeydown` included, as a `window` capture listener is in a
        //    browser. Render surfaces do not hijack this slot — they are
        //    routed by `FocusTarget::Surface` below.
        if let Some(kd) = &key_data
            && events::dispatch_key_interceptor(kd)
        {
            actions.push(AppAction::RequestRedraw);
            return;
        }

        // 3. The target phase: the focused editor's `EditorHandle::on_key`, a
        //    node view's `on_key`, or the focused element's `onkeydown` and
        //    its ancestors'. Each sits ahead of the popups' key entries and
        //    the dismiss stack (step 4), which is what lets an autocomplete
        //    popup in an editor, or a `[[` picker in a field, inside a
        //    `Modal` take Escape: the popup closes and the modal stays
        //    (#1678).
        #[cfg(feature = "windowing")]
        if let FocusTarget::Editor(container) = self.focus_target
            && let Some(ks) = key_str.as_deref()
            && let Some(handle) = crate::editor::editor_for_doc(self.doc_key(), container)
        {
            let offered = crate::editor::EditorKey {
                // The browser's spelling, which `on_key` speaks on
                // both platforms (the spacebar is `" "`, #1161).
                key: ks,
                primary: ctrl,
                ctrl: modifiers.ctrl,
                meta: modifiers.meta,
                shift,
                alt,
                repeat: repeat == KeyRepeat::Repeat,
            };
            if handle.offer_key(&offered) {
                // The callback may have edited or moved the selection.
                self.refresh_editor_overlays();
                self.resolve_and_repaint(vp_w, vp_h);
                actions.push(AppAction::RequestRedraw);
                return;
            }
        }

        // A key while the keyboard is inside a node view's component (an
        // input in an app's sheet drawn in an editor) is the view's
        // `NodeViewSpec::on_key`'s first, ahead of the element that holds
        // the keyboard; an Escape it leaves gives the keyboard back to the
        // editor, caret after the node (node views, decision (c)).
        #[cfg(feature = "windowing")]
        if let FocusTarget::Input(nid) | FocusTarget::Node(nid) | FocusTarget::Select(nid) =
            self.focus_target
            && let Some(ks) = key_str.as_deref()
            && let Some((container, host)) = self
                .doc
                .as_ref()
                .and_then(|d| Self::node_view_around(&d.borrow().tree, nid))
            && let Some(handle) = crate::editor::editor_for_doc(self.doc_key(), container)
        {
            let offered = crate::editor::EditorKey {
                key: ks,
                primary: ctrl,
                ctrl: modifiers.ctrl,
                meta: modifiers.meta,
                shift,
                alt,
                repeat: repeat == KeyRepeat::Repeat,
            };
            if handle.offer_node_view_key(host, &offered) {
                // An Escape posted a focus request for the editor: apply
                // it now, so the next key reaches the editor.
                self.drain_focus_requests(false);
                self.refresh_editor_overlays();
                self.resolve_and_repaint(vp_w, vp_h);
                actions.push(AppAction::RequestRedraw);
                return;
            }
        }

        //    The element's own `onkeydown` (issue #1678): the focused
        //    `<input>`/`<textarea>` or focusable node, then its ancestors,
        //    nearest first. A consumed key goes no further: not to the key
        //    entries, the dismiss stack, the field's own editing, `onsubmit`,
        //    activation or Tab.
        if let Some(kd) = &key_data {
            let claim = self.focus_target;
            let consumed = self.offer_element_key(kd);
            // A handler that moved the focus (`other.focus()`) moved
            // it for this key too: the rest of the arm runs against
            // the new claim, as Chrome inserts an unconsumed key into
            // the element focused during its keydown.
            self.drain_focus_requests(false);
            // Space activates on its release, on the element that
            // got the press: a Space whose handler moved the focus
            // to another button clicks nothing (Chrome). Enter, which
            // activates on the press, goes to the new focus.
            let space_left_its_element = kd.is_space()
                && self.focus_target != claim
                && matches!(self.focus_target, FocusTarget::Node(_));
            if consumed || space_left_its_element {
                self.resolve_and_repaint(vp_w, vp_h);
                actions.push(AppAction::RequestRedraw);
                return;
            }
        }

        // 4. The open popups' key entries (`push_key_handler`), then for
        //    Escape the dismiss stack: the components' `document` listeners.
        if let Some(kd) = &key_data
            && events::dispatch_popup_key(kd)
        {
            // #671: the open `<select>` popup's dismiss entry can only
            // ask — closing it needs `&mut self`, which a dismiss handler
            // does not have. Answer it here, the moment the dispatch that
            // set the flag returns. (Its own keys took Escape in step 1; the
            // entry answers a `dispatch_dismiss` from app code.)
            if self.select_dismiss_asked.replace(false) {
                self.close_select_popup_returning_focus();
                self.resolve_and_repaint(vp_w, vp_h);
            }
            // The text context menu's entry asks the same way (#813);
            // the field keeps the keyboard, so there is no focus to
            // return.
            if self.text_menu_dismiss_asked.replace(false) {
                self.close_text_context_menu();
                self.resolve_and_repaint(vp_w, vp_h);
            }
            actions.push(AppAction::RequestRedraw);
            return;
        }

        // 5 and 6. Route by the focus arbiter (design A10): exactly one target
        //    owns keyboard input. In each arm the app-wide key listeners
        //    (`on_app_key`, #1784) come before rinch's defaults, as a
        //    browser's `window` listener runs before its default action.
        match self.focus_target {
            #[cfg(feature = "windowing")]
            FocusTarget::Editor(container) => {
                if let Some(handle) = crate::editor::editor_for_doc(self.doc_key(), container) {
                    if key_data.as_ref().is_some_and(|kd| self.offer_app_key(kd)) {
                        // A listener took it from the keymap (#3062).
                    } else if self.focus_target != FocusTarget::Editor(container) {
                        // A listener moved the keyboard off the editor: the
                        // editor no longer holds it, so its keymap does not
                        // act on the key.
                    } else if key == KeyCode::Tab && !ctrl && !alt && !modifiers.meta {
                        // Tab and Shift+Tab are offered to the editor
                        // (`tab_key`: a table cell, a list binding that
                        // applies, per its `TabBehavior`), and when it
                        // has no use for them they go to the next and
                        // the previous Tab stop, from the editor's place
                        // in the document (`handle_tab`). A read-only
                        // editor never uses them (#833), nor one set to
                        // `MoveFocus`. Before #2210 a Tab in a plain
                        // paragraph ran the keymap's `sinkListItem`,
                        // which did nothing, and the matched binding was
                        // consumed anyway: a keyboard trap.
                        if !handle.tab_key(shift) {
                            self.handle_tab(shift);
                        }
                    } else {
                        self.dispatch_new_editor_key(
                            &handle,
                            key,
                            logical_key.as_deref(),
                            text.as_deref(),
                            shift,
                            ctrl,
                            alt,
                            modifiers,
                            (vp_w, vp_h),
                        );
                    }
                    // Position the caret against the current layout (dirtying
                    // its block if it reparented), re-layout, then the
                    // post-layout caret pass finalizes it with fresh geometry.
                    self.refresh_editor_overlays();
                    self.resolve_and_repaint(vp_w, vp_h);
                    actions.push(AppAction::RequestRedraw);
                } else {
                    // The focused editor was unmounted out from under us:
                    // drop the stale focus so this key (and future ones)
                    // fall through to the global handlers instead of being
                    // silently swallowed.
                    self.focus_target = FocusTarget::None;
                    self.sync_keyboard_editor();
                }
            }
            FocusTarget::Surface(surface_id) => {
                let surface_key_data = crate::render_surface::SurfaceKeyData {
                    key: key_str.clone().unwrap_or_default(),
                    code: format!("{:?}", key),
                    ctrl,
                    shift,
                    alt,
                    meta: modifiers.meta,
                };
                // Forward KeyDown + text input to the focused surface.
                // Always delivered, whatever the key turns out to be
                // claimed by below — `set_event_handler` is the input
                // path and issue #482's fix below does not touch it.
                crate::render_surface::dispatch_surface_event(
                    surface_id,
                    crate::render_surface::SurfaceEvent::KeyDown(surface_key_data.clone()),
                );
                if let Some(ref t) = text {
                    if !t.is_empty() && t.chars().all(|c| !c.is_control()) {
                        crate::render_surface::dispatch_surface_event(
                            surface_id,
                            crate::render_surface::SurfaceEvent::TextInput(t.clone()),
                        );
                    }
                }
                // Issue #482: a focused surface used to swallow every
                // key unconditionally, deafening a host's own
                // window-level keybindings (undo, delete, …) and
                // rinch's own DevTools/inspect/Tab handling the moment
                // a user clicked the canvas — whether or not the
                // surface did anything with the key. Now it stops
                // here only for a key the surface's own
                // `set_key_handler` claims (default: none); anything
                // else falls through to exactly the global fallback an
                // unfocused canvas already gets.
                if crate::render_surface::dispatch_surface_key_event(surface_id, &surface_key_data)
                {
                    // A claimed key is the surface's, release and all
                    // (issue #1784): the listeners did not hear this press,
                    // so its release reaches only one that heard the key go
                    // down before (a repeat of a key held into the surface).
                    actions.push(AppAction::RequestRedraw);
                } else if key_data.as_ref().is_some_and(|kd| self.offer_app_key(kd)) {
                    self.resolve_and_repaint(vp_w, vp_h);
                    actions.push(AppAction::RequestRedraw);
                } else {
                    self.handle_unclaimed_key_fallback(
                        key,
                        text.as_ref(),
                        shift,
                        ctrl,
                        alt,
                        modifiers,
                        repeat,
                        vp_w,
                        vp_h,
                        actions,
                    );
                }
            }
            // An open native <select> popup took its keys in step 1; one
            // with no popup state left has nothing to do with a key.
            FocusTarget::Select(_) => {}
            // No widget owns the key, a plain `<input>` does (its editing
            // commands live in the global handlers, gated internally on
            // `focused_input_state`), or a generic focusable node does
            // (Enter/Space activate it below; everything else falls
            // through so Tab keeps moving). Falls through to DevTools /
            // inspect / Tab navigation / read-only text-selection caret
            // motion.
            FocusTarget::Input(_) | FocusTarget::None | FocusTarget::Node(_) => {
                if self.offer_registered_node_key(
                    key,
                    key_str.as_ref(),
                    ctrl,
                    shift,
                    alt,
                    modifiers.meta,
                    actions,
                ) {
                    return;
                }
                // The app-wide key listeners (issue #1784), ahead of
                // the field's editing, Tab, activation and `onsubmit`
                // — as a browser's `window` listener runs before its
                // default action — and with nothing focused too.
                if key_data.as_ref().is_some_and(|kd| self.offer_app_key(kd)) {
                    self.resolve_and_repaint(vp_w, vp_h);
                    actions.push(AppAction::RequestRedraw);
                    return;
                }
                self.handle_unclaimed_key_fallback(
                    key,
                    text.as_ref(),
                    shift,
                    ctrl,
                    alt,
                    modifiers,
                    repeat,
                    vp_w,
                    vp_h,
                    actions,
                );
            }
        }
    }

    /// A key release.
    pub(super) fn handle_key_up(
        &mut self,
        key: KeyCode,
        logical_key: Option<String>,
        modifiers: Modifiers,
        actions: &mut Vec<AppAction>,
    ) {
        // Release the Enter/Space activation latch (issue #228): the
        // next KeyDown of this key is a fresh physical press. Only a
        // backend answering `KeyRepeat::Unknown` reads the latch
        // (issue #463), but clearing it unconditionally keeps a mixed
        // backend consistent and costs one comparison.
        if self.node_activation_held == Some(key) {
            self.node_activation_held = None;
        }

        let shift = modifiers.shift;
        let ctrl = modifiers.primary();
        let alt = modifiers.alt;

        // Spelled by the same function as the press, from the same
        // fields (issue #337). A release carries no `text` — a key
        // inserts nothing on the way up — so `hook_key_str` resolves it
        // through `logical_key` and then the physical table, which is
        // exactly what the press falls back to once a modifier has
        // suppressed its text. That is what makes a press and its
        // release agree **by construction** rather than by
        // coincidence: a consumer pairing them by `key` (the whole
        // point of hearing releases — "is W still held") cannot be
        // handed `"a"` down and `"q"` up on a non-QWERTY layout.
        let key_str: Option<String> = hook_key_str(key, None, logical_key.as_deref());

        tracing::trace!(?key, ?key_str, shift, ctrl, alt, "KeyUp event");

        // 1. The interceptor, then the element's `onkeyup`, then the
        //    app-wide listeners: the KeyDown order (issue #3062).
        //    Its **return value is ignored**: there is nothing
        //    downstream to suppress. The only runtime work a release
        //    does is clear the activation latch — which must happen
        //    whatever a handler thinks, or a consumed release strands
        //    the latch and the next press is swallowed — and the
        //    surface forward below, which is the surface's own claim.
        //    A redraw is requested unconditionally (issue #486): a
        //    handler that writes a `Signal` already gets one through
        //    the signal-change callback, but one that mutates the DOM
        //    directly through a `NodeHandle` setter has no such
        //    fallback, and named damage marks a node dirty with no
        //    way to ask the shell to paint a frame at all.
        if let Some(ref ks) = key_str {
            let key_data = events::KeyEventData::new(ks.clone(), format!("{:?}", key))
                .with_modifiers(ctrl, shift, alt, modifiers.meta)
                .with_kind(events::KeyEventKind::Up);
            // The interceptor hears every release, before the element's
            // own `onkeyup` (issue #1678), whose consumption stops only
            // the walk to its ancestors: a release has no default to
            // prevent, and a held-key tracker must see every one.
            events::dispatch_key_interceptor(&key_data);
            self.offer_element_key(&key_data);
            // The app-wide listeners that heard its press, and only those,
            // hear a release (#1784, #3062), whichever window it comes in:
            // `dispatch_app_key` pairs them per listener.
            self.offer_app_key(&key_data);
            actions.push(AppAction::RequestRedraw);
        }

        // 2. Then the focus arbiter's holder, again mirroring KeyDown.
        //    Delivered to whoever holds the claim **at release time**,
        //    browser-style — so a focus change mid-chord can hand a
        //    target a release it never saw pressed. That is the
        //    tradeoff a stateless router makes, and it is why a widget
        //    tracking held keys should treat `on_focus_lost` as
        //    "everything is up".
        match self.focus_target {
            FocusTarget::Surface(surface_id) => {
                crate::render_surface::dispatch_surface_event(
                    surface_id,
                    crate::render_surface::SurfaceEvent::KeyUp(
                        crate::render_surface::SurfaceKeyData {
                            key: key_str.clone().unwrap_or_default(),
                            code: format!("{:?}", key),
                            ctrl,
                            shift,
                            alt,
                            meta: modifiers.meta,
                        },
                    ),
                );
            }
            FocusTarget::Node(id) => {
                // The same stale-claim self-heal the KeyDown arm runs:
                // node ids are recycled slab indices, so a claim whose
                // node was unmounted must not be handed a release that
                // now names an unrelated element.
                if !self.node_target_is_live(id) {
                    self.set_focus_target(FocusTarget::None);
                } else {
                    let key_data = events::KeyEventData::new(
                        key_str.clone().unwrap_or_else(|| format!("{:?}", key)),
                        format!("{:?}", key),
                    )
                    .with_modifiers(ctrl, shift, alt, modifiers.meta)
                    .with_kind(events::KeyEventKind::Up);
                    // Return value ignored for the same reason as the
                    // interceptor above: a release has nothing
                    // downstream to suppress, so the redraw is
                    // requested whether or not the handler claims to
                    // have consumed it (issue #486).
                    crate::focus_registry::offer_key(self.doc_key(), id, &key_data);
                    actions.push(AppAction::RequestRedraw);
                }
            }
            _ => {}
        }
    }
}
