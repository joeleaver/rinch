//! Cross-platform one-shot timers.
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
//! - **Web** has no threads, so `rinch-web` installs a backend
//!   ([`set_timer_backend`]) that drives `window.setTimeout`.
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
/// `rinch-web` installs a `window.setTimeout` backend. On native a built-in
/// shared-thread scheduler is used when no backend is installed.
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

/// Run the callback parked for `id`, if it hasn't been cancelled.
///
/// Called by the timer backend **on the main thread** when the delay elapses.
/// This is a backend entry point (needed public so `rinch-web` can call it), not
/// app API — prefer [`set_timeout`] / [`clear_timeout`].
#[doc(hidden)]
pub fn fire_timeout(id: u64) {
    resume_main_callback(MainCallbackId::from_raw(id), ());
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
             The web runtime installs one at startup."
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
        fn manual(id: u64, _delay_ms: u32) {
            SCHEDULED.with(|s| s.borrow_mut().push(id));
        }
        set_timer_backend(manual);
    }

    thread_local! {
        static SCHEDULED: std::cell::RefCell<Vec<u64>> = const { std::cell::RefCell::new(Vec::new()) };
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
}
