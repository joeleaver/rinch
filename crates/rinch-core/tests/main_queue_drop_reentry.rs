//! Dropping a queued main-thread closure must not happen under the queue's own
//! lock (issue #1061).
//!
//! A closure's captures can queue main-thread work from their own `Drop` — a
//! render surface's `SurfaceLease` releases itself through
//! `dispatch_main_callback` when it cannot reach its registry, and any user type
//! whose `Drop` calls `Signal::send` does the same. `std::sync::Mutex` is not
//! re-entrant, so such a drop under `MAIN_QUEUE`'s lock deadlocked the thread
//! doing it; the only caller of `clear_main_callbacks` is desktop shutdown, so
//! the symptom was an app that hung on exit.
//!
//! Every call that could block forever runs on a spawned thread and is awaited
//! with a timeout, so a regression fails this test instead of hanging the
//! suite. `MAIN_QUEUE` is process-global, hence one test in its own binary.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use rinch_core::{
    clear_main_callbacks, drain_main_callbacks, main_callbacks_pending, queue_main_callback,
};

/// Queues a closure from its `Drop`, `depth` levels deep: the queued closure
/// captures another `QueuesOnDrop` one level shallower. Counts every closure it
/// queued that ever *ran*.
struct QueuesOnDrop {
    depth: usize,
    ran: Arc<AtomicUsize>,
}

impl Drop for QueuesOnDrop {
    fn drop(&mut self) {
        let ran = self.ran.clone();
        let inner = (self.depth > 0).then(|| QueuesOnDrop {
            depth: self.depth - 1,
            ran: self.ran.clone(),
        });
        queue_main_callback(Box::new(move || {
            ran.fetch_add(1, Ordering::SeqCst);
            drop(inner);
        }));
    }
}

/// Run `f` on another thread and fail — rather than hang — if it does not
/// return within five seconds.
fn within_timeout(what: &str, f: impl FnOnce() + Send + 'static) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        f();
        let _ = tx.send(());
    });
    rx.recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|_| panic!("{what} deadlocked (issue #1061)"));
}

#[test]
fn clearing_a_closure_whose_drop_queues_neither_deadlocks_nor_leaves_work_behind() {
    let ran = Arc::new(AtomicUsize::new(0));

    // A chain three deep: clearing the outer closure drops a guard that queues
    // a closure holding a guard that queues another, and so on. One round of
    // clearing would leave the second link queued.
    let guard = QueuesOnDrop {
        depth: 2,
        ran: ran.clone(),
    };
    queue_main_callback(Box::new(move || drop(guard)));

    within_timeout("clear_main_callbacks", clear_main_callbacks);

    // Shutdown's contract is "drop every pending callback unrun": what a
    // clear's own drops queued is pending work too, and must not survive it to
    // run against torn-down state on a later drain.
    assert!(
        !main_callbacks_pending(),
        "a closure queued by a drop during clear_main_callbacks survived it"
    );
    drain_main_callbacks();
    assert_eq!(
        ran.load(Ordering::SeqCst),
        0,
        "clear_main_callbacks must drop work, never run it"
    );
}
