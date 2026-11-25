//! The [`App`] builder — one entry point that composes every startup option.
//!
//! rinch used to have seven `run_*` functions, and each one that wanted to
//! combine features had to grow a parameter for every other feature. The
//! combinations were not merely awkward to write, they were *unexpressible*:
//! `run_with_menu` could not take window props, `run_with_window_props` could
//! not take a menu bar without a second function, and neither could take
//! anything that arrived later. Issue #493.
//!
//! `App` is one type with one method per feature:
//!
//! ```ignore
//! use rinch::prelude::*;
//!
//! App::new(my_component)
//!     .title("My App")
//!     .size(800, 600)
//!     .theme(theme)
//!     .menu(vec![("File", file_menu)])
//!     .run();
//! ```
//!
//! Adding the next feature is one more method, and it composes with everything
//! already here. The seven `run_*` functions still exist as deprecated shims
//! over this builder, so nothing breaks — but they are shims, not a second
//! implementation: every *public* desktop entry point runs the same code below.
//! Below it sits the crate-private
//! `shell::rinch_runtime::try_run_rinch_with_window_props_and_menu`, which is
//! what this dispatches into; it resolves no window props and installs no
//! theme CSS. With `native-menus` its exiting twin,
//! `shell::rinch_runtime::run_rinch_with_window_props_and_menu`, is public as
//! well (it takes a `muda::Menu`, so a build without the native menus has
//! none, #2435).

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::{ThemeProviderProps, WindowProps};
use std::marker::PhantomData;

/// `App::menu`'s labels, owned.
fn owned_menus(menus: Vec<(&str, crate::menu::Menu)>) -> Vec<(String, crate::menu::Menu)> {
    menus
        .into_iter()
        .map(|(label, menu)| (label.to_string(), menu))
        .collect()
}

/// A rinch application, configured by builder methods and started by a terminal
/// method ([`run`](App::run) on desktop, `run_android` on Android — the latter
/// exists only when building for Android, so it is not linked here — and
/// `rinch_web::AppExt::run` in the browser, #1769).
///
/// Every configuration method is independent, so any combination is
/// expressible. The `F` parameter is the root component; it is usually an
/// unnameable closure, but `App<Box<dyn FnOnce(&mut RenderScope) -> NodeHandle>>`
/// is a real type, so an `App` can be stored in a struct or passed to a
/// function when configuration has to be built up across call sites. Nothing happens until a terminal method is called; on the desktop a terminal
/// method takes over the thread and returns only when the application exits its
/// event loop, and in the browser it mounts the app and returns.
///
/// One builder chain serves both targets; a web build adds one import:
///
/// ```ignore
/// #[cfg(target_arch = "wasm32")]
/// use rinch_web::AppExt as _;
///
/// fn main() {
///     App::new(app).title("Hello").size(640, 480).theme(theme()).run();
/// }
/// ```
///
/// Without that import a web build fails with `error[E0599]: no method named
/// `run` found for struct `rinch::App``: `run` is the desktop's inherent
/// method, and the compiler names `rinch_web::AppExt` as the fix only when the
/// crate already mentions `rinch_web` somewhere. Add the `use` above.
///
/// The second type parameter is the menu-bar state ([`NoMenuBar`] by default):
/// on `wasm32`, [`menu`](App::menu) moves it to [`WithMenuBar`] so only an app
/// that has a menu bar links one.
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
///
/// #[component]
/// fn app() -> NodeHandle {
///     rsx! { div { "Hello" } }
/// }
///
/// fn main() {
///     App::new(app).title("Hello").size(640, 480).run();
/// }
/// ```
#[must_use = "an App does nothing until a terminal method (`run` on desktop or, \
              through `rinch_web::AppExt`, on the web; `run_android` on Android) is called"]
