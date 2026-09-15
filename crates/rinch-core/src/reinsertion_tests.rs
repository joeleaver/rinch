//! The post-condition of [`NodeHandle::remove`] and [`NodeHandle::discard`]
//! across the reactive helpers, issue #719 — over [`MockDomDocument`], which is
//! the host-runnable oracle for both real backends.
//!
//! # Why the mock is the right oracle here
//!
//! It does not paint or lay anything out; what it models is exactly the
//! bookkeeping the two backends disagreed about. Before #719 it faithfully
//! emulated the **browser** rule — `remove_node` dropped the subtree from its
//! table — so a caller that toggled a captured handle failed here as well as in
//! Chrome, and every fixture below was red on the host. That is why the
//! divergence is testable without a browser at all.
//!
//! The rule, in one sentence: **`remove` detaches, `discard` retires**, and a
//! reactive helper picks whichever matches what it knows about the subtree's
//! future. The browser half is pinned in real Chrome by
//! `crates/rinch-web/tests/reinsertion.rs`; the desktop half against the real
//! `RinchDocument` by `crates/rinch-dom/tests/reinsertion_handle_tests.rs`.
//!
//! # Each fixture is a pair
//!
//! A suite that only ever asserts "the subtree came back" is passed by a
//! backend that retires nothing at all, which leaks without bound — the very
//! thing #184 added the prune for. So each *re-show* fixture below has a
//! *discard* twin asserting the opposite verb still retires. Only the pair
//! distinguishes the fix from either degenerate backend.

use crate::dom::mock::MockDomDocument;
use crate::dom::{DomDocument, NodeHandle, RenderScope};
use crate::reactive::Signal;
use crate::{for_each_dom_typed, match_dom, show_dom};
use std::cell::RefCell;
use std::rc::Rc;

fn doc() -> Rc<RefCell<MockDomDocument>> {
    Rc::new(RefCell::new(MockDomDocument::new()))
}

fn body_handle(doc: &Rc<RefCell<MockDomDocument>>) -> NodeHandle {
    let body = doc.borrow().body();
    NodeHandle::new(body, Rc::downgrade(doc) as _)
}

fn scope(doc: &Rc<RefCell<MockDomDocument>>) -> RenderScope {
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    RenderScope::new(dyn_doc, body)
}

/// Tag names of the body's children, so a fixture reads the way the page does.
fn body_tags(doc: &Rc<RefCell<MockDomDocument>>) -> Vec<String> {
    let d = doc.borrow();
    d.get_children(d.body())
        .into_iter()
        .filter_map(|c| d.tag_name(c))
        .collect()
}

// ── the primitive: remove detaches, discard retires ─────────────────────────

/// The whole of #719 in four lines, with no helper in the way.
#[test]
fn a_removed_node_can_be_inserted_again_with_its_subtree() {
    let doc = doc();
    let body = doc.borrow().body();
    let panel = doc.borrow_mut().create_element("section");
    let inner = doc.borrow_mut().create_element("p");
    doc.borrow_mut().append_child(panel, inner);
    doc.borrow_mut().append_child(body, panel);

    doc.borrow_mut().remove_node(panel);
    assert!(
        doc.borrow().get_children(body).is_empty(),
        "precondition: remove takes it out of the tree"
    );

    doc.borrow_mut().append_child(body, panel);
    assert_eq!(body_tags(&doc), ["section"], "#719: the node comes back");
    assert_eq!(
        doc.borrow().get_children(panel),
        vec![inner],
        "#719: and so does its subtree"
    );
}

/// A removed node is still writable — it has left the tree, not the document.
/// A branch that restyles a hidden subtree before showing it again rests on
/// this, and the browser prune used to swallow the write.
#[test]
fn a_removed_node_can_still_be_written_to() {
    let doc = doc();
    let body = doc.borrow().body();
    let node = doc.borrow_mut().create_element("div");
    doc.borrow_mut().append_child(body, node);

    doc.borrow_mut().remove_node(node);
    doc.borrow_mut().set_attribute(node, "class", "later");

    assert_eq!(
        doc.borrow().get_attribute(node, "class").as_deref(),
        Some("later"),
        "#719: a removed node is detached, not gone"
    );
}

/// The twin. Discarding is the only route that retires an id, and a retired id
/// is a **silent no-op** everywhere — never a panic, and (since no backend
/// re-issues an id) never a write aimed at somebody else.
#[test]
fn a_discarded_node_is_a_silent_no_op_not_a_crash() {
    let doc = doc();
    let body = doc.borrow().body();
    let node = doc.borrow_mut().create_element("div");
    let inner = doc.borrow_mut().create_element("span");
    doc.borrow_mut().append_child(node, inner);
    doc.borrow_mut().append_child(body, node);

    doc.borrow_mut().discard_node(node);

    // Every one of these must simply do nothing, and none may panic.
    doc.borrow_mut().append_child(body, node);
    doc.borrow_mut().set_attribute(node, "class", "zombie");
    doc.borrow_mut().set_text_content(inner, "zombie");
    doc.borrow_mut().remove_node(node);
    doc.borrow_mut().discard_node(node);

    assert!(
        doc.borrow().get_children(body).is_empty(),
        "#719: a discarded node must not be relisted"
    );
    assert!(
        doc.borrow().tag_name(node).is_none() && doc.borrow().tag_name(inner).is_none(),
        "#184: the whole subtree is retired"
    );
}

// ── the helpers that may RE-SHOW ────────────────────────────────────────────

