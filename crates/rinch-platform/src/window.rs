//! Platform window abstraction.

/// Abstraction over a platform window.
///
/// Each platform backend provides its own implementation:
/// - Desktop: wraps a `winit::window::Window`
/// - Web: wraps an HTML canvas element
/// - Mobile: wraps the platform's native window
pub trait PlatformWindow {
    /// Get the window's inner size in physical pixels.
    fn inner_size(&self) -> (u32, u32);

    /// Get the display scale factor (device pixel ratio).
    fn scale_factor(&self) -> f64;

    /// Request a redraw on the next frame.
    fn request_redraw(&self);

    /// Set whether the window is minimized.
    fn set_minimized(&self, minimized: bool);

    /// Set whether the window is maximized.
    fn set_maximized(&self, maximized: bool);

    /// Set whether the window is visible.
    fn set_visible(&self, visible: bool);

    /// Query whether the window is currently maximized.
    fn is_maximized(&self) -> bool;

    /// Initiate a window drag operation (for custom titlebars).
    ///
    /// Returns `Err` if the platform does not support programmatic window dragging.
    fn drag_window(&self) -> Result<(), String>;

    /// Initiate a window resize drag from an edge or corner.
    ///
    /// Returns `Err` if the platform does not support programmatic window resizing.
    fn drag_resize_window(&self, _direction: crate::ResizeDirection) -> Result<(), String> {
        Err("drag_resize_window not supported".into())
    }

    /// Set the window title.
    fn set_title(&self, title: &str);
}
