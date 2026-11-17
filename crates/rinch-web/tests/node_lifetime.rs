//! Node and cache-scope lifetime in Chrome (issues #1454, #1456, #1508). The
//! host half, on the mock, is `rinch_core::node_lifetime_tests`.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{__retired_insertions, Cached, DomDocument, NodeHandle, RenderScope};
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

fn body_of(doc: &Rc<RefCell<WebDocument>>) -> NodeHandle {
    NodeHandle::new(doc.borrow().body(), Rc::downgrade(doc) as _)
}

/// #1508: a branch builds `div > (span, span)` and `remove()`s both spans,
/// dropping the handles. Each hide used to leave both in `NODE_REGISTRY`
/// (+200 per 100 cycles); now they go with the branch's scope.
#[wasm_bindgen_test]
fn removed_children_of_a_branch_go_with_the_branch_in_chrome() {
    let (doc, host) = doc();
    let mut scope = scope_for(&doc);
    let body = body_of(&doc);
    let open = Signal::new(false);
    show_dom(
        &mut scope,
        &body,
        move || open.get(),
        |s: &mut RenderScope| {
            let div = s.create_element("div");
            div.append_child(&s.create_text("SHOWN1508"));
            let (a, c) = (s.create_element("span"), s.create_element("span"));
            div.append_child(&a);
            div.append_child(&c);
            a.remove();
            c.remove();
            div
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    open.set(true);
    assert!(
        host.inner_html().contains("SHOWN1508"),
        "positive control: mounted"
    );
    open.set(false);
    let before = __node_registry_len();
    for i in 0..200 {
        open.set(i % 2 == 0);
    }
    assert_eq!(
        __node_registry_len() as isize - before as isize,
        0,
        "#1508: removed spans stranded"
    );
}

/// #1454: the cache is a `HashMap<u32, Cached>` owned by a component that
/// comes and goes. Dropping it evicts every entry: 50 mounts leave
/// `NODE_REGISTRY` flat, and a re-shown row inside one mount is the same node.
#[wasm_bindgen_test]
fn a_dropped_cache_of_entries_leaves_nothing_behind_in_chrome() {
    let (doc, host) = doc();
    let mut scope = scope_for(&doc);
    let body = body_of(&doc);
    let open = Signal::new(false);
    let label = Signal::new(String::from("a"));
    show_dom(
        &mut scope,
        &body,
        move || open.get(),
        move |s: &mut RenderScope| {
            let cache: Rc<RefCell<HashMap<u32, Cached>>> = Rc::default();
            let hostel = s.create_element("div");
            let rows = Signal::new(vec![1u32, 2]);
            for_each_dom_typed(
                s,
                &hostel,
                move || rows.get(),
                |n: &u32| n.to_string(),
                move |n: u32, s: &mut RenderScope| {
                    if let Some(hit) = cache.borrow().get(&n) {
                        return hit.node();
                    }
                    let entry = s.build_cached(|k| {
                        let row = k.create_element("article");
                        row.append_child(&k.create_text(&format!("KEPT{n};")));
                        let target = row.clone();
                        k.create_effect(move || target.set_attribute("data-l", &label.get()));
                        row
                    });
                    let row = entry.node();
                    cache.borrow_mut().insert(n, entry);
                    row
                },
            );
            hostel
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    open.set(true);
    assert!(
        host.inner_html().contains("KEPT2"),
        "positive control: mounted"
    );
    label.set("b".into());
    assert!(host.inner_html().contains("data-l=\"b\""), "and live");
    open.set(false);
    let before = __node_registry_len();
    for i in 0..100 {
        open.set(i % 2 == 0);
    }
    assert_eq!(
        __node_registry_len() as isize - before as isize,
        0,
        "#1454: a dropped cache stranded its rows"
    );
}

/// #1456: a row cached through the row's own scope, nested in a wrapper the
/// row builds: the wrapper comes back empty, and the insertion is counted.
#[wasm_bindgen_test]
fn a_retired_node_nested_in_fresh_markup_is_reported_in_chrome() {
    let (doc, host) = doc();
    let mut scope = scope_for(&doc);
    let body = body_of(&doc);
    let cache: Rc<RefCell<HashMap<u32, NodeHandle>>> = Rc::default();
    let rows = Signal::new(vec![1u32]);
    for_each_dom_typed(
        &mut scope,
        &body,
        move || rows.get(),
        |n: &u32| n.to_string(),
        move |n: u32, s: &mut RenderScope| {
            let hit = cache.borrow().get(&n).cloned();
            let row = hit.unwrap_or_else(|| {
                let row = s.create_element("article");
                row.append_child(&s.create_text("ROW1456"));
                cache.borrow_mut().insert(n, row.clone());
                row
            });
            let w = s.create_element("section");
            w.append_child(&row);
            w
        },
    );
    assert!(
        host.inner_html().contains("ROW1456"),
        "positive control: mounted"
    );
    let (seen, warned) = __retired_insertions();
    rows.set(vec![]);
    rows.set(vec![1]);
    assert!(!host.inner_html().contains("ROW1456"), "#733: still lost");
    assert!(
        host.inner_html().contains("<section>"),
        "the wrapper is back"
    );
    assert_eq!(
        __retired_insertions(),
        (seen + 1, warned + 1),
        "#1456: the nested retired node was inserted unreported"
    );
}
