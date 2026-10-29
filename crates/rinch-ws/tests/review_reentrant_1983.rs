//! Review of #1983: callbacks that own handles, dropped by the registry at
//! points other than `WsHandle::drop`.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::cell::RefCell;
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::Duration;

use common::{PUMP, dispatcher, pump_until};
use rinch_core::{register_main_thread, set_cross_thread_dispatcher};
use rinch_ws::{WsHandle, connect};

fn refused_url() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("ws://127.0.0.1:{}/", l.local_addr().unwrap().port())
}

/// A callback that replaces itself while it runs: `invoke` does not put the
/// old one back, and the old one owns another handle. Its drop must not happen
/// inside the table's borrow.
#[test]
fn a_callback_replaced_while_it_runs_drops_its_handle_outside_the_borrow() {
    PUMP.set(Mutex::new(Vec::new())).ok();
    register_main_thread();
    set_cross_thread_dispatcher(dispatcher);

    let a: Rc<RefCell<Option<WsHandle>>> = Rc::new(RefCell::new(None));
    struct Flagged(Option<WsHandle>, Rc<std::cell::Cell<bool>>);
    impl Drop for Flagged {
        fn drop(&mut self) {
            drop(self.0.take());
            self.1.set(true);
        }
    }
    let dropped = Rc::new(std::cell::Cell::new(false));
    let owned = Flagged(
        Some(connect(refused_url()).expect("connect starts")),
        dropped.clone(),
    );
    let ws = connect(refused_url()).expect("connect starts");
    *a.borrow_mut() = Some(ws);
    let ran = Rc::new(RefCell::new(0));
    let (a2, ran2) = (a.clone(), ran.clone());
    a.borrow().as_ref().unwrap().on_error(move |_| {
        let _keep = &owned;
        *ran2.borrow_mut() += 1;
        // Replace this very callback; the old one (owning `owned`) is dropped
        // by `invoke` after it returns.
        a2.borrow().as_ref().unwrap().on_error(|_| {});
    });
    assert!(
        pump_until(|| *ran.borrow() > 0, Duration::from_secs(10)),
        "on_error never ran"
    );
    pump_until(|| false, Duration::from_millis(200));
    assert!(
        dropped.get(),
        "positive control: the replaced callback (and its handle) was dropped"
    );
    // Break the cycle.
    a.borrow_mut().take();
}
