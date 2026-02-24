//! SVG element painting.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, BezPath, Cap, Join, Point, Rect, Stroke};
use peniko::{Brush, Fill, Gradient};
use vello::Scene;

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
    scene: &mut Scene,
    _scale: f64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) {
    // Parse viewBox (default "0 0 24 24" for most icon SVGs)
    let viewbox = node
        .attributes
        .get("viewBox")
        .and_then(|v| parse_viewbox(v))
        .unwrap_or((0.0, 0.0, 24.0, 24.0));

    let (vb_x, vb_y, vb_w, vb_h) = viewbox;
    if vb_w <= 0.0 || vb_h <= 0.0 || w <= 0.0 || h <= 0.0 {
        return;
    }

    // Compute transform: scale viewBox to fit layout bounds, then translate to position
    let sx = w / vb_w;
    let sy = h / vb_h;
    let s = sx.min(sy); // uniform scale (preserveAspectRatio default)
    let tx = x + (w - vb_w * s) * 0.5 - vb_x * s;
    let ty = y + (h - vb_h * s) * 0.5 - vb_y * s;
    let transform = Affine::new([s, 0.0, 0.0, s, tx, ty]);

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
                    if let Some(fc) = fill_color {
                        scene.fill(Fill::NonZero, transform, fc, None, &path);
                    }
                    if let Some(sc) = stroke_color {
                        let stroke = Stroke::new(stroke_w).with_caps(linecap).with_join(linejoin);
                        scene.stroke(&stroke, transform, sc, None, &path);
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
                    scene.fill(Fill::NonZero, transform, fc, None, &rect);
                }
                if let Some(sc) = stroke_color {
                    let stroke = Stroke::new(stroke_w).with_caps(linecap).with_join(linejoin);
                    scene.stroke(&stroke, transform, sc, None, &rect);
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
                    scene.fill(Fill::NonZero, transform, fc, None, &circle);
                }
                if let Some(sc) = stroke_color {
                    let stroke = Stroke::new(stroke_w).with_caps(linecap).with_join(linejoin);
                    scene.stroke(&stroke, transform, sc, None, &circle);
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
                    scene.stroke(&stroke, transform, sc, None, &line);
                }
            }
            "polyline" | "polygon" => {
                if let Some(points_str) = child.attributes.get("points")
                    && let Some(path) = parse_polyline_points(points_str, el.tag == "polygon")
                {
                    if let Some(fc) = fill_color {
                        scene.fill(Fill::NonZero, transform, fc, None, &path);
                    }
                    if let Some(sc) = stroke_color {
                        let stroke = Stroke::new(stroke_w).with_caps(linecap).with_join(linejoin);
                        scene.stroke(&stroke, transform, sc, None, &path);
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

/// Resolve an SVG color attribute value, handling "none" and "currentColor".
pub(super) fn resolve_svg_color(
    attr: Option<&str>,
    current_color: AlphaColor<Srgb>,
) -> Option<AlphaColor<Srgb>> {
    match attr {
        None => None,
        Some("none") => None,
        Some("currentColor") => Some(current_color),
        Some(v) => parse_color(v),
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
