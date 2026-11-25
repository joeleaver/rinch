//! Tests of ambient ownership: what `Scope::run` attributes, the owner stack
//! holding `Weak`, `unowned`, a disposed owner owning nothing, and `leak`.

use super::*;
use crate::reactive::{Effect, Memo, Signal};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The owner stack holds `Weak`, so dropping the last handle to the scope
/// that is *currently ambient* disposes it right then — not deferred to the
/// guard's epilogue.
///
/// This is the only test that distinguishes a `Weak` owner stack from an
/// `Rc` one. With `Rc`, the strong count reads 2 while pushed and the log
/// comes back `["after-drop", "cleanup", "after-guard"]` — teardown having
/// silently migrated out of the drop and into the guard.
#[test]
fn a_scope_dropped_while_ambient_disposes_immediately() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let scope = Scope::new();
    let cleanup_log = log.clone();
    scope.on_cleanup(move || cleanup_log.borrow_mut().push("cleanup"));

    let mut holder = Some(scope);
    {
        let _owner = holder.as_ref().unwrap().push_owner();
        assert_eq!(
            holder.as_ref().unwrap().strong_count(),
            1,
            "the owner stack must not hold a strong reference"
        );
        holder.take(); // drops the only `Scope` handle
        log.borrow_mut().push("after-drop");
    }
    log.borrow_mut().push("after-guard");

    assert_eq!(
        *log.borrow(),
        ["cleanup", "after-drop", "after-guard"],
        "disposal must happen when the handle drops, not when the guard pops"
    );
}

/// `run` attributes signals, memos and effects — and adopts none of them.
/// Adoption stays explicit via `add_effect`, because that is what `dispose`
/// acts on.
#[test]
fn run_attributes_resources_but_adopts_nothing() {
    let scope = Scope::new();

    scope.run(|| {
        Signal::new(0);
        Memo::new(|| 1);
        Effect::new(|| {});
    });

    assert_eq!(
        scope.owned_counts(),
        OwnedCounts {
            signals: 1,
            memos: 1,
            handlers: 0,
            effects: 1,
        },
        "everything created inside `run` is attributed to the scope"
    );
    assert_eq!(
        scope.effect_count(),
        0,
        "`run` must not adopt effects — only `add_effect` does"
    );

    // The guard popped: later allocations are not attributed.
    Signal::new(0);
    assert_eq!(
        scope.owned_counts().signals,
        1,
        "the guard must have popped"
    );
}

/// The non-breaking-ness contract (#141 SD2): with no ambient owner, a
/// resource has app lifetime and no scope claims it.
#[test]
fn no_ambient_owner_means_app_lifetime() {
    assert!(
        current_owner().is_none(),
        "tests start with an empty owner stack"
    );

    let orphan = Signal::new(7);

    let scope = Scope::new();
    assert_eq!(scope.owned_counts(), OwnedCounts::default());

    scope.dispose();
    assert!(orphan.is_alive(), "an unowned signal outlives any scope");
    assert_eq!(orphan.get(), 7);
}

/// `unowned` suspends the owner stack, nests, and — unlike a naive
/// pop/run/push-back — restores the owner when the body panics.
#[test]
fn unowned_suspends_the_owner_and_is_panic_safe() {
    let scope = Scope::new();

    scope.run(|| {
        let owner = current_owner().expect("scope is ambient");

        unowned(|| {
            assert!(current_owner().is_none(), "unowned clears the owner");
            Signal::new(0);
            unowned(|| assert!(current_owner().is_none(), "nesting is a no-op"));
        });
        assert_eq!(current_owner(), Some(owner.clone()), "owner is restored");

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            unowned(|| panic!("boom"));
        }));
        assert!(panicked.is_err(), "the body must have panicked");
        assert_eq!(
            current_owner(),
            Some(owner),
            "a panic inside `unowned` must not strand the suspension"
        );
    });

    assert_eq!(
        scope.owned_counts().signals,
        0,
        "the signal created under `unowned` belongs to nobody"
    );
}

/// A disposed scope owns nothing further, and ownership does **not** fall
/// through to the next entry down: the owner stack is not an ancestor chain
/// (`run_effect` splices a creation-time owner onto an unrelated
/// dispatch-time one), so falling through can cross component trees.
///
/// Reachable in production: `dispose_into_queue` marks a child disposed
/// while `pending` still holds a strong reference, so the `Weak` upgrades
/// onto a corpse.
#[test]
fn a_disposed_scope_on_the_owner_stack_owns_nothing() {
    let outer = Scope::new();
    let inner = Scope::new();

    let signal = outer.run(|| {
        let _inner_owner = inner.push_owner();
        inner.dispose(); // disposed while still ambient
        assert!(current_owner().is_none(), "a disposed owner is no owner");
        Signal::new(0)
    });

    assert_eq!(inner.owned_counts().signals, 0, "the corpse claims nothing");
    assert_eq!(
        outer.owned_counts().signals,
        0,
        "and ownership must not fall through to the next entry down"
    );
    assert!(signal.is_alive(), "the signal simply has app lifetime");
}

