//! Review fixtures for PR #1427 (#684, presentational `width`/`height`):
//! the cascade level, invalidation and ratio cases beyond
//! `presentational_size_hints_tests.rs`.
//! Every number is Chrome 153's (headless, standards mode, `body { margin: 0
//! }`, the element inside a `width: 400px; height: 300px` div unless stated).

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::perf::Counter;

const FORTY_BY_THIRTY: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACgAAAAeCAYAAABe3VzdAAAAPUlEQVR4nO3OIQEAIBAAMTJ9/wC0ggjIQ0zMb+2Z87NVBwQF64CgYB0QFKwDgoJ1QFCwDggK1gFBwTrwcgFD1ConI1jnigAAAABJRU5ErkJggg==";
const BOX: &str = "width: 400px; height: 300px";
const BOTH: &[(&str, &str)] = &[("width", "100"), ("height", "50")];
const W: &[(&str, &str)] = &[("width", "100")];

fn build_in(
    css: &str,
    host_style: &str,
    tag: &str,
    attrs: &[(&str, &str)],
    style: &str,
) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    if !css.is_empty() {
        doc.load_css(css);
    }
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", host_style);
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
    (doc, d, e)
}

fn size(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let n = doc.tree.get(id.0).unwrap();
    (n.layout.width, n.layout.height)
}

#[track_caller]
fn check_css(css: &str, tag: &str, attrs: &[(&str, &str)], style: &str, want: (f32, f32)) {
    let (doc, _, e) = build_in(css, BOX, tag, attrs, style);
    assert_eq!(
        size(&doc, e),
        want,
        "css {css:?} <{tag} {attrs:?} style={style:?}>"
    );
}

// ---- (1) cascade level -------------------------------------------------

#[test]
fn cascade_sheet_width_auto_beats_the_hint() {
    check_css("img { width: auto }", "img", W, "", (0.0, 0.0));
}

#[test]
fn cascade_universal_rule_beats_the_hint() {
    check_css("* { width: 60px }", "img", W, "", (60.0, 0.0));
}

/// A layered author rule loses to every unlayered author rule, and still
/// beats a presentational hint. (Kills "the hint is pushed at the unlayered
/// author level".)
#[test]
fn cascade_layered_rule_beats_the_hint() {
    check_css(
        "@layer l { img { width: 60px } }",
        "img",
        W,
        "",
        (60.0, 0.0),
    );
}

#[test]
fn cascade_important_sheet_beats_inline_and_hint() {
    check_css(
        "img { width: 60px !important }",
        "img",
        W,
        "width: 10px",
        (60.0, 0.0),
    );
}

#[test]
fn cascade_inline_unset_beats_the_hint() {
    check_css("", "img", W, "width: unset", (0.0, 0.0));
}

/// `revert` in the author origin rolls back past the hint (Chrome: 0x0).
#[test]
fn cascade_revert_removes_the_hint() {
    check_css("", "img", W, "width: revert", (0.0, 0.0));
    check_css("img { width: revert }", "img", W, "", (0.0, 0.0));
}

/// `revert-layer` in inline style keeps the hint (Chrome: 100x0).
#[test]
fn cascade_revert_layer_keeps_the_hint() {
    check_css("", "img", W, "width: revert-layer", (100.0, 0.0));
}

#[test]
fn cascade_inherit_beats_the_hint() {
    check_css("", "img", BOTH, "width: inherit", (400.0, 50.0));
}

// ---- (2) invalidation ---------------------------------------------------

#[test]
fn two_sibling_images_keep_their_own_hints() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", BOX);
    doc.append_child(body, d);
    let a = doc.create_element("img");
    doc.set_attribute(a, "width", "100");
    doc.set_attribute(a, "height", "50");
    doc.append_child(d, a);
    let b = doc.create_element("img");
    doc.set_attribute(b, "width", "30");
    doc.set_attribute(b, "height", "20");
    doc.append_child(d, b);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, a), (100.0, 50.0));
    assert_eq!(size(&doc, b), (30.0, 20.0));
}

