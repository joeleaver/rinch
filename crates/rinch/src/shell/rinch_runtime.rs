//! Rinch-native runtime using rinch-dom for rendering.
//!
//! This module wires together the platform-agnostic [`RinchApp`] with the
//! desktop backend ([`WinitWindow`] + [`WgpuRenderer`]) and the winit event
//! loop. All application logic lives in [`crate::app::RinchApp`]; this file
//! is purely the desktop glue layer.
//!
//! # Usage
//!
//! ```ignore
//! use rinch::prelude::*;
//!
//! fn app(__scope: &mut RenderScope) -> NodeHandle {
//!     rsx! { div { "Hello from rinch-dom!" } }
//! }
//!
//! fn main() {
//!     rinch::run_rinch("My App", 800, 600, app);
//! }
//! ```

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, Position, Size};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::window::{WindowAttributes, WindowId};

use rinch_core::clear_context;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::events;
#[cfg(feature = "gpu")]
use rinch_platform::PlatformRenderer;
use rinch_platform::{
    AppAction, ImeEvent, KeyCode, KeyRepeat, Modifiers, MouseButton as PlatformMouseButton,
    PlatformEvent, PlatformWindow, UserEvent, to_logical, to_logical_point,
};

use crate::app::RinchApp;
use crate::render_surface::{SurfacePointer, SurfacePointerKind};

#[cfg(feature = "gpu")]
use super::desktop::WgpuRenderer;
use super::layer_shell::{Gesture, GestureKind, LayerRequest, LayerState, WindowSurfaceKind};
use super::overlay_window::{self, Backend, WindowSteps};
use super::window::WinitWindow;
use super::window_geometry::{self, SavedGeometry};

/// Ask again in a second whether an output has appeared for a layer surface
/// the compositor closed with none left (#1523).
fn arm_layer_output_retry() {
    rinch_core::set_timeout(1000, || {
        send_native_event(RinchNativeEvent::RetryLayerOutput)
    });
}

#[cfg(feature = "debug")]
use {super::screenshot, base64::Engine, rinch_debug::DebugResult};

// ── Thread-local proxy ───────────────────────────────────────────────────────

// Thread-local proxy for the native event loop, used by window control functions.
thread_local! {
    pub(crate) static NATIVE_PROXY: RefCell<Option<EventLoopProxy>> = const { RefCell::new(None) };
}

// ── Global proxy + native event queue ────────────────────────────────────────

// Global (Send+Sync) proxy for waking the event loop from any thread.
pub(crate) static GLOBAL_PROXY: OnceLock<EventLoopProxy> = OnceLock::new();

/// The native events waiting for the next `proxy_wake_up`, with at most one
/// [`RinchNativeEvent::ReRender`] among them.
///
/// `ReRender` means "signals changed since the last resolve", and one resolve
/// answers any number of changes — so a second one queued behind the first is
/// pure cost: a mutex, an event-loop wake and a `resolve_and_repaint` preamble
/// that finds nothing to do. It used to be queued once per signal flush, which
/// a handler writing *n* signals outside a batch turned into *n* events.
///
/// `rerender_pending` is cleared by [`Self::drain`] **under the same lock** that
/// empties the queue: a `ReRender` sent after the drain (from an effect the
/// drained events run, or from another thread) finds the flag down and queues
/// afresh, and one sent before it is already in the drained batch. Clearing the
/// flag outside the lock would leave a window in which a send saw it up,
/// queued nothing, and then had its wake-up discarded.
pub(crate) struct NativeEventQueue {
    events: VecDeque<RinchNativeEvent>,
    rerender_pending: bool,
}

impl NativeEventQueue {
    const fn new() -> Self {
        Self {
            events: VecDeque::new(),
            rerender_pending: false,
        }
    }

    /// Queue `event`. Returns `false` for a `ReRender` that coalesced into one
    /// already queued — nothing was added and no wake is owed.
    pub(crate) fn push(&mut self, event: RinchNativeEvent) -> bool {
        if matches!(event, RinchNativeEvent::ReRender) {
            if self.rerender_pending {
                return false;
            }
            self.rerender_pending = true;
        }
        self.events.push_back(event);
        true
    }

    /// Take every queued event, and let the next `ReRender` queue again.
    pub(crate) fn drain(&mut self) -> Vec<RinchNativeEvent> {
        self.rerender_pending = false;
        self.events.drain(..).collect()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

// Queue of native events to process on the next proxy_wake_up.
// winit 0.31 removes send_event(T); we queue events and call proxy.wake_up().
static NATIVE_EVENT_QUEUE: Mutex<NativeEventQueue> = Mutex::new(NativeEventQueue::new());

/// Queue a native event and wake the event loop. A `ReRender` while one is
/// already queued is dropped — see [`NativeEventQueue`].
pub(crate) fn send_native_event(event: RinchNativeEvent) {
    let is_rerender = matches!(event, RinchNativeEvent::ReRender);
    if !NATIVE_EVENT_QUEUE.lock().unwrap().push(event) {
        return;
    }
    if is_rerender {
        crate::app::RERENDER_EVENTS_QUEUED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    if let Some(proxy) = GLOBAL_PROXY.get() {
        proxy.wake_up();
    }
}

/// The two halves of a `proxy_wake_up`: run the queued main-thread work
/// (`run_queued`: the callbacks, then the deferred-work inboxes they woke),
/// then the queued native events (`drain_native`, which re-arms the
/// `ReRender` coalescing in [`NativeEventQueue::drain`]).
///
/// A callback queued between the two halves is owed a wake of its own, which
/// the end of this function pays (issue #988). A free function over the steps
/// so a test can put a second thread's `run_on_main_thread` exactly between
/// them.
fn drain_wake_queues<S>(
    state: &mut S,
    run_queued: impl FnOnce(&mut S),
    drain_native: impl FnOnce(&mut S),
) {
    run_queued(state);
    drain_native(state);
    // A callback queued after `run_queued` emptied the main-thread queue and
    // before `drain_native` re-armed the coalescing saw "was empty" and sent
    // a `ReRender` — which folded into the one this wake just consumed and
    // asked for no wake of its own (issue #988). It is still queued; owe it a
    // wake now. The flag is down, so this queues and wakes. A callback queued
    // after the re-arm woke the loop itself, and one queued before
    // `run_queued` ran in it.
    if rinch_core::main_callbacks_pending() {
        send_native_event(RinchNativeEvent::ReRender);
    }
}

/// Queue a closure to run on the main (UI) thread.
///
/// The closure will execute during the next event-loop wake, before the
/// re-render pass. This is the safe way to update [`Signal`]s from a
/// background thread (e.g. after an HTTP request completes on tokio).
///
/// The queue itself lives in `rinch-core` so every host shares one
/// ([`rinch_core::queue_main_callback`], issue #172); what this shell adds is
/// the wake — without it a closure queued while the loop is idle would sit
/// there until something else happened to wake it.
///
/// It **always queues**, even when called on the main thread, and the closure
/// runs under the document current where it was queued (issue #963) — the
/// handler's document from inside a dispatch, none from a worker thread.
/// [`rinch_core::run_on_main_thread`] differs: called on the main thread it runs
/// `f` inline, under whatever document is current right then.
///
/// # Example
///
/// ```ignore
/// let loading = Signal::new(false);
/// std::thread::spawn(move || {
///     let result = do_work();
///     rinch::run_on_main_thread(move || {
///         loading.set(false);
///     });
/// });
/// ```
pub fn run_on_main_thread(f: impl FnOnce() + Send + 'static) {
    // Always ask: the `ReRender` coalesces in `NativeEventQueue`, which
    // re-arms on every native drain. Keying the wake on "the main queue was
    // empty" stranded the queue whenever a paint's native drain ate the
    // `ReRender` without draining the callbacks (issue #1540).
    rinch_core::queue_main_callback(Box::new(f));
    send_native_event(RinchNativeEvent::ReRender);
}

/// Dispatcher function for cross-thread signal updates.
///
/// Registered with `rinch_core::set_cross_thread_dispatcher()` so that
/// `Signal::send()` can automatically route updates to the main thread. Same
/// shared queue as [`run_on_main_thread`], same coalesced wake.
///
/// Every dispatch asks for a `ReRender`; [`NativeEventQueue`] keeps at most
/// one pending between two native drains, so a burst still wakes the loop
/// once. It used to ask only when the main queue was empty, and a paint's
/// native drain (which runs no callbacks) could consume that `ReRender`,
/// leaving the queue non-empty and every later dispatch asking for nothing
/// (issue #1540).
fn dispatch_to_main_thread(f: Box<dyn FnOnce() + Send>) {
    rinch_core::queue_main_callback(f);
    send_native_event(RinchNativeEvent::ReRender);
}

/// Set `GLOBAL_PROXY` and pay the wake owed to anything queued before it
/// existed (issue #1540). Both desktop entry points call this.
fn install_global_proxy(proxy: &EventLoopProxy) {
    let _ = GLOBAL_PROXY.set(proxy.clone());
    owe_startup_wake(|| proxy.wake_up());
}

/// The wake owed once the proxy exists (issue #1540): a `ReRender` sent
/// before then — by the mount's signal-change callback, or by a worker's
/// `Signal::send` — carried no wake, and it holds `NativeEventQueue`'s
/// coalescing flag up, so every later `ReRender` coalesces into it and asks
/// for no wake either. A window that paints drains it at its first frame; a
/// windowless or hidden app (tray-only, started hidden) never does, and
/// stalled. A callback queued plainly before the dispatcher existed (#1541)
/// is given a `ReRender` here first. `wake` is called when a native event is
/// waiting; returns whether it was.
fn owe_startup_wake(wake: impl FnOnce()) -> bool {
    if rinch_core::main_callbacks_pending() {
        send_native_event(RinchNativeEvent::ReRender);
    }
    let owed = !NATIVE_EVENT_QUEUE.lock().unwrap().is_empty();
    if owed {
        wake();
    }
    owed
}

/// Events sent to the event loop.
#[derive(Debug, Clone)]
pub enum RinchNativeEvent {
    /// A signal changed -- re-resolve layout and repaint.
    ReRender,
    /// A debug command is waiting on the channel (debug feature).
    #[cfg(feature = "debug")]
    DebugCommand,
    /// Minimize the window.
    MinimizeWindow,
    /// Toggle maximize state.
    ToggleMaximizeWindow,
    /// Close the window (from window controls).
    CloseWindowControl,
    /// Show the window.
    ShowWindow,
    /// Hide the window.
    HideWindow,
    /// Let pointer events through the window, or stop (issue #1519).
    SetClickThrough(bool),
    /// Move the window's outer top-left to this logical position (#1546).
    SetWindowPosition(f64, f64),
    /// Resize the window's inner area to this logical size (#1546).
    SetWindowSize(f64, f64),
    /// Apply the title `rinch::window::set_window_title` stored (#1770).
    ApplyWindowTitle,
    /// Show a Wayland layer-shell window on this output (#1523).
    SetWindowOutput(Option<String>),
    /// Start moving the window with the pointer (#1523).
    StartWindowDrag,
    /// Look again for an output to show a layer surface on, after the
    /// compositor closed it with no output left (#1523).
    RetryLayerOutput,
    /// Start resizing the window from an edge with the pointer (#1523).
    StartWindowResize(rinch_platform::ResizeDirection),
    /// An injected platform event (e.g. synthetic gamepad input).
    InjectedPlatformEvent(PlatformEvent),
    /// An accessibility action requested by the AT (delivered from the adapter's
    /// thread; applied to the focused editor on the main thread).
    #[cfg(feature = "a11y")]
    A11yAction(accesskit::ActionRequest),
}

/// Inject a synthetic [`PlatformEvent`] into the rinch event loop.
///
/// The event is queued and processed on the next event loop tick, just like
/// real platform events from winit. The runtime uses the current window size
/// and scale factor when dispatching.
///
/// This is useful for translating gamepad input or other external input
/// sources into rinch UI events (e.g. arrow key presses for focus navigation).
///
/// Must be called from the main thread or any thread — the event is queued
/// behind a `Mutex` and the event loop proxy is woken.
///
/// # Example
///
/// ```ignore
/// use rinch_platform::{PlatformEvent, KeyCode, KeyRepeat, Modifiers};
///
/// // Simulate a down-arrow key press for gamepad D-pad
/// rinch::inject_platform_event(PlatformEvent::KeyDown {
///     key: KeyCode::ArrowDown,
///     logical_key: None,
///     text: None,
///     modifiers: Modifiers::default(),
///     // An injector that re-sends a held D-pad every frame should say
///     // `KeyRepeat::Repeat` for the ones after the first, or
///     // `KeyRepeat::Unknown` to let the runtime infer it (issue #463).
///     repeat: KeyRepeat::Fresh,
/// });
/// ```
pub fn inject_platform_event(event: PlatformEvent) {
    send_native_event(RinchNativeEvent::InjectedPlatformEvent(event));
}

// ── Pointer sources ──────────────────────────────────────────────────────────

/// Touch pointers are numbered from here, so a finger never shares an id
/// with the mouse (1) or a tablet tool (2).
const TOUCH_POINTER_BASE: u64 = 1 << 32;

/// A winit force as a pressure in `0.0..=1.0`; with none measured, the
/// Pointer Events default (`0.5` while down, else `0`).
fn force_pressure(force: Option<&winit::event::Force>, down: bool) -> f32 {
    match force {
        Some(f) => (f.normalized(None) as f32).clamp(0.0, 1.0),
        None if down => 0.5,
        None => 0.0,
    }
}

/// A tablet tool's kind as a surface pointer's: an eraser is an eraser, a
/// puck or lens a mouse, a finger on the tablet a touch, every stylus a pen.
fn tool_kind(kind: winit::event::TabletToolKind) -> SurfacePointerKind {
    match kind {
        winit::event::TabletToolKind::Eraser => SurfacePointerKind::Eraser,
        winit::event::TabletToolKind::Mouse | winit::event::TabletToolKind::Lens => {
            SurfacePointerKind::Mouse
        }
        winit::event::TabletToolKind::Finger => SurfacePointerKind::Touch,
        _ => SurfacePointerKind::Pen,
    }
}

/// The one id a tablet tool has: winit numbers neither tools nor tablets.
const TABLET_POINTER_ID: u64 = 2;

/// [`SurfacePointer::id`] of the pointer a winit source describes.
fn pointer_id_of(source: &winit::event::PointerSource) -> u64 {
    match source {
        winit::event::PointerSource::Touch { finger_id, .. } => {
            TOUCH_POINTER_BASE + finger_id.into_raw() as u64
        }
        winit::event::PointerSource::TabletTool { .. } => TABLET_POINTER_ID,
        _ => SurfacePointer::mouse(false).id,
    }
}

/// `os_file_drag`'s steps carried out on winit's event loop (#1962); the
/// events are collected and handed to the app after, in order.
struct WinitDragHost<'a> {
    event_loop: &'a dyn ActiveEventLoop,
    events: Vec<PlatformEvent>,
}

impl super::os_file_drag::DragHost for WinitDragHost<'_> {
    fn accept(
        &mut self,
        id: winit::data_transfer::DataTransferId,
        actions: &[winit::event_loop::DndAction],
    ) -> Result<(), String> {
        self.event_loop
            .set_valid_dnd_actions(id, actions)
            .map_err(|e| e.to_string())
    }

    fn fetch_uri_list(
        &mut self,
        id: winit::data_transfer::DataTransferId,
    ) -> Result<winit::event_loop::AsyncRequestSerial, String> {
        self.event_loop
            .fetch_data_transfer(id, &winit::data_transfer::TypeHint::UriList)
            .map_err(|e| e.to_string())
    }

    fn emit(&mut self, event: PlatformEvent) {
        self.events.push(event);
    }

    fn warn(&mut self, message: String) {
        tracing::warn!("{message}");
    }
}

/// The pointer a winit move came from, as a render surface hears it. `down`:
/// that pointer has a button or contact down, which decides the pressure a
/// device that cannot measure it reports.
fn surface_pointer_of(
    source: &winit::event::PointerSource,
    primary: bool,
    down: bool,
) -> SurfacePointer {
    use winit::event::PointerSource;
    match source {
        PointerSource::Mouse | PointerSource::Unknown => SurfacePointer {
            primary,
            ..SurfacePointer::mouse(down)
        },
        PointerSource::Touch { finger_id, force } => SurfacePointer {
            id: TOUCH_POINTER_BASE + finger_id.into_raw() as u64,
            kind: SurfacePointerKind::Touch,
            pressure: force_pressure(force.as_ref(), down),
            primary,
        },
        PointerSource::TabletTool { kind, data } => SurfacePointer {
            id: TABLET_POINTER_ID,
            kind: tool_kind(*kind),
            pressure: force_pressure(data.force.as_ref(), down),
            primary,
        },
        // `PointerSource` is non-exhaustive (winit 0.31.0-beta.3): a source
        // this shell does not know is a mouse, as `Unknown` is.
        _ => SurfacePointer {
            primary,
            ..SurfacePointer::mouse(down)
        },
    }
}

/// [`surface_pointer_of`] for a winit button event. `pressed`: the button
/// went down (a release reports no pressure).
fn surface_pointer_of_button(
    button: &winit::event::ButtonSource,
    primary: bool,
    pressed: bool,
) -> SurfacePointer {
    use winit::event::{ButtonSource, PointerSource};
    let source = match button {
        ButtonSource::Mouse(_) | ButtonSource::Unknown(_) => PointerSource::Mouse,
        ButtonSource::Touch { finger_id, force } => PointerSource::Touch {
            finger_id: *finger_id,
            force: *force,
        },
        ButtonSource::TabletTool { kind, data, .. } => PointerSource::TabletTool {
            kind: *kind,
            data: data.clone(),
        },
        // Non-exhaustive (winit 0.31.0-beta.3): an unknown source is a mouse.
        _ => PointerSource::Mouse,
    };
    let mut pointer = surface_pointer_of(&source, primary, pressed);
    if !pressed {
        pointer.pressure = 0.0;
    }
    pointer
}

/// The buttons or contacts down, as `(SurfacePointer::id, button)`: what
/// decides the pressure a move reports for a device that cannot measure it.
/// Per button, so a chord's release (right up while left is held) leaves the
/// pointer down.
#[derive(Debug, Default)]
struct PressedButtons(Vec<(u64, Option<MouseButton>)>);

impl PressedButtons {
    /// `button` of pointer `id` went down (`pressed`) or up.
    fn note(&mut self, id: u64, button: Option<MouseButton>, pressed: bool) {
        self.0.retain(|k| *k != (id, button));
        if pressed {
            self.0.push((id, button));
        }
    }

    /// Whether pointer `id` has any button or contact down.
    fn is_down(&self, id: u64) -> bool {
        self.0.iter().any(|(p, _)| *p == id)
    }

    /// Whether the primary button (a mouse's left, a finger, a pen's tip) of
    /// any pointer is down.
    fn primary_down(&self) -> bool {
        self.0.iter().any(|(_, b)| *b == Some(MouseButton::Left))
    }

    /// Forget every button of pointer `id`; answers whether it had any down.
    fn lift(&mut self, id: u64) -> bool {
        let was = self.is_down(id);
        self.0.retain(|(p, _)| *p != id);
        was
    }

    /// Forget every press (the window lost focus).
    fn clear(&mut self) {
        self.0.clear();
    }
}

/// Resets the pointer `set_current_pointer` named when dropped.
struct ForgetCurrentPointer;

impl Drop for ForgetCurrentPointer {
    fn drop(&mut self) {
        crate::render_surface::set_current_pointer(None);
    }
}

// ── RinchRuntime ─────────────────────────────────────────────────────────────

/// The desktop runtime: thin winit `ApplicationHandler` that delegates to
/// [`RinchApp`] for all platform-agnostic logic and uses [`WgpuRenderer`]
/// + [`WinitWindow`] for the platform-specific parts.
pub struct RinchRuntime {
    /// Platform-agnostic application logic.
    app: RinchApp,
    /// Desktop window wrapper.
    window: Option<WinitWindow>,
    /// GPU renderer (only available with `gpu` feature).
    #[cfg(feature = "gpu")]
    renderer: Option<WgpuRenderer>,
    /// Software renderer (CPU pixel presentation via softbuffer). Every
    /// desktop build has it; a `gpu` build presents with it when `renderer` is
    /// `None` (software chosen, or the GPU could not start).
    soft_renderer: Option<super::softbuffer_renderer::SoftbufferRenderer>,
    /// Event loop proxy for sending events.
    proxy: Option<EventLoopProxy>,
    /// Window title.
    title: String,
    width: u32,
    height: u32,
    /// Current keyboard modifier state (winit-specific).
    modifiers: winit::keyboard::ModifiersState,
    /// Native menu bar (attached to the window).
    native_menu: Option<muda::Menu>,
    /// True while [`Self::drain_native_events`] is running. Prevents re-entrant
    /// drains: `DebugCommandKind::Screenshot` calls [`Self::paint`] from inside
    /// the drain, and the `RedrawRequested` arm drains before painting (#153).
    draining_native_events: bool,
    /// The newest pointer position winit has reported that the app has not
    /// been handed yet (logical px), and which pointer it is. See
    /// [`Self::flush_pointer_move`].
    pending_pointer_move: Option<(f32, f32, SurfacePointer)>,
    /// The OS file drag over the main window, from winit's data transfer
    /// events (#1962).
    os_file_drag: super::os_file_drag::OsFileDrag,
    /// The buttons or contacts down, as `(SurfacePointer::id, button)`: what
    /// decides the pressure a move reports for a device that cannot measure
    /// it. Per button, so a chord's release (right up while left is held)
    /// leaves the pointer down.
    pressed_pointers: PressedButtons,
    /// The cursor last applied to the window, so an unchanged
    /// `AppAction::SetCursor` — which hover emits on every pointer move — does
    /// not reach the windowing system. Reset when the window is dropped.
    applied_cursor: Option<rinch_platform::CursorStyle>,
    /// Whether the main window lets pointer events through: starts at
    /// `WindowProps::click_through`, changed by
    /// [`set_click_through_current_window`](crate::windows::set_click_through_current_window).
    click_through: bool,
    /// Whether the main window has been on screen yet. `false` after a
    /// hidden start (#1520) until the first `show_window`, which then places
    /// it as a first creation would.
    shown_once: bool,
    /// The main window's geometry as it was hidden (#1546), or as the app
    /// set it while hidden: what `show_window` re-creates it with.
    saved_geometry: SavedGeometry,
    /// The monitors connected when the main window was last (re-)created:
    /// a requested or saved position must be reachable on one of them.
    monitors: Vec<window_geometry::MonitorRect>,
    /// A main window created maximized, until the platform says it is (the
    /// X11 window manager maximizes it later).
    maximize_wait: window_geometry::MaximizeWait,
    /// The live main window's layer-shell state, while it is a Wayland
    /// layer surface (#1523).
    layer: Option<LayerState>,
    /// The output `set_current_window_output` named, over
    /// `WindowProps::output` (#1523).
    output_override: Option<Option<String>>,
    /// A layer surface was closed by the compositor with no output left: show
    /// it again when one appears (#1523).
    layer_awaiting_output: bool,
    /// Frames left in which the GPU window composites its `GameViewport`s as
    /// layers under the UI (cleared in the scene) rather than as split base
    /// layers (#1866): a base split was refused, so the window falls back
    /// for a while before it tries again.
    #[cfg(feature = "gpu")]
    base_fallback: super::base_fallback::BaseFallback,

    // ── DevTools ─────────────────────────────────────────────────
    /// Shared DevTools store (persists across open/close cycles).
    devtools_store: Option<super::devtools_store::DevToolsStore>,
    /// DevTools window app.
    devtools_app: Option<RinchApp>,
    /// DevTools window.
    devtools_window: Option<WinitWindow>,
    /// DevTools GPU renderer.
    #[cfg(feature = "gpu")]
    devtools_renderer: Option<WgpuRenderer>,
    /// DevTools software renderer (used whenever `devtools_renderer` is `None`).
    devtools_soft_renderer: Option<super::softbuffer_renderer::SoftbufferRenderer>,
    /// DevTools keyboard modifier state.
    devtools_modifiers: winit::keyboard::ModifiersState,
    /// Previous hovered node ID (for detecting changes).
    devtools_prev_hovered: Option<u64>,
    /// Timestamp of the last paint (for FPS calculation).
    last_paint_time: Option<std::time::Instant>,
    /// Recent frame times in ms (ring buffer for FPS averaging).
    frame_times: VecDeque<f64>,

    // ── IME ──────────────────────────────────────────────────────
    /// The IME state last applied to the window, so [`Self::sync_ime`] only
    /// issues a winit `request_ime_update` on an actual change
    /// ([`ImeState::change_to`](crate::app::ImeState::change_to)).
    ime_applied: crate::app::ImeState,

    // ── Accessibility (M7b) ──────────────────────────────────────
    /// The per-platform AccessKit bridge (Linux AT-SPI; no-op elsewhere). Created
    /// lazily on first window creation; pushes the editor's tree on change.
    #[cfg(feature = "a11y")]
    a11y: Option<crate::editor::a11y::AccesskitBridge>,
    /// The (doc, selection) last pushed to the AT, so the tree is re-derived only
    /// when the focused editor actually changed.
    #[cfg(feature = "a11y")]
    a11y_last: Option<(rinch_editor_core::Node, rinch_editor_core::Selection)>,

