//! A block-level `<img>` takes its natural width, not its container's (#788).
//!
//! CSS 2.1 §10.3.4: the used `width` of a block-level replaced element in
//! normal flow with `width: auto` is determined as for an inline replaced
//! element (§10.3.2) — its natural width — and the margins then follow the
//! block rules, so `margin: 0 auto` centres it. rinch used to stretch it to
//! the container like a non-replaced block: `img { display: block }`, the
//! commonest image reset, made a 40x30 image 300 wide. Taffy's block layout
//! skips the stretch for a child whose style says `item_is_replaced`.
//!
//! **The oracle is Chrome 153**, on the same markup: `* { margin: 0;
//! box-sizing: border-box } body { font-size: 10px; line-height: 20px }`,
//! the image inside a `div` styled as the fixture's container
//! (`getBoundingClientRect`). The image is 40x30 — a ratio off 1:1, a width
//! off the container's, so neither a stretch nor a square answer passes.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// A 40x30 PNG.
const FORTY_BY_THIRTY: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACgAAAAeCAYAAABe3VzdAAAAPUlEQVR4nO3OIQEAIBAAMTJ9/wC0ggjIQ0zMb+2Z87NVBwQF64CgYB0QFKwDgoJ1QFCwDggK1gFBwTrwcgFD1ConI1jnigAAAABJRU5ErkJggg==";

const BASE: &str = "margin: 0; box-sizing: border-box; font-size: 10px; line-height: 20px";

/// `body > div(container) > img(style)` laid out; answers the image's
/// `(x within the div, width, height)`.
fn measure(container: &str, style: &str, src: Option<&str>) -> (f32, f32, f32) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", &format!("{BASE}; {container}"));
    doc.append_child(body, d);
    let img = doc.create_element("img");
    if let Some(src) = src {
        doc.set_attribute(img, "src", src);
    }
    doc.set_attribute(img, "style", &format!("{BASE}; {style}"));
    doc.append_child(d, img);
    doc.resolve_layout(800.0, 600.0);
    let l = doc.tree.get(img.0).unwrap().layout;
    (l.x, l.width, l.height)
}

fn check(container: &str, style: &str, want: (f32, f32, f32)) {
    assert_eq!(
        measure(container, style, Some(FORTY_BY_THIRTY)),
        want,
        "<img style=\"{style}\"> in [{container}]: (x, width, height), Chrome 153"
    );
}

const W300: &str = "width: 300px";

#[test]
fn a_block_image_takes_its_natural_width() {
    check(W300, "display: block", (0.0, 40.0, 30.0));
    // The inline image always did.
    check(W300, "", (0.0, 40.0, 30.0));
}

#[test]
fn auto_margins_centre_and_right_align_a_block_image() {
    check(W300, "display: block; margin: 0 auto", (130.0, 40.0, 30.0));
    check(
        W300,
        "display: block; margin-left: auto",
        (260.0, 40.0, 30.0),
    );
}

#[test]
fn padding_and_border_sit_outside_the_natural_width() {
    // border-box sizing: the auto width is the content box plus 14.
    check(
        W300,
        "display: block; padding: 5px; border: 2px solid",
        (0.0, 54.0, 44.0),
    );
}

#[test]
fn explicit_and_clamped_widths_still_apply() {
    check(W300, "display: block; height: 60px", (0.0, 80.0, 60.0));
    check(W300, "display: block; min-width: 100px", (0.0, 100.0, 75.0));
    check(
        "width: 20px",
        "display: block; max-width: 100%",
        (0.0, 20.0, 15.0),
    );
}

#[test]
fn a_column_flex_item_image_is_still_stretched() {
    // `item_is_replaced` is the block algorithm's flag; a flex container
    // stretches a replaced item as Chrome does (300 wide, its ratio's 225).
    check(
        "width: 300px; display: flex; flex-direction: column",
        "display: block",
        (0.0, 300.0, 225.0),
    );
}

#[test]
fn a_block_image_with_no_source_is_zero_wide() {
    assert_eq!(
        measure(W300, "display: block", None),
        (0.0, 0.0, 0.0),
        "Chrome 153: 0x0"
    );
}

/// #1150: a style width (or height) alone gives the other dimension through
/// the image's ratio — inline, block, `inline-block` and as a flex item —
/// and a `max-width` clamp carries over to the height. The image's measure
/// used to see neither, so a lone `width: 100px` kept the natural 30 tall.
#[test]
fn one_style_dimension_gives_the_other_through_the_ratio() {
    let flex = "width: 300px; display: flex; align-items: flex-start";
    check(W300, "width: 100px", (0.0, 100.0, 75.0));
    check(W300, "height: 60px", (0.0, 80.0, 60.0));
    check(W300, "max-width: 20px", (0.0, 20.0, 15.0));
    check(
        W300,
        "display: inline-block; width: 100px",
        (0.0, 100.0, 75.0),
    );
    check(W300, "display: block; width: 100px", (0.0, 100.0, 75.0));
    check(
        "width: 320px",
        "display: block; width: 50%",
        (0.0, 160.0, 120.0),
    );
    check(W300, "display: block; max-height: 15px", (0.0, 20.0, 15.0));
    check(flex, "width: 100px", (0.0, 100.0, 75.0));
    check(flex, "height: 60px", (0.0, 80.0, 60.0));
    check(flex, "", (0.0, 40.0, 30.0));
}

/// The ratio applies to the content box: Chrome 153 gives `width: 90px;
/// padding: 5px` (border-box) 90x70 — an 80x60 image inside 5px of padding.
/// (Sized so no answer is a half pixel: rinch rounds its layout to whole
/// pixels. `content-box` is not asserted: rinch reads no `box-sizing`, #1278.)
#[test]
fn the_ratio_holds_the_content_box_not_the_padding() {
    check(
        W300,
        "display: block; width: 90px; padding: 5px",
        (0.0, 90.0, 70.0),
    );
}

/// The `Image` component's shape: `.rinch-image { display: block; width:
/// 100%; height: auto }`. Under a block wrapper it fills and keeps its
/// ratio (Chrome 153: 300x225, where rinch used to keep the natural 30 tall);
/// under the component's own `inline-block` wrapper it shrink-wraps to its
/// natural 40x30 in Chrome.
#[test]
fn the_image_component_shape_fills_a_block_and_keeps_its_ratio() {
    let shape = "display: block; width: 100%; height: auto";
    for (wrapper, want) in [
        ("display: block", (300.0, 225.0)),
        ("display: inline-block; position: relative", (40.0, 30.0)),
    ] {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let d = doc.create_element("div");
        doc.set_attribute(d, "style", &format!("{BASE}; width: 300px"));
        doc.append_child(body, d);
        let w = doc.create_element("div");
        doc.set_attribute(w, "style", &format!("{BASE}; {wrapper}"));
        doc.append_child(d, w);
        let img = doc.create_element("img");
        doc.set_attribute(img, "src", FORTY_BY_THIRTY);
        doc.set_attribute(img, "style", &format!("{BASE}; {shape}"));
        doc.append_child(w, img);
        doc.resolve_layout(800.0, 600.0);
        let l = doc.tree.get(img.0).unwrap().layout;
        assert_eq!((l.width, l.height), want, "under [{wrapper}], Chrome 153");
    }
}
