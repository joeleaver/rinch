//! Review of #1542 (F4): every host installs its cross-thread dispatcher
//! *before* it registers its main thread. In the other order a worker sending
//! in between finds a main thread registered and no dispatcher, and
//! `dispatch_to_main_thread` panics on the worker. In this order the worker
//! is still unregistered there, and its write goes through the dispatcher
//! (`dispatch_main_callback`) to be applied once the host has registered.
//!
//! The order itself is in the hosts (desktop, Android, embed); this pins that
//! the state between the two calls is a safe one. Own binary: the dispatcher
//! and `MAIN_THREAD_ID` are process-global.

use std::sync::atomic::{AtomicUsize, Ordering};

use rinch_core::{
    Signal, drain_main_callbacks, queue_main_callback, register_main_thread,
    set_cross_thread_dispatcher,
};

static DISPATCHED: AtomicUsize = AtomicUsize::new(0);

fn host_dispatcher(f: Box<dyn FnOnce() + Send>) {
    DISPATCHED.fetch_add(1, Ordering::SeqCst);
    queue_main_callback(f);
}

#[test]
fn a_worker_send_between_dispatcher_and_registration_is_queued_not_lost() {
    let n: Signal<u64> = Signal::new(0);
    set_cross_thread_dispatcher(host_dispatcher);

    // The gap: dispatcher installed, main thread not yet registered.
    std::thread::spawn(move || n.send(5))
        .join()
        .expect("a send in the gap must not panic");
    assert_eq!(
        DISPATCHED.load(Ordering::SeqCst),
        1,
        "it went through the dispatcher"
    );

    register_main_thread();
    drain_main_callbacks();
    assert_eq!(n.get(), 5);
}
