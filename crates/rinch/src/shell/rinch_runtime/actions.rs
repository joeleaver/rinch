//! Carrying out the app's actions, the pointer's pending moves, the per-turn work and the timed wakes.

use super::*;

impl RinchRuntime {
    /// Process the actions returned by RinchApp.
    pub(super) fn process_actions(
        &mut self,
        role: WindowRole,
        actions: Vec<AppAction>,
        event_loop: &dyn ActiveEventLoop,
    ) {
        for action in actions {
            match action {
                AppAction::RequestRedraw => {
                    if let Some(w) = &self.w(role).window {
                        w.request_redraw();
                    }
                }
                // The window's app agreed to its close (its close button,
                // `close_current_window`, a debug `close_app`): the process
                // ends with the main window, and DevTools just closes.
                AppAction::Exit => {
                    if self.close_soon(role) {
                        event_loop.exit();
                    }
                }
                AppAction::SetMinimized(minimized) => {
                    if let Some(w) = &self.w(role).window {
                        w.set_minimized(minimized);
                    }
                }
                AppAction::SetMaximized(_) => {
                    // Toggle maximize: the app doesn't know current state,
                    // so we toggle it here.
                    if let Some(w) = &self.w(role).window {
                        let is_max = w.is_maximized();
                        w.set_maximized(!is_max);
                    }
                }
                // DevTools is opened and closed by F12, not hidden.
                AppAction::SetVisible(_) if role == WindowRole::DevTools => {}
                AppAction::SetVisible(visible) => {
                    // The app decides now: no pending re-show (#1523).
                    self.w(role).layer_awaiting_output = false;
                    if visible {
                        self.show_role(role, event_loop);
                    } else {
                        self.hide_role(role);
                    }
                }
                AppAction::DragWindow => {
                    // A layer surface has no compositor move (#1523).
                    if !self.w(role).start_layer_gesture(GestureKind::Move) {
                        if let Some(w) = &self.w(role).window {
                            let _ = w.drag_window();
                        }
                    }
                }
                AppAction::DragResizeWindow(dir) => {
                    if !self.w(role).start_layer_gesture(GestureKind::Resize(dir)) {
                        if let Some(w) = &self.w(role).window {
                            let _ = w.drag_resize_window(dir);
                        }
                    }
                }
                AppAction::SetCursor(style) => {
                    if self.w(role).applied_cursor == Some(style) {
                        continue;
                    }
                    let win = self.w(role);
                    if let Some(w) = &win.window {
                        w.window.set_cursor(winit::cursor::Cursor::from(
                            Self::cursor_style_to_winit(style),
                        ));
                        win.applied_cursor = Some(style);
                    }
                }
                AppAction::ToggleDevTools => {
                    self.toggle_devtools(event_loop);
                }
                AppAction::ToggleInspectMode => {
                    self.toggle_inspect_mode();
                }
                // Desktop keeps the runtime's DOM menu (issue #813): this is
                // only ever emitted under a presentation the desktop shell
                // never asks for.
                AppAction::ShowTextContextMenu => {}
            }
        }
    }
}

impl RinchRuntime {}

impl RinchRuntime {
    /// Carry out what [`super::os_file_drag::OsFileDrag`] answered for a drag
    /// event (accept the drag, ask for its URI list, log), then hand the app
    /// the events it produced, in order.
    pub(super) fn run_file_drag_steps(
        &mut self,
        role: WindowRole,
        steps: Vec<super::os_file_drag::DragStep>,
        event_loop: &dyn ActiveEventLoop,
    ) {
        let mut host = WinitDragHost {
            event_loop,
            events: Vec::new(),
        };
        super::os_file_drag::run_steps(&mut self.w(role).os_file_drag, steps, &mut host);
        for event in host.events {
            self.dispatch_event(role, event, event_loop);
        }
    }
}

impl ShellWindow {
    /// This window's pointer lock goes through it (#2092; each window its
    /// own host, #2416).
    pub(super) fn install_pointer_lock_host(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        crate::pointer_lock::set_pointer_lock_host_for(
            self.facts_key,
            Some(Box::new(crate::shell::pointer_lock::WindowPointerLock {
                window: Arc::downgrade(&window.window),
            })),
        );
    }

