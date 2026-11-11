//! Control of the app's window from the code running in it: minimize,
//! maximize, hide and show, geometry, click-through, the Wayland layer-shell
//! role, and starting a move or resize with the pointer.
//!
//! Each function acts on the app's one window. They queue their request for
//! the event loop, so they can be called from any thread; the getters read
//! what the shell last published.

use crate::shell::rinch_runtime::{RinchNativeEvent, send_native_event};
use rinch_core::element::WindowSurfaceKind;

// =============================================================================
// Window Control Functions (for the current window)
// =============================================================================

/// Minimize the current window.
///
/// Call this from an event handler (e.g., onclick) to minimize the window
/// that contains the element.
///
/// # Example
///
/// ```ignore
/// button { onclick: || minimize_current_window(), "Minimize" }
/// ```
pub fn minimize_current_window() {
    send_native_event(RinchNativeEvent::MinimizeWindow);
}

/// Toggle maximize state of the current window.
///
/// If the window is currently maximized, it will be restored to its previous size.
/// If not maximized, it will be maximized.
///
/// # Example
///
/// ```ignore
/// button { onclick: || toggle_maximize_current_window(), "Maximize" }
/// ```
pub fn toggle_maximize_current_window() {
    send_native_event(RinchNativeEvent::ToggleMaximizeWindow);
}

/// Hide the current window (minimize to tray pattern).
///
/// The window becomes invisible but the application continues running.
/// Use `show_current_window()` to make it visible again.
///
/// # Example
///
/// ```ignore
/// button { onclick: || hide_current_window(), "Hide to Tray" }
/// ```
pub fn hide_current_window() {
    send_native_event(RinchNativeEvent::HideWindow);
}

/// Show the current window (restore from tray).
///
/// Makes a previously hidden window visible again and brings it to focus —
/// except a [`WindowProps::non_activating`](rinch_core::element::WindowProps::non_activating)
/// window, which is shown without taking focus or activation. It comes back
/// where it was when it was hidden and at the size it had (issue #1546; the
/// size only on Wayland, where the compositor places windows), or where
/// [`set_current_window_position`] / [`set_current_window_size`] put it
/// meanwhile.
///
/// # Example
///
/// ```ignore
/// // In a tray menu callback:
/// MenuItem::new("Show").on_click(|| show_current_window())
/// ```
pub fn show_current_window() {
    send_native_event(RinchNativeEvent::ShowWindow);
}

/// Let pointer events pass through the current window to whatever is under
/// it (`true`), or make it take them again (`false`). Starts at
/// [`WindowProps::click_through`](rinch_core::element::WindowProps::click_through)
/// and is kept across [`hide_current_window`] / [`show_current_window`]
/// (issue #1519).
///
/// A click-through window receives no pointer events at all, so nothing in it
/// can turn click-through off again: toggle it from a global hotkey, a tray
/// menu, a timer or another thread (it is `Send`-safe, like the other window
/// controls).
///
/// # Example
///
/// ```ignore
/// MenuItem::new("Make overlay interactive")
///     .on_click(|| set_click_through_current_window(false))
/// ```
pub fn set_click_through_current_window(click_through: bool) {
    send_native_event(RinchNativeEvent::SetClickThrough(click_through));
}

/// The current window's outer (frame) top-left, in logical pixels on the
/// desktop — the position [`WindowProps::x`](rinch_core::element::WindowProps::x)
/// and [`WindowProps::y`](rinch_core::element::WindowProps::y) set (issue #1546).
///
/// Kept while the window is hidden ([`hide_current_window`]): it is where
/// [`show_current_window`] puts it back. So an app can read it at exit and
/// persist it. `None` before the window first exists, and on Wayland, where
/// a client cannot learn its window's position — except a layer-shell window
/// (issue #1523), whose position is its top-left on its output, once it has
/// one (from `WindowProps::x`/`y`, [`set_current_window_position`] or a
/// move); before that the compositor centres it and this is `None`.
/// Callable from any thread.
pub fn current_window_position() -> Option<(f64, f64)> {
    crate::shell::window_geometry::published().and_then(|g| g.position)
}

/// The current window's inner (content) size in logical pixels — the size
/// [`WindowProps::width`](rinch_core::element::WindowProps::width) and
/// [`WindowProps::height`](rinch_core::element::WindowProps::height) set
/// (issue #1546). Kept while the window is hidden, like
/// [`current_window_position`]. `None` before the window first exists.
pub fn current_window_size() -> Option<(f64, f64)> {
    crate::shell::window_geometry::published().and_then(|g| g.size)
}

