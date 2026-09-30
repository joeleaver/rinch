//! A text-entry form control is as wide as its `size` or `cols` (#1177).
//!
//! An `<input>` or `<textarea>` whose width is intrinsic — inline, a flex-row
//! item, inside an `inline-block` — was **0px** wide on desktop: the Taffy
//! measure (`NodeContext::FormControl`, `form_control.rs`) answered a height
//! and a width of zero, and nothing read `size` or `cols`. Chrome takes the
//! width from the primary font's average character width (`avg`, the OS/2
//! `xAvgCharWidth` scaled to the font size) and, for an `<input>`, its widest
//! glyph extent (`max`, the `head` table's `xMax - xMin`, scaled and rounded):
//!
//! ```text
//! avg'     = max(avg, round(avg))
//! input    = ceil(avg' × size + round(max) − avg')      size: default 20
//! textarea = ceil(avg' × cols) + 15                      cols: default 20
//! ```
//!
//! `15` is the scrollbar gutter Chrome reserves in a textarea's intrinsic
//! width, unless its `overflow-y` is `hidden` or `clip`. `size` and `cols` are
//! read by HTML's rules for parsing non-negative integers, and 0 or an invalid
//! value is the default.
//!
//! Every number below is measured in Chrome 153 on Linux, on
//! `input, textarea { padding: 0; border: 0; line-height: 20px }` with this
//! crate's bundled Inter loaded through `@font-face` under the same override
//! name the fixtures register it with. Inter's metrics: `xAvgCharWidth` 1311,
//! `xMax - xMin` 6803, 2048 units per em — so at 16px `avg` is 10.2422 and
//! `max` 53.1484.
//!
//! The `max(avg, round(avg))` step is Chrome's, read off its output rather
//! than its source: sweeping Inter and DejaVu Sans from 8px to 40px in half
//! pixels, the average Chrome uses is the unrounded one wherever its fraction
//! is below one half and the rounded one wherever it is at or above — Inter at
//! 20px is 12.80, used as 13. The 20px fixtures sit on that branch.

use rinch_core::dom::{DomDocument, NodeId};
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

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    doc.set_attribute(e, "style", style);
    doc.append_child(parent, e);
    e
}

/// The bare control's style at `px`: no padding, no border, a declared line.
fn bare(px: f32) -> String {
    format!("font: {px}px/20px ProbeFace; padding: 0; border: 0")
}

/// A 600px block holding one control of `tag` with `attrs` and `extra` CSS,
/// laid out. The control is inline (the UA sheet's `inline-block`), so its
/// width is intrinsic.
fn control(tag: &str, px: f32, attrs: &[(&str, &str)], extra: &str) -> (RinchDocument, NodeId) {
    let mut doc = document();
    let body = doc.body();
    let div = el(
        &mut doc,
        body,
        "div",
        "width: 600px; font: 16px/20px ProbeFace",
    );
    let c = doc.create_element(tag);
    for (k, v) in attrs {
        doc.set_attribute(c, k, v);
    }
    doc.set_attribute(c, "style", &format!("{}; {extra}", bare(px)));
    doc.append_child(div, c);
    doc.resolve_layout(800.0, 600.0);
    (doc, c)
}

fn width(doc: &RinchDocument, id: NodeId) -> f32 {
    doc.tree.get(id.0).unwrap().layout.width
}

fn w(tag: &str, px: f32, attrs: &[(&str, &str)], extra: &str) -> f32 {
    let (doc, c) = control(tag, px, attrs, extra);
    width(&doc, c)
}

/// The default `size` (20) across font sizes. Chrome 153: 155, 202, 248, 271,
/// 313, 372. rinch gave 0 for every one.
#[test]
fn an_input_is_twenty_average_characters_plus_the_widest_glyph() {
    for (px, chrome) in [
        (10.0, 155.0),
        (13.0, 202.0),
        (16.0, 248.0),
        (17.5, 271.0),
        (20.0, 313.0),
        (24.0, 372.0),
    ] {
        assert_eq!(
            w("input", px, &[], ""),
            chrome,
            "input at {px}px: Chrome 153 {chrome}"
        );
    }
}

