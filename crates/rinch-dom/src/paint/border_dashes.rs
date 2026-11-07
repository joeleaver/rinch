//! `dashed` and `dotted` borders and outlines, laid out the way Chrome lays
//! them out (#2371, #2372).
//!
//! This is a port of Blink's geometry. The dash and gap lengths are
//! `DashEffectFromStrokeStyle` and `SelectBestDashGap` in
//! `platform/graphics/styled_stroke_data.cc`. A straight side is
//! `DrawLineWithStyle` and `EnforceDotsAtEndpoints` in
//! `core/paint/box_border_painter.cc`, and a rounded border is
//! `DrawCurvedDashedDottedBoxSide` there. Everything is in device pixels, as
//! Blink's is.
//!
//! - **Dashed:** a dash is `3w` long and a gap `2w` when the width `w` is
//!   under 3 device px, and `2w` and `w` from 3 px up. A straight side is an
//!   open line from one outer corner to the other. Its gap is stretched or
//!   shrunk (`SelectBestDashGap`) so that whole dashes start and end the
//!   side, which makes every corner solid. A side too short for two dashes
//!   and a gap gets two dashes scaled to fit, and one too short for two
//!   dashes is solid.
//! - **Dotted, `w <= 3`:** square dots `w` long with `w` gaps, from the
//!   side's start. The end dots are made whole by drawing the first and last
//!   dots on their own and moving the dashed run between them, by the
//!   length's remainder modulo `2w` (`EnforceDotsAtEndpoints`).
//! - **Dotted, `w > 3`:** round dots of diameter `w`, spaced by the same
//!   best-fit gap, with the first and last dot touching the side's ends.
//! - **Corners:** two sides of the same style and colour overlap in the
//!   corner square, and both put a whole dash or dot there. Where the colour
//!   or the style changes, a side is cut on the diagonal from the outer
//!   corner to the inner one, as Chrome's corner comes out (its miter
//!   clip). The caller paints dashed and dotted sides after the others, so
//!   the cut side lands on top of a solid neighbour's full corner square.
//! - **Rounded:** the whole centre line is one closed path, starting at the
//!   left end of the top edge and running clockwise (measured against Chrome;
//!   see [`centre_line`]). The dashes are fitted to its perimeter, `n` dashes and `n` gaps. A thin
//!   pattern is stroked `2.2 * w` wide and clipped to the border's ring; round
//!   dots are stroked at `w`.
//!
//! No dash pattern reaches a painter. The straight sides are filled
//! rectangles and circles, and the rounded path is dashed here (with
//! `kurbo::dash`) and stroked plain. So tiny-skia never draws a dash as a
//! hairline (a 1px dotted border used to come out as a near-solid line,
//! #2371), and both painters draw the same geometry.

use peniko::Fill;
use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{
    Affine, Arc, BezPath, Cap, Circle, Join, ParamCurve, ParamCurveArclen, PathEl, PathSeg, Point,
    Rect, Shape, Stroke, Vec2,
};

use super::painter::Painter;
use super::radii::BorderRadii;
use crate::computed_style::BorderStyleValue;

/// Tolerance for flattening circles and arcs, in device px.
const TOLERANCE: f64 = 0.05;

/// The two styles laid out here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Dash {
    Dashed,
    Dotted,
}

impl Dash {
    pub(super) fn of(style: BorderStyleValue) -> Option<Self> {
        match style {
            BorderStyleValue::Dashed => Some(Dash::Dashed),
            BorderStyleValue::Dotted => Some(Dash::Dotted),
            _ => None,
        }
    }

    /// Blink's `StrokeIsDashed`: drawn as butt-capped dashes, so square
    /// dots for a thin dotted line, rather than as round dots.
    fn is_dashed(self, width: f64) -> bool {
        self == Dash::Dashed || width <= 3.0
    }
}

/// A dash pattern: `on`, then `off`, repeated from the path's start. `round`
/// means zero-length dashes with round caps, which draw dots.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Pattern {
    pub(super) on: f64,
    pub(super) off: f64,
    pub(super) round: bool,
}

