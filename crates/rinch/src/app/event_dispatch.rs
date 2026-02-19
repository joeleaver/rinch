//! Event dispatch: routes platform events to the appropriate handler.

use super::*;

impl RinchApp {
    /// Process a platform event and return a list of actions for the shell.
    #[allow(clippy::too_many_lines)]
    pub fn handle_event(
        &mut self,
        event: PlatformEvent,
        window_size: (u32, u32),
        scale_factor: f64,
    ) -> Vec<AppAction> {
        let mut actions = Vec::new();

        match event {
            PlatformEvent::Resumed => {
                // Handled by the shell (window creation)
            }
            PlatformEvent::CloseRequested => {
                let should_exit = self
                    .window_props
                    .as_ref()
                    .and_then(|p| p.on_close_requested.as_ref())
                    .is_none_or(|cb| cb());
                if should_exit {
                    actions.push(AppAction::Exit);
                }
            }
            PlatformEvent::Resized { width, height } => {
                self.resize_layout(width, height);
                actions.push(AppAction::RequestRedraw);
            }
            PlatformEvent::RedrawRequested => {
                // Paint is handled by the shell after building the scene
            }
            PlatformEvent::MouseMove { x, y } => {
                self.cursor_pos = Some((x, y));

                // Handle component drag (sliders, floating panels, etc.)
                if rinch_core::update_drag(x, y) {
                    let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                    self.resolve_and_repaint(w, h);
                    actions.push(AppAction::RequestRedraw);
                    return actions;
                }

                // Handle scrollbar drag
                if let Some(drag) = &self.scrollbar_drag {
                    let node_id = drag.node_id;
                    let dy = y - drag.start_y;
                    let track_height = drag.container_height - 4.0;
                    let max_scroll = drag.content_height - drag.container_height;
                    let scroll_delta = (dy as f64 / track_height) * drag.content_height;
                    let new_scroll = (drag.start_scroll + scroll_delta).clamp(0.0, max_scroll);

                    if let Some(doc) = &self.doc {
                        let mut d = doc.borrow_mut();
                        if let Some(node) = d.tree.nodes.get_mut(node_id) {
                            node.scroll_offset.1 = new_scroll;
                            node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                            d.tree.dirty_nodes.insert(node_id);
                        }
                    }
                    actions.push(AppAction::RequestRedraw);
                    return actions;
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
                        self.sync_ce_ops_cursor();
                        self.scene_dirty = true;
                        actions.push(AppAction::RequestRedraw);
                        return actions;
                    }
                }

                // Update hover state for CSS :hover support
                if let Some(doc) = &self.doc {
                    let (hovered, cursor_style) = {
                        let d = doc.borrow();
                        let h = hit_test(&d.tree, x, y);
                        let cs = h
                            .and_then(|id| d.tree.get(id))
                            .map(|n| cursor_value_to_style(&n.computed_style.cursor))
                            .unwrap_or(rinch_platform::CursorStyle::Default);
                        (h, cs)
                    };
                    let changed = doc.borrow_mut().update_hover(hovered);
                    if changed {
                        actions.push(AppAction::RequestRedraw);
                    }
                    actions.push(AppAction::SetCursor(cursor_style));
                }
            }
            PlatformEvent::MouseDown {
                x,
                y,
                button: MouseButton::Left,
            } => {
                // Multi-click detection
                let now = Instant::now();
                let elapsed = now.duration_since(self.last_click_time);
                let (last_x, last_y) = self.last_click_pos;
                let distance = ((x - last_x).powi(2) + (y - last_y).powi(2)).sqrt();

                const DOUBLE_CLICK_TIMEOUT: rinch_platform::Duration =
                    rinch_platform::Duration::from_millis(500);
                const DOUBLE_CLICK_DISTANCE: f32 = 5.0;

                if elapsed < DOUBLE_CLICK_TIMEOUT && distance < DOUBLE_CLICK_DISTANCE {
                    self.click_count = (self.click_count % 3) + 1;
                } else {
                    self.click_count = 1;
                }

                self.last_click_time = now;
                self.last_click_pos = (x, y);

                // Update :active and :focus pseudo-class state
                if let Some(doc) = &self.doc {
                    let hit = {
                        let d = doc.borrow();
                        hit_test(&d.tree, x, y)
                    };
                    // :active applies while mouse is pressed
                    let active_changed = doc.borrow_mut().update_active(hit);
                    // :focus applies to the clicked element (persists after release)
                    let focus_changed = doc.borrow_mut().update_focus(hit);
                    if active_changed || focus_changed {
                        actions.push(AppAction::RequestRedraw);
                    }
                }

                // Check scrollbar hit first
                let scrollbar_hit = if let Some(doc) = &self.doc {
                    let d = doc.borrow();
                    find_scrollbar_hit(&d.tree, x, y)
                } else {
                    None
                };

                if let Some((node_id, content_height, container_height)) = scrollbar_hit {
                    if let Some(doc) = &self.doc {
                        let mut d = doc.borrow_mut();
                        let node_abs_y = compute_absolute_y(&d.tree, node_id);
                        let margin = 2.0_f64;
                        let track_top = node_abs_y as f64 + margin;
                        let track_height = container_height - margin * 2.0;
                        let max_scroll = content_height - container_height;
                        let click_ratio = ((y as f64 - track_top) / track_height).clamp(0.0, 1.0);
                        let new_scroll = click_ratio * max_scroll;

                        if let Some(node) = d.tree.nodes.get_mut(node_id) {
                            node.scroll_offset.1 = new_scroll;
                            node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                            d.tree.dirty_nodes.insert(node_id);
                        }

                        self.scrollbar_drag = Some(ScrollbarDrag {
                            node_id,
                            start_y: y,
                            start_scroll: new_scroll,
                            content_height,
                            container_height,
                        });
                    }
                    actions.push(AppAction::RequestRedraw);
                } else {
                    let drag_action = self.handle_click(x, y, scale_factor);
                    actions.extend(drag_action);
                }
            }
            PlatformEvent::MouseDown { .. } => {
                // Non-left button clicks: no-op for now
            }
            PlatformEvent::MouseUp { .. } => {
                rinch_core::stop_drag();
                self.scrollbar_drag = None;
                self.ce_selecting = false;

                // Clear :active pseudo-class state on mouse release
                if let Some(doc) = &self.doc {
                    let changed = doc.borrow_mut().update_active(None);
                    if changed {
                        actions.push(AppAction::RequestRedraw);
                    }
                }
            }
            PlatformEvent::MouseWheel {
                x,
                y,
                delta_x,
                delta_y,
            } => {
                self.cursor_pos = Some((x, y));
                if let Some(doc) = &self.doc {
                    let hit_node = hit_test(&doc.borrow().tree, x, y);
                    if let Some(hit_node) = hit_node {
                        let mut doc_mut = doc.borrow_mut();

                        // Vertical scrolling
                        if delta_y.abs() > 0.0
                            && let Some(scroll_node_id) =
                                find_scroll_container(&doc_mut.tree, hit_node)
                        {
                            let content_height =
                                compute_content_height(&doc_mut.tree, scroll_node_id);
                            let visible_height =
                                compute_visible_content_area_height(&doc_mut.tree, scroll_node_id);
                            let max_scroll = (content_height - visible_height).max(0.0);

                            if let Some(node) = doc_mut.tree.nodes.get_mut(scroll_node_id) {
                                let new_y = (node.scroll_offset.1 - delta_y).clamp(0.0, max_scroll);
                                if new_y != node.scroll_offset.1 {
                                    node.scroll_offset.1 = new_y;
                                    node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                                    doc_mut.tree.dirty_nodes.insert(scroll_node_id);
                                }
                            }
                        }

                        // Horizontal scrolling
                        if delta_x.abs() > 0.0
                            && let Some(scroll_node_id) =
                                find_horizontal_scroll_container(&doc_mut.tree, hit_node)
                        {
                            let content_width =
                                compute_content_width(&doc_mut.tree, scroll_node_id);
                            let visible_width =
                                compute_visible_content_area_width(&doc_mut.tree, scroll_node_id);
                            let max_scroll = (content_width - visible_width).max(0.0);

                            if let Some(node) = doc_mut.tree.nodes.get_mut(scroll_node_id) {
                                let new_x = (node.scroll_offset.0 - delta_x).clamp(0.0, max_scroll);
                                if new_x != node.scroll_offset.0 {
                                    node.scroll_offset.0 = new_x;
                                    node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                                    doc_mut.tree.dirty_nodes.insert(scroll_node_id);
                                }
                            }
                        }

                        drop(doc_mut);
                        actions.push(AppAction::RequestRedraw);
                    }
                }
            }
            PlatformEvent::ModifiersChanged(mods) => {
                self.modifiers = mods;
            }
            PlatformEvent::KeyDown {
                key,
                text,
                modifiers,
            } => {
                let shift = modifiers.shift;
                let ctrl = modifiers.primary();
                let alt = modifiers.alt;

                // Build key string for keyboard interceptor - handle ALL key types
                let key_str: Option<String> = match key {
                    // Named keys
                    KeyCode::ArrowLeft => Some("ArrowLeft".into()),
                    KeyCode::ArrowRight => Some("ArrowRight".into()),
                    KeyCode::ArrowUp => Some("ArrowUp".into()),
                    KeyCode::ArrowDown => Some("ArrowDown".into()),
                    KeyCode::Home => Some("Home".into()),
                    KeyCode::End => Some("End".into()),
                    KeyCode::Enter => Some("Enter".into()),
                    KeyCode::Backspace => Some("Backspace".into()),
                    KeyCode::Delete => Some("Delete".into()),
                    KeyCode::Tab => Some("Tab".into()),
                    KeyCode::Escape => Some("Escape".into()),
                    KeyCode::PageUp => Some("PageUp".into()),
                    KeyCode::PageDown => Some("PageDown".into()),
                    KeyCode::Space => Some("Space".into()),
                    // Ctrl+key combos: derive key letter from KeyCode
                    KeyCode::KeyA if ctrl => Some("a".into()),
                    KeyCode::KeyB if ctrl => Some("b".into()),
                    KeyCode::KeyC if ctrl => Some("c".into()),
                    KeyCode::KeyD if ctrl => Some("d".into()),
                    KeyCode::KeyE if ctrl => Some("e".into()),
                    KeyCode::KeyH if ctrl => Some("h".into()),
                    KeyCode::KeyI if ctrl => Some("i".into()),
                    KeyCode::KeyU if ctrl => Some("u".into()),
                    KeyCode::KeyV if ctrl => Some("v".into()),
                    KeyCode::KeyX if ctrl => Some("x".into()),
                    KeyCode::KeyY if ctrl => Some("y".into()),
                    KeyCode::KeyZ if ctrl => Some("z".into()),
                    // Regular character input: use text field (filter control chars)
                    _ => text.as_ref().and_then(|t| {
                        if !t.is_empty() && t.chars().all(|c| !c.is_control()) {
                            Some(t.clone())
                        } else {
                            None
                        }
                    }),
                };

                tracing::trace!(?key, ?text, ?key_str, shift, ctrl, alt, "KeyDown event");

                // Try keyboard interceptor first for ALL keys
                let handled_by_interceptor = if let Some(ref ks) = key_str {
                    let key_data = events::KeyEventData {
                        key: ks.clone(),
                        code: format!("{:?}", key),
                        ctrl,
                        shift,
                        alt,
                        meta: false,
                    };
                    events::dispatch_keyboard_event(&key_data)
                } else {
                    false
                };

                if handled_by_interceptor {
                    actions.push(AppAction::RequestRedraw);
                } else if self.focused_contenteditable.is_some() {
                    // Route keyboard events to the contenteditable editing state
                    if self.handle_contenteditable_key(key, text.as_deref(), shift, ctrl, alt) {
                        // Resolve layout immediately so the IFC text layout is
                        // rebuilt before the next paint.  Without this, the
                        // invalidated text_layout (set to None by set_text_content)
                        // causes a one-frame flicker where text is invisible.
                        let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                        self.resolve_and_repaint(w, h);
                        actions.push(AppAction::RequestRedraw);
                    }
                } else {
                    #[cfg(feature = "desktop")]
                    if key == KeyCode::F12 {
                        self.devtools.toggle();
                        tracing::info!(
                            "DevTools: {}",
                            if self.devtools.visible {
                                "opened"
                            } else {
                                "closed"
                            }
                        );
                        actions.push(AppAction::RequestRedraw);
                        return actions;
                    }

                    match key {
                        KeyCode::Backspace => self.handle_backspace(),
                        KeyCode::Delete => self.handle_delete(),
                        KeyCode::ArrowLeft => self.handle_arrow_left(shift, ctrl),
                        KeyCode::ArrowRight => self.handle_arrow_right(shift, ctrl),
                        KeyCode::Home => self.handle_home(shift),
                        KeyCode::End => self.handle_end(shift),
                        KeyCode::KeyA if ctrl => self.handle_select_all(),
                        KeyCode::KeyC if ctrl => self.handle_copy(),
                        KeyCode::KeyV if ctrl => self.handle_paste(),
                        KeyCode::KeyX if ctrl => self.handle_cut(),
                        KeyCode::Enter if !ctrl => self.handle_enter(),
                        KeyCode::ArrowUp => self.handle_arrow_up(shift),
                        KeyCode::ArrowDown => self.handle_arrow_down(shift),
                        _ => {
                            if !ctrl
                                && let Some(t) = &text
                                && !t.is_empty()
                            {
                                self.handle_text_input(t);
                            }
                        }
                    }
                }
            }
            PlatformEvent::ScaleFactorChanged(_) => {
                // The shell handles reconfiguring the renderer; we just need a redraw.
                actions.push(AppAction::RequestRedraw);
            }
            PlatformEvent::UserEvent(UserEvent::ReRender) => {
                let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                if self.resolve_and_repaint(w, h) {
                    actions.push(AppAction::RequestRedraw);
                }
            }
            PlatformEvent::UserEvent(UserEvent::MinimizeWindow) => {
                actions.push(AppAction::SetMinimized(true));
            }
            PlatformEvent::UserEvent(UserEvent::ToggleMaximizeWindow) => {
                // The shell must query `is_maximized` and toggle.
                // We cannot know from here, so emit a special action.
                // The shell interprets this as "toggle".
                actions.push(AppAction::SetMaximized(true)); // placeholder; shell toggles
            }
            PlatformEvent::UserEvent(UserEvent::CloseWindow) => {
                actions.push(AppAction::Exit);
            }
            PlatformEvent::UserEvent(UserEvent::ShowWindow) => {
                actions.push(AppAction::SetVisible(true));
            }
            PlatformEvent::UserEvent(UserEvent::HideWindow) => {
                actions.push(AppAction::SetVisible(false));
            }
            PlatformEvent::UserEvent(UserEvent::DebugCommand) => {
                #[cfg(feature = "debug")]
                self.handle_debug_commands(&mut actions, scale_factor, window_size);
            }
            PlatformEvent::AboutToWait => {
                // Tick active CSS transitions — this updates interpolated values
                // in computed_style and marks affected nodes dirty.
                let any_transitions = if let Some(doc) = &self.doc {
                    doc.borrow_mut().tick_transitions()
                } else {
                    false
                };

                // Transitions modify computed_style directly — mark scene dirty
                // so build_scene() rebuilds the Vello scene with interpolated values.
                if any_transitions {
                    self.scene_dirty = true;
                }

                if self.has_dirty_nodes() {
                    let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                    if self.resolve_and_repaint(w, h) {
                        actions.push(AppAction::RequestRedraw);
                    }
                }

                // Keep the render loop active while transitions are running
                if any_transitions {
                    actions.push(AppAction::RequestRedraw);
                }
            }
        }

        actions
    }
}
