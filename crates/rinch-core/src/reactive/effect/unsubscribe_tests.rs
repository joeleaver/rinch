use super::*;
use crate::reactive::{Memo, Scope, Signal};
use std::cell::Cell;

/// A disposed effect leaves no trace in the signal it read (issue #171).
///
/// The loop count matters for the same reason it does in the reclamation
/// tests: one cycle cannot distinguish "unsubscribed" from "the set happens
/// to be small". Pre-fix this set reaches 51.
#[test]
fn disposing_effects_leaves_no_dead_subscribers_behind() {
    let source = Signal::new(0);

    // Positive control, subscribed for the whole test: proves the removals
    // are targeted rather than the set being wrongly emptied.
    let survivor_runs = Rc::new(Cell::new(0));
    let hits = survivor_runs.clone();
    let _survivor = Effect::new(move || {
        source.get();
        hits.set(hits.get() + 1);
    });
    assert_eq!(source.subscriber_count_for_tests(), 1);

    for _ in 0..50 {
        let effect = Effect::new(move || {
            source.get();
        });
        assert_eq!(source.subscriber_count_for_tests(), 2);
        effect.dispose();
        assert_eq!(
            source.subscriber_count_for_tests(),
            1,
            "a disposed effect must not stay in the subscriber set"
        );
    }

    // And the survivor is still wired up, not merely still counted.
    source.set(1);
    assert_eq!(survivor_runs.get(), 2);
}

/// The memo half of the same contract: a memo's subscriber set is separate
/// storage, reached through a type-erased slot key.
#[test]
fn disposing_an_effect_leaves_no_dead_subscribers_in_the_memo_it_read() {
    let source = Signal::new(1);
    let doubled = Memo::new(move || source.get() * 2);

    let effect = Effect::new(move || {
        let _ = doubled.get();
    });
    assert_eq!(doubled.subscriber_count_for_tests(), 1);

    effect.dispose();
    assert_eq!(
        doubled.subscriber_count_for_tests(),
        0,
        "a disposed effect must not stay in a memo's subscriber set either"
    );
}

/// A freed memo unsubscribes from its own sources.
///
/// The memo's computation runs under its dirty-marker's id, so the marker is
/// what the sources hold — a different removal path from `dispose_effect`,
/// reached from a different level of the disposal fixpoint.
#[test]
fn freeing_a_memo_leaves_no_dead_subscribers_in_its_sources() {
    let source = Signal::new(1);
    let doubled = Memo::new(move || source.get() * 2);
    assert_eq!(doubled.get(), 2, "the computation ran and subscribed");
    assert_eq!(source.subscriber_count_for_tests(), 1);

    doubled.free_for_tests();
    assert_eq!(
        source.subscriber_count_for_tests(),
        0,
        "a freed memo's marker must leave its sources' subscriber sets"
    );
}

/// A dependency an effect stops reading loses the subscription at the
/// re-run, not merely at disposal — otherwise the dep list would hold
/// everything the effect has ever read and the leak would just move.
#[test]
fn a_dependency_an_effect_stops_reading_drops_its_subscription() {
    let which = Signal::new(true);
    let a = Signal::new(0);
    let b = Signal::new(0);
    let runs = Rc::new(Cell::new(0));

    let hits = runs.clone();
    let effect = Effect::new(move || {
        if which.get() {
            a.get()
        } else {
            b.get()
        };
        hits.set(hits.get() + 1);
    });
    assert_eq!(a.subscriber_count_for_tests(), 1);
    assert_eq!(b.subscriber_count_for_tests(), 0);

    which.set(false); // re-runs, reading `b` this time
    assert_eq!(
        a.subscriber_count_for_tests(),
        0,
        "the stale subscription must go when the effect stops reading it"
    );
    assert_eq!(b.subscriber_count_for_tests(), 1);

    // And the effect is no longer woken by what it stopped reading.
    let before = runs.get();
    a.set(1);
    assert_eq!(runs.get(), before, "a dropped dependency must not wake it");
    b.set(1);
    assert_eq!(runs.get(), before + 1, "the current one still does");

    effect.dispose();
    assert_eq!(b.subscriber_count_for_tests(), 0);
    assert_eq!(which.subscriber_count_for_tests(), 0);
}

