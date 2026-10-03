//! Issue #674 — the `<hr>` UA rule has to put **ink** on the surface, not only
//! numbers in `ComputedStyle`.
//!
//! `ua_block_defaults_tests` pins the computed box. That is not the same claim:
//! a border width of 1px with `border-style: None` computes the same width and
//! paints nothing, which is close to what HEAD did — the UA sheet's
//! `* { border-width: 0 }` reset left `<hr>` a zero-height box with no border,
//! and an app author's first report of this was "my `<hr>` is invisible".
//!
//! This is a **local** pixel oracle, in the sense
//! `reference_visual_regression_gap` argues for: it asserts on a region where
//! the correct output is provably a known colour and the broken output is
//! provably empty, rather than comparing whole screens. The surface starts
//! transparent (nothing in the tree declares a background), so every non-zero
//! pixel in it is the rule.
//!
//! Measured against this file at HEAD (the UA `hr` rule reverted): the grey
//! pixel count is **0**, and `a_bare_hr_paints_a_grey_rule` fails on its first
//! assertion.
//!
//! **#731 (fixed):** `inset` now paints a two-tone bevel, matching Chromium's
//! own `CalculateInsetOutsetColor` formula (`border_bevel_tests.rs` is the
//! dedicated fixture, with a wide border so antialiasing can't be blamed for
//! the shade). This file's rows are **not** the issue's own measured Chrome
//! values (154 over 238 at scale 1) — this renderer gives the formula's exact
//! theoretical output (44 over 212) with no antialiasing blend on a crisp
//! 1px line, where Chrome's own hairline rendering blends its edge with
//! whatever is behind it. See the comment on the assertion below.

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

/// Chrome's `gray`, which the UA rule sets as the `<hr>`'s `color` and the
/// border inherits through `currentcolor`.
const GRAY: (u8, u8, u8) = (128, 128, 128);

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let id = doc.create_element(tag);
    if !style.is_empty() {
        doc.set_attribute(id, "style", style);
    }
    doc.append_child(parent, id);
    id
}

fn rasterize(doc: &mut RinchDocument) -> Vec<u8> {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.pixels().to_vec()
}

/// Pixels whose RGB is within `tol` of `rgb` and which are fully opaque.
fn near_count(px: &[u8], rgb: (u8, u8, u8), tol: i32) -> u32 {
    let mut n = 0;
    for i in (0..px.len()).step_by(4) {
        let d = |a: u8, b: u8| (a as i32 - b as i32).abs();
        if px[i + 3] > 250
            && d(px[i], rgb.0) <= tol
            && d(px[i + 1], rgb.1) <= tol
            && d(px[i + 2], rgb.2) <= tol
        {
            n += 1;
        }
    }
    n
}

/// Any pixel at all that is not fully transparent.
///
/// `as_chunks::<4>` rather than `chunks_exact(4)`: the pixmap is RGBA8, so the
/// stride is a constant and the array form lets the compiler see it. (It is
/// also what `clippy::chunks_exact_to_as_chunks` asks for from Rust 1.98, which
/// is what CI runs.) The remainder is discarded on purpose — a pixmap is always
/// a whole number of pixels.
fn ink_count(px: &[u8]) -> u32 {
    px.as_chunks::<4>().0.iter().filter(|p| p[3] != 0).count() as u32
}

/// How many non-transparent pixels each row holds, rows with none omitted.
///
/// The strongest assertion this file can make: nothing in these trees paints a
/// background, so a row that holds ink holds the rule and nothing else.
fn ink_rows(px: &[u8]) -> std::collections::BTreeMap<u32, u32> {
    let mut rows = std::collections::BTreeMap::new();
    for (i, p) in px.as_chunks::<4>().0.iter().enumerate() {
        if p[3] != 0 {
            *rows.entry(i as u32 / VW as u32).or_insert(0u32) += 1;
        }
    }
    rows
}

