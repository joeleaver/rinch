//! #977: the shape `border-radius` gives a box — css-backgrounds-3 §5.5.
//!
//! Each corner has a **horizontal and a vertical** radius (`20px / 10px`), a
//! percentage resolves **per axis** (against the border box's width for the
//! horizontal radius, its height for the vertical one), and where the radii
//! along one side add up to more than that side, **every** radius of the box
//! is scaled down by one factor, `f = min(L / S)` over the four sides. rinch
//! kept one circular radius per corner (the horizontal one), resolved a
//! percentage against the shorter side, and let kurbo clamp each corner to
//! half the shorter side on its own — so `100px 0 0 0` on a 200x60 box drew a
//! 30px corner where Chrome draws 60.
//!
//! The oracle is the spec's own shape, computed here independently of the
//! painter: a pixel whose four corners all lie inside the shape must be
//! painted, one whose four corners all lie outside must not be. Pixels the
//! boundary crosses are anti-aliased and are not judged. Every fixture sits off
//! the fixed points where a circular radius and the spec agree (square boxes,
//! uniform radii that fit, equal horizontal and vertical radii).

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const LEFT: f64 = 50.0;
const TOP: f64 = 40.0;
const CANVAS: (u32, u32) = (400, 300);

/// Paint `<div style="… {style}">` inside `<div style="{outer}">` — the outer
/// one only when `outer` is non-empty, for the clip fixtures.
fn painted(w: f64, h: f64, style: &str, child: Option<&str>) -> TinySkiaPainter {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        &format!(
            "position: absolute; left: {LEFT}px; top: {TOP}px; width: {w}px; height: {h}px; \
             box-sizing: border-box; {style}"
        ),
    );
    if let Some(child_style) = child {
        let c = doc.create_element("div");
        doc.set_attribute(c, "style", child_style);
        doc.append_child(div, c);
    }
    doc.append_child(body, div);
    doc.resolve_layout(CANVAS.0 as f32, CANVAS.1 as f32);
    let mut painter = TinySkiaPainter::new(CANVAS.0, CANVAS.1);
    let mut paint_layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (CANVAS.0 as f32, CANVAS.1 as f32),
        &mut doc.font_cx,
        &mut paint_layout_cx,
    );
    painter
}

fn alpha(p: &TinySkiaPainter, x: i64, y: i64) -> f64 {
    let idx = ((y as u32 * p.width() + x as u32) * 4) as usize;
    p.pixels()[idx + 3] as f64 / 255.0
}

/// A rounded box: its rect and `[tl, tr, br, bl]` as `(horizontal, vertical)`.
#[derive(Clone, Copy, Debug)]
struct Shape {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    r: [(f64, f64); 4],
}

impl Shape {
    /// The spec's used radii (§5.5): a corner with a zero radius on either
    /// axis is square, then every radius is scaled by `min(1, L / S)`.
    fn new(x0: f64, y0: f64, w: f64, h: f64, raw: [(f64, f64); 4]) -> Self {
        let mut r = raw.map(|(a, b)| {
            if a <= 0.0 || b <= 0.0 {
                (0.0, 0.0)
            } else {
                (a, b)
            }
        });
        let sums = [
            (w, r[0].0 + r[1].0),
            (h, r[1].1 + r[2].1),
            (w, r[2].0 + r[3].0),
            (h, r[3].1 + r[0].1),
        ];
        let f = sums
            .iter()
            .filter(|(_, s)| *s > 0.0)
            .map(|(l, s)| l / s)
            .fold(1.0f64, f64::min);
        for c in &mut r {
            c.0 *= f;
            c.1 *= f;
        }
        Shape {
            x0,
            y0,
            x1: x0 + w,
            y1: y0 + h,
            r,
        }
    }

