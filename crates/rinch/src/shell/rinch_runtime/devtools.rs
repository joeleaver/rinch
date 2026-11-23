//! The DevTools window: opening and closing it, and its painting. Its events
//! take the main window's path (`RinchRuntime::window_event_for`).

use super::*;

impl RinchRuntime {
    // ── DevTools window management ──────────────────────────────────────────

    /// Ensure the DevToolsStore exists (created once, persists across open/close).
    pub(super) fn ensure_devtools_store(&mut self) {
        if self.devtools_store.is_none() {
            let store = super::devtools_store::DevToolsStore::new();
            rinch_core::create_store(store);
            self.devtools_store = Some(store);
        }
    }

    /// Toggle the DevTools window open/closed.
    pub(super) fn toggle_devtools(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.devtools.is_some() {
            self.devtools_closing = true;
        } else {
            self.open_devtools(event_loop);
        }
    }

    /// Close the DevTools window if it was asked to close: at the end of the
    /// handler that asked, so the window an event is being handled for is
    /// never closed under it.
    pub(super) fn settle_devtools_close(&mut self) {
        if std::mem::take(&mut self.devtools_closing) {
            self.close_devtools();
        }
    }

    /// Open the DevTools window.
    fn open_devtools(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.devtools.is_some() {
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
        // DevTools follows the app's zoom, as every window does.
        let dt_scale = crate::zoom::apply(window.scale_factor());

        // DevTools presents the way the main window does: on the GPU only when
        // the main window is, and with software if the GPU will not start for
        // this window too.
        // Within a device loss's recovery (#1788) the main window may be
        // waiting for its renderer: DevTools tries, and waits with it.
        #[cfg(feature = "gpu-renderer")]
        let recovering = super::device_loss::gpu_recovering(std::time::Instant::now());
        #[cfg(feature = "gpu-renderer")]
        let mut awaits_gpu = false;
        #[cfg(feature = "gpu-renderer")]
        let renderer =
            if self.main.renderer.is_some() || (recovering && self.awaits_gpu(WindowRole::Main)) {
                match WgpuRenderer::try_new(&*window, size.width.max(1), size.height.max(1)) {
                    Ok(gpu) => Some(gpu),
                    Err(e) if recovering => {
                        tracing::info!("DevTools: no GPU renderer yet after the device loss ({e})");
                        awaits_gpu = true;
                        None
                    }
                    Err(e) => {
                        tracing::warn!("DevTools: GPU renderer unavailable ({e}); using software");
                        None
                    }
                }
            } else {
                None
            };

        let winit_window = WinitWindow::new(window);

        // A separate RinchApp for DevTools with the panel component.
        let (dt_app, dt_logical) = devtools_app(
            super::devtools_panel::devtools_root,
            (size.width, size.height),
            dt_scale,
        );

        // Inject DevTools CSS into the document
        if let Some(doc) = &dt_app.doc {
            let mut d = doc.borrow_mut();
            d.load_css(super::devtools_css::DEVTOOLS_CSS);
            d.recompute_all_styles_full();
            d.resolve_layout(dt_logical.0 as f32, dt_logical.1 as f32);
        }

        let facts_key = Some(dt_app.doc_key());
        let mut devtools = ShellWindow::new(dt_app, "DevTools", 480, 600);
        devtools.facts_key = facts_key;
        devtools.shown_once = true;
        crate::shell::layer_shell::publish_surface_kind(facts_key, WindowSurfaceKind::Toplevel);
        devtools.note_geometry(&*winit_window.window);
        devtools.window = Some(winit_window);
        devtools.install_pointer_lock_host();
        #[cfg(feature = "gpu-renderer")]
        {
            devtools.renderer = renderer;
            devtools.awaits_gpu = awaits_gpu;
        }
        self.devtools = Some(devtools);

        // Bump version to trigger initial tree read in DevTools effects
        store.bump_version();

        // Lay it out with the initial data, and draw it.
        self.about_to_wait_for(WindowRole::DevTools, event_loop);
        if let Some(w) = self.devtools.as_ref().and_then(|d| d.window.as_ref()) {
            w.request_redraw();
        }

        tracing::info!("DevTools: opened");
    }

    /// Close the DevTools window. Store persists.
    pub(super) fn close_devtools(&mut self) {
        if let Some(store) = &self.devtools_store {
            store.visible.set(false);
        }

        // Its document goes, then its published facts (#2416), then its
        // renderer and window (`ShellWindow::close`).
        if let Some(devtools) = self.devtools.take() {
            devtools.close();
        }

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

    /// Paint the DevTools window.
    pub(super) fn paint_devtools(&mut self) -> Result<(), String> {
        #[cfg(feature = "gpu-renderer")]
        if self.devtools.as_ref().is_some_and(|d| d.renderer.is_some()) {
            return self.paint_devtools_gpu();
        }
        self.paint_devtools_software()
    }

    fn paint_devtools_software(&mut self) -> Result<(), String> {
        let Some(devtools) = self.devtools.as_mut() else {
            return Ok(());
        };
        let Some(window) = &devtools.window else {
            return Ok(());
        };
        let dt_app = &mut devtools.app;

        let scale = window.scale_factor();
        let size = window.logical_size();
        let (_base, w, h) = dt_app.build_pixels(scale, size, false);

        let pixels = dt_app
            .skia_painter
            .as_ref()
            .map(|p| p.pixels())
            .unwrap_or(&[]);

        if devtools.soft_renderer.is_none() {
            devtools.soft_renderer =
                Some(crate::shell::softbuffer_renderer::SoftbufferRenderer::new(
                    window.window.clone(),
                    w,
                    h,
                    false,
                ));
        }

        if let Some(renderer) = &mut devtools.soft_renderer {
            renderer.present_pixels(pixels, w, h);
        }

        Ok(())
    }

    #[cfg(feature = "gpu-renderer")]
    fn paint_devtools_gpu(&mut self) -> Result<(), String> {
        let Some(devtools) = self.devtools.as_mut() else {
            return Ok(());
        };
        let (Some(renderer), Some(window)) = (&mut devtools.renderer, &devtools.window) else {
            return Ok(());
        };
        let dt_app = &mut devtools.app;
        renderer.pacing.pace_by(&window.window);

        let scale = window.scale_factor();
        let size = window.logical_size();
        let scene = dt_app.build_scene(scale, size);
        renderer.paint(scene, false)?;
        Ok(())
    }
}

/// DevTools' `RinchApp`, `root` mounted at the logical size of a surface
/// `surface` device pixels big at `scale`: the surface's own size first, as
/// every caller sets it before the scale; then the display scale (issue
/// #211), before the mount at its logical viewport size, the main window's
/// order; and its facts its own (#2416), turned on before the mount, which
/// then publishes them under the new document's key before the panel is
/// built (#2789), so the panel's code reads its own window's facts from
/// its first run. Answers the app and the logical size it was mounted at.
fn devtools_app(
    root: fn(&mut RenderScope) -> NodeHandle,
    surface: (u32, u32),
    scale: f64,
) -> (RinchApp, (u32, u32)) {
    let mut app = RinchApp::new(root);
    app.set_surface_size((surface.0.max(1), surface.1.max(1)));
    app.set_device_pixel_ratio(scale);
    app.set_publishes_own_window_state();
    // DevTools ends no performance frames: its work stays nobody's (#2798).
    app.set_counts_own_perf(false);
    let logical = to_logical(surface, scale);
    app.mount_component(logical.0 as f32, logical.1 as f32);
    (app, logical)
}

#[cfg(test)]
mod facts_tests {
    use super::*;

    /// One read: the logical size, the physical size and the scale.
    type Read = ((f64, f64), (u32, u32), f64);

    thread_local! {
        /// What the effect made in [`reading_root`]'s mount read, each run.
        static READS: RefCell<Vec<Read>> = const { RefCell::new(Vec::new()) };
    }

    fn reading_root(scope: &mut RenderScope) -> NodeHandle {
        rinch_core::Effect::new(|| {
            let read = (
                crate::window::window_size(),
                crate::window::window_physical_size(),
                crate::window::scale_factor(),
            );
            READS.with(|reads| reads.borrow_mut().push(read));
        });
        scope.create_element("div")
    }

    /// DevTools' panel reads its own window's facts from its mount on, and
    /// follows them: its facts are turned on, and its surface set, before
    /// its mount (#2789).
    #[test]
    fn the_devtools_panel_reads_and_follows_its_own_facts() {
        READS.with(|reads| reads.borrow_mut().clear());
        // The thread's facts (the main window's), which it must not read.
        crate::window::publish(crate::window::WindowFacts {
            scale: Some(1.0),
            size: Some((1111.0, 777.0)),
            physical_size: Some((1111, 777)),
            focused: Some(true),
        });
        let (mut app, logical) = devtools_app(reading_root, (720, 900), 1.5);
        assert_eq!(logical, (480, 600));
        let reads = || READS.with(|reads| reads.borrow().clone());
        assert_eq!(reads(), [((480.0, 600.0), (720, 900), 1.5)]);
        app.set_surface_size((750, 900));
        let _ = app.handle_event(
            PlatformEvent::Resized {
                width: 500,
                height: 600,
            },
            (750, 900),
            1.5,
        );
        assert_eq!(reads().last(), Some(&((500.0, 600.0), (750, 900), 1.5)));
    }
}
