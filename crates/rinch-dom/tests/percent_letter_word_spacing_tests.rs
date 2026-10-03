//! #743 — `letter-spacing: <percentage>` and `word-spacing: <percentage>`
//! resolve against the element's own `font-size`, where they used to be
//! dropped to the length part of the value alone (`calc(5px + 50%)` became
//! `5.0`, a bare `50%` became `0.0`).
//!
//! # The measurement, and why the basis is `font-size` and not a glyph
//!
//! Chrome 153, headless, standards mode, `font: 20px/40px monospace;
//! white-space: pre; display: inline-block`, box width:
//!
//! | declaration | content | width | delta vs. unspaced |
//! |---|---|---|---|
//! | (none) | `abcde` | 60.21875 | — |
//! | `letter-spacing: 50%` | `abcde` | 110.21875 | **+50** (10px × 5 chars) |
//! | `letter-spacing: calc(5px + 50%)` | `abcde` | 135.21875 | **+75** (15px × 5) |
//! | (none) | `a a a` | 60.21875 | — |
//! | `word-spacing: 50%` | `a a a` | 80.21875 | **+20** (10px × 2 spaces) |
//! | `word-spacing: calc(5px + 50%)` | `a a a` | 90.21875 | **+30** (15px × 2) |
//! | `word-spacing: 100%` | `a a a` | 100.21875 | **+40** (20px × 2) |
//!
//! `10px` is exactly 50% of the 20px `font-size` — **not** 50% of the
//! monospace font's own glyph advance (measured from the unspaced row:
//! 60.21875 / 5 = 12.04375px per character, so a space-advance basis would
//! give 50% × 12.04375 ≈ 6.02px, which is not what Chrome draws). Doubling
//! `font-size` to 40px (same markup, same font) doubles the per-space/per-char
//! addition to 20px, confirming the basis scales with `font-size` and ruling
//! out a basis Chrome is holding constant. css-text-4 §10 describes
//! `word-spacing`'s percentage as relative to the space glyph's advance;
//! Chrome 153's actual behaviour, measured here, is not that — this issue and
//! these fixtures follow the measurement.
//!
//! This file complements `letter_word_spacing_tests.rs` (#698, the producers
//! that read `ComputedStyle::letter_spacing`/`word_spacing` at all) with the
//! percentage-resolution arithmetic itself, using the bundled Inter so the
//! glyph-advance fixture does not depend on the host's font set (CLAUDE.md
//! "Fonts" — any horizontal text geometry needs a declared font, not a
//! system generic). The two box-width fixtures below use `monospace` as
//! `letter_word_spacing_tests.rs` does, because a flat per-cluster px
//! addition is font-independent; only the glyph-position fixture at the end
//! needs an exact, host-stable font.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::text_query::glyph_bounds_for_offset;

const VW: f32 = 400.0;
const VH: f32 = 200.0;

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let e = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn txt(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

/// The width of the Parley line an IFC root laid out, for `text` under
/// `extra` (font-family/size/line-height folded in by the caller).
fn ifc_line_width(extra: &str, text: &str) -> f32 {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        &format!("width: 380px; white-space: pre; {extra}"),
    );
    txt(&mut doc, c, text);
    doc.resolve_layout(VW, VH);
    doc.tree.nodes[c.0]
        .text_layout
        .as_ref()
        .expect("the IFC root has no inline layout — this fixture is measuring nothing")
        .layout
        .width()
}

const BASE20: &str = "font-size: 20px; line-height: 40px; font-family: monospace";
const BASE40: &str = "font-size: 40px; line-height: 80px; font-family: monospace";

// ---------------------------------------------------------------------------
// letter-spacing: <percentage> / calc() with a percentage
// ---------------------------------------------------------------------------

#[test]
fn letter_spacing_percentage_resolves_against_font_size() {
    let plain = ifc_line_width(BASE20, "abcde");
    assert!(plain > 0.0, "positive control: the plain line has no width");
    let spaced = ifc_line_width(&format!("{BASE20}; letter-spacing: 50%"), "abcde");
    // 50% of the 20px font-size is 10px, added once per character (5), as
    // the fixed (non-percentage) case in letter_word_spacing_tests.rs
    // already establishes for the trailing-character rule.
    assert!(
        (spaced - plain - 50.0).abs() < 0.5,
        "letter-spacing: 50% at font-size: 20px must add 10px per character \
         (Chrome 153: 60.21875 -> 110.21875); got {plain} -> {spaced}"
    );
}