/// Blink's `SelectBestDashGap`: the gap, near `gap`, that fits a whole
/// number of `dash`es into `length`. An open path starts and ends with a
/// dash (`n` dashes, `n - 1` gaps), and a closed one has as many gaps as
/// dashes.
fn best_gap(length: f64, dash: f64, gap: f64, closed: bool) -> f64 {
    let available = if closed { length } else { length + gap };
    let min_dashes = (available / (dash + gap)).floor();
    let max_dashes = min_dashes + 1.0;
    let (min_gaps, max_gaps) = if closed {
        (min_dashes, max_dashes)
    } else {
        (min_dashes - 1.0, max_dashes - 1.0)
    };
    let min_gap = (length - min_dashes * dash) / min_gaps;
    let max_gap = (length - max_dashes * dash) / max_gaps;
    if max_gap <= 0.0 || (min_gap - gap).abs() < (max_gap - gap).abs() {
        min_gap
    } else {
        max_gap
    }
}

/// Blink's `DashEffectFromStrokeStyle`: the pattern for a path `length` long
/// of a border `width` wide, or `None` for a solid line.
pub(super) fn pattern(dash: Dash, length: f64, width: f64, closed: bool) -> Option<Pattern> {
    if dash.is_dashed(width) {
        let (mut on, mut off) = (width, width);
        if dash == Dash::Dashed {
            let (on_ratio, off_ratio) = if width >= 3.0 { (2.0, 1.0) } else { (3.0, 2.0) };
            on *= on_ratio;
            off *= off_ratio;
        }
        if length <= on * 2.0 {
            // No room for dashes.
            return None;
        }
        let mut two = 2.0 * on + off;
        if closed {
            two += off;
        }
        if length <= two {
            // Exactly two dashes, sized in proportion.
            let k = length / two;
            return Some(Pattern {
                on: on * k,
                off: off * k,
                round: false,
            });
        }
        if dash == Dash::Dashed {
            off = best_gap(length, on, off, closed);
        }
        return Some(Pattern {
            on,
            off,
            round: false,
        });
    }
    let per_dot = width * 2.0;
    if length < per_dot {
        // Room for one dot only: a gap longer than the line.
        return Some(Pattern {
            on: 0.0,
            off: per_dot,
            round: true,
        });
    }
    // Blink's epsilon keeps the last dot from falling off the end to rounding.
    const EPSILON: f64 = 1.0e-2;
    let gap = best_gap(length, width, width, closed);
    Some(Pattern {
        on: 0.0,
        off: gap + width - EPSILON,
        round: true,
    })
}

/// The marks along one straight run, in its own coordinate from its start.
#[derive(Default, Debug, PartialEq)]
pub(super) struct Marks {
    /// Butt-capped spans `[from, to)`, the full width of the band.
    pub(super) spans: Vec<(f64, f64)>,
    /// Round dots, by their centre.
    pub(super) dots: Vec<f64>,
}

/// The marks of a straight side `length` long and `width` thick
/// (`DrawLineWithStyle`).
pub(super) fn straight_marks(dash: Dash, length: f64, width: f64) -> Marks {
    let mut marks = Marks::default();
    if length <= 0.0 || width <= 0.0 {
        return marks;
    }
    let pattern = pattern(dash, length, width, false);
    let (mut p1, mut p2) = (0.0, length);
    if dash == Dash::Dotted {
        if dash.is_dashed(width) {
            enforce_dots_at_endpoints(&mut marks, &mut p1, &mut p2, length, width);
        } else {
            // Round dots: the caps reach half a width past each end.
            p1 += width / 2.0;
            p2 -= width / 2.0;
        }
    }
    let Some(pattern) = pattern else {
        marks.spans.push((p1, p2));
        return marks;
    };
    let period = pattern.on + pattern.off;
    let mut at = p1;
    if pattern.round {
        // Skia draws a zero-length dash at the line's start even when the
        // line is shorter than nothing (a dot wider than its side).
        loop {
            marks.dots.push(at);
            at += period;
            if at > p2 + 1e-9 {
                break;
            }
        }
        return marks;
    }
    while at < p2 - 1e-9 {
        marks.spans.push((at, (at + pattern.on).min(p2)));
        at += period;
    }
    marks
}

