//! Browser-driven fixtures for issue #733: a `for` view that builds lazily and
//! memoises. Run as `reinsertion.rs` is (chromedriver matching Chrome).
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_core::{Signal, for_each_dom_typed};
use rinch_web::web_document::WebDocument;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn browser_document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn doc() -> (Rc<RefCell<WebDocument>>, web_sys::Element) {
    let h = browser_document().create_element("div").unwrap();
    browser_document().body().unwrap().append_child(&h).unwrap();
    (
        Rc::new(RefCell::new(WebDocument::new_into(
            browser_document(),
            h.clone(),
        ))),
        h,
    )
}

fn scope_for(doc: &Rc<RefCell<WebDocument>>) -> RenderScope {
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    RenderScope::new(dyn_doc, body)
}

/// PROBE: the issue's shape, built through the row's own scope.
#[wasm_bindgen_test]
fn probe_a_lazily_built_memoised_row_comes_back() {
    let (doc, host) = doc();
    let mut scope = scope_for(&doc);
    let body = NodeHandle::new(doc.borrow().body(), Rc::downgrade(&doc) as _);
    let cache: Rc<RefCell<HashMap<u32, NodeHandle>>> = Rc::default();
    let rows = Signal::new(vec![1u32]);
    let c = cache.clone();
    for_each_dom_typed(
        &mut scope,
        &body,
        move || rows.get(),
        |n: &u32| n.to_string(),
        move |n: u32, s: &mut RenderScope| {
            let hit = c.borrow().get(&n).cloned();
            hit.unwrap_or_else(|| {
                let row = s.create_element("article");
                let t = s.create_text("ROW733");
                row.append_child(&t);
                c.borrow_mut().insert(n, row.clone());
                row
            })
        },
    );
    assert!(
        host.text_content().unwrap_or_default().contains("ROW733"),
        "positive control: mounted"
    );
    rows.set(vec![]);
    assert!(
        !host.text_content().unwrap_or_default().contains("ROW733"),
        "positive control: removed"
    );
    rows.set(vec![1]);
    assert!(
        host.text_content().unwrap_or_default().contains("ROW733"),
        "#733: the memoised row must come back; host = {:?}",
        host.inner_html()
    );
}

/// PROBE: the same cache, built through a parentless scope the cache keeps.
#[wasm_bindgen_test]
fn probe_a_row_built_through_a_kept_parentless_scope_comes_back() {
    let (doc, host) = doc();
    let mut scope = scope_for(&doc);
    let body = NodeHandle::new(doc.borrow().body(), Rc::downgrade(&doc) as _);
    let cache: Rc<RefCell<HashMap<u32, (NodeHandle, RenderScope)>>> = Rc::default();
    let rows = Signal::new(vec![1u32]);
    let label = Signal::new(String::from("a"));
    let c = cache.clone();
    for_each_dom_typed(
        &mut scope,
        &body,
        move || rows.get(),
        |n: &u32| n.to_string(),
        move |n: u32, s: &mut RenderScope| {
            if let Some((row, _)) = c.borrow().get(&n) {
                return row.clone();
            }
            let d = s.doc_weak().upgrade().unwrap();
            let mut keep = RenderScope::new(d, s.parent().node_id());
            let row = {
                let _o = keep.push_owner();
                let row = keep.create_element("article");
                let t = keep.create_text("KEPT733");
                row.append_child(&t);
                let r2 = row.clone();
                keep.create_effect(move || r2.set_attribute("data-l", &label.get()));
                row
            };
            c.borrow_mut().insert(n, (row.clone(), keep));
            row
        },
    );
    rows.set(vec![]);
    label.set("b".into());
    rows.set(vec![1]);
    assert!(host.text_content().unwrap_or_default().contains("KEPT733"));
    label.set("c".into());
    assert!(
        host.inner_html().contains("data-l=\"c\""),
        "effects of the kept scope still run: {:?}",
        host.inner_html()
    );
    let before = rinch_web::web_document::__node_registry_len();
    for i in 0..200 {
        rows.set(if i % 2 == 0 { vec![] } else { vec![1] });
    }
    assert_eq!(rinch_web::web_document::__node_registry_len(), before);
}
