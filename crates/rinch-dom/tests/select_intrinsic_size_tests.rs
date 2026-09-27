//! The closed `<select>`'s own box, measured against Chrome 153 (issue #1098).
//!
//! A closed select shows one option's label, painted by the backend; its
//! `<option>`s are `display: none`, so nothing inside the box is laid out and
//! its size has to come from somewhere else. Chrome sizes it as
//!
//! ```text
//! width  = border + padding + ceil(widest option label) + 20
//! height = border + padding + one line of `line-height: normal` + 2
//! ```
//!
//! with **no** `min-width` (`getComputedStyle(select).minWidth` is `0px`),
//! `padding: 0`, a `1px solid rgb(118, 118, 118)` border, and a line-height
//! it forces to `normal` whatever the author declares. The `20` and the `2`
//! are the menulist's inner box — a 4px label inset plus a 16px arrow box,
//! and 1px above and below the line — which the author's padding adds to
//! rather than replaces. All measured, Chrome 153 on Linux, with this crate's
//! bundled Inter loaded through `@font-face` under the same override name the
//! fixtures register it with, so the label widths are the same face's:
//!
//! | options | select CSS (plus `font: 16px/20px ProbeFace`) | Chrome |
//! |---|---|---|
//! | `a` | — | 31 x 24 |
//! | `a`, `Hello world option` | — | 160 x 24 |
//! | `Medium`, `Small` | — | 83 x 24 |
//! | none | — | 22 x 24 |
//! | `abc` | `padding: 4px 10px; border: 2px solid` | 72 x 34 |
//! | `Twenty` | `font-size: 20px` | 92 x 28 |
//! | `abc` | `line-height: 40px` | 50 x 24 |
//! | `abc` | `width: 90px` | 90 x 24 |
//! | `abc` | `height: 40px` | 50 x 40 |
//!
//! Every width is matched exactly. The heights are not quite: Chrome's
//! `normal` is the face's rounded ascent plus descent (20px for Inter at
//! 16px), rinch's is `1.2em` everywhere (19.2px), so a height here is
//! `1.2em + 2 + padding + border` and within a pixel of Chrome's. Both are
//! asserted: the rinch formula exactly, Chrome within `1.0`.
//!
//! Before #1098 an unstyled select was `max(60, 8 + 0.62em per char + 24)` wide
//! (the UA `min-width: 60px` beside an estimated label width) and `8px` tall —
//! its vertical padding alone — so its label painted over its own bounds.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

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

/// `div(font: 16px/20px ProbeFace; width: 600px) > select(style) > options`,
/// laid out. Returns the document and the select.
fn select_in(select_css: &str, options: &[&str]) -> (RinchDocument, usize) {
    let mut doc = document();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "font: 16px/20px ProbeFace; width: 600px");
    doc.append_child(body, div);
    let sel = doc.create_element("select");
    doc.set_attribute(
        sel,
        "style",
        &format!("font: 16px/20px ProbeFace; {select_css}"),
    );
    doc.append_child(div, sel);
    for label in options {
        let o = doc.create_element("option");
        doc.append_child(sel, o);
        let t = doc.create_text(label);
        doc.append_child(o, t);
    }
    doc.resolve_layout(800.0, 600.0);
    (doc, sel.0)
}

fn size(select_css: &str, options: &[&str]) -> (f32, f32) {
    let (doc, sel) = select_in(select_css, options);
    let l = doc.tree.get(sel).unwrap().layout;
    (l.width, l.height)
}

/// Asserts the width exactly against Chrome, and the height exactly against
/// rinch's formula and within a pixel of Chrome's.
fn check(case: &str, got: (f32, f32), chrome: (f32, f32), rinch_height: f32) {
    assert_eq!(
        got.0, chrome.0,
        "{case}: width {} where Chrome 153 gives {}",
        got.0, chrome.0
    );
    assert!(
        (got.1 - rinch_height).abs() < 0.01,
        "{case}: height {} where 1.2em + 2 + padding + border is {rinch_height}",
        got.1
    );
    assert!(
        (got.1 - chrome.1).abs() <= 1.0,
        "{case}: height {} is more than a pixel off Chrome's {}",
        got.1,
        chrome.1
    );
}

/// 16px: one line of `normal` is 19.2, plus the 2px inner box and the 1px
/// UA border top and bottom.
const H16: f32 = 19.2 + 2.0 + 2.0;

#[test]
fn a_one_letter_select_is_sized_from_its_label_not_its_padding() {
    check("a", size("", &["a"]), (31.0, 24.0), H16);
}

#[test]
fn the_widest_option_sizes_the_control_whichever_is_selected() {
    check(
        "a / Hello world option",
        size("", &["a", "Hello world option"]),
        (160.0, 24.0),
        H16,
    );
    check(
        "Medium / Small",
        size("", &["Medium", "Small"]),
        (83.0, 24.0),
        H16,
    );
}

