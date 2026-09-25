//! A `text-shadow` is drawn in the same physical space as the text it shadows
//! (#409).
//!
//! Paint works in physical pixels: every CSS length is multiplied by the DPI
//! `scale` on its way to the painter. `render_text` does that for the glyphs
//! (positions, baseline, advances, font size). The shadow pass did not: its
//! offset was the stylesheet's CSS px added raw to a physical origin, and
//! `render_text_shadow_pass` took no `scale` at all, so its glyphs were
//! rasterised at the layout's logical size. At scale 2 a `text-shadow: 13px
//! 50px` sat 13/50 physical px from the text instead of 26/100, at half the
//! glyph size.
//!
//! # Off the fixed point
//!
//! At `scale == 1` logical and physical coincide and the unscaled shadow is
//! exactly right — which is why the bug survived on every ordinary display.
//! Every assertion here runs at **1.5 and 2**; scale 1 is run only as the
//! control that the fixture itself measures the right thing. The offsets
//! (13px, 50px) are deliberately unequal and not a multiple of each other, so
//! a swapped axis cannot pass either.
//!
//! # Two oracles, one for each half of the claim
//!
//! - **What the painter is handed** (`Recorder`): the backend-neutral half.
//!   The shadow and main passes both go through `Painter::draw_glyphs`, which
//!   is the one call `VelloPainter` and `TinySkiaPainter` share, so pinning its
//!   arguments pins both backends. The shadow run must be handed the main run's
//!   font size, and every shadow glyph must sit exactly `scale × offset` from
//!   its main-pass twin.
//! - **What the software painter draws** (`TinySkiaPainter`): a local pixel
//!   oracle. Red shadow ink and black text ink are told apart by colour; the
//!   shadow's bounding box must be the text's bounding box moved by
//!   `scale × offset`, at the text's own size.
//!
//! The mutants each fixture kills are named on the fixture.
//!
//! The face is the bundled Inter under an override family name, so no
//! assertion depends on the host's fonts; and every assertion is a comparison
//! between two passes over the same glyphs, never an absolute coordinate.

#![cfg(feature = "software-renderer")]

use peniko::kurbo::{Affine, Point, Rect, Stroke};
use peniko::{Brush, Fill, FontData};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

/// Viewport in CSS px.
const VW: f32 = 300.0;
const VH: f32 = 200.0;

/// The shadow offset in CSS px. Unequal, and the vertical one clears the
/// glyphs (32px font, 40px line) so the two inks never overlap in the pixel
/// oracle.
const OX: f64 = 13.0;
const OY: f64 = 50.0;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// Declared font size and line height, so the line box is a statement.
const BOX: &str = "width: 260px; margin: 20px 0 0 30px; font-size: 32px; line-height: 40px; \
                   color: rgb(0, 0, 0); font-family: ProbeFace; \
                   text-shadow: 13px 50px 0 rgb(255, 0, 0)";

const TEXT: &str = "HxH";

fn document(style: &str) -> (RinchDocument, rinch_core::dom::NodeId) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");

    let body = doc.body();
    let c = doc.create_element("div");
    doc.set_attribute(c, "style", style);
    doc.append_child(body, c);
    let t = doc.create_text(TEXT);
    doc.append_child(c, t);
    doc.resolve_layout(VW, VH);
    (doc, c)
}

// ── The backend-neutral oracle ──────────────────────────────────────────────

/// One `draw_glyphs` call: its colour, font size, and each glyph's position in
/// the painter's (physical) space.
#[derive(Debug)]
struct Run {
    red: bool,
    font_size: f32,
    glyphs: Vec<(u32, Point)>,
}

#[derive(Default)]
struct Recorder {
    runs: Vec<Run>,
}

