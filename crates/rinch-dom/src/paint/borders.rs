//! Border, outline, and box-shadow painting.

use peniko::Fill;
use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, BezPath, Cap, Point, Rect, RoundedRectRadii, Shape, Stroke, Vec2};

use super::blur::Blur1d;
use super::painter::{PaintShape, Painter};
use crate::computed_style::BorderStyleValue;
use crate::node::Node;

/// Paint a CSS box-shadow effect.
///
/// Paint per-side borders with style support (solid, dashed, dotted, double).
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_borders(
    painter: &mut dyn Painter,
    node: &Node,
    scale: f64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radii: RoundedRectRadii,
    transform: Affine,
) {
    let cs = &node.computed_style;

    let sides = [
        // (width, color, style) for each side
        (
            cs.border_top_width.to_px(),
            cs.border_top_color,
            cs.border_top_style,
        ),
        (
            cs.border_right_width.to_px(),
            cs.border_right_color,
            cs.border_right_style,
        ),
        (
            cs.border_bottom_width.to_px(),
            cs.border_bottom_color,
            cs.border_bottom_style,
        ),
        (
            cs.border_left_width.to_px(),
            cs.border_left_color,
            cs.border_left_style,
        ),
    ];

    // Fast path: if all sides have the same width, color, and style, use single stroke
    let all_same = sides
        .windows(2)
        .all(|pair| pair[0].0 == pair[1].0 && pair[0].1 == pair[1].1 && pair[0].2 == pair[1].2);

    if all_same {
        let (bw, color, style) = sides[0];
        if bw <= 0.0 || matches!(style, BorderStyleValue::None | BorderStyleValue::Hidden) {
            return;
        }
        if let Some(bc) = color {
            let stroke = make_border_stroke(bw as f64 * scale, style);
            let half = bw as f64 * scale * 0.5;
            let border_rect = Rect::new(x + half, y + half, x + w - half, y + h - half);
            let has_radius = radii.top_left > 0.0
                || radii.top_right > 0.0
                || radii.bottom_right > 0.0
                || radii.bottom_left > 0.0;

            if has_radius {
                let rrect = border_rect.to_rounded_rect(radii);
                painter.stroke_color(&stroke, transform, bc, &rrect.into());
            } else {
                painter.stroke_color(&stroke, transform, bc, &border_rect.into());
            }
        }
        return;
    }

    // Per-side rendering
    let top_w = sides[0].0 as f64 * scale;
    let right_w = sides[1].0 as f64 * scale;
    let bottom_w = sides[2].0 as f64 * scale;
    let left_w = sides[3].0 as f64 * scale;

    // When widths are uniform and border-radius is present, draw arc paths per side
    // instead of straight lines. This is needed for spinners (border-radius: 50%
    // with only one side colored).
    let has_radius = radii.top_left > 0.0
        || radii.top_right > 0.0
        || radii.bottom_right > 0.0
        || radii.bottom_left > 0.0;
    let widths_uniform = (top_w - right_w).abs() < 0.01
        && (top_w - bottom_w).abs() < 0.01
        && (top_w - left_w).abs() < 0.01;

    if widths_uniform && has_radius && top_w > 0.0 {
        paint_borders_arc_per_side(painter, &sides, scale, x, y, w, h, radii, top_w, transform);
        return;
    }

    // Fallback: straight lines per side (no border-radius)

    // Top border
    if top_w > 0.0
        && !matches!(
            sides[0].2,
            BorderStyleValue::None | BorderStyleValue::Hidden
        )
        && let Some(bc) = sides[0].1
    {
        let stroke = make_border_stroke(top_w, sides[0].2);
        let half = top_w * 0.5;
        let path = peniko::kurbo::Line::new((x, y + half), (x + w, y + half));
        painter.stroke_color(&stroke, transform, bc, &path.into());
    }

    // Right border
    if right_w > 0.0
        && !matches!(
            sides[1].2,
            BorderStyleValue::None | BorderStyleValue::Hidden
        )
        && let Some(bc) = sides[1].1
    {
        let stroke = make_border_stroke(right_w, sides[1].2);
        let half = right_w * 0.5;
        let path = peniko::kurbo::Line::new((x + w - half, y), (x + w - half, y + h));
        painter.stroke_color(&stroke, transform, bc, &path.into());
    }

    // Bottom border
    if bottom_w > 0.0
        && !matches!(
            sides[2].2,
            BorderStyleValue::None | BorderStyleValue::Hidden
        )
        && let Some(bc) = sides[2].1
    {
        let stroke = make_border_stroke(bottom_w, sides[2].2);
        let half = bottom_w * 0.5;
        let path = peniko::kurbo::Line::new((x, y + h - half), (x + w, y + h - half));
        painter.stroke_color(&stroke, transform, bc, &path.into());
    }

    // Left border
    if left_w > 0.0
        && !matches!(
            sides[3].2,
            BorderStyleValue::None | BorderStyleValue::Hidden
        )
        && let Some(bc) = sides[3].1
    {
        let stroke = make_border_stroke(left_w, sides[3].2);
        let half = left_w * 0.5;
        let path = peniko::kurbo::Line::new((x + half, y), (x + half, y + h));
        painter.stroke_color(&stroke, transform, bc, &path.into());
    }
}

