//! Waking the event loop: the native event queue, the main-thread callback queue's wake, and the proxies.

use super::*;

// ── Thread-local proxy ───────────────────────────────────────────────────────

// Thread-local proxy for the native event loop, used by window control functions.
thread_local! {
    pub(crate) static NATIVE_PROXY: RefCell<Option<EventLoopProxy>> = const { RefCell::new(None) };
}

// ── Global proxy + native event queue ────────────────────────────────────────

// Global (Send+Sync) proxy for waking the event loop from any thread.
pub(crate) static GLOBAL_PROXY: OnceLock<EventLoopProxy> = OnceLock::new();

mod queue;

use queue::Drainer;
pub(crate) use queue::{EventFor, NativeEventQueue};

/// Queue of native events to process on the next proxy_wake_up.
/// winit 0.31 removes send_event(T); we queue events and call proxy.wake_up().
/// The type ([`NativeEventQueue`]) says what a send does with its event.
///
/// The crate's own unit tests drive the queue with no runtime, as though one
/// had been made, so it starts at [`Drainer::Runtime`] there.
pub(super) static NATIVE_EVENT_QUEUE: Mutex<NativeEventQueue> =
    Mutex::new(NativeEventQueue::with_drainer(if cfg!(test) {
        Drainer::Runtime
    } else {
        Drainer::Coming
    }));

/// Note that a desktop runtime exists, so native events are queued for it
/// (#2801). Called by `RinchRuntime::with_app`, before anything of its app.
pub(super) fn note_runtime_made() {
    NATIVE_EVENT_QUEUE.lock().unwrap().note_runtime();
}

/// Note that a [`RinchContext`](crate::embed::RinchContext) was made: with
/// no desktop runtime made, the process is an embed host (#2801).
#[cfg(any(feature = "gpu-renderer", feature = "embed"))]
pub(crate) fn note_embed_host() {
    NATIVE_EVENT_QUEUE.lock().unwrap().note_embed_host();
}

/// Whether [`exit`](crate::exit) was asked for before the run that is
/// starting (#3015): the run then returns `Ok(())` without opening a
/// window. Called at the top of [`App::try_run`](crate::App::try_run);
/// [`run_unless_exit_asked`] looks again just before the event loop runs,
/// which covers an exit sent from another thread while the display was being
/// connected to.
pub(crate) fn take_exit_before_run() -> bool {
    NATIVE_EVENT_QUEUE.lock().unwrap().take_exit()
}

/// Run the event loop (`run`, given the runtime it runs) unless an exit was
/// asked for since [`App::try_run`](crate::App::try_run) looked — sent from
/// another thread while the display was being connected to and the app
/// mounted, #3015 — in which case the runtime is dropped and the run ends
/// with `Ok(())` before any window is made. Either way, once the run has
/// ended the queue is as it was before any run
/// ([`NativeEventQueue::note_run_ended`]): what the ended runtime left in it
/// is dropped, and what is sent after it is kept for the next run.
pub(super) fn run_unless_exit_asked<R>(
    runtime: R,
    run: impl FnOnce(R) -> Result<(), crate::RunError>,
) -> Result<(), crate::RunError> {
    run_unless_exit_asked_in(&NATIVE_EVENT_QUEUE, runtime, run)
}

/// [`run_unless_exit_asked`] on `queue`, which its unit tests make their own.
fn run_unless_exit_asked_in<R>(
    queue: &Mutex<NativeEventQueue>,
    runtime: R,
    run: impl FnOnce(R) -> Result<(), crate::RunError>,
) -> Result<(), crate::RunError> {
    /// Notes the run's end however it ends, a panic out of the loop
    /// included, after the runtime is dropped (its shutdown's own requests
    /// are the ended run's too).
    struct RunEnds<'a>(&'a Mutex<NativeEventQueue>);
    impl Drop for RunEnds<'_> {
        fn drop(&mut self) {
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .note_run_ended();
        }
    }
    let _ends = RunEnds(queue);
    if queue.lock().unwrap().take_exit() {
        drop(runtime);
        return Ok(());
    }
    run(runtime)
}

/// How many native events are queued for the desktop runtime, for a test
/// that pins what a process without one queues (#2801, #3015). Not part of
/// the API.
#[doc(hidden)]
pub fn __native_events_queued() -> usize {
    NATIVE_EVENT_QUEUE.lock().unwrap().events.len()
}

