//! SVG element painting.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, BezPath, Cap, Join, Point, Rect, Stroke};
use peniko::{Brush, Fill, Gradient};

use super::painter::Painter;
use crate::layout::parse_color;
use crate::node::{Node, NodeKind, NodeTree};

// =============================================================================
// SVG rendering
// =============================================================================

/// Paint an `<svg>` element and its children (path, rect, circle, line, polyline).
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_svg(
    tree: &NodeTree,
    node: &Node,
    painter: &mut dyn Painter,
    _scale: f64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    css_transform: Affine,
) {
    // Parse viewBox (default "0 0 24 24" for most icon SVGs)
    let viewbox = node
        .attributes
        .get("viewBox")
        .and_then(|v| parse_viewbox(v))
        .unwrap_or((0.0, 0.0, 24.0, 24.0));

    let (_, _, vb_w, vb_h) = viewbox;
    if vb_w <= 0.0 || vb_h <= 0.0 || w <= 0.0 || h <= 0.0 {
        return;
    }

    // Compute transform: CSS transform composed with viewBox-to-layout scaling.
    // Honors `preserveAspectRatio="none"` (stretch x and y independently);
    // any other value (including the SVG default `xMidYMid meet`) collapses
    // to uniform scaling centered in the container.
    let preserve_aspect = node
        .attributes
        .get("preserveAspectRatio")
        .map(|v| v.as_str());
    let (vb_sx, vb_sy, vb_tx, vb_ty) =
        viewbox_to_box_transform(viewbox, (x, y, w, h), preserve_aspect);
    let transform = css_transform * Affine::new([vb_sx, 0.0, 0.0, vb_sy, vb_tx, vb_ty]);

    // Resolve "currentColor" — walk up the tree to find a `color` CSS property
    let current_color = resolve_current_color(tree, node);

    // Parse SVG-level fill/stroke defaults
    let svg_fill = node.attributes.get("fill").map(|v| v.as_str());
    let svg_stroke = node.attributes.get("stroke").map(|v| v.as_str());
    let svg_stroke_width = node
        .attributes
        .get("stroke-width")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(2.0);
    let svg_stroke_linecap = node
        .attributes
        .get("stroke-linecap")
        .map(|v| parse_linecap(v))
        .unwrap_or(Cap::Butt);
    let svg_stroke_linejoin = node
        .attributes
        .get("stroke-linejoin")
        .map(|v| parse_linejoin(v))
        .unwrap_or(Join::Miter);

    // Paint each child SVG element
    let child_ids: Vec<usize> = node.children.to_vec();
    for child_id in child_ids {
        let Some(child) = tree.get(child_id) else {
            continue;
        };
        let NodeKind::Element(ref el) = child.kind else {
            continue;
        };

        // Per-element fill/stroke (override SVG defaults)
        let fill_attr = child
            .attributes
            .get("fill")
            .map(|v| v.as_str())
            .or(svg_fill);
        let stroke_attr = child
            .attributes
            .get("stroke")
            .map(|v| v.as_str())
            .or(svg_stroke);
        let stroke_w = child
            .attributes
            .get("stroke-width")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(svg_stroke_width);
        let linecap = child
            .attributes
            .get("stroke-linecap")
            .map(|v| parse_linecap(v))
            .unwrap_or(svg_stroke_linecap);
        let linejoin = child
            .attributes
            .get("stroke-linejoin")
            .map(|v| parse_linejoin(v))
            .unwrap_or(svg_stroke_linejoin);

        let fill_color = resolve_svg_color(fill_attr, current_color);
        let stroke_color = resolve_svg_color(stroke_attr, current_color);

        match el.tag.as_str() {
            "path" => {
                if let Some(d) = child.attributes.get("d")
                    && let Ok(path) = BezPath::from_svg(d)
                {
                    let shape = super::painter::PaintShape::BezPath(path);
                    if let Some(fc) = fill_color {
                        painter.fill_color(Fill::NonZero, transform, fc, &shape);
                    }
                    if let Some(sc) = stroke_color {
                        let stroke = Stroke::new(stroke_w).with_caps(linecap).with_join(linejoin);
                        painter.stroke_color(&stroke, transform, sc, &shape);
                    }
                }
            }
            "rect" => {
                let rx = child
                    .attributes
                    .get("x")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let ry = child
                    .attributes
                    .get("y")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let rw = child
                    .attributes
                    .get("width")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let rh = child
                    .attributes
                    .get("height")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let rect = Rect::new(rx, ry, rx + rw, ry + rh);
                if let Some(fc) = fill_color {
                    painter.fill_color(Fill::NonZero, transform, fc, &rect.into());
                }
                if let Some(sc) = stroke_color {
                    let stroke = Stroke::new(stroke_w).with_caps(linecap).with_join(linejoin);
                    painter.stroke_color(&stroke, transform, sc, &rect.into());
                }
            }
            "circle" => {
                let cx = child
                    .attributes
                    .get("cx")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let cy = child
                    .attributes
                    .get("cy")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let r = child
                    .attributes
                    .get("r")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let circle = peniko::kurbo::Circle::new((cx, cy), r);
                if let Some(fc) = fill_color {
                    painter.fill_color(Fill::NonZero, transform, fc, &circle.into());
                }
                if let Some(sc) = stroke_color {
                    let stroke = Stroke::new(stroke_w).with_caps(linecap).with_join(linejoin);
                    painter.stroke_color(&stroke, transform, sc, &circle.into());
                }
            }
            "line" => {
                let x1 = child
                    .attributes
                    .get("x1")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let y1 = child
                    .attributes
                    .get("y1")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let x2 = child
                    .attributes
                    .get("x2")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let y2 = child
                    .attributes
                    .get("y2")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let line = peniko::kurbo::Line::new((x1, y1), (x2, y2));
                if let Some(sc) = stroke_color {
                    let stroke = Stroke::new(stroke_w).with_caps(linecap).with_join(linejoin);
                    painter.stroke_color(&stroke, transform, sc, &line.into());
                }
            }
            "polyline" | "polygon" => {
                if let Some(points_str) = child.attributes.get("points")
                    && let Some(path) = parse_polyline_points(points_str, el.tag == "polygon")
                {
                    let shape = super::painter::PaintShape::BezPath(path);
                    if let Some(fc) = fill_color {
                        painter.fill_color(Fill::NonZero, transform, fc, &shape);
                    }
                    if let Some(sc) = stroke_color {
                        let stroke = Stroke::new(stroke_w).with_caps(linecap).with_join(linejoin);
                        painter.stroke_color(&stroke, transform, sc, &shape);
                    }
                }
            }
            _ => {} // Unsupported SVG elements (text, g, etc.)
        }
    }
}

