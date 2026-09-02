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
    let scale = sane_scale(scale);
    (
        ((size.0 as f64 / scale).round() as u32).max(1),
        ((size.1 as f64 / scale).round() as u32).max(1),
    )
}

/// Convert a physical (device-pixel) *point* to a logical (CSS-pixel) one.
///
/// The pointer twin of [`to_logical`], and the conversion every shell owes the
/// runtime: `PlatformEvent`'s pointer coordinates are **logical on every host**
/// (see [`crate::PlatformEvent`]), because hit testing probes the layout tree
/// and the document is laid out in CSS pixels. A shell that forwards its
/// windowing system's physical pointer position untouched displaces every click
/// by the scale factor times its distance from the origin (issue #299).
///
/// Unlike [`to_logical`] this neither rounds nor clamps: a pointer position is
/// meaningfully subpixel, and a legal one can be negative (a drag that left the
/// window) or zero.
///
/// A non-finite or non-positive `scale` falls back to 1x rather than dividing
/// by zero, exactly as [`to_logical`] does.
pub fn to_logical_point(point: (f64, f64), scale: f64) -> (f64, f64) {
    let scale = sane_scale(scale);
    (point.0 / scale, point.1 / scale)
}

/// A scale factor safe to divide by: anything non-finite or non-positive
/// degrades to 1x rather than producing an infinity or a NaN.
fn sane_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A size rounds to whole logical pixels and never collapses to zero; a
    /// point keeps its fraction and its sign, because a pointer is not a size.
    #[test]
    fn a_point_is_not_rounded_or_clamped_the_way_a_size_is() {
        assert_eq!(to_logical((1600, 1200), 2.0), (800, 600));
        assert_eq!(to_logical((1, 1), 4.0), (1, 1), "a size never reaches 0");

        assert_eq!(to_logical_point((900.0, 441.0), 2.0), (450.0, 220.5));
        assert_eq!(
            to_logical_point((-8.0, 0.0), 2.0),
            (-4.0, 0.0),
            "a pointer that left the window keeps its negative coordinate"
        );
    }

    /// Both conversions degrade to 1x rather than dividing by zero or NaN.
    #[test]
    fn a_nonsense_scale_falls_back_to_1x() {
        for bad in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert_eq!(to_logical((800, 600), bad), (800, 600));
            assert_eq!(to_logical_point((100.0, 50.0), bad), (100.0, 50.0));
        }
    }

    /// The whole change this helper exists for is an arithmetic identity at
    /// scale 1.0 — which is why it is safe to put on every pointer path.
    #[test]
    fn scale_1_is_the_identity() {
        assert_eq!(to_logical_point((123.25, -7.5), 1.0), (123.25, -7.5));
    }
}
