//! #843: `visibility` is inherited and a descendant may set it back to
//! `visible`. Such a descendant is painted (#829), and Chrome 155's
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

/// Chrome 155: the visible child at (100, 70) answers the child; the hidden
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
/// also sets `pointer-events: auto` does. Chrome 155 agrees on both.
///
/// Mutant: testing `pointer-events` only for a visible node (or not at all for
/// a node reached through a hidden ancestor) — (100, 110) answers `inert`.
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
/// paints none, so none is offered.
///
/// Mutant: the pre-#843 early return in `find_scrollbar_hit_node` — the
/// visible scroller's bar is not found.
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
    div(&mut doc, hidden, "height: 1000px");
    doc.resolve_layout(800.0, 600.0);

    let hit = find_scrollbar_hit(&doc.tree, 212.0, 77.0).expect("the visible bar is grabbed");
    assert_eq!(hit.node_id, shown.0);
    assert!(
        find_scrollbar_hit(&doc.tree, 212.0, 177.0).is_none(),
        "a hidden scroller paints no bar and offers none"
    );
}

/// The issue's own document: a `visibility: visible` inline `<span>` inside a
/// hidden block, after a hidden block sibling. Chrome 153 answers the span's
/// element at a point inside its text. Here an inline span's text is hit as
/// its IFC root (`hit_test_node`'s "stretched IFC text" rule), and that root
/// is the hidden block; so the visible text answers no element of its own
/// here. What this fixture pins is the part that is the same: the hidden
/// block sibling, and the hidden root's empty area, fall through.
#[test]
fn the_issues_document_hidden_block_and_empty_area_fall_through() {
    let (mut doc, behind, overlay) =
        stacked("visibility: hidden; font-size: 16px; line-height: 20px");
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

    // The hidden blue block: x 20..120, y 30..70.
    assert_eq!(hit_test(&doc.tree, 70.0, 50.0), Some(behind.0));
    // Below the text line (y 70..90): the hidden overlay's own empty area.
    assert_eq!(hit_test(&doc.tree, 250.0, 150.0), Some(behind.0));
}