/// A bare `<hr>` paints a full-width grey rule, two physical rows tall, ten
/// pixels down from the top of its container.
///
/// Every number is derived from Chrome's measured values and checked
/// independently by `ua_block_defaults_tests`: `margin-block: 0.5em` at the
/// container's 20px font is 10px, the box is `height: 0` with a 1px border on
/// each side, and `color: gray` is (128, 128, 128).
#[test]
fn a_bare_hr_paints_a_grey_rule() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 800px; font-size: 20px");
    el(&mut doc, c, "hr", "");
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    // #731: `inset` darkens the top (and left) side and lightens the bottom
    // (and right) side of `gray` (128,128,128) rather than painting it flat.
    // Measured against this renderer's own output (no antialiasing blend on
    // a crisp 1px line; `border_bevel_tests.rs` is the dedicated fixture with
    // a wide border, and cross-checks these same two values independently):
    // top row (44, 44, 44), bottom row (212, 212, 212). The *columns* at
    // x=0 and x=799 are the 1px left/right border too, which the UA sheet
    // also gives `inset`: the left column is dark for the full two-row
    // height (it's `inset`'s own top/left side, not split by row) and the
    // right column is light for the full height — so a whole-image colour
    // count would find the dark shade in row 11 too (via that left column)
    // and isn't the right check for "the TOP row is darkened"; columns
    // 1..=798, away from those two border columns, are.
    const DARK: (u8, u8, u8) = (44, 44, 44);
    const LIGHT: (u8, u8, u8) = (212, 212, 212);

    fn count_in_row(px: &[u8], row: u32, rgb: (u8, u8, u8), tol: i32) -> u32 {
        let d = |a: u8, b: u8| (a as i32 - b as i32).abs();
        let mut n = 0;
        for x in 1..799u32 {
            let i = ((row * VW as u32 + x) * 4) as usize;
            if px[i + 3] > 250
                && d(px[i], rgb.0) <= tol
                && d(px[i + 1], rgb.1) <= tol
                && d(px[i + 2], rgb.2) <= tol
            {
                n += 1;
            }
        }
        n
    }

    let n_top = count_in_row(&px, 10, DARK, 2);
    let n_bottom = count_in_row(&px, 11, LIGHT, 2);
    assert!(
        n_top > 0 && n_bottom > 0,
        "a bare <hr> must paint a two-tone grey bevel; found none (this is the \
         assertion that fails at HEAD, where the UA `* {{ border-width: 0 }}` \
         reset left <hr> borderless)"
    );
    assert_eq!(
        n_top, 798,
        "the whole top row (minus the two border-column pixels) must be the \
         darkened shade; got {n_top} of 798"
    );
    assert_eq!(
        n_bottom, 798,
        "the whole bottom row (minus the two border-column pixels) must be \
         the lightened shade; got {n_bottom} of 798"
    );

    // The whole surface, row by row: exactly two rows of exactly 800 opaque
    // pixels and nothing anywhere else.
    assert_eq!(
        ink_rows(&px),
        [(10u32, 800u32), (11, 800)].into_iter().collect(),
        "the <hr> must be the only thing painted, as two solid 800px rows"
    );
    assert_eq!(ink_count(&px), 1600);
}

/// The oracle discriminates: an author `border: none` puts the surface back to
/// empty, exactly as HEAD was for every `<hr>`.
///
/// Without this, `a_bare_hr_paints_a_grey_rule` would be satisfied by anything
/// that painted grey anywhere — and it is also the paint-side half of the
/// author-override handshake the computed-value file asserts.
#[test]
fn an_author_border_none_paints_nothing() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 800px; font-size: 20px");
    el(&mut doc, c, "hr", "border: none");
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    assert_eq!(
        near_count(&px, GRAY, 2),
        0,
        "an author `border: none` must leave the <hr> invisible"
    );
    assert_eq!(ink_count(&px), 0, "and leave the surface entirely empty");
}

/// An author `color` repaints the rule, because the border is `currentcolor`
/// over the UA sheet's `color: gray` rather than a UA `border-color`.
///
/// The paint-side twin of `an_authored_color_repaints_the_hr_border`: the two
/// spellings produce identical pixels for every `<hr>` that declares no colour,
/// so this is the only place the ink can tell them apart.
#[test]
fn an_authored_color_repaints_the_rule() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let c = el(&mut doc, body, "div", "width: 800px; font-size: 20px");
    el(&mut doc, c, "hr", "color: rgb(0, 128, 0)");
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    assert_eq!(
        near_count(&px, GRAY, 2),
        0,
        "the rule must no longer be grey once the element declares a colour"
    );
    // #731: the border shades whatever colour it resolves to, not just grey —
    // the top half darkens (0,128,0) to (0,44,0), the bottom half lightens it
    // to (0,212,0), by the same formula `border_bevel_tests.rs` pins directly.
    let n_top = near_count(&px, (0, 44, 0), 2);
    let n_bottom = near_count(&px, (0, 212, 0), 2);
    assert!(
        (790..=800).contains(&n_top),
        "the darkened top half must be painted in the resolved colour's dark \
         shade; got {n_top} matching pixels"
    );
    assert!(
        (790..=800).contains(&n_bottom),
        "…and the lightened bottom half in its light shade; got {n_bottom} \
         matching pixels"
    );
    assert_eq!(
        ink_rows(&px),
        [(10u32, 800u32), (11, 800)].into_iter().collect(),
        "in the same place and at the same size as the grey one"
    );
}
