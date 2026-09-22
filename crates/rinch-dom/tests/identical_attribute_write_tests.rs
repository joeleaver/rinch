//! Writing the value an attribute already has restyles nothing.
//!
//! A reactive attribute re-runs whenever anything it reads changes, and most of
//! the time it computes the same string again. Before this, `set_attribute`
//! re-resolved the node's whole subtree on every such write (`set_text_content`
//! has always skipped identical text): in Pimble a toolbar re-wrote fifteen
//! identical button styles on each keystroke and restyled about a hundred
//! nodes.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// A laid-out document with a styled, classed element holding a child, and no
/// dirty state left over.
fn settled() -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "card");
    doc.set_attribute(div, "style", "width: 100px; color: red");
    doc.set_attribute(div, "data-state", "open");
    doc.append_child(body, div);
    let text = doc.create_text("inside");
    doc.append_child(div, text);
    doc.resolve_layout(800.0, 600.0);
    doc.tree.dirty_nodes.clear();
    assert!(!doc.tree.styles_dirty && doc.tree.style_roots.is_empty());
    (doc, div)
}

fn restyle_queued(doc: &RinchDocument) -> bool {
    doc.tree.styles_dirty || !doc.tree.style_roots.is_empty() || !doc.tree.dirty_nodes.is_empty()
}

#[test]
fn an_identical_attribute_write_queues_no_restyle() {
    let (mut doc, div) = settled();
    doc.set_attribute(div, "class", "card");
    doc.set_attribute(div, "data-state", "open");
    doc.set_attribute(div, "style", "width: 100px; color: red");
    // Names are folded before the comparison, as they are when stored.
    doc.set_attribute(div, "Data-State", "open");
    assert!(!restyle_queued(&doc));
}

#[test]
fn an_identical_inline_style_merge_queues_no_restyle() {
    let (mut doc, div) = settled();
    doc.set_style(div, "width", "100px");
    doc.set_styles(div, &[("color", "red"), ("width", "100px")]);
    assert!(!restyle_queued(&doc));
}

#[test]
fn a_changed_attribute_still_restyles() {
    let (mut doc, div) = settled();
    doc.set_attribute(div, "data-state", "closed");
    assert!(restyle_queued(&doc));

    let (mut doc, div) = settled();
    doc.set_style(div, "color", "blue");
    assert!(restyle_queued(&doc));
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.get_attribute(div, "style").as_deref(),
        Some("width: 100px; color: blue")
    );
}
