use super::*;

#[cfg(feature = "debug")]
impl RinchApp {
    // ── Debug commands ───────────────────────────────────────────────────

    pub(crate) fn handle_debug_commands(
        &mut self,
        actions: &mut Vec<AppAction>,
        scale_factor: f64,
        window_size: (u32, u32),
    ) {
        let Some(rx) = self.debug_cmd_rx.take() else {
            return;
        };

        while let Ok(cmd) = rx.0.try_recv() {
            let response = self.execute_debug_command(cmd.kind, actions, scale_factor, window_size);
            let _ = cmd.response_tx.send(response);
        }

        self.debug_cmd_rx = Some(rx);
    }

    pub(crate) fn execute_debug_command(
        &mut self,
        kind: DebugCommandKind,
        actions: &mut Vec<AppAction>,
        scale_factor: f64,
        window_size: (u32, u32),
    ) -> DebugResult {
        match kind {
            DebugCommandKind::Screenshot => {
                // Screenshot is handled by the shell -- we signal that we need
                // a screenshot capture. The shell will paint + capture.
                // For now, return an error indicating the shell must handle this.
                DebugResult::Error {
                    message: "__SCREENSHOT_DELEGATE__".into(),
                }
            }
            DebugCommandKind::DomTree => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                DebugResult::Json {
                    data: rinch_dom::testing::serialize_tree(&d.tree),
                }
            }
            DebugCommandKind::QuerySelector { selector } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                let ids = rinch_dom::testing::query_selector(&d.tree, &selector);
                let nodes: Vec<_> = ids
                    .iter()
                    .filter_map(|&id| rinch_dom::testing::get_node_detail(&d.tree, id))
                    .collect();
                DebugResult::Json { data: json!(nodes) }
            }
            DebugCommandKind::GetNode { id } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                match rinch_dom::testing::get_node_detail(&d.tree, id) {
                    Some(detail) => DebugResult::Json { data: detail },
                    None => DebugResult::Error {
                        message: format!("Node {} not found", id),
                    },
                }
            }
            DebugCommandKind::GetTextContent { id } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                DebugResult::Json {
                    data: json!(rinch_dom::testing::get_text_content(&d.tree, id)),
                }
            }
            DebugCommandKind::Click { x, y } => {
                // Simulate a full click (press + release)
                let click_actions = self.handle_click(x, y, scale_factor);
                actions.extend(click_actions);
                // Clear selection drag — Click is a complete press+release, not a drag start
                self.ce_selecting = false;
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::MouseDown { x, y } => {
                // Same as PlatformEvent::MouseDown{Left}: handle click and start selection tracking
                let click_actions = self.handle_click(x, y, scale_factor);
                actions.extend(click_actions);
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::MouseUp { x: _, y: _ } => {
                rinch_core::stop_drag();
                self.scrollbar_drag = None;
                self.ce_selecting = false;
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::MouseMove { x, y } => {
                self.cursor_pos = Some((x, y));

                // Handle component drag (sliders, floating panels, etc.)
                if rinch_core::update_drag(x, y) {
                    let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                    self.resolve_and_repaint(w, h);
                    actions.push(AppAction::RequestRedraw);
                    return DebugResult::Json { data: json!(null) };
                }

                // Handle contenteditable text selection drag
                if self.ce_selecting
                    && let Some(ref mut ce) = self.focused_contenteditable
                {
                    let ce_node_id = ce.ce_node_id;
                    if let Some(doc) = &self.doc {
                        let new_cursor = {
                            let d = doc.borrow();
                            Self::compute_dom_cursor_from_click(&d.tree, ce_node_id, x, y)
                        };
                        ce.cursor = new_cursor;
                        let anchor = ce.anchor;
                        self.set_contenteditable_attributes_dom(
                            ce_node_id, true, new_cursor, anchor,
                        );
                        self.scene_dirty = true;
                        actions.push(AppAction::RequestRedraw);
                        return DebugResult::Json { data: json!(null) };
                    }
                }

                // Update hover state
                if let Some(doc) = &self.doc {
                    let hovered = {
                        let d = doc.borrow();
                        hit_test(&d.tree, x, y)
                    };
                    let changed = doc.borrow_mut().update_hover(hovered);
                    if changed {
                        if let Some(hit_id) = hovered {
                            Self::dispatch_onenter(doc, hit_id);
                        }
                        actions.push(AppAction::RequestRedraw);
                    }
                }
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::Scroll {
                x,
                y,
                delta_x: _delta_x,
                delta_y,
            } => {
                self.cursor_pos = Some((x, y));

                if let Some(doc) = &self.doc {
                    let hit_node = hit_test(&doc.borrow().tree, x, y);
                    if let Some(hit_node) = hit_node {
                        let mut doc_mut = doc.borrow_mut();
                        if let Some(scroll_node_id) = find_scroll_container(&doc_mut.tree, hit_node)
                        {
                            let content_height =
                                compute_content_height(&doc_mut.tree, scroll_node_id);
                            let visible_height =
                                compute_visible_content_area_height(&doc_mut.tree, scroll_node_id);
                            let max_scroll = (content_height - visible_height).max(0.0);

                            if let Some(node) = doc_mut.tree.nodes.get_mut(scroll_node_id) {
                                let new_y = (node.scroll_offset.1 + delta_y).clamp(0.0, max_scroll);
                                if new_y != node.scroll_offset.1 {
                                    node.scroll_offset.1 = new_y;
                                    node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                                    doc_mut.tree.dirty_nodes.insert(scroll_node_id);
                                }
                            }
                        }
                        drop(doc_mut);
                    }
                }
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::TypeText { text } => {
                for ch in text.chars() {
                    let key = match ch {
                        ' ' => "Space".to_string(),
                        '\n' => "Enter".to_string(),
                        '\t' => "Tab".to_string(),
                        c => c.to_string(),
                    };
                    let key_data = events::KeyEventData {
                        key: key.clone(),
                        code: key,
                        ctrl: false,
                        shift: false,
                        alt: false,
                        meta: false,
                    };
                    let handled = events::dispatch_keyboard_event(&key_data);
                    if !handled {
                        if self.focused_contenteditable.is_some() {
                            // Route to contenteditable handler
                            let key_code = match ch {
                                '\n' => KeyCode::Enter,
                                '\t' => KeyCode::Tab,
                                '\x08' => KeyCode::Backspace,
                                _ => KeyCode::Space, // Use Space as a safe unmapped key
                            };
                            let text_str = ch.to_string();
                            self.handle_contenteditable_key(
                                key_code,
                                Some(&text_str),
                                false,
                                false,
                                false,
                            );
                        } else {
                            // Fallback to handle_text_input for non-intercepted chars
                            self.handle_text_input(&ch.to_string());
                        }
                    }
                }
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::WaitFrame => {
                let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                self.resolve_and_repaint(w, h);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::GetComputedStyles { id } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                match d.tree.get(id) {
                    Some(node) => DebugResult::Json {
                        data: json!(&node.computed_style),
                    },
                    None => DebugResult::Error {
                        message: format!("Node {} not found", id),
                    },
                }
            }
            DebugCommandKind::CloseApp => {
                actions.push(AppAction::Exit);
                DebugResult::Json {
                    data: json!({"status": "closing"}),
                }
            }
            DebugCommandKind::KeyPress { key, shift, ctrl } => {
                let key_data = events::KeyEventData {
                    key: key.clone(),
                    code: key.clone(),
                    ctrl,
                    shift,
                    alt: false,
                    meta: false,
                };
                let handled = events::dispatch_keyboard_event(&key_data);

                if handled {
                    actions.push(AppAction::RequestRedraw);
                }

                if !handled {
                    if self.focused_contenteditable.is_some() {
                        // Route to contenteditable handler
                        let key_code = match key.as_str() {
                            "ArrowUp" => KeyCode::ArrowUp,
                            "ArrowDown" => KeyCode::ArrowDown,
                            "ArrowLeft" => KeyCode::ArrowLeft,
                            "ArrowRight" => KeyCode::ArrowRight,
                            "Home" => KeyCode::Home,
                            "End" => KeyCode::End,
                            "Enter" => KeyCode::Enter,
                            "Backspace" => KeyCode::Backspace,
                            "Delete" => KeyCode::Delete,
                            "Tab" => KeyCode::Tab,
                            "Escape" => KeyCode::Escape,
                            // Map single letter keys to their KeyCode variants
                            "a" | "A" => KeyCode::KeyA,
                            "b" | "B" => KeyCode::KeyB,
                            "c" | "C" => KeyCode::KeyC,
                            "d" | "D" => KeyCode::KeyD,
                            "e" | "E" => KeyCode::KeyE,
                            "f" | "F" => KeyCode::KeyF,
                            "g" | "G" => KeyCode::KeyG,
                            "h" | "H" => KeyCode::KeyH,
                            "i" | "I" => KeyCode::KeyI,
                            "j" | "J" => KeyCode::KeyJ,
                            "k" | "K" => KeyCode::KeyK,
                            "l" | "L" => KeyCode::KeyL,
                            "m" | "M" => KeyCode::KeyM,
                            "n" | "N" => KeyCode::KeyN,
                            "o" | "O" => KeyCode::KeyO,
                            "p" | "P" => KeyCode::KeyP,
                            "q" | "Q" => KeyCode::KeyQ,
                            "r" | "R" => KeyCode::KeyR,
                            "s" | "S" => KeyCode::KeyS,
                            "t" | "T" => KeyCode::KeyT,
                            "u" | "U" => KeyCode::KeyU,
                            "v" | "V" => KeyCode::KeyV,
                            "w" | "W" => KeyCode::KeyW,
                            "x" | "X" => KeyCode::KeyX,
                            "y" | "Y" => KeyCode::KeyY,
                            "z" | "Z" => KeyCode::KeyZ,
                            _ => KeyCode::Space, // Safe fallback for other keys
                        };
                        let text = match key.as_str() {
                            "Enter" => Some("\n".to_string()),
                            k if k.len() == 1 => Some(k.to_string()),
                            _ => None,
                        };
                        self.handle_contenteditable_key(
                            key_code,
                            text.as_deref(),
                            shift,
                            ctrl,
                            false,
                        );
                    } else {
                        match key.as_str() {
                            "ArrowUp" => self.handle_arrow_up(shift),
                            "ArrowDown" => self.handle_arrow_down(shift),
                            "ArrowLeft" => self.handle_arrow_left(shift, ctrl),
                            "ArrowRight" => self.handle_arrow_right(shift, ctrl),
                            "Home" => self.handle_home(shift),
                            "End" => self.handle_end(shift),
                            "Enter" => self.handle_enter(),
                            "Backspace" => self.handle_backspace(),
                            "Delete" => self.handle_delete(),
                            _ => {}
                        }
                    }
                }
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::GetCaretPosition {
                node_id,
                byte_offset,
            } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };

                let scale = scale_factor as f32;

                let d = doc.borrow();
                let Some(node) = d.tree.get(node_id) else {
                    return DebugResult::Error {
                        message: format!("Node {} not found", node_id),
                    };
                };

                let mut abs_x = node.layout.x as f64;
                let mut abs_y = node.layout.y as f64;
                let mut parent_id = node.parent;
                while let Some(pid) = parent_id {
                    if let Some(parent_node) = d.tree.get(pid) {
                        abs_x += parent_node.layout.x as f64;
                        abs_y += parent_node.layout.y as f64;
                        abs_x -= parent_node.scroll_offset.0;
                        abs_y -= parent_node.scroll_offset.1;
                        parent_id = parent_node.parent;
                    } else {
                        break;
                    }
                }

                let tag = node.tag();
                if matches!(tag, Some("input" | "textarea")) {
                    let value = node.attributes.get("value").cloned().unwrap_or_default();
                    if value.is_empty() {
                        let padding_left =
                            node.computed_style.padding_left.to_px() as f64 * scale as f64;
                        let padding_top =
                            node.computed_style.padding_top.to_px() as f64 * scale as f64;
                        return DebugResult::Json {
                            data: json!({
                                "x": abs_x + padding_left,
                                "y": abs_y + padding_top,
                            }),
                        };
                    }

                    let computed_style = node.computed_style.clone();
                    let input_width = node.layout.width;
                    drop(d);

                    let layout = computed_style.build_parley_layout(
                        &value,
                        scale,
                        &mut self.hit_test_font_cx,
                        &mut self.paint_layout_cx,
                        Some(input_width),
                    );

                    let (x, y) = caret_position_for_offset_layout(&layout, byte_offset);
                    let padding_left = computed_style.padding_left.to_px() as f64 * scale as f64;
                    let padding_top = computed_style.padding_top.to_px() as f64 * scale as f64;

                    return DebugResult::Json {
                        data: json!({
                            "x": abs_x + padding_left + x as f64,
                            "y": abs_y + padding_top + y as f64,
                        }),
                    };
                }

                if let Some(ref inline_layout) = node.text_layout {
                    let (x, y) =
                        caret_position_for_offset_layout(&inline_layout.layout, byte_offset);
                    return DebugResult::Json {
                        data: json!({
                            "x": abs_x + x as f64,
                            "y": abs_y + y as f64,
                        }),
                    };
                }

                DebugResult::Error {
                    message: "Node does not have text layout".into(),
                }
            }
            DebugCommandKind::GetGlyphBounds {
                node_id,
                byte_offset,
            } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };

                let scale = scale_factor as f32;

                let d = doc.borrow();
                let Some(node) = d.tree.get(node_id) else {
                    return DebugResult::Error {
                        message: format!("Node {} not found", node_id),
                    };
                };

                let mut abs_x = node.layout.x as f64;
                let mut abs_y = node.layout.y as f64;
                let mut parent_id = node.parent;
                while let Some(pid) = parent_id {
                    if let Some(parent_node) = d.tree.get(pid) {
                        abs_x += parent_node.layout.x as f64;
                        abs_y += parent_node.layout.y as f64;
                        abs_x -= parent_node.scroll_offset.0;
                        abs_y -= parent_node.scroll_offset.1;
                        parent_id = parent_node.parent;
                    } else {
                        break;
                    }
                }

                let tag = node.tag();
                if matches!(tag, Some("input" | "textarea")) {
                    let value = node.attributes.get("value").cloned().unwrap_or_default();
                    if value.is_empty() {
                        return DebugResult::Error {
                            message: "No text content".into(),
                        };
                    }

                    let computed_style = node.computed_style.clone();
                    let input_width = node.layout.width;
                    drop(d);

                    let layout = computed_style.build_parley_layout(
                        &value,
                        scale,
                        &mut self.hit_test_font_cx,
                        &mut self.paint_layout_cx,
                        Some(input_width),
                    );

                    match glyph_bounds_for_offset_layout(&layout, byte_offset) {
                        Some(bounds) => {
                            let padding_left =
                                computed_style.padding_left.to_px() as f64 * scale as f64;
                            let padding_top =
                                computed_style.padding_top.to_px() as f64 * scale as f64;
                            return DebugResult::Json {
                                data: json!({
                                    "x": abs_x + padding_left + bounds.x as f64,
                                    "y": abs_y + padding_top + bounds.y as f64,
                                    "width": bounds.width,
                                    "height": bounds.height,
                                }),
                            };
                        }
                        None => {
                            return DebugResult::Error {
                                message: "Byte offset out of bounds".into(),
                            };
                        }
                    }
                }

                if let Some(ref inline_layout) = node.text_layout {
                    match glyph_bounds_for_offset_layout(&inline_layout.layout, byte_offset) {
                        Some(bounds) => {
                            return DebugResult::Json {
                                data: json!({
                                    "x": abs_x + bounds.x as f64,
                                    "y": abs_y + bounds.y as f64,
                                    "width": bounds.width,
                                    "height": bounds.height,
                                }),
                            };
                        }
                        None => {
                            return DebugResult::Error {
                                message: "Byte offset out of bounds".into(),
                            };
                        }
                    }
                }

                DebugResult::Error {
                    message: "Node does not have text layout".into(),
                }
            }
        }
    }
}
