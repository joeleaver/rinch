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
        // Logical viewport dimensions for ClickContext
        let vp_w = window_size.0 as f32 / scale_factor as f32;
        let vp_h = window_size.1 as f32 / scale_factor as f32;

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

                // New editor (M5): extend an in-progress drag-select.
                #[cfg(feature = "new-editor")]
                if crate::editor::drag_anchor().is_some()
                    && self.extend_editor_drag(x, y, scale_factor, window_size)
                {
                    actions.push(AppAction::RequestRedraw);
                    return actions;
                }

                // Additive: fire data-onmousemove before any drag/scroll/hover
                // logic below (which can early-return).
                self.dispatch_mouse_attr(
                    "data-onmousemove",
                    x,
                    y,
                    events::MouseButton::Left,
                    vp_w,
                    vp_h,
                );

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

                    // Hit test for drop targets — check both DOM elements and surfaces
                    let hit_id = if let Some(doc) = &self.doc {
                        let d = doc.borrow();
                        hit_test(&d.tree, x, y)
                    } else {
                        None
                    };

                    // Check if mouse is over a render surface
                    let surface_target = if let Some(doc) = &self.doc {
                        hit_id.and_then(|hid| {
                            let d = doc.borrow();
                            Self::find_render_surface_at_full(&d.tree, hid, x, y)
                        })
                    } else {
                        None
                    };

                    let new_surface = surface_target.as_ref().map(|(sid, nid, _, _)| (*sid, *nid));

                    // Handle surface drag enter/leave transitions
                    if new_surface != self.drag_over_surface {
                        // Leave old surface
                        if let Some((old_sid, _)) = self.drag_over_surface {
                            crate::render_surface::dispatch_surface_event(
                                old_sid,
                                crate::render_surface::SurfaceEvent::DragLeave,
                            );
                        }
                        // Enter new surface
                        if let Some((sid, _, lx, ly)) = &surface_target {
                            crate::render_surface::dispatch_surface_event(
                                *sid,
                                crate::render_surface::SurfaceEvent::DragEnter { x: *lx, y: *ly },
                            );
                        }
                        self.drag_over_surface = new_surface;
                    }

                    // Dispatch DragOver to current surface, or DOM drag events
                    if let Some((sid, _, lx, ly)) = surface_target {
                        // Mouse is over a surface — dispatch DragOver
                        crate::render_surface::dispatch_surface_event(
                            sid,
                            crate::render_surface::SurfaceEvent::DragOver { x: lx, y: ly },
                        );

                        // Clear DOM drop target since we're over a surface
                        let old_target = drag.over_target.take();
                        if let Some(doc) = &self.doc {
                            if let Some(old_id) = old_target {
                                Self::dispatch_drag_attr(doc, old_id, "data-ondragleave");
                            }
                        }
                    } else {
                        // Not over a surface — use DOM drop target system
                        let new_target = hit_id.and_then(|hid| {
                            self.doc.as_ref().and_then(|doc| {
                                let d = doc.borrow();
                                Self::find_drop_target(&d.tree, hid)
                            })
                        });

                        let old_target = drag.over_target;
                        if new_target != old_target {
                            drag.over_target = new_target;
                            if let Some(doc) = &self.doc {
                                if let Some(old_id) = old_target {
                                    Self::dispatch_drag_attr(doc, old_id, "data-ondragleave");
                                }
                                if let Some(new_id) = new_target {
                                    Self::dispatch_drag_attr(doc, new_id, "data-ondragenter");
                                }
                            }
                        }

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
                            viewport_width: vp_w,
                            viewport_height: vp_h,
                            button: events::MouseButton::Left,
                            modifiers: events::ModifierState::default(),
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
                let (drag_active, drag_forward_surface) = rinch_core::update_drag(x, y);
                if drag_active && !drag_forward_surface {
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

                    // Dispatch MouseMove + MouseEnter/MouseLeave to render surfaces.
                    // During an active drag with forward_surface_events, dispatch MouseMove
                    // to the captured surface even when the mouse is outside its bounds
                    // (pointer capture semantics).
                    let new_surface = if let Some(hit_id) = hovered {
                        let d = doc.borrow();
                        Self::find_render_surface_at_full(&d.tree, hit_id, x, y)
                    } else {
                        None
                    };

                    let new_surface_entry = new_surface.as_ref().map(|(id, nid, _, _)| (*id, *nid));

                    if new_surface_entry != self.hovered_surface {
                        if !drag_active {
                            // Normal (non-drag) path: dispatch enter/leave
                            if let Some((old_id, _)) = self.hovered_surface {
                                crate::render_surface::dispatch_surface_event(
                                    old_id,
                                    crate::render_surface::SurfaceEvent::MouseLeave,
                                );
                            }
                            if let Some((sid, _, lx, ly)) = &new_surface {
                                crate::render_surface::dispatch_surface_event(
                                    *sid,
                                    crate::render_surface::SurfaceEvent::MouseEnter {
                                        x: *lx,
                                        y: *ly,
                                    },
                                );
                            }
                            self.hovered_surface = new_surface_entry;
                        }
                        // During drag: don't update hovered_surface or send
                        // enter/leave — the captured surface keeps getting events.
                    }

                    if let Some((surface_id, _, local_x, local_y)) = new_surface {
                        // Mouse is over a surface — dispatch with hit-tested coords
                        crate::render_surface::dispatch_surface_event(
                            surface_id,
                            crate::render_surface::SurfaceEvent::MouseMove {
                                x: local_x,
                                y: local_y,
                            },
                        );
                    } else if drag_active && drag_forward_surface {
                        // Mouse is OFF any surface during a drag with forwarding.
                        // Pointer capture: dispatch to the captured surface with
                        // coordinates relative to its bounds (may be negative or
                        // beyond width/height).
                        if let Some((captured_sid, captured_nid)) = self.hovered_surface {
                            let d = doc.borrow();
                            let (local_x, local_y) =
                                Self::surface_local_coords(&d.tree, captured_nid, x, y);
                            drop(d);
                            crate::render_surface::dispatch_surface_event(
                                captured_sid,
                                crate::render_surface::SurfaceEvent::MouseMove {
                                    x: local_x,
                                    y: local_y,
                                },
                            );
                        }
                    }
                }
            }
            PlatformEvent::MouseDown {
                x,
                y,
                button: MouseButton::Left,
            } => {
                // Additive: fire data-onmousedown before the resize/drag/scroll/
                // click logic below (which can early-return).
                self.dispatch_mouse_attr(
                    "data-onmousedown",
                    x,
                    y,
                    events::MouseButton::Left,
                    vp_w,
                    vp_h,
                );

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

                // New editor (M5): a left click inside an editor places the cursor
                // (and arms a drag-select). This MUST run in the Left-specific arm —
                // the general MouseDown arm below never sees left buttons (the cause
                // of the original "click does nothing" bug).
                #[cfg(feature = "new-editor")]
                if self.try_new_editor_click(
                    x,
                    y,
                    scale_factor,
                    window_size,
                    self.click_count,
                    self.modifiers.shift,
                ) {
                    actions.push(AppAction::RequestRedraw);
                    return actions;
                }

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
                    let drag_action = self.handle_click(x, y, scale_factor, vp_w, vp_h);
                    actions.extend(drag_action);
                }
            }
            PlatformEvent::MouseDown { x, y, button } => {
                self.dispatch_mouse_attr(
                    "data-onmousedown",
                    x,
                    y,
                    Self::core_button(button),
                    vp_w,
                    vp_h,
                );

                let mut handled = false;
                if button == MouseButton::Right {
                    // Right-click: try oncontextmenu dispatch first
                    let mods = self.modifier_state();
                    if let Some(doc) = &self.doc {
                        let hit_id = {
                            let d = doc.borrow();
                            hit_test(&d.tree, x, y)
                        };
                        if let Some(hit_id) = hit_id {
                            let vw = window_size.0 as f32 / scale_factor as f32;
                            let vh = window_size.1 as f32 / scale_factor as f32;
                            if Self::dispatch_oncontextmenu(doc, hit_id, x, y, vw, vh, mods) {
                                actions.push(AppAction::RequestRedraw);
                                handled = true;
                            }
                        }
                    }
                }
                if !handled {
                    // Non-left button clicks (or right-click with no contextmenu handler):
                    // use handle_click_with_button so the surface gets focus.
                    let click_actions =
                        self.handle_click_with_button(x, y, scale_factor, button, vp_w, vp_h);
                    actions.extend(click_actions);
                }
            }
            PlatformEvent::MouseUp { x, y, button } => {
                self.dispatch_mouse_attr(
                    "data-onmouseup",
                    x,
                    y,
                    Self::core_button(button),
                    vp_w,
                    vp_h,
                );

                // New editor (M5): end any drag-select.
                #[cfg(feature = "new-editor")]
                crate::editor::end_drag();

                // ── Drag-and-drop: complete or cancel ─────────────────────
                if let Some(pending) = self.pending_drag.take() {
                    // Threshold was never crossed — fire normal click instead
                    let (px, py) = pending.mousedown_pos;
                    let click_actions = self.handle_click(px, py, scale_factor, vp_w, vp_h);
                    actions.extend(click_actions);
                } else if let Some(drag) = self.active_dnd.take() {
                    // Check if dropping on a surface
                    if let Some((surface_sid, surface_nid)) = self.drag_over_surface.take() {
                        // Dispatch Drop to the surface
                        if let Some(doc) = &self.doc {
                            let d = doc.borrow();
                            let (lx, ly) = Self::surface_local_coords(&d.tree, surface_nid, x, y);
                            drop(d);
                            crate::render_surface::dispatch_surface_event(
                                surface_sid,
                                crate::render_surface::SurfaceEvent::Drop { x: lx, y: ly },
                            );
                            // Also send DragLeave after Drop (cleanup)
                            crate::render_surface::dispatch_surface_event(
                                surface_sid,
                                crate::render_surface::SurfaceEvent::DragLeave,
                            );
                        }
                    } else if let Some(target_id) = drag.over_target {
                        // Dropping on a DOM element
                        if let Some(doc) = &self.doc {
                            Self::dispatch_drag_attr(doc, target_id, "data-ondrop");
                            Self::dispatch_drag_attr(doc, target_id, "data-ondragleave");
                        }
                    }
                    // Fire ondragend on source
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
                            viewport_width: vp_w,
                            viewport_height: vp_h,
                            button: events::MouseButton::Left,
                            modifiers: self.modifier_state(),
                        });
                        Self::dispatch_drag_attr(doc, drag.node_id, "data-ondragend");
                    }
                    // Reset ghost visibility for next drag
                    events::reset_drag_ghost_visibility();
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
                            let nid = rinch_core::dom::NodeId(scroll_node_id);
                            let content_height = doc_mut.scroll_height(nid);
                            let visible_height = doc_mut.client_height(nid);
                            let max_scroll = (content_height - visible_height).max(0.0);

                            // Read handler ID before mutable borrow
                            let handler_id = doc_mut
                                .tree
                                .nodes
                                .get(scroll_node_id)
                                .and_then(|n| n.attributes.get("data-onscroll"))
                                .and_then(|s| s.parse::<usize>().ok());
                            let old_y = doc_mut.scroll_top(nid);
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
                            let nid = rinch_core::dom::NodeId(scroll_node_id);
                            let content_width = doc_mut.scroll_width(nid);
                            let visible_width = doc_mut.client_width(nid);
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

                // Build key string for the user keyboard hook + global fallback.
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

                // 1. User keyboard hooks (document-level interceptor). A user
                //    handler that consumes the key wins and stops here, like a
                //    capturing DOM listener. Render surfaces no longer hijack this
                //    slot — they are routed by `FocusTarget::Surface` below.
                if let Some(ref ks) = key_str {
                    let key_data = events::KeyEventData {
                        key: ks.clone(),
                        code: format!("{:?}", key),
                        ctrl,
                        shift,
                        alt,
                        meta: false,
                    };
                    if events::dispatch_keyboard_event(&key_data) {
                        actions.push(AppAction::RequestRedraw);
                        return actions;
                    }
                }

                // 2. Route by the focus arbiter (design A10): exactly one target
                //    owns keyboard input, so there is no order-dependent fallthrough.
                match self.focus_target {
                    #[cfg(feature = "new-editor")]
                    FocusTarget::Editor(container) => {
                        if let Some(handle) = crate::editor::editor_for(container) {
                            self.dispatch_new_editor_key(
                                &handle,
                                key,
                                text.as_deref(),
                                shift,
                                ctrl,
                                alt,
                            );
                            // Position the caret against the current layout (dirtying
                            // its block if it reparented), re-layout, then the
                            // post-layout caret pass finalizes it with fresh geometry.
                            self.refresh_editor_overlays();
                            let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                            self.resolve_and_repaint(w, h);
                            actions.push(AppAction::RequestRedraw);
                        } else {
                            // The focused editor was unmounted out from under us:
                            // drop the stale focus so this key (and future ones)
                            // fall through to the global handlers instead of being
                            // silently swallowed.
                            self.focus_target = FocusTarget::None;
                        }
                    }
                    FocusTarget::Surface(surface_id) => {
                        // Forward KeyDown + text input to the focused surface.
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
                        actions.push(AppAction::RequestRedraw);
                    }
                    FocusTarget::ContentEditable(_) => {
                        // Route keyboard events to the legacy contenteditable engine.
                        if self.handle_contenteditable_key(key, text.as_deref(), shift, ctrl, alt) {
                            // Resolve layout immediately so the IFC text layout is
                            // rebuilt before the next paint. Without this, the
                            // invalidated text_layout (set to None by set_text_content)
                            // causes a one-frame flicker where text is invisible.
                            let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                            self.resolve_and_repaint(w, h);
                            actions.push(AppAction::RequestRedraw);
                        }
                    }
                    // No widget owns the key, or a plain `<input>` does (its editing
                    // commands live in the global handlers, gated internally on
                    // `focused_input_state`). Falls through to DevTools / inspect /
                    // Tab navigation / read-only text-selection caret motion.
                    FocusTarget::Input(_) | FocusTarget::None => {
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
                            KeyCode::Tab => self.handle_tab(shift),
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
            PlatformEvent::Ime(ime) => {
                // Route IME composition through the focus arbiter (design A10),
                // exactly like KeyDown: whichever text target holds focus consumes
                // it. IME is a shared runtime service, not a per-widget path.
                match self.focus_target {
                    #[cfg(feature = "new-editor")]
                    FocusTarget::Editor(container) => {
                        if let Some(handle) = crate::editor::editor_for(container) {
                            self.dispatch_editor_ime(&handle, ime);
                            self.refresh_editor_overlays();
                            let (w, h) = (window_size.0 as f32, window_size.1 as f32);
                            self.resolve_and_repaint(w, h);
                            actions.push(AppAction::RequestRedraw);
                        } else {
                            // Focused editor unmounted out from under us.
                            self.focus_target = FocusTarget::None;
                        }
                    }
                    FocusTarget::Input(node_id) => {
                        self.dispatch_input_ime(node_id, ime);
                        actions.push(AppAction::RequestRedraw);
                    }
                    // Surface / legacy contenteditable / None ignore IME. (Legacy
                    // CE is removed at M8 and the new editor replaces it, so wiring
                    // IME there would be throwaway.)
                    _ => {}
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
        modifiers: events::ModifierState,
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
                button: events::MouseButton::Right,
                modifiers,
            });
            events::dispatch_event(events::EventHandlerId(id));
            true
        } else {
            false
        }
    }

    /// Build an absolute-positioned ancestor chain for `hit_id`, from immediate
    /// parent (index 0) out to the document root.
    ///
    /// Used by [`events::set_click_ancestors`] before dispatching click handlers
    /// so handlers can convert click coords into an arbitrary ancestor's frame
    /// (see [`events::find_click_ancestor`]). One pass from root to hit so the
    /// total cost is O(depth), not O(depth²) like a per-ancestor walk-up.
    pub(crate) fn collect_click_ancestors(
        tree: &rinch_dom::NodeTree,
        hit_id: usize,
    ) -> Vec<events::AncestorBounds> {
        // Chain from hit upward: [hit, parent, grandparent, ..., root]
        let mut chain: Vec<usize> = Vec::new();
        let mut cur = Some(hit_id);
        while let Some(nid) = cur {
            chain.push(nid);
            cur = tree.get(nid).and_then(|n| n.parent);
        }
        if chain.len() <= 1 {
            return Vec::new();
        }

        // Walk root → hit, accumulating absolute (x, y). Each iteration records
        // the *current* node's top-left, then offsets by its scroll so the next
        // iteration (this node's child) sees the post-scroll content origin.
        let mut abs: Vec<events::AncestorBounds> = Vec::with_capacity(chain.len());
        let mut ax = 0.0_f32;
        let mut ay = 0.0_f32;
        for &nid in chain.iter().rev() {
            if let Some(n) = tree.get(nid) {
                ax += n.layout.x;
                ay += n.layout.y;
                abs.push(events::AncestorBounds {
                    tag: n.tag().unwrap_or("").to_string(),
                    id: n.attributes.get("id").cloned().unwrap_or_default(),
                    class: n.attributes.get("class").cloned().unwrap_or_default(),
                    x: ax,
                    y: ay,
                    width: n.layout.width,
                    height: n.layout.height,
                });
                ax -= n.scroll_offset.0 as f32;
                ay -= n.scroll_offset.1 as f32;
            }
        }

        // `abs` is currently [root, ..., hit]. Drop the hit itself and reverse
        // so index 0 is the immediate parent of the hit element.
        abs.pop();
        abs.reverse();
        abs
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
                button: events::MouseButton::Left,
                modifiers: events::ModifierState::default(),
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

    /// Map a platform mouse button onto the `rinch-core` button enum carried by
    /// [`events::ClickContext`].
    pub(super) fn core_button(button: MouseButton) -> events::MouseButton {
        match button {
            MouseButton::Left => events::MouseButton::Left,
            MouseButton::Right => events::MouseButton::Right,
            MouseButton::Middle => events::MouseButton::Middle,
        }
    }

    /// Snapshot the live platform modifier state as the core
    /// [`events::ModifierState`] stored in [`events::ClickContext`].
    pub(super) fn modifier_state(&self) -> events::ModifierState {
        events::ModifierState {
            shift: self.modifiers.shift,
            ctrl: self.modifiers.ctrl,
            alt: self.modifiers.alt,
            meta: self.modifiers.meta,
        }
    }

    /// Hit-test `(x, y)`, walk up for `attr`, set the [`events::ClickContext`]
    /// (cursor + target bounds + button + modifiers), and dispatch the handler.
    ///
    /// Additive notification used for `data-onmousedown`/`data-onmouseup`/
    /// `data-onmousemove`. It never consumes the event, so it is safe to call at
    /// the top of the MouseDown/MouseUp/MouseMove arms before the existing
    /// click/drag/scroll logic (which may early-return).
    pub(super) fn dispatch_mouse_attr(
        &self,
        attr: &str,
        x: f32,
        y: f32,
        button: events::MouseButton,
        vp_w: f32,
        vp_h: f32,
    ) {
        let Some(doc) = &self.doc else { return };
        let handler_info = {
            let d = doc.borrow();
            let Some(hit_id) = hit_test(&d.tree, x, y) else {
                return;
            };
            let mut current = Some(hit_id);
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
                mouse_x: x,
                mouse_y: y,
                element_x: elem_x,
                element_y: elem_y,
                element_width: elem_w,
                element_height: elem_h,
                text_hit: events::TextHitInfo::default(),
                viewport_width: vp_w,
                viewport_height: vp_h,
                button,
                modifiers: self.modifier_state(),
            });
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

/// A cursor motion produced by an arrow/Home/End key (design §7 keyboard).
#[cfg(feature = "new-editor")]
#[derive(Clone, Copy)]
pub(crate) enum Motion {
    CharLeft,
    CharRight,
    WordLeft,
    WordRight,
    LineUp,
    LineDown,
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
}

/// New-editor (M5) keyboard handling, behind the `new-editor` feature.
#[cfg(feature = "new-editor")]
impl RinchApp {
    /// Translate a key press into an action on the focused editor's
    /// [`EditorHandle`](crate::editor::EditorHandle). Returns whether the document
    /// or selection changed (and a repaint/caret refresh is needed).
    pub(crate) fn dispatch_new_editor_key(
        &mut self,
        handle: &crate::editor::EditorHandle,
        key: KeyCode,
        text: Option<&str>,
        shift: bool,
        ctrl: bool,
        _alt: bool,
    ) -> bool {
        // The vertical "goal column" survives only a run of Up/Down — any other key
        // (typing, horizontal arrows, Home/End, edits) abandons it.
        if !matches!(key, KeyCode::ArrowUp | KeyCode::ArrowDown) {
            self.editor_goal_x = None;
        }
        match key {
            KeyCode::Backspace => handle.command("deleteCharBackward"),
            KeyCode::Delete => handle.command("deleteCharForward"),
            KeyCode::Enter if !ctrl => handle.command("enter"),
            // Tab / Shift-Tab move between table cells when the cursor is in a table;
            // outside a table Tab does nothing here (no focus-traversal in the editor).
            KeyCode::Tab => self.tab_cell(handle, shift),
            // Cursor movement / selection extension (Shift extends, Ctrl = word/doc).
            KeyCode::ArrowLeft => self.move_editor(
                handle,
                if ctrl {
                    Motion::WordLeft
                } else {
                    Motion::CharLeft
                },
                shift,
            ),
            KeyCode::ArrowRight => self.move_editor(
                handle,
                if ctrl {
                    Motion::WordRight
                } else {
                    Motion::CharRight
                },
                shift,
            ),
            KeyCode::ArrowUp => self.move_editor(handle, Motion::LineUp, shift),
            KeyCode::ArrowDown => self.move_editor(handle, Motion::LineDown, shift),
            KeyCode::Home => self.move_editor(
                handle,
                if ctrl {
                    Motion::DocStart
                } else {
                    Motion::LineStart
                },
                shift,
            ),
            KeyCode::End => self.move_editor(
                handle,
                if ctrl {
                    Motion::DocEnd
                } else {
                    Motion::LineEnd
                },
                shift,
            ),
            KeyCode::KeyA if ctrl => {
                let state = handle.state();
                let sel = rinch_editor_core::Selection::text(
                    rinch_editor_core::Selection::at_start(&state.doc).head(),
                    rinch_editor_core::Selection::at_end(&state.doc).head(),
                );
                handle.set_selection(sel);
                true
            }
            KeyCode::KeyB if ctrl => handle.command("toggleBold"),
            KeyCode::KeyI if ctrl => handle.command("toggleItalic"),
            KeyCode::KeyU if ctrl => handle.command("toggleUnderline"),
            #[cfg(feature = "clipboard")]
            KeyCode::KeyC if ctrl => {
                self.editor_copy(handle);
                false // copy doesn't change the document or selection
            }
            #[cfg(feature = "clipboard")]
            KeyCode::KeyX if ctrl => self.editor_cut(handle),
            // Paste-and-match-style (Ctrl+Shift+V) must precede plain Ctrl+V — the
            // `if ctrl` arm below would otherwise also match with Shift held.
            #[cfg(feature = "clipboard")]
            KeyCode::KeyV if ctrl && shift => self.editor_paste_plain(handle),
            #[cfg(feature = "clipboard")]
            KeyCode::KeyV if ctrl => self.editor_paste(handle),
            KeyCode::KeyZ if ctrl && shift => handle.command("redo"),
            KeyCode::KeyZ if ctrl => handle.command("undo"),
            KeyCode::KeyY if ctrl => handle.command("redo"),
            _ => {
                if ctrl {
                    return false;
                }
                match text {
                    Some(t) if !t.is_empty() && t.chars().all(|c| !c.is_control()) => {
                        handle.insert_text(t)
                    }
                    _ => false,
                }
            }
        }
    }

    /// Tab / Shift-Tab cell navigation: move the cursor to the start of the next
    /// (or previous) table cell. A forward Tab in the last cell appends a row and
    /// moves into it (ProseMirror behavior). Returns `false` when the cursor isn't in
    /// a table — though the editor focus arbiter consumes the key either way (Tab
    /// never moves focus mid-edit), so a non-table Tab is simply a no-op.
    fn tab_cell(&mut self, handle: &crate::editor::EditorHandle, shift: bool) -> bool {
        use rinch_editor_core::{Pos, Selection, tables};
        let state = handle.state();
        let head = state.selection.head();
        let dir = if shift { -1 } else { 1 };
        if let Some(target) = tables::next_cell_in_table(&state.doc, head, dir) {
            handle.set_selection(Selection::near(&state.doc, Pos(target + 1), 1));
            return true;
        }
        // Forward Tab at the last cell: append a row and move into its first cell.
        if dir > 0
            && tables::cell_at_pos(&state.doc, head).is_some()
            && handle.command("addRowAfter")
        {
            let state = handle.state();
            if let Some(target) = tables::next_cell_in_table(&state.doc, state.selection.head(), 1)
            {
                handle.set_selection(Selection::near(&state.doc, Pos(target + 1), 1));
            }
            return true;
        }
        false
    }

    /// Apply a cursor [`Motion`] to the focused editor: compute the new head and
    /// either collapse the cursor there or, when `extend` (Shift), keep the anchor
    /// and move only the head (extending the selection).
    pub(crate) fn move_editor(
        &mut self,
        handle: &crate::editor::EditorHandle,
        motion: Motion,
        extend: bool,
    ) -> bool {
        use rinch_editor_core::{Pos, Selection};
        let state = handle.state();
        let doc = state.doc.clone();
        let head = state.selection.head();
        let new_head: Option<Pos> = match motion {
            Motion::CharLeft => {
                let near = Selection::near(&doc, Pos(head.0.saturating_sub(1)), -1);
                // A char move that lands on a selectable block atom (a horizontal
                // rule) is a node selection, not a cursor. Honor it when collapsing
                // (not extending) so the leaf is keyboard-reachable and deletable.
                if !extend && matches!(near, Selection::Node(_)) {
                    handle.set_selection(near);
                    return true;
                }
                Some(near.head())
            }
            Motion::CharRight => {
                let near = Selection::near(&doc, Pos((head.0 + 1).min(doc.content_size())), 1);
                if !extend && matches!(near, Selection::Node(_)) {
                    handle.set_selection(near);
                    return true;
                }
                Some(near.head())
            }
            Motion::WordLeft => word_boundary(&doc, head, false),
            Motion::WordRight => word_boundary(&doc, head, true),
            // Visual-line edge for wrapped paragraphs (geometry), falling back to
            // the block edge when the caret has no Parley layout (an empty block).
            Motion::LineStart => self
                .visual_line_bound(handle, head, false)
                .or_else(|| line_bound(&doc, head, false)),
            Motion::LineEnd => self
                .visual_line_bound(handle, head, true)
                .or_else(|| line_bound(&doc, head, true)),
            Motion::DocStart => Some(Selection::at_start(&doc).head()),
            Motion::DocEnd => Some(Selection::at_end(&doc).head()),
            Motion::LineUp | Motion::LineDown => {
                // Establish the goal column from the current caret on the first
                // vertical step, then reuse it so the cursor keeps its horizontal
                // position through short lines instead of drifting to line ends.
                let goal_x = self
                    .editor_goal_x
                    .or_else(|| self.editor_caret_point(handle, head).map(|(x, _, _)| x));
                self.editor_goal_x = goal_x;
                self.vertical_step(handle, head, matches!(motion, Motion::LineDown), goal_x)
                    .map(|sel| sel.head())
            }
        };
        match new_head {
            Some(nh) => {
                let sel = if extend {
                    Selection::text(state.selection.anchor(), nh)
                } else {
                    Selection::cursor(nh)
                };
                handle.set_selection(sel);
                true
            }
            None => false,
        }
    }

    // ── IME (input method editor) ──────────────────────────────────────
    // Both targets consume the same portable `ImeEvent`; only the rendering of
    // the preedit differs (the editor's decoration overlay vs the `<input>`'s
    // `data-preedit` paint splice).

    /// Apply an [`ImeEvent`] to the focused new editor: preedit becomes a
    /// transient overlay (never in the document), commit clears it and inserts
    /// the text in one transaction, delete-surrounding deletes around the caret.
    #[cfg(feature = "new-editor")]
    fn dispatch_editor_ime(&mut self, handle: &crate::editor::EditorHandle, ime: ImeEvent) {
        match ime {
            ImeEvent::Enabled => {}
            ImeEvent::Preedit { text, cursor } => {
                handle.ime_set_preedit(&text, cursor);
            }
            ImeEvent::Commit(text) => {
                handle.ime_commit(&text);
                self.editor_goal_x = None;
            }
            ImeEvent::DeleteSurrounding { before, after } => {
                handle.ime_delete_surrounding(before, after);
                self.editor_goal_x = None;
            }
            ImeEvent::Disabled => {
                handle.ime_clear_preedit();
            }
        }
    }
}

/// IME (input method editor) handling for single-line `<input>` text fields.
///
/// Ungated: `<input>` IME works without the `new-editor` feature. (The editor
/// half above is gated because it needs [`EditorHandle`](crate::editor::EditorHandle).)
impl RinchApp {
    /// Apply an [`ImeEvent`] to the focused `<input>`: preedit is rendered inline
    /// at the caret via the `data-preedit` attribute, commit clears it and inserts
    /// the text, delete-surrounding maps to backward/forward deletes.
    fn dispatch_input_ime(&mut self, node_id: usize, ime: ImeEvent) {
        match ime {
            ImeEvent::Enabled => {}
            ImeEvent::Preedit { text, cursor } => {
                self.focused_input_preedit = if text.is_empty() {
                    None
                } else {
                    Some((text, cursor))
                };
                self.sync_input_preedit_to_dom(node_id);
            }
            ImeEvent::Commit(text) => {
                self.focused_input_preedit = None;
                self.sync_input_preedit_to_dom(node_id);
                if !text.is_empty() {
                    self.handle_input_edit_command(EditCommand::InsertText(text));
                }
            }
            ImeEvent::DeleteSurrounding { before, after } => {
                for _ in 0..before {
                    self.handle_input_edit_command(EditCommand::DeleteBackward);
                }
                for _ in 0..after {
                    self.handle_input_edit_command(EditCommand::DeleteForward);
                }
            }
            ImeEvent::Disabled => {
                self.focused_input_preedit = None;
                self.sync_input_preedit_to_dom(node_id);
            }
        }
    }

    /// Write (or clear) the focused `<input>`'s `data-preedit` attribute so the
    /// paint path renders the composition string inline at the caret.
    fn sync_input_preedit_to_dom(&self, node_id: usize) {
        let Some(doc) = &self.doc else { return };
        let mut d = doc.borrow_mut();
        if let Some(node) = d.tree.nodes.get_mut(node_id) {
            match &self.focused_input_preedit {
                Some((text, _)) => {
                    node.attributes.insert("data-preedit".into(), text.clone());
                }
                None => {
                    node.attributes.remove("data-preedit");
                }
            }
            node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
        }
        d.tree.dirty_nodes.insert(node_id);
    }

    /// The focused `<input>`'s caret rect in logical window space, for the IME
    /// candidate box. Approximated at the field's text origin (left padding, full
    /// height); the exact caret-x is a follow-up.
    pub(crate) fn input_caret_area(&self, node_id: usize) -> Option<(f32, f32, f32, f32)> {
        let doc = self.doc.as_ref()?;
        let d = doc.borrow();
        let node = d.tree.get(node_id)?;
        let pad_l = node.computed_style.padding_left.to_px();
        let (_, _, _, h) = d.query_node_layout(node_id as u64)?;
        let (ax, ay) = Self::compute_absolute_position(&d.tree, node_id);
        Some((ax + pad_l, ay, 1.0, h))
    }
}

#[cfg(feature = "new-editor")]
impl RinchApp {
    /// Copy the focused editor's selection to the clipboard as both `text/html`
    /// (rich) and `text/plain` (the fall-back alternative). A no-op for an empty
    /// selection.
    #[cfg(feature = "clipboard")]
    fn editor_copy(&self, handle: &crate::editor::EditorHandle) {
        if let Some((html, text)) = handle.selection_clipboard() {
            let _ = crate::clipboard::copy_html(&html, Some(&text));
        }
    }

    /// Cut: copy the selection to the clipboard, then delete it. Returns whether the
    /// document changed.
    #[cfg(feature = "clipboard")]
    fn editor_cut(&self, handle: &crate::editor::EditorHandle) -> bool {
        match handle.selection_clipboard() {
            Some((html, text)) => {
                let _ = crate::clipboard::copy_html(&html, Some(&text));
                handle.command("deleteSelection")
            }
            None => false,
        }
    }

    /// Paste the clipboard over the selection, preferring rich `text/html`, then a
    /// raw bitmap image (as a PNG `data:` URL), then `text/plain`. Returns whether
    /// the document changed.
    #[cfg(feature = "clipboard")]
    fn editor_paste(&self, handle: &crate::editor::EditorHandle) -> bool {
        // 1. Rich HTML — preserves structure, links, and images referenced by URL.
        if let Ok(html) = crate::clipboard::paste_html()
            && !html.trim().is_empty()
            && handle.replace_selection_with_html(&html)
        {
            return true;
        }
        // 2. A raw bitmap (a screenshot / "copy image" with no HTML wrapper): encode
        //    it as a PNG `data:` URL and insert an image node.
        if crate::clipboard::has_image()
            && let Ok(img) = crate::clipboard::paste_image()
            && let Some(url) = image_rgba_to_png_data_url(img.width, img.height, &img.bytes)
            && handle.insert_image(&url, "")
        {
            return true;
        }
        // 3. Plain text.
        if let Ok(text) = crate::clipboard::paste_text()
            && !text.is_empty()
        {
            return handle.replace_selection_with_text(&text);
        }
        false
    }

    /// Paste the clipboard as **plain text**, dropping any rich formatting even when
    /// `text/html` is on the clipboard (the Ctrl+Shift+V "paste and match style"
    /// gesture). Returns whether the document changed.
    #[cfg(feature = "clipboard")]
    fn editor_paste_plain(&self, handle: &crate::editor::EditorHandle) -> bool {
        match crate::clipboard::paste_text() {
            Ok(text) if !text.is_empty() => handle.replace_selection_with_text(&text),
            _ => false,
        }
    }

    /// Like [`Self::editor_point_address`] but for a **physical** pointer position:
    /// the layout is in logical pixels while pointer events arrive in physical
    /// pixels, so we don't know up front whether the layout is logical or physical.
    ///
    /// Resolve in two passes so the scale hedge is sound (a wrong-space coordinate
    /// must fail rather than silently snap): first try an **exact** textblock hit
    /// at the raw point, then at the scale-divided point — a click on actual text
    /// lands here in the correct space. Only if neither lands directly on a
    /// textblock (a genuine click on chrome / padding / beside a short line) do we
    /// **snap** to the nearest block, preferring the raw point. Snapping last (not
    /// per-attempt) is what stops a HiDPI text-click from resolving to garbage in
    /// the physical space before the logical retry runs.
    pub(crate) fn editor_point_address_physical(
        &self,
        x: f32,
        y: f32,
        scale: f64,
    ) -> Option<(usize, usize, usize)> {
        let s = scale as f32;
        let scaled = (s - 1.0).abs() > f32::EPSILON;
        // Pass 1: exact textblock resolution (no snap) at raw, then scaled.
        if let Some(r) = self.editor_point_address_in(x, y, false) {
            return Some(r);
        }
        if scaled && let Some(r) = self.editor_point_address_in(x / s, y / s, false) {
            return Some(r);
        }
        // Pass 2: genuine chrome click — snap to the nearest block, raw first.
        if let Some(r) = self.editor_point_address_in(x, y, true) {
            return Some(r);
        }
        if scaled {
            return self.editor_point_address_in(x / s, y / s, true);
        }
        None
    }

    /// Resolve a window/logical point to `(container id, textblock id, flat IFC
    /// byte offset)` inside whatever editor it lands on — the shared primitive for
    /// click, drag-select, and vertical/Home-End movement. Snaps to the nearest
    /// block when the point misses every textblock (see [`Self::editor_point_address_in`]).
    pub(crate) fn editor_point_address(&self, x: f32, y: f32) -> Option<(usize, usize, usize)> {
        self.editor_point_address_in(x, y, true)
    }

    /// The shared resolver. When `allow_nearest` is false it requires the point to
    /// land directly on (or inside) a textblock; when true it falls back to the
    /// geometrically nearest block in the editor (snap-to-line).
    fn editor_point_address_in(
        &self,
        x: f32,
        y: f32,
        allow_nearest: bool,
    ) -> Option<(usize, usize, usize)> {
        let doc = self.doc.clone()?;
        let d = doc.borrow();
        let hit = hit_test(&d.tree, x, y)?;
        // Walk up for the nearest editor textblock (including empty ones, which
        // have no Parley layout) and the `data-pm-editor` container.
        let mut textblock = None;
        let mut container = None;
        let mut cur = Some(hit);
        while let Some(id) = cur {
            let node = d.tree.get(id)?;
            if textblock.is_none() && Self::is_editor_textblock(&d.tree, id) {
                textblock = Some(id);
            }
            if node.attributes.get("data-pm-editor").map(String::as_str) == Some("true") {
                container = Some(id);
                break;
            }
            cur = node.parent;
        }
        let cont = container?;
        // The point missed every textblock — a click on padding, a vertical gap, or
        // the whitespace beside a short line (e.g. right of a narrow list item).
        // Snap to the nearest block when allowed.
        let tb = match textblock {
            Some(tb) => tb,
            None if allow_nearest => Self::nearest_textblock_in(&d.tree, cont, x, y)?,
            None => return None,
        };
        let node = d.tree.get(tb)?;
        // An empty textblock has no inline content and so no Parley layout — the
        // only cursor position in it is offset 0.
        let Some(layout) = node.text_layout.as_ref() else {
            return Some((cont, tb, 0));
        };
        let (abs_x, abs_y) = Self::compute_absolute_position(&d.tree, tb);
        let pad_l = node.computed_style.padding_left.to_px();
        let pad_t = node.computed_style.padding_top.to_px();
        let rel_x = x - abs_x - pad_l + node.scroll_offset.0 as f32;
        let rel_y = y - abs_y - pad_t + node.scroll_offset.1 as f32;
        let ifc_byte =
            rinch_dom::text_query::byte_offset_from_position(&layout.layout, rel_x, rel_y);
        Some((cont, tb, ifc_byte))
    }

    /// Whether `id` is an editor textblock element — one that holds inline content
    /// (a `<p>`/`<h*>`/`<pre>`), as opposed to a block container (list / list item /
    /// blockquote / the editor root, which carry schema-node element children) or a
    /// void leaf (image / hr / hard break). Recognizes **empty** textblocks too,
    /// which have no Parley `text_layout`, by structure rather than layout.
    fn is_editor_textblock(tree: &rinch_dom::NodeTree, id: usize) -> bool {
        let Some(node) = tree.get(id) else {
            return false;
        };
        let Some(pm_type) = node.attributes.get("data-pm-type") else {
            return false; // text node or mark wrapper (`data-pm-mark`)
        };
        // The editor root and void leaves are never text-cursor targets.
        if matches!(
            pm_type.as_str(),
            "doc" | "image" | "horizontal_rule" | "hard_break"
        ) {
            return false;
        }
        // Containers hold schema-node element children; a textblock's children are
        // inline (text / mark wrappers) or it is empty.
        !node.children.iter().any(|&c| {
            tree.get(c)
                .is_some_and(|n| n.attributes.contains_key("data-pm-type"))
        })
    }

    /// The editor textblock inside `container` geometrically nearest to window
    /// point `(x, y)`. Distance prioritizes vertical proximity — the line/block
    /// whose vertical band contains `y` wins — then horizontal, so a click in the
    /// whitespace beside a line resolves to that line's textblock rather than an
    /// unrelated one above/below. Considers empty textblocks (no `text_layout`) too.
    fn nearest_textblock_in(
        tree: &rinch_dom::NodeTree,
        container: usize,
        x: f32,
        y: f32,
    ) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        let mut stack = vec![container];
        while let Some(id) = stack.pop() {
            let Some(node) = tree.get(id) else { continue };
            if Self::is_editor_textblock(tree, id) {
                let (ax, ay) = Self::compute_absolute_position(tree, id);
                let (w, h) = (node.layout.width, node.layout.height);
                let dy = (ay - y).max(0.0).max(y - (ay + h));
                let dx = (ax - x).max(0.0).max(x - (ax + w));
                // Vertical distance dominates so same-line beats nearer-but-other-line.
                let dist = dy * 1.0e4 + dx;
                if best.is_none_or(|(_, bd)| dist < bd) {
                    best = Some((id, dist));
                }
            }
            stack.extend(node.children.iter().copied());
        }
        best.map(|(id, _)| id)
    }

    /// The caret's `(x, y, height)` in window coordinates for a model `pos` — the
    /// forward geometry used to anchor vertical movement.
    pub(crate) fn editor_caret_point(
        &self,
        handle: &crate::editor::EditorHandle,
        pos: rinch_editor_core::Pos,
    ) -> Option<(f32, f32, f32)> {
        let (tb, flat) = handle.caret_address(pos)?;
        let doc = self.doc.clone()?;
        let d = doc.borrow();
        let (local_x, local_y) = d.query_caret_position(tb as u64, flat)?;
        let height = d
            .query_glyph_bounds(tb as u64, flat)
            .map(|g| g.height)
            .unwrap_or(18.0);
        let node = d.tree.get(tb)?;
        let (abs_x, abs_y) = Self::compute_absolute_position(&d.tree, tb);
        let pad_l = node.computed_style.padding_left.to_px();
        let pad_t = node.computed_style.padding_top.to_px();
        Some((abs_x + pad_l + local_x, abs_y + pad_t + local_y, height))
    }

    /// One vertical cursor step (Up / Down), as a text cursor. First tries the
    /// visual line above or below via Parley geometry; when that is **stuck** — the
    /// caret would stay on the same visual line because a block atom (a horizontal
    /// rule) or a large gap blocks the way — it steps **over** the atom to the
    /// next/previous textblock (`Selection::near_text`, which skips atoms). `None`
    /// when there is nowhere to go. Vertical movement never node-selects an atom;
    /// the horizontal arrows and clicks do that.
    fn vertical_step(
        &self,
        handle: &crate::editor::EditorHandle,
        head: rinch_editor_core::Pos,
        down: bool,
        goal_x: Option<f32>,
    ) -> Option<rinch_editor_core::Selection> {
        use rinch_editor_core::{Pos, Selection};
        let doc = handle.doc();
        if let Some((cx, cy, ch)) = self.editor_caret_point(handle, head)
            && let Some((_c, tb, ifc)) = {
                // Hit-test at the goal column (preserved across consecutive
                // Up/Down), falling back to the live caret x for the first step.
                let tx = goal_x.unwrap_or(cx);
                let ty = if down { cy + ch * 1.5 } else { cy - ch * 0.5 };
                self.editor_point_address(tx, ty)
            }
            && let Some(p) = handle.pos_at(tb, ifc)
        {
            // A move into a *different* textblock is always a real line change.
            let head_tb = handle.caret_address(head).map(|(t, _)| t);
            let p_tb = handle.caret_address(p).map(|(t, _)| t);
            if p_tb != head_tb {
                return Some(Selection::cursor(p));
            }
            // Same textblock: accept only if the caret actually advanced to a
            // different visual line (a wrapped paragraph) — otherwise the target
            // point snapped back to the current line (a block atom is in the way).
            if let Some((_, py, _)) = self.editor_caret_point(handle, p) {
                let advanced = if down {
                    py > cy + ch * 0.5
                } else {
                    py < cy - ch * 0.5
                };
                if advanced {
                    return Some(Selection::cursor(p));
                }
            }
        }
        // Stuck (or an empty target with no Parley geometry) — step over any block
        // atom(s) to the next/previous textblock. Probe from just outside the
        // current block (for a cursor in a textblock) or from `head` itself (a
        // doc-level boundary, e.g. coming from a node selection).
        let r = doc.resolve(head).ok()?;
        let probe = if r.parent().is_textblock() {
            let content_start = head.0 - r.parent_offset();
            if down {
                content_start + r.parent().content().size() + 1
            } else {
                content_start.checked_sub(1)?
            }
        } else {
            head.0
        };
        Selection::near_text(
            &doc,
            Pos(probe.min(doc.content_size())),
            if down { 1 } else { -1 },
        )
    }

    /// The model position at the start (`end = false`) or end (`end = true`) of the
    /// caret's current **visual** line — so Home/End land at the wrapped line's edge,
    /// not the whole block's. Hit-tests the far-left / far-right of the caret's line
    /// box via the same geometry as [`Self::vertical_step`]. `None` when the caret
    /// has no Parley geometry (an empty block); the caller falls back to the
    /// block-level [`line_bound`].
    fn visual_line_bound(
        &self,
        handle: &crate::editor::EditorHandle,
        head: rinch_editor_core::Pos,
        end: bool,
    ) -> Option<rinch_editor_core::Pos> {
        let (_cx, cy, ch) = self.editor_caret_point(handle, head)?;
        let (tb, _flat) = handle.caret_address(head)?;
        // Content-box left/right of the textblock, in window coords.
        let (content_left, content_right) = {
            let doc = self.doc.clone()?;
            let d = doc.borrow();
            let node = d.tree.get(tb)?;
            let (abs_x, _abs_y) = Self::compute_absolute_position(&d.tree, tb);
            let pad_l = node.computed_style.padding_left.to_px();
            let pad_r = node.computed_style.padding_right.to_px();
            (abs_x + pad_l, abs_x + node.layout.width - pad_r)
        };
        // Probe the middle of the caret's line box, just inside the far edge.
        let ty = cy + ch * 0.5;
        let tx = if end {
            content_right - 1.0
        } else {
            content_left + 1.0
        };
        let (_c, tb2, ifc) = self.editor_point_address(tx, ty)?;
        let p = handle.pos_at(tb2, ifc)?;
        // At a soft-wrap boundary the end-of-line byte is the same model position as
        // the start of the next visual line, and rinch-dom renders its caret with
        // *downstream* affinity (at the next line's start). For End, step back one
        // position when the target spilled onto the next line so the caret stays at
        // the visual end of the current line.
        if end
            && let Some((_, py, _)) = self.editor_caret_point(handle, p)
            && py > cy + ch * 0.5
        {
            return Some(rinch_editor_core::Pos(p.0.saturating_sub(1)));
        }
        Some(p)
    }

    /// The id of the `data-pm-editor` container under window/logical point
    /// `(x, y)`, if the click landed inside any editor — independent of whether it
    /// hit a textblock. Used so a click on the editor's padding / empty area still
    /// takes (or keeps) focus rather than blurring it.
    pub(crate) fn editor_container_at(&self, x: f32, y: f32) -> Option<usize> {
        let doc = self.doc.clone()?;
        let d = doc.borrow();
        let hit = hit_test(&d.tree, x, y)?;
        let mut cur = Some(hit);
        while let Some(id) = cur {
            let node = d.tree.get(id)?;
            if node.attributes.get("data-pm-editor").map(String::as_str) == Some("true") {
                return Some(id);
            }
            cur = node.parent;
        }
        None
    }

    /// Physical-coordinate variant of [`Self::editor_container_at`] (the raw-or-
    /// scaled HiDPI hedge, mirroring [`Self::editor_point_address_physical`]).
    pub(crate) fn editor_container_at_physical(&self, x: f32, y: f32, scale: f64) -> Option<usize> {
        self.editor_container_at(x, y).or_else(|| {
            let s = scale as f32;
            if (s - 1.0).abs() > f32::EPSILON {
                self.editor_container_at(x / s, y / s)
            } else {
                None
            }
        })
    }

    /// The host id of an editor **leaf** node element — an `<img>`/`<hr>` whose
    /// `data-pm-type` is `image` or `horizontal_rule` — under logical point
    /// `(x, y)`, if the click landed on one inside an editor container. Walks up
    /// from the hit node, recording the first leaf seen, and returns it only once
    /// the walk reaches the `data-pm-editor` container (so a leaf outside any editor,
    /// or a click on a non-leaf block, yields `None`). The click→node-select path.
    fn editor_leaf_at(&self, x: f32, y: f32) -> Option<usize> {
        let doc = self.doc.clone()?;
        let d = doc.borrow();
        let hit = hit_test(&d.tree, x, y)?;
        let mut leaf: Option<usize> = None;
        let mut cur = Some(hit);
        while let Some(id) = cur {
            let node = d.tree.get(id)?;
            if leaf.is_none()
                && let Some(t) = node.attributes.get("data-pm-type")
                && matches!(t.as_str(), "image" | "horizontal_rule")
            {
                leaf = Some(id);
            }
            if node.attributes.get("data-pm-editor").map(String::as_str) == Some("true") {
                return leaf; // inside an editor — return the leaf if we passed one
            }
            cur = node.parent;
        }
        None
    }

    /// Physical-coordinate variant of [`Self::editor_leaf_at`] (the raw-or-scaled
    /// HiDPI hedge, mirroring [`Self::editor_container_at_physical`]).
    fn editor_leaf_at_physical(&self, x: f32, y: f32, scale: f64) -> Option<usize> {
        self.editor_leaf_at(x, y).or_else(|| {
            let s = scale as f32;
            if (s - 1.0).abs() > f32::EPSILON {
                self.editor_leaf_at(x / s, y / s)
            } else {
                None
            }
        })
    }

    /// Focus the new editor under a pointer click at physical `(x, y)` and set the
    /// selection per the click gesture. Returns whether the click landed in an
    /// editor.
    ///
    /// A click **anywhere inside** the editor container takes/keeps focus — even on
    /// the container's padding or empty `min-height` area (otherwise it would fall
    /// through to `handle_click` and blur the editor, silently killing input). The
    /// click resolves to a model position via the nearest textblock (the clicked
    /// line, or the nearest line for a click on padding / in a gap); from there the
    /// gesture chooses the selection:
    /// - `click_count == 2` → select the **word** under the pointer;
    /// - `click_count == 3` → select the whole **textblock**;
    /// - `shift` → **extend** the existing selection from its anchor to the click;
    /// - otherwise → place a collapsed **cursor** and arm a drag-select.
    ///
    /// Only the single-click cases arm a drag (a plain click drags from the click;
    /// Shift+click drags from the existing anchor). The only click that keeps the
    /// prior selection is one that resolves to no textblock at all (an empty editor
    /// with no addressable block).
    pub(crate) fn try_new_editor_click(
        &mut self,
        x: f32,
        y: f32,
        scale: f64,
        window_size: (u32, u32),
        click_count: u8,
        shift: bool,
    ) -> bool {
        let Some(container) = self.editor_container_at_physical(x, y, scale) else {
            return false;
        };
        let Some(handle) = crate::editor::editor_for(container) else {
            return false;
        };
        // Take keyboard focus through the arbiter (tears down a prior surface /
        // input / CE / different editor).
        self.set_focus_target(FocusTarget::Editor(container));
        // A click places the cursor at a new column, so abandon any vertical goal
        // column from a prior Up/Down run.
        self.editor_goal_x = None;
        // A click on a leaf node (an image or horizontal rule) selects the node
        // itself — a `Selection::Node`, outlined by the view — rather than placing a
        // text cursor (design §6 node-views). A node-select never arms a drag.
        if let Some(leaf) = self.editor_leaf_at_physical(x, y, scale)
            && let Some(selection) = handle.node_selection_at_host(leaf)
        {
            handle.set_selection(selection);
            crate::editor::end_drag();
        } else if let Some((c, textblock, ifc_byte)) =
            self.editor_point_address_physical(x, y, scale)
            && c == container
            && let Some(clicked) = handle.pos_at(textblock, ifc_byte)
        {
            use rinch_editor_core::Selection;
            let doc = handle.doc();
            let prior_anchor = handle.selection().anchor();
            let (selection, drag_anchor) = match click_count {
                2 => {
                    let (from, to) = word_range_at(&doc, clicked);
                    (Selection::text(from, to), None)
                }
                3 => {
                    let (from, to) = block_range_at(&doc, clicked);
                    (Selection::text(from, to), None)
                }
                _ if shift => (Selection::text(prior_anchor, clicked), Some(prior_anchor)),
                _ => (Selection::cursor(clicked), Some(clicked)),
            };
            handle.set_selection(selection);
            if let Some(anchor) = drag_anchor {
                crate::editor::begin_drag(container, anchor.0);
            }
        }
        // Position the caret first, then re-layout so the post-layout caret pass
        // finalizes it against fresh geometry.
        self.refresh_editor_overlays();
        let (w, h) = (window_size.0 as f32, window_size.1 as f32);
        self.resolve_and_repaint(w, h);
        true
    }

    /// Extend an in-progress drag-select to physical pointer `(x, y)`: the
    /// selection runs from the mousedown anchor to the position under the pointer.
    /// Returns whether a drag was active and updated.
    pub(crate) fn extend_editor_drag(
        &mut self,
        x: f32,
        y: f32,
        scale: f64,
        window_size: (u32, u32),
    ) -> bool {
        let Some((container, anchor)) = crate::editor::drag_anchor() else {
            return false;
        };
        let Some((c, tb, ifc)) = self.editor_point_address_physical(x, y, scale) else {
            return true; // dragged off the text; keep the drag, don't change selection
        };
        if c != container {
            return true;
        }
        let Some(handle) = crate::editor::editor_for(container) else {
            return false;
        };
        if let Some(head) = handle.pos_at(tb, ifc) {
            use rinch_editor_core::{Pos, Selection, tables};
            let doc = handle.doc();
            // A drag that spans two different cells of one table is a cell (rectangle)
            // selection; otherwise it is an ordinary text selection.
            let anchor_pos = Pos(anchor);
            let sel = match (
                tables::cell_at_pos(&doc, anchor_pos),
                tables::cell_at_pos(&doc, head),
            ) {
                (Some(ac), Some(hc)) if ac != hc && tables::same_table(&doc, anchor_pos, head) => {
                    Selection::cell(Pos(ac), Pos(hc))
                }
                _ => Selection::text(anchor_pos, head),
            };
            handle.set_selection(sel);
            self.refresh_editor_overlays();
            let (w, h) = (window_size.0 as f32, window_size.1 as f32);
            self.resolve_and_repaint(w, h);
        }
        true
    }
}

