//! #974: an `inset` `box-shadow` is painted, inside the padding box.
//!
//! css-backgrounds-3 §7.1: an inner shadow is drawn inside the **padding**
//! box, as if everything outside the padding edge were opaque; its hole is
//! the padding box moved by the offset and shrunk by the spread, with the
//! padding box's radii shrunk by the spread; it paints above the background
//! and below the border; the first shadow in the list is on top; and a blur
//! is a Gaussian with a standard deviation of half the blur radius.
//!
//! Every expected value here is also what Chrome 153 paints, measured on a
//! headless screenshot of the same boxes over a white page (the probe page is
//! quoted in the PR). rinch used to skip every inset shadow outright, on both
//! painters, so each fixture here saw nothing at all before the fix.
//!
//! The fixtures stay off the fixed points: the offset is `10px 20px`, not a
//! symmetric one, so an x/y swap shows; the border is 5px and the spread 10px,
//! so the padding box and the border box disagree; and the blurred profile is
//! sampled at depths where a Gaussian and a linear ramp differ.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const BOX: i64 = 100;
const LEFT: i64 = 150;
const TOP: i64 = 150;

fn painted(style: &str) -> TinySkiaPainter {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        &format!(
            "position: absolute; left: {LEFT}px; top: {TOP}px; width: {BOX}px; height: {BOX}px; {style}"
        ),
    );
    doc.append_child(body, div);
    doc.resolve_layout(800.0, 600.0);
    let mut painter = TinySkiaPainter::new(800, 600);
    let mut paint_layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (800.0, 600.0),
        &mut doc.font_cx,
        &mut paint_layout_cx,
    );
    painter
}

/// `[r, g, b, a]` of one pixel, each 0..=1 (premultiplied, as tiny-skia
/// stores it — exact for the opaque pixels these fixtures read colour from).
fn rgba(p: &TinySkiaPainter, x: i64, y: i64) -> [f64; 4] {
    let idx = ((y as u32 * p.width() + x as u32) * 4) as usize;
    let px = &p.pixels()[idx..idx + 4];
    [
        px[0] as f64 / 255.0,
        px[1] as f64 / 255.0,
        px[2] as f64 / 255.0,
        px[3] as f64 / 255.0,
    ]
}

fn alpha(p: &TinySkiaPainter, x: i64, y: i64) -> f64 {
    rgba(p, x, y)[3]
}

fn assert_alpha(p: &TinySkiaPainter, x: i64, y: i64, want: f64, what: &str) {
    let got = alpha(p, x, y);
    assert!(
        (got - want).abs() <= 0.02,
        "{what}: alpha at ({x}, {y}) is {got:.3}, want {want:.3}"
    );
}

/// `inset 10px 20px`: the hole is the box moved right 10 and down 20, so the
/// shadow is a 10px strip down the left edge and a 20px strip across the top.
/// Chrome 153: exactly those strips, solid, and nothing else.
#[test]
fn an_offset_inset_shadow_paints_the_uncovered_strips() {
    let p = painted("box-shadow: inset 10px 20px 0 0 rgb(0,0,0)");
    let mid = TOP + 70;
    // The left strip, 10px wide.
    assert_alpha(&p, LEFT, mid, 1.0, "left strip, first column");
    assert_alpha(&p, LEFT + 9, mid, 1.0, "left strip, last column");
    assert_alpha(&p, LEFT + 10, mid, 0.0, "just right of the left strip");
    // The top strip, 20px tall.
    assert_alpha(&p, LEFT + 70, TOP, 1.0, "top strip, first row");
    assert_alpha(&p, LEFT + 70, TOP + 19, 1.0, "top strip, last row");
    assert_alpha(&p, LEFT + 70, TOP + 20, 0.0, "just below the top strip");
    // Nothing on the far sides, and nothing outside the box.
    assert_alpha(&p, LEFT + BOX - 1, mid, 0.0, "right edge");
    assert_alpha(&p, LEFT + 70, TOP + BOX - 1, 0.0, "bottom edge");
    assert_alpha(&p, LEFT - 1, mid, 0.0, "outside the box");
    assert_alpha(&p, LEFT + 70, TOP - 1, 0.0, "outside the box");
}