    /// The box inset by the border widths, with the inner radii §5.5 derives:
    /// each outer radius less the border on its own axis, floored at zero.
    fn inset(&self, top: f64, right: f64, bottom: f64, left: f64) -> Self {
        let [tl, tr, br, bl] = self.r;
        let d = |v: f64, by: f64| (v - by).max(0.0);
        Shape {
            x0: self.x0 + left,
            y0: self.y0 + top,
            x1: self.x1 - right,
            y1: self.y1 - bottom,
            r: [
                (d(tl.0, left), d(tl.1, top)),
                (d(tr.0, right), d(tr.1, top)),
                (d(br.0, right), d(br.1, bottom)),
                (d(bl.0, left), d(bl.1, bottom)),
            ],
        }
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        if x < self.x0 || x > self.x1 || y < self.y0 || y > self.y1 {
            return false;
        }
        let [tl, tr, br, bl] = self.r;
        // (corner radius, ellipse centre, is the point in the corner's quadrant)
        let corners = [
            (
                tl,
                (self.x0 + tl.0, self.y0 + tl.1),
                x < self.x0 + tl.0 && y < self.y0 + tl.1,
            ),
            (
                tr,
                (self.x1 - tr.0, self.y0 + tr.1),
                x > self.x1 - tr.0 && y < self.y0 + tr.1,
            ),
            (
                br,
                (self.x1 - br.0, self.y1 - br.1),
                x > self.x1 - br.0 && y > self.y1 - br.1,
            ),
            (
                bl,
                (self.x0 + bl.0, self.y1 - bl.1),
                x < self.x0 + bl.0 && y > self.y1 - bl.1,
            ),
        ];
        for ((rx, ry), (cx, cy), in_quadrant) in corners {
            if in_quadrant && rx > 0.0 && ry > 0.0 {
                let u = (x - cx) / rx;
                let v = (y - cy) / ry;
                if u * u + v * v > 1.0 {
                    return false;
                }
            }
        }
        true
    }
}

/// Count the pixels of the canvas the painter got wrong against `inside`:
/// a pixel all four of whose corners are inside must be painted (alpha over
/// 0.75), one all four of whose corners are outside must be blank (under
/// 0.25). Returns `(wrong, judged inside, judged outside)` so a caller can
/// see the oracle judged something.
fn mismatches(p: &TinySkiaPainter, inside: impl Fn(f64, f64) -> bool) -> (usize, usize, usize) {
    let (mut wrong, mut n_in, mut n_out) = (0, 0, 0);
    for y in 0..CANVAS.1 as i64 {
        for x in 0..CANVAS.0 as i64 {
            let (fx, fy) = (x as f64, y as f64);
            let corners = [
                inside(fx, fy),
                inside(fx + 1.0, fy),
                inside(fx, fy + 1.0),
                inside(fx + 1.0, fy + 1.0),
            ];
            let a = alpha(p, x, y);
            if corners.iter().all(|&c| c) {
                n_in += 1;
                if a < 0.75 {
                    wrong += 1;
                }
            } else if corners.iter().all(|&c| !c) {
                n_out += 1;
                if a > 0.25 {
                    wrong += 1;
                }
            }
        }
    }
    (wrong, n_in, n_out)
}

fn assert_fill(p: &TinySkiaPainter, shape: Shape, what: &str) {
    let (wrong, n_in, n_out) = mismatches(p, |x, y| shape.contains(x, y));
    assert!(
        n_in > 1000 && n_out > 1000,
        "{what}: the oracle judged too little"
    );
    assert_eq!(
        wrong, 0,
        "{what}: {wrong} pixels differ from the spec's shape {shape:?}"
    );
}

fn assert_ring(p: &TinySkiaPainter, outer: Shape, inner: Shape, what: &str) {
    let (wrong, n_in, n_out) = mismatches(p, |x, y| outer.contains(x, y) && !inner.contains(x, y));
    assert!(
        n_in > 200 && n_out > 1000,
        "{what}: the oracle judged too little"
    );
    assert_eq!(
        wrong, 0,
        "{what}: {wrong} pixels differ from the spec's ring {outer:?} - {inner:?}"
    );
}

/// `20px / 10px`-style elliptical radii: the vertical radius was dropped and
/// the corner drawn as a 40px circle.
#[test]
fn an_elliptical_radius_rounds_each_axis_by_its_own_radius() {
    let p = painted(
        200.0,
        100.0,
        "background: black; border-radius: 40px / 12px",
        None,
    );
    assert_fill(
        &p,
        Shape::new(LEFT, TOP, 200.0, 100.0, [(40.0, 12.0); 4]),
        "border-radius: 40px / 12px",
    );
}

/// A percentage resolves per axis: `20%` of 200x80 is 40 across and 16 down.
/// It used to be 20% of the shorter side, a 16px circle.
#[test]
fn a_percentage_radius_resolves_against_each_axis() {
    let p = painted(200.0, 80.0, "background: black; border-radius: 20%", None);
    assert_fill(
        &p,
        Shape::new(LEFT, TOP, 200.0, 80.0, [(40.0, 16.0); 4]),
        "border-radius: 20%",
    );
}

