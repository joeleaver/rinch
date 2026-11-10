//! #2346: an `inline-block` inside a `position: relative` inline span is hit
//! where the span's offset moved it — where it is painted and where Chrome
//! 156's `elementFromPoint` finds it — and the place it would have been
//! without the offset is not it any more.

use super::hit_test;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// `(doc, container, box)`: `lead <span rel {offset}><ib></span>` in a block
/// at `(100, 50)`.
fn case(offset: &str) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = doc.create_element("div");
    doc.set_attribute(
        c,
        "style",
        "position: absolute; left: 100px; top: 50px; width: 300px; height: 100px; \
         font: 16px/20px sans-serif",
    );
    doc.append_child(body, c);
    doc.set_inner_html(
        c,
        &format!(
            r#"lead <span style="position: relative; {offset}"><span id="ib" style="display: inline-block; width: 30px; height: 20px"></span></span>"#
        ),
    );
    doc.resolve_layout(800.0, 600.0);
    let ib = rinch_dom::testing::query_selector(&doc.tree, "#ib")[0];
    (doc, c, NodeId(ib))
}

fn center(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let r = rinch_dom::paint::painted_border_box(&doc.tree, id.0, 1.0);
    (((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32)
}

/// Moved 40px right and 12px down (more than the box's own size, so the two
/// places do not overlap): its new centre hits it, its old one hits the
/// container.
///
/// Before the fix the box was laid out, painted and hit at its unmoved place.
#[test]
fn the_moved_inline_block_is_hit_where_it_moved() {
    let (twin, _, twin_ib) = case("");
    let (old_x, old_y) = center(&twin, twin_ib);
    assert_eq!(
        hit_test(&twin.tree, old_x, old_y),
        Some(twin_ib.0),
        "positive control: the unmoved box is hit at its centre"
    );

    let (doc, c, ib) = case("left: 40px; top: 12px");
    let (x, y) = center(&doc, ib);
    assert_eq!(
        (x - old_x, y - old_y),
        (40.0, 12.0),
        "painted 40px right, 12px down"
    );
    assert_eq!(
        hit_test(&doc.tree, x, y),
        Some(ib.0),
        "hit where it is painted"
    );
    assert_eq!(
        hit_test(&doc.tree, old_x, old_y),
        Some(c.0),
        "not where it was: that point is the container's"
    );
}
