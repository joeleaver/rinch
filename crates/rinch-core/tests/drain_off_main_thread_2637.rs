//! Issue #2637: a drain off the registered main thread (an embedded context updated on a worker)
//! leaves the main thread's queued work for the main thread, instead of running it where its parked
//! state is not (a fired timer was lost that way, and a `sleep` loop on it stopped for good).
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn a_drain_off_the_main_thread_leaves_the_queue_for_the_main_thread() {
    rinch_core::register_main_thread();
    rinch_core::drain_main_callbacks();
    let ran = Arc::new(AtomicUsize::new(0));
    let on_main = Arc::new(AtomicUsize::new(0));
    let (r, m) = (ran.clone(), on_main.clone());
    rinch_core::dispatch_main_callback(Box::new(move || {
        r.fetch_add(1, Ordering::SeqCst);
        if rinch_core::is_main_thread() {
            m.fetch_add(1, Ordering::SeqCst);
        }
    }));
    std::thread::spawn(rinch_core::drain_main_callbacks)
        .join()
        .unwrap();
    assert_eq!(
        ran.load(Ordering::SeqCst),
        0,
        "a worker's drain ran the main thread's work"
    );
    assert!(
        rinch_core::main_callbacks_pending(),
        "still queued for the main thread"
    );
    rinch_core::drain_main_callbacks();
    assert_eq!(
        (ran.load(Ordering::SeqCst), on_main.load(Ordering::SeqCst)),
        (1, 1),
        "ran once, on the main thread"
    );
}
