// NOTE: Some functions in this module are not yet called by fine_grained_runtime.
// They're part of the programmatic window management API that will be implemented.
#![allow(dead_code)]

//! Window management API for opening and closing windows programmatically.
//!
//! # Example
//!
//! ```ignore
//! use rinch::prelude::*;
//! use rinch::windows::{open_window, close_window, WindowHandle};
//!
//! fn app() -> Element {
//!     let settings_window = Signal::new(None::<WindowHandle>);
//!     let settings_open = settings_window.clone();
//!     let settings_close = settings_window.clone();
//!
//!     rsx! {
//!         Window { title: "Main",
//!             button {
//!                 onclick: move || {
//!                     let handle = open_window(
//!                         WindowProps { title: "Settings".into(), width: 400, height: 300, ..Default::default() },
//!                         "<div>Settings content</div>".into()
//!                     );
//!                     settings_open.set(Some(handle));
//!                 },
//!                 "Open Settings"
//!             }
//!             button {
//!                 onclick: move || {
//!                     if let Some(handle) = settings_close.get() {
//!                         close_window(handle);
//!                         settings_close.set(None);
//!                     }
//!                 },
//!                 "Close Settings"
//!             }
//!         }
//!     }
//! }
//! ```

use rinch_core::element::WindowProps;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use crate::shell::rinch_runtime::{RinchNativeEvent, send_native_event};
use rinch_core::element::WindowSurfaceKind;

/// A handle to an open window.
///
/// This handle can be stored in signals and used to close the window later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowHandle(u64);

impl WindowHandle {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::SeqCst))
    }

    /// Get the internal ID of this handle.
    pub fn id(&self) -> u64 {
        self.0
    }
}

/// A request to open a new window.
#[derive(Debug, Clone)]
pub struct OpenWindowRequest {
    /// The handle that will identify this window.
    pub handle: WindowHandle,
    /// Window properties.
    pub props: WindowProps,
    /// HTML content for the window.
    pub html_content: String,
}

/// A request to close a window.
#[derive(Debug, Clone, Copy)]
pub struct CloseWindowRequest {
    /// The handle of the window to close.
    pub handle: WindowHandle,
}

/// Current state of a window (position, size).
///
/// This can be used by applications to save and restore window state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowState {
    /// X position of the window (outer position).
    pub x: i32,
    /// Y position of the window (outer position).
    pub y: i32,
    /// Width of the window content area.
    pub width: u32,
    /// Height of the window content area.
    pub height: u32,
    /// Whether the window is maximized.
    pub maximized: bool,
    /// Whether the window is minimized.
    pub minimized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
            maximized: false,
            minimized: false,
        }
    }
}

thread_local! {
    /// Pending window requests to be processed by the runtime.
    static WINDOW_REQUESTS: RefCell<Vec<WindowRequest>> = const { RefCell::new(Vec::new()) };
    /// Event loop proxy for triggering re-renders after window operations.
    static EVENT_PROXY: RefCell<Option<EventLoopProxy>> = const { RefCell::new(None) };
    /// Current state of all windows, updated by the runtime.
    static WINDOW_STATES: RefCell<HashMap<WindowHandle, WindowState>> = RefCell::new(HashMap::new());
    /// The window ID that is currently handling an event (set by runtime during event dispatch).
    static CURRENT_WINDOW_ID: RefCell<Option<WindowId>> = const { RefCell::new(None) };
}

/// Window request types.
#[derive(Debug, Clone)]
pub enum WindowRequest {
    Open(OpenWindowRequest),
    Close(CloseWindowRequest),
}

/// Set the event loop proxy (called by runtime during initialization).
pub(crate) fn set_event_proxy(proxy: EventLoopProxy) {
    EVENT_PROXY.with(|p| {
        *p.borrow_mut() = Some(proxy);
    });
}

/// Take all pending window requests (called by runtime).
pub(crate) fn take_window_requests() -> Vec<WindowRequest> {
    WINDOW_REQUESTS.with(|r| r.borrow_mut().drain(..).collect())
}

/// Update window state (called by runtime when window is moved/resized).
pub(crate) fn update_window_state(handle: WindowHandle, state: WindowState) {
    WINDOW_STATES.with(|s| {
        s.borrow_mut().insert(handle, state);
    });
}

/// Remove window state (called by runtime when window is closed).
pub(crate) fn remove_window_state(handle: WindowHandle) {
    WINDOW_STATES.with(|s| {
        s.borrow_mut().remove(&handle);
    });
}

/// Set the current window ID (called by runtime during event dispatch).
pub(crate) fn set_current_window_id(window_id: Option<WindowId>) {
    CURRENT_WINDOW_ID.with(|id| {
        *id.borrow_mut() = window_id;
    });
}

