//! The main window's attributes, geometry, layer-shell state, creation, hiding and showing, and its renderer.

use super::*;

/// Ask again in a second whether an output has appeared for a layer surface
/// the compositor closed with none left (#1523).
pub(super) fn arm_layer_output_retry() {
    rinch_core::set_timeout(1000, || {
        send_native_event(RinchNativeEvent::RetryLayerOutput)
    });
}

impl ShellWindow {
    /// The attributes of the main window, for its first creation and for
    /// the re-creation [`Self::show_window`] makes, with the overlay steps
    /// (#1519) folded in. `first` installs the Wayland icon and reports an icon
    /// that does not decode; a re-creation does neither again.
    /// `first_appearance` is the window's first time on screen, which takes
    /// the requested position when no geometry was saved.
    pub(super) fn main_window_attributes(
        &self,
        steps: WindowSteps,
        backend: Backend,
        first: bool,
        first_appearance: bool,
    ) -> WindowAttributes {
        let mut window_attrs = WindowAttributes::default()
            .with_title(&self.title)
            .with_surface_size(winit::dpi::LogicalSize::new(self.width, self.height));

        // app_id on Wayland (always resolved) and WM_CLASS on X11 (only an
        // explicit app_id; winit's argv[0] default otherwise, as before).
        #[cfg(target_os = "linux")]
        let app_id: Option<String> = self
            .app
            .window_props
            .as_ref()
            .map(|props| resolved_app_id(props.app_id.as_deref()));
        #[cfg(target_os = "linux")]
        if let Some(id) = &app_id {
            *WINDOW_APP_ID.lock().unwrap_or_else(|p| p.into_inner()) = Some(id.clone());
        }
        #[cfg(not(target_os = "linux"))]
        let app_id: Option<String> = None;
        let x11_class: Option<String> = self
            .app
            .window_props
            .as_ref()
            .and_then(|props| props.app_id.clone());
        if let Some(props) = &self.app.window_props {
            if props.borderless {
                window_attrs = window_attrs.with_decorations(false);
            }
            if props.transparent {
                window_attrs = window_attrs.with_transparent(true);
            }
            if !props.resizable {
                window_attrs = window_attrs.with_resizable(false);
            }
            // Every raised level is winit's `AlwaysOnTop`; macOS sets the
            // exact `StatusBar` / `ScreenSaver` level after creation (#1549).
            if props.effective_level() != rinch_core::element::WindowLevel::Normal {
                window_attrs =
                    window_attrs.with_window_level(winit::window::WindowLevel::AlwaysOnTop);
            }
            if let Some(icon_data) = props.icon {
                // Set window icon (works on X11 and Windows)
                match load_window_icon(icon_data) {
                    Ok(icon) => {
                        window_attrs = window_attrs.with_window_icon(Some(icon));
                    }
                    Err(e) if first => {
                        tracing::warn!("Failed to load window icon: {}", e);
                    }
                    Err(_) => {}
                }
                // On Wayland, write icon to a temp file and install a .desktop entry
                // so the compositor can find it via the app_id.
                #[cfg(target_os = "linux")]
                if first {
                    let app_id = resolved_app_id(props.app_id.as_deref());
                    install_wayland_icon(&app_id, icon_data);
                }
            }
        }
        // The geometry the window had when it was hidden (#1546), or the
        // requested `WindowProps::x`/`y` on its first appearance — a first
        // creation, or the first show after a hidden start (#1520).
        let requested = self.requested_position().map(|r| r.at);
        let requested_maximized = self.app.window_props.as_ref().is_some_and(|p| p.maximized);
        let window_attrs = self.saved_geometry.apply(
            window_attrs,
            requested,
            requested_maximized,
            first_appearance,
        );
        // A Wayland layer surface: the app's own (#2413), or an overlay
        // (#1523) placed by its margins: the saved position, or the
        // requested one on a first appearance.
        let layer = steps.wayland_layer.then(|| {
            LayerSpec::for_window(
                self.app
                    .window_props
                    .as_ref()
                    .unwrap_or(&Default::default()),
                self.layer_position(first_appearance),
                self.output_override.clone(),
                app_id.as_deref().unwrap_or("rinch"),
            )
        });
        overlay_window::apply_to_attributes(
            window_attrs,
            steps,
            backend,
            app_id.as_deref(),
            x11_class.as_deref(),
            layer.as_ref(),
        )
    }

    /// The overlay steps (#1519) for the main window on `backend`. The
    /// `first` creation starts click-through from
    /// `WindowProps::click_through`; a re-creation by `show_window` keeps the
    /// state the runtime toggle left.
    pub(super) fn main_window_steps(&mut self, backend: Backend, first: bool) -> WindowSteps {
        if first {
            self.click_through = self
                .app
                .window_props
                .as_ref()
                .is_some_and(|p| p.click_through);
        }
        match &self.app.window_props {
            Some(props) => WindowSteps::plan(props, self.click_through, backend),
            None => WindowSteps::plan(&Default::default(), self.click_through, backend),
        }
    }

    /// Record a click-through toggle (`set_click_through_current_window`) so
    /// a window re-created by `show_window` comes back the same. Returns the
    /// state to apply to the live window.
    pub(super) fn note_click_through(&mut self, click_through: bool) -> bool {
        self.click_through = click_through;
        self.click_through
    }

    /// Keep the saved normal geometry and the published one current
    /// (#1546). Only for a window that is on screen: a hidden start's sizing
    /// window sits wherever it was created.
    pub(super) fn note_geometry(&mut self, window: &dyn winit::window::Window) {
        if !self.shown_once {
            return;
        }
        self.take_reading(
            window_geometry::WindowReading::of(window),
            monitor_name(window),
        );
    }

    /// [`Self::note_geometry`] past the window: a reading of the live main
    /// window, through the wait for a maximize the platform has not reported
    /// yet.
    /// End the maximize wait with one reading of the live main window,
    /// believed: when its grace ends (a window the window manager never
    /// maximized, moved inside the grace and then left alone, has had no
    /// reading since) and at close (what `on_close_requested` reads).
    pub(super) fn settle_maximize_wait(&mut self) {
        if self.maximize_wait == window_geometry::MaximizeWait::Done {
            return;
        }
        let read = self.window.as_ref().filter(|_| self.shown_once).map(|w| {
            (
                window_geometry::WindowReading::of(w.raw()),
                monitor_name(w.raw()),
            )
        });
        self.settle_maximize_wait_with(read);
    }

    /// [`Self::settle_maximize_wait`] past the window.
    fn settle_maximize_wait_with(
        &mut self,
        read: Option<(window_geometry::WindowReading, Option<String>)>,
    ) {
        self.maximize_wait = window_geometry::MaximizeWait::Done;
        if let Some((reading, monitor)) = read {
            self.take_reading(reading, monitor);
        }
    }

    fn take_reading(&mut self, reading: window_geometry::WindowReading, monitor: Option<String>) {
        let reading = self
            .maximize_wait
            .filter(reading, std::time::Instant::now());
        self.saved_geometry.note(&reading);
        window_geometry::publish_window(&reading, monitor);
        self.publish_layer_position();
    }

