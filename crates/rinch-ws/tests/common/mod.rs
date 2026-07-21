//! Shared harness for the native integration tests.
//!
//! Outside a running rinch app there is no cross-thread dispatcher, so these tests
//! stand one up: the test thread registers itself as the main thread, installs a
//! dispatcher that parks main-thread work in a global queue, and pumps that queue
//! — the role the winit event loop plays in the real app.
//!
//! `register_main_thread` is process-global, so each integration-test *file* gets
//! its own process and registers exactly once. Two tests in one binary would race
//! (cargo runs them on separate threads) and the loser would never see its events.
#![allow(dead_code)]
#![cfg(not(target_arch = "wasm32"))]

use std::net::TcpListener;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use tungstenite::Message;

pub type MainJob = Box<dyn FnOnce() + Send>;

/// Queue of closures dispatched to the "main" (test) thread by the rinch runtime
/// stand-in. Drained by [`drain_pump`].
pub static PUMP: OnceLock<Mutex<Vec<MainJob>>> = OnceLock::new();

/// The cross-thread dispatcher installed into rinch-core: park work for the main
/// thread. Must be a plain `fn` (no captures), hence the global queue.
pub fn dispatcher(f: MainJob) {
    PUMP.get()
        .expect("pump initialized")
        .lock()
        .unwrap()
        .push(f);
}

pub fn drain_pump() {
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
pub fn pump_until(mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
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
pub fn spawn_echo_server() -> String {
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
