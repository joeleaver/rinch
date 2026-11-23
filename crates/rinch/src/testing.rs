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
mod idle_trim_perf_2852_tests;
#[cfg(test)]
mod perf_tests;
#[cfg(test)]
mod popup_tests;

mod children;
mod played_outputs;
mod test_window;

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

/// The component a [`TestAppBuilder`] mounts.
type BoxedComponent = Box<dyn FnOnce(&mut RenderScope) -> NodeHandle>;

/// Options for a [`TestApp`], from [`TestApp::builder`].
pub struct TestAppBuilder {
    component: BoxedComponent,
    /// The window's size, when set by [`Self::size`] (else the props').
    size: Option<(u32, u32)>,
    scale: f64,
    fonts: Vec<AppFont>,
    #[cfg(feature = "theme")]
    theme_css: Option<String>,
    props: WindowProps,
}

impl TestAppBuilder {
    /// Open the component in the window an [`App`](crate::App) with `props`
    /// would open (see [The window](self#the-window)). What a test can see of
    /// them: the size (`width`, `height`; [`Self::size`] wins whichever comes
    /// first), `visible: false` (mounted hidden: not painted and without the
    /// keyboard, until a [`WindowRequest::Show`]), `non_activating` (no
    /// keyboard at the mount or on a show), `size_to_content` (the window
    /// takes its content's size, and follows it as the content changes; an
    /// anchored layer surface in `wayland_layer` fills its anchored axes),
    /// `transparent` and `idle_trim` (the shown window's presentation is
    /// trimmed on the virtual clock as a Wayland software window's is, which
    /// only `RINCH_PERF`'s `trimmed`/`untrimmed` lines show). The rest are kept (read them back with
    /// [`TestApp::window_props`]) and do nothing here: there is no
    /// compositor.
    pub fn window_props(mut self, props: WindowProps) -> Self {
        self.props = props;
        self
    }

    /// The window's size in logical (CSS) pixels. Default 800x600.
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.size = Some((width.max(1), height.max(1)));
        self
    }

    /// The display's scale factor (device pixel ratio). Default `1.0`.
    /// Layout and input stay in logical pixels; media queries on resolution
    /// and the painted frame see the scale.
    pub fn scale_factor(mut self, scale: f64) -> Self {
        assert!(
            scale.is_finite() && scale > 0.0,
            "a scale factor is positive"
        );
        self.scale = scale;
        self
    }

    /// Fonts the app bundles, as [`crate::App::fonts`] registers them.
    ///
    /// A test that compares horizontal text geometry should bundle its font
    /// (`AppFont::sans_serif(..)`): otherwise widths depend on the fonts the
    /// host has installed.
    pub fn fonts(mut self, fonts: &[AppFont]) -> Self {
        self.fonts.extend_from_slice(fonts);
        self
    }

    /// The theme, as [`crate::App::theme`] sets it: the CSS variables the
    /// components read.
    #[cfg(feature = "theme")]
    pub fn theme(mut self, theme: crate::ThemeProviderProps) -> Self {
        self.theme_css = Some(crate::generate_theme_css_string(&theme));
        self
    }

    /// Mount the component, lay it out and paint its first frame, as the
    /// shell does when the window opens, and give the window keyboard focus
    /// — except a window the [props](Self::window_props) start hidden (mounted
    /// only) or make non-activating (painted, without the keyboard).
    ///
    /// The window publishes its size, scale and focus to
    /// [`rinch::window`](crate::window) from here on, and the thread's timers
    /// run on the app's virtual clock (see [Time](self#time)).
    pub fn mount(self) -> TestApp {
        let TestAppBuilder {
            component,
            size,
            scale,
            fonts,
            #[cfg(feature = "theme")]
            theme_css,
            props,
        } = self;
        let size = size.unwrap_or((props.width.max(1), props.height.max(1)));
        // Made as the shell makes its main window (`build_runtime`): in the
        // content-sized mode when the props ask, with the props on it.
        let mut app = crate::shell::rinch_runtime::app_for_window(component, &props);
        app.register_app_fonts(&fonts);
        #[cfg(feature = "theme")]
        if let Some(css) = theme_css {
            app.set_owned_theme_css(Some(css));
        }
        let visible = props.visible;
        let non_activating = props.non_activating;
        app.set_window_props(props);
        // This thread drains its own main-thread callbacks and writes its own
        // signals, beside other tests' apps on other threads (#2778).
        rinch_core::mark_host_thread();
        // The window's requests, and its timers, are this app's from the
        // mount on: a component may ask for either while it mounts.
        let sink = Rc::new(Sink {
            doc: Cell::new(0),
            events: RefCell::new(Vec::new()),
            opened: RefCell::new(Vec::new()),
            opening: Cell::new(false),
        });
        SINKS.with(|s| s.borrow_mut().push(sink.clone()));
        virtual_time::install();
        // The shell's order (`ShellWindow::create_window`): the scale first,
        // so the first style resolution sees the device pixel ratio (#211),
        // then the mount at the logical size. The window's state is published
        // as the shell's main window publishes it (#2742), and an unfocused
        // window is what a previous test on this thread may have left.
        app.set_device_pixel_ratio(scale);
        app.set_publishes_window_state(true);
        app.publish_window_state(WindowFacts {
            focused: Some(false),
            ..Default::default()
        });
        // A content-sized window is mounted at its largest size for the
        // output it is on, as the shell's `mount_document_before_window`
        // mounts it: the outputs another app on this thread played are
        // listed already (#2836).
        let mount_size = match content_max_of(&app) {
            Some(max) => {
                app.restart_content_limit(max);
                max
            }
            None => (size.0 as f32, size.1 as f32),
        };
        // The harness has no surface: its stand-in for one is the logical size
        // times the scale, rounded (`physical_size`).
        app.set_surface_size((
            (f64::from(mount_size.0) * scale).round() as u32,
            (f64::from(mount_size.1) * scale).round() as u32,
        ));
        app.mount_component(mount_size.0, mount_size.1);
        sink.doc.set(app.doc_key());
        let mut test = TestApp {
            app,
            size,
            scale,
            modifiers: Modifiers::default(),
            actions: Vec::new(),
            requests: Vec::new(),
            visible,
            focused: false,
            non_activating,
            sink,
            serving: false,
            #[cfg(test)]
            painted: rinch_dom::perf::FrameStats::default(),
            #[cfg(test)]
            child_turns: 0,
            #[cfg(test)]
            harness_work: HarnessWork::default(),
            fonts,
            children: Vec::new(),
            presentation: None,
        };
        if test.app.is_content_sized() {
            test.app.resolve_and_repaint(mount_size.0, mount_size.1);
            test.negotiate_content_size();
        }
        if visible {
            test.paint();
            // A window the user is typing into or clicking has focus.
            if !non_activating {
                test.set_window_focus(true);
            }
        }
        test.serve_window_requests();
        test
    }
}

