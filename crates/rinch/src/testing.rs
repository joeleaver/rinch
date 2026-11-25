//! A headless harness for testing an app's own views: mount a component, send
//! it input the way the desktop runtime does, and read the document back.
//!
//! Enable the `testing` feature (it needs `windowing`, which the default
//! `desktop` includes), usually as
//! a dev-dependency:
//!
//! ```toml
//! [dev-dependencies]
//! rinch = { workspace = true, features = ["desktop", "testing"] }
//! ```
//!
//! [`TestApp`] owns a [`RinchApp`] with no window. Every input method sends the
//! [`PlatformEvent`] the desktop shell makes of the matching winit event, to
//! the same [`RinchApp::handle_event`] the shell calls, so a test sees what a
//! real press, key or wheel does: hit testing, which handler runs, focus,
//! default actions, the menus a key reaches first. After each input the
//! harness runs one turn of the event loop — the main-thread callback queue,
//! the work other threads sent the app, `AboutToWait` (transitions, animations,
//! layout), and the frame painted with the software renderer when one was
//! asked for — so effects and layout are current when the call returns.
//!
//! No display, compositor or GPU is needed.
//!
//! # Which handler runs
//!
//! A row with its own `onmousedown`/`onmouseup` inside a container that has
//! them too: a press on the row runs the row's handler only, the nearest one
//! (as `rinch_core::events` documents); a press on the container's padding runs
//! the container's.
//!
//! ```
//! use rinch::prelude::*;
//! use rinch::testing::TestApp;
//! use std::cell::RefCell;
//! use std::rc::Rc;
//!
//! let log = Rc::new(RefCell::new(Vec::<&str>::new()));
//! let l = log.clone();
//! let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
//!     let (a, b, c, d) = (l.clone(), l.clone(), l.clone(), l.clone());
//!     rsx! {
//!         div {
//!             id: "list",
//!             style: "padding: 20px; width: 300px",
//!             onmousedown: move || a.borrow_mut().push("list down"),
//!             onmouseup: move || b.borrow_mut().push("list up"),
//!             div {
//!                 id: "row",
//!                 style: "height: 30px",
//!                 onmousedown: move || c.borrow_mut().push("row down"),
//!                 onmouseup: move || d.borrow_mut().push("row up"),
//!                 "First result"
//!             }
//!         }
//!     }
//! });
//!
//! app.click_on("#row");
//! assert_eq!(*log.borrow(), ["row down", "row up"]);
//!
//! log.borrow_mut().clear();
//! let list = app.bounds("#list");
//! app.click(list.x + 5.0, list.y + 5.0); // the container's own padding
//! assert_eq!(*log.borrow(), ["list down", "list up"]);
//! ```
//!
//! # What it does not do
//!
//! - It drives one window. Menus a key reaches are matched (as the shell does
//!   before the app sees a key), but the native menu bar, the tray, other
//!   windows and the OS (clipboard aside) are not there.
//! - Nothing advances with real time. Timers, transitions and animations,
//!   multi-click and tooltip timing are on the virtual clock (see
//!   [Time](#time)); [`TestApp::settle`] runs further turns, it does not skip
//!   ahead.
//! - A closure a worker thread queues with
//!   [`run_on_main_thread`](crate::run_on_main_thread) (or
//!   `rinch_core::dispatch_main_callback`) names no app, and is run by
//!   whichever `TestApp`'s turn drains first, on that test's thread. A signal
//!   written there (`run_on_main_thread(move || status.set(..))`) is then
//!   another thread's: the write finds nothing and is dropped with a warning
//!   (it used to land in that test's signal of the same slot). Send the value
//!   with `Signal::send` instead, which is not such a closure: see
//!   [Threads](#threads).
//! - Render surfaces (`RenderSurface`, `GameViewport`) are laid out but not
//!   driven: no per-frame render callback runs.
//! - A chord on a punctuation key (`Ctrl+/`, `Ctrl+=`) does not reach a menu
//!   shortcut: only letters, digits and the named keys are matched, as with
//!   the MCP `key_press` tool. The key still reaches the app.
//! - The window actions the app asks for are recorded, not performed: a
//!   right click in a text field yields `AppAction::ShowTextContextMenu` in
//!   [`TestApp::take_actions`], and no menu opens. The window's own requests
//!   (`hide_current_window`, `show_current_window`, …) are recorded in
//!   [`TestApp::take_window_requests`]; showing and hiding are also applied
//!   to the test's window (see [The window](#the-window)).
//!
//! # Threads
//!
//! Tests in one binary run in parallel, a `TestApp` on each test thread, and
//! each app's thread is a host of its own (`rinch_core::mark_host_thread`):
//! its turns drain only the main-thread callbacks meant for it. A
//! `Signal::send` or `update_send` reaches the app whose thread created the
//! signal, and runs there — sent from the test's own thread (applied at once),
//! from a worker thread the test spawned, or from another test's thread —
//! whatever other `TestApp`s are alive and whichever thread is the process's
//! registered main thread, if any (issue #2778). A closure the test's own
//! thread queues is for its own app too.
//!
//! ```
//! use rinch::prelude::*;
//! use rinch::testing::TestApp;
//! use std::cell::Cell;
//! use std::rc::Rc;
//!
//! let slot = Rc::new(Cell::new(None));
//! let s = slot.clone();
//! let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
//!     let status = Signal::new(String::from("idle"));
//!     s.set(Some(status));
//!     rsx! { p { id: "status", {|| status.get()} } }
//! });
//! let status = slot.get().unwrap();
//!
//! // A worker's write is queued for this app, and applied at its next turn.
//! std::thread::spawn(move || status.send(String::from("done")))
//!     .join()
//!     .unwrap();
//! app.settle();
//! assert_eq!(app.text("#status"), "done");
//! ```
//!
//! # The window
//!
//! [`TestAppBuilder::window_props`] mounts the component in the window an
//! [`App`](crate::App) with those [`WindowProps`] would open: its size,
//! `visible: false` (a hidden window is mounted but not painted, and has no
//! focus), `non_activating` (shown without taking the keyboard),
//! `size_to_content` (the window takes its content's size, and follows it),
//! `transparent`. The window publishes its state as the desktop shell does,
//! so [`window_focused`](crate::window::window_focused),
//! [`window_size`](crate::window::window_size) and
//! [`scale_factor`](crate::window::scale_factor) follow
//! [`TestApp::window_focus`], [`TestApp::resize`] and
//! [`TestApp::set_scale_factor`].
//!
//! A test sees the window's own requests in
//! [`TestApp::take_window_requests`] (and those made through the
//! [`WindowHandle`](crate::WindowHandle) of a window the app opened, as
//! [`WindowRequest::OtherWindow`], recorded only), and the ones that change
//! what the test can observe are applied: a hide makes [`TestApp::is_visible`] false and
//! takes the keyboard away (a hidden window has no focus), a show makes it
//! true, paints, and gives the window the keyboard unless it is
//! non-activating; a prepare lays out and paints the hidden window; a size
//! request resizes it.
//!
//! ```
//! use rinch::prelude::*;
//! use rinch::testing::{TestApp, WindowRequest};
//!
//! // A launcher that hides when its window loses the keyboard (a window is
//! // mounted before it has the keyboard, so not before it first had it).
//! let mut app = TestApp::mount(|__scope: &mut RenderScope| {
//!     let had_it = std::cell::Cell::new(false);
//!     rinch::Effect::new(move || {
//!         let focused = rinch::window::window_focused();
//!         if had_it.replace(focused) && !focused {
//!             rinch::windows::hide_current_window();
//!         }
//!     });
//!     rsx! { div { "Launcher" } }
//! });
//! assert!(app.is_visible());
//!
//! app.window_focus(false);
//! assert_eq!(app.take_window_requests(), [WindowRequest::Hide]);
//! assert!(!app.is_visible());
//! ```
//!
//! # Windows a component opens
//!
//! A `Window { … }` element, or [`open_window`](crate::open_window), opens a
//! child window, and the same `TestApp` drives it (#2793): it shares the
//! app's thread, reactive runtime and turns. It is made by the shell's own
//! code up to its OS window, with no OS window and no renderer, and closed
//! by the shell's own teardown.
//!
//! - **When it opens.** After the turn that built its element, when the
//!   window requests are served, never inside that turn: the shell makes it
//!   from its queue on a later wake. [`TestApp::windows`] lists the open
//!   ones, oldest first; [`TestApp::window`] (or
//!   [`TestApp::window_by_id`]) borrows one as a [`TestWindow`].
//! - **Its first facts, in the shell's order.** It is mounted at the main
//!   window's scale (unless its content sizes it: then its output's, see
//!   [Outputs](#outputs)) with no surface yet: a reader made in its mount sees a
//!   [`window_physical_size`](crate::window::window_physical_size) of
//!   `(0, 0)`. Then it gets the configure `create_window` gives it: the
//!   surface, then a `Resized` at its props' size. Then whatever the test
//!   plays: [`TestWindow::resize`], [`TestWindow::set_scale`] and
//!   [`TestWindow::set_surface_size`] (the surface for the next of those
//!   only, published with it in one batch; after it the stand-in, the
//!   logical size times the scale, rounded, returns).
//! - **Each turn**, every open child gets its `AboutToWait` and, when its
//!   layout is pending, the style, layout and bounds the shell resolves
//!   before a frame, short of painting: [`TestWindow::bounds`] and
//!   `bounds_signal()` readers are current.
//! - **Its requests** (made through its [`WindowHandle`](crate::WindowHandle),
//!   or by `rinch::windows` in its document) are its own, in
//!   [`TestWindow::take_requests`], in the order made, and **no longer show
//!   as [`WindowRequest::OtherWindow`]** in the app's
//!   [`TestApp::take_window_requests`]; a request for a window that is not
//!   open (closed, or never made) still does. They are applied as the
//!   shell's own code would without an OS. The harness stands in a Wayland
//!   compositor with layer shell: a window the shell would ask a layer
//!   surface for (`wayland_layer` set, or a non-activating window) is one,
//!   and its handle's `surface_kind()` says so; any other is a toplevel. A
//!   toplevel's size request is applied at once, as the shell feeds back
//!   the size winit grants; a layer surface's size is the compositor's to
//!   give, so it is recorded only and the test plays the compositor with
//!   [`TestWindow::resize`]. A compositor without layer shell (GNOME's
//!   toplevel fallback) is not played. Its handle's `size()` is the logical
//!   size it was configured at; `position()` is `None`.
//! - **Recorded only.** A child's show, hide, prepare, focus, title,
//!   position, layer and output requests, and a layer surface's size, are
//!   recorded and not applied. A child has no visibility and no keyboard
//!   focus in the harness: it is not painted, and
//!   [`window_focused`](crate::window::window_focused) read in a child stays
//!   false. (The main window does apply show and hide.)
//! - **Input and modifiers.** Input sent with a [`TestWindow`] goes to that
//!   window's app, and what the app answers is taken as the shell takes it
//!   for a second window (an `Exit` closes it). Each window holds its own
//!   modifiers, as the shell reports them to the window with the keyboard:
//!   [`TestWindow::set_modifiers`] for a child, [`TestApp::set_modifiers`]
//!   for the main window. A [`TestWindow`] names its window by id: used
//!   after the window closed, it panics naming it, and an input of several
//!   steps stops at the step after which the window closed. A
//!   [`NodeHandle`] from one window's document used on another panics.
//! - **Closing.** A child closes when its handle closes it (its
//!   `on_closed` runs), when its owner's scope goes (it does not), when its
//!   app agrees to its close control, and when [`TestWindow::close_by_system`]
//!   plays the system's close (a layer surface's `closed`, with no veto; a
//!   toplevel's title-bar close, which its `on_close_requested` may refuse).
//!   It is then no longer listed, and its facts are gone.
//!
//! ```
//! use rinch::prelude::*;
//! use rinch::testing::TestApp;
//!
//! let mut app = TestApp::mount(|__scope: &mut RenderScope| {
//!     rsx! {
//!         div {
//!             Window { title: "Details".to_owned(), width: 300, height: 200,
//!                 p { id: "name", "Details" }
//!             }
//!         }
//!     }
//! });
//! let child = app.windows()[0];
//! let mut window = app.window(child);
//! assert_eq!(window.props().title, "Details");
//! assert_eq!(window.text("#name"), "Details");
//! window.resize(400, 250);
//! assert_eq!(window.size(), (400, 250));
//! ```
//!
//! # Outputs
//!
//! With nothing played the harness lists no outputs, as a desktop with no
//! event loop does: [`outputs`](crate::outputs::outputs) is empty and
//! [`output_tracking`](crate::outputs::output_tracking) is `NotListed`.
//! [`TestApp::set_outputs`] plays a list, as the shell publishes the
//! compositor's at each change (#2836): through the same path, with
//! `Live`, replacing the whole list each call. Build each
//! [`Output`](crate::outputs::Output) as a literal with
//! [`OutputId::from_raw`](crate::outputs::OutputId::from_raw).
//!
//! - **Readers run again as in the app.** `outputs()`, `output_ids()`,
//!   `output(id)` and `output_tracking()`, in the app and in its child
//!   windows, run once per change; an equal list runs none. A window per
//!   `output_ids()` row opens for an output played and closes with it.
//! - **A child placed by an output id** that is not played is refused as
//!   the shell refuses it on Wayland, which the harness stands in for: it is not made, its handle reports closed and
//!   its `on_closed` is called once. One placed by a name is made whatever
//!   the name, as the compositor chooses. With nothing played no child is
//!   refused.
//! - **A content-sized window** (the main window or a child) fits the
//!   output it is on (a layer surface's output, else its `monitor`, else the
//!   first listed, as the shell picks it) and follows it when it changes; a
//!   content-sized child starts at its output's scale, and a content-sized
//!   main window mounted while outputs are played starts at its size there.
//! - **Not played, the compositor's part:** an output removed under a
//!   child leaves it open until the test plays the compositor's close with
//!   [`TestWindow::close_by_system`], and a child's scale follows its
//!   output only when the test sets it with [`TestWindow::set_scale`]. The
//!   main window waiting for an output when its layer surface is closed
//!   with none left is the shell's only, and is not played.
//! - **One list per thread**, as the shell's: two `TestApp`s on one thread
//!   read the same list, and the last `set_outputs` wins for both. An app
//!   mounted while another on the thread has played outputs reads them from
//!   its first render: it does not start empty. The list is forgotten when
//!   the last `TestApp` on the thread goes, after its documents are disposed,
//!   so none of its readers runs for it, and the next one starts with none
//!   played.
//!
//! ```
//! use rinch::outputs::{Output, OutputId, output, output_ids};
//! use rinch::prelude::*;
//! use rinch::testing::TestApp;
//!
//! let mut app = TestApp::mount(|__scope: &mut RenderScope| {
//!     rsx! {
//!         div {
//!             for id in output_ids() {
//!                 Window { key: id, title: "bar".to_owned(),
//!                     wayland_layer: Some(LayerSurface { output_id: Some(id), ..LayerSurface::default() }),
//!                     p { id: "name", {move || output(id).and_then(|o| o.name).unwrap_or_default()} }
//!                 }
//!             }
//!         }
//!     }
//! });
//! let screen = |id, name: &str| Output {
//!     id: OutputId::from_raw(id),
//!     name: Some(name.to_owned()),
//!     position: (0.0, 0.0),
//!     size: (1920.0, 1080.0),
//!     scale: 1.0,
//! };
//! app.set_outputs([screen(1, "DP-1"), screen(2, "HDMI-A-1")]);
//! let bars = app.windows();
//! assert_eq!(app.window(bars[1]).text("#name"), "HDMI-A-1");
//! app.set_outputs([screen(1, "DP-1")]);
//! assert_eq!(app.windows(), [bars[0]]);
//! ```
//!
//! # Time
//!
//! `set_timeout` and `set_interval` run on a virtual clock that starts at
//! zero when the app is mounted. [`TestApp::advance_time`] moves it forward
//! and fires every timer that comes due on the way, in deadline order (timers
//! due together in the order they were armed), with a turn of the loop after
//! each deadline: the timers due at one instant fire in one turn, and a timer
//! one of them arms for that same instant fires in it too. A timer armed for
//! later fires if it falls inside the advance. A timer due now
//! (`set_timeout(0, ..)`) fires at the next turn, as on the desktop.
//!
//! [`crate::time::now`] reads the same clock (the instant it started at plus
//! [`TestApp::elapsed`]), so code that judges elapsed time with it agrees with
//! its timers to the millisecond; `std::time::Instant::now()` does not.
//!
//! rinch's own timed behaviour reads it too (#2842): CSS transitions and
//! animations (and a wheel notch's glide), double and triple clicks (500 ms),
//! the `title` tooltip's delay (500 ms), drag auto-scroll's speed, the
//! editor's caret blink and undo grouping, `<select>` and `Select` type-ahead
//! (900 ms) and timed `poll_signal`s. None of them move unless the test
//! advances the clock: two clicks with no `advance_time` between them are a
//! double click, and a transition stays at its start until the clock passes
//! it.
//!
//! The clock is the thread's, so tests on other threads keep their own. Two
//! `TestApp`s alive at once on one thread share one clock: the second starts
//! at the time the first's clock shows, and either one's
//! [`advance_time`](TestApp::advance_time) moves both.
//!
//! ```
//! use rinch::prelude::*;
//! use rinch::testing::TestApp;
//! use std::time::Duration;
//!
//! let mut app = TestApp::mount(|__scope: &mut RenderScope| {
//!     let seconds = Signal::new(0);
//!     set_interval(1000, move || seconds.update(|s| *s += 1));
//!     rsx! { p { id: "clock", {move || seconds.get().to_string()} } }
//! });
//! app.advance_time(Duration::from_millis(2500));
//! assert_eq!(app.text("#clock"), "2");
//! ```

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};
use rinch_core::element::WindowProps;
use rinch_core::events::{ModifierState, MouseButton};
use rinch_core::reactive::ElementBounds;
use rinch_dom::RinchDocument;
use rinch_platform::{AppAction, ImeEvent, Modifiers, PlatformEvent};

