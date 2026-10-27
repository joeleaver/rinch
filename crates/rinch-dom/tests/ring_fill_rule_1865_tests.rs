//! #1865 D9: the rings rinch cuts a hole out of — a rounded solid border, an
//! elliptical solid outline, an outer box-shadow's layers and the clip round
//! its hole, an unblurred inset shadow — reach the painter as `Fill::NonZero`
//! with the hole wound against the outline, never as `Fill::EvenOdd`.
//!
//! Vello's MSAA even-odd fill races on its workgroup memory (the comment on
//! `punched` in `paint/borders.rs` has the detail): on lavapipe a rounded
//! border's corner came out as a 1/8-coverage wash, which the GPU fixture
//! `render_surface::compositor_routing_tests::
//! gpu_render_callback_frames_sit_in_the_content_box` caught once CI ran it
//! there. That fixture needs an adapter; this one pins, on any host, what the
//! painters are handed:
//!
//! * no compound path (an outline plus a hole) is filled or clipped
//!   even-odd — reverting a site to `EvenOdd` fails here;
//! * under non-zero the hole's centre winds to 0 — switching a site to
//!   `NonZero` without reversing the hole (a winding of 2 there, the hole
//!   filled) fails here.

use peniko::kurbo::{Affine, BezPath, PathEl, Point, Shape, Stroke};
use peniko::{Brush, Fill, FontData};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};

const VW: f32 = 400.0;
const VH: f32 = 400.0;

/// The box: 100x60 border box at (150, 150), centre (200, 180).
const BOX: &str = "position: absolute; left: 150px; top: 150px; box-sizing: border-box; \
                   width: 100px; height: 60px; ";

/// Every compound path the painter is handed: (what, fill rule, path).
#[derive(Default)]
struct Compound(Vec<(&'static str, Fill, BezPath)>);

impl Compound {
    fn note(&mut self, what: &'static str, fill: Fill, shape: &PaintShape) {
        if let PaintShape::BezPath(p) = shape {
            let subpaths = p
                .elements()
                .iter()
                .filter(|e| matches!(e, PathEl::MoveTo(_)))
                .count();
            if subpaths > 1 {
                self.0.push((what, fill, p.clone()));
            }
        }
    }
}

impl Painter for Compound {
    fn reset(&mut self) {
        self.0.clear();
    }
    fn fill(&mut self, fill: Fill, _: Affine, _: &Brush, shape: &PaintShape) {
        self.note("fill", fill, shape);
    }
    fn stroke(&mut self, _: &Stroke, _: Affine, _: &Brush, _: &PaintShape) {}
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
    fn push_clip(&mut self, fill: Fill, _: Affine, shape: &PaintShape) {
        self.note("clip", fill, shape);
    }
    fn push_layer(&mut self, _: BlendMode, _: f32, _: Affine, _: &PaintShape) {}
    fn pop_layer(&mut self) {}
}

fn compound_paths(style: &str) -> Vec<(&'static str, Fill, BezPath)> {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let id: NodeId = doc.create_element("div");
    doc.set_attribute(id, "style", &format!("{BOX}{style}"));
    doc.append_child(body, id);
    doc.resolve_layout(VW, VH);
    let mut painter = Compound::default();
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

/// Each case: a style, the number of compound paths it paints, and a point
/// inside the hole every one of them cuts.
const CASES: &[(&str, usize, (f64, f64))] = &[
    (
        "border: 8px solid rgb(0, 128, 0); border-radius: 20px",
        1,
        (200.0, 180.0),
    ),
    (
        "border: 8px solid rgb(0, 128, 0); border-radius: 40px / 20px",
        1,
        (200.0, 180.0),
    ),
    (
        // An elliptical corner takes the ring; a circular one the stroke.
        "outline: 6px solid rgb(0, 128, 0); outline-offset: 4px; border-radius: 30px / 15px",
        1,
        (200.0, 180.0),
    ),
    (
        // Spread, no offset: the hole fits every layer, one ring per layer.
        "box-shadow: 0 0 0 10px rgb(255, 0, 0); border-radius: 12px",
        1,
        (200.0, 180.0),
    ),
    (
        // A hard offset: the hole escapes the layer, so the layer is filled
        // whole through one clip that is everything but the hole.
        "box-shadow: 40px 30px 0 0 rgb(255, 0, 0); border-radius: 12px",
        1,
        (200.0, 180.0),
    ),
    (
        // Unblurred inset: the padding box less the hole, moved by the
        // offset (6, 4) and shrunk by the spread.
        "box-shadow: inset 6px 4px 0 10px rgb(255, 0, 0); border-radius: 12px",
        1,
        (206.0, 184.0),
    ),
];

#[test]
fn rings_are_filled_non_zero_with_the_hole_wound_against_the_outline() {
    let mut failures = Vec::new();
    for &(style, count, hole) in CASES {
        let paths = compound_paths(style);
        // Positive control: the case reaches the ring code at all.
        if paths.len() != count {
            failures.push(format!(
                "{style}: {} compound paths, expected {count}",
                paths.len()
            ));
            continue;
        }
        for (what, fill, path) in paths {
            if !matches!(fill, Fill::NonZero) {
                failures.push(format!(
                    "{style}: a {what} with a hole is {fill:?}; Vello's MSAA even-odd \
                     fill races (#1865 D9), so it must be NonZero"
                ));
            }
            let w = path.winding(Point::new(hole.0, hole.1));
            if w != 0 {
                failures.push(format!(
                    "{style}: the {what}'s hole winds to {w} at {hole:?}; under NonZero it \
                     must be 0, the hole wound against the outline"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
