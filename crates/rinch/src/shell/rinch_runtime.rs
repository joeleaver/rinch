//! Rinch-native runtime using rinch-dom for rendering.
//!
//! This module wires together the platform-agnostic [`RinchApp`] with the
//! desktop backend ([`WinitWindow`] + [`WgpuRenderer`]) and the winit event
//! loop. All application logic lives in [`crate::app::RinchApp`]; this file
//! is purely the desktop glue layer.
//!
//! # Usage
//!
//! ```ignore
//! use rinch::prelude::*;
//!
//! fn app(__scope: &mut RenderScope) -> NodeHandle {
//!     rsx! { div { "Hello from rinch-dom!" } }
//! }
//!
//! fn main() {
//!     rinch::run_rinch("My App", 800, 600, app);
//! }
//! ```

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{WindowAttributes, WindowId};

use rinch_core::clear_context;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::events;
#[cfg(feature = "gpu")]
use rinch_platform::PlatformRenderer;
use rinch_platform::{
    AppAction, KeyCode, Modifiers, MouseButton as PlatformMouseButton, PlatformEvent,
    PlatformWindow, UserEvent,
};

use crate::app::RinchApp;

#[cfg(feature = "gpu")]
use super::desktop::WgpuRenderer;
use super::window::WinitWindow;

#[cfg(feature = "debug")]
use {super::screenshot, base64::Engine, rinch_debug::DebugResult};

// ── Thread-local proxy ───────────────────────────────────────────────────────

// Thread-local proxy for the native event loop, used by window control functions.
thread_local! {
    pub(crate) static NATIVE_PROXY: RefCell<Option<EventLoopProxy>> = const { RefCell::new(None) };
}

// ── Global proxy + native event queue ────────────────────────────────────────

// Global (Send+Sync) proxy for waking the event loop from any thread.
pub(crate) static GLOBAL_PROXY: OnceLock<EventLoopProxy> = OnceLock::new();

// Queue of native events to process on the next proxy_wake_up.
// winit 0.31 removes send_event(T); we queue events and call proxy.wake_up().
static NATIVE_EVENT_QUEUE: Mutex<VecDeque<RinchNativeEvent>> = Mutex::new(VecDeque::new());

/// Queue a native event and wake the event loop.
pub(crate) fn send_native_event(event: RinchNativeEvent) {
    NATIVE_EVENT_QUEUE.lock().unwrap().push_back(event);
    if let Some(proxy) = GLOBAL_PROXY.get() {
        proxy.wake_up();
    }
}

// Queue of closures to execute on the main thread during the next ReRender.
static MAIN_QUEUE: Mutex<Vec<Box<dyn FnOnce() + Send>>> = Mutex::new(Vec::new());

/// Queue a closure to run on the main (UI) thread.
///
/// The closure will execute during the next event-loop wake, before the
/// re-render pass. This is the safe way to update [`Signal`]s from a
/// background thread (e.g. after an HTTP request completes on tokio).
///
/// # Example
///
/// ```ignore
/// let loading = Signal::new(false);
/// std::thread::spawn(move || {
///     let result = do_work();
///     rinch::run_on_main_thread(move || {
///         loading.set(false);
///     });
/// });
/// ```
pub fn run_on_main_thread(f: impl FnOnce() + Send + 'static) {
    MAIN_QUEUE.lock().unwrap().push(Box::new(f));
    send_native_event(RinchNativeEvent::ReRender);
}

/// Dispatcher function for cross-thread signal updates.
///
/// Registered with `rinch_core::set_cross_thread_dispatcher()` so that
/// `Signal::send()` can automatically route updates to the main thread.
fn dispatch_to_main_thread(f: Box<dyn FnOnce() + Send>) {
    MAIN_QUEUE.lock().unwrap().push(f);
    send_native_event(RinchNativeEvent::ReRender);
}

/// Drain and execute all pending main-thread callbacks.
fn drain_main_queue() {
    let callbacks: Vec<Box<dyn FnOnce() + Send>> = MAIN_QUEUE.lock().unwrap().drain(..).collect();
    for cb in callbacks {
        cb();
    }
}

/// Events sent to the event loop.
#[derive(Debug, Clone)]
pub enum RinchNativeEvent {
    /// A signal changed -- re-resolve layout and repaint.
    ReRender,
    /// A debug command is waiting on the channel (debug feature).
    #[cfg(feature = "debug")]
    DebugCommand,
    /// Minimize the window.
    MinimizeWindow,
    /// Toggle maximize state.
    ToggleMaximizeWindow,
    /// Close the window (from window controls).
    CloseWindowControl,
    /// Show the window.
    ShowWindow,
    /// Hide the window.
    HideWindow,
}

// ── RinchRuntime ─────────────────────────────────────────────────────────────

/// The desktop runtime: thin winit `ApplicationHandler` that delegates to
/// [`RinchApp`] for all platform-agnostic logic and uses [`WgpuRenderer`]
/// + [`WinitWindow`] for the platform-specific parts.
pub struct RinchRuntime {
    /// Platform-agnostic application logic.
    app: RinchApp,
    /// Desktop window wrapper.
    window: Option<WinitWindow>,
    /// GPU renderer (only available with `gpu` feature).
    #[cfg(feature = "gpu")]
    renderer: Option<WgpuRenderer>,
    /// Software renderer (CPU pixel presentation via softbuffer).
    #[cfg(not(feature = "gpu"))]
    soft_renderer: Option<super::softbuffer_renderer::SoftbufferRenderer>,
    /// Event loop proxy for sending events.
    proxy: Option<EventLoopProxy>,
    /// Window title.
    title: String,
    width: u32,
    height: u32,
    /// Current keyboard modifier state (winit-specific).
    modifiers: winit::keyboard::ModifiersState,
    /// Native menu bar (attached to the window).
    native_menu: Option<muda::Menu>,

    // ── DevTools ─────────────────────────────────────────────────
    /// Shared DevTools store (persists across open/close cycles).
    devtools_store: Option<super::devtools_store::DevToolsStore>,
    /// DevTools window app.
    devtools_app: Option<RinchApp>,
    /// DevTools window.
    devtools_window: Option<WinitWindow>,
    /// DevTools GPU renderer.
    #[cfg(feature = "gpu")]
    devtools_renderer: Option<WgpuRenderer>,
    /// DevTools software renderer.
    #[cfg(not(feature = "gpu"))]
    devtools_soft_renderer: Option<super::softbuffer_renderer::SoftbufferRenderer>,
    /// DevTools keyboard modifier state.
    devtools_modifiers: winit::keyboard::ModifiersState,
    /// Previous hovered node ID (for detecting changes).
    devtools_prev_hovered: Option<usize>,
    /// Timestamp of the last paint (for FPS calculation).
    last_paint_time: Option<std::time::Instant>,
    /// Recent frame times in ms (ring buffer for FPS averaging).
    frame_times: VecDeque<f64>,
}

impl RinchRuntime {
    fn new(
        title: &str,
        width: u32,
        height: u32,
        component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static,
    ) -> Self {
        Self {
            app: RinchApp::new(component),
            window: None,
            #[cfg(feature = "gpu")]
            renderer: None,
            #[cfg(not(feature = "gpu"))]
            soft_renderer: None,
            proxy: None,
            title: title.to_string(),
            width,
            height,
            modifiers: winit::keyboard::ModifiersState::empty(),
            native_menu: None,
            devtools_store: None,
            devtools_app: None,
            devtools_window: None,
            #[cfg(feature = "gpu")]
            devtools_renderer: None,
            #[cfg(not(feature = "gpu"))]
            devtools_soft_renderer: None,
            devtools_modifiers: winit::keyboard::ModifiersState::empty(),
            devtools_prev_hovered: None,
            last_paint_time: None,
            frame_times: VecDeque::with_capacity(60),
        }
    }

    /// Explicit shutdown: drop resources in the correct order and clear global state.
    fn shutdown(&mut self) {
        // 0. Close DevTools first.
        self.close_devtools();

        // 1. Clear the signal-change callback so no stale closures fire during drop.
        rinch_core::clear_on_signal_change();

        // 2. Drain any pending main-thread callbacks (they may capture app state).
        MAIN_QUEUE.lock().unwrap().clear();

        // 3. Drop the app first (disposes effects/scopes before GPU resources).
        //    RinchApp's own drop order handles _render_scope before doc.
        drop(self.app.component.take());
        drop(self.app._render_scope.take());
        drop(self.app.ce_ops.take());
        drop(self.app.doc.take());

        // 4. Drop renderer before window — Surface holds a window handle reference.
        #[cfg(feature = "gpu")]
        drop(self.renderer.take());
        #[cfg(not(feature = "gpu"))]
        drop(self.soft_renderer.take());

        // 5. Window can now be dropped safely.
        drop(self.window.take());
    }

    // ── DevTools window management ──────────────────────────────────────────

    /// Ensure the DevToolsStore exists (created once, persists across open/close).
    fn ensure_devtools_store(&mut self) {
        if self.devtools_store.is_none() {
            let store = super::devtools_store::DevToolsStore::new();
            rinch_core::create_store(store);
            self.devtools_store = Some(store);
        }
    }

    /// Toggle the DevTools window open/closed.
    fn toggle_devtools(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.devtools_window.is_some() {
            self.close_devtools();
        } else {
            self.open_devtools(event_loop);
        }
    }

