//! The memo equality cut-off, and the eager staleness that keeps a memo read
//! current inside a batch.
//!
//! Every fixture samples off the fixed points where a missing cut-off and a
//! working one agree: more than one row (with one row, "every row re-ran" and
//! "the rows that flipped re-ran" are the same number), writes that really
//! change the source, and a selection that moves between two rows that are
//! neither first nor last.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::{Effect, Memo, Signal, batch, reactive_counters};

/// How many rows the selection fixtures build.
const ROWS: usize = 40;

/// One row of the classic selection list: a per-row `is selected` memo and an
/// effect that reads it, counting its own runs.
struct Row {
    runs: Rc<Cell<u32>>,
    _effect: Effect,
}

fn selection_rows(selected: Signal<usize>) -> Vec<Row> {
    (0..ROWS)
        .map(|id| {
            let is_selected = Memo::new(move || selected.get() == id);
            let runs = Rc::new(Cell::new(0));
            let r = Rc::clone(&runs);
            let effect = Effect::new(move || {
                let _ = is_selected.get();
                r.set(r.get() + 1);
            });
            Row {
                runs,
                _effect: effect,
            }
        })
        .collect()
}

fn rows_that_ran(rows: &[Row]) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| row.runs.get() > 0)
        .map(|(i, _)| i)
        .collect()
}

fn reset(rows: &[Row]) {
    for row in rows {
        row.runs.set(0);
    }
}

/// The per-row "is selected" pattern: moving the selection re-runs the row
/// that lost it and the row that gained it — two effects, not `ROWS`.
#[test]
fn moving_the_selection_reruns_only_the_two_rows_that_flip() {
    let selected = Signal::new(7usize);
    let rows = selection_rows(selected);
    reset(&rows);

    let before = reactive_counters().effect_runs;
    selected.set(23);
    assert_eq!(rows_that_ran(&rows), vec![7, 23]);
    assert_eq!(
        reactive_counters().effect_runs - before,
        2,
        "effect_runs counts the two row effects and nothing else"
    );

    // And again, from a different pair, so the fixture does not hinge on the
    // first selection being special.
    reset(&rows);
    selected.set(31);
    assert_eq!(rows_that_ran(&rows), vec![23, 31]);
}

/// Selecting a row that does not exist flips exactly one memo.
#[test]
fn deselecting_reruns_only_the_row_that_was_selected() {
    let selected = Signal::new(12usize);
    let rows = selection_rows(selected);
    reset(&rows);

    selected.set(usize::MAX);
    assert_eq!(rows_that_ran(&rows), vec![12]);
}

/// A write that changes the signal but not the memo's answer wakes nobody —
/// while a *direct* reader of the same signal still runs, because a signal has
/// no cut-off: it is the memo's equality that is being tested, not the write's.
#[test]
fn an_unchanged_memo_does_not_wake_its_dependents_but_a_direct_reader_still_runs() {
    let n = Signal::new(2i32);
    let parity = Memo::new(move || n.get() % 2);
    let through_memo = Rc::new(Cell::new(0));
    let direct = Rc::new(Cell::new(0));

    let t = Rc::clone(&through_memo);
    let _via_memo = Effect::new(move || {
        let _ = parity.get();
        t.set(t.get() + 1);
    });
    let d = Rc::clone(&direct);
    let _direct = Effect::new(move || {
        let _ = n.get();
        d.set(d.get() + 1);
    });

    n.set(4); // parity 0 -> 0
    n.set(8); // parity 0 -> 0
    assert_eq!(through_memo.get(), 1, "only the first run");
    assert_eq!(direct.get(), 3);

    n.set(9); // parity 0 -> 1
    assert_eq!(through_memo.get(), 2);
    assert_eq!(parity.get(), 1);
}

