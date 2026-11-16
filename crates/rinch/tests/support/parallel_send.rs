//! The scenario of issue #2778, shared by `testing_parallel_send_2778.rs` and
//! `testing_parallel_send_registered_2778.rs`: two `TestApp`s alive at once
//! on two threads, each writing its own component's signal with
//! `Signal::update_send` from its own thread and from a worker thread of its
//! own. Each app must see only its own writes, applied on its own thread.
//!
//! The two apps run identical components, so their signals have the same
//! slot id in their two thread-local stores: a write run by the wrong app's
//! turn lands in that app's signal of the same id, which is how deskrinch saw
//! it (test B's settle ran test A's change, and A's effect, on B's thread).
//!
//! The order is forced with barriers, so a shared queue shows up every time:
//! both apps queue their writes, then A turns first (and, with one queue for
//! the process, drains B's writes too), then B turns.

use rinch::prelude::*;
use rinch::testing::TestApp;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Barrier, Mutex};

/// What each effect run saw: the app, the thread it ran on, the value.
pub type Seen = Arc<Mutex<Vec<(&'static str, String, Vec<String>)>>>;

fn thread_name() -> String {
    std::thread::current().name().unwrap_or("?").to_owned()
}

/// One app's half of the scenario, on the current thread.
fn run_app(name: &'static str, seen: Seen, queued: Arc<Barrier>, a_turned: Arc<Barrier>) {
    let signal: Rc<Cell<Option<Signal<Vec<String>>>>> = Rc::new(Cell::new(None));
    let slot = signal.clone();
    let log = seen.clone();
    let mut app = TestApp::mount(move |__scope: &mut RenderScope| {
        let items = Signal::new(Vec::<String>::new());
        slot.set(Some(items));
        let log = log.clone();
        rinch::Effect::new(move || {
            let value = items.get();
            if !value.is_empty() {
                log.lock().unwrap().push((name, thread_name(), value));
            }
        });
        rsx! { div { id: "items", {|| items.get().join(",")} } }
    });
    let items = signal.get().expect("the component ran");

    // From the app's own thread, and from a worker of its own.
    let own = format!("{name}-own");
    items.update_send(move |v| v.push(own));
    let worker = format!("{name}-worker");
    std::thread::Builder::new()
        .name(format!("{name}-worker"))
        .spawn(move || items.update_send(move |v| v.push(worker)))
        .unwrap()
        .join()
        .unwrap();

    queued.wait();
    if name == "a" {
        app.settle();
        a_turned.wait();
    } else {
        a_turned.wait();
        app.settle();
    }

    assert_eq!(
        app.text("#items"),
        format!("{name}-own,{name}-worker"),
        "app {name} shows its own two writes and nothing else"
    );
}

/// Run the scenario and check what every effect run saw.
pub fn two_apps_each_see_only_their_own_sends() {
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let queued = Arc::new(Barrier::new(2));
    let a_turned = Arc::new(Barrier::new(2));
    let threads: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|name| {
            let (seen, queued, a_turned) = (seen.clone(), queued.clone(), a_turned.clone());
            std::thread::Builder::new()
                .name(format!("app-{name}"))
                .spawn(move || run_app(name, seen, queued, a_turned))
                .unwrap()
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join()).collect();

    let seen = seen.lock().unwrap().clone();
    for (app, thread, value) in &seen {
        assert_eq!(
            thread,
            &format!("app-{app}"),
            "app {app}'s effect ran on its own thread (saw {value:?}); all runs: {seen:?}"
        );
        for item in value {
            assert!(
                item.starts_with(app),
                "app {app}'s signal holds only its own writes; all runs: {seen:?}"
            );
        }
    }
    for name in ["a", "b"] {
        assert!(
            seen.iter().any(|(app, _, v)| *app == name && v.len() == 2),
            "app {name}'s effect saw both of its writes; all runs: {seen:?}"
        );
    }
    for result in results {
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}