    /// Open the DevTools window.
    fn open_devtools(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.devtools_window.is_some() {
            return; // Already open
        }

        self.ensure_devtools_store();
        let store = self.devtools_store.unwrap();
        store.visible.set(true);

        // Provide the main app's document to DevTools via context so it can read directly
        if let Some(doc) = &self.app.doc {
            rinch_core::create_context(super::devtools_store::MainDocRef(doc.clone()));
        }

        // Create DevTools window
        let window_attrs = WindowAttributes::default()
            .with_title("DevTools")
            .with_surface_size(winit::dpi::LogicalSize::new(480u32, 600u32));

        let window = event_loop
            .create_window(window_attrs)
            .expect("Failed to create DevTools window");

        let size = window.surface_size();

        #[cfg(feature = "gpu")]
        {
            let gpu = WgpuRenderer::new(&*window, size.width.max(1), size.height.max(1));
            self.devtools_renderer = Some(gpu);
        }

        let winit_window = WinitWindow::new(window);
        self.devtools_window = Some(winit_window);

        // Create a separate RinchApp for DevTools with the panel component
        let mut dt_app = RinchApp::new(super::devtools_panel::devtools_root);

        // Mount the DevTools component
        dt_app.mount_component(size.width as f32, size.height as f32);

        // Inject DevTools CSS into the document
        if let Some(doc) = &dt_app.doc {
            let mut d = doc.borrow_mut();
            d.load_css(super::devtools_css::DEVTOOLS_CSS);
            d.recompute_all_styles_full();
            d.resolve_layout(size.width as f32, size.height as f32);
        }

        self.devtools_app = Some(dt_app);

        // Bump version to trigger initial tree read in DevTools effects
        store.bump_version();

        // Resolve DevTools layout with the initial data
        self.resolve_devtools();

        // Request initial draw
        self.devtools_window.as_ref().unwrap().request_redraw();

        tracing::info!("DevTools: opened");
    }

    /// Close the DevTools window. Store persists.
    fn close_devtools(&mut self) {
        if let Some(store) = &self.devtools_store {
            store.visible.set(false);
        }

        // Drop DevTools app
        if let Some(mut dt_app) = self.devtools_app.take() {
            drop(dt_app.component.take());
            drop(dt_app._render_scope.take());
            drop(dt_app.ce_ops.take());
            drop(dt_app.doc.take());
        }

        // Drop renderer before window
        #[cfg(feature = "gpu")]
        drop(self.devtools_renderer.take());
        #[cfg(not(feature = "gpu"))]
        drop(self.devtools_soft_renderer.take());

        drop(self.devtools_window.take());

        tracing::info!("DevTools: closed");
    }

