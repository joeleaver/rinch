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
        for (doc, event) in events {
            self.handle_native_event(doc, event, event_loop);
        }
        // What the turn asked of the layer surface, in one commit (#2464).
        self.main.flush_layer_changes();
        self.draining_native_events = false;
    }

    /// The window an event sent by document `doc`'s code is for: the
    /// DevTools window for DevTools' own code, the main window for the
    /// main window's and for code outside any document (`main`, a worker
    /// thread, a menu or tray callback).
    pub(super) fn role_for_doc(&self, doc: Option<u64>) -> WindowRole {
        match (doc, self.devtools.as_ref()) {
            (Some(doc), Some(devtools)) if devtools.app.doc_key() == doc => WindowRole::DevTools,
            _ => WindowRole::Main,
        }
    }

    /// Which stored titles (`set_window_title`, by the document that set it)
    /// are `role`'s window's: DevTools' document's are DevTools', every other
    /// one the main window's.
    pub(super) fn title_owner(&self, role: WindowRole) -> impl Fn(Option<u64>) -> bool + use<> {
        let devtools = self.devtools.as_ref().map(|d| d.app.doc_key());
        move |doc| match role {
            WindowRole::DevTools => doc.is_some() && doc == devtools,
            WindowRole::Main => devtools.is_none() || doc != devtools,
        }
    }

    /// Handle a single native event from the queue, sent by document `doc`'s
    /// code: a window control goes to that document's window
    /// ([`Self::role_for_doc`]).
    fn handle_native_event(
        &mut self,
        doc: Option<u64>,
        event: RinchNativeEvent,
        event_loop: &dyn ActiveEventLoop,
    ) {
        let role = self.role_for_doc(doc);
        // Layer surfaces, outputs and AccessKit are the main window's.
        if role == WindowRole::DevTools
            && matches!(
                event,
                RinchNativeEvent::SetWindowOutput(_)
                    | RinchNativeEvent::SetWindowLayer(_)
                    | RinchNativeEvent::RetryLayerOutput
            )
        {
            tracing::debug!("{event:?} from the DevTools window ignored: it is no layer surface");
            return;
        }
        let platform_event = match event {
            // Every window's resolve: the main window's here, DevTools' in its
            // `AboutToWait`.
            RinchNativeEvent::ReRender => {
                let size = self.main.window_size();
                let scale = self.main.scale_factor();
                let actions = self.main.app.handle_event(
                    PlatformEvent::UserEvent(UserEvent::ReRender),
                    size,
                    scale,
                );
                self.process_actions(WindowRole::Main, actions, event_loop);
                return;
            }
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
                let w = self.w(role);
                let apply = w.note_click_through(click_through);
                if let Some(w) = &w.window {
                    overlay_window::set_click_through(w.raw(), apply);
                }
                return;
            }
            // DevTools is an ordinary window: moved where it is asked.
            RinchNativeEvent::SetWindowPosition(x, y) if role == WindowRole::DevTools => {
                if let Some(w) = &self.w(role).window {
                    w.raw()
                        .set_outer_position(Position::Logical(LogicalPosition::new(x, y)));
                }
                return;
            }
            RinchNativeEvent::SetWindowPosition(x, y) => {
                // An app's own layer surface (#2413), shown or hidden, is
                // placed by its anchors and margins (`place_layer` says so).
                if Backend::of(event_loop) == Backend::Wayland
                    && self
                        .main
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
                let layer_window = match super::layer_shell::published_surface_kind_of(None) {
                    Some(kind) => kind == WindowSurfaceKind::LayerShell,
                    None => self.main.app.window_props.as_ref().is_some_and(|p| {
                        super::layer_shell::uses_layer_shell(p, Backend::of(event_loop))
                    }),
                };
                match &self.main.window {
                    Some(_) if self.main.layer.is_some() => self.main.stage_layer_placement((x, y)),
                    Some(w) => w.raw().set_outer_position(position),
                    None if layer_window => {
                        self.main.saved_geometry.position = Some(position);
                        window_geometry::publish_hidden(None, Some((x, y)), None);
                    }
                    // Wayland: a toplevel has no position to set or report.
                    None if Backend::of(event_loop) == Backend::Wayland => {}
                    None => {
                        self.main.saved_geometry.position = Some(position);
                        window_geometry::publish_hidden(None, Some((x, y)), None);
                    }
                }
                return;
            }
            RinchNativeEvent::SetWindowSize(width, height) => {
                let size = Size::Logical(LogicalSize::new(width, height));
                match &self.w(role).window {
                    Some(w) => {
                        if let Some(physical) = w.raw().request_surface_size(size) {
                            // Resized at once: winit sends no
                            // `SurfaceResized` for it, so hand one to the
                            // window's own path (renderer, layout, publish).
                            self.window_event_for(
                                role,
                                event_loop,
                                WindowEvent::SurfaceResized(physical),
                            );
                        }
                    }
                    None => {
                        self.w(role).saved_geometry.size = Some(size);
                        window_geometry::publish_hidden(
                            self.w(role).facts_key,
                            None,
                            Some((width, height)),
                        );
                    }
                }
                return;
            }
            RinchNativeEvent::ApplyWindowTitle => {
                let owns = self.title_owner(role);
                self.w(role).take_stored_title(owns);
                return;
            }
            RinchNativeEvent::ApplyVsync => {
                self.apply_vsync();
                return;
            }
            RinchNativeEvent::SetWindowOutput(output) => {
                self.main.output_override = Some(output);
                // A layer surface cannot change output: make it again there,
                // at the same position and size.
                if self.main.layer.is_some() {
                    self.hide_window();
                    self.show_window(event_loop);
                }
                return;
            }
            // From the app's own press handler; refused once the press is
            // over (a timer, or the release dispatched before this event).
            RinchNativeEvent::StartWindowDrag => {
                if self.w(role).pressed_pointers.primary_down() {
                    self.process_actions(role, vec![AppAction::DragWindow], event_loop);
                } else {
                    tracing::debug!("start_window_drag refused: no button is down");
                }
                return;
            }
            RinchNativeEvent::StartWindowResize(dir) => {
                if self.w(role).pressed_pointers.primary_down() {
                    self.process_actions(role, vec![AppAction::DragResizeWindow(dir)], event_loop);
                } else {
                    tracing::debug!("start_window_resize refused: no button is down");
                }
                return;
            }
            RinchNativeEvent::SetWindowLayer(surface) => {
                self.main
                    .set_layer_surface(*surface, event_loop, self.native_menu.as_ref());
                return;
            }
            RinchNativeEvent::RetryLayerOutput => {
                if self.main.layer_awaiting_output && self.main.window.is_none() {
                    if event_loop.available_monitors().next().is_some() {
                        self.main.layer_awaiting_output = false;
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
                self.main.apply_a11y_action(req);
                return;
            }
        };
        let size = self.w(role).window_size();
        let scale = self.w(role).scale_factor();
        let actions = self.w(role).app.handle_event(platform_event, size, scale);
        self.process_actions(role, actions, event_loop);
    }
}