/// The largest size of `app`'s window when its content sizes it, by the
/// shell's own decision for every window
/// ([`content_max_of`](crate::shell::rinch_runtime::content_max_of)): the
/// harness's main window has no layer position of its own and no output
/// chosen at run time. `None` for any other window.
fn content_max_of(app: &RinchApp) -> Option<(f32, f32)> {
    crate::shell::rinch_runtime::content_max_of(app, None, None)
}

/// What a [`TestApp`] method acts on: a CSS selector (`"#row"`, `".item"`,
/// the first match), or a [`NodeHandle`] the component kept.
///
/// Sealed: implemented for `&str`, `String`, `NodeHandle` and `&NodeHandle`
/// only.
pub trait Target: sealed::Sealed {
    /// The node this names in `app`'s document. Panics when it names none.
    fn node_id(&self, app: &TestApp) -> NodeId {
        self.in_doc(app.doc())
    }
}

mod sealed {
    use super::{NodeId, Rc, RefCell, RinchDocument};

    pub trait Sealed {
        /// The node this names in `doc`. Panics when it names none.
        fn in_doc(&self, doc: &Rc<RefCell<RinchDocument>>) -> NodeId;
    }

    impl Sealed for &str {
        fn in_doc(&self, doc: &Rc<RefCell<RinchDocument>>) -> NodeId {
            super::read::query_all(doc, self)
                .into_iter()
                .next()
                .unwrap_or_else(|| panic!("no element matches {self:?}"))
                .node_id()
        }
    }

    impl Sealed for String {
        fn in_doc(&self, doc: &Rc<RefCell<RinchDocument>>) -> NodeId {
            self.as_str().in_doc(doc)
        }
    }

    impl Sealed for super::NodeHandle {
        fn in_doc(&self, doc: &Rc<RefCell<RinchDocument>>) -> NodeId {
            // A node id names a node in one document only: a handle from
            // another window's document would read some other node here.
            let (mine, here) = (self.doc_key(), doc.borrow().doc_key());
            assert_eq!(
                mine, here,
                "a node of document {mine} used on the window of document {here}"
            );
            super::NodeHandle::node_id(self)
        }
    }

    impl Sealed for &super::NodeHandle {
        fn in_doc(&self, doc: &Rc<RefCell<RinchDocument>>) -> NodeId {
            (*self).clone().in_doc(doc)
        }
    }
}

impl Target for &str {}

impl Target for String {}

impl Target for NodeHandle {}

impl Target for &NodeHandle {}

/// Reading a document, the main window's or a child's.
mod read {
    use super::{DomDocument, ElementBounds, NodeHandle, NodeId, Rc, RefCell, RinchDocument, Weak};

    /// `doc` as a weak handle, for [`NodeHandle`]s.
    pub(super) fn weak(doc: &Rc<RefCell<RinchDocument>>) -> Weak<RefCell<dyn DomDocument>> {
        let doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
        Rc::downgrade(&doc)
    }

    /// Every element matching `selector`, in document order.
    pub(super) fn query_all(doc: &Rc<RefCell<RinchDocument>>, selector: &str) -> Vec<NodeHandle> {
        let ids = doc.borrow().query_selector_all(selector);
        let weak = weak(doc);
        ids.into_iter()
            .map(|id| NodeHandle::new(id, weak.clone()))
            .collect()
    }

    /// The text in `id`'s subtree.
    pub(super) fn text(doc: &Rc<RefCell<RinchDocument>>, id: NodeId) -> String {
        rinch_dom::testing::get_text_content(&doc.borrow().tree, id.0)
    }

    /// `id`'s attribute `name`.
    pub(super) fn attribute(
        doc: &Rc<RefCell<RinchDocument>>,
        id: NodeId,
        name: &str,
    ) -> Option<String> {
        let d = doc.borrow();
        d.tree.get(id.0)?.attributes.get(name).cloned()
    }

    /// The box `id` is painted in, in logical pixels from its window's
    /// top-left.
    pub(super) fn bounds(doc: &Rc<RefCell<RinchDocument>>, id: NodeId) -> ElementBounds {
        let (x, y, width, height) = crate::app::painted_element_box(&doc.borrow().tree, id.0);
        ElementBounds {
            x,
            y,
            width,
            height,
        }
    }
}

