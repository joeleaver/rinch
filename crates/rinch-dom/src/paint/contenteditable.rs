//! `<input>`/`<textarea>` value rendering (text, placeholder, caret, selection,
//! and IME preedit), the read-only text-selection highlight (`user-select: text`),
//! plus the small style/px helpers they share.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, Rect};
use peniko::{Brush, Fill};

use super::painter::Painter;
use crate::computed_style::LineHeightValue;
use crate::node::Node;

use super::text::render_text_with_shadow;

/// Find the line height from a Parley layout for the line containing the given y
/// position (layout-relative). Returns Parley's reported line height for that
/// line, which reflects the actual font metrics of the laid-out text.
fn line_height_at_y(layout: &parley::layout::Layout<Brush>, y: f32) -> Option<f64> {
    for line in layout.lines() {
        let m = line.metrics();
        let glyph_top = m.baseline - m.ascent;
        let half_leading = (m.line_height - m.ascent - m.descent) / 2.0;
        let line_box_top = glyph_top - half_leading;
        let line_box_bottom = line_box_top + m.line_height;
        if y >= line_box_top - 0.5 && y < line_box_bottom + 0.5 {
            return Some(m.line_height as f64);
        }
    }
    // If y doesn't match any line (e.g. empty layout), try the first line.
    layout
        .lines()
        .next()
        .map(|line| line.metrics().line_height as f64)
}

