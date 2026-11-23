//! `filter` (#2869) and gradient backgrounds (#2884) on a **non-atomic
//! inline** element — a plain `<span>`.
//!
//! A filter makes the span a stacking context (CSS 2.1 Appendix E paints it
//! at step 8 as one group, in tree order with the positioned boxes), and
//! Chrome applies the list to the group of everything the span draws, every
//! line's fragment at once. Before #2869 the span's text, background and
//! border were drawn with its IFC root's runs and the list was never applied.
//!
//! A gradient (or any image layer) fills each of the span's fragments as if
//! they were laid out on one line, side by side (`box-decoration-break:
//! slice`, Chrome's default): the second line's fragment continues where the
//! first one's stopped. Before #2884 an inline element recorded only its
//! `background-color`, and a gradient alone drew nothing.
//!
//! The shape that needs both is Braynard's HUD numerals: a relative span
//! whose gold gradient is cut by its text (`background-clip: text`) under a
//! `drop-shadow`.
//!
//! Every fixture is painted twice, as `positioned_inline_paint_2819_tests`
//! does: clips lifted in place (the software painter) and through [`NoLift`]
//! (Vello's route), which must agree within two levels, at scale 1 and 2.
//! Each expected pixel is Chrome 158's (`--headless=new`, 600x400, `* {
//! box-sizing: border-box }`, `body { margin: 0 }`) with this crate's
//! bundled Inter loaded through `@font-face` as `ProbeFace`.

#![cfg(feature = "software-renderer")]

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, Stroke};
use peniko::{Brush, Fill};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{
    BlendMode, GlyphSynthesis, PaintGlyph, PaintImage, PaintShape, Painter,
};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VW: f32 = 600.0;
const VH: f32 = 400.0;

/// The block container of the box fixtures: `40px/100px` text, whose
/// content area runs from 25.8 to 74.2.
const C: &str = "width: 400px; font: 40px/100px ProbeFace;";
/// The block container of the glyph fixtures: an `I` at `96px/120px`, whose
/// stem Chrome draws over `x 8..18`, `y 25..95`.
const BIG: &str = "width: 400px; font: 96px/120px ProbeFace;";
/// A blue background with a red `drop-shadow` 6px under it.
const SHADOWED: &str = "background: rgb(0, 0, 255); filter: drop-shadow(0 6px 0 rgb(255, 0, 0));";
/// The HUD numerals' fill (#2819, #2869, #2884): a gold gradient cut by the
/// text, under a dark drop shadow.
const GOLD: &str = "background: linear-gradient(to bottom, rgb(255, 226, 122), rgb(200, 144, 30)); \
                    -webkit-background-clip: text; background-clip: text; color: transparent;";
const HUD_SHADOW: &str = "filter: drop-shadow(0 6px 0 rgb(58, 30, 8));";

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const WHITE: [u8; 3] = [255, 255, 255];

/// The software painter with [`Painter::lift_clips`] left at the trait's
/// default: Vello's route through clips and groups.
struct NoLift(TinySkiaPainter);

impl Painter for NoLift {
    fn reset(&mut self) {
        self.0.reset()
    }
    fn fill(&mut self, f: Fill, t: Affine, b: &Brush, s: &PaintShape) {
        self.0.fill(f, t, b, s)
    }
    fn set_coarse_curves(&mut self, coarse: bool) {
        self.0.set_coarse_curves(coarse)
    }
    fn fill_color(&mut self, f: Fill, t: Affine, c: AlphaColor<Srgb>, s: &PaintShape) {
        self.0.fill_color(f, t, c, s)
    }
    fn stroke(&mut self, st: &Stroke, t: Affine, b: &Brush, s: &PaintShape) {
        self.0.stroke(st, t, b, s)
    }
    fn stroke_color(&mut self, st: &Stroke, t: Affine, c: AlphaColor<Srgb>, s: &PaintShape) {
        self.0.stroke_color(st, t, c, s)
    }
    #[allow(clippy::too_many_arguments)]
    fn draw_glyphs(
        &mut self,
        font: &peniko::FontData,
        size: f32,
        t: Affine,
        gt: GlyphSynthesis,
        b: &Brush,
        hint: bool,
        coords: &[i16],
        glyphs: &[PaintGlyph],
    ) {
        self.0
            .draw_glyphs(font, size, t, gt, b, hint, coords, glyphs)
    }
    fn draw_image(&mut self, i: &PaintImage<'_>, t: Affine) {
        self.0.draw_image(i, t)
    }
    fn push_clip(&mut self, f: Fill, t: Affine, s: &PaintShape) {
        self.0.push_clip(f, t, s)
    }
    fn push_layer(&mut self, b: BlendMode, o: f32, t: Affine, s: &PaintShape) {
        self.0.push_layer(b, o, t, s)
    }
    fn push_isolated_layer(&mut self, o: f32, t: Affine, s: &PaintShape) {
        self.0.push_isolated_layer(o, t, s)
    }
    fn pop_layer(&mut self) {
        self.0.pop_layer()
    }
}