/// Get the current window ID (if any).
pub(crate) fn get_current_window_id() -> Option<WindowId> {
    CURRENT_WINDOW_ID.with(|id| *id.borrow())
}

/// Get the current state of a window.
///
/// Returns `None` if the window handle is invalid or the window has been closed.
///
/// # Example
///
/// ```ignore
/// use rinch::windows::{get_window_state, WindowHandle};
///
/// fn save_window_state(handle: WindowHandle) {
///     if let Some(state) = get_window_state(handle) {
///         // Save position and size to config file
///         println!("Window at ({}, {}), size {}x{}",
///             state.x, state.y, state.width, state.height);
///     }
/// }
/// ```
pub fn get_window_state(handle: WindowHandle) -> Option<WindowState> {
    WINDOW_STATES.with(|s| s.borrow().get(&handle).copied())
}

/// Get the states of all open windows.
///
/// Returns a vector of (handle, state) pairs for all windows opened programmatically.
pub fn get_all_window_states() -> Vec<(WindowHandle, WindowState)> {
    WINDOW_STATES.with(|s| s.borrow().iter().map(|(h, s)| (*h, *s)).collect())
}

/// Open a new window with the given properties and HTML content.
///
/// Returns a `WindowHandle` that can be used to close the window later.
///
/// # Example
///
/// ```ignore
/// use rinch::windows::open_window;
/// use rinch_core::element::WindowProps;
///
/// let handle = open_window(
///     WindowProps {
///         title: "New Window".into(),
///         width: 400,
///         height: 300,
///         ..Default::default()
///     },
///     "<h1>Hello from new window!</h1>".into()
/// );
/// ```
pub fn open_window(props: WindowProps, html_content: String) -> WindowHandle {
    let handle = WindowHandle::new();

    WINDOW_REQUESTS.with(|r| {
        r.borrow_mut().push(WindowRequest::Open(OpenWindowRequest {
            handle,
            props,
            html_content,
        }));
    });

    // Trigger processing of window requests
    EVENT_PROXY.with(|p| {
        if let Some(proxy) = p.borrow().as_ref() {
            proxy.wake_up();
        }
    });

    handle
}

/// Close a window by its handle.
///
/// # Example
///
/// ```ignore
/// use rinch::windows::{open_window, close_window};
///
/// let handle = open_window(props, content);
/// // ... later ...
/// close_window(handle);
/// ```
pub fn close_window(handle: WindowHandle) {
    WINDOW_REQUESTS.with(|r| {
        r.borrow_mut()
            .push(WindowRequest::Close(CloseWindowRequest { handle }));
    });

    // Trigger processing of window requests
    EVENT_PROXY.with(|p| {
        if let Some(proxy) = p.borrow().as_ref() {
            proxy.wake_up();
        }
    });
}

/// Open a window using a builder pattern.
///
/// # Example
///
/// ```ignore
/// use rinch::windows::WindowBuilder;
///
/// let handle = WindowBuilder::new()
///     .title("Settings")
///     .size(400, 300)
///     .position(100, 100)
///     .resizable(false)
///     .content("<div>Settings</div>")
///     .open();
/// ```
pub struct WindowBuilder {
    props: WindowProps,
    html_content: String,
}

impl WindowBuilder {
    /// Create a new window builder with default properties.
    pub fn new() -> Self {
        Self {
            props: WindowProps::default(),
            html_content: String::new(),
        }
    }

    /// Set the window title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.props.title = title.into();
        self
    }

    /// Set the window size.
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.props.width = width;
        self.props.height = height;
        self
    }

    /// Set the window position.
    pub fn position(mut self, x: i32, y: i32) -> Self {
        self.props.x = Some(x);
        self.props.y = Some(y);
        self
    }

    /// Set whether the window is resizable.
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.props.resizable = resizable;
        self
    }

    /// Set whether the window is borderless (frameless).
    pub fn borderless(mut self, borderless: bool) -> Self {
        self.props.borderless = borderless;
        self
    }

    /// Set whether the window is transparent.
    pub fn transparent(mut self, transparent: bool) -> Self {
        self.props.transparent = transparent;
        self
    }

    /// Set whether the window is always on top.
    pub fn always_on_top(mut self, always_on_top: bool) -> Self {
        self.props.always_on_top = always_on_top;
        self
    }

    /// Set the HTML content of the window.
    pub fn content(mut self, html: impl Into<String>) -> Self {
        self.html_content = html.into();
        self
    }

    /// Open the window and return a handle.
    pub fn open(self) -> WindowHandle {
        open_window(self.props, self.html_content)
    }
}

impl Default for WindowBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Window Control Functions (for the current window)
// =============================================================================

