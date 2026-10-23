//! CSS gradients as brushes (#1798): linear and radial, repeating or not,
//! with stops placed along the gradient line by css-images-3 §3.5.3.
//!
//! A stop's position is a length, a percentage of the gradient line's
//! length, or both, resolved here against the line of the tile being
//! painted. The peniko gradient spans the first stop to the last (not the
//! whole line), so a stop outside `0%..100%` keeps its place, `Extend::Pad`
//! extends the end colours as CSS does, and `Extend::Repeat` repeats exactly
//! the stops' span, which is a repeating gradient's period.
//!
//! A radial gradient whose ending shape is an ellipse is a circle of radius
//! `rx` under a vertical scale of `ry / rx` about its centre: [`fill`] draws
//! it by filling the target shape mapped through the inverse of that scale,
//! under the scale.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, BezPath, Point, Rect, Shape};
use peniko::{Brush, Extend, Fill, Gradient};

use super::painter::{PaintShape, Painter};
use crate::computed_style::{
    BackgroundValue, GradientStop, LengthPercentageValue, RadialExtent, RadialShape,
};

/// The stops' positions along a line `len` long (in the tile's space;
/// `scale` turns a CSS px length into it), fixed up as css-images-3 §3.5.3
/// says: a missing first/last position is 0%/100%, a position before an
/// earlier one moves up to it, and a run of unpositioned stops is spaced
/// evenly between its neighbours. Stops whose colour did not resolve are
/// dropped after placing.
fn resolve_stops(stops: &[GradientStop], len: f64, scale: f64) -> Vec<(f64, AlphaColor<Srgb>)> {
    let n = stops.len();
    let mut pos: Vec<Option<f64>> = stops
        .iter()
        .map(|s| {
            s.position.map(|p| match p {
                LengthPercentageValue::Zero => 0.0,
                LengthPercentageValue::Length(px) => px as f64 * scale,
                LengthPercentageValue::Percent(f) => f as f64 * len,
                LengthPercentageValue::Calc { px, pct } => px as f64 * scale + pct as f64 * len,
            })
        })
        .collect();
    if n == 0 {
        return Vec::new();
    }
    if pos[0].is_none() {
        pos[0] = Some(0.0);
    }
    if pos[n - 1].is_none() {
        pos[n - 1] = Some(len);
    }
    let mut max = f64::NEG_INFINITY;
    for p in pos.iter_mut().flatten() {
        if *p < max {
            *p = max;
        }
        max = *p;
    }
    let mut i = 0;
    while i < n {
        if pos[i].is_some() {
            i += 1;
            continue;
        }
        let start = i - 1;
        let mut end = i;
        while pos[end].is_none() {
            end += 1;
        }
        let (a, b) = (pos[start].unwrap_or(0.0), pos[end].unwrap_or(len));
        for (k, slot) in pos.iter_mut().enumerate().take(end).skip(i) {
            *slot = Some(a + (b - a) * (k - start) as f64 / (end - start) as f64);
        }
        i = end;
    }
    stops
        .iter()
        .zip(pos)
        .filter_map(|(s, p)| Some((p?, s.color?)))
        .collect()
}

/// Lay `stops` (positions along a line from `start` in direction `dir`,
/// unit length) out as peniko stops over the span from the first to the
/// last. `None` when nothing is left to draw a gradient with; a span of
/// zero length is a single colour (`Some(Err)`).
fn span(
    resolved: &[(f64, AlphaColor<Srgb>)],
    repeating: bool,
) -> Option<Result<(f64, f64, Vec<(f32, AlphaColor<Srgb>)>), AlphaColor<Srgb>>> {
    let first = resolved.first()?.0;
    let last = resolved.last()?.0;
    let period = last - first;
    if period <= 1e-9 {
        // css-images-3 §3.6: a repeating gradient with a zero-length period
        // is its stops' average colour; a plain one is its last colour past
        // the (single) position and its first before it, which a 1e-3px
        // span of a hard stop gives.
        if repeating {
            let k = resolved.len() as f32;
            let mut c = [0.0f32; 4];
            for (_, col) in resolved {
                for (a, b) in c.iter_mut().zip(col.premultiply().components) {
                    *a += b / k;
                }
            }
            let a = c[3];
            let un = if a > 0.0 {
                [c[0] / a, c[1] / a, c[2] / a, a]
            } else {
                [0.0; 4]
            };
            return Some(Err(AlphaColor::new(un)));
        }
        let mut out = Vec::new();
        out.push((0.0, resolved[0].1));
        for (_, c) in resolved {
            out.push((0.0, *c));
        }
        return Some(Ok((first, first + 1e-3, out)));
    }
    let stops = resolved
        .iter()
        .map(|(p, c)| (((p - first) / period) as f32, *c))
        .collect();
    Some(Ok((first, last, stops)))
}

