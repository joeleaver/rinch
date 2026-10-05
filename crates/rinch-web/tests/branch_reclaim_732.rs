//! A branch reclaims what it built, in Chrome (issue #732): a `virtual_list`
//! inside a `show_dom` branch, and `set_inner_html` over scope-built children.
//!
//! Run with `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner`
//! and a chromedriver matching the installed Chrome.
#![cfg(target_arch = "wasm32")]
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_core::reactive::Signal;
use rinch_core::{show_dom, virtual_list};
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
fn virtual_list_in_a_branch_does_not_grow_the_registry() {
    let (_doc, body, mut scope) = mounted();
    let visible = Signal::new(false);
    show_dom(
        &mut scope,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            let list = virtual_list(
                s,
                20.0,
                || (0u32..50).collect::<Vec<_>>(),
                |n: &u32| *n,
                2,
                |n: u32, rs: &mut RenderScope| {
                    let r = rs.create_element("div");
                    r.append_child(&rs.create_text(&n.to_string()));
                    r
                },
            );
            wrap.append_child(&list);
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

#[wasm_bindgen_test]
fn set_inner_html_over_scope_built_children_does_not_grow_the_minting_table() {
    let (_doc, body, mut scope) = mounted();
    let host = scope.create_element("div");
    body.append_child(&host);
    let reg0 = __node_registry_len() as isize;
    let mb0 = rinch_core::dom::__minted_by_len() as isize;
    for _ in 0..100 {
        for i in 0..10 {
            let c = scope.create_element("p");
            c.append_child(&scope.create_text(&i.to_string()));
            host.append_child(&c);
        }
        host.set_inner_html("");
    }
    let reg = __node_registry_len() as isize - reg0;
    let mb = rinch_core::dom::__minted_by_len() as isize - mb0;
    assert_eq!(
        (reg, mb),
        (0, 0),
        "registry growth {reg}, minting-table growth {mb}"
    );
}
