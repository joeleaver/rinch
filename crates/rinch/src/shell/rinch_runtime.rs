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
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::window::{WindowAttributes, WindowId};

use rinch_core::clear_context;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::events;
#[cfg(feature = "gpu-renderer")]
use rinch_platform::PlatformRenderer;
use rinch_platform::{
    AppAction, ImeEvent, KeyCode, KeyRepeat, Modifiers, MouseButton as PlatformMouseButton,
    PlatformEvent, PlatformWindow, UserEvent, to_logical, to_logical_point,
};

use crate::app::RinchApp;
use crate::render_surface::{SurfacePointer, SurfacePointerKind};

#[cfg(feature = "gpu-renderer")]
use super::desktop::WgpuRenderer;
use super::layer_shell::{Gesture, GestureKind, LayerSpec, LayerState, WindowSurfaceKind};
use super::outputs::connected_monitors;
use super::overlay_window::{self, Backend, WindowSteps};
use super::window::WinitWindow;
use super::window_geometry::{self, SavedGeometry};

#[cfg(feature = "debug")]
use {super::screenshot, base64::Engine, rinch_debug::DebugResult};

#[cfg(feature = "gpu-renderer")]
use super::desktop;
use super::{devtools_css, devtools_panel, devtools_store, layer_shell, os_file_drag, renderer};

mod actions;
mod activation;
mod app_handler;
mod app_identity;
mod content_window;
#[cfg(feature = "testing")]
pub(crate) use content_window::app_for_window;
#[cfg(feature = "testing")]
pub(crate) use content_window::content_max_of;
#[cfg(feature = "testing")]
pub(crate) use open_windows::ChildWindow;
mod debug;
#[cfg(feature = "gpu-renderer")]
mod device_loss;
mod devtools;
mod entry;
mod input_region;
mod lifecycle;
mod main_window;
mod native_events;
mod open_windows;
mod paint;
mod popups;
mod wake;
mod window_events;
mod winit_input;

#[cfg(test)]
mod bundled_font_tests;
#[cfg(test)]
mod layer_drain_2829_tests;
#[cfg(test)]
mod physical_size_tests;
#[cfg(test)]
mod text_only_key_2848_tests;
#[cfg(test)]
mod zoom_tests;

pub(crate) use app_identity::*;
pub use entry::*;
use main_window::*;
pub use wake::*;
pub use winit_input::*;

// ── RinchRuntime ─────────────────────────────────────────────────────────────

/// The desktop runtime: thin winit `ApplicationHandler` that delegates to
/// [`RinchApp`] for all platform-agnostic logic and uses [`WgpuRenderer`]
/// + [`WinitWindow`] for the platform-specific parts.
pub struct RinchRuntime {
    /// The main window: its app, OS window, renderer and the shell's state
    /// for it.
    main: ShellWindow,
    /// Event loop proxy for sending events.
    proxy: Option<EventLoopProxy>,
    /// Native menu bar (attached to the window).
    native_menu: Option<crate::menu::NativeMenuBar>,
    /// True while [`Self::drain_native_events`] is running. Prevents re-entrant
    /// drains: `DebugCommandKind::Screenshot` calls [`Self::paint`] from inside
    /// the drain, and the `RedrawRequested` arm drains before painting (#153).
    draining_native_events: bool,

    /// The windows still to be rebuilt on a new GPU device after a device
    /// loss, trying again while the driver recovers (#1788).
    #[cfg(feature = "gpu-renderer")]
    gpu_rebuild: Option<device_loss::GpuRebuild>,
    // ── DevTools ─────────────────────────────────────────────────
    /// Shared DevTools store (persists across open/close cycles).
    devtools_store: Option<super::devtools_store::DevToolsStore>,
    /// The DevTools window while it is open: a window of its own, handled by
    /// the same event path as the main window ([`WindowRole::DevTools`]).
    devtools: Option<ShellWindow>,
    /// The DevTools window was asked to close (F12, its close button): it is
    /// closed once the handler that asked returns
    /// ([`RinchRuntime::settle_devtools_close`]), so no handler finds the
    /// window it is handling gone.
    devtools_closing: bool,
    /// Previous hovered node ID (for detecting changes).
    devtools_prev_hovered: Option<u64>,

