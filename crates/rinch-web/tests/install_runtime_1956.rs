//! The web runtime without a page (issue #1956): `rinch_web::install_runtime`
//! gives `set_timeout`, `spawn_local` and `poll_signal` their browser
//! scheduling with no root mounted, for an app that owns the page itself (a
//! game drawing its HUD through `rinch::embed::RinchContext`, say).
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test install_runtime_1956
//! ```
//!
//! One `#[wasm_bindgen_test]`, and nothing in this file mounts before the end:
//! the runtime is page-global, so the "before" phase is only true for the
//! first fixture of a page.
#![cfg(target_arch = "wasm32")]

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_core::reactive::{PollRate, Signal, poll_signal, unowned};
use rinch_core::set_timeout;
use rinch_core::task::{sleep, spawn_local};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Let the browser run its timers and microtasks for `ms`.
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

/// What a mount would add to the page: children of `<body>` and `<head>`.
fn page_shape() -> (u32, u32) {
    let doc = document();
    (
        doc.body().unwrap().child_element_count(),
        doc.head().unwrap().child_element_count(),
    )
}

#[wasm_bindgen_test]
async fn install_runtime_drives_timers_tasks_and_polls_with_no_root() {
    // 1. Positive control, before the install: nothing schedules a timer or
    //    polls a task, so neither runs.
    let early_timer = Rc::new(Cell::new(false));
    let t = early_timer.clone();
    unowned(|| set_timeout(0, move || t.set(true)));
    let early_task = Rc::new(Cell::new(false));
    let e = early_task.clone();
    spawn_local(async move { e.set(true) });
    wait(30).await;
    assert!(
        !early_timer.get() && !early_task.get(),
        "positive control: with no runtime, no timer fires and no task runs"
    );

    // 2. The install, twice (idempotent), touching nothing in the page.
    let shape = page_shape();
    rinch_web::install_runtime();
    rinch_web::install_runtime();
    assert_eq!(
        page_shape(),
        shape,
        "install_runtime must not build any DOM"
    );

    // The task spawned before the install is picked up by it.
    assert!(
        wait_for(|| early_task.get()).await,
        "a task spawned before install_runtime runs once it is installed"
    );

    // 3. A timer armed now fires, once.
    let fired = Rc::new(Cell::new(0u32));
    let f = fired.clone();
    unowned(|| set_timeout(5, move || f.set(f.get() + 1)));
    assert!(wait_for(|| fired.get() > 0).await, "set_timeout fires");
    wait(30).await;
    assert_eq!(fired.get(), 1, "one install's worth of timer backend");

    // 4. A task that sleeps (on the timer backend) and resumes.
    let slept = Rc::new(Cell::new(false));
    let s = slept.clone();
    spawn_local(async move {
        sleep(Duration::from_millis(10)).await;
        s.set(true);
    });
    assert!(
        wait_for(|| slept.get()).await,
        "spawn_local + sleep complete"
    );

    // 5. A polled signal is sampled.
    let source = Rc::new(Cell::new(1u32));
    let src = source.clone();
    let polled = unowned(|| poll_signal(move || src.get(), PollRate::Millis(5)));
    source.set(7);
    assert!(
        wait_for(|| polled.get() == 7).await,
        "poll_signal is sampled"
    );

    // 6. A mount after the install still works, and reinstalls nothing twice.
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let count = Signal::new(1u32);
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let p = scope.create_element("p");
            let t = scope.create_text("");
            p.append_child(&t);
            scope.create_effect(move || t.set_text(&count.get().to_string()));
            p
        },
    );
    let after = Rc::new(Cell::new(false));
    let a = after.clone();
    unowned(|| set_timeout(0, move || count.set(2)));
    unowned(|| set_timeout(5, move || a.set(true)));
    assert!(wait_for(|| after.get()).await, "timers fire after a mount");
    assert_eq!(host.text_content().as_deref(), Some("2"));
    root.unmount();
}
