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

/// The border width property, in CSS px: an absolute length (`px`, `pt`,
/// `pc`, `in`, `cm`, `mm`, `q`), an `em` (against the element's font size)
/// or a bare `0`. `None` for anything else, which keeps the default.
fn width_property(node: &Node) -> Option<f32> {
    let value = ph::custom_property(node, ph::BORDER_WIDTH_PROPERTY)?;
    let (v, unit) = crate::replaced::svg_length(&value)?;
    let px = match unit.as_str() {
        "px" => v,
        "" if v == 0.0 => 0.0,
        "in" => v * 96.0,
        "cm" => v * 96.0 / 2.54,
        "mm" => v * 96.0 / 25.4,
        "q" => v * 96.0 / 101.6,
        "pt" => v * 96.0 / 72.0,
        "pc" => v * 16.0,
        "em" => v * f64::from(node.computed_style.font_size),
        _ => return None,
    } as f32;
    (px.is_finite() && px >= 0.0).then_some(px)
}

/// Draw `node`'s placeholder, if it is owed one, in its content box
/// `content` (device px: the border box less the border and padding layout
/// resolved). The caller clips it to the content edge's curve when the box is
/// rounded, as it clips a picture.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_img_placeholder(
    tree: &NodeTree,
    node: &Node,
    painter: &mut dyn Painter,
    scale: f64,
    content: Rect,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
    transform: Affine,
) {
    let kind = ph::placeholder(tree, node);
    if kind == Placeholder::None {
        return;
    }
    let cs = &node.computed_style;
    let (cx, cy, cw, chh) = (content.x0, content.y0, content.width(), content.height());
    if cw <= 0.0 || chh <= 0.0 {
        return;
    }
    // A box too thin for Chrome to draw one in draws nothing: a held-back
    // spacer or divider is not a silver line (3px with no `src`, 18px after
    // a failed load, on each axis).
    if kind == Placeholder::Box {
        let min = f64::from(ph::min_box(node)) * scale;
        if cw < min - 1e-3 || chh < min - 1e-3 {
            return;
        }
    }
    let current = cs
        .color
        .unwrap_or_else(|| AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255));
    // The alt text, shaped when the element was synced
    // (`RinchDocument::sync_img_placeholder`).
    let alt = node
        .taffy_id
        .and_then(|t| match tree.taffy.get_node_context(t) {
            Some(crate::node::NodeContext::Image { alt, .. }) => alt.as_ref(),
            _ => None,
        });

    // The text's origin, the box it is clipped to and the width it wraps at.
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
                (text_box.width() / scale).max(0.0) as f32,
            )
        }
        // Laid out in whole pixels, so up to a pixel narrower than the width
        // its lines were measured at: broken there, not a word earlier.
        _ => (cx, cy, content, (cw / scale) as f32 + 1.0),
    };
    if clip.width() <= 0.0 || clip.height() <= 0.0 {
        return;
    }
    // The shaped text as it is when it fits on one line (no copy), else
    // re-broken to the width.
    let owned;
    let layout: &parley::Layout<Brush> = match alt {
        Some(alt) if max_width + 0.01 >= alt.max_content => &alt.layout,
        Some(alt) => {
            owned = alt.broken(max_width);
            &owned
        }
        // Not synced yet (no layout has run since the alt text appeared).
        None => {
            let Some(text) = ph::alt_text(node) else {
                return;
            };
            tree.perf.bump(crate::perf::Counter::ShapePaint);
            owned = ph::alt_layout(node, &text, font_cx, layout_cx, Some(max_width));
            &owned
        }
    };
    // Its lines from the top of its box.
    let shift = -f64::from(
        layout
            .lines()
            .next()
            .map_or(0.0, |l| l.metrics().block_min_coord),
    ) * scale;
    painter.push_clip(Fill::NonZero, transform, &clip.into());
    render_text_with_shadow(
        painter,
        layout,
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