/// The cleanup hook `crate::context` uses to tie a store entry to its
/// creating scope registers against a **live ambient** owner only.
#[test]
fn on_cleanup_for_ambient_owner_registers_only_against_a_live_ambient_owner() {
    let fired = Rc::new(Cell::new(0));

    // No ambient owner => not registered (the resource keeps app lifetime).
    assert!(
        !on_cleanup_for_ambient_owner(|| {}),
        "an empty owner stack must not accept a cleanup"
    );

    let scope = Scope::new();
    let hits = fired.clone();
    scope.run(|| {
        assert!(
            on_cleanup_for_ambient_owner(move || hits.set(hits.get() + 1)),
            "a live ambient owner accepts the cleanup"
        );
        // `unowned` suspends the owner stack, so nothing to register against.
        unowned(|| {
            assert!(
                !on_cleanup_for_ambient_owner(|| {}),
                "`unowned` must hide the ambient owner from the cleanup hook too"
            );
        });
    });

    assert_eq!(fired.get(), 0, "cleanups do not run before disposal");
    scope.dispose();
    assert_eq!(fired.get(), 1, "the registered cleanup ran on dispose");

    // A disposed top entry counts as no owner, and is NOT walked past.
    let outer = Scope::new();
    let inner = Scope::new();
    outer.run(|| {
        let _inner = inner.push_owner();
        inner.dispose();
        assert!(
            !on_cleanup_for_ambient_owner(|| {}),
            "a disposed ambient owner must not accept a cleanup, and must not \
             fall through to the live scope beneath it"
        );
    });
}

/// `leak` searches the whole owner stack, not just its top.
///
/// A `leak()` written inside a nested branch runs with the branch's child
/// scope ambient while the entry sits in the enclosing scope's bucket. A
/// top-of-stack-only implementation leaves `outer` at 1 here — a silent
/// no-op whose PR4 failure mode is a use-after-free panic.
#[test]
fn leak_detaches_from_an_ancestor_owner_not_just_the_innermost() {
    let outer = Scope::new();
    let signal = outer.run(|| Signal::new(0));
    assert_eq!(outer.owned_counts().signals, 1);

    let inner = Scope::new();
    outer.run(|| {
        inner.run(|| {
            // Decoy. Without it the innermost bucket is empty, so an
            // implementation that removed "the first entry it found"
            // instead of matching on `(id, generation)` would fall through
            // to `outer` and pass by luck.
            Signal::new(99);
            let _ = signal.leak();
        })
    });

    assert_eq!(
        outer.owned_counts().signals,
        0,
        "leak must reach an owner further down the stack"
    );
    assert_eq!(
        inner.owned_counts().signals,
        1,
        "and must match on the key rather than raiding the nearest bucket"
    );
    assert!(signal.is_alive());
}

/// `leak` with no ambient owner is a quiet no-op, and it works for memos.
#[test]
fn leak_is_quiet_without_an_owner_and_works_for_memos() {
    // The defensive `Signal::new(x).leak()` idiom must stay noise-free:
    // 30+ in-tree constructors run outside any render.
    let orphan = Signal::new(5).leak();
    assert_eq!(orphan.get(), 5);

    let scope = Scope::new();
    let source = Signal::new(2);

    // A memo created under the scope and left alone is owned by it...
    scope.run(|| Memo::new(move || source.get() * 2));
    assert_eq!(scope.owned_counts().memos, 1);

    // ...but `leak`, called in the same render, detaches it. Calling it
    // later would be a no-op: `leak` searches the owner stack as it stands,
    // and outside a render that stack is empty.
    let doubled = scope.run(|| Memo::new(move || source.get() * 2).leak());
    assert_eq!(
        scope.owned_counts().memos,
        1,
        "the leaked memo is not added; only the first one is owned"
    );

    // Detaching must not disturb the memo itself — the marker effect is
    // still subscribed, so invalidation still propagates.
    assert_eq!(doubled.get(), 4);
    source.set(10);
    assert_eq!(doubled.get(), 20, "the dirty marker still fires");
    assert!(doubled.is_alive());
}
