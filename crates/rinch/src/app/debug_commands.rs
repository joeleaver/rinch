use super::*;

fn parse_button(s: &Option<String>) -> MouseButton {
    match s.as_deref() {
        Some("right") => MouseButton::Right,
        Some("middle") => MouseButton::Middle,
        _ => MouseButton::Left,
    }
}

/// Map a printable character to a physical `KeyCode` for synthesizing keystrokes
/// (the `text` field carries the actual character; the keycode only matters for
/// shortcut/interceptor matching). Unknown characters fall back to `Space`.
fn char_to_keycode(c: char) -> KeyCode {
    match c.to_ascii_lowercase() {
        'a' => KeyCode::KeyA,
        'b' => KeyCode::KeyB,
        'c' => KeyCode::KeyC,
        'd' => KeyCode::KeyD,
        'e' => KeyCode::KeyE,
        'f' => KeyCode::KeyF,
        'g' => KeyCode::KeyG,
        'h' => KeyCode::KeyH,
        'i' => KeyCode::KeyI,
        'j' => KeyCode::KeyJ,
        'k' => KeyCode::KeyK,
        'l' => KeyCode::KeyL,
        'm' => KeyCode::KeyM,
        'n' => KeyCode::KeyN,
        'o' => KeyCode::KeyO,
        'p' => KeyCode::KeyP,
        'q' => KeyCode::KeyQ,
        'r' => KeyCode::KeyR,
        's' => KeyCode::KeyS,
        't' => KeyCode::KeyT,
        'u' => KeyCode::KeyU,
        'v' => KeyCode::KeyV,
        'w' => KeyCode::KeyW,
        'x' => KeyCode::KeyX,
        'y' => KeyCode::KeyY,
        'z' => KeyCode::KeyZ,
        '0' => KeyCode::Digit0,
        '1' => KeyCode::Digit1,
        '2' => KeyCode::Digit2,
        '3' => KeyCode::Digit3,
        '4' => KeyCode::Digit4,
        '5' => KeyCode::Digit5,
        '6' => KeyCode::Digit6,
        '7' => KeyCode::Digit7,
        '8' => KeyCode::Digit8,
        '9' => KeyCode::Digit9,
        _ => KeyCode::Space,
    }
}

