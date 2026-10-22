//! `rinch::worker` in a browser (issue #1772): a Web Worker running a second
//! instance of this test's own wasm module.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test worker_1772
//! ```
//!
//! The desktop twin is `crates/rinch/tests/worker_1772.rs`.
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch::worker::{Replies, Worker, WorkerEvent, is_worker};
use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_core::task::spawn_local;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

async fn wait(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// Wait up to five seconds for `done` (a worker instantiates the module first).
async fn wait_for(mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..500 {
        if done() {
            return true;
        }
        wait(10).await;
    }
    done()
}

/// Mount a root, which installs rinch-web's task scheduler (and is the
/// positive control that rinch-web's own mount runs on the page).
fn mount_page() -> rinch_web::RootHandle {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        |scope: &mut RenderScope| {
            let p = scope.create_element("p");
            p.set_attribute("id", "worker-page");
            p
        },
    );
    assert!(
        document().get_element_by_id("worker-page").is_some(),
        "positive control: the page mounted"
    );
    root
}

fn collect(worker: &Worker<String, String>) -> Rc<RefCell<Vec<WorkerEvent<String>>>> {
    let got: Rc<RefCell<Vec<WorkerEvent<String>>>> = Rc::default();
    let g = got.clone();
    worker.on_event(move |e| g.borrow_mut().push(e));
    got
}

fn msg(s: &str) -> WorkerEvent<String> {
    WorkerEvent::Message(s.to_string())
}

// --- handlers (run in the worker's instance) ------------------------------

thread_local! {
    static SEEN: Cell<u32> = const { Cell::new(0) };
}

fn numbered(text: String, replies: &Replies<String>) {
    let n = SEEN.with(|s| {
        s.set(s.get() + 1);
        s.get()
    });
    replies.send(format!("{n}:{}:{}", text.to_uppercase(), is_worker()));
}

/// Says it started, busy-waits `ms`, then announces it finished on a
/// `BroadcastChannel` — a path that does not go through rinch, so it shows
/// whether the handler really ran to the end.
fn busy(ms: String, replies: &Replies<String>) {
    replies.send("started".into());
    let until = js_sys::Date::now() + ms.parse::<f64>().unwrap();
    while js_sys::Date::now() < until {}
    let channel = web_sys::BroadcastChannel::new("rinch-1772").unwrap();
    channel.post_message(&JsValue::from_str(&ms)).unwrap();
    channel.close();
}

/// Calls a whole-page mount inside the worker, where there is no `document`.
fn mounts(_: String, replies: &Replies<String>) {
    rinch_web::mount(ThemeProviderProps::default(), |scope: &mut RenderScope| {
        scope.create_element("div")
    });
    replies.send("mount returned".into());
}

fn panics(text: String, replies: &Replies<String>) {
    if text == "boom" {
        panic!("handler gave up");
    }
    replies.send(text);
}

// --- tests ----------------------------------------------------------------

#[wasm_bindgen_test]
async fn a_worker_replies_in_order_with_its_own_state() {
    let _root = mount_page();
    assert!(!is_worker(), "the page is not a worker");

    let worker = Worker::<String, String>::spawn(numbered);
    let got = collect(&worker);
    for m in ["a", "b", "c"] {
        worker.send(m.into());
    }
    assert!(
        wait_for(|| got.borrow().len() == 3).await,
        "{:?}",
        got.borrow()
    );
    assert_eq!(
        *got.borrow(),
        vec![msg("1:A:true"), msg("2:B:true"), msg("3:C:true")]
    );

    // Awaited through `spawn_local`.
    let awaited = Rc::new(Worker::<String, String>::spawn(numbered));
    let result = Rc::new(RefCell::new(None));
    let (r, w) = (result.clone(), awaited.clone());
    spawn_local(async move {
        w.send("x".into());
        let first = w.next().await;
        w.send("y".into());
        *r.borrow_mut() = Some((first, w.next().await));
    });
    assert!(wait_for(|| result.borrow().is_some()).await);
    assert_eq!(
        result.borrow_mut().take().unwrap(),
        (Some(msg("1:X:true")), Some(msg("2:Y:true"))),
        "a new worker is a new instance: its count starts again"
    );
}

