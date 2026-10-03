//! `vertical-align: sub/super/<length>/<percentage>` (#724).
//!
//! Before this, `ComputedStyle` had no `vertical_align` field at all, and the
//! IFC placed every inline run on the same baseline — `H<sub>2</sub>O` and
//! `x<sup>2</sup>` rendered as `H2O` and `x2` at a smaller size (#674 gave
//! them that), never raised or lowered.
//!
//! # What this covers, and what it does not
//!
//! `Sub`, `Super`, a `<length>` and a `<percentage>` all have a layout
//! consumer: [`ifc::vertical_align_shift_px`] (not exported; exercised here
//! through the public query functions and the painter). `Top`/`TextTop`/
//! `Middle`/`Bottom`/`TextBottom` parse and round-trip through
//! `ComputedStyle` but lay out as `Baseline` — they only have a defined effect
//! against an atomic inline or a table cell, which is out of scope here —
//! filed as #1357.
//!
//! The shift is a **post-layout glyph move**, not a Parley style (0.11.1 has
//! no per-run baseline offset), so it does **not** grow the line box the way
//! Chrome's does — a raised `<sup>` can paint into the line above. Also
//! #1357, alongside the five keywords above.
//!
//! # Where the `sub`/`super` ratios come from
//!
//! CSS 2.1 §10.8.1 says `sub`/`super` move the box "to the proper position
//! for subscripts/superscripts of the parent's box" and leaves the amount
//! UA-defined. Chrome reads the actual OS/2 sub/superscript metrics of the
//! parent's font, non-linearly in the font size (measured: the ratio at 24px
//! Inter is about 8% off the ratio at 16px — device-pixel rounding inside
//! Chrome's own layout, not a simpler closed form reachable here). So
//! `SUB_RATIO`/`SUPER_RATIO` in `ifc.rs` are a *linear* ratio calibrated once,
//! against exactly the case below — Chrome 153, the bundled Inter
//! (registered under an override name so the host's fonts cannot answer),
//! `font-size: 16px` — and are exact only there. Measured with a local HTML
//! page serving this same ttf as `@font-face`, `x<span>y</span>x<sub>s</sub>
//! x<sup>s</sup>`, same-size spans isolating the shift from #674's font-size
//! change:
//!
//! | element | shift vs. a `baseline` sibling, 16px parent |
//! |---|---|
//! | `vertical-align: sub` | **+4.1875px** (down) |
//! | `vertical-align: super` | **−6.328125px** (up) |
//! | `vertical-align: 4px` | **−4px** (up — a positive length raises) |
//! | `vertical-align: 25%` (line-height 16px) | **−4px** (up — 25% of the
//!   element's *own* `line-height`, not the parent's) |
//!
//! The sub/super shift does not move with `line-height` (checked at 16px and
//! 32px line-height, same 16px font-size: identical shift both times) — only
//! with the parent's `font-size`, as the spec's wording implies.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::{LengthPercentageValue, VerticalAlignValue};
use rinch_dom::text_query::{caret_position_for_offset, glyph_bounds_for_offset};

const VW: f32 = 400.0;
const VH: f32 = 200.0;
const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
/// Declared, so the line box is a statement rather than a font measurement
/// (the shift itself doesn't move with it — see the module doc — but every
/// other geometry in these fixtures would otherwise pin the host's fonts).
const BASE: &str = "font-family: ProbeFace; font-size: 16px; line-height: 16px";

/// 67/256 — see [`ifc::vertical_align_shift_px`]'s doc and this module's.
const SUB_PX: f32 = 4.1875;
/// 405/1024.
const SUPER_PX: f32 = 6.328125;

fn doc() -> RinchDocument {
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
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    doc.append_child(parent, e);
    e
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
}

// ---------------------------------------------------------------------------
// 1. The property parses: all eight keywords, a length, a percentage.
// ---------------------------------------------------------------------------

