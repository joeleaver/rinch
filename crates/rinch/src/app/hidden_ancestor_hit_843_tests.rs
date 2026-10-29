//! #843: `visibility` is inherited and a descendant may set it back to
//! `visible`. Such a descendant is painted (#829), and Chrome 156's
//! `elementFromPoint` answers it, so it is hit here too; the hidden boxes
//! around it are not, and a point over them falls through to what is behind.
//!
//! Every document below puts a `behind` box under an absolutely positioned
//! `overlay` of the same size, so "falls through" has a target that is neither
//! the body nor `None`, and nothing sits at the viewport origin.

use super::{find_scrollbar_hit, hit_test};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::VisibilityValue;

fn div(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let el = doc.create_element("div");
    doc.set_attribute(el, "style", style);
    doc.append_child(parent, el);
    el
}

const BOX: &str = "position: absolute; left: 20px; top: 30px; width: 300px; height: 200px";

/// `(doc, behind, overlay)`; the overlay is painted over `behind`.
fn stacked(overlay_style: &str) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let behind = div(&mut doc, body, BOX);
    let overlay = div(&mut doc, body, &format!("{BOX}; {overlay_style}"));
    (doc, behind, overlay)
}

/// Chrome 156: the visible child at (100, 70) answers the child; the hidden
/// sibling at (100, 110) and the hidden overlay's own box at (250, 150)
/// answer `behind`.
///
/// Mutant: the pre-#843 early `return None` on a hidden node, which drops the
/// visible child with its ancestor — (100, 70) answers `behind`.
#[test]
fn a_visible_child_of_a_hidden_ancestor_is_hit_and_the_hidden_boxes_fall_through() {
    let (mut doc, behind, overlay) = stacked("visibility: hidden");
    // x 57..157, y 53..93.
    let shown = div(
        &mut doc,
        overlay,
        "visibility: visible; margin-left: 37px; margin-top: 23px; width: 100px; height: 40px",
    );
    // x 57..157, y 93..133; inherits `hidden`.
    let hidden = div(
        &mut doc,
        overlay,
        "margin-left: 37px; width: 100px; height: 40px",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.get(hidden.0).unwrap().computed_style.visibility,
        VisibilityValue::Hidden,
        "positive control: the plain child inherits `hidden`"
    );

    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(shown.0));
    assert_eq!(hit_test(&doc.tree, 100.0, 110.0), Some(behind.0));
    assert_eq!(hit_test(&doc.tree, 250.0, 150.0), Some(behind.0));
}

/// The same with the visible box one level further down, under a hidden
/// child of the hidden overlay: the descent does not stop at the first
/// hidden level either.
///
/// Mutant: descending into a hidden node's children only when the node's
/// *parent* is visible (one level of look-through) — answers `behind`.
#[test]
fn a_visible_grandchild_under_two_hidden_levels_is_hit() {
    let (mut doc, behind, overlay) = stacked("visibility: hidden");
    let middle = div(
        &mut doc,
        overlay,
        "margin-left: 37px; margin-top: 23px; width: 200px; height: 100px",
    );
    // x 68..148, y 64..94.
    let shown = div(
        &mut doc,
        middle,
        "visibility: visible; margin-left: 11px; margin-top: 11px; width: 80px; height: 30px",
    );
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(hit_test(&doc.tree, 100.0, 80.0), Some(shown.0));
    // Inside `middle`, outside `shown`.
    assert_eq!(hit_test(&doc.tree, 200.0, 120.0), Some(behind.0));
}

/// `pointer-events` is inherited as well: a `visible` child of a hidden
/// `pointer-events: none` overlay still takes no pointer events, and one that
/// also sets `pointer-events: auto` does. Chrome 156 agrees on both.
///
/// An interaction pin: the walk below a hidden node must still apply each
/// box's own `pointer-events`.
#[test]
fn pointer_events_none_still_applies_under_a_hidden_ancestor() {
    let (mut doc, behind, overlay) = stacked("visibility: hidden; pointer-events: none");
    // x 57..157, y 53..93.
    let live = div(
        &mut doc,
        overlay,
        "visibility: visible; pointer-events: auto; margin-left: 37px; margin-top: 23px; \
         width: 100px; height: 40px",
    );
    // x 57..157, y 93..133.
    let inert = div(
        &mut doc,
        overlay,
        "visibility: visible; margin-left: 37px; width: 100px; height: 40px",
    );
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(live.0));
    let hit = hit_test(&doc.tree, 100.0, 110.0);
    assert_eq!(hit, Some(behind.0));
    assert_ne!(hit, Some(inert.0));
}

