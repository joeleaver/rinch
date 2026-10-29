//! A `WsHandle` dropped while its thread's locals are torn down (#1952).
//!
//! An app that keeps a handle in its own `thread_local!` (or in anything such a
//! local owns) drops it at thread exit, possibly after rinch-ws's callback table
//! is gone. `WsHandle::drop` reached that table with `LocalKey::with`, which
//! panics there, and a panic in a thread-local destructor aborts the process:
//! on an app's main thread, an abort on every exit.
//!
//! Each test runs on a thread of its own and joins it; a regression aborts this
//! test binary.
#![cfg(not(target_arch = "wasm32"))]

use std::cell::RefCell;
use std::net::TcpListener;
use std::sync::Once;

use rinch_ws::{WsHandle, connect};

/// A URL nothing listens on. The connect fails on the worker thread; the
/// handle exists either way, which is all these tests need.
fn refused_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("ws://127.0.0.1:{}/", listener.local_addr().unwrap().port())
}

/// Outside an app there is no main thread to deliver events to: drop them.
fn no_dispatch() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| rinch_core::set_cross_thread_dispatcher(|_job| {}));
}

thread_local! {
    static HELD: RefCell<Vec<WsHandle>> = const { RefCell::new(Vec::new()) };
}

/// The issue's shape: the app's local is touched first, so (destructors run in
/// reverse registration order) it is destroyed after rinch-ws's table.
#[test]
fn a_handle_in_an_app_thread_local_drops_quietly_at_thread_exit() {
    no_dispatch();
    let t = std::thread::spawn(|| {
        HELD.with(|h| h.borrow_mut().clear());
        let ws = connect(refused_url()).expect("connect starts");
        HELD.with(|h| h.borrow_mut().push(ws));
    });
    assert!(t.join().is_ok(), "the thread panicked at exit");
}

/// The forced case, independent of destructor order: a handle owned by
/// another connection's callback is dropped by rinch-ws's own table as that
/// table is destroyed, so its drop always finds the table gone.
#[test]
fn a_handle_owned_by_a_callback_drops_quietly_as_the_table_is_destroyed() {
    no_dispatch();
    let t = std::thread::spawn(|| {
        let owner = connect(refused_url()).expect("connect starts");
        let owned = connect(refused_url()).expect("connect starts");
        owner.on_message(move |_| {
            let _keep = &owned;
        });
        // Still registered at thread exit: the table holds `owned` until then.
        std::mem::forget(owner);
    });
    assert!(t.join().is_ok(), "the thread panicked at exit");
}