/// Paint per-side borders as arcs along a rounded rect.
///
/// Each CSS side owns the straight segment plus half of each adjacent corner arc.
/// This correctly renders spinners (border-radius: 50% with only one side colored).
#[allow(clippy::too_many_arguments)]
fn paint_borders_arc_per_side(
    painter: &mut dyn Painter,
    sides: &[(f32, Option<AlphaColor<Srgb>>, BorderStyleValue); 4],
    _scale: f64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radii: RoundedRectRadii,
    bw: f64,
    transform: Affine,
) {
    let half = bw * 0.5;
    // Stroke centerline rect
    let ix = x + half;
    let iy = y + half;
    let iw = w - bw;
    let ih = h - bw;

    // Inset radii for stroke centerline (can't go negative)
    let tl = (radii.top_left - half).max(0.0);
    let tr = (radii.top_right - half).max(0.0);
    let br = (radii.bottom_right - half).max(0.0);
    let bl = (radii.bottom_left - half).max(0.0);

    // Build arc paths for each side
    let paths = build_per_side_arc_paths(ix, iy, iw, ih, tl, tr, br, bl);

    // sides: [top, right, bottom, left]
    for (i, path) in paths.iter().enumerate() {
        let (_, color, style) = sides[i];
        if matches!(style, BorderStyleValue::None | BorderStyleValue::Hidden) {
            continue;
        }
        if let Some(bc) = color {
            let stroke = make_border_stroke(bw, style);
            painter.stroke_color(&stroke, transform, bc, &path.clone().into());
        }
    }
}

/// Build BezPath for each side of a rounded rect.
///
/// Each side includes: second half of the preceding corner arc + straight segment
/// + first half of the following corner arc. Corner arcs are split at t=0.5
///   using De Casteljau subdivision.
#[allow(clippy::too_many_arguments)]
fn build_per_side_arc_paths(
    ix: f64,
    iy: f64,
    iw: f64,
    ih: f64,
    tl: f64,
    tr: f64,
    br: f64,
    bl: f64,
) -> [BezPath; 4] {
    // κ for quarter-circle cubic Bézier approximation
    const K: f64 = 0.5522847498;

    // Corner arc endpoints and control points.
    // TL: from (ix, iy+tl) to (ix+tl, iy)  [left→top, counterclockwise around corner]
    let tl_p0 = Point::new(ix, iy + tl);
    let tl_p1 = Point::new(ix, iy + tl - tl * K);
    let tl_p2 = Point::new(ix + tl - tl * K, iy);
    let tl_p3 = Point::new(ix + tl, iy);

    // TR: from (ix+iw-tr, iy) to (ix+iw, iy+tr)  [top→right]
    let tr_p0 = Point::new(ix + iw - tr, iy);
    let tr_p1 = Point::new(ix + iw - tr + tr * K, iy);
    let tr_p2 = Point::new(ix + iw, iy + tr - tr * K);
    let tr_p3 = Point::new(ix + iw, iy + tr);

    // BR: from (ix+iw, iy+ih-br) to (ix+iw-br, iy+ih)  [right→bottom]
    let br_p0 = Point::new(ix + iw, iy + ih - br);
    let br_p1 = Point::new(ix + iw, iy + ih - br + br * K);
    let br_p2 = Point::new(ix + iw - br + br * K, iy + ih);
    let br_p3 = Point::new(ix + iw - br, iy + ih);

    // BL: from (ix+bl, iy+ih) to (ix, iy+ih-bl)  [bottom→left]
    let bl_p0 = Point::new(ix + bl, iy + ih);
    let bl_p1 = Point::new(ix + bl - bl * K, iy + ih);
    let bl_p2 = Point::new(ix, iy + ih - bl + bl * K);
    let bl_p3 = Point::new(ix, iy + ih - bl);

    // Split each corner arc at t=0.5
    let (tl_first, tl_second) = split_cubic_half(tl_p0, tl_p1, tl_p2, tl_p3);
    let (tr_first, tr_second) = split_cubic_half(tr_p0, tr_p1, tr_p2, tr_p3);
    let (br_first, br_second) = split_cubic_half(br_p0, br_p1, br_p2, br_p3);
    let (bl_first, bl_second) = split_cubic_half(bl_p0, bl_p1, bl_p2, bl_p3);

    // Top: second half of TL arc + top straight + first half of TR arc
    let mut top = BezPath::new();
    top.move_to(tl_second.0);
    if tl > 0.01 {
        top.curve_to(tl_second.1, tl_second.2, tl_second.3);
    }
    top.line_to(tr_first.0);
    if tr > 0.01 {
        top.curve_to(tr_first.1, tr_first.2, tr_first.3);
    }

    // Right: second half of TR arc + right straight + first half of BR arc
    let mut right = BezPath::new();
    right.move_to(tr_second.0);
    if tr > 0.01 {
        right.curve_to(tr_second.1, tr_second.2, tr_second.3);
    }
    right.line_to(br_first.0);
    if br > 0.01 {
        right.curve_to(br_first.1, br_first.2, br_first.3);
    }

    // Bottom: second half of BR arc + bottom straight + first half of BL arc
    let mut bottom = BezPath::new();
    bottom.move_to(br_second.0);
    if br > 0.01 {
        bottom.curve_to(br_second.1, br_second.2, br_second.3);
    }
    bottom.line_to(bl_first.0);
    if bl > 0.01 {
        bottom.curve_to(bl_first.1, bl_first.2, bl_first.3);
    }

    // Left: second half of BL arc + left straight + first half of TL arc
    let mut left = BezPath::new();
    left.move_to(bl_second.0);
    if bl > 0.01 {
        left.curve_to(bl_second.1, bl_second.2, bl_second.3);
    }
    left.line_to(tl_first.0);
    if tl > 0.01 {
        left.curve_to(tl_first.1, tl_first.2, tl_first.3);
    }

    [top, right, bottom, left]
}

type CubicPoints = (Point, Point, Point, Point);

/// Split a cubic Bézier at t=0.5 using De Casteljau's algorithm.
fn split_cubic_half(p0: Point, p1: Point, p2: Point, p3: Point) -> (CubicPoints, CubicPoints) {
    let m01 = midpt(p0, p1);
    let m12 = midpt(p1, p2);
    let m23 = midpt(p2, p3);
    let m012 = midpt(m01, m12);
    let m123 = midpt(m12, m23);
    let mid = midpt(m012, m123);
    ((p0, m01, m012, mid), (mid, m123, m23, p3))
}

