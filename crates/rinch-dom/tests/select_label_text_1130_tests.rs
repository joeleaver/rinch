//! What a closed `<select>`'s label is, and how it is set (#1130).
//!
//! An option's label is its text content with ASCII whitespace stripped and
//! collapsed (HTML's `option.text`), or its `label` attribute **verbatim**
//! when that is not empty. The closed control sets it in the select's own
//! `letter-spacing`, `word-spacing` and `text-transform` (an option's own
//! style does not reach it), and sizes itself from the label as set — the
//! #320 rule: measure and paint are one builder.
//!
//! Measured in Chrome 155 on Linux, with this crate's bundled Inter loaded
//! through `@font-face` under the override name the fixtures register it
//! with, every select `font: 16px/20px ProbeFace; display: block`:
//!
//! | option(s) | select CSS | Chrome 155 width |
//! |---|---|---|
//! | `"  a   b  "` | — | 46 (`"a b"` is 46) |
//! | `"a\t\nb"` | — | 46 |
//! | `"a\u{a0}\u{a0}b"` | — | 50 (NBSP is not ASCII whitespace) |
//! | `abc` | — | 50 |
//! | `abc` | `letter-spacing: 5px` | 65 |
//! | `abc` | `letter-spacing: -2px` | 44 |
//! | `abc` | `letter-spacing: 0.5em` | 74 |
//! | `abc` | `text-transform: uppercase` | 56 |
//! | `"  hello   world "` | `text-transform: capitalize` | 110 |
//! | `a b c` | — | 59 |
//! | `a b c` | `word-spacing: 10px` | 79 |
//! | `a b c` | `word-spacing: 50%` | 75 |
//! | `a b` | all three (5px, uppercase, 10px) | 74 |
//! | `abc`, option `letter-spacing: 5px` | — | 50 |
//! | `label="  x   y "` | — | 67 (`label="x y"` is 45) |
//! | `label=" x"` | — | 36 (`label="x"` is 31) |
//! | `label="   "` | — | 36 |
//! | `label=""`, text `zz` | — | 40 (text `zz` alone is 40) |
//! | `label="x\ny"`, `label="x\ty"` | — | 45 (as `"x y"`) |
//! | `label="x\ry"`, `label="x\u{c}y"` | — | 40 (as `"xy"`) |
//!
//! Before #1130 the text was trimmed but not collapsed (`"  a   b  "` was 55
//! wide), the `label` attribute was trimmed, and none of the three properties
//! reached either the size or the paint.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn document() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    doc
}

/// One option: its text, and its `label` attribute if any, and its own style.
struct Opt<'a> {
    text: &'a str,
    label: Option<&'a str>,
    style: &'a str,
}

fn text(t: &str) -> Opt<'_> {
    Opt {
        text: t,
        label: None,
        style: "",
    }
}

fn labelled<'a>(label: &'a str, t: &'a str) -> Opt<'a> {
    Opt {
        text: t,
        label: Some(label),
        style: "",
    }
}

fn build(select_css: &str, options: &[Opt]) -> (RinchDocument, u64) {
    let mut doc = document();
    let body = doc.body();
    let sel = doc.create_element("select");
    doc.set_attribute(
        sel,
        "style",
        &format!("font: 16px/20px ProbeFace; display: block; {select_css}"),
    );
    doc.append_child(body, sel);
    for o in options {
        let opt = doc.create_element("option");
        if let Some(l) = o.label {
            doc.set_attribute(opt, "label", l);
        }
        if !o.style.is_empty() {
            doc.set_attribute(opt, "style", o.style);
        }
        doc.append_child(sel, opt);
        let t = doc.create_text(o.text);
        doc.append_child(opt, t);
    }
    (doc, sel.0)
}

fn width(select_css: &str, options: &[Opt]) -> f32 {
    let (mut doc, sel) = build(select_css, options);
    doc.resolve_layout(800.0, 600.0);
    doc.tree.get(sel).unwrap().layout.width
}

