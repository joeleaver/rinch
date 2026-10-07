//! Browser-driven fixtures for issue #733: a `for` view that builds lazily
//! and memoises. The host half, on the mock, is
//! `rinch_core::reinsertion_tests::lazy_memo_733`.
//!
//! Run as `reinsertion.rs` is (a chromedriver matching the installed Chrome).
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{__retired_view_returns, DomDocument, NodeHandle, RenderScope};
use rinch_core::{Signal, for_each_dom_typed};
use rinch_web::web_document::{__node_registry_len, WebDocument};
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

fn text(host: &web_sys::Element) -> String {
    host.text_content().unwrap_or_default()
}

type Cache = Rc<RefCell<HashMap<u32, (NodeHandle, RenderScope)>>>;

/// A list whose view builds each row once through a cache scope; `label`
/// drives an attribute on the cached row.
fn lazy_list(
    scope: &mut RenderScope,
    body: &NodeHandle,
    rows: Signal<Vec<u32>>,
    label: Signal<String>,
    cache: &Cache,
) {
    let c = cache.clone();
    for_each_dom_typed(
        scope,
        body,
        move || rows.get(),
        |n: &u32| n.to_string(),
        move |n: u32, s: &mut RenderScope| {
            if let Some((row, _)) = c.borrow().get(&n) {
                return row.clone();
            }
            let mut keep = s.cache_scope();
            let row = keep.build(|k| {
                let row = k.create_element("article");
                let t = k.create_text("KEPT733");
                row.append_child(&t);
                let target = row.clone();
                k.create_effect(move || target.set_attribute("data-l", &label.get()));
                row
            });
            c.borrow_mut().insert(n, (row.clone(), keep));
            row
        },
    );
}

/// The row comes back in a real browser, its effect ran while it was out and
/// still runs after, and 200 toggles leave the node registry where it was.
#[wasm_bindgen_test]
fn a_row_built_through_a_cache_scope_comes_back_and_does_not_leak() {
    let (doc, host) = doc();
    let mut scope = scope_for(&doc);
    let body = NodeHandle::new(doc.borrow().body(), Rc::downgrade(&doc) as _);
    let cache: Cache = Rc::default();
    let rows = Signal::new(vec![1u32]);
    let label = Signal::new(String::from("a"));
    let seen = __retired_view_returns();
    lazy_list(&mut scope, &body, rows, label, &cache);
    assert!(text(&host).contains("KEPT733"), "positive control: mounted");

    rows.set(vec![]);
    assert!(!text(&host).contains("KEPT733"), "positive control: removed");
    label.set("b".into());
    rows.set(vec![1]);
    assert!(text(&host).contains("KEPT733"), "#733: the row comes back");
    assert!(
        host.inner_html().contains("data-l=\"b\""),
        "#733: its effect ran while it was out: {}",
        host.inner_html()
    );
    label.set("c".into());
    assert!(host.inner_html().contains("data-l=\"c\""));

    let before = __node_registry_len();
    for i in 0..200 {
        rows.set(if i % 2 == 0 { vec![] } else { vec![1] });
    }
    assert_eq!(__node_registry_len(), before, "#733: no growth over 200");
    assert_eq!(__retired_view_returns(), seen, "nothing retired came back");
}

/// Evicting the cache entry — dispose the scope, discard the node — returns
/// the registry to where it was before the row existed, and the effect stops.
#[wasm_bindgen_test]
fn evicting_a_cache_entry_releases_the_row() {
    let (doc, host) = doc();
    let mut scope = scope_for(&doc);
    let body = NodeHandle::new(doc.borrow().body(), Rc::downgrade(&doc) as _);
    let cache: Cache = Rc::default();
    let rows = Signal::new(vec![]);
    let label = Signal::new(String::from("a"));
    lazy_list(&mut scope, &body, rows, label, &cache);
    let empty = __node_registry_len();

    rows.set(vec![1]);
    rows.set(vec![]);
    assert_eq!(__node_registry_len(), empty + 2, "precondition: cached");
    let (row, keep) = cache.borrow_mut().remove(&1).unwrap();
    keep.dispose();
    let element = doc.borrow().get_element(row.node_id());
    row.discard();
    assert_eq!(__node_registry_len(), empty, "#733: nodes released");
    label.set("z".into());
    if let Some(el) = element {
        assert_ne!(
            el.get_attribute("data-l").as_deref(),
            Some("z"),
            "#733: effects released"
        );
    }
    assert!(!text(&host).contains("KEPT733"));
}

/// The issue's own shape, built through the row's scope: still lost in a
/// browser, and now reported — once for the node, however often it returns.
#[wasm_bindgen_test]
fn a_row_built_through_its_own_scope_is_still_lost_and_reported_once() {
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
    assert!(text(&host).contains("ROW733"), "positive control: mounted");
    let (seen, warned) = __retired_view_returns();
    for _ in 0..3 {
        rows.set(vec![]);
        rows.set(vec![1]);
    }
    assert!(!text(&host).contains("ROW733"), "#733: still lost");
    assert_eq!(
        __retired_view_returns(),
        (seen + 3, warned + 1),
        "#733: every return is seen, one warning per node"
    );
}
