//! #2719 (item 3, `conic-gradient()` and `repeating-conic-gradient()`) on
//! the GPU painter: the scene of `rinch-dom/tests/conic_gradient_2719_tests.rs`
//! (shared through `rinch-dom/tests/support/conic_gradient_2719.rs`), painted
//! by `VelloPainter` as a peniko sweep and rendered by Vello, against Chrome
//! 157's pixels. Vello looks a sweep's colour up in a 512-texel ramp, rounded
//! to the nearest texel, where Chrome's Skia interpolates exactly; measured on
//! lavapipe, 139 of the 505 samples are one level off and 2 are two, none
//! more. [`TOL`] is 3.
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_conic_gradient_2719 -- --ignored`.
#![cfg(feature = "gpu-renderer")]

#[path = "support/vello_pixels.rs"]
mod pixels;
#[path = "../../rinch-dom/tests/support/conic_gradient_2719.rs"]
mod scene;

use peniko::Brush;
use rinch_dom::paint::vello_painter::VelloPainter;
use scene::{H, W};

/// Per channel, against Chrome (see the module docs).
const TOL: u8 = 3;

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_conic_gradients_match_chrome() {
    for (b, n) in scene::samples_per_box().iter().enumerate() {
        assert!(*n >= 6, "box {b} has only {n} of Chrome's samples");
    }
    let doc = &mut scene::document().0;
    let mut painter = VelloPainter::new();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let px = pixels::render(painter.scene(), W, H);
    let bad = scene::misses(&px, TOL);
    assert!(
        bad.is_empty(),
        "{} of {} samples differ from Chrome:\n{}",
        bad.len(),
        scene::CHROME.len(),
        bad.join("\n")
    );
}
