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
use std::time::{Duration, Instant};

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
    /// `WindowProps::x`/`y` position as [`place_requested`] resolved it, which
    /// only a first appearance takes (`first`); a later show without a
    /// recorded position is placed by the system, as before. A first
    /// appearance is maximized when `WindowProps::maximized` asks
    /// (`requested_maximized`).
    pub(crate) fn apply(
        &self,
        mut attrs: WindowAttributes,
        requested: Option<Position>,
        requested_maximized: bool,
        first: bool,
    ) -> WindowAttributes {
        if let Some(size) = self.size {
            attrs = attrs.with_surface_size(size);
        }
        let requested = requested.filter(|_| first);
        if let Some(position) = self.position.or(requested) {
            attrs = attrs.with_position(position);
        }
        if self.maximized || (first && requested_maximized) {
            attrs = attrs.with_maximized(true);
        }
        attrs
    }
}

/// A connected monitor, in the unit the platform positions windows in:
/// desktop pixels on X11 and Windows, points on macOS (where
/// `connected_monitors` gives every monitor scale 1, so a logical position is
/// compared as the points it is).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MonitorRect {
    pub position: PhysicalPosition<i32>,
    pub size: PhysicalSize<u32>,
    pub scale: f64,
    /// The platform's name for it (winit's `MonitorHandle::name`).
    pub name: Option<String>,
}

/// The part of a window's top that must be on the desktop for the person to
/// reach it (to grab its title bar and move it), logical pixels: this wide
/// (or the whole width of a narrower window) and this tall (or the whole
/// height of a shorter one), anywhere in the top [`GRIP_STRIP`] of the window.
const GRIP: (f64, f64) = (64.0, 12.0);

/// How far down the window's top the [`GRIP`] may sit, logical pixels: a
/// frame a few pixels above the monitor (Windows' invisible outer border)
/// leaves the title bar below it reachable.
const GRIP_STRIP: f64 = 24.0;

/// Whether a window at `position` (outer top-left) of `size` can be reached
/// on `monitors`: its [`GRIP`] is on the desktop (the union of the monitors,
/// so a window across two adjacent monitors counts). A window that merely
/// overlaps a monitor with its bottom edge, or a sliver of its side, cannot
/// be moved back by hand. `true` when the monitors are unknown (an empty
/// list), so a platform that lists none keeps the position. See [`landing`]
/// for a logical position.
pub(crate) fn reachable(position: Position, size: Option<Size>, monitors: &[MonitorRect]) -> bool {
    monitors.is_empty() || landing(position, size, monitors).is_some()
}

/// Where [`place_requested`] puts a window.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Placed {
    /// What winit is given.
    pub at: Position,
    /// The monitor whose scale the position was read in, by its name.
    pub monitor: Option<String>,
}

/// Where to create a window that `WindowProps::x`/`y` asks for at `logical`,
/// or `None` when it would be out of reach there (the system places it).
///
/// A logical desktop position is the outer position over the scale of the
/// monitor the window is on (what `current_window_position` publishes), so
/// on a desktop whose monitors have different scales it is resolved here,
/// against the monitors, into the desktop pixels it names, and winit gets
/// those: winit itself converts a logical position with a *guessed* scale
/// (X11: the monitor under the pointer; Windows: the scale the new window
/// starts with), which can put it on no monitor at all. On macOS, where
/// points are the desktop's own unit, the logical position is kept.
///
/// Such a position can name a reachable point on two monitors (a window at
/// desktop (2100, 100) on a scale-2 monitor right of a 1920-wide scale-1 one
/// publishes (1050, 50), which is also on the scale-1 one). `monitor` — the
/// name `current_window_restore` gave with it (`WindowProps::monitor`) —
/// settles it: the monitors of that name are tried first, in the platform's
/// order (two of one name: the first on which the window is reachable), then
/// every monitor in that order, as without a name.
pub(crate) fn place_requested(
    logical: (f64, f64),
    size: Size,
    monitor: Option<&str>,
    monitors: &[MonitorRect],
) -> Option<Placed> {
    let position = Position::Logical(LogicalPosition::new(logical.0, logical.1));
    if monitors.is_empty() {
        return Some(Placed {
            at: position,
            monitor: None,
        });
    }
    let named = monitors
        .iter()
        .filter(|m| monitor.is_some() && m.name.as_deref() == monitor);
    let (at, m) = named
        .chain(monitors.iter())
        .find_map(|m| lands_on(position, Some(size), m, monitors).map(|p| (p, m)))?;
    Some(Placed {
        at: if cfg!(target_os = "macos") {
            position
        } else {
            Position::Physical(at)
        },
        monitor: m.name.clone(),
    })
}

