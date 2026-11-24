//! The app's windows (#2416): open one with [`open_window`], name one with a
//! [`WindowHandle`], list them with [`windows`](fn@windows), and end the app with
//! [`exit`].
//!
//! Every window runs on the UI thread in one reactive graph, so a `Signal`
//! made in one window can be read and written in any. A store created in a
//! second window's content is that window's; `use_store` there falls back to
//! one created outside any window or in the main window.

use std::cell::RefCell;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::{LayerSurface, WindowProps, WindowSurfaceKind};
use rinch_core::reactive::Signal;

use crate::shell::rinch_runtime::{EventFor, RinchNativeEvent, send_native_event_for};
use crate::shell::window_geometry::{self, WindowOf};
use crate::windows::WindowRestore;

/// The content a window is built from: called once, into the window's own
/// document, as `App::new`'s root is.
pub(crate) type WindowContent = Box<dyn FnOnce(&mut RenderScope) -> NodeHandle>;

/// A window of this app.
///
/// `Copy`, so it fits in a `Signal` or a store and can be passed anywhere on
/// the UI thread. It stays valid after its window closes: every operation is
/// then a no-op, the getters answer `None`, and [`is_open`](Self::is_open)
/// answers `false`. Operations are carried out on the event loop's next turn,
/// in the order they were asked.
///
/// **Who owns a window.** A window opened while an owner is in scope belongs
/// to it, as a store does: opened in a component body, it closes when that
/// component goes; in a `for` row or an `if` / `match` branch, when the row or
/// branch goes; in an event handler, when the scope that built the handler
/// goes (its component, or the row or branch its element is in). Opened from
/// `main`, a menu or tray callback, or under
/// [`unowned`](rinch_core::reactive::unowned), it lives until it is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowHandle(u64);

/// When the event loop ends (#2416), set with
/// [`App::exit_when`](crate::App::exit_when). [`exit`] ends it under any of
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExitWhen {
    /// When the main window closes, whatever else is open: what an
    /// `App::new` app has always done, and its default.
    #[default]
    MainWindowCloses,
    /// When the last window closes, the main one or not.
    LastWindowCloses,
    /// Only when the app calls [`exit`].
    Explicitly,
}

/// One window the registry knows of.
struct Entry {
    id: u64,
    /// The window's document, once it is mounted.
    doc: Option<u64>,
    open: Signal<bool>,
    /// What the window's own reactive state lives in (its `open` signal),
    /// disposed when it closes.
    scope: rinch_core::reactive::Scope,
    /// The "close it" its owner runs when it goes, taken back when the window
    /// closes first, so a long-lived owner keeps nothing of a closed window.
    owner_cleanup: Option<rinch_core::reactive::CleanupHandle>,
    /// Its owner's going is what closes it: `on_closed` is not called.
    owner_gone: bool,
    /// Called once the window has closed, for every cause but its owner
    /// going (the `Window` component's `on_closed`).
    on_closed: Option<Box<dyn FnOnce()>>,
    /// Its title when it was opened, to name it in a warning.
    title: String,
    /// What makes it a popup (#2552), for one `open_popup` opened.
    popup: Option<crate::popup::PopupEntry>,
}

/// A window `open_window` opened that the runtime has not made yet.
pub(crate) struct Pending {
    pub(crate) id: u64,
    pub(crate) props: WindowProps,
    pub(crate) content: WindowContent,
}

#[derive(Default)]
struct Registry {
    next: u64,
    entries: Vec<Entry>,
    main: Option<u64>,
    list: Option<Signal<Vec<WindowHandle>>>,
    pending: Vec<Pending>,
    /// Each window's newest press (a button, a key, a touch), on rinch's
    /// clock: a menu popup's grab names one in its parent chain (#2552).
    /// `0` is a document that is no window of the registry's
    /// (`rinch::testing`'s).
    pressed: Vec<(u64, rinch_core::timer::Instant)>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
    /// The window whose document is being mounted, by id: its document key is
    /// only known to the registry once the mount returns, and its content's
    /// code, from its first line, is that window's.
    static MOUNTING: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

impl Registry {
    fn list(&mut self) -> Signal<Vec<WindowHandle>> {
        *self
            .list
            .get_or_insert_with(|| rinch_core::reactive::unowned(|| Signal::new(Vec::new())))
    }

    /// A new entry, open, and its handle at the end of the list. Signals are
    /// written after the registry's borrow ends (the caller's `publish`).
    fn add(&mut self) -> (u64, Signal<Vec<WindowHandle>>) {
        self.next += 1;
        let id = self.next;
        let scope = rinch_core::reactive::Scope::new();
        let open = scope.run(|| Signal::new(true));
        self.entries.push(Entry {
            id,
            doc: None,
            open,
            scope,
            owner_cleanup: None,
            owner_gone: false,
            on_closed: None,
            title: String::new(),
            popup: None,
        });
        (id, self.list())
    }

