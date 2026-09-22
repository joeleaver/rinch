//! A layout pass that changes no text must not rebuild the text layout of an IFC
//! root laid out with no width constraint.
//!
//! `build_ifc_layouts` skips a root whose text is clean and whose previous layout
//! was built at the same `max_width`. A root whose content box is not positive
//! (a zero-width box, or one inside `display: none`) is built with `max_width:
//! None`, stored as `f32::INFINITY`, and the skip compared widths with
//! `(old - new).abs() < 0.01`: `INFINITY - INFINITY` is NaN, so every such root
//! was reshaped by Parley on every layout pass. In Pimble's window that was
//! 1,503 roots (collapsed tree rows, closed menus) on every keystroke, about
//! 125 ms of a 150 ms frame.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// Stamped into a root's kept layout. A rebuild replaces the whole layout, and
/// with it `text_content`, so the stamp survives exactly when the layout was
/// kept. (Comparing the boxed layout's address instead is not sound: a rebuild
/// can land at the address the old layout was freed from.)
const KEPT: &str = "\u{0}kept";

fn stamp(doc: &mut RinchDocument, id: NodeId) {
    let node = doc.tree.nodes.get_mut(id.0).unwrap();
    node.text_layout
        .as_mut()
        .expect("the root should carry a text layout")
        .text_content = KEPT.to_string();
}

fn was_kept(doc: &RinchDocument, id: NodeId) -> bool {
    let node = doc.tree.get(id.0).unwrap();
    node.text_layout
        .as_ref()
        .is_some_and(|layout| layout.text_content == KEPT)
}

fn max_width(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree
        .get(id.0)
        .unwrap()
        .text_layout
        .as_ref()
        .expect("the root should carry a text layout")
        .max_width
}

fn text_block(doc: &mut RinchDocument, parent: NodeId, style: &str, text: &str) -> NodeId {
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", style);
    doc.append_child(parent, div);
    let t = doc.create_text(text);
    doc.append_child(div, t);
    div
}

#[test]
fn a_layout_only_pass_keeps_the_text_layout_of_a_zero_width_root() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let unconstrained = text_block(&mut doc, body, "width: 0px", "no room to wrap");
    let constrained = text_block(&mut doc, body, "width: 200px", "plenty of room");
    let resized = doc.create_element("div");
    doc.set_attribute(resized, "style", "width: 100px; height: 10px");
    doc.append_child(body, resized);

    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        max_width(&doc, unconstrained),
        f32::INFINITY,
        "the premise: a zero-width root is built with no width constraint"
    );
    stamp(&mut doc, unconstrained);
    stamp(&mut doc, constrained);

    // A layout-only change elsewhere: no text and no display changes, so no
    // IFC root is text-dirty and `build_ifc_layouts` runs in rebuild-all mode.
    doc.set_attribute(resized, "style", "width: 150px; height: 10px");
    doc.resolve_layout(800.0, 600.0);

    assert!(
        was_kept(&doc, constrained),
        "a constrained root at an unchanged width is not rebuilt"
    );
    assert!(
        was_kept(&doc, unconstrained),
        "an unconstrained root with unchanged text is not rebuilt either"
    );
}

#[test]
fn an_unconstrained_root_that_gains_a_width_is_rebuilt() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let root = text_block(&mut doc, body, "width: 0px", "now it has room");

    doc.resolve_layout(800.0, 600.0);
    stamp(&mut doc, root);

    doc.set_attribute(root, "style", "width: 300px");
    doc.resolve_layout(800.0, 600.0);

    assert!(!was_kept(&doc, root), "a width change still rebuilds");
    assert_eq!(max_width(&doc, root), 300.0);
}