    /// Toggle inspect mode on/off.
    fn toggle_inspect_mode(&mut self) {
        self.ensure_devtools_store();
        if let Some(store) = &self.devtools_store {
            let current = store.inspect_mode.get();
            store.inspect_mode.set(!current);
            if current {
                // Turning off: clear highlights
                store.hovered_node_id.set(None);
                self.app.inspect_highlight = None;
                self.app.mark_scene_dirty();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            tracing::info!("Inspect mode: {}", if !current { "ON" } else { "OFF" });
        }
    }

    /// Update inspect highlight from the DevTools store.
    /// Called before painting the main window.
    fn update_inspect_highlight(&mut self) {
        let Some(store) = &self.devtools_store else {
            self.app.inspect_highlight = None;
            return;
        };

        let hovered_id = store.hovered_node_id.get();
        let new_rect = hovered_id.and_then(|node_id| {
            let doc = self.app.doc.as_ref()?;
            let d = doc.borrow();
            let node = d.tree.nodes.get(node_id)?;
            let layout = node
                .taffy_id
                .and_then(|tid| d.tree.taffy.layout(tid).ok())?;
            let (abs_x, abs_y) = compute_absolute_pos(&d.tree, node_id);
            Some((abs_x, abs_y, layout.size.width, layout.size.height))
        });

        if self.app.inspect_highlight != new_rect {
            self.app.inspect_highlight = new_rect;
            self.app.mark_scene_dirty();
        }
    }

    /// Get the DevTools window ID (if open).
    fn devtools_window_id(&self) -> Option<WindowId> {
        self.devtools_window.as_ref().map(|w| w.window.id())
    }

    /// Paint the DevTools window.
    fn paint_devtools(&mut self) -> Result<(), String> {
        #[cfg(feature = "gpu")]
        {
            self.paint_devtools_gpu()
        }
        #[cfg(not(feature = "gpu"))]
        {
            self.paint_devtools_software()
        }
    }

    #[cfg(not(feature = "gpu"))]
    fn paint_devtools_software(&mut self) -> Result<(), String> {
        let Some(window) = &self.devtools_window else {
            return Ok(());
        };
        let Some(dt_app) = &mut self.devtools_app else {
            return Ok(());
        };

        let scale = window.scale_factor();
        let size = window.inner_size();
        let (_base, w, h) = dt_app.build_pixels(scale, size, false);

        let pixels = dt_app
            .skia_painter
            .as_ref()
            .map(|p| p.pixels())
            .unwrap_or(&[]);

        if self.devtools_soft_renderer.is_none() {
            self.devtools_soft_renderer =
                Some(super::softbuffer_renderer::SoftbufferRenderer::new(
                    window.window.clone(),
                    w,
                    h,
                    false,
                ));
        }

        if let Some(renderer) = &mut self.devtools_soft_renderer {
            renderer.present_pixels(pixels, w, h);
        }

        Ok(())
    }

    #[cfg(feature = "gpu")]
    fn paint_devtools_gpu(&mut self) -> Result<(), String> {
        let Some(renderer) = &mut self.devtools_renderer else {
            return Ok(());
        };
        let Some(window) = &self.devtools_window else {
            return Ok(());
        };
        let Some(dt_app) = &mut self.devtools_app else {
            return Ok(());
        };

        let scale = window.scale_factor();
        let size = window.inner_size();
        let scene = dt_app.build_scene(scale, size);
        renderer.paint(scene, false)?;
        Ok(())
    }

    /// Resolve and repaint the DevTools window after signal changes.
    fn resolve_devtools(&mut self) {
        let Some(dt_app) = &mut self.devtools_app else {
            return;
        };
        let Some(window) = &self.devtools_window else {
            return;
        };
        let size = window.inner_size();
        let changed = dt_app.resolve_and_repaint(size.0 as f32, size.1 as f32);
        if changed {
            window.request_redraw();
        }
    }

    fn create_window(&mut self, event_loop: &dyn ActiveEventLoop) {
        let mut window_attrs = WindowAttributes::default()
            .with_title(&self.title)
            .with_surface_size(winit::dpi::LogicalSize::new(self.width, self.height));

        // Apply window props if set
        if let Some(props) = &self.app.window_props {
            if props.borderless {
                window_attrs = window_attrs.with_decorations(false);
            }
            if props.transparent {
                window_attrs = window_attrs.with_transparent(true);
            }
            if !props.resizable {
                window_attrs = window_attrs.with_resizable(false);
            }
            if props.always_on_top {
                window_attrs =
                    window_attrs.with_window_level(winit::window::WindowLevel::AlwaysOnTop);
            }
            if let (Some(x), Some(y)) = (props.x, props.y) {
                window_attrs = window_attrs.with_position(winit::dpi::LogicalPosition::new(x, y));
            }
            if let Some(icon_data) = props.icon {
                // Set window icon (works on X11 and Windows)
                match load_window_icon(icon_data) {
                    Ok(icon) => {
                        window_attrs = window_attrs.with_window_icon(Some(icon));
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load window icon: {}", e);
                    }
                }
                // On Wayland, write icon to a temp file and install a .desktop entry
                // so the compositor can find it via the app_id.
                #[cfg(target_os = "linux")]
                {
                    let app_id = props.app_id.as_deref().unwrap_or("rinch-app");
                    install_wayland_icon(app_id, icon_data);
                }
            }
            // Set app_id / WM_CLASS on Linux for desktop integration
            #[cfg(target_os = "linux")]
            {
                let app_id = props.app_id.as_deref().unwrap_or("rinch-app");
                use winit::platform::wayland::WindowAttributesWayland;
                let wayland_attrs = WindowAttributesWayland::default().with_name(app_id, app_id);
                window_attrs = window_attrs.with_platform_attributes(Box::new(wayland_attrs));
            }
        }

        let window = event_loop
            .create_window(window_attrs)
            .expect("Failed to create window");

        let size = window.surface_size();

        // Create renderer
        #[cfg(feature = "gpu")]
        {
            let gpu = WgpuRenderer::new(&*window, size.width.max(1), size.height.max(1));
            self.renderer = Some(gpu);
        }

        // Attach native menu bar to the window if configured
        if let Some(menu) = &self.native_menu {
            crate::menu::attach_menu_to_window(menu, &*window);
        }

        // Wrap the winit window
        let winit_window = WinitWindow::new(window);
        self.window = Some(winit_window);

        // Register direct redraw callback so render surfaces can request
        // repaints without routing through the event loop queue.
        let window_arc = self.window.as_ref().unwrap().window.clone();
        crate::render_surface::set_redraw_callback(Arc::new(move || {
            window_arc.request_redraw();
        }));

        // Mount the component
        self.app
            .mount_component(size.width as f32, size.height as f32);

        // Request initial draw
        self.window.as_ref().unwrap().request_redraw();
    }

    /// Hide the window by destroying the OS window and GPU surface.
    ///
    /// On Wayland, `set_visible(false)` is a no-op, so we must destroy the
    /// actual winit Window (which destroys the xdg_toplevel) to make it
    /// disappear. The app state and DOM are preserved.
    fn hide_window(&mut self) {
        // Clear the direct redraw callback before dropping the window.
        crate::render_surface::clear_redraw_callback();
        // Drop renderer first — it holds a reference to the window surface.
        #[cfg(feature = "gpu")]
        drop(self.renderer.take());
        #[cfg(not(feature = "gpu"))]
        drop(self.soft_renderer.take());
        drop(self.window.take());
    }

    /// Show a previously hidden window by recreating the OS window and GPU surface.
    ///
    /// Rebuilds the winit Window and wgpu renderer, then triggers a repaint
    /// of the existing DOM.
    fn show_window(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.window.is_some() {
            return; // Already visible
        }

        let mut window_attrs = WindowAttributes::default()
            .with_title(&self.title)
            .with_surface_size(winit::dpi::LogicalSize::new(self.width, self.height));

        if let Some(props) = &self.app.window_props {
            if props.borderless {
                window_attrs = window_attrs.with_decorations(false);
            }
            if props.transparent {
                window_attrs = window_attrs.with_transparent(true);
            }
            if !props.resizable {
                window_attrs = window_attrs.with_resizable(false);
            }
            if props.always_on_top {
                window_attrs =
                    window_attrs.with_window_level(winit::window::WindowLevel::AlwaysOnTop);
            }
            if let Some(icon_data) = props.icon {
                if let Ok(icon) = load_window_icon(icon_data) {
                    window_attrs = window_attrs.with_window_icon(Some(icon));
                }
            }
            #[cfg(target_os = "linux")]
            {
                let app_id = props.app_id.as_deref().unwrap_or("rinch-app");
                use winit::platform::wayland::WindowAttributesWayland;
                let wayland_attrs = WindowAttributesWayland::default().with_name(app_id, app_id);
                window_attrs = window_attrs.with_platform_attributes(Box::new(wayland_attrs));
            }
        }

        let window = event_loop
            .create_window(window_attrs)
            .expect("Failed to recreate window");

        #[cfg(feature = "gpu")]
        {
            let size = window.surface_size();
            let gpu = WgpuRenderer::new(&*window, size.width.max(1), size.height.max(1));
            self.renderer = Some(gpu);
        }

        let winit_window = WinitWindow::new(window);
        self.window = Some(winit_window);

        // Update direct redraw callback for the new window.
        let window_arc = self.window.as_ref().unwrap().window.clone();
        crate::render_surface::set_redraw_callback(Arc::new(move || {
            window_arc.request_redraw();
        }));

        // Trigger a full repaint of the existing DOM
        self.app.scene_dirty = true;
        self.window.as_ref().unwrap().request_redraw();
    }

    /// Paint the current scene to the window.
    fn paint(&mut self) -> Result<(), String> {
        // Update inspect highlight rect from DevTools store before painting
        self.update_inspect_highlight();

        let paint_start = std::time::Instant::now();

        let result = {
            #[cfg(feature = "gpu")]
            {
                self.paint_gpu()
            }
            #[cfg(not(feature = "gpu"))]
            {
                self.paint_software()
            }
        };

        // Track frame timing for DevTools Performance panel
        if let Some(last) = self.last_paint_time {
            let frame_ms = paint_start.duration_since(last).as_secs_f64() * 1000.0;
            if self.frame_times.len() >= 60 {
                self.frame_times.pop_front();
            }
            self.frame_times.push_back(frame_ms);

            if let Some(store) = &self.devtools_store {
                if store.visible.get() {
                    let paint_ms = paint_start.elapsed().as_secs_f64() * 1000.0;
                    store.frame_time_ms.set(paint_ms);

                    // Average FPS over recent frames
                    if !self.frame_times.is_empty() {
                        let avg_ms: f64 =
                            self.frame_times.iter().sum::<f64>() / self.frame_times.len() as f64;
                        if avg_ms > 0.0 {
                            store.fps.set(1000.0 / avg_ms);
                        }
                    }
                }
            }
        }
        self.last_paint_time = Some(paint_start);

        result
    }

    #[cfg(not(feature = "gpu"))]
    fn paint_software(&mut self) -> Result<(), String> {
        let paint_start = std::time::Instant::now();
        let Some(window) = &self.window else {
            return Ok(());
        };

        let scale = window.scale_factor();
        let size = window.inner_size();
        let transparent = self.app.is_transparent();
        let s = scale as f32;

        // Update layout sizes for render surfaces so callbacks get correct dimensions
        let surface_ids = crate::render_surface::registered_surface_ids();
        for &surface_id in &surface_ids {
            if let Some(rect) = self.app.surface_layout_rect(surface_id) {
                let phys_w = (rect.2 * s) as u32;
                let phys_h = (rect.3 * s) as u32;
                crate::render_surface::update_layout_size_by_id(surface_id, phys_w, phys_h);
            }
        }

        // Invoke per-frame render callbacks before collecting frames.
        crate::render_surface::invoke_render_callbacks();

        // Collect surface pixel data and set it for inline painting.
        // Surfaces are painted inline during paint_document() (like <img> elements).
        let surface_pixels = crate::render_surface::collect_surface_pixels_by_id();
        if !surface_pixels.is_empty() {
            // Mark scene dirty so build_pixels() actually repaints
            self.app.mark_scene_dirty();
            rinch_dom::paint::set_surface_pixels(Some(surface_pixels));
        }

        // Also update layout sizes for video/GameViewport surfaces that use the
        // viewport_name path (data-viewport attribute).
        let reg_names = crate::render_surface::registered_viewport_names();
        for viewport_name in &reg_names {
            if let Some(viewport) = self.app.viewport_rect(viewport_name) {
                let phys_w = (viewport.2 * s) as u32;
                let phys_h = (viewport.3 * s) as u32;
                crate::render_surface::update_layout_size(viewport_name, phys_w, phys_h);
            }
        }

        // Collect compositor-path surface frames (video, GameViewport).
        let compositor_frames = crate::render_surface::collect_surface_frames();
        if !compositor_frames.is_empty() {
            self.app.mark_scene_dirty();
        }

        // Build the scene — surfaces paint inline at their layout positions
        let (_base, w, h) = self.app.build_pixels(scale, size, transparent);

        rinch_dom::paint::set_surface_pixels(None);

        // Resolve viewport rects and clip rects for compositor frames before
        // borrowing pixels mutably.
        let blit_ops: Vec<_> = compositor_frames
            .iter()
            .filter_map(|(viewport_name, src_pixels, src_w, src_h)| {
                let (viewport, _radii) = self.app.viewport_rect_with_radius(viewport_name)?;
                let dst_x = (viewport.0 * s) as i32;
                let dst_y = (viewport.1 * s) as i32;
                let dst_w = (viewport.2 * s) as u32;
                let dst_h = (viewport.3 * s) as u32;
                let src_aspect = *src_w as f32 / (*src_h).max(1) as f32;
                let vp_aspect = dst_w as f32 / dst_h.max(1) as f32;
                let (bx, by, bw, bh) = if (src_aspect - vp_aspect).abs() < 0.001 {
                    (dst_x, dst_y, dst_w, dst_h)
                } else if src_aspect > vp_aspect {
                    let fit_h = (dst_w as f32 / src_aspect) as u32;
                    let offset_y = (dst_h - fit_h) as i32 / 2;
                    (dst_x, dst_y + offset_y, dst_w, fit_h)
                } else {
                    let fit_w = (dst_h as f32 * src_aspect) as u32;
                    let offset_x = (dst_w - fit_w) as i32 / 2;
                    (dst_x + offset_x, dst_y, fit_w, dst_h)
                };
                // Get clip rect from nearest overflow-clipping ancestor
                let clip = self.app.viewport_clip_rect(viewport_name).map(|cr| {
                    (
                        (cr.0 * s) as i32,
                        (cr.1 * s) as i32,
                        (cr.2 * s) as u32,
                        (cr.3 * s) as u32,
                    )
                });
                Some((src_pixels.as_slice(), *src_w, *src_h, bx, by, bw, bh, clip))
            })
            .collect();

        // Blit compositor surface frames (video) onto the pixel buffer.
        if !blit_ops.is_empty() {
            if let Some(painter) = self.app.skia_painter.as_mut() {
                let pixels = painter.pixels_mut();
                for &(src_pixels, src_w, src_h, bx, by, bw, bh, clip) in &blit_ops {
                    blit_rgba(pixels, w, h, src_pixels, src_w, src_h, bx, by, bw, bh, clip);
                }
            }
        }

        let pixels = self
            .app
            .skia_painter
            .as_ref()
            .map(|p| p.pixels())
            .unwrap_or(&[]);

        // Lazily create or get the softbuffer renderer
        if self.soft_renderer.is_none() {
            self.soft_renderer = Some(super::softbuffer_renderer::SoftbufferRenderer::new(
                window.window.clone(),
                w,
                h,
                transparent,
            ));
        }

        if let Some(renderer) = &mut self.soft_renderer {
            renderer.present_pixels(pixels, w, h);
        }

        if std::env::var("RINCH_PERF").is_ok() {
            let elapsed = paint_start.elapsed();
            eprintln!(
                "[PERF] paint (software): {:.2}ms",
                elapsed.as_secs_f64() * 1000.0
            );
        }

        Ok(())
    }

    #[cfg(feature = "gpu")]
    fn paint_gpu(&mut self) -> Result<(), String> {
        let paint_start = std::time::Instant::now();
        let Some(renderer) = &mut self.renderer else {
            return Ok(());
        };
        let Some(window) = &self.window else {
            return Ok(());
        };

        let scale = window.scale_factor();
        let size = window.inner_size();
        let transparent = self.app.is_transparent();
        let s = scale as f32;

        // Update layout sizes for all render surfaces so render callbacks
        // receive correct dimensions.
        let surface_ids = crate::render_surface::registered_surface_ids();
        for &surface_id in &surface_ids {
            if let Some(rect) = self.app.surface_layout_rect(surface_id) {
                let phys_w = (rect.2 * s) as u32;
                let phys_h = (rect.3 * s) as u32;
                crate::render_surface::update_layout_size_by_id(surface_id, phys_w, phys_h);
            }
        }

        // Also update layout sizes for video/GameViewport surfaces that still
        // use the viewport_name path (data-viewport attribute).
        let reg_names = crate::render_surface::registered_viewport_names();
        for viewport_name in &reg_names {
            if let Some(viewport) = self.app.viewport_rect(viewport_name) {
                let phys_w = (viewport.2 * s) as u32;
                let phys_h = (viewport.3 * s) as u32;
                crate::render_surface::update_layout_size(viewport_name, phys_w, phys_h);
            }
        }

        // Invoke per-frame render callbacks before collecting frames.
        crate::render_surface::invoke_render_callbacks();

        // Read back GPU textures to CPU for inline-paint RenderSurface components.
        // This must happen before collect_surface_pixels_by_id().
        crate::render_surface::readback_gpu_textures();

        // Collect CPU surface pixel data for inline painting (RenderSurface path).
        // After readback, this includes both pure-CPU and readback-GPU surfaces.
        let surface_pixels = crate::render_surface::collect_surface_pixels_by_id();
        if !surface_pixels.is_empty() {
            self.app.mark_scene_dirty();
            rinch_dom::paint::set_surface_pixels(Some(surface_pixels));
        }

        // Video surfaces still use the compositor/hole-punch path.
        // Track which viewport names have compositor layers so we can set
        // ACTIVE_VIEWPORTS to prevent hole-punching for inline-painted surfaces.
        let mut all_layers: Vec<rinch_platform::CompositeLayer> = Vec::new();
        let mut gpu_layers = Vec::new();
        let mut compositor_viewport_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        // Collect compositor-path surface frames (video, GameViewport — not RenderSurface)
        {
            let surface_frames = crate::render_surface::collect_surface_frames();
            for (viewport_name, pixels, surf_w, surf_h) in surface_frames {
                if let Some((viewport, radii)) = self.app.viewport_rect_with_radius(&viewport_name)
                {
                    compositor_viewport_names.insert(viewport_name.clone());
                    let viewport = (
                        viewport.0 * s,
                        viewport.1 * s,
                        viewport.2 * s,
                        viewport.3 * s,
                    );
                    // Letterbox: fit source within viewport preserving aspect ratio.
                    let viewport = {
                        let (vx, vy, vw, vh) = viewport;
                        let src_aspect = surf_w as f32 / surf_h.max(1) as f32;
                        let vp_aspect = vw / vh.max(1.0);
                        if (src_aspect - vp_aspect).abs() < 0.001 {
                            viewport
                        } else if src_aspect > vp_aspect {
                            let fit_h = vw / src_aspect;
                            let offset_y = (vh - fit_h) / 2.0;
                            (vx, vy + offset_y, vw, fit_h)
                        } else {
                            let fit_w = vh * src_aspect;
                            let offset_x = (vw - fit_w) / 2.0;
                            (vx + offset_x, vy, fit_w, vh)
                        }
                    };
                    let border_radius = [radii[0] * s, radii[1] * s, radii[2] * s, radii[3] * s];
                    let clip_rect = self
                        .app
                        .viewport_clip_rect(&viewport_name)
                        .map(|cr| (cr.0 * s, cr.1 * s, cr.2 * s, cr.3 * s));
                    all_layers.push(rinch_platform::CompositeLayer {
                        pixels,
                        width: surf_w,
                        height: surf_h,
                        viewport,
                        border_radius,
                        clip_rect,
                    });
                }
            }
        }

        // Extract GPU texture sources for compositor — only non-inline surfaces
        // (video, GameViewport). Inline RenderSurface GPU textures are read back
        // to CPU above and painted inline.
        {
            let texture_sources = crate::render_surface::collect_texture_sources();
            for (surface_id, viewport_name, is_inline, tex_source_arc) in texture_sources {
                // Skip inline surfaces — they're already read back to CPU pixels
                if is_inline {
                    continue;
                }

                if let Some((viewport, radii)) = self.app.viewport_rect_with_radius(&viewport_name)
                {
                    compositor_viewport_names.insert(viewport_name.clone());
                    let phys_w = (viewport.2 * s) as u32;
                    let phys_h = (viewport.3 * s) as u32;
                    crate::render_surface::update_layout_size_by_id(surface_id, phys_w, phys_h);

                    let viewport = (
                        viewport.0 * s,
                        viewport.1 * s,
                        viewport.2 * s,
                        viewport.3 * s,
                    );
                    let border_radius = [radii[0] * s, radii[1] * s, radii[2] * s, radii[3] * s];
                    let clip_rect = self
                        .app
                        .viewport_clip_rect(&viewport_name)
                        .map(|cr| (cr.0 * s, cr.1 * s, cr.2 * s, cr.3 * s));
                    if let Some(ref ts) = *tex_source_arc.lock().unwrap() {
                        gpu_layers.push(super::desktop::GpuTextureLayer {
                            view: ts.view.clone(),
                            viewport,
                            border_radius,
                            clip_rect,
                        });
                    }
                }
            }
        }

        // Set or clear composite layers on the renderer (GPU texture + video only)
        if !all_layers.is_empty() || !gpu_layers.is_empty() {
            renderer.set_composite_layers(all_layers);
            renderer.set_gpu_layers(gpu_layers);
        } else if renderer.has_composite_layers() {
            #[cfg(feature = "video")]
            let video_loaded = rinch_video::is_video_loaded();
            #[cfg(not(feature = "video"))]
            let video_loaded = false;

            if !video_loaded {
                renderer.set_composite_layers(vec![]);
                renderer.set_gpu_layers(vec![]);
            }
        }

        // Set active viewports so hole-punching only applies to compositor surfaces
        // (GPU textures + video), not inline-painted CPU surfaces.
        if !compositor_viewport_names.is_empty() {
            rinch_dom::paint::set_active_viewports(Some(compositor_viewport_names));
        }

        // Build scene from document — CPU surfaces paint inline via draw_image()
        let scene = self.app.build_scene(scale, size);

        rinch_dom::paint::set_active_viewports(None);
        rinch_dom::paint::set_surface_pixels(None);

        // Render to screen
        renderer.paint(scene, transparent)?;

        if std::env::var("RINCH_PERF").is_ok() {
            let elapsed = paint_start.elapsed();
            eprintln!("[PERF] paint: {:.2}ms", elapsed.as_secs_f64() * 1000.0);
        }

        Ok(())
    }

    /// Get the current window size.
    fn window_size(&self) -> (u32, u32) {
        self.window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or((self.width, self.height))
    }

    /// Get the current scale factor.
    fn scale_factor(&self) -> f64 {
        self.window
            .as_ref()
            .map(|w| w.scale_factor())
            .unwrap_or(1.0)
    }

    /// Process the actions returned by RinchApp.
    fn process_actions(&mut self, actions: Vec<AppAction>, event_loop: &dyn ActiveEventLoop) {
        for action in actions {
            match action {
                AppAction::RequestRedraw => {
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
                AppAction::Exit => {
                    event_loop.exit();
                }
                AppAction::SetMinimized(minimized) => {
                    if let Some(w) = &self.window {
                        w.set_minimized(minimized);
                    }
                }
                AppAction::SetMaximized(_) => {
                    // Toggle maximize: the app doesn't know current state,
                    // so we toggle it here.
                    if let Some(w) = &self.window {
                        let is_max = w.is_maximized();
                        w.set_maximized(!is_max);
                    }
                }
                AppAction::SetVisible(visible) => {
                    if visible {
                        self.show_window(event_loop);
                    } else {
                        self.hide_window();
                    }
                }
                AppAction::DragWindow => {
                    if let Some(w) = &self.window {
                        let _ = w.drag_window();
                    }
                }
                AppAction::DragResizeWindow(dir) => {
                    if let Some(w) = &self.window {
                        let _ = w.drag_resize_window(dir);
                    }
                }
                AppAction::SetCursor(style) => {
                    if let Some(w) = &self.window {
                        w.window.set_cursor(winit::cursor::Cursor::from(
                            Self::cursor_style_to_winit(style),
                        ));
                    }
                }
                AppAction::ToggleDevTools => {
                    self.toggle_devtools(event_loop);
                }
                AppAction::ToggleInspectMode => {
                    self.toggle_inspect_mode();
                }
            }
        }
    }

    /// Convert a platform CursorStyle to a winit CursorIcon.
    fn cursor_style_to_winit(style: rinch_platform::CursorStyle) -> winit::cursor::CursorIcon {
        use rinch_platform::CursorStyle as CS;
        use winit::cursor::CursorIcon;
        match style {
            CS::Auto | CS::Default => CursorIcon::Default,
            CS::Pointer => CursorIcon::Pointer,
            CS::Text => CursorIcon::Text,
            CS::Move => CursorIcon::Move,
            CS::NotAllowed => CursorIcon::NotAllowed,
            CS::Crosshair => CursorIcon::Crosshair,
            CS::Grab => CursorIcon::Grab,
            CS::Grabbing => CursorIcon::Grabbing,
            CS::ColResize => CursorIcon::ColResize,
            CS::RowResize => CursorIcon::RowResize,
            CS::NResize => CursorIcon::NResize,
            CS::SResize => CursorIcon::SResize,
            CS::EResize => CursorIcon::EResize,
            CS::WResize => CursorIcon::WResize,
            CS::NeResize => CursorIcon::NeResize,
            CS::NwResize => CursorIcon::NwResize,
            CS::SeResize => CursorIcon::SeResize,
            CS::SwResize => CursorIcon::SwResize,
            CS::EwResize => CursorIcon::EwResize,
            CS::NsResize => CursorIcon::NsResize,
            CS::NeswResize => CursorIcon::NeswResize,
            CS::NwseResize => CursorIcon::NwseResize,
            CS::ZoomIn => CursorIcon::ZoomIn,
            CS::ZoomOut => CursorIcon::ZoomOut,
            CS::Wait => CursorIcon::Wait,
            CS::Progress => CursorIcon::Progress,
            CS::Help => CursorIcon::Help,
            CS::ContextMenu => CursorIcon::ContextMenu,
            CS::Cell => CursorIcon::Cell,
            CS::Copy => CursorIcon::Copy,
            CS::Alias => CursorIcon::Alias,
            CS::NoDrop => CursorIcon::NoDrop,
            CS::AllScroll => CursorIcon::AllScroll,
            CS::None => CursorIcon::Default, // winit doesn't have "none"
        }
    }

    /// Translate a winit KeyCode to a platform KeyCode.
    fn translate_key(key_code: winit::keyboard::KeyCode) -> KeyCode {
        use winit::keyboard::KeyCode as WK;
        match key_code {
            WK::ArrowLeft => KeyCode::ArrowLeft,
            WK::ArrowRight => KeyCode::ArrowRight,
            WK::ArrowUp => KeyCode::ArrowUp,
            WK::ArrowDown => KeyCode::ArrowDown,
            WK::Home => KeyCode::Home,
            WK::End => KeyCode::End,
            WK::PageUp => KeyCode::PageUp,
            WK::PageDown => KeyCode::PageDown,
            WK::Enter | WK::NumpadEnter => KeyCode::Enter,
            WK::Backspace => KeyCode::Backspace,
            WK::Delete => KeyCode::Delete,
            WK::Tab => KeyCode::Tab,
            WK::Escape => KeyCode::Escape,
            WK::Space => KeyCode::Space,
            WK::KeyA => KeyCode::KeyA,
            WK::KeyB => KeyCode::KeyB,
            WK::KeyC => KeyCode::KeyC,
            WK::KeyD => KeyCode::KeyD,
            WK::KeyE => KeyCode::KeyE,
            WK::KeyF => KeyCode::KeyF,
            WK::KeyG => KeyCode::KeyG,
            WK::KeyH => KeyCode::KeyH,
            WK::KeyI => KeyCode::KeyI,
            WK::KeyJ => KeyCode::KeyJ,
            WK::KeyK => KeyCode::KeyK,
            WK::KeyL => KeyCode::KeyL,
            WK::KeyM => KeyCode::KeyM,
            WK::KeyN => KeyCode::KeyN,
            WK::KeyO => KeyCode::KeyO,
            WK::KeyP => KeyCode::KeyP,
            WK::KeyQ => KeyCode::KeyQ,
            WK::KeyR => KeyCode::KeyR,
            WK::KeyS => KeyCode::KeyS,
            WK::KeyT => KeyCode::KeyT,
            WK::KeyU => KeyCode::KeyU,
            WK::KeyV => KeyCode::KeyV,
            WK::KeyW => KeyCode::KeyW,
            WK::KeyX => KeyCode::KeyX,
            WK::KeyY => KeyCode::KeyY,
            WK::KeyZ => KeyCode::KeyZ,
            WK::Digit0 => KeyCode::Digit0,
            WK::Digit1 => KeyCode::Digit1,
            WK::Digit2 => KeyCode::Digit2,
            WK::Digit3 => KeyCode::Digit3,
            WK::Digit4 => KeyCode::Digit4,
            WK::Digit5 => KeyCode::Digit5,
            WK::Digit6 => KeyCode::Digit6,
            WK::Digit7 => KeyCode::Digit7,
            WK::Digit8 => KeyCode::Digit8,
            WK::Digit9 => KeyCode::Digit9,
            WK::F1 => KeyCode::F1,
            WK::F2 => KeyCode::F2,
            WK::F3 => KeyCode::F3,
            WK::F4 => KeyCode::F4,
            WK::F5 => KeyCode::F5,
            WK::F6 => KeyCode::F6,
            WK::F7 => KeyCode::F7,
            WK::F8 => KeyCode::F8,
            WK::F9 => KeyCode::F9,
            WK::F10 => KeyCode::F10,
            WK::F11 => KeyCode::F11,
            WK::F12 => KeyCode::F12,
            WK::Equal => KeyCode::Equal,
            WK::Minus => KeyCode::Minus,
            WK::ShiftLeft => KeyCode::ShiftLeft,
            WK::ShiftRight => KeyCode::ShiftRight,
            WK::ControlLeft => KeyCode::ControlLeft,
            WK::ControlRight => KeyCode::ControlRight,
            WK::AltLeft => KeyCode::AltLeft,
            WK::AltRight => KeyCode::AltRight,
            _ => KeyCode::Other,
        }
    }

    /// Translate winit modifier state to platform Modifiers.
    fn translate_modifiers(&self) -> Modifiers {
        Modifiers {
            shift: self.modifiers.shift_key(),
            ctrl: self.modifiers.control_key(),
            alt: self.modifiers.alt_key(),
            meta: self.modifiers.meta_key(),
        }
    }

    /// Capture a screenshot from whichever renderer is active.
    #[cfg(feature = "debug")]
    fn capture_screenshot_impl(&mut self) -> DebugResult {
        #[cfg(not(feature = "gpu"))]
        {
            let scale = self.scale_factor();
            let size = self.window_size();
            // Screenshot: pass empty layers (captures UI only, not live surfaces)
            let (pixels, w, h) = self.app.build_pixels(scale, size, false);
            let png_bytes = screenshot::encode_png(pixels, w, h);
            DebugResult::Bytes {
                data: base64::engine::general_purpose::STANDARD.encode(&png_bytes),
            }
        }
        #[cfg(feature = "gpu")]
        {
            if let Some(renderer) = &self.renderer {
                match renderer.capture_screenshot() {
                    Ok((w, h, rgba)) => {
                        let png_bytes = screenshot::encode_png(&rgba, w, h);
                        DebugResult::Bytes {
                            data: base64::engine::general_purpose::STANDARD.encode(&png_bytes),
                        }
                    }
                    Err(e) => DebugResult::Error {
                        message: format!("Screenshot capture failed: {}", e),
                    },
                }
            } else {
                DebugResult::Error {
                    message: "No renderer".into(),
                }
            }
        }
    }

    /// Handle debug commands that require the renderer (e.g., screenshots).
    /// Returns true if the event loop should exit.
    #[cfg(feature = "debug")]
    fn handle_debug_commands_with_renderer(&mut self) -> bool {
        let Some(rx) = self.app.debug_cmd_rx.take() else {
            return false;
        };

        // Collect commands that need renderer access
        let mut pending = Vec::new();
        while let Ok(cmd) = rx.0.try_recv() {
            pending.push(cmd);
        }

        let mut should_exit = false;

        for cmd in pending {
            let response = match &cmd.kind {
                rinch_debug::DebugCommandKind::Screenshot => {
                    // Screenshot needs the renderer -- handle it here in the shell
                    if let Err(e) = self.paint() {
                        DebugResult::Error {
                            message: format!("Paint failed: {}", e),
                        }
                    } else {
                        self.capture_screenshot_impl()
                    }
                }
                _ => {
                    // All other commands are handled by the platform-agnostic RinchApp
                    let mut actions = Vec::new();
                    let scale = self.scale_factor();
                    let size = self.window_size();
                    // We need to take the kind out of cmd, but cmd.kind is borrowed above.
                    // Re-dispatch through execute_debug_command on the app.
                    let result =
                        self.app
                            .execute_debug_command(cmd.kind.clone(), &mut actions, scale, size);
                    // Process any actions generated
                    for action in actions {
                        match action {
                            AppAction::Exit => {
                                should_exit = true;
                            }
                            AppAction::RequestRedraw => {
                                if let Some(w) = &self.window {
                                    w.request_redraw();
                                }
                            }
                            AppAction::DragWindow => {
                                if let Some(w) = &self.window {
                                    let _ = w.drag_window();
                                }
                            }
                            _ => {}
                        }
                    }
                    result
                }
            };
            let _ = cmd.response_tx.send(response);
        }

        self.app.debug_cmd_rx = Some(rx);
        should_exit
    }
}

impl Drop for RinchRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ── ApplicationHandler impl ──────────────────────────────────────────────────

impl ApplicationHandler for RinchRuntime {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        if self.window.is_none() {
            self.create_window(event_loop);
        }
    }

    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        // Drain main-thread callback queue.
        drain_main_queue();

        // Drain queued native events.
        let events: Vec<_> = NATIVE_EVENT_QUEUE.lock().unwrap().drain(..).collect();
        for event in events {
            self.handle_native_event(event, event_loop);
        }

        // Also resolve DevTools if signals changed
        if self.devtools_window.is_some() {
            self.resolve_devtools();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Route DevTools window events separately
        if Some(window_id) == self.devtools_window_id() {
            self.handle_devtools_window_event(event_loop, event);
            return;
        }

        let platform_event = match event {
            WindowEvent::CloseRequested => PlatformEvent::CloseRequested,
            WindowEvent::SurfaceResized(size) => {
                // Also resize the renderer
                #[cfg(feature = "gpu")]
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width.max(1), size.height.max(1));
                }
                PlatformEvent::Resized {
                    width: size.width,
                    height: size.height,
                }
            }
            WindowEvent::RedrawRequested => {
                // Paint directly -- this is shell-level, not delegated
                if let Err(e) = self.paint() {
                    eprintln!("Paint error: {}", e);
                }
                return;
            }
            WindowEvent::PointerMoved { position, .. } => PlatformEvent::MouseMove {
                x: position.x as f32,
                y: position.y as f32,
            },
            WindowEvent::PointerButton {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                let platform_button = match button {
                    winit::event::ButtonSource::Mouse(MouseButton::Left) => {
                        PlatformMouseButton::Left
                    }
                    winit::event::ButtonSource::Mouse(MouseButton::Right) => {
                        PlatformMouseButton::Right
                    }
                    winit::event::ButtonSource::Mouse(MouseButton::Middle) => {
                        PlatformMouseButton::Middle
                    }
                    _ => return,
                };
                // For click handling, we need the cursor position
                let (x, y) = self.app.cursor_pos.unwrap_or((0.0, 0.0));

                // Inspect mode: click selects node and exits inspect mode
                if platform_button == PlatformMouseButton::Left {
                    if let Some(store) = &self.devtools_store {
                        if store.inspect_mode.get() {
                            if let Some(doc) = &self.app.doc {
                                let d = doc.borrow();
                                let hit = crate::app::hit_testing::hit_test(&d.tree, x, y);
                                store.selected_node_id.set(hit);
                            }
                            store
                                .active_tab
                                .set(super::devtools_store::DevToolsTab::Elements);
                            store.inspect_mode.set(false);
                            store.hovered_node_id.set(None);
                            self.app.inspect_highlight = None;
                            self.app.mark_scene_dirty();
                            if let Some(w) = &self.window {
                                w.request_redraw();
                            }
                            return;
                        }
                    }
                }

                // Set current window ID so window control functions work
                if let Some(w) = &self.window {
                    crate::windows::set_current_window_id(Some(w.window.id()));
                }
                let size = self.window_size();
                let scale = self.scale_factor();
                let actions = self.app.handle_event(
                    PlatformEvent::MouseDown {
                        x,
                        y,
                        button: platform_button,
                    },
                    size,
                    scale,
                );
                crate::windows::set_current_window_id(None);
                self.process_actions(actions, event_loop);
                return;
            }
            WindowEvent::PointerButton {
                state: ElementState::Released,
                button,
                ..
            } => {
                let platform_button = match button {
                    winit::event::ButtonSource::Mouse(MouseButton::Left) => {
                        PlatformMouseButton::Left
                    }
                    winit::event::ButtonSource::Mouse(MouseButton::Right) => {
                        PlatformMouseButton::Right
                    }
                    winit::event::ButtonSource::Mouse(MouseButton::Middle) => {
                        PlatformMouseButton::Middle
                    }
                    _ => return,
                };
                let (x, y) = self.app.cursor_pos.unwrap_or((0.0, 0.0));
                PlatformEvent::MouseUp {
                    x,
                    y,
                    button: platform_button,
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => {
                        (x as f64 * 40.0, y as f64 * 40.0)
                    }
                    winit::event::MouseScrollDelta::PixelDelta(pos) => (pos.x, pos.y),
                };
                let (cx, cy) = self.app.cursor_pos.unwrap_or((0.0, 0.0));
                PlatformEvent::MouseWheel {
                    x: cx,
                    y: cy,
                    delta_x: dx,
                    delta_y: dy,
                }
            }
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.modifiers = new_modifiers.state();
                let mods = self.translate_modifiers();
                PlatformEvent::ModifiersChanged(mods)
            }
            WindowEvent::KeyboardInput {
                event:
                    winit::event::KeyEvent {
                        physical_key: winit::keyboard::PhysicalKey::Code(key_code),
                        state: ElementState::Pressed,
                        ref text,
                        ..
                    },
                ..
            } => {
                // Check menu shortcuts first — if matched, consume the event
                let mods = self.translate_modifiers();
                if crate::menu::match_shortcut(mods.ctrl, mods.meta, mods.alt, mods.shift, key_code)
                {
                    // Shortcut matched and callback dispatched; request redraw
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }

                let platform_key = Self::translate_key(key_code);
                PlatformEvent::KeyDown {
                    key: platform_key,
                    text: text.as_ref().map(|t| t.to_string()),
                    modifiers: mods,
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    winit::event::KeyEvent {
                        physical_key: winit::keyboard::PhysicalKey::Code(key_code),
                        state: ElementState::Released,
                        ..
                    },
                ..
            } => {
                let mods = self.translate_modifiers();
                let platform_key = Self::translate_key(key_code);
                PlatformEvent::KeyUp {
                    key: platform_key,
                    modifiers: mods,
                }
            }
            WindowEvent::DragEntered { paths, position } => {
                let size = self.window_size();
                let scale = self.scale_factor();
                if paths.is_empty() {
                    // Wayland: paths arrive on drop, not on enter.
                    // Fire a FileDragMoved to trigger hover tracking.
                    let actions = self.app.handle_event(
                        PlatformEvent::FileDragMoved {
                            position: (position.x, position.y),
                        },
                        size,
                        scale,
                    );
                    self.process_actions(actions, event_loop);
                } else {
                    // X11/Windows: paths are known on enter.
                    for path in paths {
                        let actions = self.app.handle_event(
                            PlatformEvent::FileHoverEnter {
                                path,
                                position: (position.x, position.y),
                            },
                            size,
                            scale,
                        );
                        self.process_actions(actions, event_loop);
                    }
                }
                return;
            }
            WindowEvent::DragMoved { position } => PlatformEvent::FileDragMoved {
                position: (position.x, position.y),
            },
            WindowEvent::DragDropped { paths, position } => PlatformEvent::FileDropped {
                paths,
                position: (position.x, position.y),
            },
            WindowEvent::DragLeft { .. } => PlatformEvent::FileHoverCancelled,
            _ => return,
        };

