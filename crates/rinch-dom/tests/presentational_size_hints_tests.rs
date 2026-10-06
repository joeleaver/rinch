//! The `width` and `height` attributes of `<img>`, `<video>` and `<iframe>`
//! are presentational hints (#684).
//!
//! HTML maps both attributes to the `width` and `height` properties (the
//! *dimension attributes*), read by the rules for parsing dimension values,
//! at the lowest author-level precedence: any CSS declaration wins. On an
//! `<img>` and a `<video>` the pair also maps to `aspect-ratio: auto w / h`,
//! which gives an image that has not loaded (or failed to) a ratio, and which
//! a loaded image's own ratio replaces. Before the fix rinch read neither:
//! `<img width=100 height=50>` laid out 0x0 until its image arrived.
//!
//! **The oracle is Chrome 153**: `<!doctype html>`, `body { margin: 0 }`, the
//! element alone inside a `width: 400px; height: 300px` div
//! (`getBoundingClientRect`). The loaded image is 40x30 — off the hinted
//! 100x50 on both axes and in ratio (4:3 against 2:1), so an answer taken from
//! the wrong source is a different number.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::perf::Counter;

/// A 40x30 PNG.
const FORTY_BY_THIRTY: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACgAAAAeCAYAAABe3VzdAAAAPUlEQVR4nO3OIQEAIBAAMTJ9/wC0ggjIQ0zMb+2Z87NVBwQF64CgYB0QFKwDgoJ1QFCwDggK1gFBwTrwcgFD1ConI1jnigAAAABJRU5ErkJggg==";

const BOX: &str = "width: 400px; height: 300px";

/// `body > div(BOX) > <tag {attrs} [style]>`, laid out.
fn build(tag: &str, attrs: &[(&str, &str)], style: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", BOX);
    doc.append_child(body, d);
    let e = doc.create_element(tag);
    for (k, v) in attrs {
        doc.set_attribute(e, k, v);
    }
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(d, e);
    doc.resolve_layout(800.0, 600.0);
    (doc, e)
}

fn size(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let n = doc.tree.get(id.0).unwrap();
    (n.layout.width, n.layout.height)
}

#[track_caller]
fn check(tag: &str, attrs: &[(&str, &str)], style: &str, want: (f32, f32)) {
    let (doc, e) = build(tag, attrs, style);
    assert_eq!(
        size(&doc, e),
        want,
        "<{tag} {attrs:?} style=\"{style}\"> (Chrome 153)"
    );
}

const BOTH: &[(&str, &str)] = &[("width", "100"), ("height", "50")];

#[test]
fn an_image_with_no_source_is_the_size_its_attributes_say() {
    check("img", BOTH, "", (100.0, 50.0));
    check("img", BOTH, "display: block", (100.0, 50.0));
    // One attribute sizes one axis; the other has nothing to come from.
    check("img", &[("width", "100")], "", (100.0, 0.0));
    check("img", &[("height", "50")], "", (0.0, 50.0));
    check("img", &[("width", "100")], "display: block", (100.0, 0.0));
}

#[test]
fn attribute_names_are_case_insensitive() {
    check("img", &[("WIDTH", "100"), ("Height", "50")], "", (100.0, 50.0));
}

#[test]
fn a_percentage_attribute_is_a_percentage_of_the_containing_block() {
    check(
        "img",
        &[("width", "50%"), ("height", "50%")],
        "",
        (200.0, 150.0),
    );
    check(
        "img",
        &[("width", "12.5%"), ("height", "50")],
        "",
        (50.0, 50.0),
    );
}

/// HTML's rules for parsing dimension values, through layout: leading ASCII
/// whitespace, then a digit (no sign, no leading `.`), digits, an optional
/// fraction, an optional `%`; whatever follows is ignored.
#[test]
fn the_attribute_is_read_by_the_dimension_rules() {
    for (value, want) in [
        ("100px", 100.0),
        (" \t100", 100.0),
        ("100abc", 100.0),
        ("100.5", 100.5),
        ("1e2", 1.0),
        ("0", 0.0),
        (".5", 0.0),
        ("+100", 0.0),
        ("-5", 0.0),
        ("abc", 0.0),
        ("", 0.0),
        ("%", 0.0),
        // A legacy relative length (`<col width="2*">`): no length at all.
        ("50*", 0.0),
    ] {
        let (doc, e) = build("img", &[("width", value), ("height", "50")], "");
        assert_eq!(
            size(&doc, e),
            (want, 50.0),
            "<img width={value:?} height=50>"
        );
    }
}

