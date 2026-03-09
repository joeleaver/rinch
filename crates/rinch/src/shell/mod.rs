//! Shell module - window management and event loop.

#[cfg(feature = "gpu")]
pub(crate) mod compositor;
#[cfg(feature = "gpu")]
pub mod desktop;
pub mod devtools;
pub mod devtools_overlay;
#[cfg(feature = "gpu")]
pub(crate) mod frame_upload;
pub mod html_parser;
pub mod memory_profile;
pub mod rinch_runtime;
#[cfg(feature = "debug")]
pub mod screenshot;
pub mod softbuffer_renderer;
#[cfg(feature = "gpu")]
pub mod transparent_renderer;
pub mod types;
pub mod window;

pub use devtools::{DevToolsPanel, DevToolsState};
pub use devtools_overlay::render_overlay;
#[allow(deprecated)]
pub use rinch_runtime::{run_on_main_thread, run_rinch, run_rinch_with_window_props};
pub use types::{ElementLayout, HoveredElementInfo, RinchEvent};

use rinch_core::dom::{NodeHandle, RenderScope};
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
#[allow(deprecated)]
pub fn run_with_window_props<F>(component: F, props: WindowProps, theme: Option<ThemeProviderProps>)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    run_with_window_props_and_menu(component, props, theme, None);
}

/// Run a rinch application with full window configuration, theme, and optional native menu.
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

                        rinch_core::create_context(rinch_core::MenuBarContext {
                            renderer: items_renderer.clone(),
                            bar_height: 0,
                            layout: rinch_core::MenuBarLayout::InlineTitlebar,
                            items_renderer: Some(items_renderer),
                            overlay_renderer: Some(overlay_renderer),
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
