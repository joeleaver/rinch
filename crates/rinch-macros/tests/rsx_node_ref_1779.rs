//! `ref: target` in `rsx!` (issue #1779): hand the app the `NodeHandle` of an
//! inner element through a `Signal<Option<NodeHandle>>`, and take it back when
//! the element goes.
//!
//! | site | fixture |
//! |---|---|
//! | an HTML element (`html::element_to_dom_html`) | `an_inner_element_is_handed_to_the_app` |
//! | an element in a branch (`show_dom`, its scope's cleanup) | `a_hidden_branch_clears_its_ref_and_a_shown_one_sets_it` |
//! | a `for` row | `a_removed_row_clears_its_ref` |
//! | a static component's root (`element_to_dom_component`) | `a_ref_on_a_component_is_its_root` |
//! | a re-rendering component (`element_to_dom_component_reactive`) | `a_re_rendering_component_rebinds_each_new_root` |
//! | the same, as a child statement (`generate_reactive_component_stmt`) | `a_re_rendering_component_child_rebinds_its_root` |

use rinch::prelude::*;
use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeId};

use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn mount(
    build: impl FnOnce(&mut RenderScope) -> NodeHandle,
) -> (Rc<RefCell<MockDomDocument>>, RenderScope, NodeHandle) {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = build(&mut scope);
    (doc, scope, root)
}

fn id_of(r: Signal<Option<NodeHandle>>) -> Option<NodeId> {
    r.get().map(|h| h.node_id())
}

fn find_class(node: &NodeHandle, class: &str) -> Option<NodeHandle> {
    if node.get_attribute("class").as_deref() == Some(class) {
        return Some(node.clone());
    }
    node.children().iter().find_map(|c| find_class(c, class))
}

// ── an HTML element ─────────────────────────────────────────────────────────

#[component]
fn inner_ref(r: Signal<Option<NodeHandle>>) -> NodeHandle {
    rsx! {
        div { class: "outer",
            p { "before" }
            span { class: "target", ref: r, "x" }
        }
    }
}

/// The handle is the inner element itself, not the rsx root, and it is in place
/// once the tree is built.
#[test]
fn an_inner_element_is_handed_to_the_app() {
    let r: Signal<Option<NodeHandle>> = Signal::new(None);
    let (_doc, _scope, root) = mount(|s| inner_ref(s, r));
    let held = r.get().expect("ref set by the render");
    assert_eq!(held.get_attribute("class").as_deref(), Some("target"));
    assert_eq!(
        Some(held.node_id()),
        find_class(&root, "target").map(|h| h.node_id())
    );
    assert_eq!(
        held.parent_node().map(|p| p.node_id()),
        Some(root.node_id()),
        "the element is attached by the time the render returns"
    );
}

