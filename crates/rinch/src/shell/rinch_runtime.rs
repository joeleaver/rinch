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
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::events;
use rinch_core::hooks::clear_hooks;
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
        }
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
        }

        let window = event_loop
            .create_window(window_attrs)
            .expect("Failed to create window");
        let window = Arc::new(window);

        let size = window.inner_size();

        // Create GPU renderer
        let gpu = WgpuRenderer::new(&window, size.width.max(1), size.height.max(1));
        self.renderer = Some(gpu);

        // Wrap the winit window
        let winit_window = WinitWindow::new(window.clone());
        self.window = Some(winit_window);

        // Mount the component
        self.app
            .mount_component(size.width as f32, size.height as f32);

        // Request initial draw
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
                AppAction::DragWindow => {
                    if let Some(w) = &self.window {
                        let _ = w.drag_window();
                    }
                }
            }
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
    #[cfg(feature = "debug")]
    fn handle_debug_commands_with_renderer(&mut self) {
        let Some(rx) = self.app.debug_cmd_rx.take() else {
            return;
        };

        // Collect commands that need renderer access
        let mut pending = Vec::new();
        while let Ok(cmd) = rx.0.try_recv() {
            pending.push(cmd);
        }

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
        let platform_event = match event {
            RinchNativeEvent::ReRender => PlatformEvent::UserEvent(UserEvent::ReRender),
            #[cfg(feature = "debug")]
            RinchNativeEvent::DebugCommand => {
                // Debug commands may need the renderer (screenshots), so
                // we handle them here in the shell instead of delegating.
                self.handle_debug_commands_with_renderer();
                return;
            }
            RinchNativeEvent::MinimizeWindow => PlatformEvent::UserEvent(UserEvent::MinimizeWindow),
            RinchNativeEvent::ToggleMaximizeWindow => {
                PlatformEvent::UserEvent(UserEvent::ToggleMaximizeWindow)
            }
            RinchNativeEvent::CloseWindowControl => {
                PlatformEvent::UserEvent(UserEvent::CloseWindow)
            }
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
                let platform_key = Self::translate_key(key_code);
                let mods = self.translate_modifiers();
                PlatformEvent::KeyDown {
                    key: platform_key,
                    text: text.as_ref().map(|t| t.to_string()),
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
/// (Taffy + Parley + Vello) instead of blitz for layout and rendering.
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
///
/// fn app(__scope: &mut RenderScope) -> NodeHandle {
///     let count = use_signal(|| 0);
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
    clear_hooks();

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

    // Set native proxy for window control functions
    NATIVE_PROXY.with(|p| *p.borrow_mut() = Some(proxy.clone()));

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
    let _ = tracing_subscriber::fmt::try_init();

    events::clear_handlers();
    clear_hooks();

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

    // Set native proxy for window control functions
    NATIVE_PROXY.with(|p| *p.borrow_mut() = Some(proxy.clone()));

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