    /// One device motion (#2092): summed into the turn's motion when it has
    /// anywhere to go (a lock is held, or an `on_pointer_motion` handler
    /// listens), dropped otherwise, so an app that never asks pays nothing.
    pub(super) fn note_device_event(&mut self, event: &DeviceEvent) {
        if let DeviceEvent::PointerMotion { delta } = event
            && crate::pointer_lock::wants_motion()
        {
            tracing::trace!(dx = delta.0, dy = delta.1, "device motion");
            self.pending_motion.add(delta.0, delta.1);
        }
    }
}

impl RinchRuntime {
    /// Hand the app the motion summed since the last flush, as one
    /// `PointerMotion`: before any other event (so a press or a key comes
    /// after the motion that preceded it) and when the turn ends.
    pub(super) fn flush_pointer_motion(&mut self, event_loop: &dyn ActiveEventLoop) {
        for role in self.roles() {
            if let Some(motion) = self.w(role).take_pointer_motion() {
                self.dispatch_event(role, motion, event_loop);
            }
        }
    }

    /// The window the device's raw motion is for: the one whose host holds
    /// the pointer lock (#2416), else the main window (an
    /// `on_pointer_motion` handler with no lock hears it there, #2092).
    pub(super) fn motion_role(&mut self) -> WindowRole {
        self.roles()
            .find(|&role| crate::pointer_lock::is_locked_in(self.w(role).facts_key))
            .unwrap_or(WindowRole::Main)
    }
}

impl ShellWindow {
    /// Once no lock holds the pointer, the position it reached meanwhile is
    /// the move the app is owed (unless a newer one is already waiting): the
    /// document's hover catches up with the pointer.
    pub(super) fn restore_held_pointer_move(&mut self) {
        if crate::pointer_lock::is_locked_in(self.facts_key) {
            return;
        }
        if let Some(held) = self.held_pointer_move.take()
            && self.pending_pointer_move.is_none()
        {
            self.pending_pointer_move = Some(held);
        }
    }

    /// The motion [`RinchRuntime::flush_pointer_motion`] hands the app, taken.
    fn take_pointer_motion(&mut self) -> Option<PlatformEvent> {
        self.pending_motion
            .take()
            .map(|(dx, dy)| PlatformEvent::PointerMotion { dx, dy })
    }
}

impl RinchRuntime {
    /// Hand the app the pointer move [`Self::window_event`] held back, if any.
    pub(super) fn flush_pointer_move(
        &mut self,
        role: WindowRole,
        event_loop: &dyn ActiveEventLoop,
    ) {
        if let Some((x, y, pointer)) = self.w(role).pending_pointer_move.take() {
            crate::render_surface::set_current_pointer(Some(pointer));
            self.dispatch_event(role, PlatformEvent::MouseMove { x, y }, event_loop);
            crate::render_surface::set_current_pointer(None);
        }
    }

    /// Hold back a move of `pointer` to `(x, y)` (logical px) until the
    /// batch ends or another event comes, replacing a held move of the same
    /// pointer: one move per pointer per frame. A held move of **another**
    /// pointer (the other finger of a pinch) is handed over first, so no
    /// pointer's last position is lost.
    pub(super) fn queue_pointer_move(
        &mut self,
        role: WindowRole,
        x: f32,
        y: f32,
        pointer: SurfacePointer,
        event_loop: &dyn ActiveEventLoop,
    ) {
        if self
            .w(role)
            .pending_pointer_move
            .is_some_and(|(_, _, held)| held.id != pointer.id)
        {
            self.flush_pointer_move(role, event_loop);
        }
        self.w(role).pending_pointer_move = Some((x, y, pointer));
    }

    /// Dispatch one event to `role`'s app and act on what it asks for.
    pub(super) fn dispatch_event(
        &mut self,
        role: WindowRole,
        platform_event: PlatformEvent,
        event_loop: &dyn ActiveEventLoop,
    ) {
        // Inspect mode: intercept the main window's moves for hit-testing
        if role == WindowRole::Main
            && let Some(store) = &self.devtools_store
        {
            if store.inspect_mode.get() {
                if let PlatformEvent::MouseMove { x, y } = &platform_event {
                    // Hit-test the main app's document
                    self.main.app.cursor_pos = Some((*x, *y));
                    if let Some(doc) = &self.main.app.doc {
                        let d = doc.borrow();
                        let hit = crate::app::hit_testing::hit_test(&d.tree, *x, *y);
                        store.hovered_node_id.set(hit);
                    }
                }
            }
        }

        let size = self.w(role).window_size();
        let scale = self.w(role).scale_factor();
        let actions = self.w(role).app.handle_event(platform_event, size, scale);
        self.process_actions(role, actions, event_loop);
    }