    /// Where a layer surface goes (#1523): the saved position (logical,
    /// output-relative), or `WindowProps::x`/`y` on a first appearance;
    /// `None` lets the compositor centre it.
    fn layer_position(&self, first_appearance: bool) -> Option<(f64, f64)> {
        match self.saved_geometry.position {
            Some(Position::Logical(p)) => return Some((p.x, p.y)),
            Some(Position::Physical(p)) => return Some((p.x as f64, p.y as f64)),
            None => {}
        }
        let props = self.app.window_props.as_ref()?;
        first_appearance
            .then(|| Some((props.x? as f64, props.y? as f64)))
            .flatten()
    }

    /// After a main window is created: publish what it got (#1523) and keep
    /// the layer-shell state of a layer surface.
    fn note_surface_kind(&mut self, window: &dyn winit::window::Window, first_appearance: bool) {
        let kind = crate::shell::layer_shell::surface_kind_of(window);
        crate::shell::layer_shell::publish_surface_kind(kind);
        self.take_surface_kind(kind, first_appearance);
    }

    /// [`Self::note_surface_kind`] past the window: the layer-shell state
    /// for a window that got `kind` — none for a toplevel (on GNOME too, so
    /// moves and resizes go to the compositor), the app's own surface
    /// (#2413), or an overlay at its position.
    fn take_surface_kind(&mut self, kind: WindowSurfaceKind, first_appearance: bool) {
        let app_placed = self
            .app
            .window_props
            .as_ref()
            .is_some_and(|p| p.wayland_layer.is_some());
        self.layer = (kind == WindowSurfaceKind::LayerShell).then(|| {
            if app_placed {
                return LayerState::app_placed();
            }
            let position = self.layer_position(first_appearance);
            if let Some((x, y)) = position {
                self.saved_geometry.position = Some(Position::Logical(LogicalPosition::new(x, y)));
            }
            LayerState::new(position)
        });
    }

    /// Publish a layer surface's position, which no window reading has
    /// (Wayland reports none): its margins (#1523).
    fn publish_layer_position(&self) {
        if let Some(position) = self.layer.as_ref().and_then(|l| l.position) {
            window_geometry::publish_hidden(Some(position), None);
        }
    }

    /// Move the live layer surface to `position` (logical, on its output),
    /// anchored top-left.
    pub(super) fn place_layer(&mut self, position: (f64, f64)) {
        let Some(layer) = self.layer.as_mut() else {
            return;
        };
        // The app's own surface (#2413) is placed by its anchors and margins.
        if layer.app_placed {
            tracing::debug!(
                "a layer surface with WindowProps::wayland_layer has no position to set: \
                 change its margins with set_current_window_layer"
            );
            return;
        }
        layer.place(position);
        #[cfg(target_os = "linux")]
        if let Some(w) = self.window.as_ref() {
            use winit::platform::wayland::{LayerShellAnchor, WindowExtWayland};
            let raw = w.raw();
            raw.set_layer_shell_anchor(LayerShellAnchor::TOP_LEFT);
            // Confirmed by a `WindowEvent::Moved` (`layer_moved`).
            raw.set_layer_shell_margins(crate::shell::layer_shell::margins(position));
        }
        self.saved_geometry.position = Some(Position::Logical(LogicalPosition::new(
            position.0, position.1,
        )));
        window_geometry::publish_hidden(Some(position), None);
    }

    /// Start a move or resize of a layer surface at the pointer (#1523): the
    /// compositor has none. `false` when the window is no layer surface.
    pub(super) fn start_layer_gesture(&mut self, kind: GestureKind) -> bool {
        let Some(layer) = self.layer.as_ref() else {
            return false;
        };
        // The app places its own surface (#2413): no gesture moves it.
        if layer.app_placed {
            tracing::debug!(
                "window drag/resize ignored: a layer surface with WindowProps::wayland_layer \
                 is placed by its anchors and margins"
            );
            return true;
        }
        let Some(w) = self.window.as_ref() else {
            return false;
        };
        // Only while the primary button is held: nothing but its release ends
        // the gesture, so one started without it would follow the hovering
        // pointer (a compositor refuses a move with no live press, too).
        if !self.pressed_pointers.primary_down() {
            tracing::debug!("window drag/resize refused: no button is down");
            return true;
        }
        let scale = w.scale_factor();
        let size = w.raw().surface_size().to_logical::<f64>(scale);
        let size = (size.width, size.height);
        let position = match self.layer.as_ref().and_then(|l| l.position) {
            Some(p) => p,
            // Centred by the compositor: read where, from the output the
            // surface is on.
            None => {
                let output = w.raw().current_monitor().and_then(|m| {
                    let mode = m.current_video_mode()?.size();
                    Some((mode.width as f64 / scale, mode.height as f64 / scale))
                });
                let p = crate::shell::layer_shell::centred_position(output.unwrap_or(size), size);
                self.place_layer(p);
                if let Some(layer) = self.layer.as_mut() {
                    layer.place_centred(p);
                }
                p
            }
        };
        let (cx, cy) = self.app.cursor_pos.unwrap_or((0.0, 0.0));
        if let Some(layer) = self.layer.as_mut() {
            layer.gesture = Some(Gesture::start(kind, (cx as f64, cy as f64), position, size));
        }
        true
    }
}

impl RinchRuntime {
    /// Follow the pointer, at `local` (logical, over the surface), with the
    /// layer gesture. `false` when there is none.
    pub(super) fn update_layer_gesture(
        &mut self,
        local: (f64, f64),
        event_loop: &dyn ActiveEventLoop,
    ) -> bool {
        let Some(resize) = self.main.follow_layer_gesture(local) else {
            return false;
        };
        if let Some(new_size) = resize {
            let current = self.main.window.as_ref().map(|w| {
                let s = w.raw().surface_size().to_logical::<f64>(w.scale_factor());
                (s.width, s.height)
            });
            if current != Some(new_size) {
                let size = Size::Logical(LogicalSize::new(new_size.0, new_size.1));
                if let Some(w) = &self.main.window {
                    let id = w.raw().id();
                    if let Some(physical) = w.raw().request_surface_size(size) {
                        self.window_event(event_loop, id, WindowEvent::SurfaceResized(physical));
                    }
                }
            }
        }
        true
    }
}

impl ShellWindow {
    /// The window-free half of [`RinchRuntime::update_layer_gesture`]: move the
    /// surface where the gesture puts the pointer at `local`, read against
    /// where the compositor has the surface ([`LayerState`]). `None` when no
    /// gesture runs; `Some(Some(size))` for a resize, which the caller asks
    /// the window for.
    fn follow_layer_gesture(&mut self, local: (f64, f64)) -> Option<Option<(f64, f64)>> {
        let layer = self.layer.as_ref()?;
        let kind = layer.gesture?.kind;
        let (new_position, new_size) = layer.follow(local)?;
        if Some(new_position) != layer.position {
            self.place_layer(new_position);
        }
        Some(matches!(kind, GestureKind::Resize(_)).then_some(new_size))
    }

    /// Whether the GPU surface is made to composite with its alpha (#1523):
    /// for a `WindowProps::transparent` window.
    #[cfg_attr(not(feature = "gpu"), allow(dead_code))]
    fn gpu_surface_transparent(&self) -> bool {
        self.app.is_transparent()
    }

