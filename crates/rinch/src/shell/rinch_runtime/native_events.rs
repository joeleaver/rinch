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
        for (to, event) in events {
            self.handle_native_event(to, event, event_loop);
        }
        // What the turn asked of each layer surface, in one commit (#2464).
        for role in self.roles() {
            self.w(role).flush_layer_changes();
        }
        self.draining_native_events = false;
    }

    /// The window an event is for, if it is open:
    /// - sent by a document's code, that document's window: DevTools' for
    ///   its own code, a second window's for its own, and the main window's
    ///   for the main window's and for code outside any document (`main`, a
    ///   worker thread, a menu or tray callback);
    /// - named by a handle, that window, or none once it has closed.
    pub(super) fn role_for(&self, to: EventFor) -> Option<WindowRole> {
        let main = (!self.main_closed).then_some(WindowRole::Main);
        match to {
            EventFor::Sender(Some(doc)) => {
                if self
                    .devtools
                    .as_ref()
                    .is_some_and(|d| d.app.doc_key() == doc)
                {
                    return Some(WindowRole::DevTools);
                }
                self.others
                    .iter()
                    .find(|w| w.app.doc_key() == doc)
                    .map(|w| WindowRole::Other(w.handle))
                    .or(main)
            }
            EventFor::Sender(None) => main,
            EventFor::Window(id) if id == self.main_handle => main,
            EventFor::Window(id) => self
                .others
                .iter()
                .find(|w| w.handle == id)
                .map(|w| WindowRole::Other(w.handle)),
        }
    }

    /// Which stored titles (`set_window_title`, by the document that set it)
    /// are `role`'s window's: a window's own document's, and the main
    /// window's every one no other window's document set.
    pub(super) fn title_owner(&self, role: WindowRole) -> impl Fn(Option<u64>) -> bool + use<> {
        let devtools = self.devtools.as_ref().map(|d| d.app.doc_key());
        let others: Vec<(u64, u64)> = self
            .others
            .iter()
            .map(|w| (w.handle, w.app.doc_key()))
            .collect();
        move |doc| match role {
            WindowRole::DevTools => doc.is_some() && doc == devtools,
            WindowRole::Other(id) => others.iter().any(|&(h, d)| h == id && Some(d) == doc),
            WindowRole::Main => {
                (devtools.is_none() || doc != devtools)
                    && !others.iter().any(|&(_, d)| Some(d) == doc)
            }
        }
    }

    /// Handle a single native event from the queue: a window control goes to
    /// the window it is for ([`Self::role_for`]), and one for a window that
    /// is not open does nothing.
    fn handle_native_event(
        &mut self,
        to: EventFor,
        event: RinchNativeEvent,
        event_loop: &dyn ActiveEventLoop,
    ) {
        // Not a window's: the app's.
        match event {
            RinchNativeEvent::OpenWindow(id) => {
                self.open_pending_window(id, event_loop);
                return;
            }
            RinchNativeEvent::Exit => {
                event_loop.exit();
                return;
            }
            _ => {}
        }
        let Some(role) = self.role_for(to) else {
            if let (EventFor::Window(id), RinchNativeEvent::CloseWindow) = (to, &event) {
                // Closed before it was made: it is never made.
                self.close_other(id);
            } else if !matches!(event, RinchNativeEvent::ReRender) {
                tracing::debug!("{event:?} for a window that is not open: ignored");
            }
            return;
        };
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
            RinchNativeEvent::ReRender if self.main_closed => {
                if self.windowless {
                    self.warn_windowless_root_content();
                }
                return;
            }
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
            RinchNativeEvent::CloseWindow => {
                if self.close_soon(role) {
                    event_loop.exit();
                }
                return;
            }
            RinchNativeEvent::FocusWindow => {
                if let Some(w) = &self.w(role).window {
                    w.raw().focus_window();
                }
                return;
            }
            RinchNativeEvent::SetWindowTitle(title) => {
                let w = self.w(role);
                if let Some(window) = &w.window {
                    window.raw().set_title(&title);
                }
                if let Some(props) = w.app.window_props.as_mut() {
                    props.title = title.clone();
                }
                w.title = title;
                return;
            }
            RinchNativeEvent::ShowWindow => PlatformEvent::UserEvent(UserEvent::ShowWindow),
            RinchNativeEvent::HideWindow => PlatformEvent::UserEvent(UserEvent::HideWindow),
            // The shell's alone (#2633); DevTools is never hidden.
            RinchNativeEvent::PrepareWindow => {
                if role != WindowRole::DevTools {
                    self.w(role).prepare_hidden();
                }
                return;
            }
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
                let backend = Backend::of(event_loop);
                let w = self.w(role);
                // An app's own layer surface (#2413), shown or hidden, is
                // placed by its anchors and margins (`place_layer` says so).
                if backend == Backend::Wayland
                    && w.app
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
                let layer_window = match super::layer_shell::published_surface_kind_of(w.facts_key)
                {
                    Some(kind) => kind == WindowSurfaceKind::LayerShell,
                    None => w
                        .app
                        .window_props
                        .as_ref()
                        .is_some_and(|p| super::layer_shell::uses_layer_shell(p, backend)),
                };
                let facts = w.facts_key;
                match (w.window.is_some(), w.layer.is_some()) {
                    (true, true) => w.stage_layer_placement((x, y)),
                    (true, false) => {
                        if let Some(win) = &w.window {
                            win.raw().set_outer_position(position);
                        }
                    }
                    // Wayland: a toplevel has no position to set or report.
                    (false, _) if !layer_window && backend == Backend::Wayland => {}
                    (false, _) => {
                        w.saved_geometry.position = Some(position);
                        window_geometry::publish_hidden(facts, Some((x, y)), None);
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
                self.w(role).output_override = Some(output);
                // A layer surface cannot change output: make it again there,
                // at the same position and size.
                if self.w(role).layer.is_some() {
                    self.hide_role(role);
                    self.show_role(role, event_loop);
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
                match role {
                    WindowRole::Main => {
                        self.main
                            .set_layer_surface(*surface, event_loop, self.native_menu.as_ref())
                    }
                    other => self.w(other).set_layer_surface(*surface, event_loop, None),
                }
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
            RinchNativeEvent::RequestActivationToken(request) => {
                self.request_activation_token(role, request, event_loop);
                return;
            }
            RinchNativeEvent::InjectedPlatformEvent(pe) => pe,
            // The app's, taken before a window is looked for.
            RinchNativeEvent::OpenWindow(_) | RinchNativeEvent::Exit => return,
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
        let role = |doc| runtime.role_for(EventFor::Sender(doc));
        assert_eq!(role(Some(other)), Some(WindowRole::DevTools));
        assert_eq!(role(Some(main)), Some(WindowRole::Main));
        assert_eq!(role(None), Some(WindowRole::Main), "outside any document");
        assert_eq!(role(Some(main + other + 1)), Some(WindowRole::Main));
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
