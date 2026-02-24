//! ContentEditable cursor/selection painting and input value rendering.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, Rect};
use peniko::{Brush, Fill};
use vello::Scene;

use crate::computed_style::LineHeightValue;
use crate::node::{Node, NodeTree};

use super::text::render_text_with_shadow;

/// Get a CSS style property value from inline styles.
///
/// Parse an inline style property from a node's style attribute.
/// Most properties should be read from `node.computed_style` directly.
#[allow(dead_code)]
pub(super) fn get_style_property(node: &Node, property: &str) -> Option<String> {
    // Check computed_style_str (used during style resolution)
    if !node.computed_style_str.is_empty() {
        for part in node.computed_style_str.split(';') {
            let part = part.trim();
            if let Some((key, value)) = part.split_once(':')
                && key.trim() == property
            {
                return Some(value.trim().to_string());
            }
        }
        return None;
    }

    // Fallback: parse inline style attribute directly
    if let Some(style_str) = node.attributes.get("style") {
        for part in style_str.split(';') {
            let part = part.trim();
            if let Some((key, value)) = part.split_once(':')
                && key.trim() == property
            {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

/// Parse a pixel value like "10px" or "10" to f32.
#[allow(dead_code)]
pub(super) fn parse_px(value: &str) -> Option<f32> {
    let v = value.trim().strip_suffix("px").unwrap_or(value.trim());
    v.parse().ok()
}

/// Paint the value of an input element.
#[allow(clippy::too_many_arguments)]
/// Paint the cursor/selection overlay for a contenteditable element.
///
/// Uses the IFC inline layout already built for the element to position
/// the caret and selection highlight. Cursor/selection byte offsets are
/// read from `data-ce-cursor` and `data-ce-selection-start` attributes.
/// Check if a tag name represents a block-level element.
pub(super) fn is_block_tag(tag: &str) -> bool {
    matches!(
        tag,
        "div"
            | "p"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "li"
            | "ul"
            | "ol"
            | "section"
            | "article"
            | "blockquote"
            | "pre"
            | "hr"
            | "table"
            | "tr"
            | "header"
            | "footer"
            | "main"
            | "nav"
            | "aside"
            | "figure"
            | "figcaption"
            | "details"
            | "summary"
    )
}

/// Compute the flat text length for a subtree, matching extract_text_content's logic.
/// This accounts for `\n` separators between block elements.
pub(super) fn get_flat_text_len(tree: &NodeTree, node_id: usize) -> usize {
    let mut len = 0usize;
    let mut ends_with_newline = false;
    collect_text_len_recursive(tree, node_id, &mut len, &mut ends_with_newline);
    // Strip trailing newline (matching extract_text_content)
    if ends_with_newline && len > 0 {
        len -= 1;
    }
    len
}

pub(super) fn collect_text_len_recursive(
    tree: &NodeTree,
    node_id: usize,
    len: &mut usize,
    ends_with_newline: &mut bool,
) {
    if let Some(node) = tree.nodes.get(node_id) {
        if let Some(t) = node.text_content() {
            *len += t.len();
            *ends_with_newline = t.ends_with('\n');
        } else if node.tag() == Some("br") {
            // <br> is inline, contributes 1 byte ("\n") — matches walk_for_global_offset
            *len += 1;
            *ends_with_newline = true;
        } else {
            let is_block = node.tag().map(is_block_tag).unwrap_or(false);
            if is_block && *len > 0 && !*ends_with_newline {
                *len += 1;
                *ends_with_newline = true;
            }
            for &child_id in &node.children {
                collect_text_len_recursive(tree, child_id, len, ends_with_newline);
            }
            // Empty block elements must reset ends_with_newline so consecutive
            // empty blocks each get a unique offset via their own separator.
            if is_block && node.children.is_empty() {
                *ends_with_newline = false;
            }
            if is_block && *len > 0 && !*ends_with_newline {
                *len += 1;
                *ends_with_newline = true;
            }
        }
    }
}

/// Render selection/cursor across sub-blocks (e.g. ul > li items).
/// `parent_x`/`parent_y` are the absolute position of the parent block element.
/// `children` are the sub-block node IDs. `parent_accumulated` is the global
/// text offset at the start of the parent block.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_ce_sub_blocks(
    tree: &NodeTree,
    ce_node: &Node,
    scene: &mut Scene,
    scale: f64,
    parent_x: f64,
    parent_y: f64,
    children: &[usize],
    parent_accumulated: usize,
    cursor_pos: usize,
    sel_min: usize,
    sel_max: usize,
    content_width: f64,
    transform: Affine,
) {
    let mut sub_acc = parent_accumulated;
    let mut sub_first = true;

    for &sub_id in children {
        if let Some(sub_node) = tree.nodes.get(sub_id) {
            let sub_text_len = get_flat_text_len(tree, sub_id);
            if !sub_first {
                sub_acc += 1; // \n separator
            }
            sub_first = false;

            let sub_end = sub_acc + sub_text_len;
            let has_cursor = cursor_pos >= sub_acc && cursor_pos <= sub_end;
            let in_selection = sel_min < sub_end && sel_max > sub_acc && sel_min != sel_max;

            if has_cursor || in_selection {
                let local_sel_start = if in_selection {
                    sel_min.max(sub_acc) - sub_acc
                } else {
                    0
                };
                let local_sel_end = if in_selection {
                    sel_max.min(sub_end) - sub_acc
                } else {
                    0
                };
                let caret = if has_cursor {
                    Some(cursor_pos - sub_acc)
                } else {
                    None
                };

                let sub_pad_x = sub_node.computed_style.padding_left.to_px() as f64 * scale;
                let sub_pad_y = sub_node.computed_style.padding_top.to_px() as f64 * scale;
                let sub_x = parent_x + sub_node.layout.x as f64 * scale + sub_pad_x;
                let sub_y = parent_y + sub_node.layout.y as f64 * scale + sub_pad_y;

                // Try sub-node's IFC layout
                if let Some(ref il) = sub_node.text_layout {
                    paint_contenteditable_cursor(
                        ce_node,
                        scene,
                        scale,
                        sub_x,
                        sub_y,
                        &il.layout,
                        sub_text_len,
                        local_sel_start.min(sub_text_len),
                        local_sel_end.min(sub_text_len),
                        caret.map(|c| c.min(sub_text_len)),
                        content_width,
                        transform,
                    );
                } else if sub_node.children.is_empty() {
                    // Empty block — draw a simple caret
                    if caret.is_some() {
                        let cs = &sub_node.computed_style;
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
                        let caret_rect =
                            Rect::new(sub_x, sub_y, sub_x + 1.5 * scale, sub_y + caret_height);
                        scene.fill(Fill::NonZero, transform, caret_color, None, &caret_rect);
                    }
                } else {
                    // Try sub-node's text children
                    let mut found_text_child = false;
                    for &gc_id in &sub_node.children {
                        if let Some(gc) = tree.nodes.get(gc_id)
                            && let Some(ref cl) = gc.cached_text_parley
                        {
                            let gc_len = gc.text_content().map(|s| s.len()).unwrap_or(0);
                            paint_contenteditable_cursor(
                                ce_node,
                                scene,
                                scale,
                                sub_x,
                                sub_y,
                                cl,
                                gc_len,
                                local_sel_start.min(gc_len),
                                local_sel_end.min(gc_len),
                                caret.map(|c| c.min(gc_len)),
                                content_width,
                                transform,
                            );
                            found_text_child = true;
                            break;
                        }
                    }
                    // Recurse into deeper block structure (e.g., li > div + ul after indent)
                    if !found_text_child {
                        paint_ce_sub_blocks(
                            tree,
                            ce_node,
                            scene,
                            scale,
                            parent_x + sub_node.layout.x as f64 * scale,
                            parent_y + sub_node.layout.y as f64 * scale,
                            &sub_node.children,
                            sub_acc,
                            cursor_pos,
                            sel_min,
                            sel_max,
                            content_width,
                            transform,
                        );
                    }
                }
            }
            sub_acc = sub_end;
        }
    }
}

/// Render cursor and selection for a contenteditable element within a single layout.
///
/// `sel_start`/`sel_end` define the local selection range within this layout.
/// `caret_pos` is Some(offset) to draw the cursor caret, None to skip it.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_contenteditable_cursor(
    node: &Node,
    scene: &mut Scene,
    scale: f64,
    text_x: f64,
    text_y: f64,
    layout: &parley::layout::Layout<Brush>,
    text_len: usize,
    sel_start: usize,
    sel_end: usize,
    caret_pos: Option<usize>,
    content_width: f64,
    transform: Affine,
) {
    let font_size = node.computed_style.font_size;
    let scaled_font_size = font_size * scale as f32;
    let line_height_multiplier = match node.computed_style.line_height {
        LineHeightValue::Relative(r) => r as f64,
        LineHeightValue::Absolute(abs) => (abs / font_size) as f64,
        LineHeightValue::Normal => 1.2, // Default fallback
    };
    let line_height = scaled_font_size as f64 * line_height_multiplier;

    // Draw selection highlight if there's a selection range
    if sel_start != sel_end {
        let sel_start_byte = sel_start.min(text_len);
        let sel_end_byte = sel_end.min(text_len);

        let (start_x, start_y) =
            crate::text_query::caret_position_for_offset_layout(layout, sel_start_byte);
        let (end_x, end_y) =
            crate::text_query::caret_position_for_offset_layout(layout, sel_end_byte);

        let sel_color = AlphaColor::<Srgb>::from_rgba8(51, 154, 240, 100);

        if (start_y - end_y).abs() < 0.1 {
            // Same line
            let sel_rect = Rect::new(
                text_x + start_x as f64,
                text_y + start_y as f64,
                text_x + end_x as f64,
                text_y + start_y as f64 + line_height,
            );
            scene.fill(Fill::NonZero, transform, sel_color, None, &sel_rect);
        } else {
            // Multi-line selection: cursor.geometry().y0 returns the line BOX
            // top (including half-leading), not baseline - ascent (glyph top).
            // Compute line box bounds to match cursor geometry coordinates.
            for line in layout.lines() {
                let line_metrics = line.metrics();
                let glyph_top = line_metrics.baseline - line_metrics.ascent;
                // Line box top accounts for half-leading (space distributed
                // above and below glyphs when line-height > natural height)
                let half_leading =
                    (line_metrics.line_height - line_metrics.ascent - line_metrics.descent) / 2.0;
                let line_box_top = glyph_top - half_leading;
                let line_box_bottom = line_box_top + line_metrics.line_height;

                // Skip lines entirely outside the selection
                if line_box_bottom <= start_y || line_box_top > end_y + 0.5 {
                    continue;
                }

                // Check if this line contains the start/end positions
                let is_start_line = start_y >= line_box_top - 0.5 && start_y < line_box_bottom;
                let is_end_line = end_y >= line_box_top - 0.5 && end_y < line_box_bottom;

                let rect_y = text_y + glyph_top as f64;
                let (rect_start_x, rect_end_x) = if is_start_line && is_end_line {
                    // Both on same line (fallback for edge cases)
                    (start_x as f64, end_x as f64)
                } else if is_start_line {
                    (start_x as f64, content_width)
                } else if is_end_line {
                    (0.0, end_x as f64)
                } else {
                    // Middle line: full width
                    (0.0, content_width)
                };
                let sel_rect = Rect::new(
                    text_x + rect_start_x,
                    rect_y,
                    text_x + rect_end_x,
                    rect_y + line_height,
                );
                scene.fill(Fill::NonZero, transform, sel_color, None, &sel_rect);
            }
        }
    }

    // Draw cursor/caret only if requested
    if let Some(caret) = caret_pos {
        let caret_byte = caret.min(text_len);
        let (caret_offset_x, caret_offset_y) = if text_len == 0 {
            (0.0, 0.0)
        } else {
            crate::text_query::caret_position_for_offset_layout(layout, caret_byte)
        };
        let caret_x = text_x + caret_offset_x as f64;
        let caret_y = text_y + caret_offset_y as f64;
        let caret_height = line_height;

        let caret_color = node
            .computed_style
            .color
            .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(33, 37, 41, 255));
        let caret_rect = Rect::new(
            caret_x,
            caret_y,
            caret_x + 1.5 * scale,
            caret_y + caret_height,
        );
        scene.fill(Fill::NonZero, transform, caret_color, None, &caret_rect);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_input_value(
    node: &Node,
    scene: &mut Scene,
    scale: f64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
    transform: Affine,
) {
    // Get the value or placeholder
    let value = node
        .attributes
        .get("value")
        .map(|s| s.as_str())
        .unwrap_or("");
    let placeholder = node
        .attributes
        .get("placeholder")
        .map(|s| s.as_str())
        .unwrap_or("");

    // Check if this input is focused
    let is_focused = node
        .attributes
        .get("data-focused")
        .map(|s| s == "true")
        .unwrap_or(false);
    let cursor_pos = node
        .attributes
        .get("data-cursor-pos")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let selection_start = node
        .attributes
        .get("data-selection-start")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(cursor_pos);
    let cursor_visible = node
        .attributes
        .get("data-cursor-visible")
        .map(|s| s == "true")
        .unwrap_or(true);

    let (text, is_placeholder) = if value.is_empty() && !placeholder.is_empty() {
        (placeholder, true)
    } else {
        (value, false)
    };

    // Password masking: replace display text with bullets for type="password"
    let is_password = node.attributes.get("type").map(|s| s.as_str()) == Some("password");
    let password_display;
    let (text, cursor_pos, selection_start) = if is_password && !is_placeholder && !text.is_empty()
    {
        let bullet = "\u{2022}";
        let bullet_len = bullet.len();
        let total_chars = value.chars().count();
        let cursor_chars = value[..cursor_pos.min(value.len())].chars().count();
        let sel_chars = value[..selection_start.min(value.len())].chars().count();
        password_display = bullet.repeat(total_chars);
        (
            password_display.as_str(),
            cursor_chars * bullet_len,
            sel_chars * bullet_len,
        )
    } else {
        (text, cursor_pos, selection_start)
    };

    // Get font properties from computed style
    let font_size = node.computed_style.font_size;
    let font_weight = node.computed_style.font_weight;
    let font_family = if node.computed_style.font_family.is_empty() {
        "sans-serif".to_string()
    } else {
        node.computed_style.font_family.clone()
    };

    // Get text color from computed style - dimmed for placeholder
    let base_color = node
        .computed_style
        .color
        .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(33, 37, 41, 255)); // #212529

    let color = if is_placeholder {
        // Placeholder uses dimmed color
        AlphaColor::<Srgb>::from_rgba8(134, 142, 150, 255) // #868e96
    } else {
        base_color
    };

    // Get padding from computed style
    let padding_left = node.computed_style.padding_left.to_px() as f64 * scale;
    let padding_top = node.computed_style.padding_top.to_px() as f64 * scale;

    // Check if this is a textarea (multi-line) or input (single-line)
    let is_textarea = node.tag() == Some("textarea");

    // Build text layout
    let scaled_font_size = font_size * scale as f32;
    let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
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

    let mut text_layout = builder.build(text);
    text_layout.break_all_lines(Some((w - padding_left * 2.0) as f32));

    // For textarea, align to top with padding; for input, center vertically
    let text_height = text_layout.height() as f64;
    let text_y = if is_textarea {
        y + padding_top
    } else {
        y + (h - text_height) / 2.0
    };
    let text_x = x + padding_left;

    // Draw selection highlight if there's a selection
    if is_focused && cursor_pos != selection_start && !is_placeholder && !text.is_empty() {
        let sel_start_byte = cursor_pos.min(selection_start).min(text.len());
        let sel_end_byte = cursor_pos.max(selection_start).min(text.len());

        let (start_x, start_y) =
            crate::text_query::caret_position_for_offset_layout(&text_layout, sel_start_byte);
        let (end_x, end_y) =
            crate::text_query::caret_position_for_offset_layout(&text_layout, sel_end_byte);

        let sel_color = AlphaColor::<Srgb>::from_rgba8(51, 154, 240, 100); // Blue with alpha
        let line_height = scaled_font_size as f64 * 1.2;

        if (start_y - end_y).abs() < 0.1 {
            // Same line - draw single rectangle
            let sel_x = text_x + start_x as f64;
            let sel_width = (end_x - start_x) as f64;
            let sel_y = text_y + start_y as f64;

            let sel_rect =
                vello::kurbo::Rect::new(sel_x, sel_y, sel_x + sel_width, sel_y + line_height);
            scene.fill(
                vello::peniko::Fill::NonZero,
                transform,
                sel_color,
                None,
                &sel_rect,
            );
        } else {
            // Multi-line selection - draw rectangles for each line
            let content_width = (w - padding_left * 2.0) as f32;

            for line in text_layout.lines() {
                let line_metrics = line.metrics();
                let line_top = line_metrics.baseline - line_metrics.ascent;

                // Skip lines outside selection range
                if line_top + line_metrics.line_height < start_y || line_top > end_y {
                    continue;
                }

                let rect_y = text_y + line_top as f64;
                let (rect_start_x, rect_end_x) = if (line_top - start_y).abs() < 0.1 {
                    // First line of selection: from start_x to end of line
                    (start_x, content_width)
                } else if (line_top - end_y).abs() < 0.1 {
                    // Last line of selection: from start of line to end_x
                    (0.0, end_x)
                } else {
                    // Middle line: full width
                    (0.0, content_width)
                };

                let sel_rect = vello::kurbo::Rect::new(
                    text_x + rect_start_x as f64,
                    rect_y,
                    text_x + rect_end_x as f64,
                    rect_y + line_height,
                );
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    transform,
                    sel_color,
                    None,
                    &sel_rect,
                );
            }
        }
    }

    // Render text
    if !text.is_empty() {
        let text_shadows = node.computed_style.text_shadow.as_slice();
        render_text_with_shadow(scene, &text_layout, text_x, text_y, text_shadows, transform);
    }

    // Draw cursor/caret if focused and visible
    if is_focused && cursor_visible {
        let caret_pos = cursor_pos.min(text.len());
        let (caret_offset_x, caret_offset_y) = if text.is_empty() {
            (0.0, 0.0)
        } else {
            crate::text_query::caret_position_for_offset_layout(&text_layout, caret_pos)
        };
        let caret_x = text_x + caret_offset_x as f64;
        let caret_y = text_y + caret_offset_y as f64;

        let caret_height = scaled_font_size as f64 * 1.2;

        // Draw caret line
        let caret_color = base_color;
        let caret_rect = vello::kurbo::Rect::new(
            caret_x,
            caret_y,
            caret_x + 1.5 * scale,
            caret_y + caret_height,
        );
        scene.fill(
            vello::peniko::Fill::NonZero,
            transform,
            caret_color,
            None,
            &caret_rect,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_px() {
        assert_eq!(parse_px("10px"), Some(10.0));
        assert_eq!(parse_px("10"), Some(10.0));
        assert_eq!(parse_px("0"), Some(0.0));
        assert_eq!(parse_px("abc"), None);
    }
}
