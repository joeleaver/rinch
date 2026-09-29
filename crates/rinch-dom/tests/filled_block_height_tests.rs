//! A block that gains a child is as tall as that child — never a line tall.
//!
//! Written for the one-line `min-height` floor rinch used to give every
//! childless block container (`ifc::apply_empty_block_line_floor`), which only
//! a re-sync of the block's Taffy style took off again, so a block built empty
//! and filled after the first layout kept a line-tall minimum under shorter
//! content (`note_first_child` re-synced it when its first child arrived). The
//! floor and that re-sync are both gone (#296) — an empty block is 0 tall, as
//! in every browser — and these stay as pins on the filled heights, with the
//! empty one now asserting Chrome 153's 0.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

fn block(doc: &mut RinchDocument, style: &str) -> rinch_core::dom::NodeId {
    let e = doc.create_element("div");
    doc.set_attribute(e, "style", style);
    e
}

#[test]
fn a_block_filled_after_the_first_layout_loses_its_floor() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = block(&mut doc, "line-height: 20px");
    doc.append_child(body, c);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.nodes[c.0].layout.height, 0.0,
        "empty: Chrome 153: 0"
    );
    let child = block(&mut doc, "height: 10px");
    doc.append_child(c, child);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.nodes[c.0].layout.height, 10.0,
        "a 10px child and nothing left behind"
    );
}

#[test]
fn a_block_filled_before_the_first_layout_loses_its_floor() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = block(&mut doc, "line-height: 20px");
    doc.append_child(body, c);
    let child = block(&mut doc, "height: 10px");
    doc.append_child(c, child);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(doc.tree.nodes[c.0].layout.height, 10.0);
}

#[test]
fn a_block_filled_by_insert_child_or_set_text_loses_its_floor() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let a = block(&mut doc, "line-height: 30px; font-size: 10px");
    let b = block(&mut doc, "line-height: 20px");
    doc.append_child(body, a);
    doc.append_child(body, b);
    doc.resolve_layout(800.0, 600.0);
    // `insert_child` at index 0 of an empty block.
    let child = block(&mut doc, "height: 10px");
    doc.insert_child(b, child, 0);
    // `set_text_content` on an empty block: its line box.
    doc.set_text_content(a, "text");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(doc.tree.nodes[b.0].layout.height, 10.0);
    assert_eq!(doc.tree.nodes[a.0].layout.height, 30.0);
}