/// Parse SVG viewBox attribute: "minX minY width height"
pub(super) fn parse_viewbox(s: &str) -> Option<(f64, f64, f64, f64)> {
    let parts: Vec<f64> = s
        .split_whitespace()
        .flat_map(|p| p.split(','))
        .filter_map(|p| p.parse().ok())
        .collect();
    if parts.len() == 4 {
        Some((parts[0], parts[1], parts[2], parts[3]))
    } else {
        None
    }
}

/// Compute the (sx, sy, tx, ty) affine coefficients that map a viewBox of
/// (vb_x, vb_y, vb_w, vb_h) into a layout box of (x, y, w, h), honoring the
/// `preserveAspectRatio` attribute.
///
/// The full SVG spec enumerates nine alignment modes plus `meet` / `slice`
/// fitting. We support the two cases that matter in practice:
///
/// - `"none"` — stretch x and y independently to fill the layout box exactly.
/// - anything else (including the SVG default `xMidYMid meet`) — uniform
///   scale by `min(sx, sy)`, content centered in the layout box.
pub(super) fn viewbox_to_box_transform(
    viewbox: (f64, f64, f64, f64),
    layout: (f64, f64, f64, f64),
    preserve_aspect: Option<&str>,
) -> (f64, f64, f64, f64) {
    let (vb_x, vb_y, vb_w, vb_h) = viewbox;
    let (x, y, w, h) = layout;
    let sx = w / vb_w;
    let sy = h / vb_h;
    if preserve_aspect == Some("none") {
        (sx, sy, x - vb_x * sx, y - vb_y * sy)
    } else {
        let s = sx.min(sy);
        (
            s,
            s,
            x + (w - vb_w * s) * 0.5 - vb_x * s,
            y + (h - vb_h * s) * 0.5 - vb_y * s,
        )
    }
}

