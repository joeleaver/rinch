//! Invoking an app callback subscribes nobody to what the handler reads
//! (issue #285).
//!
//! A component calls its app's handlers from wherever it is — an event
//! handler, but also a coordinating effect that ends in `onchange`, or a
//! `value_fn` effect that reports. When that call is made from inside an
//! effect, every signal the handler reads used to be recorded as a dependency
//! of *the component's* effect, so the everyday controlled idiom
//! `onchange: move |v| if v != store.get() { store.set(v) }` subscribed a
//! component's internal effect to the app's store and a peer's write re-ran it
//! out of order (#282's clobbered colour).
//!
//! Each fixture here runs a "component effect" that reads a signal of its own
//! (so it is a live subscriber and a re-run is observable) and invokes an app
//! handler that reads a *different* signal, the app's store. Writing the store
//! must not re-run the effect. The handler's own write still works — it is not
//! the handler that is silenced, only the subscription.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use rinch_core::events::ScrollCallback;
use rinch_core::reactive::{Effect, Scope, on_cleanup};
use rinch_core::{Callback, FileDropCallback, InputCallback, ScrollEvent, Signal, ValueCallback};

/// Build a component effect that reads `own` and then calls `invoke`, and
/// return how many times it has run.
fn component_effect(own: Signal<i32>, invoke: impl Fn() + 'static) -> (Effect, Rc<Cell<u32>>) {
    let runs = Rc::new(Cell::new(0u32));
    let r = runs.clone();
    let effect = Effect::new(move || {
        own.get();
        r.set(r.get() + 1);
        invoke();
    });
    (effect, runs)
}

/// The shared assertion: the effect is subscribed to its own signal (positive
/// control) and not to the store the handler read.
fn assert_only_own_signal_wakes(own: Signal<i32>, store: Signal<i32>, runs: &Rc<Cell<u32>>) {
    assert_eq!(runs.get(), 1, "the effect ran once at creation");

    store.set(store.get() + 1);
    assert_eq!(
        runs.get(),
        1,
        "a write to the store the handler read must not re-run the component's effect"
    );

    // Positive control: the effect is live and does re-run on its own input,
    // so the zero above is a real "not subscribed", not a dead effect.
    own.set(own.get() + 1);
    assert_eq!(runs.get(), 2, "the effect still tracks its own signal");
}

#[test]
fn callback_invoke_does_not_subscribe_the_calling_effect() {
    let own = Signal::new(0);
    let store = Signal::new(10);
    let seen = Rc::new(Cell::new(0));
    let s = seen.clone();
    let cb = Callback::new(move || s.set(store.get()));

    let (_e, runs) = component_effect(own, move || cb.invoke());
    assert_eq!(seen.get(), 10, "the handler ran and read the store");
    assert_only_own_signal_wakes(own, store, &runs);
}

#[test]
fn value_callback_invoke_does_not_subscribe_the_calling_effect() {
    let own = Signal::new(0);
    let store = Signal::new(10);
    let cb = ValueCallback::<i32>::new(move |v| {
        // The controlled idiom: compare with the store, write only a change.
        if v != store.get() {
            store.set(v);
        }
    });

    let (_e, runs) = component_effect(own, move || cb.invoke(7));
    assert_eq!(store.get(), 7, "the handler's own write still lands");
    assert_only_own_signal_wakes(own, store, &runs);
}

#[test]
fn input_callback_invoke_does_not_subscribe_the_calling_effect() {
    let own = Signal::new(0);
    let store = Signal::new(10);
    let seen = Rc::new(Cell::new(0));
    let s = seen.clone();
    let cb = InputCallback::new(move |v: String| s.set(v.len() as i32 + store.get()));

    let (_e, runs) = component_effect(own, move || cb.invoke("abc".to_string()));
    assert_eq!(seen.get(), 13);
    assert_only_own_signal_wakes(own, store, &runs);
}

