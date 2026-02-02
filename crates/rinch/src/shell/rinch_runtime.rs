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

#[cfg(feature = "debug")]
use {
    serde_json::json,
    super::screenshot,
    rinch_debug::{CommandReceiver, DebugCommandKind, DebugResult},
};

/// Events sent to the event loop.
#[derive(Debug, Clone)]
pub enum RinchNativeEvent {
    /// A signal changed -- re-resolve layout and repaint.
    ReRender,
    /// A debug command is waiting on the channel (debug feature).
    #[cfg(feature = "debug")]
    DebugCommand,
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
    /// Debug command receiver.
    #[cfg(feature = "debug")]
    debug_cmd_rx: Option<CommandReceiver>,
    /// Debug server handle (kept alive).
    #[cfg(feature = "debug")]
    _debug_server: Option<rinch_debug::DebugServer>,
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
            #[cfg(feature = "debug")]
            debug_cmd_rx: None,
            #[cfg(feature = "debug")]
            _debug_server: None,
        }
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) {
        let window_attrs = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(winit::dpi::LogicalSize::new(self.width, self.height));
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
            let theme_css = rinch_core::get_current_theme_css().unwrap_or_default();
            if !theme_css.is_empty() {
                d.load_css(&theme_css);
            }
            // Set viewport size so vh/vw units resolve correctly during DOM construction
            d.set_viewport(size.width as f32, size.height as f32);
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
        if let (Some(window), Some(doc)) = (&self.window, &self.doc) {
            // Resolve layout
            let size = window.inner_size();
            {
                let mut d = doc.borrow_mut();
                let _ = d.take_dirty_nodes();
                d.resolve_layout(size.width as f32, size.height as f32);
            }

            window.request_redraw();
        }
    }

    fn paint(&mut self) {
        let Some(state) = &mut self.render_state else {
            return;
        };
        let Some(doc) = &self.doc else {
            return;
        };
        let Some(window) = &self.window else {
            return;
        };

        let surface_texture = match state.surface.get_current_texture() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Failed to get surface texture: {:?}", e);
                return;
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
                &mut self.paint_layout_cx,
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
                    base_color: Color::WHITE,
                    width: state.surface_config.width,
                    height: state.surface_config.height,
                    antialiasing_method: AaConfig::Msaa16,
                },
            )
            .expect("failed to render to texture");

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
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }

    #[cfg(feature = "debug")]
    fn handle_debug_commands(&mut self) {
        let Some(rx) = self.debug_cmd_rx.take() else { return };

        while let Ok(cmd) = rx.0.try_recv() {
            let result = self.execute_debug_command(cmd.kind);
            let _ = cmd.response_tx.send(result);
        }

        self.debug_cmd_rx = Some(rx);
    }

    #[cfg(feature = "debug")]
    fn execute_debug_command(&mut self, kind: DebugCommandKind) -> DebugResult {
        match kind {
            DebugCommandKind::Screenshot => {
                self.paint();
                let Some(state) = &self.render_state else {
                    return DebugResult::Error { message: "No render state".into() };
                };
                let w = state.surface_config.width;
                let h = state.surface_config.height;
                let fmt = state.surface_config.format;
                let rgba = screenshot::capture_texture_rgba(
                    &state.device, &state.queue, &state.render_texture, w, h, fmt,
                );
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
            DebugCommandKind::TypeText { text: _text } => {
                // TODO: implement keyboard input injection
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::WaitFrame => {
                self.resolve_and_repaint();
                DebugResult::Json { data: json!(null) }
            }
            DebugCommandKind::CloseApp => {
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    std::process::exit(0);
                });
                DebugResult::Json { data: json!({"status": "closing"}) }
            }
        }
    }

    fn handle_click(&self, x: f32, y: f32) {
        let Some(doc) = &self.doc else { return };
        let d = doc.borrow();

        // Walk nodes to find hit target (simple: iterate all nodes, find deepest match)
        if let Some(hit_id) = hit_test(&d.tree, x, y) {
            // Walk up to find data-rid
            let mut current = Some(hit_id);
            while let Some(node_id) = current {
                if let Some(node) = d.tree.get(node_id) {
                    if let Some(rid_str) = node.attributes.get("data-rid") {
                        if let Ok(handler_id) = rid_str.parse::<usize>() {
                            // Must drop borrow before dispatching (handler may mutate doc)
                            drop(d);
                            events::dispatch_event(events::EventHandlerId(handler_id));
                            return;
                        }
                    }
                    current = node.parent;
                } else {
                    break;
                }
            }
        }
    }
}

impl ApplicationHandler<RinchNativeEvent> for RinchRuntime {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.create_window(event_loop);
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: RinchNativeEvent) {
        match event {
            RinchNativeEvent::ReRender => {
                self.resolve_and_repaint();
            }
            #[cfg(feature = "debug")]
            RinchNativeEvent::DebugCommand => {
                self.handle_debug_commands();
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
                self.paint();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = Some((position.x as f32, position.y as f32));
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some((x, y)) = self.cursor_pos {
                    self.handle_click(x, y);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
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
    let nx = offset_x + node.layout.x;
    let ny = offset_y + node.layout.y;
    let nw = node.layout.width;
    let nh = node.layout.height;

    if x < nx || x > nx + nw || y < ny || y > ny + nh {
        return None;
    }

    // Check children in reverse order (topmost first)
    let children: Vec<_> = node.children.clone();
    for &child_id in children.iter().rev() {
        if let Some(hit) = hit_test_node(tree, child_id, nx, ny, x, y) {
            return Some(hit);
        }
    }

    Some(node_id)
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
