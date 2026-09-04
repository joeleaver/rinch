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

// ── Entry points ─────────────────────────────────────────────────────────────
//
// Every function below is a thin shim over [`crate::App`] (issue #493), which
// holds the actual startup logic. They are deprecated because they cannot
// express their own combinations — `run_with_menu` takes no window props,
// `run_with_window_props` takes no menu without a second function, and neither
// can take whatever feature lands next. `App` composes all of it.

/// Run a rinch application with fine-grained reactive rendering.
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
///     App::new(app).title("My App").size(800, 600).run();
/// }
/// ```
#[cfg(feature = "desktop")]
#[deprecated(
    since = "0.3.0",
    note = "use `App::new(component).title(title).size(width, height).run()`"
)]
pub fn run<F>(title: &str, width: u32, height: u32, component: F)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    crate::App::new(component)
        .title(title)
        .size(width, height)
        .run();
}

/// Run a rinch application with theme configuration.
///
/// This sets up theme CSS variables before running the application, making them
/// available throughout the component tree.
#[cfg(feature = "desktop")]
#[deprecated(
    since = "0.3.0",
    note = "use `App::new(component).title(title).size(width, height).theme(theme).run()`"
)]
pub fn run_with_theme<F>(
    title: &str,
    width: u32,
    height: u32,
    component: F,
    theme: ThemeProviderProps,
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    crate::App::new(component)
        .title(title)
        .size(width, height)
        .theme(theme)
        .run();
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
/// App::new(app)
///     .title("My App")
///     .size(800, 600)
///     .menu(vec![("File", file_menu)])
///     .run();
/// ```
#[cfg(feature = "desktop")]
#[deprecated(
    since = "0.3.0",
    note = "use `App::new(component).title(title).size(width, height).menu(menus).run()`"
)]
pub fn run_with_menu<F>(
    title: &str,
    width: u32,
    height: u32,
    component: F,
    menus: Vec<(&str, crate::menu::Menu)>,
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    crate::App::new(component)
        .title(title)
        .size(width, height)
        .menu(menus)
        .run();
}

/// Run a rinch application with full window configuration and theme.
#[cfg(feature = "desktop")]
#[deprecated(
    since = "0.3.0",
    note = "use `App::new(component).window_props(props).run()`, adding `.theme(theme)` if there is one"
)]
pub fn run_with_window_props<F>(component: F, props: WindowProps, theme: Option<ThemeProviderProps>)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    let mut app = crate::App::new(component).window_props(props);
    app.theme = theme;
    app.run();
}

/// Run a rinch application with full window configuration, theme, and optional native menu.
#[cfg(feature = "desktop")]
#[deprecated(
    since = "0.3.0",
    note = "use `App::new(component).window_props(props).menu(menus).run()`, adding `.theme(theme)` if there is one"
)]
pub fn run_with_window_props_and_menu<F>(
    component: F,
    props: WindowProps,
    theme: Option<ThemeProviderProps>,
    menus: Option<Vec<(&str, crate::menu::Menu)>>,
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    let mut app = crate::App::new(component).window_props(props);
    app.theme = theme;
    if let Some(menus) = menus {
        app = app.menu(menus);
    }
    app.run();
}

/// Run a rinch application, raising the compositor GPU device's features/limits.
///
/// By default rinch requests its wgpu device with `Features::default()` /
/// `Limits::default()`. When an embedding application's own renderer needs a
/// higher-capability device — so it can create its pipelines and textures on
/// **rinch's** device and hand back a `TextureView` for zero-copy present (see
/// [`create_render_surface`](crate::render_surface::create_render_surface) and
/// [`GpuTextureRegistrar`](crate::render_surface::GpuTextureRegistrar)) — pass a
/// [`RinchGpuConfig`](crate::shell::desktop::RinchGpuConfig).
///
/// rinch still owns the instance, picks a surface-compatible adapter, and
/// creates the device, so window presentation is always correct. After startup,
/// obtain the shared device via [`gpu_handle`](crate::gpu_handle).
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
///
/// let mut limits = wgpu::Limits::default();
/// limits.max_storage_buffers_per_shader_stage = 32;
/// App::new(app)
///     .window_props(WindowProps::default())
///     .gpu_config(RinchGpuConfig {
///         required_features: wgpu::Features::FLOAT32_FILTERABLE,
///         required_limits: limits,
///     })
///     .run();
/// ```
#[cfg(feature = "gpu")]
#[deprecated(
    since = "0.3.0",
    note = "use `App::new(component).window_props(props).gpu_config(gpu).run()`"
)]
pub fn run_with_gpu_config<F>(
    component: F,
    props: WindowProps,
    theme: Option<ThemeProviderProps>,
    gpu: crate::shell::desktop::RinchGpuConfig,
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    let mut app = crate::App::new(component)
        .window_props(props)
        .gpu_config(gpu);
    app.theme = theme;
    app.run();
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
/// [`App::gpu_config`](crate::App::gpu_config) when it only needs to raise
/// features/limits and would rather let rinch own device creation (which
/// guarantees surface compatibility).
///
/// # Panics
///
/// Panics if the supplied adapter cannot present to rinch's window surface. The
/// adapter/device must be created from an adapter that supports the target
/// window (on multi-GPU systems, create the adapter with a compatible surface).
#[cfg(feature = "gpu")]
#[deprecated(
    since = "0.3.0",
    note = "use `App::new(component).window_props(props).external_gpu(gpu).run()`"
)]
pub fn run_with_external_device<F>(
    component: F,
    props: WindowProps,
    theme: Option<ThemeProviderProps>,
    gpu: crate::shell::desktop::ExternalGpu,
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    let mut app = crate::App::new(component)
        .window_props(props)
        .external_gpu(gpu);
    app.theme = theme;
    app.run();
}
