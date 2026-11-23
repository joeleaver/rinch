//! A `position: relative` **inline** element paints as a positioned box
//! (#2819, CSS 2.1 Appendix E step 8): its background, border, text and
//! decorations are drawn in its own entry of the stacking context's sequence,
//! in tree order with the other positioned boxes, or at its `z-index` when it
//! has one — not with its inline formatting context's in-flow content
//! (step 7), below every positioned box.
//!
//! Before #2819 an earlier `position: absolute` sibling covered such a span
//! whatever its `z-index`: the span has no box, its entry in the sequence was
//! skipped (`paint::drawn_by_its_ifc`), and the IFC root drew its runs with
//! the rest of its lines. The shape is Braynard's HUD numerals: an absolute
//! outline layer under a relative fill layer.
//!
//! Every fixture is painted twice, as `absolute_escape_2815_tests` does:
//! clips lifted in place (the software painter) and through [`NoLift`]
//! (Vello's route), which must agree within two levels. Each expected pixel
//! is Chrome 158's (`--headless=new`, 600x400, `* { box-sizing:
//! border-box }`, `body { margin: 0 }`) with this crate's bundled Inter
//! loaded through `@font-face` as `ProbeFace`. A probe sits at `y = 70`, in
//! the span's background below the baseline of `40px/100px` text (the
//! content area runs from 25.8 to 74.2) and clear of every glyph.

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

/// The block container every fixture is in.
const C: &str = "position: relative; width: 400px; font: 40px/100px ProbeFace;";
/// An absolute red box over the whole first line's start.
const ABS: &str = "position: absolute; left: 0; top: 0; width: 300px; height: 100px; \
                   background: rgb(255, 0, 0);";
/// A relative span with a blue background.
const REL: &str = "position: relative; background: rgb(0, 0, 255);";

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];

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