use crate::app::RinchApp;
use crate::app::key_names::{NamedKeyDown, keyname_to_keycode, named_logical_key};
use crate::font::AppFont;
use crate::shell::rinch_runtime::{EventFor, RinchNativeEvent};
use crate::shell::softbuffer_renderer::{IdleTrimmed, PresentTarget, Presenter};
use crate::window::WindowFacts;
use rinch_core::timer::virtual_time;

#[cfg(test)]
mod app_key_windows_tests;
#[cfg(test)]
mod idle_trim_perf_2852_tests;
#[cfg(test)]
mod perf_tests;
#[cfg(test)]
mod popup_tests;

mod children;
mod document;
mod event_loop;
mod input;
mod lifecycle;
mod played_outputs;
/// Reading a document, the main window's or a child's.
mod read;
mod target;
mod test_window;
mod window_state;

pub use lifecycle::TestAppBuilder;
pub use target::Target;
pub use test_window::TestWindow;

/// The window size a [`TestApp`] mounts at unless told otherwise, in logical
/// (CSS) pixels.
pub const DEFAULT_SIZE: (u32, u32) = (800, 600);

/// The shell's button for the one an app's handlers are told about.
fn platform_button(button: MouseButton) -> rinch_platform::MouseButton {
    match button {
        MouseButton::Left => rinch_platform::MouseButton::Left,
        MouseButton::Middle => rinch_platform::MouseButton::Middle,
        MouseButton::Right => rinch_platform::MouseButton::Right,
    }
}

