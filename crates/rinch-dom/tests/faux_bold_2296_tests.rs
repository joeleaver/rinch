//! Synthetic bold and italic (#2296): a family with no bold face set in
//! `font-weight: bold`, or with no italic face set in `font-style: italic`, is
//! drawn from its regular face, emboldened or slanted, as Chrome draws it.
//!
//! Parley marks such a run (`Run::synthesis`); the painters used to read only
//! its skew, and only the Vello outline path applied that.
//!
//! **What Chrome does (156, Linux, measured on the bundled Inter registered
//! with only its regular face):** Skia strokes the outline as it fills it, with
//! a stroke `size × k` wide, where `k` is 1/24 of the em up to 9px, 1/32 from
//! 36px, and linear in between (`kStdFakeBoldInterpKeys`). The stem of an `l`
//! grew by 0.401 px at 9px, 0.660 at 16px, 0.866 at 24px, 1.124 at 36px, 1.498
//! at 48px and 3.004 at 96px; its left edge moved left by half that and its
//! top and bottom out by half: the growth is even on every side. Advances do
//! not change. [`chrome_growth`] is that rule.
//!
//! The face is the bundled Inter (one static regular face), registered under
//! its own name so the host's fonts cannot answer, at declared sizes and line
//! heights.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;
use rinch_dom::paint::text_rasterization::{
    ResolvedTextRasterization, StemDarkening, SystemTextSettings, TextHinting, TextRasterization,
    reset_thread_policy, set_thread_policy,
};
use rinch_dom::paint::vello_painter::VelloPainter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const W: u32 = 640;
/// Each row's band.
const BAND: u32 = 100;
/// Ten stems: `l` is a plain vertical bar in Inter.
const STEMS: &str = "llllllllll";

/// The total stem growth Chrome's faux bold gives text of `size` CSS px:
/// Skia's stroke width (see the module doc).
fn chrome_growth(size: f64) -> f64 {
    let (k0, k1) = (1.0 / 24.0, 1.0 / 32.0);
    let k = if size <= 9.0 {
        k0
    } else if size >= 36.0 {
        k1
    } else {
        k0 + (size - 9.0) / 27.0 * (k1 - k0)
    };
    size * k
}

fn off() -> ResolvedTextRasterization {
    TextRasterization::default()
        .stem_darkening(StemDarkening::Off)
        .resolve(&SystemTextSettings::default())
}

fn unhinted_off() -> ResolvedTextRasterization {
    TextRasterization::default()
        .hinting(TextHinting::None)
        .stem_darkening(StemDarkening::Off)
        .resolve(&SystemTextSettings::default())
}

/// One `BAND`-tall row per entry: `text` styled `css` (which sets the font and
/// colours), the text starting a fraction of a pixel into the row.
fn document(rows: &[(&str, &str)]) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one face: the family has no bold");
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0; background: #ffffff");
    for (css, text) in rows {
        let row = doc.create_element("div");
        doc.set_attribute(
            row,
            "style",
            &format!(
                "height: {BAND}px; padding: 8px 0 0 10.3px; box-sizing: border-box; \
                 white-space: pre; {css}"
            ),
        );
        let t = doc.create_text(text);
        doc.append_child(row, t);
        doc.append_child(body, row);
    }
    doc.resolve_layout(W as f32, (BAND * rows.len() as u32) as f32);
    doc
}

/// Every RGBA byte of `doc` painted by tiny-skia under `policy`.
fn software(doc: &mut RinchDocument, policy: ResolvedTextRasterization) -> Vec<u8> {
    let h = doc_height(doc);
    set_thread_policy(policy);
    let mut painter = TinySkiaPainter::new(W, h);
    painter.pixels_mut().fill(255);
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, h as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    reset_thread_policy();
    painter.pixels().to_vec()
}

/// The canvas height `doc`'s rows fill.
fn doc_height(doc: &RinchDocument) -> u32 {
    let body = doc.body();
    BAND * doc.get_children(body).len() as u32
}

/// Black-on-white ink of row `band`, per pixel (`0` ground, `1` ink),
/// row-major.
fn band_coverage(rgba: &[u8], band: u32) -> Vec<f64> {
    let y0 = band * BAND;
    (y0..y0 + BAND)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .map(|(x, y)| 1.0 - rgba[((y * W + x) * 4) as usize] as f64 / 255.0)
        .collect()
}