/// The brush and the transform to fill with, for a gradient laid on
/// `tile`. The transform maps the space the brush (and the shape to fill)
/// is in onto the tile's space.
fn gradient_brush(image: &BackgroundValue, tile: &Rect, scale: f64) -> Option<(Brush, Affine)> {
    match image {
        BackgroundValue::LinearGradient {
            angle_degrees,
            stops,
            repeating,
        } => {
            let angle_rad = (*angle_degrees as f64).to_radians();
            let (dx, dy) = (angle_rad.sin(), -angle_rad.cos());
            let half_w = tile.width() / 2.0;
            let half_h = tile.height() / 2.0;
            let half = half_w * dx.abs() + half_h * dy.abs();
            let c = tile.center();
            let start = Point::new(c.x - dx * half, c.y - dy * half);
            let resolved = resolve_stops(stops, 2.0 * half, scale);
            let brush = match span(&resolved, *repeating)? {
                Err(color) => Brush::Solid(color),
                Ok((a, b, stops)) => {
                    let p0 = Point::new(start.x + dx * a, start.y + dy * a);
                    let p1 = Point::new(start.x + dx * b, start.y + dy * b);
                    Brush::Gradient(
                        Gradient::new_linear(p0, p1)
                            .with_extend(if *repeating {
                                Extend::Repeat
                            } else {
                                Extend::Pad
                            })
                            .with_stops(stops.as_slice()),
                    )
                }
            };
            Some((brush, Affine::IDENTITY))
        }
        BackgroundValue::RadialGradient {
            stops,
            shape,
            position_x,
            position_y,
            repeating,
        } => {
            let (w, h) = (tile.width(), tile.height());
            let lp = |v: LengthPercentageValue, basis: f64| match v {
                LengthPercentageValue::Zero => 0.0,
                LengthPercentageValue::Length(px) => px as f64 * scale,
                LengthPercentageValue::Percent(f) => f as f64 * basis,
                LengthPercentageValue::Calc { px, pct } => px as f64 * scale + pct as f64 * basis,
            };
            let cx = lp(*position_x, w);
            let cy = lp(*position_y, h);
            let (rx, ry) = radial_radii(*shape, cx, cy, w, h, scale, &lp);
            let center = Point::new(tile.x0 + cx, tile.y0 + cy);
            let resolved = resolve_stops(stops, rx, scale);
            let Some(laid) = span(&resolved, *repeating) else {
                return None;
            };
            if !(rx > 0.0 && ry > 0.0) {
                // A degenerate ending shape (css-images-3 §3.2.1, roughly):
                // every point is past the last stop.
                let last = match laid {
                    Err(c) => c,
                    Ok(_) => resolved.last()?.1,
                };
                return Some((Brush::Solid(last), Affine::IDENTITY));
            }
            let brush = match laid {
                Err(color) => Brush::Solid(color),
                Ok(_) => {
                    // A circle of radius `r` about the centre, from 0: the
                    // software painter has no start radius, so the stops are
                    // laid out from radius 0 (a repeating gradient's period
                    // rotated to start there, a plain one's cut at 0).
                    let (radius, stops) = radial_stops(&resolved, *repeating)?;
                    Brush::Gradient(
                        Gradient::new_radial(Point::ORIGIN, radius as f32)
                            .with_extend(if *repeating {
                                Extend::Repeat
                            } else {
                                Extend::Pad
                            })
                            .with_stops(stops.as_slice()),
                    )
                }
            };
            let t = Affine::translate(center.to_vec2()) * Affine::scale_non_uniform(1.0, ry / rx);
            Some((brush, t))
        }
        _ => None,
    }
}

/// The colour at `p` along the line, interpolated premultiplied between the
/// stops around it (the end colours past the ends).
fn color_at(resolved: &[(f64, AlphaColor<Srgb>)], p: f64) -> AlphaColor<Srgb> {
    let i = resolved.partition_point(|(q, _)| *q <= p);
    if i == 0 {
        return resolved[0].1;
    }
    if i == resolved.len() {
        return resolved[i - 1].1;
    }
    let (p0, c0) = resolved[i - 1];
    let (p1, c1) = resolved[i];
    let t = if p1 > p0 {
        ((p - p0) / (p1 - p0)) as f32
    } else {
        1.0
    };
    let (a, b) = (c0.premultiply().components, c1.premultiply().components);
    let m: [f32; 4] = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t);
    if m[3] > 0.0 {
        AlphaColor::new([m[0] / m[3], m[1] / m[3], m[2] / m[3], m[3]])
    } else {
        AlphaColor::new([0.0; 4])
    }
}

