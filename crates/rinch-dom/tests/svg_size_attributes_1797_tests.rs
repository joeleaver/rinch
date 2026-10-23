//! The size of an outer `<svg>` (#1797).
//!
//! An `<svg>` is a replaced element. Its `width` and `height` attributes are
//! presentation attributes: each maps to the CSS property of its name, below
//! every author declaration, read as a CSS `<length-percentage>` in which a
//! bare number is pixels. When CSS leaves a dimension `auto`, the element's
//! **natural** size answers it: the attributes again, when they are absolute
//! lengths, and the `viewBox`'s ratio (else the attributes' own ratio) carries
//! one dimension to the other. With no natural size at all an `<svg>` takes
//! the default object size, 300x150, or, with a ratio, the containing block's
//! width.
//!
//! Before the fix rinch read none of that: an `<svg width="14" height="14">`
//! with no CSS size laid out `0x0`, so every icon sized by its attributes was
//! invisible (Playweft Studio's desktop port).
//!
//! **The oracle is Chrome 155**: `<!doctype html>`, `body { margin: 0; font:
//! 16px/20px sans-serif }`, the `<svg>` alone in a `width: 400px; height:
//! 300px` div, `getBoundingClientRect`. Sizes are chosen off the fixed points:
//! never square when a ratio is in play, never the default object size's own
//! numbers, a viewBox whose ratio differs from the attributes'.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const BOX: &str = "width: 400px; height: 300px; font: 16px/20px sans-serif";

/// `body > div(BOX) > <svg {attrs} style=…>`, laid out.
fn build(attrs: &[(&str, &str)], style: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", BOX);
    doc.append_child(body, d);
    let e = doc.create_element("svg");
    for (k, v) in attrs {
        doc.set_attribute(e, k, v);
    }
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    // An icon's shape, so the element is not childless: the children of an
    // `<svg>` are its drawing, not CSS content.
    let p = doc.create_element("path");
    doc.set_attribute(p, "d", "M0 0L16 16");
    doc.set_attribute(p, "stroke", "red");
    doc.append_child(e, p);
    doc.append_child(d, e);
    doc.resolve_layout(800.0, 600.0);
    (doc, e)
}

fn size(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let n = doc.tree.get(id.0).unwrap();
    (n.layout.width, n.layout.height)
}

#[track_caller]
fn check(attrs: &[(&str, &str)], style: &str, want: (f32, f32)) {
    let (doc, e) = build(attrs, style);
    assert_eq!(
        size(&doc, e),
        want,
        "<svg {attrs:?} style=\"{style}\"> (Chrome 155)"
    );
}

const VB: (&str, &str) = ("viewBox", "0 0 16 8");

#[test]
fn the_attributes_size_an_icon() {
    // The issue's own icon.
    check(
        &[("width", "14"), ("height", "14"), ("viewBox", "0 0 16 16")],
        "",
        (14.0, 14.0),
    );
    check(&[("width", "30"), ("height", "10")], "", (30.0, 10.0));
    check(
        &[("width", "30"), ("height", "10")],
        "display: block",
        (30.0, 10.0),
    );
    // In a flex container, as an item.
    let (doc, e) = {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let d = doc.create_element("div");
        doc.set_attribute(d, "style", &format!("{BOX}; display: flex"));
        doc.append_child(body, d);
        let e = doc.create_element("svg");
        doc.set_attribute(e, "width", "30");
        doc.set_attribute(e, "height", "10");
        doc.append_child(d, e);
        doc.resolve_layout(800.0, 600.0);
        (doc, e)
    };
    assert_eq!(size(&doc, e), (30.0, 10.0), "flex item (Chrome 155)");
}

#[test]
fn an_author_size_wins_over_the_attributes() {
    check(
        &[("width", "14"), ("height", "14"), ("viewBox", "0 0 16 16")],
        "width: 28px",
        (28.0, 14.0),
    );
    // `auto` hands both axes back to the natural size, which is the
    // attributes' again.
    check(
        &[("width", "30"), ("height", "10")],
        "width: auto; height: auto",
        (30.0, 10.0),
    );
    // The height attribute is a CSS height, so it is not carried through
    // the ratio when CSS sets only the width.
    check(
        &[("width", "30"), ("height", "10"), VB],
        "width: 64px",
        (64.0, 10.0),
    );
    check(
        &[("width", "200"), ("height", "100")],
        "max-width: 50px",
        (50.0, 100.0),
    );
}

#[test]
fn one_attribute_takes_the_other_from_the_view_box() {
    check(&[("width", "14"), VB], "", (14.0, 7.0));
    check(&[("height", "14"), VB], "", (28.0, 14.0));
}

#[test]
fn one_attribute_without_a_ratio_takes_the_other_from_the_default_size() {
    check(&[("width", "14")], "", (14.0, 150.0));
    check(&[("height", "14")], "", (300.0, 14.0));
}

