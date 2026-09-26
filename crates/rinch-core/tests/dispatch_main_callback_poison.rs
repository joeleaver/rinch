//! Issue #1035 (review of PR #1059): a poisoned dispatcher lock is read through.
use std::sync::atomic::{AtomicBool, Ordering};

static PANIC_ONCE: AtomicBool = AtomicBool::new(true);

fn dispatcher(f: Box<dyn FnOnce() + Send>) {
    if PANIC_ONCE.swap(false, Ordering::SeqCst) {
        panic!("poison the dispatcher lock");
    }
    rinch_core::queue_main_callback(f);
}

#[test]
fn a_poisoned_lock_is_read_through() {
    rinch_core::register_main_thread();
    rinch_core::set_cross_thread_dispatcher(dispatcher);
    rinch_core::drain_main_callbacks();
    // run_on_main_thread from a worker holds the lock across the dispatcher.
    let r = std::thread::spawn(|| rinch_core::run_on_main_thread(|| {})).join();
    assert!(
        r.is_err(),
        "positive control: the dispatcher panicked under the lock"
    );
    let r = std::thread::spawn(|| rinch_core::dispatch_main_callback(Box::new(|| {}))).join();
    assert!(
        r.is_ok(),
        "dispatch_main_callback panicked on a poisoned lock"
    );
    assert!(rinch_core::main_callbacks_pending());
    rinch_core::drain_main_callbacks();
}