/// `size` at 16px. Chrome 153: `5` is 94 and `1` is 53 (`round(max)` alone);
/// `0`, `-3` and `abc` are the default, 248; `" 7"` and `"7px"` parse to 7
/// (115), `"+5"` to 5 (94), `"3.9"` to 3 (74).
#[test]
fn size_is_parsed_by_the_html_integer_rules() {
    for (size, chrome) in [
        ("5", 94.0),
        ("1", 53.0),
        ("0", 248.0),
        ("-3", 248.0),
        ("abc", 248.0),
        (" 7", 115.0),
        ("7px", 115.0),
        ("+5", 94.0),
        ("3.9", 74.0),
    ] {
        assert_eq!(
            w("input", 16.0, &[("size", size)], ""),
            chrome,
            "size={size:?}: Chrome 153 {chrome}"
        );
    }
}

/// `cols` defaults to 20 and a textarea adds a 15px scrollbar gutter.
/// Chrome 153: 144, 182, 220, 240, 275, 323.
#[test]
fn a_textarea_is_twenty_average_characters_plus_the_scrollbar_gutter() {
    for (px, chrome) in [
        (10.0, 144.0),
        (13.0, 182.0),
        (16.0, 220.0),
        (17.5, 240.0),
        (20.0, 275.0),
        (24.0, 323.0),
    ] {
        assert_eq!(
            w("textarea", px, &[], ""),
            chrome,
            "textarea at {px}px: Chrome 153 {chrome}"
        );
    }
}

/// `cols` at 16px. Chrome 153: `5` is 67, `1` is 26, `0` and `-3` are 220,
/// `" 7"` and `"7px"` are 87, `"+5"` 67, `"3.9"` 46.
#[test]
fn cols_is_parsed_by_the_html_integer_rules() {
    for (cols, chrome) in [
        ("5", 67.0),
        ("1", 26.0),
        ("0", 220.0),
        ("-3", 220.0),
        (" 7", 87.0),
        ("7px", 87.0),
        ("+5", 67.0),
        ("3.9", 46.0),
    ] {
        assert_eq!(
            w("textarea", 16.0, &[("cols", cols)], ""),
            chrome,
            "cols={cols:?}: Chrome 153 {chrome}"
        );
    }
}

/// The gutter goes when the block axis cannot scroll. Chrome 153 at 16px:
/// `overflow-y: hidden` and `overflow: clip` are 205; `overflow-x: hidden`
/// (the block axis still scrolls) is 220.
#[test]
fn a_textarea_that_cannot_scroll_vertically_reserves_no_gutter() {
    assert_eq!(w("textarea", 16.0, &[], "overflow-y: hidden"), 205.0);
    assert_eq!(w("textarea", 16.0, &[], "overflow: clip"), 205.0);
    assert_eq!(w("textarea", 16.0, &[], "overflow-x: hidden"), 220.0);
}

/// The text-like types read `size`; `number` ignores it and keeps the default
/// (Chrome 153 at 16px: every one of these is 248 unsized, and 94 at
/// `size=5` except `number`, 248). `type` is case-insensitive, and an unknown
/// or empty `type` is the Text state.
#[test]
fn the_text_like_types_read_size_and_number_does_not() {
    for ty in [
        "text", "TEXT", "search", "url", "tel", "email", "password", "foo", "",
    ] {
        assert_eq!(w("input", 16.0, &[("type", ty)], ""), 248.0, "type={ty:?}");
        assert_eq!(
            w("input", 16.0, &[("type", ty), ("size", "5")], ""),
            94.0,
            "type={ty:?} size=5: Chrome 153 94"
        );
    }
    assert_eq!(
        w("input", 16.0, &[("type", "number"), ("size", "5")], ""),
        248.0
    );
}