/// The model position at the start (`end=false`) or end (`end=true`) of the
/// textblock `head` is in. (Visual-line Home/End for wrapped lines is a later
/// refinement; this is block start/end.)
#[cfg(feature = "new-editor")]
fn line_bound(
    doc: &rinch_editor_core::Node,
    head: rinch_editor_core::Pos,
    end: bool,
) -> Option<rinch_editor_core::Pos> {
    let r = doc.resolve(head).ok()?;
    if !r.parent().is_textblock() {
        return None;
    }
    let content_start = head.0 - r.parent_offset();
    Some(rinch_editor_core::Pos(if end {
        content_start + r.parent().content().size()
    } else {
        content_start
    }))
}

/// The model position at the previous/next word boundary within `head`'s
/// textblock (a simple whitespace/word-character scan; cross-block word motion is
/// a later refinement).
#[cfg(feature = "new-editor")]
fn word_boundary(
    doc: &rinch_editor_core::Node,
    head: rinch_editor_core::Pos,
    forward: bool,
) -> Option<rinch_editor_core::Pos> {
    let r = doc.resolve(head).ok()?;
    if !r.parent().is_textblock() {
        return None;
    }
    let chars: Vec<char> = block_text(r.parent()).chars().collect();
    let content_start = head.0 - r.parent_offset();
    let mut i = r.parent_offset();
    if forward {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
    } else {
        i = i.saturating_sub(1);
        while i > 0 && chars[i].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
    }
    Some(rinch_editor_core::Pos(content_start + i))
}

