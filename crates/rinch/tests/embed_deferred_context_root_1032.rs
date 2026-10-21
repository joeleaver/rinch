//! Issue #1032: a deferred callback armed inside an embedded `RinchContext`
//! resolves that context's stores.
//!
//! An `Effect` re-enters the context root it was created in (#136). A timer,
//! a parked continuation (an `rinch-http` completion) and a closure on the
//! main-thread queue recorded only the document (#963): `RinchContext::update`
//! fires them with no context root pushed, so they resolved the thread-global
//! root `0`, and `use_store` there panicked with "Store not found". Measured on
//! #1032 by the review of #1033.
//!
//! Same harness rules as `multi_context.rs`: `RinchContext::new` registers the
//! creating thread as THE main thread process-wide, so every test body runs on
//! one shared worker thread.

#![cfg(any(feature = "gpu", feature = "embed"))]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rinch::embed::{RinchContext, RinchContextConfig};
use rinch::prelude::*;

type Job = Box<dyn FnOnce() + Send>;

fn on_ui_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    static SENDER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("rinch-test-ui".into())
            .spawn(move || {
                for job in rx {
                    job();
                }
            })
            .expect("spawn ui worker");
        Mutex::new(tx)
    });
    let (rtx, rrx) = mpsc::channel();
    let job: Job = Box::new(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let _ = rtx.send(r);
    });
    sender.lock().unwrap().send(job).unwrap();
    match rrx.recv().unwrap() {
        Ok(v) => v,
        Err(p) => std::panic::resume_unwind(p),
    }
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

fn pump_until(ctx: &mut RinchContext, done: impl Fn() -> bool) {
    let t = Instant::now();
    while !done() && t.elapsed() < Duration::from_secs(3) {
        ctx.update(&[]);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(done(), "the deferred callback never ran");
}

#[derive(Clone)]
struct MyStore(u32);

/// A timer and a queued main-thread closure armed at mount read the store
/// the context created, as an effect made in the same mount does (the
/// positive control). The host's `update()` is the only thing that fires them.
#[test]
fn deferred_callbacks_in_an_embed_context_read_its_store() {
    on_ui_thread(|| {
        let timer_seen: Rc<RefCell<Option<Option<u32>>>> = Rc::default();
        let queued_seen: Arc<Mutex<Option<bool>>> = Arc::default();
        let effect_seen: Rc<RefCell<Option<u32>>> = Rc::default();
        let (ts, qs, es) = (timer_seen.clone(), queued_seen.clone(), effect_seen.clone());
        let mut c = RinchContext::new(cfg(), move |__scope: &mut RenderScope| {
            create_store(MyStore(42));
            let ts = ts.clone();
            rinch_core::set_timeout(10, move || {
                *ts.borrow_mut() = Some(try_use_store::<MyStore>().map(|m| m.0));
            });
            let qs = qs.clone();
            rinch_core::queue_main_callback(Box::new(move || {
                *qs.lock().unwrap() = Some(try_use_store::<MyStore>().is_some_and(|m| m.0 == 42));
            }));
            let es = es.clone();
            rinch::reactive::Effect::new(move || {
                *es.borrow_mut() = try_use_store::<MyStore>().map(|m| m.0);
            });
            rsx! { div { style: "width: 100px; height: 100px;" } }
        });
        let (t2, q2) = (timer_seen.clone(), queued_seen.clone());
        pump_until(&mut c, move || {
            t2.borrow().is_some() && q2.lock().unwrap().is_some()
        });
        let timer = *timer_seen.borrow();
        let queued = *queued_seen.lock().unwrap();
        let effect = *effect_seen.borrow();
        drop(c);
        assert_eq!(effect, Some(42), "positive control: an effect resolves the context's store");
        assert_eq!(timer, Some(Some(42)), "the timer resolves the context's store");
        assert_eq!(queued, Some(true), "the queued closure resolves the context's store");
    });
}