/// What the stems of a band measure.
#[derive(Debug)]
struct Stems {
    /// Mean stem width, in px, over five rows at the stems' middle.
    width: f64,
    /// The ink's coverage-weighted centre.
    centre: (f64, f64),
    /// The total ink.
    ink: f64,
    /// `(x, y)` centres of the ink's top and bottom thirds.
    top: (f64, f64),
    bottom: (f64, f64),
}

fn measure(c: &[f64], stems: usize) -> Stems {
    let at = |x: u32, y: u32| c[(y * W + x) as usize];
    let rows: Vec<u32> = (0..BAND)
        .filter(|&y| (0..W).map(|x| at(x, y)).sum::<f64>() > 0.5)
        .collect();
    assert!(!rows.is_empty(), "nothing painted");
    let (y0, y1) = (rows[0], *rows.last().unwrap());
    let mid = (y0 + y1) / 2;
    let width = (mid - 2..=mid + 2)
        .map(|y| (0..W).map(|x| at(x, y)).sum::<f64>())
        .sum::<f64>()
        / 5.0
        / stems as f64;
    let centre_of = |ys: std::ops::RangeInclusive<u32>| {
        let (mut sx, mut sy, mut s) = (0.0, 0.0, 0.0);
        for y in ys {
            for x in 0..W {
                let v = at(x, y);
                sx += v * (x as f64 + 0.5);
                sy += v * (y as f64 + 0.5);
                s += v;
            }
        }
        ((sx / s, sy / s), s)
    };
    let (centre, ink) = centre_of(0..=BAND - 1);
    let third = (y1 - y0) / 3;
    let (top, _) = centre_of(y0..=y0 + third);
    let (bottom, _) = centre_of(y1 - third..=y1);
    Stems {
        width,
        centre,
        ink,
        top,
        bottom,
    }
}

/// A bold `l` in a family with no bold face is Chrome's faux bold: each stem
/// grows by Skia's stroke width, at a size under Skia's interpolation (16px)
/// and past it (48px), with hinting off (Chrome's `hintslight` does not
/// touch stem widths; swash's full hinting of Inter can) and on.
///
/// Red before #2296: the bold row was the regular row, pixel for pixel.
#[test]
fn software_faux_bold_grows_stems_by_chromes_amount() {
    for policy in [unhinted_off(), off()] {
        for size in [16.0, 48.0] {
            let mut doc = document(&[
                (&format!("font: {size}px/{}px ProbeFace", size * 1.5), STEMS),
                (
                    &format!("font: bold {size}px/{}px ProbeFace", size * 1.5),
                    STEMS,
                ),
            ]);
            let px = software(&mut doc, policy);
            let regular = measure(&band_coverage(&px, 0), 10);
            let bold = measure(&band_coverage(&px, 1), 10);
            let growth = bold.width - regular.width;
            let want = chrome_growth(size);
            assert!(
                (growth - want).abs() < 0.08,
                "{size}px (hinting {}): stems grew {growth:.3} px, Chrome {want:.3} px \
                 ({regular:?} -> {bold:?})",
                policy.hint()
            );
        }
    }
}

/// The left and right edges of each stem in the middle row of a band's
/// coverage, in px (the left edge's exact position from its pixel's coverage).
fn stem_edges(c: &[f64]) -> Vec<(f64, f64)> {
    let rows: Vec<u32> = (0..BAND)
        .filter(|&y| (0..W).map(|x| c[(y * W + x) as usize]).sum::<f64>() > 0.5)
        .collect();
    let y = (rows[0] + rows[rows.len() - 1]) / 2;
    let row: Vec<f64> = (0..W).map(|x| c[(y * W + x) as usize]).collect();
    let mut stems = vec![];
    let mut x = 0;
    while x < row.len() {
        if row[x] > 0.0 {
            let left = x as f64 + 1.0 - row[x];
            let mut width = 0.0;
            while x < row.len() && row[x] > 0.0 {
                width += row[x];
                x += 1;
            }
            stems.push((left, left + width));
        }
        x += 1;
    }
    stems
}

