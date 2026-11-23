//! The `spellcheck` prop of the text components (#2862): `Some(on)` writes the
//! field's `spellcheck` attribute, `None` writes none, so the field inherits
//! the nearest ancestor's (on by default).

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::color_input::ColorInput;
use rinch_components::text_input::TextInput;
use rinch_components::textarea::Textarea;
use rinch_core::Component;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};

mod common;
use common::find_by_class;

fn render(component: &dyn Component, class: &str) -> Option<String> {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root: NodeHandle = component.render(&mut scope, &[]);
    find_by_class(&root, class)
        .expect("the field exists")
        .get_attribute("spellcheck")
}

#[test]
fn each_text_component_writes_its_spellcheck_prop() {
    for (on, want) in [
        (None, None),
        (Some(false), Some("false")),
        (Some(true), Some("true")),
    ] {
        let want = want.map(str::to_string);
        let text = TextInput {
            spellcheck: on,
            ..Default::default()
        };
        assert_eq!(
            render(&text, "rinch-text-input__input"),
            want,
            "TextInput {on:?}"
        );
        let area = Textarea {
            spellcheck: on,
            ..Default::default()
        };
        assert_eq!(
            render(&area, "rinch-textarea__input"),
            want,
            "Textarea {on:?}"
        );
        let color = ColorInput {
            spellcheck: on,
            ..Default::default()
        };
        assert_eq!(
            render(&color, "rinch-color-input__input"),
            want,
            "ColorInput {on:?}"
        );
    }
}
