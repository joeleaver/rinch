//! Border, outline, and box-shadow painting.

use peniko::Fill;
use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, BezPath, Cap, Point, Rect, RoundedRectRadii, Shape, Stroke};

use super::painter::Painter;
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
        let tl = cs.border_radius_top_left.resolve(resolve_size) as f64 * scale;
        let tr = cs.border_radius_top_right.resolve(resolve_size) as f64 * scale;
        let br = cs.border_radius_bottom_right.resolve(resolve_size) as f64 * scale;
        let bl = cs.border_radius_bottom_left.resolve(resolve_size) as f64 * scale;
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

    for shadow in shadows {
        // TODO: inset shadows not yet supported
        if shadow.inset {
            continue;
        }

        let offset_x = shadow.offset_x as f64 * scale;
        let offset_y = shadow.offset_y as f64 * scale;
        let blur = shadow.blur_radius as f64 * scale;
        let spread = shadow.spread_radius as f64 * scale;
        let color: AlphaColor<Srgb> = shadow
            .color
            .map(|c| {
                let rgba = c.to_rgba8();
                AlphaColor::<Srgb>::from_rgba8(rgba.r, rgba.g, rgba.b, rgba.a)
            })
            .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 40));

        if blur > 0.0 {
            // Approximate Gaussian box-shadow blur with concentric layers.
            // The max expansion is empirically tuned to match Chrome's visible
            // shadow extent. Using Gaussian-weighted alpha per layer with the
            // actual shadow color (not hardcoded black).
            let max_expand = blur * 0.5 + spread;
            let layers: usize = 8;

            // Extract actual shadow RGB
            let sr = (color.components[0] * 255.0) as u8;
            let sg = (color.components[1] * 255.0) as u8;
            let sb = (color.components[2] * 255.0) as u8;
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
                let alpha_u8 = (base_alpha * alpha_scale * 255.0).min(255.0) as u8;
                if alpha_u8 == 0 {
                    continue;
                }
                let layer_color = AlphaColor::<Srgb>::from_rgba8(sr, sg, sb, alpha_u8);
                let outer = if has_radius {
                    let expanded_radii = RoundedRectRadii::new(
                        radii.top_left + layer_expand,
                        radii.top_right + layer_expand,
                        radii.bottom_right + layer_expand,
                        radii.bottom_left + layer_expand,
                    );
                    layer_rect.to_rounded_rect(expanded_radii).into_path(0.1)
                } else {
                    layer_rect.into_path(0.1)
                };
                painter.fill_color(
                    Fill::EvenOdd,
                    transform,
                    layer_color,
                    &ring(outer, &hole).into(),
                );
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
            let outer = if has_radius {
                shadow_rect.to_rounded_rect(radii).into_path(0.1)
            } else {
                shadow_rect.into_path(0.1)
            };
            painter.fill_color(Fill::EvenOdd, transform, color, &ring(outer, &hole).into());
        }
    }
}
