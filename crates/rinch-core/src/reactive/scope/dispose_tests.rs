//! Tests of disposal: what a dispose frees, in what order, and the cases
//! where the subtree is only reachable through a closure or a cleanup.

use super::*;
use crate::reactive::{Effect, Memo, Signal};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Regression anchor for PR2's riskiest single edit: moving `Drop` from
/// `Scope` to `ScopeInner`. Every other scope test disposes explicitly and
/// would survive that impl being dropped on the floor.
#[test]
fn dropping_a_bare_scope_handle_runs_its_cleanup() {
    let ran = Rc::new(Cell::new(false));
    {
        let scope = Scope::new();
        let flag = ran.clone();
        scope.on_cleanup(move || flag.set(true));
        assert!(!ran.get(), "cleanup must not run early");
    }
    assert!(ran.get(), "dropping the last handle must dispose the scope");
}

/// The headline of #141: disposing a scope releases every kind of resource
/// attributed to it, not just the effects explicitly adopted into it.
#[test]
fn dispose_frees_everything_the_scope_owns() {
    let scope = Scope::new();
    let (signal, memo, handler) = scope.run(|| {
        let s = Signal::new(1);
        let m = Memo::new(|| 2);
        Effect::new(|| {});
        let h = crate::events::register_handler(std::rc::Rc::new(|| {}));
        (s, m, h)
    });
    assert_eq!(
        scope.owned_counts(),
        OwnedCounts {
            signals: 1,
            memos: 1,
            handlers: 1,
            effects: 1,
        }
    );
    assert!(signal.is_alive());
    assert!(memo.is_alive());
    assert!(crate::events::dispatch_event(handler));

    scope.dispose();

    assert_eq!(
        scope.owned_counts(),
        OwnedCounts::default(),
        "the bucket is emptied as it is released"
    );
    assert!(!signal.is_alive(), "the signal slot is freed");
    assert!(!memo.is_alive(), "the memo slot is freed");
    assert!(
        !crate::events::dispatch_event(handler),
        "the handler is deregistered"
    );
}

/// The leak #141 was filed for: an effect created during a render but never
/// handed to [`Scope::add_effect`] used to run for the life of the thread.
///
/// Ownership catches it even though adoption did not.
#[test]
fn an_effect_nobody_adopted_still_dies_with_its_owner() {
    let trigger = Signal::new(0);
    let runs = Rc::new(Cell::new(0));

    let scope = Scope::new();
    let hits = runs.clone();
    scope.run(|| {
        // Deliberately dropped on the floor — no `add_effect`.
        Effect::new(move || {
            trigger.get();
            hits.set(hits.get() + 1);
        });
    });
    assert_eq!(runs.get(), 1, "ran once at creation");
    assert_eq!(scope.effect_count(), 0, "and was never adopted");

    trigger.set(1);
    assert_eq!(runs.get(), 2);

    scope.dispose();

    trigger.set(2);
    assert_eq!(
        runs.get(),
        2,
        "a fire-and-forget effect must not outlive the scope that created it"
    );
}

/// Why disposal has to be a **fixpoint** rather than one pass down the
/// priority list.
///
/// A scope's subtree is not enumerable up front: `child` here is reachable
/// only through the cleanup closure, so it is not marked disposed until
/// cleanups run — which is *after* the handler level. A single pass would
/// leave its handler registered forever; the fixpoint loops back to the top
/// and catches it.
#[test]
fn a_scope_reached_only_by_a_cleanup_still_has_its_handlers_freed() {
    let outer = Scope::new();

    let child = Scope::new();
    let handler = child.run(|| crate::events::register_handler(std::rc::Rc::new(|| {})));
    assert!(crate::events::dispatch_event(handler));

    // The only handle to `child` lives inside the cleanup closure, so the
    // handler cannot be discovered before cleanups run.
    let mut held = Some(child);
    outer.on_cleanup(move || drop(held.take()));

    outer.dispose();

    assert!(
        !crate::events::dispatch_event(handler),
        "the fixpoint must revisit the handler level after cleanups ran"
    );
}

/// A grandchild scope reachable *only* through an effect closure is still
/// fully released.
///
/// This is the shape every control-flow construct produces: `show_dom`,
/// `match_dom`, `for_each_dom` and `reactive_component_dom` all keep their
/// child scope inside the closure of the effect that swaps it. So the child
/// is not enumerable when the parent is collected — it only becomes
/// reachable once that effect is disposed and its closure dropped, one level
/// down the fixpoint.
///
/// Without this, `dispose_frees_everything_the_scope_owns` is trivially
/// satisfiable by a single flat pass and proves nothing about the fixpoint.
#[test]
fn a_grandchild_reachable_only_through_an_effect_closure_is_released() {
    let outer = Scope::new();

    // Two levels down, each hidden inside the closure above it.
    let child = Scope::new();
    let grandchild = Scope::new();
    let deep_signal = grandchild.run(|| Signal::new(41));
    let deep_handler = grandchild.run(|| crate::events::register_handler(Rc::new(|| {})));

    // The grandchild handle lives *inside* an effect closure owned by
    // `child`; nothing else references it. Same for `child` one level up.
    let holder = child.run(|| {
        let held = grandchild;
        Effect::new(move || {
            let _ = &held;
        })
    });
    child.add_effect(holder);

    let outer_holder = outer.run(|| {
        let held = child;
        Effect::new(move || {
            let _ = &held;
        })
    });
    outer.add_effect(outer_holder);

    assert!(deep_signal.is_alive());
    assert!(crate::events::dispatch_event(deep_handler));

    outer.dispose();

    assert!(
        !deep_signal.is_alive(),
        "the fixpoint must reach two levels of effect-closure indirection"
    );
    assert!(
        !crate::events::dispatch_event(deep_handler),
        "including levels above the one that discovered the scope"
    );
}

