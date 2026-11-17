//! Review of #2780: a freed signal's handle must never name a later signal
//! (`docs/src/guide/signals.md`, "a freed signal is never confused with a new
//! one"). The first cut of #2778 put the thread tag in a generation's high
//! eight bits, cutting the generation counter's period to 2^24 - 1: a stale
//! handle came back to life after exactly 16 777 215 allocations on one thread
//! and read the new signal's value (a 200-row live table rebuilt at 4 Hz with
//! five signals a row wraps in about 70 minutes). The tag now lives in the id,
//! and the generation keeps its 32 bits.
//!
//! One slot is freed and reused (the free list is LIFO) more than 2^24 times.

use rinch_core::{Scope, Signal};

/// About a minute in a debug build, so ignored: run with `--ignored`. The
/// fast pin is `reactive::tests::a_signals_generation_keeps_its_32_bits`.
#[test]
#[ignore = "allocates 2^24 signals; about a minute in a debug build"]
fn a_stale_handle_stays_dead_past_two_to_the_24_allocations() {
    std::thread::spawn(|| {
        let scope = Scope::new();
        let stale: Signal<u64> = scope.run(|| Signal::new(111));
        scope.dispose();
        assert!(!stale.is_alive());
        for n in 1..=(1u64 << 24) + 16 {
            let s = Scope::new();
            let _fresh: Signal<u64> = s.run(|| Signal::new(222));
            assert!(
                !stale.is_alive(),
                "the freed handle names a live signal again after {n} allocations"
            );
            s.dispose();
        }
    })
    .join()
    .unwrap();
}
