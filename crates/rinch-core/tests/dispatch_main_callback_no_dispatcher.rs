//! Issue #1035 (review of PR #1059): with no dispatcher registered, dispatch_main_callback queues
//! (never runs inline, never panics), from a worker and from the main thread.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn no_dispatcher_queues_from_any_thread_and_never_inline() {
    rinch_core::drain_main_callbacks();
    let n = Arc::new(AtomicUsize::new(0));
    let a = n.clone();
    std::thread::spawn(move || {
        rinch_core::dispatch_main_callback(Box::new(move || {
            a.fetch_add(1, Ordering::SeqCst);
        }))
    })
    .join()
    .unwrap();
    let b = n.clone();
    rinch_core::dispatch_main_callback(Box::new(move || {
        b.fetch_add(10, Ordering::SeqCst);
    }));
    assert_eq!(n.load(Ordering::SeqCst), 0, "nothing ran inline");
    assert!(rinch_core::main_callbacks_pending(), "both queued");
    rinch_core::drain_main_callbacks();
    assert_eq!(n.load(Ordering::SeqCst), 11, "both ran on the drain");
}
