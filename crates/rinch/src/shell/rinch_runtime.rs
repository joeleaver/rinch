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
    AppAction, ImeEvent, KeyCode, Modifiers, MouseButton as PlatformMouseButton, PlatformEvent,
    PlatformWindow, UserEvent, to_logical,
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
    let was_empty = {
        let mut q = MAIN_QUEUE.lock().unwrap();
        let empty = q.is_empty();
        q.push(Box::new(f));
        empty
    };
    // Only wake the event loop if the queue was empty — subsequent calls
    // within the same batch coalesce into a single ReRender event.
    if was_empty {
        send_native_event(RinchNativeEvent::ReRender);
    }
}

/// Dispatcher function for cross-thread signal updates.
///
/// Registered with `rinch_core::set_cross_thread_dispatcher()` so that
/// `Signal::send()` can automatically route updates to the main thread.
fn dispatch_to_main_thread(f: Box<dyn FnOnce() + Send>) {
    let was_empty = {
        let mut q = MAIN_QUEUE.lock().unwrap();
        let empty = q.is_empty();
        q.push(f);
        empty
    };
    if was_empty {
        send_native_event(RinchNativeEvent::ReRender);
    }
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
    /// An injected platform event (e.g. synthetic gamepad input).
    InjectedPlatformEvent(PlatformEvent),
    /// An accessibility action requested by the AT (delivered from the adapter's
    /// thread; applied to the focused editor on the main thread).
    #[cfg(feature = "a11y")]
    A11yAction(accesskit::ActionRequest),
}