impl TestApp {
    /// Mount `component` in an 800x600 window at scale 1.
    pub fn mount(component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> Self {
        Self::builder(component).mount()
    }

    /// Choose the window's props, size, scale factor, fonts or theme before
    /// mounting.
    pub fn builder(
        component: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static,
    ) -> TestAppBuilder {
        TestAppBuilder {
            component: Box::new(component),
            size: None,
            scale: 1.0,
            fonts: Vec::new(),
            #[cfg(feature = "theme")]
            theme_css: None,
            props: WindowProps {
                width: DEFAULT_SIZE.0,
                height: DEFAULT_SIZE.1,
                ..WindowProps::default()
            },
        }
    }

    // ── The event loop ──────────────────────────────────────────────────

    /// The window size the shell hands `handle_event`: physical pixels.
    ///
    /// Also the harness's stand-in for its surface's size, which it has no
    /// surface to read: what
    /// [`window_physical_size`](crate::window::window_physical_size) answers
    /// under a `TestApp`. The shell publishes the surface's own size instead.
    fn physical_size(&self) -> (u32, u32) {
        (
            (self.size.0 as f64 * self.scale).round() as u32,
            (self.size.1 as f64 * self.scale).round() as u32,
        )
    }

    /// Hand `event` to the app as the shell's `dispatch_event` does, then run
    /// one turn of the loop.
    fn send(&mut self, event: PlatformEvent) {
        let asked = self.dispatch(event);
        self.turn(asked);
        self.serve_window_requests();
    }

    /// Send the window gaining or losing the keyboard, unless it already
    /// has (or lacks) it.
    fn set_window_focus(&mut self, focused: bool) {
        if self.focused != focused {
            self.focused = focused;
            self.send(PlatformEvent::WindowFocus(focused));
        }
    }

    /// The requests the app's code made of its window since this last ran:
    /// recorded, and the ones that change what a test sees applied. What an
    /// applied request runs may ask for more, which are served in the next
    /// round.
    fn serve_window_requests(&mut self) {
        if self.serving {
            return;
        }
        self.serving = true;
        for _ in 0..MAX_REQUEST_ROUNDS {
            let events = std::mem::take(&mut *self.sink.events.borrow_mut());
            if events.is_empty() {
                break;
            }
            for (to, event) in events {
                if let RinchNativeEvent::OpenWindow(id) = event {
                    // A window the component opened: made after the turn
                    // that built it, as the shell makes it on a later wake.
                    self.sink.opened.borrow_mut().push((id, 0));
                    self.sink.opening.set(true);
                    let opened = children::Child::open(id, self);
                    self.sink.opening.set(false);
                    if let Some(child) = opened {
                        let doc = child.doc();
                        for entry in self.sink.opened.borrow_mut().iter_mut() {
                            if entry.0 == id {
                                entry.1 = doc;
                            }
                        }
                        self.children.push(child);
                        if let Some(child) = self.children.last_mut() {
                            child.turn();
                        }
                    }
                    continue;
                }
                let (window, doc) = match to {
                    EventFor::Window(window) => (Some(window), None),
                    EventFor::Sender(doc) => (None, doc),
                };
                if let Some(at) = self.child_at(window, doc) {
                    // An open child's own request, through its handle or
                    // from its document's code: its own, never the app's.
                    if self.children[at].native(event) {
                        self.close_child(at);
                    }
                } else if let Some(window) = window {
                    // A window a `WindowHandle` names that is not open here
                    // (closed, or never made): recorded only.
                    if let Some(request) = WindowRequest::from_native(event) {
                        self.requests.push(WindowRequest::OtherWindow {
                            window,
                            request: Box::new(request),
                        });
                    }
                } else if let RinchNativeEvent::InjectedPlatformEvent(event) = event {
                    // `inject_platform_event`: the shell dispatches it.
                    self.send(event);
                } else if matches!(event, RinchNativeEvent::ApplyWindowTitle) {
                    // `rinch::window::set_window_title` in this app's
                    // document: the title it stored is the window's title
                    // request. The store is the process's, so only this
                    // document's titles are taken, never another test's.
                    let doc = self.sink.doc.get();
                    if let Some(title) = crate::window::take_stored_title(|d| d == Some(doc)) {
                        self.requests.push(WindowRequest::SetTitle(title));
                    }
                } else if let Some(request) = WindowRequest::from_native(event) {
                    self.requests.push(request.clone());
                    self.apply(request);
                }
            }
        }
        self.serving = false;
    }

    /// Apply a window request to the test's window, as far as a test can see
    /// it.
    fn apply(&mut self, request: WindowRequest) {
        match request {
            WindowRequest::Show if !self.visible => {
                self.visible = true;
                // A new surface: a content-sized window measures afresh.
                if self.app.is_content_sized() {
                    self.restart_content_size();
                }
                self.paint();
                if !self.non_activating {
                    self.set_window_focus(true);
                }
            }
            WindowRequest::Hide if self.visible => {
                self.visible = false;
                // The shell drops a hidden window's renderer, its timer with
                // it.
                self.presentation = None;
                // A hidden window has no keyboard.
                self.set_window_focus(false);
            }
            // A hidden window is shown by it; a non-activating one never
            // takes the keyboard.
            WindowRequest::Focus | WindowRequest::ActivateWithToken(_) if !self.visible => {
                self.apply(WindowRequest::Show)
            }
            WindowRequest::Focus | WindowRequest::ActivateWithToken(_) if !self.non_activating => {
                self.set_window_focus(true)
            }
            WindowRequest::Prepare(_) if !self.visible => {
                if self.app.is_content_sized() {
                    self.restart_content_size();
                }
                self.paint_now();
            }
            WindowRequest::SetSize(width, height) if !self.app.is_content_sized() => {
                self.resize(width.round() as u32, height.round() as u32);
            }
            _ => {}
        }
    }

    /// A content-sized window's new negotiation (a show, a prepare): free
    /// again at its largest size, then the content's size there.
    fn restart_content_size(&mut self) {
        if let Some(max) = self.content_max() {
            self.app.restart_content_limit(max);
        }
        self.negotiate_content_size();
    }

    /// A content-sized window's largest size, as the shell's
    /// `ShellWindow::content_max` gives it: the app's maximum, else the size
    /// of the output it is on among those played
    /// ([`TestApp::set_outputs`]), else the props' size when none is.
    fn content_max(&self) -> Option<(f32, f32)> {
        content_max_of(&self.app)
    }

    /// The outputs changed: a content-sized window whose largest size
    /// changed starts a new negotiation, which the next turn's
    /// `follow_content_size` grants, as `ShellWindow::follow_outputs` does.
    fn follow_outputs(&mut self) {
        let Some(max) = self.content_max() else {
            return;
        };
        if self.app.content_largest() != Some(max) {
            self.app.restart_content_limit(max);
            self.app.remeasure_content();
        }
    }

    /// Give a content-sized window its content's size, as the shell does when
    /// it makes the surface: measured, granted, and the window resized to it.
    fn negotiate_content_size(&mut self) {
        if let Some((width, height)) = self.app.content_size() {
            self.grant(width, height);
        }
    }

    /// After a turn: a content-sized window whose content changed size asks
    /// for the new size, and is given it at once, as a layer surface or a
    /// plain toplevel is (`RinchRuntime::follow_content_size`). A hidden
    /// window asks nothing: its show measures.
    fn follow_content_size(&mut self) -> bool {
        if !self.visible || !self.app.is_content_sized() {
            return false;
        }
        if self.app.has_pending_layout() {
            self.layout();
        }
        match self.app.content_size_changed() {
            Some((width, height)) => {
                self.grant(width, height);
                true
            }
            None => false,
        }
    }

    /// The window given `width`x`height` logical pixels: the document's grant,
    /// then the `Resized` the shell sends for a `SurfaceResized`.
    fn grant(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        self.app.set_granted_size(width as f32, height as f32);
        self.size = (width, height);
        self.dispatch(PlatformEvent::Resized { width, height });
    }

    /// `handle_event` alone, keeping what the app asked for.
    fn dispatch(&mut self, event: PlatformEvent) -> bool {
        // The shell sets the surface's size before the `Resized` its
        // `SurfaceResized` makes.
        if matches!(event, PlatformEvent::Resized { .. }) {
            self.app.set_surface_size(self.physical_size());
        }
        #[cfg(test)]
        if matches!(event, PlatformEvent::AboutToWait) {
            self.harness_work.about_to_waits += 1;
        }
        let actions = self
            .app
            .handle_event(event, self.physical_size(), self.scale);
        self.note(actions)
    }

    /// Lay the document out at the window's size and repaint what changed
    /// (`resolve_and_repaint`), as the shell does before a frame. Every
    /// layout the harness asks of the main window after mounting is asked
    /// here, where the in-crate test counts it.
    fn layout(&mut self) {
        self.app
            .resolve_and_repaint(self.size.0 as f32, self.size.1 as f32);
        #[cfg(test)]
        {
            self.harness_work.layouts += 1;
        }
    }

    /// Keep `actions`; answer whether they ask for a frame.
    fn note(&mut self, actions: Vec<AppAction>) -> bool {
        let redraw = actions.contains(&AppAction::RequestRedraw);
        self.actions.extend(actions);
        redraw
    }

    /// One turn of the desktop loop after an event: the wake's queues
    /// (`proxy_wake_up`), `AboutToWait`, and the frame if anything asked for
    /// one — `redraw_asked` is whether the event itself did (the shell's
    /// `process_actions` requests that redraw before the loop turns). What the
    /// drain left to draw asks for the frame too, as `proxy_wake_up` decides
    /// it (`RinchApp::wake_wants_frame`: a signal that changed nothing on
    /// screen asks for none, #2608). Answers whether this turn painted.
    fn turn(&mut self, redraw_asked: bool) -> bool {
        // The timers due now (their native thread queues them as main-thread
        // callbacks), the main-thread callbacks, then the work other threads
        // sent this app (#328) — `RinchRuntime::proxy_wake_up`'s order.
        self.fire_due_timers();
        rinch_core::drain_main_callbacks();
        let ran = self.app.run_deferred_work();
        let asked = self.dispatch(PlatformEvent::AboutToWait);
        let resized = self.follow_content_size();
        let painted = if (redraw_asked || asked || resized || self.app.wake_wants_frame(ran > 0))
            && self.visible
        {
            self.paint();
            true
        } else {
            // An idle window gives its buffers back (#2843), as
            // `proxy_wake_up` serves the trim when no frame is coming.
            if let Some(presentation) = &mut self.presentation {
                presentation.service_idle_trim(rinch_core::timer::now());
                if presentation.take_trimmed() {
                    crate::window_perf::trimmed(&self.app);
                }
            }
            false
        };
        // Each open child's part of the turn, short of painting (#2793).
        for child in &mut self.children {
            child.turn();
            #[cfg(test)]
            {
                self.child_turns += 1;
            }
        }
        painted
    }

    /// After input to a child window: a turn of the loop, then its
    /// requests.
    fn after_child_input(&mut self) {
        self.turn(false);
        self.serve_window_requests();
    }

    /// Fire the timers due at the virtual clock's time now, armed before this
    /// call (one armed by them waits for the next turn, as on the desktop).
    fn fire_due_timers(&mut self) {
        let Some(now) = virtual_time::elapsed() else {
            return;
        };
        let armed_before = virtual_time::next_seq();
        while let Some(id) = virtual_time::take_due(now, armed_before) {
            rinch_core::timer::fire_timeout(id);
        }
    }

    /// One redraw, as `ShellWindow::paint_software` does it: the queues it
    /// drains first, the layout it resolves, the frame. Nothing while the
    /// window is hidden: a hidden window presents nothing.
    fn paint(&mut self) {
        if self.visible {
            self.paint_now();
        }
    }

    /// [`Self::paint`], shown or not: a prepare paints the hidden window.
    fn paint_now(&mut self) {
        rinch_core::drain_main_callbacks();
        rinch_core::reactive::drain_polls();
        if self.app.has_pending_layout() {
            self.layout();
        }
        let transparent = self.app.is_transparent();
        let (_, w, h) = self.app.build_pixels(self.scale, self.size, transparent);
        // A shown window presents; a prepared one does not.
        if self.visible {
            self.present(w, h);
        }
        let _frame = self.app.end_perf_frame();
        #[cfg(test)]
        if let Some(frame) = _frame {
            self.painted.accumulate(&frame);
        }
        #[cfg(test)]
        if _frame.is_some() && crate::window_perf::capture::listening() {
            let doc = self.doc().borrow();
            let perf = &doc.tree.perf;
            crate::window_perf::capture::note(&match perf.label() {
                Some(label) => format!("[PERF] {label} frame {}", perf.frames()),
                None => format!("[PERF] frame {}", perf.frames()),
            });
        }
    }

    /// Present the frame just painted (`w` x `h`), as the shell's
    /// `paint_software_at` does: the presentation made at the first present,
    /// with its `[PERF] idle_trim=` line (#2917), its `[PERF] trimmed` line
    /// when this redraw presented nothing and served the idle trim instead,
    /// and its `[PERF] untrimmed` line when this present made its buffers
    /// again after a trim (#2852).
    fn present(&mut self, w: u32, h: u32) {
        let presentation = self.presentation.get_or_insert_with(|| {
            let period = self
                .app
                .window_props
                .as_ref()
                .map_or_else(Default::default, |p: &WindowProps| p.idle_trim)
                .period();
            let presentation = IdleTrimmed::new(Presenter::new(true, period, |_, _| NoSurface));
            crate::window_perf::idle_trim(&self.app, presentation.idle_period());
            presentation
        });
        let generation = self.app.pixels_generation();
        presentation.present_frame(&[], w, h, generation, rinch_core::timer::now());
        if presentation.take_trimmed() {
            crate::window_perf::trimmed(&self.app);
        }
        if presentation.take_untrimmed() {
            crate::window_perf::untrimmed(&self.app);
        }
    }

    /// Run turns of the loop until the app stops asking for frames — what
    /// an idle event loop does after the last input — or 16 turns have run (an
    /// animation that is still going asks every turn).
    ///
    /// Every input method already runs one turn; call this after a change made
    /// outside input (a signal written by the test) or to let a short
    /// transition finish.
    pub fn settle(&mut self) {
        for _ in 0..SETTLE_TURNS {
            let painted = self.turn(false);
            self.serve_window_requests();
            if !painted && !rinch_core::main_callbacks_pending() {
                return;
            }
        }
    }

    /// Move the virtual clock forward by `by`, firing every `set_timeout` and
    /// `set_interval` timer that comes due on the way: in deadline order
    /// (timers due together in the order they were armed), the clock at each
    /// one's deadline when it fires, and a turn of the loop after each
    /// deadline, so effects, layout and window requests are current before a
    /// later timer fires (the timers due at one instant fire in one turn). A
    /// timer one of them arms fires too if it comes due inside `by`. See
    /// [Time](self#time).
    ///
    /// Panics after 100 000 timers in one call: a timer re-arming itself
    /// with no delay never lets the clock move.
    pub fn advance_time(&mut self, by: Duration) {
        let Some(start) = virtual_time::elapsed() else {
            return;
        };
        let until = start + by;
        let mut fired = 0;
        while let Some(id) = virtual_time::take_due(until, u64::MAX) {
            fired += 1;
            assert!(
                fired <= MAX_TIMERS_PER_ADVANCE,
                "advance_time fired {MAX_TIMERS_PER_ADVANCE} timers without reaching \
                 its end: a timer is re-arming itself with no delay"
            );
            rinch_core::timer::fire_timeout(id);
            self.turn(false);
            self.serve_window_requests();
        }
        virtual_time::advance_to(until);
        self.turn(false);
        self.serve_window_requests();
    }

    /// The virtual clock's time: how far [`Self::advance_time`] has moved it
    /// since the mount (since the first mount, when another `TestApp` on this
    /// thread was alive then and shares its clock).
    pub fn elapsed(&self) -> Duration {
        virtual_time::elapsed().unwrap_or_default()
    }

    /// The requests the app's code made of its own window since the last
    /// call (`hide_current_window`, `show_current_window`,
    /// `prepare_current_window`, a size, a position, …), oldest first. Shows,
    /// hides, prepares, focus and size requests have also been applied (see
    /// [The window](self#the-window)); the rest are only recorded.
    pub fn take_window_requests(&mut self) -> Vec<WindowRequest> {
        std::mem::take(&mut self.requests)
    }

    /// The [`WindowProps`] the app was mounted with
    /// ([`TestAppBuilder::window_props`], the defaults otherwise), as given:
    /// [`Self::is_visible`] and [`Self::size`] follow what happened
    /// since, these do not.
    pub fn window_props(&self) -> &WindowProps {
        self.app
            .window_props
            .as_ref()
            .expect("a TestApp's app is given its props at the mount")
    }

    /// Whether the window is shown: from the props' `visible`, then each
    /// show and hide the app asked for.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Whether the window has the keyboard: what
    /// [`window_focused`](crate::window::window_focused) answers the app.
    pub fn is_window_focused(&self) -> bool {
        self.focused
    }

    /// The actions the app asked the window for since the last call
    /// (a cursor change, a window drag, exit), oldest first, without the
    /// frame requests the harness itself serves.
    pub fn take_actions(&mut self) -> Vec<AppAction> {
        std::mem::take(&mut self.actions)
            .into_iter()
            .filter(|a| *a != AppAction::RequestRedraw)
            .collect()
    }

    /// The app being driven, for anything this harness does not wrap.
    pub fn app(&mut self) -> &mut RinchApp {
        &mut self.app
    }

    /// The window size, logical pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The scale factor.
    pub fn scale_factor(&self) -> f64 {
        self.scale
    }

    /// Resize the window to `width`x`height` logical pixels, as a
    /// `SurfaceResized` does. [`window_size`](crate::window::window_size)
    /// follows.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.app
            .set_granted_size(self.size.0 as f32, self.size.1 as f32);
        self.send(PlatformEvent::Resized {
            width: self.size.0,
            height: self.size.1,
        });
    }

