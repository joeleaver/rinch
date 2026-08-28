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

/// A GPU texture source for compositing or readback.
///
/// When set on a [`RenderSurfaceHandle`], the runtime reads this texture
/// each frame. For inline-paint surfaces (RenderSurface), the texture is
/// read back to CPU pixels for inline painting. The texture must be created
/// on the same wgpu Device (available via [`super::shell::desktop::gpu_handle`]).
#[cfg(feature = "gpu")]
pub struct TextureSource {
    /// The underlying texture (needed for GPU→CPU readback).
    pub texture: wgpu::Texture,
    /// The texture view (used by the compositor for video/GameViewport).
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

    // ── Drag-and-drop events ─────────────────────────────────────────
    // Fired when a DOM drag (from a `draggable="true"` element) interacts
    // with this surface. Access the dragged data via your `DragContext<T>`.
    /// A drag entered the surface bounds.
    DragEnter { x: f32, y: f32 },
    /// A drag is moving over the surface (fires every mouse move).
    DragOver { x: f32, y: f32 },
    /// A drag left the surface bounds.
    DragLeave,
    /// A drag was dropped on the surface.
    Drop { x: f32, y: f32 },
}

/// Mouse button identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceMouseButton {
    Left,
    Right,
    Middle,
}

#[cfg(any(feature = "desktop", feature = "android", feature = "embed"))]
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
    /// Whether this surface carries decoded **video** frames.
    ///
    /// Video and `GameViewport` both arrive through
    /// [`create_render_surface_with_name`]-shaped construction and both stamp a
    /// `data-viewport` attribute, so the name cannot tell them apart — and
    /// renaming video's surface would silently reroute `GameViewport` with it.
    /// This flag is set at video's registration site and nowhere else (issue
    /// #358): on the software backend video paints inline, at its own z-order,
    /// while a `GameViewport` keeps the compositor blit it has always had.
    pub(crate) is_video: bool,
    /// Layout size in physical pixels, updated by the compositor each frame.
    pub(crate) layout_size: Arc<Mutex<(u32, u32)>>,
    /// Layout position in logical pixels (window coordinates), updated each frame.
    pub(crate) layout_position: Arc<Mutex<(f32, f32)>>,
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
    /// Resize notification callback (main-thread only).
    ///
    /// Invoked whenever the surface's backing size changes, with the new size in
    /// **physical pixels** — on web `CSS px × devicePixelRatio` (from the
    /// `ResizeObserver`), on desktop `logical × scale_factor` (from the
    /// compositor's per-frame layout update). Lets a GPU app reconfigure its
    /// wgpu/WebGL surface on resize.
    #[allow(clippy::type_complexity)]
    pub(crate) resize_callback: std::rc::Rc<RefCell<Option<Box<dyn FnMut(u32, u32)>>>>,
    /// Web teardown state — the `ResizeObserver` and canvas event listeners, kept
    /// alive here (not leaked) so they can be disconnected/removed on unmount.
    #[cfg(target_arch = "wasm32")]
    pub(crate) web_cleanup: std::rc::Rc<RefCell<Option<WebSurfaceCleanup>>>,
    /// Whether a `requestAnimationFrame` loop is currently driving this surface's
    /// render callback (web only). Prevents double-starting a loop and lets a
    /// remount restart one after the previous loop self-terminated on unmount.
    #[cfg(target_arch = "wasm32")]
    pub(crate) raf_running: std::rc::Rc<Cell<bool>>,
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

    /// Set a GPU texture as the frame source.
    ///
    /// The runtime reads this texture each frame — for RenderSurface components,
    /// the pixels are read back to CPU for inline painting. The texture must be
    /// created on the shared wgpu Device (via [`super::shell::desktop::gpu_handle`]).
    ///
    /// Call this once at init and again whenever the texture is recreated
    /// (e.g., on viewport resize).
    #[cfg(feature = "gpu")]
    pub fn set_texture_source(
        &self,
        texture: wgpu::Texture,
        view: wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        *self.texture_source.lock().unwrap() = Some(TextureSource {
            texture,
            view,
            width,
            height,
        });
        self.needs_redraw.store(true, Ordering::Release);
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

    /// Get the surface position and size in window coordinates.
    ///
    /// Returns `(x, y, width, height)` where `(x, y)` is the top-left corner
    /// in logical pixels relative to the window, and `(width, height)` is in
    /// physical pixels. Returns `(0.0, 0.0, 0, 0)` before the first paint.
    pub fn layout_rect(&self) -> (f32, f32, u32, u32) {
        let (x, y) = *self.layout_position.lock().unwrap();
        let (w, h) = *self.layout_size.lock().unwrap();
        (x, y, w, h)
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
        start_raf_loop(self.id, self.raf_running.clone());
    }

    /// Set a callback invoked whenever the surface's backing size changes.
    ///
    /// The size is reported in **physical pixels** (web: `CSS px ×
    /// devicePixelRatio`; desktop: `logical × scale_factor`), matching
    /// [`layout_size`](Self::layout_size). Use it to reconfigure a GPU
    /// (wgpu / WebGL) surface on resize — HiDPI-correct out of the box.
    ///
    /// The callback runs on the main thread. On web it fires once with the
    /// initial size shortly after mount (set it before/at mount to catch that
    /// first call), and again on every subsequent resize. Only one callback per
    /// surface.
    pub fn set_resize_callback(&self, callback: impl FnMut(u32, u32) + 'static) {
        *self.resize_callback.borrow_mut() = Some(Box::new(callback));
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
    /// Register a GPU texture as the frame source.
    ///
    /// Safe to call from any thread. Requests a repaint directly.
    pub fn set_texture_source(
        &self,
        texture: wgpu::Texture,
        view: wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        *self.texture_source.lock().unwrap() = Some(TextureSource {
            texture,
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
            .field("is_video", &self.is_video)
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
    new_surface_handle(id, format!("__render_surface_{id}"), false)
}

/// The one place a [`RenderSurfaceHandle`] is built.
///
/// Every entry point funnels through here so a field added to the handle —
/// `is_video` was the latest — cannot be wired up in one constructor and
/// forgotten in the other.
fn new_surface_handle(id: usize, viewport_name: String, is_video: bool) -> RenderSurfaceHandle {
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
        viewport_name,
        is_video,
        layout_size: Arc::new(Mutex::new((0, 0))),
        layout_position: Arc::new(Mutex::new((0.0, 0.0))),
        #[cfg(target_arch = "wasm32")]
        canvas: std::rc::Rc::new(RefCell::new(None)),
        #[cfg(target_arch = "wasm32")]
        canvas_ctx: std::rc::Rc::new(RefCell::new(None)),
        render_callback: std::rc::Rc::new(RefCell::new(None)),
        resize_callback: std::rc::Rc::new(RefCell::new(None)),
        #[cfg(target_arch = "wasm32")]
        web_cleanup: std::rc::Rc::new(RefCell::new(None)),
        #[cfg(target_arch = "wasm32")]
        raf_running: std::rc::Rc::new(Cell::new(false)),
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
    create_named_surface(viewport_name, false)
}

/// Create the render surface a **video player** delivers decoded frames into.
///
/// Identical to [`create_render_surface_with_name`] except that the surface is
/// marked as carrying video, which is what routes it away from the compositor
/// blit on the software backend (issue #358). Separate entry point rather than
/// a name convention: `GameViewport` shares
/// [`create_render_surface_with_name`], so a naming rule would reroute it too.
pub fn create_video_surface(viewport_name: &str) -> RenderSurfaceHandle {
    create_named_surface(viewport_name, true)
}

fn create_named_surface(viewport_name: &str, is_video: bool) -> RenderSurfaceHandle {
    let id = next_surface_id();
    let handle = new_surface_handle(id, viewport_name.to_string(), is_video);

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

/// Check if a surface uses inline painting (RenderSurface component)
/// vs compositor/hole-punch (video, GameViewport).
///
/// Surfaces created by `create_render_surface()` have auto-generated names
/// starting with `__render_surface_` and paint inline. Surfaces created by
/// `create_render_surface_with_name()` have custom names and use the compositor.
///
/// Every caller is behind `desktop` or `gpu` (which implies `desktop`), so this is
/// dead code on a wasm build — gate it the same way rather than warn there.
#[cfg(feature = "desktop")]
fn is_inline_surface(surface: &RenderSurfaceHandle) -> bool {
    surface.viewport_name.starts_with("__render_surface_")
}

/// Whether a surface belongs on the **compositor** path — a layer on GPU, a
/// post-paint blit on software — rather than being painted inline during
/// `paint_document`.
///
/// The whole routing table, in one place, decided by backend × purpose:
///
/// | | `RenderSurface` | video | `GameViewport` |
/// |---|---|---|---|
/// | software | inline | inline (#358) | compositor blit |
/// | GPU | inline | compositor + backdrop (#354) | compositor |
///
/// Software blits its compositor frames onto the *finished* pixel buffer, after
/// the whole UI has been painted and clipped only by the viewport's
/// overflow-clipping ancestors — a write with no notion of occlusion, which
/// destroyed every overlay above a playing video. GPU has no such problem: its
/// layers are blitted first and the Vello UI alpha-blends on top, so an opaque
/// drawer already covers the video there.
///
/// A plain `const fn` of three booleans rather than a `cfg`-gated branch, so
/// both columns of the table stay reachable to tests whichever backend the
/// crate was built for.
///
/// Gated like `is_inline_surface`: nothing on a wasm build has a compositor to
/// route to.
#[cfg(feature = "desktop")]
pub(crate) const fn surface_takes_compositor_path(
    is_inline: bool,
    is_video: bool,
    gpu: bool,
) -> bool {
    !is_inline && (gpu || !is_video)
}

/// Collect frames from registered surfaces that use the compositor path
/// (video, GameViewport — NOT RenderSurface components).
///
/// Returns `(viewport_name, pixels, width, height)` for every compositor
/// surface with a non-empty buffer that does NOT have a GPU texture source set.
/// Clears dirty flags as a side effect.
#[cfg(feature = "desktop")]
pub fn collect_surface_frames() -> Vec<(String, Vec<u8>, u32, u32)> {
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        let mut frames = Vec::new();
        for surface in reg.iter() {
            // Skip anything that paints inline: a `RenderSurface` component on
            // both backends, plus video on software (#358).
            if !surface_takes_compositor_path(
                is_inline_surface(surface),
                surface.is_video,
                // "has a GPU compositor", not "has the `gpu` feature": the
                // Android shell picks its painter with `android-gpu`, and both
                // are what the `software_shell` alias is the negation of. With
                // `cfg!(feature = "gpu")` an `android-gpu` build would route
                // video off the compositor with nothing painting it inline.
                cfg!(not(software_shell)),
            ) {
                continue;
            }
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

/// Collect surface pixel data keyed by surface ID for inline painting.
///
/// Returns a HashMap suitable for passing to `rinch_dom::paint::set_surface_pixels()`.
/// Only includes RenderSurface components (not video/GameViewport).
/// For GPU texture surfaces, call [`readback_gpu_textures`] first to
/// populate the CPU buffers.
/// Clears dirty flags as a side effect.
#[cfg(feature = "desktop")]
pub fn collect_surface_pixels_by_id()
-> std::collections::HashMap<usize, rinch_dom::paint::SurfacePixelData> {
    use std::collections::HashMap;
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        let mut map = HashMap::new();
        for surface in reg.iter() {
            // Only collect inline-paint surfaces (RenderSurface components)
            if !is_inline_surface(surface) {
                continue;
            }
            surface.needs_redraw.store(false, Ordering::Release);
            let buf = surface.buffer.lock().unwrap();
            if !buf.pixels.is_empty() {
                map.insert(
                    surface.id,
                    rinch_dom::paint::SurfacePixelData {
                        data: buf.pixels.clone(),
                        width: buf.width,
                        height: buf.height,
                    },
                );
            }
        }
        map
    })
}

/// Collect **video** frames keyed by `data-viewport` name, for inline painting.
///
/// The software counterpart of [`collect_surface_pixels_by_id`] (issue #358).
/// The two registries cannot share a key space: a `RenderSurface` component
/// stamps its `usize` surface id into `data-render-surface`, while a video
/// viewport carries only the name its player was created with, so this one is
/// keyed by name and feeds `rinch_dom::paint::set_viewport_pixels()`.
///
/// Returns every video surface with a non-empty buffer — not only the ones with
/// a *new* frame — because paint redraws the node whenever anything else on the
/// frame does. Clears dirty flags as a side effect, exactly as the other
/// collectors do.
#[cfg(feature = "desktop")]
pub fn collect_video_frames_by_name()
-> std::collections::HashMap<String, rinch_dom::paint::SurfacePixelData> {
    use std::collections::HashMap;
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        let mut map = HashMap::new();
        for surface in reg.iter() {
            if !surface.is_video {
                continue;
            }
            surface.needs_redraw.store(false, Ordering::Release);
            let buf = surface.buffer.lock().unwrap();
            if !buf.pixels.is_empty() {
                map.insert(
                    surface.viewport_name.clone(),
                    rinch_dom::paint::SurfacePixelData {
                        data: buf.pixels.clone(),
                        width: buf.width,
                        height: buf.height,
                    },
                );
            }
        }
        map
    })
}

/// Update a surface's layout size and, if it changed, fire its resize callback.
///
/// Central point so both the by-id (web `ResizeObserver`) and by-name (desktop
/// compositor) update paths deliver the same push resize notification.
fn set_and_notify_size(surface: &RenderSurfaceHandle, width: u32, height: u32) {
    let changed = {
        let mut sz = surface.layout_size.lock().unwrap();
        if *sz != (width, height) {
            *sz = (width, height);
            true
        } else {
            false
        }
    };
    if changed {
        if let Some(cb) = surface.resize_callback.borrow_mut().as_mut() {
            cb(width, height);
        }
    }
}

/// Update the layout size for a render surface by ID.
pub fn update_layout_size_by_id(surface_id: usize, width: u32, height: u32) {
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        if let Some(surface) = reg.iter().find(|s| s.id == surface_id) {
            set_and_notify_size(surface, width, height);
        }
    });
}

/// Return the IDs of all registered render surfaces.
pub fn registered_surface_ids() -> Vec<usize> {
    SURFACE_REGISTRY.with(|reg| reg.borrow().iter().map(|s| s.id).collect())
}

/// Check if a specific surface has new pixels waiting.
pub fn is_surface_dirty_by_id(id: usize) -> bool {
    SURFACE_REGISTRY.with(|reg| {
        reg.borrow()
            .iter()
            .any(|s| s.id == id && s.needs_redraw.load(Ordering::Acquire))
    })
}

/// Read back GPU textures to CPU pixel buffers for inline-paint surfaces.
///
/// For each RenderSurface component with a GPU texture source, copies the
/// texture to a staging buffer, maps it, and stores the pixels in the
/// surface's CPU buffer. After this, `collect_surface_pixels_by_id()` will
/// include these surfaces.
///
/// Non-inline surfaces (video, GameViewport) are skipped — they use the
/// compositor path which reads the texture directly.
#[cfg(feature = "gpu")]
pub fn readback_gpu_textures() {
    let Some(gpu) = crate::shell::desktop::gpu_handle() else {
        return;
    };

    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        for surface in reg.iter() {
            // Only readback inline surfaces (RenderSurface components)
            if !is_inline_surface(surface) {
                continue;
            }

            let ts_guard = surface.texture_source.lock().unwrap();
            let Some(ref ts) = *ts_guard else {
                continue;
            };

            let width = ts.width;
            let height = ts.height;
            if width == 0 || height == 0 {
                continue;
            }

            // Compute row alignment: wgpu requires bytes_per_row aligned to 256
            let bytes_per_pixel = 4u32;
            let unpadded_row = width * bytes_per_pixel;
            let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let padded_row = unpadded_row.div_ceil(align) * align;

            let buffer_size = (padded_row * height) as u64;
            let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rinch_surface_readback"),
                size: buffer_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("rinch_surface_readback"),
                });

            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &ts.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            gpu.queue.submit(std::iter::once(encoder.finish()));

            // Map the buffer synchronously
            let buffer_slice = staging.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());

            if rx.recv().ok().and_then(|r| r.ok()).is_some() {
                let data = buffer_slice.get_mapped_range();

                // Copy to the surface's CPU buffer, stripping row padding
                let mut pixels = Vec::with_capacity((width * height * bytes_per_pixel) as usize);
                for row in 0..height {
                    let start = (row * padded_row) as usize;
                    let end = start + unpadded_row as usize;
                    pixels.extend_from_slice(&data[start..end]);
                }
                drop(data);
                staging.unmap();

                // Store in the CPU buffer
                let mut buf = surface.buffer.lock().unwrap();
                buf.pixels = pixels;
                buf.width = width;
                buf.height = height;
                // Mark as needing redraw so collect_surface_pixels_by_id picks it up
                surface.needs_redraw.store(true, Ordering::Release);
            }
        }
    });
}

