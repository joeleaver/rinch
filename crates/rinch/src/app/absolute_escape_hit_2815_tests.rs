//! #2815 in hit testing: an absolutely positioned box is tapped wherever
//! paint draws it — outside the clippers between its containing block and the
//! stacking context that paints it, its own clip included — and stays
//! untappable past its containing block's own clip and the clippers above
//! it. Chrome 158 paints each shape below the same way (headless, the same
//! markup); its `elementFromPoint` follows its paint.
//!
//! Paint's half is `rinch-dom/tests/absolute_escape_2815_tests.rs`; the
//! shapes and the probe points are the same.

use super::hit_test;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const CB: &str = "position: relative; width: 300px; height: 300px";
const CLIPPER: &str = "overflow: hidden; width: 100px; height: 100px";
const CONTEXT: &str = "opacity: 0.5; width: 50px; height: 50px";
const ABSOLUTE: &str = "position: absolute; left: 25px; top: 25px; width: 200px; height: 200px";
/// Inside the clippers, on the absolute box only.
const IN: (f32, f32) = (75.0, 75.0);
/// Outside the clippers, on the absolute box.
const OUT: (f32, f32) = (200.0, 200.0);

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let n = doc.create_element("div");
    doc.set_attribute(n, "style", style);
    doc.append_child(parent, n);
    n
}

/// `body > s[0] > … > last`, laid out; the innermost box is returned.
fn chain(styles: &[&str]) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let mut parent = body;
    for s in styles {
        parent = el(&mut doc, parent, s);
    }
    doc.resolve_layout(800.0, 600.0);
    (doc, parent)
}

fn hit(doc: &RinchDocument, (x, y): (f32, f32)) -> Option<u64> {
    hit_test(&doc.tree, x, y)
}

/// **The issue's shape**, and the variants paint escapes: no positioned
/// ancestor at all (#204), the context's own clip (`stacking.rs`'s old
/// "Known gap"), and a fixed box contained by a transformed ancestor.
///
/// Kills: the walk keeping an absolute entry clipped by a clip it escapes
/// (the probe outside answers the containing block or the body) — `main`
/// before #2815.
#[test]
fn an_absolute_box_is_tapped_outside_the_clips_it_escapes() {
    for styles in [
        &[CB, CLIPPER, CONTEXT, ABSOLUTE][..],
        &[CLIPPER, CONTEXT, ABSOLUTE][..],
        // A `clip-path` element above the clipper does not keep it on.
        &[
            CB,
            "clip-path: inset(0); width: 300px; height: 300px",
            CLIPPER,
            CONTEXT,
            ABSOLUTE,
        ][..],
        &[
            CB,
            "opacity: 0.5; overflow: hidden; width: 100px; height: 100px",
            ABSOLUTE,
        ][..],
        &[
            "transform: translate(0px, 0px); width: 300px; height: 300px",
            CLIPPER,
            "position: relative; z-index: 1; width: 50px; height: 50px",
            "position: fixed; left: 25px; top: 25px; width: 200px; height: 200px",
        ][..],
    ] {
        let (doc, abs) = chain(styles);
        assert_eq!(hit(&doc, IN), Some(abs.0), "inside, {styles:?}");
        assert_eq!(hit(&doc, OUT), Some(abs.0), "outside, {styles:?}");
    }
}

/// The containing block's own clip, a clipper above the containing block,
/// and a clipper above a `clip-path` element on the way, still clip the
/// box's taps, as they clip its paint.
///
/// Kills: an escape that clears every clip, as a fixed box's does.
#[test]
fn the_clips_of_the_containing_block_and_above_still_apply() {
    for styles in [
        &[
            CLIPPER,
            "position: relative; width: 100px; height: 100px",
            CONTEXT,
            ABSOLUTE,
        ][..],
        &[
            "position: relative; overflow: hidden; width: 100px; height: 100px",
            "width: 300px; height: 300px",
            CONTEXT,
            ABSOLUTE,
        ][..],
        // A `clip-path` element between the clipper and the box keeps the
        // clipper's clip on (Chrome 158), whether it is an ancestor of the
        // context or the context itself.
        &[
            CB,
            CLIPPER,
            "clip-path: inset(0); width: 150px; height: 150px",
            CONTEXT,
            ABSOLUTE,
        ][..],
        &[
            CB,
            CLIPPER,
            "opacity: 0.5; width: 50px; height: 50px; clip-path: inset(-100px)",
            ABSOLUTE,
        ][..],
    ] {
        let (doc, abs) = chain(styles);
        assert_eq!(hit(&doc, IN), Some(abs.0), "positive control, {styles:?}");
        assert_ne!(hit(&doc, OUT), Some(abs.0), "outside, {styles:?}");
    }
}
