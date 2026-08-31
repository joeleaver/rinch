//! Click handling: processes mouse clicks with hit testing.

use super::*;

impl RinchApp {
    // ── Click handling ───────────────────────────────────────────────────

    pub(super) fn handle_click(
        &mut self,
        x: f32,
        y: f32,
        scale_factor: f64,
        viewport_width: f32,
        viewport_height: f32,
    ) -> Vec<AppAction> {
        self.handle_click_with_button(
            x,
            y,
            scale_factor,
            MouseButton::Left,
            viewport_width,
            viewport_height,
        )
    }

    pub(super) fn handle_click_with_button(
        &mut self,
        x: f32,
        y: f32,
        _scale_factor: f64,
        button: MouseButton,
        viewport_width: f32,
        viewport_height: f32,
    ) -> Vec<AppAction> {
        let mut actions = Vec::new();
        let Some(doc) = self.doc.clone() else {
            return actions;
        };
        let doc = &doc;

        // ── Phase -1: open native <select> popup is modal ────────────
        // While a popup is open its backdrop covers the window: the click either
        // commits an option, no-ops on the panel, or dismisses. Handled here so it
        // pre-empts every other phase.
        if self.is_select_open() {
            self.handle_open_select_click(x, y, viewport_width, viewport_height);
            actions.push(AppAction::RequestRedraw);
            return actions;
        }

        // ── Phase 0: render surface detection ────────────────────────
        // Check if the click lands on a render surface. If so, dispatch
        // the event, set focus, and return early.
        {
            let surface_hit = {
                let d = doc.borrow();
                if let Some(hit_id) = hit_test(&d.tree, x, y) {
                    Self::find_render_surface_at(&d.tree, hit_id, x, y)
                } else {
                    None
                }
            };

            if let Some((surface_id, local_x, local_y)) = surface_hit {
                // Dispatch MouseDown to the surface (every click, regardless of
                // whether focus changes).
                crate::render_surface::dispatch_surface_event(
                    surface_id,
                    crate::render_surface::SurfaceEvent::MouseDown {
                        x: local_x,
                        y: local_y,
                        button: crate::render_surface::SurfaceMouseButton::from_platform(button),
                    },
                );

                // Take keyboard focus through the arbiter: it tears down whatever
                // was focused before (input / CE / editor / a prior surface), then
                // we register this surface. `set_focused_surface` dispatches
                // `FocusGained` (and `FocusLost` to any prior surface) — surfaces
                // are routed by `FocusTarget::Surface` now, not the old keyboard
                // interceptor hijack.
                if self.set_focus_target(FocusTarget::Surface(surface_id)) {
                    crate::render_surface::set_focused_surface(Some(surface_id));
                }

                actions.push(AppAction::RequestRedraw);
                return actions;
            }

            // Clicking outside any surface drops surface focus (the arbiter's
            // teardown dispatches FocusLost). Other targets are taken below.
            if matches!(self.focus_target, FocusTarget::Surface(_)) {
                self.set_focus_target(FocusTarget::None);
            }
        }

        // ── Phase 0.5: open a native <select> popup ──────────────────
        // A click on a closed `<select>` (its options are display:none, so the hit
        // lands on the control itself) opens the combobox popup and consumes the
        // click.
        {
            let select_hit = {
                let d = doc.borrow();
                hit_test(&d.tree, x, y).and_then(|hid| Self::select_ancestor(&d.tree, hid))
            };
            if let Some(select_id) = select_hit {
                self.open_select_popup(select_id, viewport_width, viewport_height);
                actions.push(AppAction::RequestRedraw);
                return actions;
            }
        }

        // ── Phase 1: editor focus management (short borrow) ─────────
        // A click that misses every `data-rid` handler blurs the focused editor
        // (unless it lands inside the editor's own container chrome). A click
        // that *hits* a `data-rid` (e.g. a toolbar button) preserves editor
        // focus so "click Bold, keep typing" works (audit S2) and falls through
        // to Phase 3 to dispatch the handler. Left-clicks inside the editor are
        // handled earlier by `try_new_editor_click` and never reach here.
        enum ClickFocus {
            /// Hit a `data-rid` handler — preserve editor focus, continue.
            PreserveEditor,
            /// No `data-rid` under the click — blur the editor if outside it.
            Blur,
            /// No hit at all.
            NoHit,
        }

        let click_focus = {
            let d = doc.borrow();
            if let Some(hit_id) = hit_test(&d.tree, x, y) {
                let mut walk = Some(hit_id);
                let mut has_rid = false;
                while let Some(nid) = walk {
                    if let Some(node) = d.tree.get(nid) {
                        if node.attributes.contains_key("data-rid") {
                            has_rid = true;
                            break;
                        }
                        walk = node.parent;
                    } else {
                        break;
                    }
                }
                if has_rid {
                    ClickFocus::PreserveEditor
                } else {
                    ClickFocus::Blur
                }
            } else {
                ClickFocus::NoHit
            }
        }; // d dropped here

        // ── Phase 2: apply the focus change ─────────────────────────
        match click_focus {
            ClickFocus::Blur => {
                // A click on the focused editor's OWN chrome (padding / empty
                // area) is not a blur — only blur when the click landed outside
                // its container. (Left clicks inside the editor never reach here:
                // they short-circuit in `try_new_editor_click`. This covers
                // right-click and clicks elsewhere in the window.)
                #[cfg(feature = "desktop")]
                if let FocusTarget::Editor(focused) = self.focus_target
                    && self.editor_container_at(x, y) != Some(focused)
                {
                    self.set_focus_target(FocusTarget::None);
                }
            }
            ClickFocus::PreserveEditor => {
                // Editor focus preserved — fall through to Phase 3 for the
                // `data-rid` dispatch (the toolbar-button case).
            }
            ClickFocus::NoHit => {
                return actions;
            }
        }

        // ── Phase 2.5: selectable text detection ─────────────────────
        // If we didn't land on a CE element, check if we hit selectable text.
        // Gather data from the doc borrow, then mutate after dropping the borrow.
        enum TextSelAction {
            StartSelection { ifc_node_id: usize, offset: usize },
            Clear,
        }
        let text_sel_action = {
            let d = doc.borrow();
            if let Some(hit_id) = hit_test(&d.tree, x, y) {
                if let Some(ifc_node_id) = Self::find_selectable_ifc(&d.tree, hit_id) {
                    let offset = Self::compute_ifc_offset_from_click(&d.tree, ifc_node_id, x, y);
                    TextSelAction::StartSelection {
                        ifc_node_id,
                        offset,
                    }
                } else {
                    TextSelAction::Clear
                }
            } else {
                TextSelAction::Clear
            }
        };
        match text_sel_action {
            TextSelAction::StartSelection {
                ifc_node_id,
                offset,
            } => {
                let prev_ifc = self.text_selection.as_ref().map(|s| s.ifc_node_id);
                if prev_ifc.is_some() && prev_ifc != Some(ifc_node_id) {
                    self.clear_text_selection();
                }
                self.text_selection = Some(TextSelection {
                    ifc_node_id,
                    anchor_offset: offset,
                    focus_offset: offset,
                });
                self.text_selecting = true;
                self.set_text_selection_attributes(ifc_node_id, offset, offset);
                self.scene_dirty = true;
                actions.push(AppAction::RequestRedraw);
            }
            TextSelAction::Clear => {
                self.clear_text_selection();
            }
        }

        // ── Phase 3: normal click handling (data-oninput, data-rid) ─
        let d = doc.borrow();
        let Some(hit_id) = hit_test(&d.tree, x, y) else {
            return actions;
        };

        // Walk up from hit target to detect text input focus (data-oninput).
        // This must happen before the data-rid walk which may return early.
        let mut found_input_focus: Option<(usize, usize, String)> = None; // (node_id, handler_id, value)
        {
            let mut check = Some(hit_id);
            while let Some(nid) = check {
                if let Some(node) = d.tree.get(nid) {
                    if let Some(oninput_str) = node.attributes.get("data-oninput") {
                        if let Ok(handler_id) = oninput_str.parse::<usize>() {
                            let value = node.attributes.get("value").cloned().unwrap_or_default();
                            found_input_focus = Some((nid, handler_id, value));
                        }
                        break;
                    }
                    check = node.parent;
                } else {
                    break;
                }
            }
        }

        // Compute click-to-cursor byte offset if we're focusing an input
        let input_cursor_offset = if let Some((nid, _, _)) = found_input_focus {
            Some(Self::compute_input_cursor_from_click(
                &d.tree,
                &mut self.hit_test_font_cx,
                &mut self.hit_test_layout_cx,
                nid,
                x,
                y,
            ))
        } else {
            None
        };

        // A click on or inside a keyboard-focused generic node keeps its focus
        // (the ring already dropped on mousedown); a click elsewhere blurs it.
        let hit_inside_focused_node = if let FocusTarget::Node(fid) = self.focus_target {
            let mut cur = Some(hit_id);
            loop {
                match cur {
                    Some(nid) if nid == fid => break true,
                    Some(nid) => cur = d.tree.get(nid).and_then(|n| n.parent),
                    None => break false,
                }
            }
        } else {
            false
        };

        // Pre-resolve the click's claim (data-rid / data-drag-window) BEFORE
        // any focus change below: a blurred input's `data-onchange` commit
        // (issue #226) is user code that may re-render the clicked subtree, and
        // node ids are recycled slab indices — walking with the stale `hit_id`
        // after the commit dispatched an unrelated element's handler in the
        // #244 review's empirical probe. The claim is re-verified against the
        // post-commit tree before dispatching.
        //
        // The liveness probe is what keeps a *stale* `data-rid` from swallowing
        // the click. Disposing a scope frees its handlers (issue #141) but
        // nothing strips the attribute from a node that outlives them, and this
        // walk commits to the first node carrying one — so without the probe a
        // dead button would consume clicks that belong to the row, backdrop or
        // modal wrapping it, silently and permanently.
        enum ClickClaim {
            Rid(usize, usize), // (node_id, handler_id)
            DragWindow,
        }
        let click_claim = {
            let mut claim = None;
            let mut cur = Some(hit_id);
            while let Some(node_id) = cur {
                let Some(node) = d.tree.get(node_id) else {
                    break;
                };
                if let Some(rid_str) = node.attributes.get("data-rid")
                    && let Ok(handler_id) = rid_str.parse::<usize>()
                    && events::has_click_handler(events::EventHandlerId(handler_id))
                {
                    claim = Some(ClickClaim::Rid(node_id, handler_id));
                    break;
                }
                if node.attributes.contains_key("data-drag-window") {
                    claim = Some(ClickClaim::DragWindow);
                    break;
                }
                cur = node.parent;
            }
            claim
        };

        // Drop the borrow before calling methods that re-borrow self.doc
        drop(d);

        if let Some((nid, handler_id, value)) = found_input_focus {
            // Take input focus through the arbiter: tears down a prior surface /
            // CE / editor / different input (re-clicking the same input is a no-op
            // teardown, so we just move its cursor below). The blurred input's
            // change commit is deferred until this input's state is installed —
            // the handler is user code and may rewrite the very input being
            // focused (#244 review).
            let (focus_changed, work) = self.set_focus_target_deferred(FocusTarget::Input(nid));
            // A re-click inside the already-focused input absorbs the DOM value
            // into the buffer below without moving the baseline, so a write that
            // landed since the last frame would look like a user edit and commit
            // `onchange` by itself. Adopt it first, under the baseline rule (a
            // no-op on a fresh focus: the teardown cleared the state).
            self.adopt_focused_input_value_from_dom();
            self.focused_input_handler_id = Some(handler_id);
            self.focused_input_value = value.clone();
            self.focused_input_node_id = Some(nid);
            if focus_changed {
                // A fresh gesture: snapshot the commit baseline (issue #226). A
                // re-click inside the already-focused input is a caret move —
                // the gesture (and its baseline) continues.
                self.focused_input_baseline = value.clone();
            }

            // Create EditableState from the current value
            let mut state = EditableState::new(StringDocument::with_text(&value));
            let byte_offset = input_cursor_offset.unwrap_or(value.len());
            state.selection = Selection::cursor(byte_offset);
            self.focused_input_state = Some(state);
            self.sync_input_cursor_to_dom();
            self.scene_dirty = true;

            // Installation complete: fire the blurred input's commit, then adopt
            // any rewrite its handler made to THIS input (the Enter path's
            // pattern — a no-op when the DOM value already matches, so the
            // click-placed caret survives the common case).
            let commit_fired = Self::fire_focus_work(work);
            if commit_fired {
                self.adopt_focused_input_value_from_dom();
                self.focused_input_baseline = self.focused_input_value.clone();
            }
        } else if !hit_inside_focused_node
            && matches!(
                self.focus_target,
                FocusTarget::Input(_) | FocusTarget::Surface(_) | FocusTarget::Node(_)
            )
        {
            // Clicked a non-input target: drop input/surface/generic-node focus.
            // Editor focus is preserved here so its toolbar buttons (data-rid,
            // dispatched below) keep working; an empty/non-toolbar click already
            // blurred the editor in Phase 2 (`ClickFocus::Blur`).
            self.set_focus_target(FocusTarget::None);
        }

        // Dispatch the pre-resolved click claim. For a data-rid claim the node
        // is re-verified against the (possibly commit-mutated) tree: it must
        // still carry the SAME handler id, still live — a re-rendered or
        // recycled slot fails the check and the click lands nowhere, exactly
        // like a browser click on an element that was removed under the
        // pointer.
        match click_claim {
            Some(ClickClaim::Rid(node_id, handler_id)) => {
                let d = doc.borrow();
                let still_valid = d
                    .tree
                    .get(node_id)
                    .and_then(|n| n.attributes.get("data-rid"))
                    .and_then(|s| s.parse::<usize>().ok())
                    .is_some_and(|h| {
                        h == handler_id
                            && events::has_click_handler(events::EventHandlerId(handler_id))
                    });
                if !still_valid {
                    return actions;
                }
                let text_hit = Self::compute_text_hit_info(&d.tree, hit_id, x, y);

                // The element bounds a handler reads are the bounds it is
                // *painted* at, which is what rinch-web already reports here
                // (`getBoundingClientRect`) and what `Drag::percent()` divides
                // by. The hand-rolled parent-chain sum this replaces composed no
                // transform and had no `position: fixed` exception, so a slider
                // inside the `translate(-50%, -50%)` centring idiom measured
                // itself against a box nobody can see (#203). Hit testing works
                // in layout pixels, so the painted box is asked for at scale 1.
                let (elem_x, elem_y, elem_w, elem_h) = painted_element_box(&d.tree, node_id);

                events::set_click_context(events::ClickContext {
                    mouse_x: x,
                    mouse_y: y,
                    element_x: elem_x,
                    element_y: elem_y,
                    element_width: elem_w,
                    element_height: elem_h,
                    text_hit,
                    viewport_width,
                    viewport_height,
                    button: events::MouseButton::Left,
                    modifiers: self.modifier_state(),
                });
                events::set_click_ancestors(Self::collect_click_ancestors(&d.tree, node_id));

                drop(d);
                events::dispatch_event(events::EventHandlerId(handler_id));
                // Process any pending focus request from the event handler
                // (e.g., a handler may call request_focus on an input element).
                if let Some(focus_node_id) = rinch_core::take_pending_focus_request(self.doc_key())
                {
                    self.try_focus_input(focus_node_id);
                }
                actions.push(AppAction::RequestRedraw);
                return actions;
            }
            Some(ClickClaim::DragWindow) => {
                actions.push(AppAction::DragWindow);
                return actions;
            }
            None => {}
        }
        actions
    }