/// Blink's `EnforceDotsAtEndpoints`, for square dots: draw the first and
/// last dots on their own, grown or moved by a pixel where the length does
/// not divide evenly, and move the run between them. Only defined for whole
/// pixel widths 1 to 3 and a whole length, as Blink's are; anything else is
/// left to the plain pattern.
fn enforce_dots_at_endpoints(marks: &mut Marks, p1: &mut f64, p2: &mut f64, length: f64, w: f64) {
    let wi = w.round();
    let li = length.round();
    if (w - wi).abs() > 1e-3 || (length - li).abs() > 1e-3 || !(1.0..=3.0).contains(&wi) {
        return;
    }
    let (width, path_length) = (wi as i64, li as i64);
    let mod_4 = path_length % 4;
    let mod_6 = path_length % 6;
    let mut use_start_dot = false;
    let mut start_dot_growth = 0;
    let mut start_line_offset = 0;
    let mut use_end_dot = false;
    let mut end_dot_growth = 0;
    if (width == 1 && path_length % 2 == 0) || (width == 3 && mod_6 == 0) {
        use_start_dot = true;
        start_dot_growth = 1;
        start_line_offset = 1;
    }
    if (width == 2 && (mod_4 == 0 || mod_4 == 1)) || (width == 3 && (mod_6 == 1 || mod_6 == 2)) {
        use_start_dot = true;
        start_line_offset = -1;
    }
    if (width == 2 && mod_4 == 0) || (width == 3 && mod_6 == 1) {
        use_end_dot = true;
    }
    if (width == 2 && mod_4 == 3) || (width == 3 && (mod_6 == 4 || mod_6 == 5)) {
        use_start_dot = true;
        start_line_offset = 1;
    }
    if width == 3 && mod_6 == 5 {
        use_end_dot = true;
    } else if width == 3 && mod_6 == 0 {
        use_end_dot = true;
        end_dot_growth = 1;
    }
    if use_start_dot {
        marks
            .spans
            .push((*p1, *p1 + (width + start_dot_growth) as f64));
        *p1 += (2 * width + start_line_offset) as f64;
    }
    if use_end_dot {
        marks
            .spans
            .push((*p2 - (width + end_dot_growth) as f64, *p2));
        *p2 -= (width + end_dot_growth + 1) as f64;
    }
}

/// Which side of a box.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

/// A half-plane a side is cut to at a mitred corner: points `p` with
/// `(p - origin) x dir` of the same sign as `keep`'s.
#[derive(Clone, Copy, Debug)]
pub(super) struct Cut {
    origin: Point,
    dir: Vec2,
    sign: f64,
}

impl Cut {
    fn new(origin: Point, through: Point, keep: Point) -> Self {
        let dir = through - origin;
        let sign = dir.cross(keep - origin).signum();
        Cut { origin, dir, sign }
    }

    fn side(&self, p: Point) -> f64 {
        self.dir.cross(p - self.origin) * self.sign
    }
}

/// The border box and the four widths, in device px.
#[derive(Clone, Copy, Debug)]
pub(super) struct BoxEdges {
    pub(super) rect: Rect,
    /// Top, right, bottom, left.
    pub(super) widths: [f64; 4],
}

impl BoxEdges {
    fn width(&self, side: Side) -> f64 {
        self.widths[side as usize]
    }