    fn open_ids(&self) -> Vec<WindowHandle> {
        let mut ids: Vec<WindowHandle> = self
            .entries
            .iter()
            .filter(|e| peek(e.open))
            .map(|e| WindowHandle(e.id))
            .collect();
        // The main window first, then oldest first.
        if let Some(main) = self.main {
            ids.sort_by_key(|h| (h.0 != main, h.0));
        }
        ids
    }
}

/// A signal's value, read without subscribing whatever is running.
fn peek(signal: Signal<bool>) -> bool {
    rinch_core::reactive::untracked(|| signal.get())
}

fn with<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    REGISTRY.with(|r| f(&mut r.borrow_mut()))
}

/// Republish the open windows' list.
fn publish_list() {
    let (list, ids) = with(|r| (r.list(), r.open_ids()));
    list.set(ids);
}

/// Open a window whose content is `content`, built once into the window's own
/// document, as `App::new`'s root is. Main thread only. In `rsx!` the same is
/// the `Window { … }` element, whose children are the content and whose
/// `on_closed` is called once it has closed for any cause but the element
/// going, a `handle.close()` from the app's own code included. The handle is usable
/// at once; the window appears on the event loop's next turn.
///
/// `props` mean what they mean for the main window: title, size, position,
/// `wayland_layer`, `output`, `on_close_requested` and the rest.
/// `menu_in_titlebar` is the main window's only (the native menu bar belongs
/// to it); the menu's shortcuts work in every window.
///
/// The window belongs to the owner in scope (see [`WindowHandle`]'s *Who
/// owns a window*): it closes when that owner goes. Under
/// [`unowned`](rinch_core::reactive::unowned) it lives until it is closed.
pub fn open_window<F>(props: WindowProps, content: F) -> WindowHandle
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    open(props, Box::new(content), None, None)
}

/// [`open_window`], with what is called once the window has closed for any
/// cause but its owner going.
pub(crate) fn open(
    props: WindowProps,
    content: WindowContent,
    on_closed: Option<Box<dyn FnOnce()>>,
    popup: Option<crate::popup::PopupEntry>,
) -> WindowHandle {
    let title = props.title.clone();
    let (id, _) = with(|r| r.add());
    with(|r| {
        if let Some(e) = r.entries.iter_mut().find(|e| e.id == id) {
            e.on_closed = on_closed;
            e.title = title;
            let scope = &e.scope;
            e.popup = popup.map(|p| p.in_scope(scope));
        }
    });
    with(|r| r.pending.push(Pending { id, props, content }));
    publish_list();
    let handle = WindowHandle(id);
    // Its owner's disposal closes it; with no owner it lives until closed.
    let cleanup = rinch_core::reactive::on_cleanup_removable(move || {
        with(|r| {
            if let Some(e) = r.entries.iter_mut().find(|e| e.id == id) {
                e.owner_gone = true;
            }
        });
        handle.close();
    });
    with(|r| {
        if let Some(e) = r.entries.iter_mut().find(|e| e.id == id) {
            e.owner_cleanup = cleanup;
        }
    });
    send_native_event_for(EventFor::Window(id), RinchNativeEvent::OpenWindow(id));
    handle
}

/// The app's open windows, the main window first, then oldest first.
/// Reactive: an effect that reads it runs again when a window opens or
/// closes.
pub fn windows() -> Vec<WindowHandle> {
    with(|r| r.list()).get()
}

/// End the event loop after this turn, whatever [`ExitWhen`] says. Every
/// window closes with it.
///
/// Callable from any thread. Asked for before the app runs (from a thread
/// that watches for `SIGTERM`, started before [`App::run`](crate::App::run)),
/// it is kept, and the run that comes returns at once, `Ok(())` from
/// [`App::try_run`](crate::App::try_run), without opening a window (#3015).
///
/// It takes a lock, so it is not async-signal-safe: never call it from inside
/// a signal handler itself. Have the handler wake a thread that calls it
/// (signal-hook's `iterator::Signals`, or its `flag` checked by a thread).
pub fn exit() {
    send_native_event_for(EventFor::Sender(None), RinchNativeEvent::Exit);
}

