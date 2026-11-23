//! Control of the app's window from the code running in it: minimize,
//! maximize, hide and show, geometry, click-through, the Wayland layer-shell
//! role, and starting a move or resize with the pointer.
//!
//! Each function acts on the app's one window. They queue their request for
//! the event loop, so they can be called from any thread; the getters read
//! what the shell last published.

use crate::shell::rinch_runtime::{RinchNativeEvent, send_native_event};
use rinch_core::element::WindowSurfaceKind;

pub use crate::activation::{
    ActivationToken, ActivationTokenError, ActivationTokenResult, request_activation_token,
    take_launch_activation_token,
};

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

/// Bring the current window to the front and ask for keyboard focus: shown if
/// it was hidden ([`hide_current_window`]), restored if it was minimized. For
/// an app asked to show something from outside itself, such as a second launch
/// handing over what it was started with (a link from a browser).
/// [`WindowHandle::focus`](crate::WindowHandle::focus) does the same for any
/// of the app's windows.
///
/// The operating system has the last word: a system that prevents focus
/// stealing may only mark the window as wanting attention. On Wayland an app
/// cannot raise its own window at all without an xdg-activation token from
/// the input that asked; this has none to give, so a shown window is only
/// marked as wanting attention (the compositor may still raise it). When the
/// request comes with a token (a second launch's `XDG_ACTIVATION_TOKEN`),
/// use [`activate_current_window_with_token`] instead. A
/// [`WindowProps::non_activating`](rinch_core::element::WindowProps::non_activating)
/// window is restored but never takes focus.
///
/// # Example
///
/// ```ignore
/// on_link_from_outside(|link| { focus_current_window(); open(link) })
/// ```
pub fn focus_current_window() {
    send_native_event(RinchNativeEvent::FocusWindow(None));
}

/// [`focus_current_window`] with an xdg-activation `token`, which lets a
/// Wayland compositor raise the window and give it the keyboard (#2817).
///
/// For a single-instance app: its second launch takes the token it was
/// started with ([`take_launch_activation_token`]) and hands it to the
/// running instance over the app's own IPC, which calls this. On Wayland the
/// shown window is activated with the token
/// (`xdg_activation_v1.activate`), and a hidden one is shown with the token
/// in its new window's attributes (winit's `with_activation_token`). The
/// current window is the window of the document whose code calls this;
/// from anywhere else (an IPC thread, `main`) it is the main window, so a
/// multi-window or windowless app uses
/// [`WindowHandle::activate_with_token`](crate::WindowHandle::activate_with_token).
/// The compositor decides what the token is worth: a
/// token tied to a recent user action (a click on a link, a launcher's
/// entry) raises the window on KWin and on sway set to
/// `focus_on_window_activation focus` (sway's default marks it urgent); a
/// stale or unknown token is ignored, and nothing says so.
///
/// Elsewhere (X11, Windows, macOS) the token means nothing to rinch and this
/// is [`focus_current_window`]. Everywhere it is limited as that is: a
/// minimized window is restored, a
/// [`non_activating`](rinch_core::element::WindowProps::non_activating) one
/// is only restored, and a Wayland layer-shell window is not activated. On
/// a compositor without `xdg_activation_v1` the window is marked as wanting
/// attention, as [`focus_current_window`] marks it.
///
/// # Example
///
/// ```ignore
/// on_second_launch(|link, token: Option<String>| {
///     match token {
///         Some(token) => activate_current_window_with_token(&token),
///         None => focus_current_window(),
///     }
///     open(link)
/// })
/// ```
pub fn activate_current_window_with_token(token: &str) {
    send_native_event(RinchNativeEvent::FocusWindow(Some(token.to_string())));
}

/// Prepare the current window, while it is hidden, for its next show
/// (issue #2633), without showing it.
///
/// A window's first show pays for everything its first frame needs: on the
/// GPU the renderer, its device and its pipelines (the shader compiles, on a
/// cold shader cache), and on both renderers the glyphs its text needs
/// rasterized and its images decoded. A hidden window presents nothing, so
/// none of that happens before the show, and a launcher or a menu kept
/// hidden until summoned is slowest the first time it is summoned. This
/// does that work now, at a time the app chooses — at start-up, or after it
/// has changed the hidden window's content: what is pending is styled and
/// laid out at the size the window will be shown at, and a frame is painted
/// there and not presented. The window is not mapped and nothing reaches
/// the screen. The next [`show_current_window`] then costs what a show after
/// a hide does.
///
/// The cost is memory: a prepared hidden window keeps what a shown one has,
/// on the GPU its renderer and UI texture (a hide keeps them too, #2545),
/// in software its pixmap and glyph atlas.
///
/// Off by default, as a browser does not paint what is not shown. Content
/// changed after the call is styled, laid out and painted by the show as
/// usual; call it again to prepare that too. It does nothing while the
/// window is shown, in an embedded context (`RinchContext`, which has no
/// window of its own), on Android, and on the web (no such function there:
/// the browser paints). Callable from any thread.
///
/// # Example
///
/// ```ignore
/// // A launcher started with `WindowProps { visible: false, .. }`:
/// #[component]
/// fn launcher() -> NodeHandle {
///     prepare_current_window(); // runs once the root is mounted
///     rsx! { /* ... */ }
/// }
/// ```
pub fn prepare_current_window() {
    send_prepare(None);
}

/// [`prepare_current_window`] at the display scale `scale` (physical pixels
/// per logical pixel: `1.0`, `2.0`, `1.5`), for an app that knows the scale
/// the window will be shown at better than rinch's guess. A scale that is
/// not finite and positive is ignored, and the guess used.
pub fn prepare_current_window_at_scale(scale: f64) {
    send_prepare(Some(scale));
}

/// The window the calling code is in ([`crate::WindowHandle::current`]: a
/// window's content during its own mount too, before its document is the
/// dispatching one), else whatever `send_native_event` names.
fn send_prepare(scale: Option<f64>) {
    match crate::WindowHandle::current() {
        Some(window) => window.prepare_with(scale),
        None => send_native_event(RinchNativeEvent::PrepareWindow(scale)),
    }
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

/// Change where the current window takes pointer input (issue #2741):
/// [`WindowProps::input_region`](rinch_core::element::WindowProps::input_region)
/// at run time, kept across [`hide_current_window`] / [`show_current_window`].
/// While click-through ([`set_click_through_current_window`]) is on, it wins
/// and the window takes no input anywhere. See
/// [`InputRegion`](rinch_core::element::InputRegion).
pub fn set_input_region_current_window(region: rinch_core::element::InputRegion) {
    send_native_event(RinchNativeEvent::SetInputRegion(region));
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
    restore_of(crate::shell::window_geometry::published()?)
}

/// What a window's published geometry says to restore it to.
pub(crate) fn restore_of(
    g: crate::shell::window_geometry::PublishedGeometry,
) -> Option<WindowRestore> {
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
