//! Embed external renderers into the rinch layout.
//!
//! `RenderSurface` lets you feed raw RGBA pixels from any source (game engine,
//! terminal emulator, video decoder, custom GPU renderer) into a rinch layout.
//! On desktop, rinch composites the pixels via a hole-punch + WGSL compositor.
//! On web, a native `<canvas>` element is used and the browser handles compositing.
//!
//! # Usage
//!
//! ```ignore
//! use rinch::prelude::*;
//! use rinch::render_surface::*;
//!
//! let surface = create_render_surface();
//!
//! // Receive mouse/keyboard events (main thread)
//! surface.set_event_handler(move |event| match event {
//!     SurfaceEvent::MouseDown { x, y, .. } => { /* local coords */ },
//!     _ => {}
//! });
//!
//! // Writer is Send + Sync + Clone — use from any thread
//! let writer = surface.writer();
//! std::thread::spawn(move || {
//!     loop {
//!         let pixels = my_render();
//!         writer.submit_frame(&pixels, 640, 480);
//!     }
//! });
//!
//! rsx! {
//!     div { style: "width: 640px; height: 480px;",
//!         RenderSurface { surface: surface }
//!     }
//! }
//! ```

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

// ── TextureSource (desktop only) ────────────────────────────────────────────

/// A GPU texture source for zero-copy compositing.
///
/// When set on a [`RenderSurfaceHandle`], the compositor reads this texture
/// directly instead of uploading CPU pixel data. The texture must be created
/// on the same wgpu Device (available via [`super::shell::desktop::gpu_handle`]).
#[cfg(feature = "desktop")]
pub struct TextureSource {
    /// The texture view to composite.
    pub view: wgpu::TextureView,
    /// Texture width in pixels.
    pub width: u32,
    /// Texture height in pixels.
    pub height: u32,
}

// ── Surface ID counter ───────────────────────────────────────────────────────

static NEXT_SURFACE_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

fn next_surface_id() -> usize {
    NEXT_SURFACE_ID.fetch_add(1, Ordering::Relaxed)
}

// ── SurfaceEvent ─────────────────────────────────────────────────────────────

/// Events dispatched to a render surface's event handler.
///
/// Coordinates are in logical pixels relative to the surface's top-left corner.
#[derive(Debug, Clone)]
pub enum SurfaceEvent {
    /// Mouse button pressed inside the surface.
    MouseDown {
        x: f32,
        y: f32,
        button: SurfaceMouseButton,
    },
    /// Mouse moved over the surface.
    MouseMove { x: f32, y: f32 },
    /// Mouse button released.
    MouseUp {
        x: f32,
        y: f32,
        button: SurfaceMouseButton,
    },
    /// Mouse wheel scrolled over the surface.
    MouseWheel {
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
    },
    /// Key pressed while the surface is focused.
    KeyDown(SurfaceKeyData),
    /// Key released while the surface is focused.
    KeyUp(SurfaceKeyData),
    /// Text input while the surface is focused.
    TextInput(String),
    /// Mouse cursor entered the surface bounds.
    MouseEnter { x: f32, y: f32 },
    /// Mouse cursor left the surface bounds.
    MouseLeave,
    /// The surface gained keyboard focus.
    FocusGained,
    /// The surface lost keyboard focus.
    FocusLost,
}

/// Mouse button identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceMouseButton {
    Left,
    Right,
    Middle,
}

#[cfg(feature = "desktop")]
impl SurfaceMouseButton {
    /// Convert from platform MouseButton.
    pub fn from_platform(button: rinch_platform::MouseButton) -> Self {
        match button {
            rinch_platform::MouseButton::Left => Self::Left,
            rinch_platform::MouseButton::Right => Self::Right,
            rinch_platform::MouseButton::Middle => Self::Middle,
        }
    }
}

