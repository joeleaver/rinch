//! #2719 (item 3, `conic-gradient()`) on the web backend: the browser paints
//! the gradient itself, so all rinch-web has to do is hand the declaration
//! through. This pins that it does, through a `style` attribute and through
//! `NodeHandle::set_style` (`style.setProperty`), a repeating one and a
//! layer list included, by reading the browser's computed style back. The
//! desktop half is `crates/rinch-dom/tests/conic_gradient_2719_tests.rs`.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

#[wasm_bindgen_test]
fn conic_gradients_reach_the_browser() {
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let el = scope.create_element("div");
            el.set_attribute(
                "style",
                "width: 40px; height: 40px; \
                 background-image: conic-gradient(from 37deg at 30% 65%, red, blue)",
            );
            el.set_attribute("data-t2719", "");
            *slot_in.borrow_mut() = Some(el.clone());
            el
        },
    );
    let dom = host.query_selector("[data-t2719]").unwrap().unwrap();
    let node = slot.borrow_mut().take().unwrap();
    let image = || {
        web_sys::window()
            .unwrap()
            .get_computed_style(&dom)
            .unwrap()
            .unwrap()
            .get_property_value("background-image")
            .unwrap()
    };
    let got = image();
    assert!(
        got.starts_with("conic-gradient(") && got.contains("37deg"),
        "{got}"
    );
    node.set_style(
        "background-image",
        "repeating-conic-gradient(#222 0deg 17deg, #eee 17deg 34deg)",
    );
    let got = image();
    assert!(got.starts_with("repeating-conic-gradient("), "{got}");
    node.set_style(
        "background-image",
        "conic-gradient(red, blue), linear-gradient(green, white)",
    );
    let got = image();
    assert!(
        got.starts_with("conic-gradient(") && got.contains("linear-gradient("),
        "{got}"
    );
    root.unmount();
    host.remove();
}