impl Painter for Recorder {
    fn reset(&mut self) {
        self.runs.clear();
    }
    fn fill(&mut self, _: Fill, _: Affine, _: &Brush, _: &PaintShape) {}
    fn stroke(&mut self, _: &Stroke, _: Affine, _: &Brush, _: &PaintShape) {}
    #[allow(clippy::too_many_arguments)]
    fn draw_glyphs(
        &mut self,
        _: &FontData,
        font_size: f32,
        transform: Affine,
        glyph_transform: Option<Affine>,
        brush: &Brush,
        _: bool,
        _: &[i16],
        glyphs: &[PaintGlyph],
    ) {
        assert!(glyph_transform.is_none(), "an upright face has no skew");
        let Brush::Solid(c) = brush else {
            panic!("text is painted with a solid brush");
        };
        let [r, g, b, _] = c.to_rgba8().to_u8_array();
        self.runs.push(Run {
            red: r == 255 && g == 0 && b == 0,
            font_size,
            glyphs: glyphs
                .iter()
                .map(|gl| (gl.id, transform * Point::new(gl.x as f64, gl.y as f64)))
                .collect(),
        });
    }
    fn draw_image(&mut self, _: &PaintImage<'_>, _: Affine) {}
    fn push_clip(&mut self, _: Fill, _: Affine, _: &PaintShape) {}
    fn push_layer(&mut self, _: BlendMode, _: f32, _: Affine, _: &PaintShape) {}
    fn pop_layer(&mut self) {}
}

/// The shadow run and the main run of `TEXT`, painted at `scale`.
fn recorded(scale: f64) -> (Run, Run) {
    let (mut doc, _) = document(BOX);
    let mut painter = Recorder::default();
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (VW, VH),
        &mut doc.font_cx,
        &mut cx,
    );
    let mut shadow = painter.runs.iter().filter(|r| r.red);
    let mut main = painter.runs.iter().filter(|r| !r.red);
    let (s, m) = (shadow.next(), main.next());
    assert!(
        shadow.next().is_none() && main.next().is_none(),
        "one line of one face: exactly one shadow run and one main run, got {:?}",
        painter.runs
    );
    let (s, m) = (
        s.expect("the shadow pass drew nothing; the fixture measures nothing"),
        m.expect("the main pass drew nothing; the fixture measures nothing"),
    );
    assert_eq!(m.glyphs.len(), TEXT.len(), "{TEXT:?} shapes to one glyph per char");
    (
        Run {
            red: true,
            font_size: s.font_size,
            glyphs: s.glyphs.clone(),
        },
        Run {
            red: false,
            font_size: m.font_size,
            glyphs: m.glyphs.clone(),
        },
    )
}

/// The painter is handed the shadow at the text's own size, every glyph
/// exactly `scale × offset` from its twin.
///
/// Kills: dropping the shadow pass's `* sf` on `font_size` (size 16 vs 32 at
/// scale 2), on the glyph positions (the H at 30px lands at 30 not 60), and on
/// the offset (`x + shadow.offset_x` unscaled: 13 not 26) — each one alone.
fn assert_shadow_follows_scale(scale: f64) {
    let (shadow, main) = recorded(scale);
    assert_eq!(
        shadow.font_size, main.font_size,
        "at scale {scale} the shadow run is rasterised at {} while the text is at {}: \
         `render_text_shadow_pass` must scale the font size as `render_text` does (#409)",
        shadow.font_size, main.font_size
    );
    assert_eq!(shadow.glyphs.len(), main.glyphs.len());
    for ((sid, sp), (mid, mp)) in shadow.glyphs.iter().zip(&main.glyphs) {
        assert_eq!(sid, mid, "the shadow pass draws the same glyphs");
        let (dx, dy) = (sp.x - mp.x, sp.y - mp.y);
        assert!(
            (dx - OX * scale).abs() < 1e-3 && (dy - OY * scale).abs() < 1e-3,
            "at scale {scale} glyph {sid}'s shadow is ({dx:.3}, {dy:.3}) physical px from \
             it; `text-shadow: {OX}px {OY}px` must put it at ({:.3}, {:.3}) (#409)",
            OX * scale,
            OY * scale
        );
    }
}

/// Control: at scale 1 the fixture's arithmetic holds with or without #409 —
/// and the main pass really is scaled, so the numbers above are not vacuous.
#[test]
fn control_scale_one_and_the_main_pass_scales() {
    assert_shadow_follows_scale(1.0);
    let (_, m1) = recorded(1.0);
    let (_, m2) = recorded(2.0);
    assert_eq!(m1.font_size, 32.0, "the declared font size reaches the painter");
    assert_eq!(m2.font_size, 64.0, "the main pass is rasterised at scale × size");
    for ((_, p1), (_, p2)) in m1.glyphs.iter().zip(&m2.glyphs) {
        assert!(
            (p2.x - 2.0 * p1.x).abs() < 1e-3 && (p2.y - 2.0 * p1.y).abs() < 1e-3,
            "the main pass at scale 2 is the scale-1 pass doubled: {p1:?} vs {p2:?}"
        );
    }
}

#[test]
fn the_shadow_is_handed_to_the_painter_scaled_at_1_5() {
    assert_shadow_follows_scale(1.5);
}

#[test]
fn the_shadow_is_handed_to_the_painter_scaled_at_2() {
    assert_shadow_follows_scale(2.0);
}

// ── The pixel oracle ────────────────────────────────────────────────────────