/// Resolve `currentColor` by walking up the DOM tree to find a CSS `color` property.
pub(super) fn resolve_current_color(tree: &NodeTree, node: &Node) -> AlphaColor<Srgb> {
    let mut current = Some(node.id);
    while let Some(id) = current {
        if let Some(n) = tree.get(id) {
            // Check computed_style.color
            if let Some(c) = n.computed_style.color {
                return c;
            }
            current = n.parent;
        } else {
            break;
        }
    }
    // Default: black
    AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255)
}

/// Resolve an SVG `fill`/`stroke` attribute: `none`, `currentcolor` (any
/// casing — the spec spells it `currentcolor`, Tabler emits `currentColor`),
/// or a CSS `<color>`.
pub(super) fn resolve_svg_color(
    attr: Option<&str>,
    current_color: AlphaColor<Srgb>,
) -> Option<AlphaColor<Srgb>> {
    let value = attr?.trim();
    if value.eq_ignore_ascii_case("none") {
        None
    } else if value.eq_ignore_ascii_case("currentcolor") {
        // Checked before the parse: this runs per SVG child per paint, and
        // every icon says it.
        Some(current_color)
    } else {
        parse_color(value)
    }
}

/// Parse SVG stroke-linecap attribute.
pub(super) fn parse_linecap(v: &str) -> Cap {
    match v {
        "round" => Cap::Round,
        "square" => Cap::Square,
        _ => Cap::Butt,
    }
}

/// Parse SVG stroke-linejoin attribute.
pub(super) fn parse_linejoin(v: &str) -> Join {
    match v {
        "round" => Join::Round,
        "bevel" => Join::Bevel,
        _ => Join::Miter,
    }
}

/// Parse SVG polyline/polygon points attribute into a BezPath.
pub(super) fn parse_polyline_points(points_str: &str, close: bool) -> Option<BezPath> {
    let nums: Vec<f64> = points_str
        .split_whitespace()
        .flat_map(|p| p.split(','))
        .filter_map(|p| p.parse().ok())
        .collect();
    if nums.len() < 4 || !nums.len().is_multiple_of(2) {
        return None;
    }
    let mut path = BezPath::new();
    path.move_to((nums[0], nums[1]));
    for i in (2..nums.len()).step_by(2) {
        path.line_to((nums[i], nums[i + 1]));
    }
    if close {
        path.close_path();
    }
    Some(path)
}

/// Convert `GradientStop` list into peniko `ColorStop` tuples for `with_stops`.
pub(super) fn gradient_color_stops(
    stops: &[crate::computed_style::GradientStop],
) -> Vec<(f32, AlphaColor<Srgb>)> {
    stops
        .iter()
        .filter_map(|s| s.color.map(|c| (s.offset, c)))
        .collect()
}

