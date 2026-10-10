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

// `Cell` is used in every configuration since the handle's pointer-events flag;
// before that a host build with neither `desktop` nor wasm warned on it.
use std::cell::Cell;
use std::cell::RefCell;
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
///
/// **A press keeps the pointer.** From a press on the surface until the
/// release of the button that started it, that pointer's moves and releases
/// go to the surface, wherever it is (coordinates then fall outside
/// `0..width`, `0..height`), and the surface hears no `MouseLeave` /
/// `MouseEnter` until then; a press or release of another button meanwhile
/// (a chord) is the surface's too and ends nothing. That is the browser's
/// pointer capture, which the web backend takes on every press. The desktop
/// runtime routes the surface's events the same way, but the document's DOM
/// hover (`:hover`, `onmouseenter`/`onmouseleave`) and the cursor still
/// follow the element under the pointer meanwhile.
///
/// A press whose release never arrives ends without one: the surface hears
/// `PointerCancel` (with pointer events) or a `MouseUp` where the cursor is
/// (without). On desktop that happens when the window loses focus, when the
/// same pointer presses the left button again, and on a platform
/// `PointerCancel`; a surface that unmounts just stops holding it.
///
/// **Pointer events are opt-in.** A surface that calls
/// [`RenderSurfaceHandle::set_pointer_events`] hears presses, moves and
/// releases as [`PointerDown`](Self::PointerDown) /
/// [`PointerMove`](Self::PointerMove) / [`PointerUp`](Self::PointerUp) /
/// [`PointerCancel`](Self::PointerCancel), which say which device and how
/// hard ([`SurfacePointer`]), **instead of** `MouseDown` / `MouseMove` /
/// `MouseUp`, plus [`Pinch`](Self::Pinch). Every other surface hears exactly
/// what it always did, whatever the device.
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

    // ── Pointer events (opt-in: `RenderSurfaceHandle::set_pointer_events`) ──
    /// A mouse button, pen tip or finger went down on the surface.
    PointerDown {
        x: f32,
        y: f32,
        /// The button; a pen tip or a finger is `Left`, a pen's barrel
        /// button `Right`.
        button: SurfaceMouseButton,
        pointer: SurfacePointer,
    },
    /// A pointer moved over the surface, or anywhere while it holds a press
    /// that started on it (see the capture note above). Also sent when only
    /// the pressure changed.
    PointerMove {
        x: f32,
        y: f32,
        pointer: SurfacePointer,
    },
    /// A press that started on the surface ended.
    PointerUp {
        x: f32,
        y: f32,
        button: SurfaceMouseButton,
        pointer: SurfacePointer,
    },
    /// The platform took a pressed pointer away (a touch the system
    /// cancelled, a palm rejected): end what the press was doing, as if it
    /// had not happened if that is possible. No `PointerUp` follows.
    PointerCancel { pointer: SurfacePointer },
    /// A zoom gesture, about `(x, y)`: two touches spreading or closing, a
    /// trackpad pinch, or Ctrl+wheel (what a browser turns a trackpad pinch
    /// into). `scale` multiplies the zoom: above 1 is in, below 1 is out, one
    /// event's factor relative to the last. A two-touch pinch still sends
    /// each touch's `PointerMove`; an app that pinches ignores them while
    /// two touches are down.
    Pinch { x: f32, y: f32, scale: f32 },
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

// ── Pointer events (opt-in) ──────────────────────────────────────────────────

/// What kind of device a pointer event came from — see
/// [`RenderSurfaceHandle::set_pointer_events`].
///
/// `#[non_exhaustive]`: a kind can be added without breaking an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SurfacePointerKind {
    /// A mouse, or a touchpad driving the cursor.
    Mouse,
    /// A pen or stylus tip (a tablet tool on desktop, `pointerType == "pen"`
    /// in the browser).
    Pen,
    /// A pen held the other way round (the eraser end), where the platform
    /// says so: a tablet tool of kind eraser on desktop, a pen whose
    /// `buttons` has the eraser bit (32) in the browser.
    Eraser,
    /// A finger on a touch screen.
    Touch,
    /// A pointer the platform did not describe.
    Unknown,
}

/// The pointer a [`SurfaceEvent::PointerDown`] / `PointerMove` / `PointerUp`
/// / `PointerCancel` came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfacePointer {
    /// Which pointer: stable from its down to its up, unique among the
    /// pointers down at once (two fingers are two ids). Compare for equality
    /// only. On desktop (and Android and embed, where every pointer is the
    /// mouse) the mouse is `1`; in the browser it is the event's
    /// `pointerId`, whose value for a mouse differs between engines.
    pub id: u64,
    /// The device.
    pub kind: SurfacePointerKind,
    /// How hard it presses, `0.0..=1.0`. A device that cannot tell reports
    /// `0.5` while a button or contact is down and `0.0` otherwise, the
    /// Pointer Events rule (a mouse always, a touch screen without force
    /// sensing).
    pub pressure: f32,
    /// The pointer a single-pointer app follows: the mouse, the first finger
    /// down, a pen.
    pub primary: bool,
}

impl SurfacePointer {
    /// The mouse, with `pressure` per the Pointer Events rule for a button
    /// down (`0.5`) or up (`0.0`).
    pub const fn mouse(down: bool) -> Self {
        Self {
            id: 1,
            kind: SurfacePointerKind::Mouse,
            pressure: if down { 0.5 } else { 0.0 },
            primary: true,
        }
    }
}

/// Where a pointer is in its press, for [`dispatch_surface_pointer`].
#[cfg(any(
    feature = "desktop",
    feature = "android",
    feature = "embed",
    target_arch = "wasm32"
))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PointerPhase {
    Down(SurfaceMouseButton),
    Move,
    Up(SurfaceMouseButton),
}

