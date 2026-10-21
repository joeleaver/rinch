//! Round-2 review fixtures for #1673 (#1060). Pixel oracles on tiny-skia.
#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 400.0;
const VH: f32 = 160.0;
const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const BASE_CSS: &str = "
    body { margin: 0; }
    .t { margin: 20px 0 0 30px; font-size: 40px; line-height: 50px;
         color: rgb(0, 0, 0); font-family: ProbeFace; white-space: nowrap; }
";

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
    doc.load_css(BASE_CSS);
    doc.load_css(css);
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(VW, VH);
    doc.resolve_layout(VW, VH);
    doc
}

/// RGBA pixels, `[y][x]`.
fn paint(css: &str, html: &str) -> Vec<[u8; 4]> {
    let mut doc = doc_with(css, html);
    rinch_dom::paint::clear_text_shadow_cache();
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    painter.fill_white();
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut cx,
    );
    painter.pixels().as_chunks::<4>().0.to_vec()
}

fn at(px: &[[u8; 4]], x: usize, y: usize) -> [u8; 4] {
    px[y * VW as usize + x]
}

fn is_red(p: [u8; 4]) -> bool {
    p[0] > 200 && p[1] < 60 && p[2] < 60
}
fn is_dark(p: [u8; 4]) -> bool {
    p[0] < 80 && p[1] < 80 && p[2] < 80
}

/// The colour test reads the earlier fragment's text colour only. Here the
/// text and the later fragment's hard shadow are both black, so the reach-back
/// is ruled out — but the earlier fragment's underline is red, and the
/// interleaved order (Chrome's, and this PR's per-fragment path) draws the
/// later black shadow over it. At 157e44ef the one pass keeps the underline
/// red on top: fails.
#[test]
fn q1_a_later_shadow_over_an_earlier_underline_of_another_colour_is_on_top() {
    let px = paint(
        ".u { text-decoration: underline; text-decoration-color: rgb(255, 0, 0);
              text-decoration-thickness: 6px; }
         .s { text-shadow: -60px 8px 0 rgb(0, 0, 0); }",
        r#"<p class="t"><span class="u">HH</span><span class="s">HH</span></p>"#,
    );
    // The underline's rows: rows holding many red pixels.
    let rows: Vec<usize> = (0..VH as usize)
        .filter(|&y| (0..VW as usize).filter(|&x| is_red(at(&px, x, y))).count() > 20)
        .collect();
    assert!(!rows.is_empty(), "the underline is drawn");
    let dark = rows
        .iter()
        .map(|&y| (20..100).filter(|&x| is_dark(at(&px, x, y))).count())
        .sum::<usize>();
    assert!(
        dark > 10,
        "the later black shadow covers part of the red underline ({dark} dark px in rows {rows:?})"
    );
}

/// A later fragment of underlined spaces only: its shadow (the underline's)
/// lands across the earlier H's stems. The decorated spaces are ink, so the
/// fragment is drawn after the H and the stems are red in that band.
///
/// Kills: decorated spaces not counted as ink (`st.underline` test dropped).
#[test]
fn q2_an_underlined_space_fragment_casts_over_earlier_text() {
    let px = paint(
        ".su { text-decoration: underline; text-decoration-thickness: 10px;
               text-decoration-color: rgb(0, 0, 255);
               text-shadow: -70px -20px 0 rgb(255, 0, 0); }",
        r#"<p class="t">HH<span class="su">&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;</span></p>"#,
    );
    let red = (0..VH as usize)
        .map(|y| (25..90).filter(|&x| is_red(at(&px, x, y))).count())
        .sum::<usize>();
    let dark = (0..VH as usize)
        .filter(|&y| (25..90).filter(|&x| is_red(at(&px, x, y))).count() > 15)
        .map(|y| (25..90).filter(|&x| is_dark(at(&px, x, y))).count())
        .sum::<usize>();
    assert!(
        red > 50,
        "the underline's shadow is drawn over the Hs ({red} red px)"
    );
    assert_eq!(dark, 0, "no black H stem shows through the later shadow");
}

/// The same with a wavy underline: where the shadowed wave crosses the earlier
/// H's stems, the stems are red (the later shadow is on top).
///
/// Kills: the `wavy` flag ignored.
#[test]
fn q3_a_wavy_underlined_space_fragment_casts_over_earlier_text() {
    let html = r#"<p class="t">HH<span class="su">&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;</span></p>"#;
    let plain = paint(
        ".su { text-decoration: underline wavy rgb(0, 0, 255); }",
        html,
    );
    let px = paint(
        ".su { text-decoration: underline wavy rgb(0, 0, 255);
               text-shadow: -70px -20px 0 rgb(255, 0, 0); }",
        html,
    );
    let red_on_stems = (0..plain.len())
        .filter(|&i| is_dark(plain[i]) && px[i][0] > 120 && px[i][1] < 100 && px[i][2] < 100)
        .count();
    assert!(
        red_on_stems > 3,
        "the shadowed wave is drawn over the H stems ({red_on_stems} px)"
    );
}

/// A raised glyph's blurred shadow is not cut at the line's top: the mask's
/// extent moves with the `vertical-align` shift.
///
/// Kills: `text_extent` ignoring `vertical-align`.
#[test]
fn q4_a_raised_glyphs_blurred_shadow_is_not_cut_at_the_line_top() {
    let css = ".v { vertical-align: 60px; } .t { margin-top: 90px; text-shadow: 0 -10px 4px rgb(255, 0, 0); }";
    let raised = paint(css, r#"<p class="t">x<span class="v">H</span></p>"#);
    // The topmost row holding a reddish pixel.
    let top = (0..VH as usize)
        .find(|&y| {
            (0..VW as usize).any(|x| {
                let p = at(&raised, x, y);
                p[0] > 200 && p[1] < 230
            })
        })
        .unwrap();
    // H's top at 40px is ~29px above the baseline; raised 60 and shadowed 10
    // more, the blur's tail starts around 99 + 8 px above the baseline at
    // y = 90 + ~38 = 128: well above y = 40 — and the mask would be cut at the
    // line's top less its margins (~75) if the extent ignored the shift.
    assert!(top < 45, "the blurred shadow reaches up to y = {top}");
}
