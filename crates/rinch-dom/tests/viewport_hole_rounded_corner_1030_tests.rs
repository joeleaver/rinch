//! #1030: a `data-viewport` hole cuts only the part of itself that lies
//! inside the box's rounded shape.
//!
//! The background (and an inset shadow's clip) is the box's shape with each
//! hole cut out under `Fill::EvenOdd`. The hole rect used to go in whole, so
//! where it reached past the shape — the sliver between a rounded corner's arc
//! and the rect's corner, or past the box altogether — the even-odd count was
//! one and the background (or the shadow) was painted **outside** the box.
//!
//! Each fixture reads a pixel outside the shape inside the hole (want: nothing
//! painted), a pixel inside the shape inside the hole (want: nothing — the
//! hole still cuts), and a pixel inside the shape outside the hole (want:
//! black — the box paints at all).

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use peniko::kurbo::{PathEl, Point, Rect};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::radii::BorderRadii;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const W: u32 = 500;
const H: u32 = 450;

fn painted(html: &str) -> TinySkiaPainter {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(W as f32, H as f32);
    let mut painter = TinySkiaPainter::new(W, H);
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut lcx,
    );
    painter
}

fn alpha(p: &TinySkiaPainter, x: u32, y: u32) -> u8 {
    p.pixels()[((y * W + x) * 4 + 3) as usize]
}

const STYLES: [(&str, &str); 3] = [
    ("background", "background: rgb(0,0,0)"),
    (
        "unblurred inset shadow",
        "box-shadow: inset 0 0 0 30px rgb(0,0,0)",
    ),
    (
        "blurred inset shadow",
        "box-shadow: inset 0 0 20px 10px rgb(0,0,0)",
    ),
];

fn boxed(radius: &str, style: &str, hole_style: &str) -> String {
    format!(
        "<div style=\"position: absolute; left: 150px; top: 150px; width: 200px; \
         height: 200px; overflow: hidden; border-radius: {radius}; {style}\">\
         <div data-viewport=\"g\" style=\"width: 100px; height: 100px; {hole_style}\"></div></div>"
    )
}

/// The issue's case: a 100x100 hole at the top-left corner of a box rounded
/// by a 40px circle. (152, 152) is 53.7px from the corner's centre (190, 190).
#[test]
fn a_hole_at_a_circular_corner_paints_nothing_outside_the_arc() {
    for (label, style) in STYLES {
        let p = painted(&boxed("40px", style, ""));
        for (x, y) in [(152, 152), (151, 160), (160, 151)] {
            assert_eq!(
                alpha(&p, x, y),
                0,
                "{label}: ({x}, {y}) is outside the rounded corner and was painted"
            );
        }
        // Inside the arc, inside the hole: still cut.
        assert_eq!(
            alpha(&p, 170, 170),
            0,
            "{label}: the hole no longer cuts (170, 170)"
        );
        assert_eq!(
            alpha(&p, 200, 200),
            0,
            "{label}: the hole no longer cuts (200, 200)"
        );
        // Inside the shape, outside the hole: painted.
        assert!(
            alpha(&p, 155, 300) > 150,
            "{label}: the ring below the hole is not painted ({})",
            alpha(&p, 155, 300)
        );
    }
}

/// An elliptical corner (#977's `BorderRadii`): 80px across, 30px down,
/// centred at (230, 180). (175, 153) is outside that ellipse and inside the
/// 30px circle a smaller-radius stand-in (`circular_within`) would draw.
#[test]
fn a_hole_at_an_elliptical_corner_paints_nothing_outside_the_ellipse() {
    for (label, style) in STYLES {
        let p = painted(&boxed("80px / 30px", style, "width: 120px; height: 60px"));
        for (x, y) in [(160, 155), (152, 165), (190, 151), (175, 153)] {
            assert_eq!(
                alpha(&p, x, y),
                0,
                "{label}: ({x}, {y}) is outside the elliptical corner and was painted"
            );
        }
        // Inside the ellipse, inside the hole: cut.
        assert_eq!(
            alpha(&p, 240, 165),
            0,
            "{label}: the hole no longer cuts (240, 165)"
        );
        // Right of the hole, inside the top-right corner's ellipse: painted.
        if label == "background" {
            assert!(
                alpha(&p, 280, 165) > 200,
                "{label}: (280, 165) is not painted"
            );
        }
    }
}