    /// Move the window to a display with scale factor `scale`, as a
    /// `ScaleFactorChanged` does. The logical size stays;
    /// [`scale_factor`](crate::window::scale_factor) follows.
    pub fn set_scale_factor(&mut self, scale: f64) {
        assert!(
            scale.is_finite() && scale > 0.0,
            "a scale factor is positive"
        );
        self.scale = scale;
        self.send(PlatformEvent::ScaleFactorChanged(scale));
        // The surface's new size, which the shell's `SurfaceResized` after a
        // `ScaleFactorChanged` publishes.
        self.app.set_surface_size(self.physical_size());
        self.app.publish_window_state(WindowFacts {
            physical_size: Some(self.physical_size()),
            ..Default::default()
        });
    }

    /// The window gaining (`true`) or losing keyboard focus.
    /// [`window_focused`](crate::window::window_focused) follows.
    ///
    /// A hidden window cannot gain the keyboard: `window_focus(true)` does
    /// nothing while [`Self::is_visible`] is false.
    pub fn window_focus(&mut self, focused: bool) {
        if focused && !self.visible {
            return;
        }
        self.focused = focused;
        self.send(PlatformEvent::WindowFocus(focused));
    }

    // ── Pointer ─────────────────────────────────────────────────────────

    /// Move the mouse to `(x, y)`.
    pub fn mouse_move(&mut self, x: f32, y: f32) {
        self.send(PlatformEvent::MouseMove { x, y });
    }