/// Toggling an ancestor's `visibility` — by its `style` and by a class a
/// stylesheet matches — reaches the child that inherits it, and the next hit
/// test answers from the new value, though the stacking sequences and extents
/// were cached by the probe before the toggle.
///
/// This is the case the old early return's comment worried about ("Stylo not
/// cascading visibility to children when a parent's class changes"): with a
/// per-node test a child left with a stale `visible` would stay hittable. The
/// computed-style asserts are the positive control that it does not.
#[test]
fn toggling_an_ancestors_visibility_reaches_the_inheriting_child() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let style = doc.create_element("style");
    let css = doc.create_text(".off { visibility: hidden }");
    doc.append_child(style, css);
    doc.append_child(body, style);
    let behind = div(&mut doc, body, BOX);
    let overlay = div(&mut doc, body, BOX);
    // x 57..157, y 53..93.
    let child = div(
        &mut doc,
        overlay,
        "margin-left: 37px; margin-top: 23px; width: 100px; height: 40px",
    );
    doc.resolve_layout(800.0, 600.0);
    let vis = |doc: &RinchDocument| doc.tree.get(child.0).unwrap().computed_style.visibility;
    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(child.0));

    // By class.
    doc.set_attribute(overlay, "class", "off");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(vis(&doc), VisibilityValue::Hidden);
    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(behind.0));

    // The child opts back in.
    doc.set_attribute(
        child,
        "style",
        "visibility: visible; margin-left: 37px; margin-top: 23px; width: 100px; height: 40px",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(child.0));
    // ...and back out.
    doc.set_attribute(
        child,
        "style",
        "margin-left: 37px; margin-top: 23px; width: 100px; height: 40px",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(vis(&doc), VisibilityValue::Hidden);
    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(behind.0));

    // The class comes off; then the overlay is hidden by its `style` instead.
    doc.remove_attribute(overlay, "class");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(vis(&doc), VisibilityValue::Visible);
    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(child.0));
    doc.set_attribute(overlay, "style", &format!("{BOX}; visibility: hidden"));
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(vis(&doc), VisibilityValue::Hidden);
    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(behind.0));
}

/// A scroller that is `visibility: visible` inside a hidden ancestor paints
/// its bar (#829), so the bar is grabbed; a scroller that inherits `hidden`
/// paints none, so none is offered — even with visible content of its own,
/// which makes the walk enter it.
///
/// Mutants: the pre-#843 early return in `find_scrollbar_hit_node` (the
/// visible scroller's bar is not found); dropping the hidden node's own-bar
/// skip (the hidden scroller's bar is offered at (212, 177)).
#[test]
fn a_visible_scroller_under_a_hidden_ancestor_offers_its_bar_and_a_hidden_one_does_not() {
    let (mut doc, _behind, overlay) = stacked("visibility: hidden");
    // x 20..220, y 30..130; its vertical strip is x 204..220.
    let shown = div(
        &mut doc,
        overlay,
        "visibility: visible; width: 200px; height: 100px; overflow: auto",
    );
    div(&mut doc, shown, "height: 1000px");
    // y 130..230.
    let hidden = div(
        &mut doc,
        overlay,
        "width: 200px; height: 100px; overflow: auto",
    );
    div(&mut doc, hidden, "visibility: visible; height: 1000px");
    doc.resolve_layout(800.0, 600.0);

    let hit = find_scrollbar_hit(&doc.tree, 212.0, 77.0).expect("the visible bar is grabbed");
    assert_eq!(hit.node_id, shown.0);
    assert!(
        find_scrollbar_hit(&doc.tree, 212.0, 177.0).is_none(),
        "a hidden scroller paints no bar and offers none"
    );
}