/// Two touches on one surface turned into a scale factor: the distance
/// between them now over the distance at the last report.
#[cfg(any(
    feature = "desktop",
    feature = "android",
    feature = "embed",
    target_arch = "wasm32"
))]
#[derive(Debug, Default)]
pub(crate) struct PinchTracker {
    /// The touches down, `(pointer id, x, y)`, in the order they came down.
    touches: Vec<(u64, f32, f32)>,
    /// The distance between the first two at the last report (or when the
    /// second came down).
    last: Option<f32>,
}

#[cfg(any(
    feature = "desktop",
    feature = "android",
    feature = "embed",
    target_arch = "wasm32"
))]
impl PinchTracker {
    fn spread(&self) -> Option<(f32, f32, f32)> {
        let [(_, ax, ay), (_, bx, by), ..] = self.touches[..] else {
            return None;
        };
        let d = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
        Some(((ax + bx) / 2.0, (ay + by) / 2.0, d))
    }

    /// A touch came down at `(x, y)`.
    pub(crate) fn down(&mut self, id: u64, x: f32, y: f32) {
        self.touches.retain(|t| t.0 != id);
        self.touches.push((id, x, y));
        self.last = self.spread().map(|s| s.2);
    }

    /// A touch was lifted or cancelled.
    pub(crate) fn up(&mut self, id: u64) {
        self.touches.retain(|t| t.0 != id);
        self.last = self.spread().map(|s| s.2);
    }

    /// A touch moved: the pinch it makes, as `(centre x, centre y, scale)`,
    /// when the distance between the first two touches down changed. A third
    /// touch is in no pinch: moving it leaves that distance as it was (`last`
    /// is always the current spread between moves, which every `down`, `up`
    /// and `moved` keeps true), so it reports nothing without a check of its
    /// own.
    pub(crate) fn moved(&mut self, id: u64, x: f32, y: f32) -> Option<(f32, f32, f32)> {
        let i = self.touches.iter().position(|t| t.0 == id)?;
        self.touches[i] = (id, x, y);
        let (cx, cy, d) = self.spread()?;
        let last = self.last.replace(d)?;
        (last > 0.0 && d > 0.0 && d != last).then(|| (cx, cy, d / last))
    }
}

/// Keyboard event data for render surfaces.
#[derive(Debug, Clone)]
pub struct SurfaceKeyData {
    /// The logical key value, spelled like `KeyboardEvent.key` on both
    /// backends (e.g., "a", "Enter", "Backspace", and `" "` for the space bar).
    pub key: String,
    /// The physical key code (e.g., "KeyA", "Enter", "Space").
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
    /// Shared with whatever paint was handed it (`SurfacePixelData::data`),
    /// so collecting a frame copies nothing; `submit_frame` writes in place
    /// when nothing else holds it and allocates a fresh buffer when a paint
    /// still does.
    pixels: Arc<Vec<u8>>,
    width: u32,
    height: u32,
    /// Every alpha in `pixels` is 255. Found by `submit_frame` on the thread
    /// that submits, so the UI thread neither scans nor premultiplies an
    /// opaque frame (#361).
    opaque: bool,
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

