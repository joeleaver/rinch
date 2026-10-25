//! Shared harness for rinch-rtc's native integration tests.
//!
//! - A stand-in for the rinch runtime, as in rinch-ws's tests: the test
//!   thread registers itself as the main thread, installs a dispatcher that
//!   parks main-thread work in a global queue, and pumps it. Registration is
//!   process-global, so each test *file* has exactly one `#[test]`.
//! - [`LitePeer`] (in `lite.rs`, shared with `examples/lite_echo.rs`): the
//!   interop target, shaped like Playweft's hosted game server.
#![allow(dead_code)]
#![cfg(not(target_arch = "wasm32"))]

use std::future::Future;
use std::pin::pin;
use std::sync::{Mutex, OnceLock};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::{Duration, Instant};

use rinch_core::{register_main_thread, set_cross_thread_dispatcher};

pub type MainJob = Box<dyn FnOnce() + Send>;

static PUMP: OnceLock<Mutex<Vec<MainJob>>> = OnceLock::new();

fn dispatcher(f: MainJob) {
    PUMP.get().expect("pump").lock().unwrap().push(f);
}

/// Make this thread the main thread, and bind peers to loopback.
pub fn init() {
    PUMP.set(Mutex::new(Vec::new())).ok();
    register_main_thread();
    set_cross_thread_dispatcher(dispatcher);
    rinch_rtc::testing::set_bind_addrs(&["127.0.0.1:0"]);
}

pub fn drain_pump() {
    loop {
        let jobs: Vec<_> = PUMP
            .get()
            .expect("pump")
            .lock()
            .unwrap()
            .drain(..)
            .collect();
        if jobs.is_empty() {
            return;
        }
        for job in jobs {
            job();
        }
    }
}

/// Pump until `cond` holds; false on timeout.
pub fn pump_until(mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        drain_pump();
        if cond() {
            return true;
        }
        if start.elapsed() > timeout {
            return false;
        }
        thread::sleep(Duration::from_millis(2));
    }
}

/// Pump while polling `fut` on this (main) thread until it resolves.
pub fn drive<T>(fut: impl Future<Output = T>, timeout: Duration) -> T {
    let mut fut = pin!(fut);
    let mut cx = Context::from_waker(Waker::noop());
    let start = Instant::now();
    loop {
        drain_pump();
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
        assert!(
            start.elapsed() < timeout,
            "future did not resolve in {timeout:?}"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

pub const T: Duration = Duration::from_secs(15);

mod lite;
#[allow(unused_imports)]
pub use lite::{LitePeer, LiteStats};
