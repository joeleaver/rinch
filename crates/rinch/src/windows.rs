//! Window management API for opening and closing windows programmatically.
//!
//! # Example
//!
//! ```ignore
//! use rinch::prelude::*;
//! use rinch::windows::{open_window, close_window, WindowHandle};
//!
//! fn app() -> Element {
//!     let settings_window = use_signal(|| None::<WindowHandle>);
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

use crate::shell::runtime::RinchEvent;

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
    static WINDOW_REQUESTS: RefCell<Vec<WindowRequest>> = RefCell::new(Vec::new());
    /// Event loop proxy for triggering re-renders after window operations.
    static EVENT_PROXY: RefCell<Option<EventLoopProxy<RinchEvent>>> = RefCell::new(None);
    /// Current state of all windows, updated by the runtime.
    static WINDOW_STATES: RefCell<HashMap<WindowHandle, WindowState>> = RefCell::new(HashMap::new());
    /// The window ID that is currently handling an event (set by runtime during event dispatch).
    static CURRENT_WINDOW_ID: RefCell<Option<WindowId>> = RefCell::new(None);
}

/// Window request types.
#[derive(Debug, Clone)]
pub enum WindowRequest {
    Open(OpenWindowRequest),
    Close(CloseWindowRequest),
}

/// Set the event loop proxy (called by runtime during initialization).
pub(crate) fn set_event_proxy(proxy: EventLoopProxy<RinchEvent>) {
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
    WINDOW_STATES.with(|s| {
        s.borrow()
            .iter()
            .map(|(h, s)| (*h, *s))
            .collect()
    })
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
            let _ = proxy.send_event(RinchEvent::ProcessWindowRequests);
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
        r.borrow_mut().push(WindowRequest::Close(CloseWindowRequest { handle }));
    });

    // Trigger processing of window requests
    EVENT_PROXY.with(|p| {
        if let Some(proxy) = p.borrow().as_ref() {
            let _ = proxy.send_event(RinchEvent::ProcessWindowRequests);
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
    if let Some(window_id) = get_current_window_id() {
        EVENT_PROXY.with(|p| {
            if let Some(proxy) = p.borrow().as_ref() {
                let _ = proxy.send_event(RinchEvent::MinimizeWindow { window_id });
            }
        });
    }
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
    if let Some(window_id) = get_current_window_id() {
        EVENT_PROXY.with(|p| {
            if let Some(proxy) = p.borrow().as_ref() {
                let _ = proxy.send_event(RinchEvent::ToggleMaximizeWindow { window_id });
            }
        });
    }
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
    if let Some(window_id) = get_current_window_id() {
        EVENT_PROXY.with(|p| {
            if let Some(proxy) = p.borrow().as_ref() {
                let _ = proxy.send_event(RinchEvent::CloseWindowControl { window_id });
            }
        });
    }
}
