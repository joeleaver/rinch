//! #2344: a uniform 1px border on the software painter (tiny-skia) at scale
//! 1 drew its **left** edge blended across two pixels, half into the page.
//! Every stroke one device pixel wide or less went to tiny-skia's
//! anti-aliased *hairline* scan converter (`treat_as_hairline` in
//! `stroke_path`), whose coverage is not the stroke's area; on a closed rect
//! it smeared the closing (left) segment. A square uniform border and a
//! square outline are both such strokes. Chrome 156 and Vello draw all four
//! edges as solid one-pixel lines.
//!
//! The scene and its pixels are in `support/border_edges_2344.rs`, shared
//! with `gpu_border_edges_2344` (in `rinch`), which holds the GPU painter to
//! the same pixels.

#![cfg(feature = "software-renderer")]

#[path = "support/border_edges_2344.rs"]
mod scene;

use rinch_dom::paint::skia_painter::TinySkiaPainter;
use scene::{H, W};

fn check(scale: u32) {
    let doc = &mut scene::document();
    let (pw, ph) = (W * scale, H * scale);
    let mut painter = TinySkiaPainter::new(pw, ph);
    painter.fill_white();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale as f64,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    let px = painter.pixels().as_chunks::<4>().0;
    let bad = scene::misses(px, pw, scale);
    assert!(
        bad.is_empty(),
        "scale {scale}: (device point, tiny-skia, chrome): {bad:?}"
    );
    let (dashes, gaps) = scene::dashes_and_gaps(px, pw, scale);
    assert!(
        dashes > 0 && gaps > 0,
        "scale {scale}: the dashed border, {dashes} dash and {gaps} gap pixels"
    );
}

#[test]
fn a_uniform_1px_border_is_solid_on_every_side_at_scale_1() {
    check(1);
}

#[test]
fn a_uniform_1px_border_is_solid_on_every_side_at_scale_2() {
    check(2);
}
