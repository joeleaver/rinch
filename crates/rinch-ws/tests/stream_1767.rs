//! `WsHandle::into_stream` (issue #1767): a connection read by awaiting its
//! events inside a `spawn_local` task, against the in-process echo server.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::cell::RefCell;
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::Duration;

use common::{PUMP, dispatcher, pump_until, spawn_echo_server};
use rinch_core::{register_main_thread, set_cross_thread_dispatcher, spawn_local};
use rinch_ws::{WsEvent, WsMessage, connect};

fn name(event: &WsEvent) -> String {
    match event {
        WsEvent::Open => "open".into(),
        WsEvent::Message(WsMessage::Text(t)) => format!("text:{t}"),
        WsEvent::Message(WsMessage::Binary(b)) => format!("binary:{b:?}"),
        WsEvent::Close(c) => format!("close:{}", c.code),
        WsEvent::Error(_) => "error".into(),
    }
}

#[test]
fn a_task_awaits_a_connections_events_until_it_ends() {
    PUMP.set(Mutex::new(Vec::new())).ok();
    register_main_thread();
    set_cross_thread_dispatcher(dispatcher);

    // A round trip, then a close: the stream ends after the close.
    let url = spawn_echo_server();
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let l = log.clone();
    let task = spawn_local(async move {
        let mut ws = connect(url).unwrap().into_stream();
        let open = ws.next().await.unwrap();
        l.borrow_mut().push(name(&open));
        ws.send_text("hello");
        let echo = ws.next().await.unwrap();
        l.borrow_mut().push(name(&echo));
        ws.close();
        while let Some(event) = ws.next().await {
            l.borrow_mut().push(name(&event));
        }
        l.borrow_mut().push("end".into());
    });
    assert!(
        pump_until(|| task.is_finished(), Duration::from_secs(10)),
        "{:?}",
        log.borrow()
    );
    // 1005: the client closes with no status code, which the echo server returns.
    assert_eq!(*log.borrow(), ["open", "text:hello", "close:1005", "end"]);

    // A failed connect (nothing listens on the port): the error, then the end.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let l = log.clone();
    let task = spawn_local(async move {
        let mut ws = connect(format!("ws://127.0.0.1:{port}/"))
            .unwrap()
            .into_stream();
        while let Some(event) = ws.next().await {
            l.borrow_mut().push(name(&event));
        }
        l.borrow_mut().push("end".into());
    });
    assert!(
        pump_until(|| task.is_finished(), Duration::from_secs(10)),
        "{:?}",
        log.borrow()
    );
    assert_eq!(*log.borrow(), ["error", "end"]);
}
