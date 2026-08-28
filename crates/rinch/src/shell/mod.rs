//! Shell module - window management and event loop.

#[cfg(feature = "gpu")]
pub(crate) mod compositor;
#[cfg(feature = "gpu")]
pub mod desktop;
pub mod devtools_css;
pub mod devtools_panel;
pub mod devtools_store;
#[cfg(feature = "gpu")]
pub(crate) mod frame_upload;
pub mod html_parser;
pub mod memory_profile;
#[cfg(feature = "desktop")]
pub mod rinch_runtime;
pub mod screenshot;
#[cfg(feature = "desktop")]
pub mod softbuffer_renderer;
pub mod types;
#[cfg(feature = "desktop")]
pub mod window;

#[cfg(all(feature = "android", target_os = "android"))]
pub mod android_runtime;

// One Android frame's app-level work, host-compiled for the same reason as the
// IME translator below: what a frame *is* — turn the clock, then decide what
// still has to happen before the surface is presented — is where the loop's
// ordering is decided, and it is testable without a phone.
#[cfg(any(all(feature = "android", target_os = "android"), test))]
pub(crate) mod android_frame;

// The Android IME composition translator. `android_runtime` compiles only for
// `target_os = "android"`, but the composing region is a state machine over
// `InputConnection` calls — pure, and the part that has to be pinned down by
// tests — so it is compiled for the host test build too.
#[cfg(any(all(feature = "android", target_os = "android"), test))]
pub(crate) mod android_ime;

// The Android touch recogniser. `android_runtime` compiles only for
// `target_os = "android"`, but the translation from finger to pointer events is
// pure and is the part that has to be pinned down by tests, so it is compiled
// for the host test build too.
#[cfg(any(all(feature = "android", target_os = "android"), test))]
pub(crate) mod touch_gesture;

pub use devtools_store::DevToolsStore;
#[cfg(feature = "desktop")]
pub use rinch_runtime::inject_platform_event;
#[cfg(feature = "desktop")]
#[allow(deprecated)]
pub use rinch_runtime::{run_on_main_thread, run_rinch, run_rinch_with_window_props};
#[cfg(feature = "desktop")]
pub use types::RinchEvent;
pub use types::{ElementLayout, HoveredElementInfo};

#[cfg(feature = "desktop")]
use rinch_core::dom::{NodeHandle, RenderScope};
#[cfg(feature = "desktop")]
use rinch_core::element::ThemeProviderProps;

/// Run a rinch application with fine-grained reactive rendering.
///
/// This is the primary entry point for rinch applications. The component function
/// receives a RenderScope and builds the DOM tree directly.
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
///
/// fn app(__scope: &mut RenderScope) -> NodeHandle {
///     let count = Signal::new(0);
///     rsx! {
///         div {
///             p { "Count: " {|| count.get().to_string()} }
///             button { onclick: move || count.update(|n| *n += 1), "+" }
///         }
///     }
/// }
///
/// fn main() {
///     run("My App", 800, 600, app);
/// }
/// ```
#[cfg(feature = "desktop")]
#[allow(deprecated)]
pub fn run<F>(title: &str, width: u32, height: u32, component: F)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    // Auto-load default theme CSS when theme feature is enabled
    // This ensures components are visible even without run_with_theme()
    #[cfg(feature = "theme")]
    {
        crate::setup_theme_css(&rinch_core::element::ThemeProviderProps::default());
    }

    rinch_runtime::run_rinch(title, width, height, component);
}

/// Run a rinch application with theme configuration.
///
/// This sets up theme CSS variables before running the application, making them
/// available throughout the component tree.
#[cfg(feature = "desktop")]
#[allow(deprecated)]
pub fn run_with_theme<F>(
    title: &str,
    width: u32,
    height: u32,
    component: F,
    theme: ThemeProviderProps,
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    crate::setup_theme_css(&theme);
    rinch_runtime::run_rinch(title, width, height, component);
}

#[cfg(feature = "desktop")]
use rinch_core::element::WindowProps;