/// Queue a native event and wake the event loop. A `ReRender` while one is
/// already queued is dropped — see [`NativeEventQueue`].
pub(crate) fn send_native_event(event: RinchNativeEvent) {
    send_native_event_for(
        EventFor::Sender(rinch_core::current_dispatching_doc()),
        event,
    );
}

/// [`send_native_event`] for the window `to` names.
pub(crate) fn send_native_event_for(to: EventFor, event: RinchNativeEvent) {
    // A `TestApp` on this thread takes its window's requests (#2742).
    #[cfg(all(feature = "testing", feature = "windowing"))]
    let Some(event) = crate::testing::capture_native_event(to, event) else {
        return;
    };
    // An embedded `RinchContext`'s code: its host takes it (#2801).
    #[cfg(any(feature = "gpu-renderer", feature = "embed"))]
    let Some(event) = crate::embed::window_requests::capture(to, event) else {
        return;
    };
    // Kept for the runtime to come, queued to the one made, or dropped where
    // none will drain it (an embed host, #2801): `NativeEventQueue::push`
    // decides, under the lock the runtime's construction takes (#3015).
    let is_rerender = matches!(event, RinchNativeEvent::ReRender);
    if !NATIVE_EVENT_QUEUE.lock().unwrap().push(to, event) {
        return;
    }
    if is_rerender {
        // The sender's document's, when this thread counts it (#2798).
        crate::app::count_doc_rerender(match to {
            EventFor::Sender(doc) => doc,
            EventFor::Window(_) => None,
        });
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
pub(super) fn drain_wake_queues<S>(
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
/// re-render pass. This is the safe way to update `Signal`s from a
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
pub(super) fn dispatch_to_main_thread(f: Box<dyn FnOnce() + Send>) {
    rinch_core::queue_main_callback(f);
    send_native_event(RinchNativeEvent::ReRender);
}

/// Set `GLOBAL_PROXY` and pay the wake owed to anything queued before it
/// existed (issue #1540). Both desktop entry points call this.
pub(super) fn install_global_proxy(proxy: &EventLoopProxy) {
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
    /// Prepare the hidden window's next frame without showing it (#2633),
    /// at the app's scale or else the likely one.
    PrepareWindow(Option<f64>),
    /// Let pointer events through the window, or stop (issue #1519).
    SetClickThrough(bool),
    /// `set_input_region_current_window` / `WindowHandle::set_input_region` (#2741).
    SetInputRegion(rinch_core::element::InputRegion),
    /// Move the window's outer top-left to this logical position (#1546).
    SetWindowPosition(f64, f64),
    /// Resize the window's inner area to this logical size (#1546).
    SetWindowSize(f64, f64),
    /// Apply the title `rinch::window::set_window_title` stored (#1770).
    ApplyWindowTitle,
    /// Apply the vsync setting `rinch::window::set_vsync` stored (#2095).
    ApplyVsync,
    /// Apply the zoom `rinch::window::set_zoom` set to every open window.
    ApplyZoom,
    /// Register the faces `rinch::font::register_font` queued with every
    /// open window, and keep them for windows opened later.
    RegisterFonts,
    /// Show a Wayland layer-shell window on this output (#1523).
    SetWindowOutput(Option<String>),
    /// Start moving the window with the pointer (#1523).
    StartWindowDrag,
    /// Replace a Wayland layer surface's configuration (#2413).
    SetWindowLayer(Box<rinch_core::element::LayerSurface>),
    /// Look again for an output to show a layer surface on, after the
    /// compositor closed it with no output left (#1523).
    RetryLayerOutput,
    /// Start resizing the window from an edge with the pointer (#1523).
    StartWindowResize(rinch_platform::ResizeDirection),
    /// Ask the compositor for an xdg-activation token for the sending
    /// document's window, answering the parked callback (#2501).
    RequestActivationToken(rinch_core::MainCallbackId),
    /// An injected platform event (e.g. synthetic gamepad input).
    InjectedPlatformEvent(PlatformEvent),
    /// Make the OS window of the window `open_window` opened, by its handle's
    /// id (#2416); its document waits in `window_handle`'s pending list.
    OpenWindow(u64),
    /// Close the window, with no veto asked (`WindowHandle::close`, #2416).
    CloseWindow,
    /// Bring the window to the front and give it the keyboard (#2416), with
    /// the xdg-activation token that lets a Wayland compositor do so, when
    /// the app has one (#2817).
    FocusWindow(Option<String>),
    /// Set the window's title (`WindowHandle::set_title`, #2416).
    SetWindowTitle(String),
    /// End the event loop after this turn (`rinch::exit`, #2416).
    Exit,
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

#[cfg(test)]
mod tests;
