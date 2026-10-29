//! Every labelled field component associates its `<label>` with its field
//! (issue #2034).
//!
//! `TextInput`, `Textarea`, `NumberInput`, `PasswordInput` and `ColorInput`
//! rendered `<label class="…__label">` as a *sibling* of the field with no
//! `for` and no `id` on the field, so the label labelled nothing: a press on it
//! focused nothing in a browser, nor on desktop once a label forwards its press
//! to its control (#866). Mantine's `Input.Wrapper` associates the two through
//! a generated id — `htmlFor` on the label, `id` on the input, `{id}-label`,
//! `{id}-description` and `{id}-error` on the rest, and `aria-describedby` on
//! the input naming the error then the description. That is the markup pinned
//! here, with `id` (the field's own prop, Mantine's `id`) respected when the
//! app gives one.
//!
//! `Select`'s field is a `div role="combobox"`, which is not a labelable
//! element, so `for` cannot reach it on either backend: its label carries an
//! id the trigger names in `aria-labelledby`, and a press on the label focuses
//! the trigger through the label's own click handler.
//!
//! The browser half — a real press on the label focuses the field — is
//! `rinch-web/tests/field_label_focus_2034.rs`; the desktop half is
//! `rinch`'s `app::field_label_2034_tests`.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::color_input::ColorInput;
use rinch_components::number_input::NumberInput;
use rinch_components::password_input::PasswordInput;
use rinch_components::select::{Select, SelectOption};
use rinch_components::text_input::TextInput;
use rinch_components::textarea::Textarea;
use rinch_core::Component;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::dispatch_event;

mod common;
use common::{find_by_class, handler};

struct Mounted {
    doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
}

fn mount<C: Component>(component: C) -> Mounted {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = component.render(&mut scope, &[]);
    scope.parent().append_child(&root);
    Mounted {
        doc,
        _scope: scope,
        root,
    }
}

impl Mounted {
    fn part(&self, class: &str) -> NodeHandle {
        find_by_class(&self.root, class).unwrap_or_else(|| panic!("no .{class} rendered"))
    }
}

/// One labelled field component, by the classes of its parts.
struct Kind {
    name: &'static str,
    label: &'static str,
    field: &'static str,
    description: &'static str,
    error: &'static str,
    /// Renders it with this label, description, error and app `id` ("" = none).
    render: fn(&str, &str, &str, &str) -> Mounted,
}

const KINDS: &[Kind] = &[
    Kind {
        name: "TextInput",
        label: "rinch-text-input__label",
        field: "rinch-text-input__input",
        description: "rinch-text-input__description",
        error: "rinch-text-input__error",
        render: |label, description, error, id| {
            mount(TextInput {
                label: label.into(),
                description: description.into(),
                error: error.into(),
                id: id.into(),
                ..Default::default()
            })
        },
    },
    Kind {
        name: "Textarea",
        label: "rinch-textarea__label",
        field: "rinch-textarea__input",
        description: "rinch-textarea__description",
        error: "rinch-textarea__error",
        render: |label, description, error, id| {
            mount(Textarea {
                label: label.into(),
                description: description.into(),
                error: error.into(),
                id: id.into(),
                ..Default::default()
            })
        },
    },
    Kind {
        name: "NumberInput",
        label: "rinch-number-input__label",
        field: "rinch-number-input__input",
        description: "rinch-number-input__description",
        error: "rinch-number-input__error",
        render: |label, description, error, id| {
            mount(NumberInput {
                label: label.into(),
                description: description.into(),
                error: error.into(),
                id: id.into(),
                ..Default::default()
            })
        },
    },
    Kind {
        name: "PasswordInput",
        label: "rinch-password-input__label",
        field: "rinch-password-input__input",
        description: "rinch-password-input__description",
        error: "rinch-password-input__error",
        render: |label, description, error, id| {
            mount(PasswordInput {
                label: label.into(),
                description: description.into(),
                error: error.into(),
                id: id.into(),
                ..Default::default()
            })
        },
    },
    Kind {
        name: "ColorInput",
        label: "rinch-color-input__label",
        field: "rinch-color-input__input",
        description: "rinch-color-input__description",
        error: "rinch-color-input__error",
        render: |label, description, error, id| {
            mount(ColorInput {
                label: label.into(),
                description: description.into(),
                error: error.into(),
                id: id.into(),
                ..Default::default()
            })
        },
    },
];

fn attr(node: &NodeHandle, name: &str) -> Option<String> {
    node.get_attribute(name)
}

#[test]
fn each_label_names_its_field_by_id() {
    for k in KINDS {
        let m = (k.render)("Name", "", "", "");
        let field_id = attr(&m.part(k.field), "id")
            .unwrap_or_else(|| panic!("{}: the field carries no id", k.name));
        assert!(!field_id.is_empty(), "{}: empty field id", k.name);
        assert_eq!(
            attr(&m.part(k.label), "for").as_deref(),
            Some(field_id.as_str()),
            "{}: the label's `for` must be the field's id",
            k.name
        );
        assert_eq!(
            attr(&m.part(k.label), "id"),
            Some(format!("{field_id}-label")),
            "{}: the label's id (Mantine's `{{id}}-label`)",
            k.name
        );
    }
}

#[test]
fn two_instances_get_different_ids() {
    for k in KINDS {
        let a = (k.render)("A", "", "", "");
        let b = (k.render)("B", "", "", "");
        let ia = attr(&a.part(k.field), "id").unwrap();
        let ib = attr(&b.part(k.field), "id").unwrap();
        assert_ne!(ia, ib, "{}: two instances share a field id", k.name);
        assert_eq!(attr(&a.part(k.label), "for"), Some(ia), "{}", k.name);
        assert_eq!(attr(&b.part(k.label), "for"), Some(ib), "{}", k.name);
    }
}

