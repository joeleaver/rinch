//! #1007: a partial repaint paints what the full repaint of the same frame
//! paints, byte for byte, for a box inside the damage.
//!
//! tiny-skia draws an opaque `SourceOver` fill through one pipeline with no
//! mask (reduced to `Source`, edge pixels lerped) and another through a mask
//! (colour scaled by coverage, blended over), and the two round an
//! anti-aliased edge pixel differently by one LSB. A full repaint draws a box
//! outside every clip with no mask; the partial repaint drew it through the
//! damage mask. `TinySkiaPainter::push_damage_clip` is the damage clip that
//! knows the difference.
//!
//! Every fixture draws its edges over a non-transparent, non-uniform backdrop
//! and at fractional radii: over a transparent backdrop the two pipelines
//! agree (`d = 0`), which is why `r2d-round-op` — an opacity layer — never
//! showed the bug.

#![cfg(feature = "software-renderer")]

use peniko::kurbo::{Affine, Rect, RoundedRect, Stroke};
use peniko::{Brush, Color, Fill};
use rinch_dom::paint::DamageRegion;
use rinch_dom::paint::painter::{BlendMode, PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const W: u32 = 120;
const H: u32 = 90;

/// A backdrop with a different colour in every pixel, so an edge pixel's
/// rounding cannot sit on a value both pipelines map to the same byte.
fn backdrop(p: &mut TinySkiaPainter) {
    for y in 0..H {
        for x in 0..W {
            let c = Color::from_rgba8(
                (37 + x * 3) as u8,
                (200 - y * 2) as u8,
                ((x * 7 + y * 5) % 251) as u8,
                255,
            );
            p.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                &Brush::Solid(c),
                &PaintShape::Rect(Rect::new(
                    x as f64,
                    y as f64,
                    x as f64 + 1.0,
                    y as f64 + 1.0,
                )),
            );
        }
    }
}

fn card(x0: f64, y0: f64, x1: f64, y1: f64) -> PaintShape {
    PaintShape::RoundedRect(RoundedRect::new(x0, y0, x1, y1, 9.3))
}

fn opaque(r: u8, g: u8, b: u8) -> Brush {
    Brush::Solid(Color::from_rgba8(r, g, b, 255))
}

fn region(rects: &[Rect]) -> DamageRegion {
    let mut d = DamageRegion::new(W as f64, H as f64);
    for r in rects {
        d.add(*r);
    }
    assert_eq!(d.rects().len(), rects.len(), "the rects stay apart");
    d
}

fn diffs(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| x != y).count()
}

/// Paint `draw` over the backdrop twice — once as a full repaint (no damage
/// clip), once inside `damage` — and count the pixels that differ.
fn partial_vs_full(damage: &[Rect], draw: impl Fn(&mut TinySkiaPainter)) -> usize {
    let mut full = TinySkiaPainter::new(W, H);
    backdrop(&mut full);
    draw(&mut full);

    let mut partial = TinySkiaPainter::new(W, H);
    backdrop(&mut partial);
    partial.push_damage_clip(&region(damage));
    draw(&mut partial);
    partial.pop_layer();

    diffs(full.pixels(), partial.pixels())
}

/// Positive control for every fixture below: a plain `push_clip` of the same
/// rect — what `build_pixels` did before — does move edge pixels. Without it,
/// a pass could mean the fixture draws no edge the pipelines disagree on.
#[test]
fn a_plain_clip_moves_the_edge_pixels_this_suite_measures() {
    let damage = Rect::new(10.0, 10.0, 70.0, 60.0);
    let mut full = TinySkiaPainter::new(W, H);
    backdrop(&mut full);
    full.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        &opaque(250, 250, 250),
        &card(10.0, 10.0, 70.0, 60.0),
    );

    let mut partial = TinySkiaPainter::new(W, H);
    backdrop(&mut partial);
    partial.push_clip(Fill::NonZero, Affine::IDENTITY, &PaintShape::Rect(damage));
    partial.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        &opaque(250, 250, 250),
        &card(10.0, 10.0, 70.0, 60.0),
    );
    partial.pop_layer();

    assert!(diffs(full.pixels(), partial.pixels()) > 10);
}

