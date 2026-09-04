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
//! implementation: every desktop startup path runs the same code below.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::{ThemeProviderProps, WindowProps};

/// A rinch application, configured by builder methods and started by a terminal
/// method ([`run`](App::run) on desktop, [`run_android`](App::run_android) on
/// Android).
///
/// Every configuration method is independent, so any combination is
/// expressible. Nothing happens until a terminal method is called; the terminal
/// methods take over the thread and do not return.
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
pub struct App<F> {
    pub(crate) component: F,
    /// The window configuration. [`title`](App::title) and [`size`](App::size)
    /// are held separately and applied over this at startup, so setting them
    /// never silently loses to a later [`window_props`](App::window_props).
    pub(crate) props: WindowProps,
    pub(crate) title: Option<String>,
    pub(crate) size: Option<(u32, u32)>,
    pub(crate) theme: Option<ThemeProviderProps>,
    #[cfg(feature = "desktop")]
    pub(crate) menus: Option<Vec<(String, crate::menu::Menu)>>,
    #[cfg(feature = "gpu")]
    pub(crate) gpu: Option<crate::shell::desktop::GpuInit>,
}

impl<F> App<F>
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    /// Start configuring an application around its root component.
    pub fn new(component: F) -> Self {
        Self {
            component,
            props: WindowProps::default(),
            title: None,
            size: None,
            theme: None,
            #[cfg(feature = "desktop")]
            menus: None,
            #[cfg(feature = "gpu")]
            gpu: None,
        }
    }

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

    /// Set the full window configuration.
    ///
    /// [`title`](App::title) and [`size`](App::size) are applied *over* this,
    /// so `.title("X").window_props(p)` still gets the title `"X"` rather than
    /// `p`'s. Every other field comes from `props`.
    pub fn window_props(mut self, props: WindowProps) -> Self {
        self.props = props;
        self
    }

    /// Add a native menu bar. Each `(label, Menu)` pair becomes a top-level
    /// submenu.
    ///
    /// Ignored (with a warning) by [`run_android`](App::run_android) — Android
    /// has no window menu bar.
    #[cfg(feature = "desktop")]
    pub fn menu(mut self, menus: Vec<(&str, crate::menu::Menu)>) -> Self {
        self.menus = Some(
            menus
                .into_iter()
                .map(|(label, menu)| (label.to_string(), menu))
                .collect(),
        );
        self
    }

    /// Raise the compositor GPU device's features and limits.
    ///
    /// rinch still owns the instance, picks a surface-compatible adapter and
    /// creates the device, so window presentation is always correct. Use this
    /// when an embedding renderer needs a higher-capability device so it can
    /// build its pipelines and textures on *rinch's* device and hand back a
    /// `TextureView` for zero-copy present. After startup, obtain the shared
    /// device via [`gpu_handle`](crate::gpu_handle).
    ///
    /// Mutually exclusive with [`external_gpu`](App::external_gpu) — the last
    /// one set wins.
    #[cfg(feature = "gpu")]
    pub fn gpu_config(mut self, gpu: crate::shell::desktop::RinchGpuConfig) -> Self {
        self.gpu = Some(crate::shell::desktop::GpuInit::Config(gpu));
        self
    }

    /// Run on an embedder-provided GPU device.
    ///
    /// The embedder creates the whole GPU stack with its own `DeviceDescriptor`
    /// and hands it over; rinch creates only the window surface, validates that
    /// the adapter can present to it, and composites directly onto the provided
    /// device — no `request_device`, no CPU readback.
    ///
    /// Prefer [`gpu_config`](App::gpu_config) when you only need to raise
    /// features and limits: letting rinch own device creation guarantees
    /// surface compatibility.
    ///
    /// # Panics
    ///
    /// [`run`](App::run) panics if the supplied adapter cannot present to
    /// rinch's window surface.
    #[cfg(feature = "gpu")]
    pub fn external_gpu(mut self, gpu: crate::shell::desktop::ExternalGpu) -> Self {
        self.gpu = Some(crate::shell::desktop::GpuInit::External(gpu));
        self
    }

    /// Start the application on the desktop. Takes over the thread and does not
    /// return.
    ///
    /// # Panics
    ///
    /// Panics if the event loop, window, or (with an external GPU device) the
    /// surface cannot be created.
    #[cfg(feature = "desktop")]
    pub fn run(self) {
        #[cfg(feature = "gpu")]
        if let Some(gpu) = self.gpu {
            crate::shell::desktop::set_gpu_init(gpu);
        }

        let props = resolve_window_props(self.props, self.title, self.size);

        // A no-op when the `theme` feature is off, which is why this is not
        // itself feature-gated.
        crate::setup_theme_css(&self.theme.unwrap_or_default());

        let component = self.component;
        let menus = self.menus;

        #[cfg(target_os = "linux")]
        {
            run_desktop_linux(component, props, menus);
        }

        #[cfg(not(target_os = "linux"))]
        {
            let native_menu = menus.map(|menus| {
                let refs: Vec<(&str, crate::menu::Menu)> = menus
                    .iter()
                    .map(|(label, menu)| (label.as_str(), menu.clone()))
                    .collect();
                crate::menu::build_native_menu_bar(refs)
            });
            crate::shell::rinch_runtime::run_rinch_with_window_props_and_menu(
                component,
                props,
                native_menu,
            );
        }
    }

    /// Start the application on Android. Takes over the thread and does not
    /// return.
    ///
    /// The same configuration methods apply everywhere; only the terminal
    /// differs, so an app that targets both platforms writes the chain once.
    /// Android draws into the Activity's own surface, so
    /// [`size`](App::size) and the window fields of
    /// [`window_props`](App::window_props) do not apply, and
    /// [`menu`](App::menu) warns and is ignored.
    ///
    /// Call it from `android_main`:
    ///
    /// ```ignore
    /// #[unsafe(no_mangle)]
    /// fn android_main(android_app: AndroidApp) {
    ///     App::new(app).title("My App").run_android(android_app);
    /// }
    /// ```
    #[cfg(all(feature = "android", target_os = "android"))]
    pub fn run_android(self, android_app: android_activity::AndroidApp) {
        #[cfg(feature = "desktop")]
        if self.menus.is_some() {
            tracing::warn!(
                "App::menu() has no effect on Android — there is no window menu bar; \
                 the menu was ignored"
            );
        }

        crate::setup_theme_css(&self.theme.unwrap_or_default());
        crate::shell::android_runtime::run_component(android_app, self.component);
    }
}