#[test]
fn computed_style_parses_every_vertical_align_keyword() {
    let mut d = RinchDocument::new();
    let body = d.body();
    for (decl, want) in [
        ("baseline", VerticalAlignValue::Baseline),
        ("sub", VerticalAlignValue::Sub),
        ("super", VerticalAlignValue::Super),
        ("top", VerticalAlignValue::Top),
        ("text-top", VerticalAlignValue::TextTop),
        ("middle", VerticalAlignValue::Middle),
        ("bottom", VerticalAlignValue::Bottom),
        ("text-bottom", VerticalAlignValue::TextBottom),
    ] {
        let s = el(&mut d, body, "span", &format!("vertical-align: {decl}"));
        d.resolve_layout(VW, VH);
        assert_eq!(
            d.tree.nodes[s.0].computed_style.vertical_align, want,
            "vertical-align: {decl}"
        );
    }
}

#[test]
fn computed_style_parses_a_length_and_a_percentage() {
    let mut d = RinchDocument::new();
    let body = d.body();
    let len = el(&mut d, body, "span", "vertical-align: 10px");
    let pct = el(&mut d, body, "span", "vertical-align: 25%");
    d.resolve_layout(VW, VH);
    assert_eq!(
        d.tree.nodes[len.0].computed_style.vertical_align,
        VerticalAlignValue::LengthPercentage(LengthPercentageValue::Length(10.0))
    );
    assert_eq!(
        d.tree.nodes[pct.0].computed_style.vertical_align,
        VerticalAlignValue::LengthPercentage(LengthPercentageValue::Percent(0.25))
    );
}

// ---------------------------------------------------------------------------
// 2. The UA stylesheet gives `<sub>`/`<sup>` their half of #724.
// ---------------------------------------------------------------------------

#[test]
fn the_ua_stylesheet_gives_sub_and_sup_their_vertical_align() {
    let mut d = RinchDocument::new();
    let body = d.body();
    let sub = el(&mut d, body, "sub", "");
    let sup = el(&mut d, body, "sup", "");
    d.resolve_layout(VW, VH);
    assert_eq!(
        d.tree.nodes[sub.0].computed_style.vertical_align,
        VerticalAlignValue::Sub
    );
    assert_eq!(
        d.tree.nodes[sup.0].computed_style.vertical_align,
        VerticalAlignValue::Super
    );
}

// ---------------------------------------------------------------------------
// 3. The layout consumer: glyph bounds and caret position, calibrated
//    against Chrome 153 + Inter at 16px (see module doc).
// ---------------------------------------------------------------------------

/// `line`'s flat IFC text is `"ABCDEF"`: `A`/`C`/`E` plain, `B` in a plain
/// `<span>` (the baseline control, isolating #674's font-size change from
/// this issue's vertical shift), `D` in a `<sub>`, `F` in a `<sup>`.
fn sub_sup_line(d: &mut RinchDocument) -> NodeId {
    let body = d.body();
    let line = el(d, body, "div", &format!("{BASE}; white-space: nowrap"));
    text(d, line, "A");
    let base = el(d, line, "span", "");
    text(d, base, "B");
    text(d, line, "C");
    let sub = el(d, line, "sub", "");
    text(d, sub, "D");
    text(d, line, "E");
    let sup = el(d, line, "sup", "");
    text(d, sup, "F");
    d.resolve_layout(VW, VH);
    line
}

/// **If this test fails on `main` (before #724), it reproduces the issue**:
/// `sub_shift`/`sup_shift` are both `0.0` there, because `ComputedStyle` has
/// no `vertical_align` field for `glyph_bounds_for_offset` to consult.
#[test]
fn sub_and_super_shift_the_glyph_bounds_by_the_calibrated_amount() {
    let mut d = doc();
    let line = sub_sup_line(&mut d);

    let base = glyph_bounds_for_offset(&d, line.0 as u64, 1).unwrap(); // B
    let sub = glyph_bounds_for_offset(&d, line.0 as u64, 3).unwrap(); // D
    let sup = glyph_bounds_for_offset(&d, line.0 as u64, 5).unwrap(); // F

    let sub_shift = sub.y - base.y;
    let sup_shift = sup.y - base.y;

    assert!(
        (sub_shift - SUB_PX).abs() < 0.01,
        "sub shift = {sub_shift}, want {SUB_PX}"
    );
    assert!(
        (sup_shift - -SUPER_PX).abs() < 0.01,
        "super shift = {sup_shift}, want {}",
        -SUPER_PX
    );
}