/// How many turns [`TestApp::settle`] runs at most before it gives up waiting
/// for the app to stop asking for frames (a running animation never does).
const SETTLE_TURNS: usize = 16;

/// How many timers one [`TestApp::advance_time`] fires at most: past this, a
/// timer is re-arming itself with no delay and the advance would never end.
const MAX_TIMERS_PER_ADVANCE: usize = 100_000;

/// How many rounds of window requests one input serves at most: a request
/// whose effect asks for another, which asks for the first again, would
/// otherwise never end.
const MAX_REQUEST_ROUNDS: usize = 64;

pub use crate::window_request::WindowRequest;

/// Where a [`TestApp`] takes the native events its app's code sends: the
/// requests for its window.
struct Sink {
    /// The key of the app's document, once it is mounted (`0` before).
    doc: Cell<u64>,
    /// Each event with the window it is for.
    events: RefCell<Vec<(EventFor, RinchNativeEvent)>>,
    /// Every window this app's component opened, open or closed: its id and
    /// its document's key (`0` until it is made), so a request for one, or
    /// from its document's code, reaches this app and not another on the
    /// thread. A closed one's stays: a request for it is still this app's to
    /// record, and what its document's cleanups send as it closes is this
    /// app's (for its main window, as the shell takes a sender it no longer
    /// knows).
    opened: RefCell<Vec<(u64, u64)>>,
    /// Mounting a child now: what the child's mount sends from its document
    /// (whose key `opened` has not got yet) is this app's.
    opening: Cell<bool>,
}

