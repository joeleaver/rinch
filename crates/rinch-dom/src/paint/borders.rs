//! Border, outline, and box-shadow painting.

use peniko::Fill;
use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, BezPath, Cap, Join, Point, Rect, Shape, Stroke, Vec2};

use super::blur::Blur1d;
use super::painter::{PaintShape, Painter};
use super::radii::BorderRadii;
use crate::computed_style::BorderStyleValue;
use crate::node::Node;

/// Shading math for the four "3D" border styles (`inset`/`outset`/`groove`/
/// `ridge`, #731).
///
/// This is Chromium's own `CalculateInsetOutsetColor` /
/// `Color::Light`/`Color::Dark` (the `TableDefaultBorderColorCurrentColorEnabled`
/// path, which has shipped at `status: "stable"` since before Chrome 153 —
/// read from `box_border_painter.cc` and `color.cc` directly rather than
/// re-derived, because the two colour functions are not CSS-spec text: they
/// are Chromium's own tuned constants). Measuring a 1px border against this
/// is unreliable (antialiasing blends the computed shade with whatever is
/// behind it — the #731 issue's own measured `(154, 238)` for a 1px `<hr>` is
/// such a blend of this module's `(44, 212)` with the white page behind it,
/// not a different formula), which is exactly why the fixtures in
/// `border_bevel_tests.rs` use a several-px-wide border.
mod bevel {
    use peniko::color::{AlphaColor, Srgb};