/// Bounding boxes of red (shadow) ink and of dark (text) ink, in physical px.
fn inks(scale: f64) -> (Rect, Rect) {
    let (mut doc, _) = document(BOX);
    let (w, h) = (
        (VW as f64 * scale).ceil() as u32,
        (VH as f64 * scale).ceil() as u32,
    );
    let mut painter = TinySkiaPainter::new(w, h);
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (VW, VH),
        &mut doc.font_cx,
        &mut cx,
    );
    let px: Vec<[u8; 4]> = painter.pixels().as_chunks::<4>().0.to_vec();
    let mut red: Option<Rect> = None;
    let mut dark: Option<Rect> = None;
    let grow = |r: &mut Option<Rect>, x: f64, y: f64| {
        let cell = Rect::new(x, y, x + 1.0, y + 1.0);
        *r = Some(r.map_or(cell, |r| r.union(cell)));
    };
    for y in 0..h as usize {
        for x in 0..w as usize {
            let [r, g, b, a] = px[y * w as usize + x];
            if a < 128 {
                continue;
            }
            // Red over white stays r≈255 with g == b dropping; black over
            // white is grey (r == g == b). A half-covered pixel of either is
            // excluded by the thresholds, so each box is the solid ink.
            if r > 200 && g < 128 && b < 128 {
                grow(&mut red, x as f64, y as f64);
            } else if r < 128 && g < 128 && b < 128 {
                grow(&mut dark, x as f64, y as f64);
            }
        }
    }
    (
        red.expect("no shadow ink; the fixture measures nothing"),
        dark.expect("no text ink; the fixture measures nothing"),
    )
}

/// The drawn shadow is the drawn text moved by `scale × offset`, same size.
///
/// Tolerance 2 physical px: anti-aliasing and hinting at a different subpixel
/// phase move a thresholded edge by at most one pixel each side. The broken
/// shadow at scale 2 is off by 13 and 50 px and half the size — nowhere near.
fn assert_drawn_shadow_follows_scale(scale: f64) {
    let (shadow, text) = inks(scale);
    const TOL: f64 = 2.0;
    let (dx, dy) = (shadow.x0 - text.x0, shadow.y0 - text.y0);
    assert!(
        (dx - OX * scale).abs() <= TOL && (dy - OY * scale).abs() <= TOL,
        "at scale {scale} the shadow ink starts ({dx}, {dy}) physical px from the text \
         ink; `text-shadow: {OX}px {OY}px` puts it at ({}, {}) (#409). shadow={shadow:?} \
         text={text:?}",
        OX * scale,
        OY * scale
    );
    assert!(
        (shadow.width() - text.width()).abs() <= TOL
            && (shadow.height() - text.height()).abs() <= TOL,
        "at scale {scale} the shadow ink is {}x{} and the text ink {}x{}: the shadow \
         must be drawn at the text's own size (#409)",
        shadow.width(),
        shadow.height(),
        text.width(),
        text.height()
    );
}

/// Control: at scale 1 the pixel oracle agrees with the recorded one, and the
/// text ink at scale 2 is the scale-1 ink resampled — so the thresholds find
/// glyphs and not noise.
#[test]
fn control_pixels_at_scale_one_and_text_ink_resamples() {
    assert_drawn_shadow_follows_scale(1.0);
    let (_, t1) = inks(1.0);
    let (_, t2) = inks(2.0);
    assert!(
        (t2.width() - 2.0 * t1.width()).abs() <= 3.0
            && (t2.height() - 2.0 * t1.height()).abs() <= 3.0,
        "the text ink at scale 2 ({t2:?}) is the scale-1 ink ({t1:?}) doubled"
    );
}

#[test]
fn the_shadow_is_drawn_scaled_at_1_5() {
    assert_drawn_shadow_follows_scale(1.5);
}

#[test]
fn the_shadow_is_drawn_scaled_at_2() {
    assert_drawn_shadow_follows_scale(2.0);
}

// ── The layer bounds that must contain it ──────────────────────────────────

/// How far right of the unshadowed bounds an opacity layer's bounds reach
/// because of a `text-shadow: 100px 0`, at `scale`.
fn layer_reach_right(scale: f64) -> f64 {
    // A 10px box the unwrapped text overflows, so the unshadowed bounds end at
    // the text's right edge and the difference is the shadow's reach alone.
    let base = "width: 10px; white-space: nowrap; font-size: 16px; line-height: 20px; font-family: ProbeFace; \
                opacity: 0.5";
    let right = |extra: &str| {
        let (doc, c) = document(&format!("{base}; {extra}"));
        rinch_dom::paint::opacity_layer_bounds(&doc.tree, c.0, scale, 0.0, 0.0).x1
    };
    right("text-shadow: 100px 0 0 red") - right("")
}

/// `text_shadow_reach` must grow the bounds by `offset × scale`, the distance
/// the shadow is now drawn at — below scale 1 too.
///
/// Kills the `scale.max(1.0)` #409 left behind: at scale 0.5 it reaches 100
/// physical px for a shadow drawn at 50. (Above scale 1 the `max` is the
/// identity, so this is the only scale that can see it — and the growth is
/// conservative, so no pixel oracle can; tiny-skia ignores layer bounds.)
#[test]
fn opacity_layer_bounds_reach_the_scaled_shadow() {
    for scale in [0.5, 2.0] {
        let reach = layer_reach_right(scale);
        assert!(
            (reach - 100.0 * scale).abs() < 1.0,
            "at scale {scale} a `text-shadow: 100px 0` grows the layer bounds by {reach} \
             physical px; the shadow is drawn {} px right of the text",
            100.0 * scale
        );
    }
}
