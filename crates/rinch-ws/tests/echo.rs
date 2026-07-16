//! Native end-to-end test: drive a real `rinch-ws` connection against an
//! in-process tungstenite echo server, exercising the full wire path
//! (connect → open → send → receive → close) and the worker→main-thread event
//! dispatch.
//!
//! Outside a running rinch app there is no cross-thread dispatcher, so this test
//! stands one up: it registers itself as the main thread, installs a dispatcher
//! that parks main-thread work in a global queue, and pumps that queue from the
//! test thread — the role the winit event loop plays in the real app.
#![cfg(not(target_arch = "wasm32"))]

use std::cell::{Cell, RefCell};
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use rinch_core::{register_main_thread, set_cross_thread_dispatcher};
use rinch_ws::{WsMessage, connect};
use tungstenite::Message;

type MainJob = Box<dyn FnOnce() + Send>;

/// Queue of closures dispatched to the "main" (test) thread by the rinch runtime
/// stand-in. Drained by [`drain_pump`].
static PUMP: OnceLock<Mutex<Vec<MainJob>>> = OnceLock::new();

/// The cross-thread dispatcher installed into rinch-core: park work for the main
/// thread. Must be a plain `fn` (no captures), hence the global queue.
fn dispatcher(f: MainJob) {
    PUMP.get()
        .expect("pump initialized")
        .lock()
        .unwrap()
        .push(f);
}

fn drain_pump() {
    let jobs: Vec<_> = PUMP
        .get()
        .expect("pump initialized")
        .lock()
        .unwrap()
        .drain(..)
        .collect();
    for job in jobs {
        job();
    }
}

/// Pump the main-thread queue until `cond` holds or `timeout` elapses.
fn pump_until(mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        drain_pump();
        if cond() {
            return true;
        }
        if start.elapsed() >= timeout {
            drain_pump();
            return cond();
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// Spawn a WebSocket echo server on an ephemeral port; returns its `ws://` URL.
fn spawn_echo_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let mut ws = match tungstenite::accept(stream) {
            Ok(ws) => ws,
            Err(_) => return,
        };
        loop {
            match ws.read() {
                Ok(Message::Text(t)) => {
                    if ws.send(Message::Text(t)).is_err() {
                        break;
                    }
                }
                Ok(Message::Binary(b)) => {
                    if ws.send(Message::Binary(b)).is_err() {
                        break;
                    }
                }
                Ok(Message::Close(_)) => {
                    let _ = ws.close(None);
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    format!("ws://{addr}/")
}

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
