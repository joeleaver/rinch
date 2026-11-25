use super::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[test]
fn batch_prevents_multiple_runs() {
    let count = Signal::new(0);
    let run_count = Rc::new(Cell::new(0));

    let run_count_clone = Rc::clone(&run_count);
    Effect::new(move || {
        let _ = count.get();
        run_count_clone.set(run_count_clone.get() + 1);
    });

    // Effect runs immediately
    assert_eq!(run_count.get(), 1);

    // Batch multiple updates
    batch(|| {
        count.set(1);
        count.set(2);
        count.set(3);
    });

    // Effect only ran once more
    assert_eq!(run_count.get(), 2);
    assert_eq!(count.get(), 3);
}

/// A panic inside the closure must not leak `batching = true` (#232).
///
/// A leaked flag makes every later write queue observers that nothing ever
/// flushes — the UI silently freezes, which is strictly worse than the
/// panic itself. Nothing flushes *during* the unwind either: the queued
/// effects stay pending and run at the next flush. The panic this test
/// provokes is deliberate — its message on stderr is expected output.
#[test]
fn a_panicking_batch_does_not_leak_the_batching_flag() {
    let count = Signal::new(0);
    let runs = Rc::new(Cell::new(0));

    let r = runs.clone();
    let _effect = Effect::new(move || {
        count.get();
        r.set(r.get() + 1);
    });
    assert_eq!(runs.get(), 1, "the effect runs once at creation");

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        batch(|| {
            count.set(1);
            panic!("boom");
        });
    }));
    assert!(result.is_err(), "the batch closure must have panicked");
    assert_eq!(runs.get(), 1, "nothing flushes while unwinding");
    assert!(
        !RUNTIME.with(|rt| rt.borrow().batching),
        "the flag must be restored by the unwind"
    );

    // The observer the aborted batch queued is not lost: the next
    // unbatched write flushes it along with its own.
    count.set(2);
    assert_eq!(runs.get(), 2, "later writes must still flush");
    assert_eq!(count.get(), 2);
}

/// Nested batches are one transaction (#232): nothing flushes until the
/// outermost batch exits — not at the inner exit, and not on a write made
/// *after* the inner batch (which is what would leak if the inner guard
/// cleared the flag instead of restoring it). At the single flush,
/// observers of the *same* signal run in registration order (#154's
/// BTreeSet path, exercised through nesting: `first` before `third`),
/// while across different signals the order is the FIFO enqueue order —
/// the order the signals were first written.
#[test]
fn nested_batches_flush_once_at_the_outermost_exit() {
    let a = Signal::new(0);
    let b = Signal::new(0);

    let log = Rc::new(RefCell::new(Vec::new()));

    let l = Rc::clone(&log);
    let _first = Effect::new(move || {
        a.get();
        l.borrow_mut().push("first");
    });
    let l = Rc::clone(&log);
    let _second = Effect::new(move || {
        b.get();
        l.borrow_mut().push("second");
    });
    let l = Rc::clone(&log);
    let _third = Effect::new(move || {
        a.get();
        l.borrow_mut().push("third");
    });
    log.borrow_mut().clear(); // discard the creation runs

    let l = Rc::clone(&log);
    batch(|| {
        a.set(1);
        batch(|| b.set(1));
        // This write is the probe for a guard that *clears* rather than
        // restores: with the outer flag gone it would flush right here.
        a.set(2);
        assert!(
            l.borrow().is_empty(),
            "the inner batch must not end the outer transaction"
        );
    });

    assert_eq!(
        *log.borrow(),
        vec!["first", "third", "second"],
        "one flush at the outermost exit: same-signal observers in \
         registration order, signals in first-write order"
    );
}

/// A panicking *inner* batch restores `batching` to `true`, not `false`:
/// the outer transaction survives a caught panic, keeps batching writes
/// made after it, and still flushes exactly once at the outermost exit.
/// The panic this test provokes is deliberate — its message on stderr is
/// expected output.
#[test]
fn a_caught_panic_in_an_inner_batch_leaves_the_outer_transaction_open() {
    let a = Signal::new(0);
    let runs = Rc::new(Cell::new(0));

    let r = runs.clone();
    let _effect = Effect::new(move || {
        a.get();
        r.set(r.get() + 1);
    });
    assert_eq!(runs.get(), 1, "the effect runs once at creation");

    batch(|| {
        a.set(1);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            batch(|| {
                a.set(2);
                panic!("inner boom");
            });
        }));
        assert!(result.is_err(), "the inner closure must have panicked");
        // The inner unwind restored `true`: this write still batches
        // instead of flushing mid-transaction.
        a.set(3);
        assert_eq!(runs.get(), 1, "the outer transaction must still be open");
    });

    assert_eq!(runs.get(), 2, "one flush at the outermost exit");
    assert_eq!(a.get(), 3);
}

/// A batch opened *during* another batch's flush — from inside an effect —
/// is a fresh outermost batch and flushes synchronously before it returns.
///
/// ColorPicker's `value_fn` echo path relies on this composition: the
/// batched writes its `ApplyGuard` spans must flush inside the guard's
/// window, even though that whole apply runs from an effect another
/// batch woke.
#[test]
fn a_batch_inside_a_flush_still_flushes_before_returning() {
    let outer_sig = Signal::new(0);
    let inner_sig = Signal::new(0);

    let inner_runs = Rc::new(Cell::new(0));
    let r = Rc::clone(&inner_runs);
    let _inner_effect = Effect::new(move || {
        inner_sig.get();
        r.set(r.get() + 1);
    });

    // What `inner_runs` read as, right after the nested batch returned.
    let seen_after_nested_batch = Rc::new(Cell::new(0));
    let seen = Rc::clone(&seen_after_nested_batch);
    let ir = Rc::clone(&inner_runs);
    let _outer_effect = Effect::new(move || {
        if outer_sig.get() == 0 {
            return; // skip the creation run
        }
        batch(|| inner_sig.set(7));
        seen.set(ir.get());
    });

    let runs_before = inner_runs.get();
    batch(|| outer_sig.set(1));
    assert_eq!(
        seen_after_nested_batch.get(),
        runs_before + 1,
        "the nested batch must have flushed inside the outer effect's body"
    );
}
