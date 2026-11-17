//! Issue #2637 beside #2778: a drain off the registered main thread is refused, but a host thread
//! (`mark_host_thread`, one per `rinch::testing::TestApp`) is not "off the main thread". With a
//! main thread registered elsewhere in the process, a worker's write to a host thread's signal is
//! queued for that host and applied by the host's own drain.
//!
//! Its own binary: registering the main thread is process-wide, and
//! `drain_off_main_thread_2637.rs` needs its test thread to be the registered one.

use rinch_core::{
    Signal, drain_main_callbacks, main_callbacks_pending, mark_host_thread, queue_main_callback,
    register_main_thread, set_cross_thread_dispatcher,
};

#[test]
fn a_host_thread_drains_with_a_main_thread_registered_elsewhere() {
    // The registered main thread is another thread, which has ended: this test's threads are
    // none of them it.
    std::thread::spawn(register_main_thread).join().unwrap();
    set_cross_thread_dispatcher(|f| {
        queue_main_callback(f);
    });

    let got = std::thread::spawn(|| {
        mark_host_thread();
        assert!(
            rinch_core::is_main_thread(),
            "a host thread answers is_main_thread"
        );
        let items: Signal<Vec<&'static str>> = Signal::new(Vec::new());
        std::thread::spawn(move || items.update_send(|v| v.push("from a worker")))
            .join()
            .unwrap();
        assert!(
            main_callbacks_pending(),
            "the worker's write is queued for this host"
        );
        drain_main_callbacks();
        items.get()
    })
    .join()
    .unwrap();
    assert_eq!(
        got,
        ["from a worker"],
        "the host thread's drain applied the write"
    );
}
