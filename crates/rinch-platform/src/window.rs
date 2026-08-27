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

    /// The window's inner size in logical (CSS) pixels — the layout viewport.
    ///
    /// Documents are laid out in CSS pixels and paint multiplies every layout
    /// coordinate by [`Self::scale_factor`], so layout must be handed *this*,
    /// never [`Self::inner_size`]. See [`to_logical`].
    fn logical_size(&self) -> (u32, u32) {
        to_logical(self.inner_size(), self.scale_factor())
    }

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

/// Convert a physical (device-pixel) size to a logical (CSS-pixel) one.
///
/// The single conversion every shell shares. A document is laid out in CSS
/// pixels and paint multiplies each layout coordinate by the scale factor, so
/// the layout viewport must be the *logical* size: handing layout the physical
/// size lays the page out `scale` times too wide and paint then scales it up
/// again, pushing the right edge off the surface (which reads as a flex sizing
/// fault even though every box measured correctly), and sizes a software
/// framebuffer `scale` times larger than the surface.
///
/// A non-finite or non-positive `scale` falls back to 1x rather than dividing
/// by zero, and the result never collapses to zero.
pub fn to_logical(size: (u32, u32), scale: f64) -> (u32, u32) {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    (
        ((size.0 as f64 / scale).round() as u32).max(1),
        ((size.1 as f64 / scale).round() as u32).max(1),
    )
}
