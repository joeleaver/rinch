//! A `text-shadow`'s blur radius softens it (#980), and the shadow is cast by
//! the text's decorations as well as its glyphs (#981).
//!
//! Both painters used to draw every shadow as a hard-edged copy of the glyphs
//! alone: the blur radius sized the layer bounds and nothing else, and an
//! `underline`, a `line-through` or a wavy underline cast no shadow at all.
//!
//! # The blur, and why both painters agree on it
//!
//! Vello has no general blur, so the shadow is blurred the one way both
//! painters can draw identically: as a weighted sum of copies of the shadow at
//! the taps of a Gaussian kernel, each tap drawn into its own layer composited
//! with [`BlendMode::Plus`] at the tap's weight, inside one group layer at the
//! shadow colour's alpha. The sum of the weights is 1, so a shadow's solid
//! interior stays solid; the kernel's reach is the blur radius, which is what
//! `layer_bounds::text_shadow_reach` and the damage ink already allow.
//!
//! # Two oracles
//!
//! - **What the painter is handed** (`Recorder`): the backend-neutral half.
//!   Pinning the calls pins both backends, since both are handed the same.
//! - **What the software painter draws** (`TinySkiaPainter`): a local pixel
//!   oracle that tells red shadow ink from black text ink by colour.
//!
//! Every assertion runs off `scale == 1`, where an unscaled length and a scaled
//! one agree. The mutant each fixture kills is named on it.

#![cfg(feature = "software-renderer")]

use peniko::kurbo::{Affine, Point, Rect, Shape, Stroke};
use peniko::{Brush, Fill, FontData};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::{BlendMode, PaintGlyph, PaintImage, PaintShape, Painter};
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const VW: f32 = 300.0;
const VH: f32 = 200.0;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// Declared font size and line height, so the line box is a statement.
const BASE: &str = "width: 260px; margin: 20px 0 0 30px; font-size: 32px; line-height: 40px; \
                    color: rgb(0, 0, 0); font-family: ProbeFace";

fn document(style: &str, html_children: &[(&str, Option<&str>)]) -> RinchDocument {
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
    for (text, span_style) in html_children {
        let t = doc.create_text(text);
        match span_style {
            None => doc.append_child(c, t),
            Some(s) => {
                let span = doc.create_element("span");
                doc.set_attribute(span, "style", s);
                doc.append_child(c, span);
                doc.append_child(span, t);
            }
        }
    }
    doc.resolve_layout(VW, VH);
    doc
}

fn rgb(brush: &Brush) -> [u8; 4] {
    let Brush::Solid(c) = brush else {
        panic!("text and decorations are painted with a solid brush");
    };
    c.to_rgba8().to_u8_array()
}

fn is_red(c: [u8; 4]) -> bool {
    c[0] == 255 && c[1] == 0 && c[2] == 0
}

// ── The backend-neutral oracle ──────────────────────────────────────────────

