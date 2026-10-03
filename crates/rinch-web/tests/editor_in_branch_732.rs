//! An `Editor` inside a `show_dom` branch, in Chrome (issue #732): its
//! raw-minted block nodes must be discarded with the branch.
#![cfg(target_arch = "wasm32")]
use rinch_core::Component;
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_core::reactive::Signal;
use rinch_core::show_dom;
use rinch_web::web_document::{__node_registry_len, WebDocument};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

fn mounted() -> (Rc<RefCell<WebDocument>>, NodeHandle, RenderScope) {
    let d = web_sys::window().unwrap().document().unwrap();
    let host = d.create_element("div").unwrap();
    d.body().unwrap().append_child(&host).unwrap();
    let doc = Rc::new(RefCell::new(WebDocument::new_into(d, host)));
    let body = doc.borrow().body();
    let bh = NodeHandle::new(body, Rc::downgrade(&doc) as _);
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let scope = RenderScope::new(dyn_doc, body);
    (doc, bh, scope)
}

#[wasm_bindgen_test]
fn an_editor_in_a_branch_does_not_grow_the_registry() {
    let (_doc, body, mut scope) = mounted();
    let visible = Signal::new(false);
    show_dom(
        &mut scope,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            let ed = rinch_web::Editor {
                content: "<p>one</p><p>two <strong>bold</strong></p><p>three</p>".into(),
                ..Default::default()
            };
            let n = ed.render(s, &[]);
            wrap.append_child(&n);
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    visible.set(true);
    visible.set(false);
    let base = __node_registry_len() as isize;
    for _ in 0..100 {
        visible.set(true);
        visible.set(false);
    }
    let g = __node_registry_len() as isize - base;
    assert_eq!(g, 0, "registry grew by {g} over 100 toggles");
}