/// Run a rinch application with a native menu bar.
///
/// Each `(label, Menu)` pair becomes a top-level submenu in the window menu bar.
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
/// use rinch::menu::{Menu, MenuItem};
///
/// let file_menu = Menu::new()
///     .item(MenuItem::new("New").shortcut("Ctrl+N").on_click(|| println!("New!")))
///     .separator()
///     .item(MenuItem::new("Quit").on_click(|| std::process::exit(0)));
///
/// run_with_menu("My App", 800, 600, app, vec![("File", file_menu)]);
/// ```
#[cfg(feature = "desktop")]
pub fn run_with_menu<F>(
    title: &str,
    width: u32,
    height: u32,
    component: F,
    menus: Vec<(&str, crate::menu::Menu)>,
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    #[cfg(feature = "theme")]
    {
        crate::setup_theme_css(&rinch_core::element::ThemeProviderProps::default());
    }

    let props = WindowProps {
        title: title.into(),
        width,
        height,
        ..Default::default()
    };

    #[cfg(target_os = "linux")]
    {
        // Clone menu data before build_native_menu_bar consumes it
        let menu_data: Vec<(String, crate::menu::Menu)> = menus
            .iter()
            .map(|(l, m)| (l.to_string(), m.clone()))
            .collect();
        // Still build native menu to register shortcuts + callbacks in thread-local
        let native_menu = crate::menu::build_native_menu_bar(menus);
        // run_with_menu uses default WindowProps (not borderless), so no title bar offset
        let wrapped = move |scope: &mut RenderScope| {
            let content = component(scope);
            let menu_refs: Vec<(&str, &crate::menu::Menu)> =
                menu_data.iter().map(|(l, m)| (l.as_str(), m)).collect();
            crate::menu::app_menu_bar::render_with_menu_bar(scope, &menu_refs, content, 0)
        };
        rinch_runtime::run_rinch_with_window_props_and_menu(wrapped, props, Some(native_menu));
    }

    #[cfg(not(target_os = "linux"))]
    {
        let native_menu = crate::menu::build_native_menu_bar(menus);
        rinch_runtime::run_rinch_with_window_props_and_menu(component, props, Some(native_menu));
    }
}

/// Run a rinch application with full window configuration and theme.
#[cfg(feature = "desktop")]
#[allow(deprecated)]
pub fn run_with_window_props<F>(component: F, props: WindowProps, theme: Option<ThemeProviderProps>)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    run_with_window_props_and_menu(component, props, theme, None);
}

