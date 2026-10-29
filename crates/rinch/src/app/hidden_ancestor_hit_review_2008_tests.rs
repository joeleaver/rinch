//! Review of #2008 (#843): cache-invalidation and shape probes.
use super::{find_scrollbar_hit, hit_test};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::VisibilityValue;
use rinch_dom::transition::TransitionProperty;

fn div(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let el = doc.create_element("div");
    doc.set_attribute(el, "style", style);
    doc.append_child(parent, el);
    el
}
const BOX: &str = "position: absolute; left: 20px; top: 30px; width: 300px; height: 200px";
const KID: &str = "margin-left: 37px; margin-top: 23px; width: 100px; height: 40px";

fn stacked(overlay_style: &str) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let behind = div(&mut doc, body, BOX);
    let overlay = div(&mut doc, body, &format!("{BOX}; {overlay_style}"));
    (doc, behind, overlay)
}

/// Insert a visible child under a hidden ancestor whose `shows` bit was
/// cached `false`; then remove it again.
#[test]
fn probe_insert_and_remove_under_hidden() {
    let (mut doc, behind, overlay) = stacked("visibility: hidden");
    let _plain = div(&mut doc, overlay, "width: 10px; height: 10px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        hit_test(&doc.tree, 100.0, 70.0),
        Some(behind.0),
        "warm, shows=false"
    );
    let shown = doc.create_element("div");
    doc.set_attribute(shown, "style", &format!("visibility: visible; {KID}"));
    doc.append_child(overlay, shown);
    doc.resolve_layout(800.0, 600.0);
    // (100,70): plain is 10px at top, shown below with margin-top 23 → y 63..103
    assert_eq!(
        hit_test(&doc.tree, 100.0, 80.0),
        Some(shown.0),
        "after insert"
    );
    doc.remove_child(overlay, shown);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        hit_test(&doc.tree, 100.0, 80.0),
        Some(behind.0),
        "after remove"
    );
}

/// Re-parent: a visible-declared child moved into a hidden subtree is hit; a
/// plain (inheriting) one moved in is not; moved out again, it is.
#[test]
fn probe_reparent() {
    let (mut doc, behind, overlay) = stacked("visibility: hidden");
    let body = doc.body();
    let side = div(
        &mut doc,
        body,
        "position: absolute; left: 400px; top: 30px; width: 300px; height: 200px",
    );
    let kid = div(&mut doc, side, &format!("visibility: visible; {KID}"));
    let plain = div(
        &mut doc,
        side,
        "margin-left: 37px; width: 100px; height: 40px",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(behind.0));
    doc.append_child(overlay, kid);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        hit_test(&doc.tree, 100.0, 70.0),
        Some(kid.0),
        "visible kid moved in"
    );
    doc.remove_child(overlay, kid);
    doc.append_child(overlay, plain);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.get(plain.0).unwrap().computed_style.visibility,
        VisibilityValue::Hidden
    );
    assert_eq!(
        hit_test(&doc.tree, 100.0, 50.0),
        Some(behind.0),
        "plain kid moved in"
    );
    doc.append_child(side, plain);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        hit_test(&doc.tree, 480.0, 50.0),
        Some(plain.0),
        "plain moved out"
    );
}

const CSS: &str = "
    .fade { transition: visibility 300ms linear; }
    .shut { visibility: hidden; }
    .open { visibility: visible; }
";

/// The descendant's own visibility transitions (visible → hidden) under a
/// hidden ancestor that does not change.
#[test]
fn probe_descendant_transition_under_static_hidden_ancestor() {
    let (mut doc, behind, overlay) = stacked("visibility: hidden");
    doc.load_css(CSS);
    let kid = doc.create_element("div");
    doc.set_attribute(kid, "class", "fade open");
    doc.set_attribute(kid, "style", KID);
    doc.append_child(overlay, kid);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(kid.0));
    doc.set_attribute(kid, "class", "fade");
    doc.resolve_layout(801.0, 600.0);
    let start = doc.tree.active_transitions[&kid.0][&TransitionProperty::Visibility].start_time_ms;
    rinch_dom::transition::tick_transitions(&mut doc.tree, start + 270.0);
    assert_eq!(
        hit_test(&doc.tree, 100.0, 70.0),
        Some(kid.0),
        "still visible at 0.9"
    );
    rinch_dom::transition::tick_transitions(&mut doc.tree, start + 301.0);
    assert_eq!(
        doc.tree.get(kid.0).unwrap().computed_style.visibility,
        VisibilityValue::Hidden
    );
    assert_eq!(
        hit_test(&doc.tree, 100.0, 70.0),
        Some(behind.0),
        "hidden after end"
    );
}

