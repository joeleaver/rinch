//! The DevTools window: opening and closing it, its painting, and its events.

use super::*;

/// One turn of the event loop for the DevTools window's app, which is sent
/// no `AboutToWait` of its own: a frame of any wheel glide (#2287 — the
/// DevTools window sends line wheels too, and a glide nothing ticks never
/// moves), then resolve and repaint what changed. Answers whether the window
/// needs a redraw: while a glide runs it does, so the loop keeps turning.
pub(crate) fn devtools_turn(app: &mut RinchApp, width: f32, height: f32) -> bool {
    let gliding = app.tick_wheel_scroll();
    app.resolve_and_repaint(width, height) || gliding
}

impl RinchRuntime {
    // ── DevTools window management ──────────────────────────────────────────

    /// Ensure the DevToolsStore exists (created once, persists across open/close).
    fn ensure_devtools_store(&mut self) {
        if self.devtools_store.is_none() {
            let store = super::devtools_store::DevToolsStore::new();
            rinch_core::create_store(store);
            self.devtools_store = Some(store);
        }
    }

    /// Toggle the DevTools window open/closed.
    pub(super) fn toggle_devtools(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.devtools_window.is_some() {
            self.close_devtools();
        } else {
            self.open_devtools(event_loop);
        }
    }

    /// Open the DevTools window.
    fn open_devtools(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.devtools_window.is_some() {
            return; // Already open
        }

        self.ensure_devtools_store();
        let store = self.devtools_store.unwrap();
        store.visible.set(true);

        // Provide the main app's document to DevTools via context so it can read directly
        if let Some(doc) = &self.main.app.doc {
            rinch_core::create_context(super::devtools_store::MainDocRef(doc.clone()));
        }

        // Create DevTools window
        let window_attrs = WindowAttributes::default()
            .with_title("DevTools")
            .with_surface_size(winit::dpi::LogicalSize::new(480u32, 600u32));

        let window = event_loop
            .create_window(window_attrs)
            .expect("Failed to create DevTools window");

        let size = window.surface_size();
        let dt_scale = window.scale_factor();

        // DevTools presents the way the main window does: on the GPU only when
        // the main window is, and with software if the GPU will not start for
        // this window too.
        #[cfg(feature = "gpu")]
        if self.main.renderer.is_some() {
            match WgpuRenderer::try_new(&*window, size.width.max(1), size.height.max(1)) {
                Ok(gpu) => self.devtools_renderer = Some(gpu),
                Err(e) => {
                    tracing::warn!("DevTools: GPU renderer unavailable ({e}); using software")
                }
            }
        }
        #[cfg(not(feature = "gpu"))]
        let _ = size;

        let winit_window = WinitWindow::new(window);
        self.devtools_window = Some(winit_window);

        // Create a separate RinchApp for DevTools with the panel component
        let mut dt_app = RinchApp::new(super::devtools_panel::devtools_root);

        // Mount the DevTools component at its logical viewport size, with the
        // display scale pushed first (issue #211) — same order as the main
        // window mount.
        dt_app.set_device_pixel_ratio(dt_scale);
        let dt_logical = to_logical((size.width, size.height), dt_scale);
        dt_app.mount_component(dt_logical.0 as f32, dt_logical.1 as f32);

        // Inject DevTools CSS into the document
        if let Some(doc) = &dt_app.doc {
            let mut d = doc.borrow_mut();
            d.load_css(super::devtools_css::DEVTOOLS_CSS);
            d.recompute_all_styles_full();
            d.resolve_layout(dt_logical.0 as f32, dt_logical.1 as f32);
        }

        self.devtools_app = Some(dt_app);

        // Bump version to trigger initial tree read in DevTools effects
        store.bump_version();

        // Resolve DevTools layout with the initial data
        self.resolve_devtools();

        // Request initial draw
        self.devtools_window.as_ref().unwrap().request_redraw();

        tracing::info!("DevTools: opened");
    }

    /// Close the DevTools window. Store persists.
    pub(super) fn close_devtools(&mut self) {
        if let Some(store) = &self.devtools_store {
            store.visible.set(false);
        }

        // Drop DevTools app
        if let Some(mut dt_app) = self.devtools_app.take() {
            drop(dt_app.component.take());
            drop(dt_app._render_scope.take());
            drop(dt_app.doc.take());
        }

        // Drop renderer before window
        #[cfg(feature = "gpu")]
        drop(self.devtools_renderer.take());
        drop(self.devtools_soft_renderer.take());

        drop(self.devtools_window.take());

        tracing::info!("DevTools: closed");
    }

