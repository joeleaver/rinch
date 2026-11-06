//! Painting an `<img>` that has no picture (#2309; the rules are
//! [`crate::img_placeholder`]'s). Both painters draw it through the same
//! [`Painter`] calls: fills and glyphs.

use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{Affine, BezPath, Rect, Shape};
use peniko::{Brush, Fill};

use super::painter::Painter;
use super::text::render_text_with_shadow;
use crate::img_placeholder::{self as ph, Placeholder};
use crate::node::{Node, NodeTree};

/// A colour custom property on `node`, `currentcolor` read as its `color`.
fn colour_property(node: &Node, name: &str, current: peniko::Color) -> Option<peniko::Color> {
    let value = ph::custom_property(node, name)?;
    crate::layout::parse_color_with_current(&value, current)
}

/// The border width property: a `px` length or a bare `0`.
fn width_property(node: &Node) -> Option<f32> {
    let value = ph::custom_property(node, ph::BORDER_WIDTH_PROPERTY)?;
    let value = value.trim();
    let n = value
        .strip_suffix("px")
        .or_else(|| value.strip_suffix("PX"))
        .unwrap_or(value);
    if n != value || n.parse::<f32>().ok() == Some(0.0) {
        n.trim()
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.0)
    } else {
        None
    }
}

/// Draw `node`'s placeholder, if it is owed one, in its content box (the
/// border box `x, y, w, h` in device px less its border and padding).
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_img_placeholder(
    tree: &NodeTree,
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
    let kind = ph::placeholder(tree, node);
    if kind == Placeholder::None {
        return;
    }
    let cs = &node.computed_style;
    let (l, t, r, b) = super::content_box_insets(node);
    let cx = x + f64::from(l) * scale;
    let cy = y + f64::from(t) * scale;
    let cw = (w - f64::from(l + r) * scale).max(0.0);
    let chh = (h - f64::from(t + b) * scale).max(0.0);
    if cw <= 0.0 || chh <= 0.0 {
        return;
    }
    let content = Rect::new(cx, cy, cx + cw, cy + chh);
    let current = cs
        .color
        .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255));
    let Some(text) = ph::alt_text(node).or_else(|| (kind == Placeholder::Box).then(String::new))
    else {
        return;
    };

    // The text's origin and the box it is clipped to.
    let (origin_x, origin_y, clip, max_width) = match kind {
        Placeholder::Box => {
            let bw = width_property(node).unwrap_or(ph::BORDER_WIDTH);
            let bwd = f64::from(bw) * scale;
            let inner = content.inset(-bwd);
            if let Some(bg) = colour_property(node, ph::BACKGROUND_PROPERTY, current)
                && bg.components[3] > 0.0
                && inner.width() > 0.0
                && inner.height() > 0.0
            {
                painter.fill_color(Fill::NonZero, transform, bg, &inner.into());
            }
            if bwd > 0.0 {
                let colour = colour_property(node, ph::BORDER_COLOR_PROPERTY, current)
                    .unwrap_or_else(|| {
                        let [r, g, b] = ph::BORDER_COLOR;
                        AlphaColor::<Srgb>::from_rgba8(r, g, b, 255)
                    });
                let mut ring = BezPath::new();
                ring.extend(content.path_elements(0.1));
                if inner.width() > 0.0 && inner.height() > 0.0 {
                    ring.extend(inner.path_elements(0.1));
                }
                painter.fill(
                    Fill::EvenOdd,
                    transform,
                    &Brush::Solid(colour),
                    &ring.into(),
                );
            }
            let pad = f64::from(bw + ph::PADDING) * scale;
            let text_box = content.inset(-pad);
            (
                text_box.x0,
                text_box.y0,
                inner,
                Some((text_box.width() / scale).max(0.0) as f32),
            )
        }
        _ => (cx, cy, content, None),
    };
    if text.is_empty() || clip.width() <= 0.0 || clip.height() <= 0.0 {
        return;
    }

    tree.perf.bump(crate::perf::Counter::ShapePaint);
    let layout = ph::alt_layout(node, &text, font_cx, layout_cx, max_width);
    // An inline one's box is its line box (`AltBox`), from its top.
    let shift = if kind == Placeholder::Inline {
        -f64::from(
            layout
                .lines()
                .next()
                .map_or(0.0, |l| l.metrics().block_min_coord),
        ) * scale
    } else {
        0.0
    };
    painter.push_clip(Fill::NonZero, transform, &clip.into());
    render_text_with_shadow(
        painter,
        &layout,
        origin_x,
        origin_y + shift,
        cs.text_shadow.as_slice(),
        transform,
        scale,
        None,
        Some(current),
        None,
    );
    painter.pop_layer();
}
