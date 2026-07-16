#![cfg(not(target_arch = "wasm32"))]
//! The built-in **native** timer scheduler, end to end.
//!
//! This lives in `tests/` (its own process) on purpose: the timer backend is
//! global state, and the unit tests in `timer.rs` install a manual backend. Here
//! no backend is installed, so `set_timeout` exercises the real native path —
//! worker thread sleeps, then hops back to the main thread via the runtime's
//! cross-thread dispatcher.

use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rinch_core::{clear_timeout, register_main_thread, set_cross_thread_dispatcher, set_timeout};

/// A closure the (simulated) runtime must run on the main thread.
type Job = Box<dyn FnOnce() + Send>;

/// Stands in for the runtime's event loop: closures queued from worker threads,
/// drained on the main thread.
static QUEUE: OnceLock<Mutex<VecDeque<Job>>> = OnceLock::new();

fn queue() -> &'static Mutex<VecDeque<Job>> {
    QUEUE.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn dispatcher(f: Job) {
    queue().lock().unwrap().push_back(f);
}

/// Pump the dispatcher queue on this (main) thread until `done` or the deadline.
fn pump_until(done: impl Fn() -> bool, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        while let Some(job) = queue().lock().unwrap().pop_front() {
            job();
        }
        if done() {
            return true;
        }
        if start.elapsed() > timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn native_scheduler_fires_on_the_main_thread_and_honours_cancel() {
    register_main_thread();
    set_cross_thread_dispatcher(dispatcher);

    let main_thread = std::thread::current().id();

    // An `Rc` capture: the callback is parked main-thread-side and must never be
    // required to be `Send`. This would not compile if it were.
    let fired_on = Rc::new(std::cell::Cell::new(None::<std::thread::ThreadId>));

    let f = fired_on.clone();
    set_timeout(20, move || f.set(Some(std::thread::current().id())));

    assert!(
        pump_until(|| fired_on.get().is_some(), Duration::from_secs(5)),
        "native timer never fired"
    );
    assert_eq!(
        fired_on.get(),
        Some(main_thread),
        "callback must run on the main thread, not the sleeping worker"
    );

    // A cancelled timer must stay silent even after its deadline passes.
    let ran = Rc::new(std::cell::Cell::new(false));
    let r = ran.clone();
    let h = set_timeout(20, move || r.set(true));
    clear_timeout(h);

    // Pump well past the deadline; nothing should run.
    let _ = pump_until(|| false, Duration::from_millis(200));
    assert!(!ran.get(), "cleared timeout must not fire");
}
