//! No-op stubs for window control functions on Android.

pub fn minimize_current_window() {}
pub fn toggle_maximize_current_window() {}
pub fn hide_current_window() {}
pub fn show_current_window() {}
pub fn close_current_window() {
    std::process::exit(0);
}