/// The desktop pixels a window at `position` of `size` lands on, when its
/// [`GRIP`] is on the desktop there. A logical position names a point in the
/// scale of the monitor the window is on, so each monitor in turn is tried
/// as that monitor ([`lands_on`]). On a mixed-scale desktop a logical
/// position can name a reachable point on two monitors; the first in the
/// platform's monitor order wins. A physical position is the same on every
/// monitor; the grip is sized in the scale of a monitor its top is on.
fn landing(
    position: Position,
    size: Option<Size>,
    monitors: &[MonitorRect],
) -> Option<PhysicalPosition<i32>> {
    monitors
        .iter()
        .find_map(|m| lands_on(position, size, m, monitors))
}

/// Where a window at `position` of `size` lands when the position is read in
/// the scale of `m`: there, if its top strip is on `m` and its grip on the
/// desktop (`monitors`).
fn lands_on(
    position: Position,
    size: Option<Size>,
    m: &MonitorRect,
    monitors: &[MonitorRect],
) -> Option<PhysicalPosition<i32>> {
    let p: PhysicalPosition<i32> = position.to_physical(m.scale);
    let (w, h) = match size {
        Some(s) => {
            let s: PhysicalSize<u32> = s.to_physical(m.scale);
            (s.width.max(1) as i64, s.height.max(1) as i64)
        }
        // Unknown: one column, and as tall as anything.
        None => (1, i64::MAX),
    };
    let grip_w = ((GRIP.0 * m.scale) as i64).clamp(1, w);
    let grip_h = ((GRIP.1 * m.scale) as i64).clamp(1, h);
    let strip_h = ((GRIP_STRIP * m.scale) as i64).clamp(grip_h, h);
    let (x, y) = (p.x as i64, p.y as i64);
    let strip = Span {
        x0: x,
        x1: x.saturating_add(w),
        y0: y,
        y1: y.saturating_add(strip_h),
    };
    (strip.overlaps(m) && grip_on_desktop(strip, grip_w, grip_h, monitors)).then_some(p)
}

/// A rectangle in desktop pixels, `[x0, x1) x [y0, y1)`.
#[derive(Debug, Clone, Copy)]
struct Span {
    x0: i64,
    x1: i64,
    y0: i64,
    y1: i64,
}

impl Span {
    fn of(m: &MonitorRect) -> Span {
        let (x, y) = (m.position.x as i64, m.position.y as i64);
        Span {
            x0: x,
            x1: x + m.size.width as i64,
            y0: y,
            y1: y + m.size.height as i64,
        }
    }

    fn overlaps(&self, m: &MonitorRect) -> bool {
        let o = Span::of(m);
        self.x0.max(o.x0) < self.x1.min(o.x1) && self.y0.max(o.y0) < self.y1.min(o.y1)
    }
}

/// Whether a `w` x `h` rectangle inside `strip` is wholly covered by the
/// union of `monitors`. Its top can be taken at the strip's top or at a
/// monitor's top edge (moving a covered rectangle up keeps it covered until
/// it meets one of those), and for each such row band the longest run of
/// covered columns is measured over the slabs between monitor edges.
fn grip_on_desktop(strip: Span, w: i64, h: i64, monitors: &[MonitorRect]) -> bool {
    let rects: Vec<Span> = monitors.iter().map(Span::of).collect();
    let mut xs: Vec<i64> = vec![strip.x0, strip.x1];
    for r in &rects {
        for x in [r.x0, r.x1] {
            if x > strip.x0 && x < strip.x1 {
                xs.push(x);
            }
        }
    }
    xs.sort_unstable();
    xs.dedup();
    let tops = std::iter::once(strip.y0).chain(
        rects
            .iter()
            .map(|r| r.y0)
            .filter(|&t| t > strip.y0 && t.saturating_add(h) <= strip.y1),
    );
    for top in tops {
        let bottom = top.saturating_add(h);
        if bottom > strip.y1 {
            continue;
        }
        let mut run = 0;
        for slab in xs.windows(2) {
            let (a, b) = (slab[0], slab[1]);
            let mut ys: Vec<(i64, i64)> = rects
                .iter()
                .filter(|r| r.x0 <= a && r.x1 >= b)
                .map(|r| (r.y0, r.y1))
                .collect();
            ys.sort_unstable();
            let mut reach = top;
            for (y0, y1) in ys {
                if y0 > reach {
                    break;
                }
                reach = reach.max(y1);
            }
            if reach >= bottom {
                run += b - a;
                if run >= w {
                    return true;
                }
            } else {
                run = 0;
            }
        }
    }
    false
}

/// How long after its creation a window created maximized may report a
/// maximize it has not yet said it has (see [`MaximizeWait`]). The X11 window
/// manager maximizes a window as it maps it, within milliseconds; a reading
/// after this is believed.
pub(crate) const MAXIMIZE_GRACE: Duration = Duration::from_secs(2);

