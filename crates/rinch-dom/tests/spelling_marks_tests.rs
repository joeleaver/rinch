//! A text control's spelling marks (#2862): `RinchDocument::set_spelling_marks`
//! and the wave paint draws for them under the control's value.
//!
//! Relative facts only, as `wavy_underline_tests.rs` asserts them: which
//! pixels are the mark's colour, and where they sit against each other and
//! against the glyphs, never an absolute row or an ink count (those pin the
//! host's fonts).

#![cfg(feature = "software-renderer")]
// One mark is a list of one.
#![allow(clippy::single_range_in_vec_init)]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::{RinchDocument, SpellingMarks};

const VW: f32 = 400.0;
const VH: f32 = 120.0;
const VALUE: &str = "helo wrld cat";

/// A `tag` control holding [`VALUE`], with `style` added, laid out.
fn control(tag: &str, style: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = doc.create_element(tag);
    doc.set_attribute(
        c,
        "style",
        &format!(
            "display: block; width: 360px; height: 50px; font-size: 20px; \
             line-height: 40px; color: black; border: none; padding: 0; {style}"
        ),
    );
    doc.set_attribute(c, "value", VALUE);
    doc.append_child(body, c);
    doc.resolve_layout(VW, VH);
    (doc, c)
}

fn marks(ranges: &[std::ops::Range<usize>]) -> Option<SpellingMarks> {
    Some(SpellingMarks {
        text: VALUE.to_string(),
        ranges: ranges.to_vec(),
        show_when_blurred: false,
    })
}

fn pixels(doc: &mut RinchDocument) -> Vec<(usize, usize, [u8; 4])> {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut cx,
    );
    let px: Vec<[u8; 4]> = painter.pixels().as_chunks::<4>().0.to_vec();
    px.iter()
        .enumerate()
        .filter(|(_, p)| p[3] > 0 && !(p[0] > 240 && p[1] > 240 && p[2] > 240))
        .map(|(i, p)| (i % VW as usize, i / VW as usize, *p))
        .collect()
}

/// Pixels clearly of colour `hue` (0 red, 2 blue) rather than the black text.
fn tinted(doc: &mut RinchDocument, hue: usize) -> Vec<(usize, usize)> {
    pixels(doc)
        .into_iter()
        .filter(|(_, _, p)| {
            p[hue] > 100 && (0..3).all(|c| c == hue || p[hue] as i16 - p[c] as i16 > 60)
        })
        .map(|(x, y, _)| (x, y))
        .collect()
}

fn x_extent(px: &[(usize, usize)]) -> (usize, usize) {
    let min = px.iter().map(|p| p.0).min().unwrap();
    let max = px.iter().map(|p| p.0).max().unwrap();
    (min, max)
}

/// The dark (glyph) pixels' horizontal extent and lowest row.
fn glyphs(doc: &mut RinchDocument) -> (usize, usize, usize) {
    let dark: Vec<(usize, usize)> = pixels(doc)
        .into_iter()
        .filter(|(_, _, p)| p[0] < 90 && p[1] < 90 && p[2] < 90)
        .map(|(x, y, _)| (x, y))
        .collect();
    let (min, max) = x_extent(&dark);
    (min, max, dark.iter().map(|p| p.1).max().unwrap())
}

/// The marks of a `<textarea>` are red waves under exactly the words named:
/// "helo" (0..4) starts where the text starts, "wrld" (5..9) further right
/// and past the end of the first, "cat" is not marked, and each wave is below
/// the glyphs. Without the marks nothing red is painted at all.
#[test]
fn a_textareas_marks_are_red_waves_under_the_named_words() {
    let (mut doc, c) = control("textarea", "");
    assert!(tinted(&mut doc, 0).is_empty(), "red before any mark");
    let (text_left, text_right, glyph_bottom) = glyphs(&mut doc);

    doc.set_spelling_marks(c.0, marks(&[0..4]));
    let first = tinted(&mut doc, 0);
    assert!(!first.is_empty(), "a mark painted nothing");
    let (a0, a1) = x_extent(&first);
    assert!(
        a0.abs_diff(text_left) <= 1,
        "the first word's wave starts at {a0}, the text at {text_left}"
    );
    assert!(
        a1 < text_right / 2,
        "\"helo\" is the first third of the text: its wave ends at {a1} of {text_right}"
    );
    assert!(
        first.iter().all(|p| p.1 + 3 >= glyph_bottom),
        "the wave rides up into the glyphs (bottom glyph row {glyph_bottom})"
    );
    let rows: std::collections::BTreeSet<usize> = first.iter().map(|p| p.1).collect();
    assert!(
        rows.len() >= 2,
        "a straight line, not a wave: rows {rows:?}"
    );

    doc.set_spelling_marks(c.0, marks(&[5..9]));
    let (b0, b1) = x_extent(&tinted(&mut doc, 0));
    assert!(
        b0 > a1,
        "\"wrld\" starts at {b0}, before \"helo\" ended at {a1}"
    );
    assert!(b1 < text_right, "\"wrld\" reaches the last word");
}