#[component]
fn raw_ref(r: Signal<Option<NodeHandle>>) -> NodeHandle {
    rsx! { input { r#ref: r, class: "field" } }
}

/// `r#ref:` is the same prop, on a void element.
#[test]
fn a_raw_ref_on_a_void_element_is_bound() {
    let r: Signal<Option<NodeHandle>> = Signal::new(None);
    let (_doc, _scope, root) = mount(|s| raw_ref(s, r));
    assert_eq!(id_of(r), Some(root.node_id()));
}

// ── a branch ────────────────────────────────────────────────────────────────

#[component]
fn branch_ref(open: Signal<bool>, r: Signal<Option<NodeHandle>>) -> NodeHandle {
    rsx! {
        div {
            if open.get() {
                section { class: "panel", ref: r }
            } else {
                aside { class: "closed", ref: r }
            }
        }
    }
}

/// Hiding a branch clears the ref its element set, unless the other branch's
/// element has already taken it; either way the ref ends on the element shown.
#[test]
fn a_hidden_branch_clears_its_ref_and_a_shown_one_sets_it() {
    let open = Signal::new(true);
    let r: Signal<Option<NodeHandle>> = Signal::new(None);
    let (_doc, _scope, root) = mount(|s| branch_ref(s, open, r));
    let panel = id_of(r).expect("first branch bound");
    assert_eq!(Some(panel), find_class(&root, "panel").map(|h| h.node_id()));

    open.set(false);
    let closed = find_class(&root, "closed").expect("second branch shown");
    assert_eq!(id_of(r), Some(closed.node_id()));

    open.set(true);
    let panel2 = find_class(&root, "panel").expect("first branch shown again");
    assert_eq!(id_of(r), Some(panel2.node_id()));
}

#[component]
fn lone_branch_ref(open: Signal<bool>, r: Signal<Option<NodeHandle>>) -> NodeHandle {
    rsx! {
        div {
            if open.get() {
                section { class: "panel", ref: r }
            }
        }
    }
}

#[test]
fn a_hidden_branch_with_nothing_in_its_place_clears_the_ref() {
    let open = Signal::new(true);
    let r: Signal<Option<NodeHandle>> = Signal::new(None);
    let (_doc, _scope, _root) = mount(|s| lone_branch_ref(s, open, r));
    assert!(r.get().is_some());
    open.set(false);
    assert!(r.get().is_none(), "the element went, so the ref is cleared");
    open.set(true);
    assert!(r.get().is_some());
}

/// The clear subscribes nothing: an effect that reads the ref re-runs when it
/// changes, and the branch effect that ran the cleanup does not start
/// re-rendering on the ref's writes.
#[test]
fn a_ref_is_reactive_and_its_clear_subscribes_nothing() {
    let open = Signal::new(true);
    let r: Signal<Option<NodeHandle>> = Signal::new(None);
    let runs = Rc::new(Cell::new(0));
    let (_doc, _scope, root) = mount(|s| {
        let root = lone_branch_ref(s, open, r);
        let runs = runs.clone();
        s.create_effect(move || {
            let _ = r.get();
            runs.set(runs.get() + 1);
        });
        root
    });
    assert_eq!(runs.get(), 1);
    open.set(false);
    assert_eq!(runs.get(), 2, "the clear woke the reader once");
    open.set(true);
    assert_eq!(runs.get(), 3, "the new element woke it again");
    // The branch effect ran the old scope's cleanup when it re-ran; had the
    // cleanup's read been tracked, this write would rebuild the branch.
    open.set(false);
    open.set(true);
    let panel = find_class(&root, "panel").unwrap().node_id();
    r.set(None);
    assert_eq!(
        find_class(&root, "panel").map(|h| h.node_id()),
        Some(panel),
        "a write to the ref does not re-run the branch"
    );
    assert!(r.get().is_none());
}

// ── a for row ───────────────────────────────────────────────────────────────

#[component]
fn row_refs(ids: Signal<Vec<u32>>, r: Signal<Option<NodeHandle>>) -> NodeHandle {
    rsx! {
        ul {
            for id in ids.get() {
                li { key: id, class: {format!("row-{id}")}, ref: r }
            }
        }
    }
}

/// The last row built holds the ref; removing it clears the ref, and removing
/// a row that does not hold it leaves it alone.
#[test]
fn a_removed_row_clears_its_ref() {
    let ids = Signal::new(vec![1, 2]);
    let r: Signal<Option<NodeHandle>> = Signal::new(None);
    let (_doc, _scope, root) = mount(|s| row_refs(s, ids, r));
    let two = find_class(&root, "row-2").unwrap().node_id();
    assert_eq!(id_of(r), Some(two));

    ids.set(vec![2]);
    assert_eq!(id_of(r), Some(two), "row 1 did not hold the ref");

    ids.set(vec![]);
    assert_eq!(id_of(r), None, "row 2 held it and went");
}

// ── components ──────────────────────────────────────────────────────────────

#[component]
fn Tagged(label: String) -> NodeHandle {
    let root = __scope.create_element("section");
    root.set_attribute("class", "tagged");
    root.set_attribute("data-label", &label);
    root
}

#[component]
fn component_ref(r: Signal<Option<NodeHandle>>) -> NodeHandle {
    rsx! { div { Tagged { label: "a", ref: r } } }
}

#[test]
fn a_ref_on_a_component_is_its_root() {
    let r: Signal<Option<NodeHandle>> = Signal::new(None);
    let (_doc, _scope, root) = mount(|s| component_ref(s, r));
    let tagged = find_class(&root, "tagged").unwrap();
    assert_eq!(id_of(r), Some(tagged.node_id()));
}

#[component]
fn rerender_root_ref(label: Signal<String>, r: Signal<Option<NodeHandle>>) -> NodeHandle {
    rsx! { Tagged { label: {move || label.get()}, ref: r } }
}

/// A re-render builds a new root; the ref follows it, and the old render's
/// cleanup does not clear the new one.
#[test]
fn a_re_rendering_component_rebinds_each_new_root() {
    let label = Signal::new("a".to_string());
    let r: Signal<Option<NodeHandle>> = Signal::new(None);
    let (_doc, _scope, root) = mount(|s| rerender_root_ref(s, label, r));
    let first = find_class(&root, "tagged").unwrap();
    assert_eq!(id_of(r), Some(first.node_id()));

    label.set("b".to_string());
    let second = find_class(&root, "tagged").unwrap();
    assert_eq!(second.get_attribute("data-label").as_deref(), Some("b"));
    assert_ne!(
        second.node_id(),
        first.node_id(),
        "the component re-rendered"
    );
    assert_eq!(id_of(r), Some(second.node_id()));
}

#[component]
fn rerender_child_ref(label: Signal<String>, r: Signal<Option<NodeHandle>>) -> NodeHandle {
    rsx! { div { class: "host", Tagged { label: {move || label.get()}, ref: r } } }
}

#[test]
fn a_re_rendering_component_child_rebinds_its_root() {
    let label = Signal::new("a".to_string());
    let r: Signal<Option<NodeHandle>> = Signal::new(None);
    let (_doc, _scope, root) = mount(|s| rerender_child_ref(s, label, r));
    let first = find_class(&root, "tagged").unwrap();
    assert_eq!(id_of(r), Some(first.node_id()));
    label.set("b".to_string());
    let second = find_class(&root, "tagged").unwrap();
    assert_ne!(second.node_id(), first.node_id());
    assert_eq!(id_of(r), Some(second.node_id()));
}

/// The handle works from a handler like any other: here it reads back an
/// attribute an effect wrote after mount.
#[test]
fn the_handle_reads_the_live_element() {
    let r: Signal<Option<NodeHandle>> = Signal::new(None);
    let (_doc, _scope, _root) = mount(|s| inner_ref(s, r));
    let h = r.get().unwrap();
    h.set_attribute("data-x", "1");
    assert_eq!(
        r.get().unwrap().get_attribute("data-x").as_deref(),
        Some("1")
    );
}