/// Keyboard event data for render surfaces.
#[derive(Debug, Clone)]
pub struct SurfaceKeyData {
    /// The logical key value (e.g., "a", "Enter", "Backspace").
    pub key: String,
    /// The physical key code (e.g., "KeyA", "Enter").
    pub code: String,
    /// Whether Ctrl/Cmd is pressed.
    pub ctrl: bool,
    /// Whether Shift is pressed.
    pub shift: bool,
    /// Whether Alt is pressed.
    pub alt: bool,
    /// Whether Meta/Super is pressed.
    pub meta: bool,
}

// ── SurfaceBuffer ────────────────────────────────────────────────────────────

/// Shared pixel buffer between the writer thread and the main thread.
pub(crate) struct SurfaceBuffer {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

// ── SurfaceWriter ────────────────────────────────────────────────────────────

/// Write handle for submitting frames from any thread.
///
/// This is `Send + Sync + Clone` so it can be sent to render threads,
/// worker pools, or async runtimes.
#[derive(Clone)]
pub struct SurfaceWriter {
    buffer: Arc<Mutex<SurfaceBuffer>>,
    needs_redraw: Arc<AtomicBool>,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    surface_id: usize,
}

impl SurfaceWriter {
    /// Submit a new RGBA frame. Non-blocking (locks a mutex briefly).
    ///
    /// `pixels` must be `width * height * 4` bytes of RGBA8 data.
    /// Wakes the event loop so the frame is composited on the next paint.
    pub fn submit_frame(&self, pixels: &[u8], width: u32, height: u32) {
        debug_assert_eq!(
            pixels.len(),
            (width * height * 4) as usize,
            "pixel buffer size mismatch: expected {}x{}x4={}, got {}",
            width,
            height,
            width * height * 4,
            pixels.len()
        );

        {
            let mut buf = self.buffer.lock().unwrap();
            buf.pixels.clear();
            buf.pixels.extend_from_slice(pixels);
            buf.width = width;
            buf.height = height;
        }

        self.needs_redraw.store(true, Ordering::Release);

        #[cfg(feature = "desktop")]
        {
            // Wake the event loop so it picks up the new frame
            crate::shell::rinch_runtime::run_on_main_thread(|| {});
        }

        #[cfg(target_arch = "wasm32")]
        {
            web_blit_surface(self.surface_id, &self.buffer);
        }
    }
}

// ── RenderSurfaceHandle ──────────────────────────────────────────────────────

/// Main-thread handle for a render surface.
///
/// Create with [`create_render_surface`]. Use [`writer`](Self::writer) to get
/// a thread-safe writer, and pass the handle to the [`RenderSurface`] component.
#[derive(Clone)]
pub struct RenderSurfaceHandle {
    /// Unique surface ID.
    pub(crate) id: usize,
    /// Shared pixel buffer (CPU path).
    pub(crate) buffer: Arc<Mutex<SurfaceBuffer>>,
    /// GPU texture source for zero-copy compositing (replaces pixel path when set).
    #[cfg(feature = "desktop")]
    pub(crate) texture_source: Arc<Mutex<Option<TextureSource>>>,
    /// Dirty flag (set by writer, cleared by frame collector).
    pub(crate) needs_redraw: Arc<AtomicBool>,
    /// Event handler (main-thread only).
    #[allow(clippy::type_complexity)]
    pub(crate) event_handler: std::rc::Rc<RefCell<Option<Box<dyn Fn(SurfaceEvent)>>>>,
    /// Viewport name for hole-punch compositing.
    pub(crate) viewport_name: String,
    /// Layout size in physical pixels, updated by the compositor each frame.
    pub(crate) layout_size: Arc<Mutex<(u32, u32)>>,
    /// The underlying canvas element (web only).
    #[cfg(target_arch = "wasm32")]
    pub(crate) canvas: std::rc::Rc<RefCell<Option<web_sys::HtmlCanvasElement>>>,
    /// The 2D rendering context for the canvas (web only).
    #[cfg(target_arch = "wasm32")]
    pub(crate) canvas_ctx: std::rc::Rc<RefCell<Option<web_sys::CanvasRenderingContext2d>>>,
    /// Per-frame render callback invoked each animation frame.
    #[allow(clippy::type_complexity)]
    pub(crate) render_callback:
        std::rc::Rc<RefCell<Option<Box<dyn FnMut(&SurfaceWriter, u32, u32)>>>>,
}

impl RenderSurfaceHandle {
    /// Get a thread-safe writer for submitting frames.
    pub fn writer(&self) -> SurfaceWriter {
        SurfaceWriter {
            buffer: self.buffer.clone(),
            needs_redraw: self.needs_redraw.clone(),
            surface_id: self.id,
        }
    }

