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
use super::layer_shell::{Gesture, GestureKind, LayerSpec, LayerState, WindowSurfaceKind};
use super::outputs::connected_monitors;
use super::overlay_window::{self, Backend, WindowSteps};
use super::window::WinitWindow;
use super::window_geometry::{self, SavedGeometry};

#[cfg(feature = "debug")]
use {super::screenshot, base64::Engine, rinch_debug::DebugResult};

#[cfg(feature = "gpu")]
use super::desktop;
use super::{devtools_css, devtools_panel, devtools_store, layer_shell, os_file_drag, renderer};

mod actions;
mod activation;
mod app_identity;
mod debug;
mod devtools;
mod entry;
mod main_window;
mod native_events;
mod open_windows;
mod paint;
mod wake;
mod winit_input;

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
    native_menu: Option<muda::Menu>,
    /// True while [`Self::drain_native_events`] is running. Prevents re-entrant
    /// drains: `DebugCommandKind::Screenshot` calls [`Self::paint`] from inside
    /// the drain, and the `RedrawRequested` arm drains before painting (#153).
    draining_native_events: bool,

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
    /// The app's bundled fonts, for every window's document.
    fonts: Vec<crate::font::AppFont>,

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
    /// Desktop window wrapper.
    window: Option<WinitWindow>,
    /// GPU renderer (only available with `gpu` feature).
    #[cfg(feature = "gpu")]
    renderer: Option<WgpuRenderer>,
    /// Software renderer (CPU pixel presentation via softbuffer). Every
    /// desktop build has it; a `gpu` build presents with it when `renderer` is
    /// `None` (software chosen, or the GPU could not start).
    soft_renderer: Option<super::softbuffer_renderer::SoftbufferRenderer>,
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
    /// Which window the facts this window publishes are of
    /// (`window_geometry::WindowOf`, #2416): `None` for the main window,
    /// whose are what code outside any document reads, and the window's
    /// document for any other.
    facts_key: Option<u64>,
    /// The layer-shell changes the native events of this turn asked for
    /// (`set_current_window_layer`, `set_current_window_position`), in the
    /// order they were asked, sent together in one commit when the turn's
    /// drain ends ([`ShellWindow::flush_layer_changes`], #2464). A size
    /// asked for in the same turn is staged on the surface by winit and goes
    /// out with that commit.
    pending_layer: Vec<super::layer_shell::LayerSet>,
    /// The window's handle id ([`crate::WindowHandle`]); 0 for DevTools,
    /// which has none.
    handle: u64,
    /// Frames left in which the GPU window composites its `GameViewport`s as
    /// layers under the UI (cleared in the scene) rather than as split base
    /// layers (#1866): a base split was refused, so the window falls back
    /// for a while before it tries again.
    #[cfg(feature = "gpu")]
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
}

impl ShellWindow {
    /// A window's state before the shell makes its OS window: the main
    /// window's `app` is mounted by the first `create_window`, DevTools' by
    /// `open_devtools`, which makes its window itself.
    fn new(app: RinchApp, title: &str, width: u32, height: u32) -> Self {
        Self {
            app,
            window: None,
            #[cfg(feature = "gpu")]
            renderer: None,
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
            shown_once: false,
            saved_geometry: SavedGeometry::default(),
            monitors: Vec::new(),
            maximize_wait: window_geometry::MaximizeWait::Done,
            layer: None,
            output_override: None,
            layer_awaiting_output: false,
            pending_layer: Vec::new(),
            handle: 0,
            facts_key: None,
            #[cfg(feature = "gpu")]
            base_fallback: Default::default(),
            pacing: None,
            last_paint_time: None,
            frame_times: VecDeque::with_capacity(60),
            ime_applied: crate::app::ImeState::default(),
            #[cfg(feature = "a11y")]
            a11y: None,
            #[cfg(feature = "a11y")]
            a11y_last: None,
        }
    }
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
        let main_handle = crate::window_handle::register_main();
        let mut main = ShellWindow::new(app, title, width, height);
        main.handle = main_handle;
        Self {
            main,
            proxy: None,
            native_menu: None,
            draining_native_events: false,
            devtools_store: None,
            devtools: None,
            devtools_closing: false,
            devtools_prev_hovered: None,
            others: Vec::new(),
            others_closing: Vec::new(),
            main_handle,
            main_closing: false,
            main_closed: false,
            exit_when: crate::window_handle::exit_when(),
            fonts: fonts.to_vec(),
            #[cfg(test)]
            _queue_lock,
        }
    }

    /// Explicit shutdown: drop resources in the correct order and clear global state.
    fn shutdown(&mut self) {
        // 0. Close DevTools and the other windows first.
        self.close_devtools();
        self.close_all_others();

        // 1. Clear the signal-change callback so no stale closures fire during drop.
        rinch_core::clear_on_signal_change();

        // 2. Drop any pending main-thread callbacks (they may capture app state).
        rinch_core::clear_main_callbacks();

        // 3. Drop the app first (disposes effects/scopes before GPU resources).
        //    RinchApp's own drop order handles _render_scope before doc.
        drop(self.main.app.component.take());
        drop(self.main.app._render_scope.take());
        drop(self.main.app.doc.take());

        // 4. Drop renderer before window — Surface holds a window handle reference.
        #[cfg(feature = "gpu")]
        drop(self.main.renderer.take());
        drop(self.main.soft_renderer.take());

        // 5. Window can now be dropped safely.
        drop(self.main.window.take());

        // 6. Nothing presents any more: `set_vsync` answers `Pending` (#2095).
        self.main.pacing = None;
        crate::window::clear_pacing();

        // 7. No window is open.
        crate::window_handle::reset();
    }
}