/// A window created maximized, until the platform says it is: the window
/// manager of X11 maximizes it some time after the creation, and the resize
/// that does it can arrive while `is_maximized` still answers `false` (winit
/// reads the `_NET_WM_STATE` property, and no event follows when it
/// changes). Believed, that reading would record the full-screen size as the
/// normal geometry and the window as not maximized.
///
/// So for [`MAXIMIZE_GRACE`] after the creation, a reading that does not say
/// maximized is taken as maximized while the window is at its created size
/// (before the maximize) or at the first other size it reached (the
/// maximize). The wait ends — every reading believed from then on — at the
/// first reading that says maximized, at the end of the grace, or at a
/// reading at yet another size, or back at the created one, after that first
/// other size: an un-maximize (to the created size, or to a smaller one the
/// window manager chose because the created size does not fit the screen),
/// or a resize by the person. Windows and macOS maximize at creation, so
/// their first reading ends the wait.
///
/// What it does not see through: a window manager that refuses the maximize
/// (a tiling one, or none) leaves the window reported maximized until the
/// first reading after the grace, so a move made in the first two seconds
/// and never followed by another event is not recorded; and a maximize the
/// window manager makes in two resizes of different sizes, both before the
/// property says so, ends the wait at the second. A window that cannot be
/// maximized (not resizable: winit gives it equal minimum and maximum sizes;
/// a layer surface) never waits.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) enum MaximizeWait {
    #[default]
    Done,
    /// Created maximized at this (normal) inner size, at `since`; no reading
    /// at another size yet.
    Created { size: Size, since: Instant },
    /// A reading at another size came (the maximize, not yet reported): the
    /// window is taken as maximized while it stays at `at`.
    Resized {
        at: PhysicalSize<u32>,
        since: Instant,
    },
}

/// Whether two surface sizes are the same, to a rounding pixel.
fn same_size(a: PhysicalSize<u32>, b: PhysicalSize<u32>) -> bool {
    a.width.abs_diff(b.width) <= 1 && a.height.abs_diff(b.height) <= 1
}

impl MaximizeWait {
    /// For a window created with `attrs` at `now`; `layer_surface` for a
    /// Wayland layer surface, which has no maximize.
    pub(crate) fn after_create(
        attrs: &WindowAttributes,
        layer_surface: bool,
        now: Instant,
    ) -> Self {
        match attrs.surface_size {
            Some(size) if attrs.maximized && attrs.resizable && !layer_surface => {
                MaximizeWait::Created { size, since: now }
            }
            _ => MaximizeWait::Done,
        }
    }

    /// `r`, read at `now`, as the geometry rules should take it.
    pub(crate) fn filter(&mut self, mut r: WindowReading, now: Instant) -> WindowReading {
        let since = match *self {
            MaximizeWait::Done => return r,
            MaximizeWait::Created { since, .. } | MaximizeWait::Resized { since, .. } => since,
        };
        if r.maximized || now.saturating_duration_since(since) >= MAXIMIZE_GRACE {
            *self = MaximizeWait::Done;
            return r;
        }
        if r.minimized {
            // No geometry to judge by (Windows reports 0x0); the rules
            // record nothing of a minimized window anyway.
            return r;
        }
        match *self {
            MaximizeWait::Created { size, since } => {
                let created: PhysicalSize<u32> = size.to_physical(r.scale);
                if !same_size(created, r.surface) {
                    *self = MaximizeWait::Resized {
                        at: r.surface,
                        since,
                    };
                }
            }
            MaximizeWait::Resized { at, .. } => {
                // A scale change since is another size too.
                if !same_size(at, r.surface) {
                    *self = MaximizeWait::Done;
                    return r;
                }
            }
            MaximizeWait::Done => return r,
        }
        r.maximized = true;
        r
    }
}

/// The main window's geometry as an app reads it: logical pixels.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PublishedGeometry {
    /// Outer position; `None` on Wayland.
    pub position: Option<(f64, f64)>,
    /// Inner size.
    pub size: Option<(f64, f64)>,
    /// Whether the window is maximized.
    pub maximized: bool,
    /// The position and size it had when it was last neither maximized nor
    /// minimized: what an un-maximize returns to.
    pub normal_position: Option<(f64, f64)>,
    pub normal_size: Option<(f64, f64)>,
    /// The name of the monitor `position` is read in the scale of (the one
    /// the window is on), and the one `normal_position` was; `None` with no
    /// position or no name.
    pub monitor: Option<String>,
    pub normal_monitor: Option<String>,
}

/// The last geometry the shell saw for the main window, kept while it is
/// hidden. `None` until the main window first exists.
static PUBLISHED: Mutex<Option<PublishedGeometry>> = Mutex::new(None);

/// Publish the geometry of the live main window, on the monitor named
/// `monitor` (winit's `current_monitor`, whose scale the window has).
pub(crate) fn publish_window(r: &WindowReading, monitor: Option<String>) {
    publish(read_on(published().unwrap_or_default(), r, monitor));
}

