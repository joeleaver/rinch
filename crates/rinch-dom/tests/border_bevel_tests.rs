//! Issue #731 — `border-style: inset/outset/groove/ridge` paint a two-tone
//! bevel, matching Chromium's own `CalculateInsetOutsetColor` /
//! `Color::Light`/`Color::Dark` (read from `box_border_painter.cc` and
//! `color.cc` at `chromium.googlesource.com`, since the exact shades are
//! Chromium's own tuned constants, not CSS-spec text).
//!
//! These are local pixel oracles (`reference_visual_regression_gap`): every
//! box here paints on an otherwise-empty transparent surface, so a sampled
//! pixel's colour IS the rule, not an artifact of anything else on screen.
//! Every box uses a wide border (20px/40px) so a sample point sits well away
//! from antialiased edges and corners — a 1px border (like the default
//! `<hr>`'s) blends its true shade with whatever is behind it, which is why
//! the #731 issue's own measured Chrome values (154 over 238) don't match
//! this file's (44 over 212): they're the same formula seen through 1px of
//! antialiasing, not a different one. `ua_hr_paint_tests.rs` carries that
//! 1px case and is updated by this PR to match the real (bevelled) output.

use peniko::Brush;
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 400.0;
const VH: f32 = 400.0;

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

/// The RGBA of the pixel at `(x, y)` in an `VW`-wide RGBA8 buffer.
fn pixel_at(px: &[u8], x: u32, y: u32) -> (u8, u8, u8, u8) {
    let i = ((y * VW as u32 + x) * 4) as usize;
    (px[i], px[i + 1], px[i + 2], px[i + 3])
}

/// Assert the pixel at `(x, y)` is `rgb` within `tol` per channel and fully
/// opaque.
fn assert_pixel(px: &[u8], x: u32, y: u32, rgb: (u8, u8, u8), tol: i32, what: &str) {
    let (r, g, b, a) = pixel_at(px, x, y);
    let d = |a: u8, b: u8| (a as i32 - b as i32).abs();
    assert!(
        a > 250 && d(r, rgb.0) <= tol && d(g, rgb.1) <= tol && d(b, rgb.2) <= tol,
        "{what}: expected ~{rgb:?} (tol {tol}) at ({x},{y}), got ({r},{g},{b},{a})"
    );
}

/// `border: 20px inset gray` on a 200x200 box: top and left are darkened to
/// (44,44,44), bottom and right are lightened to (212,212,212). Values are
/// Chromium's `CalculateInsetOutsetColor` applied to `rgb(128,128,128)`
/// (computed independently in Python against the same formula this PR's
/// `bevel` module implements, cross-checked against the derivation in the
/// commit's module doc).
#[test]
fn inset_darkens_top_left_and_lightens_bottom_right() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    el(
        &mut doc,
        body,
        "div",
        "width: 200px; height: 200px; margin: 0; border: 20px inset gray;",
    );
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    const DARK: (u8, u8, u8) = (44, 44, 44);
    const LIGHT: (u8, u8, u8) = (212, 212, 212);

    // Top band, mid-width, 10px down (well inside the 20px band).
    assert_pixel(&px, 100, 10, DARK, 2, "top (inset)");
    // Left band, mid-height.
    assert_pixel(&px, 10, 100, DARK, 2, "left (inset)");
    // Bottom band, mid-width, 10px up from the box's bottom edge (y=200).
    assert_pixel(&px, 100, 190, LIGHT, 2, "bottom (inset)");
    // Right band, mid-height.
    assert_pixel(&px, 190, 100, LIGHT, 2, "right (inset)");
}

/// `outset` is the exact reverse of `inset`: top/left lighten, bottom/right
/// darken.
#[test]
fn outset_is_the_reverse_of_inset() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    el(
        &mut doc,
        body,
        "div",
        "width: 200px; height: 200px; margin: 0; border: 20px outset gray;",
    );
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    const DARK: (u8, u8, u8) = (44, 44, 44);
    const LIGHT: (u8, u8, u8) = (212, 212, 212);

    assert_pixel(&px, 100, 10, LIGHT, 2, "top (outset)");
    assert_pixel(&px, 10, 100, LIGHT, 2, "left (outset)");
    assert_pixel(&px, 100, 190, DARK, 2, "bottom (outset)");
    assert_pixel(&px, 190, 100, DARK, 2, "right (outset)");
}

/// `groove` splits each 40px edge in half along its own thickness: the half
/// nearer the box's outer edge takes `inset`'s shading, the half nearer the
/// content takes `outset`'s. On every side that puts dark physically
/// "outward" and light physically "inward" for top/left, and the reverse for
/// bottom/right — not a uniform "dark on top, light on bottom" rule; see the
/// `bevel::groove_or_ridge` doc comment for the derivation from Chromium's
/// own per-side case analysis.
#[test]
fn groove_splits_each_edge_into_two_shaded_halves() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    el(
        &mut doc,
        body,
        "div",
        "width: 200px; height: 200px; margin: 0; border: 40px groove gray;",
    );
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    const DARK: (u8, u8, u8) = (44, 44, 44);
    const LIGHT: (u8, u8, u8) = (212, 212, 212);

    // Top band spans y in [0, 40]. Outer half [0,20) is dark, inner [20,40) light.
    assert_pixel(&px, 100, 10, DARK, 2, "top outer (groove)");
    assert_pixel(&px, 100, 30, LIGHT, 2, "top inner (groove)");

    // Left band spans x in [0, 40]: outer dark, inner light.
    assert_pixel(&px, 10, 100, DARK, 2, "left outer (groove)");
    assert_pixel(&px, 30, 100, LIGHT, 2, "left inner (groove)");

    // Bottom band spans y in [160, 200] (box bottom at y=200): outer
    // (nearest y=200) is LIGHT, inner (nearest the content) is DARK — the
    // reverse of top's placement, because "outer" is a different physical
    // direction on this side.
    assert_pixel(&px, 100, 190, LIGHT, 2, "bottom outer (groove)");
    assert_pixel(&px, 100, 170, DARK, 2, "bottom inner (groove)");

    // Right band spans x in [160, 200]: outer (nearest x=200) LIGHT, inner DARK.
    assert_pixel(&px, 190, 100, LIGHT, 2, "right outer (groove)");
    assert_pixel(&px, 170, 100, DARK, 2, "right inner (groove)");
}