fn midpt(a: Point, b: Point) -> Point {
    Point::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5)
}

/// Create a Stroke with dash pattern based on border style.
pub(super) fn make_border_stroke(width: f64, style: BorderStyleValue) -> Stroke {
    match style {
        BorderStyleValue::Dashed => Stroke::new(width).with_dashes(0.0, [width * 3.0, width * 3.0]),
        BorderStyleValue::Dotted => Stroke::new(width)
            .with_dashes(0.0, [width, width])
            .with_caps(Cap::Round),
        BorderStyleValue::Double => {
            // For double, draw at 1/3 width (the caller draws two passes)
            // We approximate by drawing a single thinner stroke
            Stroke::new((width / 3.0).max(1.0))
        }
        _ => Stroke::new(width), // Solid and others
    }
}

/// Paint outline outside the box model.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_outline(
    painter: &mut dyn Painter,
    node: &Node,
    scale: f64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radii: RoundedRectRadii,
    transform: Affine,
) {
    let cs = &node.computed_style;
    let ow = cs.outline_width as f64 * scale;
    if ow <= 0.0
        || matches!(
            cs.outline_style,
            BorderStyleValue::None | BorderStyleValue::Hidden
        )
    {
        return;
    }

    let color = match cs.outline_color {
        Some(c) => c,
        None => return,
    };

    let offset = cs.outline_offset as f64 * scale;
    let half = ow * 0.5;
    let stroke = make_border_stroke(ow, cs.outline_style);

    // Outline is drawn outside the border box, offset by outline-offset
    let outline_rect = Rect::new(
        x - offset - half,
        y - offset - half,
        x + w + offset + half,
        y + h + offset + half,
    );

    let has_radius = radii.top_left > 0.0
        || radii.top_right > 0.0
        || radii.bottom_right > 0.0
        || radii.bottom_left > 0.0;

    if has_radius {
        let expand = offset + half;
        let outline_radii = RoundedRectRadii::new(
            radii.top_left + expand,
            radii.top_right + expand,
            radii.bottom_right + expand,
            radii.bottom_left + expand,
        );
        let rrect = outline_rect.to_rounded_rect(outline_radii);
        painter.stroke_color(&stroke, transform, color, &rrect.into());
    } else {
        painter.stroke_color(&stroke, transform, color, &outline_rect.into());
    }
}

/// The corner radius of an outer shadow's shape, for a border radius `r`
/// grown by `spread` (css-backgrounds-3 §7.1.1, "Shadow Shape, Spread, and
/// Knockout").
///
/// The radius grows with the spread, as the rect does, and a negative spread
/// shrinks it, floored at zero. But where `r` is less than the spread, the
/// spread is first multiplied by `1 + (r/spread - 1)^3` — so a square corner
/// (`r == 0`, a factor of `0`) stays square however far it is spread, and a
/// small radius grows continuously out of it instead of jumping to
/// `r + spread`. Chrome 153 paints exactly this (#351, measured in
/// `tests/box_shadow_spread_radius_tests.rs`).
pub(super) fn spread_corner_radius(r: f64, spread: f64) -> f64 {
    if spread > 0.0 && r < spread {
        let ratio = r / spread;
        r + spread * (1.0 + (ratio - 1.0).powi(3))
    } else {
        (r + spread).max(0.0)
    }
}

/// [`spread_corner_radius`] for all four corners.
fn spread_radii(radii: RoundedRectRadii, spread: f64) -> RoundedRectRadii {
    RoundedRectRadii::new(
        spread_corner_radius(radii.top_left, spread),
        spread_corner_radius(radii.top_right, spread),
        spread_corner_radius(radii.bottom_right, spread),
        spread_corner_radius(radii.bottom_left, spread),
    )
}

