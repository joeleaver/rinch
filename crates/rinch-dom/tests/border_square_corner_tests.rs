//! Issue #1022 — a square box's thick border must fill its outer corners.
//!
//! A border (or outline) with the same width, colour and style on all four
//! sides is painted as **one stroke** around the rectangle through the middle
//! of the border band. kurbo's `Stroke::new` defaults to `Join::Round`, and a
//! round join at a 90° corner leaves the outer corner square of the band — a
//! quarter-disc's complement, `half × half` in size — unpainted. At 20px that
//! is the 3x3-ish notch the issue measured; at 50px on a 100px box the whole
//! border reads as rounded. Chrome 153 fills the corners (the band is the
//! border box minus the padding box, a rectangle with square corners).
//!
//! These are **local** pixel oracles on the software painter: every sampled
//! pixel sits inside the border box, where the correct output is provably the
//! border colour, and the surface starts transparent. The corner pixel itself
//! is sampled, because a bevel join paints the pixels a round join paints
//! *plus* a triangle, and still leaves the exact corner pixel empty — so a
//! sample one pixel in from the corner would let `Join::Bevel` pass.
//!
//! The GPU painter is handed the very same `kurbo::Stroke` (Vello's
//! `stroke` passes it straight through), so a recording painter pins the join
//! the painters receive, backend-neutrally.

use peniko::kurbo::{Affine, Join, Stroke};
use peniko::{Brush, Fill, FontData};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 400.0;
const VH: f32 = 400.0;

const GREEN: [u8; 4] = [0, 128, 0, 255];

fn document(style: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let id: NodeId = doc.create_element("div");
    doc.set_attribute(id, "style", style);
    doc.append_child(body, id);
    doc.resolve_layout(VW, VH);
    doc
}

fn rasterize(doc: &mut RinchDocument) -> Vec<u8> {
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter.pixels().to_vec()
}

fn px(p: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * VW as u32 + x) * 4) as usize;
    [p[i], p[i + 1], p[i + 2], p[i + 3]]
}

/// The box of the issue: 100x100 border box at (150, 150).
const BOX: &str = "position: absolute; left: 150px; top: 150px; box-sizing: border-box; \
                   width: 100px; height: 100px; ";

/// The four outer corner pixels of the 100x100 box at (150, 150).
const CORNERS: [(u32, u32); 4] = [(150, 150), (249, 150), (249, 249), (150, 249)];

fn assert_corners(p: &[u8], expected: [u8; 4], what: &str) {
    for (x, y) in CORNERS {
        assert_eq!(
            px(p, x, y),
            expected,
            "{what}: the outer corner pixel ({x}, {y}) must be the border colour, as Chrome \
             paints it; an unpainted corner is the round join of #1022"
        );
    }
}

/// The issue's own fixture: a uniform 20px solid border. Chrome 153:
/// (150,150), (151,151), (249,249), (150,249) all rgb(0,128,0).
#[test]
fn a_uniform_solid_border_fills_its_outer_corners() {
    let mut doc = document(&format!("{BOX}border: 20px solid rgb(0, 128, 0)"));
    let p = rasterize(&mut doc);
    assert_corners(&p, GREEN, "border: 20px solid");
    assert_eq!(px(&p, 151, 151), GREEN);
    assert_eq!(px(&p, 153, 153), GREEN);
    // The padding box is untouched: the band is 20px, not more.
    assert_eq!(px(&p, 171, 171)[3], 0, "the content area is not painted");
}

/// The issue's large case: a 50px border fills the whole 100px box. With a
/// round join it read as a circle-cornered square.
#[test]
fn a_border_that_fills_the_box_is_a_full_square() {
    let mut doc = document(&format!("{BOX}border: 50px solid rgb(0, 128, 0)"));
    let p = rasterize(&mut doc);
    assert_corners(&p, GREEN, "border: 50px solid");
    for (x, y) in [(152, 152), (160, 155), (245, 245), (155, 240)] {
        assert_eq!(px(&p, x, y), GREEN, "({x}, {y}) inside the border box");
    }
}

