//! A worker that cannot start, spawned before any mount (issue #1772): the
//! failure is reported though rinch-web has not installed its task scheduler
//! yet. Its own file: it must run before any mount on the page.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch::worker::{Replies, Worker, WorkerEvent};
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

async fn wait(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

fn echo(text: String, replies: &Replies<String>) {
    replies.send(text);
}

#[wasm_bindgen_test]
async fn a_failed_start_before_any_mount_is_reported() {
    // Make the `Worker` constructor throw, as a page that forbids workers does.
    js_sys::Function::new_no_args(
        "globalThis.__rinchWorker = globalThis.Worker; \
         globalThis.Worker = function () { throw new Error('workers are off'); };",
    )
    .call0(&JsValue::NULL)
    .unwrap();
    let worker = Worker::<String, String>::spawn(echo);
    js_sys::Function::new_no_args("globalThis.Worker = globalThis.__rinchWorker;")
        .call0(&JsValue::NULL)
        .unwrap();

    let got: Rc<RefCell<Vec<WorkerEvent<String>>>> = Rc::default();
    let g = got.clone();
    worker.on_event(move |e| g.borrow_mut().push(e));
    wait(50).await;
    assert_eq!(got.borrow().len(), 1, "{:?}", got.borrow());
    assert!(
        matches!(&got.borrow()[0], WorkerEvent::Error(e) if e.contains("workers are off")),
        "{:?}",
        got.borrow()
    );
    assert!(worker.is_ended());

    // Positive control: with the constructor back, a worker starts and replies.
    let ok = Worker::<String, String>::spawn(echo);
    let got2: Rc<RefCell<Vec<WorkerEvent<String>>>> = Rc::default();
    let g = got2.clone();
    ok.on_event(move |e| g.borrow_mut().push(e));
    ok.send("hi".into());
    for _ in 0..500 {
        if !got2.borrow().is_empty() {
            break;
        }
        wait(10).await;
    }
    assert_eq!(*got2.borrow(), vec![WorkerEvent::Message("hi".to_string())]);
}