        // Scanned here, before the lock and on the submitting thread: a
        // producer's frame is usually opaque, and knowing it lets the software
        // painter skip the premultiply — and, at the viewport's own size, copy
        // rows instead of sampling (#361).
        let opaque = pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255);
        {
            let mut buf = self.buffer.lock().unwrap();
            match Arc::get_mut(&mut buf.pixels) {
                Some(own) => {
                    own.clear();
                    own.extend_from_slice(pixels);
                }
                None => buf.pixels = Arc::new(pixels.to_vec()),
            }
            buf.width = width;
            buf.height = height;
            buf.opaque = opaque;
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
    /// Key-claim handler (main-thread only), set by
    /// [`set_key_handler`](RenderSurfaceHandle::set_key_handler). Issue #482:
    /// additive and separate from `event_handler` — the ordinary handler keeps
    /// receiving every `KeyDown`/`KeyUp` for input, this one only answers
    /// whether the key should stop at the surface.
    #[allow(clippy::type_complexity)]
    pub(crate) key_handler: std::rc::Rc<RefCell<Option<Box<dyn Fn(&SurfaceKeyData) -> bool>>>>,
    /// Whether presses, moves and releases arrive as pointer events — see
    /// [`set_pointer_events`](RenderSurfaceHandle::set_pointer_events).
    pub(crate) pointer_events: std::rc::Rc<Cell<bool>>,
    /// Viewport name for hole-punch compositing.
    pub(crate) viewport_name: String,
    /// Whether this surface carries decoded **video** frames.
    ///
    /// Video and `GameViewport` both arrive through
    /// [`create_render_surface_with_name`]-shaped construction and both stamp a
    /// `data-viewport` attribute, so the name cannot tell them apart — and
    /// renaming video's surface would silently reroute `GameViewport` with it.
    /// This flag is set at video's registration site and nowhere else (issue
    /// #358). Both paint inline on the software backend (#358, #361); what the
    /// flag decides there is the hole: a `GameViewport` punches one through its
    /// ancestors' backgrounds and its letterbox is that hole, while video
    /// punches none and paints its own black bars.
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

    /// Declares which keys this focused surface actually uses (issue #482).
    ///
    /// Additive and independent of [`set_event_handler`](Self::set_event_handler):
    /// every `KeyDown`/`KeyUp` the surface receives while focused is *still*
    /// delivered there first, exactly as before — this handler answers a
    /// narrower question asked afterward: should the key stop at the surface,
    /// or continue past it?
    ///
    /// Before this existed, a focused surface swallowed **every** key
    /// unconditionally — a host's own `window`-level keybindings (undo,
    /// delete, a browser reload shortcut) and rinch's own DevTools/inspect/Tab
    /// handling all went deaf the moment a user clicked the canvas, whether or
    /// not the surface did anything with the key. Return `true` for a key the
    /// surface genuinely consumes (WASD steering a character, Space to jump);
    /// return `false` — or leave this unset — for everything else, and it
    /// continues: on desktop to the document-level keyboard interceptor and
    /// the built-in DevTools/inspect-mode/Tab handling, on web to
    /// `preventDefault`/`stopPropagation` being skipped so the event reaches
    /// `window` listeners and the browser's own shortcuts.
    ///
    /// **The default, with no handler set, is "claims nothing."** A focused
    /// surface with no `set_key_handler` call no longer swallows any key — the
    /// fail-safe direction, since a host that never opts in should not lose
    /// its keybindings to a canvas it clicked.
    ///
    /// **On the web, that also means an unclaimed key keeps the browser's own
    /// default action** — the same thing that no longer `preventDefault()`s —
    /// matching how an unfocused `<canvas>` already behaves: Space,
    /// PageUp/PageDown and the arrow keys can scroll the page, and Tab leaves
    /// the canvas for the next tab stop. A web game steering with any of
    /// those keys must claim them here, or the page scrolls out from under
    /// it.
    pub fn set_key_handler(&self, handler: impl Fn(&SurfaceKeyData) -> bool + 'static) {
        *self.key_handler.borrow_mut() = Some(Box::new(handler));
    }

    /// Hear presses, moves and releases as [`SurfaceEvent::PointerDown`] /
    /// `PointerMove` / `PointerUp` / `PointerCancel`, which say which device
    /// (mouse, pen, eraser, touch) and how hard it presses, instead of
    /// `MouseDown` / `MouseMove` / `MouseUp`; and zoom gestures (two touches,
    /// a trackpad pinch, Ctrl+wheel) as [`SurfaceEvent::Pinch`] instead of a
    /// `MouseWheel`. Off by default, so a surface that never asks hears what
    /// it always has. `MouseEnter`, `MouseLeave`, a plain `MouseWheel`, keys
    /// and drag-and-drop are the same either way.
    ///
    /// What each platform reports: in the browser, Pointer Events
    /// (`pointerType`, `pressure`, `pointerId`, `isPrimary`); on desktop,
    /// winit's pointer source (a tablet tool's force, a touch's force where
    /// the platform measures one, else `0.5` while down). Desktop delivers
    /// one move per pointer per frame, as the browser does without
    /// `getCoalescedEvents`. On Android every finger is folded into the mouse
    /// (kind `Mouse`, id `1`: no `Touch`, no two-finger `Pinch`), and an
    /// embedded `RinchContext` is fed mouse events, so it reports the mouse
    /// too; Ctrl+wheel still pinches on both.
    pub fn set_pointer_events(&self, on: bool) {
        self.pointer_events.set(on);
    }

    /// Whether [`set_pointer_events`](Self::set_pointer_events) is on.
    pub fn pointer_events(&self) -> bool {
        self.pointer_events.get()
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
            pixels: Arc::new(Vec::new()),
            width: 0,
            height: 0,
            opaque: false,
        })),
        #[cfg(feature = "gpu")]
        texture_source: Arc::new(Mutex::new(None)),
        needs_redraw: Arc::new(AtomicBool::new(false)),
        event_handler: std::rc::Rc::new(RefCell::new(None)),
        key_handler: std::rc::Rc::new(RefCell::new(None)),
        pointer_events: std::rc::Rc::new(Cell::new(false)),
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
/// marked as carrying video, which on the software backend means it punches no
/// hole and paints black letterbox bars (issues #358, #354). Separate entry
/// point rather than a name convention: `GameViewport` shares
/// [`create_render_surface_with_name`], so a naming rule would reroute it too.
pub fn create_video_surface(viewport_name: &str) -> RenderSurfaceHandle {
    create_named_surface(viewport_name, true)
}

/// A callback receiving decoded RGBA frames as `(pixels, width, height)` —
/// the shape of `rinch_video`'s `FrameSink`.
pub type VideoFrameSink = Arc<dyn Fn(&[u8], u32, u32) + Send + Sync>;

/// The frame sink a **video player** delivers decoded frames through: a video
/// surface named `viewport_id` plus a closure that submits into it.
///
/// This is what the desktop shell hands `rinch_video::set_frame_sink_factory`.
///
/// **The sink owns the surface's registration** (issue #363): the surface stays
/// in the registry while any clone of the sink is alive and is unregistered
/// when the last one drops — which a player does on `cleanup()` (what
/// `use_video_player` runs when its component unmounts) and when it is itself
/// dropped. The factory runs before any DOM exists, with no render scope to tie
/// the surface to, which is why the handle used to be `mem::forget`-leaked:
/// every video ever played kept a registered surface, holding its last decoded
/// frame, for the rest of the process.
pub fn create_video_frame_sink(viewport_id: &str) -> VideoFrameSink {
    let handle = create_video_surface(viewport_id);
    let writer = handle.writer();
    let lease = SurfaceLease {
        id: handle.id,
        thread: std::thread::current().id(),
    };
    Arc::new(move |pixels: &[u8], w: u32, h: u32| {
        let _ = &lease; // owned by the closure: released with its last clone
        writer.submit_frame(pixels, w, h);
    })
}

/// Unregisters a surface when dropped.
///
/// The registry is thread-local, so a drop on another thread cannot reach it:
/// that release is queued for the main thread instead (the sink is `Send`, even
/// though every player in the workspace drops it on the main thread), and runs
/// at the next drain of the main-thread queue. It is queued through the host's
/// dispatcher ([`rinch_core::dispatch_main_callback`]), so a host that wakes
/// for queued work wakes for it, and it never takes the wake of a closure
/// queued behind it (issue #1035). Not `run_on_main_thread`: that panics where
/// no dispatcher is registered, which a `Drop` must not risk, and on the main
/// thread it runs inline, inside the registry borrow this path exists to avoid.
///
/// A lease dropped after the thread's registry was destroyed — a player still
/// playing at thread or process exit, whose `ACTIVE_PLAYERS` entry is torn down
/// after the registry — has nothing left to unregister and does nothing: a
/// `with` there would panic inside a TLS destructor, which aborts.
struct SurfaceLease {
    id: usize,
    thread: std::thread::ThreadId,
}

