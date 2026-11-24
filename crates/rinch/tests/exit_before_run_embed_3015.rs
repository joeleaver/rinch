//! Issue #3015 in a process that makes a `RinchContext` before it runs (an
//! app that renders offscreen first, then calls `App::try_run`): an exit
//! asked for before the context, or between the context and the run, is
//! kept for the run, which returns `Ok(())` at once without opening a
//! window. An embed host drops every other request from outside a context
//! (#2801), and used to drop the exit too, so the app ran on until killed.
//!
//! Its own test binary, so no desktop runtime is made in the process before
//! the runs. No display is needed: each run ends before it connects to one.
//!     cargo test -p rinch --features desktop,embed --test exit_before_run_embed_3015

#![cfg(all(feature = "embed", feature = "windowing"))]

use std::sync::atomic::{AtomicBool, Ordering};

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;
use rinch::shell::rinch_runtime::__native_events_queued as queued;

static MOUNTED: AtomicBool = AtomicBool::new(false);

#[component]
fn app() -> NodeHandle {
    MOUNTED.store(true, Ordering::SeqCst);
    __scope.create_element("div")
}

/// An offscreen render: a context made and dropped on a thread of its own
/// (`RinchContext::new` registers its thread as the main thread).
fn render_offscreen() {
    std::thread::spawn(|| {
        let cfg = RinchContextConfig {
            width: 64,
            height: 64,
            scale_factor: 1.0,
            theme: None,
            fonts: Vec::new(),
        };
        let _ctx = RinchContext::new(cfg, |__scope: &mut RenderScope| {
            __scope.create_element("div")
        });
    })
    .join()
    .unwrap();
}

fn exit_from_another_thread() {
    std::thread::spawn(rinch::exit).join().unwrap();
}

#[test]
fn an_exit_before_or_after_a_context_ends_the_run_that_comes() {
    // Before the context: kept through it, the other requests dropped.
    std::thread::spawn(|| rinch::window::set_window_title("kept, then dropped"))
        .join()
        .unwrap();
    exit_from_another_thread();
    render_offscreen();
    assert_eq!(
        queued(),
        1,
        "the embed host kept more or less than the exit"
    );

    let result = App::new(app).try_run();
    assert!(result.is_ok(), "the run ended with {result:?}");
    assert!(
        !MOUNTED.load(Ordering::SeqCst),
        "a run asked to exit was built"
    );
    assert_eq!(queued(), 0, "the exit was not taken");

    // Between a context and the run.
    render_offscreen();
    exit_from_another_thread();
    assert_eq!(queued(), 1, "the embed host dropped the exit");

    let result = App::new(app).try_run();
    assert!(result.is_ok(), "the run ended with {result:?}");
    assert!(
        !MOUNTED.load(Ordering::SeqCst),
        "a run asked to exit was built"
    );
    assert_eq!(queued(), 0, "the exit was not taken");
}
