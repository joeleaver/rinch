//! A childless block's one-line floor comes off when it gains a child.
//!
//! `ifc::apply_empty_block_line_floor` gives a childless block container a
//! `min-height` of one line on its Taffy style, and only a re-sync from its
//! computed values takes it off again. The first layout used to do that by
//! accident — its viewport branch re-cascaded every element — so a block built
//! empty and filled before the first layout lost its floor, and one filled
//! after it kept a line-tall minimum under shorter content for good. A resize
//! no longer re-cascades anything it does not reach, which removed the
//! accident; `note_first_child` re-syncs the block when its first child
//! arrives.

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
    assert_eq!(doc.tree.nodes[c.0].layout.height, 20.0, "empty: one line");
    let child = block(&mut doc, "height: 10px");
    doc.append_child(c, child);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.nodes[c.0].layout.height, 10.0,
        "a 10px child and no floor left behind"
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
    // `set_text_content` on an empty block: its line box, not the floor.
    doc.set_text_content(a, "text");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(doc.tree.nodes[b.0].layout.height, 10.0);
    assert_eq!(doc.tree.nodes[a.0].layout.height, 30.0);
}
