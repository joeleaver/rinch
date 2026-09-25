//! #351: a `box-shadow` spread grows the shadow's corner radii with its rect.
//!
//! css-backgrounds-3 §7.1.1 ("Shadow Shape, Spread, and Knockout"): an outer
//! shadow's corner radius is the border radius **plus the spread** (floored at
//! zero for a negative spread), except that where the border radius `r` is
//! less than the spread `s` the spread is first multiplied by
//! `1 + (r/s - 1)^3` — which is what keeps a square corner (`r == 0`) square
//! however far it is spread, and makes a small radius grow continuously out of
//! it rather than jumping to `r + s`.
//!
//! rinch grew the rect by the spread and kept the element's radii, so every
//! spread shadow on a rounded box had corners tighter than a browser's. The
//! expected radii below are the spec formula, and they are also what Chrome
//! 153 paints, measured with the same uncovered-area method on a headless
//! screenshot of the same boxes (`box-shadow: 0 0 0 <s> #000` on a 100x100
//! white box, corner radius recovered from the uncovered area of the corner):
//!
//! | r  | s  | spec  | Chrome 153 | rinch before |
//! |----|----|-------|------------|--------------|
//! | 4  | 20 | 13.76 | 13.79      | 4            |
//! | 12 | 20 | 30.72 | 30.90      | 12           |
//! | 30 | 10 | 40    | 40.00      | 30           |
//! | 8  | 4  | 12    | 12.11      | 8            |
//! | 20 | 10 | 30    | 30.29      | 20           |
//! | 0  | 10 | 0     | 0.00       | 0            |
//! | 8  | -4 | 4     | 3.7        | 8            |
//!
//! The rows are deliberately off the fixed points: `r == 0` is the one input
//! on which the old code, a bare `r + s` and the spec all agree for a square
//! box, and `r >= s` is the one region on which a bare `r + s` and the spec
//! agree — so each formula a plausible fix could use is killed by some row.
//!
//! The boxes have **no background**, which is what makes the second half of
//! each test mean something. An outer shadow is never painted inside the
//! border box; rinch punches the box out of the shadow only where it can prove
//! the box lies inside the shadow's shape, and falls back to a whole fill where
//! it cannot. With the element's own radii on a rect grown by the spread, the
//! box's corner arcs escaped the shadow's and every rounded spread shadow took
//! that fallback — painting the shadow *through* a transparent element.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const BOX: f64 = 100.0;
const LEFT: f64 = 150.0;
const TOP: f64 = 150.0;

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

fn alpha(p: &TinySkiaPainter, x: i64, y: i64) -> f64 {
    let idx = ((y as u32 * p.width() + x as u32) * 4) as usize;
    p.pixels()[idx + 3] as f64 / 255.0
}

#[derive(Clone, Copy)]
enum Corner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

/// The radius of one corner of an **opaque, unblurred** shadow grown by
/// `spread` around the box, recovered from how much of the corner it leaves
/// uncovered: a quarter-round corner of radius `R` leaves `R^2 (1 - pi/4)`.
///
/// Only pixels outside the element's own square box are counted — the element
/// is transparent and its inside is empty whatever the shadow's shape, so
/// counting it would swamp the measure. The window is 60px on a side, wider
/// than any radius measured here.
fn corner_radius(p: &TinySkiaPainter, corner: Corner, spread: f64, dx: f64, dy: f64) -> f64 {
    const N: i64 = 60;
    // The shadow rect's outer corner pixel, and the inward step along x and y.
    let (cx, cy, sx, sy) = match corner {
        Corner::TopLeft => (LEFT + dx - spread, TOP + dy - spread, 1, 1),
        Corner::TopRight => (LEFT + BOX + dx + spread - 1.0, TOP + dy - spread, -1, 1),
        Corner::BottomRight => (
            LEFT + BOX + dx + spread - 1.0,
            TOP + BOX + dy + spread - 1.0,
            -1,
            -1,
        ),
        Corner::BottomLeft => (LEFT + dx - spread, TOP + BOX + dy + spread - 1.0, 1, -1),
    };
    let mut uncovered = 0.0;
    for j in 0..N {
        for i in 0..N {
            let (x, y) = (cx as i64 + sx * i, cy as i64 + sy * j);
            // Inside the element's square box: not the shadow's business.
            let ex = x as f64 + 0.5;
            let ey = y as f64 + 0.5;
            if ex > LEFT && ex < LEFT + BOX && ey > TOP && ey < TOP + BOX {
                continue;
            }
            uncovered += 1.0 - alpha(p, x, y);
        }
    }
    (uncovered / (1.0 - std::f64::consts::PI / 4.0)).sqrt()
}

/// The spec's radius for an outer shadow: css-backgrounds-3 §7.1.1.
fn spec_radius(r: f64, s: f64) -> f64 {
    if s > 0.0 && r < s {
        let q = r / s;
        r + s * (1.0 + (q - 1.0).powi(3))
    } else {
        (r + s).max(0.0)
    }
}