        // Inspect mode: intercept mouse events for hit-testing
        if let Some(store) = &self.devtools_store {
            if store.inspect_mode.get() {
                if let PlatformEvent::MouseMove { x, y } = &platform_event {
                    // Hit-test the main app's document
                    self.app.cursor_pos = Some((*x, *y));
                    if let Some(doc) = &self.app.doc {
                        let d = doc.borrow();
                        let hit = crate::app::hit_testing::hit_test(&d.tree, *x, *y);
                        store.hovered_node_id.set(hit);
                    }
                }
            }
        }

        let size = self.window_size();
        let scale = self.scale_factor();
        let actions = self.app.handle_event(platform_event, size, scale);
        self.process_actions(actions, event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        let size = self.window_size();
        let scale = self.scale_factor();
        let actions = self
            .app
            .handle_event(PlatformEvent::AboutToWait, size, scale);
        self.process_actions(actions, event_loop);

        // Notify DevTools when main app DOM changed, then resolve its layout
        if self.devtools_window.is_some() {
            // Bump version counter so DevTools effects re-read the document.
            // The version only changes when the main app actually resolved
            // dirty nodes (scene_dirty is set in resolve_and_repaint).
            if self.app.scene_dirty {
                if let Some(store) = &self.devtools_store {
                    store.bump_version();
                }
            }

            self.resolve_devtools();

            // If hovered node changed in DevTools, request main window repaint
            // for inspect highlight
            if let Some(store) = &self.devtools_store {
                let new_hovered = store.hovered_node_id.get();
                if new_hovered != self.devtools_prev_hovered {
                    self.devtools_prev_hovered = new_hovered;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
        }
    }
}

impl RinchRuntime {
    /// Handle a single native event from the queue.
    fn handle_native_event(&mut self, event: RinchNativeEvent, event_loop: &dyn ActiveEventLoop) {
        let platform_event = match event {
            RinchNativeEvent::ReRender => PlatformEvent::UserEvent(UserEvent::ReRender),
            #[cfg(feature = "debug")]
            RinchNativeEvent::DebugCommand => {
                // Debug commands may need the renderer (screenshots), so
                // we handle them here in the shell instead of delegating.
                if self.handle_debug_commands_with_renderer() {
                    event_loop.exit();
                }
                return;
            }
            RinchNativeEvent::MinimizeWindow => PlatformEvent::UserEvent(UserEvent::MinimizeWindow),
            RinchNativeEvent::ToggleMaximizeWindow => {
                PlatformEvent::UserEvent(UserEvent::ToggleMaximizeWindow)
            }
            RinchNativeEvent::CloseWindowControl => {
                PlatformEvent::UserEvent(UserEvent::CloseWindow)
            }
            RinchNativeEvent::ShowWindow => PlatformEvent::UserEvent(UserEvent::ShowWindow),
            RinchNativeEvent::HideWindow => PlatformEvent::UserEvent(UserEvent::HideWindow),
        };
        let size = self.window_size();
        let scale = self.scale_factor();
        let actions = self.app.handle_event(platform_event, size, scale);
        self.process_actions(actions, event_loop);
    }

    /// Handle events for the DevTools window.
    fn handle_devtools_window_event(
        &mut self,
        _event_loop: &dyn ActiveEventLoop,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.close_devtools();
            }
            WindowEvent::SurfaceResized(size) => {
                #[cfg(feature = "gpu")]
                if let Some(renderer) = &mut self.devtools_renderer {
                    renderer.resize(size.width.max(1), size.height.max(1));
                }
                if let Some(dt_app) = &mut self.devtools_app {
                    dt_app.resize_layout(size.width, size.height);
                }
                if let Some(w) = &self.devtools_window {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.paint_devtools() {
                    eprintln!("DevTools paint error: {}", e);
                }
            }
            WindowEvent::PointerMoved { position, .. } => {
                if let Some(dt_app) = &mut self.devtools_app {
                    let size = self
                        .devtools_window
                        .as_ref()
                        .map(|w| w.inner_size())
                        .unwrap_or((480, 600));
                    let scale = self
                        .devtools_window
                        .as_ref()
                        .map(|w| w.scale_factor())
                        .unwrap_or(1.0);
                    let actions = dt_app.handle_event(
                        PlatformEvent::MouseMove {
                            x: position.x as f32,
                            y: position.y as f32,
                        },
                        size,
                        scale,
                    );
                    for action in actions {
                        if let AppAction::RequestRedraw = action {
                            if let Some(w) = &self.devtools_window {
                                w.request_redraw();
                            }
                        }
                    }
                }
            }
            WindowEvent::PointerButton {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                let platform_button = match button {
                    winit::event::ButtonSource::Mouse(MouseButton::Left) => {
                        PlatformMouseButton::Left
                    }
                    winit::event::ButtonSource::Mouse(MouseButton::Right) => {
                        PlatformMouseButton::Right
                    }
                    winit::event::ButtonSource::Mouse(MouseButton::Middle) => {
                        PlatformMouseButton::Middle
                    }
                    _ => return,
                };
                if let Some(dt_app) = &mut self.devtools_app {
                    let (x, y) = dt_app.cursor_pos.unwrap_or((0.0, 0.0));
                    let size = self
                        .devtools_window
                        .as_ref()
                        .map(|w| w.inner_size())
                        .unwrap_or((480, 600));
                    let scale = self
                        .devtools_window
                        .as_ref()
                        .map(|w| w.scale_factor())
                        .unwrap_or(1.0);
                    let actions = dt_app.handle_event(
                        PlatformEvent::MouseDown {
                            x,
                            y,
                            button: platform_button,
                        },
                        size,
                        scale,
                    );
                    for action in actions {
                        if let AppAction::RequestRedraw = action {
                            if let Some(w) = &self.devtools_window {
                                w.request_redraw();
                            }
                        }
                    }
                }
            }
            WindowEvent::PointerButton {
                state: ElementState::Released,
                button,
                ..
            } => {
                let platform_button = match button {
                    winit::event::ButtonSource::Mouse(MouseButton::Left) => {
                        PlatformMouseButton::Left
                    }
                    winit::event::ButtonSource::Mouse(MouseButton::Right) => {
                        PlatformMouseButton::Right
                    }
                    winit::event::ButtonSource::Mouse(MouseButton::Middle) => {
                        PlatformMouseButton::Middle
                    }
                    _ => return,
                };
                if let Some(dt_app) = &mut self.devtools_app {
                    let (x, y) = dt_app.cursor_pos.unwrap_or((0.0, 0.0));
                    let size = self
                        .devtools_window
                        .as_ref()
                        .map(|w| w.inner_size())
                        .unwrap_or((480, 600));
                    let scale = self
                        .devtools_window
                        .as_ref()
                        .map(|w| w.scale_factor())
                        .unwrap_or(1.0);
                    let actions = dt_app.handle_event(
                        PlatformEvent::MouseUp {
                            x,
                            y,
                            button: platform_button,
                        },
                        size,
                        scale,
                    );
                    for action in actions {
                        if let AppAction::RequestRedraw = action {
                            if let Some(w) = &self.devtools_window {
                                w.request_redraw();
                            }
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => {
                        (x as f64 * 40.0, y as f64 * 40.0)
                    }
                    winit::event::MouseScrollDelta::PixelDelta(pos) => (pos.x, pos.y),
                };
                if let Some(dt_app) = &mut self.devtools_app {
                    let (cx, cy) = dt_app.cursor_pos.unwrap_or((0.0, 0.0));
                    let size = self
                        .devtools_window
                        .as_ref()
                        .map(|w| w.inner_size())
                        .unwrap_or((480, 600));
                    let scale = self
                        .devtools_window
                        .as_ref()
                        .map(|w| w.scale_factor())
                        .unwrap_or(1.0);
                    let actions = dt_app.handle_event(
                        PlatformEvent::MouseWheel {
                            x: cx,
                            y: cy,
                            delta_x: dx,
                            delta_y: dy,
                        },
                        size,
                        scale,
                    );
                    for action in actions {
                        if let AppAction::RequestRedraw = action {
                            if let Some(w) = &self.devtools_window {
                                w.request_redraw();
                            }
                        }
                    }
                }
            }
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.devtools_modifiers = new_modifiers.state();
            }
            WindowEvent::KeyboardInput {
                event:
                    winit::event::KeyEvent {
                        physical_key: winit::keyboard::PhysicalKey::Code(key_code),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                // F12 in DevTools window also toggles
                if key_code == winit::keyboard::KeyCode::F12 {
                    self.close_devtools();
                }
            }
            _ => {}
        }
    }
}

// ── DevTools data extraction helpers ─────────────────────────────────────────

use rinch_dom::node::RawNodeId;

/// Compute absolute position of a node by walking up the parent chain.
fn compute_absolute_pos(tree: &rinch_dom::node::NodeTree, id: RawNodeId) -> (f32, f32) {
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut current = Some(id);
    while let Some(nid) = current {
        if let Some(node) = tree.get(nid) {
            x += node.layout.x;
            y += node.layout.y;
            if node.computed_style.position == rinch_dom::computed_style::PositionValue::Fixed {
                break;
            }
            if let Some(parent_id) = node.parent {
                if let Some(parent) = tree.get(parent_id) {
                    x -= parent.scroll_offset.0 as f32;
                    y -= parent.scroll_offset.1 as f32;
                }
            }
            current = node.parent;
        } else {
            break;
        }
    }
    (x, y)
}

// ── Software compositor blit helper ──────────────────────────────────────────

/// Nearest-neighbor blit of an RGBA source into a destination pixel buffer.
///
/// Scales `src` (src_w x src_h) into the destination rectangle
/// (blit_x, blit_y, blit_w, blit_h) within the `dst` buffer (dst_w x dst_h).
/// Clips to both destination bounds and an optional clip rect from a parent
/// overflow container.
#[cfg(not(feature = "gpu"))]
#[allow(clippy::too_many_arguments)]
fn blit_rgba(
    dst: &mut [u8],
    dst_w: u32,
    dst_h: u32,
    src: &[u8],
    src_w: u32,
    src_h: u32,
    blit_x: i32,
    blit_y: i32,
    blit_w: u32,
    blit_h: u32,
    clip: Option<(i32, i32, u32, u32)>,
) {
    if blit_w == 0 || blit_h == 0 || src_w == 0 || src_h == 0 {
        return;
    }

    // Compute effective clip bounds (intersection of dst bounds and clip rect)
    let (clip_min_x, clip_min_y, clip_max_x, clip_max_y) = if let Some((cx, cy, cw, ch)) = clip {
        (cx, cy, cx + cw as i32, cy + ch as i32)
    } else {
        (0, 0, dst_w as i32, dst_h as i32)
    };

    let dst_stride = dst_w as usize * 4;
    let src_stride = src_w as usize * 4;

    for dy in 0..blit_h {
        let out_y = blit_y + dy as i32;
        if out_y < 0 || out_y >= dst_h as i32 || out_y < clip_min_y || out_y >= clip_max_y {
            continue;
        }
        let sy = ((dy as f32 / blit_h as f32) * src_h as f32) as u32;
        let sy = sy.min(src_h - 1) as usize;

        for dx in 0..blit_w {
            let out_x = blit_x + dx as i32;
            if out_x < 0 || out_x >= dst_w as i32 || out_x < clip_min_x || out_x >= clip_max_x {
                continue;
            }
            let sx = ((dx as f32 / blit_w as f32) * src_w as f32) as u32;
            let sx = sx.min(src_w - 1) as usize;

            let src_off = sy * src_stride + sx * 4;
            let dst_off = out_y as usize * dst_stride + out_x as usize * 4;

            if src_off + 3 < src.len() && dst_off + 3 < dst.len() {
                let r = src[src_off];
                let g = src[src_off + 1];
                let b = src[src_off + 2];
                let a = src[src_off + 3];
                if a == 255 {
                    dst[dst_off] = r;
                    dst[dst_off + 1] = g;
                    dst[dst_off + 2] = b;
                    dst[dst_off + 3] = a;
                } else if a > 0 {
                    let af = a as f32 / 255.0;
                    dst[dst_off] = (r as f32 * af) as u8;
                    dst[dst_off + 1] = (g as f32 * af) as u8;
                    dst[dst_off + 2] = (b as f32 * af) as u8;
                    dst[dst_off + 3] = a;
                }
            }
        }
    }
}

// ── Public entry points ──────────────────────────────────────────────────────

/// Run a rinch application using the rinch-dom rendering pipeline.
///
/// This is an alternative to [`run`](crate::shell::run) that uses rinch-dom
/// (Taffy + Parley + Vello) for layout and rendering.
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
///
/// fn app(__scope: &mut RenderScope) -> NodeHandle {
///     let count = Signal::new(0);
///     let count_inc = count.clone();
///     rsx! {
///         div { style: "display: flex; flex-direction: column; padding: 20px; gap: 10px;",
///             p { "Count: " {|| count.get().to_string()} }
///             button {
///                 onclick: move || count_inc.update(|n| *n += 1),
///                 style: "padding: 8px 16px; background-color: #007bff; color: white;",
///                 "Increment"
///             }
///         }
///     }
/// }
///
/// fn main() {
///     rinch::run_rinch("Counter", 800, 600, app);
/// }
/// ```
#[deprecated(since = "0.2.0", note = "Use `run` instead")]
pub fn run_rinch<F>(title: &str, width: u32, height: u32, component: F)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();

    // Clear stale state
    events::clear_handlers();
    clear_context();

    let event_loop = EventLoop::new().expect("Failed to create event loop");

    let proxy = event_loop.create_proxy();

    // Register main thread and cross-thread dispatcher for Signal::send()
    rinch_core::register_main_thread();
    rinch_core::set_cross_thread_dispatcher(dispatch_to_main_thread);

    // Set up signal change notification
    rinch_core::set_on_signal_change(move || {
        send_native_event(RinchNativeEvent::ReRender);
    });

    let mut runtime = RinchRuntime::new(title, width, height, component);
    runtime.proxy = Some(proxy.clone());

    // Install push-based menu event handler (covers native + tray menus)
    crate::menu::install_menu_event_handler();

    // Set native proxy for window control functions
    NATIVE_PROXY.with(|p| *p.borrow_mut() = Some(proxy.clone()));

    // Set global proxy so run_on_main_thread() works from any thread
    let _ = GLOBAL_PROXY.set(proxy.clone());

    // Register video frame sink factory so video players deliver frames
    // through the RenderSurface compositing pipeline.
    #[cfg(feature = "video")]
    {
        rinch_video::set_frame_sink_factory(|viewport_id: &str| {
            let handle = crate::render_surface::create_render_surface_with_name(viewport_id);
            let writer = handle.writer();
            // Keep the handle alive by leaking it — the surface lives for
            // the lifetime of the video player.
            std::mem::forget(handle);
            std::sync::Arc::new(move |pixels: &[u8], w: u32, h: u32| {
                writer.submit_frame(pixels, w, h);
            })
        });
    }

    // Start debug IPC server if feature is enabled (disable with RINCH_DEBUG=0)
    #[cfg(feature = "debug")]
    {
        if std::env::var("RINCH_DEBUG").map_or(true, |v| v != "0") {
            match rinch_debug::attach(title, move || {
                send_native_event(RinchNativeEvent::DebugCommand);
            }) {
                Ok((debug_server, cmd_rx)) => {
                    tracing::info!("rinch-debug listening on port {}", debug_server.port());
                    runtime.app._debug_server = Some(debug_server);
                    runtime.app.debug_cmd_rx = Some(cmd_rx);
                }
                Err(e) => {
                    tracing::warn!("Failed to start rinch-debug server: {}", e);
                }
            }
        }
    }

    event_loop.run_app(runtime).expect("Event loop error");
}

/// Run a rinch-dom application with full window configuration.
#[deprecated(since = "0.2.0", note = "Use `run_with_window_props` instead")]
pub fn run_rinch_with_window_props<F>(component: F, props: rinch_core::element::WindowProps)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    run_rinch_with_window_props_and_menu(component, props, None);
}

/// Run a rinch-dom application with full window configuration and optional native menu.
pub fn run_rinch_with_window_props_and_menu<F>(
    component: F,
    props: rinch_core::element::WindowProps,
    native_menu: Option<muda::Menu>,
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();

    events::clear_handlers();
    clear_context();

    let event_loop = EventLoop::new().expect("Failed to create event loop");

    let proxy = event_loop.create_proxy();

    // Register main thread and cross-thread dispatcher for Signal::send()
    rinch_core::register_main_thread();
    rinch_core::set_cross_thread_dispatcher(dispatch_to_main_thread);

    rinch_core::set_on_signal_change(move || {
        send_native_event(RinchNativeEvent::ReRender);
    });

    let mut runtime = RinchRuntime::new(&props.title, props.width, props.height, component);
    runtime.proxy = Some(proxy.clone());
    runtime.app.set_window_props(props.clone());
    runtime.native_menu = native_menu;

    // Install push-based menu event handler (covers native + tray menus)
    crate::menu::install_menu_event_handler();

    // Set native proxy for window control functions
    NATIVE_PROXY.with(|p| *p.borrow_mut() = Some(proxy.clone()));

    // Set global proxy so run_on_main_thread() works from any thread
    let _ = GLOBAL_PROXY.set(proxy.clone());

    // Register video frame sink factory so video players deliver frames
    // through the RenderSurface compositing pipeline.
    #[cfg(feature = "video")]
    {
        rinch_video::set_frame_sink_factory(|viewport_id: &str| {
            let handle = crate::render_surface::create_render_surface_with_name(viewport_id);
            let writer = handle.writer();
            std::mem::forget(handle);
            std::sync::Arc::new(move |pixels: &[u8], w: u32, h: u32| {
                writer.submit_frame(pixels, w, h);
            })
        });
    }

    #[cfg(feature = "debug")]
    {
        if std::env::var("RINCH_DEBUG").map_or(true, |v| v != "0") {
            match rinch_debug::attach(&props.title, move || {
                send_native_event(RinchNativeEvent::DebugCommand);
            }) {
                Ok((debug_server, cmd_rx)) => {
                    tracing::info!("rinch-debug listening on port {}", debug_server.port());
                    runtime.app._debug_server = Some(debug_server);
                    runtime.app.debug_cmd_rx = Some(cmd_rx);
                }
                Err(e) => {
                    tracing::warn!("Failed to start rinch-debug server: {}", e);
                }
            }
        }
    }

    event_loop.run_app(runtime).expect("Event loop error");
}

/// Decode a PNG from raw bytes into RGBA pixel data.
///
/// Returns `(rgba_bytes, width, height)`.
pub(crate) fn decode_png_to_rgba(
    png_data: &[u8],
) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    let decoder = png::Decoder::new(png_data);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    let bytes = &buf[..info.buffer_size()];

