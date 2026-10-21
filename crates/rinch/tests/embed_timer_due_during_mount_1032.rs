//! Issue #1032 (CI failure of #1663): a timer armed at an embedded context's
//! mount that comes due before the mount finishes still fires.
//!
//! Its own test binary on purpose: `RinchContext::new` registers the main
//! thread process-wide, so in a binary where another test has already created
//! a context the bug cannot show.

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;

fn cfg() -> RinchContextConfig {
    RinchContextConfig {
        width: 400,
        height: 300,
        scale_factor: 1.0,
        theme: None,
        fonts: Vec::new(),
    }
}

fn pump_until(ctx: &mut RinchContext, done: impl Fn() -> bool) {
    let t = Instant::now();
    while !done() && t.elapsed() < Duration::from_secs(3) {
        ctx.update(&[]);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(done(), "the deferred callback never ran");
}

/// A timer armed at mount that comes due **during** the mount still fires.
///
/// `RinchContext::new` used to register its thread as the main thread only
/// after mounting. While no main thread is registered `is_main_thread()`
/// answers `true` on every thread, so a timer due before the mount finished
/// had its `run_on_main_thread` hop run inline on the `rinch-timer` thread,
/// where `resume_main_callback` found nothing parked: the callback was lost.
/// CI's debug mount takes longer than the 10 ms timer of
/// `embed_deferred_context_root_1032.rs`, which is how that test failed there
/// and passed on a faster host. A 0 ms timer comes due
/// at once, whatever the host's speed.
#[test]
fn a_timer_due_during_the_mount_still_fires() {
    let fired: Rc<RefCell<bool>> = Rc::default();
    let f = fired.clone();
    let mut c = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
        let f = f.clone();
        rinch_core::set_timeout(0, move || *f.borrow_mut() = true);
        // Give the timer thread time to come due before the mount ends.
        std::thread::sleep(Duration::from_millis(50));
        rsx! { div { style: "width: 100px; height: 100px;" } }
    });
    let f2 = fired.clone();
    pump_until(&mut c, move || *f2.borrow());
    drop(c);
}
