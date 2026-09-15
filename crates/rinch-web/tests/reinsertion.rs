//! Browser-driven tests for re-inserting a removed `NodeHandle`, issue #719.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_core::{Signal, show_dom};
use rinch_web::web_document::WebDocument;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn browser_document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn host() -> web_sys::Element {
    let el = browser_document().create_element("div").unwrap();
    el.set_attribute("data-test-host-719", "true").unwrap();
    browser_document()
        .body()
        .unwrap()
        .append_child(&el)
        .unwrap();
    el
}

fn doc() -> (Rc<RefCell<WebDocument>>, web_sys::Element) {
    let h = host();
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

/// The #654 shape, through the real `show_dom`: a branch closure that returns a
/// **captured** `NodeHandle` rather than building a fresh one, which is what
/// `rsx!` generates for `if cond { {panel} }`.
#[wasm_bindgen_test]
fn a_captured_handle_comes_back_when_the_branch_is_shown_again() {
    let (doc, host) = doc();
    let mut scope = scope_for(&doc);
    let body = doc.borrow().body();
    let body_handle = NodeHandle::new(body, Rc::downgrade(&doc) as _);

    // The panel is built once, outside the branch, and captured by it.
    let panel = scope.create_element("section");
    let text = scope.create_text("PANEL");
    panel.append_child(&text);

    let visible = Signal::new(true);
    let captured = panel.clone();
    show_dom(
        &mut scope,
        &body_handle,
        move || visible.get(),
        move |_s: &mut RenderScope| captured.clone(),
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );

    assert!(
        host.text_content().unwrap_or_default().contains("PANEL"),
        "precondition: the branch is shown on the first pass"
    );

    visible.set(false);
    assert!(
        !host.text_content().unwrap_or_default().contains("PANEL"),
        "precondition: hiding the branch takes the panel out of the page"
    );

    visible.set(true);
    assert!(
        host.text_content().unwrap_or_default().contains("PANEL"),
        "#719: re-showing a captured NodeHandle must put its subtree back; page text was {:?}",
        host.text_content().unwrap_or_default()
    );
}

/// The mechanism underneath, with no reactive helper in the way: `remove_node`
/// then `append_child` of the same id.
#[wasm_bindgen_test]
fn append_child_after_remove_node_puts_the_subtree_back() {
    let (doc, host) = doc();
    let body = doc.borrow().body();

    let panel = doc.borrow_mut().create_element("div");
    let text = doc.borrow_mut().create_text("BACK");
    doc.borrow_mut().append_child(panel, text);
    doc.borrow_mut().append_child(body, panel);
    assert!(host.text_content().unwrap_or_default().contains("BACK"));

    doc.borrow_mut().remove_node(panel);
    assert!(!host.text_content().unwrap_or_default().contains("BACK"));

    doc.borrow_mut().append_child(body, panel);
    assert!(
        host.text_content().unwrap_or_default().contains("BACK"),
        "#719: append_child of a removed id must re-insert it; page text was {:?}",
        host.text_content().unwrap_or_default()
    );
}
