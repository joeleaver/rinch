//! A closure queued for the main thread runs under the document whose code
//! queued it (issue #963).
//!
//! The queue is drained by the hosts outside any event dispatch — or, in an
//! embedded `RinchContext`, at the top of whichever context's `update()` runs
//! next — so without a recorded document a closure queued by document 1 ran
//! under no document, or under another context's. A closure queued from a
//! worker thread was created under no document and runs under none.
//!
//! `MAIN_QUEUE` is a process-global `static`, so this lives in its own
//! integration-test binary with a single test function: libtest would otherwise
//! let another test's drain run these closures on its thread.

use std::sync::{Arc, Mutex};

use rinch_core::{
    current_dispatching_doc, drain_main_callbacks, push_dispatching_doc, queue_main_callback,
};

/// Which closure ran, and the document it ran under.
type Seen = Arc<Mutex<Vec<(&'static str, Option<u64>)>>>;

#[test]
fn a_queued_closure_runs_under_the_document_that_queued_it() {
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));

    {
        let _doc1 = push_dispatching_doc(1);
        let s = Arc::clone(&seen);
        queue_main_callback(Box::new(move || {
            s.lock().unwrap().push(("doc1", current_dispatching_doc()));
        }));
    }
    {
        let s = Arc::clone(&seen);
        std::thread::spawn(move || {
            queue_main_callback(Box::new(move || {
                s.lock()
                    .unwrap()
                    .push(("worker", current_dispatching_doc()));
            }));
        })
        .join()
        .unwrap();
    }

    {
        // Drained from inside document 2 — an embedded context's `update()` —
        // off the fixed point where "inherit the drainer's" and "record the
        // queuer's" agree.
        let _drainer = push_dispatching_doc(2);
        drain_main_callbacks();
        assert_eq!(
            current_dispatching_doc(),
            Some(2),
            "the drainer's document is restored after each closure"
        );
    }

    assert_eq!(*seen.lock().unwrap(), [("doc1", Some(1)), ("worker", None)]);
}
