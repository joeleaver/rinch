//! parley #528 made [`parley::layout::Glyph::y`] Y-**down**, and the compiler
//! cannot see it.
//!
//! The struct is byte-identical either side of the change; only the value
//! parley fills it with is negated (`layout/data.rs`, "Convert from font space
//! (Y-up) to layout space (Y-down)"). rinch's two glyph consumers therefore had
//! to stop subtracting it and start adding it —
//! `crates/rinch-dom/src/paint/text.rs`, the main pass and the shadow pass.
//!
//! Nothing in the suite noticed. `y_offset` is **0** for ordinary Latin
//! shaping, so every Latin fixture and every English screenshot agrees whether
//! the sign is right or wrong — the fixed-point trap this repo keeps meeting.
//! Measured on the un-flipped tree at the moment of the upgrade: the whole
//! workspace was green with both signs wrong.
//!
//! The value is non-zero for **mark positioning** — combining diacritics,
//! vowelled Arabic, and any font using GPOS vertical adjustment. Two stacked
//! Latin combining marks are the portable shape: `a` + U+0301 + U+0308 shapes
//! to two glyphs, a precomposed `á` at `y == 0` and a combining diaeresis at a
//! non-zero `y`. Flipping the sign moves that second mark by `2 * |y|`, from
//! `|y|` above the baseline-relative position to `|y|` below it — far enough
//! that it lands *under* the acute it is supposed to stack on top of.
//!
//! `|y|` is **per face**, which is why nothing here asserts it. Measured at a
//! 32px font size: **6.875 on the bundled Inter** the fixture registers (a
//! 13.75px displacement), and 7.36 on this host's default `sans-serif`. An
//! earlier revision of this paragraph printed the second number and credited it
//! to Inter — the two were transposed. Nothing downstream depended on it,
//! because every assertion below is a comparison between two strings rendered
//! on the same face, so either number satisfies them.
//!
//! # What is asserted, and why it is a comparison
//!
//! The oracle is the **topmost inked row** of a `TinySkiaPainter` pixmap, and
//! the assertion is always a difference between two strings rendered the same
//! way, never an absolute row: an absolute row is a pin on the local font set
//! (CI has disagreed with this machine by 3px on a text-derived literal
//! before), a difference is the behaviour.
//!
//! With the correct sign the two-mark string inks **7 rows higher** than the
//! one-mark string on the bundled face. With the un-flipped sign the difference
//! is **0** — the pushed-down diaeresis no longer reaches above the acute, so
//! the topmost ink is the acute's in both. The assertion is a `>= 3` lower
//! bound, which sits between those two and is not a glyph measurement.
//!
//! Note the 7 rows is *not* the full 13.75px displacement: it is how far the
//! second mark's ink pokes above the first mark's, since once the mark is
//! pushed down the acute becomes the topmost ink and caps the difference.
//!
//! Both consumers get their own fixture. Measured fail-first, one site at a
//! time: reverting only the main pass leaves the shadow fixture green and vice
//! versa, so neither test stands in for the other.
//!
//! The face is registered under an override family name from
//! `assets/fonts/Inter-Regular.ttf`, so the fixture does not depend on the host
//! having any particular system font — the same discipline `fonts.rs`'s
//! `claim_generic_families` tests use.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 300.0;
const VH: f32 = 160.0;

/// Declared, so the line box and the glyph size are statements rather than
/// font measurements.
const BOX: &str = "width: 260px; margin-top: 60px; font-size: 32px; line-height: 40px; \
                   color: rgb(0, 0, 0); font-family: ProbeFace";

/// The bundled face, registered under an override name so the host's own fonts
/// cannot answer instead.
const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// `a` on its own — no combining mark, so no glyph carries a non-zero `y`.
const PLAIN: &str = "a";
/// `a` + U+0301: one mark, which this face shapes into a precomposed `á` at
/// `y == 0`. The sign cannot move it.
const ONE_MARK: &str = "a\u{0301}";
/// `a` + U+0301 + U+0308: the second mark stays a separate glyph, positioned by
/// GPOS at a non-zero `y`. This is the only string here the sign moves.
const TWO_MARKS: &str = "a\u{0301}\u{0308}";