    // ── The app's other windows (#2416) ──────────────────────────
    /// The windows `open_window` opened, oldest first.
    others: Vec<ShellWindow>,
    /// The windows asked to close by their own handlers (a close button,
    /// `close_current_window`), closed once that handler returns
    /// ([`Self::settle_closes`]), by handle id.
    others_closing: Vec<u64>,
    /// The main window's handle id ([`crate::WindowHandle::main`]).
    main_handle: u64,
    /// The main window's own close was agreed to, in an app that runs on
    /// without it: closed once the handler returns ([`Self::settle_closes`]).
    main_closing: bool,
    /// The main window has closed, and the app runs on without it
    /// ([`crate::ExitWhen`]).
    main_closed: bool,
    /// When the event loop ends.
    exit_when: crate::ExitWhen,
    /// The app has no main window (`App::windowless`): its root is mounted in
    /// a document with no OS window ([`Self::mount_windowless_root`]).
    windowless: bool,
    /// The windowless root's painted content was warned of.
    windowless_root_warned: bool,
    /// The app's bundled fonts, for every window's document.
    fonts: Vec<crate::font::AppFont>,
    /// The popups' state beside the windows (#2552).
    popups: popups::PopupState,

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

/// Which of the runtime's windows an event or an action is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowRole {
    /// The app's window.
    Main,
    /// The DevTools window (F12), while it is open.
    DevTools,
    /// A window `open_window` opened, by its handle's id.
    Other(u64),
}

impl RinchRuntime {
    /// The window winit's `id` names, if it is one of the runtime's.
    fn role_of(&self, id: WindowId) -> Option<WindowRole> {
        let is = |w: &ShellWindow| w.window.as_ref().is_some_and(|w| w.window.id() == id);
        if is(&self.main) {
            Some(WindowRole::Main)
        } else if self.devtools.as_ref().is_some_and(is) {
            Some(WindowRole::DevTools)
        } else {
            self.others
                .iter()
                .find(|w| is(w))
                .map(|w| WindowRole::Other(w.handle))
        }
    }

    /// The window `role` names. The DevTools window is only named while it
    /// is open (it closes between handlers, [`Self::settle_devtools_close`]).
    fn w(&mut self, role: WindowRole) -> &mut ShellWindow {
        match role {
            WindowRole::Main => &mut self.main,
            WindowRole::DevTools => self
                .devtools
                .as_mut()
                .expect("an event for the DevTools window while it is open"),
            WindowRole::Other(id) => self
                .others
                .iter_mut()
                .find(|w| w.handle == id)
                .expect("an event for an open window"),
        }
    }

    /// Window `role`, if it is still open (#2891): a role kept from before
    /// its window closed (the device-loss rebuild's) names nothing.
    #[cfg(feature = "gpu-renderer")]
    fn open_w(&mut self, role: WindowRole) -> Option<&mut ShellWindow> {
        match role {
            WindowRole::Main => (!self.main_closed).then_some(&mut self.main),
            WindowRole::DevTools => self.devtools.as_mut(),
            WindowRole::Other(id) => self.others.iter_mut().find(|w| w.handle == id),
        }
    }

    /// The windows that are open: the main one (unless it has closed), then
    /// DevTools, then the others, oldest first.
    fn roles(&self) -> impl Iterator<Item = WindowRole> + use<> {
        let main = (!self.main_closed).then_some(WindowRole::Main);
        let devtools = self.devtools.is_some().then_some(WindowRole::DevTools);
        let others: Vec<WindowRole> = self
            .others
            .iter()
            .map(|w| WindowRole::Other(w.handle))
            .collect();
        main.into_iter().chain(devtools).chain(others)
    }
}

