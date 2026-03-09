//! Text rendering functions.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, Stroke};
use peniko::{Brush, Fill};

use super::painter::{PaintGlyph, Painter};
use crate::computed_style::TextShadowValue;
use crate::node::NodeTree;

use super::paint_node;

/// Paint inline content from a cached InlineLayout (IFC root).
///
/// Renders glyph runs directly from the Parley layout. Inline boxes
/// (inline-block elements) are painted by looking up the child node
/// and painting it at the Parley-computed position.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_inline_layout(
    tree: &NodeTree,
    painter: &mut dyn Painter,
    scale: f64,
    parent_x: f64,
    parent_y: f64,
    inline_layout: &crate::node::InlineLayout,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
    text_shadows: &[TextShadowValue],
    transform: Affine,
) {
    // Paint inline element backgrounds BEFORE text so text renders on top
    if !inline_layout.background_spans.is_empty() {
        paint_inline_backgrounds(painter, parent_x, parent_y, inline_layout, transform);
    }

    // Render the Parley layout at the IFC root's position
    // Scale is already applied to font sizes during layout building
    render_text_with_shadow(
        painter,
        &inline_layout.layout,
        parent_x,
        parent_y,
        text_shadows,
        transform,
    );

    // Paint inline-block boxes by looking them up in tree and painting
    for line in inline_layout.layout.lines() {
        for item in line.items() {
            if let parley::layout::PositionedLayoutItem::InlineBox(positioned_box) = item {
                let child_id = positioned_box.id as usize;
                paint_node(
                    tree, child_id, painter, scale, parent_x, parent_y, font_cx, layout_cx,
                    transform,
                );
            }
        }
    }
}

/// Paint background rectangles for inline elements (code, mark, etc.).
///
/// Uses Parley's cluster-based byte-to-position mapping to determine the
/// exact visual bounds of each inline background span. Handles multi-line
/// spans by iterating lines and computing per-line background rectangles.
fn paint_inline_backgrounds(
    painter: &mut dyn Painter,
    parent_x: f64,
    parent_y: f64,
    inline_layout: &crate::node::InlineLayout,
    css_transform: Affine,
) {
    use parley::layout::Cluster;
    use peniko::kurbo::{Rect, RoundedRect};

    let layout = &inline_layout.layout;
    let transform = css_transform * Affine::translate((parent_x, parent_y));

    for bg_span in &inline_layout.background_spans {
        let brush = Brush::Solid(bg_span.color);

        // Iterate lines to find those overlapping this background span
        for line in layout.lines() {
            let line_range = line.text_range();

            // Skip lines that don't overlap the background span
            if line_range.end <= bg_span.start || line_range.start >= bg_span.end {
                continue;
            }

            let line_metrics = line.metrics();
            let baseline = line_metrics.baseline as f64;
            let ascent = line_metrics.ascent as f64;
            let descent = line_metrics.descent as f64;

            // Clamp background span to this line's text range
            let overlap_start = bg_span.start.max(line_range.start);
            let overlap_end = bg_span.end.min(line_range.end);

            // Use Parley's cluster API for exact byte-to-position mapping
            let Some(sc) = Cluster::from_byte_index(layout, overlap_start) else {
                continue;
            };
            let end_byte = overlap_end.saturating_sub(1).max(overlap_start);
            let Some(ec) = Cluster::from_byte_index(layout, end_byte) else {
                continue;
            };

            let Some(x_start) = sc.visual_offset() else {
                continue;
            };
            let Some(ec_offset) = ec.visual_offset() else {
                continue;
            };
            let x_end = ec_offset + ec.advance();

            // Determine if this line segment gets padding
            let is_first_line = overlap_start == bg_span.start;
            let is_last_line = overlap_end == bg_span.end;
            let pl = if is_first_line {
                bg_span.padding_left as f64
            } else {
                0.0
            };
            let pr = if is_last_line {
                bg_span.padding_right as f64
            } else {
                0.0
            };
            let pt = bg_span.padding_top as f64;
            let pb = bg_span.padding_bottom as f64;

            let rect = Rect::new(
                x_start as f64 - pl,
                baseline - ascent - pt,
                x_end as f64 + pr,
                baseline + descent + pb,
            );

            if bg_span.border_radius > 0.0 {
                let r = bg_span.border_radius as f64;
                let rrect = RoundedRect::from_rect(rect, r);
                painter.fill(Fill::NonZero, transform, &brush, &rrect.into());
            } else {
                painter.fill(Fill::NonZero, transform, &brush, &rect.into());
            }
        }
    }
}

