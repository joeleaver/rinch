//! `spawn_local` and `sleep` (issue #1767), driven by hand.
//!
//! Every test in this binary installs the same two backends — a task scheduler
//! and a timer backend that only record what they were asked, per thread — so a
//! test decides exactly when each poll and each timer runs. The default path
//! (the main-thread callback queue, a worker thread's wake, a scheduler installed
//! late) is `task_default_wake_1767.rs`, its own binary because that queue is
//! process-global.

use std::cell::{Cell, RefCell};
use std::future::{Future, poll_fn};
use std::rc::Rc;
use std::task::{Poll, Waker};
use std::time::Duration;

use rinch_core::context::{current_dispatching_doc, push_dispatching_doc};
use rinch_core::reactive::{Scope, Signal, unowned};
use rinch_core::task::{set_task_scheduler, sleep, spawn_local};

thread_local! {
    static POLLS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    static TIMERS: RefCell<Vec<(u64, u32)>> = const { RefCell::new(Vec::new()) };
}

fn record_poll(id: u64) {
    POLLS.with(|q| q.borrow_mut().push(id));
}

fn record_timer(id: u64, delay_ms: u32) {
    TIMERS.with(|t| t.borrow_mut().push((id, delay_ms)));
}

fn install() {
    set_task_scheduler(record_poll);
    rinch_core::set_timer_backend(record_timer);
}

fn queued_polls() -> usize {
    POLLS.with(|q| q.borrow().len())
}

/// Run every scheduled poll, and the polls those schedule, until none is left.
fn run_polls() {
    loop {
        let ids: Vec<u64> = POLLS.with(|q| std::mem::take(&mut *q.borrow_mut()));
        if ids.is_empty() {
            return;
        }
        for id in ids {
            rinch_core::task::poll_task(id);
        }
    }
}

