//! Review fixture for PR #1435, in its own wasm binary because a failure here
//! is a `BorrowMutError` panic, which poisons the page for later tests.
//!
//! The resolver / loader is app code, and on the web it is called
//! synchronously from inside `WebDocument::set_attribute`, i.e. while
//! `NodeHandle::set_attribute` holds `doc.borrow_mut()`. The natural thing for
//! a "not yet" resolver to do is note that the picture is wanted — a signal
//! write (`pending.update(..)`, a spinner flag). Any effect that write runs
//! touches the DOM through the same `RefCell`.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::NodeHandle;
use rinch_core::element::ThemeProviderProps;
use rinch_core::reactive::{Effect, Signal};
use rinch_web::register_image_url_scheme;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn finding_a_resolver_that_writes_a_signal_panics_inside_set_attribute() {
    let wanted: Rc<RefCell<Option<Signal<u32>>>> = Rc::new(RefCell::new(None));
    let wanted_in = wanted.clone();
    register_image_url_scheme("rv-reentry", move |_src| {
        // "not yet": remember that somebody wants it.
        if let Some(sig) = *wanted_in.borrow() {
            sig.update(|n| *n += 1);
        }
        None
    });

    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let img_out: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let img_in = img_out.clone();
    let wanted_mount = wanted.clone();
    let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let sig = Signal::new(0u32);
        *wanted_mount.borrow_mut() = Some(sig);
        let root = scope.create_element("div");
        let badge = scope.create_element("span");
        badge.set_attribute("id", "rv-reentry-badge");
        root.append_child(&badge);
        // What `span { {|| format!("{} loading", wanted.get())} }` compiles to.
        let badge_in = badge.clone();
        Effect::new(move || badge_in.set_text(&format!("{} loading", sig.get())));
        let img = scope.create_element("img");
        root.append_child(&img);
        *img_in.borrow_mut() = Some(img);
        root
    });

    // Later (a timer, a network completion, a test): the element is given a
    // source the app answers for.
    let img = img_out.borrow().clone().unwrap();
    img.set_attribute("src", "rv-reentry:pic");
    // The resolver is asked from a microtask, outside the document's borrow.
    for _ in 0..2 {
        wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(
            &wasm_bindgen::JsValue::NULL,
        ))
        .await
        .unwrap();
    }

    let badge = document
        .get_element_by_id("rv-reentry-badge")
        .unwrap()
        .text_content();
    root.unmount();
    host.remove();
    assert_eq!(badge.as_deref(), Some("1 loading"));
}