#[test]
fn letter_spacing_calc_adds_its_percentage_part_to_its_length_part() {
    let plain = ifc_line_width(BASE20, "abcde");
    let spaced = ifc_line_width(&format!("{BASE20}; letter-spacing: calc(5px + 50%)"), "abcde");
    // 5px + 50% of 20px = 15px per character, 5 characters.
    assert!(
        (spaced - plain - 75.0).abs() < 0.5,
        "letter-spacing: calc(5px + 50%) at font-size: 20px must add 15px \
         per character (Chrome 153: 60.21875 -> 135.21875); got {plain} -> {spaced}"
    );
}

/// Doubling the font-size doubles the resolved percentage — the fixed point
/// a mutant that hardcodes a particular basis (e.g. always 16px, or the
/// value from some other node) would still pass at one font-size but not two.
#[test]
fn letter_spacing_percentage_scales_with_the_elements_own_font_size() {
    let delta_20 = ifc_line_width(&format!("{BASE20}; letter-spacing: 50%"), "abcde")
        - ifc_line_width(BASE20, "abcde");
    let delta_40 = ifc_line_width(&format!("{BASE40}; letter-spacing: 50%"), "abcde")
        - ifc_line_width(BASE40, "abcde");
    assert!(
        (delta_20 - 50.0).abs() < 0.5,
        "expected +50px at font-size 20px, got {delta_20}"
    );
    assert!(
        (delta_40 - 100.0).abs() < 0.5,
        "expected +100px at font-size 40px (double the 20px case), got {delta_40}"
    );
}

// ---------------------------------------------------------------------------
// word-spacing: <percentage> / calc() with a percentage
// ---------------------------------------------------------------------------

#[test]
fn word_spacing_percentage_resolves_against_font_size() {
    let plain = ifc_line_width(BASE20, "a a a");
    assert!(plain > 0.0, "positive control: the plain line has no width");
    let spaced = ifc_line_width(&format!("{BASE20}; word-spacing: 50%"), "a a a");
    // 50% of the 20px font-size is 10px, added once per space (2) — NOT 50%
    // of the monospace glyph's own ~12.04px advance, which would give ~6.02px
    // and a delta of ~12, not 20. See the module doc's measurement table.
    assert!(
        (spaced - plain - 20.0).abs() < 0.5,
        "word-spacing: 50% at font-size: 20px must add 10px per space \
         (Chrome 153: 60.21875 -> 80.21875); got {plain} -> {spaced}"
    );
}

#[test]
fn word_spacing_calc_adds_its_percentage_part_to_its_length_part() {
    let plain = ifc_line_width(BASE20, "a a a");
    let spaced = ifc_line_width(&format!("{BASE20}; word-spacing: calc(5px + 50%)"), "a a a");
    // 5px + 50% of 20px = 15px per space, 2 spaces.
    assert!(
        (spaced - plain - 30.0).abs() < 0.5,
        "word-spacing: calc(5px + 50%) at font-size: 20px must add 15px per \
         space (Chrome 153: 60.21875 -> 90.21875); got {plain} -> {spaced}"
    );
}

#[test]
fn word_spacing_percentage_scales_with_the_elements_own_font_size() {
    let delta_20 =
        ifc_line_width(&format!("{BASE20}; word-spacing: 50%"), "a a a") - ifc_line_width(BASE20, "a a a");
    let delta_40 =
        ifc_line_width(&format!("{BASE40}; word-spacing: 50%"), "a a a") - ifc_line_width(BASE40, "a a a");
    assert!(
        (delta_20 - 20.0).abs() < 0.5,
        "expected +20px at font-size 20px, got {delta_20}"
    );
    assert!(
        (delta_40 - 40.0).abs() < 0.5,
        "expected +40px at font-size 40px (double the 20px case), got {delta_40}"
    );
}

