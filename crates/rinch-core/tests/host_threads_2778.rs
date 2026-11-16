//! Host threads and the per-thread main queue (issue #2778).
//!
//! The main-thread queue is one per process, and a test harness runs an app
//! on each test thread, several at once. Each such thread is a host of its own
//! (`mark_host_thread`): a write to a signal is queued for the thread whose
//! store holds the signal, a closure a host thread queues is for that thread,
//! and a drain runs only those and the untargeted ones. Before, any thread's
//! drain ran every entry, writing the signal of the same id in its own store.
//!
//! No main thread is registered in this binary (an app's test binary);
//! `rinch/tests/testing_parallel_send_registered_2778.rs` covers a process
//! where one is.

use rinch_core::{
    Signal, drain_main_callbacks, main_callbacks_pending, mark_host_thread, queue_main_callback,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex};

/// Run `f` on a new thread named `name` and return what it returns.
fn on_thread<R: Send + 'static>(name: &str, f: impl FnOnce() -> R + Send + 'static) -> R {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

fn worker(f: impl FnOnce() + Send + 'static) {
    on_thread("worker", f);
}

/// Two host threads with signals of the same id: each one's workers' writes,
/// and its own, reach its signal only, whichever thread drains first.
#[test]
fn each_host_thread_drains_only_the_writes_to_its_own_signals() {
    let queued = Arc::new(Barrier::new(2));
    let a_drained = Arc::new(Barrier::new(2));
    let run = |name: &'static str| {
        let (queued, a_drained) = (queued.clone(), a_drained.clone());
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                mark_host_thread();
                let items: Signal<Vec<String>> = Signal::new(Vec::new());
                let own = format!("{name}-own");
                items.update_send(move |v| v.push(own));
                assert_eq!(
                    items.get(),
                    [format!("{name}-own")],
                    "a host thread's write to its own signal applies at once"
                );
                let sent = format!("{name}-worker");
                worker(move || items.update_send(move |v| v.push(sent)));
                queued.wait();
                if name == "a" {
                    drain_main_callbacks();
                    // Not "nothing pending": another test's worker may have
                    // queued an untargeted closure meanwhile.
                    a_drained.wait();
                } else {
                    a_drained.wait();
                    assert!(main_callbacks_pending(), "b's write waits for b");
                    drain_main_callbacks();
                }
                items.get()
            })
            .unwrap()
    };
    let (a, b) = (run("a"), run("b"));
    assert_eq!(a.join().unwrap(), ["a-own", "a-worker"]);
    assert_eq!(b.join().unwrap(), ["b-own", "b-worker"]);
}

/// A closure a host thread queues waits for that thread's drain; another
/// host thread's drain leaves it.
#[test]
fn a_closure_a_host_thread_queues_is_for_that_thread() {
    let ran_on: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let r = ran_on.clone();
    let (queued_tx, queued_rx) = std::sync::mpsc::channel::<()>();
    let (other_drained_tx, other_drained_rx) = std::sync::mpsc::channel::<()>();
    let owner = std::thread::Builder::new()
        .name("owner".into())
        .spawn(move || {
            mark_host_thread();
            queue_main_callback(Box::new(move || {
                let name = std::thread::current().name().unwrap().to_owned();
                r.lock().unwrap().push(name);
            }));
            queued_tx.send(()).unwrap();
            other_drained_rx.recv().unwrap();
            drain_main_callbacks();
        })
        .unwrap();
    queued_rx.recv().unwrap();
    on_thread("other", || {
        mark_host_thread();
        drain_main_callbacks();
    });
    other_drained_tx.send(()).unwrap();
    owner.join().unwrap();
    assert_eq!(*ran_on.lock().unwrap(), ["owner"]);
}

/// A host thread writing another host thread's signal: queued for the owner,
/// not applied to the signal of the same id here.
#[test]
fn a_write_to_another_host_threads_signal_goes_to_that_thread() {
    let (signal_tx, signal_rx) = std::sync::mpsc::channel::<Signal<u32>>();
    let (sent_tx, sent_rx) = std::sync::mpsc::channel::<()>();
    let owner = std::thread::Builder::new()
        .name("owner".into())
        .spawn(move || {
            mark_host_thread();
            let value = Signal::new(0u32);
            signal_tx.send(value).unwrap();
            sent_rx.recv().unwrap();
            drain_main_callbacks();
            value.get()
        })
        .unwrap();
    let theirs = signal_rx.recv().unwrap();
    let mine_after = on_thread("writer", move || {
        mark_host_thread();
        // The same slot id as the owner's signal, in this thread's store.
        let mine = Signal::new(0u32);
        // The slot index lives in an id's low 24 bits, the thread's tag above.
        let slot = |id: u32| id & 0x00ff_ffff;
        assert_eq!(
            slot(mine.debug_id()),
            slot(theirs.debug_id()),
            "the slots alias"
        );
        theirs.send(7);
        drain_main_callbacks();
        mine.get()
    });
    sent_tx.send(()).unwrap();
    assert_eq!(
        mine_after, 0,
        "the write did not land in the writer's signal"
    );
    assert_eq!(
        owner.join().unwrap(),
        7,
        "the owner's signal took the write"
    );
}

/// A write to a signal of a thread that has ended does not reach a signal of
/// a later thread, though that thread may have taken over the ended one's
/// tag: the generation counter continues across the tag's owners.
#[test]
fn a_write_to_an_ended_threads_signal_reaches_no_later_threads_signal() {
    let stale = on_thread("ended", || {
        mark_host_thread();
        Signal::new(0u32)
    });
    let after = on_thread("later", move || {
        mark_host_thread();
        let mine = Signal::new(0u32);
        worker(move || stale.send(9));
        drain_main_callbacks();
        mine.get()
    });
    assert_eq!(after, 0);
}

/// A closure a worker queues names no thread: whichever host thread drains
/// runs it, as before #2778.
#[test]
fn a_workers_closure_is_run_by_whichever_host_drains() {
    let ran = Arc::new(AtomicBool::new(false));
    let r = ran.clone();
    worker(move || {
        queue_main_callback(Box::new(move || r.store(true, Ordering::SeqCst)));
    });
    on_thread("host", move || {
        mark_host_thread();
        drain_main_callbacks();
    });
    // Another test's host thread may have drained it first: run anywhere.
    for _ in 0..200 {
        if ran.load(Ordering::SeqCst) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the worker's closure was never run");
}
