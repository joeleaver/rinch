//! What a paragraph of N spans casting N distinct `text-shadow` lists costs to
//! paint (#1066), beside the cases that cost one pass.
//!
//! `#[ignore]`d — a timing harness, not a gate. Run it with
//! `cargo test -p rinch-dom --release --test span_shadow_groups_bench -- --ignored --nocapture`.
//!
//! One 780px-wide paragraph of N `<span>ab </span>` at 10px in the bundled
//! Inter, painted by the software painter at 800x600, best of 7. Hard
//! shadows, as in the issue's table; the `coloured` column gives every span
//! its own colour too (one glyph run a span), and the `blurred` column a 2px
//! blur (the mask path), with the mask cache cleared before every paint.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

#[derive(Clone, Copy, Debug)]
enum Shape {
    Distinct,
    Shared,
    Root,
    NoShadow,
    DistinctBlurred,
    DistinctColoured,
}

fn page(n: usize, shape: Shape) -> RinchDocument {
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
        "p { width: 780px; margin: 0; font: 10px/12px ProbeFace; color: rgb(0, 0, 0); }
         .s { text-shadow: 0 2px 0 rgb(200, 0, 0); }",
    );
    let root = match shape {
        Shape::Root => r#" style="text-shadow: 0 2px 0 rgb(200, 0, 0)""#,
        _ => "",
    };
    let mut html = format!("<p{root}>");
    for i in 0..n {
        let attr = match shape {
            Shape::Distinct => format!(
                r#" style="text-shadow: 0 2px 0 rgb({}, {}, 0)""#,
                i % 256,
                i / 256
            ),
            Shape::DistinctBlurred => format!(
                r#" style="text-shadow: 0 2px 2px rgb({}, {}, 0)""#,
                i % 256,
                i / 256
            ),
            // Each span in its own colour too: one glyph run a span, as in a
            // syntax-highlighted listing with per-token glows.
            Shape::DistinctColoured => format!(
                r#" style="color: rgb(0, {}, {}); text-shadow: 0 2px 0 rgb({}, {}, 0)""#,
                i % 256,
                i / 256,
                i % 256,
                i / 256
            ),
            Shape::Shared => r#" class="s""#.to_string(),
            Shape::Root | Shape::NoShadow => String::new(),
        };
        html.push_str(&format!("<span{attr}>ab </span>"));
    }
    html.push_str("</p>");
    let body = doc.body();
    doc.set_inner_html(body, &html);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn paint_ms(doc: &mut RinchDocument) -> f64 {
    let mut painter = TinySkiaPainter::new(800, 600);
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    let mut best = f64::INFINITY;
    for _ in 0..7 {
        rinch_dom::paint::clear_text_shadow_cache();
        painter.fill_white();
        let t = std::time::Instant::now();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
            (800.0, 600.0),
            &mut doc.font_cx,
            &mut cx,
        );
        best = best.min(t.elapsed().as_secs_f64() * 1000.0);
    }
    best
}

#[test]
#[ignore]
fn n_distinct_span_shadow_lists() {
    println!("| N | distinct | distinct, coloured | shared | root | none | distinct blurred |");
    for n in [100, 200, 400] {
        let mut row = format!("| {n} |");
        for shape in [
            Shape::Distinct,
            Shape::DistinctColoured,
            Shape::Shared,
            Shape::Root,
            Shape::NoShadow,
            Shape::DistinctBlurred,
        ] {
            let mut doc = page(n, shape);
            row.push_str(&format!(" {:.2} ms |", paint_ms(&mut doc)));
        }
        println!("{row}");
    }
}