/// Inject a synthetic [`PlatformEvent`] into the rinch event loop.
///
/// The event is queued and processed on the next event loop tick, just like
/// real platform events from winit. The runtime uses the current window size
/// and scale factor when dispatching.
///
/// This is useful for translating gamepad input or other external input
/// sources into rinch UI events (e.g. arrow key presses for focus navigation).
///
/// Must be called from the main thread or any thread — the event is queued
/// behind a `Mutex` and the event loop proxy is woken.
///
/// # Example
///
/// ```ignore
/// use rinch_platform::{PlatformEvent, KeyCode, Modifiers};
///
/// // Simulate a down-arrow key press for gamepad D-pad
/// rinch::inject_platform_event(PlatformEvent::KeyDown {
///     key: KeyCode::ArrowDown,
///     text: None,
///     modifiers: Modifiers::default(),
/// });
/// ```
pub fn inject_platform_event(event: PlatformEvent) {
    send_native_event(RinchNativeEvent::InjectedPlatformEvent(event));
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
    /// True while [`Self::drain_native_events`] is running. Prevents re-entrant
    /// drains: `DebugCommandKind::Screenshot` calls [`Self::paint`] from inside
    /// the drain, and the `RedrawRequested` arm drains before painting (#153).
    draining_native_events: bool,

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

    // ── IME ──────────────────────────────────────────────────────
    /// Whether IME composition is currently enabled on the window. Mirrors the
    /// desired state from the focus arbiter so [`Self::sync_ime`] only issues a
    /// winit `request_ime_update` on an actual change.
    ime_enabled: bool,
    /// Last-applied IME cursor area (logical window `x, y, w, h`), to avoid
    /// re-issuing identical `Update` requests every frame.
    ime_cursor_area: Option<(f32, f32, f32, f32)>,

    // ── Accessibility (M7b) ──────────────────────────────────────
    /// The per-platform AccessKit bridge (Linux AT-SPI; no-op elsewhere). Created
    /// lazily on first window creation; pushes the editor's tree on change.
    #[cfg(feature = "a11y")]
    a11y: Option<crate::editor::a11y::AccesskitBridge>,
    /// The (doc, selection) last pushed to the AT, so the tree is re-derived only
    /// when the focused editor actually changed.
    #[cfg(feature = "a11y")]
    a11y_last: Option<(rinch_editor_core::Node, rinch_editor_core::Selection)>,
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
            draining_native_events: false,
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
            ime_enabled: false,
            ime_cursor_area: None,
            #[cfg(feature = "a11y")]
            a11y: None,
            #[cfg(feature = "a11y")]
            a11y_last: None,
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
        let dt_scale = window.scale_factor();

        #[cfg(feature = "gpu")]
        {
            let gpu = WgpuRenderer::new(&*window, size.width.max(1), size.height.max(1));
            self.devtools_renderer = Some(gpu);
        }

        let winit_window = WinitWindow::new(window);
        self.devtools_window = Some(winit_window);

        // Create a separate RinchApp for DevTools with the panel component
        let mut dt_app = RinchApp::new(super::devtools_panel::devtools_root);

        // Mount the DevTools component at its logical viewport size.
        let dt_logical = to_logical((size.width, size.height), dt_scale);
        dt_app.mount_component(dt_logical.0 as f32, dt_logical.1 as f32);

        // Inject DevTools CSS into the document
        if let Some(doc) = &dt_app.doc {
            let mut d = doc.borrow_mut();
            d.load_css(super::devtools_css::DEVTOOLS_CSS);
            d.recompute_all_styles_full();
            d.resolve_layout(dt_logical.0 as f32, dt_logical.1 as f32);
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

    /// The DevTools window's surface size, in physical pixels — what
    /// `RinchApp::handle_event` expects (it derives the logical viewport
    /// itself). Falls back to the size DevTools is created at.
    fn devtools_size(&self) -> (u32, u32) {
        self.devtools_window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or((480, 600))
    }

    /// The DevTools window's scale factor.
    fn devtools_scale_factor(&self) -> f64 {
        self.devtools_window
            .as_ref()
            .map(|w| w.scale_factor())
            .unwrap_or(1.0)
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
        let size = window.logical_size();
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
        let size = window.logical_size();
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
        let size = window.logical_size();
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
        let scale = window.scale_factor();

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

        // Mount the component at the *logical* viewport size — `size` above is
        // the physical surface size (see `to_logical`).
        let logical = to_logical((size.width, size.height), scale);
        self.app.mount_component(logical.0 as f32, logical.1 as f32);

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

        // Ensure pending cross-thread closures are processed and layout
        // is resolved before painting (same as paint_gpu).
        drain_main_queue();
        rinch_core::reactive::drain_polls();
        let scale = window.scale_factor();
        // Layout and paint work in logical (CSS) pixels; `inner_size` is
        // physical (see `to_logical`).
        let size = window.logical_size();
        if self.app.has_pending_layout() {
            self.app.resolve_and_repaint(size.0 as f32, size.1 as f32);
        }

        let transparent = self.app.is_transparent();
        let s = scale as f32;

        // Update layout sizes for render surfaces. Scans the DOM for
        // data-render-surface attributes rather than relying on the registry,
        // so surfaces survive reactive scope rebuilds (where unregister runs
        // after re-mount, temporarily removing the surface from the registry).
        for (surface_id, rect) in self.app.all_surface_layout_rects() {
            let phys_w = (rect.2 * s) as u32;
            let phys_h = (rect.3 * s) as u32;
            crate::render_surface::update_layout_size_by_id(surface_id, phys_w, phys_h);
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
                crate::render_surface::update_layout_position(
                    viewport_name,
                    viewport.0,
                    viewport.1,
                );
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

        // Ensure pending cross-thread closures (run_on_main_thread) are
        // processed and layout is resolved before painting. Without this,
        // continuous RedrawRequested from render surfaces can paint with
        // stale layout if the ReRender event from signal changes hasn't
        // been processed yet.
        drain_main_queue();
        rinch_core::reactive::drain_polls();
        let scale = window.scale_factor();
        // Layout and paint work in logical (CSS) pixels; `inner_size` is
        // physical (see `to_logical`).
        let size = window.logical_size();
        if self.app.has_pending_layout() {
            self.app.resolve_and_repaint(size.0 as f32, size.1 as f32);
        }

        let transparent = self.app.is_transparent();
        let s = scale as f32;

        // Update layout sizes for all render surfaces. DOM-based scan
        // so surfaces survive reactive scope rebuilds.
        for (surface_id, rect) in self.app.all_surface_layout_rects() {
            let phys_w = (rect.2 * s) as u32;
            let phys_h = (rect.3 * s) as u32;
            crate::render_surface::update_layout_size_by_id(surface_id, phys_w, phys_h);
        }

        // Also update layout sizes for video/GameViewport surfaces that still
        // use the viewport_name path (data-viewport attribute).
        let reg_names = crate::render_surface::registered_viewport_names();
        for viewport_name in &reg_names {
            if let Some(viewport) = self.app.viewport_rect(viewport_name) {
                let phys_w = (viewport.2 * s) as u32;
                let phys_h = (viewport.3 * s) as u32;
                crate::render_surface::update_layout_size(viewport_name, phys_w, phys_h);
                crate::render_surface::update_layout_position(
                    viewport_name,
                    viewport.0,
                    viewport.1,
                );
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

    /// Get the current window size, in physical pixels.
    ///
    /// This is what the OS surface and the renderer are sized in, and what
    /// pointer coordinates arrive in — `RinchApp::handle_event` divides it by
    /// the scale factor itself. Layout and paint want [`Self::logical_size`].
    ///
    /// Before the window exists there is no surface, so the fallback is the
    /// requested size, which `create_window` passes as a `LogicalSize`; the
    /// scale factor is 1x until then, so the two agree.
    fn window_size(&self) -> (u32, u32) {
        self.window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or((self.width, self.height))
    }

    /// Get the current window size in logical (CSS) pixels.
    ///
    /// Only the software debug-screenshot path needs this; the paint paths
    /// derive it from the `window` they already hold.
    #[cfg(all(feature = "debug", not(feature = "gpu")))]
    fn logical_size(&self) -> (u32, u32) {
        // `self.width`/`self.height` are already the requested *logical* size
        // (`create_window` hands them to winit as a `LogicalSize`), so the
        // no-window fallback must not divide them again.
        self.window
            .as_ref()
            .map(|w| w.logical_size())
            .unwrap_or((self.width.max(1), self.height.max(1)))
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

    /// The single ASCII letter a winit **logical** key produces in the active layout,
    /// lowercased — the layout-mapped identity of a letter key (e.g. the Dvorak key at
    /// the QWERTY-N position reports `Character("b")` → `Some('b')`). `None` for
    /// non-letters (so the editor falls back to the physical `KeyCode`). Feeds
    /// `PlatformEvent::KeyDown::logical_key` so `Mod`+letter shortcuts follow the keycap.
    fn winit_logical_letter(key: &winit::keyboard::Key) -> Option<char> {
        if let winit::keyboard::Key::Character(s) = key {
            let mut it = s.chars();
            if let (Some(c), None) = (it.next(), it.next())
                && c.is_ascii_alphabetic()
            {
                return Some(c.to_ascii_lowercase());
            }
        }
        None
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
            let size = self.logical_size();
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

        let mut should_exit = false;

        // Process commands as they arrive. After answering command N, wait a
        // short beat for a pipelined follow-up — a fast client sends command
        // N+1 the moment it reads response N, but by then the paint N induced
        // is usually already dispatched, so without this window N+1 would
        // stall behind that full paint (#153). Batching here lets the whole
        // burst ride a single paint. The window is bounded so a spamming
        // client cannot starve painting.
        let batch_start = std::time::Instant::now();
        let mut next = rx.0.try_recv().ok();
        while let Some(cmd) = next {
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

            next = if batch_start.elapsed() < std::time::Duration::from_millis(25) {
                rx.0.recv_timeout(std::time::Duration::from_millis(2)).ok()
            } else {
                rx.0.try_recv().ok()
            };
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
        // Track whether any reactive state changes while we drain queued work,
        // so the redraw decision below can key off the actual cause (a signal
        // changed) instead of repainting on every wake.
        rinch_core::clear_signals_changed();

        // Drain main-thread callback queue.
        drain_main_queue();

        // Drain queued native events.
        self.drain_native_events(event_loop);

        // A cross-thread Signal::send()/update_send() drained above runs its
        // effects on this thread, but the ReRender handler's resolve_and_repaint
        // doesn't always flag that mutation as a paint-worthy change (e.g. the
        // dirty state was already consumed earlier in this batch). If a signal
        // actually changed — or the DOM is still dirty — force a repaint so
        // background-thread updates reliably show up instead of waiting for the
        // next input event. Wakes that change nothing (window controls, debug
        // commands, no-op callbacks) skip the redraw.
        if rinch_core::signals_changed() || self.app.has_pending_layout() {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
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
                // Also resize the renderer — it works in physical pixels.
                #[cfg(feature = "gpu")]
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width.max(1), size.height.max(1));
                }
                // `Resized` re-lays out the document, so it carries the
                // logical size (see `to_logical`).
                let logical = to_logical((size.width, size.height), self.scale_factor());
                PlatformEvent::Resized {
                    width: logical.0,
                    height: logical.1,
                }
            }
            WindowEvent::RedrawRequested => {
                // Drain queued native events (debug commands, injected input)
                // before painting so a command that arrived while the loop was
                // busy isn't serialized behind a full paint (#153). Pre-paint
                // is safe: the paint preamble re-resolves layout when
                // has_pending_layout(), so the paint sees the drained state.
                self.drain_native_events(event_loop);
                // Paint directly -- this is shell-level, not delegated
                if let Err(e) = self.paint() {
                    eprintln!("Paint error: {}", e);
                }
                // winit coalesces wake_up() calls arbitrarily, so a wake that
                // landed mid-paint may already have been consumed. If events
                // queued while we painted, self-wake so they're delivered
                // immediately instead of waiting for the next external event
                // (#153).
                if !NATIVE_EVENT_QUEUE.lock().unwrap().is_empty() {
                    if let Some(proxy) = GLOBAL_PROXY.get() {
                        proxy.wake_up();
                    }
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
                    winit::event::ButtonSource::Mouse(MouseButton::Left)
                    | winit::event::ButtonSource::Touch { .. } => PlatformMouseButton::Left,
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
                    winit::event::ButtonSource::Mouse(MouseButton::Left)
                    | winit::event::ButtonSource::Touch { .. } => PlatformMouseButton::Left,
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
            WindowEvent::Focused(focused) => {
                // Tell the AT whether this window has OS focus.
                #[cfg(feature = "a11y")]
                if let Some(bridge) = self.a11y.as_mut() {
                    bridge.set_window_focused(focused);
                }
                // …and the app, which notifies the focused widget without
                // releasing its claim (issue #147, decision 1). This used to
                // `return` here, so window blur was invisible to the document:
                // a custom widget's caret kept blinking in an unfocused window
                // and the OS IME stayed armed on a window without the keyboard.
                PlatformEvent::WindowFocus(focused)
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
                        logical_key: ref win_logical,
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
                    // The layout-mapped letter (so Mod+letter follows the keycap, not the
                    // physical position) — winit's logical key carries it even when a
                    // modifier suppresses `text`.
                    logical_key: Self::winit_logical_letter(win_logical),
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
            WindowEvent::Ime(ime) => {
                // Translate winit's IME composition events into the portable
                // `ImeEvent` contract. `handle_event` then routes them through
                // the focus arbiter, exactly like keyboard input — so the
                // editor, `<input>`, etc. all consume IME the same way.
                let ime_event = match ime {
                    winit::event::Ime::Enabled => ImeEvent::Enabled,
                    winit::event::Ime::Preedit(text, cursor) => ImeEvent::Preedit { text, cursor },
                    winit::event::Ime::Commit(text) => ImeEvent::Commit(text),
                    winit::event::Ime::DeleteSurrounding {
                        before_bytes,
                        after_bytes,
                    } => ImeEvent::DeleteSurrounding {
                        // winit reports UTF-8 byte counts; the target converts to
                        // its char-based model. Exact byte→char conversion is only
                        // needed once we advertise the surrounding-text capability
                        // (not enabled yet), so this passes the counts through.
                        before: before_bytes,
                        after: after_bytes,
                    },
                    winit::event::Ime::Disabled => ImeEvent::Disabled,
                };
                PlatformEvent::Ime(ime_event)
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

        // Reconcile the window's IME state with the focus arbiter (enable on a
        // focused text target, follow the caret, disable on blur).
        self.sync_ime();

        // Push the focused editor's accessibility tree to the AT when it changed.
        #[cfg(feature = "a11y")]
        self.sync_a11y();

        // Drive the focused editor's caret blink. This is the only thing that
        // arms a timed wake (`WaitUntil`); when nothing is blinking it returns the
        // loop to `Wait` so the app stays idle.
        #[cfg(feature = "desktop")]
        self.tick_caret_blink(event_loop);
    }
}

impl RinchRuntime {
    /// Blink the focused editor's caret by arming a `WaitUntil` wake for the next
    /// phase toggle (see [`crate::editor::caret_blink_tick`]). Owns the event
    /// loop's control flow: `WaitUntil` while a caret blinks, `Wait` otherwise.
    #[cfg(feature = "desktop")]
    fn tick_caret_blink(&mut self, event_loop: &dyn ActiveEventLoop) {
        let focused = self.app.focused_editor_id();
        match crate::editor::caret_blink_tick(self.app.doc_key(), focused) {
            Some(blink) => {
                if blink.redraw
                    && let Some(w) = &self.window
                {
                    w.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::WaitUntil(
                    std::time::Instant::now() + blink.next,
                ));
            }
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    /// Push the focus arbiter's desired IME state to the window. Enables IME with
    /// a cursor area when a text target is focused, disables it otherwise, and
    /// moves the candidate-box rect as the caret moves. Diffs against the
    /// last-applied state so it only issues a `request_ime_update` on a real
    /// change. This is the single place the winit IME surface is touched; every
    /// text target (editor, `<input>`, …) feeds it through [`RinchApp::ime_state`].
    fn sync_ime(&mut self) {
        use winit::window::{ImeCapabilities, ImeEnableRequest, ImeRequest, ImeRequestData};

        let Some(w) = &self.window else { return };
        let desired = self.app.ime_state();

        let to_area = |a: (f32, f32, f32, f32)| {
            let (x, y, width, height) = a;
            (
                winit::dpi::LogicalPosition::new(x as f64, y as f64),
                winit::dpi::LogicalSize::new(width.max(1.0) as f64, height.max(1.0) as f64),
            )
        };

        if desired.enabled != self.ime_enabled {
            if desired.enabled {
                let area = desired.cursor_area.unwrap_or((0.0, 0.0, 1.0, 16.0));
                let (pos, size) = to_area(area);
                let caps = ImeCapabilities::new().with_cursor_area();
                let data = ImeRequestData::default().with_cursor_area(pos.into(), size.into());
                if let Some(req) = ImeEnableRequest::new(caps, data) {
                    let _ = w.window.request_ime_update(ImeRequest::Enable(req));
                }
                self.ime_cursor_area = desired.cursor_area;
            } else {
                let _ = w.window.request_ime_update(ImeRequest::Disable);
                self.ime_cursor_area = None;
            }
            self.ime_enabled = desired.enabled;
        } else if desired.enabled && desired.cursor_area != self.ime_cursor_area {
            if let Some(area) = desired.cursor_area {
                let (pos, size) = to_area(area);
                let data = ImeRequestData::default().with_cursor_area(pos.into(), size.into());
                let _ = w.window.request_ime_update(ImeRequest::Update(data));
            }
            self.ime_cursor_area = desired.cursor_area;
        }
    }

    /// Push the focused editor's accessibility tree to the AT when its document or
    /// selection changed since the last push (the AT consumes a full snapshot). The
    /// per-platform bridge is created lazily on first editor focus; on Linux it is
    /// the AT-SPI adapter, elsewhere a no-op. The whole method compiles to nothing
    /// without the `a11y` feature.
    #[cfg(feature = "a11y")]
    fn sync_a11y(&mut self) {
        let Some(handle) = self
            .app
            .focused_editor_id()
            .and_then(|id| crate::editor::editor_for_doc(self.app.doc_key(), id))
        else {
            return;
        };
        let state = handle.state();
        let unchanged = matches!(
            &self.a11y_last,
            Some((doc, sel)) if doc.same_ref(&state.doc) && *sel == state.selection
        );
        if unchanged {
            return;
        }
        let bridge = self
            .a11y
            .get_or_insert_with(crate::editor::a11y::AccesskitBridge::new);
        bridge.update(&state);
        self.a11y_last = Some((state.doc.clone(), state.selection.clone()));
    }

    /// Apply an AT-requested action to the focused editor. A screen reader moving
    /// the caret/selection arrives as `SetTextSelection`; `Focus` is advisory (the
    /// focus arbiter owns focus). Runs on the main thread (from the native-event
    /// queue) after the adapter thread forwarded the request.
    #[cfg(feature = "a11y")]
    fn apply_a11y_action(&mut self, req: accesskit::ActionRequest) {
        use accesskit::{Action, ActionData};
        let Some(handle) = self
            .app
            .focused_editor_id()
            .and_then(|id| crate::editor::editor_for_doc(self.app.doc_key(), id))
        else {
            return;
        };
        if req.action == Action::SetTextSelection
            && let Some(ActionData::SetTextSelection(ak_sel)) = &req.data
            && let Some(sel) =
                rinch_editor_core::a11y::accesskit_selection_to_model(&handle.doc(), ak_sel)
        {
            handle.set_selection(sel);
            send_native_event(RinchNativeEvent::ReRender);
        }
    }

    /// Drain queued native events (debug commands, window controls, injected
    /// platform events) and dispatch each through [`Self::handle_native_event`].
    ///
    /// Called from `proxy_wake_up` and from the top of the `RedrawRequested`
    /// arm — the latter so a command that arrived while the loop was busy is
    /// processed *before* the paint instead of stalling a full paint behind it
    /// (#153). Guarded against re-entry: `DebugCommandKind::Screenshot` calls
    /// [`Self::paint`] from inside the drain, so a nested drain must no-op.
    fn drain_native_events(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.draining_native_events {
            return;
        }
        self.draining_native_events = true;
        let events: Vec<_> = NATIVE_EVENT_QUEUE.lock().unwrap().drain(..).collect();
        for event in events {
            self.handle_native_event(event, event_loop);
        }
        self.draining_native_events = false;
    }

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
            RinchNativeEvent::InjectedPlatformEvent(pe) => pe,
            #[cfg(feature = "a11y")]
            RinchNativeEvent::A11yAction(req) => {
                self.apply_a11y_action(req);
                return;
            }
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
                let dt_scale = self.devtools_scale_factor();
                if let Some(dt_app) = &mut self.devtools_app {
                    let logical = to_logical((size.width, size.height), dt_scale);
                    dt_app.resize_layout(logical.0, logical.1);
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
                let size = self.devtools_size();
                let scale = self.devtools_scale_factor();
                if let Some(dt_app) = &mut self.devtools_app {
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
                    winit::event::ButtonSource::Mouse(MouseButton::Left)
                    | winit::event::ButtonSource::Touch { .. } => PlatformMouseButton::Left,
                    winit::event::ButtonSource::Mouse(MouseButton::Right) => {
                        PlatformMouseButton::Right
                    }
                    winit::event::ButtonSource::Mouse(MouseButton::Middle) => {
                        PlatformMouseButton::Middle
                    }
                    _ => return,
                };
                let size = self.devtools_size();
                let scale = self.devtools_scale_factor();
                if let Some(dt_app) = &mut self.devtools_app {
                    let (x, y) = dt_app.cursor_pos.unwrap_or((0.0, 0.0));
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
                    winit::event::ButtonSource::Mouse(MouseButton::Left)
                    | winit::event::ButtonSource::Touch { .. } => PlatformMouseButton::Left,
                    winit::event::ButtonSource::Mouse(MouseButton::Right) => {
                        PlatformMouseButton::Right
                    }
                    winit::event::ButtonSource::Mouse(MouseButton::Middle) => {
                        PlatformMouseButton::Middle
                    }
                    _ => return,
                };
                let size = self.devtools_size();
                let scale = self.devtools_scale_factor();
                if let Some(dt_app) = &mut self.devtools_app {
                    let (x, y) = dt_app.cursor_pos.unwrap_or((0.0, 0.0));
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
                let size = self.devtools_size();
                let scale = self.devtools_scale_factor();
                if let Some(dt_app) = &mut self.devtools_app {
                    let (cx, cy) = dt_app.cursor_pos.unwrap_or((0.0, 0.0));
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
                        physical_key:
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F12),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                // F12 in DevTools window also toggles
                self.close_devtools();
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
    // If the app already has a system-wide launcher (e.g. it was installed from
    // a package), don't write a user-level NoDisplay stub of the same name: it
    // would shadow that launcher and hide the app from the application menu. The
    // packaged .desktop already supplies the taskbar icon via app_id/StartupWMClass.
    if system_desktop_entry_exists(app_id) {
        return;
    }

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

/// Returns true if a `.desktop` entry for `app_id` exists in any system
/// application directory (`$XDG_DATA_DIRS`, default `/usr/local/share:/usr/share`).
#[cfg(target_os = "linux")]
fn system_desktop_entry_exists(app_id: &str) -> bool {
    // Read XDG_DATA_DIRS as an OsString and split with split_paths so non-UTF8
    // directory entries are preserved (var() would drop the whole value to the
    // default on any non-UTF8 byte).
    let dirs = std::env::var_os("XDG_DATA_DIRS")
        .unwrap_or_else(|| std::ffi::OsString::from("/usr/local/share:/usr/share"));
    let rel = format!("applications/{app_id}.desktop");
    std::env::split_paths(&dirs)
        .filter(|p| !p.as_os_str().is_empty())
        .any(|p| p.join(&rel).exists())
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

#[cfg(test)]
mod viewport_tests {
    use super::*;

    /// A window that reports a physical surface size and a scale factor.
    struct FakeWindow {
        surface: (u32, u32),
        scale: f64,
    }

    impl PlatformWindow for FakeWindow {
        fn inner_size(&self) -> (u32, u32) {
            self.surface
        }
        fn scale_factor(&self) -> f64 {
            self.scale
        }
        fn request_redraw(&self) {}
        fn set_minimized(&self, _minimized: bool) {}
        fn set_maximized(&self, _maximized: bool) {}
        fn set_visible(&self, _visible: bool) {}
        fn is_maximized(&self) -> bool {
            false
        }
        fn drag_window(&self) -> Result<(), String> {
            Ok(())
        }
        fn set_title(&self, _title: &str) {}
    }

    /// The desktop shell used to hand the document `PlatformWindow::inner_size`
    /// — the *physical* surface size — as its layout viewport, while paint
    /// multiplies every layout coordinate by the same window's scale factor.
    /// On a 1.25x display that laid the page out 1.25x too wide and then drew
    /// it 1.25x larger again, so the rightmost fifth fell outside the surface:
    /// a `flex: 1` child's trailing siblings (a row's confidence dots, the box
    /// after a growing one) were laid out correctly and simply never appeared.
    ///
    /// `run("probe", 460, 300, ..)` on a 1.25x display gets a 575x975 surface;
    /// the viewport is 460 CSS pixels wide, not 575.
    #[test]
    fn layout_viewport_is_logical_not_physical() {
        let window = FakeWindow {
            surface: (575, 975),
            scale: 1.25,
        };
        assert_eq!(
            window.logical_size(),
            (460, 780),
            "layout must get the logical viewport, not the 575x975 surface size"
        );
    }

    /// The page must fit the surface once paint has scaled it — the invariant
    /// the physical-size viewport broke.
    #[test]
    fn scaled_layout_viewport_fits_the_surface() {
        for &(surface, scale) in &[
            ((575u32, 975u32), 1.25f64),
            ((786, 1704), 2.0),
            ((491, 1065), 1.25),
        ] {
            let window = FakeWindow { surface, scale };
            let (vw, vh) = window.logical_size();
            let painted = (vw as f64 * scale, vh as f64 * scale);
            assert!(
                (painted.0 - surface.0 as f64).abs() <= 1.0
                    && (painted.1 - surface.1 as f64).abs() <= 1.0,
                "viewport {vw}x{vh} painted at {scale}x is {painted:?}, \
                 which does not fill the {surface:?} surface"
            );
        }
    }

    /// A 1x display is unaffected, and a nonsensical scale factor must not
    /// collapse the viewport to zero.
    #[test]
    fn layout_viewport_degenerate_scales() {
        assert_eq!(
            FakeWindow {
                surface: (800, 600),
                scale: 1.0
            }
            .logical_size(),
            (800, 600)
        );
        assert_eq!(
            FakeWindow {
                surface: (800, 600),
                scale: 0.0
            }
            .logical_size(),
            (800, 600),
            "a zero scale factor must fall back to 1x, not divide by zero"
        );
        assert_eq!(
            FakeWindow {
                surface: (1, 1),
                scale: 4.0
            }
            .logical_size(),
            (1, 1),
            "the viewport must never round down to zero"
        );
        for bad in [f64::NAN, f64::INFINITY, -2.0] {
            assert_eq!(
                FakeWindow {
                    surface: (800, 600),
                    scale: bad
                }
                .logical_size(),
                (800, 600),
                "a {bad} scale factor must fall back to 1x"
            );
        }
    }

    /// The viewport the shell lays out at and the one `RinchApp::handle_event`
    /// re-lays out at on every `ReRender` / `AboutToWait` must be the *same*
    /// number. They are derived on opposite sides of the shell boundary — the
    /// shell from the window, the app from the physical size it was handed — so
    /// a divergence here silently reintroduces the oversized layout one frame
    /// after mount.
    #[test]
    fn app_and_shell_agree_on_the_layout_viewport() {
        for &(surface, scale) in &[
            ((575u32, 975u32), 1.25f64),
            ((577, 977), 1.25),
            ((786, 1704), 2.0),
            ((800, 600), 1.0),
        ] {
            let window = FakeWindow { surface, scale };
            let (sw, sh) = window.logical_size();
            let (aw, ah) = RinchApp::layout_viewport(surface, scale);
            assert_eq!(
                (sw as f32, sh as f32),
                (aw, ah),
                "shell laid out {surface:?}@{scale}x at {sw}x{sh} but the app \
                 re-lays it out at {aw}x{ah}"
            );
        }
    }
}
