//! Closed `<select>` control rendering: the selected option's label plus a
//! dropdown arrow. The interactive popup lives in the app/shell layer (issue
//! #121); this only draws the resting, closed control so a `<select>` looks like
//! a real form control instead of a run of stacked option text.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, BezPath, Point};
use peniko::{Brush, Fill};

use super::painter::Painter;
use super::text::render_text_with_shadow;
use crate::node::{NodeTree, RawNodeId};
use crate::select::resolve_select_model;

/// Width reserved on the right of the control for the dropdown arrow. Matches the
/// `24px` right padding the UA stylesheet gives `<select>`.
const ARROW_BOX: f64 = 24.0;

/// Paint the closed `<select>`: the selected option's label, clipped to the
/// content box, plus a chevron arrow on the right.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_select_value(
    tree: &NodeTree,
    node_id: RawNodeId,
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
    let Some(node) = tree.get(node_id) else {
        return;
    };

    let model = resolve_select_model(tree, node_id);

    let padding_left = node.computed_style.padding_left.to_px() as f64 * scale;
    let padding_right = node.computed_style.padding_right.to_px() as f64 * scale;

    let base_color = node
        .computed_style
        .color
        .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(33, 37, 41, 255)); // #212529

    // Draw the arrow first so it always shows, even for an empty <select>.
    paint_arrow(
        painter,
        scale,
        x,
        y,
        w,
        h,
        padding_right,
        base_color,
        transform,
    );

    // A single <select> with options always has a selected label; an empty
    // <select> has nothing to show.
    let Some(label) = model.selected_label().filter(|s| !s.is_empty()) else {
        return;
    };

    let font_size = node.computed_style.font_size;
    let font_weight = node.computed_style.font_weight;
    let font_family = if node.computed_style.font_family.is_empty() {
        "sans-serif".to_string()
    } else {
        node.computed_style.font_family.clone()
    };

    let scaled_font_size = font_size * scale as f32;
    let mut builder = layout_cx.ranged_builder(font_cx, label, 1.0, true);
    builder.push_default(parley::style::StyleProperty::FontSize(scaled_font_size));
    builder.push_default(parley::style::StyleProperty::Brush(Brush::Solid(
        base_color,
    )));
    builder.push_default(parley::style::StyleProperty::FontStack(
        parley::style::FontStack::Source(std::borrow::Cow::Owned(font_family)),
    ));
    if (font_weight - 400.0).abs() > 1.0 {
        builder.push_default(parley::style::StyleProperty::FontWeight(
            parley::style::FontWeight::new(font_weight),
        ));
    }

    let mut text_layout = builder.build(label);
    // Single line, clipped: never wrap the closed control's label.
    text_layout.break_all_lines(None);

    let text_height = text_layout.height() as f64;
    let text_x = x + padding_left;
    let text_y = y + (h - text_height) / 2.0;

    // Clip the label to the content box (control width minus the arrow area) so a
    // long option ellipsis-style clips instead of running under the arrow.
    let arrow_w = padding_right.max(ARROW_BOX * scale);
    let content_right = (x + w - arrow_w).max(text_x);
    let clip_rect = peniko::kurbo::Rect::new(text_x, y, content_right, y + h);
    painter.push_clip(Fill::NonZero, transform, &clip_rect.into());

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

    painter.pop_layer();
}

/// Draw the downward chevron in the arrow box on the right of the control.
#[allow(clippy::too_many_arguments)]
fn paint_arrow(
    painter: &mut dyn Painter,
    scale: f64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    padding_right: f64,
    color: AlphaColor<Srgb>,
    transform: Affine,
) {
    let arrow_w = padding_right.max(ARROW_BOX * scale);
    // Center the chevron within the arrow box on the right edge.
    let cx = x + w - arrow_w / 2.0;
    let cy = y + h / 2.0;
    let half = 4.0 * scale; // half-width of the chevron
    let drop = 3.0 * scale; // vertical extent

    let mut path = BezPath::new();
    path.move_to(Point::new(cx - half, cy - drop / 2.0));
    path.line_to(Point::new(cx + half, cy - drop / 2.0));
    path.line_to(Point::new(cx, cy + drop / 2.0 + drop / 2.0));
    path.close_path();

    painter.fill(Fill::NonZero, transform, &Brush::Solid(color), &path.into());
}
