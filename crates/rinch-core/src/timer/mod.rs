//! Cross-platform timers: one-shot ([`set_timeout`]) and repeating ([`set_interval`]).
//!
//! rinch-core is platform-agnostic, so *scheduling* is pluggable while the
//! callback lives in the shared main-thread registry ([`crate::main_thread`]):
//!
//! - **Native** works out of the box: a **single** shared timer thread sleeps
//!   until the earliest pending deadline (an ordered set of timers), then hops each
//!   fired id back to the main thread via the runtime's cross-thread dispatcher
//!   (see [`set_cross_thread_dispatcher`](crate::set_cross_thread_dispatcher)).
//!   One daemon thread serves every timer — arming N timers does not spawn N
//!   threads, so a debounce that re-arms on every keystroke stays cheap.
//!   [`clear_timeout`] takes the timer out of the heap, so a cleared timer
//!   wakes neither that thread nor the main thread at its deadline (#2832).
//! - **Web** has no threads, so the web runtime installs a backend
//!   ([`set_timer_backend_with_cancel`]) that drives `window.setTimeout` and,
//!   on a clear, `window.clearTimeout`:
//!   `rinch::install_web_runtime`, called by every `rinch_web` mount and by
//!   `rinch::embed::RinchContext::new` on wasm32.
//!
//! The callback is parked via [`park_main_callback`],
//! so only its id crosses the thread boundary and it need not be `Send` — it can
//! capture `Rc`-based signals / editor handles.
//!
//! ```ignore
//! let t = rinch_core::set_timeout(300, move || save(editor.doc()));
//! rinch_core::clear_timeout(t); // debounce: cancel and re-arm
//! ```
//!
//! A timeout armed during a render is also cancelled if that component unmounts
//! before it fires — see [`set_timeout`].

use std::sync::Mutex;

use crate::main_thread::{
    MainCallbackId, cancel_main_callback, park_main_callback, resume_main_callback,
};

/// Schedules [`fire_timeout`] for `id` to run **on the main thread** after
/// `delay_ms`. Installed by the platform shell via [`set_timer_backend`].
type TimerBackend = fn(id: u64, delay_ms: u32);

/// Withdraws timer `id` scheduled by a [`TimerBackend`], so it does not wake
/// anything at its deadline: called by [`clear_timeout`].
type TimerCancel = fn(id: u64);

static BACKEND: Mutex<Option<(TimerBackend, Option<TimerCancel>)>> = Mutex::new(None);

/// A scheduled timeout. Pass to [`clear_timeout`] to cancel it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TimeoutHandle(MainCallbackId);

/// Install the platform's timer scheduler. Called by the runtime at startup;
/// on wasm32 the web runtime (`rinch::install_web_runtime`, called by every
/// `rinch_web` mount and by `RinchContext::new`) installs a `window.setTimeout`
/// backend. On native a built-in shared-thread scheduler is used when no
/// backend is installed.
///
/// A backend installed this way cannot withdraw a timer: a cleared timeout
/// still reaches it at its deadline, and [`fire_timeout`] then finds nothing
/// to run. Install with [`set_timer_backend_with_cancel`] to withdraw it.
pub fn set_timer_backend(backend: TimerBackend) {
    *BACKEND.lock().unwrap() = Some((backend, None));
}

/// [`set_timer_backend`], plus `cancel`, which [`clear_timeout`] calls with
/// the id it scheduled so the backend withdraws the wake (the web backend
/// calls `window.clearTimeout`). `cancel` may get an id that has already
/// fired, or one it never saw; it ignores both.
pub fn set_timer_backend_with_cancel(backend: TimerBackend, cancel: TimerCancel) {
    *BACKEND.lock().unwrap() = Some((backend, Some(cancel)));
}

