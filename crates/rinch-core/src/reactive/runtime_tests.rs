use super::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// How many observers each ordering test registers.
///
/// Large enough that hash-iteration order coinciding with registration
/// order is a 1-in-40320 accident rather than a coin flip — these tests
/// have to *fail* against the old `HashSet` + LIFO pair to be worth having.
const OBSERVERS: usize = 8;

/// Effects sharing a signal run in registration order (#154).
#[test]
fn same_signal_effects_run_in_registration_order() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let sig = Signal::new(0);

    let _effects: Vec<Effect> = (0..OBSERVERS)
        .map(|i| {
            let log = log.clone();
            Effect::new(move || {
                sig.get();
                log.borrow_mut().push(i);
            })
        })
        .collect();

    // Discard the runs that `Effect::new` performs at creation.
    log.borrow_mut().clear();
    sig.set(1);

    assert_eq!(
        *log.borrow(),
        (0..OBSERVERS).collect::<Vec<_>>(),
        "effects on one signal must run in the order they were created"
    );
}

/// The same guarantee for a memo's dependents: notification flows through
/// `MemoInner::subscribers`, which is a separate set from the signal's.
#[test]
fn memo_dependents_run_in_registration_order() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let sig = Signal::new(0);
    let doubled = Memo::new(move || sig.get() * 2);

    let _effects: Vec<Effect> = (0..OBSERVERS)
        .map(|i| {
            let log = log.clone();
            Effect::new(move || {
                doubled.get();
                log.borrow_mut().push(i);
            })
        })
        .collect();

    log.borrow_mut().clear();
    sig.set(1);

    assert_eq!(
        *log.borrow(),
        (0..OBSERVERS).collect::<Vec<_>>(),
        "effects on one memo must run in the order they were created"
    );
}

/// The idiom the contract exists to protect: an effect registered *after*
/// a tree of rendering effects observes their post-write state in the same
/// synchronous flush ("run me last").
///
/// Here `dom` stands in for the patched DOM — the measuring effect must
/// read the width the rendering effect just wrote, not the previous one.
#[test]
fn an_effect_registered_last_observes_earlier_effects_writes() {
    let dom = Rc::new(RefCell::new(String::new()));
    let measured = Rc::new(RefCell::new(Vec::new()));
    let width = Signal::new(1);

    let render_dom = dom.clone();
    let _render = Effect::new(move || {
        *render_dom.borrow_mut() = "x".repeat(width.get());
    });

    let measure_dom = dom.clone();
    let measure_log = measured.clone();
    let _measure = Effect::new(move || {
        width.get();
        let len = measure_dom.borrow().len();
        measure_log.borrow_mut().push(len);
    });

    width.set(5);

    assert_eq!(
        *measured.borrow(),
        vec![1, 5],
        "the later-registered effect must observe the post-write state"
    );
}

/// A signal written *during* a flush queues its observers behind the rest
/// of that flush (FIFO), rather than jumping ahead of already-queued work.
#[test]
fn effects_queued_during_a_flush_run_after_the_already_queued_ones() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let trigger = Signal::new(0);
    let cascade = Signal::new(0);

    let writer_log = log.clone();
    let _writer = Effect::new(move || {
        writer_log.borrow_mut().push("writer");
        cascade.set(trigger.get());
    });

    let other_log = log.clone();
    let _other = Effect::new(move || {
        trigger.get();
        other_log.borrow_mut().push("other");
    });

    let cascaded_log = log.clone();
    let _cascaded = Effect::new(move || {
        cascade.get();
        cascaded_log.borrow_mut().push("cascaded");
    });

    log.borrow_mut().clear();
    trigger.set(1);

    assert_eq!(
        *log.borrow(),
        vec!["writer", "other", "cascaded"],
        "the cascade must not preempt an effect already queued for this flush"
    );
}

#[test]
fn signal_basic() {
    let signal = Signal::new(0);
    assert_eq!(signal.get(), 0);

    signal.set(5);
    assert_eq!(signal.get(), 5);

    signal.update(|n| *n += 1);
    assert_eq!(signal.get(), 6);
}

#[test]
fn effect_tracks_signals() {
    let count = Signal::new(0);
    let run_count = Rc::new(Cell::new(0));

    let run_count_clone = Rc::clone(&run_count);
    Effect::new(move || {
        let _ = count.get();
        run_count_clone.set(run_count_clone.get() + 1);
    });

    // Effect runs immediately
    assert_eq!(run_count.get(), 1);

    // Effect runs when signal changes
    count.set(1);
    assert_eq!(run_count.get(), 2);

    count.set(2);
    assert_eq!(run_count.get(), 3);
}

#[test]
fn memo_caches_value() {
    let count = Signal::new(2);
    let compute_count = Rc::new(Cell::new(0));

    let compute_count_clone = Rc::clone(&compute_count);
    let doubled = Memo::new(move || {
        compute_count_clone.set(compute_count_clone.get() + 1);
        count.get() * 2
    });

    // First access computes
    assert_eq!(doubled.get(), 4);
    assert_eq!(compute_count.get(), 1);

    // Second access uses cache
    assert_eq!(doubled.get(), 4);
    assert_eq!(compute_count.get(), 1);

    // Update signal
    count.set(3);

    // Next access recomputes
    assert_eq!(doubled.get(), 6);
    assert_eq!(compute_count.get(), 2);
}

#[test]
fn untracked_prevents_subscription() {
    let count = Signal::new(0);
    let run_count = Rc::new(Cell::new(0));

    let run_count_clone = Rc::clone(&run_count);
    Effect::new(move || {
        untracked(|| {
            let _ = count.get();
        });
        run_count_clone.set(run_count_clone.get() + 1);
    });

    // Effect runs immediately
    assert_eq!(run_count.get(), 1);

    // Effect does NOT run when signal changes (untracked)
    count.set(1);
    assert_eq!(run_count.get(), 1);
}

#[test]
fn memo_is_copy() {
    let count = Signal::new(2);
    let doubled = Memo::new(move || count.get() * 2);

    // Memo is Copy - can use in multiple closures without .clone()
    let a = doubled;
    let b = doubled;
    assert_eq!(a.get(), 4);
    assert_eq!(b.get(), 4);

    count.set(5);
    assert_eq!(a.get(), 10);
    assert_eq!(b.get(), 10);
}
