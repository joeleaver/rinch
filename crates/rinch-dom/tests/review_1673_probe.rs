#![cfg(feature = "software-renderer")]
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::perf::Counter;
use std::time::Instant;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VP: (f32, f32) = (800.0, 600.0);
const LOREM: &str = "The quick brown fox jumps over the lazy dog while a sphinx of black \
    quartz judges my vow; pack my box with five dozen liquor jugs, then 0123456789.";

fn doc(css: &str, html: &str) -> RinchDocument {
    use parley::fontique::Blob;
    let mut doc = RinchDocument::new();
    doc.font_cx
        .collection
        .register_fonts(Blob::new(std::sync::Arc::new(FACE)), None);
    doc.load_css(css);
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc
}

fn paint(doc: &mut RinchDocument, p: &mut TinySkiaPainter) {
    rinch_dom::paint::paint_document(&doc.tree, p, 1.0, VP, &mut doc.font_cx, &mut doc.layout_cx);
}

fn measure(name: &str, css: &str, html: &str) {
    let mut d = doc(css, html);
    let mut p = TinySkiaPainter::new(800, 600);
    paint(&mut d, &mut p);
    let mut cold = f64::MAX;
    let mut warm = f64::MAX;
    let mut masks = 0;
    for _ in 0..7 {
        rinch_dom::paint::clear_text_shadow_cache();
        d.tree.perf.reset();
        let t = Instant::now();
        paint(&mut d, &mut p);
        cold = cold.min(t.elapsed().as_secs_f64() * 1e3);
        masks = d.tree.perf.get(Counter::TextShadowMasksRasterised);
        let t = Instant::now();
        paint(&mut d, &mut p);
        warm = warm.min(t.elapsed().as_secs_f64() * 1e3);
    }
    eprintln!("PROBE {name}: masks={masks} cold={cold:.2}ms warm={warm:.2}ms");
}

const BASE: &str = "body { margin: 0; font-family: Inter; font-size: 14px; line-height: 18px; \
    color: rgb(30, 30, 40); } p { margin: 0 0 2px 0; text-shadow: 0 1px 4px rgba(0, 0, 0, 0.4); }";

/// Measurement probe from the review of #1673 (run with `--release --ignored
/// --nocapture`): masks rasterised and cold/warm paint times per page.
#[test]
#[ignore]
fn probe_costs() {
    // A: the bench page (identical second lines).
    let mut h = String::new();
    for i in 0..40 {
        h.push_str(&format!("<p>{i}: {LOREM}</p>"));
    }
    measure("bench_page", BASE, &h);
    // B: distinct paragraphs (rotate the words).
    let words: Vec<&str> = LOREM.split(' ').collect();
    let mut h = String::new();
    for i in 0..40 {
        let n = words.len();
        let t: Vec<&str> = (0..n).map(|k| words[(k + i * 3) % n]).collect();
        h.push_str(&format!("<p>{i}: {}</p>", t.join(" ")));
    }
    measure("distinct_paragraphs", BASE, &h);
    // C: one line per paragraph, each word its own span (syntax-highlight shape).
    let mut h = String::new();
    for i in 0..30 {
        h.push_str("<p style=\"white-space: nowrap\">");
        for (k, w) in words.iter().take(12).enumerate() {
            h.push_str(&format!("<span>{w}{i}{k}</span> "));
        }
        h.push_str("</p>");
    }
    measure("spans_per_line", BASE, &h);
    // D: two text nodes per line (a label and a reactive value), single line.
    let mut h = String::new();
    for i in 0..30 {
        h.push_str(&format!(
            "<p>Count: <span>{i}</span> items in the basket today</p>"
        ));
    }
    measure("two_nodes_one_line", BASE, &h);
}

/// Writes raw pixels of each page to `$PIX_OUT` (run with `--ignored`).
#[test]
#[ignore]
fn probe_pixels() {
    let out = std::env::var("PIX_OUT").unwrap();
    let pages = [
        (
            "a_one_node_one_line",
            "p{text-shadow: 2px 2px 0 rgb(200,0,0)}",
            "<p>Hello world shadow</p>".to_string(),
        ),
        (
            "b_one_node_multiline_hard",
            "p{width:120px; text-shadow: 1px 1px 0 rgb(200,0,0)}",
            format!("<p>{LOREM}</p>"),
        ),
        (
            "c_one_node_multiline_blur",
            "p{width:300px; text-shadow: 0 1px 4px rgba(0,0,0,0.4)}",
            format!("<p>{LOREM}</p>"),
        ),
        (
            "d_spans_right_offset",
            "p{text-shadow: 2px 2px 0 rgb(200,0,0)}",
            "<p>Ham<span>burg</span> <b>ok</b> there</p>".to_string(),
        ),
        (
            "e_two_nodes_blur",
            "p{text-shadow: 0 0 3px rgb(0,0,200)}",
            "<p>Count: <span>42</span> items</p>".to_string(),
        ),
    ];
    for (name, css, html) in pages {
        let mut d = doc(&format!("{BASE} {css}"), &html);
        rinch_dom::paint::clear_text_shadow_cache();
        let mut p = TinySkiaPainter::new(800, 600);
        p.fill_white();
        paint(&mut d, &mut p);
        std::fs::write(format!("{out}/{name}.raw"), p.pixels()).unwrap();
    }
}
