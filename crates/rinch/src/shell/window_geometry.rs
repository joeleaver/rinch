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

use winit::dpi::{LogicalPosition, PhysicalPosition, PhysicalSize, Position, Size};
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

/// What the shell reads off the live window to keep [`SavedGeometry`]
/// current. Separate from the window so the rules are tested without one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WindowReading {
    pub maximized: bool,
    /// `Some(true)` only where the platform says so. A minimized window's
    /// position is no place to come back to: Windows reports
    /// (-32000, -32000) for it.
    pub minimized: bool,
    pub scale: f64,
    /// Outer position in desktop pixels, `None` on Wayland.
    pub position: Option<PhysicalPosition<i32>>,
    pub surface: PhysicalSize<u32>,
}

impl WindowReading {
    pub(crate) fn of(window: &dyn Window) -> Self {
        WindowReading {
            maximized: window.is_maximized(),
            minimized: window.is_minimized() == Some(true),
            scale: window.scale_factor(),
            position: super::overlay_window::outer_position(window),
            surface: window.surface_size(),
        }
    }

    /// Whether this is the window's normal (restore) geometry.
    fn is_normal(&self) -> bool {
        !self.maximized && !self.minimized
    }
}

impl SavedGeometry {
    /// Keep the normal geometry current while the window is live: called on
    /// creation, `Moved`, `SurfaceResized` and `ScaleFactorChanged`. A
    /// maximized or minimized window's geometry is not the one to come back
    /// to, so it changes nothing; the geometry from before it was maximized
    /// stays, which is what an un-maximize after hide / show restores.
    pub(crate) fn note(&mut self, r: &WindowReading) {
        if !r.is_normal() {
            return;
        }
        if let Some(pos) = r.position {
            self.position = Some(if cfg!(target_os = "macos") {
                Position::Logical(pos.to_logical::<f64>(r.scale))
            } else {
                Position::Physical(pos)
            });
        }
        if r.surface.width > 0 && r.surface.height > 0 {
            // Logical, so a window shown again on a display with another
            // scale keeps its apparent size.
            self.size = Some(Size::Logical(r.surface.to_logical::<f64>(r.scale)));
        }
    }