    /// Compute text hit info for click-to-position in rich text editors.
    fn compute_text_hit_info(
        tree: &rinch_dom::NodeTree,
        hit_id: usize,
        click_x: f32,
        click_y: f32,
    ) -> events::TextHitInfo {
        let mut block_index = 0usize;
        let mut block_node_id = None;
        let mut current = Some(hit_id);

        while let Some(node_id) = current {
            if let Some(node) = tree.get(node_id) {
                if let Some(idx_str) = node.attributes.get("data-block-index")
                    && let Ok(idx) = idx_str.parse::<usize>()
                {
                    block_index = idx;
                    block_node_id = Some(node_id);
                    break;
                }
                current = node.parent;
            } else {
                break;
            }
        }

        let Some(block_id) = block_node_id else {
            return events::TextHitInfo::default();
        };

        let Some(block_node) = tree.get(block_id) else {
            return events::TextHitInfo::default();
        };

        // The click in the block's own space — Parley's layout lives there, so a
        // transformed block still resolves to the character under the pointer
        // (#203). The parent-chain sum this replaces composed no transform and
        // made no `position: fixed` exception.
        let (local_x, local_y) = pointer_in_node(tree, block_id, click_x, click_y);
        let rel_x = local_x.max(0.0);
        let rel_y = local_y.max(0.0);

        let byte_offset = if let Some(ref layout) = block_node.text_layout {
            byte_offset_from_position(&layout.layout, rel_x, rel_y)
        } else if let Some(ref layout) = block_node.cached_text_parley {
            byte_offset_from_position(layout, rel_x, rel_y)
        } else {
            let mut offset = 0usize;
            for &child_id in &block_node.children {
                if let Some(child) = tree.nodes.get(child_id)
                    && let Some(ref layout) = child.cached_text_parley
                {
                    offset = byte_offset_from_position(layout, rel_x, rel_y);
                    break;
                }
            }
            offset
        };

        events::TextHitInfo {
            block_index,
            byte_offset,
            inline_root_node_id: block_id,
            valid: true,
        }
    }