    /// A layer surface's `WindowEvent::Moved` (#1523): the compositor has
    /// processed the move to `physical`, and pointer positions after this
    /// are over the surface there.
    pub(super) fn layer_moved(&mut self, physical: (i32, i32)) {
        let scale = self.scale_factor();
        if let Some(layer) = self.layer.as_mut() {
            layer.moved(crate::shell::layer_shell::moved_origin(physical, scale));
        }
    }

    /// `set_current_window_layer` (#2413): the new configuration replaces
    /// `WindowProps::wayland_layer`. A live layer surface takes the changes
    /// through the fork's setters, or is made again for a new output; a live
    /// window that was not asked to be a layer surface is made again as one.
    pub(super) fn set_layer_surface(
        &mut self,
        surface: rinch_core::element::LayerSurface,
        event_loop: &dyn ActiveEventLoop,
        native_menu: Option<&muda::Menu>,
    ) {
        let backend = Backend::of(event_loop);
        let position = self.layer.and_then(|l| l.position);
        let props = self.app.window_props.get_or_insert_with(Default::default);
        let was_layer = crate::shell::layer_shell::uses_layer_shell(props, backend);
        let was = LayerSpec::for_window(props, position, self.output_override.take(), "").surface;
        props.wayland_layer = Some(surface.clone());
        if self.window.is_none() {
            return;
        }
        if self.layer.is_none() {
            // A toplevel asked for (becomes a layer surface), or given for a
            // layer surface (GNOME: stays one).
            if !was_layer && backend == Backend::Wayland {
                self.hide_window(native_menu);
                self.show_window(event_loop, native_menu);
            }
            return;
        }
        match crate::shell::layer_shell::layer_update(&was, &surface) {
            crate::shell::layer_shell::LayerUpdate::Recreate => {
                self.hide_window(native_menu);
                self.show_window(event_loop, native_menu);
            }
            crate::shell::layer_shell::LayerUpdate::Apply(sets) => {
                crate::shell::layer_shell::warn_unassignable(&surface);
                self.layer = Some(LayerState::app_placed());
                if let Some(w) = &self.window {
                    crate::shell::layer_shell::apply_layer_sets(w.raw(), &sets);
                }
            }
        }
    }

    /// The compositor closed the layer surface (#1523): its output was
    /// unplugged or disabled. Make it again, at the same position and size,
    /// on the output the compositor picks — the named one may be gone, so
    /// the name is dropped until `set_current_window_output` sets one again.
    /// With no output left, wait for one.
    pub(super) fn layer_surface_closed(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        native_menu: Option<&muda::Menu>,
    ) {
        tracing::warn!(
            "the compositor closed the layer surface (its output went away); making it again"
        );
        self.output_override = Some(None);
        self.hide_window(native_menu);
        if event_loop.available_monitors().next().is_some() {
            self.show_window(event_loop, native_menu);
        } else {
            self.layer_awaiting_output = true;
            arm_layer_output_retry();
        }
    }

    /// Whether the main window asked to stay on top: `always_on_top`, or
    /// any raised `WindowProps::level` (#1549).
    pub(super) fn main_window_always_on_top(&self) -> bool {
        self.app
            .window_props
            .as_ref()
            .is_some_and(|p| p.effective_level() != rinch_core::element::WindowLevel::Normal)
    }

    /// Take a title `set_window_title` stored (#1770) into `self.title`, so a
    /// window created later (or re-created by `show_window`) has it, and set
    /// it on the live window.
    pub(super) fn take_stored_title(&mut self) {
        if let Some(title) = crate::window::take_stored_title() {
            if let Some(w) = &self.window {
                w.raw().set_title(&title);
            }
            self.title = title;
        }
    }

    /// `WindowProps::x`/`y`, when the window's top would be reachable on the
    /// monitors connected now (`self.monitors`); otherwise the system places
    /// the window, so a position saved on a monitor since unplugged never
    /// opens it out of reach. `at` is where winit is asked to put it
    /// ([`window_geometry::place_requested`]: desktop pixels, resolved in the
    /// scale of the monitor the position is on).
    fn requested_position(&self) -> Option<RequestedPosition> {
        let props = self.app.window_props.as_ref()?;
        let (x, y) = (props.x?, props.y?);
        let size = Size::Logical(LogicalSize::new(self.width as f64, self.height as f64));
        let placed = window_geometry::place_requested(
            (x as f64, y as f64),
            size,
            props.monitor.as_deref(),
            &self.monitors,
        )?;
        Some(RequestedPosition {
            logical: (x as f64, y as f64),
            at: placed.at,
            monitor: placed.monitor,
        })
    }

    /// The geometry half of putting the main window on screen, before winit
    /// creates it: the monitors connected now (`monitors`), a saved position
    /// out of reach on them dropped (one unplugged while the window was
    /// hidden), and on its first appearance — a creation, or the first show
    /// after a hidden start (#1520) — the geometry it is created with
    /// published as the one an un-maximize returns to: a window that opens
    /// maximized has had no normal geometry to read. `on_screen` is false for
    /// a hidden start's sizing window. Returns whether this is the first
    /// appearance.
    fn begin_appearance(
        &mut self,
        monitors: Vec<window_geometry::MonitorRect>,
        on_screen: bool,
    ) -> bool {
        self.monitors = monitors;
        if !on_screen {
            return false;
        }
        let first = !std::mem::replace(&mut self.shown_once, true);
        if let Some(position) = self.saved_geometry.position {
            let size = self
                .saved_geometry
                .size
                .unwrap_or(Size::Logical(LogicalSize::new(
                    self.width as f64,
                    self.height as f64,
                )));
            if !window_geometry::reachable(position, Some(size), &self.monitors) {
                self.saved_geometry.position = None;
            }
        }
        if first {
            let requested = self.requested_position();
            window_geometry::publish_created(
                requested.as_ref().map(|r| r.logical),
                requested.and_then(|r| r.monitor),
                (self.width as f64, self.height as f64),
                self.app.window_props.as_ref().is_some_and(|p| p.maximized),
            );
        }
        first
    }

