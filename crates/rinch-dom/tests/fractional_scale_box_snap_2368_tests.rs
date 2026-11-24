//! #2368: at a fractional scale factor a box is drawn on whole device pixels.
//!
//! Chrome rounds every edge of a box's border box — and the clip rect derived
//! from it — to the nearest device pixel (`ToPixelSnappedRect`), so at 125% a
//! box at `left: 13px` (device x 16.25) has a hard edge at 16. rinch drew the
//! fractional edge, a one-pixel blend: 63 where Chrome has 0, and a clip's
//! blended edge multiplied by its child's (143 against 255, #2321).
//!
//! The oracle is Chrome 158 itself (`--headless=new --disable-gpu
//! --force-device-scale-factor=1.25` and `1.5`, this scene as HTML), read at
//! the same device pixels: a row of pixels across each box's left and right
//! edges and a column across its top and bottom, for twelve boxes — a
//! background; a 1px and a 3px border; an `overflow: hidden` clip, square,
//! rounded, inside a 3px border, and nested in another clip; a box inside a
//! padded box (snapped from its own accumulated position, 17.5 → 18, not from
//! its parent's snapped one, 16 + 1.25 → 17); `translateX(0.4px)` on the box
//! and on its parent (snapped first, then moved half a pixel, so the edge
//! blends as Chrome's does: a transform is not snapped); `scale(1.1)`; and an
//! `opacity: 0.5` box (whose group layer must hold the snapped box).
//!
//! Off the fixed point: every box is at 13px, which is 16.25 device px at
//! 1.25x and a tie (19.5) at 1.5x, and the parent and child cases put the two
//! on different fractions. At 1x every edge is already whole, so nothing here
//! would tell snapping from not snapping there. No text, so no fonts.

#![cfg(feature = "software-renderer")]

#[path = "support/box_snap_2368.rs"]
mod scene;

use rinch_dom::paint::skia_painter::TinySkiaPainter;

/// The scene painted by the software painter at `scale`: the green channel
/// (the scene is gray), one byte a device pixel, and the row width.
fn paint(scale: f64) -> (Vec<u8>, usize) {
    let mut doc = scene::document(scale);
    let (w, h) = scene::surface(scale);
    let mut painter = TinySkiaPainter::new(w, h);
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (scene::VW, scene::VH),
        &mut doc.font_cx,
        &mut cx,
    );
    let gray = painter
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| p[1])
        .collect();
    (gray, w as usize)
}

fn matches_chrome(scale: f64, chrome: &[scene::Probe]) {
    let (g, w) = paint(scale);
    let wrong = scene::mismatches(&g, w, scale, chrome);
    assert!(wrong.is_empty(), "at {scale}x:\n{}", wrong.join("\n"));
}

#[test]
fn box_edges_and_clips_are_on_device_pixels_at_125_percent() {
    matches_chrome(1.25, &scene::CHROME_125);
}

#[test]
fn box_edges_and_clips_are_on_device_pixels_at_150_percent() {
    matches_chrome(1.5, &scene::CHROME_15);
}