#[test]
fn no_attribute_and_no_view_box_is_the_default_object_size() {
    check(&[], "", (300.0, 150.0));
    check(&[], "display: block", (300.0, 150.0));
    // A viewBox with no area is no ratio.
    check(&[("viewBox", "0 0 0 8")], "", (300.0, 150.0));
}

#[test]
fn a_view_box_alone_fills_the_containing_blocks_width() {
    check(&[VB], "", (400.0, 200.0));
    check(&[VB], "display: block", (400.0, 200.0));
    check(&[VB], "width: 32px", (32.0, 16.0));
    check(&[VB], "height: 32px", (64.0, 32.0));
    check(&[VB], "max-width: 100px", (100.0, 50.0));
}

#[test]
fn a_percentage_attribute_is_a_percentage_of_the_containing_block() {
    check(
        &[("width", "50%"), ("height", "20"), ("viewBox", "0 0 16 16")],
        "",
        (200.0, 20.0),
    );
    check(
        &[("width", "50%"), ("height", "50%")],
        "display: block",
        (200.0, 150.0),
    );
}

/// The attribute is a CSS `<length-percentage>` (a bare number is pixels):
/// any length unit, in any case, and a number in CSS's grammar.
#[test]
fn the_attribute_is_read_as_a_css_length() {
    check(&[("width", "14px"), ("height", "2em")], "", (14.0, 32.0));
    check(
        &[("width", "1em"), ("height", "1em")],
        "font-size: 20px",
        (20.0, 20.0),
    );
    check(&[("width", "14.5"), ("height", " 14")], "", (14.5, 14.0));
    check(&[("width", "1e1"), ("height", "+14")], "", (10.0, 14.0));
    check(&[("width", "30"), ("height", "14PX")], "", (30.0, 14.0));
    // 1in is 96px.
    check(&[("width", "30"), ("height", "1in")], "", (30.0, 96.0));
    // A negative length is zero, on either axis.
    check(&[("height", "-3")], "", (300.0, 0.0));
    check(&[("width", "-3")], "", (0.0, 150.0));
}

#[test]
fn an_author_aspect_ratio_replaces_the_natural_one() {
    check(
        &[("width", "14"), ("height", "14")],
        "aspect-ratio: 2; height: auto",
        (14.0, 7.0),
    );
}

#[test]
fn writing_an_attribute_after_layout_resizes_the_svg() {
    let (mut doc, e) = build(&[("width", "14"), ("height", "14")], "");
    assert_eq!(size(&doc, e), (14.0, 14.0));
    doc.set_attribute(e, "width", "30");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, e), (30.0, 14.0), "after width=30");
    doc.remove_attribute(e, "height");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, e), (30.0, 150.0), "after removing height");
    doc.set_attribute(e, "viewBox", "0 0 16 8");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, e), (30.0, 15.0), "after viewBox 16x8");
}

/// The children of an `<svg>` are its drawing: they take no part in its
/// size, whatever they are.
#[test]
fn the_svgs_children_do_not_size_it() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", BOX);
    doc.append_child(body, d);
    let e = doc.create_element("svg");
    doc.set_attribute(e, "width", "30");
    doc.set_attribute(e, "height", "10");
    let t = doc.create_element("text");
    let tx = doc.create_text("a long label that would wrap a line box");
    doc.append_child(t, tx);
    doc.append_child(e, t);
    doc.append_child(d, e);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, e), (30.0, 10.0));
}

/// The icon draws: a pixel oracle over the region the attributes size.
#[cfg(feature = "software-renderer")]
#[test]
fn an_icon_sized_by_its_attributes_paints() {
    use peniko::Brush;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let e = doc.create_element("svg");
    doc.set_attribute(e, "width", "14");
    doc.set_attribute(e, "height", "14");
    doc.set_attribute(e, "viewBox", "0 0 16 16");
    doc.set_attribute(e, "style", "display: block; margin: 10px 0 0 20px");
    let r = doc.create_element("rect");
    for (k, v) in [
        ("x", "0"),
        ("y", "0"),
        ("width", "16"),
        ("height", "16"),
        ("fill", "rgb(0, 128, 0)"),
    ] {
        doc.set_attribute(r, k, v);
    }
    doc.append_child(e, r);
    doc.append_child(body, e);
    doc.resolve_layout(100.0, 100.0);

    let mut painter = TinySkiaPainter::new(100, 100);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (100.0, 100.0),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let px = |x: u32, y: u32| {
        let i = ((y * painter.width() + x) * 4) as usize;
        let d = painter.pixels();
        [d[i], d[i + 1], d[i + 2], d[i + 3]]
    };
    let mut green = 0;
    for y in 10..24 {
        for x in 20..34 {
            if px(x, y) == [0, 128, 0, 255] {
                green += 1;
            }
        }
    }
    assert_eq!(green, 14 * 14, "the 14x14 box is filled");
    // Nothing past it.
    for (x, y) in [(35, 15), (25, 25), (19, 15), (25, 9)] {
        assert_ne!(px(x, y)[1], 128, "({x}, {y}) is outside the icon");
    }
}
