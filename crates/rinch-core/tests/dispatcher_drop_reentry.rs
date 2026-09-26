//! The cross-thread dispatcher must not be called under its own slot's lock
//! (issue #1061's sweep).
//!
//! `run_on_main_thread` from a worker thread hands its closure to the
//! registered dispatcher. A dispatcher may drop a closure instead of queueing
//! it (a host whose loop has gone), and a closure's captures may dispatch again
//! from their `Drop` — from the same worker thread. With the dispatcher called
//! while `CROSS_THREAD_DISPATCHER` was locked, that second dispatch re-locked it
//! on the same thread: a deadlock. `dispatch_main_callback` already copied the
//! function pointer out first; `run_on_main_thread` did not.
//!
//! Registers the main thread and a dispatcher, both process-global, so it lives
//! in its own binary.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use rinch_core::{register_main_thread, run_on_main_thread, set_cross_thread_dispatcher};

static DISPATCHED: AtomicUsize = AtomicUsize::new(0);

/// A dispatcher whose host has gone: it drops what it is given.
fn dropping_dispatcher(f: Box<dyn FnOnce() + Send>) {
    DISPATCHED.fetch_add(1, Ordering::SeqCst);
    drop(f);
}

/// Dispatches a no-op from its `Drop`.
struct DispatchesOnDrop(Arc<AtomicUsize>);

impl Drop for DispatchesOnDrop {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
        run_on_main_thread(|| {});
    }
}

#[test]
fn a_dispatcher_dropping_a_closure_whose_drop_dispatches_does_not_deadlock() {
    register_main_thread();
    set_cross_thread_dispatcher(dropping_dispatcher);

    let dropped = Arc::new(AtomicUsize::new(0));
    let guard = DispatchesOnDrop(dropped.clone());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        run_on_main_thread(move || drop(guard));
        let _ = tx.send(());
    });
    rx.recv_timeout(Duration::from_secs(5))
        .expect("run_on_main_thread deadlocked on a re-entrant dispatch (issue #1061)");

    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    // Positive control: both the outer closure and the one its drop sent
    // reached the dispatcher, so the re-entrant path was really taken.
    assert_eq!(DISPATCHED.load(Ordering::SeqCst), 2);
}