    /// Set the event handler for mouse/keyboard events on this surface.
    ///
    /// The handler runs on the main thread. Only one handler per surface.
    pub fn set_event_handler(&self, handler: impl Fn(SurfaceEvent) + 'static) {
        *self.event_handler.borrow_mut() = Some(Box::new(handler));
    }

    /// Get the unique surface ID.
    pub fn id(&self) -> usize {
        self.id
    }

    /// Get the viewport name used for compositing.
    pub fn viewport_name(&self) -> &str {
        &self.viewport_name
    }

    /// Set a GPU texture as the frame source for zero-copy compositing.
    ///
    /// When set, the compositor reads this texture directly instead of uploading
    /// CPU pixel data. The texture must be created on the shared wgpu Device
    /// (available via [`super::shell::desktop::gpu_handle`]).
    ///
    /// Call this once at init and again whenever the texture is recreated
    /// (e.g., on viewport resize). The texture content is read each frame
    /// by the compositor — only the view reference needs to be set, not
    /// the pixel data.
    #[cfg(feature = "desktop")]
    pub fn set_texture_source(&self, view: wgpu::TextureView, width: u32, height: u32) {
        *self.texture_source.lock().unwrap() = Some(TextureSource {
            view,
            width,
            height,
        });
        self.needs_redraw.store(true, Ordering::Release);

        // Wake the event loop so it picks up the new texture
        crate::shell::rinch_runtime::run_on_main_thread(|| {});
    }

    /// Check if this surface has a GPU texture source set.
    #[cfg(feature = "desktop")]
    pub fn has_texture_source(&self) -> bool {
        self.texture_source.lock().unwrap().is_some()
    }

    /// Get the current layout size in physical pixels.
    ///
    /// Updated by the compositor each frame based on the actual DOM layout
    /// dimensions of this surface's element. Returns `(0, 0)` before the
    /// first paint.
    pub fn layout_size(&self) -> (u32, u32) {
        *self.layout_size.lock().unwrap()
    }

    /// Get a `Send + Sync` handle for registering GPU textures from background threads.
    ///
    /// This extracts the `Send`-able parts of `RenderSurfaceHandle` so a background
    /// renderer (e.g., a game engine on a worker thread) can call `set_texture_source`
    /// without holding the main-thread `Rc<RefCell<...>>` event handler field.
    #[cfg(feature = "desktop")]
    pub fn gpu_registrar(&self) -> GpuTextureRegistrar {
        GpuTextureRegistrar {
            texture_source: self.texture_source.clone(),
            needs_redraw: self.needs_redraw.clone(),
            layout_size: self.layout_size.clone(),
        }
    }

    /// Get the underlying canvas element (web only).
    ///
    /// Users can create a WebGPU or WebGL context on this canvas
    /// and render directly — the browser composites it in DOM order.
    #[cfg(target_arch = "wasm32")]
    pub fn canvas_element(&self) -> Option<web_sys::HtmlCanvasElement> {
        self.canvas.borrow().clone()
    }