/// Build a `Brush` for a CSS linear-gradient.
pub(super) fn build_linear_gradient_brush(
    angle_degrees: f32,
    stops: &[crate::computed_style::GradientStop],
    rect: &Rect,
) -> Brush {
    let angle_rad = angle_degrees.to_radians();
    let dx = (angle_rad.sin()) as f64;
    let dy = -(angle_rad.cos()) as f64;
    let half_w = (rect.x1 - rect.x0) / 2.0;
    let half_h = (rect.y1 - rect.y0) / 2.0;
    let len = half_w * dx.abs() + half_h * dy.abs();
    let cx = rect.x0 + half_w;
    let cy = rect.y0 + half_h;
    let start = Point::new(cx - dx * len, cy - dy * len);
    let end = Point::new(cx + dx * len, cy + dy * len);
    let color_stops = gradient_color_stops(stops);
    let gradient = Gradient::new_linear(start, end).with_stops(color_stops.as_slice());
    Brush::Gradient(gradient)
}

/// Build a `Brush` for a CSS radial-gradient.
pub(super) fn build_radial_gradient_brush(
    stops: &[crate::computed_style::GradientStop],
    rect: &Rect,
) -> Brush {
    let half_w = (rect.x1 - rect.x0) / 2.0;
    let half_h = (rect.y1 - rect.y0) / 2.0;
    let center = Point::new(rect.x0 + half_w, rect.y0 + half_h);
    let radius = half_w.max(half_h) as f32;
    let color_stops = gradient_color_stops(stops);
    let gradient = Gradient::new_radial(center, radius).with_stops(color_stops.as_slice());
    Brush::Gradient(gradient)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {a} ≈ {b}");
    }

    #[test]
    fn default_aspect_uniform_scales_and_centers() {
        // Square viewBox in a wide container — content should fit by height
        // and be centered horizontally.
        let (sx, sy, tx, ty) =
            viewbox_to_box_transform((0.0, 0.0, 100.0, 100.0), (0.0, 0.0, 400.0, 200.0), None);
        approx(sx, 2.0); // min(400/100, 200/100) = 2.0
        approx(sy, 2.0);
        approx(tx, 100.0); // (400 - 200) / 2 = 100
        approx(ty, 0.0);
    }

    #[test]
    fn aspect_none_stretches_independently() {
        // The use case in rawdaw's arrangement: a 24:100 viewBox painting a
        // long horizontal gridline strip. Without "none" support, the lines
        // bunch into the center; with it, they fill the full width.
        let (sx, sy, tx, ty) = viewbox_to_box_transform(
            (0.0, 0.0, 24.0, 100.0),
            (0.0, 0.0, 1600.0, 150.0),
            Some("none"),
        );
        approx(sx, 1600.0 / 24.0);
        approx(sy, 1.5);
        approx(tx, 0.0);
        approx(ty, 0.0);
    }

    #[test]
    fn aspect_none_honors_layout_origin_and_vb_offset() {
        // Layout starts at (10, 20); viewBox starts at (5, 5). Both contribute
        // to the translate.
        let (sx, sy, tx, ty) = viewbox_to_box_transform(
            (5.0, 5.0, 100.0, 100.0),
            (10.0, 20.0, 400.0, 200.0),
            Some("none"),
        );
        approx(sx, 4.0);
        approx(sy, 2.0);
        approx(tx, 10.0 - 5.0 * 4.0); // -10
        approx(ty, 20.0 - 5.0 * 2.0); // 10
    }

    #[test]
    fn unknown_aspect_string_falls_back_to_default() {
        // Anything other than literal "none" — including the SVG default
        // `xMidYMid meet` — uses uniform scale. The current implementation
        // doesn't distinguish among the nine alignment modes; this test
        // pins that behavior so future spec-completion changes have a
        // baseline to update.
        let (sx_a, sy_a, tx_a, ty_a) =
            viewbox_to_box_transform((0.0, 0.0, 100.0, 100.0), (0.0, 0.0, 400.0, 200.0), None);
        let (sx_b, sy_b, tx_b, ty_b) = viewbox_to_box_transform(
            (0.0, 0.0, 100.0, 100.0),
            (0.0, 0.0, 400.0, 200.0),
            Some("xMidYMid meet"),
        );
        approx(sx_a, sx_b);
        approx(sy_a, sy_b);
        approx(tx_a, tx_b);
        approx(ty_a, ty_b);
    }
}