/// The shadow is drawn inside the **padding** box, and its hole takes the
/// padding box's radii shrunk by the spread. Border 5px (transparent),
/// radius 30px, so the padding box is inset 5px with radius 25; spread 10px
/// makes the hole inset 15px with radius 15. Chrome 153, row through the
/// middle: transparent for 5px, black for 10px, clear after. Along the
/// diagonal from the corner: clear to 11px in, black 13..=18, clear from 20.
#[test]
fn an_inset_shadow_fills_the_padding_box_and_its_hole_is_rounded_by_the_spread() {
    let p = painted(
        "border: 5px solid transparent; border-radius: 30px; \
         box-shadow: inset 0 0 0 10px rgb(0,0,0)",
    );
    let mid = TOP + 55;
    for i in 0..5 {
        assert_alpha(&p, LEFT + i, mid, 0.0, "border area");
    }
    for i in 5..15 {
        assert_alpha(&p, LEFT + i, mid, 1.0, "the 10px ring");
    }
    assert_alpha(&p, LEFT + 15, mid, 0.0, "inside the hole");
    // Outside the padding box's rounded corner (radius 25 about (30, 30)).
    assert_alpha(&p, LEFT + 8, TOP + 8, 0.0, "outside the padding corner");
    for i in 13..=18 {
        assert_alpha(&p, LEFT + i, TOP + i, 1.0, "the ring across the corner");
    }
    // Inside the hole's corner (radius 15 about (30, 30)). A hole that kept
    // the padding radius 25 would put its arc's centre at (40, 40) and leave
    // this pixel in the shadow.
    assert_alpha(&p, LEFT + 22, TOP + 22, 0.0, "inside the hole's corner");
}

/// A blurred inset shadow is a Gaussian of `sigma = blur / 2` about the
/// hole's edge. `inset 0 0 20px`, row through the middle, alpha at depth `d`
/// from the left edge (pixel centre `d + 0.5`). Chrome 153 measures
/// 0.467 / 0.290 / 0.141 / 0.055 / 0.012; the Gaussian gives
/// 0.480 / 0.291 / 0.147 / 0.061 / 0.020.
#[test]
fn a_blurred_inset_shadow_falls_off_as_a_gaussian() {
    let p = painted("box-shadow: inset 0 0 20px rgb(0,0,0)");
    let mid = TOP + 50;
    for (d, chrome) in [
        (0, 0.467),
        (5, 0.290),
        (10, 0.141),
        (15, 0.055),
        (20, 0.012),
    ] {
        let got = alpha(&p, LEFT + d, mid);
        assert!(
            (got - chrome).abs() <= 0.04,
            "blurred inset shadow at depth {d}: alpha {got:.3}, Chrome 153 paints {chrome:.3}"
        );
    }
    assert!(
        alpha(&p, LEFT + 50, mid) <= 0.01,
        "the middle of the box is 2.5 sigma from every edge"
    );
    assert_alpha(&p, LEFT - 1, mid, 0.0, "outside the box");
}

/// An inset shadow paints **above** the element's background (an outer one
/// paints below it, where the background hides it).
#[test]
fn an_inset_shadow_paints_above_the_background() {
    let p = painted("background: rgb(255,255,255); box-shadow: inset 10px 20px 0 0 rgb(0,0,0)");
    let [r, g, b, a] = rgba(&p, LEFT + 5, TOP + 70);
    assert_eq!(
        (r, g, b, a),
        (0.0, 0.0, 0.0, 1.0),
        "the background painted over the inset shadow"
    );
    let [r, g, b, _] = rgba(&p, LEFT + 50, TOP + 70);
    assert_eq!((r, g, b), (1.0, 1.0, 1.0), "the background itself");
}

/// The first shadow in the list is on top. A 10px black ring over a 20px red
/// one: black for the first 10px, red for the next 10.
#[test]
fn the_first_inset_shadow_is_on_top() {
    let p = painted("box-shadow: inset 0 0 0 10px rgb(0,0,0), inset 0 0 0 20px rgb(255,0,0)");
    let mid = TOP + 50;
    let [r, g, b, a] = rgba(&p, LEFT + 5, mid);
    assert_eq!((r, g, b, a), (0.0, 0.0, 0.0, 1.0), "depth 5: the first shadow");
    let [r, g, b, a] = rgba(&p, LEFT + 15, mid);
    assert_eq!((r, g, b, a), (1.0, 0.0, 0.0, 1.0), "depth 15: the second shadow");
    assert_alpha(&p, LEFT + 25, mid, 0.0, "depth 25: neither");
}

/// A negative spread grows the hole past the padding box, which leaves no
/// shadow at all (Chrome 153: nothing painted).
#[test]
fn a_negative_spread_inset_shadow_paints_nothing() {
    let p = painted("box-shadow: inset 0 0 0 -10px rgb(0,0,0)");
    for (x, y) in [(LEFT, TOP), (LEFT + 5, TOP + 50), (LEFT + 50, TOP + 50)] {
        assert_alpha(&p, x, y, 0.0, "negative-spread inset shadow");
    }
}

/// A spread larger than half the box leaves no hole: the whole padding box is
/// shadow, including its middle.
#[test]
fn an_inset_spread_past_the_middle_fills_the_whole_padding_box() {
    let p = painted("box-shadow: inset 0 0 0 60px rgb(0,0,0)");
    assert_alpha(&p, LEFT + 50, TOP + 50, 1.0, "the middle");
    assert_alpha(&p, LEFT - 1, TOP + 50, 0.0, "outside the box");
}