/// A hole that reaches past a square box (a viewport moved out of its
/// clipper by a negative margin) painted the background beside the box.
#[test]
fn a_hole_past_the_box_edge_paints_nothing_outside_the_box() {
    for (label, style) in STYLES {
        let p = painted(&boxed("0", style, "margin-left: -50px; margin-top: 60px"));
        // Left of the box, level with the hole.
        assert_eq!(
            alpha(&p, 120, 230),
            0,
            "{label}: (120, 230) is left of the box and was painted"
        );
        // In the box, in the hole.
        assert_eq!(
            alpha(&p, 175, 230),
            0,
            "{label}: the hole no longer cuts (175, 230)"
        );
        // In the box, below the hole.
        assert!(
            alpha(&p, 155, 340) > 200,
            "{label}: the box is not painted below the hole"
        );
    }
}

/// Every pixel of a hole at a rounded corner is clear — the ones on the arc
/// too, where the shape's edge and the hole's cut meet. A hole that cut a
/// slightly different curve from the one the background fills leaves a
/// faint line of background along the arc there.
#[test]
fn no_pixel_of_a_corner_hole_is_painted() {
    let p = painted(&boxed("40px", "background: rgb(0,0,0)", ""));
    let mut painted_px = Vec::new();
    for y in 150..250 {
        for x in 150..250 {
            if alpha(&p, x, y) != 0 {
                painted_px.push((x, y, alpha(&p, x, y)));
            }
        }
    }
    assert!(
        painted_px.is_empty(),
        "painted inside the hole: {painted_px:?}"
    );
}

/// A hole narrower than the box's straight top side ends partway along it,
/// off its midpoint (the hole's right edge at x = 230 crosses the top side,
/// which runs 190..310, a third of the way along). The background right of
/// the hole along that side is still painted.
#[test]
fn a_corner_hole_ends_where_its_rect_ends() {
    let p = painted(&boxed(
        "40px",
        "background: rgb(0,0,0)",
        "width: 80px; height: 80px",
    ));
    for (x, y) in [(235, 151), (260, 152), (300, 151), (151, 235), (152, 260)] {
        assert!(
            alpha(&p, x, y) > 200,
            "({x}, {y}) is outside the hole and not painted ({})",
            alpha(&p, x, y)
        );
    }
    assert_eq!(
        alpha(&p, 152, 152),
        0,
        "(152, 152) is outside the corner and painted"
    );
    assert_eq!(alpha(&p, 225, 225), 0, "the hole no longer cuts (225, 225)");
}

/// A hole at each corner in turn (100x100, moved there by its margins): the
/// pixel just outside that corner's arc is clear, the pixel inside the
/// shape beside the hole is painted.
#[test]
fn a_hole_at_each_corner_paints_nothing_outside_its_arc() {
    // (margin-left, margin-top, outside the arc, inside the shape beside the hole)
    for (ml, mt, outside, beside) in [
        (0, 0, (152, 152), (155, 300)),
        (100, 0, (347, 152), (345, 300)),
        (100, 100, (347, 347), (345, 200)),
        (0, 100, (152, 347), (155, 200)),
    ] {
        for (label, style) in STYLES {
            let p = painted(&boxed(
                "40px",
                style,
                &format!("margin-left: {ml}px; margin-top: {mt}px"),
            ));
            assert_eq!(
                alpha(&p, outside.0, outside.1),
                0,
                "{label}, hole at ({ml}, {mt}): {outside:?} is outside the arc and painted"
            );
            assert!(
                alpha(&p, beside.0, beside.1) > 150,
                "{label}, hole at ({ml}, {mt}): {beside:?} is not painted"
            );
        }
    }
}