#[test]
fn file_drop_callback_invoke_does_not_subscribe_the_calling_effect() {
    let own = Signal::new(0);
    let store = Signal::new(10);
    let seen = Rc::new(Cell::new(0));
    let s = seen.clone();
    let cb = FileDropCallback::new(move |p: Vec<PathBuf>| s.set(p.len() as i32 + store.get()));

    let (_e, runs) = component_effect(own, move || cb.invoke(vec![PathBuf::from("a")]));
    assert_eq!(seen.get(), 11);
    assert_only_own_signal_wakes(own, store, &runs);
}

#[test]
fn scroll_callback_invoke_does_not_subscribe_the_calling_effect() {
    let own = Signal::new(0);
    let store = Signal::new(10);
    let seen = Rc::new(Cell::new(0.0));
    let s = seen.clone();
    let cb = ScrollCallback::new(move |e: ScrollEvent| s.set(e.scroll_top + store.get() as f64));

    let (_e, runs) = component_effect(own, move || cb.invoke(ScrollEvent::new(2.0, 0.0)));
    assert_eq!(seen.get(), 12.0);
    assert_only_own_signal_wakes(own, store, &runs);
}

/// The guarantee holds at any nesting depth, not just one level down.
///
/// `reactive::untracked` pops exactly **one** observer, so a read inside it
/// subscribes the next observer down the stack. A component effect created
/// inside another running effect (depth 2) whose invocation were only
/// `untracked` would therefore subscribe the *outer* effect to the store — the
/// same bug one frame further out. The outer effect here is the one that must
/// stay quiet.
#[test]
fn a_callback_invoked_two_effects_deep_subscribes_neither() {
    let outer_own = Signal::new(0);
    let inner_own = Signal::new(0);
    let store = Signal::new(10);
    let seen = Rc::new(Cell::new(0));

    let outer_runs = Rc::new(Cell::new(0u32));
    let inner_runs = Rc::new(Cell::new(0u32));
    let inner_slot: Rc<RefCell<Option<Effect>>> = Rc::default();

    let (o, i, s, slot) = (
        outer_runs.clone(),
        inner_runs.clone(),
        seen.clone(),
        inner_slot.clone(),
    );
    let _outer = Effect::new(move || {
        outer_own.get();
        o.set(o.get() + 1);
        let s = s.clone();
        let cb = Callback::new(move || s.set(store.get()));
        let i = i.clone();
        // Created while the outer effect is running: the stack is
        // [outer, inner] while the inner body runs.
        let inner = Effect::new(move || {
            inner_own.get();
            i.set(i.get() + 1);
            cb.invoke();
        });
        *slot.borrow_mut() = Some(inner);
    });

    assert_eq!(seen.get(), 10, "the handler ran");
    assert_eq!((outer_runs.get(), inner_runs.get()), (1, 1));

    store.set(11);
    assert_eq!(
        (outer_runs.get(), inner_runs.get()),
        (1, 1),
        "neither the outer nor the inner effect may be subscribed to the store"
    );

    // Positive controls: both are live.
    inner_own.set(1);
    assert_eq!(
        inner_runs.get(),
        2,
        "the inner effect tracks its own signal"
    );
    outer_own.set(1);
    assert_eq!(
        outer_runs.get(),
        2,
        "the outer effect tracks its own signal"
    );
}

/// Untracking is all it does: a handler's write inside an event batch still
/// flushes once, at the end of the batch, exactly as before (CLAUDE.md:
/// "Event handlers are `batch()`es").
#[test]
fn an_untracked_invoke_leaves_batch_flushing_alone() {
    let store = Signal::new(0);
    let runs = Rc::new(Cell::new(0u32));
    let r = runs.clone();
    let _watcher = Effect::new(move || {
        store.get();
        r.set(r.get() + 1);
    });
    let cb = Callback::new(move || {
        store.set(1);
        store.set(2);
    });

    rinch_core::batch(|| {
        cb.invoke();
        assert_eq!(runs.get(), 1, "no flush inside the batch");
    });
    assert_eq!(runs.get(), 2, "one flush at the batch's end");
    assert_eq!(store.get(), 2);

    // Outside any batch, each write flushes on its own, as a bare call did.
    cb.invoke();
    assert_eq!(runs.get(), 4);
}