/// The growth is even on every side, as Chrome's is: each stem's left edge
/// moves left by as much as its right edge moves right, and the ink's centre
/// does not move. So is the stem darkening of light text, which faux bold
/// adds to: swash grows an outline up and to the right only, and its mask is
/// moved back (Vello's expansion is centred already).
///
/// Red before #2296 for faux bold (it was not drawn) and, with only the
/// faux bold drawn, for both: the left edges stayed put and the centre moved
/// right by the amount per side, 0.75 px here.
#[test]
fn software_faux_bold_is_even_on_every_side() {
    const LIGHT: &str = "color: #fff; background: #000";
    for (base, grown, policy) in [
        // Faux bold alone.
        (
            "font: 48px/72px ProbeFace".to_string(),
            "font: bold 48px/72px ProbeFace".to_string(),
            unhinted_off(),
        ),
        // Faux bold under a faux italic: the growth is centred before the
        // shear, as Vello's expansion and glyph transform leave it.
        (
            "font: italic 48px/72px ProbeFace".to_string(),
            "font: italic bold 48px/72px ProbeFace".to_string(),
            unhinted_off(),
        ),
        // Stem darkening alone: light text against the same text dark.
        (
            "font: 20px/30px ProbeFace".to_string(),
            format!("font: 20px/30px ProbeFace; {LIGHT}"),
            ResolvedTextRasterization::default(),
        ),
        // Both: light faux bold against dark regular.
        (
            "font: 20px/30px ProbeFace".to_string(),
            format!("font: bold 20px/30px ProbeFace; {LIGHT}"),
            ResolvedTextRasterization::default(),
        ),
    ] {
        let mut doc = document(&[(&base, STEMS), (&grown, STEMS)]);
        let px = software(&mut doc, policy);
        let coverage = |band: u32, css: &str| {
            let mut c = band_coverage(&px, band);
            if css.contains(LIGHT) {
                c.iter_mut().for_each(|v| *v = 1.0 - *v);
            }
            c
        };
        let (c0, c1) = (coverage(0, &base), coverage(1, &grown));
        let (regular, bold) = (measure(&c0, 10), measure(&c1, 10));
        assert!(
            bold.ink > regular.ink * 1.05,
            "{grown}: {regular:?} -> {bold:?}"
        );
        let (dx, dy) = (
            bold.centre.0 - regular.centre.0,
            bold.centre.1 - regular.centre.1,
        );
        assert!(
            dx.abs() < 0.03 && dy.abs() < 0.03,
            "{grown}: the ink's centre moved by ({dx:.3}, {dy:.3}): the growth is not even"
        );
        // A slanted edge's position is not its first pixel's coverage, so
        // the italic case is held to its centre alone.
        if grown.contains("italic") {
            continue;
        }
        let (e0, e1) = (stem_edges(&c0), stem_edges(&c1));
        assert_eq!(e0.len(), 10, "{base}: {e0:?}");
        assert_eq!(e1.len(), 10, "{grown}: {e1:?}");
        for ((l0, r0), (l1, r1)) in e0.into_iter().zip(e1) {
            let (left, right) = (l0 - l1, r1 - r0);
            assert!(
                left > 0.05 && (left - right).abs() < 0.03,
                "{grown}: a stem's left edge moved out {left:.3} px, its right {right:.3}"
            );
        }
    }
}

/// An italic in a family with no italic face is slanted by Parley's 14
/// degrees (Chrome's too) on the software painter: the top of a stem sits
/// `tan 14°` of the height to the right of its bottom.
///
/// Red before #2296: the software painter drew it upright.
#[test]
fn software_faux_italic_is_slanted() {
    let mut doc = document(&[
        ("font: 48px/72px ProbeFace", STEMS),
        ("font: italic 48px/72px ProbeFace", STEMS),
    ]);
    let px = software(&mut doc, off());
    let lean = |s: &Stems| (s.top.0 - s.bottom.0) / (s.bottom.1 - s.top.1);
    let upright = measure(&band_coverage(&px, 0), 10);
    let italic = measure(&band_coverage(&px, 1), 10);
    assert!(
        lean(&upright).abs() < 0.01,
        "the control leans: {upright:?}"
    );
    let want = 14f64.to_radians().tan();
    assert!(
        (lean(&italic) - want).abs() < 0.01,
        "italic leans {:.4}, want {want:.4}: {italic:?}",
        lean(&italic)
    );
}

/// The per-side amount each run's outlines are expanded by on the Vello
/// painter's outline path (the atlas off), with the darkening it adds to.
fn vello_amounts(rows: &[(&str, &str)], policy: ResolvedTextRasterization) -> Vec<f64> {
    set_thread_policy(policy);
    let mut doc = document(rows);
    let h = doc_height(&doc);
    let mut painter = VelloPainter::new();
    painter.set_glyph_atlas_enabled(false);
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, h as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    reset_thread_policy();
    painter
        .scene()
        .encoding()
        .resources
        .glyph_runs
        .iter()
        .map(|r| {
            assert_eq!(r.font_embolden.amount.xx, r.font_embolden.amount.yy);
            r.font_embolden.amount.xx
        })
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-5
}

