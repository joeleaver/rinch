//! Vello scene building for rinch-dom.
//!
//! Walks the node tree and emits Vello drawing commands
//! for backgrounds, borders, and text.

mod borders;
mod contenteditable;
mod svg;
mod text;

use borders::*;
use contenteditable::*;
use svg::*;
use text::*;

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, Rect, RoundedRect};
use peniko::{Brush, Fill};
use vello::Scene;

use crate::computed_style::{
    BackgroundValue, LineHeightValue, OverflowValue, PositionValue, VisibilityValue,
};
use crate::node::{NodeKind, NodeTree, RawNodeId};

/// Paint the entire document to a Vello scene.
///
/// `scale` is the DPI scale factor (1.0 = 96dpi).
/// `viewport` is the viewport size in physical pixels.
pub fn paint_document(
    tree: &NodeTree,
    scene: &mut Scene,
    scale: f64,
    _viewport: (f32, f32),
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
) {
    scene.reset();
    paint_node(
        tree,
        tree.body_id,
        scene,
        scale,
        0.0,
        0.0,
        font_cx,
        layout_cx,
        Affine::IDENTITY,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_node(
    tree: &NodeTree,
    node_id: RawNodeId,
    scene: &mut Scene,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
    parent_transform: Affine,
) {
    let Some(node) = tree.get(node_id) else {
        return;
    };

    // Skip hidden elements (display: none, style/script tags)
    if let NodeKind::Element(ref el) = node.kind
        && matches!(
            el.tag.as_str(),
            "style" | "script" | "head" | "meta" | "link"
        )
    {
        return;
    }
    let layout = &node.layout;

    // Skip zero-size elements (display: none produces 0x0 layout)
    if layout.width == 0.0 && layout.height == 0.0 {
        return;
    }

    // Absolute position of this node
    let x = offset_x + layout.x as f64 * scale;
    let y = offset_y + layout.y as f64 * scale;
    let w = layout.width as f64 * scale;
    let h = layout.height as f64 * scale;

    // Sticky position adjustment
    let (x, y) = if node.computed_style.position == PositionValue::Sticky {
        // Find the nearest scroll ancestor's scroll offset
        let mut scroll_y = 0.0_f64;
        let mut ancestor_id = node.parent;
        while let Some(aid) = ancestor_id {
            if let Some(ancestor) = tree.get(aid) {
                let ov = ancestor.computed_style.overflow_y;
                if matches!(
                    ov,
                    OverflowValue::Auto | OverflowValue::Scroll | OverflowValue::Hidden
                ) {
                    scroll_y = ancestor.scroll_offset.1 * scale;
                    break;
                }
                ancestor_id = ancestor.parent;
            } else {
                break;
            }
        }

        // Apply sticky top constraint: element should not scroll above `top` offset from container
        let sticky_top = node.computed_style.top.to_px() as f64 * scale;
        let adjusted_y = y.max(offset_y + scroll_y + sticky_top);
        (x, adjusted_y)
    } else {
        (x, y)
    };

    match &node.kind {
        NodeKind::Element(el) if el.tag == "svg" => {
            paint_svg(tree, node, scene, scale, x, y, w, h);
        }
        NodeKind::Element(_) => {
            let rect = Rect::new(x, y, x + w, y + h);
            let visible = !matches!(
                node.computed_style.visibility,
                VisibilityValue::Hidden | VisibilityValue::Collapse
            );

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

            // Compute composed CSS transform for this node
            let node_transform = if !node.computed_style.transform.is_identity {
                let m = &node.computed_style.transform.matrix;
                let cs = &node.computed_style;
                let ox = cs.transform_origin_x.resolve(node.layout.width);
                let oy = cs.transform_origin_y.resolve(node.layout.height);
                let cx = x + ox as f64 * scale;
                let cy = y + oy as f64 * scale;
                parent_transform
                    * Affine::translate((cx, cy))
                    * Affine::new(*m)
                    * Affine::translate((-cx, -cy))
            } else {
                parent_transform
            };

            // Get opacity from computed style and push layer if needed
            let opacity = node.computed_style.opacity;
            let has_opacity = opacity < 1.0;
            if has_opacity {
                scene.push_layer(
                    Fill::NonZero,
                    peniko::Mix::Normal,
                    opacity,
                    node_transform,
                    &rect,
                );
            }

            // Only paint this element's own visuals if visible
            // (children may override with visibility: visible)
            if visible {
                // Parse and render box-shadow
                if let Some(shadow_str) = get_style_property(node, "box-shadow") {
                    paint_box_shadow(scene, &shadow_str, x, y, w, h, scale, node, node_transform);
                }

                // Get background from computed style (solid color or gradient)
                match &node.computed_style.background {
                    BackgroundValue::Color(bg_color) => {
                        if radius > 0.0 {
                            let rrect = RoundedRect::from_rect(rect, radius);
                            scene.fill(Fill::NonZero, node_transform, *bg_color, None, &rrect);
                        } else {
                            scene.fill(Fill::NonZero, node_transform, *bg_color, None, &rect);
                        }
                    }
                    BackgroundValue::LinearGradient {
                        angle_degrees,
                        stops,
                    } => {
                        let brush = build_linear_gradient_brush(*angle_degrees, stops, &rect);
                        if radius > 0.0 {
                            let rrect = RoundedRect::from_rect(rect, radius);
                            scene.fill(Fill::NonZero, node_transform, &brush, None, &rrect);
                        } else {
                            scene.fill(Fill::NonZero, node_transform, &brush, None, &rect);
                        }
                    }
                    BackgroundValue::RadialGradient { stops } => {
                        let brush = build_radial_gradient_brush(stops, &rect);
                        if radius > 0.0 {
                            let rrect = RoundedRect::from_rect(rect, radius);
                            scene.fill(Fill::NonZero, node_transform, &brush, None, &rrect);
                        } else {
                            scene.fill(Fill::NonZero, node_transform, &brush, None, &rect);
                        }
                    }
                    BackgroundValue::None => {}
                }

                // Render borders per-side with style support
                paint_borders(scene, node, scale, x, y, w, h, radius, node_transform);

                // Render outline (drawn outside the box model)
                paint_outline(scene, node, scale, x, y, w, h, radius, node_transform);

                // Render input element value
                if matches!(node.tag(), Some("input" | "textarea")) {
                    paint_input_value(
                        node,
                        scene,
                        scale,
                        x,
                        y,
                        w,
                        h,
                        font_cx,
                        layout_cx,
                        node_transform,
                    );
                }
            }

            // Handle overflow clipping from computed style
            // Pushed early so CE overlay and children are both clipped
            let overflow_y = node.computed_style.overflow_y;
            let clips = matches!(
                overflow_y,
                OverflowValue::Hidden | OverflowValue::Scroll | OverflowValue::Auto
            );

            if clips {
                scene.push_clip_layer(Fill::NonZero, node_transform, &rect);
            }

            // Render contenteditable cursor/selection overlay
            if node
                .attributes
                .get("data-ce-focused")
                .map(|s| s == "true")
                .unwrap_or(false)
            {
                // Extract cursor position and selection from attributes
                let cursor_pos = node
                    .attributes
                    .get("data-ce-cursor")
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                let selection_start = node
                    .attributes
                    .get("data-ce-selection-start")
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(cursor_pos);

                let padding_left = node.computed_style.padding_left.to_px() as f64 * scale;
                let content_width = node.layout.width as f64 * scale - padding_left * 2.0;

                // Account for scroll offset so selection moves with content
                let ce_scroll_x = node.scroll_offset.0 * scale;
                let ce_scroll_y = node.scroll_offset.1 * scale;

                // Try to find a Parley layout for cursor rendering:
                // 1. IFC root: node.text_layout (multi-child inline formatting context)
                // 2. Single text child: child's cached_text_parley
                // 3. Block-level children (h2, p, li, etc.)
                if let Some(ref inline_layout) = node.text_layout {
                    // IFC layout coords are relative to content box (inside padding+border)
                    let cs = &node.computed_style;
                    let pad_x =
                        (cs.padding_left.to_px() + cs.border_left_width.to_px()) as f64 * scale;
                    let pad_y =
                        (cs.padding_top.to_px() + cs.border_top_width.to_px()) as f64 * scale;
                    let text_x = x + pad_x - ce_scroll_x;
                    let text_y = y + pad_y - ce_scroll_y;
                    let text_len = inline_layout.text_content.len();
                    paint_contenteditable_cursor(
                        node,
                        scene,
                        scale,
                        text_x,
                        text_y,
                        &inline_layout.layout,
                        text_len,
                        cursor_pos.min(selection_start),
                        cursor_pos.max(selection_start),
                        Some(cursor_pos),
                        content_width,
                        node_transform,
                    );
                } else {
                    // Check for single text child first
                    let mut handled = false;
                    for &child_id in &node.children {
                        if let Some(child) = tree.nodes.get(child_id)
                            && let Some(ref cached_layout) = child.cached_text_parley
                        {
                            let text_len = child.text_content().map(|s| s.len()).unwrap_or(0);
                            // child.layout positions already account for parent padding
                            let text_x = x + child.layout.x as f64 * scale - ce_scroll_x;
                            let text_y = y + child.layout.y as f64 * scale - ce_scroll_y;
                            paint_contenteditable_cursor(
                                node,
                                scene,
                                scale,
                                text_x,
                                text_y,
                                cached_layout,
                                text_len,
                                cursor_pos.min(selection_start),
                                cursor_pos.max(selection_start),
                                Some(cursor_pos),
                                content_width,
                                node_transform,
                            );
                            handled = true;
                            break;
                        }
                    }

                    // Empty CE root — no children, no text layout.
                    // Draw a caret at the content-box origin.
                    if !handled && node.children.is_empty() {
                        if cursor_pos == 0 {
                            let cs = &node.computed_style;
                            let pad_x = (cs.padding_left.to_px() + cs.border_left_width.to_px())
                                as f64
                                * scale;
                            let pad_y = (cs.padding_top.to_px() + cs.border_top_width.to_px())
                                as f64
                                * scale;
                            let font_size = cs.font_size;
                            let line_h = match cs.line_height {
                                LineHeightValue::Relative(r) => font_size * r,
                                LineHeightValue::Absolute(a) => a,
                                LineHeightValue::Normal => font_size * 1.2,
                            };
                            let caret_height = line_h as f64 * scale;
                            let caret_color = cs
                                .color
                                .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(33, 37, 41, 255));
                            let caret_rect = Rect::new(
                                x + pad_x,
                                y + pad_y,
                                x + pad_x + 1.5 * scale,
                                y + pad_y + caret_height,
                            );
                            scene.fill(
                                Fill::NonZero,
                                node_transform,
                                caret_color,
                                None,
                                &caret_rect,
                            );
                        }
                        handled = true;
                    }

                    // Block-level children: walk children accumulating text offsets
                    // Renders selection across ALL blocks in the selection range
                    if !handled {
                        let sel_min = cursor_pos.min(selection_start);
                        let sel_max = cursor_pos.max(selection_start);
                        let mut accumulated = 0usize;
                        let mut first_block = true;

                        for &child_id in &node.children {
                            if let Some(child) = tree.nodes.get(child_id) {
                                let child_text_len = get_flat_text_len(tree, child_id);

                                // Account for newline separator between blocks
                                if !first_block {
                                    accumulated += 1; // \n
                                }
                                first_block = false;

                                let block_end = accumulated + child_text_len;

                                // Check if this block overlaps the selection range or contains cursor
                                let has_cursor =
                                    cursor_pos >= accumulated && cursor_pos <= block_end;
                                let in_selection = sel_min < block_end
                                    && sel_max > accumulated
                                    && sel_min != sel_max;

                                if has_cursor || in_selection {
                                    // Compute local selection range within this block
                                    let local_sel_start = if in_selection {
                                        sel_min.max(accumulated) - accumulated
                                    } else {
                                        0
                                    };
                                    let local_sel_end = if in_selection {
                                        sel_max.min(block_end) - accumulated
                                    } else {
                                        0
                                    };
                                    let caret = if has_cursor {
                                        Some(cursor_pos - accumulated)
                                    } else {
                                        None
                                    };

                                    let child_pad_x =
                                        child.computed_style.padding_left.to_px() as f64 * scale;
                                    let child_pad_y =
                                        child.computed_style.padding_top.to_px() as f64 * scale;
                                    let child_x = x + child.layout.x as f64 * scale + child_pad_x
                                        - ce_scroll_x;
                                    let child_y = y + child.layout.y as f64 * scale + child_pad_y
                                        - ce_scroll_y;

                                    // Try this child's IFC layout
                                    if let Some(ref inline_layout) = child.text_layout {
                                        paint_contenteditable_cursor(
                                            node,
                                            scene,
                                            scale,
                                            child_x,
                                            child_y,
                                            &inline_layout.layout,
                                            child_text_len,
                                            local_sel_start.min(child_text_len),
                                            local_sel_end.min(child_text_len),
                                            caret.map(|c| c.min(child_text_len)),
                                            content_width,
                                            node_transform,
                                        );
                                    } else {
                                        // Try child's text children
                                        let mut found_gc = false;
                                        for &grandchild_id in &child.children {
                                            if let Some(grandchild) = tree.nodes.get(grandchild_id)
                                                && let Some(ref cached_layout) =
                                                    grandchild.cached_text_parley
                                            {
                                                let gc_text_len = grandchild
                                                    .text_content()
                                                    .map(|s| s.len())
                                                    .unwrap_or(0);
                                                paint_contenteditable_cursor(
                                                    node,
                                                    scene,
                                                    scale,
                                                    child_x,
                                                    child_y,
                                                    cached_layout,
                                                    gc_text_len,
                                                    local_sel_start.min(gc_text_len),
                                                    local_sel_end.min(gc_text_len),
                                                    caret.map(|c| c.min(gc_text_len)),
                                                    content_width,
                                                    node_transform,
                                                );
                                                found_gc = true;
                                                break;
                                            }
                                        }
                                        if !found_gc {
                                            if child.children.is_empty() {
                                                // Empty block — draw a simple caret
                                                if caret.is_some() {
                                                    let cs = &child.computed_style;
                                                    let font_size = cs.font_size;
                                                    let line_h = match cs.line_height {
                                                        LineHeightValue::Relative(r) => {
                                                            font_size * r
                                                        }
                                                        LineHeightValue::Absolute(a) => a,
                                                        LineHeightValue::Normal => font_size * 1.2,
                                                    };
                                                    let caret_height = line_h as f64 * scale;
                                                    let caret_color =
                                                        cs.color.unwrap_or_else(|| {
                                                            AlphaColor::<Srgb>::from_rgba8(
                                                                33, 37, 41, 255,
                                                            )
                                                        });
                                                    let caret_rect = Rect::new(
                                                        child_x,
                                                        child_y,
                                                        child_x + 1.5 * scale,
                                                        child_y + caret_height,
                                                    );
                                                    scene.fill(
                                                        Fill::NonZero,
                                                        node_transform,
                                                        caret_color,
                                                        None,
                                                        &caret_rect,
                                                    );
                                                }
                                            } else {
                                                // Recurse into sub-blocks (ul > li, etc.)
                                                paint_ce_sub_blocks(
                                                    tree,
                                                    node,
                                                    scene,
                                                    scale,
                                                    x + child.layout.x as f64 * scale - ce_scroll_x,
                                                    y + child.layout.y as f64 * scale - ce_scroll_y,
                                                    &child.children,
                                                    accumulated,
                                                    cursor_pos,
                                                    sel_min,
                                                    sel_max,
                                                    content_width,
                                                    node_transform,
                                                );
                                            }
                                        }
                                    }
                                }

                                accumulated = block_end;
                            }
                        }
                    }
                }
            }

            // Check if this is an IFC root with a cached inline layout
            if let Some(inline_layout) = &node.text_layout {
                // Paint inline content at the content-box origin (inside padding+border),
                // accounting for scroll offset.
                let cs = &node.computed_style;
                let scroll_x = node.scroll_offset.0 * scale;
                let scroll_y = node.scroll_offset.1 * scale;
                let content_x = x
                    + (cs.padding_left.to_px() + cs.border_left_width.to_px()) as f64 * scale
                    - scroll_x;
                let content_y = y
                    + (cs.padding_top.to_px() + cs.border_top_width.to_px()) as f64 * scale
                    - scroll_y;
                let ifc_text_shadows = node.computed_style.text_shadow.as_slice();
                paint_inline_layout(
                    tree,
                    scene,
                    scale,
                    content_x,
                    content_y,
                    inline_layout,
                    font_cx,
                    layout_cx,
                    ifc_text_shadows,
                    node_transform,
                );

                // Still paint non-inline (block) children normally
                let child_ids = sorted_paint_order(tree, &node.children);
                for child_id in child_ids {
                    let child = match tree.get(child_id) {
                        Some(c) => c,
                        None => continue,
                    };
                    // Skip inline children — they're painted via the Parley layout
                    if child.ifc_root.is_some() {
                        continue;
                    }
                    paint_node(
                        tree,
                        child_id,
                        scene,
                        scale,
                        x - scroll_x,
                        y - scroll_y,
                        font_cx,
                        layout_cx,
                        node_transform,
                    );
                }
            } else {
                // Normal paint path: recurse into all children
                let scroll_x = node.scroll_offset.0 * scale;
                let scroll_y = node.scroll_offset.1 * scale;
                let child_ids = sorted_paint_order(tree, &node.children);
                for child_id in child_ids {
                    paint_node(
                        tree,
                        child_id,
                        scene,
                        scale,
                        x - scroll_x,
                        y - scroll_y,
                        font_cx,
                        layout_cx,
                        node_transform,
                    );
                }
            }

            if clips {
                scene.pop_layer();
            }

            // Paint scrollbar overlay for scroll containers
            if matches!(overflow_y, OverflowValue::Scroll | OverflowValue::Auto) {
                let node = tree.get(node_id).unwrap(); // re-borrow after children done
                let mut content_height: f64 = 0.0;
                for &child_id in &node.children {
                    if let Some(child) = tree.get(child_id) {
                        let bottom = (child.layout.y + child.layout.height) as f64 * scale;
                        if bottom > content_height {
                            content_height = bottom;
                        }
                    }
                }
                // Visible content area = layout height minus padding and border
                let cs = &node.computed_style;
                let pad_v = (cs.padding_top.to_px() + cs.padding_bottom.to_px()) as f64 * scale;
                let border_v =
                    (cs.border_top_width.to_px() + cs.border_bottom_width.to_px()) as f64 * scale;
                let visible_h = (h - pad_v - border_v).max(0.0);
                if content_height > visible_h {
                    let scrollbar_width = 6.0 * scale;
                    let scrollbar_margin = 2.0 * scale;
                    let scrollbar_x = x + w - scrollbar_width - scrollbar_margin;

                    // Thumb sizing
                    let visible_ratio = visible_h / content_height;
                    let max_scroll = content_height - visible_h;
                    let scroll_ratio = if max_scroll > 0.0 {
                        (node.scroll_offset.1 * scale / max_scroll).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };

                    let track_height = h - scrollbar_margin * 2.0;
                    let thumb_height = (track_height * visible_ratio).max(20.0 * scale);
                    let thumb_travel = track_height - thumb_height;
                    let thumb_y = y + scrollbar_margin + thumb_travel * scroll_ratio;

                    let thumb_rect = RoundedRect::from_rect(
                        Rect::new(
                            scrollbar_x,
                            thumb_y,
                            scrollbar_x + scrollbar_width,
                            thumb_y + thumb_height,
                        ),
                        scrollbar_width * 0.5,
                    );
                    let thumb_color = AlphaColor::<Srgb>::new([0.0, 0.0, 0.0, 0.4_f32]);
                    scene.fill(
                        Fill::NonZero,
                        node_transform,
                        &Brush::Solid(thumb_color),
                        None,
                        &thumb_rect,
                    );
                }
            }

            // Apply CSS filter approximations (after content is painted, before opacity pop)
            let cs = &tree.get(node_id).unwrap().computed_style;
            let has_filter = cs.filter_brightness != 1.0 || cs.filter_grayscale > 0.0;

            if has_filter {
                // Brightness: overlay black (darken) or white (brighten) with calculated alpha
                if cs.filter_brightness != 1.0 {
                    let brightness = cs.filter_brightness;
                    if brightness < 1.0 {
                        // Darken: overlay black with alpha = 1.0 - brightness
                        let alpha = ((1.0 - brightness).clamp(0.0, 1.0) * 255.0) as u8;
                        let dark = AlphaColor::<Srgb>::from_rgba8(0, 0, 0, alpha);
                        if radius > 0.0 {
                            let rrect = RoundedRect::from_rect(rect, radius);
                            scene.fill(Fill::NonZero, node_transform, dark, None, &rrect);
                        } else {
                            scene.fill(Fill::NonZero, node_transform, dark, None, &rect);
                        }
                    } else if brightness > 1.0 {
                        // Brighten: overlay white with alpha proportional to excess brightness
                        // brightness=2.0 → fully white, so alpha = (brightness - 1.0) clamped
                        let alpha = ((brightness - 1.0).clamp(0.0, 1.0) * 255.0) as u8;
                        let light = AlphaColor::<Srgb>::from_rgba8(255, 255, 255, alpha);
                        if radius > 0.0 {
                            let rrect = RoundedRect::from_rect(rect, radius);
                            scene.fill(Fill::NonZero, node_transform, light, None, &rrect);
                        } else {
                            scene.fill(Fill::NonZero, node_transform, light, None, &rect);
                        }
                    }
                }

                // Grayscale approximation: overlay a semi-transparent gray matching average luminance
                // This is a rough approximation — proper grayscale needs color matrix support
                // For now, we apply a desaturation effect by overlaying gray at the grayscale intensity
                if cs.filter_grayscale > 0.0 {
                    // Use Mix::Saturation blend mode if available, otherwise skip
                    // Vello's push_layer supports peniko::Mix blend modes
                    // Mix::Saturation would desaturate the content underneath
                    let grayscale = cs.filter_grayscale.clamp(0.0, 1.0);
                    // Push a saturation layer: gray rect with Saturation blend at grayscale alpha
                    scene.push_layer(
                        Fill::NonZero,
                        peniko::Mix::Saturation,
                        grayscale,
                        node_transform,
                        &rect,
                    );
                    // Fill with neutral gray
                    let gray = AlphaColor::<Srgb>::from_rgba8(128, 128, 128, 255);
                    if radius > 0.0 {
                        let rrect = RoundedRect::from_rect(rect, radius);
                        scene.fill(Fill::NonZero, node_transform, gray, None, &rrect);
                    } else {
                        scene.fill(Fill::NonZero, node_transform, gray, None, &rect);
                    }
                    scene.pop_layer();
                }
            }

            if has_opacity {
                scene.pop_layer();
            }
        }

        NodeKind::Text(text_data) => {
            if text_data.content.is_empty() {
                return;
            }

            // Check visibility (inherited from parent)
            let parent_visibility = node
                .parent
                .and_then(|p| tree.get(p))
                .map(|p| &p.computed_style.visibility);
            if matches!(
                parent_visibility,
                Some(VisibilityValue::Hidden | VisibilityValue::Collapse)
            ) {
                return;
            }

            // Use cached layout if available (built after Taffy layout with final widths)
            if let Some(cached_layout) = &node.cached_text_parley {
                // Layout is already aligned during caching, use it directly
                let text_shadows = node
                    .parent
                    .and_then(|p| tree.get(p))
                    .map(|p| p.computed_style.text_shadow.as_slice())
                    .unwrap_or(&[]);
                render_text_with_shadow(scene, cached_layout, x, y, text_shadows, parent_transform);
                return;
            }

            // Fallback: build layout on demand (should not happen with caching)
            let parent_computed = node
                .parent
                .and_then(|p| tree.get(p))
                .map(|p| &p.computed_style);

            let font_size = parent_computed.map(|s| s.font_size).unwrap_or(16.0);
            let font_weight = parent_computed.map(|s| s.font_weight).unwrap_or(400.0);
            let font_family = parent_computed
                .map(|s| {
                    if s.font_family.is_empty() {
                        "sans-serif".to_string()
                    } else {
                        s.font_family.clone()
                    }
                })
                .unwrap_or_else(|| "sans-serif".to_string());

            let color = parent_computed
                .and_then(|s| s.color)
                .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255));

            // Build Parley layout with identical parameters to measurement
            let scaled_font_size = font_size * scale as f32;
            let mut builder = layout_cx.ranged_builder(font_cx, &text_data.content, 1.0, true);
            builder.push_default(parley::style::StyleProperty::FontSize(scaled_font_size));
            builder.push_default(parley::style::StyleProperty::Brush(Brush::Solid(color)));
            builder.push_default(parley::style::StyleProperty::FontStack(
                parley::style::FontStack::Source(std::borrow::Cow::Owned(font_family)),
            ));
            if (font_weight - 400.0).abs() > 1.0 {
                builder.push_default(parley::style::StyleProperty::FontWeight(
                    parley::style::FontWeight::new(font_weight),
                ));
            }
            if let Some(lh) = parent_computed.and_then(|s| s.line_height.to_parley()) {
                builder.push_default(parley::style::StyleProperty::LineHeight(lh));
            }

            let mut text_layout = builder.build(&text_data.content);

            // Text was measured without width constraint to get natural width.
            // Paint should use the same (no constraint) to avoid re-wrapping.
            // The parent element's width constrains positioning, not text wrapping.
            text_layout.break_all_lines(None);

            // Read text-align from parent's computed style
            let alignment = parent_computed
                .map(|s| s.text_align.to_parley())
                .unwrap_or(parley::layout::Alignment::Start);
            text_layout.align(alignment, parley::layout::AlignmentOptions::default());

            // Render text glyphs to scene
            let text_shadows = parent_computed
                .map(|s| s.text_shadow.as_slice())
                .unwrap_or(&[]);
            render_text_with_shadow(scene, &text_layout, x, y, text_shadows, parent_transform);
        }

        _ => {} // Document, Comment -- invisible
    }
}

