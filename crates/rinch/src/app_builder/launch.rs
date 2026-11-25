//! Turning a configured [`App`] into a running one: the startup record,
//! the parts the web backend takes, and `run` / `try_run` / `run_android`.

#[cfg(all(
    feature = "windowing",
    any(
        target_os = "linux",
        target_os = "windows",
        not(feature = "native-menus")
    )
))]
use super::menu_bar::run_desktop_in_app_bar;
use super::*;

impl<F, M> App<F, M>
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    /// Resolve the whole configuration. Pure: no thread is taken over, no
    /// global is written, nothing is registered — so a test can assert that a
    /// builder chain produced what it claims to.
    #[cfg(feature = "windowing")]
    pub(crate) fn into_startup(self) -> Startup<F> {
        Startup {
            props: resolve_window_props(self.props, self.title, self.size),
            theme: self.theme.unwrap_or_default(),
            fonts: self.fonts,
            font_defaults: self.font_defaults,
            menus: self.menus,
            spellcheck: self.spellcheck,
            renderer: self.renderer,
            shortcut_matching: self.shortcut_matching,
            chord_yield: self.chord_yield,
            prefer_x11: self.prefer_x11,
            exit_when: self.exit_when,
            windowless: self.windowless,
            title_tooltips: self.title_tooltips,
            font_warmup: self.font_warmup,
            smooth_wheel_scroll: self.smooth_wheel_scroll,
            devtools: self.devtools,
            debug_interface: self.debug_interface,
            vsync: self.vsync,
            text_rasterization: self.text_rasterization,
            #[cfg(feature = "gpu-renderer")]
            gpu_antialiasing: self.gpu_antialiasing,
            activation_policy: self.activation_policy,
            #[cfg(feature = "gpu-renderer")]
            gpu: self.gpu,
            component: self.component,
        }
    }

    /// The parts of the configuration a web mount reads, for `rinch_web::AppExt`
    /// (#1769). Not for apps: call `run()` (desktop) or `rinch_web::AppExt::run`
    /// (web). Pure, so a test can check what a builder chain hands the web.
    ///
    /// `title` is [`title`](App::title) if one was set, else the title carried
    /// by [`window_props`](App::window_props) when it is not the default one
    /// (`WindowProps::default().title`), else `None` — so a web page keeps its
    /// own `<title>` unless the app named one. Size, the other window props
    /// and the GPU configuration have no web counterpart and are not handed
    /// over; the fonts are, for the mount to ignore.
    #[doc(hidden)]
    pub fn into_web_parts(self) -> WebParts<F> {
        let default_title = WindowProps::default().title;
        let title = self
            .title
            .or_else(|| (self.props.title != default_title).then(|| self.props.title.clone()));
        WebParts {
            component: self.component,
            title,
            theme: self.theme.unwrap_or_default(),
            menus: self.menus,
            fonts: self.fonts,
            spellcheck: self.spellcheck,
        }
    }

    /// Start the application on the desktop.
    ///
    /// Takes over the thread and returns only when the application exits its
    /// event loop — which it does through
    /// [`close_current_window`](crate::windows::close_current_window) or an
    /// `on_close_requested` that answers `true`. Most applications treat this
    /// as the last statement in `main`.
    ///
    /// When the event loop ends with an error instead — the Wayland
    /// compositor went away under the app, at a logout or a compositor crash
    /// (#2531) — the app's state is dropped as at an ordinary exit, the error
    /// is logged in one line and the process exits with status 1; `run` does
    /// not return. The exit is `std::process::exit`: it does not unwind, so
    /// the locals of `main` (a log flusher's guard, say) are not dropped. To
    /// handle that end yourself (a desktop component that treats the display
    /// going away as an ordinary end, or a `main` with guards to drop), call
    /// [`try_run`](App::try_run). X11 differs: when the X server goes away or drops the
    /// connection (`xkill`), Xlib's I/O error handler prints "X connection
    /// to :N broken" and exits the process with status 1 from inside the
    /// event loop, before winit or rinch hear of it, so nothing is dropped
    /// and neither `run` nor `try_run` returns.
    ///
    /// The event loop that cannot be made is handled the same way (#2761):
    /// with no display server to connect to, or a compositor that went away
    /// before the loop was made, the error is logged in one line and the
    /// process exits with status 1 (it used to panic, status 101).
    ///
    /// # Panics
    ///
    /// The rest of startup does not report failure — it aborts. The window
    /// and the software presentation surface each panic rather than returning
    /// an error. With the `gpu-renderer` feature a GPU that will not start (adapter,
    /// surface, device, renderer) falls back to the software renderer under
    /// [`Renderer::Auto`](crate::Renderer::Auto) and panics under
    /// [`Renderer::Gpu`](crate::Renderer::Gpu) or with an app-configured
    /// device (see [`renderer`](App::renderer)).
    ///
    /// With [`external_gpu`](App::external_gpu), the supplied adapter being
    /// unable to present to rinch's window surface panics too. An error the
    /// event loop reports *while running* is not a panic (see above).
    #[cfg(feature = "windowing")]
    pub fn run(self) {
        crate::shell::run_error::exit_on_run_error(self.try_run());
    }

    /// [`run`](App::run), answering how the event loop ended instead of
    /// exiting on an error.
    ///
    /// `Ok(())` when the app closed (its main window, the last window or
    /// [`exit`](crate::exit), as [`exit_when`](App::exit_when) says).
    /// [`RunError::DisplayLost`](crate::RunError::DisplayLost) when the
    /// Wayland compositor went away, while the app ran or after it connected
    /// but before the event loop ran;
    /// [`RunError::NoDisplay`](crate::RunError::NoDisplay) when there was no
    /// display server to connect to at all (nothing named, or a stale
    /// `WAYLAND_DISPLAY` or `DISPLAY`; Linux only); and
    /// [`RunError::EventLoop`](crate::RunError::EventLoop) for any other
    /// error of the event loop, or of building it. The rule between the
    /// first two is whether a connection was ever made (#2761). When the
    /// loop ran, the app's state has been dropped by the time this returns,
    /// as at an ordinary exit: its windows are closed and their documents
    /// disposed (their effects' cleanups and the scopes' `on_cleanup`s have
    /// run); when it could not be made, none of that was built yet. An X
    /// server going away is not reported here: see [`run`](App::run).
    ///
    /// Write the error with `writeln!` and ignore its result rather than
    /// `eprintln!`: when the session has ended, stderr may be a pipe nobody
    /// reads any more, and `eprintln!` panics on it.
    ///
    /// ```ignore
    /// use std::io::Write;
    ///
    /// fn main() {
    ///     if let Err(e) = App::new(app).try_run() {
    ///         let _ = writeln!(std::io::stderr(), "{e}");
    ///         std::process::exit(if e.is_display_lost() { 0 } else { 1 });
    ///     }
    /// }
    /// ```
    ///
    /// The other startup failures (a window or a presentation surface that
    /// cannot be made) panic, as [`run`](App::run)'s do (see its
    /// `# Panics`).
    #[cfg(feature = "windowing")]
    pub fn try_run(self) -> Result<(), crate::RunError> {
        // `rinch::exit` before the run (a signal-watching thread's, #3015):
        // the app ends before it connects to the display or opens a window.
        if crate::shell::rinch_runtime::take_exit_before_run() {
            return Ok(());
        }
        let Startup {
            component,
            props,
            theme,
            fonts,
            font_defaults,
            menus,
            spellcheck,
            renderer,
            shortcut_matching,
            chord_yield,
            prefer_x11,
            exit_when,
            windowless,
            title_tooltips,
            font_warmup,
            smooth_wheel_scroll,
            devtools,
            debug_interface,
            vsync,
            text_rasterization,
            #[cfg(feature = "gpu-renderer")]
            gpu_antialiasing,
            activation_policy,
            #[cfg(feature = "gpu-renderer")]
            gpu,
        } = self.into_startup();
        // The app's text rasterization; the OS's settings it follows are read
        // when the first glyph run is painted (#2286, #2773).
        crate::text_rasterization::set_text_rasterization(text_rasterization);
        // The OS's reduced-motion setting (#2209), read again whenever a
        // window gains focus.
        crate::reduced_motion::refresh_from_os();
        // The app's spellcheck settings, for its text fields and editors
        // (#2862). Installing them loads nothing: the dictionaries are read
        // on the checking thread when the user first edits or focuses a field.
        rinch_spell::runtime::set_config(spellcheck);
        #[cfg(feature = "gpu-renderer")]
        if let Some(mode) = gpu_antialiasing {
            crate::gpu_antialiasing::set_gpu_antialiasing(mode);
        }
        // Before any font context exists: the documents' contexts take the
        // generic families from it (#2255).
        if let Some(defaults) = font_defaults {
            rinch_dom::fonts::set_font_defaults(defaults);
        }
        // Fonts installed while the app runs (#2486, #2537), unless the app
        // chose: a thread that waits for the first document's font scan
        // (#2774), then sleeps in inotify until something it read changes.
        crate::system_font_watch::start_default();
        crate::app::set_title_tooltips_default(title_tooltips);
        // After the font defaults above: the warm-up heads its lists with
        // them.
        crate::font_warmup::app_started(font_warmup);
        crate::app::set_smooth_wheel_scroll_default(smooth_wheel_scroll);
        crate::app::set_devtools_default(devtools);
        crate::shell::rinch_runtime::set_debug_interface_default(debug_interface);
        // Stored before the window exists: its renderer takes it (#2095).
        if let Some(on) = vsync {
            crate::window::set_vsync(on);
        }

        crate::shell::renderer::set_requested(renderer);
        crate::shell::overlay_window::set_prefer_x11(prefer_x11);
        crate::window_handle::set_exit_when(exit_when);
        crate::window_handle::set_windowless(windowless);
        crate::menu::set_shortcut_matching(shortcut_matching);
        crate::menu::set_chord_yield(chord_yield);
        if let Some(policy) = activation_policy {
            crate::shell::activation::set_requested(policy);
        }
        #[cfg(feature = "gpu-renderer")]
        if let Some(gpu) = gpu {
            crate::shell::desktop::set_gpu_init(gpu);
        }

        // A no-op when the `theme` feature is off, which is why this is not
        // itself feature-gated.
        crate::setup_theme_css(&theme);

        // Every Linux window draws its menu bar in the document, and so does
        // every window of a build without the native menus (#2435).
        #[cfg(any(target_os = "linux", not(feature = "native-menus")))]
        {
            #[cfg(not(target_os = "linux"))]
            if menus.is_some() {
                tracing::debug!(
                    "rinch was built without `native-menus`: the menu bar is drawn in \
                     the window's document"
                );
            }
            run_desktop_in_app_bar(component, props, menus, &fonts)
        }

        // A borderless window has no frame for a Win32 menu bar: it gets
        // the one drawn in the document, as on Linux, and the native bar is
        // built (it registers the items and their chords) but never attached.
        #[cfg(all(target_os = "windows", feature = "native-menus"))]
        if props.borderless && menus.is_some() {
            crate::menu::set_menu_bar_in_document();
            return run_desktop_in_app_bar(component, props, menus, &fonts);
        }

        #[cfg(all(not(target_os = "linux"), feature = "native-menus"))]
        {
            let native_menu = menus.map(|menus| {
                let refs: Vec<(&str, crate::menu::Menu)> = menus
                    .iter()
                    .map(|(label, menu)| (label.as_str(), menu.clone()))
                    .collect();
                crate::menu::build_native_menu_bar(refs)
            });
            crate::shell::rinch_runtime::try_run_rinch_with_window_props_and_menu(
                component,
                props,
                native_menu,
                &fonts,
            )
        }
    }

    /// Start the application on Android.
    ///
    /// Takes over the thread and returns only when the Activity's loop ends.
    ///
    /// The same configuration methods apply everywhere; only the terminal
    /// differs, so an app that targets both platforms writes the chain once.
    /// Android draws into the Activity's own surface and takes its label from
    /// the manifest, so [`title`](App::title), [`size`](App::size) and
    /// [`window_props`](App::window_props) are all inert here — only
    /// [`theme`](App::theme) and the component apply. [`menu`](App::menu),
    /// where it exists, warns and is ignored.
    ///
    /// Call it from `android_main`:
    ///
    /// ```ignore
    /// #[unsafe(no_mangle)]
    /// fn android_main(android_app: AndroidApp) {
    ///     App::new(app).run_android(android_app);
    /// }
    /// ```
    #[cfg(all(feature = "android", target_os = "android"))]
    pub fn run_android(self, android_app: android_activity::AndroidApp) {
        if self.menus.is_some() {
            tracing::warn!(
                "App::menu() has no effect on Android — there is no window menu bar; \
                 the menu was ignored"
            );
        }

        if let Some(defaults) = self.font_defaults {
            rinch_dom::fonts::set_font_defaults(defaults);
        }
        crate::text_rasterization::set_text_rasterization(self.text_rasterization);
        rinch_spell::runtime::set_config(self.spellcheck.clone());
        crate::setup_theme_css(&self.theme.unwrap_or_default());
        crate::shell::android_runtime::run_component(android_app, self.component, &self.fonts);
    }
}
