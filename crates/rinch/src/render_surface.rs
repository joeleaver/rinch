//! Embed external renderers into the rinch layout.
//!
//! `RenderSurface` lets you feed raw RGBA pixels from any source (game engine,
//! terminal emulator, video decoder, custom GPU renderer) into a rinch layout.
//! On desktop, rinch composites the pixels via a hole-punch + WGSL compositor.
//! On web, a native `<canvas>` element is used and the browser handles compositing.
//!
//! # CPU Pixel Rendering
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
//!
//! # GPU Rendering
//!
//! On **desktop**, use [`RenderSurfaceHandle::set_texture_source`] or
//! [`GpuTextureRegistrar`] to register a wgpu texture for zero-copy compositing.
//!
//! On **web**, call [`RenderSurfaceHandle::canvas_element`] to get the underlying
//! `<canvas>` and create a WebGPU or WebGL context on it. The 2D context for CPU
//! blitting is created lazily on the first [`SurfaceWriter::submit_frame`] call,
//! so creating a GPU context first prevents it from being claimed. Events, layout
//! size, and resize observation work regardless of context type.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

// ── Direct redraw callback ──────────────────────────────────────────────────

/// Global callback that calls `window.request_redraw()` directly.
///
/// Updated by the runtime whenever the window is (re)created. This lets render
/// surfaces trigger a repaint from any thread without routing through the
/// event loop queue, eliminating the extra event-loop iteration of latency.
#[cfg(feature = "desktop")]
static REDRAW_CALLBACK: Mutex<Option<Arc<dyn Fn() + Send + Sync>>> = Mutex::new(None);

/// Register the direct redraw callback. Called by the runtime on window create.
#[cfg(feature = "desktop")]
pub(crate) fn set_redraw_callback(cb: Arc<dyn Fn() + Send + Sync>) {
    *REDRAW_CALLBACK.lock().unwrap() = Some(cb);
}

/// Clear the redraw callback (e.g., when the window is hidden/destroyed).
#[cfg(feature = "desktop")]
pub(crate) fn clear_redraw_callback() {
    *REDRAW_CALLBACK.lock().unwrap() = None;
}

/// Request a window repaint directly, bypassing the event loop queue.
#[cfg(feature = "desktop")]
fn request_repaint() {
    // Suppress during invoke_render_callbacks — the paint cycle is already
    // in progress, so requesting another redraw would create an infinite loop.
    if IN_RENDER_CALLBACK.with(|f| f.get()) {
        return;
    }
    if let Some(cb) = REDRAW_CALLBACK.lock().unwrap().as_ref() {
        cb();
    }
}