impl Drop for SurfaceLease {
    fn drop(&mut self) {
        let id = self.id;
        // A drop from inside a registry walk (a render callback letting go of a
        // player) cannot take the registry mutably either; defer it the same way.
        // At thread exit (and at process exit, for the main thread) the
        // registry may already have been destroyed: then there is nothing left
        // to unregister, and `with` would panic inside a TLS destructor (abort).
        let Ok(registry_free) = SURFACE_REGISTRY.try_with(|reg| reg.try_borrow_mut().is_ok())
        else {
            return;
        };
        if std::thread::current().id() == self.thread && registry_free {
            unregister_render_surface(id);
        } else {
            rinch_core::dispatch_main_callback(Box::new(move || unregister_render_surface(id)));
        }
    }
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
///
/// A surface that was registered asks the window for a redraw on its way out
/// (desktop): the next paint is handed a viewport set without it, which is
/// what takes its last frame and its hole off the screen (issue #349) — and a
/// surface can go with nothing else changing, so nothing else would ask.
pub fn unregister_render_surface(id: usize) {
    let removed = SURFACE_REGISTRY.with(|reg| {
        let mut reg = reg.borrow_mut();
        let before = reg.len();
        reg.retain(|s| s.id != id);
        reg.len() != before
    });
    #[cfg(feature = "desktop")]
    if removed {
        request_repaint();
    }
    #[cfg(not(feature = "desktop"))]
    let _ = removed;
    // Clear focus if this surface was focused
    FOCUSED_SURFACE.with(|f| {
        let mut f = f.borrow_mut();
        if *f == Some(id) {
            *f = None;
        }
    });
    // Its touches go with it.
    #[cfg(any(
        feature = "desktop",
        feature = "android",
        feature = "embed",
        target_arch = "wasm32"
    ))]
    let _ = PINCHES.try_with(|p| p.borrow_mut().retain(|(s, _)| *s != id));
}

/// Check if any surface has a new frame waiting.
pub fn any_surface_dirty() -> bool {
    SURFACE_REGISTRY.with(|reg| {
        reg.borrow()
            .iter()
            .any(|s| s.needs_redraw.load(Ordering::Acquire))
    })
}

/// Whether the thread's registry holds any **inline** surface (a
/// `RenderSurface` component). When it holds none, an embedded context's
/// surface pass has nothing to size, drive or collect, and skips the document
/// walk that finds its surfaces (issue #331).
#[cfg(any(feature = "desktop", feature = "embed"))]
pub fn any_inline_surface_registered() -> bool {
    SURFACE_REGISTRY.with(|reg| reg.borrow().iter().any(is_inline_surface))
}

/// Whether a surface draws inline from its CPU buffer **in an embedded
/// context**: a `RenderSurface` component whose frames arrive as pixels. A
/// `GpuTextureRegistrar` texture is read back only by the desktop runtime's
/// own device, so in an embedded context it draws nothing, and its
/// `notify_frame_ready` must not be read as a frame to draw.
#[cfg(any(feature = "desktop", feature = "embed"))]
fn draws_cpu_frames_inline(surface: &RenderSurfaceHandle) -> bool {
    #[cfg(feature = "gpu")]
    if surface.texture_source.lock().unwrap().is_some() {
        return false;
    }
    is_inline_surface(surface)
}

/// Whether one of the surfaces `ids` — one embedded context's own — has a new
/// CPU frame waiting (issue #331). Scoped to the ids because the registry is
/// thread-global and several contexts may share the thread; scoped to inline
/// CPU surfaces because nothing in an embedded context clears a compositor
/// surface's flag.
#[cfg(any(feature = "desktop", feature = "embed"))]
pub fn inline_surfaces_dirty(ids: &[usize]) -> bool {
    if ids.is_empty() {
        return false;
    }
    SURFACE_REGISTRY.with(|reg| {
        reg.borrow().iter().any(|s| {
            ids.contains(&s.id)
                && draws_cpu_frames_inline(s)
                && s.needs_redraw.load(Ordering::Acquire)
        })
    })
}

/// [`collect_surface_pixels_by_id`] for the surfaces `ids` only, clearing only
/// their new-frame flags — one embedded context's own surfaces, so a context
/// sharing the thread keeps its fresh frame for its own scene (issue #331).
/// Returns the pixels and the ids whose frame was new.
#[cfg(any(feature = "desktop", feature = "embed"))]
pub fn collect_surface_pixels_for(
    ids: &[usize],
) -> (
    std::collections::HashMap<usize, rinch_dom::paint::SurfacePixelData>,
    Vec<usize>,
) {
    use std::collections::HashMap;
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        let mut map = HashMap::new();
        let mut fresh = Vec::new();
        for surface in reg.iter() {
            if !ids.contains(&surface.id) || !is_inline_surface(surface) {
                continue;
            }
            let was_dirty = surface.needs_redraw.swap(false, Ordering::AcqRel);
            if !draws_cpu_frames_inline(surface) {
                continue;
            }
            let buf = surface.buffer.lock().unwrap();
            if !buf.pixels.is_empty() {
                if was_dirty {
                    fresh.push(surface.id);
                }
                map.insert(
                    surface.id,
                    rinch_dom::paint::SurfacePixelData {
                        data: Arc::clone(&buf.pixels),
                        width: buf.width,
                        height: buf.height,
                        opaque: buf.opaque,
                    },
                );
            }
        }
        (map, fresh)
    })
}

/// Check if a surface uses inline painting (RenderSurface component)
/// vs compositor/hole-punch (video, GameViewport).
///
/// Surfaces created by `create_render_surface()` have auto-generated names
/// starting with `__render_surface_` and paint inline. Surfaces created by
/// `create_render_surface_with_name()` have custom names and use the compositor.
///
/// Every caller is behind `desktop`, `gpu` (which implies `desktop`) or `embed`,
/// so this is dead code on a wasm build without `embed` — gate it the same way
/// rather than warn there.
#[cfg(any(feature = "desktop", feature = "embed"))]
fn is_inline_surface(surface: &RenderSurfaceHandle) -> bool {
    surface.viewport_name.starts_with("__render_surface_")
}

