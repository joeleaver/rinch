//! A block whose only rendered inline content goes `display: none` by a class
//! change collapses, as a document built in that state does.
//!
//! The block was an IFC root, so the marking pass had detached its children
//! from its Taffy list — the inline span, and the hidden `<p>` beside it
//! (#487). Once the span is hidden too the block is no root, so nothing
//! re-detaches them, and `reattach_departed_ifc_children` must put them back.
//! It used to keep a hidden child out whenever its owner *could* be an IFC
//! root, which left the block childless in Taffy and one line (22px) tall.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

fn build(
    span_class: &str,
) -> (
    RinchDocument,
    rinch_core::dom::NodeId,
    rinch_core::dom::NodeId,
) {
    let mut doc = RinchDocument::new();
    doc.load_css(".e { display: none; }");
    let body = doc.body();
    let outer = doc.create_element("div");
    doc.append_child(body, outer);
    let p = doc.create_element("p");
    doc.set_attribute(p, "class", "e");
    doc.append_child(outer, p);
    let span = doc.create_element("span");
    doc.set_attribute(span, "class", span_class);
    doc.append_child(outer, span);
    let inner = doc.create_element("span");
    doc.append_child(span, inner);
    let t = doc.create_text("t");
    doc.append_child(inner, t);
    let input = doc.create_element("input");
    doc.append_child(span, input);
    doc.resolve_layout(800.0, 600.0);
    doc.resolve_layout(800.0, 600.0);
    (doc, outer, span)
}

#[test]
fn hiding_the_last_inline_collapses_the_block() {
    let (mut inc, outer, span) = build("c");
    assert!(
        inc.tree.nodes[outer.0].layout.height > 0.0,
        "shown: one line"
    );
    inc.set_attribute(span, "class", "c e");
    inc.resolve_layout(800.0, 600.0);
    let (fresh, fo, _) = build("c e");
    assert_eq!(fresh.tree.nodes[fo.0].layout.height, 0.0);
    assert_eq!(
        inc.tree.nodes[outer.0].layout.height,
        fresh.tree.nodes[fo.0].layout.height
    );
}
