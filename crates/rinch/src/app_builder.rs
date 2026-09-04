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
//! (`shell::rinch_runtime::run_rinch_with_window_props_and_menu` remains public
//! and is what this dispatches into; it sits below the layer that resolves
//! window props and installs theme CSS, and does neither.)

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::{ThemeProviderProps, WindowProps};

/// A rinch application, configured by builder methods and started by a terminal
/// method ([`run`](App::run) on desktop, `run_android` on Android — the latter
/// exists only when building for Android, so it is not linked here).
///
/// Every configuration method is independent, so any combination is
/// expressible. Nothing happens until a terminal method is called; a terminal
/// method takes over the thread and returns only when the application exits its
/// event loop.
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
#[must_use = "an App does nothing until a terminal method (`run` on desktop, \
              `run_android` on Android) is called"]
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
    /// Desktop only, in every sense that matters: Android has no window menu
    /// bar, and an Android build does not have this method — it is gated on
    /// `desktop`, which cannot currently be enabled for an Android target at
    /// all (`muda` has no Android backend). The `run_android` path therefore
    /// carries a warning for a configured menu that nothing can presently
    /// reach.
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
    /// device via [`gpu_handle`](crate::shell::desktop::gpu_handle).
    ///
    /// Mutually exclusive with [`external_gpu`](App::external_gpu) — the last
    /// one set on *this* builder wins.
    ///
    /// Across builders it does not: the choice is installed into a process-wide
    /// `OnceLock` at [`run`](App::run), and a second install is ignored. Since
    /// `run` returns when the event loop exits, a program that runs a second
    /// `App` in the same process silently keeps the first one's GPU
    /// configuration.
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

    /// Resolve the whole configuration. Pure: no thread is taken over, no
    /// global is written, nothing is registered — so a test can assert that a
    /// builder chain produced what it claims to.
    #[cfg(feature = "desktop")]
    pub(crate) fn into_startup(self) -> Startup<F> {
        Startup {
            props: resolve_window_props(self.props, self.title, self.size),
            theme: self.theme.unwrap_or_default(),
            menus: self.menus,
            #[cfg(feature = "gpu")]
            gpu: self.gpu,
            component: self.component,
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
    /// # Panics
    ///
    /// Startup does not report failure — it aborts. The event loop, the window,
    /// the presentation surface, and (on the `gpu` feature) the adapter,
    /// device and renderer each panic rather than returning an error, on both
    /// the default software backend and the GPU one.
    ///
    /// Two cases are worth separating from that:
    ///
    /// - The event loop reporting an error *while running*, which fires long
    ///   after startup — so a panic out of `run` is not by itself evidence
    ///   that the configuration was wrong.
    /// - With [`external_gpu`](App::external_gpu), the supplied adapter being
    ///   unable to present to rinch's window surface.
    #[cfg(feature = "desktop")]
    pub fn run(self) {
        let Startup {
            component,
            props,
            theme,
            menus,
            #[cfg(feature = "gpu")]
            gpu,
        } = self.into_startup();

        #[cfg(feature = "gpu")]
        if let Some(gpu) = gpu {
            crate::shell::desktop::set_gpu_init(gpu);
        }

        // A no-op when the `theme` feature is off, which is why this is not
        // itself feature-gated.
        crate::setup_theme_css(&theme);

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

/// Everything a terminal method needs, with every builder rule already applied.
///
/// This is the seam that makes the chain testable. [`App::run`] itself takes
/// over the thread and can never be unit-tested, so without a pure step in
/// front of it every configuration method could silently be a no-op — `.title()`,
/// `.size()`, `.theme()` and `.menu()` all could, and a suite that only tested
/// [`resolve_window_props`] in isolation stayed green through all of them.
#[cfg(feature = "desktop")]
pub(crate) struct Startup<F> {
    pub(crate) component: F,
    /// Window props with `title` / `size` folded in and the borderless resize
    /// inset armed.
    pub(crate) props: WindowProps,
    /// The theme to install — the default when none was configured, which is
    /// what makes components visible out of the box.
    pub(crate) theme: ThemeProviderProps,
    pub(crate) menus: Option<Vec<(String, crate::menu::Menu)>>,
    #[cfg(feature = "gpu")]
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

    /// A component that is never called — every test here stops at
    /// [`App::into_startup`], which runs no user code.
    fn component(scope: &mut RenderScope) -> NodeHandle {
        scope.create_element("div")
    }

    /// The GPU choice is one field, so the last of
    /// [`App::gpu_config`] / [`App::external_gpu`] set on a builder is the one
    /// that reaches `run`.
    ///
    /// Only the `gpu_config` half is exercised: `ExternalGpu` needs a real
    /// instance, adapter and device, which a unit test has no way to build.
    /// What this pins is the part that would break silently — moving
    /// `set_gpu_init` out of `run()` and into the builder methods would invert
    /// the semantics to first-one-wins, and nothing else in the suite touches
    /// `gpu` at all.
    #[cfg(feature = "gpu")]
    #[test]
    fn the_last_gpu_choice_set_is_the_one_that_reaches_run() {
        use crate::shell::desktop::{GpuInit, RinchGpuConfig};

        let first = RinchGpuConfig {
            required_features: crate::wgpu::Features::empty(),
            required_limits: crate::wgpu::Limits {
                max_texture_dimension_2d: 4096,
                ..Default::default()
            },
        };
        let second = RinchGpuConfig {
            required_features: crate::wgpu::Features::empty(),
            required_limits: crate::wgpu::Limits {
                max_texture_dimension_2d: 8192,
                ..Default::default()
            },
        };
        assert_ne!(
            first.required_limits.max_texture_dimension_2d,
            second.required_limits.max_texture_dimension_2d,
            "the two configs must be distinguishable or this proves nothing"
        );

        let startup = App::new(component)
            .gpu_config(first)
            .gpu_config(second)
            .into_startup();

        match startup.gpu {
            Some(GpuInit::Config(cfg)) => {
                assert_eq!(cfg.required_limits.max_texture_dimension_2d, 8192)
            }
            _ => panic!("gpu_config must record a Config"),
        }
    }

    /// **The canonical chain.** Every doc example, and almost every migrated
    /// call site, sets `title` AND `size` together — and until this test, no
    /// fixture passed both overrides at once, so a `resolve_window_props` that
    /// handled either alone but dropped both together was invisible.
    #[test]
    fn title_and_size_are_both_applied_when_both_are_set() {
        let startup = App::new(component)
            .title("From title()")
            .size(640, 480)
            .window_props(distinctive_props())
            .into_startup();
        assert_eq!(startup.props.title, "From title()");
        assert_eq!((startup.props.width, startup.props.height), (640, 480));
    }

    /// Order does not matter: `.window_props()` after `.title()`/`.size()` must
    /// not silently swallow them. Replace-semantics here would open the window
    /// named "Rinch Window" with no diagnostic.
    #[test]
    fn window_props_does_not_swallow_an_earlier_title_or_size() {
        let after = App::new(component)
            .title("Mine")
            .size(640, 480)
            .window_props(distinctive_props())
            .into_startup();
        let before = App::new(component)
            .window_props(distinctive_props())
            .title("Mine")
            .size(640, 480)
            .into_startup();
        assert_eq!(after.props.title, "Mine");
        assert_eq!(before.props.title, "Mine");
        assert_eq!(
            (after.props.width, after.props.height),
            (before.props.width, before.props.height)
        );
    }

    /// Each builder method must actually record its argument. Without this,
    /// every one of them could be a no-op body and the suite stayed green.
    #[test]
    fn every_builder_method_records_its_argument() {
        let theme = ThemeProviderProps {
            primary_color: Some("cyan".into()),
            ..Default::default()
        };
        let startup = App::new(component)
            .title("Recorded")
            .size(1280, 720)
            .theme(theme)
            .menu(vec![("File", crate::menu::Menu::new())])
            .into_startup();

        assert_eq!(startup.props.title, "Recorded");
        assert_eq!((startup.props.width, startup.props.height), (1280, 720));
        assert_eq!(startup.theme.primary_color.as_deref(), Some("cyan"));
        let labels: Vec<&str> = startup
            .menus
            .as_ref()
            .expect("menu() was called")
            .iter()
            .map(|(label, _)| label.as_str())
            .collect();
        assert_eq!(labels, vec!["File"]);
    }

    /// With no `.theme()`, the DEFAULT theme is installed rather than nothing —
    /// that is what makes `rinch-components` visible out of the box, and a
    /// regression would leave every component in every app unstyled.
    #[test]
    fn omitting_the_theme_still_installs_the_default() {
        let startup = App::new(component).into_startup();
        assert_eq!(
            startup.theme.primary_color,
            ThemeProviderProps::default().primary_color
        );
        assert!(startup.menus.is_none());
    }

    /// The fields the builder does **not** resolve must survive untouched. This
    /// covers `app_id`, on which the Wayland identity now depends, and
    /// `on_close_requested`, which a minimize-to-tray app cannot work without.
    #[test]
    fn window_props_fields_the_builder_does_not_resolve_are_preserved() {
        let props = WindowProps {
            transparent: true,
            always_on_top: true,
            menu_in_titlebar: true,
            x: Some(17),
            y: Some(23),
            app_id: Some("com.example.notes".into()),
            ..distinctive_props()
        };
        let startup = App::new(component)
            .title("Anything")
            .size(1, 2)
            .window_props(props)
            .into_startup();

        assert!(startup.props.transparent);
        assert!(startup.props.always_on_top);
        assert!(startup.props.menu_in_titlebar);
        assert_eq!((startup.props.x, startup.props.y), (Some(17), Some(23)));
        assert_eq!(startup.props.app_id.as_deref(), Some("com.example.notes"));
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
