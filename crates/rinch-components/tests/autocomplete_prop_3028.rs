//! The `autocomplete` prop of the text-entry components (#3028): a value is
//! written on the field as its `autocomplete` attribute, an empty one writes
//! none (the browser's default). `PasswordInput` used to hard-code `"off"`,
//! which kept password managers out of every rinch password field.
//! `ColorInput` defaults to `"off"`: its picker opens where the browser would
//! list remembered entries.

// `rsx!` names the facade crate's paths (`rinch::…`); this crate stands in.
use rinch_components as rinch;

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::color_input::ColorInput;
use rinch_components::number_input::NumberInput;
use rinch_components::password_input::PasswordInput;
use rinch_components::text_input::TextInput;
use rinch_components::textarea::Textarea;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::{Component, Signal};
use rinch_macros::rsx;

mod common;
use common::find_by_class;

/// The `autocomplete` attribute of the element carrying `class` in what
/// `build` renders. The scope is kept alive until the read, so effects the
/// render registered still run.
fn rendered(build: impl FnOnce(&mut RenderScope) -> NodeHandle, class: &str) -> Option<String> {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = build(&mut scope);
    find_by_class(&root, class)
        .expect("the field exists")
        .get_attribute("autocomplete")
}

fn render(component: &dyn Component, class: &str) -> Option<String> {
    rendered(|scope| component.render(scope, &[]), class)
}

/// A component's name, the component, and the class of its field.
type Case = (&'static str, Box<dyn Component>, &'static str);

/// Every component and the class of its `<input>`/`<textarea>`, built with
/// `autocomplete` set to `value`.
fn each_component(value: &str) -> Vec<Case> {
    let autocomplete = value.to_string();
    vec![
        (
            "TextInput",
            Box::new(TextInput {
                autocomplete: autocomplete.clone(),
                ..Default::default()
            }),
            "rinch-text-input__input",
        ),
        (
            "Textarea",
            Box::new(Textarea {
                autocomplete: autocomplete.clone(),
                ..Default::default()
            }),
            "rinch-textarea__input",
        ),
        (
            "PasswordInput",
            Box::new(PasswordInput {
                autocomplete: autocomplete.clone(),
                ..Default::default()
            }),
            "rinch-password-input__input",
        ),
        (
            "NumberInput",
            Box::new(NumberInput {
                autocomplete: autocomplete.clone(),
                ..Default::default()
            }),
            "rinch-number-input__input",
        ),
        (
            "ColorInput",
            Box::new(ColorInput {
                autocomplete,
                ..Default::default()
            }),
            "rinch-color-input__input",
        ),
    ]
}

#[test]
fn each_text_component_writes_a_set_autocomplete() {
    for value in ["username", "current-password", "one-time-code"] {
        for (name, component, class) in each_component(value) {
            assert_eq!(
                render(component.as_ref(), class).as_deref(),
                Some(value),
                "{name} with autocomplete {value:?}"
            );
        }
    }
}

#[test]
fn an_empty_autocomplete_writes_no_attribute() {
    for (name, component, class) in each_component("") {
        assert_eq!(render(component.as_ref(), class), None, "{name}");
    }
}

#[test]
fn the_defaults_leave_autofill_to_the_browser_except_the_colour_field() {
    let defaults: [Case; 5] = [
        (
            "TextInput",
            Box::new(TextInput::default()),
            "rinch-text-input__input",
        ),
        (
            "Textarea",
            Box::new(Textarea::default()),
            "rinch-textarea__input",
        ),
        (
            "PasswordInput",
            Box::new(PasswordInput::default()),
            "rinch-password-input__input",
        ),
        (
            "NumberInput",
            Box::new(NumberInput::default()),
            "rinch-number-input__input",
        ),
        (
            "ColorInput",
            Box::new(ColorInput::default()),
            "rinch-color-input__input",
        ),
    ];
    for (name, component, class) in defaults {
        // PasswordInput is no longer the hard-coded "off" (#3028): a password
        // manager may fill the field. Only the colour field turns autofill off.
        let want = (name == "ColorInput").then_some("off");
        assert_eq!(render(component.as_ref(), class).as_deref(), want, "{name}");
    }
}

#[test]
fn a_password_input_takes_the_tokens_a_password_manager_reads() {
    for value in ["current-password", "new-password"] {
        let field = PasswordInput {
            autocomplete: value.into(),
            ..Default::default()
        };
        assert_eq!(
            render(&field, "rinch-password-input__input").as_deref(),
            Some(value)
        );
    }
}

#[test]
fn the_prop_takes_a_literal_and_a_reactive_closure_through_rsx() {
    let literal = rendered(
        |__scope| rsx! { div { TextInput { autocomplete: "email" } } },
        "rinch-text-input__input",
    );
    assert_eq!(literal.as_deref(), Some("email"));

    // The `{|| expr}` form: the field follows the signal.
    let token = Signal::new(String::from("current-password"));
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let __scope = &mut scope;
    let root = rsx! {
        div { PasswordInput { autocomplete: {move || token.get()} } }
    };
    let read = || {
        find_by_class(&root, "rinch-password-input__input")
            .expect("the field exists")
            .get_attribute("autocomplete")
    };
    assert_eq!(read().as_deref(), Some("current-password"));
    token.set("new-password".into());
    assert_eq!(read().as_deref(), Some("new-password"));
    token.set(String::new());
    assert_eq!(read(), None, "an empty value removes the attribute");
}
