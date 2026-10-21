//! The rounded shape `border-radius` gives a box (css-backgrounds-3 §5.5, #977).
//!
//! A corner has **two** radii — horizontal and vertical — so it is a quarter
//! ellipse, not a quarter circle; a percentage resolves per axis (against the
//! box's width for the horizontal radius, its height for the vertical one);
//! a corner with a zero radius on either axis is square; and where the radii
//! along one side add up to more than that side, **every** radius of the box is
//! scaled by one factor `f = min(Lᵢ / Sᵢ)` over the four sides (§5.5,
//! "Overlapping Curves").
//!
//! rinch used kurbo's [`RoundedRectRadii`] — one circular radius per corner,
//! the horizontal one — and let kurbo clamp each corner to half the shorter
//! side on its own, so `20px / 10px` drew a 20px circle and `100px 0 0 0` on a
//! 200x60 box a 30px corner where CSS (and Chrome) draw 60. [`BorderRadii`] is
//! the shape every painter of a rounded box asks now: the background, the
//! clip, the border, the outline and the outer shadow.
//!
//! [`BorderRadii::shape`] still hands the painter a kurbo [`RoundedRect`]
//! whenever the corners are circular and each fits in half the shorter side —
//! the case kurbo draws exactly — so the *shape* of every box that was right
//! before is the same `RoundedRect` it always was, and only an elliptical or
//! overlapping one becomes a [`BezPath`] of elliptical arcs. (Its pixels are
//! not all unchanged: a solid rounded border is now filled as the ring
//! between the border-box and padding-box shapes rather than stroked, which
//! moves the corner pixels of every outlined component to CSS's radii.)

use peniko::kurbo::{
    Arc, BezPath, ParamCurve, PathEl, PathSeg, Point, Rect, RoundedRect, RoundedRectRadii, Shape,
    Vec2,
};

use super::painter::PaintShape;

/// Path flattening tolerance for the elliptical arcs, the same kurbo uses for
/// a [`RoundedRect`] everywhere else in paint.
const TOLERANCE: f64 = 0.1;

/// Flattening tolerance for the corners [`BorderRadii::path_with_holes`]
/// flattens (the ones a hole meets): finer than [`TOLERANCE`], so such a
/// corner stays within a twentieth of a pixel of its arc. Its anti-aliased
/// pixels still differ from the arc's by a few supersamples.
const HOLE_TOLERANCE: f64 = 0.05;

/// A closed polygon contour (nothing for fewer than three points).
fn push_polygon(path: &mut BezPath, points: &[Point]) {
    if points.len() < 3 {
        return;
    }
    path.move_to(points[0]);
    for &p in &points[1..] {
        path.line_to(p);
    }
    path.close_path();
}