/// Run a rinch application with full window configuration, theme, and optional native menu.
#[cfg(feature = "desktop")]
pub fn run_with_window_props_and_menu<F>(
    component: F,
    mut props: WindowProps,
    theme: Option<ThemeProviderProps>,
    menus: Option<Vec<(&str, crate::menu::Menu)>>,
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    // Auto-enable resize handles for borderless windows
    if props.borderless && props.resizable && props.resize_inset.is_none() {
        props.resize_inset = Some(8.0);
    }
    #[cfg(feature = "theme")]
    {
        if let Some(theme) = theme {
            crate::setup_theme_css(&theme);
        } else {
            crate::setup_theme_css(&rinch_core::element::ThemeProviderProps::default());
        }
    }
    #[cfg(not(feature = "theme"))]
    {
        let _ = theme;
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(menus) = menus {
            // Clone menu data before build_native_menu_bar consumes it
            let menu_data: Vec<(String, crate::menu::Menu)> = menus
                .iter()
                .map(|(l, m)| (l.to_string(), m.clone()))
                .collect();
            // Still build native menu to register shortcuts + callbacks in thread-local
            let native_menu = crate::menu::build_native_menu_bar(menus);

            if props.borderless {
                // Borderless: set MenuBarContext — BorderlessWindow renders the bar internally.
                // The bar uses absolute positioning and must be the LAST child for correct
                // hit testing. BorderlessWindow handles this ordering.
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
                                crate::menu::app_menu_bar::render_menu_items_inline(
                                    scope,
                                    &refs,
                                    active_menu,
                                )
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
                rinch_runtime::run_rinch_with_window_props_and_menu(
                    wrapped,
                    props,
                    Some(native_menu),
                );
            } else {
                // Non-borderless: existing wrapper approach
                let wrapped = move |scope: &mut RenderScope| {
                    let content = component(scope);
                    let menu_refs: Vec<(&str, &crate::menu::Menu)> =
                        menu_data.iter().map(|(l, m)| (l.as_str(), m)).collect();
                    crate::menu::app_menu_bar::render_with_menu_bar(scope, &menu_refs, content, 0)
                };
                rinch_runtime::run_rinch_with_window_props_and_menu(
                    wrapped,
                    props,
                    Some(native_menu),
                );
            }
        } else {
            rinch_runtime::run_rinch_with_window_props_and_menu(component, props, None);
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let native_menu = menus.map(crate::menu::build_native_menu_bar);
        rinch_runtime::run_rinch_with_window_props_and_menu(component, props, native_menu);
    }
}

/// Run a rinch application, raising the compositor GPU device's features/limits.
///
/// By default rinch requests its wgpu device with `Features::default()` /
/// `Limits::default()`. When an embedding application's own renderer needs a
/// higher-capability device — so it can create its pipelines and textures on
/// **rinch's** device and hand back a `TextureView` for zero-copy present (see
/// [`create_render_surface`](crate::render_surface::create_render_surface) and
/// [`GpuTextureRegistrar`](crate::render_surface::GpuTextureRegistrar)) — pass a
/// [`RinchGpuConfig`](crate::shell::desktop::RinchGpuConfig) here.
///
/// rinch still owns the instance, picks a surface-compatible adapter, and
/// creates the device, so window presentation is always correct. After startup,
/// obtain the shared device via [`gpu_handle`](crate::gpu_handle).
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
/// use rinch::shell::desktop::RinchGpuConfig;
///
/// let mut limits = wgpu::Limits::default();
/// limits.max_storage_buffers_per_shader_stage = 32;
/// let gpu = RinchGpuConfig {
///     required_features: wgpu::Features::FLOAT32_FILTERABLE,
///     required_limits: limits,
/// };
/// run_with_gpu_config(app, WindowProps::default(), None, gpu);
/// ```
#[cfg(feature = "gpu")]
pub fn run_with_gpu_config<F>(
    component: F,
    props: WindowProps,
    theme: Option<ThemeProviderProps>,
    gpu: crate::shell::desktop::RinchGpuConfig,
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    crate::shell::desktop::set_gpu_init(crate::shell::desktop::GpuInit::Config(gpu));
    run_with_window_props(component, props, theme);
}

/// Run a rinch application on an embedder-provided GPU device.
///
/// The embedder creates the whole GPU stack with its own `DeviceDescriptor` and
/// hands it to rinch via [`ExternalGpu`](crate::shell::desktop::ExternalGpu).
/// rinch creates only the window surface (from the provided `instance`),
/// validates that the adapter can present to it, and composites directly onto
/// the provided device — no `request_device`, no CPU readback. The provided
/// device is published through [`gpu_handle`](crate::gpu_handle).
///
/// Prefer this when the embedder must keep its exact device descriptor; prefer
/// [`run_with_gpu_config`] when it only needs to raise features/limits and would
/// rather let rinch own device creation (which guarantees surface
/// compatibility).
///
/// # Panics
///
/// Panics if the supplied adapter cannot present to rinch's window surface. The
/// adapter/device must be created from an adapter that supports the target
/// window (on multi-GPU systems, create the adapter with a compatible surface).
#[cfg(feature = "gpu")]
pub fn run_with_external_device<F>(
    component: F,
    props: WindowProps,
    theme: Option<ThemeProviderProps>,
    gpu: crate::shell::desktop::ExternalGpu,
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    crate::shell::desktop::set_gpu_init(crate::shell::desktop::GpuInit::External(gpu));
    run_with_window_props(component, props, theme);
}
