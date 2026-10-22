//! `rinch::worker` on the desktop (issue #1772): a thread per worker, replies
//! through the main-thread callback queue.
//!
//! One `#[test]`: the main-thread queue is process-global and the worker
//! registry is thread-local, so a second test thread draining the queue would
//! run this thread's deliveries against its own (empty) registry.

#![cfg(feature = "worker")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use rinch::worker::{Replies, Worker, WorkerEvent, WorkerMessage, is_worker};
use rinch_core::reactive::{Scope, drain_main_callbacks};

/// Drain the main-thread queue until `done` holds, or fail after 5 s.
fn drain_until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    loop {
        drain_main_callbacks();
        if done() {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "timed out: {what}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Drain for a while, for "nothing more arrives" checks.
fn drain_for(ms: u64) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        drain_main_callbacks();
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn collect(worker: &Worker<String, String>) -> Rc<RefCell<Vec<WorkerEvent<String>>>> {
    let got: Rc<RefCell<Vec<WorkerEvent<String>>>> = Rc::default();
    let g = got.clone();
    worker.on_event(move |e| g.borrow_mut().push(e));
    got
}

fn msg(s: &str) -> WorkerEvent<String> {
    WorkerEvent::Message(s.to_string())
}

// --- handlers -------------------------------------------------------------

thread_local! {
    static SEEN: Cell<u32> = const { Cell::new(0) };
}

/// Numbers each message by how many this worker has handled: state kept across
/// messages, per worker.
fn numbered(text: String, replies: &Replies<String>) {
    let n = SEEN.with(|s| {
        s.set(s.get() + 1);
        s.get()
    });
    replies.send(format!("{n}:{}:{}", text.to_uppercase(), is_worker()));
}

static SPIN_STARTED: AtomicBool = AtomicBool::new(false);
static SPIN_RETURNED: AtomicBool = AtomicBool::new(false);
static SPIN_HANDLED: AtomicU32 = AtomicU32::new(0);

/// Runs until terminated, then tries to reply (which must be dropped).
fn spin_until_terminated(_: String, replies: &Replies<String>) {
    SPIN_HANDLED.fetch_add(1, Ordering::SeqCst);
    SPIN_STARTED.store(true, Ordering::SeqCst);
    while !replies.is_terminated() {
        std::thread::yield_now();
    }
    replies.send("after terminate".into());
    SPIN_RETURNED.store(true, Ordering::SeqCst);
}

fn panics(text: String, replies: &Replies<String>) {
    if text == "boom" {
        panic!("handler gave up");
    }
    replies.send(text);
}

/// A reply type whose codec cannot read back what it wrote for zero.
#[derive(Debug, Clone, PartialEq)]
struct Lossy(u8);
impl WorkerMessage for Lossy {
    fn into_bytes(self) -> Vec<u8> {
        vec![self.0]
    }
    fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        match bytes.as_slice() {
            [0] | [] => None,
            [v] => Some(Lossy(*v)),
            _ => None,
        }
    }
}

fn lossy(n: String, replies: &Replies<Lossy>) {
    replies.send(Lossy(n.parse().unwrap()));
}

fn echo(text: String, replies: &Replies<String>) {
    replies.send(text);
}

// --- cases ----------------------------------------------------------------

fn round_trip_in_order_with_state() {
    let worker = Worker::<String, String>::spawn(numbered);
    let got = collect(&worker);
    for m in ["a", "b", "c"] {
        worker.send(m.into());
    }
    drain_until("three replies", || got.borrow().len() == 3);
    assert_eq!(
        *got.borrow(),
        vec![msg("1:A:true"), msg("2:B:true"), msg("3:C:true")],
        "in order, numbered by the worker's own state, inside a worker"
    );
    assert!(!is_worker(), "the UI thread is not a worker");

    // A second worker has state of its own.
    let other = Worker::<String, String>::spawn(numbered);
    let got2 = collect(&other);
    other.send("z".into());
    drain_until("other's reply", || got2.borrow().len() == 1);
    assert_eq!(*got2.borrow(), vec![msg("1:Z:true")]);
}

fn replies_go_through_bytes_on_the_desktop() {
    let worker = Worker::<String, Lossy>::spawn(lossy);
    let got: Rc<RefCell<Vec<WorkerEvent<Lossy>>>> = Rc::default();
    let g = got.clone();
    worker.on_event(move |e| g.borrow_mut().push(e));
    worker.send("7".into());
    worker.send("0".into());
    drain_until("two events", || got.borrow().len() == 2);
    assert_eq!(got.borrow()[0], WorkerEvent::Message(Lossy(7)));
    assert!(
        matches!(got.borrow()[1], WorkerEvent::Error(_)),
        "a reply the codec cannot read back is an error on the desktop too: {:?}",
        got.borrow()[1]
    );
    assert!(!worker.is_ended(), "a decode error does not end the worker");
}

fn terminate_is_cooperative() {
    let worker = Worker::<String, String>::spawn(spin_until_terminated);
    let got = collect(&worker);
    worker.send("spin".into());
    worker.send("never handled".into());
    drain_until("the handler is running", || {
        SPIN_STARTED.load(Ordering::SeqCst)
    });
    assert!(!SPIN_RETURNED.load(Ordering::SeqCst), "still spinning");

    worker.terminate();
    assert!(worker.is_ended());
    let start = Instant::now();
    while !SPIN_RETURNED.load(Ordering::SeqCst) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the handler saw is_terminated"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    drain_for(50);
    assert!(
        got.borrow().is_empty(),
        "a reply sent after terminate is dropped: {:?}",
        got.borrow()
    );
    assert_eq!(
        SPIN_HANDLED.load(Ordering::SeqCst),
        1,
        "the queued message was dropped"
    );
    worker.send("ignored".into());
}

fn a_panic_ends_the_worker() {
    let worker = Worker::<String, String>::spawn(panics);
    worker.send("ok".into());
    worker.send("boom".into());
    worker.send("after".into());
    let events: Rc<RefCell<Option<Vec<WorkerEvent<String>>>>> = Rc::default();
    let w = Rc::new(worker);
    let (e, w2) = (events.clone(), w.clone());
    rinch_core::spawn_local(async move {
        let mut all = Vec::new();
        while let Some(event) = w2.next().await {
            all.push(event);
        }
        *e.borrow_mut() = Some(all);
    });
    drain_until("the stream ends", || events.borrow().is_some());
    let all = events.borrow_mut().take().unwrap();
    assert_eq!(all.len(), 2, "{all:?}");
    assert_eq!(all[0], msg("ok"));
    assert!(
        matches!(&all[1], WorkerEvent::Error(e) if e.contains("handler gave up")),
        "{:?}",
        all[1]
    );
    assert!(w.is_ended());
}

fn replies_are_awaitable() {
    let worker = Rc::new(Worker::<String, String>::spawn(echo));
    let result = Rc::new(RefCell::new(None));
    let (r, w) = (result.clone(), worker.clone());
    rinch_core::spawn_local(async move {
        w.send("ping".into());
        let first = w.next().await;
        w.send("pong".into());
        let second = w.next().await;
        *r.borrow_mut() = Some((first, second));
    });
    drain_until("both replies awaited", || result.borrow().is_some());
    assert_eq!(
        result.borrow_mut().take().unwrap(),
        (Some(msg("ping")), Some(msg("pong")))
    );

    // A pending `next` resolves to `None` when the worker is terminated: the
    // deadline pattern.
    let slow = Rc::new(Worker::<String, String>::spawn(echo));
    let ended = Rc::new(Cell::new(None));
    let (e, s) = (ended.clone(), slow.clone());
    rinch_core::spawn_local(async move { e.set(Some(s.next().await.is_none())) });
    drain_for(20);
    assert_eq!(ended.get(), None, "still waiting");
    slow.terminate();
    drain_until("next resolved", || ended.get().is_some());
    assert_eq!(ended.get(), Some(true));
}

fn unmount_terminates() {
    let scope = Scope::new();
    let worker = scope.run(|| Worker::<String, String>::spawn(echo));
    let got = collect(&worker);
    worker.send("before".into());
    drain_until("a reply", || got.borrow().len() == 1);
    assert!(!worker.is_ended());

    scope.dispose();
    assert!(worker.is_ended(), "the component's unmount terminated it");
    worker.send("after".into());
    drain_for(30);
    assert_eq!(*got.borrow(), vec![msg("before")]);
}

fn a_callback_stops_with_its_component() {
    let worker = Worker::<String, String>::spawn(echo);
    let calls = Rc::new(Cell::new(0));
    let scope = Scope::new();
    let c = calls.clone();
    scope.run(|| worker.on_event(move |_| c.set(c.get() + 1)));
    worker.send("one".into());
    drain_until("one call", || calls.get() == 1);
    scope.dispose();
    worker.send("two".into());
    worker.send("three".into());
    drain_for(30);
    assert_eq!(calls.get(), 1, "not called after its component unmounted");
    assert!(
        !worker.is_ended(),
        "the worker itself was spawned outside it"
    );

    // What arrived after the unmount was dropped, not kept for `next`.
    let worker = Rc::new(worker);
    let seen = Rc::new(RefCell::new(None));
    let (s, w) = (seen.clone(), worker.clone());
    rinch_core::spawn_local(async move { *s.borrow_mut() = Some(w.next().await) });
    drain_for(20);
    worker.terminate();
    drain_until("next resolved", || seen.borrow().is_some());
    assert_eq!(
        seen.borrow_mut().take().unwrap(),
        None,
        "nothing piled up for next() after the callback's component unmounted"
    );

    // A new callback receives again.
    let worker = Worker::<String, String>::spawn(echo);
    let scope = Scope::new();
    scope.run(|| worker.on_event(|_| {}));
    scope.dispose();
    worker.send("dropped".into());
    drain_for(20);
    let got = collect(&worker);
    worker.send("kept".into());
    drain_until("the new callback's event", || got.borrow().len() == 1);
    assert_eq!(*got.borrow(), vec![msg("kept")]);
}

fn two_waiters_are_both_served() {
    let worker = Rc::new(Worker::<String, String>::spawn(echo));
    let results: Rc<RefCell<Vec<Option<WorkerEvent<String>>>>> = Rc::default();
    for _ in 0..2 {
        let (r, w) = (results.clone(), worker.clone());
        rinch_core::spawn_local(async move {
            let event = w.next().await;
            r.borrow_mut().push(event);
        });
    }
    drain_for(20);
    worker.send("a".into());
    worker.send("b".into());
    drain_until("both waiters", || results.borrow().len() == 2);
    let mut got: Vec<_> = results.borrow_mut().drain(..).collect();
    got.sort_by_key(|e| format!("{e:?}"));
    assert_eq!(got, vec![Some(msg("a")), Some(msg("b"))]);

    // And both see the end.
    for _ in 0..2 {
        let (r, w) = (results.clone(), worker.clone());
        rinch_core::spawn_local(async move {
            let event = w.next().await;
            r.borrow_mut().push(event);
        });
    }
    drain_for(20);
    worker.terminate();
    drain_until("both ended", || results.borrow().len() == 2);
    assert_eq!(*results.borrow(), vec![None, None]);
}

fn uses_spawn_local(_: String, _: &Replies<String>) {
    rinch_core::spawn_local(async {});
}

fn uses_set_timeout(_: String, _: &Replies<String>) {
    rinch_core::set_timeout(1, || {});
}

fn uses_set_interval(_: String, _: &Replies<String>) {
    rinch_core::set_interval(1, || {});
}

fn spawns_a_worker(_: String, replies: &Replies<String>) {
    let inner = Worker::<String, String>::spawn(echo);
    replies.send(format!("ended:{}", inner.is_ended()));
}

fn tasks_and_timers_refuse_inside_a_handler() {
    for handler in [uses_spawn_local, uses_set_timeout, uses_set_interval] {
        let worker = Rc::new(Worker::<String, String>::spawn(handler));
        worker.send("go".into());
        let events = Rc::new(RefCell::new(None));
        let (e, w) = (events.clone(), worker.clone());
        rinch_core::spawn_local(async move {
            let mut all = Vec::new();
            while let Some(event) = w.next().await {
                all.push(event);
            }
            *e.borrow_mut() = Some(all);
        });
        drain_until("the worker ended", || events.borrow().is_some());
        let all = events.borrow_mut().take().unwrap();
        assert!(
            matches!(&all[..], [WorkerEvent::Error(e)] if e.contains("inside a rinch::worker handler")),
            "{all:?}"
        );
    }
    // The UI thread is unaffected.
    let ran = Rc::new(Cell::new(false));
    let r = ran.clone();
    rinch_core::spawn_local(async move { r.set(true) });
    drain_until("a UI task still runs", || ran.get());
}

fn a_worker_spawns_no_worker() {
    let worker = Worker::<String, String>::spawn(spawns_a_worker);
    let got = collect(&worker);
    worker.send("go".into());
    drain_until("a reply", || got.borrow().len() == 1);
    assert_eq!(*got.borrow(), vec![msg("ended:true")]);
}

fn dropping_terminates() {
    SPIN_STARTED.store(false, Ordering::SeqCst);
    SPIN_RETURNED.store(false, Ordering::SeqCst);
    let worker = Worker::<String, String>::spawn(spin_until_terminated);
    worker.send("spin".into());
    let start = Instant::now();
    while !SPIN_STARTED.load(Ordering::SeqCst) {
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1));
    }
    drop(worker);
    while !SPIN_RETURNED.load(Ordering::SeqCst) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "drop told the handler"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn workers_on_the_desktop() {
    round_trip_in_order_with_state();
    replies_go_through_bytes_on_the_desktop();
    terminate_is_cooperative();
    a_panic_ends_the_worker();
    replies_are_awaitable();
    unmount_terminates();
    a_callback_stops_with_its_component();
    two_waiters_are_both_served();
    tasks_and_timers_refuse_inside_a_handler();
    a_worker_spawns_no_worker();
    dropping_terminates();
}
