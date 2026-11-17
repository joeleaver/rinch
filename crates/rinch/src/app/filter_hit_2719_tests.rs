//! #2719: a `filter` does not change hit testing. A `drop-shadow()` is not
//! hittable (Chrome 157's `elementFromPoint` over the shadow answers what is
//! behind it), a `blur()`'s spread is not either, and the filtered box's own
//! border box is, as without the filter.

use super::hit_test;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

fn div(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let el = doc.create_element("div");
    doc.set_attribute(el, "style", style);
    doc.append_child(parent, el);
    el
}

/// `behind` fills 10..310 x 10..210; `filtered` is 60..160 x 40..110 over it.
fn page(filter: &str) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let behind = div(
        &mut doc,
        body,
        "position: absolute; left: 10px; top: 10px; width: 300px; height: 200px",
    );
    let filtered = div(
        &mut doc,
        body,
        &format!(
            "position: absolute; left: 60px; top: 40px; width: 100px; height: 70px; \
             background: #48f; filter: {filter}"
        ),
    );
    doc.resolve_layout(800.0, 600.0);
    (doc, behind, filtered)
}

#[test]
fn a_drop_shadow_is_not_hit_and_the_box_is() {
    let (doc, behind, filtered) = page("drop-shadow(30px 25px 6px #000)");
    // Inside the box.
    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(filtered.0));
    // On the shadow alone: past the box's right and bottom, under its offset.
    assert_eq!(hit_test(&doc.tree, 175.0, 120.0), Some(behind.0));
    // And where the shadow does not reach.
    assert_eq!(hit_test(&doc.tree, 300.0, 200.0), Some(behind.0));
}

#[test]
fn a_blurs_spread_is_not_hit() {
    let (doc, behind, filtered) = page("blur(9px)");
    assert_eq!(hit_test(&doc.tree, 61.0, 41.0), Some(filtered.0));
    // 8px past the box's left edge: blurred ink, not the box.
    assert_eq!(hit_test(&doc.tree, 52.0, 70.0), Some(behind.0));
    assert_eq!(hit_test(&doc.tree, 159.0, 109.0), Some(filtered.0));
    assert_eq!(hit_test(&doc.tree, 165.0, 109.0), Some(behind.0));
}

/// A `backdrop-filter` does not change hit testing either: the element is hit
/// in its border box and what it blurs behind it is hit as before.
#[test]
fn a_backdrop_filter_box_is_hit_as_its_box() {
    let (doc, behind, filtered) = page("none; backdrop-filter: blur(9px)");
    assert_eq!(hit_test(&doc.tree, 100.0, 70.0), Some(filtered.0));
    assert_eq!(hit_test(&doc.tree, 55.0, 70.0), Some(behind.0));
    assert_eq!(hit_test(&doc.tree, 165.0, 105.0), Some(behind.0));
}