#[derive(Debug)]
enum Op {
    Glyphs { color: [u8; 4], glyphs: Vec<Point> },
    Stroke { color: [u8; 4], bbox: Rect },
    Push { blend: BlendMode, opacity: f32 },
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
            PaintShape::Line(l) => {
                let (a, b) = (transform * l.p0, transform * l.p1);
                Rect::from_points(a, b)
            }
            PaintShape::BezPath(p) => (transform * p.clone()).bounding_box(),
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
        self.ops.push(Op::Glyphs {
            color: rgb(brush),
            glyphs: glyphs
                .iter()
                .map(|g| transform * Point::new(g.x as f64, g.y as f64))
                .collect(),
        });
    }
    fn draw_image(&mut self, _: &PaintImage<'_>, _: Affine) {}
    fn push_clip(&mut self, _: Fill, _: Affine, _: &PaintShape) {}
    fn push_layer(&mut self, blend: BlendMode, opacity: f32, _: Affine, _: &PaintShape) {
        self.ops.push(Op::Push { blend, opacity });
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

/// The first glyph of every red run (one per tap), and the first glyph of the
/// one black run.
fn first_glyphs(ops: &[Op]) -> (Vec<Point>, Point) {
    let mut red = Vec::new();
    let mut black = Vec::new();
    for op in ops {
        if let Op::Glyphs { color, glyphs } = op {
            if is_red(*color) {
                red.push(glyphs[0]);
            } else {
                black.push(glyphs[0]);
            }
        }
    }
    assert_eq!(black.len(), 1, "one line, one face: one main run");
    (red, black[0])
}

/// A blurred shadow is handed to the painter as a Gaussian kernel of copies:
/// each in a `Plus` layer at its weight, the weights summing to 1, the copies
/// spread symmetrically out to the blur radius — in physical px.
///
/// Kills: the blur ignored (one hard copy, no `Plus` layer); the blur radius
/// unscaled (the spread at scale 2 reaches 8, not 16); a kernel whose weights
/// do not sum to 1 (a solid shadow would come out faint or clipped); a
/// one-sided kernel (the weighted mean of the taps moves off the offset).
#[test]
fn a_blurred_shadow_is_a_normalised_kernel_out_to_the_scaled_radius() {
    const BLUR: f64 = 8.0;
    for scale in [1.5, 2.0] {
        let mut doc = document(
            &format!("{BASE}; text-shadow: 3px 50px {BLUR}px rgb(255, 0, 0)"),
            &[("HxH", None)],
        );
        let ops = record(&mut doc, scale);

        let weights: Vec<f32> = ops
            .iter()
            .filter_map(|op| match op {
                Op::Push {
                    blend: BlendMode::Plus,
                    opacity,
                } => Some(*opacity),
                _ => None,
            })
            .collect();
        assert!(
            weights.len() >= 9,
            "at scale {scale} a {BLUR}px blur must be drawn as a kernel of `Plus` layers; \
             got {} of them (#980)",
            weights.len()
        );
        let sum: f32 = weights.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-4,
            "the kernel's weights sum to {sum}, not 1: a solid shadow would not stay solid"
        );

        let (taps, main) = first_glyphs(&ops);
        assert_eq!(taps.len(), weights.len(), "one shadow copy per tap");
        let rel: Vec<(f64, f64)> = taps
            .iter()
            .map(|p| (p.x - main.x - 3.0 * scale, p.y - main.y - 50.0 * scale))
            .collect();
        let reach = rel
            .iter()
            .map(|(dx, dy)| dx.hypot(*dy))
            .fold(0.0_f64, f64::max);
        let radius = BLUR * scale;
        assert!(
            reach <= radius + 1e-6 && reach >= 0.75 * radius,
            "at scale {scale} the kernel reaches {reach:.2} physical px from the offset; \
             a {BLUR}px blur reaches {radius} (#980)"
        );
        let (mx, my) = rel
            .iter()
            .zip(&weights)
            .fold((0.0, 0.0), |(ax, ay), ((dx, dy), w)| {
                (ax + dx * *w as f64, ay + dy * *w as f64)
            });
        assert!(
            mx.abs() < 0.05 && my.abs() < 0.05,
            "the kernel's weighted centre is ({mx:.3}, {my:.3}) from the offset: it must be \
             symmetric"
        );
    }
}

/// Control: an unblurred shadow is still one direct copy — no layer at all.
#[test]
fn control_an_unblurred_shadow_is_one_copy_and_no_layer() {
    let mut doc = document(
        &format!("{BASE}; text-shadow: 3px 50px 0 rgb(255, 0, 0)"),
        &[("HxH", None)],
    );
    let ops = record(&mut doc, 2.0);
    assert!(
        !ops.iter().any(|op| matches!(op, Op::Push { .. })),
        "no layer for a hard shadow: {ops:?}"
    );
    let (taps, _) = first_glyphs(&ops);
    assert_eq!(taps.len(), 1);
}

/// The strokes of `ops` by colour: red ones (shadow) and the rest.
fn strokes(ops: &[Op]) -> (Vec<Rect>, Vec<Rect>) {
    let mut red = Vec::new();
    let mut other = Vec::new();
    for op in ops {
        if let Op::Stroke { color, bbox } = op {
            if is_red(*color) {
                red.push(*bbox);
            } else {
                other.push(*bbox);
            }
        }
    }
    (red, other)
}

fn assert_translated(shadow: &[Rect], main: &[Rect], dx: f64, dy: f64, what: &str) {
    assert!(!main.is_empty(), "positive control: the {what} is drawn");
    assert_eq!(
        shadow.len(),
        main.len(),
        "every {what} stroke casts a shadow stroke (#981): shadow={shadow:?} main={main:?}"
    );
    for (s, m) in shadow.iter().zip(main) {
        let moved = Rect::new(m.x0 + dx, m.y0 + dy, m.x1 + dx, m.y1 + dy);
        assert!(
            (s.x0 - moved.x0).abs() < 1e-3
                && (s.y0 - moved.y0).abs() < 1e-3
                && (s.x1 - moved.x1).abs() < 1e-3
                && (s.y1 - moved.y1).abs() < 1e-3,
            "the {what}'s shadow stroke is {s:?}; the {what} {m:?} moved by ({dx}, {dy}) \
             is {moved:?} (#981)"
        );
    }
}

/// An underline and a line-through cast the text's shadow, stroke for stroke,
/// at the shadow's offset in physical px — and a `visibility: hidden` stretch,
/// which draws no underline, casts none (#829).
///
/// Kills: the shadow pass drawing no decorations (the #981 bug); its offset
/// unscaled; its segments taken unmasked (one long shadow stroke under two
/// short main ones).
#[test]
fn underline_and_line_through_cast_the_shadow() {
    let scale = 1.5;
    for deco in ["underline", "line-through"] {
        let mut doc = document(
            &format!(
                "{BASE}; text-decoration: {deco} rgb(0, 0, 255); \
                 text-shadow: 3px 5px 0 rgb(255, 0, 0)"
            ),
            &[
                ("Hx", None),
                ("HHH", Some("visibility: hidden")),
                ("xH", None),
            ],
        );
        let ops = record(&mut doc, scale);
        let (red, main) = strokes(&ops);
        assert_eq!(
            main.len(),
            2,
            "positive control: the hidden stretch splits the {deco} in two: {main:?}"
        );
        assert_translated(&red, &main, 3.0 * scale, 5.0 * scale, deco);
    }
}

/// A wavy underline casts the shadow too.
///
/// Kills: the wavy decoration left out of the shadow pass.
#[test]
fn a_wavy_underline_casts_the_shadow() {
    let scale = 1.5;
    let mut doc = document(
        &format!("{BASE}; text-shadow: 3px 5px 0 rgb(255, 0, 0)"),
        &[
            ("Hx ", None),
            ("wavy", Some("text-decoration: underline wavy rgb(0, 0, 255)")),
        ],
    );
    let ops = record(&mut doc, scale);
    let (red, main) = strokes(&ops);
    assert_translated(&red, &main, 3.0 * scale, 5.0 * scale, "wavy underline");
}

// ── The pixel oracle ────────────────────────────────────────────────────────

fn paint(doc: &mut RinchDocument, scale: f64) -> (Vec<[u8; 4]>, u32, u32) {
    let (w, h) = (
        (VW as f64 * scale).ceil() as u32,
        (VH as f64 * scale).ceil() as u32,
    );
    let mut painter = TinySkiaPainter::new(w, h);
    painter.fill_white();
    let mut cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        scale,
        (VW, VH),
        &mut doc.font_cx,
        &mut cx,
    );
    (painter.pixels().as_chunks::<4>().0.to_vec(), w, h)
}

