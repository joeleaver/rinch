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
use std::sync::{Arc, Mutex, OnceLock};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use rinch_core::clear_context;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::events;
use rinch_platform::{
    AppAction, KeyCode, Modifiers, MouseButton as PlatformMouseButton, PlatformEvent,
    PlatformRenderer, PlatformWindow, UserEvent,
};

use crate::app::RinchApp;

use super::desktop::{WgpuRenderer, WinitWindow};

#[cfg(feature = "debug")]
use {super::screenshot, base64::Engine, rinch_debug::DebugResult};

// ── Thread-local proxy ───────────────────────────────────────────────────────

// Thread-local proxy for the native event loop, used by window control functions.
thread_local! {
    pub(crate) static NATIVE_PROXY: RefCell<Option<EventLoopProxy<RinchNativeEvent>>> = const { RefCell::new(None) };
}

// ── Global proxy + main-thread callback queue ────────────────────────────────

// Global (Send+Sync) proxy for waking the event loop from any thread.
// Public within the crate so render_surface.rs can send SurfaceRedraw directly.
pub(crate) static GLOBAL_PROXY: OnceLock<EventLoopProxy<RinchNativeEvent>> = OnceLock::new();

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
    if let Some(proxy) = GLOBAL_PROXY.get() {
        let _ = proxy.send_event(RinchNativeEvent::ReRender);
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
    /// A RenderSurface has new GPU texture content — repaint the compositor
    /// without re-resolving DOM layout.
    SurfaceRedraw,
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
    /// GPU renderer.
    renderer: Option<WgpuRenderer>,
    /// Event loop proxy for sending events.
    proxy: Option<EventLoopProxy<RinchNativeEvent>>,
    /// Window title.
    title: String,
    width: u32,
    height: u32,
    /// Current keyboard modifier state (winit-specific).
    modifiers: winit::keyboard::ModifiersState,
    /// Native menu bar (attached to the window).
    native_menu: Option<muda::Menu>,
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
            renderer: None,
            proxy: None,
            title: title.to_string(),
            width,
            height,
            modifiers: winit::keyboard::ModifiersState::empty(),
            native_menu: None,
        }
    }

    /// Explicit shutdown: drop resources in the correct order and clear global state.
    fn shutdown(&mut self) {
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

        // 4. Drop GPU renderer before window — Surface holds a window handle reference.
        drop(self.renderer.take());

        // 5. Window can now be dropped safely.
        drop(self.window.take());
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) {
        let mut window_attrs = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(winit::dpi::LogicalSize::new(self.width, self.height));

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
                use winit::platform::wayland::WindowAttributesExtWayland;
                window_attrs = window_attrs.with_name(app_id, app_id);
            }
        }

        let window = event_loop
            .create_window(window_attrs)
            .expect("Failed to create window");
        let window = Arc::new(window);

        let size = window.inner_size();

        // Create GPU renderer
        let gpu = WgpuRenderer::new(&window, size.width.max(1), size.height.max(1));
        self.renderer = Some(gpu);

        // Attach native menu bar to the window if configured
        if let Some(menu) = &self.native_menu {
            crate::menu::attach_menu_to_window(menu, &window);
        }

        // Wrap the winit window
        let winit_window = WinitWindow::new(window.clone());
        self.window = Some(winit_window);

        // Mount the component
        self.app
            .mount_component(size.width as f32, size.height as f32);

        // Request initial draw
        window.request_redraw();
    }

    /// Hide the window by destroying the OS window and GPU surface.
    ///
    /// On Wayland, `set_visible(false)` is a no-op, so we must destroy the
    /// actual winit Window (which destroys the xdg_toplevel) to make it
    /// disappear. The app state and DOM are preserved.
    fn hide_window(&mut self) {
        // Drop renderer first — it holds a reference to the window surface.
        drop(self.renderer.take());
        drop(self.window.take());
    }

    /// Show a previously hidden window by recreating the OS window and GPU surface.
    ///
    /// Rebuilds the winit Window and wgpu renderer, then triggers a repaint
    /// of the existing DOM.
    fn show_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return; // Already visible
        }

        let mut window_attrs = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(winit::dpi::LogicalSize::new(self.width, self.height));

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
                use winit::platform::wayland::WindowAttributesExtWayland;
                window_attrs = window_attrs.with_name(app_id, app_id);
            }
        }

        let window = event_loop
            .create_window(window_attrs)
            .expect("Failed to recreate window");
        let window = Arc::new(window);

        let size = window.inner_size();
        let gpu = WgpuRenderer::new(&window, size.width.max(1), size.height.max(1));
        self.renderer = Some(gpu);

        let winit_window = WinitWindow::new(window.clone());
        self.window = Some(winit_window);

        // Trigger a full repaint of the existing DOM
        self.app.scene_dirty = true;
        window.request_redraw();
    }

    /// Paint the current scene to the window.
    fn paint(&mut self) -> Result<(), String> {
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

        // Collect all composite layers (video + render surfaces)
        let mut all_layers: Vec<rinch_platform::CompositeLayer> = Vec::new();
        let s = scale as f32;

        // Extract video frames for compositing
        #[cfg(feature = "video")]
        {
            // When videos are actively playing, grab new frames for compositing.
            // When paused/ended but still loaded, keep the last frame visible.
            // Only clear video layers when all videos are fully unloaded (cleanup).
            if rinch_video::is_video_active() {
                let frames = rinch_video::collect_video_frames();
                for (viewport_name, pixels, vid_w, vid_h) in frames {
                    if let Some(viewport) = self.app.viewport_rect(&viewport_name) {
                        // Scale viewport from logical to physical pixels
                        let viewport = (
                            viewport.0 * s,
                            viewport.1 * s,
                            viewport.2 * s,
                            viewport.3 * s,
                        );

                        // Letterbox: fit video within viewport preserving aspect ratio
                        let viewport = {
                            let (vx, vy, vw, vh) = viewport;
                            let video_aspect = vid_w as f32 / vid_h as f32;
                            let viewport_aspect = vw / vh;
                            if video_aspect > viewport_aspect {
                                let fit_h = vw / video_aspect;
                                let offset_y = (vh - fit_h) / 2.0;
                                (vx, vy + offset_y, vw, fit_h)
                            } else {
                                let fit_w = vh * video_aspect;
                                let offset_x = (vw - fit_w) / 2.0;
                                (vx + offset_x, vy, fit_w, vh)
                            }
                        };

                        all_layers.push(rinch_platform::CompositeLayer {
                            pixels,
                            width: vid_w,
                            height: vid_h,
                            viewport,
                        });
                    }
                }
            }
        }

        // Update layout sizes for all render surfaces so render callbacks
        // receive correct dimensions.  Previously this was only done for GPU
        // texture-source surfaces; CPU callback-only surfaces never got their
        // layout_size set, so their callbacks were skipped (w==0, h==0).
        for viewport_name in crate::render_surface::registered_viewport_names() {
            if let Some(viewport) = self.app.viewport_rect(&viewport_name) {
                let phys_w = (viewport.2 * s) as u32;
                let phys_h = (viewport.3 * s) as u32;
                crate::render_surface::update_layout_size(&viewport_name, phys_w, phys_h);
            }
        }

        // Invoke per-frame render callbacks before collecting frames
        crate::render_surface::invoke_render_callbacks();

        // Extract render surface frames for compositing (CPU pixel path)
        {
            let surface_frames = crate::render_surface::collect_surface_frames();
            for (viewport_name, pixels, surf_w, surf_h) in surface_frames {
                if let Some(viewport) = self.app.viewport_rect(&viewport_name) {
                    // Scale viewport from logical to physical pixels
                    let viewport = (
                        viewport.0 * s,
                        viewport.1 * s,
                        viewport.2 * s,
                        viewport.3 * s,
                    );
                    all_layers.push(rinch_platform::CompositeLayer {
                        pixels,
                        width: surf_w,
                        height: surf_h,
                        viewport,
                    });
                }
            }
        }

        // Extract GPU texture sources for zero-copy compositing
        let mut gpu_layers = Vec::new();
        {
            let texture_sources = crate::render_surface::collect_texture_sources();
            for (viewport_name, tex_source_arc) in texture_sources {
                if let Some(viewport) = self.app.viewport_rect(&viewport_name) {
                    let phys_w = (viewport.2 * s) as u32;
                    let phys_h = (viewport.3 * s) as u32;

                    // Update layout size so the engine thread can match its
                    // offscreen texture to the actual viewport dimensions.
                    crate::render_surface::update_layout_size(&viewport_name, phys_w, phys_h);

                    let viewport = (
                        viewport.0 * s,
                        viewport.1 * s,
                        viewport.2 * s,
                        viewport.3 * s,
                    );
                    // Lock the texture source to get the view
                    if let Some(ref ts) = *tex_source_arc.lock().unwrap() {
                        gpu_layers.push(super::desktop::GpuTextureLayer {
                            view: ts.view.clone(),
                            viewport,
                        });
                    }
                }
            }
        }

        // Set or clear composite layers on the renderer
        if !all_layers.is_empty() || !gpu_layers.is_empty() {
            renderer.set_composite_layers(all_layers);
            renderer.set_gpu_layers(gpu_layers);
        } else if renderer.has_composite_layers() {
            // Clear layers only when nothing is active (no video, no surfaces)
            #[cfg(feature = "video")]
            let video_loaded = rinch_video::is_video_loaded();
            #[cfg(not(feature = "video"))]
            let video_loaded = false;

            if !video_loaded && !crate::render_surface::any_surfaces_registered() {
                renderer.set_composite_layers(vec![]);
                renderer.set_gpu_layers(vec![]);
            }
        }

        // Build scene from document
        let scene = self.app.build_scene(scale, size);

        // Render to screen
        renderer.paint(scene, transparent)?;

        // Log paint time if RINCH_PERF is set
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
    fn process_actions(&mut self, actions: Vec<AppAction>, event_loop: &ActiveEventLoop) {
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
                        w.window.set_cursor(Self::cursor_style_to_winit(style));
                    }
                }
            }
        }
    }

    /// Convert a platform CursorStyle to a winit CursorIcon.
    fn cursor_style_to_winit(style: rinch_platform::CursorStyle) -> winit::window::CursorIcon {
        use rinch_platform::CursorStyle as CS;
        use winit::window::CursorIcon;
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
            meta: self.modifiers.super_key(),
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
                    } else if let Some(renderer) = &self.renderer {
                        match renderer.capture_screenshot() {
                            Ok((w, h, rgba)) => {
                                let png_bytes = screenshot::encode_png(&rgba, w, h);
                                DebugResult::Bytes {
                                    data: base64::engine::general_purpose::STANDARD
                                        .encode(&png_bytes),
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

impl ApplicationHandler<RinchNativeEvent> for RinchRuntime {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.create_window(event_loop);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: RinchNativeEvent) {
        // Drain main-thread callback queue on every user event (primarily ReRender).
        drain_main_queue();

        let platform_event = match event {
            RinchNativeEvent::ReRender => PlatformEvent::UserEvent(UserEvent::ReRender),
            RinchNativeEvent::SurfaceRedraw => {
                // Direct repaint — skip DOM resolution entirely.
                // The engine thread rendered new content to its GPU texture;
                // we just need the compositor to re-composite.
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                return;
            }
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

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let platform_event = match event {
            WindowEvent::CloseRequested => PlatformEvent::CloseRequested,
            WindowEvent::Resized(size) => {
                // Also resize the renderer
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
            WindowEvent::CursorMoved { position, .. } => PlatformEvent::MouseMove {
                x: position.x as f32,
                y: position.y as f32,
            },
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                let platform_button = match button {
                    MouseButton::Left => PlatformMouseButton::Left,
                    MouseButton::Right => PlatformMouseButton::Right,
                    MouseButton::Middle => PlatformMouseButton::Middle,
                    _ => return,
                };
                // For click handling, we need the cursor position
                let (x, y) = self.app.cursor_pos.unwrap_or((0.0, 0.0));

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
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button,
                ..
            } => {
                let platform_button = match button {
                    MouseButton::Left => PlatformMouseButton::Left,
                    MouseButton::Right => PlatformMouseButton::Right,
                    MouseButton::Middle => PlatformMouseButton::Middle,
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
            _ => return,
        };

        let size = self.window_size();
        let scale = self.scale_factor();
        let actions = self.app.handle_event(platform_event, size, scale);
        self.process_actions(actions, event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let size = self.window_size();
        let scale = self.scale_factor();
        let actions = self
            .app
            .handle_event(PlatformEvent::AboutToWait, size, scale);
        self.process_actions(actions, event_loop);
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
    let _ = tracing_subscriber::fmt::try_init();

    // Clear stale state
    events::clear_handlers();
    clear_context();

    let event_loop = EventLoop::<RinchNativeEvent>::with_user_event()
        .build()
        .expect("Failed to create event loop");

    let proxy = event_loop.create_proxy();

    // Set up signal change notification
    let render_proxy = proxy.clone();
    rinch_core::set_on_signal_change(move || {
        let _ = render_proxy.send_event(RinchNativeEvent::ReRender);
    });

    let mut runtime = RinchRuntime::new(title, width, height, component);
    runtime.proxy = Some(proxy.clone());

    // Install push-based menu event handler (covers native + tray menus)
    crate::menu::install_menu_event_handler();

    // Set native proxy for window control functions
    NATIVE_PROXY.with(|p| *p.borrow_mut() = Some(proxy.clone()));

    // Set global proxy so run_on_main_thread() works from any thread
    let _ = GLOBAL_PROXY.set(proxy.clone());

    // Start debug IPC server if feature is enabled (disable with RINCH_DEBUG=0)
    #[cfg(feature = "debug")]
    {
        if std::env::var("RINCH_DEBUG").map_or(true, |v| v != "0") {
            let debug_proxy = proxy.clone();
            match rinch_debug::attach(title, move || {
                let _ = debug_proxy.send_event(RinchNativeEvent::DebugCommand);
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

    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut runtime).expect("Event loop error");
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
    let _ = tracing_subscriber::fmt::try_init();

    events::clear_handlers();
    clear_context();

    let event_loop = EventLoop::<RinchNativeEvent>::with_user_event()
        .build()
        .expect("Failed to create event loop");

    let proxy = event_loop.create_proxy();

    let render_proxy = proxy.clone();
    rinch_core::set_on_signal_change(move || {
        let _ = render_proxy.send_event(RinchNativeEvent::ReRender);
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

    #[cfg(feature = "debug")]
    {
        if std::env::var("RINCH_DEBUG").map_or(true, |v| v != "0") {
            let debug_proxy = proxy.clone();
            match rinch_debug::attach(&props.title, move || {
                let _ = debug_proxy.send_event(RinchNativeEvent::DebugCommand);
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

    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut runtime).expect("Event loop error");
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
fn load_window_icon(png_data: &[u8]) -> Result<winit::window::Icon, Box<dyn std::error::Error>> {
    let (rgba, width, height) = decode_png_to_rgba(png_data)?;
    Ok(winit::window::Icon::from_rgba(rgba, width, height)?)
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