/// Minimize the current window.
///
/// Call this from an event handler (e.g., onclick) to minimize the window
/// that contains the element.
///
/// # Example
///
/// ```ignore
/// button { onclick: || minimize_current_window(), "Minimize" }
/// ```
pub fn minimize_current_window() {
    send_native_event(RinchNativeEvent::MinimizeWindow);
}

/// Toggle maximize state of the current window.
///
/// If the window is currently maximized, it will be restored to its previous size.
/// If not maximized, it will be maximized.
///
/// # Example
///
/// ```ignore
/// button { onclick: || toggle_maximize_current_window(), "Maximize" }
/// ```
pub fn toggle_maximize_current_window() {
    send_native_event(RinchNativeEvent::ToggleMaximizeWindow);
}

/// Hide the current window (minimize to tray pattern).
///
/// The window becomes invisible but the application continues running.
/// Use `show_current_window()` to make it visible again.
///
/// # Example
///
/// ```ignore
/// button { onclick: || hide_current_window(), "Hide to Tray" }
/// ```
pub fn hide_current_window() {
    send_native_event(RinchNativeEvent::HideWindow);
}

/// Show the current window (restore from tray).
///
/// Makes a previously hidden window visible again and brings it to focus —
/// except a [`WindowProps::non_activating`](rinch_core::element::WindowProps::non_activating)
/// window, which is shown without taking focus or activation. It comes back
/// where it was when it was hidden and at the size it had (issue #1546; the
/// size only on Wayland, where the compositor places windows), or where
/// [`set_current_window_position`] / [`set_current_window_size`] put it
/// meanwhile.
///
/// # Example
///
/// ```ignore
/// // In a tray menu callback:
/// MenuItem::new("Show").on_click(|| show_current_window())
/// ```
pub fn show_current_window() {
    send_native_event(RinchNativeEvent::ShowWindow);
}

/// Let pointer events pass through the current window to whatever is under
/// it (`true`), or make it take them again (`false`). Starts at
/// [`WindowProps::click_through`](rinch_core::element::WindowProps::click_through)
/// and is kept across [`hide_current_window`] / [`show_current_window`]
/// (issue #1519).
///
/// A click-through window receives no pointer events at all, so nothing in it
/// can turn click-through off again: toggle it from a global hotkey, a tray
/// menu, a timer or another thread (it is `Send`-safe, like the other window
/// controls).
///
/// # Example
///
/// ```ignore
/// MenuItem::new("Make overlay interactive")
///     .on_click(|| set_click_through_current_window(false))
/// ```
pub fn set_click_through_current_window(click_through: bool) {
    send_native_event(RinchNativeEvent::SetClickThrough(click_through));
}

/// The current window's outer (frame) top-left, in logical pixels on the
/// desktop — the position [`WindowProps::x`](rinch_core::element::WindowProps::x)
/// and [`WindowProps::y`](rinch_core::element::WindowProps::y) set (issue #1546).
///
/// Kept while the window is hidden ([`hide_current_window`]): it is where
/// [`show_current_window`] puts it back. So an app can read it at exit and
/// persist it. `None` before the window first exists, and on Wayland, where
/// a client cannot learn its window's position — except a layer-shell window
/// (issue #1523), whose position is its top-left on its output, once it has
/// one (from `WindowProps::x`/`y`, [`set_current_window_position`] or a
/// move); before that the compositor centres it and this is `None`.
/// Callable from any thread.
pub fn current_window_position() -> Option<(f64, f64)> {
    crate::shell::window_geometry::published().and_then(|g| g.position)
}

/// The current window's inner (content) size in logical pixels — the size
/// [`WindowProps::width`](rinch_core::element::WindowProps::width) and
/// [`WindowProps::height`](rinch_core::element::WindowProps::height) set
/// (issue #1546). Kept while the window is hidden, like
/// [`current_window_position`]. `None` before the window first exists.
pub fn current_window_size() -> Option<(f64, f64)> {
    crate::shell::window_geometry::published().and_then(|g| g.size)
}

/// How to bring the main window back as it is now, for an app that keeps
/// it across runs: the position and size it has when it is neither maximized
/// nor minimized (what an un-maximize returns to), and whether it is
/// maximized. Give them back as `WindowProps::x`/`y`, `width`/`height` and
/// `maximized`; a position out of reach on the monitors connected then (one
/// since unplugged) is left to the system.
///
/// Separate from [`WindowState`], which is the programmatic-window API's
/// *current* geometry per [`WindowHandle`]: this is the main window's
/// *normal* geometry, and its position is `None` where the platform gives
/// none (Wayland).
#[derive(Debug, Clone, PartialEq)]
pub struct WindowRestore {
    /// Outer top-left on the desktop, logical pixels; `None` on Wayland.
    pub position: Option<(f64, f64)>,
    /// Inner (content) size, logical pixels.
    pub size: (f64, f64),
    /// Whether the window is maximized.
    pub maximized: bool,
    /// The platform's name for the monitor `position` is on; give it back as
    /// `WindowProps::monitor`. On a desktop whose monitors have different
    /// scales a logical position can name a point on two of them, and this
    /// says which. `None` with no position, or where the platform names no
    /// monitor.
    pub monitor: Option<String>,
}

