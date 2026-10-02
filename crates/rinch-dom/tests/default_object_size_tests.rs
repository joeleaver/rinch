//! `<canvas>`, `<video>` and `<iframe>` have a default object size of
//! **300x150** (#1173).
//!
//! HTML gives a canvas natural dimensions from its `width` / `height`
//! attributes (300 and 150 when absent or invalid, each on its own), so a
//! canvas also has a natural aspect ratio. A `<video>` with no video and no
//! poster, and an `<iframe>`, take the CSS default object size, 300x150, with
//! **no** aspect ratio. Before the fix rinch gave all three no intrinsic size:
//! a bare canvas laid out 0 tall.
//!
//! **The oracle is Chrome 153**, measured with
//! `* { margin: 0; box-sizing: border-box } body { font-size: 10px;
//! line-height: 20px }` and the element inside a `width: 400px` container
//! (`getBoundingClientRect`). The container is wider than 300, so a width of
//! 300 is the natural width and not a fill. Chrome's `<iframe>` also carries
//! its UA `border: 2px inset`, which is why it measures 304x154.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const BASE: &str = "margin: 0; box-sizing: border-box; font-size: 10px; line-height: 20px";

/// `body > div(width: {container}) > <tag {attrs} style="{style}">`, laid out.
fn build(
    container: &str,
    tag: &str,
    attrs: &[(&str, &str)],
    style: &str,
) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", &format!("{BASE}; {container}"));
    doc.append_child(body, d);
    let e = doc.create_element(tag);
    for (k, v) in attrs {
        doc.set_attribute(e, k, v);
    }
    doc.set_attribute(e, "style", &format!("{BASE}; {style}"));
    doc.append_child(d, e);
    doc.resolve_layout(800.0, 600.0);
    (doc, d, e)
}

fn size(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let n = doc.tree.get(id.0).unwrap();
    (n.layout.width, n.layout.height)
}

fn check(
    container: &str,
    tag: &str,
    attrs: &[(&str, &str)],
    style: &str,
    want: (f32, f32),
) -> (RinchDocument, NodeId, NodeId) {
    let (doc, d, e) = build(container, tag, attrs, style);
    assert_eq!(
        size(&doc, e),
        want,
        "<{tag} {attrs:?} style=\"{style}\"> in [{container}] (Chrome 153)"
    );
    (doc, d, e)
}

const W400: &str = "width: 400px";

#[test]
fn a_bare_block_canvas_video_and_iframe_are_300_by_150() {
    check(W400, "canvas", &[], "display: block", (300.0, 150.0));
    check(W400, "video", &[], "display: block", (300.0, 150.0));
    check(W400, "iframe", &[], "display: block", (304.0, 154.0));
}

#[test]
fn an_inline_canvas_is_300_by_150_too() {
    // Chrome: the div is 157 tall (the line's strut adds its descent, #624),
    // the canvas itself 300x150.
    let (doc, d, _) = check(W400, "canvas", &[], "", (300.0, 150.0));
    assert!(size(&doc, d).1 >= 150.0, "the div holds its canvas");
}

#[test]
fn a_canvas_keeps_its_aspect_ratio_and_video_and_iframe_have_none() {
    check(
        W400,
        "canvas",
        &[],
        "display: block; width: 100px",
        (100.0, 50.0),
    );
    check(
        W400,
        "canvas",
        &[],
        "display: block; height: 100px",
        (200.0, 100.0),
    );
    check(
        W400,
        "canvas",
        &[],
        "display: block; width: 50%",
        (200.0, 100.0),
    );
    check(
        W400,
        "video",
        &[],
        "display: block; width: 100px",
        (100.0, 150.0),
    );
    check(
        W400,
        "video",
        &[],
        "display: block; height: 100px",
        (300.0, 100.0),
    );
    check(
        W400,
        "iframe",
        &[],
        "display: block; width: 100px",
        (100.0, 154.0),
    );
    check(
        W400,
        "iframe",
        &[],
        "display: block; height: 100px",
        (304.0, 100.0),
    );
}

#[test]
fn canvas_width_and_height_attributes_are_its_natural_size() {
    let wh = [("width", "40"), ("height", "30")];
    check(W400, "canvas", &wh, "display: block", (40.0, 30.0));
    check(
        W400,
        "canvas",
        &wh,
        "display: block; width: 80px",
        (80.0, 60.0),
    );
    check(
        W400,
        "canvas",
        &[("width", "0"), ("height", "0")],
        "display: block",
        (0.0, 0.0),
    );
    // A zero dimension has no ratio: the other keeps its natural length.
    check(
        W400,
        "canvas",
        &[("width", "0"), ("height", "30")],
        "display: block; width: 100px",
        (100.0, 30.0),
    );
    // Invalid values fall back to the defaults, each on its own.
    check(
        W400,
        "canvas",
        &[("width", "abc"), ("height", "-5")],
        "display: block",
        (300.0, 150.0),
    );
}

#[test]
fn changing_a_canvas_attribute_resizes_it() {
    let (mut doc, _, e) = check(W400, "canvas", &[], "display: block", (300.0, 150.0));
    doc.set_attribute(e, "width", "60");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, e), (60.0, 150.0));
    doc.set_attribute(e, "height", "20");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, e), (60.0, 20.0));
    doc.remove_attribute(e, "width");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, e), (300.0, 20.0));
}