#[cfg(feature = "desktop")]
thread_local! {
    /// Guard flag: true while `invoke_render_callbacks` is running.
    static IN_RENDER_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

// ── TextureSource (desktop only) ────────────────────────────────────────────

/// A GPU texture source for zero-copy compositing.
///
/// When set on a [`RenderSurfaceHandle`], the compositor reads this texture
/// directly instead of uploading CPU pixel data. The texture must be created
/// on the same wgpu Device (available via [`super::shell::desktop::gpu_handle`]).
#[cfg(feature = "gpu")]
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
            // Request a repaint directly — calls window.request_redraw() without
            // routing through the event loop queue. This eliminates a full
            // event-loop round-trip of latency for high-performance surfaces.
            request_repaint();
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
    #[cfg(feature = "gpu")]
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
    #[cfg(feature = "gpu")]
    pub fn set_texture_source(&self, view: wgpu::TextureView, width: u32, height: u32) {
        *self.texture_source.lock().unwrap() = Some(TextureSource {
            view,
            width,
            height,
        });
        self.needs_redraw.store(true, Ordering::Release);

        // Request repaint so the compositor picks up the new texture
        request_repaint();
    }

    /// Check if this surface has a GPU texture source set.
    #[cfg(feature = "gpu")]
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
    #[cfg(feature = "gpu")]
    pub fn gpu_registrar(&self) -> GpuTextureRegistrar {
        GpuTextureRegistrar {
            texture_source: self.texture_source.clone(),
            needs_redraw: self.needs_redraw.clone(),
            layout_size: self.layout_size.clone(),
        }
    }

    /// Get the underlying canvas element (web only).
    ///
    /// Returns `None` before the component is mounted in the DOM.
    ///
    /// For GPU rendering, create a WebGPU or WebGL context on this canvas
    /// **before** calling [`SurfaceWriter::submit_frame`]. The first
    /// `submit_frame` call lazily creates a 2D context for CPU blitting;
    /// if a GPU context already exists, CPU blitting is skipped and the
    /// user's GPU rendering takes over. The browser composites the canvas
    /// in DOM order — no rinch compositor involvement needed.
    ///
    /// Events, layout size, and the resize observer work regardless of
    /// which context type is used.
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
#[cfg(feature = "gpu")]
#[derive(Clone)]
pub struct GpuTextureRegistrar {
    /// GPU texture source for zero-copy compositing.
    texture_source: Arc<Mutex<Option<TextureSource>>>,
    /// Dirty flag — set to wake the compositor.
    needs_redraw: Arc<AtomicBool>,
    /// Layout size in physical pixels, updated by compositor.
    layout_size: Arc<Mutex<(u32, u32)>>,
}

#[cfg(feature = "gpu")]
impl GpuTextureRegistrar {
    /// Register a GPU texture as the frame source for zero-copy compositing.
    ///
    /// Safe to call from any thread. Requests a repaint directly.
    pub fn set_texture_source(&self, view: wgpu::TextureView, width: u32, height: u32) {
        *self.texture_source.lock().unwrap() = Some(TextureSource {
            view,
            width,
            height,
        });
        self.needs_redraw.store(true, Ordering::Release);
        request_repaint();
    }

    /// Signal the compositor that the texture content has been updated.
    ///
    /// Call this after each frame render. The engine writes new content to the
    /// same `TextureView` every frame — this calls `window.request_redraw()`
    /// directly without routing through the event loop queue.
    pub fn notify_frame_ready(&self) {
        self.needs_redraw.store(true, Ordering::Release);
        request_repaint();
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
    /// Mounted render surfaces — only surfaces with a live DOM element.
    ///
    /// Surfaces are added when their [`RenderSurface`] component mounts and
    /// removed when the component's scope is disposed (tab switch, conditional
    /// hide, etc.). This ensures the compositor, render callbacks, and event
    /// dispatch only operate on surfaces that are actually visible.
    static SURFACE_REGISTRY: RefCell<Vec<RenderSurfaceHandle>> = const { RefCell::new(Vec::new()) };
    /// Currently focused surface ID (receives keyboard events).
    static FOCUSED_SURFACE: RefCell<Option<usize>> = const { RefCell::new(None) };
}

/// Create a new render surface.
///
/// Returns a handle that should be passed to the [`RenderSurface`] component.
/// The surface is **not** registered for compositing until the component mounts.
/// Writers and GPU registrars can be obtained immediately and will buffer data
/// until the surface becomes visible.
pub fn create_render_surface() -> RenderSurfaceHandle {
    let id = next_surface_id();
    RenderSurfaceHandle {
        id,
        buffer: Arc::new(Mutex::new(SurfaceBuffer {
            pixels: Vec::new(),
            width: 0,
            height: 0,
        })),
        #[cfg(feature = "gpu")]
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
    }
}

/// Create a render surface with a specific viewport name and auto-register it.
///
/// Used internally to bridge video players to the RenderSurface compositing
/// pipeline. The viewport name must match the `data-viewport` attribute on
/// the corresponding DOM element (e.g., `VideoViewport`).
///
/// Unlike [`create_render_surface`], this auto-registers because video surfaces
/// bypass the [`RenderSurface`] component (they use `VideoViewport` + a raw
/// `SurfaceWriter` instead).
pub fn create_render_surface_with_name(viewport_name: &str) -> RenderSurfaceHandle {
    let id = next_surface_id();
    let handle = RenderSurfaceHandle {
        id,
        buffer: Arc::new(Mutex::new(SurfaceBuffer {
            pixels: Vec::new(),
            width: 0,
            height: 0,
        })),
        #[cfg(feature = "gpu")]
        texture_source: Arc::new(Mutex::new(None)),
        needs_redraw: Arc::new(AtomicBool::new(false)),
        event_handler: std::rc::Rc::new(RefCell::new(None)),
        viewport_name: viewport_name.to_string(),
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

/// Register a surface as mounted (visible in the DOM).
///
/// Called by [`RenderSurface::render`] when the component mounts. If the
/// surface is already registered (e.g., re-mounted), this is a no-op.
pub fn mount_render_surface(handle: &RenderSurfaceHandle) {
    SURFACE_REGISTRY.with(|reg| {
        let mut reg = reg.borrow_mut();
        // Avoid duplicate registration (e.g., if same handle passed to two components)
        if !reg.iter().any(|s| s.id == handle.id) {
            reg.push(handle.clone());
        }
    });
}

/// Unregister a render surface by ID (unmount).
///
/// Called when the [`RenderSurface`] component's scope is disposed.
/// Removes the surface from the mounted registry so the compositor
/// stops collecting its frames and invoking its render callback.
/// Also clears focus if this surface was focused.
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
            #[cfg(feature = "gpu")]
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
#[cfg(feature = "gpu")]
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

/// Check if any render surfaces are currently mounted (have a live DOM element).
pub fn any_surfaces_registered() -> bool {
    SURFACE_REGISTRY.with(|reg| !reg.borrow().is_empty())
}

/// Return the viewport names of all registered render surfaces.
///
/// Used by the desktop compositor to look up layout rects and update
/// `layout_size` before invoking render callbacks.
pub fn registered_viewport_names() -> Vec<String> {
    SURFACE_REGISTRY.with(|reg| {
        reg.borrow()
            .iter()
            .map(|s| s.viewport_name.clone())
            .collect()
    })
}

/// Invoke render callbacks on all registered surfaces that have one set.
///
/// Called once per frame on desktop (before `collect_surface_frames`).
/// On web, each surface with a callback drives its own `requestAnimationFrame` loop instead.
pub fn invoke_render_callbacks() {
    // Set guard so submit_frame() inside callbacks won't call request_repaint()
    // — we're already inside a paint cycle.
    #[cfg(feature = "desktop")]
    IN_RENDER_CALLBACK.with(|f| f.set(true));

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

    #[cfg(feature = "desktop")]
    IN_RENDER_CALLBACK.with(|f| f.set(false));
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
        if let Some(ref surface) = self.surface {
            // Mount: register the surface so the compositor, render callbacks,
            // and event dispatch can find it.
            mount_render_surface(surface);

            // Unmount: unregister when this scope is disposed (tab switch,
            // conditional hide, etc.) so stale surfaces aren't composited.
            let surface_id = surface.id;
            scope.on_cleanup(move || {
                unregister_render_surface(surface_id);
            });
        }

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
            let Some(canvas) = canvas.as_ref() else {
                return;
            };

            // Lazily create the 2D context on first CPU blit.
            // If the user has already created a WebGPU/WebGL context on this
            // canvas (via `canvas_element()`), `getContext("2d")` returns None
            // and we skip CPU blitting — the user is rendering via GPU instead.
            let mut ctx_ref = surface.canvas_ctx.borrow_mut();
            if ctx_ref.is_none() {
                use wasm_bindgen::JsCast;
                match canvas.get_context("2d") {
                    Ok(Some(ctx)) => {
                        *ctx_ref =
                            Some(ctx.dyn_into::<web_sys::CanvasRenderingContext2d>().unwrap());
                    }
                    _ => return, // Canvas has a GPU context — skip CPU blit
                }
            }

            let Some(ctx) = ctx_ref.as_ref() else {
                return;
            };
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
            if let Ok(img) =
                web_sys::ImageData::new_with_u8_clamped_array_and_sh(clamped, buf.width, buf.height)
            {
                let _ = ctx.put_image_data(&img, 0.0, 0.0);
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
            // Set up event listeners before storing refs
            setup_canvas_events(&canvas_el, surface.id);
            setup_resize_observer(&canvas_el, surface.id);
            // Store canvas ref. The 2D context is created lazily on the first
            // submit_frame() call. This allows users to call canvas_element()
            // and create a WebGPU or WebGL context first for GPU rendering.
            *surface.canvas.borrow_mut() = Some(canvas_el);
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
