//! #1060 — `text-shadow` is painted shadow-then-text per text **fragment**
//! (one text node's stretch of one line), as Chrome does.
//!
//! Desktop drew every shadow of an inline formatting context under all of its
//! text. Chrome 155 (measured, `headless=new`, 48px monospace) paints each
//! fragment's shadows and then its text before the next fragment's, so a
//! later fragment's shadow reaching back is drawn **over** earlier text:
//!
//! | markup | the later shadow over the earlier text? |
//! |---|---|
//! | `<p style="text-shadow: -29px 0 0 blue">AB</p>` (one text node, one line) | no |
//! | `<p style="text-shadow: -29px 0 0 blue">A<span>B</span></p>` | yes |
//! | `<p>A<span style="text-shadow: -29px 0 0 red">B</span></p>` | yes |
//! | `<p style="width: 40px; text-shadow: 0 -60px 0 green">I W</p>` (one text node, two lines) | yes |
//! | `<div style="text-shadow: 0 -60px 0 red">I<br>W</div>` | yes |
//!
//! Two oracles. `Recorder` sees the order the painter is handed the draws in —
//! the one order both painters get from `paint_document`, the Vello scene's
//! included. The pixel fixtures paint with tiny-skia and look where the
//! answer is one colour: a shadow laid exactly over an earlier glyph of the
//! same shape, whose core pixels are red when the shadow is on top and black
//! when the text is.
//!
//! The mutant each fixture kills is named on it.

#![cfg(feature = "software-renderer")]

use peniko::kurbo::{Affine, Point, Stroke};
use peniko::{Brush, Fill, FontData};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 400.0;
const VH: f32 = 240.0;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

const BASE_CSS: &str = "
    .t { margin: 20px 0 0 30px; font-size: 40px; line-height: 50px;
         color: rgb(0, 0, 0); font-family: ProbeFace; }
";

fn doc_with(css: &str, html: &str) -> RinchDocument {
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
    doc.load_css(BASE_CSS);
    doc.load_css(css);
    let body = doc.body();
    doc.set_inner_html(body, html);
    doc.resolve_layout(VW, VH);
    doc
}

fn rgb(brush: &Brush) -> [u8; 3] {
    let Brush::Solid(c) = brush else {
        panic!("text is painted with a solid brush");
    };
    let [r, g, b, _] = c.to_rgba8().to_u8_array();
    [r, g, b]
}

const BLACK: [u8; 3] = [0, 0, 0];
const RED: [u8; 3] = [255, 0, 0];

/// One `draw_glyphs` call: its colour and its glyph positions.
#[derive(Debug, Clone)]
struct Draw {
    color: [u8; 3],
    glyphs: Vec<Point>,
}

#[derive(Default)]
struct Recorder {
    draws: Vec<Draw>,
}

impl Painter for Recorder {
    fn reset(&mut self) {
        self.draws.clear();
    }
    fn fill(&mut self, _: Fill, _: Affine, _: &Brush, _: &PaintShape) {}
    fn stroke(&mut self, _: &Stroke, _: Affine, _: &Brush, _: &PaintShape) {}
    #[allow(clippy::too_many_arguments)]
    fn draw_glyphs(
        &mut self,
        _: &FontData,
        _: f32,
        transform: Affine,
        _: Option<Affine>,
        brush: &Brush,
        _: bool,
        _: &[i16],
        glyphs: &[PaintGlyph],
    ) {
        if glyphs.is_empty() {
            return;
        }
        self.draws.push(Draw {
            color: rgb(brush),
            glyphs: glyphs
                .iter()
                .map(|g| transform * Point::new(g.x as f64, g.y as f64))
                .collect(),
        });
    }
    fn draw_image(&mut self, _: &PaintImage<'_>, _: Affine) {}
    fn push_clip(&mut self, _: Fill, _: Affine, _: &PaintShape) {}
    fn push_layer(&mut self, _: BlendMode, _: f32, _: Affine, _: &PaintShape) {}
    fn pop_layer(&mut self) {}
}

fn record(doc: &mut RinchDocument) -> Vec<Draw> {
    let mut painter = Recorder::default();
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut cx,
    );
    painter.draws
}

/// The main (black) glyph positions, in the order drawn.
fn main_glyphs(draws: &[Draw]) -> Vec<Point> {
    draws
        .iter()
        .filter(|d| d.color == BLACK)
        .flat_map(|d| d.glyphs.clone())
        .collect()
}

