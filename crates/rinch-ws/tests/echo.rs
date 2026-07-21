//! Native end-to-end test: drive a real `rinch-ws` connection against an
//! in-process tungstenite echo server, exercising the full wire path
//! (connect → open → send → receive → close) and the worker→main-thread event
//! dispatch.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Mutex;
use std::time::Duration;

use common::{PUMP, dispatcher, pump_until, spawn_echo_server};
use rinch_core::{register_main_thread, set_cross_thread_dispatcher};
use rinch_ws::{WsMessage, connect};

#[test]
fn connect_send_receive_close() {
    // Stand in for the rinch runtime: this thread is "main"; dispatched work is
    // pumped by us.
    PUMP.set(Mutex::new(Vec::new())).ok();
    register_main_thread();
    set_cross_thread_dispatcher(dispatcher);

    let url = spawn_echo_server();

    let opened = Rc::new(Cell::new(false));
    let messages: Rc<RefCell<Vec<WsMessage>>> = Rc::new(RefCell::new(Vec::new()));
    let closed = Rc::new(Cell::new(false));

    let ws = connect(&url).expect("connect starts");

    let o = opened.clone();
    ws.on_open(move || o.set(true));
    let m = messages.clone();
    ws.on_message(move |msg| m.borrow_mut().push(msg));
    let c = closed.clone();
    ws.on_close(move |_| c.set(true));

    assert!(
        pump_until(|| opened.get(), Duration::from_secs(10)),
        "connection should open"
    );

    // Text round-trip.
    ws.send_text("hello");
    assert!(
        pump_until(|| messages.borrow().len() == 1, Duration::from_secs(10)),
        "text frame should echo back"
    );

    // Binary round-trip.
    ws.send_bytes(vec![1, 2, 3, 4]);
    assert!(
        pump_until(|| messages.borrow().len() == 2, Duration::from_secs(10)),
        "binary frame should echo back"
    );

    assert_eq!(
        *messages.borrow(),
        vec![
            WsMessage::Text("hello".to_string()),
            WsMessage::Binary(vec![1, 2, 3, 4]),
        ]
    );

    // Graceful close fires on_close.
    ws.close();
    assert!(
        pump_until(|| closed.get(), Duration::from_secs(10)),
        "on_close should fire after close()"
    );
}