    /// Press `button` at `(x, y)`. The pointer moves there first if it is not
    /// there already: a real button is pressed where the pointer is, and the
    /// shell presses at the position its last move left.
    pub fn mouse_down(&mut self, x: f32, y: f32, button: MouseButton) {
        self.move_unless_there(x, y);
        // The menus a press here closes, closed after it is delivered (#2552).
        let closes = crate::popup::light_dismiss(self.window_id(), Some((x, y)));
        self.send(PlatformEvent::MouseDown {
            x,
            y,
            button: platform_button(button),
        });
        self.close_popups(&closes);
    }

    /// Release `button` at `(x, y)`, moving there first if needed.
    pub fn mouse_up(&mut self, x: f32, y: f32, button: MouseButton) {
        self.move_unless_there(x, y);
        self.send(PlatformEvent::MouseUp {
            x,
            y,
            button: platform_button(button),
        });
    }

    /// A left click at `(x, y)`: press and release.
    pub fn click(&mut self, x: f32, y: f32) {
        self.mouse_down(x, y, MouseButton::Left);
        self.mouse_up(x, y, MouseButton::Left);
    }

    /// A right click at `(x, y)` (the context-menu press).
    pub fn right_click(&mut self, x: f32, y: f32) {
        self.mouse_down(x, y, MouseButton::Right);
        self.mouse_up(x, y, MouseButton::Right);
    }