fn document(c: &str, inner: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.load_css("* { box-sizing: border-box; } body { margin: 0; }");
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    let body = doc.body();
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, &format!(r#"<div style="{c}">{inner}</div>"#));
    doc.append_child(body, wrap);
    doc.resolve_layout(VW, VH);
    doc
}

fn paint_with(doc: &mut RinchDocument, scale: f64, no_lift: bool) -> TinySkiaPainter {
    let painter = TinySkiaPainter::new((VW as f64 * scale) as u32, (VH as f64 * scale) as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    let mut paint_into = |painter: &mut dyn Painter| {
        rinch_dom::paint::paint_document(
            &doc.tree,
            painter,
            scale,
            (VW, VH),
            &mut doc.font_cx,
            &mut layout_cx,
        )
    };
    if no_lift {
        let mut wrapped = NoLift(painter);
        paint_into(&mut wrapped);
        wrapped.0
    } else {
        let mut painter = painter;
        paint_into(&mut painter);
        painter
    }
}

/// `doc` painted by the software painter, checked against the [`NoLift`]
/// route.
fn paint(doc: &mut RinchDocument, scale: f64) -> TinySkiaPainter {
    let lifted = paint_with(doc, scale, false);
    let popped = paint_with(doc, scale, true);
    let w = lifted.width() as usize;
    for (i, (a, b)) in lifted.pixels().iter().zip(popped.pixels()).enumerate() {
        assert!(
            a.abs_diff(*b) <= 2,
            "lifted in place and popped disagree at ({}, {}) channel {}: {a} vs {b}",
            i / 4 % w,
            i / 4 / w,
            i % 4
        );
    }
    lifted
}

/// The pixel at CSS point `(x, y)`, composited over white (the page Chrome
/// screenshots).
fn at(p: &TinySkiaPainter, (x, y): (u32, u32), scale: f64) -> [u8; 3] {
    let (x, y) = ((x as f64 * scale) as u32, (y as f64 * scale) as u32);
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    let a = d[i + 3];
    [d[i], d[i + 1], d[i + 2]].map(|c| c.saturating_add(255 - a))
}

/// Within 2 of Chrome's pixel, per channel.
#[track_caller]
fn assert_near(got: [u8; 3], chrome: [u8; 3], what: &str) {
    let off = got
        .iter()
        .zip(chrome)
        .map(|(&g, c)| g.abs_diff(c))
        .max()
        .unwrap();
    assert!(off <= 2, "{what}: got {got:?}, Chrome draws {chrome:?}");
}

/// Paint `inner` in a container styled `c` and check each probe.
#[track_caller]
fn check(c: &str, inner: &str, probes: &[((u32, u32), [u8; 3])]) {
    for scale in [1.0, 2.0] {
        let mut doc = document(c, inner);
        let p = paint(&mut doc, scale);
        for &(pt, want) in probes {
            assert_near(at(&p, pt, scale), want, &format!("{pt:?} at scale {scale}"));
        }
    }
}

/// **#2869's case**: a `drop-shadow` on a plain span draws the shadow of
/// everything the span draws, its background here, 6px down: Chrome 158 has
/// red at `y 74..80` under the blue box.
///
/// Kills: the span drawn by its IFC root's runs, the filter never applied
/// (the base before #2869: white under the box).
#[test]
fn a_drop_shadow_on_a_span_shadows_its_box() {
    check(
        C,
        &format!(r#"<span style="{SHADOWED}">MM</span>"#),
        &[
            ((10, 70), BLUE),
            ((10, 77), RED),
            ((60, 77), RED),
            ((10, 82), WHITE),
        ],
    );
}

/// The issue's own shape, text alone: the `I`'s stem shadowed 6px under it
/// (Chrome 158: red at `y 95..101` under a stem ending at 95).
#[test]
fn a_drop_shadow_on_a_span_shadows_its_text() {
    check(
        BIG,
        r#"<span style="color: rgb(0, 0, 255); filter: drop-shadow(0 6px 0 rgb(255, 0, 0))">I</span>"#,
        &[((12, 60), BLUE), ((12, 97), RED), ((30, 97), WHITE)],
    );
}

/// A colour function is applied too: `invert(1)` turns the blue box yellow.
///
/// Kills: only the pixel-moving functions handled for a span.
#[test]
fn a_colour_filter_on_a_span_is_applied() {
    check(
        C,
        r#"<span style="background: rgb(0, 0, 255); filter: invert(1)">MM</span>"#,
        &[((10, 70), [255, 255, 0])],
    );
}

/// A wrapped span's fragments are filtered together: both lines' boxes are
/// shadowed (Chrome 158).
///
/// Kills: a filter group around the first fragment only.
#[test]
fn a_wrapped_filtered_span_shadows_every_fragment() {
    check(
        "width: 130px; font: 40px/100px ProbeFace;",
        &format!(r#"<span style="{SHADOWED}">MM MM</span>"#),
        &[
            ((10, 70), BLUE),
            ((10, 77), RED),
            ((10, 170), BLUE),
            ((10, 177), RED),
        ],
    );
}

/// A relative span (#2819's own entry) with a filter: shadowed where its
/// offset moved it, `top: 10px` (Chrome 158: the box ends at 84, red to 90).
///
/// Kills: the filter applied only to a static span's new entry.
#[test]
fn a_relative_filtered_span_is_shadowed_where_it_moved() {
    check(
        C,
        &format!(r#"<span style="position: relative; top: 10px; {SHADOWED}">MM</span>"#),
        &[((10, 80), BLUE), ((10, 87), RED), ((10, 30), WHITE)],
    );
}

/// **#2884's case**: a gradient on a span fills its fragment, left to right
/// across the box (Chrome 158's pixels along `y = 70`).
///
/// Kills: an inline element recording only its `background-color` (the base
/// before #2884: white).
#[test]
fn a_gradient_fills_a_span() {
    check(
        C,
        r#"<span style="background: linear-gradient(to right, rgb(255, 0, 0), rgb(0, 0, 255))">MMMM</span>"#,
        &[
            ((10, 70), [236, 0, 18]),
            ((70, 70), [131, 0, 124]),
            ((130, 70), [25, 0, 229]),
            ((160, 70), WHITE),
        ],
    );
}

/// A wrapped span's gradient is laid out over its fragments side by side
/// (`box-decoration-break: slice`): the second line starts where the first
/// stopped, half way (Chrome 158).
///
/// Kills: each fragment taking the gradient over its own box (the second
/// line red again at its start).
#[test]
fn a_wrapped_gradient_continues_on_the_next_line() {
    check(
        "width: 160px; font: 40px/100px ProbeFace;",
        r#"<span style="background: linear-gradient(to right, rgb(255, 0, 0), rgb(0, 0, 255))">MMM MMM</span>"#,
        &[
            ((10, 70), [242, 0, 12]),
            ((100, 70), [137, 0, 118]),
            ((10, 170), [115, 0, 139]),
            ((100, 170), [9, 0, 246]),
        ],
    );
}

/// `background-clip: text` with a gradient on a static span: the gold is
/// drawn inside the `I`, top to bottom over the span's box, and nowhere else
/// (Chrome 158).
///
/// Kills: the clipped layer drawn as a colour only (nothing, the span has
/// none).
#[test]
fn a_gradient_clipped_to_a_spans_text() {
    check(
        BIG,
        &format!(r#"<span style="{GOLD}">I</span>"#),
        &[
            ((12, 30), [241, 205, 99]),
            ((12, 60), [227, 184, 75]),
            ((30, 60), WHITE),
            ((12, 97), WHITE),
        ],
    );
}

/// **The HUD numerals**: a relative span, its gold gradient cut by its
/// text, under a dark `drop-shadow` (Chrome 158: gold in the stem, the
/// shadow under it).
#[test]
fn the_hud_numeral_is_gold_with_a_shadow() {
    check(
        BIG,
        &format!(r#"<span style="position: relative; {GOLD} {HUD_SHADOW}">I</span>"#),
        &[
            ((12, 30), [241, 205, 99]),
            ((12, 60), [227, 184, 75]),
            ((12, 97), [58, 30, 8]),
            ((30, 60), WHITE),
        ],
    );
}
