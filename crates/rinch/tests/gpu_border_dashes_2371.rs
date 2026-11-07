//! #2371 and #2372 on the GPU painter: the scene of
//! `rinch-dom/tests/border_dashes_2371_tests.rs` (shared through
//! `rinch-dom/tests/support/border_dashes_2371.rs`), painted by `VelloPainter`
//! and rendered by Vello, at 1x and 2x, against Chrome 156's pixels.
//! Dashed and dotted borders are laid out as Chrome lays them out (dashes
//! fitted so that each side's corners are solid, square dots under 3 device
//! px, round dots above), and both painters draw the same geometry.
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_border_dashes_2371 -- --ignored`.
#![cfg(feature = "gpu")]

#[path = "support/vello_pixels.rs"]
mod pixels;
#[path = "../../rinch-dom/tests/support/border_dashes_2371.rs"]
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
fn gpu_dashed_and_dotted_borders_match_chrome() {
    for scale in [1, 2] {
        let (pw, px) = render(scale);
        let bad = scene::misses(&px, pw, scale);
        assert!(
            bad.is_empty(),
            "{} rows differ from Chrome:\n{}",
            bad.len(),
            bad.join("\n")
        );
        let bad = scene::corner_misses(&px, pw, scale);
        assert!(bad.is_empty(), "per-side corners:\n{}", bad.join("\n"));
    }
}