/// The issue's third case: `100px 0 0 0` on 200x60. The left side holds
/// 100 of vertical radius against 60 of height, so every radius is scaled by
/// 0.6 — a 60px corner. kurbo clamped the one corner to half the shorter
/// side, 30.
#[test]
fn overlapping_radii_are_scaled_down_together() {
    let p = painted(
        200.0,
        60.0,
        "background: black; border-radius: 100px 0 0 0",
        None,
    );
    assert_fill(
        &p,
        Shape::new(
            LEFT,
            TOP,
            200.0,
            60.0,
            [(100.0, 100.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)],
        ),
        "border-radius: 100px 0 0 0",
    );
}

/// The scaling takes the **smallest** ratio over the four sides and applies
/// it to every corner, the ones on sides that fit included: here the top side
/// needs 0.8, so the bottom corners shrink from 20/40 to 16/32 too.
#[test]
fn one_factor_scales_every_corner() {
    let p = painted(
        200.0,
        100.0,
        "background: black; border-radius: 150px 100px 20px 20px / 30px 30px 40px 40px",
        None,
    );
    assert_fill(
        &p,
        Shape::new(
            LEFT,
            TOP,
            200.0,
            100.0,
            [(150.0, 30.0), (100.0, 30.0), (20.0, 40.0), (20.0, 40.0)],
        ),
        "border-radius: 150px 100px 20px 20px / 30px 30px 40px 40px",
    );
}

/// `50%` on a non-square box is an ellipse — the pill-to-ellipse case.
#[test]
fn fifty_percent_on_a_wide_box_is_an_ellipse() {
    let p = painted(240.0, 100.0, "background: black; border-radius: 50%", None);
    assert_fill(
        &p,
        Shape::new(LEFT, TOP, 240.0, 100.0, [(120.0, 50.0); 4]),
        "border-radius: 50%",
    );
}

/// A clipping box clips its content to its padding box with the inner radii:
/// here `40px / 12px` less a 4px border.
#[test]
fn overflow_clips_to_the_elliptical_padding_shape() {
    let p = painted(
        200.0,
        100.0,
        "overflow: hidden; border-radius: 40px / 12px; border: 4px solid transparent",
        Some("width: 300px; height: 300px; background: black"),
    );
    let outer = Shape::new(LEFT, TOP, 200.0, 100.0, [(40.0, 12.0); 4]);
    assert_fill(&p, outer.inset(4.0, 4.0, 4.0, 4.0), "the clip");
}

/// A uniform solid border is the ring between the border box's shape and the
/// padding box's, both elliptical.
#[test]
fn a_uniform_border_is_the_elliptical_ring() {
    let p = painted(
        200.0,
        100.0,
        "border: 6px solid black; border-radius: 50px / 20px",
        None,
    );
    let outer = Shape::new(LEFT, TOP, 200.0, 100.0, [(50.0, 20.0); 4]);
    assert_ring(
        &p,
        outer,
        outer.inset(6.0, 6.0, 6.0, 6.0),
        "border: 6px solid",
    );
}

/// The per-side path (one colour per side, the spinner shape) rounds
/// elliptically too.
#[test]
fn a_per_side_border_rounds_elliptically() {
    let p = painted(
        240.0,
        100.0,
        "border: 6px solid black; border-top-color: rgb(0, 0, 1); border-radius: 50%",
        None,
    );
    let outer = Shape::new(LEFT, TOP, 240.0, 100.0, [(120.0, 50.0); 4]);
    assert_ring(
        &p,
        outer,
        outer.inset(6.0, 6.0, 6.0, 6.0),
        "per-side border, 50%",
    );
}

/// The issue's measurement shape: a spread shadow outlining the element
/// follows its scaled radii (here the overlap case, grown by the spread).
#[test]
fn a_spread_shadow_follows_the_scaled_radii() {
    let p = painted(
        200.0,
        60.0,
        "border-radius: 100px 0 0 0; box-shadow: 0 0 0 10px black",
        None,
    );
    let outer = Shape::new(
        LEFT - 10.0,
        TOP - 10.0,
        220.0,
        80.0,
        [(70.0, 70.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)],
    );
    let element = Shape::new(
        LEFT,
        TOP,
        200.0,
        60.0,
        [(60.0, 60.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)],
    );
    assert_ring(&p, outer, element, "0 0 0 10px over 100px 0 0 0");
}