/// The index of the first draw holding a glyph of `color` at `at`.
fn draw_of(draws: &[Draw], color: [u8; 3], at: Point) -> usize {
    draws
        .iter()
        .position(|d| {
            d.color == color
                && d.glyphs
                    .iter()
                    .any(|g| (g.x - at.x).abs() < 0.01 && (g.y - at.y).abs() < 0.01)
        })
        .unwrap_or_else(|| panic!("no {color:?} glyph at {at:?}: {draws:?}"))
}

/// The offset that lays glyph `to`'s shadow exactly over glyph `from` of the
/// same markup with no shadow: `from - to`, in CSS px.
fn offset_onto(html: &str, glyphs: usize, from: usize, to: usize) -> (f64, f64) {
    let main = main_glyphs(&record(&mut doc_with("", html)));
    assert_eq!(main.len(), glyphs, "one glyph per character: {main:?}");
    (main[from].x - main[to].x, main[from].y - main[to].y)
}

fn shadow_css(class: &str, (dx, dy): (f64, f64)) -> String {
    format!(".{class} {{ text-shadow: {dx}px {dy}px 0 rgb(255, 0, 0); }}")
}

// ── The order the painter is handed ─────────────────────────────────────────

/// The issue's third row: a span's shadow reaching back over the text before
/// it is drawn after that text.
///
/// Kills: every shadow drawn before any text (HEAD); the reach-back test
/// answering "no" (`may_change_pixels` → `false`).
#[test]
fn a_spans_shadow_reaching_back_is_drawn_over_the_text_before_it() {
    let html = r#"<p class="t">H<span class="s">H</span></p>"#;
    let off = offset_onto(html, 2, 0, 1);
    assert!(off.0 < -10.0, "the second H is to the right: {off:?}");
    let draws = record(&mut doc_with(&shadow_css("s", off), html));
    let main = main_glyphs(&draws);
    let first_text = draw_of(&draws, BLACK, main[0]);
    let shadow = draw_of(&draws, RED, main[0]);
    assert!(
        shadow > first_text,
        "the span's shadow (draw {shadow}) is drawn after the text it covers \
         (draw {first_text}): {draws:?}"
    );
}

/// The issue's second row: the paragraph's own list, inherited by a span — two
/// text nodes, one list. Each fragment draws its shadow then its text.
///
/// Kills: interleaving keyed by shadow list rather than by text node (one
/// list here, so it would take the one-pass order).
#[test]
fn an_inherited_list_is_still_drawn_per_text_node() {
    let html = r#"<p class="t s">H<span>H</span></p>"#;
    let off = offset_onto(html, 2, 0, 1);
    let draws = record(&mut doc_with(&shadow_css("s", off), html));
    let main = main_glyphs(&draws);
    // The first H's own shadow lands left of it; the second H's on it.
    let first_text = draw_of(&draws, BLACK, main[0]);
    let second_shadow = draw_of(&draws, RED, main[0]);
    assert!(
        second_shadow > first_text,
        "draws: {draws:?} ({second_shadow} <= {first_text})"
    );
}

/// The fourth row: one text node over two lines. A line's shadow reaching up
/// is drawn over the line above it.
///
/// Kills: fragments made per text node and not per line (one node here).
#[test]
fn a_second_lines_shadow_reaching_up_is_drawn_over_the_first_line() {
    // Three glyphs: the space stays on the first line.
    let html = r#"<p class="t s" style="width: 40px">H H</p>"#;
    let off = offset_onto(html, 3, 0, 2);
    assert!(off.1 < -40.0, "the second H is on the next line: {off:?}");
    let draws = record(&mut doc_with(&shadow_css("s", off), html));
    let main = main_glyphs(&draws);
    let first_text = draw_of(&draws, BLACK, main[0]);
    let second_shadow = draw_of(&draws, RED, main[0]);
    assert!(
        second_shadow > first_text,
        "draws: {draws:?} ({second_shadow} <= {first_text})"
    );
}

/// The first row: one text node on one line keeps the one-pass order — the
/// later glyph's shadow under the earlier glyph — and costs exactly one shadow
/// draw and one text draw.
///
/// Kills: fragments made per glyph or per cluster; any extra draw call in the
/// common case.
#[test]
fn one_text_node_on_one_line_keeps_one_shadow_draw_under_one_text_draw() {
    let html = r#"<p class="t s">HH</p>"#;
    let off = offset_onto(html, 2, 0, 1);
    let draws = record(&mut doc_with(&shadow_css("s", off), html));
    assert_eq!(draws.len(), 2, "{draws:?}");
    assert_eq!(draws[0].color, RED, "{draws:?}");
    assert_eq!(draws[1].color, BLACK, "{draws:?}");
}