pub struct App<F, M = NoMenuBar> {
    pub(crate) component: F,
    /// The window configuration. [`title`](App::title) and [`size`](App::size)
    /// are held separately and applied over this at startup, so setting them
    /// never silently loses to a later [`window_props`](App::window_props).
    pub(crate) props: WindowProps,
    pub(crate) title: Option<String>,
    pub(crate) size: Option<(u32, u32)>,
    pub(crate) theme: Option<ThemeProviderProps>,
    /// Faces the build carries, in the order they were added. Not an `Option`:
    /// "no bundled fonts" and "an empty list of them" are the same startup.
    pub(crate) fonts: Vec<crate::font::AppFont>,
    /// See [`font_defaults`](App::font_defaults); `None` keeps Chrome's.
    pub(crate) font_defaults: Option<crate::font::FontDefaults>,
    pub(crate) menus: Option<Vec<(String, crate::menu::Menu)>>,
    /// See [`spellcheck`](App::spellcheck).
    pub(crate) spellcheck: rinch_spell::runtime::SpellcheckConfig,
    #[cfg(feature = "windowing")]
    pub(crate) renderer: crate::shell::renderer::Renderer,
    /// See [`shortcut_matching`](App::shortcut_matching).
    #[cfg(feature = "windowing")]
    pub(crate) shortcut_matching: crate::menu::ShortcutMatching,
    /// See [`chord_yield`](App::chord_yield).
    #[cfg(feature = "windowing")]
    pub(crate) chord_yield: crate::menu::ChordYield,
    /// See [`prefer_x11`](App::prefer_x11).
    #[cfg(feature = "windowing")]
    pub(crate) prefer_x11: bool,
    /// See [`exit_when`](App::exit_when).
    #[cfg(feature = "windowing")]
    pub(crate) exit_when: crate::ExitWhen,
    /// See [`windowless`](App::windowless).
    #[cfg(feature = "windowing")]
    pub(crate) windowless: bool,
    /// See [`title_tooltips`](App::title_tooltips).
    #[cfg(feature = "windowing")]
    pub(crate) title_tooltips: bool,
    /// See [`font_warmup`](App::font_warmup).
    #[cfg(feature = "windowing")]
    pub(crate) font_warmup: crate::font::FontWarmup,
    /// See [`smooth_wheel_scroll`](App::smooth_wheel_scroll).
    #[cfg(feature = "windowing")]
    pub(crate) smooth_wheel_scroll: bool,
    /// See [`devtools`](App::devtools).
    #[cfg(feature = "windowing")]
    pub(crate) devtools: bool,
    /// See [`debug_interface`](App::debug_interface).
    #[cfg(feature = "windowing")]
    pub(crate) debug_interface: bool,
    /// See [`vsync`](App::vsync); `None` leaves the setting as it is.
    #[cfg(feature = "windowing")]
    pub(crate) vsync: Option<bool>,
    /// See [`activation_policy`](App::activation_policy). `None` leaves it to
    /// winit (and a bundle's `Info.plist`).
    #[cfg(feature = "windowing")]
    pub(crate) activation_policy: Option<crate::shell::activation::ActivationPolicy>,
    /// See [`text_rasterization`](App::text_rasterization).
    #[cfg(any(feature = "windowing", feature = "android"))]
    pub(crate) text_rasterization: crate::text_rasterization::TextRasterization,
    /// See [`gpu_antialiasing`](App::gpu_antialiasing); `None` leaves it to
    /// `RINCH_GPU_AA` and the default.
    #[cfg(feature = "gpu-renderer")]
    pub(crate) gpu_antialiasing: Option<crate::gpu_antialiasing::GpuAntialiasing>,
    #[cfg(feature = "gpu-renderer")]
    pub(crate) gpu: Option<crate::shell::desktop::GpuInit>,
    /// Whether a web build was given a menu bar — see [`NoMenuBar`].
    pub(crate) menu_bar: PhantomData<M>,
}

/// The menu-bar state of an [`App`] that was given no menu bar: the default
/// second type parameter, so `App<F>` names it.
///
/// It exists for the web (#1769): on `wasm32` [`App::menu`] returns an
/// `App<F, WithMenuBar>`, and `rinch_web::AppExt` is implemented for each state
/// separately, so an app that never calls `.menu()` does not link the DOM menu
/// bar. On every other target `.menu()` returns `Self` and an `App` stays in
/// this state.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoMenuBar;

/// The menu-bar state of an [`App`] that [`App::menu`] gave a menu bar, on a
/// `wasm32` build. See [`NoMenuBar`].
#[derive(Debug, Clone, Copy, Default)]
pub struct WithMenuBar;

