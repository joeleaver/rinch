//! `WsRequest` (issue #1773): offering subprotocols, sending headers (native
//! only) and reading the subprotocol the server chose, against an in-process
//! tungstenite server that reports what it received.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::cell::RefCell;
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use common::{PUMP, dispatcher, pump_until, spawn_echo_server};
use rinch_core::{register_main_thread, set_cross_thread_dispatcher, spawn_local};
use rinch_ws::{WsError, WsEvent, WsMessage, WsRequest, connect};
use tungstenite::Message;
use tungstenite::handshake::server::{Request, Response};

/// How the server answers the offered subprotocols.
#[derive(Clone, Copy)]
enum Choose {
    /// The last one offered (so "the first one" is a wrong answer).
    Last,
    /// None at all.
    Nothing,
}

/// A server for one connection. It answers the upgrade per `choose`, then
/// sends one text frame `protocol=<offered header>;auth=<Authorization>` and
/// echoes text frames after that.
fn spawn_reporting_server(choose: Choose) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let seen: Rc<RefCell<(String, String)>> = Rc::default();
        let s = seen.clone();
        let callback = move |req: &Request, mut resp: Response| {
            let get = |name: &str| {
                req.headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string()
            };
            let offered = get("Sec-WebSocket-Protocol");
            *s.borrow_mut() = (offered.clone(), get("Authorization"));
            if let (Choose::Last, Some(last)) = (choose, offered.split(',').next_back()) {
                if !last.is_empty() {
                    resp.headers_mut()
                        .insert("Sec-WebSocket-Protocol", last.trim().parse().unwrap());
                }
            }
            Ok(resp)
        };
        let Ok(mut ws) = tungstenite::accept_hdr(stream, callback) else {
            return;
        };
        let (offered, auth) = seen.borrow().clone();
        if ws
            .send(Message::Text(format!("protocol={offered};auth={auth}")))
            .is_err()
        {
            return;
        }
        loop {
            match ws.read() {
                Ok(Message::Text(t)) => {
                    if ws.send(Message::Text(t)).is_err() {
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

/// Connect with `request`, wait for the open and the server's report, and
/// return (the protocol `on_open` read, the report).
fn open_and_report(request: WsRequest) -> (String, String) {
    let ws = Rc::new(request.connect().expect("connect starts"));
    let at_open: Rc<RefCell<Option<String>>> = Rc::default();
    let report: Rc<RefCell<Option<String>>> = Rc::default();
    let (w, a) = (Rc::downgrade(&ws), at_open.clone());
    ws.on_open(move || *a.borrow_mut() = Some(w.upgrade().unwrap().protocol()));
    let r = report.clone();
    ws.on_message(move |m| {
        if let WsMessage::Text(t) = m {
            r.borrow_mut().get_or_insert(t);
        }
    });
    ws.on_error(|e| panic!("unexpected error: {e}"));
    assert!(
        pump_until(
            || at_open.borrow().is_some() && report.borrow().is_some(),
            Duration::from_secs(10)
        ),
        "the connection opens and the server reports"
    );
    // Still readable after the open.
    assert_eq!(Some(ws.protocol()), *at_open.borrow());
    let out = (at_open.take().unwrap(), report.take().unwrap());
    out
}

#[test]
fn subprotocols_headers_and_the_chosen_protocol() {
    PUMP.set(Mutex::new(Vec::new())).ok();
    register_main_thread();
    set_cross_thread_dispatcher(dispatcher);

    // 1. Two subprotocols offered, in order, and a header; the server takes the
    //    last one, and `protocol()` says so from `on_open` on.
    let url = spawn_reporting_server(Choose::Last);
    let (protocol, report) = open_and_report(
        WsRequest::new(url)
            .protocol("chat.v2")
            .protocols(["auth.t0ken"])
            .header("Authorization", "Bearer abc"),
    );
    assert_eq!(protocol, "auth.t0ken");
    assert_eq!(report, "protocol=chat.v2,auth.t0ken;auth=Bearer abc");

    // 2. Nothing offered: no header goes out and the protocol is "".
    let url = spawn_reporting_server(Choose::Last);
    let (protocol, report) = open_and_report(WsRequest::new(url));
    assert_eq!(protocol, "");
    assert_eq!(report, "protocol=;auth=");

    // 3. `protocol()` before the open is "".
    let url = spawn_echo_server();
    let ws = connect(&url).unwrap();
    assert_eq!(ws.protocol(), "");
    drop(ws);

    // 4. A request the browser would refuse is refused synchronously here too.
    for bad in [
        WsRequest::new("ws://127.0.0.1:1/").protocol(""),
        WsRequest::new("ws://127.0.0.1:1/").protocol("two words"),
        WsRequest::new("ws://127.0.0.1:1/").protocol("a,b"),
        WsRequest::new("ws://127.0.0.1:1/").protocols(["x", "y", "x"]),
        WsRequest::new("ws://127.0.0.1:1/").header("bad name", "v"),
        WsRequest::new("ws://127.0.0.1:1/").header("X-Ok", "line\nbreak"),
        WsRequest::new("http://127.0.0.1:1/"),
    ] {
        match bad.connect() {
            Err(WsError::InvalidUrl(_)) => {}
            Err(e) => panic!("wrong error: {e}"),
            Ok(_) => panic!("accepted a request a browser refuses"),
        }
    }
    // Positive control: the same shapes, valid, are accepted.
    assert!(
        WsRequest::new("ws://127.0.0.1:1/")
            .protocols(["x", "y", "X"])
            .header("X-Ok", "fine")
            .connect()
            .is_ok()
    );

    // 5. Native difference, pinned: offered subprotocols and a server that
    //    chooses none fail the connect here (tungstenite), where a browser
    //    opens with protocol "".
    let url = spawn_reporting_server(Choose::Nothing);
    let ws = WsRequest::new(url).protocol("chat.v2").connect().unwrap();
    let outcome: Rc<RefCell<Option<&str>>> = Rc::default();
    let o = outcome.clone();
    ws.on_open(move || *o.borrow_mut() = Some("open"));
    let o = outcome.clone();
    ws.on_error(move |e| {
        assert!(matches!(e, WsError::Connect(_)), "{e}");
        o.borrow_mut().get_or_insert("error");
    });
    assert!(pump_until(
        || outcome.borrow().is_some(),
        Duration::from_secs(10)
    ));
    assert_eq!(*outcome.borrow(), Some("error"));
    drop(ws);

    // 6. A stream reads the chosen protocol once `Open` is yielded.
    let url = spawn_reporting_server(Choose::Last);
    let seen: Rc<RefCell<Vec<String>>> = Rc::default();
    let s = seen.clone();
    let task = spawn_local(async move {
        let mut ws = WsRequest::new(url)
            .protocols(["a", "b"])
            .connect()
            .unwrap()
            .into_stream();
        s.borrow_mut().push(ws.protocol());
        assert!(matches!(ws.next().await, Some(WsEvent::Open)));
        s.borrow_mut().push(ws.protocol());
    });
    assert!(pump_until(|| task.is_finished(), Duration::from_secs(10)));
    assert_eq!(*seen.borrow(), ["", "b"]);
}