/// [`read_into`], with the name of the monitor the window is on.
fn read_on(g: PublishedGeometry, r: &WindowReading, monitor: Option<String>) -> PublishedGeometry {
    let mut g = read_into(g, r);
    if !r.minimized {
        g.monitor = monitor.filter(|_| g.position.is_some());
        if r.is_normal() {
            g.normal_monitor = g.monitor.clone();
        }
    }
    g
}

/// `g` after reading the live window.
fn read_into(mut g: PublishedGeometry, r: &WindowReading) -> PublishedGeometry {
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
        g.maximized = r.maximized;
        if r.is_normal() {
            g.normal_position = g.position;
            g.normal_size = g.size;
        }
    }
    g
}

/// Publish the geometry a window is created with, before anything is read
/// off it: a window that first appears maximized has had no normal geometry
/// yet, and this is the one an un-maximize returns to.
/// `monitor` names the monitor the position was placed on.
pub(crate) fn publish_created(
    position: Option<(f64, f64)>,
    monitor: Option<String>,
    size: (f64, f64),
    maximized: bool,
) {
    publish(created_on(
        published().unwrap_or_default(),
        position,
        monitor,
        size,
        maximized,
    ));
}

fn created_on(
    mut g: PublishedGeometry,
    position: Option<(f64, f64)>,
    monitor: Option<String>,
    size: (f64, f64),
    maximized: bool,
) -> PublishedGeometry {
    if g.normal_size.is_none() {
        g.normal_position = position;
        g.normal_monitor = monitor.filter(|_| position.is_some());
        g.normal_size = Some(size);
    }
    g.maximized = maximized;
    g
}

#[cfg(test)]
fn created_into(
    g: PublishedGeometry,
    position: Option<(f64, f64)>,
    size: (f64, f64),
    maximized: bool,
) -> PublishedGeometry {
    created_on(g, position, None, size, maximized)
}

/// Publish a position or size the app set while the window was hidden: what
/// it will come back with. Logical pixels.
pub(crate) fn publish_hidden(position: Option<(f64, f64)>, size: Option<(f64, f64)>) {
    let mut g = published().unwrap_or_default();
    if position.is_some() {
        // Set by the app (or a layer surface's margins): on no monitor
        // rinch can name.
        g.position = position;
        g.normal_position = position;
        g.monitor = None;
        g.normal_monitor = None;
    }
    if size.is_some() {
        g.size = size;
        g.normal_size = size;
    }
    publish(g);
}

fn publish(g: PublishedGeometry) {
    *PUBLISHED.lock().unwrap_or_else(|p| p.into_inner()) = Some(g);
}

