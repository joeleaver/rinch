//! Where the main window is and how big it is (issue #1546).
//!
//! Two jobs:
//!
//! - **Keep the geometry across hide / show.** `hide_current_window` destroys
//!   the OS window (Wayland has no invisible toplevel, and on macOS winit's
//!   `set_visible(true)` is `makeKeyAndOrderFront`, which would activate a
//!   non-activating panel — #1525), and `show_current_window` creates a new
//!   one. The new one used to take `WindowProps::width`/`height` and a
//!   system-chosen position, so a window the user had moved and resized came
//!   back somewhere else at its configured size. [`SavedGeometry::record`]
//!   reads the outgoing window and [`SavedGeometry::apply`] puts it on the
//!   attributes of the incoming one.
//! - **Publish it**, so an app can read it from any thread
//!   ([`crate::windows::current_window_position`] /
//!   [`crate::windows::current_window_size`]) and persist it.
//!
//! The outer position is kept in the unit the platform positions windows in:
//! physical pixels on X11 and Windows (desktop coordinates, which do not
//! depend on any monitor's scale), logical points on macOS (where winit
//! converts a physical position with a guessed scale). The inner size is kept
//! in logical pixels everywhere, so a window that comes back on a display with
//! another scale keeps its apparent size. Wayland gives a client no position
//! (`outer_position` is an error), so only the size comes back there and the
//! compositor places the window.

use std::sync::Mutex;

use winit::dpi::{LogicalPosition, Position, Size};
use winit::window::{Window, WindowAttributes};

/// The main window's geometry, recorded when it is hidden and applied when it
/// is created again.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct SavedGeometry {
    /// Outer (frame) position; `None` where the platform reports none
    /// (Wayland), or nothing was recorded yet.
    pub position: Option<Position>,
    /// Inner (surface) size, logical.
    pub size: Option<Size>,
    /// Whether the window was maximized: it comes back maximized, and
    /// `position` / `size` keep the geometry it had before.
    pub maximized: bool,
}

impl SavedGeometry {
    /// Record `window` over what was saved before. A maximized window keeps
    /// the earlier position and size (its maximized geometry is not the one
    /// to restore when it is un-maximized) and only notes that it was
    /// maximized.
    pub(crate) fn record(&mut self, window: &dyn Window) {
        if window.is_maximized() {
            self.maximized = true;
            return;
        }
        self.maximized = false;
        let scale = window.scale_factor();
        if let Some(pos) = super::overlay_window::outer_position(window) {
            self.position = Some(if cfg!(target_os = "macos") {
                Position::Logical(pos.to_logical::<f64>(scale))
            } else {
                Position::Physical(pos)
            });
        }
        let size = window.surface_size();
        if size.width > 0 && size.height > 0 {
            self.size = Some(Size::Logical(size.to_logical::<f64>(scale)));
        }
    }

    /// The attributes for a (re-)creation of the main window: the saved
    /// geometry where there is one, otherwise `requested` — the
    /// `WindowProps::x`/`y` position, which only a first appearance takes
    /// (`first`); a later show without a recorded position is placed by the
    /// system, as before.
    pub(crate) fn apply(
        &self,
        mut attrs: WindowAttributes,
        requested: Option<(i32, i32)>,
        first: bool,
    ) -> WindowAttributes {
        if let Some(size) = self.size {
            attrs = attrs.with_surface_size(size);
        }
        let requested = requested
            .filter(|_| first)
            .map(|(x, y)| Position::Logical(LogicalPosition::new(x as f64, y as f64)));
        if let Some(position) = self.position.or(requested) {
            attrs = attrs.with_position(position);
        }
        if self.maximized {
            attrs = attrs.with_maximized(true);
        }
        attrs
    }
}

/// The main window's geometry as an app reads it: logical pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct PublishedGeometry {
    /// Outer position; `None` on Wayland.
    pub position: Option<(f64, f64)>,
    /// Inner size.
    pub size: Option<(f64, f64)>,
}

/// The last geometry the shell saw for the main window, kept while it is
/// hidden. `None` until the main window first exists.
static PUBLISHED: Mutex<Option<PublishedGeometry>> = Mutex::new(None);

/// Publish the geometry of the live main window.
pub(crate) fn publish_window(window: &dyn Window) {
    let scale = window.scale_factor();
    let position = super::overlay_window::outer_position(window).map(|p| {
        let l = p.to_logical::<f64>(scale);
        (l.x, l.y)
    });
    let s = window.surface_size().to_logical::<f64>(scale);
    publish(PublishedGeometry {
        position,
        size: Some((s.width, s.height)),
    });
}

/// Publish a position or size the app set while the window was hidden: what
/// it will come back with. Logical pixels.
pub(crate) fn publish_hidden(position: Option<(f64, f64)>, size: Option<(f64, f64)>) {
    let mut g = published().unwrap_or_default();
    if position.is_some() {
        g.position = position;
    }
    if size.is_some() {
        g.size = size;
    }
    publish(g);
}

fn publish(g: PublishedGeometry) {
    *PUBLISHED.lock().unwrap_or_else(|p| p.into_inner()) = Some(g);
}

/// The published geometry.
pub(crate) fn published() -> Option<PublishedGeometry> {
    *PUBLISHED.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::{LogicalSize, PhysicalPosition};

    fn attrs() -> WindowAttributes {
        WindowAttributes::default().with_surface_size(LogicalSize::new(560.0, 280.0))
    }

    /// Nothing recorded: the configured size, the requested position on a
    /// first appearance only.
    #[test]
    fn with_nothing_saved_a_first_show_takes_the_requested_position_and_a_later_one_none() {
        let none = SavedGeometry::default();
        let first = none.apply(attrs(), Some((40, 50)), true);
        assert_eq!(
            first.position,
            Some(Position::Logical(LogicalPosition::new(40.0, 50.0)))
        );
        assert_eq!(
            first.surface_size,
            Some(Size::Logical(LogicalSize::new(560.0, 280.0)))
        );
        assert_eq!(none.apply(attrs(), Some((40, 50)), false).position, None);
        assert_eq!(none.apply(attrs(), None, true).position, None);
    }

    /// A recorded geometry wins over the configured size and the requested
    /// position, on a first show (set while a hidden start) and a later one.
    #[test]
    fn a_saved_geometry_wins_over_the_props() {
        let saved = SavedGeometry {
            position: Some(Position::Physical(PhysicalPosition::new(300, 378))),
            size: Some(Size::Logical(LogicalSize::new(700.0, 200.0))),
            maximized: false,
        };
        for first in [true, false] {
            let a = saved.apply(attrs(), Some((40, 50)), first);
            assert_eq!(a.position, saved.position);
            assert_eq!(a.surface_size, saved.size);
            assert!(!a.maximized);
        }
    }

    /// Wayland records no position: the size comes back alone.
    #[test]
    fn a_size_without_a_position_restores_the_size_only() {
        let saved = SavedGeometry {
            position: None,
            size: Some(Size::Logical(LogicalSize::new(700.0, 200.0))),
            maximized: false,
        };
        let a = saved.apply(attrs(), Some((40, 50)), false);
        assert_eq!(a.position, None);
        assert_eq!(a.surface_size, saved.size);
    }

    #[test]
    fn a_maximized_window_comes_back_maximized() {
        let saved = SavedGeometry {
            maximized: true,
            ..Default::default()
        };
        assert!(saved.apply(attrs(), None, false).maximized);
    }
}
