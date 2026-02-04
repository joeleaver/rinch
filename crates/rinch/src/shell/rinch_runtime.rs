//! Rinch-native runtime using rinch-dom for rendering.
//!
//! This is a parallel runtime that replaces the blitz-based `fine_grained_runtime`
//! with a direct Taffy + Parley + Vello pipeline via rinch-dom. The existing
//! blitz-based runtime continues to work unchanged.
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
use std::rc::Rc;
use std::sync::Arc;

use peniko::Color;
use vello::{AaConfig, AaSupport, RenderParams, Renderer as VelloRenderer, RendererOptions, Scene};
use wgpu::{
    Backends, CommandEncoderDescriptor, Device, Extent3d, Instance, InstanceDescriptor, Limits,
    MemoryHints, PresentMode, Queue, Surface, SurfaceConfiguration, Texture, TextureDescriptor,
    TextureDimension, TextureFormat, TextureUsages,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use rinch_core::dom::{clear_render_scope, set_render_scope, DomDocument, NodeHandle, RenderScope};
use rinch_core::events;
use rinch_core::hooks::{begin_render, clear_hooks, end_render};
use rinch_dom::RinchDocument;
use rinch_dom::text_query::{byte_offset_from_position, caret_position_for_offset_layout, glyph_bounds_for_offset_layout};

use super::devtools::DevToolsState;

#[cfg(feature = "debug")]
use {
    serde_json::json,
    super::screenshot,
    rinch_debug::{CommandReceiver, DebugCommandKind, DebugResult},
};

// Thread-local proxy for the native event loop, used by window control functions.
thread_local! {
    pub(crate) static NATIVE_PROXY: RefCell<Option<EventLoopProxy<RinchNativeEvent>>> = RefCell::new(None);
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

/// GPU state for Vello rendering.
struct RenderState {
    renderer: VelloRenderer,
    surface: Surface<'static>,
    surface_config: SurfaceConfiguration,
    device: Device,
    queue: Queue,
    render_texture: Texture,
}

/// State for an active scrollbar drag operation.
struct ScrollbarDrag {
    /// The node ID of the scroll container being scrolled.
    node_id: usize,
    /// The Y coordinate where the drag started (screen pixels).
    start_y: f32,
    /// The scroll_offset.1 value when the drag started.
    start_scroll: f64,
    /// Content height of the scroll container (for ratio calculation).
    content_height: f64,
    /// Container height of the scroll container.
    container_height: f64,
}

/// The main application runtime using rinch-dom.
pub struct RinchRuntime {
    /// Component function to render.
    component: Option<Box<dyn FnOnce(&mut RenderScope) -> NodeHandle>>,
    /// Window title.
    title: String,
    width: u32,
    height: u32,
    /// The winit window.
    window: Option<Arc<Window>>,
    /// GPU render state.
    render_state: Option<RenderState>,
    /// The document (shared with RenderScope).
    doc: Option<Rc<RefCell<RinchDocument>>>,
    /// Render scope (kept alive for effects).
    _render_scope: Option<Rc<RefCell<RenderScope>>>,
    /// Event loop proxy for sending events.
    proxy: Option<EventLoopProxy<RinchNativeEvent>>,
    /// Vello scene (reused across frames).
    scene: Scene,
    /// Parley layout context for paint-time text layout (uses Brush, not [u8; 4]).
    paint_layout_cx: parley::LayoutContext<peniko::Brush>,
    /// Current cursor position.
    cursor_pos: Option<(f32, f32)>,
    /// Active scrollbar drag state.
    scrollbar_drag: Option<ScrollbarDrag>,
    /// Debug command receiver.
    #[cfg(feature = "debug")]
    debug_cmd_rx: Option<CommandReceiver>,
    /// Debug server handle (kept alive).
    #[cfg(feature = "debug")]
    _debug_server: Option<rinch_debug::DebugServer>,
    /// Window properties for configuring borderless, transparent, etc.
    window_props: Option<rinch_core::element::WindowProps>,
    /// DevTools state.
    devtools: DevToolsState,
    /// Last theme CSS loaded into the document (for change detection).
    last_theme_css: Option<String>,
    /// Currently focused input element (node ID).
    focused_input: Option<usize>,
    /// Cursor position within the focused input (byte offset).
    input_cursor: usize,
    /// Selection start position (byte offset). If equal to input_cursor, no selection.
    input_selection_start: usize,
    /// Last time the cursor blink state changed.
    cursor_blink_time: std::time::Instant,
    /// Whether the cursor is currently visible (for blinking).
    cursor_visible: bool,
    /// Current keyboard modifier state (Shift, Ctrl, Alt).
    modifiers: winit::keyboard::ModifiersState,
    /// Whether mouse is being pressed for text selection drag.
    input_mouse_drag: bool,
    /// Timestamp of last mouse click (for multi-click detection).
    last_click_time: std::time::Instant,
    /// Position of last mouse click.
    last_click_pos: (f32, f32),
    /// Current click count (1 = single, 2 = double, 3 = triple).
    click_count: u8,
    /// Font context for hit testing (reused across frames).
    hit_test_font_cx: parley::FontContext,
}

impl RinchRuntime {
    fn new(
        title: &str,
        width: u32,
        height: u32,
        component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static,
    ) -> Self {
        Self {
            component: Some(Box::new(component)),
            title: title.to_string(),
            width,
            height,
            window: None,
            render_state: None,
            doc: None,
            _render_scope: None,
            proxy: None,
            scene: Scene::new(),
            paint_layout_cx: parley::LayoutContext::new(),
            cursor_pos: None,
            scrollbar_drag: None,
            #[cfg(feature = "debug")]
            debug_cmd_rx: None,
            #[cfg(feature = "debug")]
            _debug_server: None,
            window_props: None,
            devtools: DevToolsState::new(),
            last_theme_css: None,
            focused_input: None,
            input_cursor: 0,
            input_selection_start: 0,
            cursor_blink_time: std::time::Instant::now(),
            cursor_visible: true,
            modifiers: winit::keyboard::ModifiersState::empty(),
            input_mouse_drag: false,
            last_click_time: std::time::Instant::now(),
            last_click_pos: (0.0, 0.0),
            click_count: 0,
            hit_test_font_cx: parley::FontContext::new(),
        }
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) {
        let mut window_attrs = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(winit::dpi::LogicalSize::new(self.width, self.height));

        // Apply window props if set
        if let Some(props) = &self.window_props {
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
                window_attrs = window_attrs.with_window_level(winit::window::WindowLevel::AlwaysOnTop);
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

        // Create GPU state
        let state = self.create_render_state(&window, size.width.max(1), size.height.max(1));
        self.render_state = Some(state);
        self.window = Some(window.clone());

        // Create RinchDocument
        let doc = Rc::new(RefCell::new(RinchDocument::new()));

        // Load theme + widget CSS into the document's stylesheet
        {
            let mut d = doc.borrow_mut();
            #[cfg(feature = "theme")]
            {
                let theme_css = rinch_core::get_current_theme_css().unwrap_or_default();
                if !theme_css.is_empty() {
                    d.load_css(&theme_css);
                }
            }
            // Set viewport size so vh/vw units resolve correctly during DOM construction
            d.set_viewport(size.width as f32, size.height as f32);
        }
        // Remember the initial theme CSS so we can detect changes later
        #[cfg(feature = "theme")]
        {
            self.last_theme_css = Some(rinch_core::get_current_theme_css().unwrap_or_default());
        }

        // Create RenderScope
        let doc_as_dom: Rc<RefCell<dyn DomDocument>> = doc.clone();
        let body_id = doc.borrow().body();
        let scope = Rc::new(RefCell::new(RenderScope::new(doc_as_dom, body_id)));

        // Set thread-local context
        set_render_scope(scope.clone());
        begin_render();

        // Run the component
        let component = self.component.take().expect("component already consumed");
        let root = {
            let mut scope_ref = scope.borrow_mut();
            component(&mut scope_ref)
        };

        // Append root to body
        doc.borrow_mut().append_child(body_id, root.node_id());

        end_render();
        clear_render_scope();

        // Initial layout
        {
            let mut d = doc.borrow_mut();
            d.resolve_layout(size.width as f32, size.height as f32);
            let _ = d.take_dirty_nodes();
        }

        // Request initial draw
        window.request_redraw();

        self.doc = Some(doc);
        self._render_scope = Some(scope);
    }

    fn create_render_texture(
        device: &Device,
        format: TextureFormat,
        width: u32,
        height: u32,
    ) -> Texture {
        device.create_texture(&TextureDescriptor {
            label: Some("rinch-dom render texture"),
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

    fn create_render_state(&self, window: &Arc<Window>, width: u32, height: u32) -> RenderState {
        let backends = Backends::from_env().unwrap_or_default();
        let instance = Instance::new(&InstanceDescriptor {
            backends,
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        });

        let surface = instance
            .create_surface(window.clone())
            .expect("Failed to create surface");

        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            }))
            .expect("Failed to find adapter");

        let caps = surface.get_capabilities(&adapter);

        let format = if caps.formats.contains(&TextureFormat::Rgba8Unorm) {
            TextureFormat::Rgba8Unorm
        } else if caps.formats.contains(&TextureFormat::Bgra8Unorm) {
            TextureFormat::Bgra8Unorm
        } else {
            caps.formats[0]
        };

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("rinch-dom device"),
                required_features: wgpu::Features::default(),
                required_limits: Limits::default(),
                memory_hints: MemoryHints::MemoryUsage,
                trace: wgpu::Trace::default(),
                experimental_features: wgpu::ExperimentalFeatures::default(),
            },
        ))
        .expect("Failed to create device");

        let surface_config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_DST,
            format,
            width,
            height,
            present_mode: PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
        };

        surface.configure(&device, &surface_config);

        let render_texture = Self::create_render_texture(&device, format, width, height);

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

        tracing::info!(
            "rinch-dom runtime: backend={:?}, format={:?}",
            adapter.get_info().backend,
            format
        );

        RenderState {
            renderer,
            surface,
            surface_config,
            device,
            queue,
            render_texture,
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);

        if let Some(state) = &mut self.render_state {
            state.surface_config.width = width;
            state.surface_config.height = height;
            state.surface.configure(&state.device, &state.surface_config);
            state.render_texture = Self::create_render_texture(
                &state.device,
                state.surface_config.format,
                width,
                height,
            );
        }

        if let Some(doc) = &self.doc {
            let mut d = doc.borrow_mut();
            d.resolve_layout(width as f32, height as f32);
            let _ = d.take_dirty_nodes();
        }
    }

    fn resolve_and_repaint(&mut self) {
        let frame_start = std::time::Instant::now();
        if let (Some(window), Some(doc)) = (&self.window, &self.doc) {
            // Check if theme CSS has changed (e.g. primary color or dark mode toggled)
            #[cfg(feature = "theme")]
            {
                let current_theme = rinch_core::get_current_theme_css().unwrap_or_default();
                let theme_changed = self.last_theme_css.as_deref() != Some(current_theme.as_str());

                if theme_changed {
                    self.last_theme_css = Some(current_theme.clone());
                    if !current_theme.is_empty() {
                        let mut d = doc.borrow_mut();
                        d.update_theme_variables(&current_theme);
                        d.recompute_all_styles_full();
                    }
                }
            }

            // Resolve layout
            let size = window.inner_size();
            {
                let mut d = doc.borrow_mut();
                let _ = d.take_dirty_nodes();
                d.resolve_layout(size.width as f32, size.height as f32);
            }

            window.request_redraw();
        }
        // Log frame time if RINCH_PERF is set
        if std::env::var("RINCH_PERF").is_ok() {
            let elapsed = frame_start.elapsed();
            let fps = 1.0 / elapsed.as_secs_f64();
            eprintln!("[PERF] resolve: {:.2}ms ({:.0} fps)", elapsed.as_secs_f64() * 1000.0, fps);
        }
    }

    fn paint(&mut self) -> Result<(), String> {
        let paint_start = std::time::Instant::now();
        let Some(state) = &mut self.render_state else {
            return Ok(());
        };
        let Some(doc) = &self.doc else {
            return Ok(());
        };
        let Some(window) = &self.window else {
            return Ok(());
        };

        let surface_texture = match state.surface.get_current_texture() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Failed to get surface texture: {:?}", e);
                return Ok(());
            }
        };

        let render_texture_view = state
            .render_texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Build scene from document
        self.scene.reset();
        let scale = window.scale_factor();
        let size = window.inner_size();
        {
            let mut d = doc.borrow_mut();
            let d = &mut *d; // reborrow as &mut RinchDocument
            rinch_dom::paint::paint_document(
                &d.tree,
                &mut self.scene,
                scale,
                (size.width as f32, size.height as f32),
                &mut d.font_cx,
                &mut d.layout_cx,  // Use same LayoutContext as measurement
            );
        }

        // Render to intermediate texture
        state
            .renderer
            .render_to_texture(
                &state.device,
                &state.queue,
                &self.scene,
                &render_texture_view,
                &RenderParams {
                    base_color: if self.window_props.as_ref().is_some_and(|p| p.transparent) {
                        Color::TRANSPARENT
                    } else {
                        Color::WHITE
                    },
                    width: state.surface_config.width,
                    height: state.surface_config.height,
                    antialiasing_method: AaConfig::Msaa16,
                },
            )
            .map_err(|e| format!("Failed to render to texture: {:?}", e))?;

        // Copy to surface
        let mut encoder = state
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("rinch-dom copy encoder"),
            });

        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &state.render_texture,
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
                width: state.surface_config.width,
                height: state.surface_config.height,
                depth_or_array_layers: 1,
            },
        );

        state.queue.submit(Some(encoder.finish()));
        surface_texture.present();
        state
            .device
            .poll(wgpu::PollType::Poll)
            .map_err(|e| format!("GPU poll failed: {:?}", e))?;

        // Log paint time if RINCH_PERF is set
        if std::env::var("RINCH_PERF").is_ok() {
            let elapsed = paint_start.elapsed();
            eprintln!("[PERF] paint: {:.2}ms", elapsed.as_secs_f64() * 1000.0);
        }

        Ok(())
    }

    #[cfg(feature = "debug")]
    fn handle_debug_commands(&mut self) {
        let Some(rx) = self.debug_cmd_rx.take() else { return };

        while let Ok(cmd) = rx.0.try_recv() {
            let response = self.execute_debug_command(cmd.kind);
            let _ = cmd.response_tx.send(response);
        }

        self.debug_cmd_rx = Some(rx);
    }

    #[cfg(feature = "debug")]
    fn execute_debug_command(&mut self, kind: DebugCommandKind) -> DebugResult {
        match kind {
            DebugCommandKind::Screenshot => {
                if let Err(e) = self.paint() {
                    return DebugResult::Error { message: format!("Paint failed: {}", e) };
                }
                let Some(state) = &self.render_state else {
                    return DebugResult::Error { message: "No render state".into() };
                };
                let w = state.surface_config.width;
                let h = state.surface_config.height;
                let fmt = state.surface_config.format;
                let rgba = match screenshot::capture_texture_rgba(
                    &state.device, &state.queue, &state.render_texture, w, h, fmt,
                ) {
                    Ok(data) => data,
                    Err(e) => return DebugResult::Error { message: format!("Screenshot capture failed: {}", e) },
                };
                let png_bytes = screenshot::encode_png(&rgba, w, h);
                use base64::Engine;
                DebugResult::Bytes { data: base64::engine::general_purpose::STANDARD.encode(&png_bytes) }
            }
            DebugCommandKind::DomTree => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error { message: "No document".into() };
                };
                let d = doc.borrow();
                DebugResult::Json { data: rinch_dom::testing::serialize_tree(&d.tree) }
            }
            DebugCommandKind::QuerySelector { selector } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error { message: "No document".into() };
                };
                let d = doc.borrow();
                let ids = rinch_dom::testing::query_selector(&d.tree, &selector);
                let nodes: Vec<_> = ids.iter()
                    .filter_map(|&id| rinch_dom::testing::get_node_detail(&d.tree, id))
                    .collect();
                DebugResult::Json { data: json!(nodes) }
            }
            DebugCommandKind::GetNode { id } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error { message: "No document".into() };
                };
                let d = doc.borrow();
                match rinch_dom::testing::get_node_detail(&d.tree, id) {
                    Some(detail) => DebugResult::Json { data: detail },
                    None => DebugResult::Error { message: format!("Node {} not found", id) },
                }
            }
            DebugCommandKind::GetTextContent { id } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error { message: "No document".into() };
                };
                let d = doc.borrow();
                DebugResult::Json { data: json!(rinch_dom::testing::get_text_content(&d.tree, id)) }
            }
            DebugCommandKind::Click { x, y } => {
                self.handle_click(x, y);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::MouseMove { x, y } => {
                self.cursor_pos = Some((x, y));
                // Update hover state for CSS :hover support
                if let Some(doc) = &self.doc {
                    let hovered = {
                        let d = doc.borrow();
                        hit_test(&d.tree, x, y)
                    };
                    let changed = doc.borrow_mut().update_hover(hovered);
                    if changed {
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                }
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::Scroll { x, y, delta_x: _delta_x, delta_y } => {
                self.cursor_pos = Some((x, y));

                if let Some(doc) = &self.doc {
                    let hit_node = hit_test(&doc.borrow().tree, x, y);
                    if let Some(hit_node) = hit_node {
                        let mut doc = doc.borrow_mut();
                        if let Some(scroll_node_id) = find_scroll_container(&doc.tree, hit_node) {
                            let content_height = compute_content_height(&doc.tree, scroll_node_id);
                            let container_height = doc.tree.get(scroll_node_id)
                                .map(|n| n.layout.height as f64)
                                .unwrap_or(0.0);
                            let max_scroll = (content_height - container_height).max(0.0);

                            if let Some(node) = doc.tree.nodes.get_mut(scroll_node_id) {
                                let new_y = (node.scroll_offset.1 + delta_y).clamp(0.0, max_scroll);
                                if new_y != node.scroll_offset.1 {
                                    node.scroll_offset.1 = new_y;
                                    node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                                    doc.tree.dirty_nodes.insert(scroll_node_id);
                                }
                            }
                        }
                        drop(doc);
                    }
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::TypeText { text } => {
                // Inject keyboard input into the focused input element
                self.handle_text_input(&text);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::WaitFrame => {
                self.resolve_and_repaint();
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::GetComputedStyles { id } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error { message: "No document".into() };
                };
                let d = doc.borrow();
                match d.tree.get(id) {
                    Some(node) => {
                        DebugResult::Json { data: json!(&node.computed_style) }
                    }
                    None => DebugResult::Error { message: format!("Node {} not found", id) },
                }
            }
            DebugCommandKind::CloseApp => {
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    std::process::exit(0);
                });
                DebugResult::Json { data: json!({"status": "closing"}) }
            }
            DebugCommandKind::KeyPress { key, shift, ctrl } => {
                // First, try to dispatch to the keyboard interceptor (used by rich text editor)
                let key_data = events::KeyEventData {
                    key: key.clone(),
                    code: key.clone(), // Use same as key for debug commands
                    ctrl,
                    shift,
                    alt: false,
                    meta: false,
                };
                let handled = events::dispatch_keyboard_event(&key_data);

                // If not handled by interceptor, fall back to default handling
                if !handled {
                    match key.as_str() {
                        "ArrowUp" => self.handle_arrow_up(shift),
                        "ArrowDown" => self.handle_arrow_down(shift),
                        "ArrowLeft" => self.handle_arrow_left(shift, ctrl),
                        "ArrowRight" => self.handle_arrow_right(shift, ctrl),
                        "Home" => self.handle_home(shift),
                        "End" => self.handle_end(shift),
                        "Enter" => self.handle_enter(),
                        "Backspace" => self.handle_backspace(),
                        "Delete" => self.handle_delete(),
                        _ => {}
                    }
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::GetCaretPosition { node_id, byte_offset } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error { message: "No document".into() };
                };

                let scale = self.window.as_ref()
                    .map(|w| w.scale_factor() as f32)
                    .unwrap_or(1.0);

                let d = doc.borrow();
                let Some(node) = d.tree.get(node_id) else {
                    return DebugResult::Error { message: format!("Node {} not found", node_id) };
                };

                // Calculate absolute position by walking up the tree
                let mut abs_x = node.layout.x as f64;
                let mut abs_y = node.layout.y as f64;
                let mut parent_id = node.parent;
                while let Some(pid) = parent_id {
                    if let Some(parent_node) = d.tree.get(pid) {
                        abs_x += parent_node.layout.x as f64;
                        abs_y += parent_node.layout.y as f64;
                        abs_x -= parent_node.scroll_offset.0;
                        abs_y -= parent_node.scroll_offset.1;
                        parent_id = parent_node.parent;
                    } else {
                        break;
                    }
                }

                // Check if this is an input/textarea - build layout from value
                let tag = node.tag();
                if matches!(tag, Some("input" | "textarea")) {
                    let value = node.attributes.get("value").cloned().unwrap_or_default();
                    if value.is_empty() {
                        let padding_left = node.computed_style.padding_left.to_px() as f64 * scale as f64;
                        let padding_top = node.computed_style.padding_top.to_px() as f64 * scale as f64;
                        return DebugResult::Json { data: json!({
                            "x": abs_x + padding_left,
                            "y": abs_y + padding_top,
                        })};
                    }

                    let computed_style = node.computed_style.clone();
                    let input_width = node.layout.width;
                    drop(d);

                    let layout = computed_style.build_parley_layout(
                        &value,
                        scale,
                        &mut self.hit_test_font_cx,
                        &mut self.paint_layout_cx,
                        Some(input_width),
                    );

                    let (x, y) = caret_position_for_offset_layout(&layout, byte_offset);
                    let padding_left = computed_style.padding_left.to_px() as f64 * scale as f64;
                    let padding_top = computed_style.padding_top.to_px() as f64 * scale as f64;

                    return DebugResult::Json { data: json!({
                        "x": abs_x + padding_left + x as f64,
                        "y": abs_y + padding_top + y as f64,
                    })};
                }

                // Check if node has inline text layout (IFC text)
                if let Some(ref inline_layout) = node.text_layout {
                    let (x, y) = caret_position_for_offset_layout(&inline_layout.layout, byte_offset);
                    return DebugResult::Json { data: json!({
                        "x": abs_x + x as f64,
                        "y": abs_y + y as f64,
                    })};
                }

                DebugResult::Error { message: "Node does not have text layout".into() }
            }
            DebugCommandKind::GetGlyphBounds { node_id, byte_offset } => {
                let Some(doc) = &self.doc else {
                    return DebugResult::Error { message: "No document".into() };
                };

                let scale = self.window.as_ref()
                    .map(|w| w.scale_factor() as f32)
                    .unwrap_or(1.0);

                let d = doc.borrow();
                let Some(node) = d.tree.get(node_id) else {
                    return DebugResult::Error { message: format!("Node {} not found", node_id) };
                };

                // Calculate absolute position by walking up the tree
                let mut abs_x = node.layout.x as f64;
                let mut abs_y = node.layout.y as f64;
                let mut parent_id = node.parent;
                while let Some(pid) = parent_id {
                    if let Some(parent_node) = d.tree.get(pid) {
                        abs_x += parent_node.layout.x as f64;
                        abs_y += parent_node.layout.y as f64;
                        abs_x -= parent_node.scroll_offset.0;
                        abs_y -= parent_node.scroll_offset.1;
                        parent_id = parent_node.parent;
                    } else {
                        break;
                    }
                }

                // Check if this is an input/textarea - build layout from value
                let tag = node.tag();
                if matches!(tag, Some("input" | "textarea")) {
                    let value = node.attributes.get("value").cloned().unwrap_or_default();
                    if value.is_empty() {
                        return DebugResult::Error { message: "No text content".into() };
                    }

                    let computed_style = node.computed_style.clone();
                    let input_width = node.layout.width;
                    drop(d);

                    let layout = computed_style.build_parley_layout(
                        &value,
                        scale,
                        &mut self.hit_test_font_cx,
                        &mut self.paint_layout_cx,
                        Some(input_width),
                    );

                    match glyph_bounds_for_offset_layout(&layout, byte_offset) {
                        Some(bounds) => {
                            let padding_left = computed_style.padding_left.to_px() as f64 * scale as f64;
                            let padding_top = computed_style.padding_top.to_px() as f64 * scale as f64;
                            return DebugResult::Json { data: json!({
                                "x": abs_x + padding_left + bounds.x as f64,
                                "y": abs_y + padding_top + bounds.y as f64,
                                "width": bounds.width,
                                "height": bounds.height,
                            })};
                        }
                        None => {
                            return DebugResult::Error { message: "Byte offset out of bounds".into() };
                        }
                    }
                }

                // Check if node has inline text layout (IFC text)
                if let Some(ref inline_layout) = node.text_layout {
                    match glyph_bounds_for_offset_layout(&inline_layout.layout, byte_offset) {
                        Some(bounds) => {
                            return DebugResult::Json { data: json!({
                                "x": abs_x + bounds.x as f64,
                                "y": abs_y + bounds.y as f64,
                                "width": bounds.width,
                                "height": bounds.height,
                            })};
                        }
                        None => {
                            return DebugResult::Error { message: "Byte offset out of bounds".into() };
                        }
                    }
                }

                DebugResult::Error { message: "Node does not have text layout".into() }
            }
        }
    }

    fn handle_click(&mut self, x: f32, y: f32) {
        let Some(doc) = &self.doc else { return };
        let d = doc.borrow();

        // Walk nodes to find hit target (simple: iterate all nodes, find deepest match)
        if let Some(hit_id) = hit_test(&d.tree, x, y) {
            // Walk up to find data-rid, data-oninput, or data-drag-window
            let mut current = Some(hit_id);
            while let Some(node_id) = current {
                if let Some(node) = d.tree.get(node_id) {
                    // Check for focusable input element (input/textarea tags, not disabled)
                    let is_input = matches!(node.tag(), Some("input" | "textarea"));
                    let is_disabled = node.attributes.contains_key("disabled");
                    if is_input && !is_disabled {
                        // Get node info for cursor positioning
                        let value = node.attributes.get("value").cloned().unwrap_or_default();
                        let value_len = value.len();
                        let computed_style = node.computed_style.clone();
                        let input_width = node.layout.width;

                        // Calculate absolute x and y position of the input node
                        let mut abs_x = node.layout.x;
                        let mut abs_y = node.layout.y;
                        let mut parent_id = node.parent;
                        while let Some(pid) = parent_id {
                            if let Some(parent_node) = d.tree.get(pid) {
                                abs_x += parent_node.layout.x;
                                abs_y += parent_node.layout.y;
                                // Account for scroll offset
                                abs_x -= parent_node.scroll_offset.0 as f32;
                                abs_y -= parent_node.scroll_offset.1 as f32;
                                parent_id = parent_node.parent;
                            } else {
                                break;
                            }
                        }

                        drop(d);

                        // Clear focus attributes from previously focused input
                        if let Some(prev_id) = self.focused_input {
                            if prev_id != node_id {
                                if let Some(doc) = &self.doc {
                                    let mut d = doc.borrow_mut();
                                    if let Some(prev_node) = d.tree.nodes.get_mut(prev_id) {
                                        prev_node.attributes.remove("data-focused");
                                        prev_node.attributes.remove("data-cursor-pos");
                                        prev_node.attributes.remove("data-selection-start");
                                        prev_node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                                        d.tree.dirty_nodes.insert(prev_id);
                                    }
                                }
                            }
                        }

                        // Calculate cursor position from click coordinates
                        let cursor_pos = if !value.is_empty() {
                            // Get scale factor
                            let scale = self.window.as_ref()
                                .map(|w| w.scale_factor() as f32)
                                .unwrap_or(1.0);

                            // Build text layout using computed_style
                            let layout = computed_style.build_parley_layout(
                                &value,
                                scale,
                                &mut self.hit_test_font_cx,
                                &mut self.paint_layout_cx,
                                Some(input_width),
                            );

                            // Calculate padding using computed_style
                            let padding_left = computed_style.padding_left.to_px() * scale;
                            let padding_top = computed_style.padding_top.to_px() * scale;

                            // Calculate click position relative to text start
                            let click_rel_x = (x - abs_x - padding_left).max(0.0);
                            let click_rel_y = (y - abs_y - padding_top).max(0.0);

                            Self::byte_offset_from_xy(&layout, click_rel_x, click_rel_y)
                        } else {
                            0
                        };

                        self.focused_input = Some(node_id);
                        self.input_cursor = cursor_pos.min(value_len);
                        self.input_selection_start = self.input_cursor;
                        self.cursor_visible = true;
                        self.cursor_blink_time = std::time::Instant::now();
                        self.input_mouse_drag = true;

                        // Set focus attributes on the node
                        self.sync_cursor_to_dom(node_id);

                        tracing::debug!("Focused input element: node_id={}, cursor={}", node_id, self.input_cursor);
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                        return;
                    }
                    // Check for click handler
                    if let Some(rid_str) = node.attributes.get("data-rid") {
                        if let Ok(handler_id) = rid_str.parse::<usize>() {
                            // Compute text hit info for rich text editor click-to-position
                            let text_hit = Self::compute_text_hit_info(&d.tree, hit_id, x, y);

                            // Get element bounds for click context
                            let (elem_x, elem_y, elem_w, elem_h) = {
                                let mut ax = node.layout.x;
                                let mut ay = node.layout.y;
                                let mut pid = node.parent;
                                while let Some(p) = pid {
                                    if let Some(pn) = d.tree.get(p) {
                                        ax += pn.layout.x;
                                        ay += pn.layout.y;
                                        ax -= pn.scroll_offset.0 as f32;
                                        ay -= pn.scroll_offset.1 as f32;
                                        pid = pn.parent;
                                    } else {
                                        break;
                                    }
                                }
                                (ax, ay, node.layout.width, node.layout.height)
                            };

                            // Set click context before dispatching
                            events::set_click_context(events::ClickContext {
                                mouse_x: x,
                                mouse_y: y,
                                element_x: elem_x,
                                element_y: elem_y,
                                element_width: elem_w,
                                element_height: elem_h,
                                text_hit,
                            });

                            // Must drop borrow before dispatching (handler may mutate doc)
                            drop(d);
                            // Clear focus when clicking non-input elements
                            self.clear_input_focus();
                            // Set current window ID so window control functions work
                            if let Some(w) = &self.window {
                                crate::windows::set_current_window_id(Some(w.id()));
                            }
                            events::dispatch_event(events::EventHandlerId(handler_id));
                            crate::windows::set_current_window_id(None);
                            // Check for pending focus requests from the event handler
                            if let Some(focus_node_id) = rinch_core::take_pending_focus_request() {
                                self.focused_input = Some(focus_node_id);
                                self.input_cursor = 0;
                                self.input_selection_start = 0;
                                self.cursor_visible = true;
                                self.cursor_blink_time = std::time::Instant::now();
                                tracing::debug!("Applied focus request: node_id={}", focus_node_id);
                            }
                            return;
                        }
                    }
                    // Check for drag-window region
                    if node.attributes.get("data-drag-window").is_some() {
                        drop(d);
                        if let Some(w) = &self.window {
                            let _ = w.drag_window();
                        }
                        return;
                    }
                    current = node.parent;
                } else {
                    break;
                }
            }
            // Clicked on something without a handler - clear focus
            drop(d);
            self.clear_input_focus();
        }
    }

    /// Compute text hit info for click-to-position in rich text editors.
    ///
    /// Walks up from the hit node to find block index and computes byte offset
    /// from the text layout at the click position.
    fn compute_text_hit_info(
        tree: &rinch_dom::NodeTree,
        hit_id: usize,
        click_x: f32,
        click_y: f32,
    ) -> events::TextHitInfo {
        // Walk up from hit node to find block index (data-block-index attribute)
        let mut block_index = 0usize;
        let mut block_node_id = None;
        let mut current = Some(hit_id);

        while let Some(node_id) = current {
            if let Some(node) = tree.get(node_id) {
                if let Some(idx_str) = node.attributes.get("data-block-index") {
                    if let Ok(idx) = idx_str.parse::<usize>() {
                        block_index = idx;
                        block_node_id = Some(node_id);
                        break;
                    }
                }
                current = node.parent;
            } else {
                break;
            }
        }

        let Some(block_id) = block_node_id else {
            return events::TextHitInfo::default();
        };

        // Get the block node and compute click position relative to it
        let Some(block_node) = tree.get(block_id) else {
            return events::TextHitInfo::default();
        };

        // Calculate absolute position of the block node
        let mut abs_x = block_node.layout.x;
        let mut abs_y = block_node.layout.y;
        let mut parent_id = block_node.parent;
        while let Some(pid) = parent_id {
            if let Some(pn) = tree.get(pid) {
                abs_x += pn.layout.x;
                abs_y += pn.layout.y;
                abs_x -= pn.scroll_offset.0 as f32;
                abs_y -= pn.scroll_offset.1 as f32;
                parent_id = pn.parent;
            } else {
                break;
            }
        }

        // Calculate click position relative to block
        let rel_x = (click_x - abs_x).max(0.0);
        let rel_y = (click_y - abs_y).max(0.0);

        // Try to get text layout from the block node or its first text child
        let byte_offset = if let Some(ref layout) = block_node.text_layout {
            // IFC root has text_layout
            byte_offset_from_position(&layout.layout, rel_x, rel_y)
        } else if let Some(ref layout) = block_node.cached_text_parley {
            // Block has cached text layout
            byte_offset_from_position(layout, rel_x, rel_y)
        } else {
            // Check first-level children for text layout (non-IFC case)
            let mut offset = 0usize;
            for &child_id in &block_node.children {
                if let Some(child) = tree.nodes.get(child_id) {
                    if let Some(ref layout) = child.cached_text_parley {
                        offset = byte_offset_from_position(layout, rel_x, rel_y);
                        break;
                    }
                }
            }
            offset
        };

        events::TextHitInfo {
            block_index,
            byte_offset,
            inline_root_node_id: block_id,
            valid: true,
        }
    }

    /// Handle double-click to select a word.
    fn handle_double_click(&mut self, x: f32, y: f32) {
        // First do normal click to focus and position
        self.handle_click(x, y);

        let Some(focused_id) = self.focused_input else { return };
        let Some(doc) = &self.doc else { return };

        let value = {
            let d = doc.borrow();
            let Some(node) = d.tree.get(focused_id) else { return };
            node.attributes.get("value").cloned().unwrap_or_default()
        };

        if value.is_empty() {
            return;
        }

        // Find word boundaries around cursor
        let cursor = self.input_cursor.min(value.len());
        let word_start = Self::find_word_start(&value, cursor);
        let word_end = Self::find_word_end(&value, cursor);

        self.input_selection_start = word_start;
        self.input_cursor = word_end;
        self.cursor_visible = true;
        self.cursor_blink_time = std::time::Instant::now();
        self.input_mouse_drag = false; // Don't start drag on double-click

        self.sync_cursor_to_dom(focused_id);
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Handle triple-click to select all text.
    fn handle_triple_click(&mut self, x: f32, y: f32) {
        // First do normal click to focus
        self.handle_click(x, y);

        let Some(focused_id) = self.focused_input else { return };
        let Some(doc) = &self.doc else { return };

        let value_len = {
            let d = doc.borrow();
            let Some(node) = d.tree.get(focused_id) else { return };
            node.attributes.get("value").map(|v| v.len()).unwrap_or(0)
        };

        // Select all
        self.input_selection_start = 0;
        self.input_cursor = value_len;
        self.cursor_visible = true;
        self.cursor_blink_time = std::time::Instant::now();
        self.input_mouse_drag = false;

        self.sync_cursor_to_dom(focused_id);
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Clear input focus and remove focus attributes from DOM.
    fn clear_input_focus(&mut self) {
        if let Some(focused_id) = self.focused_input.take() {
            if let Some(doc) = &self.doc {
                let mut d = doc.borrow_mut();
                if let Some(node) = d.tree.nodes.get_mut(focused_id) {
                    node.attributes.remove("data-focused");
                    node.attributes.remove("data-cursor-pos");
                    node.attributes.remove("data-selection-start");
                    node.attributes.remove("data-cursor-visible");
                    node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                    d.tree.dirty_nodes.insert(focused_id);
                }
            }
        }
    }

    /// Sync cursor state to DOM attributes for paint.
    fn sync_cursor_to_dom(&mut self, node_id: usize) {
        if let Some(doc) = &self.doc {
            let mut d = doc.borrow_mut();
            if let Some(node) = d.tree.nodes.get_mut(node_id) {
                node.attributes.insert("data-focused".to_string(), "true".to_string());
                node.attributes.insert("data-cursor-pos".to_string(), self.input_cursor.to_string());
                node.attributes.insert("data-selection-start".to_string(), self.input_selection_start.to_string());
                node.attributes.insert("data-cursor-visible".to_string(), self.cursor_visible.to_string());
                node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                d.tree.dirty_nodes.insert(node_id);
            }
        }
    }

    /// Handle keyboard input for focused input elements.
    fn handle_text_input(&mut self, text: &str) {
        let Some(focused_id) = self.focused_input else { return };
        let Some(doc) = &self.doc else { return };

        // Get the current value and handler ID
        let (current_value, handler_id, is_disabled) = {
            let d = doc.borrow();
            let Some(node) = d.tree.get(focused_id) else { return };
            let is_disabled = node.attributes.contains_key("disabled");
            let value = node.attributes.get("value").cloned().unwrap_or_default();
            let handler_id = node.attributes.get("data-oninput")
                .and_then(|s| s.parse::<usize>().ok());
            (value, handler_id, is_disabled)
        };

        // Block input to disabled fields
        if is_disabled {
            return;
        }

        // Clamp cursor positions to valid range
        let len = current_value.len();
        let cursor = self.input_cursor.min(len);
        let sel_start = self.input_selection_start.min(len);

        // Determine the range to replace (either selection or just cursor position)
        let (start, end) = if cursor != sel_start {
            (cursor.min(sel_start), cursor.max(sel_start))
        } else {
            (cursor, cursor)
        };

        // Snap to valid UTF-8 boundaries
        let start = Self::snap_to_char_boundary(&current_value, start);
        let end = Self::snap_to_char_boundary(&current_value, end);

        // Build new value: before + inserted text + after
        let new_value = format!(
            "{}{}{}",
            &current_value[..start],
            text,
            &current_value[end..]
        );

        // Update cursor to end of inserted text
        let new_cursor = start + text.len();
        self.input_cursor = new_cursor;
        self.input_selection_start = new_cursor;

        // Reset cursor blink
        self.cursor_visible = true;
        self.cursor_blink_time = std::time::Instant::now();

        // Update the DOM value and sync cursor state
        {
            let mut d = doc.borrow_mut();
            if let Some(node) = d.tree.nodes.get_mut(focused_id) {
                node.attributes.insert("value".to_string(), new_value.clone());
                node.attributes.insert("data-focused".to_string(), "true".to_string());
                node.attributes.insert("data-cursor-pos".to_string(), self.input_cursor.to_string());
                node.attributes.insert("data-selection-start".to_string(), self.input_selection_start.to_string());
                node.attributes.insert("data-cursor-visible".to_string(), "true".to_string());
                node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                d.tree.dirty_nodes.insert(focused_id);
            }
        }

        // Dispatch the input event
        if let Some(id) = handler_id {
            events::dispatch_input_event(events::EventHandlerId(id), new_value);
        }

        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Handle backspace key for focused input elements.
    fn handle_backspace(&mut self) {
        let Some(focused_id) = self.focused_input else { return };
        let Some(doc) = &self.doc else { return };

        // Get the current value and handler ID
        let (current_value, handler_id) = {
            let d = doc.borrow();
            let Some(node) = d.tree.get(focused_id) else { return };
            let value = node.attributes.get("value").cloned().unwrap_or_default();
            let handler_id = node.attributes.get("data-oninput")
                .and_then(|s| s.parse::<usize>().ok());
            (value, handler_id)
        };

        // Clamp cursor positions to valid range
        let len = current_value.len();
        let cursor = self.input_cursor.min(len);
        let sel_start = self.input_selection_start.min(len);

        // Determine what to delete
        let (start, end, new_cursor) = if cursor != sel_start {
            // Delete selection
            let s = cursor.min(sel_start);
            let e = cursor.max(sel_start);
            (s, e, s)
        } else if cursor > 0 {
            // Delete character before cursor (find char boundary)
            let prev_char_start = current_value[..cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            (prev_char_start, cursor, prev_char_start)
        } else {
            // At start, nothing to delete
            return;
        };

        // Snap to valid UTF-8 boundaries
        let start = Self::snap_to_char_boundary(&current_value, start);
        let end = Self::snap_to_char_boundary(&current_value, end);

        // Build new value
        let new_value = format!("{}{}", &current_value[..start], &current_value[end..]);

        // Update cursor
        self.input_cursor = new_cursor;
        self.input_selection_start = new_cursor;

        // Reset cursor blink
        self.cursor_visible = true;
        self.cursor_blink_time = std::time::Instant::now();

        // Update the DOM value and sync cursor state
        {
            let mut d = doc.borrow_mut();
            if let Some(node) = d.tree.nodes.get_mut(focused_id) {
                node.attributes.insert("value".to_string(), new_value.clone());
                node.attributes.insert("data-focused".to_string(), "true".to_string());
                node.attributes.insert("data-cursor-pos".to_string(), self.input_cursor.to_string());
                node.attributes.insert("data-selection-start".to_string(), self.input_selection_start.to_string());
                node.attributes.insert("data-cursor-visible".to_string(), "true".to_string());
                node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                d.tree.dirty_nodes.insert(focused_id);
            }
        }

        // Dispatch the input event
        if let Some(id) = handler_id {
            events::dispatch_input_event(events::EventHandlerId(id), new_value);
        }

        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Handle Delete key for focused input elements.
    fn handle_delete(&mut self) {
        let Some(focused_id) = self.focused_input else { return };
        let Some(doc) = &self.doc else { return };

        let (current_value, handler_id) = {
            let d = doc.borrow();
            let Some(node) = d.tree.get(focused_id) else { return };
            let value = node.attributes.get("value").cloned().unwrap_or_default();
            let handler_id = node.attributes.get("data-oninput")
                .and_then(|s| s.parse::<usize>().ok());
            (value, handler_id)
        };

        let len = current_value.len();
        let cursor = self.input_cursor.min(len);
        let sel_start = self.input_selection_start.min(len);

        let (start, end, new_cursor) = if cursor != sel_start {
            // Delete selection
            let s = cursor.min(sel_start);
            let e = cursor.max(sel_start);
            (s, e, s)
        } else if cursor < len {
            // Delete character after cursor (find next char boundary)
            let next_char_end = current_value[cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| cursor + i)
                .unwrap_or(len);
            (cursor, next_char_end, cursor)
        } else {
            return; // At end, nothing to delete
        };

        // Snap to valid UTF-8 boundaries
        let start = Self::snap_to_char_boundary(&current_value, start);
        let end = Self::snap_to_char_boundary(&current_value, end);

        let new_value = format!("{}{}", &current_value[..start], &current_value[end..]);
        self.input_cursor = new_cursor;
        self.input_selection_start = new_cursor;
        self.cursor_visible = true;
        self.cursor_blink_time = std::time::Instant::now();

        {
            let mut d = doc.borrow_mut();
            if let Some(node) = d.tree.nodes.get_mut(focused_id) {
                node.attributes.insert("value".to_string(), new_value.clone());
                node.attributes.insert("data-focused".to_string(), "true".to_string());
                node.attributes.insert("data-cursor-pos".to_string(), self.input_cursor.to_string());
                node.attributes.insert("data-selection-start".to_string(), self.input_selection_start.to_string());
                node.attributes.insert("data-cursor-visible".to_string(), "true".to_string());
                node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                d.tree.dirty_nodes.insert(focused_id);
            }
        }

        if let Some(id) = handler_id {
            events::dispatch_input_event(events::EventHandlerId(id), new_value);
        }

        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Handle left arrow key.
    fn handle_arrow_left(&mut self, shift: bool, ctrl: bool) {
        let Some(focused_id) = self.focused_input else { return };
        let Some(doc) = &self.doc else { return };

        let value = {
            let d = doc.borrow();
            let Some(node) = d.tree.get(focused_id) else { return };
            node.attributes.get("value").cloned().unwrap_or_default()
        };

        let len = value.len();
        let cursor = self.input_cursor.min(len);

        let new_cursor = if ctrl {
            // Move to previous word boundary
            Self::find_prev_word_boundary(&value, cursor)
        } else if cursor > 0 {
            // Move to previous character
            value[..cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0)
        } else {
            0
        };

        self.input_cursor = new_cursor;
        if !shift {
            self.input_selection_start = new_cursor;
        }
        self.cursor_visible = true;
        self.cursor_blink_time = std::time::Instant::now();
        self.sync_cursor_to_dom(focused_id);

        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Handle right arrow key.
    fn handle_arrow_right(&mut self, shift: bool, ctrl: bool) {
        let Some(focused_id) = self.focused_input else { return };
        let Some(doc) = &self.doc else { return };

        let value = {
            let d = doc.borrow();
            let Some(node) = d.tree.get(focused_id) else { return };
            node.attributes.get("value").cloned().unwrap_or_default()
        };

        let len = value.len();
        let cursor = self.input_cursor.min(len);

        let new_cursor = if ctrl {
            // Move to next word boundary
            Self::find_next_word_boundary(&value, cursor)
        } else if cursor < len {
            // Move to next character
            value[cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| cursor + i)
                .unwrap_or(len)
        } else {
            len
        };

        self.input_cursor = new_cursor;
        if !shift {
            self.input_selection_start = new_cursor;
        }
        self.cursor_visible = true;
        self.cursor_blink_time = std::time::Instant::now();
        self.sync_cursor_to_dom(focused_id);

        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Handle Enter key - insert newline for textarea elements.
    fn handle_enter(&mut self) {
        let Some(focused_id) = self.focused_input else { return };
        let Some(doc) = &self.doc else { return };

        // Check if this is a textarea element
        let is_textarea = {
            let d = doc.borrow();
            d.tree.get(focused_id)
                .map(|node| node.tag() == Some("textarea"))
                .unwrap_or(false)
        };

        if is_textarea {
            self.handle_text_input("\n");
        }
    }

    /// Handle up arrow key for multi-line navigation.
    fn handle_arrow_up(&mut self, shift: bool) {
        let Some(focused_id) = self.focused_input else { return };
        let Some(doc) = &self.doc else { return };

        let value = {
            let d = doc.borrow();
            let Some(node) = d.tree.get(focused_id) else { return };
            node.attributes.get("value").cloned().unwrap_or_default()
        };

        let len = value.len();
        let cursor = self.input_cursor.min(len);

        // Find the start of the current line
        let line_start = value[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);

        if line_start == 0 {
            // Already on first line - move to start
            self.input_cursor = 0;
        } else {
            // Find the end of the previous line (the newline before current line)
            let prev_line_end = line_start - 1;
            // Find the start of the previous line
            let prev_line_start = value[..prev_line_end].rfind('\n').map(|i| i + 1).unwrap_or(0);

            // Calculate column offset on current line
            let column = cursor - line_start;
            // Calculate the length of the previous line
            let prev_line_len = prev_line_end - prev_line_start;

            // Move to same column on previous line (or end of line if shorter)
            self.input_cursor = prev_line_start + column.min(prev_line_len);
        }

        if !shift {
            self.input_selection_start = self.input_cursor;
        }
        self.cursor_visible = true;
        self.cursor_blink_time = std::time::Instant::now();
        self.sync_cursor_to_dom(focused_id);

        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Handle down arrow key for multi-line navigation.
    fn handle_arrow_down(&mut self, shift: bool) {
        let Some(focused_id) = self.focused_input else { return };
        let Some(doc) = &self.doc else { return };

        let value = {
            let d = doc.borrow();
            let Some(node) = d.tree.get(focused_id) else { return };
            node.attributes.get("value").cloned().unwrap_or_default()
        };

        let len = value.len();
        let cursor = self.input_cursor.min(len);

        // Find the start of the current line
        let line_start = value[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);

        // Find the end of the current line (next newline or end of text)
        let line_end = value[cursor..].find('\n').map(|i| cursor + i).unwrap_or(len);

        if line_end == len {
            // Already on last line - move to end
            self.input_cursor = len;
        } else {
            // Find the start of the next line
            let next_line_start = line_end + 1;
            // Find the end of the next line
            let next_line_end = value[next_line_start..].find('\n')
                .map(|i| next_line_start + i)
                .unwrap_or(len);

            // Calculate column offset on current line
            let column = cursor - line_start;
            // Calculate the length of the next line
            let next_line_len = next_line_end - next_line_start;

            // Move to same column on next line (or end of line if shorter)
            self.input_cursor = next_line_start + column.min(next_line_len);
        }

        if !shift {
            self.input_selection_start = self.input_cursor;
        }
        self.cursor_visible = true;
        self.cursor_blink_time = std::time::Instant::now();
        self.sync_cursor_to_dom(focused_id);

        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Handle Home key.
    fn handle_home(&mut self, shift: bool) {
        let Some(focused_id) = self.focused_input else { return };

        self.input_cursor = 0;
        if !shift {
            self.input_selection_start = 0;
        }
        self.cursor_visible = true;
        self.cursor_blink_time = std::time::Instant::now();
        self.sync_cursor_to_dom(focused_id);

        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Handle End key.
    fn handle_end(&mut self, shift: bool) {
        let Some(focused_id) = self.focused_input else { return };
        let Some(doc) = &self.doc else { return };

        let len = {
            let d = doc.borrow();
            let Some(node) = d.tree.get(focused_id) else { return };
            node.attributes.get("value").map(|v| v.len()).unwrap_or(0)
        };

        self.input_cursor = len;
        if !shift {
            self.input_selection_start = len;
        }
        self.cursor_visible = true;
        self.cursor_blink_time = std::time::Instant::now();
        self.sync_cursor_to_dom(focused_id);

        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Handle Ctrl+A (select all).
    fn handle_select_all(&mut self) {
        let Some(focused_id) = self.focused_input else { return };
        let Some(doc) = &self.doc else { return };

        let len = {
            let d = doc.borrow();
            let Some(node) = d.tree.get(focused_id) else { return };
            node.attributes.get("value").map(|v| v.len()).unwrap_or(0)
        };

        self.input_selection_start = 0;
        self.input_cursor = len;
        self.cursor_visible = true;
        self.cursor_blink_time = std::time::Instant::now();
        self.sync_cursor_to_dom(focused_id);

        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Handle Ctrl+C (copy).
    fn handle_copy(&mut self) {
        let Some(focused_id) = self.focused_input else { return };
        let Some(doc) = &self.doc else { return };

        let (value, is_password) = {
            let d = doc.borrow();
            let Some(node) = d.tree.get(focused_id) else { return };
            let value = node.attributes.get("value").cloned().unwrap_or_default();
            let is_password = node.attributes.get("type").map(|t| t == "password").unwrap_or(false);
            (value, is_password)
        };

        if is_password {
            return; // Security: never copy from password fields
        }

        let len = value.len();
        let cursor = self.input_cursor.min(len);
        let sel_start = self.input_selection_start.min(len);

        if cursor == sel_start {
            return; // No selection to copy
        }

        let start = cursor.min(sel_start);
        let end = cursor.max(sel_start);
        let selected_text = &value[start..end];

        #[cfg(feature = "clipboard")]
        {
            let _ = crate::clipboard::copy_text(selected_text);
        }
        #[cfg(not(feature = "clipboard"))]
        {
            tracing::warn!("Clipboard feature not enabled, cannot copy");
            let _ = selected_text;
        }
    }

    /// Handle Ctrl+V (paste).
    fn handle_paste(&mut self) {
        #[cfg(feature = "clipboard")]
        {
            if let Ok(text) = crate::clipboard::paste_text() {
                if !text.is_empty() {
                    self.handle_text_input(&text);
                }
            }
        }
        #[cfg(not(feature = "clipboard"))]
        {
            tracing::warn!("Clipboard feature not enabled, cannot paste");
        }
    }

    /// Handle Ctrl+X (cut).
    fn handle_cut(&mut self) {
        let Some(focused_id) = self.focused_input else { return };
        let Some(doc) = &self.doc else { return };

        let (value, handler_id, is_password) = {
            let d = doc.borrow();
            let Some(node) = d.tree.get(focused_id) else { return };
            let value = node.attributes.get("value").cloned().unwrap_or_default();
            let handler_id = node.attributes.get("data-oninput")
                .and_then(|s| s.parse::<usize>().ok());
            let is_password = node.attributes.get("type").map(|t| t == "password").unwrap_or(false);
            (value, handler_id, is_password)
        };

        if is_password {
            return; // Security: never cut from password fields
        }

        let len = value.len();
        let cursor = self.input_cursor.min(len);
        let sel_start = self.input_selection_start.min(len);

        if cursor == sel_start {
            return; // No selection to cut
        }

        let start = cursor.min(sel_start);
        let end = cursor.max(sel_start);

        // Snap to valid UTF-8 boundaries
        let start = Self::snap_to_char_boundary(&value, start);
        let end = Self::snap_to_char_boundary(&value, end);

        let selected_text = &value[start..end];

        // Copy to clipboard
        #[cfg(feature = "clipboard")]
        {
            let _ = crate::clipboard::copy_text(selected_text);
        }

        // Delete the selection
        let new_value = format!("{}{}", &value[..start], &value[end..]);
        self.input_cursor = start;
        self.input_selection_start = start;
        self.cursor_visible = true;
        self.cursor_blink_time = std::time::Instant::now();

        {
            let mut d = doc.borrow_mut();
            if let Some(node) = d.tree.nodes.get_mut(focused_id) {
                node.attributes.insert("value".to_string(), new_value.clone());
                node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                d.tree.dirty_nodes.insert(focused_id);
            }
        }

        if let Some(id) = handler_id {
            events::dispatch_input_event(events::EventHandlerId(id), new_value);
        }

        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Find the previous word boundary from cursor position.
    fn find_prev_word_boundary(text: &str, cursor: usize) -> usize {
        let cursor = Self::snap_to_char_boundary(text, cursor);
        let before = &text[..cursor];
        // Skip any trailing whitespace/punctuation
        let trimmed = before.trim_end_matches(|c: char| !c.is_alphanumeric());
        if trimmed.is_empty() {
            return 0;
        }
        // Find the start of the current word
        trimmed
            .rfind(|c: char| !c.is_alphanumeric())
            .map(|i| i + 1)
            .unwrap_or(0)
    }

    /// Find the next word boundary from cursor position.
    fn find_next_word_boundary(text: &str, cursor: usize) -> usize {
        let cursor = Self::snap_to_char_boundary(text, cursor);
        let after = &text[cursor..];
        // Skip current word
        let word_end = after
            .find(|c: char| !c.is_alphanumeric())
            .unwrap_or(after.len());
        // Skip whitespace/punctuation after word
        let rest = &after[word_end..];
        let space_end = rest
            .find(|c: char| c.is_alphanumeric())
            .unwrap_or(rest.len());
        cursor + word_end + space_end
    }

    /// Find the start of the word containing the given position.
    fn find_word_start(text: &str, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        let pos = Self::snap_to_char_boundary(text, pos);
        let before = &text[..pos];
        // Walk backwards to find non-word character
        for (i, c) in before.char_indices().rev() {
            if !c.is_alphanumeric() && c != '_' {
                return i + c.len_utf8();
            }
        }
        0
    }

    /// Find the end of the word containing the given position.
    fn find_word_end(text: &str, pos: usize) -> usize {
        let pos = Self::snap_to_char_boundary(text, pos);
        let after = &text[pos..];
        // Walk forwards to find non-word character
        for (i, c) in after.char_indices() {
            if !c.is_alphanumeric() && c != '_' {
                return pos + i;
            }
        }
        text.len()
    }

    /// Snap a byte position to a valid UTF-8 character boundary.
    /// If pos is already at a boundary, returns pos unchanged.
    /// Otherwise walks backwards to find the nearest valid boundary.
    fn snap_to_char_boundary(s: &str, pos: usize) -> usize {
        if pos >= s.len() {
            return s.len();
        }
        if s.is_char_boundary(pos) {
            return pos;
        }
        // Walk backwards to find valid boundary
        (0..pos).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0)
    }

    /// Calculate byte offset from click coordinates relative to text start.
    /// Returns the byte offset closest to the click position, accounting for which line was clicked.
    fn byte_offset_from_xy(
        layout: &parley::layout::Layout<peniko::Brush>,
        click_x: f32,
        click_y: f32,
    ) -> usize {
        byte_offset_from_position(layout, click_x, click_y)
    }
}

impl ApplicationHandler<RinchNativeEvent> for RinchRuntime {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.create_window(event_loop);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: RinchNativeEvent) {
        match event {
            RinchNativeEvent::ReRender => {
                self.resolve_and_repaint();
            }
            #[cfg(feature = "debug")]
            RinchNativeEvent::DebugCommand => {
                self.handle_debug_commands();
            }
            RinchNativeEvent::MinimizeWindow => {
                if let Some(w) = &self.window {
                    w.set_minimized(true);
                }
            }
            RinchNativeEvent::ToggleMaximizeWindow => {
                if let Some(w) = &self.window {
                    w.set_maximized(!w.is_maximized());
                }
            }
            RinchNativeEvent::CloseWindowControl => {
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.resize(size.width, size.height);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.paint() {
                    eprintln!("Paint error: {}", e);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let x = position.x as f32;
                let y = position.y as f32;
                self.cursor_pos = Some((x, y));

                // Handle scrollbar drag
                if let Some(drag) = &self.scrollbar_drag {
                    let node_id = drag.node_id;
                    let dy = y - drag.start_y;
                    let track_height = drag.container_height - 4.0; // 2px margin each side
                    let max_scroll = drag.content_height - drag.container_height;
                    let scroll_delta = (dy as f64 / track_height) * drag.content_height;
                    let new_scroll = (drag.start_scroll + scroll_delta).clamp(0.0, max_scroll);

                    if let Some(doc) = &self.doc {
                        let mut d = doc.borrow_mut();
                        if let Some(node) = d.tree.nodes.get_mut(node_id) {
                            node.scroll_offset.1 = new_scroll;
                            node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                            d.tree.dirty_nodes.insert(node_id);
                        }
                    }
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }

                // Handle text selection drag in input
                if self.input_mouse_drag {
                    if let Some(focused_id) = self.focused_input {
                        if let Some(doc) = &self.doc {
                            let (value, computed_style, input_width, abs_x, abs_y) = {
                                let d = doc.borrow();
                                if let Some(node) = d.tree.get(focused_id) {
                                    let value = node.attributes.get("value").cloned().unwrap_or_default();
                                    let style = node.computed_style.clone();
                                    let width = node.layout.width;

                                    // Calculate absolute x and y position
                                    let mut ax = node.layout.x;
                                    let mut ay = node.layout.y;
                                    let mut parent_id = node.parent;
                                    while let Some(pid) = parent_id {
                                        if let Some(parent_node) = d.tree.get(pid) {
                                            ax += parent_node.layout.x;
                                            ay += parent_node.layout.y;
                                            ax -= parent_node.scroll_offset.0 as f32;
                                            ay -= parent_node.scroll_offset.1 as f32;
                                            parent_id = parent_node.parent;
                                        } else {
                                            break;
                                        }
                                    }
                                    (value, style, width, ax, ay)
                                } else {
                                    (String::new(), rinch_dom::ComputedStyle::default(), 0.0, 0.0, 0.0)
                                }
                            };

                            if !value.is_empty() {
                                let scale = self.window.as_ref()
                                    .map(|w| w.scale_factor() as f32)
                                    .unwrap_or(1.0);

                                let layout = computed_style.build_parley_layout(
                                    &value,
                                    scale,
                                    &mut self.hit_test_font_cx,
                                    &mut self.paint_layout_cx,
                                    Some(input_width),
                                );

                                let padding_left = computed_style.padding_left.to_px() * scale;
                                let padding_top = computed_style.padding_top.to_px() * scale;

                                let click_rel_x = (x - abs_x - padding_left).max(0.0);
                                let click_rel_y = (y - abs_y - padding_top).max(0.0);
                                let cursor_pos = Self::byte_offset_from_xy(&layout, click_rel_x, click_rel_y).min(value.len());

                                if cursor_pos != self.input_cursor {
                                    self.input_cursor = cursor_pos;
                                    self.cursor_visible = true;
                                    self.cursor_blink_time = std::time::Instant::now();
                                    self.sync_cursor_to_dom(focused_id);

                                    if let Some(w) = &self.window {
                                        w.request_redraw();
                                    }
                                }
                            }
                        }
                    }
                    return;
                }

                // Update hover state for CSS :hover support
                if let Some(doc) = &self.doc {
                    let hovered = {
                        let d = doc.borrow();
                        hit_test(&d.tree, x, y)
                    };
                    let changed = doc.borrow_mut().update_hover(hovered);
                    if changed {
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some((x, y)) = self.cursor_pos {
                    // Multi-click detection constants
                    const DOUBLE_CLICK_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);
                    const DOUBLE_CLICK_DISTANCE: f32 = 5.0;

                    // Check for multi-click
                    let now = std::time::Instant::now();
                    let elapsed = now.duration_since(self.last_click_time);
                    let (last_x, last_y) = self.last_click_pos;
                    let distance = ((x - last_x).powi(2) + (y - last_y).powi(2)).sqrt();

                    if elapsed < DOUBLE_CLICK_TIMEOUT && distance < DOUBLE_CLICK_DISTANCE {
                        self.click_count = (self.click_count % 3) + 1;
                    } else {
                        self.click_count = 1;
                    }

                    self.last_click_time = now;
                    self.last_click_pos = (x, y);

                    // Check if clicking on a scrollbar first
                    let scrollbar_hit = if let Some(doc) = &self.doc {
                        let d = doc.borrow();
                        find_scrollbar_hit(&d.tree, x, y)
                    } else {
                        None
                    };

                    if let Some((node_id, content_height, container_height)) = scrollbar_hit {
                        if let Some(doc) = &self.doc {
                            let mut d = doc.borrow_mut();
                            let node_abs_y = compute_absolute_y(&d.tree, node_id);
                            let margin = 2.0_f64;
                            let track_top = node_abs_y as f64 + margin;
                            let track_height = container_height - margin * 2.0;
                            let max_scroll = content_height - container_height;
                            let click_ratio = ((y as f64 - track_top) / track_height).clamp(0.0, 1.0);
                            let new_scroll = click_ratio * max_scroll;

                            if let Some(node) = d.tree.nodes.get_mut(node_id) {
                                node.scroll_offset.1 = new_scroll;
                                node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                                d.tree.dirty_nodes.insert(node_id);
                            }

                            self.scrollbar_drag = Some(ScrollbarDrag {
                                node_id,
                                start_y: y,
                                start_scroll: new_scroll,
                                content_height,
                                container_height,
                            });
                        }
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    } else {
                        // Route to appropriate handler based on click count
                        match self.click_count {
                            2 => self.handle_double_click(x, y),
                            3 => self.handle_triple_click(x, y),
                            _ => self.handle_click(x, y),
                        }
                    }
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.scrollbar_drag = None;
                self.input_mouse_drag = false;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // Convert delta to pixels
                let (_dx, dy) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => {
                        (x as f64 * 40.0, y as f64 * 40.0)
                    }
                    winit::event::MouseScrollDelta::PixelDelta(pos) => {
                        (pos.x, pos.y)
                    }
                };

                if let (Some((cx, cy)), Some(doc)) = (self.cursor_pos, &self.doc) {
                    let hit_node = hit_test(&doc.borrow().tree, cx, cy);
                    if let Some(hit_node) = hit_node {
                        let mut doc = doc.borrow_mut();
                        if let Some(scroll_node_id) = find_scroll_container(&doc.tree, hit_node) {
                            let content_height = compute_content_height(&doc.tree, scroll_node_id);
                            let container_height = doc.tree.get(scroll_node_id)
                                .map(|n| n.layout.height as f64)
                                .unwrap_or(0.0);
                            let max_scroll = (content_height - container_height).max(0.0);

                            if let Some(node) = doc.tree.nodes.get_mut(scroll_node_id) {
                                let new_y = (node.scroll_offset.1 - dy).clamp(0.0, max_scroll);
                                if new_y != node.scroll_offset.1 {
                                    node.scroll_offset.1 = new_y;
                                    node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                                    doc.tree.dirty_nodes.insert(scroll_node_id);
                                }
                            }
                        }
                        drop(doc);
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                }
            }
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.modifiers = new_modifiers.state();
            }
            WindowEvent::KeyboardInput {
                event: winit::event::KeyEvent {
                    physical_key: winit::keyboard::PhysicalKey::Code(key_code),
                    state: ElementState::Pressed,
                    ref text,
                    ..
                },
                ..
            } => {
                use winit::keyboard::KeyCode;
                let shift = self.modifiers.shift_key();
                // On macOS, Cmd (super_key) is used for shortcuts; on other platforms, Ctrl
                #[cfg(target_os = "macos")]
                let ctrl = self.modifiers.super_key();
                #[cfg(not(target_os = "macos"))]
                let ctrl = self.modifiers.control_key();
                let alt = self.modifiers.alt_key();

                // Convert key code to string for interceptor
                let key_str = match key_code {
                    KeyCode::ArrowLeft => Some("ArrowLeft"),
                    KeyCode::ArrowRight => Some("ArrowRight"),
                    KeyCode::ArrowUp => Some("ArrowUp"),
                    KeyCode::ArrowDown => Some("ArrowDown"),
                    KeyCode::Home => Some("Home"),
                    KeyCode::End => Some("End"),
                    KeyCode::Enter => Some("Enter"),
                    KeyCode::Backspace => Some("Backspace"),
                    KeyCode::Delete => Some("Delete"),
                    _ => None,
                };

                // Try keyboard interceptor first for navigation/editing keys
                let handled_by_interceptor = if let Some(key) = key_str {
                    let key_data = events::KeyEventData {
                        key: key.to_string(),
                        code: key.to_string(),
                        ctrl,
                        shift,
                        alt,
                        meta: false,
                    };
                    events::dispatch_keyboard_event(&key_data)
                } else {
                    false
                };

                // If interceptor handled it, skip default handling (except F12 which is always runtime)
                if handled_by_interceptor {
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                } else {
                    match key_code {
                        KeyCode::F12 => {
                            self.devtools.toggle();
                            tracing::info!("DevTools: {}", if self.devtools.visible { "opened" } else { "closed" });
                            if let Some(w) = &self.window {
                                w.request_redraw();
                            }
                        }
                        KeyCode::Backspace => {
                            self.handle_backspace();
                        }
                        KeyCode::Delete => {
                            self.handle_delete();
                        }
                        KeyCode::ArrowLeft => {
                            self.handle_arrow_left(shift, ctrl);
                        }
                        KeyCode::ArrowRight => {
                            self.handle_arrow_right(shift, ctrl);
                        }
                        KeyCode::Home => {
                            self.handle_home(shift);
                        }
                        KeyCode::End => {
                            self.handle_end(shift);
                        }
                        KeyCode::KeyA if ctrl => {
                            self.handle_select_all();
                        }
                        KeyCode::KeyC if ctrl => {
                            self.handle_copy();
                        }
                        KeyCode::KeyV if ctrl => {
                            self.handle_paste();
                        }
                        KeyCode::KeyX if ctrl => {
                            self.handle_cut();
                        }
                        KeyCode::Enter if !ctrl => {
                            // Insert newline only for textarea elements
                            self.handle_enter();
                        }
                        KeyCode::ArrowUp => {
                            self.handle_arrow_up(shift);
                        }
                        KeyCode::ArrowDown => {
                            self.handle_arrow_down(shift);
                        }
                        _ => {
                            // Handle text input for focused input elements
                            // Skip if Ctrl is held (except for Ctrl+V which is handled above)
                            if !ctrl {
                                if let Some(t) = text {
                                    if !t.is_empty() {
                                        self.handle_text_input(t.as_str());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Handle cursor blink for focused input
        if let Some(focused_id) = self.focused_input {
            let elapsed = self.cursor_blink_time.elapsed();
            const BLINK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(530);

            if elapsed >= BLINK_INTERVAL {
                self.cursor_visible = !self.cursor_visible;
                self.cursor_blink_time = std::time::Instant::now();

                // Update the DOM attribute
                if let Some(doc) = &self.doc {
                    let mut d = doc.borrow_mut();
                    if let Some(node) = d.tree.nodes.get_mut(focused_id) {
                        node.attributes.insert(
                            "data-cursor-visible".to_string(),
                            self.cursor_visible.to_string(),
                        );
                        node.dirty.insert(rinch_dom::DirtyFlags::PAINT);
                        d.tree.dirty_nodes.insert(focused_id);
                    }
                }

                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
        }

        if let Some(doc) = &self.doc {
            let has_dirty = !doc.borrow().tree.dirty_nodes.is_empty();
            if has_dirty {
                self.resolve_and_repaint();
            }
        }
    }
}

/// Simple hit testing: find the deepest node whose layout rect contains (x, y).
fn hit_test(tree: &rinch_dom::NodeTree, x: f32, y: f32) -> Option<usize> {
    hit_test_node(tree, tree.body_id, 0.0, 0.0, x, y)
}

fn hit_test_node(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
) -> Option<usize> {
    let node = tree.get(node_id)?;

    // Skip elements with pointer-events: none
    if let Some(style) = node.attributes.get("style") {
        if style.contains("pointer-events: none") || style.contains("pointer-events:none") {
            return None;
        }
    }

    let nx = offset_x + node.layout.x;
    let ny = offset_y + node.layout.y;
    let nw = node.layout.width;
    let nh = node.layout.height;

    if x < nx || x > nx + nw || y < ny || y > ny + nh {
        return None;
    }

    // Check children in reverse order (topmost first)
    let sx = node.scroll_offset.0 as f32;
    let sy = node.scroll_offset.1 as f32;
    let children: Vec<_> = node.children.clone();
    for &child_id in children.iter().rev() {
        if let Some(hit) = hit_test_node(tree, child_id, nx - sx, ny - sy, x, y) {
            return Some(hit);
        }
    }

    Some(node_id)
}

/// Find the nearest ancestor (or self) that is a scroll container.
fn find_scroll_container(tree: &rinch_dom::NodeTree, start: usize) -> Option<usize> {
    use rinch_dom::computed_style::OverflowValue;

    let mut current = Some(start);
    while let Some(node_id) = current {
        let node = tree.get(node_id)?;
        // Use computed_style instead of cached_style_props
        let overflow_y = &node.computed_style.overflow_y;
        match overflow_y {
            OverflowValue::Scroll | OverflowValue::Auto => return Some(node_id),
            OverflowValue::Hidden => {
                let content_h = compute_content_height(tree, node_id);
                if content_h > node.layout.height as f64 {
                    return Some(node_id);
                }
            }
            _ => {}
        }
        current = node.parent;
    }
    // Fall back to body if content overflows
    let body = tree.get(tree.body_id)?;
    let content_h = compute_content_height(tree, tree.body_id);
    if content_h > body.layout.height as f64 {
        return Some(tree.body_id);
    }
    None
}

/// Compute the total content height of a node from its children's layout bounds.
fn compute_content_height(tree: &rinch_dom::NodeTree, node_id: usize) -> f64 {
    let node = match tree.get(node_id) {
        Some(n) => n,
        None => return 0.0,
    };
    let mut max_bottom: f64 = 0.0;
    for &child_id in &node.children {
        if let Some(child) = tree.get(child_id) {
            let bottom = (child.layout.y + child.layout.height) as f64;
            if bottom > max_bottom {
                max_bottom = bottom;
            }
        }
    }
    max_bottom
}

/// Check if a point (x, y) hits a scrollbar. Returns the scroll container node_id,
/// content height, and container height if hit.
fn find_scrollbar_hit(
    tree: &rinch_dom::NodeTree,
    x: f32,
    y: f32,
) -> Option<(usize, f64, f64)> {
    find_scrollbar_hit_node(tree, tree.body_id, 0.0, 0.0, x, y)
}

fn find_scrollbar_hit_node(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
) -> Option<(usize, f64, f64)> {
    let node = tree.get(node_id)?;
    let nx = offset_x + node.layout.x;
    let ny = offset_y + node.layout.y;
    let nw = node.layout.width;
    let nh = node.layout.height;

    if x < nx || x > nx + nw || y < ny || y > ny + nh {
        return None;
    }

    // Check children first (depth-first, reverse order for topmost)
    let sx = node.scroll_offset.0 as f32;
    let sy = node.scroll_offset.1 as f32;
    let children: Vec<_> = node.children.clone();
    for &child_id in children.iter().rev() {
        if let Some(hit) = find_scrollbar_hit_node(tree, child_id, nx - sx, ny - sy, x, y) {
            return Some(hit);
        }
    }

    // Check if this node is a scroll container with a visible scrollbar
    use rinch_dom::computed_style::OverflowValue;
    let overflow_y = &node.computed_style.overflow_y;

    if matches!(overflow_y, OverflowValue::Scroll | OverflowValue::Auto) {
        let content_height = compute_content_height(tree, node_id);
        let container_height = nh as f64;

        if content_height > container_height {
            // Scrollbar hit area: right 16px of the container
            let scrollbar_hit_width: f32 = 16.0;
            let scrollbar_left = nx + nw - scrollbar_hit_width;

            if x >= scrollbar_left && x <= nx + nw && y >= ny && y <= ny + nh {
                return Some((node_id, content_height, container_height));
            }
        }
    }

    None
}

/// Compute the absolute Y position of a node by walking up its parent chain.
fn compute_absolute_y(tree: &rinch_dom::NodeTree, node_id: usize) -> f32 {
    let mut y = 0.0_f32;
    let mut current = Some(node_id);
    while let Some(id) = current {
        if let Some(node) = tree.get(id) {
            y += node.layout.y;
            if let Some(parent_id) = node.parent {
                if let Some(parent) = tree.get(parent_id) {
                    y -= parent.scroll_offset.1 as f32;
                }
            }
            current = node.parent;
        } else {
            break;
        }
    }
    y
}

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
                    runtime._debug_server = Some(debug_server);
                    runtime.debug_cmd_rx = Some(cmd_rx);
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
    runtime.window_props = Some(props.clone());

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
                    runtime._debug_server = Some(debug_server);
                    runtime.debug_cmd_rx = Some(cmd_rx);
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