impl Drop for RinchRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ── ApplicationHandler impl ──────────────────────────────────────────────────

impl ApplicationHandler for RinchRuntime {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        // The outputs, before the window's first render reads them (#2414).
        super::outputs::publish(event_loop);
        // Mounted already means the window was hidden on purpose — started
        // hidden (#1520) or sent to the tray — and creating one here would
        // mount the root a second time; `show_window` is what brings it back.
        if creates_window_on_surfaces(self.main.window.is_some(), self.main.app.doc.is_some()) {
            self.create_window(event_loop);
        }
    }

    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        // Keep the pointer ahead of whatever the wake carries (an injected
        // press, a debug command that reads the hover state).
        for role in self.roles() {
            self.flush_pointer_move(role, event_loop);
        }
        // Track whether any reactive state changes while we drain queued work,
        // so the redraw decision below can key off the actual cause (a signal
        // changed) instead of repainting on every wake.
        rinch_core::clear_signals_changed();

        let mut deferred_ran = Vec::new();
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
                for role in rt.roles() {
                    if rt.w(role).app.run_deferred_work() > 0 {
                        deferred_ran.push(role);
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
        let changed = rinch_core::signals_changed();
        for role in self.roles() {
            let w = self.w(role);
            if deferred_ran.contains(&role) || changed || w.app.has_pending_layout() {
                if let Some(w) = &w.window {
                    w.request_redraw();
                }
            }
        }
        self.settle_closes(event_loop);
    }

    fn monitors_changed(&mut self, event_loop: &dyn ActiveEventLoop) {
        // An output was added, removed or changed (#2414): publish the new
        // list, and draw what its readers changed.
        rinch_core::clear_signals_changed();
        super::outputs::publish(event_loop);
        let changed = rinch_core::signals_changed();
        for role in self.roles() {
            let w = self.w(role);
            if changed || w.app.has_pending_layout() {
                if let Some(w) = &w.window {
                    w.request_redraw();
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Found by its request's serial, not by `window_id`: winit sends it
        // even when the window that asked has been dropped, under that
        // window's id, which a newer window can have (#2501).
        if let WindowEvent::ActivationTokenDone { serial, token } = event {
            super::activation_tokens::done(serial, token.into_raw());
            return;
        }
        let Some(role) = self.role_of(window_id) else {
            return;
        };
        self.window_event_for(role, event_loop, event);
        self.settle_closes(event_loop);
    }

    fn device_event(
        &mut self,
        _event_loop: &dyn ActiveEventLoop,
        _device_id: Option<winit::event::DeviceId>,
        event: DeviceEvent,
    ) {
        let role = self.motion_role();
        self.w(role).note_device_event(&event);
    }

    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        // The batch's raw motion and last pointer move, before anything is
        // laid out.
        self.flush_pointer_motion(event_loop);
        for role in self.roles() {
            self.w(role).restore_held_pointer_move();
            self.flush_pointer_move(role, event_loop);
        }
        self.about_to_wait_inner(event_loop);
        self.settle_closes(event_loop);
    }
}

impl RinchRuntime {
    /// One window's event, for whichever of the runtime's windows it is: the
    /// main window and the DevTools window take the same path. What only the
    /// main window has is gated on [`WindowRole::Main`]: the pointer lock and
    /// raw motion (#2092), the published geometry (#1546), DevTools' inspect
    /// mode, and the native events the RedrawRequested arm drains.
    fn window_event_for(
        &mut self,
        role: WindowRole,
        event_loop: &dyn ActiveEventLoop,
        event: WindowEvent,
    ) {
        // Whatever pointer this event names (`set_current_pointer` below) is
        // forgotten when it has been handled, on every return path, so an
        // event that names none (a key, an injected debug `mouse_down`) is
        // never read as the last pen or finger.
        let _forget_pointer = ForgetCurrentPointer;
        // Raw motion that came before this event reaches the app first.
        self.flush_pointer_motion(event_loop);

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
            let (lx, ly) = to_logical_point((position.x, position.y), self.w(role).scale_factor());
            // While the pointer is locked or confined (#2092) its position is
            // held where the lock found it, as a browser holds it: a confined
            // cursor (X11) still moves, and must not move hover under a
            // camera the mouse is turning. The motion arrives as raw deltas.
            // Where it really is is kept for when the lock ends
            // (`restore_held_pointer_move`), so hover is not left stale.
            if crate::pointer_lock::is_locked_in(self.w(role).facts_key) {
                let id = pointer_id_of(source);
                let down = self.w(role).pressed_pointers.is_down(id);
                let pointer = surface_pointer_of(source, *primary, down);
                self.w(role).held_pointer_move = Some((lx as f32, ly as f32, pointer));
                return;
            }
            // A layer surface's move or resize (#1523) takes the pointer, as
            // a compositor move does.
            if self.update_layer_gesture(role, (lx, ly), event_loop) {
                return;
            }
            let id = pointer_id_of(source);
            let down = self.w(role).pressed_pointers.is_down(id);
            let pointer = surface_pointer_of(source, *primary, down);
            self.queue_pointer_move(role, lx as f32, ly as f32, pointer, event_loop);
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
            let (lx, ly) = to_logical_point((position.x, position.y), self.w(role).scale_factor());
            let down = self
                .w(role)
                .pressed_pointers
                .is_down(SurfacePointer::mouse(false).id);
            let pointer = SurfacePointer {
                primary: *primary,
                ..SurfacePointer::mouse(down)
            };
            self.queue_pointer_move(role, lx as f32, ly as f32, pointer, event_loop);
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
            if let Some(layer) = self.w(role).layer.as_mut() {
                layer.gesture = None;
            }
            let pointer = surface_pointer_of_button(button, *primary, pressed);
            if !matches!(button, winit::event::ButtonSource::Mouse(_)) {
                let (lx, ly) =
                    to_logical_point((position.x, position.y), self.w(role).scale_factor());
                self.queue_pointer_move(role, lx as f32, ly as f32, pointer, event_loop);
            }
            self.flush_pointer_move(role, event_loop);
            self.w(role)
                .pressed_pointers
                .note(pointer.id, button.clone().mouse_button(), pressed);
            crate::render_surface::set_current_pointer(Some(pointer));
        }
        self.flush_pointer_move(role, event_loop);

        // A layer surface's move reached the compositor (#1523).
        if let WindowEvent::Moved(p) = &event {
            self.w(role).layer_moved((p.x, p.y));
        }
        // Keep the published geometry (#1546) current: each window its own.
        if matches!(
            event,
            WindowEvent::Moved(_)
                | WindowEvent::SurfaceResized(_)
                | WindowEvent::ScaleFactorChanged { .. }
        ) {
            if let Some(w) = self.w(role).window.as_ref().map(|w| w.window.clone()) {
                self.w(role).note_geometry(&*w);
            }
        }

        let platform_event = match event {
            // A layer surface has no close button: its close is the
            // compositor's, for an output that went away. Show it again
            // rather than quitting (#1523).
            // Only the main window can be a layer surface.
            WindowEvent::CloseRequested
                if role == WindowRole::Main && self.main.layer.is_some() =>
            {
                self.main
                    .layer_surface_closed(event_loop, self.native_menu.as_ref());
                return;
            }
            // A second window's layer surface closed by the compositor (its
            // output went) closes the window, with no veto: the surface is
            // gone already (#2416).
            WindowEvent::CloseRequested
                if matches!(role, WindowRole::Other(_)) && self.w(role).layer.is_some() =>
            {
                self.close_soon(role);
                return;
            }
            WindowEvent::CloseRequested => {
                // What the app reads in `on_close_requested` (#1884).
                self.w(role).settle_maximize_wait();
                PlatformEvent::CloseRequested
            }
            WindowEvent::SurfaceResized(size) => {
                // Also resize the renderer — it works in physical pixels.
                #[cfg(feature = "gpu")]
                if let Some(renderer) = &mut self.w(role).renderer {
                    renderer.resize(size.width.max(1), size.height.max(1));
                }
                // `Resized` re-lays out the document, so it carries the
                // logical size (see `to_logical`).
                let logical = to_logical((size.width, size.height), self.w(role).scale_factor());
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
                let painted = match role {
                    WindowRole::Main => self.paint(),
                    WindowRole::DevTools => self.paint_devtools(),
                    WindowRole::Other(_) => self.w(role).paint_window(),
                };
                if let Err(e) = painted {
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
                let (x, y) = self.w(role).app.cursor_pos.unwrap_or((0.0, 0.0));

                // Inspect mode: click selects node and exits inspect mode
                if role == WindowRole::Main && platform_button == PlatformMouseButton::Left {
                    if let Some(store) = &self.devtools_store {
                        if store.inspect_mode.get() {
                            if let Some(doc) = &self.main.app.doc {
                                let d = doc.borrow();
                                let hit = crate::app::hit_testing::hit_test(&d.tree, x, y);
                                store.selected_node_id.set(hit);
                            }
                            store
                                .active_tab
                                .set(super::devtools_store::DevToolsTab::Elements);
                            store.inspect_mode.set(false);
                            store.hovered_node_id.set(None);
                            self.main.app.inspect_highlight = None;
                            self.main.app.request_repaint();
                            if let Some(w) = &self.main.window {
                                w.request_redraw();
                            }
                            return;
                        }
                    }
                }

                let size = self.w(role).window_size();
                let scale = self.w(role).scale_factor();
                let actions = self.w(role).app.handle_event(
                    PlatformEvent::MouseDown {
                        x,
                        y,
                        button: platform_button,
                    },
                    size,
                    scale,
                );
                self.process_actions(role, actions, event_loop);
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
                let (x, y) = self.w(role).app.cursor_pos.unwrap_or((0.0, 0.0));
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
                self.w(role).app.hide_title_tooltip();
                if let Some(w) = &self.w(role).window {
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
                if !self.w(role).pressed_pointers.lift(id) {
                    return;
                }
                let pointer = SurfacePointer {
                    id,
                    kind: SurfacePointerKind::Touch,
                    pressure: 0.0,
                    primary: false,
                };
                self.w(role).app.cancel_surface_pointer(pointer);
                return;
            }
            // A trackpad pinch (macOS, iOS, Wayland): a `Pinch` for a
            // surface with pointer events under the cursor.
            // A pinch no surface takes is an `onwheel` with Ctrl held, as a
            // browser reports it.
            WindowEvent::PinchGesture { delta, .. } => {
                if delta.is_finite()
                    && !self.w(role).app.surface_pinch_gesture(delta as f32)
                    && self.w(role).app.pinch_as_wheel(delta as f32)
                    && let Some(w) = &self.w(role).window
                {
                    w.window.request_redraw();
                }
                return;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // Already logical: set from the converted `PointerMoved` above.
                let cursor = self.w(role).app.cursor_pos.unwrap_or((0.0, 0.0));
                match wheel_platform_event(delta, cursor, self.w(role).scale_factor()) {
                    Some(event) => event,
                    None => return,
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
                    self.w(role).pressed_pointers.clear();
                }
                // Tell the AT whether this window has OS focus.
                #[cfg(feature = "a11y")]
                if let Some(bridge) = self.w(role).a11y.as_mut() {
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
                self.w(role).modifiers = new_modifiers.state();
                let mods = self.w(role).translate_modifiers();
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
                let mods = self.w(role).translate_modifiers();
                // Escape while the pointer is locked is the lock's (#2092):
                // `handle_event` ends it and consumes the key.
                let lock_escape = key_code == winit::keyboard::KeyCode::Escape
                    && crate::pointer_lock::is_locked_in(self.w(role).facts_key);
                if !lock_escape && self.w(role).app.try_menu_shortcut(mods, key_code, win_kwm) {
                    // Shortcut matched and callback dispatched; request redraw
                    if let Some(w) = &self.w(role).window {
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
                let mods = self.w(role).translate_modifiers();
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
                let position =
                    position.map(|p| to_logical_point((p.x, p.y), self.w(role).scale_factor()));
                let steps = self
                    .w(role)
                    .os_file_drag
                    .entered(id, position, has_uri_list);
                self.run_file_drag_steps(role, steps, event_loop);
                return;
            }
            WindowEvent::DragPosition { id, position, .. } => {
                let position =
                    to_logical_point((position.x, position.y), self.w(role).scale_factor());
                let steps = self.w(role).os_file_drag.moved(id, position);
                self.run_file_drag_steps(role, steps, event_loop);
                return;
            }
            WindowEvent::DataTransferReceived { id, serial, value } => {
                let uris = super::os_file_drag::uris_from(value.try_as_uris());
                let steps = self.w(role).os_file_drag.received(id, serial, uris);
                self.run_file_drag_steps(role, steps, event_loop);
                return;
            }
            WindowEvent::DragDropped { id, .. } => {
                let steps = self
                    .w(role)
                    .os_file_drag
                    .dropped(id, std::time::Instant::now());
                self.run_file_drag_steps(role, steps, event_loop);
                return;
            }
            WindowEvent::DragLeft { id } => {
                let steps = self.w(role).os_file_drag.left(id);
                self.run_file_drag_steps(role, steps, event_loop);
                return;
            }
            _ => return,
        };
        self.dispatch_event(role, platform_event, event_loop);
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
            .main
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
            .main
            .app
            .pending_fonts
            .iter()
            .map(|f| f.font.generics[0])
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
        assert!(runtime.main.app.pending_fonts.is_empty());
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
        let first = runtime.main.main_window_steps(Backend::X11, true);
        assert!(first.click_through, "the prop reaches the first creation");
        assert!(after_create_actions(first, false).contains(&AfterCreate::ClickThrough));

        assert!(!runtime.main.note_click_through(false));
        let reshown = runtime.main.main_window_steps(Backend::X11, false);
        assert!(
            !reshown.click_through,
            "the toggle is kept across hide/show"
        );
        assert!(!after_create_actions(reshown, false).contains(&AfterCreate::ClickThrough));

        assert!(runtime.main.note_click_through(true));
        assert!(
            runtime
                .main
                .main_window_steps(Backend::X11, false)
                .click_through
        );

        // Off in the props: a re-show after a toggle on keeps it on.
        let mut plain = build_runtime(component, &Default::default(), None, &[]);
        assert!(
            !plain
                .main
                .main_window_steps(Backend::Wayland, true)
                .click_through
        );
        plain.main.note_click_through(true);
        assert!(
            plain
                .main
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
        assert!(runtime.main.main_window_always_on_top());
        for first in [true, false] {
            let mac = runtime.main.main_window_steps(Backend::MacOs, first);
            assert!(
                after_create_actions(mac, true).contains(&AfterCreate::MacPlacement {
                    level: Some(1000),
                    collection: 1 | 256,
                })
            );
            let x11 = runtime.main.main_window_steps(Backend::X11, first);
            assert!(after_create_actions(x11, true).contains(&AfterCreate::ReassertOnTop));
            for backend in [Backend::Windows, Backend::Wayland, Backend::MacOs] {
                let steps = runtime.main.main_window_steps(backend, first);
                let attrs = runtime
                    .main
                    .main_window_attributes(steps, backend, first, first);
                assert_eq!(
                    attrs.window_level,
                    winit::window::WindowLevel::AlwaysOnTop,
                    "{backend:?}"
                );
            }
        }
        let plain = build_runtime(component, &Default::default(), None, &[]);
        assert!(!plain.main.main_window_always_on_top());
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
            let steps = runtime.main.main_window_steps(backend, true);
            assert!(after_create_actions(steps, false).is_empty(), "{backend:?}");
            assert!(after_create_actions(steps, true).is_empty(), "{backend:?}");
        }
        let steps = runtime.main.main_window_steps(Backend::Wayland, true);
        let first = runtime
            .main
            .main_window_attributes(steps, Backend::Wayland, true, true);
        assert!(first.position.is_some(), "the first creation is placed");
        let reshown = runtime
            .main
            .main_window_attributes(steps, Backend::Wayland, false, false);
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
            runtime.main.app.pending_fonts.len(),
            1,
            "the bundled face must reach the app the window will mount"
        );
        assert_eq!(
            runtime.main.app.pending_fonts[0].font.generics[0],
            parley::fontique::GenericFamily::Monospace,
            "and reach it with the generic it declared"
        );
        assert_eq!((runtime.main.width, runtime.main.height), (1024, 768));
        assert_eq!(runtime.main.title, "Carried");
        let carried = runtime
            .main
            .app
            .window_props
            .as_ref()
            .expect("window props must reach the app");
        assert_eq!(carried.app_id.as_deref(), Some("com.example.carried"));
    }
}