/// Hand-down: an ancestor's closing transition hands `hidden` down at its end.
#[test]
fn probe_ancestor_transition_hand_down() {
    let (mut doc, behind, overlay) = stacked("");
    doc.load_css(CSS);
    doc.set_attribute(overlay, "class", "fade");
    let mid = div(&mut doc, overlay, "width: 300px; height: 200px");
    let kid = div(&mut doc, mid, KID);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(kid.0));
    doc.set_attribute(overlay, "class", "fade shut");
    doc.resolve_layout(801.0, 600.0);
    let start =
        doc.tree.active_transitions[&overlay.0][&TransitionProperty::Visibility].start_time_ms;
    rinch_dom::transition::tick_transitions(&mut doc.tree, start + 150.0);
    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(kid.0), "mid-close");
    rinch_dom::transition::tick_transitions(&mut doc.tree, start + 301.0);
    assert_eq!(
        doc.tree.get(kid.0).unwrap().computed_style.visibility,
        VisibilityValue::Hidden
    );
    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(behind.0), "closed");
}

/// `visibility: collapse` on a non-table ancestor acts as hidden; a visible
/// child is hit (Chrome: same as hidden).
#[test]
fn probe_collapse_ancestor() {
    let (mut doc, behind, overlay) = stacked("visibility: collapse");
    let kid = div(&mut doc, overlay, &format!("visibility: visible; {KID}"));
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(kid.0));
    assert_eq!(hit_test(&doc.tree, 250.0, 150.0), Some(behind.0));
}

/// A hidden `display: contents` wrapper between the hidden overlay and a
/// visible grandchild.
#[test]
fn probe_display_contents_hidden_wrapper() {
    let (mut doc, behind, overlay) = stacked("visibility: hidden");
    let wrap = div(&mut doc, overlay, "display: contents");
    let kid = div(&mut doc, wrap, &format!("visibility: visible; {KID}"));
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(kid.0));
    assert_eq!(hit_test(&doc.tree, 250.0, 150.0), Some(behind.0));
}

/// A hidden stacking context (`z-index`) holding a visible positioned
/// grandchild, and a visible `position: fixed` descendant far from it.
#[test]
fn probe_stacking_and_fixed_under_hidden() {
    let (mut doc, behind, overlay) = stacked("visibility: hidden; z-index: 2; opacity: 0.99");
    let mid = div(
        &mut doc,
        overlay,
        "position: relative; width: 300px; height: 200px",
    );
    let pos = div(
        &mut doc,
        mid,
        "visibility: visible; position: absolute; left: 50px; top: 50px; width: 40px; height: 40px",
    );
    let fixed = div(
        &mut doc,
        mid,
        "visibility: visible; position: fixed; left: 500px; top: 400px; width: 40px; height: 40px",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        hit_test(&doc.tree, 90.0, 100.0),
        Some(pos.0),
        "abs grandchild"
    );
    assert_eq!(
        hit_test(&doc.tree, 520.0, 420.0),
        Some(fixed.0),
        "fixed descendant"
    );
    assert_eq!(hit_test(&doc.tree, 250.0, 200.0), Some(behind.0));
}

