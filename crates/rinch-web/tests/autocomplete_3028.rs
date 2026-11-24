//! The `autocomplete` prop on the web (#3028): the browser's autofill and
//! password managers read the real `<input>`'s attribute, so the prop must
//! reach it. What is asserted is the browser's own answer, the
//! `HTMLInputElement.autocomplete` / `HTMLTextAreaElement.autocomplete`
//! property, which reflects the content attribute, and not only rinch's
//! record of having written it.
//!
//! Its own file (its own page): `run` mounts under `#rinch-body` for the
//! page's lifetime.
#![cfg(target_arch = "wasm32")]

use rinch::App;
use rinch::components::{PasswordInput, TextInput, Textarea};
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
    root.set_attribute("id", "autocomplete-3028");
    wrap(
        scope,
        &root,
        "user",
        TextInput {
            autocomplete: "username".into(),
            ..Default::default()
        },
    );
    wrap(
        scope,
        &root,
        "pass",
        PasswordInput {
            autocomplete: "current-password".into(),
            ..Default::default()
        },
    );
    wrap(scope, &root, "plain-pass", PasswordInput::default());
    wrap(
        scope,
        &root,
        "notes",
        Textarea {
            autocomplete: "off".into(),
            ..Default::default()
        },
    );
    root
}

fn input(id: &str) -> web_sys::HtmlInputElement {
    document()
        .query_selector(&format!("#{id} input"))
        .unwrap()
        .expect("the field is on the page")
        .dyn_into()
        .unwrap()
}

#[wasm_bindgen_test]
fn the_prop_reaches_the_browsers_field() {
    App::new(app).run();
    assert!(document().get_element_by_id("autocomplete-3028").is_some());

    assert_eq!(input("user").autocomplete(), "username");
    assert_eq!(input("pass").autocomplete(), "current-password");
    // The field is a real password field, the kind a manager fills.
    assert_eq!(input("pass").type_(), "password");

    // No prop: no attribute, so the browser's default applies; the old
    // hard-coded "off" is gone.
    let plain = input("plain-pass");
    assert_eq!(plain.get_attribute("autocomplete"), None);
    assert_eq!(plain.autocomplete(), "");

    let notes: web_sys::HtmlTextAreaElement = document()
        .query_selector("#notes textarea")
        .unwrap()
        .expect("the textarea is on the page")
        .dyn_into()
        .unwrap();
    assert_eq!(notes.autocomplete(), "off");
}
