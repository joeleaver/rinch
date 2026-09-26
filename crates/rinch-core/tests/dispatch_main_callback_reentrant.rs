//! Issue #1035 (review of PR #1059): a dispatcher that drops `f`, whose captures dispatch again in
//! their Drop, must not deadlock; a main-thread call queues under the current
//! document (#963) instead of running inline.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

static DROP_MODE: AtomicBool = AtomicBool::new(false);
static NESTED: AtomicUsize = AtomicUsize::new(0);

fn dispatcher(f: Box<dyn FnOnce() + Send>) {
    if DROP_MODE.swap(false, Ordering::SeqCst) {
        drop(f);
    } else {
        rinch_core::queue_main_callback(f);
    }
}

struct DispatchesOnDrop;
impl Drop for DispatchesOnDrop {
    fn drop(&mut self) {
        rinch_core::dispatch_main_callback(Box::new(|| {
            NESTED.fetch_add(1, Ordering::SeqCst);
        }));
    }
}

#[test]
fn reentrant_dispatch_from_a_dropped_closure_does_not_deadlock() {
    rinch_core::register_main_thread();
    rinch_core::set_cross_thread_dispatcher(dispatcher);
    rinch_core::drain_main_callbacks();

    // Positive control: the dispatcher really is the one being called.
    DROP_MODE.store(true, Ordering::SeqCst);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let guard = DispatchesOnDrop;
        rinch_core::dispatch_main_callback(Box::new(move || drop(guard)));
        tx.send(()).unwrap();
    });
    rx.recv_timeout(Duration::from_secs(5))
        .expect("dispatch_main_callback deadlocked re-entering from a Drop");
    assert!(!DROP_MODE.load(Ordering::SeqCst), "the dispatcher ran");
    assert!(
        rinch_core::main_callbacks_pending(),
        "the nested dispatch queued"
    );
    rinch_core::drain_main_callbacks();
    assert_eq!(NESTED.load(Ordering::SeqCst), 1);

    // Main thread: queued, not inline, and run under the queuing document.
    let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
    let s = seen.clone();
    {
        let _doc = rinch_core::context::push_dispatching_doc(77);
        rinch_core::dispatch_main_callback(Box::new(move || {
            *s.lock().unwrap() = Some(rinch_core::context::current_dispatching_doc());
        }));
    }
    assert!(
        seen.lock().unwrap().is_none(),
        "ran inline on the main thread"
    );
    rinch_core::drain_main_callbacks();
    assert_eq!(
        *seen.lock().unwrap(),
        Some(Some(77)),
        "runs under the queuing document"
    );
}
