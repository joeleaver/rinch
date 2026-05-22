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
                // Dispatch MouseDown to the surface
                crate::render_surface::dispatch_surface_event(
                    surface_id,
                    crate::render_surface::SurfaceEvent::MouseDown {
                        x: local_x,
                        y: local_y,
                        button: crate::render_surface::SurfaceMouseButton::from_platform(button),
                    },
                );

                // Focus the surface
                let prev_focused = crate::render_surface::focused_surface_id();
                if prev_focused != Some(surface_id) {
                    // Dispatch FocusLost to the previous surface
                    if let Some(prev_id) = prev_focused {
                        crate::render_surface::dispatch_surface_event(
                            prev_id,
                            crate::render_surface::SurfaceEvent::FocusLost,
                        );
                    }
                    crate::render_surface::set_focused_surface(Some(surface_id));
                    crate::render_surface::dispatch_surface_event(
                        surface_id,
                        crate::render_surface::SurfaceEvent::FocusGained,
                    );

                    // Install keyboard interceptor that forwards all keys to the surface
                    let sid = surface_id;
                    events::set_keyboard_interceptor(move |key_data| {
                        crate::render_surface::dispatch_surface_event(
                            sid,
                            crate::render_surface::SurfaceEvent::KeyDown(
                                crate::render_surface::SurfaceKeyData {
                                    key: key_data.key.clone(),
                                    code: key_data.code.clone(),
                                    ctrl: key_data.ctrl,
                                    shift: key_data.shift,
                                    alt: key_data.alt,
                                    meta: key_data.meta,
                                },
                            ),
                        );
                        true // consume all keys
                    });
                }

                // Clear any existing input/CE focus
                self.clear_input_focus_attrs();
                self.focused_input_handler_id = None;
                self.focused_input_value.clear();
                self.focused_input_state = None;
                self.focused_input_node_id = None;
                if let Some(prev_ce) = self.focused_contenteditable.take() {
                    ce::clear_active_ce_api();
                    self.ce_ops = None;
                    self.set_contenteditable_attributes(prev_ce.ce_node_id, false, 0, 0);
                }

                actions.push(AppAction::RequestRedraw);
                return actions;
            }

            // If clicking outside a focused render surface, unfocus it
            if crate::render_surface::focused_surface_id().is_some() {
                let prev_id = crate::render_surface::focused_surface_id().unwrap();
                crate::render_surface::dispatch_surface_event(
                    prev_id,
                    crate::render_surface::SurfaceEvent::FocusLost,
                );
                crate::render_surface::set_focused_surface(None);
                events::clear_keyboard_interceptor();
            }
        }

        // ── Phase 1: contenteditable detection (short borrow) ───────
        // Do a quick read-only scan to decide if we hit a contenteditable.
        // Gather all needed data, then drop the borrow before mutating.
        enum CeAction {
            /// We hit a contenteditable element — focus it.
            Focus {
                ce_node_id: usize,
                dom_cursor: DomCursor,
                prev_node_id: Option<usize>,
            },
            /// We did NOT hit contenteditable — clear previous if any.
            Clear { prev_node_id: Option<usize> },
            /// Hit a data-rid handler outside CE — preserve CE focus, continue to Phase 3.
            PreserveCe,
            /// No hit at all.
            NoHit,
        }

        let ce_action = {
            let d = doc.borrow();
            if let Some(hit_id) = hit_test(&d.tree, x, y) {
                let mut ce_result = None;
                let mut check = Some(hit_id);
                while let Some(nid) = check {
                    if let Some(node) = d.tree.get(nid) {
                        if let Some(ce_val) = node.attributes.get("contenteditable") {
                            let is_editable =
                                matches!(ce_val.as_str(), "plaintext-only" | "true" | "");
                            if is_editable {
                                let dom_cursor =
                                    Self::compute_dom_cursor_from_click(&d.tree, nid, x, y);
                                ce_result = Some((nid, dom_cursor));
                            }
                            break;
                        }
                        check = node.parent;
                    } else {
                        break;
                    }
                }

                let prev_node_id = self.focused_contenteditable.as_ref().map(|f| f.ce_node_id);

                if let Some((ce_node_id, dom_cursor)) = ce_result {
                    CeAction::Focus {
                        ce_node_id,
                        dom_cursor,
                        prev_node_id,
                    }
                } else {
                    // Check if the click target has a data-rid handler — if so,
                    // preserve CE focus so toolbar buttons can use the CE API.
                    let has_rid = {
                        let mut walk = Some(hit_id);
                        let mut found = false;
                        while let Some(nid) = walk {
                            if let Some(node) = d.tree.get(nid) {
                                if node.attributes.contains_key("data-rid") {
                                    found = true;
                                    break;
                                }
                                walk = node.parent;
                            } else {
                                break;
                            }
                        }
                        found
                    };
                    if has_rid {
                        CeAction::PreserveCe // Preserve CE focus, let Phase 3 handle the click
                    } else {
                        CeAction::Clear { prev_node_id }
                    }
                }
            } else {
                CeAction::NoHit
            }
        }; // d dropped here

        // ── Phase 2: apply contenteditable mutations ────────────────
        match ce_action {
            CeAction::Focus {
                ce_node_id,
                mut dom_cursor,
                prev_node_id,
            } => {
                let input_handler = InputHandler::new()
                    .with_multiline(true)
                    .with_macos(cfg!(target_os = "macos"));

                // Handle double-click (word select) and triple-click (line select)
                let mut anchor = dom_cursor;
                match self.click_count {
                    2 => {
                        // Double-click: select word at cursor position
                        if let Some(doc) = &self.doc {
                            let d = doc.borrow();
                            if let Some(node) = d.tree.get(dom_cursor.node_id)
                                && let Some(text) = node.text_content()
                            {
                                let ws = Self::find_word_start(text, dom_cursor.offset);
                                let we = Self::find_word_end(text, dom_cursor.offset);
                                anchor = DomCursor {
                                    node_id: dom_cursor.node_id,
                                    offset: ws,
                                };
                                dom_cursor = DomCursor {
                                    node_id: dom_cursor.node_id,
                                    offset: we,
                                };
                            }
                        }
                    }
                    3 => {
                        // Triple-click: select all text in the CE
                        if let Some(doc) = &self.doc {
                            let d = doc.borrow();
                            if let Some(first) = Self::first_text_cursor(&d.tree, ce_node_id) {
                                anchor = first;
                            }
                            if let Some(last) = Self::last_text_cursor(&d.tree, ce_node_id) {
                                dom_cursor = last;
                            }
                        }
                    }
                    _ => {} // Single click: cursor already set
                }

                self.focused_contenteditable = Some(ContentEditableFocus {
                    ce_node_id,
                    cursor: dom_cursor,
                    anchor,
                    input_handler,
                });
                self.register_ce_ops(ce_node_id, dom_cursor);

                // Start mouse-drag selection tracking
                self.ce_selecting = true;

                // Clear regular input focus
                self.clear_input_focus_attrs();
                self.focused_input_handler_id = None;
                self.focused_input_value.clear();
                self.focused_input_state = None;
                self.focused_input_node_id = None;

                // Clear previous contenteditable focus attributes
                if let Some(prev_id) = prev_node_id
                    && prev_id != ce_node_id
                {
                    self.set_contenteditable_attributes(prev_id, false, 0, 0);
                }
                // Set cursor/selection attributes on the new focused node
                self.set_contenteditable_attributes_dom(ce_node_id, true, dom_cursor, anchor);
                self.scene_dirty = true;
                actions.push(AppAction::RequestRedraw);
                return actions;
            }
            CeAction::Clear { prev_node_id } => {
                if let Some(prev_id) = prev_node_id {
                    self.focused_contenteditable = None;
                    ce::clear_active_ce_api();
                    self.ce_ops = None;
                    self.set_contenteditable_attributes(prev_id, false, 0, 0);
                    self.scene_dirty = true;
                }
            }
            CeAction::PreserveCe => {
                // CE focus preserved — fall through to Phase 3 for data-rid dispatch
                tracing::debug!("click: PreserveCe — CE focus preserved, dispatching data-rid");
            }
            CeAction::NoHit => {
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
            Some(Self::compute_input_cursor_from_click(&d.tree, nid, x, y))
        } else {
            None
        };

        // Drop the borrow before calling methods that re-borrow self.doc
        drop(d);

        if let Some((nid, handler_id, value)) = found_input_focus {
            // Clear previous input focus attrs if switching to a different input
            if self.focused_input_node_id.is_some() && self.focused_input_node_id != Some(nid) {
                self.clear_input_focus_attrs();
            }
            self.focused_input_handler_id = Some(handler_id);
            self.focused_input_value = value.clone();
            self.focused_input_node_id = Some(nid);

            // Create EditableState from the current value
            let mut state = EditableState::new(StringDocument::with_text(&value));
            let byte_offset = input_cursor_offset.unwrap_or(value.len());
            state.selection = Selection::cursor(byte_offset);
            self.focused_input_state = Some(state);
            self.sync_input_cursor_to_dom();
            self.scene_dirty = true;
        } else {
            // Clicked outside any input — clear focus
            if self.focused_input_node_id.is_some() {
                self.clear_input_focus_attrs();
                self.scene_dirty = true;
            }
            self.focused_input_handler_id = None;
            self.focused_input_value.clear();
            self.focused_input_state = None;
            self.focused_input_node_id = None;
        }

        // Re-borrow for the data-rid walk
        let d = doc.borrow();
        let mut current = Some(hit_id);
        while let Some(node_id) = current {
            if let Some(node) = d.tree.get(node_id) {
                // Check for click handler
                if let Some(rid_str) = node.attributes.get("data-rid")
                    && let Ok(handler_id) = rid_str.parse::<usize>()
                {
                    let text_hit = Self::compute_text_hit_info(&d.tree, hit_id, x, y);

                    let (elem_x, elem_y, elem_w, elem_h) = {
                        let mut ax = node.layout.x;
                        let mut ay = node.layout.y;
                        let mut pid = node.parent;
                        while let Some(p) = pid {
                            if let Some(pn) = d.tree.get(p) {
                                ax += pn.layout.x;
                                ay += pn.layout.y;
                                ax -= pn.scroll_offset.0 as f32;
                                ay -= pn.scroll_offset.1 as f32;
                                pid = pn.parent;
                            } else {
                                break;
                            }
                        }
                        (ax, ay, node.layout.width, node.layout.height)
                    };

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
                    });
                    events::set_click_ancestors(Self::collect_click_ancestors(&d.tree, node_id));

                    drop(d);
                    // Sync CE cursor before handler so toolbar buttons see current selection
                    self.sync_ce_ops_cursor();
                    events::dispatch_event(events::EventHandlerId(handler_id));
                    // Sync selection back from CE API after handler (e.g., toggle_wrap changes it)
                    if let Some(ops) = &self.ce_ops
                        && let Ok(ops) = ops.try_borrow()
                    {
                        let sel = ops.get_selection();
                        if let Some(ce) = self.focused_contenteditable.as_mut() {
                            ce.cursor = sel.head;
                            ce.anchor = sel.anchor;
                        }
                    }
                    // Process any pending focus request from the event handler
                    // (e.g., a handler may call request_focus on an input element).
                    if let Some(focus_node_id) = rinch_core::take_pending_focus_request() {
                        self.try_focus_input(focus_node_id);
                    }
                    actions.push(AppAction::RequestRedraw);
                    return actions;
                }
                // Check for drag-window region
                if node.attributes.contains_key("data-drag-window") {
                    drop(d);
                    actions.push(AppAction::DragWindow);
                    return actions;
                }
                current = node.parent;
            } else {
                break;
            }
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

        let mut abs_x = block_node.layout.x;
        let mut abs_y = block_node.layout.y;
        let mut parent_id = block_node.parent;
        while let Some(pid) = parent_id {
            if let Some(pn) = tree.get(pid) {
                abs_x += pn.layout.x;
                abs_y += pn.layout.y;
                abs_x -= pn.scroll_offset.0 as f32;
                abs_y -= pn.scroll_offset.1 as f32;
                parent_id = pn.parent;
            } else {
                break;
            }
        }

        let rel_x = (click_x - abs_x).max(0.0);
        let rel_y = (click_y - abs_y).max(0.0);

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
    /// Builds a temporary Parley layout from the node's value and font properties,
    /// then uses `byte_offset_from_position` to find the character at click coords.
    fn compute_input_cursor_from_click(
        tree: &rinch_dom::NodeTree,
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

        // Compute node's absolute position
        let mut abs_x = node.layout.x;
        let mut abs_y = node.layout.y;
        let mut parent_id = node.parent;
        while let Some(pid) = parent_id {
            if let Some(pn) = tree.get(pid) {
                abs_x += pn.layout.x;
                abs_y += pn.layout.y;
                abs_x -= pn.scroll_offset.0 as f32;
                abs_y -= pn.scroll_offset.1 as f32;
                parent_id = pn.parent;
            } else {
                break;
            }
        }

        let padding_left = node.computed_style.padding_left.to_px();
        let padding_top = node.computed_style.padding_top.to_px();

        // Local coordinates within the text area
        let local_x = (click_x - abs_x - padding_left).max(0.0);
        let local_y = (click_y - abs_y - padding_top).max(0.0);

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

        // Use the tree's font context to build the layout
        // We can't borrow font_cx mutably here since we only have &NodeTree.
        // Instead, create a temporary font context for hit testing.
        let mut font_cx = parley::FontContext::new();
        let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();

        let mut builder = layout_cx.ranged_builder(&mut font_cx, display_text, 1.0, true);
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
    pub(crate) fn surface_local_coords(
        tree: &rinch_dom::NodeTree,
        surface_dom_node: usize,
        screen_x: f32,
        screen_y: f32,
    ) -> (f32, f32) {
        let node = match tree.get(surface_dom_node) {
            Some(n) => n,
            None => return (screen_x, screen_y),
        };
        let mut abs_x = node.layout.x;
        let mut abs_y = node.layout.y;
        let mut pid = node.parent;
        while let Some(p) = pid {
            if let Some(pn) = tree.get(p) {
                abs_x += pn.layout.x;
                abs_y += pn.layout.y;
                abs_x -= pn.scroll_offset.0 as f32;
                abs_y -= pn.scroll_offset.1 as f32;
                pid = pn.parent;
            } else {
                break;
            }
        }
        (screen_x - abs_x, screen_y - abs_y)
    }
}
