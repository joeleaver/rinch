//! Painting one subtree on its own, for a drag ghost: [`paint_subtree`],
//! [`paint_subtree_at`], and [`paint_subtree_bounds`], what they can draw.

use peniko::Brush;
use peniko::kurbo::{Affine, Rect};

use super::layer_bounds::unbudgeted_subtree_bounds;
use super::painter::Painter;
use super::{
    ClipTrackingPainter, SnapshotPaintScope, compose_node_transform, fold_span_line_visits,
    paint_node,
};
use crate::node::{NodeTree, RawNodeId};

/// Paint a subtree rooted at `root_node_id` into `painter`, positioned at (0, 0).
///
/// Used for drag-and-drop snapshot capture. The subtree is translated so its
/// top-left corner is at the scene origin, making it easy to reposition later.
pub fn paint_subtree(
    tree: &NodeTree,
    painter: &mut dyn Painter,
    root_node_id: RawNodeId,
    scale: f64,
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
) {
    paint_subtree_at(
        tree,
        painter,
        root_node_id,
        scale,
        (0.0, 0.0),
        font_cx,
        layout_cx,
    );
}

/// [`paint_subtree`] with the subtree's top-left corner put at `origin`
/// (device pixels) rather than at (0, 0): everything it draws, its own
/// transform included, moved by `origin`.
///
/// The software drag ghost is a pixmap whose (0, 0) is the top-left of what
/// the ghost draws ([`paint_subtree_bounds`]), which is left of and above the
/// box wherever an outline, a shadow or a scaled-up transform reaches past
/// it (#3065). It paints at minus that corner.
pub fn paint_subtree_at(
    tree: &NodeTree,
    painter: &mut dyn Painter,
    root_node_id: RawNodeId,
    scale: f64,
    origin: (f64, f64),
    font_cx: &mut parley::FontContext,
    layout_cx: &mut parley::LayoutContext<Brush>,
) {
    // paint_node computes each node's position as: offset + layout.x * scale.
    // To place the subtree root at `origin`, we negate just the root's own
    // layout position — NOT the full absolute position (which would over-shift
    // by the entire ancestor chain).
    let Some(node) = tree.get(root_node_id) else {
        return;
    };
    let offset_x = origin.0 - node.layout.x as f64 * scale;
    let offset_y = origin.1 - node.layout.y as f64 * scale;
    let _snapshot = SnapshotPaintScope::enter(root_node_id);
    ClipTrackingPainter::run(painter, scale, &tree.perf, |painter| {
        paint_node(
            tree,
            root_node_id,
            painter,
            scale,
            offset_x,
            offset_y,
            font_cx,
            layout_cx,
            Affine::IDENTITY,
        )
    });
    fold_span_line_visits(tree);
}

/// Where [`paint_subtree`] can draw for the same `root_node_id` and `scale`:
/// device pixels around the scene origin its border box is put at, every
/// descendant, shadow and outline included, and its own transform applied.
/// `None` when the walk cannot bound what the subtree paints: a `position:
/// fixed` or `sticky` descendant, or one deeper than the walk descends.
///
/// A drag ghost is the scene `paint_subtree` records, appended at the
/// pointer: the frame that moves it on or takes it away must repaint every
/// pixel it covered, and its box alone misses an outline, a shadow or the
/// antialiased edge at a fractional offset. Asked once per drag, so with no
/// visit budget: a large subtree is measured, not given up on.
pub fn paint_subtree_bounds(tree: &NodeTree, root_node_id: RawNodeId, scale: f64) -> Option<Rect> {
    let node = tree.get(root_node_id)?;
    let bounds = unbudgeted_subtree_bounds(tree, root_node_id, scale, 0.0, 0.0)?;
    let transform = compose_node_transform(node, 0.0, 0.0, scale, Affine::IDENTITY);
    Some(transform.transform_rect_bbox(bounds))
}