#[test]
fn ids_never_repeat_across_kinds() {
    // The generator is shared: a TextInput and a NumberInput on one page must
    // not mint the same id either (a per-component counter would).
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..3 {
        for k in KINDS {
            let m = (k.render)("L", "", "", "");
            let id = attr(&m.part(k.field), "id").unwrap();
            assert!(seen.insert(id.clone()), "{}: id {id} minted twice", k.name);
        }
    }
}

#[test]
fn an_app_id_is_the_fields_id() {
    for k in KINDS {
        let m = (k.render)("Email", "Your work address", "Required", "email");
        assert_eq!(
            attr(&m.part(k.field), "id").as_deref(),
            Some("email"),
            "{}",
            k.name
        );
        assert_eq!(
            attr(&m.part(k.label), "for").as_deref(),
            Some("email"),
            "{}",
            k.name
        );
        assert_eq!(
            attr(&m.part(k.label), "id").as_deref(),
            Some("email-label"),
            "{}",
            k.name
        );
        assert_eq!(
            attr(&m.part(k.description), "id").as_deref(),
            Some("email-description"),
            "{}",
            k.name
        );
        assert_eq!(
            attr(&m.part(k.error), "id").as_deref(),
            Some("email-error"),
            "{}",
            k.name
        );
    }
}

#[test]
fn the_field_is_described_by_its_error_then_its_description() {
    for k in KINDS {
        // Both: Mantine's order, error first.
        let m = (k.render)("L", "Help", "Wrong", "");
        let field = m.part(k.field);
        let id = attr(&field, "id").unwrap();
        assert_eq!(
            attr(&field, "aria-describedby"),
            Some(format!("{id}-error {id}-description")),
            "{}",
            k.name
        );
        assert_eq!(
            attr(&m.part(k.error), "id"),
            Some(format!("{id}-error")),
            "{}",
            k.name
        );
        assert_eq!(
            attr(&m.part(k.description), "id"),
            Some(format!("{id}-description")),
            "{}",
            k.name
        );
        assert_eq!(
            attr(&field, "aria-invalid").as_deref(),
            Some("true"),
            "{}",
            k.name
        );

        // Description only.
        let m = (k.render)("L", "Help", "", "");
        let field = m.part(k.field);
        let id = attr(&field, "id").unwrap();
        assert_eq!(
            attr(&field, "aria-describedby"),
            Some(format!("{id}-description")),
            "{}",
            k.name
        );
        assert_eq!(attr(&field, "aria-invalid"), None, "{}", k.name);

        // Error only.
        let m = (k.render)("L", "", "Wrong", "");
        let field = m.part(k.field);
        let id = attr(&field, "id").unwrap();
        assert_eq!(
            attr(&field, "aria-describedby"),
            Some(format!("{id}-error")),
            "{}",
            k.name
        );

        // Neither: no dangling reference.
        let m = (k.render)("L", "", "", "");
        assert_eq!(
            attr(&m.part(k.field), "aria-describedby"),
            None,
            "{}",
            k.name
        );
    }
}

#[test]
fn an_unlabelled_field_still_gets_an_app_id() {
    // `id` is the field's id whether or not a label is rendered — an app's
    // own `<label for="email">` elsewhere on the page must find it.
    for k in KINDS {
        let m = (k.render)("", "", "", "email");
        assert_eq!(
            attr(&m.part(k.field), "id").as_deref(),
            Some("email"),
            "{}",
            k.name
        );
    }
}

// ------------------------------------------------------------------ Select

const SELECT_LABEL: &str = "rinch-select__label";
const SELECT_TRIGGER: &str = "rinch-select__input";

fn select(id: &str, disabled: bool) -> Mounted {
    mount(Select {
        label: "Fruit".into(),
        description: "Pick one".into(),
        error: "Required".into(),
        id: id.into(),
        disabled,
        data: vec![SelectOption::new("apple", "Apple")],
        ..Default::default()
    })
}

#[test]
fn select_trigger_is_labelled_by_its_label() {
    let a = select("", false);
    let b = select("", false);
    let trigger = a.part(SELECT_TRIGGER);
    let id = attr(&trigger, "id").expect("the trigger carries an id");
    assert_ne!(Some(id.clone()), attr(&b.part(SELECT_TRIGGER), "id"));
    let label_id = attr(&a.part(SELECT_LABEL), "id").expect("the label carries an id");
    assert_eq!(label_id, format!("{id}-label"));
    assert_eq!(attr(&trigger, "aria-labelledby"), Some(label_id));
    assert_eq!(
        attr(&trigger, "aria-describedby"),
        Some(format!("{id}-error {id}-description"))
    );
    assert_eq!(attr(&trigger, "aria-invalid").as_deref(), Some("true"));

    let app = select("fruit", false);
    assert_eq!(
        attr(&app.part(SELECT_TRIGGER), "id").as_deref(),
        Some("fruit")
    );
    assert_eq!(
        attr(&app.part(SELECT_TRIGGER), "aria-labelledby").as_deref(),
        Some("fruit-label")
    );
}

#[test]
fn a_press_on_the_select_label_focuses_the_trigger() {
    let m = select("", false);
    let trigger = m.part(SELECT_TRIGGER);
    assert_ne!(m.doc.borrow().active_element(), Some(trigger.node_id()));
    assert!(dispatch_event(handler(&m.root, SELECT_LABEL, "data-rid")));
    assert_eq!(m.doc.borrow().active_element(), Some(trigger.node_id()));
}

#[test]
fn a_disabled_select_label_focuses_nothing() {
    let m = select("", true);
    assert_eq!(attr(&m.part(SELECT_LABEL), "data-rid"), None);
}