/// An outline takes the same single-stroke path (`paint_outline` →
/// `make_border_stroke`) around a rectangle outside the border box, and
/// Chrome paints a square box's outline with square corners too.
#[test]
fn a_uniform_solid_outline_fills_its_outer_corners() {
    // Outline 20px outside the 100x100 box: its outer edge is (130,130)-(270,270).
    let mut doc = document(&format!("{BOX}outline: 20px solid rgb(0, 128, 0)"));
    let p = rasterize(&mut doc);
    for (x, y) in [(130, 130), (269, 130), (269, 269), (130, 269)] {
        assert_eq!(
            px(&p, x, y),
            GREEN,
            "outline: 20px solid — outer corner pixel ({x}, {y}) (#1022)"
        );
    }
}

/// Different widths per side take the per-side path (four butt-capped lines
/// running the full box width), which already filled its corners at HEAD. A
/// guard that the fix does not route them anywhere worse.
#[test]
fn different_widths_per_side_fill_every_corner() {
    let mut doc = document(&format!(
        "{BOX}border-style: solid; border-color: rgb(0, 128, 0); \
         border-width: 20px 10px 30px 5px"
    ));
    let p = rasterize(&mut doc);
    assert_corners(&p, GREEN, "border-width: 20px 10px 30px 5px");
}

/// A rounded border keeps its rounded outer corner: the corner pixel of a
/// `border-radius: 30px` box is outside the curve and stays unpainted.
#[test]
fn a_rounded_border_keeps_its_rounded_corner() {
    let mut doc = document(&format!(
        "{BOX}border: 20px solid rgb(0, 128, 0); border-radius: 30px"
    ));
    let p = rasterize(&mut doc);
    for (x, y) in CORNERS {
        assert_eq!(
            px(&p, x, y)[3],
            0,
            "a 30px radius cuts the outer corner pixel ({x}, {y})"
        );
    }
    // …while the band's straight middle is painted.
    assert_eq!(px(&p, 200, 155), GREEN);
}

// ── Backend-neutral: the join both painters are handed ──────────────────────

#[derive(Default)]
struct Joins(Vec<(Join, f64)>);

impl Painter for Joins {
    fn reset(&mut self) {
        self.0.clear();
    }
    fn fill(&mut self, _: Fill, _: Affine, _: &Brush, _: &PaintShape) {}
    fn stroke(&mut self, s: &Stroke, _: Affine, _: &Brush, _: &PaintShape) {
        self.0.push((s.join, s.width));
    }
    #[allow(clippy::too_many_arguments)]
    fn draw_glyphs(
        &mut self,
        _: &FontData,
        _: f32,
        _: Affine,
        _: Option<Affine>,
        _: &Brush,
        _: bool,
        _: &[i16],
        _: &[PaintGlyph],
    ) {
    }
    fn draw_image(&mut self, _: &PaintImage<'_>, _: Affine) {}
    fn push_clip(&mut self, _: Fill, _: Affine, _: &PaintShape) {}
    fn push_layer(&mut self, _: BlendMode, _: f32, _: Affine, _: &PaintShape) {}
    fn pop_layer(&mut self) {}
}

fn joins(style: &str) -> Vec<(Join, f64)> {
    let mut doc = document(style);
    let mut painter = Joins::default();
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (VW, VH),
        &mut doc.font_cx,
        &mut cx,
    );
    painter.0
}

/// Solid and dashed borders reach the painter with a miter join — on the
/// Vello path as on tiny-skia, since both are handed this `Stroke`.
#[test]
fn solid_and_dashed_border_strokes_carry_a_miter_join() {
    for style in ["solid", "dashed"] {
        let got = joins(&format!("{BOX}border: 20px {style} rgb(0, 128, 0)"));
        assert_eq!(got.len(), 1, "{style}: one stroke for a uniform border");
        assert!(
            matches!(got[0].0, Join::Miter),
            "{style}: a square corner needs a miter join, got {:?} (#1022)",
            got[0].0
        );
    }
}

/// Dotted keeps its round join, as its round caps: a dot is round.
#[test]
fn dotted_border_strokes_keep_a_round_join() {
    let got = joins(&format!("{BOX}border: 20px dotted rgb(0, 128, 0)"));
    assert_eq!(got.len(), 1);
    assert!(matches!(got[0].0, Join::Round), "got {:?}", got[0].0);
}