/// The issue's shape: a rounded box exactly as big as the damage rect (a
/// frame damages the viewport's box, which is the card's).
#[test]
fn a_rounded_box_filling_the_damage_rect_paints_as_the_full_repaint_does() {
    let n = partial_vs_full(&[Rect::new(10.0, 10.0, 70.0, 60.0)], |p| {
        p.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &opaque(250, 250, 250),
            &card(10.0, 10.0, 70.0, 60.0),
        );
    });
    assert_eq!(n, 0);
}

/// Inside the second of two damage rects: every rect is asked, not the first.
#[test]
fn a_box_inside_any_damage_rect_paints_as_the_full_repaint_does() {
    let damage = [
        Rect::new(2.0, 2.0, 20.0, 20.0),
        Rect::new(40.0, 30.0, 110.0, 85.0),
    ];
    let n = partial_vs_full(&damage, |p| {
        p.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &opaque(20, 90, 240),
            &card(45.5, 33.25, 104.75, 80.5),
        );
        p.stroke(
            &Stroke::new(2.5),
            Affine::IDENTITY,
            &opaque(240, 30, 60),
            &card(52.0, 40.0, 98.0, 74.0),
        );
    });
    assert_eq!(n, 0);
}

/// An opacity layer inside the damage is drawn unmasked by the full repaint
/// too, so it does not end the rule. The second fill lands on the first, so
/// the layer's backdrop is not transparent where it matters.
#[test]
fn a_box_inside_an_opacity_layer_in_the_damage_paints_as_the_full_repaint_does() {
    let damage = Rect::new(10.0, 10.0, 90.0, 80.0);
    let n = partial_vs_full(&[damage], |p| {
        p.push_layer(
            BlendMode::Normal,
            0.6,
            Affine::IDENTITY,
            &PaintShape::Rect(damage),
        );
        p.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &opaque(200, 40, 40),
            &PaintShape::Rect(damage),
        );
        p.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &opaque(30, 220, 90),
            &card(20.5, 18.0, 80.0, 70.5),
        );
        p.pop_layer();
    });
    assert_eq!(n, 0);
}

/// A clip opened inside the damage masks what is drawn under it in the full
/// repaint, so it must here too — even when `push_clip` skips it as covering
/// the whole damage (#907) and leaves the damage mask in force.
#[test]
fn a_clip_covering_the_damage_keeps_its_boxes_masked_as_the_full_repaint_does() {
    let damage = Rect::new(20.0, 15.0, 80.0, 70.0);
    let n = partial_vs_full(&[damage], |p| {
        // Covers the damage with a margin, so its own fully covered rect
        // does too: the skipped case.
        p.push_clip(
            Fill::NonZero,
            Affine::IDENTITY,
            &PaintShape::Rect(Rect::new(5.0, 5.0, 100.0, 85.0)),
        );
        p.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &opaque(250, 250, 250),
            &card(25.0, 20.0, 75.0, 65.0),
        );
        p.pop_layer();
        // And after its pop the rule is back.
        p.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &opaque(10, 10, 10),
            &card(30.5, 25.5, 70.0, 60.0),
        );
    });
    assert_eq!(n, 0);
}

/// A box that reaches past the damage is still clipped to it: the pixels
/// outside are the ones the frame on screen already has. Its right edge ends
/// a fraction past the damage, so a box test that rounds that edge down
/// instead of up would draw one column outside.
#[test]
fn a_box_reaching_past_the_damage_is_still_clipped_to_it() {
    let damage = Rect::new(10.0, 10.0, 60.0, 50.0);
    let mut before = TinySkiaPainter::new(W, H);
    backdrop(&mut before);

    let mut p = TinySkiaPainter::new(W, H);
    backdrop(&mut p);
    p.push_damage_clip(&region(&[damage]));
    p.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        &opaque(0, 0, 0),
        &card(12.0, 12.0, 60.4, 48.0),
    );
    p.pop_layer();

    let (a, b) = (before.pixels(), p.pixels());
    for y in 0..H {
        for x in 0..W {
            let inside = (10..60).contains(&x) && (10..50).contains(&y);
            let i = ((y * W + x) * 4) as usize;
            if !inside {
                assert_eq!(
                    a[i..i + 4],
                    b[i..i + 4],
                    "({x}, {y}) outside the damage changed"
                );
            }
        }
    }
    // Positive control: the box did paint inside.
    let i = ((30 * W + 30) * 4) as usize;
    assert_eq!(&b[i..i + 4], &[0, 0, 0, 255]);
}
