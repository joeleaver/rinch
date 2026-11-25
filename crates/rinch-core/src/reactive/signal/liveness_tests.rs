use super::*;
use crate::reactive::Effect;
use std::cell::Cell;
use std::rc::Rc;

// ---- liveness queries ---------------------------------------------------

#[test]
fn is_alive_flips_when_the_slot_is_freed() {
    let s = Signal::new(1);
    assert!(s.is_alive());
    s.free_for_tests();
    assert!(!s.is_alive());
}

#[test]
fn try_get_and_try_with_return_none_after_free() {
    let s = Signal::new(7);
    assert_eq!(s.try_get(), Some(7));
    assert_eq!(s.try_with(|v| *v + 1), Some(8));

    s.free_for_tests();
    assert_eq!(s.try_get(), None);
    assert_eq!(s.try_with(|v| *v + 1), None);
}

#[test]
fn try_with_does_not_call_its_closure_on_a_freed_signal() {
    let calls = Rc::new(Cell::new(0));
    let s = Signal::new(0);
    s.free_for_tests();

    let c = Rc::clone(&calls);
    let out: Option<()> = s.try_with(move |_| c.set(c.get() + 1));

    assert!(out.is_none());
    assert_eq!(calls.get(), 0, "closure must not run on a freed signal");
}

#[test]
fn try_get_subscribes_the_current_observer_just_like_get() {
    let s = Signal::new(0);
    let runs = Rc::new(Cell::new(0));

    let r = Rc::clone(&runs);
    let _e = Effect::new(move || {
        let _ = s.try_get();
        r.set(r.get() + 1);
    });
    assert_eq!(runs.get(), 1);

    s.set(1);
    assert_eq!(runs.get(), 2, "try_get must track like get");
}

#[test]
#[should_panic(expected = "Signal::get() on a freed signal")]
fn get_panics_after_free() {
    let s = Signal::new(1);
    s.free_for_tests();
    let _ = s.get();
}

#[test]
#[should_panic(expected = "Signal::with() on a freed signal")]
fn with_panics_after_free() {
    let s = Signal::new(1);
    s.free_for_tests();
    s.with(|v| *v);
}

// ---- lenient writes -----------------------------------------------------

#[test]
fn every_write_to_a_freed_signal_is_a_no_op_rather_than_a_panic() {
    let s = Signal::new(1i32);
    s.free_for_tests();

    // None of these may panic — a detached worker holding a Copy handle
    // cannot know the signal is gone (issue #141, SD1).
    s.set(2);
    s.set_if_changed(3);
    s.update(|v| *v += 1);

    assert!(!s.is_alive());
}

#[test]
fn update_does_not_run_its_closure_on_a_freed_signal() {
    let ran = Rc::new(Cell::new(false));
    let s = Signal::new(0);
    s.free_for_tests();

    let r = Rc::clone(&ran);
    s.update(move |v| {
        *v += 1;
        r.set(true);
    });

    assert!(
        !ran.get(),
        "the closure — and any side effect in it — is dropped with the write"
    );
}

#[test]
fn a_write_to_a_freed_signal_does_not_notify_its_own_observers() {
    // The observer must subscribe to `doomed` *before* it is freed —
    // otherwise the test proves nothing, since an effect that never read
    // the signal would not re-run either way.
    let doomed = Signal::new(0);
    let runs = Rc::new(Cell::new(0));

    let r = Rc::clone(&runs);
    let _e = Effect::new(move || {
        // try_get, not get: after the free this effect must be able to run
        // without panicking if anything else queues it.
        let _ = doomed.try_get();
        r.set(r.get() + 1);
    });
    assert_eq!(runs.get(), 1);

    // Positive control: while alive, a write DOES re-run the observer.
    doomed.set(1);
    assert_eq!(runs.get(), 2, "control — the effect really observes it");

    doomed.free_for_tests();
    doomed.set(99);

    assert_eq!(runs.get(), 2, "a dropped write must not flush effects");
}

#[test]
fn a_deferred_cross_thread_write_reports_the_send_call_site() {
    // `send`/`update_send` perform the write inside a closure that runs
    // later, where #[track_caller] no longer reaches the caller. If the
    // location is not captured up front, every cross-thread freed write
    // dedups to one line inside `signal/send.rs` and all but the first go silent.
    let before = warn_count_for_tests();
    let a = Signal::new(0i32);
    let b = Signal::new(0i32);
    a.free_for_tests();
    b.free_for_tests();

    a.send(1);
    b.send(2);

    assert_eq!(
        warn_count_for_tests() - before,
        2,
        "two distinct send sites must produce two warnings"
    );

    let c = Signal::new(0i32);
    c.free_for_tests();
    c.update_send(|v| *v += 1);
    assert_eq!(
        warn_count_for_tests() - before,
        3,
        "update_send is attributed to its own call site too"
    );
}

#[test]
fn a_freed_write_warns_once_per_call_site() {
    let before = warn_count_for_tests();
    let s = Signal::new(0);
    s.free_for_tests();

    // One call site, hammered — exactly one warning.
    for _ in 0..50 {
        s.set(1);
    }
    assert_eq!(
        warn_count_for_tests() - before,
        1,
        "a hot loop must not spam the log"
    );

    // A *second*, distinct call site is still reported.
    s.set(2);
    assert_eq!(
        warn_count_for_tests() - before,
        2,
        "dedup is per call site, not global"
    );
}

// ---- values are dropped outside the store borrow ------------------------

/// A value whose `Drop` writes to another signal — the shape that made
/// dropping a displaced value *inside* `SIGNAL_STORE.borrow_mut()` a
/// `BorrowMutError`.
struct TouchesASignalOnDrop(Signal<i32>);

impl Drop for TouchesASignalOnDrop {
    fn drop(&mut self) {
        self.0.update(|n| *n += 1);
    }
}

#[test]
fn replacing_a_value_whose_drop_touches_a_signal_does_not_reenter_the_store() {
    let drops = Signal::new(0);
    let holder = Signal::new(TouchesASignalOnDrop(drops));

    // `set` displaces the old value; it must be dropped after the store
    // borrow is released, or this is a BorrowMutError.
    holder.set(TouchesASignalOnDrop(drops));
    assert_eq!(drops.get(), 1);

    holder.set(TouchesASignalOnDrop(drops));
    assert_eq!(drops.get(), 2);

    // Free explicitly rather than leaving the value for the thread-local
    // store's destructor: its `Drop` touches `SIGNAL_STORE`, which is
    // unreachable once TLS teardown has begun.
    holder.free_for_tests();
}

#[test]
fn discarding_a_write_to_a_freed_signal_drops_the_value_outside_the_store() {
    let drops = Signal::new(0);
    let holder = Signal::new(TouchesASignalOnDrop(drops));

    // Freeing drops the stored value, itself outside the borrow.
    holder.free_for_tests();
    let after_free = drops.get();
    assert_eq!(after_free, 1);

    // The incoming value now has nowhere to go; dropping it must also
    // happen outside the borrow.
    holder.set(TouchesASignalOnDrop(drops));
    assert_eq!(drops.get(), after_free + 1);
}
