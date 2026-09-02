use super::*;

fn parse_button(s: &Option<String>) -> MouseButton {
    match s.as_deref() {
        Some("right") => MouseButton::Right,
        Some("middle") => MouseButton::Middle,
        _ => MouseButton::Left,
    }
}

/// Map a printable character to a physical `KeyCode` for synthesizing keystrokes
/// (the `text` field carries the actual character). Characters without a
/// dedicated keycode map to `KeyCode::Other` — exactly how real hardware
/// delivers punctuation (`shell/rinch_runtime.rs` translates unlisted winit
/// keys to `Other`) — so the keyboard hook's key string falls through to the
/// `text` field instead of masquerading as a named key. Space keeps its
/// explicit arm: an injected `' '` must still read as `key = "Space"`, matching
/// a physical spacebar press (issue #151).
fn char_to_keycode(c: char) -> KeyCode {
    match c.to_ascii_lowercase() {
        ' ' => KeyCode::Space,
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
        _ => KeyCode::Other,
    }
}

/// Map a debug `key_press` key-name string to a `KeyCode`. Single-character
/// names go through [`char_to_keycode`] (punctuation → `KeyCode::Other`, with
/// the character delivered via the event's `text` field); unknown multi-char
/// names return `None` so the caller can fail loud instead of synthesizing a
/// silent no-text `Other` press.
fn keyname_to_keycode(key: &str) -> Option<KeyCode> {
    Some(match key {
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
        "Space" => KeyCode::Space,
        "F12" => KeyCode::F12,
        k if k.chars().count() == 1 => char_to_keycode(k.chars().next().unwrap()),
        _ => return None,
    })
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
            DebugCommandKind::DomTree {
                max_depth,
                root_id,
                verbose,
            } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error {
                        message: "No document".into(),
                    };
                };
                let d = doc.borrow();
                DebugResult::Json {
                    data: rinch_dom::testing::serialize_tree_full(
                        &d.tree,
                        max_depth.or(Some(3)),
                        root_id,
                        verbose,
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
                        let mut scroll_handler_to_fire: Option<(usize, usize)> = None;
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
                                        scroll_handler_to_fire = Some((hid, scroll_node_id));
                                    }
                                }
                            }
                        }
                        let event = scroll_handler_to_fire.map(|(hid, node_id)| {
                            (hid, Self::scroll_event_for(&doc_mut.tree, node_id))
                        });
                        drop(doc_mut);
                        if let Some((handler_id, event)) = event {
                            use rinch_core::events::{EventHandlerId, dispatch_scroll_event};
                            dispatch_scroll_event(EventHandlerId(handler_id), event);
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
                    // The typed character is its own key value (MCP has no
                    // layout to consult) — case-accurate, like the field asks.
                    // The '\n'/'\t'/'\x08' branches fall out as `None` via the
                    // control-character test, and their named `KeyCode`s spell
                    // them instead.
                    let logical_key = (!ch.is_control()).then(|| ch.to_string());
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
                // Layout is resolved at the logical viewport, not the physical
                // surface size (see `RinchApp::layout_viewport`).
                let (w, h) = Self::layout_viewport(window_size, scale_factor);
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
                mut shift,
                mut ctrl,
                mut alt,
                modifiers,
            } => {
                // Synthesize a real KeyDown and route through `handle_event` (which
                // owns Escape/F12/inspect/editor/CE handling) so MCP `key_press`
                // matches a physical keystroke.
                //
                // Fold the optional `modifiers` name array into the flat booleans
                // (issue #152) — the only path that can request `meta`. An unknown
                // name fails loud instead of silently altering the simulated input.
                let mut meta = false;
                if let Err(name) = rinch_debug::fold_modifier_names(
                    &modifiers, &mut shift, &mut ctrl, &mut alt, &mut meta,
                ) {
                    return DebugResult::Error {
                        message: format!(
                            "Unknown modifier name: {name:?} (expected ctrl/control, shift, alt/option, meta/cmd/super)"
                        ),
                    };
                }
                // Unknown multi-char key names fail loud too — a silent no-text
                // `Other` press would be indistinguishable from a dead key (#151).
                let Some(key_code) = keyname_to_keycode(&key) else {
                    return DebugResult::Error {
                        message: format!("Unknown key name: {key:?}"),
                    };
                };
                let text = match key.as_str() {
                    "Enter" => Some("\n".to_string()),
                    k if k.chars().count() == 1 => Some(k.to_string()),
                    _ => None,
                };
                // A single-character key name is its own key value, case and
                // all (MCP has no layout to consult). Named keys stay `None`:
                // their `KeyCode` spells them, and fabricating the name here
                // would just shadow that table.
                let logical_key = {
                    let mut it = key.chars();
                    match (it.next(), it.next()) {
                        (Some(c), None) if !c.is_control() => Some(c.to_string()),
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
                            meta,
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

                // The node's painted frame. Every coordinate reported below is a
                // position *inside* the box, so it is pushed forward through the
                // composed transform rather than added to an origin — under a
                // `scale()` container the two differ (#203). Copied out before
                // the document borrow is released for the Parley rebuild.
                let (box_x, box_y, node_transform) =
                    rinch_dom::paint::compute_absolute_position_and_transform(
                        &d.tree, node_id, 1.0,
                    );
                let fwd = move |lx: f64, ly: f64| -> (f64, f64) {
                    let p = node_transform * peniko::kurbo::Point::new(box_x + lx, box_y + ly);
                    (p.x, p.y)
                };

                let tag = node.tag();
                if matches!(tag, Some("input" | "textarea")) {
                    let value = node.attributes.get("value").cloned().unwrap_or_default();
                    if value.is_empty() {
                        let padding_left =
                            node.computed_style.padding_left.to_px() as f64 * scale as f64;
                        let padding_top =
                            node.computed_style.padding_top.to_px() as f64 * scale as f64;
                        let (cx, cy) = fwd(padding_left, padding_top);
                        return DebugResult::Json {
                            data: json!({ "x": cx, "y": cy }),
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

                    let (cx, cy) = fwd(padding_left + x as f64, padding_top + y as f64);
                    return DebugResult::Json {
                        data: json!({ "x": cx, "y": cy }),
                    };
                }

                if let Some(ref inline_layout) = node.text_layout {
                    let (x, y) =
                        caret_position_for_offset_layout(&inline_layout.layout, byte_offset);
                    let (cx, cy) = fwd(x as f64, y as f64);
                    return DebugResult::Json {
                        data: json!({ "x": cx, "y": cy }),
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

                // The node's painted frame. Every coordinate reported below is a
                // position *inside* the box, so it is pushed forward through the
                // composed transform rather than added to an origin — under a
                // `scale()` container the two differ (#203). Copied out before
                // the document borrow is released for the Parley rebuild.
                let (box_x, box_y, node_transform) =
                    rinch_dom::paint::compute_absolute_position_and_transform(
                        &d.tree, node_id, 1.0,
                    );
                let fwd = move |lx: f64, ly: f64| -> (f64, f64) {
                    let p = node_transform * peniko::kurbo::Point::new(box_x + lx, box_y + ly);
                    (p.x, p.y)
                };

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
                            let (gx, gy) = fwd(
                                padding_left + bounds.x as f64,
                                padding_top + bounds.y as f64,
                            );
                            return DebugResult::Json {
                                data: json!({
                                    "x": gx,
                                    "y": gy,
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
                            let (gx, gy) = fwd(bounds.x as f64, bounds.y as f64);
                            return DebugResult::Json {
                                data: json!({
                                    "x": gx,
                                    "y": gy,
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

#[cfg(test)]
mod keycode_mapping_tests {
    use super::{char_to_keycode, keyname_to_keycode};
    use rinch_platform::KeyCode;

    #[test]
    fn punctuation_maps_to_other_not_space() {
        // Punctuation must not masquerade as the spacebar (issue #151): with
        // `Other`, the hook's key string falls through to the `text` field.
        assert_eq!(char_to_keycode('.'), KeyCode::Other);
    }

    #[test]
    fn space_char_keeps_its_named_keycode() {
        // Space-parity gotcha: an injected ' ' must still read as
        // `key = "Space"`, exactly like a physical spacebar press.
        assert_eq!(char_to_keycode(' '), KeyCode::Space);
    }

    #[test]
    fn keyname_space_maps_to_space() {
        assert_eq!(keyname_to_keycode("Space"), Some(KeyCode::Space));
    }

    #[test]
    fn keyname_single_char_punctuation_maps_to_other() {
        assert_eq!(keyname_to_keycode("."), Some(KeyCode::Other));
    }

    #[test]
    fn unknown_multi_char_keyname_is_rejected() {
        // The KeyPress handler turns this into a DebugResult::Error rather
        // than a silent no-text `Other` press.
        assert_eq!(keyname_to_keycode("NoSuchKey"), None);
    }
}
