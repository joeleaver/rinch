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

/// A point on the span's moved **text** finds the span's text node (#2404),
/// which link hover and the editor's link click ask: moved 40px right, its
/// drawn text overlaps where the unmoved lines have ` tail`, which the
/// point used to find.
#[test]
fn the_moved_text_is_found_where_it_is_drawn() {
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
        r#"lead <span id="s" style="position: relative; left: 40px; top: 4px">text</span> tail"#,
    );
    doc.resolve_layout(800.0, 600.0);
    let s = rinch_dom::testing::query_selector(&doc.tree, "#s")[0];
    let span_text = doc.tree.get(s).unwrap().children[0];
    // The drawn `e` (flat byte 6), from where its glyph is drawn.
    let g = rinch_dom::text_query::glyph_bounds_for_offset(&doc, c.0, 6).unwrap();
    let (cx, cy) = rinch_dom::paint::compute_absolute_position(&doc.tree, c.0, 1.0);
    let (x, y) = (
        cx as f32 + g.x + g.width / 2.0,
        cy as f32 + g.y + g.height / 2.0,
    );
    assert_eq!(
        super::text_node_at(&doc.tree, c.0, x, y),
        Some(span_text),
        "the span's text is under the point"
    );
}

/// A block inside a relative split inline (#2408) is hit across the whole
/// area it is drawn in, even where it overlaps the unmoved anonymous block
/// box of the run after it (review of #2497): Chrome 157's
/// `elementFromPoint` answers the block there, the inline being positioned.
/// Moved `left: 40px; top: 30px`, the block covers `y 50..70` of the
/// container and the `b` run's box `y 40..60`; with `top: 5px` on a link,
/// the block's `y 5..25` overlaps the `more` run's `20..40`. Both with the
/// container in flow and as a stacking context of its own.
#[test]
fn a_moved_block_in_a_split_inline_is_hit_over_the_next_runs_box() {
    for container in ["", "position: relative; z-index: 1;"] {
        for (inner, probe) in [
            (
                r#"lead <span style="position: relative; left: 40px; top: 30px">a<div id="d" style="width: 30px; height: 20px"></div>b</span>"#,
                5.0,
            ),
            (
                r#"<a href="x" style="position: relative; top: 5px"><div id="d" style="width: 30px; height: 20px"></div>more</a>"#,
                17.0,
            ),
        ] {
            let mut doc = RinchDocument::new();
            let body = doc.body();
            let c = doc.create_element("div");
            doc.set_attribute(
                c,
                "style",
                &format!("{container} width: 300px; font: 16px/20px sans-serif"),
            );
            doc.append_child(body, c);
            doc.set_inner_html(c, inner);
            doc.resolve_layout(800.0, 600.0);
            let d = rinch_dom::testing::query_selector(&doc.tree, "#d")[0];
            let r = rinch_dom::paint::painted_border_box(&doc.tree, d, 1.0);
            for dy in [probe, 18.0] {
                let (x, y) = ((r.x0 + 15.0) as f32, (r.y0 + dy) as f32);
                assert_eq!(
                    hit_test(&doc.tree, x, y),
                    Some(d),
                    "{container:?} {inner}: {dy}px into the block"
                );
            }
        }
    }
}
