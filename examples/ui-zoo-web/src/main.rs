//! UI Zoo WASM entry point.
//!
//! This is the web runtime for the UI Zoo widget showcase. It mirrors the desktop
//! runtime's architecture (winit event loop + wgpu renderer + vello) but uses
//! WASM-specific async GPU initialization and `spawn_app()` for the non-blocking
//! web event loop.
//!
//! The app shell uses a sidebar layout with navigation links and theme controls
//! from the shared `ui_zoo` library.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use peniko::Color;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use rinch::app::RinchApp;
use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use rinch_core::events;
use rinch_core::hooks::clear_hooks;
use rinch_platform::{
    AppAction, KeyCode, Modifiers, MouseButton as PlatformMouseButton, PlatformEvent, UserEvent,
};
use ui_zoo::{
    init_all_sections, nav_links, overlays_demo_overlays, section_content, theme_controls,
};
use vello::{AaConfig, AaSupport, RenderParams, Renderer as VelloRenderer, RendererOptions};
use wgpu::{
    CommandEncoderDescriptor, Extent3d, MemoryHints, Texture, TextureDescriptor, TextureDimension,
    TextureFormat, TextureUsages,
};

/// Global CSS for the web app layout with sidebar.
const CSS_WEB: &str = r#"
* {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
}

html, body {
    height: 100vh;
    font-family: var(--rinch-font-family);
    background: var(--rinch-color-body);
    color: var(--rinch-color-text);
    overflow: hidden;
}

.app-shell {
    display: flex;
    height: 100vh;
}

.sidebar {
    width: 220px;
    min-width: 220px;
    padding: var(--rinch-spacing-md);
    border-right: 1px solid var(--rinch-color-gray-3);
    background: var(--rinch-color-body);
    overflow-y: auto;
}

.sidebar-header {
    padding: var(--rinch-spacing-sm) var(--rinch-spacing-xs);
}

.main-content {
    flex: 1;
    padding: var(--rinch-spacing-xl);
    overflow-y: auto;
}
"#;

/// The web app shell: sidebar navigation layout.
fn app(__scope: &mut RenderScope) -> NodeHandle {
    let current_section = use_signal(|| 0_usize);
    let primary_color = use_signal(|| "blue");
    let dark_mode = use_signal(|| false);

    init_all_sections();

    let nav = move |idx: usize| {
        move || {
            current_section.set(idx);
        }
    };

    rsx! {
        ThemeProvider {
            primary_color_fn: Rc::new(move || primary_color.get()),
            dark_mode_fn: Rc::new(move || dark_mode.get()),

            style { {CSS_WEB} }

            div { class: "app-shell",
                // Sidebar with navigation
                div { class: "sidebar",
                    div { class: "sidebar-header",
                        Title { order: 3, "UI Zoo" }
                        Text { size: "xs", color: "dimmed", "Rinch Widget Showcase" }
                    }
                    Space { h: "md" }
                    {nav_links(__scope, current_section, nav)}
                    Space { h: "xl" }
                    {theme_controls(__scope, primary_color, dark_mode)}
                }

                // Main content area
                div { class: "main-content",
                    {section_content(__scope, current_section)}
                }
            }

            // Overlays section demo components
            {overlays_demo_overlays(__scope)}
        }
    }
}

// ── Custom event ─────────────────────────────────────────────────────────────

/// Custom event for signaling re-renders from signal changes.
#[derive(Debug, Clone)]
enum WebEvent {
    /// A signal changed; re-resolve layout and repaint.
    ReRender,
}

// ── GPU state ────────────────────────────────────────────────────────────────

/// GPU state initialized asynchronously on WASM.
struct GpuState {
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: VelloRenderer,
    render_texture: Texture,
}