/// The Vello outline path expands a faux-bold run by half Chrome's stroke
/// on each side, and adds it to the stem darkening of light text: a white
/// bold run is both.
///
/// Red before #2296: every dark run was expanded by 0.
#[test]
fn vello_faux_bold_adds_to_the_darkening() {
    let rows = [
        ("font: 13px/20px ProbeFace; color: #000", "Hamburg"),
        ("font: bold 13px/20px ProbeFace; color: #000", "Hamburg"),
        (
            "font: bold 13px/20px ProbeFace; color: #fff; background: #000",
            "Hamburg",
        ),
        (
            "font: 13px/20px ProbeFace; color: #fff; background: #000",
            "Hamburg",
        ),
    ];
    let faux = chrome_growth(13.0) / 2.0;
    let dark = 6.0 / 64.0;
    let got = vello_amounts(&rows, ResolvedTextRasterization::default());
    assert_eq!(got.len(), 4, "a run per row: {got:?}");
    assert_eq!(got[0], 0.0, "regular: {got:?}");
    assert!(close(got[1], faux), "bold: {got:?}, want {faux}");
    assert!(
        close(got[2], faux + dark),
        "white bold: {got:?}, want {}",
        faux + dark
    );
    assert!(close(got[3], dark), "white regular: {got:?}");
    // Off: the faux bold alone.
    let got = vello_amounts(&rows, off());
    assert!(
        got[0] == 0.0 && close(got[1], faux) && close(got[2], faux) && got[3] == 0.0,
        "darkening off: {got:?}"
    );
}

/// Faux bold is a function of the CSS size, scaled with the text, as Skia's
/// stroke is: 20px bold under `scale(2)` grows twice what it grows unscaled,
/// not what 40px text grows. Vello folds a hinted uniform scale into the font
/// size and expands at device size; under a rotation it expands at the run's
/// own size and the transform doubles it.
#[test]
fn vello_faux_bold_scales_with_the_text() {
    let faux = chrome_growth(20.0) / 2.0;
    let one = |css: &str| {
        let got = vello_amounts(&[(css, "Hamburg")], off());
        assert_eq!(got.len(), 1, "{css}: {got:?}");
        got[0]
    };
    let base = "font: bold 20px/24px ProbeFace; transform-origin: 0 0";
    assert!(close(one(base), faux), "unscaled");
    assert!(
        close(one(&format!("{base}; transform: scale(2)")), 2.0 * faux),
        "scale(2)"
    );
    assert!(
        close(
            one(&format!("{base}; transform: scale(2) rotate(90deg)")),
            faux
        ),
        "rotated: expanded at 20px, doubled by the transform"
    );
}

/// Faux bold and faux italic reach a blurred `text-shadow`, whose mask is
/// rasterised from a copy of the text and cached by what it draws: a bold
/// run's shadow carries more ink than the same text's regular shadow, though
/// the glyphs, the font and the positions in the mask are the same.
#[test]
fn software_faux_bold_reaches_text_shadows() {
    for shadow in ["0 0 0 #000", "1px 1px 3px #000"] {
        let mut doc = document(&[
            (
                &format!("font: 24px/36px ProbeFace; color: transparent; text-shadow: {shadow}"),
                "Hamburgefonstiv",
            ),
            (
                &format!(
                    "font: bold 24px/36px ProbeFace; color: transparent; text-shadow: {shadow}"
                ),
                "Hamburgefonstiv",
            ),
        ]);
        let px = software(&mut doc, off());
        let ink = |b| band_coverage(&px, b).iter().sum::<f64>();
        let (regular, bold) = (ink(0), ink(1));
        assert!(regular > 100.0, "{shadow}: no shadow drawn ({regular})");
        assert!(
            bold > regular * 1.08,
            "{shadow}: the bold shadow carries {bold:.1}, the regular {regular:.1}"
        );
    }
}