    /// Test builds only (#1733): the lock every test that queues or drains the
    /// process-wide main-thread and native queues holds. Dropping a runtime
    /// runs [`Self::shutdown`], which drops every queued main-thread callback
    /// unrun, so a test runtime holds the lock from construction until after
    /// that clear (fields drop after `Drop::drop`). Last, so it is released
    /// last. Shared by every runtime alive on one thread, so a test may hold
    /// two.
    #[cfg(test)]
    _queue_lock: TestQueueLock,
}

/// One thread's share of `RERENDER_EVENTS_TEST_LOCK` for its test runtimes
/// (#1733): the first runtime on the thread takes the lock, the last to drop
/// releases it.
#[cfg(test)]
struct TestQueueLock;

#[cfg(test)]
thread_local! {
    static TEST_QUEUE_LOCK: RefCell<Option<(std::sync::MutexGuard<'static, ()>, usize)>> =
        const { RefCell::new(None) };
}

#[cfg(test)]
impl TestQueueLock {
    fn hold() -> Self {
        TEST_QUEUE_LOCK.with(|held| {
            let mut held = held.borrow_mut();
            match held.as_mut() {
                Some((_, count)) => *count += 1,
                None => {
                    let guard = crate::app::RERENDER_EVENTS_TEST_LOCK
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    *held = Some((guard, 1));
                }
            }
        });
        TestQueueLock
    }
}

#[cfg(test)]
impl Drop for TestQueueLock {
    fn drop(&mut self) {
        // The guard is dropped after the thread-local's borrow ends.
        let released = TEST_QUEUE_LOCK.with(|held| {
            let mut held = held.borrow_mut();
            let (_guard, count) = held.as_mut()?;
            *count -= 1;
            if *count == 0 { held.take() } else { None }
        });
        drop(released);
    }
}

impl RinchRuntime {
    fn new(
        title: &str,
        width: u32,
        height: u32,
        component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static,
        fonts: &[crate::font::AppFont],
    ) -> Self {
        // Before the window, before the component, and therefore before the
        // first layout pass: an app-bundled face has to be in the font context
        // that measures the first frame, or that frame is measured against a
        // fallback and reflows once the real face arrives.
        #[cfg(test)]
        let _queue_lock = TestQueueLock::hold();
        // The named video sink factory (rinch-core's hook), so video players
        // and rinch-rtc's remote tracks deliver frames through the
        // RenderSurface compositing pipeline. Here rather than in the two
        // entry points, which create the event loop and cannot be tested.
        rinch_core::video_sink::set_video_sink_factory(
            crate::render_surface::create_video_frame_sink,
        );
        let mut app = RinchApp::new(component);
        app.register_app_fonts(fonts);
        // The main window's size, scale and focus reach `rinch::window`
        // (#1770); the DevTools window's do not.
        app.set_publishes_window_state(true);
        Self {
            app,
            window: None,
            #[cfg(feature = "gpu")]
            renderer: None,
            soft_renderer: None,
            proxy: None,
            title: title.to_string(),
            width,
            height,
            modifiers: winit::keyboard::ModifiersState::empty(),
            native_menu: None,
            draining_native_events: false,
            pending_pointer_move: None,
            os_file_drag: Default::default(),
            pressed_pointers: PressedButtons::default(),
            applied_cursor: None,
            click_through: false,
            shown_once: false,
            saved_geometry: SavedGeometry::default(),
            monitors: Vec::new(),
            maximize_wait: window_geometry::MaximizeWait::Done,
            layer: None,
            output_override: None,
            layer_awaiting_output: false,
            #[cfg(feature = "gpu")]
            base_fallback: Default::default(),
            devtools_store: None,
            devtools_app: None,
            devtools_window: None,
            #[cfg(feature = "gpu")]
            devtools_renderer: None,
            devtools_soft_renderer: None,
            devtools_modifiers: winit::keyboard::ModifiersState::empty(),
            devtools_prev_hovered: None,
            last_paint_time: None,
            frame_times: VecDeque::with_capacity(60),
            ime_applied: crate::app::ImeState::default(),
            #[cfg(feature = "a11y")]
            a11y: None,
            #[cfg(feature = "a11y")]
            a11y_last: None,
            #[cfg(test)]
            _queue_lock,
        }
    }

    /// Explicit shutdown: drop resources in the correct order and clear global state.
    fn shutdown(&mut self) {
        // 0. Close DevTools first.
        self.close_devtools();

        // 1. Clear the signal-change callback so no stale closures fire during drop.
        rinch_core::clear_on_signal_change();

        // 2. Drop any pending main-thread callbacks (they may capture app state).
        rinch_core::clear_main_callbacks();

        // 3. Drop the app first (disposes effects/scopes before GPU resources).
        //    RinchApp's own drop order handles _render_scope before doc.
        drop(self.app.component.take());
        drop(self.app._render_scope.take());
        drop(self.app.doc.take());

        // 4. Drop renderer before window — Surface holds a window handle reference.
        #[cfg(feature = "gpu")]
        drop(self.renderer.take());
        drop(self.soft_renderer.take());

        // 5. Window can now be dropped safely.
        drop(self.window.take());
    }

    // ── DevTools window management ──────────────────────────────────────────

    /// Ensure the DevToolsStore exists (created once, persists across open/close).
    fn ensure_devtools_store(&mut self) {
        if self.devtools_store.is_none() {
            let store = super::devtools_store::DevToolsStore::new();
            rinch_core::create_store(store);
            self.devtools_store = Some(store);
        }
    }

    /// Toggle the DevTools window open/closed.
    fn toggle_devtools(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.devtools_window.is_some() {
            self.close_devtools();
        } else {
            self.open_devtools(event_loop);
        }
    }

    /// Open the DevTools window.
    fn open_devtools(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.devtools_window.is_some() {
            return; // Already open
        }

        self.ensure_devtools_store();
        let store = self.devtools_store.unwrap();
        store.visible.set(true);

        // Provide the main app's document to DevTools via context so it can read directly
        if let Some(doc) = &self.app.doc {
            rinch_core::create_context(super::devtools_store::MainDocRef(doc.clone()));
        }

        // Create DevTools window
        let window_attrs = WindowAttributes::default()
            .with_title("DevTools")
            .with_surface_size(winit::dpi::LogicalSize::new(480u32, 600u32));

        let window = event_loop
            .create_window(window_attrs)
            .expect("Failed to create DevTools window");

        let size = window.surface_size();
        let dt_scale = window.scale_factor();

        // DevTools presents the way the main window does: on the GPU only when
        // the main window is, and with software if the GPU will not start for
        // this window too.
        #[cfg(feature = "gpu")]
        if self.renderer.is_some() {
            match WgpuRenderer::try_new(&*window, size.width.max(1), size.height.max(1)) {
                Ok(gpu) => self.devtools_renderer = Some(gpu),
                Err(e) => {
                    tracing::warn!("DevTools: GPU renderer unavailable ({e}); using software")
                }
            }
        }
        #[cfg(not(feature = "gpu"))]
        let _ = size;

        let winit_window = WinitWindow::new(window);
        self.devtools_window = Some(winit_window);

        // Create a separate RinchApp for DevTools with the panel component
        let mut dt_app = RinchApp::new(super::devtools_panel::devtools_root);

        // Mount the DevTools component at its logical viewport size, with the
        // display scale pushed first (issue #211) — same order as the main
        // window mount.
        dt_app.set_device_pixel_ratio(dt_scale);
        let dt_logical = to_logical((size.width, size.height), dt_scale);
        dt_app.mount_component(dt_logical.0 as f32, dt_logical.1 as f32);

        // Inject DevTools CSS into the document
        if let Some(doc) = &dt_app.doc {
            let mut d = doc.borrow_mut();
            d.load_css(super::devtools_css::DEVTOOLS_CSS);
            d.recompute_all_styles_full();
            d.resolve_layout(dt_logical.0 as f32, dt_logical.1 as f32);
        }

        self.devtools_app = Some(dt_app);

        // Bump version to trigger initial tree read in DevTools effects
        store.bump_version();

        // Resolve DevTools layout with the initial data
        self.resolve_devtools();

        // Request initial draw
        self.devtools_window.as_ref().unwrap().request_redraw();

        tracing::info!("DevTools: opened");
    }

    /// Close the DevTools window. Store persists.
    fn close_devtools(&mut self) {
        if let Some(store) = &self.devtools_store {
            store.visible.set(false);
        }

        // Drop DevTools app
        if let Some(mut dt_app) = self.devtools_app.take() {
            drop(dt_app.component.take());
            drop(dt_app._render_scope.take());
            drop(dt_app.doc.take());
        }

        // Drop renderer before window
        #[cfg(feature = "gpu")]
        drop(self.devtools_renderer.take());
        drop(self.devtools_soft_renderer.take());

        drop(self.devtools_window.take());

        tracing::info!("DevTools: closed");
    }

    /// Toggle inspect mode on/off.
    fn toggle_inspect_mode(&mut self) {
        self.ensure_devtools_store();
        if let Some(store) = &self.devtools_store {
            let current = store.inspect_mode.get();
            store.inspect_mode.set(!current);
            if current {
                // Turning off: clear highlights
                store.hovered_node_id.set(None);
                self.app.inspect_highlight = None;
                // The highlight's last rect is the damage (`last_inspect_rect`).
                self.app.request_repaint();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            tracing::info!("Inspect mode: {}", if !current { "ON" } else { "OFF" });
        }
    }

    /// Update inspect highlight from the DevTools store.
    /// Called before painting the main window.
    fn update_inspect_highlight(&mut self) {
        let Some(store) = &self.devtools_store else {
            self.app.inspect_highlight = None;
            return;
        };

        let hovered_id = store.hovered_node_id.get();
        let new_rect = hovered_id.and_then(|node_id| {
            let doc = self.app.doc.as_ref()?;
            let d = doc.borrow();
            d.tree.nodes.get(node_id)?;
            // The box the node is *painted* in, so the inspect overlay outlines
            // what is on screen rather than the untransformed layout box (#203).
            let r = rinch_dom::paint::painted_border_box(&d.tree, node_id, 1.0);
            Some((
                r.x0 as f32,
                r.y0 as f32,
                r.width() as f32,
                r.height() as f32,
            ))
        });

        if self.app.inspect_highlight != new_rect {
            self.app.inspect_highlight = new_rect;
            // Its old and new rects are the damage (`last_inspect_rect`).
            self.app.request_repaint();
        }
    }

    /// Get the DevTools window ID (if open).
    fn devtools_window_id(&self) -> Option<WindowId> {
        self.devtools_window.as_ref().map(|w| w.window.id())
    }

    /// The DevTools window's surface size, in physical pixels — what
    /// `RinchApp::handle_event` expects (it derives the logical viewport
    /// itself). Falls back to the size DevTools is created at.
    fn devtools_size(&self) -> (u32, u32) {
        self.devtools_window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or((480, 600))
    }

    /// The DevTools window's scale factor.
    fn devtools_scale_factor(&self) -> f64 {
        self.devtools_window
            .as_ref()
            .map(|w| w.scale_factor())
            .unwrap_or(1.0)
    }

    /// Paint the DevTools window.
    fn paint_devtools(&mut self) -> Result<(), String> {
        #[cfg(feature = "gpu")]
        if self.devtools_renderer.is_some() {
            return self.paint_devtools_gpu();
        }
        self.paint_devtools_software()
    }

    fn paint_devtools_software(&mut self) -> Result<(), String> {
        let Some(window) = &self.devtools_window else {
            return Ok(());
        };
        let Some(dt_app) = &mut self.devtools_app else {
            return Ok(());
        };

        let scale = window.scale_factor();
        let size = window.logical_size();
        let (_base, w, h) = dt_app.build_pixels(scale, size, false);

        let pixels = dt_app
            .skia_painter
            .as_ref()
            .map(|p| p.pixels())
            .unwrap_or(&[]);

        if self.devtools_soft_renderer.is_none() {
            self.devtools_soft_renderer =
                Some(super::softbuffer_renderer::SoftbufferRenderer::new(
                    window.window.clone(),
                    w,
                    h,
                    false,
                ));
        }

        if let Some(renderer) = &mut self.devtools_soft_renderer {
            renderer.present_pixels(pixels, w, h);
        }

        Ok(())
    }

    #[cfg(feature = "gpu")]
    fn paint_devtools_gpu(&mut self) -> Result<(), String> {
        let Some(renderer) = &mut self.devtools_renderer else {
            return Ok(());
        };
        let Some(window) = &self.devtools_window else {
            return Ok(());
        };
        let Some(dt_app) = &mut self.devtools_app else {
            return Ok(());
        };

        let scale = window.scale_factor();
        let size = window.logical_size();
        let scene = dt_app.build_scene(scale, size);
        renderer.paint(scene, false)?;
        Ok(())
    }

    /// Resolve and repaint the DevTools window after signal changes.
    fn resolve_devtools(&mut self) {
        let Some(dt_app) = &mut self.devtools_app else {
            return;
        };
        let Some(window) = &self.devtools_window else {
            return;
        };
        let size = window.logical_size();
        let changed = dt_app.resolve_and_repaint(size.0 as f32, size.1 as f32);
        if changed {
            window.request_redraw();
        }
    }

    /// The attributes of the main window, for its first creation and for
    /// the re-creation [`Self::show_window`] makes, with the overlay steps
    /// (#1519) folded in. `first` installs the Wayland icon and reports an icon
    /// that does not decode; a re-creation does neither again.
    /// `first_appearance` is the window's first time on screen, which takes
    /// the requested position when no geometry was saved.
    fn main_window_attributes(
        &self,
        steps: WindowSteps,
        backend: Backend,
        first: bool,
        first_appearance: bool,
    ) -> WindowAttributes {
        let mut window_attrs = WindowAttributes::default()
            .with_title(&self.title)
            .with_surface_size(winit::dpi::LogicalSize::new(self.width, self.height));

        // app_id on Wayland (always resolved) and WM_CLASS on X11 (only an
        // explicit app_id; winit's argv[0] default otherwise, as before).
        #[cfg(target_os = "linux")]
        let app_id: Option<String> = self
            .app
            .window_props
            .as_ref()
            .map(|props| resolved_app_id(props.app_id.as_deref()));
        #[cfg(target_os = "linux")]
        if let Some(id) = &app_id {
            *WINDOW_APP_ID.lock().unwrap_or_else(|p| p.into_inner()) = Some(id.clone());
        }
        #[cfg(not(target_os = "linux"))]
        let app_id: Option<String> = None;
        let x11_class: Option<String> = self
            .app
            .window_props
            .as_ref()
            .and_then(|props| props.app_id.clone());
        if let Some(props) = &self.app.window_props {
            if props.borderless {
                window_attrs = window_attrs.with_decorations(false);
            }
            if props.transparent {
                window_attrs = window_attrs.with_transparent(true);
            }
            if !props.resizable {
                window_attrs = window_attrs.with_resizable(false);
            }
            // Every raised level is winit's `AlwaysOnTop`; macOS sets the
            // exact `StatusBar` / `ScreenSaver` level after creation (#1549).
            if props.effective_level() != rinch_core::element::WindowLevel::Normal {
                window_attrs =
                    window_attrs.with_window_level(winit::window::WindowLevel::AlwaysOnTop);
            }
            if let Some(icon_data) = props.icon {
                // Set window icon (works on X11 and Windows)
                match load_window_icon(icon_data) {
                    Ok(icon) => {
                        window_attrs = window_attrs.with_window_icon(Some(icon));
                    }
                    Err(e) if first => {
                        tracing::warn!("Failed to load window icon: {}", e);
                    }
                    Err(_) => {}
                }
                // On Wayland, write icon to a temp file and install a .desktop entry
                // so the compositor can find it via the app_id.
                #[cfg(target_os = "linux")]
                if first {
                    let app_id = resolved_app_id(props.app_id.as_deref());
                    install_wayland_icon(&app_id, icon_data);
                }
            }
        }
        // The geometry the window had when it was hidden (#1546), or the
        // requested `WindowProps::x`/`y` on its first appearance — a first
        // creation, or the first show after a hidden start (#1520).
        let requested = self.requested_position().map(|r| r.at);
        let requested_maximized = self.app.window_props.as_ref().is_some_and(|p| p.maximized);
        let window_attrs = self.saved_geometry.apply(
            window_attrs,
            requested,
            requested_maximized,
            first_appearance,
        );
        // A Wayland layer surface (#1523) is placed by its margins: the
        // saved position, or the requested one on a first appearance.
        let layer = steps.wayland_layer.then(|| {
            LayerRequest::new(
                self.app
                    .window_props
                    .as_ref()
                    .unwrap_or(&Default::default()),
                self.layer_position(first_appearance),
                self.layer_output(),
                app_id.as_deref().unwrap_or("rinch"),
            )
        });
        overlay_window::apply_to_attributes(
            window_attrs,
            steps,
            backend,
            app_id.as_deref(),
            x11_class.as_deref(),
            layer.as_ref(),
        )
    }

    /// The overlay steps (#1519) for the main window on `backend`. The
    /// `first` creation starts click-through from
    /// `WindowProps::click_through`; a re-creation by `show_window` keeps the
    /// state the runtime toggle left.
    fn main_window_steps(&mut self, backend: Backend, first: bool) -> WindowSteps {
        if first {
            self.click_through = self
                .app
                .window_props
                .as_ref()
                .is_some_and(|p| p.click_through);
        }
        match &self.app.window_props {
            Some(props) => WindowSteps::plan(props, self.click_through, backend),
            None => WindowSteps::plan(&Default::default(), self.click_through, backend),
        }
    }

    /// Record a click-through toggle (`set_click_through_current_window`) so
    /// a window re-created by `show_window` comes back the same. Returns the
    /// state to apply to the live window.
    fn note_click_through(&mut self, click_through: bool) -> bool {
        self.click_through = click_through;
        self.click_through
    }

    /// Keep the saved normal geometry and the published one current
    /// (#1546). Only for a window that is on screen: a hidden start's sizing
    /// window sits wherever it was created.
    fn note_geometry(&mut self, window: &dyn winit::window::Window) {
        if !self.shown_once {
            return;
        }
        self.take_reading(
            window_geometry::WindowReading::of(window),
            monitor_name(window),
        );
    }

    /// [`Self::note_geometry`] past the window: a reading of the live main
    /// window, through the wait for a maximize the platform has not reported
    /// yet.
    /// End the maximize wait with one reading of the live main window,
    /// believed: when its grace ends (a window the window manager never
    /// maximized, moved inside the grace and then left alone, has had no
    /// reading since) and at close (what `on_close_requested` reads).
    fn settle_maximize_wait(&mut self) {
        if self.maximize_wait == window_geometry::MaximizeWait::Done {
            return;
        }
        let read = self.window.as_ref().filter(|_| self.shown_once).map(|w| {
            (
                window_geometry::WindowReading::of(w.raw()),
                monitor_name(w.raw()),
            )
        });
        self.settle_maximize_wait_with(read);
    }

    /// [`Self::settle_maximize_wait`] past the window.
    fn settle_maximize_wait_with(
        &mut self,
        read: Option<(window_geometry::WindowReading, Option<String>)>,
    ) {
        self.maximize_wait = window_geometry::MaximizeWait::Done;
        if let Some((reading, monitor)) = read {
            self.take_reading(reading, monitor);
        }
    }

    fn take_reading(&mut self, reading: window_geometry::WindowReading, monitor: Option<String>) {
        let reading = self
            .maximize_wait
            .filter(reading, std::time::Instant::now());
        self.saved_geometry.note(&reading);
        window_geometry::publish_window(&reading, monitor);
        self.publish_layer_position();
    }

    /// Where a layer surface goes (#1523): the saved position (logical,
    /// output-relative), or `WindowProps::x`/`y` on a first appearance;
    /// `None` lets the compositor centre it.
    fn layer_position(&self, first_appearance: bool) -> Option<(f64, f64)> {
        match self.saved_geometry.position {
            Some(Position::Logical(p)) => return Some((p.x, p.y)),
            Some(Position::Physical(p)) => return Some((p.x as f64, p.y as f64)),
            None => {}
        }
        let props = self.app.window_props.as_ref()?;
        first_appearance
            .then(|| Some((props.x? as f64, props.y? as f64)))
            .flatten()
    }

    /// The output a layer surface is shown on (#1523).
    fn layer_output(&self) -> Option<String> {
        match &self.output_override {
            Some(output) => output.clone(),
            None => self.app.window_props.as_ref()?.output.clone(),
        }
    }

    /// After a main window is created: publish what it got (#1523) and keep
    /// the layer-shell state of a layer surface.
    fn note_surface_kind(&mut self, window: &dyn winit::window::Window, first_appearance: bool) {
        let kind = super::layer_shell::surface_kind_of(window);
        super::layer_shell::publish_surface_kind(kind);
        self.layer = (kind == WindowSurfaceKind::LayerShell).then(|| {
            let position = self.layer_position(first_appearance);
            if let Some((x, y)) = position {
                self.saved_geometry.position = Some(Position::Logical(LogicalPosition::new(x, y)));
            }
            LayerState::new(position)
        });
    }

    /// Publish a layer surface's position, which no window reading has
    /// (Wayland reports none): its margins (#1523).
    fn publish_layer_position(&self) {
        if let Some(position) = self.layer.as_ref().and_then(|l| l.position) {
            window_geometry::publish_hidden(Some(position), None);
        }
    }

    /// Move the live layer surface to `position` (logical, on its output),
    /// anchored top-left.
    fn place_layer(&mut self, position: (f64, f64)) {
        let Some(layer) = self.layer.as_mut() else {
            return;
        };
        layer.place(position);
        #[cfg(target_os = "linux")]
        if let Some(w) = self.window.as_ref() {
            use winit::platform::wayland::{LayerShellAnchor, WindowExtWayland};
            let raw = w.raw();
            raw.set_layer_shell_anchor(LayerShellAnchor::TOP_LEFT);
            // Confirmed by a `WindowEvent::Moved` (`layer_moved`).
            raw.set_layer_shell_margins(super::layer_shell::margins(position));
        }
        self.saved_geometry.position = Some(Position::Logical(LogicalPosition::new(
            position.0, position.1,
        )));
        window_geometry::publish_hidden(Some(position), None);
    }

    /// Start a move or resize of a layer surface at the pointer (#1523): the
    /// compositor has none. `false` when the window is no layer surface.
    fn start_layer_gesture(&mut self, kind: GestureKind) -> bool {
        let Some(w) = self.window.as_ref() else {
            return false;
        };
        if self.layer.is_none() {
            return false;
        }
        // Only while the primary button is held: nothing but its release ends
        // the gesture, so one started without it would follow the hovering
        // pointer (a compositor refuses a move with no live press, too).
        if !self.pressed_pointers.primary_down() {
            tracing::debug!("window drag/resize refused: no button is down");
            return true;
        }
        let scale = w.scale_factor();
        let size = w.raw().surface_size().to_logical::<f64>(scale);
        let size = (size.width, size.height);
        let position = match self.layer.as_ref().and_then(|l| l.position) {
            Some(p) => p,
            // Centred by the compositor: read where, from the output the
            // surface is on.
            None => {
                let output = w.raw().current_monitor().and_then(|m| {
                    let mode = m.current_video_mode()?.size();
                    Some((mode.width as f64 / scale, mode.height as f64 / scale))
                });
                let p = super::layer_shell::centred_position(output.unwrap_or(size), size);
                self.place_layer(p);
                if let Some(layer) = self.layer.as_mut() {
                    layer.place_centred(p);
                }
                p
            }
        };
        let (cx, cy) = self.app.cursor_pos.unwrap_or((0.0, 0.0));
        if let Some(layer) = self.layer.as_mut() {
            layer.gesture = Some(Gesture::start(kind, (cx as f64, cy as f64), position, size));
        }
        true
    }

    /// Follow the pointer, at `local` (logical, over the surface), with the
    /// layer gesture. `false` when there is none.
    fn update_layer_gesture(
        &mut self,
        local: (f64, f64),
        event_loop: &dyn ActiveEventLoop,
    ) -> bool {
        let Some(resize) = self.follow_layer_gesture(local) else {
            return false;
        };
        if let Some(new_size) = resize {
            let current = self.window.as_ref().map(|w| {
                let s = w.raw().surface_size().to_logical::<f64>(w.scale_factor());
                (s.width, s.height)
            });
            if current != Some(new_size) {
                let size = Size::Logical(LogicalSize::new(new_size.0, new_size.1));
                if let Some(w) = &self.window {
                    let id = w.raw().id();
                    if let Some(physical) = w.raw().request_surface_size(size) {
                        self.window_event(event_loop, id, WindowEvent::SurfaceResized(physical));
                    }
                }
            }
        }
        true
    }

    /// The window-free half of [`Self::update_layer_gesture`]: move the
    /// surface where the gesture puts the pointer at `local`, read against
    /// where the compositor has the surface ([`LayerState`]). `None` when no
    /// gesture runs; `Some(Some(size))` for a resize, which the caller asks
    /// the window for.
    fn follow_layer_gesture(&mut self, local: (f64, f64)) -> Option<Option<(f64, f64)>> {
        let layer = self.layer.as_ref()?;
        let kind = layer.gesture?.kind;
        let (new_position, new_size) = layer.follow(local)?;
        if Some(new_position) != layer.position {
            self.place_layer(new_position);
        }
        Some(matches!(kind, GestureKind::Resize(_)).then_some(new_size))
    }

    /// Whether the GPU surface is made to composite with its alpha (#1523):
    /// for a `WindowProps::transparent` window.
    #[cfg_attr(not(feature = "gpu"), allow(dead_code))]
    fn gpu_surface_transparent(&self) -> bool {
        self.app.is_transparent()
    }

    /// A layer surface's `WindowEvent::Moved` (#1523): the compositor has
    /// processed the move to `physical`, and pointer positions after this
    /// are over the surface there.
    fn layer_moved(&mut self, physical: (i32, i32)) {
        let scale = self.scale_factor();
        if let Some(layer) = self.layer.as_mut() {
            layer.moved(super::layer_shell::moved_origin(physical, scale));
        }
    }

    /// The compositor closed the layer surface (#1523): its output was
    /// unplugged or disabled. Make it again, at the same position and size,
    /// on the output the compositor picks — the named one may be gone, so
    /// the name is dropped until `set_current_window_output` sets one again.
    /// With no output left, wait for one.
    fn layer_surface_closed(&mut self, event_loop: &dyn ActiveEventLoop) {
        tracing::warn!(
            "the compositor closed the layer surface (its output went away); making it again"
        );
        self.output_override = Some(None);
        self.hide_window();
        if event_loop.available_monitors().next().is_some() {
            self.show_window(event_loop);
        } else {
            self.layer_awaiting_output = true;
            arm_layer_output_retry();
        }
    }

    /// Whether the main window asked to stay on top: `always_on_top`, or
    /// any raised `WindowProps::level` (#1549).
    fn main_window_always_on_top(&self) -> bool {
        self.app
            .window_props
            .as_ref()
            .is_some_and(|p| p.effective_level() != rinch_core::element::WindowLevel::Normal)
    }

    /// Take a title `set_window_title` stored (#1770) into `self.title`, so a
    /// window created later (or re-created by `show_window`) has it, and set
    /// it on the live window.
    fn take_stored_title(&mut self) {
        if let Some(title) = crate::window::take_stored_title() {
            if let Some(w) = &self.window {
                w.raw().set_title(&title);
            }
            self.title = title;
        }
    }

    /// `WindowProps::x`/`y`, when the window's top would be reachable on the
    /// monitors connected now (`self.monitors`); otherwise the system places
    /// the window, so a position saved on a monitor since unplugged never
    /// opens it out of reach. `at` is where winit is asked to put it
    /// ([`window_geometry::place_requested`]: desktop pixels, resolved in the
    /// scale of the monitor the position is on).
    fn requested_position(&self) -> Option<RequestedPosition> {
        let props = self.app.window_props.as_ref()?;
        let (x, y) = (props.x?, props.y?);
        let size = Size::Logical(LogicalSize::new(self.width as f64, self.height as f64));
        let placed = window_geometry::place_requested(
            (x as f64, y as f64),
            size,
            props.monitor.as_deref(),
            &self.monitors,
        )?;
        Some(RequestedPosition {
            logical: (x as f64, y as f64),
            at: placed.at,
            monitor: placed.monitor,
        })
    }

    /// The geometry half of putting the main window on screen, before winit
    /// creates it: the monitors connected now (`monitors`), a saved position
    /// out of reach on them dropped (one unplugged while the window was
    /// hidden), and on its first appearance — a creation, or the first show
    /// after a hidden start (#1520) — the geometry it is created with
    /// published as the one an un-maximize returns to: a window that opens
    /// maximized has had no normal geometry to read. `on_screen` is false for
    /// a hidden start's sizing window. Returns whether this is the first
    /// appearance.
    fn begin_appearance(
        &mut self,
        monitors: Vec<window_geometry::MonitorRect>,
        on_screen: bool,
    ) -> bool {
        self.monitors = monitors;
        if !on_screen {
            return false;
        }
        let first = !std::mem::replace(&mut self.shown_once, true);
        if let Some(position) = self.saved_geometry.position {
            let size = self
                .saved_geometry
                .size
                .unwrap_or(Size::Logical(LogicalSize::new(
                    self.width as f64,
                    self.height as f64,
                )));
            if !window_geometry::reachable(position, Some(size), &self.monitors) {
                self.saved_geometry.position = None;
            }
        }
        if first {
            let requested = self.requested_position();
            window_geometry::publish_created(
                requested.as_ref().map(|r| r.logical),
                requested.and_then(|r| r.monitor),
                (self.width as f64, self.height as f64),
                self.app.window_props.as_ref().is_some_and(|p| p.maximized),
            );
        }
        first
    }

    fn create_window(&mut self, event_loop: &dyn ActiveEventLoop) {
        // A title set before the loop started (#1770).
        self.take_stored_title();
        let hidden = starts_hidden(self.app.window_props.as_ref());
        self.begin_appearance(connected_monitors(event_loop), !hidden);
        let backend = Backend::of(event_loop);
        let steps = self.main_window_steps(backend, true);
        let window_attrs = hidden_start_attributes(
            self.main_window_attributes(steps, backend, true, true),
            hidden,
        );
        self.maximize_wait = window_geometry::MaximizeWait::after_create(
            &window_attrs,
            steps.wayland_layer,
            std::time::Instant::now(),
        );

        let window = event_loop
            .create_window(window_attrs)
            .expect("Failed to create window");
        // A hidden start skips the post-creation steps: they would show a
        // window created hidden (`AfterCreate::Show`), and they run on the
        // window `show_window` creates instead.
        if !hidden {
            overlay_window::after_create(&*window, steps, self.main_window_always_on_top());
            self.note_surface_kind(&*window, true);
            self.note_geometry(&*window);
        }

        let size = window.surface_size();
        let scale = window.scale_factor();

        // Create the GPU renderer when the choice wants one and it starts; the
        // software renderer is created lazily by the first software paint. A
        // window that starts hidden gets none: it is destroyed before it could
        // present, and `show_window` creates the renderer with the window it
        // creates.
        #[cfg(feature = "gpu")]
        if !hidden {
            self.renderer = start_gpu_renderer(&*window, self.gpu_surface_transparent());
        }
        #[cfg(not(feature = "gpu"))]
        announce_software_renderer();

        // Attach native menu bar to the window if configured
        if let Some(menu) = &self.native_menu {
            crate::menu::attach_menu_to_window(menu, &*window);
        }

        // Wrap the winit window
        let winit_window = WinitWindow::new(window);
        self.window = Some(winit_window);

        // Register direct redraw callback so render surfaces can request
        // repaints without routing through the event loop queue.
        let window_arc = self.window.as_ref().unwrap().window.clone();
        crate::render_surface::set_redraw_callback(Arc::new(move || {
            window_arc.request_redraw();
        }));

        // Mount the component at the *logical* viewport size — `size` above is
        // the physical surface size (see `to_logical`). The display scale goes
        // in first so the initial style resolution sees the right
        // `device_pixel_ratio` (issue #211) — winit does not promise a
        // `ScaleFactorChanged` at window creation.
        self.app.set_device_pixel_ratio(scale);
        let logical = to_logical((size.width, size.height), scale);
        self.app.mount_component(logical.0 as f32, logical.1 as f32);

        // `WindowProps::visible: false` (issue #1520): the app runs with no
        // window on screen — a tray-only app — until `show_current_window`.
        // The window above existed only to give the root its size and display
        // scale; the root is mounted now, once, and `show_window` re-creates
        // the OS window over the same document without mounting it again.
        // Destroyed rather than kept invisible for the reason `hide_window`
        // gives: Wayland has no invisible toplevel. No frame was requested and
        // no renderer made, so nothing was ever presented.
        if hidden {
            self.hide_window();
            return;
        }

        // Request initial draw
        self.window.as_ref().unwrap().request_redraw();
    }

    /// Hide the window by destroying the OS window and GPU surface.
    ///
    /// On Wayland, `set_visible(false)` is a no-op, so we must destroy the
    /// actual winit Window (which destroys the xdg_toplevel) to make it
    /// disappear. The app state and DOM are preserved.
    fn hide_window(&mut self) {
        if let Some(w) = &self.window {
            // Keep where the window is and how big (#1546): `show_window`
            // creates the new one with it. Not the sizing window of a hidden
            // start (#1520), which was never on screen and sits wherever the
            // system put it.
            if self.shown_once {
                let reading = self.maximize_wait.filter(
                    window_geometry::WindowReading::of(w.raw()),
                    std::time::Instant::now(),
                );
                self.saved_geometry.record(&reading);
                window_geometry::publish_window(&reading, monitor_name(w.raw()));
                self.publish_layer_position();
            }
            // The menu bar leaves the window with it (Windows: muda keeps a
            // list of the HWNDs it serves, and a recycled HWND would be
            // refused at the next attach); `show_window` attaches it again.
            if let Some(menu) = &self.native_menu {
                crate::menu::detach_menu_from_window(menu, w.raw());
            }
        }
        // A window hidden to the tray has no focus to report losing (#1770).
        self.app.publish_window_state(crate::window::WindowFacts {
            focused: Some(false),
            ..Default::default()
        });
        // Clear the direct redraw callback before dropping the window.
        crate::render_surface::clear_redraw_callback();
        // Drop renderer first — it holds a reference to the window surface.
        #[cfg(feature = "gpu")]
        drop(self.renderer.take());
        drop(self.soft_renderer.take());
        drop(self.window.take());
        // Its position stays in `saved_geometry`; a gesture ends with it.
        self.layer = None;
        // A re-created window starts on the default cursor.
        self.applied_cursor = None;
    }

    /// Show a previously hidden window by recreating the OS window and GPU surface.
    ///
    /// Rebuilds the winit Window and wgpu renderer, then triggers a repaint
    /// of the existing DOM.
    fn show_window(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.window.is_some() {
            return; // Already visible
        }

        let backend = Backend::of(event_loop);
        let steps = self.main_window_steps(backend, false);
        // After a hidden start (#1520) the first show is the window's first
        // appearance, so it takes the requested position as a first creation
        // does; a restore from the tray comes back where it was hidden
        // (#1546), with the size it had.
        let first_show = self.begin_appearance(connected_monitors(event_loop), true);
        let window_attrs = self.main_window_attributes(steps, backend, false, first_show);
        self.maximize_wait = window_geometry::MaximizeWait::after_create(
            &window_attrs,
            steps.wayland_layer,
            std::time::Instant::now(),
        );

        let window = event_loop
            .create_window(window_attrs)
            .expect("Failed to recreate window");
        overlay_window::after_create(&*window, steps, self.main_window_always_on_top());
        self.note_surface_kind(&*window, first_show);

        #[cfg(feature = "gpu")]
        {
            self.renderer = start_gpu_renderer(&*window, self.gpu_surface_transparent());
        }

        // The new window gets the menu bar the first one had (#1533: on
        // Windows the bar is per HWND, so a re-created window had none, and
        // its chords were skipped by rinch's matcher while no bar fired them).
        if let Some(menu) = &self.native_menu {
            crate::menu::attach_menu_to_window(menu, &*window);
            // Windows: `SetMenu` keeps the outer rect and takes the bar out of
            // the client area, so the content would lose the bar's height on
            // every hide / show. Ask for the recorded content size again.
            #[cfg(target_os = "windows")]
            if let Some(size) = self.saved_geometry.size {
                let _ = window.request_surface_size(size);
            }
        }
        self.note_geometry(&*window);

        let winit_window = WinitWindow::new(window);
        self.window = Some(winit_window);

        // Update direct redraw callback for the new window.
        let window_arc = self.window.as_ref().unwrap().window.clone();
        crate::render_surface::set_redraw_callback(Arc::new(move || {
            window_arc.request_redraw();
        }));

        // Trigger a full repaint of the existing DOM: a new surface, and
        // nothing named what it should show.
        self.app.mark_scene_dirty();
        self.window.as_ref().unwrap().request_redraw();
    }

    /// Paint the current scene to the window.
    fn paint(&mut self) -> Result<(), String> {
        // Update inspect highlight rect from DevTools store before painting
        self.update_inspect_highlight();

        let paint_start = std::time::Instant::now();

        let result = {
            #[cfg(feature = "gpu")]
            {
                if self.renderer.is_some() {
                    self.paint_gpu()
                } else {
                    self.paint_software()
                }
            }
            #[cfg(not(feature = "gpu"))]
            {
                self.paint_software()
            }
        };

        // Track frame timing for DevTools Performance panel
        if let Some(last) = self.last_paint_time {
            let frame_ms = paint_start.duration_since(last).as_secs_f64() * 1000.0;
            if self.frame_times.len() >= 60 {
                self.frame_times.pop_front();
            }
            self.frame_times.push_back(frame_ms);

            if let Some(store) = &self.devtools_store {
                if store.visible.get() {
                    let paint_ms = paint_start.elapsed().as_secs_f64() * 1000.0;
                    store.frame_time_ms.set(paint_ms);

                    // Average FPS over recent frames
                    if !self.frame_times.is_empty() {
                        let avg_ms: f64 =
                            self.frame_times.iter().sum::<f64>() / self.frame_times.len() as f64;
                        if avg_ms > 0.0 {
                            store.fps.set(1000.0 / avg_ms);
                        }
                    }
                }
            }
        }
        self.last_paint_time = Some(paint_start);

        // One redraw is one performance frame (`rinch_dom::perf`).
        self.app.end_perf_frame();

        result
    }

    fn paint_software(&mut self) -> Result<(), String> {
        let Some(window) = &self.window else {
            return Ok(());
        };

        // Ensure pending cross-thread closures are processed and layout
        // is resolved before painting (same as paint_gpu).
        rinch_core::drain_main_callbacks();
        rinch_core::reactive::drain_polls();
        let scale = window.scale_factor();
        // Layout and paint work in logical (CSS) pixels; `inner_size` is
        // physical (see `to_logical`).
        let size = window.logical_size();
        if self.app.has_pending_layout() {
            self.app.resolve_and_repaint(size.0 as f32, size.1 as f32);
        }

        let transparent = self.app.is_transparent();
        let s = scale as f32;

        // Update layout sizes for render surfaces. Scans the DOM for
        // data-render-surface attributes rather than relying on the registry,
        // so surfaces survive reactive scope rebuilds (where unregister runs
        // after re-mount, temporarily removing the surface from the registry).
        self.app.update_render_surface_layouts(s, true);

        // Invoke per-frame render callbacks before collecting frames.
        crate::render_surface::invoke_render_callbacks();

        // Collect surface pixel data and set it for inline painting.
        // Surfaces are painted inline during paint_document() (like <img> elements).
        let surface_pixels = crate::render_surface::collect_surface_pixels_by_id();
        if !surface_pixels.is_empty() {
            // Mark scene dirty so build_pixels() actually repaints — the
            // damage is the surface nodes, marked just below.
            self.app.request_repaint();
            // ...and mark the surface nodes themselves, for the same reason the
            // video viewports are marked below: inline painting is subject to
            // the dirty-region cache, and a small dirty region elsewhere would
            // otherwise prune the surface's subtree and freeze its last frame.
            // This has to happen here, at collect time — the collector above
            // has just cleared `needs_redraw`, so nothing downstream can still
            // tell which surfaces delivered a frame.
            let ids: Vec<usize> = surface_pixels.keys().copied().collect();
            self.app.mark_surface_nodes_paint_dirty(&ids);
            rinch_dom::paint::set_surface_pixels(Some(surface_pixels));
        }

        // Also update layout sizes for video/GameViewport surfaces that use the
        // viewport_name path (data-viewport attribute).
        let reg_names = crate::render_surface::registered_viewport_names();
        for viewport_name in &reg_names {
            if let Some(viewport) = self.app.viewport_rect(viewport_name) {
                let phys_w = (viewport.2 * s) as u32;
                let phys_h = (viewport.3 * s) as u32;
                crate::render_surface::update_layout_size(viewport_name, phys_w, phys_h);
                crate::render_surface::update_layout_position(
                    viewport_name,
                    viewport.0,
                    viewport.1,
                );
            }
        }

        // Video and `GameViewport` frames paint **inline**, during paint, at
        // the viewport node's own z-order (issues #358, #361) — the software
        // backend has no compositor path. That is what lets a drawer, a modal,
        // a dropdown or a game's HUD above one survive: ordinary paint order
        // does the occluding, with no occlusion tracking anywhere. They used to
        // be blitted over the finished pixels, which destroyed all of those.
        //
        // Only a `GameViewport` still punches a hole in its ancestors'
        // backgrounds, and only while it has a frame (#186); its letterbox is
        // that hole. Video punches none and `rinch-dom` aspect-fits it over an
        // opaque black fill, which is #354's letterbox bars on this backend.
        //
        // Inline painting is subject to the dirty-region cache, so the viewport
        // nodes are marked as the damage — see `install_viewport_frames`.
        self.app
            .install_viewport_frames(crate::render_surface::collect_viewport_frames_by_name());

        // Build the scene — surfaces paint inline at their layout positions
        let (_base, w, h) = self.app.build_pixels(scale, size, transparent);

        RinchApp::clear_viewport_frames();
        rinch_dom::paint::set_surface_pixels(None);

        let pixels = self
            .app
            .skia_painter
            .as_ref()
            .map(|p| p.pixels())
            .unwrap_or(&[]);

        // Lazily create or get the softbuffer renderer
        if self.soft_renderer.is_none() {
            self.soft_renderer = Some(super::softbuffer_renderer::SoftbufferRenderer::new(
                window.window.clone(),
                w,
                h,
                transparent,
            ));
        }

        let present_start = std::time::Instant::now();
        if let Some(renderer) = &mut self.soft_renderer {
            renderer.present_pixels(pixels, w, h);
        }
        self.app.perf_add(
            rinch_dom::perf::Counter::TimePresentNs,
            present_start.elapsed().as_nanos() as u64,
        );

        Ok(())
    }

    #[cfg(feature = "gpu")]
    fn paint_gpu(&mut self) -> Result<(), String> {
        let Some(renderer) = &mut self.renderer else {
            return Ok(());
        };
        let Some(window) = &self.window else {
            return Ok(());
        };

        // Ensure pending cross-thread closures (run_on_main_thread) are
        // processed and layout is resolved before painting. Without this,
        // continuous RedrawRequested from render surfaces can paint with
        // stale layout if the ReRender event from signal changes hasn't
        // been processed yet.
        rinch_core::drain_main_callbacks();
        rinch_core::reactive::drain_polls();
        let scale = window.scale_factor();
        // Layout and paint work in logical (CSS) pixels; `inner_size` is
        // physical (see `to_logical`).
        let size = window.logical_size();
        if self.app.has_pending_layout() {
            self.app.resolve_and_repaint(size.0 as f32, size.1 as f32);
        }

        let transparent = self.app.is_transparent();
        let s = scale as f32;

        // Update layout sizes for all render surfaces. DOM-based scan
        // so surfaces survive reactive scope rebuilds.
        self.app.update_render_surface_layouts(s, true);

        // Also update layout sizes for video/GameViewport surfaces that still
        // use the viewport_name path (data-viewport attribute).
        let reg_names = crate::render_surface::registered_viewport_names();
        for viewport_name in &reg_names {
            if let Some(viewport) = self.app.viewport_rect(viewport_name) {
                let phys_w = (viewport.2 * s) as u32;
                let phys_h = (viewport.3 * s) as u32;
                crate::render_surface::update_layout_size(viewport_name, phys_w, phys_h);
                crate::render_surface::update_layout_position(
                    viewport_name,
                    viewport.0,
                    viewport.1,
                );
            }
        }

        // Invoke per-frame render callbacks before collecting frames.
        crate::render_surface::invoke_render_callbacks();

        // Read back GPU textures to CPU for inline-paint RenderSurface components.
        // This must happen before collect_surface_pixels_by_id().
        crate::render_surface::readback_gpu_textures();

        // Collect CPU surface pixel data for inline painting (RenderSurface path).
        // After readback, this includes both pure-CPU and readback-GPU surfaces.
        let surface_pixels = crate::render_surface::collect_surface_pixels_by_id();
        if !surface_pixels.is_empty() {
            // The surface nodes are the damage, as on the software path: a
            // partial frame elsewhere would otherwise prune them (#1718).
            self.app.request_repaint();
            let ids: Vec<usize> = surface_pixels.keys().copied().collect();
            self.app.mark_surface_nodes_paint_dirty(&ids);
            rinch_dom::paint::set_surface_pixels(Some(surface_pixels));
        }

        // `RenderSurface` GPU frames (#1768): each surface's target texture,
        // registered with this window's Vello renderer and drawn in the scene
        // as the surface element's content. A surface that drew this paint is
        // the damage. Always synced, so a surface that stopped drawing (or
        // left) has its texture unregistered.
        let gpu_frames = crate::render_surface::collect_gpu_surface_frames();
        let fresh: Vec<usize> = gpu_frames
            .iter()
            .filter(|(_, _, fresh)| *fresh)
            .map(|(id, ..)| *id)
            .collect();
        let gpu_images = renderer
            .sync_surface_images(gpu_frames.into_iter().map(|(id, texture, _)| (id, texture)));
        // A surface the window composites split (#1865) is not damage: its new
        // frame is composited from the kept slices, with no repaint.
        self.app.mark_fresh_surfaces(
            &fresh,
            &renderer.split_surface_ids(),
            &renderer.split_stale_ids(),
        );
        if !gpu_images.is_empty() {
            rinch_dom::paint::set_surface_gpu_images(Some(gpu_images));
        }

        // `GameViewport` surfaces still use the compositor/hole-punch path.
        // Track which viewport names have compositor layers so we can set
        // ACTIVE_VIEWPORTS to prevent hole-punching for inline-painted surfaces.
        let all_layers: Vec<rinch_platform::CompositeLayer> = Vec::new();
        let mut gpu_layers = Vec::new();
        let mut compositor_viewport_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        // A viewport that says it is not ready (`data-viewport-ready`) gets no
        // layer (#348): paint cuts it no hole and its node paints its own
        // background, and a layer under a node whose background is transparent
        // would show its last frame where the software backend shows none.
        let mut skipped_unready = false;

        // `GameViewport`s as split base layers (#1866): what paint offers to
        // the painter, the GPU textures and the CPU frames the compositor
        // reads. The layers above are gathered too, for a frame that falls
        // back to them.
        let try_base = self.base_fallback.try_split();
        let mut base_viewports: std::collections::HashMap<String, rinch_dom::paint::BaseViewport> =
            std::collections::HashMap::new();
        let mut base_views = std::collections::HashMap::new();
        let mut base_pending: Vec<super::split_frame::PendingFrame> = Vec::new();
        // The CPU frames as the layered path's layers would be built from them.
        #[allow(clippy::type_complexity)]
        let mut cpu_layers: Vec<(
            std::sync::Arc<Vec<u8>>,
            u32,
            u32,
            (f32, f32, f32, f32),
            (f32, f32, f32, f32),
            [f32; 4],
            Option<(f32, f32, f32, f32)>,
        )> = Vec::new();

        // Collect compositor-path surface frames (video, GameViewport — not RenderSurface)
        {
            let surface_frames = crate::render_surface::collect_surface_frames_shared();
            for (id, viewport_name, pixels, surf_w, surf_h, fresh) in surface_frames {
                if !self.app.viewport_ready(&viewport_name) {
                    skipped_unready = true;
                    continue;
                }
                if let Some((viewport, radii)) = self.app.viewport_rect_with_radius(&viewport_name)
                {
                    compositor_viewport_names.insert(viewport_name.clone());
                    if surf_w > 0 && surf_h > 0 {
                        base_viewports.insert(
                            viewport_name.clone(),
                            rinch_dom::paint::BaseViewport {
                                surface_id: id,
                                extents: (surf_w, surf_h),
                                letterbox: true,
                            },
                        );
                        // Shared, not copied; uploaded only when new, or
                        // when the window holds no texture for it.
                        base_pending.push(super::base_fallback::pending_base_frame(
                            id,
                            &pixels,
                            (surf_w, surf_h),
                            fresh,
                        ));
                    }
                    // The whole box — what paint punched the hole over.
                    let box_viewport = (
                        viewport.0 * s,
                        viewport.1 * s,
                        viewport.2 * s,
                        viewport.3 * s,
                    );
                    // Letterbox: fit source within viewport preserving aspect ratio.
                    let viewport = {
                        let (vx, vy, vw, vh) = box_viewport;
                        let src_aspect = surf_w as f32 / surf_h.max(1) as f32;
                        let vp_aspect = vw / vh.max(1.0);
                        if (src_aspect - vp_aspect).abs() < 0.001 {
                            box_viewport
                        } else if src_aspect > vp_aspect {
                            let fit_h = vw / src_aspect;
                            let offset_y = (vh - fit_h) / 2.0;
                            (vx, vy + offset_y, vw, fit_h)
                        } else {
                            let fit_w = vh * src_aspect;
                            let offset_x = (vw - fit_w) / 2.0;
                            (vx + offset_x, vy, fit_w, vh)
                        }
                    };
                    let border_radius = [radii[0] * s, radii[1] * s, radii[2] * s, radii[3] * s];
                    let clip_rect = self
                        .app
                        .viewport_clip_rect(&viewport_name)
                        .map(|cr| (cr.0 * s, cr.1 * s, cr.2 * s, cr.3 * s));
                    // Black backdrop under the frame, covering the whole box, so
                    // the letterbox bars are black rather than see-through
                    // (issue #354). Pushed first: the compositor draws layers in
                    // order. Its radii are corrected to the corners the box
                    // actually shares with its clip, or a viewport that is only
                    // the top half of a rounded card would have its *middle*
                    // rounded away, leaving a see-through notch.
                    //
                    // Built only for the layered path: the split path reads the
                    // shared frame, and a copy per frame is not free.
                    cpu_layers.push((
                        pixels,
                        surf_w,
                        surf_h,
                        box_viewport,
                        viewport,
                        border_radius,
                        clip_rect,
                    ));
                }
            }
        }

        // Extract GPU texture sources for compositor — only non-inline surfaces
        // (video, GameViewport). Inline RenderSurface GPU textures are read back
        // to CPU above and painted inline.
        {
            let texture_sources = crate::render_surface::collect_texture_sources();
            for (surface_id, viewport_name, is_inline, tex_source_arc) in texture_sources {
                // Skip inline surfaces — they're already read back to CPU pixels
                if is_inline {
                    continue;
                }

                if let Some((viewport, radii)) = self.app.viewport_rect_with_radius(&viewport_name)
                {
                    let phys_w = (viewport.2 * s) as u32;
                    let phys_h = (viewport.3 * s) as u32;
                    crate::render_surface::update_layout_size_by_id(surface_id, phys_w, phys_h);

                    let viewport = (
                        viewport.0 * s,
                        viewport.1 * s,
                        viewport.2 * s,
                        viewport.3 * s,
                    );
                    let border_radius = [radii[0] * s, radii[1] * s, radii[2] * s, radii[3] * s];
                    let clip_rect = self
                        .app
                        .viewport_clip_rect(&viewport_name)
                        .map(|cr| (cr.0 * s, cr.1 * s, cr.2 * s, cr.3 * s));
                    if !self.app.viewport_ready(&viewport_name) {
                        // Sized above, so the app goes on rendering at the
                        // right size; only the layer is withheld.
                        skipped_unready = true;
                        continue;
                    }
                    // A hole only once there is a texture to fill it, on both
                    // paths (#1883 review): before, a viewport whose source
                    // was cleared punched with nothing behind it.
                    if let Some(ref ts) = *tex_source_arc.lock().unwrap() {
                        compositor_viewport_names.insert(viewport_name.clone());
                        base_viewports.insert(
                            viewport_name.clone(),
                            rinch_dom::paint::BaseViewport {
                                surface_id,
                                extents: (ts.width, ts.height),
                                letterbox: false,
                            },
                        );
                        base_views.insert(surface_id, ts.view.clone());
                        gpu_layers.push(super::desktop::GpuTextureLayer {
                            view: ts.view.clone(),
                            viewport,
                            border_radius,
                            clip_rect,
                        });
                    }
                }
            }
        }

        // Set or clear composite layers on the renderer (`GameViewport` CPU
        // frames and GPU textures only). Video is drawn in the scene since
        // #1768, so the retention a loaded video needed between its frames
        // (the last cycle's layers kept, with their holes) is gone with it.
        //
        // On the split path (#1866) the viewports are base layers and the
        // renderer holds no layers: the UI is rendered over the window's base
        // and the compositor replaces it inside each viewport with its game.
        let use_base = try_base && !base_viewports.is_empty();
        let layered = move || {
            let mut all_layers = all_layers;
            for (pixels, w, h, box_viewport, viewport, border_radius, clip_rect) in cpu_layers {
                all_layers.push(black_backdrop_layer(
                    box_viewport,
                    radii_at_shared_corners(box_viewport, clip_rect, border_radius),
                    clip_rect,
                ));
                all_layers.push(rinch_platform::CompositeLayer {
                    pixels: pixels.as_ref().clone(),
                    width: w,
                    height: h,
                    viewport,
                    border_radius,
                    clip_rect,
                });
            }
            (all_layers, gpu_layers)
        };
        let mut fallback_layers = Some(layered);
        if use_base {
            if renderer.has_composite_layers() {
                renderer.set_composite_layers(vec![]);
                renderer.set_gpu_layers(vec![]);
            }
            // A kept slice made for a frame of another size has a stale
            // image transform: the viewport is painted, and split, again.
            let stale: Vec<&str> = base_viewports
                .iter()
                .filter(|(_, b)| {
                    renderer
                        .split
                        .kept_extents(b.surface_id)
                        .is_some_and(|e| e != b.extents)
                })
                .map(|(name, _)| name.as_str())
                .collect();
            if !stale.is_empty() {
                self.app.request_repaint();
                self.app.mark_viewport_nodes_paint_dirty(&stale);
            }
        } else if let Some((all_layers, gpu_layers)) = fallback_layers.take().map(|f| f())
            && (!all_layers.is_empty() || !gpu_layers.is_empty() || skipped_unready)
        {
            renderer.set_composite_layers(all_layers);
            renderer.set_gpu_layers(gpu_layers);
        } else if renderer.has_composite_layers() {
            renderer.set_composite_layers(vec![]);
            renderer.set_gpu_layers(vec![]);
        }

        // Set active viewports so hole-punching only applies to compositor
        // surfaces (`GameViewport`), not inline-painted ones.
        //
        // Installed unconditionally, empty set included. Skipping the call when
        // nothing has a layer left `ACTIVE_VIEWPORTS` at `None`, which
        // `viewport_punches` reads as "no filter — punch everything": the
        // gate was inoperative in exactly the case it exists for, a viewport
        // with no frame behind it (issue #186).
        //
        // Through the app rather than straight into paint: the set is an input
        // to the scene, so a set that differs from the last paint's has to
        // repaint the viewports it differs in (issue #349). `build_scene`
        // otherwise returns the cached scene, with last paint's holes in it —
        // a hole left behind by a surface that unregistered, or none yet for
        // one that has just delivered its first frame.
        //
        // Video frames go in with them: painted inline, in the scene (#1768).
        // Split GPU surfaces and video (#1865) while no `GameViewport` layer is
        // under the UI: the base-layer path and the split path do not compose
        // yet (a base layer under split slices is the design's next step).
        let has_layers = !use_base && (renderer.has_composite_layers() || skipped_unready);
        let mut viewport_frames = crate::render_surface::collect_viewport_frames_by_name();
        let mut split_uploads = super::split_frame::prepare_split_viewports(
            &mut viewport_frames,
            &mut renderer.split,
            !has_layers,
        );
        self.app
            .install_gpu_viewport_frames(viewport_frames, Some(compositor_viewport_names));
        if use_base {
            split_uploads.extend(base_pending);
        }
        renderer.split.set_base_views(if use_base {
            base_views
        } else {
            Default::default()
        });

        // Build the frame from the document — CPU surfaces paint inline via
        // draw_image(). Only what its damage reaches is encoded and rendered
        // while the renderer's UI texture holds the last frame (#1718). A
        // refused base split builds it again, in full, layered (R1).
        let target = renderer.ui_invalid_reason(transparent);
        let mut fell_back = false;
        let split_change = self.base_fallback.build(
            &mut self.app,
            scale,
            size,
            use_base.then_some(base_viewports),
            !has_layers,
            target,
            || fell_back = true,
        );
        if fell_back {
            if let Some((all_layers, gpu_layers)) = fallback_layers.take().map(|f| f()) {
                renderer.set_composite_layers(all_layers);
                renderer.set_gpu_layers(gpu_layers);
            }
            renderer.split.set_base_views(Default::default());
            split_uploads.clear();
        }
        let frame = self.app.last_gpu_frame();
        let has_layers = renderer.has_composite_layers();

        RinchApp::clear_viewport_frames();
        rinch_dom::paint::set_surface_pixels(None);
        rinch_dom::paint::set_surface_gpu_images(None);
        rinch_dom::paint::set_viewport_split_ids(None);

        // Render to screen
        let present_start = std::time::Instant::now();
        let before: Vec<usize> = renderer.split_surface_ids();
        if let Some(change) = split_change {
            renderer.apply_split(change)?;
        }
        if has_layers {
            renderer.split.clear();
        }
        // After the change: only the videos this frame keeps split upload
        // their frame (#1906 review: a hidden or refused one uploaded too).
        let uploads = {
            let renderer = &mut *renderer;
            renderer
                .split
                .upload_split_frames(&renderer.device, &renderer.queue, split_uploads)
        };

        #[cfg(debug_assertions)]
        {
            let after = renderer.split_surface_ids();
            for id in after.iter().filter(|id| !before.contains(id)) {
                tracing::debug!("rinch: render surface {id} composited split (#1865)");
            }
            for id in before.iter().filter(|id| !after.contains(id)) {
                tracing::debug!("rinch: render surface {id} drawn in the scene (#1865)");
            }
        }
        #[cfg(not(debug_assertions))]
        let _ = before;
        renderer.paint_frame(frame, transparent)?;
        self.app
            .perf_add(rinch_dom::perf::Counter::SplitFrameUploads, uploads);
        self.app.perf_add(
            rinch_dom::perf::Counter::TimePresentNs,
            present_start.elapsed().as_nanos() as u64,
        );

        Ok(())
    }

    /// Get the current window size, in physical pixels.
    ///
    /// This is what the OS surface and the renderer are sized in, and what
    /// pointer coordinates arrive in — `RinchApp::handle_event` divides it by
    /// the scale factor itself. Layout and paint want [`Self::logical_size`].
    ///
    /// Before the window exists there is no surface, so the fallback is the
    /// requested size, which `create_window` passes as a `LogicalSize`; the
    /// scale factor is 1x until then, so the two agree.
    fn window_size(&self) -> (u32, u32) {
        self.window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or((self.width, self.height))
    }

    /// Get the current window size in logical (CSS) pixels.
    ///
    /// Only the software debug-screenshot path needs this; the paint paths
    /// derive it from the `window` they already hold.
    #[cfg(feature = "debug")]
    fn logical_size(&self) -> (u32, u32) {
        // `self.width`/`self.height` are already the requested *logical* size
        // (`create_window` hands them to winit as a `LogicalSize`), so the
        // no-window fallback must not divide them again.
        self.window
            .as_ref()
            .map(|w| w.logical_size())
            .unwrap_or((self.width.max(1), self.height.max(1)))
    }

    /// Get the current scale factor.
    fn scale_factor(&self) -> f64 {
        self.window
            .as_ref()
            .map(|w| w.scale_factor())
            .unwrap_or(1.0)
    }

    /// Process the actions returned by RinchApp.
    fn process_actions(&mut self, actions: Vec<AppAction>, event_loop: &dyn ActiveEventLoop) {
        for action in actions {
            match action {
                AppAction::RequestRedraw => {
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
                AppAction::Exit => {
                    event_loop.exit();
                }
                AppAction::SetMinimized(minimized) => {
                    if let Some(w) = &self.window {
                        w.set_minimized(minimized);
                    }
                }
                AppAction::SetMaximized(_) => {
                    // Toggle maximize: the app doesn't know current state,
                    // so we toggle it here.
                    if let Some(w) = &self.window {
                        let is_max = w.is_maximized();
                        w.set_maximized(!is_max);
                    }
                }
                AppAction::SetVisible(visible) => {
                    // The app decides now: no pending re-show (#1523).
                    self.layer_awaiting_output = false;
                    if visible {
                        self.show_window(event_loop);
                    } else {
                        self.hide_window();
                    }
                }
                AppAction::DragWindow => {
                    // A layer surface has no compositor move (#1523).
                    if !self.start_layer_gesture(GestureKind::Move) {
                        if let Some(w) = &self.window {
                            let _ = w.drag_window();
                        }
                    }
                }
                AppAction::DragResizeWindow(dir) => {
                    if !self.start_layer_gesture(GestureKind::Resize(dir)) {
                        if let Some(w) = &self.window {
                            let _ = w.drag_resize_window(dir);
                        }
                    }
                }
                AppAction::SetCursor(style) => {
                    if self.applied_cursor == Some(style) {
                        continue;
                    }
                    if let Some(w) = &self.window {
                        self.applied_cursor = Some(style);
                        w.window.set_cursor(winit::cursor::Cursor::from(
                            Self::cursor_style_to_winit(style),
                        ));
                    }
                }
                AppAction::ToggleDevTools => {
                    self.toggle_devtools(event_loop);
                }
                AppAction::ToggleInspectMode => {
                    self.toggle_inspect_mode();
                }
                // Desktop keeps the runtime's DOM menu (issue #813): this is
                // only ever emitted under a presentation the desktop shell
                // never asks for.
                AppAction::ShowTextContextMenu => {}
            }
        }
    }

    /// Convert a platform CursorStyle to a winit CursorIcon.
    fn cursor_style_to_winit(style: rinch_platform::CursorStyle) -> winit::cursor::CursorIcon {
        use rinch_platform::CursorStyle as CS;
        use winit::cursor::CursorIcon;
        match style {
            CS::Auto | CS::Default => CursorIcon::Default,
            CS::Pointer => CursorIcon::Pointer,
            CS::Text => CursorIcon::Text,
            CS::Move => CursorIcon::Move,
            CS::NotAllowed => CursorIcon::NotAllowed,
            CS::Crosshair => CursorIcon::Crosshair,
            CS::Grab => CursorIcon::Grab,
            CS::Grabbing => CursorIcon::Grabbing,
            CS::ColResize => CursorIcon::ColResize,
            CS::RowResize => CursorIcon::RowResize,
            CS::NResize => CursorIcon::NResize,
            CS::SResize => CursorIcon::SResize,
            CS::EResize => CursorIcon::EResize,
            CS::WResize => CursorIcon::WResize,
            CS::NeResize => CursorIcon::NeResize,
            CS::NwResize => CursorIcon::NwResize,
            CS::SeResize => CursorIcon::SeResize,
            CS::SwResize => CursorIcon::SwResize,
            CS::EwResize => CursorIcon::EwResize,
            CS::NsResize => CursorIcon::NsResize,
            CS::NeswResize => CursorIcon::NeswResize,
            CS::NwseResize => CursorIcon::NwseResize,
            CS::ZoomIn => CursorIcon::ZoomIn,
            CS::ZoomOut => CursorIcon::ZoomOut,
            CS::Wait => CursorIcon::Wait,
            CS::Progress => CursorIcon::Progress,
            CS::Help => CursorIcon::Help,
            CS::ContextMenu => CursorIcon::ContextMenu,
            CS::Cell => CursorIcon::Cell,
            CS::Copy => CursorIcon::Copy,
            CS::Alias => CursorIcon::Alias,
            CS::NoDrop => CursorIcon::NoDrop,
            CS::AllScroll => CursorIcon::AllScroll,
            CS::None => CursorIcon::Default, // winit doesn't have "none"
        }
    }

    /// The DOM-style key value a winit **logical** key spells — what feeds
    /// `PlatformEvent::KeyDown::logical_key` (and `KeyUp`'s), see the contract
    /// there. winit already speaks the browser's language, so this is nearly a
    /// transcription:
    ///
    /// - `Key::Character` is the layout-produced string, **verbatim**. Case is
    ///   preserved deliberately: `KeyboardEvent.key` in a browser is
    ///   case-*accurate* — measured, Chromium reports `"A"` for Shift+A and
    ///   `"S"` for Ctrl+Shift+S, on the press **and** the release. The old
    ///   lowercased-single-letter form made a release disagree with its own
    ///   press for every capital (a press spelled itself from `text`, which
    ///   kept the capital; a release has no text), which breaks the one thing
    ///   releases exist for — pairing (issue #337). Consumers that want a
    ///   case-insensitive identity fold at the comparison site, as
    ///   `editor_key_binding` does. The spacebar arrives here too, as `" "`,
    ///   the spec's own spelling.
    /// - `Key::Named` is `keyboard_types::NamedKey`, the W3C key-values enum;
    ///   its `Display` **is** the spec string (`"Enter"`, `"ArrowLeft"`,
    ///   `"Shift"`). winit's backends already fold the platform quirks the
    ///   browser folds — X11/Wayland's Super keysym is reported as
    ///   `NamedKey::Meta` "because browsers do" (winit-common's xkb keymap) —
    ///   so no respelling happens here.
    /// - A dead key is `"Dead"`, the value every browser reports for one.
    /// - `Key::Unidentified` is `None`, not the spec's `"Unidentified"`: it
    ///   carries only a native keysym rinch cannot spell, and `None` is what
    ///   lets the consumer fall back to the physical `key` — two different
    ///   unknown keys must not pair with each other by a shared placeholder
    ///   string.
    fn winit_logical_key_str(key: &winit::keyboard::Key) -> Option<String> {
        use winit::keyboard::Key;
        match key {
            Key::Character(s) => Some(s.to_string()),
            Key::Named(n) => Some(n.to_string()),
            Key::Dead(_) => Some("Dead".to_string()),
            Key::Unidentified(_) => None,
        }
    }

    /// The [`PlatformEvent::KeyDown`] a winit press translates to.
    ///
    /// Split out of the `WindowEvent::KeyboardInput` arm so it can be tested at
    /// all: `window_event` needs a live `ActiveEventLoop` and a real window, and
    /// the only other statement in that arm is the menu-shortcut consume.
    ///
    /// What that buys is the **one** pin on this backend's `repeat` answer.
    /// Every consumer fixture supplies its own [`KeyRepeat`], so inverting the
    /// mapping here — desktop refusing every fresh press and activating on
    /// every auto-repeat, i.e. #463 made worse than before it — was invisible
    /// to the entire runtime suite (measured: 602 passed, 0 failed, in the
    /// adversarial review of PR #796). The assertion in
    /// `key_repeat_translation` is trivial by construction; the point is that
    /// there was nothing at all.
    fn key_down_from_winit(
        key_code: winit::keyboard::KeyCode,
        logical_key: &winit::keyboard::Key,
        text: Option<&str>,
        repeat: bool,
        modifiers: Modifiers,
    ) -> PlatformEvent {
        PlatformEvent::KeyDown {
            key: Self::translate_key(key_code),
            // The layout-mapped key value (so Mod+letter follows the keycap, not
            // the physical position) — winit's logical key carries it even when a
            // modifier suppresses `text`.
            logical_key: Self::winit_logical_key_str(logical_key),
            text: text.map(str::to_string),
            modifiers,
            // winit's `repeat` is `true` if and only if the OS made this press
            // by holding the key down, so desktop never has to infer it from a
            // release that may never arrive (#463).
            repeat: if repeat {
                KeyRepeat::Repeat
            } else {
                KeyRepeat::Fresh
            },
        }
    }

    /// Translate a winit KeyCode to a platform KeyCode.
    fn translate_key(key_code: winit::keyboard::KeyCode) -> KeyCode {
        use winit::keyboard::KeyCode as WK;
        match key_code {
            WK::ArrowLeft => KeyCode::ArrowLeft,
            WK::ArrowRight => KeyCode::ArrowRight,
            WK::ArrowUp => KeyCode::ArrowUp,
            WK::ArrowDown => KeyCode::ArrowDown,
            WK::Home => KeyCode::Home,
            WK::End => KeyCode::End,
            WK::PageUp => KeyCode::PageUp,
            WK::PageDown => KeyCode::PageDown,
            WK::Enter | WK::NumpadEnter => KeyCode::Enter,
            WK::Backspace => KeyCode::Backspace,
            WK::Delete => KeyCode::Delete,
            WK::Tab => KeyCode::Tab,
            WK::Escape => KeyCode::Escape,
            WK::Space => KeyCode::Space,
            WK::KeyA => KeyCode::KeyA,
            WK::KeyB => KeyCode::KeyB,
            WK::KeyC => KeyCode::KeyC,
            WK::KeyD => KeyCode::KeyD,
            WK::KeyE => KeyCode::KeyE,
            WK::KeyF => KeyCode::KeyF,
            WK::KeyG => KeyCode::KeyG,
            WK::KeyH => KeyCode::KeyH,
            WK::KeyI => KeyCode::KeyI,
            WK::KeyJ => KeyCode::KeyJ,
            WK::KeyK => KeyCode::KeyK,
            WK::KeyL => KeyCode::KeyL,
            WK::KeyM => KeyCode::KeyM,
            WK::KeyN => KeyCode::KeyN,
            WK::KeyO => KeyCode::KeyO,
            WK::KeyP => KeyCode::KeyP,
            WK::KeyQ => KeyCode::KeyQ,
            WK::KeyR => KeyCode::KeyR,
            WK::KeyS => KeyCode::KeyS,
            WK::KeyT => KeyCode::KeyT,
            WK::KeyU => KeyCode::KeyU,
            WK::KeyV => KeyCode::KeyV,
            WK::KeyW => KeyCode::KeyW,
            WK::KeyX => KeyCode::KeyX,
            WK::KeyY => KeyCode::KeyY,
            WK::KeyZ => KeyCode::KeyZ,
            WK::Digit0 => KeyCode::Digit0,
            WK::Digit1 => KeyCode::Digit1,
            WK::Digit2 => KeyCode::Digit2,
            WK::Digit3 => KeyCode::Digit3,
            WK::Digit4 => KeyCode::Digit4,
            WK::Digit5 => KeyCode::Digit5,
            WK::Digit6 => KeyCode::Digit6,
            WK::Digit7 => KeyCode::Digit7,
            WK::Digit8 => KeyCode::Digit8,
            WK::Digit9 => KeyCode::Digit9,
            WK::F1 => KeyCode::F1,
            WK::F2 => KeyCode::F2,
            WK::F3 => KeyCode::F3,
            WK::F4 => KeyCode::F4,
            WK::F5 => KeyCode::F5,
            WK::F6 => KeyCode::F6,
            WK::F7 => KeyCode::F7,
            WK::F8 => KeyCode::F8,
            WK::F9 => KeyCode::F9,
            WK::F10 => KeyCode::F10,
            WK::F11 => KeyCode::F11,
            WK::F12 => KeyCode::F12,
            WK::Equal => KeyCode::Equal,
            WK::Minus => KeyCode::Minus,
            WK::ShiftLeft => KeyCode::ShiftLeft,
            WK::ShiftRight => KeyCode::ShiftRight,
            WK::ControlLeft => KeyCode::ControlLeft,
            WK::ControlRight => KeyCode::ControlRight,
            WK::AltLeft => KeyCode::AltLeft,
            WK::AltRight => KeyCode::AltRight,
            _ => KeyCode::Other,
        }
    }

    /// Translate winit modifier state to platform Modifiers.
    fn translate_modifiers(&self) -> Modifiers {
        Modifiers {
            shift: self.modifiers.shift_key(),
            ctrl: self.modifiers.control_key(),
            alt: self.modifiers.alt_key(),
            meta: self.modifiers.meta_key(),
        }
    }

    /// Capture a screenshot from whichever renderer is active.
    #[cfg(feature = "debug")]
    fn capture_screenshot_impl(&mut self) -> DebugResult {
        #[cfg(feature = "gpu")]
        if let Some(renderer) = &self.renderer {
            return match renderer.capture_screenshot() {
                Ok((w, h, rgba)) => {
                    let png_bytes = screenshot::encode_png(&rgba, w, h);
                    DebugResult::Bytes {
                        data: base64::engine::general_purpose::STANDARD.encode(&png_bytes),
                    }
                }
                Err(e) => DebugResult::Error {
                    message: format!("Screenshot capture failed: {}", e),
                },
            };
        }
        let scale = self.scale_factor();
        let size = self.logical_size();
        // The frame `paint()` just presented, read back without repainting —
        // a repaint here would have no surface or video frame maps (#364).
        let (pixels, w, h) = self.app.screenshot_pixels(scale, size);
        let png_bytes = screenshot::encode_png(pixels, w, h);
        DebugResult::Bytes {
            data: base64::engine::general_purpose::STANDARD.encode(&png_bytes),
        }
    }

    /// Handle debug commands that require the renderer (e.g., screenshots).
    /// Returns true if the event loop should exit.
    #[cfg(feature = "debug")]
    fn handle_debug_commands_with_renderer(&mut self) -> bool {
        let Some(rx) = self.app.debug_cmd_rx.take() else {
            return false;
        };

        let mut should_exit = false;

        // Process commands as they arrive. After answering command N, wait a
        // short beat for a pipelined follow-up — a fast client sends command
        // N+1 the moment it reads response N, but by then the paint N induced
        // is usually already dispatched, so without this window N+1 would
        // stall behind that full paint (#153). Batching here lets the whole
        // burst ride a single paint. The window is bounded so a spamming
        // client cannot starve painting.
        let batch_start = std::time::Instant::now();
        let mut next = rx.0.try_recv().ok();
        while let Some(cmd) = next {
            let response = match &cmd.kind {
                rinch_debug::DebugCommandKind::Screenshot => {
                    // Screenshot needs the renderer -- handle it here in the shell
                    if let Err(e) = self.paint() {
                        DebugResult::Error {
                            message: format!("Paint failed: {}", e),
                        }
                    } else {
                        self.capture_screenshot_impl()
                    }
                }
                _ => {
                    // All other commands are handled by the platform-agnostic RinchApp
                    let mut actions = Vec::new();
                    let scale = self.scale_factor();
                    let size = self.window_size();
                    // We need to take the kind out of cmd, but cmd.kind is borrowed above.
                    // Re-dispatch through execute_debug_command on the app.
                    let result =
                        self.app
                            .execute_debug_command(cmd.kind.clone(), &mut actions, scale, size);
                    // Process any actions generated
                    for action in actions {
                        match action {
                            AppAction::Exit => {
                                should_exit = true;
                            }
                            AppAction::RequestRedraw => {
                                if let Some(w) = &self.window {
                                    w.request_redraw();
                                }
                            }
                            AppAction::DragWindow => {
                                if let Some(w) = &self.window {
                                    let _ = w.drag_window();
                                }
                            }
                            _ => {}
                        }
                    }
                    result
                }
            };
            let _ = cmd.response_tx.send(response);

            next = if batch_start.elapsed() < std::time::Duration::from_millis(25) {
                rx.0.recv_timeout(std::time::Duration::from_millis(2)).ok()
            } else {
                rx.0.try_recv().ok()
            };
        }

        self.app.debug_cmd_rx = Some(rx);
        should_exit
    }
}

impl Drop for RinchRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Whether the app's window starts hidden: `WindowProps::visible` is `false`
/// (issue #1520). Absent props mean a visible window.
fn starts_hidden(props: Option<&rinch_core::element::WindowProps>) -> bool {
    props.is_some_and(|p| !p.visible)
}

/// The first creation's attributes for a hidden start (#1520): `visible:
/// false` whatever the overlay steps (#1519) chose. That window only sizes the
/// root and is destroyed before anything is presented (X11 and Windows honour
/// `with_visible(false)`; Wayland maps no toplevel until a buffer is attached,
/// and none ever is). The overlay steps' own creation visibility is about the
/// window `show_window` makes later — on macOS a non-activating panel is
/// created *visible*, since a hidden-then-`set_visible` show would make it key —
/// and that re-creation takes the overlay attributes unchanged. A start that
/// is not hidden is left exactly as the overlay steps built it.
fn hidden_start_attributes(attrs: WindowAttributes, hidden: bool) -> WindowAttributes {
    if hidden {
        attrs.with_visible(false)
    } else {
        attrs
    }
}

/// Whether `can_create_surfaces` creates the main window: only when there is
/// none **and** nothing is mounted. A mounted document with no window was
/// hidden on purpose (started hidden, #1520, or sent to the tray), and
/// creating the window here would mount the root a second time.
fn creates_window_on_surfaces(has_window: bool, mounted: bool) -> bool {
    !has_window && !mounted
}

#[cfg(test)]
mod hidden_start_tests {
    use super::*;
    use rinch_core::element::WindowProps;

    /// #1520: `visible: false` starts hidden; the default and absent props
    /// do not.
    #[test]
    fn only_visible_false_starts_hidden() {
        assert!(!starts_hidden(None));
        assert!(!starts_hidden(Some(&WindowProps::default())));
        assert!(starts_hidden(Some(&WindowProps {
            visible: false,
            ..Default::default()
        })));
    }

    /// #1520 with #1519: a hidden start creates its sizing window invisible
    /// even where the overlay steps would create it visible (a macOS
    /// non-activating panel), and leaves every other start as the steps built
    /// it — an ordinary window visible, an X11 overlay created hidden.
    #[test]
    fn a_hidden_start_overrides_the_overlay_visibility_and_nothing_else() {
        use super::overlay_window::{Backend, WindowSteps, apply_to_attributes};
        let panel = WindowProps {
            non_activating: true,
            ..Default::default()
        };
        let mac = WindowSteps::plan(&panel, false, Backend::MacOs);
        let built = || {
            apply_to_attributes(
                WindowAttributes::default(),
                mac,
                Backend::MacOs,
                None,
                None,
                None,
            )
        };
        assert!(built().visible, "the macOS panel itself is created visible");
        assert!(!hidden_start_attributes(built(), true).visible);
        assert!(
            !hidden_start_attributes(built(), true).active,
            "the steps' other attributes stay"
        );
        assert!(hidden_start_attributes(built(), false).visible);
        let x11 = WindowSteps::plan(&panel, false, Backend::X11);
        let x11_attrs = apply_to_attributes(
            WindowAttributes::default(),
            x11,
            Backend::X11,
            None,
            None,
            None,
        );
        assert!(
            !hidden_start_attributes(x11_attrs, false).visible,
            "the X11 overlay is created hidden as before"
        );
        assert!(hidden_start_attributes(WindowAttributes::default(), false).visible);
    }

    /// #1520: after a hidden start the document is mounted and the window
    /// gone, and a later `can_create_surfaces` must not create (and mount)
    /// again. Only the first call, with nothing mounted, creates one.
    #[test]
    fn surfaces_create_the_window_only_before_anything_is_mounted() {
        assert!(creates_window_on_surfaces(false, false), "first start");
        assert!(
            !creates_window_on_surfaces(false, true),
            "hidden start or tray: mounted, no window"
        );
        assert!(!creates_window_on_surfaces(true, true), "already shown");
    }
}

// ── ApplicationHandler impl ──────────────────────────────────────────────────

impl ApplicationHandler for RinchRuntime {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        // Mounted already means the window was hidden on purpose — started
        // hidden (#1520) or sent to the tray — and creating one here would
        // mount the root a second time; `show_window` is what brings it back.
        if creates_window_on_surfaces(self.window.is_some(), self.app.doc.is_some()) {
            self.create_window(event_loop);
        }
    }

    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        // Keep the pointer ahead of whatever the wake carries (an injected
        // press, a debug command that reads the hover state).
        self.flush_pointer_move(event_loop);
        // Track whether any reactive state changes while we drain queued work,
        // so the redraw decision below can key off the actual cause (a signal
        // changed) instead of repainting on every wake.
        rinch_core::clear_signals_changed();

        let mut deferred_ran = false;
        drain_wake_queues(
            self,
            |rt| {
                // Drain main-thread callback queue.
                rinch_core::drain_main_callbacks();

                // Then the work other threads sent an app itself (issue #328):
                // a plain `<input>` paste whose clipboard read just answered.
                // Its wake is one of the callbacks drained above, so this is
                // the turn it asked for. It dirties the field it pastes into,
                // but a field with no layout change leaves
                // `has_pending_layout` false, so ask for the frame here.
                deferred_ran = rt.app.run_deferred_work() > 0;
                if let Some(dt_app) = &mut rt.devtools_app
                    && dt_app.run_deferred_work() > 0
                {
                    if let Some(w) = &rt.devtools_window {
                        w.request_redraw();
                    }
                }
            },
            // Drain queued native events.
            |rt| rt.drain_native_events(event_loop),
        );

        // A cross-thread Signal::send()/update_send() drained above runs its
        // effects on this thread, but the ReRender handler's resolve_and_repaint
        // doesn't always flag that mutation as a paint-worthy change (e.g. the
        // dirty state was already consumed earlier in this batch). If a signal
        // actually changed — or the DOM is still dirty — force a repaint so
        // background-thread updates reliably show up instead of waiting for the
        // next input event. Wakes that change nothing (window controls, debug
        // commands, no-op callbacks) skip the redraw.
        if deferred_ran || rinch_core::signals_changed() || self.app.has_pending_layout() {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }

        // Also resolve DevTools if signals changed
        if self.devtools_window.is_some() {
            self.resolve_devtools();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Route DevTools window events separately
        if Some(window_id) == self.devtools_window_id() {
            self.handle_devtools_window_event(event_loop, event);
            return;
        }

        // Whatever pointer this event names (`set_current_pointer` below) is
        // forgotten when it has been handled, on every return path, so an
        // event that names none (a key, an injected debug `mouse_down`) is
        // never read as the last pen or finger.
        let _forget_pointer = ForgetCurrentPointer;

        // Pointer moves are coalesced (update-path audit F2.2): a high-rate
        // mouse reports several per frame, and each used to run hover, the
        // `data-onmousemove` walk and — during a component drag — a whole
        // layout. Only the newest position is kept; it reaches the app before
        // anything else does (so a press is still judged at the position the
        // pointer last reported) and at the end of the batch, in
        // `about_to_wait`. That is the browser's model: one `pointermove` per
        // frame at most.
        if let WindowEvent::PointerMoved {
            position,
            primary,
            source,
            ..
        } = &event
        {
            // winit reports the pointer in **physical** pixels; every
            // `PlatformEvent` coordinate is logical (#299). This is the
            // conversion for the whole pointer stream: `MouseDown`, `MouseUp`
            // and `MouseWheel` below all read the position back out of
            // `app.cursor_pos`, which the flushed move sets.
            let (lx, ly) = to_logical_point((position.x, position.y), self.scale_factor());
            // A layer surface's move or resize (#1523) takes the pointer, as
            // a compositor move does.
            if self.update_layer_gesture((lx, ly), event_loop) {
                return;
            }
            let id = pointer_id_of(source);
            let down = self.pressed_pointers.is_down(id);
            let pointer = surface_pointer_of(source, *primary, down);
            self.queue_pointer_move(lx as f32, ly as f32, pointer, event_loop);
            return;
        }
        // A mouse entering the window is where it is: a press can follow with
        // no move in between (an input injector, a window mapped under a
        // still pointer), and it must not be judged at a stale position
        // (#1523: a layer surface's drag started from (0, 0)).
        if let WindowEvent::PointerEntered {
            position,
            primary,
            kind: winit::event::PointerKind::Mouse,
            ..
        } = &event
        {
            let (lx, ly) = to_logical_point((position.x, position.y), self.scale_factor());
            let down = self
                .pressed_pointers
                .is_down(SurfacePointer::mouse(false).id);
            let pointer = SurfacePointer {
                primary: *primary,
                ..SurfacePointer::mouse(down)
            };
            self.queue_pointer_move(lx as f32, ly as f32, pointer, event_loop);
            return;
        }
        // A press or release is judged where *its* pointer is: a finger or a
        // pen reports no move before it touches down, so the position the
        // press carries is queued as that pointer's move first (a mouse's
        // press is where its last move already put it).
        if let WindowEvent::PointerButton {
            state,
            position,
            primary,
            button,
            ..
        } = &event
        {
            let pressed = *state == ElementState::Pressed;
            // Any press or release ends a layer move or resize (#1523); a
            // press proves a release was missed.
            if let Some(layer) = self.layer.as_mut() {
                layer.gesture = None;
            }
            let pointer = surface_pointer_of_button(button, *primary, pressed);
            if !matches!(button, winit::event::ButtonSource::Mouse(_)) {
                let (lx, ly) = to_logical_point((position.x, position.y), self.scale_factor());
                self.queue_pointer_move(lx as f32, ly as f32, pointer, event_loop);
            }
            self.flush_pointer_move(event_loop);
            self.pressed_pointers
                .note(pointer.id, button.clone().mouse_button(), pressed);
            crate::render_surface::set_current_pointer(Some(pointer));
        }
        self.flush_pointer_move(event_loop);

        // A layer surface's move reached the compositor (#1523).
        if let WindowEvent::Moved(p) = &event {
            self.layer_moved((p.x, p.y));
        }
        // Keep the published geometry (#1546) current.
        if matches!(
            event,
            WindowEvent::Moved(_)
                | WindowEvent::SurfaceResized(_)
                | WindowEvent::ScaleFactorChanged { .. }
        ) {
            if let Some(w) = self.window.as_ref().map(|w| w.window.clone()) {
                self.note_geometry(&*w);
            }
        }

        let platform_event = match event {
            // A layer surface has no close button: its close is the
            // compositor's, for an output that went away. Show it again
            // rather than quitting (#1523).
            WindowEvent::CloseRequested if self.layer.is_some() => {
                self.layer_surface_closed(event_loop);
                return;
            }
            WindowEvent::CloseRequested => {
                // What the app reads in `on_close_requested` (#1884).
                self.settle_maximize_wait();
                PlatformEvent::CloseRequested
            }
            WindowEvent::SurfaceResized(size) => {
                // Also resize the renderer — it works in physical pixels.
                #[cfg(feature = "gpu")]
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width.max(1), size.height.max(1));
                }
                // `Resized` re-lays out the document, so it carries the
                // logical size (see `to_logical`).
                let logical = to_logical((size.width, size.height), self.scale_factor());
                PlatformEvent::Resized {
                    width: logical.0,
                    height: logical.1,
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // The window moved to a display with a different scale (or the
                // user changed it). `handle_event` pushes the value into
                // Stylo's `device_pixel_ratio` (issue #211); the renderer
                // needs no reconfigure here — winit follows this event with a
                // `SurfaceResized` carrying the new physical size, handled
                // above.
                PlatformEvent::ScaleFactorChanged(scale_factor)
            }
            WindowEvent::RedrawRequested => {
                // Drain queued native events (debug commands, injected input)
                // before painting so a command that arrived while the loop was
                // busy isn't serialized behind a full paint (#153). Pre-paint
                // is safe: the paint preamble re-resolves layout when
                // has_pending_layout(), so the paint sees the drained state.
                self.drain_native_events(event_loop);
                // Paint directly -- this is shell-level, not delegated
                if let Err(e) = self.paint() {
                    eprintln!("Paint error: {}", e);
                }
                // winit coalesces wake_up() calls arbitrarily, so a wake that
                // landed mid-paint may already have been consumed. If events
                // queued while we painted, self-wake so they're delivered
                // immediately instead of waiting for the next external event
                // (#153).
                if !NATIVE_EVENT_QUEUE.lock().unwrap().is_empty() {
                    if let Some(proxy) = GLOBAL_PROXY.get() {
                        proxy.wake_up();
                    }
                }
                return;
            }
            // Coalesced above; never reaches this match.
            WindowEvent::PointerMoved { .. } => return,
            WindowEvent::PointerButton {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                // A finger is the left button, and so is a pen's tip (its
                // barrel button the right): winit's own mapping.
                let platform_button = match button.mouse_button() {
                    Some(MouseButton::Left) => PlatformMouseButton::Left,
                    Some(MouseButton::Right) => PlatformMouseButton::Right,
                    Some(MouseButton::Middle) => PlatformMouseButton::Middle,
                    _ => return,
                };
                // For click handling, we need the cursor position
                let (x, y) = self.app.cursor_pos.unwrap_or((0.0, 0.0));

                // Inspect mode: click selects node and exits inspect mode
                if platform_button == PlatformMouseButton::Left {
                    if let Some(store) = &self.devtools_store {
                        if store.inspect_mode.get() {
                            if let Some(doc) = &self.app.doc {
                                let d = doc.borrow();
                                let hit = crate::app::hit_testing::hit_test(&d.tree, x, y);
                                store.selected_node_id.set(hit);
                            }
                            store
                                .active_tab
                                .set(super::devtools_store::DevToolsTab::Elements);
                            store.inspect_mode.set(false);
                            store.hovered_node_id.set(None);
                            self.app.inspect_highlight = None;
                            self.app.request_repaint();
                            if let Some(w) = &self.window {
                                w.request_redraw();
                            }
                            return;
                        }
                    }
                }

                // Set current window ID so window control functions work
                if let Some(w) = &self.window {
                    crate::windows::set_current_window_id(Some(w.window.id()));
                }
                let size = self.window_size();
                let scale = self.scale_factor();
                let actions = self.app.handle_event(
                    PlatformEvent::MouseDown {
                        x,
                        y,
                        button: platform_button,
                    },
                    size,
                    scale,
                );
                crate::windows::set_current_window_id(None);
                self.process_actions(actions, event_loop);
                return;
            }
            WindowEvent::PointerButton {
                state: ElementState::Released,
                button,
                ..
            } => {
                // A finger is the left button, and so is a pen's tip (its
                // barrel button the right): winit's own mapping.
                let platform_button = match button.mouse_button() {
                    Some(MouseButton::Left) => PlatformMouseButton::Left,
                    Some(MouseButton::Right) => PlatformMouseButton::Right,
                    Some(MouseButton::Middle) => PlatformMouseButton::Middle,
                    _ => return,
                };
                let (x, y) = self.app.cursor_pos.unwrap_or((0.0, 0.0));
                PlatformEvent::MouseUp {
                    x,
                    y,
                    button: platform_button,
                }
            }
            // The mouse left the window: no tooltip for what it was over
            // (#1646).
            WindowEvent::PointerLeft {
                kind: winit::event::PointerKind::Mouse,
                ..
            } => {
                self.app.hide_title_tooltip();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                return;
            }
            // A touch the system stopped tracking without a release (the
            // window lost focus, a palm was rejected): the press it held is
            // cancelled.
            WindowEvent::PointerLeft {
                kind: winit::event::PointerKind::Touch(finger_id),
                ..
            } => {
                let id = TOUCH_POINTER_BASE + finger_id.into_raw() as u64;
                if !self.pressed_pointers.lift(id) {
                    return;
                }
                let pointer = SurfacePointer {
                    id,
                    kind: SurfacePointerKind::Touch,
                    pressure: 0.0,
                    primary: false,
                };
                self.app.cancel_surface_pointer(pointer);
                return;
            }
            // A trackpad pinch (macOS, iOS, Wayland): a `Pinch` for a
            // surface with pointer events under the cursor.
            // A pinch no surface takes is an `onwheel` with Ctrl held, as a
            // browser reports it.
            WindowEvent::PinchGesture { delta, .. } => {
                if delta.is_finite()
                    && !self.app.surface_pinch_gesture(delta as f32)
                    && self.app.pinch_as_wheel(delta as f32)
                    && let Some(w) = &self.window
                {
                    w.window.request_redraw();
                }
                return;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    // Lines, not pixels: the `* 40.0` is already a logical-px
                    // convention, so there is nothing to convert.
                    winit::event::MouseScrollDelta::LineDelta(x, y) => {
                        (x as f64 * 40.0, y as f64 * 40.0)
                    }
                    // A pixel delta is a *physical* distance, and it is compared
                    // against logical scroll extents — so it divides like a
                    // position does, or a HiDPI trackpad scrolls `scale` times
                    // too far (#299).
                    winit::event::MouseScrollDelta::PixelDelta(pos) => {
                        to_logical_point((pos.x, pos.y), self.scale_factor())
                    }
                    // `MouseScrollDelta` is non-exhaustive (winit 0.31.0-beta.3):
                    // a kind of delta this shell does not know scrolls nothing.
                    _ => return,
                };
                // Already logical: set from the converted `PointerMoved` above.
                let (cx, cy) = self.app.cursor_pos.unwrap_or((0.0, 0.0));
                PlatformEvent::MouseWheel {
                    x: cx,
                    y: cy,
                    delta_x: dx,
                    delta_y: dy,
                }
            }
            WindowEvent::Focused(focused) => {
                // A window that lost focus may never be sent the release of a
                // button held across the blur (it goes to the window that took
                // focus): forget every press, or a hover on a surface with
                // pointer events would go on reporting a pressure of 0.5 until
                // that button is pressed and released here again. The app's
                // own heal ends the press gestures and surface captures.
                if !focused {
                    self.pressed_pointers.clear();
                }
                // Tell the AT whether this window has OS focus.
                #[cfg(feature = "a11y")]
                if let Some(bridge) = self.a11y.as_mut() {
                    bridge.set_window_focused(focused);
                }
                // …and the app, which notifies the focused widget without
                // releasing its claim (issue #147, decision 1). This used to
                // `return` here, so window blur was invisible to the document:
                // a custom widget's caret kept blinking in an unfocused window
                // and the OS IME stayed armed on a window without the keyboard.
                PlatformEvent::WindowFocus(focused)
            }
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.modifiers = new_modifiers.state();
                let mods = self.translate_modifiers();
                PlatformEvent::ModifiersChanged(mods)
            }
            WindowEvent::KeyboardInput {
                event:
                    winit::event::KeyEvent {
                        physical_key: winit::keyboard::PhysicalKey::Code(key_code),
                        state: ElementState::Pressed,
                        logical_key: ref win_logical,
                        key_without_modifiers: ref win_kwm,
                        ref text,
                        repeat,
                        ..
                    },
                ..
            } => {
                // Check menu shortcuts first — if matched, consume the event.
                // A chord with no Ctrl/Cmd/Alt leaves its key to a focused
                // text field (#1169). `win_kwm` is the layout's character for
                // this physical key, ignoring Shift — issue #1170's letter
                // chords match it before falling back to the physical key.
                let mods = self.translate_modifiers();
                if self.app.try_menu_shortcut(mods, key_code, win_kwm) {
                    // Shortcut matched and callback dispatched; request redraw
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }

                Self::key_down_from_winit(key_code, win_logical, text.as_deref(), repeat, mods)
            }
            WindowEvent::KeyboardInput {
                event:
                    winit::event::KeyEvent {
                        physical_key: winit::keyboard::PhysicalKey::Code(key_code),
                        state: ElementState::Released,
                        logical_key: ref win_logical,
                        ..
                    },
                ..
            } => {
                let mods = self.translate_modifiers();
                let platform_key = Self::translate_key(key_code);
                PlatformEvent::KeyUp {
                    key: platform_key,
                    // winit's `KeyEvent` is one struct for both states and fills
                    // this on each; only `text` is press-gated. Forwarding it
                    // is what lets a release be spelled by the same rule as its
                    // press (issue #337) — the shell was simply dropping it.
                    logical_key: Self::winit_logical_key_str(win_logical),
                    modifiers: mods,
                }
            }
            WindowEvent::Ime(ime) => {
                // Translate winit's IME composition events into the portable
                // `ImeEvent` contract. `handle_event` then routes them through
                // the focus arbiter, exactly like keyboard input — so the
                // editor, `<input>`, etc. all consume IME the same way.
                let ime_event = match ime {
                    winit::event::Ime::Enabled => ImeEvent::Enabled,
                    winit::event::Ime::Preedit(text, cursor) => ImeEvent::Preedit { text, cursor },
                    winit::event::Ime::Commit(text) => ImeEvent::Commit(text),
                    winit::event::Ime::DeleteSurrounding {
                        before_bytes,
                        after_bytes,
                    } => ImeEvent::DeleteSurrounding {
                        // winit reports UTF-8 byte counts; the target converts to
                        // its char-based model. Exact byte→char conversion is only
                        // needed once we advertise the surrounding-text capability
                        // (not enabled yet), so this passes the counts through.
                        before: before_bytes,
                        after: after_bytes,
                    },
                    winit::event::Ime::Disabled => ImeEvent::Disabled,
                };
                PlatformEvent::Ime(ime_event)
            }
            // An OS file drag (#1962): winit's data transfer events, through
            // `OsFileDrag`, become the path-carrying `File*` events. A
            // file-drag position is a pointer position like any other, and is
            // hit-tested against the layout tree (#299).
            WindowEvent::DragEntered { id, position } => {
                use winit::data_transfer::TypeHint;
                let has_uri_list = event_loop
                    .data_transfer(id)
                    .is_ok_and(|transfer| transfer.has_type(&TypeHint::UriList));
                let position = position.map(|p| to_logical_point((p.x, p.y), self.scale_factor()));
                let steps = self.os_file_drag.entered(id, position, has_uri_list);
                self.run_file_drag_steps(steps, event_loop);
                return;
            }
            WindowEvent::DragPosition { id, position, .. } => {
                let position = to_logical_point((position.x, position.y), self.scale_factor());
                let steps = self.os_file_drag.moved(id, position);
                self.run_file_drag_steps(steps, event_loop);
                return;
            }
            WindowEvent::DataTransferReceived { id, serial, value } => {
                let uris = super::os_file_drag::uris_from(value.try_as_uris());
                let steps = self.os_file_drag.received(id, serial, uris);
                self.run_file_drag_steps(steps, event_loop);
                return;
            }
            WindowEvent::DragDropped { id, .. } => {
                let steps = self.os_file_drag.dropped(id, std::time::Instant::now());
                self.run_file_drag_steps(steps, event_loop);
                return;
            }
            WindowEvent::DragLeft { id } => {
                let steps = self.os_file_drag.left(id);
                self.run_file_drag_steps(steps, event_loop);
                return;
            }
            _ => return,
        };
        self.dispatch_main_event(platform_event, event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        // The batch's last pointer move, before anything is laid out.
        self.flush_pointer_move(event_loop);
        self.about_to_wait_inner(event_loop);
    }
}

impl RinchRuntime {
    /// Carry out what [`super::os_file_drag::OsFileDrag`] answered for a drag
    /// event (accept the drag, ask for its URI list, log), then hand the app
    /// the events it produced, in order.
    fn run_file_drag_steps(
        &mut self,
        steps: Vec<super::os_file_drag::DragStep>,
        event_loop: &dyn ActiveEventLoop,
    ) {
        let mut host = WinitDragHost {
            event_loop,
            events: Vec::new(),
        };
        super::os_file_drag::run_steps(&mut self.os_file_drag, steps, &mut host);
        for event in host.events {
            self.dispatch_main_event(event, event_loop);
        }
    }