/// A corner no hole meets paints exactly as it does with no hole: only the
/// corner a hole meets is flattened, so a viewport entering one corner does
/// not change the anti-aliasing of the other three.
#[test]
fn corners_no_hole_meets_paint_as_without_a_hole() {
    let with = painted(&boxed("40px", "background: rgb(0,0,0)", ""));
    let without = painted(
        "<div style=\"position: absolute; left: 150px; top: 150px; width: 200px; \
         height: 200px; overflow: hidden; border-radius: 40px; background: rgb(0,0,0)\"></div>",
    );
    let mut differ = Vec::new();
    for y in 140..360 {
        for x in 140..360 {
            let in_hole = (150..251).contains(&x) && (150..251).contains(&y);
            if !in_hole && alpha(&with, x, y) != alpha(&without, x, y) {
                differ.push((x, y, alpha(&without, x, y), alpha(&with, x, y)));
            }
        }
    }
    assert!(differ.is_empty(), "differ outside the hole: {differ:?}");
}

fn contours(path: &peniko::kurbo::BezPath) -> Vec<Vec<Point>> {
    let mut out: Vec<Vec<Point>> = Vec::new();
    peniko::kurbo::flatten(path.iter(), 0.01, |el| match el {
        PathEl::MoveTo(p) => out.push(vec![p]),
        PathEl::LineTo(p) => out.last_mut().unwrap().push(p),
        _ => {}
    });
    out
}

/// `poly` clipped to `clip` (an independent Sutherland–Hodgman).
fn clip_to(poly: &[Point], clip: Rect) -> Vec<Point> {
    let mut out = poly.to_vec();
    type Edge = (fn(Point, f64) -> f64, f64);
    let edges: [Edge; 4] = [
        (|p, a| p.x - a, clip.x0),
        (|p, a| a - p.x, clip.x1),
        (|p, a| p.y - a, clip.y0),
        (|p, a| a - p.y, clip.y1),
    ];
    for (d, a) in edges {
        let input = std::mem::take(&mut out);
        let Some(&last) = input.last() else { break };
        let mut prev = last;
        for &cur in &input {
            let (dp, dc) = (d(prev, a), d(cur, a));
            if dc >= 0.0 {
                if dp < 0.0 {
                    out.push(prev.lerp(cur, dp / (dp - dc)));
                }
                out.push(cur);
            } else if dp >= 0.0 {
                out.push(prev.lerp(cur, dp / (dp - dc)));
            }
            prev = cur;
        }
    }
    out
}

fn signed_area(poly: &[Point]) -> f64 {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum::<f64>()
        * 0.5
}

/// Review of #1658, F1: Vello's analytic area anti-aliasing (what Android
/// renders with) sums each pixel's signed coverage over every contour and
/// folds it even-odd as `|a - 2 round(a / 2)|` (`fine.wgsl`'s `fill_path`).
/// A hole contour wound the same way as the outline adds its share of a
/// shared edge pixel to the outline's instead of cancelling it, and drew a
/// near-opaque line along the arc. Emulated here exactly, per pixel, over
/// the hole, on the path itself: no GPU needed.
#[test]
fn the_hole_cancels_the_outline_under_area_anti_aliasing() {
    for (rect, r) in [
        (Rect::new(150.0, 150.0, 350.0, 350.0), 40.0),
        (Rect::new(150.3, 150.3, 350.3, 350.3), 40.0),
        (Rect::new(300.0, 300.0, 900.0, 900.0), 120.0),
    ] {
        let hole = Rect::new(
            rect.x0,
            rect.y0,
            rect.x0 + rect.width() / 2.0,
            rect.y0 + rect.height() / 2.0,
        );
        let cs = contours(&BorderRadii::uniform(r).path_with_holes(rect, &[hole]));
        assert_eq!(cs.len(), 2, "one outline and one hole");
        let (mut max, mut n) = (0.0f64, 0);
        for y in hole.y0 as i32..hole.y1 as i32 {
            for x in hole.x0 as i32..hole.x1 as i32 {
                let px = Rect::new(x as f64, y as f64, x as f64 + 1.0, y as f64 + 1.0);
                let a: f64 = cs.iter().map(|c| signed_area(&clip_to(c, px))).sum();
                let cov = (a - 2.0 * (0.5 * a).round_ties_even()).abs();
                if cov > 1.0 / 255.0 {
                    n += 1;
                }
                max = max.max(cov);
            }
        }
        assert!(
            n == 0,
            "{rect:?} r {r}: {n} px in the hole covered, max coverage {max:.3}"
        );
    }
}