/// The concatenated inline text of a textblock, with each non-text leaf counted as
/// one placeholder char (so word/line offsets line up with the position space).
#[cfg(feature = "new-editor")]
fn block_text(block: &rinch_editor_core::Node) -> String {
    let mut s = String::new();
    for i in 0..block.child_count() {
        let child = block.child(i);
        match child.text() {
            Some(t) => s.push_str(t),
            None => s.push(' '),
        }
    }
    s
}

/// The model word range `(start, end)` around `pos` — the contiguous run of
/// like-classed characters (word vs. whitespace) under a double-click, classified
/// by the character to the right of the gap (or the last character at block end).
/// Falls back to a collapsed range at `pos` when it is not inside a textblock or
/// the block is empty.
#[cfg(feature = "new-editor")]
fn word_range_at(
    doc: &rinch_editor_core::Node,
    pos: rinch_editor_core::Pos,
) -> (rinch_editor_core::Pos, rinch_editor_core::Pos) {
    use rinch_editor_core::Pos;
    let Ok(r) = doc.resolve(pos) else {
        return (pos, pos);
    };
    if !r.parent().is_textblock() {
        return (pos, pos);
    }
    let chars: Vec<char> = block_text(r.parent()).chars().collect();
    if chars.is_empty() {
        return (pos, pos);
    }
    let content_start = pos.0 - r.parent_offset();
    let off = r.parent_offset().min(chars.len());
    let is_word_char = |c: char| !c.is_whitespace();
    // Classify the run to select. Normally use the char to the right of the gap, but
    // at a word's *trailing* edge (the right char is whitespace, or we're at block
    // end, while the left char is a word char) prefer the word — a double-click just
    // after a word selects the word, not the following space. A click genuinely
    // inside a whitespace run (whitespace on both sides) still selects the whitespace.
    let classify_right = off < chars.len()
        && (is_word_char(chars[off]) || off == 0 || !is_word_char(chars[off - 1]));
    let class_idx = if classify_right { off } else { off - 1 };
    let target = is_word_char(chars[class_idx]);
    let mut start = off;
    while start > 0 && is_word_char(chars[start - 1]) == target {
        start -= 1;
    }
    let mut end = off;
    while end < chars.len() && is_word_char(chars[end]) == target {
        end += 1;
    }
    (Pos(content_start + start), Pos(content_start + end))
}