/// Map a debug `key_press` key-name string to a `KeyCode`.
fn keyname_to_keycode(key: &str) -> KeyCode {
    match key {
        "ArrowLeft" => KeyCode::ArrowLeft,
        "ArrowRight" => KeyCode::ArrowRight,
        "ArrowUp" => KeyCode::ArrowUp,
        "ArrowDown" => KeyCode::ArrowDown,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        "Enter" => KeyCode::Enter,
        "Backspace" => KeyCode::Backspace,
        "Delete" => KeyCode::Delete,
        "Tab" => KeyCode::Tab,
        "Escape" => KeyCode::Escape,
        "F12" => KeyCode::F12,
        k if k.chars().count() == 1 => char_to_keycode(k.chars().next().unwrap()),
        _ => KeyCode::Space,
    }
}

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
            DebugCommandKind::DomTree { max_depth, root_id } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                DebugResult::Json {
                    data: rinch_dom::testing::serialize_tree_with_options(
                        &d.tree,
                        max_depth.or(Some(3)),
                        root_id,
                    ),
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
                    .filter_map(|&id| rinch_dom::testing::get_node_summary(&d.tree, id))
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
            DebugCommandKind::Click { x, y, ref button } => {
                // Route through the REAL input path (press + release) so MCP
                // exercises exactly what a real mouse does — `handle_event` owns all
                // the click logic (contextmenu, focus, editor, drag). Injecting via a
                // parallel path here is what hid the dual-arm MouseDown bug.
                let mouse_button = parse_button(button);
                self.cursor_pos = Some((x, y));
                actions.extend(self.handle_event(
                    PlatformEvent::MouseDown {
                        x,
                        y,
                        button: mouse_button,
                    },
                    window_size,
                    scale_factor,
                ));
                actions.extend(self.handle_event(
                    PlatformEvent::MouseUp {
                        x,
                        y,
                        button: mouse_button,
                    },
                    window_size,
                    scale_factor,
                ));
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::MouseDown { x, y, ref button } => {
                // Route through the real input path so MCP matches a real press.
                let mouse_button = parse_button(button);
                self.cursor_pos = Some((x, y));
                actions.extend(self.handle_event(
                    PlatformEvent::MouseDown {
                        x,
                        y,
                        button: mouse_button,
                    },
                    window_size,
                    scale_factor,
                ));
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::MouseUp { x, y, ref button } => {
                // Route through the real input path so MCP matches a real release.
                let mouse_button = parse_button(button);
                self.cursor_pos = Some((x, y));
                actions.extend(self.handle_event(
                    PlatformEvent::MouseUp {
                        x,
                        y,
                        button: mouse_button,
                    },
                    window_size,
                    scale_factor,
                ));
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::MouseMove { x, y } => {
                // Route through the real input path so MCP matches a real move
                // (drag-select, hover, surface dispatch all live in `handle_event`).
                self.cursor_pos = Some((x, y));
                actions.extend(self.handle_event(
                    PlatformEvent::MouseMove { x, y },
                    window_size,
                    scale_factor,
                ));
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::Scroll {
                x,
                y,
                delta_x: _delta_x,
                delta_y,
            } => {
                self.cursor_pos = Some((x, y));

                // Dispatch scroll to render surface if applicable
                let surface_consumed = if let Some(doc) = &self.doc {
                    let surface_hit = {
                        let d = doc.borrow();
                        if let Some(hit_id) = hit_test(&d.tree, x, y) {
                            Self::find_render_surface_at(&d.tree, hit_id, x, y)
                        } else {
                            None
                        }
                    };
                    if let Some((surface_id, local_x, local_y)) = surface_hit {
                        crate::render_surface::dispatch_surface_event(
                            surface_id,
                            crate::render_surface::SurfaceEvent::MouseWheel {
                                x: local_x,
                                y: local_y,
                                delta_x: _delta_x as f32,
                                delta_y: delta_y as f32,
                            },
                        );
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };

                if surface_consumed {
                    actions.push(AppAction::RequestRedraw);
                    return DebugResult::Json { data: json!(null) };
                }

                if let Some(doc) = &self.doc {
                    let hit_node = hit_test(&doc.borrow().tree, x, y);
                    if let Some(hit_node) = hit_node {
                        let mut doc_mut = doc.borrow_mut();
                        let mut scroll_handler_to_fire: Option<(usize, f64)> = None;
                        if let Some(scroll_node_id) = find_scroll_container(&doc_mut.tree, hit_node)
                            .or_else(|| find_scroll_container_at_point(&doc_mut.tree, x, y))
                        {
                            let nid = rinch_core::dom::NodeId(scroll_node_id);
                            let content_height = doc_mut.scroll_height(nid);
                            let visible_height = doc_mut.client_height(nid);
                            let max_scroll = (content_height - visible_height).max(0.0);

                            let handler_id = doc_mut
                                .tree
                                .nodes
                                .get(scroll_node_id)
                                .and_then(|n| n.attributes.get("data-onscroll"))
                                .and_then(|s| s.parse::<usize>().ok());
                            if let Some(node) = doc_mut.tree.nodes.get_mut(scroll_node_id) {
                                let new_y = (node.scroll_offset.1 + delta_y).clamp(0.0, max_scroll);
                                if new_y != node.scroll_offset.1 {
                                    node.scroll_offset.1 = new_y;
                                    node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                                    doc_mut.tree.dirty_nodes.insert(scroll_node_id);
                                    self.scene_dirty = true;
                                    if let Some(hid) = handler_id {
                                        scroll_handler_to_fire = Some((hid, new_y));
                                    }
                                }
                            }
                        }
                        drop(doc_mut);
                        if let Some((handler_id, scroll_top)) = scroll_handler_to_fire {
                            use rinch_core::events::{EventHandlerId, dispatch_scroll_event};
                            dispatch_scroll_event(EventHandlerId(handler_id), scroll_top);
                        }
                    }
                }
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::TypeText { text } => {
                // Synthesize a real KeyDown per character and route through
                // `handle_event`, so MCP `type_text` drives the exact same path as
                // physical keystrokes.
                for ch in text.chars() {
                    let (key, txt) = match ch {
                        '\n' => (KeyCode::Enter, None),
                        '\t' => (KeyCode::Tab, None),
                        '\x08' => (KeyCode::Backspace, None),
                        c => (char_to_keycode(c), Some(c.to_string())),
                    };
                    let logical_key = ch.is_ascii_alphabetic().then(|| ch.to_ascii_lowercase());
                    actions.extend(self.handle_event(
                        PlatformEvent::KeyDown {
                            key,
                            logical_key,
                            text: txt,
                            modifiers: rinch_platform::Modifiers::default(),
                        },
                        window_size,
                        scale_factor,
                    ));
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
            DebugCommandKind::KeyPress {
                key,
                shift,
                ctrl,
                alt,
            } => {
                // Synthesize a real KeyDown and route through `handle_event` (which
                // owns Escape/F12/inspect/editor/CE handling) so MCP `key_press`
                // matches a physical keystroke.
                let key_code = keyname_to_keycode(&key);
                let text = match key.as_str() {
                    "Enter" => Some("\n".to_string()),
                    k if k.chars().count() == 1 => Some(k.to_string()),
                    _ => None,
                };
                // A single-letter key name is its own logical letter (MCP has no layout).
                let logical_key = {
                    let mut it = key.chars();
                    match (it.next(), it.next()) {
                        (Some(c), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_lowercase()),
                        _ => None,
                    }
                };
                actions.extend(self.handle_event(
                    PlatformEvent::KeyDown {
                        key: key_code,
                        logical_key,
                        text,
                        modifiers: rinch_platform::Modifiers {
                            shift,
                            ctrl,
                            alt,
                            meta: false,
                        },
                    },
                    window_size,
                    scale_factor,
                ));
                actions.push(AppAction::RequestRedraw);
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::Ime {
                action,
                text,
                cursor,
            } => {
                // Synthesize a real `PlatformEvent::Ime` and route it through
                // `handle_event` — exactly the path winit's `WindowEvent::Ime` takes
                // — so a debug-injected composition drives the focus arbiter and the
                // focused target's preedit/commit identically to a physical IME.
                let ime_event = match action.as_str() {
                    "enable" => Some(rinch_platform::ImeEvent::Enabled),
                    "preedit" => Some(rinch_platform::ImeEvent::Preedit { text, cursor }),
                    "commit" => Some(rinch_platform::ImeEvent::Commit(text)),
                    "disable" => Some(rinch_platform::ImeEvent::Disabled),
                    other => {
                        return DebugResult::Error {
                            message: format!("Unknown ime action: {other}"),
                        };
                    }
                };
                if let Some(ev) = ime_event {
                    actions.extend(self.handle_event(
                        PlatformEvent::Ime(ev),
                        window_size,
                        scale_factor,
                    ));
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