    // Convert to RGBA8 if needed
    let rgba = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(bytes.len() / 3 * 4);
            for chunk in bytes.chunks(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        other => {
            return Err(format!("Unsupported PNG color type: {:?}", other).into());
        }
    };

    Ok((rgba, info.width, info.height))
}

/// Decode a PNG icon from raw bytes into a winit Icon.
fn load_window_icon(png_data: &[u8]) -> Result<winit::icon::Icon, Box<dyn std::error::Error>> {
    let (rgba, width, height) = decode_png_to_rgba(png_data)?;
    let rgba_icon = winit::icon::RgbaIcon::new(rgba, width, height)?;
    Ok(rgba_icon.into())
}

/// Write the icon PNG to a data directory and create a `.desktop` file so Wayland
/// compositors can display the icon in the taskbar via `app_id` matching.
#[cfg(target_os = "linux")]
fn install_wayland_icon(app_id: &str, png_data: &[u8]) {
    let Some(data_home) = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs_icon_fallback)
    else {
        return;
    };

    // Decode and resize icon to 256x256 for desktop integration
    let icon_dir = data_home.join("rinch/icons");
    if std::fs::create_dir_all(&icon_dir).is_err() {
        return;
    }
    let icon_path = icon_dir.join(format!("{app_id}.png"));
    let resized_png = match resize_png_icon(png_data, 256) {
        Ok(data) => data,
        Err(e) => {
            tracing::warn!("Failed to resize icon: {}", e);
            return;
        }
    };
    if std::fs::write(&icon_path, &resized_png).is_err() {
        return;
    }

    // Write .desktop file with absolute icon path (most reliable across compositors)
    let desktop_dir = data_home.join("applications");
    if std::fs::create_dir_all(&desktop_dir).is_err() {
        return;
    }
    let desktop_path = desktop_dir.join(format!("{app_id}.desktop"));
    let desktop_content = format!(
        "[Desktop Entry]\nType=Application\nName={app_id}\nIcon={}\nNoDisplay=true\n",
        icon_path.display()
    );
    let _ = std::fs::write(&desktop_path, desktop_content);
}