/// The model content range `(start, end)` of the textblock containing `pos` — the
/// whole-block selection a triple-click makes. Falls back to a collapsed range at
/// `pos` when it is not inside a textblock.
#[cfg(feature = "new-editor")]
fn block_range_at(
    doc: &rinch_editor_core::Node,
    pos: rinch_editor_core::Pos,
) -> (rinch_editor_core::Pos, rinch_editor_core::Pos) {
    use rinch_editor_core::Pos;
    let Ok(r) = doc.resolve(pos) else {
        return (pos, pos);
    };
    if !r.parent().is_textblock() {
        return (pos, pos);
    }
    let content_start = pos.0 - r.parent_offset();
    let content_end = content_start + r.parent().content().size();
    (Pos(content_start), Pos(content_end))
}

/// Encode `width`×`height` RGBA8 pixels (the clipboard bitmap format) as a
/// `data:image/png;base64,…` URL for an image node `src`. Returns `None` if the
/// buffer isn't exactly `width * height * 4` bytes or PNG encoding fails.
///
/// `png` and `base64` are guaranteed present here: this is reached only via the
/// `new-editor` editor paste path, and `new-editor` ⇒ `desktop`/`android`, both of
/// which enable `dep:png`/`dep:base64`.
#[cfg(all(feature = "new-editor", feature = "clipboard"))]
fn image_rgba_to_png_data_url(width: usize, height: usize, rgba: &[u8]) -> Option<String> {
    use base64::Engine;
    if width == 0 || height == 0 || rgba.len() != width.checked_mul(height)?.checked_mul(4)? {
        return None;
    }
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    Some(format!("data:image/png;base64,{b64}"))
}

