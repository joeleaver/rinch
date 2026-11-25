//! A [`TestApp`]'s life: the builder that mounts it and the teardown
//! when it is dropped.

use super::*;

/// The component a [`TestAppBuilder`] mounts.
type BoxedComponent = Box<dyn FnOnce(&mut RenderScope) -> NodeHandle>;

/// Options for a [`TestApp`], from [`TestApp::builder`].
pub struct TestAppBuilder {
    component: BoxedComponent,
    /// The window's size, when set by [`Self::size`] (else the props').
    size: Option<(u32, u32)>,
    scale: f64,
    fonts: Vec<AppFont>,
    #[cfg(feature = "theme")]
    theme_css: Option<String>,
    props: WindowProps,
}

impl TestAppBuilder {
    /// Open the component in the window an [`App`](crate::App) with `props`
    /// would open (see [The window](super#the-window)). What a test can see of
    /// them: the size (`width`, `height`; [`Self::size`] wins whichever comes
    /// first), `visible: false` (mounted hidden: not painted and without the
    /// keyboard, until a [`WindowRequest::Show`]), `non_activating` (no
    /// keyboard at the mount or on a show), `size_to_content` (the window
    /// takes its content's size, and follows it as the content changes; an
    /// anchored layer surface in `wayland_layer` fills its anchored axes),
    /// `transparent` and `idle_trim` (the shown window's presentation is
    /// trimmed on the virtual clock as a Wayland software window's is, which
    /// only `RINCH_PERF`'s `trimmed`/`untrimmed` lines show). The rest are kept (read them back with
    /// [`TestApp::window_props`]) and do nothing here: there is no
    /// compositor.
    pub fn window_props(mut self, props: WindowProps) -> Self {
        self.props = props;
        self
    }

    /// The window's size in logical (CSS) pixels. Default 800x600.
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.size = Some((width.max(1), height.max(1)));
        self
    }

    /// The display's scale factor (device pixel ratio). Default `1.0`.
    /// Layout and input stay in logical pixels; media queries on resolution
    /// and the painted frame see the scale.
    pub fn scale_factor(mut self, scale: f64) -> Self {
        assert!(
            scale.is_finite() && scale > 0.0,
            "a scale factor is positive"
        );
        self.scale = scale;
        self
    }

    /// Fonts the app bundles, as [`crate::App::fonts`] registers them.
    ///
    /// A test that compares horizontal text geometry should bundle its font
    /// (`AppFont::sans_serif(..)`): otherwise widths depend on the fonts the
    /// host has installed.
    pub fn fonts(mut self, fonts: &[AppFont]) -> Self {
        self.fonts.extend_from_slice(fonts);
        self
    }

    /// The theme, as [`crate::App::theme`] sets it: the CSS variables the
    /// components read.
    #[cfg(feature = "theme")]
    pub fn theme(mut self, theme: crate::ThemeProviderProps) -> Self {
        self.theme_css = Some(crate::generate_theme_css_string(&theme));
        self
    }

    /// Mount the component, lay it out and paint its first frame, as the
    /// shell does when the window opens, and give the window keyboard focus
    /// — except a window the [props](Self::window_props) start hidden (mounted
    /// only) or make non-activating (painted, without the keyboard).
    ///
    /// The window publishes its size, scale and focus to
    /// [`rinch::window`](crate::window) from here on, and the thread's timers
    /// run on the app's virtual clock (see [Time](super#time)).
    pub fn mount(self) -> TestApp {
        let TestAppBuilder {
            component,
            size,
            scale,
            fonts,
            #[cfg(feature = "theme")]
            theme_css,
            props,
        } = self;
        let size = size.unwrap_or((props.width.max(1), props.height.max(1)));
        // Made as the shell makes its main window (`build_runtime`): in the
        // content-sized mode when the props ask, with the props on it.
        let mut app = crate::shell::rinch_runtime::app_for_window(component, &props);
        let fonts = registered_fonts::with_queued(fonts);
        app.register_app_fonts(&fonts);
        #[cfg(feature = "theme")]
        if let Some(css) = theme_css {
            app.set_owned_theme_css(Some(css));
        }
        let visible = props.visible;
        let non_activating = props.non_activating;
        app.set_window_props(props);
        // This thread drains its own main-thread callbacks and writes its own
        // signals, beside other tests' apps on other threads (#2778).
        rinch_core::mark_host_thread();
        // The window's requests, and its timers, are this app's from the
        // mount on: a component may ask for either while it mounts.
        let sink = Rc::new(Sink {
            doc: Cell::new(0),
            events: RefCell::new(Vec::new()),
            opened: RefCell::new(Vec::new()),
            opening: Cell::new(false),
        });
        SINKS.with(|s| s.borrow_mut().push(sink.clone()));
        virtual_time::install();
        // The shell's order (`ShellWindow::create_window`): the scale first,
        // so the first style resolution sees the device pixel ratio (#211),
        // then the mount at the logical size. The window's state is published
        // as the shell's main window publishes it (#2742), and an unfocused
        // window is what a previous test on this thread may have left.
        app.set_device_pixel_ratio(scale);
        app.set_publishes_window_state(true);
        app.publish_window_state(WindowFacts {
            focused: Some(false),
            ..Default::default()
        });
        // A content-sized window is mounted at its largest size for the
        // output it is on, as the shell's `mount_document_before_window`
        // mounts it: the outputs another app on this thread played are
        // listed already (#2836).
        let mount_size = match content_max_of(&app) {
            Some(max) => {
                app.restart_content_limit(max);
                max
            }
            None => (size.0 as f32, size.1 as f32),
        };
        // The harness has no surface: its stand-in for one is the logical size
        // times the scale, rounded (`physical_size`).
        app.set_surface_size((
            (f64::from(mount_size.0) * scale).round() as u32,
            (f64::from(mount_size.1) * scale).round() as u32,
        ));
        app.mount_component(mount_size.0, mount_size.1);
        sink.doc.set(app.doc_key());
        let mut test = TestApp {
            app,
            size,
            scale,
            modifiers: Modifiers::default(),
            actions: Vec::new(),
            requests: Vec::new(),
            visible,
            focused: false,
            non_activating,
            sink,
            serving: false,
            #[cfg(test)]
            painted: rinch_dom::perf::FrameStats::default(),
            #[cfg(test)]
            child_turns: 0,
            #[cfg(test)]
            harness_work: HarnessWork::default(),
            fonts,
            children: Vec::new(),
            presentation: None,
        };
        if test.app.is_content_sized() {
            test.app.resolve_and_repaint(mount_size.0, mount_size.1);
            test.negotiate_content_size();
        }
        if visible {
            test.paint();
            // A window the user is typing into or clicking has focus.
            if !non_activating {
                test.set_window_focus(true);
            }
        }
        test.serve_window_requests();
        test
    }
}