    pub(super) fn create_window(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        native_menu: Option<&muda::Menu>,
    ) {
        // A title set before the loop started (#1770).
        self.take_stored_title();
        let hidden = starts_hidden(self.app.window_props.as_ref());
        self.begin_appearance(connected_monitors(event_loop), !hidden);
        let backend = Backend::of(event_loop);
        let steps = self.main_window_steps(backend, true);
        let window_attrs = hidden_start_attributes(
            self.main_window_attributes(steps, backend, true, true),
            hidden,
        );
        self.maximize_wait = window_geometry::MaximizeWait::after_create(
            &window_attrs,
            steps.wayland_layer,
            std::time::Instant::now(),
        );

        let window = event_loop
            .create_window(window_attrs)
            .expect("Failed to create window");
        // A hidden start skips the post-creation steps: they would show a
        // window created hidden (`AfterCreate::Show`), and they run on the
        // window `show_window` creates instead.
        if !hidden {
            overlay_window::after_create(&*window, steps, self.main_window_always_on_top());
            self.note_surface_kind(&*window, true);
            self.note_geometry(&*window);
        }

        let size = window.surface_size();
        let scale = window.scale_factor();

        // Create the GPU renderer when the choice wants one and it starts; the
        // software renderer is created lazily by the first software paint. A
        // window that starts hidden gets none: it is destroyed before it could
        // present, and `show_window` creates the renderer with the window it
        // creates.
        #[cfg(feature = "gpu")]
        if !hidden {
            self.renderer_started(start_gpu_renderer(&*window, self.gpu_surface_transparent()));
        }
        #[cfg(not(feature = "gpu"))]
        {
            announce_software_renderer();
            if !hidden {
                self.note_renderer_pacing();
            }
        }

        // Attach native menu bar to the window if configured
        if let Some(menu) = native_menu {
            crate::menu::attach_menu_to_window(menu, &*window);
        }

        // Wrap the winit window
        let winit_window = WinitWindow::new(window);
        self.window = Some(winit_window);

        // Register direct redraw callback so render surfaces can request
        // repaints without routing through the event loop queue.
        let window_arc = self.window.as_ref().unwrap().window.clone();
        crate::render_surface::set_redraw_callback(Arc::new(move || {
            window_arc.request_redraw();
        }));
        self.install_pointer_lock_host();

        // Mount the component at the *logical* viewport size — `size` above is
        // the physical surface size (see `to_logical`). The display scale goes
        // in first so the initial style resolution sees the right
        // `device_pixel_ratio` (issue #211) — winit does not promise a
        // `ScaleFactorChanged` at window creation.
        self.app.set_device_pixel_ratio(scale);
        let logical = to_logical((size.width, size.height), scale);
        self.app.mount_component(logical.0 as f32, logical.1 as f32);

        // `WindowProps::visible: false` (issue #1520): the app runs with no
        // window on screen — a tray-only app — until `show_current_window`.
        // The window above existed only to give the root its size and display
        // scale; the root is mounted now, once, and `show_window` re-creates
        // the OS window over the same document without mounting it again.
        // Destroyed rather than kept invisible for the reason `hide_window`
        // gives: Wayland has no invisible toplevel. No frame was requested and
        // no renderer made, so nothing was ever presented.
        if hidden {
            self.hide_window(native_menu);
            return;
        }

        // Request initial draw
        self.window.as_ref().unwrap().request_redraw();
    }

    /// Hide the window by destroying the OS window and GPU surface.
    ///
    /// On Wayland, `set_visible(false)` is a no-op, so we must destroy the
    /// actual winit Window (which destroys the xdg_toplevel) to make it
    /// disappear. The app state and DOM are preserved.
    pub(super) fn hide_window(&mut self, native_menu: Option<&muda::Menu>) {
        if let Some(w) = &self.window {
            // Keep where the window is and how big (#1546): `show_window`
            // creates the new one with it. Not the sizing window of a hidden
            // start (#1520), which was never on screen and sits wherever the
            // system put it.
            if self.shown_once {
                let reading = self.maximize_wait.filter(
                    window_geometry::WindowReading::of(w.raw()),
                    std::time::Instant::now(),
                );
                self.saved_geometry.record(&reading);
                window_geometry::publish_window(&reading, monitor_name(w.raw()));
                self.publish_layer_position();
            }
            // The menu bar leaves the window with it (Windows: muda keeps a
            // list of the HWNDs it serves, and a recycled HWND would be
            // refused at the next attach); `show_window` attaches it again.
            if let Some(menu) = native_menu {
                crate::menu::detach_menu_from_window(menu, w.raw());
            }
        }
        // A window hidden to the tray has no focus to report losing (#1770).
        self.app.publish_window_state(crate::window::WindowFacts {
            focused: Some(false),
            ..Default::default()
        });
        // Clear the direct redraw callback before dropping the window.
        crate::render_surface::clear_redraw_callback();
        // A lock ends with its window (#2092): released through the window,
        // then no host until one is shown again.
        crate::window::set_pointer_lock_host(None);
        // Drop renderer first — it holds a reference to the window surface.
        #[cfg(feature = "gpu")]
        drop(self.renderer.take());
        drop(self.soft_renderer.take());
        drop(self.window.take());
        // Nothing presents until it is shown again (#2095).
        self.pacing = None;
        crate::window::publish_pacing(None);
        // Its position stays in `saved_geometry`; a gesture ends with it.
        self.layer = None;
        // A re-created window starts on the default cursor.
        self.applied_cursor = None;
    }

    /// Show a previously hidden window by recreating the OS window and GPU surface.
    ///
    /// Rebuilds the winit Window and wgpu renderer, then triggers a repaint
    /// of the existing DOM.
    pub(super) fn show_window(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        native_menu: Option<&muda::Menu>,
    ) {
        if self.window.is_some() {
            return; // Already visible
        }

        let backend = Backend::of(event_loop);
        let steps = self.main_window_steps(backend, false);
        // After a hidden start (#1520) the first show is the window's first
        // appearance, so it takes the requested position as a first creation
        // does; a restore from the tray comes back where it was hidden
        // (#1546), with the size it had.
        let first_show = self.begin_appearance(connected_monitors(event_loop), true);
        let window_attrs = self.main_window_attributes(steps, backend, false, first_show);
        self.maximize_wait = window_geometry::MaximizeWait::after_create(
            &window_attrs,
            steps.wayland_layer,
            std::time::Instant::now(),
        );

        let window = event_loop
            .create_window(window_attrs)
            .expect("Failed to recreate window");
        overlay_window::after_create(&*window, steps, self.main_window_always_on_top());
        self.note_surface_kind(&*window, first_show);

        #[cfg(feature = "gpu")]
        self.renderer_started(start_gpu_renderer(&*window, self.gpu_surface_transparent()));
        #[cfg(not(feature = "gpu"))]
        self.note_renderer_pacing();

        // The new window gets the menu bar the first one had (#1533: on
        // Windows the bar is per HWND, so a re-created window had none, and
        // its chords were skipped by rinch's matcher while no bar fired them).
        if let Some(menu) = native_menu {
            crate::menu::attach_menu_to_window(menu, &*window);
            // Windows: `SetMenu` keeps the outer rect and takes the bar out of
            // the client area, so the content would lose the bar's height on
            // every hide / show. Ask for the recorded content size again.
            #[cfg(target_os = "windows")]
            if let Some(size) = self.saved_geometry.size {
                let _ = window.request_surface_size(size);
            }
        }
        self.note_geometry(&*window);

        let winit_window = WinitWindow::new(window);
        self.window = Some(winit_window);

        // Update direct redraw callback for the new window.
        let window_arc = self.window.as_ref().unwrap().window.clone();
        crate::render_surface::set_redraw_callback(Arc::new(move || {
            window_arc.request_redraw();
        }));
        self.install_pointer_lock_host();

        // Trigger a full repaint of the existing DOM: a new surface, and
        // nothing named what it should show.
        self.app.mark_scene_dirty();
        self.window.as_ref().unwrap().request_redraw();
    }

    /// Take the main window's new renderer (`None`: it presents with
    /// software), note what it does with vsync on and off, and publish what
    /// it does now (#2095): the GPU renderer's surface says; the software
    /// renderer has no present mode to switch.
    #[cfg(feature = "gpu")]
    fn renderer_started(&mut self, mut renderer: Option<WgpuRenderer>) {
        if let Some(r) = renderer.as_mut() {
            // The app's window hands its frames to `on_frame_composited`
            // (#2215); the DevTools window's renderer is made elsewhere.
            r.hands_over_frames = true;
        }
        self.renderer = renderer;
        self.note_renderer_pacing();
    }

