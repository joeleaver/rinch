//! No-op stubs for window control functions on Android.

pub fn minimize_current_window() {}
pub fn toggle_maximize_current_window() {}
pub fn hide_current_window() {}
pub fn show_current_window() {}
pub fn set_click_through_current_window(_click_through: bool) {}
pub fn current_window_position() -> Option<(f64, f64)> {
    None
}
pub fn current_window_size() -> Option<(f64, f64)> {
    None
}
/// The desktop's `WindowRestore`; nothing here has one to give.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowRestore {
    pub position: Option<(f64, f64)>,
    pub size: (f64, f64),
    pub maximized: bool,
    /// The platform's name for the monitor `position` is on; give it back as
    /// `WindowProps::monitor`. On a desktop whose monitors have different
    /// scales a logical position can name a point on two of them, and this
    /// says which. `None` with no position, or where the platform names no
    /// monitor.
    pub monitor: Option<String>,
}
pub fn current_window_restore() -> Option<WindowRestore> {
    None
}
pub fn set_current_window_position(_x: f64, _y: f64) {}
pub fn set_current_window_size(_width: f64, _height: f64) {}
pub fn close_current_window() {
    std::process::exit(0);
}
pub fn current_window_surface_kind() -> Option<rinch_core::element::WindowSurfaceKind> {
    None
}
pub fn set_current_window_output(_output: Option<&str>) {}
pub fn start_window_drag() {}
pub fn start_window_resize(_edge: rinch_platform::ResizeDirection) {}
