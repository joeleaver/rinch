//! A closed `<select>`'s intrinsic size composes with the author's sizing
//! properties as any box's content size does (issue #1129).
//!
//! #1098 sized a select from its widest option label by **overwriting its
//! `min-width`** (and `min-height`) with that size. A `min-width` is not a
//! content size: it beat an author `max-width`, replaced an author percentage
//! `min-width`, and a `display: block` select was stretched to its container
//! where Chrome keeps it at its label width. The label size is now the
//! select's Taffy **measure** (`NodeContext::FormControl`, the one a text
//! input carries), so the author's properties apply to it normally.
//!
//! Every expectation is Chrome 155, measured on Linux with this crate's
//! bundled Inter under the same override name (`ProbeFace`), options `a` and
//! `Hello world option`, the select in a `div(font: 16px/20px ProbeFace;
//! width: 600px)` unless the row says otherwise:
//!
//! | container | select CSS | Chrome 155 |
//! |---|---|---|
//! | — | — | 160 x 24 |
//! | — | `max-width: 30px` | 30 x 24 |
//! | — | `padding: 4px 10px; max-width: 30px` | 30 x 32 |
//! | — | `min-width: 50%` | 300 x 24 |
//! | — | `min-width: 200px` | 200 x 24 |
//! | — | `width: 50%` | 300 x 24 |
//! | `width: 100px` | `display: block; max-width: 50%` | 50 x 24 |
//! | — | `display: block` | 160 x 24 |
//! | `width: 50px` | `display: block` | 160 x 24 |
//! | `width: 50px` | — | 160 x 24 |
//! | `height: 300px` | `display: block; min-height: 50%` | 160 x 150 |
//! | `height: 300px` | `display: block; max-height: 10%; height: 200px` | 160 x 30 |
//! | `display: flex; width: 50px` | — | 160 x 24 |
//! | `display: flex; width: 50px` | `min-width: 0` | 50 x 24 |
//! | `display: grid; width: 50px` | — | 160 x 24 |
//! | `display: flex; flex-direction: column` | — | 600 x 24 |
//!
//! Chrome computes a select's `overflow` as `clip`, not `hidden`: a `clip`
//! box is no scroll container, so its automatic minimum size as a flex or
//! grid item is its content size and it does not shrink in a 50px row. With
//! `hidden` the measure would be served as a scroll container's, and shrink.
//!
//! At its default `display` a select is an atomic inline, and an atomic
//! inline's percentage `max-width`, `min-height` and `max-height` do not
//! resolve as Chrome's yet, whatever the box (#1738): Chrome gives the
//! inline select 50 x 24 and 160 x 150 for the two percentage rows above,
//! rinch 40 x 23 and 160 x 23. Pinned below as a known difference.
//!
//! Heights: rinch's `line-height: normal` is `1.2em` (19.2px at 16px) where
//! Chrome's is the face's rounded ascent plus descent (20px), so a select is
//! 23 tall here where Chrome's is 24 — the #1098 difference, asserted within
//! a pixel. A height an author property sets is asserted exactly.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn document() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    doc
}

/// `div(font: 16px/20px ProbeFace; width: 600px; {container}) >
/// select(font: 16px/20px ProbeFace; {select_css}) > option a, option Hello
/// world option`, laid out; the select's border box.
fn size(container: &str, select_css: &str) -> (f32, f32) {
    let mut doc = document();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        &format!("font: 16px/20px ProbeFace; width: 600px; {container}"),
    );
    doc.append_child(body, div);
    let sel = doc.create_element("select");
    doc.set_attribute(
        sel,
        "style",
        &format!("font: 16px/20px ProbeFace; {select_css}"),
    );
    doc.append_child(div, sel);
    for label in ["a", "Hello world option"] {
        let o = doc.create_element("option");
        doc.append_child(sel, o);
        let t = doc.create_text(label);
        doc.append_child(o, t);
    }
    doc.resolve_layout(800.0, 600.0);
    let l = doc.tree.get(sel.0).unwrap().layout;
    (l.width, l.height)
}