    /// Hand the app the pointer move [`Self::window_event`] held back, if any.
    fn flush_pointer_move(&mut self, event_loop: &dyn ActiveEventLoop) {
        if let Some((x, y, pointer)) = self.pending_pointer_move.take() {
            crate::render_surface::set_current_pointer(Some(pointer));
            self.dispatch_main_event(PlatformEvent::MouseMove { x, y }, event_loop);
            crate::render_surface::set_current_pointer(None);
        }
    }

    /// Hold back a move of `pointer` to `(x, y)` (logical px) until the
    /// batch ends or another event comes, replacing a held move of the same
    /// pointer: one move per pointer per frame. A held move of **another**
    /// pointer (the other finger of a pinch) is handed over first, so no
    /// pointer's last position is lost.
    fn queue_pointer_move(
        &mut self,
        x: f32,
        y: f32,
        pointer: SurfacePointer,
        event_loop: &dyn ActiveEventLoop,
    ) {
        if self
            .pending_pointer_move
            .is_some_and(|(_, _, held)| held.id != pointer.id)
        {
            self.flush_pointer_move(event_loop);
        }
        self.pending_pointer_move = Some((x, y, pointer));
    }

    /// Dispatch one main-window event to the app and act on what it asks for.
    fn dispatch_main_event(
        &mut self,
        platform_event: PlatformEvent,
        event_loop: &dyn ActiveEventLoop,
    ) {
        // Inspect mode: intercept mouse events for hit-testing
        if let Some(store) = &self.devtools_store {
            if store.inspect_mode.get() {
                if let PlatformEvent::MouseMove { x, y } = &platform_event {
                    // Hit-test the main app's document
                    self.app.cursor_pos = Some((*x, *y));
                    if let Some(doc) = &self.app.doc {
                        let d = doc.borrow();
                        let hit = crate::app::hit_testing::hit_test(&d.tree, *x, *y);
                        store.hovered_node_id.set(hit);
                    }
                }
            }
        }

        let size = self.window_size();
        let scale = self.scale_factor();
        let actions = self.app.handle_event(platform_event, size, scale);
        self.process_actions(actions, event_loop);
    }

