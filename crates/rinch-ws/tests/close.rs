//! Native close-semantics test, in its own binary so it gets its own process (and
//! thus its own `register_main_thread`) — see `common`.
//!
//! All three scenarios share one test function on purpose: `register_main_thread`
//! is per-process, so two `#[test]`s in one binary would race for the main-thread
//! role and the loser would never see its events.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::cell::{Cell, RefCell};
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use common::{PUMP, dispatcher, pump_until};
use rinch_core::{register_main_thread, set_cross_thread_dispatcher};
use rinch_ws::connect;
use tungstenite::Message;
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;

/// A peer that echoes until it sees a close, waits `ack_delay`, then completes the
/// closing handshake with a **status-less** close frame (`ws.close(None)`).
fn peer_acking_without_status(ack_delay: Duration) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let Ok(mut ws) = tungstenite::accept(stream) else {
            return;
        };
        loop {
            match ws.read() {
                Ok(Message::Close(_)) => {
                    thread::sleep(ack_delay);
                    let _ = ws.close(None);
                    let _ = ws.flush();
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    format!("ws://{addr}/")
}

/// A peer that initiates the close itself, with an explicit code and reason.
fn peer_closing_first() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let Ok(mut ws) = tungstenite::accept(stream) else {
            return;
        };
        thread::sleep(Duration::from_millis(50));
        let _ = ws.close(Some(CloseFrame {
            code: CloseCode::Normal,
            reason: "bye".into(),
        }));
        let _ = ws.flush();
        for _ in 0..50 {
            if ws.read().is_err() {
                break;
            }
        }
    });
    format!("ws://{addr}/")
}

/// Connect, run `after_open`, and return the close `(code, reason)` reported.
fn close_code_for(url: String, after_open: impl FnOnce(&rinch_ws::WsHandle)) -> (u16, String) {
    let ws = connect(url).expect("connect starts");
    let opened = Rc::new(Cell::new(false));
    let closed: Rc<RefCell<Option<(u16, String)>>> = Rc::new(RefCell::new(None));

    let o = opened.clone();
    ws.on_open(move || o.set(true));
    let c = closed.clone();
    ws.on_close(move |cl| *c.borrow_mut() = Some((cl.code, cl.reason.clone())));

    assert!(
        pump_until(|| opened.get(), Duration::from_secs(10)),
        "connection should open"
    );
    after_open(&ws);
    assert!(
        pump_until(|| closed.borrow().is_some(), Duration::from_secs(10)),
        "on_close should fire"
    );
    closed.borrow().clone().unwrap()
}

#[test]
fn close_codes_match_the_websocket_spec_and_the_web_backend() {
    PUMP.set(Mutex::new(Vec::new())).ok();
    register_main_thread();
    set_cross_thread_dispatcher(dispatcher);

    // 1. A local close(), acked promptly by a status-less close frame.
    //
    // RFC 6455 reserves **1005** for "close frame carried no status code"; 1006 means
    // no close frame arrived at all. This previously reported 1006, so a clean
    // shutdown was indistinguishable from a dropped connection — and it disagreed
    // with the web backend, which passes through the browser's CloseEvent.code()
    // (1005 in this case).
    let (code, _) = close_code_for(peer_acking_without_status(Duration::ZERO), |ws| ws.close());
    assert_eq!(
        code, 1005,
        "a completed close handshake is not 1006/abnormal"
    );

    // 2. The same, with a peer slower than one poll interval (50ms).
    //
    // The loop used to give up at the first read timeout after a local close, so any
    // peer slower than a poll interval — i.e. anything off-localhost — reported an
    // abnormal close. The close grace period keeps reading so the handshake lands.
    let (code, _) = close_code_for(
        peer_acking_without_status(Duration::from_millis(200)),
        |ws| ws.close(),
    );
    assert_eq!(
        code, 1005,
        "a peer slower than POLL_INTERVAL must still complete the handshake"
    );

    // 3. A peer-initiated close carries its code and reason through untouched.
    let (code, reason) = close_code_for(peer_closing_first(), |_ws| {});
    assert_eq!((code, reason.as_str()), (1000, "bye"));
}
