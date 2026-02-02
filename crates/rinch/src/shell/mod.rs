//! Shell module - window management and event loop.

pub mod devtools;
pub mod devtools_overlay;
pub mod html_parser;
pub mod memory_profile;
pub mod transparent_renderer;
pub mod types;
pub mod rinch_runtime;
#[cfg(feature = "debug")]
pub mod screenshot;

pub use devtools::{DevToolsPanel, DevToolsState};
pub use devtools_overlay::render_overlay;
pub use types::{ElementLayout, HoveredElementInfo, RinchEvent};
pub use rinch_runtime::{run_rinch, run_rinch_with_window_props};

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
///     let count = use_signal(|| 0);
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
pub fn run<F>(title: &str, width: u32, height: u32, component: F)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    rinch_runtime::run_rinch(title, width, height, component);
}

/// Run a rinch application with theme configuration.
///
/// This sets up theme CSS variables before running the application, making them
/// available throughout the component tree.
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

/// Run a rinch application with full window configuration and theme.
pub fn run_with_window_props<F>(
    component: F,
    props: WindowProps,
    theme: Option<ThemeProviderProps>,
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    if let Some(theme) = theme {
        crate::setup_theme_css(&theme);
    }
    rinch_runtime::run_rinch_with_window_props(component, props);
}