#[test]
fn attributes_on_a_detached_image_apply_when_it_is_inserted_later() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", BOX);
    doc.append_child(body, d);
    doc.resolve_layout(800.0, 600.0);
    let img = doc.create_element("img");
    doc.set_attribute(img, "width", "100");
    doc.set_attribute(img, "height", "50");
    doc.set_attribute(img, "style", "width: 200px; height: auto");
    doc.resolve_layout(800.0, 600.0);
    doc.append_child(d, img);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, img), (200.0, 100.0));
    // Out and back in.
    doc.remove_child(d, img);
    doc.resolve_layout(800.0, 600.0);
    doc.set_attribute(img, "height", "25");
    doc.append_child(d, img);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, img), (200.0, 50.0));
}

#[test]
fn inner_html_images_take_the_hints() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", BOX);
    doc.append_child(body, d);
    doc.set_inner_html(
        d,
        "<img width=\"100\" height=\"50\" style=\"width: 200px; height: auto\">",
    );
    doc.resolve_layout(800.0, 600.0);
    let img = doc.tree.get(d.0).unwrap().children[0];
    assert_eq!(size(&doc, NodeId(img)), (200.0, 100.0));
}

#[test]
fn uppercase_removal_removes_the_hint() {
    let (mut doc, _, img) = build_in("", BOX, "img", BOTH, "");
    doc.remove_attribute(img, "WIDTH");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, img), (0.0, 50.0));
    doc.remove_attribute(img, "Height");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, img), (0.0, 0.0));
    assert!(doc.tree.get(img.0).unwrap().presentational_hints.is_none());
}

#[test]
fn unmapped_elements_hold_no_hint_block() {
    for tag in [
        "div", "span", "canvas", "input", "svg", "image", "rect", "table", "td", "hr", "object",
        "embed",
    ] {
        let (doc, _, e) = build_in("", BOX, tag, BOTH, "");
        assert!(
            doc.tree.get(e.0).unwrap().presentational_hints.is_none(),
            "<{tag}>"
        );
    }
    // <input type=image>
    let (doc, _, e) = build_in(
        "",
        BOX,
        "input",
        &[("type", "image"), ("width", "100"), ("height", "50")],
        "",
    );
    assert!(doc.tree.get(e.0).unwrap().presentational_hints.is_none());
}

/// An svg's own `width`/`height` and an svg `<image>`'s are untouched.
#[test]
fn svg_sizes_are_unchanged() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", BOX);
    doc.append_child(body, d);
    let svg = doc.create_element("svg");
    doc.set_attribute(svg, "width", "120");
    doc.set_attribute(svg, "height", "60");
    doc.append_child(d, svg);
    let image = doc.create_element("image");
    doc.set_attribute(image, "width", "30");
    doc.set_attribute(image, "height", "20");
    doc.append_child(svg, image);
    doc.resolve_layout(800.0, 600.0);
    assert!(doc.tree.get(svg.0).unwrap().presentational_hints.is_none());
    assert!(
        doc.tree
            .get(image.0)
            .unwrap()
            .presentational_hints
            .is_none()
    );
}

/// A class / hover change on an image re-cascades it and rebuilds no hint
/// block; `[width]` selectors and their sibling combinators still follow a
/// write.
#[test]
fn a_class_change_reuses_the_hint_block_and_attribute_selectors_follow() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        "img.big { height: 80px } img[width=\"30\"] { height: 70px } \
         img[width=\"30\"] + p { height: 33px } p { height: 10px; margin: 0 }",
    );
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", BOX);
    doc.append_child(body, d);
    let img = doc.create_element("img");
    doc.set_attribute(img, "width", "100");
    doc.set_attribute(img, "height", "50");
    doc.set_attribute(img, "style", "display: block");
    doc.append_child(d, img);
    let p = doc.create_element("p");
    doc.append_child(d, p);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, img), (100.0, 50.0));
    assert_eq!(size(&doc, p).1, 10.0);
    let ptr = |doc: &RinchDocument| {
        let b = doc
            .tree
            .get(img.0)
            .unwrap()
            .presentational_hints
            .as_ref()
            .unwrap();
        &**b as *const _ as usize
    };
    let before = ptr(&doc);
    doc.set_attribute(img, "class", "big");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, img), (100.0, 80.0));
    assert_eq!(ptr(&doc), before, "a class write rebuilt the hint block");
    doc.set_attribute(img, "alt", "x");
    doc.set_attribute(img, "src", "nope.png");
    assert_eq!(ptr(&doc), before, "alt/src rebuilt the hint block");
    doc.set_attribute(img, "class", "");
    doc.set_attribute(img, "width", "30");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(size(&doc, img), (30.0, 70.0));
    assert_eq!(size(&doc, p).1, 33.0, "img[width] + p after a write");
}