    /// The cut at the corner between `side` and `adjacent`: the line from
    /// the outer corner to the inner one, keeping `side`'s part.
    pub(super) fn miter(&self, side: Side, adjacent: Side) -> Cut {
        let r = self.rect;
        let [t, rt, b, l] = self.widths;
        let (outer, inner) = match (side, adjacent) {
            (Side::Top, Side::Left) | (Side::Left, Side::Top) => {
                (Point::new(r.x0, r.y0), Point::new(r.x0 + l, r.y0 + t))
            }
            (Side::Top, Side::Right) | (Side::Right, Side::Top) => {
                (Point::new(r.x1, r.y0), Point::new(r.x1 - rt, r.y0 + t))
            }
            (Side::Bottom, Side::Right) | (Side::Right, Side::Bottom) => {
                (Point::new(r.x1, r.y1), Point::new(r.x1 - rt, r.y1 - b))
            }
            _ => (Point::new(r.x0, r.y1), Point::new(r.x0 + l, r.y1 - b)),
        };
        let keep = match side {
            Side::Top => Point::new((r.x0 + r.x1) / 2.0, r.y0),
            Side::Bottom => Point::new((r.x0 + r.x1) / 2.0, r.y1),
            Side::Left => Point::new(r.x0, (r.y0 + r.y1) / 2.0),
            Side::Right => Point::new(r.x1, (r.y0 + r.y1) / 2.0),
        };
        Cut::new(outer, inner, keep)
    }
}

/// Append one straight side's marks to `out`, cut by `cuts`.
pub(super) fn straight_side(
    out: &mut BezPath,
    edges: &BoxEdges,
    side: Side,
    dash: Dash,
    cuts: &[Cut],
) {
    let r = edges.rect;
    let w = edges.width(side);
    if w <= 0.0 {
        return;
    }
    let horizontal = matches!(side, Side::Top | Side::Bottom);
    let (start, length) = if horizontal {
        (r.x0, r.width())
    } else {
        (r.y0, r.height())
    };
    // The band across the side.
    let (b0, b1) = match side {
        Side::Top => (r.y0, r.y0 + w),
        Side::Bottom => (r.y1 - w, r.y1),
        Side::Left => (r.x0, r.x0 + w),
        Side::Right => (r.x1 - w, r.x1),
    };
    let at = |along: f64, across: f64| {
        if horizontal {
            Point::new(start + along, across)
        } else {
            Point::new(across, start + along)
        }
    };
    let marks = straight_marks(dash, length, w);
    for (a, b) in marks.spans {
        let poly = [at(a, b0), at(b, b0), at(b, b1), at(a, b1)];
        push_polygon(out, &poly, cuts);
    }
    for c in marks.dots {
        // Flattened, so it is wound like the rects and cut like them.
        let circle = Circle::new(at(c, (b0 + b1) / 2.0), w / 2.0);
        let mut flat = Vec::new();
        peniko::kurbo::flatten(circle.path_elements(TOLERANCE), TOLERANCE, |el| {
            if let PathEl::MoveTo(p) | PathEl::LineTo(p) = el {
                flat.push(p);
            }
        });
        push_polygon(out, &flat, cuts);
    }
}

/// Append `poly` cut by every half-plane in `cuts` (Sutherland-Hodgman), as a
/// closed subpath, wound clockwise (on screen) whichever way it was built, so
/// a non-zero fill of them all is their union.
fn push_polygon(out: &mut BezPath, poly: &[Point], cuts: &[Cut]) {
    let mut pts: Vec<Point> = poly.to_vec();
    for cut in cuts {
        if pts.is_empty() {
            break;
        }
        let mut next = Vec::with_capacity(pts.len() + 2);
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            let (sa, sb) = (cut.side(a), cut.side(b));
            if sa >= 0.0 {
                next.push(a);
            }
            if (sa >= 0.0) != (sb >= 0.0) {
                let t = sa / (sa - sb);
                next.push(a.lerp(b, t));
            }
        }
        pts = next;
    }
    if pts.len() < 3 {
        return;
    }
    let area: f64 = (0..pts.len())
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    if area.abs() < 1e-9 {
        return;
    }
    if area < 0.0 {
        pts.reverse();
    }
    out.move_to(pts[0]);
    for &p in &pts[1..] {
        out.line_to(p);
    }
    out.close_path();
}