impl GpuState {
    fn create_render_texture(
        device: &wgpu::Device,
        format: TextureFormat,
        width: u32,
        height: u32,
    ) -> Texture {
        device.create_texture(&TextureDescriptor {
            label: Some("rinch web render texture"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::STORAGE_BINDING
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
        self.render_texture = Self::create_render_texture(
            &self.device,
            self.surface_config.format,
            width,
            height,
        );
    }
}

// ── WebRuntime ───────────────────────────────────────────────────────────────

/// Web runtime: thin winit `ApplicationHandler` that delegates to
/// [`RinchApp`] for platform-agnostic logic.
struct WebRuntime {
    /// Platform-agnostic application logic.
    rinch_app: RinchApp,
    /// The winit window (wraps a canvas element).
    window: Option<Arc<Window>>,
    /// GPU state (initialized asynchronously after window creation).
    gpu: Rc<RefCell<Option<GpuState>>>,
    /// Current cursor position (tracked locally since RinchApp.cursor_pos is crate-private).
    cursor_pos: (f32, f32),
    /// Current keyboard modifier state.
    modifiers: winit::keyboard::ModifiersState,
}

impl WebRuntime {
    fn new() -> Self {
        // Set up theme before creating the app
        let theme = ThemeProviderProps {
            primary_color: Some("blue".into()),
            default_radius: Some("md".into()),
            dark_mode: false,
            ..Default::default()
        };
        rinch::setup_theme_css(&theme);

        Self {
            rinch_app: RinchApp::new(app),
            window: None,
            gpu: Rc::new(RefCell::new(None)),
            cursor_pos: (0.0, 0.0),
            modifiers: winit::keyboard::ModifiersState::empty(),
        }
    }

    fn window_size(&self) -> (u32, u32) {
        self.window
            .as_ref()
            .map(|w| {
                let s = w.inner_size();
                (s.width.max(1), s.height.max(1))
            })
            .unwrap_or((1200, 800))
    }

    fn scale_factor(&self) -> f64 {
        self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0)
    }

    /// Paint the current scene to the canvas via GPU.
    fn paint(&mut self) {
        // Early exit if GPU not ready
        if self.gpu.borrow().is_none() {
            return;
        }

        let Some(window) = &self.window else { return };
        let scale = window.scale_factor();
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        // Build scene from document (borrows self.rinch_app)
        let scene = self.rinch_app.build_scene(scale, (width, height));

        // Now borrow GPU state (independent Rc<RefCell>)
        let mut gpu_ref = self.gpu.borrow_mut();
        let Some(gpu) = gpu_ref.as_mut() else { return };

        // Resize if needed
        if gpu.surface_config.width != width || gpu.surface_config.height != height {
            gpu.resize(width, height);
        }

        let render_texture_view = gpu
            .render_texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Render to intermediate texture (vello uses compute shaders)
        if gpu
            .renderer
            .render_to_texture(
                &gpu.device,
                &gpu.queue,
                scene,
                &render_texture_view,
                &RenderParams {
                    base_color: Color::WHITE,
                    width,
                    height,
                    antialiasing_method: AaConfig::Area,
                },
            )
            .is_err()
        {
            return;
        }

        // Copy to surface
        let surface_texture = match gpu.surface.get_current_texture() {
            Ok(t) => t,
            Err(_) => return,
        };

        let mut encoder = gpu
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("rinch web copy encoder"),
            });

        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &gpu.render_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &surface_texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            Extent3d {
                width: gpu.surface_config.width,
                height: gpu.surface_config.height,
                depth_or_array_layers: 1,
            },
        );

        gpu.queue.submit(Some(encoder.finish()));
        surface_texture.present();
    }

    /// Process actions returned by RinchApp.
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
                // Minimize/maximize/drag are not applicable on web
                _ => {}
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
            WK::Enter => KeyCode::Enter,
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
}

// ── ApplicationHandler impl ──────────────────────────────────────────────────

impl ApplicationHandler<WebEvent> for WebRuntime {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        // Get canvas element from DOM
        let document = web_sys::window().unwrap().document().unwrap();
        let canvas = document
            .get_element_by_id("rinch-canvas")
            .expect("Canvas element #rinch-canvas not found");
        let canvas: web_sys::HtmlCanvasElement = canvas.dyn_into().unwrap();