/// What [`WindowRestore`] says of the current window, to read at exit and
/// persist. `None` before the window first exists. Callable from any thread.
pub fn current_window_restore() -> Option<WindowRestore> {
    let g = crate::shell::window_geometry::published()?;
    Some(WindowRestore {
        position: g.normal_position,
        size: g.normal_size.or(g.size)?,
        maximized: g.maximized,
        monitor: g.normal_monitor,
    })
}

/// Move the current window's outer top-left to `(x, y)`, logical pixels on
/// the desktop (issue #1546). While the window is hidden, this is where
/// [`show_current_window`] puts it. Applied on the event loop's next turn, so
/// [`current_window_position`] answers the new position after that. Ignored
/// on Wayland, where a client cannot position its window — except a
/// layer-shell window (issue #1523), which is anchored to its output's
/// top-left corner with `(x, y)` as its margins: logical pixels relative to
/// its output ([`set_current_window_output`]), not the desktop.
pub fn set_current_window_position(x: f64, y: f64) {
    send_native_event(RinchNativeEvent::SetWindowPosition(x, y));
}

/// Resize the current window's inner (content) area to `width` x `height`
/// logical pixels (issue #1546). While the window is hidden, this is the size
/// [`show_current_window`] gives it. Applied on the event loop's next turn,
/// like [`set_current_window_position`]; the window manager may refuse or
/// adjust it (a tiling window manager, a size constraint).
pub fn set_current_window_size(width: f64, height: f64) {
    send_native_event(RinchNativeEvent::SetWindowSize(width, height));
}

/// What kind of surface the current window got (issue #1523): on native
/// Wayland a [`WindowProps::non_activating`](rinch_core::element::WindowProps::non_activating)
/// window (or one with `wayland_layer_shell: Some(true)`) is a
/// [`WindowSurfaceKind::LayerShell`] surface where the compositor supports
/// `wlr-layer-shell`, and a [`WindowSurfaceKind::Toplevel`] where it does not
/// (GNOME/Mutter). Every other window is `Toplevel`. Kept while the window is
/// hidden; `None` before the window first exists. Callable from any thread.
pub fn current_window_surface_kind() -> Option<WindowSurfaceKind> {
    crate::shell::layer_shell::published_surface_kind()
}

/// Show the current window on the output (monitor) with this connector name
/// (`"DP-1"`, `"eDP-1"`), or let the compositor choose (`None`) — for a
/// Wayland layer-shell window (issue #1523), whose position
/// ([`set_current_window_position`]) is relative to its output. The window is
/// made again on that output at the same position and size. Overrides
/// [`WindowProps::output`](rinch_core::element::WindowProps::output); no
/// effect on any other kind of window. Monitor names are what
/// `wl_output.name` reports, as `wlr-randr` / `kscreen-doctor -o` list them.
pub fn set_current_window_output(output: Option<&str>) {
    send_native_event(RinchNativeEvent::SetWindowOutput(
        output.map(str::to_string),
    ));
}

/// Start moving the current window with the pointer, from a handler of the
/// press that should move it (a custom title bar's `onmousedown`) — what a
/// `data-drag-window` region does (issue #1523). An ordinary window asks the
/// window manager for an interactive move. A Wayland layer-shell window has
/// none, so rinch follows the pointer until the button is released and moves
/// the window by its margins; [`current_window_position`] then answers where
/// it is. Ignored when no button is held (a timer, a press already over).
pub fn start_window_drag() {
    send_native_event(RinchNativeEvent::StartWindowDrag);
}

/// Start resizing the current window from `edge` with the pointer, from a
/// handler of the press on that edge — what `WindowProps::resize_inset` does
/// for a borderless window (issue #1523). A Wayland layer-shell window is
/// resized by rinch, following the pointer until the button is released (the
/// opposite edge stays put; 16 logical pixels at the least). Ignored when no
/// button is held.
pub fn start_window_resize(edge: rinch_platform::ResizeDirection) {
    send_native_event(RinchNativeEvent::StartWindowResize(edge));
}

/// Close the current window.
///
/// Call this from an event handler (e.g., onclick) to close the window
/// that contains the element.
///
/// # Example
///
/// ```ignore
/// button { onclick: || close_current_window(), "Close" }
/// ```
pub fn close_current_window() {
    send_native_event(RinchNativeEvent::CloseWindowControl);
}