    /// Set a per-frame render callback.
    ///
    /// The callback receives `(&SurfaceWriter, width, height)` and is invoked
    /// each animation frame. On desktop this happens in the paint cycle before
    /// frames are collected; on web it drives a `requestAnimationFrame` loop.
    ///
    /// Use this instead of spawning a thread — it works on both desktop and WASM.
    pub fn set_render_callback(&self, callback: impl FnMut(&SurfaceWriter, u32, u32) + 'static) {
        *self.render_callback.borrow_mut() = Some(Box::new(callback));

        #[cfg(target_arch = "wasm32")]
        start_raf_loop(self.id);
    }
}

// ── GpuTextureRegistrar (desktop only) ──────────────────────────────────────

/// A `Send + Sync` handle for registering GPU textures on a render surface from
/// a background thread.
///
/// Obtained via [`RenderSurfaceHandle::gpu_registrar`]. Wraps only the thread-safe
/// `Arc<Mutex<>>` fields — the main-thread `Rc` event handler is excluded.
#[cfg(feature = "desktop")]
#[derive(Clone)]
pub struct GpuTextureRegistrar {
    /// GPU texture source for zero-copy compositing.
    texture_source: Arc<Mutex<Option<TextureSource>>>,
    /// Dirty flag — set to wake the compositor.
    needs_redraw: Arc<AtomicBool>,
    /// Layout size in physical pixels, updated by compositor.
    layout_size: Arc<Mutex<(u32, u32)>>,
}

#[cfg(feature = "desktop")]
impl GpuTextureRegistrar {
    /// Register a GPU texture as the frame source for zero-copy compositing.
    ///
    /// Safe to call from any thread. Wakes the event loop via `run_on_main_thread`.
    pub fn set_texture_source(&self, view: wgpu::TextureView, width: u32, height: u32) {
        *self.texture_source.lock().unwrap() = Some(TextureSource {
            view,
            width,
            height,
        });
        self.needs_redraw.store(true, Ordering::Release);
        // Wake the event loop so it picks up the new texture.
        crate::shell::rinch_runtime::run_on_main_thread(|| {});
    }

    /// Signal the compositor that the texture content has been updated.
    ///
    /// Call this after each frame render. The engine writes new content to the
    /// same `TextureView` every frame — this wakes the event loop and triggers
    /// a direct window repaint (bypasses DOM resolution entirely).
    pub fn notify_frame_ready(&self) {
        self.needs_redraw.store(true, Ordering::Release);
        // Send SurfaceRedraw — goes directly to window.request_redraw()
        // without going through resolve_and_repaint().
        if let Some(proxy) = crate::shell::rinch_runtime::GLOBAL_PROXY.get() {
            let _ = proxy.send_event(crate::shell::rinch_runtime::RinchNativeEvent::SurfaceRedraw);
        }
    }

    /// Get the current layout size in physical pixels.
    ///
    /// Updated by the compositor each frame. Returns `(0, 0)` before
    /// the first paint.
    pub fn layout_size(&self) -> (u32, u32) {
        *self.layout_size.lock().unwrap()
    }
}

impl std::fmt::Debug for RenderSurfaceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderSurfaceHandle")
            .field("id", &self.id)
            .field("viewport_name", &self.viewport_name)
            .finish()
    }
}

// ── Thread-local registry ────────────────────────────────────────────────────

thread_local! {
    /// All active render surfaces.
    static SURFACE_REGISTRY: RefCell<Vec<RenderSurfaceHandle>> = const { RefCell::new(Vec::new()) };
    /// Currently focused surface ID (receives keyboard events).
    static FOCUSED_SURFACE: RefCell<Option<usize>> = const { RefCell::new(None) };
}