/// An elliptical spread shadow: each radius grows by the spread on its own
/// axis (§7.1.1, `r >= spread` on both).
#[test]
fn an_elliptical_spread_shadow_grows_each_axis() {
    let p = painted(
        200.0,
        100.0,
        "border-radius: 40px / 12px; box-shadow: 0 0 0 8px black",
        None,
    );
    let outer = Shape::new(LEFT - 8.0, TOP - 8.0, 216.0, 116.0, [(48.0, 20.0); 4]);
    let element = Shape::new(LEFT, TOP, 200.0, 100.0, [(40.0, 12.0); 4]);
    assert_ring(&p, outer, element, "0 0 0 8px over 40px / 12px");
}

/// The control: a circular radius that fits draws as it always did — this
/// one passed before #977 too, and proves the oracle agrees with the painter
/// where nothing was wrong.
#[test]
fn a_circular_radius_that_fits_is_unchanged() {
    let p = painted(200.0, 100.0, "background: black; border-radius: 20px", None);
    assert_fill(
        &p,
        Shape::new(LEFT, TOP, 200.0, 100.0, [(20.0, 20.0); 4]),
        "border-radius: 20px",
    );
}

/// A uniform circular border's outer edge is the border box's radius and its
/// inner edge that less the width. The one-stroke path stroked the border's
/// centre line with the **outer** radius, so both edges sat half a width too
/// round (outer 23, inner 17 here, for 20 and 14).
#[test]
fn a_uniform_circular_border_has_the_outer_radius_outside() {
    let p = painted(
        200.0,
        100.0,
        "border: 6px solid black; border-radius: 20px",
        None,
    );
    let outer = Shape::new(LEFT, TOP, 200.0, 100.0, [(20.0, 20.0); 4]);
    assert_ring(
        &p,
        outer,
        outer.inset(6.0, 6.0, 6.0, 6.0),
        "border: 6px, 20px",
    );
}

/// A `border-radius` transition moves both radii of a corner together. Only
/// the horizontal radius was transitioned (it was the only one rinch kept), so
/// with the vertical one written straight to its end value a circular corner
/// went elliptical for the whole transition.
#[test]
fn a_radius_transition_interpolates_both_axes() {
    use rinch_dom::transition::TransitionProperty;
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let style_el = doc.create_element("style");
    let text = doc.create_text(
        ".b { width: 200px; height: 100px; border-radius: 10px; \
              transition: border-radius 100ms linear; } \
         .b.on { border-radius: 30px / 50px; }",
    );
    doc.append_child(style_el, text);
    doc.append_child(body, style_el);
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "b");
    doc.append_child(body, div);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);

    doc.set_attribute(div, "class", "b on");
    doc.resolve_layout(800.0, 600.0);
    let start = doc
        .tree
        .active_transitions
        .get(&div.0)
        .and_then(|t| t.get(&TransitionProperty::BorderRadiusTopLeft))
        .expect("the class change should start a border-radius transition")
        .start_time_ms;
    rinch_dom::transition::tick_transitions(&mut doc.tree, start + 50.0);
    let cs = &doc.tree.nodes[div.0].computed_style;
    let (x, y) = (
        cs.border_radius_top_left.to_px(),
        cs.border_radius_top_left_y.to_px(),
    );
    assert!(
        (x - 20.0).abs() < 0.01 && (y - 30.0).abs() < 0.01,
        "half-way from 10px to 30px / 50px the corner is 20 / 30, got {x} / {y}"
    );
}

/// With unequal borders each inner radius loses the border **on its own
/// axis**: `30px` less a 12px left/right and a 2px top/bottom border is
/// `18 / 28`, an ellipse. It was one circle less the average of the two
/// widths, 23.
#[test]
fn overflow_clip_inner_radii_lose_each_axis_border() {
    let p = painted(
        200.0,
        100.0,
        "overflow: hidden; border-radius: 30px; border-style: solid; \
         border-color: transparent; border-width: 2px 12px",
        Some("width: 300px; height: 300px; background: black"),
    );
    let outer = Shape::new(LEFT, TOP, 200.0, 100.0, [(30.0, 30.0); 4]);
    assert_fill(&p, outer.inset(2.0, 12.0, 2.0, 12.0), "the clip");
}

