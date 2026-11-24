//! The `inputmode` prop on the web (#3048): the browser picks the on-screen
//! keyboard from the real `<input>`'s `inputmode` attribute, so the prop must
//! reach it. What is asserted is the browser's own answer, the
//! `HTMLElement.inputMode` property, which reflects the content attribute,
//! and not only rinch's record of having written it.
//!
//! Its own file (its own page): `run` mounts under `#rinch-body` for the
//! page's lifetime.
#![cfg(target_arch = "wasm32")]

use rinch::App;
use rinch::components::{NumberInput, PasswordInput, TextInput, Textarea};
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

fn app(scope: &mut RenderScope) -> NodeHandle {
    let root = scope.create_element("div");
    root.set_attribute("id", "inputmode-3048");
    wrap(
        scope,
        &root,
        "code",
        TextInput {
            autocomplete: "one-time-code".into(),
            inputmode: "numeric".into(),
            ..Default::default()
        },
    );
    wrap(
        scope,
        &root,
        "pin",
        PasswordInput {
            inputmode: "numeric".into(),
            ..Default::default()
        },
    );
    wrap(
        scope,
        &root,
        "amount",
        NumberInput {
            inputmode: "decimal".into(),
            ..Default::default()
        },
    );
    wrap(
        scope,
        &root,
        "notes",
        Textarea {
            inputmode: "email".into(),
            ..Default::default()
        },
    );
    wrap(scope, &root, "plain", TextInput::default());
    root
}

fn field(selector: &str) -> web_sys::HtmlElement {
    document()
        .query_selector(selector)
        .unwrap()
        .expect("the field is on the page")
        .dyn_into()
        .unwrap()
}

/// The browser's `HTMLElement.inputMode` (web-sys binds no getter for it):
/// the property reflecting the `inputmode` content attribute, which is what
/// the browser itself reads when it picks a keyboard.
fn input_mode(el: &web_sys::HtmlElement) -> String {
    js_sys::Reflect::get(el, &"inputMode".into())
        .unwrap()
        .as_string()
        .expect("inputMode is a string")
}

#[wasm_bindgen_test]
fn the_prop_reaches_the_browsers_field() {
    App::new(app).run();
    assert!(document().get_element_by_id("inputmode-3048").is_some());

    assert_eq!(input_mode(&field("#code input")), "numeric");
    let pin = field("#pin input");
    assert_eq!(input_mode(&pin), "numeric");
    // The masked field is still a real password field.
    assert_eq!(pin.get_attribute("type").as_deref(), Some("password"));
    assert_eq!(input_mode(&field("#amount input")), "decimal");
    assert_eq!(input_mode(&field("#notes textarea")), "email");

    // No prop: no attribute, the browser's default (as Chrome leaves a bare
    // `<input>`); the reflected property is then the empty string.
    let plain = field("#plain input");
    assert_eq!(plain.get_attribute("inputmode"), None);
    assert_eq!(input_mode(&plain), "");
}