/// A panicking cleanup must not disable disposal for the rest of the
/// thread's life.
///
/// The fixpoint installs a thread-local context and uses "is it installed?"
/// to tell an outermost `dispose` from a nested one. Clearing it only on the
/// normal path would leave it stranded after a panic, and every later
/// `dispose()` would then collect into it and never drain — silently, and
/// for every scope in the process, not just this one.
#[test]
fn a_panicking_cleanup_does_not_strand_the_disposal_context() {
    let scope = Scope::new();
    scope.on_cleanup(|| panic!("boom"));

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scope.dispose()));
    assert!(result.is_err(), "the cleanup must have panicked");

    // The positive control: a completely unrelated scope disposed
    // afterwards still frees what it owns.
    let later = Scope::new();
    let signal = later.run(|| Signal::new(0));
    assert!(signal.is_alive());

    later.dispose();

    assert!(
        !signal.is_alive(),
        "disposal must still work after a cleanup panicked in an earlier scope"
    );
}

/// A cleanup runs after the scope's effects are disposed, so it cannot
/// resurrect one — the ordering ratified for #141 (SD4).
#[test]
fn a_cleanup_cannot_resurrect_a_disposed_effect() {
    let trigger = Signal::new(0);
    let runs = Rc::new(Cell::new(0));

    let scope = Scope::new();
    let hits = runs.clone();
    let effect = scope.run(|| {
        Effect::new(move || {
            trigger.get();
            hits.set(hits.get() + 1);
        })
    });
    scope.add_effect(effect);
    assert_eq!(runs.get(), 1);

    // Writing a signal from a cleanup flushes effects synchronously.
    scope.on_cleanup(move || trigger.set(1));
    scope.dispose();

    assert_eq!(
        runs.get(),
        1,
        "the effect was already disposed when the cleanup wrote its dependency"
    );
}

/// Cleanups run *before* the scope's signals and memos are freed, so a
/// cleanup can still read the state it is tearing down.
///
/// This is what makes "save my draft on unmount" expressible at all.
#[test]
fn a_cleanup_can_still_read_the_signals_it_is_tearing_down() {
    let saved = Rc::new(RefCell::new(String::new()));

    let scope = Scope::new();
    let draft = scope.run(|| Signal::new(String::from("unsent")));

    let sink = saved.clone();
    scope.on_cleanup(move || *sink.borrow_mut() = draft.get());

    scope.dispose();

    assert_eq!(*saved.borrow(), "unsent", "the read must not have panicked");
    assert!(!draft.is_alive(), "and the signal is freed afterwards");
}

/// A signal's value is dropped with no store borrow held, and only once
/// every sibling has been freed — so a `Drop` that touches the reactive
/// stores neither panics nor observes a half-torn-down scope.
#[test]
fn freed_values_drop_outside_the_store_borrow_and_after_their_siblings() {
    struct Probe {
        sibling: Signal<i32>,
        sibling_was_alive: Rc<Cell<bool>>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            // Touching SIGNAL_STORE here is a BorrowMutError if the value
            // is dropped in place inside `SignalStore::free`.
            self.sibling_was_alive.set(self.sibling.is_alive());
        }
    }

    let sibling_was_alive = Rc::new(Cell::new(true));
    let scope = Scope::new();
    let flag = sibling_was_alive.clone();
    scope.run(|| {
        let sibling = Signal::new(0);
        Signal::new(Probe {
            sibling,
            sibling_was_alive: flag,
        });
    });

    scope.dispose(); // must not panic

    assert!(
        !sibling_was_alive.get(),
        "value drops are deferred until every signal the scope owned is gone"
    );
}

/// Disposal is re-entrant: a cleanup may dispose another scope, and that
/// scope's own work joins the fixpoint already in flight instead of
/// starting a nested one.
#[test]
fn a_cleanup_may_dispose_another_scope() {
    let log = Rc::new(RefCell::new(Vec::new()));

    let inner = Scope::new();
    let inner_signal = inner.run(|| Signal::new(0));
    let inner_log = log.clone();
    inner.on_cleanup(move || inner_log.borrow_mut().push("inner"));

    let outer = Scope::new();
    let outer_log = log.clone();
    outer.on_cleanup(move || {
        outer_log.borrow_mut().push("outer");
        inner.dispose();
    });

    outer.dispose();

    assert_eq!(*log.borrow(), ["outer", "inner"]);
    assert!(
        !inner_signal.is_alive(),
        "the nested scope was fully released"
    );
}

/// Freeing a memo releases its dirty-marker effect too, so writing a source
/// afterwards neither resurrects its dependents nor panics.
///
/// The distinction between `free_memo` and `MemoStore::free`, pinned at the
/// level that actually calls it.
#[test]
fn disposing_a_scope_releases_a_memos_marker_effect_as_well() {
    let source = Signal::new(1);
    let scope = Scope::new();
    let memo = scope.run(|| Memo::new(move || source.get() * 2));
    assert_eq!(memo.get(), 2);

    let runs = Rc::new(Cell::new(0));
    let hits = runs.clone();
    // Lives outside the scope, so it survives the dispose below.
    let _dependent = Effect::new(move || {
        source.get();
        hits.set(hits.get() + 1);
    });
    assert_eq!(runs.get(), 1);

    scope.dispose();
    source.set(5); // must not panic

    assert!(!memo.is_alive());
    assert_eq!(runs.get(), 2, "the surviving observer still runs");
}
