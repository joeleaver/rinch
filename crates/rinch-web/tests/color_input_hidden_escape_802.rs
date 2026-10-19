//! An open `ColorInput` picker its ancestor stopped rendering does not take
//! Escape, in a real browser (issue #802).
//!
//! The twin of the `escape_passes_a_picker_*` fixtures in
//! `rinch/src/app/color_input_dismiss_465_tests.rs`. The open-time dismiss
//! entry asks `NodeHandle::is_rendered` before it consumes, and on this backend
//! that is `WebDocument::is_rendered`, answered from `getComputedStyle` — a
//! different implementation from desktop's, so it needs its own fixture.
//!
//! "The key fell through" is observed with a counting dismiss entry pushed
//! before the picker opens, so it sits under the picker's entry. The control
//! (`a_rendered_picker_still_takes_its_escape_in_chrome`) is what proves the
//! synthetic Escape reaches rinch's listener at all.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test color_input_hidden_escape_802
//! ```
#![cfg(target_arch = "wasm32")]

use rinch::components::ColorInput;
use rinch_core::Component;
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-802";

thread_local! {
    static PREVIOUS: RefCell<Option<RootHandle>> = const { RefCell::new(None) };
}

fn browser_document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Mount a `ColorInput` in a fresh host and return the host.
fn mount() -> web_sys::Element {
    let bdoc = browser_document();
    PREVIOUS.with(|p| {
        if let Some(h) = p.borrow_mut().take() {
            h.unmount();
        }
    });
    while let Ok(Some(el)) = bdoc.query_selector(&format!("[{HOST_ATTR}]")) {
        el.remove();
    }
    let host = bdoc.create_element("div").unwrap();
    host.set_attribute(HOST_ATTR, "true").unwrap();
    host.set_attribute("style", "width: 240px; margin: 40px")
        .unwrap();
    bdoc.body().unwrap().append_child(&host).unwrap();
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        ColorInput {
            value: "#000000".to_string(),
            ..Default::default()
        }
        .render(scope, &[])
    });
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
    host
}

fn one(selector: &str) -> web_sys::Element {
    browser_document()
        .query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("nothing matches `{selector}`"))
}

fn is_open() -> bool {
    one(".rinch-color-input")
        .get_attribute("class")
        .is_some_and(|c| {
            c.split_whitespace()
                .any(|one| one == "rinch-color-input--opened")
        })
}

fn open() {
    one(".rinch-color-input__input-group")
        .dyn_ref::<web_sys::HtmlElement>()
        .unwrap()
        .click();
    assert!(is_open(), "precondition: the picker is open");
}

fn escape() {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key("Escape");
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    browser_document()
        .body()
        .unwrap()
        .dispatch_event(&ev)
        .unwrap();
}

/// A dismiss entry under everything opened later, counting what reaches it.
fn count_escapes_behind() -> (Rc<Cell<usize>>, rinch_core::DismissHandle) {
    let reached = Rc::new(Cell::new(0));
    let r = reached.clone();
    let handle = rinch_core::push_dismiss_handler(0, move || {
        r.set(r.get() + 1);
        true
    });
    (reached, handle)
}

#[wasm_bindgen_test]
fn a_rendered_picker_still_takes_its_escape_in_chrome() {
    mount();
    let (behind, _keep) = count_escapes_behind();
    open();
    escape();
    assert!(!is_open(), "the picker answers the Escape");
    assert_eq!(behind.get(), 0, "and the key goes no further");
}

#[wasm_bindgen_test]
fn a_picker_under_a_display_none_ancestor_passes_escape_on_in_chrome() {
    let host = mount();
    let (behind, _keep) = count_escapes_behind();
    open();
    host.set_attribute("style", "width: 240px; margin: 40px; display: none")
        .unwrap();
    escape();
    assert_eq!(behind.get(), 1, "the hidden picker must not swallow Escape");
    assert!(is_open(), "and it is declined, not closed");
}

#[wasm_bindgen_test]
fn a_picker_under_a_visibility_hidden_ancestor_passes_escape_on_in_chrome() {
    let host = mount();
    let (behind, _keep) = count_escapes_behind();
    open();
    host.set_attribute("style", "width: 240px; margin: 40px; visibility: hidden")
        .unwrap();
    escape();
    assert_eq!(behind.get(), 1, "the hidden picker must not swallow Escape");
}