/// The bounding box of every pixel with any red tint at all, and the reddest
/// tint anywhere (`255 - green`, over white).
fn red_tint(px: &[[u8; 4]], w: u32, h: u32) -> (Rect, u8) {
    let mut bbox: Option<Rect> = None;
    let mut peak = 0u8;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let [r, g, b, _] = px[y * w as usize + x];
            // Red over white keeps r at 255 and lowers g and b together;
            // black text over white lowers all three.
            if r >= 254 && g <= 248 && g.abs_diff(b) <= 2 {
                let cell = Rect::new(x as f64, y as f64, x as f64 + 1.0, y as f64 + 1.0);
                bbox = Some(bbox.map_or(cell, |r| r.union(cell)));
                peak = peak.max(255 - g);
            }
        }
    }
    (bbox.expect("no shadow ink; the fixture measures nothing"), peak)
}

/// The software painter draws a blurred shadow soft: its faint edge reaches
/// most of the way out to the blur radius past the hard shadow's, and no
/// further; and its strongest pixel is paler than the hard shadow's.
///
/// Kills: the blur ignored (the extents coincide and the peak is solid); the
/// blur radius unscaled (at scale 2 the edge reaches about half as far); the
/// `Plus` layer composited source-over in the software painter (the copies
/// saturate: the peak comes back near solid).
#[test]
fn the_software_painter_draws_the_blur() {
    const BLUR: f64 = 8.0;
    for scale in [1.0, 2.0] {
        let hard = format!("{BASE}; text-shadow: 0 50px 0 rgb(255, 0, 0)");
        let soft = format!("{BASE}; text-shadow: 0 50px {BLUR}px rgb(255, 0, 0)");
        let (hp, w, h) = paint(&mut document(&hard, &[("HxH", None)]), scale);
        let (sp, _, _) = paint(&mut document(&soft, &[("HxH", None)]), scale);
        let (hb, hpeak) = red_tint(&hp, w, h);
        let (sb, speak) = red_tint(&sp, w, h);
        let radius = BLUR * scale;
        for (side, grew) in [
            ("left", hb.x0 - sb.x0),
            ("top", hb.y0 - sb.y0),
            ("right", sb.x1 - hb.x1),
            ("bottom", sb.y1 - hb.y1),
        ] {
            assert!(
                grew >= 0.6 * radius && grew <= radius + 2.0,
                "at scale {scale} the blurred shadow's {side} edge is {grew} physical px past \
                 the hard one's; a {BLUR}px blur softens it out to about {radius} (#980). \
                 hard={hb:?} soft={sb:?}"
            );
        }
        assert!(
            hpeak > 200,
            "positive control: the hard shadow is solid red somewhere ({hpeak})"
        );
        assert!(
            speak < 160,
            "at scale {scale} the blurred shadow peaks at {speak}: a blur spreads the ink, \
             so no pixel of a thin-stroked shadow stays near solid (#980)"
        );
    }
}