/// Collect GPU texture sources with surface ID, viewport name, and inline flag.
///
/// Returns `(id, viewport_name, is_inline, texture_source_arc)` for each surface
/// with a texture source set. The compositor reads the `TextureView` directly from
/// the `Arc<Mutex<Option<TextureSource>>>` — no pixel upload needed.
/// Clears dirty flags as a side effect.
#[cfg(feature = "gpu")]
#[allow(clippy::type_complexity)]
pub fn collect_texture_sources() -> Vec<(usize, String, bool, Arc<Mutex<Option<TextureSource>>>)> {
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        let mut sources = Vec::new();
        for surface in reg.iter() {
            if surface.texture_source.lock().unwrap().is_some() {
                surface.needs_redraw.store(false, Ordering::Release);
                sources.push((
                    surface.id,
                    surface.viewport_name.clone(),
                    is_inline_surface(surface),
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
                set_and_notify_size(surface, width, height);
                return;
            }
        }
    });
}

/// Update the layout position for a render surface by viewport name.
///
/// Called by the compositor each frame. Position is in logical pixels
/// relative to the window.
pub fn update_layout_position(viewport_name: &str, x: f32, y: f32) {
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        for surface in reg.iter() {
            if surface.viewport_name == viewport_name {
                *surface.layout_position.lock().unwrap() = (x, y);
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
            // conditional hide, etc.) so stale surfaces aren't composited. On web
            // also disconnect the ResizeObserver and remove the canvas event
            // listeners so nothing leaks.
            let surface = surface.clone();
            scope.on_cleanup(move || {
                unregister_render_surface(surface.id);
                #[cfg(target_arch = "wasm32")]
                teardown_web_surface(&surface);
            });
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let div = scope.create_element("div");

            if let Some(ref surface) = self.surface {
                div.set_attribute("data-render-surface", &surface.id.to_string());
            }

            div.set_attribute("style", "width: 100%; height: 100%;");

            div
        }

        #[cfg(target_arch = "wasm32")]
        {
            let canvas = scope.create_element("canvas");
            // `touch-action: none` so a touch/pen drag on the surface drives its
            // input (via the pointer events below) instead of scrolling the page.
            canvas.set_attribute(
                "style",
                "width: 100%; height: 100%; display: block; touch-action: none;",
            );

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

/// Owns the canvas event listeners and `ResizeObserver` for a mounted web
/// surface so they can be torn down cleanly on unmount.
///
/// The `Closure`s are kept alive here (not `forget()`-leaked) — dropping this
/// struct after removing the listeners frees them and their captures.
#[cfg(target_arch = "wasm32")]
pub(crate) struct WebSurfaceCleanup {
    canvas: web_sys::HtmlCanvasElement,
    observer: web_sys::ResizeObserver,
    #[allow(clippy::type_complexity)]
    pointer_listeners: Vec<(
        &'static str,
        wasm_bindgen::closure::Closure<dyn FnMut(web_sys::PointerEvent)>,
    )>,
    wheel_listener: (
        &'static str,
        wasm_bindgen::closure::Closure<dyn FnMut(web_sys::WheelEvent)>,
    ),
    // Kept alive so the observer's callback stays valid until we disconnect it.
    _resize_closure:
        wasm_bindgen::closure::Closure<dyn FnMut(js_sys::Array, wasm_bindgen::JsValue)>,
}

#[cfg(target_arch = "wasm32")]
impl WebSurfaceCleanup {
    /// Disconnect the observer and remove every canvas listener, then drop the
    /// closures (freeing their captures).
    fn teardown(self) {
        use wasm_bindgen::JsCast;
        self.observer.disconnect();
        let target: &web_sys::EventTarget = self.canvas.as_ref();
        for (ty, cb) in &self.pointer_listeners {
            let _ = target.remove_event_listener_with_callback(ty, cb.as_ref().unchecked_ref());
        }
        let _ = target.remove_event_listener_with_callback(
            self.wheel_listener.0,
            self.wheel_listener.1.as_ref().unchecked_ref(),
        );
    }
}

/// Tear down a web surface's listeners/observer and drop its canvas refs.
///
/// Called from the [`RenderSurface`] component's cleanup when the scope is
/// disposed. Idempotent — safe to call even if the canvas never initialized.
#[cfg(target_arch = "wasm32")]
fn teardown_web_surface(surface: &RenderSurfaceHandle) {
    if let Some(cleanup) = surface.web_cleanup.borrow_mut().take() {
        cleanup.teardown();
    }
    *surface.canvas.borrow_mut() = None;
    *surface.canvas_ctx.borrow_mut() = None;
}

/// Start a `requestAnimationFrame` loop that invokes the render callback for
/// the given surface each frame. The loop self-terminates when the surface is
/// unregistered or its callback is removed.
///
/// `running` guards against two loops for the same surface: it's set here and
/// cleared when the loop stops, so `set_render_callback` and a remount can both
/// call this safely and only one loop exists at a time. After an unmount stops
/// the loop, a remount (via [`schedule_canvas_init`]) restarts it — otherwise a
/// render-callback surface would stay blank after a hide→show cycle.
/// A `requestAnimationFrame` callback that has to reference itself in order to
/// re-schedule, so it can only be built as a cell filled in after construction.
#[cfg(target_arch = "wasm32")]
type RafClosure = std::rc::Rc<RefCell<Option<wasm_bindgen::prelude::Closure<dyn FnMut()>>>>;

#[cfg(target_arch = "wasm32")]
fn start_raf_loop(surface_id: usize, running: std::rc::Rc<Cell<bool>>) {
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;

    if running.get() {
        return; // a loop is already driving this surface
    }
    running.set(true);

    let closure: RafClosure = Rc::new(RefCell::new(None));
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
        } else {
            // Allow a future remount to start a fresh loop.
            running.set(false);
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
                // The dx/dy args are `f64` in stable web-sys but `i32` under the
                // `web_sys_unstable_apis` cfg (which a future OPFS storage backend
                // needs). Pick the literal type per-cfg so both builds compile.
                #[cfg(web_sys_unstable_apis)]
                let _ = ctx.put_image_data(&img, 0, 0);
                #[cfg(not(web_sys_unstable_apis))]
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
            // Set up event listeners + observer, keeping their closures alive in
            // WebSurfaceCleanup so they can be removed on unmount (no leaks).
            let (pointer_listeners, wheel_listener) = setup_canvas_events(&canvas_el, surface.id);
            let (observer, resize_closure) = setup_resize_observer(&canvas_el, surface.id);
            *surface.web_cleanup.borrow_mut() = Some(WebSurfaceCleanup {
                canvas: canvas_el.clone(),
                observer,
                pointer_listeners,
                wheel_listener,
                _resize_closure: resize_closure,
            });
            // Store canvas ref. The 2D context is created lazily on the first
            // submit_frame() call. This allows users to call canvas_element()
            // and create a WebGPU or WebGL context first for GPU rendering.
            *surface.canvas.borrow_mut() = Some(canvas_el);

            // On a remount, the render-callback rAF loop from the previous mount
            // has self-terminated (the surface was unregistered). Restart it if a
            // render callback is set, so a hidden→shown surface keeps rendering.
            // `raf_running` makes this a no-op on the first mount (the loop that
            // set_render_callback already started is still live).
            if surface.render_callback.borrow().is_some() {
                start_raf_loop(surface.id, surface.raf_running.clone());
            }
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
#[allow(clippy::type_complexity)]
fn setup_canvas_events(
    canvas: &web_sys::HtmlCanvasElement,
    surface_id: usize,
) -> (
    Vec<(
        &'static str,
        wasm_bindgen::closure::Closure<dyn FnMut(web_sys::PointerEvent)>,
    )>,
    (
        &'static str,
        wasm_bindgen::closure::Closure<dyn FnMut(web_sys::WheelEvent)>,
    ),
) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    let target: &web_sys::EventTarget = canvas.as_ref();

    // Pointer closures are collected (not forgotten) so they can be removed on
    // unmount. Each is stored with its event-type string for removeEventListener.
    let mut pointer_listeners: Vec<(&'static str, Closure<dyn FnMut(web_sys::PointerEvent)>)> =
        Vec::new();

    // Pointer events (deref to MouseEvent) cover mouse, touch, and pen on one
    // path — so a game/custom-render surface receives input on touch devices too.
    // The canvas carries `touch-action: none` (set at creation) so a touch drag
    // drives the surface instead of scrolling the page.

    // pointerdown — grab pointer capture so a drag keeps delivering moves/up even
    // if the contact leaves the canvas (SurfaceEvent names stay Mouse* for API
    // stability).
    {
        let canvas = canvas.clone();
        let closure = Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
            event.stop_propagation();
            let x = event.offset_x() as f32;
            let y = event.offset_y() as f32;
            let button = mouse_button_from_i16(event.button());
            set_focused_surface(Some(surface_id));
            let _ = canvas.set_pointer_capture(event.pointer_id());
            dispatch_surface_event(surface_id, SurfaceEvent::MouseDown { x, y, button });
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback("pointerdown", closure.as_ref().unchecked_ref())
            .unwrap();
        pointer_listeners.push(("pointerdown", closure));
    }

    // pointerup — release the capture taken on pointerdown.
    {
        let canvas = canvas.clone();
        let closure = Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
            let x = event.offset_x() as f32;
            let y = event.offset_y() as f32;
            let button = mouse_button_from_i16(event.button());
            let _ = canvas.release_pointer_capture(event.pointer_id());
            dispatch_surface_event(surface_id, SurfaceEvent::MouseUp { x, y, button });
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback("pointerup", closure.as_ref().unchecked_ref())
            .unwrap();
        pointer_listeners.push(("pointerup", closure));
    }

    // pointercancel — the browser took the pointer away; release capture and end
    // the interaction like an up (so the surface isn't left mid-drag).
    {
        let canvas = canvas.clone();
        let closure = Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
            let x = event.offset_x() as f32;
            let y = event.offset_y() as f32;
            let button = mouse_button_from_i16(event.button());
            let _ = canvas.release_pointer_capture(event.pointer_id());
            dispatch_surface_event(surface_id, SurfaceEvent::MouseUp { x, y, button });
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback("pointercancel", closure.as_ref().unchecked_ref())
            .unwrap();
        pointer_listeners.push(("pointercancel", closure));
    }

    // pointermove
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
            let x = event.offset_x() as f32;
            let y = event.offset_y() as f32;
            dispatch_surface_event(surface_id, SurfaceEvent::MouseMove { x, y });
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback("pointermove", closure.as_ref().unchecked_ref())
            .unwrap();
        pointer_listeners.push(("pointermove", closure));
    }

    // pointerenter
    {
        let closure = Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
            let x = event.offset_x() as f32;
            let y = event.offset_y() as f32;
            dispatch_surface_event(surface_id, SurfaceEvent::MouseEnter { x, y });
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback("pointerenter", closure.as_ref().unchecked_ref())
            .unwrap();
        pointer_listeners.push(("pointerenter", closure));
    }

    // pointerleave
    {
        let closure = Closure::wrap(Box::new(move |_event: web_sys::PointerEvent| {
            dispatch_surface_event(surface_id, SurfaceEvent::MouseLeave);
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback("pointerleave", closure.as_ref().unchecked_ref())
            .unwrap();
        pointer_listeners.push(("pointerleave", closure));
    }

    // wheel
    let wheel_listener = {
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
        ("wheel", closure)
    };

    (pointer_listeners, wheel_listener)
}

#[cfg(target_arch = "wasm32")]
#[allow(clippy::type_complexity)]
fn setup_resize_observer(
    canvas: &web_sys::HtmlCanvasElement,
    surface_id: usize,
) -> (
    web_sys::ResizeObserver,
    wasm_bindgen::closure::Closure<dyn FnMut(js_sys::Array, wasm_bindgen::JsValue)>,
) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    let canvas_for_cb = canvas.clone();
    let callback = Closure::wrap(Box::new(move |entries: js_sys::Array, _observer: JsValue| {
        if let Some(entry) = entries.get(0).dyn_ref::<web_sys::ResizeObserverEntry>() {
            let rect = entry.content_rect();
            // Physical backing px = CSS px × devicePixelRatio, matching the
            // desktop convention that layout_size is physical. This is what makes
            // HiDPI correct out of the box for GPU (wgpu / WebGL) apps.
            let dpr = web_sys::window()
                .map(|w| w.device_pixel_ratio())
                .filter(|d| *d > 0.0)
                .unwrap_or(1.0);
            let width = (rect.width() * dpr).round() as u32;
            let height = (rect.height() * dpr).round() as u32;
            if width > 0 && height > 0 {
                // Size the canvas backing store to physical px so an app-owned
                // GPU context renders sharp. Skip when rinch owns a 2D context for
                // CPU blitting — web_blit_surface manages that path's dimensions
                // from the submitted frame instead.
                let owns_2d_ctx = SURFACE_REGISTRY.with(|reg| {
                    reg.borrow()
                        .iter()
                        .find(|s| s.id == surface_id)
                        .map(|s| s.canvas_ctx.borrow().is_some())
                        .unwrap_or(false)
                });
                if !owns_2d_ctx {
                    if canvas_for_cb.width() != width {
                        canvas_for_cb.set_width(width);
                    }
                    if canvas_for_cb.height() != height {
                        canvas_for_cb.set_height(height);
                    }
                }
                // Update layout size + fire the surface's resize callback on
                // change (after the canvas is sized, so the app reconfigures its
                // GPU surface against the correct backing store).
                update_layout_size_by_id(surface_id, width, height);
            }
        }
    }) as Box<dyn FnMut(js_sys::Array, JsValue)>);

    let observer = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()).unwrap();
    observer.observe(canvas);
    (observer, callback)
}

// ── #358: which surfaces reach the compositor, and which paint inline ────────

#[cfg(all(test, feature = "desktop"))]
mod compositor_routing_tests {
    use super::*;

    /// The whole table from `surface_takes_compositor_path`'s doc comment,
    /// pinned in both columns regardless of which backend this build is.
    ///
    /// The asymmetry is the point: on software the frame would otherwise be
    /// written over the finished pixel buffer, on top of any overlay above it;
    /// on GPU the layers go down *first* and the UI blends over them, so video
    /// genuinely belongs on the compositor there.
    #[test]
    fn the_backend_routing_table() {
        const SOFTWARE: bool = false;
        const GPU: bool = true;
        // (is_inline, is_video)
        const RENDER_SURFACE: (bool, bool) = (true, false);
        const VIDEO: (bool, bool) = (false, true);
        const GAME_VIEWPORT: (bool, bool) = (false, false);

        for (label, (inline, video), gpu, expected) in [
            ("RenderSurface / software", RENDER_SURFACE, SOFTWARE, false),
            ("RenderSurface / gpu", RENDER_SURFACE, GPU, false),
            ("video / software", VIDEO, SOFTWARE, false),
            ("video / gpu", VIDEO, GPU, true),
            ("GameViewport / software", GAME_VIEWPORT, SOFTWARE, true),
            ("GameViewport / gpu", GAME_VIEWPORT, GPU, true),
        ] {
            assert_eq!(
                surface_takes_compositor_path(inline, video, gpu),
                expected,
                "{label} takes the compositor path? expected {expected}"
            );
        }
    }

    /// A video surface's frame is collected by name for inline painting, and —
    /// on software — is *not* also handed to the blit that would write it over
    /// the finished UI. That double delivery is #358.
    #[test]
    fn a_video_frame_goes_to_the_inline_map_and_off_the_software_blit() {
        let video = create_video_surface("test-video");
        video.writer().submit_frame(&[10, 20, 30, 255], 1, 1);

        let by_name = collect_video_frames_by_name();
        let frame = by_name
            .get("test-video")
            .expect("the video frame is collected by viewport name");
        assert_eq!((frame.width, frame.height), (1, 1));
        assert_eq!(frame.data, vec![10, 20, 30, 255]);

        let blitted = collect_surface_frames();
        assert_eq!(
            blitted.iter().any(|(name, ..)| name == "test-video"),
            cfg!(not(software_shell)),
            "video reaches the compositor path on GPU only — on software it \
             paints inline instead (#358)"
        );

        unregister_render_surface(video.id());
    }

    /// The regression guard the design calls for: `GameViewport` shares
    /// `create_render_surface_with_name`, so it must keep the compositor blit it
    /// has always had on **both** backends.
    #[test]
    fn a_game_viewport_surface_still_reaches_the_compositor() {
        let game = create_render_surface_with_name("game");
        game.writer().submit_frame(&[1, 2, 3, 255], 1, 1);

        assert!(
            !collect_video_frames_by_name().contains_key("game"),
            "a GameViewport is not video and must not be painted inline"
        );
        assert!(
            collect_surface_frames()
                .iter()
                .any(|(name, ..)| name == "game"),
            "a GameViewport surface still takes the compositor path"
        );

        unregister_render_surface(game.id());
    }

    /// And a `RenderSurface` component keeps its own inline path, by id.
    #[test]
    fn a_render_surface_component_is_still_collected_by_id() {
        let surface = create_render_surface();
        mount_render_surface(&surface);
        surface.writer().submit_frame(&[9, 9, 9, 255], 1, 1);

        assert!(collect_surface_pixels_by_id().contains_key(&surface.id()));
        assert!(collect_video_frames_by_name().is_empty());
        assert!(collect_surface_frames().is_empty());

        unregister_render_surface(surface.id());
    }
}
