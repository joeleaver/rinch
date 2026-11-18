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
pub mod virtual_time {
    use std::cell::RefCell;
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    use std::time::Duration;

    use crate::main_thread::{MainCallbackId, cancel_main_callback};

    struct Clock {
        /// The wall-clock instant the clock started at: the epoch of the
        /// instants [`now`] answers.
        start: web_time::Instant,
        /// The virtual time since `start`.
        elapsed: Duration,
        /// Waiting timers by deadline, then by the order they were armed.
        heap: BinaryHeap<Reverse<(Duration, u64, u64)>>,
        /// The next arming's sequence number.
        seq: u64,
        /// How many installs are outstanding.
        depth: usize,
    }

    thread_local! {
        static CLOCK: RefCell<Option<Clock>> = const { RefCell::new(None) };
    }

    /// Install the thread's virtual clock at zero, or nest in the one there.
    pub fn install() {
        CLOCK.with(|c| {
            let mut c = c.borrow_mut();
            match c.as_mut() {
                Some(clock) => clock.depth += 1,
                None => {
                    *c = Some(Clock {
                        start: web_time::Instant::now(),
                        elapsed: Duration::ZERO,
                        heap: BinaryHeap::new(),
                        seq: 0,
                        depth: 1,
                    })
                }
            }
        });
    }

    /// Undo one [`install`]. The last one removes the clock and cancels the
    /// timers still waiting on it.
    pub fn uninstall() {
        let gone = CLOCK.with(|c| {
            let mut c = c.borrow_mut();
            let clock = c.as_mut()?;
            clock.depth -= 1;
            if clock.depth > 0 {
                return None;
            }
            c.take()
        });
        // Cancelled outside the borrow: dropping a callback can run code.
        if let Some(clock) = gone {
            for Reverse((_, _, id)) in clock.heap {
                cancel_main_callback(MainCallbackId::from_raw(id));
            }
        }
    }

    /// Whether this thread has a virtual clock.
    pub fn is_installed() -> bool {
        CLOCK.with(|c| c.borrow().is_some())
    }

    /// The virtual time since the clock was installed; `None` without one.
    pub fn elapsed() -> Option<Duration> {
        CLOCK.with(|c| c.borrow().as_ref().map(|c| c.elapsed))
    }

    /// The clock's instant now; `None` without one.
    pub(super) fn now() -> Option<web_time::Instant> {
        CLOCK.with(|c| c.borrow().as_ref().map(|c| c.start + c.elapsed))
    }

    /// Queue timer `id` on the virtual clock, if this thread has one.
    pub(super) fn schedule(id: u64, delay_ms: u32) -> bool {
        CLOCK.with(|c| {
            let mut c = c.borrow_mut();
            let Some(clock) = c.as_mut() else {
                return false;
            };
            let deadline = clock.elapsed + Duration::from_millis(u64::from(delay_ms));
            let seq = clock.seq;
            clock.seq += 1;
            clock.heap.push(Reverse((deadline, seq, id)));
            true
        })
    }

    /// Take timer `id` off the virtual clock, if this thread has one (then
    /// `true`, whether or not `id` was waiting on it).
    pub(super) fn cancel(id: u64) -> bool {
        CLOCK.with(|c| {
            let mut c = c.borrow_mut();
            let Some(clock) = c.as_mut() else {
                return false;
            };
            clock.heap.retain(|&Reverse((_, _, waiting))| waiting != id);
            true
        })
    }

    /// The sequence number the next timer armed will get: a timer armed from
    /// now on has one at least this.
    pub fn next_seq() -> u64 {
        CLOCK.with(|c| c.borrow().as_ref().map_or(0, |c| c.seq))
    }