#[test]
fn min_and_max_sizes_transfer_through_a_canvas_ratio() {
    check(
        W400,
        "canvas",
        &[],
        "display: block; min-width: 500px",
        (500.0, 250.0),
    );
    check(
        W400,
        "canvas",
        &[],
        "display: block; max-width: 100px",
        (100.0, 50.0),
    );
    check(
        W400,
        "canvas",
        &[],
        "display: block; min-height: 300px",
        (600.0, 300.0),
    );
    check(
        W400,
        "canvas",
        &[],
        "display: block; max-height: 50px",
        (100.0, 50.0),
    );
    check(
        "width: 200px",
        "canvas",
        &[],
        "display: block; max-width: 100%",
        (200.0, 100.0),
    );
    // No ratio, no transfer.
    check(
        W400,
        "video",
        &[],
        "display: block; max-width: 100px",
        (100.0, 150.0),
    );
}

#[test]
fn padding_and_border_sit_outside_the_natural_size() {
    check(
        W400,
        "canvas",
        &[],
        "display: block; padding: 10px; border: 5px solid",
        (330.0, 180.0),
    );
    check(
        W400,
        "canvas",
        &[],
        "display: block; padding: 10px; border: 5px solid; box-sizing: content-box",
        (330.0, 180.0),
    );
    // The ratio is the content box's: 100 - 20 = 80 wide, 40 + 20 tall.
    check(
        W400,
        "canvas",
        &[],
        "display: block; width: 100px; padding: 10px",
        (100.0, 60.0),
    );
    check(
        W400,
        "canvas",
        &[],
        "display: block; height: 100px; padding: 10px",
        (180.0, 100.0),
    );
}

#[test]
fn a_block_replaced_element_does_not_fill_its_container() {
    // Narrower than the natural width: it overflows rather than shrinks.
    check(
        "width: 100px",
        "canvas",
        &[],
        "display: block",
        (300.0, 150.0),
    );
    // `margin: auto` centres it, as for any block of a definite width.
    let (doc, d, e) = check(
        W400,
        "canvas",
        &[],
        "display: block; margin: 0 auto",
        (300.0, 150.0),
    );
    let (dx, ex) = (
        doc.tree.get(d.0).unwrap().layout.x,
        doc.tree.get(e.0).unwrap().layout.x,
    );
    assert_eq!(
        (dx, ex),
        (0.0, 50.0),
        "centred: (400 - 300) / 2 inside the div"
    );
}

#[test]
fn flex_items_stretch_and_carry_the_ratio() {
    // A column flex container stretches the canvas across, and the ratio
    // gives its height (Chrome: 400x200).
    check(
        "width: 400px; display: flex; flex-direction: column",
        "canvas",
        &[],
        "",
        (400.0, 200.0),
    );
    // Not stretched: its natural size.
    check(
        "width: 400px; display: flex; flex-direction: column; align-items: flex-start",
        "canvas",
        &[],
        "",
        (300.0, 150.0),
    );
    // The stretched width is the border box; the ratio is the content
    // box's: 400 - 20 = 380 wide, 190 + 20 tall (Chrome: 400x210).
    check(
        "width: 400px; display: flex; flex-direction: column",
        "canvas",
        &[],
        "padding: 10px",
        (400.0, 210.0),
    );
    // A row flex container: the natural size, and no shrinking below it
    // (Chrome: 300 wide in a 200px row).
    check(
        "width: 200px; display: flex",
        "canvas",
        &[],
        "",
        (300.0, 150.0),
    );
    check(
        "width: 400px; display: flex",
        "video",
        &[],
        "",
        (300.0, 150.0),
    );
}

#[test]
fn fallback_text_in_a_canvas_does_not_size_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", &format!("{BASE}; {W400}"));
    doc.append_child(body, d);
    let c = doc.create_element("canvas");
    doc.set_attribute(c, "style", &format!("{BASE}; display: block"));
    let t = doc.create_text(
        "fallback text that is long long long long long long long long long long long long long",
    );
    doc.append_child(c, t);
    doc.append_child(d, c);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, c), (300.0, 150.0), "Chrome: 300x150");
    assert!(
        doc.tree.get(c.0).unwrap().text_layout.is_none(),
        "the fallback text is not shaped (Chrome renders none of it)"
    );
    // And losing the child keeps the default size.
    doc.remove_child(c, t);
    doc.resolve_layout(801.0, 600.0);
    assert_eq!(size(&doc, c), (300.0, 150.0));
}

#[test]
fn an_explicit_size_wins() {
    check(
        W400,
        "canvas",
        &[],
        "display: block; width: 37px; height: 11px",
        (37.0, 11.0),
    );
    // A percentage height against an auto-height container is `auto`.
    check(
        W400,
        "video",
        &[],
        "display: block; width: 100%; height: 100%",
        (400.0, 150.0),
    );
    check(
        W400,
        "canvas",
        &[],
        "display: block; width: 100%; height: 100%",
        (400.0, 200.0),
    );
    check(
        "width: 400px; height: 80px",
        "video",
        &[],
        "display: block; width: 100%; height: 100%",
        (400.0, 80.0),
    );
}