/// Paint CSS box-shadow from typed computed values.
/// Approximates blur by drawing expanded, semi-transparent rounded rects.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_box_shadow(
    painter: &mut dyn Painter,
    shadows: &[crate::computed_style::BoxShadowValue],
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    scale: f64,
    node: &crate::node::Node,
    transform: Affine,
) {
    // Get per-corner border-radius from computed style
    let radii = {
        let cs = &node.computed_style;
        let resolve_size = node.layout.width.min(node.layout.height);
        let tl = cs.border_radius_top_left.resolve(resolve_size).max(0.0) as f64 * scale;
        let tr = cs.border_radius_top_right.resolve(resolve_size).max(0.0) as f64 * scale;
        let br = cs.border_radius_bottom_right.resolve(resolve_size).max(0.0) as f64 * scale;
        let bl = cs.border_radius_bottom_left.resolve(resolve_size).max(0.0) as f64 * scale;
        RoundedRectRadii::new(tl, tr, br, bl)
    };
    let has_radius = radii.top_left > 0.0
        || radii.top_right > 0.0
        || radii.bottom_right > 0.0
        || radii.bottom_left > 0.0;

    // The element's own border box, which an outer shadow is never painted
    // inside of. Every layer below is drawn as this hole punched out of the
    // layer's expanded rect, with `Fill::EvenOdd` doing the punching.
    //
    // It is what CSS says — an outer `box-shadow` is clipped to the outside of
    // the border box — and it is also, on a software rasteriser, the difference
    // between a shadow costing what it looks like it costs and costing what the
    // element covers. A blurred shadow here is approximated by eight concentric
    // layers, and each of those was being filled across the *whole* element,
    // not just the few pixels of ring it contributes to. On the moto g stylus
    // 5G the bottom sheet's panel is 1080×1672 physical pixels, so its shadow
    // was eight fills of 1.8 megapixels each — about 60ms a frame, every frame
    // the sheet was on screen, to darken pixels that the panel's own opaque
    // background then painted over. Punching the hole drops those eight fills
    // to the ring itself, and leaves the ring's own pixels alone: one is
    // outside the hole and inside exactly the same set of layers as before.
    //
    // It is not quite a no-op on screen, and the one place it shows is worth
    // knowing about. Along the element's own anti-aliased edge — the rounded
    // corners of a FAB, say — a pixel is partly inside the border box and
    // partly outside it, so it used to be blended over shadow that had been
    // painted underneath the element and is now blended over whatever is
    // actually behind. Measured on this app's library screen, that is 238
    // pixels of a 491×1065 capture, none of them differing by more than 11 of
    // 255 in any channel, all of them within the 76×76 box the FAB and its
    // shadow occupy. The new pixels are the correct ones: an outer shadow is
    // painted outside the border box, and what shows through an element's
    // anti-aliased edge should be the page, not a shadow the element covers.
    // See card K24.
    let element_box = Rect::new(x, y, x + w, y + h);
    let hole: BezPath = if has_radius {
        element_box.to_rounded_rect(radii).into_path(0.1)
    } else {
        element_box.into_path(0.1)
    };

    /// The layer's expanded shape with the element's border box cut out of it.
    ///
    /// `Fill::EvenOdd` over the two subpaths is what does the cutting: a point
    /// inside both is crossed an even number of times and so is left alone,
    /// which is the ring, and only the ring.
    fn ring(outer: BezPath, hole: &BezPath) -> BezPath {
        let mut path = outer;
        path.extend(hole.iter());
        path
    }

    /// Whether the hole is wholly inside `outer` — the precondition the punch
    /// needs, and not a rounding detail.
    ///
    /// `Fill::EvenOdd` counts crossings, so a point inside the hole and
    /// *outside* the layer is crossed once and fills. Where the border box
    /// escapes the layer's rect the punch therefore stops subtracting and
    /// starts adding, painting shadow inside the element that CSS says is not
    /// there. It escapes whenever a shadow is offset further than the layer is
    /// expanded — the near edge of `0 4px 12px`, say, whose first layers are
    /// expanded by less than the 4px they are pushed down — and the strip it
    /// paints only stays hidden while the element's own background is fully
    /// opaque. So the punch is applied only where it provably subtracts, and
    /// the layer is filled whole (as it always was) where it does not.
    fn hole_fits(
        hole_rect: Rect,
        hole_radii: RoundedRectRadii,
        outer_rect: Rect,
        outer_radii: RoundedRectRadii,
    ) -> bool {
        if hole_rect.x0 < outer_rect.x0
            || hole_rect.y0 < outer_rect.y0
            || hole_rect.x1 > outer_rect.x1
            || hole_rect.y1 > outer_rect.y1
        {
            return false;
        }
        // Per corner: the hole's arc has to sit inside the layer's. Their
        // centres are `d` apart, so the layer's radius has to beat the hole's
        // by at least that much. A square layer corner (`or == 0`) cuts
        // nothing off, so there the rect check above is the whole story.
        //
        // Nor does a layer corner the hole never reaches. The layer's arc
        // only cuts the quadrant beyond its own centre — for the top-left,
        // `x < oc.0 && y < oc.1` — and a hole whose rect starts at or past
        // that centre on either axis has no point there at all. That is the
        // shape a small radius under a large spread takes (#351): the spread
        // ratio rule makes the shadow's corner *sharper* than `r + spread`, so
        // its arc's centre sits outside the element and the arc-in-arc test
        // below, which assumes the two arcs face each other, fails a hole that
        // is plainly inside.
        //
        // `inward` is the corner's direction into the box: `(1, 1)` for the
        // top-left, `(-1, 1)` for the top-right, and so on.
        let corner = |inward: (f64, f64), hc: (f64, f64), hr: f64, oc: (f64, f64), or: f64| {
            // The hole's rect edges at this corner.
            let edge = (hc.0 - inward.0 * hr, hc.1 - inward.1 * hr);
            or <= 0.0
                || (edge.0 - oc.0) * inward.0 >= 0.0
                || (edge.1 - oc.1) * inward.1 >= 0.0
                || (hc.0 - oc.0).hypot(hc.1 - oc.1) + hr <= or + 1e-6
        };
        corner(
            (1.0, 1.0),
            (
                hole_rect.x0 + hole_radii.top_left,
                hole_rect.y0 + hole_radii.top_left,
            ),
            hole_radii.top_left,
            (
                outer_rect.x0 + outer_radii.top_left,
                outer_rect.y0 + outer_radii.top_left,
            ),
            outer_radii.top_left,
        ) && corner(
            (-1.0, 1.0),
            (
                hole_rect.x1 - hole_radii.top_right,
                hole_rect.y0 + hole_radii.top_right,
            ),
            hole_radii.top_right,
            (
                outer_rect.x1 - outer_radii.top_right,
                outer_rect.y0 + outer_radii.top_right,
            ),
            outer_radii.top_right,
        ) && corner(
            (-1.0, -1.0),
            (
                hole_rect.x1 - hole_radii.bottom_right,
                hole_rect.y1 - hole_radii.bottom_right,
            ),
            hole_radii.bottom_right,
            (
                outer_rect.x1 - outer_radii.bottom_right,
                outer_rect.y1 - outer_radii.bottom_right,
            ),
            outer_radii.bottom_right,
        ) && corner(
            (1.0, -1.0),
            (
                hole_rect.x0 + hole_radii.bottom_left,
                hole_rect.y1 - hole_radii.bottom_left,
            ),
            hole_radii.bottom_left,
            (
                outer_rect.x0 + outer_radii.bottom_left,
                outer_rect.y1 - outer_radii.bottom_left,
            ),
            outer_radii.bottom_left,
        )
    }

    const SQUARE: RoundedRectRadii = RoundedRectRadii {
        top_left: 0.0,
        top_right: 0.0,
        bottom_right: 0.0,
        bottom_left: 0.0,
    };

    for shadow in shadows {
        // Inset shadows are painted above the background, by
        // `paint_inset_box_shadow`.
        if shadow.inset {
            continue;
        }

        let offset_x = shadow.offset_x as f64 * scale;
        let offset_y = shadow.offset_y as f64 * scale;
        let blur = shadow.blur_radius as f64 * scale;
        let spread = shadow.spread_radius as f64 * scale;
        let spread_shape_radii = spread_radii(radii, spread);
        // `peniko::Color` *is* `AlphaColor<Srgb>`, so the `to_rgba8` round trip
        // this used to do was a re-snap of a value already in the painter's
        // colour space. It was **redundant, not lossy**: a shadow colour
        // reaches `ComputedStyle` through `color_from_absolute`, which has
        // already rounded every channel to 8 bits, so no input exists that the
        // round trip could change — including the `color-mix()` and percentage
        // `rgb()` cases you would expect to be the counterexamples. Removed so
        // there is one fewer quantiser to keep in step, not to recover
        // precision.
        let color: AlphaColor<Srgb> = shadow
            .color
            .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 40));

        if blur > 0.0 {
            // Approximate Gaussian box-shadow blur with concentric layers.
            // The max expansion is empirically tuned to match Chrome's visible
            // shadow extent. Using Gaussian-weighted alpha per layer with the
            // actual shadow color (not hardcoded black).
            let max_expand = blur * 0.5 + spread;
            let layers: usize = 8;

            let base_alpha = color.components[3] as f64;

            for i in 0..layers {
                let t = (i as f64 + 1.0) / layers as f64;
                let layer_expand = max_expand * t;
                let layer_rect = Rect::new(
                    x + offset_x - layer_expand,
                    y + offset_y - layer_expand,
                    x + w + offset_x + layer_expand,
                    y + h + offset_y + layer_expand,
                );
                let alpha_scale = (1.0 - t * 0.7) / layers as f64;
                // Skip a layer only when its alpha *rounds* to zero, i.e. when
                // it would genuinely tint nothing.
                //
                // This is the most visible line in the change, not a cost
                // saving. The threshold used to be `(… * 255.0) as u8 == 0`,
                // which truncates, so it discarded every layer under a *whole*
                // level rather than under half of one — and `alpha_scale` peaks
                // at 0.114, so for a faint shadow that is every layer there is.
                // `box-shadow: 0 0 40px rgba(0,0,0,0.03)` and anything fainter
                // painted **nothing at all**: not a dim shadow, an absent one.
                //
                // (`alpha_scale` peaking at 0.114 also means the product can
                // never exceed 1, so the old `.min(255.0)` clamp was dead.)
                if base_alpha * alpha_scale * 255.0 < 0.5 {
                    continue;
                }
                let layer_color = color.multiply_alpha(alpha_scale as f32);
                let (outer_radii, outer) = if has_radius {
                    // The spread shape's radius, then grown (or shrunk, for
                    // the inner layers) by how far this layer's blur takes it
                    // past the spread. Where `r >= spread` this is the
                    // `r + layer_expand` it always was; it differs only where
                    // the spread ratio rule sharpens a small corner (#351).
                    let grow = |r: f64| (r + layer_expand - spread).max(0.0);
                    let expanded_radii = RoundedRectRadii::new(
                        grow(spread_shape_radii.top_left),
                        grow(spread_shape_radii.top_right),
                        grow(spread_shape_radii.bottom_right),
                        grow(spread_shape_radii.bottom_left),
                    );
                    (
                        expanded_radii,
                        layer_rect.to_rounded_rect(expanded_radii).into_path(0.1),
                    )
                } else {
                    (SQUARE, layer_rect.into_path(0.1))
                };
                if hole_fits(element_box, radii, layer_rect, outer_radii) {
                    painter.fill_color(
                        Fill::EvenOdd,
                        transform,
                        layer_color,
                        &ring(outer, &hole).into(),
                    );
                } else {
                    painter.fill_color(Fill::NonZero, transform, layer_color, &outer.into());
                }
            }
        } else {
            // No blur: simple offset shadow
            let total_expand = spread;
            let shadow_rect = Rect::new(
                x + offset_x - total_expand,
                y + offset_y - total_expand,
                x + w + offset_x + total_expand,
                y + h + offset_y + total_expand,
            );
            let (outer_radii, outer) = if has_radius {
                (
                    spread_shape_radii,
                    shadow_rect
                        .to_rounded_rect(spread_shape_radii)
                        .into_path(0.1),
                )
            } else {
                (SQUARE, shadow_rect.into_path(0.1))
            };
            if hole_fits(element_box, radii, shadow_rect, outer_radii) {
                painter.fill_color(Fill::EvenOdd, transform, color, &ring(outer, &hole).into());
            } else {
                painter.fill_color(Fill::NonZero, transform, color, &outer.into());
            }
        }
    }
}

