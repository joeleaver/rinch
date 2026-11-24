use super::*;
use crate::reactive::{Scope, Signal, batch};
use std::cell::Cell;

fn is_pending(id: ObserverId) -> bool {
    RUNTIME.with(|rt| rt.borrow().pending_effects_set.contains(&id))
}

/// The issue's shape: an effect disposes a scope in its own body, and a
/// cleanup on that scope writes a signal the effect reads.
///
/// The write lands, every *other* observer of the signal runs, and the
/// effect itself keeps what it computed on this pass — the wake is not left
/// in the queue for a later flush either. It sees the value on its next run
/// for an unrelated reason.
///
/// The child scope is created on the first run only, so a variant of the
/// guard that re-runs the effect once its body has returned ("queue instead
/// of skip") fails an assertion here instead of hanging: with a new scope
/// per run — `for_each_dom`'s own shape — every re-run would dispose one
/// more and wake itself again, without end.
#[test]
fn a_cleanup_write_during_the_effects_own_body_does_not_re_run_it() {
    let trigger = Signal::new(0);
    let tally = Signal::new(0);
    let runs = Rc::new(Cell::new(0));
    let seen = Rc::new(Cell::new(-1));
    let child: Rc<RefCell<Option<Scope>>> = Rc::new(RefCell::new(None));

    let (r, s, c) = (runs.clone(), seen.clone(), child.clone());
    let effect = Effect::new(move || {
        trigger.get();
        s.set(tally.get());
        r.set(r.get() + 1);
        // Bound first: an `if let` on the `take()` would hold the `RefMut`
        // across the disposal and its cleanup.
        let old = c.borrow_mut().take();
        match old {
            // Its cleanup writes `tally`, which this effect reads.
            Some(old) => old.dispose(),
            None if r.get() == 1 => {
                let scope = Scope::new();
                scope.on_cleanup(move || tally.update(|n| *n += 1));
                *c.borrow_mut() = Some(scope);
            }
            None => {}
        }
    });

    // Positive control: registered after the effect, observing the same
    // signal. Proves the cleanup's write does wake observers — only the
    // effect that was running is left out.
    let bystander_seen = Rc::new(Cell::new(-1));
    let b = bystander_seen.clone();
    let _bystander = Effect::new(move || b.set(tally.get()));

    assert_eq!((runs.get(), seen.get()), (1, 0));
    assert!(child.borrow().is_some());

    trigger.set(1); // re-runs the effect, which disposes the scope

    assert!(child.borrow().is_none(), "the cleanup ran");
    assert_eq!(tally.get(), 1, "and its write landed");
    assert_eq!(bystander_seen.get(), 1, "another observer was woken by it");
    assert_eq!(runs.get(), 2, "the effect was not re-run for its own wake");
    assert_eq!(seen.get(), 0, "so it keeps the value it read before");
    assert!(
        !is_pending(effect.id()),
        "and the wake is not waiting in the queue"
    );

    // Nothing later picks the dropped wake up: an empty batch flushes
    // whatever is queued, and nothing is.
    batch(|| {});
    assert_eq!(runs.get(), 2);

    // The next genuine change runs it, and it reads the current value.
    trigger.set(2);
    assert_eq!((runs.get(), seen.get()), (3, 1));
}

/// The counter-case: a body running while a batch is open queues its own
/// wake instead of flushing it, and that wake is **not** dropped — the
/// effect runs again when the batch flushes.
///
/// Reached by `Effect::new` (and [`Effect::run`]) called inside a
/// [`batch`], which includes every event handler. Every re-run *caused by*
/// a signal change happens inside a flush, where the batch flag is down, so
/// it is the other case.
#[test]
fn a_self_write_during_a_first_run_inside_a_batch_is_kept() {
    let signal = Signal::new(0);
    let runs = Rc::new(Cell::new(0));
    let seen = Rc::new(Cell::new(-1));

    let (r, s) = (runs.clone(), seen.clone());
    let effect = batch(|| {
        let effect = Effect::new(move || {
            let v = signal.get();
            s.set(v);
            r.set(r.get() + 1);
            if v == 0 {
                signal.set(1);
            }
        });
        assert_eq!(runs.get(), 1, "the first run happened immediately");
        assert!(
            is_pending(effect.id()),
            "the self-write was queued, not flushed"
        );
        effect
    });

    assert_eq!(runs.get(), 2, "the batch's flush ran the queued wake");
    assert_eq!(seen.get(), 1, "and the effect saw its own write");
    assert!(!is_pending(effect.id()));
}

/// The same self-write made outside any batch is the dropped case: the
/// write flushes synchronously while the body is still running.
#[test]
fn the_same_self_write_outside_a_batch_is_dropped() {
    let signal = Signal::new(0);
    let runs = Rc::new(Cell::new(0));
    let seen = Rc::new(Cell::new(-1));

    let (r, s) = (runs.clone(), seen.clone());
    let effect = Effect::new(move || {
        let v = signal.get();
        s.set(v);
        r.set(r.get() + 1);
        if v == 0 {
            signal.set(1);
        }
    });

    assert_eq!((runs.get(), seen.get()), (1, 0));
    assert_eq!(signal.get(), 1);
    assert!(!is_pending(effect.id()));
    batch(|| {});
    assert_eq!(runs.get(), 1, "nothing picks the dropped wake up later");
}

/// [`Effect::run`] inside a batch is the same kept case as `Effect::new`
/// there: the body's self-write is queued, and the batch's flush runs the
/// effect again (issue #343).
#[test]
fn a_self_write_during_a_manual_run_inside_a_batch_is_kept() {
    let signal = Signal::new(0);
    let armed = Rc::new(Cell::new(false));
    let runs = Rc::new(Cell::new(0));

    let (a, r) = (armed.clone(), runs.clone());
    let effect = Effect::new(move || {
        let v = signal.get();
        r.set(r.get() + 1);
        if a.get() && v == 0 {
            signal.set(1);
        }
    });
    assert_eq!(runs.get(), 1);

    armed.set(true);
    batch(|| {
        effect.run();
        assert_eq!(runs.get(), 2);
    });
    assert_eq!(runs.get(), 3, "the batch's flush ran the queued self-wake");

    // Control: a re-run caused by a change runs inside a flush, with the
    // batch closed, so the same self-write there is dropped.
    signal.set(0); // runs (4), writes 1 in its body: not run again
    assert_eq!(runs.get(), 4);
    assert_eq!(signal.get(), 1);
}