/// Whether a GPU compositor presents the window: never on a desktop build
/// without `gpu`, and on a `gpu` build whenever the GPU renderer started
/// (`shell::renderer`), which is a run-time question since a `gpu` build can
/// fall back to software. Gated like its callers.
#[cfg(feature = "desktop")]
fn has_gpu_compositor() -> bool {
    #[cfg(feature = "gpu")]
    {
        crate::shell::renderer::gpu_presenting()
    }
    #[cfg(not(feature = "gpu"))]
    {
        false
    }
}

/// Whether a surface belongs on the **compositor** path — a layer on GPU, a
/// post-paint blit on software — rather than being painted inline during
/// `paint_document`.
///
/// The whole routing table, in one place, decided by backend × purpose:
///
/// | | `RenderSurface` | video | `GameViewport` |
/// |---|---|---|---|
/// | software | inline | inline (#358) | inline (#361) |
/// | GPU | inline | compositor + backdrop (#354) | compositor |
///
/// Software has **no** compositor path any more. It used to blit compositor
/// frames onto the *finished* pixel buffer, after the whole UI had been painted
/// and clipped only by the viewport's overflow-clipping ancestors — a write
/// with no notion of occlusion, which destroyed every overlay above a playing
/// video (#358) and every HUD, modal and dropdown above a `GameViewport`
/// (#361). Painted inline, a frame sits at its node's own z-order and ordinary
/// paint order does the occluding. GPU has no such problem: its layers are
/// blitted first and the Vello UI alpha-blends on top, so an opaque drawer
/// already covers the layer there.
///
/// A plain `const fn` of booleans rather than a `cfg`-gated branch, so both
/// rows of the table stay reachable to tests whichever backend the crate was
/// built for.
///
/// Gated like `is_inline_surface`: nothing on a wasm build has a compositor to
/// route to.
#[cfg(feature = "desktop")]
pub(crate) const fn surface_takes_compositor_path(is_inline: bool, gpu: bool) -> bool {
    !is_inline && gpu
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
            // both backends, plus every named viewport on software (#358, #361).
            if !surface_takes_compositor_path(
                is_inline_surface(surface),
                // "a GPU compositor presents", not "has the `gpu` feature": a
                // `gpu` build presents with software when the GPU would not
                // start, and software paints every viewport inline.
                has_gpu_compositor(),
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
                    buf.pixels.to_vec(),
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
#[cfg(any(feature = "desktop", feature = "embed"))]
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
                        data: Arc::clone(&buf.pixels),
                        width: buf.width,
                        height: buf.height,
                        opaque: buf.opaque,
                    },
                );
            }
        }
        map
    })
}

/// The frames a software paint draws **inline** at their `data-viewport`
/// nodes, and the viewports among them that punch a hole. See
/// [`collect_viewport_frames_by_name`].
#[cfg(feature = "desktop")]
#[derive(Default)]
pub struct ViewportFrames {
    /// Every frame to paint inline, keyed by `data-viewport` name — the map
    /// `rinch_dom::paint::set_viewport_pixels()` takes.
    pub frames: std::collections::HashMap<String, rinch_dom::paint::SurfacePixelData>,
    /// The names among [`Self::frames`] that are a `GameViewport`, and so cut a
    /// hole through their ancestors' backgrounds — the set
    /// `rinch_dom::paint::set_active_viewports()` takes. Video is never in it:
    /// it paints its own black letterbox instead (#354).
    pub holes: std::collections::HashSet<String>,
    /// The names among [`Self::frames`] that delivered a **new** frame since
    /// the last collection — the only viewports this paint has to repaint. The
    /// others are in `frames` only so a region that crosses them for another
    /// reason (a hover, a tick) can redraw the frame already on screen.
    pub fresh: std::collections::HashSet<String>,
}

/// Collect the frames the software backend paints **inline**, keyed by
/// `data-viewport` name: video (issue #358) and `GameViewport` (issue #361).
///
/// The software counterpart of [`collect_surface_pixels_by_id`]. The two
/// registries cannot share a key space: a `RenderSurface` component stamps its
/// `usize` surface id into `data-render-surface`, while a named viewport
/// carries only the name its surface was created with, so this one is keyed by
/// name and feeds `rinch_dom::paint::set_viewport_pixels()`.
///
/// Routed by [`surface_takes_compositor_path`], so it answers nothing while a
/// GPU compositor presents: there every named viewport is a compositor layer.
/// A surface with a GPU **texture source** is skipped on either backend — the
/// inline path has CPU pixels only, and such a surface belongs to the GPU
/// compositor that `gpu_handle()` exists for.
///
/// Returns every such surface with a non-empty buffer — not only the ones with
/// a *new* frame — because paint redraws the node whenever anything else on the
/// frame does. Clears dirty flags as a side effect, exactly as the other
/// collectors do.
#[cfg(feature = "desktop")]
pub fn collect_viewport_frames_by_name() -> ViewportFrames {
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        let mut out = ViewportFrames::default();
        for surface in reg.iter() {
            let is_inline = is_inline_surface(surface);
            if is_inline || surface_takes_compositor_path(is_inline, has_gpu_compositor()) {
                continue;
            }
            #[cfg(feature = "gpu")]
            if surface.texture_source.lock().unwrap().is_some() {
                continue;
            }
            let fresh = surface.needs_redraw.swap(false, Ordering::AcqRel);
            let buf = surface.buffer.lock().unwrap();
            if !buf.pixels.is_empty() {
                if fresh {
                    out.fresh.insert(surface.viewport_name.clone());
                }
                out.frames.insert(
                    surface.viewport_name.clone(),
                    rinch_dom::paint::SurfacePixelData {
                        data: Arc::clone(&buf.pixels),
                        width: buf.width,
                        height: buf.height,
                        opaque: buf.opaque,
                    },
                );
                if !surface.is_video {
                    out.holes.insert(surface.viewport_name.clone());
                }
            }
        }
        out
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
                buf.pixels = Arc::new(pixels);
                buf.width = width;
                buf.height = height;
                buf.opaque = false; // a texture's alpha is not inspected
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
    invoke_render_callbacks_where(|_| true);
}