/// The caret and the glyph bounds are two independent public entry points
/// (`text_query::caret_position_for_offset` and `::glyph_bounds_for_offset`)
/// over two different Parley geometries (`Cursor::geometry` vs. a cluster's
/// line metrics) — they disagree about where the *unshifted* baseline is, so
/// this does not compare their absolute `y`. It compares the **delta** each
/// one's own vertical-align consult adds, which must be identical: both read
/// the same `InlineLayout::vertical_align_shift_at`, so a byte inside the
/// `<sub>` must move the caret by exactly the amount it moves the glyph box,
/// and a mutant that patches one lookup and not the other is exactly what
/// this is for.
#[test]
fn the_caret_and_the_glyph_bounds_agree_on_the_sub_shift() {
    let mut d = doc();
    let line = sub_sup_line(&mut d);

    let caret_plain = caret_position_for_offset(&d, line.0 as u64, 2).unwrap().1; // before C
    let caret_sub = caret_position_for_offset(&d, line.0 as u64, 3).unwrap().1; // before D
    let caret_delta = caret_sub - caret_plain;

    let base = glyph_bounds_for_offset(&d, line.0 as u64, 1).unwrap();
    let sub = glyph_bounds_for_offset(&d, line.0 as u64, 3).unwrap();
    let bounds_delta = sub.y - base.y;

    assert!(
        (caret_delta - bounds_delta).abs() < 0.01,
        "caret moved {caret_delta}, glyph bounds moved {bounds_delta} — paint and \
         hit-test/caret disagree about the sub shift"
    );
    assert!(
        (caret_delta - SUB_PX).abs() < 0.01,
        "caret shift = {caret_delta}, want {SUB_PX}"
    );
}

// ---------------------------------------------------------------------------
// 4. `<length>`/`<percentage>`: sign (positive raises) and basis (the
//    element's own `line-height`, not the parent's).
// ---------------------------------------------------------------------------

#[test]
fn a_positive_length_raises_the_box() {
    let mut d = doc();
    let body = d.body();
    let line = el(&mut d, body, "div", &format!("{BASE}; white-space: nowrap"));
    text(&mut d, line, "A");
    let raised = el(&mut d, line, "span", "vertical-align: 4px");
    text(&mut d, raised, "B");
    d.resolve_layout(VW, VH);

    let base = glyph_bounds_for_offset(&d, line.0 as u64, 0).unwrap();
    let up = glyph_bounds_for_offset(&d, line.0 as u64, 1).unwrap();
    let shift = up.y - base.y;
    assert!(
        (shift - -4.0).abs() < 0.01,
        "vertical-align: 4px shifted the box by {shift}, want -4.0 (up)"
    );
}

/// A percentage resolves against the **element's own** `line-height`, not
/// the parent's or the IFC root's — this span's `line-height: 16px` is what
/// `25%` is taken of, even though the root (and therefore every other glyph
/// on the line) uses whatever `BASE` declares.
#[test]
fn a_percentage_resolves_against_the_elements_own_line_height() {
    let mut d = doc();
    let body = d.body();
    let line = el(&mut d, body, "div", &format!("{BASE}; white-space: nowrap"));
    text(&mut d, line, "A");
    let raised = el(
        &mut d,
        line,
        "span",
        "vertical-align: 25%; line-height: 16px",
    );
    text(&mut d, raised, "B");
    d.resolve_layout(VW, VH);

    let base = glyph_bounds_for_offset(&d, line.0 as u64, 0).unwrap();
    let up = glyph_bounds_for_offset(&d, line.0 as u64, 1).unwrap();
    let shift = up.y - base.y;
    assert!(
        (shift - -4.0).abs() < 0.01,
        "vertical-align: 25% of a 16px line-height shifted the box by {shift}, want -4.0"
    );
}

// ---------------------------------------------------------------------------
// 5. The painter moves the glyph, not just the query functions.
// ---------------------------------------------------------------------------

#[cfg(feature = "software-renderer")]
mod painted {
    use super::*;
    use rinch_dom::paint::skia_painter::TinySkiaPainter;