/// Run `cb` on the main (UI) thread after `delay_ms` milliseconds.
///
/// Call from the main thread: the callback is parked in the main-thread registry
/// and invoked there, so it needn't be `Send` and may capture `Rc`-based UI state.
/// The native scheduler additionally requires the runtime's cross-thread
/// dispatcher to be registered (it hops the fired id back onto the main thread).
///
/// # Unmounting cancels it
///
/// A timeout armed **during a render** does not fire if the component that armed
/// it is unmounted first — its captured signals are freed with that component
/// (issue #141), so firing would read dead state. This is what you want for the
/// usual cases (debounce, toast auto-dismiss, retry): there is no UI left to
/// update.
///
/// For work that must outlive the component — flushing to disk, releasing an
/// external resource — arm it outside any render, or opt out explicitly:
///
/// ```ignore
/// rinch_core::reactive::unowned(|| set_timeout(500, move || flush_to_disk()));
/// ```
pub fn set_timeout(delay_ms: u32, cb: impl FnOnce() + 'static) -> TimeoutHandle {
    crate::task::refuse_in_worker("set_timeout");
    let id = park_main_callback::<()>(move |()| cb());
    schedule(id.raw(), delay_ms);
    TimeoutHandle(id)
}

/// Whether a [`set_timeout`] armed now on this thread will ever run: `true`
/// on native (the built-in scheduler stands in for a missing backend), under
/// a test's virtual clock, and on the web once a backend is installed
/// ([`set_timer_backend`]). `false` only on wasm32 with no backend, where a
/// timeout is parked and never fires.
///
/// For code whose correctness rests on a deferred check: where this is
/// `false`, it decides at once rather than wait for a check that never comes.
pub fn timers_fire() -> bool {
    virtual_time::is_installed()
        || cfg!(not(target_arch = "wasm32"))
        || BACKEND.lock().unwrap().is_some()
}

/// Schedules a [`yield_now`] wake: run timer `id` (through
/// [`fire_timeout`]) in a new task as soon as the platform can, without a
/// timer's delay.
type YieldBackend = fn(id: u64);

static YIELD_BACKEND: Mutex<Option<YieldBackend>> = Mutex::new(None);

/// Install the platform's [`yield_now`] scheduler. The web runtime installs
/// one that posts to a `MessageChannel`: a nested `setTimeout(0)` is clamped
/// to 4 ms by browsers, a message is not. Without one, [`yield_now`] is a
/// [`set_timeout`] of 0 ms.
pub fn set_yield_backend(backend: YieldBackend) {
    *YIELD_BACKEND.lock().unwrap() = Some(backend);
}

/// Run `cb` on the main thread in a task of its own, as soon as the page (or
/// event loop) has had a turn: what long work cut into slices waits on
/// between slices, so input and painting run in between. Cancel it with
/// [`clear_timeout`], like a timeout; it is owned by the running component
/// the same way.
///
/// On the web this is a `MessageChannel` post, not a `setTimeout(0)`,
/// which browsers clamp to 4 ms once nested; under a test's virtual clock it
/// is a 0 ms timeout on that clock.
pub fn yield_now(cb: impl FnOnce() + 'static) -> TimeoutHandle {
    crate::task::refuse_in_worker("yield_now");
    let id = park_main_callback::<()>(move |()| cb());
    let backend = *YIELD_BACKEND.lock().unwrap();
    match backend {
        Some(post) if !virtual_time::schedule(id.raw(), 0) => post(id.raw()),
        Some(_) => {}
        None => schedule(id.raw(), 0),
    }
    TimeoutHandle(id)
}

/// Cancel a scheduled timeout. Idempotent, and safe to call after it has fired.
///
/// This drops the parked callback and withdraws the timer from its scheduler,
/// so nothing wakes at its deadline: on native the timer thread re-arms for the
/// next deadline (or sleeps until a timer is set, when this was the last) and
/// posts nothing to the main thread; on the web it is a `window.clearTimeout`
/// (#2832). A backend installed with plain [`set_timer_backend`] has no way to
/// withdraw it, so there the wake still comes and finds nothing to run.
///
/// A timeout that is *not* cleared but whose component unmounted is dropped
/// unrun when its deadline comes; that wake still happens.
pub fn clear_timeout(handle: TimeoutHandle) {
    cancel_main_callback(handle.0);
    unschedule(handle.0.raw());
}

/// A repeating timer started by [`set_interval`]. Pass to [`clear_interval`]
/// to stop it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct IntervalHandle(u64);

