//! #976: an outer `box-shadow` is never painted inside the element's border
//! box (css-backgrounds-3 §7.1: "the box-shadow is clipped … to the outside of
//! the border box").
//!
//! `paint_box_shadow` punched the border box out of each shadow layer with an
//! even-odd fill of the layer's shape plus the box's — which subtracts only
//! where the box lies inside the layer. A shadow offset further than a layer
//! is expanded leaves part of the box outside that layer, and there the
//! even-odd fill *adds*; so such a layer was filled whole instead, and on an
//! element without an opaque background the shadow showed through it.
//! Measured in the issue (Chrome 153 vs tiny-skia, a transparent 100x100
//! box): `border-radius: 12px; box-shadow: 10px 10px 8px 6px #000` covered
//! 4143 px inside the border box where Chrome covers 25 (the rounded corners
//! only); `border-radius: 40px; box-shadow: 30px 30px 0 -10px #000` 3278
//! against 283.
//!
//! Two oracles, both **local**: everything sampled lies inside the element's
//! border-box shape (its rounded corners included), 1.5px in from its edge,
//! where the correct output is provably nothing — the element has no
//! background, no border and no content, and the surface starts transparent.
//!
//! - **tiny-skia**: the software painter's pixels.
//! - **backend-neutral**: a recording painter evaluates the fills and clips
//!   both painters are handed (Vello is handed the same commands as tiny-skia)
//!   at the same points, by winding number under each command's fill rule.
//!
//! Each case also asserts the shadow is still painted outside the box, so a
//! "fix" that drops the layers the punch cannot handle fails here too.

use peniko::kurbo::{Affine, PathEl, Point, Rect, RoundedRect, RoundedRectRadii, Shape, Stroke};
use peniko::{Brush, Fill, FontData};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 400.0;
const VH: f32 = 400.0;
const LEFT: f64 = 120.0;
const TOP: f64 = 120.0;
const BOX: f64 = 100.0;

fn document(style: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        &format!(
            "position: absolute; left: {LEFT}px; top: {TOP}px; \
             width: {BOX}px; height: {BOX}px; {style}"
        ),
    );
    doc.append_child(body, div);
    doc.resolve_layout(VW, VH);
    doc
}

fn paint_into(doc: &mut RinchDocument, painter: &mut dyn Painter) {
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(&doc.tree, painter, 1.0, (VW, VH), &mut doc.font_cx, &mut cx);
}

/// Pixel centres at least 1.5px inside the border-box shape of radius `r`
/// (uniform), in the element's untransformed space.
fn inside_points(r: f64) -> Vec<Point> {
    let m = 1.5;
    let inner = RoundedRect::from_rect(
        Rect::new(LEFT + m, TOP + m, LEFT + BOX - m, TOP + BOX - m),
        RoundedRectRadii::from_single_radius((r - m).max(0.0)),
    );
    let mut out = Vec::new();
    for y in TOP as i64..(TOP + BOX) as i64 {
        for x in LEFT as i64..(LEFT + BOX) as i64 {
            let p = Point::new(x as f64 + 0.5, y as f64 + 0.5);
            if inner.contains(p) {
                out.push(p);
            }
        }
    }
    out
}

// ── tiny-skia ──────────────────────────────────────────────────────────────

fn skia(style: &str) -> TinySkiaPainter {
    let mut doc = document(style);
    let mut painter = TinySkiaPainter::new(VW as u32, VH as u32);
    paint_into(&mut doc, &mut painter);
    painter
}

fn alpha(p: &TinySkiaPainter, x: f64, y: f64) -> u8 {
    let idx = ((y as u32 * p.width() + x as u32) * 4) as usize;
    p.pixels()[idx + 3]
}

/// The summed alpha (in 0..=255 units) of every sampled pixel inside the box.
fn skia_inside(p: &TinySkiaPainter, r: f64) -> u64 {
    inside_points(r)
        .into_iter()
        .map(|pt| alpha(p, pt.x, pt.y) as u64)
        .sum()
}

// ── Backend-neutral: the commands both painters are handed ─────────────────

