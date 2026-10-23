//! `WsRequest` in a browser (issue #1773): the subprotocols reach the
//! `WebSocket` constructor, and `protocol()` reads what the server chose.
//!
//! The constructor half needs no server. The live half needs one, which the
//! test runner does not serve: build with `RINCH_WS_TEST_URL` set to a server
//! that answers the upgrade with the LAST subprotocol offered and then sends
//! `protocol=<offered header>` (the native test's `spawn_reporting_server`; a
//! Python `websockets` equivalent is in the PR that added this file). Without
//! it the live test logs that it was skipped and passes.
#![cfg(target_arch = "wasm32")]

use rinch_ws::{WsError, WsEvent, WsMessage, WsRequest, connect};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn the_browser_takes_the_offered_list_and_refuses_what_it_should() {
    // Accepted by the constructor: distinct tokens, case counting.
    let ws = WsRequest::new("ws://127.0.0.1:9/")
        .protocols(["x", "y", "X"])
        .connect()
        .expect("the browser accepts three distinct tokens");
    assert_eq!(ws.protocol(), "", "nothing is chosen before the open");
    drop(ws);
    let ws = connect("ws://127.0.0.1:9/").unwrap();
    assert_eq!(ws.protocol(), "");
    drop(ws);

    // Refused, as by the native backend: by rinch's own check, before the
    // browser sees it.
    for bad in [
        WsRequest::new("ws://127.0.0.1:9/").protocol(""),
        WsRequest::new("ws://127.0.0.1:9/").protocol("two words"),
        WsRequest::new("ws://127.0.0.1:9/").protocols(["x", "x"]),
    ] {
        assert!(matches!(bad.connect(), Err(WsError::InvalidUrl(_))));
    }
    // Positive control that the web backend's constructor runs: a URL only the
    // browser can tell is broken.
    let err = WsRequest::new("ws://[::1/")
        .protocol("x")
        .connect()
        .err()
        .expect("the browser refuses a malformed URL");
    assert!(matches!(err, WsError::InvalidUrl(_)));
}

#[wasm_bindgen_test]
async fn the_chosen_subprotocol_is_read_at_the_open() {
    let Some(url) = option_env!("RINCH_WS_TEST_URL") else {
        web_sys_log("RINCH_WS_TEST_URL unset: live subprotocol test skipped");
        return;
    };
    let mut ws = WsRequest::new(url)
        .protocol("chat.v2")
        .protocol("auth.t0ken")
        .connect()
        .unwrap()
        .into_stream();
    assert_eq!(ws.protocol(), "");
    assert!(matches!(ws.next().await, Some(WsEvent::Open)));
    assert_eq!(ws.protocol(), "auth.t0ken");
    match ws.next().await {
        Some(WsEvent::Message(WsMessage::Text(t))) => {
            assert_eq!(t, "protocol=chat.v2, auth.t0ken")
        }
        other => panic!("expected the server's report, got {other:?}"),
    }
}

fn web_sys_log(msg: &str) {
    wasm_bindgen_test::console_log!("{msg}");
}