#[cfg(all(test, feature = "new-editor", feature = "clipboard"))]
mod paste_image_tests {
    use super::image_rgba_to_png_data_url;

    #[test]
    fn encodes_valid_rgba_as_a_png_data_url() {
        // 2×1 RGBA (red, green) = 8 bytes.
        let rgba = [255, 0, 0, 255, 0, 255, 0, 255];
        let url = image_rgba_to_png_data_url(2, 1, &rgba).expect("valid rgba encodes");
        assert!(url.starts_with("data:image/png;base64,"));
        // The payload decodes and carries the PNG magic signature.
        use base64::Engine;
        let b64 = url.strip_prefix("data:image/png;base64,").unwrap();
        let png = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("base64 decodes");
        assert_eq!(
            &png[..8],
            &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']
        );
    }

    #[test]
    fn rejects_a_mismatched_buffer() {
        // 2×2 RGBA needs 16 bytes; give it 8 → None (not a panic).
        assert!(image_rgba_to_png_data_url(2, 2, &[0; 8]).is_none());
        assert!(image_rgba_to_png_data_url(0, 0, &[]).is_none());
    }
}

#[cfg(all(test, feature = "new-editor"))]
mod editor_selection_tests {
    use super::{block_range_at, word_range_at};
    use rinch_editor_core::model::Fragment;
    use rinch_editor_core::{Node, Pos, Schema};