/// Hidden `overflow: hidden` overlay with a visible child: the child is
/// clipped to the overlay's box (paint clips a hidden box's descendants too).
#[test]
fn probe_hidden_clip_still_clips() {
    let (mut doc, behind, overlay) = stacked("visibility: hidden; overflow: hidden");
    let kid = div(
        &mut doc,
        overlay,
        "visibility: visible; margin-left: 250px; margin-top: 10px; width: 200px; height: 40px",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(hit_test(&doc.tree, 300.0, 60.0), Some(kid.0), "inside clip");
    assert_ne!(
        hit_test(&doc.tree, 400.0, 60.0),
        Some(kid.0),
        "outside clip"
    );
    let _ = behind;
}

/// Scrollbar of a visible scroller deep under two hidden levels, after a
/// cached `false`: toggling the scroller's own visibility.
#[test]
fn probe_scrollbar_toggle() {
    let (mut doc, _behind, overlay) = stacked("visibility: hidden");
    let mid = div(&mut doc, overlay, "width: 300px; height: 200px");
    let sc = div(&mut doc, mid, "width: 200px; height: 100px; overflow: auto");
    div(&mut doc, sc, "height: 1000px");
    doc.resolve_layout(800.0, 600.0);
    assert!(find_scrollbar_hit(&doc.tree, 212.0, 77.0).is_none());
    doc.set_attribute(
        sc,
        "style",
        "visibility: visible; width: 200px; height: 100px; overflow: auto",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        find_scrollbar_hit(&doc.tree, 212.0, 77.0).map(|h| h.node_id),
        Some(sc.0)
    );
}

/// A padded hidden IFC root: the visible span's text is hit where paint draws
/// it (inside the padding offset). Chrome 156: `span` at (145, 90), `behind`
/// at (25, 40). Kills M10 (content origin dropped).
#[test]
fn probe_padded_hidden_ifc_root() {
    let (mut doc, behind, overlay) = stacked(
        "visibility: hidden; padding-left: 120px; padding-top: 50px; width: 180px; height: 150px; \
         font-size: 16px; line-height: 20px; font-family: sans-serif",
    );
    let span = doc.create_element("span");
    doc.set_attribute(span, "style", "visibility: visible");
    let text = doc.create_text("VIS");
    doc.append_child(span, text);
    doc.append_child(overlay, span);
    doc.resolve_layout(800.0, 600.0);
    let root = doc.tree.get(text.0).unwrap().ifc_root.unwrap();
    assert_eq!(hit_test(&doc.tree, 145.0, 90.0), Some(root));
    assert_eq!(hit_test(&doc.tree, 25.0, 40.0), Some(behind.0));
}

/// A scrolled hidden IFC root: the visible span's text moves with the scroll.
/// Kills M9 (scroll offset dropped).
#[test]
fn probe_scrolled_hidden_ifc_root() {
    let (mut doc, behind, overlay) = stacked(
        "visibility: hidden; overflow: auto; font-size: 16px; line-height: 20px; font-family: sans-serif",
    );
    let own = doc.create_text(&"HIDDEN ".repeat(200));
    doc.append_child(overlay, own);
    let span = doc.create_element("span");
    doc.set_attribute(span, "style", "visibility: visible");
    let text = doc.create_text(&"VISIBLE ".repeat(200));
    doc.append_child(span, text);
    doc.append_child(overlay, span);
    doc.resolve_layout(800.0, 600.0);
    let root = doc.tree.get(text.0).unwrap().ifc_root.unwrap();
    // Before scrolling, the top lines are the hidden root's own text.
    assert_eq!(
        hit_test(&doc.tree, 30.0, 40.0),
        Some(behind.0),
        "own text at top"
    );
    let h = doc
        .tree
        .get(root)
        .unwrap()
        .text_layout
        .as_ref()
        .unwrap()
        .layout
        .height();
    doc.set_scroll_top(overlay, (h - 200.0) as f64);
    doc.resolve_layout(800.0, 600.0);
    assert!(
        doc.tree.get(root).unwrap().scroll_offset.1 > 100.0,
        "positive control: scrolled"
    );
    assert_eq!(
        hit_test(&doc.tree, 30.0, 40.0),
        Some(root),
        "visible text scrolled into view"
    );
}
