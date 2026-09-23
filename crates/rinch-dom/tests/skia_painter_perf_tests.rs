//! Software painter cost: timings (`#[ignore]`d) for five paint workloads.
//!
//! ```text
//! cargo test --release -p rinch-dom --test skia_painter_perf_tests -- --ignored --nocapture
//! ```

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use peniko::kurbo::{Affine, Rect};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

pub const VW: f32 = 1200.0;
pub const VH: f32 = 800.0;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

const LOREM: &str = "The quick brown fox jumps over the lazy dog while a sphinx of black \
    quartz judges my vow; pack my box with five dozen liquor jugs, then 0123456789.";

pub fn doc_with(css: &str, html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    doc.load_css(
        "body { margin: 0; font-family: ProbeFace, sans-serif; font-size: 14px; \
         line-height: 18px; color: rgb(30, 30, 40); }",
    );
    doc.load_css(css);
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(VW, VH);
    doc.resolve_layout(VW, VH);
    doc
}

pub fn text_page() -> RinchDocument {
    let mut html = String::new();
    for i in 0..40 {
        html.push_str(&format!("<p style=\"margin: 0 0 2px 0\">{i}: {LOREM}</p>"));
    }
    doc_with("", &html)
}

/// The text page with every glyph dropped at paint time (`visibility:
/// hidden` hides the text but keeps the layout and the walk): the part of the
/// text page's cost that is not drawing glyphs.
pub fn text_page_hidden() -> RinchDocument {
    let mut html = String::new();
    for i in 0..40 {
        html.push_str(&format!(
            "<p style=\"margin: 0 0 2px 0; visibility: hidden\">{i}: {LOREM}</p>"
        ));
    }
    doc_with("", &html)
}

pub fn scroller_page() -> RinchDocument {
    let mut html = String::from("<div style=\"display: flex; flex-wrap: wrap; gap: 6px\">");
    for i in 0..40 {
        html.push_str(
            "<div style=\"width: 140px; height: 140px; overflow: auto; border-radius: 8px; \
             background: rgb(240, 240, 250)\">",
        );
        for j in 0..10 {
            html.push_str(&format!("<div>row {i}.{j} fox</div>"));
        }
        html.push_str("</div>");
    }
    html.push_str("</div>");
    doc_with("", &html)
}

pub fn layer_page() -> RinchDocument {
    let mut html = String::from("<div style=\"display: flex; flex-wrap: wrap; gap: 6px\">");
    for i in 0..30 {
        html.push_str(&format!(
            "<div style=\"width: 180px; height: 120px; opacity: 0.6; \
             background: rgb(200, 80, 40)\">layer {i} {LOREM}</div>"
        ));
    }
    html.push_str("</div>");
    doc_with("", &html)
}

/// A 128x128 RGBA PNG with a translucent gradient, as a data URI.
pub fn png_data_uri(seed: u8) -> String {
    use base64::Engine;
    let mut img = image::RgbaImage::new(128, 128);
    for (x, y, p) in img.enumerate_pixels_mut() {
        *p = image::Rgba([
            (x as u8).wrapping_mul(2).wrapping_add(seed),
            (y as u8).wrapping_mul(2),
            seed,
            ((x + y) as u8) | 1,
        ]);
    }
    let mut bytes = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
    .unwrap();
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

pub fn image_page() -> RinchDocument {
    let uris: Vec<String> = (0..4).map(|s| png_data_uri(s * 40)).collect();
    let mut html = String::from("<div style=\"display: flex; flex-wrap: wrap; gap: 4px\">");
    for i in 0..60 {
        html.push_str(&format!(
            "<img src=\"{}\" style=\"width: 96px; height: 96px\">",
            uris[i % uris.len()]
        ));
    }
    html.push_str("</div>");
    doc_with("", &html)
}

pub fn full_paint(doc: &mut RinchDocument, painter: &mut TinySkiaPainter) {
    painter.reset();
    painter.fill_white();
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
}

/// What `RinchApp::build_pixels` does for a partial frame.
pub fn partial_paint(doc: &mut RinchDocument, painter: &mut TinySkiaPainter, region: Rect) {
    painter.clear_rect_white(
        region.x0 as u32,
        region.y0 as u32,
        region.width() as u32,
        region.height() as u32,
    );
    rinch_dom::paint::set_dirty_region(Some(region));
    painter.push_clip(
        peniko::Fill::NonZero,
        Affine::IDENTITY,
        &PaintShape::Rect(region),
    );
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.pop_layer();
    rinch_dom::paint::set_dirty_region(None);
}
fn time(label: &str, reps: usize, mut f: impl FnMut()) -> f64 {
    // Warm up once, so a cache (if any) is in its steady state.
    f();
    let mut samples = Vec::with_capacity(reps);
    for _ in 0..reps {
        let t = std::time::Instant::now();
        f();
        samples.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "{label:<40} best {:>8.3} ms   median {:>8.3} ms",
        samples[0],
        samples[reps / 2]
    );
    samples[0]
}

type Scenario = (&'static str, fn() -> RinchDocument, Option<Rect>);

/// Every scenario twice: with the painter as it was (reference mode: no glyph
/// cache, no pools, a full-surface buffer per clip and layer, a premultiply
/// per image draw) and as it is, plus the counters of one steady-state frame.
#[test]
#[ignore = "timing harness; run in release with --ignored --nocapture"]
fn skia_painter_timings() {
    const REPS: usize = 30;
    let partial = Some(Rect::new(100.0, 200.0, 300.0, 240.0));
    let scenarios: [Scenario; 7] = [
        ("full: text page", text_page, None),
        ("full: text page, glyphs hidden", text_page_hidden, None),
        ("partial 200x40: text page", text_page, partial),
        ("full: 40 scrollers", scroller_page, None),
        ("partial 200x40: 40 scrollers", scroller_page, partial),
        ("full: 30 opacity layers", layer_page, None),
        ("full: 60 images", image_page, None),
    ];
    for (name, build, region) in scenarios {
        let mut doc = build();
        let mut best = [0.0; 2];
        for (i, reference) in [true, false].into_iter().enumerate() {
            let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
            painter.set_reference_mode(reference);
            full_paint(&mut doc, &mut painter);
            let label = format!("{name} [{}]", if reference { "before" } else { "after" });
            best[i] = time(&label, REPS, || match region {
                Some(r) => partial_paint(&mut doc, &mut painter, r),
                None => full_paint(&mut doc, &mut painter),
            });
            painter.take_stats();
            match region {
                Some(r) => partial_paint(&mut doc, &mut painter, r),
                None => full_paint(&mut doc, &mut painter),
            }
            println!("    one frame: {:?}", painter.take_stats());
        }
        println!("    speed-up x{:.1}", best[0] / best[1]);
    }
}