    pub(super) fn about_to_wait_inner(&mut self, event_loop: &dyn ActiveEventLoop) {
        if !self.main_closed {
            self.about_to_wait_for(WindowRole::Main, event_loop);
        }
        self.about_to_wait_others(event_loop);

        // Notify DevTools when main app DOM changed, then resolve its layout
        if self.devtools.is_some() {
            // Bump version counter so DevTools effects re-read the document.
            // The version only changes when the main app actually resolved
            // dirty nodes (scene_dirty is set in resolve_and_repaint).
            if self.main.app.scene_dirty {
                if let Some(store) = &self.devtools_store {
                    store.bump_version();
                }
            }

            // Its turn, after the bump, so it lays out what the bump changed.
            self.about_to_wait_for(WindowRole::DevTools, event_loop);

            // If hovered node changed in DevTools, request main window repaint
            // for inspect highlight
            if let Some(store) = &self.devtools_store {
                let new_hovered = store.hovered_node_id.get();
                if new_hovered != self.devtools_prev_hovered {
                    self.devtools_prev_hovered = new_hovered;
                    if let Some(w) = &self.main.window {
                        w.request_redraw();
                    }
                }
            }
        }

        // Reconcile each window's IME state with its focus arbiter (enable on
        // a focused text target, follow the caret, disable on blur).
        for role in self.roles() {
            self.w(role).sync_ime();
        }

        // Push the focused editor's accessibility tree to the AT when it changed.
        #[cfg(feature = "a11y")]
        self.main.sync_a11y();

        // Drive the focused editor's caret blink, and wake for a resting
        // pointer's `title` tooltip (#1646) and a file drop's deadline (#1962).
        // These are the only timed wakes (`WaitUntil`); with none it returns
        // the loop to `Wait` so the app stays idle.
        #[cfg(feature = "desktop")]
        self.tick_caret_blink(event_loop);
    }

    /// One window's `AboutToWait`: its transitions, animations, glides and
    /// layout for the turn, and what it asks for after them.
    pub(super) fn about_to_wait_for(&mut self, role: WindowRole, event_loop: &dyn ActiveEventLoop) {
        let size = self.w(role).window_size();
        let scale = self.w(role).scale_factor();
        let actions = self
            .w(role)
            .app
            .handle_event(PlatformEvent::AboutToWait, size, scale);
        self.process_actions(role, actions, event_loop);
        // A change of a content-sized window's content asks for its size (#2552).
        self.follow_content_size(role, event_loop);
    }
}

