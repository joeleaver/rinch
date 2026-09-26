//! Hit testing through a `display: contents` element that declares a stacking
//! or clipping property — issue #1038.
//!
//! The paint half, and the Chrome 153 table every answer here is taken from,
//! is `crates/rinch-dom/tests/display_contents_stacking_tests.rs`. Hit testing
//! walks the same `stacking_paint_order` paint draws and gates on the same
//! `clips_overflow`, so these are the same markup asked with a point:
//! `elementFromPoint` in Chrome 153 answers `t` at every probe below.

use super::hit_testing::hit_test;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

fn doc_of(html: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    doc.set_inner_html(body, html);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn id_of(doc: &RinchDocument, class: &str) -> usize {
    doc.query_selector(&format!(".{class}"))
        .expect("fixture element")
        .0
}

const CLIP: &str = "margin: 40px 0 0 60px; width: 100px; height: 100px; overflow: hidden";
const ABS: &str = "position: absolute; left: 10px; top: 20px; width: 300px; height: 250px";

fn clip_doc(wrapper: &str, target: &str) -> RinchDocument {
    doc_of(&format!(
        r#"<div style="{CLIP}"><div style="{wrapper}"><div class="t" style="{target}"></div></div></div>"#
    ))
}

/// (200, 200) is outside the clipper and inside the absolute; (100, 100) is
/// inside both, the positive control.
#[test]
fn an_absolute_under_a_contents_wrapper_is_hit_outside_the_clipper() {
    for style in [
        "position: relative",
        "position: relative; z-index: 1",
        "transform: translateX(5px)",
        "opacity: 0.5",
        "position: sticky",
        "position: fixed",
    ] {
        let doc = clip_doc(&format!("display: contents; {style}"), ABS);
        let t = id_of(&doc, "t");
        assert_eq!(
            hit_test(&doc.tree, 100.0, 100.0),
            Some(t),
            "`{style}`: inside"
        );
        assert_eq!(
            hit_test(&doc.tree, 200.0, 200.0),
            Some(t),
            "`display: contents; {style}`: Chrome 153 answers the absolute"
        );
    }
}

#[test]
fn a_fixed_box_under_a_transformed_contents_wrapper_is_hit_outside_the_clipper() {
    let doc = clip_doc(
        "display: contents; transform: translateX(5px)",
        "position: fixed; left: 10px; top: 20px; width: 300px; height: 250px",
    );
    assert_eq!(hit_test(&doc.tree, 200.0, 200.0), Some(id_of(&doc, "t")));
}

/// `position: fixed` on a contents element anchors nothing to the viewport.
#[test]
fn an_in_flow_child_of_a_fixed_contents_wrapper_is_hit_in_the_clipper() {
    let doc = clip_doc(
        "display: contents; position: fixed",
        "width: 50px; height: 50px",
    );
    let t = id_of(&doc, "t");
    assert_eq!(hit_test(&doc.tree, 105.0, 85.0), Some(t), "Chrome 153: `t`");
    assert_ne!(hit_test(&doc.tree, 10.0, 10.0), Some(t));
}

/// The control: a boxed `relative` wrapper is the containing block and sits
/// inside the clipper, so the absolute is not hit outside it.
#[test]
fn control_an_absolute_under_a_boxed_relative_wrapper_is_clipped() {
    let doc = clip_doc("position: relative", ABS);
    let t = id_of(&doc, "t");
    assert_eq!(hit_test(&doc.tree, 100.0, 100.0), Some(t));
    assert_ne!(hit_test(&doc.tree, 200.0, 200.0), Some(t));
}

/// A positioned contents wrapper is not hoisted, so the later sibling `s` is
/// on top where the two overlap.
#[test]
fn a_positioned_contents_wrapper_does_not_lift_its_child_over_a_later_sibling() {
    let doc = doc_of(
        r#"<div style="display: contents; position: relative; z-index: 5"><div class="t" style="position: relative; z-index: 1; height: 50px"></div></div><div class="s" style="position: relative; z-index: 2; height: 50px; margin-top: -25px"></div>"#,
    );
    assert_eq!(hit_test(&doc.tree, 10.0, 35.0), Some(id_of(&doc, "s")));
    assert_eq!(
        hit_test(&doc.tree, 10.0, 10.0),
        Some(id_of(&doc, "t")),
        "positive control: `t` is hittable where nothing covers it"
    );
}

/// `overflow: hidden` on a contents element gates nothing: its `0x0` rect used
/// to make `check_children` refuse every point.
#[test]
fn overflow_on_a_contents_element_does_not_gate_its_children() {
    for extra in ["", "; opacity: 0.5"] {
        let doc = doc_of(&format!(
            r#"<div style="width: 100px; height: 100px"><div style="display: contents; overflow: hidden{extra}"><div class="t" style="position: relative; width: 300px; height: 60px"></div></div></div>"#
        ));
        let t = id_of(&doc, "t");
        assert_eq!(
            hit_test(&doc.tree, 10.0, 30.0),
            Some(t),
            "`{extra}`: inside"
        );
        assert_eq!(
            hit_test(&doc.tree, 250.0, 30.0),
            Some(t),
            "`{extra}`: past the 100px parent — Chrome 153 answers `t`"
        );
    }
}