/// One window of the app and everything the shell keeps for it: its
/// [`RinchApp`] (document, scope, focus, painter), the OS window, its
/// renderer, and the shell-side state of that window (geometry, layer shell,
/// pointer, cursor, IME, accessibility).
struct ShellWindow {
    /// Platform-agnostic application logic.
    app: RinchApp,
    /// An xdg-activation token for the window's next OS window, given in its
    /// attributes (`WindowAttributesWayland::with_activation_token`, #2817):
    /// the app's launch token (`App::activation_token`) for the main
    /// window's first shown window, or the token of a focus request that
    /// shows a hidden window. Cleared once a shown window is made.
    activation_token: Option<String>,
    /// Desktop window wrapper.
    window: Option<WinitWindow>,
    /// GPU renderer (only available with the `gpu-renderer` feature).
    #[cfg(feature = "gpu-renderer")]
    renderer: Option<WgpuRenderer>,
    /// The GPU renderer of a hidden window, its surface gone with the OS
    /// window: the next show presents with it on a new surface rather than
    /// building Vello's pipelines again (#2545).
    #[cfg(feature = "gpu-renderer")]
    parked_renderer: Option<WgpuRenderer>,
    /// Shown (or made) while a device loss's rebuild was still waiting for a
    /// device, and its renderer would not start: it waits for the rebuild
    /// too, which takes it at its next turn (#1788).
    #[cfg(feature = "gpu-renderer")]
    awaits_gpu: bool,
    /// Software renderer (CPU pixel presentation via softbuffer). Every
    /// desktop build has it; a `gpu-renderer` build presents with it when `renderer` is
    /// `None` (software chosen, or the GPU could not start).
    soft_renderer: Option<super::softbuffer_renderer::SoftbufferRenderer>,
    /// While a hidden window stays prepared (#2633): the scale its last
    /// preparation painted at, and how many image load answers its document
    /// had taken then. A picture that lands later is painted into it again
    /// ([`RinchRuntime::keep_prepared`]). `None` while shown, and from a hide
    /// until the app prepares the window.
    /// The renderer choice the window was last prepared with (#2633): a
    /// rebuild after a device loss builds its renderer by it (#2891).
    #[cfg(feature = "gpu-renderer")]
    prepared_with: super::renderer::Renderer,
    prepared: Option<(f64, u64)>,
    /// Window title.
    title: String,
    width: u32,
    height: u32,
    /// Current keyboard modifier state (winit-specific).
    modifiers: winit::keyboard::ModifiersState,
    /// The newest pointer position winit has reported that the app has not
    /// been handed yet (logical px), and which pointer it is. See
    /// [`RinchRuntime::flush_pointer_move`].
    pending_pointer_move: Option<(f32, f32, SurfacePointer)>,
    /// The device's raw motion not yet handed to the app, summed (#2092). See
    /// [`RinchRuntime::flush_pointer_motion`].
    pending_motion: super::pointer_lock::MotionSum,
    /// The newest pointer position winit reported while a lock held it still
    /// (a confined pointer on X11 moves): handed over as a move when the lock
    /// ends. See [`Self::restore_held_pointer_move`].
    held_pointer_move: Option<(f32, f32, SurfacePointer)>,
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
    /// Where the window takes pointer input (#2741): starts at
    /// `WindowProps::input_region`, changed by
    /// [`set_input_region_current_window`](crate::windows::set_input_region_current_window).
    input_region: rinch_core::element::InputRegion,
    /// The region last computed from the window's layout, and its key.
    input_region_state: crate::app::input_region::InputRegionState,
    /// The region the OS window was last given (`input_region::AppliedRegion`).
    applied_region: Option<input_region::AppliedRegion>,
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
    /// The live layer surface's namespace kept against a different one given
    /// at run time, warned of once per name the surface is made under (#2821).
    layer_namespace: super::layer_shell::NamespaceKept,
    /// Which window the facts this window publishes are of
    /// (`window_geometry::WindowOf`, #2416): `None` for the main window,
    /// whose are what code outside any document reads, and the window's
    /// document for any other.
    facts_key: Option<u64>,
    /// The layer-shell changes the native events of this turn asked for
    /// (`set_current_window_layer`, `set_current_window_position`), in the
    /// order they were asked, sent together in one commit when the turn's
    /// drain ends ([`ShellWindow::flush_layer_changes`], #2464), with
    /// [`Self::pending_layer_size`].
    pending_layer: Vec<super::layer_shell::LayerSet>,
    /// The size the native events of this turn asked for a live layer
    /// surface (`set_current_window_size`), the newest: sent in the same
    /// commit as [`Self::pending_layer`]. Not asked for at once, because
    /// the fork commits a layer surface's `set_size` when it is asked
    /// (one request in flight, #2552), which would put the size and the
    /// turn's other changes in two commits.
    pending_layer_size: Option<Size>,
    /// The window's handle id ([`crate::WindowHandle`]); 0 for DevTools,
    /// which has none.
    handle: u64,
    /// Frames left in which the GPU window composites its `GameViewport`s as
    /// layers under the UI (cleared in the scene) rather than as split base
    /// layers (#1866): a base split was refused, so the window falls back
    /// for a while before it tries again.
    #[cfg(feature = "gpu-renderer")]
    base_fallback: super::base_fallback::BaseFallback,
    /// What the main window's surface does with vsync on and off (#2095),
    /// while it has a renderer; `None` while it is hidden or not yet made.
    pacing: Option<crate::window::PacingOutcomes>,
    /// Timestamp of the last paint (for FPS calculation).
    last_paint_time: Option<std::time::Instant>,
    /// Recent frame times in ms (ring buffer for FPS averaging).
    frame_times: VecDeque<f64>,
    /// The IME state last applied to the window, so [`Self::sync_ime`] only
    /// issues a winit `request_ime_update` on an actual change
    /// ([`ImeState::change_to`](crate::app::ImeState::change_to)).
    ime_applied: crate::app::ImeState,
    /// The per-platform AccessKit bridge (Linux AT-SPI; no-op elsewhere). Created
    /// lazily on first window creation; pushes the editor's tree on change.
    #[cfg(feature = "a11y")]
    a11y: Option<crate::editor::a11y::AccesskitBridge>,
    /// The (doc, selection) last pushed to the AT, so the tree is re-derived only
    /// when the focused editor actually changed.
    #[cfg(feature = "a11y")]
    a11y_last: Option<(rinch_editor_core::Node, rinch_editor_core::Selection)>,
    /// A popup whose first frame is still to come: when the app asked for
    /// it and when each step of its opening ended (#2552): `[PERF] window N
    /// first frame`.
    popup_first_frame: Option<crate::window_perf::PopupTimeline>,
}

