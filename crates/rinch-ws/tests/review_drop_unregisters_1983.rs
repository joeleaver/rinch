//! Review of #1983: dropping a handle releases its callbacks at once (not at
//! thread exit). Kills "WsHandle::drop skips the unregister" — the try_with
//! rewrite's degenerate form, which every #1952 fixture passes.
#![cfg(not(target_arch = "wasm32"))]

use std::cell::Cell;
use std::net::TcpListener;
use std::rc::Rc;
use std::sync::Once;

use rinch_ws::connect;

#[test]
fn dropping_a_handle_drops_its_callbacks_now() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| rinch_core::set_cross_thread_dispatcher(|_job| {}));
    let url = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("ws://127.0.0.1:{}/", l.local_addr().unwrap().port())
    };
    struct Flag(Rc<Cell<bool>>);
    impl Drop for Flag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    let dropped = Rc::new(Cell::new(false));
    let flag = Flag(dropped.clone());
    let ws = connect(url).expect("connect starts");
    ws.on_close(move |_| {
        let _keep = &flag;
    });
    assert!(!dropped.get());
    drop(ws);
    assert!(dropped.get(), "the handle's callbacks outlived the handle");
}