/// How to bring the main window back as it is now, for an app that keeps
/// it across runs: the position and size it has when it is neither maximized
/// nor minimized (what an un-maximize returns to), and whether it is
/// maximized. Give them back as `WindowProps::x`/`y`, `width`/`height` and
/// `maximized`; a position out of reach on the monitors connected then (one
/// since unplugged) is left to the system.
///
/// This is the window's *normal* geometry, not its current one, and its
/// position is `None` where the platform gives none (Wayland).
#[derive(Debug, Clone, PartialEq)]
pub struct WindowRestore {
    /// Outer top-left on the desktop, logical pixels; `None` on Wayland.
    pub position: Option<(f64, f64)>,
    /// Inner (content) size, logical pixels.
    pub size: (f64, f64),
    /// Whether the window is maximized.
    pub maximized: bool,
    /// The platform's name for the monitor `position` is on; give it back as
    /// `WindowProps::monitor`. On a desktop whose monitors have different
    /// scales a logical position can name a point on two of them, and this
    /// says which. `None` with no position, or where the platform names no
    /// monitor.
    pub monitor: Option<String>,
}

/// What [`WindowRestore`] says of the current window, to read at exit and
/// persist. `None` before the window first exists. Callable from any thread.
pub fn current_window_restore() -> Option<WindowRestore> {
    let g = crate::shell::window_geometry::published()?;
    Some(WindowRestore {
        position: g.normal_position,
        size: g.normal_size.or(g.size)?,
        maximized: g.maximized,
        monitor: g.normal_monitor,
    })
}

/// Move the current window's outer top-left to `(x, y)`, logical pixels on
/// the desktop (issue #1546). While the window is hidden, this is where
/// [`show_current_window`] puts it. Applied on the event loop's next turn, so
/// [`current_window_position`] answers the new position after that. Ignored
/// on Wayland, where a client cannot position its window — except a
/// layer-shell window (issue #1523), which is anchored to its output's
/// top-left corner with `(x, y)` as its margins: logical pixels relative to
/// its output ([`set_current_window_output`]), not the desktop.
///
/// **On a layer surface, one turn is one commit** (issue #2464). What the
/// current window is asked for in one turn of the event loop through this
/// function, [`set_current_window_size`] and [`set_current_window_layer`] reaches the compositor
/// in **one** commit, in whatever order the calls were made: a panel that
/// takes a new height, anchors and exclusive zone at once is arranged once,
/// never at its new height with its old zone. A turn's requests are the ones
/// made on the UI thread from one event handler, effect, timer or
/// [`run_on_main_thread`](crate::run_on_main_thread) closure; calls made from
/// another thread can fall into different turns.
pub fn set_current_window_position(x: f64, y: f64) {
    send_native_event(RinchNativeEvent::SetWindowPosition(x, y));
}

/// Resize the current window's inner (content) area to `width` x `height`
/// logical pixels (issue #1546). While the window is hidden, this is the size
/// [`show_current_window`] gives it. Applied on the event loop's next turn,
/// like [`set_current_window_position`]; the window manager may refuse or
/// adjust it (a tiling window manager, a size constraint).
///
/// **On a layer surface, one turn is one commit** (issue #2464). What the
/// current window is asked for in one turn of the event loop through this
/// function, [`set_current_window_position`] and [`set_current_window_layer`] reaches the compositor
/// in **one** commit, in whatever order the calls were made: a panel that
/// takes a new height, anchors and exclusive zone at once is arranged once,
/// never at its new height with its old zone. A turn's requests are the ones
/// made on the UI thread from one event handler, effect, timer or
/// [`run_on_main_thread`](crate::run_on_main_thread) closure; calls made from
/// another thread can fall into different turns.
pub fn set_current_window_size(width: f64, height: f64) {
    send_native_event(RinchNativeEvent::SetWindowSize(width, height));
}