/// The Linux desktop startup path, which has to wrap the component so the
/// in-app menu bar can render inside the document.
///
/// Split out of [`App::run`] only because the `#[cfg(target_os = "linux")]`
/// body is long; it is not a second entry point.
#[cfg(all(feature = "desktop", target_os = "linux"))]
fn run_desktop_linux<F>(
    component: F,
    props: WindowProps,
    menus: Option<Vec<(String, crate::menu::Menu)>>,
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    let Some(menu_data) = menus else {
        crate::shell::rinch_runtime::run_rinch_with_window_props_and_menu(component, props, None);
        return;
    };

    // Build the native menu even on Linux: that is what registers the
    // shortcuts and callbacks in the thread-local registry. The in-app bar
    // below renders from `menu_data`, which is why the two are separate.
    let native_menu = {
        let refs: Vec<(&str, crate::menu::Menu)> = menu_data
            .iter()
            .map(|(label, menu)| (label.as_str(), menu.clone()))
            .collect();
        crate::menu::build_native_menu_bar(refs)
    };

    if !props.borderless {
        let wrapped = move |scope: &mut RenderScope| {
            let content = component(scope);
            let menu_refs: Vec<(&str, &crate::menu::Menu)> = menu_data
                .iter()
                .map(|(label, menu)| (label.as_str(), menu))
                .collect();
            crate::menu::app_menu_bar::render_with_menu_bar(scope, &menu_refs, content, 0)
        };
        crate::shell::rinch_runtime::run_rinch_with_window_props_and_menu(
            wrapped,
            props,
            Some(native_menu),
        );
        return;
    }

    // Borderless: set MenuBarContext — BorderlessWindow renders the bar
    // internally. The bar uses absolute positioning and must be the LAST child
    // for correct hit testing. BorderlessWindow handles this ordering.
    const TITLEBAR_HEIGHT: u32 = 36;
    let menu_in_titlebar = props.menu_in_titlebar;
    let wrapped = move |scope: &mut RenderScope| {
        let menu_data_rc = std::rc::Rc::new(menu_data);

        if menu_in_titlebar {
            // Inline titlebar layout: split renderers share a Signal
            use rinch_core::reactive::Signal;
            let active_menu: Signal<i32> = Signal::new(-1);

            let items_renderer: rinch_core::MenuBarRenderer = {
                let md = menu_data_rc.clone();
                std::rc::Rc::new(move |scope| {
                    let refs: Vec<(&str, &crate::menu::Menu)> =
                        md.iter().map(|(l, m)| (l.as_str(), m)).collect();
                    crate::menu::app_menu_bar::render_menu_items_inline(scope, &refs, active_menu)
                })
            };

            let overlay_renderer: rinch_core::MenuBarRenderer = {
                std::rc::Rc::new(move |scope| {
                    crate::menu::app_menu_bar::render_inline_overlay(scope, active_menu)
                })
            };

            // Estimate inline row width for titlebar spacer:
            // 10px padding-left + hamburger(~36px) + 2px gap per item
            // + each label (~8px/char + 16px padding) + 10px padding-right
            let labels_width: u32 = menu_data_rc
                .iter()
                .map(|(l, _)| (l.len() as u32) * 8 + 16 + 2)
                .sum();
            let spacer_w = 10 + 36 + labels_width + 10;

            rinch_core::create_context(rinch_core::MenuBarContext {
                renderer: items_renderer.clone(),
                bar_height: 0,
                layout: rinch_core::MenuBarLayout::InlineTitlebar,
                items_renderer: Some(items_renderer),
                overlay_renderer: Some(overlay_renderer),
                spacer_width: spacer_w,
            });
        } else {
            // Below-titlebar layout: single standalone renderer
            let renderer: rinch_core::MenuBarRenderer = {
                let md = menu_data_rc.clone();
                std::rc::Rc::new(move |scope| {
                    let refs: Vec<(&str, &crate::menu::Menu)> =
                        md.iter().map(|(l, m)| (l.as_str(), m)).collect();
                    crate::menu::app_menu_bar::render_menu_bar_standalone(
                        scope,
                        &refs,
                        TITLEBAR_HEIGHT,
                    )
                })
            };
            rinch_core::create_context(rinch_core::MenuBarContext {
                renderer,
                bar_height: crate::menu::app_menu_bar::MENU_BAR_HEIGHT,
                layout: rinch_core::MenuBarLayout::BelowTitlebar,
                items_renderer: None,
                overlay_renderer: None,
                spacer_width: 0,
            });
        }

        component(scope)
    };
    crate::shell::rinch_runtime::run_rinch_with_window_props_and_menu(
        wrapped,
        props,
        Some(native_menu),
    );
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
#[cfg(feature = "desktop")]
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

#[cfg(all(test, feature = "desktop"))]
mod tests {
    use super::*;

    /// Window props that share **no** field value with `WindowProps::default()`
    /// among the fields these tests read, so a mutant that ignores the argument
    /// and returns the default cannot pass by coincidence.
    fn distinctive_props() -> WindowProps {
        WindowProps {
            title: "From props".into(),
            width: 1024,
            height: 768,
            ..Default::default()
        }
    }

    #[test]
    fn window_props_survive_when_no_override_is_set() {
        let resolved = resolve_window_props(distinctive_props(), None, None);
        assert_eq!(resolved.title, "From props");
        assert_eq!((resolved.width, resolved.height), (1024, 768));
    }

    #[test]
    fn title_override_beats_the_window_props_title() {
        // Both titles are non-empty and different, so the assertion
        // distinguishes "override applied" from "override dropped" — a fixture
        // where they matched could not.
        let resolved = resolve_window_props(distinctive_props(), Some("From title()".into()), None);
        assert_eq!(resolved.title, "From title()");
        // The override touches nothing else.
        assert_eq!((resolved.width, resolved.height), (1024, 768));
    }

    #[test]
    fn size_override_beats_the_window_props_size() {
        let resolved = resolve_window_props(distinctive_props(), None, Some((640, 480)));
        assert_eq!((resolved.width, resolved.height), (640, 480));
        assert_eq!(resolved.title, "From props");
    }

    #[test]
    fn borderless_resizable_arms_the_default_resize_inset() {
        let resolved = resolve_window_props(
            WindowProps {
                borderless: true,
                resizable: true,
                resize_inset: None,
                ..Default::default()
            },
            None,
            None,
        );
        assert_eq!(resolved.resize_inset, Some(8.0));
    }

    #[test]
    fn an_explicit_resize_inset_is_never_overwritten() {
        // 12.0, not 8.0: a mutant that assigns unconditionally would be
        // invisible against a fixture that already held the default.
        let resolved = resolve_window_props(
            WindowProps {
                borderless: true,
                resizable: true,
                resize_inset: Some(12.0),
                ..Default::default()
            },
            None,
            None,
        );
        assert_eq!(resolved.resize_inset, Some(12.0));
    }

    #[test]
    fn a_decorated_window_gets_no_resize_inset() {
        let resolved = resolve_window_props(
            WindowProps {
                borderless: false,
                resizable: true,
                resize_inset: None,
                ..Default::default()
            },
            None,
            None,
        );
        assert_eq!(resolved.resize_inset, None);
    }

    #[test]
    fn a_fixed_size_borderless_window_gets_no_resize_inset() {
        let resolved = resolve_window_props(
            WindowProps {
                borderless: true,
                resizable: false,
                resize_inset: None,
                ..Default::default()
            },
            None,
            None,
        );
        assert_eq!(resolved.resize_inset, None);
    }
}
