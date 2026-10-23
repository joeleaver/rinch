//! A connect that fails (nothing listening) reports an error and then a close,
//! as a browser does (`error`, then `close` with 1006). Native used to report
//! the error alone, so an app that reconnects from `on_close` stopped after the
//! first refused attempt on desktop and retried on the web.
//!
//! Its own binary: `register_main_thread` is per process (see `common`).
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::cell::RefCell;
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::Duration;

use common::{PUMP, dispatcher, pump_until};
use rinch_core::{register_main_thread, set_cross_thread_dispatcher};
use rinch_ws::{WsError, connect};

#[test]
fn a_refused_connect_reports_an_error_then_one_abnormal_close() {
    PUMP.set(Mutex::new(Vec::new())).ok();
    register_main_thread();
    set_cross_thread_dispatcher(dispatcher);

    // A port nothing listens on: bind, read the port, let it go.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };

    let ws = connect(format!("ws://127.0.0.1:{port}/")).expect("connect starts");
    let events: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let e = events.clone();
    ws.on_error(move |err| {
        let kind = match err {
            WsError::Connect(_) => "connect",
            _ => "other",
        };
        e.borrow_mut().push(format!("error:{kind}"));
    });
    let c = events.clone();
    ws.on_close(move |cl| c.borrow_mut().push(format!("close:{}", cl.code)));

    assert!(
        pump_until(
            || events.borrow().iter().any(|e| e.starts_with("close")),
            Duration::from_secs(10)
        ),
        "a failed connect must be followed by a close; got {:?}",
        events.borrow()
    );
    // Give any duplicate a chance to arrive.
    pump_until(|| false, Duration::from_millis(200));
    assert_eq!(
        *events.borrow(),
        vec!["error:connect".to_string(), "close:1006".to_string()]
    );
}