/// Paint the `inset` shadows of `shadows` (#974). Called after the element's
/// background and before its border, which is where css-backgrounds-3 §7.1
/// puts an inner shadow; the outer ones are `paint_box_shadow`'s.
///
/// An inner shadow is drawn inside the **padding** box, as if everything
/// outside the padding edge were opaque: the shadow is the padding box minus
/// a hole, the hole being the padding box moved by the offset and shrunk by
/// the spread, its radii the padding box's shrunk by the spread (the inset
/// half of §7.1.1 — a negative spread grows them by the ratio rule outer
/// shadows use). First shadow on top.
///
/// Each shadow reaches each pixel **once**, through **one** clip — the padding box's
/// rounded shape, less any `viewport_holes` (a `data-viewport` hole shows the
/// layer beneath, so the shadow is cut out of it exactly as the background
/// is). Once matters on the software painter: tiny-skia applies the clip
/// mask per draw, so a stack of translucent fills through one rounded clip
/// multiplies the clip's edge coverage into itself and leaves a dark rim
/// along the curve (review of #1014, F1), where Vello composites the clip
/// once.
///
/// - **Unblurred**: the ring is a vector fill (`EvenOdd`, hole cut out).
/// - **Blurred**: the shadow is drawn as images whose alpha is the colour's
///   alpha times one minus the hole's coverage blurred by a Gaussian of
///   `sigma = blur / 2` — the definition in §7.1 — so corners and rounded
///   holes come out as in Chrome 153 (`tests/box_shadow_inset_tests.rs`).
///   See [`blurred_inset_images`] for how, and for what it costs. A blur as
///   wide as the box or wider comes out darker than Chrome's (the centre of
///   `inset 0 0 200px` on a 100px box: alpha 0.87 here, 0.74 in Chrome 153).
///
/// Nothing is drawn outside the border box, so no ink reach, layer bound or
/// damage rect needs to know about an inset shadow (`own_ink_outsets` and
/// `layer_bounds` skip them).
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_inset_box_shadow(
    painter: &mut dyn Painter,
    shadows: &[crate::computed_style::BoxShadowValue],
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    scale: f64,
    radii: RoundedRectRadii,
    node: &Node,
    transform: Affine,
    viewport_holes: &[Rect],
    perf: &crate::perf::PerfCounters,
) {
    if !shadows.iter().any(|s| s.inset) {
        return;
    }
    let cs = &node.computed_style;
    let width = |w: f32, style: BorderStyleValue| {
        if matches!(style, BorderStyleValue::None | BorderStyleValue::Hidden) {
            0.0
        } else {
            w.max(0.0) as f64 * scale
        }
    };
    let bt = width(cs.border_top_width.to_px(), cs.border_top_style);
    let br = width(cs.border_right_width.to_px(), cs.border_right_style);
    let bb = width(cs.border_bottom_width.to_px(), cs.border_bottom_style);
    let bl = width(cs.border_left_width.to_px(), cs.border_left_style);
    let pad = Rect::new(x + bl, y + bt, x + w - br, y + h - bb);
    if pad.width() <= 0.0 || pad.height() <= 0.0 {
        return;
    }
    // The padding box's radii: the border radius less the adjacent borders.
    // CSS makes a corner elliptical when its two borders differ; kurbo's radii
    // are circular, so the corner takes the larger border, which keeps the
    // padding shape inside the border shape.
    let pad_radii = RoundedRectRadii::new(
        (radii.top_left - bt.max(bl)).max(0.0),
        (radii.top_right - bt.max(br)).max(0.0),
        (radii.bottom_right - bb.max(br)).max(0.0),
        (radii.bottom_left - bb.max(bl)).max(0.0),
    );
    let rounded = pad_radii.top_left > 0.0
        || pad_radii.top_right > 0.0
        || pad_radii.bottom_right > 0.0
        || pad_radii.bottom_left > 0.0;
    let (clip, clip_fill): (PaintShape, Fill) = if viewport_holes.is_empty() {
        let shape = if rounded {
            pad.to_rounded_rect(pad_radii).into()
        } else {
            pad.into()
        };
        (shape, Fill::NonZero)
    } else {
        let mut path: BezPath = if rounded {
            pad.to_rounded_rect(pad_radii).into_path(0.1)
        } else {
            pad.into_path(0.1)
        };
        // A hole only subtracts where it overlaps the padding box; the part
        // outside would count as *inside* under `EvenOdd`.
        for hole in viewport_holes {
            let hole = hole.intersect(pad);
            if hole.width() > 0.0 && hole.height() > 0.0 {
                path.extend(hole.into_path(0.1).iter());
            }
        }
        (path.into(), Fill::EvenOdd)
    };

    // The first shadow is on top, so paint back to front.
    for shadow in shadows.iter().rev().filter(|s| s.inset) {
        let offset_x = shadow.offset_x as f64 * scale;
        let offset_y = shadow.offset_y as f64 * scale;
        let blur = shadow.blur_radius.max(0.0) as f64 * scale;
        let spread = shadow.spread_radius as f64 * scale;
        let color: AlphaColor<Srgb> = shadow
            .color
            .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 40));
        if color.components[3] <= 0.0 {
            continue;
        }
        let hole_radii = RoundedRectRadii::new(
            spread_corner_radius(pad_radii.top_left, -spread),
            spread_corner_radius(pad_radii.top_right, -spread),
            spread_corner_radius(pad_radii.bottom_right, -spread),
            spread_corner_radius(pad_radii.bottom_left, -spread),
        );
        let hole = (pad + Vec2::new(offset_x, offset_y)).inset(-spread);
        let hole_empty = hole.width() <= 0.0 || hole.height() <= 0.0;

        if blur > 0.0 && !hole_empty {
            let images =
                blurred_inset_images(pad, hole, hole_radii, blur * 0.5, color, transform, perf);
            if !images.is_empty() {
                painter.push_clip(clip_fill, transform, &clip);
                for (origin, image) in &images {
                    painter.draw_image(
                        &super::painter::PaintImage {
                            data: &image.data,
                            width: image.width,
                            height: image.height,
                            decoded: None,
                            opaque: false,
                        },
                        transform * Affine::translate(*origin),
                    );
                }
                painter.pop_layer();
            }
            continue;
        }

        painter.push_clip(clip_fill, transform, &clip);
        if hole_empty {
            // No hole left: the whole padding box is shadow.
            painter.fill_color(
                Fill::NonZero,
                transform,
                color,
                &pad.inflate(1.0, 1.0).into(),
            );
        } else {
            let hole_path: BezPath = if rounded {
                hole.to_rounded_rect(hole_radii).into_path(0.1)
            } else {
                hole.into_path(0.1)
            };
            // The padding box (and the hole, wherever the offset took it)
            // with the hole cut out by `EvenOdd`; the clip trims it to the
            // padding shape. The outer rect contains the hole, so the
            // even-odd count only ever subtracts.
            let mut path = pad.union(hole).inflate(1.0, 1.0).into_path(0.1);
            path.extend(hole_path.iter());
            painter.fill_color(Fill::EvenOdd, transform, color, &path.into());
        }
        painter.pop_layer();
    }
}