/// A `double` border is stroked along the border's centre line, whose radius
/// is the outer one less half the width. The ring's middle on the corner's
/// diagonal (45px from the corner centre, between the 72 outer and 24 inner
/// radii) is on that line; stroked with the outer radius instead, the line
/// sits about 10px further in and the 16px band misses the point.
#[test]
fn a_double_border_strokes_the_centre_line_radius() {
    let p = painted(
        300.0,
        240.0,
        "border: 48px double black; border-radius: 96px",
        None,
    );
    let outer = Shape::new(LEFT, TOP, 300.0, 240.0, [(96.0, 96.0); 4]);
    let inner = outer.inset(48.0, 48.0, 48.0, 48.0);
    // Nothing outside the ring.
    let ring = |x: f64, y: f64| outer.contains(x, y) && !inner.contains(x, y);
    let mut outside = 0;
    for y in 0..CANVAS.1 as i64 {
        for x in 0..CANVAS.0 as i64 {
            let (fx, fy) = (x as f64, y as f64);
            let out = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
                .iter()
                .all(|(dx, dy)| !ring(fx + dx, fy + dy));
            if out && alpha(&p, x, y) > 0.25 {
                outside += 1;
            }
        }
    }
    assert_eq!(outside, 0, "the double border painted outside its ring");
    // The ring's middle radius, 72, on the top-left corner's diagonal.
    let d = 72.0 / std::f64::consts::SQRT_2;
    let (cx, cy) = (LEFT + 96.0 - d, TOP + 96.0 - d);
    assert!(
        alpha(&p, cx as i64, cy as i64) > 0.5,
        "the double border's band misses the ring's centre on the corner diagonal \
         at ({cx:.1}, {cy:.1})"
    );
}

/// An outline follows the elliptical corner, each radius grown by the
/// outline's distance from the border edge.
#[test]
fn an_outline_grows_each_radius_of_an_elliptical_corner() {
    let p = painted(
        200.0,
        100.0,
        "outline: 10px solid black; border-radius: 40px / 12px",
        None,
    );
    let element = Shape::new(LEFT, TOP, 200.0, 100.0, [(40.0, 12.0); 4]);
    let outer = Shape::new(LEFT - 10.0, TOP - 10.0, 220.0, 120.0, [(50.0, 22.0); 4]);
    assert_ring(&p, outer, element, "outline: 10px over 40px / 12px");
}

/// A blurred inset shadow's hole has elliptical corners: with an `80px /
/// 24px` box and an 8px spread the hole's corner is `72 / 16` (§7.1.1), and a
/// pixel just outside that ellipse — 2.5px out, 5.5px and more from the
/// hole's straight edges — is in shadow. Measured against the circle of the
/// horizontal radius, it was inside the hole and clear.
#[test]
fn a_blurred_inset_shadow_hole_has_elliptical_corners() {
    let p = painted(
        200.0,
        100.0,
        "border-radius: 80px / 24px; box-shadow: inset 0 0 2px 8px black",
        None,
    );
    for (x, y) in [(LEFT + 16.0, TOP + 13.0), (LEFT + 183.0, TOP + 86.0)] {
        assert!(
            alpha(&p, x as i64, y as i64) > 0.6,
            "({x}, {y}) is outside the hole's elliptical corner and should be in shadow"
        );
    }
    // Control: well inside the hole there is no shadow.
    assert!(alpha(&p, (LEFT + 60.0) as i64, (TOP + 30.0) as i64) < 0.05);
}

/// Half-way through a `border-radius` transition from `from` to `to` on a
/// 200x100 box: the top-left corner's `(horizontal, vertical)` radii in px, or
/// `None` when no transition started.
fn radius_mid_transition(from: &str, to: &str) -> Option<(f32, f32)> {
    use rinch_dom::transition::TransitionProperty;
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let style_el = doc.create_element("style");
    let text = doc.create_text(&format!(
        ".b {{ width: 200px; height: 100px; border-radius: {from}; \
              transition: border-radius 100ms linear; }} \
         .b.on {{ border-radius: {to}; }}"
    ));
    doc.append_child(style_el, text);
    doc.append_child(body, style_el);
    let div = doc.create_element("div");
    doc.set_attribute(div, "class", "b");
    doc.append_child(body, div);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    doc.set_attribute(div, "class", "b on");
    doc.resolve_layout(800.0, 600.0);
    let start = doc
        .tree
        .active_transitions
        .get(&div.0)
        .and_then(|t| t.get(&TransitionProperty::BorderRadiusTopLeft))?
        .start_time_ms;
    rinch_dom::transition::tick_transitions(&mut doc.tree, start + 50.0);
    let cs = &doc.tree.nodes[div.0].computed_style;
    Some((
        cs.border_radius_top_left.resolve(200.0),
        cs.border_radius_top_left_y.resolve(100.0),
    ))
}