impl WindowHandle {
    /// The window whose document's code is running (an event handler, an
    /// effect, a timer, a closure queued with `run_on_main_thread`, and its
    /// content as it is built, from the first line); outside any window's
    /// document, the main window. `None` when there is none.
    pub fn current() -> Option<WindowHandle> {
        let doc = rinch_core::current_dispatching_doc();
        let mounting = MOUNTING.with(|m| m.get());
        with(|r| {
            doc.and_then(|d| r.entries.iter().find(|e| e.doc == Some(d)))
                .map(|e| e.id)
                .or(mounting)
                .or(r.main)
                .map(WindowHandle)
        })
        .filter(|h| h.is_open_untracked())
    }

    /// A number naming this window for the whole run, never reused: what its
    /// `RINCH_PERF` lines carry (`[PERF] window 3 frame 1:`, [`crate::perf`])
    /// and the `perf_stats` debug command reports.
    pub fn id(self) -> u64 {
        self.0
    }

    /// The handle of the window `id` names.
    pub(crate) fn from_id(id: u64) -> WindowHandle {
        WindowHandle(id)
    }

    /// The main window, while it is open.
    pub fn main() -> Option<WindowHandle> {
        with(|r| r.main.map(WindowHandle)).filter(|h| h.is_open_untracked())
    }

    /// Whether the window is open. Reactive: an effect that reads it runs
    /// again when the window closes.
    pub fn is_open(self) -> bool {
        with(|r| r.entries.iter().find(|e| e.id == self.0).map(|e| e.open)).is_some_and(|s| s.get())
    }

    pub(crate) fn is_open_untracked(self) -> bool {
        with(|r| {
            r.entries
                .iter()
                .find(|e| e.id == self.0)
                .is_some_and(|e| peek(e.open))
        })
    }

    /// Close the window. Its `on_close_requested` is not asked: the app is the
    /// one deciding. Its document is disposed (its effects, handlers, stores
    /// and the windows it owns), then [`is_open`](Self::is_open) turns false
    /// and [`ExitWhen`] is consulted.
    pub fn close(self) {
        // Not made yet (its owner went in the turn it was opened, or the app
        // closed it at once): it never is.
        if take_pending(self.0).is_some() {
            note_closed(self.0);
            return;
        }
        self.send(RinchNativeEvent::CloseWindow);
    }

    /// Show the window, made again where it was hidden.
    pub fn show(self) {
        self.send(RinchNativeEvent::ShowWindow);
    }
    /// Hide the window. Its document lives on.
    pub fn hide(self) {
        self.send(RinchNativeEvent::HideWindow);
    }
    /// Prepare the window, while it is hidden, for its next show, without
    /// showing it: [`prepare_current_window`](crate::windows::prepare_current_window)
    /// for this window (#2633).
    pub fn prepare(self) {
        self.prepare_with(None);
    }
    /// [`prepare`](Self::prepare) at the display scale `scale`, for an app
    /// that knows the scale the window will be shown at better than rinch's
    /// guess (see [`prepare_current_window`](crate::windows::prepare_current_window)).
    pub fn prepare_at_scale(self, scale: f64) {
        self.prepare_with(Some(scale));
    }
    /// [`prepare`](Self::prepare) at `scale`, or the likely scale.
    pub(crate) fn prepare_with(self, scale: Option<f64>) {
        self.send(RinchNativeEvent::PrepareWindow(scale));
    }
    /// Minimize the window.
    pub fn minimize(self) {
        self.send(RinchNativeEvent::MinimizeWindow);
    }
    /// Maximize the window, or restore it when it is maximized.
    pub fn toggle_maximize(self) {
        self.send(RinchNativeEvent::ToggleMaximizeWindow);
    }
    /// Bring the window to the front and give it the keyboard, where the
    /// platform lets an app do so: shown if it was hidden, restored if it was
    /// minimized, and only restored if it is
    /// [`non_activating`](rinch_core::element::WindowProps::non_activating).
    /// What [`focus_current_window`](crate::windows::focus_current_window)
    /// does for the current window, and with the same limits: on Wayland a
    /// shown window is only marked as wanting attention (with a token,
    /// [`activate_with_token`](Self::activate_with_token) raises it).
    pub fn focus(self) {
        self.send(RinchNativeEvent::FocusWindow(None));
    }
    /// [`focus`](Self::focus) with an xdg-activation `token`, which lets a
    /// Wayland compositor raise the window (#2817): what
    /// [`activate_current_window_with_token`](crate::windows::activate_current_window_with_token)
    /// does for the current window, with the same limits. Elsewhere it is
    /// [`focus`](Self::focus).
    pub fn activate_with_token(self, token: &str) {
        self.send(RinchNativeEvent::FocusWindow(Some(token.to_string())));
    }
    /// Set the window's title.
    pub fn set_title(self, title: &str) {
        self.send(RinchNativeEvent::SetWindowTitle(title.to_string()));
    }
    /// Move the window, as
    /// [`set_current_window_position`](crate::windows::set_current_window_position)
    /// does for the current one.
    pub fn set_position(self, x: f64, y: f64) {
        self.send(RinchNativeEvent::SetWindowPosition(x, y));
    }
    /// Resize the window's inner area, as
    /// [`set_current_window_size`](crate::windows::set_current_window_size)
    /// does for the current one.
    pub fn set_size(self, width: f64, height: f64) {
        self.send(RinchNativeEvent::SetWindowSize(width, height));
    }
    /// Show a layer-shell window on this output, as
    /// [`set_current_window_output`](crate::windows::set_current_window_output)
    /// does for the current one.
    pub fn set_output(self, output: Option<&str>) {
        self.send(RinchNativeEvent::SetWindowOutput(
            output.map(str::to_string),
        ));
    }
    /// Replace the window's layer-shell configuration, as
    /// [`set_current_window_layer`](crate::windows::set_current_window_layer)
    /// does for the current one.
    pub fn set_layer(self, surface: LayerSurface) {
        self.send(RinchNativeEvent::SetWindowLayer(Box::new(surface)));
    }
    /// Let pointer events through the window, or stop.
    pub fn set_click_through(self, on: bool) {
        self.send(RinchNativeEvent::SetClickThrough(on));
    }
    /// Change where the window takes pointer input, as
    /// [`set_input_region_current_window`](crate::windows::set_input_region_current_window)
    /// does for the current one.
    pub fn set_input_region(self, region: rinch_core::element::InputRegion) {
        self.send(RinchNativeEvent::SetInputRegion(region));
    }

