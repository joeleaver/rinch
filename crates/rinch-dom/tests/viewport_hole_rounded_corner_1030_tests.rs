//! #1030: a `data-viewport` hole at a rounded box's corner leaves nothing
//! painted outside the box's rounded shape.
//!
//! The hole was the box's shape with each viewport rect cut out under
//! `Fill::EvenOdd`, and the rect used to go in whole, so where it reached past
//! the shape — the sliver between a rounded corner's arc and the rect's
//! corner, or past the box altogether — the even-odd count was one and the
//! background (or the shadow) was painted **outside** the box. Since #1866 the
//! background is the plain shape and the viewport clears its own box, through
//! the box's rounded clip, when paint reaches it; these fixtures keep the
//! outcome pinned.
//!
//! Each fixture reads a pixel outside the shape inside the hole (want: nothing
//! painted), a pixel inside the shape inside the hole (want: nothing — the
//! hole still cuts), and a pixel inside the shape outside the hole (want:
//! black — the box paints at all).

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
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

/// Every pixel of a hole at a rounded corner is clear, except the arc's own
/// anti-aliasing. Since #1866 the background is painted whole and the clear
/// erases it through the box's rounded clip, so an arc pixel the shape covers
/// by `c` keeps `c` of background less `c` of it erased: `c (1 - c)`, at most
/// a quarter, and only on the arc (Vello conflates the two edges the same
/// way). Nothing off the arc is left, and no pixel more than a quarter.
#[test]
fn a_corner_hole_leaves_only_the_arcs_anti_aliasing() {
    let p = painted(&boxed("40px", "background: rgb(0,0,0)", ""));
    let mut bad = Vec::new();
    for y in 150..250 {
        for x in 150..250 {
            let a = alpha(&p, x, y);
            // The corner's circle: centre (190, 190), radius 40.
            let d = ((x as f64 + 0.5 - 190.0).powi(2) + (y as f64 + 0.5 - 190.0).powi(2)).sqrt();
            let on_arc = x < 190 && y < 190 && (d - 40.0).abs() < 1.0;
            if a != 0 && (!on_arc || a > 64) {
                bad.push((x, y, a));
            }
        }
    }
    assert!(bad.is_empty(), "painted inside the hole: {bad:?}");
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