/// Render a Parley text layout using a Painter.
pub(super) fn render_text(
    painter: &mut dyn Painter,
    layout: &parley::layout::Layout<Brush>,
    x: f64,
    y: f64,
    css_transform: Affine,
) {
    let transform = css_transform * Affine::translate((x, y));
    for line in layout.lines() {
        for item in line.items() {
            let parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let mut gx = glyph_run.offset();
            let gy = glyph_run.baseline();
            let run = glyph_run.run();
            let font = run.font();
            let font_size = run.font_size();
            let synthesis = run.synthesis();
            let glyph_xform = synthesis
                .skew()
                .map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));
            let style = glyph_run.style();
            let brush = style.brush.clone();

            // Track run width for decorations
            let run_x = glyph_run.offset();

            let glyphs: Vec<PaintGlyph> = glyph_run
                .glyphs()
                .map(|glyph| {
                    let px = gx + glyph.x;
                    let py = gy - glyph.y;
                    gx += glyph.advance;
                    PaintGlyph {
                        id: glyph.id,
                        x: px,
                        y: py,
                    }
                })
                .collect();
            painter.draw_glyphs(
                font,
                font_size,
                transform,
                glyph_xform,
                &brush,
                true,
                run.normalized_coords(),
                &glyphs,
            );

            // Draw underline decoration
            if let Some(underline) = &style.underline {
                let run_metrics = run.metrics();
                let offset = underline.offset.unwrap_or(run_metrics.underline_offset);
                let size = underline.size.unwrap_or(run_metrics.underline_size);
                let dec_brush = &underline.brush;
                // underline_offset from font metrics is negative (below baseline),
                // so subtract it to move the line below the baseline.
                let line_y = (gy - offset) as f64;
                let run_width = (gx - run_x) as f64;
                let line = peniko::kurbo::Line::new(
                    (run_x as f64, line_y),
                    (run_x as f64 + run_width, line_y),
                );
                let stroke = Stroke::new(size.max(1.0) as f64);
                painter.stroke(&stroke, transform, dec_brush, &line.into());
            }

            // Draw strikethrough decoration
            if let Some(strikethrough) = &style.strikethrough {
                let run_metrics = run.metrics();
                let offset = strikethrough
                    .offset
                    .unwrap_or(run_metrics.strikethrough_offset);
                let size = strikethrough.size.unwrap_or(run_metrics.strikethrough_size);
                let dec_brush = &strikethrough.brush;
                let line_y = (gy - offset) as f64;
                let run_width = (gx - run_x) as f64;
                let line = peniko::kurbo::Line::new(
                    (run_x as f64, line_y),
                    (run_x as f64 + run_width, line_y),
                );
                let stroke = Stroke::new(size.max(1.0) as f64);
                painter.stroke(&stroke, transform, dec_brush, &line.into());
            }
        }
    }
}

/// Render a single shadow pass of a Parley text layout (glyphs only, no decorations).
///
/// Draws all glyph runs at the given offset with the specified shadow color,
/// ignoring the original brush from the layout styles.
pub(super) fn render_text_shadow_pass(
    painter: &mut dyn Painter,
    layout: &parley::layout::Layout<Brush>,
    x: f64,
    y: f64,
    shadow_color: AlphaColor<Srgb>,
    css_transform: Affine,
) {
    let transform = css_transform * Affine::translate((x, y));
    let shadow_brush = Brush::Solid(shadow_color);
    for line in layout.lines() {
        for item in line.items() {
            let parley::layout::PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let mut gx = glyph_run.offset();
            let gy = glyph_run.baseline();
            let run = glyph_run.run();
            let font = run.font();
            let font_size = run.font_size();
            let synthesis = run.synthesis();
            let glyph_xform = synthesis
                .skew()
                .map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));

            let glyphs: Vec<PaintGlyph> = glyph_run
                .glyphs()
                .map(|glyph| {
                    let px = gx + glyph.x;
                    let py = gy - glyph.y;
                    gx += glyph.advance;
                    PaintGlyph {
                        id: glyph.id,
                        x: px,
                        y: py,
                    }
                })
                .collect();
            painter.draw_glyphs(
                font,
                font_size,
                transform,
                glyph_xform,
                &shadow_brush,
                true,
                run.normalized_coords(),
                &glyphs,
            );
        }
    }
}

/// Render text with optional text-shadow effects.
///
/// Draws shadow passes (in reverse order so first shadow renders on top of later ones)
/// at the specified offsets, then draws the normal text on top.
pub(super) fn render_text_with_shadow(
    painter: &mut dyn Painter,
    layout: &parley::layout::Layout<Brush>,
    x: f64,
    y: f64,
    text_shadows: &[TextShadowValue],
    css_transform: Affine,
) {
    if text_shadows.is_empty() {
        render_text(painter, layout, x, y, css_transform);
        return;
    }

    // Render shadows in reverse order (first shadow = topmost, drawn last before main text)
    for shadow in text_shadows.iter().rev() {
        let shadow_color = shadow.color.unwrap_or_else(|| {
            AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255) // default: black
        });
        let sx = x + shadow.offset_x as f64;
        let sy = y + shadow.offset_y as f64;
        // Note: blur_radius is ignored for now (Vello lacks a blur API)
        render_text_shadow_pass(painter, layout, sx, sy, shadow_color, css_transform);
    }

    // Render the main text on top
    render_text(painter, layout, x, y, css_transform);
}
