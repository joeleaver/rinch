//! `Stepper` releases the glyph it replaces, issue #719.
//!
//! `Stepper::render` fills a step's default completed/progress icon by clearing
//! the step's icon box and appending a fresh `render_tabler_icon` subtree. That
//! clearing loop is a **discard** in intent — the old glyph is replaced two
//! lines later and nothing holds a handle to it — and it was the one
//! `NodeHandle::remove()` site PR #728's first round missed.
//!
//! It matters because `rinch-components` is compiled for the browser
//! (`crates/rinch-web/Cargo.toml` depends on `rinch` with
//! `features = ["components"]`), where a merely-*removed* node keeps a strong
//! `web_sys::Node` in two page-global maps for the life of the wasm module. A
//! `Stepper` re-renders per step per signal change, and a Tabler glyph is a
//! whole SVG subtree, so `remove` here strands one of those every time.
//!
//! `MockDomDocument` is the host-runnable oracle for that bookkeeping: it
//! retires a discarded subtree and keeps a removed one, so `tag_name` answering
//! `None` is what tells the two verbs apart without a browser.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::stepper::{Stepper, StepperStep};
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, NodeId, RenderScope, mock::MockDomDocument};
use rinch_core::element::Component;
use rinch_tabler_icons::TablerIcon;

fn has_class(node: &NodeHandle, class: &str) -> bool {
    node.get_attribute("class")
        .unwrap_or_default()
        .split_whitespace()
        .any(|c| c == class)
}

fn find_by_class(node: &NodeHandle, class: &str) -> Option<NodeHandle> {
    if has_class(node, class) {
        return Some(node.clone());
    }
    node.children().iter().find_map(|c| find_by_class(c, class))
}

/// Every node in a subtree, so the assertion covers the whole discarded glyph
/// and not just its root.
fn subtree(node: &NodeHandle, out: &mut Vec<NodeId>) {
    out.push(node.node_id());
    for child in node.children() {
        subtree(&child, out);
    }
}

#[test]
fn the_default_icon_a_stepper_replaces_is_discarded() {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);

    // Render the step on its own first, so the glyph the stepper is about to
    // throw away can be captured before it is replaced.
    let step = StepperStep {
        state: "completed".into(),
        ..Default::default()
    }
    .render(&mut scope, &[]);

    let icon_box = find_by_class(&step, "rinch-stepper__step-icon")
        .expect("precondition: the step has an icon box");
    let mut doomed = Vec::new();
    for child in icon_box.children() {
        subtree(&child, &mut doomed);
    }
    assert!(
        !doomed.is_empty(),
        "precondition: the step rendered a default glyph for the stepper to replace"
    );

    // The stepper's own `completed_icon` replaces it.
    let _root = Stepper {
        completed_icon: Some(TablerIcon::CircleCheck),
        ..Default::default()
    }
    .render(&mut scope, &[step]);

    for id in doomed {
        assert_eq!(
            doc.borrow().tag_name(id),
            None,
            "#719: the glyph a Stepper replaces must be discarded — {id:?} is still \
             registered, which on rinch-web pins its SVG subtree for the life of the page"
        );
    }
    assert!(
        !icon_box.children().is_empty(),
        "precondition: the replacement glyph was appended"
    );
}
