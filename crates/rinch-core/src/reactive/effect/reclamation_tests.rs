use super::*;
use crate::reactive::{Memo, Scope, Signal};
use std::cell::Cell;

/// Disposing a scope returns the effect registry to its previous size.
///
/// The last of the five leaks in issue #141: `ObserverId`s are monotonic, so
/// an id-indexed vec grew by one entry per observer ever created and never
/// shrank. Disposal emptied the slot; the slot itself stayed forever.
///
/// The loop count matters. A single cycle cannot distinguish "reclaimed"
/// from "the container happens to be this size" — the pre-fix vec is *also*
/// small after one cycle. 100 cycles makes the two answers differ by two
/// orders of magnitude.
#[test]
fn disposing_a_scope_returns_the_effect_registry_to_its_baseline() {
    // Positive control, created *outside* the loop: proves the shrinking
    // below is entries being reclaimed rather than the registry being
    // wrongly emptied. An implementation that removed too much passes a
    // bare length assertion and fails here.
    let survivor_source = Signal::new(0);
    let survivor_runs = Rc::new(Cell::new(0));
    let hits = survivor_runs.clone();
    let _survivor = Effect::new(move || {
        survivor_source.get();
        hits.set(hits.get() + 1);
    });
    assert_eq!(survivor_runs.get(), 1);

    let baseline = registry_len_for_tests();
    let baseline_ids = registry_ids_for_tests();

    for _ in 0..100 {
        let scope = Scope::new();
        scope.run(|| {
            let source = Signal::new(1);
            let doubled = Memo::new(move || source.get() * 2);
            Effect::new(move || {
                // Read the memo so its marker is genuinely subscribed, not
                // an inert entry nothing ever touches.
                let _ = doubled.get();
            });
        });
        scope.dispose();
    }

    assert_eq!(
        registry_len_for_tests(),
        baseline,
        "every disposed scope's effect and memo marker must leave the registry; \
         pre-fix this grows by ~2 per cycle"
    );
    // Assert *which* entries survived, not merely how many — a removal that
    // evicts the wrong id keeps the count right and the contents wrong.
    assert_eq!(registry_ids_for_tests(), baseline_ids);

    // And the survivor is still wired up, not merely still counted.
    survivor_source.set(1);
    assert_eq!(
        survivor_runs.get(),
        2,
        "a live effect outside the disposed scopes still runs"
    );
}

/// The two removal paths are independent and both must reclaim.
///
/// An effect leaves via `dispose_effect`; a memo's dirty marker leaves via
/// `free_memo`, which is reached from a different level of the disposal
/// fixpoint. Testing them only together lets a fix for one hide a miss on
/// the other.
#[test]
fn both_removal_paths_reclaim_independently() {
    let baseline = registry_len_for_tests();

    // Effect-only: `dispose_effect`.
    let scope = Scope::new();
    scope.run(|| {
        Effect::new(|| {});
    });
    assert_eq!(registry_len_for_tests(), baseline + 1);
    scope.dispose();
    assert_eq!(
        registry_len_for_tests(),
        baseline,
        "dispose_effect must remove the entry, not empty it"
    );

    // Memo-only: `free_memo`'s marker removal.
    let source = Signal::new(1);
    let scope = Scope::new();
    scope.run(|| {
        let doubled = Memo::new(move || source.get() * 2);
        assert_eq!(doubled.get(), 2);
    });
    assert_eq!(
        registry_len_for_tests(),
        baseline + 1,
        "a memo registers exactly one marker"
    );
    scope.dispose();
    assert_eq!(
        registry_len_for_tests(),
        baseline,
        "free_memo must remove the marker entry, not empty it"
    );
}

/// Explicit `Effect::dispose` reclaims too, not just scope teardown.
#[test]
fn explicitly_disposing_an_effect_removes_its_entry() {
    let baseline = registry_len_for_tests();
    let effect = Effect::new(|| {});
    assert_eq!(registry_len_for_tests(), baseline + 1);
    effect.dispose();
    assert_eq!(registry_len_for_tests(), baseline);
    // Idempotent: a second dispose is a no-op, not an underflow or a panic.
    effect.dispose();
    assert_eq!(registry_len_for_tests(), baseline);
}

/// Dropping an `Effect` handle deliberately does **not** reclaim.
///
/// Pinned because it is the surprising half of the contract and the reason
/// scope ownership exists: an unowned `Effect::new(..)` whose handle is
/// dropped keeps running for the life of the thread.
#[test]
fn dropping_an_effect_handle_does_not_remove_its_entry() {
    let baseline = registry_len_for_tests();
    drop(Effect::new(|| {}));
    assert_eq!(
        registry_len_for_tests(),
        baseline + 1,
        "the handle is not the owner — disposal is explicit or via a scope"
    );
}
