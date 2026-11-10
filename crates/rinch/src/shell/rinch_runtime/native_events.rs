//! Draining and handling the queued native events.

use super::*;

impl RinchRuntime {
    /// Drain queued native events (debug commands, window controls, injected
    /// platform events) and dispatch each through [`Self::handle_native_event`].
    ///
    /// Called from `proxy_wake_up` and from the top of the `RedrawRequested`
    /// arm — the latter so a command that arrived while the loop was busy is
    /// processed *before* the paint instead of stalling a full paint behind it
    /// (#153). Guarded against re-entry: `DebugCommandKind::Screenshot` calls
    /// [`Self::paint`] from inside the drain, so a nested drain must no-op.
    pub(super) fn drain_native_events(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.draining_native_events {
            return;
        }
        self.draining_native_events = true;
        let events = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        for event in events {
            self.handle_native_event(event, event_loop);
        }
        self.draining_native_events = false;
    }

    /// Handle a single native event from the queue.
    fn handle_native_event(&mut self, event: RinchNativeEvent, event_loop: &dyn ActiveEventLoop) {
        let platform_event = match event {
            RinchNativeEvent::ReRender => PlatformEvent::UserEvent(UserEvent::ReRender),
            #[cfg(feature = "debug")]
            RinchNativeEvent::DebugCommand => {
                // Debug commands may need the renderer (screenshots), so
                // we handle them here in the shell instead of delegating.
                if self.handle_debug_commands_with_renderer() {
                    event_loop.exit();
                }
                return;
            }
            RinchNativeEvent::MinimizeWindow => PlatformEvent::UserEvent(UserEvent::MinimizeWindow),
            RinchNativeEvent::ToggleMaximizeWindow => {
                PlatformEvent::UserEvent(UserEvent::ToggleMaximizeWindow)
            }
            RinchNativeEvent::CloseWindowControl => {
                PlatformEvent::UserEvent(UserEvent::CloseWindow)
            }
            RinchNativeEvent::ShowWindow => PlatformEvent::UserEvent(UserEvent::ShowWindow),
            RinchNativeEvent::HideWindow => PlatformEvent::UserEvent(UserEvent::HideWindow),
            RinchNativeEvent::SetClickThrough(click_through) => {
                // The shell's alone: no app state depends on it.
                let apply = self.note_click_through(click_through);
                if let Some(w) = &self.window {
                    overlay_window::set_click_through(w.raw(), apply);
                }
                return;
            }
            RinchNativeEvent::SetWindowPosition(x, y) => {
                // An app's own layer surface (#2413), shown or hidden, is
                // placed by its anchors and margins (`place_layer` says so).
                if Backend::of(event_loop) == Backend::Wayland
                    && self
                        .app
                        .window_props
                        .as_ref()
                        .is_some_and(|p| p.wayland_layer.is_some())
                {
                    tracing::debug!(
                        "set_current_window_position ignored: a layer surface with \
                         WindowProps::wayland_layer is placed by its anchors and margins"
                    );
                    return;
                }
                let position = Position::Logical(LogicalPosition::new(x, y));
                // A layer surface (#1523) is placed by its margins, on its
                // output; hidden, it comes back there.
                // What the window got, once it has been created (a toplevel
                // on GNOME); before that, what it will ask for.
                let layer_window = match super::layer_shell::published_surface_kind() {
                    Some(kind) => kind == WindowSurfaceKind::LayerShell,
                    None => self.app.window_props.as_ref().is_some_and(|p| {
                        super::layer_shell::uses_layer_shell(p, Backend::of(event_loop))
                    }),
                };
                match &self.window {
                    Some(_) if self.layer.is_some() => self.place_layer((x, y)),
                    Some(w) => w.raw().set_outer_position(position),
                    None if layer_window => {
                        self.saved_geometry.position = Some(position);
                        window_geometry::publish_hidden(Some((x, y)), None);
                    }
                    // Wayland: a toplevel has no position to set or report.
                    None if Backend::of(event_loop) == Backend::Wayland => {}
                    None => {
                        self.saved_geometry.position = Some(position);
                        window_geometry::publish_hidden(Some((x, y)), None);
                    }
                }
                return;
            }
            RinchNativeEvent::SetWindowSize(width, height) => {
                let size = Size::Logical(LogicalSize::new(width, height));
                match &self.window {
                    Some(w) => {
                        let id = w.raw().id();
                        if let Some(physical) = w.raw().request_surface_size(size) {
                            // Resized at once: winit sends no
                            // `SurfaceResized` for it, so hand one to the
                            // window's own path (renderer, layout, publish).
                            self.window_event(
                                event_loop,
                                id,
                                WindowEvent::SurfaceResized(physical),
                            );
                        }
                    }
                    None => {
                        self.saved_geometry.size = Some(size);
                        window_geometry::publish_hidden(None, Some((width, height)));
                    }
                }
                return;
            }
            RinchNativeEvent::ApplyWindowTitle => {
                self.take_stored_title();
                return;
            }
            RinchNativeEvent::ApplyVsync => {
                self.apply_vsync();
                return;
            }
            RinchNativeEvent::SetWindowOutput(output) => {
                self.output_override = Some(output);
                // A layer surface cannot change output: make it again there,
                // at the same position and size.
                if self.layer.is_some() {
                    self.hide_window();
                    self.show_window(event_loop);
                }
                return;
            }
            // From the app's own press handler; refused once the press is
            // over (a timer, or the release dispatched before this event).
            RinchNativeEvent::StartWindowDrag => {
                if self.pressed_pointers.primary_down() {
                    self.process_actions(vec![AppAction::DragWindow], event_loop);
                } else {
                    tracing::debug!("start_window_drag refused: no button is down");
                }
                return;
            }
            RinchNativeEvent::StartWindowResize(dir) => {
                if self.pressed_pointers.primary_down() {
                    self.process_actions(vec![AppAction::DragResizeWindow(dir)], event_loop);
                } else {
                    tracing::debug!("start_window_resize refused: no button is down");
                }
                return;
            }
            RinchNativeEvent::SetWindowLayer(surface) => {
                self.set_layer_surface(*surface, event_loop);
                return;
            }
            RinchNativeEvent::RetryLayerOutput => {
                if self.layer_awaiting_output && self.window.is_none() {
                    if event_loop.available_monitors().next().is_some() {
                        self.layer_awaiting_output = false;
                        self.show_window(event_loop);
                    } else {
                        arm_layer_output_retry();
                    }
                }
                return;
            }
            RinchNativeEvent::InjectedPlatformEvent(pe) => pe,
            #[cfg(feature = "a11y")]
            RinchNativeEvent::A11yAction(req) => {
                self.apply_a11y_action(req);
                return;
            }
        };
        let size = self.window_size();
        let scale = self.scale_factor();
        let actions = self.app.handle_event(platform_event, size, scale);
        self.process_actions(actions, event_loop);
    }
}