/// An open clip: its fill rule, transform and shape.
type Clip = (Fill, Affine, PaintShape);

enum Cmd {
    Fill(Fill, Affine, PaintShape),
}

#[derive(Default)]
struct Recorder {
    /// Open clips; `None` is a layer that clips nothing.
    stack: Vec<Option<Clip>>,
    /// Each fill with the clips open when it was made.
    fills: Vec<(Cmd, Vec<Clip>)>,
}

fn covers(fill: Fill, transform: Affine, shape: &PaintShape, p: Point) -> bool {
    let local = transform.inverse() * p;
    let w = match shape {
        PaintShape::Rect(r) => r.winding(local),
        PaintShape::RoundedRect(r) => r.winding(local),
        PaintShape::BezPath(b) => b.winding(local),
        PaintShape::Circle(c) => c.winding(local),
        PaintShape::Line(_) => 0,
    };
    match fill {
        Fill::NonZero => w != 0,
        Fill::EvenOdd => w % 2 != 0,
    }
}

impl Recorder {
    /// Whether any recorded fill paints `p`, inside every clip open around it.
    fn paints(&self, p: Point) -> bool {
        self.fills.iter().any(|(Cmd::Fill(f, t, s), clips)| {
            covers(*f, *t, s, p) && clips.iter().all(|(cf, ct, cs)| covers(*cf, *ct, cs, p))
        })
    }
}