impl<F> App<F>
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    /// An application with no main window (#2416): `component` is mounted in
    /// a document with no window, which draws nothing and holds the
    /// components that open the app's windows (a `Window` element, a keyed
    /// `for` of them over `outputs()`). Content put there shows nowhere, and
    /// rinch warns once if any is. In it, `WindowHandle::current()` and
    /// `WindowHandle::main()` are `None`, and a `*_current_window` call does
    /// nothing. It runs until [`exit`](crate::exit)
    /// ([`ExitWhen::Explicitly`](crate::ExitWhen::Explicitly)), because a
    /// desktop shell with no outputs connected has no windows and must keep
    /// running; [`exit_when`](App::exit_when) changes that. Every other
    /// option (theme, fonts, renderer, `app_id`, …) applies as for
    /// [`App::new`](App::new); `title` and `size` name no window.
    #[cfg(feature = "windowing")]
    pub fn windowless(component: F) -> Self {
        let mut app = Self::new(component);
        app.windowless = true;
        app.exit_when = crate::ExitWhen::Explicitly;
        app
    }

    /// Start configuring an application around its root component.
    pub fn new(component: F) -> Self {
        Self {
            component,
            props: WindowProps::default(),
            title: None,
            size: None,
            theme: None,
            fonts: Vec::new(),
            font_defaults: None,
            menus: None,
            spellcheck: Default::default(),
            #[cfg(feature = "windowing")]
            renderer: crate::shell::renderer::Renderer::Auto,
            #[cfg(feature = "windowing")]
            shortcut_matching: crate::menu::ShortcutMatching::default(),
            #[cfg(feature = "windowing")]
            chord_yield: crate::menu::ChordYield::default(),
            #[cfg(feature = "windowing")]
            prefer_x11: false,
            #[cfg(feature = "windowing")]
            exit_when: crate::ExitWhen::MainWindowCloses,
            #[cfg(feature = "windowing")]
            windowless: false,
            #[cfg(feature = "windowing")]
            title_tooltips: true,
            #[cfg(feature = "windowing")]
            font_warmup: crate::font::FontWarmup::AfterFirstFrame,
            #[cfg(feature = "windowing")]
            smooth_wheel_scroll: true,
            #[cfg(feature = "windowing")]
            devtools: crate::app::DEVTOOLS_DEFAULT,
            #[cfg(feature = "windowing")]
            debug_interface: false,
            #[cfg(feature = "windowing")]
            vsync: None,
            #[cfg(feature = "windowing")]
            activation_policy: None,
            #[cfg(any(feature = "windowing", feature = "android"))]
            text_rasterization: Default::default(),
            #[cfg(feature = "gpu-renderer")]
            gpu_antialiasing: None,
            #[cfg(feature = "gpu-renderer")]
            gpu: None,
            menu_bar: PhantomData,
        }
    }
}

mod launch;
#[cfg(all(
    feature = "windowing",
    any(
        target_os = "linux",
        target_os = "windows",
        not(feature = "native-menus")
    )
))]
pub(crate) mod menu_bar;
mod render_options;
mod window_options;

#[cfg(all(
    test,
    feature = "windowing",
    any(
        target_os = "linux",
        target_os = "windows",
        not(feature = "native-menus")
    )
))]
mod menu_bar_2435_tests;

/// What [`App::into_web_parts`] hands a web mount (#1769). Not for apps.
#[doc(hidden)]
pub struct WebParts<F> {
    /// The root component.
    pub component: F,
    /// The document title to set, if the app named one.
    pub title: Option<String>,
    /// The theme to install (the default when none was configured).
    pub theme: ThemeProviderProps,
    /// The menu bar, if one was configured.
    pub menus: Option<Vec<(String, crate::menu::Menu)>>,
    /// The faces given to [`App::fonts`]. A browser mount ignores them (the
    /// page loads its fonts with `@font-face`); handed over so dropping them
    /// is the mount's documented choice.
    pub fonts: Vec<crate::font::AppFont>,
    /// See [`App::spellcheck`]. The browser checks on the web: a mount turns
    /// it off with `spellcheck="false"` on `<body>` when this is off.
    pub spellcheck: rinch_spell::runtime::SpellcheckConfig,
}