    /// Take the earliest waiting timer due at or before `until` (virtual
    /// time) and armed before sequence number `armed_before`, moving the clock
    /// to its deadline, and answer its id for [`super::fire_timeout`]. Timers
    /// due together come in the order they were armed. `None` when there is
    /// none (or no clock).
    pub fn take_due(until: Duration, armed_before: u64) -> Option<u64> {
        CLOCK.with(|c| {
            let mut c = c.borrow_mut();
            let clock = c.as_mut()?;
            let &Reverse((deadline, seq, id)) = clock.heap.peek()?;
            if deadline > until || seq >= armed_before {
                return None;
            }
            clock.heap.pop();
            clock.elapsed = clock.elapsed.max(deadline);
            Some(id)
        })
    }

    /// Move the clock forward to `to` (it never goes back).
    pub fn advance_to(to: Duration) {
        CLOCK.with(|c| {
            if let Some(clock) = c.borrow_mut().as_mut() {
                clock.elapsed = clock.elapsed.max(to);
            }
        });
    }
}

/// Built-in native scheduler: a single shared daemon thread driving an ordered
/// set of deadlines. Sleeps (`Condvar::wait_timeout`) until the nearest
/// deadline, or indefinitely when there is none, and hops each fired id to the
/// main thread. It is woken only when the nearest deadline changes: a timer
/// armed sooner than it, or `cancel` taking the nearest one out, after which
/// it re-arms for the next. A cancelled timer leaves the set, so it wakes
/// nothing at its deadline (#2832).
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::{BTreeSet, HashMap};
    use std::sync::{Condvar, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use crate::reactive::run_on_main_thread;

    #[derive(Default)]
    struct Pending {
        /// Ordered by deadline; ties broken by id.
        by_deadline: BTreeSet<(Instant, u64)>,
        /// Each pending id's deadline, to find it in `by_deadline` on a cancel.
        deadlines: HashMap<u64, Instant>,
    }

    pub(super) struct Scheduler {
        pending: Mutex<Pending>,
        wake: Condvar,
        /// Hands a due id to the main thread.
        post: Box<dyn Fn(u64) + Send + Sync>,
        /// How many times the thread woke because a deadline came.
        #[cfg(test)]
        pub(super) deadline_wakes: std::sync::atomic::AtomicUsize,
        /// How many ids it handed to the main thread.
        #[cfg(test)]
        pub(super) main_wakes: std::sync::atomic::AtomicUsize,
    }

    impl Scheduler {
        /// A scheduler with its own thread, living for the rest of the process.
        fn start(post: Box<dyn Fn(u64) + Send + Sync>) -> &'static Scheduler {
            let sched: &'static Scheduler = Box::leak(Box::new(Scheduler {
                pending: Mutex::new(Pending::default()),
                wake: Condvar::new(),
                post,
                #[cfg(test)]
                deadline_wakes: Default::default(),
                #[cfg(test)]
                main_wakes: Default::default(),
            }));
            std::thread::Builder::new()
                .name("rinch-timer".into())
                .spawn(move || sched.run())
                .expect("failed to spawn rinch-timer thread");
            sched
        }

        pub(super) fn schedule(&self, id: u64, delay_ms: u32) {
            self.schedule_at(
                id,
                Instant::now() + Duration::from_millis(u64::from(delay_ms)),
            );
        }

        /// Schedule `a` and `b` on one and the same deadline, `delay` from now.
        #[cfg(test)]
        pub(super) fn schedule_at_for_test(&self, a: u64, b: u64, delay: Duration) {
            let deadline = Instant::now() + delay;
            self.schedule_at(a, deadline);
            self.schedule_at(b, deadline);
        }

        fn schedule_at(&self, id: u64, deadline: Instant) {
            let mut p = self.pending.lock().unwrap();
            p.by_deadline.insert((deadline, id));
            p.deadlines.insert(id, deadline);
            // Wake the thread only if this is now the nearest deadline: it is
            // sleeping until a later one, or indefinitely.
            if p.by_deadline.first() == Some(&(deadline, id)) {
                self.wake.notify_one();
            }
        }

        pub(super) fn cancel(&self, id: u64) {
            let mut p = self.pending.lock().unwrap();
            let Some(deadline) = p.deadlines.remove(&id) else {
                return;
            };
            let was_nearest = p.by_deadline.first() == Some(&(deadline, id));
            p.by_deadline.remove(&(deadline, id));
            // The thread is sleeping until this deadline: have it re-arm for
            // the next one (or sleep indefinitely) instead.
            if was_nearest {
                self.wake.notify_one();
            }
        }

        fn run(&self) {
            let mut p = self.pending.lock().unwrap();
            loop {
                let now = Instant::now();
                // Hand every timer whose deadline has passed to the main thread.
                while let Some(&(deadline, id)) = p.by_deadline.first() {
                    if deadline > now {
                        break;
                    }
                    p.by_deadline.pop_first();
                    p.deadlines.remove(&id);
                    #[cfg(test)]
                    self.main_wakes
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    // Called with the scheduler's mutex held: the cross-thread
                    // dispatcher behind `post` must not block (it queues the
                    // id and wakes the event loop), or every `set_timeout` and
                    // `clear_timeout` on the main thread waits behind it.
                    (self.post)(id);
                }
                // Sleep until the next deadline, or indefinitely if none,
                // unless woken because the nearest deadline changed.
                p = match p.by_deadline.first() {
                    Some(&(deadline, _)) => {
                        let dur = deadline.saturating_duration_since(Instant::now());
                        let (p, _timeout) = self.wake.wait_timeout(p, dur).unwrap();
                        #[cfg(test)]
                        if _timeout.timed_out() {
                            self.deadline_wakes
                                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        }
                        p
                    }
                    None => self.wake.wait(p).unwrap(),
                };
            }
        }
    }

    static SCHEDULER: OnceLock<&'static Scheduler> = OnceLock::new();

    fn scheduler() -> &'static Scheduler {
        SCHEDULER.get_or_init(|| {
            // `fire_timeout` resumes the parked (cancel-aware) callback there.
            Scheduler::start(Box::new(|id| {
                run_on_main_thread(move || super::fire_timeout(id))
            }))
        })
    }

    pub(super) fn schedule(id: u64, delay_ms: u32) {
        scheduler().schedule(id, delay_ms);
    }

    pub(super) fn cancel(id: u64) {
        // No scheduler yet means no native timer was ever armed.
        if let Some(sched) = SCHEDULER.get() {
            sched.cancel(id);
        }
    }

    #[cfg(test)]
    thread_local! {
        /// A test's own scheduler, used by the timers this thread arms instead
        /// of the process-wide one (other tests in the binary install manual
        /// backends process-wide).
        static TEST_SCHEDULER: std::cell::Cell<Option<&'static Scheduler>> =
            const { std::cell::Cell::new(None) };
    }

    #[cfg(test)]
    pub(super) fn test_scheduler() -> Option<&'static Scheduler> {
        TEST_SCHEDULER.with(|s| s.get())
    }

    /// Give this thread a scheduler of its own whose due ids arrive on the
    /// returned channel, with the instant each was handed over; the test plays
    /// the main thread by passing each to `fire_timeout`.
    #[cfg(test)]
    pub(super) fn install_test_scheduler() -> (
        &'static Scheduler,
        std::sync::mpsc::Receiver<(u64, Instant)>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Mutex::new(tx);
        let sched = Scheduler::start(Box::new(move |id| {
            let _ = tx.lock().unwrap().send((id, Instant::now()));
        }));
        TEST_SCHEDULER.with(|s| s.set(Some(sched)));
        (sched, rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    /// Drive timers synchronously: record what was scheduled, fire it on demand.
    /// (Mirrors what a real backend does, minus the waiting.)
    fn install_manual_backend() {
        fn manual(id: u64, delay_ms: u32) {
            SCHEDULED.with(|s| s.borrow_mut().push(id));
            DELAYS.with(|d| d.borrow_mut().push(delay_ms));
        }
        set_timer_backend(manual);
    }

    thread_local! {
        static SCHEDULED: std::cell::RefCell<Vec<u64>> = const { std::cell::RefCell::new(Vec::new()) };
        static DELAYS: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn drain_and_fire() {
        let ids: Vec<u64> = SCHEDULED.with(|s| std::mem::take(&mut *s.borrow_mut()));
        for id in ids {
            fire_timeout(id);
        }
    }

    /// Outside a harness, [`now`] is the wall clock (#2839): it lands between
    /// two `Instant::now()` read either side of it, here and after a sleep.
    #[test]
    fn now_is_the_wall_clock_without_a_harness() {
        for _ in 0..2 {
            let before = Instant::now();
            let n = now();
            let after = Instant::now();
            assert!(
                before <= n && n <= after,
                "{before:?} <= {n:?} <= {after:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Under a virtual clock [`now`] moves by exactly what the clock is
    /// advanced by, and not with the wall clock. The clock is the thread's:
    /// another thread reads the wall clock meanwhile, and once uninstalled
    /// this one does again (#2839).
    #[test]
    fn now_follows_the_threads_virtual_clock() {
        use std::time::Duration;
        virtual_time::install();
        let t0 = now();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(now(), t0, "the wall clock moved; the virtual one did not");
        virtual_time::advance_to(Duration::from_millis(299));
        assert_eq!(now() - t0, Duration::from_millis(299));
        virtual_time::advance_to(Duration::from_millis(300));
        assert_eq!(now() - t0, Duration::from_millis(300));

        let (before, other, after) = std::thread::spawn(|| (Instant::now(), now(), Instant::now()))
            .join()
            .unwrap();
        assert!(
            before <= other && other <= after,
            "another thread reads the wall clock"
        );

        virtual_time::uninstall();
        let before = Instant::now();
        let n = now();
        assert!(
            before <= n && n <= Instant::now(),
            "uninstalled: the wall clock again"
        );
    }

    #[test]
    fn fires_once_and_can_be_cancelled() {
        install_manual_backend();

        // Fires when the deadline arrives.
        let hits = Rc::new(std::cell::Cell::new(0));
        let h = hits.clone();
        set_timeout(10, move || h.set(h.get() + 1));
        drain_and_fire();
        assert_eq!(hits.get(), 1);

        // One-shot: firing again does nothing (the callback was taken).
        drain_and_fire();
        assert_eq!(hits.get(), 1);

        // Cancelled timers never run — the debounce case.
        let h2 = hits.clone();
        let t = set_timeout(10, move || h2.set(h2.get() + 100));
        clear_timeout(t);
        drain_and_fire();
        assert_eq!(hits.get(), 1, "cleared timeout must not fire");

        // clear_timeout after firing is a harmless no-op.
        clear_timeout(t);
    }

    /// A timer fires outside any document — the shells drain it between
    /// events — but it runs as the code of the document that armed it (issue
    /// #963). So an interceptor a timer armed in document 1 registers, and one
    /// it clears, is document 1's: document 2's own interceptor is untouched
    /// by both. Before, the fire ran unmarked, and an unmarked install or clear
    /// reaches every document on the thread (#960's slot rule).
    #[test]
    fn a_timer_registers_and_clears_interceptors_for_the_document_that_armed_it() {
        use crate::context::push_dispatching_doc;
        use crate::events::{
            KeyEventData, clear_keyboard_interceptor, dispatch_keyboard_event,
            set_keyboard_interceptor,
        };
        use std::cell::RefCell;

        install_manual_backend();
        clear_keyboard_interceptor();
        let hits: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
        let key = KeyEventData::new("x".to_string(), "KeyX".to_string());
        let dispatch_as = |doc: u64| {
            let _d = push_dispatching_doc(doc);
            dispatch_keyboard_event(&key)
        };

        {
            let _b = push_dispatching_doc(2);
            let h = hits.clone();
            set_keyboard_interceptor(move |_| {
                h.borrow_mut().push("doc2");
                true
            });
        }
        {
            let _a = push_dispatching_doc(1);
            let h = hits.clone();
            set_timeout(0, move || {
                set_keyboard_interceptor(move |_| {
                    h.borrow_mut().push("doc1-timer");
                    true
                })
            });
        }
        drain_and_fire();
        assert!(dispatch_as(1));
        assert!(dispatch_as(2));
        assert_eq!(*hits.borrow(), ["doc1-timer", "doc2"]);

        {
            let _a = push_dispatching_doc(1);
            set_timeout(0, clear_keyboard_interceptor);
        }
        drain_and_fire();
        hits.borrow_mut().clear();
        assert!(!dispatch_as(1), "document 1's timer cleared document 1's");
        assert!(dispatch_as(2), "and left document 2's alone");
        assert_eq!(*hits.borrow(), ["doc2"]);

        clear_keyboard_interceptor();
    }

    #[test]
    fn callback_need_not_be_send_and_may_be_re_armed() {
        install_manual_backend();

        // An `Rc` capture — the whole point of parking main-thread-side.
        let log = Rc::new(std::cell::RefCell::new(Vec::<&'static str>::new()));

        let l = log.clone();
        let first = set_timeout(5, move || l.borrow_mut().push("first"));
        // Debounce: cancel and re-arm before the deadline.
        clear_timeout(first);
        let l = log.clone();
        set_timeout(5, move || l.borrow_mut().push("second"));

        drain_and_fire();
        assert_eq!(*log.borrow(), vec!["second"]);
    }

    fn take_delays() -> Vec<u32> {
        DELAYS.with(|d| std::mem::take(&mut *d.borrow_mut()))
    }

    fn intervals_alive() -> usize {
        INTERVALS.with(|m| m.borrow().len())
    }

    #[test]
    fn an_interval_fires_every_period_until_cleared() {
        install_manual_backend();
        let hits = Rc::new(std::cell::Cell::new(0));
        let h = hits.clone();
        let tick = set_interval(10, move || h.set(h.get() + 1));
        for expected in 1..=3 {
            drain_and_fire();
            assert_eq!(hits.get(), expected, "tick {expected} fired and re-armed");
        }
        // (The manual backend fires each tick early, so the delays it saw
        // are not periods; the two rate tests below wait for real.)
        take_delays();

        clear_interval(tick);
        assert_eq!(intervals_alive(), 0);
        drain_and_fire();
        assert_eq!(hits.get(), 3, "a cleared interval does not fire");
        assert!(
            SCHEDULED.with(|s| s.borrow().is_empty()),
            "and nothing re-armed it"
        );
        clear_interval(tick); // idempotent
    }

    #[test]
    fn an_interval_may_clear_itself_from_its_own_callback() {
        install_manual_backend();
        let hits = Rc::new(std::cell::Cell::new(0));
        let me: Rc<std::cell::Cell<Option<IntervalHandle>>> = Rc::new(std::cell::Cell::new(None));
        let (h, m) = (hits.clone(), me.clone());
        let tick = set_interval(5, move || {
            h.set(h.get() + 1);
            if h.get() == 2 {
                clear_interval(m.get().unwrap());
            }
        });
        me.set(Some(tick));
        drain_and_fire();
        drain_and_fire();
        assert_eq!(hits.get(), 2);
        assert!(
            SCHEDULED.with(|s| s.borrow().is_empty()),
            "the tick that cleared it armed no further tick"
        );
        assert_eq!(intervals_alive(), 0);
    }

    #[test]
    fn an_interval_armed_in_a_component_stops_when_it_unmounts() {
        install_manual_backend();
        let hits = Rc::new(std::cell::Cell::new(0));
        let scope = crate::reactive::Scope::new();
        let h = hits.clone();
        scope.run(|| set_interval(10, move || h.set(h.get() + 1)));
        drain_and_fire();
        assert_eq!(hits.get(), 1);
        assert_eq!(intervals_alive(), 1);

        scope.dispose();
        assert_eq!(intervals_alive(), 0, "unmount drops the interval");
        drain_and_fire();
        assert_eq!(hits.get(), 1, "and it does not fire again");
    }

    /// Fixed rate, not fixed delay: the next tick is scheduled from the last
    /// tick's deadline, so a callback that takes 25ms of a 40ms period leaves
    /// about 15ms to the next tick (a fixed-delay timer would wait 40).
    #[test]
    fn a_slow_callback_does_not_push_the_next_tick_back() {
        install_manual_backend();
        set_interval(40, || {
            std::thread::sleep(std::time::Duration::from_millis(25))
        });
        take_delays();
        std::thread::sleep(std::time::Duration::from_millis(40));
        drain_and_fire();
        let next = take_delays();
        assert_eq!(next.len(), 1);
        assert!(
            next[0] < 30,
            "the next tick keeps the rate, got {}ms",
            next[0]
        );
    }

    /// Between one and two periods late (here ~1.5): the tick it missed is
    /// skipped too, not fired at once. Off the fixed points: exactly one or
    /// two periods late is where a `<` / `<=` or a `+ period` slip agrees.
    #[test]
    fn a_tick_one_and_a_half_periods_late_skips_the_missed_tick() {
        install_manual_backend();
        set_interval(40, || {
            std::thread::sleep(std::time::Duration::from_millis(60))
        });
        take_delays();
        std::thread::sleep(std::time::Duration::from_millis(40));
        drain_and_fire();
        let next = take_delays();
        assert_eq!(next.len(), 1);
        assert!(
            next[0] >= 20,
            "the tick due 20ms ago is skipped; the next is a period from now \
             (got {}ms)",
            next[0]
        );
    }

    /// A tick more than a period late skips the ticks it missed instead of
    /// firing them back to back.
    #[test]
    fn a_tick_more_than_a_period_late_skips_the_missed_ticks() {
        install_manual_backend();
        set_interval(20, || {
            std::thread::sleep(std::time::Duration::from_millis(60))
        });
        take_delays();
        std::thread::sleep(std::time::Duration::from_millis(20));
        drain_and_fire();
        let next = take_delays();
        assert_eq!(next.len(), 1);
        assert!(
            next[0] >= 10,
            "two ticks were missed; the next comes a period from now, not at once \
             (got {}ms)",
            next[0]
        );
    }

    /// A cleared timer wakes nothing at its deadline (#2832): neither the
    /// timer thread nor the main thread. These run on a native scheduler of
    /// the test's own (a real thread on the wall clock), counting the thread's
    /// deadline wakes and the ids it hands to the main thread; the test plays
    /// the main thread, passing each id it is handed to `fire_timeout`.
    /// Deadlines are short and the bounds generous: what is pinned is a
    /// count, and the order of the deadlines, not their exact time.
    #[cfg(not(target_arch = "wasm32"))]
    mod cleared_timers_2832 {
        use super::super::native::{Scheduler, install_test_scheduler};
        use super::*;
        use std::cell::Cell;
        use std::sync::atomic::Ordering::SeqCst;
        use std::sync::mpsc::Receiver;
        use std::time::{Duration, Instant};

        /// Be the main thread for `dur`: fire each id the scheduler hands
        /// over, and answer them with when each was handed over.
        fn pump(rx: &Receiver<(u64, Instant)>, dur: Duration) -> Vec<(u64, Instant)> {
            pump_until(rx, dur, || false)
        }

        /// [`pump`], stopping early once `done` says so.
        fn pump_until(
            rx: &Receiver<(u64, Instant)>,
            dur: Duration,
            done: impl Fn() -> bool,
        ) -> Vec<(u64, Instant)> {
            let end = Instant::now() + dur;
            let mut posts = Vec::new();
            while !done() {
                let left = end.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                if let Ok((id, at)) = rx.recv_timeout(left) {
                    fire_timeout(id);
                    posts.push((id, at));
                }
            }
            posts
        }

        fn wakes(s: &Scheduler) -> (usize, usize) {
            (s.deadline_wakes.load(SeqCst), s.main_wakes.load(SeqCst))
        }

        fn ms(n: u64) -> Duration {
            Duration::from_millis(n)
        }

        /// Give the timer thread time to take the timer just armed and go to
        /// sleep until its deadline, so a clear after this has a sleeping
        /// thread to re-arm (cleared before the thread ever ran, it would
        /// never have seen the timer, and the test would pin nothing).
        fn let_it_arm() {
            std::thread::sleep(ms(10));
        }

        #[test]
        fn a_cleared_timeout_wakes_nothing_at_its_deadline() {
            let (sched, rx) = install_test_scheduler();
            let hit = Rc::new(Cell::new(false));
            let h = hit.clone();
            let t = set_timeout(40, move || h.set(true));
            let_it_arm();
            clear_timeout(t);
            let posts = pump(&rx, ms(150));
            assert!(posts.is_empty(), "nothing reached the main thread");
            assert_eq!(
                wakes(sched),
                (0, 0),
                "(timer-thread deadline wakes, main-thread wakes)"
            );
            assert!(!hit.get());
        }

        #[test]
        fn clearing_the_earliest_of_two_wakes_once_at_the_second() {
            let (sched, rx) = install_test_scheduler();
            let start = Instant::now();
            let hit = Rc::new(Cell::new(0));
            let (h1, h2) = (hit.clone(), hit.clone());
            let first = set_timeout(30, move || h1.set(h1.get() + 1));
            let second = set_timeout(90, move || h2.set(h2.get() + 10));
            let_it_arm();
            clear_timeout(first);
            let posts = pump(&rx, ms(250));
            assert_eq!(wakes(sched), (1, 1));
            assert_eq!(posts.len(), 1);
            assert_eq!(posts[0].0, second.0.raw(), "the wake is the second one's");
            assert!(
                posts[0].1 >= start + ms(90),
                "at the second deadline, not the first ({:?} after start)",
                posts[0].1 - start
            );
            assert_eq!(hit.get(), 10, "and the second one fired");
        }

        #[test]
        fn a_cleared_interval_wakes_nothing_at_its_deadline() {
            let (sched, rx) = install_test_scheduler();
            let hits = Rc::new(Cell::new(0));
            let h = hits.clone();
            let tick = set_interval(40, move || h.set(h.get() + 1));
            let_it_arm();
            clear_interval(tick);
            let posts = pump(&rx, ms(150));
            assert!(posts.is_empty());
            assert_eq!(wakes(sched), (0, 0));
            assert_eq!(hits.get(), 0);
        }

        #[test]
        fn clearing_the_earliest_of_two_intervals_wakes_only_for_the_second() {
            let (sched, rx) = install_test_scheduler();
            let start = Instant::now();
            let (a_hits, b_hits) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
            let (a, b) = (a_hits.clone(), b_hits.clone());
            let ia = set_interval(30, move || a.set(a.get() + 1));
            let ib = set_interval(100, move || b.set(b.get() + 1));
            let_it_arm();
            clear_interval(ia);
            let posts = pump(&rx, ms(160));
            clear_interval(ib);
            let (deadline_wakes, main_wakes) = wakes(sched);
            assert!(!posts.is_empty(), "the second interval ticked");
            assert_eq!(main_wakes, posts.len());
            assert_eq!(
                deadline_wakes, main_wakes,
                "every deadline wake was a tick of the second interval"
            );
            for &(_, at) in &posts {
                assert!(
                    at >= start + ms(100),
                    "no wake at the cleared interval's deadline ({:?} after start)",
                    at - start
                );
            }
            assert_eq!(a_hits.get(), 0);
            assert_eq!(b_hits.get(), posts.len());
        }

        #[test]
        fn a_timeout_cleared_from_another_timers_callback_wakes_nothing() {
            let (sched, rx) = install_test_scheduler();
            let later: Rc<Cell<Option<TimeoutHandle>>> = Rc::new(Cell::new(None));
            let hit = Rc::new(Cell::new(false));
            let l = later.clone();
            let first = set_timeout(30, move || clear_timeout(l.get().unwrap()));
            let h = hit.clone();
            later.set(Some(set_timeout(90, move || h.set(true))));
            let posts = pump(&rx, ms(250));
            assert_eq!(posts.len(), 1);
            assert_eq!(posts[0].0, first.0.raw());
            assert_eq!(wakes(sched), (1, 1), "only the first one's deadline woke");
            assert!(!hit.get(), "the cleared one did not fire");
        }

        /// After the clear the thread sleeps with no deadline at all; a timer
        /// armed then must still wake it, and at its own (earlier) deadline,
        /// not the cleared one's — and the cleared one's deadline, passed
        /// later, still wakes nothing.
        #[test]
        fn a_sooner_timeout_set_after_a_clear_fires_on_time() {
            let (sched, rx) = install_test_scheduler();
            let start = Instant::now();
            let cleared = set_timeout(300, || panic!("cleared"));
            let_it_arm();
            clear_timeout(cleared);
            let fired: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
            let f = fired.clone();
            set_timeout(40, move || f.set(Some(Instant::now())));
            let posts = pump(&rx, ms(450));
            let at = fired.get().expect("the new timeout fired");
            assert!(at >= start + ms(40), "not early: {:?}", at - start);
            assert!(
                at < start + ms(250),
                "on time, not at the cleared deadline: {:?}",
                at - start
            );
            assert_eq!(posts.len(), 1, "nothing at the cleared deadline");
            assert_eq!(wakes(sched), (1, 1));
        }

        /// Two timers on one deadline (one `Instant`, so the set orders them by
        /// id alone): both fire, and clearing one leaves the other. A
        /// scheduler keyed by deadline alone would keep only one of them, and
        /// a cancel that matched by deadline would take both.
        #[test]
        fn timers_sharing_a_deadline_fire_and_clear_independently() {
            let (sched, rx) = install_test_scheduler();
            let hits = Rc::new(Cell::new(0));
            let (h1, h2) = (hits.clone(), hits.clone());
            // Through the scheduler directly, so both get the very same
            // deadline rather than two `Instant::now()`s.
            let a = park_main_callback::<()>(move |()| h1.set(h1.get() + 1));
            let b = park_main_callback::<()>(move |()| h2.set(h2.get() + 10));
            sched.schedule_at_for_test(a.raw(), b.raw(), ms(40));
            let posts = pump(&rx, ms(150));
            assert_eq!(hits.get(), 11, "both fired");
            assert_eq!(posts.len(), 2);
            assert_eq!(wakes(sched), (1, 2), "one deadline wake for both");

            let (h3, h4) = (hits.clone(), hits.clone());
            let c = park_main_callback::<()>(move |()| h3.set(h3.get() + 100));
            let d = park_main_callback::<()>(move |()| h4.set(h4.get() + 1000));
            sched.schedule_at_for_test(c.raw(), d.raw(), ms(40));
            let_it_arm();
            clear_timeout(TimeoutHandle(c));
            let posts = pump(&rx, ms(150));
            assert_eq!(posts.len(), 1);
            assert_eq!(posts[0].0, d.raw(), "the other one is still there");
            assert_eq!(hits.get(), 1011, "and it fired; the cleared one did not");
            assert_eq!(wakes(sched), (2, 3));
        }

        /// The test harness's virtual clock drops a cleared timer too, so an
        /// advance has no deadline of it to stop at.
        #[test]
        fn a_cleared_timeout_leaves_the_virtual_clock() {
            virtual_time::install();
            let first = set_timeout(50, || panic!("cleared"));
            let second = set_timeout(80, || {});
            clear_timeout(first);
            let due = virtual_time::take_due(Duration::from_millis(100), u64::MAX);
            assert_eq!(due, Some(second.0.raw()), "the cleared one is not due");
            assert_eq!(
                virtual_time::take_due(Duration::from_millis(100), u64::MAX),
                None
            );
            virtual_time::uninstall();
        }
    }
}