#[wasm_bindgen_test]
async fn terminate_stops_a_spinning_handler() {
    let _root = mount_page();
    let finished: Rc<RefCell<Vec<String>>> = Rc::default();
    let channel = web_sys::BroadcastChannel::new("rinch-1772").unwrap();
    let f = finished.clone();
    let on_message =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            f.borrow_mut().push(e.data().as_string().unwrap())
        });
    channel.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    // Positive control: left alone, a busy handler finishes and says so.
    let control = Worker::<String, String>::spawn(busy);
    control.send("300".into());

    let killed = Rc::new(Worker::<String, String>::spawn(busy));
    let got = collect(&killed);
    killed.send("4000".into());
    assert!(
        wait_for(|| got.borrow().len() == 1).await,
        "the handler started"
    );
    let ended = Rc::new(Cell::new(None));
    let (e, k) = (ended.clone(), killed.clone());
    // A pending `next` resolves to `None` on terminate (events go to the
    // callback, so the stream holds none).
    spawn_local(async move { e.set(Some(k.next().await.is_none())) });
    wait(20).await;
    killed.terminate();
    assert!(killed.is_ended());
    assert!(wait_for(|| ended.get().is_some()).await);
    assert_eq!(ended.get(), Some(true));

    assert!(
        wait_for(|| finished.borrow().iter().any(|m| m == "300")).await,
        "positive control: the untouched worker finished"
    );
    // Past the point where the killed handler would have finished. Chrome
    // stops a worker that does not yield about two seconds after `terminate`
    // (its forcible-termination delay; a 1.5 s busy loop runs to its end
    // there, measured), Firefox at once.
    wait(4500).await;
    assert_eq!(
        *finished.borrow(),
        vec!["300".to_string()],
        "the terminated handler never reached its end"
    );
    assert_eq!(*got.borrow(), vec![msg("started")]);
    channel.close();
}

#[wasm_bindgen_test]
async fn a_whole_page_mount_returns_early_in_a_worker() {
    let _root = mount_page();
    let worker = Worker::<String, String>::spawn(mounts);
    let got = collect(&worker);
    worker.send("go".into());
    assert!(wait_for(|| !got.borrow().is_empty()).await);
    assert_eq!(*got.borrow(), vec![msg("mount returned")]);
}

#[wasm_bindgen_test]
async fn a_panic_ends_the_worker() {
    let _root = mount_page();
    let worker = Worker::<String, String>::spawn(panics);
    let got = collect(&worker);
    worker.send("ok".into());
    worker.send("boom".into());
    worker.send("after".into());
    assert!(
        wait_for(|| got.borrow().len() == 2).await,
        "{:?}",
        got.borrow()
    );
    wait(100).await;
    let got = got.borrow();
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(got[0], msg("ok"));
    assert!(matches!(&got[1], WorkerEvent::Error(_)), "{:?}", got[1]);
    assert!(worker.is_ended());
}

fn uses_spawn_local(_: String, _: &Replies<String>) {
    spawn_local(async {});
}

fn spawns_a_worker(_: String, replies: &Replies<String>) {
    let inner = Worker::<String, String>::spawn(numbered);
    replies.send(format!("ended:{}", inner.is_ended()));
}

#[wasm_bindgen_test]
async fn spawn_local_in_a_handler_refuses_loudly() {
    let _root = mount_page();
    let worker = Worker::<String, String>::spawn(uses_spawn_local);
    let got = collect(&worker);
    worker.send("go".into());
    assert!(wait_for(|| !got.borrow().is_empty()).await);
    assert!(
        matches!(&got.borrow()[0], WorkerEvent::Error(_)),
        "{:?}",
        got.borrow()
    );
    assert!(worker.is_ended(), "a panic ends the worker");
}

#[wasm_bindgen_test]
async fn a_worker_spawns_no_worker() {
    let _root = mount_page();
    let worker = Worker::<String, String>::spawn(spawns_a_worker);
    let got = collect(&worker);
    worker.send("go".into());
    assert!(wait_for(|| !got.borrow().is_empty()).await);
    assert_eq!(*got.borrow(), vec![msg("ended:true")]);
}
