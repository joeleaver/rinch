//! Event dispatch: routes platform events to the appropriate handler.

use super::*;

impl RinchApp {
    /// The logical (CSS-pixel) viewport `window_size` presents.
    ///
    /// `window_size` is the **physical** surface size, and pointer coordinates
    /// arrive in the same physical units. The document, however, is laid out in
    /// CSS pixels and paint multiplies every layout coordinate by the scale
    /// factor, so anything that resolves layout — every `resolve_and_repaint`
    /// below, and `ClickContext`'s viewport — must be handed *this*, never the
    /// raw surface size. Handing layout the physical size lays the page out
    /// `scale_factor` times too wide and paint then scales it up again.
    ///
    /// Shares `rinch_platform::to_logical` with the shells rather than dividing
    /// again here: mount and resize lay out at the *rounded* logical size, so a
    /// separately-rounded viewport would relayout the document to a fractionally
    /// different width on the next `ReRender`.
    pub(crate) fn layout_viewport(window_size: (u32, u32), scale_factor: f64) -> (f32, f32) {
        let (w, h) = rinch_platform::to_logical(window_size, scale_factor);
        (w as f32, h as f32)
    }

    /// Process a platform event and return a list of actions for the shell.
    ///
    /// `window_size` is in **physical** pixels (so are the pointer coordinates
    /// carried by the mouse events); the logical layout viewport is derived from
    /// it by [`Self::layout_viewport`].
    #[allow(clippy::too_many_lines)]
    pub fn handle_event(
        &mut self,
        event: PlatformEvent,
        window_size: (u32, u32),
        scale_factor: f64,
    ) -> Vec<AppAction> {
        // Mark this document as the one dispatching, for the whole of the call
        // (issue #139). Several `RinchApp`s share one thread — an app and its
        // DevTools panel, or two embedded `RinchContext`s — and process-lifetime
        // input state (the pointer-capture drag) must be able to tell whose
        // event stream it is being fed. RAII, not a set/clear pair: handlers,
        // effect flushes and layout all run under this and any of them may
        // unwind.
        let _dispatching = rinch_core::push_dispatching_doc(self.doc_key());

        let mut actions = Vec::new();
        // Logical (CSS-pixel) viewport — for ClickContext *and* for every
        // `resolve_and_repaint` below.
        let (vp_w, vp_h) = Self::layout_viewport(window_size, scale_factor);

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

                // New editor (M5): extend an in-progress drag-select. The
                // `drag_anchor` lookup lives inside `extend_editor_drag`, which
                // answers `false` when this document has no drag-select — no
                // need to run it twice on every single pointer move.
                #[cfg(feature = "desktop")]
                if self.extend_editor_drag(x, y, scale_factor, window_size) {
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
                    self.resolve_and_repaint(vp_w, vp_h);
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
                #[cfg(feature = "desktop")]
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
                if let Some(doc) = self.doc.clone() {
                    let hit = {
                        let d = doc.borrow();
                        hit_test(&d.tree, x, y)
                    };
                    // The nearest focusable ancestor-or-self of the hit, as a
                    // browser resolves a mousedown's focus target (issue #147,
                    // decision 2): any parseable `tabindex` — including `-1`,
                    // which is click-focusable but not tabbable — claims
                    // `FocusTarget::Node`, so a click-focused custom control has
                    // live Enter/Space and `on_key` immediately instead of only
                    // after being reached by Tab.
                    //
                    // The walk stops at the first node that is focusable *or*
                    // carries `data-oninput`: an `<input>` inside a focusable
                    // wrapper belongs to the text engine, and `handle_click`
                    // claims it below — taking Node focus first would announce a
                    // gain-then-loss on the wrapper for a click that was never
                    // the wrapper's.
                    let click_focus_node = {
                        let d = doc.borrow();
                        let mut cur = hit;
                        let mut found = None;
                        while let Some(nid) = cur {
                            let Some(node) = d.tree.get(nid) else { break };
                            if node.attributes.contains_key("data-oninput") {
                                break;
                            }
                            if !Self::node_is_disabled(node)
                                && node
                                    .attributes
                                    .get("tabindex")
                                    .and_then(|v| v.parse::<i32>().ok())
                                    .is_some()
                            {
                                found = Some(nid);
                                break;
                            }
                            cur = node.parent;
                        }
                        found
                    };
                    // An arbiter-held generic node (issue #228), and whether
                    // this press lands back on it — i.e. resolves to the same
                    // focusable, so a press on a plain child of the focused node
                    // is still "inside" it. A press that resolves anywhere else
                    // moves or releases the claim right here, so paths that
                    // return before `handle_click` (pending drag, scrollbar, no
                    // hit) can't strand an invisible, still-Enter-activatable
                    // claim.
                    let node_claim = if let FocusTarget::Node(fid) = self.focus_target {
                        Some((fid, click_focus_node == Some(fid)))
                    } else {
                        None
                    };
                    // Any mousedown drops the keyboard focus ring wherever it
                    // is (it only ever lives on the focused node): pointer
                    // interaction never shows :focus-visible, and `update_focus`
                    // below is a no-op when the hit node already holds
                    // (Tab-driven) focus.
                    {
                        let mut d = doc.borrow_mut();
                        if let Some(prev) = d.tree.focused_node {
                            d.set_focus_visible(prev, false);
                        }
                        // :active applies while mouse is pressed
                        d.update_active(hit);
                        // :focus applies to the clicked element (persists after
                        // release); anchored on the focusable ancestor for a
                        // press inside one.
                        d.update_focus(click_focus_node.or(hit));
                    }
                    // No outstanding doc borrow from here on: the arbiter's
                    // teardown re-borrows, and the callbacks it defers are user
                    // code that may mutate the DOM.
                    match (click_focus_node, node_claim) {
                        // Re-press inside the already-focused node: nothing to
                        // do, the claim and its state stay put.
                        (_, Some((_, true))) => {}
                        (Some(nid), _) => {
                            let (_, work) = self.set_focus_target_deferred(FocusTarget::Node(nid));
                            Self::fire_focus_work(work);
                            self.notify_node_focus_gained(nid);
                        }
                        (None, Some((_, false))) => {
                            self.set_focus_target(FocusTarget::None);
                        }
                        (None, None) => {}
                    }
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
                #[cfg(feature = "desktop")]
                crate::editor::end_drag(self.input_doc());

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
            PlatformEvent::WindowFocus(focused) => {
                // Notify-and-retain (issue #147, decision 1): the in-document
                // claim survives an alt-tab — releasing it would fire
                // `data-onchange` on every window switch, a straight #226
                // regression — but a registered target is told, and told again
                // when the window comes back, so it can hide its caret and idle
                // its blink timer. `ime_state()` reports disabled meanwhile.
                if self.window_focused != focused {
                    self.window_focused = focused;
                    if let FocusTarget::Node(id) = self.focus_target {
                        let doc_key = self.doc_key();
                        if focused {
                            crate::focus_registry::notify_focus_gained(doc_key, id);
                        } else {
                            crate::focus_registry::notify_focus_lost(doc_key, id);
                        }
                    }
                    self.scene_dirty = true;
                    actions.push(AppAction::RequestRedraw);
                }
            }
            PlatformEvent::KeyDown {
                key,
                logical_key,
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
                let key_str: Option<String> = hook_key_str(key, text.as_deref(), ctrl);

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
                    #[cfg(feature = "desktop")]
                    FocusTarget::Editor(container) => {
                        if let Some(handle) =
                            crate::editor::editor_for_doc(self.doc_key(), container)
                        {
                            self.dispatch_new_editor_key(
                                &handle,
                                key,
                                logical_key,
                                text.as_deref(),
                                shift,
                                ctrl,
                                alt,
                            );
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
                    // A native <select> popup owns the keyboard while open:
                    // navigate / commit / dismiss / type-ahead.
                    FocusTarget::Select(_) => {
                        if self.handle_select_key(key, text.as_deref(), vp_w, vp_h) {
                            actions.push(AppAction::RequestRedraw);
                            return actions;
                        }
                    }
                    // No widget owns the key, a plain `<input>` does (its editing
                    // commands live in the global handlers, gated internally on
                    // `focused_input_state`), or a generic focusable node does
                    // (Enter/Space activate it below; everything else falls
                    // through so Tab keeps moving). Falls through to DevTools /
                    // inspect / Tab navigation / read-only text-selection caret
                    // motion.
                    FocusTarget::Input(_) | FocusTarget::None | FocusTarget::Node(_) => {
                        // Self-heal a stale Node claim before routing, exactly
                        // like the Editor arm's unmount handling above: node ids
                        // are recycled slab indices (see
                        // `live_focused_input_handler`), so a claim whose node
                        // was unmounted must not swallow Enter/Space, anchor
                        // Tab, or — worst — activate whatever unrelated node
                        // reused the slot.
                        if let FocusTarget::Node(id) = self.focus_target
                            && !self.node_target_is_live(id)
                        {
                            self.set_focus_target(FocusTarget::None);
                        }

                        // A registered custom widget (issue #147) gets first
                        // refusal on its own keys, ahead of every global
                        // handler below — that is what "owns the keyboard"
                        // means. `true` consumes; `false` falls through to
                        // DevTools / inspect / Tab / Enter-Space activation
                        // exactly as before, so registering costs an
                        // unregistered node nothing.
                        //
                        // The `key` string matches the document-level
                        // interceptor's spelling (`hook_key_str`), falling back
                        // to the physical code for keys it has no name for
                        // (function keys), so a widget always sees a non-empty
                        // key.
                        if let FocusTarget::Node(id) = self.focus_target {
                            let key_data = events::KeyEventData {
                                key: key_str.clone().unwrap_or_else(|| format!("{:?}", key)),
                                code: format!("{:?}", key),
                                ctrl,
                                shift,
                                alt,
                                meta: modifiers.meta,
                            };
                            if crate::focus_registry::offer_key(self.doc_key(), id, &key_data) {
                                actions.push(AppAction::RequestRedraw);
                                return actions;
                            }
                        }

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
                            KeyCode::Enter | KeyCode::Space
                                if !ctrl && matches!(self.focus_target, FocusTarget::Node(_)) =>
                            {
                                if let FocusTarget::Node(id) = self.focus_target {
                                    // One activation per physical press: the OS
                                    // auto-repeats KeyDown and `PlatformEvent`
                                    // carries no repeat flag, so latch until the
                                    // matching KeyUp (on the web a held Space
                                    // activates exactly once, on keyup).
                                    if self.node_activation_held != Some(key) {
                                        self.node_activation_held = Some(key);
                                        self.activate_focused_node(id, vp_w, vp_h);
                                        actions.push(AppAction::RequestRedraw);
                                    }
                                }
                            }
                            KeyCode::Enter if !ctrl => self.handle_enter(shift),
                            // Space with no Node target falls through to the `_`
                            // arm below — the one text-input path (pre-#228), so
                            // a future change to that gate can't miss Space.
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
                // Release the Enter/Space activation latch (issue #228): the
                // next KeyDown of this key is a fresh physical press.
                if self.node_activation_held == Some(key) {
                    self.node_activation_held = None;
                }
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
                    #[cfg(feature = "desktop")]
                    FocusTarget::Editor(container) => {
                        if let Some(handle) =
                            crate::editor::editor_for_doc(self.doc_key(), container)
                        {
                            self.dispatch_editor_ime(&handle, ime);
                            self.refresh_editor_overlays();
                            self.resolve_and_repaint(vp_w, vp_h);
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
                    // Surfaces and no focus do not consume IME.
                    _ => {}
                }
            }
            PlatformEvent::ScaleFactorChanged(_) => {
                // The shell handles reconfiguring the renderer; we just need a redraw.
                actions.push(AppAction::RequestRedraw);
            }
            PlatformEvent::UserEvent(UserEvent::ReRender) => {
                if self.resolve_and_repaint(vp_w, vp_h) {
                    actions.push(AppAction::RequestRedraw);
                }
                // Process any pending input focus request (e.g., from an Effect
                // triggered by run_on_main_thread that called request_focus).
                if let Some(focus_node_id) = rinch_core::take_pending_focus_request(self.doc_key())
                {
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
                    if self.resolve_and_repaint(vp_w, vp_h) {
                        actions.push(AppAction::RequestRedraw);
                    }
                }

                // Process any pending input focus request from effects
                if let Some(focus_node_id) = rinch_core::take_pending_focus_request(self.doc_key())
                {
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
                    if self.resolve_and_repaint(vp_w, vp_h) {
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
                    // A stale attribute must not stop the walk: disposing a
                    // scope frees its handlers (issue #141) while nothing strips
                    // the attribute from a node that outlives them, and this
                    // walk commits to the first node carrying one.
                    if let Some(id) = node
                        .attributes
                        .get("data-onenter")
                        .and_then(|v| v.parse::<usize>().ok())
                        .filter(|&id| events::has_click_handler(events::EventHandlerId(id)))
                    {
                        found = Some(id);
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
                    // A stale attribute must not stop the walk: disposing a
                    // scope frees its handlers (issue #141) while nothing strips
                    // the attribute from a node that outlives them, and this
                    // walk commits to the first node carrying one.
                    if let Some(id) = node
                        .attributes
                        .get("data-onleave")
                        .and_then(|v| v.parse::<usize>().ok())
                        .filter(|&id| events::has_click_handler(events::EventHandlerId(id)))
                    {
                        found = Some(id);
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
                    // A stale attribute must not stop the walk — see the note
                    // on the `data-onenter` walk (issue #141).
                    if let Some(val) = node.attributes.get("data-oncontextmenu") {
                        if let Some(id) = val
                            .parse::<usize>()
                            .ok()
                            .filter(|&id| events::has_click_handler(events::EventHandlerId(id)))
                        {
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
                    // A stale attribute must not stop the walk — see the note
                    // on the `data-onenter` walk (issue #141).
                    if let Some(val) = node.attributes.get(attr) {
                        if let Some(id) = val
                            .parse::<usize>()
                            .ok()
                            .filter(|&id| events::has_click_handler(events::EventHandlerId(id)))
                        {
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
                    // A stale attribute must not stop the walk — see the note
                    // on the `data-onenter` walk (issue #141).
                    if let Some(id) = node
                        .attributes
                        .get(attr)
                        .and_then(|v| v.parse::<usize>().ok())
                        .filter(|&id| events::has_click_handler(events::EventHandlerId(id)))
                    {
                        found = Some(id);
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
                    // A stale attribute must not stop the walk — see the note
                    // on the `data-onenter` walk (issue #141).
                    if let Some(val) = node.attributes.get(attr) {
                        if let Some(id) = val
                            .parse::<usize>()
                            .ok()
                            .filter(|&id| events::has_click_handler(events::EventHandlerId(id)))
                        {
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
                    // A stale attribute must not stop the walk: disposing a
                    // scope frees its handlers (issue #141) while nothing strips
                    // the attribute from a node that outlives them, and this
                    // walk commits to the first node carrying one.
                    if let Some(id) = node
                        .attributes
                        .get("data-onfiledrop")
                        .and_then(|v| v.parse::<usize>().ok())
                        .filter(|&id| events::has_file_drop_handler(events::EventHandlerId(id)))
                    {
                        found = Some(id);
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

        // Computed up-front, once: both snapshot blocks below may be compiled
        // in the same build (desktop software + embed carries both painters).
        let anchor = {
            let d = doc.borrow();
            let (abs_x, abs_y) =
                rinch_dom::paint::compute_absolute_position(&d.tree, node_id, scale_factor);
            (
                mousedown_pos.0 - abs_x as f32,
                mousedown_pos.1 - abs_y as f32,
            )
        };

        #[cfg(any(feature = "gpu", feature = "android-gpu", feature = "embed"))]
        let snapshot = {
            let mut painter = VelloPainter::new();
            let mut d = doc.borrow_mut();
            let d = &mut *d;

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

        #[cfg(software_shell)]
        let (snapshot_pixels, snapshot_width, snapshot_height) = {
            let mut d = doc.borrow_mut();
            let d = &mut *d;

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
            #[cfg(any(feature = "gpu", feature = "android-gpu", feature = "embed"))]
            snapshot,
            #[cfg(software_shell)]
            snapshot_pixels,
            #[cfg(software_shell)]
            snapshot_width,
            #[cfg(software_shell)]
            snapshot_height,
            anchor,
            cursor,
            over_target: None,
        });
    }
}

/// A cursor motion produced by an arrow/Home/End key (design §7 keyboard).
#[cfg(feature = "desktop")]
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

/// Derive the key string handed to the user keyboard hook (and global
/// fallback) from a `KeyDown`'s keycode + text. Named keys report their name
/// (`"Space"`, `"Enter"`, …); Ctrl+letter combos report the letter; everything
/// else — including `KeyCode::Other`, which is how both real hardware and the
/// debug channel deliver punctuation — falls through to the event's `text`
/// field, so a hook sees `key: "."` for a period but `key: "Space"` for a
/// spacebar press.
pub(crate) fn hook_key_str(key: KeyCode, text: Option<&str>, ctrl: bool) -> Option<String> {
    match key {
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
        _ => text.and_then(|t| {
            if !t.is_empty() && t.chars().all(|c| !c.is_control()) {
                Some(t.to_string())
            } else {
                None
            }
        }),
    }
}

/// Translate a platform key event into an editor-core `KeyBinding` for keymap lookup.
///
/// **Letters resolve LOGICALLY** (`logical_key`, the layout-mapped letter from winit) so
/// `Mod-b` is correct on every layout — on Dvorak/AZERTY the key labelled B fires bold,
/// not the physical QWERTY-B position. **Digits, symbols, and named keys resolve
/// PHYSICALLY** (`KeyCode`), because the shifted glyph is layout-dependent — `Mod-Shift-8`
/// must match the `8` key regardless of what Shift+8 types. This mirrors the web view
/// (logical `event.key()` for letters, physical `event.code()` otherwise). Returns `None`
/// for keys with no bindable identity, which then fall through to text input.
#[cfg(feature = "desktop")]
fn editor_key_binding(
    key: KeyCode,
    logical_key: Option<char>,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> Option<rinch_editor_core::KeyBinding> {
    use rinch_editor_core::{Key, KeyBinding, Modifiers};
    // A layout-mapped ASCII letter wins over the physical position.
    if let Some(c) = logical_key.filter(|c| c.is_ascii_alphabetic()) {
        return Some(KeyBinding::new(
            Key::Char(c.to_ascii_lowercase()),
            Modifiers {
                primary: ctrl,
                shift,
                alt,
            },
        ));
    }
    let k = match key {
        KeyCode::KeyA => Key::Char('a'),
        KeyCode::KeyB => Key::Char('b'),
        KeyCode::KeyC => Key::Char('c'),
        KeyCode::KeyD => Key::Char('d'),
        KeyCode::KeyE => Key::Char('e'),
        KeyCode::KeyF => Key::Char('f'),
        KeyCode::KeyG => Key::Char('g'),
        KeyCode::KeyH => Key::Char('h'),
        KeyCode::KeyI => Key::Char('i'),
        KeyCode::KeyJ => Key::Char('j'),
        KeyCode::KeyK => Key::Char('k'),
        KeyCode::KeyL => Key::Char('l'),
        KeyCode::KeyM => Key::Char('m'),
        KeyCode::KeyN => Key::Char('n'),
        KeyCode::KeyO => Key::Char('o'),
        KeyCode::KeyP => Key::Char('p'),
        KeyCode::KeyQ => Key::Char('q'),
        KeyCode::KeyR => Key::Char('r'),
        KeyCode::KeyS => Key::Char('s'),
        KeyCode::KeyT => Key::Char('t'),
        KeyCode::KeyU => Key::Char('u'),
        KeyCode::KeyV => Key::Char('v'),
        KeyCode::KeyW => Key::Char('w'),
        KeyCode::KeyX => Key::Char('x'),
        KeyCode::KeyY => Key::Char('y'),
        KeyCode::KeyZ => Key::Char('z'),
        KeyCode::Digit0 => Key::Char('0'),
        KeyCode::Digit1 => Key::Char('1'),
        KeyCode::Digit2 => Key::Char('2'),
        KeyCode::Digit3 => Key::Char('3'),
        KeyCode::Digit4 => Key::Char('4'),
        KeyCode::Digit5 => Key::Char('5'),
        KeyCode::Digit6 => Key::Char('6'),
        KeyCode::Digit7 => Key::Char('7'),
        KeyCode::Digit8 => Key::Char('8'),
        KeyCode::Digit9 => Key::Char('9'),
        KeyCode::Minus => Key::Char('-'),
        KeyCode::Equal => Key::Char('='),
        KeyCode::Enter => Key::Enter,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Tab => Key::Tab,
        KeyCode::Escape => Key::Escape,
        KeyCode::Space => Key::Space,
        KeyCode::ArrowLeft => Key::ArrowLeft,
        KeyCode::ArrowRight => Key::ArrowRight,
        KeyCode::ArrowUp => Key::ArrowUp,
        KeyCode::ArrowDown => Key::ArrowDown,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        _ => return None,
    };
    Some(KeyBinding::new(
        k,
        Modifiers {
            primary: ctrl,
            shift,
            alt,
        },
    ))
}

/// Rich-text editor keyboard handling (desktop).
#[cfg(feature = "desktop")]
impl RinchApp {
    /// Translate a key press into an action on the focused editor's
    /// [`EditorHandle`](crate::editor::EditorHandle). Returns whether the document
    /// or selection changed (and a repaint/caret refresh is needed).
    // A key event's physical key, logical letter, insertable text, and three modifier
    // flags are all distinct inputs — passing them individually is clearer here than a
    // one-off struct.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch_new_editor_key(
        &mut self,
        handle: &crate::editor::EditorHandle,
        key: KeyCode,
        logical_key: Option<char>,
        text: Option<&str>,
        shift: bool,
        ctrl: bool,
        alt: bool,
    ) -> bool {
        // The vertical "goal column" survives only a run of Up/Down — any other key
        // (typing, horizontal arrows, Home/End, edits) abandons it.
        if !matches!(key, KeyCode::ArrowUp | KeyCode::ArrowDown) {
            self.editor_goal_x = None;
        }
        // 1. Cursor movement / selection extension — geometry-dependent (visual lines,
        //    goal column), so it stays view-owned and never touches the keymap. (Shift
        //    extends, Ctrl = word/doc.)
        match key {
            KeyCode::ArrowLeft => {
                return self.move_editor(
                    handle,
                    if ctrl {
                        Motion::WordLeft
                    } else {
                        Motion::CharLeft
                    },
                    shift,
                );
            }
            KeyCode::ArrowRight => {
                return self.move_editor(
                    handle,
                    if ctrl {
                        Motion::WordRight
                    } else {
                        Motion::CharRight
                    },
                    shift,
                );
            }
            KeyCode::ArrowUp => return self.move_editor(handle, Motion::LineUp, shift),
            KeyCode::ArrowDown => return self.move_editor(handle, Motion::LineDown, shift),
            KeyCode::Home => {
                return self.move_editor(
                    handle,
                    if ctrl {
                        Motion::DocStart
                    } else {
                        Motion::LineStart
                    },
                    shift,
                );
            }
            KeyCode::End => {
                return self.move_editor(
                    handle,
                    if ctrl {
                        Motion::DocEnd
                    } else {
                        Motion::LineEnd
                    },
                    shift,
                );
            }
            _ => {}
        }
        // 2. Tab: navigate table cells when in a table (shared handle method); otherwise
        //    fall through to the keymap, which binds `Tab`→sinkListItem /
        //    `Shift-Tab`→liftListItem. Consumed either way — no editor focus traversal.
        if matches!(key, KeyCode::Tab) && handle.tab_cell(shift) {
            return true;
        }
        // 3. Clipboard (Ctrl+C/X/V, Ctrl+Shift+V) — needs the platform clipboard, so it
        //    can't be an editor-core command. Runs before the keymap, which never binds
        //    these keys. (Ctrl+Shift+V — paste-and-match-style — precedes plain Ctrl+V.)
        #[cfg(feature = "clipboard")]
        if ctrl {
            match key {
                KeyCode::KeyC if !shift => {
                    self.editor_copy(handle);
                    return false; // copy changes neither the document nor the selection
                }
                KeyCode::KeyX if !shift => return self.editor_cut(handle),
                KeyCode::KeyV if shift => return self.editor_paste_plain(handle),
                KeyCode::KeyV => return self.editor_paste(handle),
                _ => {}
            }
        }
        // 4. THE KEYMAP — the single source of truth for every command key (marks, block
        //    types, lists, blockquote, headings, hr / hard break, enter, backspace /
        //    delete, undo / redo, select-all). A matched binding is always consumed, even
        //    if the command no-op'd at this position (so e.g. Tab at top level never
        //    falls through to text insertion / focus traversal).
        if let Some(binding) = editor_key_binding(key, logical_key, ctrl, shift, alt)
            && handle.dispatch_key(binding).is_some()
        {
            return true;
        }
        // 5. Plain text insertion (never with the primary modifier) — the resolved char,
        //    which runs the markdown input rules.
        if !ctrl
            && let Some(t) = text
            && !t.is_empty()
            && t.chars().all(|c| !c.is_control())
        {
            return handle.insert_text(t);
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
        use rinch_editor_core::{CursorMotion, Pos, Selection, motion as core_motion};
        // Horizontal / word / document motions resolve entirely in the shared
        // editor-core model path (identical on desktop and web — see
        // `EditorHandle::move_cursor`). Only visual-line edges and vertical motion
        // need the renderer's laid-out geometry, handled below.
        let model_motion = match motion {
            Motion::CharLeft => Some(CursorMotion::CharLeft),
            Motion::CharRight => Some(CursorMotion::CharRight),
            Motion::WordLeft => Some(CursorMotion::WordLeft),
            Motion::WordRight => Some(CursorMotion::WordRight),
            Motion::DocStart => Some(CursorMotion::DocStart),
            Motion::DocEnd => Some(CursorMotion::DocEnd),
            Motion::LineStart | Motion::LineEnd | Motion::LineUp | Motion::LineDown => None,
        };
        if let Some(cm) = model_motion {
            return handle.move_cursor(cm, extend);
        }
        let state = handle.state();
        let doc = state.doc.clone();
        let head = state.selection.head();
        let new_head: Option<Pos> = match motion {
            // Visual-line edge for wrapped paragraphs (geometry), falling back to
            // the block edge when the caret has no Parley layout (an empty block).
            Motion::LineStart => self
                .visual_line_bound(handle, head, false)
                .or_else(|| core_motion::line_boundary(&doc, head, false)),
            Motion::LineEnd => self
                .visual_line_bound(handle, head, true)
                .or_else(|| core_motion::line_boundary(&doc, head, true)),
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
            // The model motions returned early via `handle.move_cursor` above.
            _ => None,
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
    #[cfg(feature = "desktop")]
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
/// Ungated: `<input>` IME works on every build with an app loop. (The editor IME
/// half above is `desktop`-gated because it needs the [`EditorHandle`](crate::editor::EditorHandle).)
impl RinchApp {
    /// Apply an [`ImeEvent`] to the focused `<input>`: preedit is rendered inline
    /// at the caret via the `data-preedit` attribute, commit clears it and inserts
    /// the text, delete-surrounding maps to backward/forward deletes.
    ///
    /// `pub(super)` so the focus arbiter can flush a pending composition as an
    /// implicit commit when the input blurs (issue #226) — the browser's
    /// compositionend-before-blur.
    pub(super) fn dispatch_input_ime(&mut self, node_id: usize, ime: ImeEvent) {
        match ime {
            ImeEvent::Enabled => {}
            ImeEvent::Preedit { text, cursor } => {
                let ended = text.is_empty();
                self.focused_input_preedit = if ended { None } else { Some((text, cursor)) };
                self.sync_input_preedit_to_dom(node_id);
                if ended {
                    // An empty preedit *is* the end of the composition on the
                    // winit backends (an IME cancel delivers only this — no
                    // Commit, no Disabled). Drain a write the composition
                    // deferred, exactly like the Commit/Disabled arms below;
                    // otherwise it stays parked and later wins over whatever
                    // was written in the meantime (issue #238).
                    self.adopt_focused_input_value_from_dom();
                }
            }
            ImeEvent::Commit(text) => {
                self.focused_input_preedit = None;
                self.sync_input_preedit_to_dom(node_id);
                // A value write deferred by the composition applies first, so
                // the committed text is inserted into it (issue #238).
                self.adopt_focused_input_value_from_dom();
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
                // The composition is over either way: apply a deferred write.
                self.adopt_focused_input_value_from_dom();
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

#[cfg(feature = "desktop")]
impl RinchApp {
    /// This app's document identity for scoping shared input state, or `None`
    /// before mount (issue #139).
    ///
    /// `doc_key()` answers `0` until `self.doc` is assigned while `next_doc_key`
    /// starts at `1`, so `Some(0)` would read as a real — and *shared* —
    /// document identity. Two pre-mount apps would then look like the same one.
    /// The sentinel rule is [`rinch_core::doc_identity`]'s, not a second copy of
    /// it: `push_dispatching_doc` applies the very same one to the very same key.
    fn input_doc(&self) -> Option<u64> {
        rinch_core::doc_identity(self.doc_key())
    }

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

    /// Whether a physical click at `(x, y)` landed in a **task item's checkbox
    /// gutter** — the strip left of the item's content where the `::before` checkbox
    /// renders. A task item is the only block with an interactive marker (bullets and
    /// numbers are inert), so this gates the checkbox-toggle click path. Tries raw
    /// then scaled coordinates, matching the other physical click helpers.
    fn editor_task_checkbox_at_physical(&self, x: f32, y: f32, scale: f64) -> bool {
        let s = scale as f32;
        let scaled = (s - 1.0).abs() > f32::EPSILON;
        self.editor_task_checkbox_at(x, y) || (scaled && self.editor_task_checkbox_at(x / s, y / s))
    }

    /// The logical-coordinate core of [`Self::editor_task_checkbox_at_physical`].
    fn editor_task_checkbox_at(&self, x: f32, y: f32) -> bool {
        let Some(doc) = self.doc.clone() else {
            return false;
        };
        let d = doc.borrow();
        let Some(hit) = hit_test(&d.tree, x, y) else {
            return false;
        };
        // Walk up to the nearest task_item element (stopping at the editor container).
        let mut task_item = None;
        let mut cur = Some(hit);
        while let Some(id) = cur {
            let Some(node) = d.tree.get(id) else {
                return false;
            };
            if task_item.is_none()
                && node.attributes.get("data-pm-type").map(String::as_str) == Some("task_item")
            {
                task_item = Some(id);
            }
            if node.attributes.get("data-pm-editor").map(String::as_str) == Some("true") {
                break;
            }
            cur = node.parent;
        }
        let Some(item) = task_item else {
            return false;
        };
        // The item's first block child is its content (e.g. the paragraph); the
        // checkbox gutter is everything LEFT of that child's content box. A click there
        // toggles the box; a click on/after the text is a normal caret placement.
        let Some(item_node) = d.tree.get(item) else {
            return false;
        };
        let content = item_node.children.iter().copied().find(|&c| {
            d.tree
                .get(c)
                .is_some_and(|n| n.attributes.contains_key("data-pm-type"))
        });
        let Some(content) = content else {
            return false;
        };
        let (content_x, _) = Self::compute_absolute_position(&d.tree, content);
        let pad_l = d
            .tree
            .get(content)
            .map(|n| n.computed_style.padding_left.to_px())
            .unwrap_or(0.0);
        x < content_x + pad_l
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
    /// block-level `rinch_editor_core::motion::line_boundary`.
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
        let Some(handle) = crate::editor::editor_for_doc(self.doc_key(), container) else {
            return false;
        };
        // Take keyboard focus through the arbiter (tears down a prior surface /
        // input / CE / different editor).
        self.set_focus_target(FocusTarget::Editor(container));
        // A click places the cursor at a new column, so abandon any vertical goal
        // column from a prior Up/Down run.
        self.editor_goal_x = None;
        // A click in a task item's checkbox gutter toggles its `checked` state instead
        // of placing a caret (the checkbox is a CSS `::before`, so the hit is geometric,
        // not a real element). Resolve the nearest textblock for a document position,
        // then toggle the enclosing task item.
        if self.editor_task_checkbox_at_physical(x, y, scale)
            && let Some((c, tb, ifc)) = self.editor_point_address_physical(x, y, scale)
            && c == container
            && let Some(pos) = handle.pos_at(tb, ifc)
            && handle.toggle_task_checked_at(pos.0)
        {
            crate::editor::end_drag(self.input_doc());
            self.refresh_editor_overlays();
            let (w, h) = Self::layout_viewport(window_size, scale);
            self.resolve_and_repaint(w, h);
            return true;
        }
        // A click on a leaf node (an image or horizontal rule) selects the node
        // itself — a `Selection::Node`, outlined by the view — rather than placing a
        // text cursor (design §6 node-views). A node-select never arms a drag.
        if let Some(leaf) = self.editor_leaf_at_physical(x, y, scale)
            && let Some(selection) = handle.node_selection_at_host(leaf)
        {
            handle.set_selection(selection);
            crate::editor::end_drag(self.input_doc());
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
                    let (from, to) = rinch_editor_core::word_range_at(&doc, clicked);
                    (Selection::text(from, to), None)
                }
                3 => {
                    let (from, to) = rinch_editor_core::block_range_at(&doc, clicked);
                    (Selection::text(from, to), None)
                }
                _ if shift => (Selection::text(prior_anchor, clicked), Some(prior_anchor)),
                _ => (Selection::cursor(clicked), Some(clicked)),
            };
            handle.set_selection(selection);
            if let Some(anchor) = drag_anchor {
                crate::editor::begin_drag(self.input_doc(), container, anchor.0);
            }
        }
        // Position the caret first, then re-layout so the post-layout caret pass
        // finalizes it against fresh geometry.
        self.refresh_editor_overlays();
        let (w, h) = Self::layout_viewport(window_size, scale);
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
        let Some((container, anchor)) = crate::editor::drag_anchor(self.input_doc()) else {
            return false;
        };
        let Some((c, tb, ifc)) = self.editor_point_address_physical(x, y, scale) else {
            return true; // dragged off the text; keep the drag, don't change selection
        };
        if c != container {
            return true;
        }
        let Some(handle) = crate::editor::editor_for_doc(self.doc_key(), container) else {
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
            let (w, h) = Self::layout_viewport(window_size, scale);
            self.resolve_and_repaint(w, h);
        }
        true
    }
}

/// Encode `width`×`height` RGBA8 pixels (the clipboard bitmap format) as a
/// `data:image/png;base64,…` URL for an image node `src`. Returns `None` if the
/// buffer isn't exactly `width * height * 4` bytes or PNG encoding fails.
///
/// `png` and `base64` are guaranteed present here: this is reached only via the
/// editor paste path, gated on `desktop`, which enables `dep:png`/`dep:base64`.
#[cfg(all(feature = "desktop", feature = "clipboard"))]
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

#[cfg(all(test, feature = "desktop", feature = "clipboard"))]
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

#[cfg(test)]
mod hook_key_str_tests {
    use super::hook_key_str;
    use rinch_platform::KeyCode;

    #[test]
    fn other_keycode_falls_through_to_the_text_field() {
        // Punctuation — hardware and debug channel alike — arrives as
        // `KeyCode::Other` with the character in `text`; the hook must see the
        // character, not a named key (issue #151).
        assert_eq!(
            hook_key_str(KeyCode::Other, Some("."), false),
            Some(".".to_string())
        );
    }

    #[test]
    fn spacebar_reports_the_named_key_not_its_text() {
        // A real (or injected) spacebar press is `KeyCode::Space` with
        // text=" " — the named-key arm must win so hooks see "Space".
        assert_eq!(
            hook_key_str(KeyCode::Space, Some(" "), false),
            Some("Space".to_string())
        );
    }
}

#[cfg(all(test, feature = "desktop"))]
mod editor_key_binding_tests {
    use super::editor_key_binding;
    use rinch_editor_core::{Key, KeyBinding, Modifiers};
    use rinch_platform::KeyCode;

    #[test]
    fn logical_letter_wins_over_physical_position() {
        // Dvorak: the key that types 'b' sits at the physical QWERTY-N position, so
        // `key`=KeyN but `logical`=Some('b'). The logical letter must win → Mod-b.
        let b = editor_key_binding(KeyCode::KeyN, Some('b'), true, false, false).unwrap();
        assert_eq!(
            b,
            KeyBinding::new(
                Key::Char('b'),
                Modifiers {
                    primary: true,
                    shift: false,
                    alt: false
                }
            )
        );
    }

    #[test]
    fn falls_back_to_the_physical_key_without_a_logical_letter() {
        // No logical letter (e.g. an injected/synthetic key) → the physical KeyCode maps.
        let b = editor_key_binding(KeyCode::KeyB, None, true, false, false).unwrap();
        assert_eq!(b.key, Key::Char('b'));
    }

    #[test]
    fn digits_use_the_physical_key_ignoring_a_non_letter_logical() {
        // Shift+8 has a logical '*' (not a letter) → fall back to the physical Digit8='8'
        // so `Mod-Shift-8` matches regardless of the shifted glyph.
        let b = editor_key_binding(KeyCode::Digit8, Some('*'), true, true, false).unwrap();
        assert_eq!(b.key, Key::Char('8'));
        assert!(b.mods.primary && b.mods.shift);
    }
}