    /// At hide: the last normal geometry, and whether the window is
    /// maximized (it comes back maximized).
    pub(crate) fn record(&mut self, r: &WindowReading) {
        self.note(r);
        self.maximized = r.maximized;
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

/// A connected monitor, in desktop pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MonitorRect {
    pub position: PhysicalPosition<i32>,
    pub size: PhysicalSize<u32>,
    pub scale: f64,
}

/// Whether a window at `position` (outer top-left) of `size` overlaps one of
/// `monitors`. `true` when the monitors are unknown (an empty list), so a
/// platform that lists none keeps the position. A logical position (macOS
/// points) is compared in each monitor's own scale.
pub(crate) fn on_some_monitor(
    position: Position,
    size: Option<Size>,
    monitors: &[MonitorRect],
) -> bool {
    if monitors.is_empty() {
        return true;
    }
    monitors.iter().any(|m| {
        let p: PhysicalPosition<i32> = position.to_physical(m.scale);
        let (w, h): (u32, u32) = size
            .map(|s| s.to_physical::<u32>(m.scale).into())
            .unwrap_or((1, 1));
        let (mx, my) = (m.position.x as i64, m.position.y as i64);
        let (x, y) = (p.x as i64, p.y as i64);
        x < mx + m.size.width as i64
            && x + w.max(1) as i64 > mx
            && y < my + m.size.height as i64
            && y + h.max(1) as i64 > my
    })
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
pub(crate) fn publish_window(r: &WindowReading) {
    let mut g = published().unwrap_or_default();
    // A minimized window keeps the position it had (Windows reports
    // (-32000, -32000) while minimized, which an app persisting it would
    // restore off screen); Wayland has none.
    if !r.minimized {
        g.position = r.position.map(|p| {
            let l = p.to_logical::<f64>(r.scale);
            (l.x, l.y)
        });
        if r.surface.width > 0 && r.surface.height > 0 {
            let s = r.surface.to_logical::<f64>(r.scale);
            g.size = Some((s.width, s.height));
        }
    }
    publish(g);
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
    use winit::dpi::LogicalSize;

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

    fn reading(scale: f64, pos: (i32, i32), size: (u32, u32)) -> WindowReading {
        WindowReading {
            maximized: false,
            minimized: false,
            scale,
            position: Some(PhysicalPosition::new(pos.0, pos.1)),
            surface: PhysicalSize::new(size.0, size.1),
        }
    }

    /// The size is kept in logical pixels: a window hidden at scale 2 and
    /// shown at scale 1 keeps its apparent size, 350x100 logical, not
    /// 700x200 physical (a size kept physical would come back twice as big).
    #[test]
    fn the_size_survives_a_scale_change_in_logical_pixels() {
        let mut saved = SavedGeometry::default();
        saved.record(&reading(2.0, (300, 378), (700, 200)));
        let a = saved.apply(attrs(), None, false);
        let at_1: PhysicalSize<u32> = a.surface_size.unwrap().to_physical(1.0);
        assert_eq!(at_1, PhysicalSize::new(350, 100));
        let at_15: PhysicalSize<u32> = a.surface_size.unwrap().to_physical(1.5);
        assert_eq!(at_15, PhysicalSize::new(525, 150));
    }

    /// The normal geometry follows the live window and is kept through a
    /// maximize: an un-maximize after hide / show returns to the latest
    /// normal geometry, also when the first hide is while maximized.
    #[test]
    fn a_maximized_or_minimized_window_keeps_the_latest_normal_geometry() {
        let mut saved = SavedGeometry::default();
        saved.note(&reading(1.0, (220, 224), (560, 280)));
        saved.note(&reading(1.0, (120, 140), (600, 300)));
        let maxed = WindowReading {
            maximized: true,
            ..reading(1.0, (0, 0), (1280, 800))
        };
        saved.note(&maxed);
        saved.record(&maxed);
        assert!(saved.maximized);
        assert_eq!(
            saved.position,
            Some(Position::Physical(PhysicalPosition::new(120, 140)))
        );
        // Windows reports a minimized window at (-32000, -32000) and 0x0.
        let minimized = WindowReading {
            minimized: true,
            ..reading(1.0, (-32000, -32000), (0, 0))
        };
        saved.record(&minimized);
        assert!(!saved.maximized);
        assert_eq!(
            saved.position,
            Some(Position::Physical(PhysicalPosition::new(120, 140)))
        );
        assert_eq!(
            saved.size,
            Some(Size::Logical(LogicalSize::new(600.0, 300.0)))
        );
    }

    #[test]
    fn a_position_off_every_monitor_is_dropped() {
        let monitors = [
            MonitorRect {
                position: PhysicalPosition::new(0, 0),
                size: PhysicalSize::new(1920, 1080),
                scale: 1.0,
            },
            MonitorRect {
                position: PhysicalPosition::new(1920, 0),
                size: PhysicalSize::new(2560, 1440),
                scale: 2.0,
            },
        ];
        let size = Some(Size::Logical(LogicalSize::new(400.0, 300.0)));
        let at = |x, y| Position::Physical(PhysicalPosition::new(x, y));
        assert!(on_some_monitor(at(300, 378), size, &monitors));
        assert!(on_some_monitor(at(3000, 1000), size, &monitors));
        // Overlapping the left monitor from off its left edge.
        assert!(on_some_monitor(at(-200, 100), size, &monitors));
        // A third monitor that is gone.
        assert!(!on_some_monitor(at(5000, 100), size, &monitors));
        assert!(!on_some_monitor(at(-32000, -32000), size, &monitors));
        assert!(on_some_monitor(at(5000, 100), size, &[]), "unknown: kept");
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
