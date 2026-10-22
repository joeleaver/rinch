//! `poll_signal` in the browser, issue #1773.
//!
//! The native hosts sample polls once per frame (`drain_polls`); rinch-web had
//! no caller of it, so a polled signal kept its first value for ever. These
//! mount through `rinch_web::mount_into` (which installs the web's poll
//! driver) and wait in real time.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test poll_signal_1773
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::Signal;
use rinch_core::element::ThemeProviderProps;
use rinch_core::reactive::{PollRate, poll_signal};
use rinch_web::RootHandle;
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Mount a root whose render registers a poll over `source` at `rate`, and
/// return the polled signal, how often the source was read, and the root.
fn mount_poll(source: Rc<Cell<i32>>, rate: PollRate) -> (Signal<i32>, Rc<Cell<u32>>, RootHandle) {
    let bdoc = web_sys::window().unwrap().document().unwrap();
    let host = bdoc.create_element("div").unwrap();
    bdoc.body().unwrap().append_child(&host).unwrap();
    let out: Rc<Cell<Option<Signal<i32>>>> = Rc::new(Cell::new(None));
    let reads = Rc::new(Cell::new(0u32));
    let (o, r) = (out.clone(), reads.clone());
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let sig = poll_signal(
            move || {
                r.set(r.get() + 1);
                source.get()
            },
            rate,
        );
        o.set(Some(sig));
        let p = scope.create_element("p");
        p.set_attribute("data-poll-1773", "");
        p
    });
    // Positive control: the mount really ran the render.
    assert!(
        host.query_selector("[data-poll-1773]").unwrap().is_some(),
        "the root rendered"
    );
    (
        out.get().expect("the render registered the poll"),
        reads,
        root,
    )
}

async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

#[wasm_bindgen_test]
async fn a_timed_poll_is_sampled_in_the_browser() {
    let source = Rc::new(Cell::new(0));
    let (sig, _reads, root) = mount_poll(source.clone(), PollRate::Hz(50));
    assert_eq!(sig.get(), 0, "the first read happens at registration");
    source.set(7);
    sleep_ms(150).await;
    assert_eq!(sig.get(), 7, "a 50Hz poll is sampled within 150ms");
    source.set(9);
    sleep_ms(150).await;
    assert_eq!(sig.get(), 9, "and keeps being sampled");
    root.unmount();
}

#[wasm_bindgen_test]
async fn an_every_frame_poll_is_sampled_on_animation_frames() {
    let source = Rc::new(Cell::new(0));
    let (sig, _reads, root) = mount_poll(source.clone(), PollRate::EveryFrame);
    source.set(3);
    sleep_ms(150).await;
    assert_eq!(sig.get(), 3);
    root.unmount();
}

#[wasm_bindgen_test]
async fn an_unmounted_poll_stops_being_read() {
    let source = Rc::new(Cell::new(0));
    let (_sig, reads, root) = mount_poll(source, PollRate::Hz(100));
    sleep_ms(100).await;
    assert!(
        reads.get() > 2,
        "the poll ran while mounted ({})",
        reads.get()
    );
    root.unmount();
    sleep_ms(50).await;
    let after = reads.get();
    sleep_ms(150).await;
    assert_eq!(reads.get(), after, "no read after the unmount reaped it");
}
