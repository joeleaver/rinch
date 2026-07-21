//! Native TLS-path test, in its own binary so it gets its own process (and thus
//! its own `register_main_thread`) — see `common`.
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

/// The `wss://` path must reach a real TLS handshake and report failures through
/// `on_error` — not panic.
///
/// Regression test for a missing rustls crypto provider. `tungstenite`'s
/// `rustls-tls-webpki-roots` enables the `rustls` dependency but no provider
/// feature, so rustls had no compiled-in default and panicked on the first
/// handshake with "Could not automatically determine the process-level
/// CryptoProvider". Because that panic happened on the connection's worker
/// thread, it surfaced as a *silent hang*: no `on_error`, no `on_open`. It was
/// masked whenever an app also linked `rinch-http` (ureq enables `rustls/ring`
/// and cargo unifies features), so `wss://` worked only by accident of what else
/// was in the dependency graph.
///
/// A full `wss://` round-trip isn't practical offline (webpki roots reject a
/// self-signed cert by design), so this drives the handshake against a plain-TCP
/// listener that answers with garbage: enough to prove rustls initialized and
/// that the failure is reported rather than swallowed.
#[test]
fn wss_handshake_failure_is_reported_not_panicked() {
    PUMP.set(Mutex::new(Vec::new())).ok();
    register_main_thread();
    set_cross_thread_dispatcher(dispatcher);

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            use std::io::Write;
            // Not a TLS ServerHello — the handshake must fail *after* rustls has
            // successfully resolved its crypto provider.
            let _ = stream.write_all(b"not a tls handshake\r\n\r\n");
            thread::sleep(Duration::from_millis(300));
        }
    });

    let ws = connect(format!("wss://127.0.0.1:{}/", addr.port())).expect("wss url is accepted");
    let err: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let opened = Rc::new(Cell::new(false));

    let e = err.clone();
    ws.on_error(move |x| *e.borrow_mut() = Some(format!("{x:?}")));
    let o = opened.clone();
    ws.on_open(move || o.set(true));

    assert!(
        pump_until(
            || err.borrow().is_some() || opened.get(),
            Duration::from_secs(10)
        ),
        "a failed wss handshake must surface via on_error, not hang"
    );
    assert!(
        !opened.get(),
        "the handshake cannot succeed against garbage"
    );
    assert!(
        err.borrow().is_some(),
        "on_error must fire (a worker-thread panic would leave this None)"
    );
}