/// Shadows that reach back over no earlier text keep the one-pass order —
/// every shadow, then the text — draw for draw: two spans whose shadows fall
/// below the line, two spans whose shadows fall to their right, and two text
/// nodes whose shadows overlap each other but are all one colour (painting
/// one colour over another in either order gives the same pixels).
///
/// Kills: interleaving whenever an IFC holds more than one text node with a
/// shadow (the reach-back test skipped); a reach-back test on the vertical
/// axis only (the second case); shadows compared with earlier shadows even
/// when all are one colour (the third).
#[test]
fn shadows_that_reach_back_over_nothing_keep_the_one_pass_order() {
    for (css, html) in [
        (
            ".s { text-shadow: 3px 60px 0 rgb(255, 0, 0); }",
            r#"<p class="t">Ham<span class="s">burg</span> <span class="s">ok</span></p>"#,
        ),
        (
            ".s { text-shadow: 3px 0 0 rgb(255, 0, 0); }",
            r#"<p class="t">Ham<span class="s">burg</span> <span class="s">ok</span></p>"#,
        ),
        (
            ".s { text-shadow: 3px 0 0 rgb(255, 0, 0), 9px 0 0 rgb(255, 0, 0); }",
            r#"<p class="t s">H<span>H</span></p>"#,
        ),
    ] {
        let draws = record(&mut doc_with(css, html));
        let last_shadow = draws.iter().rposition(|d| d.color == RED).unwrap();
        let first_text = draws.iter().position(|d| d.color == BLACK).unwrap();
        assert!(
            last_shadow < first_text,
            "{css} {html}: every shadow before any text: {:?}",
            draws.iter().map(|d| d.color).collect::<Vec<_>>()
        );
    }
}

/// Shadows of two colours that overlap each other are drawn per fragment:
/// the later fragment's (both of them) over the earlier fragment's.
///
/// Kills: earlier shadows never compared (only earlier text).
#[test]
fn shadows_of_two_colours_overlapping_are_drawn_per_fragment() {
    let draws = record(&mut doc_with(
        ".s { text-shadow: 3px 0 0 rgb(255, 0, 0), 9px 0 0 rgb(0, 0, 255); }",
        r#"<p class="t s">H<span>H</span></p>"#,
    ));
    let main = main_glyphs(&draws);
    let first_text = draw_of(&draws, BLACK, main[0]);
    let second_shadow = draws
        .iter()
        .position(|d| {
            d.color == [0, 0, 255]
                && d.glyphs
                    .iter()
                    .any(|g| (g.x - main[1].x - 9.0).abs() < 0.01)
        })
        .unwrap();
    assert!(second_shadow > first_text, "{draws:?}");
}

/// A faint blurred tail that cannot change an 8-bit pixel decides nothing:
/// a 4px blur at alpha 0.1 reaches about 4.1px where it stays above half a
/// level (6px is where the mask is cut), so a `0 3px` offset leaves the line
/// above alone and the one-pass order is kept. The same blur at full alpha
/// reaches about 5.8px, into the line above, and is drawn per line.
///
/// Kills: the overlap decided with the mask's 3-sigma reach (the faint case
/// interleaves); the alpha left out of the visible reach.
#[test]
fn a_faint_blur_tail_keeps_the_one_pass_order() {
    let prev = rinch_dom::paint::force_tapped_text_shadows(true);
    let html = r#"<p class="t s" style="width: 40px">H H</p>"#;
    let order = |css: &str| {
        let draws = record(&mut doc_with(css, html));
        let first_text = draws.iter().position(|d| d.color == BLACK).unwrap();
        let last_shadow = draws.iter().rposition(|d| d.color == RED).unwrap();
        last_shadow > first_text
    };
    let faint = order(".s { text-shadow: 0 3px 4px rgba(255, 0, 0, 0.1); }");
    let full = order(".s { text-shadow: 0 3px 4px rgb(255, 0, 0); }");
    rinch_dom::paint::force_tapped_text_shadows(prev);
    assert!(!faint, "the faint tail keeps the one-pass order");
    assert!(full, "the opaque blur reaches the line above: per line");
}