impl RinchRuntime {
    /// Blink the focused editor's caret by arming a `WaitUntil` wake for the next
    /// phase toggle (see [`crate::editor::caret_blink_tick`]), for a pending
    /// `title` tooltip's deadline, or for a file drop's (ending the drops past
    /// theirs), whichever is sooner. Owns the event loop's control flow:
    /// `WaitUntil` while any is due, `Wait` otherwise.
    ///
    /// *Which* editor blinks is [`RinchApp::blinking_editor_id`]'s decision, not
    /// this function's — it excludes a blurred window, so a backgrounded app
    /// stops arming the only timed wake the loop has (issue #316).
    #[cfg(feature = "desktop")]
    fn tick_caret_blink(&mut self, event_loop: &dyn ActiveEventLoop) {
        let soonest = |a: Option<std::time::Instant>, b: Option<std::time::Instant>| match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let mut wake = None;
        for role in self.roles() {
            // Each window's document blinks its own focused editor.
            let w = self.w(role);
            let focused = w.app.blinking_editor_id();
            if let Some(blink) = crate::editor::caret_blink_tick(w.app.doc_key(), focused) {
                if blink.redraw
                    && let Some(w) = &w.window
                {
                    w.request_redraw();
                }
                wake = soonest(wake, Some(std::time::Instant::now() + blink.next));
            }
            // A pointer resting over a `title` shows its tooltip at its
            // deadline (#1646): another timed wake.
            wake = soonest(wake, self.w(role).app.title_tooltip_deadline());
            // A file drop still waiting for its paths gives up at its
            // deadline (#1962): the third.
            let expired = self.w(role).os_file_drag.expire(std::time::Instant::now());
            if !expired.is_empty() {
                self.run_file_drag_steps(role, expired, event_loop);
            }
            wake = soonest(wake, self.w(role).os_file_drag.next_deadline());
        }
        // A window created maximized is read once more when the maximize
        // wait's grace ends (#1884): the fourth. Settled, the wait has no
        // deadline, so it arms nothing after.
        if self
            .main
            .maximize_wait
            .deadline()
            .is_some_and(|d| std::time::Instant::now() >= d)
        {
            self.main.settle_maximize_wait();
        }
        let wake = match (wake, self.main.maximize_wait.deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        // Vello's counters on their way back from the GPU (#2071): the fifth.
        // Armed only until they arrive, so a window at rest goes back to `Wait`.
        #[cfg(feature = "gpu")]
        let wake = match (wake, self.check_vello_frames()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        match wake {
            Some(at) => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    /// See what became of the frames the windows gave Vello (#2071). A frame
    /// that overflowed Vello's buffers went out undrawn, and on a window at rest
    /// nothing else would ever draw it, so this asks for the redraw that does:
    /// [`WgpuRenderer::check_frames`] has made the next frame a full one, at the
    /// buffers' new sizes. Answers when to look again, `None` once every frame
    /// is accounted for.
    #[cfg(feature = "gpu")]
    fn check_vello_frames(&mut self) -> Option<std::time::Instant> {
        use vello::FrameCheck;
        let main = self.main.renderer.as_mut().map(WgpuRenderer::check_frames);
        if main == Some(FrameCheck::Lost)
            && let Some(w) = &self.main.window
        {
            w.request_redraw();
        }
        let devtools = self.devtools.as_mut().and_then(|d| {
            let check = d.renderer.as_mut().map(WgpuRenderer::check_frames);
            if check == Some(FrameCheck::Lost)
                && let Some(w) = &d.window
            {
                w.request_redraw();
            }
            check
        });
        [main, devtools]
            .contains(&Some(FrameCheck::Pending))
            .then(|| std::time::Instant::now() + VELLO_CHECK_INTERVAL)
    }
}

impl ShellWindow {
    /// Push the focus arbiter's desired IME state to the window. Enables IME with
    /// a cursor area when a text target is focused, disables it otherwise, and
    /// moves the candidate-box rect as the caret moves. Diffs against the
    /// last-applied state so it only issues a `request_ime_update` on a real
    /// change. This is the single place the winit IME surface is touched; every
    /// text target (editor, `<input>`, …) feeds it through [`RinchApp::ime_state`].
    fn sync_ime(&mut self) {
        use winit::window::{ImeCapabilities, ImeEnableRequest, ImeRequest, ImeRequestData};

        let Some(w) = &self.window else { return };
        let desired = self.app.ime_state();

        let to_area = |a: rinch_core::ElementBounds| {
            (
                winit::dpi::LogicalPosition::new(a.x as f64, a.y as f64),
                winit::dpi::LogicalSize::new(a.width.max(1.0) as f64, a.height.max(1.0) as f64),
            )
        };

        match self.ime_applied.change_to(desired) {
            Some(crate::app::ImeChange::Enable { cursor_area, .. }) => {
                let area = cursor_area.unwrap_or(rinch_core::ElementBounds {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 16.0,
                });
                let (pos, size) = to_area(area);
                let caps = ImeCapabilities::new().with_cursor_area();
                let data = ImeRequestData::default().with_cursor_area(pos.into(), size.into());
                if let Some(req) = ImeEnableRequest::new(caps, data) {
                    let _ = w.window.request_ime_update(ImeRequest::Enable(req));
                }
            }
            Some(crate::app::ImeChange::Update { cursor_area, .. }) => {
                let (pos, size) = to_area(cursor_area);
                let data = ImeRequestData::default().with_cursor_area(pos.into(), size.into());
                let _ = w.window.request_ime_update(ImeRequest::Update(data));
            }
            Some(crate::app::ImeChange::Disable) => {
                let _ = w.window.request_ime_update(ImeRequest::Disable);
            }
            None => {}
        }
    }

    /// Push the focused editor's accessibility tree to the AT when its document or
    /// selection changed since the last push (the AT consumes a full snapshot). The
    /// per-platform bridge is created lazily on first editor focus; on Linux it is
    /// the AT-SPI adapter, elsewhere a no-op. The whole method compiles to nothing
    /// without the `a11y` feature.
    #[cfg(feature = "a11y")]
    fn sync_a11y(&mut self) {
        let Some(handle) = self
            .app
            .focused_editor_id()
            .and_then(|id| crate::editor::editor_for_doc(self.app.doc_key(), id))
        else {
            return;
        };
        let state = handle.state();
        let unchanged = matches!(
            &self.a11y_last,
            Some((doc, sel)) if doc.same_ref(&state.doc) && *sel == state.selection
        );
        if unchanged {
            return;
        }
        let bridge = self
            .a11y
            .get_or_insert_with(crate::editor::a11y::AccesskitBridge::new);
        bridge.update(&state);
        self.a11y_last = Some((state.doc.clone(), state.selection.clone()));
    }

    /// Apply an AT-requested action to the focused editor. A screen reader moving
    /// the caret/selection arrives as `SetTextSelection`; `Focus` is advisory (the
    /// focus arbiter owns focus). Runs on the main thread (from the native-event
    /// queue) after the adapter thread forwarded the request.
    #[cfg(feature = "a11y")]
    pub(super) fn apply_a11y_action(&mut self, req: accesskit::ActionRequest) {
        use accesskit::{Action, ActionData};
        let Some(handle) = self
            .app
            .focused_editor_id()
            .and_then(|id| crate::editor::editor_for_doc(self.app.doc_key(), id))
        else {
            return;
        };
        if req.action == Action::SetTextSelection
            && let Some(ActionData::SetTextSelection(ak_sel)) = &req.data
            && let Some(sel) =
                rinch_editor_core::a11y::accesskit_selection_to_model(&handle.doc(), ak_sel)
        {
            handle.set_selection(sel);
            send_native_event(RinchNativeEvent::ReRender);
        }
    }
}

#[cfg(test)]
mod pointer_lock_2092_tests {
    //! The shell's half of a pointer lock (#2092): winit's device motion,
    //! summed over a turn and handed to the app as one `PointerMotion` only
    //! when something listens, and the lock a hidden window gives back.
    use super::*;
    use crate::pointer_lock::test_host::install;
    use crate::render_surface::{RenderSurfaceHandle, SurfaceEvent, create_render_surface};
    use crate::window::{PointerLockError, PointerLockMode, pointer_lock, request_pointer_lock};
    use rinch_core::Component;
    use std::cell::RefCell;
    use std::rc::Rc;

    thread_local! {
        static SURFACE: RefCell<Option<RenderSurfaceHandle>> = const { RefCell::new(None) };
    }

    fn component(scope: &mut RenderScope) -> NodeHandle {
        let root = scope.create_element("div");
        let surface = SURFACE.with(|s| s.borrow().clone()).unwrap();
        let canvas = crate::render_surface::RenderSurface {
            surface: Some(surface),
        }
        .render(scope, &[]);
        root.append_child(&canvas);
        root
    }

    /// The `PointerMotion` steps a surface heard.
    type Heard = Rc<RefCell<Vec<(f32, f32)>>>;

    fn device_motion(dx: f64, dy: f64) -> DeviceEvent {
        DeviceEvent::PointerMotion { delta: (dx, dy) }
    }

    /// A runtime whose app has mounted a surface, and what the surface heard.
    fn runtime() -> (RinchRuntime, RenderSurfaceHandle, Heard) {
        crate::window::set_pointer_lock_host(None);
        let surface = create_render_surface();
        SURFACE.with(|s| *s.borrow_mut() = Some(surface.clone()));
        let mut runtime = RinchRuntime::new("t", 640, 480, component, &[]);
        runtime.main.app.mount_component(640.0, 480.0);
        let heard: Heard = Rc::default();
        let h = heard.clone();
        surface.set_event_handler(move |e| {
            if let SurfaceEvent::PointerMotion { dx, dy } = e {
                h.borrow_mut().push((dx, dy));
            }
        });
        (runtime, surface, heard)
    }

    #[test]
    fn device_motion_nobody_wants_is_dropped() {
        let (mut runtime, _surface, _heard) = runtime();
        runtime.main.note_device_event(&device_motion(5.0, 5.0));
        assert!(runtime.main.take_pointer_motion().is_none());
    }

    #[test]
    fn device_motion_is_summed_and_reaches_the_locked_surface() {
        let (mut runtime, surface, heard) = runtime();
        let _log = install(Ok(Some(PointerLockMode::Locked)));
        surface.request_pointer_lock().unwrap();
        for (dx, dy) in [(3.0, -1.0), (0.5, 2.25), (-10.0, 0.0)] {
            runtime.main.note_device_event(&device_motion(dx, dy));
        }
        // Not motion: ignored.
        runtime.main.note_device_event(&DeviceEvent::MouseWheel {
            delta: winit::event::MouseScrollDelta::LineDelta(0.0, 1.0),
        });
        let motion = runtime
            .main
            .take_pointer_motion()
            .expect("the turn's motion");
        assert!(runtime.main.take_pointer_motion().is_none(), "taken once");
        runtime.main.app.handle_event(motion, (640, 480), 1.0);
        assert_eq!(*heard.borrow(), [(-6.5, 1.25)]);
        crate::window::set_pointer_lock_host(None);
    }

    /// A confined pointer that moved while locked: when the lock ends, its
    /// real position is the app's next move, so hover is not left stale.
    #[test]
    fn the_position_a_lock_held_back_is_restored_when_it_ends() {
        let (mut runtime, surface, _heard) = runtime();
        let _log = install(Ok(Some(PointerLockMode::Confined)));
        surface.request_pointer_lock().unwrap();
        runtime.main.held_pointer_move = Some((310.0, 205.0, SurfacePointer::mouse(false)));
        runtime.main.restore_held_pointer_move();
        assert!(
            runtime.main.pending_pointer_move.is_none(),
            "still locked: held"
        );
        crate::window::exit_pointer_lock();
        runtime.main.restore_held_pointer_move();
        assert_eq!(
            runtime.main.pending_pointer_move.map(|(x, y, _)| (x, y)),
            Some((310.0, 205.0))
        );
        assert!(runtime.main.held_pointer_move.is_none());
        crate::window::set_pointer_lock_host(None);
    }

    /// Hiding the window destroys it: the lock is given back first, through
    /// it, and nothing can lock until a window is shown again.
    #[test]
    fn hiding_the_window_releases_the_lock() {
        let (mut runtime, _surface, _heard) = runtime();
        let log = install(Ok(Some(PointerLockMode::Locked)));
        request_pointer_lock().unwrap();
        runtime.hide_window();
        assert_eq!(pointer_lock(), None);
        assert_eq!(log.borrow().unlocks, 1);
        assert_eq!(request_pointer_lock(), Err(PointerLockError::Unsupported));
    }
}

/// #2416 stage 3a: the DevTools window is one of the runtime's windows, and a
/// close asked of a window is that window's.
#[cfg(test)]
mod windows_3a_tests {
    use super::*;

    fn component(scope: &mut RenderScope) -> NodeHandle {
        scope.create_element("div")
    }

    fn devtools_window() -> ShellWindow {
        ShellWindow::new(RinchApp::new(component), "DevTools", 480, 600)
    }

    #[test]
    fn the_roles_are_the_open_windows_main_first() {
        let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        assert_eq!(runtime.roles().collect::<Vec<_>>(), [WindowRole::Main]);
        runtime.devtools = Some(devtools_window());
        assert_eq!(
            runtime.roles().collect::<Vec<_>>(),
            [WindowRole::Main, WindowRole::DevTools]
        );
        // Each role names its own window.
        assert_eq!(runtime.w(WindowRole::Main).title, "t");
        assert_eq!(runtime.w(WindowRole::DevTools).title, "DevTools");
    }

    #[test]
    fn the_main_windows_close_ends_the_loop() {
        let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        runtime.devtools = Some(devtools_window());
        assert!(runtime.close_soon(WindowRole::Main));
        assert!(!runtime.devtools_closing, "DevTools is not closed with it");
        runtime.settle_devtools_close();
        assert!(runtime.devtools.is_some());
    }

    #[test]
    fn devtools_close_closes_devtools_after_the_handler() {
        let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        runtime.ensure_devtools_store();
        runtime.devtools_store.unwrap().visible.set(true);
        runtime.devtools = Some(devtools_window());
        assert!(
            !runtime.close_soon(WindowRole::DevTools),
            "the loop goes on"
        );
        // Still there for the rest of the handler that asked …
        assert!(runtime.devtools.is_some());
        assert_eq!(runtime.w(WindowRole::DevTools).title, "DevTools");
        // … and gone once it returns.
        runtime.settle_devtools_close();
        assert!(runtime.devtools.is_none());
        assert!(!runtime.devtools_closing);
        assert!(!runtime.devtools_store.unwrap().visible.get());
        assert_eq!(runtime.roles().collect::<Vec<_>>(), [WindowRole::Main]);
    }
}
