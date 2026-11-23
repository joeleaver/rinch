//! `background-clip: text` (#2719): a background painted only inside the
//! glyphs of the element's text and its in-flow descendants' text.
//!
//! The background is drawn into an isolated layer, and the text is drawn
//! over it into a [`BlendMode::DestinationIn`] layer, opaque, as a mask:
//! what is left is the background where a glyph covers it, at the glyph's
//! coverage. The glyphs are the ones paint draws (the glyph-mask atlas'
//! fills, hinted as the text is), so the background's edge is the text's.
//! As in Chrome, a `-webkit-text-stroke` widens the mask by the stroke
//! (whatever the stroke's colour), and decorations, `text-shadow`s and
//! hidden text are not in it. The text itself is drawn later, over the
//! background, as always: with `color: transparent` (or a transparent
//! `-webkit-text-fill-color`) only its stroke shows.

use peniko::kurbo::Affine;
use peniko::{Brush, Fill};

use super::painter::{BlendMode, GlyphSynthesis, PaintGlyph, PaintShape, Painter};
use super::text::{
    InlineScope, PaintText, TextEffects, TextMask, TextPass, WeightSynthesisOff, draw_text,
};
use crate::computed_style::{DisplayValue, PositionValue, VisibilityValue};
use crate::node::{Node, NodeKind, NodeTree};

/// The opaque colour a mask is drawn in.
fn mask_brush() -> Brush {
    Brush::Solid(peniko::Color::BLACK)
}

/// Draw `draw` (a background, or one of its layers) clipped to the text
/// `text` draws: `draw` into an isolated layer over `bounds`, then `text`
/// into a `destination-in` layer over it.
pub(super) fn clip_to_text(
    painter: &mut dyn Painter,
    transform: Affine,
    bounds: &PaintShape,
    draw: impl FnOnce(&mut dyn Painter),
    text: &mut dyn FnMut(&mut dyn Painter),
) {
    painter.push_isolated_layer(1.0, transform, bounds);
    draw(painter);
    painter.push_layer(BlendMode::DestinationIn, 1.0, transform, bounds);
    text(painter);
    painter.pop_layer();
    painter.pop_layer();
}

/// Draw the text of `node` and of its in-flow descendants as a mask
/// ([`TextPass::Mask`]): every glyph and its stroke, opaque, nothing else.
/// `(x, y)` is where paint places `node`'s box (device px, before
/// `transform`), as the element's own paint has it.
pub(super) fn draw_subtree_text_mask(
    tree: &NodeTree,
    node: &Node,
    painter: &mut dyn Painter,
    scale: f64,
    x: f64,
    y: f64,
    transform: Affine,
) {
    let mut glyphs = GlyphsOnly(painter);
    visit(tree, node, &mut glyphs, scale, x, y, transform, 0);
}

/// How deep the walk goes: a mask is the element's own text, not a page's.
const MAX_DEPTH: usize = 64;

