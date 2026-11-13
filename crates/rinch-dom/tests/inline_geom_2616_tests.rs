//! #2616: a wrapped right-to-left inline element's `bounding_rect` took its
//! padding and border on the left of its first fragment and the right of its
//! last — a left-to-right element's sides. A right-to-left element closes
//! the right of its first fragment and the left of its last, as paint draws
//! it (`paint::text::inline_background_rects`) and as Chrome 157 reports it;
//! and the containing block a positioned one forms runs between those same
//! sides (CSS 2.1 §10.1). The scene and Chrome's answers are in
//! `support/inline_geom_2616.rs`, shared with `rinch`'s
//! `inline_box_hit_2618_tests`.

#[path = "support/inline_geom_2616.rs"]
mod scene;

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::testing::query_selector;

fn node(doc: &rinch_dom::RinchDocument, id: &str) -> NodeId {
    NodeId(query_selector(&doc.tree, &format!("#{id}"))[0])
}

/// Each element's `getBoundingClientRect` is Chrome's: left-to-right and
/// right-to-left, on two and three lines, nested, padded unevenly.
#[test]
fn a_wrapped_elements_bounding_rect_is_chromes_in_either_direction() {
    let mut doc = scene::document();
    let mut bad = Vec::new();
    for &(id, (x, y, w, h)) in scene::BOUNDS {
        let r = doc.bounding_rect(node(&doc, id)).expect("a box");
        let got = [r.x, r.y, r.width, r.height].map(f64::from);
        if got
            .iter()
            .zip([x, y, w, h])
            .any(|(g, c)| (g - c).abs() > 0.01)
        {
            bad.push(format!("#{id}: {got:?}, Chrome {:?}", (x, y, w, h)));
        }
    }
    assert!(bad.is_empty(), "bounding rects:\n{}", bad.join("\n"));
}

/// The boxes hung from a positioned element's corners (`left: 0; top: 0`
/// and `right: 0; bottom: 0`) are where Chrome puts them: from the first
/// fragment's top and its start side to the last one's bottom and its end
/// side, inside the padding edges, a right-to-left element's start side
/// being the right. Within half a pixel: rinch places an absolute box on
/// a whole pixel (Chrome's `w2`, left-to-right and placed so before #2616, is at
/// `183.9063`, rinch's at `184`); the wrong side is many pixels off.
#[test]
fn a_wrapped_right_to_left_elements_containing_block_is_chromes() {
    let mut doc = scene::document();
    let mut bad = Vec::new();
    for &(id, (x, y)) in scene::ABS {
        let r = doc.bounding_rect(node(&doc, id)).expect("a box");
        let got = (f64::from(r.x), f64::from(r.y));
        if (got.0 - x).abs() > 0.5 || (got.1 - y).abs() > 0.5 {
            bad.push(format!("#{id}: {got:?}, Chrome {:?}", (x, y)));
        }
    }
    assert!(bad.is_empty(), "absolute boxes:\n{}", bad.join("\n"));
}
