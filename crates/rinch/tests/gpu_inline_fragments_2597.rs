//! #2597 and #2607 on the GPU painter: the scene and checks of
//! `rinch-dom/tests/inline_fragments_2597_tests.rs` (shared through
//! `rinch-dom/tests/support/inline_fragments_2597.rs`), painted by
//! `VelloPainter` and rendered by Vello (Area).
//!
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_inline_fragments_2597 -- --ignored`.
#![cfg(feature = "gpu")]

#[path = "support/vello_pixels.rs"]
mod pixels;
#[path = "../../rinch-dom/tests/support/inline_fragments_2597.rs"]
mod scene;

use peniko::Brush;
use rinch_dom::RinchDocument;
use rinch_dom::paint::vello_painter::VelloPainter;

/// `doc` painted by Vello over white at `scale` on a `w` x `h` CSS px viewport.
fn render(doc: &mut RinchDocument, (w, h): (u32, u32), scale: f64) -> (u32, Vec<[u8; 4]>) {
    let (pw, ph) = (
        (w as f64 * scale).round() as u32,
        (h as f64 * scale).round() as u32,
    );
    let mut painter = VelloPainter::new();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (w as f32, h as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    (pw, pixels::render(painter.scene(), pw, ph))
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_inline_fragments_are_drawn_as_chrome_draws_them() {
    let mut doc = scene::document();
    for scale in [1.0, 2.0] {
        let (pw, px) = render(&mut doc, (scene::W, scene::H), scale);
        let bad = scene::misses(&px, pw, scale);
        assert!(bad.is_empty(), "#2597/#2607:\n{}", bad.join("\n"));
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_solid_sides_end_on_whole_pixels_without_seams() {
    use scene::seams::*;
    let mut doc = document();
    for scale in [1.0, 1.5] {
        let (pw, px) = render(&mut doc, (W, H), scale);
        let bad = misses(&px, pw, scale);
        assert!(bad.is_empty(), "seams at {scale}x:\n{}", bad.join("\n"));
    }
}
