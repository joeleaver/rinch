//! #1048 — a `text-shadow` belongs to the text of the element that declares
//! it, not only to the IFC root's.
//!
//! Desktop paint took one shadow list, the IFC root's, and drew it under the
//! whole inline layout. A `<span>` that declared its own `text-shadow` (or
//! `text-shadow: none` under a shadowed paragraph) therefore painted exactly
//! what its paragraph did. `text-shadow` is inherited, so each text run casts
//! the list its **DOM parent** element computed — the paragraph's, unless an
//! inline element on the way down declared another — the same per-range rule
//! the #829 `TextMask` applies to `visibility`.
//!
//! # The oracle
//!
//! `Recorder` sees what the painter is handed: a hard shadow is a copy of the
//! glyphs in the shadow's colour. Every text-bearing element here differs from
//! its neighbours only in `text-shadow`, which Parley does not see, so the line
//! is one glyph run and its main glyphs are in text order: the glyphs a shadow
//! copy holds must be exactly the main glyphs of the text that casts it, moved
//! by its offset. Declared `font-size` / `line-height` and the bundled Inter
//! keep every position off the local font set.
//!
//! The mutant each fixture kills is named on it.

#![cfg(feature = "software-renderer")]

use peniko::kurbo::{Affine, Point, Rect, Stroke};
use peniko::{Brush, Fill, FontData};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 400.0;
const VH: f32 = 200.0;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

const CSS: &str = "
    .t { width: 360px; margin: 20px 0 0 30px; font-size: 24px; line-height: 28px;
         color: rgb(0, 0, 0); font-family: ProbeFace; }
    .red { text-shadow: 0 40px 0 rgb(255, 0, 0); }
    .blue { text-shadow: 5px 40px 0 rgb(0, 0, 255); }
    .green { text-shadow: -4px 40px 0 rgb(0, 200, 0); }
    .none { text-shadow: none; }
";

fn doc_with(html: &str) -> RinchDocument {
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
    doc.load_css(CSS);
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
const BLUE: [u8; 3] = [0, 0, 255];
const GREEN: [u8; 3] = [0, 200, 0];

#[derive(Debug)]
enum Op {
    Glyphs { color: [u8; 3], glyphs: Vec<Point> },
    Stroke { color: [u8; 3], bbox: Rect },
    Push { bounds: Rect },
    Image { rect: Rect },
    Pop,
}

#[derive(Default)]
struct Recorder {
    ops: Vec<Op>,
}

impl Painter for Recorder {
    fn reset(&mut self) {
        self.ops.clear();
    }
    fn fill(&mut self, _: Fill, _: Affine, _: &Brush, _: &PaintShape) {}
    fn stroke(&mut self, _: &Stroke, transform: Affine, brush: &Brush, shape: &PaintShape) {
        let bbox = match shape {
            PaintShape::Line(l) => Rect::from_points(transform * l.p0, transform * l.p1),
            other => transform.transform_rect_bbox(other.bounding_box()),
        };
        self.ops.push(Op::Stroke {
            color: rgb(brush),
            bbox,
        });
    }
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
        self.ops.push(Op::Glyphs {
            color: rgb(brush),
            glyphs: glyphs
                .iter()
                .map(|g| transform * Point::new(g.x as f64, g.y as f64))
                .collect(),
        });
    }
    fn draw_image(&mut self, image: &PaintImage<'_>, transform: Affine) {
        self.ops.push(Op::Image {
            rect: transform.transform_rect_bbox(Rect::new(
                0.0,
                0.0,
                image.width as f64,
                image.height as f64,
            )),
        });
    }
    fn push_clip(&mut self, _: Fill, _: Affine, _: &PaintShape) {}
    fn push_layer(&mut self, _: BlendMode, _: f32, t: Affine, bounds: &PaintShape) {
        self.ops.push(Op::Push {
            bounds: t.transform_rect_bbox(bounds.bounding_box()),
        });
    }
    fn pop_layer(&mut self) {
        self.ops.push(Op::Pop);
    }
}