    fn about_to_wait_inner(&mut self, event_loop: &dyn ActiveEventLoop) {
        let size = self.window_size();
        let scale = self.scale_factor();
        let actions = self
            .app
            .handle_event(PlatformEvent::AboutToWait, size, scale);
        self.process_actions(actions, event_loop);

        // Notify DevTools when main app DOM changed, then resolve its layout
        if self.devtools_window.is_some() {
            // Bump version counter so DevTools effects re-read the document.
            // The version only changes when the main app actually resolved
            // dirty nodes (scene_dirty is set in resolve_and_repaint).
            if self.app.scene_dirty {
                if let Some(store) = &self.devtools_store {
                    store.bump_version();
                }
            }

            self.resolve_devtools();

            // If hovered node changed in DevTools, request main window repaint
            // for inspect highlight
            if let Some(store) = &self.devtools_store {
                let new_hovered = store.hovered_node_id.get();
                if new_hovered != self.devtools_prev_hovered {
                    self.devtools_prev_hovered = new_hovered;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
        }

        // Reconcile the window's IME state with the focus arbiter (enable on a
        // focused text target, follow the caret, disable on blur).
        self.sync_ime();

        // Push the focused editor's accessibility tree to the AT when it changed.
        #[cfg(feature = "a11y")]
        self.sync_a11y();

        // Drive the focused editor's caret blink, and wake for a resting
        // pointer's `title` tooltip (#1646) and a file drop's deadline (#1962).
        // These are the only timed wakes (`WaitUntil`); with none it returns
        // the loop to `Wait` so the app stays idle.
        #[cfg(feature = "desktop")]
        self.tick_caret_blink(event_loop);
    }
}

impl RinchRuntime {
    /// Blink the focused editor's caret by arming a `WaitUntil` wake for the next
    /// phase toggle (see [`crate::editor::caret_blink_tick`]), for a pending
    /// `title` tooltip's deadline, or for a file drop's (ending the drops past
    /// theirs), whichever is sooner. Owns the event loop's control flow:
    /// `WaitUntil` while any is due, `Wait` otherwise.
    ///
    /// *Which* editor blinks is [`RinchApp::blinking_editor_id`]'s decision, not
    /// this function's — it excludes a blurred window, so a backgrounded app
    /// stops arming the only timed wake the loop has (issue #316).
    #[cfg(feature = "desktop")]
    fn tick_caret_blink(&mut self, event_loop: &dyn ActiveEventLoop) {
        let focused = self.app.blinking_editor_id();
        let blink_at = match crate::editor::caret_blink_tick(self.app.doc_key(), focused) {
            Some(blink) => {
                if blink.redraw
                    && let Some(w) = &self.window
                {
                    w.request_redraw();
                }
                Some(std::time::Instant::now() + blink.next)
            }
            None => None,
        };
        // A pointer resting over a `title` shows its tooltip at its deadline
        // (#1646): another timed wake.
        let wake = match (blink_at, self.app.title_tooltip_deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        // A file drop still waiting for its paths gives up at its deadline
        // (#1962): the third.
        let expired = self.os_file_drag.expire(std::time::Instant::now());
        if !expired.is_empty() {
            self.run_file_drag_steps(expired, event_loop);
        }
        let wake = match (wake, self.os_file_drag.next_deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        // A window created maximized is read once more when the maximize
        // wait's grace ends (#1884): the fourth. Settled, the wait has no
        // deadline, so it arms nothing after.
        if self
            .maximize_wait
            .deadline()
            .is_some_and(|d| std::time::Instant::now() >= d)
        {
            self.settle_maximize_wait();
        }
        let wake = match (wake, self.maximize_wait.deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        match wake {
            Some(at) => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    /// Push the focus arbiter's desired IME state to the window. Enables IME with
    /// a cursor area when a text target is focused, disables it otherwise, and
    /// moves the candidate-box rect as the caret moves. Diffs against the
    /// last-applied state so it only issues a `request_ime_update` on a real
    /// change. This is the single place the winit IME surface is touched; every
    /// text target (editor, `<input>`, …) feeds it through [`RinchApp::ime_state`].
    fn sync_ime(&mut self) {
        use winit::window::{ImeCapabilities, ImeEnableRequest, ImeRequest, ImeRequestData};

        let Some(w) = &self.window else { return };
        let desired = self.app.ime_state();

        let to_area = |a: rinch_core::ElementBounds| {
            (
                winit::dpi::LogicalPosition::new(a.x as f64, a.y as f64),
                winit::dpi::LogicalSize::new(a.width.max(1.0) as f64, a.height.max(1.0) as f64),
            )
        };

        match self.ime_applied.change_to(desired) {
            Some(crate::app::ImeChange::Enable { cursor_area, .. }) => {
                let area = cursor_area.unwrap_or(rinch_core::ElementBounds {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 16.0,
                });
                let (pos, size) = to_area(area);
                let caps = ImeCapabilities::new().with_cursor_area();
                let data = ImeRequestData::default().with_cursor_area(pos.into(), size.into());
                if let Some(req) = ImeEnableRequest::new(caps, data) {
                    let _ = w.window.request_ime_update(ImeRequest::Enable(req));
                }
            }
            Some(crate::app::ImeChange::Update { cursor_area, .. }) => {
                let (pos, size) = to_area(cursor_area);
                let data = ImeRequestData::default().with_cursor_area(pos.into(), size.into());
                let _ = w.window.request_ime_update(ImeRequest::Update(data));
            }
            Some(crate::app::ImeChange::Disable) => {
                let _ = w.window.request_ime_update(ImeRequest::Disable);
            }
            None => {}
        }
    }

    /// Push the focused editor's accessibility tree to the AT when its document or
    /// selection changed since the last push (the AT consumes a full snapshot). The
    /// per-platform bridge is created lazily on first editor focus; on Linux it is
    /// the AT-SPI adapter, elsewhere a no-op. The whole method compiles to nothing
    /// without the `a11y` feature.
    #[cfg(feature = "a11y")]
    fn sync_a11y(&mut self) {
        let Some(handle) = self
            .app
            .focused_editor_id()
            .and_then(|id| crate::editor::editor_for_doc(self.app.doc_key(), id))
        else {
            return;
        };
        let state = handle.state();
        let unchanged = matches!(
            &self.a11y_last,
            Some((doc, sel)) if doc.same_ref(&state.doc) && *sel == state.selection
        );
        if unchanged {
            return;
        }
        let bridge = self
            .a11y
            .get_or_insert_with(crate::editor::a11y::AccesskitBridge::new);
        bridge.update(&state);
        self.a11y_last = Some((state.doc.clone(), state.selection.clone()));
    }

    /// Apply an AT-requested action to the focused editor. A screen reader moving
    /// the caret/selection arrives as `SetTextSelection`; `Focus` is advisory (the
    /// focus arbiter owns focus). Runs on the main thread (from the native-event
    /// queue) after the adapter thread forwarded the request.
    #[cfg(feature = "a11y")]
    fn apply_a11y_action(&mut self, req: accesskit::ActionRequest) {
        use accesskit::{Action, ActionData};
        let Some(handle) = self
            .app
            .focused_editor_id()
            .and_then(|id| crate::editor::editor_for_doc(self.app.doc_key(), id))
        else {
            return;
        };
        if req.action == Action::SetTextSelection
            && let Some(ActionData::SetTextSelection(ak_sel)) = &req.data
            && let Some(sel) =
                rinch_editor_core::a11y::accesskit_selection_to_model(&handle.doc(), ak_sel)
        {
            handle.set_selection(sel);
            send_native_event(RinchNativeEvent::ReRender);
        }
    }

    /// Drain queued native events (debug commands, window controls, injected
    /// platform events) and dispatch each through [`Self::handle_native_event`].
    ///
    /// Called from `proxy_wake_up` and from the top of the `RedrawRequested`
    /// arm — the latter so a command that arrived while the loop was busy is
    /// processed *before* the paint instead of stalling a full paint behind it
    /// (#153). Guarded against re-entry: `DebugCommandKind::Screenshot` calls
    /// [`Self::paint`] from inside the drain, so a nested drain must no-op.
    fn drain_native_events(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.draining_native_events {
            return;
        }
        self.draining_native_events = true;
        let events = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        for event in events {
            self.handle_native_event(event, event_loop);
        }
        self.draining_native_events = false;
    }

    /// Handle a single native event from the queue.
    fn handle_native_event(&mut self, event: RinchNativeEvent, event_loop: &dyn ActiveEventLoop) {
        let platform_event = match event {
            RinchNativeEvent::ReRender => PlatformEvent::UserEvent(UserEvent::ReRender),
            #[cfg(feature = "debug")]
            RinchNativeEvent::DebugCommand => {
                // Debug commands may need the renderer (screenshots), so
                // we handle them here in the shell instead of delegating.
                if self.handle_debug_commands_with_renderer() {
                    event_loop.exit();
                }
                return;
            }
            RinchNativeEvent::MinimizeWindow => PlatformEvent::UserEvent(UserEvent::MinimizeWindow),
            RinchNativeEvent::ToggleMaximizeWindow => {
                PlatformEvent::UserEvent(UserEvent::ToggleMaximizeWindow)
            }
            RinchNativeEvent::CloseWindowControl => {
                PlatformEvent::UserEvent(UserEvent::CloseWindow)
            }
            RinchNativeEvent::ShowWindow => PlatformEvent::UserEvent(UserEvent::ShowWindow),
            RinchNativeEvent::HideWindow => PlatformEvent::UserEvent(UserEvent::HideWindow),
            RinchNativeEvent::SetClickThrough(click_through) => {
                // The shell's alone: no app state depends on it.
                let apply = self.note_click_through(click_through);
                if let Some(w) = &self.window {
                    overlay_window::set_click_through(w.raw(), apply);
                }
                return;
            }
            RinchNativeEvent::SetWindowPosition(x, y) => {
                let position = Position::Logical(LogicalPosition::new(x, y));
                // A layer surface (#1523) is placed by its margins, on its
                // output; hidden, it comes back there.
                // What the window got, once it has been created (a toplevel
                // on GNOME); before that, what it will ask for.
                let layer_window = match super::layer_shell::published_surface_kind() {
                    Some(kind) => kind == WindowSurfaceKind::LayerShell,
                    None => self.app.window_props.as_ref().is_some_and(|p| {
                        super::layer_shell::uses_layer_shell(p, Backend::of(event_loop))
                    }),
                };
                match &self.window {
                    Some(_) if self.layer.is_some() => self.place_layer((x, y)),
                    Some(w) => w.raw().set_outer_position(position),
                    None if layer_window => {
                        self.saved_geometry.position = Some(position);
                        window_geometry::publish_hidden(Some((x, y)), None);
                    }
                    // Wayland: a toplevel has no position to set or report.
                    None if Backend::of(event_loop) == Backend::Wayland => {}
                    None => {
                        self.saved_geometry.position = Some(position);
                        window_geometry::publish_hidden(Some((x, y)), None);
                    }
                }
                return;
            }
            RinchNativeEvent::SetWindowSize(width, height) => {
                let size = Size::Logical(LogicalSize::new(width, height));
                match &self.window {
                    Some(w) => {
                        let id = w.raw().id();
                        if let Some(physical) = w.raw().request_surface_size(size) {
                            // Resized at once: winit sends no
                            // `SurfaceResized` for it, so hand one to the
                            // window's own path (renderer, layout, publish).
                            self.window_event(
                                event_loop,
                                id,
                                WindowEvent::SurfaceResized(physical),
                            );
                        }
                    }
                    None => {
                        self.saved_geometry.size = Some(size);
                        window_geometry::publish_hidden(None, Some((width, height)));
                    }
                }
                return;
            }
            RinchNativeEvent::ApplyWindowTitle => {
                self.take_stored_title();
                return;
            }
            RinchNativeEvent::SetWindowOutput(output) => {
                self.output_override = Some(output);
                // A layer surface cannot change output: make it again there,
                // at the same position and size.
                if self.layer.is_some() {
                    self.hide_window();
                    self.show_window(event_loop);
                }
                return;
            }
            // From the app's own press handler; refused once the press is
            // over (a timer, or the release dispatched before this event).
            RinchNativeEvent::StartWindowDrag => {
                if self.pressed_pointers.primary_down() {
                    self.process_actions(vec![AppAction::DragWindow], event_loop);
                } else {
                    tracing::debug!("start_window_drag refused: no button is down");
                }
                return;
            }
            RinchNativeEvent::StartWindowResize(dir) => {
                if self.pressed_pointers.primary_down() {
                    self.process_actions(vec![AppAction::DragResizeWindow(dir)], event_loop);
                } else {
                    tracing::debug!("start_window_resize refused: no button is down");
                }
                return;
            }
            RinchNativeEvent::RetryLayerOutput => {
                if self.layer_awaiting_output && self.window.is_none() {
                    if event_loop.available_monitors().next().is_some() {
                        self.layer_awaiting_output = false;
                        self.show_window(event_loop);
                    } else {
                        arm_layer_output_retry();
                    }
                }
                return;
            }
            RinchNativeEvent::InjectedPlatformEvent(pe) => pe,
            #[cfg(feature = "a11y")]
            RinchNativeEvent::A11yAction(req) => {
                self.apply_a11y_action(req);
                return;
            }
        };
        let size = self.window_size();
        let scale = self.scale_factor();
        let actions = self.app.handle_event(platform_event, size, scale);
        self.process_actions(actions, event_loop);
    }

    /// Handle events for the DevTools window.
    fn handle_devtools_window_event(
        &mut self,
        _event_loop: &dyn ActiveEventLoop,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.close_devtools();
            }
            WindowEvent::SurfaceResized(size) => {
                #[cfg(feature = "gpu")]
                if let Some(renderer) = &mut self.devtools_renderer {
                    renderer.resize(size.width.max(1), size.height.max(1));
                }
                let dt_scale = self.devtools_scale_factor();
                if let Some(dt_app) = &mut self.devtools_app {
                    let logical = to_logical((size.width, size.height), dt_scale);
                    dt_app.resize_layout(logical.0, logical.1);
                }
                if let Some(w) = &self.devtools_window {
                    w.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // The DevTools window is its own document — push the new
                // display scale into its Stylo device too (issue #211).
                let size = self.devtools_size();
                let dt_scale = self.devtools_scale_factor();
                if let Some(dt_app) = &mut self.devtools_app {
                    let _ = dt_app.handle_event(
                        PlatformEvent::ScaleFactorChanged(scale_factor),
                        size,
                        dt_scale,
                    );
                }
                if let Some(w) = &self.devtools_window {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.paint_devtools() {
                    eprintln!("DevTools paint error: {}", e);
                }
            }
            WindowEvent::PointerMoved { position, .. } => {
                let size = self.devtools_size();
                let scale = self.devtools_scale_factor();
                // Same physical→logical conversion as the main window, against
                // the DevTools window's own scale factor (#299).
                let (lx, ly) = to_logical_point((position.x, position.y), scale);
                if let Some(dt_app) = &mut self.devtools_app {
                    let actions = dt_app.handle_event(
                        PlatformEvent::MouseMove {
                            x: lx as f32,
                            y: ly as f32,
                        },
                        size,
                        scale,
                    );
                    for action in actions {
                        if let AppAction::RequestRedraw = action {
                            if let Some(w) = &self.devtools_window {
                                w.request_redraw();
                            }
                        }
                    }
                }
            }
            WindowEvent::PointerButton {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                let platform_button = match button {
                    winit::event::ButtonSource::Mouse(MouseButton::Left)
                    | winit::event::ButtonSource::Touch { .. } => PlatformMouseButton::Left,
                    winit::event::ButtonSource::Mouse(MouseButton::Right) => {
                        PlatformMouseButton::Right
                    }
                    winit::event::ButtonSource::Mouse(MouseButton::Middle) => {
                        PlatformMouseButton::Middle
                    }
                    _ => return,
                };
                let size = self.devtools_size();
                let scale = self.devtools_scale_factor();
                if let Some(dt_app) = &mut self.devtools_app {
                    let (x, y) = dt_app.cursor_pos.unwrap_or((0.0, 0.0));
                    let actions = dt_app.handle_event(
                        PlatformEvent::MouseDown {
                            x,
                            y,
                            button: platform_button,
                        },
                        size,
                        scale,
                    );
                    for action in actions {
                        if let AppAction::RequestRedraw = action {
                            if let Some(w) = &self.devtools_window {
                                w.request_redraw();
                            }
                        }
                    }
                }
            }
            WindowEvent::PointerButton {
                state: ElementState::Released,
                button,
                ..
            } => {
                let platform_button = match button {
                    winit::event::ButtonSource::Mouse(MouseButton::Left)
                    | winit::event::ButtonSource::Touch { .. } => PlatformMouseButton::Left,
                    winit::event::ButtonSource::Mouse(MouseButton::Right) => {
                        PlatformMouseButton::Right
                    }
                    winit::event::ButtonSource::Mouse(MouseButton::Middle) => {
                        PlatformMouseButton::Middle
                    }
                    _ => return,
                };
                let size = self.devtools_size();
                let scale = self.devtools_scale_factor();
                if let Some(dt_app) = &mut self.devtools_app {
                    let (x, y) = dt_app.cursor_pos.unwrap_or((0.0, 0.0));
                    let actions = dt_app.handle_event(
                        PlatformEvent::MouseUp {
                            x,
                            y,
                            button: platform_button,
                        },
                        size,
                        scale,
                    );
                    for action in actions {
                        if let AppAction::RequestRedraw = action {
                            if let Some(w) = &self.devtools_window {
                                w.request_redraw();
                            }
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let size = self.devtools_size();
                let scale = self.devtools_scale_factor();
                let (dx, dy) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => {
                        (x as f64 * 40.0, y as f64 * 40.0)
                    }
                    // Physical, like the main window's (#299).
                    winit::event::MouseScrollDelta::PixelDelta(pos) => {
                        to_logical_point((pos.x, pos.y), scale)
                    }
                    _ => return,
                };
                if let Some(dt_app) = &mut self.devtools_app {
                    let (cx, cy) = dt_app.cursor_pos.unwrap_or((0.0, 0.0));
                    let actions = dt_app.handle_event(
                        PlatformEvent::MouseWheel {
                            x: cx,
                            y: cy,
                            delta_x: dx,
                            delta_y: dy,
                        },
                        size,
                        scale,
                    );
                    for action in actions {
                        if let AppAction::RequestRedraw = action {
                            if let Some(w) = &self.devtools_window {
                                w.request_redraw();
                            }
                        }
                    }
                }
            }
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.devtools_modifiers = new_modifiers.state();
            }
            WindowEvent::KeyboardInput {
                event:
                    winit::event::KeyEvent {
                        physical_key:
                            winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::F12),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                // F12 in DevTools window also toggles
                self.close_devtools();
            }
            _ => {}
        }
    }
}

// ── DevTools data extraction helpers ─────────────────────────────────────────

// ── Compositor letterbox backdrop ────────────────────────────────

/// Keep only the radii of corners where the viewport's box actually meets the
/// corner of what clips it, zeroing the rest.
///
/// `RinchApp::viewport_rect_with_radius` reports the **clipping ancestor's**
/// radii, but the viewport is usually only part of that ancestor — in the UI
/// Zoo demo, a video filling the top of a card whose lower half is the controls
/// bar. Applying those radii to the viewport's own box then rounds an edge that
/// runs through the middle of the card, carving a see-through notch out of a
/// hole nothing else fills: exactly the defect this is meant to close. A corner
/// is only a real corner where both of the viewport's edges coincide with the
/// clip's.
///
/// With no clip rect there is no ancestor box to compare against, so the radii
/// are taken as given — the viewport is then the rounded box itself.
///
/// All rects are `(x, y, w, h)` in physical pixels; radii are `[tl, tr, br, bl]`.
#[cfg(any(feature = "gpu", test))]
fn radii_at_shared_corners(
    box_rect: (f32, f32, f32, f32),
    clip_rect: Option<(f32, f32, f32, f32)>,
    radii: [f32; 4],
) -> [f32; 4] {
    let Some((cx, cy, cw, ch)) = clip_rect else {
        return radii;
    };
    // Both rects are truncated from the same layout values, so allow a pixel
    // of slop rather than demanding bit-exact edges.
    let near = |a: f32, b: f32| (a - b).abs() <= 1.0;
    let (bx, by, bw, bh) = box_rect;
    let (left, top) = (near(bx, cx), near(by, cy));
    let (right, bottom) = (near(bx + bw, cx + cw), near(by + bh, cy + ch));
    [
        if left && top { radii[0] } else { 0.0 },
        if right && top { radii[1] } else { 0.0 },
        if right && bottom { radii[2] } else { 0.0 },
        if left && bottom { radii[3] } else { 0.0 },
    ]
}

/// The opaque-black layer that goes **under** a video frame's own layer,
/// covering the viewport's whole box.
///
/// Paint punches its hole over the entire viewport box, but the frame is
/// aspect-fitted inside it, so a source whose aspect ratio differs from the
/// box's leaves letterbox/pillarbox bars that no layer covers — see-through to
/// the desktop on a transparent window (issue #354). This layer covers them,
/// black, which is what a browser paints for `<video>`.
///
/// It has to be the **compositor** that paints the black, not the viewport
/// element: this backend draws these layers *under* the Vello UI, so an opaque
/// element background would hide the frame outright. Pushing a backdrop layer
/// instead needs no change to [`rinch_platform::CompositeLayer`] or the
/// compositor shader — layers are drawn in order, and this one carries the same
/// `border_radius` and `clip_rect` as the frame's layer, so it picks up the
/// viewport's rounded corners and its ancestors' clipping for free.
///
/// The source is a single black texel stretched over `viewport`; every sample
/// of a 1×1 texture is that texel, whatever the filter mode. `Queue::write_texture`
/// places no row-alignment requirement on the upload.
///
/// Compiled under `test` as well as `gpu` so its geometry stays covered by the
/// default `cargo test`, which builds the software backend and cannot reach
/// `paint_gpu` at all.
#[cfg(any(feature = "gpu", test))]
fn black_backdrop_layer(
    viewport: (f32, f32, f32, f32),
    border_radius: [f32; 4],
    clip_rect: Option<(f32, f32, f32, f32)>,
) -> rinch_platform::CompositeLayer {
    rinch_platform::CompositeLayer {
        pixels: vec![0, 0, 0, 255],
        width: 1,
        height: 1,
        viewport,
        border_radius,
        clip_rect,
    }
}

// ── Renderer choice ──────────────────────────────────────────────────────────

/// The GPU renderer for `window`, or `None` to present with software: when
/// software was chosen (and the app did not configure the GPU device itself,
/// see `renderer::tries_gpu`), or when the GPU would not start under
/// [`Renderer::Auto`](super::renderer::Renderer::Auto). Also records which one
/// presents (`renderer::gpu_presenting`) and logs it.
///
/// # Panics
///
/// When the GPU would not start and the choice does not allow a fallback
/// (`Renderer::Gpu`, or a device the app configured itself).
#[cfg(feature = "gpu")]
fn start_gpu_renderer(
    window: &dyn winit::window::Window,
    transparent: bool,
) -> Option<WgpuRenderer> {
    use super::renderer::{OnGpuFailure, on_gpu_failure, resolved, set_gpu_presenting, tries_gpu};

    let choice = resolved();
    if !tries_gpu(choice, super::desktop::gpu_init_installed()) {
        set_gpu_presenting(false);
        tracing::info!("rinch: presenting with the software renderer (chosen)");
        return None;
    }
    let size = window.surface_size();
    match WgpuRenderer::try_new_transparent(
        window,
        size.width.max(1),
        size.height.max(1),
        transparent,
    ) {
        Ok(gpu) => {
            set_gpu_presenting(true);
            Some(gpu)
        }
        Err(e) => match on_gpu_failure(choice, super::desktop::gpu_init_installed()) {
            OnGpuFailure::FallBack => {
                set_gpu_presenting(false);
                tracing::warn!(
                    "rinch: the GPU renderer could not start ({e}); presenting with the \
                     software renderer"
                );
                None
            }
            OnGpuFailure::Panic => panic!("{e}"),
        },
    }
}

/// A build without `gpu` has only the software renderer: say so, and say it
/// louder when the app or `RINCH_RENDERER` asked for the GPU.
#[cfg(not(feature = "gpu"))]
fn announce_software_renderer() {
    use super::renderer::{Renderer, resolved};
    if resolved() == Renderer::Gpu {
        tracing::warn!(
            "rinch: the GPU renderer was requested, but this build has no `gpu` feature; \
             presenting with the software renderer"
        );
    } else {
        tracing::info!("rinch: presenting with the software renderer");
    }
}

// ── Public entry points ──────────────────────────────────────────────────────

/// Run a rinch application using the rinch-dom rendering pipeline.
///
/// This is an alternative to [`run`](crate::shell::run) that uses rinch-dom
/// (Taffy + Parley + Vello) for layout and rendering.
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
///
/// fn app(__scope: &mut RenderScope) -> NodeHandle {
///     let count = Signal::new(0);
///     let count_inc = count.clone();
///     rsx! {
///         div { style: "display: flex; flex-direction: column; padding: 20px; gap: 10px;",
///             p { "Count: " {|| count.get().to_string()} }
///             button {
///                 onclick: move || count_inc.update(|n| *n += 1),
///                 style: "padding: 8px 16px; background-color: #007bff; color: white;",
///                 "Increment"
///             }
///         }
///     }
/// }
///
/// fn main() {
///     rinch::run_rinch("Counter", 800, 600, app);
/// }
/// ```
#[deprecated(
    since = "0.2.0",
    note = "use `App::new(component).title(title).size(width, height).run()`"
)]
pub fn run_rinch<F>(title: &str, width: u32, height: u32, component: F)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();

    // Clear stale state
    events::clear_handlers();
    clear_context();

    let event_loop = overlay_window::new_event_loop();

    let proxy = event_loop.create_proxy();

    // Register main thread and cross-thread dispatcher for Signal::send()
    // Dispatcher first: a worker sending between the two would otherwise be
    // classified "not the main thread" with no dispatcher to take its write,
    // and panic. In this order it is still unregistered there, and its write
    // is queued through the dispatcher (#1541).
    rinch_core::set_cross_thread_dispatcher(dispatch_to_main_thread);
    rinch_core::register_main_thread();

    // Set up signal change notification
    rinch_core::set_on_signal_change(move || {
        send_native_event(RinchNativeEvent::ReRender);
    });

    let mut runtime = RinchRuntime::new(title, width, height, component, &[]);
    runtime.proxy = Some(proxy.clone());

    // Install push-based menu event handler (covers native + tray menus)
    crate::menu::install_menu_event_handler();

    // Set native proxy for window control functions
    NATIVE_PROXY.with(|p| *p.borrow_mut() = Some(proxy.clone()));

    // Set global proxy so run_on_main_thread() works from any thread
    install_global_proxy(&proxy);

    // Start debug IPC server if feature is enabled (disable with RINCH_DEBUG=0)
    #[cfg(feature = "debug")]
    {
        if std::env::var("RINCH_DEBUG").map_or(true, |v| v != "0") {
            match rinch_debug::attach(title, move || {
                send_native_event(RinchNativeEvent::DebugCommand);
            }) {
                Ok((debug_server, cmd_rx)) => {
                    tracing::info!("rinch-debug listening on port {}", debug_server.port());
                    runtime.app._debug_server = Some(debug_server);
                    runtime.app.debug_cmd_rx = Some(cmd_rx);
                }
                Err(e) => {
                    tracing::warn!("Failed to start rinch-debug server: {}", e);
                }
            }
        }
    }

    event_loop.run_app(runtime).expect("Event loop error");
}

/// Run a rinch-dom application with full window configuration.
#[deprecated(
    since = "0.2.0",
    note = "use `App::new(component).window_props(props).run()`"
)]
pub fn run_rinch_with_window_props<F>(component: F, props: rinch_core::element::WindowProps)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    run_rinch_with_window_props_and_menu(component, props, None, &[]);
}

/// Assemble the runtime from everything that is **not** the event loop.
///
/// Extracted so this assembly can be unit-tested: [`RinchRuntime::new`] opens
/// no window, so window props, the native menu and the bundled faces can all be
/// pinned without a display. Its caller cannot be tested at all — it creates the
/// winit event loop and then never returns — which is exactly why as little as
/// possible happens there.
///
/// What that leaves untested is the caller forwarding its own four arguments
/// into this call. That residue is structural and shared with every other
/// terminal path in the crate (issue #493): a *new* call site cannot silently
/// drop the fonts, because the parameter is required and has no default.
fn build_runtime<F>(
    component: F,
    props: &rinch_core::element::WindowProps,
    native_menu: Option<muda::Menu>,
    fonts: &[crate::font::AppFont],
) -> RinchRuntime
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    let mut runtime = RinchRuntime::new(&props.title, props.width, props.height, component, fonts);
    runtime.app.set_window_props(props.clone());
    runtime.native_menu = native_menu;
    runtime
}

/// Run a rinch-dom application with full window configuration, an optional
/// native menu, and the app's own bundled typefaces.
///
/// The single desktop dispatch target: [`App::run`](crate::App::run) resolves
/// every builder option and lands here. `fonts` is a parameter rather than
/// something registered beforehand because the faces have to reach the
/// `RinchApp` at construction — ahead of the window, and therefore ahead of
/// the first layout pass. Pass `&[]` for a build that carries none.
pub fn run_rinch_with_window_props_and_menu<F>(
    component: F,
    props: rinch_core::element::WindowProps,
    native_menu: Option<muda::Menu>,
    fonts: &[crate::font::AppFont],
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();

    events::clear_handlers();
    clear_context();

    let event_loop = overlay_window::new_event_loop();

    let proxy = event_loop.create_proxy();

    // Register main thread and cross-thread dispatcher for Signal::send()
    // Dispatcher first: a worker sending between the two would otherwise be
    // classified "not the main thread" with no dispatcher to take its write,
    // and panic. In this order it is still unregistered there, and its write
    // is queued through the dispatcher (#1541).
    rinch_core::set_cross_thread_dispatcher(dispatch_to_main_thread);
    rinch_core::register_main_thread();

    rinch_core::set_on_signal_change(move || {
        send_native_event(RinchNativeEvent::ReRender);
    });

    #[cfg(all(target_os = "linux", feature = "system-tray"))]
    if let Some(id) = props.app_id.as_deref() {
        let _ = DECLARED_APP_ID.set(id.to_string());
    }
    let mut runtime = build_runtime(component, &props, native_menu, fonts);
    runtime.proxy = Some(proxy.clone());

    // Install push-based menu event handler (covers native + tray menus)
    crate::menu::install_menu_event_handler();

    // Set native proxy for window control functions
    NATIVE_PROXY.with(|p| *p.borrow_mut() = Some(proxy.clone()));

    // Set global proxy so run_on_main_thread() works from any thread
    install_global_proxy(&proxy);

    #[cfg(feature = "debug")]
    {
        if std::env::var("RINCH_DEBUG").map_or(true, |v| v != "0") {
            match rinch_debug::attach(&props.title, move || {
                send_native_event(RinchNativeEvent::DebugCommand);
            }) {
                Ok((debug_server, cmd_rx)) => {
                    tracing::info!("rinch-debug listening on port {}", debug_server.port());
                    runtime.app._debug_server = Some(debug_server);
                    runtime.app.debug_cmd_rx = Some(cmd_rx);
                }
                Err(e) => {
                    tracing::warn!("Failed to start rinch-debug server: {}", e);
                }
            }
        }
    }

    event_loop.run_app(runtime).expect("Event loop error");
}

/// Decode a PNG from raw bytes into RGBA pixel data.
///
/// Returns `(rgba_bytes, width, height)`.
pub(crate) fn decode_png_to_rgba(
    png_data: &[u8],
) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    let decoder = png::Decoder::new(png_data);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    let bytes = &buf[..info.buffer_size()];

