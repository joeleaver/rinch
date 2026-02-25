//! Text rendering functions.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, Stroke};
use peniko::{Brush, Fill};
use vello::Scene;

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
    scene: &mut Scene,
    scale: f64,
    parent_x: f64,
    parent_y: f64,
    inline_layout: &crate::node::InlineLayout,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
    text_shadows: &[TextShadowValue],
    transform: Affine,
) {
    // Render the Parley layout at the IFC root's position
    // Scale is already applied to font sizes during layout building
    render_text_with_shadow(
        scene,
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
                    tree, child_id, scene, scale, parent_x, parent_y, font_cx, layout_cx, transform,
                );
            }
        }
    }
}

/// Render a Parley text layout to a Vello scene.
pub(super) fn render_text(
    scene: &mut Scene,
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

            scene
                .draw_glyphs(font)
                .font_size(font_size)
                .transform(transform)
                .glyph_transform(glyph_xform)
                .brush(&brush)
                .hint(true)
                .normalized_coords(run.normalized_coords())
                .draw(
                    Fill::NonZero,
                    glyph_run.glyphs().map(|glyph| {
                        let px = gx + glyph.x;
                        let py = gy - glyph.y;
                        gx += glyph.advance;
                        vello::Glyph {
                            id: glyph.id,
                            x: px,
                            y: py,
                        }
                    }),
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
                scene.stroke(&stroke, transform, dec_brush, None, &line);
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
                scene.stroke(&stroke, transform, dec_brush, None, &line);
            }
        }
    }
}

/// Render a single shadow pass of a Parley text layout (glyphs only, no decorations).
///
/// Draws all glyph runs at the given offset with the specified shadow color,
/// ignoring the original brush from the layout styles.
pub(super) fn render_text_shadow_pass(
    scene: &mut Scene,
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

            scene
                .draw_glyphs(font)
                .font_size(font_size)
                .transform(transform)
                .glyph_transform(glyph_xform)
                .brush(&shadow_brush)
                .hint(true)
                .normalized_coords(run.normalized_coords())
                .draw(
                    Fill::NonZero,
                    glyph_run.glyphs().map(|glyph| {
                        let px = gx + glyph.x;
                        let py = gy - glyph.y;
                        gx += glyph.advance;
                        vello::Glyph {
                            id: glyph.id,
                            x: px,
                            y: py,
                        }
                    }),
                );
        }
    }
}

/// Render text with optional text-shadow effects.
///
/// Draws shadow passes (in reverse order so first shadow renders on top of later ones)
/// at the specified offsets, then draws the normal text on top.
pub(super) fn render_text_with_shadow(
    scene: &mut Scene,
    layout: &parley::layout::Layout<Brush>,
    x: f64,
    y: f64,
    text_shadows: &[TextShadowValue],
    css_transform: Affine,
) {
    if text_shadows.is_empty() {
        render_text(scene, layout, x, y, css_transform);
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
        render_text_shadow_pass(scene, layout, sx, sy, shadow_color, css_transform);
    }

    // Render the main text on top
    render_text(scene, layout, x, y, css_transform);
}