fn record(doc: &mut RinchDocument, scale: f64) -> Vec<Op> {
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
    painter.ops
}

/// Every glyph drawn in `color`, in the order drawn.
fn glyphs_in(ops: &[Op], color: [u8; 3]) -> Vec<Point> {
    ops.iter()
        .filter_map(|op| match op {
            Op::Glyphs { color: c, glyphs } if *c == color => Some(glyphs.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// `main[range]` moved by `(dx, dy)` CSS px at `scale`.
fn moved(
    main: &[Point],
    range: std::ops::Range<usize>,
    dx: f64,
    dy: f64,
    scale: f64,
) -> Vec<Point> {
    main[range]
        .iter()
        .map(|p| Point::new(p.x + dx * scale, p.y + dy * scale))
        .collect()
}

fn assert_points(got: &[Point], want: &[Point], what: &str) {
    assert_eq!(
        got.len(),
        want.len(),
        "{what}: {} shadow glyphs where {} cast one\n got {got:?}\nwant {want:?}",
        got.len(),
        want.len()
    );
    for (g, w) in got.iter().zip(want) {
        assert!(
            (g.x - w.x).abs() < 0.01 && (g.y - w.y).abs() < 0.01,
            "{what}: shadow glyph at {g:?}, want {w:?}\n got {got:?}\nwant {want:?}"
        );
    }
}

/// The main (black) glyphs, which must be one glyph per character here.
fn main_glyphs(ops: &[Op], chars: usize) -> Vec<Point> {
    let main = glyphs_in(ops, BLACK);
    assert_eq!(main.len(), chars, "one glyph per character: {ops:?}");
    main
}

// ── Whose shadow ─────────────────────────────────────────────────────────────

/// The issue's table: a span declares the shadow, the paragraph does not. The
/// span's glyphs — and only those — cast it; whole span, trailing span and
/// leading span alike, at scale 1 and off it.
///
/// Kills: the IFC root's list used for every run (no red at all — the bug);
/// a shadow group drawn without its range mask (every glyph cast red).
#[test]
fn a_span_casts_its_own_shadow() {
    for scale in [1.0, 1.5] {
        for (html, range, chars) in [
            (
                r#"<p class="t"><span class="red">Hamburg</span></p>"#,
                0..7,
                7,
            ),
            (
                r#"<p class="t">Ham<span class="red">burg</span></p>"#,
                3..7,
                7,
            ),
            (
                r#"<p class="t"><span class="red">Ham</span>burg</p>"#,
                0..3,
                7,
            ),
        ] {
            let ops = record(&mut doc_with(html), scale);
            let main = main_glyphs(&ops, chars);
            assert_points(
                &glyphs_in(&ops, RED),
                &moved(&main, range, 0.0, 40.0, scale),
                &format!("{html} at scale {scale}"),
            );
        }
    }
}

/// `text-shadow: none` on a span under a shadowed paragraph: the span's text
/// casts nothing, the rest casts the paragraph's.
///
/// Kills: the root's list drawn over the whole layout (the span casts it too).
#[test]
fn a_span_with_none_casts_nothing_under_a_shadowed_paragraph() {
    let ops = record(
        &mut doc_with(r#"<p class="t red">Ham<span class="none">bu</span>rg</p>"#),
        1.0,
    );
    let main = main_glyphs(&ops, 7);
    let mut want = moved(&main, 0..3, 0.0, 40.0, 1.0);
    want.extend(moved(&main, 5..7, 0.0, 40.0, 1.0));
    let mut got = glyphs_in(&ops, RED);
    got.sort_by(|a, b| a.x.total_cmp(&b.x));
    assert_points(&got, &want, "the paragraph's text only");
}

/// Three lists in one line: the paragraph's red, a span's blue, another's
/// green — each at its own offset, under its own text.
///
/// Kills: groups keyed by colour alone or by offset alone (a mask built for one
/// group used for another); the ranges assigned to the wrong group.
#[test]
fn each_span_casts_its_own_colour_and_offset() {
    let ops = record(
        &mut doc_with(
            r#"<p class="t red">ab<span class="blue">cd</span>ef<span class="green">gh</span></p>"#,
        ),
        1.0,
    );
    let main = main_glyphs(&ops, 8);
    let mut red = glyphs_in(&ops, RED);
    red.sort_by(|a, b| a.x.total_cmp(&b.x));
    let mut want = moved(&main, 0..2, 0.0, 40.0, 1.0);
    want.extend(moved(&main, 4..6, 0.0, 40.0, 1.0));
    assert_points(&red, &want, "red");
    assert_points(
        &glyphs_in(&ops, BLUE),
        &moved(&main, 2..4, 5.0, 40.0, 1.0),
        "blue",
    );
    assert_points(
        &glyphs_in(&ops, GREEN),
        &moved(&main, 6..8, -4.0, 40.0, 1.0),
        "green",
    );
}

/// Nested inline elements: text inside an inner span that declares nothing
/// inherits the outer span's shadow; an innermost `none` casts nothing.
///
/// Kills: the list read from the IFC root or from the outermost inline element
/// instead of the text's own parent.
#[test]
fn nested_spans_inherit_and_override() {
    let ops = record(
        &mut doc_with(
            r#"<p class="t">a<span class="blue">b<em style="font-style: normal">c<span class="none">d</span></em>e</span>f</p>"#,
        ),
        1.0,
    );
    let main = main_glyphs(&ops, 6);
    let mut blue = glyphs_in(&ops, BLUE);
    blue.sort_by(|a, b| a.x.total_cmp(&b.x));
    let mut want = moved(&main, 1..3, 5.0, 40.0, 1.0);
    want.extend(moved(&main, 4..5, 5.0, 40.0, 1.0));
    assert_points(
        &blue,
        &want,
        "b, c and e cast the outer span's shadow; d casts none",
    );
    assert!(glyphs_in(&ops, RED).is_empty());
}

/// Every shadow is drawn before any text, as with a paragraph's own list: the
/// main glyphs come last.
///
/// Kills: the text drawn per group (a later group's shadow over earlier text).
#[test]
fn every_shadow_is_under_every_glyph() {
    let ops = record(
        &mut doc_with(
            r#"<p class="t">Ham<span class="red">burg</span> x<span class="blue">y</span></p>"#,
        ),
        1.0,
    );
    let first_main = ops
        .iter()
        .position(|op| matches!(op, Op::Glyphs { color: BLACK, .. }))
        .expect("the text is drawn");
    let last_shadow = ops
        .iter()
        .rposition(|op| matches!(op, Op::Glyphs { color, .. } if *color != BLACK))
        .expect("the shadows are drawn");
    assert!(last_shadow < first_main, "{ops:?}");
}

/// A span's shadow is cut where its text is `visibility: hidden` (#829), and a
/// hidden span under a shadowed span casts nothing.
///
/// Kills: a group mask that forgets the visibility mask (the hidden glyphs cast
/// their shadow).
#[test]
fn a_hidden_run_inside_a_shadowed_span_casts_nothing() {
    let ops = record(
        &mut doc_with(
            r#"<p class="t">a<span class="red">bc<span style="visibility: hidden">de</span>f</span></p>"#,
        ),
        1.0,
    );
    // The hidden glyphs are not drawn at all: four main glyphs.
    let main = glyphs_in(&ops, BLACK);
    assert_eq!(main.len(), 4, "{ops:?}");
    let mut red = glyphs_in(&ops, RED);
    red.sort_by(|a, b| a.x.total_cmp(&b.x));
    assert_points(&red, &moved(&main, 1..4, 0.0, 40.0, 1.0), "b, c and f");
}

/// A decoration the paragraph draws casts a span's shadow only under the span.
///
/// Kills: the group mask not reaching the decorations (the whole underline
/// cast red).
#[test]
fn a_decoration_casts_the_span_shadow_under_the_span_only() {
    let ops = record(
        &mut doc_with(
            r#"<p class="t" style="text-decoration: underline">Hamb<span class="red">urg</span></p>"#,
        ),
        1.0,
    );
    let main = main_glyphs(&ops, 7);
    let strokes = |want: [u8; 3]| -> Vec<Rect> {
        ops.iter()
            .filter_map(|op| match op {
                Op::Stroke { color, bbox } if *color == want => Some(*bbox),
                _ => None,
            })
            .collect()
    };
    let red = strokes(RED);
    assert_eq!(red.len(), 1, "one red underline: {ops:?}");
    let underline = strokes(BLACK);
    assert!(
        !underline.is_empty(),
        "positive control: the underline is drawn"
    );
    let x0 = underline.iter().map(|r| r.x0).fold(f64::INFINITY, f64::min);
    // The red underline starts at the span's first glyph, not the line's.
    assert!(
        (red[0].x0 - main[4].x).abs() < 0.5 && red[0].x0 > x0 + 10.0,
        "red {:?} should start at the span's `u` ({}), not the line's start ({x0})",
        red[0],
        main[4].x
    );
}

/// A blurred span shadow draws (the issue's third row), as one image placed
/// under the span's text — not under the paragraph's start.
///
/// Kills: no shadow for a span (no image); the group mask missing from the
/// rasterised copy (the image reaches back over `Ham`).
#[test]
fn a_blurred_span_shadow_is_drawn_under_the_span() {
    let mut doc = doc_with(
        r#"<p class="t">Hamburg <span style="text-shadow: 0 40px 4px rgb(255, 0, 0)">xyz</span></p>"#,
    );
    let ops = record(&mut doc, 1.0);
    let main = main_glyphs(&ops, 11);
    let images: Vec<Rect> = ops
        .iter()
        .filter_map(|op| match op {
            Op::Image { rect } => Some(*rect),
            _ => None,
        })
        .collect();
    assert_eq!(images.len(), 1, "{ops:?}");

    // The pixels: red ink, and none of it left of `x`'s shadow's reach.
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
    let mut min_x = u32::MAX;
    let mut n = 0;
    for (i, p) in px.iter().enumerate() {
        if p[0] >= 254 && p[1] <= 240 && p[1].abs_diff(p[2]) <= 2 {
            n += 1;
            min_x = min_x.min(i as u32 % VW as u32);
        }
    }
    assert!(n > 50, "the blurred span shadow is drawn ({n} red px)");
    let reach = 1.5 * 4.0 + 2.0;
    assert!(
        min_x as f64 >= main[8].x - reach,
        "red ink at x {min_x}, left of `x`'s shadow ({} - {reach}): the paragraph's text cast it",
        main[8].x
    );
}

/// An opacity layer around the paragraph holds its span's shadow: Vello clips
/// a layer to its bounds (tiny-skia does not, so no pixel test sees this).
///
/// Kills: `layer_bounds` measuring the IFC root's own list only.
#[test]
fn an_opacity_layer_holds_a_span_shadow() {
    let ops = record(
        &mut doc_with(r#"<p class="t" style="opacity: 0.5">Ham<span class="blue">burg</span></p>"#),
        1.0,
    );
    let bounds = ops
        .iter()
        .find_map(|op| match op {
            Op::Push { bounds } => Some(*bounds),
            _ => None,
        })
        .expect("the opacity layer");
    let blue = glyphs_in(&ops, BLUE);
    assert_eq!(blue.len(), 4);
    for p in &blue {
        assert!(
            bounds.contains(*p),
            "the layer {bounds:?} cuts the span's shadow glyph at {p:?}"
        );
    }
}

/// A blurred span shadow's mask covers the span's lines, not the paragraph's:
/// a span on the second line rasterises no mask over the first.
///
/// Kills: `TextMask::may_show` answering `true` for every line (each group's
/// pass walks, and its mask spans, the whole paragraph — the cost the review
/// of #1063 measured at 298 ms for 400 distinct span lists).
#[test]
fn a_span_shadow_mask_covers_only_the_spans_lines() {
    let ops = record(
        &mut doc_with(
            r#"<p class="t" style="width: 120px">aaaa<br>bb<span style="text-shadow: 0 0 4px rgb(255, 0, 0)">cc</span></p>"#,
        ),
        1.0,
    );
    let images: Vec<Rect> = ops
        .iter()
        .filter_map(|op| match op {
            Op::Image { rect } => Some(*rect),
            _ => None,
        })
        .collect();
    assert_eq!(images.len(), 1, "{ops:?}");
    // Line 1 is 20..48, line 2 48..76; the blur reaches 6px, plus the ink
    // margin (a quarter of 24px) and a pixel of rounding.
    assert!(
        images[0].y0 >= 48.0 - 6.0 - 6.0 - 2.0,
        "the mask {:?} reaches up over the first line",
        images[0]
    );
}

// ── What pays for the per-run walk ───────────────────────────────────────────

/// A page where no inline element casts a list of its own never turns the
/// per-run walk on: a paragraph's own shadow, a block's, an inline that
/// inherits. One that does turns it on.
///
/// Kills: the flag set by any non-empty `text-shadow` (every page with a
/// shadowed heading pays the walk: the Perf job's `hover_frame` measured
/// +4.1% for the walk on a page with no shadow at all).
#[test]
fn only_an_inline_casting_its_own_list_turns_the_walk_on() {
    for html in [
        r#"<p class="t">Ham<span>burg</span></p>"#,
        r#"<p class="t red">Ham<span>burg</span></p>"#,
        r#"<div class="red"><p class="t">Ham<span>burg</span></p></div>"#,
        r#"<p class="t"><span style="display: inline-block" class="red">Ham</span></p>"#,
    ] {
        assert!(!doc_with(html).tree.inline_text_shadows, "{html}");
    }
    for html in [
        r#"<p class="t">Ham<span class="red">burg</span></p>"#,
        r#"<p class="t red">Ham<span class="none">burg</span></p>"#,
    ] {
        assert!(doc_with(html).tree.inline_text_shadows, "{html}");
    }
}

/// A `display: contents` wrapper's own list reaches its text.
///
/// Kills: the flag ignoring `display: contents` (the wrapper's text casts
/// nothing).
#[test]
fn a_contents_wrapper_casts_its_own_shadow() {
    let ops = record(
        &mut doc_with(
            r#"<p class="t">Ham<b style="display: contents; font-weight: normal" class="red">burg</b></p>"#,
        ),
        1.0,
    );
    let main = main_glyphs(&ops, 7);
    assert_points(
        &glyphs_in(&ops, RED),
        &moved(&main, 3..7, 0.0, 40.0, 1.0),
        "the wrapper's text",
    );
}

/// Generated content casts its pseudo-element's own list.
///
/// Kills: the pseudo-element cascade not turning the walk on (a page whose
/// only span shadow is on a `::before` draws none).
#[test]
fn a_before_pseudo_element_casts_its_own_shadow() {
    let mut doc = RinchDocument::new();
    {
        use parley::fontique::{Blob, FontInfoOverride};
        doc.font_cx.collection.register_fonts(
            Blob::new(std::sync::Arc::new(FACE)),
            Some(FontInfoOverride {
                family_name: Some("ProbeFace"),
                ..Default::default()
            }),
        );
    }
    doc.load_css(CSS);
    doc.load_css(".g::before { content: \"Ham\"; text-shadow: 0 40px 0 rgb(255, 0, 0); }");
    let body = doc.body();
    doc.set_inner_html(body, r#"<p class="t g">burg</p>"#);
    doc.resolve_layout(VW, VH);
    let ops = record(&mut doc, 1.0);
    let main = main_glyphs(&ops, 7);
    assert_points(
        &glyphs_in(&ops, RED),
        &moved(&main, 0..3, 0.0, 40.0, 1.0),
        "the generated text",
    );
}
