//! [`WindowRequest`]: a request an app's code made of its own window, as a
//! host that is not rinch's desktop shell reads it (`rinch::testing`'s
//! `TestApp`, an embedded `RinchContext`).

use rinch_core::element::LayerSurface;
use rinch_platform::ResizeDirection;

use crate::shell::rinch_runtime::RinchNativeEvent;

/// A request the app's code made of its own window: what
/// [`rinch::windows`](mod@crate::windows) and [`WindowHandle`](crate::WindowHandle)
/// queue for the desktop shell, as `rinch::testing`'s `TestApp` and an
/// embedded `RinchContext` report it (`take_window_requests` on each).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum WindowRequest {
    /// [`show_current_window`](crate::windows::show_current_window).
    Show,
    /// [`hide_current_window`](crate::windows::hide_current_window).
    Hide,
    /// [`prepare_current_window`](crate::windows::prepare_current_window), at
    /// the scale [`prepare_current_window_at_scale`](crate::windows::prepare_current_window_at_scale)
    /// named, if one.
    Prepare(Option<f64>),
    /// [`minimize_current_window`](crate::windows::minimize_current_window).
    Minimize,
    /// [`toggle_maximize_current_window`](crate::windows::toggle_maximize_current_window).
    ToggleMaximize,
    /// The window closing: its close control, or
    /// [`WindowHandle::close`](crate::WindowHandle::close).
    Close,
    /// [`WindowHandle::focus`](crate::WindowHandle::focus) or
    /// [`focus_current_window`](crate::windows::focus_current_window): bring
    /// the window to the front and give it the keyboard, showing it if it was
    /// hidden.
    Focus,
    /// [`WindowHandle::activate_with_token`](crate::WindowHandle::activate_with_token)
    /// or
    /// [`activate_current_window_with_token`](crate::windows::activate_current_window_with_token),
    /// with the token: [`Focus`](Self::Focus) with an xdg-activation token,
    /// and applied as it is.
    ActivateWithToken(String),
    /// [`set_click_through_current_window`](crate::windows::set_click_through_current_window).
    SetClickThrough(bool),
    /// [`set_current_window_position`](crate::windows::set_current_window_position),
    /// logical pixels.
    SetPosition(f64, f64),
    /// [`set_current_window_size`](crate::windows::set_current_window_size),
    /// logical pixels.
    SetSize(f64, f64),
    /// [`WindowHandle::set_title`](crate::WindowHandle::set_title).
    SetTitle(String),
    /// A layer surface's output (by name; `None` for the compositor's choice).
    SetOutput(Option<String>),
    /// A layer surface's new configuration.
    SetLayer(Box<LayerSurface>),
    /// Start moving the window with the pointer.
    StartDrag,
    /// Start resizing the window from an edge with the pointer.
    StartResize(ResizeDirection),
    /// An xdg-activation token asked for. Nothing answers it here.
    RequestActivationToken,
    /// [`rinch::exit`](crate::exit).
    Exit,
    /// A request for another window than the test's: one a
    /// [`WindowHandle`](crate::WindowHandle) names (a window the app opened
    /// with [`open_window`](crate::open_window)), by its
    /// [`id`](crate::WindowHandle::id). Recorded only: the harness drives one
    /// window, and never applies another's requests to it.
    OtherWindow {
        /// The window's [`WindowHandle::id`](crate::WindowHandle::id).
        window: u64,
        /// What was asked of it.
        request: Box<WindowRequest>,
    },
}

impl WindowRequest {
    /// The request a native event stands for; `None` for the events that are
    /// no request of the window's (a re-render, a debug command).
    pub(crate) fn from_native(event: RinchNativeEvent) -> Option<Self> {
        use RinchNativeEvent as E;
        Some(match event {
            E::ShowWindow => Self::Show,
            E::HideWindow => Self::Hide,
            E::PrepareWindow(scale) => Self::Prepare(scale),
            E::MinimizeWindow => Self::Minimize,
            E::ToggleMaximizeWindow => Self::ToggleMaximize,
            E::CloseWindowControl | E::CloseWindow => Self::Close,
            E::FocusWindow(None) => Self::Focus,
            E::FocusWindow(Some(token)) => Self::ActivateWithToken(token),
            E::SetClickThrough(on) => Self::SetClickThrough(on),
            E::SetWindowPosition(x, y) => Self::SetPosition(x, y),
            E::SetWindowSize(w, h) => Self::SetSize(w, h),
            E::SetWindowTitle(title) => Self::SetTitle(title),
            E::SetWindowOutput(output) => Self::SetOutput(output),
            E::SetWindowLayer(layer) => Self::SetLayer(layer),
            E::StartWindowDrag => Self::StartDrag,
            E::StartWindowResize(direction) => Self::StartResize(direction),
            E::RequestActivationToken(_) => Self::RequestActivationToken,
            E::Exit => Self::Exit,
            _ => return None,
        })
    }
}