impl Painter for Recorder {
    fn reset(&mut self) {
        self.stack.clear();
        self.fills.clear();
    }
    fn fill(&mut self, fill: Fill, transform: Affine, _: &Brush, shape: &PaintShape) {
        let clips = self.stack.iter().flatten().cloned().collect();
        self.fills
            .push((Cmd::Fill(fill, transform, shape.clone()), clips));
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
    fn push_clip(&mut self, fill: Fill, transform: Affine, shape: &PaintShape) {
        self.stack.push(Some((fill, transform, shape.clone())));
    }
    fn push_layer(&mut self, _: BlendMode, _: f32, _: Affine, _: &PaintShape) {
        self.stack.push(None);
    }
    fn pop_layer(&mut self) {
        self.stack.pop();
    }
}

fn recorded(style: &str) -> Recorder {
    let mut doc = document(style);
    let mut r = Recorder::default();
    paint_into(&mut doc, &mut r);
    r
}

/// Both oracles: nothing inside the box, and the shadow outside it at
/// `(ox, oy)`.
fn assert_outside_only(style: &str, radius: f64, ox: f64, oy: f64) {
    let p = skia(style);
    let inside = skia_inside(&p, radius);
    assert_eq!(
        inside, 0,
        "tiny-skia: `{style}` painted {inside} (alpha units) of outer shadow inside \
         the transparent element's border box"
    );
    assert!(
        alpha(&p, ox, oy) > 0,
        "tiny-skia: `{style}` painted no shadow outside the box at ({ox}, {oy})"
    );

    let r = recorded(style);
    let hits: Vec<Point> = inside_points(radius)
        .into_iter()
        .filter(|&pt| r.paints(pt))
        .collect();
    assert!(
        hits.is_empty(),
        "painter commands: `{style}` fill {} points inside the border box, first {:?}",
        hits.len(),
        hits.first()
    );
    assert!(
        r.paints(Point::new(ox + 0.5, oy + 0.5)),
        "painter commands: `{style}` fill nothing outside the box at ({ox}, {oy})"
    );
}

/// The issue's first measurement: a blurred, spread, offset shadow under
/// rounded corners. Its inner layers are expanded by less than the 10px
/// offset, so the box escapes them on the top and left.
#[test]
fn a_blurred_offset_shadow_is_not_painted_inside_a_rounded_box() {
    assert_outside_only(
        "border-radius: 12px; box-shadow: 10px 10px 8px 6px rgb(0, 0, 0)",
        12.0,
        LEFT + BOX + 5.0,
        TOP + BOX + 5.0,
    );
}

/// The issue's second measurement: unblurred, negative spread, large radius.
#[test]
fn an_unblurred_shrunk_shadow_is_not_painted_inside_a_rounded_box() {
    assert_outside_only(
        "border-radius: 40px; box-shadow: 30px 30px 0 -10px rgb(0, 0, 0)",
        40.0,
        LEFT + BOX + 5.0,
        TOP + BOX - 30.0,
    );
}

/// The common card shadow: square box, a few pixels down, blurred. Its first
/// layers are expanded by 0.75px and pushed 4px down.
#[test]
fn a_card_shadow_is_not_painted_inside_a_square_box() {
    assert_outside_only(
        "box-shadow: 0 4px 12px rgb(0, 0, 0)",
        0.0,
        LEFT + 50.0,
        TOP + BOX + 3.0,
    );
}

/// Unblurred and unspread: the whole shadow is one layer the box escapes.
#[test]
fn a_hard_offset_shadow_is_not_painted_inside_a_square_box() {
    assert_outside_only(
        "box-shadow: 20px 20px 0 0 rgb(0, 0, 0)",
        0.0,
        LEFT + BOX + 10.0,
        TOP + BOX + 10.0,
    );
}

/// Several shadows, each offset its own way; one fits and one does not.
#[test]
fn every_shadow_of_a_list_stays_outside_the_box() {
    assert_outside_only(
        "border-radius: 8px; box-shadow: 0 0 0 4px rgb(0, 0, 255), \
         -30px 0 0 rgb(255, 0, 0), 0 25px 6px rgb(0, 0, 0)",
        8.0,
        LEFT - 20.0,
        TOP + 50.0,
    );
}

/// A shadow whose layers all fit keeps today's pixels on its ring: the
/// spread ring right outside the box is opaque.
#[test]
fn a_shadow_that_fits_still_paints_its_whole_ring() {
    let p = skia("border-radius: 8px; box-shadow: 0 0 0 4px rgb(0, 0, 0)");
    assert_eq!(alpha(&p, LEFT - 2.0, TOP + 50.0), 255);
    assert_eq!(alpha(&p, LEFT + BOX + 1.0, TOP + 50.0), 255);
    assert_eq!(skia_inside(&p, 8.0), 0);
}

/// Under a transform the cut-out follows the element: a rotated box's own
/// (rotated) border box stays empty. Sampled in a disc well inside it, since
/// `inside_points` is in untransformed space.
#[test]
fn a_rotated_box_keeps_its_shadow_outside() {
    let style = "transform: rotate(20deg); box-shadow: 25px 25px 0 rgb(0, 0, 0)";
    let p = skia(style);
    let r = recorded(style);
    let (cx, cy) = (LEFT + BOX / 2.0, TOP + BOX / 2.0);
    let mut inside = 0u64;
    for y in (cy - 40.0) as i64..(cy + 40.0) as i64 {
        for x in (cx - 40.0) as i64..(cx + 40.0) as i64 {
            let pt = Point::new(x as f64 + 0.5, y as f64 + 0.5);
            if (pt - Point::new(cx, cy)).hypot() < 45.0 {
                inside += alpha(&p, pt.x, pt.y) as u64;
                assert!(
                    !r.paints(pt),
                    "painter commands fill {pt:?} inside the rotated box"
                );
            }
        }
    }
    assert_eq!(inside, 0, "tiny-skia painted shadow inside the rotated box");
}

/// The clip cuts only the box: the shadow's own anti-aliased outer edge is
/// not cut by it too. A half-pixel offset puts the shadow's right and bottom
/// edges through the middle of a pixel column and row, which a clip whose
/// outer edge sat on the shadow's would cover at a quarter, not a half.
#[test]
fn the_clip_leaves_the_shadows_outer_edge_alone() {
    let p = skia("box-shadow: 20.5px 20.5px 0 rgb(0, 0, 0)");
    for (x, y) in [
        (LEFT + BOX + 20.0, TOP + 60.0),
        (LEFT + 60.0, TOP + BOX + 20.0),
    ] {
        let a = alpha(&p, x, y);
        assert!(
            (118..=138).contains(&a),
            "the shadow's half-covered edge pixel ({x}, {y}) has alpha {a}, not about 128"
        );
    }
    assert_eq!(alpha(&p, LEFT + BOX + 19.0, TOP + 60.0), 255);
    assert_eq!(skia_inside(&p, 0.0), 0);
}

/// The clip follows the element's transform: the whole rotated border box,
/// 1.5px in from its edge, stays empty — not just a disc both the rotated and
/// the unrotated box contain (review of #1638: a clip pushed with the identity
/// transform passed the disc and put 120742 alpha units here).
#[test]
fn the_whole_rotated_border_box_stays_empty() {
    for (deg, shadow) in [
        (45.0f64, "15px 0 0 rgb(0, 0, 0)"),
        (20.0, "25px 25px 0 rgb(0, 0, 0)"),
        (30.0, "0 8px 12px rgb(0, 0, 0)"),
    ] {
        let style = format!("transform: rotate({deg}deg); box-shadow: {shadow}");
        let p = skia(&style);
        let r = recorded(&style);
        let centre = Point::new(LEFT + BOX / 2.0, TOP + BOX / 2.0);
        let to_local = Affine::rotate(-deg.to_radians());
        let half = BOX / 2.0 - 1.5;
        let inner = Rect::new(-half, -half, half, half);
        let mut inside = 0u64;
        for y in 0..VH as i64 {
            for x in 0..VW as i64 {
                let pt = Point::new(x as f64 + 0.5, y as f64 + 0.5);
                if inner.contains(to_local * (pt - centre.to_vec2())) {
                    inside += alpha(&p, pt.x, pt.y) as u64;
                    assert!(
                        !r.paints(pt),
                        "{deg}deg `{shadow}`: painter commands fill {pt:?} inside the box"
                    );
                }
            }
        }
        assert_eq!(
            inside, 0,
            "{deg}deg `{shadow}`: tiny-skia painted inside the box"
        );
    }
}

/// The clip that keeps the shadow out of the box reaches two **device**
/// pixels past the shadow, at any scale, so it never cuts the shadow's own
/// outer edge. Under `scale(0.125)` a pad of two local pixels is a quarter of
/// a device pixel and dimmed the edge pixel (review of #1638: up to 41 alpha
/// levels). Checked on the commands: every point 1.5 device pixels outside a
/// clipped shadow fill's device bounds is inside its box clip — the clip
/// that is a compound path, a rect round the hole plus the hole (non-zero,
/// the hole wound against the rect, since #1865 D9; it was even-odd).
#[test]
fn the_box_clip_reaches_past_the_shadow_in_device_pixels() {
    for scale in [1.0f64, 0.5, 0.125, 3.0] {
        let style = format!("transform: scale({scale}); box-shadow: 30.3px 30.3px 0 rgb(0, 0, 0)");
        let r = recorded(&style);
        let mut checked = 0;
        for (Cmd::Fill(_, t, shape), clips) in &r.fills {
            let cut: Vec<_> = clips
                .iter()
                .filter(|(_, _, cs)| {
                    matches!(cs, PaintShape::BezPath(p) if p
                        .elements()
                        .iter()
                        .filter(|e| matches!(e, PathEl::MoveTo(_)))
                        .count()
                        > 1)
                })
                .collect();
            if cut.is_empty() {
                continue;
            }
            let b = t.transform_rect_bbox(shape.bounding_box());
            let (mx, my) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
            // The far sides: the shadow is pushed right and down, so its
            // near sides run through the box, where the clip is meant to cut.
            for pt in [Point::new(b.x1 + 1.5, my), Point::new(mx, b.y1 + 1.5)] {
                for (f, ct, cs) in &cut {
                    assert!(
                        covers(*f, *ct, cs, pt),
                        "scale({scale}): the box clip stops short of {pt:?}, \
                         1.5 device px outside the shadow"
                    );
                }
            }
            checked += 1;
        }
        assert!(
            checked > 0,
            "scale({scale}): no shadow was painted through the box clip"
        );
    }
}
