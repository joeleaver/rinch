//! A closure queued for the main thread runs in the **context root** whose
//! code queued it, as well as under its document (issue #1032).
//!
//! An embedded `RinchContext` drains the queue at the top of `update()`, with
//! no context root pushed — so without a recorded root a closure queued by a
//! context's handler resolved the thread-global store namespace, not its own
//! context's (#136). A closure queued from a worker thread is no context's code
//! and runs at the thread-global root.
//!
//! `MAIN_QUEUE` is a process-global `static`, so this lives in its own
//! integration-test binary with a single test function.

use std::sync::{Arc, Mutex};

use rinch_core::{
    current_context_root, current_dispatching_doc, drain_main_callbacks, push_context_root,
    push_dispatching_doc, queue_main_callback,
};

/// Which closure ran, and the root and document it ran in.
type Seen = Arc<Mutex<Vec<(&'static str, u64, Option<u64>)>>>;

#[test]
fn a_queued_closure_runs_in_the_context_root_that_queued_it() {
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));

    {
        let _root = push_context_root(9);
        let _doc = push_dispatching_doc(1);
        let s = Arc::clone(&seen);
        queue_main_callback(Box::new(move || {
            s.lock()
                .unwrap()
                .push(("context", current_context_root(), current_dispatching_doc()));
        }));
    }
    {
        let s = Arc::clone(&seen);
        std::thread::spawn(move || {
            queue_main_callback(Box::new(move || {
                s.lock().unwrap().push((
                    "worker",
                    current_context_root(),
                    current_dispatching_doc(),
                ));
            }));
        })
        .join()
        .unwrap();
    }

    {
        // Drained from inside another root and document, off the fixed point
        // where "inherit the drainer's" and "record the queuer's" agree.
        let _root = push_context_root(4);
        let _doc = push_dispatching_doc(2);
        drain_main_callbacks();
        assert_eq!(
            (current_context_root(), current_dispatching_doc()),
            (4, Some(2)),
            "the drainer's root and document are restored after each closure"
        );
    }

    assert_eq!(
        *seen.lock().unwrap(),
        [("context", 9, Some(1)), ("worker", 0, None)]
    );
}