impl ShellWindow {
    /// A window's state before the shell makes its OS window: the main
    /// window's `app` is mounted by the first `create_window`, DevTools' by
    /// `open_devtools`, which makes its window itself.
    fn new(mut app: RinchApp, title: &str, width: u32, height: u32) -> Self {
        // Not focused until the system says so (#2731), as
        // `rinch::window::window_focused()` already answered: an element's
        // `focus` event waits for the window's keyboard enter, and so do the
        // IME and the caret blink.
        app.window_focused = false;
        Self {
            app,
            activation_token: None,
            window: None,
            #[cfg(feature = "gpu-renderer")]
            renderer: None,
            #[cfg(feature = "gpu-renderer")]
            parked_renderer: None,
            #[cfg(feature = "gpu-renderer")]
            awaits_gpu: false,
            #[cfg(feature = "gpu-renderer")]
            prepared_with: super::renderer::Renderer::Auto,
            prepared: None,
            soft_renderer: None,
            title: title.to_string(),
            width,
            height,
            modifiers: winit::keyboard::ModifiersState::empty(),
            pending_pointer_move: None,
            pending_motion: Default::default(),
            held_pointer_move: None,
            os_file_drag: Default::default(),
            pressed_pointers: PressedButtons::default(),
            applied_cursor: None,
            click_through: false,
            input_region: Default::default(),
            input_region_state: Default::default(),
            applied_region: None,
            shown_once: false,
            saved_geometry: SavedGeometry::default(),
            monitors: Vec::new(),
            maximize_wait: window_geometry::MaximizeWait::Done,
            layer: None,
            output_override: None,
            layer_awaiting_output: false,
            layer_namespace: Default::default(),
            pending_layer: Vec::new(),
            pending_layer_size: None,
            handle: 0,
            facts_key: None,
            #[cfg(feature = "gpu-renderer")]
            base_fallback: Default::default(),
            pacing: None,
            last_paint_time: None,
            frame_times: VecDeque::with_capacity(60),
            ime_applied: crate::app::ImeState::default(),
            #[cfg(feature = "a11y")]
            a11y: None,
            #[cfg(feature = "a11y")]
            a11y_last: None,
            popup_first_frame: None,
        }
    }