/// Paint the translucent selection highlight for a **read-only** text selection
/// (`user-select: text`) over a block's inline `layout`. `sel_start`/`sel_end`
/// are byte offsets into the laid-out text; `text_x`/`text_y` are the content-box
/// origin. (The editor and `<input>` render their own selections elsewhere; this
/// is only for selecting static, non-editable text.)
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_text_selection_highlight(
    node: &Node,
    painter: &mut dyn Painter,
    scale: f64,
    text_x: f64,
    text_y: f64,
    layout: &parley::layout::Layout<Brush>,
    text_len: usize,
    sel_start: usize,
    sel_end: usize,
    content_width: f64,
    transform: Affine,
) {
    if sel_start == sel_end {
        return;
    }

    // Parley's line metrics are authoritative (the layout may belong to a child
    // block with its own font size); fall back to computed style if needed.
    let fallback_line_height = {
        let font_size = node.computed_style.font_size;
        let scaled = font_size * scale as f32;
        let mult = match node.computed_style.line_height {
            LineHeightValue::Relative(r) => r as f64,
            LineHeightValue::Absolute(abs) => (abs / font_size) as f64,
            LineHeightValue::Normal => 1.2,
        };
        scaled as f64 * mult
    };

    let sel_start_byte = sel_start.min(text_len);
    let sel_end_byte = sel_end.min(text_len);

    let (start_x, start_y) =
        crate::text_query::caret_position_for_offset_layout(layout, sel_start_byte);
    let (end_x, end_y) = crate::text_query::caret_position_for_offset_layout(layout, sel_end_byte);

    let sel_color = AlphaColor::<Srgb>::from_rgba8(51, 154, 240, 100);

    if (start_y - end_y).abs() < 0.1 {
        // Same line — find the matching line's actual height from Parley.
        let actual_line_height = line_height_at_y(layout, start_y).unwrap_or(fallback_line_height);
        let sel_rect = Rect::new(
            text_x + start_x as f64,
            text_y + start_y as f64,
            text_x + end_x as f64,
            text_y + start_y as f64 + actual_line_height,
        );
        painter.fill_color(Fill::NonZero, transform, sel_color, &sel_rect.into());
    } else {
        // Multi-line selection.
        for line in layout.lines() {
            let line_metrics = line.metrics();
            let glyph_top = line_metrics.baseline - line_metrics.ascent;
            let half_leading =
                (line_metrics.line_height - line_metrics.ascent - line_metrics.descent) / 2.0;
            let line_box_top = glyph_top - half_leading;
            let line_box_bottom = line_box_top + line_metrics.line_height;

            // Skip lines entirely outside the selection.
            if line_box_bottom <= start_y || line_box_top > end_y + 0.5 {
                continue;
            }

            let is_start_line = start_y >= line_box_top - 0.5 && start_y < line_box_bottom;
            let is_end_line = end_y >= line_box_top - 0.5 && end_y < line_box_bottom;

            let rect_y = text_y + line_box_top as f64;
            let (rect_start_x, rect_end_x) = if is_start_line && is_end_line {
                (start_x as f64, end_x as f64)
            } else if is_start_line {
                (start_x as f64, content_width)
            } else if is_end_line {
                (0.0, end_x as f64)
            } else {
                (0.0, content_width)
            };
            let sel_rect = Rect::new(
                text_x + rect_start_x,
                rect_y,
                text_x + rect_end_x,
                rect_y + line_metrics.line_height as f64,
            );
            painter.fill_color(Fill::NonZero, transform, sel_color, &sel_rect.into());
        }
    }
}

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

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_input_value(
    node: &Node,
    painter: &mut dyn Painter,
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

    let is_password = node.attributes.get("type").map(|s| s.as_str()) == Some("password");

    // IME composition (preedit): the in-progress composition string lives only in
    // `data-preedit` (written by the runtime during composition) — never in the
    // field's `value`. Splice it into the displayed text at the caret so it flows
    // inline, underline that run, and put the caret at its end. Composition takes
    // precedence over the placeholder (composing into an empty field shows the
    // composition, not the placeholder).
    let preedit = node
        .attributes
        .get("data-preedit")
        .map(|s| s.as_str())
        .unwrap_or("");
    let composing = is_focused && !preedit.is_empty() && !is_password;
    let composed_text;
    let (text, is_placeholder, cursor_pos, selection_start, preedit_range) = if composing {
        let c = cursor_pos.min(value.len());
        composed_text = format!("{}{}{}", &value[..c], preedit, &value[c..]);
        let end = c + preedit.len();
        (composed_text.as_str(), false, end, end, Some((c, end)))
    } else if value.is_empty() && !placeholder.is_empty() {
        (placeholder, true, cursor_pos, selection_start, None)
    } else {
        (value, false, cursor_pos, selection_start, None)
    };

    // Password masking: replace display text with bullets for type="password"
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
    // Kept glyph-for-glyph in step with `RinchApp::compute_input_cursor_from_click`
    // (crates/rinch/src/app/click_handling.rs) — a property added here and not
    // there moves the painted text out from under the caret. Both should move to
    // `ComputedStyle::build_parley_layout` together (#320).
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

            let sel_rect = Rect::new(sel_x, sel_y, sel_x + sel_width, sel_y + line_height);
            painter.fill_color(Fill::NonZero, transform, sel_color, &sel_rect.into());
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

                let sel_rect = Rect::new(
                    text_x + rect_start_x as f64,
                    rect_y,
                    text_x + rect_end_x as f64,
                    rect_y + line_height,
                );
                painter.fill_color(Fill::NonZero, transform, sel_color, &sel_rect.into());
            }
        }
    }

    // Render text (layout already uses scaled_font_size, so pass scale=1.0)
    if !text.is_empty() {
        let text_shadows = node.computed_style.text_shadow.as_slice();
        render_text_with_shadow(
            painter,
            &text_layout,
            text_x,
            text_y,
            text_shadows,
            transform,
            1.0,
        );
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
        let caret_rect = Rect::new(
            caret_x,
            caret_y,
            caret_x + 1.5 * scale,
            caret_y + caret_height,
        );
        painter.fill_color(Fill::NonZero, transform, caret_color, &caret_rect.into());
    }

    // Underline the IME composition run so it reads as an in-progress composition.
    if let Some((a, b)) = preedit_range {
        let a = a.min(text.len());
        let b = b.min(text.len());
        let (start_x, start_y) =
            crate::text_query::caret_position_for_offset_layout(&text_layout, a);
        let (end_x, _end_y) = crate::text_query::caret_position_for_offset_layout(&text_layout, b);
        let line_height = scaled_font_size as f64 * 1.2;
        let underline_y = text_y + start_y as f64 + line_height - scale;
        let ul_rect = Rect::new(
            text_x + start_x as f64,
            underline_y,
            text_x + end_x as f64,
            underline_y + scale,
        );
        painter.fill_color(Fill::NonZero, transform, base_color, &ul_rect.into());
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