    /// The window's outer position, as
    /// [`current_window_position`](crate::windows::current_window_position)
    /// answers for the current one.
    pub fn position(self) -> Option<(f64, f64)> {
        window_geometry::published_of(self.facts()?).and_then(|g| g.position)
    }
    /// The window's inner size, as
    /// [`current_window_size`](crate::windows::current_window_size) answers
    /// for the current one.
    pub fn size(self) -> Option<(f64, f64)> {
        window_geometry::published_of(self.facts()?).and_then(|g| g.size)
    }
    /// What to restore the window to, as
    /// [`current_window_restore`](crate::windows::current_window_restore)
    /// answers for the current one.
    pub fn restore(self) -> Option<WindowRestore> {
        crate::windows::restore_of(window_geometry::published_of(self.facts()?)?)
    }
    /// What kind of surface the window got, as
    /// [`current_window_surface_kind`](crate::windows::current_window_surface_kind)
    /// answers for the current one.
    pub fn surface_kind(self) -> Option<WindowSurfaceKind> {
        crate::shell::layer_shell::published_surface_kind_of(self.facts()?)
    }

    /// Which window's published facts are this one's: `None` for the main
    /// window's, its document for another's. `None` (outer) when it is closed
    /// or not made yet.
    fn facts(self) -> Option<WindowOf> {
        with(|r| {
            if r.main == Some(self.0) {
                return Some(None);
            }
            r.entries
                .iter()
                .find(|e| e.id == self.0 && peek(e.open))
                .and_then(|e| e.doc.map(Some))
        })
    }

    fn send(self, event: RinchNativeEvent) {
        send_native_event_for(EventFor::Window(self.0), event);
    }
}

/// What `rsx!`'s `Window { … }` element becomes (#2416): its window, opened
/// with `props` and owned by the scope building it, so it closes when the
/// element goes; `content` is the element's children, built in the window's
/// own document. `on_open` is called with its handle at once; `on_closed`
/// once it has closed for any cause but the element going. Answers the
/// element's place in the document it sits in, which shows nothing.
#[doc(hidden)]
pub fn window_component<O, C, F>(
    scope: &mut RenderScope,
    props: WindowProps,
    on_open: O,
    on_closed: C,
    content: F,
) -> NodeHandle
where
    O: FnOnce(WindowHandle),
    C: FnOnce() + 'static,
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    let handle = open(props, Box::new(content), Some(Box::new(on_closed)), None);
    rinch_core::reactive::untracked_handler(|| on_open(handle));
    scope.create_comment("window")
}

mod popups;
mod runtime;

pub(crate) use popups::{
    can_parent, chain_pressed, dismiss, note_dismissal, note_press, popups, window_of_doc,
    with_popup,
};
pub(crate) use runtime::{
    describe, exit_when, is_pending, mounting, note_closed, note_mounted, register_main, reset,
    set_exit_when, set_windowless, take_pending, windowless,
};

#[cfg(test)]
#[path = "window_handle_tests.rs"]
mod tests;