    /// The app's zoom changed (`rinch::window::set_zoom`): the document
    /// takes the window's new scale (the display's times the zoom), as a
    /// `ScaleFactorChanged` gives it, and is laid out again at the logical
    /// size the same surface has at it, as the `SurfaceResized` that follows
    /// a new display scale does. The surface keeps its size, so the renderer
    /// is not reconfigured; the whole window is painted again.
    pub(super) fn rezoom(&mut self) -> Vec<AppAction> {
        let scale = self.scale_factor();
        let size = self.window_size();
        let mut actions =
            self.app
                .handle_event(PlatformEvent::ScaleFactorChanged(scale), size, scale);
        let resized = self.surface_resized(winit::dpi::PhysicalSize::new(size.0, size.1), scale);
        actions.extend(self.app.handle_event(resized, size, scale));
        self.app.mark_scene_dirty();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
        actions
    }

    /// What a `SurfaceResized` to the physical `size` at `scale` asks of
    /// this window, and the `Resized` it becomes: the renderer and the
    /// published surface size follow the surface, the document is laid out
    /// at the logical size.
    fn surface_resized(
        &mut self,
        size: winit::dpi::PhysicalSize<u32>,
        scale: f64,
    ) -> PlatformEvent {
        let physical = (size.width.max(1), size.height.max(1));
        // Also resize the renderer — it works in physical pixels.
        #[cfg(feature = "gpu-renderer")]
        if let Some(renderer) = &mut self.renderer {
            renderer.resize(physical.0, physical.1);
        }
        // The surface's own size, the renderer's above
        // (`window_physical_size`), published with the `Resized`: never the
        // logical size times the scale, which can be a pixel off.
        self.app.set_surface_size(physical);
        // `Resized` re-lays out the document, so it carries the logical size
        // (see `to_logical`).
        let logical = to_logical((size.width, size.height), scale);
        // The size the window was given: a content-sized document's grant
        // (#2552). Nothing for any other.
        self.app
            .set_granted_size(logical.0 as f32, logical.1 as f32);
        PlatformEvent::Resized {
            width: logical.0,
            height: logical.1,
        }
    }
}

/// What a window's move to a display at `os_scale` (winit's
/// `ScaleFactorChanged`) hands its document: the new display's scale times
/// the app's zoom (`rinch::window::set_zoom`), so the zoom is kept.
fn display_scale_changed(os_scale: f64) -> PlatformEvent {
    PlatformEvent::ScaleFactorChanged(crate::zoom::apply(os_scale))
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
