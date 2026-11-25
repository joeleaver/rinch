//! Making the runtime (its main window's app, the bundled fonts, the
//! native event queue's drainer) and tearing it down in order.

use super::*;

impl RinchRuntime {
    pub(super) fn new(
        title: &str,
        width: u32,
        height: u32,
        component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static,
        fonts: &[crate::font::AppFont],
    ) -> Self {
        Self::with_app(title, width, height, RinchApp::new(component), fonts)
    }

    /// [`Self::new`] around an app already made: a main window sized by its
    /// content is made in that mode (`RinchApp::new_content_sized`), which is
    /// fixed at construction.
    pub(super) fn with_app(
        title: &str,
        width: u32,
        height: u32,
        mut app: RinchApp,
        fonts: &[crate::font::AppFont],
    ) -> Self {
        // Before the window, before the component, and therefore before the
        // first layout pass: an app-bundled face has to be in the font context
        // that measures the first frame, or that frame is measured against a
        // fallback and reflows once the real face arrives.
        #[cfg(test)]
        let _queue_lock = TestQueueLock::hold();
        // From here native events are queued: this runtime drains them
        // (#2801).
        wake::note_runtime_made();
        // Faces dropped while the process looked like an embed host are
        // lost; say so now that it turned out to be an app (#3042).
        #[cfg(any(feature = "gpu-renderer", feature = "embed"))]
        let _ = registered_fonts::warn_dropped();
        // The named video sink factory (rinch-core's hook), so video players
        // and rinch-rtc's remote tracks deliver frames through the
        // RenderSurface compositing pipeline. Here rather than in the two
        // entry points, which create the event loop and cannot be tested.
        rinch_core::video_sink::set_video_sink_factory(
            crate::render_surface::create_video_frame_sink,
        );
        // The build's faces, then any `rinch::font::register_font` queued
        // before there was a runtime to wake: those reach the first frame too.
        // A queued face identical to one already there is skipped.
        let mut fonts = fonts.to_vec();
        registered_fonts::take_new_fonts(&mut fonts);
        app.register_app_fonts(&fonts);
        // The main window's size, scale and focus reach `rinch::window`
        // (#1770); the DevTools window's do not.
        let windowless = crate::window_handle::windowless();
        // A windowless root has no window to describe.
        app.set_publishes_window_state(!windowless);
        // Nor any frames: its reactive work is nobody's, on the pass line
        // (#2798), not on a line never printed.
        app.set_counts_own_perf(!windowless);
        // A windowless app's root is no window: `WindowHandle::main()` is
        // `None` and `windows()` lists only what it opens.
        let main_handle = if windowless {
            0
        } else {
            crate::window_handle::register_main()
        };
        let mut main = ShellWindow::new(app, title, width, height);
        main.handle = main_handle;
        Self {
            main,
            proxy: None,
            native_menu: None,
            draining_native_events: false,
            devtools_store: None,
            devtools: None,
            #[cfg(feature = "gpu-renderer")]
            gpu_rebuild: None,
            devtools_closing: false,
            devtools_prev_hovered: None,
            others: Vec::new(),
            others_closing: Vec::new(),
            main_handle,
            main_closing: false,
            main_closed: windowless,
            exit_when: crate::window_handle::exit_when(),
            windowless,
            windowless_root_warned: false,
            fonts,
            popups: Default::default(),
            #[cfg(test)]
            _queue_lock,
        }
    }

    /// Explicit shutdown: drop resources in the correct order and clear global state.
    pub(super) fn shutdown(&mut self) {
        // 0. Close DevTools and the other windows first: the main window's
        // popups first of all, told their parent closed (#2552).
        self.close_devtools();
        if !self.main_closed {
            let main = self.main_handle;
            self.close_popups_above(main);
        }
        self.close_all_others();
        // The renderers closed popups left, while the thread's locals are
        // alive (#2552).
        #[cfg(feature = "gpu-renderer")]
        popups::drop_spare_renderers();

        // 1. Clear the signal-change callback so no stale closures fire during drop.
        rinch_core::clear_on_signal_change();

        // 2. Drop any pending main-thread callbacks (they may capture app state).
        rinch_core::clear_main_callbacks();

        // 3. Drop the app first (disposes effects/scopes before GPU resources).
        //    RinchApp's own drop order handles _render_scope before doc.
        drop(self.main.app.component.take());
        drop(self.main.app._render_scope.take());
        drop(self.main.app.doc.take());

        // 4. Drop renderer before window — Surface holds a window handle reference.
        #[cfg(feature = "gpu-renderer")]
        drop(self.main.renderer.take());
        drop(self.main.soft_renderer.take());

        // 5. Window can now be dropped safely.
        drop(self.main.window.take());

        // 6. Nothing presents any more: `set_vsync` answers `Pending` (#2095).
        self.main.pacing = None;
        crate::window::clear_pacing();

        // 7. No window is open.
        crate::window_handle::reset();
    }
}

impl Drop for RinchRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}