/// #2416: a native event is for the window of the document whose code sent
/// it, and a title for the window whose code set it.
#[cfg(test)]
mod per_window_3b_tests {
    use super::*;

    fn component(scope: &mut RenderScope) -> NodeHandle {
        scope.create_element("div")
    }

    /// A runtime with a mounted main window and a mounted second window in
    /// the DevTools slot; answers the two documents' keys.
    fn two_windows() -> (RinchRuntime, u64, u64) {
        let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        runtime.main.app.mount_component(800.0, 600.0);
        let mut other = RinchApp::new(component);
        other.mount_component(480.0, 600.0);
        runtime.devtools = Some(ShellWindow::new(other, "DevTools", 480, 600));
        let main = runtime.main.app.doc_key();
        let other = runtime.devtools.as_ref().unwrap().app.doc_key();
        assert!(main != 0 && other != 0 && main != other);
        (runtime, main, other)
    }

    #[test]
    fn an_event_is_for_the_window_of_the_document_that_sent_it() {
        let (runtime, main, other) = two_windows();
        assert_eq!(runtime.role_for_doc(Some(other)), WindowRole::DevTools);
        assert_eq!(runtime.role_for_doc(Some(main)), WindowRole::Main);
        assert_eq!(
            runtime.role_for_doc(None),
            WindowRole::Main,
            "outside any document"
        );
        assert_eq!(
            runtime.role_for_doc(Some(main + other + 1)),
            WindowRole::Main
        );
    }

    #[test]
    fn a_title_is_the_window_s_whose_code_set_it() {
        let (runtime, main, other) = two_windows();
        let devtools = runtime.title_owner(WindowRole::DevTools);
        let main_owns = runtime.title_owner(WindowRole::Main);
        assert!(devtools(Some(other)));
        assert!(!devtools(Some(main)) && !devtools(None));
        assert!(main_owns(Some(main)) && main_owns(None));
        assert!(!main_owns(Some(other)), "not the other window's");
    }
}