    // Convert to RGBA8 if needed
    let rgba = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(bytes.len() / 3 * 4);
            for chunk in bytes.chunks(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        other => {
            return Err(format!("Unsupported PNG color type: {:?}", other).into());
        }
    };

    Ok((rgba, info.width, info.height))
}

/// Decode a PNG icon from raw bytes into a winit Icon.
fn load_window_icon(png_data: &[u8]) -> Result<winit::icon::Icon, Box<dyn std::error::Error>> {
    let (rgba, width, height) = decode_png_to_rgba(png_data)?;
    let rgba_icon = winit::icon::RgbaIcon::new(rgba, width, height)?;
    Ok(rgba_icon.into())
}

/// The app id the last window was created with, for a service that has to
/// name the application to the desktop the same way the window does (the
/// global-shortcuts portal, `crate::hotkey`; notifications' `desktop-entry`
/// hint, `crate::notification`).
#[cfg(target_os = "linux")]
static WINDOW_APP_ID: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The app id this application presents to the desktop: the window's, once
/// one has been created, else the one a window would get with no
/// `WindowProps::app_id`.
#[cfg(target_os = "linux")]
#[cfg_attr(
    not(any(feature = "global-hotkeys", feature = "notifications")),
    allow(dead_code)
)]
pub(crate) fn current_app_id() -> String {
    WINDOW_APP_ID
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .unwrap_or_else(|| resolved_app_id(None))
}