/// [`invoke_render_callbacks`] for the surfaces `ids` only — one embedded
/// context's own, so a context sharing the thread does not run another's
/// callbacks from its `scene()` (issue #331).
pub fn invoke_render_callbacks_for(ids: &[usize]) {
    if !ids.is_empty() {
        invoke_render_callbacks_where(|id| ids.contains(&id));
    }
}

fn invoke_render_callbacks_where(wanted: impl Fn(usize) -> bool) {
    // Set guard so submit_frame() inside callbacks won't call request_repaint()
    // — we're already inside a paint cycle.
    #[cfg(feature = "desktop")]
    IN_RENDER_CALLBACK.with(|f| f.set(true));

    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        for surface in reg.iter() {
            if !wanted(surface.id) {
                continue;
            }
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

#[cfg(any(feature = "desktop", feature = "android", feature = "embed"))]
thread_local! {
    /// The pointer the desktop runtime is handing the app an event for; see
    /// [`set_current_pointer`].
    static CURRENT_POINTER: Cell<Option<SurfacePointer>> = const { Cell::new(None) };
}

#[cfg(any(
    feature = "desktop",
    feature = "android",
    feature = "embed",
    target_arch = "wasm32"
))]
thread_local! {
    /// Each surface's touches, for [`SurfaceEvent::Pinch`].
    static PINCHES: RefCell<Vec<(usize, PinchTracker)>> = const { RefCell::new(Vec::new()) };
}

/// The desktop runtime says which pointer the mouse events it hands the app
/// next come from (winit reports a pen, a finger and the mouse on one
/// stream). `None`: the mouse, which is also what an event injected without
/// one (a test, the debug server) is.
#[cfg(feature = "desktop")]
pub(crate) fn set_current_pointer(pointer: Option<SurfacePointer>) {
    CURRENT_POINTER.with(|c| c.set(pointer));
}

/// The pointer the event being handled comes from: what the runtime said,
/// else the mouse, with `down` deciding the mouse's pressure.
#[cfg(any(feature = "desktop", feature = "android", feature = "embed"))]
pub(crate) fn current_pointer(down: bool) -> SurfacePointer {
    CURRENT_POINTER
        .with(Cell::get)
        .unwrap_or(SurfacePointer::mouse(down))
}

/// Whether surface `id` is registered (created and not yet unregistered —
/// an unmounted `RenderSurface` unregisters itself).
#[cfg(any(feature = "desktop", feature = "android", feature = "embed"))]
pub(crate) fn surface_is_registered(id: usize) -> bool {
    SURFACE_REGISTRY.with(|reg| reg.borrow().iter().any(|s| s.id == id))
}

/// Whether surface `id` asked for pointer events
/// ([`RenderSurfaceHandle::set_pointer_events`]).
#[cfg(any(
    feature = "desktop",
    feature = "android",
    feature = "embed",
    target_arch = "wasm32"
))]
fn wants_pointer_events(id: usize) -> bool {
    SURFACE_REGISTRY.with(|reg| {
        reg.borrow()
            .iter()
            .any(|s| s.id == id && s.pointer_events.get())
    })
}

/// Deliver a press, move or release of `pointer` at `(x, y)` (surface-local)
/// to surface `id`: as `PointerDown`/`PointerMove`/`PointerUp` to a surface
/// that asked for pointer events, followed by a `Pinch` when it moved one
/// of two touches; as `MouseDown`/`MouseMove`/`MouseUp` to any other.
#[cfg(any(
    feature = "desktop",
    feature = "android",
    feature = "embed",
    target_arch = "wasm32"
))]
pub(crate) fn dispatch_surface_pointer(
    id: usize,
    phase: PointerPhase,
    x: f32,
    y: f32,
    pointer: SurfacePointer,
) {
    if !wants_pointer_events(id) {
        let event = match phase {
            PointerPhase::Down(button) => SurfaceEvent::MouseDown { x, y, button },
            PointerPhase::Move => SurfaceEvent::MouseMove { x, y },
            PointerPhase::Up(button) => SurfaceEvent::MouseUp { x, y, button },
        };
        dispatch_surface_event(id, event);
        return;
    }
    let event = match phase {
        PointerPhase::Down(button) => SurfaceEvent::PointerDown {
            x,
            y,
            button,
            pointer,
        },
        PointerPhase::Move => SurfaceEvent::PointerMove { x, y, pointer },
        PointerPhase::Up(button) => SurfaceEvent::PointerUp {
            x,
            y,
            button,
            pointer,
        },
    };
    dispatch_surface_event(id, event);
    if pointer.kind != SurfacePointerKind::Touch {
        return;
    }
    let pinch = with_pinch(id, |t| match phase {
        PointerPhase::Down(_) => {
            t.down(pointer.id, x, y);
            None
        }
        PointerPhase::Move => t.moved(pointer.id, x, y),
        PointerPhase::Up(_) => {
            t.up(pointer.id);
            None
        }
    });
    if let Some((x, y, scale)) = pinch {
        dispatch_surface_event(id, SurfaceEvent::Pinch { x, y, scale });
    }
}

/// The platform took `pointer` away mid-press (a touch the system
/// cancelled): `PointerCancel` to a surface that asked for pointer events, a
/// `MouseUp` at `(x, y)` to any other, so it is not left mid-drag.
#[cfg(any(
    feature = "desktop",
    feature = "android",
    feature = "embed",
    target_arch = "wasm32"
))]
pub(crate) fn dispatch_surface_pointer_cancel(id: usize, x: f32, y: f32, pointer: SurfacePointer) {
    if !wants_pointer_events(id) {
        let button = SurfaceMouseButton::Left;
        dispatch_surface_event(id, SurfaceEvent::MouseUp { x, y, button });
        return;
    }
    dispatch_surface_event(id, SurfaceEvent::PointerCancel { pointer });
    with_pinch(id, |t| t.up(pointer.id));
}