    /// Toggle inspect mode on/off.
    pub(super) fn toggle_inspect_mode(&mut self) {
        self.ensure_devtools_store();
        if let Some(store) = &self.devtools_store {
            let current = store.inspect_mode.get();
            store.inspect_mode.set(!current);
            if current {
                // Turning off: clear highlights
                store.hovered_node_id.set(None);
                self.main.app.inspect_highlight = None;
                // The highlight's last rect is the damage (`last_inspect_rect`).
                self.main.app.request_repaint();
                if let Some(w) = &self.main.window {
                    w.request_redraw();
                }
            }
            tracing::info!("Inspect mode: {}", if !current { "ON" } else { "OFF" });
        }
    }

    /// Update inspect highlight from the DevTools store.
    /// Called before painting the main window.
    pub(super) fn update_inspect_highlight(&mut self) {
        let Some(store) = &self.devtools_store else {
            self.main.app.inspect_highlight = None;
            return;
        };

        let hovered_id = store.hovered_node_id.get();
        let new_rect = hovered_id.and_then(|node_id| {
            let doc = self.main.app.doc.as_ref()?;
            let d = doc.borrow();
            d.tree.nodes.get(node_id)?;
            // The box the node is *painted* in, so the inspect overlay outlines
            // what is on screen rather than the untransformed layout box (#203).
            let r = rinch_dom::paint::painted_border_box(&d.tree, node_id, 1.0);
            Some((
                r.x0 as f32,
                r.y0 as f32,
                r.width() as f32,
                r.height() as f32,
            ))
        });

        if self.main.app.inspect_highlight != new_rect {
            self.main.app.inspect_highlight = new_rect;
            // Its old and new rects are the damage (`last_inspect_rect`).
            self.main.app.request_repaint();
        }
    }

    /// Get the DevTools window ID (if open).
    pub(super) fn devtools_window_id(&self) -> Option<WindowId> {
        self.devtools_window.as_ref().map(|w| w.window.id())
    }

    /// The DevTools window's surface size, in physical pixels — what
    /// `RinchApp::handle_event` expects (it derives the logical viewport
    /// itself). Falls back to the size DevTools is created at.
    fn devtools_size(&self) -> (u32, u32) {
        self.devtools_window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or((480, 600))
    }

    /// The DevTools window's scale factor.
    fn devtools_scale_factor(&self) -> f64 {
        self.devtools_window
            .as_ref()
            .map(|w| w.scale_factor())
            .unwrap_or(1.0)
    }

    /// Paint the DevTools window.
    fn paint_devtools(&mut self) -> Result<(), String> {
        #[cfg(feature = "gpu")]
        if self.devtools_renderer.is_some() {
            return self.paint_devtools_gpu();
        }
        self.paint_devtools_software()
    }

    fn paint_devtools_software(&mut self) -> Result<(), String> {
        let Some(window) = &self.devtools_window else {
            return Ok(());
        };
        let Some(dt_app) = &mut self.devtools_app else {
            return Ok(());
        };

        let scale = window.scale_factor();
        let size = window.logical_size();
        let (_base, w, h) = dt_app.build_pixels(scale, size, false);

        let pixels = dt_app
            .skia_painter
            .as_ref()
            .map(|p| p.pixels())
            .unwrap_or(&[]);

        if self.devtools_soft_renderer.is_none() {
            self.devtools_soft_renderer =
                Some(super::softbuffer_renderer::SoftbufferRenderer::new(
                    window.window.clone(),
                    w,
                    h,
                    false,
                ));
        }

        if let Some(renderer) = &mut self.devtools_soft_renderer {
            renderer.present_pixels(pixels, w, h);
        }

        Ok(())
    }

    #[cfg(feature = "gpu")]
    fn paint_devtools_gpu(&mut self) -> Result<(), String> {
        let Some(renderer) = &mut self.devtools_renderer else {
            return Ok(());
        };
        let Some(window) = &self.devtools_window else {
            return Ok(());
        };
        let Some(dt_app) = &mut self.devtools_app else {
            return Ok(());
        };

        let scale = window.scale_factor();
        let size = window.logical_size();
        let scene = dt_app.build_scene(scale, size);
        renderer.paint(scene, false)?;
        Ok(())
    }

    /// Resolve and repaint the DevTools window after signal changes.
    pub(super) fn resolve_devtools(&mut self) {
        let Some(dt_app) = &mut self.devtools_app else {
            return;
        };
        let Some(window) = &self.devtools_window else {
            return;
        };
        let size = window.logical_size();
        if devtools_turn(dt_app, size.0 as f32, size.1 as f32) {
            window.request_redraw();
        }
    }
}

