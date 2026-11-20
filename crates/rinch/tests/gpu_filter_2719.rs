//! #2719 on the GPU painter: `filter: blur()` and `drop-shadow()` painted by
//! `VelloPainter` (the filtered box's raster drawn as an image, a
//! drop-shadow's content drawn by Vello itself) and rendered by Vello, against
//! Chrome 157's software raster of the same page, with the software painter's
//! tolerances (`crates/rinch-dom/tests/filter_blur_2719_tests.rs` says what
//! they are and why).
//!
//! Needs an adapter, so `#[ignore]`d: run with
//! `cargo test -p rinch --features gpu --test gpu_filter_2719 -- --ignored`.
#![cfg(feature = "gpu-renderer")]

#[path = "support/vello_pixels.rs"]
mod pixels;

#[path = "../../rinch-dom/tests/support/filter_2719_chrome.rs"]
mod chrome;

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::vello_painter::VelloPainter;

const W: f32 = 360.0;
const H: f32 = 260.0;

fn part<'a>(page: &'a str, open: &str, close: &str) -> &'a str {
    let start = page.find(open).unwrap() + open.len();
    &page[start..start + page[start..].find(close).unwrap()]
}

fn render(scale: f64) -> (u32, Vec<u8>) {
    let page = chrome::PAGE;
    let mut doc = RinchDocument::new();
    doc.load_css(part(page, "<style>", "</style>"));
    let body = doc.body();
    doc.set_inner_html(body, part(page, "<body>", "</body>"));
    doc.resolve_layout(W, H);
    let (pw, ph) = ((W as f64 * scale) as u32, (H as f64 * scale) as u32);
    let mut painter = VelloPainter::new();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (W, H),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let rgba = pixels::render(painter.scene(), pw, ph);
    (pw, rgba.into_iter().flatten().collect())
}

/// Chrome's RGB pixels at `scale`.
fn decode(scale: u32) -> Vec<[u8; 3]> {
    let mut reader = png::Decoder::new(chrome::chrome_png(scale))
        .read_info()
        .unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    buf.truncate((info.width * info.height * 3) as usize);
    buf.chunks(3).map(|p| [p[0], p[1], p[2]]).collect()
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_filters_match_chrome() {
    for scale in [1, 2] {
        let (w, px) = render(scale as f64);
        chrome::check_against_chrome(scale, w, &px, &decode(scale), chrome::CASES);
    }
}