/// What a reach-back must clear to change a pixel: a blur's visible reach
/// over the earlier text depends on how far the shadow's colour is from the
/// text's (painting a 40%-black shadow over `rgb(40, 40, 40)` text or under
/// it differs by at most 0.4 x 40 levels, times the blur's coverage), and a
/// space between two fragments has no ink. Here `burg`'s 4px blur reaches
/// about 3.7px over grey text — less than the 14px space before it — so the
/// one-pass order is kept.
///
/// Kills: the colour difference left out of the visible reach (5.2px, past
/// the space); a space's advance counted as ink.
#[test]
fn a_blur_that_cannot_reach_ink_of_another_colour_keeps_the_one_pass_order() {
    let prev = rinch_dom::paint::force_tapped_text_shadows(true);
    let draws = record(&mut doc_with(
        ".g { font-size: 14px; line-height: 18px; color: rgb(40, 40, 40); }
         .s { text-shadow: 0 0 4px rgba(0, 0, 0, 0.4); }",
        r#"<p class="t g">Ham <span class="s">burg</span></p>"#,
    ));
    rinch_dom::paint::force_tapped_text_shadows(prev);
    const GREY: [u8; 3] = [40, 40, 40];
    let first_text = draws.iter().position(|d| d.color == GREY).unwrap();
    let last_shadow = draws.iter().rposition(|d| d.color == BLACK).unwrap();
    assert!(
        last_shadow < first_text,
        "one pass: {:?}",
        draws.iter().map(|d| d.color).collect::<Vec<_>>()
    );
}

/// A `visibility: hidden` glyph run before a raised one in the same parley
/// run: the raised glyph keeps its `vertical-align` shift. The shift cursor
/// hands out a run's glyph runs in order and was skipped past hidden ones, so
/// the raised glyph took the hidden run's shifts (none) and sat on the
/// baseline. (Found while working on #1060; reachable on main with no shadow.)
///
/// Kills: `run_shifts` called after the hidden test's `continue`.
#[test]
fn a_raised_glyph_after_a_hidden_run_keeps_its_shift() {
    let html = r#"<p class="t">X<span style="visibility: hidden; color: rgb(0, 0, 255)">Y</span><span style="vertical-align: 20px">Z</span></p>"#;
    let main = main_glyphs(&record(&mut doc_with("", html)));
    assert_eq!(main.len(), 2, "X and Z drawn, Y hidden: {main:?}");
    assert!(
        (main[1].y - (main[0].y - 20.0)).abs() < 0.01,
        "Z is raised 20px above X: {main:?}"
    );
}

// ── The pixels ──────────────────────────────────────────────────────────────

/// Paint `doc` with tiny-skia over white and count the pixels around
/// `glyph` (its origin; a 40px H spans about 24 x 30 px above it) that are
/// near black: the earlier glyph's core is black only where its own text is
/// on top of the shadow laid exactly over it.
fn black_core_pixels(doc: &mut RinchDocument, glyph: Point) -> (usize, usize) {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    painter.fill_white();
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut cx,
    );
    let px = painter.pixels().as_chunks::<4>().0.to_vec();
    let (mut black, mut red) = (0, 0);
    let x0 = glyph.x.floor() as i64 - 4;
    let y0 = glyph.y.floor() as i64 - 36;
    for y in y0..y0 + 42 {
        for x in x0..x0 + 31 {
            let p = px[(y * VW as i64 + x) as usize];
            // Shadow on top: every covered pixel is at least 3/4 red (an
            // edge pixel of coverage `a` reads r = 255(a + (1 - a)^2)).
            if p[0] < 100 && p[1] < 100 && p[2] < 100 {
                black += 1;
            }
            if p[0] > 200 && p[1] < 60 && p[2] < 60 {
                red += 1;
            }
        }
    }
    (black, red)
}

/// The span and the line cases in pixels: the earlier H is wholly red.
///
/// Kills: every shadow drawn before any text (HEAD), on the software painter.
#[test]
fn the_earlier_glyph_under_a_later_shadow_is_red_in_pixels() {
    for (html, glyphs, to) in [
        (r#"<p class="t">H<span class="s">H</span></p>"#, 2, 1),
        (r#"<p class="t s">H<span>H</span></p>"#, 2, 1),
        (r#"<p class="t s" style="width: 40px">H H</p>"#, 3, 2),
    ] {
        let off = offset_onto(html, glyphs, 0, to);
        let mut doc = doc_with(&shadow_css("s", off), html);
        let main = main_glyphs(&record(&mut doc));
        let (black, red) = black_core_pixels(&mut doc, main[0]);
        assert!(
            red > 100,
            "{html}: the shadow is drawn there ({red} red px)"
        );
        assert_eq!(
            black, 0,
            "{html}: {black} black px of the earlier H show through the later shadow"
        );
    }
}

/// The first row in pixels: within one fragment the text stays on top, so
/// the earlier H's core is black.
///
/// Kills: fragments made per glyph.
#[test]
fn within_one_fragment_the_text_stays_on_top_in_pixels() {
    let html = r#"<p class="t s">HH</p>"#;
    let off = offset_onto(html, 2, 0, 1);
    let mut doc = doc_with(&shadow_css("s", off), html);
    let main = main_glyphs(&record(&mut doc));
    let (black, _) = black_core_pixels(&mut doc, main[0]);
    assert!(black > 100, "the H's own text is on top ({black} black px)");
}