// ---- (3) ratio ------------------------------------------------------------

/// The responsive-image idiom.
#[test]
fn responsive_idiom() {
    let css = "img { max-width: 100%; height: auto }";
    let big = &[("width", "800"), ("height", "400")];
    check_css(css, "img", big, "", (400.0, 200.0));
    check_css(css, "img", big, "display: block", (400.0, 200.0));
    check_css(
        css,
        "img",
        &[
            ("src", FORTY_BY_THIRTY),
            ("width", "800"),
            ("height", "400"),
        ],
        "",
        (400.0, 300.0),
    );
}

/// (Content-box rows — 116x66 and 110x60 in Chrome — are #1278.)
#[test]
fn a_border_box_image_takes_the_ratio_on_its_content_box() {
    check_css(
        "",
        "img",
        BOTH,
        "padding: 5px; height: auto; box-sizing: border-box",
        (100.0, 55.0),
    );
}

#[test]
fn clamps_through_the_ratio() {
    check_css(
        "",
        "img",
        BOTH,
        "min-width: 150px; height: auto",
        (150.0, 75.0),
    );
    check_css(
        "",
        "img",
        BOTH,
        "width: auto; max-height: 20px",
        (40.0, 20.0),
    );
}

#[test]
fn out_of_flow_images() {
    let host = "width: 400px; height: 300px; position: relative";
    let (doc, _, e) = build_in(
        "",
        host,
        "img",
        BOTH,
        "position: absolute; height: auto; width: 200px",
    );
    assert_eq!(size(&doc, e), (200.0, 100.0), "abs, height: auto");
    let (doc, _, e) = build_in("", host, "img", BOTH, "position: absolute");
    assert_eq!(size(&doc, e), (100.0, 50.0), "abs");
    let (doc, _, e) = build_in("", BOX, "img", BOTH, "position: fixed");
    assert_eq!(size(&doc, e), (100.0, 50.0), "fixed");
}

#[test]
fn flex_column_stretch_goes_through_the_ratio() {
    let host = "display: flex; flex-direction: column; width: 400px; height: 300px";
    let (doc, _, e) = build_in("", host, "img", BOTH, "");
    assert_eq!(size(&doc, e), (100.0, 50.0));
    let (doc, _, e) = build_in("", host, "img", BOTH, "width: auto; height: auto");
    assert_eq!(size(&doc, e), (400.0, 200.0), "img");
    let (doc, _, e) = build_in("", host, "video", BOTH, "width: auto; height: auto");
    assert_eq!(size(&doc, e), (400.0, 200.0), "video");
}

#[test]
fn grid_item_image() {
    let host = "display: grid; width: 400px; height: 300px";
    let (doc, _, e) = build_in("", host, "img", BOTH, "");
    assert_eq!(size(&doc, e), (100.0, 50.0));
    // (auto/auto is Chrome 0x0 and rinch stretched: #1280.)
}

#[test]
fn zero_attributes_on_a_loaded_image() {
    check_css(
        "",
        "img",
        &[("src", FORTY_BY_THIRTY), ("width", "0"), ("height", "0")],
        "",
        (0.0, 0.0),
    );
    check_css(
        "",
        "img",
        &[("src", FORTY_BY_THIRTY), ("width", "0")],
        "",
        (0.0, 0.0),
    );
    check_css(
        "",
        "img",
        &[("src", FORTY_BY_THIRTY), ("width", "100"), ("height", "50")],
        "width: auto; height: auto",
        (40.0, 30.0),
    );
}