#[test]
fn any_author_declaration_wins() {
    // Inline style.
    check("img", BOTH, "width: 200px", (200.0, 50.0));
    // A stylesheet rule of the lowest specificity there is.
    let mut doc = RinchDocument::new();
    doc.load_css("img { width: 60px }");
    let body = doc.body();
    let img = doc.create_element("img");
    doc.set_attribute(img, "width", "100");
    doc.set_attribute(img, "height", "50");
    doc.append_child(body, img);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, img), (60.0, 50.0), "img {{ width: 60px }}");
}

/// The pair gives an image with no natural size its ratio: an `auto`
/// dimension follows the other one through it.
#[test]
fn both_attributes_give_an_unloaded_image_a_ratio() {
    check("img", BOTH, "width: 200px; height: auto", (200.0, 100.0));
    check("img", BOTH, "height: auto", (100.0, 50.0));
    check("img", BOTH, "width: auto", (100.0, 50.0));
    check(
        "img",
        BOTH,
        "display: block; width: 100%; height: auto",
        (400.0, 200.0),
    );
    // A ratio and no size: nothing to scale.
    check("img", BOTH, "width: auto; height: auto", (0.0, 0.0));
    // … until a minimum gives one axis a size.
    check(
        "img",
        BOTH,
        "width: auto; height: auto; min-width: 40px",
        (40.0, 20.0),
    );
    check(
        "img",
        BOTH,
        "width: auto; height: auto; min-height: 40px",
        (80.0, 40.0),
    );
}

#[test]
fn clamps_apply_to_the_hinted_size() {
    // `height` is still the attribute's 50.
    check("img", BOTH, "max-width: 50px", (50.0, 50.0));
    // `height: auto` follows the clamped width through the ratio.
    check("img", BOTH, "max-width: 50px; height: auto", (50.0, 25.0));
    check("img", BOTH, "min-height: 80px", (100.0, 80.0));
}

/// No ratio comes from a zero, a percentage or an invalid value.
#[test]
fn a_degenerate_pair_gives_no_ratio() {
    check(
        "img",
        &[("width", "100"), ("height", "0")],
        "height: auto",
        (100.0, 0.0),
    );
    check(
        "img",
        &[("width", "50%"), ("height", "50")],
        "height: auto",
        (200.0, 0.0),
    );
    check(
        "img",
        &[("width", "100"), ("height", "50%")],
        "height: auto",
        (100.0, 0.0),
    );
    check(
        "img",
        &[("width", "100"), ("height", "x")],
        "width: 200px",
        (200.0, 0.0),
    );
}

#[test]
fn a_flex_item_image_keeps_its_hinted_size() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", "display: flex; width: 400px; height: 80px");
    doc.append_child(body, d);
    let img = doc.create_element("img");
    doc.set_attribute(img, "width", "100");
    doc.set_attribute(img, "height", "50");
    doc.append_child(d, img);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, img), (100.0, 50.0));
}

/// A loaded image's own ratio replaces the attributes' (`aspect-ratio: auto
/// w / h` prefers the natural ratio), and its natural size fills in for a
/// missing attribute.
#[test]
fn a_loaded_image_uses_its_own_ratio() {
    let loaded = |attrs: &[(&str, &str)], style: &str, want: (f32, f32)| {
        let mut all = vec![("src", FORTY_BY_THIRTY)];
        all.extend_from_slice(attrs);
        check("img", &all, style, want);
    };
    loaded(&[("width", "100")], "", (100.0, 75.0));
    loaded(&[("height", "60")], "", (80.0, 60.0));
    loaded(&[("width", "100")], "display: block", (100.0, 75.0));
    loaded(&[("width", "50%")], "", (200.0, 150.0));
    loaded(BOTH, "", (100.0, 50.0));
    loaded(BOTH, "height: auto", (100.0, 75.0));
    loaded(BOTH, "width: 200px; height: auto", (200.0, 150.0));
    loaded(BOTH, "max-width: 50px", (50.0, 50.0));
    loaded(BOTH, "max-width: 50px; height: auto", (50.0, 37.5));
}

/// The source set after the attributes, as `rsx!` may order them.
#[test]
fn the_source_may_arrive_after_the_attributes() {
    let (mut doc, img) = build("img", BOTH, "height: auto");
    assert_eq!(size(&doc, img), (100.0, 50.0), "before the image");
    doc.set_attribute(img, "src", FORTY_BY_THIRTY);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, img), (100.0, 75.0), "after it");
}