/// A zoom gesture over surface `id` at `(x, y)`: a trackpad pinch, or
/// Ctrl+wheel (what a browser turns a trackpad pinch into). Delivered as a
/// `Pinch` to a surface that asked for pointer events; answers whether it
/// was, so the caller delivers a wheel event as it always has otherwise.
#[cfg(any(
    feature = "desktop",
    feature = "android",
    feature = "embed",
    target_arch = "wasm32"
))]
pub(crate) fn dispatch_surface_zoom(id: usize, x: f32, y: f32, scale: f32) -> bool {
    if !wants_pointer_events(id) || !scale.is_finite() || scale <= 0.0 {
        return false;
    }
    dispatch_surface_event(id, SurfaceEvent::Pinch { x, y, scale });
    true
}

/// The scale a wheel turn of `delta_y` logical pixels zooms by: 100 px of
/// wheel is a factor of e, the curve browsers and most canvas apps use.
/// `delta_y` has the DOM's sign (`WheelEvent.deltaY`: positive is the wheel
/// turned towards the user, which zooms out); desktop negates winit's.
#[cfg(any(
    feature = "desktop",
    feature = "android",
    feature = "embed",
    target_arch = "wasm32"
))]
pub(crate) fn wheel_zoom_scale(delta_y: f32) -> f32 {
    (-delta_y / 100.0).exp()
}

#[cfg(any(
    feature = "desktop",
    feature = "android",
    feature = "embed",
    target_arch = "wasm32"
))]
fn with_pinch<R>(id: usize, f: impl FnOnce(&mut PinchTracker) -> R) -> R {
    PINCHES.with(|p| {
        let mut p = p.borrow_mut();
        let i = match p.iter().position(|(s, _)| *s == id) {
            Some(i) => i,
            None => {
                p.push((id, PinchTracker::default()));
                p.len() - 1
            }
        };
        f(&mut p[i].1)
    })
}

