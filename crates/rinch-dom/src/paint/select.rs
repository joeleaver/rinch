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
use crate::select::{
    SELECT_ARROW_BOX, SELECT_LABEL_INSET, resolve_select_model, select_label_layout,
};

/// Paint the closed `<select>`: the selected option's label, clipped to the
/// content box less its arrow box, plus a chevron centred in the arrow box.
///
/// `x, y, w, h` is the border box. The label and the arrow sit in the
/// **content** box — inside the border and the padding — at the menulist inner
/// box's offsets (`select::SELECT_LABEL_INSET`, `select::SELECT_ARROW_BOX`),
/// the same ones the box was sized with, so the label drawn is the label that
/// was measured (#1098).
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

    let cs = &node.computed_style;
    let left = (cs.border_left_width.to_px() + cs.padding_left.to_px()) as f64 * scale;
    let right = (cs.border_right_width.to_px() + cs.padding_right.to_px()) as f64 * scale;
    let top = (cs.border_top_width.to_px() + cs.padding_top.to_px()) as f64 * scale;
    let bottom = (cs.border_bottom_width.to_px() + cs.padding_bottom.to_px()) as f64 * scale;
    let (cx0, cy0) = (x + left, y + top);
    let cw = (w - left - right).max(0.0);
    let ch = (h - top - bottom).max(0.0);
    let arrow_w = SELECT_ARROW_BOX as f64 * scale;

    let base_color = cs
        .color
        .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(33, 37, 41, 255)); // #212529

    // Draw the arrow first so it always shows, even for an empty <select>.
    paint_arrow(
        painter,
        scale,
        cx0 + cw - arrow_w / 2.0,
        cy0 + ch / 2.0,
        base_color,
        transform,
    );

    // A single <select> with options always has a selected label; an empty
    // <select> has nothing to show.
    let Some(label) = model.selected_label().filter(|s| !s.is_empty()) else {
        return;
    };

    tree.perf.bump(crate::perf::Counter::ShapePaint);
    let text_layout = select_label_layout(
        font_cx,
        layout_cx,
        cs,
        cs.font_size * scale as f32,
        Brush::Solid(base_color),
        label,
    );

    let text_height = text_layout.height() as f64;
    let text_x = cx0 + SELECT_LABEL_INSET as f64 * scale;
    let text_y = cy0 + (ch - text_height) / 2.0;

    // Clip the label to the content box less the arrow box, so a long option
    // clips instead of running under the arrow.
    let content_right = (cx0 + cw - arrow_w).max(text_x);
    let clip_rect = peniko::kurbo::Rect::new(text_x, y, content_right, y + h);
    painter.push_clip(Fill::NonZero, transform, &clip_rect.into());

    let text_shadows = cs.text_shadow.as_slice();
    render_text_with_shadow(
        painter,
        &text_layout,
        text_x,
        text_y,
        text_shadows,
        transform,
        1.0,
        None,
        None,
        None,
    );

    painter.pop_layer();
}

/// Draw the downward chevron centred on `(cx, cy)`.
fn paint_arrow(
    painter: &mut dyn Painter,
    scale: f64,
    cx: f64,
    cy: f64,
    color: AlphaColor<Srgb>,
    transform: Affine,
) {
    let half = 4.0 * scale; // half-width of the chevron
    let drop = 3.0 * scale; // vertical extent

    let mut path = BezPath::new();
    path.move_to(Point::new(cx - half, cy - drop / 2.0));
    path.line_to(Point::new(cx + half, cy - drop / 2.0));
    path.line_to(Point::new(cx, cy + drop / 2.0 + drop / 2.0));
    path.close_path();

    painter.fill(Fill::NonZero, transform, &Brush::Solid(color), &path.into());
}
