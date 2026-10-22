//! `spawn_local` in a browser (issue #1767): a task polled in microtasks once a
//! root is mounted, `sleep` on `window.setTimeout`, `fetch_async` on the Fetch
//! API, and the task dropped when the component that spawned it unmounts.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test spawn_local_1767
//! ```
//!
//! One `#[wasm_bindgen_test]`: its first phase spawns before any mount, which
//! only the first fixture of a page can do. The desktop twin is
//! `rinch-core/tests/task_1767.rs`.
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_core::reactive::{Signal, unowned};
use rinch_core::task::{sleep, spawn_local};
use rinch_http::{Request, fetch_async};
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

fn host() -> web_sys::Element {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    host
}

#[wasm_bindgen_test]
async fn tasks_run_on_microtasks_await_timers_and_fetches_and_end_with_their_component() {
    // 1. Spawned before the first mount: no scheduler yet, and nothing in a
    //    browser drains the main-thread queue. The mount must pick it up.
    let early = Rc::new(Cell::new(false));
    let e = early.clone();
    spawn_local(async move { e.set(true) });
    wait(20).await;
    assert!(!early.get(), "positive control: no mount, no poll");

    // 2. A component whose task sleeps, fetches, and writes what it read into
    //    the DOM through a signal.
    let host_a = host();
    let root = rinch_web::mount_into(
        &host_a,
        ThemeProviderProps::default(),
        |scope: &mut RenderScope| {
            let text = Signal::new(String::from("waiting"));
            let p = scope.create_element("p");
            let t = scope.create_text("");
            p.append_child(&t);
            scope.create_effect(move || t.set_text(&text.get()));
            spawn_local(async move {
                sleep(Duration::from_millis(15)).await;
                text.set("slept".into());
                let response = fetch_async(Request::get("data:text/plain,fetched"))
                    .await
                    .unwrap();
                text.set(response.text());
            });
            p
        },
    );
    assert!(
        wait_for(|| early.get()).await,
        "the mount schedules a task spawned before it"
    );
    let shown = || host_a.text_content().unwrap_or_default();
    assert_eq!(shown(), "waiting", "nothing polled inside the render");
    assert!(wait_for(|| shown() == "fetched").await, "got {:?}", shown());
    root.unmount();

    // 3. Unmount drops a task its component spawned; `unowned` opts out.
    let owned_ran = Rc::new(Cell::new(false));
    let unowned_ran = Rc::new(Cell::new(false));
    let dropped = Rc::new(RefCell::new(false));
    struct Flag(Rc<RefCell<bool>>);
    impl Drop for Flag {
        fn drop(&mut self) {
            *self.0.borrow_mut() = true;
        }
    }
    let host_b = host();
    let (o, u, d) = (owned_ran.clone(), unowned_ran.clone(), dropped.clone());
    let root = rinch_web::mount_into(
        &host_b,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            spawn_local(async move {
                let _flag = Flag(d);
                sleep(Duration::from_millis(40)).await;
                o.set(true);
            });
            unowned(|| {
                spawn_local(async move {
                    sleep(Duration::from_millis(40)).await;
                    u.set(true);
                })
            });
            scope.create_element("div")
        },
    );
    wait(5).await;
    root.unmount();
    assert!(
        *dropped.borrow(),
        "the owned task's future is dropped at unmount"
    );
    assert!(
        wait_for(|| unowned_ran.get()).await,
        "the unowned task runs on"
    );
    assert!(!owned_ran.get(), "the owned task never resumed");

    host_a.remove();
    host_b.remove();
}
