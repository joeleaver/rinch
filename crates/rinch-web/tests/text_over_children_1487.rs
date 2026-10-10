//! Text written over an element's children, in a browser (issue #1487).
//!
//! `set_text_content` is `textContent = …` here: the old children leave the
//! page and stay in the node table, and no hide of the branch that built them
//! ever walks to them. `NodeHandle::set_text` still only detaches them; the
//! ones the written element's render built are discarded when its scope goes,
//! and the ones it was handed are the caller's.
//! The mock's twins are `reinsertion_tests::text_over_children_1487` in
//! `rinch-core`.
//!
//! Run with `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner`
//! and a chromedriver matching the installed Chrome.
#![cfg(target_arch = "wasm32")]
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_core::reactive::Signal;
use rinch_core::show_dom;
use rinch_web::web_document::{__node_registry_len, WebDocument};
use std::cell::{Cell, RefCell};
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

fn counts() -> (isize, isize) {
    (
        __node_registry_len() as isize,
        rinch_core::dom::__minted_by_len() as isize,
    )
}

/// `(registry, minting table)` growth over 100 show/hide pairs after a
/// warm-up pair.
fn cycle_growth(visible: Signal<bool>) -> (isize, isize) {
    visible.set(true);
    visible.set(false);
    let base = counts();
    for _ in 0..100 {
        visible.set(true);
        visible.set(false);
    }
    let now = counts();
    (now.0 - base.0, now.1 - base.1)
}

#[wasm_bindgen_test]
fn text_over_branch_built_children_does_not_grow_the_registry() {
    let (_doc, body, mut scope) = mounted();
    let visible = Signal::new(false);
    let minted = Rc::new(Cell::new(0isize));
    let seen = minted.clone();
    show_dom(
        &mut scope,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let before = __node_registry_len() as isize;
            let wrap = s.create_element("div");
            let a = s.create_element("span");
            a.append_child(&s.create_text("a"));
            wrap.append_child(&a);
            wrap.append_child(&s.create_element("span"));
            seen.set(__node_registry_len() as isize - before);
            wrap.set_text("over");
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    let grown = cycle_growth(visible);
    assert_eq!(
        minted.get(),
        4,
        "control: the registry counts the four nodes a show mints"
    );
    assert_eq!(
        grown,
        (0, 0),
        "(registry, minting table) growth over 100 show/hide cycles"
    );
}

/// A reactive text's shape: an element holding one scope-built text node,
/// written through the element. `textContent` replaces that text node.
#[wasm_bindgen_test]
fn text_over_an_elements_own_text_node_does_not_grow_the_registry() {
    let (_doc, body, mut scope) = mounted();
    let visible = Signal::new(false);
    show_dom(
        &mut scope,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let label = s.create_element("p");
            label.append_child(&s.create_text("one"));
            label.set_text("two");
            label
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    assert_eq!(cycle_growth(visible), (0, 0));
}

#[wasm_bindgen_test]
fn text_over_a_captured_handle_only_detaches_it() {
    let (doc, body, mut scope) = mounted();
    let panel = scope.create_element("article");
    panel.set_attribute("id", "kept-1487");
    let panel_text = scope.create_text("kept");
    panel.append_child(&panel_text);

    let visible = Signal::new(false);
    let fresh: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let (handed, built) = (panel.clone(), fresh.clone());
    show_dom(
        &mut scope,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            let own = s.create_element("span");
            wrap.append_child(&own);
            wrap.append_child(&handed);
            *built.borrow_mut() = Some(own);
            wrap.set_text("over");
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    visible.set(true);
    visible.set(false);

    let own = fresh.borrow().clone().expect("the branch rendered");
    assert!(
        doc.borrow().is_retired(own.node_id()),
        "control: the span the branch built beside the panel went with the branch"
    );
    assert!(!doc.borrow().is_retired(panel.node_id()));
    assert!(!doc.borrow().is_retired(panel_text.node_id()));
    let page = web_sys::window().unwrap().document().unwrap();
    assert!(
        page.get_element_by_id("kept-1487").is_none(),
        "the write took the panel out of the page"
    );

    body.append_child(&panel);
    let back = page
        .get_element_by_id("kept-1487")
        .expect("the captured panel re-inserts");
    assert_eq!(back.text_content().as_deref(), Some("kept"));
    panel.remove();

    assert_eq!(
        cycle_growth(visible),
        (0, 0),
        "with the panel kept, nothing else accumulates"
    );
    assert!(!doc.borrow().is_retired(panel.node_id()));
}
