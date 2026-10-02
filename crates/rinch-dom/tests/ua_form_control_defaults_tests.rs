//! Issue #1194 — the browser's UA defaults for `<input>` and `<textarea>`.
//!
//! rinch's UA sheet gave both controls `display: inline-block` and nothing
//! else, so a raw control had no padding, no border, its parent's font and —
//! for a textarea — `overflow: visible`. `rinch-web` runs on the browser's own
//! UA sheet and had all of them: the desktop/web divergence class #627 and
//! #674 closed for headings and blocks.
//!
//! **Every expected value is measured in Chrome 153** (`getComputedStyle`, a
//! standards-mode page) under a body declaring `font: italic bold 20px/40px
//! serif; letter-spacing: 5px; word-spacing: 7px; text-transform: uppercase;
//! text-align: right; white-space: pre; overflow-wrap: anywhere`, so every
//! value below that differs from the parent's is one the control does **not**
//! inherit:
//!
//! | | text-state `input` | `textarea` |
//! |---|---|---|
//! | padding | `1px 2px` | `2px` |
//! | border | `2px inset rgb(118,118,118)` | `1px solid rgb(118,118,118)` |
//! | font | `normal 400 13.3333px/normal Arial` | `normal 400 13.3333px/normal monospace` |
//! | letter-/word-spacing, text-transform, text-align | `normal`, `0`, `none`, `start` | same |
//! | white-space | inherited (`pre`) | `pre-wrap` |
//! | overflow-wrap | inherited (`anywhere`) | `break-word` |
//! | overflow | `clip`, whatever the author says | `auto`; an author `visible` computes `auto` |
//!
//! Per type: `checkbox`, `radio`, `range`, `file`, `image`, `hidden` have no
//! padding and no border; the date/time family has `0 0 0 1px` padding (inline start only), the
//! `2px inset` border and **monospace**; `checkbox`, `radio` and `range` are
//! not forced to `clip` (an author `overflow: hidden` on a checkbox computes
//! `hidden`). Every type has the 13.3333px font.
//!
//! ## The fixed points this file samples off
//!
//! - A control under a 16px parent would read 16 whether the rule says
//!   `13.3333px` or nothing at all only if the rule were missing *and* the
//!   parent were 13.3333 — so every parent here is 20px, bold and italic.
//! - `overflow: auto` on a textarea is indistinguishable from Chrome's
//!   adjuster (`visible` → `auto`) until the author declares `visible`;
//!   `an_author_visible_on_a_textarea_computes_auto` declares it.
//! - `overflow: clip` on an input is indistinguishable from `clip
//!   !important` until the author declares another value.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::ComputedStyle;
use rinch_dom::computed_style::values::{
    BorderStyleValue, FontStyleValue, LengthPercentageValue, LineHeightValue, OverflowValue,
    OverflowWrapValue, TextAlignValue, TextTransformValue, WhiteSpaceValue,
};

const VW: f32 = 800.0;
const VH: f32 = 600.0;

/// A parent that disagrees with every value the controls reset.
const PARENT: &str = "width: 800px; font: italic bold 20px/40px serif; letter-spacing: 5px; \
     word-spacing: 7px; text-transform: uppercase; text-align: right; white-space: pre; \
     overflow-wrap: anywhere";

const SMALL_CONTROL: f32 = 13.333_333;
const GREY: [u8; 3] = [118, 118, 118];

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, attrs: &[(&str, &str)]) -> NodeId {
    let id = doc.create_element(tag);
    for (k, v) in attrs {
        doc.set_attribute(id, k, v);
    }
    doc.append_child(parent, id);
    id
}

fn style(doc: &RinchDocument, id: NodeId) -> &ComputedStyle {
    &doc.tree.get(id.0).unwrap().computed_style
}

fn px(v: &LengthPercentageValue) -> f32 {
    match v {
        LengthPercentageValue::Length(v) => *v,
        other => panic!("expected a length, got {other:?}"),
    }
}

/// `(top, right, bottom, left)` padding.
fn padding(s: &ComputedStyle) -> (f32, f32, f32, f32) {
    (
        px(&s.padding_top),
        px(&s.padding_right),
        px(&s.padding_bottom),
        px(&s.padding_left),
    )
}

/// `(top, right, bottom, left)` border widths.
fn border(s: &ComputedStyle) -> (f32, f32, f32, f32) {
    (
        px(&s.border_top_width),
        px(&s.border_right_width),
        px(&s.border_bottom_width),
        px(&s.border_left_width),
    )
}

fn rgb(c: Option<peniko::Color>) -> [u8; 3] {
    let c = c.expect("border colour").to_rgba8();
    [c.r, c.g, c.b]
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.001
}

fn setup() -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = doc.create_element("div");
    doc.set_attribute(c, "style", PARENT);
    doc.append_child(body, c);
    (doc, c)
}

