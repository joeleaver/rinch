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
///
/// Each event carries the window it is for ([`EventFor`], #2416): the document
/// whose code sent it, or a window a [`WindowHandle`](crate::WindowHandle)
/// names. The runtime resolves it to one of its windows
/// ([`RinchRuntime::role_for`]), which is how `hide_current_window` from the
/// DevTools window's code is DevTools'.
pub(crate) struct NativeEventQueue {
    events: VecDeque<(EventFor, RinchNativeEvent)>,
    rerender_pending: bool,
}

impl NativeEventQueue {
    const fn new() -> Self {
        Self {
            events: VecDeque::new(),
            rerender_pending: false,
        }
    }

    /// Queue `event` for the window `to` names. Returns `false` for a
    /// `ReRender` that coalesced into one already queued — nothing was added
    /// and no wake is owed.
    pub(crate) fn push(&mut self, to: EventFor, event: RinchNativeEvent) -> bool {
        if matches!(event, RinchNativeEvent::ReRender) {
            if self.rerender_pending {
                return false;
            }
            self.rerender_pending = true;
        }
        self.events.push_back((to, event));
        true
    }

    /// Take every queued event with the window it is for, and let the next
    /// `ReRender` queue again.
    pub(crate) fn drain(&mut self) -> Vec<(EventFor, RinchNativeEvent)> {
        self.rerender_pending = false;
        self.events.drain(..).collect()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Which window a queued native event is for (#2416).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EventFor {
    /// The window of the document whose code sent it, `None` from outside any
    /// document (`main`, a worker thread, a menu callback): the main window.
    Sender(Option<u64>),
    /// The window a [`WindowHandle`](crate::WindowHandle) names, by its id.
    /// For nothing once that window has closed.
    Window(u64),
}

// Queue of native events to process on the next proxy_wake_up.
// winit 0.31 removes send_event(T); we queue events and call proxy.wake_up().
pub(super) static NATIVE_EVENT_QUEUE: Mutex<NativeEventQueue> = Mutex::new(NativeEventQueue::new());

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
    let is_rerender = matches!(event, RinchNativeEvent::ReRender);
    if !NATIVE_EVENT_QUEUE.lock().unwrap().push(to, event) {
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
    /// Move the window's outer top-left to this logical position (#1546).
    SetWindowPosition(f64, f64),
    /// Resize the window's inner area to this logical size (#1546).
    SetWindowSize(f64, f64),
    /// Apply the title `rinch::window::set_window_title` stored (#1770).
    ApplyWindowTitle,
    /// Apply the vsync setting `rinch::window::set_vsync` stored (#2095).
    ApplyVsync,
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
    /// Bring the window to the front and give it the keyboard (#2416).
    FocusWindow,
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
mod native_event_queue_tests {
    use super::*;

    fn rerenders(events: &[(EventFor, RinchNativeEvent)]) -> usize {
        events
            .iter()
            .filter(|(_, e)| matches!(e, RinchNativeEvent::ReRender))
            .count()
    }

    /// Any number of `ReRender`s between two drains queue one event, and only
    /// the first asks for a wake.
    #[test]
    fn rerenders_coalesce_until_the_queue_is_drained() {
        let mut q = NativeEventQueue::new();
        let accepted: Vec<bool> = (0..7)
            .map(|_| q.push(EventFor::Sender(None), RinchNativeEvent::ReRender))
            .collect();
        assert_eq!(accepted, [true, false, false, false, false, false, false]);
        assert_eq!(rerenders(&q.drain()), 1);

        // Drained: the next one queues again, and again coalesces its followers.
        assert!(q.push(EventFor::Sender(None), RinchNativeEvent::ReRender));
        assert!(!q.push(EventFor::Sender(None), RinchNativeEvent::ReRender));
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
            .push(EventFor::Sender(None), RinchNativeEvent::ReRender);

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
        assert!(q.push(EventFor::Sender(None), RinchNativeEvent::MinimizeWindow));
        assert!(q.push(EventFor::Sender(None), RinchNativeEvent::ReRender));
        assert!(q.push(EventFor::Sender(None), RinchNativeEvent::MinimizeWindow));
        assert!(!q.push(EventFor::Sender(None), RinchNativeEvent::ReRender));
        assert!(q.push(EventFor::Sender(None), RinchNativeEvent::ShowWindow));
        let drained = q.drain();
        assert_eq!(drained.len(), 4, "{drained:?}");
        assert!(matches!(
            drained[1],
            (EventFor::Sender(None), RinchNativeEvent::ReRender)
        ));
        assert_eq!(rerenders(&drained), 1);
    }

    /// An event carries the document whose code sent it (#2416), so the
    /// runtime can give it to that document's window; from outside any
    /// document it carries none.
    #[test]
    fn an_event_carries_the_document_that_sent_it() {
        let _lock = crate::app::RERENDER_EVENTS_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        send_native_event(RinchNativeEvent::MinimizeWindow);
        {
            let _doc = rinch_core::push_dispatching_doc(42);
            send_native_event(RinchNativeEvent::HideWindow);
        }
        send_native_event_for(EventFor::Window(7), RinchNativeEvent::CloseWindow);
        let drained = NATIVE_EVENT_QUEUE.lock().unwrap().drain();
        assert!(matches!(
            drained[0],
            (EventFor::Sender(None), RinchNativeEvent::MinimizeWindow)
        ));
        assert!(matches!(
            drained[1],
            (EventFor::Sender(Some(42)), RinchNativeEvent::HideWindow)
        ));
        assert!(matches!(
            drained[2],
            (EventFor::Window(7), RinchNativeEvent::CloseWindow)
        ));
    }
}