    /// Move the mouse to the centre of `target`'s box.
    pub fn hover(&mut self, target: impl Target) {
        let (x, y) = self.center(target);
        self.mouse_move(x, y);
    }

    /// Press `button` at the centre of `target`'s box.
    ///
    /// The press goes through hit testing like any other: it lands on
    /// whatever is painted at that point, which is `target` unless something
    /// covers it (or `target` has no box of its own).
    pub fn press(&mut self, target: impl Target, button: MouseButton) {
        let (x, y) = self.center(target);
        self.mouse_down(x, y, button);
    }

    /// Release `button` at the centre of `target`'s box.
    pub fn release(&mut self, target: impl Target, button: MouseButton) {
        let (x, y) = self.center(target);
        self.mouse_up(x, y, button);
    }

    /// A left click at the centre of `target`'s box. See [`Self::press`].
    pub fn click_on(&mut self, target: impl Target) {
        let (x, y) = self.center(target);
        self.click(x, y);
    }

    /// A wheel or trackpad scroll at `(x, y)` of `delta_x`/`delta_y` pixels,
    /// with the DOM's sign: positive `delta_y` scrolls the content down (the
    /// `WheelEvent.deltaY` an `onwheel` handler receives), as the MCP
    /// `scroll` tool takes it. Scrolls directly, like a trackpad.
    pub fn wheel(&mut self, x: f32, y: f32, delta_x: f64, delta_y: f64) {
        self.move_unless_there(x, y);
        // A wheel delta is negative toward "down" (winit's sign, which
        // `handle_event`'s `MouseWheel` arm subtracts from the offset).
        self.send(PlatformEvent::MouseWheel {
            x,
            y,
            delta_x: -delta_x,
            delta_y: -delta_y,
        });
    }

