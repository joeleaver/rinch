//! #2368 on the GPU painter: at 1.25x and 1.5x a box's edges, its borders
//! and its `overflow` clip rect are on whole device pixels, as Chrome 158
//! draws them, and a translated box's edge blends as Chrome's does. The scene
//! and Chrome's pixels are shared with the software painter's
//! `rinch-dom/tests/fractional_scale_box_snap_2368_tests.rs`, whose doc says
//! what each box pins.
//!
//! The `opacity: 0.5` box is this painter's own case: Vello clips a group
//! layer to its bounds, anti-aliased, so bounds taken from the unsnapped box
//! cut the snapped background's edge column back to a blend (the software
//! painter's layers are whole pixels, so it cannot tell).
//!
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_box_snap_2368 -- --ignored`.

#![cfg(feature = "gpu-renderer")]

#[path = "support/vello_pixels.rs"]
mod pixels;
#[path = "../../rinch-dom/tests/support/box_snap_2368.rs"]
mod scene;

use peniko::Brush;
use rinch_dom::paint::vello_painter::VelloPainter;

/// The scene painted by Vello at `scale`: the green channel, one byte a
/// device pixel, and the row width.
fn render(scale: f64) -> (Vec<u8>, usize) {
    let mut doc = scene::document(scale);
    let (w, h) = scene::surface(scale);
    let mut painter = VelloPainter::new();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (scene::VW, scene::VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let px = pixels::render(painter.scene(), w, h);
    (px.iter().map(|p| p[1]).collect(), w as usize)
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_box_edges_and_clips_are_on_device_pixels() {
    for (scale, chrome) in [(1.25, &scene::CHROME_125), (1.5, &scene::CHROME_15)] {
        let (g, w) = render(scale);
        let wrong = scene::mismatches(&g, w, scale, chrome);
        assert!(wrong.is_empty(), "at {scale}x:\n{}", wrong.join("\n"));
    }
}