    /// Compute the byte offset for a click position within a text input node.
    ///
    /// Rebuilds the Parley layout `paint_input_value` drew the field's text with
    /// — same font stack, same font context, same wrap width — and asks it which
    /// character the click landed on. The two layouts have to agree glyph for
    /// glyph, and so do the coordinate spaces: the click arrives in layout
    /// pixels, and the box's painted origin is where paint and hit testing say
    /// it is, IFC content-box offset included.
    fn compute_input_cursor_from_click(
        tree: &rinch_dom::NodeTree,
        font_cx: &mut parley::FontContext,
        layout_cx: &mut parley::LayoutContext<peniko::Brush>,
        node_id: usize,
        click_x: f32,
        click_y: f32,
    ) -> usize {
        let Some(node) = tree.get(node_id) else {
            return 0;
        };

        let value = node
            .attributes
            .get("value")
            .map(|s| s.as_str())
            .unwrap_or("");
        if value.is_empty() {
            return 0;
        }

        // The click in the field's own space, relative to its border-box origin
        // — the same descent paint makes, so the IFC content-box offset a field
        // a text flow positions needs is already inside it. A `<textarea>`
        // between two blocks is one: the anonymous block box the IFC wraps it in
        // clones the *containing block's* computed style (ifc.rs:672), padding
        // included, and paint reads that padding when it hands
        // `paint_inline_layout` a content-box origin.
        let (box_x, box_y) = pointer_in_node(tree, node_id, click_x, click_y);

        let padding_left = node.computed_style.padding_left.to_px();
        let padding_top = node.computed_style.padding_top.to_px();

        // Local coordinates within the text area
        let local_x = (box_x - padding_left).max(0.0);
        let local_y = (box_y - padding_top).max(0.0);

        // Build a Parley layout matching paint_input_value's parameters
        let font_size = node.computed_style.font_size;
        let font_weight = node.computed_style.font_weight;
        let font_family = if node.computed_style.font_family.is_empty() {
            "sans-serif".to_string()
        } else {
            node.computed_style.font_family.clone()
        };

        // Password masking: map click position through bullet text
        let is_password = node.attributes.get("type").map(|s| s.as_str()) == Some("password");
        let bullet = "\u{2022}";
        let password_display;
        let display_text = if is_password {
            let total_chars = value.chars().count();
            password_display = bullet.repeat(total_chars);
            password_display.as_str()
        } else {
            value
        };

        // Both contexts are the app's own, never fresh ones: a
        // `FontContext::new()` here would re-scan the system on every click and
        // would not have the app's own registered typefaces in it at all, so it
        // would measure the text against a face the painter never used — and a
        // `LayoutContext::new()` would throw away its shaping caches once per
        // click for no gain.
        let mut builder = layout_cx.ranged_builder(font_cx, display_text, 1.0, true);
        builder.push_default(parley::style::StyleProperty::FontSize(font_size));
        builder.push_default(parley::style::StyleProperty::FontStack(
            parley::style::FontStack::Source(std::borrow::Cow::Owned(font_family)),
        ));
        if (font_weight - 400.0).abs() > 1.0 {
            builder.push_default(parley::style::StyleProperty::FontWeight(
                parley::style::FontWeight::new(font_weight),
            ));
        }

        let content_width = node.layout.width - padding_left * 2.0;
        let mut text_layout = builder.build(display_text);
        text_layout.break_all_lines(Some(content_width));

        // For single-line inputs, adjust local_y to account for vertical centering
        let is_textarea = node.tag() == Some("textarea");
        let adjusted_y = if !is_textarea {
            let text_height = text_layout.height();
            let content_height = node.layout.height;
            let vertical_offset = (content_height - text_height) / 2.0 - padding_top;
            (local_y - vertical_offset.max(0.0)).max(0.0)
        } else {
            local_y
        };

        let byte_offset = byte_offset_from_position(&text_layout, local_x, adjusted_y);

        // For password fields, map display byte offset back to real value byte offset
        if is_password {
            let bullet_len = bullet.len();
            let char_index = byte_offset / bullet_len;
            value
                .char_indices()
                .nth(char_index)
                .map(|(i, _)| i)
                .unwrap_or(value.len())
        } else {
            byte_offset
        }
    }

