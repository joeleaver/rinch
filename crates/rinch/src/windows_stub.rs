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
pub fn set_current_window_position(_x: f64, _y: f64) {}
pub fn set_current_window_size(_width: f64, _height: f64) {}
pub fn close_current_window() {
    std::process::exit(0);
}