    /// [`Self::renderer_started`]'s second half; a build without `gpu` has
    /// only the software renderer, and calls it alone.
    fn note_renderer_pacing(&mut self) {
        #[cfg(feature = "gpu")]
        let outcomes = self
            .renderer
            .as_ref()
            .map_or(crate::window::PacingOutcomes::UNSUPPORTED, |r| {
                r.pacing_outcomes()
            });
        #[cfg(not(feature = "gpu"))]
        let outcomes = crate::window::PacingOutcomes::UNSUPPORTED;
        self.pacing = Some(outcomes);
        crate::window::publish_pacing(self.pacing);
    }
}

impl RinchRuntime {
    /// Create the main window ([`ShellWindow::create_window`]), with the
    /// app's native menu bar.
    pub(super) fn create_window(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.main
            .create_window(event_loop, self.native_menu.as_ref());
    }

    /// Hide the main window ([`ShellWindow::hide_window`]), taking the native
    /// menu bar off it.
    pub(super) fn hide_window(&mut self) {
        self.main.hide_window(self.native_menu.as_ref());
    }

    /// Show the main window again ([`ShellWindow::show_window`]), with the
    /// native menu bar.
    pub(super) fn show_window(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.main.show_window(event_loop, self.native_menu.as_ref());
    }

    /// Apply `rinch::window::set_vsync` (#2095) to every GPU surface this
    /// runtime presents to — the main window's and the DevTools window's, so
    /// a DevTools window repainting beside an uncapped main window does not
    /// pace the loop to its refresh — publish what the main window's does,
    /// and redraw both, so the new mode takes effect without waiting for
    /// something else to ask for a frame. Each surface is configured at its
    /// next present ([`WgpuRenderer::set_vsync`]).
    pub(super) fn apply_vsync(&mut self) {
        #[cfg(feature = "gpu")]
        super::desktop::apply_vsync_to(
            crate::window::vsync_requested(),
            [self.main.renderer.as_mut(), self.devtools_renderer.as_mut()],
        );
        crate::window::publish_pacing(self.main.pacing);
        for window in [self.main.window.as_ref(), self.devtools_window.as_ref()]
            .into_iter()
            .flatten()
        {
            window.request_redraw();
        }
    }
}

/// Whether the app's window starts hidden: `WindowProps::visible` is `false`
/// (issue #1520). Absent props mean a visible window.
fn starts_hidden(props: Option<&rinch_core::element::WindowProps>) -> bool {
    props.is_some_and(|p| !p.visible)
}

/// The first creation's attributes for a hidden start (#1520): `visible:
/// false` whatever the overlay steps (#1519) chose. That window only sizes the
/// root and is destroyed before anything is presented (X11 and Windows honour
/// `with_visible(false)`; Wayland maps no toplevel until a buffer is attached,
/// and none ever is). The overlay steps' own creation visibility is about the
/// window `show_window` makes later — on macOS a non-activating panel is
/// created *visible*, since a hidden-then-`set_visible` show would make it key —
/// and that re-creation takes the overlay attributes unchanged. A start that
/// is not hidden is left exactly as the overlay steps built it.
fn hidden_start_attributes(attrs: WindowAttributes, hidden: bool) -> WindowAttributes {
    if hidden {
        attrs.with_visible(false)
    } else {
        attrs
    }
}

/// Whether `can_create_surfaces` creates the main window: only when there is
/// none **and** nothing is mounted. A mounted document with no window was
/// hidden on purpose (started hidden, #1520, or sent to the tray), and
/// creating the window here would mount the root a second time.
pub(super) fn creates_window_on_surfaces(has_window: bool, mounted: bool) -> bool {
    !has_window && !mounted
}

#[cfg(test)]
mod hidden_start_tests {
    use super::*;
    use rinch_core::element::WindowProps;

    /// #1520: `visible: false` starts hidden; the default and absent props
    /// do not.
    #[test]
    fn only_visible_false_starts_hidden() {
        assert!(!starts_hidden(None));
        assert!(!starts_hidden(Some(&WindowProps::default())));
        assert!(starts_hidden(Some(&WindowProps {
            visible: false,
            ..Default::default()
        })));
    }

    /// #1520 with #1519: a hidden start creates its sizing window invisible
    /// even where the overlay steps would create it visible (a macOS
    /// non-activating panel), and leaves every other start as the steps built
    /// it — an ordinary window visible, an X11 overlay created hidden.
    #[test]
    fn a_hidden_start_overrides_the_overlay_visibility_and_nothing_else() {
        use super::overlay_window::{Backend, WindowSteps, apply_to_attributes};
        let panel = WindowProps {
            non_activating: true,
            ..Default::default()
        };
        let mac = WindowSteps::plan(&panel, false, Backend::MacOs);
        let built = || {
            apply_to_attributes(
                WindowAttributes::default(),
                mac,
                Backend::MacOs,
                None,
                None,
                None,
            )
        };
        assert!(built().visible, "the macOS panel itself is created visible");
        assert!(!hidden_start_attributes(built(), true).visible);
        assert!(
            !hidden_start_attributes(built(), true).active,
            "the steps' other attributes stay"
        );
        assert!(hidden_start_attributes(built(), false).visible);
        let x11 = WindowSteps::plan(&panel, false, Backend::X11);
        let x11_attrs = apply_to_attributes(
            WindowAttributes::default(),
            x11,
            Backend::X11,
            None,
            None,
            None,
        );
        assert!(
            !hidden_start_attributes(x11_attrs, false).visible,
            "the X11 overlay is created hidden as before"
        );
        assert!(hidden_start_attributes(WindowAttributes::default(), false).visible);
    }

    /// #1520: after a hidden start the document is mounted and the window
    /// gone, and a later `can_create_surfaces` must not create (and mount)
    /// again. Only the first call, with nothing mounted, creates one.
    #[test]
    fn surfaces_create_the_window_only_before_anything_is_mounted() {
        assert!(creates_window_on_surfaces(false, false), "first start");
        assert!(
            !creates_window_on_surfaces(false, true),
            "hidden start or tray: mounted, no window"
        );
        assert!(!creates_window_on_surfaces(true, true), "already shown");
    }
}

// ── Renderer choice ──────────────────────────────────────────────────────────

/// How soon the loop looks again for Vello's counters of a frame the GPU has
/// not finished (#2071): half a 60 Hz frame. A window that overflowed is drawn
/// again this much after the GPU finishes, at the worst.
#[cfg(feature = "gpu")]
pub(super) const VELLO_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(8);