    /// A mouse wheel's notches at `(x, y)`, DOM sign as in [`Self::wheel`]:
    /// the scroll glides, as one from a real wheel does (unless the app
    /// turned smooth wheel scrolling off). The glide runs on the virtual
    /// clock, so [`Self::advance_time`] carries it to its end (see
    /// [Time](self#time)).
    pub fn wheel_lines(&mut self, x: f32, y: f32, lines_x: f64, lines_y: f64) {
        self.move_unless_there(x, y);
        self.send(PlatformEvent::MouseWheelLines {
            x,
            y,
            lines_x: -lines_x,
            lines_y: -lines_y,
        });
    }

    /// [`Self::wheel`] at the centre of `target`'s box.
    pub fn wheel_on(&mut self, target: impl Target, delta_x: f64, delta_y: f64) {
        let (x, y) = self.center(target);
        self.wheel(x, y, delta_x, delta_y);
    }

    fn move_unless_there(&mut self, x: f32, y: f32) {
        if self.app.cursor_pos != Some((x, y)) {
            self.mouse_move(x, y);
        }
    }

    // ── Keyboard ────────────────────────────────────────────────────────

    /// Hold `modifiers` (and release any not in it), as the shell's
    /// `ModifiersChanged` reports a modifier key. Every later key, click and
    /// wheel sees them held until the next call.
    pub fn set_modifiers(&mut self, modifiers: ModifierState) {
        let ModifierState {
            shift,
            ctrl,
            alt,
            meta,
        } = modifiers;
        self.modifiers = Modifiers {
            shift,
            ctrl,
            alt,
            meta,
        };
        self.send(PlatformEvent::ModifiersChanged(self.modifiers));
    }

    /// The modifiers held.
    pub fn modifiers(&self) -> ModifierState {
        let Modifiers {
            shift,
            ctrl,
            alt,
            meta,
        } = self.modifiers;
        ModifierState {
            shift,
            ctrl,
            alt,
            meta,
        }
    }

    /// Press the key `key` names, with the [held modifiers](Self::set_modifiers).
    ///
    /// Names are the W3C `KeyboardEvent.key` spellings: a single character
    /// (`"a"`, `"A"`, `"/"`) is that character, and `"Enter"`, `"Tab"`,
    /// `"Escape"`, `"Backspace"`, `"Delete"`, `"Space"`, `"ArrowLeft"`,
    /// `"ArrowRight"`, `"ArrowUp"`, `"ArrowDown"`, `"Home"`, `"End"`,
    /// `"PageUp"`, `"PageDown"` and `"F1"`–`"F12"` are those keys — the names
    /// the MCP `key_press` tool takes. With Shift held, a letter types its
    /// capital (`"c"` types `"C"`), as a QWERTY layout does; other characters
    /// type themselves (name `"!"`, not Shift+`"1"`). A menu shortcut matching the chord takes it
    /// first, and the app then sees no key, as with a real keyboard.
    ///
    /// Panics on a name it does not know.
    pub fn key_down(&mut self, key: &str) {
        // While a menu popup is open, its topmost menu takes the keys (#2552).
        if let Some(menu) = self.popup_key_target(self.window_id()) {
            self.window_by_id(menu).key_down(key);
            return;
        }
        let mut actions = Vec::new();
        let size = self.physical_size();
        let outcome =
            self.app
                .dispatch_named_key_down(key, self.modifiers, &mut actions, size, self.scale);
        assert!(
            outcome != NamedKeyDown::UnknownKey,
            "unknown key name {key:?}"
        );
        let asked = self.note(actions);
        self.turn(asked);
        self.serve_window_requests();
    }

    /// Release the key `key` names (see [`Self::key_down`]).
    pub fn key_up(&mut self, key: &str) {
        let code = keyname_to_keycode(key).unwrap_or_else(|| panic!("unknown key name {key:?}"));
        self.send(PlatformEvent::KeyUp {
            key: code,
            logical_key: named_logical_key(key, self.modifiers.shift),
            modifiers: self.modifiers,
        });
    }

    /// A keystroke: [`Self::key_down`] then [`Self::key_up`].
    pub fn press_key(&mut self, key: &str) {
        self.key_down(key);
        self.key_up(key);
    }

    /// Type `text`, one keystroke per character: `'\n'` is Enter, `'\t'` Tab,
    /// any other character its own key. With the held modifiers, like
    /// [`Self::press_key`].
    pub fn type_text(&mut self, text: &str) {
        let mut buf = [0u8; 4];
        for ch in text.chars() {
            let key = match ch {
                '\n' => "Enter",
                '\t' => "Tab",
                c => c.encode_utf8(&mut buf),
            };
            let key = key.to_owned();
            self.press_key(&key);
        }
    }