impl Sink {
    /// Whether `doc` is this app's document or one of its children's.
    fn has_doc(&self, doc: u64) -> bool {
        self.doc.get() == doc || self.opened.borrow().iter().any(|&(_, d)| d == doc)
    }

    /// Whether this app opened window `id`.
    fn opened(&self, id: u64) -> bool {
        self.opened.borrow().iter().any(|&(i, _)| i == id)
    }
}

/// Whether a [`TestApp`] on this thread takes the window requests, for the
/// ones the desktop sends only while its event loop runs (a stored title's
/// `ApplyWindowTitle`).
pub(crate) fn captures() -> bool {
    SINKS
        .try_with(|sinks| !sinks.borrow().is_empty())
        .unwrap_or(false)
}

/// Whether a [`TestApp`] on this thread drives the windows, standing in a
/// Wayland compositor with layer shell (see
/// [Windows a component opens](self#windows-a-component-opens)): the shell's
/// own decisions that ask whether the layer shell is offered (a child's
/// largest size, from its layer surface and the output that names) take it
/// as offered here (#2836).
pub(crate) fn stands_in_layer_shell() -> bool {
    captures()
}

thread_local! {
    /// The live `TestApp`s of this thread, newest last.
    static SINKS: RefCell<Vec<Rc<Sink>>> = const { RefCell::new(Vec::new()) };
}

