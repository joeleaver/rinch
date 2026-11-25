//! The [`App`] builder's options for the window, its content and the
//! app's behaviour: title, size, theme, fonts, images, menus, shortcuts,
//! tooltips, scrolling, spellcheck, devtools and the debug interface,
//! reduced motion, the windowing platform, activation, exit and popups.

use super::*;

impl<F, M> App<F, M>
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    /// Set the window title.
    ///
    /// Wins over the title carried by [`window_props`](App::window_props)
    /// regardless of which is called first, so a title set here is never lost.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the window's initial size in logical pixels.
    ///
    /// Wins over the size carried by [`window_props`](App::window_props)
    /// regardless of which is called first.
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.size = Some((width, height));
        self
    }

    /// Configure the theme (colors, radius, dark mode).
    ///
    /// Without this, the default theme CSS is loaded — which is what makes
    /// `rinch-components` visible out of the box.
    pub fn theme(mut self, theme: ThemeProviderProps) -> Self {
        self.theme = Some(theme);
        self
    }

    /// Bundle typefaces the build carries in its own binary.
    ///
    /// The platform's font list is not a promise anyone can keep — a phone has
    /// whatever the OEM shipped, a Linux desktop whatever the user installed —
    /// so an app with a designed identity, or one whose content needs a real
    /// monospaced face, has to carry the file. This is where those bytes come
    /// in.
    ///
    /// They are part of the startup configuration rather than something to
    /// register beforehand because a face has to be in the font context
    /// **before the first layout pass**, or the first frame is measured against
    /// a fallback and then reflows. Passing them here leaves no ordering to get
    /// wrong.
    ///
    /// See [`AppFont`](crate::font::AppFont) for what a face is reachable
    /// *as*. The short version: registering a file is enough for
    /// `font-family: <the name in the file>`, and a generic like `monospace`
    /// is a slot rather than a name, so a bundled face answers it only by
    /// asking to.
    ///
    /// Calling this more than once registers all of them, in call order —
    /// unlike [`title`](App::title) or [`menu`](App::menu), a second call adds
    /// rather than replaces, because two font lists have nothing to disagree
    /// about and silently dropping the first would be the worse default.
    ///
    /// ```ignore
    /// use rinch::prelude::*;
    ///
    /// const FACES: &[AppFont] = &[
    ///     AppFont::serif(include_bytes!("../assets/Newsreader.ttf")),
    ///     AppFont::monospace(include_bytes!("../assets/DejaVuSansMono.ttf")),
    /// ];
    ///
    /// App::new(app).title("My App").fonts(FACES).run();
    /// ```
    ///
    /// Applies on Android too — the one configuration method besides
    /// [`theme`](App::theme) that does, and the platform that needs it most.
    ///
    /// On a web build (`rinch_web::AppExt`) the faces are accepted and
    /// ignored: the browser loads fonts from the page's CSS (`@font-face`), so
    /// a cross-platform chain needs no `cfg` around this call (#1769).
    pub fn fonts(mut self, fonts: &[crate::font::AppFont]) -> Self {
        self.fonts.extend_from_slice(fonts);
        self
    }

    /// The browser's font settings for this app: the default font a
    /// `font-family` list whose every family is missing falls back to, and
    /// the families `serif` and `sans-serif` mean (#2255). See
    /// [`FontDefaults`](crate::font::FontDefaults).
    ///
    /// Without it they are Chrome's: Times New Roman for the default font and
    /// `serif`, Arial for `sans-serif` (Times, Times and Helvetica on macOS),
    /// each where the host has it. So `font-family: Oswald` on a host without
    /// Oswald is drawn in Times New Roman, as in Chrome, and in the
    /// `sans-serif` face on a host without Times New Roman either, as Chrome
    /// does there. An app whose own face should take that role names it:
    ///
    /// ```ignore
    /// use rinch::prelude::*;
    ///
    /// App::new(app)
    ///     .fonts(&[AppFont::sans_serif(include_bytes!("../assets/Inter.ttf"))])
    ///     .font_defaults(FontDefaults::default().with_standard("Inter, sans-serif"))
    ///     .run();
    /// ```
    ///
    /// A theme's `font-family` (rinch's ends in `sans-serif`) always resolves,
    /// so it never reaches the default font; only an element whose own list
    /// is exhausted does. A face claimed for a generic with
    /// [`AppFont`](crate::font::AppFont) still heads that slot, ahead of
    /// `serif`/`sans_serif` here.
    ///
    /// A second call replaces the first. On a web build the browser's own
    /// settings apply and this is ignored.
    pub fn font_defaults(mut self, defaults: crate::font::FontDefaults) -> Self {
        self.font_defaults = Some(defaults);
        self
    }

    /// Answer for an image URL scheme the app alone can resolve.
    ///
    /// Every `<img src>` and `background-image: url(…)` whose source starts
    /// with `scheme:` is loaded by `loader`; every other source (a path, and
    /// `http(s)` with the `image-network` feature) still goes to rinch's
    /// default loader, so there is nothing to forward. Call it once per
    /// scheme; a second call for the same scheme replaces the first.
    ///
    /// The loader returns the picture's **encoded** bytes (a PNG, JPEG, GIF or
    /// WebP file's contents; rinch decodes them) and is called on a background
    /// thread spawned for that load, never the UI thread, so it may block on
    /// I/O. A failure is cached for that source: when the bytes can arrive
    /// later, answer [`ImageLoadResult::Failed`](crate::image::ImageLoadResult::Failed)
    /// now and call [`rinch::image::reload_image`](crate::image::reload_image)
    /// with the same source once they are there. See
    /// [`ImageLoader`](crate::image::ImageLoader).
    ///
    /// ```ignore
    /// use rinch::image::ImageLoadResult;
    /// use rinch::prelude::*;
    ///
    /// App::new(app)
    ///     .image_scheme("myapp-blob", |src: &str| match blobs::read(src) {
    ///         Some(bytes) => ImageLoadResult::Loaded(bytes),
    ///         None => ImageLoadResult::Failed(format!("{src} is not here yet")),
    ///     })
    ///     .run();
    /// ```
    ///
    /// The registration is process-wide and takes effect at this call
    /// ([`register_image_scheme`](crate::image::register_image_scheme) is the
    /// same thing without a builder, for an embedded or Android-only host), so
    /// it covers every window the app opens.
    ///
    /// # Panics
    ///
    /// If `scheme` is not a URL scheme of two or more characters, or is `data`.
    pub fn image_scheme(self, scheme: &str, loader: impl crate::image::ImageLoader) -> Self {
        crate::image::register_image_scheme(scheme, loader);
        self
    }

    /// Set the full window configuration.
    ///
    /// [`title`](App::title) and [`size`](App::size) are applied *over* this,
    /// so `.title("X").window_props(p)` still gets the title `"X"` rather than
    /// `p`'s. Every other field comes from `props`.
    pub fn window_props(mut self, props: WindowProps) -> Self {
        self.props = props;
        self
    }

    /// Add a menu bar. Each `(label, Menu)` pair becomes a top-level
    /// submenu. Calling this twice replaces the menu rather than appending to
    /// it.
    ///
    /// On the desktop it is the native menu bar (the DOM bar on Linux); on the
    /// web, run through `rinch_web::AppExt`, it is the DOM menu bar
    /// `rinch_web::mount_with_menu_bar` draws (#1769). Android has no window
    /// menu bar: `run_android` warns and ignores it.
    ///
    /// On `wasm32` it returns an `App<F, WithMenuBar>` rather than `Self`, so
    /// that only an app that has a menu bar links it (see [`NoMenuBar`]); a
    /// chain reads the same either way.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    pub fn menu(mut self, menus: Vec<(&str, crate::menu::Menu)>) -> Self {
        self.menus = Some(owned_menus(menus));
        self
    }

    /// See the non-`wasm32` [`menu`](App::menu): the same, in the
    /// [`WithMenuBar`] state.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub fn menu(self, menus: Vec<(&str, crate::menu::Menu)>) -> App<F, WithMenuBar> {
        App {
            component: self.component,
            props: self.props,
            title: self.title,
            size: self.size,
            theme: self.theme,
            fonts: self.fonts,
            font_defaults: self.font_defaults,
            menus: Some(owned_menus(menus)),
            spellcheck: self.spellcheck,
            #[cfg(feature = "windowing")]
            renderer: self.renderer,
            #[cfg(feature = "windowing")]
            shortcut_matching: self.shortcut_matching,
            #[cfg(feature = "windowing")]
            chord_yield: self.chord_yield,
            #[cfg(feature = "windowing")]
            prefer_x11: self.prefer_x11,
            #[cfg(feature = "windowing")]
            exit_when: self.exit_when,
            #[cfg(feature = "windowing")]
            windowless: self.windowless,
            #[cfg(feature = "windowing")]
            title_tooltips: self.title_tooltips,
            #[cfg(feature = "windowing")]
            font_warmup: self.font_warmup,
            #[cfg(feature = "windowing")]
            smooth_wheel_scroll: self.smooth_wheel_scroll,
            #[cfg(feature = "windowing")]
            devtools: self.devtools,
            #[cfg(feature = "windowing")]
            debug_interface: self.debug_interface,
            #[cfg(feature = "windowing")]
            vsync: self.vsync,
            #[cfg(feature = "windowing")]
            activation_policy: self.activation_policy,
            #[cfg(any(feature = "windowing", feature = "android"))]
            text_rasterization: self.text_rasterization,
            #[cfg(feature = "gpu-renderer")]
            gpu_antialiasing: self.gpu_antialiasing,
            #[cfg(feature = "gpu-renderer")]
            gpu: self.gpu,
            menu_bar: PhantomData,
        }
    }

    /// Choose how a menu's letter chords are matched against a keystroke
    /// (issue #1170).
    ///
    /// The default, [`ShortcutMatching::LayoutAware`](crate::menu::ShortcutMatching::LayoutAware),
    /// matches a letter chord (`"Ctrl+Z"`) to the *character* the active
    /// keyboard layout types for the pressed key — so Ctrl+Z fires from
    /// whichever key is labelled Z, on QWERTZ and AZERTY as well as QWERTY —
    /// falling back to the physical key when the layout types no Latin
    /// letter there at all (Cyrillic, Thai, …). Punctuation and digit chords
    /// are unaffected either way; they have always matched the physical key.
    ///
    /// [`ShortcutMatching::Physical`](crate::menu::ShortcutMatching::Physical)
    /// turns that off and matches every chord by physical key alone, as rinch
    /// did before #1170 — the escape hatch for a user who wants shortcut
    /// *positions* to stay where they are on a remapped or non-Latin layout
    /// rather than follow the character (the same choice VS Code's
    /// `keyboard.dispatch` setting and JetBrains' "use national layout for
    /// shortcuts" toggle offer).
    ///
    /// Applied at [`run`](App::run); on the web, call
    /// [`rinch_web::set_shortcut_matching`](https://docs.rs/rinch-web)
    /// (`rinch::menu::set_shortcut_matching`) directly before mounting.
    #[cfg(feature = "windowing")]
    pub fn shortcut_matching(mut self, mode: crate::menu::ShortcutMatching) -> Self {
        self.shortcut_matching = mode;
        self
    }

    /// Which focused controls a menu chord with no Ctrl/Cmd/Alt leaves its key
    /// to (issue #1290). By default every focused control keeps the keys it
    /// acts on, as in a browser: a text field its typing and editing keys, a
    /// `<select>` its arrows, letters, Space and Enter, a button or `tabindex`
    /// control Space and Enter, a `RenderSurface` every bare key.
    ///
    /// [`ChordYield::TextOnly`](crate::menu::ChordYield::TextOnly) is the
    /// escape hatch: only text targets keep their keys, and a bare chord fires
    /// from a focused select, button or surface, as before #1290. It also
    /// decides which window-bar items get a native accelerator on macOS and
    /// Windows ([`crate::menu::native_accelerator_allowed`], #1284).
    ///
    /// Applied at [`run`](App::run), before the menu is built; on the web, call
    /// `rinch::menu::set_chord_yield` directly before mounting.
    #[cfg(feature = "windowing")]
    pub fn chord_yield(mut self, mode: crate::menu::ChordYield) -> Self {
        self.chord_yield = mode;
        self
    }

    /// Whether an element's `title` shows as a tooltip after the pointer
    /// rests over it (issue #1646). On by default; `false` turns it off for
    /// every window of the app. (Android has no resting pointer and never
    /// shows one; an embedded `RinchContext` uses
    /// [`RinchApp::set_title_tooltips`](crate::app::RinchApp::set_title_tooltips)
    /// through `app_mut()`.)
    #[cfg(feature = "windowing")]
    pub fn title_tooltips(mut self, enabled: bool) -> Self {
        self.title_tooltips = enabled;
        self
    }

    /// When the system's generic font families (`sans-serif`, `system-ui`,
    /// `serif`, `monospace`, headed by the app's own) are looked up on a
    /// thread of their own (issue #2552):
    /// [`FontWarmup::AfterFirstFrame`](crate::font::FontWarmup::AfterFirstFrame)
    /// by default, [`AtStart`](crate::font::FontWarmup::AtStart) for an app
    /// whose first text comes late (a hidden launcher, a notification
    /// daemon), or [`Off`](crate::font::FontWarmup::Off). Text with a glyph
    /// the first fonts of its stack lack (a menu's `✓`) otherwise looks them
    /// up one by one the first time, on the UI thread: a panel's first menu
    /// took 80–560 ms to show.
    #[cfg(feature = "windowing")]
    pub fn font_warmup(mut self, when: crate::font::FontWarmup) -> Self {
        self.font_warmup = when;
        self
    }

    /// Whether a mouse wheel's notch glides to where it scrolls, as
    /// Chrome's does, rather than jumping there in one frame (issue #2287).
    /// On by default; `false` makes every notch scroll its whole distance at
    /// once, for every window of the app. Only a *line* wheel glides: a
    /// trackpad or a high-resolution wheel reports pixels, and scrolls
    /// directly either way. Change it while the app runs with
    /// [`RinchApp::set_smooth_wheel_scroll`](crate::app::RinchApp::set_smooth_wheel_scroll)
    /// (an embedded `RinchContext` reaches it through `app_mut()`).
    ///
    /// rinch does not read the OS's reduced-motion setting on the desktop
    /// yet (#2209); an app that does can pass its answer here.
    #[cfg(feature = "windowing")]
    pub fn smooth_wheel_scroll(mut self, enabled: bool) -> Self {
        self.smooth_wheel_scroll = enabled;
        self
    }

    /// The app's spellchecking (#2862): `false` turns it off everywhere;
    /// a [`SpellcheckConfig`](crate::spellcheck::SpellcheckConfig) picks the
    /// dictionaries, languages, personal dictionary, skip rules, the pause
    /// after typing, and whether checking waits for the user's interaction
    /// and a blurred `<input>` hides its marks. On by default, as in Chrome:
    /// misspelled words in a `<textarea>`, a prose `<input>` (no type,
    /// `text`, `search`) and the rich-text editor get a red squiggle once the
    /// user has focused or edited it, checked by the operating system's
    /// spellchecker on macOS and Windows and against the system's Hunspell
    /// dictionaries for the user's locale elsewhere (none installed: nothing
    /// is marked; [`Backend`](crate::spellcheck::Backend) chooses). A
    /// `spellcheck: false` attribute or component prop turns it off for one
    /// field or a subtree. On the web the browser checks, and `false` here
    /// (or a later [`set_enabled(false)`](crate::spellcheck::set_enabled))
    /// sets `spellcheck="false"` on `<body>`.
    ///
    /// ```ignore
    /// use rinch::spellcheck::SpellcheckConfig;
    /// App::new(app).spellcheck(false).run();
    /// App::new(app).spellcheck(SpellcheckConfig::new().languages(["en_GB"])).run();
    /// ```
    ///
    /// Change it while the app runs with
    /// [`rinch::spellcheck::configure`](crate::spellcheck::configure).
    pub fn spellcheck(mut self, config: impl Into<rinch_spell::runtime::SpellcheckConfig>) -> Self {
        self.spellcheck = config.into();
        self
    }

    /// Whether the built-in DevTools key works (issue #2689): F12 toggles the
    /// DevTools window, in every window of the app, when nothing in the app
    /// claimed the key. On in debug builds and off in release builds unless
    /// set ([`DEVTOOLS_DEFAULT`](crate::app::DEVTOOLS_DEFAULT),
    /// `cfg!(debug_assertions)`); `true` keeps it in a release build. Off,
    /// F12 is not consumed: it goes on to the rest of rinch's key handling
    /// like any other key. An app's own key handlers and menu shortcuts run
    /// before it either way, so they can already take F12 with DevTools on.
    /// Inspect mode has no key (#3062): it is the DevTools window's Inspect
    /// button.
    ///
    /// `RINCH_DEVTOOLS=on` or `off` in the environment overrides this, so an
    /// app that ships with DevTools off can be debugged without a rebuild.
    /// Change it while the app runs with
    /// [`RinchApp::set_devtools`](crate::app::RinchApp::set_devtools) (an
    /// embedded `RinchContext` reaches it through `app_mut()`).
    #[cfg(feature = "windowing")]
    pub fn devtools(mut self, enabled: bool) -> Self {
        self.devtools = enabled;
        self
    }

    /// Start the debug interface, the server the rinch MCP tools drive (the
    /// `debug` feature), without `RINCH_DEBUG=1` in the environment (issue
    /// #2481). Off by default: an app built with the feature listens only when
    /// asked to. `RINCH_DEBUG` still wins either way (`0` keeps it off, `1`
    /// turns it on), so whoever starts the process can decide. Without the
    /// `debug` feature this does nothing.
    ///
    /// Only the user running the app can drive it: a client must send the
    /// per-session token from the app's discovery file, which only that user
    /// can read (`~/.rinch/debug/<pid>.json`, mode 0600).
    #[cfg(feature = "windowing")]
    pub fn debug_interface(mut self, on: bool) -> Self {
        self.debug_interface = on;
        self
    }

    /// Force the reduced-motion preference (issue #2209): `Some(true)` makes
    /// `@media (prefers-reduced-motion: reduce)` match in every window,
    /// `Some(false)` makes `no-preference` match, and `None` (the default)
    /// follows the OS's setting, as Chrome does. The theme's reduced-motion
    /// rule (`Theme::respect_reduced_motion`, on by default) then shortens
    /// every transition and animation to 0.01ms.
    ///
    /// The same as [`rinch::set_reduced_motion`](crate::reduced_motion::set_reduced_motion),
    /// which it calls and which also changes it while the app runs; read it
    /// reactively with [`rinch::prefers_reduced_motion`](crate::reduced_motion::prefers_reduced_motion).
    #[cfg(any(feature = "windowing", feature = "android"))]
    pub fn reduced_motion(self, value: Option<bool>) -> Self {
        crate::reduced_motion::set_reduced_motion(value);
        self
    }

    /// On Linux, run on X11 — under a Wayland session, through XWayland —
    /// whenever an X server is reachable (`DISPLAY` is set); otherwise on
    /// whatever winit picks, as without this (issue #1519).
    ///
    /// The backend is chosen once per process, when the event loop is built,
    /// so this is an application option, not a window one. Its use is an
    /// overlay: [`WindowProps::non_activating`] and
    /// [`WindowProps::skip_taskbar`] work on X11 (`WM_HINTS.input = False`,
    /// `_NET_WM_STATE_SKIP_TASKBAR`) and cannot be had from xdg-toplevel on
    /// native Wayland. Ignored on every other platform.
    #[cfg(feature = "windowing")]
    pub fn prefer_x11(mut self, prefer: bool) -> Self {
        self.prefer_x11 = prefer;
        self
    }

    /// The xdg-activation token the app was launched with, for its first
    /// window (#2817): `window`'s first shown OS window is created with it
    /// (winit's `with_activation_token`), so that on Wayland the compositor
    /// gives it the focus. `None` gives none.
    ///
    /// For an app that takes its token first thing in `main`
    /// ([`take_launch_activation_token`](crate::windows::take_launch_activation_token)),
    /// which a single-instance app does to hand it to a copy already running.
    /// When no copy is running the token is this launch's own, and goes
    /// here, as GTK and Qt give theirs to their first window: taken from the
    /// environment, it would otherwise be lost. An app that never takes it
    /// needs none of this: winit does not read `XDG_ACTIVATION_TOKEN` itself,
    /// but a new window is given the focus anyway on most compositors (KWin
    /// and sway), and the token decides it only where focus-stealing
    /// prevention holds a new window back.
    ///
    /// A main window that starts hidden ([`WindowProps::visible`] `false`)
    /// is given it at its first show. Ignored off Wayland, by a layer-shell
    /// window and by a non-activating one.
    ///
    /// ```ignore
    /// fn main() {
    ///     // SAFETY: nothing else runs yet.
    ///     let token = unsafe { rinch::windows::take_launch_activation_token() };
    ///     if let Some(running) = my_ipc::connect_to_running_copy() {
    ///         running.send_open(std::env::args().nth(1), token);
    ///         return;
    ///     }
    ///     App::new(app).activation_token(token).run();
    /// }
    /// ```
    #[cfg(feature = "windowing")]
    pub fn activation_token(self, token: Option<String>) -> Self {
        crate::shell::activation_tokens::set_launch_token(token);
        self
    }

    /// When the event loop ends (#2416). `App::new` defaults to
    /// [`ExitWhen::MainWindowCloses`](crate::ExitWhen::MainWindowCloses), as
    /// every app has always run: closing the main window ends the app, whatever
    /// else [`open_window`](crate::open_window) opened.
    /// [`ExitWhen::LastWindowCloses`](crate::ExitWhen::LastWindowCloses) lets
    /// the other windows outlive it, and
    /// [`ExitWhen::Explicitly`](crate::ExitWhen::Explicitly) runs until
    /// [`exit`](crate::exit).
    #[cfg(feature = "windowing")]
    pub fn exit_when(mut self, when: crate::ExitWhen) -> Self {
        self.exit_when = when;
        self
    }

    /// Whether the app may open popup windows (#2552, on by default): with
    /// `false`, [`popups_supported`](crate::popups_supported) answers
    /// `false` and [`open_popup`](crate::open_popup) `Unsupported`, so every
    /// component and every caller that checks takes its in-window form.
    #[cfg(feature = "windowing")]
    pub fn popup_windows(self, on: bool) -> Self {
        crate::popup::set_turned_off(!on);
        self
    }

    /// How many closed popups' GPU renderers are kept for the next popups
    /// (#2552; [`DEFAULT_RENDERERS_KEPT`](crate::popup::DEFAULT_RENDERERS_KEPT),
    /// 3, by default). A GPU popup builds Vello's renderer when it opens
    /// (about 5 ms on a GPU, 55–85 ms on a software Vulkan driver), unless
    /// one a closed popup left is kept: then it only makes a surface. The
    /// kept ones go once the popups' idle period (`WindowProps::idle_trim`,
    /// two seconds by default) has passed with none kept, and all of them
    /// when the app ends. `0` keeps none. The software renderer keeps
    /// nothing either way.
    #[cfg(feature = "windowing")]
    pub fn popup_renderers_kept(self, count: usize) -> Self {
        crate::popup::set_renderers_kept(count);
        self
    }

    /// Set the macOS activation policy (issue #1528): whether the app has a
    /// Dock icon and a menu bar of its own.
    ///
    /// A tray-resident app — a [tray icon](crate::tray) plus a window it shows
    /// on request, typically started with `WindowProps::visible: false` —
    /// wants [`ActivationPolicy::Accessory`](crate::ActivationPolicy::Accessory):
    /// no Dock icon and no app menu, while its window can still be shown,
    /// focused and typed into. `visible: false` alone hides the window, not the
    /// Dock icon, and does not imply this; pair the two yourself.
    ///
    /// Not calling this leaves the policy to winit, unchanged from before: an
    /// unbundled executable is `Regular`, and a bundled `.app` follows its
    /// `Info.plist` (`LSUIElement`). Calling it overrides the `Info.plist`.
    ///
    /// macOS only. On Windows and Linux it does nothing: there an app has a
    /// taskbar entry exactly while it has a visible window, so a tray-only app
    /// needs nothing beyond `visible: false`. Applied at [`run`](App::run).
    #[cfg(feature = "windowing")]
    pub fn activation_policy(mut self, policy: crate::shell::activation::ActivationPolicy) -> Self {
        self.activation_policy = Some(policy);
        self
    }
}
