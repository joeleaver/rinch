//! `spawn_local` with no task scheduler installed (issue #1767): polls go
//! through the main-thread callback queue every host drains, a worker thread
//! may wake a task, and a scheduler installed later picks up tasks already
//! waiting. One test, alone in its binary, because that queue is process-global.

use std::cell::{Cell, RefCell};
use std::future::poll_fn;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};

use rinch_core::task::{set_task_scheduler, spawn_local};

thread_local! {
    static LATE: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

fn late_scheduler(id: u64) {
    LATE.with(|q| q.borrow_mut().push(id));
}

#[test]
fn polls_ride_the_main_callback_queue_until_a_scheduler_is_installed() {
    let polls = Rc::new(Cell::new(0u32));
    let saved: Arc<Mutex<Option<Waker>>> = Arc::default();

    let (p, s) = (polls.clone(), saved.clone());
    spawn_local(poll_fn(move |cx| {
        p.set(p.get() + 1);
        *s.lock().unwrap() = Some(cx.waker().clone());
        if p.get() == 2 {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }));
    assert!(
        rinch_core::reactive::main_callbacks_pending(),
        "first poll queued"
    );
    rinch_core::reactive::drain_main_callbacks();
    assert_eq!(polls.get(), 1);

    // A worker thread wakes it (a request finishing off-thread).
    let waker = saved.lock().unwrap().clone().unwrap();
    std::thread::spawn(move || waker.wake()).join().unwrap();
    assert!(rinch_core::reactive::main_callbacks_pending());
    rinch_core::reactive::drain_main_callbacks();
    assert_eq!(
        polls.get(),
        2,
        "resumed on this thread after the worker's wake"
    );

    // Spawned before a scheduler exists (a web app spawning before its first
    // mount): the poll waits on the queue, which a browser never drains, so
    // installing the scheduler must schedule it there too.
    let ran = Rc::new(Cell::new(false));
    let r = ran.clone();
    spawn_local(async move { r.set(true) });
    set_task_scheduler(late_scheduler);
    let ids: Vec<u64> = LATE.with(|q| std::mem::take(&mut *q.borrow_mut()));
    assert_eq!(
        ids.len(),
        1,
        "the waiting task is handed to the new scheduler"
    );
    for id in ids {
        rinch_core::task::poll_task(id);
    }
    assert!(ran.get());
    // Its stale queue entry is harmless: the task is gone.
    rinch_core::reactive::drain_main_callbacks();
}
