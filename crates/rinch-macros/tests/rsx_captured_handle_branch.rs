//! `rsx!`'s `if` and `match` re-showing a **captured** `NodeHandle`, issue #719.
//!
//! This is the end-to-end shape the issue was reported from: a subtree built
//! once outside the branch, yielded by the branch body, and toggled. `rsx!`
//! lowers `if` to `show_dom` and `match` to `match_dom` with `move` branch
//! closures, and `IntoNode for NodeHandle` returns `self` — so the *same node
//! id* comes back on every show rather than a rebuild.
//!
//! These run over `MockDomDocument`, which since #719 is the host-runnable
//! oracle for both real backends: it detaches on `remove_node` and retires on
//! `discard_node`, the way `rinch-dom` and `rinch-web` now both do. The browser
//! half is pinned in real Chrome by `crates/rinch-web/tests/reinsertion.rs`.
//!
//! The lower-level fixtures (`show_dom`/`match_dom` called directly) are
//! `rinch_core::reinsertion_tests`. What these add is the **macro** step: they
//! are what fails if a future lowering rebuilds the branch body instead of
//! yielding the handle, which would keep the subtree's *markup* and silently
//! lose its identity.

use rinch::prelude::*;
use rinch_core::dom::DomDocument;
use rinch_core::dom::mock::MockDomDocument;
use std::cell::RefCell;
use std::rc::Rc;

/// Rendered text of a subtree, skipping the control-flow comment markers.
fn text(node: &NodeHandle) -> String {
    if node.node_type() == Some(8) {
        return String::new();
    }
    let children = node.children();
    if children.is_empty() {
        return node.text_content().unwrap_or_default();
    }
    children.iter().map(text).collect()
}

#[component]
fn if_over_a_captured_handle(open: Signal<bool>, panel: NodeHandle) -> NodeHandle {
    rsx! {
        div {
            if open.get() {
                {panel.clone()}
            }
        }
    }
}

#[test]
fn an_if_can_re_show_a_captured_handle_and_keeps_its_identity() {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut outer = RenderScope::new(doc.clone(), body);

    // Built once, outside the branch — and given some state of its own, so the
    // fixture can tell "the same subtree came back" from "markup that looks the
    // same was rebuilt".
    let panel = outer.create_element("section");
    let label = outer.create_text("PANEL");
    panel.append_child(&label);
    panel.set_attribute("data-built-once", "yes");
    let panel_id = panel.node_id();

    let open = Signal::new(true);
    let root = if_over_a_captured_handle(&mut outer, open, panel.clone());
    assert_eq!(
        text(&root),
        "PANEL",
        "precondition: shown on the first pass"
    );

    open.set(false);
    assert_eq!(text(&root), "", "precondition: hidden");

    open.set(true);
    assert_eq!(
        text(&root),
        "PANEL",
        "#719: re-showing a captured handle must put its subtree back"
    );
    assert_eq!(
        root.children()
            .iter()
            .filter(|c| c.node_type() != Some(8))
            .map(|c| c.node_id())
            .collect::<Vec<_>>(),
        vec![panel_id],
        "#719: and it must be the SAME node, not a rebuild"
    );
    assert_eq!(
        panel.get_attribute("data-built-once").as_deref(),
        Some("yes"),
        "#719: with whatever state it was carrying"
    );

    // A second round trip, so the fixture is not sitting on one toggle.
    open.set(false);
    open.set(true);
    assert_eq!(text(&root), "PANEL", "#719: on every later toggle too");
}

#[component]
fn match_over_captured_handles(arm: Signal<usize>, a: NodeHandle, b: NodeHandle) -> NodeHandle {
    rsx! {
        div {
            match arm.get() {
                0 => {a.clone()},
                _ => {b.clone()},
            }
        }
    }
}

#[test]
fn a_match_arm_can_re_show_a_captured_handle() {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut outer = RenderScope::new(doc.clone(), body);

    let first = outer.create_element("section");
    let first_text = outer.create_text("FIRST");
    first.append_child(&first_text);
    let second = outer.create_element("aside");
    let second_text = outer.create_text("SECOND");
    second.append_child(&second_text);
    let first_id = first.node_id();

    let arm = Signal::new(0usize);
    let root = match_over_captured_handles(&mut outer, arm, first.clone(), second.clone());
    assert_eq!(text(&root), "FIRST");

    arm.set(1);
    assert_eq!(text(&root), "SECOND");

    arm.set(0);
    assert_eq!(
        text(&root),
        "FIRST",
        "#719: switching back must restore the captured arm"
    );
    assert_eq!(
        root.children()
            .iter()
            .filter(|c| c.node_type() != Some(8))
            .map(|c| c.node_id())
            .collect::<Vec<_>>(),
        vec![first_id],
        "#719: and it must be the SAME node"
    );
}

// ── the scratch container a component site mints ────────────────────────────

#[component]
fn Card(label: String, children: &[NodeHandle]) -> NodeHandle {
    rsx! {
        div { class: "card", {label.clone()} }
    }
}

#[component]
fn card_branch(open: Signal<bool>) -> NodeHandle {
    rsx! {
        div {
            if open.get() {
                Card { label: "hi" }
            }
        }
    }
}

/// A component **site** mints a scratch `<template>` to build its children in,
/// and that container is in **no subtree** — it is never attached to anything.
/// So the recursive discard of a branch's content root cannot reach it, and one
/// orphan was stranded per component render (issue #719).
///
/// This is the counter-example to "a root this scope built takes its whole
/// subtree with it": true for everything *in* a subtree, and a node in none is
/// never reached. `release_scratch_container` is the site that knows.
///
/// Measured before the fix at **+100 over 200 toggles** on `rinch-web`; the same
/// shape here, over `MockDomDocument`, which counts the same quantity.
#[test]
fn a_component_site_in_a_branch_does_not_grow_the_document() {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);

    let open = Signal::new(false);
    let _root = card_branch(&mut scope, open);

    // Baselined after the first show/hide pair, so the initial mount is not
    // counted as growth.
    open.set(true);
    open.set(false);
    let baseline = doc.borrow().__node_count() as isize;
    for i in 2..200 {
        open.set(i % 2 == 0);
    }
    let delta = doc.borrow().__node_count() as isize - baseline;

    assert_eq!(
        delta, 0,
        "#719: a component site inside a branch must not strand its scratch \
         container — leaked {delta} nodes over 198 toggles"
    );
}

/// The other half: the component's **children** still reach it. A growth
/// fixture on its own is passed by a site that discards the children too.
#[test]
fn a_component_site_still_renders_its_children() {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);

    let root = rsx_card(&mut scope);
    assert_eq!(
        text(&root),
        "hi",
        "#719: releasing the scratch container must not take the rendered \
         content with it"
    );
}

#[component]
fn rsx_card() -> NodeHandle {
    rsx! {
        div {
            Card { label: "hi" }
        }
    }
}
