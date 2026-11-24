//! Tests that teardown user code subscribes no observer (issue #494): the
//! reads a cleanup or a dropped value makes during disposal are untracked.

use super::*;
use crate::reactive::{Effect, Memo, Signal};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

// ------------------------------------------------------------------
// Issue #494: teardown user code must not subscribe any observer.
// ------------------------------------------------------------------

/// Reads a signal from inside `Drop` — arbitrary user code at teardown
/// time, the way a cached handle or a pooled resource might check state on
/// the way out.
#[derive(Clone)]
struct ReadsSignalOnDrop {
    probe: Signal<u32>,
}

/// `Memo<T>` needs `T: PartialEq`. Never equal, so a recompute always
/// counts as a change, and comparing reads no signal.
impl PartialEq for ReadsSignalOnDrop {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

impl Drop for ReadsSignalOnDrop {
    fn drop(&mut self) {
        let _ = self.probe.get();
    }
}

/// An `on_cleanup` that reads a signal must not subscribe the effect that
/// disposed the scope (issue #494).
///
/// Every reconcile entry point disposes outgoing scopes from inside its own
/// effect, so disposal user code runs with that effect as the current
/// observer. A tracked cleanup read would join the effect's dependency set:
/// the effect then re-runs whenever that signal changes, driven by a scope
/// that no longer exists. The final state is identical either way — the
/// spurious run re-reads its real inputs and settles — so the assertion
/// counts runs, not state.
#[test]
fn a_cleanup_that_reads_a_signal_does_not_subscribe_the_disposing_effect() {
    let probe = Signal::new(0u32);
    let trigger = Signal::new(0u32);
    let runs = Rc::new(Cell::new(0usize));

    let doomed = Scope::new();
    doomed.on_cleanup(move || {
        // The whole point: user code, reading a signal, at teardown.
        let _ = probe.get();
    });
    let doomed = Rc::new(RefCell::new(Some(doomed)));

    let hits = runs.clone();
    let _effect = Effect::new(move || {
        hits.set(hits.get() + 1);
        trigger.get();
        // `take()` on its own line, mirroring the reconcile sites (#141).
        let scope = doomed.borrow_mut().take();
        if let Some(scope) = scope {
            scope.dispose();
        }
    });
    assert_eq!(runs.get(), 1, "the effect ran once and disposed the scope");

    probe.set(1);
    assert_eq!(
        runs.get(),
        1,
        "a write to a signal only the cleanup read must not re-run the disposing effect"
    );

    // Positive control: the counter can tell a re-run apart from silence.
    trigger.set(1);
    assert_eq!(runs.get(), 2, "a tracked write still re-runs the effect");
}

/// A freed signal value's `Drop` runs user code too (fixpoint step 6), and
/// a read from it must not subscribe the disposing effect either (issue
/// #494) — the other half of the defect, distinct from `on_cleanup`.
#[test]
fn a_freed_signal_values_drop_that_reads_a_signal_does_not_subscribe_the_disposing_effect() {
    let probe = Signal::new(0u32);
    let trigger = Signal::new(0u32);
    let runs = Rc::new(Cell::new(0usize));

    let doomed = Scope::new();
    doomed.run(|| {
        // Owned by the scope: freed in step 5, its value dropped in step 6.
        Signal::new(ReadsSignalOnDrop { probe });
    });
    let doomed = Rc::new(RefCell::new(Some(doomed)));

    let hits = runs.clone();
    let _effect = Effect::new(move || {
        hits.set(hits.get() + 1);
        trigger.get();
        let scope = doomed.borrow_mut().take();
        if let Some(scope) = scope {
            scope.dispose();
        }
    });
    assert_eq!(runs.get(), 1);

    probe.set(1);
    assert_eq!(
        runs.get(),
        1,
        "a write to a signal only the freed value's Drop read must not re-run the effect"
    );

    trigger.set(1);
    assert_eq!(runs.get(), 2, "a tracked write still re-runs the effect");
}

/// Dropping a disposed effect's closure (fixpoint step 2) runs user `Drop`
/// code too — a captured value can read a signal on the way out — so the
/// guard covers the whole fixpoint, not only the cleanup and value-drop
/// steps issue #494 names.
#[test]
fn an_effect_closure_drop_that_reads_a_signal_does_not_subscribe_the_disposing_effect() {
    let probe = Signal::new(0u32);
    let trigger = Signal::new(0u32);
    let runs = Rc::new(Cell::new(0usize));

    let doomed = Scope::new();
    doomed.run(|| {
        let payload = ReadsSignalOnDrop { probe };
        // Fire-and-forget: the scope owns the effect (#141), and disposing
        // it drops the closure — and with it, `payload`.
        Effect::new(move || {
            let _keep = &payload;
        });
    });
    let doomed = Rc::new(RefCell::new(Some(doomed)));

    let hits = runs.clone();
    let _effect = Effect::new(move || {
        hits.set(hits.get() + 1);
        trigger.get();
        let scope = doomed.borrow_mut().take();
        if let Some(scope) = scope {
            scope.dispose();
        }
    });
    assert_eq!(runs.get(), 1);

    probe.set(1);
    assert_eq!(
        runs.get(),
        1,
        "a write to a signal only the dropped closure's payload read must not re-run the effect"
    );

    trigger.set(1);
    assert_eq!(runs.get(), 2, "a tracked write still re-runs the effect");
}

/// A disposal that cascades — a cleanup that disposes a further scope —
/// keeps its teardown untracked at every level (issue #494).
///
/// The inner scope's work joins the in-flight fixpoint: its cleanup and its
/// value drop run in *later* fixpoint iterations, not inside the outer
/// cleanup call, so a guard has to hold across the whole drain rather than
/// around any one call into user code.
#[test]
fn a_cascading_disposals_teardown_reads_do_not_subscribe_the_disposing_effect() {
    let probe_cleanup = Signal::new(0u32);
    let probe_drop = Signal::new(0u32);
    let trigger = Signal::new(0u32);
    let runs = Rc::new(Cell::new(0usize));

    let inner = Scope::new();
    inner.on_cleanup(move || {
        let _ = probe_cleanup.get();
    });
    inner.run(|| {
        Signal::new(ReadsSignalOnDrop { probe: probe_drop });
    });

    let outer = Scope::new();
    outer.on_cleanup(move || inner.dispose());
    let doomed = Rc::new(RefCell::new(Some(outer)));

    let hits = runs.clone();
    let _effect = Effect::new(move || {
        hits.set(hits.get() + 1);
        trigger.get();
        let scope = doomed.borrow_mut().take();
        if let Some(scope) = scope {
            scope.dispose();
        }
    });
    assert_eq!(runs.get(), 1);

    probe_cleanup.set(1);
    probe_drop.set(1);
    assert_eq!(
        runs.get(),
        1,
        "neither the cascaded cleanup's read nor the cascaded value drop's read may \
         re-run the disposing effect"
    );

    trigger.set(1);
    assert_eq!(runs.get(), 2, "a tracked write still re-runs the effect");
}

/// A teardown read subscribes no observer at ANY stack depth (issue #494).
///
/// Disposal is not a render boundary: it can begin with several observers
/// live — here a bare `Effect::new` running inside another effect, stack
/// `[outer, inner]`, where nothing has balanced the outer frame (bare
/// `Effect::new` is not one of the #491 boundaries). A one-`untracked`
/// guard — the render-side shape — would pop `inner` and hand the
/// cleanup's read to `outer`, trading one wrong subscriber for another.
/// This test fails the unfixed code (the inner effect re-runs) AND that
/// mutant (the outer effect re-runs): teardown suspends the whole stack.
#[test]
fn a_teardown_read_subscribes_no_observer_at_any_stack_depth() {
    let probe = Signal::new(0u32);
    let outer_trigger = Signal::new(0u32);
    let outer_runs = Rc::new(Cell::new(0usize));
    let inner_runs = Rc::new(Cell::new(0usize));

    let doomed = Scope::new();
    doomed.on_cleanup(move || {
        let _ = probe.get();
    });
    let doomed = Rc::new(RefCell::new(Some(doomed)));

    let outer_hits = outer_runs.clone();
    let inner_hits = inner_runs.clone();
    let _outer = Effect::new(move || {
        outer_hits.set(outer_hits.get() + 1);
        outer_trigger.get();
        // Only the first run builds the inner effect and hands it the
        // scope; a (buggy) outer re-run must not re-arm the fixture.
        if let Some(scope) = doomed.borrow_mut().take() {
            let inner_hits = inner_hits.clone();
            let scope = RefCell::new(Some(scope));
            let _inner = Effect::new(move || {
                inner_hits.set(inner_hits.get() + 1);
                if let Some(scope) = scope.borrow_mut().take() {
                    scope.dispose();
                }
            });
        }
    });
    assert_eq!(outer_runs.get(), 1);
    assert_eq!(inner_runs.get(), 1);

    probe.set(1);
    assert_eq!(
        inner_runs.get(),
        1,
        "the top observer (the inner effect) must not be subscribed by the cleanup's read"
    );
    assert_eq!(
        outer_runs.get(),
        1,
        "nor the observer beneath it (the outer effect) — one pop is not enough"
    );

    // Positive control on the outer effect.
    outer_trigger.set(1);
    assert_eq!(outer_runs.get(), 2);
}

/// `Effect::dispose` drops the closure directly — outside any scope
/// fixpoint — and a captured value's `Drop` that reads a signal must not
/// subscribe the effect that called it (issue #494). The guard for this
/// path lives in `dispose_effect` itself, which the fixpoint's step 2 also
/// calls (a second suspension of an already-empty stack is a no-op).
#[test]
fn a_direct_effect_disposes_closure_drop_does_not_subscribe_the_disposing_effect() {
    let probe = Signal::new(0u32);
    let trigger = Signal::new(0u32);
    let runs = Rc::new(Cell::new(0usize));

    let payload = ReadsSignalOnDrop { probe };
    let victim = Effect::new(move || {
        let _keep = &payload;
    });
    let victim = Rc::new(RefCell::new(Some(victim)));

    let hits = runs.clone();
    let _disposer = Effect::new(move || {
        hits.set(hits.get() + 1);
        trigger.get();
        if let Some(victim) = victim.borrow_mut().take() {
            victim.dispose();
        }
    });
    assert_eq!(runs.get(), 1);

    probe.set(1);
    assert_eq!(
        runs.get(),
        1,
        "a write to a signal only the dropped closure's payload read must not re-run the effect"
    );

    trigger.set(1);
    assert_eq!(runs.get(), 2, "a tracked write still re-runs the effect");
}

/// Dropping an unregistered handler's callback (fixpoint step 1) runs user
/// `Drop` code as well — the callback is an `Rc<dyn Fn()>` closing over
/// arbitrary user state — so step 1 is a defect site like the others
/// (issue #494). Together with the step-4 test below, this pins the two
/// steps no other #494 test reaches: a guard rebuilt per-step that left
/// either bare would pass every other test in this family and leak here.
#[test]
fn a_handler_closure_drop_that_reads_a_signal_does_not_subscribe_the_disposing_effect() {
    let probe = Signal::new(0u32);
    let trigger = Signal::new(0u32);
    let runs = Rc::new(Cell::new(0usize));

    let doomed = Scope::new();
    doomed.run(|| {
        let payload = ReadsSignalOnDrop { probe };
        // Recorded against the ambient owner (#141); disposal unregisters
        // it, dropping the callback — and with it, `payload`.
        let _id = crate::events::register_handler(Rc::new(move || {
            let _keep = &payload;
        }));
    });
    let doomed = Rc::new(RefCell::new(Some(doomed)));

    let hits = runs.clone();
    let _effect = Effect::new(move || {
        hits.set(hits.get() + 1);
        trigger.get();
        let scope = doomed.borrow_mut().take();
        if let Some(scope) = scope {
            scope.dispose();
        }
    });
    assert_eq!(runs.get(), 1);

    probe.set(1);
    assert_eq!(
        runs.get(),
        1,
        "a write to a signal only the dropped handler's payload read must not re-run the effect"
    );

    trigger.set(1);
    assert_eq!(runs.get(), 2, "a tracked write still re-runs the effect");
}

/// Freeing a memo (fixpoint step 4) drops its cached value — more user
/// `Drop` code — and a read from it must not subscribe the disposing
/// effect either (issue #494). See the step-1 test above for why these two
/// are pinned individually.
#[test]
fn a_freed_memos_cached_value_drop_that_reads_a_signal_does_not_subscribe_the_disposing_effect() {
    let probe = Signal::new(0u32);
    let trigger = Signal::new(0u32);
    let runs = Rc::new(Cell::new(0usize));

    let doomed = Scope::new();
    let memo = doomed.run(|| Memo::new(move || ReadsSignalOnDrop { probe }));
    // Materialize the cache: an unread memo holds no value to drop. The
    // clone `get` hands back drops right here — before the effect below
    // exists, with no observer live — so the only drop that happens during
    // disposal is the *cached* value's, in step 4.
    let _ = memo.get();
    let doomed = Rc::new(RefCell::new(Some(doomed)));

    let hits = runs.clone();
    let _effect = Effect::new(move || {
        hits.set(hits.get() + 1);
        trigger.get();
        let scope = doomed.borrow_mut().take();
        if let Some(scope) = scope {
            scope.dispose();
        }
    });
    assert_eq!(runs.get(), 1);

    probe.set(1);
    assert_eq!(
        runs.get(),
        1,
        "a write to a signal only the freed memo value's Drop read must not re-run the effect"
    );

    trigger.set(1);
    assert_eq!(runs.get(), 2, "a tracked write still re-runs the effect");
}
