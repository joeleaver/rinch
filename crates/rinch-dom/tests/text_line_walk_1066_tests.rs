//! #1066 — an IFC paint walks each line's positioned items once, however many
//! passes it draws from them.
//!
//! Each `text-shadow` list a span casts of its own (#1048) is drawn as its own
//! pass over the paragraph, and so is each text fragment of the per-fragment
//! order (#1060); a blurred pass walks the text twice more (the mask's cache
//! key and its rasterisation). Every one of those passes used to call Parley's
//! `Line::items` and `GlyphRun::glyphs`, both of which re-walk a run's clusters
//! from its start for every glyph run — quadratic in the glyph runs of a line,
//! times the passes. `paint::text::PaintText` collects the items and slices
//! each glyph run's glyphs out of one walk of its run, once per paint.
//!
//! Two pins: the walk count (one per line per paint, whatever the passes), and
//! the pixels — byte-identical to the same pages painted with Parley's own
//! `GlyphRun::glyphs` (`set_reference_text_walk`) on every path a pass can
//! take: hard shadows, blurred masks, the kernel of copies (the Vello-only
//! build's path), the per-fragment order, a HiDPI scale and a transform.
//!
//! The mutant each fixture kills is named on it.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VW: f32 = 400.0;
const VH: f32 = 300.0;

fn doc_with(css: &str, html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    doc.load_css(&format!(
        "p {{ width: 380px; margin: 10px; font: 14px/18px ProbeFace; color: rgb(0, 0, 0); }} {css}"
    ));
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(VW, VH);
    doc
}

fn paint(doc: &mut RinchDocument, scale: f64) -> Vec<u8> {
    let (w, h) = ((VW as f64 * scale) as u32, (VH as f64 * scale) as u32);
    let mut painter = TinySkiaPainter::new(w, h);
    painter.fill_white();
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::clear_text_shadow_cache();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (VW, VH),
        &mut doc.font_cx,
        &mut cx,
    );
    painter.pixels().to_vec()
}

/// `n` spans, each casting its own `text-shadow` list `0 2px {blur}px` in its
/// own colour, and — when `coloured` — drawn in its own colour (one glyph run
/// a span).
fn spans(n: usize, blur: u32, coloured: bool) -> String {
    let mut html = String::from("<p>");
    for i in 0..n {
        let colour = if coloured {
            format!("color: rgb(0, {}, 90); ", (i * 37) % 200)
        } else {
            String::new()
        };
        html.push_str(&format!(
            r#"<span style="{colour}text-shadow: 0 2px {blur}px rgb({}, 0, {})">w{i}x </span>"#,
            (i * 53) % 256,
            (i * 97) % 256
        ));
    }
    html.push_str("</p>");
    html
}

fn walks(doc: &mut RinchDocument) -> u64 {
    rinch_dom::paint::take_text_line_walks();
    paint(doc, 1.0);
    rinch_dom::paint::take_text_line_walks()
}

/// A paragraph whose spans cast 60 distinct lists walks its lines exactly as
/// often as the same paragraph with no shadow at all: once each per paint.
/// Hard and blurred, coloured and not.
///
/// Kills: a `PaintText` built per pass (in `render_text_shadows`, or in
/// `draw_masked` for the key and the rasterisation) — each line is then walked
/// once per pass that reaches it.
#[test]
fn a_paint_walks_each_line_once_whatever_the_passes() {
    let plain = {
        let mut html = String::from("<p>");
        for i in 0..60 {
            html.push_str(&format!("<span>w{i}x </span>"));
        }
        html.push_str("</p>");
        walks(&mut doc_with("", &html))
    };
    // The positive control: the instrument fired, and the paragraph wraps.
    assert!(plain >= 3, "{plain} line walks for a wrapped paragraph");
    for (blur, coloured) in [(0, false), (0, true), (3, false), (3, true)] {
        let n = walks(&mut doc_with("", &spans(60, blur, coloured)));
        assert_eq!(
            n, plain,
            "blur {blur}, coloured {coloured}: {n} line walks for {plain} lines"
        );
    }
}

/// Paint `html` twice, through the cached items and through Parley's own
/// `GlyphRun::glyphs`, and demand the same bytes.
fn assert_same_pixels(what: &str, css: &str, html: &str, scale: f64) {
    let mut doc = doc_with(css, html);
    let cached = paint(&mut doc, scale);
    let was = rinch_dom::paint::set_reference_text_walk(true);
    let reference = paint(&mut doc, scale);
    rinch_dom::paint::set_reference_text_walk(was);
    // The positive control: something other than the white page was drawn.
    assert!(
        cached.as_chunks::<4>().0.iter().any(|p| p[0] < 250),
        "{what}: nothing painted"
    );
    let differ = cached
        .as_chunks::<4>()
        .0
        .iter()
        .zip(reference.as_chunks::<4>().0)
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        differ, 0,
        "{what}: {differ} pixels differ from Parley's walk"
    );
}

/// The pixels are Parley's walk's, on every path a pass takes.
///
/// Kills: a glyph run's glyphs taken as the rest of its run (no split by
/// style index); the run's glyphs not walked again for the next run (a line of
/// two fonts, or of a run split at a line); a run split by style anywhere but
/// where `GlyphRunIter` splits it.
#[test]
fn the_pixels_are_parleys_walks_on_every_path() {
    let pages: &[(&str, &str, String)] = &[
        ("hard", "", spans(40, 0, true)),
        ("blurred", "", spans(30, 3, true)),
        (
            "fragments",
            "span { text-shadow: 0 1px 4px rgba(0, 0, 0, 0.6); }",
            {
                let mut h = String::from("<p>");
                for i in 0..24 {
                    h.push_str(&format!(
                        r#"<span style="color: rgb({}, 0, 200)">tok{i}</span> "#,
                        (i * 41) % 256
                    ));
                }
                h.push_str("</p>");
                h
            },
        ),
        (
            "mixed",
            ".u { text-decoration: underline; } .w { text-decoration: underline wavy red; } \
             .h { visibility: hidden; } .b { display: inline-block; width: 30px; height: 10px; \
             background: rgb(0, 120, 0); } p { text-shadow: 1px 1px 0 rgb(0, 0, 255); }",
            r#"<p>Ham<span class="u" style="color: rgb(200, 0, 0)">burg</span> fi<sub>2</sub>
               <sup style="text-shadow: 0 0 2px red">n</sup> e&#x301;t&#xe9;
               <span class="w">wave</span> <span class="h">gone</span> mid<span class="b"></span>end
               <b>bold</b> <i>slant</i> שלום world <span style="font-size: 20px">big</span>
               and a long tail to wrap the line onto a second one</p>"#
                .to_string(),
        ),
        (
            "transformed",
            "p { transform: rotate(4deg) scale(1.1); }",
            spans(20, 2, true),
        ),
    ];
    for (name, css, html) in pages {
        assert_same_pixels(name, css, html, 1.0);
        assert_same_pixels(&format!("{name} at 1.5x"), css, html, 1.5);
        let was = rinch_dom::paint::force_tapped_text_shadows(true);
        assert_same_pixels(&format!("{name}, kernel of copies"), css, html, 1.0);
        rinch_dom::paint::force_tapped_text_shadows(was);
    }
}
