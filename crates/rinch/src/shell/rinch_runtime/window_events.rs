//! One window's winit `WindowEvent`, turned into what the runtime and its
//! app do with it.

use super::*;

impl RinchRuntime {
    /// One window's event, for whichever of the runtime's windows it is: the
    /// main window and the DevTools window take the same path. What only the
    /// main window has is gated on [`WindowRole::Main`]: the pointer lock and
    /// raw motion (#2092), the published geometry (#1546), DevTools' inspect
    /// mode, and the native events the RedrawRequested arm drains.
    pub(super) fn window_event_for(
        &mut self,
        role: WindowRole,
        event_loop: &dyn ActiveEventLoop,
        event: WindowEvent,
    ) {
        // Whatever pointer this event names (`set_current_pointer` below) is
        // forgotten when it has been handled, on every return path, so an
        // event that names none (a key, an injected debug `mouse_down`) is
        // never read as the last pen or finger.
        let _forget_pointer = ForgetCurrentPointer;
        // The OS window is gone already (#2489): nothing else is asked of it.
        if matches!(event, WindowEvent::Destroyed) {
            if self.window_destroyed(role) {
                event_loop.exit();
            }
            return;
        }
        // Keys reaching any window of a menu chain go to its topmost menu
        // (#2552).
        let role = self.input_role(role, &event);
        // Raw motion that came before this event reaches the app first.
        self.flush_pointer_motion(event_loop);

        // Pointer moves are coalesced (update-path audit F2.2): a high-rate
        // mouse reports several per frame, and each used to run hover, the
        // `data-onmousemove` walk and — during a component drag — a whole
        // layout. Only the newest position is kept; it reaches the app before
        // anything else does (so a press is still judged at the position the
        // pointer last reported) and at the end of the batch, in
        // `about_to_wait`. That is the browser's model: one `pointermove` per
        // frame at most.
        if let WindowEvent::PointerMoved {
            position,
            primary,
            source,
            ..
        } = &event
        {
            // winit reports the pointer in **physical** pixels; every
            // `PlatformEvent` coordinate is logical (#299). This is the
            // conversion for the whole pointer stream: `MouseDown`, `MouseUp`
            // and `MouseWheel` below all read the position back out of
            // `app.cursor_pos`, which the flushed move sets.
            let (lx, ly) = to_logical_point((position.x, position.y), self.w(role).scale_factor());
            // While the pointer is locked or confined (#2092) its position is
            // held where the lock found it, as a browser holds it: a confined
            // cursor (X11) still moves, and must not move hover under a
            // camera the mouse is turning. The motion arrives as raw deltas.
            // Where it really is is kept for when the lock ends
            // (`restore_held_pointer_move`), so hover is not left stale.
            if crate::pointer_lock::is_locked_in(self.w(role).facts_key) {
                let id = pointer_id_of(source);
                let down = self.w(role).pressed_pointers.is_down(id);
                let pointer = surface_pointer_of(source, *primary, down);
                self.w(role).held_pointer_move = Some((lx as f32, ly as f32, pointer));
                return;
            }
            // A layer surface's move or resize (#1523) takes the pointer, as
            // a compositor move does.
            // In the display's logical pixels, as the layer's place is.
            let zoom = crate::zoom::current();
            if self.update_layer_gesture(role, (lx * zoom, ly * zoom), event_loop) {
                return;
            }
            let id = pointer_id_of(source);
            let down = self.w(role).pressed_pointers.is_down(id);
            let pointer = surface_pointer_of(source, *primary, down);
            self.queue_pointer_move(role, lx as f32, ly as f32, pointer, event_loop);
            return;
        }
        // A mouse entering the window is where it is: a press can follow with
        // no move in between (an input injector, a window mapped under a
        // still pointer), and it must not be judged at a stale position
        // (#1523: a layer surface's drag started from (0, 0)).
        if let WindowEvent::PointerEntered {
            position,
            primary,
            kind: winit::event::PointerKind::Mouse,
            ..
        } = &event
        {
            let (lx, ly) = to_logical_point((position.x, position.y), self.w(role).scale_factor());
            let down = self
                .w(role)
                .pressed_pointers
                .is_down(SurfacePointer::mouse(false).id);
            let pointer = SurfacePointer {
                primary: *primary,
                ..SurfacePointer::mouse(down)
            };
            self.queue_pointer_move(role, lx as f32, ly as f32, pointer, event_loop);
            return;
        }
        // A press or release is judged where *its* pointer is: a finger or a
        // pen reports no move before it touches down, so the position the
        // press carries is queued as that pointer's move first (a mouse's
        // press is where its last move already put it).
        if let WindowEvent::PointerButton {
            state,
            position,
            primary,
            button,
            ..
        } = &event
        {
            let pressed = *state == ElementState::Pressed;
            // Any press or release ends a layer move or resize (#1523); a
            // press proves a release was missed.
            if let Some(layer) = self.w(role).layer.as_mut() {
                layer.gesture = None;
            }
            let pointer = surface_pointer_of_button(button, *primary, pressed);
            if !matches!(button, winit::event::ButtonSource::Mouse(_)) {
                let (lx, ly) =
                    to_logical_point((position.x, position.y), self.w(role).scale_factor());
                self.queue_pointer_move(role, lx as f32, ly as f32, pointer, event_loop);
            }
            self.flush_pointer_move(role, event_loop);
            self.w(role)
                .pressed_pointers
                .note(pointer.id, button.clone().mouse_button(), pressed);
            crate::render_surface::set_current_pointer(Some(pointer));
            // A press outside the popups above it closes them, after it is
            // delivered (#2552).
            if pressed {
                self.light_dismiss_press(role);
            }
        }
        self.flush_pointer_move(role, event_loop);

        // A layer surface's move reached the compositor (#1523).
        if let WindowEvent::Moved(p) = &event {
            self.w(role).layer_moved((p.x, p.y));
        }
        // Where the system put a popup (#2552).
        match &event {
            WindowEvent::Moved(p) => self.popup_placed(role, Some((p.x, p.y))),
            WindowEvent::SurfaceResized(_) => self.popup_placed(role, None),
            _ => {}
        }
        // Keep the published geometry (#1546) current: each window its own.
        if matches!(
            event,
            WindowEvent::Moved(_)
                | WindowEvent::SurfaceResized(_)
                | WindowEvent::ScaleFactorChanged { .. }
        ) {
            if let Some(w) = self.w(role).window.as_ref().map(|w| w.window.clone()) {
                self.w(role).note_geometry(&*w);
            }
        }

        let platform_event = match event {
            // A layer surface has no close button: its close is the
            // compositor's, for an output that went away. Show it again
            // rather than quitting (#1523).
            // Only the main window can be a layer surface.
            // A popup the system dismissed (`popup_done`, #2552): closed with
            // no veto, its surface is gone.
            WindowEvent::CloseRequested if self.popup_done(role) => return,
            WindowEvent::CloseRequested
                if role == WindowRole::Main && self.main.layer.is_some() =>
            {
                // Its popups go first, topmost first (#2552).
                let main = self.main_handle;
                self.close_popups_above(main);
                self.main
                    .layer_surface_closed(event_loop, self.native_menu.as_ref());
                return;
            }
            // A second window's layer surface closed by the compositor (its
            // output went) closes the window, with no veto: the surface is
            // gone already (#2416).
            WindowEvent::CloseRequested
                if matches!(role, WindowRole::Other(_)) && self.w(role).layer.is_some() =>
            {
                self.close_soon(role);
                return;
            }
            WindowEvent::CloseRequested => {
                // What the app reads in `on_close_requested` (#1884).
                self.w(role).settle_maximize_wait();
                PlatformEvent::CloseRequested
            }
            WindowEvent::SurfaceResized(size) => {
                let scale = self.w(role).scale_factor();
                self.w(role).surface_resized(size, scale)
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // The window moved to a display with a different scale (or the
                // user changed it). `handle_event` pushes the value into
                // Stylo's `device_pixel_ratio` (issue #211); the renderer
                // needs no reconfigure here — winit follows this event with a
                // `SurfaceResized` carrying the new physical size, handled
                // above.
                display_scale_changed(scale_factor)
            }
            WindowEvent::RedrawRequested => {
                // winit delivers a window's `RedrawRequested` only once the
                // frame callback its last paced present asked for has arrived
                // (#2669), so presenting to it no longer waits.
                #[cfg(feature = "gpu-renderer")]
                if let Some(renderer) = &mut self.w(role).renderer {
                    renderer.pacing.gate.callback_arrived();
                }
                // Drain queued native events (debug commands, injected input)
                // before painting so a command that arrived while the loop was
                // busy isn't serialized behind a full paint (#153). Pre-paint
                // is safe: the paint preamble re-resolves layout when
                // has_pending_layout(), so the paint sees the drained state.
                self.drain_native_events(event_loop);
                // Never a frame on a lost GPU device (#1788).
                #[cfg(feature = "gpu-renderer")]
                self.settle_gpu_device();
                // A content-sized window asks for its new size before the
                // frame, so the frame is drawn at it (#2552).
                self.follow_content_size(role, event_loop);
                // A window waiting for its renderer after a device loss
                // paints nothing until it has one (#1788).
                #[cfg(feature = "gpu-renderer")]
                if self.awaits_gpu(role) {
                    return;
                }
                // Paint directly -- this is shell-level, not delegated
                let painted = match role {
                    WindowRole::Main => self.paint(),
                    WindowRole::DevTools => self.paint_devtools(),
                    WindowRole::Other(_) => {
                        self.w(role).mark_popup("to frame");
                        let painted = self.w(role).paint_window();
                        self.w(role).note_popup_frame();
                        painted
                    }
                };
                // The first frame shown: the generic families are resolved
                // on a thread from here (#2552).
                if painted.is_ok() {
                    crate::font_warmup::window_presented();
                }
                if let Err(e) = painted {
                    // Not `eprintln!`, which panics on a closed stderr (#2531).
                    tracing::error!("Paint error: {}", e);
                }
                // winit coalesces wake_up() calls arbitrarily, so a wake that
                // landed mid-paint may already have been consumed. If events
                // queued while we painted, self-wake so they're delivered
                // immediately instead of waiting for the next external event
                // (#153).
                if !NATIVE_EVENT_QUEUE.lock().unwrap().is_empty() {
                    if let Some(proxy) = GLOBAL_PROXY.get() {
                        proxy.wake_up();
                    }
                }
                return;
            }
            // Coalesced above; never reaches this match.
            WindowEvent::PointerMoved { .. } => return,
            WindowEvent::PointerButton {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                // A finger is the left button, and so is a pen's tip (its
                // barrel button the right): winit's own mapping.
                let platform_button = match button.mouse_button() {
                    Some(MouseButton::Left) => PlatformMouseButton::Left,
                    Some(MouseButton::Right) => PlatformMouseButton::Right,
                    Some(MouseButton::Middle) => PlatformMouseButton::Middle,
                    _ => return,
                };
                // For click handling, we need the cursor position
                let (x, y) = self.w(role).app.cursor_pos.unwrap_or((0.0, 0.0));

                // Inspect mode: click selects node and exits inspect mode
                if role == WindowRole::Main && platform_button == PlatformMouseButton::Left {
                    if let Some(store) = &self.devtools_store {
                        if store.inspect_mode.get() {
                            if let Some(doc) = &self.main.app.doc {
                                let d = doc.borrow();
                                let hit = crate::app::hit_testing::hit_test(&d.tree, x, y);
                                store.selected_node_id.set(hit);
                            }
                            store
                                .active_tab
                                .set(crate::shell::devtools_store::DevToolsTab::Elements);
                            store.inspect_mode.set(false);
                            store.hovered_node_id.set(None);
                            self.main.app.inspect_highlight = None;
                            self.main.app.request_repaint();
                            if let Some(w) = &self.main.window {
                                w.request_redraw();
                            }
                            return;
                        }
                    }
                }

                // Outside the window's input region (#2741): the windowing
                // system sends no press there where it has the region; where
                // it has none, this is what keeps the press from the content.
                if !self.w(role).takes_press_at(x, y) {
                    return;
                }
                let size = self.w(role).window_size();
                let scale = self.w(role).scale_factor();
                let actions = self.w(role).app.handle_event(
                    PlatformEvent::MouseDown {
                        x,
                        y,
                        button: platform_button,
                    },
                    size,
                    scale,
                );
                self.process_actions(role, actions, event_loop);
                return;
            }
            WindowEvent::PointerButton {
                state: ElementState::Released,
                button,
                ..
            } => {
                // A finger is the left button, and so is a pen's tip (its
                // barrel button the right): winit's own mapping.
                let platform_button = match button.mouse_button() {
                    Some(MouseButton::Left) => PlatformMouseButton::Left,
                    Some(MouseButton::Right) => PlatformMouseButton::Right,
                    Some(MouseButton::Middle) => PlatformMouseButton::Middle,
                    _ => return,
                };
                let (x, y) = self.w(role).app.cursor_pos.unwrap_or((0.0, 0.0));
                PlatformEvent::MouseUp {
                    x,
                    y,
                    button: platform_button,
                }
            }
            // The mouse left the window: nothing is under it (#2672). `:hover`
            // clears, the elements it was over hear their leave, and the
            // `title` tooltip goes (#1646).
            WindowEvent::PointerLeft {
                kind: winit::event::PointerKind::Mouse,
                ..
            } => PlatformEvent::MouseLeft,
            // A touch the system stopped tracking without a release (the
            // window lost focus, a palm was rejected): the press it held is
            // cancelled.
            WindowEvent::PointerLeft {
                kind: winit::event::PointerKind::Touch(finger_id),
                ..
            } => {
                let id = TOUCH_POINTER_BASE + finger_id.into_raw() as u64;
                if !self.w(role).pressed_pointers.lift(id) {
                    return;
                }
                let pointer = SurfacePointer {
                    id,
                    kind: SurfacePointerKind::Touch,
                    pressure: 0.0,
                    primary: false,
                };
                self.w(role).app.cancel_surface_pointer(pointer);
                return;
            }
            // A trackpad pinch (macOS, iOS, Wayland): a `Pinch` for a
            // surface with pointer events under the cursor.
            // A pinch no surface takes is an `onwheel` with Ctrl held, as a
            // browser reports it.
            WindowEvent::PinchGesture { delta, .. } => {
                if delta.is_finite()
                    && !self.w(role).app.surface_pinch_gesture(delta as f32)
                    && self.w(role).app.pinch_as_wheel(delta as f32)
                    && let Some(w) = &self.w(role).window
                {
                    w.window.request_redraw();
                }
                return;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // Already logical: set from the converted `PointerMoved` above.
                let cursor = self.w(role).app.cursor_pos.unwrap_or((0.0, 0.0));
                match wheel_platform_event(delta, cursor, self.w(role).scale_factor()) {
                    Some(event) => event,
                    None => return,
                }
            }
            // KWin moves the keyboard from a menu's parent to the menu: the
            // parent did not lose it to the user (#2552).
            // (held, and told when the chain closes)
            WindowEvent::Focused(focused) if self.hold_focus_change(role, focused) => return,
            WindowEvent::Focused(focused) => {
                // A parent raised for a menu's keyboard has it: the menu is
                // made now (#2552).
                if focused && let Some(id) = self.window_id_of(role) {
                    self.parent_got_keyboard(id, event_loop);
                }
                // A change of the OS's reduced-motion setting is made in
                // another window; read it again as the focus comes back
                // (#2209).
                if focused {
                    crate::reduced_motion::refresh_from_os();
                }
                // A window that lost focus may never be sent the release of a
                // button held across the blur (it goes to the window that took
                // focus): forget every press, or a hover on a surface with
                // pointer events would go on reporting a pressure of 0.5 until
                // that button is pressed and released here again. The app's
                // own heal ends the press gestures and surface captures.
                if !focused {
                    self.w(role).pressed_pointers.clear();
                }
                // Tell the AT whether this window has OS focus.
                #[cfg(feature = "a11y")]
                if let Some(bridge) = self.w(role).a11y.as_mut() {
                    bridge.set_window_focused(focused);
                }
                // …and the app, which notifies the focused widget without
                // releasing its claim (issue #147, decision 1). This used to
                // `return` here, so window blur was invisible to the document:
                // a custom widget's caret kept blinking in an unfocused window
                // and the OS IME stayed armed on a window without the keyboard.
                PlatformEvent::WindowFocus(focused)
            }
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.w(role).modifiers = new_modifiers.state();
                let mods = self.w(role).translate_modifiers();
                PlatformEvent::ModifiersChanged(mods)
            }
            WindowEvent::KeyboardInput { ref event, .. } => {
                let mods = self.w(role).translate_modifiers();
                let locked = crate::pointer_lock::is_locked_in(self.w(role).facts_key);
                match Self::route_key_event(&self.w(role).app, event, mods, locked) {
                    KeyRoute::Event(event) => event,
                    KeyRoute::ShortcutRan => {
                        // Shortcut matched and callback dispatched.
                        if let Some(w) = &self.w(role).window {
                            w.request_redraw();
                        }
                        return;
                    }
                    KeyRoute::Nothing => return,
                }
            }
            WindowEvent::Ime(ime) => {
                // Translate winit's IME composition events into the portable
                // `ImeEvent` contract. `handle_event` then routes them through
                // the focus arbiter, exactly like keyboard input — so the
                // editor, `<input>`, etc. all consume IME the same way.
                let ime_event = match ime {
                    winit::event::Ime::Enabled => ImeEvent::Enabled,
                    winit::event::Ime::Preedit(text, cursor) => ImeEvent::Preedit { text, cursor },
                    winit::event::Ime::Commit(text) => ImeEvent::Commit(text),
                    winit::event::Ime::DeleteSurrounding {
                        before_bytes,
                        after_bytes,
                    } => ImeEvent::DeleteSurrounding {
                        // winit reports UTF-8 byte counts; the target converts to
                        // its char-based model. Exact byte→char conversion is only
                        // needed once we advertise the surrounding-text capability
                        // (not enabled yet), so this passes the counts through.
                        before: before_bytes,
                        after: after_bytes,
                    },
                    winit::event::Ime::Disabled => ImeEvent::Disabled,
                };
                PlatformEvent::Ime(ime_event)
            }
            // An OS file drag (#1962): winit's data transfer events, through
            // `OsFileDrag`, become the path-carrying `File*` events. A
            // file-drag position is a pointer position like any other, and is
            // hit-tested against the layout tree (#299).
            WindowEvent::DragEntered { id, position } => {
                use winit::data_transfer::TypeHint;
                let has_uri_list = event_loop
                    .data_transfer(id)
                    .is_ok_and(|transfer| transfer.has_type(&TypeHint::UriList));
                let position =
                    position.map(|p| to_logical_point((p.x, p.y), self.w(role).scale_factor()));
                let steps = self
                    .w(role)
                    .os_file_drag
                    .entered(id, position, has_uri_list);
                self.run_file_drag_steps(role, steps, event_loop);
                return;
            }
            WindowEvent::DragPosition { id, position, .. } => {
                let position =
                    to_logical_point((position.x, position.y), self.w(role).scale_factor());
                let steps = self.w(role).os_file_drag.moved(id, position);
                self.run_file_drag_steps(role, steps, event_loop);
                return;
            }
            WindowEvent::DataTransferReceived { id, serial, value } => {
                let uris = crate::shell::os_file_drag::uris_from(value.try_as_uris());
                let steps = self.w(role).os_file_drag.received(id, serial, uris);
                self.run_file_drag_steps(role, steps, event_loop);
                return;
            }
            WindowEvent::DragDropped { id, .. } => {
                let steps = self
                    .w(role)
                    .os_file_drag
                    .dropped(id, std::time::Instant::now());
                self.run_file_drag_steps(role, steps, event_loop);
                return;
            }
            WindowEvent::DragLeft { id } => {
                let steps = self.w(role).os_file_drag.left(id);
                self.run_file_drag_steps(role, steps, event_loop);
                return;
            }
            _ => return,
        };
        self.dispatch_event(role, platform_event, event_loop);
    }
}