/// A radial gradient's stops laid out from radius 0: the radius of the
/// peniko circle and its stops. A repeating gradient's stops span one
/// period starting at radius 0 (rotated by where the period falls); a plain
/// one's span `0..last`, cut at 0 when the first stop is before it.
fn radial_stops(
    resolved: &[(f64, AlphaColor<Srgb>)],
    repeating: bool,
) -> Option<(f64, Vec<(f32, AlphaColor<Srgb>)>)> {
    let first = resolved.first()?.0;
    let last = resolved.last()?.0;
    if repeating {
        let period = last - first;
        // Radius `r` shows the pattern at `first + (r - first) mod period`,
        // so radius 0 shows it at `start`; the period from there runs to
        // `last`, wraps to `first` and comes back to `start`.
        let start = first + (-first).rem_euclid(period);
        let mut out = vec![(0.0f32, color_at(resolved, start))];
        for &(q, c) in resolved {
            if q > start {
                out.push((((q - start) / period) as f32, c));
            }
        }
        for &(q, c) in resolved {
            let shifted = q + period;
            if shifted <= start + period && q < start {
                out.push((((shifted - start) / period) as f32, c));
            }
        }
        out.push((1.0, color_at(resolved, start)));
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        return Some((period, out));
    }
    if last <= 0.0 {
        return None;
    }
    let mut out = Vec::new();
    if first < 0.0 {
        out.push((0.0f32, color_at(resolved, 0.0)));
    }
    for &(q, c) in resolved {
        if q >= 0.0 {
            out.push(((q / last) as f32, c));
        }
    }
    Some((last, out))
}

/// A radial gradient's radii (css-images-3 §3.2.2).
fn radial_radii(
    shape: RadialShape,
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
    scale: f64,
    lp: &dyn Fn(LengthPercentageValue, f64) -> f64,
) -> (f64, f64) {
    match shape {
        RadialShape::Circle(r) => {
            let r = r as f64 * scale;
            (r, r)
        }
        RadialShape::Ellipse(rx, ry) => (lp(rx, w), lp(ry, h)),
        RadialShape::Extent { circle, extent } => {
            let (sx_near, sx_far) = (cx.abs().min((w - cx).abs()), cx.abs().max((w - cx).abs()));
            let (sy_near, sy_far) = (cy.abs().min((h - cy).abs()), cy.abs().max((h - cy).abs()));
            let corner = |far: bool| {
                let (x, y) = if far {
                    (sx_far, sy_far)
                } else {
                    (sx_near, sy_near)
                };
                (x, y)
            };
            if circle {
                let r = match extent {
                    RadialExtent::ClosestSide => sx_near.min(sy_near),
                    RadialExtent::FarthestSide => sx_far.max(sy_far),
                    RadialExtent::ClosestCorner => {
                        let (x, y) = corner(false);
                        x.hypot(y)
                    }
                    RadialExtent::FarthestCorner => {
                        let (x, y) = corner(true);
                        x.hypot(y)
                    }
                };
                (r, r)
            } else {
                match extent {
                    RadialExtent::ClosestSide => (sx_near, sy_near),
                    RadialExtent::FarthestSide => (sx_far, sy_far),
                    // The ellipse through the corner with the closest-/
                    // farthest-side ellipse's aspect ratio.
                    RadialExtent::ClosestCorner | RadialExtent::FarthestCorner => {
                        let (x, y) = corner(matches!(extent, RadialExtent::FarthestCorner));
                        if x == 0.0 || y == 0.0 {
                            (x, y)
                        } else {
                            let k = std::f64::consts::SQRT_2;
                            (x * k, y * k)
                        }
                    }
                }
            }
        }
    }
}

/// Fill `target` with the gradient `image` laid on `tile`. `false` when
/// `image` is not a gradient or has nothing to draw.
pub(super) fn fill(
    painter: &mut dyn Painter,
    image: &BackgroundValue,
    scale: f64,
    fill: Fill,
    transform: Affine,
    tile: Rect,
    target: &PaintShape,
) -> bool {
    let Some((brush, t)) = gradient_brush(image, &tile, scale) else {
        return false;
    };
    if t == Affine::IDENTITY {
        painter.fill(fill, transform, &brush, target);
    } else {
        let inv = t.inverse();
        let path: BezPath = match target {
            PaintShape::Rect(r) => inv * r.to_path(0.1),
            PaintShape::RoundedRect(r) => inv * r.to_path(0.1),
            PaintShape::BezPath(p) => inv * p.clone(),
            PaintShape::Circle(c) => inv * c.to_path(0.1),
            PaintShape::Line(_) => return true,
        };
        painter.fill(fill, transform * t, &brush, &PaintShape::BezPath(path));
    }
    true
}