fn check(case: &str, select_css: &str, options: &[Opt], chrome: f32) {
    let got = width(select_css, options);
    assert_eq!(
        got, chrome,
        "{case}: width {got} where Chrome 155 gives {chrome}"
    );
}

/// Kills: the text label trimmed but not collapsed (55), and a collapse that
/// takes only spaces (tab and newline left in).
#[test]
fn an_options_text_is_stripped_and_collapsed() {
    check("'  a   b  '", "", &[text("  a   b  ")], 46.0);
    check("'a\\t\\nb'", "", &[text("a\t\nb")], 46.0);
    // Off the collapse: NBSP is no ASCII whitespace, so both stay.
    check("'a nbsp nbsp b'", "", &[text("a\u{a0}\u{a0}b")], 50.0);
}

/// Kills: `letter-spacing` not pushed into the label builder.
#[test]
fn the_selects_letter_spacing_sets_the_label() {
    check("no spacing", "", &[text("abc")], 50.0);
    check("5px", "letter-spacing: 5px", &[text("abc")], 65.0);
    check("-2px", "letter-spacing: -2px", &[text("abc")], 44.0);
    check("0.5em", "letter-spacing: 0.5em", &[text("abc")], 74.0);
}

/// Kills: `word-spacing` not pushed into the label builder.
#[test]
fn the_selects_word_spacing_sets_the_label() {
    check("no spacing", "", &[text("a b c")], 59.0);
    check("10px", "word-spacing: 10px", &[text("a b c")], 79.0);
    check("50%", "word-spacing: 50%", &[text("a b c")], 75.0);
}

/// Kills: `text-transform` not applied to the label; and (capitalize after a
/// collapse) a transform applied before the collapse would still give 110, so
/// the uppercase case is what pins the transform itself.
#[test]
fn the_selects_text_transform_sets_the_label() {
    check(
        "uppercase",
        "text-transform: uppercase",
        &[text("abc")],
        56.0,
    );
    check(
        "capitalize",
        "text-transform: capitalize",
        &[text("  hello   world ")],
        110.0,
    );
    check(
        "all three",
        "letter-spacing: 5px; text-transform: uppercase; word-spacing: 10px",
        &[text("a b")],
        74.0,
    );
}

/// An option's own spacing is not the label's: Chrome sets the closed label in
/// the select's style.
#[test]
fn an_options_own_letter_spacing_does_not_reach_the_label() {
    check(
        "option letter-spacing",
        "",
        &[Opt {
            text: "abc",
            label: None,
            style: "letter-spacing: 5px",
        }],
        50.0,
    );
}

/// Kills: the `label` attribute trimmed (67 → 45, 36 → 31), an empty one not
/// falling back to the text, and its tab / newline / CR / FF not mapped as
/// Chrome draws them (a space, a space, nothing, nothing).
#[test]
fn a_label_attribute_is_used_verbatim() {
    check("'  x   y '", "", &[labelled("  x   y ", "zz")], 67.0);
    check("'x y'", "", &[labelled("x y", "zz")], 45.0);
    check("' x'", "", &[labelled(" x", "zz")], 36.0);
    check("'   '", "", &[labelled("   ", "zz")], 36.0);
    check("''", "", &[labelled("", "zz")], 40.0);
    check("'x\\ny'", "", &[labelled("x\ny", "zz")], 45.0);
    check("'x\\ty'", "", &[labelled("x\ty", "zz")], 45.0);
    check("'x\\ry'", "", &[labelled("x\ry", "zz")], 40.0);
    check("'x\\u{c}y'", "", &[labelled("x\u{c}y", "zz")], 40.0);
}