/// Create a new render surface and register it.
///
/// Returns a handle that should be passed to the [`RenderSurface`] component.
pub fn create_render_surface() -> RenderSurfaceHandle {
    let id = next_surface_id();
    let handle = RenderSurfaceHandle {
        id,
        buffer: Arc::new(Mutex::new(SurfaceBuffer {
            pixels: Vec::new(),
            width: 0,
            height: 0,
        })),
        #[cfg(feature = "desktop")]
        texture_source: Arc::new(Mutex::new(None)),
        needs_redraw: Arc::new(AtomicBool::new(false)),
        event_handler: std::rc::Rc::new(RefCell::new(None)),
        viewport_name: format!("__render_surface_{id}"),
        layout_size: Arc::new(Mutex::new((0, 0))),
        #[cfg(target_arch = "wasm32")]
        canvas: std::rc::Rc::new(RefCell::new(None)),
        #[cfg(target_arch = "wasm32")]
        canvas_ctx: std::rc::Rc::new(RefCell::new(None)),
        render_callback: std::rc::Rc::new(RefCell::new(None)),
    };

    SURFACE_REGISTRY.with(|reg| {
        reg.borrow_mut().push(handle.clone());
    });

    handle
}

/// Unregister a render surface by ID.
pub fn unregister_render_surface(id: usize) {
    SURFACE_REGISTRY.with(|reg| {
        reg.borrow_mut().retain(|s| s.id != id);
    });
    // Clear focus if this surface was focused
    FOCUSED_SURFACE.with(|f| {
        let mut f = f.borrow_mut();
        if *f == Some(id) {
            *f = None;
        }
    });
}

/// Check if any surface has a new frame waiting.
pub fn any_surface_dirty() -> bool {
    SURFACE_REGISTRY.with(|reg| {
        reg.borrow()
            .iter()
            .any(|s| s.needs_redraw.load(Ordering::Acquire))
    })
}

/// Collect frames from registered surfaces that use CPU pixel data.
///
/// Returns `(viewport_name, pixels, width, height)` for every surface with
/// a non-empty buffer that does NOT have a GPU texture source set.
/// Surfaces with a texture source are handled separately via
/// [`collect_texture_sources`].
/// Clears dirty flags as a side effect.
#[cfg(feature = "desktop")]
pub fn collect_surface_frames() -> Vec<(String, Vec<u8>, u32, u32)> {
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        let mut frames = Vec::new();
        for surface in reg.iter() {
            // Skip surfaces that use GPU texture source
            if surface.texture_source.lock().unwrap().is_some() {
                continue;
            }
            // Clear dirty flag (only used for triggering redraws)
            surface.needs_redraw.store(false, Ordering::Release);
            let buf = surface.buffer.lock().unwrap();
            if !buf.pixels.is_empty() {
                frames.push((
                    surface.viewport_name.clone(),
                    buf.pixels.clone(),
                    buf.width,
                    buf.height,
                ));
            }
        }
        frames
    })
}

/// Collect viewport names for surfaces that have a GPU texture source.
///
/// Returns `(viewport_name, texture_source_arc)` for each surface with a
/// texture source set. The compositor reads the `TextureView` directly from
/// the `Arc<Mutex<Option<TextureSource>>>` — no pixel upload needed.
/// Clears dirty flags as a side effect.
#[cfg(feature = "desktop")]
pub fn collect_texture_sources() -> Vec<(String, Arc<Mutex<Option<TextureSource>>>)> {
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        let mut sources = Vec::new();
        for surface in reg.iter() {
            if surface.texture_source.lock().unwrap().is_some() {
                surface.needs_redraw.store(false, Ordering::Release);
                sources.push((
                    surface.viewport_name.clone(),
                    surface.texture_source.clone(),
                ));
            }
        }
        sources
    })
}

/// Update the layout size for a render surface by viewport name.
///
/// Called by the compositor each frame after resolving layout. The size is in
/// physical pixels (logical × scale_factor).
pub fn update_layout_size(viewport_name: &str, width: u32, height: u32) {
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        for surface in reg.iter() {
            if surface.viewport_name == viewport_name {
                *surface.layout_size.lock().unwrap() = (width, height);
                return;
            }
        }
    });
}

