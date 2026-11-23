//! `Button`'s loading state and colour, against the props it already had
//! (issues #1334 and #2197).
//!
//! Until #2197 `loading: true` only added `rinch-button--loading`: no
//! `.rinch-button__label` and no `.rinch-button__loader` was ever created, so the
//! stylesheet's spinner and label-hiding rules matched nothing and a loading
//! button looked exactly like an idle one. And `color` accepted only a palette
//! name: `"#ff0000"` became `var(--rinch-color-#ff0000-6)`, which is invalid.
//!
//! Every fixture here uses only fields `Button` had before the fix, so each one
//! is red at the parent commit by assertion rather than by a compile error.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::Button;
use rinch_core::Component;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};

mod common;
use common::{find_by_class, has_class, sheet};

/// A rendered button with its document and scope kept alive.
struct Mounted {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
    child: NodeHandle,
}

fn mount(button: Button) -> Mounted {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let child = scope.create_text("Save");
    let root = button.render(&mut scope, std::slice::from_ref(&child));
    Mounted {
        _doc: doc,
        _scope: scope,
        root,
        child,
    }
}

/// A loading button keeps its label (so its width does not change) and adds
/// the loader beside it: the two elements `styles/button.rs` styles.
#[test]
fn a_loading_button_renders_its_label_and_a_loader() {
    let m = mount(Button {
        loading: true,
        ..Default::default()
    });
    let label = find_by_class(&m.root, "rinch-button__label")
        .expect("a loading button wraps its children in `.rinch-button__label`");
    assert!(
        label
            .children()
            .iter()
            .any(|c| c.node_id() == m.child.node_id()),
        "the caller's children are inside the label"
    );
    let loader = find_by_class(&m.root, "rinch-button__loader")
        .expect("a loading button renders `.rinch-button__loader`");
    let spinner = find_by_class(&loader, "rinch-loader")
        .expect("the loader slot holds the shared `Loader` component (#2205)");
    assert!(
        has_class(&spinner, "rinch-loader--oval"),
        "the default loader type is Loader's own default, oval"
    );
    assert!(
        find_by_class(&spinner, "rinch-loader__oval").is_some(),
        "an oval loader draws its oval"
    );

    // The rules that hide the label and place the loader exist.
    let css = sheet();
    assert!(css.contains(".rinch-button--loading .rinch-button__label"));
    assert!(css.contains(".rinch-button__loader {"));
}

/// An idle button is the tree it always was: the children directly under the
/// `<button>`, no label wrapper, no loader. (The visual default is unchanged.)
#[test]
fn an_idle_button_keeps_its_children_directly_under_the_button() {
    let m = mount(Button::default());
    let ids: Vec<_> = m.root.children().iter().map(|c| c.node_id()).collect();
    assert_eq!(ids, vec![m.child.node_id()]);
    assert!(find_by_class(&m.root, "rinch-button__label").is_none());
    assert!(find_by_class(&m.root, "rinch-button__loader").is_none());
    assert_eq!(
        m.root.get_attribute("type"),
        None,
        "no `type` is written by default: the HTML default, `submit`, is what a \
         `<button>` with no attribute already is"
    );
}

/// A raw colour is used as a colour; a palette name still goes through the
/// palette. The other components (ActionIcon, Loader) already did this.
#[test]
fn a_raw_colour_is_used_directly_and_a_palette_name_still_resolves() {
    for raw in ["#ff0000", "rgb(10, 20, 30)", "hsl(120 50% 50%)"] {
        let m = mount(Button {
            color: raw.to_string(),
            ..Default::default()
        });
        let style = m.root.get_attribute("style").expect("a coloured button");
        assert!(
            style.contains(&format!("--rinch-button-color: {raw};")),
            "{raw}: the colour itself, got {style:?}"
        );
        assert!(
            !style.contains("var(--rinch-color-#")
                && !style.contains("var(--rinch-color-rgb")
                && !style.contains("var(--rinch-color-hsl"),
            "{raw}: never a palette lookup of a raw colour, got {style:?}"
        );
        assert!(has_class(&m.root, "rinch-button--colored"));
    }

    let m = mount(Button {
        color: "red".to_string(),
        ..Default::default()
    });
    let style = m.root.get_attribute("style").expect("a coloured button");
    assert!(
        style.contains("--rinch-button-color: var(--rinch-color-red-6);")
            && style.contains("--rinch-button-color-hover: var(--rinch-color-red-7);")
            && style.contains("--rinch-button-color-light: var(--rinch-color-red-light);"),
        "a palette name keeps its palette shades, got {style:?}"
    );
}
