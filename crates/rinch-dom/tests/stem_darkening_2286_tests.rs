//! Stem darkening of light text (#2286, stage 1b) on both painters.
//!
//! Both painters blend glyph coverage in sRGB-encoded space, so a light glyph
//! on a dark ground is the exact inverse of the same glyph dark on light: with
//! darkening off, `light + dark == 255` at every pixel of two rows that differ
//! only in their colours. That makes the dark row the control for the light
//! one. Darkening grows light glyphs only, by the same amount on every side.
//!
//! The text is the bundled Inter, at a declared size and line height, so the
//! numbers do not move with the host's fonts.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::paint::text_rendering::{
    ResolvedTextRendering, StemDarkening, SystemTextSettings, TextHinting, TextRendering,
    reset_text_rendering, set_text_rendering,
};
use rinch_dom::paint::vello_painter::VelloPainter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const W: u32 = 320;
const H: u32 = 80;
const TEXT: &str = "Hamburgefonstiv 0123 Wq";

/// Two 40px rows, the same text at 13px/20px Inter in each: white on black
/// at the top, black on white below.
fn document() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1);
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    for (fg, bg) in [("#ffffff", "#000000"), ("#000000", "#ffffff")] {
        let row = doc.create_element("div");
        doc.set_attribute(
            row,
            "style",
            &format!(
                "height: 40px; padding: 10px 6px 0; box-sizing: border-box; \
                 font: 13px/20px ProbeFace; color: {fg}; background: {bg}"
            ),
        );
        let t = doc.create_text(TEXT);
        doc.append_child(row, t);
        doc.append_child(body, row);
    }
    doc.resolve_layout(W as f32, H as f32);
    doc
}

/// The red channel of every pixel, painted by tiny-skia under `policy`.
fn software(policy: ResolvedTextRendering) -> Vec<u8> {
    set_text_rendering(policy);
    let mut doc = document();
    let mut painter = TinySkiaPainter::new(W, H);
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    reset_text_rendering();
    painter.pixels().chunks(4).map(|p| p[0]).collect()
}

/// Coverage of the light row (`top`) or the dark row, per pixel, row-major
/// over a 40px band.
fn coverage(red: &[u8], top: bool) -> Vec<f64> {
    let y0 = if top { 0 } else { 40 };
    (y0..y0 + 40)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .map(|(x, y)| {
            let v = red[(y * W + x) as usize] as f64 / 255.0;
            if top { v } else { 1.0 - v }
        })
        .collect()
}

fn ink(c: &[f64]) -> f64 {
    c.iter().sum()
}

/// The coverage-weighted centre of a band, in pixels.
fn centroid(c: &[f64]) -> (f64, f64) {
    let (mut sx, mut sy, mut s) = (0.0, 0.0, 0.0);
    for (i, &v) in c.iter().enumerate() {
        let (x, y) = ((i as u32 % W) as f64 + 0.5, (i as u32 / W) as f64 + 0.5);
        sx += v * x;
        sy += v * y;
        s += v;
    }
    (sx / s, sy / s)
}

fn off() -> ResolvedTextRendering {
    TextRendering::default()
        .stem_darkening(StemDarkening::Off)
        .resolve(&SystemTextSettings::default())
}

/// With darkening off the two rows are each other's inverse, pixel for pixel:
/// the control that makes the dark row stand for "the same glyphs, not
/// darkened" in the tests below. A darkening that ignored `Off` breaks it.
#[test]
fn software_with_darkening_off_light_text_is_the_inverse_of_dark() {
    let red = software(off());
    let (light, dark) = (coverage(&red, true), coverage(&red, false));
    assert!(ink(&dark) > 300.0, "no text painted: {}", ink(&dark));
    let worst = light
        .iter()
        .zip(&dark)
        .map(|(l, d)| (l - d).abs())
        .fold(0.0, f64::max);
    assert!(
        worst <= 1.0 / 255.0 + 1e-9,
        "light and dark rows differ by {worst} somewhere: darkening was not off"
    );
}

/// By default the light row carries more ink than the dark row (its glyphs
/// are grown), the dark row is byte-for-byte what it is with darkening off,
/// and the growth is even on every side: the ink's centre does not move.
#[test]
fn software_light_text_is_darkened_evenly_and_dark_text_is_not() {
    let base = software(off());
    let dark_base = coverage(&base, false);
    let light_base = coverage(&base, true);
    let red = software(ResolvedTextRendering::default());
    let (light, dark) = (coverage(&red, true), coverage(&red, false));
    assert_eq!(dark, dark_base, "dark text changed with darkening on");
    let gain = ink(&light) / ink(&dark);
    assert!(
        gain > 1.08,
        "light text should carry visibly more ink than dark text, got {gain:.4}x"
    );
    assert!(gain < 1.35, "light text is far too heavy: {gain:.4}x");
    let (cx0, cy0) = centroid(&light_base);
    let (cx1, cy1) = centroid(&light);
    assert!(
        (cx1 - cx0).abs() < 0.04 && (cy1 - cy0).abs() < 0.04,
        "darkening moved the ink's centre from ({cx0:.3}, {cy0:.3}) to ({cx1:.3}, {cy1:.3}): \
         it is not even on every side"
    );
}

/// The amounts the Vello painter encodes: the light run's `font_embolden` is
/// the policy's, the dark run's is zero, and `Off` zeroes both. `hinting:
/// None` turns the run's hinting off.
#[test]
fn vello_encodes_the_darkening_on_light_runs_only() {
    let runs = |policy: ResolvedTextRendering| {
        set_text_rendering(policy);
        let mut doc = document();
        let mut painter = VelloPainter::new();
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            1.0,
            (W as f32, H as f32),
            &mut doc.font_cx,
            &mut cx,
        );
        reset_text_rendering();
        painter
            .scene()
            .encoding()
            .resources
            .glyph_runs
            .iter()
            .map(|r| (r.font_embolden.amount.xx, r.font_embolden.amount.yy, r.hint))
            .collect::<Vec<_>>()
    };
    let on = runs(ResolvedTextRendering::default());
    assert_eq!(on.len(), 2, "one run per row: {on:?}");
    assert_eq!(on[0], (6.0 / 64.0, 6.0 / 64.0, true), "the white run");
    assert_eq!(on[1], (0.0, 0.0, true), "the black run");
    assert_eq!(runs(off()), vec![(0.0, 0.0, true), (0.0, 0.0, true)]);
    let unhinted = TextRendering::default()
        .hinting(TextHinting::None)
        .resolve(&SystemTextSettings::default());
    assert_eq!(
        runs(unhinted),
        vec![(6.0 / 64.0, 6.0 / 64.0, false), (0.0, 0.0, false)]
    );
}

/// `hinting: None` reaches the software painter: the same text rasterises
/// differently unhinted (Inter carries TrueType instructions, so hinting
/// moves its outlines).
#[test]
fn software_hinting_none_draws_unhinted_outlines() {
    let hinted = software(off());
    let unhinted = software(
        TextRendering::default()
            .hinting(TextHinting::None)
            .stem_darkening(StemDarkening::Off)
            .resolve(&SystemTextSettings::default()),
    );
    let (dark_h, dark_u) = (coverage(&hinted, false), coverage(&unhinted, false));
    let moved = dark_h
        .iter()
        .zip(&dark_u)
        .filter(|(a, b)| (*a - *b).abs() > 8.0 / 255.0)
        .count();
    assert!(
        moved > 50,
        "only {moved} pixels differ between hinted and unhinted text: hinting was not turned off"
    );
}