/// Check if any render surfaces are registered (even if they haven't submitted frames yet).
pub fn any_surfaces_registered() -> bool {
    SURFACE_REGISTRY.with(|reg| !reg.borrow().is_empty())
}

/// Invoke render callbacks on all registered surfaces that have one set.
///
/// Called once per frame on desktop (before `collect_surface_frames`).
/// On web, each surface with a callback drives its own `requestAnimationFrame` loop instead.
pub fn invoke_render_callbacks() {
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        for surface in reg.iter() {
            let mut cb = surface.render_callback.borrow_mut();
            if let Some(ref mut callback) = *cb {
                let (w, h) = *surface.layout_size.lock().unwrap();
                if w == 0 || h == 0 {
                    continue; // not yet measured
                }
                let writer = SurfaceWriter {
                    buffer: surface.buffer.clone(),
                    needs_redraw: surface.needs_redraw.clone(),
                    surface_id: surface.id,
                };
                callback(&writer, w, h);
            }
        }
    });
}

/// Dispatch a surface event to the handler of the surface with the given ID.
pub fn dispatch_surface_event(id: usize, event: SurfaceEvent) {
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        if let Some(surface) = reg.iter().find(|s| s.id == id) {
            if let Some(ref handler) = *surface.event_handler.borrow() {
                handler(event);
            }
        }
    });
}

/// Set the currently focused render surface.
pub fn set_focused_surface(id: Option<usize>) {
    let old = focused_surface_id();
    FOCUSED_SURFACE.with(|f| {
        *f.borrow_mut() = id;
    });
    // Dispatch focus events
    if old != id {
        if let Some(old_id) = old {
            dispatch_surface_event(old_id, SurfaceEvent::FocusLost);
        }
        if let Some(new_id) = id {
            dispatch_surface_event(new_id, SurfaceEvent::FocusGained);
        }
    }
}

/// Get the currently focused render surface ID.
pub fn focused_surface_id() -> Option<usize> {
    FOCUSED_SURFACE.with(|f| *f.borrow())
}

// ── RenderSurface component ──────────────────────────────────────────────────

/// Component that renders an external pixel source into the layout.
///
/// Place inside a sized container. The surface fills its parent and
/// composites the pixel data submitted via [`SurfaceWriter`].
///
/// # Example
///
/// ```ignore
/// let surface = create_render_surface();
/// rsx! {
///     div { style: "width: 640px; height: 480px;",
///         RenderSurface { surface: surface }
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct RenderSurface {
    /// The surface handle to render.
    pub surface: Option<RenderSurfaceHandle>,
}

impl Component for RenderSurface {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let div = scope.create_element("div");

            if let Some(ref surface) = self.surface {
                div.set_attribute("data-viewport", &surface.viewport_name);
                div.set_attribute("data-render-surface", &surface.id.to_string());
            }

            // Transparent background so the composited layer pixels show through
            // (hole-punch pattern: Vello UI is alpha-blended over the layer content).
            div.set_attribute("style", "width: 100%; height: 100%;");

            div
        }

        #[cfg(target_arch = "wasm32")]
        {
            let canvas = scope.create_element("canvas");
            canvas.set_attribute("style", "width: 100%; height: 100%; display: block;");

            if let Some(ref surface) = self.surface {
                let canvas_id = format!("rinch-surface-{}", surface.id);
                canvas.set_attribute("id", &canvas_id);
                canvas.set_attribute("data-render-surface", &surface.id.to_string());
                // Defer canvas context acquisition to after DOM mount
                schedule_canvas_init(surface.clone());
            }

            canvas
        }
    }
}

// ── Web-specific support ─────────────────────────────────────────────────────