impl TestApp {
    /// Mount `component` in an 800x600 window at scale 1.
    pub fn mount(component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> Self {
        Self::builder(component).mount()
    }

    /// Choose the window's props, size, scale factor, fonts or theme before
    /// mounting.
    pub fn builder(
        component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static,
    ) -> TestAppBuilder {
        TestAppBuilder {
            component: Box::new(component),
            size: None,
            scale: 1.0,
            fonts: Vec::new(),
            #[cfg(feature = "theme")]
            theme_css: None,
            props: WindowProps {
                width: DEFAULT_SIZE.0,
                height: DEFAULT_SIZE.1,
                ..WindowProps::default()
            },
        }
    }
}

impl Drop for TestApp {
    /// The thread's window requests and timers are no longer this app's.
    fn drop(&mut self) {
        // Every document is disposed as its own code (the shell's teardown)
        // while this app's sink is registered: what their cleanups send (a
        // request for this app's window, the closes of the windows it owns)
        // is this app's, not another live app's, older or newer. Left to the
        // field drop, the main document would be disposed after the sink is
        // gone, and as no document's code.
        self.app.dispose_document();
        // Its idle-trim timer is cleared on the clock it was armed on.
        self.presentation = None;
        // Newest first: a popup closes before the window it is on (#2552).
        for child in std::mem::take(&mut self.children).into_iter().rev() {
            child.close();
        }
        let last = SINKS
            .try_with(|s| {
                let mut sinks = s.borrow_mut();
                sinks.retain(|k| !Rc::ptr_eq(k, &self.sink));
                sinks.is_empty()
            })
            .unwrap_or(false);
        // The outputs are the thread's (`played_outputs`): kept for the
        // apps still live here, forgotten with the last, so the next app on
        // this thread starts with none played. Forgotten here, after the
        // main document was disposed (first, above) and the children closed
        // (each `close` disposes its document): no reader of this app is
        // left to run for it, and no other app lives on the thread, so the
        // reset wakes nobody. An app never sees its outputs vanish because
        // it is going, as in the shell. Pinned by
        // `the_last_apps_readers_do_not_run_as_it_forgets_the_outputs`.
        if last {
            crate::outputs::unpublish();
        }
        virtual_time::uninstall();
    }
}
