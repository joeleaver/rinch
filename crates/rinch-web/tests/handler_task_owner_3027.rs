//! A task spawned from a component's click handler belongs to the component
//! that wrote the handler, not to one render of the component it was handed to
//! (issue #3027), in a browser.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test handler_task_owner_3027
//! ```
//!
//! The report's shape: a `Button` with `loading: {|| busy.get()}` whose
//! `onclick` sets `busy` and spawns a request. The write re-renders the Button
//! when the click's batch ends; the task used to be adopted by that render's
//! scope and dropped before its first poll, so the request was never sent. The
//! desktop twin, with the `for`-row case, is
//! `rinch-macros/tests/rsx_handler_task_owner_3027.rs`.
#![cfg(target_arch = "wasm32")]

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use rinch::prelude::*;
use rinch_core::element::ThemeProviderProps;
use wasm_bindgen::JsCast;
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

/// Sets its flag when dropped: the task's future went away.
struct Flag(Rc<Cell<bool>>);

impl Drop for Flag {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

/// What the click's task did.
#[derive(Clone, Default)]
struct Request {
    sent: Rc<Cell<u32>>,
    dropped: Rc<Cell<bool>>,
}

#[wasm_bindgen_test]
async fn a_task_from_a_click_survives_the_rerender_and_ends_with_its_component() {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();

    let busy = Signal::new(false);
    // How long the request takes; the second click's never finishes in time.
    let delay_ms = Rc::new(Cell::new(20u64));
    let first = Request::default();
    let second = Request::default();
    let clicks = Rc::new(Cell::new(0u32));
    let (f, s, d, c) = (
        first.clone(),
        second.clone(),
        delay_ms.clone(),
        clicks.clone(),
    );
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |__scope: &mut RenderScope| {
            rsx! {
                div {
                    Button {
                        loading: {|| busy.get()},
                        onclick: {
                            let (f, s, d, c) = (f.clone(), s.clone(), d.clone(), c.clone());
                            move || {
                                busy.set(true);
                                c.set(c.get() + 1);
                                let request = if c.get() == 1 { f.clone() } else { s.clone() };
                                let delay = d.get();
                                // Made outside the future, so a drop before its
                                // first poll is seen too.
                                let flag = Flag(request.dropped.clone());
                                spawn_local(async move {
                                    let _flag = flag;
                                    sleep(Duration::from_millis(delay)).await;
                                    request.sent.set(request.sent.get() + 1);
                                });
                            }
                        },
                        "Send"
                    }
                }
            }
        },
    );

    let button = || -> web_sys::HtmlElement {
        host.query_selector(".rinch-button")
            .unwrap()
            .expect("the Button is mounted")
            .dyn_into()
            .unwrap()
    };
    let before = button();
    before.click();
    assert_eq!(clicks.get(), 1, "positive control: the click reached rinch");
    assert!(busy.get(), "positive control: the handler ran");
    assert!(
        !before.is_same_node(Some(&button())),
        "positive control: the click's write re-rendered the Button"
    );
    assert!(
        wait_for(|| first.sent.get() == 1).await,
        "the request the click spawned was never sent (dropped: {})",
        first.dropped.get()
    );

    // (b) Unmounting the component that wrote the handler still drops a task
    // in flight.
    busy.set(false);
    delay_ms.set(60_000);
    button().click();
    assert_eq!(clicks.get(), 2, "positive control: the second click");
    wait(20).await;
    assert!(
        !second.dropped.get(),
        "the re-render after the second click leaves its task running"
    );
    root.unmount();
    host.remove();
    assert!(
        second.dropped.get(),
        "unmounting drops the task the handler spawned"
    );
    assert_eq!(second.sent.get(), 0, "and it never finished");
}