/// The GPU renderer for `window`, or `None` to present with software: when
/// software was chosen (and the app did not configure the GPU device itself,
/// see `renderer::tries_gpu`), or when the GPU would not start under
/// [`Renderer::Auto`](super::renderer::Renderer::Auto), a software adapter
/// included (`renderer::adapter_use`, #2436). Also records which one
/// presents (`renderer::gpu_presenting`) and logs it.
///
/// # Panics
///
/// When the GPU would not start and the choice does not allow a fallback
/// (`Renderer::Gpu`, or a device the app configured itself).
#[cfg(feature = "gpu")]
fn start_gpu_renderer(
    window: &dyn winit::window::Window,
    transparent: bool,
) -> Option<WgpuRenderer> {
    use super::desktop::GpuStartError;
    use super::renderer::{OnGpuFailure, on_gpu_failure, resolved, set_gpu_presenting, tries_gpu};

    let choice = resolved();
    if !tries_gpu(choice, super::desktop::gpu_init_installed()) {
        set_gpu_presenting(false);
        tracing::info!("rinch: presenting with the software renderer (chosen)");
        return None;
    }
    let size = window.surface_size();
    match WgpuRenderer::try_new_for(
        window,
        size.width.max(1),
        size.height.max(1),
        transparent,
        choice,
    ) {
        Ok(gpu) => {
            set_gpu_presenting(true);
            Some(gpu)
        }
        Err(GpuStartError::DeclinedCpuAdapter(adapter)) => {
            set_gpu_presenting(false);
            tracing::warn!("{}", super::renderer::declined_adapter_warning(&adapter));
            None
        }
        Err(e) => match on_gpu_failure(choice, super::desktop::gpu_init_installed()) {
            OnGpuFailure::FallBack => {
                set_gpu_presenting(false);
                tracing::warn!(
                    "rinch: the GPU renderer could not start ({e}); presenting with the \
                     software renderer"
                );
                None
            }
            OnGpuFailure::Panic => panic!("{e}"),
        },
    }
}

/// A build without `gpu` has only the software renderer: say so, and say it
/// louder when the app or `RINCH_RENDERER` asked for the GPU.
#[cfg(not(feature = "gpu"))]
fn announce_software_renderer() {
    use super::renderer::{Renderer, resolved};
    if resolved() == Renderer::Gpu {
        tracing::warn!(
            "rinch: the GPU renderer was requested, but this build has no `gpu` feature; \
             presenting with the software renderer"
        );
    } else {
        tracing::info!("rinch: presenting with the software renderer");
    }
}

#[cfg(test)]
mod layer_drag_tests {
    use super::*;
    use rinch_core::element::WindowProps;

    fn component(scope: &mut RenderScope) -> NodeHandle {
        scope.create_element("div")
    }

    /// The runtime's half of a layer drag (#1523), with no window: each
    /// pointer position is followed against where the compositor has the
    /// surface, which only `layer_moved` changes. The compositor here holds
    /// `lag` margin commits before it processes the oldest and reports it.
    fn drag(lag: usize) -> (f64, f64) {
        let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        let start = (80.0, 60.0);
        let grab = (16.0, 22.0);
        let mut layer = LayerState::new(Some(start));
        layer.gesture = Some(Gesture::start(GestureKind::Move, grab, start, (84.0, 84.0)));
        runtime.main.layer = Some(layer);
        let mut origin = start;
        let mut commits = std::collections::VecDeque::new();
        let mut pointer = (start.0 + grab.0, start.1 + grab.1);
        for _ in 0..60 {
            pointer = (pointer.0 + 10.0, pointer.1 + 6.0);
            let before = runtime.main.layer.unwrap().position;
            let resize = runtime
                .main
                .follow_layer_gesture((pointer.0 - origin.0, pointer.1 - origin.1));
            assert_eq!(resize, Some(None), "a move asks for no size");
            let after = runtime.main.layer.unwrap().position;
            if after != before {
                commits.push_back(after.unwrap());
            }
            while commits.len() > lag {
                origin = commits.pop_front().unwrap();
                runtime.main.layer_moved((origin.0 as i32, origin.1 as i32));
            }
        }
        while let Some(o) = commits.pop_front() {
            origin = o;
            runtime.main.layer_moved((o.0 as i32, o.1 as i32));
        }
        assert_eq!(runtime.main.layer.unwrap().position, Some(origin));
        origin
    }

    #[test]
    fn a_layer_drag_lands_under_the_pointer_at_every_lag() {
        for lag in [0, 1, 4, 60] {
            assert_eq!(drag(lag), (680.0, 420.0), "lag {lag}");
        }
    }

    #[test]
    fn no_gesture_follows_nothing() {
        let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        assert_eq!(
            runtime.main.follow_layer_gesture((5.0, 5.0)),
            None,
            "no layer"
        );
        runtime.main.layer = Some(LayerState::new(Some((10.0, 10.0))));
        assert_eq!(
            runtime.main.follow_layer_gesture((5.0, 5.0)),
            None,
            "no gesture"
        );
        assert_eq!(runtime.main.layer.unwrap().position, Some((10.0, 10.0)));
    }

    #[test]
    fn a_layer_resize_asks_for_its_size() {
        let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        let mut layer = LayerState::new(Some((100.0, 100.0)));
        layer.gesture = Some(Gesture::start(
            GestureKind::Resize(rinch_platform::ResizeDirection::East),
            (84.0, 40.0),
            (100.0, 100.0),
            (84.0, 84.0),
        ));
        runtime.main.layer = Some(layer);
        assert_eq!(
            runtime.main.follow_layer_gesture((94.0, 40.0)),
            Some(Some((94.0, 84.0)))
        );
    }

    fn bar_props() -> WindowProps {
        use rinch_core::element::{LayerAnchor, LayerAxes, LayerExclusiveZone, LayerSurface};
        WindowProps {
            height: 32,
            wayland_layer: Some(LayerSurface {
                anchor: LayerAnchor::BOTTOM | LayerAnchor::LEFT | LayerAnchor::RIGHT,
                exclusive_zone: LayerExclusiveZone::Reserve(32),
                compositor_size: LayerAxes::WIDTH,
                ..Default::default()
            }),
            // The shorthand would give a top-left overlay; the surface wins.
            non_activating: true,
            x: Some(40),
            y: Some(50),
            ..Default::default()
        }
    }

    /// An app's own layer surface (#2413) is placed by its anchors and
    /// margins: no move or resize gesture starts on it, and a position set
    /// on it re-anchors nothing. The request is taken (`true`), so it does
    /// not fall through to the compositor's move either.
    #[test]
    fn an_app_placed_layer_surface_takes_no_gesture_and_no_position() {
        let mut runtime = RinchRuntime::new("t", 800, 32, component, &[]);
        runtime.main.app.window_props = Some(bar_props());
        runtime
            .main
            .take_surface_kind(WindowSurfaceKind::LayerShell, true);
        assert_eq!(runtime.main.layer, Some(LayerState::app_placed()));
        assert!(runtime.main.start_layer_gesture(GestureKind::Move));
        assert!(
            runtime
                .main
                .start_layer_gesture(GestureKind::Resize(rinch_platform::ResizeDirection::West))
        );
        assert_eq!(runtime.main.layer.unwrap().gesture, None);
        runtime.main.place_layer((300.0, 200.0));
        assert_eq!(runtime.main.layer.unwrap().position, None);
        assert_eq!(
            runtime.main.saved_geometry.position, None,
            "x/y are not its margins"
        );
    }

    /// An overlay keeps its gestures and its position, as before #2413.
    #[test]
    fn an_overlay_layer_surface_keeps_its_position() {
        let mut runtime = RinchRuntime::new("t", 800, 32, component, &[]);
        runtime.main.app.window_props = Some(WindowProps {
            wayland_layer: None,
            ..bar_props()
        });
        runtime
            .main
            .take_surface_kind(WindowSurfaceKind::LayerShell, true);
        let layer = runtime.main.layer.unwrap();
        assert!(!layer.app_placed);
        assert_eq!(layer.position, Some((40.0, 50.0)));
    }

