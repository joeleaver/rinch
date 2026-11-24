//! Two text-entry props from #3048.
//!
//! * `inputmode`, on every text-entry component: written on the field as its
//!   HTML `inputmode` attribute, which picks the on-screen keyboard on the web
//!   (`numeric` for a one-time code). Empty, the default, writes none, as
//!   Chrome leaves an `<input>` with no `inputmode`.
//! * `onsubmit` on `PasswordInput`, as `TextInput` has it: Enter submits. It
//!   is written the way `TextInput`'s is, as `data-onsubmit`, which the
//!   runtimes read on Enter (the Enter itself is driven in
//!   `rinch::app::password_submit_3048_tests`).

// `rsx!` names the facade crate's paths (`rinch::…`); this crate stands in.
use rinch_components as rinch;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_components::color_input::ColorInput;
use rinch_components::number_input::NumberInput;
use rinch_components::password_input::PasswordInput;
use rinch_components::text_input::TextInput;
use rinch_components::textarea::Textarea;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::dispatch_event;
use rinch_core::{Callback, Component, IntoEventHandler, Signal};
use rinch_macros::rsx;

mod common;
use common::{find_by_class, handler};

/// The document and the scope a render was made in. Held until the reads:
/// the nodes read through the document, and the scope keeps the render's
/// handlers and effects registered.
type Keep = (Rc<RefCell<MockDomDocument>>, RenderScope);

/// What `build` renders, with what it needs kept alive.
fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle) -> (Keep, NodeHandle) {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = build(&mut scope);
    ((doc, scope), root)
}

/// The `inputmode` attribute of the element carrying `class`.
fn inputmode(component: &dyn Component, class: &str) -> Option<String> {
    let (_keep, root) = mount(|scope| component.render(scope, &[]));
    find_by_class(&root, class)
        .expect("the field exists")
        .get_attribute("inputmode")
}

/// A component's name, the component, and the class of its field.
type Case = (&'static str, Box<dyn Component>, &'static str);

/// Every text-entry component and the class of its `<input>`/`<textarea>`,
/// built with `inputmode` set to `value`.
fn each_component(value: &str) -> Vec<Case> {
    let inputmode = value.to_string();
    vec![
        (
            "TextInput",
            Box::new(TextInput {
                inputmode: inputmode.clone(),
                ..Default::default()
            }),
            "rinch-text-input__input",
        ),
        (
            "Textarea",
            Box::new(Textarea {
                inputmode: inputmode.clone(),
                ..Default::default()
            }),
            "rinch-textarea__input",
        ),
        (
            "PasswordInput",
            Box::new(PasswordInput {
                inputmode: inputmode.clone(),
                ..Default::default()
            }),
            "rinch-password-input__input",
        ),
        (
            "NumberInput",
            Box::new(NumberInput {
                inputmode: inputmode.clone(),
                ..Default::default()
            }),
            "rinch-number-input__input",
        ),
        (
            "ColorInput",
            Box::new(ColorInput {
                inputmode,
                ..Default::default()
            }),
            "rinch-color-input__input",
        ),
    ]
}

#[test]
fn each_text_component_writes_a_set_inputmode() {
    for value in ["numeric", "decimal", "email", "none"] {
        for (name, component, class) in each_component(value) {
            assert_eq!(
                inputmode(component.as_ref(), class).as_deref(),
                Some(value),
                "{name} with inputmode {value:?}"
            );
        }
    }
}

/// Empty is "not set", and it is the default everywhere: Chrome's default
/// `<input>` has no `inputmode`, and nothing here guesses one.
#[test]
fn an_empty_or_default_inputmode_writes_no_attribute() {
    for (name, component, class) in each_component("") {
        assert_eq!(inputmode(component.as_ref(), class), None, "{name}");
    }
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
        assert_eq!(inputmode(component.as_ref(), class), None, "{name} default");
    }
}