/// The underline's shadow is drawn: the row the underline is on, moved by the
/// shadow's offset, is red under the whole underline — at scale 1.5.
///
/// Kills: the shadow pass drawing no decorations (#981).
#[test]
fn the_software_painter_draws_the_underline_shadow() {
    let scale = 1.5;
    let style = format!("{BASE}; text-decoration: underline; text-shadow: 0 30px 0 rgb(255, 0, 0)");
    let (px, w, _) = paint(&mut document(&style, &[("HxH", None)]), scale);
    let at = |x: u32, y: u32| px[(y * w + x) as usize];
    // The underline is the lowest dark row ("HxH" has no descender).
    let mut row = None;
    let mut span = (u32::MAX, 0);
    for y in 0..(VH as f64 * scale) as u32 {
        for x in 0..w {
            let [r, g, b, _] = at(x, y);
            if r < 100 && g < 100 && b < 100 {
                row = Some(y);
            }
        }
    }
    let row = row.expect("no text ink; the fixture measures nothing");
    for x in 0..w {
        let [r, g, b, _] = at(x, row);
        if r < 100 && g < 100 && b < 100 {
            span = (span.0.min(x), span.1.max(x));
        }
    }
    assert!(
        span.1 > span.0 + 40,
        "positive control: the underline row spans the text ({span:?})"
    );
    let shadow_row = row + (30.0 * scale) as u32;
    let red = (span.0..=span.1)
        .filter(|&x| {
            let [r, g, _, _] = at(x, shadow_row);
            r > 200 && g < 128
        })
        .count();
    let wide = (span.1 - span.0 + 1) as usize;
    assert!(
        red * 10 >= wide * 8,
        "only {red} of the {wide} pixels under the underline's shadow row {shadow_row} are \
         red: the underline casts no shadow (#981)"
    );
}
