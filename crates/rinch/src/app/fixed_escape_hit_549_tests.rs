//! #549 in hit testing: a `position: fixed` box is tapped wherever paint
//! draws it — outside every clip between it and the viewport, not only
//! outside the clip of the stacking context that owns it — and stays
//! untappable past a clip at or above a `transform`, `filter` or `mask-image`
//! ancestor, which paint keeps applying. Chrome 155's `elementFromPoint`
//! answers each probe below the same way (headless, the same markup).
//!
//! Paint's half is `rinch-dom/tests/fixed_escape_549_tests.rs`; the shapes and
//! the probe points are the same.

use super::hit_test;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FIXED: &str = "position: fixed; left: 130px; top: 110px; width: 270px; height: 290px";
/// Inside every clipper below, and on the fixed box.
const IN: (f32, f32) = (150.0, 130.0);
/// Outside every clipper below, and on the fixed box.
const OUT: (f32, f32) = (380.0, 380.0);

const CLIPPER: &str = "overflow: hidden; width: 200px; height: 200px";
const OWNER: &str = "position: relative; z-index: 1; width: 50px; height: 50px";

fn el(doc: &mut RinchDocument, parent: NodeId, style: &str) -> NodeId {
    let n = doc.create_element("div");
    doc.set_attribute(n, "style", style);
    doc.append_child(parent, n);
    n
}

/// `body > s[0] > … > fixed`, laid out; the fixed box is returned.
fn chain(styles: &[&str]) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let mut parent = body;
    for s in styles {
        parent = el(&mut doc, parent, s);
    }
    let fixed = el(&mut doc, parent, FIXED);
    doc.resolve_layout(800.0, 600.0);
    (doc, fixed)
}

fn hit(doc: &RinchDocument, (x, y): (f32, f32)) -> Option<usize> {
    hit_test(&doc.tree, x, y)
}

/// **The issue's shape**, and every variant of it Chrome escapes: the owner's
/// own clip (#545), a clipper above it, two clippers above an `opacity` owner,
/// and nested contexts that both clip.
///
/// Kills: the walk skipping an entry whose clip chain or root gate rejects
/// the point (the probe outside answers the body) — `main` before #549.
#[test]
fn a_fixed_box_is_tapped_outside_every_clip_above_its_context() {
    for styles in [
        &[CLIPPER, OWNER][..],
        &["position: relative; z-index: 1; overflow: hidden; width: 50px; height: 50px"][..],
        &[
            CLIPPER,
            "overflow: clip; width: 150px; height: 150px",
            "opacity: 0.5; width: 50px; height: 50px",
        ][..],
        &[
            CLIPPER,
            "position: relative; z-index: 1; width: 100px; height: 100px",
            "position: relative; z-index: 1; overflow: hidden; width: 50px; height: 50px",
        ][..],
    ] {
        let (doc, fixed) = chain(styles);
        assert_eq!(hit(&doc, IN), Some(fixed.0), "inside, {styles:?}");
        assert_eq!(hit(&doc, OUT), Some(fixed.0), "outside, {styles:?}");
    }
}

/// A rounded clipper: hit testing tests a chain link's rect only, so the
/// probe that matters is the one past the clipper's box.
#[test]
fn a_fixed_box_is_tapped_past_a_rounded_clipper() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let clipper = el(
        &mut doc,
        body,
        "overflow: hidden; border-radius: 60px; width: 200px; height: 200px",
    );
    let owner = el(&mut doc, clipper, OWNER);
    let fixed = el(
        &mut doc,
        owner,
        "position: fixed; left: 0; top: 0; width: 300px; height: 300px",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(hit(&doc, (4.0, 4.0)), Some(fixed.0));
    assert_eq!(hit(&doc, (250.0, 250.0)), Some(fixed.0));
}

/// **A `transform`, `filter` or `mask-image` ancestor keeps the clip above
/// it**: the probe outside the clipper is not on the fixed box.
///
/// Kills: the clipped walk descending through such an ancestor (the probe
/// outside answers the fixed box).
#[test]
fn a_fixed_box_is_not_tapped_past_a_clip_above_its_containing_block() {
    for between in [
        "transform: translate(0px, 0px); width: 100px; height: 100px",
        "filter: brightness(0.9); width: 100px; height: 100px",
        "mask-image: linear-gradient(#000, #000); width: 300px; height: 300px",
    ] {
        let (doc, fixed) = chain(&[CLIPPER, between, OWNER]);
        assert_eq!(hit(&doc, IN), Some(fixed.0), "positive control, {between}");
        assert_ne!(hit(&doc, OUT), Some(fixed.0), "past the clip, {between}");
    }
}

/// The clipped walk only goes looking for fixed boxes: what the owner and the
/// clipper hold besides one stays untappable past the clip, and the fixed box
/// is still found under it in paint order.
///
/// Kills: the clipped walk hitting ordinary boxes (the owner's wide child
/// answers at `(600, 30)`), and its own box (the owner answers past it).
#[test]
fn only_the_fixed_box_escapes_the_clip() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let clipper = el(&mut doc, body, CLIPPER);
    let owner = el(
        &mut doc,
        clipper,
        "position: relative; z-index: 1; width: 700px; height: 50px",
    );
    let fixed = el(&mut doc, owner, FIXED);
    let wide = el(
        &mut doc,
        owner,
        "position: absolute; z-index: 2; left: 0; top: 20px; width: 700px; height: 30px",
    );
    let later = el(
        &mut doc,
        clipper,
        "position: relative; z-index: 3; top: 100px; width: 700px; height: 30px",
    );
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(hit(&doc, (100.0, 30.0)), Some(wide.0), "positive control");
    assert_eq!(hit(&doc, (600.0, 30.0)), Some(doc.tree.body_id));
    assert_eq!(
        hit(&doc, (600.0, 10.0)),
        Some(doc.tree.body_id),
        "the owner"
    );
    assert_eq!(hit(&doc, (100.0, 160.0)), Some(later.0), "positive control");
    assert_eq!(hit(&doc, (600.0, 160.0)), Some(doc.tree.body_id));
    assert_eq!(hit(&doc, OUT), Some(fixed.0));
}

/// The clipper is scrolled so the owner is above its viewport.
#[test]
fn a_fixed_box_under_a_context_scrolled_out_of_its_clipper_is_tapped() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let clipper = el(&mut doc, body, CLIPPER);
    let owner = el(&mut doc, clipper, OWNER);
    let fixed = el(&mut doc, owner, FIXED);
    el(&mut doc, clipper, "width: 10px; height: 2000px");
    doc.resolve_layout(800.0, 600.0);
    doc.tree.nodes[clipper.0].scroll_offset = (0.0, 700.0);
    doc.tree.hit_cache.invalidate();
    assert_eq!(hit(&doc, IN), Some(fixed.0));
    assert_eq!(hit(&doc, OUT), Some(fixed.0));
}