/// Take `event` for a [`TestApp`] on this thread, if one is live: the one
/// whose window it is for, else the newest. Answers the event back when there
/// is none, for the shell's queue. Called by `send_native_event_for`.
///
/// Whose window it is for: a sender's document is an app's own or one of its
/// children's (open or closed); a window named by id is one an app opened
/// (open or closed). Failing that, the app whose code is running: the
/// opener, for a window not made yet (its `OpenWindow`, or a request through
/// its handle before it is made), and the app whose code sent an event that
/// names no window (`rinch::exit`). Then the app mounting a child now (what
/// the child's mount sends before its document's key is known).
pub(crate) fn capture_native_event(
    to: EventFor,
    event: RinchNativeEvent,
) -> Option<RinchNativeEvent> {
    let running = rinch_core::current_dispatching_doc();
    let sink = SINKS
        .try_with(|sinks| {
            let sinks = sinks.borrow();
            let by_doc = |doc: u64| sinks.iter().rev().find(|s| s.has_doc(doc));
            let owner = match to {
                EventFor::Sender(Some(doc)) => by_doc(doc),
                EventFor::Window(id) => sinks.iter().rev().find(|s| s.opened(id)),
                EventFor::Sender(None) => None,
            };
            owner
                .or_else(|| running.and_then(by_doc))
                .or_else(|| sinks.iter().rev().find(|s| s.opening.get()))
                .or(sinks.last())
                .cloned()
        })
        .ok()
        .flatten();
    match sink {
        Some(sink) => {
            sink.events.borrow_mut().push((to, event));
            None
        }
        None => Some(event),
    }
}