/// A restyle that changes only the select's `letter-spacing` re-sizes it.
/// Kills: the cached label width keyed without the spacing (the select keeps
/// its old width).
#[test]
fn a_letter_spacing_change_resizes_the_select() {
    let (mut doc, sel) = build("", &[text("abc")]);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(doc.tree.get(sel).unwrap().layout.width, 50.0);
    let id = rinch_core::dom::NodeId(sel);
    doc.set_style(id, "letter-spacing", "5px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(doc.tree.get(sel).unwrap().layout.width, 65.0);
    doc.set_style(id, "text-transform", "uppercase");
    doc.resolve_layout(800.0, 600.0);
    // "ABC" at 5px: 56 + 15.
    assert_eq!(doc.tree.get(sel).unwrap().layout.width, 71.0);
}

/// Paint `select(style) > option > label` at `scale` and return the rightmost
/// column, in device px, holding label ink (left of the arrow box).
fn rightmost_label_ink(style: &str, label: &str, scale: f32) -> u32 {
    const VW: u32 = 800;
    const VH: u32 = 120;
    let (mut doc, sel) = build(
        &format!("position: absolute; left: 10px; top: 10px; color: rgb(0, 0, 0); {style}"),
        &[text(label)],
    );
    doc.resolve_layout(VW as f32 / scale, VH as f32 / scale);
    let l = doc.tree.get(sel).unwrap().layout;
    let mut painter = TinySkiaPainter::new(VW, VH);
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale as f64,
        (VW as f32, VH as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    let px = painter.pixels().as_chunks::<4>().0.to_vec();
    let s = |v: f32| (v * scale).round() as u32;
    let arrow_l = s(10.0 + l.width - 1.0 - 16.0);
    let (top, bottom) = (s(10.0 + 1.0), s(10.0 + l.height - 1.0));
    (0..arrow_l)
        .rev()
        .find(|&x| {
            (top..bottom).any(|y| {
                let p = px[(y * VW + x) as usize];
                p[3] > 128 && p[0] < 128
            })
        })
        .expect("label ink")
}

/// The painted label is the measured one: with `letter-spacing: 30px` the
/// last letter of `ii` is drawn 30 CSS px further right than without, at
/// device scale 1 and 2 alike.
/// Kills: paint not pushing the spacing (no shift), and paint pushing it in
/// CSS px under a physical font size (a 30 device-px shift at scale 2).
#[test]
fn the_painted_label_carries_the_spacing_at_every_scale() {
    for scale in [1.0f32, 2.0] {
        let plain = rightmost_label_ink("", "ii", scale) as f32;
        let spaced = rightmost_label_ink("letter-spacing: 30px", "ii", scale) as f32;
        let shift = (spaced - plain) / scale;
        assert!(
            (shift - 30.0).abs() <= 1.0,
            "scale {scale}: the second letter moved {shift} CSS px, not 30"
        );
        // And `word-spacing`, on the space of `i i`. Kills: word-spacing
        // pushed unscaled under a device-px font size (review of #1744).
        let plain = rightmost_label_ink("", "i i", scale) as f32;
        let spaced = rightmost_label_ink("word-spacing: 30px", "i i", scale) as f32;
        let shift = (spaced - plain) / scale;
        assert!(
            (shift - 30.0).abs() <= 1.0,
            "scale {scale}: word-spacing moved the last letter {shift} CSS px, not 30"
        );
    }
}

/// An option with a `label` attribute and no `value` reports its **text** as
/// its value, stripped and collapsed — not the label (Chrome 155:
/// `<option label="L">  t   x </option>` has `value` `"t x"`, `label` `"L"`).
/// Kills: the value fallback taking the label (review of #1744).
#[test]
fn a_labelled_options_value_is_its_text() {
    let (doc, sel) = build("", &[labelled("L", "  t   x ")]);
    let m = rinch_dom::select::resolve_select_model(&doc.tree, sel);
    assert_eq!(m.options[0].label, "L");
    assert_eq!(m.options[0].value, "t x");
}

/// The painted label is the transformed one: uppercase `abc` is wider ink
/// than lowercase.
/// Kills: paint not applying `text-transform` (measure does).
#[test]
fn the_painted_label_is_transformed() {
    let lower = rightmost_label_ink("", "abc", 1.0);
    let upper = rightmost_label_ink("text-transform: uppercase", "abc", 1.0);
    assert!(
        upper >= lower + 4,
        "uppercase ink ends at {upper}, lowercase at {lower}"
    );
}