/// Both painters key a faux-bold run's masks apart from the regular run's
/// (#2296): one face, one size, two runs in the atlas, the bold one carrying
/// the device amount; and a faux italic only on the software painter (Vello
/// draws it as outlines, its glyph transform).
#[test]
fn both_painters_key_faux_bold_masks_apart() {
    use rinch_dom::paint::glyph_atlas::{GlyphAtlas, SharedAtlas};
    let rows = [
        ("font: 13px/20px ProbeFace", "Hamburg"),
        ("font: bold 13px/20px ProbeFace", "Hamburg"),
        ("font: italic 13px/20px ProbeFace", "Hamburg"),
    ];
    let faux = (chrome_growth(13.0) / 2.0) as f32;
    let skew = 14f32.to_radians().tan();
    let new_atlas =
        || -> SharedAtlas { std::rc::Rc::new(std::cell::RefCell::new(GlyphAtlas::default())) };
    for scale in [1.0, 2.0] {
        let mut doc = document(&rows);
        let h = doc_height(&doc);
        set_thread_policy(off());
        let atlas = new_atlas();
        let mut painter =
            TinySkiaPainter::new((W as f64 * scale) as u32, (h as f64 * scale) as u32);
        painter.set_glyph_atlas(atlas.clone());
        let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            scale,
            (W as f32, h as f32),
            &mut doc.font_cx,
            &mut cx,
        );
        let software = atlas.borrow().run_synthesis();
        let atlas = new_atlas();
        let mut painter = VelloPainter::new();
        painter.set_glyph_atlas(atlas.clone());
        rinch_dom::paint::paint_document(
            &doc.tree,
            &mut painter,
            scale,
            (W as f32, h as f32),
            &mut doc.font_cx,
            &mut cx,
        );
        reset_thread_policy();
        let gpu = atlas.borrow().run_synthesis();
        let device = faux * scale as f32;
        let near = |got: &[(f32, f32)], want: &[(f32, f32)]| {
            got.len() == want.len()
                && got
                    .iter()
                    .zip(want)
                    .all(|(g, w)| (g.0 - w.0).abs() < 1e-5 && (g.1 - w.1).abs() < 1e-6)
        };
        assert!(
            near(&software, &[(0.0, 0.0), (device, 0.0), (0.0, skew)]),
            "{scale}x software: {software:?}, want {device} and {skew}"
        );
        assert!(
            near(&gpu, &[(0.0, 0.0), (device, 0.0)]),
            "{scale}x Vello: {gpu:?}, want {device}"
        );
    }
    // Under a CSS `scale(2)` the masks are rasterised at the device size, and
    // so is the faux bold: twice the 13px amount on both painters.
    let mut doc = document(&[(
        "font: bold 13px/20px ProbeFace; transform: scale(2); transform-origin: 0 0",
        "Hamburg",
    )]);
    set_thread_policy(off());
    let mut cx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();
    let software = new_atlas();
    let mut painter = TinySkiaPainter::new(W, BAND);
    painter.set_glyph_atlas(software.clone());
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, BAND as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    let gpu = new_atlas();
    let mut painter = VelloPainter::new();
    painter.set_glyph_atlas(gpu.clone());
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, BAND as f32),
        &mut doc.font_cx,
        &mut cx,
    );
    reset_thread_policy();
    for (name, atlas) in [("software", software), ("Vello", gpu)] {
        let got = atlas.borrow().run_synthesis();
        assert!(
            got.len() == 1 && (got[0].0 - 2.0 * faux).abs() < 1e-5,
            "{name} under scale(2): {got:?}, want {}",
            2.0 * faux
        );
    }
}

/// On the software painter too, faux bold scales with the text: 20px bold
/// under `scale(2)` grows its stems by twice Chrome's 20px amount, on the
/// atlas's grid (a uniform scale, rasterised at 40 device px) and off it (a
/// hair of rotation: the 20px mask drawn through the transform).
#[test]
fn software_faux_bold_scales_with_the_text() {
    for transform in ["scale(2)", "scale(2) rotate(0.01deg)"] {
        // One document each: the scaled rows are taller than a band.
        let stems = |weight: &str| {
            let mut doc = document(&[(
                &format!(
                    "font: {weight} 20px/30px ProbeFace; transform: {transform}; \
                     transform-origin: 0 0"
                ),
                STEMS,
            )]);
            measure(&band_coverage(&software(&mut doc, unhinted_off()), 0), 10)
        };
        let (regular, bold) = (stems("normal"), stems("bold"));
        let growth = bold.width - regular.width;
        let want = 2.0 * chrome_growth(20.0);
        assert!(
            (growth - want).abs() < 0.1,
            "{transform}: stems grew {growth:.3} px, want {want:.3} ({regular:?} -> {bold:?})"
        );
    }
}
