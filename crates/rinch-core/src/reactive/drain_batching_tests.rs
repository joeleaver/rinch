use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

/// Each drained main-thread callback is its own transaction, **not** one
/// for the whole drain: callbacks are queued independently, and a later one
/// may rely on an earlier one's effects having run. Here the second reads a
/// signal an effect derives from the first one's write. With the drain as a
/// single batch it read the pre-drain value.
///
/// The only test in this crate that drains the process-global queue, so no
/// other test's callbacks can land on this thread.
#[test]
fn each_drained_callback_sees_the_effects_of_the_ones_before_it() {
    let source = Signal::new(0i32);
    let derived = Signal::new(0i32);
    let _derive = Effect::new(move || derived.set(source.get() * 10));

    let seen = Arc::new(AtomicI32::new(i32::MIN));
    let s = Arc::clone(&seen);
    queue_main_callback(Box::new(move || source.set(5)));
    queue_main_callback(Box::new(move || s.store(derived.get(), Ordering::SeqCst)));
    drain_main_callbacks();

    assert_eq!(seen.load(Ordering::SeqCst), 50);
}