/// The Wayland `app_id` / X11 `WM_CLASS` this window identifies itself by.
///
/// An app that sets [`WindowProps::app_id`] gets exactly that. Everything else
/// gets its **own** identity, derived from the executable's file stem, rather
/// than a constant shared with every other rinch application.
///
/// A shared default is not merely a cosmetic label. `install_wayland_icon`
/// writes `~/.local/share/applications/{app_id}.desktop` carrying
/// `Name={app_id}`, and compositors match a window to a desktop entry by
/// `app_id` — so under one constant, the first rinch app to ship an icon
/// supplies the dock icon *and* the displayed name for every other rinch app
/// on the machine, and they group together as a single application.
///
/// Falls back to `"rinch-app"` only when the executable path is unavailable,
/// which is where the old behaviour came from.
#[cfg(target_os = "linux")]
pub(crate) fn resolved_app_id(explicit: Option<&str>) -> String {
    if let Some(id) = explicit {
        return id.to_string();
    }
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(app_id_from_exe_path)
        .unwrap_or_else(|| "rinch-app".to_string())
}

/// The `WindowProps::app_id` the running app declared, recorded when its
/// event loop starts. Read by the tray (`rinch::tray`, #1529) so a tray built
/// after that point reports the same identity as the window; a tray built
/// earlier (the usual order: in `main`, before `App::run`) cannot see it and
/// takes `TrayIconBuilder::with_id` or the executable name instead.
#[cfg(all(target_os = "linux", feature = "system-tray"))]
static DECLARED_APP_ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// See [`DECLARED_APP_ID`].
#[cfg(all(target_os = "linux", feature = "system-tray"))]
pub(crate) fn declared_app_id() -> Option<&'static str> {
    DECLARED_APP_ID.get().map(String::as_str)
}

/// The identity an executable path implies: its **whole** file name.
///
/// Not `file_stem`. A stem truncates at the last dot, so the reverse-DNS
/// binary names this convention is built for — `com.example.notes` — would
/// register as `com.example` and collide with every sibling in that namespace,
/// which is the exact failure the derivation exists to prevent. Linux
/// executables carry no extension for a stem to usefully strip, and this
/// function is Linux-only.
#[cfg(target_os = "linux")]
fn app_id_from_exe_path(path: &std::path::Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
}

/// Write the icon PNG to a data directory and create a `.desktop` file so Wayland
/// compositors can display the icon in the taskbar via `app_id` matching.
#[cfg(target_os = "linux")]
fn install_wayland_icon(app_id: &str, png_data: &[u8]) {
    // If the app already has a system-wide launcher (e.g. it was installed from
    // a package), don't write a user-level NoDisplay stub of the same name: it
    // would shadow that launcher and hide the app from the application menu. The
    // packaged .desktop already supplies the taskbar icon via app_id/StartupWMClass.
    if system_desktop_entry_exists(app_id) {
        return;
    }

    let Some(data_home) = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs_icon_fallback)
    else {
        return;
    };
    install_wayland_icon_in(&data_home, app_id, png_data);
}

/// [`install_wayland_icon`]'s writes, against an explicit data directory.
#[cfg(target_os = "linux")]
fn install_wayland_icon_in(data_home: &std::path::Path, app_id: &str, png_data: &[u8]) {
    // Decode and resize icon to 256x256 for desktop integration
    let icon_dir = data_home.join("rinch/icons");
    if std::fs::create_dir_all(&icon_dir).is_err() {
        return;
    }
    let icon_path = icon_dir.join(format!("{app_id}.png"));
    let resized_png = match resize_png_icon(png_data, 256) {
        Ok(data) => data,
        Err(e) => {
            tracing::warn!("Failed to resize icon: {}", e);
            return;
        }
    };
    if std::fs::write(&icon_path, &resized_png).is_err() {
        return;
    }

    // Write .desktop file with absolute icon path (most reliable across compositors)
    let desktop_dir = data_home.join("applications");
    if std::fs::create_dir_all(&desktop_dir).is_err() {
        return;
    }
    let desktop_path = desktop_dir.join(format!("{app_id}.desktop"));
    // An entry there that is not a stub is the app's own launcher (an installer
    // that writes to the user's data directory, as an app that updates itself
    // must): replacing it with a NoDisplay stub took the app out of the
    // application menu on every start. That entry names the icon itself.
    let Some(carried) = icon_stub_carries(&desktop_path) else {
        return;
    };
    let desktop_content = format!(
        "[Desktop Entry]\nType=Application\nName={app_id}\nIcon={}\nNoDisplay=true\n\
         {carried}{RINCH_STUB_MARKER}\n",
        icon_path.display()
    );
    let _ = std::fs::write(&desktop_path, desktop_content);
}

/// The key every `.desktop` stub rinch writes carries (the icon stub and the
/// global-hotkeys stub): an entry with it is rinch's own to rewrite.
#[cfg(target_os = "linux")]
const RINCH_STUB_MARKER: &str = "X-Rinch-Stub=true";

/// Whether [`install_wayland_icon`] may write its stub at `path`, and the
/// lines of the entry there it must keep: `Some("")` when nothing is there,
/// `Some(lines)` for a stub rinch wrote — one carrying [`RINCH_STUB_MARKER`],
/// whose `Exec=` and `X-Rinch-Generated=` lines are kept (the global-hotkeys
/// stub's `Exec=` is what the portal needs), or an icon stub from before the
/// marker (`NoDisplay=true` and no `Exec=`). `None` for anything else: the
/// app's own launcher, or one the user hid through a menu editor, which
/// carries `NoDisplay=true` beside its own `Exec=` and no marker.
#[cfg(target_os = "linux")]
fn icon_stub_carries(path: &std::path::Path) -> Option<String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(String::new()),
        Err(_) => return None,
    };
    let has = |line: &str| text.lines().any(|l| l.trim() == line);
    let marked = has(RINCH_STUB_MARKER);
    let old_icon_stub =
        has("NoDisplay=true") && !text.lines().any(|l| l.trim_start().starts_with("Exec="));
    if !marked && !old_icon_stub {
        return None;
    }
    Some(
        text.lines()
            .map(str::trim)
            .filter(|l| l.starts_with("Exec=") || l.starts_with("X-Rinch-Generated="))
            .map(|l| format!("{l}\n"))
            .collect(),
    )
}

/// Returns true if a `.desktop` entry for `app_id` exists in any system
/// application directory (`$XDG_DATA_DIRS`, default `/usr/local/share:/usr/share`).
#[cfg(target_os = "linux")]
fn system_desktop_entry_exists(app_id: &str) -> bool {
    desktop_entry_in(&system_data_dirs(), app_id)
}

/// `$XDG_DATA_DIRS`, read as an `OsString` so non-UTF8 directory entries are
/// preserved (`var()` would drop the whole value to the default on any
/// non-UTF8 byte).
#[cfg(target_os = "linux")]
fn system_data_dirs() -> std::ffi::OsString {
    std::env::var_os("XDG_DATA_DIRS")
        .unwrap_or_else(|| std::ffi::OsString::from("/usr/local/share:/usr/share"))
}

/// Whether `applications/{app_id}.desktop` exists under any of `dirs`.
#[cfg(target_os = "linux")]
fn desktop_entry_in(dirs: &std::ffi::OsStr, app_id: &str) -> bool {
    let rel = format!("applications/{app_id}.desktop");
    std::env::split_paths(dirs)
        .filter(|p| !p.as_os_str().is_empty())
        .any(|p| p.join(&rel).exists())
}

/// Write a minimal `NoDisplay` desktop entry for `app_id` in the user's
/// applications directory, unless one exists there or in any system
/// directory — never shadowing a packaged launcher, never overwriting a file.
///
/// For a service that the desktop accepts only from an application with an
/// installed entry: the Wayland global-shortcuts portal (`crate::hotkey`)
/// refuses `Registry.Register` with "App info not found" otherwise. Answers
/// the path written, `Ok(None)` when an entry already existed.
#[cfg(target_os = "linux")]
#[cfg_attr(not(feature = "global-hotkeys"), allow(dead_code))]
pub(crate) fn ensure_desktop_entry(app_id: &str) -> Result<Option<std::path::PathBuf>, String> {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs_icon_fallback)
        .ok_or("neither XDG_DATA_HOME nor HOME is set")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    ensure_desktop_entry_in(&system_data_dirs(), &data_home, app_id, &exe)
}

/// [`ensure_desktop_entry`] against explicit directories.
#[cfg(target_os = "linux")]
#[cfg_attr(not(feature = "global-hotkeys"), allow(dead_code))]
fn ensure_desktop_entry_in(
    system_dirs: &std::ffi::OsStr,
    data_home: &std::path::Path,
    app_id: &str,
    exe: &std::path::Path,
) -> Result<Option<std::path::PathBuf>, String> {
    if desktop_entry_in(system_dirs, app_id) {
        return Ok(None);
    }
    write_desktop_entry_stub(data_home, app_id, exe)
}

/// [`ensure_desktop_entry`]'s write, against an explicit data directory.
#[cfg(target_os = "linux")]
#[cfg_attr(not(feature = "global-hotkeys"), allow(dead_code))]
fn write_desktop_entry_stub(
    data_home: &std::path::Path,
    app_id: &str,
    exe: &std::path::Path,
) -> Result<Option<std::path::PathBuf>, String> {
    if app_id.is_empty() || app_id.contains(['/', '\n']) {
        return Err(format!("{app_id:?} is not a usable app id"));
    }
    let dir = data_home.join("applications");
    let path = dir.join(format!("{app_id}.desktop"));
    if path.exists() {
        return Ok(None);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Exec is required for Type=Application; quoted per the desktop entry
    // specification, with its reserved characters escaped.
    let exec = exe
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$");
    let content = format!(
        "[Desktop Entry]\nType=Application\nName={app_id}\nExec=\"{exec}\"\nNoDisplay=true\n\
         X-Rinch-Generated=global-hotkeys\n{RINCH_STUB_MARKER}\n"
    );
    // `create_new`: a file that appeared since the check is not overwritten.
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut f) => f
            .write_all(content.as_bytes())
            .map(|()| Some(path.clone()))
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[cfg(target_os = "linux")]
fn dirs_icon_fallback() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
}

/// Decode a PNG, resize to `target_size` x `target_size` with bilinear filtering, re-encode as PNG.
#[cfg(target_os = "linux")]
fn resize_png_icon(
    png_data: &[u8],
    target_size: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Decode source PNG
    let decoder = png::Decoder::new(png_data);
    let mut reader = decoder.read_info()?;
    let mut src_buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut src_buf)?;
    let src = &src_buf[..info.buffer_size()];
    let (sw, sh) = (info.width as usize, info.height as usize);

    // Convert to RGBA if needed
    let rgba_src: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => src.to_vec(),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(sw * sh * 4);
            for chunk in src.chunks(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        other => return Err(format!("Unsupported color type: {:?}", other).into()),
    };

    let ts = target_size as usize;
    let mut dst = vec![0u8; ts * ts * 4];

    // Bilinear downscale
    for dy in 0..ts {
        for dx in 0..ts {
            let sx_f = (dx as f64 + 0.5) * sw as f64 / ts as f64 - 0.5;
            let sy_f = (dy as f64 + 0.5) * sh as f64 / ts as f64 - 0.5;
            let x0 = (sx_f.floor() as isize).clamp(0, sw as isize - 1) as usize;
            let y0 = (sy_f.floor() as isize).clamp(0, sh as isize - 1) as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let y1 = (y0 + 1).min(sh - 1);
            let fx = (sx_f - x0 as f64).clamp(0.0, 1.0);
            let fy = (sy_f - y0 as f64).clamp(0.0, 1.0);

            for c in 0..4 {
                let p00 = rgba_src[(y0 * sw + x0) * 4 + c] as f64;
                let p10 = rgba_src[(y0 * sw + x1) * 4 + c] as f64;
                let p01 = rgba_src[(y1 * sw + x0) * 4 + c] as f64;
                let p11 = rgba_src[(y1 * sw + x1) * 4 + c] as f64;
                let val = p00 * (1.0 - fx) * (1.0 - fy)
                    + p10 * fx * (1.0 - fy)
                    + p01 * (1.0 - fx) * fy
                    + p11 * fx * fy;
                dst[(dy * ts + dx) * 4 + c] = val.round() as u8;
            }
        }
    }

    // Encode to PNG
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, target_size, target_size);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&dst)?;
    }
    Ok(out)
}

#[cfg(test)]
mod viewport_tests {
    use super::*;

    /// A window that reports a physical surface size and a scale factor.
    struct FakeWindow {
        surface: (u32, u32),
        scale: f64,
    }

    impl PlatformWindow for FakeWindow {
        fn inner_size(&self) -> (u32, u32) {
            self.surface
        }
        fn scale_factor(&self) -> f64 {
            self.scale
        }
        fn request_redraw(&self) {}
        fn set_minimized(&self, _minimized: bool) {}
        fn set_maximized(&self, _maximized: bool) {}
        fn set_visible(&self, _visible: bool) {}
        fn is_maximized(&self) -> bool {
            false
        }
        fn drag_window(&self) -> Result<(), String> {
            Ok(())
        }
        fn set_title(&self, _title: &str) {}
    }

    /// The desktop shell used to hand the document `PlatformWindow::inner_size`
    /// — the *physical* surface size — as its layout viewport, while paint
    /// multiplies every layout coordinate by the same window's scale factor.
    /// On a 1.25x display that laid the page out 1.25x too wide and then drew
    /// it 1.25x larger again, so the rightmost fifth fell outside the surface:
    /// a `flex: 1` child's trailing siblings (a row's confidence dots, the box
    /// after a growing one) were laid out correctly and simply never appeared.
    ///
    /// `run("probe", 460, 300, ..)` on a 1.25x display gets a 575x975 surface;
    /// the viewport is 460 CSS pixels wide, not 575.
    #[test]
    fn layout_viewport_is_logical_not_physical() {
        let window = FakeWindow {
            surface: (575, 975),
            scale: 1.25,
        };
        assert_eq!(
            window.logical_size(),
            (460, 780),
            "layout must get the logical viewport, not the 575x975 surface size"
        );
    }

    /// The page must fit the surface once paint has scaled it — the invariant
    /// the physical-size viewport broke.
    #[test]
    fn scaled_layout_viewport_fits_the_surface() {
        for &(surface, scale) in &[
            ((575u32, 975u32), 1.25f64),
            ((786, 1704), 2.0),
            ((491, 1065), 1.25),
        ] {
            let window = FakeWindow { surface, scale };
            let (vw, vh) = window.logical_size();
            let painted = (vw as f64 * scale, vh as f64 * scale);
            assert!(
                (painted.0 - surface.0 as f64).abs() <= 1.0
                    && (painted.1 - surface.1 as f64).abs() <= 1.0,
                "viewport {vw}x{vh} painted at {scale}x is {painted:?}, \
                 which does not fill the {surface:?} surface"
            );
        }
    }

    /// A 1x display is unaffected, and a nonsensical scale factor must not
    /// collapse the viewport to zero.
    #[test]
    fn layout_viewport_degenerate_scales() {
        assert_eq!(
            FakeWindow {
                surface: (800, 600),
                scale: 1.0
            }
            .logical_size(),
            (800, 600)
        );
        assert_eq!(
            FakeWindow {
                surface: (800, 600),
                scale: 0.0
            }
            .logical_size(),
            (800, 600),
            "a zero scale factor must fall back to 1x, not divide by zero"
        );
        assert_eq!(
            FakeWindow {
                surface: (1, 1),
                scale: 4.0
            }
            .logical_size(),
            (1, 1),
            "the viewport must never round down to zero"
        );
        for bad in [f64::NAN, f64::INFINITY, -2.0] {
            assert_eq!(
                FakeWindow {
                    surface: (800, 600),
                    scale: bad
                }
                .logical_size(),
                (800, 600),
                "a {bad} scale factor must fall back to 1x"
            );
        }
    }

    /// The viewport the shell lays out at and the one `RinchApp::handle_event`
    /// re-lays out at on every `ReRender` / `AboutToWait` must be the *same*
    /// number. They are derived on opposite sides of the shell boundary — the
    /// shell from the window, the app from the physical size it was handed — so
    /// a divergence here silently reintroduces the oversized layout one frame
    /// after mount.
    #[test]
    fn app_and_shell_agree_on_the_layout_viewport() {
        for &(surface, scale) in &[
            ((575u32, 975u32), 1.25f64),
            ((577, 977), 1.25),
            ((786, 1704), 2.0),
            ((800, 600), 1.0),
        ] {
            let window = FakeWindow { surface, scale };
            let (sw, sh) = window.logical_size();
            let (aw, ah) = RinchApp::layout_viewport(surface, scale);
            assert_eq!(
                (sw as f32, sh as f32),
                (aw, ah),
                "shell laid out {surface:?}@{scale}x at {sw}x{sh} but the app \
                 re-lays it out at {aw}x{ah}"
            );
        }
    }
}

// ── #354: the GPU compositor covers the whole punched box ────────────────
//
// Paint cuts the hole over a viewport's entire layout box; the compositor
// aspect-fits the frame inside it. Whatever the compositor does not draw in
// that box is background-removed *and* frame-free — see-through to the desktop
// on a transparent window. The GPU backend therefore draws a black backdrop
// layer under the frame, the way a browser paints `<video>` bars, from the
// compositor rather than from the element (this backend draws the video *under*
// the UI, so an opaque element background would hide the frame outright).

#[cfg(test)]
mod letterbox_backdrop {
    use super::*;

    /// The GPU compositor draws `composite_layers` in order, so the backdrop
    /// must cover the **whole box** (not the fitted rect) and carry the frame
    /// layer's radii and clip, or the bars are unpainted or the corners square.
    ///
    /// `paint_gpu` itself needs a device and a window, so this covers the one
    /// part of it that is pure geometry. It is compiled in every configuration
    /// (see `black_backdrop_layer`'s `cfg`), so the default `cargo test` — which
    /// builds the software backend — still guards the GPU path's shape.
    #[test]
    fn gpu_backdrop_layer_covers_the_whole_box_with_black() {
        let box_viewport = (10.0, 20.0, 200.0, 100.0);
        let radii = [8.0, 8.0, 8.0, 8.0];
        let clip = Some((0.0, 0.0, 400.0, 300.0));
        let layer = black_backdrop_layer(box_viewport, radii, clip);

        assert_eq!(
            layer.viewport, box_viewport,
            "the backdrop covers the punched box, not the fitted rect (#354)"
        );
        assert_eq!(layer.border_radius, radii, "and the viewport's corners");
        assert_eq!(layer.clip_rect, clip, "and its ancestors' clipping");
        assert_eq!(
            (layer.width, layer.height),
            (1, 1),
            "one texel is enough — every sample of a 1x1 texture is that texel"
        );
        assert_eq!(
            layer.pixels,
            vec![0, 0, 0, 255],
            "opaque black, so the bars are covered rather than see-through"
        );
    }

    /// The radii reported for a viewport belong to its *clipping ancestor*, so
    /// they only describe corners the two rects share. The UI Zoo shape — a
    /// video filling the top of a card whose lower half is the controls bar —
    /// is the one that bites: rounding the viewport's bottom edge carves a
    /// notch out of the middle of the card that nothing then fills.
    #[test]
    fn only_corners_shared_with_the_clip_keep_their_radius() {
        let card = Some((0.0, 0.0, 200.0, 300.0));
        // The video occupies the card's top 100px: top corners are real, the
        // bottom edge runs through the middle of the card.
        assert_eq!(
            radii_at_shared_corners((0.0, 0.0, 200.0, 100.0), card, [8.0; 4]),
            [8.0, 8.0, 0.0, 0.0],
            "the bottom corners are not corners — rounding them would cut a \
             see-through notch mid-card (#354)"
        );
        // A viewport filling its clip keeps every corner.
        assert_eq!(
            radii_at_shared_corners((0.0, 0.0, 200.0, 300.0), card, [8.0; 4]),
            [8.0; 4]
        );
        // Floating in the middle of the clip: no corner is shared.
        assert_eq!(
            radii_at_shared_corners((50.0, 50.0, 100.0, 100.0), card, [8.0; 4]),
            [0.0; 4]
        );
        // No clip rect: nothing to compare against, so take the radii as given.
        assert_eq!(
            radii_at_shared_corners((0.0, 0.0, 200.0, 100.0), None, [8.0; 4]),
            [8.0; 4]
        );
        // A pixel of slop, because both rects are truncated from the same
        // layout floats.
        assert_eq!(
            radii_at_shared_corners((1.0, 0.0, 199.0, 300.0), card, [8.0; 4]),
            [8.0; 4]
        );
    }
}