/// Width exactly; height within a pixel of Chrome's (see the module doc).
fn check(container: &str, select_css: &str, chrome: (f32, f32)) {
    let got = size(container, select_css);
    assert_eq!(
        got.0, chrome.0,
        "[{container}] select {{{select_css}}}: width {} where Chrome 155 gives {}",
        got.0, chrome.0
    );
    assert!(
        (got.1 - chrome.1).abs() <= 1.0,
        "[{container}] select {{{select_css}}}: height {} is more than a pixel off \
         Chrome's {}",
        got.1,
        chrome.1
    );
}

#[test]
fn an_author_max_width_below_the_label_width_wins() {
    check("", "max-width: 30px", (30.0, 24.0));
    check("", "padding: 4px 10px; max-width: 30px", (30.0, 32.0));
    check("width: 100px", "display: block; max-width: 50%", (50.0, 24.0));
}

#[test]
fn an_author_min_width_applies_as_a_minimum_percentage_included() {
    check("", "min-width: 50%", (300.0, 24.0));
    check("", "min-width: 200px", (200.0, 24.0));
    check("", "width: 50%", (300.0, 24.0));
}

#[test]
fn a_block_select_keeps_its_label_width() {
    check("", "display: block", (160.0, 24.0));
    check("width: 50px", "display: block", (160.0, 24.0));
    check("width: 50px", "", (160.0, 24.0));
}

#[test]
fn a_percentage_min_or_max_height_resolves() {
    let got = size("height: 300px", "display: block; min-height: 50%");
    assert_eq!(got, (160.0, 150.0), "Chrome 155: 160 x 150");
    let got = size("height: 300px", "display: block; max-height: 10%; height: 200px");
    assert_eq!(got, (160.0, 30.0), "Chrome 155: 160 x 30");
}

/// Not Chrome's: an atomic inline's percentage `max-width` and `min-height`
/// (#1738). Chrome 155 gives 50 x 24 and 160 x 150. When this fails because
/// rinch got Chrome's answer, move these rows into the tests above.
#[test]
fn known_difference_an_inline_selects_percentage_max_width_and_min_height_1738() {
    assert_eq!(size("width: 100px", "max-width: 50%"), (40.0, 23.0));
    assert_eq!(size("height: 300px", "min-height: 50%"), (160.0, 23.0));
}

#[test]
fn as_a_flex_or_grid_item_it_does_not_shrink_below_its_label_unless_asked() {
    check("display: flex; width: 50px", "", (160.0, 24.0));
    check("display: flex; width: 50px", "min-width: 0", (50.0, 24.0));
    check("display: grid; width: 50px", "", (160.0, 24.0));
    check("display: flex; flex-direction: column", "", (600.0, 24.0));
}

#[test]
fn the_default_select_is_unchanged() {
    check("", "", (160.0, 24.0));
}

/// The measure follows the labels and the width after the first layout: an
/// option appended later widens the select (the content change re-syncs the
/// measure, marking it dirty), and so does a `px` width taken back to
/// `auto`, which shapes the labels it did not need while the width was
/// declared.
#[test]
fn the_measure_follows_a_later_option_and_a_width_taken_back_to_auto() {
    let mut doc = document();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "font: 16px/20px ProbeFace; width: 600px");
    doc.append_child(body, div);
    let sel = doc.create_element("select");
    doc.set_attribute(sel, "style", "font: 16px/20px ProbeFace; width: 90px");
    doc.append_child(div, sel);
    let o = doc.create_element("option");
    doc.append_child(sel, o);
    let t = doc.create_text("a");
    doc.append_child(o, t);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(doc.tree.get(sel.0).unwrap().layout.width, 90.0);

    doc.set_attribute(sel, "style", "font: 16px/20px ProbeFace");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.get(sel.0).unwrap().layout.width,
        31.0,
        "Chrome 153: an `a`-only select is 31 wide"
    );

    let o2 = doc.create_element("option");
    doc.append_child(sel, o2);
    let t2 = doc.create_text("Hello world option");
    doc.append_child(o2, t2);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.get(sel.0).unwrap().layout.width,
        160.0,
        "Chrome 153: with `Hello world option` it is 160 wide"
    );
}