    fn row_ink(d: &mut RinchDocument) -> Vec<usize> {
        let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &d.tree,
            &mut painter,
            1.0,
            (VW, VH),
            &mut d.font_cx,
            &mut cx,
        );
        let px: Vec<[u8; 4]> = painter.pixels().as_chunks::<4>().0.to_vec();
        (0..VH as usize)
            .map(|y| {
                let row = y * VW as usize;
                px[row..row + VW as usize]
                    .iter()
                    .filter(|p| p[3] > 0 && !(p[0] > 240 && p[1] > 240 && p[2] > 240))
                    .count()
            })
            .collect()
    }

    /// The first and last inked row, found by difference against an
    /// otherwise-identical document with no text at all — so this is a pin on
    /// *where this glyph painted*, not on the local font's exact shape.
    fn ink_band(
        with_glyph: &mut RinchDocument,
        without_glyph: &mut RinchDocument,
    ) -> (usize, usize) {
        let a = row_ink(with_glyph);
        let b = row_ink(without_glyph);
        let first = a
            .iter()
            .zip(&b)
            .position(|(x, y)| x != y)
            .expect("the glyph painted nothing");
        let last = a.iter().zip(&b).rposition(|(x, y)| x != y).unwrap();
        (first, last)
    }

    /// Forces `node`'s `vertical-align` straight on `ComputedStyle` (no CSS
    /// spelling needed — `sub`/`super`/a length/a percentage all go through
    /// the same field) and re-lays-out, the same technique
    /// `underline_offset_tests.rs` uses: `layout_dirty` alone is not enough,
    /// Taffy serves its cached measure unless the IFC's `text_layout` is
    /// dropped too.
    fn force_vertical_align(
        d: &mut RinchDocument,
        ifc_root: NodeId,
        node: NodeId,
        va: VerticalAlignValue,
    ) {
        d.tree.nodes[node.0].computed_style.vertical_align = va;
        d.tree.nodes[ifc_root.0].text_layout = None;
        d.tree.layout_dirty = true;
        d.resolve_layout(VW, VH);
    }

    /// One `<div>` holding a lone `<sub>D</sub>` (or, with `va`, that element
    /// forced to some other `vertical-align` after its first layout).
    fn one_sub(va: Option<VerticalAlignValue>) -> RinchDocument {
        let mut d = doc();
        let body = d.body();
        let line = el(&mut d, body, "div", &format!("{BASE}; white-space: nowrap"));
        let sub = el(&mut d, line, "sub", "");
        text(&mut d, sub, "D");
        d.resolve_layout(VW, VH);
        if let Some(va) = va {
            force_vertical_align(&mut d, line, sub, va);
        }
        d
    }

    fn empty_line() -> RinchDocument {
        let mut d = doc();
        let body = d.body();
        el(&mut d, body, "div", &format!("{BASE}; white-space: nowrap"));
        d.resolve_layout(VW, VH);
        d
    }

    /// `<sub>`'s glyph paints in a lower band than the same glyph forced back
    /// to `baseline` — the pixel-level half of
    /// [`super::sub_and_super_shift_the_glyph_bounds_by_the_calibrated_amount`].
    ///
    /// **If this fails on `main` (before #724), `sub_first == baseline_first`**:
    /// nothing moved the glyph, because paint never consulted a
    /// `vertical_align` field that did not exist.
    #[test]
    fn a_sub_glyph_paints_lower_than_the_same_glyph_at_baseline() {
        let mut with_sub = one_sub(None);
        let mut forced_baseline = one_sub(Some(VerticalAlignValue::Baseline));
        let mut blank = empty_line();

        let (sub_first, _) = ink_band(&mut with_sub, &mut blank);
        let (baseline_first, _) = ink_band(&mut forced_baseline, &mut blank);

        // Not asserted as an exact row count: anti-aliasing at a sub-pixel
        // shift can move the measured band edge by a row either way, and the
        // 4px-class calibrated shift is well above that noise.
        assert!(
            sub_first > baseline_first,
            "the sub glyph's first inked row ({sub_first}) is not below the \
             baseline-forced control's ({baseline_first}) — vertical-align: sub \
             painted no lower than baseline"
        );
    }
}
