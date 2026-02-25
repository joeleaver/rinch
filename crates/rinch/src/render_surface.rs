//! Embed external renderers into the rinch layout.
//!
//! `RenderSurface` lets you feed raw RGBA pixels from any source (game engine,
//! terminal emulator, video decoder, custom GPU renderer) into a rinch layout.
//! Rinch positions the surface via CSS/Taffy and composites the pixels into
//! the final frame.
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
    /// Text input while the surface is focused.
    TextInput(String),
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

        // Wake the event loop so it picks up the new frame
        crate::shell::rinch_runtime::run_on_main_thread(|| {});
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
    /// Shared pixel buffer.
    pub(crate) buffer: Arc<Mutex<SurfaceBuffer>>,
    /// Dirty flag (set by writer, cleared by frame collector).
    pub(crate) needs_redraw: Arc<AtomicBool>,
    /// Event handler (main-thread only).
    #[allow(clippy::type_complexity)]
    pub(crate) event_handler: std::rc::Rc<RefCell<Option<Box<dyn Fn(SurfaceEvent)>>>>,
    /// Viewport name for hole-punch compositing.
    pub(crate) viewport_name: String,
}

impl RenderSurfaceHandle {
    /// Get a thread-safe writer for submitting frames.
    pub fn writer(&self) -> SurfaceWriter {
        SurfaceWriter {
            buffer: self.buffer.clone(),
            needs_redraw: self.needs_redraw.clone(),
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
        needs_redraw: Arc::new(AtomicBool::new(false)),
        event_handler: std::rc::Rc::new(RefCell::new(None)),
        viewport_name: format!("__render_surface_{id}"),
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

/// Collect frames from ALL registered surfaces that have pixel data.
///
/// Returns `(viewport_name, pixels, width, height)` for every surface with
/// a non-empty buffer, regardless of whether new data was submitted since the
/// last collection. This ensures the compositor always has all active layers.
/// Clears dirty flags as a side effect.
pub fn collect_surface_frames() -> Vec<(String, Vec<u8>, u32, u32)> {
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        let mut frames = Vec::new();
        for surface in reg.iter() {
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

/// Check if any render surfaces are registered (even if they haven't submitted frames yet).
pub fn any_surfaces_registered() -> bool {
    SURFACE_REGISTRY.with(|reg| !reg.borrow().is_empty())
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
    FOCUSED_SURFACE.with(|f| {
        *f.borrow_mut() = id;
    });
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
}
