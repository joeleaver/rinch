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

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use rinch_core::events::ScrollCallback;
use rinch_core::reactive::Effect;
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
    let inner_slot: Rc<std::cell::RefCell<Option<Effect>>> = Rc::default();

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
    assert_eq!(inner_runs.get(), 2, "the inner effect tracks its own signal");
    outer_own.set(1);
    assert_eq!(outer_runs.get(), 2, "the outer effect tracks its own signal");
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
