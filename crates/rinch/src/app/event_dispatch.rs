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

                // ── Drag-and-drop: pending → active transition ────────────
                if let Some(ref pending) = self.pending_drag {
                    let dx = x - pending.mousedown_pos.0;
                    let dy = y - pending.mousedown_pos.1;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist >= DRAG_THRESHOLD {
                        let node_id = pending.node_id;
                        let mousedown_pos = pending.mousedown_pos;
                        self.pending_drag = None;

                        // Capture snapshot and compute anchor
                        self.activate_drag(node_id, mousedown_pos, (x, y), scale_factor);

                        // Fire ondragstart handler
                        if let Some(doc) = &self.doc {
                            Self::dispatch_drag_attr(doc, node_id, "data-ondragstart");
                        }

                        actions.push(AppAction::SetCursor(rinch_platform::CursorStyle::Grabbing));
                        actions.push(AppAction::RequestRedraw);
                        return actions;
                    }
                    // Under threshold — stay pending, suppress all other mouse handling
                    return actions;
                }

                // ── Drag-and-drop: active drag tracking ───────────────────
                if let Some(ref mut drag) = self.active_dnd {
                    drag.cursor = (x, y);

                    // Hit test for drop targets
                    let new_target = if let Some(doc) = &self.doc {
                        let d = doc.borrow();
                        hit_test(&d.tree, x, y)
                            .and_then(|hit_id| Self::find_drop_target(&d.tree, hit_id))
                    } else {
                        None
                    };

                    let old_target = drag.over_target;
                    if new_target != old_target {
                        drag.over_target = new_target;
                        // Fire leave/enter events
                        if let Some(doc) = &self.doc {
                            if let Some(old_id) = old_target {
                                Self::dispatch_drag_attr(doc, old_id, "data-ondragleave");
                            }
                            if let Some(new_id) = new_target {
                                Self::dispatch_drag_attr(doc, new_id, "data-ondragenter");
                            }
                        }
                    }

                    // Fire ondragover on current drop target so it can track cursor position.
                    if let Some(target_id) = drag.over_target {
                        if let Some(doc) = &self.doc {
                            let (cx, cy) = drag.cursor;
                            Self::dispatch_drag_attr_with_context(
                                doc,
                                target_id,
                                "data-ondragover",
                                cx,
                                cy,
                            );
                        }
                    }

                    // Fire ondragmove on source so editor can track cursor position.
                    if let Some(doc) = &self.doc {
                        let (cx, cy) = drag.cursor;
                        events::set_click_context(events::ClickContext {
                            mouse_x: cx,
                            mouse_y: cy,
                            element_x: 0.0,
                            element_y: 0.0,
                            element_width: 0.0,
                            element_height: 0.0,
                            text_hit: Default::default(),
                            viewport_width: 0.0,
                            viewport_height: 0.0,
                        });
                        Self::dispatch_drag_attr(doc, drag.node_id, "data-ondragmove");
                    }

                    self.scene_dirty = true;
                    actions.push(AppAction::SetCursor(rinch_platform::CursorStyle::Grabbing));
                    actions.push(AppAction::RequestRedraw);
                    return actions;
                }

                // Check resize edge for borderless windows
                if let Some(ref props) = self.window_props {
                    if props.borderless && props.resizable {
                        if let Some(inset) = props.resize_inset {
                            let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                            let inset_physical = inset * scale_factor as f32;
                            if let Some(dir) = detect_resize_edge(x, y, w, h, inset_physical) {
                                actions
                                    .push(AppAction::SetCursor(resize_direction_to_cursor(&dir)));
                                return actions;
                            }
                        }
                    }
                }

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

                    let mut scroll_handler_to_fire: Option<(usize, f64)> = None;
                    if let Some(doc) = &self.doc {
                        let mut d = doc.borrow_mut();
                        // Extract handler ID before mutable node access to avoid borrow conflicts
                        let handler_id = d
                            .tree
                            .nodes
                            .get(node_id)
                            .and_then(|n| n.attributes.get("data-onscroll"))
                            .and_then(|s| s.parse::<usize>().ok());
                        if let Some(node) = d.tree.nodes.get_mut(node_id) {
                            node.scroll_offset.1 = new_scroll;
                            node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                            d.tree.push_dirty(node_id);
                        }
                        d.tree.dirty_nodes.insert(node_id);
                        if let Some(hid) = handler_id {
                            scroll_handler_to_fire = Some((hid, new_scroll));
                        }
                    }
                    if let Some((handler_id, scroll_top)) = scroll_handler_to_fire {
                        use rinch_core::events::{EventHandlerId, dispatch_scroll_event};
                        dispatch_scroll_event(EventHandlerId(handler_id), scroll_top);
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

                // Handle read-only text selection drag
                if self.text_selecting {
                    if let Some(sel) = &self.text_selection {
                        let ifc_node_id = sel.ifc_node_id;
                        let anchor = sel.anchor_offset;
                        let new_offset = if let Some(doc) = &self.doc {
                            let d = doc.borrow();
                            Self::compute_ifc_offset_from_click(&d.tree, ifc_node_id, x, y)
                        } else {
                            anchor
                        };
                        if let Some(sel) = &mut self.text_selection {
                            sel.focus_offset = new_offset;
                        }
                        self.set_text_selection_attributes(ifc_node_id, anchor, new_offset);
                        self.scene_dirty = true;
                        actions.push(AppAction::SetCursor(rinch_platform::CursorStyle::Text));
                        actions.push(AppAction::RequestRedraw);
                        return actions;
                    }
                }

                // Update hover state and cursor
                if let Some(doc) = &self.doc {
                    let (hovered, cursor_style, old_hovered) = {
                        let d = doc.borrow();
                        let h = hit_test(&d.tree, x, y);
                        let mut cs = h
                            .and_then(|id| d.tree.get(id))
                            .map(|n| cursor_value_to_style(&n.computed_style.cursor))
                            .unwrap_or(rinch_platform::CursorStyle::Default);
                        // Override cursor to I-beam when hovering over selectable text
                        if let Some(hit_id) = h {
                            if Self::find_selectable_ifc(&d.tree, hit_id).is_some() {
                                cs = rinch_platform::CursorStyle::Text;
                            }
                        }
                        (h, cs, d.tree.hovered_node)
                    };
                    let mut hovered_changed = false;
                    doc.borrow_mut().update_hover(hovered, &mut hovered_changed);
                    if hovered_changed {
                        if let Some(old_id) = old_hovered {
                            Self::dispatch_onleave(doc, old_id);
                        }
                        if let Some(hit_id) = hovered {
                            Self::dispatch_onenter(doc, hit_id);
                        }
                    }
                    // Don't request redraw — AboutToWait batches dirty state.
                    actions.push(AppAction::SetCursor(cursor_style));

                    // Dispatch MouseMove + MouseEnter/MouseLeave to render surfaces
                    let new_surface = if let Some(hit_id) = hovered {
                        let d = doc.borrow();
                        Self::find_render_surface_at(&d.tree, hit_id, x, y)
                    } else {
                        None
                    };

                    let new_surface_id = new_surface.as_ref().map(|(id, _, _)| *id);
                    if new_surface_id != self.hovered_surface {
                        // Dispatch MouseLeave to old surface
                        if let Some(old_id) = self.hovered_surface {
                            crate::render_surface::dispatch_surface_event(
                                old_id,
                                crate::render_surface::SurfaceEvent::MouseLeave,
                            );
                        }
                        // Dispatch MouseEnter to new surface
                        if let Some((sid, lx, ly)) = &new_surface {
                            crate::render_surface::dispatch_surface_event(
                                *sid,
                                crate::render_surface::SurfaceEvent::MouseEnter { x: *lx, y: *ly },
                            );
                        }
                        self.hovered_surface = new_surface_id;
                    }

                    if let Some((surface_id, local_x, local_y)) = new_surface {
                        crate::render_surface::dispatch_surface_event(
                            surface_id,
                            crate::render_surface::SurfaceEvent::MouseMove {
                                x: local_x,
                                y: local_y,
                            },
                        );
                    }
                }
            }
            PlatformEvent::MouseDown {
                x,
                y,
                button: MouseButton::Left,
            } => {
                // Check resize edge for borderless windows
                if let Some(ref props) = self.window_props {
                    if props.borderless && props.resizable {
                        if let Some(inset) = props.resize_inset {
                            let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                            let inset_physical = inset * scale_factor as f32;
                            if let Some(dir) = detect_resize_edge(x, y, w, h, inset_physical) {
                                actions.push(AppAction::DragResizeWindow(dir));
                                return actions;
                            }
                        }
                    }
                }

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

                // Update :active and :focus pseudo-class state.
                // Don't request redraw here — AboutToWait will pick up the
                // dirty styles and batch them into a single repaint.
                if let Some(doc) = &self.doc {
                    let hit = {
                        let d = doc.borrow();
                        hit_test(&d.tree, x, y)
                    };
                    // :active applies while mouse is pressed
                    doc.borrow_mut().update_active(hit);
                    // :focus applies to the clicked element (persists after release)
                    doc.borrow_mut().update_focus(hit);
                }

                // Check for draggable element — enter pending drag instead of
                // immediate click handling
                let draggable_node = if let Some(doc) = &self.doc {
                    let d = doc.borrow();
                    if let Some(hit_id) = hit_test(&d.tree, x, y) {
                        Self::find_draggable(&d.tree, hit_id)
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some(drag_node_id) = draggable_node {
                    self.pending_drag = Some(PendingDrag {
                        node_id: drag_node_id,
                        mousedown_pos: (x, y),
                    });
                    // Don't fire click yet — wait for threshold or mouseup
                    return actions;
                }

                // Check scrollbar hit first
                let scrollbar_hit = if let Some(doc) = &self.doc {
                    let d = doc.borrow();
                    find_scrollbar_hit(&d.tree, x, y)
                } else {
                    None
                };

                if let Some((node_id, content_height, container_height)) = scrollbar_hit {
                    let mut scroll_handler_to_fire: Option<(usize, f64)> = None;
                    if let Some(doc) = &self.doc {
                        let mut d = doc.borrow_mut();
                        let node_abs_y = compute_absolute_y(&d.tree, node_id);
                        let margin = 2.0_f64;
                        let track_top = node_abs_y as f64 + margin;
                        let track_height = container_height - margin * 2.0;
                        let max_scroll = content_height - container_height;
                        let click_ratio = ((y as f64 - track_top) / track_height).clamp(0.0, 1.0);
                        let new_scroll = click_ratio * max_scroll;

                        let handler_id = d
                            .tree
                            .nodes
                            .get(node_id)
                            .and_then(|n| n.attributes.get("data-onscroll"))
                            .and_then(|s| s.parse::<usize>().ok());
                        if let Some(node) = d.tree.nodes.get_mut(node_id) {
                            node.scroll_offset.1 = new_scroll;
                            node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                            d.tree.push_dirty(node_id);
                        }
                        d.tree.dirty_nodes.insert(node_id);
                        if let Some(hid) = handler_id {
                            scroll_handler_to_fire = Some((hid, new_scroll));
                        }

                        self.scrollbar_drag = Some(ScrollbarDrag {
                            node_id,
                            start_y: y,
                            start_scroll: new_scroll,
                            content_height,
                            container_height,
                        });
                    }
                    if let Some((handler_id, scroll_top)) = scroll_handler_to_fire {
                        use rinch_core::events::{EventHandlerId, dispatch_scroll_event};
                        dispatch_scroll_event(EventHandlerId(handler_id), scroll_top);
                    }
                    actions.push(AppAction::RequestRedraw);
                } else {
                    let drag_action = self.handle_click(x, y, scale_factor);
                    actions.extend(drag_action);
                }
            }
            PlatformEvent::MouseDown { x, y, button } => {
                let mut handled = false;
                if button == MouseButton::Right {
                    // Right-click: try oncontextmenu dispatch first
                    if let Some(doc) = &self.doc {
                        let hit_id = {
                            let d = doc.borrow();
                            hit_test(&d.tree, x, y)
                        };
                        if let Some(hit_id) = hit_id {
                            let vw = window_size.0 as f32 / scale_factor as f32;
                            let vh = window_size.1 as f32 / scale_factor as f32;
                            if Self::dispatch_oncontextmenu(doc, hit_id, x, y, vw, vh) {
                                actions.push(AppAction::RequestRedraw);
                                handled = true;
                            }
                        }
                    }
                }
                if !handled {
                    // Non-left button clicks (or right-click with no contextmenu handler):
                    // use handle_click_with_button so the surface gets focus.
                    let click_actions = self.handle_click_with_button(x, y, scale_factor, button);
                    actions.extend(click_actions);
                }
            }
            PlatformEvent::MouseUp { x, y, button } => {
                // ── Drag-and-drop: complete or cancel ─────────────────────
                if let Some(pending) = self.pending_drag.take() {
                    // Threshold was never crossed — fire normal click instead
                    let (px, py) = pending.mousedown_pos;
                    let click_actions = self.handle_click(px, py, scale_factor);
                    actions.extend(click_actions);
                } else if let Some(drag) = self.active_dnd.take() {
                    // Fire ondrop on target if present
                    if let Some(target_id) = drag.over_target {
                        if let Some(doc) = &self.doc {
                            Self::dispatch_drag_attr(doc, target_id, "data-ondrop");
                            Self::dispatch_drag_attr(doc, target_id, "data-ondragleave");
                        }
                    }
                    // Fire ondragend on source — set click context so handler can read cursor pos.
                    if let Some(doc) = &self.doc {
                        let (cx, cy) = drag.cursor;
                        events::set_click_context(events::ClickContext {
                            mouse_x: cx,
                            mouse_y: cy,
                            element_x: 0.0,
                            element_y: 0.0,
                            element_width: 0.0,
                            element_height: 0.0,
                            text_hit: Default::default(),
                            viewport_width: 0.0,
                            viewport_height: 0.0,
                        });
                        Self::dispatch_drag_attr(doc, drag.node_id, "data-ondragend");
                    }
                    self.scene_dirty = true;
                    actions.push(AppAction::RequestRedraw);
                }

                // Dispatch MouseUp to focused render surface
                if let Some(surface_id) = crate::render_surface::focused_surface_id() {
                    if let Some(doc) = &self.doc {
                        let surface_hit = {
                            let d = doc.borrow();
                            if let Some(hit_id) = hit_test(&d.tree, x, y) {
                                Self::find_render_surface_at(&d.tree, hit_id, x, y)
                            } else {
                                None
                            }
                        };
                        let (local_x, local_y) = surface_hit
                            .filter(|(sid, _, _)| *sid == surface_id)
                            .map(|(_, lx, ly)| (lx, ly))
                            .unwrap_or((x, y));
                        crate::render_surface::dispatch_surface_event(
                            surface_id,
                            crate::render_surface::SurfaceEvent::MouseUp {
                                x: local_x,
                                y: local_y,
                                button: crate::render_surface::SurfaceMouseButton::from_platform(
                                    button,
                                ),
                            },
                        );
                    }
                }

                rinch_core::finish_drag(x, y);
                self.scrollbar_drag = None;
                self.ce_selecting = false;
                self.text_selecting = false;

                // Clear :active pseudo-class state on mouse release.
                // Don't request redraw — AboutToWait batches dirty state.
                if let Some(doc) = &self.doc {
                    doc.borrow_mut().update_active(None);
                }
            }
            PlatformEvent::MouseWheel {
                x,
                y,
                delta_x,
                delta_y,
            } => {
                self.cursor_pos = Some((x, y));

                // Check if scrolling over a render surface — dispatch and skip normal scroll
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
                                delta_x: delta_x as f32,
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

                if !surface_consumed && let Some(doc) = &self.doc {
                    let hit_node = hit_test(&doc.borrow().tree, x, y);
                    if let Some(hit_node) = hit_node {
                        let mut doc_mut = doc.borrow_mut();
                        let mut scroll_handler_to_fire: Option<(usize, f64)> = None;

                        // Vertical scrolling
                        // First try the hit node's ancestor chain. If the hit node is in
                        // a different DOM branch (e.g., an absolutely-positioned overlay),
                        // fall back to finding the scroll container geometrically at (x, y).
                        if delta_y.abs() > 0.0
                            && let Some(scroll_node_id) =
                                find_scroll_container(&doc_mut.tree, hit_node)
                                    .or_else(|| find_scroll_container_at_point(&doc_mut.tree, x, y))
                        {
                            let content_height =
                                compute_content_height(&doc_mut.tree, scroll_node_id);
                            let visible_height =
                                compute_visible_content_area_height(&doc_mut.tree, scroll_node_id);
                            let max_scroll = (content_height - visible_height).max(0.0);

                            // Read handler ID before mutable borrow
                            let handler_id = doc_mut
                                .tree
                                .nodes
                                .get(scroll_node_id)
                                .and_then(|n| n.attributes.get("data-onscroll"))
                                .and_then(|s| s.parse::<usize>().ok());
                            let old_y = doc_mut
                                .tree
                                .nodes
                                .get(scroll_node_id)
                                .map(|n| n.scroll_offset.1)
                                .unwrap_or(0.0);
                            let new_y = (old_y - delta_y).clamp(0.0, max_scroll);
                            if new_y != old_y {
                                if let Some(node) = doc_mut.tree.nodes.get_mut(scroll_node_id) {
                                    node.scroll_offset.1 = new_y;
                                    node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                                    doc_mut.tree.push_dirty(scroll_node_id);
                                    self.scene_dirty = true;
                                }
                                doc_mut.tree.dirty_nodes.insert(scroll_node_id);
                                if let Some(hid) = handler_id {
                                    scroll_handler_to_fire = Some((hid, new_y));
                                }
                            }
                        }

                        // Horizontal scrolling
                        if delta_x.abs() > 0.0
                            && let Some(scroll_node_id) = find_horizontal_scroll_container(
                                &doc_mut.tree,
                                hit_node,
                            )
                            .or_else(|| {
                                find_horizontal_scroll_container_at_point(&doc_mut.tree, x, y)
                            })
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
                                    doc_mut.tree.push_dirty(scroll_node_id);
                                    self.scene_dirty = true;
                                }
                            }
                        }

                        drop(doc_mut);
                        if let Some((handler_id, scroll_top)) = scroll_handler_to_fire {
                            use rinch_core::events::{EventHandlerId, dispatch_scroll_event};
                            dispatch_scroll_event(EventHandlerId(handler_id), scroll_top);
                        }
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
                // ── Drag-and-drop: Escape cancels active drag ─────────────
                if key == KeyCode::Escape {
                    if let Some(drag) = self.active_dnd.take() {
                        if let Some(doc) = &self.doc {
                            if let Some(target_id) = drag.over_target {
                                Self::dispatch_drag_attr(doc, target_id, "data-ondragleave");
                            }
                            Self::dispatch_drag_attr(doc, drag.node_id, "data-ondragend");
                        }
                        self.scene_dirty = true;
                        actions.push(AppAction::RequestRedraw);
                        return actions;
                    }
                    if self.pending_drag.take().is_some() {
                        return actions;
                    }
                }

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
                    // Modifier keys (as physical key presses)
                    KeyCode::ShiftLeft => Some("Shift".into()),
                    KeyCode::ShiftRight => Some("Shift".into()),
                    KeyCode::ControlLeft => Some("Control".into()),
                    KeyCode::ControlRight => Some("Control".into()),
                    KeyCode::AltLeft => Some("Alt".into()),
                    KeyCode::AltRight => Some("Alt".into()),
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
                    // If a render surface is focused, forward KeyDown + text input
                    if let Some(surface_id) = crate::render_surface::focused_surface_id() {
                        crate::render_surface::dispatch_surface_event(
                            surface_id,
                            crate::render_surface::SurfaceEvent::KeyDown(
                                crate::render_surface::SurfaceKeyData {
                                    key: key_str.clone().unwrap_or_default(),
                                    code: format!("{:?}", key),
                                    ctrl,
                                    shift,
                                    alt,
                                    meta: false,
                                },
                            ),
                        );
                        if let Some(ref t) = text {
                            if !t.is_empty() && t.chars().all(|c| !c.is_control()) {
                                crate::render_surface::dispatch_surface_event(
                                    surface_id,
                                    crate::render_surface::SurfaceEvent::TextInput(t.clone()),
                                );
                            }
                        }
                    }
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
                        actions.push(AppAction::ToggleDevTools);
                        return actions;
                    }

                    // Alt+I: toggle inspect mode
                    if key == KeyCode::KeyI && alt && !ctrl && !shift {
                        actions.push(AppAction::ToggleInspectMode);
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
            PlatformEvent::KeyUp { key, modifiers } => {
                // Forward key release to focused render surface.
                if let Some(surface_id) = crate::render_surface::focused_surface_id() {
                    let key_str = format!("{:?}", key);
                    crate::render_surface::dispatch_surface_event(
                        surface_id,
                        crate::render_surface::SurfaceEvent::KeyUp(
                            crate::render_surface::SurfaceKeyData {
                                key: key_str.clone(),
                                code: key_str,
                                ctrl: modifiers.primary(),
                                shift: modifiers.shift,
                                alt: modifiers.alt,
                                meta: modifiers.meta,
                            },
                        ),
                    );
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
                // Process any pending input focus request (e.g., from an Effect
                // triggered by run_on_main_thread that called request_focus).
                if let Some(focus_node_id) = rinch_core::take_pending_focus_request() {
                    self.try_focus_input(focus_node_id);
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
            PlatformEvent::FileHoverEnter { position, .. }
            | PlatformEvent::FileDragMoved { position } => {
                // OS file drag entered or is moving over the window.
                // Hit-test the drag position and fire data-onfiledragenter
                // on the first ancestor with that attribute (like ondragenter for
                // internal DnD).
                let (x, y) = (position.0 as f32, position.1 as f32);
                if let Some(doc) = &self.doc {
                    let hit_id = {
                        let d = doc.borrow();
                        hit_test(&d.tree, x, y)
                    };
                    if let Some(hit_id) = hit_id {
                        let target = Self::find_file_drop_target(&doc.borrow().tree, hit_id);
                        let old_target = self.file_hover_target;
                        if target != old_target {
                            // Fire leave on old target, enter on new target
                            if let Some(old_id) = old_target {
                                Self::dispatch_drag_attr(doc, old_id, "data-onfiledragleave");
                            }
                            if let Some(new_id) = target {
                                Self::dispatch_drag_attr(doc, new_id, "data-onfiledragenter");
                            }
                            self.file_hover_target = target;
                            actions.push(AppAction::RequestRedraw);
                        }
                    }
                }
            }
            PlatformEvent::FileHoverCancelled => {
                // OS file drag left the window without dropping.
                if let Some(target_id) = self.file_hover_target.take() {
                    if let Some(doc) = &self.doc {
                        Self::dispatch_drag_attr(doc, target_id, "data-onfiledragleave");
                        actions.push(AppAction::RequestRedraw);
                    }
                }
            }
            PlatformEvent::FileDropped { paths, position } => {
                // Files were dropped from the OS. Hit-test and dispatch to the
                // nearest ancestor with data-onfiledrop.
                let (x, y) = (position.0 as f32, position.1 as f32);
                if let Some(doc) = &self.doc {
                    let hit_id = {
                        let d = doc.borrow();
                        hit_test(&d.tree, x, y)
                    };
                    if let Some(hit_id) = hit_id {
                        Self::dispatch_file_drop(doc, hit_id, paths);
                    }
                    // Clean up hover state
                    if let Some(target_id) = self.file_hover_target.take() {
                        Self::dispatch_drag_attr(doc, target_id, "data-onfiledragleave");
                    }
                    actions.push(AppAction::RequestRedraw);
                }
            }
            PlatformEvent::AboutToWait => {
                // Tick active CSS transitions — this updates interpolated values
                // in computed_style and marks affected nodes dirty.
                let any_transitions = if let Some(doc) = &self.doc {
                    doc.borrow_mut().tick_transitions()
                } else {
                    false
                };

                let any_animations = if let Some(doc) = &self.doc {
                    doc.borrow_mut().tick_animations()
                } else {
                    false
                };

                // Transitions/animations modify computed_style directly — mark scene dirty
                // so build_scene() rebuilds the Vello scene with interpolated values.
                if any_transitions || any_animations {
                    self.scene_dirty = true;
                }

                if self.has_dirty_nodes() {
                    let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                    if self.resolve_and_repaint(w, h) {
                        actions.push(AppAction::RequestRedraw);
                    }
                }

                // Process any pending input focus request from effects
                if let Some(focus_node_id) = rinch_core::take_pending_focus_request() {
                    self.try_focus_input(focus_node_id);
                    actions.push(AppAction::RequestRedraw);
                }

                // Poll active video players for signal updates (position, duration, etc.)
                // and keep the render loop active while video is playing.
                let any_video = {
                    #[cfg(feature = "video")]
                    {
                        let active = rinch_video::is_video_active();
                        if active {
                            rinch_video::poll_active_players();
                        }
                        active
                    }
                    #[cfg(not(feature = "video"))]
                    {
                        false
                    }
                };

                // Video polling may have dirtied nodes (signal updates) — check again
                if any_video && self.has_dirty_nodes() {
                    let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                    if self.resolve_and_repaint(w, h) {
                        actions.push(AppAction::RequestRedraw);
                    }
                }

                if any_transitions || any_animations || any_video {
                    actions.push(AppAction::RequestRedraw);
                }
            }
        }

        actions
    }

    /// Dispatch `data-onenter` handler for the hovered node or its ancestors.
    ///
    /// Walks up from `hit_id` looking for a `data-onenter` attribute. If found,
    /// dispatches the registered event handler. Used for menu hover-to-switch.
    pub(super) fn dispatch_onenter(doc: &Rc<RefCell<RinchDocument>>, hit_id: usize) {
        let handler_id = {
            let d = doc.borrow();
            let mut current = Some(hit_id);
            let mut found = None;
            while let Some(nid) = current {
                if let Some(node) = d.tree.get(nid) {
                    if let Some(val) = node.attributes.get("data-onenter") {
                        if let Ok(id) = val.parse::<usize>() {
                            found = Some(id);
                        }
                        break;
                    }
                    current = node.parent;
                } else {
                    break;
                }
            }
            found
        };
        if let Some(id) = handler_id {
            events::dispatch_event(events::EventHandlerId(id));
        }
    }

    /// Dispatch `data-onleave` handler for the previously hovered node or its ancestors.
    ///
    /// Walks up from `old_id` looking for a `data-onleave` attribute. If found,
    /// dispatches the registered event handler. Used for tooltip hover-out.
    pub(super) fn dispatch_onleave(doc: &Rc<RefCell<RinchDocument>>, old_id: usize) {
        let handler_id = {
            let d = doc.borrow();
            let mut current = Some(old_id);
            let mut found = None;
            while let Some(nid) = current {
                if let Some(node) = d.tree.get(nid) {
                    if let Some(val) = node.attributes.get("data-onleave") {
                        if let Ok(id) = val.parse::<usize>() {
                            found = Some(id);
                        }
                        break;
                    }
                    current = node.parent;
                } else {
                    break;
                }
            }
            found
        };
        if let Some(id) = handler_id {
            events::dispatch_event(events::EventHandlerId(id));
        }
    }

    /// Dispatch `data-oncontextmenu` handler for the right-clicked node or its ancestors.
    ///
    /// Walks up from `hit_id` looking for a `data-oncontextmenu` attribute.
    /// If found, sets the [`ClickContext`] with mouse position and element bounds,
    /// then dispatches the registered event handler. Returns `true` if a handler
    /// was found and dispatched.
    pub(super) fn dispatch_oncontextmenu(
        doc: &Rc<RefCell<RinchDocument>>,
        hit_id: usize,
        x: f32,
        y: f32,
        viewport_width: f32,
        viewport_height: f32,
    ) -> bool {
        let handler_info = {
            let d = doc.borrow();
            let mut current = Some(hit_id);
            let mut found = None;
            while let Some(nid) = current {
                if let Some(node) = d.tree.get(nid) {
                    if let Some(val) = node.attributes.get("data-oncontextmenu") {
                        if let Ok(id) = val.parse::<usize>() {
                            // Compute element absolute position
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
                            found = Some((id, ax, ay, node.layout.width, node.layout.height));
                        }
                        break;
                    }
                    current = node.parent;
                } else {
                    break;
                }
            }
            found
        };
        if let Some((id, elem_x, elem_y, elem_w, elem_h)) = handler_info {
            events::set_click_context(events::ClickContext {
                mouse_x: x,
                mouse_y: y,
                element_x: elem_x,
                element_y: elem_y,
                element_width: elem_w,
                element_height: elem_h,
                text_hit: events::TextHitInfo::default(),
                viewport_width,
                viewport_height,
            });
            events::dispatch_event(events::EventHandlerId(id));
            true
        } else {
            false
        }
    }

    // ── Drag-and-drop helpers ─────────────────────────────────────────────

    /// Walk up from `hit_id` looking for a `draggable="true"` attribute.
    /// Returns the node ID of the first draggable ancestor (or self).
    pub(crate) fn find_draggable(tree: &rinch_dom::NodeTree, hit_id: usize) -> Option<usize> {
        let mut current = Some(hit_id);
        while let Some(nid) = current {
            if let Some(node) = tree.get(nid) {
                if node.attributes.get("draggable").map(|v| v.as_str()) == Some("true") {
                    return Some(nid);
                }
                current = node.parent;
            } else {
                break;
            }
        }
        None
    }

    /// Walk up from `hit_id` looking for a `data-ondrop` attribute.
    /// Returns the node ID of the first drop target ancestor (or self).
    pub(crate) fn find_drop_target(tree: &rinch_dom::NodeTree, hit_id: usize) -> Option<usize> {
        let mut current = Some(hit_id);
        while let Some(nid) = current {
            if let Some(node) = tree.get(nid) {
                if node.attributes.contains_key("data-ondrop") {
                    return Some(nid);
                }
                current = node.parent;
            } else {
                break;
            }
        }
        None
    }

    /// Dispatch a drag event attribute with ClickContext set to cursor position
    /// and target element bounds. Used for `data-ondragover` where handlers need
    /// to know the cursor position relative to the target.
    pub(crate) fn dispatch_drag_attr_with_context(
        doc: &Rc<RefCell<RinchDocument>>,
        node_id: usize,
        attr: &str,
        cursor_x: f32,
        cursor_y: f32,
    ) {
        let handler_info = {
            let d = doc.borrow();
            let mut current = Some(node_id);
            let mut found = None;
            while let Some(nid) = current {
                if let Some(node) = d.tree.get(nid) {
                    if let Some(val) = node.attributes.get(attr) {
                        if let Ok(id) = val.parse::<usize>() {
                            // Compute element absolute position
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
                            found = Some((id, ax, ay, node.layout.width, node.layout.height));
                        }
                        break;
                    }
                    current = node.parent;
                } else {
                    break;
                }
            }
            found
        };
        if let Some((id, elem_x, elem_y, elem_w, elem_h)) = handler_info {
            events::set_click_context(events::ClickContext {
                mouse_x: cursor_x,
                mouse_y: cursor_y,
                element_x: elem_x,
                element_y: elem_y,
                element_width: elem_w,
                element_height: elem_h,
                text_hit: events::TextHitInfo::default(),
                viewport_width: 0.0,
                viewport_height: 0.0,
            });
            events::dispatch_event(events::EventHandlerId(id));
        }
    }

    /// Dispatch a drag event attribute (data-ondragstart, data-ondrop, etc.)
    /// by walking up from `node_id` looking for the specified attribute.
    pub(crate) fn dispatch_drag_attr(doc: &Rc<RefCell<RinchDocument>>, node_id: usize, attr: &str) {
        let handler_id = {
            let d = doc.borrow();
            let mut current = Some(node_id);
            let mut found = None;
            while let Some(nid) = current {
                if let Some(node) = d.tree.get(nid) {
                    if let Some(val) = node.attributes.get(attr) {
                        if let Ok(id) = val.parse::<usize>() {
                            found = Some(id);
                        }
                        break;
                    }
                    current = node.parent;
                } else {
                    break;
                }
            }
            found
        };
        if let Some(id) = handler_id {
            events::dispatch_event(events::EventHandlerId(id));
        }
    }

    /// Walk up from `hit_id` looking for a `data-onfiledrop` attribute.
    /// Returns the node ID of the first file-drop target ancestor (or self).
    pub(crate) fn find_file_drop_target(
        tree: &rinch_dom::NodeTree,
        hit_id: usize,
    ) -> Option<usize> {
        let mut current = Some(hit_id);
        while let Some(nid) = current {
            if let Some(node) = tree.get(nid) {
                if node.attributes.contains_key("data-onfiledrop") {
                    return Some(nid);
                }
                current = node.parent;
            } else {
                break;
            }
        }
        None
    }

    /// Dispatch an OS file-drop event by walking up from `hit_id` looking for
    /// `data-onfiledrop` and invoking the registered `FileDropCallback`.
    pub(crate) fn dispatch_file_drop(
        doc: &Rc<RefCell<RinchDocument>>,
        hit_id: usize,
        paths: Vec<std::path::PathBuf>,
    ) {
        let handler_id = {
            let d = doc.borrow();
            let mut current = Some(hit_id);
            let mut found = None;
            while let Some(nid) = current {
                if let Some(node) = d.tree.get(nid) {
                    if let Some(val) = node.attributes.get("data-onfiledrop") {
                        if let Ok(id) = val.parse::<usize>() {
                            found = Some(id);
                        }
                        break;
                    }
                    current = node.parent;
                } else {
                    break;
                }
            }
            found
        };
        if let Some(id) = handler_id {
            events::dispatch_file_drop_event(events::EventHandlerId(id), paths);
        }
    }

    /// Transition from pending drag to active drag: capture snapshot and set
    /// up the active drag state.
    pub(crate) fn activate_drag(
        &mut self,
        node_id: usize,
        mousedown_pos: (f32, f32),
        cursor: (f32, f32),
        scale_factor: f64,
    ) {
        let Some(doc) = &self.doc else { return };

        let anchor;

        #[cfg(feature = "gpu")]
        let snapshot = {
            let mut painter = VelloPainter::new();
            let mut d = doc.borrow_mut();
            let d = &mut *d;

            let (abs_x, abs_y) =
                rinch_dom::paint::compute_absolute_position(&d.tree, node_id, scale_factor);
            anchor = (
                mousedown_pos.0 - abs_x as f32,
                mousedown_pos.1 - abs_y as f32,
            );

            rinch_dom::paint::paint_subtree(
                &d.tree,
                &mut painter,
                node_id,
                scale_factor,
                &mut d.font_cx,
                &mut d.layout_cx,
            );
            painter
        };

        #[cfg(not(feature = "gpu"))]
        let (snapshot_pixels, snapshot_width, snapshot_height) = {
            let mut d = doc.borrow_mut();
            let d = &mut *d;

            let (abs_x, abs_y) =
                rinch_dom::paint::compute_absolute_position(&d.tree, node_id, scale_factor);
            anchor = (
                mousedown_pos.0 - abs_x as f32,
                mousedown_pos.1 - abs_y as f32,
            );

            // Compute the element's size in physical pixels for the snapshot pixmap
            let node = d.tree.get(node_id);
            let (sw, sh) = node
                .map(|n| {
                    let w = (n.layout.width as f64 * scale_factor).ceil() as u32;
                    let h = (n.layout.height as f64 * scale_factor).ceil() as u32;
                    (w.max(1), h.max(1))
                })
                .unwrap_or((1, 1));

            let mut painter = TinySkiaPainter::new(sw, sh);
            painter.fill_transparent();

            rinch_dom::paint::paint_subtree(
                &d.tree,
                &mut painter,
                node_id,
                scale_factor,
                &mut d.font_cx,
                &mut d.layout_cx,
            );
            (painter.pixels().to_vec(), sw, sh)
        };

        self.active_dnd = Some(ActiveDrag {
            node_id,
            #[cfg(feature = "gpu")]
            snapshot,
            #[cfg(not(feature = "gpu"))]
            snapshot_pixels,
            #[cfg(not(feature = "gpu"))]
            snapshot_width,
            #[cfg(not(feature = "gpu"))]
            snapshot_height,
            anchor,
            cursor,
            over_target: None,
        });
    }
}