#[cfg(target_os = "linux")]
fn dirs_icon_fallback() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
}

/// Decode a PNG, resize to `target_size` x `target_size` with bilinear filtering, re-encode as PNG.
#[cfg(target_os = "linux")]
fn resize_png_icon(
    png_data: &[u8],
    target_size: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Decode source PNG
    let decoder = png::Decoder::new(png_data);
    let mut reader = decoder.read_info()?;
    let mut src_buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut src_buf)?;
    let src = &src_buf[..info.buffer_size()];
    let (sw, sh) = (info.width as usize, info.height as usize);

    // Convert to RGBA if needed
    let rgba_src: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => src.to_vec(),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(sw * sh * 4);
            for chunk in src.chunks(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        other => return Err(format!("Unsupported color type: {:?}", other).into()),
    };

    let ts = target_size as usize;
    let mut dst = vec![0u8; ts * ts * 4];

    // Bilinear downscale
    for dy in 0..ts {
        for dx in 0..ts {
            let sx_f = (dx as f64 + 0.5) * sw as f64 / ts as f64 - 0.5;
            let sy_f = (dy as f64 + 0.5) * sh as f64 / ts as f64 - 0.5;
            let x0 = (sx_f.floor() as isize).clamp(0, sw as isize - 1) as usize;
            let y0 = (sy_f.floor() as isize).clamp(0, sh as isize - 1) as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let y1 = (y0 + 1).min(sh - 1);
            let fx = (sx_f - x0 as f64).clamp(0.0, 1.0);
            let fy = (sy_f - y0 as f64).clamp(0.0, 1.0);

            for c in 0..4 {
                let p00 = rgba_src[(y0 * sw + x0) * 4 + c] as f64;
                let p10 = rgba_src[(y0 * sw + x1) * 4 + c] as f64;
                let p01 = rgba_src[(y1 * sw + x0) * 4 + c] as f64;
                let p11 = rgba_src[(y1 * sw + x1) * 4 + c] as f64;
                let val = p00 * (1.0 - fx) * (1.0 - fy)
                    + p10 * fx * (1.0 - fy)
                    + p01 * (1.0 - fx) * fy
                    + p11 * fx * fy;
                dst[(dy * ts + dx) * 4 + c] = val.round() as u8;
            }
        }
    }

    // Encode to PNG
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, target_size, target_size);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&dst)?;
    }
    Ok(out)
}
