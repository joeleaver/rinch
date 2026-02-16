//! Border, outline, and box-shadow painting.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, Cap, Rect, RoundedRect, Stroke};
use peniko::Fill;
use vello::Scene;

use crate::computed_style::BorderStyleValue;
use crate::layout::parse_color;
use crate::node::{Node, NodeTree};

use super::contenteditable::parse_px;

/// Paint a CSS box-shadow effect.
///
/// Paint per-side borders with style support (solid, dashed, dotted, double).
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_borders(
    scene: &mut Scene,
    node: &Node,
    scale: f64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
    transform: Affine,
) {
    let cs = &node.computed_style;

    let sides = [
        // (width, color, style, start, end) for each side
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

            if radius > 0.0 {
                let rrect = RoundedRect::from_rect(border_rect, radius);
                scene.stroke(&stroke, transform, bc, None, &rrect);
            } else {
                scene.stroke(&stroke, transform, bc, None, &border_rect);
            }
        }
        return;
    }

    // Per-side rendering
    let top_w = sides[0].0 as f64 * scale;
    let right_w = sides[1].0 as f64 * scale;
    let bottom_w = sides[2].0 as f64 * scale;
    let left_w = sides[3].0 as f64 * scale;

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
        scene.stroke(&stroke, transform, bc, None, &path);
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
        scene.stroke(&stroke, transform, bc, None, &path);
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
        scene.stroke(&stroke, transform, bc, None, &path);
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
        scene.stroke(&stroke, transform, bc, None, &path);
    }
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
    scene: &mut Scene,
    node: &Node,
    scale: f64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
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

    if radius > 0.0 {
        let rrect = RoundedRect::from_rect(outline_rect, radius + offset + half);
        scene.stroke(&stroke, transform, color, None, &rrect);
    } else {
        scene.stroke(&stroke, transform, color, None, &outline_rect);
    }
}

/// Sort child nodes by z-index for correct paint order.
///
/// Returns children sorted in CSS stacking order:
/// 1. Negative z-index children (sorted ascending by z-index)
/// 2. Auto/0 z-index children (DOM order preserved)
/// 3. Positive z-index children (sorted ascending by z-index)
pub(super) fn sorted_paint_order(tree: &NodeTree, children: &[usize]) -> Vec<usize> {
    let mut negative_z: Vec<(i32, usize)> = Vec::new();
    let mut normal: Vec<usize> = Vec::new();
    let mut positive_z: Vec<(i32, usize)> = Vec::new();

    for &child_id in children {
        match tree.get(child_id).and_then(|c| c.computed_style.z_index) {
            Some(z) if z < 0 => negative_z.push((z, child_id)),
            Some(z) if z > 0 => positive_z.push((z, child_id)),
            _ => normal.push(child_id),
        }
    }

    negative_z.sort_by_key(|(z, _)| *z);
    positive_z.sort_by_key(|(z, _)| *z);

    let mut result = Vec::with_capacity(children.len());
    result.extend(negative_z.into_iter().map(|(_, id)| id));
    result.extend(normal);
    result.extend(positive_z.into_iter().map(|(_, id)| id));
    result
}

/// Parses shadow format: `offset-x offset-y blur-radius color`
/// Approximates blur by drawing expanded, semi-transparent rounded rects.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_box_shadow(
    scene: &mut Scene,
    shadow_str: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    scale: f64,
    node: &crate::node::Node,
    transform: Affine,
) {
    // Parse box-shadow: offset-x offset-y blur-radius [spread-radius] color
    // May also have "none" or multiple shadows separated by commas
    if shadow_str == "none" {
        return;
    }

    // Handle single shadow (multi-shadow not yet supported)
    let parts: Vec<&str> = shadow_str.split_whitespace().collect();
    if parts.len() < 3 {
        return;
    }

    let offset_x = parse_px(parts[0]).unwrap_or(0.0) as f64 * scale;
    let offset_y = parse_px(parts[1]).unwrap_or(0.0) as f64 * scale;
    let blur = parse_px(parts[2]).unwrap_or(0.0) as f64 * scale;

    // Remaining parts form the color (could be "rgba(0, 0, 0, 0.1)" spread across multiple parts)
    let (spread, color_start) = if parts.len() > 4 {
        // Check if parts[3] is a number (spread-radius) or start of color
        if parse_px(parts[3]).is_some()
            && !parts[3].starts_with('#')
            && !parts[3].starts_with("rgb")
        {
            (parse_px(parts[3]).unwrap_or(0.0) as f64 * scale, 4)
        } else {
            (0.0, 3)
        }
    } else {
        (0.0, 3)
    };

    let color_str = parts[color_start..].join(" ");
    let color = parse_color(&color_str).unwrap_or_else(|| {
        AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 40) // default shadow color
    });

    // Get border-radius from computed style (use average of all 4 corners)
    // Resolve percentage values against element dimensions
    let radius = {
        let cs = &node.computed_style;
        let resolve_size = node.layout.width.min(node.layout.height);
        let tl = cs.border_radius_top_left.resolve(resolve_size);
        let tr = cs.border_radius_top_right.resolve(resolve_size);
        let br = cs.border_radius_bottom_right.resolve(resolve_size);
        let bl = cs.border_radius_bottom_left.resolve(resolve_size);
        let avg = (tl + tr + br + bl) / 4.0;
        avg as f64 * scale
    };

    // Draw shadow as expanded rounded rect behind the element
    // Approximate blur with multiple layers of decreasing opacity
    let expand = blur * 0.5 + spread;
    let shadow_rect = Rect::new(
        x + offset_x - expand,
        y + offset_y - expand,
        x + w + offset_x + expand,
        y + h + offset_y + expand,
    );

    if blur > 0.0 {
        // Multi-layer blur approximation: 3 layers with increasing expansion
        let layers = 3;
        for i in 0..layers {
            let t = (i as f64 + 1.0) / layers as f64;
            let layer_expand = expand * t;
            let layer_rect = Rect::new(
                x + offset_x - layer_expand,
                y + offset_y - layer_expand,
                x + w + offset_x + layer_expand,
                y + h + offset_y + layer_expand,
            );
            let alpha_scale = (1.0 - t * 0.6) / layers as f64;
            let layer_color = AlphaColor::<Srgb>::from_rgba8(
                0,
                0,
                0,
                (color.components[3] * alpha_scale as f32 * 255.0) as u8,
            );
            if radius > 0.0 {
                let rrect = RoundedRect::from_rect(layer_rect, radius + layer_expand);
                scene.fill(Fill::NonZero, transform, layer_color, None, &rrect);
            } else {
                scene.fill(Fill::NonZero, transform, layer_color, None, &layer_rect);
            }
        }
    } else {
        // No blur: simple offset shadow
        if radius > 0.0 {
            let rrect = RoundedRect::from_rect(shadow_rect, radius);
            scene.fill(Fill::NonZero, transform, color, None, &rrect);
        } else {
            scene.fill(Fill::NonZero, transform, color, None, &shadow_rect);
        }
    }
}