        // Create winit window backed by the canvas
        use winit::platform::web::WindowAttributesExtWebSys;
        let attrs = Window::default_attributes()
            .with_canvas(Some(canvas))
            .with_prevent_default(true);
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("Failed to create window"),
        );

        self.window = Some(window.clone());

        // Mount the component (builds DOM tree, doesn't need GPU)
        let size = window.inner_size();
        self.rinch_app
            .mount_component(size.width.max(1) as f32, size.height.max(1) as f32);

        // Spawn async GPU initialization
        let gpu = self.gpu.clone();
        let window_for_init = window.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let instance = wgpu::Instance::default();
            let surface = instance
                .create_surface(window_for_init.clone())
                .expect("Failed to create surface");

            // Try hardware adapter first, then fall back to software rendering
            let adapter = match instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                })
                .await
            {
                Ok(a) => a,
                Err(e) => {
                    log::warn!("No hardware WebGPU adapter found ({e}), trying fallback");
                    instance
                        .request_adapter(&wgpu::RequestAdapterOptions {
                            power_preference: wgpu::PowerPreference::LowPower,
                            compatible_surface: Some(&surface),
                            force_fallback_adapter: true,
                        })
                        .await
                        .expect("No WebGPU adapter found (tried hardware and fallback). Check chrome://gpu for WebGPU support.")
                }
            };

            let caps = surface.get_capabilities(&adapter);

            // Prefer Rgba8Unorm (guaranteed STORAGE_BINDING in WebGPU core)
            let format = if caps.formats.contains(&TextureFormat::Rgba8Unorm) {
                TextureFormat::Rgba8Unorm
            } else if caps.formats.contains(&TextureFormat::Bgra8Unorm) {
                TextureFormat::Bgra8Unorm
            } else {
                caps.formats[0]
            };

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("rinch web device"),
                    required_features: wgpu::Features::default(),
                    required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                        .using_resolution(adapter.limits()),
                    memory_hints: MemoryHints::MemoryUsage,
                    trace: wgpu::Trace::default(),
                    experimental_features: wgpu::ExperimentalFeatures::default(),
                })
                .await
                .expect("Failed to create device");

            let size = window_for_init.inner_size();
            let width = size.width.max(1);
            let height = size.height.max(1);

            let surface_config = wgpu::SurfaceConfiguration {
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_DST,
                format,
                width,
                height,
                present_mode: wgpu::PresentMode::AutoVsync,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![],
            };
            surface.configure(&device, &surface_config);

            let render_texture =
                GpuState::create_render_texture(&device, format, width, height);

            let renderer = VelloRenderer::new(
                &device,
                RendererOptions {
                    antialiasing_support: AaSupport::all(),
                    use_cpu: false,
                    num_init_threads: None,
                    pipeline_cache: None,
                },
            )
            .expect("Failed to create Vello renderer");

            log::info!("WebGPU initialized: format={:?}", format);

            *gpu.borrow_mut() = Some(GpuState {
                surface,
                surface_config,
                device,
                queue,
                renderer,
                render_texture,
            });

            // Trigger first paint
            window_for_init.request_redraw();
        });
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: WebEvent) {
        match event {
            WebEvent::ReRender => {
                let size = self.window_size();
                let scale = self.scale_factor();
                let actions = self.rinch_app.handle_event(
                    PlatformEvent::UserEvent(UserEvent::ReRender),
                    size,
                    scale,
                );
                self.process_actions(actions, event_loop);
            }
        }
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
                if let Some(gpu) = self.gpu.borrow_mut().as_mut() {
                    gpu.resize(size.width.max(1), size.height.max(1));
                }
                PlatformEvent::Resized {
                    width: size.width,
                    height: size.height,
                }
            }
            WindowEvent::RedrawRequested => {
                self.paint();
                return;
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = (position.x as f32, position.y as f32);
                PlatformEvent::MouseMove {
                    x: position.x as f32,
                    y: position.y as f32,
                }
            }
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
                let (x, y) = self.cursor_pos;
                let size = self.window_size();
                let scale = self.scale_factor();
                let actions = self.rinch_app.handle_event(
                    PlatformEvent::MouseDown {
                        x,
                        y,
                        button: platform_button,
                    },
                    size,
                    scale,
                );
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
                let (x, y) = self.cursor_pos;
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
                let (cx, cy) = self.cursor_pos;
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
        let actions = self.rinch_app.handle_event(platform_event, size, scale);
        self.process_actions(actions, event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let size = self.window_size();
        let scale = self.scale_factor();
        let actions = self
            .rinch_app
            .handle_event(PlatformEvent::AboutToWait, size, scale);
        self.process_actions(actions, event_loop);
    }
}

// ── Entry point ──────────────────────────────────────────────────────────────

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Info).ok();

    // Clear stale state
    events::clear_handlers();
    clear_hooks();

    let event_loop = EventLoop::<WebEvent>::with_user_event()
        .build()
        .expect("Failed to create event loop");

    let proxy = event_loop.create_proxy();

    // Set up signal change notification
    let render_proxy = proxy.clone();
    rinch_core::set_on_signal_change(move || {
        let _ = render_proxy.send_event(WebEvent::ReRender);
    });

    let runtime = WebRuntime::new();

    // On WASM, spawn_app is non-blocking (returns immediately)
    use winit::platform::web::EventLoopExtWebSys;
    event_loop.spawn_app(runtime);
}

fn main() {
    // Entry point is `start()` via #[wasm_bindgen(start)].
    // This empty main is required for the binary crate.
}