/// `word-spacing: 100%` at `font-size: 20px` adds the whole font-size per
/// space — the third row of the module doc's table, and a value a
/// `(px, pct)` split that silently clamped percentages to `0.0..=1.0` (there
/// is no such clamp in the spec or in Chrome) would fail at.
#[test]
fn word_spacing_one_hundred_percent_adds_the_whole_font_size() {
    let plain = ifc_line_width(BASE20, "a a a");
    let spaced = ifc_line_width(&format!("{BASE20}; word-spacing: 100%"), "a a a");
    assert!(
        (spaced - plain - 40.0).abs() < 0.5,
        "word-spacing: 100% at font-size: 20px must add 20px per space \
         (Chrome 153: 60.21875 -> 100.21875); got {plain} -> {spaced}"
    );
}

// ---------------------------------------------------------------------------
// A restyle: font-size moves, so the resolved spacing must move with it
// (`same_text_layout_inputs`/`same_measured_text_inputs` compare `font_size`
// directly, so this is really pinning that they still do).
// ---------------------------------------------------------------------------

#[test]
fn a_font_size_change_reshapes_a_percentage_letter_spacing() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let b = el(
        &mut doc,
        body,
        "div",
        "display: inline-block; white-space: pre; \
         font-family: monospace; line-height: 40px; font-size: 20px; letter-spacing: 50%",
    );
    txt(&mut doc, b, "abcde");
    doc.resolve_layout(VW, VH);
    let before = doc.tree.nodes[b.0].layout.width;
    assert!(before > 0.0, "positive control: nothing was laid out");

    doc.set_attribute(
        b,
        "style",
        "display: inline-block; white-space: pre; \
         font-family: monospace; line-height: 80px; font-size: 40px; letter-spacing: 50%",
    );
    doc.resolve_layout(VW, VH);
    let after = doc.tree.nodes[b.0].layout.width;
    // Doubling font-size doubles both the glyph advances (an unaffected
    // monospace cell is ~2x) AND the resolved spacing (also ~2x) — so the
    // box roughly doubles, not merely grows.
    assert!(
        after > before * 1.8,
        "a font-size change must re-measure the box including its resolved \
         percentage spacing; got {before} -> {after}"
    );
}

// ---------------------------------------------------------------------------
// Direct glyph-advance check, with the bundled Inter rather than a system
// generic (CLAUDE.md "Fonts"): the resolved percentage is a flat px amount
// independent of any glyph's own metrics, so this isolates exactly that
// addition at the glyph level rather than at a box's total width.
// ---------------------------------------------------------------------------

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn doc_with_inter() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace743"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    doc
}

/// `x` positions of 'a' and 'b' in `"ab"`, isolating the advance between
/// the two glyphs — unaffected by anything that moved the container itself.
fn ab_advance(extra: &str) -> f32 {
    let mut doc = doc_with_inter();
    let body = doc.body();
    let c = el(
        &mut doc,
        body,
        "div",
        &format!(
            "width: 380px; white-space: pre; font-family: ProbeFace743; \
             font-size: 20px; line-height: 24px; {extra}"
        ),
    );
    txt(&mut doc, c, "ab");
    doc.resolve_layout(VW, VH);
    let a = glyph_bounds_for_offset(&doc, c.0 as u64, 0).expect("glyph 'a'");
    let b = glyph_bounds_for_offset(&doc, c.0 as u64, 1).expect("glyph 'b'");
    b.x - a.x
}

#[test]
fn letter_spacing_percentage_shifts_the_next_glyph_by_the_resolved_amount() {
    let plain = ab_advance("");
    assert!(plain > 0.0, "positive control: 'a' and 'b' did not advance at all");
    let spaced = ab_advance("letter-spacing: 50%");
    // 50% of the 20px font-size is 10px, inserted as one letter-spacing step
    // between 'a' and 'b' — on top of 'a's own (Inter) advance, whatever
    // that measures as on this host.
    assert!(
        (spaced - plain - 10.0).abs() < 0.5,
        "'b' must sit 10px further right of 'a' than it does unspaced \
         (50% of the 20px font-size); got plain advance {plain}, spaced {spaced}"
    );
}