/// A uniform dashed or dotted border on a square box: the four sides, which
/// overlap in the corners, as one non-zero fill.
pub(super) fn paint_square_uniform(
    painter: &mut dyn Painter,
    transform: Affine,
    color: AlphaColor<Srgb>,
    rect: Rect,
    width: f64,
    dash: Dash,
) {
    let edges = BoxEdges {
        rect,
        widths: [width; 4],
    };
    let mut path = BezPath::new();
    for side in [Side::Top, Side::Right, Side::Bottom, Side::Left] {
        straight_side(&mut path, &edges, side, dash, &[]);
    }
    if !path.is_empty() {
        painter.fill_color(Fill::NonZero, transform, color, &path.into());
    }
}

/// The closed centre line of a rounded border: `rect` is the centre line's
/// box, `radii` its corners (fitted here). It starts at the left end of the
/// top edge and runs clockwise, which is where Chrome 156's dashes start:
/// of the eight places a rounded rect's path can start, in either direction,
/// only this one (and its mirror image, the right end of the bottom edge run
/// anticlockwise, which lays the same dashes on a symmetric box) reproduces
/// Chrome's dashes on both a `1px` and a `2px` rounded border. It is built
/// here, not taken from a kurbo `RoundedRect`, so the dashes do not depend on
/// where a shape's path happens to start.
pub(super) fn centre_line(rect: Rect, radii: BorderRadii) -> BezPath {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = radii.fitted(rect.width(), rect.height());
    let Rect { x0, y0, x1, y1 } = rect;
    let mut path = BezPath::new();
    path.move_to((x0 + r.top_left.x, y0));
    let corner = |path: &mut BezPath, rad: Vec2, center: Point, start: f64| {
        if rad.x > 0.0 && rad.y > 0.0 {
            let arc = Arc::new(center, rad, start, FRAC_PI_2, 0.0);
            path.line_to(arc.center + Vec2::new(rad.x * start.cos(), rad.y * start.sin()));
            path.extend(arc.append_iter(TOLERANCE));
        } else {
            path.line_to(center);
        }
    };
    corner(
        &mut path,
        r.top_right,
        Point::new(x1 - r.top_right.x, y0 + r.top_right.y),
        -FRAC_PI_2,
    );
    corner(
        &mut path,
        r.bottom_right,
        Point::new(x1 - r.bottom_right.x, y1 - r.bottom_right.y),
        0.0,
    );
    corner(
        &mut path,
        r.bottom_left,
        Point::new(x0 + r.bottom_left.x, y1 - r.bottom_left.y),
        FRAC_PI_2,
    );
    corner(
        &mut path,
        r.top_left,
        Point::new(x0 + r.top_left.x, y0 + r.top_left.y),
        PI,
    );
    path.close_path();
    path
}

/// The length of a path.
fn path_length(path: &BezPath) -> f64 {
    path.segments().map(|s| s.arclen(1e-3)).sum()
}

/// The points at which a closed `path` puts its round dots, every `period`
/// along it from its start.
fn dots_along(path: &BezPath, period: f64, length: f64) -> Vec<Point> {
    let mut out = Vec::new();
    let mut next = 0.0;
    let mut walked = 0.0;
    for seg in path.segments() {
        let len = seg.arclen(1e-3);
        while next <= walked + len + 1e-9 && next <= length + 1e-9 {
            let t = if len > 0.0 {
                seg_param(&seg, next - walked)
            } else {
                0.0
            };
            out.push(seg.eval(t));
            next += period;
        }
        walked += len;
    }
    // A closed path's end is its start: a dot there would sit on the first.
    if let (Some(first), Some(last)) = (out.first(), out.last())
        && out.len() > 1
        && (*first - *last).hypot() < 1e-6
    {
        out.pop();
    }
    out
}

fn seg_param(seg: &PathSeg, at: f64) -> f64 {
    seg.inv_arclen(at.max(0.0), 1e-4).clamp(0.0, 1.0)
}

