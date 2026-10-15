//! Review of #1542: an unregistered worker's send is queued process-wide. If no
//! host ever drains it on the owning thread (a unit test, a CLI), whichever
//! thread next calls `drain_main_callbacks` applies it to ITS OWN store, where
//! signal ids alias (every thread's first signal is (0, 1)).
use rinch_core::{Signal, drain_main_callbacks};

#[test]
fn a_stray_pre_host_send_lands_in_another_threads_signal() {
    // "Test A": owns a String signal, a worker writes it, nobody drains.
    std::thread::spawn(|| {
        let s: Signal<String> = Signal::new(String::new());
        std::thread::spawn(move || s.send("from A's worker".into()))
            .join()
            .unwrap();
    })
    .join()
    .unwrap();

    // "Test B": a different thread, its own first signal, then a drain
    // (what `RinchContext::update` or a host-less test helper does).
    let r = std::thread::spawn(|| {
        let n: Signal<u64> = Signal::new(42);
        drain_main_callbacks();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| n.get()))
    })
    .join()
    .unwrap();
    assert_eq!(
        r.ok(),
        Some(42),
        "B's signal was overwritten by A's stray send"
    );
}