/// Everything a terminal method needs, with every builder rule already applied.
///
/// This is the seam that makes the chain testable. [`App::run`] itself takes
/// over the thread and can never be unit-tested, so without a pure step in
/// front of it every configuration method could silently be a no-op — `.title()`,
/// `.size()`, `.theme()` and `.menu()` all could, and a suite that only tested
/// [`resolve_window_props`] in isolation stayed green through all of them.
#[cfg(feature = "windowing")]
pub(crate) struct Startup<F> {
    pub(crate) component: F,
    /// Window props with `title` / `size` folded in and the borderless resize
    /// inset armed.
    pub(crate) props: WindowProps,
    /// The theme to install — the default when none was configured, which is
    /// what makes components visible out of the box.
    pub(crate) theme: ThemeProviderProps,
    /// Faces to register before the first layout pass.
    pub(crate) fonts: Vec<crate::font::AppFont>,
    /// See [`App::font_defaults`].
    pub(crate) font_defaults: Option<crate::font::FontDefaults>,
    pub(crate) menus: Option<Vec<(String, crate::menu::Menu)>>,
    /// See [`App::spellcheck`].
    pub(crate) spellcheck: rinch_spell::runtime::SpellcheckConfig,
    /// The renderer the app asked for (`RINCH_RENDERER` still overrides it).
    pub(crate) renderer: crate::shell::renderer::Renderer,
    /// See [`App::shortcut_matching`].
    pub(crate) shortcut_matching: crate::menu::ShortcutMatching,
    /// See [`App::chord_yield`].
    pub(crate) chord_yield: crate::menu::ChordYield,
    /// See [`App::prefer_x11`].
    pub(crate) prefer_x11: bool,
    /// See [`App::exit_when`].
    pub(crate) exit_when: crate::ExitWhen,
    /// See [`App::windowless`].
    pub(crate) windowless: bool,
    /// See [`App::title_tooltips`].
    pub(crate) title_tooltips: bool,
    /// See [`App::font_warmup`].
    pub(crate) font_warmup: crate::font::FontWarmup,
    /// See [`App::smooth_wheel_scroll`].
    pub(crate) smooth_wheel_scroll: bool,
    /// See [`App::devtools`].
    pub(crate) devtools: bool,
    /// See [`App::debug_interface`].
    pub(crate) debug_interface: bool,
    /// See [`App::vsync`].
    pub(crate) vsync: Option<bool>,
    /// See [`App::activation_policy`].
    pub(crate) activation_policy: Option<crate::shell::activation::ActivationPolicy>,
    /// See [`App::text_rasterization`].
    pub(crate) text_rasterization: crate::text_rasterization::TextRasterization,
    /// See [`App::gpu_antialiasing`].
    #[cfg(feature = "gpu-renderer")]
    pub(crate) gpu_antialiasing: Option<crate::gpu_antialiasing::GpuAntialiasing>,
    #[cfg(feature = "gpu-renderer")]
    pub(crate) gpu: Option<crate::shell::desktop::GpuInit>,
}

/// Fold the builder's `title` / `size` overrides into the window props, and
/// arm the borderless resize handles.
///
/// Pure, and the only place the precedence rule lives: an explicit
/// [`App::title`] / [`App::size`] beats whatever [`App::window_props`] carried,
/// in either call order. Losing a title to a later `window_props` would be
/// silent — the window would just open named "Rinch Window".
///
/// Desktop-only: Android draws into the Activity's surface, so it has no window
/// props to resolve.
#[cfg(feature = "windowing")]
pub(crate) fn resolve_window_props(
    mut props: WindowProps,
    title: Option<String>,
    size: Option<(u32, u32)>,
) -> WindowProps {
    if let Some(title) = title {
        props.title = title;
    }
    if let Some((width, height)) = size {
        props.width = width;
        props.height = height;
    }
    // Borderless windows have no native resize handles, so arm the custom ones
    // unless the app opted out by setting an inset itself.
    if props.borderless && props.resizable && props.resize_inset.is_none() {
        props.resize_inset = Some(8.0);
    }
    props
}

#[cfg(all(test, feature = "windowing"))]
mod tests;
