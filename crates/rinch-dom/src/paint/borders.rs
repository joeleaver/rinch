//! Border, outline, and box-shadow painting.

use peniko::Fill;
use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, Cap, Rect, RoundedRectRadii, Stroke};

use super::painter::Painter;
use crate::computed_style::BorderStyleValue;
use crate::node::{Node, NodeTree};

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

/// Check whether a node creates a CSS stacking context.
/// Delegates to `Node::creates_stacking_context()`.
pub(super) fn creates_stacking_context(node: &Node) -> bool {
    node.creates_stacking_context()
}

/// A descendant stacking context entry collected for painting at an ancestor SC level.
pub(super) struct StackingContextEntry {
    /// The z-index of the SC node (0 if `z_index` is `None`).
    pub z_index: i32,
    /// The node ID of the SC.
    pub node_id: usize,
    /// Accumulated offset X (physical pixels) from the collecting ancestor to this node's parent.
    pub offset_x: f64,
    /// Accumulated offset Y (physical pixels) from the collecting ancestor to this node's parent.
    pub offset_y: f64,
    /// DOM order index for stable sorting within the same z-index.
    pub dom_order: usize,
}

/// Collect descendant stacking contexts that should be painted at this SC level.
///
/// Walks descendants starting from `children`, collecting nodes that form SCs.
/// Stops recursion at each SC boundary (those belong to the deeper SC).
/// Accumulates layout offsets through intermediate (non-SC) nodes so each
/// collected entry has the correct offset for `paint_node`.
///
/// `offset_x`/`offset_y` should be the scroll-adjusted parent position
/// (i.e., what would be passed to `paint_node` as offset for direct children).
pub(super) fn collect_stacking_contexts(
    tree: &NodeTree,
    children: &[usize],
    scale: f64,
    offset_x: f64,
    offset_y: f64,
) -> Vec<StackingContextEntry> {
    let mut result = Vec::new();
    let mut order_counter = 0usize;
    collect_sc_recursive(
        tree,
        children,
        scale,
        offset_x,
        offset_y,
        &mut result,
        &mut order_counter,
    );
    result
}

fn collect_sc_recursive(
    tree: &NodeTree,
    children: &[usize],
    scale: f64,
    parent_offset_x: f64,
    parent_offset_y: f64,
    result: &mut Vec<StackingContextEntry>,
    order_counter: &mut usize,
) {
    for &child_id in children {
        let Some(child) = tree.get(child_id) else {
            continue;
        };

        if creates_stacking_context(child) {
            // This child is a SC — collect it with the current accumulated offset.
            // paint_node will add child.layout.x/y itself, so we pass parent_offset.
            let z = child.computed_style.z_index.unwrap_or(0);
            result.push(StackingContextEntry {
                z_index: z,
                node_id: child_id,
                offset_x: parent_offset_x,
                offset_y: parent_offset_y,
                dom_order: *order_counter,
            });
            *order_counter += 1;
            // Don't recurse — deeper SCs belong to this child's SC level.
        } else {
            // Not a SC — recurse through its children to find deeper SCs.
            // Accumulate this node's layout offset.
            let child_x = parent_offset_x + child.layout.x as f64 * scale;
            let child_y = parent_offset_y + child.layout.y as f64 * scale;
            let sx = child.scroll_offset.0 * scale;
            let sy = child.scroll_offset.1 * scale;
            *order_counter += 1;
            collect_sc_recursive(
                tree,
                &child.children,
                scale,
                child_x - sx,
                child_y - sy,
                result,
                order_counter,
            );
        }
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
                if has_radius {
                    let expanded_radii = RoundedRectRadii::new(
                        radii.top_left + layer_expand,
                        radii.top_right + layer_expand,
                        radii.bottom_right + layer_expand,
                        radii.bottom_left + layer_expand,
                    );
                    let rrect = layer_rect.to_rounded_rect(expanded_radii);
                    painter.fill_color(Fill::NonZero, transform, layer_color, &rrect.into());
                } else {
                    painter.fill_color(Fill::NonZero, transform, layer_color, &layer_rect.into());
                }
            }
        } else {
            // No blur: simple offset shadow
            if has_radius {
                let rrect = shadow_rect.to_rounded_rect(radii);
                painter.fill_color(Fill::NonZero, transform, color, &rrect.into());
            } else {
                painter.fill_color(Fill::NonZero, transform, color, &shadow_rect.into());
            }
        }
    }
}