/// A change of the vertical radius alone (`10px` → `10px / 30px`) is a
/// change of the corner and starts a transition (review of #1640).
#[test]
fn a_vertical_only_radius_change_transitions() {
    let (x, y) = radius_mid_transition("10px", "10px / 30px")
        .expect("a vertical-only change should start a transition");
    assert!(
        (x - 10.0).abs() < 0.01 && (y - 20.0).abs() < 0.01,
        "{x} / {y}"
    );
}

/// Each axis of a corner interpolates on its own (review of #1640: the first
/// cut snapped the whole corner, where the horizontal radius animated before
/// #977). A length against a percentage interpolates as a `calc()` (#767):
/// Chrome 156 reads `15px calc(25% + 5px)` at the midpoint of
/// `10px` → `20px 50%`, which is 30px on a 100px-tall box. It used to snap to
/// its end (50).
#[test]
fn a_corner_interpolates_each_axis_on_its_own() {
    let (x, y) = radius_mid_transition("10px", "20px / 50%")
        .expect("the class change should start a transition");
    assert!((x - 15.0).abs() < 0.01, "the length axis animates: {x}");
    assert!(
        (y - 30.0).abs() < 0.01,
        "the mixed axis interpolates as calc(25% + 5px): {y}"
    );
}

/// An elliptical hard shadow offset past its box does not contain the box,
/// so the box is not punched out of it; the part of the (transparent) box the
/// shadow does not cover stays blank (review of #1640).
#[test]
fn an_offset_elliptical_shadow_does_not_paint_inside_the_box() {
    let p = painted(
        160.0,
        60.0,
        "border-radius: 40px / 15px; box-shadow: 30px 0 0 rgb(0,0,0)",
        None,
    );
    // Inside the box, left of where the shadow starts.
    assert_eq!(alpha(&p, 60, 70), 0.0, "inside the box, outside the shadow");
    // Positive control: the shadow is painted right of the box.
    assert!(alpha(&p, 225, 70) > 0.8, "the shadow itself is painted");
}

/// A circular pill — any radius at least half the shorter side, at the
/// common scales — stays on kurbo's `RoundedRect` path rather than slipping
/// onto the elliptical `BezPath` by a float (review of #1640).
#[test]
fn pills_take_the_rounded_rect_path() {
    use peniko::kurbo::Rect;
    use rinch_dom::paint::painter::PaintShape;
    use rinch_dom::paint::radii::BorderRadii;
    let mut slipped = Vec::new();
    for &r in &[1000.0f64, 9999.0, 500.0, 32.0] {
        for &scale in &[1.0f64, 1.25, 1.5, 2.0] {
            for w in 1..=240 {
                for h in 1..=64 {
                    let (sw, sh) = (w as f64 * scale, h as f64 * scale);
                    let radii = BorderRadii::uniform(r * scale).fitted(sw, sh);
                    if let PaintShape::BezPath(_) = radii.shape(Rect::new(0.0, 0.0, sw, sh)) {
                        slipped.push((r, scale, w, h));
                    }
                }
            }
        }
    }
    assert!(
        slipped.is_empty(),
        "pills on the BezPath path: {:?}",
        &slipped[..slipped.len().min(8)]
    );
}

/// A `data-render-surface` with no frame yet paints its background with the
/// element's rounded shape (it used to take the top-left horizontal radius
/// as one circle).
#[test]
fn a_render_surface_fallback_background_is_elliptical() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "data-render-surface", "nothing-registered");
    doc.set_attribute(
        div,
        "style",
        &format!(
            "position: absolute; left: {LEFT}px; top: {TOP}px; width: 200px; height: 100px; \
             background: black; border-radius: 40px / 12px"
        ),
    );
    doc.append_child(body, div);
    doc.resolve_layout(CANVAS.0 as f32, CANVAS.1 as f32);
    let mut p = TinySkiaPainter::new(CANVAS.0, CANVAS.1);
    let mut lcx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut p,
        1.0,
        (CANVAS.0 as f32, CANVAS.1 as f32),
        &mut doc.font_cx,
        &mut lcx,
    );
    assert_fill(
        &p,
        Shape::new(LEFT, TOP, 200.0, 100.0, [(40.0, 12.0); 4]),
        "render-surface fallback",
    );
}
