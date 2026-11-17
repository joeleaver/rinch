//! [`on_cleanup_removable`] and [`CleanupHandle::remove`].

use super::*;
use std::cell::RefCell;

/// A log the cleanups write their names to, in the order they run.
fn log() -> Rc<RefCell<Vec<&'static str>>> {
    Rc::new(RefCell::new(Vec::new()))
}

fn push(log: &Rc<RefCell<Vec<&'static str>>>, name: &'static str) -> impl FnOnce() + 'static {
    let log = log.clone();
    move || log.borrow_mut().push(name)
}

#[test]
fn a_removed_cleanup_never_runs_and_its_closure_is_dropped_at_once() {
    let scope = Scope::new();
    let ran = log();
    let captured = Rc::new(());
    let held = captured.clone();
    let handle = scope.run(|| {
        on_cleanup_removable(move || {
            let _ = &held;
        })
    });
    let handle = handle.expect("a live owner");
    assert_eq!(Rc::strong_count(&captured), 2);
    handle.remove();
    assert_eq!(Rc::strong_count(&captured), 1, "dropped by the removal");
    scope.on_cleanup(push(&ran, "plain"));
    scope.dispose();
    assert_eq!(*ran.borrow(), ["plain"]);
}

#[test]
fn removable_cleanups_run_in_their_place_among_the_others() {
    let ran = log();
    let scope = Scope::new();
    scope.on_cleanup(push(&ran, "a"));
    let b = scope.run(|| on_cleanup_removable(push(&ran, "b"))).unwrap();
    scope.on_cleanup(push(&ran, "c"));
    let d = scope.run(|| on_cleanup_removable(push(&ran, "d"))).unwrap();
    scope.on_cleanup(push(&ran, "e"));
    drop(b); // dropping the handle keeps the cleanup
    d.remove();
    scope.dispose();
    assert_eq!(*ran.borrow(), ["a", "b", "c", "e"]);
}

#[test]
fn the_handle_keeps_nothing_alive_and_is_inert_once_the_owner_is_gone() {
    let ran = log();
    let scope = Scope::new();
    let before = scope.strong_count();
    let handle = scope.run(|| on_cleanup_removable(push(&ran, "x"))).unwrap();
    assert_eq!(scope.strong_count(), before, "a weak reference only");
    drop(scope); // dropping an undisposed scope disposes it
    assert_eq!(*ran.borrow(), ["x"]);
    handle.remove(); // nothing to remove: a no-op
    assert_eq!(*ran.borrow(), ["x"]);
}

#[test]
fn removing_after_the_owner_was_disposed_does_nothing() {
    let ran = log();
    let scope = Scope::new();
    let handle = scope.run(|| on_cleanup_removable(push(&ran, "x"))).unwrap();
    scope.dispose();
    handle.remove();
    assert_eq!(*ran.borrow(), ["x"], "it ran at the disposal; nothing more");
}

#[test]
fn a_cleanup_that_removes_a_later_one_during_the_disposal_keeps_it_from_running() {
    let ran = log();
    let scope = Scope::new();
    let later: Rc<RefCell<Option<CleanupHandle>>> = Rc::new(RefCell::new(None));
    let to_remove = later.clone();
    let first = push(&ran, "first");
    scope.on_cleanup(move || {
        first();
        if let Some(handle) = to_remove.borrow_mut().take() {
            handle.remove();
        }
    });
    *later.borrow_mut() = scope.run(|| on_cleanup_removable(push(&ran, "removed")));
    scope.on_cleanup(push(&ran, "last"));
    scope.dispose();
    assert_eq!(*ran.borrow(), ["first", "last"]);
}

#[test]
fn an_owner_that_registers_and_removes_many_keeps_none_of_them() {
    let scope = Scope::new();
    scope.on_cleanup(|| {});
    let before = scope.cleanup_count();
    for _ in 0..100 {
        let handle = scope.run(|| on_cleanup_removable(|| {})).unwrap();
        handle.remove();
    }
    assert_eq!(scope.cleanup_count(), before);
}

#[test]
fn with_no_owner_nothing_is_registered() {
    assert!(unowned(|| on_cleanup_removable(|| {})).is_none());
    let scope = Scope::new();
    scope.dispose();
    assert!(
        scope.run(|| on_cleanup_removable(|| {})).is_none(),
        "a disposed owner"
    );
}

/// Nothing of a disposal outlives it: its context, `removed` keys included,
/// goes when the outermost disposal ends.
fn no_disposal_in_flight() -> bool {
    DISPOSE_CTX.with(|ctx| ctx.borrow().is_none())
}

#[test]
fn a_stray_removal_during_another_disposal_leaves_nothing_behind() {
    // A handle whose owner is gone.
    let gone = Scope::new();
    let stray = gone.run(|| on_cleanup_removable(|| {})).unwrap();
    gone.dispose();
    drop(gone);
    // Removed from inside an unrelated scope's cleanup, mid-disposal.
    let other = Scope::new();
    let stray = RefCell::new(Some(stray));
    other.on_cleanup(move || {
        if let Some(handle) = stray.borrow_mut().take() {
            handle.remove();
        }
    });
    other.dispose();
    assert!(
        no_disposal_in_flight(),
        "the context, stray key and all, went with it"
    );
}

/// Why `remove` cannot skip the context when its owner is gone: a child
/// scope is dropped as its disposal collects it, while its cleanups still
/// wait in the batch. A sibling's removal of one of them must hold.
#[test]
fn a_removal_holds_when_its_owner_was_dropped_by_the_disposal_running_it() {
    let ran = log();
    let parent = Scope::new();
    let child = Scope::new();
    let in_child = child
        .run(|| on_cleanup_removable(push(&ran, "child")))
        .unwrap();
    parent.add_child(child); // the parent now holds the child's only handle
    let in_child = RefCell::new(Some(in_child));
    let first = push(&ran, "parent");
    // The parent's own cleanup runs first in the batch, after the child was
    // collected and dropped.
    parent.on_cleanup(move || {
        first();
        if let Some(handle) = in_child.borrow_mut().take() {
            handle.remove();
        }
    });
    parent.dispose();
    assert_eq!(
        *ran.borrow(),
        ["parent"],
        "the child's removed cleanup did not run"
    );
    assert!(no_disposal_in_flight());
}
