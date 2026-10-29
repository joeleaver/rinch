//! `RinchContext::new` on wasm32 installs the browser's timer backend and task
//! scheduler, but not the poll drain (issue #1956, review of PR #1988).
//!
//! A test of `rinch` alone, with no `rinch-web` in the binary: the app this is
//! for depends on `rinch` only, and a `rinch-web` test would build `rinch`
//! without `embed`.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch --no-default-features --features embed \
//!     --target wasm32-unknown-unknown --test embed_web_runtime_1956
//! ```
//!
//! One `#[wasm_bindgen_test]`: the runtime is page-global, so the "before"
//! phase is true only for the first fixture of a page.
#![cfg(all(target_arch = "wasm32", feature = "embed"))]

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::reactive::{PollRate, poll_signal, unowned};
use rinch_core::set_timeout;
use rinch_core::task::{sleep, spawn_local};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Let the browser run its timers, microtasks and animation frames for `ms`.
async fn wait(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// Wait up to two seconds for `done`.
async fn wait_for(mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..200 {
        if done() {
            return true;
        }
        wait(10).await;
    }
    done()
}

fn cfg() -> RinchContextConfig {
    RinchContextConfig {
        width: 400,
        height: 300,
        scale_factor: 1.0,
        theme: None,
        fonts: Vec::new(),
    }
}

#[wasm_bindgen_test]
async fn a_rinch_context_installs_timers_and_tasks_but_not_the_poll_drain() {
    // 1. Positive control: no context and no install yet, so a timer armed
    //    now is never scheduled (and stays lost: the documented limit).
    let early = Rc::new(Cell::new(false));
    let e = early.clone();
    unowned(|| set_timeout(0, move || e.set(true)));
    wait(30).await;
    assert!(!early.get(), "positive control: no runtime, no timer");

    // 2. A context whose component arms a timer and spawns a sleeping task.
    //    `update()` is never called in this phase: nothing but the context's
    //    own install can schedule them.
    let fired = Rc::new(Cell::new(0u32));
    let slept = Rc::new(Cell::new(false));
    let (f, s) = (fired.clone(), slept.clone());
    let mut ctx = RinchContext::new(cfg(), move |scope: &mut RenderScope| -> NodeHandle {
        let f = f.clone();
        set_timeout(5, move || f.set(f.get() + 1));
        let s = s.clone();
        spawn_local(async move {
            sleep(Duration::from_millis(10)).await;
            s.set(true);
        });
        scope.create_element("div")
    });
    assert!(
        wait_for(|| fired.get() > 0).await,
        "a timer armed by a RinchContext's component fires with no update()"
    );
    assert!(
        wait_for(|| slept.get()).await,
        "a task spawned by a RinchContext's component runs (and sleeps) with no update()"
    );
    wait(30).await;
    assert_eq!(fired.get(), 1, "the timer fired once");
    assert!(!early.get(), "a timer armed before any install stays lost");

    // 3. Not the poll drain: with no `update()`, a timed poll is not sampled
    //    between frames; one `update()` samples it.
    let source = Rc::new(Cell::new(1u32));
    let src = source.clone();
    let polled = unowned(|| poll_signal(move || src.get(), PollRate::Millis(5)));
    source.set(7);
    wait(60).await;
    assert_eq!(
        polled.get(),
        1,
        "a RinchContext alone installs no between-frames poll drain"
    );
    ctx.update(&[]);
    assert_eq!(polled.get(), 7, "update() samples the poll");

    // 4. The full install on top: idempotent for timers (one backend, one
    //    fire per timer), and it adds the between-frames poll drain.
    rinch::install_web_runtime();
    rinch::install_web_runtime();
    let again = Rc::new(Cell::new(0u32));
    let a = again.clone();
    unowned(|| set_timeout(5, move || a.set(a.get() + 1)));
    source.set(9);
    assert!(
        wait_for(|| polled.get() == 9).await,
        "install_web_runtime adds the poll drain a context leaves out"
    );
    assert!(wait_for(|| again.get() > 0).await, "timers still fire");
    wait(30).await;
    assert_eq!(again.get(), 1, "one timer backend after both installs");
    drop(ctx);
}