/// The published geometry.
pub(crate) fn published() -> Option<PublishedGeometry> {
    PUBLISHED.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// For a test that publishes: holds every other such test off and starts
/// from nothing published.
#[cfg(test)]
pub(crate) fn published_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    *PUBLISHED.lock().unwrap_or_else(|p| p.into_inner()) = None;
    guard
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::LogicalSize;

    fn attrs() -> WindowAttributes {
        WindowAttributes::default().with_surface_size(LogicalSize::new(560.0, 280.0))
    }

    fn req() -> Option<Position> {
        Some(Position::Logical(LogicalPosition::new(40.0, 50.0)))
    }

    /// Nothing recorded: the configured size, the requested position on a
    /// first appearance only.
    #[test]
    fn with_nothing_saved_a_first_show_takes_the_requested_position_and_a_later_one_none() {
        let none = SavedGeometry::default();
        let first = none.apply(attrs(), req(), false, true);
        assert_eq!(
            first.position,
            Some(Position::Logical(LogicalPosition::new(40.0, 50.0)))
        );
        assert_eq!(
            first.surface_size,
            Some(Size::Logical(LogicalSize::new(560.0, 280.0)))
        );
        assert_eq!(none.apply(attrs(), req(), false, false).position, None);
        assert_eq!(none.apply(attrs(), None, false, true).position, None);
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
            let a = saved.apply(attrs(), req(), false, first);
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
        let a = saved.apply(attrs(), req(), false, false);
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
        let a = saved.apply(attrs(), None, false, false);
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
    fn a_position_out_of_reach_is_dropped() {
        let monitors = [
            MonitorRect {
                position: PhysicalPosition::new(0, 0),
                size: PhysicalSize::new(1920, 1080),
                scale: 1.0,
                name: None,
            },
            MonitorRect {
                position: PhysicalPosition::new(1920, 0),
                size: PhysicalSize::new(2560, 1440),
                scale: 2.0,
                name: None,
            },
        ];
        let size = Some(Size::Logical(LogicalSize::new(400.0, 300.0)));
        let at = |x, y| Position::Physical(PhysicalPosition::new(x, y));
        assert!(reachable(at(300, 378), size, &monitors));
        assert!(reachable(at(3000, 1000), size, &monitors));
        // Overlapping the left monitor from off its left edge.
        assert!(reachable(at(-200, 100), size, &monitors));
        // A third monitor that is gone.
        assert!(!reachable(at(5000, 100), size, &monitors));
        assert!(!reachable(at(-32000, -32000), size, &monitors));
        assert!(reachable(at(5000, 100), size, &[]), "unknown: kept");
        // Only its bottom on the screen: the title bar is above every monitor.
        assert!(!reachable(at(300, -250), size, &monitors));
        // Its top just below the bottom edge.
        assert!(!reachable(at(300, 1075), size, &monitors));
        // A sliver of its right side on the left monitor.
        assert!(!reachable(at(-380, 100), size, &monitors));
        // A frame a few pixels above the top (Windows' outer frame) is fine.
        assert!(reachable(at(300, -8), size, &monitors));
    }

    /// `WindowProps::maximized` maximizes a first appearance only; a saved
    /// geometry says for a later one.
    #[test]
    fn a_requested_maximize_is_for_the_first_appearance() {
        let none = SavedGeometry::default();
        assert!(none.apply(attrs(), None, true, true).maximized);
        assert!(!none.apply(attrs(), None, true, false).maximized);
        assert!(!none.apply(attrs(), None, false, true).maximized);
    }

    /// What an app reads to bring the window back: the normal geometry kept
    /// through a maximize, and that it was maximized.
    #[test]
    fn the_published_normal_geometry_survives_a_maximize() {
        let g = created_into(
            PublishedGeometry::default(),
            Some((10.0, 20.0)),
            (800.0, 600.0),
            true,
        );
        assert_eq!(
            (g.normal_position, g.normal_size, g.maximized),
            (Some((10.0, 20.0)), Some((800.0, 600.0)), true)
        );
        let g = read_into(g, &reading(1.0, (120, 140), (600, 300)));
        let maxed = WindowReading {
            maximized: true,
            ..reading(1.0, (0, 0), (1280, 800))
        };
        let g = read_into(g, &maxed);
        assert_eq!(g.size, Some((1280.0, 800.0)));
        assert_eq!(
            (g.normal_position, g.normal_size, g.maximized),
            (Some((120.0, 140.0)), Some((600.0, 300.0)), true)
        );
    }

    #[test]
    fn a_maximized_window_comes_back_maximized() {
        let saved = SavedGeometry {
            maximized: true,
            ..Default::default()
        };
        assert!(saved.apply(attrs(), None, false, false).maximized);
    }

    fn monitor(x: i32, y: i32, w: u32, h: u32, scale: f64) -> MonitorRect {
        MonitorRect {
            position: PhysicalPosition::new(x, y),
            size: PhysicalSize::new(w, h),
            scale,
            name: None,
        }
    }

    fn named(name: &str, m: MonitorRect) -> MonitorRect {
        MonitorRect {
            name: Some(name.to_string()),
            ..m
        }
    }

    fn at(x: i32, y: i32) -> Position {
        Position::Physical(PhysicalPosition::new(x, y))
    }

    fn logical(w: f64, h: f64) -> Option<Size> {
        Some(Size::Logical(LogicalSize::new(w, h)))
    }

    /// The grip is capped at the window's height as at its width: a 400x4
    /// bar wholly on screen at the bottom edge is reachable, one half off it
    /// is not.
    #[test]
    fn a_window_shorter_than_the_grip_needs_its_whole_height_on_screen() {
        let one = [monitor(0, 0, 1920, 1080, 1.0)];
        let bar = logical(400.0, 4.0);
        assert!(reachable(at(300, 1076), bar, &one));
        assert!(!reachable(at(300, 1078), bar, &one));
    }

    /// 12 logical pixels of the top must be on screen: 13 rows are, 9 are
    /// not (a threshold of 6 would take them).
    #[test]
    fn twelve_rows_of_the_top_must_be_on_screen() {
        let one = [monitor(0, 0, 1920, 1080, 1.0)];
        let size = logical(400.0, 300.0);
        assert!(reachable(at(300, 1080 - 13), size, &one));
        assert!(!reachable(at(300, 1080 - 9), size, &one));
        // Above the top, 24 pixels down the window: 13 rows on screen are
        // enough, 9 are not.
        assert!(reachable(at(300, -11), size, &one));
        assert!(!reachable(at(300, -15), size, &one));
    }

    /// The grip is in the monitor's scale: at scale 2 it is 128 desktop
    /// pixels wide, so 100 on screen is not enough (64 unscaled would be),
    /// and 24 rows are not either (12 unscaled would be).
    #[test]
    fn the_grip_is_scaled_by_the_monitor() {
        let hidpi = [monitor(0, 0, 3840, 2160, 2.0)];
        let size = logical(400.0, 300.0);
        assert!(!reachable(at(3840 - 100, 100), size, &hidpi));
        assert!(reachable(at(3840 - 130, 100), size, &hidpi));
        assert!(!reachable(at(300, 2160 - 20), size, &hidpi));
        assert!(reachable(at(300, 2160 - 26), size, &hidpi));
    }

    /// The top only has to be on the desktop, not on one monitor: a 100 px
    /// window split 40 / 60 over two adjacent monitors is reachable, and so
    /// is one across two monitors stacked vertically. A gap between two
    /// monitors is not desktop.
    #[test]
    fn a_window_across_adjacent_monitors_is_reachable() {
        let side = [
            monitor(0, 0, 1920, 1080, 1.0),
            monitor(1920, 0, 1920, 1080, 1.0),
        ];
        let narrow = logical(100.0, 300.0);
        assert!(reachable(at(1880, 100), narrow, &side));
        let stacked = [
            monitor(0, 0, 1920, 1080, 1.0),
            monitor(0, 1080, 1920, 1080, 1.0),
        ];
        assert!(reachable(at(300, 1074), logical(400.0, 300.0), &stacked));
        let apart = [
            monitor(0, 0, 1920, 1080, 1.0),
            monitor(1960, 0, 1920, 1080, 1.0),
        ];
        assert!(!reachable(at(1880, 100), narrow, &apart));
        // 64 columns on one side of the gap are enough.
        assert!(reachable(at(1880, 100), logical(200.0, 300.0), &apart));
    }

    /// A logical position on a mixed-scale desktop names the desktop pixels
    /// of the monitor it is on: (5000, 100) next to a scale-2 primary is
    /// (5000, 100) on the scale-1 monitor, which winit is given as such; X11
    /// would convert it with the scale under the pointer (2) and put the
    /// window at (10000, 200), on no monitor. Off every monitor in every
    /// scale, it is left to the system.
    #[test]
    fn a_requested_position_is_resolved_in_the_scale_of_its_monitor() {
        let mixed = [
            monitor(0, 0, 3840, 2160, 2.0),
            monitor(3840, 0, 1920, 1080, 1.0),
        ];
        let size = Size::Logical(LogicalSize::new(400.0, 300.0));
        let expected = if cfg!(target_os = "macos") {
            Position::Logical(LogicalPosition::new(5000.0, 100.0))
        } else {
            at(5000, 100)
        };
        let place = |x, y, m: &[MonitorRect]| place_requested((x, y), size, None, m).map(|p| p.at);
        assert_eq!(place(5000.0, 100.0, &mixed), Some(expected));
        // On the primary: its scale.
        if !cfg!(target_os = "macos") {
            assert_eq!(place(500.0, 100.0, &mixed), Some(at(1000, 200)));
        }
        assert_eq!(place(9000.0, 100.0, &mixed), None);
        assert_eq!(
            place(9000.0, 100.0, &[]),
            Some(Position::Logical(LogicalPosition::new(9000.0, 100.0))),
            "unknown monitors: kept"
        );
    }

    fn maxed(size: (u32, u32)) -> WindowReading {
        WindowReading {
            maximized: true,
            ..reading(1.0, (0, 0), size)
        }
    }

    /// A wait for a window created maximized at 560x280 at `t0`, and the
    /// instant `ms` milliseconds after.
    fn waiting() -> (MaximizeWait, impl Fn(u64) -> Instant) {
        let t0 = Instant::now();
        let wait = MaximizeWait::after_create(&attrs().with_maximized(true), false, t0);
        (wait, move |ms| t0 + Duration::from_millis(ms))
    }

    /// Feeds `readings` (each with its milliseconds after the creation)
    /// through `wait` into the published geometry of a window created
    /// maximized at (40, 50), 560x280.
    fn feed(
        wait: &mut MaximizeWait,
        t: &impl Fn(u64) -> Instant,
        readings: &[(u64, WindowReading)],
    ) -> PublishedGeometry {
        let mut g = created_into(
            PublishedGeometry::default(),
            Some((40.0, 50.0)),
            (560.0, 280.0),
            true,
        );
        for (ms, r) in readings {
            g = read_into(g, &wait.filter(*r, t(*ms)));
        }
        g
    }

    /// Only a window that can be maximized waits: not one created
    /// un-maximized, not a fixed-size one (winit gives it equal minimum and
    /// maximum sizes, which window managers refuse to maximize), not a layer
    /// surface.
    #[test]
    fn only_a_window_that_can_be_maximized_waits() {
        let now = Instant::now();
        let max = attrs().with_maximized(true);
        assert!(matches!(
            MaximizeWait::after_create(&max, false, now),
            MaximizeWait::Created { .. }
        ));
        assert_eq!(
            MaximizeWait::after_create(&attrs(), false, now),
            MaximizeWait::Done
        );
        assert_eq!(
            MaximizeWait::after_create(&max.clone().with_resizable(false), false, now),
            MaximizeWait::Done
        );
        assert_eq!(
            MaximizeWait::after_create(&max, true, now),
            MaximizeWait::Done
        );
    }

    /// X11: the window manager maximizes after the creation, and the resize
    /// can come before `is_maximized` says so. Until a reading says
    /// maximized, the full-screen size is not taken as the normal geometry
    /// and the window stays maximized.
    #[test]
    fn a_window_created_maximized_waits_for_the_platform_to_say_so() {
        let (mut wait, t) = waiting();
        let mut saved = SavedGeometry::default();
        for (ms, r) in [
            (5, reading(1.0, (40, 50), (560, 280))),
            (30, reading(1.0, (0, 0), (1920, 1050))),
            (40, reading(1.0, (0, 0), (1920, 1050))),
        ] {
            saved.note(&wait.filter(r, t(ms)));
        }
        assert_eq!(saved.size, None, "no normal geometry recorded yet");
        let (mut wait, t) = waiting();
        let g = feed(
            &mut wait,
            &t,
            &[
                (5, reading(1.0, (40, 50), (560, 280))),
                (30, reading(1.0, (0, 0), (1920, 1050))),
            ],
        );
        assert!(g.maximized, "a reading before the property is not believed");
        assert_eq!(g.normal_size, Some((560.0, 280.0)));
        // The property arrives: from here on, readings are believed.
        let g = read_into(g, &wait.filter(maxed((1920, 1050)), t(60)));
        assert_eq!(wait, MaximizeWait::Done);
        let g = read_into(g, &wait.filter(reading(1.0, (40, 50), (560, 280)), t(70)));
        assert!(!g.maximized, "an un-maximize after it is believed");
    }

    /// Un-maximized back to the created size before any reading saw it
    /// maximized: that is an un-maximize, and the wait ends.
    #[test]
    fn back_at_the_created_size_after_another_ends_the_wait() {
        let (mut wait, t) = waiting();
        assert!(
            wait.filter(reading(1.0, (40, 50), (560, 280)), t(5))
                .maximized
        );
        assert!(
            wait.filter(reading(1.0, (0, 0), (1920, 1050)), t(30))
                .maximized
        );
        assert!(
            wait.filter(reading(1.0, (0, 0), (1920, 1050)), t(40))
                .maximized
        );
        let back = wait.filter(reading(1.0, (40, 50), (560, 280)), t(900));
        assert!(!back.maximized);
        assert_eq!(wait, MaximizeWait::Done);
        // In logical pixels: the created size at scale 2.
        let t0 = Instant::now();
        let mut hidpi = MaximizeWait::after_create(&attrs().with_maximized(true), false, t0);
        assert!(
            hidpi
                .filter(reading(2.0, (0, 0), (3840, 2100)), t0)
                .maximized
        );
        assert!(
            !hidpi
                .filter(reading(2.0, (80, 100), (1120, 560)), t0)
                .maximized
        );
    }

    /// #1884 second review: the window manager refuses the maximize (a
    /// tiling one, or none at all). Every reading is at the created size,
    /// and none says maximized: the move to (700, 400) is recorded once the
    /// grace is over, and the window is not maximized.
    #[test]
    fn a_maximize_the_window_manager_refuses_ends_at_the_grace() {
        let (mut wait, t) = waiting();
        let g = feed(
            &mut wait,
            &t,
            &[
                (5, reading(1.0, (40, 50), (560, 280))),
                (1500, reading(1.0, (400, 300), (560, 280))),
            ],
        );
        assert!(g.maximized, "inside the grace, still taken as maximized");
        let g = read_into(
            g,
            &wait.filter(reading(1.0, (700, 400), (560, 280)), t(2100)),
        );
        assert_eq!(
            (g.maximized, g.normal_position, g.normal_size),
            (false, Some((700.0, 400.0)), Some((560.0, 280.0)))
        );
        assert_eq!(wait, MaximizeWait::Done);
    }

    /// A tiling window manager puts the window in a tile instead: a resize
    /// to another size that never says maximized. After the grace, the tile
    /// is the normal geometry.
    #[test]
    fn a_tiling_window_manager_ends_the_wait_at_the_grace() {
        let (mut wait, t) = waiting();
        let g = feed(
            &mut wait,
            &t,
            &[
                (20, reading(1.0, (0, 0), (960, 1050))),
                (2500, reading(1.0, (0, 0), (960, 1050))),
            ],
        );
        assert_eq!((g.maximized, g.normal_size), (false, Some((960.0, 1050.0))));
    }

    /// #1884 second review: the created size does not fit the screen, the
    /// window manager maximizes (not yet reported), then the person
    /// un-maximizes and it restores to a smaller size than the created one;
    /// then they resize it. The un-maximize ends the wait, inside the grace.
    #[test]
    fn an_un_maximize_to_another_size_ends_the_wait() {
        let t0 = Instant::now();
        let t = |ms| t0 + Duration::from_millis(ms);
        let big = WindowAttributes::default()
            .with_surface_size(LogicalSize::new(1920.0, 1200.0))
            .with_maximized(true);
        let mut wait = MaximizeWait::after_create(&big, false, t0);
        let mut g = created_into(PublishedGeometry::default(), None, (1920.0, 1200.0), true);
        for (ms, r) in [
            (20, reading(1.0, (0, 0), (1920, 1050))),
            (800, reading(1.0, (50, 40), (1800, 1000))),
        ] {
            g = read_into(g, &wait.filter(r, t(ms)));
        }
        assert_eq!(wait, MaximizeWait::Done);
        assert_eq!(
            (g.maximized, g.normal_size),
            (false, Some((1800.0, 1000.0)))
        );
        g = read_into(
            g,
            &wait.filter(reading(1.0, (300, 200), (1200, 800)), t(1200)),
        );
        assert_eq!((g.maximized, g.normal_size), (false, Some((1200.0, 800.0))));
    }

    /// #1884 third review: monitor A (scale 1) at x 0..1920, B (scale 2) at
    /// 1920..5760. A window on B at desktop (2100, 100) publishes (1050, 50),
    /// which is also on A: without a name it went to A, listed first. The
    /// name published with it puts it back on B.
    #[test]
    fn the_monitor_name_settles_a_mixed_scale_position() {
        let a = named("A", monitor(0, 0, 1920, 1080, 1.0));
        let b = named("B", monitor(1920, 0, 3840, 2160, 2.0));
        let mons = [a.clone(), b.clone()];
        let g = read_on(
            PublishedGeometry::default(),
            &reading(2.0, (2100, 100), (1600, 1200)),
            Some("B".to_string()),
        );
        assert_eq!(g.normal_monitor.as_deref(), Some("B"));
        let (x, y) = g.normal_position.unwrap();
        let size = Size::Logical(LogicalSize::new(800.0, 600.0));
        let placed = place_requested((x, y), size, g.normal_monitor.as_deref(), &mons).unwrap();
        assert_eq!(placed.monitor.as_deref(), Some("B"));
        if !cfg!(target_os = "macos") {
            assert_eq!(placed.at, at(2100, 100));
            // No name, or a name no monitor has now: the first that fits.
            for hint in [None, Some("C")] {
                let p = place_requested((x, y), size, hint, &mons).unwrap();
                assert_eq!((p.at, p.monitor.as_deref()), (at(1050, 50), Some("A")));
            }
            // A named monitor on which it is out of reach: the others.
            let p = place_requested((2500.0, 50.0), size, Some("A"), &mons).unwrap();
            assert_eq!((p.at, p.monitor.as_deref()), (at(5000, 100), Some("B")));
        }
    }

    /// Two monitors of one name (two of the same model, where the platform
    /// names them by model): the first of them on which the window is
    /// reachable.
    #[test]
    fn of_two_monitors_of_one_name_the_first_that_fits_wins() {
        let left = named("Dell", monitor(0, 0, 1920, 1080, 1.0));
        let other = named("Lenovo", monitor(1920, 0, 1920, 1080, 1.0));
        let right = named("Dell", monitor(3840, 0, 3840, 2160, 2.0));
        let mons = [left, other, right];
        let size = Size::Logical(LogicalSize::new(400.0, 300.0));
        if !cfg!(target_os = "macos") {
            // (2000, 100): on "Lenovo" at scale 1 and on the right "Dell" at
            // scale 2 (4000, 200); the left "Dell" has no such point. The
            // name picks the right "Dell".
            let p = place_requested((2000.0, 100.0), size, Some("Dell"), &mons).unwrap();
            assert_eq!(p.at, at(4000, 200));
            // (100, 100) is on the left "Dell" first.
            let p = place_requested((100.0, 100.0), size, Some("Dell"), &mons).unwrap();
            assert_eq!(p.at, at(100, 100));
        }
    }

    /// The name goes with the position: kept through a maximize as the
    /// normal monitor, dropped with no position (Wayland) and by a position
    /// set by the app.
    #[test]
    fn the_published_monitor_follows_the_normal_position() {
        let g = read_on(
            PublishedGeometry::default(),
            &reading(1.0, (120, 140), (600, 300)),
            Some("A".to_string()),
        );
        let g = read_on(g, &maxed((1920, 1050)), Some("B".to_string()));
        assert_eq!(
            (g.monitor.as_deref(), g.normal_monitor.as_deref()),
            (Some("B"), Some("A"))
        );
        let wayland = WindowReading {
            position: None,
            ..reading(1.0, (0, 0), (600, 300))
        };
        let g2 = read_on(
            PublishedGeometry::default(),
            &wayland,
            Some("eDP-1".to_string()),
        );
        assert_eq!(g2.normal_monitor, None);
        let g3 = created_on(
            PublishedGeometry::default(),
            Some((10.0, 20.0)),
            Some("A".to_string()),
            (800.0, 600.0),
            true,
        );
        assert_eq!(g3.normal_monitor.as_deref(), Some("A"));
    }
}