/// The Wayland/X11 application identity (issue #493 review).
///
/// These pin the property that makes the identity worth deriving at all: two
/// different rinch applications must not answer to the same `app_id`. A
/// constant default made every app that did not set one share a single desktop
/// identity — and because `install_wayland_icon` writes
/// `{app_id}.desktop` with `Name={app_id}`, that meant the first rinch app to
/// ship an icon supplied the dock icon and name for all the others.
#[cfg(all(test, target_os = "linux"))]
mod app_id_tests {
    use super::{
        app_id_from_exe_path, ensure_desktop_entry_in, icon_stub_carries, install_wayland_icon_in,
        resolved_app_id, write_desktop_entry_stub,
    };

    /// The icon stub never replaces the app's own launcher in the user's
    /// applications directory (an app whose installer writes its launcher
    /// there lost it to a NoDisplay stub on every start, and left the menu).
    /// Nothing there, or an earlier stub, may be written.
    #[test]
    fn the_icon_stub_never_replaces_a_users_launcher() {
        let root = std::env::temp_dir().join(format!("rinch-icon-stub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let launcher = root.join("notes.desktop");
        std::fs::write(
            &launcher,
            "[Desktop Entry]\nType=Application\nName=Notes\nExec=/opt/notes\n",
        )
        .unwrap();
        assert_eq!(icon_stub_carries(&launcher), None);
        let stub = root.join("stub.desktop");
        std::fs::write(
            &stub,
            "[Desktop Entry]\nType=Application\nName=stub\nIcon=/x.png\nNoDisplay=true\n",
        )
        .unwrap();
        assert_eq!(icon_stub_carries(&stub).as_deref(), Some(""));
        assert_eq!(
            icon_stub_carries(&root.join("absent.desktop")).as_deref(),
            Some("")
        );
        // A launcher the user hid through a menu editor: `NoDisplay=true`
        // beside its own `Exec=`. It is the user's, not a stub.
        let hidden = root.join("hidden.desktop");
        std::fs::write(
            &hidden,
            "[Desktop Entry]\nType=Application\nName=Notes\nExec=/opt/x\nNoDisplay=true\n",
        )
        .unwrap();
        assert_eq!(icon_stub_carries(&hidden), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 2, 2);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[200u8; 16]).unwrap();
        }
        out
    }