    /// GNOME (no layer shell): the window asked for a layer surface and got
    /// an ordinary toplevel. The decision is the layer role, its check warns,
    /// and the toplevel keeps no layer state, so moves and resizes go to the
    /// compositor as for any window.
    #[test]
    fn without_layer_shell_an_explicit_surface_is_an_ordinary_toplevel() {
        use super::overlay_window::{AfterCreate, WindowSteps, after_create_actions};
        let props = bar_props();
        let steps = WindowSteps::plan(&props, false, Backend::Wayland);
        assert!(steps.wayland_layer);
        assert!(!steps.wayland_unsupported);
        assert!(after_create_actions(steps, false).contains(&AfterCreate::CheckLayerShell));
        let mut runtime = RinchRuntime::new("t", 800, 32, component, &[]);
        runtime.main.app.window_props = Some(props);
        runtime
            .main
            .take_surface_kind(WindowSurfaceKind::Toplevel, true);
        assert_eq!(runtime.main.layer, None);
        assert!(!runtime.main.start_layer_gesture(GestureKind::Move));
    }

    #[test]
    fn only_a_transparent_window_gets_a_blending_gpu_surface() {
        let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        assert!(!runtime.main.gpu_surface_transparent());
        runtime.main.app.window_props = Some(WindowProps {
            transparent: true,
            ..Default::default()
        });
        assert!(runtime.main.gpu_surface_transparent());
    }
}

/// A `WindowProps::x`/`y` the window can be reached at.
#[derive(Debug, Clone, PartialEq)]
struct RequestedPosition {
    /// As asked, logical pixels: what an app reads back.
    logical: (f64, f64),
    /// What winit is given.
    at: Position,
    /// The monitor it was read in the scale of, by name.
    monitor: Option<String>,
}

/// The platform's name for the monitor `window` is on (whose scale it has):
/// exactly what `MonitorHandle::name` answers, so a name published with a
/// position finds the same monitor in `connected_monitors`.
fn monitor_name(window: &dyn winit::window::Window) -> Option<String> {
    window
        .current_monitor()
        .and_then(|m| m.name().map(|n| n.into_owned()))
}

/// #1884: the runtime's half of bringing a window back — the requested
/// position filtered by reachability, a saved one dropped at a show, the
/// creation geometry published on a first appearance (a hidden start's
/// first show included), and readings taken through the maximize wait.
#[cfg(test)]
mod window_restore_tests {
    use super::window_geometry::{MonitorRect, WindowReading, published_test_guard};
    use super::*;
    use crate::windows::{WindowRestore, current_window_restore};
    use rinch_core::element::WindowProps;
    use winit::dpi::{PhysicalPosition, PhysicalSize};

    fn component(scope: &mut RenderScope) -> NodeHandle {
        scope.create_element("div")
    }

    fn one_monitor() -> Vec<MonitorRect> {
        vec![MonitorRect {
            position: PhysicalPosition::new(0, 0),
            size: PhysicalSize::new(1920, 1080),
            scale: 1.0,
            name: None,
        }]
    }

    fn runtime(props: WindowProps) -> RinchRuntime {
        build_runtime(
            component,
            &WindowProps {
                width: 800,
                height: 600,
                ..props
            },
            None,
            &[],
        )
    }

    fn attributes(runtime: &mut RinchRuntime, first_appearance: bool) -> WindowAttributes {
        let steps = runtime.main.main_window_steps(Backend::X11, true);
        runtime
            .main
            .main_window_attributes(steps, Backend::X11, false, first_appearance)
    }

    /// #2095's runtime wiring, without a surface: a renderer that starts
    /// publishes what it does (software: `Unsupported`), `apply_vsync`
    /// publishes the request against it, hiding and shutting down return to
    /// `Pending`.
    #[test]
    fn the_runtime_publishes_its_windows_pacing_2095() {
        use crate::window::{Presentation, presentation, set_vsync};
        let _lock = crate::window::VSYNC_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set_vsync(true);
        crate::window::publish_pacing(None);
        let mut rt = runtime(WindowProps::default());
        assert_eq!(presentation(), Presentation::Pending);

        // The software renderer (no GPU renderer started).
        #[cfg(feature = "gpu")]
        rt.main.renderer_started(None);
        #[cfg(not(feature = "gpu"))]
        rt.main.note_renderer_pacing();
        assert_eq!(presentation(), Presentation::Unsupported);
        assert_eq!(set_vsync(false), Presentation::Unsupported);

        // A GPU surface with Mailbox, as `pacing_outcomes` reports one.
        rt.main.pacing = Some(crate::window::PacingOutcomes {
            on: Presentation::Vsync,
            off: Presentation::Mailbox,
        });
        rt.apply_vsync();
        assert_eq!(presentation(), Presentation::Mailbox, "off, as applied");

        rt.hide_window();
        assert_eq!(presentation(), Presentation::Pending, "hidden");
        assert_eq!(set_vsync(true), Presentation::Pending);

        rt.main.pacing = Some(crate::window::PacingOutcomes::UNSUPPORTED);
        crate::window::publish_pacing(rt.main.pacing);
        drop(rt);
        assert_eq!(set_vsync(true), Presentation::Pending, "shut down");
        crate::window::publish_pacing(None);
    }

    impl RinchRuntime {
        fn take_reading_unnamed(&mut self, r: WindowReading) {
            self.main.take_reading(r, None);
        }
    }

    fn reading(maximized: bool, pos: (i32, i32), size: (u32, u32)) -> WindowReading {
        WindowReading {
            maximized,
            minimized: false,
            scale: 1.0,
            position: Some(PhysicalPosition::new(pos.0, pos.1)),
            surface: PhysicalSize::new(size.0, size.1),
        }
    }

    /// The tray-app path: `visible: false` + `maximized: true`. The first
    /// show is the first appearance, so it publishes the geometry the window
    /// is created with; without it the restore answered the maximized size
    /// and no position (Windows maximizes at creation, so the first reading
    /// is already maximized).
    #[test]
    fn a_hidden_start_publishes_the_restore_geometry_at_its_first_show() {
        let _guard = published_test_guard();
        let mut rt = runtime(WindowProps {
            visible: false,
            maximized: true,
            x: Some(100),
            y: Some(80),
            ..Default::default()
        });
        assert!(
            !rt.main.begin_appearance(one_monitor(), false),
            "the sizing window"
        );
        assert_eq!(current_window_restore(), None, "nothing on screen yet");
        assert!(
            rt.main.begin_appearance(one_monitor(), true),
            "the first show"
        );
        let attrs = attributes(&mut rt, true);
        assert!(attrs.maximized);
        assert_eq!(
            attrs.position,
            Some(Position::Physical(PhysicalPosition::new(100, 80)))
        );
        rt.main.maximize_wait =
            window_geometry::MaximizeWait::after_create(&attrs, false, std::time::Instant::now());
        rt.take_reading_unnamed(reading(true, (0, 0), (1920, 1050)));
        assert_eq!(
            current_window_restore(),
            Some(WindowRestore {
                position: Some((100.0, 80.0)),
                size: (800.0, 600.0),
                maximized: true,
                monitor: None,
            })
        );
        // A later show is no first appearance and publishes nothing new.
        rt.main.shown_once = true;
        assert!(!rt.main.begin_appearance(one_monitor(), true));
    }