/// A memo that reads a memo is only re-validated when the inner one did not
/// move: its own computation does not run, and neither do its dependents.
#[test]
fn a_chain_stops_at_the_first_memo_whose_value_held() {
    let n = Signal::new(3i32);
    let parity = Memo::new(move || n.get() % 2);
    let label_computes = Rc::new(Cell::new(0));
    let lc = Rc::clone(&label_computes);
    let label = Memo::new(move || {
        lc.set(lc.get() + 1);
        if parity.get() == 0 { "even" } else { "odd" }
    });
    let runs = Rc::new(Cell::new(0));
    let r = Rc::clone(&runs);
    let _effect = Effect::new(move || {
        let _ = label.get();
        r.set(r.get() + 1);
    });
    assert_eq!((label_computes.get(), runs.get()), (1, 1));

    n.set(5); // parity 1 -> 1: `label` is re-validated, not recomputed
    assert_eq!((label_computes.get(), runs.get()), (1, 1));

    n.set(6); // parity 1 -> 0: both move
    assert_eq!((label_computes.get(), runs.get()), (2, 2));
    assert_eq!(label.get(), "even");
}

/// A memo that recomputes to an equal value in a chain *does* recompute the
/// next one when that one also reads a source that moved, and the effect sees
/// its new value.
#[test]
fn a_memo_with_a_moving_second_source_still_recomputes() {
    let n = Signal::new(3i32);
    let k = Signal::new(10i32);
    let parity = Memo::new(move || n.get() % 2);
    let sum = Memo::new(move || parity.get() + k.get());
    let seen = Rc::new(Cell::new(0));
    let s = Rc::clone(&seen);
    let _effect = Effect::new(move || s.set(sum.get()));
    assert_eq!(seen.get(), 11);

    batch(|| {
        n.set(5); // parity holds
        k.set(20); // but `sum` reads `k` directly
    });
    assert_eq!(seen.get(), 21);
}

/// Inside a batch — or an event handler, which is one — a memo read made after
/// a write to one of its sources returns the new value, although no effect has
/// run yet. Two levels deep, so the `Check` state is exercised as well as
/// `Dirty`.
#[test]
fn a_memo_read_inside_a_batch_sees_the_write_before_it() {
    let n = Signal::new(1i32);
    let doubled = Memo::new(move || n.get() * 2);
    let quadrupled = Memo::new(move || doubled.get() * 2);
    let effect_saw = Rc::new(Cell::new(0));
    let e = Rc::clone(&effect_saw);
    let _effect = Effect::new(move || e.set(quadrupled.get()));

    let read_in_batch = batch(|| {
        n.set(5);
        let first = (doubled.get(), quadrupled.get(), effect_saw.get());
        n.set(6);
        (first, (doubled.get(), quadrupled.get()))
    });
    assert_eq!(
        read_in_batch,
        ((10, 20, 4), (12, 24)),
        "fresh memo values inside the batch; the effect has not run yet"
    );
    assert_eq!(effect_saw.get(), 24, "…and runs once, after it");
}

/// The #154 ordering contract survives the cut-off: of the dependents of one
/// memo, the ones that run still run in registration order, and the skipped
/// ones do not disturb it. Here every row reads the shared `selected` memo
/// family through a second memo, and a selection move lands on rows that are
/// registered *out* of id order relative to the write.
#[test]
fn dependents_that_do_run_keep_registration_order() {
    let selected = Signal::new(5usize);
    let log = Rc::new(RefCell::new(Vec::new()));
    let _effects: Vec<Effect> = (0..ROWS)
        .map(|id| {
            let is_selected = Memo::new(move || selected.get() == id);
            let log = Rc::clone(&log);
            Effect::new(move || {
                let _ = is_selected.get();
                log.borrow_mut().push(id);
            })
        })
        .collect();

    log.borrow_mut().clear();
    selected.set(2); // 5 loses, 2 gains: 2 was registered first
    assert_eq!(*log.borrow(), vec![2, 5]);

    log.borrow_mut().clear();
    selected.set(37);
    assert_eq!(*log.borrow(), vec![2, 37]);
}

/// A memo nobody reads is not recomputed by the flush — the cut-off pulls only
/// what a woken observer asks for.
#[test]
fn an_unread_memo_is_not_recomputed_by_a_write() {
    let n = Signal::new(1i32);
    let computes = Rc::new(Cell::new(0));
    let c = Rc::clone(&computes);
    let m = Memo::new(move || {
        c.set(c.get() + 1);
        n.get() + 1
    });
    assert_eq!(m.get(), 2);
    n.set(2);
    n.set(3);
    assert_eq!(computes.get(), 1, "stale, but not recomputed");
    assert_eq!(m.get(), 4);
    assert_eq!(computes.get(), 2);
}