/// The gap the flip is worth on the bundled face is 7 rows; the un-flipped sign
/// gives 0. Asserting a lower bound between them keeps the test off any glyph
/// measurement.
const MIN_GAP: usize = 3;

fn document(text: &str, extra: &str) -> RinchDocument {
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

    let body = doc.body();
    let c = doc.create_element("div");
    doc.set_attribute(c, "style", &format!("{BOX}; {extra}"));
    doc.append_child(body, c);
    let t = doc.create_text(text);
    doc.append_child(c, t);
    doc.resolve_layout(VW, VH);
    doc
}

/// The first pixel row carrying anything that is not the white page.
fn top_inked_row(text: &str, extra: &str) -> usize {
    let mut doc = document(text, extra);
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
    (0..VH as usize)
        .find(|&y| {
            let row = y * VW as usize;
            px[row..row + VW as usize]
                .iter()
                .any(|p| p[3] > 0 && !(p[0] > 240 && p[1] > 240 && p[2] > 240))
        })
        .unwrap_or_else(|| panic!("nothing was painted for {text:?}; the fixture measures nothing"))
}

/// The three strings, top-inked-row each, with the two controls checked.
///
/// `extra` is appended to the container's style, which is how the shadow
/// fixture reuses this without a second copy of the geometry.
fn stacked_mark_gap(extra: &str) -> usize {
    let plain = top_inked_row(PLAIN, extra);
    let one = top_inked_row(ONE_MARK, extra);
    let two = top_inked_row(TWO_MARKS, extra);

    // Positive control: the acute inks above the bare `a`. Without this the
    // assertion below could pass on a page where no mark renders at all, or
    // where the whole run failed to shape and every string collapsed to the
    // same row.
    assert!(
        plain > one + MIN_GAP,
        "the accent on {ONE_MARK:?} did not ink above bare {PLAIN:?} \
         (plain={plain}, one mark={one}); this fixture is measuring nothing"
    );

    assert!(
        one >= two,
        "adding a second stacked mark moved the topmost ink DOWN \
         (one mark={one}, two marks={two})"
    );
    one - two
}

/// The main glyph pass: `paint/text.rs`'s `render_text`.
///
/// Fails against `let py = gy - glyph.y * sf;` — measured, gap 0 instead of 7.
#[test]
fn a_second_stacked_mark_inks_above_the_first_in_the_main_pass() {
    let gap = stacked_mark_gap("");
    assert!(
        gap >= MIN_GAP,
        "the second stacked combining mark did not stack: the two-mark string's \
         topmost ink is only {gap} rows above the one-mark string's. \
         `parley::Glyph::y` is Y-DOWN since parley #528, so \
         `crates/rinch-dom/src/paint/text.rs`'s main pass must ADD it to the \
         baseline, not subtract it."
    );
}

/// The shadow pass: `paint/text.rs`'s `render_text_shadow_pass`, a second copy
/// of the same arithmetic that the main fixture does not reach.
///
/// The shadow is offset 40px **up**, clear of the glyphs themselves, so the
/// topmost ink on the page belongs to the shadow pass and to nothing else.
///
/// Fails against `let py = gy - glyph.y;` — measured, gap 0 instead of 7, with
/// the main pass left correct.
#[test]
fn a_second_stacked_mark_inks_above_the_first_in_the_shadow_pass() {
    let gap = stacked_mark_gap("text-shadow: 0 -40px 0 rgb(0, 0, 0)");
    assert!(
        gap >= MIN_GAP,
        "the second stacked combining mark did not stack in the text-shadow \
         pass: only {gap} rows. `crates/rinch-dom/src/paint/text.rs`'s \
         `render_text_shadow_pass` must ADD `parley::Glyph::y`, not subtract it."
    );
}