/// The font reset both controls share, and the text properties that go with it.
fn assert_control_font(s: &ComputedStyle, what: &str, family: &str) {
    assert!(
        close(s.font_size, SMALL_CONTROL),
        "{what}: font-size {} (expected Chrome's 13.3333px)",
        s.font_size
    );
    assert!(
        s.font_family.contains(family),
        "{what}: font-family {:?} (expected {family})",
        s.font_family
    );
    assert_eq!(s.font_weight, 400.0, "{what}: font-weight");
    assert_eq!(s.font_style, FontStyleValue::Normal, "{what}: font-style");
    assert!(
        matches!(s.line_height, LineHeightValue::Normal),
        "{what}: line-height {:?}",
        s.line_height
    );
    assert_eq!(s.letter_spacing, 0.0, "{what}: letter-spacing");
    assert_eq!(s.word_spacing, 0.0, "{what}: word-spacing");
    assert_eq!(
        s.text_transform,
        TextTransformValue::None,
        "{what}: text-transform"
    );
    assert_eq!(s.text_align, TextAlignValue::Start, "{what}: text-align");
}

#[test]
fn a_raw_text_input_carries_chromes_box_and_font() {
    let (mut doc, c) = setup();
    let plain = el(&mut doc, c, "input", &[]);
    let typed = [
        "text", "TEXT", "search", "email", "password", "number", "foo",
    ];
    let typed: Vec<NodeId> = typed
        .iter()
        .map(|t| el(&mut doc, c, "input", &[("type", t)]))
        .collect();
    doc.resolve_layout(VW, VH);

    for id in std::iter::once(plain).chain(typed) {
        let s = style(&doc, id);
        let what = format!(
            "input {:?}",
            doc.tree.get(id.0).unwrap().attributes.get("type")
        );
        assert_eq!(padding(s), (1.0, 2.0, 1.0, 2.0), "{what}: padding");
        assert_eq!(border(s), (2.0, 2.0, 2.0, 2.0), "{what}: border width");
        assert_ne!(
            s.border_top_style,
            BorderStyleValue::None,
            "{what}: border style"
        );
        assert_eq!(rgb(s.border_top_color), GREY, "{what}: border colour");
        assert_control_font(s, &what, "Arial");
        // Not reset on an input: these two still inherit.
        assert_eq!(s.white_space, WhiteSpaceValue::Pre, "{what}: white-space");
        assert_eq!(
            s.overflow_wrap,
            OverflowWrapValue::Anywhere,
            "{what}: overflow-wrap"
        );
        assert_eq!(
            (s.overflow_x, s.overflow_y),
            (OverflowValue::Clip, OverflowValue::Clip),
            "{what}: overflow"
        );
    }
}

#[test]
fn a_raw_textarea_carries_chromes_box_font_and_scrolling() {
    let (mut doc, c) = setup();
    let t = el(&mut doc, c, "textarea", &[]);
    doc.resolve_layout(VW, VH);
    let s = style(&doc, t);
    assert_eq!(padding(s), (2.0, 2.0, 2.0, 2.0), "textarea padding");
    assert_eq!(border(s), (1.0, 1.0, 1.0, 1.0), "textarea border width");
    assert_eq!(s.border_top_style, BorderStyleValue::Solid);
    assert_eq!(rgb(s.border_top_color), GREY);
    assert_control_font(s, "textarea", "monospace");
    assert_eq!(s.white_space, WhiteSpaceValue::PreWrap);
    assert_eq!(s.overflow_wrap, OverflowWrapValue::BreakWord);
    assert_eq!(
        (s.overflow_x, s.overflow_y),
        (OverflowValue::Auto, OverflowValue::Auto)
    );
}

/// Chrome adjusts a textarea's `visible` to `auto` after the cascade (it is a
/// scroll container whatever the author says), while an author `hidden` on one
/// axis is kept. A UA `overflow: auto` alone would compute `visible` here.
#[test]
fn an_author_visible_on_a_textarea_computes_auto() {
    let (mut doc, c) = setup();
    let all = el(&mut doc, c, "textarea", &[("style", "overflow: visible")]);
    let mixed = el(
        &mut doc,
        c,
        "textarea",
        &[("style", "overflow-x: visible; overflow-y: hidden")],
    );
    doc.resolve_layout(VW, VH);
    let s = style(&doc, all);
    assert_eq!(
        (s.overflow_x, s.overflow_y),
        (OverflowValue::Auto, OverflowValue::Auto)
    );
    let s = style(&doc, mixed);
    assert_eq!(
        (s.overflow_x, s.overflow_y),
        (OverflowValue::Auto, OverflowValue::Hidden)
    );
}

/// An input's `clip` is Chrome's `!important` UA rule: an author `visible` or
/// `auto` does not move it. A checkbox is outside that rule.
#[test]
fn an_input_clips_whatever_the_author_says_but_a_checkbox_does_not() {
    let (mut doc, c) = setup();
    let vis = el(&mut doc, c, "input", &[("style", "overflow: visible")]);
    let auto = el(&mut doc, c, "input", &[("style", "overflow: auto")]);
    let cb = el(
        &mut doc,
        c,
        "input",
        &[("type", "checkbox"), ("style", "overflow: hidden")],
    );
    let cb_plain = el(&mut doc, c, "input", &[("type", "checkbox")]);
    doc.resolve_layout(VW, VH);
    for id in [vis, auto] {
        let s = style(&doc, id);
        assert_eq!(
            (s.overflow_x, s.overflow_y),
            (OverflowValue::Clip, OverflowValue::Clip)
        );
    }
    assert_eq!(style(&doc, cb).overflow_y, OverflowValue::Hidden);
    assert_eq!(style(&doc, cb_plain).overflow_y, OverflowValue::Visible);
}