/// A component mounted into a headless app, for a test to drive.
///
/// Built by [`TestApp::mount`] (an 800x600 window at scale 1) or
/// [`TestApp::builder`]. See the [module documentation](self).
///
/// Positions are logical (CSS) pixels at every scale factor — the space
/// [`TestApp::bounds`] answers in and the pointer arrives in on every host —
/// so a test reads a box and presses inside it without converting anything.
pub struct TestApp {
    app: RinchApp,
    /// Logical size.
    size: (u32, u32),
    scale: f64,
    /// The modifiers held, as the shell's `ModifiersChanged` last reported.
    modifiers: Modifiers,
    actions: Vec<AppAction>,
    /// The window's own requests since the last `take_window_requests`.
    requests: Vec<WindowRequest>,
    /// Whether the window is shown.
    visible: bool,
    /// Whether the window has the keyboard, as last sent to the app.
    focused: bool,
    /// Shown without taking the keyboard (`WindowProps::non_activating`).
    non_activating: bool,
    /// Where this app's window requests arrive.
    sink: Rc<Sink>,
    /// Serving window requests now (they are served one round at a time).
    serving: bool,
    /// What the frames painted since the last take counted: for the
    /// in-crate test that pins what a turn does.
    #[cfg(test)]
    painted: rinch_dom::perf::FrameStats,
    /// How many times a turn has run a child window's part: for the
    /// in-crate test that a single-window app never enters it.
    #[cfg(test)]
    child_turns: usize,
    /// The harness's own work, which no DOM counter sees: how many
    /// `AboutToWait`s it has sent the main window (every event it sends the
    /// main window goes through `dispatch`), and how many layouts it has
    /// asked of it (all through `layout`), for the in-crate test that pins
    /// what a turn does.
    #[cfg(test)]
    harness_work: HarnessWork,
    /// The fonts the app bundles, for the windows it opens too.
    fonts: Vec<AppFont>,
    /// The windows the component opened that are open, oldest first.
    children: Vec<children::Child>,
    /// The window's software presentation while it is shown, as the shell's
    /// `SoftbufferRenderer` keeps one: made by the first present after a
    /// show, its buffers given back by the idle trim (`WindowProps::idle_trim`,
    /// #2843) and made again by the next present. Its surface keeps its
    /// contents, as a Wayland one does, so it is trimmed.
    presentation: Option<IdleTrimmed<NoSurface>>,
}

/// The harness's stand-in for a window's softbuffer surface: it presents
/// nothing.
struct NoSurface;

impl PresentTarget for NoSurface {
    fn present(&mut self, _pixels: &[u8], _width: u32, _height: u32) {}
}

/// The harness's own work on the main window, counted for the in-crate test
/// that pins what a turn does.
#[cfg(test)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct HarnessWork {
    /// `AboutToWait`s sent to the main window.
    about_to_waits: usize,
    /// Layouts asked of it (`resolve_and_repaint`).
    layouts: usize,
}

/// The largest size of `app`'s window when its content sizes it, by the
/// shell's own decision for every window
/// ([`content_max_of`](crate::shell::rinch_runtime::content_max_of)): the
/// harness's main window has no layer position of its own and no output
/// chosen at run time. `None` for any other window.
fn content_max_of(app: &RinchApp) -> Option<(f32, f32)> {
    crate::shell::rinch_runtime::content_max_of(app, None, None)
}
