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
//! **The colours here are rinch's, not Chrome's.** Chrome paints `inset` as a
//! bevel (154 over 238 at scale 1); rinch collapses every bevelled border style
//! to `solid` in `border_style_from_stylo`, so both rows are the flat 128. See
//! the comment on that assertion.

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

/// `(x0, y0, x1, y1)` inclusive bounding box of the pixels `near_count` counts.
fn near_bbox(px: &[u8], rgb: (u8, u8, u8), tol: i32) -> Option<(u32, u32, u32, u32)> {
    let w = VW as u32;
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut any = false;
    for i in (0..px.len()).step_by(4) {
        let d = |a: u8, b: u8| (a as i32 - b as i32).abs();
        if px[i + 3] > 250
            && d(px[i], rgb.0) <= tol
            && d(px[i + 1], rgb.1) <= tol
            && d(px[i + 2], rgb.2) <= tol
        {
            let p = (i / 4) as u32;
            let (x, y) = (p % w, p / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            any = true;
        }
    }
    any.then_some((x0, y0, x1, y1))
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

    let n = near_count(&px, GRAY, 2);
    assert!(
        n > 0,
        "a bare <hr> must paint grey ink; found none (this is the assertion that \
         fails at HEAD, where the UA `* {{ border-width: 0 }}` reset left <hr> \
         borderless)"
    );

    let (x0, y0, x1, y1) = near_bbox(&px, GRAY, 2).expect("counted grey but found no bbox");
    assert_eq!(
        (y0, y1),
        (10, 11),
        "the rule sits at its 0.5em = 10px margin and is 2 rows tall (two 1px \
         borders around a `height: 0` box), got rows {y0}..={y1}"
    );
    assert!(
        x0 <= 1 && x1 >= 798,
        "the rule spans its container's full width, got columns {x0}..={x1}"
    );

    // The whole surface, row by row: exactly two rows of exactly 800 opaque
    // pixels and nothing anywhere else. Six of those 1600 are the antialiased
    // corner caps where the horizontal and vertical borders meet, which is why
    // the exact-grey count is 1594 rather than 1600 and why the bbox above
    // allows one column of slack at each end.
    assert_eq!(
        ink_rows(&px),
        [(10u32, 800u32), (11, 800)].into_iter().collect(),
        "the <hr> must be the only thing painted, as two solid 800px rows"
    );
    assert_eq!(ink_count(&px), 1600);
    // **This pins rinch's rendering of `inset`, not Chrome's.** Chrome paints a
    // bevel — measured at scale 1, the top row is (154, 154, 154) and the bottom
    // (238, 238, 238), so not one pixel of a default `<hr>` is actually `gray`.
    // rinch paints the flat `solid` row twice because
    // `computed_style/from_stylo/box_model.rs`'s `border_style_from_stylo` ends
    // `_ => BorderStyleValue::Solid`, collapsing groove/ridge/inset/outset. That
    // mapping is pre-existing and #674 is only what makes it visible, since an
    // `<hr>` painted nothing at all before. If rinch ever grows a bevel painter
    // this assertion is the one it will hit, and 154 over 238 is the target.
    assert!(
        (1590..=1600).contains(&n),
        "almost all of that ink is exactly gray; got {n} of 1600"
    );
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
    let n = near_count(&px, (0, 128, 0), 2);
    assert!(
        (1590..=1600).contains(&n),
        "…and must be painted in that colour instead; got {n} green pixels"
    );
    assert_eq!(
        ink_rows(&px),
        [(10u32, 800u32), (11, 800)].into_iter().collect(),
        "in the same place and at the same size as the grey one"
    );
}