fn assert_radius(p: &TinySkiaPainter, corner: Corner, r: f64, s: f64, dx: f64, dy: f64) {
    let got = corner_radius(p, corner, s, dx, dy);
    let want = spec_radius(r, s);
    assert!(
        (got - want).abs() <= 0.5,
        "r={r} spread={s}: the shadow's corner radius measured {got:.2}, \
         the spec (and Chrome) give {want:.2}"
    );
}

/// Nothing of an outer shadow is painted inside the element's border box.
fn assert_inside_empty(p: &TinySkiaPainter) {
    for (x, y) in [(200, 200), (160, 160), (240, 240), (160, 240)] {
        assert_eq!(
            alpha(p, x, y),
            0.0,
            "an outer box-shadow painted inside the transparent element at ({x}, {y})"
        );
    }
}

#[test]
fn a_small_radius_grows_by_the_spread_ratio_rule() {
    let p = painted("border-radius: 4px; box-shadow: 0 0 0 20px rgb(0,0,0)");
    assert_radius(&p, Corner::TopLeft, 4.0, 20.0, 0.0, 0.0);
    assert_radius(&p, Corner::BottomRight, 4.0, 20.0, 0.0, 0.0);
    assert_inside_empty(&p);
}

/// `r / s = 0.6`: the ratio rule gives 30.72 where a bare `r + s` gives 32.
#[test]
fn a_radius_just_under_the_spread_is_not_simply_added() {
    let p = painted("border-radius: 12px; box-shadow: 0 0 0 20px rgb(0,0,0)");
    assert_radius(&p, Corner::TopRight, 12.0, 20.0, 0.0, 0.0);
    assert_inside_empty(&p);
}

#[test]
fn a_radius_at_least_the_spread_grows_by_the_whole_spread() {
    let p = painted("border-radius: 30px; box-shadow: 0 0 0 10px rgb(0,0,0)");
    assert_radius(&p, Corner::TopLeft, 30.0, 10.0, 0.0, 0.0);
    assert_radius(&p, Corner::BottomLeft, 30.0, 10.0, 0.0, 0.0);
    assert_inside_empty(&p);
}

/// The issue's own example: `0 0 0 4px` over `border-radius: 8px` is 12.
#[test]
fn the_issue_example_is_twelve() {
    let p = painted("border-radius: 8px; box-shadow: 0 0 0 4px rgb(0,0,0)");
    assert_radius(&p, Corner::TopLeft, 8.0, 4.0, 0.0, 0.0);
    assert_inside_empty(&p);
}

/// A square corner stays square when its neighbours are round: the ratio rule
/// multiplies the spread by `1 + (0 - 1)^3 = 0`. A bare `r + s` rounds it to
/// the spread.
#[test]
fn a_square_corner_beside_round_ones_stays_square() {
    let p = painted("border-radius: 0 20px 20px 20px; box-shadow: 0 0 0 10px rgb(0,0,0)");
    assert_radius(&p, Corner::TopLeft, 0.0, 10.0, 0.0, 0.0);
    assert_radius(&p, Corner::TopRight, 20.0, 10.0, 0.0, 0.0);
    assert_inside_empty(&p);
}

/// A negative spread shrinks the radius with the rect. The shadow is offset
/// so that its bottom-right corner is clear of the element, where it can be
/// seen; its radius is `8 - 4`.
#[test]
fn a_negative_spread_shrinks_the_radius() {
    let p = painted("border-radius: 8px; box-shadow: 40px 40px 0 -4px rgb(0,0,0)");
    assert_radius(&p, Corner::BottomRight, 8.0, -4.0, 40.0, 40.0);
}

/// ...and floors it at zero: a 2px radius spread by -6px is a square corner,
/// not a negative radius.
#[test]
fn a_negative_spread_past_the_radius_leaves_a_square_corner() {
    let p = painted("border-radius: 2px; box-shadow: 40px 40px 0 -6px rgb(0,0,0)");
    assert_radius(&p, Corner::BottomRight, 2.0, -6.0, 40.0, 40.0);
}

/// The blurred shadow's layers carry the spread shape's radius too. Blur 2px
/// and spread 20px expand the outermost layer by `1 + 20 = 21`; its radius is
/// the spread shape's `13.76` plus the one pixel of blur, so its arc crosses
/// the diagonal `0.29 x 14.76 = 4.3px` in from the layer's corner. The pixel
/// 5.5px in is therefore inside that layer. The old `r + 21 = 25` put the
/// crossing at 7.3px and left that pixel empty.
#[test]
fn a_blurred_spread_shadow_layers_take_the_spread_radius() {
    let p = painted("border-radius: 4px; box-shadow: 0 0 2px 20px rgb(0,0,0)");
    let corner = LEFT - 21.0;
    let a = alpha(&p, corner as i64 + 5, corner as i64 + 5);
    assert!(
        a > 0.0,
        "the outermost blur layer's corner is too round: alpha {a} at 5.5px in"
    );
    // And not so sharp that its corner pixel is covered.
    assert_eq!(alpha(&p, corner as i64, corner as i64), 0.0);
    assert_inside_empty(&p);
}