/// The width is the content box's: padding and border add to it. Chrome 153:
/// `padding: 0 10px; border: 2px solid` is 272 (248 + 24), in rinch's
/// `border-box` as in Chrome's `content-box`, since the width is `auto`.
#[test]
fn padding_and_border_add_to_the_intrinsic_width() {
    assert_eq!(
        w(
            "input",
            16.0,
            &[],
            "padding: 0 10px; border: 2px solid black"
        ),
        272.0
    );
}

/// An author width wins, and so does a `max-width` (Chrome 153: 100 each).
#[test]
fn an_author_width_or_max_width_wins() {
    assert_eq!(w("input", 16.0, &[], "width: 100px"), 100.0);
    assert_eq!(w("input", 16.0, &[], "max-width: 100px"), 100.0);
    assert_eq!(w("textarea", 16.0, &[], "width: 100px"), 100.0);
    assert_eq!(w("textarea", 16.0, &[], "max-width: 100px"), 100.0);
}

/// A flex-row item's width is its content width too (Chrome 153: 248 and
/// 220), and an inline input in a container narrower than it does not shrink
/// (Chrome 153: 248 in a 50px div) — its min-content width is its width.
#[test]
fn a_flex_item_and_an_overflowing_inline_keep_the_intrinsic_width() {
    let mut doc = document();
    let body = doc.body();
    let row = el(&mut doc, body, "div", "display: flex; width: 600px");
    let input = el(&mut doc, row, "input", &bare(16.0));
    let textarea = el(&mut doc, row, "textarea", &bare(16.0));
    let narrow = el(&mut doc, body, "div", "width: 50px");
    let squeezed = el(&mut doc, narrow, "input", &bare(16.0));
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, input), 248.0);
    assert_eq!(width(&doc, textarea), 220.0);
    assert_eq!(width(&doc, squeezed), 248.0);
}

/// A textarea whose value is a text child is as wide as an empty one (#1159:
/// its children are its value, not content).
#[test]
fn a_text_child_textarea_is_as_wide_as_an_empty_one() {
    let mut doc = document();
    let body = doc.body();
    let div = el(&mut doc, body, "div", "width: 600px");
    let ta = el(&mut doc, div, "textarea", &bare(16.0));
    let t = doc.create_text("a long default value that is wider than twenty columns");
    doc.append_child(ta, t);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, ta), 220.0);
}

/// Writing `size` or `cols` after the first layout resizes the control: the
/// attribute is no selector input, so the write has to restyle the node for
/// the measure to be re-read. So does a `font-size` change, which moves no
/// Taffy style.
#[test]
fn a_size_cols_or_font_change_after_layout_resizes_the_control() {
    let (mut doc, input) = control("input", 16.0, &[], "");
    assert_eq!(width(&doc, input), 248.0);
    doc.set_attribute(input, "size", "5");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        width(&doc, input),
        94.0,
        "size=5 after layout: Chrome 153 94"
    );
    doc.set_style(input, "font-size", "20px");
    doc.resolve_layout(800.0, 600.0);
    // 5 × 13 + 66 − 13 = 118 (Chrome 153: 118).
    assert_eq!(width(&doc, input), 118.0, "size=5 at 20px: Chrome 153 118");

    let (mut doc, ta) = control("textarea", 16.0, &[], "");
    doc.set_attribute(ta, "cols", "5");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, ta), 67.0, "cols=5 after layout: Chrome 153 67");
    doc.remove_attribute(ta, "cols");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, ta), 220.0, "cols removed: the default again");
}