#[allow(clippy::too_many_arguments)]
fn visit(
    tree: &NodeTree,
    node: &Node,
    painter: &mut dyn Painter,
    scale: f64,
    x: f64,
    y: f64,
    transform: Affine,
    depth: usize,
) {
    if depth > MAX_DEPTH || node.computed_style.display == DisplayValue::None {
        return;
    }
    let brush = mask_brush();
    match &node.kind {
        NodeKind::Text(_) => {
            let Some(layout) = node.cached_text_parley.as_ref() else {
                return;
            };
            let Some(parent) = node.parent.and_then(|p| tree.get_live(p)) else {
                return;
            };
            if is_hidden(parent) {
                return;
            }
            let mut text = PaintText::new(layout);
            text.set_weight_synthesis_off(WeightSynthesisOff::uniform(
                parent.computed_style.font_synthesis_weight,
            ));
            text.set_effects(TextEffects::uniform(parent.computed_style.text_paint));
            draw_text(
                painter,
                &text,
                transform * Affine::translate((x, y)),
                scale,
                None,
                Some(&brush),
                Some(&brush),
                None,
                None,
                TextPass::Mask,
            );
            return;
        }
        NodeKind::Element(_) => {}
        _ => return,
    }
    let scroll_x = node.scroll_offset.0 * scale;
    let scroll_y = node.scroll_offset.1 * scale;
    if let Some(inline_layout) = &node.text_layout {
        let (off_x, off_y) = super::ifc_root_content_origin(node);
        let mut text = PaintText::of_ifc(inline_layout);
        text.set_weight_synthesis_off(WeightSynthesisOff::for_ifc(tree, inline_layout, node));
        text.set_effects(TextEffects::for_ifc(tree, inline_layout, node));
        let mask = TextMask::for_ifc(tree, inline_layout, is_hidden(node), InlineScope::All);
        draw_text(
            painter,
            &text,
            transform
                * Affine::translate((
                    x + off_x as f64 * scale - scroll_x,
                    y + off_y as f64 * scale - scroll_y,
                )),
            scale,
            mask.as_ref(),
            Some(&brush),
            Some(&brush),
            Some(inline_layout),
            None,
            TextPass::Mask,
        );
    }
    // The box tree: inline runs beside a block child are anonymous block
    // boxes, IFC roots of their own (#2719 review).
    let children = crate::RinchDocument::box_tree_children(&tree.nodes, node.id);
    for &child_id in children.iter() {
        let Some(child) = tree.get_live(child_id) else {
            continue;
        };
        // Its text is the IFC's, drawn above; an out-of-flow box is not an
        // in-flow descendant; a transformed one is placed by its transform,
        // which this walk does not compose.
        if super::drawn_by_its_ifc(tree, child)
            || matches!(
                child.computed_style.position,
                PositionValue::Absolute | PositionValue::Fixed
            )
            || !child.computed_style.transform.is_identity
        {
            continue;
        }
        let cx = x - scroll_x + child.layout.x as f64 * scale;
        let cy = y - scroll_y + child.layout.y as f64 * scale;
        visit(tree, child, painter, scale, cx, cy, transform, depth + 1);
    }
}

/// Draw `text`, through `mask`, as a mask ([`TextPass::Mask`]) in `brush`:
/// an inline element's own text.
pub(super) fn draw_text_mask(
    painter: &mut dyn Painter,
    text: &PaintText<'_>,
    transform: Affine,
    scale: f64,
    mask: &TextMask,
    brush: &Brush,
    inline_layout: &crate::node::InlineLayout,
) {
    draw_text(
        &mut GlyphsOnly(painter),
        text,
        transform,
        scale,
        Some(mask),
        Some(brush),
        Some(brush),
        Some(inline_layout),
        None,
        TextPass::Mask,
    );
}

fn is_hidden(node: &Node) -> bool {
    matches!(
        node.computed_style.visibility,
        VisibilityValue::Hidden | VisibilityValue::Collapse
    )
}

/// A painter that passes on glyphs and strokes (a text's fill and outline)
/// and drops every fill (its decorations): what a text mask is made of.
struct GlyphsOnly<'p>(&'p mut dyn Painter);

impl Painter for GlyphsOnly<'_> {
    fn reset(&mut self) {}
    fn fill(&mut self, _: Fill, _: Affine, _: &Brush, _: &PaintShape) {}
    fn stroke(
        &mut self,
        stroke: &peniko::kurbo::Stroke,
        transform: Affine,
        brush: &Brush,
        shape: &PaintShape,
    ) {
        self.0.stroke(stroke, transform, brush, shape);
    }
    fn draw_glyphs(
        &mut self,
        font: &peniko::FontData,
        font_size: f32,
        transform: Affine,
        synthesis: GlyphSynthesis,
        brush: &Brush,
        hint: bool,
        normalized_coords: &[i16],
        glyphs: &[PaintGlyph],
    ) {
        self.0.draw_glyphs(
            font,
            font_size,
            transform,
            synthesis,
            brush,
            hint,
            normalized_coords,
            glyphs,
        );
    }
    fn draw_image(&mut self, _: &super::painter::PaintImage<'_>, _: Affine) {}
    fn push_clip(&mut self, fill: Fill, transform: Affine, shape: &PaintShape) {
        self.0.push_clip(fill, transform, shape);
    }
    fn push_layer(
        &mut self,
        blend: BlendMode,
        opacity: f32,
        transform: Affine,
        bounds: &PaintShape,
    ) {
        self.0.push_layer(blend, opacity, transform, bounds);
    }
    fn pop_layer(&mut self) {
        self.0.pop_layer();
    }
}
