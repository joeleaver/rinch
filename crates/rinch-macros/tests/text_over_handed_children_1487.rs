//! `set_text` over children a component was HANDED, or keeps a handle to (review of PR #1507, #1487).
//!
//! A static `#[component]` renders with its caller's `__scope` (component
//! codegen: `Component::render(&comp, __scope, &children)`), so the children an
//! `rsx!` site hands it were minted by the same scope as the root the component
//! builds. A component that shows "Loading…" over its children with
//! `root.set_text(..)` and puts them back afterwards loses them under #1507's
//! rule on every backend that retires a discard (rinch-web, the mock), while
//! rinch-dom (discard = detach, #723) keeps them.

use rinch::prelude::*;
use rinch_core::dom::DomDocument;
use rinch_core::dom::mock::MockDomDocument;
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static ROOT: RefCell<Option<(NodeHandle, Vec<NodeHandle>)>> = const { RefCell::new(None) };
}

#[component]
pub fn Busy(children: &[NodeHandle]) -> NodeHandle {
    let root = __scope.create_element("section");
    for c in children {
        root.append_child(c);
    }
    ROOT.with(|r| *r.borrow_mut() = Some((root.clone(), children.to_vec())));
    root
}

#[component]
fn app() -> NodeHandle {
    rsx! {
        div {
            Busy { span { "one" } span { "two" } }
        }
    }
}

#[test]
fn a_component_toggling_text_over_its_handed_children_gets_them_back() {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut outer = RenderScope::new(doc.clone(), body);
    let _root = app(&mut outer);
    let (section, kids) = ROOT.with(|r| r.borrow().clone()).expect("rendered");
    assert_eq!(kids.len(), 2, "control: two children handed");

    // "Loading…" over the children, then the children back.
    section.set_text("Loading…");
    section.set_text("");
    for k in &kids {
        section.append_child(k);
    }
    let d = doc.borrow();
    for k in &kids {
        assert!(
            !d.is_retired(k.node_id()),
            "a child the component was handed by its caller is the caller's: \
             text written over it must not retire it"
        );
    }
    assert_eq!(d.get_children(section.node_id()).len(), 2);
}

/// Same component file, same scope: a handle the component keeps for itself.
#[component]
fn holder() -> NodeHandle {
    let label = rsx! { span { "content" } };
    let host = rsx! { div { {label.clone()} } };
    ROOT.with(|r| *r.borrow_mut() = Some((host.clone(), vec![label])));
    host
}

#[test]
fn a_handle_kept_by_the_same_component_survives_a_text_write() {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut outer = RenderScope::new(doc.clone(), body);
    let _ = holder(&mut outer);
    let (host, kept) = ROOT.with(|r| r.borrow().clone()).expect("rendered");
    host.set_text("Loading…");
    host.set_text("");
    host.append_child(&kept[0]);
    assert!(
        !doc.borrow().is_retired(kept[0].node_id()),
        "the documented pre-#1507 contract: a handle to an orphaned child re-inserts"
    );
}