// ── Which average, from which face (review of #1196) ───────────────────────
//
// The faces below are the bundled Inter with its OS/2 `xAvgCharWidth`
// rewritten (the same edit, made with fontTools, was measured in Chrome 153
// through `@font-face`; its `0` stays 1292 units).

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Inter with its `xAvgCharWidth` set to `avg` font units (of 2048).
fn inter_with_avg(avg: u16) -> Vec<u8> {
    let mut f = FACE.to_vec();
    let n = be16(&f, 4) as usize;
    let os2 = (0..n)
        .map(|i| 12 + 16 * i)
        .find(|&rec| &f[rec..rec + 4] == b"OS/2")
        .map(|rec| be32(&f, rec + 8) as usize)
        .expect("Inter has an OS/2 table");
    f[os2 + 2..os2 + 4].copy_from_slice(&avg.to_be_bytes());
    f
}

fn register_as(
    doc: &mut RinchDocument,
    data: Vec<u8>,
    family: &str,
    weight: Option<f32>,
    italic: bool,
) {
    use parley::fontique::{Blob, FontInfoOverride, FontStyle, FontWeight};
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(data)),
        Some(FontInfoOverride {
            family_name: Some(family),
            weight: weight.map(FontWeight::new),
            style: italic.then_some(FontStyle::Italic),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1);
}

/// One control of `tag` in `css`, in a document holding whatever `setup`
/// registers.
fn w_in(
    setup: impl FnOnce(&mut RinchDocument),
    tag: &str,
    attrs: &[(&str, &str)],
    css: &str,
) -> f32 {
    let mut doc = document();
    setup(&mut doc);
    let body = doc.body();
    let div = el(&mut doc, body, "div", "width: 600px");
    let c = doc.create_element(tag);
    for (k, v) in attrs {
        doc.set_attribute(c, k, v);
    }
    doc.set_attribute(
        c,
        "style",
        &format!("{css}; line-height: 20px; padding: 0; border: 0"),
    );
    doc.append_child(div, c);
    doc.resolve_layout(800.0, 600.0);
    width(&doc, c)
}

/// Chrome does not trust an `xAvgCharWidth` more than 1.7 times the face's
/// `0` — the shape of a CJK face, whose average is its full-width ideograph
/// (Noto Sans CJK: 979 against a `0` of 555, and a 16px input 178px wide in
/// Chrome 153, which the OS/2 value would make 367). It then sizes the control
/// from the `0` advance alone: `ceil(zero × size)`, no extent term, and no
/// rounding of the average. Measured on this Inter at 2190 units (1.695 × its
/// `0`: trusted) and 2200 (1.703: not), and on Noto Sans CJK at 942 and 944 —
/// the same cut.
#[test]
fn an_average_wider_than_1_7_zeros_is_replaced_by_the_zero() {
    let wide = |d: &mut RinchDocument| register_as(d, inter_with_avg(2200), "Wide", None, false);
    // 20 × 10.09375 = 201.875; the textarea adds its 15.
    assert_eq!(w_in(wide, "input", &[], "font: 16px Wide"), 202.0);
    assert_eq!(
        w_in(wide, "input", &[("size", "5")], "font: 16px Wide"),
        51.0
    );
    assert_eq!(w_in(wide, "textarea", &[], "font: 16px Wide"), 217.0);
    assert_eq!(
        w_in(wide, "textarea", &[("cols", "5")], "font: 16px Wide"),
        66.0
    );
    // At 20px the `0` is 12.617: 253, where rounding it as the OS/2 average
    // is rounded would give 260. Chrome 153: 253, 64, 268.
    assert_eq!(w_in(wide, "input", &[], "font: 20px Wide"), 253.0);
    assert_eq!(
        w_in(wide, "input", &[("size", "5")], "font: 20px Wide"),
        64.0
    );
    assert_eq!(w_in(wide, "textarea", &[], "font: 20px Wide"), 268.0);

    // Just under the cut the OS/2 average stands. Chrome 153: 379 and 358.
    let edge = |d: &mut RinchDocument| register_as(d, inter_with_avg(2190), "Edge", None, false);
    assert_eq!(w_in(edge, "input", &[], "font: 16px Edge"), 379.0);
    assert_eq!(w_in(edge, "textarea", &[], "font: 16px Edge"), 358.0);
}

