//! Issue #1473: what `reload_image` costs while a document does not lay out.
//!
//! The queue of reloads a document has not drained was de-duplicated by a
//! scan of the whole queue, for each live document, under the process-wide
//! mutex: N distinct sources reloaded before a drain looked at N²/2 entries
//! (10,000 took 1.0 s, 20,000 4.8 s). These pin the cost as a count,
//! `reload_queue_steps` — the entries a call looked at to decide whether its
//! source was queued already — never as a time.
//!
//! This file is its own test binary and its tests run one at a time, so the
//! only live documents are the ones a test made.

use rinch_dom::RinchDocument;
use rinch_dom::image_cache::{has_pending, pending_reload_count, reload_image, reload_queue_steps};
use std::sync::Mutex;

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
fn alone() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner())
}

/// The steps `calls` reloads of distinct sources cost, cumulatively on top of
/// whatever the queue already holds.
fn steps_for(tag: &str, calls: usize) -> u64 {
    let before = reload_queue_steps();
    for i in 0..calls {
        reload_image(&format!("q1473:{tag}/{i:06}"));
    }
    reload_queue_steps() - before
}

#[test]
fn a_reload_looks_at_one_entry_however_long_the_queue_is() {
    let _alone = alone();
    let doc = RinchDocument::new();
    let key = doc.doc_key();

    // Three batches, never drained: the queue is 0, 300 and 1,500 long when
    // each starts. A scan costs 44,850, then 990,600, then 6,121,500.
    assert_eq!(steps_for("a", 300), 300, "an empty queue");
    assert_eq!(pending_reload_count(key), 300);
    assert_eq!(steps_for("b", 1_200), 1_200, "300 queued");
    assert_eq!(steps_for("c", 2_700), 2_700, "1,500 queued");
    assert_eq!(pending_reload_count(key), 4_200, "every source is queued");
    assert!(has_pending(key));
    drop(doc);
}

#[test]
fn a_source_queued_already_costs_one_step_and_is_not_queued_twice() {
    let _alone = alone();
    let doc = RinchDocument::new();
    let key = doc.doc_key();
    assert_eq!(steps_for("dup", 500), 500);

    // The first source, the last, and one in the middle, again: a scan finds
    // them at 1, 500 and 251 entries in.
    let before = reload_queue_steps();
    reload_image("q1473:dup/000000");
    reload_image("q1473:dup/000499");
    reload_image("q1473:dup/000250");
    assert_eq!(reload_queue_steps() - before, 3);
    assert_eq!(pending_reload_count(key), 500, "coalesced, not appended");
    drop(doc);
}

#[test]
fn each_live_document_costs_one_step_a_call() {
    let _alone = alone();
    let a = RinchDocument::new();
    let b = RinchDocument::new();
    let c = RinchDocument::new();
    assert_eq!(steps_for("docs", 400), 1_200, "three documents");
    for doc in [&a, &b, &c] {
        assert_eq!(pending_reload_count(doc.doc_key()), 400);
    }

    // One drops: its entries go with it, the others keep theirs, and a call
    // costs two steps from here on.
    let gone = b.doc_key();
    drop(b);
    assert_eq!(pending_reload_count(gone), 0, "purged at the drop");
    assert!(!has_pending(gone));
    assert_eq!(pending_reload_count(a.doc_key()), 400);
    assert_eq!(steps_for("docs-2", 400), 800, "two documents");
    assert_eq!(pending_reload_count(gone), 0, "nothing queued for it since");
    assert_eq!(pending_reload_count(c.doc_key()), 800);
    drop((a, c));
}

#[test]
fn a_reload_with_no_live_document_queues_nothing_and_costs_nothing() {
    let _alone = alone();
    let doc = RinchDocument::new();
    let key = doc.doc_key();
    assert_eq!(steps_for("live", 10), 10, "control: a live document is reached");
    drop(doc);
    assert_eq!(steps_for("none", 1_000), 0);
    assert_eq!(pending_reload_count(key), 0);
}