/// What kind of surface the current window got (issue #1523): on native
/// Wayland a [`WindowProps::non_activating`](rinch_core::element::WindowProps::non_activating)
/// window (or one with `wayland_layer_shell: Some(true)`) is a
/// [`WindowSurfaceKind::LayerShell`] surface where the compositor supports
/// `wlr-layer-shell`, and a [`WindowSurfaceKind::Toplevel`] where it does not
/// (GNOME/Mutter). Every other window is `Toplevel`. Kept while the window is
/// hidden; `None` before the window first exists. Callable from any thread.
pub fn current_window_surface_kind() -> Option<WindowSurfaceKind> {
    crate::shell::layer_shell::published_surface_kind()
}

/// Show the current window on the output (monitor) with this connector name
/// (`"DP-1"`, `"eDP-1"`), or let the compositor choose (`None`) — for a
/// Wayland layer-shell window (issue #1523), whose position
/// ([`set_current_window_position`]) is relative to its output. The window is
/// made again on that output at the same position and size. Overrides
/// [`WindowProps::output`](rinch_core::element::WindowProps::output); no
/// effect on any other kind of window. Monitor names are what
/// `wl_output.name` reports, as `wlr-randr` / `kscreen-doctor -o` list them.
pub fn set_current_window_output(output: Option<&str>) {
    send_native_event(RinchNativeEvent::SetWindowOutput(
        output.map(str::to_string),
    ));
}

/// Change the current window's `wlr-layer-shell` configuration while it is
/// open (issue #2413): what [`WindowProps::wayland_layer`](rinch_core::element::WindowProps::wayland_layer)
/// set, replaced whole. Callable from any thread, and from an effect that
/// follows a signal (a panel's exclusive zone following its height).
///
/// - Layer, anchors, margins, exclusive zone, keyboard interactivity and
///   the compositor-sized axes change on the live surface together, in one
///   commit: the compositor sees the old configuration, then the new one,
///   and nothing in between (a bar moved to another edge with a new margin
///   and zone is arranged once).
/// - A different [`output`](rinch_core::element::LayerSurface::output) makes
///   the surface again on that output, at the same size, over the same
///   document: the protocol fixes a layer surface's output when it is made.
///   It replaces an output [`set_current_window_output`] chose.
/// - A window that was not a layer surface (an overlay shorthand counts as
///   one) becomes one, also by being made again.
///
/// While the window is hidden it is what [`show_current_window`] makes. It
/// is kept, and does nothing, where the window is not a layer surface: on
/// X11, Windows, macOS, and a compositor with no layer shell (GNOME).
///
/// **On a layer surface, one turn is one commit** (issue #2464). What the
/// current window is asked for in one turn of the event loop through this
/// function, [`set_current_window_size`] and [`set_current_window_position`] reaches the compositor
/// in **one** commit, in whatever order the calls were made: a panel that
/// takes a new height, anchors and exclusive zone at once is arranged once,
/// never at its new height with its old zone. A turn's requests are the ones
/// made on the UI thread from one event handler, effect, timer or
/// [`run_on_main_thread`](crate::run_on_main_thread) closure; calls made from
/// another thread can fall into different turns.
pub fn set_current_window_layer(surface: rinch_core::element::LayerSurface) {
    send_native_event(RinchNativeEvent::SetWindowLayer(Box::new(surface)));
}

/// Start moving the current window with the pointer, from a handler of the
/// press that should move it (a custom title bar's `onmousedown`) — what a
/// `data-drag-window` region does (issue #1523). An ordinary window asks the
/// window manager for an interactive move. A Wayland layer-shell window has
/// none, so rinch follows the pointer until the button is released and moves
/// the window by its margins; [`current_window_position`] then answers where
/// it is. Ignored when no button is held (a timer, a press already over).
pub fn start_window_drag() {
    send_native_event(RinchNativeEvent::StartWindowDrag);
}

/// Start resizing the current window from `edge` with the pointer, from a
/// handler of the press on that edge — what `WindowProps::resize_inset` does
/// for a borderless window (issue #1523). A Wayland layer-shell window is
/// resized by rinch, following the pointer until the button is released (the
/// opposite edge stays put; 16 logical pixels at the least). Ignored when no
/// button is held.
pub fn start_window_resize(edge: rinch_platform::ResizeDirection) {
    send_native_event(RinchNativeEvent::StartWindowResize(edge));
}

/// Close the current window.
///
/// Call this from an event handler (e.g., onclick) to close the window
/// that contains the element.
///
/// # Example
///
/// ```ignore
/// button { onclick: || close_current_window(), "Close" }
/// ```
pub fn close_current_window() {
    send_native_event(RinchNativeEvent::CloseWindowControl);
}