    /// An input method committing `text` to the focused field, as an IME
    /// (or a dictation or emoji picker) does.
    pub fn ime_commit(&mut self, text: &str) {
        self.send(PlatformEvent::Ime(ImeEvent::Commit(text.to_owned())));
    }

    /// Focus `target` as app code does with [`NodeHandle::focus`], then run
    /// a turn.
    pub fn focus(&mut self, target: impl Target) {
        self.handle(target).focus();
        self.turn(true);
        self.serve_window_requests();
    }

    // ── Reading the document ────────────────────────────────────────────

    fn doc(&self) -> &Rc<RefCell<RinchDocument>> {
        self.app.doc().expect("a mounted app has a document")
    }

    fn doc_weak(&self) -> Weak<RefCell<dyn DomDocument>> {
        read::weak(self.doc())
    }

    /// A handle to `target`, for the [`NodeHandle`] API (attributes,
    /// children, scroll offsets, `live_value`, …).
    pub fn handle(&self, target: impl Target) -> NodeHandle {
        NodeHandle::new(target.node_id(self), self.doc_weak())
    }

    /// The first element matching the CSS `selector`, in document order.
    pub fn query(&self, selector: &str) -> Option<NodeHandle> {
        self.query_all(selector).into_iter().next()
    }

    /// Every element matching the CSS `selector`, in document order.
    pub fn query_all(&self, selector: &str) -> Vec<NodeHandle> {
        read::query_all(self.doc(), selector)
    }

    /// The text in `target`'s subtree (`textContent`).
    pub fn text(&self, target: impl Target) -> String {
        read::text(self.doc(), target.node_id(self))
    }

    /// `target`'s attribute `name`.
    pub fn attribute(&self, target: impl Target, name: &str) -> Option<String> {
        read::attribute(self.doc(), target.node_id(self), name)
    }

    /// The value of a form control (`<input>`, `<textarea>`, `<select>`) as the
    /// user left it.
    pub fn value(&self, target: impl Target) -> Option<String> {
        let id = target.node_id(self).0;
        let d = self.doc().borrow();
        rinch_dom::form_control::control_value(&d.tree.nodes, id).map(|v| v.into_owned())
    }

    /// One of `target`'s computed style properties, in the form the MCP
    /// `get_computed_styles` tool reports it: `name` is a field of rinch's
    /// computed style (`"display"`, `"background_color"`; a CSS spelling
    /// such as `"background-color"` is accepted), and a value that is not a
    /// plain string comes back as its JSON (`"12.0"`, `"[0,0,0,255]"`).
    ///
    /// **Unstable:** the names and the value spellings are rinch's internal
    /// computed style, and change when it does. Prefer [`Self::bounds`] or an
    /// attribute where they answer the question.
    pub fn computed_style(&self, target: impl Target, name: &str) -> Option<String> {
        let id = target.node_id(self).0;
        let d = self.doc().borrow();
        let style = serde_json::to_value(&d.tree.get(id)?.computed_style).ok()?;
        match style.get(name.replace('-', "_"))? {
            serde_json::Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        }
    }

    /// The box `target` is painted in (its border box), in logical pixels
    /// from the window's top-left — `getBoundingClientRect()`, the box a
    /// click lands in.
    pub fn bounds(&self, target: impl Target) -> ElementBounds {
        read::bounds(self.doc(), target.node_id(self))
    }

    /// The centre of [`Self::bounds`].
    pub fn center(&self, target: impl Target) -> (f32, f32) {
        let b = self.bounds(target);
        (b.x + b.width / 2.0, b.y + b.height / 2.0)
    }

    /// The element with keyboard focus, if any.
    pub fn focused(&self) -> Option<NodeHandle> {
        let id = self.doc().borrow().active_element()?;
        Some(NodeHandle::new(id, self.doc_weak()))
    }
}

impl Drop for TestApp {
    /// The thread's window requests and timers are no longer this app's.
    fn drop(&mut self) {
        // Every document is disposed as its own code (the shell's teardown)
        // while this app's sink is registered: what their cleanups send (a
        // request for this app's window, the closes of the windows it owns)
        // is this app's, not another live app's, older or newer. Left to the
        // field drop, the main document would be disposed after the sink is
        // gone, and as no document's code.
        self.app.dispose_document();
        // Its idle-trim timer is cleared on the clock it was armed on.
        self.presentation = None;
        // Newest first: a popup closes before the window it is on (#2552).
        for child in std::mem::take(&mut self.children).into_iter().rev() {
            child.close();
        }
        let last = SINKS
            .try_with(|s| {
                let mut sinks = s.borrow_mut();
                sinks.retain(|k| !Rc::ptr_eq(k, &self.sink));
                sinks.is_empty()
            })
            .unwrap_or(false);
        // The outputs are the thread's (`played_outputs`): kept for the
        // apps still live here, forgotten with the last, so the next app on
        // this thread starts with none played. Forgotten here, after the
        // main document was disposed (first, above) and the children closed
        // (each `close` disposes its document): no reader of this app is
        // left to run for it, and no other app lives on the thread, so the
        // reset wakes nobody. An app never sees its outputs vanish because
        // it is going, as in the shell. Pinned by
        // `the_last_apps_readers_do_not_run_as_it_forgets_the_outputs`.
        if last {
            crate::outputs::unpublish();
        }
        virtual_time::uninstall();
    }
}
