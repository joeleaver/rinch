//! A worker's `Signal::update_send` made before the host has called
//! `register_main_thread` (a worker spawned in `main()` ahead of
//! `App::run()`) must reach the signal once the host starts (issue #1541). Found while
//! probing beardictate's report: with no main thread registered,
//! `is_main_thread()` answers `true` on the worker, so `update_send` ran the
//! write right there, against the worker's own (empty) thread-local signal
//! store, where the signal does not exist — `warn_write_to_freed`, and the
//! write is gone. `rinch::run_on_main_thread` always queues, so the same
//! write made through it survives.
//!
//! Own binary: `MAIN_THREAD_ID` is a process-wide `OnceLock`.

use std::collections::HashMap;

use rinch_core::{
    Signal, drain_main_callbacks, queue_main_callback, register_main_thread,
    set_cross_thread_dispatcher,
};

fn host_dispatcher(f: Box<dyn FnOnce() + Send>) {
    queue_main_callback(f);
}

#[test]
fn an_update_send_from_a_worker_before_the_host_registers_is_not_lost() {
    let states: Signal<HashMap<String, u64>> = Signal::new(HashMap::new());
    let scalar: Signal<u64> = Signal::new(0);

    // The owning thread, unregistered (a test, or `rinch-web`, which registers
    // no main thread): its own send applies at once, as it always has.
    scalar.send(1);
    assert_eq!(
        scalar.get(),
        1,
        "a send on the owning thread applies at once"
    );

    // The worker runs to completion before the host starts.
    std::thread::spawn(move || {
        states.update_send(|s| {
            s.insert("model".into(), 7);
        });
        scalar.send(3);
    })
    .join()
    .expect("worker must not panic");

    // The host starts (what `App::run` / `RinchContext::new` do), then drains.
    register_main_thread();
    set_cross_thread_dispatcher(host_dispatcher);
    drain_main_callbacks();

    assert_eq!(scalar.get(), 3, "Signal::send before registration was lost");
    assert_eq!(
        states.with(|s| s.get("model").copied()),
        Some(7),
        "Signal::update_send before registration was lost"
    );
}