/// The face is the one the control's weight and style select. `ProbeFace`
/// gets a 700 face whose average is 2150 units and an italic one at 2100.
/// Chrome 153, the same three faces under one `@font-face` family: 248 / 220
/// regular, 376 / 355 bold, 365 / 344 italic.
#[test]
fn the_weight_and_style_pick_the_face_the_average_is_read_from() {
    let faces = |d: &mut RinchDocument| {
        register_as(d, inter_with_avg(2150), "ProbeFace", Some(700.0), false);
        register_as(d, inter_with_avg(2100), "ProbeFace", None, true);
    };
    assert_eq!(w_in(faces, "input", &[], "font: 16px ProbeFace"), 248.0);
    assert_eq!(
        w_in(faces, "input", &[], "font: bold 16px ProbeFace"),
        376.0
    );
    assert_eq!(
        w_in(faces, "textarea", &[], "font: bold 16px ProbeFace"),
        355.0
    );
    assert_eq!(
        w_in(faces, "input", &[], "font: italic 16px ProbeFace"),
        365.0
    );
    assert_eq!(
        w_in(faces, "textarea", &[], "font: italic 16px ProbeFace"),
        344.0
    );
}

/// A style change after layout finds the italic face again: the metrics cached
/// on the node are keyed on the style as well as the family and weight.
#[test]
fn a_style_change_after_layout_rereads_the_face() {
    let mut doc = document();
    register_as(&mut doc, inter_with_avg(2100), "ProbeFace", None, true);
    let body = doc.body();
    let div = el(&mut doc, body, "div", "width: 600px");
    let c = el(&mut doc, div, "input", &bare(16.0));
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, c), 248.0);
    doc.set_style(c, "font-style", "italic");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, c), 365.0, "Chrome 153: 365");
}

/// A family that is not installed measures the face the stack falls back to
/// for text, not the one fontique picks for a lone digit (a colour-emoji face,
/// on a host that has one, whose average is 1.25 em). So it is exactly as wide
/// as the fallback it renders in, named or not.
#[test]
fn a_missing_family_measures_the_face_its_text_falls_back_to() {
    let missing = w_in(|_| {}, "input", &[], "font: 16px NoSuchFamily1196");
    let fallback = w_in(|_| {}, "input", &[], "font: 16px sans-serif");
    assert!(missing > 0.0);
    assert_eq!(missing, fallback);
}

/// Chrome's width is capped at `LayoutUnit`'s maximum: `size="2147483647"` is
/// 33554432 in Chrome 153.
#[test]
fn a_huge_size_is_capped_where_chrome_caps_it() {
    assert_eq!(
        w("input", 16.0, &[("size", "2147483647")], ""),
        33_554_432.0
    );
}

/// A face registered after the control was sized is the face it is then sized
/// from (`RinchDocument::note_fonts_registered`, which `RinchApp::register_app_font`
/// calls for a live document): as wide as in a document that had the face
/// from the start.
#[test]
fn a_face_registered_after_layout_resizes_the_control() {
    let css = "font: 16px/20px LateFace, sans-serif; padding: 0; border: 0";
    let fresh = {
        let mut doc = RinchDocument::new();
        register_as(&mut doc, FACE.to_vec(), "LateFace", None, false);
        let body = doc.body();
        let c = el(&mut doc, body, "input", css);
        doc.resolve_layout(800.0, 600.0);
        width(&doc, c)
    };
    assert_eq!(fresh, 248.0);
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "input", css);
    doc.resolve_layout(800.0, 600.0);
    register_as(&mut doc, FACE.to_vec(), "LateFace", None, false);
    doc.note_fonts_registered();
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(width(&doc, c), fresh);
}