/// Only the handler's own reads are hidden: an effect the handler *creates*
/// pushes its own observer and tracks normally.
#[test]
fn an_effect_created_by_a_handler_still_tracks() {
    let own = Signal::new(0);
    let store = Signal::new(0);
    let child_runs = Rc::new(Cell::new(0u32));
    let slot: Rc<RefCell<Option<Effect>>> = Rc::default();

    let (c, sl) = (child_runs.clone(), slot.clone());
    let cb = Callback::new(move || {
        if sl.borrow().is_some() {
            return;
        }
        let c = c.clone();
        let child = Effect::new(move || {
            store.get();
            c.set(c.get() + 1);
        });
        *sl.borrow_mut() = Some(child);
    });

    let (_e, runs) = component_effect(own, move || cb.invoke());
    assert_eq!(child_runs.get(), 1);

    store.set(1);
    assert_eq!(
        child_runs.get(),
        2,
        "the handler-created effect tracks the store"
    );
    assert_eq!(runs.get(), 1, "the calling effect does not");
}

/// Only tracking is suspended, not ownership: a signal created by a handler
/// invoked from an effect inside a scope belongs to that scope, exactly like one
/// the effect creates itself, and the scope's disposal frees both. (Found by the
/// review of #926: wrapping the handler in `unowned` passes every fixture above.)
#[test]
fn a_handler_invoked_from_an_effect_keeps_the_ambient_owner() {
    let scope = Scope::new();
    let made: Rc<RefCell<Vec<Signal<i32>>>> = Rc::default();
    let m = made.clone();
    let cb = Callback::new(move || m.borrow_mut().push(Signal::new(1)));
    let m2 = made.clone();
    scope.run(|| {
        let _e = Effect::new(move || {
            cb.invoke();
            m2.borrow_mut().push(Signal::new(2));
        });
    });
    assert_eq!(made.borrow().len(), 2);
    assert!(made.borrow().iter().all(|s| s.is_alive()));

    scope.dispose();
    let alive: Vec<bool> = made.borrow().iter().map(|s| s.is_alive()).collect();
    assert_eq!(
        alive,
        vec![false, false],
        "the handler's signal and the effect's are both freed by the scope"
    );
}

/// An `on_cleanup` a handler registers belongs to the ambient scope too, and
/// runs when that scope is disposed — as it would for a bare call.
#[test]
fn an_on_cleanup_registered_by_a_handler_runs_at_scope_disposal() {
    let scope = Scope::new();
    let ran = Rc::new(Cell::new(0));
    let r = ran.clone();
    let cb = Callback::new(move || {
        let r = r.clone();
        on_cleanup(move || r.set(r.get() + 1));
    });
    scope.run(|| {
        let _e = Effect::new(move || cb.invoke());
    });
    assert_eq!(ran.get(), 0);

    scope.dispose();
    assert_eq!(ran.get(), 1, "the handler's cleanup ran with its scope");
}

/// A handler that panics, caught inside the calling effect, leaves the
/// observer stack as it found it: the effect's reads after the panic still
/// subscribe it. (Kills a suspension that forgets to restore on unwind.)
#[test]
fn a_caught_handler_panic_restores_the_observer_stack() {
    let own = Signal::new(0);
    let later = Signal::new(0);
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    let cb = Callback::new(|| panic!("boom"));
    let _e = Effect::new(move || {
        own.get();
        r.set(r.get() + 1);
        let cb = cb.clone();
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cb.invoke()));
        assert!(res.is_err());
        later.get(); // read AFTER the panic: must still track
    });
    assert_eq!(runs.get(), 1);

    later.set(1);
    assert_eq!(
        runs.get(),
        2,
        "a read after a caught handler panic still subscribes the effect"
    );
}
