//! The app-callback slots outside the five callback types #926 covered are
//! invoked untracked too (issue #931).
//!
//! Each slot here is one a public call runs **synchronously**, so the call can
//! be made from inside an effect: `query_selection_ranges` and
//! `fire_selection_sync` run the selection callbacks, `dispatch_dismiss` runs
//! the dismiss stack and `Drag::cancel` runs `on_cancel`. A callback's reads
//! used to be recorded on whatever effect made the call. (The child observers,
//! which every `for` reconcile reaches from inside its effect, are pinned in
//! `dom/late_child.rs`'s unit tests: they need the crate-private mock.)
//!
//! Every fixture calls from an effect created **inside another effect's run**,
//! so the call sits two frames deep on the observer stack. That is what tells
//! the fix apart from plain `untracked`, which hides only the top frame and
//! would hand the callback's reads to the outer effect (#932): the fixture
//! asserts that *neither* effect re-runs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::Signal;
use rinch_core::events::{
    Drag, SelectionAction, dispatch_dismiss, fire_selection_sync, push_dismiss_handler,
    query_selection_ranges, set_selection_callback, set_selection_sync_callback,
};
use rinch_core::reactive::Effect;

/// Run counts of the two effects [`nested`] builds.
struct Runs {
    outer: Rc<Cell<u32>>,
    inner: Rc<Cell<u32>>,
    _outer: Effect,
    _inner: Rc<RefCell<Option<Effect>>>,
}

/// An outer effect whose run creates an inner effect; the inner one reads
/// `own` (so a re-run is observable) and then calls `drive`, two frames deep.
fn nested(own: Signal<i32>, drive: impl Fn() + 'static) -> Runs {
    let drive = Rc::new(drive);
    let outer = Rc::new(Cell::new(0u32));
    let inner = Rc::new(Cell::new(0u32));
    let slot: Rc<RefCell<Option<Effect>>> = Rc::default();
    let effect = Effect::new({
        let (outer, inner, slot) = (outer.clone(), inner.clone(), slot.clone());
        move || {
            outer.set(outer.get() + 1);
            let (inner, drive) = (inner.clone(), drive.clone());
            let e = Effect::new(move || {
                own.get();
                inner.set(inner.get() + 1);
                drive();
            });
            slot.replace(Some(e));
        }
    });
    Runs {
        outer,
        inner,
        _outer: effect,
        _inner: slot,
    }
}

/// `store` is read only by the callback; writing it must re-run neither
/// effect. `own` is the positive control: the inner effect is live.
fn assert_untracked(runs: &Runs, own: Signal<i32>, store: Signal<i32>, fired: &Cell<u32>) {
    assert!(
        fired.get() >= 1,
        "control: the callback ran inside the effect"
    );
    assert_eq!((runs.outer.get(), runs.inner.get()), (1, 1));

    store.set(store.get() + 1);
    assert_eq!(
        (runs.outer.get(), runs.inner.get()),
        (1, 1),
        "a signal read only by the callback must subscribe neither the calling \
         effect nor the effect around it"
    );

    own.set(own.get() + 1);
    assert_eq!(
        runs.inner.get(),
        2,
        "positive control: the inner effect still tracks its own signal"
    );
}

/// A callback body that counts its runs and reads `store`.
fn reads(store: Signal<i32>, fired: &Rc<Cell<u32>>) -> impl Fn() + 'static {
    let fired = fired.clone();
    move || {
        fired.set(fired.get() + 1);
        store.get();
    }
}

#[test]
fn the_selection_callback_run_by_query_selection_ranges_is_untracked() {
    let (own, store) = (Signal::new(0), Signal::new(0));
    let fired = Rc::new(Cell::new(0));
    let body = reads(store, &fired);
    set_selection_callback(move |_: SelectionAction| {
        body();
        Vec::new()
    });
    let runs = nested(own, || drop(query_selection_ranges()));
    assert_untracked(&runs, own, store, &fired);
}

#[test]
fn the_selection_sync_callback_is_untracked() {
    let (own, store) = (Signal::new(0), Signal::new(0));
    let fired = Rc::new(Cell::new(0));
    let body = reads(store, &fired);
    set_selection_sync_callback(move |_| body());
    let runs = nested(own, fire_selection_sync);
    assert_untracked(&runs, own, store, &fired);
}

#[test]
fn a_dismiss_handler_run_by_dispatch_dismiss_is_untracked() {
    let (own, store) = (Signal::new(0), Signal::new(0));
    let fired = Rc::new(Cell::new(0));
    let body = reads(store, &fired);
    let _handle = push_dismiss_handler(1, move || {
        body();
        false
    });
    let runs = nested(own, || {
        dispatch_dismiss();
    });
    assert_untracked(&runs, own, store, &fired);
}

#[test]
fn on_cancel_run_by_drag_cancel_is_untracked() {
    let (own, store) = (Signal::new(0), Signal::new(0));
    let fired = Rc::new(Cell::new(0));
    let body = reads(store, &fired);
    Drag::absolute().on_cancel(move |_, _| body()).start();
    let runs = nested(own, Drag::cancel);
    assert_eq!(fired.get(), 1, "control: the effect cancelled the drag");
    assert_untracked(&runs, own, store, &fired);
}
