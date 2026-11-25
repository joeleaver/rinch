use super::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[test]
fn scope_disposes_effects() {
    let count = Signal::new(0);
    let run_count = Rc::new(Cell::new(0));

    let scope = Scope::new();

    let run_count_clone = Rc::clone(&run_count);
    let effect = Effect::new(move || {
        let _ = count.get();
        run_count_clone.set(run_count_clone.get() + 1);
    });

    scope.add_effect(effect);

    // Effect runs immediately
    assert_eq!(run_count.get(), 1);

    // Effect runs when signal changes
    count.set(1);
    assert_eq!(run_count.get(), 2);

    // Dispose the scope
    scope.dispose();

    // Effect no longer runs
    count.set(2);
    assert_eq!(run_count.get(), 2);
}

#[test]
fn scope_disposes_children() {
    let count = Signal::new(0);
    let run_count = Rc::new(Cell::new(0));

    let parent = Scope::new();
    let child = Scope::new();

    let run_count_clone = Rc::clone(&run_count);
    let effect = Effect::new(move || {
        let _ = count.get();
        run_count_clone.set(run_count_clone.get() + 1);
    });

    child.add_effect(effect);
    parent.add_child(child);

    // Effect runs immediately
    assert_eq!(run_count.get(), 1);

    // Dispose parent - should also dispose child
    parent.dispose();

    // Effect no longer runs
    count.set(1);
    assert_eq!(run_count.get(), 1);
}

#[test]
fn scope_runs_cleanup() {
    let cleanup_ran = Rc::new(Cell::new(false));

    let scope = Scope::new();
    let cleanup_ran_clone = Rc::clone(&cleanup_ran);
    scope.on_cleanup(move || {
        cleanup_ran_clone.set(true);
    });

    assert!(!cleanup_ran.get());

    scope.dispose();

    assert!(cleanup_ran.get());
}

#[test]
fn scope_dispose_is_idempotent() {
    let cleanup_count = Rc::new(Cell::new(0));

    let scope = Scope::new();
    let cleanup_count_clone = Rc::clone(&cleanup_count);
    scope.on_cleanup(move || {
        cleanup_count_clone.set(cleanup_count_clone.get() + 1);
    });

    scope.dispose();
    scope.dispose();
    scope.dispose();

    // Cleanup should only run once
    assert_eq!(cleanup_count.get(), 1);
}

/// An effect re-runs under the scope that *created* it, not under whatever
/// happened to be rendering when the flush fired (issue #141).
///
/// `flush_effects` is reached from arbitrary stacks — an event handler, a
/// timer, the cross-thread drain — so without the restore in `run_effect`
/// the third run below would attribute its signal to `b`.
#[test]
fn an_effect_reruns_under_its_creation_time_owner() {
    let a = Scope::new();
    let b = Scope::new();
    let trigger = Signal::new(0);
    let seen: Rc<RefCell<Vec<Option<Owner>>>> = Rc::new(RefCell::new(Vec::new()));

    let log = seen.clone();
    let effect = a.run(|| {
        Effect::new(move || {
            trigger.get();
            Signal::new(0);
            log.borrow_mut().push(current_owner());
        })
    });
    a.add_effect(effect);

    trigger.set(1); // re-run with an empty owner stack
    b.run(|| trigger.set(2)); // re-run with an unrelated owner ambient

    let seen = seen.borrow();
    assert_eq!(seen.len(), 3, "creation run plus two re-runs");
    for (i, owner) in seen.iter().enumerate() {
        assert_eq!(
            *owner,
            Some(a.owner()),
            "run {i} must observe the creation-time owner"
        );
    }
    assert_eq!(
        b.owned_counts().signals,
        0,
        "the ambient owner at flush time must not capture the effect's work"
    );
    assert_eq!(a.owned_counts().signals, 3);
}

/// A memo's user computation runs lazily, in the *reader's* frame, so it
/// must restore its creation-time owner too — the ownership analogue of the
/// context-root bug `MemoInner::root` exists to close.
#[test]
fn a_memo_recomputes_under_its_creation_time_owner() {
    let a = Scope::new();
    let b = Scope::new();
    let source = Signal::new(1);

    let memo = a.run(|| {
        Memo::new(move || {
            Signal::new(0);
            source.get() * 2
        })
    });
    assert_eq!(
        a.owned_counts().signals,
        0,
        "a memo is lazy: nothing has computed yet"
    );

    // First read happens from inside a DIFFERENT scope.
    assert_eq!(b.run(|| memo.get()), 2);

    assert_eq!(
        b.owned_counts().signals,
        0,
        "the reading scope must not capture the memo's computation"
    );
    assert_eq!(a.owned_counts().signals, 1);
}

/// The owner guard pops while unwinding, and — critically — does not abort
/// the process doing so. A panic raised inside a `Drop` during unwind is a
/// hard abort, which would take down the whole test binary rather than fail
/// one test.
#[test]
fn an_owner_guard_pops_while_unwinding() {
    let scope = Scope::new();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scope.run(|| panic!("boom"));
    }));
    assert!(result.is_err(), "the body must have panicked");

    assert!(
        current_owner().is_none(),
        "the guard must pop while unwinding"
    );
    Signal::new(0);
    assert_eq!(
        scope.owned_counts().signals,
        0,
        "a stranded owner would capture every later allocation"
    );
}

/// Nested guards restore by depth, so an inner scope never clobbers the
/// outer one's entry.
#[test]
fn nested_owner_guards_restore_by_depth() {
    let a = Scope::new();
    let b = Scope::new();

    a.run(|| {
        let outer = current_owner().expect("a is ambient");
        b.run(|| {
            let inner = current_owner().expect("b is ambient");
            assert_ne!(inner, outer, "the inner scope is a distinct owner");
        });
        assert_eq!(current_owner(), Some(outer), "the outer owner is restored");
    });

    assert!(current_owner().is_none(), "the stack drains completely");
}
