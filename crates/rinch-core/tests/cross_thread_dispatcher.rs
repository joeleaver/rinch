//! Registration semantics for the cross-thread dispatcher (issue #172).
//!
//! Two hosts can arm cross-thread dispatch in one process — a desktop app that
//! also embeds a `RinchContext` — and only one of them owns the event loop.
//! `set_cross_thread_dispatcher` is last-wins, which is what the shell wants
//! (its dispatcher also wakes the loop, and must win); the embedded context uses
//! `set_cross_thread_dispatcher_if_unset` so it can never displace it.
//!
//! `CROSS_THREAD_DISPATCHER` is a process-global `static`, so these live in
//! their own integration-test binary and share one test function: libtest would
//! otherwise run them concurrently against the same slot.

use std::sync::atomic::{AtomicUsize, Ordering};

use rinch_core::{
    clear_main_callbacks, drain_main_callbacks, queue_main_callback, register_main_thread,
    run_on_main_thread, set_cross_thread_dispatcher, set_cross_thread_dispatcher_if_unset,
};

/// How many times each dispatcher was invoked. A dispatcher is a bare `fn`
/// pointer, so its bookkeeping has to be static too.
static SHELL_CALLS: AtomicUsize = AtomicUsize::new(0);
static EMBED_CALLS: AtomicUsize = AtomicUsize::new(0);

/// Stands in for the desktop shell's dispatcher: queue, then wake the loop.
fn shell_dispatcher(f: Box<dyn FnOnce() + Send>) {
    SHELL_CALLS.fetch_add(1, Ordering::Relaxed);
    queue_main_callback(f);
}

/// Stands in for an embedded context's dispatcher: queue only, nothing to wake.
fn embed_dispatcher(f: Box<dyn FnOnce() + Send>) {
    EMBED_CALLS.fetch_add(1, Ordering::Relaxed);
    queue_main_callback(f);
}

/// Dispatch once from a genuinely background thread and report which dispatcher
/// took it.
fn dispatch_from_worker() {
    let before = (
        SHELL_CALLS.load(Ordering::Relaxed),
        EMBED_CALLS.load(Ordering::Relaxed),
    );
    std::thread::spawn(|| run_on_main_thread(|| {}))
        .join()
        .expect("the worker must not panic — a dispatcher is registered");
    let after = (
        SHELL_CALLS.load(Ordering::Relaxed),
        EMBED_CALLS.load(Ordering::Relaxed),
    );
    assert_ne!(before, after, "some dispatcher must have taken the closure");
}

#[test]
fn registration_order_never_costs_a_host_its_dispatcher() {
    // Arm the cross-thread check: without this `is_main_thread()` answers `true`
    // unconditionally and nothing below reaches a dispatcher at all.
    register_main_thread();

    // 1. `_if_unset` installs into an empty slot — this is embed arming itself
    //    in a process with no shell.
    set_cross_thread_dispatcher_if_unset(embed_dispatcher);
    dispatch_from_worker();
    assert_eq!(EMBED_CALLS.load(Ordering::Relaxed), 1);
    assert_eq!(SHELL_CALLS.load(Ordering::Relaxed), 0);

    // 2. A shell starting afterwards takes the slot: plain `set_` is last-wins,
    //    which it must be — only the shell's dispatcher wakes the event loop.
    set_cross_thread_dispatcher(shell_dispatcher);
    dispatch_from_worker();
    assert_eq!(SHELL_CALLS.load(Ordering::Relaxed), 1);
    assert_eq!(
        EMBED_CALLS.load(Ordering::Relaxed),
        1,
        "embed must be off the hook now"
    );

    // 3. A context created *inside* that shell must not displace it — this is
    //    the regression `_if_unset` exists to prevent, since a queue-only
    //    dispatcher in a windowed app would land every cross-thread write a
    //    frame late, or not at all while the loop is idle.
    set_cross_thread_dispatcher_if_unset(embed_dispatcher);
    dispatch_from_worker();
    assert_eq!(SHELL_CALLS.load(Ordering::Relaxed), 2);
    assert_eq!(
        EMBED_CALLS.load(Ordering::Relaxed),
        1,
        "`_if_unset` must not overwrite the shell's dispatcher"
    );

    // Whichever dispatcher ran, every closure went to the one shared queue, so
    // either host's drain site sees all of them.
    let ran = std::sync::Arc::new(AtomicUsize::new(0));
    for _ in 0..3 {
        let ran = ran.clone();
        queue_main_callback(Box::new(move || {
            ran.fetch_add(1, Ordering::Relaxed);
        }));
    }
    drain_main_callbacks();
    assert_eq!(ran.load(Ordering::Relaxed), 3);

    // And the shutdown door drops what is left instead of running it.
    let dropped = std::sync::Arc::new(AtomicUsize::new(0));
    let d = dropped.clone();
    queue_main_callback(Box::new(move || {
        d.fetch_add(1, Ordering::Relaxed);
    }));
    clear_main_callbacks();
    drain_main_callbacks();
    assert_eq!(
        dropped.load(Ordering::Relaxed),
        0,
        "clear_main_callbacks must drop queued work, not run it"
    );
}
