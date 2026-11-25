use super::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[test]
fn signal_change_subscriptions_are_independent() {
    let a = Rc::new(Cell::new(0));
    let b = Rc::new(Cell::new(0));

    let a_hits = a.clone();
    let sub_a = subscribe_signal_change(move || a_hits.set(a_hits.get() + 1));
    let b_hits = b.clone();
    let sub_b = subscribe_signal_change(move || b_hits.set(b_hits.get() + 1));

    // Both subscribers see every change.
    let sig = Signal::new(0);
    sig.set(1);
    assert_eq!(a.get(), 1, "first subscriber fires");
    assert_eq!(b.get(), 1, "second subscriber fires alongside the first");

    // Dropping one guard detaches only its own callback (#134).
    drop(sub_b);
    sig.set(2);
    assert_eq!(a.get(), 2, "surviving subscriber still fires");
    assert_eq!(b.get(), 1, "dropped subscriber no longer fires");

    drop(sub_a);
    sig.set(3);
    assert_eq!(a.get(), 2, "dropped subscriber no longer fires");
}

#[test]
fn subscription_dropped_mid_notification_does_not_fire() {
    // Subscriber A (registered first) drops B's guard from inside its own
    // callback — B must NOT fire for that same notification: the guard's
    // contract is that dropping detaches immediately.
    let b_hits = Rc::new(Cell::new(0));
    let b_guard: Rc<RefCell<Option<SignalChangeSubscription>>> = Rc::new(RefCell::new(None));

    let b_guard_for_a = b_guard.clone();
    let _sub_a = subscribe_signal_change(move || {
        *b_guard_for_a.borrow_mut() = None; // drop B mid-notification
    });
    let b_hits_clone = b_hits.clone();
    *b_guard.borrow_mut() = Some(subscribe_signal_change(move || {
        b_hits_clone.set(b_hits_clone.get() + 1);
    }));

    let sig = Signal::new(0);
    sig.set(1);
    assert_eq!(
        b_hits.get(),
        0,
        "a subscription dropped by an earlier callback in the same \
         notification must not fire"
    );
}

#[test]
fn legacy_slot_is_last_write_wins_but_spares_guard_subscribers() {
    let legacy = Rc::new(Cell::new(0));
    let guard = Rc::new(Cell::new(0));

    let guard_hits = guard.clone();
    let _sub = subscribe_signal_change(move || guard_hits.set(guard_hits.get() + 1));

    // Legacy slot: second set_on_signal_change replaces the first…
    let first = Rc::new(Cell::new(0));
    let first_hits = first.clone();
    set_on_signal_change(move || first_hits.set(first_hits.get() + 1));
    let legacy_hits = legacy.clone();
    set_on_signal_change(move || legacy_hits.set(legacy_hits.get() + 1));

    let sig = Signal::new(0);
    sig.set(1);
    assert_eq!(first.get(), 0, "evicted legacy callback does not fire");
    assert_eq!(legacy.get(), 1, "current legacy callback fires");
    assert_eq!(
        guard.get(),
        1,
        "guard-based subscriber unaffected by legacy churn"
    );

    // …and clear_on_signal_change removes only the legacy slot.
    clear_on_signal_change();
    sig.set(2);
    assert_eq!(legacy.get(), 1, "cleared legacy callback does not fire");
    assert_eq!(guard.get(), 2, "guard-based subscriber survives the clear");
}

/// Counts the notifications that reach the host — the signal-change
/// callback is what the desktop shell turns into a `ReRender` and an
/// embedded context into its dirty flag.
fn count_notifies() -> (Rc<Cell<u32>>, SignalChangeSubscription) {
    let hits = Rc::new(Cell::new(0));
    let h = hits.clone();
    let sub = subscribe_signal_change(move || h.set(h.get() + 1));
    (hits, sub)
}

