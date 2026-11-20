//! Issue #2778: two `TestApp`s on two threads at once, in a process where no
//! host has registered a main thread (an app's ordinary test binary). Each
//! app's `Signal::update_send` writes, from its own thread and from a worker,
//! reach that app only, on its thread. The scenario is in
//! `support/parallel_send.rs`.
//!
//! Its own binary: `testing_parallel_send_registered_2778.rs` registers a
//! main thread, which is per process.

#![cfg(all(feature = "testing", feature = "windowing"))]

#[path = "support/parallel_send.rs"]
mod parallel_send;

#[test]
fn two_test_apps_on_two_threads_each_see_only_their_own_sends() {
    parallel_send::two_apps_each_see_only_their_own_sends();
}
