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