/// An outermost batch that wrote nothing tells the host nothing (issue
/// #234, item 2). Every event dispatch is a batch, so before this a click
/// on a handler that wrote no signal — or a timer, or a drained main-thread
/// callback that changed nothing — posted a desktop `ReRender` and dirtied
/// every embedded context for a no-op.
///
/// The positive control is a write to a signal **nobody observes**: it
/// queues no effect and leaves the queue as empty as the no-op batch does,
/// so the only thing telling the two apart is whether a write happened.
#[test]
fn an_outermost_batch_that_writes_nothing_does_not_notify() {
    let (hits, _sub) = count_notifies();
    let sig = Signal::new(0);

    batch(|| {});
    assert_eq!(hits.get(), 0, "an empty batch does not notify");

    batch(|| batch(|| {}));
    assert_eq!(hits.get(), 0, "nor does an empty batch nested in one");

    batch(|| sig.get());
    assert_eq!(hits.get(), 0, "a read is not a write");

    batch(|| sig.set(1));
    assert_eq!(
        hits.get(),
        1,
        "a write with no observer still notifies, once, at the batch's exit"
    );

    batch(|| {
        batch(|| sig.set(2));
    });
    assert_eq!(
        hits.get(),
        2,
        "a write in a nested batch reaches the outer exit"
    );
}

/// A batch that changed host state outside the signal graph notifies like
/// one that wrote a signal (PR #1134's review, F1: `update_theme` from a
/// handler that wrote nothing stopped restyling every host). Off the
/// one-change point: two changes in one batch still notify once.
#[test]
fn a_batch_that_notes_a_host_change_notifies_once() {
    let (hits, _sub) = count_notifies();
    batch(note_host_change);
    assert_eq!(hits.get(), 1, "a host change owes the host a notification");
    batch(|| {
        note_host_change();
        batch(note_host_change);
    });
    assert_eq!(hits.get(), 2, "once per outermost batch, nested or not");
    batch(|| {});
    assert_eq!(
        hits.get(),
        2,
        "and an empty batch after it still owes nothing"
    );
}

/// A host change made outside any batch is owed to the next batch, which
/// notifies for it once, written to or not (PR #1134 round-2 review, N1).
/// Any notification pays the debt: after one, an empty batch owes nothing.
#[test]
fn a_host_change_outside_a_batch_is_owed_to_the_next_batch() {
    let (hits, _sub) = count_notifies();
    note_host_change();
    assert_eq!(
        hits.get(),
        0,
        "outside a batch it notifies nothing by itself"
    );
    batch(|| {});
    assert_eq!(hits.get(), 1, "the next batch pays it");
    batch(|| {});
    assert_eq!(hits.get(), 1, "once");

    note_host_change();
    Signal::new(0).set(1); // an unbatched write notifies, and pays it too
    assert_eq!(hits.get(), 2);
    batch(|| {});
    assert_eq!(hits.get(), 2, "nothing left owed after that notification");
}

/// A batch whose writes were already flushed mid-batch still notifies at
/// its exit: the queue is empty by then, but the writes happened and the
/// host has not been told (`flush_pending_effects` never calls the
/// signal-change callbacks). A gate on "is anything queued" alone would
/// drop this re-render.
#[test]
fn a_batch_whose_writes_flushed_mid_batch_still_notifies() {
    let (hits, _sub) = count_notifies();
    let sig = Signal::new(0);
    let seen = Rc::new(Cell::new(-1));
    let s = seen.clone();
    let _effect = Effect::new(move || s.set(sig.get()));

    batch(|| {
        sig.set(7);
        flush_pending_effects();
        assert_eq!(seen.get(), 7, "the mid-batch flush ran the effect");
        assert_eq!(hits.get(), 0, "and told the host nothing yet");
    });
    assert_eq!(hits.get(), 1, "the batch's exit notifies for the write");
}

/// An empty batch still flushes, and notifies for, effects an earlier
/// batch left queued — the documented recovery for a `batch()` whose
/// closure panicked and was caught (issue #234, item 1, decided as "keep
/// today's behaviour": no flush is scheduled on unwind, the next flush
/// runs them). Every event dispatch is a batch, so the next handler of
/// any kind is that next flush; a gate on "was anything written in *this*
/// batch" alone would strand them until a write.
#[test]
fn an_empty_batch_flushes_effects_a_panicked_batch_left_queued() {
    let sig = Signal::new(0);
    let seen = Rc::new(Cell::new(-1));
    let s = seen.clone();
    let _effect = Effect::new(move || s.set(sig.get()));

    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        batch(|| {
            sig.set(5);
            panic!("boom");
        })
    }));
    assert!(caught.is_err(), "the batch closure panicked");
    assert_eq!(seen.get(), 0, "nothing flushed on the way out");

    let (hits, _sub) = count_notifies();
    batch(|| {});
    assert_eq!(seen.get(), 5, "the next batch ran the stranded effect");
    assert_eq!(hits.get(), 1, "and told the host about the write it showed");
}