    /// Which of the four sides is being painted. `top_left()` is the one bit
    /// every formula below keys on: CSS always darkens top/left and lightens
    /// bottom/right for `inset` (the reverse for `outset`), regardless of box
    /// size or which side physically touches the content box.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) enum Side {
        Top,
        Right,
        Bottom,
        Left,
    }

    impl Side {
        fn top_left(self) -> bool {
            matches!(self, Side::Top | Side::Left)
        }
    }

    /// `nextafterf(256.0f, 0.0f)` — the largest `f32` strictly below 256,
    /// used by Chromium's `QuantizeTo8Bit` so that a component of exactly
    /// 1.0 quantizes to 255 rather than overflowing to 256.
    fn quantize_scale() -> f32 {
        f32::from_bits(256.0f32.to_bits() - 1)
    }

    /// Chromium's `QuantizeTo8Bit`: round a `[0, 1]` component to the nearest
    /// representable 8-bit sRGB value, expressed back as a `[0, 1]` float.
    fn quantize_to_8bit(v: f32) -> f32 {
        ((v * quantize_scale()) as i32) as f32 / 255.0
    }

    /// `color_utils::Linearize` (sRGB -> linear), the IEC 61966-2-1 transfer
    /// function with the 0.04045 breakpoint color_utils.cc uses (not the
    /// 0.03928 the W3C formula's text uses — see that file's own comment).
    fn linearize(c: f32) -> f32 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    /// `color_utils::GetRelativeLuminance4f`.
    fn relative_luminance(c: AlphaColor<Srgb>) -> f32 {
        let [r, g, b, _] = c.components;
        0.2126 * linearize(r) + 0.7152 * linearize(g) + 0.0722 * linearize(b)
    }

    /// `blink::Color::Light()`.
    fn light(c: AlphaColor<Srgb>) -> AlphaColor<Srgb> {
        let [r, g, b, a] = c.components;
        // Hardcoded fast path for opaque black, matching Chromium exactly.
        if r == 0.0 && g == 0.0 && b == 0.0 && a == 1.0 {
            return AlphaColor::new([84.0 / 255.0, 84.0 / 255.0, 84.0 / 255.0, 1.0]);
        }
        let v = r.max(g).max(b);
        if v == 0.0 {
            // Lightened black, alpha preserved.
            return AlphaColor::new([84.0 / 255.0, 84.0 / 255.0, 84.0 / 255.0, a]);
        }
        let multiplier = (v + 0.33).min(1.0) / v;
        AlphaColor::new([
            quantize_to_8bit(r * multiplier),
            quantize_to_8bit(g * multiplier),
            quantize_to_8bit(b * multiplier),
            a,
        ])
    }

    /// `blink::Color::Dark()`.
    fn dark(c: AlphaColor<Srgb>) -> AlphaColor<Srgb> {
        let [r, g, b, a] = c.components;
        if r == 1.0 && g == 1.0 && b == 1.0 && a == 1.0 {
            return AlphaColor::new([171.0 / 255.0, 171.0 / 255.0, 171.0 / 255.0, 1.0]);
        }
        let v = r.max(g).max(b);
        let multiplier = if v == 0.0 {
            0.0
        } else {
            (v - 0.33).max(0.0) / v
        };
        AlphaColor::new([
            quantize_to_8bit(r * multiplier),
            quantize_to_8bit(g * multiplier),
            quantize_to_8bit(b * multiplier),
            a,
        ])
    }

    /// Luminance of `rgb(32, 32, 32)` — very dark colours get extra contrast
    /// instead of darkening further into near-invisibility.
    const BASE_DARK_LUMINANCE: f32 = 0.014443844;
    /// Luminance of `rgb(235, 235, 235)` — very light colours are left alone
    /// on their light side rather than clipped to white.
    const BASE_LIGHT_LUMINANCE: f32 = 0.83077;

    /// `blink::CalculateInsetOutsetColor`. `is_darken` is this side/half's own
    /// `DarkenBoxSide` bit — true for the shadow side, false for the
    /// highlight side.
    fn inset_outset_color(is_darken: bool, color: AlphaColor<Srgb>) -> AlphaColor<Srgb> {
        let luminance = relative_luminance(color);
        if luminance <= BASE_DARK_LUMINANCE {
            return if is_darken {
                light(color)
            } else {
                light(light(color))
            };
        }
        if is_darken {
            return dark(color);
        }
        if luminance > BASE_LIGHT_LUMINANCE {
            color
        } else {
            light(color)
        }
    }

    /// The whole-edge colour for `border-style: inset` or `outset`.
    pub(super) fn inset_or_outset(
        inset: bool,
        side: Side,
        color: AlphaColor<Srgb>,
    ) -> AlphaColor<Srgb> {
        // `DarkenBoxSide`: top/left darkens under `inset`, and the opposite
        // sides darken under `outset`.
        let darken = side.top_left() == inset;
        inset_outset_color(darken, color)
    }

    /// The `(outer, inner)` half-colours for `border-style: groove` or
    /// `ridge`, where "outer" is the half nearer the box's own outer edge
    /// and "inner" is the half nearer the padding/content box.
    ///
    /// `groove`'s outer half takes `inset`'s shading and its inner half
    /// takes `outset`'s (the reverse for `ridge`) — Chromium's
    /// `DrawRidgeOrGrooveBoxSide` does this by literally recursing into the
    /// solid-edge painter with `style` swapped to `Inset`/`Outset` per half;
    /// this is that recursion collapsed to the two colours it produces.
    pub(super) fn groove_or_ridge(
        groove: bool,
        side: Side,
        color: AlphaColor<Srgb>,
    ) -> (AlphaColor<Srgb>, AlphaColor<Srgb>) {
        let outer = inset_or_outset(groove, side, color);
        let inner = inset_or_outset(!groove, side, color);
        (outer, inner)
    }
}

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
    radii: BorderRadii,
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

    // Fast path: if all sides have the same width, color, and style, use single stroke.
    // Excluded for the four "3D" styles even when every side's (width, color,
    // style) tuple is literally equal: the PAINTED colour still differs per
    // side (and per half, for groove/ridge), so a single stroke in one colour
    // would be #731 all over again.
    let all_same = sides
        .windows(2)
        .all(|pair| pair[0].0 == pair[1].0 && pair[0].1 == pair[1].1 && pair[0].2 == pair[1].2)
        && !matches!(
            sides[0].2,
            BorderStyleValue::Inset
                | BorderStyleValue::Outset
                | BorderStyleValue::Groove
                | BorderStyleValue::Ridge
        );

    if all_same {
        let (bw, color, style) = sides[0];
        if bw <= 0.0 || matches!(style, BorderStyleValue::None | BorderStyleValue::Hidden) {
            return;
        }
        if let Some(bc) = color {
            let bw = bw as f64 * scale;
            let stroke = make_border_stroke(bw, style);
            let half = bw * 0.5;
            let border_rect = Rect::new(x + half, y + half, x + w - half, y + h - half);

            if radii.is_square() {
                painter.stroke_color(&stroke, transform, bc, &border_rect.into());
            } else if matches!(style, BorderStyleValue::Solid) {
                // A solid rounded border is exactly the ring between the
                // border box's shape and the padding box's (css-backgrounds-3
                // §5.5, #977), elliptical corners included — one even-odd
                // fill, which a stroke of an ellipse cannot draw (an offset
                // ellipse is no ellipse). Non-zero, the hole wound against
                // the outline (`punched`).
                let outer = Rect::new(x, y, x + w, y + h);
                let mut ring = radii.path(outer);
                let inner = outer.inset(-bw);
                if inner.width() > 0.0 && inner.height() > 0.0 {
                    ring = punched(ring, &radii.inner(bw, bw, bw, bw).path(inner));
                }
                painter.fill_color(Fill::NonZero, transform, bc, &ring.into());
            } else {
                // The dashed, dotted and double strokes run along the
                // border's centre line, whose radii are the outer ones less
                // half the width — the stroke's outer edge then has the
                // border box's radius. (It used to stroke the centre line
                // with the outer radii, half a width too round, #977.)
                let centre = radii.inner(half, half, half, half);
                painter.stroke_color(&stroke, transform, bc, &centre.shape(border_rect));
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
    let has_radius = !radii.is_square();
    let widths_uniform = (top_w - right_w).abs() < 0.01
        && (top_w - bottom_w).abs() < 0.01
        && (top_w - left_w).abs() < 0.01;

    // groove/ridge split each edge into two differently-coloured halves along
    // its own thickness, which the arc path below has no way to express (it
    // strokes one whole-edge colour per side) — so a radius combined with
    // groove/ridge falls through to the straight-line path instead, which
    // loses the rounding at the corners but keeps the two-tone split. That is
    // a narrower version of the pre-existing limitation just above (per-side
    // rendering already ignores `border-radius` whenever widths aren't
    // uniform); not covered by this issue.
    let has_split_style = sides
        .iter()
        .any(|s| matches!(s.2, BorderStyleValue::Groove | BorderStyleValue::Ridge));

    if widths_uniform && has_radius && top_w > 0.0 && !has_split_style {
        paint_borders_arc_per_side(painter, &sides, scale, x, y, w, h, radii, top_w, transform);
        return;
    }

    // Fallback: straight lines per side (no border-radius)
    if let Some(bc) = sides[0].1 {
        paint_border_side(
            painter,
            bevel::Side::Top,
            top_w,
            sides[0].2,
            bc,
            x,
            y,
            w,
            h,
            transform,
        );
    }
    if let Some(bc) = sides[1].1 {
        paint_border_side(
            painter,
            bevel::Side::Right,
            right_w,
            sides[1].2,
            bc,
            x,
            y,
            w,
            h,
            transform,
        );
    }
    if let Some(bc) = sides[2].1 {
        paint_border_side(
            painter,
            bevel::Side::Bottom,
            bottom_w,
            sides[2].2,
            bc,
            x,
            y,
            w,
            h,
            transform,
        );
    }
    if let Some(bc) = sides[3].1 {
        paint_border_side(
            painter,
            bevel::Side::Left,
            left_w,
            sides[3].2,
            bc,
            x,
            y,
            w,
            h,
            transform,
        );
    }
}

/// The straight-line path of one border side at `(x, y, w, h)`, offset
/// `offset_from_outer` logical units in from that side's own outer edge
/// (toward the box's content), perpendicular to the edge.
fn side_perpendicular_line(
    side: bevel::Side,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    offset_from_outer: f64,
) -> peniko::kurbo::Line {
    use peniko::kurbo::Line;
    match side {
        bevel::Side::Top => Line::new((x, y + offset_from_outer), (x + w, y + offset_from_outer)),
        bevel::Side::Bottom => Line::new(
            (x, y + h - offset_from_outer),
            (x + w, y + h - offset_from_outer),
        ),
        bevel::Side::Left => Line::new((x + offset_from_outer, y), (x + offset_from_outer, y + h)),
        bevel::Side::Right => Line::new(
            (x + w - offset_from_outer, y),
            (x + w - offset_from_outer, y + h),
        ),
    }
}

/// Paint one straight (non-rounded) border side, splitting it into two
/// differently-shaded halves for `groove`/`ridge` and resolving the
/// whole-edge shade for `inset`/`outset` (#731). `width` is already scaled.
#[allow(clippy::too_many_arguments)]
fn paint_border_side(
    painter: &mut dyn Painter,
    side: bevel::Side,
    width: f64,
    style: BorderStyleValue,
    color: AlphaColor<Srgb>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    transform: Affine,
) {
    if width <= 0.0 || matches!(style, BorderStyleValue::None | BorderStyleValue::Hidden) {
        return;
    }
    // Chromium's `BorderEdge::EffectiveStyle`: a groove/ridge edge one
    // (device) pixel thick has no room for two visible halves and paints
    // solid, in its own undarkened colour.
    let style =
        if width <= 1.0 && matches!(style, BorderStyleValue::Groove | BorderStyleValue::Ridge) {
            BorderStyleValue::Solid
        } else {
            style
        };
    match style {
        BorderStyleValue::Groove | BorderStyleValue::Ridge => {
            let groove = matches!(style, BorderStyleValue::Groove);
            let (outer_color, inner_color) = bevel::groove_or_ridge(groove, side, color);
            // The outer half gets the extra pixel on an odd width, matching
            // Chromium's `(y1 + y2 + 1) / 2` midpoint rounding.
            let outer_w = (width * 0.5).ceil();
            let inner_w = width - outer_w;
            // `Stroke::new`'s default cap is `Round`, which bleeds a
            // half-width half-disc beyond each open line's endpoint. The
            // `all_same` fast path strokes a closed rect/rrect instead
            // (where "cap" doesn't apply), so this round bleed was never
            // visible for a uniform border until this per-side path — which
            // every bevel style now always takes (see the comment on
            // `all_same` above) — started drawing open line segments for
            // what used to be a single rect stroke. `Butt` reaches exactly
            // to each line's own endpoint, which is already the box's own
            // corner, so the perpendicular side fills in the rest with no
            // gap and no bleed, the same "reach exactly the corner" result
            // `make_border_stroke`'s doc comment already relies on `Miter`
            // for on a closed path.
            let outer_line = side_perpendicular_line(side, x, y, w, h, outer_w * 0.5);
            painter.stroke_color(
                &Stroke::new(outer_w)
                    .with_join(Join::Miter)
                    .with_caps(Cap::Butt),
                transform,
                outer_color,
                &outer_line.into(),
            );
            if inner_w > 0.0 {
                let inner_line = side_perpendicular_line(side, x, y, w, h, outer_w + inner_w * 0.5);
                painter.stroke_color(
                    &Stroke::new(inner_w)
                        .with_join(Join::Miter)
                        .with_caps(Cap::Butt),
                    transform,
                    inner_color,
                    &inner_line.into(),
                );
            }
        }
        BorderStyleValue::Inset | BorderStyleValue::Outset => {
            let resolved =
                bevel::inset_or_outset(matches!(style, BorderStyleValue::Inset), side, color);
            // `Butt`, not whatever `make_border_stroke` returns (`Round`):
            // see the long comment on the groove/ridge case above.
            let stroke = make_border_stroke(width, style).with_caps(Cap::Butt);
            let line = side_perpendicular_line(side, x, y, w, h, width * 0.5);
            painter.stroke_color(&stroke, transform, resolved, &line.into());
        }
        _ => {
            let stroke = make_border_stroke(width, style);
            let line = side_perpendicular_line(side, x, y, w, h, width * 0.5);
            painter.stroke_color(&stroke, transform, color, &line.into());
        }
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
    radii: BorderRadii,
    bw: f64,
    transform: Affine,
) {
    let half = bw * 0.5;
    // Stroke centerline rect
    let ix = x + half;
    let iy = y + half;
    let iw = w - bw;
    let ih = h - bw;

    // Inset radii for stroke centerline (can't go negative), per axis (#977).
    let centre = radii.inner(half, half, half, half);

    // Build arc paths for each side
    let paths = build_per_side_arc_paths(
        ix,
        iy,
        iw,
        ih,
        centre.top_left,
        centre.top_right,
        centre.bottom_right,
        centre.bottom_left,
    );

    // sides: [top, right, bottom, left]. The caller never routes a
    // groove/ridge side here (it falls back to the straight-line path, which
    // can express a two-tone split and this arc path cannot); inset/outset
    // resolve to their one whole-side shade.
    const SIDE_ORDER: [bevel::Side; 4] = [
        bevel::Side::Top,
        bevel::Side::Right,
        bevel::Side::Bottom,
        bevel::Side::Left,
    ];
    for (i, path) in paths.iter().enumerate() {
        let (_, color, style) = sides[i];
        if matches!(style, BorderStyleValue::None | BorderStyleValue::Hidden) {
            continue;
        }
        if let Some(bc) = color {
            let resolved = match style {
                BorderStyleValue::Inset | BorderStyleValue::Outset => bevel::inset_or_outset(
                    matches!(style, BorderStyleValue::Inset),
                    SIDE_ORDER[i],
                    bc,
                ),
                _ => bc,
            };
            let stroke = make_border_stroke(bw, style);
            painter.stroke_color(&stroke, transform, resolved, &path.clone().into());
        }
    }
}

/// Build BezPath for each side of a rounded rect.
///
/// Each side includes: second half of the preceding corner arc + straight segment
/// + first half of the following corner arc. Corner arcs are split at t=0.5
///   using De Casteljau subdivision. Each corner is a quarter ellipse with its
///   own horizontal (`x`) and vertical (`y`) radius (#977).
#[allow(clippy::too_many_arguments)]
fn build_per_side_arc_paths(
    ix: f64,
    iy: f64,
    iw: f64,
    ih: f64,
    tl: Vec2,
    tr: Vec2,
    br: Vec2,
    bl: Vec2,
) -> [BezPath; 4] {
    // κ for quarter-ellipse cubic Bézier approximation
    const K: f64 = 0.5522847498;

    // Corner arc endpoints and control points.
    // TL: from (ix, iy+tl.y) to (ix+tl.x, iy)  [left→top, counterclockwise around corner]
    let tl_p0 = Point::new(ix, iy + tl.y);
    let tl_p1 = Point::new(ix, iy + tl.y - tl.y * K);
    let tl_p2 = Point::new(ix + tl.x - tl.x * K, iy);
    let tl_p3 = Point::new(ix + tl.x, iy);

    // TR: from (ix+iw-tr.x, iy) to (ix+iw, iy+tr.y)  [top→right]
    let tr_p0 = Point::new(ix + iw - tr.x, iy);
    let tr_p1 = Point::new(ix + iw - tr.x + tr.x * K, iy);
    let tr_p2 = Point::new(ix + iw, iy + tr.y - tr.y * K);
    let tr_p3 = Point::new(ix + iw, iy + tr.y);

    // BR: from (ix+iw, iy+ih-br.y) to (ix+iw-br.x, iy+ih)  [right→bottom]
    let br_p0 = Point::new(ix + iw, iy + ih - br.y);
    let br_p1 = Point::new(ix + iw, iy + ih - br.y + br.y * K);
    let br_p2 = Point::new(ix + iw - br.x + br.x * K, iy + ih);
    let br_p3 = Point::new(ix + iw - br.x, iy + ih);

    // BL: from (ix+bl.x, iy+ih) to (ix, iy+ih-bl.y)  [bottom→left]
    let bl_p0 = Point::new(ix + bl.x, iy + ih);
    let bl_p1 = Point::new(ix + bl.x - bl.x * K, iy + ih);
    let bl_p2 = Point::new(ix, iy + ih - bl.y + bl.y * K);
    let bl_p3 = Point::new(ix, iy + ih - bl.y);
    // A corner is drawn when it is rounded at all (a square one is `ZERO`).
    let (tl, tr, br, bl) = (
        tl.x.max(tl.y),
        tr.x.max(tr.y),
        br.x.max(br.y),
        bl.x.max(bl.y),
    );

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
///
/// Every style but `dotted` takes a **miter** join. A uniform border or an
/// outline is one stroke around the rectangle through the middle of its band,
/// and kurbo's `Stroke::new` defaults to `Join::Round`, which leaves each
/// outer corner square of the band unpainted — a notch `width / 2` on a side,
/// enough to make a 50px border on a 100px box read as rounded (#1022). A 90°
/// miter reaches exactly the corner (ratio √2, under the default limit of 4),
/// so the band is the border box minus the padding box, as CSS draws it. On a
/// rounded rect the path is smooth at every joint and the join is moot.
/// `dotted` keeps its round join along with its round caps.
pub(super) fn make_border_stroke(width: f64, style: BorderStyleValue) -> Stroke {
    match style {
        BorderStyleValue::Dashed => Stroke::new(width)
            .with_join(Join::Miter)
            .with_dashes(0.0, [width * 3.0, width * 3.0]),
        BorderStyleValue::Dotted => Stroke::new(width)
            .with_dashes(0.0, [width, width])
            .with_caps(Cap::Round),
        BorderStyleValue::Double => {
            // For double, draw at 1/3 width (the caller draws two passes)
            // We approximate by drawing a single thinner stroke
            Stroke::new((width / 3.0).max(1.0)).with_join(Join::Miter)
        }
        _ => Stroke::new(width).with_join(Join::Miter), // Solid and others
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
    radii: BorderRadii,
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

    if radii.as_circular().is_none() && matches!(cs.outline_style, BorderStyleValue::Solid) {
        // A solid outline round an elliptical corner (#977) is the ring
        // between the border box grown by the offset and that grown by the
        // offset and the width, each radius grown by the same amount on both
        // axes — one even-odd fill, as for a solid border, since an offset
        // ellipse is no ellipse and a stroke of one draws the wrong band. A
        // circular corner keeps the stroke, which is exact for it.
        let inner_rect = Rect::new(x - offset, y - offset, x + w + offset, y + h + offset);
        let outer_rect = inner_rect.inflate(ow, ow);
        let mut ring = radii.grown(offset + ow).path(outer_rect);
        if inner_rect.width() > 0.0 && inner_rect.height() > 0.0 {
            ring = punched(ring, &radii.grown(offset).path(inner_rect));
        }
        painter.fill_color(Fill::NonZero, transform, color, &ring.into());
    } else if !radii.is_square() {
        // Each radius grows by how far the outline's centre line is from the
        // border edge, on both axes (an elliptical corner stays elliptical).
        let outline_radii = radii.grown(offset + half);
        painter.stroke_color(
            &stroke,
            transform,
            color,
            &outline_radii.shape(outline_rect),
        );
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

/// [`spread_corner_radius`] for all four corners, each axis on its own
/// (§7.1.1 applies the rule to every radius; #977).
fn spread_radii(radii: BorderRadii, spread: f64) -> BorderRadii {
    radii.map(|c| {
        Vec2::new(
            spread_corner_radius(c.x, spread),
            spread_corner_radius(c.y, spread),
        )
    })
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
    // The element's corners, as its background draws them (#977: per axis,
    // fitted to the box).
    let radii = super::border_radii(node, scale);
    let has_radius = !radii.is_square();

    // The element's own border box, which an outer shadow is never painted
    // inside of. Every layer below is drawn as this hole punched out of the
    // layer's expanded rect, wound against it under `Fill::NonZero`
    // (`punched`).
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
    let hole: BezPath = radii.path(element_box);

    /// Whether the hole is wholly inside `outer` — the precondition the punch
    /// needs, and not a rounding detail.
    ///
    /// The punch is the hole wound against the layer (`punched`), so a point
    /// inside the hole and *outside* the layer winds to -1 and fills (under
    /// `Fill::EvenOdd`, which it used to be, it is crossed once and fills
    /// just the same). Where the border box
    /// escapes the layer's rect the punch therefore stops subtracting and
    /// starts adding, painting shadow inside the element that CSS says is not
    /// there. It escapes whenever a shadow is offset further than the layer is
    /// expanded — the near edge of `0 4px 12px`, say, whose first layers are
    /// expanded by less than the 4px they are pushed down — and the strip it
    /// paints only stays hidden while the element's own background is fully
    /// opaque. So the punch is applied only where it provably subtracts; where
    /// it does not, the shadow is filled through a clip that excludes the box
    /// instead (`paint_shadow_layers`, #976).
    fn hole_fits(
        hole_rect: Rect,
        hole_radii: BorderRadii,
        outer_rect: Rect,
        outer_radii: BorderRadii,
    ) -> bool {
        // The test below is for circular corners. With an elliptical one
        // (#977) both shapes are convex, so the hole is inside the layer when
        // every vertex of the hole's outline, flattened finely, is.
        let (Some(hole_radii), Some(outer_radii)) =
            (hole_radii.as_circular(), outer_radii.as_circular())
        else {
            let mut inside = true;
            peniko::kurbo::flatten(hole_radii.path(hole_rect), 0.05, |el| {
                use peniko::kurbo::PathEl;
                if let PathEl::MoveTo(p) | PathEl::LineTo(p) = el {
                    inside &= outer_radii.contains(outer_rect, p);
                }
            });
            return inside;
        };
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

    const SQUARE: BorderRadii = BorderRadii::ZERO;

    // Every layer of every outer shadow, in paint order (#976).
    let mut shadow_layers: Vec<ShadowLayer> = Vec::new();

    // css-backgrounds-3 §7.1: the first shadow is on top, so the list is
    // painted back to front (#1009) — as `paint_inset_box_shadow` paints its
    // own.
    for shadow in shadows.iter().rev() {
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
                    let expanded_radii = spread_shape_radii
                        .map(|c| Vec2::new(grow(c.x), grow(c.y)))
                        .fitted(layer_rect.width(), layer_rect.height());
                    (expanded_radii, expanded_radii.path(layer_rect))
                } else {
                    (SQUARE, layer_rect.into_path(0.1))
                };
                let fits = hole_fits(element_box, radii, layer_rect, outer_radii);
                shadow_layers.push((layer_color, layer_rect, outer, fits));
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
                let fitted = spread_shape_radii.fitted(shadow_rect.width(), shadow_rect.height());
                (fitted, fitted.path(shadow_rect))
            } else {
                (SQUARE, shadow_rect.into_path(0.1))
            };
            let fits = hole_fits(element_box, radii, shadow_rect, outer_radii);
            shadow_layers.push((color, shadow_rect, outer, fits));
        }
    }

    paint_shadow_layers(painter, transform, &hole, shadow_layers);
}

/// One layer of an outer shadow: its colour, its rect, its shape, and whether
/// the element's border box lies inside it (`hole_fits`).
type ShadowLayer = (AlphaColor<Srgb>, Rect, BezPath, bool);

/// Paint an element's outer-shadow layers, in paint order, outside its border
/// box `hole` (#976).
///
/// Where the hole lies inside every layer, each layer is its ring — the
/// layer's shape with the hole wound against it (`punched`), which subtracts the
/// hole and fills only the pixels the ring covers (see `hole_fits` for why
/// that needs the hole inside the layer).
///
/// Where it does not — a shadow offset further than some layer is expanded,
/// which is every hard offset shadow and the near edge of every blurred one —
/// the even-odd fill would *add* the part of the hole outside the layer, so
/// every layer is filled whole through **one** clip for the element that is
/// everything but the hole: a rect around all the layers and the hole, plus
/// the hole wound against it (`punched`). They used to be filled whole with no clip,
/// which painted the shadow through an element without an opaque background,
/// where CSS clips an outer shadow to the outside of the border box. The hole
/// is the same for every shadow of the element, so a shadow list costs one
/// clip mask, not one per shadow, and all its layers share one edge along the
/// hole.
fn paint_shadow_layers(
    painter: &mut dyn Painter,
    transform: Affine,
    hole: &BezPath,
    layers: Vec<ShadowLayer>,
) {
    if layers.iter().all(|&(_, _, _, fits)| fits) {
        for (color, _, layer, _) in layers {
            // The layer with the hole wound against it is what cuts: a
            // point inside both winds to zero and is left alone.
            let ring = punched(layer, hole);
            painter.fill_color(Fill::NonZero, transform, color, &ring.into());
        }
        return;
    }
    let Some(around) = layers
        .iter()
        .map(|(_, rect, _, _)| *rect)
        .reduce(|a, b| a.union(b))
    else {
        return;
    };
    // Grown past the layers' anti-aliased edges and around the hole, so the
    // clip has no edge but the hole's anywhere a layer paints. Two *device*
    // pixels: the rect is in the element's own space, and under a downscale
    // (`scale(0.125)`) a pad of two local pixels would sit inside the
    // shadow's own edge pixel and dim it.
    let linear = transform.determinant().abs().sqrt();
    let pad = if linear > 1e-6 { 2.0 / linear } else { 2.0 };
    let around = around.union(hole.bounding_box()).inflate(pad, pad);
    let outside_hole = punched(around.into_path(0.1), hole);
    painter.push_clip(Fill::NonZero, transform, &outside_hole.into());
    for (color, _, outer, _) in layers {
        painter.fill_color(Fill::NonZero, transform, color, &outer.into());
    }
    painter.pop_layer();
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
/// rounded shape. (A `GameViewport` inside the box clears the shadow under it
/// when paint reaches it, #1866.) Once matters on the software painter:
/// tiny-skia applies the clip mask per draw, so a stack of translucent fills through one rounded clip
/// multiplies the clip's edge coverage into itself and leaves a dark rim
/// along the curve (review of #1014, F1), where Vello composites the clip
/// once.
///
/// - **Unblurred**: the ring is a vector fill (the hole cut out by `punched`).
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
    radii: BorderRadii,
    node: &Node,
    transform: Affine,
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
    // The padding box's radii: the border radius less the adjacent borders,
    // each axis by its own (§5.5; a corner whose two borders differ is
    // elliptical, #977).
    let pad_radii = radii.inner(bt, br, bb, bl);
    let (clip, clip_fill): (PaintShape, Fill) = (pad_radii.shape(pad), Fill::NonZero);

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
        let hole = (pad + Vec2::new(offset_x, offset_y)).inset(-spread);
        let hole_radii = spread_radii(pad_radii, -spread).fitted(hole.width(), hole.height());
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
                            frame: None,
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
            let hole_path: BezPath = hole_radii.path(hole);
            // The padding box (and the hole, wherever the offset took it)
            // with the hole cut out (`punched`); the clip trims it to the
            // padding shape. The outer rect contains the hole, so the
            // hole wound against it only ever subtracts.
            let path = punched(pad.union(hole).inflate(1.0, 1.0).into_path(0.1), &hole_path);
            painter.fill_color(Fill::NonZero, transform, color, &path.into());
        }
        painter.pop_layer();
    }
}

/// `outer` with `hole` cut out of it under `Fill::NonZero`: the hole's
/// contours are appended wound against `outer`'s, so a point inside both
/// winds to zero. `hole` must lie inside `outer` — a part of it outside
/// winds to -1 and fills — which every caller ensures (a border's padding
/// box, an outline's border box, a shadow's hole that `hole_fits`, the rects
/// grown around a hole).
///
/// Not `Fill::EvenOdd`, which gives the same coverage for a hole inside
/// `outer` (#1865 D9). Vello's MSAA even-odd fill (`fill_path_ms_evenodd` in
/// `vello_shaders`' `fine.wgsl`, 0.7 and upstream `main`) clears its
/// workgroup winding arrays at the start of a fill with no barrier after the
/// previous fill's last reads of them, so a subgroup narrower than the
/// 64-invocation workgroup can zero a row another subgroup has not read yet.
/// Lavapipe (8-wide subgroups, run one after another) does it every time:
/// a rounded border came out as a 1/8-coverage wash over its whole corner
/// tile. A GPU whose subgroups are narrower than the workgroup has the same
/// race in principle (it did not show on RADV, whose wave runs the whole
/// workgroup). The
/// non-zero path reads only its own quad's words after its last barrier, and
/// a hole wound against its outline is also what Vello's analytic area
/// coverage needs (`BorderRadii::path_with_holes`).
fn punched(mut outer: BezPath, hole: &BezPath) -> BezPath {
    if (outer.area() > 0.0) == (hole.area() > 0.0) {
        outer.extend(hole.reverse_subpaths().iter());
    } else {
        outer.extend(hole.iter());
    }
    outer
}

/// The signed distance from `(u, v)` — relative to an ellipse's centre — to
/// the ellipse of radii `r`, positive outside: the first-order estimate
/// `(e - 1) / |∇e|` with `e = |(u / rx, v / ry)|`. Exactly `|(u, v)| - r` for
/// a circle, which is what the blurred inset shadow's corners used before
/// they could be elliptical (#977).
fn ellipse_distance(u: f64, v: f64, r: Vec2) -> f64 {
    let (nu, nv) = (u / r.x, v / r.y);
    let e = nu.hypot(nv);
    if e <= 1e-12 {
        return -r.x.min(r.y);
    }
    let grad = (nu / r.x).hypot(nv / r.y) / e;
    (e - 1.0) / grad
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
    hole_radii: BorderRadii,
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

    // Less each rounded corner's sliver, blurred. The radii are already
    // fitted to the hole (§5.5), and each corner is a quarter ellipse (#977).
    let corners = [
        (hole_radii.top_left, hole.x0, hole.y0, 1.0, 1.0),
        (hole_radii.top_right, hole.x1, hole.y0, -1.0, 1.0),
        (hole_radii.bottom_right, hole.x1, hole.y1, -1.0, -1.0),
        (hole_radii.bottom_left, hole.x0, hole.y1, 1.0, -1.0),
    ];
    for (r, hx, hy, sx, sy) in corners {
        if r.x <= 0.0 || r.y <= 0.0 {
            continue;
        }
        // The corner's box, and the arc's centre.
        let sq = Rect::new(hx, hy, hx + sx * r.x, hy + sy * r.y).abs();
        let (ccx, ccy) = (hx + sx * r.x, hy + sy * r.y);
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
                let d = ellipse_distance(x + 0.5 - ccx, y + 0.5 - ccy, r);
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