/// Asks the surface's [`RenderSurfaceHandle::set_key_handler`] whether `key`
/// is claimed (issue #482). This is independent of, and runs in addition to,
/// the normal [`dispatch_surface_event`] delivery of `KeyDown`/`KeyUp` — call
/// both, not one instead of the other.
///
/// Answers `false` — the fail-safe "not claimed" default — when the surface
/// has no key handler registered, or no surface with this id exists, so an
/// unclaimed key is never swallowed by a surface that never opted in.
pub fn dispatch_surface_key_event(id: usize, key: &SurfaceKeyData) -> bool {
    SURFACE_REGISTRY.with(|reg| {
        let reg = reg.borrow();
        reg.iter()
            .find(|s| s.id == id)
            .and_then(|s| s.key_handler.borrow().as_ref().map(|handler| handler(key)))
            .unwrap_or(false)
    })
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
fn pointer_of(event: &web_sys::PointerEvent) -> SurfacePointer {
    let kind = match event.pointer_type().as_str() {
        "mouse" => SurfacePointerKind::Mouse,
        // The eraser end of a pen sets `buttons` bit 5 (Pointer Events §
        // "The button property": 32 is the eraser button).
        "pen" if event.buttons() & 32 != 0 => SurfacePointerKind::Eraser,
        "pen" => SurfacePointerKind::Pen,
        "touch" => SurfacePointerKind::Touch,
        _ => SurfacePointerKind::Unknown,
    };
    SurfacePointer {
        id: event.pointer_id() as u32 as u64,
        kind,
        pressure: event.pressure().clamp(0.0, 1.0),
        primary: event.is_primary(),
    }
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
            dispatch_surface_pointer(
                surface_id,
                PointerPhase::Down(button),
                x,
                y,
                pointer_of(&event),
            );
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
            dispatch_surface_pointer(
                surface_id,
                PointerPhase::Up(button),
                x,
                y,
                pointer_of(&event),
            );
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback("pointerup", closure.as_ref().unchecked_ref())
            .unwrap();
        pointer_listeners.push(("pointerup", closure));
    }

    // pointercancel — the browser took the pointer away; release capture and end
    // the interaction (a `PointerCancel`, or like an up for a surface without
    // pointer events, so it isn't left mid-drag).
    {
        let canvas = canvas.clone();
        let closure = Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
            let x = event.offset_x() as f32;
            let y = event.offset_y() as f32;
            let _ = canvas.release_pointer_capture(event.pointer_id());
            dispatch_surface_pointer_cancel(surface_id, x, y, pointer_of(&event));
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
            dispatch_surface_pointer(surface_id, PointerPhase::Move, x, y, pointer_of(&event));
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
            // Ctrl+wheel is how the browser reports a trackpad pinch (and a
            // mouse's zoom chord): a `Pinch` to a surface with pointer events.
            if mouse.ctrl_key() {
                // Lines (Firefox's mouse wheel) as the desktop counts them.
                let px = if event.delta_mode() == web_sys::WheelEvent::DOM_DELTA_LINE {
                    delta_y * 40.0
                } else {
                    delta_y
                };
                if dispatch_surface_zoom(surface_id, x, y, wheel_zoom_scale(px)) {
                    return;
                }
            }
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

    /// `renderer::GPU_PRESENTING` is process-global, and two tests below flip
    /// it on a `gpu` build while the other one asks it: each holds this for its
    /// whole body, or one reads the other's `true` halfway through.
    static PRESENTING: Mutex<()> = Mutex::new(());

    fn presenting_lock() -> std::sync::MutexGuard<'static, ()> {
        PRESENTING.lock().unwrap_or_else(|e| e.into_inner())
    }

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
        // Only whether the surface is a `RenderSurface` component decides it
        // now: video and `GameViewport` route identically (#361).
        const RENDER_SURFACE: bool = true;
        const VIDEO: bool = false;
        const GAME_VIEWPORT: bool = false;

        for (label, inline, gpu, expected) in [
            ("RenderSurface / software", RENDER_SURFACE, SOFTWARE, false),
            ("RenderSurface / gpu", RENDER_SURFACE, GPU, false),
            ("video / software", VIDEO, SOFTWARE, false),
            ("video / gpu", VIDEO, GPU, true),
            ("GameViewport / software", GAME_VIEWPORT, SOFTWARE, false),
            ("GameViewport / gpu", GAME_VIEWPORT, GPU, true),
        ] {
            assert_eq!(
                surface_takes_compositor_path(inline, gpu),
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
        let _presenting = presenting_lock();
        let video = create_video_surface("test-video");
        video.writer().submit_frame(&[10, 20, 30, 255], 1, 1);

        let by_name = collect_viewport_frames_by_name();
        assert!(
            !by_name.holes.contains("test-video"),
            "video punches no hole: it paints its own black letterbox (#354)"
        );
        let frame = by_name
            .frames
            .get("test-video")
            .expect("the video frame is collected by viewport name");
        assert_eq!((frame.width, frame.height), (1, 1));
        assert_eq!(*frame.data, vec![10, 20, 30, 255]);

        let blitted = collect_surface_frames();
        assert_eq!(
            blitted.iter().any(|(name, ..)| name == "test-video"),
            has_gpu_compositor(),
            "video reaches the compositor path on GPU only — on software it \
             paints inline instead (#358)"
        );

        // A `gpu` build decides at run time: the same surface is routed to the
        // compositor once the GPU renderer is the one presenting.
        #[cfg(feature = "gpu")]
        {
            crate::shell::renderer::set_gpu_presenting(true);
            video.writer().submit_frame(&[10, 20, 30, 255], 1, 1);
            let blitted = collect_surface_frames();
            crate::shell::renderer::set_gpu_presenting(false);
            assert!(
                blitted.iter().any(|(name, ..)| name == "test-video"),
                "a gpu build presenting on the GPU routes video to the compositor"
            );
        }

        unregister_render_surface(video.id());
    }

    /// #361: a `GameViewport` follows video. On software its frame is
    /// collected by name for inline painting and never reaches the post-paint
    /// blit, which wrote it over every HUD, modal and dropdown above it; on GPU
    /// it keeps the compositor layer it has always had.
    #[test]
    fn a_game_viewport_frame_paints_inline_on_software() {
        let _presenting = presenting_lock();
        let game = create_render_surface_with_name("game");
        game.writer().submit_frame(&[1, 2, 3, 255], 1, 1);

        let software = !has_gpu_compositor();
        let inline = collect_viewport_frames_by_name();
        assert_eq!(
            inline.frames.contains_key("game"),
            software,
            "a GameViewport's frame is painted inline on software only"
        );
        assert_eq!(
            inline.holes.contains("game"),
            software,
            "and, painted inline, it still punches its hole"
        );
        game.writer().submit_frame(&[1, 2, 3, 255], 1, 1);
        assert_eq!(
            collect_surface_frames()
                .iter()
                .any(|(name, ..)| name == "game"),
            !software,
            "a GameViewport reaches the compositor path on GPU only"
        );

        // A `gpu` build decides at run time: once the GPU renderer presents,
        // the same surface is a compositor layer again and nothing is inline.
        #[cfg(feature = "gpu")]
        {
            crate::shell::renderer::set_gpu_presenting(true);
            game.writer().submit_frame(&[1, 2, 3, 255], 1, 1);
            let inline = collect_viewport_frames_by_name();
            game.writer().submit_frame(&[1, 2, 3, 255], 1, 1);
            let layers = collect_surface_frames();
            crate::shell::renderer::set_gpu_presenting(false);
            assert!(inline.frames.is_empty() && inline.holes.is_empty());
            assert!(layers.iter().any(|(name, ..)| name == "game"));
        }

        unregister_render_surface(game.id());
    }

    /// And a `RenderSurface` component keeps its own inline path, by id.
    #[test]
    fn a_render_surface_component_is_still_collected_by_id() {
        let _presenting = presenting_lock();
        let surface = create_render_surface();
        mount_render_surface(&surface);
        surface.writer().submit_frame(&[9, 9, 9, 255], 1, 1);

        assert!(collect_surface_pixels_by_id().contains_key(&surface.id()));
        assert!(collect_viewport_frames_by_name().frames.is_empty());
        assert!(collect_surface_frames().is_empty());

        unregister_render_surface(surface.id());
    }

    /// #349: a registered surface that unregisters asks for a redraw — the
    /// paint that takes its frame off the screen — and an id that names no
    /// registered surface asks for none.
    ///
    /// The callback is process-global and other tests' surfaces reach it from
    /// their own threads, so it records *who* called.
    #[test]
    fn unregistering_a_registered_surface_asks_for_a_redraw() {
        use std::sync::{Arc, Mutex};
        use std::thread::ThreadId;
        let callers: Arc<Mutex<Vec<ThreadId>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = callers.clone();
        set_redraw_callback(Arc::new(move || {
            sink.lock().unwrap().push(std::thread::current().id());
        }));
        let me = std::thread::current().id();
        let mine = || callers.lock().unwrap().iter().filter(|t| **t == me).count();

        // No frame submitted: the only thing that can ask is the unregister.
        let game = create_render_surface_with_name("game-349-redraw");
        assert_eq!(mine(), 0, "positive control: registering asks for nothing");
        unregister_render_surface(game.id());
        assert_eq!(mine(), 1, "the surface that went asked once");
        unregister_render_surface(game.id());
        assert_eq!(mine(), 1, "an id that is not registered asks for nothing");

        clear_redraw_callback();
    }
}

#[cfg(all(test, feature = "video"))]
#[path = "video_surface_lifetime_tests.rs"]
mod video_surface_lifetime_tests;
