//! A write that replaces an element's child list tells the removal observer
//! above it, in Chrome (issue #1440): `NodeHandle::set_text` on an element and
//! `NodeHandle::set_inner_html`.
//!
//! The browser's `textContent = …` detaches the children (they stay in
//! `rinch-web`'s node table, re-insertable, as after `remove()`), and the text
//! node it makes has no rinch id — so the element reads back no children.
//!
//! Run with `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner`
//! and a chromedriver matching the installed Chrome.
#![cfg(target_arch = "wasm32")]
use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope, on_child_removed};
use rinch_web::web_document::WebDocument;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

struct Fixture {
    _doc: Rc<RefCell<WebDocument>>,
    _scope: RenderScope,
    wrapper: NodeHandle,
    kids: Vec<NodeHandle>,
    seen: Rc<RefCell<Vec<NodeId>>>,
}

/// `host > div (observed) > section#wrapper > (span, span)`.
fn fixture() -> Fixture {
    let d = web_sys::window().unwrap().document().unwrap();
    let host = d.create_element("div").unwrap();
    d.body().unwrap().append_child(&host).unwrap();
    let doc = Rc::new(RefCell::new(WebDocument::new_into(d, host)));
    let body = doc.borrow().body();
    let body = NodeHandle::new(body, Rc::downgrade(&doc) as _);
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut scope = RenderScope::new(dyn_doc, body.node_id());

    let root = scope.create_element("div");
    body.append_child(&root);
    let wrapper = scope.create_element("section");
    root.append_child(&wrapper);
    let kids: Vec<_> = (0..2)
        .map(|_| {
            let k = scope.create_element("span");
            wrapper.append_child(&k);
            k
        })
        .collect();

    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    on_child_removed(&root, move |node| sink.borrow_mut().push(node.node_id()));

    // Positive control: the observer hears an ordinary removal here.
    let extra = scope.create_element("i");
    wrapper.append_child(&extra);
    wrapper.remove_child(&extra);
    assert_eq!(*seen.borrow(), vec![wrapper.node_id()], "control");
    seen.borrow_mut().clear();

    Fixture {
        _doc: doc,
        _scope: scope,
        wrapper,
        kids,
        seen,
    }
}

#[wasm_bindgen_test]
fn text_written_over_an_elements_children_tells_the_removal_observer() {
    let f = fixture();

    f.wrapper.set_text("gone");

    assert!(
        f.kids.iter().all(|k| k.parent_node().is_none()),
        "precondition: the browser detached both spans"
    );
    assert_eq!(
        *f.seen.borrow(),
        vec![f.wrapper.node_id()],
        "#1440: the spans left `wrapper`"
    );

    // The text the browser made has no id, so there is no child left to lose:
    // a second write over it tells nobody.
    f.seen.borrow_mut().clear();
    f.wrapper.set_text("again");
    assert!(f.seen.borrow().is_empty());

    // Detached, not retired: a span goes back in.
    f.wrapper.append_child(&f.kids[0]);
    assert_eq!(
        f.kids[0].parent_node().map(|p| p.node_id()),
        Some(f.wrapper.node_id())
    );
}

#[wasm_bindgen_test]
fn html_written_over_an_elements_children_tells_the_removal_observer() {
    let f = fixture();

    f.wrapper.set_inner_html("<b>new</b>");

    assert_eq!(
        *f.seen.borrow(),
        vec![f.wrapper.node_id()],
        "#1440: the replaced children left `wrapper`"
    );
    assert_eq!(
        f.wrapper.children().len(),
        1,
        "the parsed <b> is registered"
    );
}