/// Start a `requestAnimationFrame` loop that invokes the render callback for
/// the given surface each frame. The loop self-terminates when the surface is
/// unregistered or its callback is removed.
#[cfg(target_arch = "wasm32")]
fn start_raf_loop(surface_id: usize) {
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;

    let closure: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
    let closure_clone = closure.clone();

    *closure.borrow_mut() = Some(Closure::wrap(Box::new(move || {
        let should_continue = SURFACE_REGISTRY.with(|reg| {
            let reg = reg.borrow();
            if let Some(surface) = reg.iter().find(|s| s.id == surface_id) {
                let mut cb = surface.render_callback.borrow_mut();
                if let Some(ref mut callback) = *cb {
                    let (w, h) = *surface.layout_size.lock().unwrap();
                    if w > 0 && h > 0 {
                        let writer = SurfaceWriter {
                            buffer: surface.buffer.clone(),
                            needs_redraw: surface.needs_redraw.clone(),
                            surface_id: surface.id,
                        };
                        callback(&writer, w, h);
                    }
                    true // callback exists, keep looping
                } else {
                    false // callback removed, stop
                }
            } else {
                false // surface unregistered, stop
            }
        });

        if should_continue {
            let window = web_sys::window().unwrap();
            let cb_ref = closure_clone.borrow();
            if let Some(ref cb) = *cb_ref {
                let _ = window.request_animation_frame(cb.as_ref().unchecked_ref());
            }
        }
    }) as Box<dyn FnMut()>));

    // Kick off the first frame
    {
        let window = web_sys::window().unwrap();
        let cb_ref = closure.borrow();
        if let Some(ref cb) = *cb_ref {
            let _ = window.request_animation_frame(cb.as_ref().unchecked_ref());
        }
    }

    // Keep the closure alive — it self-references via Rc and stops when done
    std::mem::forget(closure);
}

#[cfg(target_arch = "wasm32")]
fn web_blit_surface(surface_id: usize, buffer: &Arc<Mutex<SurfaceBuffer>>) {
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        if let Some(surface) = reg.iter().find(|s| s.id == surface_id) {
            let canvas = surface.canvas.borrow();
            let ctx = surface.canvas_ctx.borrow();
            if let (Some(canvas), Some(ctx)) = (canvas.as_ref(), ctx.as_ref()) {
                let buf = buffer.lock().unwrap();
                if buf.pixels.is_empty() {
                    return;
                }
                // Resize canvas bitmap if dimensions changed
                if canvas.width() != buf.width || canvas.height() != buf.height {
                    canvas.set_width(buf.width);
                    canvas.set_height(buf.height);
                }
                let clamped = wasm_bindgen::Clamped(&buf.pixels[..]);
                if let Ok(img) = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
                    clamped, buf.width, buf.height,
                ) {
                    let _ = ctx.put_image_data(&img, 0.0, 0.0);
                }
            }
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn schedule_canvas_init(surface: RenderSurfaceHandle) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    let closure = Closure::once(move || {
        let document = web_sys::window().unwrap().document().unwrap();
        let canvas_id = format!("rinch-surface-{}", surface.id);
        if let Some(el) = document.get_element_by_id(&canvas_id) {
            let canvas_el: web_sys::HtmlCanvasElement = el.dyn_into().unwrap();
            let ctx = canvas_el
                .get_context("2d")
                .unwrap()
                .unwrap()
                .dyn_into::<web_sys::CanvasRenderingContext2d>()
                .unwrap();
            // Set up event listeners before storing refs
            setup_canvas_events(&canvas_el, surface.id);
            setup_resize_observer(&canvas_el, surface.id);
            // Store canvas refs for blitting
            *surface.canvas.borrow_mut() = Some(canvas_el);
            *surface.canvas_ctx.borrow_mut() = Some(ctx);
        }
    });
    let window = web_sys::window().unwrap();
    window.queue_microtask(closure.as_ref().unchecked_ref());
    closure.forget();
}

#[cfg(target_arch = "wasm32")]
fn mouse_button_from_i16(button: i16) -> SurfaceMouseButton {
    match button {
        0 => SurfaceMouseButton::Left,
        1 => SurfaceMouseButton::Middle,
        2 => SurfaceMouseButton::Right,
        _ => SurfaceMouseButton::Left,
    }
}