/// `show_dom` over a branch closure that returns a **captured** handle — the
/// `rsx!` shape `if cond { {panel} }`, and the exact report in #654/#719.
#[test]
fn show_dom_can_re_show_a_captured_handle() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let panel = sc.create_element("section");
    let inner = sc.create_element("p");
    panel.append_child(&inner);

    let visible = Signal::new(true);
    let captured = panel.clone();
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |_: &mut RenderScope| captured.clone(),
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    assert_eq!(body_tags(&doc), ["section"], "precondition: shown");

    visible.set(false);
    assert_eq!(
        body_tags(&doc),
        Vec::<String>::new(),
        "precondition: hidden"
    );

    visible.set(true);
    assert_eq!(
        body_tags(&doc),
        ["section"],
        "#719: re-showing a captured handle must put its subtree back"
    );
    assert_eq!(
        panel.children().len(),
        1,
        "#719: with its children still under it"
    );

    // Twice, so the fixture is not sitting on a single toggle: a backend that
    // retired on the *second* removal would pass a one-toggle test.
    visible.set(false);
    visible.set(true);
    assert_eq!(
        body_tags(&doc),
        ["section"],
        "#719: and on every later toggle"
    );
}

/// The same for `match_dom`: switching away from an arm and back must bring a
/// captured subtree with it.
#[test]
fn match_dom_can_re_show_a_captured_arm() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let first = sc.create_element("section");
    let second = sc.create_element("aside");

    let arm = Signal::new(0usize);
    let (a, b) = (first.clone(), second.clone());
    match_dom(
        &mut sc,
        &body,
        move || arm.get(),
        vec![
            Box::new(move |_: &mut RenderScope| a.clone())
                as Box<dyn Fn(&mut RenderScope) -> NodeHandle>,
            Box::new(move |_: &mut RenderScope| b.clone()),
        ],
    );
    assert_eq!(body_tags(&doc), ["section"]);

    arm.set(1);
    assert_eq!(body_tags(&doc), ["aside"]);

    arm.set(0);
    assert_eq!(
        body_tags(&doc),
        ["section"],
        "#719: switching back to an arm must restore its captured subtree"
    );
}

// ── the helpers that DISCARD ────────────────────────────────────────────────

/// A dropped `for` row is gone for good, so the helper says so and the backend
/// lets go. This is the half that keeps #184's leak closed: without it, a `for`
/// churning a list would pin every row it ever rendered.
#[test]
fn a_dropped_for_row_is_discarded() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let rows = Signal::new(vec![1u32, 2u32]);
    let built: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let seen = built.clone();
    for_each_dom_typed(
        &mut sc,
        &body,
        move || rows.get(),
        |n: &u32| n.to_string(),
        move |n: u32, s: &mut RenderScope| {
            let node = s.create_element("div");
            node.set_attribute("data-n", &n.to_string());
            seen.borrow_mut().push(node.clone());
            node
        },
    );
    assert_eq!(built.borrow().len(), 2, "precondition: two rows rendered");
    let doomed = built.borrow()[1].clone();

    rows.set(vec![1u32]);

    assert_eq!(
        doc.borrow().tag_name(doomed.node_id()),
        None,
        "#719: a dropped `for` row is discarded, so the backend can release it"
    );
}

/// A `for` row whose *data* changed is re-rendered, and the node it replaces is
/// discarded too — a second site in the same helper, and the one with no
/// `ListOp::Remove` to make it obvious.
#[test]
fn a_rerendered_for_row_discards_the_node_it_replaces() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let rows = Signal::new(vec![(1u32, "a".to_string())]);
    let built: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let seen = built.clone();
    for_each_dom_typed(
        &mut sc,
        &body,
        move || rows.get(),
        |r: &(u32, String)| r.0.to_string(),
        move |r: (u32, String), s: &mut RenderScope| {
            let node = s.create_element("div");
            node.set_attribute("data-v", &r.1);
            seen.borrow_mut().push(node.clone());
            node
        },
    );
    let first = built.borrow()[0].clone();

    // Same key, different data — the `Changed` arm, not `Remove`.
    rows.set(vec![(1u32, "b".to_string())]);

    assert_eq!(
        built.borrow().len(),
        2,
        "precondition: the row was re-rendered"
    );
    assert_eq!(
        doc.borrow().tag_name(first.node_id()),
        None,
        "#719: the replaced node is discarded, not merely detached"
    );
    assert_eq!(body_tags(&doc), ["div"], "exactly one row is mounted");
}

/// The component re-render effect throws its previous output away, so it
/// discards. Its `render_fn` builds afresh every run — the contract stated on
/// `reactive_component_dom` — and this is the fixture that holds it to it.
#[test]
fn a_rerendered_component_discards_its_previous_output() {
    use crate::dom::reactive_component_dom;

    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let version = Signal::new(0u32);
    let built: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let seen = built.clone();
    reactive_component_dom(&mut sc, &body, move |s: &mut RenderScope| {
        let node = s.create_element("div");
        node.set_attribute("data-v", &version.get().to_string());
        seen.borrow_mut().push(node.clone());
        node
    });
    let first = built.borrow()[0].clone();

    version.set(1);

    assert_eq!(built.borrow().len(), 2, "precondition: it re-rendered");
    assert_eq!(
        doc.borrow().tag_name(first.node_id()),
        None,
        "#719: the previous output is discarded, so the backend can release it"
    );
    assert_eq!(body_tags(&doc), ["div"], "exactly one output is mounted");
}