/// `polygon` clipped to `clip` (Sutherland–Hodgman over its four edges):
/// exact for a convex polygon, which a rounded rectangle's outline is.
fn clip_polygon(polygon: &[Point], clip: Rect) -> Vec<Point> {
    type Edge = (fn(Point, f64) -> f64, f64);
    // Each edge as "signed distance inside" for a point.
    let edges: [Edge; 4] = [
        (|p, x0| p.x - x0, clip.x0),
        (|p, x1| x1 - p.x, clip.x1),
        (|p, y0| p.y - y0, clip.y0),
        (|p, y1| y1 - p.y, clip.y1),
    ];
    let mut out = polygon.to_vec();
    for (dist, at) in edges {
        let input = std::mem::take(&mut out);
        let Some(&last) = input.last() else {
            break;
        };
        let mut prev = last;
        for &cur in &input {
            let (dp, dc) = (dist(prev, at), dist(cur, at));
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

/// A box's four corner radii, each `(horizontal, vertical)` as a [`Vec2`]
/// (`x` across, `y` down), in whatever units the caller works in.
///
/// Constructed through [`BorderRadii::new`] or [`BorderRadii::map`], which
/// make a corner with a non-positive radius on either axis square — CSS's
/// rule, and the invariant the shape builders rely on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BorderRadii {
    pub top_left: Vec2,
    pub top_right: Vec2,
    pub bottom_right: Vec2,
    pub bottom_left: Vec2,
}

fn square_if_degenerate(v: Vec2) -> Vec2 {
    if v.x > 0.0 && v.y > 0.0 {
        v
    } else {
        Vec2::ZERO
    }
}

impl BorderRadii {
    /// Four square corners.
    pub const ZERO: BorderRadii = BorderRadii {
        top_left: Vec2::ZERO,
        top_right: Vec2::ZERO,
        bottom_right: Vec2::ZERO,
        bottom_left: Vec2::ZERO,
    };

    /// `[top_left, top_right, bottom_right, bottom_left]`.
    pub fn new(top_left: Vec2, top_right: Vec2, bottom_right: Vec2, bottom_left: Vec2) -> Self {
        Self::from_corners([top_left, top_right, bottom_right, bottom_left])
    }

    /// Every corner a circle of radius `r`.
    pub fn uniform(r: f64) -> Self {
        let v = Vec2::new(r, r);
        Self::new(v, v, v, v)
    }

    /// Circular corners, as kurbo describes them.
    pub fn from_circular(r: RoundedRectRadii) -> Self {
        let c = |r: f64| Vec2::new(r, r);
        Self::new(
            c(r.top_left),
            c(r.top_right),
            c(r.bottom_right),
            c(r.bottom_left),
        )
    }

    /// `[top_left, top_right, bottom_right, bottom_left]`.
    pub fn corners(&self) -> [Vec2; 4] {
        [
            self.top_left,
            self.top_right,
            self.bottom_right,
            self.bottom_left,
        ]
    }

    fn from_corners(c: [Vec2; 4]) -> Self {
        let c = c.map(square_if_degenerate);
        BorderRadii {
            top_left: c[0],
            top_right: c[1],
            bottom_right: c[2],
            bottom_left: c[3],
        }
    }

    /// Each corner through `f` (a corner that comes out degenerate is square).
    pub fn map(&self, f: impl Fn(Vec2) -> Vec2) -> Self {
        Self::from_corners(self.corners().map(f))
    }

    /// Whether every corner is square.
    pub fn is_square(&self) -> bool {
        self.corners().iter().all(|c| *c == Vec2::ZERO)
    }

    /// The radii a `width` x `height` box actually draws: all of them scaled
    /// by `min(1, Lᵢ / Sᵢ)` over the four sides, where `Sᵢ` is the sum of the
    /// two radii along side `i` and `Lᵢ` its length (§5.5).
    pub fn fitted(&self, width: f64, height: f64) -> Self {
        let [tl, tr, br, bl] = self.corners();
        let w = width.max(0.0);
        let h = height.max(0.0);
        let f = [
            (w, tl.x + tr.x),
            (h, tr.y + br.y),
            (w, br.x + bl.x),
            (h, bl.y + tl.y),
        ]
        .into_iter()
        .filter(|&(_, sum)| sum > 0.0)
        .map(|(len, sum)| len / sum)
        .fold(1.0f64, f64::min);
        if f < 1.0 { self.map(|c| c * f) } else { *self }
    }

    /// The radii of the box inset by `top`/`right`/`bottom`/`left` — the
    /// padding box from the border box, given the border widths (§5.5,
    /// "Corner Shaping"): each radius less the width on its own axis, floored
    /// at zero. A corner that loses either radius is square.
    pub fn inner(&self, top: f64, right: f64, bottom: f64, left: f64) -> Self {
        let d = |r: f64, by: f64| (r - by).max(0.0);
        Self::new(
            Vec2::new(d(self.top_left.x, left), d(self.top_left.y, top)),
            Vec2::new(d(self.top_right.x, right), d(self.top_right.y, top)),
            Vec2::new(
                d(self.bottom_right.x, right),
                d(self.bottom_right.y, bottom),
            ),
            Vec2::new(d(self.bottom_left.x, left), d(self.bottom_left.y, bottom)),
        )
    }

    /// Every radius grown by `by` on both axes (an outline's offset); a
    /// square corner stays square.
    pub fn grown(&self, by: f64) -> Self {
        self.map(|c| {
            if c == Vec2::ZERO {
                c
            } else {
                Vec2::new((c.x + by).max(0.0), (c.y + by).max(0.0))
            }
        })
    }

    /// The corners as circles, when they are: `Some` only if every corner's
    /// two radii are equal.
    pub fn as_circular(&self) -> Option<RoundedRectRadii> {
        let [tl, tr, br, bl] = self.corners();
        let round = |c: Vec2| ((c.x - c.y).abs() <= 1e-9).then_some(c.x);
        Some(RoundedRectRadii::new(
            round(tl)?,
            round(tr)?,
            round(br)?,
            round(bl)?,
        ))
    }

    /// A circular stand-in for a consumer that can only draw circles (the
    /// compositor's corner mask, an inset shadow's blurred mask): each corner
    /// its smaller radius, which keeps the stand-in's shape inside the real
    /// one. Exact when the corners are circular.
    pub fn circular_within(&self) -> RoundedRectRadii {
        let m = |c: Vec2| c.x.min(c.y);
        RoundedRectRadii::new(
            m(self.top_left),
            m(self.top_right),
            m(self.bottom_right),
            m(self.bottom_left),
        )
    }

    /// The shape `rect` takes with these radii, fitted to it first
    /// ([`Self::fitted`]): a plain [`Rect`] when every corner is square, a
    /// kurbo [`RoundedRect`] when the corners are circular and fit in half
    /// the shorter side (what kurbo draws without clamping — every shape that
    /// was right before #977), and a [`BezPath`] of elliptical arcs
    /// otherwise.
    pub fn shape(&self, rect: Rect) -> PaintShape {
        let r = self.fitted(rect.width(), rect.height());
        if r.is_square() {
            return rect.into();
        }
        let half = rect.width().min(rect.height()) * 0.5;
        if let Some(c) = r.as_circular()
            && [c.top_left, c.top_right, c.bottom_right, c.bottom_left]
                .iter()
                .all(|&v| v <= half)
        {
            return RoundedRect::from_rect(rect, c).into();
        }
        r.path_unfitted(rect).into()
    }

    /// [`Self::shape`] as a path, for callers that combine it with others
    /// (a ring, a shape with holes).
    pub fn path(&self, rect: Rect) -> BezPath {
        match self.shape(rect) {
            PaintShape::Rect(r) => r.into_path(TOLERANCE),
            PaintShape::RoundedRect(r) => r.into_path(TOLERANCE),
            PaintShape::BezPath(p) => p,
            other => other.bounding_box().into_path(TOLERANCE),
        }
    }

    /// Whether `p` lies inside (or on) `rect` with these radii fitted to it.
    pub fn contains(&self, rect: Rect, p: Point) -> bool {
        if p.x < rect.x0 || p.x > rect.x1 || p.y < rect.y0 || p.y > rect.y1 {
            return false;
        }
        let r = self.fitted(rect.width(), rect.height());
        let corners = [
            (
                r.top_left,
                Point::new(rect.x0 + r.top_left.x, rect.y0 + r.top_left.y),
                -1.0,
                -1.0,
            ),
            (
                r.top_right,
                Point::new(rect.x1 - r.top_right.x, rect.y0 + r.top_right.y),
                1.0,
                -1.0,
            ),
            (
                r.bottom_right,
                Point::new(rect.x1 - r.bottom_right.x, rect.y1 - r.bottom_right.y),
                1.0,
                1.0,
            ),
            (
                r.bottom_left,
                Point::new(rect.x0 + r.bottom_left.x, rect.y1 - r.bottom_left.y),
                -1.0,
                1.0,
            ),
        ];
        corners.into_iter().all(|(c, centre, sx, sy)| {
            let (u, v) = (p.x - centre.x, p.y - centre.y);
            // Only the quadrant beyond the ellipse's centre is rounded off.
            c == Vec2::ZERO
                || u * sx <= 0.0
                || v * sy <= 0.0
                || (u / c.x).powi(2) + (v / c.y).powi(2) <= 1.0 + 1e-9
        })
    }

    /// [`Self::path`] less `holes`, for a `Fill::EvenOdd` fill: the
    /// background a `data-viewport` hole is cut out of, and an inset
    /// shadow's clip (#1030).
    ///
    /// Each hole is added only where it lies inside the shape. Under
    /// even-odd a hole contour reaching past the shape — the sliver between a
    /// rounded corner's arc and the hole's corner, or past `rect` altogether
    /// — counts once there, and that part was filled, outside the box.
    ///
    /// A hole clipped to `rect` that meets no rounded corner's box lies
    /// inside the shape and goes in as its rect. One that meets a corner's
    /// box is the shape flattened to a polygon and clipped to the hole (the
    /// shape is convex, so that is the exact intersection). The outer
    /// contour then has the corners a hole meets flattened the same way, so
    /// it and the hole share their vertices along those arcs; every other
    /// segment is the shape's own, so a corner no hole meets draws as it does
    /// with no hole. Each hole contour is wound against the outline, so the
    /// shared edges cancel under every anti-aliasing scheme — Vello's
    /// analytic area coverage (Android) included, which sums the two
    /// fractional coverages of same-way edges and drew a line along the arc.
    pub fn path_with_holes(&self, rect: Rect, holes: &[Rect]) -> BezPath {
        let r = self.fitted(rect.width(), rect.height());
        let Rect { x0, y0, x1, y1 } = rect;
        let [tl, tr, br, bl] = r.corners();
        let corner_boxes: Vec<Rect> = [
            (tl, Rect::new(x0, y0, x0 + tl.x, y0 + tl.y)),
            (tr, Rect::new(x1 - tr.x, y0, x1, y0 + tr.y)),
            (br, Rect::new(x1 - br.x, y1 - br.y, x1, y1)),
            (bl, Rect::new(x0, y1 - bl.y, x0 + bl.x, y1)),
        ]
        .into_iter()
        .filter(|(c, _)| *c != Vec2::ZERO)
        .map(|(_, b)| b)
        .collect();
        let overlaps =
            |h: &Rect, c: &Rect| h.x0 < c.x1 && h.x1 > c.x0 && h.y0 < c.y1 && h.y1 > c.y0;
        let at_corner = |h: &Rect| corner_boxes.iter().any(|c| overlaps(h, c));
        let holes: Vec<Rect> = holes
            .iter()
            .map(|h| h.intersect(rect))
            .filter(|h| h.width() > 0.0 && h.height() > 0.0)
            .collect();
        // The outline runs clockwise on screen (y down); a hole contour runs
        // the other way.
        let push_rect = |path: &mut BezPath, h: &Rect| {
            path.move_to((h.x0, h.y0));
            path.line_to((h.x0, h.y1));
            path.line_to((h.x1, h.y1));
            path.line_to((h.x1, h.y0));
            path.close_path();
        };
        let shape = self.path(rect);
        let touched: Vec<&Rect> = corner_boxes
            .iter()
            .filter(|c| holes.iter().any(|h| overlaps(h, c)))
            .collect();
        if touched.is_empty() {
            let mut path = shape;
            for h in &holes {
                push_rect(&mut path, h);
            }
            return path;
        }
        // Each segment of the shape flattened on its own, so a segment comes
        // out the same points in the outline and in the polygon a hole clips.
        let flat = |seg: PathSeg| {
            let mut pts = Vec::new();
            let mut one = BezPath::new();
            one.move_to(seg.start());
            one.push(seg.as_path_el());
            peniko::kurbo::flatten(one, HOLE_TOLERANCE, |el| {
                if let PathEl::LineTo(p) = el {
                    pts.push(p);
                }
            });
            pts
        };
        let mut polygon: Vec<Point> = Vec::new();
        let mut path = BezPath::new();
        for (i, seg) in shape.segments().enumerate() {
            if i == 0 {
                polygon.push(seg.start());
                path.move_to(seg.start());
            }
            let pts = flat(seg);
            if touched.iter().any(|c| overlaps(&seg.bounding_box(), c)) {
                for &p in &pts {
                    path.line_to(p);
                }
            } else {
                path.push(seg.as_path_el());
            }
            polygon.extend(pts);
        }
        path.close_path();
        if polygon.len() > 1 && polygon.first() == polygon.last() {
            polygon.pop();
        }
        for h in &holes {
            if at_corner(h) {
                let mut cut = clip_polygon(&polygon, *h);
                cut.reverse();
                push_polygon(&mut path, &cut);
            } else {
                push_rect(&mut path, h);
            }
        }
        path
    }

    /// The outline, clockwise from the end of the top-left corner, with each
    /// rounded corner an elliptical arc. The radii must already fit `rect`.
    fn path_unfitted(&self, rect: Rect) -> BezPath {
        let Rect { x0, y0, x1, y1 } = rect;
        let [tl, tr, br, bl] = self.corners();
        let mut path = BezPath::new();
        path.move_to((x0 + tl.x, y0));
        let corner = |path: &mut BezPath, r: Vec2, center: Point, start: f64, end: Point| {
            if r == Vec2::ZERO {
                path.line_to(end);
            } else {
                let arc = Arc::new(center, r, start, std::f64::consts::FRAC_PI_2, 0.0);
                path.line_to(arc.center + Vec2::new(r.x * start.cos(), r.y * start.sin()));
                path.extend(arc.append_iter(TOLERANCE));
            }
        };
        use std::f64::consts::{FRAC_PI_2, PI};
        corner(
            &mut path,
            tr,
            Point::new(x1 - tr.x, y0 + tr.y),
            -FRAC_PI_2,
            Point::new(x1, y0),
        );
        corner(
            &mut path,
            br,
            Point::new(x1 - br.x, y1 - br.y),
            0.0,
            Point::new(x1, y1),
        );
        corner(
            &mut path,
            bl,
            Point::new(x0 + bl.x, y1 - bl.y),
            FRAC_PI_2,
            Point::new(x0, y1),
        );
        corner(
            &mut path,
            tl,
            Point::new(x0 + tl.x, y0 + tl.y),
            PI,
            Point::new(x0, y0),
        );
        path.close_path();
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f64, y: f64) -> Vec2 {
        Vec2::new(x, y)
    }

    #[test]
    fn a_corner_square_on_one_axis_is_square() {
        let r = BorderRadii::new(v(10.0, 0.0), v(0.0, 5.0), v(3.0, 4.0), v(-1.0, 2.0));
        assert_eq!(r.top_left, Vec2::ZERO);
        assert_eq!(r.top_right, Vec2::ZERO);
        assert_eq!(r.bottom_right, v(3.0, 4.0));
        assert_eq!(r.bottom_left, Vec2::ZERO);
    }

    /// The smallest ratio wins and scales every corner, a corner on a side
    /// that fits included.
    #[test]
    fn fitting_scales_every_corner_by_the_smallest_ratio() {
        let r = BorderRadii::new(v(150.0, 30.0), v(100.0, 30.0), v(20.0, 40.0), v(20.0, 40.0))
            .fitted(200.0, 100.0);
        assert_eq!(r.top_left, v(120.0, 24.0));
        assert_eq!(r.bottom_right, v(16.0, 32.0));
        // Radii that fit are left alone.
        let fits = BorderRadii::uniform(30.0);
        assert_eq!(fits.fitted(200.0, 60.0), fits);
    }

    #[test]
    fn inner_radii_lose_the_border_on_their_own_axis() {
        let r = BorderRadii::uniform(10.0).inner(2.0, 4.0, 12.0, 6.0);
        assert_eq!(r.top_left, v(4.0, 8.0));
        assert_eq!(r.top_right, v(6.0, 8.0));
        // 10 - 12 is floored at 0, and a corner with a zero radius is square.
        assert_eq!(r.bottom_right, Vec2::ZERO);
    }

    #[test]
    fn a_fitting_circle_stays_a_kurbo_rounded_rect() {
        let rect = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert!(matches!(
            BorderRadii::uniform(20.0).shape(rect),
            PaintShape::RoundedRect(_)
        ));
        assert!(matches!(BorderRadii::ZERO.shape(rect), PaintShape::Rect(_)));
        assert!(matches!(
            BorderRadii::uniform(10.0)
                .map(|c| v(c.x * 2.0, c.y))
                .shape(rect),
            PaintShape::BezPath(_)
        ));
    }
}