/// Marks are of one value: once the control shows another, they are not
/// drawn (the checker has not caught up with it yet).
#[test]
fn marks_for_another_value_are_not_drawn() {
    let (mut doc, c) = control("textarea", "");
    doc.set_spelling_marks(
        c.0,
        Some(SpellingMarks {
            text: "helo wrld".into(),
            ranges: vec![0..4],
            show_when_blurred: false,
        }),
    );
    assert!(tinted(&mut doc, 0).is_empty());
}

/// A single-line `<input>` shows its marks only while it has the keyboard,
/// as Chrome 158 does; a textarea always shows them.
#[test]
fn an_inputs_marks_show_only_while_it_is_focused() {
    let (mut doc, c) = control("input", "");
    doc.set_spelling_marks(c.0, marks(&[0..4]));
    assert!(
        tinted(&mut doc, 0).is_empty(),
        "an unfocused input drew its marks"
    );
    doc.set_attribute(c, "data-focused", "true");
    assert!(!tinted(&mut doc, 0).is_empty(), "a focused input drew none");

    // The app's option: shown blurred too.
    let (mut doc, c) = control("input", "");
    let mut shown = marks(&[0..4]);
    shown.as_mut().unwrap().show_when_blurred = true;
    doc.set_spelling_marks(c.0, shown);
    assert!(
        !tinted(&mut doc, 0).is_empty(),
        "`show_when_blurred` drew nothing"
    );
}

/// The colour and the line are the app's, through the cascade:
/// `--rinch-spellcheck-color` on an ancestor reaches the control, and
/// `--rinch-spellcheck-style: none` draws nothing.
#[test]
fn the_marks_colour_and_style_come_from_custom_properties() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let form = doc.create_element("div");
    doc.set_attribute(form, "style", "--rinch-spellcheck-color: rgb(0, 0, 255)");
    doc.append_child(body, form);
    let c = doc.create_element("textarea");
    doc.set_attribute(
        c,
        "style",
        "display: block; width: 360px; height: 50px; font-size: 20px; line-height: 40px; \
         color: black; border: none; padding: 0",
    );
    doc.set_attribute(c, "value", VALUE);
    doc.append_child(form, c);
    doc.resolve_layout(VW, VH);
    doc.set_spelling_marks(c.0, marks(&[0..4]));
    assert!(
        tinted(&mut doc, 0).is_empty(),
        "still red under a blue override"
    );
    assert!(
        !tinted(&mut doc, 2).is_empty(),
        "the blue override drew nothing"
    );

    let (mut doc, c) = control("textarea", "--rinch-spellcheck-style: none");
    doc.set_spelling_marks(c.0, marks(&[0..4]));
    assert!(tinted(&mut doc, 0).is_empty(), "`none` still drew a mark");
}

/// A write that changes nothing is no write: no repaint, no count.
#[test]
fn an_unchanged_write_does_nothing() {
    let (mut doc, c) = control("textarea", "");
    let _ = doc.tree.perf.end_frame();
    doc.set_spelling_marks(c.0, marks(&[0..4]));
    doc.set_spelling_marks(c.0, marks(&[0..4]));
    let s = doc.tree.perf.end_frame();
    assert_eq!(s.get(rinch_dom::perf::Counter::SpellingMarkWrites), 1);
    assert_eq!(doc.spelling_marks(c.0), marks(&[0..4]).as_ref());
}
