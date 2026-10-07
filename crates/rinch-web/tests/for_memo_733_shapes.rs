//! More shapes of a cache filled from inside a closure (issue #733), in
//! Chrome — from the review of PR #1452. The first file is `for_memo_733.rs`.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{__retired_view_returns, DomDocument, NodeHandle, RenderScope};
use rinch_core::{Signal, for_each_dom_typed, show_dom};
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

type Cache = Rc<RefCell<HashMap<u32, (NodeHandle, RenderScope)>>>;

fn cached(s: &mut RenderScope, c: &Cache, n: u32, label: Signal<String>) -> NodeHandle {
    if let Some((row, _)) = c.borrow().get(&n) {
        return row.clone();
    }
    let mut keep = s.cache_scope();
    let row = keep.build(|k| {
        let row = k.create_element("article");
        let t = k.create_text(&format!("KEPT{n};"));
        row.append_child(&t);
        let target = row.clone();
        k.create_effect(move || target.set_attribute("data-l", &label.get()));
        row
    });
    c.borrow_mut().insert(n, (row.clone(), keep));
    row
}

/// The brief's (2): a cached row nested inside row-built markup survives the
/// wrapper's discard in a real browser, the wrapper is reclaimed, no growth.
#[wasm_bindgen_test]
fn a_cached_node_inside_row_built_markup_survives_in_chrome() {
    let (doc, host) = doc();
    let mut scope = scope_for(&doc);
    let body = NodeHandle::new(doc.borrow().body(), Rc::downgrade(&doc) as _);
    let cache: Cache = Rc::default();
    let rows = Signal::new(vec![1u32]);
    let label = Signal::new(String::from("a"));
    let c = cache.clone();
    for_each_dom_typed(
        &mut scope,
        &body,
        move || rows.get(),
        |n: &u32| n.to_string(),
        move |n: u32, s: &mut RenderScope| {
            let row = cached(s, &c, n, label);
            let w = s.create_element("section");
            w.append_child(&row);
            w
        },
    );
    assert!(host.inner_html().contains("<section><article"), "control");
    rows.set(vec![]);
    assert!(!host.inner_html().contains("KEPT1"), "control: removed");
    label.set("b".into());
    rows.set(vec![1]);
    let html = host.inner_html();
    assert!(
        html.contains("<section><article data-l=\"b\">KEPT1;</article></section>"),
        "nested cached row came back: {html}"
    );
    let before = __node_registry_len();
    for i in 0..200 {
        rows.set(if i % 2 == 0 { vec![] } else { vec![1] });
    }
    assert_eq!(__node_registry_len(), before, "no growth over 200");
    assert_eq!(__retired_view_returns(), (0, 0));
}

/// The `for` Changed arm hands the same cached node back (`insert_after` of a
/// node after itself): still shown, in order, in a browser.
#[wasm_bindgen_test]
fn a_cached_row_whose_item_data_changes_stays_shown_in_chrome() {
    let (doc, host) = doc();
    let mut scope = scope_for(&doc);
    let body = NodeHandle::new(doc.borrow().body(), Rc::downgrade(&doc) as _);
    let cache: Cache = Rc::default();
    let rows = Signal::new(vec![(1u32, 0u32), (2, 0)]);
    let label = Signal::new(String::from("a"));
    let c = cache.clone();
    for_each_dom_typed(
        &mut scope,
        &body,
        move || rows.get(),
        |n: &(u32, u32)| n.0.to_string(),
        move |n: (u32, u32), s: &mut RenderScope| cached(s, &c, n.0, label),
    );
    assert_eq!(host.text_content().unwrap(), "KEPT1;KEPT2;", "control");
    let before = __node_registry_len();
    for i in 1..6 {
        rows.set(vec![(1, i), (2, 0)]);
        assert_eq!(host.text_content().unwrap(), "KEPT1;KEPT2;", "change {i}");
    }
    rows.set(vec![(2, 7), (1, 9)]);
    assert_eq!(
        host.text_content().unwrap(),
        "KEPT2;KEPT1;",
        "moved+changed"
    );
    assert_eq!(__node_registry_len(), before);
    assert_eq!(__retired_view_returns(), (0, 0));
}

/// FINDING: the cache's owner unmounts. Dropping the cache drops the scopes,
/// and every cached node stays in `NODE_REGISTRY` for the life of the page.
#[wasm_bindgen_test]
fn finding_a_dropped_cache_leaks_every_cached_node_in_chrome() {
    let (doc, host) = doc();
    let mut scope = scope_for(&doc);
    let body = NodeHandle::new(doc.borrow().body(), Rc::downgrade(&doc) as _);
    let open = Signal::new(false);
    let label = Signal::new(String::from("a"));
    show_dom(
        &mut scope,
        &body,
        move || open.get(),
        move |s: &mut RenderScope| {
            let cache: Cache = Rc::default();
            let hostel = s.create_element("div");
            let rows = Signal::new(vec![1u32, 2]);
            let c = cache.clone();
            for_each_dom_typed(
                s,
                &hostel,
                move || rows.get(),
                |n: &u32| n.to_string(),
                move |n: u32, s: &mut RenderScope| cached(s, &c, n, label),
            );
            hostel
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    open.set(true);
    assert!(host.inner_html().contains("KEPT2"), "control: mounted");
    open.set(false);
    let before = __node_registry_len();
    for i in 0..100 {
        open.set(i % 2 == 0);
    }
    let grew = __node_registry_len() as isize - before as isize;
    assert_eq!(grew, 200, "50 mounts x 2 rows x (article + text) stranded");
}

/// The same component with the recipe's `on_cleanup` drain — dispose each
/// scope, discard each node: 50 mounts leave `NODE_REGISTRY` where it was.
#[wasm_bindgen_test]
fn a_cache_drained_in_on_cleanup_leaves_nothing_behind_in_chrome() {
    let (doc, host) = doc();
    let mut scope = scope_for(&doc);
    let body = NodeHandle::new(doc.borrow().body(), Rc::downgrade(&doc) as _);
    let open = Signal::new(false);
    let label = Signal::new(String::from("a"));
    show_dom(
        &mut scope,
        &body,
        move || open.get(),
        move |s: &mut RenderScope| {
            let cache: Cache = Rc::default();
            let evict = cache.clone();
            s.on_cleanup(move || {
                for (_, (row, keep)) in evict.borrow_mut().drain() {
                    keep.dispose();
                    row.discard();
                }
            });
            let hostel = s.create_element("div");
            let rows = Signal::new(vec![1u32, 2]);
            let c = cache.clone();
            for_each_dom_typed(
                s,
                &hostel,
                move || rows.get(),
                |n: &u32| n.to_string(),
                move |n: u32, s: &mut RenderScope| cached(s, &c, n, label),
            );
            hostel
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    open.set(true);
    assert!(host.inner_html().contains("KEPT2"), "control: mounted");
    open.set(false);
    let before = __node_registry_len();
    for i in 0..100 {
        open.set(i % 2 == 0);
    }
    assert_eq!(
        __node_registry_len(),
        before,
        "#733: an evicting cache leaks nothing"
    );
}