#[test]
fn a_video_takes_the_hints_and_the_ratio() {
    check("video", BOTH, "", (100.0, 50.0));
    // The other axis is the default object size's.
    check("video", &[("width", "100")], "", (100.0, 150.0));
    check("video", &[("height", "50")], "", (300.0, 50.0));
    check(
        "video",
        &[("width", "50%"), ("height", "50%")],
        "",
        (200.0, 150.0),
    );
    check("video", BOTH, "width: 200px; height: auto", (200.0, 100.0));
    let square = &[("width", "100"), ("height", "100")];
    check("video", square, "height: auto", (100.0, 100.0));
    check(
        "video",
        square,
        "width: auto; height: auto; max-width: 60px",
        (60.0, 60.0),
    );
}

/// An iframe takes the two hints and **no** ratio; its 2px UA border sits
/// outside them.
#[test]
fn an_iframe_takes_the_hints_and_no_ratio() {
    check("iframe", BOTH, "", (104.0, 54.0));
    check("iframe", &[("width", "100")], "", (104.0, 154.0));
    check(
        "iframe",
        &[("width", "50%"), ("height", "50%")],
        "",
        (204.0, 154.0),
    );
    check("iframe", BOTH, "width: 200px; height: auto", (204.0, 154.0));
}

/// A canvas's attributes are its bitmap's size, read as integers — not
/// dimension values — and unchanged by this (#1173).
#[test]
fn a_canvas_still_reads_integers() {
    check("canvas", BOTH, "", (100.0, 50.0));
    check("canvas", BOTH, "width: 200px", (200.0, 100.0));
    check(
        "canvas",
        &[("width", "50%"), ("height", "50")],
        "",
        (50.0, 50.0),
    );
}

/// `width`/`height` mean nothing on an element HTML does not map them for.
#[test]
fn other_elements_take_no_hint() {
    check("div", BOTH, "", (400.0, 0.0));
    check("span", BOTH, "display: inline-block", (0.0, 0.0));
}

/// Writing, changing and removing an attribute after the first layout each
/// reach the box, at an unchanged viewport.
#[test]
fn a_later_write_or_removal_relays_out() {
    let (mut doc, img) = build("img", &[], "");
    assert_eq!(size(&doc, img), (0.0, 0.0));
    let mut step = |doc: &mut RinchDocument, what: &str, want: (f32, f32)| {
        doc.resolve_layout(800.0, 600.0);
        assert_eq!(size(doc, img), want, "{what}");
    };
    doc.set_attribute(img, "width", "100");
    doc.set_attribute(img, "height", "50");
    step(&mut doc, "set both", (100.0, 50.0));
    doc.set_attribute(img, "width", "30");
    step(&mut doc, "width 100 -> 30", (30.0, 50.0));
    doc.set_style(img, "height", "auto");
    step(&mut doc, "height: auto, ratio 30 / 50", (30.0, 50.0));
    doc.set_attribute(img, "height", "60");
    step(&mut doc, "ratio 30 / 60", (30.0, 60.0));
    doc.remove_attribute(img, "height");
    step(&mut doc, "height removed: no ratio", (30.0, 0.0));
    doc.remove_attribute(img, "width");
    step(&mut doc, "width removed", (0.0, 0.0));
}

#[test]
fn a_video_ratio_follows_a_later_write() {
    let (mut doc, v) = build("video", BOTH, "width: 200px; height: auto");
    assert_eq!(size(&doc, v), (200.0, 100.0));
    doc.set_attribute(v, "height", "25");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, v), (200.0, 50.0), "ratio 100 / 25");
    doc.remove_attribute(v, "width");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, v), (200.0, 150.0), "no ratio");
}

/// Only the two mapped attributes restyle an image: any other attribute
/// write cascades nothing, as before.
#[test]
fn only_width_and_height_restyle_the_image() {
    let (mut doc, img) = build("img", BOTH, "");
    let cascaded = |doc: &mut RinchDocument, name: &str, value: &str| {
        let before = doc.tree.perf.total();
        doc.set_attribute(img, name, value);
        doc.resolve_layout(800.0, 600.0);
        doc.tree
            .perf
            .total()
            .since(&before)
            .get(Counter::ElementsCascaded)
    };
    assert_eq!(cascaded(&mut doc, "alt", "a picture"), 0, "alt");
    assert_eq!(cascaded(&mut doc, "data-x", "1"), 0, "data-x");
    assert_eq!(cascaded(&mut doc, "width", "120"), 1, "width");
    assert_eq!(cascaded(&mut doc, "width", "120"), 0, "the same width again");
    // And on an element with no mapping, not even those two.
    let (mut doc, div) = build("div", &[], "");
    let before = doc.tree.perf.total();
    doc.set_attribute(div, "width", "100");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree
            .perf
            .total()
            .since(&before)
            .get(Counter::ElementsCascaded),
        0,
        "<div width>"
    );
}