    /// `install_wayland_icon` itself (through its data-directory half) leaves
    /// a launcher it does not own byte for byte as it was — the app's own and
    /// a user-hidden one alike — and writes its stub where there is none.
    #[test]
    fn installing_the_icon_keeps_a_launcher_it_does_not_own() {
        let root = std::env::temp_dir().join(format!("rinch-icon-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let apps = root.join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        let png = tiny_png();
        for (id, text) in [
            (
                "notes",
                "[Desktop Entry]\nType=Application\nName=Notes\nExec=/opt/notes\n",
            ),
            (
                "hidden",
                "[Desktop Entry]\nType=Application\nName=Hidden\nExec=/opt/x\nNoDisplay=true\n",
            ),
        ] {
            let path = apps.join(format!("{id}.desktop"));
            std::fs::write(&path, text).unwrap();
            install_wayland_icon_in(&root, id, &png);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{id}");
        }
        // Nothing there: the stub is written, naming the icon it installed.
        install_wayland_icon_in(&root, "fresh", &png);
        let stub = std::fs::read_to_string(apps.join("fresh.desktop")).unwrap();
        assert!(stub.contains("NoDisplay=true"), "{stub}");
        assert!(stub.contains("rinch/icons/fresh.png"), "{stub}");
        assert!(root.join("rinch/icons/fresh.png").exists());
        assert!(stub.contains("X-Rinch-Stub=true"), "{stub}");
        // An icon stub from before the marker is replaced (here: rewritten in
        // place), and carries the marker from then on.
        std::fs::write(
            apps.join("fresh.desktop"),
            "[Desktop Entry]\nType=Application\nName=fresh\nIcon=/old.png\nNoDisplay=true\n",
        )
        .unwrap();
        install_wayland_icon_in(&root, "fresh", &png);
        let again = std::fs::read_to_string(apps.join("fresh.desktop")).unwrap();
        assert!(again.contains("rinch/icons/fresh.png"), "{again}");
        assert!(again.contains("X-Rinch-Stub=true"), "{again}");
        // The global-hotkeys stub, as `ensure_desktop_entry` writes it: it is
        // rinch's own, so it gets the icon, and keeps the `Exec=` the portal
        // needs.
        let exe = std::path::Path::new("/opt/keys/bin");
        let written = write_desktop_entry_stub(&root, "keys", exe)
            .unwrap()
            .unwrap();
        let before = std::fs::read_to_string(&written).unwrap();
        assert!(before.contains("X-Rinch-Stub=true"), "{before}");
        install_wayland_icon_in(&root, "keys", &png);
        let keys = std::fs::read_to_string(&written).unwrap();
        assert!(keys.contains("rinch/icons/keys.png"), "{keys}");
        assert!(keys.contains("Exec=\"/opt/keys/bin\""), "{keys}");
        assert!(keys.contains("X-Rinch-Generated=global-hotkeys"), "{keys}");
        assert!(keys.contains("X-Rinch-Stub=true"), "{keys}");
        assert_eq!(keys.matches("Exec=").count(), 1, "{keys}");
        // Installing again changes nothing.
        install_wayland_icon_in(&root, "keys", &png);
        assert_eq!(std::fs::read_to_string(&written).unwrap(), keys);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A packaged launcher in a system directory is never shadowed: no stub
    /// is written for an app id one of them has.
    #[test]
    fn no_desktop_entry_stub_shadows_a_system_entry() {
        let root = std::env::temp_dir().join(format!("rinch-desktop-sys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (sys, home) = (root.join("sys"), root.join("home"));
        std::fs::create_dir_all(sys.join("applications")).unwrap();
        std::fs::write(sys.join("applications/com.example.notes.desktop"), "x").unwrap();
        let dirs = std::env::join_paths([root.join("empty"), sys.clone()]).unwrap();
        let exe = std::path::Path::new("/bin/notes");
        assert_eq!(
            ensure_desktop_entry_in(&dirs, &home, "com.example.notes", exe),
            Ok(None)
        );
        assert!(!home.join("applications/com.example.notes.desktop").exists());
        // Positive control: an id no system directory has is written.
        assert!(
            ensure_desktop_entry_in(&dirs, &home, "com.example.other", exe)
                .unwrap()
                .is_some()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The desktop-entry stub the hotkey portal needs: written once with
    /// `NoDisplay`, and an existing file — the app's own, or the user's — is
    /// never overwritten.
    #[test]
    fn a_desktop_entry_stub_never_overwrites() {
        let home = std::env::temp_dir().join(format!("rinch-desktop-stub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let exe = std::path::Path::new("/opt/My App/bin/notes");
        let written = write_desktop_entry_stub(&home, "com.example.notes", exe).unwrap();
        let path = home.join("applications/com.example.notes.desktop");
        assert_eq!(written.as_deref(), Some(path.as_path()));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("[Desktop Entry]\nType=Application\n"),
            "{text}"
        );
        assert!(
            text.contains("NoDisplay=true\n") && text.contains("Exec=\"/opt/My App/bin/notes\"\n"),
            "{text}"
        );

        std::fs::write(&path, "[Desktop Entry]\nName=Mine\n").unwrap();
        assert_eq!(
            write_desktop_entry_stub(&home, "com.example.notes", exe),
            Ok(None)
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[Desktop Entry]\nName=Mine\n"
        );
        assert!(write_desktop_entry_stub(&home, "a/b", exe).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The identity `resolved_app_id` would derive from a given executable path.
    fn id_of(path: &str) -> Option<String> {
        app_id_from_exe_path(std::path::Path::new(path))
    }

    /// An explicit `app_id` is handed back untouched — never re-derived, and
    /// never blended with the executable name.
    #[test]
    fn an_explicit_app_id_is_used_verbatim() {
        assert_eq!(
            resolved_app_id(Some("com.example.notes")),
            "com.example.notes"
        );
    }

    /// The explicit value wins even when it happens to be the old constant, so
    /// an app that deliberately asks for `rinch-app` still gets it.
    #[test]
    fn an_explicit_app_id_equal_to_the_old_default_is_still_honoured() {
        assert_eq!(resolved_app_id(Some("rinch-app")), "rinch-app");
    }

    /// With nothing declared, the identity comes from *this* executable — the
    /// whole point. Asserting against the real `current_exe` file stem rather
    /// than a literal keeps the test true under `cargo test`, a renamed test
    /// binary, and CI alike.
    #[test]
    fn an_undeclared_app_id_is_derived_from_the_executable() {
        let expected = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .expect("the test binary has a path");
        assert!(!expected.is_empty());
        assert_eq!(resolved_app_id(None), expected);
    }

    /// The regression this replaces: an undeclared id must NOT be the shared
    /// constant. A test binary is never named `rinch-app`, so this fails the
    /// moment the derivation is dropped — which the equality test above would
    /// too, but this one names the defect.
    #[test]
    fn an_undeclared_app_id_is_not_the_shared_constant() {
        assert_ne!(
            resolved_app_id(None),
            "rinch-app",
            "every rinch app sharing one app_id makes compositors group them \
             as a single application and match one another's .desktop entry"
        );
    }

    /// Two different executables get two different identities. This is the
    /// property the collision fix rests on, and neither test above states it:
    /// both would pass a derivation that returned a constant of its own.
    #[test]
    fn different_executables_get_different_identities() {
        assert_ne!(id_of("/usr/bin/my-notes"), id_of("/usr/bin/my-paint"));
        assert_eq!(
            id_of("/home/u/target/release/my-notes"),
            Some("my-notes".into())
        );
    }

    /// A dotted, reverse-DNS binary name survives whole.
    ///
    /// This is the case that makes `file_name` the right call and `file_stem`
    /// the wrong one: a stem truncates at the last dot, so `com.example.notes`
    /// would register as `com.example` and collide with every sibling in that
    /// namespace — reintroducing, in a smaller blast radius, the exact
    /// collision this derivation exists to remove. A fixture using only
    /// undotted names cannot see the difference, because there `file_stem` and
    /// `file_name` agree.
    #[test]
    fn a_dotted_executable_name_is_not_truncated_at_the_dot() {
        assert_eq!(
            id_of("/usr/bin/com.example.notes"),
            Some("com.example.notes".into())
        );
        assert_ne!(
            id_of("/usr/bin/com.example.notes"),
            id_of("/usr/bin/com.example.paint")
        );
    }

    /// A path with no file name at all falls back rather than yielding an
    /// empty identity.
    #[test]
    fn a_path_with_no_file_name_yields_nothing_to_derive_from() {
        assert_eq!(id_of("/"), None);
    }
}

#[cfg(test)]
mod logical_key_spelling {
    use super::*;
    use winit::keyboard::{Key, NamedKey, SmolStr};

    fn spell(key: Key) -> Option<String> {
        RinchRuntime::winit_logical_key_str(&key)
    }

    /// A character key is passed through verbatim — case included, because a
    /// browser's `KeyboardEvent.key` is case-accurate and a lowercased source
    /// made `Shift+A` release as `"a"` after pressing as `"A"` (issue #337's
    /// review finding). Non-letters ride the same arm now: `Shift+1` on a US
    /// layout is `"!"` down *and* up, where the old single-ASCII-letter filter
    /// dropped it and the release fell back to the physical `"1"`.
    #[test]
    fn a_character_key_is_verbatim_case_included() {
        assert_eq!(
            spell(Key::Character(SmolStr::new("a"))).as_deref(),
            Some("a")
        );
        assert_eq!(
            spell(Key::Character(SmolStr::new("A"))).as_deref(),
            Some("A"),
            "Shift+A is \"A\" in a browser — no lowercasing at the source"
        );
        assert_eq!(
            spell(Key::Character(SmolStr::new("!"))).as_deref(),
            Some("!")
        );
        assert_eq!(
            spell(Key::Character(SmolStr::new("é"))).as_deref(),
            Some("é")
        );
        assert_eq!(
            spell(Key::Character(SmolStr::new(" "))).as_deref(),
            Some(" "),
            "the spacebar's key value is the space character, per spec"
        );
    }

    /// A named key spells its W3C key-values name — `NamedKey` *is*
    /// `keyboard_types::NamedKey` and its `Display` is the spec string, so
    /// this pins the transcription, not a hand-kept table.
    #[test]
    fn a_named_key_spells_its_w3c_name() {
        assert_eq!(spell(Key::Named(NamedKey::Enter)).as_deref(), Some("Enter"));
        assert_eq!(
            spell(Key::Named(NamedKey::ArrowLeft)).as_deref(),
            Some("ArrowLeft")
        );
        assert_eq!(spell(Key::Named(NamedKey::Shift)).as_deref(), Some("Shift"));
        assert_eq!(
            spell(Key::Named(NamedKey::Meta)).as_deref(),
            Some("Meta"),
            "the OS key: winit folds X11's Super into Meta the way browsers do"
        );
    }

    /// A dead key is `"Dead"` (every browser's value for one); an unidentified
    /// key is `None`, so the consumer falls back to the physical `key` instead
    /// of pairing two different unknown keys by a shared placeholder.
    #[test]
    fn dead_spells_dead_and_unidentified_spells_nothing() {
        assert_eq!(spell(Key::Dead(Some('^'))).as_deref(), Some("Dead"));
        assert_eq!(spell(Key::Dead(None)).as_deref(), Some("Dead"));
        assert_eq!(
            spell(Key::Unidentified(winit::keyboard::NativeKey::Unidentified)),
            None
        );
    }
}

/// The desktop backend's `KeyRepeat` answer (issue #463).
///
/// The one producer pin on this backend. Every `key_repeat_tests` fixture hands
/// the runtime a `KeyRepeat` of its own, so nothing there can tell whether the
/// shell reports winit's flag, its negation, or a constant — inverting it
/// survived all 602 tests of `cargo test -p rinch --lib --features debug` in the
/// adversarial review of PR #796. That is the hole these two assertions close,
/// and it is worth knowing they are the *only* thing closing it.
#[cfg(test)]
mod key_repeat_translation {
    use super::*;
    use winit::keyboard::{Key, KeyCode as WK, SmolStr};

    fn press(repeat: bool) -> PlatformEvent {
        // The spacebar's logical key is its character, not a named key — the
        // W3C spells it `" "`, and so does winit.
        RinchRuntime::key_down_from_winit(
            WK::Space,
            &Key::Character(SmolStr::new(" ")),
            Some(" "),
            repeat,
            Modifiers::default(),
        )
    }

    fn repeat_of(event: &PlatformEvent) -> KeyRepeat {
        match event {
            PlatformEvent::KeyDown { repeat, .. } => *repeat,
            other => panic!("expected a KeyDown, got {other:?}"),
        }
    }

    /// winit's `repeat` is documented `true` **iff** the OS produced this press
    /// by holding the key, so the mapping is the identity and not a negation.
    /// Getting it backwards would make desktop refuse every real press and
    /// activate on every auto-repeat — #463 inverted, which is worse than #463.
    #[test]
    fn winits_flag_is_reported_not_inverted() {
        assert_eq!(repeat_of(&press(false)), KeyRepeat::Fresh);
        assert_eq!(repeat_of(&press(true)), KeyRepeat::Repeat);
    }

    /// And never `Unknown`: this backend always knows, so it must never fall
    /// back to the latch — which is the whole of #463 on desktop. Kills a
    /// shell that quietly answers `KeyRepeat::Unknown` (the `Default`, and so
    /// the shape a careless refactor reaches for).
    #[test]
    fn the_desktop_shell_never_answers_unknown() {
        for r in [false, true] {
            assert_ne!(repeat_of(&press(r)), KeyRepeat::Unknown);
        }
    }

    /// The extraction moved the whole press translation, not just the flag, so
    /// the rest of it is pinned here too — the physical key, the layout-mapped
    /// `logical_key` and the inserted `text` all still ride along.
    #[test]
    fn the_rest_of_the_press_survives_the_split() {
        let PlatformEvent::KeyDown {
            key,
            logical_key,
            text,
            ..
        } = press(false)
        else {
            panic!("expected a KeyDown");
        };
        assert_eq!(key, KeyCode::Space);
        assert_eq!(logical_key.as_deref(), Some(" "));
        assert_eq!(text.as_deref(), Some(" "));
    }
}

#[cfg(test)]
mod bundled_font_tests {
    use super::*;
    use crate::font::AppFont;

    const FACE: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

    fn component(scope: &mut RenderScope) -> NodeHandle {
        scope.create_element("div")
    }

    /// The runtime registers a build's bundled faces **at construction** — the
    /// only moment early enough, since `RinchRuntime::new` runs before the
    /// window and therefore before the first layout pass. A face that arrived
    /// later would leave the first frame measured against a fallback, then
    /// reflow.
    ///
    /// This is testable only because `new` is a pure struct build: it opens no
    /// window and touches no event loop. [`App::run`](crate::App::run) around
    /// it is not, which is exactly why the registration lives here rather than
    /// further in.
    #[test]
    fn hiding_the_window_publishes_not_focused_1770() {
        let mut runtime = RinchRuntime::new("Hidden", 640, 480, component, &[]);
        runtime
            .app
            .handle_event(PlatformEvent::WindowFocus(true), (640, 480), 1.0);
        assert!(crate::window::window_focused(), "positive control");
        runtime.hide_window();
        assert!(!crate::window::window_focused());
    }

    #[test]
    fn the_runtime_registers_bundled_fonts_at_construction() {
        let runtime = RinchRuntime::new(
            "t",
            800,
            600,
            component,
            &[AppFont::monospace(FACE), AppFont::serif(FACE)],
        );
        let generics: Vec<_> = runtime
            .app
            .pending_fonts
            .iter()
            .map(|f| f.generics[0])
            .collect();
        assert_eq!(
            generics,
            vec![
                parley::fontique::GenericFamily::Monospace,
                parley::fontique::GenericFamily::Serif
            ],
            "both faces must reach the app, in the order they were declared"
        );
    }

    /// The shell installs the named video sink factory (rinch-core's hook
    /// that rinch-video and rinch-rtc draw through). Counted on this thread,
    /// so another test installing it meanwhile cannot make this pass.
    #[test]
    fn the_runtime_installs_the_video_sink_factory() {
        let before = rinch_core::video_sink::installs_on_this_thread();
        let _runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        assert_eq!(
            rinch_core::video_sink::installs_on_this_thread(),
            before + 1
        );
        assert!(rinch_core::video_sink::has_video_sink_factory());
    }

    /// And a build that bundles none is not handed a phantom face.
    #[test]
    fn no_bundled_fonts_registers_nothing() {
        let runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        assert!(runtime.app.pending_fonts.is_empty());
    }

    /// The assembly the real desktop path actually uses. `App::run` resolves
    /// the builder and lands in `run_rinch_with_window_props_and_menu`, which
    /// creates the event loop and can never be tested — so everything it does
    /// *before* that lives in `build_runtime`, and this is what pins it.
    ///
    /// Both halves matter and neither was covered before: the faces must
    /// survive the trip (a `&[]` here and `App::fonts` would be a builder
    /// method with no effect on any real window), and so must the window props
    /// that carry the title, size and `app_id`.
    /// #1519: the click-through state the window is created with. The first
    /// creation takes `WindowProps::click_through`; a toggle is kept across a
    /// hide/show re-creation (which does not re-read the props); a new first
    /// creation starts from the props again.
    #[test]
    fn click_through_starts_at_the_props_and_a_toggle_survives_a_re_show() {
        use super::overlay_window::{AfterCreate, after_create_actions};
        let props = rinch_core::element::WindowProps {
            click_through: true,
            ..Default::default()
        };
        let mut runtime = build_runtime(component, &props, None, &[]);
        let first = runtime.main_window_steps(Backend::X11, true);
        assert!(first.click_through, "the prop reaches the first creation");
        assert!(after_create_actions(first, false).contains(&AfterCreate::ClickThrough));

        assert!(!runtime.note_click_through(false));
        let reshown = runtime.main_window_steps(Backend::X11, false);
        assert!(
            !reshown.click_through,
            "the toggle is kept across hide/show"
        );
        assert!(!after_create_actions(reshown, false).contains(&AfterCreate::ClickThrough));

        assert!(runtime.note_click_through(true));
        assert!(runtime.main_window_steps(Backend::X11, false).click_through);

        // Off in the props: a re-show after a toggle on keeps it on.
        let mut plain = build_runtime(component, &Default::default(), None, &[]);
        assert!(
            !plain
                .main_window_steps(Backend::Wayland, true)
                .click_through
        );
        plain.note_click_through(true);
        assert!(
            plain
                .main_window_steps(Backend::Wayland, false)
                .click_through
        );
    }

    /// #1549: the level and workspace settings come from the props on every
    /// creation, so a hide/show re-creation (#1546) puts them back; and a
    /// raised level is winit's `AlwaysOnTop` at creation on every backend.
    #[test]
    fn the_level_and_workspaces_survive_a_re_show() {
        use super::overlay_window::{AfterCreate, after_create_actions};
        use rinch_core::element::WindowLevel;
        let props = rinch_core::element::WindowProps {
            level: WindowLevel::ScreenSaver,
            visible_on_all_workspaces: true,
            ..Default::default()
        };
        let mut runtime = build_runtime(component, &props, None, &[]);
        assert!(runtime.main_window_always_on_top());
        for first in [true, false] {
            let mac = runtime.main_window_steps(Backend::MacOs, first);
            assert!(
                after_create_actions(mac, true).contains(&AfterCreate::MacPlacement {
                    level: Some(1000),
                    collection: 1 | 256,
                })
            );
            let x11 = runtime.main_window_steps(Backend::X11, first);
            assert!(after_create_actions(x11, true).contains(&AfterCreate::ReassertOnTop));
            for backend in [Backend::Windows, Backend::Wayland, Backend::MacOs] {
                let steps = runtime.main_window_steps(backend, first);
                let attrs = runtime.main_window_attributes(steps, backend, first, first);
                assert_eq!(
                    attrs.window_level,
                    winit::window::WindowLevel::AlwaysOnTop,
                    "{backend:?}"
                );
            }
        }
        let plain = build_runtime(component, &Default::default(), None, &[]);
        assert!(!plain.main_window_always_on_top());
    }

    /// #1519, the default path: an ordinary window takes no hit-test call and
    /// no extra step, and a tray re-show is not moved to `WindowProps::x/y`
    /// (it never was); only the first creation is placed.
    #[test]
    fn an_ordinary_window_takes_the_old_path() {
        use super::overlay_window::after_create_actions;
        let props = rinch_core::element::WindowProps {
            x: Some(40),
            y: Some(50),
            ..Default::default()
        };
        let mut runtime = build_runtime(component, &props, None, &[]);
        for backend in [
            Backend::X11,
            Backend::Wayland,
            Backend::Windows,
            Backend::MacOs,
        ] {
            let steps = runtime.main_window_steps(backend, true);
            assert!(after_create_actions(steps, false).is_empty(), "{backend:?}");
            assert!(after_create_actions(steps, true).is_empty(), "{backend:?}");
        }
        let steps = runtime.main_window_steps(Backend::Wayland, true);
        let first = runtime.main_window_attributes(steps, Backend::Wayland, true, true);
        assert!(first.position.is_some(), "the first creation is placed");
        let reshown = runtime.main_window_attributes(steps, Backend::Wayland, false, false);
        assert!(
            reshown.position.is_none(),
            "a re-show with no recorded geometry is placed by the system"
        );
        assert!(first.active && first.visible && reshown.active && reshown.visible);
    }

    #[test]
    fn build_runtime_carries_both_the_window_props_and_the_bundled_fonts() {
        let props = rinch_core::element::WindowProps {
            title: "Carried".into(),
            width: 1024,
            height: 768,
            app_id: Some("com.example.carried".into()),
            ..Default::default()
        };

        let runtime = build_runtime(component, &props, None, &[AppFont::monospace(FACE)]);

        assert_eq!(
            runtime.app.pending_fonts.len(),
            1,
            "the bundled face must reach the app the window will mount"
        );
        assert_eq!(
            runtime.app.pending_fonts[0].generics[0],
            parley::fontique::GenericFamily::Monospace,
            "and reach it with the generic it declared"
        );
        assert_eq!((runtime.width, runtime.height), (1024, 768));
        assert_eq!(runtime.title, "Carried");
        let carried = runtime
            .app
            .window_props
            .as_ref()
            .expect("window props must reach the app");
        assert_eq!(carried.app_id.as_deref(), Some("com.example.carried"));
    }
}

#[cfg(test)]
mod native_event_queue_tests {
    use super::*;

    fn rerenders(events: &[RinchNativeEvent]) -> usize {
        events
            .iter()
            .filter(|e| matches!(e, RinchNativeEvent::ReRender))
            .count()
    }

    /// Any number of `ReRender`s between two drains queue one event, and only
    /// the first asks for a wake.
    #[test]
    fn rerenders_coalesce_until_the_queue_is_drained() {
        let mut q = NativeEventQueue::new();
        let accepted: Vec<bool> = (0..7).map(|_| q.push(RinchNativeEvent::ReRender)).collect();
        assert_eq!(accepted, [true, false, false, false, false, false, false]);
        assert_eq!(rerenders(&q.drain()), 1);

        // Drained: the next one queues again, and again coalesces its followers.
        assert!(q.push(RinchNativeEvent::ReRender));
        assert!(!q.push(RinchNativeEvent::ReRender));
        assert_eq!(rerenders(&q.drain()), 1);
        assert!(q.is_empty());
    }

    /// Through the real entry point: `rerender_events_queued` (what
    /// `perf_stats` reports) counts the events actually queued, so a burst of
    /// signal flushes between two drains counts one. The only test in the
    /// process that sends a `ReRender` through the static queue.
    #[test]
    fn send_native_event_counts_one_rerender_per_drain() {
        use std::sync::atomic::Ordering;
        let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        let before = crate::app::RERENDER_EVENTS_QUEUED.load(Ordering::Relaxed);
        for _ in 0..6 {
            send_native_event(RinchNativeEvent::ReRender);
        }
        assert_eq!(
            crate::app::RERENDER_EVENTS_QUEUED.load(Ordering::Relaxed) - before,
            1
        );
        assert_eq!(rerenders(&NATIVE_EVENT_QUEUE.lock().unwrap().drain()), 1);
        send_native_event(RinchNativeEvent::ReRender);
        assert_eq!(
            crate::app::RERENDER_EVENTS_QUEUED.load(Ordering::Relaxed) - before,
            2,
            "a drain re-arms it"
        );
        let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
    }

    /// Issue #988. A second thread's `run_on_main_thread` lands between a
    /// wake's two halves: after the main-thread queue was emptied (so
    /// `queue_main_callback` answers "was empty" and it sends a `ReRender`)
    /// and before the native drain (so that `ReRender` coalesces into the one
    /// that caused this wake, and asks for no wake of its own). The drain then
    /// consumes the old `ReRender`, and nothing is left to wake the loop for
    /// the new callback.
    ///
    /// Two injection points, both inside that window: the end of the queued
    /// half, and the start of the native half (before its drain takes the
    /// lock). The second is what a fix that samples "is anything queued?"
    /// *before* the native drain gets wrong.
    ///
    /// The invariant: once the wake returns, the injected callback has either
    /// run or has a wake owed (a `ReRender` in the queue — which is what
    /// `send_native_event` wakes the loop for). "Or has run" keeps the fixture
    /// honest if another test in this process drains the shared main queue in
    /// the meantime; that drain runs it, which is not a lost wake.
    fn callback_queued_mid_wake_keeps_its_wake(inject_in_native_half: bool) {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        // The wake being served: one queued callback and its ReRender.
        run_on_main_thread(|| {});
        NATIVE_EVENT_QUEUE
            .lock()
            .unwrap()
            .push(RinchNativeEvent::ReRender);

        let injected_ran = Arc::new(AtomicBool::new(false));
        let flag = injected_ran.clone();
        // Another thread, inside the window.
        let inject = move || {
            std::thread::spawn(move || {
                run_on_main_thread(move || flag.store(true, Ordering::SeqCst))
            })
            .join()
            .unwrap();
        };
        let mut inject = Some(inject);
        drain_wake_queues(
            &mut inject,
            |inject| {
                rinch_core::drain_main_callbacks();
                if !inject_in_native_half {
                    (inject.take().unwrap())();
                }
            },
            |inject| {
                if inject_in_native_half {
                    (inject.take().unwrap())();
                }
                let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
            },
        );
        assert!(inject.is_none(), "the injection ran");

        let wake_owed = rerenders(&NATIVE_EVENT_QUEUE.lock().unwrap().drain()) > 0;
        if !wake_owed {
            // Only another test's drain may have taken it; give that drain
            // time to finish running it.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
            while !injected_ran.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                std::thread::yield_now();
            }
        }
        let ran = injected_ran.load(Ordering::SeqCst);
        // Leave the shared queue as we found it.
        rinch_core::drain_main_callbacks();
        assert!(
            wake_owed || ran,
            "the callback queued mid-wake is stranded: still queued, no wake owed"
        );
    }

    #[test]
    fn a_callback_queued_after_the_callback_drain_keeps_its_wake() {
        callback_queued_mid_wake_keeps_its_wake(false);
    }

    #[test]
    fn a_callback_queued_as_the_native_drain_starts_keeps_its_wake() {
        callback_queued_mid_wake_keeps_its_wake(true);
    }

    /// Issue #1733: what made the two tests above flaky. Dropping a
    /// [`RinchRuntime`] runs its shutdown, which **drops** every queued
    /// main-thread callback without running it (`clear_main_callbacks`) — and
    /// the runtime-building tests in this binary did that on other threads
    /// while a wake test's injected callback sat in the shared queue: neither
    /// run nor owed a wake. A real process has one runtime, dropped at exit,
    /// so only the test binary had the race. Every test runtime now holds
    /// `RERENDER_EVENTS_TEST_LOCK` for its lifetime (`RinchRuntime::_queue_lock`).
    ///
    /// The runtime here is built and dropped on another thread, inside the
    /// wake test's window: before the fix its drop landed there every time.
    #[test]
    fn a_runtime_dropped_on_another_thread_mid_wake_strands_nothing() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        rinch_core::drain_main_callbacks();
        run_on_main_thread(|| {});
        let injected_ran = Arc::new(AtomicBool::new(false));
        let flag = injected_ran.clone();
        let dropped = Arc::new(AtomicBool::new(false));
        let dropped_flag = dropped.clone();
        let mut dropper = None;
        drain_wake_queues(
            &mut (),
            |_| {
                rinch_core::drain_main_callbacks();
                run_on_main_thread(move || flag.store(true, Ordering::SeqCst));
                dropper = Some(std::thread::spawn(move || {
                    drop(RinchRuntime::new(
                        "t",
                        800,
                        600,
                        |s: &mut RenderScope| s.create_element("div"),
                        &[],
                    ));
                    dropped_flag.store(true, Ordering::SeqCst);
                }));
                // Long enough for an unserialised runtime to be built and
                // dropped; a serialised one waits for this test's lock.
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
                while !dropped.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            },
            |_| {
                let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
            },
        );
        let wake_owed = rerenders(&NATIVE_EVENT_QUEUE.lock().unwrap().drain()) > 0;
        let ran = injected_ran.load(Ordering::SeqCst);
        rinch_core::drain_main_callbacks();
        drop(_lock);
        dropper.unwrap().join().unwrap();
        assert!(
            wake_owed || ran,
            "a runtime dropped mid-wake threw away the queued callback"
        );
    }

    /// Why every dispatch asks for a `ReRender` (#1540) rather than only a push
    /// onto an empty main queue: the queue can be non-empty with no wake owed
    /// for it — a closure pushed past the dispatcher (#1035's shape), or one
    /// queued plainly before the dispatcher existed (#1541) — and under the
    /// empty-queue rule every later sender, from any thread, then found it
    /// non-empty and asked for nothing. Run through both senders the shell
    /// installs: the dispatcher behind `Signal::send`, and the shell's own
    /// `run_on_main_thread`.
    fn a_dispatch_behind_an_unwoken_callback_still_wakes(send: fn()) {
        let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        rinch_core::drain_main_callbacks();
        // A callback with no wake of its own.
        rinch_core::queue_main_callback(Box::new(|| {}));
        // The next write from a worker.
        std::thread::spawn(send).join().unwrap();
        let wake_owed = rerenders(&NATIVE_EVENT_QUEUE.lock().unwrap().drain()) > 0;
        let still_queued = rinch_core::main_callbacks_pending();
        rinch_core::drain_main_callbacks();
        assert!(
            wake_owed || !still_queued,
            "cross-thread writes stranded: queued, and no wake owed"
        );
    }

    #[test]
    fn a_signal_send_behind_an_unwoken_callback_still_wakes() {
        a_dispatch_behind_an_unwoken_callback_still_wakes(|| {
            dispatch_to_main_thread(Box::new(|| {}))
        });
    }

    #[test]
    fn a_run_on_main_thread_behind_an_unwoken_callback_still_wakes() {
        a_dispatch_behind_an_unwoken_callback_still_wakes(|| run_on_main_thread(|| {}));
    }

    /// Issue #1540's fix asks for a `ReRender` on every dispatch; the
    /// coalescing #988 / #1035 rely on still holds: a burst of dispatches
    /// between two native drains leaves exactly one pending `ReRender`, and
    /// counts one in `rerender_events_queued`.
    #[test]
    fn a_burst_of_dispatches_leaves_one_rerender_per_drain() {
        use std::sync::atomic::Ordering;
        let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        for round in 0..2 {
            let before = crate::app::RERENDER_EVENTS_QUEUED.load(Ordering::Relaxed);
            std::thread::spawn(|| {
                for _ in 0..25 {
                    dispatch_to_main_thread(Box::new(|| {}));
                    run_on_main_thread(|| {});
                }
            })
            .join()
            .unwrap();
            assert_eq!(
                crate::app::RERENDER_EVENTS_QUEUED.load(Ordering::Relaxed) - before,
                1,
                "round {round}: one ReRender queued for 50 dispatches"
            );
            assert_eq!(rerenders(&NATIVE_EVENT_QUEUE.lock().unwrap().drain()), 1);
        }
        rinch_core::drain_main_callbacks();
    }

    /// Issue #1541: a callback queued plainly before the dispatcher existed
    /// is given a `ReRender`, and the wake is paid, once the proxy is set.
    #[test]
    fn work_queued_before_the_proxy_is_owed_a_wake() {
        let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        // Queued plainly, as `dispatch_main_callback` does with no dispatcher.
        rinch_core::queue_main_callback(Box::new(|| {}));
        let mut woke = 0;
        owe_startup_wake(|| woke += 1);
        let owed = rerenders(&NATIVE_EVENT_QUEUE.lock().unwrap().drain());
        let still_queued = rinch_core::main_callbacks_pending();
        rinch_core::drain_main_callbacks();
        assert!(
            (owed == 1 && woke == 1) || !still_queued,
            "queued before the proxy, and no wake owed or paid (owed {owed}, woke {woke})"
        );
    }

    /// Issue #1540: a `ReRender` sent before the proxy existed (the mount's
    /// signal-change callback, an early `Signal::send`) carried no wake and
    /// holds the coalescing flag up, so every later `ReRender` coalesces into
    /// it and wakes nothing. A window that paints drains it; a windowless or
    /// hidden app never does. The wake owed once the proxy is set is what
    /// clears it — and it is the `wake` call, not a further `ReRender`, that
    /// does: one more `ReRender` coalesces into the stale one.
    #[test]
    fn a_rerender_queued_before_the_proxy_is_woken_for() {
        let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        rinch_core::drain_main_callbacks();
        // The stale `ReRender`, flag up, nothing woken.
        send_native_event(RinchNativeEvent::ReRender);
        // A worker's write behind it: coalesced, no wake of its own.
        std::thread::spawn(|| dispatch_to_main_thread(Box::new(|| {})))
            .join()
            .unwrap();
        let mut woke = 0;
        let owed = owe_startup_wake(|| woke += 1);
        let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        rinch_core::drain_main_callbacks();
        assert!(owed, "a ReRender is waiting");
        assert_eq!(woke, 1, "and the loop is woken for it");
    }

    /// Only `ReRender` coalesces: every other event is a distinct request, and
    /// the `ReRender` keeps the position it was first queued at.
    #[test]
    fn other_events_are_never_coalesced() {
        let mut q = NativeEventQueue::new();
        assert!(q.push(RinchNativeEvent::MinimizeWindow));
        assert!(q.push(RinchNativeEvent::ReRender));
        assert!(q.push(RinchNativeEvent::MinimizeWindow));
        assert!(!q.push(RinchNativeEvent::ReRender));
        assert!(q.push(RinchNativeEvent::ShowWindow));
        let drained = q.drain();
        assert_eq!(drained.len(), 4, "{drained:?}");
        assert!(matches!(drained[1], RinchNativeEvent::ReRender));
        assert_eq!(rerenders(&drained), 1);
    }
}

/// winit's pointer sources as a render surface hears them
/// (`SurfacePointer`): which device, which id, how hard.
#[cfg(test)]
mod pointer_source_tests {
    use super::*;
    use winit::event::{
        ButtonSource, FingerId, Force, PointerSource, TabletToolButton, TabletToolData,
        TabletToolKind,
    };

    fn tool(kind: TabletToolKind, force: Option<f64>) -> PointerSource {
        PointerSource::TabletTool {
            kind,
            data: TabletToolData {
                force: force.map(Force::Normalized),
                ..Default::default()
            },
        }
    }

    /// A chord's release leaves the held button down; a window blur forgets
    /// every press, since the release of a button held across it may go to
    /// another window (review of #1502, N3).
    #[test]
    fn pressed_buttons_track_chords_and_a_blur_forgets_them() {
        let mut p = PressedButtons::default();
        p.note(1, Some(MouseButton::Left), true);
        p.note(1, Some(MouseButton::Right), true);
        p.note(1, Some(MouseButton::Right), false);
        assert!(p.is_down(1), "the left button is still held");
        p.note(1, Some(MouseButton::Left), false);
        assert!(!p.is_down(1));
        p.note(1, Some(MouseButton::Left), true);
        p.note(TOUCH_POINTER_BASE, None, true);
        p.clear();
        assert!(!p.is_down(1) && !p.is_down(TOUCH_POINTER_BASE));
        p.note(TOUCH_POINTER_BASE, None, true);
        assert!(p.lift(TOUCH_POINTER_BASE) && !p.lift(TOUCH_POINTER_BASE));
    }

    #[test]
    fn a_pen_reports_its_force_and_an_eraser_says_so() {
        let p = surface_pointer_of(&tool(TabletToolKind::Pen, Some(0.3)), true, true);
        assert_eq!(p.kind, SurfacePointerKind::Pen);
        assert!((p.pressure - 0.3).abs() < 1e-6);
        assert_eq!(p.id, TABLET_POINTER_ID);
        let e = surface_pointer_of(&tool(TabletToolKind::Eraser, Some(1.5)), true, true);
        assert_eq!(e.kind, SurfacePointerKind::Eraser);
        assert_eq!(e.pressure, 1.0, "clamped");
        // A tool without force sensing reports the Pointer Events default.
        assert_eq!(
            surface_pointer_of(&tool(TabletToolKind::Pen, None), true, true).pressure,
            0.5
        );
        assert_eq!(
            surface_pointer_of(&tool(TabletToolKind::Pen, None), true, false).pressure,
            0.0
        );
    }

    #[test]
    fn each_finger_is_its_own_pointer_and_never_the_mouse() {
        let finger = |raw: usize| PointerSource::Touch {
            finger_id: FingerId::from_raw(raw),
            force: None,
        };
        let a = surface_pointer_of(&finger(0), true, true);
        let b = surface_pointer_of(&finger(1), false, true);
        assert_eq!(a.kind, SurfacePointerKind::Touch);
        assert_ne!(a.id, b.id);
        assert!(a.id != SurfacePointer::mouse(true).id && a.id != TABLET_POINTER_ID);
        // No finger's id is the mouse's or the tablet's, the first two
        // included (a touch base of 0 gives finger 1 the mouse's id).
        for raw in 0..4 {
            let id = surface_pointer_of(&finger(raw), false, true).id;
            assert!(id != SurfacePointer::mouse(true).id && id != TABLET_POINTER_ID);
        }
        assert_eq!(a.id, pointer_id_of(&finger(0)));
        assert!(a.primary && !b.primary);
        assert_eq!(a.pressure, 0.5, "no force measured, contact down");
    }

    #[test]
    fn the_mouse_is_pointer_one_and_a_release_has_no_pressure() {
        let m = surface_pointer_of(&PointerSource::Mouse, true, true);
        assert_eq!(m, SurfacePointer::mouse(true));
        let pressed = surface_pointer_of_button(
            &ButtonSource::TabletTool {
                kind: TabletToolKind::Pen,
                button: TabletToolButton::Contact,
                data: TabletToolData {
                    force: Some(Force::Normalized(0.8)),
                    ..Default::default()
                },
            },
            true,
            true,
        );
        assert!((pressed.pressure - 0.8).abs() < 1e-6);
        let released =
            surface_pointer_of_button(&ButtonSource::Mouse(MouseButton::Left), true, false);
        assert_eq!(released.pressure, 0.0);
        assert_eq!(released.id, 1);
    }
}

#[cfg(test)]
mod layer_drag_tests {
    use super::*;
    use rinch_core::element::WindowProps;

    fn component(scope: &mut RenderScope) -> NodeHandle {
        scope.create_element("div")
    }

    /// The runtime's half of a layer drag (#1523), with no window: each
    /// pointer position is followed against where the compositor has the
    /// surface, which only `layer_moved` changes. The compositor here holds
    /// `lag` margin commits before it processes the oldest and reports it.
    fn drag(lag: usize) -> (f64, f64) {
        let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        let start = (80.0, 60.0);
        let grab = (16.0, 22.0);
        let mut layer = LayerState::new(Some(start));
        layer.gesture = Some(Gesture::start(GestureKind::Move, grab, start, (84.0, 84.0)));
        runtime.layer = Some(layer);
        let mut origin = start;
        let mut commits = std::collections::VecDeque::new();
        let mut pointer = (start.0 + grab.0, start.1 + grab.1);
        for _ in 0..60 {
            pointer = (pointer.0 + 10.0, pointer.1 + 6.0);
            let before = runtime.layer.unwrap().position;
            let resize = runtime.follow_layer_gesture((pointer.0 - origin.0, pointer.1 - origin.1));
            assert_eq!(resize, Some(None), "a move asks for no size");
            let after = runtime.layer.unwrap().position;
            if after != before {
                commits.push_back(after.unwrap());
            }
            while commits.len() > lag {
                origin = commits.pop_front().unwrap();
                runtime.layer_moved((origin.0 as i32, origin.1 as i32));
            }
        }
        while let Some(o) = commits.pop_front() {
            origin = o;
            runtime.layer_moved((o.0 as i32, o.1 as i32));
        }
        assert_eq!(runtime.layer.unwrap().position, Some(origin));
        origin
    }

    #[test]
    fn a_layer_drag_lands_under_the_pointer_at_every_lag() {
        for lag in [0, 1, 4, 60] {
            assert_eq!(drag(lag), (680.0, 420.0), "lag {lag}");
        }
    }

    #[test]
    fn no_gesture_follows_nothing() {
        let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        assert_eq!(runtime.follow_layer_gesture((5.0, 5.0)), None, "no layer");
        runtime.layer = Some(LayerState::new(Some((10.0, 10.0))));
        assert_eq!(runtime.follow_layer_gesture((5.0, 5.0)), None, "no gesture");
        assert_eq!(runtime.layer.unwrap().position, Some((10.0, 10.0)));
    }

    #[test]
    fn a_layer_resize_asks_for_its_size() {
        let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        let mut layer = LayerState::new(Some((100.0, 100.0)));
        layer.gesture = Some(Gesture::start(
            GestureKind::Resize(rinch_platform::ResizeDirection::East),
            (84.0, 40.0),
            (100.0, 100.0),
            (84.0, 84.0),
        ));
        runtime.layer = Some(layer);
        assert_eq!(
            runtime.follow_layer_gesture((94.0, 40.0)),
            Some(Some((94.0, 84.0)))
        );
    }

    #[test]
    fn only_a_transparent_window_gets_a_blending_gpu_surface() {
        let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
        assert!(!runtime.gpu_surface_transparent());
        runtime.app.window_props = Some(WindowProps {
            transparent: true,
            ..Default::default()
        });
        assert!(runtime.gpu_surface_transparent());
    }
}

/// A `WindowProps::x`/`y` the window can be reached at.
#[derive(Debug, Clone, PartialEq)]
struct RequestedPosition {
    /// As asked, logical pixels: what an app reads back.
    logical: (f64, f64),
    /// What winit is given.
    at: Position,
    /// The monitor it was read in the scale of, by name.
    monitor: Option<String>,
}

/// The platform's name for the monitor `window` is on (whose scale it has):
/// exactly what `MonitorHandle::name` answers, so a name published with a
/// position finds the same monitor in `connected_monitors`.
fn monitor_name(window: &dyn winit::window::Window) -> Option<String> {
    window
        .current_monitor()
        .and_then(|m| m.name().map(|n| n.into_owned()))
}

/// The monitors connected now, in the unit windows are positioned in:
/// desktop pixels, or points on macOS (given scale 1). Each monitor's
/// rectangle is the platform's own where rinch can read it
/// ([`native_monitor_rect`]): winit gives only a position and the current
/// video mode, whose size on X11 ignores rotation.
fn connected_monitors(event_loop: &dyn ActiveEventLoop) -> Vec<window_geometry::MonitorRect> {
    let backend = Backend::of(event_loop);
    event_loop
        .available_monitors()
        .filter_map(|m| {
            let mode = m.current_video_mode().map(|v| v.size());
            let position = m.position();
            let scale = m.scale_factor();
            let name = m.name().map(|n| n.into_owned());
            let native = native_monitor_rect(&m, backend);
            if cfg!(target_os = "macos") {
                // Points, the unit AppKit positions windows in.
                let (p, s) = match native {
                    Some(rect) => rect,
                    None => {
                        let p = position?.to_logical::<f64>(scale);
                        let s = mode?.to_logical::<f64>(scale);
                        (
                            winit::dpi::PhysicalPosition::new(
                                p.x.round() as i32,
                                p.y.round() as i32,
                            ),
                            winit::dpi::PhysicalSize::new(
                                s.width.round() as u32,
                                s.height.round() as u32,
                            ),
                        )
                    }
                };
                return Some(window_geometry::MonitorRect::of(p, s, 1.0, name, None));
            }
            if native.is_none() && (position.is_none() || mode.is_none()) {
                return None;
            }
            Some(window_geometry::MonitorRect::of(
                position.unwrap_or_default(),
                mode.unwrap_or_default(),
                scale,
                name,
                native,
            ))
        })
        .collect()
}

/// A monitor's rectangle on the desktop as the platform keeps it: the RandR
/// CRTC on X11 (rotation included), `rcMonitor` on Windows (physical
/// desktop pixels), `CGDisplayBounds` on macOS (points). `None` where it
/// cannot be read, and on Wayland, which gives no desktop coordinates.
#[allow(unused_variables)]
fn native_monitor_rect(
    m: &winit::monitor::MonitorHandle,
    backend: Backend,
) -> Option<(
    winit::dpi::PhysicalPosition<i32>,
    winit::dpi::PhysicalSize<u32>,
)> {
    use winit::dpi::{PhysicalPosition, PhysicalSize};
    #[cfg(target_os = "linux")]
    if backend == Backend::X11 {
        let (x, y, w, h) = overlay_window::x11_crtc_rect(m.native_id() as u32)?;
        return Some((PhysicalPosition::new(x, y), PhysicalSize::new(w, h)));
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO};
        // SAFETY: a zeroed MONITORINFO with its size set is what
        // GetMonitorInfoW fills; the handle is winit's HMONITOR.
        let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let ok = unsafe { GetMonitorInfoW(m.native_id() as usize as HMONITOR, &mut info) };
        if ok == 0 {
            return None;
        }
        let r = info.rcMonitor;
        return Some((
            PhysicalPosition::new(r.left, r.top),
            PhysicalSize::new(
                (r.right - r.left).max(0) as u32,
                (r.bottom - r.top).max(0) as u32,
            ),
        ));
    }
    #[cfg(target_os = "macos")]
    {
        #[repr(C)]
        struct CGPoint {
            x: f64,
            y: f64,
        }
        #[repr(C)]
        struct CGSize {
            width: f64,
            height: f64,
        }
        #[repr(C)]
        struct CGRect {
            origin: CGPoint,
            size: CGSize,
        }
        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            fn CGDisplayBounds(display: u32) -> CGRect;
        }
        // SAFETY: CGDisplayBounds takes any display id (an unknown one gives
        // an empty rect) and returns a plain struct.
        let r = unsafe { CGDisplayBounds(m.native_id() as u32) };
        if r.size.width <= 0.0 || r.size.height <= 0.0 {
            return None;
        }
        return Some((
            PhysicalPosition::new(r.origin.x.round() as i32, r.origin.y.round() as i32),
            PhysicalSize::new(r.size.width.round() as u32, r.size.height.round() as u32),
        ));
    }
    #[allow(unreachable_code)]
    None
}

/// #1884: the runtime's half of bringing a window back — the requested
/// position filtered by reachability, a saved one dropped at a show, the
/// creation geometry published on a first appearance (a hidden start's
/// first show included), and readings taken through the maximize wait.
#[cfg(test)]
mod window_restore_tests {
    use super::window_geometry::{MonitorRect, WindowReading, published_test_guard};
    use super::*;
    use crate::windows::{WindowRestore, current_window_restore};
    use rinch_core::element::WindowProps;
    use winit::dpi::{PhysicalPosition, PhysicalSize};

    fn component(scope: &mut RenderScope) -> NodeHandle {
        scope.create_element("div")
    }

    fn one_monitor() -> Vec<MonitorRect> {
        vec![MonitorRect {
            position: PhysicalPosition::new(0, 0),
            size: PhysicalSize::new(1920, 1080),
            scale: 1.0,
            name: None,
        }]
    }

    fn runtime(props: WindowProps) -> RinchRuntime {
        build_runtime(
            component,
            &WindowProps {
                width: 800,
                height: 600,
                ..props
            },
            None,
            &[],
        )
    }

    fn attributes(runtime: &mut RinchRuntime, first_appearance: bool) -> WindowAttributes {
        let steps = runtime.main_window_steps(Backend::X11, true);
        runtime.main_window_attributes(steps, Backend::X11, false, first_appearance)
    }

    impl RinchRuntime {
        fn take_reading_unnamed(&mut self, r: WindowReading) {
            self.take_reading(r, None);
        }
    }

    fn reading(maximized: bool, pos: (i32, i32), size: (u32, u32)) -> WindowReading {
        WindowReading {
            maximized,
            minimized: false,
            scale: 1.0,
            position: Some(PhysicalPosition::new(pos.0, pos.1)),
            surface: PhysicalSize::new(size.0, size.1),
        }
    }

    /// The tray-app path: `visible: false` + `maximized: true`. The first
    /// show is the first appearance, so it publishes the geometry the window
    /// is created with; without it the restore answered the maximized size
    /// and no position (Windows maximizes at creation, so the first reading
    /// is already maximized).
    #[test]
    fn a_hidden_start_publishes_the_restore_geometry_at_its_first_show() {
        let _guard = published_test_guard();
        let mut rt = runtime(WindowProps {
            visible: false,
            maximized: true,
            x: Some(100),
            y: Some(80),
            ..Default::default()
        });
        assert!(
            !rt.begin_appearance(one_monitor(), false),
            "the sizing window"
        );
        assert_eq!(current_window_restore(), None, "nothing on screen yet");
        assert!(rt.begin_appearance(one_monitor(), true), "the first show");
        let attrs = attributes(&mut rt, true);
        assert!(attrs.maximized);
        assert_eq!(
            attrs.position,
            Some(Position::Physical(PhysicalPosition::new(100, 80)))
        );
        rt.maximize_wait =
            window_geometry::MaximizeWait::after_create(&attrs, false, std::time::Instant::now());
        rt.take_reading_unnamed(reading(true, (0, 0), (1920, 1050)));
        assert_eq!(
            current_window_restore(),
            Some(WindowRestore {
                position: Some((100.0, 80.0)),
                size: (800.0, 600.0),
                maximized: true,
                monitor: None,
            })
        );
        // A later show is no first appearance and publishes nothing new.
        rt.shown_once = true;
        assert!(!rt.begin_appearance(one_monitor(), true));
    }

    /// X11: the window manager maximizes later, and a resize can come while
    /// the window does not say it is maximized yet. The restore keeps the
    /// creation geometry and `maximized: true`.
    #[test]
    fn a_resize_before_the_maximize_is_reported_is_not_the_restore_geometry() {
        let _guard = published_test_guard();
        let mut rt = runtime(WindowProps {
            maximized: true,
            x: Some(100),
            y: Some(80),
            ..Default::default()
        });
        assert!(rt.begin_appearance(one_monitor(), true));
        let attrs = attributes(&mut rt, true);
        rt.maximize_wait =
            window_geometry::MaximizeWait::after_create(&attrs, false, std::time::Instant::now());
        rt.take_reading_unnamed(reading(false, (100, 80), (800, 600)));
        rt.take_reading_unnamed(reading(false, (0, 0), (1920, 1050)));
        assert_eq!(
            current_window_restore(),
            Some(WindowRestore {
                position: Some((100.0, 80.0)),
                size: (800.0, 600.0),
                maximized: true,
                monitor: None,
            })
        );
        assert_eq!(rt.saved_geometry.size, None, "no normal geometry recorded");
    }

    /// `WindowProps::x`/`y` out of reach (a monitor unplugged since it was
    /// saved) is not given to winit; in reach it is.
    #[test]
    fn a_requested_position_out_of_reach_is_left_to_the_system() {
        let _guard = published_test_guard();
        let mut gone = runtime(WindowProps {
            x: Some(5000),
            y: Some(100),
            ..Default::default()
        });
        gone.begin_appearance(one_monitor(), true);
        assert_eq!(gone.requested_position(), None);
        assert_eq!(attributes(&mut gone, true).position, None);
        assert_eq!(current_window_restore().and_then(|r| r.position), None);
    }

    /// And one in reach is, in desktop pixels, and is what the restore
    /// answers.
    #[test]
    fn a_requested_position_in_reach_is_taken() {
        let _guard = published_test_guard();
        let mut here = runtime(WindowProps {
            x: Some(300),
            y: Some(200),
            ..Default::default()
        });
        here.begin_appearance(one_monitor(), true);
        assert_eq!(
            attributes(&mut here, true).position,
            Some(Position::Physical(PhysicalPosition::new(300, 200)))
        );
        assert_eq!(
            current_window_restore().and_then(|r| r.position),
            Some((300.0, 200.0))
        );
    }

    /// A show after a hide drops a saved position out of reach on the
    /// monitors connected now, and keeps one in reach.
    #[test]
    fn a_show_drops_a_saved_position_out_of_reach() {
        let _guard = published_test_guard();
        let mut rt = runtime(WindowProps::default());
        rt.shown_once = true;
        let gone = Position::Physical(PhysicalPosition::new(5000, 100));
        rt.saved_geometry.position = Some(gone);
        assert!(!rt.begin_appearance(one_monitor(), true));
        assert_eq!(rt.saved_geometry.position, None);

        let kept = Position::Physical(PhysicalPosition::new(300, 200));
        rt.saved_geometry.position = Some(kept);
        rt.begin_appearance(one_monitor(), true);
        assert_eq!(rt.saved_geometry.position, Some(kept));
        // With no saved size, the window's own size decides: 800 wide at
        // x = 1880 leaves 40 columns on screen.
        let sliver = Position::Physical(PhysicalPosition::new(1880, 200));
        rt.saved_geometry.position = Some(sliver);
        rt.begin_appearance(one_monitor(), true);
        assert_eq!(rt.saved_geometry.position, None);
    }

    /// #1884: `WindowProps::monitor` settles a mixed-scale position — A
    /// (scale 1) at x 0..1920, B (scale 2) at 1920..5760; (1050, 50) is on
    /// both — and the restore answers the monitor it was placed on.
    #[test]
    fn the_requested_monitor_places_the_window_and_is_published() {
        let _guard = published_test_guard();
        let mons = || {
            vec![
                MonitorRect {
                    position: PhysicalPosition::new(0, 0),
                    size: PhysicalSize::new(1920, 1080),
                    scale: 1.0,
                    name: Some("A".to_string()),
                },
                MonitorRect {
                    position: PhysicalPosition::new(1920, 0),
                    size: PhysicalSize::new(3840, 2160),
                    scale: 2.0,
                    name: Some("B".to_string()),
                },
            ]
        };
        let mut rt = runtime(WindowProps {
            x: Some(1050),
            y: Some(50),
            monitor: Some("B".to_string()),
            ..Default::default()
        });
        assert!(rt.begin_appearance(mons(), true));
        assert_eq!(
            attributes(&mut rt, true).position,
            Some(Position::Physical(PhysicalPosition::new(2100, 100)))
        );
        let restore = current_window_restore().unwrap();
        assert_eq!(
            (restore.position, restore.monitor.as_deref()),
            (Some((1050.0, 50.0)), Some("B"))
        );
    }

    /// A position the app sets while the window is hidden is on no monitor
    /// rinch can name: the published name goes with the old position.
    #[test]
    fn a_position_set_while_hidden_drops_the_monitor_name() {
        let _guard = published_test_guard();
        window_geometry::publish_window(
            &reading(false, (100, 80), (800, 600)),
            Some("A".to_string()),
        );
        assert_eq!(
            current_window_restore().unwrap().monitor.as_deref(),
            Some("A")
        );
        window_geometry::publish_hidden(Some((300.0, 200.0)), None);
        assert_eq!(current_window_restore().unwrap().monitor, None);
    }

    /// #1884 re-review (D1): the window manager never maximizes a window
    /// created maximized; it is dragged inside the grace and then left
    /// alone, so no event reads it again. The reading taken when the grace
    /// ends records the move and that it is not maximized; after it the
    /// wait has no deadline, so no timed wake is armed for it again.
    #[test]
    fn the_reading_at_the_end_of_the_grace_records_a_refused_maximize() {
        let _guard = published_test_guard();
        let mut rt = runtime(WindowProps {
            maximized: true,
            x: Some(100),
            y: Some(80),
            ..Default::default()
        });
        assert!(rt.begin_appearance(one_monitor(), true));
        let attrs = attributes(&mut rt, true);
        rt.maximize_wait =
            window_geometry::MaximizeWait::after_create(&attrs, false, std::time::Instant::now());
        rt.take_reading_unnamed(reading(false, (100, 80), (800, 600)));
        rt.take_reading_unnamed(reading(false, (700, 400), (800, 600)));
        let restore = current_window_restore().unwrap();
        assert!(restore.maximized, "inside the grace: still waiting");
        assert!(rt.maximize_wait.deadline().is_some(), "a wake is due");
        rt.settle_maximize_wait_with(Some((reading(false, (700, 400), (800, 600)), None)));
        let restore = current_window_restore().unwrap();
        assert_eq!(
            (restore.maximized, restore.position),
            (false, Some((700.0, 400.0)))
        );
        assert_eq!(rt.maximize_wait.deadline(), None, "no wake after it");
        // A window that is not shown (or none) still ends the wait.
        rt.maximize_wait =
            window_geometry::MaximizeWait::after_create(&attrs, false, std::time::Instant::now());
        rt.window = None;
        rt.settle_maximize_wait();
        assert_eq!(rt.maximize_wait.deadline(), None);
    }
}
