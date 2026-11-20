//! #2719 on the GPU painter, rendered by Vello (lavapipe in CI):
//! `background-clip: text`, `-webkit-text-fill-color`, `-webkit-text-stroke`
//! and `paint-order` against Chrome's screenshots of the same page, region by
//! region, as `crates/rinch-dom/tests/text_clip_stroke_2719_tests.rs` holds
//! the software painter (whose header says what the page, the fixtures and
//! the comparison are).
//!
//! The clip is a `destination-in` layer over an isolated one, and a stroke a
//! `Scene::stroke` of the glyphs' outlines, so these pin that Vello draws
//! both as tiny-skia does: every region within the same tolerances of
//! Chrome's pixels, at 1x and 2x.
//!
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_text_clip_stroke_2719 -- --ignored`.
#![cfg(feature = "gpu-renderer")]

#[path = "support/vello_pixels.rs"]
mod pixels;
#[path = "../../rinch-dom/tests/support/text_clip_stroke_2719.rs"]
mod twin;

use rinch_dom::paint::vello_painter::VelloPainter;
use twin::*;

fn chrome(bytes: &[u8]) -> Image {
    let mut reader = png::Decoder::new(bytes).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    Image {
        px: buf[..info.buffer_size()].as_chunks::<3>().0.to_vec(),
        w: info.width,
        h: info.height,
    }
}

fn render(scale: u32) -> Image {
    let mut d = doc(&page_body(), W as f32, H as f32);
    let mut painter = VelloPainter::new();
    let mut layout_cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &d.tree,
        &mut painter,
        scale as f64,
        (W as f32, H as f32),
        &mut d.font_cx,
        &mut layout_cx,
    );
    let (pw, ph) = (W * scale, H * scale);
    Image::from_rgba(&pixels::render(painter.scene(), pw, ph), pw, ph)
}

#[test]
#[ignore = "needs a GPU adapter"]
fn every_region_matches_chrome_on_the_gpu() {
    for (scale, png) in [(1, CHROME_1X), (2, CHROME_2X)] {
        let bad = unlike_chrome(&render(scale), &chrome(png), scale, &ALL);
        assert!(bad.is_empty(), "regions unlike Chrome at {scale}x: {bad:?}");
    }
}