/// `ridge` is the exact reverse split of `groove`.
#[test]
fn ridge_is_the_reverse_split_of_groove() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    el(
        &mut doc,
        body,
        "div",
        "width: 200px; height: 200px; margin: 0; border: 40px ridge gray;",
    );
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    const DARK: (u8, u8, u8) = (44, 44, 44);
    const LIGHT: (u8, u8, u8) = (212, 212, 212);

    assert_pixel(&px, 100, 10, LIGHT, 2, "top outer (ridge)");
    assert_pixel(&px, 100, 30, DARK, 2, "top inner (ridge)");
    assert_pixel(&px, 10, 100, LIGHT, 2, "left outer (ridge)");
    assert_pixel(&px, 30, 100, DARK, 2, "left inner (ridge)");
    assert_pixel(&px, 100, 190, DARK, 2, "bottom outer (ridge)");
    assert_pixel(&px, 100, 170, LIGHT, 2, "bottom inner (ridge)");
    assert_pixel(&px, 190, 100, DARK, 2, "right outer (ridge)");
    assert_pixel(&px, 170, 100, LIGHT, 2, "right inner (ridge)");
}

/// Black edge case: `CalculateInsetOutsetColor` special-cases very dark
/// colours (luminance <= that of `rgb(32,32,32)`) to get extra contrast —
/// the darkened side LIGHTENS to (84,84,84) instead of trying to darken
/// black past itself, and the lightened side lightens twice, to (168,168,168).
/// An `inset` black border therefore shows grey on *both* sides, never pure
/// black anywhere — the case the issue specifically calls out.
#[test]
fn black_inset_border_shows_grey_on_both_sides_never_pure_black() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    el(
        &mut doc,
        body,
        "div",
        "width: 200px; height: 200px; margin: 0; border: 20px inset black;",
    );
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    assert_pixel(&px, 100, 10, (84, 84, 84), 2, "top (inset black)");
    assert_pixel(&px, 100, 190, (168, 168, 168), 2, "bottom (inset black)");

    // Never pure black anywhere the border painted.
    let (r, g, b, _) = pixel_at(&px, 100, 10);
    assert!(
        !(r == 0 && g == 0 && b == 0),
        "the darkened side of an inset black border must not be pure black, got ({r},{g},{b})"
    );
}

/// White edge case: luminance is above both thresholds, so the darkened side
/// uses the plain `Dark()` shade (171,171,171) and the lightened side is left
/// as-is — white can't get any lighter.
#[test]
fn white_inset_border_keeps_the_light_side_pure_white() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    el(
        &mut doc,
        body,
        "div",
        "width: 200px; height: 200px; margin: 0; border: 20px inset white;",
    );
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    assert_pixel(&px, 100, 10, (171, 171, 171), 2, "top (inset white)");
    assert_pixel(&px, 10, 100, (171, 171, 171), 2, "left (inset white)");
    assert_pixel(&px, 100, 190, (255, 255, 255), 1, "bottom (inset white)");
}

/// `currentcolor` (the `border-color` initial value) must resolve before
/// the bevel shading, not paint the shaded box's own `color` property
/// unresolved.
#[test]
fn currentcolor_resolves_before_shading() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    el(
        &mut doc,
        body,
        "div",
        "width: 200px; height: 200px; margin: 0; color: #888888; \
         border-width: 20px; border-style: inset;",
    );
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    // #888888 = rgb(136,136,136); computed independently against the same
    // formula: dark -> (52,52,52), light -> (221,221,221).
    assert_pixel(&px, 100, 10, (52, 52, 52), 2, "top (inset currentcolor)");
    assert_pixel(
        &px,
        100,
        190,
        (221, 221, 221),
        2,
        "bottom (inset currentcolor)",
    );
}

/// Mixed per-side styles (valid CSS: each side's `border-*-style` is
/// independent) exercise the straight-line fallback picking the right
/// `bevel::Side` for each edge rather than one shared value.
#[test]
fn mixed_per_side_styles_use_the_correct_side_for_each_edge() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    el(
        &mut doc,
        body,
        "div",
        "width: 200px; height: 200px; margin: 0; border-color: gray; \
         border-top-width: 20px; border-top-style: inset; \
         border-right-width: 20px; border-right-style: outset; \
         border-bottom-width: 20px; border-bottom-style: solid; \
         border-left-width: 20px; border-left-style: inset;",
    );
    doc.resolve_layout(VW, VH);
    let px = rasterize(&mut doc);

    const DARK: (u8, u8, u8) = (44, 44, 44);
    const LIGHT: (u8, u8, u8) = (212, 212, 212);
    const FLAT: (u8, u8, u8) = (128, 128, 128);

    assert_pixel(&px, 100, 10, DARK, 2, "top (inset)");
    assert_pixel(&px, 10, 100, DARK, 2, "left (inset)");
    assert_pixel(&px, 190, 100, DARK, 2, "right (outset darkens right)");
    assert_pixel(&px, 100, 190, FLAT, 2, "bottom (solid, untouched)");
    let _ = LIGHT; // documents the symmetric case tested elsewhere
}
