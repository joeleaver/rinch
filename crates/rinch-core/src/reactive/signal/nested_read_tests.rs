use super::*;
use crate::reactive::Effect;
use std::cell::Cell;
use std::rc::Rc;

#[test]
fn nested_with_of_different_signals_works_outside_an_effect() {
    let a = Signal::new(1i32);
    let b = Signal::new(2i32);
    let sum = a.with(|av| b.with(|bv| av + bv));
    assert_eq!(sum, 3);
}

#[test]
fn nested_with_of_different_signals_works_inside_an_effect() {
    // The exact repro from #546: outside an effect `track()` never takes
    // `borrow_mut()`, so this shape passed before the fix too. Only
    // inside an effect did `b`'s `track()` collide with `a`'s still-live
    // shared borrow. An observer must be on the stack for `track()` to
    // reach the mutable path at all, so this has to run inside `Effect`.
    let a = Signal::new(10i32);
    let b = Signal::new(20i32);
    let sum = Rc::new(Cell::new(0));
    let s = Rc::clone(&sum);

    let _e = Effect::new(move || {
        let total = a.with(|av| b.with(|bv| *av + *bv));
        s.set(total);
    });

    assert_eq!(
        sum.get(),
        30,
        "the nested read must not panic inside an effect"
    );

    // Changing either source re-runs the effect and both reads still work.
    a.set(11);
    assert_eq!(sum.get(), 31);
    b.set(21);
    assert_eq!(sum.get(), 32);
}

#[test]
fn nested_get_inside_with_works_inside_an_effect() {
    let a = Signal::new(1i32);
    let b = Signal::new(2i32);
    let sum = Rc::new(Cell::new(0));
    let s = Rc::clone(&sum);

    let _e = Effect::new(move || {
        // `b.get()` tracks exactly like `b.with()`, from inside `a`'s
        // closure — the other spelling named in the issue.
        let total = a.with(|av| *av + b.get());
        s.set(total);
    });

    assert_eq!(sum.get(), 3);
}

#[test]
fn nested_update_of_a_different_signal_works_inside_update() {
    // `update` held `borrow_mut()` across its own closure unconditionally
    // before the fix, so *any* nested signal access inside it panicked,
    // in or out of an effect. This checks it is fixed outright.
    let a = Signal::new(1i32);
    let b = Signal::new(10i32);

    a.update(|av| {
        b.update(|bv| *bv += *av);
    });

    assert_eq!(b.get(), 11);
}

#[test]
fn nested_update_of_a_different_signal_works_inside_an_effect() {
    let a = Signal::new(1i32);
    let b = Signal::new(10i32);
    let runs = Rc::new(Cell::new(0));
    let r = Rc::clone(&runs);

    let _e = Effect::new(move || {
        let _ = a.get();
        a.update(|av| {
            b.update(|bv| *bv += *av);
        });
        r.set(r.get() + 1);
    });

    assert_eq!(runs.get(), 1);
    assert_eq!(b.get(), 11);
}

#[test]
fn three_signals_nest_in_an_effect() {
    // Not just a pairwise coincidence: N independent signals must all be
    // reachable while the others' borrows are live.
    let a = Signal::new(1i32);
    let b = Signal::new(2i32);
    let c = Signal::new(3i32);
    let sum = Rc::new(Cell::new(0));
    let s = Rc::clone(&sum);

    let _e = Effect::new(move || {
        let total = a.with(|av| b.with(|bv| c.with(|cv| av + bv + cv)));
        s.set(total);
    });

    assert_eq!(sum.get(), 6);
}

#[test]
fn a_memo_computation_can_nest_reads_of_different_signals() {
    // A memo's refresh runs with the memo's marker as the current
    // observer too, so `track()` takes the mutable path there as well.
    use crate::reactive::Memo;

    let a = Signal::new(1i32);
    let b = Signal::new(2i32);
    let m = Memo::new(move || a.with(|av| b.with(|bv| av + bv)));

    assert_eq!(m.get(), 3);
    a.set(5);
    assert_eq!(m.get(), 7);
}

#[test]
fn reading_a_signal_inside_its_own_with_does_not_panic() {
    // Two SHARED borrows of one `RefCell` coexist happily — `with` holds
    // a shared borrow of `a`'s own value cell, and a nested `a.get()`
    // takes another shared borrow of that same cell. This is not the
    // reentrancy case: nothing here ever asks for `borrow_mut()`.
    let a = Signal::new(1i32);
    let seen = a.with(|v| *v + a.get());
    assert_eq!(seen, 2);
}

#[test]
#[should_panic(expected = "already borrowed")]
fn a_signal_writing_itself_inside_its_own_with_still_panics() {
    // Genuine reentrancy on ONE signal's own cell, not an artifact of a
    // shared store lock — this must still fail. A fix that makes the
    // whole class of nested access "just work" without preserving this
    // is wrong: it would mean `with` no longer borrows the value it
    // hands out at all. `with` holds a *shared* borrow of `a`'s cell
    // across the closure; `a.set()` inside it needs `borrow_mut()` on
    // that very same cell, which must fail.
    let a = Signal::new(1i32);
    a.with(|_| {
        a.set(2);
    });
}

#[test]
#[should_panic(expected = "already borrowed")]
fn a_signal_writing_itself_inside_its_own_update_still_panics() {
    let a = Signal::new(1i32);
    a.update(|_| {
        a.set(2);
    });
}