/// A rounded dashed or dotted border (`DrawCurvedDashedDottedBoxSide`):
/// the closed centre line dashed, clipped to `ring`, or to `ring` and
/// `side_clip` for one side of a border whose sides differ.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_rounded(
    painter: &mut dyn Painter,
    transform: Affine,
    color: AlphaColor<Srgb>,
    centre: &BezPath,
    ring: &BezPath,
    side_clip: Option<&BezPath>,
    width: f64,
    stroke_width: f64,
    dash: Dash,
) {
    // Blink takes the length as a whole number of pixels.
    let length = path_length(centre).floor();
    let pattern = pattern(dash, length, width, true);
    painter.push_clip(Fill::NonZero, transform, &ring.clone().into());
    if let Some(clip) = side_clip {
        painter.push_clip(Fill::NonZero, transform, &clip.clone().into());
    }
    match pattern {
        Some(p) if p.round => {
            let mut dots = BezPath::new();
            for c in dots_along(centre, p.on + p.off, length) {
                let mut flat = Vec::new();
                peniko::kurbo::flatten(
                    Circle::new(c, width / 2.0).path_elements(TOLERANCE),
                    TOLERANCE,
                    |el| {
                        if let PathEl::MoveTo(p) | PathEl::LineTo(p) = el {
                            flat.push(p);
                        }
                    },
                );
                push_polygon(&mut dots, &flat, &[]);
            }
            if !dots.is_empty() {
                painter.fill_color(Fill::NonZero, transform, color, &dots.into());
            }
        }
        Some(p) => {
            let dashes: BezPath = peniko::kurbo::dash(centre.iter(), 0.0, &[p.on, p.off]).collect();
            // The stroke is `2.2 w` wide and the ring clips it, so the
            // dashes' ends are clipped along with their sides.
            let stroke = Stroke::new(stroke_width * 2.2)
                .with_caps(Cap::Butt)
                .with_join(Join::Miter);
            painter.stroke_color(&stroke, transform, color, &dashes.into());
        }
        None => {
            let stroke = Stroke::new(stroke_width * 2.2).with_join(Join::Miter);
            painter.stroke_color(&stroke, transform, color, &centre.clone().into());
        }
    }
    if side_clip.is_some() {
        painter.pop_layer();
    }
    painter.pop_layer();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(dash: Dash, length: f64, width: f64) -> Vec<(f64, f64)> {
        straight_marks(dash, length, width).spans
    }

    #[test]
    fn a_1px_dotted_side_of_even_length_doubles_its_first_dot() {
        // Chrome 156 on a 40px side: `##.#.#…#.#`.
        let s = spans(Dash::Dotted, 40.0, 1.0);
        assert_eq!(s[0], (0.0, 2.0));
        assert_eq!(s[1], (3.0, 4.0));
        assert_eq!(*s.last().unwrap(), (39.0, 40.0));
        assert_eq!(s.len(), 1 + 19);
    }

    #[test]
    fn a_1px_dotted_side_of_odd_length_alternates() {
        let s = spans(Dash::Dotted, 61.0, 1.0);
        assert_eq!(s.len(), 31);
        assert!(
            s.iter()
                .enumerate()
                .all(|(i, &(a, b))| a == 2.0 * i as f64 && b == a + 1.0)
        );
    }

    #[test]
    fn dashes_start_and_end_a_side() {
        for (len, w) in [
            (60.0, 1.0),
            (61.0, 2.0),
            (60.0, 3.0),
            (60.0, 5.0),
            (31.0, 3.0),
        ] {
            let s = spans(Dash::Dashed, len, w);
            assert_eq!(s[0].0, 0.0, "{len} {w}");
            assert!((s.last().unwrap().1 - len).abs() < 1e-9, "{len} {w}: {s:?}");
            let on = if w >= 3.0 { 2.0 * w } else { 3.0 * w };
            assert!(
                s.iter().all(|&(a, b)| ((b - a) - on).abs() < 1e-9),
                "{len} {w}: {s:?}"
            );
        }
    }

    #[test]
    fn round_dots_touch_both_ends() {
        let m = straight_marks(Dash::Dotted, 60.0, 5.0);
        assert!(m.spans.is_empty());
        assert_eq!(m.dots[0], 2.5);
        // Blink's epsilon shortens each period by 0.01, so the last of seven
        // dots is 0.06 short of the end, as Chrome's is.
        assert!((m.dots.last().unwrap() - 57.5).abs() < 0.1, "{:?}", m.dots);
    }
}
