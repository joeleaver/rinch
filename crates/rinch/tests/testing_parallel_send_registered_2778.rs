//! Issue #2778 as deskrinch met it: two `TestApp`s on two threads at once, in
//! a process where some other thread is the registered main thread (as a
//! `RinchContext` or another host makes one). Every `Signal::update_send`
//! then leaves its thread through the dispatcher, the app's own thread
//! included, and used to be run by whichever app turned first. Each app's
//! writes must reach that app only, on its thread. The scenario is in
//! `support/parallel_send.rs`.
//!
//! Its own binary: registering the main thread is per process.

#![cfg(all(feature = "testing", feature = "windowing"))]

#[path = "support/parallel_send.rs"]
mod parallel_send;

#[test]
fn two_test_apps_beside_a_registered_main_thread_each_see_only_their_own_sends() {
    // A host on a thread of its own, queueing onto the shared queue as the
    // embed dispatcher does. It never drains: the apps must drain their own.
    std::thread::spawn(|| {
        rinch::core::set_cross_thread_dispatcher(|f| {
            rinch::core::queue_main_callback(f);
        });
        rinch::core::register_main_thread();
    })
    .join()
    .unwrap();
    assert!(!rinch::core::is_main_thread());

    parallel_send::two_apps_each_see_only_their_own_sends();
}