impl RinchRuntime {
    /// Handle events for the DevTools window.
    pub(super) fn handle_devtools_window_event(
        &mut self,
        _event_loop: &dyn ActiveEventLoop,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.close_devtools();
            }
            WindowEvent::SurfaceResized(size) => {
                #[cfg(feature = "gpu")]
                if let Some(renderer) = &mut self.devtools_renderer {
                    renderer.resize(size.width.max(1), size.height.max(1));
                }
                let dt_scale = self.devtools_scale_factor();
                if let Some(dt_app) = &mut self.devtools_app {
                    let logical = to_logical((size.width, size.height), dt_scale);
                    dt_app.resize_layout(logical.0, logical.1);
                }
                if let Some(w) = &self.devtools_window {
                    w.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // The DevTools window is its own document — push the new
                // display scale into its Stylo device too (issue #211).
                let size = self.devtools_size();
                let dt_scale = self.devtools_scale_factor();
                if let Some(dt_app) = &mut self.devtools_app {
                    let _ = dt_app.handle_event(
                        PlatformEvent::ScaleFactorChanged(scale_factor),
                        size,
                        dt_scale,
                    );
                }
                if let Some(w) = &self.devtools_window {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.paint_devtools() {
                    eprintln!("DevTools paint error: {}", e);
                }
            }
            WindowEvent::PointerMoved { position, .. } => {
                let size = self.devtools_size();
                let scale = self.devtools_scale_factor();
                // Same physical→logical conversion as the main window, against
                // the DevTools window's own scale factor (#299).
                let (lx, ly) = to_logical_point((position.x, position.y), scale);
                if let Some(dt_app) = &mut self.devtools_app {
                    let actions = dt_app.handle_event(
                        PlatformEvent::MouseMove {
                            x: lx as f32,
                            y: ly as f32,
                        },
                        size,
                        scale,
                    );
                    for action in actions {
                        if let AppAction::RequestRedraw = action {
                            if let Some(w) = &self.devtools_window {
                                w.request_redraw();
                            }
                        }
                    }
                }
            }
            WindowEvent::PointerButton {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                let platform_button = match button {
                    winit::event::ButtonSource::Mouse(MouseButton::Left)
                    | winit::event::ButtonSource::Touch { .. } => PlatformMouseButton::Left,
                    winit::event::ButtonSource::Mouse(MouseButton::Right) => {
                        PlatformMouseButton::Right
                    }
                    winit::event::ButtonSource::Mouse(MouseButton::Middle) => {
                        PlatformMouseButton::Middle
                    }
                    _ => return,
                };
                let size = self.devtools_size();
                let scale = self.devtools_scale_factor();
                if let Some(dt_app) = &mut self.devtools_app {
                    let (x, y) = dt_app.cursor_pos.unwrap_or((0.0, 0.0));
                    let actions = dt_app.handle_event(
                        PlatformEvent::MouseDown {
                            x,
                            y,
                            button: platform_button,
                        },
                        size,
                        scale,
                    );
                    for action in actions {
                        if let AppAction::RequestRedraw = action {
                            if let Some(w) = &self.devtools_window {
                                w.request_redraw();
                            }
                        }
                    }
                }
            }
            WindowEvent::PointerButton {
                state: ElementState::Released,
                button,
                ..
            } => {
                let platform_button = match button {
                    winit::event::ButtonSource::Mouse(MouseButton::Left)
                    | winit::event::ButtonSource::Touch { .. } => PlatformMouseButton::Left,
                    winit::event::ButtonSource::Mouse(MouseButton::Right) => {
                        PlatformMouseButton::Right
                    }
                    winit::event::ButtonSource::Mouse(MouseButton::Middle) => {
                        PlatformMouseButton::Middle
                    }
                    _ => return,
                };
                let size = self.devtools_size();
                let scale = self.devtools_scale_factor();
                if let Some(dt_app) = &mut self.devtools_app {
                    let (x, y) = dt_app.cursor_pos.unwrap_or((0.0, 0.0));
                    let actions = dt_app.handle_event(
                        PlatformEvent::MouseUp {
                            x,
                            y,
                            button: platform_button,
                        },
                        size,
                        scale,
                    );
                    for action in actions {
                        if let AppAction::RequestRedraw = action {
                            if let Some(w) = &self.devtools_window {
                                w.request_redraw();
                            }
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let size = self.devtools_size();
                let scale = self.devtools_scale_factor();
                if let Some(dt_app) = &mut self.devtools_app {
                    let cursor = dt_app.cursor_pos.unwrap_or((0.0, 0.0));
                    let Some(event) = wheel_platform_event(delta, cursor, scale) else {
                        return;
                    };
                    let actions = dt_app.handle_event(event, size, scale);
                    for action in actions {
                        if let AppAction::RequestRedraw = action {
                            if let Some(w) = &self.devtools_window {
                                w.request_redraw();
                            }
                        }
                    }
                }
            }
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.devtools_modifiers = new_modifiers.state();
            }
            WindowEvent::KeyboardInput {
                event:
                    winit::event::KeyEvent {
                        physical_key:
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F12),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                // F12 in DevTools window also toggles
                self.close_devtools();
            }
            _ => {}
        }
    }
}