    fn paragraph_doc(text: &str) -> Node {
        let s = Schema::starter_kit();
        let p = s
            .branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
            .unwrap();
        s.branch("doc", Fragment::from_node(p)).unwrap()
    }

    #[test]
    fn word_range_selects_word_under_pos() {
        let doc = paragraph_doc("hello world");
        // Positions: 0[p 1 'hello world'(11) 12]13.
        assert_eq!(
            word_range_at(&doc, Pos(3)),
            (Pos(1), Pos(6)),
            "inside 'hello'"
        );
        assert_eq!(
            word_range_at(&doc, Pos(9)),
            (Pos(7), Pos(12)),
            "inside 'world'"
        );
    }

    #[test]
    fn word_range_at_block_end_selects_last_word() {
        let doc = paragraph_doc("hello world");
        // The gap at block end classifies by the last char ('d').
        assert_eq!(word_range_at(&doc, Pos(12)), (Pos(7), Pos(12)));
    }

    #[test]
    fn word_range_on_whitespace_selects_whitespace_run() {
        let doc = paragraph_doc("a  b");
        // chars a,_,_,b — a click *between* the two spaces (both neighbours
        // whitespace) selects the run of spaces.
        assert_eq!(word_range_at(&doc, Pos(3)), (Pos(2), Pos(4)));
    }