/// The issue's own document: a `visibility: visible` inline `<span>` inside a
/// hidden block, after a hidden block sibling. Chrome 153 and 156 answer the span
/// at a point inside its text. An IFC's text is hit as its IFC root here (the
/// span has no box of its own to hit, visible root or not), so the visible
/// span's text answers that root — the anonymous block around the span — and
/// a click there reaches the span's ancestors. The hidden block sibling, the
/// hidden root's line beside the text and its empty area fall through.
///
/// Mutant: not testing the hidden root's text (`visible_text_at` answering
/// `false`) — the text falls through to `behind`.
#[test]
fn the_issues_document_visible_span_text_is_hit_and_the_rest_falls_through() {
    let (mut doc, behind, overlay) =
        stacked("visibility: hidden; font-size: 16px; line-height: 20px; font-family: sans-serif");
    div(
        &mut doc,
        overlay,
        "width: 100px; height: 40px; background: blue",
    );
    let span = doc.create_element("span");
    doc.set_attribute(span, "style", "visibility: visible");
    let text = doc.create_text("VIS");
    doc.append_child(span, text);
    doc.append_child(overlay, span);
    doc.resolve_layout(800.0, 600.0);
    let root = doc
        .tree
        .get(text.0)
        .unwrap()
        .ifc_root
        .expect("the span's text is laid out by an IFC");

    // The text line is y 70..90 from x 20; "V" is well under 10px wide.
    assert_eq!(hit_test(&doc.tree, 25.0, 80.0), Some(root));
    // The same line far right of the text: the hidden root's own area.
    assert_eq!(hit_test(&doc.tree, 250.0, 80.0), Some(behind.0));
    // The hidden blue block: x 20..120, y 30..70.
    assert_eq!(hit_test(&doc.tree, 70.0, 50.0), Some(behind.0));
    // Below the text line: the hidden overlay's own empty area.
    assert_eq!(hit_test(&doc.tree, 250.0, 150.0), Some(behind.0));
}

/// The text of a hidden element is hidden with it, and a text node takes its
/// visibility from its parent: in a hidden flex container a text run is a
/// flex item laid out as a box of its own (no IFC root), and computes no
/// `visibility` of its own. Walked because a visible sibling makes the
/// container's subtree worth entering, it still must not be hit.
///
/// Mutant: testing a text node's own computed `visibility` — the text node
/// answers at (40, 40).
#[test]
fn a_hidden_flex_containers_text_falls_through_beside_a_visible_item() {
    let (mut doc, behind, overlay) =
        stacked("visibility: hidden; display: flex; font-size: 16px; line-height: 20px");
    let text = doc.create_text("XXXXXXXX");
    doc.append_child(overlay, text);
    let shown = div(
        &mut doc,
        overlay,
        "visibility: visible; width: 50px; height: 20px",
    );
    doc.resolve_layout(800.0, 600.0);
    let t = doc.tree.get(text.0).unwrap();
    assert!(
        t.ifc_root.is_none() && t.layout.width > 30.0,
        "positive control: the text is a flex item with a box of its own"
    );
    let shown_x = 20.0 + doc.tree.get(shown.0).unwrap().layout.x;

    let hit = hit_test(&doc.tree, 40.0, 40.0);
    assert_eq!(hit, Some(behind.0));
    assert_ne!(hit, Some(text.0));
    assert_eq!(hit_test(&doc.tree, shown_x + 25.0, 40.0), Some(shown.0));
}

/// The other half of the issue's document: the hidden root's **own** text,
/// beside a visible span in the same line, is not hit. Chrome 156 answers
/// `behind` over it.
///
/// Mutant: answering any text under the point of a hidden root, whatever its
/// element's visibility — (25, 40) answers the root.
#[test]
fn a_hidden_roots_own_text_beside_a_visible_span_falls_through() {
    let (mut doc, behind, overlay) =
        stacked("visibility: hidden; font-size: 16px; line-height: 20px; font-family: sans-serif");
    let own = doc.create_text("HIDDEN HIDDEN ");
    doc.append_child(overlay, own);
    let span = doc.create_element("span");
    doc.set_attribute(span, "style", "visibility: visible");
    let text = doc.create_text("VIS");
    doc.append_child(span, text);
    doc.append_child(overlay, span);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.get(own.0).unwrap().ifc_root,
        Some(overlay.0),
        "positive control: the overlay is the IFC root of both texts"
    );

    // The line is y 30..50 from x 20; (25, 40) is on the first "H".
    assert_eq!(hit_test(&doc.tree, 25.0, 40.0), Some(behind.0));
}