/// **The issue's case**: an absolute box, then a relative span. Both are
/// positioned at `z-index: auto`, so they paint in tree order and the span is
/// on top (Chrome 158: blue).
///
/// Kills: the span's entry skipped and its runs drawn by the IFC root
/// (`main` before #2819: red).
#[test]
fn a_relative_span_paints_over_an_earlier_absolute_box() {
    check(
        C,
        &format!(r#"<div style="{ABS}"></div><span style="{REL}">MM</span>"#),
        &[((10, 70), BLUE), ((60, 70), BLUE)],
    );
}

/// The controls: a **static** span is in-flow content, under the absolute
/// box; and an absolute box **after** the relative span is above it.
///
/// Kills: a fix that lifts every inline element over the positioned boxes,
/// or paints the span after every entry rather than at its own place.
#[test]
fn a_static_span_and_a_later_absolute_box_stay_on_top_of_the_text() {
    check(
        C,
        &format!(r#"<div style="{ABS}"></div><span style="background: rgb(0, 0, 255)">MM</span>"#),
        &[((10, 70), RED)],
    );
    check(
        C,
        &format!(r#"<span style="{REL}">MM</span><div style="{ABS}"></div>"#),
        &[((10, 70), RED)],
    );
}

/// A `z-index` on the span orders it as a stacking context: `1` is over an
/// `auto` absolute box before or after it and under a `z-index: 2` one; `-1`
/// is under an `auto` one after it (Chrome 158).
///
/// Kills: the span's entry sorted at `z == 0` whatever its `z-index`, and
/// the span drawn by the IFC root (red where `1` is over the box).
#[test]
fn a_z_indexed_span_is_ordered_by_its_z_index() {
    let c = format!("{C} z-index: 0;");
    check(
        &c,
        &format!(r#"<div style="{ABS}"></div><span style="{REL} z-index: 1">MM</span>"#),
        &[((10, 70), BLUE)],
    );
    check(
        &c,
        &format!(r#"<span style="{REL} z-index: 1">MM</span><div style="{ABS}"></div>"#),
        &[((10, 70), BLUE)],
    );
    check(
        &c,
        &format!(r#"<div style="{ABS} z-index: 2"></div><span style="{REL} z-index: 1">MM</span>"#),
        &[((10, 70), RED)],
    );
    check(
        &c,
        &format!(r#"<span style="{REL} z-index: -1">MM</span><div style="{ABS}"></div>"#),
        &[((10, 70), RED)],
    );
    // Both negative: the span at `-2` is under the box at `-1`, and nothing
    // of it is drawn with the IFC root's in-flow content above them both.
    check(
        &c,
        &format!(
            r#"<div style="{ABS} z-index: -1"></div><span style="{REL} z-index: -2">MM</span>"#
        ),
        &[((10, 70), RED)],
    );
}

/// A span that wraps paints every fragment in its entry: both lines' pieces
/// are over the absolute box (Chrome 158).
///
/// Kills: a span entry that paints only its first line's fragment.
#[test]
fn a_wrapped_span_paints_both_fragments_over_the_box() {
    check(
        "position: relative; width: 120px; font: 40px/100px ProbeFace;",
        &format!(
            r#"<div style="position: absolute; left: 0; top: 0; width: 120px; height: 200px; background: rgb(255, 0, 0);"></div><span style="{REL}">MM MM</span>"#
        ),
        &[((10, 70), BLUE), ((10, 170), BLUE)],
    );
}

/// Nested relative spans are entries of one sequence in tree order, both
/// after the absolute box: the outer span (`AA`, green) and the inner one
/// (`BB`, blue) are each over it (Chrome 158).
///
/// Kills: a skipped outer entry (red at `AA`), a skipped inner entry (green
/// at `BB`: the outer span's background).
#[test]
fn nested_relative_spans_both_paint_over_an_earlier_box() {
    check(
        C,
        &format!(
            r#"<div style="{ABS}"></div><span style="position: relative; background: rgb(0, 255, 0)">AA<span style="{REL}">BB</span></span>"#
        ),
        &[((10, 70), [0, 255, 0]), ((70, 70), BLUE)],
    );
}

/// The outer span's entry leaves the inner span's content to the inner
/// span's own entry: at `z-index: -1` that is under the absolute box, and
/// the outer span (no background) draws nothing over it (Chrome 158: red).
///
/// Kills: the outer entry drawing the whole of its byte range, the inner
/// span's background with it (blue at `BB`).
#[test]
fn an_outer_span_does_not_draw_a_nested_layered_span() {
    check(
        &format!("{C} z-index: 0;"),
        &format!(
            r#"<div style="{ABS}"></div><span style="position: relative">AA<span style="{REL} z-index: -1">BB</span></span>"#
        ),
        &[((10, 70), RED), ((70, 70), RED)],
    );
}

/// The span's entry draws it where its offset moves it (#1425): `left: 7px`
/// leaves the first 7px of the box uncovered (Chrome 158).
///
/// Kills: the entry painting at the unmoved position, or at the span's
/// parent's origin rather than the IFC root's content origin.
#[test]
fn a_moved_span_is_drawn_at_its_offset() {
    check(
        C,
        &format!(r#"<div style="{ABS}"></div><span style="{REL} left: 7px; top: 5px">MM</span>"#),
        &[((10, 70), BLUE), ((3, 70), RED)],
    );
}

/// `opacity` on the span makes it a stacking context, composited as one
/// group over the box (Chrome 158: half blue over red).
///
/// Kills: the span drawn by the IFC root (red, its opacity ignored).
#[test]
fn a_translucent_span_is_composited_over_the_box() {
    check(
        C,
        &format!(r#"<div style="{ABS}"></div><span style="{REL} opacity: 0.5">MM</span>"#),
        &[((10, 70), [126, 0, 128])],
    );
}

/// Alone over the page, the translucent span is drawn once, at half
/// strength (Chrome 158: `(126, 126, 255)`).
///
/// Kills: the IFC root still drawing the span's runs under its entry (full
/// blue).
#[test]
fn a_translucent_span_is_drawn_once() {
    check(
        C,
        &format!(r#"<span style="{REL} opacity: 0.5">MM</span>"#),
        &[((10, 70), [126, 126, 255])],
    );
}

/// The entry draws the span from its IFC root's **content** origin: in a
/// container padded `9px 0 0 13px` the span starts 13px in, and the box left
/// of that stays red (Chrome 158).
///
/// Kills: the entry painting the span's lines at the root's border-box
/// origin (blue at `x = 5`).
#[test]
fn a_span_in_a_padded_container_is_drawn_inside_the_padding() {
    check(
        &format!("{C} padding: 9px 0 0 13px;"),
        r#"<div style="position: absolute; left: 0; top: 0; width: 300px; height: 120px; background: rgb(255, 0, 0);"></div><span style="position: relative; background: rgb(0, 0, 255);">MM</span>"#,
        &[((23, 79), BLUE), ((5, 79), RED)],
    );
}

/// **The HUD's text**: an absolute red `88` under a relative blue `88` drawn
/// on the same glyphs. Chrome 158 leaves no red pixel (508 blue ones in
/// the 200x100 box at scale 1).
///
/// Kills: the relative span's glyphs drawn by the IFC root, under the
/// absolute span (red everywhere both draw).
#[test]
fn the_relative_layers_glyphs_cover_the_absolute_layers() {
    for scale in [1.0, 2.0] {
        let mut doc = document(
            C,
            r#"<span style="position: absolute; left: 0; top: 0; color: rgb(255, 0, 0)">88</span><span style="position: relative; color: rgb(0, 0, 255)">88</span>"#,
        );
        let p = paint(&mut doc, scale);
        let (mut red, mut blue) = (0, 0);
        for y in 0..(100.0 * scale) as u32 {
            for x in 0..(200.0 * scale) as u32 {
                let i = ((y * p.width() + x) * 4) as usize;
                let d = p.pixels();
                let a = d[i + 3];
                let [r, _, b] = [d[i], d[i + 1], d[i + 2]].map(|c| c.saturating_add(255 - a));
                if r > 128 && b < 100 {
                    red += 1;
                }
                if b > 128 && r < 100 {
                    blue += 1;
                }
            }
        }
        assert_eq!(red, 0, "no red glyph pixel at scale {scale} ({blue} blue)");
        assert!(
            blue > 400,
            "the blue glyphs are drawn at scale {scale}: {blue}"
        );
    }
}

/// The red and blue pixels of the `300x150` box at the page origin.
fn red_and_blue(p: &TinySkiaPainter) -> (u32, u32) {
    let (mut red, mut blue) = (0, 0);
    for y in 0..150 {
        for x in 0..300 {
            let [r, _, b] = at(p, (x, y), 1.0);
            if r > 128 && b < 100 {
                red += 1;
            }
            if b > 128 && r < 100 {
                blue += 1;
            }
        }
    }
    (red, blue)
}

/// **The issue's own shape** (#2819's investigation): a 12px red
/// `-webkit-text-stroke` layer, absolute, under a relative layer whose blue
/// background is clipped to its text. Chrome 158 draws 4850 blue pixels and
/// 5265 red ones (the outer half of the stroke); with the fill layer
/// `static`, 10310 red and no blue — the stroke covers the whole fill.
///
/// Kills: the fill layer's background drawn by the IFC root, under the
/// stroke (no blue pixel: `main` before #2819).
#[test]
fn the_hud_fill_layer_covers_the_inner_half_of_the_stroke() {
    const STROKE: &str = r#"<span style="position: absolute; left: 0; top: 0; color: transparent; -webkit-text-stroke: 12px rgb(255, 0, 0)">88</span>"#;
    let c = "position: relative; font: 120px/150px ProbeFace;";
    let mut doc = document(
        c,
        &format!(
            r#"{STROKE}<span style="position: relative; color: transparent; background: rgb(0, 0, 255); background-clip: text">88</span>"#
        ),
    );
    let (red, blue) = red_and_blue(&paint(&mut doc, 1.0));
    assert!(
        (4500..=5200).contains(&blue) && (4900..=5700).contains(&red),
        "relative fill: {blue} blue, {red} red; Chrome draws 4850 and 5265"
    );
    let mut doc = document(
        c,
        &format!(
            r#"{STROKE}<span style="color: transparent; background: rgb(0, 0, 255); background-clip: text">88</span>"#
        ),
    );
    let (red, blue) = red_and_blue(&paint(&mut doc, 1.0));
    assert!(
        blue < 100 && red > 9500,
        "static fill: {blue} blue, {red} red; Chrome draws 0 and 10310"
    );
}

/// The pixel at `pt` (scale 1) of `inner` in container `c`, with the span
/// styled `span`, and of its twin with a `static` span: a highlight is drawn
/// over a relative span's background exactly as over a static one's.
fn relative_and_static(
    c: &str,
    inner: impl Fn(&str) -> String,
    setup: impl Fn(&mut RinchDocument),
    pt: (u32, u32),
) -> ([u8; 3], [u8; 3]) {
    let mut out = [[0; 3]; 2];
    for (k, span) in [REL, "background: rgb(0, 0, 255);"].into_iter().enumerate() {
        let mut doc = document(c, &inner(span));
        setup(&mut doc);
        doc.resolve_layout(VW, VH + k as f32);
        out[k] = at(&paint(&mut doc, 1.0), pt, 1.0);
    }
    (out[0], out[1])
}

/// **A read-only text selection is drawn over the relative span's
/// background** (review of #2819): the IFC root draws its selection under
/// its text, and the span's entry, drawn after it, draws it again over its
/// own background, clipped to it. Chrome draws a selection over a relative
/// span's background as over a static one's.
///
/// Kills: the span's entry covering the root's selection (pure blue, the
/// first version of #2819).
#[test]
fn a_text_selection_is_drawn_over_a_relative_spans_background() {
    let (rel, stat) = relative_and_static(
        C,
        |span| {
            format!(
                r#"<p data-text-sel="true" data-text-sel-start="0" data-text-sel-end="8" style="margin: 0">xx<span style="{span}">MMMM</span>xx</p>"#
            )
        },
        |_| {},
        (70, 70),
    );
    assert_ne!(stat, BLUE, "positive control: the static span is selected");
    assert_near(rel, stat, "the relative span, selected");
}

/// **The editor's highlight** (a box with a highlight owner, #2666) is drawn
/// over the relative span's background as over a static one's.
///
/// Kills: the span's entry covering the held highlight the root drew.
#[test]
fn an_owned_highlight_is_drawn_over_a_relative_spans_background() {
    let (rel, stat) = relative_and_static(
        &format!("{C} z-index: 0;"),
        |span| {
            format!(
                r#"<p id="para" style="margin: 0">xx<span style="{span}">MMMM</span>xx</p><div id="wash" style="position: absolute; left: 0; top: 0; width: 400px; height: 100px; background: rgba(255, 0, 0, 0.5); z-index: -1"></div>"#
            )
        },
        |doc| {
            let wash = doc.query_selector("#wash").unwrap();
            let para = doc.query_selector("#para").unwrap();
            doc.set_highlight_owner(wash, Some(para));
        },
        (70, 70),
    );
    assert_ne!(
        stat, BLUE,
        "positive control: the static span is highlighted"
    );
    assert_near(rel, stat, "the relative span, highlighted");
}

/// **A drag ghost** (`paint_subtree`) of a plain box holding a relative span
/// draws the span: the box is no stacking context, so no sequence of the
/// snapshot holds the span's entry, and its IFC root draws it.
///
/// Kills: the root leaving the span out in a snapshot (white, the first
/// version of #2819).
#[test]
fn a_drag_ghost_of_a_plain_box_draws_its_relative_span() {
    for span in [REL, "background: rgb(0, 0, 255);"] {
        let mut doc = document(
            "width: 400px; font: 40px/100px ProbeFace;",
            &format!(r#"<div id="ghost">xx<span style="{span}">MMMM</span>xx</div>"#),
        );
        let ghost = doc.query_selector("#ghost").unwrap();
        let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
        let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
        rinch_dom::paint::paint_subtree(
            &doc.tree,
            &mut painter,
            ghost.0,
            1.0,
            &mut doc.font_cx,
            &mut layout_cx,
        );
        assert_near(
            at(&painter, (70, 70), 1.0),
            BLUE,
            &format!("ghost, span {span}"),
        );
    }
}
