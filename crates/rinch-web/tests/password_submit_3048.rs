//! Enter in a `PasswordInput` submits on the web (#3048), as it does in a
//! `TextInput`: both write `onsubmit` as `data-onsubmit` through one helper,
//! which rinch-web's delegated keydown reads on Enter.
//!
//! Its own file (its own page): `run` mounts under `#rinch-body` for the
//! page's lifetime.
#![cfg(target_arch = "wasm32")]

use rinch::App;
use rinch::components::{PasswordInput, TextInput};
use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_web::AppExt as _;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn wrap(scope: &mut RenderScope, root: &NodeHandle, id: &str, field: impl Component) {
    let node = field.render(scope, &[]);
    node.set_attribute("id", id);
    root.append_child(&node);
}

fn field(selector: &str) -> web_sys::HtmlElement {
    document()
        .query_selector(selector)
        .unwrap()
        .expect("the field is on the page")
        .dyn_into()
        .unwrap()
}

thread_local! {
    /// What each field's `onsubmit` logged, by field id.
    static SUBMITS: std::cell::RefCell<Vec<&'static str>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn submit_log(id: &'static str) -> rinch_core::Callback {
    rinch_core::Callback::new(move || SUBMITS.with(|s| s.borrow_mut().push(id)))
}

fn submit_app(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    root.set_attribute("id", "submit-3048");
    wrap(
        scope,
        &root,
        "user",
        TextInput {
            onsubmit: Some(submit_log("user")),
            ..Default::default()
        },
    );
    wrap(
        scope,
        &root,
        "pass",
        PasswordInput {
            onsubmit: Some(submit_log("pass")),
            ..Default::default()
        },
    );
    root
}

/// A bubbling, cancelable `keydown` of `key` dispatched at `el`, as the
/// browser fires one at the focused field.
fn keydown(el: &web_sys::Element, key: &str) -> web_sys::KeyboardEvent {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev
}

/// Enter in a `PasswordInput` submits, through the same delegated
/// `data-onsubmit` path as a `TextInput` (the positive control: if the
/// listener were not installed, neither would log).
#[wasm_bindgen_test]
fn enter_in_a_password_input_submits_as_in_a_text_input() {
    App::new(submit_app).run();
    assert!(document().get_element_by_id("submit-3048").is_some());

    for id in ["user", "pass"] {
        let input = field(&format!("#{id} input"));
        input.focus().unwrap();
        let ev = keydown(&input, "Enter");
        assert!(ev.default_prevented(), "{id}: the submit consumed Enter");
    }
    assert_eq!(SUBMITS.with(|s| s.borrow().clone()), ["user", "pass"]);
}