/// No label: the arrow box and the border alone, where the UA `min-width:
/// 60px` used to hold it open.
#[test]
fn an_empty_select_is_its_arrow_box_wide() {
    check("empty", size("", &[]), (22.0, 24.0), H16);
}

/// The author's padding and border add to the inner box rather than replace
/// it — Chrome keeps the 20px arrow area inside any padding.
#[test]
fn author_padding_and_border_add_to_the_inner_box() {
    check(
        "padding + 2px border",
        size("padding: 4px 10px; border: 2px solid", &["abc"]),
        (72.0, 34.0),
        19.2 + 2.0 + 8.0 + 4.0,
    );
}

/// Off the 16px fixed point: the label width scales with the font, the 20px
/// arrow box does not.
#[test]
fn a_larger_font_widens_the_label_but_not_the_arrow_box() {
    check(
        "20px",
        size("font-size: 20px", &["Twenty"]),
        (92.0, 28.0),
        24.0 + 2.0 + 2.0,
    );
}

/// Chrome forces a closed select's `line-height` to `normal`, so a declared
/// one moves nothing.
#[test]
fn a_declared_line_height_does_not_change_the_height() {
    check(
        "line-height: 40px",
        size("line-height: 40px", &["abc"]),
        (50.0, 24.0),
        H16,
    );
}

#[test]
fn an_author_width_or_height_wins() {
    check("width: 90px", size("width: 90px", &["abc"]), (90.0, 24.0), H16);
    check("height: 40px", size("height: 40px", &["abc"]), (50.0, 40.0), 40.0);
}

/// The issue's own case: no author CSS on the select at all. Chrome gives a
/// bare select its own font (13.33px Arial), which rinch does not; with the
/// font inherited the box is the formula's, not the 60 x 8 it was.
#[test]
fn an_unstyled_select_is_not_its_padding() {
    let mut doc = document();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "font: 16px/20px ProbeFace; width: 280px");
    doc.append_child(body, div);
    let sel = doc.create_element("select");
    doc.append_child(div, sel);
    let o = doc.create_element("option");
    doc.append_child(sel, o);
    let t = doc.create_text("a");
    doc.append_child(o, t);
    doc.resolve_layout(800.0, 600.0);
    let l = doc.tree.get(sel.0).unwrap().layout;
    assert_eq!((l.width, l.height), (31.0, H16));
}

/// A local pixel oracle for where paint puts the label and the arrow now that
/// the UA sheet gives the select no padding: the label starts 4px inside the
/// border, and the arrow sits in the last 16px of the content box.
///
/// `IIII` in 40px Inter; Chrome draws its first stem from x = 18 with the
/// select at x = 10 (border 1, inset 4, the glyph's own side bearing 3).
#[test]
fn the_label_is_inset_and_the_arrow_is_in_the_arrow_box() {
    const VW: u32 = 300;
    const VH: u32 = 120;
    let mut doc = document();
    let body = doc.body();
    let sel = doc.create_element("select");
    doc.set_attribute(
        sel,
        "style",
        "position: absolute; left: 10px; top: 10px; font: 40px ProbeFace; \
         color: rgb(0, 0, 0)",
    );
    doc.append_child(body, sel);
    let o = doc.create_element("option");
    doc.append_child(sel, o);
    let t = doc.create_text("IIII");
    doc.append_child(o, t);
    doc.resolve_layout(VW as f32, VH as f32);
    let l = doc.tree.get(sel.0).unwrap().layout;
    assert_eq!(l.width, 65.0, "Chrome's width for this select");

    let mut painter = TinySkiaPainter::new(VW, VH);
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW as f32, VH as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    let px: Vec<[u8; 4]> = painter.pixels().as_chunks::<4>().0.to_vec();
    // Black ink only: the UA border is rgb(118, 118, 118).
    let black = |x: u32, y: u32| {
        let p = px[(y * VW + x) as usize];
        p[3] > 200 && p[0] < 60 && p[1] < 60 && p[2] < 60
    };
    let top = 10 + 1;
    let bottom = 10 + l.height as u32 - 1;
    let inked: Vec<u32> = (11..74)
        .filter(|&x| (top..bottom).any(|y| black(x, y)))
        .collect();
    assert!(!inked.is_empty(), "nothing black was painted: no positive control");
    let first = inked[0];
    assert!(
        (17..=19).contains(&first),
        "the label's first stem is at x = {first}; Chrome's is at 18 \
         (border 1 + inset 4 + side bearing)"
    );
    // The arrow: black ink in the last 16px of the content box, x in 58..74.
    assert!(
        inked.iter().any(|&x| (58..74).contains(&x)),
        "no arrow in the arrow box: {inked:?}"
    );
    // Nothing between the label's end and the arrow box's start — the last
    // stem of `IIII` ends at 53 in Chrome.
    assert!(
        !inked.iter().any(|&x| (55..58).contains(&x)),
        "ink in the gap before the arrow box: {inked:?}"
    );
}
