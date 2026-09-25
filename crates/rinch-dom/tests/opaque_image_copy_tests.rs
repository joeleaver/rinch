//! #361: the software painter copies an **opaque** image straight into the
//! surface when the draw is one a copy reproduces exactly — no rotation or
//! skew, a positive scale, whole-pixel destination edges, and no partial clip —
//! instead of premultiplying it and sampling it through `draw_pixmap`.
//!
//! The oracle is the painter in reference mode, which never takes the copy:
//! every draw below must come out **byte for byte** what `draw_pixmap` draws,
//! over a background of noise (so a pixel the copy writes that the draw would
//! not, or misses, shows), and every draw the copy is meant to take is checked
//! to have taken it (`opaque_image_copies`), so a copy that silently declined
//! everything cannot pass by agreeing with the reference.

#![cfg(feature = "software-renderer")]

use peniko::Fill;
use peniko::kurbo::{Affine, Rect, RoundedRect};
use rinch_dom::paint::painter::{PaintImage, PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const SW: u32 = 160;
const SH: u32 = 120;

/// Deterministic opaque pixels with no two neighbours alike, so a sample taken
/// one column or row off shows.
fn opaque_image(w: u32, h: u32, seed: u32) -> Vec<u8> {
    (0..w * h)
        .flat_map(|i| {
            let v = i.wrapping_mul(2_654_435_761).wrapping_add(seed);
            [(v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8, 255]
        })
        .collect()
}

/// A painter whose surface is noise, laid down through the ordinary (not
/// opaque) image path so both painters start identical.
fn painter(reference: bool) -> TinySkiaPainter {
    let mut p = TinySkiaPainter::new(SW, SH);
    p.set_reference_mode(reference);
    let noise = opaque_image(SW, SH, 7);
    p.draw_image(
        &PaintImage {
            data: &noise,
            width: SW,
            height: SH,
            decoded: None,
            opaque: false,
        },
        Affine::IDENTITY,
    );
    p
}

enum Clip {
    None,
    /// A rect clip covering the whole draw: the copy still applies.
    Covering,
    /// A whole-pixel rect clip cutting the draw: every mask byte is 0 or 255,
    /// so the copy writes exactly the pixels the mask lets through.
    Cutting,
    /// A rect clip with fractional edges: partial bytes along them, which only
    /// `draw_pixmap` can blend, so the copy must decline.
    Fractional,
    /// A rounded clip whose corner crosses the draw: partial along the arc.
    Rounded,
}

/// Draw `w x h` opaque pixels under `ts` into both painters and compare.
/// Returns how many draws the fast painter copied.
fn check(label: &str, w: u32, h: u32, ts: Affine, clip: Clip) -> u64 {
    let img = opaque_image(w, h, 99);
    let mut out = Vec::new();
    for reference in [true, false] {
        let mut p = painter(reference);
        let before = p.stats().opaque_image_copies;
        let premultiplied = p.stats().image_premultiplies;
        let shape: Option<PaintShape> = match clip {
            Clip::None => None,
            Clip::Covering => Some(Rect::new(-20.0, -20.0, 300.0, 300.0).into()),
            Clip::Cutting => Some(Rect::new(30.0, 25.0, 90.0, 70.0).into()),
            Clip::Fractional => Some(Rect::new(30.4, 25.0, 90.0, 70.6).into()),
            Clip::Rounded => {
                Some(RoundedRect::from_rect(Rect::new(20.0, 12.0, 150.0, 110.0), 30.0).into())
            }
        };
        if let Some(s) = &shape {
            p.push_clip(Fill::NonZero, Affine::IDENTITY, s);
        }
        p.draw_image(
            &PaintImage {
                data: &img,
                width: w,
                height: h,
                decoded: None,
                opaque: true,
            },
            ts,
        );
        if shape.is_some() {
            p.pop_layer();
        }
        if !reference {
            // Copied or not, opaque pixels are their own premultiplied form.
            assert_eq!(
                p.stats().image_premultiplies,
                premultiplied,
                "{label}: an opaque image is never premultiplied"
            );
        }
        out.push((p.pixels().to_vec(), p.stats().opaque_image_copies - before));
    }
    let (reference, copied) = (&out[0].0, &out[1]);
    let diff = reference
        .chunks(4)
        .zip(copied.0.chunks(4))
        .position(|(a, b)| a != b);
    assert!(
        diff.is_none(),
        "{label}: the copy differs from draw_pixmap at pixel ({}, {})",
        diff.unwrap() as u32 % SW,
        diff.unwrap() as u32 / SW,
    );
    copied.1
}

#[test]
fn an_unscaled_whole_pixel_draw_is_copied_exactly() {
    for (x, y) in [
        (0.0, 0.0),
        (17.0, 9.0),
        (-13.0, -7.0),
        (120.0, 100.0),
        (150.0, -30.0),
    ] {
        let label = format!("scale 1 at ({x}, {y})");
        assert_eq!(
            check(&label, 64, 48, Affine::translate((x, y)), Clip::None),
            1,
            "{label}"
        );
    }
}

#[test]
fn a_scaled_draw_with_whole_pixel_edges_is_copied_exactly() {
    // (image w, h, scale x, scale y, offset): destination edges all whole
    // pixels. Down-scaling by a half and a quarter (a 1080p frame into a
    // smaller viewport), up-scaling (a low-resolution game), and scales that
    // are not a power of two, where the sampling arithmetic is the whole
    // question.
    for (w, h, sx, sy, (x, y)) in [
        (192u32, 108u32, 0.5, 0.5, (3.0, 5.0)),
        (320, 180, 0.25, 0.25, (10.0, 20.0)),
        (32, 18, 2.0, 2.0, (0.0, 0.0)),
        (30, 20, 3.0, 3.0, (-11.0, 4.0)),
        (3, 5, 7.0 / 3.0, 11.0 / 5.0, (21.0, 13.0)),
        (150, 90, 0.6, 0.6, (1.0, 2.0)),
        (100, 60, 1.5, 1.5, (4.0, 6.0)),
        (96, 54, 1.25, 1.0, (0.0, 30.0)),
    ] {
        let label = format!("{w}x{h} scaled {sx}x{sy} at ({x}, {y})");
        let ts = Affine::translate((x, y)) * Affine::scale_non_uniform(sx, sy);
        assert_eq!(check(&label, w, h, ts, Clip::None), 1, "{label}");
    }
}

#[test]
fn a_whole_pixel_clip_keeps_the_copy() {
    let at = Affine::translate((17.0, 9.0));
    assert_eq!(check("covering clip", 64, 48, at, Clip::Covering), 1);
    assert_eq!(check("cutting clip", 64, 48, at, Clip::Cutting), 1);
    let scaled = at * Affine::scale(2.0);
    assert_eq!(
        check("cutting clip, scaled", 32, 24, scaled, Clip::Cutting),
        1
    );
}

/// Every draw the copy cannot reproduce is left to `draw_pixmap` — and still
/// comes out identical, because it *is* `draw_pixmap`.
#[test]
fn draws_a_copy_cannot_reproduce_are_declined() {
    let at = Affine::translate((17.0, 9.0));
    for (label, ts, clip) in [
        ("a clip with fractional edges", at, Clip::Fractional),
        ("a rounded clip", at, Clip::Rounded),
        (
            "a fractional offset",
            Affine::translate((17.4, 9.0)),
            Clip::None,
        ),
        (
            "a scale leaving a fractional edge",
            at * Affine::scale(0.7),
            Clip::None,
        ),
        ("a rotation", at * Affine::rotate(0.3), Clip::None),
        (
            "a mirror",
            Affine::translate((100.0, 9.0)) * Affine::scale_non_uniform(-1.0, 1.0),
            Clip::None,
        ),
    ] {
        assert_eq!(check(label, 64, 48, ts, clip), 0, "{label}: declined");
    }
}

/// A non-opaque image never takes the copy, whatever the draw.
#[test]
fn an_image_not_vouched_opaque_is_never_copied() {
    let img = opaque_image(64, 48, 3);
    let mut p = painter(false);
    let before = p.stats().opaque_image_copies;
    p.draw_image(
        &PaintImage {
            data: &img,
            width: 64,
            height: 48,
            decoded: None,
            opaque: false,
        },
        Affine::translate((5.0, 5.0)),
    );
    assert_eq!(p.stats().opaque_image_copies, before);
}

/// Inside an opacity layer the copy writes the layer, which is then
/// composited like any other draw — and records the area it touched, or the
/// layer would composite (and clear) too little of it.
#[test]
fn a_copy_inside_an_opacity_layer_is_composited_like_a_draw() {
    use rinch_dom::paint::painter::BlendMode;
    let img = opaque_image(64, 48, 5);
    let mut out = Vec::new();
    for reference in [true, false] {
        let mut p = painter(reference);
        let before = p.stats().opaque_image_copies;
        let bounds: PaintShape = Rect::new(0.0, 0.0, SW as f64, SH as f64).into();
        p.push_layer(BlendMode::Normal, 0.5, Affine::IDENTITY, &bounds);
        p.draw_image(
            &PaintImage {
                data: &img,
                width: 64,
                height: 48,
                decoded: None,
                opaque: true,
            },
            Affine::translate((40.0, 30.0)),
        );
        p.pop_layer();
        out.push((p.pixels().to_vec(), p.stats().opaque_image_copies - before));
    }
    assert_eq!(out[1].1, 1, "the fast painter copied");
    assert!(
        out[0].0 == out[1].0,
        "and composited exactly what draw_pixmap did"
    );
}