fn take_timers() -> Vec<(u64, u32)> {
    TIMERS.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

/// A hand-made one-shot the test completes, standing in for a callback API.
#[derive(Clone, Default)]
struct Gate {
    open: Rc<Cell<bool>>,
    waker: Rc<RefCell<Option<Waker>>>,
}

impl Gate {
    fn wait(&self) -> impl Future<Output = ()> + use<> {
        let gate = self.clone();
        poll_fn(move |cx| {
            if gate.open.get() {
                Poll::Ready(())
            } else {
                *gate.waker.borrow_mut() = Some(cx.waker().clone());
                Poll::Pending
            }
        })
    }

    fn open(&self) {
        self.open.set(true);
        if let Some(waker) = self.waker.borrow_mut().take() {
            waker.wake();
        }
    }
}

/// Sets a flag when dropped: whether a task's future was let go.
struct DropFlag(Rc<Cell<bool>>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test]
fn a_task_runs_after_spawn_returns_and_resumes_on_each_wake() {
    install();
    let gate = Gate::default();
    let log = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let count = Signal::new(0);

    let (l, g) = (log.clone(), gate.clone());
    let handle = spawn_local(async move {
        l.borrow_mut().push("start");
        g.wait().await;
        count.set(count.get() + 5);
        l.borrow_mut().push("end");
    });

    assert!(log.borrow().is_empty(), "spawn_local polls nothing inline");
    assert_eq!(queued_polls(), 1, "the first poll is scheduled");
    run_polls();
    assert_eq!(*log.borrow(), ["start"]);
    assert!(!handle.is_finished());

    gate.open();
    run_polls();
    assert_eq!(*log.borrow(), ["start", "end"]);
    assert_eq!(
        count.get(),
        5,
        "a signal set after an await, on this thread"
    );
    assert!(handle.is_finished());
}

#[test]
fn many_wakes_before_a_poll_schedule_one_poll() {
    install();
    let polls = Rc::new(Cell::new(0u32));
    let saved: Rc<RefCell<Option<Waker>>> = Rc::default();

    let (p, s) = (polls.clone(), saved.clone());
    spawn_local(poll_fn(move |cx| {
        p.set(p.get() + 1);
        *s.borrow_mut() = Some(cx.waker().clone());
        Poll::<()>::Pending
    }));
    run_polls();
    assert_eq!(polls.get(), 1);

    let waker = saved.borrow().clone().unwrap();
    waker.wake_by_ref();
    waker.wake_by_ref();
    let by_value = waker.clone();
    by_value.wake();
    assert_eq!(queued_polls(), 1, "three wakes, one scheduled poll");
    run_polls();
    assert_eq!(polls.get(), 2);
}

/// A poll a nested drain runs while the task is already being polled must not
/// be lost: the task is polled again once the running poll returns.
#[test]
fn a_wake_drained_during_the_tasks_own_poll_polls_it_again() {
    install();
    let polls = Rc::new(Cell::new(0u32));
    let p = polls.clone();
    spawn_local(poll_fn(move |cx| {
        p.set(p.get() + 1);
        if p.get() == 1 {
            cx.waker().wake_by_ref();
            // Something inside the poll drains the queue (the poll it just
            // asked for runs here, nested).
            let ids: Vec<u64> = POLLS.with(|q| std::mem::take(&mut *q.borrow_mut()));
            for id in ids {
                rinch_core::task::poll_task(id);
            }
        }
        if p.get() >= 2 {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }));
    run_polls();
    assert_eq!(
        polls.get(),
        2,
        "the wake taken during the first poll was kept"
    );
}

#[test]
fn sleep_arms_a_timer_on_first_poll_and_completes_when_it_fires() {
    install();
    take_timers();
    let done = Rc::new(Cell::new(false));
    let d = done.clone();
    spawn_local(async move {
        sleep(Duration::from_micros(2_500)).await;
        d.set(true);
    });
    assert!(
        take_timers().is_empty(),
        "nothing is armed before the first poll"
    );
    run_polls();
    let timers = take_timers();
    assert_eq!(timers.len(), 1);
    assert_eq!(
        timers[0].1, 3,
        "2.5 ms rounds up, never down to an early wake"
    );
    assert!(!done.get());

    rinch_core::fire_timeout(timers[0].0);
    assert!(!done.get(), "the fire wakes the task; it runs at its poll");
    run_polls();
    assert!(done.get());
}

#[test]
fn a_dropped_sleep_clears_its_timer() {
    install();
    take_timers();
    let woke = Rc::new(Cell::new(0u32));
    let w = woke.clone();
    let handle = spawn_local(async move {
        sleep(Duration::from_millis(40)).await;
        w.set(w.get() + 1);
    });
    run_polls();
    let timers = take_timers();
    assert_eq!(timers.len(), 1);
    handle.abort();
    rinch_core::fire_timeout(timers[0].0);
    assert_eq!(queued_polls(), 0, "the cleared timer woke nothing");
    run_polls();
    assert_eq!(woke.get(), 0);
}

#[test]
fn abort_drops_the_future_and_it_is_never_polled_again() {
    install();
    let gate = Gate::default();
    let dropped = Rc::new(Cell::new(false));
    let resumed = Rc::new(Cell::new(false));
    let (g, flag, r) = (gate.clone(), DropFlag(dropped.clone()), resumed.clone());
    let handle = spawn_local(async move {
        let _flag = flag;
        g.wait().await;
        r.set(true);
    });
    run_polls();
    handle.abort();
    assert!(dropped.get(), "abort drops the future at once");
    assert!(handle.is_finished());
    gate.open();
    run_polls();
    assert!(!resumed.get());
    handle.abort(); // again: a no-op
}

#[test]
fn a_task_aborting_itself_is_dropped_when_its_poll_returns() {
    install();
    let dropped = Rc::new(Cell::new(false));
    let me: Rc<Cell<Option<rinch_core::TaskHandle>>> = Rc::default();
    let (m, flag) = (me.clone(), DropFlag(dropped.clone()));
    let handle = spawn_local(poll_fn(move |cx| {
        let _keep = &flag;
        m.get().unwrap().abort();
        cx.waker().wake_by_ref();
        Poll::<()>::Pending
    }));
    me.set(Some(handle));
    run_polls();
    assert!(handle.is_finished());
    assert!(dropped.get(), "dropped when its own poll returned");
    assert_eq!(queued_polls(), 0, "its wake was not kept");
}

/// The lifetime rule: a task spawned during a render is dropped — not merely
/// skipped at its next wake — when the component unmounts. Its future here is
/// never woken again, so a check made only at the next poll would keep it (and
/// what it holds, a socket say) for the life of the app.
#[test]
fn a_task_spawned_during_a_render_is_dropped_at_unmount() {
    install();
    let gate = Gate::default();
    let dropped = Rc::new(Cell::new(false));
    let resumed = Rc::new(Cell::new(false));
    let component = Scope::new();
    let handle = component.run(|| {
        let state = Signal::new(1);
        let (g, flag, r) = (gate.clone(), DropFlag(dropped.clone()), resumed.clone());
        spawn_local(async move {
            let _flag = flag;
            g.wait().await;
            // Would panic on a freed signal if the task outlived its component.
            state.set(state.get() + 1);
            r.set(true);
        })
    });
    run_polls();
    assert!(!dropped.get());

    component.dispose();
    assert!(dropped.get(), "unmount drops the future right away");
    assert!(handle.is_finished());
    gate.open();
    run_polls();
    assert!(!resumed.get());
}

#[test]
fn an_unowned_task_spawned_during_a_render_outlives_the_component() {
    install();
    let gate = Gate::default();
    let resumed = Rc::new(Cell::new(false));
    let component = Scope::new();
    component.run(|| {
        let (g, r) = (gate.clone(), resumed.clone());
        unowned(|| {
            spawn_local(async move {
                g.wait().await;
                r.set(true);
            })
        })
    });
    run_polls();
    component.dispose();
    gate.open();
    run_polls();
    assert!(resumed.get(), "unowned is the opt-out");
}

/// A task runs with its component as the ambient owner, so a signal it
/// creates is the component's, and freed with it.
#[test]
fn a_task_allocates_under_the_component_that_spawned_it() {
    install();
    let component = Scope::new();
    let made: Rc<Cell<Option<Signal<u32>>>> = Rc::default();
    let m = made.clone();
    component.run(|| spawn_local(async move { m.set(Some(Signal::new(9))) }));
    let before = component.owned_counts().signals;
    run_polls();
    assert_eq!(component.owned_counts().signals, before + 1);
    component.dispose();
    assert!(!made.get().unwrap().is_alive());
}

/// Off the fixed point where "the resumer's document" and "the spawner's"
/// agree: spawned in document 1, polled from inside document 2.
#[test]
fn a_task_polls_under_the_document_that_spawned_it() {
    install();
    let seen: Rc<RefCell<Vec<Option<u64>>>> = Rc::default();
    for doc in [1u64, 0] {
        let _spawner = push_dispatching_doc(doc);
        let s = seen.clone();
        spawn_local(async move { s.borrow_mut().push(current_dispatching_doc()) });
    }
    {
        let _poller = push_dispatching_doc(2);
        run_polls();
        assert_eq!(current_dispatching_doc(), Some(2));
    }
    assert_eq!(*seen.borrow(), [Some(1), None]);
}

/// A poll is one transaction: an effect reading two signals the task sets
/// between two awaits runs once for the pair.
#[test]
fn each_poll_is_one_batch() {
    install();
    let a = Signal::new(0);
    let b = Signal::new(0);
    let runs = Rc::new(Cell::new(0u32));
    let r = runs.clone();
    let _effect = rinch_core::reactive::Effect::new(move || {
        let _ = (a.get(), b.get());
        r.set(r.get() + 1);
    });
    let before = runs.get();
    spawn_local(async move {
        a.set(1);
        b.set(1);
    });
    run_polls();
    assert_eq!(runs.get(), before + 1);
}

/// A thread that ends with a task still waiting on a `sleep` drops it from a TLS
/// destructor; clearing the sleep's timer there must not panic, which would
/// abort the whole process.
#[test]
fn a_thread_ending_with_a_sleeping_task_does_not_abort() {
    std::thread::spawn(|| {
        install();
        spawn_local(async {
            sleep(Duration::from_secs(60)).await;
        });
        run_polls();
        assert_eq!(take_timers().len(), 1, "the sleep armed its timer");
    })
    .join()
    .unwrap();
}
