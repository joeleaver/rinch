//! Issue #3015: `rinch::exit()` asked for before the app runs — from a thread
//! that watches for `SIGTERM`, started before `App::try_run`, as deskrinch's
//! components do — is kept, and the run returns `Ok(())` at once without
//! opening a window. It used to be dropped (#2801 dropped every request sent
//! while no desktop runtime existed), and the app ran on until killed.
//!
//! Its own test binary, so no desktop runtime and no `RinchContext` is made
//! in the process before the run. No display is needed: the run ends before
//! it connects to one.

#![cfg(feature = "windowing")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rinch::prelude::*;
use rinch::shell::rinch_runtime::__native_events_queued as queued;

static MOUNTED: AtomicBool = AtomicBool::new(false);

#[component]
fn app() -> NodeHandle {
    MOUNTED.store(true, Ordering::SeqCst);
    __scope.create_element("div")
}

#[test]
fn an_exit_before_the_run_ends_it_at_once() {
    // A process that has not run yet keeps what it asks for, bounded however
    // much it asks (#2801's concern, for a process that never runs).
    std::thread::spawn(|| {
        for i in 0..5000 {
            rinch::window::set_window_title(&format!("title {i}"));
            rinch::windows::hide_current_window();
            rinch::windows::show_current_window();
        }
    })
    .join()
    .unwrap();
    let kept = queued();
    assert!(
        (1..=1024).contains(&kept),
        "{kept} requests kept for a runtime that has not been made"
    );

    // The signal-watching thread's exit, before the run.
    std::thread::spawn(rinch::exit).join().unwrap();

    let started = Instant::now();
    let result = App::new(app).try_run();
    let took = started.elapsed();

    assert!(result.is_ok(), "the run ended with {result:?}");
    assert!(
        took < Duration::from_secs(2),
        "the run took {took:?} to answer an exit asked for before it"
    );
    assert!(
        !MOUNTED.load(Ordering::SeqCst),
        "the app was built (a window opened) for a run asked to exit"
    );
    assert_eq!(queued(), 0, "the exited run's requests are still queued");
}
