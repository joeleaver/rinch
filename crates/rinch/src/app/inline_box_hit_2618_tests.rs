//! #2618: a press on an inline element's padding or border reached the
//! block, where Chrome 157's `elementFromPoint` finds the element. The hit
//! test found only glyphs (`text_node_at`), and the edge boxes that hold the
//! padding and border on a line (#1426) name no node. Where no glyph is under
//! the point, it now asks the boxes paint draws
//! (`rinch_dom::paint::inline_element_at`), the innermost first.
//!
//! The scene and Chrome's answers are in `rinch-dom`'s
//! `tests/support/inline_geom_2616.rs`: elements over two and three lines,
//! left-to-right and right-to-left, nested, with padding, border and margin,
//! and one with a padding alone.

use super::{event_target_at, hit_test};
use rinch_core::dom::{DomDocument, NodeId};

#[path = "../../../rinch-dom/tests/support/inline_geom_2616.rs"]
mod scene;

/// The element a press at `(x, y)` is delivered to, from the hit up, as the
/// click path starts its walk: its `id`, or `None` for an element with none
/// (the block).
fn pressed(doc: &rinch_dom::RinchDocument, x: f32, y: f32) -> Option<String> {
    let tree = &doc.tree;
    let hit = hit_test(tree, x, y)?;
    let mut at = event_target_at(tree, hit, x, y).unwrap_or(hit);
    if tree.get(at)?.is_text() {
        at = tree.get(at)?.parent?;
    }
    doc.get_attribute(NodeId(at), "id")
}

/// Every point lands where Chrome's lands: on the element whose padding or
/// border it is in, the inner one of two nested ones, and on the block in
/// the margin past an element's side.
#[test]
fn a_press_on_an_inline_elements_padding_or_border_reaches_the_element() {
    let doc = scene::document();
    let mut bad = Vec::new();
    for &(what, x, y, want) in scene::HITS {
        let got = pressed(&doc, x, y);
        if got.as_deref() != want {
            bad.push(format!("{what} ({x}, {y}): {got:?}, Chrome {want:?}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{} of {} points:\n{}",
        bad.len(),
        scene::HITS.len(),
        bad.join("\n")
    );
}