thread_local! {
    /// Each live interval's pending tick. An interval is a chain of one-shot
    /// timeouts, each re-arming the next; this map is what keeps the handle
    /// stable across the chain. An entry leaves on [`clear_interval`] or when
    /// the component that armed it unmounts.
    static INTERVALS: std::cell::RefCell<std::collections::HashMap<u64, TimeoutHandle>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

static NEXT_INTERVAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Run `cb` on the main (UI) thread every `period_ms` milliseconds, until
/// [`clear_interval`] stops it.
///
/// The first call comes one period after this one. The rate is fixed, as a
/// browser's `setInterval` is: each tick is scheduled from the previous tick's
/// deadline, not from when its callback finished, so a slow callback does not
/// make the ticks drift. A tick that comes more than one period late (a busy
/// main thread, a suspended laptop, a backgrounded tab) skips the ticks it
/// missed rather than firing them in a burst. A `period_ms` of 0 is treated as
/// 1.
///
/// Like [`set_timeout`], the callback runs on the main thread, need not be
/// `Send`, and runs as one batch. An interval armed **during a render** stops
/// when that component unmounts; one armed outside any render runs until
/// cleared. It may clear itself from inside its own callback.
///
/// ```ignore
/// let elapsed = Signal::new(0u32);
/// let tick = set_interval(1000, move || elapsed.update(|s| *s += 1));
/// // later:
/// clear_interval(tick);
/// ```
pub fn set_interval(period_ms: u32, cb: impl FnMut() + 'static) -> IntervalHandle {
    crate::task::refuse_in_worker("set_interval");
    use web_time::Duration;

    let period = Duration::from_millis(u64::from(period_ms.max(1)));
    let id = NEXT_INTERVAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let cb = std::rc::Rc::new(std::cell::RefCell::new(cb));

    /// Arm the tick due at `deadline`.
    fn arm(
        id: u64,
        deadline: Instant,
        period: Duration,
        cb: std::rc::Rc<std::cell::RefCell<dyn FnMut()>>,
    ) -> TimeoutHandle {
        let delay = deadline.saturating_duration_since(now());
        // Rounded up, from nanoseconds: rounded down, the tick fired up to a
        // millisecond before its deadline (from microseconds, up to 1 µs).
        let delay_ms = u32::try_from(delay.as_nanos().div_ceil(1_000_000)).unwrap_or(u32::MAX);
        set_timeout(delay_ms, move || {
            // Stopped since this tick was armed: nothing to run or re-arm.
            if !INTERVALS.with(|m| m.borrow().contains_key(&id)) {
                return;
            }
            // The callback is called with no registry borrow held, so it may
            // clear this interval or arm others.
            (cb.borrow_mut())();
            let now = now();
            let mut next = deadline + period;
            if next <= now {
                // The next tick is already due: more than a period behind.
                // Skip the missed ticks rather than fire them back to back.
                next = now + period;
            }
            // Re-armed only if the callback did not clear it.
            if INTERVALS.with(|m| m.borrow().contains_key(&id)) {
                let pending = arm(id, next, period, cb);
                INTERVALS.with(|m| {
                    if let Some(slot) = m.borrow_mut().get_mut(&id) {
                        *slot = pending;
                    }
                });
            }
        })
    }

    let first = arm(id, now() + period, period, cb);
    INTERVALS.with(|m| m.borrow_mut().insert(id, first));
    // Unmounting the component that armed it stops it. The ticks' own owner
    // check would end the chain anyway; this also drops the map entry.
    crate::reactive::on_cleanup(move || clear_interval(IntervalHandle(id)));
    IntervalHandle(id)
}

/// Stop an interval started by [`set_interval`]. Idempotent; safe to call
/// from inside the interval's own callback.
pub fn clear_interval(handle: IntervalHandle) {
    let pending = INTERVALS.with(|m| m.borrow_mut().remove(&handle.0));
    if let Some(pending) = pending {
        clear_timeout(pending);
    }
}

/// Run the callback parked for `id`, if it hasn't been cancelled.
///
/// Called by the timer backend **on the main thread** when the delay elapses.
/// This is a backend entry point (needed public so `rinch-web` can call it), not
/// app API — prefer [`set_timeout`] / [`clear_timeout`].
#[doc(hidden)]
pub fn fire_timeout(id: u64) {
    // One transaction per callback, like every event handler
    // (`crate::reactive::batch`). The desktop path is already inside one (the
    // main-thread drain batches each callback); a browser `setTimeout` calls
    // this directly.
    crate::reactive::batch(|| resume_main_callback(MainCallbackId::from_raw(id), ()));
}

/// The instant type [`now`] answers: `std::time::Instant` natively, and
/// `web_time::Instant` (backed by `performance.now()`) on wasm32, where
/// `std::time::Instant::now` panics.
pub use web_time::Instant;

/// The time on the clock rinch's timers run on (#2839).
///
/// Use it for any elapsed-time judgement that has to agree with
/// [`set_timeout`] and [`set_interval`]: a grace period, a debounce window,
/// "has it been 300 ms since this was heard of". A timeout armed for `d`
/// from here fires when `now()` has moved on by `d`, on the same clock.
///
/// Normally it is the wall clock, [`Instant::now`]. Under the test harness
/// (`rinch::testing::TestApp`) it is the harness's virtual clock: the instant
/// that clock was installed at plus the virtual time since, which only
/// `TestApp::advance_time` moves, so a test can stand exactly on a boundary
/// (299 ms: not yet; 300 ms: over) and get the same answer every run.
/// `std::time::Instant::now()` keeps reading the wall clock there, and does
/// not agree with the timers.
///
/// The virtual clock is the thread's: on another thread, and on a thread
/// with no harness, this is the wall clock.
pub fn now() -> Instant {
    virtual_time::now().unwrap_or_else(Instant::now)
}

fn schedule(id: u64, delay_ms: u32) {
    if virtual_time::schedule(id, delay_ms) {
        return;
    }
    #[cfg(all(test, not(target_arch = "wasm32")))]
    if let Some(sched) = native::test_scheduler() {
        sched.schedule(id, delay_ms);
        return;
    }
    if let Some((backend, _)) = *BACKEND.lock().unwrap() {
        backend(id, delay_ms);
        return;
    }

    #[cfg(not(target_arch = "wasm32"))]
    native::schedule(id, delay_ms);

    #[cfg(target_arch = "wasm32")]
    {
        let _ = (id, delay_ms);
        tracing::warn!(
            "set_timeout: no timer backend installed — the callback will never fire. \
             Call `rinch::install_web_runtime()` (or `rinch_web::install_runtime()`) \
             at startup; a `rinch_web` mount or a `RinchContext` installs it too."
        );
    }
}

/// Withdraw timer `id` from whichever scheduler [`schedule`] gave it to.
/// Harmless for an id that has fired or was never scheduled.
fn unschedule(id: u64) {
    if virtual_time::cancel(id) {
        return;
    }
    #[cfg(all(test, not(target_arch = "wasm32")))]
    if let Some(sched) = native::test_scheduler() {
        sched.cancel(id);
        return;
    }
    let backend = *BACKEND.lock().unwrap();
    match backend {
        Some((_, Some(cancel))) => cancel(id),
        Some((_, None)) => {}
        #[cfg(not(target_arch = "wasm32"))]
        None => native::cancel(id),
        #[cfg(target_arch = "wasm32")]
        None => {}
    }
}

/// A virtual clock for a test harness (`rinch::testing::TestApp`, #2742): while
/// one is installed on a thread, a timer armed on that thread waits on it
/// instead of the wall clock, and fires only when the harness advances it.
///
/// Thread-local, so tests running in parallel on other threads keep their own
/// clocks (or the wall clock). Installs nest: the clock goes when the last
/// [`install`](virtual_time::install) is matched by an [`uninstall`](virtual_time::uninstall), cancelling the timers still
/// waiting on it. Not app API.
#[doc(hidden)]
pub mod virtual_time;

/// Built-in native scheduler: a single shared daemon thread driving an ordered
/// set of deadlines. Sleeps (`Condvar::wait_timeout`) until the nearest
/// deadline, or indefinitely when there is none, and hops each fired id to the
/// main thread. It is woken only when the nearest deadline changes: a timer
/// armed sooner than it, or `cancel` taking the nearest one out, after which
/// it re-arms for the next. A cancelled timer leaves the set, so it wakes
/// nothing at its deadline (#2832).
#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(test)]
mod tests;
