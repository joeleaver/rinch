//! Click handling: processes mouse clicks with hit testing.

use super::*;

impl RinchApp {
    // ── Click handling ───────────────────────────────────────────────────

    pub(super) fn handle_click(&mut self, x: f32, y: f32, _scale_factor: f64) -> Vec<AppAction> {
        let mut actions = Vec::new();
        let Some(doc) = &self.doc else {
            return actions;
        };

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
                    CeAction::Clear { prev_node_id }
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
                    undo_stack: Vec::new(),
                });
                self.register_ce_ops(ce_node_id, dom_cursor);

                // Start mouse-drag selection tracking
                self.ce_selecting = true;

                // Clear regular input focus
                self.focused_input_handler_id = None;
                self.focused_input_value.clear();

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
            CeAction::NoHit => {
                return actions;
            }
        }

        // ── Phase 3: normal click handling (data-oninput, data-rid) ─
        let d = doc.borrow();
        let Some(hit_id) = hit_test(&d.tree, x, y) else {
            return actions;
        };

        // Walk up from hit target to detect text input focus (data-oninput).
        // This must happen before the data-rid walk which may return early.
        let mut found_input_focus = false;
        {
            let mut check = Some(hit_id);
            while let Some(nid) = check {
                if let Some(node) = d.tree.get(nid) {
                    if let Some(oninput_str) = node.attributes.get("data-oninput") {
                        if let Ok(handler_id) = oninput_str.parse::<usize>() {
                            self.focused_input_handler_id = Some(handler_id);
                            self.focused_input_value =
                                node.attributes.get("value").cloned().unwrap_or_default();
                            found_input_focus = true;
                        }
                        break;
                    }
                    check = node.parent;
                } else {
                    break;
                }
            }
        }
        if !found_input_focus {
            self.focused_input_handler_id = None;
            self.focused_input_value.clear();
        }

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
                    });

                    drop(d);
                    events::dispatch_event(events::EventHandlerId(handler_id));
                    let _ = rinch_core::take_pending_focus_request();
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
}
