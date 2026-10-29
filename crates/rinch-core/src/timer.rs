//! Cross-platform timers: one-shot ([`set_timeout`]) and repeating ([`set_interval`]).
//!
//! rinch-core is platform-agnostic, so *scheduling* is pluggable while the
//! callback lives in the shared main-thread registry ([`crate::main_thread`]):
//!
//! - **Native** works out of the box: a **single** shared timer thread sleeps
//!   until the earliest pending deadline (a min-heap of timers), then hops each
//!   fired id back to the main thread via the runtime's cross-thread dispatcher
//!   (see [`set_cross_thread_dispatcher`](crate::set_cross_thread_dispatcher)).
//!   One daemon thread serves every timer — arming N timers does not spawn N
//!   threads, so a debounce that re-arms on every keystroke stays cheap.
//! - **Web** has no threads, so the web runtime installs a backend
//!   ([`set_timer_backend`]) that drives `window.setTimeout`:
//!   `rinch::install_web_runtime`, called by every `rinch_web` mount and by
//!   `rinch::embed::RinchContext::new` on wasm32.
//!
//! The callback is parked via [`park_main_callback`](crate::park_main_callback),
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

static BACKEND: Mutex<Option<TimerBackend>> = Mutex::new(None);

/// A scheduled timeout. Pass to [`clear_timeout`] to cancel it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TimeoutHandle(MainCallbackId);

/// Install the platform's timer scheduler. Called by the runtime at startup;
/// on wasm32 the web runtime (`rinch::install_web_runtime`, called by every
/// `rinch_web` mount and by `RinchContext::new`) installs a `window.setTimeout`
/// backend. On native a built-in shared-thread scheduler is used when no
/// backend is installed.
pub fn set_timer_backend(backend: TimerBackend) {
    *BACKEND.lock().unwrap() = Some(backend);
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
/// This drops the parked callback; the underlying platform wake may still occur,
/// but [`fire_timeout`] then finds nothing to run.
pub fn clear_timeout(handle: TimeoutHandle) {
    cancel_main_callback(handle.0);
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
    use web_time::{Duration, Instant};

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
        let delay = deadline.saturating_duration_since(Instant::now());
        let delay_ms = u32::try_from(delay.as_millis()).unwrap_or(u32::MAX);
        set_timeout(delay_ms, move || {
            // Stopped since this tick was armed: nothing to run or re-arm.
            if !INTERVALS.with(|m| m.borrow().contains_key(&id)) {
                return;
            }
            // The callback is called with no registry borrow held, so it may
            // clear this interval or arm others.
            (cb.borrow_mut())();
            let now = Instant::now();
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

    let first = arm(id, Instant::now() + period, period, cb);
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

fn schedule(id: u64, delay_ms: u32) {
    if let Some(backend) = *BACKEND.lock().unwrap() {
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

/// Built-in native scheduler: a single shared daemon thread driving a min-heap of
/// deadlines. Sleeps (`Condvar::wait_timeout`) until the nearest deadline, is
/// woken when a nearer timer is armed, and hops each fired id to the main thread.
/// Cancelled ids stay in the heap and simply no-op when [`fire_timeout`] finds
/// nothing parked — so cancellation costs nothing here.
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    use std::sync::{Condvar, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use crate::reactive::run_on_main_thread;

    struct Scheduler {
        /// Min-heap by deadline (`Reverse` flips the max-heap); ties broken by id.
        heap: Mutex<BinaryHeap<Reverse<(Instant, u64)>>>,
        wake: Condvar,
    }

    static SCHEDULER: OnceLock<&'static Scheduler> = OnceLock::new();

    fn scheduler() -> &'static Scheduler {
        SCHEDULER.get_or_init(|| {
            let sched: &'static Scheduler = Box::leak(Box::new(Scheduler {
                heap: Mutex::new(BinaryHeap::new()),
                wake: Condvar::new(),
            }));
            std::thread::Builder::new()
                .name("rinch-timer".into())
                .spawn(move || run(sched))
                .expect("failed to spawn rinch-timer thread");
            sched
        })
    }

    pub(super) fn schedule(id: u64, delay_ms: u32) {
        let deadline = Instant::now() + Duration::from_millis(u64::from(delay_ms));
        let sched = scheduler();
        sched.heap.lock().unwrap().push(Reverse((deadline, id)));
        // Wake the timer thread: a newly-armed timer may be sooner than what it is
        // currently sleeping for.
        sched.wake.notify_one();
    }

    fn run(sched: &'static Scheduler) {
        let mut heap = sched.heap.lock().unwrap();
        loop {
            let now = Instant::now();
            // Fire every timer whose deadline has passed.
            while let Some(&Reverse((deadline, id))) = heap.peek() {
                if deadline <= now {
                    heap.pop();
                    // Hop back to the main thread; `fire_timeout` resumes the
                    // parked (cancel-aware) callback there.
                    run_on_main_thread(move || super::fire_timeout(id));
                } else {
                    break;
                }
            }
            // Sleep until the next deadline, or indefinitely if none, until woken.
            heap = match heap.peek() {
                Some(&Reverse((deadline, _))) => {
                    let dur = deadline.saturating_duration_since(Instant::now());
                    sched.wake.wait_timeout(heap, dur).unwrap().0
                }
                None => sched.wake.wait(heap).unwrap(),
            };
        }
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
}