/// A straight-alpha RGBA8 image to draw.
struct ShadowImage {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

/// The largest blurred inset shadow built, in image pixels. Past it the
/// shadow is not drawn. The image is cropped to what can be seen first, so
/// only a box this large painted with no known visible rect (`paint_subtree`
/// into a pixmap of its own) or under a singular transform reaches it.
const MAX_INSET_IMAGE_PIXELS: usize = 16 * 1024 * 1024;

/// Past this `sigma` a 1-D blur is three box blurs rather than a sampled
/// Gaussian kernel: the kernel costs `6 sigma` per sample, the boxes a
/// constant, and above it the boxes are within a level of the Gaussian.
const DIRECT_KERNEL_MAX_SIGMA: f64 = 2.5;

/// The blurred inset shadow over the padding box `pad` (device pixels), as
/// images and the device-space origins to draw them at; empty when nothing of
/// it can be seen.
///
/// The shadow's alpha is the colour's alpha times one minus the hole's
/// coverage blurred by a Gaussian of `sigma`. The hole is a rounded rect, and
/// the blur is computed as the rect's (which is separable: the product of two
/// 1-D blurs, so O(width + height)) less, at each rounded corner, the blur of
/// the sliver the rounding takes off the rect (a 2-D blur, but only over the
/// corner's own square grown by the blur's reach). So the cost is the image
/// itself plus four small corner patches, whatever the blur.
fn blurred_inset_images(
    pad: Rect,
    hole: Rect,
    hole_radii: RoundedRectRadii,
    sigma: f64,
    color: AlphaColor<Srgb>,
    transform: Affine,
    perf: &crate::perf::PerfCounters,
) -> Vec<(Vec2, ShadowImage)> {
    // The image covers `pad` on whole device pixels, cropped to what can be
    // seen: the render target, every clip the painter has open, and the
    // damage of a partial repaint, pulled back through the box's transform
    // (its bounding box, so a rotated crop keeps every pixel the rotated image
    // can land on). A pixel's value does not depend on the crop — each is
    // computed from lines grown by the blur's reach — so the crop changes no
    // pixel, only how many are computed. Without the damage, a caret blink in
    // a large panel rebuilt the panel's whole mask every frame (review of
    // #1014, round 2).
    let mut area = Rect::new(pad.x0.floor(), pad.y0.floor(), pad.x1.ceil(), pad.y1.ceil());
    if transform.determinant().abs() > 1e-9 {
        if let Some(visible) = super::visible_paint_rect() {
            let visible = transform.inverse().transform_rect_bbox(visible);
            area = area.intersect(Rect::new(
                visible.x0.floor(),
                visible.y0.floor(),
                visible.x1.ceil(),
                visible.y1.ceil(),
            ));
        }
    }
    if area.width() <= 0.0 || area.height() <= 0.0 {
        return Vec::new();
    }
    let (iw, ih) = (area.width() as usize, area.height() as usize);
    if iw.saturating_mul(ih) > MAX_INSET_IMAGE_PIXELS {
        return Vec::new();
    }
    perf.add(crate::perf::Counter::InsetShadowMaskPx, (iw * ih) as u64);
    let blur = Blur1d::new(sigma, DIRECT_KERNEL_MAX_SIGMA);
    let reach = blur.reach();
    let mut tmp = Vec::new();

    // The rect's blurred coverage, one axis at a time, over the image grown
    // by the reach (what lies further out cannot reach the image).
    let axis = |lo: f64, h0: f64, h1: f64, n: usize| -> Vec<f32> {
        let mut line: Vec<f32> = (0..n + 2 * reach)
            .map(|i| {
                let p = lo - reach as f64 + i as f64;
                ((p + 1.0).min(h1) - p.max(h0)).max(0.0) as f32
            })
            .collect();
        blur.apply(&mut line, &mut Vec::new());
        line.drain(..reach);
        line.truncate(n);
        line
    };
    let bx = axis(area.x0, hole.x0, hole.x1, iw);
    let by = axis(area.y0, hole.y0, hole.y1, ih);
    let mut covered = vec![0.0f32; iw * ih];
    for (j, row) in covered.chunks_exact_mut(iw).enumerate() {
        for (v, &x) in row.iter_mut().zip(&bx) {
            *v = x * by[j];
        }
    }

    // Less each rounded corner's sliver, blurred.
    let limit = (hole.width().min(hole.height()) * 0.5).max(0.0);
    let corners = [
        (hole_radii.top_left, hole.x0, hole.y0, 1.0, 1.0),
        (hole_radii.top_right, hole.x1, hole.y0, -1.0, 1.0),
        (hole_radii.bottom_right, hole.x1, hole.y1, -1.0, -1.0),
        (hole_radii.bottom_left, hole.x0, hole.y1, 1.0, -1.0),
    ];
    for (r, hx, hy, sx, sy) in corners {
        let r = r.min(limit);
        if r <= 0.0 {
            continue;
        }
        // The corner's square, and the arc's centre.
        let sq = Rect::new(hx, hy, hx + sx * r, hy + sy * r).abs();
        let (ccx, ccy) = (hx + sx * r, hy + sy * r);
        // The patch: the square on whole pixels, grown by the reach, kept to
        // the image grown by the reach.
        let px0 = (sq.x0.floor() as i64 - reach as i64).max(area.x0 as i64 - reach as i64);
        let py0 = (sq.y0.floor() as i64 - reach as i64).max(area.y0 as i64 - reach as i64);
        let px1 = (sq.x1.ceil() as i64 + reach as i64).min(area.x1 as i64 + reach as i64);
        let py1 = (sq.y1.ceil() as i64 + reach as i64).min(area.y1 as i64 + reach as i64);
        if px1 <= px0 || py1 <= py0 {
            continue;
        }
        let (pw, ph) = ((px1 - px0) as usize, (py1 - py0) as usize);
        // The sliver: the part of the pixel inside the rect's corner square
        // and outside the arc, approximated at the pixel centre with one
        // pixel of anti-aliasing across the arc.
        let mut patch = vec![0.0f32; pw * ph];
        for j in 0..ph {
            let y = (py0 + j as i64) as f64;
            let oy = ((y + 1.0).min(sq.y1) - y.max(sq.y0)).max(0.0);
            if oy <= 0.0 {
                continue;
            }
            for i in 0..pw {
                let x = (px0 + i as i64) as f64;
                let ox = ((x + 1.0).min(sq.x1) - x.max(sq.x0)).max(0.0);
                if ox <= 0.0 {
                    continue;
                }
                let d = (x + 0.5 - ccx).hypot(y + 0.5 - ccy) - r;
                let outside_arc = (0.5 + d).clamp(0.0, 1.0);
                patch[j * pw + i] = (ox * oy * outside_arc) as f32;
            }
        }
        // Only the rows the sliver is on have anything to blur across.
        for row in patch.chunks_exact_mut(pw) {
            if row.iter().any(|&v| v != 0.0) {
                blur.apply(row, &mut tmp);
            }
        }
        let mut col = vec![0.0f32; ph];
        for i in 0..pw {
            for j in 0..ph {
                col[j] = patch[j * pw + i];
            }
            blur.apply(&mut col, &mut tmp);
            for j in 0..ph {
                patch[j * pw + i] = col[j];
            }
        }
        // Subtract where the patch meets the image.
        let ax0 = area.x0 as i64;
        let ay0 = area.y0 as i64;
        for j in 0..ph {
            let iy = py0 + j as i64 - ay0;
            if iy < 0 || iy >= ih as i64 {
                continue;
            }
            for i in 0..pw {
                let ix = px0 + i as i64 - ax0;
                if ix < 0 || ix >= iw as i64 {
                    continue;
                }
                covered[iy as usize * iw + ix as usize] -= patch[j * pw + i];
            }
        }
    }

    let alpha = color.components[3];
    let alpha_at = |m: f32| (alpha * (1.0 - m.clamp(0.0, 1.0)) * 255.0).round() as u8;
    // Only the pixels with some shadow are drawn: a blurred inset shadow is a
    // ring, often only a band along one edge, and on the software painter a
    // drawn pixel costs a pipeline stage whether it is transparent or not. So
    // the ring is cut into up to four images — the rows above and below the
    // clear middle, and the columns either side of it — which never overlap,
    // so each pixel is still drawn through the clip once.
    let clear_row = |j: usize, cols: std::ops::Range<usize>| {
        cols.into_iter().all(|i| alpha_at(covered[j * iw + i]) == 0)
    };
    // The clear middle: the columns and rows where the rect's blur leaves no
    // shadow, shrunk until every pixel in it is clear (the corners' slivers
    // can reach into it).
    let first = |v: &[f32]| v.iter().position(|&m| alpha_at(m) == 0);
    let last = |v: &[f32]| v.iter().rposition(|&m| alpha_at(m) == 0).map(|i| i + 1);
    let mut regions: Vec<(usize, usize, usize, usize)> = Vec::new();
    match (first(&bx), last(&bx), first(&by), last(&by)) {
        (Some(xl), Some(xr), Some(mut yt), Some(mut yb)) if xl < xr && yt < yb => {
            while yt < yb && !clear_row(yt, xl..xr) {
                yt += 1;
            }
            while yb > yt && !clear_row(yb - 1, xl..xr) {
                yb -= 1;
            }
            if yt < yb {
                regions.push((0, 0, iw, yt));
                regions.push((0, yb, iw, ih));
                regions.push((0, yt, xl, yb));
                regions.push((xr, yt, iw, yb));
            } else {
                regions.push((0, 0, iw, ih));
            }
        }
        _ => regions.push((0, 0, iw, ih)),
    }

    let rgba = color.to_rgba8();
    let mut out = Vec::new();
    for (x0, y0, x1, y1) in regions {
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let (w, h) = (x1 - x0, y1 - y0);
        let mut data = vec![0u8; w * h * 4];
        for (j, row) in data.chunks_exact_mut(w * 4).enumerate() {
            let src = &covered[(y0 + j) * iw + x0..(y0 + j) * iw + x1];
            for (px, &m) in row.as_chunks_mut::<4>().0.iter_mut().zip(src) {
                px[0] = rgba.r;
                px[1] = rgba.g;
                px[2] = rgba.b;
                px[3] = alpha_at(m);
            }
        }
        out.push((
            Vec2::new(area.x0 + x0 as f64, area.y0 + y0 as f64),
            ShadowImage {
                data,
                width: w as u32,
                height: h as u32,
            },
        ));
    }
    out
}