#[test]
fn non_text_input_types_get_their_own_box() {
    let (mut doc, c) = setup();
    let bare = ["checkbox", "radio", "range", "file", "image", "hidden"];
    let bare: Vec<NodeId> = bare
        .iter()
        .map(|t| el(&mut doc, c, "input", &[("type", t)]))
        .collect();
    let dates = ["date", "month", "week", "time", "datetime-local"];
    let dates: Vec<NodeId> = dates
        .iter()
        .map(|t| el(&mut doc, c, "input", &[("type", t)]))
        .collect();
    doc.resolve_layout(VW, VH);
    for id in bare {
        let s = style(&doc, id);
        let what = format!("{:?}", doc.tree.get(id.0).unwrap().attributes.get("type"));
        assert_eq!(padding(s), (0.0, 0.0, 0.0, 0.0), "{what}: padding");
        assert_eq!(border(s), (0.0, 0.0, 0.0, 0.0), "{what}: border");
        assert!(close(s.font_size, SMALL_CONTROL), "{what}: font-size");
    }
    for id in dates {
        let s = style(&doc, id);
        let what = format!("{:?}", doc.tree.get(id.0).unwrap().attributes.get("type"));
        // Chrome 153: `padding: 0px 0px 0px 1px` -- the inline start only.
        assert_eq!(padding(s), (0.0, 0.0, 0.0, 1.0), "{what}: padding");
        assert_eq!(border(s), (2.0, 2.0, 2.0, 2.0), "{what}: border");
        assert_control_font(s, &what, "monospace");
    }
}

/// Cascade rules, not a patch: an author declaration wins, including the
/// rinch theme's own `font-family: inherit` (which keeps the 13.3333px size,
/// as Chrome does under the same sheet).
#[test]
fn author_declarations_beat_the_new_ua_rules() {
    let (mut doc, c) = setup();
    doc.load_css("input, textarea { font-family: inherit; }");
    let i = el(
        &mut doc,
        c,
        "input",
        &[(
            "style",
            "padding: 0; border: 0; font-size: 18px; letter-spacing: 3px",
        )],
    );
    let t = el(
        &mut doc,
        c,
        "textarea",
        &[(
            "style",
            "padding: 5px; border-width: 4px; white-space: nowrap",
        )],
    );
    doc.resolve_layout(VW, VH);
    let s = style(&doc, i);
    assert_eq!(padding(s), (0.0, 0.0, 0.0, 0.0));
    assert_eq!(border(s), (0.0, 0.0, 0.0, 0.0));
    assert!(close(s.font_size, 18.0));
    assert!(close(s.letter_spacing, 3.0));
    assert!(s.font_family.contains("serif"), "{:?}", s.font_family);
    let s = style(&doc, t);
    assert_eq!(padding(s), (5.0, 5.0, 5.0, 5.0));
    assert_eq!(border(s), (4.0, 4.0, 4.0, 4.0));
    assert_eq!(s.white_space, WhiteSpaceValue::NoWrap);
    assert!(close(s.font_size, SMALL_CONTROL));
    assert!(s.font_family.contains("serif"), "{:?}", s.font_family);
}

/// The layout consequence the issue measured: a textarea is a scroll
/// container, so its automatic minimum size is 0 and it shrinks to a 100px
/// flex row (Chrome: 100px). Before, it kept its full intrinsic width.
#[test]
fn a_raw_textarea_shrinks_in_a_narrow_flex_row() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let row = doc.create_element("div");
    doc.set_attribute(row, "style", "display: flex; width: 100px");
    doc.append_child(body, row);
    let t = el(&mut doc, row, "textarea", &[]);
    doc.resolve_layout(VW, VH);
    let w = doc.tree.get(t.0).unwrap().layout.width;
    assert!(close(w, 100.0), "textarea border-box width {w}, Chrome 100");
}

/// Review of #1265: `range` is outside the `clip !important` rule like the
/// checkbox (Chrome 153: an author `overflow: auto` on a range computes
/// `auto`), and a textarea's author `clip` is kept (Chrome: `clip clip`) —
/// only `visible` is adjusted.
#[test]
fn a_range_is_not_clipped_and_a_textarea_keeps_an_author_clip() {
    let (mut doc, c) = setup();
    let range = el(
        &mut doc,
        c,
        "input",
        &[("type", "range"), ("style", "overflow: auto")],
    );
    let ta = el(&mut doc, c, "textarea", &[("style", "overflow: clip")]);
    doc.resolve_layout(VW, VH);
    assert_eq!(style(&doc, range).overflow_y, OverflowValue::Auto);
    let s = style(&doc, ta);
    assert_eq!(
        (s.overflow_x, s.overflow_y),
        (OverflowValue::Clip, OverflowValue::Clip)
    );
}
