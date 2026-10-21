#![cfg(feature = "software-renderer")]
#![allow(dead_code)]

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

// ── Review of #1673: shapes the box test does not decide as Chrome 155 does ──

/// KNOWN DIFFERENCE (#1675), pinned as rinch draws it today.
///
/// Chrome 155 splits one text node into two fragments at a bidi boundary:
/// in `<p style="text-shadow:-29px 0 0 blue">Aא</p>` (48px monospace) the
/// shadow of א is drawn over A, exactly as with `A<span>א</span>` (measured:
/// identical pixels in A's cell). rinch takes the one-text-node-one-line fast
/// path and draws every shadow first.
#[test]
fn r1_known_difference_1675_a_bidi_boundary_inside_one_text_node_is_no_fragment_boundary() {
    let html = "<p class=\"t s\">A\u{5d0}</p>";
    let off = offset_onto(html, 2, 0, 1);
    let draws = record(&mut doc_with(&shadow_css("s", off), html));
    let main = main_glyphs(&draws);
    let first_text = draw_of(&draws, BLACK, main[0]);
    let last_red = draws.iter().rposition(|d| d.color == RED).unwrap();
    // Chrome: `last_red > first_text`. rinch: one pass (#1675).
    assert!(
        last_red < first_text,
        "#1675 fixed? update this pin: {draws:?}"
    );
}

/// KNOWN DIFFERENCE (#1675), pinned as rinch draws it today.
///
/// Chrome 155 paints a line's fragments in visual order: in an LTR paragraph
/// `<span>א</span><span>ב</span>` shows ב left of א, and the shadow of א
/// (-29px) is drawn OVER ב (measured). rinch orders fragments by text, so ב
/// (later in the text) is drawn last and stays on top.
#[test]
fn r2_known_difference_1675_fragments_are_painted_in_text_order() {
    let html = "<p class=\"t s\"><span>\u{5d0}</span><span>\u{5d1}</span></p>";
    let plain = main_glyphs(&record(&mut doc_with("", html)));
    assert_eq!(plain.len(), 2);
    // Alef (first in the text) is shown on the right.
    let (bet, alef) = if plain[0].x < plain[1].x {
        (plain[0], plain[1])
    } else {
        (plain[1], plain[0])
    };
    let off = (bet.x - alef.x, 0.0);
    let draws = record(&mut doc_with(&shadow_css("s", off), html));
    let bet_text = draw_of(&draws, BLACK, bet);
    let alef_shadow = draw_of(&draws, RED, bet);
    // Chrome: `alef_shadow > bet_text`. rinch: text order (#1675).
    assert!(
        alef_shadow < bet_text,
        "#1675 fixed? update this pin ({alef_shadow} >= {bet_text}): {draws:?}"
    );
}

/// A raised span's glyphs are drawn above its line box (rinch does not grow
/// the line for `vertical-align`, #1357), so the box test moves each
/// cluster's box by its shift. A later shadow laid on the raised glyph is
/// drawn over it, as Chrome 155 does (measured, `vertical-align: 70px`,
/// offset -29px -70px).
///
/// Kills: the box left at the line's ascent and descent ("no overlap"); the
/// `vertical-align` cursor advanced only past shown glyph runs (the raised A
/// drawn at the baseline on the per-fragment path).
#[test]
fn r3_a_shadow_on_a_raised_glyph_is_drawn_over_it() {
    let html = "<p class=\"t\">X<span style=\"vertical-align: 70px\">A</span><span class=\"s\">B</span></p>";
    let off = offset_onto(html, 3, 1, 2);
    assert!(off.1 < -60.0, "A is raised: {off:?}");
    let draws = record(&mut doc_with(&shadow_css("s", off), html));
    let main = main_glyphs(&draws);
    let a_text = draw_of(&draws, BLACK, main[1]);
    let b_shadow = draw_of(&draws, RED, main[1]);
    assert!(
        b_shadow > a_text,
        "B's shadow over the raised A ({b_shadow} <= {a_text}): {draws:?}"
    );
}