    /// Walk up from `hit_id` looking for a `data-render-surface` attribute.
    ///
    /// Returns `(surface_id, local_x, local_y)` where local coords are
    /// relative to the surface element's top-left corner.
    pub(crate) fn find_render_surface_at(
        tree: &rinch_dom::NodeTree,
        hit_id: usize,
        screen_x: f32,
        screen_y: f32,
    ) -> Option<(usize, f32, f32)> {
        Self::find_render_surface_at_full(tree, hit_id, screen_x, screen_y)
            .map(|(sid, _nid, lx, ly)| (sid, lx, ly))
    }

    /// Like `find_render_surface_at` but also returns the surface's DOM node ID.
    pub(crate) fn find_render_surface_at_full(
        tree: &rinch_dom::NodeTree,
        hit_id: usize,
        screen_x: f32,
        screen_y: f32,
    ) -> Option<(usize, usize, f32, f32)> {
        let mut current = Some(hit_id);
        while let Some(nid) = current {
            if let Some(node) = tree.get(nid) {
                if let Some(id_str) = node.attributes.get("data-render-surface") {
                    if let Ok(surface_id) = id_str.parse::<usize>() {
                        let (local_x, local_y) =
                            Self::surface_local_coords(tree, nid, screen_x, screen_y);
                        return Some((surface_id, nid, local_x, local_y));
                    }
                }
                current = node.parent;
            } else {
                break;
            }
        }
        None
    }

    /// Compute screen-to-local coordinates for a surface DOM node.
    ///
    /// The surface's own space, not the painted box's origin subtracted: the
    /// app renders into a buffer sized from the surface's layout box, so a
    /// pointer inside a `transform: scale()` container has to come back divided
    /// rather than merely shifted (#203).
    pub(crate) fn surface_local_coords(
        tree: &rinch_dom::NodeTree,
        surface_dom_node: usize,
        screen_x: f32,
        screen_y: f32,
    ) -> (f32, f32) {
        if tree.get(surface_dom_node).is_none() {
            return (screen_x, screen_y);
        }
        pointer_in_node(tree, surface_dom_node, screen_x, screen_y)
    }
}