    #[test]
    fn word_range_at_word_trailing_edge_selects_the_word() {
        let doc = paragraph_doc("a  b");
        // A click in the gap just *after* 'a' (right char is whitespace, left char is
        // a word char) selects the word 'a', not the following space.
        assert_eq!(word_range_at(&doc, Pos(2)), (Pos(1), Pos(2)));
        // And the gap just before 'b' selects 'b'.
        assert_eq!(word_range_at(&doc, Pos(4)), (Pos(4), Pos(5)));
    }

    #[test]
    fn word_range_empty_block_is_collapsed() {
        let s = Schema::starter_kit();
        let doc = s
            .branch(
                "doc",
                Fragment::from_node(s.branch("paragraph", Fragment::empty()).unwrap()),
            )
            .unwrap();
        assert_eq!(word_range_at(&doc, Pos(1)), (Pos(1), Pos(1)));
    }

    #[test]
    fn block_range_selects_whole_textblock() {
        let doc = paragraph_doc("hello world");
        assert_eq!(block_range_at(&doc, Pos(3)), (Pos(1), Pos(12)));
        assert_eq!(block_range_at(&doc, Pos(8)), (Pos(1), Pos(12)));
    }

    #[test]
    fn block_range_targets_the_clicked_block() {
        let s = Schema::starter_kit();
        let p1 = s
            .branch("paragraph", Fragment::from_node(s.text("ab").unwrap()))
            .unwrap();
        let p2 = s
            .branch("paragraph", Fragment::from_node(s.text("cdef").unwrap()))
            .unwrap();
        let doc = s
            .branch("doc", Fragment::from_children(vec![p1, p2]))
            .unwrap();
        // Positions: 0[p 1 ab 3]4[p 5 cdef 9]10 — second block content is [5, 9).
        assert_eq!(block_range_at(&doc, Pos(7)), (Pos(5), Pos(9)));
    }
}
