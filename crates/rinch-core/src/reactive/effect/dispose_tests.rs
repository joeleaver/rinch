use super::*;
use crate::reactive::Signal;
use std::cell::Cell;

/// Disposing an effect drops its closure **after** the `EFFECTS` borrow is
/// released.
///
/// The closure captures arbitrary user state. If that state's `Drop` writes
/// a signal, the write flushes effects synchronously into `run_effect`,
/// whose `EFFECTS.borrow()` conflicts with a `borrow_mut` still held by the
/// dispose — a `BorrowMutError`, not a leak. Rare before #141, routine now
/// that disposal drops every closure a scope owns.
#[test]
fn disposing_an_effect_drops_its_closure_outside_the_registry_borrow() {
    struct Noisy(Signal<i32>);
    impl Drop for Noisy {
        fn drop(&mut self) {
            // Wakes `_observer` below, re-entering `run_effect`.
            self.0.set(1);
        }
    }

    let signal = Signal::new(0);
    let woken = Rc::new(Cell::new(0));

    let hits = woken.clone();
    let _observer = Effect::new(move || {
        signal.get();
        hits.set(hits.get() + 1);
    });
    assert_eq!(woken.get(), 1);

    let noisy = Noisy(signal);
    let effect = Effect::new(move || {
        let _held = &noisy;
    });

    effect.dispose(); // must not panic

    assert_eq!(
        woken.get(),
        2,
        "the drop's write reached the surviving observer"
    );
}

/// An effect that writes a signal it observes does not re-enter itself.
///
/// `EffectInner::f` is borrowed for the whole body, and a write outside
/// `batch` flushes synchronously — so a self-observing write lands back in
/// `run_effect` one frame down its own stack. Unguarded that is a
/// `BorrowMutError`, and since #141 it has a routine trigger: an effect that
/// disposes a scope runs that scope's cleanups, and a cleanup that writes a
/// signal the effect reads arrives exactly here.
#[test]
fn an_effect_that_writes_a_signal_it_observes_does_not_re_enter_itself() {
    let signal = Signal::new(0);
    let runs = Rc::new(Cell::new(0));

    let hits = runs.clone();
    let _effect = Effect::new(move || {
        let seen = signal.get();
        hits.set(hits.get() + 1);
        if seen == 0 {
            // Flushes synchronously and re-enters this very effect.
            signal.set(1);
        }
    });

    assert_eq!(runs.get(), 1, "the re-entrant run is skipped, not panicked");
    assert_eq!(signal.get(), 1, "and the write itself still landed");

    // The effect is not wedged: a later genuine change still runs it.
    signal.set(2);
    assert_eq!(runs.get(), 2);
}