    /// X11: the window manager maximizes later, and a resize can come while
    /// the window does not say it is maximized yet. The restore keeps the
    /// creation geometry and `maximized: true`.
    #[test]
    fn a_resize_before_the_maximize_is_reported_is_not_the_restore_geometry() {
        let _guard = published_test_guard();
        let mut rt = runtime(WindowProps {
            maximized: true,
            x: Some(100),
            y: Some(80),
            ..Default::default()
        });
        assert!(rt.main.begin_appearance(one_monitor(), true));
        let attrs = attributes(&mut rt, true);
        rt.main.maximize_wait =
            window_geometry::MaximizeWait::after_create(&attrs, false, std::time::Instant::now());
        rt.take_reading_unnamed(reading(false, (100, 80), (800, 600)));
        rt.take_reading_unnamed(reading(false, (0, 0), (1920, 1050)));
        assert_eq!(
            current_window_restore(),
            Some(WindowRestore {
                position: Some((100.0, 80.0)),
                size: (800.0, 600.0),
                maximized: true,
                monitor: None,
            })
        );
        assert_eq!(
            rt.main.saved_geometry.size, None,
            "no normal geometry recorded"
        );
    }

    /// `WindowProps::x`/`y` out of reach (a monitor unplugged since it was
    /// saved) is not given to winit; in reach it is.
    #[test]
    fn a_requested_position_out_of_reach_is_left_to_the_system() {
        let _guard = published_test_guard();
        let mut gone = runtime(WindowProps {
            x: Some(5000),
            y: Some(100),
            ..Default::default()
        });
        gone.main.begin_appearance(one_monitor(), true);
        assert_eq!(gone.main.requested_position(), None);
        assert_eq!(attributes(&mut gone, true).position, None);
        assert_eq!(current_window_restore().and_then(|r| r.position), None);
    }

    /// And one in reach is, in desktop pixels, and is what the restore
    /// answers.
    #[test]
    fn a_requested_position_in_reach_is_taken() {
        let _guard = published_test_guard();
        let mut here = runtime(WindowProps {
            x: Some(300),
            y: Some(200),
            ..Default::default()
        });
        here.main.begin_appearance(one_monitor(), true);
        assert_eq!(
            attributes(&mut here, true).position,
            Some(Position::Physical(PhysicalPosition::new(300, 200)))
        );
        assert_eq!(
            current_window_restore().and_then(|r| r.position),
            Some((300.0, 200.0))
        );
    }

    /// A show after a hide drops a saved position out of reach on the
    /// monitors connected now, and keeps one in reach.
    #[test]
    fn a_show_drops_a_saved_position_out_of_reach() {
        let _guard = published_test_guard();
        let mut rt = runtime(WindowProps::default());
        rt.main.shown_once = true;
        let gone = Position::Physical(PhysicalPosition::new(5000, 100));
        rt.main.saved_geometry.position = Some(gone);
        assert!(!rt.main.begin_appearance(one_monitor(), true));
        assert_eq!(rt.main.saved_geometry.position, None);

        let kept = Position::Physical(PhysicalPosition::new(300, 200));
        rt.main.saved_geometry.position = Some(kept);
        rt.main.begin_appearance(one_monitor(), true);
        assert_eq!(rt.main.saved_geometry.position, Some(kept));
        // With no saved size, the window's own size decides: 800 wide at
        // x = 1880 leaves 40 columns on screen.
        let sliver = Position::Physical(PhysicalPosition::new(1880, 200));
        rt.main.saved_geometry.position = Some(sliver);
        rt.main.begin_appearance(one_monitor(), true);
        assert_eq!(rt.main.saved_geometry.position, None);
    }

    /// #1884: `WindowProps::monitor` settles a mixed-scale position — A
    /// (scale 1) at x 0..1920, B (scale 2) at 1920..5760; (1050, 50) is on
    /// both — and the restore answers the monitor it was placed on.
    #[test]
    fn the_requested_monitor_places_the_window_and_is_published() {
        let _guard = published_test_guard();
        let mons = || {
            vec![
                MonitorRect {
                    position: PhysicalPosition::new(0, 0),
                    size: PhysicalSize::new(1920, 1080),
                    scale: 1.0,
                    name: Some("A".to_string()),
                },
                MonitorRect {
                    position: PhysicalPosition::new(1920, 0),
                    size: PhysicalSize::new(3840, 2160),
                    scale: 2.0,
                    name: Some("B".to_string()),
                },
            ]
        };
        let mut rt = runtime(WindowProps {
            x: Some(1050),
            y: Some(50),
            monitor: Some("B".to_string()),
            ..Default::default()
        });
        assert!(rt.main.begin_appearance(mons(), true));
        assert_eq!(
            attributes(&mut rt, true).position,
            Some(Position::Physical(PhysicalPosition::new(2100, 100)))
        );
        let restore = current_window_restore().unwrap();
        assert_eq!(
            (restore.position, restore.monitor.as_deref()),
            (Some((1050.0, 50.0)), Some("B"))
        );
    }

    /// A position the app sets while the window is hidden is on no monitor
    /// rinch can name: the published name goes with the old position.
    #[test]
    fn a_position_set_while_hidden_drops_the_monitor_name() {
        let _guard = published_test_guard();
        window_geometry::publish_window(
            &reading(false, (100, 80), (800, 600)),
            Some("A".to_string()),
        );
        assert_eq!(
            current_window_restore().unwrap().monitor.as_deref(),
            Some("A")
        );
        window_geometry::publish_hidden(Some((300.0, 200.0)), None);
        assert_eq!(current_window_restore().unwrap().monitor, None);
    }

    /// #1884 re-review (D1): the window manager never maximizes a window
    /// created maximized; it is dragged inside the grace and then left
    /// alone, so no event reads it again. The reading taken when the grace
    /// ends records the move and that it is not maximized; after it the
    /// wait has no deadline, so no timed wake is armed for it again.
    #[test]
    fn the_reading_at_the_end_of_the_grace_records_a_refused_maximize() {
        let _guard = published_test_guard();
        let mut rt = runtime(WindowProps {
            maximized: true,
            x: Some(100),
            y: Some(80),
            ..Default::default()
        });
        assert!(rt.main.begin_appearance(one_monitor(), true));
        let attrs = attributes(&mut rt, true);
        rt.main.maximize_wait =
            window_geometry::MaximizeWait::after_create(&attrs, false, std::time::Instant::now());
        rt.take_reading_unnamed(reading(false, (100, 80), (800, 600)));
        rt.take_reading_unnamed(reading(false, (700, 400), (800, 600)));
        let restore = current_window_restore().unwrap();
        assert!(restore.maximized, "inside the grace: still waiting");
        assert!(rt.main.maximize_wait.deadline().is_some(), "a wake is due");
        rt.main
            .settle_maximize_wait_with(Some((reading(false, (700, 400), (800, 600)), None)));
        let restore = current_window_restore().unwrap();
        assert_eq!(
            (restore.maximized, restore.position),
            (false, Some((700.0, 400.0)))
        );
        assert_eq!(rt.main.maximize_wait.deadline(), None, "no wake after it");
        // A window that is not shown (or none) still ends the wait.
        rt.main.maximize_wait =
            window_geometry::MaximizeWait::after_create(&attrs, false, std::time::Instant::now());
        rt.main.window = None;
        rt.main.settle_maximize_wait();
        assert_eq!(rt.main.maximize_wait.deadline(), None);
    }
}
