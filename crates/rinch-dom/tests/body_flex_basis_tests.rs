//! `<body>`'s box, pinned across #1260's flex-basis fold.
//!
//! `<html>` is a fixed flex column the size of the viewport and `<body>` its
//! only item (`node::body_taffy_overrides`). For the UA sheet's
//! `overflow-y: auto` body that can grow and shrink, the final height is the
//! viewport's less its margins, clamped by `min-height`/`max-height`, whatever
//! the flex basis — so #1260 stopped measuring the basis (it is `0`), which
//! saved a whole `ComputeSize` pass over the body's content per compute.
//!
//! These pin that the box did not move. Every geometry here is declared
//! (`height` on the filler blocks), so no number depends on the host's fonts.
//! Each fixture passes before and after #1260; what they are for is the
//! mutants of the fold, named on each.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const VP: (f32, f32) = (800.0, 600.0);

/// A body (with `extra` appended to its rule) holding `blocks` blocks of
/// 100px, laid out at the viewport, then once more at another height so a
/// fixture also covers a relayout (`resolve_layout` early-returns at an
/// unchanged viewport).
fn body_with(extra: &str, blocks: usize) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!(
        "* {{ border-width: 0; }} body {{ margin: 0; {extra} }} .b {{ height: 100px; }}"
    ));
    let body = doc.body();
    for _ in 0..blocks {
        let b = doc.create_element("div");
        doc.set_attribute(b, "class", "b");
        doc.append_child(body, b);
    }
    doc.resolve_layout(VP.0, VP.1 + 50.0);
    doc.resolve_layout(VP.0, VP.1);
    (doc, body)
}

fn body_box(doc: &RinchDocument, body: NodeId) -> (f32, f32, f32, f32) {
    let l = doc.tree.nodes[body.0].layout;
    (l.x, l.y, l.width, l.height)
}

/// The default body over content taller than the viewport is the viewport's
/// height, and scrolls the rest.
#[test]
fn a_tall_body_is_the_viewport_and_scrolls() {
    let (doc, body) = body_with("", 13);
    assert_eq!(body_box(&doc, body), (0.0, 0.0, 800.0, 600.0));
    assert_eq!(doc.scroll_height(body), 1300.0);
}

/// The default body over content shorter than the viewport still fills it.
#[test]
fn a_short_body_fills_the_viewport() {
    let (doc, body) = body_with("", 2);
    assert_eq!(body_box(&doc, body), (0.0, 0.0, 800.0, 600.0));
}

/// Margins come off the viewport's height, whichever way the body got there.
#[test]
fn a_margined_body_is_the_viewport_less_its_margins() {
    let (doc, body) = body_with("margin: 10px 0 30px 0;", 13);
    assert_eq!(body_box(&doc, body), (0.0, 10.0, 800.0, 560.0));
}

/// `min-height` and `max-height` clamp the filled height, off both sides of
/// the viewport.
#[test]
fn min_and_max_height_clamp_the_body() {
    let (doc, body) = body_with("max-height: 250px;", 13);
    assert_eq!(body_box(&doc, body).3, 250.0);
    let (doc, body) = body_with("min-height: 900px;", 2);
    assert_eq!(body_box(&doc, body).3, 900.0);
}

/// **`flex-shrink: 0` keeps a content-sized basis**: such a body cannot
/// shrink to the viewport, so its basis decides its height. Kills the mutant
/// that drops the `flex_shrink > 0.0` condition from the fold (the body
/// comes out 600 tall).
#[test]
fn a_body_that_cannot_shrink_is_as_tall_as_its_content() {
    let (doc, body) = body_with("flex-shrink: 0;", 13);
    assert_eq!(body_box(&doc, body), (0.0, 0.0, 800.0, 1300.0));
}

/// A body that is not a scroll container is outside the fold: its automatic
/// minimum height is its content's, so it is as tall as its content. A
/// regression pin for the path the fold leaves alone.
#[test]
fn a_visible_overflow_body_is_as_tall_as_its_content() {
    let (doc, body) = body_with("overflow: visible;", 13);
    assert_eq!(body_box(&doc, body), (0.0, 0.0, 800.0, 1300.0));
}