#[cfg(target_arch = "wasm32")]
fn setup_canvas_events(canvas: &web_sys::HtmlCanvasElement, surface_id: usize) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    let target: &web_sys::EventTarget = canvas.as_ref();

    // mousedown
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
            event.stop_propagation();
            let x = event.offset_x() as f32;
            let y = event.offset_y() as f32;
            let button = mouse_button_from_i16(event.button());
            set_focused_surface(Some(surface_id));
            dispatch_surface_event(surface_id, SurfaceEvent::MouseDown { x, y, button });
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback("mousedown", closure.as_ref().unchecked_ref())
            .unwrap();
        closure.forget();
    }

    // mouseup
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
            let x = event.offset_x() as f32;
            let y = event.offset_y() as f32;
            let button = mouse_button_from_i16(event.button());
            dispatch_surface_event(surface_id, SurfaceEvent::MouseUp { x, y, button });
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback("mouseup", closure.as_ref().unchecked_ref())
            .unwrap();
        closure.forget();
    }

    // mousemove
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
            let x = event.offset_x() as f32;
            let y = event.offset_y() as f32;
            dispatch_surface_event(surface_id, SurfaceEvent::MouseMove { x, y });
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback("mousemove", closure.as_ref().unchecked_ref())
            .unwrap();
        closure.forget();
    }

    // mouseenter
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
            let x = event.offset_x() as f32;
            let y = event.offset_y() as f32;
            dispatch_surface_event(surface_id, SurfaceEvent::MouseEnter { x, y });
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback("mouseenter", closure.as_ref().unchecked_ref())
            .unwrap();
        closure.forget();
    }

    // mouseleave
    {
        let closure = Closure::wrap(Box::new(move |_event: web_sys::MouseEvent| {
            dispatch_surface_event(surface_id, SurfaceEvent::MouseLeave);
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback("mouseleave", closure.as_ref().unchecked_ref())
            .unwrap();
        closure.forget();
    }

    // wheel
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::WheelEvent| {
            event.prevent_default();
            event.stop_propagation();
            let mouse: &web_sys::MouseEvent = event.as_ref();
            let x = mouse.offset_x() as f32;
            let y = mouse.offset_y() as f32;
            let delta_x = event.delta_x() as f32;
            let delta_y = event.delta_y() as f32;
            dispatch_surface_event(
                surface_id,
                SurfaceEvent::MouseWheel {
                    x,
                    y,
                    delta_x,
                    delta_y,
                },
            );
        }) as Box<dyn FnMut(_)>);
        // Use non-passive listener so we can preventDefault on wheel
        let opts = web_sys::AddEventListenerOptions::new();
        opts.set_passive(false);
        target
            .add_event_listener_with_callback_and_add_event_listener_options(
                "wheel",
                closure.as_ref().unchecked_ref(),
                &opts,
            )
            .unwrap();
        closure.forget();
    }
}

#[cfg(target_arch = "wasm32")]
fn setup_resize_observer(canvas: &web_sys::HtmlCanvasElement, surface_id: usize) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    let callback = Closure::wrap(Box::new(move |entries: js_sys::Array, _observer: JsValue| {
        if let Some(entry) = entries.get(0).dyn_ref::<web_sys::ResizeObserverEntry>() {
            let rect = entry.content_rect();
            let width = rect.width() as u32;
            let height = rect.height() as u32;
            if width > 0 && height > 0 {
                // Update layout_size via viewport name lookup
                let viewport_name = format!("__render_surface_{surface_id}");
                update_layout_size(&viewport_name, width, height);
            }
        }
    }) as Box<dyn FnMut(js_sys::Array, JsValue)>);

    let observer = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()).unwrap();
    observer.observe(canvas);
    callback.forget();
    // Keep observer alive by leaking it — canvas lifetime = app lifetime
    std::mem::forget(observer);
}
