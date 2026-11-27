//! `Decoration::Node` in the browser: the class lands on the block's own
//! element, an app's sibling rule follows it, and it comes off again.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::RenderScope;
use rinch_core::element::ThemeProviderProps;
use rinch_editor_core::decoration::{Decoration, DecorationSet};
use rinch_editor_core::{EditorState, Plugin, PluginKey, Pos, Selection};
use rinch_web::create_editor;
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Folds what follows the first block, while switched on.
#[derive(Default)]
struct Fold(Cell<bool>);

impl Plugin for Fold {
    fn key(&self) -> PluginKey {
        PluginKey("test.node-decoration")
    }
    fn decorations(&self, state: &EditorState) -> DecorationSet {
        if !self.0.get() {
            return DecorationSet::empty();
        }
        let end = Pos(state.doc.child(0).node_size());
        DecorationSet::new(vec![Decoration::node(Pos(0), end, "folded")])
    }
}

fn element(selector: &str, n: u32) -> web_sys::Element {
    use wasm_bindgen::JsCast;
    document()
        .query_selector_all(selector)
        .unwrap()
        .item(n)
        .unwrap()
        .dyn_into()
        .unwrap()
}

#[wasm_bindgen_test]
fn a_node_decorations_class_restyles_the_block_and_its_next_sibling() {
    let style = document().create_element("style").unwrap();
    style.set_text_content(Some("[data-pm-editor] p.folded + p { display: none; }"));
    document().body().unwrap().append_child(&style).unwrap();
    let host = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&host).unwrap();
    let handle = create_editor();
    let fold = Rc::new(Fold::default());
    assert!(handle.add_plugin(fold.clone()));
    assert!(handle.load_html("<p>one</p><p>two</p>"));
    let mounted = handle.clone();
    let root = rinch_web::mount_into(
        &host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| mounted.mount(scope),
    );

    let hidden = || {
        element("[data-pm-editor] p", 1)
            .get_bounding_client_rect()
            .height()
            == 0.0
    };
    let class = |n| element("[data-pm-editor] p", n).get_attribute("class");
    let refresh = || assert!(handle.update(|st| Some(st.tr())));

    assert!(!hidden());
    fold.0.set(true);
    refresh();
    assert_eq!(class(0).as_deref(), Some("folded"));
    assert_eq!(class(1), None);
    assert!(hidden());

    // A retag builds the block's element anew.
    handle.set_selection(Selection::cursor(Pos(2)));
    assert!(handle.command("setHeading2"));
    assert_eq!(
        element("[data-pm-editor] h2", 0)
            .get_attribute("class")
            .as_deref(),
        Some("folded")
    );
    assert!(handle.command("setParagraph"));
    assert_eq!(class(0).as_deref(), Some("folded"));

    fold.0.set(false);
    refresh();
    assert_eq!(class(0), None);
    assert!(!hidden());

    root.unmount();
    host.remove();
    style.remove();
}
