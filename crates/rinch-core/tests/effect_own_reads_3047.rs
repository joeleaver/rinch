//! An effect depends only on what it read during its own run (issue #3047).
//!
//! An effect's write flushes the effects it wakes synchronously, one frame
//! down its own stack (outside a batch, which is where a batch's exit flush
//! runs effects too). Those nested runs used to sit *on top of* the writing
//! effect on the observer stack, and the frame boundary between them was only
//! a push: `untracked` popped one frame, so an `untracked` read inside the
//! nested run subscribed the **writer**. Every `match` / `if let` swap renders
//! its new branch under `untracked`, so an effect that wrote the signal a
//! branch reads picked up the branch's reads, and re-ran when they changed —
//! re-ofscp's admin console sent each request twice.
//!
//! The rule pinned here: each effect run and each memo recompute is its own
//! tracking frame. Nothing that runs inside it — a nested effect, a memo, an
//! `untracked` region inside either — can hand a read to the frame below.
//!
//! Every fixture has a positive control showing the nested observer is live,
//! and counts runs of the writer, so an extra run fails it.

use std::cell::Cell;
use std::rc::Rc;

use rinch_core::Signal;
use rinch_core::reactive::{Effect, Memo, batch, untracked};

/// The issue's shape, without rsx: `writer` reads `trigger` and writes
/// `shown`; `view` (created first, so it already exists) reads `shown` and,
/// like a branch swap, reads `detail` under `untracked`.
struct Fixture {
    trigger: Signal<i32>,
    shown: Signal<i32>,
    detail: Signal<i32>,
    writer_runs: Rc<Cell<u32>>,
    view_runs: Rc<Cell<u32>>,
    _view: Effect,
    _writer: Effect,
}

fn fixture() -> Fixture {
    let trigger = Signal::new(0);
    let shown = Signal::new(0);
    let detail = Signal::new(0);
    let view_runs = Rc::new(Cell::new(0u32));
    let view = Effect::new({
        let view_runs = view_runs.clone();
        move || {
            shown.get();
            view_runs.set(view_runs.get() + 1);
            untracked(|| detail.get());
        }
    });
    let writer_runs = Rc::new(Cell::new(0u32));
    let writer = Effect::new({
        let writer_runs = writer_runs.clone();
        move || {
            let t = trigger.get();
            writer_runs.set(writer_runs.get() + 1);
            shown.set(t + 1);
        }
    });
    Fixture {
        trigger,
        shown,
        detail,
        writer_runs,
        view_runs,
        _view: view,
        _writer: writer,
    }
}

#[test]
fn an_untracked_read_in_an_effect_its_write_flushes_does_not_subscribe_it() {
    let f = fixture();
    assert_eq!(f.writer_runs.get(), 1);
    assert_eq!(
        f.view_runs.get(),
        2,
        "control: the writer's write re-ran the view inside the writer's run"
    );

    f.detail.set(7);
    assert_eq!(
        f.writer_runs.get(),
        1,
        "the writer never read `detail`; the view's untracked read must not subscribe it"
    );

    f.trigger.set(5);
    assert_eq!(
        f.writer_runs.get(),
        2,
        "positive control: the writer tracks its own signal"
    );
    assert_eq!(f.view_runs.get(), 3);
    f.detail.set(8);
    assert_eq!(
        f.writer_runs.get(),
        2,
        "nor after a re-run: the nested flush is the same on every run"
    );
}

/// A handler's batch: the writer runs at the batch's exit flush, and its own
/// write flushes the view nested inside it from there.
#[test]
fn the_same_holds_for_a_run_from_a_batch_exit_flush() {
    let f = fixture();
    batch(|| f.trigger.set(3));
    assert_eq!(
        (f.writer_runs.get(), f.view_runs.get()),
        (2, 3),
        "control: the batch ran the writer, whose write re-ran the view"
    );
    assert_eq!(f.shown.get(), 4);

    f.detail.set(9);
    assert_eq!(
        f.writer_runs.get(),
        2,
        "a read the writer never made must not re-run it"
    );
}

/// A memo the writer reads computes inside the writer's run; an `untracked`
/// read in that computation is nobody's.
#[test]
fn an_untracked_read_in_a_memo_computation_does_not_subscribe_its_reader() {
    let source = Signal::new(1);
    let side = Signal::new(0);
    let computed = Rc::new(Cell::new(0u32));
    let memo = Memo::new({
        let computed = computed.clone();
        move || {
            computed.set(computed.get() + 1);
            source.get() + untracked(|| side.get())
        }
    });
    let runs = Rc::new(Cell::new(0u32));
    let _reader = Effect::new({
        let runs = runs.clone();
        move || {
            memo.get();
            runs.set(runs.get() + 1);
        }
    });
    assert_eq!((runs.get(), computed.get()), (1, 1));

    side.set(5);
    assert_eq!(
        runs.get(),
        1,
        "`side` is read untracked inside the memo; the memo's reader never read it"
    );

    source.set(2);
    assert_eq!(
        (runs.get(), computed.get()),
        (2, 2),
        "positive control: the reader follows the memo"
    );
}

/// An effect created inside another effect's run is a frame of its own from
/// its first run: an `untracked` read there subscribes neither effect.
#[test]
fn an_untracked_read_in_an_effect_created_inside_a_run_subscribes_neither() {
    let own = Signal::new(0);
    let side = Signal::new(0);
    let outer_runs = Rc::new(Cell::new(0u32));
    let inner_runs = Rc::new(Cell::new(0u32));
    let slot: Rc<std::cell::RefCell<Option<Effect>>> = Rc::default();
    let _outer = Effect::new({
        let (outer_runs, inner_runs, slot) = (outer_runs.clone(), inner_runs.clone(), slot.clone());
        move || {
            outer_runs.set(outer_runs.get() + 1);
            let inner_runs = inner_runs.clone();
            slot.replace(Some(Effect::new(move || {
                own.get();
                inner_runs.set(inner_runs.get() + 1);
                untracked(|| side.get());
            })));
        }
    });

    side.set(1);
    assert_eq!(
        (outer_runs.get(), inner_runs.get()),
        (1, 1),
        "a read under `untracked` belongs to no effect, at any depth"
    );
    own.set(1);
    assert_eq!(
        inner_runs.get(),
        2,
        "positive control: the inner effect tracks its own signal"
    );
}

/// The reads an effect makes itself, around a nested flush, still count: the
/// fix isolates frames, it does not stop tracking after a write.
#[test]
fn an_effect_still_tracks_what_it_reads_after_its_own_write_flushed() {
    let f = fixture();
    let later = Signal::new(0);
    let runs = Rc::new(Cell::new(0u32));
    let _e = Effect::new({
        let runs = runs.clone();
        let shown = f.shown;
        move || {
            runs.set(runs.get() + 1);
            shown.set(100);
            later.get();
        }
    });
    assert_eq!(runs.get(), 1);
    later.set(1);
    assert_eq!(
        runs.get(),
        2,
        "a read after the write's nested flush is the effect's own"
    );
}
