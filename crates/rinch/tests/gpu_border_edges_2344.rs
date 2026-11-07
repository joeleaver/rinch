//! #2344 on the GPU painter: the scene of
//! `rinch-dom/tests/border_edges_2344_tests.rs` (shared through
//! `rinch-dom/tests/support/border_edges_2344.rs`), painted by `VelloPainter`
//! and rendered by Vello, at 1x and 2x. A uniform 1px border and a 1px
//! outline are solid one-pixel lines on every side, as Chrome 156 draws
//! them; Vello always did this, and the software painter now does too.
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_border_edges_2344 -- --ignored`.
#![cfg(feature = "gpu")]

#[path = "support/vello_pixels.rs"]
mod pixels;
#[path = "../../rinch-dom/tests/support/border_edges_2344.rs"]
mod scene;

use peniko::Brush;
use rinch_dom::paint::vello_painter::VelloPainter;
use scene::{H, W};

fn render(scale: u32) -> (u32, Vec<[u8; 4]>) {
    let (pw, ph) = (W * scale, H * scale);
    let doc = &mut scene::document();
    let mut painter = VelloPainter::new();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale as f64,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    (pw, pixels::render(painter.scene(), pw, ph))
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_a_uniform_1px_border_is_solid_on_every_side() {
    for scale in [1, 2] {
        let (pw, px) = render(scale);
        let bad = scene::misses(&px, pw, scale);
        assert!(
            bad.is_empty(),
            "scale {scale}: (device point, vello, chrome): {bad:?}"
        );
        let (dashes, gaps) = scene::dashes_and_gaps(&px, pw, scale);
        assert!(
            dashes > 0 && gaps > 0,
            "scale {scale}: the dashed border, {dashes} dash and {gaps} gap pixels"
        );
    }
}
