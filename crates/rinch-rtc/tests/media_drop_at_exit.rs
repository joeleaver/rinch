//! A local track still held when its thread exits (#1916): a `LocalTrack` in
//! a signal or another thread-local is dropped while the thread's locals are
//! torn down, after rinch-rtc's own tables are gone. Its drop used to reach
//! for those tables with `LocalKey::with`, which panics there, and a panic in
//! a thread-local destructor aborts the process. ui-zoo's in-page call test
//! found it: its signals hold the tracks it shows, and the test thread's exit
//! aborted. On an app's main thread that is an abort at exit.
//!
//! The test thread is the one that exits holding the track; a regression
//! aborts this test binary.
#![cfg(all(not(target_arch = "wasm32"), feature = "media"))]

mod common;

use std::cell::RefCell;

use rinch_rtc::media::{LocalTrack, VideoSourceOptions};

thread_local! {
    /// Touched before any rinch-rtc table, so it is destroyed after them.
    static HELD: RefCell<Vec<LocalTrack>> = const { RefCell::new(Vec::new()) };
}

#[test]
fn a_track_dropped_as_its_thread_exits_does_not_abort() {
    HELD.with(|h| h.borrow_mut().clear());
    common::init();
    let (track, writer) = LocalTrack::video_source(VideoSourceOptions::default()).unwrap();
    writer.push_rgba(&vec![90u8; 640 * 480 * 4], 640, 480);
    HELD.with(|h| h.borrow_mut().push(track));
}