/// Cost: each fragment's blurred mask covers its whole LINE (`text_extent`
/// measures every run on a line the mask may show), so a line of N spans
/// rasterises and blurs N line-wide masks, and fills N line-wide images on
/// every warm paint. Measured (release, 30 nowrap lines of 12 spans,
/// `0 1px 4px`): base 30 masks, 11.9 ms cold / 5.5 ms warm; head 618 masks,
/// 72.6 ms cold / 25.5 ms warm.
#[test]
fn r4_a_fragments_mask_is_no_wider_than_its_fragment() {
    let mut html = String::from(
        "<p class=\"t\" style=\"white-space: nowrap; font-size: 14px; line-height: 18px; text-shadow: 0 1px 4px rgb(255, 0, 0)\">",
    );
    for k in 0..12 {
        html.push_str(&format!("<span>w{k}</span> "));
    }
    html.push_str("</p>");
    let mut doc = doc_with("", &html);
    rinch_dom::paint::clear_text_shadow_cache();
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut cx,
    );
    let n = doc
        .tree
        .perf
        .get(rinch_dom::perf::Counter::TextShadowMasksRasterised);
    let line = main_glyphs(&record(&mut doc));
    let line_w = line.last().unwrap().x - line[0].x;
    let (w, _) = rinch_dom::paint::text_shadow_scratch_size().unwrap();
    assert!(
        (w as f64) < line_w / 2.0,
        "{n} masks, the widest {w}px for a {line_w}px line"
    );
}

/// A `visibility: hidden` span between two fragments, with a shadow that
/// reaches back (so the per-fragment path runs): its text and its shadow
/// stay undrawn. Pins the hidden check in `in_paint_order` (mutant: hidden
/// fragments pushed — survives the PR's own suite).
#[test]
fn r5_a_hidden_span_stays_undrawn_on_the_per_fragment_path() {
    let html = r#"<p class="t s">H<span style="visibility: hidden">H</span>H</p>"#;
    let draws = record(&mut doc_with(
        ".s { text-shadow: -30px 0 0 rgb(255, 0, 0); }",
        html,
    ));
    let black: usize = draws
        .iter()
        .filter(|d| d.color == BLACK)
        .map(|d| d.glyphs.len())
        .sum();
    let red: usize = draws
        .iter()
        .filter(|d| d.color == RED)
        .map(|d| d.glyphs.len())
        .sum();
    let first_black = draws.iter().position(|d| d.color == BLACK).unwrap();
    let last_red = draws.iter().rposition(|d| d.color == RED).unwrap();
    assert!(
        last_red > first_black,
        "the per-fragment path ran: {draws:?}"
    );
    assert_eq!((black, red), (2, 2), "{draws:?}");
}

/// A blurred shadow whose offset alone misses the line above but whose blur
/// reaches it (3px down, 6px blur: about 7.3px of reach above the 2-level
/// tolerance) is drawn per line. Pins the
/// blur reach in the box test (mutant: reach 0 — survives the PR's rinch-dom
/// suite; only rinch-bench's mask count catches it).
#[test]
fn r6_a_blur_reaching_the_line_above_is_drawn_per_line() {
    let prev = rinch_dom::paint::force_tapped_text_shadows(true);
    let html = r#"<p class="t s" style="width: 40px">H H</p>"#;
    let draws = record(&mut doc_with(
        ".s { text-shadow: 0 3px 6px rgb(255, 0, 0); }",
        html,
    ));
    rinch_dom::paint::force_tapped_text_shadows(prev);
    let first_black = draws.iter().position(|d| d.color == BLACK).unwrap();
    let last_red = draws.iter().rposition(|d| d.color == RED).unwrap();
    assert!(
        last_red > first_black,
        "{:?}",
        draws.iter().map(|d| d.color).collect::<Vec<_>>()
    );
}