/// `inputmode` and `autocomplete` are independent: setting one leaves the
/// other as it was.
#[test]
fn inputmode_sits_beside_autocomplete() {
    let (_keep, root) = mount(|scope| {
        TextInput {
            autocomplete: "one-time-code".into(),
            inputmode: "numeric".into(),
            ..Default::default()
        }
        .render(scope, &[])
    });
    let field = find_by_class(&root, "rinch-text-input__input").expect("the field");
    assert_eq!(field.get_attribute("inputmode").as_deref(), Some("numeric"));
    assert_eq!(
        field.get_attribute("autocomplete").as_deref(),
        Some("one-time-code")
    );
}

#[test]
fn inputmode_takes_a_literal_and_a_reactive_closure_through_rsx() {
    let (_keep, root) = mount(|__scope| rsx! { div { NumberInput { inputmode: "decimal" } } });
    let read = |root: &NodeHandle, class: &str| {
        find_by_class(root, class)
            .expect("the field exists")
            .get_attribute("inputmode")
    };
    assert_eq!(
        read(&root, "rinch-number-input__input").as_deref(),
        Some("decimal")
    );

    // The `{|| expr}` form: the field follows the signal.
    let mode = Signal::new(String::from("numeric"));
    let (_keep, root) =
        mount(|__scope| rsx! { div { PasswordInput { inputmode: {move || mode.get()} } } });
    let class = "rinch-password-input__input";
    assert_eq!(read(&root, class).as_deref(), Some("numeric"));
    mode.set("text".into());
    assert_eq!(read(&root, class).as_deref(), Some("text"));
    mode.set(String::new());
    assert_eq!(
        read(&root, class),
        None,
        "an empty value removes the attribute"
    );
}

/// The handler `data-onsubmit` names on `class`'s field runs the component's
/// `onsubmit`, once per dispatch.
fn submit_runs_onsubmit(build: impl FnOnce(Callback) -> Box<dyn Component>, class: &str) {
    let submits = Rc::new(Cell::new(0));
    let s = submits.clone();
    let component = build(Callback::new(move || s.set(s.get() + 1)));
    let (_keep, root) = mount(|scope| component.render(scope, &[]));
    assert_eq!(submits.get(), 0, "rendering does not submit");
    assert!(dispatch_event(handler(&root, class, "data-onsubmit")));
    assert_eq!(submits.get(), 1);
}

#[test]
fn a_password_input_submits_as_a_text_input_does() {
    submit_runs_onsubmit(
        |cb| {
            Box::new(TextInput {
                onsubmit: Some(cb),
                ..Default::default()
            })
        },
        "rinch-text-input__input",
    );
    submit_runs_onsubmit(
        |cb| {
            Box::new(PasswordInput {
                onsubmit: Some(cb),
                ..Default::default()
            })
        },
        "rinch-password-input__input",
    );
}

/// No `onsubmit`, no `data-onsubmit`: Enter in a lone password field does not
/// submit anything (the default, as before).
#[test]
fn a_password_input_without_onsubmit_writes_no_submit_handler() {
    let (_keep, root) = mount(|scope| PasswordInput::default().render(scope, &[]));
    let field = find_by_class(&root, "rinch-password-input__input").expect("the field");
    assert_eq!(field.get_attribute("data-onsubmit"), None);
}

/// The issue's shape, through `rsx!`: `onsubmit: move || …` on a
/// `PasswordInput` compiles (the macro wraps the closure) and is wired.
#[test]
fn a_password_input_takes_onsubmit_through_rsx() {
    let submits = Rc::new(Cell::new(0));
    let s = submits.clone();
    let (_keep, root) = mount(|__scope| {
        rsx! { div { PasswordInput { onsubmit: move || s.set(s.get() + 1) } } }
    });
    assert!(dispatch_event(handler(
        &root,
        "rinch-password-input__input",
        "data-onsubmit"
    )));
    assert_eq!(submits.get(), 1);
}