/// Chrome clamps to 33554428px. Not asserting the number, only that layout
/// finishes with something finite and nothing explodes.
#[test]
fn huge_values_are_finite() {
    for v in ["1000000000", "99999999999999999999", &"9".repeat(300)] {
        let (doc, _, e) = build_in("", BOX, "img", &[("width", v), ("height", "50")], "");
        let (w, h) = size(&doc, e);
        assert!(w.is_finite() && h == 50.0, "{v}: {w}x{h}");
    }
    let (doc, _, e) = build_in(
        "",
        BOX,
        "img",
        &[("width", "99999999999999999999"), ("height", "1")],
        "width: 200px; height: auto",
    );
    assert!(size(&doc, e).1.is_finite());
    let (doc, _, e) = build_in(
        "",
        BOX,
        "img",
        &[("width", "1"), ("height", "99999999999999999999")],
        "width: 200px; height: auto",
    );
    assert!(size(&doc, e).1.is_finite());
}

// ---- (5) perf -------------------------------------------------------------

#[test]
fn an_image_heavy_page_cascades_one_element_per_change() {
    let mut doc = RinchDocument::new();
    doc.load_css("img.on { opacity: 0.5 }");
    let body = doc.body();
    let d = doc.create_element("div");
    doc.set_attribute(d, "style", "width: 400px");
    doc.append_child(body, d);
    let mut imgs = vec![];
    for i in 0..500 {
        let img = doc.create_element("img");
        doc.set_attribute(img, "width", &format!("{}", 20 + i % 7));
        doc.set_attribute(img, "height", "20");
        doc.set_attribute(img, "style", "display: block");
        doc.append_child(d, img);
        imgs.push(img);
    }
    doc.resolve_layout(800.0, 600.0);
    let before = doc.tree.perf.total();
    doc.set_attribute(imgs[250], "class", "on");
    doc.resolve_layout(800.0, 600.0);
    let diff = doc.tree.perf.total().since(&before);
    assert_eq!(diff.get(Counter::ElementsCascaded), 1);
    assert_eq!(
        diff.get(Counter::TaffyRootComputes),
        0,
        "a paint-only class change"
    );
    let before = doc.tree.perf.total();
    doc.set_attribute(imgs[250], "width", "99");
    doc.resolve_layout(800.0, 600.0);
    let diff = doc.tree.perf.total().since(&before);
    assert_eq!(diff.get(Counter::ElementsCascaded), 1);
    assert_eq!(size(&doc, imgs[250]), (99.0, 20.0));
    assert_eq!(size(&doc, imgs[251]).1, 20.0);
}

/// A ratio-only change (the hinted `height` loses to an author `height:
/// auto`, so no Taffy style moves) must still re-measure a **block-level**
/// and a **flex-item** image. The PR's own later-write fixtures are an inline
/// image (re-measured by the atomic-inline sizer) and a video
/// (`set_node_context`), so neither reaches `sync_replaced_measure`'s
/// `mark_dirty` for an image.
#[test]
fn a_ratio_only_change_remeasures_a_block_and_a_flex_item_image() {
    for host in [
        BOX,
        "display: flex; flex-direction: column; align-items: flex-start; width: 400px; height: 300px",
    ] {
        let (mut doc, _, img) = build_in(
            "",
            host,
            "img",
            BOTH,
            "display: block; width: 200px; height: auto",
        );
        assert_eq!(size(&doc, img), (200.0, 100.0), "{host}");
        doc.set_attribute(img, "height", "25");
        doc.resolve_layout(800.0, 600.0);
        assert_eq!(size(&doc, img), (200.0, 50.0), "ratio 100 / 25, {host}");
        doc.remove_attribute(img, "width");
        doc.resolve_layout(800.0, 600.0);
        assert_eq!(size(&doc, img), (200.0, 0.0), "no ratio, {host}");
    }
}