/// Disposing one effect does not disturb another's subscription to the same
/// signal — even when both were created and disposed in the same cycle.
#[test]
fn disposing_one_effect_leaves_its_neighbours_subscribed() {
    let source = Signal::new(0);
    let first = Effect::new(move || {
        source.get();
    });
    let second = Effect::new(move || {
        source.get();
    });
    assert_eq!(source.subscriber_count_for_tests(), 2);

    first.dispose();
    assert_eq!(source.subscriber_count_for_tests(), 1);
    second.dispose();
    assert_eq!(source.subscriber_count_for_tests(), 0);
}

/// The shape issue #171 is actually about: a long-lived root signal under an
/// app that mounts and unmounts components.
#[test]
fn disposing_scopes_leaves_a_long_lived_signal_with_no_dead_subscribers() {
    let root = Signal::new(0);

    for _ in 0..50 {
        let scope = Scope::new();
        scope.run(|| {
            let derived = Memo::new(move || root.get() + 1);
            Effect::new(move || {
                // Both a direct dependency and one through a memo, so the
                // effect, the memo's marker and the memo's own subscriber
                // set are all exercised.
                root.get();
                let _ = derived.get();
            });
        });
        scope.dispose();
        assert_eq!(
            root.subscriber_count_for_tests(),
            0,
            "an unmounted component must leave nothing behind in a signal \
             that outlives it"
        );
    }
}

/// A re-entrant run is skipped, and the skip must not strip the
/// subscriptions of the run already in progress beneath it.
///
/// This is why `run_effect` takes the body borrow *before* pushing the
/// observer: the push is what releases the previous run's subscriptions, and
/// releasing them on a run that then bails out would leave an effect that
/// never wakes again.
#[test]
fn a_skipped_re_entrant_run_keeps_the_subscriptions_of_the_run_in_progress() {
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

    assert_eq!(runs.get(), 1, "the re-entrant run is skipped");
    assert_eq!(
        signal.subscriber_count_for_tests(),
        1,
        "the skipped run must not have released the live run's subscription"
    );

    signal.set(2);
    assert_eq!(runs.get(), 2, "and the effect is still woken");
}

/// A memo's dirty-marker keeps the subscriptions its *recompute* took out.
///
/// The marker runs like any other effect when a source changes, but its body
/// only flips a flag — it reads nothing. Retracking it would therefore
/// unsubscribe the memo from its own sources, and a dirty memo that nobody
/// happens to read would stay disconnected from them: the next change to a
/// source would not reach the marker, so the memo's dependents would not be
/// queued. The memo's dependencies belong to the lazy recompute in
/// `Memo::try_get`, which pushes the same id and replaces them there.
#[test]
fn a_memo_marker_keeps_its_sources_across_its_own_run() {
    let source = Signal::new(1);
    let doubled = Memo::new(move || source.get() * 2);
    assert_eq!(doubled.get(), 2);
    assert_eq!(source.subscriber_count_for_tests(), 1);

    source.set(2); // the marker runs, without anyone reading the memo
    assert_eq!(
        source.subscriber_count_for_tests(),
        1,
        "a marker's run must not release what its recompute subscribed to"
    );
    assert_eq!(doubled.get(), 4, "and the memo still recomputes correctly");
    assert_eq!(
        source.subscriber_count_for_tests(),
        1,
        "the recompute replaces the set rather than growing it"
    );
}

/// A dep whose slot was freed and handed to a new signal is not removed from
/// the new occupant.
///
/// Both halves matter: the removal is generation-filtered, and `ObserverId`s
/// are never reused, so a stale key can only ever look for an id the new
/// occupant's set does not hold.
#[test]
fn a_dep_whose_slot_was_recycled_does_not_disturb_the_new_occupant() {
    let doomed = Signal::new(0);
    let effect = Effect::new(move || {
        doomed.get();
    });
    assert_eq!(doomed.subscriber_count_for_tests(), 1);

    doomed.free_for_tests(); // the effect still holds the DepKey
    let recycled = Signal::new(0);
    assert_eq!(
        recycled.debug_id(),
        doomed.debug_id(),
        "the store must have reused the slot for this test to mean anything"
    );

    let runs = Rc::new(Cell::new(0));
    let hits = runs.clone();
    let _other = Effect::new(move || {
        recycled.get();
        hits.set(hits.get() + 1);
    });
    assert_eq!(recycled.subscriber_count_for_tests(), 1);

    effect.dispose(); // walks the stale key

    assert_eq!(
        recycled.subscriber_count_for_tests(),
        1,
        "the stale key names a generation this slot no longer has"
    );
    recycled.set(1);
    assert_eq!(runs.get(), 2, "the new occupant's subscriber still fires");
}
